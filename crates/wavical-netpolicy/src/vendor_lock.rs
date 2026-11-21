//! `vendor.lock.json` parser and integrity validator (Plan 1.11).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::limits::{MAX_VENDOR_FILE_BYTES, MAX_VENDOR_FILE_COUNT, MAX_VENDOR_TOTAL_BYTES};

/// Errors when parsing or validating `vendor.lock.json`.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum VendorLockError {
    #[error("invalid vendor.lock.json: {0}")]
    InvalidJson(String),
    #[error("unsupported vendor lock version {0}; expected 1")]
    UnsupportedVersion(u32),
    #[error("vendor lock exceeds maximum of {MAX_VENDOR_FILE_COUNT} files")]
    TooManyFiles,
    #[error("vendored file '{path}' exceeds max size of {MAX_VENDOR_FILE_BYTES} bytes")]
    FileTooLarge { path: String },
    #[error("total vendored size exceeds max of {MAX_VENDOR_TOTAL_BYTES} bytes")]
    TotalSizeTooLarge,
    #[error("invalid sha256 hash for '{path}'")]
    InvalidSha256 { path: String },
    #[error("unsafe local path '{path}': must be relative under 'vendor/'")]
    UnsafeLocalPath { path: String },
    #[error("duplicate local path '{0}' in vendor.lock.json")]
    DuplicateLocalPath(String),
    #[error("entry '{path}' URL must use https://")]
    NonHttpsUrl { path: String },
}

/// A single vendored asset entry in `vendor.lock.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct VendorLockEntry {
    pub url: String,
    pub local_path: String,
    pub sha256: String,
    pub size: u64,
    pub fetched_at: String,
    #[serde(default)]
    pub license: Option<String>,
}

/// Parsed and validated `vendor.lock.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VendorLock {
    pub version: u32,
    pub entries: Vec<VendorLockEntry>,
}

impl VendorLock {
    /// Parses and validates `vendor.lock.json` bytes (strictly HTTPS-only).
    pub fn parse_bytes(raw: &[u8]) -> Result<Self, VendorLockError> {
        Self::parse_bytes_with_options(raw, false)
    }

    /// Parses and validates `vendor.lock.json` bytes, optionally allowing
    /// `http://127.0.0.1` / `http://localhost` for local fixture server tests.
    pub fn parse_bytes_with_options(
        raw: &[u8],
        allow_test_localhost: bool,
    ) -> Result<Self, VendorLockError> {
        let lock: Self =
            serde_json::from_slice(raw).map_err(|e| VendorLockError::InvalidJson(e.to_string()))?;

        if lock.version != 1 {
            return Err(VendorLockError::UnsupportedVersion(lock.version));
        }
        if lock.entries.len() > MAX_VENDOR_FILE_COUNT {
            return Err(VendorLockError::TooManyFiles);
        }

        let mut total_bytes: u64 = 0;
        let mut seen_paths = HashSet::with_capacity(lock.entries.len());

        for entry in &lock.entries {
            let Ok(parsed_url) = Url::parse(&entry.url) else {
                return Err(VendorLockError::NonHttpsUrl {
                    path: entry.local_path.clone(),
                });
            };
            let is_loopback_http = allow_test_localhost
                && parsed_url.scheme() == "http"
                && parsed_url
                    .host_str()
                    .is_some_and(|h| h == "127.0.0.1" || h == "localhost");
            if (parsed_url.scheme() != "https" && !is_loopback_http) || parsed_url.host().is_none()
            {
                return Err(VendorLockError::NonHttpsUrl {
                    path: entry.local_path.clone(),
                });
            }
            if !is_safe_vendor_local_path(&entry.local_path) {
                return Err(VendorLockError::UnsafeLocalPath {
                    path: entry.local_path.clone(),
                });
            }
            if !seen_paths.insert(entry.local_path.clone()) {
                return Err(VendorLockError::DuplicateLocalPath(
                    entry.local_path.clone(),
                ));
            }
            if !is_valid_sha256_hex(&entry.sha256) {
                return Err(VendorLockError::InvalidSha256 {
                    path: entry.local_path.clone(),
                });
            }
            if entry.size > MAX_VENDOR_FILE_BYTES {
                return Err(VendorLockError::FileTooLarge {
                    path: entry.local_path.clone(),
                });
            }
            total_bytes = total_bytes
                .checked_add(entry.size)
                .ok_or(VendorLockError::TotalSizeTooLarge)?;
            if total_bytes > MAX_VENDOR_TOTAL_BYTES {
                return Err(VendorLockError::TotalSizeTooLarge);
            }
        }

        Ok(lock)
    }
}

fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Validates that `path` is a safe relative POSIX path strictly under `vendor/`.
#[must_use]
pub fn is_safe_vendor_local_path(path: &str) -> bool {
    if path.is_empty()
        || path.contains('\0')
        || path.contains('\\')
        || path.contains(':')
        || path.starts_with('/')
    {
        return false;
    }
    let segs: Vec<&str> = path.split('/').collect();
    if segs.len() < 2 || segs[0] != "vendor" {
        return false;
    }
    for seg in &segs {
        if seg.is_empty() || *seg == "." || *seg == ".." {
            return false;
        }
        if seg.ends_with('.') || seg.ends_with(' ') {
            return false;
        }
    }
    true
}
