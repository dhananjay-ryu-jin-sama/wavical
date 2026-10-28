#![forbid(unsafe_code)]

use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use tempfile::tempdir;
use wavical_ai::catalog::{ModelCatalog, ModelCatalogEntry, ModelKind};
use wavical_ai::commands::{CommandError, list_models, remove_model, verify_model};
use wavical_ai::downloader::{DownloadError, DownloadOptions, download_model};
use wavical_ai::gguf::create_fake_gguf_bytes;
use wavical_ai::license::{LicenseError, check_and_prompt_license};
use wavical_ai::sha256::sha256_bytes;

/// Lightweight mock HTTP server for testing download behaviors.
struct MockHttpServer {
    port: u16,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

enum MockResponse {
    Fixed(Vec<u8>),
    Redirect(String),
    Truncated(Vec<u8>, usize),
}

impl MockHttpServer {
    fn start(
        response_fn: Arc<dyn Fn(&str, Option<&str>) -> MockResponse + Send + Sync + 'static>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local mock server");
        let port = listener.local_addr().unwrap().port();
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);

        let handle = thread::spawn(move || {
            while running_clone.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    if !running_clone.load(Ordering::SeqCst) {
                        break;
                    }
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request_line = String::new();
                    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
                        continue;
                    }

                    let path = request_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .to_string();

                    // Read headers looking for Range
                    let mut range_header = None;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty()
                        {
                            break;
                        }
                        if line.to_ascii_lowercase().starts_with("range:") {
                            range_header = Some(line.trim().to_string());
                        }
                    }

                    let mock_resp = response_fn(&path, range_header.as_deref());
                    match mock_resp {
                        MockResponse::Fixed(bytes) => {
                            if let Some(ref range) = range_header {
                                // Range: bytes=X-
                                if let Some(idx) = range.find('=') {
                                    let val = range[idx + 1..].trim();
                                    if let Some(start_str) = val.split('-').next() {
                                        if let Ok(start) = start_str.parse::<usize>() {
                                            if start < bytes.len() {
                                                let slice = &bytes[start..];
                                                let resp = format!(
                                                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                                                    slice.len(),
                                                    start,
                                                    bytes.len() - 1,
                                                    bytes.len()
                                                );
                                                let _ = stream.write_all(resp.as_bytes());
                                                let _ = stream.write_all(slice);
                                                let _ = stream.flush();
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }

                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                bytes.len()
                            );
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.write_all(&bytes);
                            let _ = stream.flush();
                        }
                        MockResponse::Redirect(target) => {
                            let resp = format!(
                                "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                target
                            );
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.flush();
                        }
                        MockResponse::Truncated(bytes, cutoff) => {
                            let slice = &bytes[..cutoff.min(bytes.len())];
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                slice.len()
                            );
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.write_all(slice);
                            let _ = stream.flush();
                        }
                    }
                }
            }
        });

        Self {
            port,
            running,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

impl Drop for MockHttpServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = TcpStream::connect(format!("127.0.0.1:{}", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn create_test_catalog(entry: ModelCatalogEntry) -> ModelCatalog {
    ModelCatalog {
        version: 1,
        entries: vec![entry],
    }
}

// ============================================================================
// WP3 TASK 7 TEST SUITE
// ============================================================================

#[test]
fn test_download_success_with_mock_server() {
    let fake_gguf = create_fake_gguf_bytes(3, 512);
    let expected_hash = sha256_bytes(&fake_gguf);
    let expected_size = fake_gguf.len() as u64;

    let payload = fake_gguf.clone();
    let server = MockHttpServer::start(Arc::new(move |_, _| MockResponse::Fixed(payload.clone())));

    let entry = ModelCatalogEntry {
        id: "mock-model-v3".to_string(),
        name: "Mock Model".to_string(),
        kind: ModelKind::Chat,
        file_name: "mock-model-v3.gguf".to_string(),
        url: server.url("/mock.gguf"),
        size_bytes: expected_size,
        sha256: expected_hash.clone(),
        license_name: "Apache-2.0".to_string(),
        license_url: "https://example.com/license".to_string(),
        min_ram_bytes: 1_000_000,
        notes: None,
    };

    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    let target_path =
        download_model(&entry, models_dir, &options, None).expect("download should succeed");
    assert!(target_path.is_file());
    assert_eq!(fs::metadata(&target_path).unwrap().len(), expected_size);
    assert_eq!(
        wavical_ai::sha256::sha256_file(&target_path).unwrap(),
        expected_hash
    );
}

#[test]
fn test_download_tampered_content_hash_mismatch() {
    let fake_gguf = create_fake_gguf_bytes(3, 512);
    let tampered_gguf = create_fake_gguf_bytes(3, 512); // same size, but we alter expected hash

    let payload = tampered_gguf.clone();
    let server = MockHttpServer::start(Arc::new(move |_, _| MockResponse::Fixed(payload.clone())));

    let entry = ModelCatalogEntry {
        id: "mock-tampered".to_string(),
        name: "Tampered".to_string(),
        kind: ModelKind::Chat,
        file_name: "mock-tampered.gguf".to_string(),
        url: server.url("/mock.gguf"),
        size_bytes: fake_gguf.len() as u64,
        sha256: "0".repeat(64), // Mismatched hash
        license_name: "MIT".to_string(),
        license_url: "https://example.com/license".to_string(),
        min_ram_bytes: 1_000_000,
        notes: None,
    };

    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    let err = download_model(&entry, models_dir, &options, None).unwrap_err();
    assert!(matches!(err, DownloadError::HashMismatch { .. }));

    // Assert partial and target files are cleaned up on failure
    assert!(!models_dir.join("mock-tampered.gguf").exists());
    assert!(!models_dir.join("mock-tampered.gguf.part").exists());
}

#[test]
fn test_download_truncated_stream_fails_and_cleans_up() {
    let fake_gguf = create_fake_gguf_bytes(3, 1024);
    let expected_hash = sha256_bytes(&fake_gguf);
    let expected_size = fake_gguf.len() as u64;

    let payload = fake_gguf.clone();
    // Server cuts off at 256 bytes
    let server = MockHttpServer::start(Arc::new(move |_, _| {
        MockResponse::Truncated(payload.clone(), 256)
    }));

    let entry = ModelCatalogEntry {
        id: "mock-trunc".to_string(),
        name: "Trunc".to_string(),
        kind: ModelKind::Chat,
        file_name: "mock-trunc.gguf".to_string(),
        url: server.url("/mock.gguf"),
        size_bytes: expected_size,
        sha256: expected_hash,
        license_name: "MIT".to_string(),
        license_url: "https://example.com/license".to_string(),
        min_ram_bytes: 1_000_000,
        notes: None,
    };

    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    let err = download_model(&entry, models_dir, &options, None).unwrap_err();
    assert!(matches!(err, DownloadError::SizeMismatch { .. }));

    assert!(!models_dir.join("mock-trunc.gguf").exists());
    assert!(!models_dir.join("mock-trunc.gguf.part").exists());
}

#[test]
fn test_download_wrong_size_mismatch() {
    let fake_gguf = create_fake_gguf_bytes(3, 200);
    let expected_hash = sha256_bytes(&fake_gguf);

    let payload = fake_gguf.clone();
    let server = MockHttpServer::start(Arc::new(move |_, _| MockResponse::Fixed(payload.clone())));

    // Catalog declares size as 500, but server sends 208 bytes
    let entry = ModelCatalogEntry {
        id: "mock-wrong-size".to_string(),
        name: "Wrong Size".to_string(),
        kind: ModelKind::Chat,
        file_name: "mock-wrong-size.gguf".to_string(),
        url: server.url("/mock.gguf"),
        size_bytes: 500,
        sha256: expected_hash,
        license_name: "MIT".to_string(),
        license_url: "https://example.com/license".to_string(),
        min_ram_bytes: 1_000_000,
        notes: None,
    };

    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    let err = download_model(&entry, models_dir, &options, None).unwrap_err();
    match err {
        DownloadError::SizeMismatch { expected, actual } => {
            assert_eq!(expected, 500);
            assert_eq!(actual, fake_gguf.len() as u64);
        }
        other => panic!("Expected SizeMismatch, got: {other:?}"),
    }

    assert!(!models_dir.join("mock-wrong-size.gguf").exists());
    assert!(!models_dir.join("mock-wrong-size.gguf.part").exists());
}

#[test]
fn test_download_redirect_to_non_allowlisted_host_refused() {
    let server = MockHttpServer::start(Arc::new(move |_, _| {
        MockResponse::Redirect("https://unapproved-malicious-host.com/model.gguf".to_string())
    }));

    let entry = ModelCatalogEntry {
        id: "mock-redirect".to_string(),
        name: "Redirect".to_string(),
        kind: ModelKind::Chat,
        file_name: "mock-redirect.gguf".to_string(),
        url: server.url("/start.gguf"),
        size_bytes: 1000,
        sha256: "0".repeat(64),
        license_name: "MIT".to_string(),
        license_url: "https://example.com/license".to_string(),
        min_ram_bytes: 1_000_000,
        notes: None,
    };

    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    let err = download_model(&entry, models_dir, &options, None).unwrap_err();
    assert!(matches!(err, DownloadError::DisallowedHost(_)));
}

#[test]
fn test_download_plain_http_url_rejected_without_localhost() {
    let entry = ModelCatalogEntry {
        id: "mock-http".to_string(),
        name: "HTTP".to_string(),
        kind: ModelKind::Chat,
        file_name: "mock-http.gguf".to_string(),
        url: "http://huggingface.co/model.gguf".to_string(),
        size_bytes: 1000,
        sha256: "0".repeat(64),
        license_name: "MIT".to_string(),
        license_url: "https://example.com/license".to_string(),
        min_ram_bytes: 1_000_000,
        notes: None,
    };

    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    // Default options enforce HTTPS strictly
    let options = DownloadOptions::default();

    let err = download_model(&entry, models_dir, &options, None).unwrap_err();
    assert!(matches!(err, DownloadError::InsecureScheme(_)));
}

#[test]
fn test_download_resume_interrupted_file() {
    let fake_gguf = create_fake_gguf_bytes(3, 1024);
    let expected_hash = sha256_bytes(&fake_gguf);
    let expected_size = fake_gguf.len() as u64;

    let payload = fake_gguf.clone();
    let server = MockHttpServer::start(Arc::new(move |_, _| MockResponse::Fixed(payload.clone())));

    let entry = ModelCatalogEntry {
        id: "mock-resume".to_string(),
        name: "Resume".to_string(),
        kind: ModelKind::Chat,
        file_name: "mock-resume.gguf".to_string(),
        url: server.url("/mock.gguf"),
        size_bytes: expected_size,
        sha256: expected_hash.clone(),
        license_name: "Apache-2.0".to_string(),
        license_url: "https://example.com/license".to_string(),
        min_ram_bytes: 1_000_000,
        notes: None,
    };

    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    // 1. Pre-create a partial file with first 256 bytes
    let part_path = models_dir.join("mock-resume.gguf.part");
    fs::write(&part_path, &fake_gguf[..256]).unwrap();
    assert_eq!(fs::metadata(&part_path).unwrap().len(), 256);

    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    // 2. Call download_model: it should send Range: bytes=256- and complete
    let target = download_model(&entry, models_dir, &options, None)
        .expect("resumed download should succeed");
    assert!(target.is_file());
    assert_eq!(fs::metadata(&target).unwrap().len(), expected_size);
    assert_eq!(
        wavical_ai::sha256::sha256_file(&target).unwrap(),
        expected_hash
    );
}

#[test]
fn test_license_prompt_bypass_attempts() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path();

    let entry = ModelCatalogEntry {
        id: "qwen2.5-1.5b-instruct".to_string(),
        name: "Qwen".to_string(),
        kind: ModelKind::Chat,
        file_name: "qwen2.5.gguf".to_string(),
        url: "https://huggingface.co/model.gguf".to_string(),
        size_bytes: 1000,
        sha256: "0".repeat(64),
        license_name: "Apache-2.0".to_string(),
        license_url: "https://example.com/license".to_string(),
        min_ram_bytes: 1_000_000,
        notes: None,
    };

    // 1. Empty input
    let mut in_empty = Cursor::new(b"\n");
    let mut out = Vec::new();
    assert!(matches!(
        check_and_prompt_license(&entry, data_dir, 1, None, &mut in_empty, &mut out).unwrap_err(),
        LicenseError::ConfirmationAborted(_)
    ));

    // 2. Whitespace only
    let mut in_ws = Cursor::new(b"   \t  \n");
    let mut out = Vec::new();
    assert!(matches!(
        check_and_prompt_license(&entry, data_dir, 1, None, &mut in_ws, &mut out).unwrap_err(),
        LicenseError::ConfirmationAborted(_)
    ));

    // 3. Wrong model ID
    let mut in_wrong = Cursor::new(b"qwen2.5-wrong\n");
    let mut out = Vec::new();
    assert!(matches!(
        check_and_prompt_license(&entry, data_dir, 1, None, &mut in_wrong, &mut out).unwrap_err(),
        LicenseError::ConfirmationAborted(_)
    ));

    // 4. Wrong license ID in --accept-license
    let mut dummy_in = Cursor::new(b"");
    let mut out = Vec::new();
    assert!(matches!(
        check_and_prompt_license(&entry, data_dir, 1, Some("MIT"), &mut dummy_in, &mut out)
            .unwrap_err(),
        LicenseError::LicenseMismatch { .. }
    ));
}

#[test]
fn test_remove_uninstalled_model_fails() {
    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    let err = remove_model("qwen2.5-1.5b-instruct", models_dir, None).unwrap_err();
    assert_eq!(
        err,
        CommandError::ModelNotInstalled("qwen2.5-1.5b-instruct".to_string())
    );
}

#[test]
fn test_verify_model_after_modifying_one_byte_fails() {
    let fake_gguf = create_fake_gguf_bytes(3, 512);
    let original_hash = sha256_bytes(&fake_gguf);

    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    let entry = ModelCatalogEntry {
        id: "test-verify-model".to_string(),
        name: "Test Model".to_string(),
        kind: ModelKind::Chat,
        file_name: "test-verify-model.gguf".to_string(),
        url: "https://huggingface.co/model.gguf".to_string(),
        size_bytes: fake_gguf.len() as u64,
        sha256: original_hash.clone(),
        license_name: "MIT".to_string(),
        license_url: "https://example.com/license".to_string(),
        min_ram_bytes: 1_000_000,
        notes: None,
    };
    let catalog = create_test_catalog(entry);

    // 1. Write valid file
    let file_path = models_dir.join("test-verify-model.gguf");
    fs::write(&file_path, &fake_gguf).unwrap();

    // Verification succeeds initially
    assert!(verify_model("test-verify-model", models_dir, Some(&catalog)).is_ok());

    // 2. Modify exactly one byte in the installed file
    let mut tampered = fake_gguf;
    tampered[15] ^= 0xFF;
    fs::write(&file_path, &tampered).unwrap();

    // Verification must now fail with HashMismatch
    let err = verify_model("test-verify-model", models_dir, Some(&catalog)).unwrap_err();
    match err {
        CommandError::HashMismatch {
            model_id,
            expected,
            actual,
        } => {
            assert_eq!(model_id, "test-verify-model");
            assert_eq!(expected, original_hash);
            assert_ne!(actual, original_hash);
        }
        other => panic!("Expected HashMismatch, got: {other:?}"),
    }
}

#[test]
fn test_cli_list_models_status() {
    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    // Default catalog has 2 models, neither installed
    let list = list_models(models_dir, None);
    assert_eq!(list.len(), 2);
    assert!(!list[0].installed);
    assert!(!list[1].installed);

    // Install one model file
    fs::write(
        models_dir.join("qwen2.5-1.5b-instruct-q4_k_m.gguf"),
        b"sample",
    )
    .unwrap();
    let list2 = list_models(models_dir, None);
    let qwen = list2
        .iter()
        .find(|m| m.id == "qwen2.5-1.5b-instruct")
        .unwrap();
    let bge = list2.iter().find(|m| m.id == "bge-small-en-v1.5").unwrap();
    assert!(qwen.installed);
    assert!(!bge.installed);
}

#[test]
fn test_download_lock_prevents_concurrent_downloads() {
    let temp = tempdir().unwrap();
    let models_dir = temp.path();

    let guard1 = wavical_ai::downloader::DownloadLockGuard::acquire(models_dir).unwrap();
    let err = wavical_ai::downloader::DownloadLockGuard::acquire(models_dir).unwrap_err();
    assert!(matches!(err, DownloadError::DownloadAlreadyInProgress(_)));

    drop(guard1);
    // After dropping, acquisition succeeds again
    let guard2 = wavical_ai::downloader::DownloadLockGuard::acquire(models_dir);
    assert!(guard2.is_ok());
}
