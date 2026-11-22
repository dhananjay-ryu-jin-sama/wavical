#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use tempfile::tempdir;
use wavical_netpolicy::NetworkMode;
use wavical_netpolicy::csp::ULTRA_SAFE_CSP;
use wavical_netpolicy::html::hash_inline_script;
use wavical_shell::HEADER_CSP;
use wavical_shell::net_policy::{
    handle_protocol_request_with_policy, resolve_shell_network_policy_with_remote,
    verify_safe_mode_startup_integrity,
};
use wry::http::{Request, StatusCode};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn ensure_wavical_cli_bin() -> &'static PathBuf {
    static CLI_BIN: OnceLock<PathBuf> = OnceLock::new();
    CLI_BIN.get_or_init(|| {
        let root = workspace_root();
        let bin = root.join("target").join("debug").join("wavical.exe");
        if !bin.is_file() {
            let out = Command::new("cargo")
                .args(["build", "-p", "wavical-cli", "--bin", "wavical"])
                .current_dir(&root)
                .output()
                .expect("build wavical CLI");
            assert!(
                out.status.success(),
                "failed to build wavical CLI: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        bin
    })
}

fn make_fixture(root: &Path, name: &str) -> PathBuf {
    let proj = root.join(name);
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(
        proj.join("wavical.config.js"),
        format!("export default {{ appId: '{name}', version: '1.0.0', capabilities: {{}} }};\n"),
    )
    .unwrap();
    fs::write(
        proj.join("src").join("index.html"),
        r#"<!doctype html><html><head><script src="https://cdn.tailwindcss.com"></script></head><body><script>window.__k=42;</script></body></html>"#,
    )
    .unwrap();
    fs::write(
        proj.join("wavical.allowlist.json"),
        r#"{"version":1,"scripts":["https://cdn.tailwindcss.com"]}"#,
    )
    .unwrap();
    proj
}

#[test]
fn shell_resolver_and_protocol_handler_work_with_real_cli_manifests() {
    let temp = tempdir().unwrap();
    let wavical = ensure_wavical_cli_bin();

    // 1. Open build
    let open_proj = make_fixture(temp.path(), "shell-open-app");
    let out_open = Command::new(wavical)
        .args(["build", "--allow-open"])
        .arg(&open_proj)
        .output()
        .unwrap();
    assert!(out_open.status.success());
    let open_dist = open_proj.join("dist");
    let p_open =
        resolve_shell_network_policy_with_remote(&open_dist, None, false, None, None, None, vec![]);
    assert_eq!(p_open.mode, NetworkMode::Open);
    assert!(verify_safe_mode_startup_integrity(&open_dist, None, false).is_ok());
    let resp_open = handle_protocol_request_with_policy(
        Request::builder()
            .uri("wavical://localhost/index.html")
            .body(vec![])
            .unwrap(),
        &open_dist,
        &p_open,
        "w1",
    );
    assert_eq!(resp_open.status(), StatusCode::OK);

    // 2. Safe build -> resolves Safe with inline script hash CSP, and tampering fails startup check
    let safe_proj = make_fixture(temp.path(), "shell-safe-app");
    let out_safe = Command::new(wavical)
        .arg("build-safe")
        .arg(&safe_proj)
        .output()
        .unwrap();
    assert!(out_safe.status.success());
    let safe_dist = safe_proj.join("dist");
    let p_safe =
        resolve_shell_network_policy_with_remote(&safe_dist, None, false, None, None, None, vec![]);
    assert_eq!(p_safe.mode, NetworkMode::Safe);
    assert!(verify_safe_mode_startup_integrity(&safe_dist, None, false).is_ok());

    let resp_safe = handle_protocol_request_with_policy(
        Request::builder()
            .uri("wavical://localhost/index.html")
            .body(vec![])
            .unwrap(),
        &safe_dist,
        &p_safe,
        "w1",
    );
    assert_eq!(resp_safe.status(), StatusCode::OK);
    let csp = resp_safe
        .headers()
        .get(HEADER_CSP)
        .unwrap()
        .to_str()
        .unwrap();
    let expected_hash = hash_inline_script(b"window.__k=42;").as_csp_token();
    assert!(csp.contains(&expected_hash));
    assert!(csp.contains("https://cdn.tailwindcss.com"));

    // Modify allowlist after build -> startup integrity refuses, while resolver keeps UltraSafe fallback
    fs::write(
        safe_dist.join("wavical.allowlist.json"),
        br#"{"version":1,"scripts":["https://evil.com"]}"#,
    )
    .unwrap();
    assert!(verify_safe_mode_startup_integrity(&safe_dist, None, false).is_err());
    let p_tampered =
        resolve_shell_network_policy_with_remote(&safe_dist, None, false, None, None, None, vec![]);
    assert_eq!(p_tampered.mode, NetworkMode::UltraSafe);

    // 3. Ultra-safe build -> resolves UltraSafe and serves rewritten HTML + extracted JS with 200 OK
    let ultra_proj = make_fixture(temp.path(), "shell-ultra-app");
    let out_ultra = Command::new(wavical)
        .arg("build-ultra-safe")
        .arg(&ultra_proj)
        .output()
        .unwrap();
    assert!(out_ultra.status.success());
    let ultra_dist = ultra_proj.join("dist");
    let p_ultra = resolve_shell_network_policy_with_remote(
        &ultra_dist,
        None,
        false,
        None,
        None,
        None,
        vec![],
    );
    assert_eq!(p_ultra.mode, NetworkMode::UltraSafe);
    assert!(verify_safe_mode_startup_integrity(&ultra_dist, None, false).is_ok());
    let resp_ultra = handle_protocol_request_with_policy(
        Request::builder()
            .uri("wavical://localhost/index.html")
            .body(vec![])
            .unwrap(),
        &ultra_dist,
        &p_ultra,
        "w1",
    );
    assert_eq!(resp_ultra.status(), StatusCode::OK);
    assert_eq!(
        resp_ultra.headers().get(HEADER_CSP).unwrap(),
        ULTRA_SAFE_CSP
    );
    let resp_ext = handle_protocol_request_with_policy(
        Request::builder()
            .uri("wavical://localhost/.wavical-extracted/index_html_inline_0.js")
            .body(vec![])
            .unwrap(),
        &ultra_dist,
        &p_ultra,
        "w1",
    );
    assert_eq!(resp_ext.status(), StatusCode::OK);
}
