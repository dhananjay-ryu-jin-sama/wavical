#![forbid(unsafe_code)]

use std::fs;
use tempfile::tempdir;
use wavical_protocol::HEADER_NOSNIFF;
use wavical_shell::{
    DEFAULT_CSP, HEADER_CSP, IpcOriginError, WAVICAL_INIT_SCRIPT, WindowConfig,
    build_delivery_script, check_webview2_runtime_with_override, handle_navigation_request,
    handle_new_window_request, handle_protocol_request, is_devtools_enabled, is_navigation_allowed,
    is_origin_allowed, process_ipc_message, truncate_payload,
};
use wry::NewWindowResponse;
use wry::http::{Request, StatusCode, Uri};

/// 1. Startup & WebView2 Missing Path
#[test]
fn test_shell_webview2_missing_runtime_simulation() {
    let result = check_webview2_runtime_with_override(Some("1"));
    assert!(
        result.is_err(),
        "WAVICAL_SIMULATE_NO_WEBVIEW2 must cause check_webview2_runtime to return Err"
    );
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("Simulated missing WebView2 runtime"),
        "Error message must be friendly and descriptive: {err_msg}"
    );
}

/// 1b. Devtools check matches debug assertions and strictly disabled in release (F-A5-01)
#[test]
fn test_shell_devtools_disabled_in_release() {
    let devtools = is_devtools_enabled();
    assert_eq!(
        devtools,
        cfg!(debug_assertions),
        "Devtools must strictly match cfg!(debug_assertions) in debug profile"
    );

    // F-A5-01: Verify under release profile that is_devtools_enabled() returns false
    let output = std::process::Command::new("cargo")
        .args([
            "test",
            "-p",
            "wavical-shell",
            "--release",
            "--lib",
            "--",
            "test_devtools_strictly_disabled_in_release",
        ])
        .output()
        .expect("cargo test --release must execute");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success()
            && stdout.contains("test tests::test_devtools_strictly_disabled_in_release ... ok"),
        "DevTools must be verified in release profile and pass: status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout,
        stderr
    );
}

/// 2. Bridge script: immutability, freezing, and properties
#[test]
fn test_shell_bridge_script_immutability_and_safety() {
    let script = WAVICAL_INIT_SCRIPT;

    // Must not use eval or new Function
    assert!(!script.contains("eval("), "Bridge script must not use eval");
    assert!(
        !script.contains("new Function"),
        "Bridge script must not use new Function"
    );

    // Must freeze API
    assert!(
        script.contains("Object.freeze({") || script.contains("Object.freeze(wavicalApi)"),
        "Bridge script must freeze API object"
    );
    assert!(
        script.contains("Object.freeze(invoke)"),
        "Bridge script must freeze invoke function"
    );

    // Must define non-writable, non-configurable window.wavical
    assert!(
        script.contains("Object.defineProperty(window, 'wavical'"),
        "Bridge script must define window.wavical using Object.defineProperty"
    );
    assert!(
        script.contains("writable: false"),
        "window.wavical must be non-writable"
    );
    assert!(
        script.contains("configurable: false"),
        "window.wavical must be non-configurable"
    );

    // Request ID rollover at MAX_SAFE_INTEGER
    assert!(
        script.contains("Number.MAX_SAFE_INTEGER"),
        "Bridge script must guard against ID overflow"
    );
}

/// 2b. Finding: Bridge script exposes more than `invoke` and adds extra window properties
/// Documented requirement: "only invoke is exposed; no other globals are added"
/// In wavical-shell, onMenu and onShortcut are exposed on window.wavical, and
/// __wavical_receive__, __wavical_event__, __wavical_shortcut_event__ are added to window.
#[test]
fn test_shell_bridge_script_only_exposes_invoke_and_no_other_globals() {
    let script = WAVICAL_INIT_SCRIPT;

    // Check that ONLY invoke is in wavicalApi
    assert!(
        !script.contains("onMenu:"),
        "Phase 0 spec requires only 'invoke' on window.wavical; 'onMenu' is exposed"
    );
    assert!(
        !script.contains("onShortcut:"),
        "Phase 0 spec requires only 'invoke' on window.wavical; 'onShortcut' is exposed"
    );

    // Check that no other globals are attached to window
    assert!(
        !script.contains("__wavical_event__"),
        "Extra global '__wavical_event__' attached to window"
    );
    assert!(
        !script.contains("__wavical_shortcut_event__"),
        "Extra global '__wavical_shortcut_event__' attached to window"
    );
}

/// 3 & 4. Origin check: rigorous table-driven test vectors
#[test]
fn test_shell_origin_check_vectors() {
    let config = WindowConfig::default();
    let allowed = &config.allowed_origins;

    // Scheme + Host + Port equality
    let valid_origins = [
        "http://wavical.localhost",
        "wavical://localhost",
        "https://wavical.localhost",
        "http://wavical.localhost/path",
        "http://wavical.localhost:80/path", // default port stripped
        "https://wavical.localhost:443/path", // default port stripped
        "http://WAVICAL.LOCALHOST/path",    // case insensitivity on host
        "https://WAVICAL.LOCALHOST/index.html",
    ];

    for valid in valid_origins {
        let uri = valid.parse::<Uri>().expect("valid uri");
        assert!(
            is_origin_allowed(&uri, allowed),
            "Expected '{valid}' to be allowed by origin check"
        );
    }

    // Attacks and invalid origins
    let invalid_origins = [
        "wavical://app.evil",
        "http://wavical.localhost.evil.com",
        "http://evil.com/?http://wavical.localhost",
        "http://wavical.localhost@evil.com",
        "http://wavical.localhost:8080",
        "http://wavical.localhost.", // trailing dot FQDN
        "https://wavical.localhost:8443",
        "https://evil.com/phishing",
        "http://attacker.com/",
        "file:///C:/Windows/win.ini",
        "data:text/html,evil",
        "javascript:alert(1)",
        "about:blank",
    ];

    for invalid in invalid_origins {
        if let Ok(uri) = invalid.parse::<Uri>() {
            assert!(
                !is_origin_allowed(&uri, allowed),
                "Expected '{invalid}' to be REJECTED by origin check"
            );
        }
    }
}

/// 4b. Origin check URL evaluation for sub-frames
#[test]
fn test_shell_subframe_and_special_schemes() {
    let config = WindowConfig::default();
    let allowed = &config.allowed_origins;

    // blob: scheme has no authority/host in RFC 3986 parse
    if let Ok(blob_uri) = "blob:http://wavical.localhost/uuid-1234".parse::<Uri>() {
        assert!(
            !is_origin_allowed(&blob_uri, allowed),
            "blob: URI must not be accepted as application origin without unwrapping"
        );
    }

    // data: scheme has no host
    if let Ok(data_uri) = "data:text/html,<h1>attack</h1>".parse::<Uri>() {
        assert!(!is_origin_allowed(&data_uri, allowed));
    }

    // about:blank has no host
    if let Ok(about_uri) = "about:blank".parse::<Uri>() {
        assert!(!is_origin_allowed(&about_uri, allowed));
    }
}

/// 5. Response delivery: double JSON serialization prevents script concatenation breakout
#[test]
fn test_shell_response_delivery_escaping() {
    let attack_strings = [
        "hello 'world'",
        "hello \"world\"",
        "hello `world`",
        "backslash \\ and \\\\ test",
        "</script><script>alert('xss')</script>",
        "line separator \u{2028} test",
        "paragraph separator \u{2029} test",
        "${x} and `${x}` template interpolation",
        &"A".repeat(1024 * 1024), // 1 MB string
    ];

    for attack in attack_strings {
        let payload = serde_json::json!({ "field": attack });
        let resp = wavical_ipc::Response::success(42, payload);

        let script =
            build_delivery_script(&resp).expect("double JSON serialization should always succeed");

        // Must strictly use JSON.parse wrapper without raw concatenation
        assert!(
            script.starts_with("window.__wavical_receive__(JSON.parse(\""),
            "Script must start with window.__wavical_receive__(JSON.parse(\""
        );
        assert!(
            script.ends_with("\"));"),
            "Script must terminate with \"));"
        );

        // Verify that extracting the argument and parsing JSON yields exact original value
        let prefix = "window.__wavical_receive__(JSON.parse(";
        let suffix = "));";
        assert!(script.starts_with(prefix));
        assert!(script.ends_with(suffix));
        let inner_json_literal = &script[prefix.len()..script.len() - suffix.len()];
        let deserialized_literal: String = serde_json::from_str(inner_json_literal)
            .expect("inner string literal must be valid JSON string");
        let parsed_obj: serde_json::Value =
            serde_json::from_str(&deserialized_literal).expect("deserialized JSON object");

        assert_eq!(parsed_obj["id"], 42);
        assert_eq!(parsed_obj["ok"], true);
        assert_eq!(parsed_obj["result"]["field"], attack);
    }
}

/// 6. Navigation and popup guard
#[test]
fn test_shell_navigation_and_popup_guard() {
    let config = WindowConfig::default();
    let allowed = &config.allowed_origins;

    // Allowed navigation
    assert!(is_navigation_allowed(
        "http://wavical.localhost/page2.html",
        allowed
    ));
    assert!(is_navigation_allowed(
        "wavical://localhost/index.html",
        allowed
    ));
    assert!(is_navigation_allowed(
        "https://wavical.localhost/dashboard",
        allowed
    ));

    // Blocked navigations
    let blocked_navs = [
        "https://google.com",
        "https://evil.com/phishing",
        "javascript:alert(1)",
        "data:text/html,evil",
        "blob:http://wavical.localhost/123",
        "file:///C:/Windows/win.ini",
        "ftp://ftp.example.com",
        "mailto:user@example.com",
        "ms-calc:",
    ];

    for nav in blocked_navs {
        assert!(
            !handle_navigation_request(nav, allowed, "test-win"),
            "Navigation to '{nav}' must be blocked"
        );
    }

    // Popups: unconditionally denied
    assert!(matches!(
        handle_new_window_request("https://any-url.com", "test-win"),
        NewWindowResponse::Deny
    ));
    assert!(matches!(
        handle_new_window_request("http://wavical.localhost/popup", "test-win"),
        NewWindowResponse::Deny
    ));
}

/// 7. CSP header analysis
#[test]
fn test_shell_csp_directives_strictness() {
    let csp = DEFAULT_CSP;

    // Mandatory strict directives
    assert!(
        csp.contains("default-src 'self'"),
        "CSP must contain default-src 'self'"
    );
    assert!(
        csp.contains("script-src 'self'"),
        "CSP must contain script-src 'self'"
    );
    assert!(
        csp.contains("object-src 'none'"),
        "CSP must contain object-src 'none'"
    );
    assert!(
        csp.contains("base-uri 'none'"),
        "CSP must contain base-uri 'none'"
    );
    assert!(
        csp.contains("frame-ancestors 'none'"),
        "CSP must contain frame-ancestors 'none'"
    );
    assert!(
        csp.contains("form-action 'self'"),
        "CSP must contain form-action 'self'"
    );

    // FORBIDDEN in script-src
    assert!(
        !csp.contains("'unsafe-inline'"),
        "CSP must never contain 'unsafe-inline' in script-src"
    );
    assert!(
        !csp.contains("'unsafe-eval'"),
        "CSP must never contain 'unsafe-eval'"
    );
    assert!(
        !csp.contains("script-src *"),
        "CSP must not allow wildcard scripts"
    );
    assert!(
        !csp.contains("script-src https:"),
        "CSP must not allow https in script-src"
    );
    assert!(
        !csp.contains("script-src data:"),
        "CSP must not allow data in script-src"
    );
    assert!(
        !csp.contains("script-src blob:"),
        "CSP must not allow blob in script-src"
    );
}

/// FINDING-S5: Default CSP must explicitly declare connect-src 'self' and frame-src 'none'
#[test]
fn test_shell_csp_explicit_connect_and_frame_src() {
    let csp = DEFAULT_CSP;
    assert!(
        csp.contains("connect-src 'self'"),
        "DEFAULT_CSP must explicitly declare connect-src 'self' (FINDING-S5)"
    );
    assert!(
        csp.contains("frame-src 'none'"),
        "DEFAULT_CSP must explicitly declare frame-src 'none' (FINDING-S5)"
    );
}

/// 7b. Protocol handler attaches CSP and nosniff to all responses (200, 404, rejection)
#[test]
fn test_shell_protocol_response_headers_on_all_responses() {
    let tmp = tempdir().expect("tempdir");
    let asset_root = tmp.path();

    // Create test files
    fs::write(
        asset_root.join("index.html"),
        "<html><body>Hello</body></html>",
    )
    .unwrap();
    fs::write(asset_root.join("app.js"), "console.log('test');").unwrap();
    fs::write(asset_root.join("style.css"), "body { margin: 0; }").unwrap();
    fs::write(asset_root.join("image.png"), b"\x89PNG\r\n\x1a\n").unwrap();

    let csp = DEFAULT_CSP;

    // 1. 200 OK HTML
    let req_html = Request::builder()
        .uri("wavical://localhost/index.html")
        .body(Vec::new())
        .unwrap();
    let resp_html = handle_protocol_request(req_html, asset_root, csp, "win");
    assert_eq!(resp_html.status(), StatusCode::OK);
    assert_eq!(
        resp_html.headers().get("Content-Type").unwrap(),
        "text/html; charset=utf-8"
    );
    assert_eq!(
        resp_html.headers().get(HEADER_NOSNIFF.0).unwrap(),
        HEADER_NOSNIFF.1
    );
    assert_eq!(resp_html.headers().get(HEADER_CSP).unwrap(), csp);

    // 2. 200 OK JS
    let req_js = Request::builder()
        .uri("wavical://localhost/app.js")
        .body(Vec::new())
        .unwrap();
    let resp_js = handle_protocol_request(req_js, asset_root, csp, "win");
    assert_eq!(resp_js.status(), StatusCode::OK);
    assert_eq!(
        resp_js.headers().get("Content-Type").unwrap(),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        resp_js.headers().get(HEADER_NOSNIFF.0).unwrap(),
        HEADER_NOSNIFF.1
    );
    assert_eq!(resp_js.headers().get(HEADER_CSP).unwrap(), csp);

    // 3. 200 OK Image
    let req_img = Request::builder()
        .uri("wavical://localhost/image.png")
        .body(Vec::new())
        .unwrap();
    let resp_img = handle_protocol_request(req_img, asset_root, csp, "win");
    assert_eq!(resp_img.status(), StatusCode::OK);
    assert_eq!(resp_img.headers().get("Content-Type").unwrap(), "image/png");
    assert_eq!(
        resp_img.headers().get(HEADER_NOSNIFF.0).unwrap(),
        HEADER_NOSNIFF.1
    );
    assert_eq!(resp_img.headers().get(HEADER_CSP).unwrap(), csp);

    // 4. 404 Missing File
    let req_404 = Request::builder()
        .uri("wavical://localhost/nonexistent.html")
        .body(Vec::new())
        .unwrap();
    let resp_404 = handle_protocol_request(req_404, asset_root, csp, "win");
    assert_eq!(resp_404.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        resp_404.headers().get("Content-Type").unwrap(),
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        resp_404.headers().get(HEADER_NOSNIFF.0).unwrap(),
        HEADER_NOSNIFF.1
    );
    assert_eq!(resp_404.headers().get(HEADER_CSP).unwrap(), csp);

    // 5. Rejected Traversal Path
    let req_rejection = Request::builder()
        .uri("wavical://localhost/../secret.txt")
        .body(Vec::new())
        .unwrap();
    let resp_rejection = handle_protocol_request(req_rejection, asset_root, csp, "win");
    assert_eq!(resp_rejection.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        resp_rejection.headers().get("Content-Type").unwrap(),
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        resp_rejection.headers().get(HEADER_NOSNIFF.0).unwrap(),
        HEADER_NOSNIFF.1
    );
    assert_eq!(resp_rejection.headers().get(HEADER_CSP).unwrap(), csp);
}

/// 8. Raw channel: fuzzing, type checking, size enforcement
#[test]
fn test_shell_raw_ipc_channel_enforcement() {
    let config = WindowConfig::default();
    let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
    let valid_uri = "http://wavical.localhost/index.html"
        .parse::<Uri>()
        .unwrap();

    // A. Method without capability (core.echo)
    let ungranted = r#"{"id": 1, "method": "core.echo", "params": "foo"}"#;
    let res_ungranted = process_ipc_message(&valid_uri, ungranted, &config, &registry)
        .expect("valid origin")
        .expect("response expected for capability denial");
    assert_eq!(res_ungranted.id, 1);
    assert!(!res_ungranted.ok);
    assert_eq!(
        res_ungranted.error.as_deref(),
        Some(wavical_ipc::GENERIC_METHOD_ERROR)
    );

    // B. Malformed JSON -> dropped silently in Rust (returns Ok(None))
    let malformed = "{ not valid json ...";
    let res_malformed = process_ipc_message(&valid_uri, malformed, &config, &registry).unwrap();
    assert!(res_malformed.is_none());

    // C. 2 MB Oversize payload -> dropped silently in Rust (returns Ok(None))
    let oversize = "X".repeat(2 * 1024 * 1024);
    let res_oversize = process_ipc_message(&valid_uri, &oversize, &config, &registry).unwrap();
    assert!(res_oversize.is_none());

    // D. Wrong types (negative ID)
    let wrong_id = r#"{"id": -10, "method": "core.ping"}"#;
    let res_wrong_id = process_ipc_message(&valid_uri, wrong_id, &config, &registry).unwrap();
    assert!(res_wrong_id.is_none());

    // E. Unauthorized origin -> returns Err(IpcOriginError::DisallowedOrigin)
    let evil_uri = "https://evil.com/page.html".parse::<Uri>().unwrap();
    let res_evil = process_ipc_message(
        &evil_uri,
        r#"{"id": 2, "method": "core.ping"}"#,
        &config,
        &registry,
    );
    assert!(matches!(res_evil, Err(IpcOriginError::DisallowedOrigin(_))));
}

/// 9. Error sanitization: folder name with SECRETMARKER never leaked
#[test]
fn test_shell_secret_marker_never_leaked_to_page() {
    let tmp = tempdir().expect("tempdir");
    let marker_folder = tmp.path().join("SECRETMARKER_PATH_98765");
    fs::create_dir(&marker_folder).expect("dir");
    fs::write(marker_folder.join("index.html"), "Hello").unwrap();

    let csp = DEFAULT_CSP;
    let attack_paths = [
        "/nonexistent_file.html",
        "/../SECRETMARKER_PATH_98765/index.html",
        "/..%5Cindex.html",
        "/C:%5CWindows%5Cwin.ini",
        "/CON",
        "/index.html::$DATA",
    ];

    for path in attack_paths {
        let req = Request::builder()
            .uri(format!("wavical://localhost{path}"))
            .body(Vec::new())
            .unwrap();
        let resp = handle_protocol_request(req, &marker_folder, csp, "win");
        let body = String::from_utf8_lossy(resp.body());

        assert!(
            !body.contains("SECRETMARKER"),
            "Body must never leak SECRETMARKER for path '{path}': {body}"
        );
        for (header_name, header_value) in resp.headers() {
            let val_str = header_value.to_str().unwrap_or_default();
            assert!(
                !val_str.contains("SECRETMARKER"),
                "Header '{header_name}' leaked SECRETMARKER: {val_str}"
            );
        }
    }
}

/// 10. Payload truncation helper prevents log bloat
#[test]
fn test_shell_truncate_payload_boundaries() {
    assert_eq!(truncate_payload("short", 10), "short");
    let long_ascii = "A".repeat(100);
    let trunc_ascii = truncate_payload(&long_ascii, 10);
    assert_eq!(trunc_ascii, "AAAAAAAAAA... [truncated]");

    // Multibyte character boundary protection (crab emoji: 4 bytes)
    let crabs = "🦀🦀🦀🦀";
    let trunc_crabs = truncate_payload(crabs, 6);
    // 6 bytes fits exactly one crab (4 bytes), cannot split second crab at byte 6
    assert_eq!(trunc_crabs, "🦀... [truncated]");
}

/// FINDING-S2: Integration harness must dynamic-match attack suites count and attack page must not hang on oversized payload
#[test]
fn test_integration_harness_dynamic_count_and_no_hang() {
    let integration_main = include_str!("../../../tests/integration/src/main.rs");
    assert!(
        !integration_main.contains("report.summary.passed == 7"),
        "tests/integration/src/main.rs must not hardcode 'report.summary.passed == 7' (FINDING-S2)"
    );
    assert!(
        integration_main.contains("report.summary.passed == report.summary.total"),
        "tests/integration/src/main.rs must match report.summary.passed == report.summary.total"
    );

    let attack_app = include_str!("../../../tests/attack-page/app.js");
    assert!(
        !attack_app
            .contains("await window.wavical.invoke(\"clipboard.write\", { text: hugeText });"),
        "tests/attack-page/app.js must not await unreplied oversized payload without timeout protection (FINDING-S2)"
    );
}

/// FINDING-S3: WebView2 user data directory must be isolated per app ID and configured on WebViewBuilder
#[test]
fn test_shell_webview2_user_data_directory_isolated() {
    let shell_src = include_str!("../../../crates/wavical-shell/src/lib.rs");
    assert!(
        shell_src.contains("new_with_web_context(")
            && shell_src.contains("get_webview_data_directory("),
        "crates/wavical-shell/src/lib.rs must configure isolated user data directory via WebContext (FINDING-S3)"
    );

    let data_dir1 = wavical_shell::get_webview_data_directory("my-app-1");
    let data_dir2 = wavical_shell::get_webview_data_directory("my-app-2");

    assert_ne!(
        data_dir1, data_dir2,
        "WebView2 user data directories must be isolated between different app IDs"
    );
    assert!(
        data_dir1.to_string_lossy().contains("my-app-1"),
        "Data directory must contain app ID: {:?}",
        data_dir1
    );
    assert!(
        data_dir1.to_string_lossy().contains("EBWebView"),
        "Data directory must contain EBWebView subfolder: {:?}",
        data_dir1
    );
}

/// FINDING-S4: Dynamic capability reload and --listen-stdin must be gated behind debug_assertions
#[test]
fn test_shell_stdin_dynamic_capability_reload_gated_in_release() {
    let main_src = include_str!("../../../crates/wavical-shell/src/main.rs");
    assert!(
        main_src.contains("#[cfg(debug_assertions)]\n    #[arg(long)]\n    pub listen_stdin: bool")
            || main_src.contains(
                "#[cfg(debug_assertions)]\r\n    #[arg(long)]\r\n    pub listen_stdin: bool"
            ),
        "crates/wavical-shell/src/main.rs must gate listen_stdin CLI flag behind #[cfg(debug_assertions)] (FINDING-S4)"
    );

    let lib_src = include_str!("../../../crates/wavical-shell/src/lib.rs");
    assert!(
        lib_src.contains("#[cfg(debug_assertions)]\n    if config.listen_stdin")
            || lib_src.contains("#[cfg(debug_assertions)]\r\n    if config.listen_stdin"),
        "crates/wavical-shell/src/lib.rs must gate stdin listener thread behind #[cfg(debug_assertions)] (FINDING-S4)"
    );
    assert!(
        lib_src.contains("#[cfg(debug_assertions)]\n            Event::UserEvent(UserEvent::UpdateCapabilities")
            || lib_src.contains("#[cfg(debug_assertions)]\r\n            Event::UserEvent(UserEvent::UpdateCapabilities")
            || lib_src.contains("#[cfg(debug_assertions)]\n                {\n                    tracing::info!(\n                        \"Updating capabilities via dev server control")
            || lib_src.contains("#[cfg(debug_assertions)]\r\n                {\r\n                    tracing::info!(\r\n                        \"Updating capabilities via dev server control"),
        "crates/wavical-shell/src/lib.rs must gate UpdateCapabilities event handler behind #[cfg(debug_assertions)] (FINDING-S4)"
    );
}
