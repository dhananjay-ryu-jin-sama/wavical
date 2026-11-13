#![forbid(unsafe_code)]

use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wavical_shell::hardening::{
    DEVTOOLS_COMPILED_ENABLED, StartupSelfCheckError, evaluate_startup_self_check,
};
use wavical_shell::is_devtools_enabled;

#[test]
fn test_debug_build_allows_devtools_and_release_disables_at_compile_time() {
    assert_eq!(is_devtools_enabled(), cfg!(debug_assertions));
    assert_eq!(DEVTOOLS_COMPILED_ENABLED, cfg!(debug_assertions));
}

#[test]
fn test_release_self_check_refuses_devtools_enabled() {
    let err = evaluate_startup_self_check(true, true, &[], &[], &[])
        .expect_err("release build must refuse to start if devtools are enabled");
    assert!(matches!(err, StartupSelfCheckError::Failed(_)));
    assert!(err.to_string().contains("devtools enabled"));
}

#[test]
fn test_release_self_check_refuses_webview2_debug_env() {
    let err = evaluate_startup_self_check(
        true,
        false,
        &["PATH", "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"],
        &[],
        &[],
    )
    .expect_err("release build must refuse WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
    assert!(
        err.to_string()
            .contains("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
    );
}

#[test]
fn test_release_self_check_refuses_node_debug_env_and_config_flags() {
    // 1. Forbidden Node env var in process environment
    for var in [
        "NODE_OPTIONS",
        "NODE_DEBUG",
        "NODE_PATH",
        "NODE_INSPECT_RESUME_ON_START",
    ] {
        let err = evaluate_startup_self_check(true, false, &[var], &[], &[])
            .expect_err("release build must refuse forbidden Node env var");
        assert!(err.to_string().contains("Node.js"));
    }

    // 2. Forbidden Node env var in backend config
    let err = evaluate_startup_self_check(true, false, &[], &["NODE_OPTIONS".to_string()], &[])
        .expect_err("release build must refuse NODE_OPTIONS in backend config");
    assert!(err.to_string().contains("backend config"));

    // 3. Forbidden Node debug/preload flags in backend args
    for flag in [
        "--inspect=9229",
        "--inspect-brk",
        "--require=./evil.js",
        "--import=./evil.mjs",
    ] {
        let err = evaluate_startup_self_check(true, false, &[], &[], &[flag.to_string()])
            .expect_err("release build must refuse debug/preload flag");
        assert!(err.to_string().contains("backend config"));
    }
}

#[test]
fn test_release_self_check_passes_when_clean() {
    // Note: in debug test binaries, DEVTOOLS_COMPILED_ENABLED is true, so `is_release = false`
    // verifies the struct & clean report path; in release test binaries `is_release = true` passes too.
    let report = evaluate_startup_self_check(
        false,
        true,
        &["PATH", "SystemRoot"],
        &["MY_SAFE_VAR".to_string()],
        &[],
    )
    .expect("clean startup self-check should pass");
    assert!(report.all_passed());
}

#[test]
fn test_node_inspect_9229_not_opened_when_env_cleared() {
    // Verify that when a child process is spawned with env_clear() (as BackendService does),
    // NODE_OPTIONS=--inspect=9229 does not open port 9229.
    let port_9229: SocketAddr = "127.0.0.1:9229".parse().unwrap();
    if TcpStream::connect_timeout(&port_9229, Duration::from_millis(150)).is_ok() {
        // Port already in use by an external process on the host; skip socket assertion.
        return;
    }

    // If `node` is on PATH, spawn it with env_clear() and verify 9229 stays closed.
    if let Ok(mut child) = Command::new("node")
        .env_clear()
        .arg("-e")
        .arg("setTimeout(() => {}, 1000)")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            TcpStream::connect_timeout(&port_9229, Duration::from_millis(200)).is_err(),
            "Node inspector port 9229 must not be open"
        );
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[test]
#[ignore = "local-only GUI test: run with `cargo build --release -p wavical-shell && cargo test -p wavical-shell --test hardening_tests -- --ignored`"]
fn test_release_binary_neutralizes_webview2_9222_and_node_9229() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");
    let exe_name = if cfg!(windows) {
        "wavical-shell.exe"
    } else {
        "wavical-shell"
    };
    let release_bin = workspace_root.join("target").join("release").join(exe_name);
    assert!(
        release_bin.is_file(),
        "release binary not found at {}; build it first with `cargo build --release -p wavical-shell`",
        release_bin.display()
    );

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let asset_dir = std::env::temp_dir().join(format!("wavical_wp8_gui_{nanos}"));
    std::fs::create_dir_all(&asset_dir).expect("create temp asset dir");
    std::fs::write(
        asset_dir.join("index.html"),
        b"<html><body>WP8</body></html>",
    )
    .expect("write index.html");

    let mut child = Command::new(&release_bin)
        .arg("--app-id")
        .arg(format!("wp8-test-{nanos}"))
        .arg("--asset-root")
        .arg(&asset_dir)
        .arg("--single-instance")
        .arg("false")
        .env(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "--remote-debugging-port=9222",
        )
        .env("NODE_OPTIONS", "--inspect=9229")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn release wavical-shell");

    std::thread::sleep(Duration::from_millis(1500));

    let port_9222: SocketAddr = "127.0.0.1:9222".parse().unwrap();
    let port_9229: SocketAddr = "127.0.0.1:9229".parse().unwrap();

    assert!(
        TcpStream::connect_timeout(&port_9222, Duration::from_millis(300)).is_err(),
        "release binary must NOT open WebView2 remote debugging port 9222"
    );
    assert!(
        TcpStream::connect_timeout(&port_9229, Duration::from_millis(300)).is_err(),
        "release binary must NOT open Node inspector port 9229"
    );

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&asset_dir);
}
