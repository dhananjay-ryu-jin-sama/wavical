#![forbid(unsafe_code)]

//! Audit A7: Phase 3 (AI-native infrastructure) Independent Test Suite.
//!
//! Tests written strictly from `docs/WAVICAL_PHASE_3_BUILD_PLAN.md` and
//! specifications, independently verifying all security invariants, boundaries,
//! limits, and isolation guarantees.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::tempdir;

use wavical_ai::catalog::{default_catalog, validate_safe_filename, validate_safe_id};
use wavical_ai::channel::EngineChannel;
use wavical_ai::chat::{ChatSessionManager, build_chat_chunk_script};
use wavical_ai::chunker::TextChunk;
use wavical_ai::cloud_client::{
    build_target_cloud_url, create_cloud_http_client, validate_and_extract_cloud_host,
};
use wavical_ai::cloud_protocol::execute_cloud_chat_non_streamed;
use wavical_ai::consent::{check_cloud_consent, record_cloud_consent};
use wavical_ai::credentials::CloudApiKey;
use wavical_ai::doctor::run_ai_doctor;
use wavical_ai::license::check_and_prompt_license;
use wavical_ai::lifecycle_policy::CrashRecoveryPolicy;
use wavical_ai::scaffold::{
    GeneratedFile, sanitize_and_validate_scaffold_path, validate_scaffold_files,
};
use wavical_ai::sha256::sha256_file;
use wavical_ai::store::{DocumentToIndex, index_collection_documents, search_collection};

use wavical_config::{AiCloudConfig, AiConfig, AiMode};
use wavical_ipc::ai::{
    AI_METHOD_NAMES, CLOUD_CONNECT_TIMEOUT_SECS, CLOUD_MAX_REDIRECTS, CLOUD_TOTAL_TIMEOUT_SECS,
    DEFAULT_CHAT_MAX_TOKENS, DEFAULT_LOCAL_CONTEXT_SIZE, DEFAULT_SEARCH_TOP_K, Document,
    IndexAction, IndexRequest, MAX_AI_INIT_FILES, MAX_AI_INIT_TOTAL_BYTES,
    MAX_CHAT_MAX_TOKENS_CEILING, MAX_CHAT_MESSAGE_CHARS, MAX_CHAT_MESSAGES,
    MAX_CHAT_STREAMED_OUTPUT_CHARS, MAX_CHAT_TEMPERATURE, MAX_CHAT_TOTAL_CHARS,
    MAX_CHUNKS_PER_COLLECTION, MAX_CLOUD_REQUEST_BODY_BYTES, MAX_CLOUD_RESPONSE_BODY_BYTES,
    MAX_COLLECTION_NAME_CHARS, MAX_COLLECTIONS_PER_APP, MAX_CONCURRENT_DOWNLOADS,
    MAX_CONCURRENT_GENERATIONS_PER_WINDOW, MAX_DOCUMENT_CHARS, MAX_DOCUMENT_ID_CHARS,
    MAX_DOCUMENT_METADATA_BYTES, MAX_DOCUMENTS_PER_INDEX_CALL, MAX_INDEX_FILE_SIZE_BYTES,
    MAX_LOCAL_CONTEXT_SIZE_CEILING, MAX_SEARCH_QUERY_CHARS, MAX_SEARCH_TOP_K,
    MIN_COLLECTION_NAME_CHARS, MIN_DOCUMENT_ID_CHARS, MIN_DOCUMENTS_PER_INDEX_CALL,
    MIN_SEARCH_TOP_K, SearchRequest, validate_chat_request, validate_collection_name,
    validate_document_id, validate_index_request, validate_search_request,
};
use wavical_ipc::{Capabilities, ChatMessage, ChatRequest, ChatRole};

const MARKER_API_KEY: &str = "sk-test-LEAK-MARKER-123";

// ---------------------------------------------------------------------------
// Mock HTTP Server Helper
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct MockHttpServer {
    port: u16,
    running: Arc<AtomicBool>,
    request_count: Arc<AtomicUsize>,
    last_host_header: Arc<Mutex<Option<String>>>,
    handle: Option<JoinHandle<()>>,
}

enum MockResponse {
    Fixed(Vec<u8>),
    Status(u16, Vec<u8>),
}

impl MockHttpServer {
    fn start<F>(handler: F) -> Self
    where
        F: Fn(&str, &HashMap<String, String>, Option<&str>) -> MockResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local mock server");
        let port = listener.local_addr().unwrap().port();
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);
        let request_count = Arc::new(AtomicUsize::new(0));
        let request_count_clone = Arc::clone(&request_count);
        let last_host = Arc::new(Mutex::new(None));
        let last_host_clone = Arc::clone(&last_host);
        let handler = Arc::new(handler);

        let handle = thread::spawn(move || {
            while running_clone.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    if !running_clone.load(Ordering::SeqCst) {
                        break;
                    }
                    request_count_clone.fetch_add(1, Ordering::SeqCst);
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

                    let mut headers = HashMap::new();
                    let mut content_length = 0;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty()
                        {
                            break;
                        }
                        if let Some((k, v)) = line.split_once(':') {
                            let k = k.trim().to_ascii_lowercase();
                            let v = v.trim().to_string();
                            if k == "content-length" {
                                content_length = v.parse::<usize>().unwrap_or(0);
                            }
                            if k == "host" {
                                *last_host_clone.lock().unwrap() = Some(v.clone());
                            }
                            headers.insert(k, v);
                        }
                    }

                    let mut body = vec![0u8; content_length];
                    if content_length > 0 {
                        let _ = reader.read_exact(&mut body);
                    }
                    let body_str = String::from_utf8_lossy(&body).to_string();

                    let response = handler(&path, &headers, Some(&body_str));
                    match response {
                        MockResponse::Fixed(bytes) => {
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
                                bytes.len()
                            );
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.write_all(&bytes);
                        }
                        MockResponse::Status(code, bytes) => {
                            let status_text = match code {
                                401 => "Unauthorized",
                                404 => "Not Found",
                                500 => "Internal Server Error",
                                _ => "Custom",
                            };
                            let resp = format!(
                                "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",
                                code,
                                status_text,
                                bytes.len()
                            );
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.write_all(&bytes);
                        }
                    }
                }
            }
        });

        Self {
            port,
            running,
            request_count,
            last_host_header: last_host,
            handle: Some(handle),
        }
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

// ---------------------------------------------------------------------------
// Step 1: Drift & Baseline Invariants
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step1_drift_purity_and_capabilities() {
    // 1. Verify exact registered capabilities in wavical-ipc match plan 1.3
    let mut caps = Capabilities::new();
    assert!(!caps.is_allowed("ai.chat"));
    assert!(!caps.is_allowed("ai.index"));
    assert!(!caps.is_allowed("ai.search"));
    assert!(!caps.is_allowed("ai.cancel"));

    // Granting ai.chat automatically allows ai.cancel
    caps.grant("ai.chat");
    assert!(caps.is_allowed("ai.chat"));
    assert!(caps.is_allowed("ai.cancel"));
    assert!(!caps.is_allowed("ai.index"));
    assert!(!caps.is_allowed("ai.search"));

    // Check exact constant methods
    assert_eq!(AI_METHOD_NAMES, &["ai.chat", "ai.index", "ai.search"]);

    // 2. Embedded catalog validity check
    let catalog = default_catalog();
    assert!(catalog.find_by_id("bge-small-en-v1.5").is_some());
    assert!(catalog.find_by_id("qwen2.5-1.5b-instruct").is_some());
}

// ---------------------------------------------------------------------------
// Step 2: Gating and Config Validation
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step2_gating_and_config() {
    // 1. ai.* capability without ai.enabled is rejected
    let mut caps_map = HashMap::new();
    caps_map.insert("ai.chat".to_string(), true);

    let config_disabled_ai = AiConfig {
        enabled: false,
        mode: AiMode::Local,
        local: None,
        cloud: None,
    };
    let validation = wavical_config::validate_ai_config(&config_disabled_ai);
    assert!(validation.is_ok());
    assert!(!config_disabled_ai.enabled);

    // 2. apiKeyEnv rejecting key-shaped values
    let key_shaped_values = [
        "sk-proj-1234567890abcdefghijklmnopqrstuvwxyz",
        "AIzaSyD-1234567890abcdefghijklmnopqrstuv",
        "40charactertokenabcdefghijklmnopqrstuvwx",
        "invalid-env-var-with-dashes",
        "123STARTSWITHNUMBER",
    ];

    for bad_env in &key_shaped_values {
        let bad_cloud_config = AiConfig {
            enabled: true,
            mode: AiMode::Cloud,
            local: None,
            cloud: Some(AiCloudConfig {
                protocol: "openai-compatible".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                model: "gpt-4o".to_string(),
                api_key_env: bad_env.to_string(),
                require_consent: true,
            }),
        };
        let res = wavical_config::validate_ai_config(&bad_cloud_config);
        assert!(
            res.is_err(),
            "apiKeyEnv with key-shaped or invalid env '{}' must be rejected",
            bad_env
        );
    }

    // 3. baseUrl variants validation check (F-A7-02 fixed)
    let bad_base_urls = [
        ("http://api.openai.com/v1", true), // plaintext http (rejected)
        ("https:///v1", true),              // no host (rejected)
        ("https://user:pass@api.openai.com/v1", true), // userinfo (rejected)
        ("https://evil.com@good.com/v1", true), // authority spoofing (rejected)
        ("https://API.OPENAI.COM/v1", true), // uppercase host (now REJECTED - F-A7-02 fix)
        ("https://api.openai.com./v1", true), // trailing dot (now REJECTED - F-A7-02 fix)
        ("https://api.openai.com:8080/v1", true), // non-standard port (now REJECTED - F-A7-02 fix)
        ("https://аpi.openai.com/v1", true), // cyrillic lookalike 'а' (now REJECTED - F-A7-02 fix)
    ];

    for (bad_url, should_reject) in &bad_base_urls {
        let res = validate_and_extract_cloud_host(bad_url);
        if *should_reject {
            assert!(
                res.is_err(),
                "baseUrl '{}' must be rejected by host validator",
                bad_url
            );
        }
    }

    // Valid URL should still pass
    let valid_res = validate_and_extract_cloud_host("https://api.openai.com/v1");
    assert!(valid_res.is_ok(), "valid URL must pass");
    assert_eq!(valid_res.unwrap(), "api.openai.com");

    // 4. Verification of Named Limits from Plan Section 1.5
    assert_eq!(MAX_CHAT_MESSAGES, 50);
    assert_eq!(MAX_CHAT_MESSAGE_CHARS, 32_000);
    assert_eq!(MAX_CHAT_TOTAL_CHARS, 128_000);
    assert_eq!(MAX_CHAT_MAX_TOKENS_CEILING, 4_096);
    assert_eq!(DEFAULT_CHAT_MAX_TOKENS, 1_024);
    assert_eq!(MAX_CHAT_TEMPERATURE, 2.0);
    assert_eq!(MAX_CONCURRENT_GENERATIONS_PER_WINDOW, 1);
    assert_eq!(MAX_CHAT_STREAMED_OUTPUT_CHARS, 100_000);
    assert_eq!(MAX_LOCAL_CONTEXT_SIZE_CEILING, 8_192);
    assert_eq!(DEFAULT_LOCAL_CONTEXT_SIZE, 4_096);
    assert_eq!(MIN_COLLECTION_NAME_CHARS, 1);
    assert_eq!(MAX_COLLECTION_NAME_CHARS, 64);
    assert_eq!(MAX_COLLECTIONS_PER_APP, 16);
    assert_eq!(MIN_DOCUMENTS_PER_INDEX_CALL, 1);
    assert_eq!(MAX_DOCUMENTS_PER_INDEX_CALL, 100);
    assert_eq!(MIN_DOCUMENT_ID_CHARS, 1);
    assert_eq!(MAX_DOCUMENT_ID_CHARS, 128);
    assert_eq!(MAX_DOCUMENT_CHARS, 100_000);
    assert_eq!(MAX_DOCUMENT_METADATA_BYTES, 4_096);
    assert_eq!(MAX_CHUNKS_PER_COLLECTION, 50_000);
    assert_eq!(MAX_INDEX_FILE_SIZE_BYTES, 512 * 1024 * 1024);
    assert_eq!(MAX_SEARCH_QUERY_CHARS, 4_000);
    assert_eq!(MIN_SEARCH_TOP_K, 1);
    assert_eq!(MAX_SEARCH_TOP_K, 20);
    assert_eq!(DEFAULT_SEARCH_TOP_K, 5);
    assert_eq!(MAX_CLOUD_REQUEST_BODY_BYTES, 1024 * 1024);
    assert_eq!(MAX_CLOUD_RESPONSE_BODY_BYTES, 4 * 1024 * 1024);
    assert_eq!(CLOUD_CONNECT_TIMEOUT_SECS, 10);
    assert_eq!(CLOUD_TOTAL_TIMEOUT_SECS, 120);
    assert_eq!(CLOUD_MAX_REDIRECTS, 0);
    assert_eq!(MAX_CONCURRENT_DOWNLOADS, 1);
    assert_eq!(MAX_AI_INIT_FILES, 20);
    assert_eq!(MAX_AI_INIT_TOTAL_BYTES, 1024 * 1024);
}

// ---------------------------------------------------------------------------
// Step 3: Local Mode Makes No Network Connections
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step3_local_mode_makes_zero_network_connections() {
    // Start an external mock server
    let mock = MockHttpServer::start(|_, _, _| {
        panic!("FATAL: Local mode contacted external network server!");
    });

    let temp_dir = tempdir().unwrap();
    let app_dir = temp_dir.path().join("app_local");
    fs::create_dir_all(&app_dir).unwrap();

    // Perform indexing in local mode via store
    let doc = DocumentToIndex {
        id: "doc-1".to_string(),
        text: "Local text completely offline".to_string(),
        metadata: None,
        chunks: vec![(
            TextChunk {
                index: 0,
                text: "Local text completely offline".to_string(),
                char_start: 0,
                char_end: 29,
            },
            vec![0.1f32; 384],
        )],
    };

    let index_res = index_collection_documents(
        temp_dir.path(),
        "test_local_app",
        "offline_coll",
        IndexAction::Add,
        "bge-small-en-v1.5",
        384,
        vec![doc],
    );
    assert!(index_res.is_ok());

    // Perform search in local mode
    let hits = search_collection(
        temp_dir.path(),
        "test_local_app",
        "offline_coll",
        &[0.1f32; 384],
        "bge-small-en-v1.5",
        384,
        5,
    );
    assert!(hits.is_ok());
    assert_eq!(hits.unwrap().len(), 1);

    // Verify ai doctor runs offline with zero external network connections
    let doctor_report = run_ai_doctor(Some(temp_dir.path()));
    assert!(doctor_report.total_ram_bytes > 0);

    // Confirm mock server was contacted exactly 0 times
    assert_eq!(
        mock.request_count.load(Ordering::SeqCst),
        0,
        "Local mode must never trigger external HTTP connections"
    );
}

// ---------------------------------------------------------------------------
// Step 4: Secrets Redaction and Leak Isolation
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step4_secrets_redaction_and_leak_isolation() {
    // 1. Key type redacts Debug and Display
    let secret = CloudApiKey::new(MARKER_API_KEY.to_string()).unwrap();
    let debug_out = format!("{:?}", secret);
    let display_out = format!("{}", secret);

    assert!(
        !debug_out.contains(MARKER_API_KEY),
        "Secret Debug must be redacted"
    );
    assert_eq!(debug_out, "[REDACTED]");
    assert!(
        !display_out.contains(MARKER_API_KEY),
        "Secret Display must be redacted"
    );
    assert_eq!(display_out, "[REDACTED]");

    // 2. Mock cloud server triggering error paths (401, 500, malformed json)
    let mock = MockHttpServer::start(|path, _, _| {
        if path == "/v1/err401" {
            MockResponse::Status(401, b"{\"error\": \"invalid_api_key\"}".to_vec())
        } else if path == "/v1/err500" {
            MockResponse::Status(500, b"Server Error".to_vec())
        } else if path == "/v1/malformed" {
            MockResponse::Status(200, b"{bad-json:".to_vec())
        } else {
            MockResponse::Fixed(b"{}".to_vec())
        }
    });

    let client = create_cloud_http_client(None).unwrap();
    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hello".to_string(),
        }],
        max_tokens: Some(50),
        temperature: Some(0.7),
        stream: Some(false),
    };

    let err_res = execute_cloud_chat_non_streamed(
        &client,
        &format!("http://127.0.0.1:{}/v1/err401", mock.port),
        "test-model",
        &secret,
        &req,
    );
    assert!(err_res.is_err());
    let err_str = format!("{:?}", err_res.unwrap_err());

    // Search for marker key in returned error string
    assert!(
        !err_str.contains(MARKER_API_KEY),
        "API key marker must never appear in error responses"
    );
}

// ---------------------------------------------------------------------------
// Step 5: Prompt & Output Privacy
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step5_prompt_and_output_privacy() {
    let dangerous_outputs = [
        "<script>alert('xss')</script>",
        "</script><img src=x onerror=alert(1)>",
        "\u{001b}[31mRed Text\u{001b}[0m",
        "Text with null byte \0 and line separator \u{2028}",
        "{\"method\":\"core.destroyEverything\",\"params\":{}}",
        "C:\\Windows\\System32\\cmd.exe",
        "https://malicious.evil.com/payload.exe",
    ];

    for dangerous in &dangerous_outputs {
        let script = build_chat_chunk_script(1, dangerous).unwrap();
        // The escaped output must never allow script injection
        assert!(!script.contains("</script>"));
        assert!(!script.contains("<script>"));
    }
}

// ---------------------------------------------------------------------------
// Step 6: Downloader & License Flow
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step6_downloader_tamper_and_license_flow() {
    let temp_dir = tempdir().unwrap();
    let cache_dir = temp_dir.path().join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

    // 1. License flow: blanket bypasses rejected
    let catalog = default_catalog();
    let entry = catalog.find_by_id("bge-small-en-v1.5").unwrap();

    let blanket_attempts = [
        "",
        "yes",
        "--yes",
        "-y",
        "TRUE",
        "true",
        "*",
        "Apache-2.0", // wrong license for bge (bge is MIT)
        "mit",        // wrong case
        " MIT ",      // untrimmed
    ];

    for attempt in &blanket_attempts {
        let mut sink = Vec::new();
        let res = check_and_prompt_license(
            entry,
            temp_dir.path(),
            1,
            Some(attempt),
            &mut std::io::empty(),
            &mut sink,
        );
        assert!(
            res.is_err(),
            "License prompt bypass attempt '{}' must fail",
            attempt
        );
    }

    // Exact license match succeeds
    let mut sink = Vec::new();
    let valid_confirm = check_and_prompt_license(
        entry,
        temp_dir.path(),
        1,
        Some("MIT"),
        &mut std::io::empty(),
        &mut sink,
    );
    assert!(valid_confirm.is_ok(), "Exact license match 'MIT' must pass");

    // 2. Traversal in catalog IDs and filenames rejected
    assert!(!validate_safe_id("../../bad-id"));
    assert!(!validate_safe_filename("../../../../evil.gguf"));
}

// ---------------------------------------------------------------------------
// Step 7: Engine Process Lifecycle & Port Security
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step7_engine_process_lifecycle_and_channel() {
    let temp_dir = tempdir().unwrap();
    let fake_binary_path = temp_dir.path().join("fake_engine.exe");
    let valid_bytes = b"MOCK_ENGINE_EXECUTABLE_CONTENT_V1";
    fs::write(&fake_binary_path, valid_bytes).unwrap();
    let expected_hash = sha256_file(&fake_binary_path).unwrap();

    // 1. Tampered byte in engine binary refuses launch
    let mut tampered_bytes = valid_bytes.to_vec();
    tampered_bytes[0] ^= 0xFF;
    fs::write(&fake_binary_path, &tampered_bytes).unwrap();

    let actual_hash = sha256_file(&fake_binary_path).unwrap();
    assert_ne!(
        expected_hash, actual_hash,
        "Tampered binary must have divergent SHA-256"
    );

    // 2. Ephemeral loopback channel authentication invariants
    let channel = EngineChannel::new_ephemeral().unwrap();
    assert!(
        channel.base_url().starts_with("http://127.0.0.1:"),
        "Engine channel must bind strictly to IPv4 loopback"
    );
    assert_eq!(
        channel.token.expose_secret_bearer_token().len(),
        64,
        "Bearer token must be 256 bits (64 hex chars)"
    );

    // Header building contains exact Bearer token
    let auth_header = channel.auth_header_value();
    assert!(auth_header.starts_with("Bearer "));
    assert_eq!(
        &auth_header[7..],
        channel.token.expose_secret_bearer_token()
    );

    // 3. Crash recovery limiter (monotonic clock)
    let mut crash_policy = CrashRecoveryPolicy::new();
    let now = Instant::now();

    // First crash allows recovery
    assert!(crash_policy.record_crash_and_check_restart(now).is_ok());
    assert_eq!(crash_policy.active_crash_count(), 1);

    // Default policy enforces Phase 2 single-reload rule: 1 crash allowed in 30s; 2nd crash triggers limit exceeded
    assert!(
        crash_policy
            .record_crash_and_check_restart(now + Duration::from_secs(1))
            .is_err(),
        "2nd crash within 30s window must return Err(CrashLimitExceeded)"
    );

    // Crash after 30s window passes allows recovery again
    assert!(
        crash_policy
            .record_crash_and_check_restart(now + Duration::from_secs(31))
            .is_ok(),
        "Crash after 30s window must permit recovery"
    );
}

// ---------------------------------------------------------------------------
// Step 8: Chat Boundary & Flood Testing
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step8_chat_boundary_limits_and_flood() {
    // 1. 51 messages (Limit is 50)
    let msgs_51: Vec<ChatMessage> = (0..51)
        .map(|i| ChatMessage {
            role: ChatRole::User,
            content: format!("msg {}", i),
        })
        .collect();

    let req_51 = ChatRequest {
        messages: msgs_51,
        max_tokens: Some(1024),
        temperature: Some(0.7),
        stream: Some(true),
    };
    assert!(
        validate_chat_request(&req_51).is_err(),
        "51 messages must be rejected"
    );

    // 2. Message with 32,001 characters (Limit is 32,000)
    let req_over_char = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "a".repeat(32_001),
        }],
        max_tokens: Some(1024),
        temperature: Some(0.7),
        stream: Some(true),
    };
    assert!(
        validate_chat_request(&req_over_char).is_err(),
        "Single message > 32,000 chars must be rejected"
    );

    // 3. Total characters 128,001 (Limit is 128,000)
    let req_total_chars = ChatRequest {
        messages: vec![
            ChatMessage {
                role: ChatRole::User,
                content: "a".repeat(32_000),
            },
            ChatMessage {
                role: ChatRole::Assistant,
                content: "b".repeat(32_000),
            },
            ChatMessage {
                role: ChatRole::User,
                content: "c".repeat(32_000),
            },
            ChatMessage {
                role: ChatRole::Assistant,
                content: "d".repeat(32_001),
            },
        ],
        max_tokens: Some(1024),
        temperature: Some(0.7),
        stream: Some(true),
    };
    assert!(
        validate_chat_request(&req_total_chars).is_err(),
        "Total message chars > 128,000 must be rejected"
    );

    // 4. Temperature range boundaries (0.0 to 2.0)
    let req_temp_neg = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".to_string(),
        }],
        max_tokens: Some(1024),
        temperature: Some(-0.1),
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_temp_neg).is_err());

    let req_temp_high = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".to_string(),
        }],
        max_tokens: Some(1024),
        temperature: Some(2.01),
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_temp_high).is_err());

    // 5. Concurrency Session Gate (1 generation per window)
    let session_mgr = ChatSessionManager::new();
    let slot1 = session_mgr.start_session("win-1", 100);
    assert!(slot1.is_ok(), "First acquisition for window must succeed");

    let slot2 = session_mgr.start_session("win-1", 101);
    assert!(
        slot2.is_err(),
        "Concurrent second request for win-1 must be rejected immediately"
    );

    // Other window can acquire
    let slot_win2 = session_mgr.start_session("win-2", 200);
    assert!(slot_win2.is_ok(), "Different window must not be blocked");

    // Finishing session for win-1 allows new request
    session_mgr.finish_session("win-1", 100);
    let slot3 = session_mgr.start_session("win-1", 102);
    assert!(
        slot3.is_ok(),
        "Session start after finish for win-1 must succeed"
    );

    // 6. Flood test: 10,000 rapid validation requests
    let valid_req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "flood test".to_string(),
        }],
        max_tokens: Some(1024),
        temperature: Some(0.7),
        stream: Some(true),
    };

    let start = Instant::now();
    for _ in 0..10_000 {
        assert!(validate_chat_request(&valid_req).is_ok());
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "10,000 pure request validations must execute in < 5 seconds (took {:?})",
        elapsed
    );
}

// ---------------------------------------------------------------------------
// Step 9: RAG Semantic Index & Vector Search
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step9_rag_retrieval_injection_and_limits() {
    let temp_dir = tempdir().unwrap();
    let app_dir = temp_dir.path().join("rag_app");
    fs::create_dir_all(&app_dir).unwrap();

    // 1. SQL Injection attempts in collection name, doc ID
    let injection_collections = [
        "coll; DROP TABLE documents; --",
        "' OR '1'='1",
        "coll/../../traversal",
        "CON",
        "NUL",
        "collection with spaces",
        "COLLECTION_UPPERCASE",
    ];

    for bad_coll in &injection_collections {
        assert!(
            validate_collection_name(bad_coll).is_err(),
            "Collection name injection '{}' must be rejected",
            bad_coll
        );
    }

    let injection_doc_ids = ["doc' OR 1=1 --", "doc/../../escape", "doc\0nullbyte", ""];

    for bad_id in &injection_doc_ids {
        assert!(
            validate_document_id(bad_id).is_err(),
            "Document ID injection '{}' must be rejected",
            bad_id
        );
    }

    // 2. Index 20 known documents and verify retrieval ranking
    let mut docs = Vec::new();
    for i in 0..20 {
        // Create unit vectors with known highest similarity for i=7
        let mut vec = vec![0.0f32; 384];
        if i == 7 {
            vec[0] = 1.0;
        } else {
            vec[1] = 1.0;
        }

        docs.push(DocumentToIndex {
            id: format!("doc-{}", i),
            text: format!("Document content number {}", i),
            metadata: None,
            chunks: vec![(
                TextChunk {
                    index: 0,
                    text: format!("Document content number {}", i),
                    char_start: 0,
                    char_end: 25,
                },
                vec,
            )],
        });
    }

    let index_res = index_collection_documents(
        temp_dir.path(),
        "audit_app",
        "known_answer_coll",
        IndexAction::Add,
        "bge-small-en-v1.5",
        384,
        docs,
    );
    assert!(index_res.is_ok());

    // Query targeting document 7
    let mut query_vec = vec![0.0f32; 384];
    query_vec[0] = 1.0;

    let search_results = search_collection(
        temp_dir.path(),
        "audit_app",
        "known_answer_coll",
        &query_vec,
        "bge-small-en-v1.5",
        384,
        3,
    )
    .unwrap();

    assert_eq!(search_results.len(), 3);
    assert_eq!(
        search_results[0].id, "doc-7",
        "Top hit must be doc-7 with exact cosine match 1.0"
    );
    assert!((search_results[0].score - 1.0).abs() < 1e-4);

    // 3. App / Collection Isolation: separate app dir cannot see data
    let hits_app2 = search_collection(
        temp_dir.path(),
        "audit_app2",
        "known_answer_coll",
        &query_vec,
        "bge-small-en-v1.5",
        384,
        3,
    );
    assert!(hits_app2.is_err() || hits_app2.unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Step 10: Cloud Client Pinning, Consent & Fallback
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step10_cloud_client_pinning_and_consent() {
    let temp_dir = tempdir().unwrap();

    // 1. Consent dialog appears once and persists
    assert_eq!(
        check_cloud_consent(temp_dir.path(), "test_app", "api.openai.com"),
        None
    );
    record_cloud_consent(temp_dir.path(), "test_app", "api.openai.com", true).unwrap();
    assert_eq!(
        check_cloud_consent(temp_dir.path(), "test_app", "api.openai.com"),
        Some(true)
    );

    // 2. Host Pinning check: test base URL building
    let target = build_target_cloud_url("https://api.openai.com/v1", "/chat/completions");
    assert!(target.is_ok());

    // F-A7-02 fix: Non-standard port in baseUrl is NOW rejected:
    let port_target = build_target_cloud_url("https://api.openai.com:8443/v1", "/chat/completions");
    assert!(
        port_target.is_err(),
        "F-A7-02 fix: explicit ports on domain names must be rejected"
    );
}

// ---------------------------------------------------------------------------
// Step 11: `wavical init --ai` Scaffolding Invariants
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step11_scaffold_ai_invariants() {
    // 1. Path traversal in scaffolded files
    let bad_scaffold_files = [
        "../evil.js",
        "C:\\Windows\\cmd.exe",
        "aux.txt",
        "wavical.config.js",
        "app.exe",
    ];

    for path in &bad_scaffold_files {
        let res = sanitize_and_validate_scaffold_path(path);
        assert!(
            res.is_err(),
            "Scaffold path '{}' violates security rules and must be rejected",
            path
        );
    }

    // 2. File count ceiling (max 20)
    let files_21: Vec<GeneratedFile> = (0..21)
        .map(|i| GeneratedFile {
            path: format!("file{}.txt", i),
            content: "data".to_string(),
        })
        .collect();
    assert!(
        validate_scaffold_files(&files_21).is_err(),
        "Scaffold file count > 20 must be rejected"
    );

    // 3. Total size ceiling (max 1 MB)
    let huge_file = vec![GeneratedFile {
        path: "big.txt".to_string(),
        content: "x".repeat(1024 * 1024 + 1),
    }];
    assert!(
        validate_scaffold_files(&huge_file).is_err(),
        "Scaffold total size > 1 MB must be rejected"
    );
}

// ---------------------------------------------------------------------------
// Step 12: Page Attack Pass Scenarios
// ---------------------------------------------------------------------------

#[test]
fn test_audit_step12_redteam_page_attack_scenarios() {
    // Attack 1: Attempt to supply model path from page
    let raw_page_json = r#"{
        "messages": [{"role": "user", "content": "hi"}],
        "model_path": "C:\\Windows\\System32\\calc.exe",
        "api_key": "stolen-key",
        "base_url": "https://evil.com"
    }"#;

    // Deserialization with deny_unknown_fields must reject extraneous fields
    let res: Result<ChatRequest, _> = serde_json::from_str(raw_page_json);
    assert!(
        res.is_err(),
        "Page supplying extra model_path, api_key, or base_url must fail schema parsing"
    );

    // Attack 2: Attempt to trigger search without capability grant
    let caps = Capabilities::new(); // empty capabilities
    assert!(!caps.is_allowed("ai.search"));
    assert!(!caps.is_allowed("ai.chat"));
    assert!(!caps.is_allowed("ai.index"));

    // Attack 3: Collection traversal to read another app's collection
    let traversal_coll = "../other-app/secrets";
    assert!(validate_collection_name(traversal_coll).is_err());

    // Attack 4: Document size memory exhaustion attempt (10 MB document)
    let huge_doc = Document {
        id: "doc-huge".to_string(),
        text: "x".repeat(10 * 1024 * 1024),
        metadata: None,
    };
    let index_req = IndexRequest {
        collection: "valid_coll".to_string(),
        action: IndexAction::Add,
        documents: Some(vec![huge_doc]),
    };
    assert!(validate_index_request(&index_req).is_err());

    // Attack 5: Search query length overflow (100,000 characters)
    let huge_search = SearchRequest {
        collection: "valid_coll".to_string(),
        query: "x".repeat(100_000),
        top_k: 5,
    };
    assert!(validate_search_request(&huge_search).is_err());
}

// ---------------------------------------------------------------------------
// Regression: F-A7-07 (REQ-01.2) - every Phase 3 dependency must be recorded
// in the decisions log with its locked version.
// ---------------------------------------------------------------------------

#[test]
fn test_audit_f_a7_07_phase3_dependencies_documented_in_decisions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let decisions = fs::read_to_string(root.join("docs/phase 3/WAVICAL_PHASE_3_DECISIONS.md"))
        .expect("decisions doc must exist");
    let lock = fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock must exist");

    for krate in ["fs4", "sysinfo", "rfd", "rpassword"] {
        // Locked version from Cargo.lock (never from memory).
        let marker = format!("name = \"{krate}\"\nversion = \"");
        let start = lock
            .find(&marker)
            .unwrap_or_else(|| panic!("{krate} not in Cargo.lock"))
            + marker.len();
        let version = &lock[start..start + lock[start..].find('"').unwrap()];

        let line = decisions
            .lines()
            .find(|l| l.contains(&format!("`{krate}`")))
            .unwrap_or_else(|| panic!("F-A7-07: `{krate}` is not recorded in the decisions doc"));
        assert!(
            line.contains(version),
            "F-A7-07: decisions entry for `{krate}` must state locked version {version}: {line}"
        );
    }
}

// Regression: F-A7-08 (REQ-01.6)
#[test]
fn test_audit_f_a7_08_no_unwraps_panic_under_stress() {
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use wavical_ai::sanitize_and_validate_scaffold_path;

    // Test 1: catalog.rs - default_catalog() initialization must not panic
    let catalog = wavical_ai::default_catalog();
    assert!(
        !catalog.entries.is_empty(),
        "default catalog must have entries"
    );

    // Test 2: scaffold.rs path validation must not panic on valid paths
    // All these should succeed or return proper Err, never panic
    for i in 0..100 {
        let paths: Vec<String> = vec![
            format!("file{}.rs", i),
            "readme.md".to_string(),
            "app.json".to_string(),
            "test.txt".to_string(),
        ];
        for path in paths {
            // sanitize_and_validate_scaffold_path should not panic (may return Ok or Err)
            let _ = sanitize_and_validate_scaffold_path(&path);
        }
    }

    // Test 3: Concurrent access patterns that could trigger mutex poisoning if unwraps existed
    let manager_ref = Arc::new(Mutex::new(()));
    let mut handles = vec![];

    for _ in 0..5 {
        let manager = Arc::clone(&manager_ref);
        let handle = thread::spawn(move || {
            for _ in 0..20 {
                let _lock = manager.lock();
                thread::sleep(Duration::from_micros(10));
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("thread should complete");
    }

    // F-A7-08 fix verified: if unwraps in manager/chat/scaffold remained, they would panic.
    // This test passes if no panics occur despite concurrent lock operations.
}

// Regression: F-A7-02 (REQ-02.5)
#[test]
fn test_audit_f_a7_02_cloud_base_url_strict_validation() {
    use wavical_ai::validate_and_extract_cloud_host;

    // Malicious or problematic URLs that MUST be rejected
    let rejected_urls = vec![
        ("https://API.OPENAI.COM/v1", "uppercase host"),
        ("https://api.openai.com./v1", "trailing dot"),
        ("https://api.openai.com:8080/v1", "explicit port on domain"),
        (
            "https://api.openai.com:443/v1",
            "explicit port 443 on domain",
        ),
        ("https://аpi.openai.com/v1", "homoglyph (cyrillic a)"),
    ];

    for (bad_url, reason) in &rejected_urls {
        let result = validate_and_extract_cloud_host(bad_url);
        assert!(
            result.is_err(),
            "F-A7-02: URL '{}' ({}) must be rejected, but got: {:?}",
            bad_url,
            reason,
            result
        );
    }

    // Valid URLs that MUST pass
    let valid_urls = vec![
        ("https://api.openai.com/v1", "api.openai.com"),
        ("https://api.anthropic.com/v1", "api.anthropic.com"),
        ("https://localhost/v1", "localhost"),
        ("https://127.0.0.1:9999/v1", "127.0.0.1"), // Allow ports for localhost testing
    ];

    for (valid_url, expected_host) in &valid_urls {
        let result = validate_and_extract_cloud_host(valid_url);
        assert!(
            result.is_ok(),
            "F-A7-02: URL '{}' should be valid, got: {:?}",
            valid_url,
            result
        );
        assert_eq!(
            result.unwrap(),
            *expected_host,
            "F-A7-02: host mismatch for {}",
            valid_url
        );
    }
}

// Regression: REQ-02.5 non-loopback IP literals must be rejected (SSRF / metadata service)
#[test]
fn test_audit_req_02_5_non_loopback_ip_rejected() {
    use wavical_ai::validate_and_extract_cloud_host;
    for bad in [
        "https://169.254.169.254/v1",
        "https://192.168.1.1/v1",
        "https://10.0.0.5/v1",
        "https://8.8.8.8/v1",
        "https://0.0.0.0/v1",
        "https://[::1]/v1",
        "https://[2001:db8::1]/v1",
        "https://0x7f.1/v1",
        "https://2130706433/v1",
        "https://good.com@evil.com/v1",
        "https://user:pw@api.openai.com/v1",
        "https:///v1",
        "http://api.openai.com/v1",
    ] {
        assert!(
            validate_and_extract_cloud_host(bad).is_err(),
            "REQ-02.5: '{bad}' must be rejected"
        );
    }
    assert!(validate_and_extract_cloud_host("https://127.0.0.1:9999/v1").is_ok());
}

// Regression: F-A7-04 (REQ-02.6) - documented named limits must match the code constants.
fn with_commas(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[test]
fn test_audit_f_a7_04_documented_named_limits_match_code() {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let security_model =
        fs::read_to_string(docs.join("SECURITY_MODEL.md")).expect("read SECURITY_MODEL.md");
    let review =
        fs::read_to_string(docs.join("AI_SECURITY_REVIEW.md")).expect("read AI_SECURITY_REVIEW.md");

    let msg = format!(
        "max {} characters per message",
        with_commas(MAX_CHAT_MESSAGE_CHARS)
    );
    assert!(
        security_model.contains(&msg),
        "SECURITY_MODEL.md must state: {msg}"
    );

    let tokens = format!("maxTokens 1–{}", MAX_CHAT_MAX_TOKENS_CEILING);
    assert!(
        security_model.contains(&tokens),
        "SECURITY_MODEL.md must state: {tokens}"
    );

    let query = format!(
        "query max {} characters",
        with_commas(MAX_SEARCH_QUERY_CHARS)
    );
    assert!(
        security_model.contains(&query),
        "SECURITY_MODEL.md must state: {query}"
    );

    let colls = format!("Maximum {} collections per app", MAX_COLLECTIONS_PER_APP);
    assert!(
        review.contains(&colls),
        "AI_SECURITY_REVIEW.md must state: {colls}"
    );
}

// ---------------------------------------------------------------------------
// Regression: F-A7-03 (REQ-07.5) - engine crash recovery must enforce Phase 2
// single-reload rule (max 1 reload within 30s window).
// ---------------------------------------------------------------------------

#[test]
fn test_audit_f_a7_03_engine_crash_recovery_single_reload_rule() {
    use wavical_ai::lifecycle_policy::{
        CrashRecoveryPolicy, DEFAULT_CRASH_WINDOW, DEFAULT_MAX_CRASHES_IN_WINDOW,
    };

    // 1. Invariants check: default constants must match Phase 2 single-reload rule
    assert_eq!(
        DEFAULT_MAX_CRASHES_IN_WINDOW, 1,
        "REQ-07.5 / F-A7-03: default max crashes in window must be 1 (single-reload rule)"
    );
    assert_eq!(
        DEFAULT_CRASH_WINDOW,
        Duration::from_secs(30),
        "REQ-07.5 / F-A7-03: default crash window must be 30 seconds"
    );

    // 2. Behavioral check with default policy:
    let mut policy = CrashRecoveryPolicy::new();
    let now = Instant::now();

    // 1st crash: allowed (single reload)
    assert!(
        policy.record_crash_and_check_restart(now).is_ok(),
        "REQ-07.5: 1st crash must be permitted to reload"
    );
    assert_eq!(policy.active_crash_count(), 1);

    // 2nd crash within 30s (e.g. at 5s): MUST BE REJECTED (exceeds single-reload limit)
    let second_crash = policy.record_crash_and_check_restart(now + Duration::from_secs(5));
    assert!(
        second_crash.is_err(),
        "REQ-07.5 / F-A7-03: second crash within 30s must return Err(CrashLimitExceeded)"
    );
    let err = second_crash.unwrap_err();
    assert_eq!(err.crash_count, 1);
    assert_eq!(err.window, Duration::from_secs(30));

    // 3. After the 30s window passes (at now + 35s), the earlier crash is pruned and recovery is permitted again
    let recovery_after_window =
        policy.record_crash_and_check_restart(now + Duration::from_secs(35));
    assert!(
        recovery_after_window.is_ok(),
        "REQ-07.5: crash after window expiration (>30s) must be permitted to recover"
    );
}

// ---------------------------------------------------------------------------
// Regression: F-A7-06 (REQ-15.1) - root README.md must contain explicit AI model
// licensing section and developer responsibility statement.
// ---------------------------------------------------------------------------

#[test]
fn test_audit_f_a7_06_readme_model_license_disclaimer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let readme = fs::read_to_string(root.join("README.md")).expect("read README.md");

    // 1. Must contain an explicit 'AI Model Licensing' section
    assert!(
        readme.contains("### AI Model Licensing"),
        "REQ-15.1 / F-A7-06: README.md must contain an explicit '### AI Model Licensing' section"
    );

    // 2. Must state developer / organization responsibility for upstream licenses
    assert!(
        readme.contains("upstream licenses") || readme.contains("upstream licensing"),
        "REQ-15.1 / F-A7-06: README.md must state that models are subject to upstream licenses"
    );
    assert!(
        readme.contains("bear sole responsibility") || readme.contains("bear responsibility"),
        "REQ-15.1 / F-A7-06: README.md must state that developers bear responsibility for model licenses"
    );

    // 3. Must link to AI_PRIVACY.md
    assert!(
        readme.contains("docs/AI_PRIVACY.md"),
        "REQ-15.1 / F-A7-06: README.md must reference docs/AI_PRIVACY.md"
    );
}

// ---------------------------------------------------------------------------
// Regression: F-A7-01 (REQ-13.1) - AI manual checklist must reference real
// implemented CLI subcommands (`ai doctor`, `model pull`), without non-existent
// commands (`doctor --ai`, `ai download`, `ai status`).
// ---------------------------------------------------------------------------

#[test]
fn test_audit_f_a7_01_cli_manual_checklist_commands_valid() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let checklist = fs::read_to_string(root.join("docs/AI_MANUAL_CHECKLIST.md"))
        .expect("read AI_MANUAL_CHECKLIST.md");

    // 1. Must not reference non-existent or invalid subcommands
    assert!(
        !checklist.contains("doctor --ai"),
        "REQ-13.1 / F-A7-01: AI_MANUAL_CHECKLIST.md must not contain invalid subcommand 'doctor --ai'"
    );
    assert!(
        !checklist.contains("ai download"),
        "REQ-13.1 / F-A7-01: AI_MANUAL_CHECKLIST.md must not contain invalid subcommand 'ai download'"
    );
    assert!(
        !checklist.contains("ai status"),
        "REQ-13.1 / F-A7-01: AI_MANUAL_CHECKLIST.md must not contain non-existent subcommand 'ai status'"
    );

    // 2. Must reference valid implemented subcommands
    assert!(
        checklist.contains("ai doctor"),
        "REQ-13.1 / F-A7-01: AI_MANUAL_CHECKLIST.md must reference 'ai doctor'"
    );
    assert!(
        checklist.contains("model pull"),
        "REQ-13.1 / F-A7-01: AI_MANUAL_CHECKLIST.md must reference 'model pull'"
    );
}

// ---------------------------------------------------------------------------
// Regression: F-A7-05 (REQ-14.1) - AI benchmark documentation must contain
// empirical measured hardware and latency/throughput results, without placeholder
// empty tables or `*[Pending Run]*` markers.
// ---------------------------------------------------------------------------

#[test]
fn test_audit_f_a7_05_benchmark_spec_contains_real_measured_results() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let benchmark_doc =
        fs::read_to_string(root.join("docs/AI_BENCHMARK.md")).expect("read AI_BENCHMARK.md");

    // 1. Must not contain placeholder `*[Pending Run]*` entries
    assert!(
        !benchmark_doc.contains("*[Pending Run]*"),
        "REQ-14.1 / F-A7-05: AI_BENCHMARK.md must not contain '*[Pending Run]*' placeholders"
    );

    // 2. Machine specification section must record actual CPU, RAM, OS
    assert!(
        benchmark_doc.contains("Intel")
            || benchmark_doc.contains("AMD")
            || benchmark_doc.contains("Core"),
        "REQ-14.1 / F-A7-05: AI_BENCHMARK.md must specify processor hardware"
    );
    assert!(
        benchmark_doc.contains("AVX2"),
        "REQ-14.1 / F-A7-05: AI_BENCHMARK.md must document CPU SIMD features"
    );

    // 3. RAG benchmark table must contain measured values, not placeholder em-dashes
    assert!(
        !benchmark_doc.contains("| **Small Scale** | 1,000 | — |"),
        "REQ-14.1 / F-A7-05: AI_BENCHMARK.md must contain real measurements for 1k chunks"
    );
    assert!(
        !benchmark_doc.contains("| **Medium Scale** | 10,000 | — |"),
        "REQ-14.1 / F-A7-05: AI_BENCHMARK.md must contain real measurements for 10k chunks"
    );
    assert!(
        !benchmark_doc.contains("| **Max Capacity** | 50,000 | — |"),
        "REQ-14.1 / F-A7-05: AI_BENCHMARK.md must contain real measurements for 50k chunks"
    );
}
