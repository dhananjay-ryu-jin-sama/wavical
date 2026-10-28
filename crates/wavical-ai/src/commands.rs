#![forbid(unsafe_code)]

use crate::catalog::{ModelCatalog, ModelKind, default_catalog};
use crate::downloader::{DownloadError, DownloadOptions, ProgressCallback, download_model};
use crate::gguf::validate_gguf_header;
use crate::license::{LicenseError, check_and_prompt_license};
use crate::sha256::sha256_file;
use crate::storage::resolve_model_file_path;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// High-level model management command errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error("model '{0}' was not found in the model catalog")]
    ModelNotFound(String),

    #[error("model '{0}' is not installed")]
    ModelNotInstalled(String),

    #[error(
        "SHA-256 verification failed for '{model_id}': expected '{expected}', computed '{actual}'"
    )]
    HashMismatch {
        model_id: String,
        expected: String,
        actual: String,
    },

    #[error("GGUF header check failed for '{model_id}': {message}")]
    GgufHeaderFailed { model_id: String, message: String },

    #[error("license check failed: {0}")]
    LicenseError(#[from] LicenseError),

    #[error("download failed: {0}")]
    DownloadError(#[from] DownloadError),

    #[error("I/O error: {0}")]
    IoError(String),
}

/// Information returned for `wavical model list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelListInfo {
    pub id: String,
    pub name: String,
    pub kind: ModelKind,
    pub size_bytes: u64,
    pub license_name: String,
    pub installed: bool,
    pub path: Option<PathBuf>,
}

/// Retrieve status list of all catalog models against the local cache.
pub fn list_models(models_dir: &Path, catalog: Option<&ModelCatalog>) -> Vec<ModelListInfo> {
    let cat = match catalog {
        Some(c) => c,
        None => default_catalog(),
    };
    cat.entries
        .iter()
        .map(|entry| {
            let target = resolve_model_file_path(models_dir, &entry.file_name).ok();
            let installed = target.as_ref().map(|p| p.is_file()).unwrap_or(false);

            ModelListInfo {
                id: entry.id.clone(),
                name: entry.name.clone(),
                kind: entry.kind,
                size_bytes: entry.size_bytes,
                license_name: entry.license_name.clone(),
                installed,
                path: if installed { target } else { None },
            }
        })
        .collect()
}

/// Execute `wavical model pull <id>`.
#[allow(clippy::too_many_arguments)]
pub fn pull_model(
    model_id: &str,
    models_dir: &Path,
    data_dir: &Path,
    flag_accept_license: Option<&str>,
    options: &DownloadOptions,
    on_progress: Option<ProgressCallback>,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
    catalog: Option<&ModelCatalog>,
) -> Result<PathBuf, CommandError> {
    let cat = match catalog {
        Some(c) => c,
        None => default_catalog(),
    };
    let entry = cat
        .find_by_id(model_id)
        .ok_or_else(|| CommandError::ModelNotFound(model_id.to_string()))?;

    // Check if already installed and valid
    if let Ok(target) = resolve_model_file_path(models_dir, &entry.file_name)
        && target.is_file()
        && let Ok(hash) = sha256_file(&target)
        && hash == entry.sha256
    {
        writeln!(
            writer,
            "Model '{}' is already installed and verified at: {}",
            model_id,
            target.display()
        )
        .map_err(|e| CommandError::IoError(e.to_string()))?;
        return Ok(target);
    }

    // License confirmation
    check_and_prompt_license(
        entry,
        data_dir,
        cat.version,
        flag_accept_license,
        reader,
        writer,
    )?;

    // Download model
    let target = download_model(entry, models_dir, options, on_progress)?;
    writeln!(
        writer,
        "Successfully downloaded and verified '{}' -> {}",
        model_id,
        target.display()
    )
    .map_err(|e| CommandError::IoError(e.to_string()))?;

    Ok(target)
}

/// Execute `wavical model remove <id>`.
pub fn remove_model(
    model_id: &str,
    models_dir: &Path,
    catalog: Option<&ModelCatalog>,
) -> Result<PathBuf, CommandError> {
    let cat = match catalog {
        Some(c) => c,
        None => default_catalog(),
    };
    let entry = cat
        .find_by_id(model_id)
        .ok_or_else(|| CommandError::ModelNotFound(model_id.to_string()))?;

    let target = resolve_model_file_path(models_dir, &entry.file_name)
        .map_err(|e| CommandError::IoError(e.to_string()))?;

    if !target.is_file() {
        return Err(CommandError::ModelNotInstalled(model_id.to_string()));
    }

    fs::remove_file(&target).map_err(|e| CommandError::IoError(e.to_string()))?;
    Ok(target)
}

/// Execute `wavical model verify <id>`.
pub fn verify_model(
    model_id: &str,
    models_dir: &Path,
    catalog: Option<&ModelCatalog>,
) -> Result<PathBuf, CommandError> {
    let cat = match catalog {
        Some(c) => c,
        None => default_catalog(),
    };
    let entry = cat
        .find_by_id(model_id)
        .ok_or_else(|| CommandError::ModelNotFound(model_id.to_string()))?;

    let target = resolve_model_file_path(models_dir, &entry.file_name)
        .map_err(|e| CommandError::IoError(e.to_string()))?;

    if !target.is_file() {
        return Err(CommandError::ModelNotInstalled(model_id.to_string()));
    }

    // Recompute SHA-256
    let hash = sha256_file(&target).map_err(|e| CommandError::IoError(e.to_string()))?;
    if hash != entry.sha256 {
        return Err(CommandError::HashMismatch {
            model_id: model_id.to_string(),
            expected: entry.sha256.clone(),
            actual: hash,
        });
    }

    // GGUF header check
    if let Err(e) = validate_gguf_header(&target) {
        return Err(CommandError::GgufHeaderFailed {
            model_id: model_id.to_string(),
            message: e.to_string(),
        });
    }

    Ok(target)
}
