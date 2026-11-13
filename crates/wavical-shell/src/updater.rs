#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH, Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use wavical_ai::downloader::{
    DownloadError, DownloadKind, DownloadOptions, download_bounded_bytes, download_verified_file,
};
use wavical_ipc::{
    MAX_UPDATE_MANIFEST_BYTES, UpdateHealthState, UpdateManifest, UpdateManifestError,
    parse_update_manifest_with_options, validate_manifest_for_install,
};
use wavical_protocol::validate_archive_entry_path;

/// Maximum size of a `.sig` file downloaded from an update server (1 KB).
pub const MAX_UPDATE_SIGNATURE_BYTES: usize = 1024;

/// Maximum extracted payload bytes for an update archive (1 GB, Plan 1.10).
pub const MAX_UPDATE_EXTRACT_BYTES: u64 = 1024 * 1024 * 1024;

/// Maximum extracted files for an update archive (20,000 files, Plan 1.10).
pub const MAX_UPDATE_EXTRACT_FILES: usize = 20_000;

/// Maximum single file size inside an update archive (1 GB).
pub const MAX_UPDATE_SINGLE_FILE_BYTES: u64 = 1024 * 1024 * 1024;

/// Default health-check window in seconds after launching a newly applied update.
pub const UPDATE_HEALTH_CHECK_WINDOW_SECS: u64 = 30;

/// Errors returned by the in-app updater (never exposed verbatim to the page).
#[derive(Debug, Error)]
pub enum UpdaterError {
    #[error("updater is not configured")]
    NotConfigured,

    #[error("download failed: {0}")]
    Download(#[from] DownloadError),

    #[error("manifest validation failed: {0}")]
    Manifest(#[from] UpdateManifestError),

    #[error("invalid embedded public key")]
    InvalidPublicKey,

    #[error("invalid update signature format")]
    InvalidSignatureFormat,

    #[error("update signature verification failed")]
    SignatureInvalid,

    #[error("update not eligible for rollout (bucket {bucket} >= {rollout_percent}%)")]
    NotInRollout { bucket: u8, rollout_percent: u8 },

    #[error("archive extraction failed: {0}")]
    ArchiveExtract(String),

    #[error("no verified pending update is ready to apply")]
    NoPendingUpdate,

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Safe subset of update check metadata returned to the page (`updater.check`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResponse {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Metadata persisted when an update has been verified and staged in `versions/<version>/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdateRecord {
    pub version: String,
    pub release_notes: String,
    pub payload_sha256: String,
}

/// Result of `prepare_launch` describing which version should start and whether rollback occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTarget {
    pub version: String,
    pub version_dir: PathBuf,
    pub rolled_back_from: Option<String>,
}

/// Options controlling `check_and_stage_update` (allows test overrides for time and download hosts).
#[derive(Debug, Clone)]
pub struct UpdateCheckOptions {
    pub app_id: String,
    pub installed_version: String,
    pub channel: String,
    pub manifest_url: String,
    pub public_keys: Vec<String>,
    pub download_options: DownloadOptions,
    pub now_unix: Option<u64>,
}

/// Atomically write `contents` to `target` using a sibling `.tmp` file and `fs::rename`.
fn atomic_write_bytes(target: &Path, contents: &[u8]) -> Result<(), UpdaterError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_name = match target.file_name() {
        Some(n) => format!("{}.tmp", n.to_string_lossy()),
        None => "state.tmp".to_string(),
    };
    let tmp_path = target.with_file_name(tmp_name);
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, target)?;
    Ok(())
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
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

fn decode_verifying_key(pubkey_str: &str) -> Result<VerifyingKey, UpdaterError> {
    let key_line = pubkey_str
        .trim()
        .lines()
        .find(|line| {
            let t = line.trim();
            !t.is_empty() && !t.starts_with("untrusted comment:")
        })
        .ok_or(UpdaterError::InvalidPublicKey)?;
    let decoded = decode_base64(key_line).ok_or(UpdaterError::InvalidPublicKey)?;
    let key_bytes: [u8; PUBLIC_KEY_LENGTH] = if decoded.len() == PUBLIC_KEY_LENGTH {
        decoded
            .try_into()
            .map_err(|_| UpdaterError::InvalidPublicKey)?
    } else if decoded.len() == 42 && &decoded[..2] == b"Ed" {
        decoded[10..42]
            .try_into()
            .map_err(|_| UpdaterError::InvalidPublicKey)?
    } else {
        return Err(UpdaterError::InvalidPublicKey);
    };
    VerifyingKey::from_bytes(&key_bytes).map_err(|_| UpdaterError::InvalidPublicKey)
}

fn decode_signature(sig_raw: &[u8]) -> Result<Signature, UpdaterError> {
    let sig_bytes = if sig_raw.len() == SIGNATURE_LENGTH {
        sig_raw.to_vec()
    } else {
        let sig_text =
            std::str::from_utf8(sig_raw).map_err(|_| UpdaterError::InvalidSignatureFormat)?;
        decode_base64(sig_text).ok_or(UpdaterError::InvalidSignatureFormat)?
    };
    let arr: [u8; SIGNATURE_LENGTH] = sig_bytes
        .try_into()
        .map_err(|_| UpdaterError::InvalidSignatureFormat)?;
    Ok(Signature::from_bytes(&arr))
}

/// Verify `manifest_bytes` against `sig_raw` using up to 2 embedded public keys (supports key rotation).
pub fn verify_manifest_signature_with_keys(
    manifest_bytes: &[u8],
    sig_raw: &[u8],
    public_keys: &[String],
) -> Result<(), UpdaterError> {
    if public_keys.is_empty() {
        return Err(UpdaterError::InvalidPublicKey);
    }
    let signature = decode_signature(sig_raw)?;
    for key_str in public_keys {
        if let Ok(vk) = decode_verifying_key(key_str)
            && vk.verify_strict(manifest_bytes, &signature).is_ok()
        {
            return Ok(());
        }
    }
    Err(UpdaterError::SignatureInvalid)
}

/// Load or generate a persistent random rollout bucket (0..=99) stored once per install.
/// Never transmitted over the network.
pub fn load_or_create_rollout_bucket(updater_dir: &Path) -> Result<u8, UpdaterError> {
    let bucket_path = updater_dir.join("rollout-bucket");
    if let Ok(raw) = fs::read_to_string(&bucket_path)
        && let Ok(val) = raw.trim().parse::<u8>()
        && val < 100
    {
        return Ok(val);
    }

    // Rejection sampling on [0, 200) for uniform 0..=99 distribution.
    let mut byte = [0u8; 1];
    let bucket = loop {
        getrandom::fill(&mut byte)
            .map_err(|e| UpdaterError::Io(io::Error::other(e.to_string())))?;
        if byte[0] < 200 {
            break byte[0] % 100;
        }
    };
    atomic_write_bytes(&bucket_path, format!("{bucket}\n").as_bytes())?;
    Ok(bucket)
}

fn validate_safe_version_dir_name(version: &str) -> Result<(), UpdaterError> {
    if version.is_empty()
        || version.contains('/')
        || version.contains('\\')
        || version.contains("..")
    {
        return Err(UpdaterError::ArchiveExtract(
            "invalid version directory name".to_string(),
        ));
    }
    Ok(())
}

fn is_zip_file(path: &Path) -> bool {
    let mut buf = [0u8; 4];
    if let Ok(mut f) = File::open(path)
        && f.read_exact(&mut buf).is_ok()
    {
        return &buf == b"PK\x03\x04" || &buf == b"PK\x05\x06";
    }
    false
}

/// Safely extract a zip update payload into `dest_dir`, enforcing Plan 1.10 limits,
/// path traversal checks, and symlink rejection.
fn extract_update_zip_safely(archive_path: &Path, dest_dir: &Path) -> Result<(), UpdaterError> {
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| UpdaterError::ArchiveExtract(format!("invalid zip archive: {e}")))?;

    if archive.len() > MAX_UPDATE_EXTRACT_FILES {
        return Err(UpdaterError::ArchiveExtract(format!(
            "archive exceeds maximum file count ({MAX_UPDATE_EXTRACT_FILES})"
        )));
    }

    fs::create_dir_all(dest_dir)?;
    let canonical_dest = fs::canonicalize(dest_dir)?;

    let mut total_bytes: u64 = 0;
    let mut file_count: usize = 0;
    let mut seen_entries_lower = HashSet::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| UpdaterError::ArchiveExtract(format!("zip entry error: {e}")))?;

        let raw_name = entry
            .name()
            .map_err(|e| UpdaterError::ArchiveExtract(format!("invalid zip entry name: {e}")))?
            .into_owned();
        let is_symlink = entry
            .unix_mode()
            .is_some_and(|mode| (mode & 0o170000) == 0o120000);

        let validated_rel =
            validate_archive_entry_path(&raw_name, is_symlink, false, &mut seen_entries_lower)
                .map_err(|e| UpdaterError::ArchiveExtract(e.to_string()))?;

        let mut target_path = canonical_dest.clone();
        for comp in validated_rel.components() {
            if let Component::Normal(part) = comp {
                target_path.push(part);
            } else {
                return Err(UpdaterError::ArchiveExtract(
                    "invalid path component in archive".to_string(),
                ));
            }
        }

        if entry.is_dir() {
            fs::create_dir_all(&target_path)?;
            continue;
        }

        file_count = file_count.saturating_add(1);
        if file_count > MAX_UPDATE_EXTRACT_FILES {
            return Err(UpdaterError::ArchiveExtract(
                "archive file count limit exceeded".to_string(),
            ));
        }

        if entry.size() > MAX_UPDATE_SINGLE_FILE_BYTES {
            return Err(UpdaterError::ArchiveExtract(
                "archive single file size limit exceeded".to_string(),
            ));
        }

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut out_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target_path)?;
        let mut buf = [0u8; 32 * 1024];
        let mut entry_written: u64 = 0;
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            entry_written = entry_written.saturating_add(n as u64);
            total_bytes = total_bytes.saturating_add(n as u64);
            if entry_written > MAX_UPDATE_SINGLE_FILE_BYTES
                || total_bytes > MAX_UPDATE_EXTRACT_BYTES
            {
                return Err(UpdaterError::ArchiveExtract(
                    "archive uncompressed size limit exceeded".to_string(),
                ));
            }
            out_file.write_all(&buf[..n])?;
        }
        out_file.sync_all()?;
    }

    Ok(())
}

/// Perform a full Plan 1.6 update check and stage the verified update into `versions/<new>/`.
///
/// Check order (all required):
/// 1. Download `manifest.json` and `manifest.json.sig` via shared downloader (`DownloadKind::Update`).
/// 2. Verify Ed25519 signature over exact manifest bytes with embedded public keys.
/// 3. Parse manifest (`deny_unknown_fields`, <= 64 KB) and verify `appId`, `channel`,
///    `version > installed && version >= minimumSupportedVersion`, not expired & validity <= 30d.
/// 4. Check rollout bucket (`bucket < rolloutPercent`).
/// 5. Download payload via shared downloader (`download_verified_file`: size + SHA-256).
/// 6. Stage into `versions/<new>.staged/` (safe zip extractor if zip, else copy), rename to
///    `versions/<new>/`, and write `pending_update.json`.
///
/// Any failure cleans up staged artifacts, leaves the installed version untouched, and logs one event.
pub fn check_and_stage_update(
    updater_dir: &Path,
    opts: &UpdateCheckOptions,
) -> Result<UpdateCheckResponse, UpdaterError> {
    let result = check_and_stage_update_inner(updater_dir, opts);
    if let Err(ref err) = result {
        tracing::warn!(
            target: "wavical.updater",
            event = "updater.check_failed",
            app_id = %opts.app_id,
            channel = %opts.channel,
            reason = %err,
            "update check or staging failed; installed version untouched"
        );
    }
    result
}

fn check_and_stage_update_inner(
    updater_dir: &Path,
    opts: &UpdateCheckOptions,
) -> Result<UpdateCheckResponse, UpdaterError> {
    if opts.manifest_url.trim().is_empty() || opts.public_keys.is_empty() {
        return Err(UpdaterError::NotConfigured);
    }

    let sig_url = format!("{}.sig", opts.manifest_url);

    // 1. Download manifest and signature through shared downloader (allowlist + HTTPS + redirects)
    let manifest_bytes = download_bounded_bytes(
        &opts.manifest_url,
        MAX_UPDATE_MANIFEST_BYTES,
        DownloadKind::Update,
        &opts.download_options,
    )?;
    let sig_bytes = download_bounded_bytes(
        &sig_url,
        MAX_UPDATE_SIGNATURE_BYTES,
        DownloadKind::Update,
        &opts.download_options,
    )?;

    // 2. Signature check over exact manifest bytes with embedded keys
    verify_manifest_signature_with_keys(&manifest_bytes, &sig_bytes, &opts.public_keys)?;

    // 3. Pure schema & policy validation (appId, channel, version > installed, expiry <= 30d, rollout)
    let manifest: UpdateManifest = parse_update_manifest_with_options(
        &manifest_bytes,
        opts.download_options.allow_test_localhost,
    )?;
    let now_unix = match opts.now_unix {
        Some(t) => t,
        None => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| UpdaterError::Io(io::Error::other(e.to_string())))?
            .as_secs(),
    };

    // 4. Rollout bucket (generated once per install, never sent anywhere)
    let bucket = load_or_create_rollout_bucket(updater_dir)?;

    if let Err(err) = validate_manifest_for_install(
        &manifest,
        &opts.app_id,
        &opts.channel,
        &opts.installed_version,
        bucket,
        now_unix,
    ) {
        return match err {
            UpdateManifestError::NotNewerVersion { .. }
            | UpdateManifestError::RolloutBucketExcluded { .. } => Ok(UpdateCheckResponse {
                available: false,
                version: None,
                notes: None,
            }),
            other => Err(UpdaterError::Manifest(other)),
        };
    }

    validate_safe_version_dir_name(&manifest.version)?;
    let versions_dir = updater_dir.join("versions");
    let final_version_dir = versions_dir.join(&manifest.version);
    let staged_version_dir = versions_dir.join(format!("{}.staged", manifest.version));
    let downloads_dir = updater_dir.join("downloads");
    fs::create_dir_all(&downloads_dir)?;
    let payload_path = downloads_dir.join(format!("{}.payload", manifest.version));

    // 5. Download payload + verify size & SHA-256 through shared downloader
    if let Err(err) = download_verified_file(
        &manifest.payload_url,
        &payload_path,
        manifest.payload_size,
        &manifest.payload_sha256,
        DownloadKind::Update,
        &opts.download_options,
        None,
    ) {
        let _ = fs::remove_file(&payload_path);
        return Err(UpdaterError::Download(err));
    }

    // 6. Stage into versions/<new>.staged/ then atomically rename to versions/<new>/
    if staged_version_dir.exists() {
        let _ = fs::remove_dir_all(&staged_version_dir);
    }

    let stage_result = if is_zip_file(&payload_path) {
        extract_update_zip_safely(&payload_path, &staged_version_dir)
    } else {
        fs::create_dir_all(&staged_version_dir)?;
        fs::copy(&payload_path, staged_version_dir.join("payload.bin"))
            .map(|_| ())
            .map_err(UpdaterError::Io)
    };

    if let Err(err) = stage_result {
        let _ = fs::remove_dir_all(&staged_version_dir);
        let _ = fs::remove_file(&payload_path);
        return Err(err);
    }

    if final_version_dir.exists() {
        let _ = fs::remove_dir_all(&final_version_dir);
    }
    if let Err(err) = fs::rename(&staged_version_dir, &final_version_dir) {
        let _ = fs::remove_dir_all(&staged_version_dir);
        let _ = fs::remove_file(&payload_path);
        return Err(UpdaterError::Io(err));
    }
    let _ = fs::remove_file(&payload_path);

    let pending = PendingUpdateRecord {
        version: manifest.version.clone(),
        release_notes: manifest.release_notes.clone(),
        payload_sha256: manifest.payload_sha256,
    };
    let pending_json = serde_json::to_vec_pretty(&pending)?;
    atomic_write_bytes(&updater_dir.join("pending_update.json"), &pending_json)?;

    tracing::info!(
        target: "wavical.updater",
        event = "updater.staged",
        version = %manifest.version,
        "verified update staged and ready"
    );

    Ok(UpdateCheckResponse {
        available: true,
        version: Some(manifest.version),
        notes: Some(manifest.release_notes),
    })
}

/// Apply an already-verified pending update by atomically switching `previous` and `current`
/// pointer files while keeping the previous version on disk.
pub fn apply_pending_update(updater_dir: &Path) -> Result<String, UpdaterError> {
    let pending_path = updater_dir.join("pending_update.json");
    if !pending_path.is_file() {
        return Err(UpdaterError::NoPendingUpdate);
    }
    let pending_bytes = fs::read(&pending_path)?;
    let pending: PendingUpdateRecord = serde_json::from_slice(&pending_bytes)?;
    validate_safe_version_dir_name(&pending.version)?;

    let new_dir = updater_dir.join("versions").join(&pending.version);
    if !new_dir.is_dir() {
        return Err(UpdaterError::NoPendingUpdate);
    }

    let current_path = updater_dir.join("current");
    let previous_path = updater_dir.join("previous");
    if let Ok(prev) = fs::read_to_string(&current_path) {
        let prev_trimmed = prev.trim();
        if !prev_trimmed.is_empty() && prev_trimmed != pending.version {
            atomic_write_bytes(&previous_path, format!("{prev_trimmed}\n").as_bytes())?;
        }
    }

    atomic_write_bytes(&current_path, format!("{}\n", pending.version).as_bytes())?;

    let health = UpdateHealthState::new(&pending.version);
    let health_json = serde_json::to_vec_pretty(&health)?;
    atomic_write_bytes(&updater_dir.join("health.json"), &health_json)?;

    let _ = fs::remove_file(&pending_path);

    tracing::info!(
        target: "wavical.updater",
        event = "updater.applied",
        version = %pending.version,
        "switched current pointer to new update version"
    );

    Ok(pending.version)
}

/// Prepare the application version to launch, applying any pending update first, and
/// automatically rolling back to `previous` if the newly applied version exceeds the
/// 2-failure health-check limit.
pub fn prepare_launch(
    updater_dir: &Path,
    fallback_version: &str,
) -> Result<LaunchTarget, UpdaterError> {
    if updater_dir.join("pending_update.json").is_file() {
        let _ = apply_pending_update(updater_dir);
    }

    let current_path = updater_dir.join("current");
    let current_version = match fs::read_to_string(&current_path) {
        Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            atomic_write_bytes(&current_path, format!("{fallback_version}\n").as_bytes())?;
            fallback_version.to_string()
        }
    };

    let health_path = updater_dir.join("health.json");
    if health_path.is_file()
        && let Ok(raw) = fs::read(&health_path)
        && let Ok(mut health) = serde_json::from_slice::<UpdateHealthState>(&raw)
        && health.active_version == current_version
        && !health.ready
    {
        if health.record_launch_and_should_rollback() {
            // Exceeded failure limit: roll back to `previous` if present on disk
            let previous_path = updater_dir.join("previous");
            if let Ok(prev_raw) = fs::read_to_string(&previous_path) {
                let prev_ver = prev_raw.trim().to_string();
                let prev_dir = updater_dir.join("versions").join(&prev_ver);
                if !prev_ver.is_empty() && prev_dir.is_dir() {
                    atomic_write_bytes(&current_path, format!("{prev_ver}\n").as_bytes())?;
                    let mut restored = UpdateHealthState::new(&prev_ver);
                    restored.mark_ready();
                    let restored_json = serde_json::to_vec_pretty(&restored)?;
                    atomic_write_bytes(&health_path, &restored_json)?;

                    tracing::warn!(
                        target: "wavical.updater",
                        event = "updater.rollback",
                        failed_version = %current_version,
                        restored_version = %prev_ver,
                        "rolled back update after repeated health-check failures"
                    );

                    return Ok(LaunchTarget {
                        version: prev_ver,
                        version_dir: prev_dir,
                        rolled_back_from: Some(current_version),
                    });
                }
            }
        }
        let updated_json = serde_json::to_vec_pretty(&health)?;
        atomic_write_bytes(&health_path, &updated_json)?;
    }

    Ok(LaunchTarget {
        version_dir: updater_dir.join("versions").join(&current_version),
        version: current_version,
        rolled_back_from: None,
    })
}

/// Mark the currently running version as healthy (called when the app reports "ready").
pub fn mark_app_ready(updater_dir: &Path, version: &str) -> Result<(), UpdaterError> {
    let health_path = updater_dir.join("health.json");
    let mut state = if health_path.is_file() {
        let raw = fs::read(&health_path)?;
        serde_json::from_slice::<UpdateHealthState>(&raw)
            .unwrap_or_else(|_| UpdateHealthState::new(version))
    } else {
        UpdateHealthState::new(version)
    };
    state.active_version = version.to_string();
    state.mark_ready();
    let json = serde_json::to_vec_pretty(&state)?;
    atomic_write_bytes(&health_path, &json)?;
    Ok(())
}
