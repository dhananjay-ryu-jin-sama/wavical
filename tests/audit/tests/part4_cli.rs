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
    assert_eq!(
        err_missing,
        ValidationError::MissingField("appId".to_string())
    );

    // B. Empty appId
    let err_empty = parse_config_json(r#"{"appId": "   "}"#).unwrap_err();
    assert_eq!(
        err_empty,
        ValidationError::MissingField("appId".to_string())
    );

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
    let err_win =
        parse_config_json(r#"{"appId": "app", "window": {"width": 800, "unknownKey": true}}"#)
            .unwrap_err();
    assert!(matches!(err_win, ValidationError::MalformedJson(_)));
    assert!(err_win.to_string().contains("unknown field `unknownKey`"));

    // G. Unknown key inside dev
    let err_dev =
        parse_config_json(r#"{"appId": "app", "dev": {"port": 7890, "unknownDev": 123}}"#)
            .unwrap_err();
    assert!(matches!(err_dev, ValidationError::MalformedJson(_)));
    assert!(err_dev.to_string().contains("unknown field `unknownDev`"));

    // H. Capabilities as an array
    let err_arr =
        parse_config_json(r#"{"appId": "app", "capabilities": ["core.ping"]}"#).unwrap_err();
    assert!(matches!(err_arr, ValidationError::MalformedJson(_)));

    // I. Capability values not boolean
    let err_val_str =
        parse_config_json(r#"{"appId": "app", "capabilities": {"core.ping": "yes"}}"#).unwrap_err();
    assert!(matches!(err_val_str, ValidationError::MalformedJson(_)));
    let err_val_num =
        parse_config_json(r#"{"appId": "app", "capabilities": {"core.ping": 1}}"#).unwrap_err();
    assert!(matches!(err_val_num, ValidationError::MalformedJson(_)));
    let err_val_null =
        parse_config_json(r#"{"appId": "app", "capabilities": {"core.ping": null}}"#).unwrap_err();
    assert!(matches!(err_val_null, ValidationError::MalformedJson(_)));

    // J. Unknown capability with close typo -> suggestion
    let err_typo =
        parse_config_json(r#"{"appId": "app", "capabilities": {"core.echo2": true}}"#).unwrap_err();
    assert!(matches!(err_typo, ValidationError::UnknownCapability(_, _)));
    assert_eq!(err_typo.suggestion(), Some("core.echo"));
    assert!(err_typo.to_string().contains("did you mean 'core.echo'?"));

    // K. Unknown capability with no close match -> no suggestion
    let err_no_match = parse_config_json(
        r#"{"appId": "app", "capabilities": {"completely.unrelated.foo.bar": true}}"#,
    )
    .unwrap_err();
    assert!(matches!(
        err_no_match,
        ValidationError::UnknownCapability(_, _)
    ));
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
    assert!(
        err_throw
            .to_string()
            .contains("Config failed to initialize")
    );

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
    assert!(
        custom_file.is_file(),
        "Extra files must not be deleted by --force"
    );
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
    let b1 = Command::new(&wavical_bin)
        .arg("build")
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(b1.status.success());
    assert!(proj.join("dist/temp_deleted.txt").is_file());

    // 2. Delete file from src/
    fs::remove_file(proj.join("src/temp_deleted.txt")).unwrap();

    // 3. Second build
    let b2 = Command::new(&wavical_bin)
        .arg("build")
        .current_dir(&proj)
        .output()
        .unwrap();
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

/// F-A0-02: Supply-chain gate check (`cargo deny check`) must pass cleanly
#[test]
fn test_supply_chain_cargo_deny_clean() {
    let mut deny_path = PathBuf::from("deny.toml");
    if !deny_path.is_file() {
        deny_path = PathBuf::from("../../deny.toml");
    }
    let deny_content = fs::read_to_string(&deny_path).expect("deny.toml must exist");

    assert!(
        deny_content.contains("\"BSL-1.0\""),
        "deny.toml must explicitly allow BSL-1.0 license (F-A0-02)"
    );
    assert!(
        deny_content.contains("RUSTSEC-2026-0370")
            && deny_content.contains("RUSTSEC-2026-0371")
            && deny_content.contains("RUSTSEC-2026-0372"),
        "deny.toml must ignore transitive unmaintained D-Bus advisories (F-A0-02)"
    );

    let output = match Command::new("cargo")
        .args(["deny", "--offline", "check"])
        .output()
    {
        Ok(out) => out,
        Err(e) => {
            eprintln!("cargo-deny not available ({e}), skipping CLI invocation");
            return;
        }
    };

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() && stderr.contains("no such subcommand") {
        eprintln!("cargo-deny plugin not installed on runner, skipping");
        return;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "cargo deny check failed:\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
}

/// F-A0-03: dev_tests.rs must be guarded with cfg(windows) and taskkill must not be unguarded
#[test]
fn test_linux_ci_no_unguarded_windows_taskkill() {
    let dev_tests = include_str!("../../../crates/wavical-cli/tests/dev_tests.rs");
    assert!(
        dev_tests.contains("#![cfg(windows)]"),
        "crates/wavical-cli/tests/dev_tests.rs must be guarded with #![cfg(windows)] for Linux CI (F-A0-03)"
    );

    let e2e_tests = include_str!("../../../crates/wavical-cli/tests/e2e_tests.rs");

    for (file_name, content) in [("dev_tests.rs", dev_tests), ("e2e_tests.rs", e2e_tests)] {
        let lines: Vec<&str> = content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains("Command::new(\"taskkill\")") {
                let prev_window = lines[i.saturating_sub(4)..i].join("\n");
                assert!(
                    prev_window.contains("#[cfg(windows)]"),
                    "{file_name} line {}: taskkill must be guarded by #[cfg(windows)]",
                    i + 1
                );
            }
        }
    }
}

/// F-A0-04: Architecture docs must formally specify version, tray, shortcuts, and native dependencies
#[test]
fn test_config_schema_and_dependencies_documented() {
    let schema_doc = include_str!("../../../docs/phase 1/WAVICAL_PHASE_1_CONFIG_SCHEMA.md");
    assert!(
        schema_doc.contains("version")
            && schema_doc.contains("tray")
            && schema_doc.contains("shortcuts"),
        "docs/phase 1/WAVICAL_PHASE_1_CONFIG_SCHEMA.md must document version, tray, and shortcuts (F-A0-04)"
    );

    let decisions_doc = include_str!("../../../docs/phase 2/WAVICAL_PHASE_2_DECISIONS.md");
    assert!(
        decisions_doc.contains("getrandom")
            && decisions_doc.contains("ctrlc")
            && decisions_doc.contains("windows-registry"),
        "docs/phase 2/WAVICAL_PHASE_2_DECISIONS.md must document getrandom, ctrlc, and windows-registry dependencies (F-A0-04)"
    );
}

/// F-A0-05: Phase 2 WP9 hostile review notes and tests-fail proof records must exist in docs/
#[test]
fn test_phase_2_wp9_hostile_review_artifacts() {
    let mut doc_path = PathBuf::from("docs/phase 2/WAVICAL_PHASE_2_HOSTILE_REVIEW.md");
    if !doc_path.is_file() {
        doc_path = PathBuf::from("../../docs/phase 2/WAVICAL_PHASE_2_HOSTILE_REVIEW.md");
    }
    assert!(
        doc_path.is_file(),
        "docs/phase 2/WAVICAL_PHASE_2_HOSTILE_REVIEW.md must exist (F-A0-05)"
    );

    let content = fs::read_to_string(&doc_path).expect("hostile review doc must be readable");
    assert!(
        content.contains("Hostile Review") || content.contains("Hostile Self-Review"),
        "Hostile review document must contain hostile review section (F-A0-05)"
    );
    assert!(
        content.to_lowercase().contains("grant reuse"),
        "Hostile review must analyze grant reuse (F-A0-05)"
    );
    assert!(
        content.to_lowercase().contains("toctou"),
        "Hostile review must analyze TOCTOU risks (F-A0-05)"
    );
    assert!(
        content.to_lowercase().contains("tests-fail")
            || content.to_lowercase().contains("tests fail"),
        "Hostile review must contain tests-fail proof records (F-A0-05)"
    );
    assert!(
        content.to_lowercase().contains("capability check")
            && content.to_lowercase().contains("grant check")
            && content.to_lowercase().contains("size limit"),
        "Hostile review must document the three WP9 tests-fail proofs (capability check, grant check, size limit) (F-A0-05)"
    );
}

/// F-A0-07: CI hygiene - GitHub Actions workflow must declare top-level permissions and pin actions to full commit SHAs
#[test]
fn test_ci_hygiene_pinned_actions_and_permissions() {
    let mut ci_path = PathBuf::from(".github/workflows/ci.yml");
    if !ci_path.is_file() {
        ci_path = PathBuf::from("../../.github/workflows/ci.yml");
    }
    assert!(
        ci_path.is_file(),
        ".github/workflows/ci.yml must exist (F-A0-07)"
    );

    let content = fs::read_to_string(&ci_path).expect("ci.yml must be readable");

    // 1. Top-level permissions block must restrict default GITHUB_TOKEN permissions
    assert!(
        content.contains("permissions:") && content.contains("contents: read"),
        "ci.yml must declare explicit top-level permissions: {{ contents: read }} (F-A0-07)"
    );

    // 2. All actions must be pinned to full 40-character commit SHAs instead of mutable tags/branches
    let mut uses_count = 0;
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("uses:") {
            let action_spec = rest.split('#').next().unwrap().trim();
            if action_spec.is_empty() {
                continue;
            }
            uses_count += 1;
            let parts: Vec<&str> = action_spec.split('@').collect();
            assert_eq!(
                parts.len(),
                2,
                "Line {}: Action reference '{}' must follow 'action@ref' format (F-A0-07)",
                line_idx + 1,
                action_spec
            );

            let action_name = parts[0].trim();
            let action_ref = parts[1].trim();

            assert_eq!(
                action_ref.len(),
                40,
                "Line {}: Action '{}' uses mutable ref '{}' instead of a 40-character commit SHA (F-A0-07)",
                line_idx + 1,
                action_name,
                action_ref
            );
            assert!(
                action_ref.chars().all(|c| c.is_ascii_hexdigit()),
                "Line {}: Action ref '{}' must be a valid hex commit SHA (F-A0-07)",
                line_idx + 1,
                action_ref
            );
        }
    }

    assert!(
        uses_count >= 8,
        "Expected at least 8 action invocations in ci.yml, found {}",
        uses_count
    );
}

/// F-A0-08: Pure crates production dependency purity (-e normal) and documented dev-only test exception
#[test]
fn test_pure_crates_production_dependency_purity_and_dev_exception() {
    let prohibited_crates = [
        "tao",
        "wry",
        "rfd",
        "arboard",
        "notify-rust",
        "global-hotkey",
        "tray-icon",
        "muda",
        "windows",
        "windows-sys",
        "winreg",
        "windows-registry",
        "webview2-com",
    ];

    // 1. Verify production dependency trees (-e normal) strictly contain zero prohibited crates
    for crate_name in ["wavical-ipc", "wavical-protocol"] {
        let output = Command::new("cargo")
            .args(["tree", "-p", crate_name, "-e", "normal"])
            .output()
            .expect("cargo tree must succeed");
        assert!(
            output.status.success(),
            "cargo tree -p {crate_name} -e normal failed"
        );
        let tree_stdout = String::from_utf8_lossy(&output.stdout);

        for prohibited in prohibited_crates {
            assert!(
                !tree_stdout.contains(prohibited),
                "Pure crate {crate_name} in production (-e normal) must not contain prohibited dependency '{prohibited}' (F-A0-08)"
            );
        }
    }

    // 2. Formally verify that crates/wavical-ipc/Cargo.toml documents the dev-only test exception
    let mut ipc_cargo_path = PathBuf::from("crates/wavical-ipc/Cargo.toml");
    if !ipc_cargo_path.is_file() {
        ipc_cargo_path = PathBuf::from("../../crates/wavical-ipc/Cargo.toml");
    }
    let ipc_toml = fs::read_to_string(&ipc_cargo_path).expect("wavical-ipc Cargo.toml must exist");
    assert!(
        ipc_toml.contains("Dev-only test exception") || ipc_toml.contains("F-A0-08"),
        "crates/wavical-ipc/Cargo.toml must formally document the dev-only test exception for transitive windows-sys (F-A0-08)"
    );

    // 3. Formally verify that crates/wavical-protocol/Cargo.toml documents the dev-only test exception
    let mut proto_cargo_path = PathBuf::from("crates/wavical-protocol/Cargo.toml");
    if !proto_cargo_path.is_file() {
        proto_cargo_path = PathBuf::from("../../crates/wavical-protocol/Cargo.toml");
    }
    let proto_toml =
        fs::read_to_string(&proto_cargo_path).expect("wavical-protocol Cargo.toml must exist");
    assert!(
        proto_toml.contains("Dev-only test exception") || proto_toml.contains("F-A0-08"),
        "crates/wavical-protocol/Cargo.toml must formally document the dev-only test exception for transitive windows-sys (F-A0-08)"
    );

    // 4. Formally verify that architecture documents record this exception
    let mut plan_path = PathBuf::from("docs/phase 0/WAVICAL_PHASE_0_BUILD_PLAN.md");
    if !plan_path.is_file() {
        plan_path = PathBuf::from("../../docs/phase 0/WAVICAL_PHASE_0_BUILD_PLAN.md");
    }
    let plan_content = fs::read_to_string(&plan_path).expect("Phase 0 build plan must exist");
    assert!(
        plan_content.contains("windows-sys") || plan_content.contains("dev-dependencies"),
        "docs/phase 0/WAVICAL_PHASE_0_BUILD_PLAN.md must formally document the dev-only test dependency exception (F-A0-08)"
    );
}

/// FINDING-C5: Config execution trust model must be documented in README.md, SECURITY.md, and docs
#[test]
fn test_config_execution_trust_model_documented() {
    let mut readme_path = PathBuf::from("README.md");
    if !readme_path.is_file() {
        readme_path = PathBuf::from("../../README.md");
    }
    let readme_content = fs::read_to_string(&readme_path).expect("README.md must exist");
    assert!(
        readme_content.contains("wavical.config.js"),
        "README.md must document wavical.config.js execution trust model (FINDING-C5)"
    );
    assert!(
        readme_content.to_lowercase().contains("developer")
            && (readme_content.to_lowercase().contains("privilege")
                || readme_content.to_lowercase().contains("permission")),
        "README.md must warn that config executes with developer privileges (FINDING-C5)"
    );

    let mut security_path = PathBuf::from("SECURITY.md");
    if !security_path.is_file() {
        security_path = PathBuf::from("../../SECURITY.md");
    }
    let security_content = fs::read_to_string(&security_path).expect("SECURITY.md must exist");
    assert!(
        security_content.contains("wavical.config.js"),
        "SECURITY.md must document wavical.config.js execution trust model (FINDING-C5)"
    );
    assert!(
        security_content.to_lowercase().contains("developer")
            && (security_content.to_lowercase().contains("privilege")
                || security_content.to_lowercase().contains("permission")),
        "SECURITY.md must warn that config executes with developer privileges (FINDING-C5)"
    );

    let mut sec_model_path = PathBuf::from("docs/phase 1/SECURITY_MODEL.md");
    if !sec_model_path.is_file() {
        sec_model_path = PathBuf::from("../../docs/phase 1/SECURITY_MODEL.md");
    }
    let sec_model_content =
        fs::read_to_string(&sec_model_path).expect("docs/phase 1/SECURITY_MODEL.md must exist");
    assert!(
        sec_model_content.contains("wavical.config.js"),
        "docs/phase 1/SECURITY_MODEL.md must document wavical.config.js trust boundary (FINDING-C5)"
    );
}

/// FINDING-C6: Node.js (>= 14.8.0) prerequisite must be declared in setup guides and README
#[test]
fn test_setup_documentation_declares_nodejs_prerequisite() {
    let mut setup_path = PathBuf::from("docs/WAVICAL_SETUP.md");
    if !setup_path.is_file() {
        setup_path = PathBuf::from("../../docs/WAVICAL_SETUP.md");
    }
    let setup_content = fs::read_to_string(&setup_path).expect("docs/WAVICAL_SETUP.md must exist");
    assert!(
        setup_content.contains("Node.js") && setup_content.contains("14.8"),
        "docs/WAVICAL_SETUP.md must explicitly declare Node.js (>= 14.8.0) as a prerequisite (FINDING-C6)"
    );
    assert!(
        setup_content.contains("OpenJS.NodeJS"),
        "docs/WAVICAL_SETUP.md must provide installation command for Node.js (FINDING-C6)"
    );

    let mut readme_path = PathBuf::from("README.md");
    if !readme_path.is_file() {
        readme_path = PathBuf::from("../../README.md");
    }
    let readme_content = fs::read_to_string(&readme_path).expect("README.md must exist");
    assert!(
        readme_content.contains("OpenJS.NodeJS")
            && (readme_content.contains("Node.js") && readme_content.contains("14.8")),
        "README.md must include Node.js prerequisite in Quick Start setup commands (FINDING-C6)"
    );
}
