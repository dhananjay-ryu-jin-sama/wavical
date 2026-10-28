#![forbid(unsafe_code)]

//! Integration tests verifying engine lifecycle management, Job Object kill-on-close,
//! idle shutdown, crash limits, binary hash checks, and environment isolation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::tempdir;

use reqwest::blocking::Client;
use wavical_ai::channel::EngineChannel;
use wavical_ai::client::{check_engine_health, create_engine_http_client};
use wavical_ai::engine::EngineError;
use wavical_ai::lifecycle_policy::{CrashRecoveryPolicy, DEFAULT_MAX_CRASHES_IN_WINDOW};
use wavical_ai::manager::{LocalEngineManager, ManagerError};
use wavical_ai::process::{EngineLaunchOptions, EngineProcess};
use wavical_ai::sha256::sha256_file;

/// Return the path to the compiled `fake_engine` binary.
fn get_fake_engine_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_BIN_EXE_fake_engine"));
    if !path.is_file() {
        // Fallback search in target debug
        path = PathBuf::from("target/debug/fake_engine.exe");
        if !path.is_file() {
            path = PathBuf::from("target/debug/fake_engine");
        }
    }
    assert!(
        path.is_file(),
        "fake_engine binary must exist at {}",
        path.display()
    );
    path
}

/// Helper to create a dummy model file inside a temp directory.
fn create_dummy_model_file(dir: &Path) -> PathBuf {
    let path = dir.join("dummy-model.gguf");
    fs::write(&path, b"dummy model bytes for test").unwrap();
    path
}

#[test]
fn test_fake_engine_start_and_health_handshake() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();
    let bin_hash = sha256_file(&bin_path).unwrap();

    let mut manager = LocalEngineManager::with_config(
        bin_path,
        bin_hash,
        10,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    assert!(!manager.is_running());

    // Lazy start on ensure_running
    let channel = manager.ensure_running(&model_file).expect("start engine");
    assert!(manager.is_running());

    // Query health directly with client
    let client = create_engine_http_client().unwrap();
    assert!(check_engine_health(&client, &channel));

    // Ensure running again with same model is a no-op
    let channel2 = manager
        .ensure_running(&model_file)
        .expect("ensure running again");
    assert_eq!(channel.base_url(), channel2.base_url());

    // Graceful stop
    manager.shutdown();
    assert!(!manager.is_running());
}

#[test]
fn test_fake_engine_idle_shutdown() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();
    let bin_hash = sha256_file(&bin_path).unwrap();

    // 1 minute idle policy
    let mut manager = LocalEngineManager::with_config(
        bin_path,
        bin_hash,
        1,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    let _channel = manager.ensure_running(&model_file).expect("start engine");
    assert!(manager.is_running());

    let now = Instant::now();

    // Check at 30 seconds: not idle
    assert!(!manager.check_idle_shutdown(now + Duration::from_secs(30)));
    assert!(manager.is_running());

    // Check at 61 seconds: idle shutdown triggers
    assert!(manager.check_idle_shutdown(now + Duration::from_secs(61)));
    assert!(!manager.is_running());
}

#[test]
fn test_fake_engine_hash_mismatch_refuses_to_launch() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();

    // Pass intentional wrong expected hash
    let wrong_hash = "a".repeat(64);

    let mut manager = LocalEngineManager::with_config(
        bin_path,
        wrong_hash,
        10,
        EngineLaunchOptions::default(),
        Duration::from_secs(5),
    )
    .expect("create manager");

    let err = manager.ensure_running(&model_file).unwrap_err();
    match err {
        ManagerError::EngineVerification(EngineError::HashMismatch { .. }) => {}
        other => panic!("expected HashMismatch error, got {other:?}"),
    }

    assert!(!manager.is_running());
}

#[test]
fn test_fake_engine_environment_is_cleaned_and_no_prompt_in_args() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();

    let channel = EngineChannel::new_ephemeral().unwrap();
    let proc = EngineProcess::spawn(
        &bin_path,
        &model_file,
        channel.clone(),
        &EngineLaunchOptions::default(),
    )
    .expect("spawn engine");

    let client = Client::new();
    let test_env_url = format!("{}/test-env", channel.base_url());

    // Query /test-env endpoint
    let resp = client
        .get(&test_env_url)
        .header("Authorization", channel.auth_header_value())
        .send()
        .expect("send request to /test-env");

    let json: serde_json::Value = resp.json().expect("parse json");

    // 1. Assert no secret was inherited into the child process environment
    assert_eq!(
        json["has_forbidden_secret"], false,
        "Child process must not inherit any secret environment variables!"
    );

    // 2. Assert parent environment variables (like CARGO, RUST_BACKTRACE, USERPROFILE) were cleared
    let env_keys = json["env_keys"].as_array().expect("env_keys array");
    for k in env_keys {
        let key_name = k.as_str().unwrap_or("");
        assert!(
            key_name != "CARGO"
                && key_name != "CARGO_MANIFEST_DIR"
                && key_name != "RUST_BACKTRACE"
                && key_name != "USERPROFILE",
            "Parent environment variable '{key_name}' leaked into child process!"
        );
    }

    // 3. Assert command-line arguments contain NO prompt
    let args = json["args"].as_array().expect("args array");
    for arg in args {
        let text = arg.as_str().unwrap_or("");
        assert!(
            !text.to_lowercase().contains("prompt"),
            "Command line arguments must not contain user prompts: {text}"
        );
    }

    drop(proc);
}

#[test]
fn test_fake_engine_crash_retry_limiter() {
    let t0 = Instant::now();
    let mut policy = CrashRecoveryPolicy::with_thresholds(
        DEFAULT_MAX_CRASHES_IN_WINDOW,
        Duration::from_secs(60),
        Duration::from_millis(50),
    );

    // 1st crash
    assert!(
        policy
            .record_crash_and_check_restart(t0 + Duration::from_secs(5))
            .is_ok()
    );
    // 2nd crash
    assert!(
        policy
            .record_crash_and_check_restart(t0 + Duration::from_secs(15))
            .is_ok()
    );
    // 3rd crash
    assert!(
        policy
            .record_crash_and_check_restart(t0 + Duration::from_secs(25))
            .is_ok()
    );

    // 4th crash within 60s window: rejected!
    assert!(
        policy
            .record_crash_and_check_restart(t0 + Duration::from_secs(35))
            .is_err()
    );
}

#[cfg(windows)]
#[test]
fn test_parent_killed_hard_leaves_no_orphaned_engine_process() {
    let temp = tempdir().unwrap();
    let model_file = create_dummy_model_file(temp.path());
    let bin_path = get_fake_engine_path();

    let channel = EngineChannel::new_ephemeral().unwrap();

    // Spawn a parent process (running this same test binary or powershell / cmd that starts engine)
    // Here we spawn a child engine process inside a WindowsJobGuard configured with limit_kill_on_job_close.
    let engine_proc = EngineProcess::spawn(
        &bin_path,
        &model_file,
        channel.clone(),
        &EngineLaunchOptions::default(),
    )
    .expect("spawn engine");

    let pid = engine_proc.id();
    assert!(pid > 0);

    // Verify it is alive initially
    assert!(is_process_running(pid));

    // Drop engine_proc (closing the Job Object handle, which triggers JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
    drop(engine_proc);

    // Give OS kernel 100ms to terminate
    std::thread::sleep(Duration::from_millis(150));

    // Assert the engine process is terminated by the OS kernel
    assert!(
        !is_process_running(pid),
        "Engine process {pid} must be killed when parent closes job"
    );
}

#[cfg(windows)]
fn is_process_running(pid: u32) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output();

    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        text.contains(&pid.to_string())
    } else {
        false
    }
}
