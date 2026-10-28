#![forbid(unsafe_code)]

use std::fs::File;
use std::io::Read;
use std::path::Path;
use thiserror::Error;

/// Expected GGUF magic bytes: ASCII `GGUF` (0x47, 0x47, 0x55, 0x46).
pub const GGUF_MAGIC: &[u8; 4] = b"GGUF";

/// Errors occurring during GGUF header sanity validation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GgufError {
    #[error(
        "file is too small to contain a valid GGUF header (minimum 8 bytes required, got {size})"
    )]
    FileTooSmall { size: u64 },

    #[error("invalid GGUF magic bytes: expected 'GGUF', got {0:?}")]
    InvalidMagic(String),

    #[error("unsupported GGUF version {0}: Wavical supports GGUF versions 1, 2, and 3")]
    UnsupportedVersion(u32),

    #[error("I/O error reading GGUF header: {0}")]
    IoError(String),
}

/// Validate the GGUF header from an in-memory byte slice (first >=8 bytes).
pub fn validate_gguf_header_bytes(bytes: &[u8]) -> Result<u32, GgufError> {
    if bytes.len() < 8 {
        return Err(GgufError::FileTooSmall {
            size: bytes.len() as u64,
        });
    }

    if &bytes[0..4] != GGUF_MAGIC {
        let magic_repr = String::from_utf8_lossy(&bytes[0..4]).to_string();
        return Err(GgufError::InvalidMagic(magic_repr));
    }

    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if !(1..=3).contains(&version) {
        return Err(GgufError::UnsupportedVersion(version));
    }

    Ok(version)
}

/// Validate that a file on disk starts with a valid GGUF magic header and supported version.
pub fn validate_gguf_header(path: &Path) -> Result<u32, GgufError> {
    let mut file = File::open(path).map_err(|e| GgufError::IoError(e.to_string()))?;
    let mut header = [0u8; 8];
    let n = file
        .read(&mut header)
        .map_err(|e| GgufError::IoError(e.to_string()))?;

    if n < 8 {
        return Err(GgufError::FileTooSmall { size: n as u64 });
    }

    validate_gguf_header_bytes(&header)
}

/// Helper for tests to create a valid minimal fake GGUF file buffer.
pub fn create_fake_gguf_bytes(version: u32, payload_size: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + payload_size);
    bytes.extend_from_slice(GGUF_MAGIC);
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.resize(8 + payload_size, 0x42);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_gguf_header_v2_and_v3() {
        let v2 = create_fake_gguf_bytes(2, 64);
        assert_eq!(validate_gguf_header_bytes(&v2).unwrap(), 2);

        let v3 = create_fake_gguf_bytes(3, 128);
        assert_eq!(validate_gguf_header_bytes(&v3).unwrap(), 3);
    }

    #[test]
    fn test_invalid_magic_bytes() {
        let mut bad_magic = create_fake_gguf_bytes(3, 16);
        bad_magic[0] = b'X';
        assert!(matches!(
            validate_gguf_header_bytes(&bad_magic).unwrap_err(),
            GgufError::InvalidMagic(_)
        ));
    }

    #[test]
    fn test_unsupported_version() {
        let bad_ver = create_fake_gguf_bytes(99, 16);
        assert_eq!(
            validate_gguf_header_bytes(&bad_ver).unwrap_err(),
            GgufError::UnsupportedVersion(99)
        );
    }

    #[test]
    fn test_file_too_small() {
        let tiny = b"GGU";
        assert!(matches!(
            validate_gguf_header_bytes(tiny).unwrap_err(),
            GgufError::FileTooSmall { .. }
        ));
    }
}
