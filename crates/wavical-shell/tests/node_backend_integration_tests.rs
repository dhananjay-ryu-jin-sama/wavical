#![forbid(unsafe_code)]

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

use wavical_shell::backend::{BackendLaunchConfig, BackendLauncher};

fn is_node_available() -> bool {
    Command::new(if cfg!(windows) { "node.exe" } else { "node" })
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn check_process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(&["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
            .output();
        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            text.contains(&pid.to_string())
        } else {
            false
        }
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("kill")
            .args(&["-0", &pid.to_string()])
            .output();
        output.map(|o| o.status.success()).unwrap_or(false)
    }
}

#[derive(serde::Deserialize, Debug)]
struct PermissionTestResults {
    read_denied: bool,
    write_denied: bool,
    child_denied: bool,
    worker_denied: bool,
}

#[test]
fn test_node_permission_model_enforcement() {
    if !is_node_available() {
        eprintln!("SKIP: Node.js is not installed or available in PATH");
        return;
    }

    let temp = tempdir().expect("tempdir");
    let proj_dir = temp.path().join("proj");
    let backend_dir = proj_dir.join("backend");
    let app_data_dir = temp.path().join("app_data");
    let outside_forbidden_dir = temp.path().join("forbidden_area");

    fs::create_dir_all(&backend_dir).unwrap();
    fs::create_dir_all(&app_data_dir).unwrap();
    fs::create_dir_all(&outside_forbidden_dir).unwrap();

    let forbidden_read_path = outside_forbidden_dir.join("secret_data.txt");
    fs::write(&forbidden_read_path, "SUPER_SECRET_TOKEN").unwrap();

    let forbidden_write_path = outside_forbidden_dir.join("should_not_be_written.txt");
    let result_path = app_data_dir.join("perm_results.json");

    let script_code = format!(
        r#"
const fs = require('fs');
const child_process = require('child_process');
const worker_threads = require('worker_threads');

const results = {{
    read_denied: false,
    write_denied: false,
    child_denied: false,
    worker_denied: false
}};

// 1. Try reading forbidden file outside allowed roots
try {{
    fs.readFileSync({:?}, 'utf8');
}} catch (e) {{
    if (e.code === 'ERR_ACCESS_DENIED') {{
        results.read_denied = true;
    }}
}}

// 2. Try writing forbidden file outside app data directory
try {{
    fs.writeFileSync({:?}, 'tampered content');
}} catch (e) {{
    if (e.code === 'ERR_ACCESS_DENIED') {{
        results.write_denied = true;
    }}
}}

// 3. Try spawning child process
try {{
    child_process.spawn('cmd.exe');
}} catch (e) {{
    if (e.code === 'ERR_ACCESS_DENIED') {{
        results.child_denied = true;
    }}
}}

// 4. Try creating worker thread
try {{
    new worker_threads.Worker('const x = 1;', {{ eval: true }});
}} catch (e) {{
    if (e.code === 'ERR_ACCESS_DENIED') {{
        results.worker_denied = true;
    }}
}}

// Write results to allowed app_data_dir
fs.writeFileSync({:?}, JSON.stringify(results));
"#,
        forbidden_read_path.to_string_lossy(),
        forbidden_write_path.to_string_lossy(),
        result_path.to_string_lossy()
    );

    let script_path = backend_dir.join("index.js");
    fs::write(&script_path, script_code).unwrap();

    let mut config = BackendLaunchConfig::new("perm-test-app", &proj_dir, "backend/index.js");
    config.app_data_dir = app_data_dir.clone();
    config.allow_child_process = false;
    config.allow_workers = false;

    let mut proc = BackendLauncher::spawn(&config).expect("spawn with permissions");

    // Wait up to 10 seconds for process completion
    let start = Instant::now();
    let mut exited = false;
    while start.elapsed() < Duration::from_secs(10) {
        if !proc.is_alive() {
            exited = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(exited, "backend node process should finish within 10s");

    // Verify forbidden write file was never created
    assert!(
        !forbidden_write_path.exists(),
        "forbidden write file must NOT be created"
    );

    // Read result file
    assert!(
        result_path.exists(),
        "result file must be created in allowed app_data_dir"
    );
    let result_content = fs::read_to_string(&result_path).unwrap();
    let res: PermissionTestResults = serde_json::from_str(&result_content).unwrap();

    assert!(
        res.read_denied,
        "Read outside allowed roots must fail with ERR_ACCESS_DENIED"
    );
    assert!(
        res.write_denied,
        "Write outside allowed roots must fail with ERR_ACCESS_DENIED"
    );
    assert!(
        res.child_denied,
        "Child process spawn must fail with ERR_ACCESS_DENIED"
    );
    assert!(
        res.worker_denied,
        "Worker thread creation must fail with ERR_ACCESS_DENIED"
    );
}

#[test]
fn test_node_parent_env_node_options_ignored() {
    if !is_node_available() {
        eprintln!("SKIP: Node.js is not installed or available in PATH");
        return;
    }

    let temp = tempdir().expect("tempdir");
    let proj_dir = temp.path().join("proj");
    let backend_dir = proj_dir.join("backend");
    fs::create_dir_all(&backend_dir).unwrap();

    let marker_executed_file = temp.path().join("marker_was_executed.txt");
    let marker_script = temp.path().join("marker.js");
    fs::write(
        &marker_script,
        format!(
            "require('fs').writeFileSync({:?}, 'pwned');",
            marker_executed_file.to_string_lossy()
        ),
    )
    .unwrap();

    let index_js = backend_dir.join("index.js");
    fs::write(&index_js, "console.log('main script running');").unwrap();

    let mut config = BackendLaunchConfig::new("env-ignore-app", &proj_dir, "backend/index.js");
    config.extra_env_vars = vec!["NODE_OPTIONS".to_string()];

    let mut proc = BackendLauncher::spawn(&config).expect("spawn backend");

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if !proc.is_alive() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    assert!(
        !marker_executed_file.exists(),
        "Parent NODE_OPTIONS preload script must NEVER execute"
    );
}

#[test]
#[cfg(windows)]
fn test_node_job_object_cleanup_on_parent_kill_repeated_10_times() {
    if !is_node_available() {
        eprintln!("SKIP: Node.js is not installed or available in PATH");
        return;
    }

    let harness_bin = PathBuf::from(env!("CARGO_BIN_EXE_job_harness"));
    assert!(
        harness_bin.is_file(),
        "job_harness binary must exist at {:?}",
        harness_bin
    );

    for iteration in 1..=10 {
        // 1. Spawn the harness process
        let mut harness_child = Command::new(&harness_bin)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| {
                panic!("iteration {}: failed to spawn job_harness: {e}", iteration)
            });

        let harness_pid = harness_child.id();
        let stdout = harness_child.stdout.take().expect("piped stdout");
        let mut reader = BufReader::new(stdout);

        // 2. Read CHILD_PID from harness
        let mut line = String::new();
        let mut child_node_pid: Option<u32> = None;
        for _ in 0..50 {
            line.clear();
            if reader.read_line(&mut line).is_ok() && line.starts_with("CHILD_PID:") {
                let pid_str = line.trim_start_matches("CHILD_PID:").trim();
                if let Ok(pid) = pid_str.parse::<u32>() {
                    child_node_pid = Some(pid);
                    break;
                }
            }
            thread::sleep(Duration::from_millis(50));
        }

        let node_pid = child_node_pid.expect(&format!(
            "iteration {}: failed to get child node PID",
            iteration
        ));

        // Confirm node process is alive
        assert!(
            check_process_alive(node_pid),
            "iteration {}: node process (pid {}) should be running",
            iteration,
            node_pid
        );

        // 3. Forcibly kill parent harness process with Stop-Process -Force / taskkill /F
        let kill_status = Command::new("powershell")
            .args(&[
                "-NoProfile",
                "-Command",
                &format!("Stop-Process -Id {} -Force", harness_pid),
            ])
            .status();
        assert!(
            kill_status.is_ok(),
            "iteration {}: Stop-Process failed",
            iteration
        );
        let _ = harness_child.wait();

        // 4. Verify that child node.exe process is killed by Windows Job Object
        let mut dead = false;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if !check_process_alive(node_pid) {
                dead = true;
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        assert!(
            dead,
            "iteration {}: node process (pid {}) was NOT killed when parent terminated",
            iteration, node_pid
        );
    }
}
