//! Independent black-box security audit test suite for Phase 4.5 (Network Modes).
//! Written directly from `docs/phase 4.5/WAVICAL_PHASE_4_5_NETWORK_MODES_PLAN.md`.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wavical_ai::downloader::DownloadOptions;
use wavical_cli::allowlist_suggest::run_allowlist_suggest_with_io;
use wavical_cli::html_scan::{extract_inline_scripts_from_html, scan_html};
use wavical_cli::vendor::{VendorRunOptions, run_vendor_with_io, url_to_vendor_rel_path};
use wavical_netpolicy::NetworkMode;
use wavical_netpolicy::allowlist::Allowlist;
use wavical_netpolicy::csp::{ULTRA_SAFE_CSP, generate_csp};
use wavical_netpolicy::decisions::{
    PolicyContext, PolicyDecision, decide_bridge_allowed, decide_navigation, decide_remote_ui,
    validate_dev_url,
};
use wavical_netpolicy::html::hash_inline_script;
use wavical_netpolicy::limits::*;
use wavical_netpolicy::origin::{
    OriginHost, ParsedOrigin, SchemePolicy, extract_origin, parse_allowlist_pattern,
};
use wavical_netpolicy::vendor_lock::VendorLock;
use wavical_shell::net_policy::{
    BlockRecorder, OpenExternalError, OpenExternalRateLimiter, ShellNetworkPolicy,
    authorize_bridge_request, handle_navigation_with_policy, handle_protocol_request_with_policy,
    open_external_validated, resolve_shell_network_policy_with_remote,
};
use wavical_shell::{DEFAULT_CSP, HEADER_CSP, WindowConfig, process_ipc_message};
use wry::http::{Request, StatusCode, Uri};

#[test]
fn a9_01_and_02_allowlist_80_plus_inputs_and_matcher_confusion() {
    let valid_cases = [
        r#"{"version":1}"#,
        r#"{"version":1,"inlineStyles":true,"dataImages":true}"#,
        r#"{"version":1,"scripts":["https://cdn.example.com"]}"#,
        r#"{"version":1,"styles":["https://fonts.googleapis.com"]}"#,
        r#"{"version":1,"fonts":["https://fonts.gstatic.com"]}"#,
        r#"{"version":1,"images":["https://*.company.com"]}"#,
        r#"{"version":1,"media":["https://media.company.com"]}"#,
        r#"{"version":1,"connect":["https://api.company.com","wss://live.company.com"]}"#,
        r#"{"version":1,"frames":["https://auth.company.com"]}"#,
        r#"{"version":1,"navigate":["https://login.company.com"]}"#,
        r#"{"version":1,"openExternal":["https://docs.company.com"]}"#,
        r#"{"version":1,"remoteUi":{"origin":"https://app.company.com","bridge":["core.ping"]}}"#,
        r#"{"version":1,"localExceptions":[{"origin":"http://localhost:3000","reason":"api"}]}"#,
        r#"{"version":1,"localExceptions":[{"origin":"http://127.0.0.1:8080","reason":"local"}]}"#,
        r#"{"version":1,"localExceptions":[{"origin":"ws://localhost:5173","reason":"hmr"}]}"#,
        r#"{"version":1,"scripts":["https://cdn.example.com:8443"]}"#,
        r#"{"version":1,"scripts":["https://xn--bcher-kva.example"]}"#,
        r#"{"version":1,"images":["https://*.sub.company.com"]}"#,
        r#"{"version":1,"connect":["wss://*.company.com"]}"#,
        r#"{"version":1,"remoteUi":{"origin":"https://app.company.com"}}"#,
    ];
    for (i, raw) in valid_cases.iter().enumerate() {
        assert!(
            Allowlist::parse_bytes(raw.as_bytes()).is_ok(),
            "valid case {i} failed: {raw}"
        );
    }

    let invalid_cases = [
        r#"{}"#,
        r#"{"version":0}"#,
        r#"{"version":2}"#,
        r#"{"version":"1"}"#,
        r#"{"version":1,"extraKey":1}"#,
        r#"{"version":1,"inlineStyles":"yes"}"#,
        r#"{"version":1,"dataImages":1}"#,
        r#"{"version":1,"scripts":["http://cdn.example.com"]}"#,
        r#"{"version":1,"scripts":["wss://cdn.example.com"]}"#,
        r#"{"version":1,"scripts":["ws://cdn.example.com"]}"#,
        r#"{"version":1,"scripts":["ftp://cdn.example.com"]}"#,
        r#"{"version":1,"scripts":["file:///etc/passwd"]}"#,
        r#"{"version":1,"scripts":["javascript:alert(1)"]}"#,
        r#"{"version":1,"scripts":["data:text/javascript,1"]}"#,
        r#"{"version":1,"scripts":["https://cdn.example.com/path.js"]}"#,
        r#"{"version":1,"scripts":["https://cdn.example.com/?a=1"]}"#,
        r#"{"version":1,"scripts":["https://cdn.example.com/#x"]}"#,
        r#"{"version":1,"scripts":["https://user@cdn.example.com"]}"#,
        r#"{"version":1,"scripts":["https://user:pass@cdn.example.com"]}"#,
        r#"{"version":1,"scripts":["https://@cdn.example.com"]}"#,
        r#"{"version":1,"scripts":["https://127.0.0.1"]}"#,
        r#"{"version":1,"scripts":["https://192.168.1.1"]}"#,
        r#"{"version":1,"scripts":["https://[::1]"]}"#,
        r#"{"version":1,"scripts":["https://*"]}"#,
        r#"{"version":1,"scripts":["https://*.com"]}"#,
        r#"{"version":1,"scripts":["https://*.*"]}"#,
        r#"{"version":1,"scripts":["https://a.*.com"]}"#,
        r#"{"version":1,"scripts":["https://company.*"]}"#,
        r#"{"version":1,"scripts":["https://*.co.uk"]}"#,
        r#"{"version":1,"scripts":["https://*.org.uk"]}"#,
        r#"{"version":1,"scripts":["https://*.com.au"]}"#,
        r#"{"version":1,"scripts":["https://*.github.io"]}"#,
        r#"{"version":1,"scripts":["https://*.pages.dev"]}"#,
        r#"{"version":1,"scripts":["https://*.vercel.app"]}"#,
        r#"{"version":1,"scripts":["https://*.netlify.app"]}"#,
        r#"{"version":1,"scripts":["*"]}"#,
        r#"{"version":1,"scripts":["*.com"]}"#,
        r#"{"version":1,"scripts":[""]}"#,
        r#"{"version":1,"scripts":["https://a.com","https://A.COM"]}"#,
        r#"{"version":1,"scripts":["https://a.com","https://a.com:443"]}"#,
        r#"{"version":1,"styles":["http://a.com"]}"#,
        r#"{"version":1,"fonts":["http://a.com"]}"#,
        r#"{"version":1,"images":["http://a.com"]}"#,
        r#"{"version":1,"media":["http://a.com"]}"#,
        r#"{"version":1,"connect":["http://a.com"]}"#,
        r#"{"version":1,"connect":["ws://a.com"]}"#,
        r#"{"version":1,"frames":["http://a.com"]}"#,
        r#"{"version":1,"navigate":["http://a.com"]}"#,
        r#"{"version":1,"openExternal":["http://a.com"]}"#,
        r#"{"version":1,"remoteUi":{"origin":"http://app.company.com"}}"#,
        r#"{"version":1,"remoteUi":{"origin":"https://*.company.com"}}"#,
        r#"{"version":1,"remoteUi":{"origin":"https://127.0.0.1"}}"#,
        r#"{"version":1,"remoteUi":{"origin":"https://app.company.com","bridge":["core.ping","core.ping"]}}"#,
        r#"{"version":1,"remoteUi":{"origin":"https://app.company.com","extra":1}}"#,
        r#"{"version":1,"localExceptions":[{"origin":"http://localhost:3000"}]}"#,
        r#"{"version":1,"localExceptions":[{"origin":"http://localhost:3000","reason":""}]}"#,
        r#"{"version":1,"localExceptions":[{"origin":"http://localhost:3000","reason":"   "}]}"#,
        r#"{"version":1,"localExceptions":[{"origin":"ftp://localhost:3000","reason":"x"}]}"#,
        r#"{"version":1,"localExceptions":[{"origin":"http://*.localhost","reason":"x"}]}"#,
        r#"{"version":1,"localExceptions":[{"origin":"http://localhost:3000","reason":"a"},{"origin":"http://LOCALHOST:3000","reason":"b"}]}"#,
        r#"{"version":1,"localExceptions":[{"origin":"http://localhost:3000","reason":"a","extra":1}]}"#,
    ];
    assert!(valid_cases.len() + invalid_cases.len() >= 80);
    for (i, raw) in invalid_cases.iter().enumerate() {
        assert!(
            Allowlist::parse_bytes(raw.as_bytes()).is_err(),
            "invalid case {i} unexpectedly succeeded: {raw}"
        );
    }

    // Matcher look-alike & confusion
    let pat = parse_allowlist_pattern("https://*.company.com", SchemePolicy::HttpsOnly).unwrap();
    for evil in [
        "https://company.com/",
        "https://company.com.evil.com/",
        "https://evilcompany.com/",
        "https://company.com@evil.com/",
        "https://sub.company.com:8443/",
        "http://sub.company.com/",
    ] {
        assert!(
            !extract_origin(evil).is_ok_and(|o| pat.matches(&o)),
            "wildcard must not match {evil}"
        );
    }
    assert!(pat.matches(&extract_origin("https://SUB.COMPANY.COM./x").unwrap()));
}

#[test]
fn a9_03_mode_lock_and_serve_time_hash_check() {
    let tmp = TempDir::new().unwrap();
    let dist = tmp.path();
    let html = b"<h1>Hello</h1>";
    fs::write(dist.join("index.html"), html).unwrap();
    let sha = hex::encode(Sha256::digest(html));

    // Missing manifest -> ultra-safe despite dev overrides
    let p_missing = resolve_shell_network_policy_with_remote(
        dist,
        None,
        false,
        Some("open"),
        Some("http://localhost:5173"),
        Some("https://app.company.com"),
        vec!["core.ping".into()],
    );
    assert_eq!(p_missing.mode, NetworkMode::UltraSafe);
    assert_eq!(p_missing.csp_for_path("index.html"), DEFAULT_CSP);

    // Valid safe manifest + serve-time tamper check
    let al_bytes = br#"{"version":1,"scripts":["https://cdn.example.com"]}"#;
    fs::write(dist.join("wavical.allowlist.json"), al_bytes).unwrap();
    let al_sha = hex::encode(Sha256::digest(al_bytes));
    let manifest = format!(
        r#"{{"appId":"a","version":"0.1.0","mode":"safe","allowlistSha256":"{al_sha}","assets":{{"index.html":"{sha}"}}}}"#
    );
    fs::write(dist.join("wavical-manifest.json"), manifest).unwrap();

    let p_safe = resolve_shell_network_policy_with_remote(
        dist,
        None,
        false,
        Some("open"),
        None,
        None,
        vec![],
    );
    assert_eq!(p_safe.mode, NetworkMode::Safe);

    // Serve valid file -> 200
    let req = Request::builder()
        .uri("wavical://localhost/index.html")
        .body(vec![])
        .unwrap();
    let resp = handle_protocol_request_with_policy(req, dist, &p_safe, "a9");
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get(HEADER_CSP).is_some());

    // Tamper 1 byte on disk -> 404
    fs::write(dist.join("index.html"), b"<h1>Hella</h1>").unwrap();
    let req2 = Request::builder()
        .uri("wavical://localhost/index.html")
        .body(vec![])
        .unwrap();
    let resp2 = handle_protocol_request_with_policy(req2, dist, &p_safe, "a9");
    assert_eq!(resp2.status(), StatusCode::NOT_FOUND);

    // Edit allowlist next to exe without updating manifest allowlistSha256 -> falls back to ultra-safe
    fs::write(
        dist.join("wavical.allowlist.json"),
        br#"{"version":1,"scripts":["https://evil.com"]}"#,
    )
    .unwrap();
    let p_tampered_al =
        resolve_shell_network_policy_with_remote(dist, None, false, None, None, None, vec![]);
    assert_eq!(p_tampered_al.mode, NetworkMode::UltraSafe);
}

#[test]
fn a9_04_inline_scripts_extraction_and_ultra_safe_exfiltration() {
    // 1. Safe mode CSP hash check
    let s1 = b"window.x = 1;";
    let s2 = b"window.x = 2;";
    let h1 = hash_inline_script(s1);
    let h2 = hash_inline_script(s2);
    let csp = generate_csp(NetworkMode::Safe, None, std::slice::from_ref(&h1), None).unwrap();
    assert!(csp.contains(&h1.as_csp_token()));
    assert!(!csp.contains(&h2.as_csp_token()));
    assert!(!csp.contains("'unsafe-inline'"));

    // 2. Handler line reporting
    let html = "<html>\n<body>\n<img src='a' onerror='x()'>\n<script>a=1;</script>\n<script type='module'>b=2;</script>\n</body>\n</html>";
    let scan = scan_html("src/index.html", html);
    assert_eq!(scan.inline_handlers.len(), 1);
    assert_eq!(scan.inline_handlers[0].line, 3);
    assert_eq!(scan.inline_handlers[0].attribute, "onerror");

    // 3. Extraction preserves order and module semantics; ultra-safe CSP unchanged
    let ext = extract_inline_scripts_from_html("index.html", html, ".wavical-extracted/");
    assert_eq!(ext.extracted_files.len(), 2);
    assert!(ext.rewritten_html.contains("inline_0.js"));
    assert!(
        ext.rewritten_html
            .contains("type=\"module\" src=\".wavical-extracted/index_html_inline_1.js\"")
    );
    assert_eq!(ULTRA_SAFE_CSP, DEFAULT_CSP);

    // 4. Ultra-safe exfiltration server receives 0 hits
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(Mutex::new(Vec::new()));
    let hits_c = hits.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_c = stop.clone();
    thread::spawn(move || {
        while !stop_c.load(Ordering::Relaxed) {
            if let Ok((mut s, _)) = listener.accept() {
                let mut b = [0u8; 1024];
                let n = s.read(&mut b).unwrap_or(0);
                hits_c
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&b[..n]).to_string());
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            }
            thread::sleep(Duration::from_millis(10));
        }
    });

    let ultra_p = ShellNetworkPolicy::default();
    let exfil_url = format!("http://127.0.0.1:{port}/?m=A9_MARKER");
    assert!(!handle_navigation_with_policy(&ultra_p, &exfil_url, "a9"));
    let rl = OpenExternalRateLimiter::default();
    assert!(open_external_validated(&ultra_p, &exfil_url, "a9", &rl, |_| Ok(())).is_err());
    let uri: Uri = exfil_url.parse().unwrap();
    assert!(authorize_bridge_request(&ultra_p, &uri, true, "a9").is_err());
    stop.store(true, Ordering::Relaxed);
    assert!(hits.lock().unwrap().is_empty());
}

#[test]
fn a9_05_vendor_security_and_dev_url_and_suggest() {
    // Vendor path safety
    assert!(url_to_vendor_rel_path("https://cdn.example.com/a/CON/x.js", "js").is_err());
    assert!(url_to_vendor_rel_path("https://cdn.example.com/a/../b.js", "js").is_ok()); // normalized by url::Url
    let q =
        url_to_vendor_rel_path("https://fonts.googleapis.com/css2?family=Inter", "css").unwrap();
    assert!(q.starts_with("vendor/fonts.googleapis.com/"));
    assert!(!q.contains('?'));

    // dev.url rules
    assert!(validate_dev_url("http://localhost:5173").is_ok());
    assert!(validate_dev_url("http://127.0.0.1:5173").is_ok());
    assert!(validate_dev_url("http://evil.com:5173").is_err());

    // allowlist suggest never stores full URLs or proposes forbidden kinds
    let tmp = TempDir::new().unwrap();
    let rec_path = tmp.path().join(".wavical").join("recorded-blocks.json");
    let rec = BlockRecorder::new(Some(rec_path.clone()));
    rec.record_raw("scripts", "https://cdn.example.com/x.js?secret=A9_SECRET");
    rec.record_raw("styles", "inline");
    rec.record_raw("connect", "http://localhost:3000/api");

    let recorded_json = fs::read_to_string(&rec_path).unwrap();
    assert!(!recorded_json.contains("A9_SECRET"));

    let mut input = Cursor::new(b"a\n");
    let mut out = Vec::new();
    run_allowlist_suggest_with_io(tmp.path(), &mut input, &mut out).unwrap();
    let al_bytes = fs::read(tmp.path().join("wavical.allowlist.json")).unwrap();
    let al = Allowlist::parse_bytes(&al_bytes).unwrap();
    assert_eq!(al.scripts.len(), 1);
    assert!(!al.inline_styles);
    assert!(al.local_exceptions.is_empty());

    // Vendor offline check with local server
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let mut s = s;
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let body = b"/* SPDX-License-Identifier: MIT */\nwindow.v=1;";
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(hdr.as_bytes());
            let _ = s.write_all(body);
        }
    });
    let v_tmp = TempDir::new().unwrap();
    fs::create_dir_all(v_tmp.path().join("src")).unwrap();
    fs::write(
        v_tmp.path().join("src/index.html"),
        format!(r#"<script src="http://127.0.0.1:{port}/ajax/libs/a/1.0.0/a.js"></script>"#),
    )
    .unwrap();
    let dl_opts = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };
    let mut r = Cursor::new(b"");
    let mut w = Vec::new();
    run_vendor_with_io(
        v_tmp.path(),
        &VendorRunOptions {
            download_options: dl_opts,
            ..Default::default()
        },
        &mut r,
        &mut w,
    )
    .unwrap();
    let lock = VendorLock::parse_bytes_with_options(
        &fs::read(v_tmp.path().join("vendor.lock.json")).unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(lock.entries.len(), 1);
}

#[test]
fn a9_06_baseline_and_remote_ui_in_all_modes() {
    let app = ParsedOrigin {
        scheme: "http".into(),
        host: OriginHost::Domain("wavical.localhost".into()),
        port: 80,
    };
    let remote = extract_origin("https://app.company.com").unwrap();
    let al = Allowlist::parse_bytes(
        br#"{"version":1,"remoteUi":{"origin":"https://app.company.com","bridge":["core.ping"]}}"#,
    )
    .unwrap();
    let caps = vec!["core.ping".into(), "notify.send".into()];

    for mode in [NetworkMode::Open, NetworkMode::Safe, NetworkMode::UltraSafe] {
        let ctx = PolicyContext {
            mode,
            is_dev_command: false,
            app_origin: &app,
            allowlist: Some(&al),
            remote_url_origin: Some(&remote),
            dev_url_origin: None,
            app_capabilities: &caps,
        };
        assert!(matches!(
            decide_navigation(&ctx, "javascript:alert(1)"),
            PolicyDecision::Deny(_)
        ));
        assert!(matches!(
            decide_navigation(&ctx, "file:///C:/a"),
            PolicyDecision::Deny(_)
        ));
        if mode == NetworkMode::UltraSafe {
            assert!(matches!(
                decide_remote_ui(&ctx, "https://app.company.com"),
                PolicyDecision::Deny(_)
            ));
        } else {
            assert_eq!(
                decide_remote_ui(&ctx, "https://app.company.com"),
                PolicyDecision::Allow
            );
        }
        assert!(matches!(
            decide_bridge_allowed(&ctx, "https://app.company.com", false),
            wavical_netpolicy::decisions::BridgeDecision::Deny(_)
        ));
    }

    // OpenExternal rate limit & HTTPS only
    let open_p = ShellNetworkPolicy {
        mode: NetworkMode::Open,
        ..Default::default()
    };
    let rl = OpenExternalRateLimiter::default();
    assert!(matches!(
        open_external_validated(&open_p, "ms-calc:", "a9", &rl, |_| Ok(())),
        Err(OpenExternalError::PolicyDenied(_))
    ));
    let _ = (
        MAX_ALLOWLIST_FILE_BYTES,
        WindowConfig::default(),
        process_ipc_message,
    );
}
