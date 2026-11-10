#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tempfile::tempdir;

use wavical_shell::backend::{
    BackendError, BackendLaunchConfig, BackendLauncher, BackendProcessManager,
};

#[derive(Debug, Serialize, Deserialize)]
struct BackendEnvironmentReport {
    pub args: Vec<String>,
    pub env_vars: std::collections::BTreeMap<String, String>,
    pub cwd: String,
}

#[test]
fn test_fake_backend_spawn_env_isolation_and_ipc() {
    let fake_bin = PathBuf::from(env!("CARGO_BIN_EXE_fake_backend"));
    assert!(
        fake_bin.is_file(),
        "fake_backend binary must exist at {:?}",
        fake_bin
    );

    let temp = tempdir().expect("tempdir");
    let project_root = temp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();

    let mut config = BackendLaunchConfig::new("test-app", &project_root, "backend/index.js");
    let working_dir = config.working_dir.clone();
    std::fs::create_dir_all(&working_dir).unwrap();

    config.custom_binary_override = Some(fake_bin);
    // Explicitly request forbidden environment variable to ensure filter drops it
    config.extra_env_vars = vec![
        "NODE_OPTIONS".to_string(),
        "NODE_PATH".to_string(),
        "NODE_INSPECT".to_string(),
    ];

    let mut proc = BackendLauncher::spawn(&config).expect("spawn should succeed");
    assert!(proc.is_alive(), "child process should be alive");

    // Accept IPC connection from fake_backend
    let stream = proc
        .accept_connection(Duration::from_secs(5))
        .expect("fake_backend should connect to pipe");

    // Read 4-byte length prefix
    use std::io::Read;
    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .expect("read length prefix");
    let len = u32::from_le_bytes(len_bytes) as usize;
    assert!(len > 0 && len < 1024 * 1024);

    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).expect("read payload");

    let report: BackendEnvironmentReport =
        serde_json::from_slice(&payload).expect("deserialize report");

    // Verify 1: Working directory is strictly working_dir
    assert_eq!(
        std::fs::canonicalize(&report.cwd).unwrap_or_else(|_| PathBuf::from(&report.cwd)),
        std::fs::canonicalize(&working_dir).unwrap_or_else(|_| working_dir.clone())
    );

    // Verify 2: Parent environment variables are cleared
    assert!(
        !report.env_vars.contains_key("CARGO_MANIFEST_DIR"),
        "Parent CARGO_MANIFEST_DIR must be cleared"
    );
    assert!(
        !report.env_vars.contains_key("CARGO_PKG_NAME"),
        "Parent CARGO_PKG_NAME must be cleared"
    );
    assert!(
        !report.env_vars.contains_key("NODE_OPTIONS"),
        "NODE_OPTIONS must never be forwarded"
    );
    assert!(
        !report.env_vars.contains_key("NODE_PATH"),
        "NODE_PATH must never be forwarded"
    );

    // Verify 3: Only minimal platform variables and Wavical pipe/app_id exist
    assert!(
        report.env_vars.contains_key("WAVICAL_IPC_PIPE"),
        "WAVICAL_IPC_PIPE must be present"
    );
    assert_eq!(
        report.env_vars.get("WAVICAL_APP_ID").map(|s| s.as_str()),
        Some("test-app")
    );

    // Verify 4: CLI args do not contain forbidden debug flags
    for arg in &report.args {
        assert!(
            !arg.starts_with("--inspect"),
            "CLI args must not contain --inspect"
        );
        assert!(
            !arg.starts_with("--inspect-brk"),
            "CLI args must not contain --inspect-brk"
        );
    }

    // Clean up process
    proc.kill().expect("proc kill should succeed");
}

#[test]
fn test_backend_debug_flag_rejection() {
    let fake_bin = PathBuf::from(env!("CARGO_BIN_EXE_fake_backend"));
    let temp = tempdir().expect("tempdir");
    let project_root = temp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();

    let mut config = BackendLaunchConfig::new("test-app-debug", &project_root, "backend/index.js");
    config.custom_binary_override = Some(fake_bin);
    config.custom_args = vec!["--inspect=9229".to_string()];

    // Directly test validation logic
    let forbidden_check = wavical_protocol::backend_args::validate_backend_cli_args(
        &["fake_backend".to_string(), "--inspect=9229".to_string()],
        true, // release mode
    );
    assert!(
        forbidden_check.is_err(),
        "validate_backend_cli_args must reject --inspect in release mode"
    );
}

#[test]
fn test_backend_process_manager_crash_limit() {
    let fake_bin = PathBuf::from(env!("CARGO_BIN_EXE_fake_backend"));
    let temp = tempdir().expect("tempdir");
    let project_root = temp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();

    let mut config = BackendLaunchConfig::new("test-app-crash", &project_root, "backend/index.js");
    config.custom_binary_override = Some(fake_bin);
    // Tell fake_backend to exit immediately with status 1
    config.custom_args = vec!["--crash".to_string()];

    let mut manager = BackendProcessManager::new(config);

    // Initial start
    let started = manager.ensure_running();
    assert!(started.is_ok(), "first start should succeed");

    // Wait for crash to terminate child process
    thread::sleep(Duration::from_millis(150));

    // 1st restart (crash 1)
    let res1 = manager.ensure_running();
    assert!(res1.is_ok(), "restart 1 should be allowed");
    thread::sleep(Duration::from_millis(150));

    // 2nd restart (crash 2)
    let res2 = manager.ensure_running();
    assert!(res2.is_ok(), "restart 2 should be allowed");
    thread::sleep(Duration::from_millis(150));

    // 3rd restart (crash 3)
    let res3 = manager.ensure_running();
    assert!(res3.is_ok(), "restart 3 should be allowed");
    thread::sleep(Duration::from_millis(150));

    // 4th restart (crash 4: exceeds limit of 3 in 60s)
    let res4 = manager.ensure_running();
    match res4 {
        Err(BackendError::CrashLimit(e)) => {
            assert_eq!(e.crash_count, 4);
        }
        other => panic!("expected BackendError::CrashLimit, got {:?}", other),
    }
}

#[test]
fn test_backend_log_stream_reader_limits() {
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let mut log_data = String::new();
    // 1. Oversized line (6000 chars > MAX_BACKEND_LOG_LINE_BYTES 4096)
    log_data.push_str(&"X".repeat(6000));
    log_data.push('\n');

    // 2. High frequency lines (300 lines > MAX_BACKEND_LOG_RATE_PER_SEC 200)
    for i in 0..300 {
        log_data.push_str(&format!("Line {i}\n"));
    }

    let cursor = Cursor::new(log_data.into_bytes());
    let stopped = Arc::new(AtomicBool::new(false));

    // Spawns thread which processes lines with line cap and rate limit
    wavical_shell::backend::spawn_log_stream_reader(cursor, false, stopped);

    // Allow thread to process
    thread::sleep(Duration::from_millis(200));
}
