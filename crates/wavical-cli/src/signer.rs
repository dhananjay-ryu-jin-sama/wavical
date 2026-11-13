#![forbid(unsafe_code)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{
    PUBLIC_KEY_LENGTH, SECRET_KEY_LENGTH, SIGNATURE_LENGTH, Signature, Signer, SigningKey,
    VerifyingKey,
};
use serde_json;
use thiserror::Error;
use wavical_ipc::{
    MAX_UPDATE_MANIFEST_BYTES, MAX_UPDATE_MANIFEST_VALIDITY_SECS, MAX_UPDATE_PAYLOAD_BYTES,
    MAX_UPDATE_RELEASE_NOTES_BYTES, UpdateManifest, UpdateSemver, parse_update_manifest,
};

use crate::sha256::sha256_file;

/// Errors returned by `wavical signer` and `wavical release` commands.
#[derive(Debug, Error)]
pub enum SignerError {
    #[error("refusing to write private key inside project or repository directory: {0}")]
    UnsafeKeyLocation(PathBuf),

    #[error("private key file already exists at {0}; remove it first or choose a new path")]
    KeyAlreadyExists(PathBuf),

    #[error("I/O error at '{path}': {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to generate cryptographic randomness: {0}")]
    RngFailed(String),

    #[error("invalid key format")]
    InvalidKeyFormat,

    #[error("invalid signature format")]
    InvalidSignatureFormat,

    #[error("signature verification failed")]
    SignatureInvalid,

    #[error("invalid manifest parameter: {0}")]
    InvalidManifestParam(String),

    #[error("manifest validation failed: {0}")]
    ManifestValidation(String),
}

const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode raw bytes as standard Base64 with `=` padding.
pub fn encode_base64(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() {
            input[i + 1] as u32
        } else {
            0
        };
        let b2 = if i + 2 < input.len() {
            input[i + 2] as u32
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(B64_TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64_TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < input.len() {
            out.push(B64_TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < input.len() {
            out.push(B64_TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

/// Decode standard Base64 (ignoring surrounding ASCII whitespace).
pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.is_empty() || !clean.len().is_multiple_of(4) {
        return None;
    }

    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    for chunk in clean.as_chunks::<4>().0 {
        let mut vals = [0u8; 4];
        let mut pad = 0usize;
        for (idx, &ch) in chunk.iter().enumerate() {
            if ch == b'=' {
                if idx < 2 || (idx == 2 && chunk[3] != b'=') {
                    return None;
                }
                pad += 1;
                vals[idx] = 0;
            } else if pad > 0 {
                return None;
            } else {
                vals[idx] = match ch {
                    b'A'..=b'Z' => ch - b'A',
                    b'a'..=b'z' => ch - b'a' + 26,
                    b'0'..=b'9' => ch - b'0' + 52,
                    b'+' => 62,
                    b'/' => 63,
                    _ => return None,
                };
            }
        }
        let triple = ((vals[0] as u32) << 18)
            | ((vals[1] as u32) << 12)
            | ((vals[2] as u32) << 6)
            | (vals[3] as u32);
        out.push(((triple >> 16) & 0xFF) as u8);
        if pad < 2 {
            out.push(((triple >> 8) & 0xFF) as u8);
        }
        if pad == 0 {
            out.push((triple & 0xFF) as u8);
        }
    }
    Some(out)
}

fn normalize_for_ancestry(path: &Path) -> Result<PathBuf, SignerError> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| SignerError::Io {
                path: path.to_path_buf(),
                source: e,
            })?
            .join(path)
    };

    // Resolve existing prefix if possible (handles symlinks / 8.3 paths on Windows)
    let mut existing_prefix = abs.clone();
    let mut suffix_parts = Vec::new();
    while !existing_prefix.exists() {
        if let Some(name) = existing_prefix.file_name() {
            suffix_parts.push(name.to_os_string());
        } else {
            break;
        }
        if !existing_prefix.pop() {
            break;
        }
    }

    let mut resolved = if existing_prefix.exists() {
        fs::canonicalize(&existing_prefix).unwrap_or(existing_prefix)
    } else {
        abs
    };
    for part in suffix_parts.into_iter().rev() {
        resolved.push(part);
    }

    let mut normalized = PathBuf::new();
    for comp in resolved.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    Ok(normalized)
}

/// Ensure `out_path` does not reside inside any git repository or Wavical project directory.
pub fn validate_private_key_output_path(out_path: &Path) -> Result<PathBuf, SignerError> {
    let normalized = normalize_for_ancestry(out_path)?;
    let mut cursor = normalized.parent();
    while let Some(dir) = cursor {
        if dir.join(".git").exists()
            || dir.join("wavical.config.js").is_file()
            || dir.join("wavical.config.json").is_file()
        {
            return Err(SignerError::UnsafeKeyLocation(normalized));
        }
        cursor = dir.parent();
    }
    Ok(normalized)
}

/// Generate a new Ed25519 signing keypair, write the private key to `out_path`
/// (refusing any path inside a project or git repository), and return the
/// base64-encoded public key.
///
/// File permissions:
/// - On Unix, permissions are restricted to `0o600` (owner read/write only).
/// - On Windows, Rust's safe `std` API (under `#![forbid(unsafe_code)]`) does
///   not expose NTFS DACL manipulation, so users should store private keys in
///   a user-private directory (e.g. `%USERPROFILE%\.wavical\keys\`).
pub fn generate_signing_key(out_path: &Path) -> Result<String, SignerError> {
    let target = validate_private_key_output_path(out_path)?;
    if target.exists() {
        return Err(SignerError::KeyAlreadyExists(target));
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| SignerError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    let mut secret_bytes = [0u8; SECRET_KEY_LENGTH];
    getrandom::fill(&mut secret_bytes).map_err(|e| SignerError::RngFailed(e.to_string()))?;

    let signing_key = SigningKey::from_bytes(&secret_bytes);
    let verifying_key = signing_key.verifying_key();
    let pubkey_b64 = encode_base64(&verifying_key.to_bytes());
    let privkey_b64 = encode_base64(&signing_key.to_bytes());

    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let mut file = opts.open(&target).map_err(|e| SignerError::Io {
        path: target.clone(),
        source: e,
    })?;
    writeln!(file, "{privkey_b64}").map_err(|e| SignerError::Io {
        path: target.clone(),
        source: e,
    })?;
    file.flush().map_err(|e| SignerError::Io {
        path: target,
        source: e,
    })?;

    Ok(pubkey_b64)
}

fn load_signing_key(key_path: &Path) -> Result<SigningKey, SignerError> {
    let raw = fs::read_to_string(key_path).map_err(|e| SignerError::Io {
        path: key_path.to_path_buf(),
        source: e,
    })?;
    let trimmed = raw.trim();
    let key_line = trimmed
        .lines()
        .find(|line| {
            let t = line.trim();
            !t.is_empty() && !t.starts_with("untrusted comment:")
        })
        .ok_or(SignerError::InvalidKeyFormat)?;
    let decoded = decode_base64(key_line).ok_or(SignerError::InvalidKeyFormat)?;
    let arr: [u8; SECRET_KEY_LENGTH] = decoded
        .try_into()
        .map_err(|_| SignerError::InvalidKeyFormat)?;
    Ok(SigningKey::from_bytes(&arr))
}

/// Decode a base64-encoded Ed25519 public key (32-byte raw or 42-byte Minisign payload).
pub fn decode_verifying_key(pubkey_str: &str) -> Result<VerifyingKey, SignerError> {
    let key_line = pubkey_str
        .trim()
        .lines()
        .find(|line| {
            let t = line.trim();
            !t.is_empty() && !t.starts_with("untrusted comment:")
        })
        .ok_or(SignerError::InvalidKeyFormat)?;
    let decoded = decode_base64(key_line).ok_or(SignerError::InvalidKeyFormat)?;
    let key_bytes: [u8; PUBLIC_KEY_LENGTH] = if decoded.len() == PUBLIC_KEY_LENGTH {
        decoded
            .try_into()
            .map_err(|_| SignerError::InvalidKeyFormat)?
    } else if decoded.len() == 42 && &decoded[..2] == b"Ed" {
        decoded[10..42]
            .try_into()
            .map_err(|_| SignerError::InvalidKeyFormat)?
    } else {
        return Err(SignerError::InvalidKeyFormat);
    };
    VerifyingKey::from_bytes(&key_bytes).map_err(|_| SignerError::InvalidKeyFormat)
}

/// Build an unsigned update manifest JSON file for a release payload.
#[allow(clippy::too_many_arguments)]
pub fn build_release_manifest(
    payload_path: &Path,
    version: &str,
    channel: &str,
    expires_days: u64,
    rollout_percent: u8,
    notes_path: &Path,
    app_id: &str,
    payload_url: &str,
    min_version: Option<&str>,
    out_path: &Path,
) -> Result<(), SignerError> {
    UpdateSemver::parse(version)
        .map_err(|e| SignerError::InvalidManifestParam(format!("version: {e}")))?;

    let min_ver = min_version.unwrap_or("0.0.0");
    UpdateSemver::parse(min_ver)
        .map_err(|e| SignerError::InvalidManifestParam(format!("minimumSupportedVersion: {e}")))?;

    if expires_days == 0 {
        return Err(SignerError::InvalidManifestParam(
            "--expires must be at least 1 day".to_string(),
        ));
    }
    let validity_secs = expires_days.saturating_mul(86_400);
    if validity_secs > MAX_UPDATE_MANIFEST_VALIDITY_SECS {
        return Err(SignerError::InvalidManifestParam(format!(
            "--expires exceeds maximum validity of {} days",
            MAX_UPDATE_MANIFEST_VALIDITY_SECS / 86_400
        )));
    }

    if rollout_percent > 100 {
        return Err(SignerError::InvalidManifestParam(
            "--rollout must be between 0 and 100".to_string(),
        ));
    }

    let meta = fs::metadata(payload_path).map_err(|e| SignerError::Io {
        path: payload_path.to_path_buf(),
        source: e,
    })?;
    let payload_size = meta.len();
    if payload_size == 0 || payload_size > MAX_UPDATE_PAYLOAD_BYTES {
        return Err(SignerError::InvalidManifestParam(format!(
            "payload size {payload_size} bytes outside allowed 1..={MAX_UPDATE_PAYLOAD_BYTES} range"
        )));
    }

    let payload_sha256 = sha256_file(payload_path).map_err(|e| SignerError::Io {
        path: payload_path.to_path_buf(),
        source: e,
    })?;

    let notes_raw = fs::read_to_string(notes_path).map_err(|e| SignerError::Io {
        path: notes_path.to_path_buf(),
        source: e,
    })?;
    if notes_raw.len() > MAX_UPDATE_RELEASE_NOTES_BYTES {
        return Err(SignerError::InvalidManifestParam(format!(
            "release notes exceed {MAX_UPDATE_RELEASE_NOTES_BYTES} bytes"
        )));
    }

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| SignerError::InvalidManifestParam(format!("system clock error: {e}")))?
        .as_secs();

    let manifest = UpdateManifest {
        app_id: app_id.to_string(),
        channel: channel.to_string(),
        version: version.to_string(),
        minimum_supported_version: min_ver.to_string(),
        payload_url: payload_url.to_string(),
        payload_size,
        payload_sha256,
        release_notes: notes_raw,
        expires: now_unix.saturating_add(validity_secs),
        rollout_percent,
    };

    let json_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| SignerError::ManifestValidation(e.to_string()))?;
    if json_bytes.len() > MAX_UPDATE_MANIFEST_BYTES {
        return Err(SignerError::ManifestValidation(format!(
            "serialized manifest exceeds {MAX_UPDATE_MANIFEST_BYTES} bytes"
        )));
    }
    parse_update_manifest(&json_bytes)
        .map_err(|e| SignerError::ManifestValidation(e.to_string()))?;

    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| SignerError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    fs::write(out_path, &json_bytes).map_err(|e| SignerError::Io {
        path: out_path.to_path_buf(),
        source: e,
    })?;

    Ok(())
}

/// Sign the exact bytes of `manifest_path` using the Ed25519 private key at `key_path`,
/// writing `<manifest_path>.sig`.
pub fn sign_manifest(key_path: &Path, manifest_path: &Path) -> Result<PathBuf, SignerError> {
    let manifest_bytes = fs::read(manifest_path).map_err(|e| SignerError::Io {
        path: manifest_path.to_path_buf(),
        source: e,
    })?;
    if manifest_bytes.len() > MAX_UPDATE_MANIFEST_BYTES {
        return Err(SignerError::ManifestValidation(format!(
            "manifest exceeds {MAX_UPDATE_MANIFEST_BYTES} bytes"
        )));
    }
    parse_update_manifest(&manifest_bytes)
        .map_err(|e| SignerError::ManifestValidation(e.to_string()))?;

    let signing_key = load_signing_key(key_path)?;
    let sig = signing_key.sign(&manifest_bytes);
    let sig_b64 = encode_base64(&sig.to_bytes());

    let sig_path = PathBuf::from(format!("{}.sig", manifest_path.display()));
    fs::write(&sig_path, format!("{sig_b64}\n")).map_err(|e| SignerError::Io {
        path: sig_path.clone(),
        source: e,
    })?;
    Ok(sig_path)
}

/// Verify `<manifest_path>` against `<manifest_path>.sig` (or `sig_path` override)
/// using `pubkey_str`, and validate the manifest schema.
pub fn verify_manifest_signature(
    pubkey_str: &str,
    manifest_path: &Path,
    sig_path: Option<&Path>,
) -> Result<UpdateManifest, SignerError> {
    let manifest_bytes = fs::read(manifest_path).map_err(|e| SignerError::Io {
        path: manifest_path.to_path_buf(),
        source: e,
    })?;
    let default_sig = PathBuf::from(format!("{}.sig", manifest_path.display()));
    let sig_file = sig_path.unwrap_or(&default_sig);
    let sig_raw = fs::read(sig_file).map_err(|e| SignerError::Io {
        path: sig_file.to_path_buf(),
        source: e,
    })?;

    let sig_bytes = if sig_raw.len() == SIGNATURE_LENGTH {
        sig_raw
    } else {
        let sig_text =
            std::str::from_utf8(&sig_raw).map_err(|_| SignerError::InvalidSignatureFormat)?;
        decode_base64(sig_text).ok_or(SignerError::InvalidSignatureFormat)?
    };
    let sig_arr: [u8; SIGNATURE_LENGTH] = sig_bytes
        .try_into()
        .map_err(|_| SignerError::InvalidSignatureFormat)?;
    let signature = Signature::from_bytes(&sig_arr);

    let verifying_key = decode_verifying_key(pubkey_str)?;
    verifying_key
        .verify_strict(&manifest_bytes, &signature)
        .map_err(|_| SignerError::SignatureInvalid)?;

    parse_update_manifest(&manifest_bytes)
        .map_err(|e| SignerError::ManifestValidation(e.to_string()))
}
