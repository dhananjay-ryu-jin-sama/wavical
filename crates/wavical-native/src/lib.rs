#![forbid(unsafe_code)]

use wavical_ipc::NativeHandler;

/// Native service implementing the IPC `NativeHandler` trait.
///
/// In WP2, all handlers are empty/stubbed out and return "Not implemented" errors.
/// Feature-specific functionality is implemented in subsequent work packages
/// (WP3: clipboard & notifications, WP4: dialogs & fs grants, WP7: menus & tray, WP8: shortcuts & badges).
#[derive(Debug, Default, Clone)]
pub struct NativeService;

impl NativeService {
    /// Create a new native service instance.
    pub fn new() -> Self {
        Self
    }

    /// Read text from clipboard (WP3).
    ///
    /// Non-text data or oversize clipboard content (>1 MB) returns a generic error.
    /// Never returns truncated values.
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
    ///
    /// Validates payload format (text only, <=1 MB) and writes to OS clipboard.
    /// Reject non-text and oversize payloads with a generic error.
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
    ///
    /// Enforces title (128 chars) and body (1024 chars) limits as named constants.
    /// Rejects actions, URLs, or invalid payloads with a generic error.
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
    pub fn dialog_open_file(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("dialog.openFile not implemented yet".to_string())
    }

    /// Show save file dialog and return scoped grant (WP4).
    pub fn dialog_save_file(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("dialog.saveFile not implemented yet".to_string())
    }

    /// Read file content via scoped grant (WP4).
    pub fn fs_read_granted(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("fs.readGranted not implemented yet".to_string())
    }

    /// Write file content via scoped grant (WP4).
    pub fn fs_write_granted(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("fs.writeGranted not implemented yet".to_string())
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
    use wavical_ipc::{dispatch_native, Capabilities, Request};

    #[test]
    fn test_native_service_unimplemented_handlers_return_not_implemented() {
        let service = NativeService::new();
        let window_id = "test-window";

        let remaining_unimplemented = [
            "dialog.openFile",
            "dialog.saveFile",
            "fs.readGranted",
            "fs.writeGranted",
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
}
