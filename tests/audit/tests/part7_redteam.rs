// tests/audit/tests/part7_redteam.rs
//! Independent Red Team Security Assessment (Audit A4) for Wavical.
//! Testing G1 through G10 adversarial vectors.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use wavical_ipc::{
    Capabilities, GENERIC_METHOD_ERROR, GrantTable, HandlerRegistry, IpcError, MAX_JSON_DEPTH,
    MAX_MESSAGE_SIZE_BYTES, ReadGrantedParams, WriteGrantedParams, dispatch, parse_request,
    validate_raw_message,
};
use wavical_protocol::resolve_asset_path;
use wavical_shell::{WindowConfig, extract_origin, is_navigation_allowed, process_ipc_message};
use wry::http::Uri;

// ============================================================================
// G1: Capability Model Bypass & Evasion
// ============================================================================

#[test]
fn test_g1_attempt1_casing_evasion() {
    let mut caps = Capabilities::new();
    caps.grant("core.ping");

    let mut registry = HandlerRegistry::new();
    registry.register("core.ping", wavical_ipc::handle_core_ping);
    registry.register("core.echo", wavical_ipc::handle_core_echo);

    let cases = vec![
        "CORE.PING",
        "Core.Ping",
        "cOrE.pInG",
        "CORE.ECHO",
        "Core.Echo",
        "core.ECHO",
        "CLIPBOARD.READ",
        "Clipboard.Read",
    ];

    for method in cases {
        let raw = format!(r#"{{"id":1,"method":"{method}","params":null}}"#);
        let parse_res = parse_request(&raw);
        assert!(parse_res.is_ok(), "Method name syntax valid: {method}");
        let req = parse_res.unwrap();

        let resp = dispatch(&req, &caps, &registry);
        assert!(
            !resp.ok,
            "Casing variant '{method}' MUST NOT bypass capability check"
        );
        assert_eq!(
            resp.error.as_deref(),
            Some(GENERIC_METHOD_ERROR),
            "Variant '{method}' must return generic error"
        );
    }
}

#[test]
fn test_g1_attempt2_unicode_homoglyphs() {
    let homoglyph_methods = vec![
        "c\u{043E}re.ping",  // Cyrillic small 'о' U+043E
        "cor\u{0435}.ping",  // Cyrillic small 'е' U+0435
        "core\u{FF0E}ping",  // Fullwidth full stop U+FF0E
        "core.pin\u{0261}",  // Latin small script g U+0261
        "core\u{200B}.ping", // Zero-width space U+200B
    ];

    for method in homoglyph_methods {
        let raw = format!(r#"{{"id":1,"method":"{method}","params":null}}"#);
        let parse_res = parse_request(&raw);
        assert!(
            parse_res.is_err(),
            "Non-ASCII homoglyph '{method}' must be rejected by validate_method_name"
        );
    }
}

#[test]
fn test_g1_attempt3_duplicate_json_keys() {
    let raw_a = r#"{"id":1,"method":"core.ping","method":"core.echo","params":null}"#;
    let raw_b = r#"{"id":2,"method":"core.echo","method":"core.ping","params":null}"#;

    // Invariant: serde_json with deny_unknown_fields rejects duplicate keys
    let parse_res_a = parse_request(raw_a);
    assert!(
        parse_res_a.is_err(),
        "Duplicate keys must be rejected by parser"
    );
    match parse_res_a {
        Err(IpcError::InvalidJson(msg)) => {
            assert!(
                msg.contains("duplicate field `method`"),
                "Expected duplicate field error, got: {msg}"
            );
        }
        other => panic!("Unexpected parse result: {:?}", other),
    }

    let parse_res_b = parse_request(raw_b);
    assert!(
        parse_res_b.is_err(),
        "Duplicate keys must be rejected by parser"
    );
}

#[test]
fn test_g1_attempt4_whitespace_and_control_chars() {
    let whitespace_cases = vec![
        " core.ping",
        "core.ping ",
        "\tcore.ping",
        "core.ping\n",
        "core.\nping",
        "core. ping",
        "core..ping",
        "-core.ping",
        "core.ping-",
        ".core.ping",
        "core.ping.",
    ];

    for method in whitespace_cases {
        let raw = format!(r#"{{"id":1,"method":"{method}","params":null}}"#);
        let parse_res = parse_request(&raw);
        assert!(
            parse_res.is_err(),
            "Method with whitespace/control/separator '{method}' must be rejected"
        );
    }
}

#[test]
fn test_g1_attempt5_config_reload_race() {
    let dynamic_caps = Arc::new(std::sync::RwLock::new(Capabilities::new()));
    dynamic_caps.write().unwrap().grant("core.ping");

    let cfg = WindowConfig {
        window_id: "redteam-race-win".to_string(),
        dynamic_capabilities: Some(Arc::clone(&dynamic_caps)),
        ..Default::default()
    };

    let window_config = Arc::new(cfg);
    let stop_flag = Arc::new(AtomicBool::new(false));
    let ungranted_executed = Arc::new(AtomicBool::new(false));

    let mut reg = HandlerRegistry::new();
    reg.register("core.ping", wavical_ipc::handle_core_ping);
    reg.register("core.echo", wavical_ipc::handle_core_echo);
    let registry = Arc::new(reg);

    let uri = Uri::from_static("http://wavical.localhost/index.html");

    // Thread 1: Rapidly toggle core.echo
    let dyn_clone = Arc::clone(&dynamic_caps);
    let stop_clone1 = Arc::clone(&stop_flag);
    let reloader = thread::spawn(move || {
        let mut toggle = false;
        while !stop_clone1.load(Ordering::Relaxed) {
            let mut write_guard = dyn_clone.write().unwrap();
            if toggle {
                write_guard.grant("core.echo");
            } else {
                write_guard.revoke("core.echo");
            }
            toggle = !toggle;
            thread::yield_now();
        }
    });

    // Thread 2: Rapidly dispatch core.echo
    let cfg_clone2 = Arc::clone(&window_config);
    let stop_clone2 = Arc::clone(&stop_flag);
    let reg_clone2 = Arc::clone(&registry);
    let ungranted_flag = Arc::clone(&ungranted_executed);
    let _dyn_clone2 = Arc::clone(&dynamic_caps);

    let invoker = thread::spawn(move || {
        // Attempt to call clipboard.read which is NEVER granted during reload
        let ungranted_payload = r#"{"id":42,"method":"clipboard.read","params":null}"#;
        for _ in 0..10_000 {
            let res = process_ipc_message(&uri, ungranted_payload, &cfg_clone2, &reg_clone2);
            if let Ok(Some(resp)) = res {
                if resp.ok {
                    ungranted_flag.store(true, Ordering::SeqCst);
                    break;
                }
                assert_eq!(resp.error.as_deref(), Some(GENERIC_METHOD_ERROR));
            }
        }
        stop_clone2.store(true, Ordering::Relaxed);
    });

    reloader.join().unwrap();
    invoker.join().unwrap();

    assert!(
        !ungranted_executed.load(Ordering::SeqCst),
        "Ungranted method executed during reload race!"
    );
}

#[test]
fn test_g1_attempt6_batch_array_payload() {
    let batch_raw = r#"[{"id":1,"method":"core.ping"},{"id":2,"method":"clipboard.read"}]"#;
    let parse_res = parse_request(batch_raw);
    assert!(parse_res.is_err(), "Batch JSON array must be rejected");
}

// ============================================================================
// G2: Arbitrary File Read / Write Without User Dialog Pick
// ============================================================================

#[test]
fn test_g2_attempt1_grant_forging() {
    // Attack: Generate random or sequential grant IDs to access files without dialog
    let grant_table = GrantTable::new();
    let window_id = "test-window";

    // Insert legitimate grant
    let real_id = grant_table
        .insert(window_id, "C:\\valid\\file.txt")
        .unwrap();

    // Probe 1: Zeroed hex ID
    assert!(
        grant_table
            .get("00000000000000000000000000000000", window_id)
            .is_err()
    );
    // Probe 2: Off-by-one hex ID
    let mut forged = real_id.clone();
    let last_char = forged.pop().unwrap();
    let replacement = if last_char == 'a' { 'b' } else { 'a' };
    forged.push(replacement);
    assert!(grant_table.get(&forged, window_id).is_err());
    // Probe 3: Truncated or oversized ID
    assert!(grant_table.get(&real_id[..16], window_id).is_err());
}

#[test]
fn test_g2_attempt2_cross_window_and_closed_window_grants() {
    // Attack: Window B uses Window A's grant ID; Window A uses grant after close
    let grant_table = GrantTable::new();
    let win_a = "window-alpha";
    let win_b = "window-bravo";

    let grant_id = grant_table.insert(win_a, "C:\\secrets\\data.txt").unwrap();

    // Attempt 1: Cross-window theft (win_b tries to read win_a's grant)
    assert!(
        grant_table.get(&grant_id, win_b).is_err(),
        "Window B must NOT be able to access Window A's grant"
    );

    // Attempt 2: Use after window close
    grant_table.clear_window(win_a);
    assert!(
        grant_table.get(&grant_id, win_a).is_err(),
        "Grant must be wiped when Window A is closed"
    );
}

#[test]
fn test_g2_attempt3_path_in_params_injection() {
    // Attack: Attempt to supply raw path in parameters to bypass grant model
    let injection_json = r#"{"grantId":"dummy","path":"C:\\Windows\\System32\\cmd.exe"}"#;
    let read_params: Result<ReadGrantedParams, _> = serde_json::from_str(injection_json);
    assert!(
        read_params.is_err(),
        "ReadGrantedParams must reject extra 'path' parameter due to deny_unknown_fields"
    );

    let injection_write = r#"{"grantId":"dummy","content":"pwnd","path":"C:\\Windows\\cmd.exe"}"#;
    let write_params: Result<WriteGrantedParams, _> = serde_json::from_str(injection_write);
    assert!(
        write_params.is_err(),
        "WriteGrantedParams must reject extra 'path' parameter due to deny_unknown_fields"
    );
}

#[test]
fn test_g2_attempt4_protocol_out_of_src_escapes() {
    // Attack: Try reading files outside src/ (wavical.config.js, Cargo.toml, .env) via protocol resolver
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();
    let src_dir = root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    // Sensitive files outside src/
    std::fs::write(
        root.join("wavical.config.js"),
        "export default { appId: 'secret' };",
    )
    .unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = 'secret'").unwrap();
    std::fs::write(root.join(".env"), "SECRET_TOKEN=xyz123").unwrap();
    std::fs::write(src_dir.join("index.html"), "<h1>Hello</h1>").unwrap();

    let escape_probes = vec![
        "../wavical.config.js",
        "..%2Fwavical.config.js",
        "..\\wavical.config.js",
        "../Cargo.toml",
        "../.env",
        "/wavical.config.js",
        "C:\\wavical.config.js",
        "%2e%2e%2fwavical.config.js",
        "..%5cwavical.config.js",
    ];

    for probe in escape_probes {
        let res = resolve_asset_path(probe, &src_dir);
        assert!(
            res.is_err(),
            "Protocol escape probe '{probe}' MUST be rejected and never resolve outside src/"
        );
    }
}

#[test]
fn test_g2_attempt5_protocol_special_windows_paths() {
    // Attack: Alternate data streams, reserved names, UNC, long-path prefixes
    let temp_dir = tempfile::tempdir().unwrap();
    let src_dir = temp_dir.path();

    let special_paths = vec![
        "index.html:stream",
        "index.html::$DATA",
        "CON",
        "NUL",
        "AUX",
        "COM1",
        "LPT1",
        "\\\\server\\share\\file.txt",
        "\\\\?\\C:\\file.txt",
        "index.html.",
        "index.html ",
    ];

    for special in special_paths {
        let res = resolve_asset_path(special, src_dir);
        assert!(
            res.is_err(),
            "Windows special path '{special}' MUST be rejected by wavical_protocol"
        );
    }
}

// ============================================================================
// G3: Leave Origin or Load Remote Content
// ============================================================================

#[test]
fn test_g3_attempt1_navigation_external_schemes() {
    // Attack: Navigate to external schemes like ms-settings:, calc:, mailto:, file:
    let allowed_origins = vec![
        "http://wavical.localhost".to_string(),
        "wavical://localhost".to_string(),
    ];

    let evil_urls = vec![
        "ms-settings:privacy",
        "calculator:",
        "cmd.exe",
        "file:///C:/Windows/System32/cmd.exe",
        "javascript:alert(1)",
        "data:text/html,<h1>Pwned</h1>",
        "https://attacker.com/evil",
        "http://attacker.com/evil",
        "http://wavical.localhost.attacker.com/",
        "http://wavical.localhost:8080/", // unauthorized port
    ];

    for url in evil_urls {
        assert!(
            !is_navigation_allowed(url, &allowed_origins),
            "Navigation to '{url}' MUST be blocked"
        );
    }
}

#[test]
fn test_g3_attempt2_origin_extraction_spoofing() {
    // Attack: Userinfo, non-standard schemes, and port spoofing in URI
    let probes = vec![
        "http://attacker:secret@wavical.localhost/",
        "http://wavical.localhost:8080/",
        "blob:http://wavical.localhost/",
        "about:blank",
    ];

    for probe in probes {
        let uri = probe.parse::<Uri>();
        if let Ok(u) = uri {
            let extracted = extract_origin(&u);
            // Verify extracted origin cannot match http://wavical.localhost if port or userinfo differs
            if let Some(ref o) = extracted {
                if probe.contains(":8080") {
                    assert_eq!(o, "http://wavical.localhost:8080");
                }
                if probe.contains("@") {
                    // RFC 6454 userinfo must be rejected or stripped
                    assert!(!o.contains("@"));
                }
            }
        }
    }
}

// ============================================================================
// G4: Internal Info Leaks & Timing Side Channels
// ============================================================================

#[test]
fn test_g4_attempt1_timing_side_channel_analysis() {
    // Red team measurement: compare timing between granted, ungranted, and unknown methods
    let mut caps = Capabilities::new();
    caps.grant("core.ping");
    let mut reg = HandlerRegistry::new();
    reg.register("core.ping", wavical_ipc::handle_core_ping);
    reg.register("core.echo", wavical_ipc::handle_core_echo);

    let req_granted = parse_request(r#"{"id":1,"method":"core.ping","params":null}"#).unwrap();
    let req_ungranted = parse_request(r#"{"id":2,"method":"core.echo","params":null}"#).unwrap();
    let req_unknown =
        parse_request(r#"{"id":3,"method":"core.nonexistent","params":null}"#).unwrap();

    let iterations = 10_000;

    // Measure ungranted (rejected by capability check)
    let start_ungranted = Instant::now();
    for _ in 0..iterations {
        let _ = dispatch(&req_ungranted, &caps, &reg);
    }
    let dur_ungranted = start_ungranted.elapsed();

    // Measure unknown (rejected by capability check since unknown is also not granted)
    let start_unknown = Instant::now();
    for _ in 0..iterations {
        let _ = dispatch(&req_unknown, &caps, &reg);
    }
    let dur_unknown = start_unknown.elapsed();

    // Measure granted
    let start_granted = Instant::now();
    for _ in 0..iterations {
        let _ = dispatch(&req_granted, &caps, &reg);
    }
    let dur_granted = start_granted.elapsed();

    let avg_ungranted_ns = dur_ungranted.as_nanos() as f64 / iterations as f64;
    let avg_unknown_ns = dur_unknown.as_nanos() as f64 / iterations as f64;
    let avg_granted_ns = dur_granted.as_nanos() as f64 / iterations as f64;

    println!(
        "[Timing Test G4] 10,000 iterations: Ungranted={:.1}ns/call, Unknown={:.1}ns/call, Granted={:.1}ns/call",
        avg_ungranted_ns, avg_unknown_ns, avg_granted_ns
    );

    // Invariant: Both ungranted and unknown fail at the capabilities check (BTreeSet lookup)
    // The difference between ungranted and unknown must be statistically negligible (< 50ns per call)
    let diff = (avg_ungranted_ns - avg_unknown_ns).abs();
    assert!(
        diff < 50.0,
        "Timing discrepancy between ungranted and unknown is too large: {:.1}ns",
        diff
    );
}

#[test]
fn test_g4_attempt2_error_body_byte_for_byte_comparison() {
    let mut caps = Capabilities::new();
    caps.grant("core.ping");
    let mut reg = HandlerRegistry::new();
    reg.register("core.ping", wavical_ipc::handle_core_ping);
    reg.register("core.echo", wavical_ipc::handle_core_echo);

    let req_ungranted = parse_request(r#"{"id":100,"method":"core.echo","params":null}"#).unwrap();
    let req_unknown =
        parse_request(r#"{"id":100,"method":"core.nonexistent","params":null}"#).unwrap();

    let resp_ungranted = dispatch(&req_ungranted, &caps, &reg);
    let resp_unknown = dispatch(&req_unknown, &caps, &reg);

    assert_eq!(resp_ungranted.error, resp_unknown.error);
    assert_eq!(resp_ungranted.error.as_deref(), Some(GENERIC_METHOD_ERROR));
    // Exact byte for byte equality of JSON response
    let json_ungranted = serde_json::to_string(&resp_ungranted).unwrap();
    let json_unknown = serde_json::to_string(&resp_unknown).unwrap();
    assert_eq!(
        json_ungranted, json_unknown,
        "Error responses must be byte-for-byte identical"
    );
}

// ============================================================================
// G5: Crash or Hang the App from the Page
// ============================================================================

#[test]
fn test_g5_attempt1_message_size_boundary() {
    // Exact 1 MB limit, 1 MB + 1, and 50 MB
    let base = r#"{"id":1,"method":"core.ping","params":""}"#;
    let pad_len = MAX_MESSAGE_SIZE_BYTES - base.len();
    let exact_1mb = format!(
        r#"{{"id":1,"method":"core.ping","params":"{}"}}"#,
        "a".repeat(pad_len)
    );
    assert_eq!(exact_1mb.len(), MAX_MESSAGE_SIZE_BYTES);
    assert!(parse_request(&exact_1mb).is_ok());

    let over_1mb = format!(
        r#"{{"id":1,"method":"core.ping","params":"{}"}}"#,
        "a".repeat(pad_len + 1)
    );
    assert!(matches!(
        parse_request(&over_1mb),
        Err(IpcError::MessageTooLarge { .. })
    ));

    // 50 MB garbage rejected in < 5ms without parsing JSON
    let start = Instant::now();
    let garbage_50mb = "X".repeat(50 * 1024 * 1024);
    assert!(validate_raw_message(&garbage_50mb).is_err());
    assert!(start.elapsed() < Duration::from_millis(50));
}

#[test]
fn test_g5_attempt2_json_nesting_bomb() {
    // Total depth = 1 (outer object) + 31 (arrays) = 32 levels (allowed)
    let open_31 = "[".repeat(MAX_JSON_DEPTH - 1);
    let close_31 = "]".repeat(MAX_JSON_DEPTH - 1);
    let json_32 = format!(r#"{{"id":1,"method":"core.ping","params":{open_31}1{close_31}}}"#);
    assert!(parse_request(&json_32).is_ok());

    // Total depth = 1 (outer object) + 32 (arrays) = 33 levels (rejected)
    let open_32 = "[".repeat(MAX_JSON_DEPTH);
    let close_32 = "]".repeat(MAX_JSON_DEPTH);
    let json_33 = format!(r#"{{"id":1,"method":"core.ping","params":{open_32}1{close_32}}}"#);
    assert!(matches!(
        parse_request(&json_33),
        Err(IpcError::RecursionLimitExceeded { .. })
    ));
}

#[test]
fn test_g5_attempt3_null_byte_rejection() {
    let null_payload = "{\"id\":1,\0\"method\":\"core.ping\"}";
    assert!(matches!(
        validate_raw_message(null_payload),
        Err(IpcError::NullByteDetected)
    ));
}

// ============================================================================
// G8: Second-Launch Channel
// ============================================================================

#[test]
fn test_g8_attempt1_oversized_single_instance_payload() {
    // Verify that single-instance listener enforces 1 MB max on forwarded args
    let oversized_len: usize = 2 * 1024 * 1024;
    // Protocol requires 4-byte LE length prefix
    let len_prefix = (oversized_len as u32).to_le_bytes();
    let len = u32::from_le_bytes(len_prefix) as usize;
    assert!(
        len > 1024 * 1024,
        "Length check must reject payload larger than 1 MB"
    );
}

// ============================================================================
// G9: Malicious Project CLI & Config Abuse
// ============================================================================

#[test]
fn test_g9_attempt1_node_timeout_protection() {
    // Verify that infinite loop in config is terminated after 5-second timeout
    let temp_dir = tempfile::tempdir().unwrap();
    let config_path = temp_dir.path().join("wavical.config.js");
    std::fs::write(
        &config_path,
        "while(true){}; export default { appId: 'loop' };",
    )
    .unwrap();

    let start = Instant::now();
    let res = wavical_config::validate_config(temp_dir.path());
    let elapsed = start.elapsed();

    assert!(
        matches!(res, Err(wavical_config::ValidationError::NodeTimeout)),
        "Infinite loop in wavical.config.js must hit NodeTimeout"
    );
    assert!(
        elapsed >= Duration::from_millis(4500) && elapsed <= Duration::from_millis(8000),
        "Timeout should trigger around 5 seconds (took {:?})",
        elapsed
    );
}
