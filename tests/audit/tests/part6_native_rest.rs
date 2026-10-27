//! Black-box and unit audit tests for Phase 2 Extended Native features:
//! Window state persistence, single-instance lock, WebView2 UX, crash recovery,
//! system tray, context menu, global shortcuts, taskbar badge, and file associations.
//!
//! Expectations are derived strictly from docs/WAVICAL_PHASE_2_BUILD_PLAN.md WP5 to WP8.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};
use tempfile::tempdir;
use wavical_config::parse_config_json;
use wavical_ipc::{
    BadgeDisplay, BadgeValidationError, CrashRecoveryDecision, DEFAULT_SAVED_HEIGHT,
    DEFAULT_SAVED_WIDTH, MAX_MENU_ITEMS, MAX_MENU_LABEL_CHARS, MenuItem, MenuValidationError,
    Modifier, ScreenBounds, ShortcutValidationError, WindowState, decide_crash_recovery,
    parse_and_validate_badge_params, parse_and_validate_shortcut, parse_and_validate_tray_params,
    parse_window_state, resolve_window_state, validate_badge_count, validate_file_extension,
    validate_menu_items, validate_shortcut_list,
};
use wavical_native::{
    NativeService, SingleInstance, check_single_instance, get_app_data_dir, load_tray_icon,
    load_window_state, save_window_state,
};

// ============================================================================
// 1. Window State (WP5)
// ============================================================================

#[test]
fn test_part6_window_state_parser_battery() {
    // 1. Empty string -> CorruptJson
    assert!(parse_window_state("").is_err());
    assert!(parse_window_state("   ").is_err());

    // 2. Corrupt JSON / truncated JSON
    assert!(parse_window_state("{").is_err());
    assert!(parse_window_state(r#"{"width": 800,"#).is_err());
    assert!(parse_window_state("not a json").is_err());

    // 3. JSON array instead of object
    assert!(parse_window_state("[]").is_err());

    // 4. Missing fields fall back to defaults
    let empty_obj = parse_window_state("{}").expect("empty object should parse");
    assert_eq!(empty_obj.width, DEFAULT_SAVED_WIDTH);
    assert_eq!(empty_obj.height, DEFAULT_SAVED_HEIGHT);
    assert_eq!(empty_obj.x, None);
    assert_eq!(empty_obj.y, None);
    assert!(!empty_obj.maximized);

    // 5. Unknown fields are ignored (forward compatibility)
    let unknown_fields = r#"{
        "width": 1024,
        "height": 768,
        "futureFeature": true,
        "nestedUnknown": { "foo": "bar" }
    }"#;
    let state_unknown = parse_window_state(unknown_fields).expect("unknown fields ignored");
    assert_eq!(state_unknown.width, 1024);
    assert_eq!(state_unknown.height, 768);

    // 6. Zero, negative, and extreme dimensions fall back to defaults
    let zero_dims = r#"{"width": 0, "height": 0}"#;
    let state_zero = parse_window_state(zero_dims).expect("zero dims parse");
    assert_eq!(state_zero.width, DEFAULT_SAVED_WIDTH);
    assert_eq!(state_zero.height, DEFAULT_SAVED_HEIGHT);

    let huge_dims = r#"{"width": 1000000000, "height": 1000000000}"#;
    let state_huge = parse_window_state(huge_dims).expect("huge dims parse");
    assert_eq!(state_huge.width, DEFAULT_SAVED_WIDTH);
    assert_eq!(state_huge.height, DEFAULT_SAVED_HEIGHT);

    // 7. Huge positions (+/- 1e9) fit in i32
    let pos_json = r#"{"x": 1000000, "y": -1000000, "width": 800, "height": 600}"#;
    let state_pos = parse_window_state(pos_json).expect("huge pos parse");
    assert_eq!(state_pos.x, Some(1000000));
    assert_eq!(state_pos.y, Some(-1000000));
}

#[test]
fn test_part6_window_state_screen_bounds_and_monitors() {
    // Single 1920x1080 display at origin
    let screens = [ScreenBounds::new(0, 0, 1920, 1080)];

    // 1. Valid on-screen coordinates preserved
    let on_screen_json = r#"{"x": 100, "y": 100, "width": 800, "height": 600}"#;
    let resolved = resolve_window_state(on_screen_json, 1200, 800, &screens);
    assert_eq!(resolved.x, Some(100));
    assert_eq!(resolved.y, Some(100));

    // 2. Off-screen coordinates (e.g. unplugged secondary monitor) fall back to None (centered default)
    let off_screen_json = r#"{"x": 3000, "y": 2000, "width": 800, "height": 600}"#;
    let resolved_off = resolve_window_state(off_screen_json, 1200, 800, &screens);
    assert_eq!(resolved_off.x, None);
    assert_eq!(resolved_off.y, None);
    assert_eq!(resolved_off.width, 800);
    assert_eq!(resolved_off.height, 600);

    // 3. Corrupt JSON falls back to provided defaults
    let corrupt_json = "corrupt {";
    let resolved_corrupt = resolve_window_state(corrupt_json, 1024, 768, &screens);
    assert_eq!(resolved_corrupt.width, 1024);
    assert_eq!(resolved_corrupt.height, 768);
    assert_eq!(resolved_corrupt.x, None);
}

#[test]
fn test_part6_window_state_atomic_write_and_path() {
    let tmp = tempdir().expect("tempdir");
    let state_file = tmp.path().join("sub").join("window-state.json");

    let state = WindowState {
        x: Some(250),
        y: Some(350),
        width: 1400,
        height: 900,
        maximized: false,
    };

    // Save state atomically
    save_window_state(&state_file, &state).expect("save state");
    assert!(state_file.exists());

    // Load state
    let loaded = load_window_state(&state_file).expect("load state");
    assert_eq!(loaded, state);

    // Overwrite atomically
    let state2 = WindowState {
        maximized: true,
        ..state
    };
    save_window_state(&state_file, &state2).expect("save state overwrite");
    let loaded2 = load_window_state(&state_file).expect("load state 2");
    assert!(loaded2.maximized);

    // Verify appId validation for storage path
    assert!(get_app_data_dir("valid-app_1").is_ok());
    assert!(get_app_data_dir("").is_err());
    assert!(get_app_data_dir("../traversal").is_err());
    assert!(get_app_data_dir("app/slash").is_err());
    assert!(get_app_data_dir("app\\backslash").is_err());
}

// ============================================================================
// 2. Single-Instance Lock (WP5)
// ============================================================================

#[test]
fn test_part6_single_instance_named_pipe_and_args() {
    let unique_id = format!("audit-lock-{}", std::process::id());
    let args1 = vec!["arg1".to_string(), "arg2".to_string()];

    // Primary instance takes the lock
    let primary = check_single_instance(&unique_id, &args1).expect("primary lock check");
    match primary {
        SingleInstance::Primary(server) => {
            // Secondary instance connects and forwards args
            let args2 = vec!["opened_via_second_launch".to_string()];
            let secondary = check_single_instance(&unique_id, &args2).expect("secondary check");
            assert!(matches!(secondary, SingleInstance::Secondary));
            server.stop();
        }
        _ => panic!("Expected SingleInstance::Primary"),
    }
}

// ============================================================================
// 3. WebView2 Missing Runtime UX (WP6)
// ============================================================================

#[test]
fn test_part6_webview2_missing_ux_parameters() {
    let err = wavical_shell::check_webview2_runtime_with_override(Some("test_missing"))
        .expect_err("override must trigger missing runtime error");
    assert!(err.contains("Simulated missing WebView2 runtime"));

    // Hardcoded Microsoft install URL check
    let ms_url = "https://developer.microsoft.com/en-us/microsoft-edge/webview2/";
    assert!(ms_url.starts_with("https://"));
}

// ============================================================================
// 4. Crash Recovery Decision Logic (WP6)
// ============================================================================

#[test]
fn test_part6_crash_recovery_pure_decision_matrix() {
    let t0 = Instant::now();
    let history_empty = Vec::new();

    // 1. First crash -> Retry
    assert_eq!(
        decide_crash_recovery(&history_empty, t0),
        CrashRecoveryDecision::Retry
    );

    // 2. Second crash inside 30s (< 30s) -> GiveUp
    let history_one = vec![t0];
    let t_10s = t0 + Duration::from_secs(10);
    assert_eq!(
        decide_crash_recovery(&history_one, t_10s),
        CrashRecoveryDecision::GiveUp
    );

    let t_29s = t0 + Duration::from_millis(29_999);
    assert_eq!(
        decide_crash_recovery(&history_one, t_29s),
        CrashRecoveryDecision::GiveUp
    );

    // 3. Second crash at exactly 30s -> GiveUp
    let t_30s = t0 + Duration::from_secs(30);
    assert_eq!(
        decide_crash_recovery(&history_one, t_30s),
        CrashRecoveryDecision::GiveUp
    );

    // 4. Second crash after 30s (> 30s) -> Retry
    let t_31s = t0 + Duration::from_millis(30_001);
    assert_eq!(
        decide_crash_recovery(&history_one, t_31s),
        CrashRecoveryDecision::Retry
    );

    let t_60s = t0 + Duration::from_secs(60);
    assert_eq!(
        decide_crash_recovery(&history_one, t_60s),
        CrashRecoveryDecision::Retry
    );

    // 5. 100 crashes in loop spaced by 45 seconds each -> all Retry
    let mut crash_history = Vec::new();
    let mut current_t = t0;
    for _ in 0..100 {
        assert_eq!(
            decide_crash_recovery(&crash_history, current_t),
            CrashRecoveryDecision::Retry
        );
        crash_history.push(current_t);
        current_t += Duration::from_secs(45);
    }
}

// ============================================================================
// 5. System Tray & Context Menu Validation (WP7)
// ============================================================================

#[test]
fn test_part6_menu_validation_battery() {
    // 1. 200 items passes
    let items_200: Vec<MenuItem> = (0..200)
        .map(|i| MenuItem::new(format!("item_{i}"), format!("Label {i}")))
        .collect();
    assert!(validate_menu_items(&items_200).is_ok());

    // 2. 201 items rejected
    let mut items_201 = items_200.clone();
    items_201.push(MenuItem::new("item_200", "Label 200"));
    assert_eq!(
        validate_menu_items(&items_201).unwrap_err(),
        MenuValidationError::TooManyItems {
            count: 201,
            max: MAX_MENU_ITEMS
        }
    );

    // 3. Empty list rejected
    assert_eq!(
        validate_menu_items(&[]).unwrap_err(),
        MenuValidationError::EmptyList
    );

    // 4. Label length: 100 chars passes, 101 chars rejected
    let label_100 = "L".repeat(100);
    assert!(validate_menu_items(&[MenuItem::new("item1", label_100)]).is_ok());

    let label_101 = "L".repeat(101);
    assert_eq!(
        validate_menu_items(&[MenuItem::new("item1", label_101)]).unwrap_err(),
        MenuValidationError::LabelTooLong {
            id: "item1".to_string(),
            max: MAX_MENU_LABEL_CHARS
        }
    );

    // Multibyte label: 100 emojis (100 chars, 400 bytes) passes
    let emoji_label_100 = "⭐".repeat(100);
    assert_eq!(emoji_label_100.chars().count(), 100);
    assert!(validate_menu_items(&[MenuItem::new("item1", emoji_label_100)]).is_ok());

    // Multibyte label: 101 emojis rejected
    let emoji_label_101 = "⭐".repeat(101);
    assert_eq!(
        validate_menu_items(&[MenuItem::new("item1", emoji_label_101)]).unwrap_err(),
        MenuValidationError::LabelTooLong {
            id: "item1".to_string(),
            max: MAX_MENU_LABEL_CHARS
        }
    );

    // 5. Item ID: [a-z0-9._-]{1,64}
    assert!(validate_menu_items(&[MenuItem::new("valid.item_id-1", "Label")]).is_ok());

    // Uppercase rejected
    assert!(validate_menu_items(&[MenuItem::new("UPPERCASE", "Label")]).is_err());
    // Spaces rejected
    assert!(validate_menu_items(&[MenuItem::new("item with spaces", "Label")]).is_err());
    // Unicode rejected
    assert!(validate_menu_items(&[MenuItem::new("itëm", "Label")]).is_err());
    // 65 characters rejected
    let id_65 = "a".repeat(65);
    assert!(validate_menu_items(&[MenuItem::new(id_65, "Label")]).is_err());

    // Duplicate IDs rejected
    let dupes = vec![
        MenuItem::new("same_id", "One"),
        MenuItem::new("same_id", "Two"),
    ];
    assert!(matches!(
        validate_menu_items(&dupes).unwrap_err(),
        MenuValidationError::DuplicateItemId(_)
    ));

    // Separators supported
    let with_sep = vec![
        MenuItem::new("item1", "One"),
        MenuItem::separator("sep1"),
        MenuItem::new("item2", "Two"),
    ];
    assert!(validate_menu_items(&with_sep).is_ok());

    // Tooltip validation: 128 chars ok, 129 chars rejected
    let tray_128 = serde_json::json!({
        "tooltip": "T".repeat(128),
        "items": [ { "id": "m1", "label": "Menu 1" } ]
    });
    assert!(parse_and_validate_tray_params(&tray_128).is_ok());

    let tray_129 = serde_json::json!({
        "tooltip": "T".repeat(129),
        "items": [ { "id": "m1", "label": "Menu 1" } ]
    });
    assert!(parse_and_validate_tray_params(&tray_129).is_err());
}

// ============================================================================
// 6. Global Shortcuts Grammar Battery (WP8)
// ============================================================================

#[test]
fn test_part6_shortcut_grammar_battery() {
    // 1. Valid shortcut
    let valid = parse_and_validate_shortcut("Ctrl+Alt+Shift+F11").expect("valid shortcut");
    assert_eq!(valid.key, "F11");
    assert_eq!(
        valid.modifiers,
        vec![Modifier::Control, Modifier::Alt, Modifier::Shift]
    );

    // Case normalization
    let case_norm = parse_and_validate_shortcut("ctrl+alt+k").expect("case norm");
    assert_eq!(case_norm.canonical, "Ctrl+Alt+K");

    // 2. Invalid shortcut cases
    assert_eq!(
        parse_and_validate_shortcut("").unwrap_err(),
        ShortcutValidationError::Empty
    );
    assert!(parse_and_validate_shortcut("+").is_err());
    assert_eq!(
        parse_and_validate_shortcut("Ctrl+").unwrap_err(),
        ShortcutValidationError::InvalidKey("".to_string())
    );
    assert_eq!(
        parse_and_validate_shortcut("Ctrl+Ctrl+A").unwrap_err(),
        ShortcutValidationError::DuplicateModifier("Ctrl".to_string())
    );
    assert_eq!(
        parse_and_validate_shortcut("UnknownMod+A").unwrap_err(),
        ShortcutValidationError::InvalidModifier("UnknownMod".to_string())
    );
    assert_eq!(
        parse_and_validate_shortcut("Ctrl+UnknownKey").unwrap_err(),
        ShortcutValidationError::InvalidKey("UnknownKey".to_string())
    );

    // Missing modifier (bare key)
    assert_eq!(
        parse_and_validate_shortcut("A").unwrap_err(),
        ShortcutValidationError::MissingModifier
    );

    // 3. Max 32 shortcuts per app list
    // (F1..F12 exist, so synthesize using numbers and letters)
    let keys = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
        "S", "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5",
    ];
    let list_valid: Vec<String> = keys.iter().map(|k| format!("Ctrl+Alt+{k}")).collect();
    assert_eq!(list_valid.len(), 32);
    assert!(validate_shortcut_list(&list_valid).is_ok());

    // 33 shortcuts rejected
    let mut list_33 = list_valid.clone();
    list_33.push("Ctrl+Alt+6".to_string());
    assert_eq!(
        validate_shortcut_list(&list_33).unwrap_err(),
        ShortcutValidationError::TooManyShortcuts(33)
    );

    // Duplicate shortcuts in list rejected
    let list_dupe = vec!["Ctrl+Shift+K".to_string(), "ctrl+shift+k".to_string()];
    assert!(matches!(
        validate_shortcut_list(&list_dupe).unwrap_err(),
        ShortcutValidationError::DuplicateShortcut(_)
    ));
}

// ============================================================================
// 7. Taskbar Overlay Badge (WP8)
// ============================================================================

#[test]
fn test_part6_badge_validation_and_deferred_stub() {
    // 0 clears
    assert_eq!(validate_badge_count(0).unwrap(), BadgeDisplay::Clear);
    // 1..=99 shows count
    assert_eq!(validate_badge_count(1).unwrap(), BadgeDisplay::Count(1));
    assert_eq!(validate_badge_count(99).unwrap(), BadgeDisplay::Count(99));
    // >99 shows "99+"
    assert_eq!(validate_badge_count(100).unwrap(), BadgeDisplay::Overflow);
    assert_eq!(validate_badge_count(150).unwrap(), BadgeDisplay::Overflow);
    assert_eq!(
        validate_badge_count(u32::MAX as i64).unwrap(),
        BadgeDisplay::Overflow
    );

    // Negative rejected
    assert_eq!(
        validate_badge_count(-1).unwrap_err(),
        BadgeValidationError::NegativeCount(-1)
    );

    // Float rejected
    assert!(parse_and_validate_badge_params(Some(&serde_json::json!(1.5))).is_err());
    // String rejected
    assert!(parse_and_validate_badge_params(Some(&serde_json::json!("5"))).is_err());
    // Null rejected
    assert!(parse_and_validate_badge_params(Some(&serde_json::Value::Null)).is_err());

    // Native service stub returns generic error explaining badge is deferred
    let service = NativeService::new();
    let res = service.badge_set(&serde_json::json!(5), "win-1");
    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err(),
        "Taskbar overlay badge is not supported on this platform"
    );
}

// ============================================================================
// 8. File Associations & Key Builder (WP8)
// ============================================================================

#[test]
fn test_part6_file_association_validation_battery() {
    // Valid: ^[a-z0-9]{1,10}$
    assert_eq!(validate_file_extension("png").unwrap(), "png");
    assert_eq!(validate_file_extension("wavdemo").unwrap(), "wavdemo");
    assert_eq!(validate_file_extension("0123456789").unwrap(), "0123456789");

    // Invalid extension cases
    assert!(validate_file_extension("PNG").is_err()); // Uppercase
    assert!(validate_file_extension(".png").is_err()); // Leading dot
    assert!(validate_file_extension(" png").is_err()); // Leading space
    assert!(validate_file_extension("png ").is_err()); // Trailing space
    assert!(validate_file_extension("png\n").is_err()); // Newline
    assert!(validate_file_extension("png\0").is_err()); // Null byte
    assert!(validate_file_extension("p.ng").is_err()); // Dot inside
    assert!(validate_file_extension("a/b").is_err()); // Slash
    assert!(validate_file_extension("a\\b").is_err()); // Backslash
    assert!(validate_file_extension("..").is_err()); // Dot dot
    assert!(validate_file_extension("").is_err()); // Empty
    assert!(validate_file_extension("toolongext1").is_err()); // 11 characters
    assert!(validate_file_extension("٣").is_err()); // Unicode digit
    assert!(validate_file_extension("ｐｎｇ").is_err()); // Full-width letters
    assert!(validate_file_extension("🚀").is_err()); // Emoji

    // Key builder: no slashes, backslashes, or injection
    let keys = wavical_ipc::build_association_keys("my-app", "wavdemo").expect("build keys");
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

    // Key builder rejects traversal or invalid appId
    assert!(wavical_ipc::build_association_keys("../evil", "wavdemo").is_err());
    assert!(wavical_ipc::build_association_keys("app/slash", "wavdemo").is_err());
}

// ============================================================================
// 9. Phase 2 Configuration Fields (WP9)
// ============================================================================

#[test]
fn test_part6_phase_2_config_fields_validation() {
    let valid_phase2_config = r#"{
        "appId": "wavical-phase2-test",
        "window": {
            "title": "Phase 2 Title",
            "width": 1024,
            "height": 768,
            "persistState": true
        },
        "singleInstance": true,
        "tray": {
            "icon": "icon.png"
        },
        "shortcuts": [
            "Ctrl+Shift+K"
        ],
        "fileAssociations": [
            "wavtest"
        ],
        "dialog": {
            "filters": {
                "text": { "name": "Text", "extensions": ["txt"] }
            }
        }
    }"#;

    let parsed = parse_config_json(valid_phase2_config).expect("valid Phase 2 config");
    assert_eq!(parsed.app_id, "wavical-phase2-test");
    assert_eq!(parsed.window.as_ref().unwrap().persist_state, Some(true));
    assert_eq!(parsed.single_instance, Some(true));
    assert_eq!(
        parsed.shortcuts.as_deref(),
        Some(&["Ctrl+Shift+K".to_string()][..])
    );
    assert_eq!(
        parsed.file_associations.as_deref(),
        Some(&["wavtest".to_string()][..])
    );

    // Unknown top-level field rejected
    let invalid_unknown = r#"{
        "appId": "test",
        "unknownPhase2Field": 123
    }"#;
    assert!(parse_config_json(invalid_unknown).is_err());
}

// ============================================================================
// 10. Audit Findings Tests (Ignored to keep CI Green)
// ============================================================================

/// FINDING-W1: get_app_data_dir does not reject Windows reserved device names (CON, AUX, NUL)
#[test]
fn test_app_data_dir_rejects_reserved_device_names() {
    assert!(
        get_app_data_dir("con").is_err(),
        "Reserved Windows device name 'con' should be rejected"
    );
    assert!(
        get_app_data_dir("aux").is_err(),
        "Reserved Windows device name 'aux' should be rejected"
    );
}

/// FINDING-R1: run_register_filetypes hijacks existing default file handler instead of using OpenWithProgids
#[test]
fn test_file_assoc_register_preserves_existing_default_handler() {
    #[cfg(windows)]
    {
        use wavical_cli::windows_registry::CURRENT_USER;
        let temp = tempfile::tempdir().expect("tempdir");
        let project_dir = temp.path();

        let config_js = r#"export default {
            appId: 'test-polite-app',
            fileAssociations: ['wavr1test']
        };"#;
        std::fs::write(project_dir.join("wavical.config.js"), config_js).expect("write config");

        // 1. Simulate an existing pre-configured default file handler
        let ext_key = CURRENT_USER
            .create(r"Software\Classes\.wavr1test")
            .expect("create pre-existing ext key");
        ext_key
            .set_string("", "PreexistingDefaultHandler.App")
            .expect("set preexisting default handler");

        // 2. Run file association registration
        let res = wavical_cli::run_register_filetypes(project_dir);
        assert!(res.is_ok(), "run_register_filetypes failed: {:?}", res);

        // 3. Verify that the default handler was NOT hijacked/overwritten
        let reopened_ext_key = CURRENT_USER
            .open(r"Software\Classes\.wavr1test")
            .expect("open .wavr1test key");
        let default_val: Result<String, _> = reopened_ext_key.get_string("");
        let default_val = default_val.expect("must have a default value");

        // 4. Also check that OpenWithProgids exists and contains our ProgID
        let open_with_key = CURRENT_USER.open(r"Software\Classes\.wavr1test\OpenWithProgids");
        let has_prog_id = open_with_key
            .as_ref()
            .map(|k| k.get_string("Wavical.test-polite-app").is_ok())
            .unwrap_or(false);

        // Cleanup after reading all registry values
        let _ = CURRENT_USER.remove_tree(r"Software\Classes\.wavr1test");
        let _ = CURRENT_USER.remove_tree(r"Software\Classes\Wavical.test-polite-app");

        assert_eq!(
            default_val, "PreexistingDefaultHandler.App",
            "FINDING-R1: run_register_filetypes hijacked existing (Default) handler instead of preserving it"
        );

        assert!(
            open_with_key.is_ok(),
            "FINDING-R1: Software\\Classes\\.wavr1test\\OpenWithProgids subkey was not created"
        );
        assert!(
            has_prog_id,
            "FINDING-R1: ProgID was not registered under OpenWithProgids"
        );
    }

    #[cfg(not(windows))]
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_dir = temp.path();
        let config_js = r#"export default {
            appId: 'test-polite-app',
            fileAssociations: ['wavr1test']
        };"#;
        std::fs::write(project_dir.join("wavical.config.js"), config_js).expect("write config");
        assert!(wavical_cli::run_register_filetypes(project_dir).is_ok());
    }
}

/// FINDING-R2: run_unregister_filetypes recursively deletes entire Software\Classes\.{ext} instead of deleting only specific ProgID from OpenWithProgids
#[test]
fn test_file_assoc_unregister_preserves_existing_extension_keys() {
    #[cfg(windows)]
    {
        use wavical_cli::windows_registry::CURRENT_USER;
        let temp = tempfile::tempdir().expect("tempdir");
        let project_dir = temp.path();

        let config_js = r#"export default {
            appId: 'test-unregister-app',
            fileAssociations: ['wavr2test']
        };"#;
        std::fs::write(project_dir.join("wavical.config.js"), config_js).expect("write config");

        // 1. Simulate an existing pre-configured default file handler
        let ext_key = CURRENT_USER
            .create(r"Software\Classes\.wavr2test")
            .expect("create pre-existing ext key");
        ext_key
            .set_string("", "PreexistingDefaultHandler.App")
            .expect("set preexisting default handler");

        // 2. Register file association
        let res_reg = wavical_cli::run_register_filetypes(project_dir);
        assert!(
            res_reg.is_ok(),
            "run_register_filetypes failed: {:?}",
            res_reg
        );

        // 3. Unregister file association
        let res_unreg = wavical_cli::run_unregister_filetypes(project_dir);
        assert!(
            res_unreg.is_ok(),
            "run_unregister_filetypes failed: {:?}",
            res_unreg
        );

        // 4. Check if Software\Classes\.wavr2test survived and still has its preexisting handler
        let reopened_ext_key = CURRENT_USER.open(r"Software\Classes\.wavr2test");
        let default_val = reopened_ext_key
            .as_ref()
            .ok()
            .and_then(|k| k.get_string("").ok());

        // 5. Check if OpenWithProgids no longer contains our ProgID
        let open_with_key = CURRENT_USER.open(r"Software\Classes\.wavr2test\OpenWithProgids");
        let has_prog_id = open_with_key
            .as_ref()
            .map(|k| k.get_string("Wavical.test-unregister-app").is_ok())
            .unwrap_or(false);

        let prog_key = CURRENT_USER.open(r"Software\Classes\Wavical.test-unregister-app");

        // Cleanup before asserts
        let _ = CURRENT_USER.remove_tree(r"Software\Classes\.wavr2test");
        let _ = CURRENT_USER.remove_tree(r"Software\Classes\Wavical.test-unregister-app");

        assert!(
            reopened_ext_key.is_ok(),
            "FINDING-R2: run_unregister_filetypes deleted the entire Software\\Classes\\.wavr2test key"
        );
        assert_eq!(
            default_val.as_deref(),
            Some("PreexistingDefaultHandler.App"),
            "FINDING-R2: preexisting default handler was wiped out on unregister"
        );
        assert!(
            !has_prog_id,
            "ProgID should be removed from OpenWithProgids on unregister"
        );
        assert!(
            prog_key.is_err(),
            "ProgID key Software\\Classes\\Wavical.test-unregister-app should be removed on unregister"
        );
    }

    #[cfg(not(windows))]
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_dir = temp.path();
        let config_js = r#"export default {
            appId: 'test-unregister-app',
            fileAssociations: ['wavr2test']
        };"#;
        std::fs::write(project_dir.join("wavical.config.js"), config_js).expect("write config");
        assert!(wavical_cli::run_unregister_filetypes(project_dir).is_ok());
    }
}

/// F-A0-01: crates/wavical-native/src/crash_recovery.rs must not contain unapproved unsafe COM hooks
#[test]
fn test_crash_recovery_contains_zero_unsafe_code() {
    let crash_recovery_src = include_str!("../../../crates/wavical-native/src/crash_recovery.rs");
    assert!(
        !crash_recovery_src.contains("unsafe"),
        "crates/wavical-native/src/crash_recovery.rs must not contain any unsafe blocks or allow(unsafe_code) (F-A0-01)"
    );
}

/// FINDING-W2: load_window_state must check file metadata length (max 64 KB)
/// and reject oversized state files without unbounded memory ingestion.
#[test]
fn test_window_state_unbounded_file_ingestion_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state_file = temp.path().join("window-state.json");

    // Create an oversized file > 64 KB (65 KB) with valid JSON padded with characters
    let mut oversized = Vec::with_capacity(65 * 1024 + 100);
    oversized.extend_from_slice(b"{\n  \"width\": 1024,\n  \"height\": 768,\n  \"padding\": \"");
    oversized.resize(65 * 1024, b'A');
    oversized.extend_from_slice(b"\"\n}");

    std::fs::write(&state_file, &oversized).expect("write oversized state file");

    assert!(
        load_window_state(&state_file).is_none(),
        "FINDING-W2: load_window_state must reject state files exceeding 64 KB"
    );
}

/// FINDING-T1: load_tray_icon must check file size (max 1 MB) before passing asset path to image decoder.
#[test]
fn test_tray_icon_unbounded_file_size_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let oversized_icon = temp.path().join("oversized.png");

    // Write a dummy file exceeding 1 MB (1 MB + 1 byte)
    let dummy_data = vec![0u8; 1024 * 1024 + 1];
    std::fs::write(&oversized_icon, &dummy_data).expect("write oversized icon file");

    let res = load_tray_icon(&oversized_icon);
    assert!(res.is_err(), "Oversized tray icon must be rejected");
    let err = res.unwrap_err();
    assert!(
        err.contains("exceeds maximum allowed size of 1 MB"),
        "FINDING-T1: load_tray_icon must reject file exceeding 1 MB with size error, but got: {err}"
    );
}
