use std::fs;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wavical_netpolicy::NetworkMode;
use wavical_netpolicy::html::hash_inline_script;
use wavical_netpolicy::limits::{
    MAX_OPEN_EXTERNAL_URL_LEN, MAX_RECORDED_EVENTS_PER_SESSION, OPEN_EXTERNAL_RATE_LIMIT_COUNT,
};
use wavical_netpolicy::origin::extract_origin;
use wavical_protocol::HEADER_NOSNIFF;
use wavical_shell::net_policy::{
    BlockRecorder, HEADER_DNS_PREFETCH_CONTROL, OpenExternalError, OpenExternalRateLimiter,
    ShellNetworkPolicy, ULTRA_SAFE_BROWSER_ARGS, authorize_bridge_request,
    handle_navigation_with_policy, handle_protocol_request_with_policy, open_external_validated,
    resolve_shell_network_policy,
};
use wavical_shell::{DEFAULT_CSP, HEADER_CSP, WAVICAL_INIT_SCRIPT};
use wry::http::{Request, StatusCode, Uri};

#[test]
fn built_app_cannot_be_loosened_and_missing_or_invalid_manifest_falls_back_to_ultra_safe() {
    let tmp = TempDir::new().unwrap();
    let dist = tmp.path();
    fs::write(dist.join("index.html"), b"<h1>hi</h1>").unwrap();

    // Missing manifest -> falls back to ultra-safe even if dev overrides are passed
    let p_missing = resolve_shell_network_policy(
        dist,
        None,
        false,
        Some("open"),
        Some("http://localhost:5173"),
        vec!["core.ping".into()],
    );
    assert_eq!(p_missing.mode, NetworkMode::UltraSafe);
    assert!(p_missing.dev_url_origin.is_none());

    // Invalid manifest mode -> falls back to ultra-safe
    fs::write(
        dist.join("wavical-manifest.json"),
        br#"{"appId":"a","version":"0.1.0","mode":"totally-open","assets":{}}"#,
    )
    .unwrap();
    let p_invalid = resolve_shell_network_policy(
        dist,
        None,
        false,
        Some("open"),
        Some("http://localhost:5173"),
        vec![],
    );
    assert_eq!(p_invalid.mode, NetworkMode::UltraSafe);

    // Valid open manifest -> mode is open, dev.url still ignored in built app
    fs::write(
        dist.join("wavical-manifest.json"),
        br#"{"appId":"a","version":"0.1.0","mode":"open","assets":{}}"#,
    )
    .unwrap();
    let p_open = resolve_shell_network_policy(
        dist,
        None,
        false,
        Some("ultra-safe"),
        Some("http://localhost:5173"),
        vec![],
    );
    assert_eq!(p_open.mode, NetworkMode::Open);
    assert!(p_open.dev_url_origin.is_none());

    // Safe manifest with tampered allowlist -> falls back to ultra-safe
    fs::write(dist.join("wavical.allowlist.json"), br#"{"version":1}"#).unwrap();
    fs::write(
        dist.join("wavical-manifest.json"),
        br#"{"appId":"a","version":"0.1.0","mode":"safe","allowlistSha256":"0000","assets":{}}"#,
    )
    .unwrap();
    let p_tampered = resolve_shell_network_policy(dist, None, false, None, None, vec![]);
    assert_eq!(p_tampered.mode, NetworkMode::UltraSafe);
}

#[test]
fn protocol_responses_have_csp_nosniff_and_serve_time_hash_check() {
    let tmp = TempDir::new().unwrap();
    let dist = tmp.path();
    let index_bytes = b"<!doctype html><script>console.log(1);</script>";
    fs::write(dist.join("index.html"), index_bytes).unwrap();
    let index_sha = hex::encode(Sha256::digest(index_bytes));

    let al_bytes = br#"{"version":1,"scripts":["https://cdn.example.com"]}"#;
    fs::write(dist.join("wavical.allowlist.json"), al_bytes).unwrap();
    let al_sha = hex::encode(Sha256::digest(al_bytes));

    let script_hash = hash_inline_script(b"console.log(1);");
    let manifest_json = format!(
        r#"{{
            "appId":"a",
            "version":"0.1.0",
            "mode":"safe",
            "allowlistSha256":"{al_sha}",
            "inlineScriptHashes":{{"index.html":["{}"]}},
            "assets":{{"index.html":"{index_sha}"}}
        }}"#,
        script_hash.as_csp_token()
    );
    fs::write(dist.join("wavical-manifest.json"), manifest_json).unwrap();

    let policy = resolve_shell_network_policy(dist, None, false, None, None, vec![]);
    assert_eq!(policy.mode, NetworkMode::Safe);

    // 200 OK includes safe CSP with exact script hash + nosniff
    let req = Request::builder()
        .uri("wavical://localhost/index.html")
        .body(vec![])
        .unwrap();
    let resp = handle_protocol_request_with_policy(req, dist, &policy, "win1");
    assert_eq!(resp.status(), StatusCode::OK);
    let csp = resp.headers().get(HEADER_CSP).unwrap().to_str().unwrap();
    assert!(csp.contains(&script_hash.as_csp_token()));
    assert!(csp.contains("https://cdn.example.com"));
    assert_eq!(
        resp.headers().get(HEADER_NOSNIFF.0).unwrap(),
        HEADER_NOSNIFF.1
    );

    // Modified inline script hash does NOT match CSP
    let modified_hash = hash_inline_script(b"console.log(2);");
    assert!(!csp.contains(&modified_hash.as_csp_token()));

    // Rejected path traversal returns 404 with CSP and nosniff
    let bad_req = Request::builder()
        .uri("wavical://localhost/../secret.txt")
        .body(vec![])
        .unwrap();
    let bad_resp = handle_protocol_request_with_policy(bad_req, dist, &policy, "win1");
    assert_eq!(bad_resp.status(), StatusCode::NOT_FOUND);
    assert!(bad_resp.headers().get(HEADER_CSP).is_some());
    assert_eq!(
        bad_resp.headers().get(HEADER_NOSNIFF.0).unwrap(),
        HEADER_NOSNIFF.1
    );

    // Tamper 1 byte on disk -> serve-time hash check fails and returns 404
    fs::write(
        dist.join("index.html"),
        b"<!doctype html><script>console.log(2);</script>",
    )
    .unwrap();
    let req2 = Request::builder()
        .uri("wavical://localhost/index.html")
        .body(vec![])
        .unwrap();
    let resp2 = handle_protocol_request_with_policy(req2, dist, &policy, "win1");
    assert_eq!(resp2.status(), StatusCode::NOT_FOUND);
    assert!(resp2.headers().get(HEADER_CSP).is_some());
}

#[test]
fn ultra_safe_csp_unchanged_and_leak_channels_closed() {
    let tmp = TempDir::new().unwrap();
    let dist = tmp.path();
    fs::write(dist.join("index.html"), b"<h1>ultra</h1>").unwrap();
    let policy = resolve_shell_network_policy(dist, None, false, None, None, vec![]);
    assert_eq!(policy.mode, NetworkMode::UltraSafe);

    let req = Request::builder()
        .uri("wavical://localhost/index.html")
        .body(vec![])
        .unwrap();
    let resp = handle_protocol_request_with_policy(req, dist, &policy, "win1");
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get(HEADER_CSP).unwrap(), DEFAULT_CSP);
    assert_eq!(
        resp.headers().get(HEADER_DNS_PREFETCH_CONTROL).unwrap(),
        "off"
    );

    let bad_req = Request::builder()
        .uri("wavical://localhost/../x")
        .body(vec![])
        .unwrap();
    let bad_resp = handle_protocol_request_with_policy(bad_req, dist, &policy, "win1");
    assert_eq!(bad_resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(bad_resp.headers().get(HEADER_CSP).unwrap(), DEFAULT_CSP);
    assert_eq!(
        bad_resp.headers().get(HEADER_DNS_PREFETCH_CONTROL).unwrap(),
        "off"
    );

    assert!(ULTRA_SAFE_BROWSER_ARGS.contains("--dns-prefetch-disable"));
    assert!(ULTRA_SAFE_BROWSER_ARGS.contains("OptimizationHints"));
    assert!(ULTRA_SAFE_BROWSER_ARGS.contains("NetworkTimeServiceQuerying"));
    assert!(ULTRA_SAFE_BROWSER_ARGS.contains("msWebOOUI,msPdfOOUI,msSmartScreenProtection"));
}

#[test]
fn multi_origin_local_server_and_baseline_in_all_three_modes() {
    // Multi-origin local listeners (distinct ephemeral ports = distinct RFC 6454 origins)
    let l_allowed = TcpListener::bind("127.0.0.1:0").unwrap();
    let p_allowed = l_allowed.local_addr().unwrap().port();
    let l_blocked = TcpListener::bind("127.0.0.1:0").unwrap();
    let p_blocked = l_blocked.local_addr().unwrap().port();

    let allowed_origin = format!("http://127.0.0.1:{p_allowed}");
    let blocked_origin = format!("http://127.0.0.1:{p_blocked}");

    let tmp = TempDir::new().unwrap();
    let al_json = format!(
        r#"{{
            "version": 1,
            "openExternal": ["https://docs.company.com"],
            "remoteUi": {{ "origin": "https://app.company.com", "bridge": ["core.ping"] }},
            "localExceptions": [{{ "origin": "{allowed_origin}", "reason": "local test origin" }}]
        }}"#
    );
    fs::write(tmp.path().join("wavical.allowlist.json"), al_json).unwrap();

    for mode in [NetworkMode::Open, NetworkMode::Safe, NetworkMode::UltraSafe] {
        let mode_str = match mode {
            NetworkMode::Open => "open",
            NetworkMode::Safe => "safe",
            NetworkMode::UltraSafe => "ultra-safe",
        };
        let policy = resolve_shell_network_policy(
            tmp.path(),
            Some(tmp.path()),
            true,
            Some(mode_str),
            Some("http://localhost:5173"),
            vec!["core.ping".into(), "notify.send".into()],
        );

        // Baseline: blocked schemes denied in EVERY mode
        for bad in [
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "data:text/html,<script>",
            "blob:https://app.company.com/1",
            "ftp://example.com/a",
            "ms-settings:privacy",
        ] {
            assert!(!handle_navigation_with_policy(&policy, bad, "win1"));
            let rl = OpenExternalRateLimiter::default();
            assert!(matches!(
                open_external_validated(&policy, bad, "win1", &rl, |_| Ok(())),
                Err(OpenExternalError::PolicyDenied(_))
            ));
        }

        // Baseline: bridge denied for iframes and unconfigured origins in EVERY mode
        let app_uri: Uri = "http://wavical.localhost/index.html".parse().unwrap();
        assert!(authorize_bridge_request(&policy, &app_uri, true, "win1").is_ok());
        assert!(authorize_bridge_request(&policy, &app_uri, false, "win1").is_err());

        let evil_uri: Uri = format!("{blocked_origin}/").parse().unwrap();
        assert!(authorize_bridge_request(&policy, &evil_uri, true, "win1").is_err());

        // dev.url bridge works in dev command in EVERY mode
        let dev_uri: Uri = "http://localhost:5173/".parse().unwrap();
        assert!(authorize_bridge_request(&policy, &dev_uri, true, "win1").is_ok());

        // localExceptions: allowed in open & safe, refused in ultra-safe
        let can_nav_local = handle_navigation_with_policy(&policy, &allowed_origin, "win1");
        let can_nav_blocked = handle_navigation_with_policy(&policy, &blocked_origin, "win1");
        assert!(!can_nav_blocked);
        match mode {
            NetworkMode::Open | NetworkMode::Safe => assert!(can_nav_local),
            NetworkMode::UltraSafe => assert!(!can_nav_local),
        }
    }

    // Init script guards against iframes
    assert!(WAVICAL_INIT_SCRIPT.contains("window.top !== window.self"));
    assert_ne!(
        extract_origin(&allowed_origin).unwrap(),
        extract_origin(&blocked_origin).unwrap()
    );
}

#[test]
fn open_external_length_https_and_rate_limit_enforced() {
    let policy = ShellNetworkPolicy {
        mode: NetworkMode::Open,
        ..Default::default()
    };

    let rl = OpenExternalRateLimiter::default();
    let launched = Arc::new(AtomicUsize::new(0));

    // HTTP rejected even in open mode
    assert!(matches!(
        open_external_validated(&policy, "http://example.com", "win1", &rl, |_| Ok(())),
        Err(OpenExternalError::PolicyDenied(_))
    ));

    // Over 2048 chars rejected
    let too_long = format!(
        "https://example.com/{}",
        "a".repeat(MAX_OPEN_EXTERNAL_URL_LEN)
    );
    assert!(matches!(
        open_external_validated(&policy, &too_long, "win1", &rl, |_| Ok(())),
        Err(OpenExternalError::PolicyDenied(_))
    ));

    // 5 calls allowed, 6th rate-limited
    for _ in 0..OPEN_EXTERNAL_RATE_LIMIT_COUNT {
        let c = launched.clone();
        assert!(
            open_external_validated(
                &policy,
                "https://example.com/docs",
                "win1",
                &rl,
                move |_| {
                    c.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }
            )
            .is_ok()
        );
    }
    assert_eq!(
        launched.load(Ordering::Relaxed),
        OPEN_EXTERNAL_RATE_LIMIT_COUNT
    );
    assert_eq!(
        open_external_validated(&policy, "https://example.com/docs", "win1", &rl, |_| Ok(())),
        Err(OpenExternalError::RateLimited)
    );

    // Rate limiter resets after window
    let rl2 = OpenExternalRateLimiter::default();
    let t0 = Instant::now();
    for _ in 0..OPEN_EXTERNAL_RATE_LIMIT_COUNT {
        assert!(rl2.check_and_record(t0));
    }
    assert!(!rl2.check_and_record(t0));
    assert!(rl2.check_and_record(t0 + std::time::Duration::from_secs(11)));
}

#[test]
fn built_app_ignores_dev_url_and_remote_ui_bridge_subset_enforced() {
    let tmp = TempDir::new().unwrap();
    let mut policy = resolve_shell_network_policy(
        tmp.path(),
        None,
        false,
        Some("open"),
        Some("http://localhost:5173"),
        vec!["core.ping".into(), "notify.send".into()],
    );
    let dev_uri: Uri = "http://localhost:5173/".parse().unwrap();
    assert!(authorize_bridge_request(&policy, &dev_uri, true, "win1").is_err());

    // Safe mode remoteUi bridge returns only remoteUi.bridge subset
    let al = wavical_netpolicy::allowlist::Allowlist::parse_bytes(
        br#"{"version":1,"remoteUi":{"origin":"https://app.company.com","bridge":["core.ping"]}}"#,
    )
    .unwrap();
    policy.mode = NetworkMode::Safe;
    policy.allowlist = Some(al);
    policy.remote_url_origin = Some(extract_origin("https://app.company.com").unwrap());

    let rui_uri: Uri = "https://app.company.com/dashboard".parse().unwrap();
    let granted = authorize_bridge_request(&policy, &rui_uri, true, "win1").unwrap();
    assert_eq!(granted, vec!["core.ping".to_string()]);
    assert!(authorize_bridge_request(&policy, &rui_uri, false, "win1").is_err());
}

#[test]
fn recorder_caps_at_10000_and_strips_queries_and_paths() {
    let tmp = TempDir::new().unwrap();
    let rec_path = tmp.path().join(".wavical").join("recorded-blocks.json");
    let rec = BlockRecorder::new(Some(rec_path.clone()));

    rec.record_raw(
        "scripts",
        "https://cdn.example.com/v1/app.js?token=SECRET_MARKER_QUERY_123&user=bob#frag",
    );
    let snap = rec.snapshot();
    assert_eq!(snap.total_events, 1);
    assert_eq!(snap.entries.len(), 1);
    assert_eq!(snap.entries[0].block_type, "scripts");
    assert_eq!(snap.entries[0].origin, "https://cdn.example.com");
    assert_eq!(snap.entries[0].count, 1);

    let on_disk = fs::read_to_string(&rec_path).unwrap();
    assert!(!on_disk.contains("SECRET_MARKER_QUERY_123"));
    assert!(!on_disk.contains("/v1/app.js"));

    for i in 0..(MAX_RECORDED_EVENTS_PER_SESSION + 50) {
        rec.record_raw(
            "images",
            &format!("https://img{}.example.com/pic.png?q=1", i % 5),
        );
    }
    let capped = rec.snapshot();
    assert_eq!(capped.total_events, MAX_RECORDED_EVENTS_PER_SESSION);
}
