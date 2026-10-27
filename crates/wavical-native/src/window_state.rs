//! Window layout state filesystem persistence.
//!
//! Enforces:
//! - App data directory constructed strictly from validated `appId` (no directory traversal).
//! - Atomic writes (write to temporary sibling file, then rename).
//! - Loading state with corrupt file handling and fallback to defaults.

use std::io::Write;
use std::path::{Path, PathBuf};
use wavical_ipc::WindowState;

/// Validate `appId` and compute the standard application data folder: `%APPDATA%/<appId>/`
///
/// Rejects empty, whitespace, or invalid identifiers to prevent directory traversal or injection.
pub fn get_app_data_dir(app_id: &str) -> Result<PathBuf, String> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() {
        return Err("appId cannot be empty or whitespace".to_string());
    }

    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "Invalid appId '{app_id}': only alphanumeric, '-', and '_' are allowed"
        ));
    }

    if wavical_protocol::is_windows_reserved_device(trimmed) {
        return Err(format!(
            "Invalid appId '{app_id}': reserved Windows device name"
        ));
    }

    let base = if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata)
    } else if let Ok(userprofile) = std::env::var("USERPROFILE") {
        PathBuf::from(userprofile).join("AppData").join("Roaming")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        std::env::temp_dir()
    };

    Ok(base.join(trimmed))
}

/// Compute the path to `window-state.json` for a given `appId`.
pub fn get_window_state_path(app_id: &str) -> Result<PathBuf, String> {
    Ok(get_app_data_dir(app_id)?.join("window-state.json"))
}

/// Write data to target path atomically.
///
/// Writes to a sibling `.tmp` file in the same directory, flushes, syncs to disk,
/// and renames over the target path. If any step fails, the temporary file is cleaned up.
pub fn write_file_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Path must have a parent directory",
        )
    })?;

    std::fs::create_dir_all(parent)?;

    let pid = std::process::id();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_name = format!(".window-state-{pid}-{now}.tmp");
    let temp_path = parent.join(temp_name);

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(data)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }

    if let Err(e) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }

    Ok(())
}

/// Save window state to target path atomically as formatted JSON.
pub fn save_window_state(path: &Path, state: &WindowState) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_file_atomic(path, json.as_bytes())
}

/// Maximum allowed file size for window state ingestion (64 KB).
pub const MAX_WINDOW_STATE_FILE_BYTES: u64 = 64 * 1024;

/// Load window state from target path.
///
/// Returns `None` if the file does not exist, if the file size exceeds 64 KB,
/// or if the content is corrupt JSON.
pub fn load_window_state(path: &Path) -> Option<WindowState> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_WINDOW_STATE_FILE_BYTES {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    wavical_ipc::parse_window_state(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_data_path_validation() {
        assert!(get_app_data_dir("my-app").is_ok());
        assert!(get_app_data_dir("chat_app_123").is_ok());

        // Rejection of invalid characters and traversal sequences
        assert!(get_app_data_dir("").is_err());
        assert!(get_app_data_dir("   ").is_err());
        assert!(get_app_data_dir("../evil").is_err());
        assert!(get_app_data_dir("app/nested").is_err());
        assert!(get_app_data_dir("app\\nested").is_err());
        assert!(get_app_data_dir("app:stream").is_err());

        // FINDING-W1: Rejection of Windows reserved device names
        assert!(get_app_data_dir("con").is_err());
        assert!(get_app_data_dir("aux").is_err());
        assert!(get_app_data_dir("prn").is_err());
        assert!(get_app_data_dir("nul").is_err());
        assert!(get_app_data_dir("COM1").is_err());
    }

    #[test]
    fn test_atomic_write_creates_and_overwrites() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let target_file = temp_dir.path().join("sub").join("state.json");

        // Write initial data
        write_file_atomic(&target_file, b"initial content").expect("first write should succeed");
        assert_eq!(
            std::fs::read_to_string(&target_file).unwrap(),
            "initial content"
        );

        // Atomic overwrite
        write_file_atomic(&target_file, b"overwritten content")
            .expect("second write should succeed");
        assert_eq!(
            std::fs::read_to_string(&target_file).unwrap(),
            "overwritten content"
        );

        // Verify no leftover .tmp files
        let parent = target_file.parent().unwrap();
        let entries: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["state.json"]);
    }

    #[test]
    fn test_save_and_load_window_state_roundtrip() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let state_path = temp_dir.path().join("window-state.json");

        let original = WindowState {
            x: Some(300),
            y: Some(400),
            width: 1600,
            height: 1000,
            maximized: true,
        };

        save_window_state(&state_path, &original).expect("save should succeed");
        let loaded = load_window_state(&state_path).expect("load should succeed");
        assert_eq!(loaded, original);

        // Corrupt file -> returns None
        std::fs::write(&state_path, "invalid corrupt json {{{{").expect("corrupt write");
        assert_eq!(load_window_state(&state_path), None);

        // Oversized file > 64 KB -> returns None
        let oversized = vec![b' '; 65 * 1024];
        std::fs::write(&state_path, &oversized).expect("write oversized file");
        assert_eq!(load_window_state(&state_path), None);
    }
}
