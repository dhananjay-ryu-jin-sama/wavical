#![forbid(unsafe_code)]

//! Safe archive extraction with streaming limit enforcement and pure entry path checks.
//!
//! Enforces:
//! 1. Rejection of directory traversal, absolute paths, drive letters, UNC, ADS, reserved names.
//! 2. Rejection of symlinks and hardlinks.
//! 3. Rejection of case-colliding entries.
//! 4. Streaming byte-count ceiling (decompression bomb protection).
//! 5. Entry-count ceiling.
//! 6. Extraction into an isolated temporary folder, finalized via atomic rename.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use wavical_protocol::{ArchiveEntryError, validate_archive_entry_path};

/// Limits applied during archive decompression.
#[derive(Debug, Clone, Copy)]
pub struct ExtractionLimits {
    /// Maximum total uncompressed bytes allowed across all files.
    pub max_bytes: u64,
    /// Maximum number of file/directory entries allowed.
    pub max_files: usize,
}

impl ExtractionLimits {
    /// Limits for backend runtime archives (Plan 1.10: 500 MB / 20,000 files).
    pub fn for_runtime() -> Self {
        Self {
            max_bytes: 500 * 1024 * 1024,
            max_files: 20_000,
        }
    }

    /// Limits for application updates (Plan 1.10: 1 GB / 20,000 files).
    pub fn for_update() -> Self {
        Self {
            max_bytes: 1024 * 1024 * 1024,
            max_files: 20_000,
        }
    }
}

/// Errors during archive extraction.
#[derive(Debug, Error)]
pub enum ExtractorError {
    #[error("entry path security check failed: {0}")]
    EntrySecurity(#[from] ArchiveEntryError),

    #[error("archive exceeded maximum allowed file count limit of {0}")]
    FileCountLimitExceeded(usize),

    #[error(
        "archive exceeded maximum allowed uncompressed size of {limit} bytes (wrote {actual} bytes)"
    )]
    SizeLimitExceeded { limit: u64, actual: u64 },

    #[error("failed to open archive at '{path}': {message}")]
    ArchiveOpenFailed { path: PathBuf, message: String },

    #[error("corrupted or invalid zip archive: {0}")]
    ZipFormatError(String),

    #[error("I/O error during extraction at '{path}': {message}")]
    IoError { path: PathBuf, message: String },
}

/// Helper guard ensuring the temporary extraction folder is cleaned up if extraction fails.
struct TempDirGuard {
    path: PathBuf,
    committed: bool,
}

impl TempDirGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if !self.committed && self.path.exists() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Safely extract a zip archive to `destination_dir` adhering to all security limits.
///
/// If `strip_single_root` is true and every entry in the archive resides under a single
/// common top-level folder (e.g. `node-v22.11.0-win-x64/`), that folder is stripped
/// so the contents are unpacked directly into `destination_dir`.
pub fn extract_zip_safely(
    archive_path: &Path,
    destination_dir: &Path,
    limits: ExtractionLimits,
    strip_single_root: bool,
) -> Result<PathBuf, ExtractorError> {
    let file = File::open(archive_path).map_err(|e| ExtractorError::ArchiveOpenFailed {
        path: archive_path.to_path_buf(),
        message: e.to_string(),
    })?;

    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| ExtractorError::ZipFormatError(e.to_string()))?;

    let dest_parent = destination_dir.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dest_parent).map_err(|e| ExtractorError::IoError {
        path: dest_parent.to_path_buf(),
        message: e.to_string(),
    })?;

    // Create an isolated staging directory in dest_parent
    let rand_suffix: u64 = {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345)
    };
    let dest_name = destination_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("extracted");
    let temp_staging_dir = dest_parent.join(format!(".tmp-extract-{}-{}", dest_name, rand_suffix));

    fs::create_dir_all(&temp_staging_dir).map_err(|e| ExtractorError::IoError {
        path: temp_staging_dir.clone(),
        message: e.to_string(),
    })?;

    let mut guard = TempDirGuard::new(temp_staging_dir.clone());

    // 1. Detect common top-level directory if strip_single_root is enabled
    let mut common_prefix: Option<String> = None;
    if strip_single_root && !archive.is_empty() {
        let mut candidate_prefix: Option<String> = None;
        let mut all_match = true;

        for i in 0..archive.len() {
            let entry = archive
                .by_index(i)
                .map_err(|e| ExtractorError::ZipFormatError(e.to_string()))?;
            let name = entry
                .name()
                .map_err(|e| ExtractorError::ZipFormatError(e.to_string()))?
                .replace('\\', "/");
            let first_seg = match name.split('/').next() {
                Some(s) if !s.is_empty() => s,
                _ => {
                    all_match = false;
                    break;
                }
            };

            match candidate_prefix {
                None => candidate_prefix = Some(first_seg.to_string()),
                Some(ref p) if p == first_seg => continue,
                _ => {
                    all_match = false;
                    break;
                }
            }
        }

        if all_match && candidate_prefix.is_some() {
            common_prefix = candidate_prefix.map(|p| format!("{}/", p));
        }
    }

    // 2. Iterate through entries, validate paths, and stream decompress with limits
    let mut seen_entries = HashSet::new();
    let mut total_files: usize = 0;
    let mut total_bytes: u64 = 0;

    for i in 0..archive.len() {
        total_files = total_files.saturating_add(1);
        if total_files > limits.max_files {
            return Err(ExtractorError::FileCountLimitExceeded(limits.max_files));
        }

        let mut entry = archive
            .by_index(i)
            .map_err(|e| ExtractorError::ZipFormatError(e.to_string()))?;

        let raw_name = entry
            .name()
            .map_err(|e| ExtractorError::ZipFormatError(e.to_string()))?
            .to_string();

        // Check for symlinks/hardlinks
        let is_symlink = entry.is_symlink()
            || entry
                .unix_mode()
                .map(|mode| (mode & 0o170000) == 0o120000)
                .unwrap_or(false);

        // Strip common prefix if applicable
        let path_to_validate = if let Some(ref prefix) = common_prefix {
            if raw_name == *prefix || raw_name == prefix.trim_end_matches('/') {
                // Root folder itself, skip
                continue;
            } else if let Some(stripped) = raw_name.strip_prefix(prefix) {
                stripped
            } else {
                &raw_name
            }
        } else {
            &raw_name
        };

        if path_to_validate.trim().is_empty() {
            continue;
        }

        let clean_relative_path =
            validate_archive_entry_path(path_to_validate, is_symlink, false, &mut seen_entries)?;

        let target_entry_path = temp_staging_dir.join(&clean_relative_path);

        if entry.is_dir() || raw_name.ends_with('/') || raw_name.ends_with('\\') {
            fs::create_dir_all(&target_entry_path).map_err(|e| ExtractorError::IoError {
                path: target_entry_path.clone(),
                message: e.to_string(),
            })?;
        } else {
            if let Some(parent) = target_entry_path.parent() {
                fs::create_dir_all(parent).map_err(|e| ExtractorError::IoError {
                    path: parent.to_path_buf(),
                    message: e.to_string(),
                })?;
            }

            let mut out_file =
                File::create(&target_entry_path).map_err(|e| ExtractorError::IoError {
                    path: target_entry_path.clone(),
                    message: e.to_string(),
                })?;

            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = entry
                    .read(&mut buf)
                    .map_err(|e| ExtractorError::ZipFormatError(e.to_string()))?;
                if n == 0 {
                    break;
                }

                total_bytes = total_bytes.saturating_add(n as u64);
                if total_bytes > limits.max_bytes {
                    return Err(ExtractorError::SizeLimitExceeded {
                        limit: limits.max_bytes,
                        actual: total_bytes,
                    });
                }

                out_file
                    .write_all(&buf[..n])
                    .map_err(|e| ExtractorError::IoError {
                        path: target_entry_path.clone(),
                        message: e.to_string(),
                    })?;
            }

            out_file.flush().map_err(|e| ExtractorError::IoError {
                path: target_entry_path.clone(),
                message: e.to_string(),
            })?;
        }
    }

    // 3. Finalize: Atomic move to target destination
    if destination_dir.exists() {
        let _ = fs::remove_dir_all(destination_dir);
    }

    fs::rename(&temp_staging_dir, destination_dir).map_err(|e| ExtractorError::IoError {
        path: destination_dir.to_path_buf(),
        message: e.to_string(),
    })?;

    guard.commit();
    Ok(destination_dir.to_path_buf())
}
