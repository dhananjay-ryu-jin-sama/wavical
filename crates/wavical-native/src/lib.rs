#![forbid(unsafe_code)]

use std::io::Read;
use std::sync::{Arc, OnceLock};
use wavical_ipc::{
    GrantEntry, GrantTable, NativeHandler, OpenFileDialogParams, ReadGrantedParams,
    SaveFileDialogParams, WriteGrantedParams, MAX_READ_GRANTED_BYTES, MAX_WRITE_GRANTED_BYTES,
};

pub mod single_instance;
pub use single_instance::*;

pub mod window_state;
pub use window_state::*;

static GLOBAL_GRANT_TABLE: OnceLock<Arc<GrantTable>> = OnceLock::new();
#[cfg(test)]
static CLIPBOARD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn default_grant_table() -> Arc<GrantTable> {
    GLOBAL_GRANT_TABLE
        .get_or_init(|| Arc::new(GrantTable::new()))
        .clone()
}

/// Resolve a filter ID into a human-readable title and allowed file extensions.
/// Returns None if the filter ID is not recognized.
fn resolve_filter_id(filter_id: &str) -> Option<(&'static str, &'static [&'static str])> {
    match filter_id {
        "text" => Some(("Text Documents", &["txt", "md", "json", "csv", "log"])),
        "images" | "image" => Some(("Images", &["png", "jpg", "jpeg", "gif", "webp", "svg"])),
        "documents" | "document" => Some(("Documents", &["txt", "md", "pdf", "docx"])),
        "all" => Some(("All Files", &["*"])),
        _ => None,
    }
}

/// Validate all security rules for a granted path at use time.
///
/// Rules:
/// 1. Path must not contain null bytes.
/// 2. Path must not contain alternate data stream syntax (colon after drive prefix).
/// 3. Path components must not match Windows reserved device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9).
/// 4. Path components must not end in trailing dot or space.
/// 5. If `must_exist` is true, the target path must exist on disk.
/// 6. Path must not be a symlink or junction.
/// 7. Path must be a regular file (not directory, device, socket, etc.).
/// 8. Current canonical path must match the canonical path recorded at grant time.
fn validate_granted_target(entry: &GrantEntry, must_exist: bool) -> Result<(), &'static str> {
    let path = &entry.path;

    // 1. Null byte check
    if path.to_string_lossy().contains('\0') {
        return Err("Null byte detected in path");
    }

    // 2. Alternate data stream check (no colon after drive prefix)
    let path_str = path.to_string_lossy();
    let bytes = path_str.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b':' {
            // Allow Windows drive letter like C: at index 1
            if i == 1 && bytes[0].is_ascii_alphabetic() {
                continue;
            }
            return Err("Alternate data streams are not permitted");
        }
    }

    // 3. Reserved Windows device names & 4. Trailing dot or space
    for comp in path.components() {
        let seg = comp.as_os_str().to_string_lossy();
        if wavical_protocol::is_windows_reserved_device(&seg) {
            return Err("Target path contains reserved Windows device name");
        }
        if seg.ends_with('.') || seg.ends_with(' ') {
            return Err("Target path segment contains trailing dot or space");
        }
    }

    // 5. Existence check
    if must_exist && !path.exists() {
        return Err("Granted file does not exist");
    }

    // If target exists, verify regular file and symlink status
    if path.exists() {
        // 6. Symlink / junction check
        let symlink_meta = std::fs::symlink_metadata(path)
            .map_err(|_| "Failed to read file metadata")?;
        if symlink_meta.is_symlink() {
            return Err("Symlinks and junctions are strictly forbidden");
        }

        // 7. Regular file check
        let meta = std::fs::metadata(path)
            .map_err(|_| "Failed to read file metadata")?;
        if !meta.is_file() {
            return Err("Target is not a regular file");
        }

        // 8. Canonical path comparison
        let current_canonical = path.canonicalize()
            .map_err(|_| "Failed to canonicalize path")?;
        if let Some(ref orig_canonical) = entry.canonical_path {
            if &current_canonical != orig_canonical {
                return Err("Canonical path mismatch: target was replaced or moved");
            }
        }
    } else if let Some(parent) = path.parent() {
        // If file does not exist yet (e.g. newly created file in save dialog):
        // Ensure parent directory exists, is not a symlink, and canonical parent matches expected parent
        if !parent.exists() {
            return Err("Parent directory does not exist");
        }
        let parent_symlink_meta = std::fs::symlink_metadata(parent)
            .map_err(|_| "Failed to read parent directory metadata")?;
        if parent_symlink_meta.is_symlink() {
            return Err("Symlink parent directories are strictly forbidden");
        }
        let current_parent_canonical = parent.canonicalize()
            .map_err(|_| "Failed to canonicalize parent directory")?;
        if let (Some(orig_canonical), Some(file_name)) = (&entry.canonical_path, path.file_name()) {
            let expected = current_parent_canonical.join(file_name);
            if orig_canonical != &expected {
                return Err("Canonical path mismatch: parent directory was replaced or moved");
            }
        }
    }

    Ok(())
}

/// Native service implementing the IPC `NativeHandler` trait.
#[derive(Debug, Clone)]
pub struct NativeService {
    grant_table: Arc<GrantTable>,
}

impl Default for NativeService {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeService {
    /// Create a new native service instance using the shared grant table.
    pub fn new() -> Self {
        Self {
            grant_table: default_grant_table(),
        }
    }

    /// Create a native service instance using an isolated grant table (useful for tests).
    pub fn with_grant_table(grant_table: Arc<GrantTable>) -> Self {
        Self { grant_table }
    }

    /// Access the underlying grant table.
    pub fn grant_table(&self) -> &Arc<GrantTable> {
        &self.grant_table
    }

    /// Clear all grants for the given window ID.
    pub fn clear_window(&self, window_id: &str) {
        self.grant_table.clear_window(window_id);
    }

    /// Read text from clipboard (WP3).
    pub fn clipboard_read(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| {
            tracing::error!("Failed to initialize clipboard: {e}");
            wavical_ipc::GENERIC_CLIPBOARD_ERROR.to_string()
        })?;

        let text = clipboard.get_text().map_err(|e| {
            tracing::warn!("Failed to get clipboard text (may be non-text): {e}");
            wavical_ipc::GENERIC_CLIPBOARD_ERROR.to_string()
        })?;

        let valid_text = wavical_ipc::validate_clipboard_read(&text)
            .map_err(|err| err.to_string())?;

        Ok(serde_json::json!({
            "text": valid_text
        }))
    }

    /// Write text to clipboard (WP3).
    pub fn clipboard_write(
        &self,
        params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        let text = wavical_ipc::validate_clipboard_write(params)
            .map_err(|err| err.to_string())?;

        let mut clipboard = arboard::Clipboard::new().map_err(|e| {
            tracing::error!("Failed to initialize clipboard: {e}");
            wavical_ipc::GENERIC_CLIPBOARD_ERROR.to_string()
        })?;

        clipboard.set_text(text).map_err(|e| {
            tracing::error!("Failed to set clipboard text: {e}");
            wavical_ipc::GENERIC_CLIPBOARD_ERROR.to_string()
        })?;

        Ok(serde_json::json!({
            "success": true
        }))
    }

    /// Show desktop notification (WP3).
    pub fn notify_send(
        &self,
        params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        let payload = wavical_ipc::validate_notify_send(params)
            .map_err(|err| err.to_string())?;

        notify_rust::Notification::new()
            .appname("Wavical")
            .summary(payload.title)
            .body(payload.body)
            .show()
            .map_err(|e| {
                tracing::warn!("Failed to dispatch notification: {e}");
                wavical_ipc::GENERIC_NOTIFICATION_ERROR.to_string()
            })?;

        Ok(serde_json::json!({
            "success": true
        }))
    }

    /// Show open file dialog and return scoped grant (WP4).
    ///
    /// The page receives only an opaque grant ID, NEVER the raw file path.
    /// Cancelled dialog returns `{"cancelled": true}`, not an error.
    pub fn dialog_open_file(
        &self,
        params: &serde_json::Value,
        window_id: &str,
    ) -> Result<serde_json::Value, String> {
        let parsed: OpenFileDialogParams = serde_json::from_value(params.clone())
            .map_err(|e| format!("Invalid open file dialog parameters: {e}"))?;

        let mut dialog = rfd::FileDialog::new();

        if let Some(ref title) = parsed.title {
            dialog = dialog.set_title(title);
        }

        if let Some(ref filter_id) = parsed.filter_id {
            let (filter_name, extensions) = resolve_filter_id(filter_id)
                .ok_or_else(|| format!("Unknown dialog filter ID: '{filter_id}'. Raw filter strings are rejected."))?;
            dialog = dialog.add_filter(filter_name, extensions);
        }

        let chosen_path = dialog.pick_file();

        match chosen_path {
            Some(path) => {
                let grant_id = self.grant_table.insert(window_id, path)
                    .map_err(|e| format!("Failed to record file grant: {e}"))?;
                Ok(serde_json::json!({
                    "cancelled": false,
                    "grantId": grant_id,
                }))
            }
            None => {
                Ok(serde_json::json!({
                    "cancelled": true
                }))
            }
        }
    }

    /// Show save file dialog and return scoped grant (WP4).
    ///
    /// The page receives only an opaque grant ID, NEVER the raw file path.
    /// Cancelled dialog returns `{"cancelled": true}`, not an error.
    pub fn dialog_save_file(
        &self,
        params: &serde_json::Value,
        window_id: &str,
    ) -> Result<serde_json::Value, String> {
        let parsed: SaveFileDialogParams = serde_json::from_value(params.clone())
            .map_err(|e| format!("Invalid save file dialog parameters: {e}"))?;

        let mut dialog = rfd::FileDialog::new();

        if let Some(ref title) = parsed.title {
            dialog = dialog.set_title(title);
        }

        if let Some(ref default_name) = parsed.default_name {
            dialog = dialog.set_file_name(default_name);
        }

        if let Some(ref filter_id) = parsed.filter_id {
            let (filter_name, extensions) = resolve_filter_id(filter_id)
                .ok_or_else(|| format!("Unknown dialog filter ID: '{filter_id}'. Raw filter strings are rejected."))?;
            dialog = dialog.add_filter(filter_name, extensions);
        }

        let chosen_path = dialog.save_file();

        match chosen_path {
            Some(path) => {
                let grant_id = self.grant_table.insert(window_id, path)
                    .map_err(|e| format!("Failed to record file grant: {e}"))?;
                Ok(serde_json::json!({
                    "cancelled": false,
                    "grantId": grant_id,
                }))
            }
            None => {
                Ok(serde_json::json!({
                    "cancelled": true
                }))
            }
        }
    }

    /// Read file content via scoped grant (WP4).
    ///
    /// Takes only a grant ID (never a path).
    /// Re-checks at use time: exists, regular file, canonical path still matches,
    /// not a reserved device name, no alternate data streams, and enforces 10 MB limit.
    pub fn fs_read_granted(
        &self,
        params: &serde_json::Value,
        window_id: &str,
    ) -> Result<serde_json::Value, String> {
        let parsed: ReadGrantedParams = serde_json::from_value(params.clone())
            .map_err(|e| format!("Invalid read parameters: {e}"))?;

        let entry = self.grant_table.get(&parsed.grant_id, window_id)
            .map_err(|_| "Grant not found or not owned by this window".to_string())?;

        validate_granted_target(&entry, true)
            .map_err(|err| err.to_string())?;

        let meta = std::fs::metadata(&entry.path)
            .map_err(|e| format!("Failed to read metadata: {e}"))?;

        if meta.len() > MAX_READ_GRANTED_BYTES as u64 {
            return Err("File size exceeds 10 MB limit".to_string());
        }

        let mut file = std::fs::File::open(&entry.path)
            .map_err(|e| format!("Failed to open file: {e}"))?;

        let mut buffer = Vec::new();
        let bytes_read = file
            .by_ref()
            .take(MAX_READ_GRANTED_BYTES as u64 + 1)
            .read_to_end(&mut buffer)
            .map_err(|e| format!("Failed to read file: {e}"))?;

        if bytes_read > MAX_READ_GRANTED_BYTES {
            return Err("File size exceeds 10 MB limit".to_string());
        }

        let content = String::from_utf8(buffer)
            .map_err(|_| "File contains invalid UTF-8 data".to_string())?;

        Ok(serde_json::json!({
            "content": content
        }))
    }

    /// Write file content via scoped grant (WP4).
    ///
    /// Takes only a grant ID (never a path).
    /// Re-checks at use time: not a reserved device name, no alternate data streams,
    /// canonical path matches, and enforces 10 MB payload limit.
    pub fn fs_write_granted(
        &self,
        params: &serde_json::Value,
        window_id: &str,
    ) -> Result<serde_json::Value, String> {
        let parsed: WriteGrantedParams = serde_json::from_value(params.clone())
            .map_err(|e| format!("Invalid write parameters: {e}"))?;

        if parsed.content.len() > MAX_WRITE_GRANTED_BYTES {
            return Err("Payload exceeds 10 MB limit".to_string());
        }

        let entry = self.grant_table.get(&parsed.grant_id, window_id)
            .map_err(|_| "Grant not found or not owned by this window".to_string())?;

        validate_granted_target(&entry, false)
            .map_err(|err| err.to_string())?;

        std::fs::write(&entry.path, parsed.content.as_bytes())
            .map_err(|e| format!("Failed to write file: {e}"))?;

        Ok(serde_json::json!({
            "success": true,
            "bytesWritten": parsed.content.len()
        }))
    }

    /// Show window context menu (WP7).
    pub fn menu_show_context_menu(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("menu.showContextMenu not implemented yet".to_string())
    }

    /// Update system tray icon, tooltip, and menu (WP7).
    pub fn tray_set(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("tray.set not implemented yet".to_string())
    }

    /// Register global hotkey shortcut (WP8).
    pub fn shortcut_register(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("shortcut.register not implemented yet".to_string())
    }

    /// Set taskbar overlay badge (WP8).
    pub fn badge_set(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("badge.set not implemented yet".to_string())
    }
}

impl NativeHandler for NativeService {
    fn handle_native(
        &self,
        method: &str,
        params: &serde_json::Value,
        window_id: &str,
    ) -> Result<serde_json::Value, String> {
        match method {
            "clipboard.read" => self.clipboard_read(params, window_id),
            "clipboard.write" => self.clipboard_write(params, window_id),
            "notify.send" => self.notify_send(params, window_id),
            "dialog.openFile" => self.dialog_open_file(params, window_id),
            "dialog.saveFile" => self.dialog_save_file(params, window_id),
            "fs.readGranted" => self.fs_read_granted(params, window_id),
            "fs.writeGranted" => self.fs_write_granted(params, window_id),
            "menu.showContextMenu" => self.menu_show_context_menu(params, window_id),
            "tray.set" => self.tray_set(params, window_id),
            "shortcut.register" => self.shortcut_register(params, window_id),
            "badge.set" => self.badge_set(params, window_id),
            unknown => Err(format!("Unknown native method: {unknown}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use wavical_ipc::{dispatch_native, Capabilities, Request};

    #[test]
    fn test_native_service_unimplemented_handlers_return_not_implemented() {
        let service = NativeService::new();
        let window_id = "test-window";

        let remaining_unimplemented = [
            "menu.showContextMenu",
            "tray.set",
            "shortcut.register",
            "badge.set",
        ];

        for &method in &remaining_unimplemented {
            let res = service.handle_native(method, &serde_json::Value::Null, window_id);
            assert!(res.is_err(), "Handler for '{method}' should return Err");
            let err = res.unwrap_err();
            assert!(
                err.contains("not implemented yet"),
                "Error for '{method}' was: {err}"
            );
        }
    }

    #[test]
    fn test_native_service_clipboard_roundtrip() {
        let _test_lock = CLIPBOARD_LOCK.lock().unwrap();
        let service = NativeService::new();
        let window_id = "test-window";

        let test_text = "Hello from Wavical native clipboard test!";
        let write_res = service.clipboard_write(
            &serde_json::json!({"text": test_text}),
            window_id,
        );

        if let Ok(resp) = write_res {
            assert_eq!(resp, serde_json::json!({"success": true}));

            let read_res = service.clipboard_read(&serde_json::Value::Null, window_id);
            assert!(read_res.is_ok());
            let read_val = read_res.unwrap();
            assert_eq!(read_val["text"], test_text);
        } else {
            // In headless/container environments where clipboard is unavailable, verify error is generic
            assert_eq!(
                write_res.unwrap_err(),
                wavical_ipc::GENERIC_CLIPBOARD_ERROR
            );
        }
    }

    #[test]
    fn test_native_service_clipboard_oversize_rejected() {
        let service = NativeService::new();
        let window_id = "test-window";

        let huge_text = "x".repeat(wavical_ipc::MAX_CLIPBOARD_TEXT_BYTES + 10);
        let write_res = service.clipboard_write(
            &serde_json::json!({"text": huge_text}),
            window_id,
        );
        assert!(write_res.is_err());
        assert_eq!(write_res.unwrap_err(), wavical_ipc::GENERIC_CLIPBOARD_ERROR);
    }

    #[test]
    fn test_native_service_notify_send_limits_rejected() {
        let service = NativeService::new();
        let window_id = "test-window";

        // Oversize title > 128
        let huge_title = "T".repeat(129);
        let bad_title = service.notify_send(
            &serde_json::json!({"title": huge_title, "body": "msg"}),
            window_id,
        );
        assert!(bad_title.is_err());
        assert_eq!(bad_title.unwrap_err(), wavical_ipc::GENERIC_NOTIFICATION_ERROR);

        // Disallowed url parameter
        let bad_url = service.notify_send(
            &serde_json::json!({"title": "Test", "body": "msg", "url": "https://example.com"}),
            window_id,
        );
        assert!(bad_url.is_err());
        assert_eq!(bad_url.unwrap_err(), wavical_ipc::GENERIC_NOTIFICATION_ERROR);
    }

    #[test]
    fn test_native_service_dispatched_via_ipc_pipeline() {
        let _test_lock = CLIPBOARD_LOCK.lock().unwrap();
        let service = NativeService::new();
        let mut caps = Capabilities::new();
        caps.grant("clipboard.write");

        let req = Request {
            id: 1,
            method: "clipboard.write".to_string(),
            params: serde_json::json!({"text": "test dispatch"}),
        };

        let resp = dispatch_native(&req, "main-win", &caps, &service);
        // Either succeeds with true, or fails with generic error if headless OS clipboard is locked
        if resp.ok {
            assert_eq!(resp.result, Some(serde_json::json!({"success": true})));
        } else {
            assert_eq!(resp.error.as_deref(), Some(wavical_ipc::GENERIC_CLIPBOARD_ERROR));
        }
    }

    // =========================================================================
    // Work Package 4 Tests (Required - Each must fail if the rule is removed)
    // =========================================================================

    // Test 1: Forged grant ID is rejected.
    #[test]
    fn test_wp4_forged_grant_id_rejected() {
        let service = NativeService::with_grant_table(Arc::new(GrantTable::new()));
        let window_id = "window-A";

        // Attempt read with forged, random grant ID
        let res = service.fs_read_granted(
            &serde_json::json!({"grantId": "0123456789abcdef0123456789abcdef"}),
            window_id,
        );
        assert!(res.is_err(), "Forged grant ID must be rejected");
        assert!(res.unwrap_err().contains("Grant not found"));

        // Attempt write with forged grant ID
        let res_write = service.fs_write_granted(
            &serde_json::json!({
                "grantId": "0123456789abcdef0123456789abcdef",
                "content": "malicious data"
            }),
            window_id,
        );
        assert!(res_write.is_err(), "Forged grant ID must be rejected on write");
        assert!(res_write.unwrap_err().contains("Grant not found"));
    }

    // Test 2: Grant from window A is rejected for window B.
    #[test]
    fn test_wp4_cross_window_grant_rejected() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let file_path = temp_dir.path().join("secret.txt");
        std::fs::write(&file_path, "confidential window A data").expect("write file");

        let grant_table = Arc::new(GrantTable::new());
        let service = NativeService::with_grant_table(Arc::clone(&grant_table));

        // Create grant for Window A
        let grant_id = grant_table
            .insert("window-A", &file_path)
            .expect("grant insertion should succeed");

        // Window A can read it successfully
        let read_a = service.fs_read_granted(
            &serde_json::json!({"grantId": grant_id}),
            "window-A",
        );
        assert!(read_a.is_ok(), "Window A should read its own grant");
        assert_eq!(read_a.unwrap()["content"], "confidential window A data");

        // Window B attempts to use Window A's grant ID -> MUST be treated as not found
        let read_b = service.fs_read_granted(
            &serde_json::json!({"grantId": grant_id}),
            "window-B",
        );
        assert!(read_b.is_err(), "Window B must not access Window A's grant");
        assert!(read_b.unwrap_err().contains("Grant not found"));

        // Window B attempts write -> MUST be rejected
        let write_b = service.fs_write_granted(
            &serde_json::json!({"grantId": grant_id, "content": "hijacked"}),
            "window-B",
        );
        assert!(write_b.is_err(), "Window B must not write to Window A's grant");
        assert!(write_b.unwrap_err().contains("Grant not found"));
    }

    // Test 3: Grants are gone after the window closes.
    #[test]
    fn test_wp4_grants_cleared_on_window_close() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let file_path = temp_dir.path().join("closing.txt");
        std::fs::write(&file_path, "temporary window data").expect("write file");

        let grant_table = Arc::new(GrantTable::new());
        let service = NativeService::with_grant_table(Arc::clone(&grant_table));
        let window_id = "window-closing";

        let grant_id = grant_table
            .insert(window_id, &file_path)
            .expect("insert grant");

        // Verify grant works initially
        let before_close = service.fs_read_granted(
            &serde_json::json!({"grantId": grant_id}),
            window_id,
        );
        assert!(before_close.is_ok());

        // Simulate window close
        service.clear_window(window_id);

        // After window closes, grant is GONE
        let after_close = service.fs_read_granted(
            &serde_json::json!({"grantId": grant_id}),
            window_id,
        );
        assert!(after_close.is_err(), "Grant must be gone after window closes");
        assert!(after_close.unwrap_err().contains("Grant not found"));
    }

    // Test 4: Grant count limit: 65th grant fails.
    #[test]
    fn test_wp4_grant_count_limit_65th_fails() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let grant_table = Arc::new(GrantTable::new());
        let window_id = "window-limit";

        // Insert exactly 64 grants (all succeed)
        for i in 0..wavical_ipc::MAX_GRANTS_PER_WINDOW {
            let file_path = temp_dir.path().join(format!("file_{i}.txt"));
            std::fs::write(&file_path, format!("content {i}")).expect("write file");
            let res = grant_table.insert(window_id, file_path);
            assert!(res.is_ok(), "Grant {i} within limit of 64 should succeed");
        }

        assert_eq!(grant_table.count_for_window(window_id), 64);

        // Attempt 65th grant -> MUST fail with LimitReached
        let file_65 = temp_dir.path().join("file_65.txt");
        std::fs::write(&file_65, "content 65").expect("write file");
        let res_65 = grant_table.insert(window_id, file_65);
        assert!(res_65.is_err(), "65th grant must fail with limit reached");
        assert_eq!(res_65.unwrap_err(), wavical_ipc::GrantError::LimitReached);

        // Another window can still create grants independently
        let other_window_res = grant_table.insert("window-other", temp_dir.path().join("other.txt"));
        assert!(other_window_res.is_ok(), "Different window limit is separate");
    }

    // Test 5: A granted path replaced by a symlink or junction after the grant is rejected.
    #[test]
    fn test_wp4_granted_path_replaced_by_symlink_rejected() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let real_file = temp_dir.path().join("real.txt");
        let secret_file = temp_dir.path().join("secret.txt");

        std::fs::write(&real_file, "real content").expect("write real file");
        std::fs::write(&secret_file, "sensitive secret data").expect("write secret file");

        let grant_table = Arc::new(GrantTable::new());
        let service = NativeService::with_grant_table(Arc::clone(&grant_table));
        let window_id = "window-symlink";

        // Create grant while real_file is a normal file
        let grant_id = grant_table
            .insert(window_id, &real_file)
            .expect("insert grant");

        // Verify read succeeds initially
        let initial_read = service.fs_read_granted(
            &serde_json::json!({"grantId": grant_id}),
            window_id,
        );
        assert!(initial_read.is_ok());
        assert_eq!(initial_read.unwrap()["content"], "real content");

        // Remove real_file
        std::fs::remove_file(&real_file).expect("remove real file");

        // Attempt to create symlink at real_file pointing to secret_file
        #[cfg(windows)]
        let symlink_result = std::os::windows::fs::symlink_file(&secret_file, &real_file);
        #[cfg(not(windows))]
        let symlink_result = std::os::unix::fs::symlink(&secret_file, &real_file);

        match symlink_result {
            Ok(_) => {
                // Symlink was created; re-check at use time MUST detect and reject it!
                let symlink_read = service.fs_read_granted(
                    &serde_json::json!({"grantId": grant_id}),
                    window_id,
                );
                assert!(
                    symlink_read.is_err(),
                    "Path replaced by symlink must be rejected at use time"
                );
                let err = symlink_read.unwrap_err();
                assert!(
                    err.contains("Symlinks and junctions are strictly forbidden")
                        || err.contains("Canonical path mismatch"),
                    "Unexpected error: {err}"
                );
            }
            Err(err) => {
                // On Windows without Developer Mode / Admin rights, OS blocks symlink creation.
                println!(
                    "Skipping symlink test: OS blocks creating symlink (privilege not held): {err}"
                );
            }
        }
    }

    // Test 6: Over-limit read and write fail.
    #[test]
    fn test_wp4_over_limit_read_and_write_fail() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let grant_table = Arc::new(GrantTable::new());
        let service = NativeService::with_grant_table(Arc::clone(&grant_table));
        let window_id = "window-limits";

        // 6a: Over-limit read (>10 MB)
        let huge_file = temp_dir.path().join("huge.bin");
        {
            let mut f = std::fs::File::create(&huge_file).expect("create file");
            // Write 10 MB + 1 byte
            let chunk = vec![b'A'; 1024 * 1024]; // 1 MB
            for _ in 0..10 {
                f.write_all(&chunk).expect("write chunk");
            }
            f.write_all(b"!").expect("write extra byte");
        }

        let read_grant = grant_table
            .insert(window_id, &huge_file)
            .expect("insert grant");

        let read_res = service.fs_read_granted(
            &serde_json::json!({"grantId": read_grant}),
            window_id,
        );
        assert!(read_res.is_err(), "Read over 10 MB limit must fail");
        assert!(read_res.unwrap_err().contains("10 MB limit"));

        // 6b: Over-limit write (>10 MB)
        let write_file = temp_dir.path().join("target_write.txt");
        std::fs::write(&write_file, "init").expect("init file");
        let write_grant = grant_table
            .insert(window_id, &write_file)
            .expect("insert grant");

        let huge_payload = "X".repeat(MAX_WRITE_GRANTED_BYTES + 1);
        let write_res = service.fs_write_granted(
            &serde_json::json!({"grantId": write_grant, "content": huge_payload}),
            window_id,
        );
        assert!(write_res.is_err(), "Write over 10 MB limit must fail");
        assert!(write_res.unwrap_err().contains("10 MB limit"));
    }

    // Test 7: Page-supplied path strings fail to deserialize (deny_unknown_fields).
    #[test]
    fn test_wp4_page_supplied_paths_fail_deserialization() {
        let service = NativeService::new();
        let window_id = "window-path-injection";

        // Attack 1: Injected "path" in fs.readGranted
        let bad_read = service.fs_read_granted(
            &serde_json::json!({
                "grantId": "0123456789abcdef0123456789abcdef",
                "path": "C:\\Windows\\System32\\drivers\\etc\\hosts"
            }),
            window_id,
        );
        assert!(bad_read.is_err(), "fs.readGranted must reject injected 'path'");
        assert!(bad_read.unwrap_err().contains("unknown field `path`"));

        // Attack 2: Injected "path" in fs.writeGranted
        let bad_write = service.fs_write_granted(
            &serde_json::json!({
                "grantId": "0123456789abcdef0123456789abcdef",
                "content": "hacked",
                "path": "C:\\Windows\\System32\\drivers\\etc\\hosts"
            }),
            window_id,
        );
        assert!(bad_write.is_err(), "fs.writeGranted must reject injected 'path'");
        assert!(bad_write.unwrap_err().contains("unknown field `path`"));

        // Attack 3: Injected "path" in dialog.openFile
        let bad_open = service.dialog_open_file(
            &serde_json::json!({
                "path": "C:\\Windows\\System32\\notepad.exe"
            }),
            window_id,
        );
        assert!(bad_open.is_err(), "dialog.openFile must reject injected 'path'");
        assert!(bad_open.unwrap_err().contains("unknown field `path`"));

        // Attack 4: Raw filter string in dialog.openFile
        let bad_filter = service.dialog_open_file(
            &serde_json::json!({
                "filter": "*.txt"
            }),
            window_id,
        );
        assert!(bad_filter.is_err(), "dialog.openFile must reject raw 'filter'");
        assert!(bad_filter.unwrap_err().contains("unknown field `filter`"));

        // Attack 5: Unknown filterId in dialog.openFile
        let bad_filter_id = service.dialog_open_file(
            &serde_json::json!({
                "filterId": "custom_arbitrary_unapproved"
            }),
            window_id,
        );
        assert!(bad_filter_id.is_err(), "dialog.openFile must reject unknown filterId");
        assert!(bad_filter_id.unwrap_err().contains("Unknown dialog filter ID"));
    }

    #[test]
    fn test_wp4_alternate_data_stream_rejected() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let ads_path = temp_dir.path().join("normal.txt:hidden_stream");

        let grant_table = Arc::new(GrantTable::new());
        let service = NativeService::with_grant_table(Arc::clone(&grant_table));
        let window_id = "window-ads";

        // Even if an entry with colon is somehow inserted
        let grant_id = grant_table.insert(window_id, ads_path).expect("insert");

        let res = service.fs_read_granted(
            &serde_json::json!({"grantId": grant_id}),
            window_id,
        );
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Alternate data streams are not permitted"));
    }

    #[test]
    fn test_wp4_reserved_windows_device_rejected() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let reserved_path = temp_dir.path().join("CON.txt");

        let grant_table = Arc::new(GrantTable::new());
        let service = NativeService::with_grant_table(Arc::clone(&grant_table));
        let window_id = "window-reserved";

        let grant_id = grant_table.insert(window_id, reserved_path).expect("insert");

        let res = service.fs_read_granted(
            &serde_json::json!({"grantId": grant_id}),
            window_id,
        );
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("reserved Windows device name"));
    }

    #[test]
    fn test_wp4_normal_file_read_and_write_roundtrip() {
        let temp_dir = tempfile::tempdir().expect("create tempdir");
        let test_file = temp_dir.path().join("test_roundtrip.txt");
        std::fs::write(&test_file, "Initial content 12345").expect("write file");

        let grant_table = Arc::new(GrantTable::new());
        let service = NativeService::with_grant_table(Arc::clone(&grant_table));
        let window_id = "window-roundtrip";

        let grant_id = grant_table.insert(window_id, &test_file).expect("insert");

        // Read initial content
        let read_res = service.fs_read_granted(
            &serde_json::json!({"grantId": grant_id}),
            window_id,
        ).expect("read should succeed");
        assert_eq!(read_res["content"], "Initial content 12345");

        // Write new content
        let write_res = service.fs_write_granted(
            &serde_json::json!({
                "grantId": grant_id,
                "content": "Updated content successfully written!"
            }),
            window_id,
        ).expect("write should succeed");
        assert_eq!(write_res["success"], true);

        // Read updated content
        let read_updated = service.fs_read_granted(
            &serde_json::json!({"grantId": grant_id}),
            window_id,
        ).expect("read should succeed");
        assert_eq!(read_updated["content"], "Updated content successfully written!");
    }
}
