use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;
use wavical_cli::run_init_with_options;

fn find_system_node() -> PathBuf {
    let candidates = if cfg!(windows) {
        vec!["where.exe", "node"]
    } else {
        vec!["which", "node"]
    };
    let out = Command::new(candidates[0])
        .arg(candidates[1])
        .output()
        .expect("locate system node");
    assert!(
        out.status.success(),
        "system node must be available for test"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    PathBuf::from(stdout.lines().next().expect("node path").trim())
}

fn sha256_hex(path: &Path) -> String {
    let bytes = fs::read(path).expect("read file for sha256");
    // Use powershell/certutil-free pure computation via wavical_ai or simple check?
    // Actually we can compute SHA-256 via `wavical_cli::runtime::compute_sha256`.
    let _ = bytes;
    wavical_cli::runtime::compute_sha256(path).expect("compute sha256")
}

fn setup_mock_managed_runtime(runtimes_dir: &Path, version: &str) -> (PathBuf, String) {
    let arch = wavical_protocol::RuntimeArch::current().as_str();
    let runtime_dir = runtimes_dir.join(format!("node-{version}-{arch}"));
    fs::create_dir_all(&runtime_dir).expect("create runtime dir");

    let sys_node = find_system_node();
    let exe_name = if cfg!(windows) { "node.exe" } else { "node" };
    let dst_node = runtime_dir.join(exe_name);
    fs::copy(&sys_node, &dst_node).expect("copy node binary");

    let exe_sha256 = sha256_hex(&dst_node);
    let meta = serde_json::json!({
        "runtime": "node",
        "version": version,
        "arch": arch,
        "executable_name": exe_name,
        "executable_sha256": exe_sha256,
        "installed_at_secs": 1700000000u64,
    });
    fs::write(
        runtime_dir.join(".wavical-runtime.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .expect("write runtime metadata");

    fs::write(runtime_dir.join("LICENSE"), "MIT License - Node.js Test")
        .expect("write runtime LICENSE");

    (runtime_dir, exe_sha256)
}

fn write_backend_config(project_root: &Path, app_id: &str) {
    let cfg = format!(
        r#"export default {{
  appId: '{app_id}',
  version: '1.0.0',
  capabilities: {{
    'backend.invoke': true
  }},
  backend: {{
    enabled: true,
    runtime: 'node',
    version: '22.11.0',
    entry: 'backend/main.js',
    methods: ['hello']
  }}
}};
"#
    );
    fs::write(project_root.join("wavical.config.js"), cfg).expect("write wavical.config.js");
}

#[test]
fn test_build_backend_requires_lockfile() {
    let temp = tempdir().expect("temp dir");
    let project_dir = temp.path().join("no-lockfile-app");
    run_init_with_options(&project_dir, false, true).expect("init with backend");
    write_backend_config(&project_dir, "no-lockfile-app");

    // Remove the scaffolded package-lock.json
    let lockfile = project_dir.join("backend").join("package-lock.json");
    assert!(lockfile.is_file());
    fs::remove_file(&lockfile).expect("remove lockfile");

    let runtimes_dir = temp.path().join("runtimes");
    setup_mock_managed_runtime(&runtimes_dir, "22.11.0");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&project_dir)
        .env("WAVICAL_RUNTIMES_DIR", &runtimes_dir)
        .output()
        .expect("run wavical build");

    assert!(!out.status.success(), "build without lockfile must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("lockfile missing") && stderr.contains("package-lock.json"),
        "expected clear lockfile error, got: {stderr}"
    );
}

#[test]
fn test_build_backend_ignores_install_scripts_and_excludes_env_and_test_folders() {
    let temp = tempdir().expect("temp dir");
    let project_dir = temp.path().join("backend-build-app");
    run_init_with_options(&project_dir, false, true).expect("init with backend");
    write_backend_config(&project_dir, "backend-build-app");

    let runtimes_dir = temp.path().join("runtimes");
    setup_mock_managed_runtime(&runtimes_dir, "22.11.0");

    // Create a local test package with a postinstall script that writes a marker file
    let evil_pkg_dir = project_dir.join("evil-pkg");
    fs::create_dir_all(&evil_pkg_dir).expect("create evil-pkg dir");
    let marker_path = project_dir.join("postinstall-marker.txt");
    let marker_js = marker_path.to_string_lossy().replace('\\', "\\\\");
    let evil_pkg_json = serde_json::json!({
        "name": "evil-pkg",
        "version": "1.0.0",
        "main": "index.js",
        "scripts": {
            "postinstall": format!("node -e \"require('fs').writeFileSync('{marker_js}', 'PWNED')\"")
        }
    });
    fs::write(
        evil_pkg_dir.join("package.json"),
        serde_json::to_string_pretty(&evil_pkg_json).unwrap(),
    )
    .expect("write evil-pkg package.json");
    fs::write(
        evil_pkg_dir.join("index.js"),
        "module.exports = { ok: true };",
    )
    .expect("write evil-pkg index.js");

    // Configure backend/package.json to depend on evil-pkg and generate lockfile
    let backend_dir = project_dir.join("backend");
    let backend_pkg_json = serde_json::json!({
        "name": "backend-build-app-backend",
        "version": "1.0.0",
        "private": true,
        "dependencies": {
            "evil-pkg": "file:../evil-pkg"
        }
    });
    fs::write(
        backend_dir.join("package.json"),
        serde_json::to_string_pretty(&backend_pkg_json).unwrap(),
    )
    .expect("write backend package.json");

    let lock_out = Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
        .args(["install", "--package-lock-only", "--ignore-scripts"])
        .current_dir(&backend_dir)
        .output()
        .expect("generate package-lock.json");
    assert!(
        lock_out.status.success(),
        "npm package-lock-only failed: {}",
        String::from_utf8_lossy(&lock_out.stderr)
    );

    // Add .env, .git, test/, source maps, and .wavicalignore entry in backend/
    fs::write(backend_dir.join(".env"), "SECRET_KEY=topsecret").expect("write .env");
    fs::write(backend_dir.join(".env.production"), "PROD_SECRET=123")
        .expect("write .env.production");
    fs::create_dir_all(backend_dir.join(".git")).expect("create .git");
    fs::write(
        backend_dir.join(".git").join("HEAD"),
        "ref: refs/heads/main",
    )
    .expect("write .git/HEAD");
    fs::create_dir_all(backend_dir.join("tests")).expect("create tests/");
    fs::write(backend_dir.join("tests").join("unit.test.js"), "test()").expect("write unit test");
    fs::write(backend_dir.join("main.js.map"), "{}").expect("write sourcemap");
    fs::write(backend_dir.join("ignored.log"), "debug log").expect("write ignored.log");
    fs::write(project_dir.join(".wavicalignore"), "*.log\n").expect("write .wavicalignore");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&project_dir)
        .env("WAVICAL_RUNTIMES_DIR", &runtimes_dir)
        .output()
        .expect("run wavical build");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "wavical build failed:\nstdout: {stdout}\nstderr: {stderr}"
    );

    // 1. Marker file must NOT exist (install scripts were ignored!)
    assert!(
        !marker_path.exists(),
        "postinstall marker file must NOT exist when --allow-install-scripts is not passed"
    );

    // 2. Output must list skipped package and exclusion list
    assert!(
        stdout.contains("evil-pkg") && stdout.contains("Skipped install scripts"),
        "stdout should list skipped install scripts for evil-pkg: {stdout}"
    );
    assert!(
        stdout.contains("Build exclusions:")
            && stdout.contains(".env*")
            && stdout.contains("*.log"),
        "stdout should show documented exclusion list: {stdout}"
    );

    // 3. Excluded files must NOT be in dist/backend/
    let dist_backend = project_dir.join("dist").join("backend");
    assert!(dist_backend.join("main.js").is_file());
    assert!(
        dist_backend
            .join("node_modules")
            .join("evil-pkg")
            .join("index.js")
            .is_file()
    );
    assert!(
        !dist_backend.join(".env").exists(),
        ".env must not be copied"
    );
    assert!(
        !dist_backend.join(".env.production").exists(),
        ".env.production must not be copied"
    );
    assert!(
        !dist_backend.join(".git").exists(),
        ".git must not be copied"
    );
    assert!(
        !dist_backend.join("tests").exists(),
        "tests/ must not be copied"
    );
    assert!(
        !dist_backend.join("main.js.map").exists(),
        "source map must not be copied by default"
    );
    assert!(
        !dist_backend.join("ignored.log").exists(),
        ".wavicalignore pattern must not be copied"
    );

    // 4. Verify manifest contains backend files with sha256 and shared-cache runtime mode
    let manifest_str = fs::read_to_string(project_dir.join("dist").join("wavical-manifest.json"))
        .expect("read manifest");
    let manifest: serde_json::Value = serde_json::from_str(&manifest_str).expect("parse manifest");

    assert_eq!(manifest["runtime"]["mode"], "shared-cache");
    assert_eq!(manifest["backend"]["installScriptsAllowed"], false);
    assert!(
        manifest["backend"]["skippedInstallScripts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "evil-pkg")
    );
    let backend_files = manifest["backend"]["files"].as_object().unwrap();
    assert!(backend_files.contains_key("main.js"));
    assert!(backend_files.contains_key("node_modules/evil-pkg/index.js"));
    assert!(
        backend_files["main.js"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(!backend_files.contains_key(".env"));

    // 5. Now test --allow-install-scripts runs the script with warning + manifest note
    let out_allow = Command::new(wavical_bin)
        .arg("build")
        .arg(&project_dir)
        .arg("--allow-install-scripts")
        .env("WAVICAL_RUNTIMES_DIR", &runtimes_dir)
        .output()
        .expect("run wavical build --allow-install-scripts");
    assert!(out_allow.status.success());
    let stderr_allow = String::from_utf8_lossy(&out_allow.stderr);
    assert!(
        stderr_allow.contains("--allow-install-scripts enabled"),
        "expected warning when --allow-install-scripts is used: {stderr_allow}"
    );
    assert!(
        marker_path.is_file(),
        "postinstall marker file should exist when --allow-install-scripts is passed"
    );
    let manifest_allow_str =
        fs::read_to_string(project_dir.join("dist").join("wavical-manifest.json"))
            .expect("read manifest");
    let manifest_allow: serde_json::Value =
        serde_json::from_str(&manifest_allow_str).expect("parse manifest");
    assert_eq!(manifest_allow["backend"]["installScriptsAllowed"], true);
    assert!(manifest_allow["backend"]["installScriptsNote"].is_string());
}

#[test]
fn test_build_bundle_runtime_includes_verified_binary_and_license() {
    let temp = tempdir().expect("temp dir");
    let project_dir = temp.path().join("bundled-app");
    run_init_with_options(&project_dir, false, true).expect("init with backend");
    write_backend_config(&project_dir, "bundled-app");

    let runtimes_dir = temp.path().join("runtimes");
    let (_rt_dir, expected_sha256) = setup_mock_managed_runtime(&runtimes_dir, "22.11.0");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&project_dir)
        .arg("--bundle-runtime")
        .env("WAVICAL_RUNTIMES_DIR", &runtimes_dir)
        .output()
        .expect("run wavical build --bundle-runtime");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "bundled build failed:\nstdout: {stdout}\nstderr: {stderr}"
    );

    let dist_dir = project_dir.join("dist");
    let exe_name = if cfg!(windows) { "node.exe" } else { "node" };
    let bundled_exe = dist_dir.join("runtime").join(exe_name);
    let bundled_license = dist_dir.join("runtime").join("LICENSE");

    assert!(
        bundled_exe.is_file(),
        "bundled runtime executable must exist"
    );
    assert!(
        bundled_license.is_file(),
        "bundled runtime LICENSE must exist"
    );

    let actual_sha256 = sha256_hex(&bundled_exe);
    assert_eq!(actual_sha256, expected_sha256);

    let manifest_str = fs::read_to_string(dist_dir.join("wavical-manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();

    assert_eq!(manifest["runtime"]["mode"], "bundled");
    assert_eq!(
        manifest["runtime"]["executable"],
        format!("runtime/{exe_name}")
    );
    assert_eq!(
        manifest["runtime"]["sha256"],
        format!("sha256:{expected_sha256}")
    );
    assert_eq!(manifest["runtime"]["license"], "runtime/LICENSE");
}

#[test]
fn test_build_fails_if_runtime_hash_is_wrong() {
    let temp = tempdir().expect("temp dir");
    let project_dir = temp.path().join("tampered-rt-app");
    run_init_with_options(&project_dir, false, true).expect("init with backend");
    write_backend_config(&project_dir, "tampered-rt-app");

    let runtimes_dir = temp.path().join("runtimes");
    let (rt_dir, _sha) = setup_mock_managed_runtime(&runtimes_dir, "22.11.0");

    // Tamper with the cached node executable
    let exe_name = if cfg!(windows) { "node.exe" } else { "node" };
    fs::write(rt_dir.join(exe_name), b"TAMPERED_BINARY").expect("tamper node binary");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&project_dir)
        .arg("--bundle-runtime")
        .env("WAVICAL_RUNTIMES_DIR", &runtimes_dir)
        .output()
        .expect("run wavical build");

    assert!(
        !out.status.success(),
        "build must fail when runtime binary hash is wrong"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("SHA-256 verification failed") || stderr.contains("HashMismatch"),
        "expected hash verification error, got: {stderr}"
    );
}
