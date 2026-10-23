#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(utf8_str) = std::str::from_utf8(data) {
        // 1. Fuzz menu parameters
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(utf8_str) {
            let _ = wavical_ipc::parse_and_validate_tray_params(&val);
            let _ = wavical_ipc::parse_and_validate_context_menu_params(&val);
            let _ = wavical_ipc::validate_notify_send(&val);
            let _ = wavical_ipc::validate_clipboard_write(&val);
            let _ = serde_json::from_value::<wavical_ipc::OpenFileDialogParams>(val.clone());
            let _ = serde_json::from_value::<wavical_ipc::SaveFileDialogParams>(val.clone());
            let _ = serde_json::from_value::<wavical_ipc::ReadGrantedParams>(val.clone());
            let _ = serde_json::from_value::<wavical_ipc::WriteGrantedParams>(val);
        }

        // 2. Fuzz shortcut parser
        let _ = wavical_ipc::validate_shortcut(utf8_str);
        let shortcuts: Vec<String> = utf8_str.split(',').map(|s| s.to_string()).collect();
        let _ = wavical_ipc::validate_shortcut_list(&shortcuts);

        // 3. Fuzz file extension parser
        let _ = wavical_ipc::validate_file_extension(utf8_str);

        // 4. Fuzz window state parser
        let _ = wavical_ipc::parse_saved_window_state(utf8_str, 800, 600);

        // 5. Fuzz badge count parser
        if let Ok(num) = utf8_str.parse::<i64>() {
            let _ = wavical_ipc::validate_badge_count(num);
        }
    }
});
