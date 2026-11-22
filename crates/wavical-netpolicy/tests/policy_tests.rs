use proptest::prelude::*;
use wavical_netpolicy::allowlist::{Allowlist, AllowlistError};
use wavical_netpolicy::csp::{CspError, ULTRA_SAFE_CSP, generate_csp};
use wavical_netpolicy::decisions::{
    BridgeDecision, NavigationDecision, PolicyContext, PolicyDecision, decide_bridge_allowed,
    decide_navigation, decide_new_window, decide_open_external, decide_remote_ui,
    decide_top_level_navigation, is_baseline_blocked_scheme, validate_dev_url,
};
use wavical_netpolicy::html::{
    ExternalRef, ExternalRefCategory, HtmlScanResult, InlineHandlerFinding, InlineScriptBlock,
    StyleAttrFinding, hash_inline_script,
};
use wavical_netpolicy::limits::*;
use wavical_netpolicy::origin::{
    OriginHost, ParsedOrigin, SchemePolicy, extract_origin, parse_allowlist_pattern,
};
use wavical_netpolicy::vendor_lock::{VendorLock, VendorLockError};
use wavical_netpolicy::version::{VersionPinStatus, detect_url_version_pin};
use wavical_netpolicy::{ExternalLinksMode, NetworkMode};

fn app_origin() -> ParsedOrigin {
    ParsedOrigin {
        scheme: "http".to_string(),
        host: OriginHost::Domain("wavical.localhost".to_string()),
        port: 80,
    }
}

#[test]
fn ultra_safe_csp_matches_phase_0_default_csp_verbatim() {
    let shell_src = include_str!("../../wavical-shell/src/lib.rs");
    let marker = "pub const DEFAULT_CSP: &str = \"";
    let start = shell_src
        .find(marker)
        .expect("DEFAULT_CSP must exist in wavical-shell/src/lib.rs")
        + marker.len();
    let end = shell_src[start..]
        .find("\";")
        .expect("DEFAULT_CSP string literal terminator");
    let phase0_csp = &shell_src[start..start + end];
    assert_eq!(ULTRA_SAFE_CSP, phase0_csp);

    let generated = generate_csp(NetworkMode::UltraSafe, None, &[], None).unwrap();
    assert_eq!(generated, phase0_csp);
}

#[test]
fn sample_page_two_inline_scripts_csp_per_mode() {
    let script1 = b"console.log('hello');";
    let script2 = b"window.appReady = true;";
    let h1 = hash_inline_script(script1);
    let h2 = hash_inline_script(script2);

    let allowlist_json = br#"{
        "version": 1,
        "scripts": ["https://cdn.tailwindcss.com", "https://cdnjs.cloudflare.com"],
        "styles": ["https://fonts.googleapis.com"],
        "fonts": ["https://fonts.gstatic.com"],
        "images": ["https://*.company.com"],
        "inlineStyles": true,
        "dataImages": true
    }"#;
    let al = Allowlist::parse_bytes(allowlist_json).unwrap();

    let open_csp = generate_csp(
        NetworkMode::Open,
        Some(&al),
        &[h1.clone(), h2.clone()],
        None,
    )
    .unwrap();
    let safe_csp = generate_csp(
        NetworkMode::Safe,
        Some(&al),
        &[h1.clone(), h2.clone()],
        None,
    )
    .unwrap();
    let ultra_csp = generate_csp(
        NetworkMode::UltraSafe,
        Some(&al),
        &[h1.clone(), h2.clone()],
        None,
    )
    .unwrap();

    assert!(open_csp.contains("'unsafe-inline'"));
    assert!(open_csp.contains("object-src 'none'"));

    assert!(safe_csp.contains(&h1.as_csp_token()));
    assert!(safe_csp.contains(&h2.as_csp_token()));
    assert!(safe_csp.contains("https://cdn.tailwindcss.com"));
    assert!(safe_csp.contains("https://*.company.com"));
    assert!(safe_csp.contains("style-src 'self' https://fonts.googleapis.com 'unsafe-inline'"));
    assert!(safe_csp.contains("img-src 'self' https://*.company.com data:"));
    assert!(!safe_csp.contains("'unsafe-eval'"));

    assert_eq!(ultra_csp, ULTRA_SAFE_CSP);
    assert!(!ultra_csp.contains(&h1.as_csp_token()));
    assert!(!ultra_csp.contains("cdn.tailwindcss.com"));

    println!("SAMPLE_OPEN_CSP={open_csp}");
    println!("SAMPLE_SAFE_CSP={safe_csp}");
    println!("SAMPLE_ULTRA_SAFE_CSP={ultra_csp}");
}

#[test]
fn known_csp_hashes_and_whitespace_sensitivity() {
    // Standard MDN/CSP example: sha256("alert(1)") = "bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI="
    let h_exact = hash_inline_script(b"alert(1)");
    assert_eq!(
        h_exact.as_csp_token(),
        "'sha256-bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI='"
    );

    let h_space = hash_inline_script(b"alert(1) ");
    let h_newline = hash_inline_script(b"alert(1)\n");
    assert_ne!(h_exact, h_space);
    assert_ne!(h_exact, h_newline);
    assert_ne!(h_space, h_newline);
}

#[test]
fn matcher_table_at_least_60_cases() {
    let cases: &[(&str, SchemePolicy, &str, bool)] = &[
        // Exact matches & normalization
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://company.com/app",
            true,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://COMPANY.COM/app",
            true,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://company.com:443/app",
            true,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://company.com./app",
            true,
        ),
        (
            "https://company.com:8443",
            SchemePolicy::HttpsOnly,
            "https://company.com:8443/api",
            true,
        ),
        (
            "https://company.com:8443",
            SchemePolicy::HttpsOnly,
            "https://company.com/api",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://company.com:8443/api",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "http://company.com/app",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://sub.company.com/app",
            false,
        ),
        // Substring / suffix / look-alike / userinfo traps
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://company.com.evil.com/",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://evilcompany.com/",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://company.com@evil.com/",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://user:pass@company.com/",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "https://@company.com/",
            false,
        ),
        // Punycode look-alikes (Cyrillic 'а' U+0430 vs Latin 'a')
        (
            "https://apple.com",
            SchemePolicy::HttpsOnly,
            "https://xn--pple-43d.com/",
            false,
        ),
        (
            "https://apple.com",
            SchemePolicy::HttpsOnly,
            "https://\u{0430}pple.com/",
            false,
        ),
        (
            "https://xn--bcher-kva.example",
            SchemePolicy::HttpsOnly,
            "https://b\u{00fc}cher.example/x",
            true,
        ),
        (
            "https://B\u{00DC}CHER.EXAMPLE",
            SchemePolicy::HttpsOnly,
            "https://xn--bcher-kva.example/x",
            true,
        ),
        // Wildcard subdomain matching
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "https://a.company.com/",
            true,
        ),
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "https://a.b.company.com/",
            true,
        ),
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "https://A.B.COMPANY.COM:443/",
            true,
        ),
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "https://company.com/",
            false,
        ),
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "https://evilcompany.com/",
            false,
        ),
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "https://company.com.evil.com/",
            false,
        ),
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "https://a.company.com.evil.com/",
            false,
        ),
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "http://a.company.com/",
            false,
        ),
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "https://a.company.com:8443/",
            false,
        ),
        (
            "https://*.company.com:8443",
            SchemePolicy::HttpsOnly,
            "https://a.company.com:8443/",
            true,
        ),
        (
            "https://*.company.com:8443",
            SchemePolicy::HttpsOnly,
            "https://a.company.com/",
            false,
        ),
        (
            "https://*.company.com",
            SchemePolicy::HttpsOnly,
            "https://a.company.com@evil.com/",
            false,
        ),
        // WebSocket ws vs wss
        (
            "wss://live.company.com",
            SchemePolicy::HttpsOrWss,
            "wss://live.company.com/ws",
            true,
        ),
        (
            "wss://live.company.com",
            SchemePolicy::HttpsOrWss,
            "wss://live.company.com:443/ws",
            true,
        ),
        (
            "wss://live.company.com",
            SchemePolicy::HttpsOrWss,
            "ws://live.company.com/ws",
            false,
        ),
        (
            "wss://live.company.com",
            SchemePolicy::HttpsOrWss,
            "https://live.company.com/ws",
            false,
        ),
        (
            "https://api.company.com",
            SchemePolicy::HttpsOrWss,
            "wss://api.company.com/ws",
            false,
        ),
        (
            "wss://*.company.com",
            SchemePolicy::HttpsOrWss,
            "wss://stream.company.com/ws",
            true,
        ),
        (
            "wss://*.company.com",
            SchemePolicy::HttpsOrWss,
            "ws://stream.company.com/ws",
            false,
        ),
        (
            "wss://*.company.com",
            SchemePolicy::HttpsOrWss,
            "wss://company.com/ws",
            false,
        ),
        // localExceptions (http/ws/loopback/IPs)
        (
            "http://localhost:3000",
            SchemePolicy::LocalException,
            "http://localhost:3000/api",
            true,
        ),
        (
            "http://localhost:3000",
            SchemePolicy::LocalException,
            "http://LOCALHOST:3000/api",
            true,
        ),
        (
            "http://localhost:3000",
            SchemePolicy::LocalException,
            "http://localhost:3001/api",
            false,
        ),
        (
            "http://localhost:3000",
            SchemePolicy::LocalException,
            "https://localhost:3000/api",
            false,
        ),
        (
            "http://localhost:80",
            SchemePolicy::LocalException,
            "http://localhost/api",
            true,
        ),
        (
            "http://localhost",
            SchemePolicy::LocalException,
            "http://localhost:80/api",
            true,
        ),
        (
            "http://127.0.0.1:5173",
            SchemePolicy::LocalException,
            "http://127.0.0.1:5173/",
            true,
        ),
        (
            "http://127.0.0.1:5173",
            SchemePolicy::LocalException,
            "http://127.0.0.1:5174/",
            false,
        ),
        (
            "http://127.0.0.1:5173",
            SchemePolicy::LocalException,
            "http://localhost:5173/",
            false,
        ),
        (
            "http://[::1]:8080",
            SchemePolicy::LocalException,
            "http://[::1]:8080/x",
            true,
        ),
        (
            "http://[::1]:8080",
            SchemePolicy::LocalException,
            "http://[0:0:0:0:0:0:0:1]:8080/x",
            true,
        ),
        (
            "ws://localhost:5173",
            SchemePolicy::LocalException,
            "ws://localhost:5173/hmr",
            true,
        ),
        (
            "ws://localhost:5173",
            SchemePolicy::LocalException,
            "wss://localhost:5173/hmr",
            false,
        ),
        // Baseline blocked schemes never match
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "file:///C:/Windows/System32/calc.exe",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "javascript:alert(1)",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "data:text/html,<script>alert(1)</script>",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "blob:https://company.com/1234",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "ftp://company.com/file",
            false,
        ),
        (
            "https://company.com",
            SchemePolicy::HttpsOnly,
            "ms-settings:privacy",
            false,
        ),
        // Path/query/port variations
        (
            "https://cdn.example.org",
            SchemePolicy::HttpsOnly,
            "https://cdn.example.org/a/b/c.js?v=1#hash",
            true,
        ),
        (
            "https://cdn.example.org",
            SchemePolicy::HttpsOnly,
            "https://cdn.example.org:444/a/b/c.js",
            false,
        ),
        (
            "https://a.b.c.example.org",
            SchemePolicy::HttpsOnly,
            "https://a.b.c.example.org/",
            true,
        ),
        (
            "https://*.c.example.org",
            SchemePolicy::HttpsOnly,
            "https://a.b.c.example.org/",
            true,
        ),
        (
            "https://*.c.example.org",
            SchemePolicy::HttpsOnly,
            "https://c.example.org/",
            false,
        ),
    ];

    assert!(
        cases.len() >= 60,
        "expected at least 60 matcher cases, got {}",
        cases.len()
    );
    for (idx, (pattern_raw, policy, candidate_raw, expected)) in cases.iter().enumerate() {
        let pat = parse_allowlist_pattern(pattern_raw, *policy)
            .unwrap_or_else(|e| panic!("case {idx} pattern '{pattern_raw}' failed: {e}"));
        let actual = extract_origin(candidate_raw).is_ok_and(|o| pat.matches(&o));
        assert_eq!(
            actual, *expected,
            "case {idx}: pattern={pattern_raw} candidate={candidate_raw}"
        );
    }
}

#[test]
fn allowlist_validation_rules_and_limits() {
    // Defaults for inlineStyles and dataImages are false
    let minimal = Allowlist::parse_bytes(br#"{"version": 1}"#).unwrap();
    assert!(!minimal.inline_styles);
    assert!(!minimal.data_images);

    // Unknown fields rejected
    assert!(matches!(
        Allowlist::parse_bytes(br#"{"version": 1, "unknownKey": []}"#),
        Err(AllowlistError::InvalidJson(_))
    ));

    // Wrong version rejected
    assert_eq!(
        Allowlist::parse_bytes(br#"{"version": 2}"#),
        Err(AllowlistError::UnsupportedVersion(2))
    );

    // File size limit: 256 KB exact accepted, 256 KB + 1 rejected
    let max_bytes = vec![b' '; MAX_ALLOWLIST_FILE_BYTES + 1];
    assert_eq!(
        Allowlist::parse_bytes(&max_bytes),
        Err(AllowlistError::FileTooLarge)
    );

    // Entries per list boundary: 200 ok, 201 rejected
    let ok_entries: Vec<String> = (0..MAX_ENTRIES_PER_LIST)
        .map(|i| format!("\"https://h{i}.example.com\""))
        .collect();
    let ok_json = format!("{{\"version\":1,\"scripts\":[{}]}}", ok_entries.join(","));
    assert!(Allowlist::parse_bytes(ok_json.as_bytes()).is_ok());

    let too_many_entries: Vec<String> = (0..=MAX_ENTRIES_PER_LIST)
        .map(|i| format!("\"https://h{i}.example.com\""))
        .collect();
    let bad_json = format!(
        "{{\"version\":1,\"scripts\":[{}]}}",
        too_many_entries.join(",")
    );
    assert_eq!(
        Allowlist::parse_bytes(bad_json.as_bytes()),
        Err(AllowlistError::ListTooLong { list: "scripts" })
    );

    // localExceptions limit: 20 ok, 21 rejected; missing reason rejected
    let ok_local: Vec<String> = (0..MAX_LOCAL_EXCEPTIONS)
        .map(|i| {
            format!(
                "{{\"origin\":\"http://localhost:{}\",\"reason\":\"r\"}}",
                3000 + i
            )
        })
        .collect();
    let ok_local_json = format!(
        "{{\"version\":1,\"localExceptions\":[{}]}}",
        ok_local.join(",")
    );
    assert!(Allowlist::parse_bytes(ok_local_json.as_bytes()).is_ok());

    let bad_local: Vec<String> = (0..=MAX_LOCAL_EXCEPTIONS)
        .map(|i| {
            format!(
                "{{\"origin\":\"http://localhost:{}\",\"reason\":\"r\"}}",
                3000 + i
            )
        })
        .collect();
    let bad_local_json = format!(
        "{{\"version\":1,\"localExceptions\":[{}]}}",
        bad_local.join(",")
    );
    assert_eq!(
        Allowlist::parse_bytes(bad_local_json.as_bytes()),
        Err(AllowlistError::TooManyLocalExceptions)
    );

    let empty_reason =
        br#"{"version":1,"localExceptions":[{"origin":"http://localhost:3000","reason":"   "}]}"#;
    assert_eq!(
        Allowlist::parse_bytes(empty_reason),
        Err(AllowlistError::MissingLocalExceptionReason { index: 0 })
    );

    // Wildcard, path, userinfo, IP literal, duplicate, public suffix rejections
    for bad in [
        "https://*",
        "https://*.com",
        "https://*.*",
        "https://a.*.com",
        "https://*.co.uk",
        "https://*.github.io",
        "https://example.com/path",
        "https://example.com?q=1",
        "https://user@example.com",
        "https://127.0.0.1",
        "http://example.com",
        "wss://example.com",
    ] {
        assert!(
            parse_allowlist_pattern(bad, SchemePolicy::HttpsOnly).is_err(),
            "expected '{bad}' to be rejected"
        );
    }

    // Duplicate after normalization
    let dup = br#"{"version":1,"scripts":["https://Example.COM","https://example.com:443"]}"#;
    assert!(matches!(
        Allowlist::parse_bytes(dup),
        Err(AllowlistError::DuplicateEntry {
            list: "scripts",
            ..
        })
    ));

    // remoteUi.bridge subset check
    let rui = Allowlist::parse_bytes(
        br#"{"version":1,"remoteUi":{"origin":"https://app.company.com","bridge":["core.ping","fs.readGranted"]}}"#,
    )
    .unwrap();
    assert!(
        rui.validate_remote_ui_bridge(&["core.ping".into(), "fs.readGranted".into()])
            .is_ok()
    );
    assert_eq!(
        rui.validate_remote_ui_bridge(&["core.ping".into()]),
        Err(AllowlistError::BridgeCapabilityNotGranted(
            "fs.readGranted".into()
        ))
    );
}

#[test]
fn baseline_and_mode_decisions_and_limits() {
    let app = app_origin();
    let dev = validate_dev_url("http://localhost:5173").unwrap();
    let remote = extract_origin("https://app.company.com").unwrap();
    let al = Allowlist::parse_bytes(
        br#"{
            "version": 1,
            "navigate": ["https://login.company.com"],
            "openExternal": ["https://docs.company.com"],
            "remoteUi": { "origin": "https://app.company.com", "bridge": ["core.ping"] },
            "localExceptions": [{ "origin": "http://localhost:3000", "reason": "local api" }]
        }"#,
    )
    .unwrap();
    let caps = vec!["core.ping".to_string(), "notify.send".to_string()];

    for mode in [NetworkMode::Open, NetworkMode::Safe, NetworkMode::UltraSafe] {
        let ctx = PolicyContext {
            mode,
            is_dev_command: false,
            app_origin: &app,
            allowlist: Some(&al),
            remote_url_origin: Some(&remote),
            dev_url_origin: Some(&dev),
            app_capabilities: &caps,
        };

        // Baseline blocked schemes in EVERY mode
        for bad in [
            "file:///C:/a.txt",
            "javascript:alert(1)",
            "data:text/html,hi",
            "blob:https://app.company.com/1",
            "ftp://example.com/a",
            "ms-settings:display",
        ] {
            assert!(is_baseline_blocked_scheme(bad));
            assert!(matches!(
                decide_navigation(&ctx, bad),
                PolicyDecision::Deny(_)
            ));
            assert!(matches!(
                decide_open_external(&ctx, bad),
                PolicyDecision::Deny(_)
            ));
            assert!(matches!(
                decide_new_window(&ctx, bad),
                PolicyDecision::Deny(_)
            ));
            assert!(matches!(
                decide_remote_ui(&ctx, bad),
                PolicyDecision::Deny(_)
            ));
            assert!(matches!(
                decide_bridge_allowed(&ctx, bad, true),
                BridgeDecision::Deny(_)
            ));
        }

        // Bridge never answers iframes or unconfigured origins or dev.url outside dev commands
        assert!(matches!(
            decide_bridge_allowed(&ctx, "http://wavical.localhost/", false),
            BridgeDecision::Deny(_)
        ));
        assert!(matches!(
            decide_bridge_allowed(&ctx, "https://evil.com/", true),
            BridgeDecision::Deny(_)
        ));
        assert!(matches!(
            decide_bridge_allowed(&ctx, "http://localhost:5173/", true),
            BridgeDecision::Deny(_)
        ));
    }

    // dev.url honoured ONLY in dev commands
    let dev_ctx = PolicyContext {
        mode: NetworkMode::UltraSafe,
        is_dev_command: true,
        app_origin: &app,
        allowlist: Some(&al),
        remote_url_origin: None,
        dev_url_origin: Some(&dev),
        app_capabilities: &caps,
    };
    assert_eq!(
        decide_bridge_allowed(&dev_ctx, "http://localhost:5173/index.html", true),
        BridgeDecision::Allow {
            granted_capabilities: caps.clone()
        }
    );

    // openExternal length boundary (2048 ok, 2049 denied)
    let open_ctx = PolicyContext {
        mode: NetworkMode::Open,
        is_dev_command: false,
        app_origin: &app,
        allowlist: Some(&al),
        remote_url_origin: None,
        dev_url_origin: None,
        app_capabilities: &caps,
    };
    let base = "https://docs.company.com/";
    let exact_max = format!(
        "{base}{}",
        "a".repeat(MAX_OPEN_EXTERNAL_URL_LEN - base.len())
    );
    let over_max = format!("{exact_max}a");
    assert_eq!(
        decide_open_external(&open_ctx, &exact_max),
        PolicyDecision::Allow
    );
    assert!(matches!(
        decide_open_external(&open_ctx, &over_max),
        PolicyDecision::Deny(_)
    ));

    // Inline script hash limit boundary (100 ok, 101 error)
    let h = hash_inline_script(b"x");
    let ok_hashes = vec![h.clone(); MAX_INLINE_SCRIPTS_HASHED_PER_PAGE];
    let bad_hashes = vec![h; MAX_INLINE_SCRIPTS_HASHED_PER_PAGE + 1];
    assert!(generate_csp(NetworkMode::Safe, Some(&al), &ok_hashes, None).is_ok());
    assert_eq!(
        generate_csp(NetworkMode::Safe, Some(&al), &bad_hashes, None),
        Err(CspError::TooManyInlineScripts(
            MAX_INLINE_SCRIPTS_HASHED_PER_PAGE + 1
        ))
    );
}

#[test]
fn dev_url_validation_rules() {
    assert!(validate_dev_url("http://localhost:5173").is_ok());
    assert!(validate_dev_url("https://localhost:5173").is_ok());
    assert!(validate_dev_url("http://127.0.0.1:3000").is_ok());
    assert!(validate_dev_url("http://localhost").is_err());
    assert!(validate_dev_url("http://example.com:5173").is_err());
    assert!(validate_dev_url("ws://localhost:5173").is_err());
    assert!(validate_dev_url("http://localhost:5173/app").is_err());
}

#[test]
fn cdn_version_detection_table() {
    assert_eq!(
        detect_url_version_pin(
            "https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.21/lodash.min.js"
        ),
        VersionPinStatus::Pinned("4.17.21".into())
    );
    assert!(matches!(
        detect_url_version_pin(
            "https://cdnjs.cloudflare.com/ajax/libs/lodash.js/latest/lodash.min.js"
        ),
        VersionPinStatus::Unpinned(_)
    ));
    assert_eq!(
        detect_url_version_pin("https://cdn.jsdelivr.net/npm/jquery@3.7.1/dist/jquery.min.js"),
        VersionPinStatus::Pinned("3.7.1".into())
    );
    assert!(matches!(
        detect_url_version_pin("https://cdn.jsdelivr.net/npm/jquery@3/dist/jquery.min.js"),
        VersionPinStatus::Unpinned(_)
    ));
    assert_eq!(
        detect_url_version_pin("https://unpkg.com/react@18.3.1/umd/react.production.min.js"),
        VersionPinStatus::Pinned("18.3.1".into())
    );
    assert!(matches!(
        detect_url_version_pin("https://unpkg.com/react@18/umd/react.production.min.js"),
        VersionPinStatus::Unpinned(_)
    ));
    assert!(matches!(
        detect_url_version_pin(
            "https://fonts.googleapis.com/css2?family=Roboto:wght@400&display=swap"
        ),
        VersionPinStatus::Unpinned(_)
    ));
    assert_eq!(
        detect_url_version_pin("https://fonts.gstatic.com/s/roboto/v30/KFOmCnqEu92Fr1Mu4mxK.woff2"),
        VersionPinStatus::Pinned("v30".into())
    );
    assert_eq!(
        detect_url_version_pin("https://cdn.tailwindcss.com/3.4.17"),
        VersionPinStatus::Pinned("3.4.17".into())
    );
    assert!(matches!(
        detect_url_version_pin("https://cdn.tailwindcss.com"),
        VersionPinStatus::Unpinned(_)
    ));
    assert_eq!(
        detect_url_version_pin("https://example.com/static/app.js"),
        VersionPinStatus::Unknown
    );
}

#[test]
fn vendor_lock_parser_and_limits() {
    let valid = format!(
        r#"{{
            "version": 1,
            "entries": [{{
                "url": "https://unpkg.com/react@18.3.1/umd/react.production.min.js",
                "localPath": "vendor/unpkg.com/react.js",
                "sha256": "{}",
                "size": 1024,
                "fetchedAt": "2026-11-21T00:00:00Z",
                "license": "MIT"
            }}]
        }}"#,
        "a".repeat(64)
    );
    assert!(VendorLock::parse_bytes(valid.as_bytes()).is_ok());

    for bad_path in [
        "../outside.js",
        "src/vendor/a.js",
        "vendor\\a.js",
        "/vendor/a.js",
        "vendor/C:/a.js",
        "vendor/../a.js",
        "vendor/a.js.",
    ] {
        let j = format!(
            r#"{{"version":1,"entries":[{{"url":"https://a.com/x.js","localPath":"{bad_path}","sha256":"{}","size":10,"fetchedAt":"2026-11-21T00:00:00Z"}}]}}"#,
            "a".repeat(64)
        );
        assert!(
            matches!(
                VendorLock::parse_bytes(j.as_bytes()),
                Err(VendorLockError::UnsafeLocalPath { .. } | VendorLockError::InvalidJson(_))
            ),
            "expected unsafe path '{bad_path}' to fail"
        );
    }

    // Size limit boundary
    let too_big = format!(
        r#"{{"version":1,"entries":[{{"url":"https://a.com/x.js","localPath":"vendor/a.js","sha256":"{}","size":{},"fetchedAt":"2026-11-21T00:00:00Z"}}]}}"#,
        "a".repeat(64),
        MAX_VENDOR_FILE_BYTES + 1
    );
    assert!(matches!(
        VendorLock::parse_bytes(too_big.as_bytes()),
        Err(VendorLockError::FileTooLarge { .. })
    ));

    // Scanner model smoke construction
    let scan = HtmlScanResult {
        file: "src/index.html".into(),
        inline_scripts: vec![InlineScriptBlock {
            index: 0,
            line: 10,
            byte_len: 9,
            hash: hash_inline_script(b"alert(1);"),
            content: "alert(1);".into(),
        }],
        inline_handlers: vec![InlineHandlerFinding {
            file: "src/index.html".into(),
            line: 12,
            column: 5,
            tag: "img".into(),
            attribute: "onerror".into(),
        }],
        style_attributes: vec![StyleAttrFinding {
            file: "src/index.html".into(),
            line: 14,
            tag: "div".into(),
        }],
        external_refs: vec![ExternalRef {
            category: ExternalRefCategory::Script,
            url: "https://cdn.tailwindcss.com/3.4.17".into(),
            line: 5,
            has_integrity: false,
        }],
    };
    assert_eq!(scan.inline_scripts.len(), 1);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn prop_parsers_never_panic(raw in ".*") {
        let _ = Allowlist::parse_bytes(raw.as_bytes());
        let _ = extract_origin(&raw);
        let _ = parse_allowlist_pattern(&raw, SchemePolicy::HttpsOnly);
        let _ = detect_url_version_pin(&raw);
        let _ = VendorLock::parse_bytes(raw.as_bytes());
    }

    #[test]
    fn prop_wildcard_never_matches_bare_parent_or_other_domain(
        sub in "[a-z0-9]{1,8}",
        other in "[a-z]{3,10}",
    ) {
        let pat = parse_allowlist_pattern("https://*.company.com", SchemePolicy::HttpsOnly).unwrap();
        let bare = extract_origin("https://company.com/").unwrap();
        prop_assert!(!pat.matches(&bare));

        let evil_suffix = format!("https://{other}company.com/");
        if let Ok(o) = extract_origin(&evil_suffix) {
            prop_assert!(!pat.matches(&o));
        }
        let evil_prefix = format!("https://company.com.{other}.com/");
        if let Ok(o) = extract_origin(&evil_prefix) {
            prop_assert!(!pat.matches(&o));
        }
        let valid_sub = format!("https://{sub}.company.com/");
        let o = extract_origin(&valid_sub).unwrap();
        prop_assert!(pat.matches(&o));
    }

    #[test]
    fn prop_ultra_safe_never_allows_external_or_inline(
        host in "[a-z0-9]{1,10}\\.[a-z]{2,5}",
        script in "[a-zA-Z0-9 ();]{1,30}",
    ) {
        let app = app_origin();
        let ctx = PolicyContext {
            mode: NetworkMode::UltraSafe,
            is_dev_command: false,
            app_origin: &app,
            allowlist: None,
            remote_url_origin: None,
            dev_url_origin: None,
            app_capabilities: &[],
        };
        let url = format!("https://{host}/");
        prop_assert!(matches!(decide_navigation(&ctx, &url), PolicyDecision::Deny(_)));
        prop_assert!(matches!(decide_open_external(&ctx, &url), PolicyDecision::Deny(_)));
        prop_assert!(matches!(decide_remote_ui(&ctx, &url), PolicyDecision::Deny(_)));

        let h = hash_inline_script(script.as_bytes());
        let csp = generate_csp(NetworkMode::UltraSafe, None, std::slice::from_ref(&h), None).unwrap();
        prop_assert_eq!(&csp, ULTRA_SAFE_CSP);
        prop_assert!(!csp.contains(&h.as_csp_token()));
        prop_assert!(!csp.contains(&host));
    }

    #[test]
    fn prop_bridge_denied_for_iframes_and_unconfigured_and_dev_outside_dev(
        host in "[a-z0-9]{1,10}\\.[a-z]{2,5}",
        port in 1024u16..65000u16,
    ) {
        let app = app_origin();
        let dev = validate_dev_url(&format!("http://localhost:{port}")).unwrap();
        let caps = vec!["core.ping".to_string()];

        for mode in [NetworkMode::Open, NetworkMode::Safe, NetworkMode::UltraSafe] {
            let ctx = PolicyContext {
                mode,
                is_dev_command: false,
                app_origin: &app,
                allowlist: None,
                remote_url_origin: None,
                dev_url_origin: Some(&dev),
                app_capabilities: &caps,
            };
            // Iframe on app origin is always denied
            prop_assert!(matches!(
                decide_bridge_allowed(&ctx, "http://wavical.localhost/", false),
                BridgeDecision::Deny(_)
            ));
            // Unconfigured origin is always denied
            let other = format!("https://{host}/");
            prop_assert!(matches!(
                decide_bridge_allowed(&ctx, &other, true),
                BridgeDecision::Deny(_)
            ));
            // dev.url is never honoured outside dev commands
            let dev_url = format!("http://localhost:{port}/");
            prop_assert!(matches!(
                decide_bridge_allowed(&ctx, &dev_url, true),
                BridgeDecision::Deny(_)
            ));
        }
    }
}

#[test]
fn f5_open_external_schemes_allowlist_and_top_level_navigation_decisions() {
    let valid_both = Allowlist::parse_bytes(
        br#"{"version":1,"openExternalSchemes":["mailto","tel"],"navigate":["https://login.company.com"],"openExternal":["https://docs.company.com"]}"#,
    )
    .unwrap();
    assert!(valid_both.allows_external_scheme("mailto"));
    assert!(valid_both.allows_external_scheme("tel"));

    for bad in [
        br#"{"version":1,"openExternalSchemes":["sms"]}"#.as_slice(),
        br#"{"version":1,"openExternalSchemes":["whatsapp"]}"#,
        br#"{"version":1,"openExternalSchemes":["skype"]}"#,
        br#"{"version":1,"openExternalSchemes":["https"]}"#,
        br#"{"version":1,"openExternalSchemes":["file"]}"#,
        br#"{"version":1,"openExternalSchemes":["ms-settings"]}"#,
        br#"{"version":1,"openExternalSchemes":["mailto:"]}"#,
        br#"{"version":1,"openExternalSchemes":["mailto","mailto"]}"#,
        br#"{"version":1,"openExternalSchemes":["mailto","MAILTO"]}"#,
    ] {
        assert!(
            Allowlist::parse_bytes(bad).is_err(),
            "expected invalid openExternalSchemes to be rejected: {}",
            String::from_utf8_lossy(bad)
        );
    }

    let app = app_origin();
    let open_ctx = PolicyContext {
        mode: NetworkMode::Open,
        is_dev_command: false,
        app_origin: &app,
        allowlist: None,
        remote_url_origin: None,
        dev_url_origin: None,
        app_capabilities: &[],
    };
    assert_eq!(
        decide_open_external(&open_ctx, "mailto:hello@example.com"),
        PolicyDecision::Allow
    );
    assert_eq!(
        decide_open_external(&open_ctx, "tel:+15550100"),
        PolicyDecision::Allow
    );
    assert!(matches!(
        decide_open_external(&open_ctx, "mailto:a@b.com?subject=x%0d%0abcc:evil@x.com"),
        PolicyDecision::Deny(_)
    ));
    assert!(matches!(
        decide_open_external(&open_ctx, "sms:+15550100"),
        PolicyDecision::Deny(_)
    ));

    // User-initiated external click in open mode (browser default vs in-app)
    assert_eq!(
        decide_top_level_navigation(
            &open_ctx,
            "https://example.com",
            true,
            ExternalLinksMode::Browser
        ),
        NavigationDecision::OpenExternal
    );
    assert_eq!(
        decide_top_level_navigation(
            &open_ctx,
            "https://example.com",
            false,
            ExternalLinksMode::Browser
        ),
        NavigationDecision::AllowInApp
    );
    assert_eq!(
        decide_top_level_navigation(
            &open_ctx,
            "https://example.com",
            true,
            ExternalLinksMode::InApp
        ),
        NavigationDecision::AllowInApp
    );
    assert_eq!(
        decide_top_level_navigation(
            &open_ctx,
            "mailto:hello@example.com",
            true,
            ExternalLinksMode::InApp
        ),
        NavigationDecision::OpenExternal
    );

    // Safe mode: navigate list stays in-app, openExternal opens externally on click, mailto/tel obey openExternalSchemes
    let safe_ctx = PolicyContext {
        mode: NetworkMode::Safe,
        is_dev_command: false,
        app_origin: &app,
        allowlist: Some(&valid_both),
        remote_url_origin: None,
        dev_url_origin: None,
        app_capabilities: &[],
    };
    assert_eq!(
        decide_top_level_navigation(
            &safe_ctx,
            "https://login.company.com/auth",
            true,
            ExternalLinksMode::Browser
        ),
        NavigationDecision::AllowInApp
    );
    assert_eq!(
        decide_top_level_navigation(
            &safe_ctx,
            "https://docs.company.com/",
            true,
            ExternalLinksMode::Browser
        ),
        NavigationDecision::OpenExternal
    );
    assert_eq!(
        decide_top_level_navigation(
            &safe_ctx,
            "mailto:hello@example.com",
            true,
            ExternalLinksMode::Browser
        ),
        NavigationDecision::OpenExternal
    );
    assert!(matches!(
        decide_top_level_navigation(
            &safe_ctx,
            "https://evil.com/",
            true,
            ExternalLinksMode::Browser
        ),
        NavigationDecision::Deny(_)
    ));
}
