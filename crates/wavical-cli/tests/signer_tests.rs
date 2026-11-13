#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use wavical_cli::signer::{
    SignerError, build_release_manifest, decode_verifying_key, encode_base64, generate_signing_key,
    sign_manifest, validate_private_key_output_path, verify_manifest_signature,
};

fn unique_temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("wavical_wp7_{name}_{nanos}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn test_keygen_refuses_repo_and_project_paths() {
    let base = unique_temp_dir("keygen_refuse");

    // 1. Inside a git repo
    let repo_dir = base.join("repo");
    fs::create_dir_all(repo_dir.join(".git")).expect("create .git");
    let inside_repo = repo_dir.join("keys").join("release.key");
    let err = generate_signing_key(&inside_repo).expect_err("must refuse key inside .git repo");
    assert!(matches!(err, SignerError::UnsafeKeyLocation(_)));
    assert!(!inside_repo.exists());

    // 2. Inside a Wavical project directory
    let proj_dir = base.join("proj");
    fs::create_dir_all(&proj_dir).expect("create proj");
    fs::write(proj_dir.join("wavical.config.js"), "module.exports = {};\n").expect("write cfg");
    let inside_proj = proj_dir.join("release.key");
    let err =
        validate_private_key_output_path(&inside_proj).expect_err("must refuse key inside project");
    assert!(matches!(err, SignerError::UnsafeKeyLocation(_)));

    // 3. Outside any repo/project succeeds and returns only valid pubkey
    let safe_key = base.join("outside").join("release.key");
    let pubkey = generate_signing_key(&safe_key).expect("keygen outside repo must succeed");
    assert!(safe_key.is_file());
    assert!(decode_verifying_key(&pubkey).is_ok());
    let key_contents = fs::read_to_string(&safe_key).expect("read key");
    assert!(!key_contents.trim().is_empty());
    assert_ne!(key_contents.trim(), pubkey.trim());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn test_manifest_sign_verify_and_marker_key_never_leaks() {
    let base = unique_temp_dir("sign_verify");

    // Generate a marker private key at test time (never committed)
    let marker_secret = [0x5A_u8; 32];
    let marker_b64 = encode_base64(&marker_secret);
    let key_path = base.join("marker.key");
    fs::write(&key_path, format!("{marker_b64}\n")).expect("write marker key");

    let signing_key = ed25519_dalek::SigningKey::from_bytes(&marker_secret);
    let pubkey_b64 = encode_base64(&signing_key.verifying_key().to_bytes());

    let payload_path = base.join("app-1.2.0.zip");
    fs::write(&payload_path, b"PK\x03\x04dummy-payload-bytes").expect("write payload");
    let notes_path = base.join("notes.txt");
    fs::write(&notes_path, "Bug fixes and security hardening.").expect("write notes");
    let manifest_path = base.join("manifest.json");

    build_release_manifest(
        &payload_path,
        "1.2.0",
        "stable",
        7,
        50,
        &notes_path,
        "com.example.app",
        "https://updates.example.com/app-1.2.0.zip",
        Some("1.0.0"),
        &manifest_path,
    )
    .expect("build manifest");

    let sig_path = sign_manifest(&key_path, &manifest_path).expect("sign manifest");
    assert!(sig_path.is_file());

    let verified = verify_manifest_signature(&pubkey_b64, &manifest_path, None)
        .expect("verify valid manifest");
    assert_eq!(verified.version, "1.2.0");
    assert_eq!(verified.rollout_percent, 50);

    // Marker private key must appear nowhere outside key_path
    let manifest_text = fs::read_to_string(&manifest_path).expect("read manifest");
    let sig_text = fs::read_to_string(&sig_path).expect("read sig");
    assert!(!manifest_text.contains(&marker_b64));
    assert!(!sig_text.contains(&marker_b64));
    assert!(!pubkey_b64.contains(&marker_b64));

    // Tamper one byte of manifest -> verification must fail without leaking key
    let mut tampered = fs::read(&manifest_path).expect("read manifest bytes");
    tampered[2] = b'X';
    fs::write(&manifest_path, &tampered).expect("write tampered manifest");
    let err = verify_manifest_signature(&pubkey_b64, &manifest_path, None)
        .expect_err("tampered manifest must fail verification");
    assert!(!err.to_string().contains(&marker_b64));

    // Expiry beyond 30 days must fail
    let err = build_release_manifest(
        &payload_path,
        "1.2.0",
        "stable",
        31,
        100,
        &notes_path,
        "com.example.app",
        "https://updates.example.com/app-1.2.0.zip",
        None,
        &base.join("bad_manifest.json"),
    )
    .expect_err("expiry > 30 days must fail");
    assert!(matches!(err, SignerError::InvalidManifestParam(_)));

    let _ = fs::remove_dir_all(&base);
}
