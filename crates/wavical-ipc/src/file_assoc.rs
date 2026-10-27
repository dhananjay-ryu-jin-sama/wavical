#![forbid(unsafe_code)]

use thiserror::Error;

/// Maximum length for a file extension (excluding dot).
pub const MAX_EXTENSION_CHARS: usize = 10;

/// Parsed and verified Windows registry key paths for a file association.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAssociationKeys {
    /// Normalized extension (e.g. "wavdemo").
    pub extension: String,
    /// Registry key for the extension: `Software\Classes\.{ext}`.
    pub ext_key: String,
    /// Registry key for OpenWithProgids: `Software\Classes\.{ext}\OpenWithProgids`.
    pub open_with_progids_key: String,
    /// Generated ProgID: `Wavical.{appId}`.
    pub prog_id: String,
    /// Registry key for the ProgID: `Software\Classes\{prog_id}`.
    pub prog_id_key: String,
    /// Registry key for open command: `Software\Classes\{prog_id}\shell\open\command`.
    pub open_command_key: String,
}

/// Errors occurring during file association extension and key validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FileAssocValidationError {
    #[error("file extension cannot be empty")]
    EmptyExtension,

    #[error("file extension '{0}' must be 1 to {MAX_EXTENSION_CHARS} characters, got {1}")]
    ExtensionLength(String, usize),

    #[error(
        "file extension '{0}' contains invalid characters (must strictly match ^[a-z0-9]{{1,10}}$)"
    )]
    InvalidCharacters(String),

    #[error("file extension '{0}' must not start with a dot '.'")]
    LeadingDot(String),

    #[error("application ID '{0}' is invalid for file association registration")]
    InvalidAppId(String),
}

/// Validate that a file extension strictly matches `^[a-z0-9]{1,10}$`.
pub fn validate_file_extension(ext: &str) -> Result<&str, FileAssocValidationError> {
    if ext.is_empty() {
        return Err(FileAssocValidationError::EmptyExtension);
    }
    if ext.starts_with('.') {
        return Err(FileAssocValidationError::LeadingDot(ext.to_string()));
    }
    let char_count = ext.chars().count();
    if !(1..=MAX_EXTENSION_CHARS).contains(&char_count) {
        return Err(FileAssocValidationError::ExtensionLength(
            ext.to_string(),
            char_count,
        ));
    }
    for c in ext.chars() {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() {
            return Err(FileAssocValidationError::InvalidCharacters(ext.to_string()));
        }
    }
    Ok(ext)
}

/// Validate application ID conforms to safe identifier rules `^[a-zA-Z0-9_-]{1,64}$`.
pub fn validate_assoc_app_id(app_id: &str) -> Result<&str, FileAssocValidationError> {
    if app_id.is_empty() || app_id.len() > 64 {
        return Err(FileAssocValidationError::InvalidAppId(app_id.to_string()));
    }
    for c in app_id.chars() {
        if !c.is_ascii_alphanumeric() && c != '-' && c != '_' {
            return Err(FileAssocValidationError::InvalidAppId(app_id.to_string()));
        }
    }
    Ok(app_id)
}

/// Build safe HKCU registry key paths for a validated appId and extension.
/// Ensures no path separators, traversal characters, or registry injection tokens exist in key components.
pub fn build_association_keys(
    app_id: &str,
    ext: &str,
) -> Result<FileAssociationKeys, FileAssocValidationError> {
    let valid_app_id = validate_assoc_app_id(app_id)?;
    let valid_ext = validate_file_extension(ext)?;

    let ext_key = format!(r"Software\Classes\.{valid_ext}");
    let open_with_progids_key = format!(r"Software\Classes\.{valid_ext}\OpenWithProgids");
    let prog_id = format!("Wavical.{valid_app_id}");
    let prog_id_key = format!(r"Software\Classes\{prog_id}");
    let open_command_key = format!(r"Software\Classes\{prog_id}\shell\open\command");

    Ok(FileAssociationKeys {
        extension: valid_ext.to_string(),
        ext_key,
        open_with_progids_key,
        prog_id,
        prog_id_key,
        open_command_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_extensions() {
        assert_eq!(validate_file_extension("txt").unwrap(), "txt");
        assert_eq!(validate_file_extension("wavdemo").unwrap(), "wavdemo");
        assert_eq!(validate_file_extension("a").unwrap(), "a");
        assert_eq!(validate_file_extension("0123456789").unwrap(), "0123456789");
    }

    #[test]
    fn test_invalid_extension_cases() {
        // Leading dot
        assert_eq!(
            validate_file_extension(".txt"),
            Err(FileAssocValidationError::LeadingDot(".txt".into()))
        );

        // Empty
        assert_eq!(
            validate_file_extension(""),
            Err(FileAssocValidationError::EmptyExtension)
        );

        // Uppercase
        assert_eq!(
            validate_file_extension("TXT"),
            Err(FileAssocValidationError::InvalidCharacters("TXT".into()))
        );

        // Too long (11 chars)
        assert_eq!(
            validate_file_extension("toolongext1"),
            Err(FileAssocValidationError::ExtensionLength(
                "toolongext1".into(),
                11
            ))
        );

        // Path separators and traversal
        assert_eq!(
            validate_file_extension("txt/foo"),
            Err(FileAssocValidationError::InvalidCharacters(
                "txt/foo".into()
            ))
        );
        assert_eq!(
            validate_file_extension(r"txt\foo"),
            Err(FileAssocValidationError::InvalidCharacters(
                r"txt\foo".into()
            ))
        );
        assert_eq!(
            validate_file_extension("txt..bar"),
            Err(FileAssocValidationError::InvalidCharacters(
                "txt..bar".into()
            ))
        );

        // Special chars and null bytes
        assert_eq!(
            validate_file_extension("txt\0"),
            Err(FileAssocValidationError::InvalidCharacters("txt\0".into()))
        );
        assert_eq!(
            validate_file_extension("txt;cmd"),
            Err(FileAssocValidationError::InvalidCharacters(
                "txt;cmd".into()
            ))
        );
    }

    #[test]
    fn test_build_association_keys() {
        let keys = build_association_keys("my-app", "wavdemo").unwrap();
        assert_eq!(keys.extension, "wavdemo");
        assert_eq!(keys.ext_key, r"Software\Classes\.wavdemo");
        assert_eq!(
            keys.open_with_progids_key,
            r"Software\Classes\.wavdemo\OpenWithProgids"
        );
        assert_eq!(keys.prog_id, "Wavical.my-app");
        assert_eq!(keys.prog_id_key, r"Software\Classes\Wavical.my-app");
        assert_eq!(
            keys.open_command_key,
            r"Software\Classes\Wavical.my-app\shell\open\command"
        );
    }

    #[test]
    fn test_build_association_keys_injection_rejected() {
        // Traversal in app_id
        assert!(build_association_keys("../evil", "wav").is_err());
        assert!(build_association_keys(r"evil\app", "wav").is_err());
        assert!(build_association_keys("app\0null", "wav").is_err());

        // Traversal in extension
        assert!(build_association_keys("my-app", "../evil").is_err());
        assert!(build_association_keys("my-app", r"evil\ext").is_err());
    }
}
