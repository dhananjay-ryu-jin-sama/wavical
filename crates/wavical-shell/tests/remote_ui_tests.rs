use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use tempfile::TempDir;
use wavical_netpolicy::NetworkMode;
use wavical_shell::net_policy::{
    authorize_bridge_request, build_cert_error_html, build_init_script, build_offline_error_html,
    resolve_shell_network_policy_with_remote,
};
use wavical_shell::{WindowConfig, process_ipc_message};
use wry::http::Uri;

#[test]
fn remote_ui_open_and_safe_bridge_origin_and_iframe_isolation() {
    let l1 = TcpListener::bind("127.0.0.1:0").unwrap();
    let p1 = l1.local_addr().unwrap().port();
    let l2 = TcpListener::bind("127.0.0.1:0").unwrap();
    let p2 = l2.local_addr().unwrap().port();

    // Spawn minimal local HTTP servers for the two distinct origins
    for (listener, body) in [(l1, "<h1>Remote UI</h1>"), (l2, "<h1>Other Origin</h1>")] {
        thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
    }

    let https_origin1 = "https://app.company.com";
    let https_origin2 = "https://other.company.com";
    let local_origin1 = format!("http://127.0.0.1:{p1}");
    let local_origin2 = format!("http://127.0.0.1:{p2}");

    let tmp = TempDir::new().unwrap();
    let al_json = format!(
        r#"{{
            "version": 1,
            "remoteUi": {{ "origin": "{https_origin1}", "bridge": ["core.ping"] }},
            "localExceptions": [
                {{ "origin": "{local_origin1}", "reason": "local remoteUi test" }},
                {{ "origin": "{local_origin2}", "reason": "second origin test" }}
            ]
        }}"#
    );
    fs::write(tmp.path().join("wavical.allowlist.json"), al_json).unwrap();

    // 1. Safe mode: configured origin gets bridge limited to remoteUi.bridge (core.ping only, NOT notify.send)
    let safe_policy = resolve_shell_network_policy_with_remote(
        tmp.path(),
        Some(tmp.path()),
        true,
        Some("safe"),
        None,
        Some(&format!("{https_origin1}/app")),
        vec!["core.ping".into(), "notify.send".into()],
    );
    assert_eq!(safe_policy.mode, NetworkMode::Safe);
    assert_eq!(
        safe_policy.remote_url.as_deref(),
        Some(format!("{https_origin1}/app").as_str())
    );

    let init_js = build_init_script(&safe_policy);
    assert!(init_js.contains(https_origin1));
    assert!(!init_js.contains(https_origin2));
    assert!(init_js.contains("window.top !== window.self"));

    let mut caps = wavical_ipc::Capabilities::new();
    caps.grant("core.ping");
    caps.grant("notify.send");
    let cfg = WindowConfig {
        capabilities: caps,
        network_policy: safe_policy.clone(),
        ..WindowConfig::default()
    };
    let registry = wavical_ipc::HandlerRegistry::with_core_handlers();

    // Configured origin top frame: core.ping succeeds, notify.send is denied by remoteUi.bridge subset
    let uri1: Uri = format!("{https_origin1}/app").parse().unwrap();
    let ping_req = r#"{"id":1,"method":"core.ping","params":{}}"#;
    let res_ping = process_ipc_message(&uri1, ping_req, &cfg, &registry)
        .unwrap()
        .unwrap();
    assert!(res_ping.ok);

    let notify_req = r#"{"id":2,"method":"notify.send","params":{"title":"x","body":"y"}}"#;
    let res_notify = process_ipc_message(&uri1, notify_req, &cfg, &registry)
        .unwrap()
        .unwrap();
    assert!(!res_notify.ok);

    // Navigating to another origin removes the bridge; returning restores it
    let uri2: Uri = format!("{https_origin2}/other").parse().unwrap();
    assert!(process_ipc_message(&uri2, ping_req, &cfg, &registry).is_err());
    let res_ping_return = process_ipc_message(&uri1, ping_req, &cfg, &registry)
        .unwrap()
        .unwrap();
    assert!(res_ping_return.ok);

    // Iframe of the configured origin inside another page does NOT get the bridge
    assert!(authorize_bridge_request(&safe_policy, &uri1, false, "win1").is_err());

    // 2. Open mode with local server: configured origin gets full app capabilities
    let open_policy = resolve_shell_network_policy_with_remote(
        tmp.path(),
        Some(tmp.path()),
        true,
        Some("open"),
        None,
        Some(&format!("{local_origin1}/app")),
        vec!["core.ping".into(), "notify.send".into()],
    );
    let local_uri1: Uri = format!("{local_origin1}/app").parse().unwrap();
    let local_uri2: Uri = format!("{local_origin2}/other").parse().unwrap();
    assert_eq!(
        authorize_bridge_request(&open_policy, &local_uri1, true, "win1").unwrap(),
        vec!["core.ping".to_string(), "notify.send".to_string()]
    );
    assert!(authorize_bridge_request(&open_policy, &local_uri2, true, "win1").is_err());

    // 3. Ultra-safe mode: window.remoteUrl is refused
    let ultra_policy = resolve_shell_network_policy_with_remote(
        tmp.path(),
        Some(tmp.path()),
        true,
        Some("ultra-safe"),
        None,
        Some(&format!("{https_origin1}/app")),
        vec!["core.ping".into()],
    );
    assert!(ultra_policy.remote_url.is_none());
    assert!(ultra_policy.remote_url_origin.is_none());
}

#[test]
fn friendly_offline_and_cert_error_pages_fail_closed() {
    let offline_html = build_offline_error_html("https://app.company.com/?q=<script>");
    assert!(offline_html.contains("Retry"));
    assert!(offline_html.contains("offline"));
    assert!(!offline_html.contains("<script>"));
    assert!(offline_html.contains("&lt;script&gt;"));

    let cert_html = build_cert_error_html();
    assert!(cert_html.contains("certificate"));
    assert!(cert_html.contains("IT"));
    assert!(!cert_html.to_ascii_lowercase().contains("proceed anyway"));
    assert!(!cert_html.to_ascii_lowercase().contains("ignore"));
}
