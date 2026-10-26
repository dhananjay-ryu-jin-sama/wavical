//! Scoped file access grant table (pure type for wavical-ipc).
//!
//! Enforces:
//! - 128-bit random, unguessable grant IDs (via `getrandom`).
//! - Strictly isolates grants per window (window A cannot access window B's grants).
//! - Maximum 64 grants per window (`MAX_GRANTS_PER_WINDOW`).
//! - Clearing grants on window close.
//! - Parameter types with `#[serde(deny_unknown_fields)]` so paths from JS fail deserialization.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;
use thiserror::Error;

/// Maximum number of file grants permitted concurrently for a single window.
pub const MAX_GRANTS_PER_WINDOW: usize = 64;

/// Maximum allowable file size for `fs.readGranted` (10 MB named constant).
pub const MAX_READ_GRANTED_BYTES: usize = 10 * 1024 * 1024;

/// Maximum allowable payload size for `fs.writeGranted` (10 MB named constant).
pub const MAX_WRITE_GRANTED_BYTES: usize = 10 * 1024 * 1024;

/// Generic error message returned when a grant lookup or operation fails.
pub const GENERIC_GRANT_ERROR: &str = "Grant operation failed";

/// Generic error message returned when a filesystem operation fails.
pub const GENERIC_FS_ERROR: &str = "File operation failed";

/// Rejection reasons for grant operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GrantError {
    #[error("Window grant limit reached (max 64)")]
    LimitReached,

    #[error("Grant not found")]
    NotFound,

    #[error("Entropy source failed to generate random grant ID: {0}")]
    RandomnessError(String),
}

/// Access permission mode for scoped file grants (FINDING-G2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GrantMode {
    /// Read-only access (granted by `dialog.openFile`).
    #[default]
    ReadOnly,
    /// Write-only access (granted by `dialog.saveFile`).
    WriteOnly,
    /// Read-write access (e.g. for internal or test use).
    ReadWrite,
}

impl GrantMode {
    /// Check whether this grant mode permits reading file content.
    pub fn can_read(&self) -> bool {
        matches!(self, GrantMode::ReadOnly | GrantMode::ReadWrite)
    }

    /// Check whether this grant mode permits writing file content.
    pub fn can_write(&self) -> bool {
        matches!(self, GrantMode::WriteOnly | GrantMode::ReadWrite)
    }
}

/// An entry in the grant table mapping a grant ID to window and path metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantEntry {
    /// Window ID that owns this grant.
    pub window_id: String,
    /// The target filesystem path chosen by the user in the native dialog.
    pub path: PathBuf,
    /// Canonical path captured at grant time (if target exists).
    pub canonical_path: Option<PathBuf>,
    /// Permission mode scoping (read-only vs write-only vs read-write).
    pub mode: GrantMode,
}

/// Generate a cryptographically secure 128-bit unguessable grant ID formatted as 32 lowercase hex characters.
pub fn generate_grant_id() -> Result<String, GrantError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| GrantError::RandomnessError(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}

/// Thread-safe in-memory table tracking active scoped file grants per window.
#[derive(Debug, Default)]
pub struct GrantTable {
    grants: RwLock<HashMap<String, GrantEntry>>,
}

impl GrantTable {
    /// Create a new, empty grant table.
    pub fn new() -> Self {
        Self {
            grants: RwLock::new(HashMap::new()),
        }
    }

    /// Insert a granted path for a window with the default mode (ReadOnly), returning an opaque 128-bit grant ID.
    ///
    /// Returns `Err(GrantError::LimitReached)` if the window already holds 64 grants.
    pub fn insert(&self, window_id: &str, path: impl Into<PathBuf>) -> Result<String, GrantError> {
        self.insert_with_mode(window_id, path, GrantMode::ReadOnly)
    }

    /// Insert a granted path for a window with an explicit permission mode (FINDING-G2).
    ///
    /// Returns `Err(GrantError::LimitReached)` if the window already holds 64 grants.
    pub fn insert_with_mode(
        &self,
        window_id: &str,
        path: impl Into<PathBuf>,
        mode: GrantMode,
    ) -> Result<String, GrantError> {
        let mut table = self.grants.write().unwrap();

        // Enforce max 64 grants per window limit
        let current_count = table.values().filter(|e| e.window_id == window_id).count();
        if current_count >= MAX_GRANTS_PER_WINDOW {
            return Err(GrantError::LimitReached);
        }

        let grant_id = generate_grant_id()?;
        let path_buf: PathBuf = path.into();
        let canonical_path = if path_buf.exists() {
            path_buf.canonicalize().ok()
        } else if let Some(parent) = path_buf.parent() {
            if let (Ok(canonical_parent), Some(file_name)) =
                (parent.canonicalize(), path_buf.file_name())
            {
                Some(canonical_parent.join(file_name))
            } else {
                None
            }
        } else {
            None
        };

        table.insert(
            grant_id.clone(),
            GrantEntry {
                window_id: window_id.to_string(),
                path: path_buf,
                canonical_path,
                mode,
            },
        );

        Ok(grant_id)
    }

    /// Look up a grant by ID and window ID.
    ///
    /// CRITICAL SECURITY INVARIANT:
    /// If the grant exists but belongs to a different window, it returns `GrantError::NotFound`
    /// to prevent cross-window access or existence probing.
    pub fn get(&self, grant_id: &str, window_id: &str) -> Result<GrantEntry, GrantError> {
        let table = self.grants.read().unwrap();
        match table.get(grant_id) {
            Some(entry) if entry.window_id == window_id => Ok(entry.clone()),
            _ => Err(GrantError::NotFound),
        }
    }

    /// Clear all grants for the given window ID (called when the window closes).
    pub fn clear_window(&self, window_id: &str) {
        let mut table = self.grants.write().unwrap();
        table.retain(|_, entry| entry.window_id != window_id);
    }

    /// Return the number of active grants for a window.
    pub fn count_for_window(&self, window_id: &str) -> usize {
        let table = self.grants.read().unwrap();
        table.values().filter(|e| e.window_id == window_id).count()
    }

    /// Return total grants currently stored across all windows.
    pub fn total_count(&self) -> usize {
        let table = self.grants.read().unwrap();
        table.len()
    }

    /// Remove a specific grant ID if owned by `window_id`.
    pub fn remove_grant(&self, grant_id: &str, window_id: &str) -> bool {
        let mut table = self.grants.write().unwrap();
        if table.get(grant_id).map(|e| e.window_id.as_str()) == Some(window_id) {
            table.remove(grant_id);
            return true;
        }
        false
    }
}

/// Parameters for `dialog.openFile`.
/// Strictly rejects page-supplied file paths and arbitrary filter strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenFileDialogParams {
    #[serde(default, rename = "filterId", alias = "filter_id")]
    pub filter_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// Parameters for `dialog.saveFile`.
/// Strictly rejects page-supplied file paths and arbitrary filter strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveFileDialogParams {
    #[serde(default, rename = "filterId", alias = "filter_id")]
    pub filter_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "defaultName", alias = "default_name")]
    pub default_name: Option<String>,
}

/// Parameters for `fs.readGranted`.
/// Strictly takes ONLY a grant ID; page-supplied paths fail deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadGrantedParams {
    #[serde(rename = "grantId", alias = "grant_id")]
    pub grant_id: String,
}

/// Parameters for `fs.writeGranted`.
/// Strictly takes ONLY a grant ID and content; page-supplied paths fail deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteGrantedParams {
    #[serde(rename = "grantId", alias = "grant_id")]
    pub grant_id: String,
    #[serde(default, alias = "data", alias = "text")]
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_grant_id_format_and_entropy() {
        let mut ids = HashSet::new();
        for _ in 0..100 {
            let id = generate_grant_id().expect("grant ID generation should succeed");
            // 128 bits = 16 bytes = 32 lowercase hex characters
            assert_eq!(id.len(), 32);
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
            assert!(ids.insert(id), "Grant IDs must be unique and unguessable");
        }
    }

    #[test]
    fn test_grant_table_window_isolation() {
        let table = GrantTable::new();
        let path = PathBuf::from("sample_doc.txt");

        let grant_id = table.insert("win-1", path.clone()).expect("insert");

        // Allowed for win-1
        let entry = table
            .get(&grant_id, "win-1")
            .expect("win-1 lookup should succeed");
        assert_eq!(entry.window_id, "win-1");
        assert_eq!(entry.path, path);

        // Denied (treated as not found) for win-2
        let err = table
            .get(&grant_id, "win-2")
            .expect_err("win-2 lookup must fail");
        assert_eq!(err, GrantError::NotFound);
    }

    #[test]
    fn test_grant_table_clear_on_window_close() {
        let table = GrantTable::new();
        let id1 = table.insert("win-close", PathBuf::from("a.txt")).unwrap();
        let id2 = table.insert("win-close", PathBuf::from("b.txt")).unwrap();
        let id_other = table.insert("win-keep", PathBuf::from("c.txt")).unwrap();

        assert_eq!(table.count_for_window("win-close"), 2);
        assert_eq!(table.count_for_window("win-keep"), 1);

        // Window closes
        table.clear_window("win-close");

        assert_eq!(table.count_for_window("win-close"), 0);
        assert_eq!(
            table.get(&id1, "win-close").unwrap_err(),
            GrantError::NotFound
        );
        assert_eq!(
            table.get(&id2, "win-close").unwrap_err(),
            GrantError::NotFound
        );

        // Other window is unaffected
        assert_eq!(table.count_for_window("win-keep"), 1);
        assert!(table.get(&id_other, "win-keep").is_ok());
    }

    #[test]
    fn test_grant_table_max_64_grants_per_window() {
        let table = GrantTable::new();
        let win = "win-max";

        for i in 0..MAX_GRANTS_PER_WINDOW {
            let res = table.insert(win, PathBuf::from(format!("file_{i}.txt")));
            assert!(res.is_ok(), "Grant {i} within limit should succeed");
        }

        assert_eq!(table.count_for_window(win), 64);

        // 65th grant fails
        let err = table.insert(win, PathBuf::from("file_65.txt")).unwrap_err();
        assert_eq!(err, GrantError::LimitReached);
    }

    #[test]
    fn test_deny_unknown_fields_rejects_page_supplied_paths() {
        // ReadGrantedParams
        let json_with_path =
            r#"{"grantId": "0123456789abcdef0123456789abcdef", "path": "secret.txt"}"#;
        let err_read: Result<ReadGrantedParams, _> = serde_json::from_str(json_with_path);
        assert!(err_read.is_err());
        assert!(
            err_read
                .unwrap_err()
                .to_string()
                .contains("unknown field `path`")
        );

        // WriteGrantedParams
        let json_write_path = r#"{"grantId": "0123456789abcdef0123456789abcdef", "content": "data", "path": "secret.txt"}"#;
        let err_write: Result<WriteGrantedParams, _> = serde_json::from_str(json_write_path);
        assert!(err_write.is_err());
        assert!(
            err_write
                .unwrap_err()
                .to_string()
                .contains("unknown field `path`")
        );

        // OpenFileDialogParams
        let json_open_path = r#"{"path": "secret.txt"}"#;
        let err_open: Result<OpenFileDialogParams, _> = serde_json::from_str(json_open_path);
        assert!(err_open.is_err());
        assert!(
            err_open
                .unwrap_err()
                .to_string()
                .contains("unknown field `path`")
        );

        let json_open_filter = r#"{"filter": "*.txt"}"#;
        let err_open_filter: Result<OpenFileDialogParams, _> =
            serde_json::from_str(json_open_filter);
        assert!(err_open_filter.is_err());
        assert!(
            err_open_filter
                .unwrap_err()
                .to_string()
                .contains("unknown field `filter`")
        );

        // SaveFileDialogParams
        let json_save_path = r#"{"path": "secret.txt"}"#;
        let err_save: Result<SaveFileDialogParams, _> = serde_json::from_str(json_save_path);
        assert!(err_save.is_err());
        assert!(
            err_save
                .unwrap_err()
                .to_string()
                .contains("unknown field `path`")
        );
    }

    #[test]
    fn test_grant_mode_permissions() {
        let table = GrantTable::new();
        let read_id = table
            .insert_with_mode("win-1", PathBuf::from("read.txt"), GrantMode::ReadOnly)
            .unwrap();
        let write_id = table
            .insert_with_mode("win-1", PathBuf::from("write.txt"), GrantMode::WriteOnly)
            .unwrap();
        let rw_id = table
            .insert_with_mode("win-1", PathBuf::from("rw.txt"), GrantMode::ReadWrite)
            .unwrap();
        let default_id = table.insert("win-1", PathBuf::from("def.txt")).unwrap();

        let read_entry = table.get(&read_id, "win-1").unwrap();
        assert_eq!(read_entry.mode, GrantMode::ReadOnly);
        assert!(read_entry.mode.can_read());
        assert!(!read_entry.mode.can_write());

        let write_entry = table.get(&write_id, "win-1").unwrap();
        assert_eq!(write_entry.mode, GrantMode::WriteOnly);
        assert!(!write_entry.mode.can_read());
        assert!(write_entry.mode.can_write());

        let rw_entry = table.get(&rw_id, "win-1").unwrap();
        assert_eq!(rw_entry.mode, GrantMode::ReadWrite);
        assert!(rw_entry.mode.can_read());
        assert!(rw_entry.mode.can_write());

        let default_entry = table.get(&default_id, "win-1").unwrap();
        assert_eq!(default_entry.mode, GrantMode::ReadOnly);
    }
}
