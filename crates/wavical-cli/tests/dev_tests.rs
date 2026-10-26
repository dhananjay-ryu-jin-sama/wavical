#![cfg(windows)]

use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use wavical_cli::run_init;

#[test]
fn test_dev_fails_on_missing_config() {
    let temp_root = tempdir().expect("create temp dir");
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");

    let out = Command::new(wavical_bin)
        .arg("dev")
        .arg(temp_root.path())
        .output()
        .expect("execute dev");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("wavical.config.js not found"));
}

#[test]
fn test_dev_fails_on_invalid_config() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("invalid-app");
    run_init(&target, false).expect("scaffold app");

    // Write invalid config (unknown capability)
    let bad_config = r#"
export default {
  appId: 'invalid-app',
  capabilities: {
    'core.echo2': true,
  }
};
"#;
    fs::write(target.join("wavical.config.js"), bad_config).expect("write bad config");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("dev")
        .arg(&target)
        .output()
        .expect("execute dev");

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error: invalid config:"));
    assert!(stderr.contains("unknown capability 'core.echo2'"));
}

#[test]
fn test_dev_server_lifecycle_and_hot_reload() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("dev-app");
    run_init(&target, false).expect("scaffold app");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");

    let mut child = Command::new(wavical_bin)
        .arg("dev")
        .arg(&target)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wavical dev");

    let stdout = child.stdout.take().expect("capture stdout");
    let stderr = child.stderr.take().expect("capture stderr");

    // Spawn non-blocking background line readers for both stdout and stderr (detached)
    let (line_tx, line_rx) = mpsc::channel();

    let tx_out = line_tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(l) = line {
                if tx_out.send(l).is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    });

    let tx_err = line_tx;
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(l) = line {
                if tx_err.send(l).is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    });

    let mut shell_pid = 0u32;

    // Helper closure to wait for a matching line with timeout
    let mut wait_for_line =
        |rx: &mpsc::Receiver<String>, pattern: &str, timeout_secs: u64| -> bool {
            let deadline = Instant::now() + Duration::from_secs(timeout_secs);
            while Instant::now() < deadline {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(line) => {
                        println!("[test output]: {line}");
                        if line.contains("Dev server running (PID:")
                            && shell_pid == 0
                            && let Some(start) = line.find("(PID: ")
                        {
                            let rem = &line[start + 6..];
                            if let Some(end) = rem.find(')') {
                                shell_pid = rem[..end].parse::<u32>().unwrap_or(0);
                            }
                        }
                        if line.contains(pattern) {
                            return true;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            false
        };

    // 1. Verify "Dev server running (PID: ...)" appears
    let server_started = wait_for_line(&line_rx, "Dev server running (PID:", 15);
    assert!(
        server_started,
        "Dev server failed to start and print PID within 15 seconds"
    );

    // 2. Modify wavical.config.js with valid capability change
    thread::sleep(Duration::from_millis(400));
    let valid_updated_config = r#"
export default {
  appId: 'dev-app',
  window: { width: 1200, height: 800, title: 'Dev App' },
  capabilities: {
    'core.ping': true
  }
};
"#;
    fs::write(target.join("wavical.config.js"), valid_updated_config).expect("update config");

    let config_reloaded = wait_for_line(&line_rx, "Config reloaded. Capability set updated.", 10);
    assert!(
        config_reloaded,
        "Expected 'Config reloaded. Capability set updated.' message was not observed"
    );

    // 3. Modify wavical.config.js with invalid config
    thread::sleep(Duration::from_millis(400));
    let invalid_updated_config = r#"
export default {
  appId: 'dev-app',
  capabilities: {
    'core.bogus_cap': true
  }
};
"#;
    fs::write(target.join("wavical.config.js"), invalid_updated_config)
        .expect("update config invalid");

    let warning_printed = wait_for_line(&line_rx, "warning: invalid config:", 10);
    assert!(
        warning_printed,
        "Expected 'warning: invalid config:' was not observed"
    );

    // Verify dev server process is still alive and running after invalid config
    assert!(
        child.try_wait().unwrap().is_none(),
        "Dev server should continue running after an invalid config edit"
    );

    // 4. Modify src/index.html to trigger asset hot reload
    thread::sleep(Duration::from_millis(400));
    let updated_html = r#"<!DOCTYPE html><html><body><h1>Updated Hot Reload!</h1></body></html>"#;
    fs::write(target.join("src").join("index.html"), updated_html).expect("write updated html");

    let asset_reloaded = wait_for_line(&line_rx, "Reloaded", 10);
    assert!(
        asset_reloaded,
        "Expected 'Reloaded' message was not observed"
    );

    // 5. Clean termination
    #[cfg(windows)]
    if shell_pid > 0 {
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &shell_pid.to_string()])
            .output();
    }
    #[cfg(not(windows))]
    if shell_pid > 0 {
        let _ = Command::new("kill")
            .args(["-9", &shell_pid.to_string()])
            .output();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn test_dev_auto_creates_missing_src_dir() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("no-src-app");
    fs::create_dir_all(&target).expect("create target dir");

    let config = r#"
export default {
  appId: 'no-src-app',
  capabilities: {}
};
"#;
    fs::write(target.join("wavical.config.js"), config).expect("write config");

    let src_dir = target.join("src");
    assert!(!src_dir.exists(), "src/ should not exist initially");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let mut child = Command::new(wavical_bin)
        .arg("dev")
        .arg(&target)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wavical dev");

    let stdout = child.stdout.take().expect("capture stdout");
    let (line_tx, line_rx) = mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut started = false;
    let mut shell_pid = 0u32;
    while Instant::now() < deadline {
        if let Ok(line) = line_rx.recv_timeout(Duration::from_millis(100))
            && line.contains("Dev server running (PID:")
        {
            started = true;
            if let Some(start) = line.find("(PID: ") {
                let rem = &line[start + 6..];
                if let Some(end) = rem.find(')') {
                    shell_pid = rem[..end].parse::<u32>().unwrap_or(0);
                }
            }
            break;
        }
    }

    assert!(started, "dev server should start even if src/ was missing");
    assert!(
        src_dir.is_dir(),
        "src/ directory must have been auto-created"
    );

    #[cfg(windows)]
    if shell_pid > 0 {
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &shell_pid.to_string()])
            .output();
    }
    #[cfg(not(windows))]
    if shell_pid > 0 {
        let _ = Command::new("kill")
            .args(["-9", &shell_pid.to_string()])
            .output();
    }
    let _ = child.kill();
    let _ = child.wait();
}
