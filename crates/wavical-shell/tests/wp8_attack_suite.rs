use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tempfile::TempDir;
use wavical_netpolicy::NetworkMode;
use wavical_netpolicy::allowlist::Allowlist;
use wavical_netpolicy::csp::{ULTRA_SAFE_CSP, generate_csp};
use wavical_netpolicy::decisions::{
    PolicyContext, PolicyDecision, decide_bridge_allowed, decide_navigation, decide_remote_ui,
};
use wavical_netpolicy::html::hash_inline_script;
use wavical_netpolicy::origin::{OriginHost, ParsedOrigin, extract_origin};
use wavical_shell::net_policy::{
    OpenExternalError, OpenExternalRateLimiter, authorize_bridge_request,
    handle_navigation_with_policy, open_external_validated,
    resolve_shell_network_policy_with_remote,
};
use wavical_shell::{DEFAULT_CSP, WAVICAL_INIT_SCRIPT, WindowConfig, process_ipc_message};
use wry::http::Uri;

const EXFIL_MARKER: &str = "WAVICAL_EXFIL_MARKER_98765";

fn spawn_recording_server() -> (u16, Arc<Mutex<Vec<String>>>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(Mutex::new(Vec::new()));
    let hits_c = hits.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_c = stop.clone();

    thread::spawn(move || {
        while !stop_c.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Ok(mut g) = hits_c.lock() {
                    g.push(req);
                }
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            }
            thread::sleep(Duration::from_millis(10));
        }
    });

    (port, hits, stop)
}

#[test]
fn wp8_attack_suite_all_modes_and_phase0_bypasses() {
    let (_exfil_port, exfil_hits, stop) = spawn_recording_server();

    let tmp = TempDir::new().unwrap();
    let al_bytes = br#"{
        "version": 1,
        "scripts": ["https://cdn.company.com"],
        "styles": ["https://styles.company.com"],
        "fonts": ["https://fonts.company.com"],
        "images": ["https://*.company.com"],
        "media": ["https://media.company.com"],
        "connect": ["https://api.company.com", "wss://live.company.com"],
        "frames": ["https://frame.company.com"],
        "navigate": ["https://login.company.com"],
        "openExternal": ["https://docs.company.com"],
        "remoteUi": { "origin": "https://app.company.com", "bridge": ["core.ping"] },
        "localExceptions": [{ "origin": "http://localhost:3000", "reason": "local api" }]
    }"#;
    let al = Allowlist::parse_bytes(al_bytes).unwrap();

    // 1. Safe mode: modified inline script hash does NOT match CSP; runtime script blocked (no 'unsafe-inline')
    let good_script = b"window.ok = 1;";
    let modified_script = b"window.ok = 2;";
    let good_hash = hash_inline_script(good_script);
    let mod_hash = hash_inline_script(modified_script);
    let safe_csp = generate_csp(
        NetworkMode::Safe,
        Some(&al),
        std::slice::from_ref(&good_hash),
        None,
    )
    .unwrap();
    assert!(safe_csp.contains(&good_hash.as_csp_token()));
    assert!(!safe_csp.contains(&mod_hash.as_csp_token()));
    assert!(!safe_csp.contains("'unsafe-inline'"));
    assert!(safe_csp.contains("https://*.company.com"));
    assert!(safe_csp.contains("object-src 'none'"));
    assert!(safe_csp.contains("base-uri 'self'"));

    // 2. Phase 0 A1b step 6 bypass vectors + wildcard look-alikes in all modes
    let app = ParsedOrigin {
        scheme: "http".into(),
        host: OriginHost::Domain("wavical.localhost".into()),
        port: 80,
    };
    let dev = extract_origin("http://localhost:5173").unwrap();
    let remote = extract_origin("https://app.company.com").unwrap();
    let caps = vec!["core.ping".to_string(), "notify.send".to_string()];

    for mode in [NetworkMode::Open, NetworkMode::Safe, NetworkMode::UltraSafe] {
        let policy = resolve_shell_network_policy_with_remote(
            tmp.path(),
            None,
            false,
            Some("open"),
            Some("http://localhost:5173"),
            Some("https://app.company.com"),
            caps.clone(),
        );
        // Built app without manifest always falls back to ultra-safe and ignores dev.url
        assert_eq!(policy.mode, NetworkMode::UltraSafe);
        assert!(policy.dev_url_origin.is_none());

        let ctx = PolicyContext {
            mode,
            is_dev_command: false,
            app_origin: &app,
            allowlist: Some(&al),
            remote_url_origin: Some(&remote),
            dev_url_origin: Some(&dev),
            app_capabilities: &caps,
        };

        // Phase 0 bypasses: userinfo, evil suffixes, look-alikes, iframes, popups, workers, dev.url in build
        for evil_origin in [
            "https://wavical.localhost@evil.com/",
            "https://app.company.com@evil.com/",
            "https://company.com.evil.com/",
            "https://evilcompany.com/",
            "https://xn--pple-43d.com/",
            "http://localhost:5173/",      // dev.url in non-dev build
            "http://unlisted.local:8080/", // non-excepted http
        ] {
            assert!(
                decide_bridge_allowed(&ctx, evil_origin, true).is_deny(),
                "bridge must deny {evil_origin} in {mode:?}"
            );
        }
        // Iframe of app or remote origin never gets the bridge
        assert!(decide_bridge_allowed(&ctx, "http://wavical.localhost/", false).is_deny());
        assert!(decide_bridge_allowed(&ctx, "https://app.company.com/", false).is_deny());

        // openExternal: file:, ms-calc:, javascript:, data:, 10,000-char URL, flood
        let rl = OpenExternalRateLimiter::default();
        let long_url = format!("https://docs.company.com/{}", "a".repeat(10_000));
        for bad_ext in [
            "file:///C:/Windows/win.ini",
            "ms-calc:",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "http://docs.company.com/",
            long_url.as_str(),
        ] {
            let mut shell_p = wavical_shell::net_policy::ShellNetworkPolicy {
                mode,
                allowlist: Some(al.clone()),
                ..Default::default()
            };
            assert!(
                matches!(
                    open_external_validated(&shell_p, bad_ext, "atk", &rl, |_| Ok(())),
                    Err(OpenExternalError::PolicyDenied(_))
                ),
                "openExternal must deny {bad_ext} in {mode:?}"
            );
            shell_p.mode = mode;
        }
        // Flood (>5 in 10s) blocked in open mode
        let open_p = wavical_shell::net_policy::ShellNetworkPolicy {
            mode: NetworkMode::Open,
            ..Default::default()
        };
        let flood_rl = OpenExternalRateLimiter::default();
        for _ in 0..5 {
            assert!(
                open_external_validated(
                    &open_p,
                    "https://docs.company.com/",
                    "atk",
                    &flood_rl,
                    |_| Ok(())
                )
                .is_ok()
            );
        }
        assert_eq!(
            open_external_validated(
                &open_p,
                "https://docs.company.com/",
                "atk",
                &flood_rl,
                |_| Ok(())
            ),
            Err(OpenExternalError::RateLimited)
        );
    }

    // 3. Remote UI navigating to evil origin loses bridge
    let safe_ctx = PolicyContext {
        mode: NetworkMode::Safe,
        is_dev_command: false,
        app_origin: &app,
        allowlist: Some(&al),
        remote_url_origin: Some(&remote),
        dev_url_origin: None,
        app_capabilities: &caps,
    };
    assert_eq!(
        decide_remote_ui(&safe_ctx, "https://app.company.com"),
        PolicyDecision::Allow
    );
    assert!(matches!(
        decide_navigation(&safe_ctx, "https://evil.com/steal"),
        PolicyDecision::Deny(_)
    ));
    assert!(decide_bridge_allowed(&safe_ctx, "https://evil.com/steal", true).is_deny());

    // 4. Ultra-safe exfiltration route checks: CSP + navigation + openExternal + bridge all block
    let ultra_p = wavical_shell::net_policy::ShellNetworkPolicy::default();
    assert_eq!(ultra_p.csp_for_path("index.html"), DEFAULT_CSP);
    assert_eq!(DEFAULT_CSP, ULTRA_SAFE_CSP);
    assert!(DEFAULT_CSP.contains("connect-src 'self'"));
    assert!(DEFAULT_CSP.contains("img-src 'self' data:"));
    assert!(DEFAULT_CSP.contains("style-src 'self'"));
    assert!(DEFAULT_CSP.contains("script-src 'self'"));
    assert!(DEFAULT_CSP.contains("form-action 'self'"));
    assert!(DEFAULT_CSP.contains("frame-src 'none'"));

    let exfil_target = format!("http://127.0.0.1:{_exfil_port}/?m={EXFIL_MARKER}");
    assert!(!handle_navigation_with_policy(
        &ultra_p,
        &exfil_target,
        "atk"
    ));
    let rl_u = OpenExternalRateLimiter::default();
    assert!(open_external_validated(&ultra_p, &exfil_target, "atk", &rl_u, |_| Ok(())).is_err());
    let exfil_uri: Uri = exfil_target.parse().unwrap();
    assert!(authorize_bridge_request(&ultra_p, &exfil_uri, true, "atk").is_err());

    // Unconfigured bridge IPC from attacker origin rejected
    let cfg = WindowConfig::default();
    let reg = wavical_ipc::HandlerRegistry::with_core_handlers();
    assert!(
        process_ipc_message(
            &exfil_uri,
            r#"{"id":1,"method":"core.ping","params":{}}"#,
            &cfg,
            &reg
        )
        .is_err()
    );
    assert!(WAVICAL_INIT_SCRIPT.contains("window.top !== window.self"));

    // Local server received ZERO marker hits
    stop.store(true, Ordering::Relaxed);
    assert!(
        exfil_hits.lock().unwrap().is_empty(),
        "exfil server must receive 0 hits"
    );
}

trait BridgeDecisionExt {
    fn is_deny(&self) -> bool;
}
impl BridgeDecisionExt for wavical_netpolicy::decisions::BridgeDecision {
    fn is_deny(&self) -> bool {
        matches!(self, Self::Deny(_))
    }
}
