#![forbid(unsafe_code)]

//! Pure update manifest parser, validator, semver comparator, and health-check state machine.
//!
//! Zero OS, process, or network dependencies (testable and fuzzable on Linux).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Plan 1.10: maximum update manifest JSON size (64 KB).
pub const MAX_UPDATE_MANIFEST_BYTES: usize = 64 * 1024;

/// Plan 1.10: maximum release notes size in plain text (8 KB).
pub const MAX_UPDATE_RELEASE_NOTES_BYTES: usize = 8 * 1024;

/// Plan 1.10: maximum update payload size (1 GB).
pub const MAX_UPDATE_PAYLOAD_BYTES: u64 = 1024 * 1024 * 1024;

/// Plan 1.10: maximum manifest validity window (`expires - now`, 30 days).
pub const MAX_UPDATE_MANIFEST_VALIDITY_SECS: u64 = 30 * 24 * 60 * 60;

/// Plan 1.10: failed health checks before automatic rollback (2).
pub const MAX_UPDATE_HEALTH_FAILURES: u32 = 2;

/// Maximum length of `appId` in update manifests.
pub const MAX_UPDATE_APP_ID_LEN: usize = 64;

/// Validated update manifest matching Plan Section 1.6.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateManifest {
    pub app_id: String,
    pub version: String,
    pub channel: String,
    pub minimum_supported_version: String,
    pub expires: u64,
    pub rollout_percent: u8,
    pub payload_url: String,
    pub payload_size: u64,
    pub payload_sha256: String,
    pub release_notes: String,
}

/// Pure errors returned by manifest parsing and install validation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UpdateManifestError {
    #[error("update manifest exceeds maximum size of {limit} bytes (got {actual} bytes)")]
    ManifestTooLarge { limit: usize, actual: usize },

    #[error("malformed update manifest JSON: {0}")]
    MalformedJson(String),

    #[error("invalid appId '{0}' in update manifest")]
    InvalidAppId(String),

    #[error("invalid version '{0}' in update manifest")]
    InvalidVersion(String),

    #[error("invalid channel '{0}'; must be 'stable', 'beta', or 'canary'")]
    InvalidChannel(String),

    #[error("invalid rolloutPercent {0}; must be 0..=100")]
    InvalidRolloutPercent(u8),

    #[error("invalid payloadUrl: {0}")]
    InvalidPayloadUrl(String),

    #[error("invalid payloadSize {size}; must be 1..={max}")]
    InvalidPayloadSize { size: u64, max: u64 },

    #[error("invalid payloadSha256; expected 64 lowercase/uppercase hex characters")]
    InvalidPayloadSha256,

    #[error("release notes exceed maximum size of {limit} bytes (got {actual} bytes)")]
    ReleaseNotesTooLarge { limit: usize, actual: usize },

    #[error("release notes must be plain text without control characters")]
    InvalidReleaseNotes,

    #[error("appId mismatch: expected '{expected}', got '{actual}'")]
    AppIdMismatch { expected: String, actual: String },

    #[error("channel mismatch: expected '{expected}', got '{actual}'")]
    ChannelMismatch { expected: String, actual: String },

    #[error("version '{candidate}' is not strictly greater than installed '{installed}'")]
    NotNewerVersion {
        candidate: String,
        installed: String,
    },

    #[error("installed version '{installed}' is below minimumSupportedVersion '{minimum}'")]
    BelowMinimumSupportedVersion { installed: String, minimum: String },

    #[error("update manifest has expired (expires={expires}, now={now})")]
    ManifestExpired { expires: u64, now: u64 },

    #[error("update manifest expiry exceeds maximum 30-day validity window")]
    ExpiryTooFarInFuture,

    #[error("install rollout bucket {bucket} is not below rolloutPercent {rollout_percent}")]
    RolloutBucketExcluded { bucket: u8, rollout_percent: u8 },

    #[error("updater IPC methods do not accept parameters")]
    UnexpectedParameters,
}

/// Pure SemVer representation (`major.minor.patch[-prerelease]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSemver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl UpdateSemver {
    pub fn parse(raw: &str) -> Result<Self, UpdateManifestError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(UpdateManifestError::InvalidVersion(raw.to_string()));
        }

        let (core, pre) = match trimmed.split_once('-') {
            Some((c, p)) => {
                if p.is_empty()
                    || !p
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                {
                    return Err(UpdateManifestError::InvalidVersion(raw.to_string()));
                }
                (c, Some(p.to_string()))
            }
            None => (trimmed, None),
        };

        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 {
            return Err(UpdateManifestError::InvalidVersion(raw.to_string()));
        }

        let mut nums = [0u64; 3];
        for (idx, part) in parts.iter().enumerate() {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(UpdateManifestError::InvalidVersion(raw.to_string()));
            }
            if part.len() > 1 && part.starts_with('0') {
                return Err(UpdateManifestError::InvalidVersion(raw.to_string()));
            }
            nums[idx] = part
                .parse::<u64>()
                .map_err(|_| UpdateManifestError::InvalidVersion(raw.to_string()))?;
        }

        Ok(Self {
            major: nums[0],
            minor: nums[1],
            patch: nums[2],
            pre,
        })
    }
}

impl Ord for UpdateSemver {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)) {
            std::cmp::Ordering::Equal => match (&self.pre, &other.pre) {
                (None, None) => std::cmp::Ordering::Equal,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(a), Some(b)) => a.cmp(b),
            },
            non_eq => non_eq,
        }
    }
}

impl PartialOrd for UpdateSemver {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn validate_app_id(app_id: &str) -> Result<(), UpdateManifestError> {
    if app_id.is_empty()
        || app_id.len() > MAX_UPDATE_APP_ID_LEN
        || app_id.contains("..")
        || !app_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
    {
        return Err(UpdateManifestError::InvalidAppId(app_id.to_string()));
    }
    Ok(())
}

fn validate_payload_url(url: &str, allow_test_localhost: bool) -> Result<(), UpdateManifestError> {
    let trimmed = url.trim();
    let rest = if let Some(r) = trimmed.strip_prefix("https://") {
        r
    } else if allow_test_localhost {
        if let Some(r) = trimmed.strip_prefix("http://127.0.0.1") {
            r
        } else if let Some(r) = trimmed.strip_prefix("http://localhost") {
            r
        } else {
            return Err(UpdateManifestError::InvalidPayloadUrl(
                "payloadUrl must use https://".to_string(),
            ));
        }
    } else {
        return Err(UpdateManifestError::InvalidPayloadUrl(
            "payloadUrl must use https://".to_string(),
        ));
    };

    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(UpdateManifestError::InvalidPayloadUrl(
            "payloadUrl must contain a valid host without userinfo".to_string(),
        ));
    }
    Ok(())
}

fn validate_sha256_hex(raw: &str) -> Result<String, UpdateManifestError> {
    let hex = raw.strip_prefix("sha256:").unwrap_or(raw);
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UpdateManifestError::InvalidPayloadSha256);
    }
    Ok(hex.to_ascii_lowercase())
}

fn validate_release_notes(notes: &str) -> Result<(), UpdateManifestError> {
    if notes.len() > MAX_UPDATE_RELEASE_NOTES_BYTES {
        return Err(UpdateManifestError::ReleaseNotesTooLarge {
            limit: MAX_UPDATE_RELEASE_NOTES_BYTES,
            actual: notes.len(),
        });
    }
    for ch in notes.chars() {
        if ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t' {
            return Err(UpdateManifestError::InvalidReleaseNotes);
        }
    }
    Ok(())
}

/// Parse and validate an `UpdateManifest` from raw bytes (HTTPS-only payloadUrl).
pub fn parse_update_manifest(raw_bytes: &[u8]) -> Result<UpdateManifest, UpdateManifestError> {
    parse_update_manifest_with_options(raw_bytes, false)
}

/// Parse and validate an `UpdateManifest` from raw bytes, with optional test-localhost URL allowance.
pub fn parse_update_manifest_with_options(
    raw_bytes: &[u8],
    allow_test_localhost: bool,
) -> Result<UpdateManifest, UpdateManifestError> {
    if raw_bytes.len() > MAX_UPDATE_MANIFEST_BYTES {
        return Err(UpdateManifestError::ManifestTooLarge {
            limit: MAX_UPDATE_MANIFEST_BYTES,
            actual: raw_bytes.len(),
        });
    }

    let mut manifest: UpdateManifest = serde_json::from_slice(raw_bytes)
        .map_err(|e| UpdateManifestError::MalformedJson(e.to_string()))?;

    validate_app_id(&manifest.app_id)?;
    let _ = UpdateSemver::parse(&manifest.version)?;
    let _ = UpdateSemver::parse(&manifest.minimum_supported_version)?;

    if !matches!(manifest.channel.as_str(), "stable" | "beta" | "canary") {
        return Err(UpdateManifestError::InvalidChannel(manifest.channel));
    }
    if manifest.rollout_percent > 100 {
        return Err(UpdateManifestError::InvalidRolloutPercent(
            manifest.rollout_percent,
        ));
    }

    validate_payload_url(&manifest.payload_url, allow_test_localhost)?;

    if manifest.payload_size == 0 || manifest.payload_size > MAX_UPDATE_PAYLOAD_BYTES {
        return Err(UpdateManifestError::InvalidPayloadSize {
            size: manifest.payload_size,
            max: MAX_UPDATE_PAYLOAD_BYTES,
        });
    }

    manifest.payload_sha256 = validate_sha256_hex(&manifest.payload_sha256)?;
    validate_release_notes(&manifest.release_notes)?;

    Ok(manifest)
}

/// Validate a parsed manifest against the installed app state in Plan 1.6 order:
/// appId -> channel -> strictly newer version & minimumSupportedVersion -> expiry -> rollout bucket.
pub fn validate_manifest_for_install(
    manifest: &UpdateManifest,
    expected_app_id: &str,
    expected_channel: &str,
    installed_version: &str,
    rollout_bucket: u8,
    now_secs: u64,
) -> Result<(), UpdateManifestError> {
    if manifest.app_id != expected_app_id {
        return Err(UpdateManifestError::AppIdMismatch {
            expected: expected_app_id.to_string(),
            actual: manifest.app_id.clone(),
        });
    }

    if manifest.channel != expected_channel {
        return Err(UpdateManifestError::ChannelMismatch {
            expected: expected_channel.to_string(),
            actual: manifest.channel.clone(),
        });
    }

    let candidate_ver = UpdateSemver::parse(&manifest.version)?;
    let installed_ver = UpdateSemver::parse(installed_version)?;
    if candidate_ver <= installed_ver {
        return Err(UpdateManifestError::NotNewerVersion {
            candidate: manifest.version.clone(),
            installed: installed_version.to_string(),
        });
    }

    let min_ver = UpdateSemver::parse(&manifest.minimum_supported_version)?;
    if installed_ver < min_ver {
        return Err(UpdateManifestError::BelowMinimumSupportedVersion {
            installed: installed_version.to_string(),
            minimum: manifest.minimum_supported_version.clone(),
        });
    }

    if manifest.expires <= now_secs {
        return Err(UpdateManifestError::ManifestExpired {
            expires: manifest.expires,
            now: now_secs,
        });
    }

    if manifest.expires - now_secs > MAX_UPDATE_MANIFEST_VALIDITY_SECS {
        return Err(UpdateManifestError::ExpiryTooFarInFuture);
    }

    if rollout_bucket >= manifest.rollout_percent {
        return Err(UpdateManifestError::RolloutBucketExcluded {
            bucket: rollout_bucket,
            rollout_percent: manifest.rollout_percent,
        });
    }

    Ok(())
}

/// Validate that `updater.check` / `updater.apply` IPC calls carry zero parameters.
pub fn validate_updater_empty_params(
    params: &serde_json::Value,
) -> Result<(), UpdateManifestError> {
    match params {
        serde_json::Value::Null => Ok(()),
        serde_json::Value::Object(map) if map.is_empty() => Ok(()),
        _ => Err(UpdateManifestError::UnexpectedParameters),
    }
}

/// Pure health-check state persisted in `health.json` for two-strike automatic rollback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateHealthState {
    pub active_version: String,
    pub consecutive_failures: u32,
    pub ready: bool,
}

impl UpdateHealthState {
    pub fn new(version: &str) -> Self {
        Self {
            active_version: version.to_string(),
            consecutive_failures: 0,
            ready: false,
        }
    }

    /// Record a launch attempt. Returns `true` if `MAX_UPDATE_HEALTH_FAILURES` were
    /// already reached without reporting ready, meaning the launcher must roll back.
    pub fn record_launch_and_should_rollback(&mut self) -> bool {
        if !self.ready && self.consecutive_failures >= MAX_UPDATE_HEALTH_FAILURES {
            return true;
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.ready = false;
        false
    }

    /// Mark the active version as healthy/ready.
    pub fn mark_ready(&mut self) {
        self.ready = true;
        self.consecutive_failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest_json(notes: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "appId": "com.example.app",
            "version": "1.1.0",
            "channel": "stable",
            "minimumSupportedVersion": "1.0.0",
            "expires": 2_000_000_000u64,
            "rolloutPercent": 50,
            "payloadUrl": "https://github.com/org/repo/releases/download/v1.1.0/app.zip",
            "payloadSize": 1024,
            "payloadSha256": "a".repeat(64),
            "releaseNotes": notes
        }))
        .unwrap()
    }

    #[test]
    fn test_manifest_size_boundary() {
        let base = sample_manifest_json("ok");
        assert!(parse_update_manifest(&base).is_ok());

        // Pad whitespace to exact 64 KB limit vs 64 KB + 1
        let mut at_limit = base.clone();
        at_limit.resize(MAX_UPDATE_MANIFEST_BYTES, b' ');
        assert!(parse_update_manifest(&at_limit).is_ok());

        let mut over_limit = base;
        over_limit.resize(MAX_UPDATE_MANIFEST_BYTES + 1, b' ');
        assert!(matches!(
            parse_update_manifest(&over_limit),
            Err(UpdateManifestError::ManifestTooLarge { .. })
        ));
    }

    #[test]
    fn test_release_notes_boundary_and_control_chars() {
        let exact = "x".repeat(MAX_UPDATE_RELEASE_NOTES_BYTES);
        assert!(parse_update_manifest(&sample_manifest_json(&exact)).is_ok());

        let over = "x".repeat(MAX_UPDATE_RELEASE_NOTES_BYTES + 1);
        assert!(matches!(
            parse_update_manifest(&sample_manifest_json(&over)),
            Err(UpdateManifestError::ReleaseNotesTooLarge { .. })
        ));

        let with_nul = "hello\x00world";
        assert!(matches!(
            parse_update_manifest(&sample_manifest_json(with_nul)),
            Err(UpdateManifestError::InvalidReleaseNotes)
        ));
    }

    #[test]
    fn test_payload_size_boundary() {
        let mut val: serde_json::Value =
            serde_json::from_slice(&sample_manifest_json("ok")).unwrap();
        val["payloadSize"] = serde_json::json!(MAX_UPDATE_PAYLOAD_BYTES);
        assert!(parse_update_manifest(&serde_json::to_vec(&val).unwrap()).is_ok());

        val["payloadSize"] = serde_json::json!(MAX_UPDATE_PAYLOAD_BYTES + 1);
        assert!(matches!(
            parse_update_manifest(&serde_json::to_vec(&val).unwrap()),
            Err(UpdateManifestError::InvalidPayloadSize { .. })
        ));
    }

    #[test]
    fn test_unknown_field_rejected() {
        let mut val: serde_json::Value =
            serde_json::from_slice(&sample_manifest_json("ok")).unwrap();
        val["evilField"] = serde_json::json!("surprise");
        assert!(matches!(
            parse_update_manifest(&serde_json::to_vec(&val).unwrap()),
            Err(UpdateManifestError::MalformedJson(_))
        ));
    }

    #[test]
    fn test_install_validation_rules_and_boundaries() {
        let now = 1_999_000_000u64;
        let mut m = parse_update_manifest(&sample_manifest_json("notes")).unwrap();
        m.expires = now + MAX_UPDATE_MANIFEST_VALIDITY_SECS;

        // Rollout bucket just below (49 < 50) passes; at percentage (50 >= 50) rejected
        assert!(
            validate_manifest_for_install(&m, "com.example.app", "stable", "1.0.0", 49, now)
                .is_ok()
        );
        assert!(matches!(
            validate_manifest_for_install(&m, "com.example.app", "stable", "1.0.0", 50, now),
            Err(UpdateManifestError::RolloutBucketExcluded { .. })
        ));

        // Expiry beyond 30 days + 1s rejected
        m.expires = now + MAX_UPDATE_MANIFEST_VALIDITY_SECS + 1;
        assert!(matches!(
            validate_manifest_for_install(&m, "com.example.app", "stable", "1.0.0", 0, now),
            Err(UpdateManifestError::ExpiryTooFarInFuture)
        ));

        // Expired manifest rejected
        m.expires = now;
        assert!(matches!(
            validate_manifest_for_install(&m, "com.example.app", "stable", "1.0.0", 0, now),
            Err(UpdateManifestError::ManifestExpired { .. })
        ));
        m.expires = now + 3600;

        // Same version & downgrade rejected
        assert!(matches!(
            validate_manifest_for_install(&m, "com.example.app", "stable", "1.1.0", 0, now),
            Err(UpdateManifestError::NotNewerVersion { .. })
        ));
        assert!(matches!(
            validate_manifest_for_install(&m, "com.example.app", "stable", "1.2.0", 0, now),
            Err(UpdateManifestError::NotNewerVersion { .. })
        ));

        // Channel & appId mismatch rejected
        assert!(matches!(
            validate_manifest_for_install(&m, "com.example.app", "beta", "1.0.0", 0, now),
            Err(UpdateManifestError::ChannelMismatch { .. })
        ));
        assert!(matches!(
            validate_manifest_for_install(&m, "other.app", "stable", "1.0.0", 0, now),
            Err(UpdateManifestError::AppIdMismatch { .. })
        ));
    }

    #[test]
    fn test_health_state_two_strike_rollback() {
        let mut h = UpdateHealthState::new("1.0.1");
        assert!(!h.record_launch_and_should_rollback()); // failure 1
        assert!(!h.record_launch_and_should_rollback()); // failure 2
        assert!(h.record_launch_and_should_rollback()); // 3rd start triggers rollback

        // If marked ready, counter resets
        let mut h2 = UpdateHealthState::new("1.0.1");
        assert!(!h2.record_launch_and_should_rollback());
        h2.mark_ready();
        assert!(!h2.record_launch_and_should_rollback());
        assert_eq!(h2.consecutive_failures, 1);
    }

    #[test]
    fn test_updater_empty_params_rejects_extra_fields() {
        assert!(validate_updater_empty_params(&serde_json::Value::Null).is_ok());
        assert!(validate_updater_empty_params(&serde_json::json!({})).is_ok());
        assert!(
            validate_updater_empty_params(&serde_json::json!({"url": "https://evil.com"})).is_err()
        );
        assert!(validate_updater_empty_params(&serde_json::json!({"version": "9.9.9"})).is_err());
    }
}
