#![forbid(unsafe_code)]

//! Shared, cryptographically-verified downloader module.
//!
//! Enforces:
//! 1. HTTPS only (unless explicitly bypassed for local test servers via `allow_test_localhost`).
//! 2. Strict host allowlists per download kind (models, engine, runtimes, updates).
//! 3. Manual HTTP redirect handling with allowlist re-validation at every single hop.
//! 4. Atomic `.part` file streaming with resume support (`Range: bytes=X-`).
//! 5. Lock-file protection: at most one download per target file / directory at a time.
//! 6. Free disk space verification with 50 MB safety headroom.
//! 7. Exact file size enforcement (rejects truncation or bloating).
//! 8. SHA-256 hash verification (deletes file on mismatch).
//! 9. Atomic rename to target destination on full verification success.

use crate::catalog::{ModelCatalogEntry, is_host_allowed as is_model_host_allowed};
use crate::gguf::validate_gguf_header;
use crate::sha256::sha256_file;
use crate::storage::resolve_model_file_path;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{HeaderValue, LOCATION, RANGE};
use reqwest::redirect::Policy;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Maximum number of HTTP redirects permitted.
pub const MAX_REDIRECT_HOPS: usize = 5;

/// Approved host allowlist for runtime binaries and archives (Node.js).
pub const ALLOWED_RUNTIME_HOSTS: &[&str] = &["nodejs.org", "dist.nodejs.org"];

/// Approved host allowlist for engine binaries (llama.cpp).
pub const ALLOWED_ENGINE_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// Approved host allowlist for application updates (default).
pub const ALLOWED_UPDATE_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// Category of file being downloaded, determining which host allowlist is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadKind {
    /// Local AI model weights (.gguf).
    Model,
    /// Native inference engine binary (.exe / .zip).
    Engine,
    /// Backend runtime distribution archive (.zip).
    Runtime,
    /// Shipped application update payload.
    Update,
}

/// Options to customize download execution (used in tests for mock servers).
#[derive(Debug, Clone, Copy, Default)]
pub struct DownloadOptions {
    /// Allow http://127.0.0.1 or http://localhost (strictly for unit tests).
    pub allow_test_localhost: bool,
    /// Skip free disk space check (strictly for unit tests in mock temp dirs).
    pub skip_disk_check: bool,
}

/// Check whether a host is permitted for a given download kind.
pub fn is_host_allowed_for_kind(
    kind: DownloadKind,
    host: &str,
    custom_allowed_hosts: &[&str],
) -> bool {
    let lower_host = host.to_ascii_lowercase();

    for &custom in custom_allowed_hosts {
        if lower_host == custom.to_ascii_lowercase() {
            return true;
        }
    }

    match kind {
        DownloadKind::Model => is_model_host_allowed(&lower_host),
        DownloadKind::Engine => ALLOWED_ENGINE_HOSTS.iter().any(|&h| lower_host == h),
        DownloadKind::Runtime => ALLOWED_RUNTIME_HOSTS
            .iter()
            .any(|&h| lower_host == h || lower_host.ends_with(".nodejs.org")),
        DownloadKind::Update => ALLOWED_UPDATE_HOSTS.iter().any(|&h| lower_host == h),
    }
}

/// Download error categories.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DownloadError {
    #[error("insecure URL '{0}': downloads strictly require https://")]
    InsecureScheme(String),

    #[error("host '{0}' is not in the approved download allowlist")]
    DisallowedHost(String),

    #[error("too many redirects (exceeded limit of {0})")]
    TooManyRedirects(usize),

    #[error("another download is already in progress (locked by {0})")]
    DownloadAlreadyInProgress(PathBuf),

    #[error(
        "insufficient disk space: need at least {needed} bytes, only {available} bytes available"
    )]
    InsufficientDiskSpace { needed: u64, available: u64 },

    #[error("server returned HTTP status {status}: {message}")]
    HttpStatus { status: u16, message: String },

    #[error("downloaded file size mismatch: expected {expected} bytes, got {actual} bytes")]
    SizeMismatch { expected: u64, actual: u64 },

    #[error("downloaded payload exceeded maximum allowed size of {limit} bytes")]
    PayloadTooLarge { limit: usize },

    #[error("SHA-256 integrity check failed: expected '{expected}', computed '{actual}'")]
    HashMismatch { expected: String, actual: String },

    #[error("GGUF header validation failed: {0}")]
    GgufHeaderFailed(String),

    #[error("I/O error at '{path}': {message}")]
    IoError { path: PathBuf, message: String },

    #[error("network transfer error: {0}")]
    NetworkError(String),
}

/// RAII lock guard guaranteeing at most one download executes per target at a time.
#[derive(Debug)]
pub struct DownloadLockGuard {
    lock_path: PathBuf,
}

impl DownloadLockGuard {
    /// Acquire a directory-wide download lock (e.g. for models directory).
    pub fn acquire(models_dir: &Path) -> Result<Self, DownloadError> {
        let lock_path = models_dir.join("download.lock");
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => Ok(Self { lock_path }),
            Err(_) => Err(DownloadError::DownloadAlreadyInProgress(lock_path)),
        }
    }

    /// Acquire a target-specific lock file (`<target_path>.download.lock`).
    pub fn acquire_for_target(target_path: &Path) -> Result<Self, DownloadError> {
        let lock_name = match target_path.file_name() {
            Some(n) => format!("{}.download.lock", n.to_string_lossy()),
            None => "target.download.lock".to_string(),
        };
        let lock_path = target_path.with_file_name(lock_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => Ok(Self { lock_path }),
            Err(_) => Err(DownloadError::DownloadAlreadyInProgress(lock_path)),
        }
    }
}

impl Drop for DownloadLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

/// Progress callback reporting current downloaded bytes and total expected bytes.
pub type ProgressCallback<'a> = &'a mut dyn FnMut(u64, u64);

/// Validate that a URL uses HTTPS and an allowlisted host for a given download kind.
pub fn validate_download_url_for_kind(
    url_str: &str,
    kind: DownloadKind,
    options: &DownloadOptions,
) -> Result<String, DownloadError> {
    if options.allow_test_localhost
        && (url_str.starts_with("http://127.0.0.1") || url_str.starts_with("http://localhost"))
    {
        return Ok(url_str.to_string());
    }

    if !url_str.starts_with("https://") {
        return Err(DownloadError::InsecureScheme(url_str.to_string()));
    }

    let after_scheme = &url_str["https://".len()..];
    let authority = after_scheme.split('/').next().unwrap_or("");
    let host = authority.split(':').next().unwrap_or("");

    if host.is_empty() {
        return Err(DownloadError::DisallowedHost("empty".to_string()));
    }

    if options.allow_test_localhost && (host == "127.0.0.1" || host == "localhost") {
        return Ok(url_str.to_string());
    }

    if !is_host_allowed_for_kind(kind, host, &[]) {
        return Err(DownloadError::DisallowedHost(host.to_string()));
    }

    Ok(url_str.to_string())
}

/// Backwards-compatible validator defaulting to `DownloadKind::Model`.
pub fn validate_download_url(
    url_str: &str,
    options: &DownloadOptions,
) -> Result<String, DownloadError> {
    validate_download_url_for_kind(url_str, DownloadKind::Model, options)
}

/// Download and cryptographically verify any arbitrary target file using the shared engine rules.
pub fn download_verified_file(
    url: &str,
    target_path: &Path,
    expected_size: u64,
    expected_sha256: &str,
    kind: DownloadKind,
    options: &DownloadOptions,
    mut on_progress: Option<ProgressCallback>,
) -> Result<PathBuf, DownloadError> {
    let parent_dir = target_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent_dir).map_err(|e| DownloadError::IoError {
        path: parent_dir.to_path_buf(),
        message: e.to_string(),
    })?;

    // 1. One download per target lock
    let _lock = DownloadLockGuard::acquire_for_target(target_path)?;

    // 2. Free disk space check
    if !options.skip_disk_check
        && let Ok(available) = fs4::available_space(parent_dir)
    {
        let needed = expected_size.saturating_add(50 * 1024 * 1024); // 50 MB headroom
        if available < needed {
            return Err(DownloadError::InsufficientDiskSpace { needed, available });
        }
    }

    let part_name = match target_path.file_name() {
        Some(n) => format!("{}.part", n.to_string_lossy()),
        None => "download.part".to_string(),
    };
    let part_path = target_path.with_file_name(part_name);

    // 3. Resume evaluation: check if a partial file already exists
    let mut resumed_bytes = 0u64;
    if part_path.exists()
        && let Ok(meta) = part_path.metadata()
    {
        let len = meta.len();
        if len < expected_size {
            resumed_bytes = len;
        } else {
            let _ = fs::remove_file(&part_path);
        }
    }

    // 4. Configure HTTP client with NO automatic redirects and follow hops safely
    let mut response = fetch_with_validated_redirects(url, resumed_bytes, kind, options)?;

    let status = response.status();
    let mut file = match status {
        StatusCode::PARTIAL_CONTENT if resumed_bytes > 0 => OpenOptions::new()
            .create(true)
            .append(true)
            .open(&part_path)
            .map_err(|e| DownloadError::IoError {
                path: part_path.clone(),
                message: e.to_string(),
            })?,
        StatusCode::OK => {
            resumed_bytes = 0;
            File::create(&part_path).map_err(|e| DownloadError::IoError {
                path: part_path.clone(),
                message: e.to_string(),
            })?
        }
        other => {
            let _ = fs::remove_file(&part_path);
            return Err(DownloadError::HttpStatus {
                status: other.as_u16(),
                message: format!("HTTP request returned status {other}"),
            });
        }
    };

    // 5. Stream download body into .part file with progress updates
    let mut downloaded = resumed_bytes;
    if let Some(ref mut cb) = on_progress {
        cb(downloaded, expected_size);
    }

    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = response
            .read(&mut buf)
            .map_err(|e| DownloadError::NetworkError(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| DownloadError::IoError {
                path: part_path.clone(),
                message: e.to_string(),
            })?;
        downloaded = downloaded.saturating_add(n as u64);

        if let Some(ref mut cb) = on_progress {
            cb(downloaded, expected_size);
        }
    }
    file.flush().map_err(|e| DownloadError::IoError {
        path: part_path.clone(),
        message: e.to_string(),
    })?;
    drop(file);

    // 6. Verify exact size
    let actual_size = part_path
        .metadata()
        .map_err(|e| DownloadError::IoError {
            path: part_path.clone(),
            message: e.to_string(),
        })?
        .len();

    if actual_size != expected_size {
        let _ = fs::remove_file(&part_path);
        return Err(DownloadError::SizeMismatch {
            expected: expected_size,
            actual: actual_size,
        });
    }

    // 7. Verify SHA-256 hash
    let computed_hash = sha256_file(&part_path).map_err(|e| DownloadError::IoError {
        path: part_path.clone(),
        message: e.to_string(),
    })?;

    if !computed_hash.eq_ignore_ascii_case(expected_sha256) {
        let _ = fs::remove_file(&part_path);
        return Err(DownloadError::HashMismatch {
            expected: expected_sha256.to_string(),
            actual: computed_hash,
        });
    }

    // 8. Atomic rename into target path
    fs::rename(&part_path, target_path).map_err(|e| DownloadError::IoError {
        path: target_path.to_path_buf(),
        message: e.to_string(),
    })?;

    Ok(target_path.to_path_buf())
}

fn fetch_with_validated_redirects(
    url: &str,
    resumed_bytes: u64,
    kind: DownloadKind,
    options: &DownloadOptions,
) -> Result<reqwest::blocking::Response, DownloadError> {
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(3600)) // 1 hour ceiling
        .build()
        .map_err(|e| DownloadError::NetworkError(e.to_string()))?;

    let mut current_url = url.to_string();
    let mut hops = 0;

    loop {
        validate_download_url_for_kind(&current_url, kind, options)?;

        let mut req_builder = client.get(&current_url);
        if resumed_bytes > 0 {
            let range_val = format!("bytes={resumed_bytes}-");
            if let Ok(val) = HeaderValue::from_str(&range_val) {
                req_builder = req_builder.header(RANGE, val);
            }
        }

        let resp = req_builder
            .send()
            .map_err(|e| DownloadError::NetworkError(e.to_string()))?;

        let status = resp.status();
        if status.is_redirection() {
            hops += 1;
            if hops > MAX_REDIRECT_HOPS {
                return Err(DownloadError::TooManyRedirects(MAX_REDIRECT_HOPS));
            }

            let loc = resp
                .headers()
                .get(LOCATION)
                .ok_or_else(|| {
                    DownloadError::NetworkError("Redirect missing Location header".to_string())
                })?
                .to_str()
                .map_err(|_| {
                    DownloadError::NetworkError("Invalid UTF-8 in Location header".to_string())
                })?;

            let next_url = if loc.starts_with("http://") || loc.starts_with("https://") {
                loc.to_string()
            } else {
                let base = reqwest::Url::parse(&current_url)
                    .map_err(|e| DownloadError::NetworkError(e.to_string()))?;
                base.join(loc)
                    .map_err(|e| DownloadError::NetworkError(e.to_string()))?
                    .to_string()
            };

            current_url = next_url;
            continue;
        }

        return Ok(resp);
    }
}

/// Download a small bounded payload (such as an update manifest or signature)
/// in memory through the shared downloader's HTTPS, host allowlist, and redirect
/// validation rules.
pub fn download_bounded_bytes(
    url: &str,
    max_bytes: usize,
    kind: DownloadKind,
    options: &DownloadOptions,
) -> Result<Vec<u8>, DownloadError> {
    let mut response = fetch_with_validated_redirects(url, 0, kind, options)?;
    let status = response.status();
    if status != StatusCode::OK {
        return Err(DownloadError::HttpStatus {
            status: status.as_u16(),
            message: format!("HTTP request returned status {status}"),
        });
    }

    if let Some(content_len) = response.content_length()
        && content_len > max_bytes as u64
    {
        return Err(DownloadError::PayloadTooLarge { limit: max_bytes });
    }

    let mut out = Vec::new();
    let mut buf = [0u8; 8 * 1024];
    loop {
        let n = response
            .read(&mut buf)
            .map_err(|e| DownloadError::NetworkError(e.to_string()))?;
        if n == 0 {
            break;
        }
        if out.len().saturating_add(n) > max_bytes {
            return Err(DownloadError::PayloadTooLarge { limit: max_bytes });
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

/// Download a model catalog entry into `models_dir` following all Phase 3 Section 1.6 rules.
pub fn download_model(
    entry: &ModelCatalogEntry,
    models_dir: &Path,
    options: &DownloadOptions,
    on_progress: Option<ProgressCallback>,
) -> Result<PathBuf, DownloadError> {
    fs::create_dir_all(models_dir).map_err(|e| DownloadError::IoError {
        path: models_dir.to_path_buf(),
        message: e.to_string(),
    })?;

    // Phase 3 directory-wide download lock
    let _dir_lock = DownloadLockGuard::acquire(models_dir)?;

    let target_path = resolve_model_file_path(models_dir, &entry.file_name).map_err(|e| {
        DownloadError::IoError {
            path: models_dir.to_path_buf(),
            message: e.to_string(),
        }
    })?;

    let downloaded_path = download_verified_file(
        &entry.url,
        &target_path,
        entry.size_bytes,
        &entry.sha256,
        DownloadKind::Model,
        options,
        on_progress,
    )?;

    // GGUF header check (magic bytes + version)
    if let Err(e) = validate_gguf_header(&downloaded_path) {
        let _ = fs::remove_file(&downloaded_path);
        return Err(DownloadError::GgufHeaderFailed(e.to_string()));
    }

    Ok(downloaded_path)
}
