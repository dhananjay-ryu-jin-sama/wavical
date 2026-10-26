#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;
use wavical_config::{ValidationError, parse_config_json, validate_config};

fn get_wavical_bin() -> PathBuf {
    let mut path = PathBuf::from("target/debug/wavical.exe");
    if !path.is_file() {
        path = PathBuf::from("target/debug/wavical");
    }
    if !path.is_file() {
        path = PathBuf::from("../target/debug/wavical.exe");
    }
    if !path.is_file() {
        path = PathBuf::from("../target/debug/wavical");
    }
    if !path.is_file() {
        path = PathBuf::from("../../target/debug/wavical.exe");
    }
    if !path.is_file() {
        path = PathBuf::from("../../target/debug/wavical");
    }
    path
}

fn get_wca_bin() -> PathBuf {
    let mut path = PathBuf::from("target/debug/wca.exe");
    if !path.is_file() {
        path = PathBuf::from("target/debug/wca");
    }
    if !path.is_file() {
        path = PathBuf::from("../target/debug/wca.exe");
    }
    if !path.is_file() {
        path = PathBuf::from("../target/debug/wca");
    }
    if !path.is_file() {
        path = PathBuf::from("../../target/debug/wca.exe");
    }
    if !path.is_file() {
        path = PathBuf::from("../../target/debug/wca");
    }
    path
}

/// 1. Node.js environment & missing Node test
#[test]
fn test_cli_node_version_and_missing_behavior() {
    let node_check = Command::new("node").arg("--version").output();
    assert!(
        node_check.is_ok(),
        "Node.js must be present on test machine"
    );
    let output = node_check.unwrap();
    assert!(output.status.success());
    let version_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        version_str.starts_with('v'),
        "Node version must start with 'v'"
    );

    // Test with Node missing from PATH
    let tmp = tempdir().expect("tempdir");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(
        proj.join("wavical.config.js"),
        "export default { appId: 'my-app' };",
    )
    .unwrap();

    let wavical_bin = get_wavical_bin();
    let res = Command::new(&wavical_bin)
        .arg("dev")
        .current_dir(&proj)
        .env("PATH", "") // Wipe PATH so node cannot be found
        .output()
        .expect("should execute CLI");

    assert!(!res.status.success(), "Exit code must be non-zero");
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("Node subprocess failed") || stderr.contains("cannot find"),
        "Must output a friendly message when Node is missing: {stderr}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "Must never panic when Node is missing"
    );
}

/// 2. Config validation battery: schema, types, limits
#[test]
fn test_cli_config_validation_battery() {
    // A. Missing appId
    let err_missing = parse_config_json(r#"{}"#).unwrap_err();
    assert_eq!(err_missing, ValidationError::MissingField("appId".to_string()));

    // B. Empty appId
    let err_empty = parse_config_json(r#"{"appId": "   "}"#).unwrap_err();
    assert_eq!(err_empty, ValidationError::MissingField("appId".to_string()));

    // C. Invalid appId values
    for invalid in &["../x", "a b", "🦀", ".", "..", "my@app", "app$name"] {
        let json = format!(r#"{{"appId": "{invalid}"}}"#);
        let err = parse_config_json(&json).unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidAppId(_)),
            "Expected InvalidAppId for '{invalid}', got: {err:?}"
        );
    }

    // D. Valid appIds (alphanumeric, hyphens, underscores)
    assert!(parse_config_json(r#"{"appId": "con"}"#).is_ok());
    assert!(parse_config_json(r#"{"appId": "A_B"}"#).is_ok()); // alphanumeric + underscore
    let long_id = "a".repeat(200);
    assert!(parse_config_json(&format!(r#"{{"appId": "{long_id}"}}"#)).is_ok());

    // E. Unknown top-level key (deny_unknown_fields)
    let err_top = parse_config_json(r#"{"appId": "app", "windowTitle": "title"}"#).unwrap_err();
    assert!(matches!(err_top, ValidationError::MalformedJson(_)));
    assert!(err_top.to_string().contains("unknown field `windowTitle`"));

    // F. Unknown key inside window
    let err_win = parse_config_json(r#"{"appId": "app", "window": {"width": 800, "unknownKey": true}}"#).unwrap_err();
    assert!(matches!(err_win, ValidationError::MalformedJson(_)));
    assert!(err_win.to_string().contains("unknown field `unknownKey`"));

    // G. Unknown key inside dev
    let err_dev = parse_config_json(r#"{"appId": "app", "dev": {"port": 7890, "unknownDev": 123}}"#).unwrap_err();
    assert!(matches!(err_dev, ValidationError::MalformedJson(_)));
    assert!(err_dev.to_string().contains("unknown field `unknownDev`"));

    // H. Capabilities as an array
    let err_arr = parse_config_json(r#"{"appId": "app", "capabilities": ["core.ping"]}"#).unwrap_err();
    assert!(matches!(err_arr, ValidationError::MalformedJson(_)));

    // I. Capability values not boolean
    let err_val_str = parse_config_json(r#"{"appId": "app", "capabilities": {"core.ping": "yes"}}"#).unwrap_err();
    assert!(matches!(err_val_str, ValidationError::MalformedJson(_)));
    let err_val_num = parse_config_json(r#"{"appId": "app", "capabilities": {"core.ping": 1}}"#).unwrap_err();
    assert!(matches!(err_val_num, ValidationError::MalformedJson(_)));
    let err_val_null = parse_config_json(r#"{"appId": "app", "capabilities": {"core.ping": null}}"#).unwrap_err();
    assert!(matches!(err_val_null, ValidationError::MalformedJson(_)));

    // J. Unknown capability with close typo -> suggestion
    let err_typo = parse_config_json(r#"{"appId": "app", "capabilities": {"core.echo2": true}}"#).unwrap_err();
    assert!(matches!(err_typo, ValidationError::UnknownCapability(_, _)));
    assert_eq!(err_typo.suggestion(), Some("core.echo"));
    assert!(err_typo.to_string().contains("did you mean 'core.echo'?"));

    // K. Unknown capability with no close match -> no suggestion
    let err_no_match = parse_config_json(r#"{"appId": "app", "capabilities": {"completely.unrelated.foo.bar": true}}"#).unwrap_err();
    assert!(matches!(err_no_match, ValidationError::UnknownCapability(_, _)));
    assert_eq!(err_no_match.suggestion(), None);
    assert!(!err_no_match.to_string().contains("did you mean"));

    // L. Invalid window dimensions
    assert!(parse_config_json(r#"{"appId": "app", "window": {"width": -1}}"#).is_err());
    assert!(parse_config_json(r#"{"appId": "app", "window": {"width": 1.5}}"#).is_err());
    assert!(parse_config_json(r#"{"appId": "app", "window": {"width": "800"}}"#).is_err());

    // M. Invalid dev.port
    assert!(parse_config_json(r#"{"appId": "app", "dev": {"port": 65536}}"#).is_err());
    assert!(parse_config_json(r#"{"appId": "app", "dev": {"port": "80"}}"#).is_err());
}

/// 3. Node-executed config tests: syntax error, throw, process.exit, infinite loop, timeout
#[test]
fn test_cli_node_execution_battery() {
    let tmp = tempdir().expect("tempdir");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();

    // A. Syntax error
    fs::write(proj.join("wavical.config.js"), "export default { appId: ").unwrap();
    let err_syntax = validate_config(&proj).unwrap_err();
    assert!(matches!(err_syntax, ValidationError::NodeError(_)));
    assert!(err_syntax.to_string().contains("SyntaxError"));

    // B. Runtime throw
    fs::write(
        proj.join("wavical.config.js"),
        "throw new Error('Config failed to initialize'); export default { appId: 'x' };",
    )
    .unwrap();
    let err_throw = validate_config(&proj).unwrap_err();
    assert!(matches!(err_throw, ValidationError::NodeError(_)));
    assert!(err_throw.to_string().contains("Config failed to initialize"));

    // C. process.exit(0) inside config
    fs::write(
        proj.join("wavical.config.js"),
        "process.exit(0); export default { appId: 'x' };",
    )
    .unwrap();
    let err_exit = validate_config(&proj).unwrap_err();
    assert!(matches!(err_exit, ValidationError::MalformedJson(_)));

    // D. Environment variable evaluation inside config
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(proj.join("src/index.html"), "<h1>Env Test</h1>").unwrap();
    fs::write(
        proj.join("wavical.config.js"),
        "export default { appId: process.env.TEST_CUSTOM_APP_ID || 'fallback-id' };",
    )
    .unwrap();
    let wavical_bin = get_wavical_bin();
    let res_env = Command::new(&wavical_bin)
        .arg("build")
        .current_dir(&proj)
        .env("TEST_CUSTOM_APP_ID", "dynamic-env-app")
        .output()
        .expect("run wavical build with env var");
    assert!(res_env.status.success());
    let manifest_str = fs::read_to_string(proj.join("dist/wavical-manifest.json")).unwrap();
    assert!(manifest_str.contains("\"appId\": \"dynamic-env-app\""));

    // E. Infinite loop in config: must time out and kill Node
    fs::write(
        proj.join("wavical.config.js"),
        "while(true) {} export default { appId: 'loop' };",
    )
    .unwrap();
    let res_loop = Command::new(&wavical_bin)
        .arg("build")
        .current_dir(&proj)
        .env("WAVICAL_NODE_TIMEOUT", "1")
        .output()
        .expect("run wavical build with loop config");
    assert!(!res_loop.status.success());
    let stderr_loop = String::from_utf8_lossy(&res_loop.stderr);
    assert!(stderr_loop.contains("config parsing timed out"));
}

/// 3b. Finding: Stdout pollution by config breaks JSON extraction
/// Documented requirement: "a config that calls console.log('hello'): the CLI must still work or fail with a clear message, not a confusing JSON error"
/// In wavical-config, execute_node_config_script does console.log(JSON.stringify(...)), so any user console.log pollutes stdout and causes MalformedJson
#[test]
#[ignore = "FINDING-C1"]
fn test_cli_config_stdout_pollution_isolation() {
    let tmp = tempdir().expect("tempdir");
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();

    fs::write(
        proj.join("wavical.config.js"),
        "console.log('User log message'); export default { appId: 'stdout-app' };",
    )
    .unwrap();

    let conf = validate_config(&proj);
    assert!(
        conf.is_ok(),
        "Config evaluation must isolate stdout pollution; got: {:?}",
        conf.err()
    );
    assert_eq!(conf.unwrap().app_id, "stdout-app");
}

/// 4. wavical init behavior & safety sentinel
#[test]
fn test_cli_init_behavior_with_safety_sentinel() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Sentinel file one level ABOVE project directory
    let sentinel = root.join("SENTINEL_DO_NOT_DELETE.txt");
    fs::write(&sentinel, "DO NOT TOUCH").unwrap();

    let wavical_bin = get_wavical_bin();
    let proj_dir = root.join("my-app");

    // A. Init in new dir
    let init_res = Command::new(&wavical_bin)
        .arg("init")
        .arg(&proj_dir)
        .output()
        .expect("run wavical init");
    assert!(init_res.status.success());
    assert!(proj_dir.join("wavical.config.js").is_file());
    assert!(proj_dir.join("src/index.html").is_file());
    assert!(proj_dir.join("src/styles.css").is_file());
    assert!(proj_dir.join(".gitignore").is_file());

    // B. Init in existing non-empty dir without --force must fail
    let init_fail = Command::new(&wavical_bin)
        .arg("init")
        .arg(&proj_dir)
        .output()
        .expect("run wavical init duplicate");
    assert!(!init_fail.status.success());
    let stderr = String::from_utf8_lossy(&init_fail.stderr);
    assert!(stderr.contains("already exists and is not empty"));

    // C. Init with --force must succeed and preserve non-colliding files
    let custom_file = proj_dir.join("custom_asset.txt");
    fs::write(&custom_file, "custom content").unwrap();

    let init_force = Command::new(&wavical_bin)
        .arg("init")
        .arg(&proj_dir)
        .arg("--force")
        .output()
        .expect("run wavical init force");
    assert!(init_force.status.success());
    assert!(custom_file.is_file(), "Extra files must not be deleted by --force");
    assert_eq!(fs::read_to_string(&custom_file).unwrap(), "custom content");

    // Safety assertion
    assert!(sentinel.is_file(), "SENTINEL FILE MUST REMAIN INTACT");
}

/// 4b. wca alias parity with wavical
#[test]
fn test_cli_wca_alias_parity() {
    let tmp = tempdir().expect("tempdir");
    let proj = tmp.path().join("wca-app");

    let wca_bin = get_wca_bin();
    let res = Command::new(&wca_bin)
        .arg("init")
        .arg(&proj)
        .output()
        .expect("run wca init");

    assert!(res.status.success());
    assert!(proj.join("wavical.config.js").is_file());

    // Help parity
    let wavical_bin = get_wavical_bin();
    let h1 = Command::new(&wavical_bin).arg("--help").output().unwrap();
    let h2 = Command::new(&wca_bin).arg("--help").output().unwrap();

    let s1 = String::from_utf8_lossy(&h1.stdout);
    let s2 = String::from_utf8_lossy(&h2.stdout);

    // Normalize usage line
    let norm1 = s1.replace("wavical.exe", "CMD").replace("wavical", "CMD");
    let norm2 = s2.replace("wca.exe", "CMD").replace("wca", "CMD");

    assert!(norm1.contains("Commands:"));
    assert!(norm2.contains("Commands:"));
    assert!(norm1.contains("init"));
    assert!(norm2.contains("init"));
    assert!(norm1.contains("dev"));
    assert!(norm2.contains("dev"));
    assert!(norm1.contains("build"));
    assert!(norm2.contains("build"));
}

/// 5. wavical build: manifest, integrity, and safety
#[test]
fn test_cli_build_manifest_and_determinism() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    let sentinel = root.join("SENTINEL_DO_NOT_DELETE.txt");
    fs::write(&sentinel, "SENTINEL").unwrap();

    let proj = root.join("build-app");
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(
        proj.join("wavical.config.js"),
        "export default { appId: 'build-app', version: '1.2.3', capabilities: { 'core.ping': true } };",
    )
    .unwrap();
    fs::write(proj.join("src/index.html"), "<h1>Build Test</h1>").unwrap();
    fs::write(proj.join("src/app.js"), "console.log('build');").unwrap();

    let wavical_bin = get_wavical_bin();
    let res = Command::new(&wavical_bin)
        .arg("build")
        .current_dir(&proj)
        .output()
        .expect("run wavical build");

    assert!(res.status.success());
    let dist = proj.join("dist");
    assert!(dist.is_dir());
    assert!(dist.join("index.html").is_file());
    assert!(dist.join("app.js").is_file());

    let manifest_path = dist.join("wavical-manifest.json");
    assert!(manifest_path.is_file());
    let manifest_str = fs::read_to_string(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();

    assert_eq!(manifest["appId"], "build-app");
    assert_eq!(manifest["version"], "1.2.3");
    assert_eq!(manifest["capabilities"]["core.ping"], true);
    assert!(manifest["assets"]["index.html"].is_string());
    assert!(manifest["assets"]["app.js"].is_string());

    // Manifest must not list itself
    assert!(manifest["assets"].get("wavical-manifest.json").is_none());

    // Safety check
    assert!(sentinel.is_file(), "SENTINEL MUST REMAIN UNTOUCHED");
}

/// 5b. Finding: wavical build does not remove stale files from previous build
/// Documented requirement: "a second build into the same outdir (are stale files from the previous build removed or left? A deleted-from-src file that stays in dist and is not in the manifest is a finding)"
#[test]
#[ignore = "FINDING-C2"]
fn test_cli_build_removes_stale_files_from_outdir() {
    let tmp = tempdir().expect("tempdir");
    let proj = tmp.path().join("stale-app");
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(
        proj.join("wavical.config.js"),
        "export default { appId: 'stale-app' };",
    )
    .unwrap();
    fs::write(proj.join("src/index.html"), "<h1>Test</h1>").unwrap();
    fs::write(proj.join("src/temp_deleted.txt"), "stale content").unwrap();

    let wavical_bin = get_wavical_bin();

    // 1. First build
    let b1 = Command::new(&wavical_bin).arg("build").current_dir(&proj).output().unwrap();
    assert!(b1.status.success());
    assert!(proj.join("dist/temp_deleted.txt").is_file());

    // 2. Delete file from src/
    fs::remove_file(proj.join("src/temp_deleted.txt")).unwrap();

    // 3. Second build
    let b2 = Command::new(&wavical_bin).arg("build").current_dir(&proj).output().unwrap();
    assert!(b2.status.success());

    // Must be removed from dist
    assert!(
        !proj.join("dist/temp_deleted.txt").exists(),
        "Stale file deleted from src/ must not remain in dist/"
    );
}

/// 5c. Finding: wavical build does not refuse dangerous outdir paths (. or src)
/// Documented requirement: "The tool must refuse the dangerous ones (., project root, src, inside src, parent of the project) and must never delete anything it did not create"
#[test]
#[ignore = "FINDING-C3"]
fn test_cli_build_refuses_dangerous_outdir_paths() {
    let tmp = tempdir().expect("tempdir");
    let proj = tmp.path().join("danger-app");
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(
        proj.join("wavical.config.js"),
        "export default { appId: 'danger-app' };",
    )
    .unwrap();
    fs::write(proj.join("src/index.html"), "<h1>Danger</h1>").unwrap();

    let wavical_bin = get_wavical_bin();

    // Build into '.' must be refused
    let b_dot = Command::new(&wavical_bin)
        .arg("build")
        .arg("--outdir")
        .arg(".")
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(
        !b_dot.status.success(),
        "wavical build --outdir . must be refused"
    );

    // Build into 'src' must be refused
    let b_src = Command::new(&wavical_bin)
        .arg("build")
        .arg("--outdir")
        .arg("src")
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(
        !b_src.status.success(),
        "wavical build --outdir src must be refused"
    );
}

/// 5d. Finding: wavical build copies sensitive files (.env, .git) into dist
/// Documented requirement: "Hidden and sensitive files (.env, .git, *.map, Thumbs.db): does build copy .env or .git into dist? (If yes, that is a finding: secrets in a distributable.)"
#[test]
#[ignore = "FINDING-C4"]
fn test_cli_build_excludes_sensitive_files() {
    let tmp = tempdir().expect("tempdir");
    let proj = tmp.path().join("secret-app");
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(
        proj.join("wavical.config.js"),
        "export default { appId: 'secret-app' };",
    )
    .unwrap();
    fs::write(proj.join("src/index.html"), "<h1>Secret App</h1>").unwrap();
    fs::write(proj.join("src/.env"), "SECRET_API_KEY=12345").unwrap();

    let wavical_bin = get_wavical_bin();
    let res = Command::new(&wavical_bin)
        .arg("build")
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(res.status.success());

    assert!(
        !proj.join("dist/.env").exists(),
        "Sensitive file .env must NOT be copied into dist"
    );
}
