#![forbid(unsafe_code)]

//! Pure archive entry path validation and safe extraction invariants.
//!
//! Evaluates archive entry paths without performing any OS-level file operations.
//! Can run safely on Linux and Windows.
//!
//! Invariants enforced:
//! 1. Rejection of symlinks and hardlinks.
//! 2. Rejection of directory traversal sequences (`..`).
//! 3. Rejection of absolute paths and Windows drive letters (`C:`).
//! 4. Rejection of UNC network paths (`\\server\share`).
//! 5. Rejection of NTFS Alternate Data Streams (`:`).
//! 6. Rejection of Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`).
//! 7. Rejection of trailing dots and spaces in any path segment.
//! 8. Rejection of case-insensitive colliding entries (e.g., `foo/bar.txt` vs `FOO/BAR.TXT`).

use std::collections::HashSet;
use std::path::PathBuf;
use thiserror::Error;

/// Windows reserved device names that must be rejected anywhere in an entry path.
const RESERVED_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Errors returned when an archive entry path violates security boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArchiveEntryError {
    #[error("archive entry is a symlink or hardlink, which is forbidden")]
    SymlinkOrHardlink,

    #[error("archive entry path is empty or blank")]
    EmptyPath,

    #[error("archive entry path contains null byte")]
    NullByte,

    #[error("archive entry path contains directory traversal sequence ('..')")]
    TraversalDetected,

    #[error("archive entry path contains absolute path or drive letter")]
    AbsolutePathOrDrive,

    #[error("archive entry path contains UNC path")]
    UncPath,

    #[error("archive entry path contains alternate data stream or colon")]
    AlternateDataStream,

    #[error("archive entry segment is a reserved Windows device name: '{0}'")]
    ReservedDeviceName(String),

    #[error("archive entry segment ends with trailing dot or space")]
    TrailingDotOrSpace,

    #[error("archive entry differs only in case from an earlier entry: '{0}'")]
    CaseDuplicate(String),
}

/// Check if a path segment matches a Windows reserved device name.
fn is_reserved_device_name(segment: &str) -> bool {
    let stem = match segment.split('.').next() {
        Some(s) => s,
        None => segment,
    };
    let stem_upper = stem.to_ascii_uppercase();
    RESERVED_DEVICE_NAMES.contains(&stem_upper.as_str())
}

/// Validate a single archive entry path against security invariants.
///
/// Returns the normalized relative `PathBuf` if valid.
/// Tracks seen paths in `seen_entries_lower` to detect case-collision attacks.
pub fn validate_archive_entry_path(
    entry_path: &str,
    is_symlink: bool,
    is_hardlink: bool,
    seen_entries_lower: &mut HashSet<String>,
) -> Result<PathBuf, ArchiveEntryError> {
    // 1. Link check
    if is_symlink || is_hardlink {
        return Err(ArchiveEntryError::SymlinkOrHardlink);
    }

    // 2. Empty check
    let trimmed = entry_path.trim();
    if trimmed.is_empty() {
        return Err(ArchiveEntryError::EmptyPath);
    }

    // 3. Null byte check
    if entry_path.contains('\0') {
        return Err(ArchiveEntryError::NullByte);
    }

    // 4. Colon / Alternate Data Stream / Drive check
    if entry_path.contains(':') {
        return Err(ArchiveEntryError::AlternateDataStream);
    }

    // 5. UNC Path check (starts with \\ or //)
    if entry_path.starts_with(r"\\") || entry_path.starts_with("//") {
        return Err(ArchiveEntryError::UncPath);
    }

    // 6. Absolute path check
    if entry_path.starts_with('/') || entry_path.starts_with('\\') {
        return Err(ArchiveEntryError::AbsolutePathOrDrive);
    }

    // Split entry by both '/' and '\'
    let raw_segments: Vec<&str> = entry_path
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .collect();

    if raw_segments.is_empty() {
        return Err(ArchiveEntryError::EmptyPath);
    }

    let mut clean_segments = Vec::with_capacity(raw_segments.len());

    for segment in raw_segments {
        // Traversal check
        if segment == ".." || segment.contains("..") {
            return Err(ArchiveEntryError::TraversalDetected);
        }

        // Current dir segment '.' can be ignored
        if segment == "." {
            continue;
        }

        // Drive letter check (e.g. C:)
        if segment.len() >= 2 && segment.as_bytes()[1] == b':' {
            return Err(ArchiveEntryError::AbsolutePathOrDrive);
        }

        // Trailing dot or space check
        if segment.ends_with('.') || segment.ends_with(' ') {
            return Err(ArchiveEntryError::TrailingDotOrSpace);
        }

        // Reserved device name check
        if is_reserved_device_name(segment) {
            return Err(ArchiveEntryError::ReservedDeviceName(segment.to_string()));
        }

        clean_segments.push(segment);
    }

    if clean_segments.is_empty() {
        return Err(ArchiveEntryError::EmptyPath);
    }

    let normalized_str = clean_segments.join("/");
    let lower_key = normalized_str.to_ascii_lowercase();

    // 7. Case collision check
    if seen_entries_lower.contains(&lower_key) {
        return Err(ArchiveEntryError::CaseDuplicate(entry_path.to_string()));
    }
    seen_entries_lower.insert(lower_key);

    let mut path_buf = PathBuf::new();
    for seg in clean_segments {
        path_buf.push(seg);
    }

    Ok(path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_entry_paths() {
        let mut seen = HashSet::new();
        assert!(validate_archive_entry_path("file.txt", false, false, &mut seen).is_ok());
        assert!(
            validate_archive_entry_path("folder/sub/file.json", false, false, &mut seen).is_ok()
        );
        assert!(
            validate_archive_entry_path("node-v22.11.0-win-x64/node.exe", false, false, &mut seen)
                .is_ok()
        );
    }

    #[test]
    fn test_symlink_and_hardlink_rejected() {
        let mut seen = HashSet::new();
        assert_eq!(
            validate_archive_entry_path("file.txt", true, false, &mut seen),
            Err(ArchiveEntryError::SymlinkOrHardlink)
        );
        assert_eq!(
            validate_archive_entry_path("file.txt", false, true, &mut seen),
            Err(ArchiveEntryError::SymlinkOrHardlink)
        );
    }

    #[test]
    fn test_traversal_zip_slip_rejected() {
        let mut seen = HashSet::new();
        assert_eq!(
            validate_archive_entry_path("../../evil.txt", false, false, &mut seen),
            Err(ArchiveEntryError::TraversalDetected)
        );
        assert_eq!(
            validate_archive_entry_path("foo/../bar.txt", false, false, &mut seen),
            Err(ArchiveEntryError::TraversalDetected)
        );
        assert_eq!(
            validate_archive_entry_path(r"foo\..\evil.txt", false, false, &mut seen),
            Err(ArchiveEntryError::TraversalDetected)
        );
    }

    #[test]
    fn test_absolute_and_drive_letters_rejected() {
        let mut seen = HashSet::new();
        assert_eq!(
            validate_archive_entry_path("/etc/passwd", false, false, &mut seen),
            Err(ArchiveEntryError::AbsolutePathOrDrive)
        );
        assert_eq!(
            validate_archive_entry_path(r"\windows\system32", false, false, &mut seen),
            Err(ArchiveEntryError::AbsolutePathOrDrive)
        );
        assert_eq!(
            validate_archive_entry_path("C:/evil.exe", false, false, &mut seen),
            Err(ArchiveEntryError::AlternateDataStream) // colon caught first
        );
    }

    #[test]
    fn test_unc_paths_rejected() {
        let mut seen = HashSet::new();
        assert_eq!(
            validate_archive_entry_path(r"\\server\share\file", false, false, &mut seen),
            Err(ArchiveEntryError::UncPath)
        );
        assert_eq!(
            validate_archive_entry_path("//server/share/file", false, false, &mut seen),
            Err(ArchiveEntryError::UncPath)
        );
    }

    #[test]
    fn test_alternate_data_streams_rejected() {
        let mut seen = HashSet::new();
        assert_eq!(
            validate_archive_entry_path("file.txt:stream", false, false, &mut seen),
            Err(ArchiveEntryError::AlternateDataStream)
        );
        assert_eq!(
            validate_archive_entry_path("sub/file.txt::$DATA", false, false, &mut seen),
            Err(ArchiveEntryError::AlternateDataStream)
        );
    }

    #[test]
    fn test_reserved_device_names_rejected() {
        let mut seen = HashSet::new();
        assert_eq!(
            validate_archive_entry_path("CON", false, false, &mut seen),
            Err(ArchiveEntryError::ReservedDeviceName("CON".to_string()))
        );
        assert_eq!(
            validate_archive_entry_path("sub/nul.txt", false, false, &mut seen),
            Err(ArchiveEntryError::ReservedDeviceName("nul.txt".to_string()))
        );
        assert_eq!(
            validate_archive_entry_path("com1/file.txt", false, false, &mut seen),
            Err(ArchiveEntryError::ReservedDeviceName("com1".to_string()))
        );
        assert_eq!(
            validate_archive_entry_path("lpt9.dat", false, false, &mut seen),
            Err(ArchiveEntryError::ReservedDeviceName(
                "lpt9.dat".to_string()
            ))
        );
    }

    #[test]
    fn test_trailing_dots_and_spaces_rejected() {
        let mut seen = HashSet::new();
        assert_eq!(
            validate_archive_entry_path("foo./bar", false, false, &mut seen),
            Err(ArchiveEntryError::TrailingDotOrSpace)
        );
        assert_eq!(
            validate_archive_entry_path("foo /bar", false, false, &mut seen),
            Err(ArchiveEntryError::TrailingDotOrSpace)
        );
        assert_eq!(
            validate_archive_entry_path("test.", false, false, &mut seen),
            Err(ArchiveEntryError::TrailingDotOrSpace)
        );
    }

    #[test]
    fn test_case_duplicate_rejected() {
        let mut seen = HashSet::new();
        assert!(validate_archive_entry_path("folder/file.txt", false, false, &mut seen).is_ok());
        assert_eq!(
            validate_archive_entry_path("FOLDER/FILE.TXT", false, false, &mut seen),
            Err(ArchiveEntryError::CaseDuplicate(
                "FOLDER/FILE.TXT".to_string()
            ))
        );
    }
}
