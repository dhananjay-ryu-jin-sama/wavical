use std::fs;
use std::process::Command;
use tempfile::tempdir;
use wavical_cli::run_init;

#[test]
fn test_build_creates_dist_and_valid_manifest() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("build-app");
    run_init(&target, false).expect("scaffold app");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .output()
        .expect("execute build");

    assert!(out.status.success(), "wavical build should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("✓ Build complete. App is in dist/. To distribute, zip and ship."));

    let dist_dir = target.join("dist");
    assert!(dist_dir.is_dir(), "dist/ directory should exist");

    let dist_index = dist_dir.join("index.html");
    let dist_styles = dist_dir.join("styles.css");
    let manifest_path = dist_dir.join("wavical-manifest.json");

    assert!(dist_index.is_file(), "dist/index.html should exist");
    assert!(dist_styles.is_file(), "dist/styles.css should exist");
    assert!(
        manifest_path.is_file(),
        "dist/wavical-manifest.json should exist"
    );

    // Verify manifest JSON content
    let manifest_str = fs::read_to_string(&manifest_path).expect("read manifest");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_str).expect("parse manifest JSON");

    assert_eq!(manifest["appId"], "build-app");
    assert_eq!(manifest["version"], "0.1.0");
    assert!(manifest["capabilities"].is_object());

    let assets = manifest["assets"]
        .as_object()
        .expect("assets should be an object");
    assert!(assets.contains_key("index.html"));
    assert!(assets.contains_key("styles.css"));

    let index_hash = assets["index.html"].as_str().expect("index.html hash");
    assert!(index_hash.starts_with("sha256:"));
    assert_eq!(index_hash.len(), 7 + 64);

    let styles_hash = assets["styles.css"].as_str().expect("styles.css hash");
    assert!(styles_hash.starts_with("sha256:"));
    assert_eq!(styles_hash.len(), 7 + 64);
}

#[test]
fn test_build_custom_version_and_capabilities_in_config() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("version-app");
    run_init(&target, false).expect("scaffold app");

    // Config with explicit version and granted capability
    let custom_config = r#"
export default {
  appId: 'version-app',
  version: '2.4.1',
  capabilities: {
    'core.ping': true,
    'core.echo': false
  }
};
"#;
    fs::write(target.join("wavical.config.js"), custom_config).expect("write custom config");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .output()
        .expect("execute build");

    assert!(out.status.success());

    let manifest_str = fs::read_to_string(target.join("dist").join("wavical-manifest.json"))
        .expect("read manifest");
    let manifest: serde_json::Value = serde_json::from_str(&manifest_str).expect("parse JSON");

    assert_eq!(manifest["appId"], "version-app");
    assert_eq!(manifest["version"], "2.4.1");
    assert_eq!(manifest["capabilities"]["core.ping"], true);
    assert_eq!(manifest["capabilities"]["core.echo"], false);
}

#[test]
fn test_build_with_outdir_flag() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("outdir-app");
    run_init(&target, false).expect("scaffold app");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .arg("--outdir")
        .arg("custom-output")
        .output()
        .expect("execute build with --outdir");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("custom-output/"));

    assert!(
        !target.join("dist").exists(),
        "default dist/ should not be created"
    );
    let custom_dir = target.join("custom-output");
    assert!(custom_dir.is_dir(), "custom-output should exist");
    assert!(custom_dir.join("index.html").is_file());
    assert!(custom_dir.join("styles.css").is_file());
    assert!(custom_dir.join("wavical-manifest.json").is_file());
}

#[test]
fn test_build_overwrites_existing_directory() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("overwrite-app");
    run_init(&target, false).expect("scaffold app");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");

    // First build
    let out1 = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .output()
        .expect("execute build 1");
    assert!(out1.status.success());

    let original_html =
        fs::read_to_string(target.join("dist").join("index.html")).expect("read html 1");
    let manifest1_str = fs::read_to_string(target.join("dist").join("wavical-manifest.json"))
        .expect("read manifest 1");
    let manifest1: serde_json::Value = serde_json::from_str(&manifest1_str).unwrap();
    let hash1 = manifest1["assets"]["index.html"]
        .as_str()
        .unwrap()
        .to_string();

    // Modify src/index.html
    let new_content = "<!-- Modified content for overwrite test -->\n<h1>New Title</h1>";
    fs::write(target.join("src").join("index.html"), new_content).expect("update src/index.html");

    // Second build in same target directory
    let out2 = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .output()
        .expect("execute build 2");
    assert!(out2.status.success());

    let updated_html =
        fs::read_to_string(target.join("dist").join("index.html")).expect("read html 2");
    assert_ne!(original_html, updated_html);
    assert_eq!(updated_html, new_content);

    let manifest2_str = fs::read_to_string(target.join("dist").join("wavical-manifest.json"))
        .expect("read manifest 2");
    let manifest2: serde_json::Value = serde_json::from_str(&manifest2_str).unwrap();
    let hash2 = manifest2["assets"]["index.html"]
        .as_str()
        .unwrap()
        .to_string();

    assert_ne!(
        hash1, hash2,
        "Manifest asset hash must reflect the overwritten file"
    );
}

#[test]
fn test_build_fails_when_src_missing() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("missing-src-app");
    run_init(&target, false).expect("scaffold app");

    // Remove src/ directory
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
fn test_build_fails_on_invalid_config() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("bad-cfg-app");
    run_init(&target, false).expect("scaffold app");

    // Write invalid config (unknown capability)
    let bad_config = r#"
export default {
  appId: 'bad-cfg-app',
  capabilities: {
    'core.echo2': true,
  }
};
"#;
    fs::write(target.join("wavical.config.js"), bad_config).expect("write bad config");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .output()
        .expect("execute build");

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error: invalid config:"));
    assert!(stderr.contains("unknown capability 'core.echo2'"));
}

#[test]
fn test_build_warns_on_missing_index_html_and_no_javascript() {
    let temp_root = tempdir().expect("create temp dir");
    let target = temp_root.path().join("warning-app");
    run_init(&target, false).expect("scaffold app");

    // Remove index.html
    fs::remove_file(target.join("src").join("index.html")).expect("remove index.html");

    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&target)
        .output()
        .expect("execute build");

    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("warning: 'src/index.html' not found"));
    assert!(stderr.contains("warning: no JavaScript files found in src/"));
}
