use std::path::{Path, PathBuf};
use wavical_ipc::MenuItem;

/// Resolve the tray icon path declared in config against the app's `src/` directory.
///
/// Security rules (Phase 0 protocol rules):
/// - Rejects directory traversal sequences (`..`).
/// - Rejects absolute paths and drive letters.
/// - Rejects UNC paths.
/// - Rejects alternate data streams (`:`).
/// - Rejects Windows reserved device names (CON, PRN, AUX, etc.).
/// - Rejects trailing spaces or dots.
/// - Ensures canonical path remains strictly within `src_dir`.
/// - Target must exist and be a regular file.
pub fn resolve_tray_icon_path(src_dir: &Path, rel_path: &str) -> Result<PathBuf, String> {
    match wavical_protocol::resolve_asset_path(rel_path, src_dir) {
        Ok(resolved) => Ok(resolved.path),
        Err(reason) => Err(format!("Invalid tray icon path '{rel_path}': {reason}")),
    }
}

/// Convert a slice of validated `MenuItem`s into a `muda::Menu`.
pub fn build_muda_menu(items: &[MenuItem]) -> muda::Menu {
    let menu = muda::Menu::new();
    for item in items {
        if item.separator {
            let _ = menu.append(&muda::PredefinedMenuItem::separator());
        } else {
            let menu_item = muda::MenuItem::with_id(
                muda::MenuId::new(&item.id),
                &item.label,
                item.enabled,
                None,
            );
            let _ = menu.append(&menu_item);
        }
    }
    menu
}

/// Load an icon from a resolved filesystem path.
pub fn load_tray_icon(path: &Path) -> Result<tray_icon::Icon, String> {
    tray_icon::Icon::from_path(path, Some((32, 32))).map_err(|e| {
        format!(
            "Failed to load tray icon from path '{}': {e}",
            path.display()
        )
    })
}

/// Show a context menu on Windows for the given HWND at the cursor position.
///
/// Safety:
/// Calls `muda::ContextMenu::show_context_menu_for_hwnd` with a valid Win32 HWND.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn show_context_menu_for_hwnd(menu: &muda::Menu, hwnd: isize) {
    use muda::ContextMenu;
    unsafe {
        menu.show_context_menu_for_hwnd(hwnd, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn test_resolve_tray_icon_path_valid() {
        let dir = tempdir().expect("tempdir");
        let icon_path = dir.path().join("icon.png");
        File::create(&icon_path).expect("create icon");

        let resolved = resolve_tray_icon_path(dir.path(), "icon.png");
        assert!(resolved.is_ok());
        assert_eq!(resolved.unwrap(), icon_path.canonicalize().unwrap());
    }

    #[test]
    fn test_resolve_tray_icon_path_traversal_rejected() {
        let dir = tempdir().expect("tempdir");
        let sub = dir.path().join("src");
        std::fs::create_dir(&sub).expect("create src");

        let secret = dir.path().join("secret.ico");
        File::create(&secret).expect("create secret");

        let res = resolve_tray_icon_path(&sub, "../secret.ico");
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .contains("Directory traversal sequence detected")
        );
    }

    #[test]
    fn test_resolve_tray_icon_path_absolute_rejected() {
        let dir = tempdir().expect("tempdir");
        let res = resolve_tray_icon_path(dir.path(), "C:\\Windows\\System32\\shell32.dll");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Absolute path or drive letter"));
    }

    #[test]
    fn test_resolve_tray_icon_path_alternate_data_stream_rejected() {
        let dir = tempdir().expect("tempdir");
        let res = resolve_tray_icon_path(dir.path(), "icon.png:hidden");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Alternate data stream"));
    }

    #[test]
    fn test_resolve_tray_icon_path_not_found_rejected() {
        let dir = tempdir().expect("tempdir");
        let res = resolve_tray_icon_path(dir.path(), "nonexistent.ico");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Requested asset not found"));
    }

    #[test]
    fn test_build_muda_menu() {
        let items = vec![
            MenuItem::new("item_1", "Item 1"),
            MenuItem::separator("sep_1"),
            MenuItem::new("item_2", "Item 2").with_enabled(false),
        ];
        let menu = build_muda_menu(&items);
        assert_eq!(menu.items().len(), 3);
    }
}
