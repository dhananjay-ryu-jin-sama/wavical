#![cfg(windows)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tempfile::tempdir;
use wavical_cli::signer::{
    build_release_manifest, generate_signing_key, sign_manifest, verify_manifest_signature,
};
use wavical_netpolicy::csp::ULTRA_SAFE_CSP;
use wavical_netpolicy::html::hash_inline_script;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn ensure_prod_shell_bin() -> &'static PathBuf {
    static PROD_SHELL_BIN: OnceLock<PathBuf> = OnceLock::new();
    PROD_SHELL_BIN.get_or_init(|| {
        let root = workspace_root();
        let bin = root.join("target").join("debug").join("wavical-shell.exe");
        if !bin.is_file() {
            let out = Command::new("cargo")
                .args(["build", "-p", "wavical-shell", "--bin", "wavical-shell"])
                .current_dir(&root)
                .output()
                .expect("build wavical-shell");
            assert!(
                out.status.success(),
                "failed to build wavical-shell: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        bin
    })
}

fn create_my_app_fixture(root: &Path, app_id: &str) -> PathBuf {
    let proj = root.join(app_id);
    fs::create_dir_all(proj.join("src")).unwrap();

    fs::write(
        proj.join("wavical.config.js"),
        format!(
            r#"export default {{
  appId: '{app_id}',
  version: '1.2.0',
  window: {{ width: 1200, height: 800, title: 'My App Fixture' }},
  capabilities: {{ 'core.ping': true }}
}};
"#
        ),
    )
    .unwrap();

    fs::write(
        proj.join("src").join("index.html"),
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>My App Fixture</title>
  <link href="https://fonts.googleapis.com/css2?family=Syne:wght@600&display=swap" rel="stylesheet">
  <script src="https://cdn.tailwindcss.com"></script>
  <script src="script.js"></script>
  <link rel="stylesheet" href="style.css">
</head>
<body>
  <img src="https://www.designarena.ai/u/logo.png" onerror="this.remove()">
  <script>window.__inlineBoot = 1;</script>
  <script type="module">export const ready = true;</script>
</body>
</html>
"#,
    )
    .unwrap();

    fs::write(
        proj.join("src").join("script.js"),
        "window.__mainReady = true;\n",
    )
    .unwrap();
    fs::write(
        proj.join("src").join("style.css"),
        "body { background: #050814; }\n",
    )
    .unwrap();

    fs::write(
        proj.join("wavical.allowlist.json"),
        r#"{
  "version": 1,
  "scripts": ["https://cdn.tailwindcss.com"],
  "styles": ["https://fonts.googleapis.com"],
  "fonts": ["https://fonts.gstatic.com"],
  "images": ["https://www.designarena.ai"],
  "inlineStyles": true,
  "dataImages": true
}
"#,
    )
    .unwrap();

    proj
}

fn wait_for_log_containing(log_path: &Path, needle: &str, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(text) = fs::read_to_string(log_path)
            && text.contains(needle)
        {
            return Some(text);
        }
        thread::sleep(Duration::from_millis(50));
    }
    fs::read_to_string(log_path).ok()
}

fn spawn_shell_and_wait_for_log(
    shell_bin: &Path,
    dist_dir: &Path,
    local_appdata: &Path,
    app_id: &str,
    needle: &str,
) -> String {
    let log_path = local_appdata
        .join("Wavical")
        .join("apps")
        .join(app_id)
        .join("logs")
        .join("shell.log");
    let _ = fs::remove_file(&log_path);

    let mut child = Command::new(shell_bin)
        .arg("--app-id")
        .arg(app_id)
        .arg("--asset-root")
        .arg(dist_dir)
        .arg("--single-instance")
        .arg("false")
        .env("LOCALAPPDATA", local_appdata)
        .env("WAVICAL_HEADLESS_TEST", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wavical-shell");

    let log_text = wait_for_log_containing(&log_path, needle, Duration::from_secs(6));
    let exited_early = child.try_wait().expect("try_wait shell");
    let _ = child.kill();
    let output = child.wait_with_output().expect("wait shell");

    assert!(
        exited_early.is_none(),
        "untampered shell exited unexpectedly with {:?}; log: {:?}; stdout: {}; stderr: {}",
        exited_early,
        log_text,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "production shell must not write logs to stdout; got: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let content = log_text.expect("shell.log should exist");
    assert!(
        content.contains(needle),
        "expected '{needle}' in shell.log, got:\n{content}"
    );
    content
}

#[test]
fn bare_wavical_build_refuses_and_lists_three_choices() {
    let temp = tempdir().unwrap();
    let proj = create_my_app_fixture(temp.path(), "refuse-app");
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");

    let out = Command::new(wavical_bin)
        .arg("build")
        .arg(&proj)
        .output()
        .expect("run bare wavical build");

    assert!(
        !out.status.success(),
        "wavical build without --allow-open must refuse"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--allow-open")
            && stderr.contains("build-safe")
            && stderr.contains("build-ultra-safe"),
        "refusal message must list all three build choices, got:\n{stderr}"
    );
    assert!(
        !proj.join("dist").exists(),
        "refused build must not create dist/"
    );
}

#[test]
fn f4_open_safe_and_ultra_safe_builds_write_manifest_fields_and_shell_enforces() {
    let temp = tempdir().unwrap();
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let prod_shell = ensure_prod_shell_bin();
    let local_appdata = temp.path().join("localappdata");
    fs::create_dir_all(&local_appdata).unwrap();

    // 1. `wavical build --allow-open`
    let open_proj = create_my_app_fixture(temp.path(), "f4-open-app");
    let open_out = Command::new(wavical_bin)
        .args(["build", "--allow-open"])
        .arg(&open_proj)
        .output()
        .expect("run wavical build --allow-open");
    assert!(
        open_out.status.success(),
        "build --allow-open failed: {}",
        String::from_utf8_lossy(&open_out.stderr)
    );
    let open_manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(open_proj.join("dist").join("wavical-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(open_manifest["mode"], "open");
    assert!(open_manifest.get("allowlistSha256").is_none());
    assert!(open_manifest.get("inlineScriptHashes").is_none());
    spawn_shell_and_wait_for_log(
        prod_shell,
        &open_proj.join("dist"),
        &local_appdata,
        "f4-open-app",
        r#"mode="open""#,
    );

    // 2. `wavical build-safe`
    let safe_proj = create_my_app_fixture(temp.path(), "f4-safe-app");
    let safe_out = Command::new(wavical_bin)
        .arg("build-safe")
        .arg(&safe_proj)
        .output()
        .expect("run wavical build-safe");
    assert!(
        safe_out.status.success(),
        "build-safe failed: {}",
        String::from_utf8_lossy(&safe_out.stderr)
    );
    let safe_dist = safe_proj.join("dist");
    let safe_manifest_raw = fs::read_to_string(safe_dist.join("wavical-manifest.json")).unwrap();
    let safe_manifest: serde_json::Value = serde_json::from_str(&safe_manifest_raw).unwrap();
    assert_eq!(safe_manifest["mode"], "safe");

    let expected_al_bytes = fs::read(safe_proj.join("wavical.allowlist.json")).unwrap();
    let expected_al_sha = hex::encode(Sha256::digest(&expected_al_bytes));
    assert_eq!(safe_manifest["allowlistSha256"], expected_al_sha);
    assert_eq!(
        fs::read(safe_dist.join("wavical.allowlist.json")).unwrap(),
        expected_al_bytes,
        "build-safe must copy exact wavical.allowlist.json into dist/"
    );

    let expected_h1 = hash_inline_script(b"window.__inlineBoot = 1;").as_csp_token();
    let expected_h2 = hash_inline_script(b"export const ready = true;").as_csp_token();
    let page_hashes = safe_manifest["inlineScriptHashes"]["index.html"]
        .as_array()
        .expect("inlineScriptHashes for index.html");
    assert_eq!(page_hashes.len(), 2);
    assert_eq!(page_hashes[0], expected_h1);
    assert_eq!(page_hashes[1], expected_h2);

    // Untampered safe-mode app starts and logs mode="safe"
    spawn_shell_and_wait_for_log(
        prod_shell,
        &safe_dist,
        &local_appdata,
        "f4-safe-app",
        r#"mode="safe""#,
    );

    // Tampered dist/wavical.allowlist.json -> shell refuses to start, exits non-zero, creates NO webview
    let tampered_app_id = "f4-safe-tampered";
    fs::write(
        safe_dist.join("wavical.allowlist.json"),
        br#"{"version":1,"scripts":["https://evil.example.com"]}"#,
    )
    .unwrap();
    let tampered_out = Command::new(prod_shell)
        .arg("--app-id")
        .arg(tampered_app_id)
        .arg("--asset-root")
        .arg(&safe_dist)
        .arg("--single-instance")
        .arg("false")
        .env("LOCALAPPDATA", &local_appdata)
        .env("WAVICAL_HEADLESS_TEST", "1")
        .output()
        .expect("run tampered safe shell");
    assert!(
        !tampered_out.status.success(),
        "tampered safe-mode app must exit non-zero"
    );
    assert!(
        !local_appdata
            .join("wavical")
            .join(tampered_app_id)
            .join("EBWebView")
            .exists(),
        "tampered safe-mode app must not create a WebView2 data directory"
    );
    let tampered_log = fs::read_to_string(
        local_appdata
            .join("Wavical")
            .join("apps")
            .join(tampered_app_id)
            .join("logs")
            .join("shell.log"),
    )
    .expect("tampered shell must write refusal to shell.log");
    assert!(
        tampered_log.contains("allowlist_integrity_refused"),
        "expected allowlist_integrity_refused in shell.log, got:\n{tampered_log}"
    );

    // 3. `wavical build-ultra-safe`
    let ultra_proj = create_my_app_fixture(temp.path(), "f4-ultra-app");
    let src_html_before = fs::read_to_string(ultra_proj.join("src").join("index.html")).unwrap();
    let ultra_out = Command::new(wavical_bin)
        .arg("build-ultra-safe")
        .arg(&ultra_proj)
        .output()
        .expect("run wavical build-ultra-safe");
    assert!(
        ultra_out.status.success(),
        "build-ultra-safe failed: {}",
        String::from_utf8_lossy(&ultra_out.stderr)
    );
    // src/ must not be modified
    assert_eq!(
        fs::read_to_string(ultra_proj.join("src").join("index.html")).unwrap(),
        src_html_before
    );
    let ultra_stderr = String::from_utf8_lossy(&ultra_out.stderr);
    assert!(
        ultra_stderr.contains("onerror") && ultra_stderr.contains("index.html:"),
        "expected inline handler warning with file and line, got:\n{ultra_stderr}"
    );

    let ultra_dist = ultra_proj.join("dist");
    let ultra_manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(ultra_dist.join("wavical-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(ultra_manifest["mode"], "ultra-safe");
    assert!(
        ultra_dist
            .join(".wavical-extracted")
            .join("index_html_inline_0.js")
            .is_file()
    );
    assert!(
        ultra_dist
            .join(".wavical-extracted")
            .join("index_html_inline_1.js")
            .is_file()
    );
    let dist_html_after = fs::read_to_string(ultra_dist.join("index.html")).unwrap();
    assert!(dist_html_after.contains(".wavical-extracted/index_html_inline_0.js"));
    assert!(
        ultra_manifest["assets"]
            .get(".wavical-extracted/index_html_inline_0.js")
            .is_some()
    );
    let actual_dist_html_hash = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(dist_html_after.as_bytes()))
    );
    assert_eq!(
        ultra_manifest["assets"]["index.html"],
        actual_dist_html_hash
    );

    spawn_shell_and_wait_for_log(
        prod_shell,
        &ultra_dist,
        &local_appdata,
        "f4-ultra-app",
        r#"mode="ultra-safe""#,
    );
}

#[test]
fn missing_mode_manifest_logs_exact_line_and_doctor_app_reports_it_with_log_path() {
    let temp = tempdir().unwrap();
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");
    let prod_shell = ensure_prod_shell_bin();
    let local_appdata = temp.path().join("localappdata");
    fs::create_dir_all(&local_appdata).unwrap();

    let dist = temp.path().join("legacy-dist");
    fs::create_dir_all(&dist).unwrap();
    let html = b"<!doctype html><h1>Legacy</h1>";
    fs::write(dist.join("index.html"), html).unwrap();
    let sha = format!("sha256:{}", hex::encode(Sha256::digest(html)));
    fs::write(
        dist.join("wavical-manifest.json"),
        format!(
            r#"{{"appId":"legacy-missing-mode","version":"1.0.0","capabilities":{{}},"assets":{{"index.html":"{sha}"}}}}"#
        ),
    )
    .unwrap();

    // 1. Production shell logs the exact line into shell.log
    spawn_shell_and_wait_for_log(
        prod_shell,
        &dist,
        &local_appdata,
        "legacy-missing-mode",
        "manifest has no mode: app runs in ultra-safe",
    );

    // 2. `wavical doctor --app` reports the exact wording and log file path
    let doc_out = Command::new(wavical_bin)
        .env("LOCALAPPDATA", &local_appdata)
        .env("WAVICAL_SHELL_BIN", prod_shell)
        .args(["doctor", "--app"])
        .arg(&dist)
        .output()
        .expect("run wavical doctor --app");
    assert!(doc_out.status.success());
    let stdout = String::from_utf8_lossy(&doc_out.stdout);
    assert!(
        stdout.contains("manifest has no mode: app runs in ultra-safe"),
        "expected missing-mode line in doctor --app output:\n{stdout}"
    );
    let expected_log = local_appdata
        .join("Wavical")
        .join("apps")
        .join("legacy-missing-mode")
        .join("logs")
        .join("shell.log");
    assert!(
        stdout.contains(&expected_log.display().to_string()),
        "expected log file path '{}' in doctor --app output:\n{stdout}",
        expected_log.display()
    );
    assert!(stdout.contains(ULTRA_SAFE_CSP));
}

#[test]
fn signed_update_payload_built_from_safe_dist_contains_manifest_and_allowlist() {
    let temp = tempdir().unwrap();
    let proj = create_my_app_fixture(temp.path(), "f4-update-app");
    let wavical_bin = env!("CARGO_BIN_EXE_wavical");

    let build_out = Command::new(wavical_bin)
        .arg("build-safe")
        .arg(&proj)
        .output()
        .expect("run wavical build-safe");
    assert!(
        build_out.status.success(),
        "build-safe failed: {}",
        String::from_utf8_lossy(&build_out.stderr)
    );
    let dist_dir = proj.join("dist");

    // Package dist/ into update payload zip
    let payload_zip = temp.path().join("f4-update-app-1.2.0.zip");
    {
        let file = fs::File::create(&payload_zip).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        for entry in fs::read_dir(&dist_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_str().unwrap();
                zip.start_file(name, opts).unwrap();
                zip.write_all(&fs::read(&path).unwrap()).unwrap();
            }
        }
        zip.finish().unwrap();
    }

    // Build and sign Phase 4 release manifest outside repo
    let key_path = temp.path().join("release.key");
    let pubkey = generate_signing_key(&key_path).unwrap();
    let notes_path = temp.path().join("notes.txt");
    fs::write(&notes_path, "Safe-mode release").unwrap();
    let release_manifest_path = temp.path().join("manifest.json");

    build_release_manifest(
        &payload_zip,
        "1.2.0",
        "stable",
        7,
        100,
        &notes_path,
        "f4-update-app",
        "https://updates.example.com/f4-update-app-1.2.0.zip",
        Some("1.0.0"),
        &release_manifest_path,
    )
    .unwrap();
    sign_manifest(&key_path, &release_manifest_path).unwrap();
    let verified = verify_manifest_signature(&pubkey, &release_manifest_path, None).unwrap();
    let zip_sha = hex::encode(Sha256::digest(fs::read(&payload_zip).unwrap()));
    assert_eq!(verified.payload_sha256, zip_sha);

    // Verify signed zip archive contains both wavical-manifest.json and wavical.allowlist.json
    let archive_file = fs::File::open(&payload_zip).unwrap();
    let mut archive = zip::ZipArchive::new(archive_file).unwrap();
    let mut has_manifest = false;
    let mut has_allowlist = false;
    let mut manifest_bytes = Vec::new();
    let mut allowlist_bytes = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let entry_name = entry.name().unwrap().into_owned();
        if entry_name == "wavical-manifest.json" {
            has_manifest = true;
            std::io::Read::read_to_end(&mut entry, &mut manifest_bytes).unwrap();
        } else if entry_name == "wavical.allowlist.json" {
            has_allowlist = true;
            std::io::Read::read_to_end(&mut entry, &mut allowlist_bytes).unwrap();
        }
    }
    assert!(
        has_manifest,
        "update payload must contain wavical-manifest.json"
    );
    assert!(
        has_allowlist,
        "update payload must contain wavical.allowlist.json"
    );

    let parsed_manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(parsed_manifest["mode"], "safe");
    assert_eq!(
        parsed_manifest["allowlistSha256"],
        hex::encode(Sha256::digest(&allowlist_bytes))
    );
    assert!(parsed_manifest["inlineScriptHashes"]["index.html"].is_array());
}
