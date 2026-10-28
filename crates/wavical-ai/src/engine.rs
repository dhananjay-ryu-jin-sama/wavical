#![forbid(unsafe_code)]

//! Engine provenance, pinning, and binary verification.
//!
//! Values are strictly copied from upstream official releases recorded in
//! `docs/WAVICAL_PHASE_3_DECISIONS.md`. The binary is never committed to git.
//! Its SHA-256 digest is verified before EVERY launch.

use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::sha256::sha256_file;
use crate::storage::wavical_data_dir;

/// Pinned official llama.cpp release tag approved in WP1.
pub const PINNED_ENGINE_TAG: &str = "b11669";

/// Target upstream commit SHA from ggml-org/llama.cpp.
pub const PINNED_ENGINE_COMMIT: &str = "620d2225cc436ab0b001e3200e590e3b286ef9ab";

/// Pinned zip asset name for Windows CPU x64 release.
pub const PINNED_ENGINE_ASSET_NAME: &str = "llama-b11669-bin-win-cpu-x64.zip";

/// Pinned SHA-256 digest of the upstream zip asset.
pub const PINNED_ENGINE_ASSET_SHA256: &str =
    "37b3efcf962847ca869745fd4ccf7f5e7e8dacfeff9fa3523b4ca8cdc66d632b";

/// Default pinned SHA-256 digest of the extracted `llama-server.exe` binary.
/// Can be overridden via `WAVICAL_ENGINE_EXPECTED_HASH` for testing.
pub const PINNED_ENGINE_EXE_SHA256: &str =
    "601bf4e9680327f1c1f7743ea415b3a4a7536d72491fa591f0bb6c8a002ebfa7";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    #[error("engine binary not found at '{0}'")]
    BinaryNotFound(PathBuf),

    #[error(
        "engine binary SHA-256 mismatch: expected '{expected}', found '{actual}'. Launch refused."
    )]
    HashMismatch { expected: String, actual: String },

    #[error("I/O error reading engine binary '{path}': {message}")]
    IoError { path: PathBuf, message: String },
}

/// Return the path to the engine directory (`%LOCALAPPDATA%\Wavical\engine`).
pub fn wavical_engine_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WAVICAL_ENGINE_DIR")
        && !dir.trim().is_empty()
    {
        return PathBuf::from(dir);
    }
    wavical_data_dir().join("engine")
}

/// Return the resolved path to the engine executable.
/// Can be overridden with `WAVICAL_ENGINE_PATH` for tests or custom installations.
pub fn resolve_engine_path() -> PathBuf {
    if let Ok(path) = std::env::var("WAVICAL_ENGINE_PATH")
        && !path.trim().is_empty()
    {
        return PathBuf::from(path);
    }

    #[cfg(windows)]
    let exe_name = "llama-server.exe";
    #[cfg(not(windows))]
    let exe_name = "llama-server";

    wavical_engine_dir().join(exe_name)
}

/// Return the expected SHA-256 hash for the engine binary.
/// Uses `WAVICAL_ENGINE_EXPECTED_HASH` if set (for testing), otherwise the pinned hash.
pub fn expected_engine_hash() -> String {
    if let Ok(hash) = std::env::var("WAVICAL_ENGINE_EXPECTED_HASH")
        && !hash.trim().is_empty()
    {
        return hash.trim().to_ascii_lowercase();
    }
    PINNED_ENGINE_EXE_SHA256.to_string()
}

/// Verify that the engine binary exists and its SHA-256 matches the expected digest.
///
/// MUST be called before every engine process launch. If the binary is missing
/// or tampered with, launch is strictly refused.
pub fn verify_engine_binary(engine_path: &Path, expected_hash: &str) -> Result<(), EngineError> {
    if !engine_path.is_file() {
        return Err(EngineError::BinaryNotFound(engine_path.to_path_buf()));
    }

    let actual_hash = sha256_file(engine_path).map_err(|e| EngineError::IoError {
        path: engine_path.to_path_buf(),
        message: e.to_string(),
    })?;

    if !actual_hash.eq_ignore_ascii_case(expected_hash) {
        return Err(EngineError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::sha256_bytes;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_engine_missing_binary_returns_not_found() {
        let temp = tempdir().unwrap();
        let missing = temp.path().join("llama-server.exe");
        let err = verify_engine_binary(&missing, "0".repeat(64).as_str()).unwrap_err();
        assert_eq!(err, EngineError::BinaryNotFound(missing));
    }

    #[test]
    fn test_engine_hash_mismatch_refuses_launch() {
        let temp = tempdir().unwrap();
        let fake_bin = temp.path().join("llama-server.exe");
        fs::write(&fake_bin, b"malicious or altered executable content").unwrap();

        let expected = "0".repeat(64);
        let err = verify_engine_binary(&fake_bin, &expected).unwrap_err();
        match err {
            EngineError::HashMismatch {
                expected: exp,
                actual,
            } => {
                assert_eq!(exp, expected);
                assert_eq!(
                    actual,
                    sha256_bytes(b"malicious or altered executable content")
                );
            }
            other => panic!("expected HashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn test_engine_hash_match_succeeds() {
        let temp = tempdir().unwrap();
        let valid_bin = temp.path().join("llama-server.exe");
        let payload = b"legitimate engine binary bytes";
        fs::write(&valid_bin, payload).unwrap();

        let expected = sha256_bytes(payload);
        assert!(verify_engine_binary(&valid_bin, &expected).is_ok());
    }
}
