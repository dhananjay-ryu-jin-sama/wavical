#![forbid(unsafe_code)]

//! Comprehensive test suite for Phase 4 WP2:
//! - Shared verified downloader (HTTPS, allowlists, redirects, resume, hash, size)
//! - Safe archive extractor (zip slip, absolute, drive letter, UNC, symlink, ADS, reserved device names, case collision, limits)
//! - Pre-launch runtime executable hash checking (Task 6)
//! - Runtime list, verify, and status commands
//!
//! Uses a LOCAL MOCK SERVER and CRAFTED ZIP ARCHIVES. Zero external network calls.

use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;
use wavical_ai::downloader::{
    DownloadError, DownloadKind, DownloadOptions, download_verified_file,
};
use wavical_ai::sha256::Sha256Hasher;
use wavical_cli::extractor::{ExtractionLimits, ExtractorError, extract_zip_safely};
use wavical_cli::runtime::{compute_sha256, verify_runtime_executable_hash};
use wavical_protocol::{RuntimeArch, find_approved_runtime};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

// ============================================================================
// Local Mock HTTP Server
// ============================================================================

enum MockResponse {
    Fixed(Vec<u8>),
    Redirect(String),
    #[allow(dead_code)]
    Truncated(Vec<u8>, usize),
}

struct MockHttpServer {
    port: u16,
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockHttpServer {
    fn new<F>(response_fn: F) -> Self
    where
        F: Fn(&str, Option<&str>) -> MockResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let port = listener.local_addr().unwrap().port();
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let response_fn = Arc::new(response_fn);

        let handle = thread::spawn(move || {
            let _ = listener.set_nonblocking(true);
            while running_clone.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
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

                        let mut range_header = None;
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).is_err()
                                || line == "\r\n"
                                || line.is_empty()
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
                                    let start_opt = range
                                        .find('=')
                                        .and_then(|idx| range[idx + 1..].trim().split('-').next())
                                        .and_then(|s| s.parse::<usize>().ok())
                                        .filter(|&start| start < bytes.len());

                                    if let Some(start) = start_opt {
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
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
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
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ============================================================================
// Helper: Build in-memory crafted Zip archives
// ============================================================================

fn create_crafted_zip<F>(build: F) -> Vec<u8>
where
    F: FnOnce(&mut ZipWriter<Cursor<Vec<u8>>>),
{
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    build(&mut writer);
    writer.finish().unwrap().into_inner()
}

// ============================================================================
// Task 6: Pre-Launch Runtime Executable Hash Verification
// ============================================================================

#[test]
fn test_task6_pre_launch_executable_hash_check() {
    let temp = tempdir().unwrap();
    let fake_node = temp.path().join("node.exe");

    let original_bytes = b"MOCK_NODE_JS_EXECUTABLE_BINARY_V22";
    fs::write(&fake_node, original_bytes).unwrap();

    let expected_hash = compute_sha256(&fake_node).unwrap();

    // 1. Pristine executable passes launch verification
    assert!(
        verify_runtime_executable_hash(&fake_node, &expected_hash).is_ok(),
        "Pristine executable must pass launch check"
    );

    // 2. Modifying a single byte causes launch check to fail and refuse execution
    let mut tampered_bytes = original_bytes.to_vec();
    tampered_bytes[0] ^= 0xAA;
    fs::write(&fake_node, &tampered_bytes).unwrap();

    let err = verify_runtime_executable_hash(&fake_node, &expected_hash).unwrap_err();
    assert!(
        matches!(err, wavical_cli::runtime::RuntimeError::HashMismatch { .. }),
        "Tampered executable must trigger HashMismatch"
    );

    // 3. Missing executable triggers ExecutableMissing
    let _ = fs::remove_file(&fake_node);
    let missing_err = verify_runtime_executable_hash(&fake_node, &expected_hash).unwrap_err();
    assert!(
        matches!(
            missing_err,
            wavical_cli::runtime::RuntimeError::ExecutableMissing(_)
        ),
        "Missing executable must trigger ExecutableMissing"
    );
}

// ============================================================================
// Task 7: Crafted Archives & Safe Extractor Tests
// ============================================================================

#[test]
fn test_extractor_rejects_zip_slip() {
    let temp = tempdir().unwrap();
    let zip_bytes = create_crafted_zip(|writer| {
        let opts = SimpleFileOptions::default();
        writer.start_file("../../evil.txt", opts).unwrap();
        writer.write_all(b"evil content").unwrap();
    });

    let zip_file = temp.path().join("zip_slip.zip");
    fs::write(&zip_file, zip_bytes).unwrap();

    let dest = temp.path().join("out");
    let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);

    assert!(res.is_err(), "Zip slip entry must be rejected");
    assert!(
        !dest.exists(),
        "Destination folder must not exist after error"
    );
    assert!(
        !temp.path().join("evil.txt").exists(),
        "Traversal file must not exist"
    );
}

#[test]
fn test_extractor_rejects_absolute_path_entry() {
    let temp = tempdir().unwrap();
    let zip_bytes = create_crafted_zip(|writer| {
        let opts = SimpleFileOptions::default();
        writer.start_file("/etc/passwd", opts).unwrap();
        writer.write_all(b"root:x:0:0").unwrap();
    });

    let zip_file = temp.path().join("abs.zip");
    fs::write(&zip_file, zip_bytes).unwrap();

    let dest = temp.path().join("out");
    let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);
    assert!(res.is_err(), "Absolute path entry must be rejected");
}

#[test]
fn test_extractor_rejects_drive_letter_entry() {
    let temp = tempdir().unwrap();
    let zip_bytes = create_crafted_zip(|writer| {
        let opts = SimpleFileOptions::default();
        writer
            .start_file("C:/windows/system32/evil.dll", opts)
            .unwrap();
        writer.write_all(b"payload").unwrap();
    });

    let zip_file = temp.path().join("drive.zip");
    fs::write(&zip_file, zip_bytes).unwrap();

    let dest = temp.path().join("out");
    let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);
    assert!(res.is_err(), "Drive letter entry must be rejected");
}

#[test]
fn test_extractor_rejects_unc_entry() {
    let temp = tempdir().unwrap();
    let zip_bytes = create_crafted_zip(|writer| {
        let opts = SimpleFileOptions::default();
        writer.start_file(r"\\server\share\evil.txt", opts).unwrap();
        writer.write_all(b"payload").unwrap();
    });

    let zip_file = temp.path().join("unc.zip");
    fs::write(&zip_file, zip_bytes).unwrap();

    let dest = temp.path().join("out");
    let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);
    assert!(res.is_err(), "UNC entry must be rejected");
}

#[test]
fn test_extractor_rejects_symlink_entry() {
    let temp = tempdir().unwrap();
    let zip_bytes = create_crafted_zip(|writer| {
        let opts = SimpleFileOptions::default();
        writer
            .add_symlink("symlink_entry", "/target/file", opts)
            .unwrap();
    });

    let zip_file = temp.path().join("symlink.zip");
    fs::write(&zip_file, zip_bytes).unwrap();

    let dest = temp.path().join("out");
    let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);
    assert!(res.is_err(), "Symlink entry must be rejected");
}

#[test]
fn test_extractor_rejects_alternate_data_stream_entry() {
    let temp = tempdir().unwrap();
    let zip_bytes = create_crafted_zip(|writer| {
        let opts = SimpleFileOptions::default();
        writer
            .start_file("innocent.txt:hidden_stream", opts)
            .unwrap();
        writer.write_all(b"payload").unwrap();
    });

    let zip_file = temp.path().join("ads.zip");
    fs::write(&zip_file, zip_bytes).unwrap();

    let dest = temp.path().join("out");
    let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);
    assert!(res.is_err(), "ADS entry must be rejected");
}

#[test]
fn test_extractor_rejects_reserved_device_names() {
    let temp = tempdir().unwrap();
    let reserved_names = ["CON", "PRN", "AUX", "NUL", "COM1.txt", "sub/LPT9"];

    for name in reserved_names {
        let zip_bytes = create_crafted_zip(|writer| {
            let opts = SimpleFileOptions::default();
            writer.start_file(name, opts).unwrap();
            writer.write_all(b"device file payload").unwrap();
        });

        let zip_file = temp
            .path()
            .join(format!("reserved_{}.zip", name.replace(['/', '.'], "_")));
        fs::write(&zip_file, zip_bytes).unwrap();

        let dest = temp.path().join("out");
        let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);
        assert!(
            res.is_err(),
            "Reserved device name '{}' must be rejected",
            name
        );
    }
}

#[test]
fn test_extractor_rejects_case_duplicate_entries() {
    let temp = tempdir().unwrap();
    let zip_bytes = create_crafted_zip(|writer| {
        let opts = SimpleFileOptions::default();
        writer.start_file("folder/file.txt", opts).unwrap();
        writer.write_all(b"first file").unwrap();

        writer.start_file("FOLDER/FILE.TXT", opts).unwrap();
        writer.write_all(b"case colliding file").unwrap();
    });

    let zip_file = temp.path().join("case_dup.zip");
    fs::write(&zip_file, zip_bytes).unwrap();

    let dest = temp.path().join("out");
    let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);
    assert!(res.is_err(), "Case duplicate entries must be rejected");
}

#[test]
fn test_extractor_enforces_file_count_limit_20001_files() {
    let temp = tempdir().unwrap();
    let zip_bytes = create_crafted_zip(|writer| {
        let opts = SimpleFileOptions::default();
        for i in 0..20_001 {
            writer.start_file(format!("file_{i}.txt"), opts).unwrap();
            writer.write_all(b"x").unwrap();
        }
    });

    let zip_file = temp.path().join("too_many_files.zip");
    fs::write(&zip_file, zip_bytes).unwrap();

    let dest = temp.path().join("out");
    let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);

    assert!(res.is_err(), "20,001 entries must exceed file count limit");
    assert!(
        matches!(
            res.unwrap_err(),
            ExtractorError::FileCountLimitExceeded(20000)
        ),
        "Must return FileCountLimitExceeded"
    );
}

#[test]
fn test_extractor_enforces_size_limit_decompression_bomb() {
    let temp = tempdir().unwrap();
    // A tiny archive expanding beyond max_bytes
    let zip_bytes = create_crafted_zip(|writer| {
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("large.bin", opts).unwrap();
        // Write 1 MB of zeros
        let zeros = [0u8; 1024 * 1024];
        writer.write_all(&zeros).unwrap();
    });

    let zip_file = temp.path().join("bomb.zip");
    fs::write(&zip_file, zip_bytes).unwrap();

    let dest = temp.path().join("out");
    // Test with a tight limit (e.g. 512 KB)
    let tight_limits = ExtractionLimits {
        max_bytes: 512 * 1024,
        max_files: 100,
    };
    let res = extract_zip_safely(&zip_file, &dest, tight_limits, false);

    assert!(res.is_err(), "Exceeding size limit must fail");
    assert!(
        matches!(res.unwrap_err(), ExtractorError::SizeLimitExceeded { .. }),
        "Must return SizeLimitExceeded"
    );
}

#[test]
fn test_extractor_rejects_corrupted_archive() {
    let temp = tempdir().unwrap();
    let zip_file = temp.path().join("corrupt.zip");
    fs::write(&zip_file, b"NOT_A_VALID_ZIP_ARCHIVE_DATA_GARBAGE").unwrap();

    let dest = temp.path().join("out");
    let res = extract_zip_safely(&zip_file, &dest, ExtractionLimits::for_runtime(), false);

    assert!(res.is_err(), "Corrupted zip file must fail");
    assert!(
        matches!(res.unwrap_err(), ExtractorError::ZipFormatError(_)),
        "Must return ZipFormatError"
    );
}

// ============================================================================
// Downloader Tests with Local Mock Server
// ============================================================================

#[test]
fn test_downloader_mock_wrong_hash() {
    let temp = tempdir().unwrap();
    let payload = b"VERIFIED_PAYLOAD_CONTENT";
    let server = MockHttpServer::new(move |_path, _range| MockResponse::Fixed(payload.to_vec()));

    let target = temp.path().join("output.bin");
    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    let err = download_verified_file(
        &server.url("/payload"),
        &target,
        payload.len() as u64,
        "0000000000000000000000000000000000000000000000000000000000000000", // wrong hash
        DownloadKind::Runtime,
        &options,
        None,
    )
    .unwrap_err();

    assert!(matches!(err, DownloadError::HashMismatch { .. }));
    assert!(
        !target.exists(),
        "Target file must not exist on hash mismatch"
    );
    assert!(
        !temp.path().join("output.bin.part").exists(),
        ".part file must be cleaned up"
    );
}

#[test]
fn test_downloader_mock_wrong_size() {
    let temp = tempdir().unwrap();
    let payload = b"SHORT";
    let server = MockHttpServer::new(move |_path, _range| MockResponse::Fixed(payload.to_vec()));

    let target = temp.path().join("output.bin");
    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    let err = download_verified_file(
        &server.url("/payload"),
        &target,
        1000, // expected 1000 bytes, server gave 5
        "irrelevant",
        DownloadKind::Runtime,
        &options,
        None,
    )
    .unwrap_err();

    assert!(matches!(err, DownloadError::SizeMismatch { .. }));
    assert!(!target.exists());
    assert!(!temp.path().join("output.bin.part").exists());
}

#[test]
fn test_downloader_mock_redirect_to_off_list_host() {
    let temp = tempdir().unwrap();
    let server = MockHttpServer::new(move |_path, _range| {
        MockResponse::Redirect("https://unapproved-malicious-domain.com/evil.zip".to_string())
    });

    let target = temp.path().join("output.bin");
    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    let err = download_verified_file(
        &server.url("/start"),
        &target,
        100,
        "dummy",
        DownloadKind::Runtime,
        &options,
        None,
    )
    .unwrap_err();

    assert!(matches!(err, DownloadError::DisallowedHost(_)));
}

#[test]
fn test_downloader_mock_interrupted_then_resumed() {
    let temp = tempdir().unwrap();
    let payload = b"0123456789ABCDEF0123456789ABCDEF"; // 32 bytes
    let server = MockHttpServer::new(move |_path, _range| MockResponse::Fixed(payload.to_vec()));

    let target = temp.path().join("resumable.bin");
    let part_path = temp.path().join("resumable.bin.part");

    // Pre-create a partial file with the first 16 bytes
    fs::write(&part_path, &payload[..16]).unwrap();

    let mut hasher = Sha256Hasher::new();
    hasher.update(payload);
    let expected_sha256 = hasher.finalize_hex();

    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    let downloaded = download_verified_file(
        &server.url("/full"),
        &target,
        payload.len() as u64,
        &expected_sha256,
        DownloadKind::Runtime,
        &options,
        None,
    )
    .expect("Resume download must succeed");

    assert_eq!(downloaded, target);
    assert_eq!(fs::read(&target).unwrap(), payload);
    assert!(
        !part_path.exists(),
        ".part file must be removed after successful atomic rename"
    );
}

#[test]
fn test_downloader_mock_part_left_with_different_content() {
    let temp = tempdir().unwrap();
    let payload = b"GENUINE_CONTENT_OF_THE_DOWNLOADED_FILE";
    let server = MockHttpServer::new(move |_path, _range| MockResponse::Fixed(payload.to_vec()));

    let target = temp.path().join("tampered.bin");
    let part_path = temp.path().join("tampered.bin.part");

    // Pre-create partial file with 10 bytes of CORRUPTED data
    fs::write(&part_path, b"XXXXXXXXXX").unwrap();

    let mut hasher = Sha256Hasher::new();
    hasher.update(payload);
    let expected_sha256 = hasher.finalize_hex();

    let options = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    // Resuming on top of corrupted initial bytes must result in SHA-256 failure at the end
    let err = download_verified_file(
        &server.url("/corrupted_part"),
        &target,
        payload.len() as u64,
        &expected_sha256,
        DownloadKind::Runtime,
        &options,
        None,
    )
    .unwrap_err();

    assert!(matches!(err, DownloadError::HashMismatch { .. }));
    assert!(!target.exists(), "Target file must not exist");
    assert!(
        !part_path.exists(),
        "Corrupted .part file must be cleaned up on hash mismatch"
    );
}

#[test]
fn test_runtime_manifest_lookup_and_status() {
    let arch = RuntimeArch::current();
    let entry = find_approved_runtime("node", "22.11.0", arch).unwrap();
    assert_eq!(entry.version, "22.11.0");
    assert!(entry.url.starts_with("https://nodejs.org/dist/v22.11.0/"));
    assert!(!entry.sha256.is_empty());
}
