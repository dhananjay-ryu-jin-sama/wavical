use std::fs;
use std::process::Command;
use tempfile::tempdir;
use wavical_cli::{CliError, run_init};

#[test]
fn test_wavical_init_scaffolds_expected_files_and_passes_validation() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("test-app");

    let summary = run_init(&target, false).expect("run_init should succeed");
    assert_eq!(summary.app_id, "test-app");
    assert_eq!(summary.title, "Test App");
    assert!(target.is_dir());

    // 1. wavical.config.js
    let config_path = target.join("wavical.config.js");
    assert!(config_path.is_file());
    let config_content = fs::read_to_string(&config_path).expect("read config");
    assert!(config_content.contains("appId: 'test-app'"));
    assert!(config_content.contains("title: 'Test App'"));
    assert!(config_content.contains("width: 1200"));
    assert!(config_content.contains("height: 800"));
    assert!(config_content.contains("capabilities: {}"));

    // 2. src/index.html
    let html_path = target.join("src").join("index.html");
    assert!(html_path.is_file());
    let html_content = fs::read_to_string(&html_path).expect("read index.html");
    assert!(html_content.contains("<title>Test App</title>"));
    assert!(html_content.contains("<h1>Welcome to Test App</h1>"));
    assert!(html_content.contains("<p>Edit src/index.html to get started.</p>"));

    // 3. src/styles.css
    let css_path = target.join("src").join("styles.css");
    assert!(css_path.is_file());
    let css_content = fs::read_to_string(&css_path).expect("read styles.css");
    assert!(css_content.contains("box-sizing: border-box;"));

    // 4. .gitignore
    let gitignore_path = target.join(".gitignore");
    assert!(gitignore_path.is_file());
    let gitignore_content = fs::read_to_string(&gitignore_path).expect("read .gitignore");
    assert!(gitignore_content.contains("dist/"));
    assert!(gitignore_content.contains("target/"));
    assert!(gitignore_content.contains("node_modules/"));

    // 5. WP1 Config validation passes!
    let validated =
        wavical_config::validate_config(&target).expect("scaffolded config must pass validation");
    assert_eq!(validated.app_id, "test-app");
    assert_eq!(validated.window_width(), 1200);
    assert_eq!(validated.window_height(), 800);
    assert_eq!(validated.window_title(), "Test App");
    assert!(validated.granted_capabilities().is_empty());
}

#[test]
fn test_wavical_init_refuses_non_empty_directory_without_force() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("existing-app");
    fs::create_dir_all(&target).expect("create dir");

    // Add a file to make it non-empty
    let placeholder = target.join("some_file.txt");
    fs::write(&placeholder, "hello").expect("write placeholder");

    let result = run_init(&target, false);
    match result {
        Err(CliError::DirectoryNotEmpty(p)) => {
            assert_eq!(p, target);
        }
        other => panic!("expected DirectoryNotEmpty error, got {other:?}"),
    }
}

#[test]
fn test_wavical_init_overwrites_with_force_flag() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("force-app");
    fs::create_dir_all(&target).expect("create dir");

    let placeholder = target.join("old.txt");
    fs::write(&placeholder, "obsolete").expect("write placeholder");

    let result = run_init(&target, true);
    assert!(result.is_ok(), "init with force should succeed");

    assert!(target.join("wavical.config.js").is_file());
    assert!(target.join("src").join("index.html").is_file());
    assert!(target.join("src").join("styles.css").is_file());
    assert!(target.join(".gitignore").is_file());
}

#[test]
fn test_binary_invocations_wavical_and_wca() {
    let temp_root = tempdir().expect("create temp dir");

    // Test `wavical` binary target
    let wavical_app = temp_root.path().join("wavical-app");
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out_wavical = Command::new(wavical_bin)
        .arg("init")
        .arg(&wavical_app)
        .output()
        .expect("execute wavical init");
    assert!(out_wavical.status.success());
    assert!(wavical_app.join("wavical.config.js").is_file());
    assert!(wavical_app.join("src").join("index.html").is_file());

    // Test `wca` binary target
    let wca_app = temp_root.path().join("wca-app");
    let wca_bin = env!("CARGO_BIN_EXE_wca");
    let out_wca = Command::new(wca_bin)
        .arg("init")
        .arg(&wca_app)
        .output()
        .expect("execute wca init");
    assert!(out_wca.status.success());
    assert!(wca_app.join("wavical.config.js").is_file());
    assert!(wca_app.join("src").join("index.html").is_file());
}
