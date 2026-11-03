#![forbid(unsafe_code)]

//! Backend runtime management, caching, provenance verification, and launch-time hash checking.
//!
//! Cache folder location:
//! `%LOCALAPPDATA%\Wavical\runtimes\<runtime>-<version>-<arch>\`

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use wavical_ai::downloader::{
    DownloadError, DownloadKind, DownloadOptions, download_verified_file,
};
use wavical_protocol::{
    APPROVED_RUNTIMES, RuntimeArch, RuntimeManifestEntry, RuntimeManifestError,
    find_approved_runtime,
};

use crate::extractor::{ExtractionLimits, ExtractorError, extract_zip_safely};

/// Metadata stored in `<runtime_dir>/.wavical-runtime.json` after successful extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMetadata {
    pub runtime: String,
    pub version: String,
    pub arch: String,
    pub executable_name: String,
    pub executable_sha256: String,
    pub installed_at_secs: u64,
}

/// Errors occurring during runtime management operations.
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("manifest error: {0}")]
    Manifest(#[from] RuntimeManifestError),

    #[error("downloader error: {0}")]
    Download(#[from] DownloadError),

    #[error("archive extraction error: {0}")]
    Extraction(#[from] ExtractorError),

    #[error("runtime directory not found: '{0}'")]
    NotFound(PathBuf),

    #[error("runtime executable '{0}' missing from installation")]
    ExecutableMissing(PathBuf),

    #[error("runtime metadata file missing or invalid: '{0}'")]
    MetadataMissing(PathBuf),

    #[error(
        "runtime executable SHA-256 verification failed: expected '{expected}', computed '{actual}'. Launch refused!"
    )]
    HashMismatch { expected: String, actual: String },

    #[error("I/O error at '{path}': {message}")]
    IoError { path: PathBuf, message: String },
}

/// Return the base runtimes cache directory (`%LOCALAPPDATA%\Wavical\runtimes`).
pub fn wavical_runtimes_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WAVICAL_RUNTIMES_DIR")
        && !dir.trim().is_empty()
    {
        return PathBuf::from(dir);
    }

    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA")
            && !local_app_data.trim().is_empty()
        {
            return PathBuf::from(local_app_data)
                .join("Wavical")
                .join("runtimes");
        }
    }

    // Fallback using wavical_ai storage data dir
    wavical_ai::storage::wavical_data_dir().join("runtimes")
}

/// Resolve the exact cache directory for an approved runtime entry.
pub fn cache_dir_for_runtime(entry: &RuntimeManifestEntry) -> PathBuf {
    wavical_runtimes_dir().join(entry.cache_dirname())
}

/// Compute SHA-256 digest of any file on disk.
pub fn compute_sha256(path: &Path) -> Result<String, RuntimeError> {
    crate::sha256::sha256_file(path).map_err(|e| RuntimeError::IoError {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// Verify the runtime executable's SHA-256 hash before launch (Task 6).
///
/// This function MUST be invoked by the backend launcher in WP4 before executing node.exe.
pub fn verify_runtime_executable_hash(
    executable_path: &Path,
    expected_sha256: &str,
) -> Result<(), RuntimeError> {
    if !executable_path.exists() {
        return Err(RuntimeError::ExecutableMissing(
            executable_path.to_path_buf(),
        ));
    }

    let actual_hash = compute_sha256(executable_path)?;
    if !actual_hash.eq_ignore_ascii_case(expected_sha256) {
        return Err(RuntimeError::HashMismatch {
            expected: expected_sha256.to_string(),
            actual: actual_hash,
        });
    }

    Ok(())
}

/// Install an approved runtime into the per-user cache directory.
pub fn install_runtime(
    runtime_name: &str,
    version: &str,
    arch: RuntimeArch,
    options: &DownloadOptions,
) -> Result<PathBuf, RuntimeError> {
    let entry = find_approved_runtime(runtime_name, version, arch)?;
    let target_dir = cache_dir_for_runtime(entry);

    if target_dir.exists() {
        // If already installed and healthy, report success
        let meta_file = target_dir.join(".wavical-runtime.json");
        if meta_file.exists() {
            let exe_path = target_dir.join(entry.executable_name);
            if exe_path.exists() {
                println!(
                    "Runtime {}-{} ({}) is already installed at: {}",
                    entry.runtime,
                    entry.version,
                    entry.arch.as_str(),
                    target_dir.display()
                );
                return Ok(target_dir);
            }
        }
    }

    let runtimes_base = wavical_runtimes_dir();
    fs::create_dir_all(&runtimes_base).map_err(|e| RuntimeError::IoError {
        path: runtimes_base.clone(),
        message: e.to_string(),
    })?;

    // 1. Download to temporary archive
    let archive_name = format!("{}.zip", entry.cache_dirname());
    let temp_archive_path = runtimes_base.join(&archive_name);

    println!(
        "Downloading verified runtime archive: {} ({} bytes)...",
        entry.url, entry.size_bytes
    );

    let downloaded_archive = download_verified_file(
        entry.url,
        &temp_archive_path,
        entry.size_bytes,
        entry.sha256,
        DownloadKind::Runtime,
        options,
        None,
    )?;

    // 2. Extract into target directory using safe extractor
    println!("Extracting runtime safely into cache...");
    let extracted_dir = extract_zip_safely(
        &downloaded_archive,
        &target_dir,
        ExtractionLimits::for_runtime(),
        true, // strip top-level node-v.../ folder
    )?;

    // 3. Clean up the downloaded zip archive
    let _ = fs::remove_file(&downloaded_archive);

    // 4. Verify executable presence and record metadata
    let exe_path = extracted_dir.join(entry.executable_name);
    if !exe_path.exists() {
        let _ = fs::remove_dir_all(&extracted_dir);
        return Err(RuntimeError::ExecutableMissing(exe_path));
    }

    let exe_sha256 = compute_sha256(&exe_path)?;
    let meta = RuntimeMetadata {
        runtime: entry.runtime.to_string(),
        version: entry.version.to_string(),
        arch: entry.arch.as_str().to_string(),
        executable_name: entry.executable_name.to_string(),
        executable_sha256: exe_sha256.clone(),
        installed_at_secs: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };

    let meta_json = serde_json::to_string_pretty(&meta).map_err(|e| RuntimeError::IoError {
        path: extracted_dir.join(".wavical-runtime.json"),
        message: e.to_string(),
    })?;

    fs::write(extracted_dir.join(".wavical-runtime.json"), meta_json).map_err(|e| {
        RuntimeError::IoError {
            path: extracted_dir.join(".wavical-runtime.json"),
            message: e.to_string(),
        }
    })?;

    println!(
        "✓ Successfully installed and verified {} v{} ({}) at: {}",
        entry.runtime,
        entry.version,
        entry.arch.as_str(),
        extracted_dir.display()
    );
    println!(
        "  Executable: {} (SHA-256: {})",
        entry.executable_name, exe_sha256
    );

    Ok(extracted_dir)
}

/// Remove a runtime from cache.
pub fn remove_runtime(
    runtime_name: &str,
    version: &str,
    arch: RuntimeArch,
) -> Result<(), RuntimeError> {
    let entry = find_approved_runtime(runtime_name, version, arch)?;
    let target_dir = cache_dir_for_runtime(entry);

    if !target_dir.exists() {
        return Err(RuntimeError::NotFound(target_dir));
    }

    fs::remove_dir_all(&target_dir).map_err(|e| RuntimeError::IoError {
        path: target_dir.clone(),
        message: e.to_string(),
    })?;

    println!(
        "✓ Successfully removed runtime {}-{} ({}) from: {}",
        entry.runtime,
        entry.version,
        entry.arch.as_str(),
        target_dir.display()
    );
    Ok(())
}

/// Verify integrity of an installed runtime.
pub fn verify_runtime(
    runtime_name: &str,
    version: &str,
    arch: RuntimeArch,
) -> Result<(), RuntimeError> {
    let entry = find_approved_runtime(runtime_name, version, arch)?;
    let target_dir = cache_dir_for_runtime(entry);

    if !target_dir.exists() {
        return Err(RuntimeError::NotFound(target_dir));
    }

    let meta_path = target_dir.join(".wavical-runtime.json");
    if !meta_path.exists() {
        return Err(RuntimeError::MetadataMissing(meta_path));
    }

    let meta_content = fs::read_to_string(&meta_path).map_err(|e| RuntimeError::IoError {
        path: meta_path.clone(),
        message: e.to_string(),
    })?;

    let meta: RuntimeMetadata =
        serde_json::from_str(&meta_content).map_err(|e| RuntimeError::IoError {
            path: meta_path.clone(),
            message: e.to_string(),
        })?;

    let exe_path = target_dir.join(&meta.executable_name);
    verify_runtime_executable_hash(&exe_path, &meta.executable_sha256)?;

    println!(
        "✓ Runtime {}-{} ({}) integrity verified OK at: {}",
        entry.runtime,
        entry.version,
        entry.arch.as_str(),
        target_dir.display()
    );
    println!("  Executable: {} (SHA-256 match)", meta.executable_name);

    Ok(())
}

/// List all approved runtimes and their local installation status.
pub fn list_runtimes(arch: RuntimeArch) {
    println!(
        "Wavical Approved Backend Runtimes (Architecture: {}):",
        arch.as_str()
    );
    println!(
        "{:<10} {:<12} {:<10} {:<15}",
        "RUNTIME", "VERSION", "ARCH", "STATUS"
    );
    println!("{:-<50}", "");

    for entry in APPROVED_RUNTIMES {
        if entry.arch != arch {
            continue;
        }

        let dir = cache_dir_for_runtime(entry);
        let status = if dir.exists() && dir.join(entry.executable_name).exists() {
            "INSTALLED"
        } else {
            "AVAILABLE"
        };

        println!(
            "{:<10} {:<12} {:<10} {:<15}",
            entry.runtime,
            entry.version,
            entry.arch.as_str(),
            status
        );
    }
}

/// Show runtime status and which apps' configs use them if known.
pub fn runtime_status(project_dir: Option<&Path>, arch: RuntimeArch) {
    println!("=== Wavical Runtime Status ===");
    println!("Cache directory: {}", wavical_runtimes_dir().display());
    println!();

    // Check installed runtimes in cache
    let mut installed_any = false;
    for entry in APPROVED_RUNTIMES {
        if entry.arch != arch {
            continue;
        }
        let dir = cache_dir_for_runtime(entry);
        if dir.exists() && dir.join(entry.executable_name).exists() {
            installed_any = true;
            println!(
                "- {}-{} [INSTALLED at {}]",
                entry.runtime,
                entry.version,
                dir.display()
            );
        }
    }

    if !installed_any {
        println!("- No runtimes installed in cache.");
    }
    println!();

    // Check project config if directory provided
    if let Some(dir) = project_dir {
        println!("Checking project at: {}", dir.display());
        match wavical_config::validate_config(dir) {
            Ok(config) => {
                println!("  App ID: {}", config.app_id);
                // In Phase 4, backend config will be added in WP3.
                println!("  Backend status: (configured in wavical.config.js)");
            }
            Err(e) => {
                println!("  (No valid wavical.config.js found in directory: {e})");
            }
        }
    }
}
