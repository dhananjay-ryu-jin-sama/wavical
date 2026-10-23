use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

#[test]
fn test_e2e_full_lifecycle_workflow() {
    let temp_root = tempdir().expect("create temp dir");
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");

    // 1. wavical init test-project
    let init_out = Command::new(wavical_bin)
        .arg("init")
        .arg("test-project")
        .current_dir(temp_root.path())
        .output()
        .expect("run wavical init");
    assert!(init_out.status.success(), "wavical init must succeed");

    let project_dir = temp_root.path().join("test-project");
    assert!(project_dir.join("wavical.config.js").is_file());
    assert!(project_dir.join("src").join("index.html").is_file());

    // 2. cd test-project && wavical dev (run, wait 3 seconds, kill cleanly)
    let mut dev_child = Command::new(wavical_bin)
        .arg("dev")
        .current_dir(&project_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wavical dev");

    let stdout = dev_child.stdout.take().expect("capture dev stdout");
    let (line_tx, line_rx) = mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut shell_started = false;
    let mut shell_pid = 0u32;

    while Instant::now() < deadline {
        if let Ok(line) = line_rx.recv_timeout(Duration::from_millis(100))
            && line.contains("Dev server running (PID:")
        {
            shell_started = true;
            if let Some(start) = line.find("(PID: ") {
                let rem = &line[start + 6..];
                if let Some(end) = rem.find(')') {
                    shell_pid = rem[..end].parse::<u32>().unwrap_or(0);
                }
            }
            break;
        }
    }

    assert!(shell_started, "Dev server failed to report running status");

    // Wait 3 seconds to let shell run and serve
    thread::sleep(Duration::from_secs(3));

    // Terminate dev server and shell cleanly
    if shell_pid > 0 {
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &shell_pid.to_string()])
            .output();
    }
    let _ = dev_child.kill();
    let dev_exit = dev_child.wait().expect("dev process wait");
    assert!(
        dev_exit.success()
            || dev_exit.code().is_none()
            || dev_exit.code() == Some(1)
            || dev_exit.code() == Some(0)
    );

    // 3. wavical build
    let build_out = Command::new(wavical_bin)
        .arg("build")
        .current_dir(&project_dir)
        .output()
        .expect("run wavical build");

    assert!(build_out.status.success(), "wavical build must succeed");
    let stdout_build = String::from_utf8_lossy(&build_out.stdout);
    assert!(
        stdout_build.contains("✓ Build complete. App is in dist/. To distribute, zip and ship.")
    );

    // 4. Verify dist/ contents and manifest
    let dist_dir = project_dir.join("dist");
    assert!(dist_dir.join("index.html").is_file());
    assert!(dist_dir.join("styles.css").is_file());
    let manifest_path = dist_dir.join("wavical-manifest.json");
    assert!(manifest_path.is_file());

    let manifest_str = fs::read_to_string(&manifest_path).expect("read manifest");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_str).expect("parse manifest JSON");
    assert_eq!(manifest["appId"], "test-project");
    assert_eq!(manifest["version"], "0.1.0");

    let assets = manifest["assets"].as_object().expect("manifest assets");
    assert!(assets.contains_key("index.html"));
    assert!(assets.contains_key("styles.css"));
    assert!(
        assets["index.html"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(
        assets["styles.css"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}

#[test]
fn test_e2e_error_init_non_empty_refuses_without_force() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("existing-non-empty");
    fs::create_dir_all(&target).expect("create dir");
    fs::write(target.join("existing_file.txt"), "hello").expect("write file");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("init")
        .arg(&target)
        .output()
        .expect("execute init");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already exists and is not empty; use --force to overwrite"));
}

#[test]
fn test_e2e_error_dev_broken_config_missing_appid() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("broken-config-app");
    wavical_cli::run_init(&target, false).expect("scaffold");

    // Remove appId
    let broken_config = r#"
export default {
  window: { width: 1000, height: 700 }
};
"#;
    fs::write(target.join("wavical.config.js"), broken_config).expect("write broken config");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("dev")
        .arg(&target)
        .output()
        .expect("execute dev");

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error: invalid config:"));
    assert!(stderr.contains("missing required field 'appId'"));
}

#[test]
fn test_e2e_error_build_without_src() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("no-src-build-app");
    wavical_cli::run_init(&target, false).expect("scaffold");
    fs::remove_dir_all(target.join("src")).expect("remove src");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .output()
        .expect("execute build");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("source directory"));
    assert!(stderr.contains("does not exist"));
}

#[test]
fn test_e2e_error_build_invalid_outdir() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("invalid-outdir-app");
    wavical_cli::run_init(&target, false).expect("scaffold");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let invalid_path = if cfg!(windows) {
        "C:\\Windows\\System32\\wavical_test_forbidden_outdir"
    } else {
        "/root/wavical_test_forbidden_outdir"
    };

    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .arg("--outdir")
        .arg(invalid_path)
        .output()
        .expect("execute build");

    // Should fail with error creating directory
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to create directory")
            || stderr.contains("Access is denied")
            || stderr.contains("Permission denied")
    );
}

#[test]
fn test_e2e_security_node_timeout_enforced() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("infinite-loop-app");
    wavical_cli::run_init(&target, false).expect("scaffold");

    // Infinite loop in config
    let loop_config = "while(true) {}\nexport default { appId: 'loop' };\n";
    fs::write(target.join("wavical.config.js"), loop_config).expect("write loop config");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let start = Instant::now();
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .env("WAVICAL_NODE_TIMEOUT", "1")
        .output()
        .expect("execute build");

    let elapsed = start.elapsed();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("config parsing timed out"),
        "Expected timeout message, got: {stderr}"
    );
    // Should terminate promptly due to timeout, not hang indefinitely
    assert!(
        elapsed < Duration::from_secs(20),
        "Process took too long to enforce timeout: {:?}",
        elapsed
    );
}

#[test]
fn test_e2e_security_capability_enforcement_bridge() {
    let mut capabilities = wavical_ipc::Capabilities::new();
    // Only grant core.ping, leave core.echo denied
    capabilities.grant("core.ping");

    let registry = wavical_ipc::HandlerRegistry::with_core_handlers();

    // 1. core.ping should be allowed
    let ping_req: wavical_ipc::Request =
        serde_json::from_str(r#"{"id": 1, "method": "core.ping"}"#).unwrap();
    let ping_res = wavical_ipc::dispatch(&ping_req, &capabilities, &registry);
    assert!(ping_res.ok);
    assert_eq!(ping_res.result.unwrap().get("message").unwrap(), "pong");

    // 2. core.echo should be denied
    let echo_req: wavical_ipc::Request =
        serde_json::from_str(r#"{"id": 2, "method": "core.echo", "params": {"secret": "leak"}}"#)
            .unwrap();
    let echo_res = wavical_ipc::dispatch(&echo_req, &capabilities, &registry);
    assert!(!echo_res.ok);
    assert_eq!(echo_res.error.unwrap(), wavical_ipc::GENERIC_METHOD_ERROR);
}

#[test]
fn test_e2e_cli_help_text_clear_and_no_panics() {
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let commands = [
        vec!["--help"],
        vec!["init", "--help"],
        vec!["dev", "--help"],
        vec!["build", "--help"],
    ];

    for args in commands {
        let out = Command::new(wavical_bin)
            .args(&args)
            .output()
            .unwrap_or_else(|e| panic!("failed to run wavical with args {args:?}: {e}"));

        assert!(
            out.status.success(),
            "Help for {args:?} must exit cleanly with code 0"
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("Usage:"));
        assert!(!stdout.is_empty());
    }
}
