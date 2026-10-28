#![forbid(unsafe_code)]

//! Comprehensive test suite for WP5: `ai.chat` (local), streaming, concurrency limits,
//! cancellation, timeouts, engine crash mid-stream, output capping, safe delivery escaping,
//! and tracing prompt-marker redaction.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;

use wavical_ai::chat::{
    ChatError, ChatSessionManager, ChatStreamStatus, StreamChatConfig, build_chat_chunk_script,
    clamp_max_tokens, execute_chat_stream,
};
use wavical_ai::client::create_engine_http_client;
use wavical_ai::manager::LocalEngineManager;
use wavical_ai::process::EngineLaunchOptions;
use wavical_ai::sha256::sha256_file;
use wavical_ipc::ai::{
    MAX_CHAT_MAX_TOKENS_CEILING, MAX_CHAT_MESSAGE_CHARS, MAX_CHAT_MESSAGES,
    MAX_CHAT_STREAMED_OUTPUT_CHARS, MAX_CHAT_TEMPERATURE, MIN_CHAT_TEMPERATURE,
    validate_chat_request,
};
use wavical_ipc::{ChatMessage, ChatRequest, ChatRole};

/// Return the path to the compiled `fake_engine` binary.
fn get_fake_engine_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_BIN_EXE_fake_engine"));
    if !path.is_file() {
        path = PathBuf::from("target/debug/fake_engine.exe");
        if !path.is_file() {
            path = PathBuf::from("target/debug/fake_engine");
        }
    }
    assert!(
        path.is_file(),
        "fake_engine binary must exist at {}",
        path.display()
    );
    path
}

/// Helper to create a dummy model file inside a temp directory.
fn create_dummy_model_file(dir: &Path) -> PathBuf {
    let path = dir.join("qwen2.5-1.5b-instruct-q4_k_m.gguf");
    fs::write(&path, b"dummy gguf bytes for test").unwrap();
    path
}

// ============================================================================
// 1. Boundary Limits Tests (Pure Validators)
// ============================================================================

#[test]
fn test_message_count_limit_boundary() {
    // 50 messages allowed
    let valid_messages: Vec<ChatMessage> = (0..MAX_CHAT_MESSAGES)
        .map(|i| ChatMessage {
            role: ChatRole::User,
            content: format!("Message {i}"),
        })
        .collect();
    let req = ChatRequest {
        messages: valid_messages,
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };
    assert!(validate_chat_request(&req).is_ok());

    // 51 messages rejected
    let mut invalid_messages = req.messages.clone();
    invalid_messages.push(ChatMessage {
        role: ChatRole::User,
        content: "Over the limit".to_string(),
    });
    let req_over = ChatRequest {
        messages: invalid_messages,
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_over).is_err());
}

#[test]
fn test_message_char_limit_boundary() {
    // 32,000 characters allowed
    let req_ok = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "x".repeat(MAX_CHAT_MESSAGE_CHARS),
        }],
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_ok).is_ok());

    // 32,001 characters rejected
    let req_err = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "x".repeat(MAX_CHAT_MESSAGE_CHARS + 1),
        }],
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_err).is_err());
}

#[test]
fn test_total_request_chars_limit_boundary() {
    // 4 messages of 32,000 chars = 128,000 chars allowed
    let msgs: Vec<ChatMessage> = (0..4)
        .map(|_| ChatMessage {
            role: ChatRole::User,
            content: "y".repeat(32_000),
        })
        .collect();
    let req_ok = ChatRequest {
        messages: msgs,
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_ok).is_ok());

    // 128,001 characters rejected
    let mut msgs_over = req_ok.messages.clone();
    msgs_over[0].content.push('z');
    let req_over = ChatRequest {
        messages: msgs_over,
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_over).is_err());
}

#[test]
fn test_temperature_range_boundary() {
    let base_msg = vec![ChatMessage {
        role: ChatRole::User,
        content: "Hello".to_string(),
    }];

    // 0.0 allowed
    let req_min = ChatRequest {
        messages: base_msg.clone(),
        max_tokens: None,
        temperature: Some(MIN_CHAT_TEMPERATURE),
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_min).is_ok());

    // 2.0 allowed
    let req_max = ChatRequest {
        messages: base_msg.clone(),
        max_tokens: None,
        temperature: Some(MAX_CHAT_TEMPERATURE),
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_max).is_ok());

    // -0.01 rejected
    let req_neg = ChatRequest {
        messages: base_msg.clone(),
        max_tokens: None,
        temperature: Some(-0.01),
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_neg).is_err());

    // 2.01 rejected
    let req_high = ChatRequest {
        messages: base_msg.clone(),
        max_tokens: None,
        temperature: Some(2.01),
        stream: Some(true),
    };
    assert!(validate_chat_request(&req_high).is_err());
}

#[test]
fn test_max_tokens_clamping_logic() {
    assert_eq!(clamp_max_tokens(Some(10_000), 1024), 1024);
    assert_eq!(clamp_max_tokens(Some(512), 1024), 512);
    assert_eq!(clamp_max_tokens(None, 1024), 1024);
    assert_eq!(
        clamp_max_tokens(Some(8_000), 8000),
        MAX_CHAT_MAX_TOKENS_CEILING
    );
}

// ============================================================================
// 2. Concurrency & Window Isolation Tests
// ============================================================================

#[test]
fn test_concurrency_one_generation_per_window_and_isolation() {
    let session_mgr = ChatSessionManager::new();

    // Start request for window A
    let cancel_a = session_mgr
        .start_session("window_a", 1)
        .expect("window A session starts");
    assert!(session_mgr.has_active_session("window_a"));

    // Second concurrent request for window A is rejected, not queued
    let second_a = session_mgr.start_session("window_a", 2).unwrap_err();
    assert_eq!(second_a, ChatError::ConcurrencyLimit);

    // Window B can run concurrently
    let cancel_b = session_mgr
        .start_session("window_b", 10)
        .expect("window B session starts");
    assert!(session_mgr.has_active_session("window_b"));

    // Forged cancel: window B cannot cancel window A's request
    let forged_cancel = session_mgr.cancel_session("window_b", 1).unwrap_err();
    assert_eq!(forged_cancel, ChatError::UnknownOrUnauthorizedRequest);
    assert!(!cancel_a.load(Ordering::SeqCst));

    // Forged cancel: window A cannot cancel window B's request
    let forged_cancel_2 = session_mgr.cancel_session("window_a", 10).unwrap_err();
    assert_eq!(forged_cancel_2, ChatError::UnknownOrUnauthorizedRequest);
    assert!(!cancel_b.load(Ordering::SeqCst));

    // Legitimate cancel by window A
    session_mgr
        .cancel_session("window_a", 1)
        .expect("window A cancels own request");
    assert!(cancel_a.load(Ordering::SeqCst));
    assert!(!session_mgr.has_active_session("window_a"));

    // Cancel after finish fails same as unknown id
    let post_finish = session_mgr.cancel_session("window_a", 1).unwrap_err();
    assert_eq!(post_finish, ChatError::UnknownOrUnauthorizedRequest);

    // Now window A can start a new request
    let cancel_a_new = session_mgr
        .start_session("window_a", 3)
        .expect("window A starts new request after previous finish");
    assert!(!cancel_a_new.load(Ordering::SeqCst));
}

// ============================================================================
// 3. Streaming, Output Capping, Markup Escaping, & Crash Tests with Fake Engine
// ============================================================================

#[test]
fn test_normal_chat_streaming_with_fake_engine() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();
    let bin_hash = sha256_file(&bin_path).unwrap();

    let mut manager = LocalEngineManager::with_config(
        bin_path,
        bin_hash,
        10,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    let channel = manager.ensure_running(&model_file).expect("engine ready");
    let http_client = create_engine_http_client().unwrap();

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Hello engine".to_string(),
        }],
        max_tokens: Some(128),
        temperature: Some(0.7),
        stream: Some(true),
    };

    let chunks = Arc::new(Mutex::new(Vec::new()));
    let chunks_clone = Arc::clone(&chunks);
    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let stream_cfg = StreamChatConfig::default();

    execute_chat_stream(
        &http_client,
        &channel,
        "test-win",
        1,
        "test-model",
        &req,
        &stream_cfg,
        &cancel_flag,
        move |chunk| {
            chunks_clone.lock().unwrap().push(chunk.to_string());
        },
        move |status, _err| {
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    let received_chunks = chunks.lock().unwrap().clone();
    assert_eq!(received_chunks, vec!["Hello ", "from fake engine!"]);
    assert_eq!(*final_status.lock().unwrap(), Some(ChatStreamStatus::Done));

    manager.shutdown();
}

#[test]
fn test_stream_output_capped_at_100k_chars_returns_truncated() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();
    let bin_hash = sha256_file(&bin_path).unwrap();

    let mut manager = LocalEngineManager::with_config(
        bin_path,
        bin_hash,
        10,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    let channel = manager.ensure_running(&model_file).expect("engine ready");
    let http_client = create_engine_http_client().unwrap();

    // Trigger fake engine's 5 MB output stream
    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "test:oversized_5mb".to_string(),
        }],
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };

    let total_chars = Arc::new(Mutex::new(0usize));
    let total_chars_clone = Arc::clone(&total_chars);
    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let stream_cfg = StreamChatConfig::default();

    execute_chat_stream(
        &http_client,
        &channel,
        "test-win",
        2,
        "test-model",
        &req,
        &stream_cfg,
        &cancel_flag,
        move |chunk| {
            let mut count = total_chars_clone.lock().unwrap();
            *count += chunk.chars().count();
        },
        move |status, _err| {
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    // Must be capped at exactly MAX_CHAT_STREAMED_OUTPUT_CHARS (100,000 chars)
    assert_eq!(*total_chars.lock().unwrap(), MAX_CHAT_STREAMED_OUTPUT_CHARS);
    assert_eq!(
        *final_status.lock().unwrap(),
        Some(ChatStreamStatus::Truncated),
        "Capped output stream must end with 'truncated' status"
    );

    manager.shutdown();
}

#[test]
fn test_cancel_before_first_token() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();
    let bin_hash = sha256_file(&bin_path).unwrap();

    let mut manager = LocalEngineManager::with_config(
        bin_path,
        bin_hash,
        10,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    let channel = manager.ensure_running(&model_file).expect("engine ready");
    let http_client = create_engine_http_client().unwrap();

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "Hello".to_string(),
        }],
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };

    let cancel_flag = Arc::new(AtomicBool::new(true)); // Pre-cancelled
    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);
    let chunks_delivered = Arc::new(Mutex::new(0));
    let chunks_clone = Arc::clone(&chunks_delivered);

    execute_chat_stream(
        &http_client,
        &channel,
        "test-win",
        3,
        "test-model",
        &req,
        &StreamChatConfig::default(),
        &cancel_flag,
        move |_| {
            *chunks_clone.lock().unwrap() += 1;
        },
        move |status, _err| {
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    assert_eq!(*chunks_delivered.lock().unwrap(), 0);
    assert_eq!(
        *final_status.lock().unwrap(),
        Some(ChatStreamStatus::Cancelled)
    );

    manager.shutdown();
}

#[test]
fn test_cancel_mid_stream() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();
    let bin_hash = sha256_file(&bin_path).unwrap();

    let mut manager = LocalEngineManager::with_config(
        bin_path,
        bin_hash,
        10,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    let channel = manager.ensure_running(&model_file).expect("engine ready");
    let http_client = create_engine_http_client().unwrap();

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "test:slow_stream".to_string(),
        }],
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_clone = Arc::clone(&cancel_flag);
    let chunks_count = Arc::new(Mutex::new(0));
    let chunks_clone = Arc::clone(&chunks_count);
    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    execute_chat_stream(
        &http_client,
        &channel,
        "test-win",
        4,
        "test-model",
        &req,
        &StreamChatConfig::default(),
        &cancel_flag,
        move |_chunk| {
            let mut c = chunks_clone.lock().unwrap();
            *c += 1;
            if *c >= 2 {
                // Cancel mid-stream after receiving 2 chunks
                cancel_flag_clone.store(true, Ordering::SeqCst);
            }
        },
        move |status, _err| {
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    let count = *chunks_count.lock().unwrap();
    assert!(
        (2..10).contains(&count),
        "Stream stopped early on cancel (delivered {count} chunks)"
    );
    assert_eq!(
        *final_status.lock().unwrap(),
        Some(ChatStreamStatus::Cancelled)
    );

    manager.shutdown();
}

#[test]
fn test_wall_clock_timeout_terminates_with_failed_status() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();
    let bin_hash = sha256_file(&bin_path).unwrap();

    let mut manager = LocalEngineManager::with_config(
        bin_path,
        bin_hash,
        10,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    let channel = manager.ensure_running(&model_file).expect("engine ready");
    let http_client = create_engine_http_client().unwrap();

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "test:slow_stream".to_string(),
        }],
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    // Enforce 100ms wall-clock timeout
    let stream_cfg = StreamChatConfig {
        wall_clock_timeout: Duration::from_millis(100),
        max_streamed_chars: MAX_CHAT_STREAMED_OUTPUT_CHARS,
        config_max_tokens: 1024,
    };

    execute_chat_stream(
        &http_client,
        &channel,
        "test-win",
        5,
        "test-model",
        &req,
        &stream_cfg,
        &cancel_flag,
        |_| {},
        move |status, _err| {
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    assert_eq!(
        *final_status.lock().unwrap(),
        Some(ChatStreamStatus::Failed),
        "Timeout must terminate stream with 'failed' status"
    );

    manager.shutdown();
}

#[test]
fn test_engine_crash_mid_stream_returns_failed_and_never_hangs() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();
    let bin_hash = sha256_file(&bin_path).unwrap();

    let mut manager = LocalEngineManager::with_config(
        bin_path,
        bin_hash,
        10,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    let channel = manager.ensure_running(&model_file).expect("engine ready");
    let http_client = create_engine_http_client().unwrap();

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "test:crash_mid_stream".to_string(),
        }],
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };

    let final_status = Arc::new(Mutex::new(None));
    let status_clone = Arc::clone(&final_status);

    execute_chat_stream(
        &http_client,
        &channel,
        "test-win",
        6,
        "test-model",
        &req,
        &StreamChatConfig::default(),
        &Arc::new(AtomicBool::new(false)),
        |_| {},
        move |status, _err| {
            *status_clone.lock().unwrap() = Some(status);
        },
    );

    assert_eq!(
        *final_status.lock().unwrap(),
        Some(ChatStreamStatus::Failed),
        "Engine crash mid-stream must return 'failed' status and never hang"
    );

    // Lifecycle manager can handle restart on next call
    let next_channel = manager.ensure_running(&model_file);
    assert!(
        next_channel.is_ok(),
        "Lifecycle manager successfully restarts engine"
    );

    manager.shutdown();
}

#[test]
fn test_dangerous_markup_and_invalid_utf8_output_received_as_pure_data() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();
    let bin_hash = sha256_file(&bin_path).unwrap();

    let mut manager = LocalEngineManager::with_config(
        bin_path,
        bin_hash,
        10,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    let channel = manager.ensure_running(&model_file).expect("engine ready");
    let http_client = create_engine_http_client().unwrap();

    // 1. Dangerous markup test
    let req_markup = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "test:xss_markup".to_string(),
        }],
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };

    let received_scripts = Arc::new(Mutex::new(Vec::new()));
    let scripts_clone = Arc::clone(&received_scripts);

    execute_chat_stream(
        &http_client,
        &channel,
        "test-win",
        7,
        "test-model",
        &req_markup,
        &StreamChatConfig::default(),
        &Arc::new(AtomicBool::new(false)),
        move |chunk| {
            let script = build_chat_chunk_script(7, chunk).expect("safe script serialization");
            scripts_clone
                .lock()
                .unwrap()
                .push((chunk.to_string(), script));
        },
        |_, _| {},
    );

    let delivered = received_scripts.lock().unwrap().clone();
    assert!(!delivered.is_empty());

    for (raw_chunk, script) in &delivered {
        // Must never contain raw unescaped script tag
        assert!(
            !script.contains("</script>"),
            "Delivery script must never contain raw </script>: {script}"
        );
        assert!(
            !script.contains("<script>"),
            "Delivery script must never contain raw <script>: {script}"
        );
        // Must strictly use Phase 0 delivery wrapper
        assert!(script.starts_with("window.__wavical_receive__(JSON.parse(\""));

        // Simulate browser JSON.parse decoding
        let prefix = "window.__wavical_receive__(JSON.parse(";
        let suffix = "));";
        let js_literal = &script[prefix.len()..script.len() - suffix.len()];
        let json_str: String = serde_json::from_str(js_literal).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(payload["chunk"].as_str().unwrap(), raw_chunk);
    }

    // 2. Invalid UTF-8 bytes from engine
    let req_utf8 = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "test:invalid_utf8".to_string(),
        }],
        max_tokens: None,
        temperature: None,
        stream: Some(true),
    };

    let utf8_chunks = Arc::new(Mutex::new(Vec::new()));
    let utf8_clone = Arc::clone(&utf8_chunks);

    execute_chat_stream(
        &http_client,
        &channel,
        "test-win",
        8,
        "test-model",
        &req_utf8,
        &StreamChatConfig::default(),
        &Arc::new(AtomicBool::new(false)),
        move |chunk| {
            utf8_clone.lock().unwrap().push(chunk.to_string());
        },
        |_, _| {},
    );

    let decoded_chunks = utf8_chunks.lock().unwrap().clone();
    assert!(!decoded_chunks.is_empty());
    // Safe lossy replacement character must be present
    assert!(
        decoded_chunks.iter().any(|c| c.contains('\u{FFFD}')),
        "Invalid UTF-8 bytes safely converted to replacement characters"
    );

    manager.shutdown();
}

// ============================================================================
// 4. Secret Marker Prompt Redaction in Tracing
// ============================================================================

struct MarkerCaptureSubscriber {
    captured_logs: Arc<Mutex<Vec<String>>>,
}

impl tracing::Subscriber for MarkerCaptureSubscriber {
    fn register_callsite(
        &self,
        _metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::always()
    }
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct FieldVisitor<'a>(&'a mut String);
        impl tracing::field::Visit for FieldVisitor<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                let _ = write!(self.0, " {}={:?}", field.name(), value);
            }
        }
        let mut msg = String::new();
        let mut visitor = FieldVisitor(&mut msg);
        event.record(&mut visitor);
        self.captured_logs.lock().unwrap().push(msg);
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

#[test]
fn test_marker_prompt_never_appears_in_any_tracing_log_line() {
    let captured_logs = Arc::new(Mutex::new(Vec::new()));
    let subscriber = MarkerCaptureSubscriber {
        captured_logs: Arc::clone(&captured_logs),
    };

    let marker_prompt = "SECRET_PROMPT_MARKER_9876543210_FORBIDDEN";

    tracing::callsite::rebuild_interest_cache();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("Direct test event inside with_default");
        let temp = tempdir().unwrap();
        let model_file = create_dummy_model_file(temp.path());
        let bin_path = get_fake_engine_path();
        let bin_hash = sha256_file(&bin_path).unwrap();

        let mut manager = LocalEngineManager::with_config(
            bin_path,
            bin_hash,
            10,
            EngineLaunchOptions::default(),
            Duration::from_secs(5),
        )
        .expect("create manager");

        let channel = manager.ensure_running(&model_file).expect("engine ready");
        let http_client = create_engine_http_client().unwrap();

        let req = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: format!("Question with secret: {marker_prompt}"),
            }],
            max_tokens: Some(64),
            temperature: Some(0.5),
            stream: Some(true),
        };

        execute_chat_stream(
            &http_client,
            &channel,
            "win-marker-test",
            42,
            "qwen2.5-1.5b-instruct",
            &req,
            &StreamChatConfig::default(),
            &Arc::new(AtomicBool::new(false)),
            |_| {},
            |_, _| {},
        );

        manager.shutdown();
    });

    let all_logs = captured_logs.lock().unwrap();
    assert!(!all_logs.is_empty(), "Must have captured tracing events");

    for line in all_logs.iter() {
        assert!(
            !line.contains(marker_prompt),
            "SECURITY VIOLATION: Marker prompt leaked into tracing log line: {line}"
        );
    }
}
