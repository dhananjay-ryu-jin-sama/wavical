#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::thread;
use std::time::Duration;

use wavical_shell::backend::{BackendLaunchConfig, BackendLauncher};

fn main() {
    let pid = std::process::id();
    let temp = std::env::temp_dir().join(format!("wavical_harness_{pid}"));
    let proj_dir = temp.join("proj");
    let backend_dir = proj_dir.join("backend");
    fs::create_dir_all(&backend_dir).unwrap();

    let script_path = backend_dir.join("index.js");
    fs::write(&script_path, "setInterval(() => {}, 1000);").unwrap();

    let config = BackendLaunchConfig::new("harness-app", &proj_dir, "backend/index.js");

    let proc = match BackendLauncher::spawn(&config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("HARNESS_ERROR: failed to spawn: {e}");
            std::process::exit(1);
        }
    };

    let child_pid = proc.pid();
    println!("CHILD_PID:{child_pid}");
    let _ = std::io::stdout().flush();

    // Sleep indefinitely so parent can be forcefully killed externally
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
