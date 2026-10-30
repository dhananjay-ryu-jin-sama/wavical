#![forbid(unsafe_code)]

//! Integration tests for Phase 3 WP8: AI project scaffolding (`wavical init <dir> --ai "<description>"`).
//!
//! Verifies:
//! 1. Untrusted model output validation:
//!    - Path traversal (`..`) rejected
//!    - Absolute paths / drive letters rejected
//!    - Windows reserved device names (`CON`, `PRN`, etc.) rejected
//!    - Alternate data streams (`a:b`) rejected
//!    - Disallowed file extensions (`.exe`, etc.) rejected
//!    - Attempting to generate `wavical.config.js` rejected
//!    - Hard limit of 20 files (21 files fails)
//!    - Hard limit of 1 MB total size (2 MB fails)
//! 2. Capability isolation:
//!    - Model output claiming capability grants is ignored
//!    - Capabilities in code are scanned (`wavical.invoke(`)
//!    - Capabilities are only granted if developer explicitly selects them
//!    - Default capability selection is empty (none)
//! 3. Project confinement:
//!    - Generated files written strictly inside `<dir>/src/`
//!    - CLI synthesizes `wavical.config.js` in safe Rust
//!    - Non-empty directory without `--force` is rejected

use clap::Parser;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;
use wavical_cli::{Cli, CliError, Commands, process_scaffold_and_write, run_init_ai};

#[test]
fn test_wp8_cli_args_parsing() {
    let cli = Cli::try_parse_from([
        "wavical",
        "init",
        "my-ai-app",
        "--ai",
        "Create a simple markdown previewer",
        "--ai-mode",
        "cloud",
        "--accept-capabilities",
        "clipboard.read,dialog.openFile",
    ])
    .expect("parse init with --ai flags");

    match cli.command {
        Commands::Init(args) => {
            assert_eq!(args.dir, PathBuf::from("my-ai-app"));
            assert_eq!(
                args.ai.as_deref(),
                Some("Create a simple markdown previewer")
            );
            assert_eq!(args.ai_mode.as_deref(), Some("cloud"));
            assert_eq!(
                args.accept_capabilities.as_deref(),
                Some("clipboard.read,dialog.openFile")
            );
            assert!(!args.force);
        }
        _ => panic!("expected init command"),
    }
}

#[test]
fn test_wp8_generated_path_dot_dot_traversal_rejected() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-traversal");

    let evil_payload = r#"[
        {"path": "../escape.js", "content": "alert('escaped');"}
    ]"#;

    let res = process_scaffold_and_write(
        &target,
        "app-traversal",
        "App Traversal",
        evil_payload,
        None,
    );
    assert!(res.is_err(), "Path traversal must be rejected");
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("path traversal ('..') is forbidden"),
        "Unexpected error message: {err_msg}"
    );
}

#[test]
fn test_wp8_generated_path_absolute_rejected() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-abs");

    // Unix style absolute path
    let evil_abs = r#"[
        {"path": "/root/escape.js", "content": "console.log('abs');"}
    ]"#;
    let res = process_scaffold_and_write(&target, "app-abs", "App Abs", evil_abs, None);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("absolute paths are forbidden"));

    // Windows drive letter absolute path
    let evil_drive = r#"[
        {"path": "C:\\Windows\\System32\\bad.js", "content": "console.log('drive');"}
    ]"#;
    let res = process_scaffold_and_write(&target, "app-abs", "App Abs", evil_drive, None);
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("drive letters and alternate data streams"));
}

#[test]
fn test_wp8_generated_path_con_reserved_rejected() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-con");

    let evil_payload = r#"[
        {"path": "CON.txt", "content": "dos device payload"}
    ]"#;

    let res = process_scaffold_and_write(&target, "app-con", "App Con", evil_payload, None);
    assert!(res.is_err(), "CON device must be rejected");
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("reserved Windows device name"),
        "Unexpected error message: {err_msg}"
    );

    let evil_prn = r#"[
        {"path": "sub/PRN.js", "content": "console.log(1);"}
    ]"#;
    let res_prn = process_scaffold_and_write(&target, "app-con", "App Con", evil_prn, None);
    assert!(res_prn.is_err(), "PRN device must be rejected");
}

#[test]
fn test_wp8_generated_path_alternate_data_stream_rejected() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-ads");

    let evil_payload = r#"[
        {"path": "a:b", "content": "hidden stream"}
    ]"#;

    let res = process_scaffold_and_write(&target, "app-ads", "App Ads", evil_payload, None);
    assert!(res.is_err(), "Alternate data streams must be rejected");
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("alternate data streams (':') are forbidden"),
        "Unexpected error message: {err_msg}"
    );
}

#[test]
fn test_wp8_twenty_one_files_rejected() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-21");

    let mut files = Vec::new();
    for i in 0..21 {
        files.push(format!(r#"{{"path": "file_{i}.txt", "content": "ok"}}"#));
    }
    let payload = format!("[{}]", files.join(",\n"));

    let res = process_scaffold_and_write(&target, "app-21", "App 21", &payload, None);
    assert!(res.is_err(), "21 files must exceed limit of 20");
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("file count limit exceeded: 21 files exceeds maximum of 20"),
        "Unexpected error message: {err_msg}"
    );

    // 20 files must pass
    let mut files_20 = Vec::new();
    for i in 0..20 {
        files_20.push(format!(r#"{{"path": "file_{i}.txt", "content": "ok"}}"#));
    }
    let payload_20 = format!("[{}]", files_20.join(",\n"));
    let target_20 = tmp.path().join("app-20");
    let res_20 = process_scaffold_and_write(&target_20, "app-20", "App 20", &payload_20, None);
    assert!(res_20.is_ok(), "20 files must pass boundary");
}

#[test]
fn test_wp8_two_megabytes_output_rejected() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-2mb");

    let large_content = "x".repeat(2 * 1024 * 1024);
    let payload = format!(r#"[{{"path": "large.txt", "content": "{large_content}"}}]"#);

    let res = process_scaffold_and_write(&target, "app-2mb", "App 2MB", &payload, None);
    assert!(res.is_err(), "2 MB output must be rejected");
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("total generated size exceeded"),
        "Unexpected error message: {err_msg}"
    );
}

#[test]
fn test_wp8_exe_extension_rejected() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-exe");

    let evil_payload = r#"[
        {"path": "malware.exe", "content": "MZfakeexe"}
    ]"#;

    let res = process_scaffold_and_write(&target, "app-exe", "App Exe", evil_payload, None);
    assert!(res.is_err(), ".exe extension must be rejected");
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("only .html, .css, .js, .json, .svg, and .txt files are permitted"),
        "Unexpected error message: {err_msg}"
    );
}

#[test]
fn test_wp8_file_tries_to_write_config_rejected() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-cfg");

    let evil_payload = r#"[
        {"path": "wavical.config.js", "content": "export default { capabilities: { 'process.spawn': true } };"}
    ]"#;

    let res = process_scaffold_and_write(&target, "app-cfg", "App Cfg", evil_payload, None);
    assert!(
        res.is_err(),
        "Model attempting to write wavical.config.js must be rejected"
    );
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("generating configuration files or files outside src/ is forbidden"),
        "Unexpected error message: {err_msg}"
    );

    // Also check disguised attempt via src/../wavical.config.js
    let evil_nested = r#"[
        {"path": "src/../wavical.config.js", "content": "export default {};"}
    ]"#;
    let res_nested = process_scaffold_and_write(&target, "app-cfg", "App Cfg", evil_nested, None);
    assert!(res_nested.is_err());
}

#[test]
fn test_wp8_capability_grant_in_model_output_ignored() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-cap-ignore");

    // Model tries to specify capabilities in JSON or in comment
    let payload = r#"[
        {"path": "index.html", "content": "<!-- wavical.grant: fs.readGranted, process.spawn --><h1>App</h1>"},
        {"path": "app.js", "content": "console.log('safe code');"}
    ]"#;

    let summary = process_scaffold_and_write(&target, "app-cap-ignore", "App Cap", payload, None)
        .expect("scaffold write should succeed");

    assert_eq!(summary.app_id, "app-cap-ignore");

    // Read the generated config created by CLI
    let config_content =
        fs::read_to_string(target.join("wavical.config.js")).expect("read wavical.config.js");

    // Capabilities must be empty by default!
    assert!(
        config_content.contains("capabilities: {}"),
        "Capabilities must be empty: {config_content}"
    );
    assert!(
        !config_content.contains("fs.readGranted"),
        "Model-claimed capability must not be granted"
    );
    assert!(
        !config_content.contains("process.spawn"),
        "Model-claimed capability must not be granted"
    );
}

#[test]
fn test_wp8_capability_scanning_and_explicit_user_selection() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("app-cap-select");

    let payload = r#"[
        {"path": "index.html", "content": "<button onclick=\"wavical.invoke('clipboard.read')\">Paste</button>"},
        {"path": "app.js", "content": "const file = await wavical.invoke('dialog.openFile');"}
    ]"#;

    // Developer explicitly accepts only clipboard.read
    let summary = process_scaffold_and_write(
        &target,
        "app-cap-select",
        "App Cap Select",
        payload,
        Some("clipboard.read"),
    )
    .expect("scaffold write should succeed");

    assert_eq!(summary.app_id, "app-cap-select");

    let config_content =
        fs::read_to_string(target.join("wavical.config.js")).expect("read wavical.config.js");

    assert!(
        config_content.contains("'clipboard.read': true,"),
        "Approved capability must be in config: {config_content}"
    );
    assert!(
        !config_content.contains("'dialog.openFile': true"),
        "Unapproved capability must not be in config: {config_content}"
    );
}

#[test]
fn test_wp8_valid_scaffold_writes_only_inside_src() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("my-new-app");

    let payload = r#"[
        {"path": "index.html", "content": "<!DOCTYPE html><html><body><h1>Hello</h1></body></html>"},
        {"path": "styles.css", "content": "body { margin: 0; background: #fafafa; }"},
        {"path": "app.js", "content": "console.log('App ready');"},
        {"path": "assets/icon.svg", "content": "<svg></svg>"}
    ]"#;

    let summary = process_scaffold_and_write(&target, "my-new-app", "My New App", payload, None)
        .expect("scaffold write should succeed");

    assert_eq!(summary.app_id, "my-new-app");
    assert_eq!(summary.title, "My New App");

    // Check files on disk
    assert!(target.join("src/index.html").is_file());
    assert!(target.join("src/styles.css").is_file());
    assert!(target.join("src/app.js").is_file());
    assert!(target.join("src/assets/icon.svg").is_file());
    assert!(target.join("wavical.config.js").is_file());
    assert!(target.join(".gitignore").is_file());

    // Verify nothing else was written in target root
    let root_entries: Vec<String> = fs::read_dir(&target)
        .expect("read dir")
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .collect();

    assert_eq!(
        root_entries.len(),
        3,
        "Target root should contain exactly src, wavical.config.js, and .gitignore: {:?}",
        root_entries
    );
    assert!(root_entries.contains(&"src".to_string()));
    assert!(root_entries.contains(&"wavical.config.js".to_string()));
    assert!(root_entries.contains(&".gitignore".to_string()));
}

#[test]
fn test_wp8_non_empty_dir_requires_force() {
    let tmp = tempdir().expect("tempdir");
    let target = tmp.path().join("non-empty-dir");
    fs::create_dir_all(&target).expect("create dir");
    fs::write(target.join("existing.txt"), "hello").expect("write existing");

    let res = run_init_ai(
        &target,
        false, // force = false
        "Create an app",
        None,
        None,
    );

    assert!(matches!(res, Err(CliError::DirectoryNotEmpty(_))));
}
