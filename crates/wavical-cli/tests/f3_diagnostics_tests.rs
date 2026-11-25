#![cfg(windows)]

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tempfile::tempdir;
use wavical_cli::{format_dev_startup_line, inspect_built_app, run_init};
use wavical_netpolicy::csp::ULTRA_SAFE_CSP;
use wavical_netpolicy::html::hash_inline_script;

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
        bin
    })
}

fn compile_diagnostic_fake_dev_shell(dir: &Path) -> PathBuf {
    let src = dir.join("fake_diag_shell.rs");
    let exe = dir.join("fake-diag-wavical-shell-dev.exe");
    let code = r#"
use std::io::{BufRead, Write};

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
    let mode = std::env::var("FAKE_DIAG_MODE").unwrap_or_else(|_| "burst".to_string());
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
                let _ = writeln!(
                    stdout,
                    "{{\"id\":{id},\"ok\":true,\"elapsedMs\":1,\"shellVersion\":\"1.2.0 (test123)\",\"cspSummary\":\"default-src 'self'; script-src 'self'\"}}"
                );
                let _ = stdout.flush();

                if mode == "burst" {
                    // Emit duplicate CSP blocks (3x), blocked nav, blocked popup, long JS error, console.error, and failed resource load
                    for _ in 0..3 {
                        let _ = writeln!(
                            stdout,
                            "{{\"event\":\"diagnostic\",\"payload\":{{\"kind\":\"csp-violation\",\"directive\":\"script-src\",\"origin\":\"https://cdn.tailwindcss.com\",\"resourceType\":\"script\"}}}}"
                        );
                    }
                    let _ = writeln!(
                        stdout,
                        "{{\"event\":\"diagnostic\",\"payload\":{{\"kind\":\"blocked-navigation\",\"origin\":\"https://evil.example.com\",\"rule\":\"ultra_safe_blocks_external_navigation\"}}}}"
                    );
                    let _ = writeln!(
                        stdout,
                        "{{\"event\":\"diagnostic\",\"payload\":{{\"kind\":\"blocked-new-window\",\"origin\":\"https://popup.example.com\",\"rule\":\"ultra_safe_blocks_open_external\"}}}}"
                    );
                    let long_err = "Z".repeat(700);
                    let _ = writeln!(
                        stdout,
                        "{{\"event\":\"diagnostic\",\"payload\":{{\"kind\":\"js-error\",\"message\":\"{long_err}\",\"location\":\"http://wavical.localhost/app.js?token=TOPSECRET:42\"}}}}"
                    );
                    let _ = writeln!(
                        stdout,
                        "{{\"event\":\"diagnostic\",\"payload\":{{\"kind\":\"console-error\",\"message\":\"bad state\",\"location\":\"http://wavical.localhost/app.js:99\"}}}}"
                    );
                    let _ = writeln!(
                        stdout,
                        "{{\"event\":\"diagnostic\",\"payload\":{{\"kind\":\"resource-load-failed\",\"resourceType\":\"image\",\"origin\":\"https://cdn.example.com\"}}}}"
                    );
                    let _ = stdout.flush();
                } else if mode == "flood" {
                    for i in 0..70 {
                        let _ = writeln!(
                            stdout,
                            "{{\"event\":\"diagnostic\",\"payload\":{{\"kind\":\"csp-violation\",\"directive\":\"script-src\",\"origin\":\"https://cdn{i}.example.com\",\"resourceType\":\"script\"}}}}"
                        );
                    }
                    let _ = stdout.flush();
                }
            } else {
                let _ = writeln!(stdout, "{{\"id\":{id},\"ok\":false,\"error\":\"bad token\",\"elapsedMs\":1}}");
                let _ = stdout.flush();
            }
            continue;
        }
        if trimmed.contains("\"cmd\":\"exit\"") {
            let _ = writeln!(stdout, "{{\"id\":{id},\"ok\":true,\"elapsedMs\":1}}");
            let _ = stdout.flush();
            break;
        }
        let _ = writeln!(stdout, "{{\"id\":{id},\"ok\":true,\"elapsedMs\":1}}");
        let _ = stdout.flush();
    }
}
"#;
    fs::write(&src, code).expect("write fake_diag_shell.rs");
    let out = Command::new("rustc")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("compile fake_diag_shell.rs");
    assert!(
        out.status.success(),
        "rustc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    exe
}

fn spawn_collector(
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

fn collect_until_lines(
    rx: &mpsc::Receiver<(bool, String)>,
    timeout: Duration,
    predicate: impl Fn(&[(bool, String)]) -> bool,
) -> Vec<(bool, String)> {
    let mut lines = Vec::new();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate(&lines) {
            break;
        }
        let rem = deadline.saturating_duration_since(Instant::now());
        if let Ok(item) = rx.recv_timeout(rem.min(Duration::from_millis(100))) {
            lines.push(item);
        }
    }
    lines
}

#[test]
fn doctor_app_reads_manifest_and_prints_mode_csp_and_shell_version() {
    let temp = tempdir().unwrap();
    let _prod_shell = ensure_prod_shell_bin();
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");

    // 1. Manifest missing mode -> reports fallback to ultra-safe + ULTRA_SAFE_CSP
    let app_missing_mode = temp.path().join("app-missing-mode");
    fs::create_dir_all(&app_missing_mode).unwrap();
    fs::write(
        app_missing_mode.join("wavical-manifest.json"),
        r#"{"appId":"demo","version":"1.2.3","capabilities":{},"assets":{}}"#,
    )
    .unwrap();

    let out_missing = Command::new(wavical_bin)
        .env("WAVICAL_SHELL_BIN", _prod_shell)
        .arg("doctor")
        .arg("--app")
        .arg(&app_missing_mode)
        .output()
        .expect("run wavical doctor --app");
    assert!(
        out_missing.status.success(),
        "doctor --app failed: {}",
        String::from_utf8_lossy(&out_missing.stderr)
    );
    let stdout_missing = String::from_utf8_lossy(&out_missing.stdout);
    assert!(stdout_missing.contains("Declared mode:  (missing)"));
    assert!(stdout_missing.contains("Effective mode: ultra-safe"));
    assert!(stdout_missing.contains(ULTRA_SAFE_CSP));
    assert!(stdout_missing.contains("Shell version:"));

    // 2. Safe manifest with allowlist + inlineScriptHashes
    let app_safe = temp.path().join("app-safe");
    fs::create_dir_all(&app_safe).unwrap();
    let al_bytes =
        br#"{"version":1,"scripts":["https://cdn.tailwindcss.com"],"inlineStyles":true}"#;
    fs::write(app_safe.join("wavical.allowlist.json"), al_bytes).unwrap();
    let al_sha = hex::encode(Sha256::digest(al_bytes));
    let script_hash = hash_inline_script(b"window.ok=1;").as_csp_token();
    let safe_manifest = format!(
        r#"{{"appId":"safe-demo","version":"2.0.0","mode":"safe","allowlistSha256":"{al_sha}","inlineScriptHashes":{{"index.html":["{script_hash}"]}},"capabilities":{{}},"assets":{{}}}}"#
    );
    fs::write(app_safe.join("wavical-manifest.json"), safe_manifest).unwrap();

    let report = inspect_built_app(&app_safe).expect("inspect safe app");
    assert_eq!(report.effective_mode, "safe");
    assert!(report.fallback_reason.is_none());
    assert!(report.csp.contains("https://cdn.tailwindcss.com"));
    assert!(report.csp.contains(&script_hash));
    assert!(report.csp.contains("'unsafe-inline'")); // inlineStyles: true

    // 3. Open manifest
    let app_open = temp.path().join("app-open");
    fs::create_dir_all(&app_open).unwrap();
    fs::write(
        app_open.join("wavical-manifest.json"),
        r#"{"appId":"open-demo","version":"1.0.0","mode":"open","capabilities":{},"assets":{}}"#,
    )
    .unwrap();
    let open_report = inspect_built_app(&app_open).expect("inspect open app");
    assert_eq!(open_report.effective_mode, "open");
    assert!(open_report.csp.contains("script-src 'self' https:"));
}

#[test]
fn dev_startup_line_and_diagnostic_forwarding_dedup_and_rate_limit() {
    let temp = tempdir().unwrap();
    let fake_shell = compile_diagnostic_fake_dev_shell(temp.path());
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");

    // 1. Check format_dev_startup_line helper directly for all fields
    let line = format_dev_startup_line(
        "safe",
        "default-src 'self'; script-src 'self'",
        Some("wavical.allowlist.json"),
        Some("http://localhost:5173"),
        "1.2.0 (abc1234)",
    );
    assert!(line.contains("mode=safe"));
    assert!(line.contains("csp=default-src 'self'; script-src 'self'"));
    assert!(line.contains("allowlist=wavical.allowlist.json"));
    assert!(line.contains("dev.url=http://localhost:5173"));
    assert!(line.contains("shell=1.2.0 (abc1234)"));

    // 2. Burst test via `wavical dev-ultra-safe`: startup line + all 4 event types + dedup [x3] + query stripping + 500-char limit
    let proj_burst = temp.path().join("proj-burst");
    run_init(&proj_burst, false).unwrap();
    fs::write(
        proj_burst.join("wavical.config.js"),
        "export default { appId: 'proj-burst', dev: { port: 7890, url: 'http://localhost:5173' } };\n",
    )
    .unwrap();

    let mut child = Command::new(wavical_bin)
        .env("WAVICAL_DEV_SHELL_BIN", &fake_shell)
        .env("FAKE_DIAG_MODE", "burst")
        .arg("dev-ultra-safe")
        .arg(&proj_burst)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wavical dev-ultra-safe");
    let rx = spawn_collector(child.stdout.take().unwrap(), child.stderr.take().unwrap());

    let lines = collect_until_lines(&rx, Duration::from_secs(10), |collected| {
        let has_startup = collected
            .iter()
            .any(|(err, l)| !err && l.contains("mode=ultra-safe"));
        let has_x3 = collected
            .iter()
            .any(|(err, l)| *err && l.contains("[x3] blocked script https://cdn.tailwindcss.com"));
        let has_res = collected.iter().any(|(err, l)| {
            *err && l.contains("failed resource load image https://cdn.example.com")
        });
        has_startup && has_x3 && has_res
    });

    let _ = child.kill();
    let _ = child.wait();

    let all_stdout: Vec<&str> = lines
        .iter()
        .filter(|(e, _)| !e)
        .map(|(_, l)| l.as_str())
        .collect();
    let all_stderr: Vec<&str> = lines
        .iter()
        .filter(|(e, _)| *e)
        .map(|(_, l)| l.as_str())
        .collect();

    let startup = all_stdout
        .iter()
        .find(|l| l.starts_with("dev: "))
        .expect("expected dev startup line");
    assert!(startup.contains("mode=ultra-safe"));
    assert!(startup.contains("csp="));
    assert!(startup.contains("dev.url=http://localhost:5173"));
    assert!(startup.contains("shell=1.2.0 (test123)"));

    let stderr_joined = all_stderr.join("\n");
    assert!(stderr_joined.contains("blocked script https://cdn.tailwindcss.com (script-src) — allowed in \"wavical dev\"; in dev-safe add it to scripts in wavical.allowlist.json"));
    assert!(stderr_joined.contains("[x3] blocked script https://cdn.tailwindcss.com"));
    assert!(stderr_joined.contains("blocked navigation https://evil.example.com"));
    assert!(stderr_joined.contains("blocked new-window https://popup.example.com"));
    assert!(
        stderr_joined.contains("console.error: bad state (http://wavical.localhost/app.js:99)")
    );
    assert!(stderr_joined.contains("failed resource load image https://cdn.example.com"));

    // Verify JS error truncated to 500 chars and query string stripped
    let js_err_line = all_stderr
        .iter()
        .find(|l| l.starts_with("js error:"))
        .expect("expected js error line");
    assert!(!js_err_line.contains("TOPSECRET"));
    assert!(js_err_line.contains("http://wavical.localhost/app.js:42"));
    let prefix_stripped = js_err_line
        .strip_prefix("js error: ")
        .unwrap()
        .split(" — ")
        .next()
        .unwrap();
    assert!(
        prefix_stripped.chars().count() <= 500,
        "js error summary exceeded 500 chars: {}",
        prefix_stripped.chars().count()
    );

    // 3. Flood test: 70 distinct diagnostics -> capped at 50 lines/sec + rate limit summary
    let proj_flood = temp.path().join("proj-flood");
    run_init(&proj_flood, false).unwrap();
    let mut flood_child = Command::new(wavical_bin)
        .env("WAVICAL_DEV_SHELL_BIN", &fake_shell)
        .env("FAKE_DIAG_MODE", "flood")
        .arg("dev-safe")
        .arg(&proj_flood)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wavical dev-safe");
    let rx_flood = spawn_collector(
        flood_child.stdout.take().unwrap(),
        flood_child.stderr.take().unwrap(),
    );

    let flood_lines = collect_until_lines(&rx_flood, Duration::from_secs(10), |collected| {
        collected
            .iter()
            .any(|(err, l)| *err && l.contains("rate limit: suppressed 20 diagnostic message(s)"))
    });
    let _ = flood_child.kill();
    let _ = flood_child.wait();

    assert!(
        flood_lines
            .iter()
            .any(|(err, l)| !err && l.contains("mode=safe") && l.contains("allowlist=")),
        "dev-safe must print mode=safe and allowlist in startup line"
    );
    assert!(
        flood_lines
            .iter()
            .any(|(err, l)| *err && l.contains("rate limit: suppressed 20 diagnostic message(s)")),
        "expected rate limit summary line in flood test"
    );
}

#[test]
fn production_shell_binary_excludes_dev_report_hooks() {
    let prod_bin = ensure_prod_shell_bin();
    let bytes = fs::read(prod_bin).expect("read production wavical-shell binary");
    for dev_only_js in [
        b"__sendDevReport".as_slice(),
        b"unhandledrejection".as_slice(),
    ] {
        assert!(
            !bytes.windows(dev_only_js.len()).any(|w| w == dev_only_js),
            "production wavical-shell must not embed dev diagnostic JS hooks"
        );
    }
}
