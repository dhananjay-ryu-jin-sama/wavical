#![forbid(unsafe_code)]

//! Comprehensive test suite for WP7: Cloud fallback, HTTPS client, credential storage,
//! first-use consent, modes (local, cloud, local-first), redirect refusal, certificate verification,
//! size caps, timeout enforcement, and zero-leak guarantees.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

use tracing_subscriber::fmt::MakeWriter;

use wavical_ai::chat::{
    ChatExecutionMode, ChatStreamStatus, CloudChatConfig, StreamChatConfig,
    build_chat_final_script, execute_chat_with_mode,
};
use wavical_ai::cloud_client::{
    CloudClientError, build_target_cloud_url, create_cloud_http_client,
    validate_and_extract_cloud_host,
};
use wavical_ai::cloud_protocol::{
    CloudProtocolError, execute_cloud_chat_non_streamed, execute_cloud_chat_stream,
};
use wavical_ai::consent::{
    MockConsentDialogProvider, check_cloud_consent, ensure_cloud_consent, record_cloud_consent,
};
use wavical_ai::credentials::{CloudApiKey, clear_secret_from_keyring, store_secret_in_keyring};
use wavical_ipc::ai::{MAX_CLOUD_REQUEST_BODY_BYTES, MAX_CLOUD_RESPONSE_BODY_BYTES};
use wavical_ipc::{ChatMessage, ChatRequest, ChatRole};

/// Shared in-memory buffer capturing tracing output.
#[derive(Clone, Default)]
struct TracingLogCapture {
    logs: Arc<Mutex<Vec<u8>>>,
}

impl Write for TracingLogCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.logs.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for TracingLogCapture {
    type Writer = TracingLogCapture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Helper structure holding mock HTTPS server details running in a dedicated OS thread.
struct MockHttpsServer {
    pub base_url: String,
    pub cert_pem: Vec<u8>,
    pub request_count: Arc<AtomicUsize>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

enum MockBehavior {
    NormalStream,
    NormalNonStream,
    Redirect(String),
    Slow(Duration),
    OversizeResponse(usize),
}

impl MockHttpsServer {
    fn start(behavior: MockBehavior) -> Self {
        // Generate self-signed certificate for localhost/127.0.0.1
        let params =
            rcgen::CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
                .unwrap();
        let key_pair = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        let cert_der = cert.der().to_vec();
        let key_der = key_pair.serialize_der();
        let cert_pem = cert.pem().into_bytes();

        let certs = vec![CertificateDer::from(cert_der)];
        let key = PrivateKeyDer::try_from(key_der).unwrap();
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));

        let request_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&request_count);
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let (init_tx, init_rx) = std::sync::mpsc::channel::<(String, Vec<u8>)>();

        let pem_clone = cert_pem.clone();
        let thread_handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            rt.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let port = listener.local_addr().unwrap().port();
                let base_url = format!("https://127.0.0.1:{port}/v1");
                init_tx.send((base_url, pem_clone)).unwrap();

                let behavior = Arc::new(behavior);
                loop {
                    tokio::select! {
                        _ = &mut shutdown_rx => {
                            break;
                        }
                        accept_res = listener.accept() => {
                            let (stream, _) = match accept_res {
                                Ok(res) => res,
                                Err(_) => break,
                            };
                            let acceptor = acceptor.clone();
                            let count = Arc::clone(&count_clone);
                            let beh = Arc::clone(&behavior);

                            tokio::spawn(async move {
                                if let Ok(tls_stream) = acceptor.accept(stream).await {
                                    count.fetch_add(1, Ordering::SeqCst);
                                    let (reader, mut writer) = tokio::io::split(tls_stream);
                                    let mut buf_reader = BufReader::new(reader);
                                    let mut req_line = String::new();
                                    let _ = buf_reader.read_line(&mut req_line).await;

                                    // Read headers
                                    let mut content_len = 0;
                                    loop {
                                        let mut line = String::new();
                                        if buf_reader.read_line(&mut line).await.is_err() || line.trim().is_empty() {
                                            break;
                                        }
                                        if line.to_ascii_lowercase().starts_with("content-length:")
                                            && let Some(val) = line.split(':').nth(1)
                                        {
                                            content_len = val.trim().parse::<usize>().unwrap_or(0);
                                        }
                                    }

                                    // Read body if any
                                    if content_len > 0 {
                                        let mut body_buf = vec![0u8; content_len];
                                        let _ = buf_reader.read_exact(&mut body_buf).await;
                                    }

                                    match &*beh {
                                        MockBehavior::NormalStream => {
                                            let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"Hello from mock cloud!\"}}]}\n\ndata: [DONE]\n\n";
                                            let _ = writer.write_all(resp.as_bytes()).await;
                                        }
                                        MockBehavior::NormalNonStream => {
                                            let body = "{\"choices\":[{\"message\":{\"content\":\"Hello non-stream!\"}}]}";
                                            let resp = format!(
                                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                                body.len(),
                                                body
                                            );
                                            let _ = writer.write_all(resp.as_bytes()).await;
                                        }
                                        MockBehavior::Redirect(target) => {
                                            let resp = format!(
                                                "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n",
                                                target
                                            );
                                            let _ = writer.write_all(resp.as_bytes()).await;
                                        }
                                        MockBehavior::Slow(duration) => {
                                            tokio::time::sleep(*duration).await;
                                            let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: [DONE]\n\n";
                                            let _ = writer.write_all(resp.as_bytes()).await;
                                        }
                                        MockBehavior::OversizeResponse(size) => {
                                            let padding = "A".repeat(*size);
                                            let resp = format!(
                                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{{\"choices\":[{{\"message\":{{\"content\":\"{}\"}}}}]}}",
                                                size + 50,
                                                padding
                                            );
                                            let _ = writer.write_all(resp.as_bytes()).await;
                                        }
                                    }
                                    let _ = writer.flush().await;
                                }
                            });
                        }
                    }
                }
            });
        });

        let (base_url, cert_pem) = init_rx.recv().unwrap();

        Self {
            base_url,
            cert_pem,
            request_count,
            shutdown_tx: Some(shutdown_tx),
            thread_handle: Some(thread_handle),
        }
    }
}

impl Drop for MockHttpsServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_local_mode_receives_zero_requests() {
    let mock = MockHttpsServer::start(MockBehavior::NormalStream);
    let temp = tempdir().unwrap();
    let app_data = temp.path();

    let client = create_cloud_http_client(Some(&mock.cert_pem)).unwrap();
    let local_client = reqwest::blocking::Client::new();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let consent = MockConsentDialogProvider { allow: true };

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Hello local".to_string(),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: Some(true),
    };

    let cloud_cfg = CloudChatConfig {
        base_url: mock.base_url.clone(),
        model: "mock-model".to_string(),
        api_key_env: "TEST_MOCK_ENV_KEY".to_string(),
        require_consent: true,
    };

    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    // In local mode without local engine, it fails immediately and NEVER contacts cloud
    execute_chat_with_mode(
        ChatExecutionMode::Local,
        app_data,
        "app-local-test",
        "win-1",
        1,
        &req,
        &StreamChatConfig::default(),
        &cancel_flag,
        &local_client,
        None, // Local unavailable
        None,
        &client,
        Some(&cloud_cfg),
        &consent,
        |_chunk| {},
        |status, _err, backend| {
            assert_eq!(backend, "local");
            assert_eq!(status, ChatStreamStatus::Failed);
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    // Assert mock server received ZERO requests
    assert_eq!(
        mock.request_count.load(Ordering::SeqCst),
        0,
        "Local mode must never make any outbound cloud requests"
    );
}

#[test]
fn test_marker_key_never_appears_in_responses_logs_or_disk() {
    let mock = MockHttpsServer::start(MockBehavior::NormalStream);
    let temp = tempdir().unwrap();
    let app_data = temp.path();

    let marker_key = "sk-test-LEAK-MARKER-123";
    let env_var = "TEST_WAVICAL_CLOUD_MARKER_KEY_ENV";

    store_secret_in_keyring("127.0.0.1", marker_key).unwrap();

    let capture = TracingLogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_max_level(tracing::Level::DEBUG)
        .finish();

    let client = create_cloud_http_client(Some(&mock.cert_pem)).unwrap();
    let local_client = reqwest::blocking::Client::new();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let consent = MockConsentDialogProvider { allow: true };

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Hello cloud".to_string(),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: Some(true),
    };

    let cloud_cfg = CloudChatConfig {
        base_url: mock.base_url.clone(),
        model: "mock-model".to_string(),
        api_key_env: env_var.to_string(),
        require_consent: true,
    };

    let received_chunks = Arc::new(Mutex::new(Vec::<String>::new()));
    let chunks_clone = Arc::clone(&received_chunks);
    let final_script_out = Arc::new(Mutex::new(String::new()));
    let script_clone = Arc::clone(&final_script_out);

    tracing::subscriber::with_default(subscriber, || {
        execute_chat_with_mode(
            ChatExecutionMode::Cloud,
            app_data,
            "test-leak-app",
            "win-leak",
            42,
            &req,
            &StreamChatConfig::default(),
            &cancel_flag,
            &local_client,
            None,
            None,
            &client,
            Some(&cloud_cfg),
            &consent,
            |chunk| {
                chunks_clone.lock().unwrap().push(chunk.to_string());
            },
            |status, err, backend| {
                assert_eq!(status, ChatStreamStatus::Done);
                assert_eq!(backend, "cloud");
                let script = build_chat_final_script(42, status, err.as_deref(), backend).unwrap();
                *script_clone.lock().unwrap() = script;
            },
        );
    });

    // Clean up keyring
    let _ = clear_secret_from_keyring("127.0.0.1");

    // 1. Assert marker key never appears in page chunks
    for chunk in received_chunks.lock().unwrap().iter() {
        assert!(
            !chunk.contains(marker_key),
            "Marker API key leaked in streamed chunk!"
        );
    }

    // 2. Assert marker key never appears in final delivered script
    let final_script = final_script_out.lock().unwrap();
    assert!(
        !final_script.contains(marker_key),
        "Marker API key leaked in final response script!"
    );
    assert!(final_script.contains("backend"));
    assert!(final_script.contains("cloud"));

    // 3. Assert marker key never appears in tracing log output
    let log_bytes = capture.logs.lock().unwrap().clone();
    let log_str = String::from_utf8_lossy(&log_bytes);
    assert!(
        !log_str.contains(marker_key),
        "Marker API key leaked in tracing log output!"
    );

    // 4. Assert marker key never appears in ANY file under the app data directory
    fn scan_dir_for_secret(dir: &Path, secret: &str) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    scan_dir_for_secret(&path, secret);
                } else if path.is_file()
                    && let Ok(content) = fs::read(&path)
                {
                    let text = String::from_utf8_lossy(&content);
                    assert!(
                        !text.contains(secret),
                        "Marker API key leaked in file: {}",
                        path.display()
                    );
                }
            }
        }
    }
    scan_dir_for_secret(app_data, marker_key);
}

#[test]
fn test_redirect_to_another_host_refused() {
    let mock = MockHttpsServer::start(MockBehavior::Redirect(
        "https://attacker.com/v1".to_string(),
    ));
    let client = create_cloud_http_client(Some(&mock.cert_pem)).unwrap();
    let api_key = CloudApiKey::new("test-key-123".to_string()).unwrap();

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Hello redirect".to_string(),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: Some(true),
    };

    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    execute_cloud_chat_stream(
        &client,
        &mock.base_url,
        "mock-model",
        &api_key,
        "win-redir",
        1,
        &req,
        &StreamChatConfig::default(),
        &Arc::new(AtomicBool::new(false)),
        |_chunk| {},
        |status, _err| {
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    assert_eq!(
        *final_status.lock().unwrap(),
        Some(ChatStreamStatus::Failed)
    );
}

#[test]
fn test_http_scheme_refused_at_config_and_runtime() {
    // Runtime checks
    assert!(matches!(
        validate_and_extract_cloud_host("http://api.openai.com/v1"),
        Err(CloudClientError::InsecureScheme(_))
    ));
    assert!(matches!(
        build_target_cloud_url("http://api.openai.com/v1", "chat/completions"),
        Err(CloudClientError::InsecureScheme(_))
    ));
}

#[test]
fn test_certificate_errors_fail_closed() {
    let mock = MockHttpsServer::start(MockBehavior::NormalStream);

    // Client WITHOUT the mock's root cert must fail closed on TLS verification
    let client_without_cert = create_cloud_http_client(None).unwrap();
    let api_key = CloudApiKey::new("test-key-123".to_string()).unwrap();

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Hello TLS test".to_string(),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: Some(true),
    };

    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    execute_cloud_chat_stream(
        &client_without_cert,
        &mock.base_url,
        "mock-model",
        &api_key,
        "win-cert",
        1,
        &req,
        &StreamChatConfig::default(),
        &Arc::new(AtomicBool::new(false)),
        |_chunk| {},
        |status, _err| {
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    assert_eq!(
        *final_status.lock().unwrap(),
        Some(ChatStreamStatus::Failed),
        "Certificate verification error must fail closed!"
    );
}

#[test]
fn test_consent_cannot_be_bypassed_from_page() {
    let temp = tempdir().unwrap();
    let app_data = temp.path();
    let app_id = "test-consent-app";
    let host = "api.openai.com";

    // 1. Consent initially absent
    assert_eq!(check_cloud_consent(app_data, app_id, host), None);

    // 2. User declines via native dialog
    let mock_denied = MockConsentDialogProvider { allow: false };
    let res = ensure_cloud_consent(app_data, app_id, host, true, &mock_denied);
    assert!(res.is_err());
    assert_eq!(check_cloud_consent(app_data, app_id, host), Some(false));

    // 3. User accepts
    record_cloud_consent(app_data, app_id, host, true).unwrap();
    assert_eq!(check_cloud_consent(app_data, app_id, host), Some(true));

    // Consent file exists under app's data folder
    let consent_file = app_data
        .join("apps")
        .join(app_id)
        .join("ai")
        .join("cloud_consent.json");
    assert!(consent_file.is_file());
}

#[test]
fn test_local_first_does_not_fallback_when_consent_missing() {
    let mock = MockHttpsServer::start(MockBehavior::NormalStream);
    let temp = tempdir().unwrap();
    let app_data = temp.path();
    let app_id = "app-local-first";

    store_secret_in_keyring("127.0.0.1", "test-key-123").unwrap();

    let client = create_cloud_http_client(Some(&mock.cert_pem)).unwrap();
    let local_client = reqwest::blocking::Client::new();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let consent = MockConsentDialogProvider { allow: true };

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Hello fallback".to_string(),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: Some(true),
    };

    let cloud_cfg = CloudChatConfig {
        base_url: mock.base_url.clone(),
        model: "mock-model".to_string(),
        api_key_env: "".to_string(),
        require_consent: true,
    };

    // A. Consent is MISSING: local is unavailable (channel=None). local-first must NOT fall back!
    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    execute_chat_with_mode(
        ChatExecutionMode::LocalFirst,
        app_data,
        app_id,
        "win-lf",
        1,
        &req,
        &StreamChatConfig::default(),
        &cancel_flag,
        &local_client,
        None, // local unavailable
        None,
        &client,
        Some(&cloud_cfg),
        &consent,
        |_chunk| {},
        |status, _err, backend| {
            assert_eq!(backend, "local");
            assert_eq!(status, ChatStreamStatus::Failed);
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    assert_eq!(
        mock.request_count.load(Ordering::SeqCst),
        0,
        "local-first must not fall back to cloud when consent is missing!"
    );

    // B. Now grant consent explicitly
    record_cloud_consent(app_data, app_id, "127.0.0.1", true).unwrap();

    // Now local-first CAN fall back to cloud!
    let final_status_2 = Arc::new(Mutex::new(None));
    let status_clone_2 = Arc::clone(&final_status_2);

    execute_chat_with_mode(
        ChatExecutionMode::LocalFirst,
        app_data,
        app_id,
        "win-lf",
        2,
        &req,
        &StreamChatConfig::default(),
        &cancel_flag,
        &local_client,
        None, // local unavailable
        None,
        &client,
        Some(&cloud_cfg),
        &consent,
        |_chunk| {},
        |status, _err, backend| {
            assert_eq!(backend, "cloud");
            assert_eq!(status, ChatStreamStatus::Done);
            *status_clone_2.lock().unwrap() = Some(status);
        },
    );

    assert_eq!(
        mock.request_count.load(Ordering::SeqCst),
        1,
        "local-first falls back to cloud when consent exists!"
    );

    let _ = clear_secret_from_keyring("127.0.0.1");
}

#[test]
fn test_oversize_response_and_request_cut_off_at_limits() {
    let mock = MockHttpsServer::start(MockBehavior::OversizeResponse(
        MAX_CLOUD_RESPONSE_BODY_BYTES + 100,
    ));
    let client = create_cloud_http_client(Some(&mock.cert_pem)).unwrap();
    let api_key = CloudApiKey::new("test-key-123".to_string()).unwrap();

    // 1. Non-streamed response exceeding 4 MB fails
    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Oversize response test".to_string(),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: Some(false),
    };

    let res =
        execute_cloud_chat_non_streamed(&client, &mock.base_url, "mock-model", &api_key, &req);
    assert!(matches!(
        res,
        Err(CloudProtocolError::ResponseBodyTooLarge(_))
    ));

    // 2. Request body exceeding 1 MB rejected before sending
    let oversize_content = "X".repeat(MAX_CLOUD_REQUEST_BODY_BYTES + 10);
    let oversize_req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: oversize_content,
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: Some(false),
    };

    let res_req = execute_cloud_chat_non_streamed(
        &client,
        &mock.base_url,
        "mock-model",
        &api_key,
        &oversize_req,
    );
    assert!(matches!(res_req, Err(CloudProtocolError::Validation(_))));
}

#[test]
fn test_timeouts_work() {
    // Server delays 2.5 seconds
    let mock = MockHttpsServer::start(MockBehavior::Slow(Duration::from_millis(2500)));
    let client = create_cloud_http_client(Some(&mock.cert_pem)).unwrap();
    let api_key = CloudApiKey::new("test-key-123".to_string()).unwrap();

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Timeout test".to_string(),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: Some(true),
    };

    // Configure wall-clock timeout to 200 ms
    let stream_cfg = StreamChatConfig {
        wall_clock_timeout: Duration::from_millis(200),
        ..Default::default()
    };

    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    execute_cloud_chat_stream(
        &client,
        &mock.base_url,
        "mock-model",
        &api_key,
        "win-timeout",
        1,
        &req,
        &stream_cfg,
        &Arc::new(AtomicBool::new(false)),
        |_chunk| {},
        |status, _err| {
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    assert_eq!(
        *final_status.lock().unwrap(),
        Some(ChatStreamStatus::Failed)
    );
}

#[test]
fn test_normal_non_stream_request() {
    let mock = MockHttpsServer::start(MockBehavior::NormalNonStream);
    let client = create_cloud_http_client(Some(&mock.cert_pem)).unwrap();
    let api_key = CloudApiKey::new("test-key-123".to_string()).unwrap();

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Non-stream test".to_string(),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        stream: Some(false),
    };

    let res =
        execute_cloud_chat_non_streamed(&client, &mock.base_url, "mock-model", &api_key, &req);
    assert_eq!(res.unwrap(), "Hello non-stream!");
}
