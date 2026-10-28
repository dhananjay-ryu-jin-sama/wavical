#![forbid(unsafe_code)]

use crate::catalog::validate_safe_filename;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Storage errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorageError {
    #[error("invalid file name '{0}': path traversal and invalid characters are forbidden")]
    InvalidFileName(String),

    #[error("failed to create directory '{path}': {message}")]
    DirCreateFailed { path: PathBuf, message: String },

    #[error("I/O error at '{path}': {message}")]
    IoError { path: PathBuf, message: String },

    #[error("failed to parse licenses file '{path}': {message}")]
    LicenseParseError { path: PathBuf, message: String },
}

/// Return the Wavical data root directory (`%LOCALAPPDATA%\Wavical` on Windows).
/// Can be overridden with the `WAVICAL_DATA_DIR` environment variable.
pub fn wavical_data_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("WAVICAL_DATA_DIR")
        && !override_dir.trim().is_empty()
    {
        return PathBuf::from(override_dir);
    }

    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA")
            && !local_app_data.trim().is_empty()
        {
            return PathBuf::from(local_app_data).join("Wavical");
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME")
            && !xdg_data.trim().is_empty()
        {
            return PathBuf::from(xdg_data).join("Wavical");
        }
        if let Ok(home) = std::env::var("HOME")
            && !home.trim().is_empty()
        {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("Wavical");
        }
    }

    std::env::temp_dir().join("Wavical")
}

/// Return the models cache directory (`%LOCALAPPDATA%\Wavical\models` on Windows).
/// Can be overridden with the `WAVICAL_MODELS_DIR` environment variable.
pub fn wavical_models_dir() -> PathBuf {
    if let Ok(override_dir) = std::env::var("WAVICAL_MODELS_DIR")
        && !override_dir.trim().is_empty()
    {
        return PathBuf::from(override_dir);
    }

    wavical_data_dir().join("models")
}

/// Resolve the target model file path inside the models folder,
/// strictly prohibiting any directory traversal.
pub fn resolve_model_file_path(
    models_dir: &Path,
    file_name: &str,
) -> Result<PathBuf, StorageError> {
    if !validate_safe_filename(file_name) {
        return Err(StorageError::InvalidFileName(file_name.to_string()));
    }

    // Additional path traversal check
    if file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
        || file_name.contains(':')
    {
        return Err(StorageError::InvalidFileName(file_name.to_string()));
    }

    let target = models_dir.join(file_name);
    Ok(target)
}

/// A recorded model license acceptance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LicenseAcceptanceRecord {
    /// Timestamp of acceptance in Unix epoch seconds.
    pub timestamp: u64,
    /// Catalog identifier of the model.
    pub model_id: String,
    /// License identifier name (e.g. 'Apache-2.0', 'MIT').
    pub license_id: String,
    /// Version of the catalog at acceptance time.
    pub catalog_version: u32,
}

/// Return the path to the license acceptances store file.
pub fn licenses_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("licenses.json")
}

/// Load all recorded license acceptances.
pub fn load_license_acceptances(
    data_dir: &Path,
) -> Result<Vec<LicenseAcceptanceRecord>, StorageError> {
    let path = licenses_file_path(data_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&path).map_err(|e| StorageError::IoError {
        path: path.clone(),
        message: e.to_string(),
    })?;

    if content.trim().is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str(&content).map_err(|e| StorageError::LicenseParseError {
        path,
        message: e.to_string(),
    })
}

/// Check if a specific model and license has already been accepted.
pub fn is_license_accepted(data_dir: &Path, model_id: &str, license_id: &str) -> bool {
    let Ok(records) = load_license_acceptances(data_dir) else {
        return false;
    };

    records
        .iter()
        .any(|r| r.model_id == model_id && r.license_id == license_id)
}

/// Record a license acceptance atomically.
pub fn record_license_acceptance(
    data_dir: &Path,
    model_id: &str,
    license_id: &str,
    catalog_version: u32,
) -> Result<(), StorageError> {
    fs::create_dir_all(data_dir).map_err(|e| StorageError::DirCreateFailed {
        path: data_dir.to_path_buf(),
        message: e.to_string(),
    })?;

    let mut records = load_license_acceptances(data_dir).unwrap_or_default();

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    records.retain(|r| !(r.model_id == model_id && r.license_id == license_id));
    records.push(LicenseAcceptanceRecord {
        timestamp: now_secs,
        model_id: model_id.to_string(),
        license_id: license_id.to_string(),
        catalog_version,
    });

    let target_file = licenses_file_path(data_dir);
    let temp_file = data_dir.join(format!("licenses.json.tmp.{}", std::process::id()));

    let json = serde_json::to_string_pretty(&records).map_err(|e| StorageError::IoError {
        path: target_file.clone(),
        message: e.to_string(),
    })?;

    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_file)
            .map_err(|e| StorageError::IoError {
                path: temp_file.clone(),
                message: e.to_string(),
            })?;
        file.write_all(json.as_bytes())
            .map_err(|e| StorageError::IoError {
                path: temp_file.clone(),
                message: e.to_string(),
            })?;
        file.flush().map_err(|e| StorageError::IoError {
            path: temp_file.clone(),
            message: e.to_string(),
        })?;
    }

    fs::rename(&temp_file, &target_file).map_err(|e| StorageError::IoError {
        path: target_file.clone(),
        message: e.to_string(),
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_resolve_model_file_path_traversal_rejections() {
        let dir = Path::new("C:\\AppData\\Local\\Wavical\\models");

        // Valid file name
        let valid = resolve_model_file_path(dir, "qwen2.5.gguf").unwrap();
        assert_eq!(valid, dir.join("qwen2.5.gguf"));

        // Traversal attempts
        assert!(resolve_model_file_path(dir, "../evil.gguf").is_err());
        assert!(resolve_model_file_path(dir, "..\\evil.gguf").is_err());
        assert!(resolve_model_file_path(dir, "sub/evil.gguf").is_err());
        assert!(resolve_model_file_path(dir, "sub\\evil.gguf").is_err());
        assert!(resolve_model_file_path(dir, "C:evil.gguf").is_err());
        assert!(resolve_model_file_path(dir, "evil.bin").is_err()); // Not .gguf
    }

    #[test]
    fn test_license_acceptance_persistence() {
        let temp = tempdir().unwrap();
        let data_dir = temp.path();

        assert!(!is_license_accepted(
            data_dir,
            "qwen2.5-1.5b-instruct",
            "Apache-2.0"
        ));

        record_license_acceptance(data_dir, "qwen2.5-1.5b-instruct", "Apache-2.0", 1).unwrap();

        assert!(is_license_accepted(
            data_dir,
            "qwen2.5-1.5b-instruct",
            "Apache-2.0"
        ));
        assert!(!is_license_accepted(
            data_dir,
            "qwen2.5-1.5b-instruct",
            "MIT"
        ));
        assert!(!is_license_accepted(data_dir, "other-model", "Apache-2.0"));
    }
}
