#![forbid(unsafe_code)]

use crate::catalog::ModelCatalogEntry;
use crate::storage::{StorageError, is_license_accepted, record_license_acceptance};
use std::io::{BufRead, Write};
use std::path::Path;
use thiserror::Error;

/// License acknowledgement errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LicenseError {
    #[error("license mismatch: expected exact license id '{expected}', got '{provided}'")]
    LicenseMismatch { expected: String, provided: String },

    #[error("license acceptance rejected: {0}")]
    ConfirmationAborted(String),

    #[error("failed to persist license record: {0}")]
    StorageError(#[from] StorageError),

    #[error("I/O error during license prompt: {0}")]
    IoError(String),
}

/// Format human-readable size for display.
pub fn format_size_human(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Execute the license acknowledgement flow.
///
/// Returns `Ok(true)` if license was already accepted, or `Ok(false)` if newly accepted.
pub fn check_and_prompt_license(
    entry: &ModelCatalogEntry,
    data_dir: &Path,
    catalog_version: u32,
    flag_accept_license: Option<&str>,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<bool, LicenseError> {
    if is_license_accepted(data_dir, &entry.id, &entry.license_name) {
        return Ok(true);
    }

    if let Some(flag_lic) = flag_accept_license {
        if flag_lic == entry.license_name {
            record_license_acceptance(data_dir, &entry.id, &entry.license_name, catalog_version)?;
            return Ok(false);
        } else {
            return Err(LicenseError::LicenseMismatch {
                expected: entry.license_name.clone(),
                provided: flag_lic.to_string(),
            });
        }
    }

    // Interactive terminal prompt
    writeln!(
        writer,
        "================================================================================"
    )
    .map_err(|e| LicenseError::IoError(e.to_string()))?;
    writeln!(writer, "Model:   {} ({})", entry.name, entry.id)
        .map_err(|e| LicenseError::IoError(e.to_string()))?;
    writeln!(
        writer,
        "Size:    {} ({} bytes)",
        format_size_human(entry.size_bytes),
        entry.size_bytes
    )
    .map_err(|e| LicenseError::IoError(e.to_string()))?;
    writeln!(
        writer,
        "License: {} ({})",
        entry.license_name, entry.license_url
    )
    .map_err(|e| LicenseError::IoError(e.to_string()))?;
    writeln!(writer).map_err(|e| LicenseError::IoError(e.to_string()))?;
    writeln!(
        writer,
        "Wavical does not provide or endorse this model. You are responsible for its license."
    )
    .map_err(|e| LicenseError::IoError(e.to_string()))?;
    writeln!(writer).map_err(|e| LicenseError::IoError(e.to_string()))?;
    write!(
        writer,
        "To accept the license and continue, type the model id '{}': ",
        entry.id
    )
    .map_err(|e| LicenseError::IoError(e.to_string()))?;
    writer
        .flush()
        .map_err(|e| LicenseError::IoError(e.to_string()))?;

    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| LicenseError::IoError(e.to_string()))?;

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(LicenseError::ConfirmationAborted(
            "empty input; confirmation aborted".to_string(),
        ));
    }

    if trimmed != entry.id {
        return Err(LicenseError::ConfirmationAborted(format!(
            "input '{}' did not match required model id '{}'",
            trimmed, entry.id
        )));
    }

    record_license_acceptance(data_dir, &entry.id, &entry.license_name, catalog_version)?;
    writeln!(writer, "License accepted and recorded for {}.\n", entry.id)
        .map_err(|e| LicenseError::IoError(e.to_string()))?;

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelKind;
    use std::io::Cursor;
    use tempfile::tempdir;

    fn sample_entry() -> ModelCatalogEntry {
        ModelCatalogEntry {
            id: "qwen2.5-1.5b-instruct".to_string(),
            name: "Qwen2.5-1.5B-Instruct".to_string(),
            kind: ModelKind::Chat,
            file_name: "qwen2.5-1.5b-instruct-q4_k_m.gguf".to_string(),
            url: "https://huggingface.co/model.gguf".to_string(),
            size_bytes: 1_000_000,
            sha256: "0".repeat(64),
            license_name: "Apache-2.0".to_string(),
            license_url: "https://example.com/license".to_string(),
            min_ram_bytes: 2_000_000,
            notes: None,
        }
    }

    #[test]
    fn test_license_interactive_prompt_success() {
        let temp = tempdir().unwrap();
        let entry = sample_entry();

        let mut input = Cursor::new(b"qwen2.5-1.5b-instruct\n");
        let mut output = Vec::new();

        let res = check_and_prompt_license(&entry, temp.path(), 1, None, &mut input, &mut output);
        assert!(!res.unwrap());
        assert!(is_license_accepted(
            temp.path(),
            &entry.id,
            &entry.license_name
        ));

        let out_str = String::from_utf8(output).unwrap();
        assert!(out_str.contains(
            "Wavical does not provide or endorse this model. You are responsible for its license."
        ));
        assert!(out_str.contains("License accepted and recorded"));
    }

    #[test]
    fn test_license_interactive_prompt_rejections() {
        let temp = tempdir().unwrap();
        let entry = sample_entry();

        // 1. Empty input
        let mut empty_input = Cursor::new(b"\n");
        let mut output = Vec::new();
        let err =
            check_and_prompt_license(&entry, temp.path(), 1, None, &mut empty_input, &mut output)
                .unwrap_err();
        assert!(matches!(err, LicenseError::ConfirmationAborted(_)));

        // 2. Whitespace only
        let mut ws_input = Cursor::new(b"   \t  \n");
        let mut output = Vec::new();
        let err =
            check_and_prompt_license(&entry, temp.path(), 1, None, &mut ws_input, &mut output)
                .unwrap_err();
        assert!(matches!(err, LicenseError::ConfirmationAborted(_)));

        // 3. Wrong model id
        let mut wrong_input = Cursor::new(b"wrong-model-id\n");
        let mut output = Vec::new();
        let err =
            check_and_prompt_license(&entry, temp.path(), 1, None, &mut wrong_input, &mut output)
                .unwrap_err();
        assert!(matches!(err, LicenseError::ConfirmationAborted(_)));
    }

    #[test]
    fn test_license_flag_accept_license_flow() {
        let temp = tempdir().unwrap();
        let entry = sample_entry();

        // Exact match flag passes
        let mut dummy_in = Cursor::new(b"");
        let mut dummy_out = Vec::new();
        let res = check_and_prompt_license(
            &entry,
            temp.path(),
            1,
            Some("Apache-2.0"),
            &mut dummy_in,
            &mut dummy_out,
        );
        assert!(!res.unwrap());
        assert!(is_license_accepted(
            temp.path(),
            &entry.id,
            &entry.license_name
        ));

        // Subsequent call reports already accepted
        let res_subsequent =
            check_and_prompt_license(&entry, temp.path(), 1, None, &mut dummy_in, &mut dummy_out);
        assert!(res_subsequent.unwrap());

        // Mismatched flag fails
        let temp2 = tempdir().unwrap();
        let err = check_and_prompt_license(
            &entry,
            temp2.path(),
            1,
            Some("MIT"),
            &mut dummy_in,
            &mut dummy_out,
        )
        .unwrap_err();
        assert!(matches!(err, LicenseError::LicenseMismatch { .. }));
    }
}
