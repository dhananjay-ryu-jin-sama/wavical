#![cfg(windows)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use wavical_cli::{run_init, summarize_config_changes};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn ensure_prod_shell_bin() -> &'static PathBuf {
    static PROD_SHELL_BIN: OnceLock<PathBuf> = OnceLock::new();
    PROD_SHELL_BIN.get_or_init(|| {
        let root = workspace_root();
        let bin = root.join("target").join("debug").join("wavical-shell.exe");
        if !bin.is_file() {
            let out = Command::new("cargo")
                .args(["build", "-p", "wavical-shell", "--bin", "wavical-shell"])
                .current_dir(&root)
                .output()
                .expect("build wavical-shell");
            assert!(
                out.status.success(),
                "failed to build wavical-shell: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        assert!(bin.is_file());
        bin
    })
}

fn ensure_dev_shell_bin() -> &'static PathBuf {
    static DEV_SHELL_BIN: OnceLock<PathBuf> = OnceLock::new();
    DEV_SHELL_BIN.get_or_init(|| {
        let root = workspace_root();
        let target_dir = root.join("target").join("dev-shell");
        let out = Command::new("cargo")
            .args([
                "build",
                "-p",
                "wavical-shell",
                "--bin",
                "wavical-shell-dev",
                "--features",
                "dev-shell",
                "--target-dir",
            ])
            .arg(&target_dir)
            .current_dir(&root)
            .output()
            .expect("build wavical-shell-dev");
        assert!(
            out.status.success(),
            "failed to build wavical-shell-dev: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bin = target_dir.join("debug").join("wavical-shell-dev.exe");
        assert!(bin.is_file());
        bin
    })
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                collect_files_recursive(&p, out);
            } else if p.is_file() {
                out.push(p);
            }
        }
    }
}

#[test]
fn test_build_excludes_all_shells_ignores_dev_url_and_prod_shell_ignores_stdin() {
    let temp = tempdir().expect("tempdir");
    let proj = temp.path().join("build-safety-app");
    run_init(&proj, false).expect("init app");

    // Configure dev.url — builds must completely ignore it
    let config_js = r#"
export default {
  appId: 'build-safety-app',
  window: { width: 1000, height: 700, title: 'Safety' },
  capabilities: { 'core.ping': true },
  dev: { port: 7890, url: 'http://localhost:5173' }
};
"#;
    fs::write(proj.join("wavical.config.js"), config_js).expect("write config");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build-ultra-safe")
        .arg(&proj)
        .output()
        .expect("run wavical build");
    assert!(
        out.status.success(),
        "wavical build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let dist = proj.join("dist");
    let mut dist_files = Vec::new();
    collect_files_recursive(&dist, &mut dist_files);

    for f in &dist_files {
        let name = f
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        assert!(
            !(name.starts_with("wavical-shell") && name.ends_with(".exe")),
            "dist/ must contain no wavical-shell*.exe of either kind, found {}",
            f.display()
        );
    }

    let manifest_raw =
        fs::read_to_string(dist.join("wavical-manifest.json")).expect("read manifest");
    assert!(
        !manifest_raw.contains("localhost:5173") && !manifest_raw.contains("devUrl"),
        "build manifest must ignore dev.url, got: {manifest_raw}"
    );

    // Production shell ignores stdin commands
    let prod_shell = ensure_prod_shell_bin();
    let mut child = Command::new(prod_shell)
        .arg("--app-id")
        .arg("f2-prod-stdin-test")
        .arg("--asset-root")
        .arg(&dist)
        .arg("--single-instance")
        .arg("false")
        .env("WAVICAL_HEADLESS_TEST", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn production shell");

    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, r#"{{"cmd":"reload","id":1,"token":"x"}}"#);
        let _ = writeln!(stdin, r#"{{"cmd":"exit","id":2,"token":"x"}}"#);
        let _ = stdin.flush();
    }

    thread::sleep(Duration::from_millis(500));
    let exited_on_stdin = child.try_wait().expect("try_wait").is_some();
    let _ = child.kill();
    let child_out = child.wait_with_output().expect("wait_with_output");
    let stdout_str = String::from_utf8_lossy(&child_out.stdout);
    assert!(!exited_on_stdin, "production shell must ignore stdin exit");
    assert!(
        !stdout_str.contains(r#""ok":"#),
        "production shell must emit no command replies, got: {stdout_str}"
    );
}

#[test]
fn test_dev_shell_refuses_without_dev_flag_token_or_with_manifest() {
    let dev_shell = ensure_dev_shell_bin();
    let temp = tempdir().expect("tempdir");
    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).expect("create src");
    fs::write(src_dir.join("index.html"), b"<h1>dev</h1>").unwrap();

    // 1. Refuses without --dev even with token set
    let no_dev = Command::new(dev_shell)
        .arg("--asset-root")
        .arg(&src_dir)
        .env("WAVICAL_DEV_SESSION_TOKEN", "tok1234567890abcdef")
        .stdin(Stdio::null())
        .output()
        .expect("run dev shell without --dev");
    assert!(!no_dev.status.success());
    assert!(
        String::from_utf8_lossy(&no_dev.stderr).contains("--dev"),
        "expected --dev error, got: {}",
        String::from_utf8_lossy(&no_dev.stderr)
    );

    // 2. Refuses with --dev but missing session token env var
    let no_token_env = Command::new(dev_shell)
        .arg("--dev")
        .arg("--asset-root")
        .arg(&src_dir)
        .env_remove("WAVICAL_DEV_SESSION_TOKEN")
        .stdin(Stdio::null())
        .output()
        .expect("run dev shell without token env");
    assert!(!no_token_env.status.success());
    assert!(
        String::from_utf8_lossy(&no_token_env.stderr).contains("WAVICAL_DEV_SESSION_TOKEN"),
        "expected token env error, got: {}",
        String::from_utf8_lossy(&no_token_env.stderr)
    );

    // 3. Refuses with --dev and env token when stdin sends wrong token
    let mut wrong_tok_child = Command::new(dev_shell)
        .arg("--dev")
        .arg("--asset-root")
        .arg(&src_dir)
        .env("WAVICAL_DEV_SESSION_TOKEN", "expected-token-1234")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dev shell with wrong stdin token");
    if let Some(mut stdin) = wrong_tok_child.stdin.take() {
        let _ = writeln!(stdin, r#"{{"cmd":"auth","id":0,"token":"wrong-token"}}"#);
    }
    let wrong_out = wrong_tok_child
        .wait_with_output()
        .expect("wait wrong token");
    assert!(!wrong_out.status.success());
    assert!(
        String::from_utf8_lossy(&wrong_out.stderr).contains("invalid dev session token"),
        "expected invalid token error, got: {}",
        String::from_utf8_lossy(&wrong_out.stderr)
    );

    // 4. Refuses with --dev and env token when stdin is closed immediately
    let closed_stdin_out = Command::new(dev_shell)
        .arg("--dev")
        .arg("--asset-root")
        .arg(&src_dir)
        .env("WAVICAL_DEV_SESSION_TOKEN", "expected-token-1234")
        .stdin(Stdio::null())
        .output()
        .expect("run dev shell with closed stdin");
    assert!(!closed_stdin_out.status.success());

    // 5. Refuses to load a build manifest
    let dist_dir = temp.path().join("dist");
    fs::create_dir_all(&dist_dir).unwrap();
    fs::write(dist_dir.join("index.html"), b"<h1>built</h1>").unwrap();
    fs::write(
        dist_dir.join("wavical-manifest.json"),
        r#"{"appId":"a","version":"0.1.0","capabilities":{},"assets":{}}"#,
    )
    .unwrap();
    let manifest_out = Command::new(dev_shell)
        .arg("--dev")
        .arg("--asset-root")
        .arg(&dist_dir)
        .env("WAVICAL_DEV_SESSION_TOKEN", "expected-token-1234")
        .stdin(Stdio::null())
        .output()
        .expect("run dev shell on dist dir");
    assert!(!manifest_out.status.success());
    assert!(
        String::from_utf8_lossy(&manifest_out.stderr).contains("wavical-manifest.json"),
        "expected manifest refusal, got: {}",
        String::from_utf8_lossy(&manifest_out.stderr)
    );
}

#[test]
fn test_cli_reports_clear_error_when_dev_shell_missing() {
    let temp = tempdir().expect("tempdir");
    let proj = temp.path().join("missing-dev-shell");
    run_init(&proj, false).expect("init");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let missing_path = temp.path().join("does-not-exist-wavical-shell-dev.exe");
    let out = Command::new(wavical_bin)
        .env("WAVICAL_DEV_SHELL_BIN", &missing_path)
        .arg("dev")
        .arg(&proj)
        .output()
        .expect("run dev");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("WAVICAL_DEV_SHELL_BIN") || stderr.contains("wavical-shell-dev"),
        "expected clear dev-shell missing error, got: {stderr}"
    );
}

fn compile_fake_dev_shell(dir: &Path) -> PathBuf {
    let src = dir.join("fake_dev_shell.rs");
    let exe = dir.join("fake-wavical-shell-dev.exe");
    let code = r#"
use std::io::{BufRead, Write};
use std::thread;
use std::time::Duration;

fn extract_u64(line: &str, key: &str) -> u64 {
    let needle = format!("\"{}\":", key);
    if let Some(pos) = line.find(&needle) {
        let rest = line[pos + needle.len()..].trim_start();
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        return digits.parse().unwrap_or(0);
    }
    0
}

fn main() {
    let mode = std::env::var("FAKE_SHELL_MODE").unwrap_or_else(|_| "ack".to_string());
    let expected_tok = std::env::var("WAVICAL_DEV_SESSION_TOKEN").unwrap_or_default();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let id = extract_u64(trimmed, "id");
        if trimmed.contains("\"cmd\":\"auth\"") {
            if !expected_tok.is_empty() && trimmed.contains(&expected_tok) {
                let _ = writeln!(stdout, "{{\"id\":{id},\"ok\":true,\"elapsedMs\":1}}");
            } else {
                let _ = writeln!(stdout, "{{\"id\":{id},\"ok\":false,\"error\":\"bad token\",\"elapsedMs\":1}}");
            }
            let _ = stdout.flush();
            continue;
        }
        if trimmed.contains("\"cmd\":\"exit\"") {
            let _ = writeln!(stdout, "{{\"id\":{id},\"ok\":true,\"elapsedMs\":1}}");
            let _ = stdout.flush();
            break;
        }
        match mode.as_str() {
            "ack" => {
                thread::sleep(Duration::from_millis(35));
                let _ = writeln!(stdout, "{{\"id\":{id},\"ok\":true,\"elapsedMs\":35}}");
                let _ = stdout.flush();
            }
            "fail" => {
                let _ = writeln!(stdout, "{{\"id\":{id},\"ok\":false,\"error\":\"simulated reload failure\",\"elapsedMs\":2}}");
                let _ = stdout.flush();
            }
            "hang" => {
                thread::sleep(Duration::from_secs(30));
            }
            _ => {}
        }
    }
}
"#;
    fs::write(&src, code).expect("write fake_dev_shell.rs");
    let out = Command::new("rustc")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("compile fake_dev_shell.rs");
    assert!(
        out.status.success(),
        "rustc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    exe
}

fn spawn_line_collector(
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
) -> mpsc::Receiver<(bool, String)> {
    let (tx, rx) = mpsc::channel();
    let tx_out = tx.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx_out.send((false, line)).is_err() {
                break;
            }
        }
    });
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx.send((true, line)).is_err() {
                break;
            }
        }
    });
    rx
}

fn wait_for_matching_line(
    rx: &mpsc::Receiver<(bool, String)>,
    want_stderr: Option<bool>,
    needle: &str,
    timeout: Duration,
) -> Option<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let rem = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(rem.min(Duration::from_millis(100))) {
            Ok((is_err, line)) => {
                if want_stderr.is_none_or(|e| e == is_err) && line.contains(needle) {
                    return Some(line);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    None
}

#[test]
fn test_cli_waits_for_shell_ack_reports_error_and_times_out_after_5s() {
    let temp = tempdir().expect("tempdir");
    let fake_shell = compile_fake_dev_shell(temp.path());
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");

    // Case 1: ack mode — real Reloaded timing + config reload change report
    let proj_ack = temp.path().join("proj-ack");
    run_init(&proj_ack, false).expect("init ack proj");
    let mut child_ack = Command::new(wavical_bin)
        .env("WAVICAL_DEV_SHELL_BIN", &fake_shell)
        .env("FAKE_SHELL_MODE", "ack")
        .arg("dev")
        .arg(&proj_ack)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wavical dev (ack)");
    let rx_ack = spawn_line_collector(
        child_ack.stdout.take().unwrap(),
        child_ack.stderr.take().unwrap(),
    );
    assert!(
        wait_for_matching_line(
            &rx_ack,
            Some(false),
            "Dev server running",
            Duration::from_secs(10)
        )
        .is_some()
    );

    thread::sleep(Duration::from_millis(250));
    fs::write(proj_ack.join("src").join("index.html"), "<h1>Updated</h1>").unwrap();
    let reloaded_line = wait_for_matching_line(
        &rx_ack,
        Some(false),
        "Reloaded in ",
        Duration::from_secs(10),
    )
    .expect("expected Reloaded line after shell ack");
    assert!(
        !reloaded_line.contains("Reloaded in 0ms"),
        "ack took >=35ms, must not print 0ms: {reloaded_line}"
    );

    thread::sleep(Duration::from_millis(250));
    fs::write(
        proj_ack.join("wavical.config.js"),
        "export default { appId: 'proj-ack', capabilities: { 'core.ping': true } };\n",
    )
    .unwrap();
    let cfg_line = wait_for_matching_line(
        &rx_ack,
        Some(false),
        "Config reloaded. Capability set updated.",
        Duration::from_secs(10),
    )
    .expect("expected config reload line");
    assert!(
        cfg_line.contains("added capabilities: core.ping"),
        "expected change summary in '{cfg_line}'"
    );

    let _ = child_ack.kill();
    let _ = child_ack.wait();

    // Case 2: fail mode — CLI prints error and NEVER prints Reloaded
    let proj_fail = temp.path().join("proj-fail");
    run_init(&proj_fail, false).expect("init fail proj");
    let mut child_fail = Command::new(wavical_bin)
        .env("WAVICAL_DEV_SHELL_BIN", &fake_shell)
        .env("FAKE_SHELL_MODE", "fail")
        .arg("dev")
        .arg(&proj_fail)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wavical dev (fail)");
    let rx_fail = spawn_line_collector(
        child_fail.stdout.take().unwrap(),
        child_fail.stderr.take().unwrap(),
    );
    assert!(
        wait_for_matching_line(
            &rx_fail,
            Some(false),
            "Dev server running",
            Duration::from_secs(10)
        )
        .is_some()
    );
    thread::sleep(Duration::from_millis(250));
    fs::write(proj_fail.join("src").join("index.html"), "<h1>Fail</h1>").unwrap();
    let fail_line = wait_for_matching_line(
        &rx_fail,
        Some(true),
        "simulated reload failure",
        Duration::from_secs(5),
    )
    .expect("expected reload failure on stderr");
    assert!(fail_line.contains("reload failed"));
    let _ = child_fail.kill();
    let _ = child_fail.wait();

    // Drain rx_fail to verify stdout NEVER printed Reloaded
    while let Ok((is_err, line)) = rx_fail.try_recv() {
        if !is_err {
            assert!(
                !line.contains("Reloaded"),
                "CLI must never print Reloaded on failure: {line}"
            );
        }
    }

    // Case 3: hang mode — CLI times out after 5 seconds and NEVER prints Reloaded
    let proj_hang = temp.path().join("proj-hang");
    run_init(&proj_hang, false).expect("init hang proj");
    let mut child_hang = Command::new(wavical_bin)
        .env("WAVICAL_DEV_SHELL_BIN", &fake_shell)
        .env("FAKE_SHELL_MODE", "hang")
        .arg("dev")
        .arg(&proj_hang)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wavical dev (hang)");
    let rx_hang = spawn_line_collector(
        child_hang.stdout.take().unwrap(),
        child_hang.stderr.take().unwrap(),
    );
    assert!(
        wait_for_matching_line(
            &rx_hang,
            Some(false),
            "Dev server running",
            Duration::from_secs(10)
        )
        .is_some()
    );
    thread::sleep(Duration::from_millis(250));
    let start_wait = Instant::now();
    fs::write(proj_hang.join("src").join("index.html"), "<h1>Hang</h1>").unwrap();
    let timeout_line = wait_for_matching_line(
        &rx_hang,
        Some(true),
        "did not acknowledge command within 5s",
        Duration::from_secs(10),
    )
    .expect("expected 5s timeout error on stderr");
    assert!(timeout_line.contains("reload failed"));
    assert!(
        start_wait.elapsed() >= Duration::from_secs(4),
        "timeout fired too early: {:?}",
        start_wait.elapsed()
    );
    let _ = child_hang.kill();
    let _ = child_hang.wait();
    while let Ok((is_err, line)) = rx_hang.try_recv() {
        if !is_err {
            assert!(
                !line.contains("Reloaded"),
                "CLI must never print Reloaded on timeout: {line}"
            );
        }
    }
}

#[test]
fn test_summarize_config_changes_reports_caps_and_csp_mode() {
    let old_cfg =
        wavical_config::parse_config_json(r#"{"appId":"a","capabilities":{"core.ping":true}}"#)
            .unwrap();
    let new_cfg = wavical_config::parse_config_json(
        r#"{"appId":"a","capabilities":{"core.echo":true},"dev":{"url":"http://localhost:5173"}}"#,
    )
    .unwrap();

    let safe_summary = summarize_config_changes(&old_cfg, &new_cfg, true, "safe");
    assert!(safe_summary.contains("added capabilities: core.echo"));
    assert!(safe_summary.contains("removed capabilities: core.ping"));
    assert!(safe_summary.contains("dev.url updated"));
    assert!(safe_summary.contains("allowlist/CSP updated"));

    let ultra_summary = summarize_config_changes(&old_cfg, &new_cfg, true, "ultra-safe");
    assert!(ultra_summary.contains("CSP unchanged (ultra-safe mode)"));
}
