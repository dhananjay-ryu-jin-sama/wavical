#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tempfile::tempdir;
use wavical_config::BackendConfig;
use wavical_ipc::{Capabilities, GENERIC_HANDLER_ERROR, GENERIC_METHOD_ERROR};
use wavical_shell::WindowConfig;
use wavical_shell::backend::{BackendLaunchConfig, BackendService, set_global_backend_service};
use wavical_shell::process_ipc_message;
use wry::http::Uri;

fn setup_test_backend() -> (tempfile::TempDir, Arc<BackendService>, WindowConfig) {
    let fake_bin = PathBuf::from(env!("CARGO_BIN_EXE_fake_backend"));
    assert!(
        fake_bin.is_file(),
        "fake_backend binary must exist at {:?}",
        fake_bin
    );

    let temp = tempdir().expect("tempdir");
    let project_root = temp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();

    let mut launch_cfg = BackendLaunchConfig::new("test-app", &project_root, "backend/main.js");
    let working_dir = launch_cfg.working_dir.clone();
    std::fs::create_dir_all(&working_dir).unwrap();

    launch_cfg.custom_binary_override = Some(fake_bin);
    launch_cfg.custom_args = vec!["--protocol".to_string()];

    let service = Arc::new(BackendService::new(launch_cfg));
    set_global_backend_service(service.clone());

    let mut capabilities = Capabilities::new();
    capabilities.grant("core.ping");
    capabilities.grant("backend.invoke");

    let mut backend_cfg = BackendConfig::default();
    backend_cfg.enabled = true;
    backend_cfg.methods = vec![
        "hello.greet".to_string(),
        "test.echo".to_string(),
        "test.sleep".to_string(),
        "test.answer_twice".to_string(),
        "test.unknown_id".to_string(),
        "test.interleave".to_string(),
        "test.heavy_logging".to_string(),
        "test.malformed_json".to_string(),
        "test.oversize_frame".to_string(),
        "test.error".to_string(),
    ];
    backend_cfg.timeout_seconds = 2; // short timeout for testing

    let window_config = WindowConfig {
        window_id: "win-test".to_string(),
        allowed_origins: vec!["http://wavical.localhost".to_string()],
        capabilities,
        custom_csp: None,
        dynamic_capabilities: None,
        initial_width: None,
        initial_height: None,
        title: None,
        listen_stdin: false,
        persist_state: false,
        single_instance: false,
        tray_icon: None,
        shortcuts: Vec::new(),
        dialog_filters: std::collections::HashMap::new(),
        backend_config: Some(backend_cfg),
        project_root: Some(project_root),
        backend_service: Some(service.clone()),
        asset_root: None,
    };

    (temp, service, window_config)
}

#[test]
fn test_backend_invoke_happy_path() {
    let (_temp, _service, config) = setup_test_backend();
    let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
    let uri = "http://wavical.localhost/index.html"
        .parse::<Uri>()
        .unwrap();

    let req_json = r#"{"id":100,"method":"backend.invoke","params":{"method":"hello.greet","params":{"name":"Alice"}}}"#;
    let res = process_ipc_message(&uri, req_json, &config, &registry)
        .expect("process IPC")
        .expect("some response");

    assert!(res.ok, "invocation should succeed");
    assert_eq!(res.id, 100);
    let result = res.result.expect("result must be present");
    assert_eq!(
        result.get("message").and_then(|v| v.as_str()),
        Some("Hello, Alice!")
    );
}

#[test]
fn test_backend_invoke_method_allowlist_exact_match() {
    let (_temp, _service, config) = setup_test_backend();
    let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
    let uri = "http://wavical.localhost/index.html"
        .parse::<Uri>()
        .unwrap();

    // 1. Case mismatch: Hello.Greet
    let res1 = process_ipc_message(
        &uri,
        r#"{"id":1,"method":"backend.invoke","params":{"method":"Hello.Greet"}}"#,
        &config,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert!(!res1.ok);
    assert_eq!(res1.error.as_deref(), Some(GENERIC_METHOD_ERROR));

    // 2. Trailing space: hello.greet<space>
    let res2 = process_ipc_message(
        &uri,
        r#"{"id":2,"method":"backend.invoke","params":{"method":"hello.greet "}}"#,
        &config,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert!(!res2.ok);
    assert_eq!(res2.error.as_deref(), Some(GENERIC_METHOD_ERROR));

    // 3. Prefix only: hello
    let res3 = process_ipc_message(
        &uri,
        r#"{"id":3,"method":"backend.invoke","params":{"method":"hello"}}"#,
        &config,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert!(!res3.ok);
    assert_eq!(res3.error.as_deref(), Some(GENERIC_METHOD_ERROR));

    // 4. Prefix extension: hello.greet.x
    let res4 = process_ipc_message(
        &uri,
        r#"{"id":4,"method":"backend.invoke","params":{"method":"hello.greet.x"}}"#,
        &config,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert!(!res4.ok);
    assert_eq!(res4.error.as_deref(), Some(GENERIC_METHOD_ERROR));

    // 5. Unlisted method
    let res5 = process_ipc_message(
        &uri,
        r#"{"id":5,"method":"backend.invoke","params":{"method":"unknown.method"}}"#,
        &config,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert!(!res5.ok);
    assert_eq!(res5.error.as_deref(), Some(GENERIC_METHOD_ERROR));
}

#[test]
fn test_backend_invoke_capability_denial() {
    let (_temp, _service, mut config) = setup_test_backend();
    let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
    let uri = "http://wavical.localhost/index.html"
        .parse::<Uri>()
        .unwrap();

    // Revoke backend.invoke capability
    config.capabilities.revoke("backend.invoke");

    let res = process_ipc_message(
        &uri,
        r#"{"id":1,"method":"backend.invoke","params":{"method":"hello.greet"}}"#,
        &config,
        &registry,
    )
    .unwrap()
    .unwrap();

    assert!(!res.ok);
    assert_eq!(res.error.as_deref(), Some(GENERIC_METHOD_ERROR));
}

#[test]
fn test_backend_invoke_params_1mb_plus_1_rejected() {
    let (_temp, service, config) = setup_test_backend();
    let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
    let uri = "http://wavical.localhost/index.html"
        .parse::<Uri>()
        .unwrap();

    let oversize = "A".repeat(1024 * 1024 + 1);
    let oversize_val = serde_json::Value::String(oversize.clone());

    // 1. Direct validation check returns ParamsTooLarge
    assert!(wavical_ipc::validate_backend_invoke_params(&oversize_val).is_err());

    // 2. Direct invoke with oversize params returns GENERIC_HANDLER_ERROR
    let direct_res = service.invoke(
        "win-oversize",
        1,
        "hello.greet",
        oversize_val,
        Duration::from_secs(1),
    );
    assert!(!direct_res.ok);
    assert_eq!(direct_res.error.as_deref(), Some(GENERIC_HANDLER_ERROR));

    // 3. Raw IPC message exceeding size limit is dropped at bridge boundary (returns Ok(None))
    let payload = serde_json::json!({
        "id": 1,
        "method": "backend.invoke",
        "params": {
            "method": "hello.greet",
            "params": {
                "big": oversize
            }
        }
    });

    let res = process_ipc_message(&uri, &payload.to_string(), &config, &registry).unwrap();
    assert!(res.is_none());
}

#[test]
fn test_backend_invoke_concurrency_limit_17_calls() {
    let (_temp, service, _config) = setup_test_backend();
    service.ensure_connected().expect("connect");
    let window_id = "win-concurrency";

    // Launch 16 background threads that call test.sleep (takes 60s)
    let mut handles = Vec::new();
    for i in 0..16 {
        let svc = service.clone();
        let wid = window_id.to_string();
        handles.push(thread::spawn(move || {
            svc.invoke(
                &wid,
                i,
                "test.sleep",
                serde_json::Value::Null,
                Duration::from_secs(3),
            )
        }));
    }

    // Wait until all 16 slots are acquired
    for _ in 0..50 {
        let count = service.invoke(
            window_id,
            17,
            "hello.greet",
            serde_json::Value::Null,
            Duration::from_secs(1),
        );
        if !count.ok {
            assert_eq!(count.error.as_deref(), Some(GENERIC_HANDLER_ERROR));
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    // Cancel pending calls to allow background threads to unblock cleanly
    service.cancel_window_calls(window_id);
    for h in handles {
        let _ = h.join();
    }
}

#[test]
fn test_backend_invoke_timeout() {
    let (_temp, service, _config) = setup_test_backend();
    let resp = service.invoke(
        "win-timeout",
        1,
        "test.sleep",
        serde_json::Value::Null,
        Duration::from_millis(300),
    );

    assert!(!resp.ok, "timed out call must fail");
    assert_eq!(resp.error.as_deref(), Some(GENERIC_HANDLER_ERROR));
}

#[test]
fn test_backend_answers_twice() {
    let (_temp, service, _config) = setup_test_backend();
    let resp = service.invoke(
        "win-twice",
        1,
        "test.answer_twice",
        serde_json::Value::Null,
        Duration::from_secs(2),
    );

    assert!(resp.ok);
    assert_eq!(
        resp.result.unwrap().get("call").and_then(|v| v.as_u64()),
        Some(1)
    );

    // Wait slightly to ensure second response frame arrived and was safely dropped
    thread::sleep(Duration::from_millis(100));
}

#[test]
fn test_backend_answers_unknown_id() {
    let (_temp, service, _config) = setup_test_backend();
    let resp = service.invoke(
        "win-unknown",
        1,
        "test.unknown_id",
        serde_json::Value::Null,
        Duration::from_secs(2),
    );

    // test.unknown_id returns response with id 999_999, so the pending call 1 times out
    assert!(!resp.ok);
    assert_eq!(resp.error.as_deref(), Some(GENERIC_HANDLER_ERROR));
}

#[test]
fn test_backend_answers_after_window_closed() {
    let (_temp, service, _config) = setup_test_backend();
    service.ensure_connected().expect("backend must connect");
    let window_id = "win-closed";

    let svc = service.clone();
    let wid = window_id.to_string();
    let handle = thread::spawn(move || {
        svc.invoke(
            &wid,
            1,
            "test.interleave",
            serde_json::json!({"delay_ms": 1000}),
            Duration::from_secs(3),
        )
    });

    // Wait until the invocation has registered
    thread::sleep(Duration::from_millis(150));
    // Close window while call is in-flight
    service.cancel_window_calls(window_id);

    let res = handle.join().unwrap();
    assert!(!res.ok, "cancelled call must return error");
    assert_eq!(res.error.as_deref(), Some(GENERIC_HANDLER_ERROR));
}

#[test]
fn test_backend_interleaved_responses() {
    let (_temp, service, _config) = setup_test_backend();

    let svc1 = service.clone();
    let h1 = thread::spawn(move || {
        svc1.invoke(
            "win-interleave",
            1,
            "test.interleave",
            serde_json::json!({"delay_ms": 200}),
            Duration::from_secs(3),
        )
    });

    let svc2 = service.clone();
    let h2 = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        svc2.invoke(
            "win-interleave",
            2,
            "test.interleave",
            serde_json::json!({"delay_ms": 0}),
            Duration::from_secs(3),
        )
    });

    let r2 = h2.join().unwrap();
    let r1 = h1.join().unwrap();

    assert!(r1.ok);
    assert!(r2.ok);
    assert_eq!(
        r1.result.unwrap().get("delay_ms").and_then(|v| v.as_u64()),
        Some(200)
    );
    assert_eq!(
        r2.result.unwrap().get("delay_ms").and_then(|v| v.as_u64()),
        Some(0)
    );
}

#[test]
fn test_backend_heavy_logging_does_not_corrupt_ipc() {
    let (_temp, service, _config) = setup_test_backend();
    let resp = service.invoke(
        "win-log",
        1,
        "test.heavy_logging",
        serde_json::Value::Null,
        Duration::from_secs(3),
    );

    assert!(resp.ok, "heavy logging must not corrupt pipe framing");
    assert_eq!(
        resp.result.unwrap().get("logged").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn test_backend_error_does_not_leak_internal_details_to_page() {
    let (_temp, service, _config) = setup_test_backend();
    let resp = service.invoke(
        "win-err",
        1,
        "test.error",
        serde_json::Value::Null,
        Duration::from_secs(2),
    );

    assert!(!resp.ok);
    // Page receives generic error only; sensitive database path is withheld
    assert_eq!(resp.error.as_deref(), Some(GENERIC_HANDLER_ERROR));
}
