#![forbid(unsafe_code)]

use crate::catalog::{ModelCatalogEntry, is_host_allowed};
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

/// Download error categories.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DownloadError {
    #[error("insecure URL '{0}': model downloads strictly require https://")]
    InsecureScheme(String),

    #[error("host '{0}' is not in the approved download allowlist")]
    DisallowedHost(String),

    #[error("too many redirects (exceeded limit of {0})")]
    TooManyRedirects(usize),

    #[error("another model download is already in progress (locked by {0})")]
    DownloadAlreadyInProgress(PathBuf),

    #[error(
        "insufficient disk space: need at least {needed} bytes, only {available} bytes available"
    )]
    InsufficientDiskSpace { needed: u64, available: u64 },

    #[error("server returned HTTP status {status}: {message}")]
    HttpStatus { status: u16, message: String },

    #[error("downloaded file size mismatch: expected {expected} bytes, got {actual} bytes")]
    SizeMismatch { expected: u64, actual: u64 },

    #[error("SHA-256 integrity check failed: expected '{expected}', computed '{actual}'")]
    HashMismatch { expected: String, actual: String },

    #[error("GGUF header validation failed: {0}")]
    GgufHeaderFailed(String),

    #[error("I/O error at '{path}': {message}")]
    IoError { path: PathBuf, message: String },

    #[error("network transfer error: {0}")]
    NetworkError(String),
}

/// Options to customize download execution (used in tests for mock servers).
#[derive(Debug, Clone, Copy, Default)]
pub struct DownloadOptions {
    /// Allow http://127.0.0.1 or http://localhost (strictly for unit tests).
    pub allow_test_localhost: bool,
    /// Skip free disk space check (strictly for unit tests in mock temp dirs).
    pub skip_disk_check: bool,
}

/// RAII lock guard guaranteeing at most one download executes at a time.
#[derive(Debug)]
pub struct DownloadLockGuard {
    lock_path: PathBuf,
}

impl DownloadLockGuard {
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
}

impl Drop for DownloadLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

/// Progress callback reporting current downloaded bytes and total expected bytes.
pub type ProgressCallback<'a> = &'a mut dyn FnMut(u64, u64);

/// Validate that a URL uses HTTPS and an allowlisted host.
pub fn validate_download_url(
    url_str: &str,
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

    if !is_host_allowed(host) {
        return Err(DownloadError::DisallowedHost(host.to_string()));
    }

    Ok(url_str.to_string())
}

/// Download a model catalog entry into `models_dir` following all Phase 3 Section 1.6 rules.
pub fn download_model(
    entry: &ModelCatalogEntry,
    models_dir: &Path,
    options: &DownloadOptions,
    mut on_progress: Option<ProgressCallback>,
) -> Result<PathBuf, DownloadError> {
    fs::create_dir_all(models_dir).map_err(|e| DownloadError::IoError {
        path: models_dir.to_path_buf(),
        message: e.to_string(),
    })?;

    // 1. One download at a time lock
    let _lock = DownloadLockGuard::acquire(models_dir)?;

    let target_path = resolve_model_file_path(models_dir, &entry.file_name).map_err(|e| {
        DownloadError::IoError {
            path: models_dir.to_path_buf(),
            message: e.to_string(),
        }
    })?;

    // 2. Free disk space check
    if !options.skip_disk_check
        && let Ok(available) = fs4::available_space(models_dir)
    {
        let needed = entry.size_bytes.saturating_add(50 * 1024 * 1024); // 50 MB headroom
        if available < needed {
            return Err(DownloadError::InsufficientDiskSpace { needed, available });
        }
    }

    let part_path = models_dir.join(format!("{}.part", entry.file_name));

    // 3. Resume evaluation: check if a partial file already exists
    let mut resumed_bytes = 0u64;
    if part_path.exists()
        && let Ok(meta) = part_path.metadata()
    {
        let len = meta.len();
        if len < entry.size_bytes {
            resumed_bytes = len;
        } else {
            let _ = fs::remove_file(&part_path);
        }
    }

    // 4. Configure HTTP client with NO automatic redirects
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(3600)) // 1 hour ceiling for large downloads
        .build()
        .map_err(|e| DownloadError::NetworkError(e.to_string()))?;

    let mut current_url = entry.url.clone();
    let mut hops = 0;

    let mut response = loop {
        validate_download_url(&current_url, options)?;

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
                // Relative redirect
                let base = reqwest::Url::parse(&current_url)
                    .map_err(|e| DownloadError::NetworkError(e.to_string()))?;
                base.join(loc)
                    .map_err(|e| DownloadError::NetworkError(e.to_string()))?
                    .to_string()
            };

            current_url = next_url;
            continue;
        }

        break resp;
    };

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
        cb(downloaded, entry.size_bytes);
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
            cb(downloaded, entry.size_bytes);
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

    if actual_size != entry.size_bytes {
        let _ = fs::remove_file(&part_path);
        return Err(DownloadError::SizeMismatch {
            expected: entry.size_bytes,
            actual: actual_size,
        });
    }

    // 7. Verify SHA-256 hash
    let computed_hash = sha256_file(&part_path).map_err(|e| DownloadError::IoError {
        path: part_path.clone(),
        message: e.to_string(),
    })?;

    if computed_hash != entry.sha256 {
        let _ = fs::remove_file(&part_path);
        return Err(DownloadError::HashMismatch {
            expected: entry.sha256.clone(),
            actual: computed_hash,
        });
    }

    // 8. GGUF header check (magic bytes + version)
    if let Err(e) = validate_gguf_header(&part_path) {
        let _ = fs::remove_file(&part_path);
        return Err(DownloadError::GgufHeaderFailed(e.to_string()));
    }

    // 9. Atomic rename into target model path
    fs::rename(&part_path, &target_path).map_err(|e| DownloadError::IoError {
        path: target_path.clone(),
        message: e.to_string(),
    })?;

    Ok(target_path)
}
