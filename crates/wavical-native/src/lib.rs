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
    pub fn clipboard_read(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("clipboard.read not implemented yet".to_string())
    }

    /// Write text to clipboard (WP3).
    pub fn clipboard_write(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("clipboard.write not implemented yet".to_string())
    }

    /// Show desktop notification (WP3).
    pub fn notify_send(
        &self,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        Err("notify.send not implemented yet".to_string())
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
    use wavical_ipc::{dispatch_native, Capabilities, Request, NATIVE_METHOD_NAMES};

    #[test]
    fn test_native_service_empty_handlers_return_not_implemented() {
        let service = NativeService::new();
        let window_id = "test-window";

        for &method in NATIVE_METHOD_NAMES {
            let res = service.handle_native(method, &serde_json::Value::Null, window_id);
            assert!(res.is_err(), "Empty handler for '{method}' should return Err");
            let err = res.unwrap_err();
            assert!(
                err.contains("not implemented yet"),
                "Error for '{method}' was: {err}"
            );
        }
    }

    #[test]
    fn test_native_service_dispatched_via_ipc() {
        let service = NativeService::new();
        let mut caps = Capabilities::new();
        caps.grant("clipboard.read");

        let req = Request {
            id: 1,
            method: "clipboard.read".to_string(),
            params: serde_json::Value::Null,
        };

        let resp = dispatch_native(&req, "main-win", &caps, &service);
        assert!(!resp.ok);
        // Error from empty handler should be captured in response
        assert!(resp.error.unwrap().contains("clipboard.read not implemented yet"));
    }
}
