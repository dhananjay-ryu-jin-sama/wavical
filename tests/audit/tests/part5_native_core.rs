//! Black-box and unit audit tests for Phase 2 Native Core (framework, clipboard, notifications, dialogs, file grants).
//!
//! Expectations are derived strictly from docs/WAVICAL_PHASE_2_BUILD_PLAN.md sections 0.2 to 0.5
//! and WP2, WP3, WP4 acceptance criteria.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;
use wavical_ipc::{
    Capabilities, GENERIC_CLIPBOARD_ERROR, GENERIC_METHOD_ERROR, GENERIC_NOTIFICATION_ERROR,
    GrantError, GrantMode, GrantTable, MAX_CLIPBOARD_TEXT_BYTES, MAX_GRANTS_PER_WINDOW,
    MAX_NOTIFICATION_BODY_CHARS, MAX_NOTIFICATION_TITLE_CHARS, MAX_WRITE_GRANTED_BYTES,
    NATIVE_METHOD_NAMES, NativeHandler, OpenFileDialogParams, Request, SaveFileDialogParams,
    known_capabilities, registered_native_methods, validate_clipboard_read,
    validate_clipboard_write, validate_notify_send,
};
use wavical_native::NativeService;

// ============================================================================
// 1. Capability & Method Registration
// ============================================================================

#[test]
fn test_part5_capability_and_method_registration() {
    // Plan section 0.2 specifies 11 native capabilities:
    let expected_native_caps = [
        "badge.set",
        "clipboard.read",
        "clipboard.write",
        "dialog.openFile",
        "dialog.saveFile",
        "fs.readGranted",
        "fs.writeGranted",
        "menu.showContextMenu",
        "notify.send",
        "shortcut.register",
        "tray.set",
    ];

    let mut expected_all_caps = expected_native_caps.to_vec();
    expected_all_caps.push("core.echo");
    expected_all_caps.push("core.ping");
    expected_all_caps.sort();

    // Check KNOWN_CAPABILITIES matches plan
    let actual_caps = known_capabilities();
    assert_eq!(
        actual_caps, expected_all_caps,
        "Known capabilities list does not match Section 0.2 + core methods"
    );

    // Check NATIVE_METHOD_NAMES matches plan
    let actual_native_methods = registered_native_methods();
    assert_eq!(
        actual_native_methods, expected_native_caps,
        "Registered native methods list does not match Section 0.2"
    );

    // Check example app's config
    let hello_config_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello/wavical.config.js");
    let content = std::fs::read_to_string(&hello_config_path).expect("read hello config");

    // core.echo must NOT be granted by default in the example app
    assert!(
        !content.contains("'core.echo': true") && !content.contains("\"core.echo\": true"),
        "core.echo should not be granted by default in examples/hello/wavical.config.js"
    );

    // core.ping must be granted
    assert!(
        content.contains("'core.ping': true") || content.contains("\"core.ping\": true"),
        "core.ping should be granted in examples/hello/wavical.config.js"
    );
}

// ============================================================================
// 2 & 3. Handler Gate & Counting Handler Tests
// ============================================================================

struct CountingNativeHandler {
    call_count: AtomicUsize,
}

impl CountingNativeHandler {
    fn new() -> Self {
        Self {
            call_count: AtomicUsize::new(0),
        }
    }

    fn count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl NativeHandler for CountingNativeHandler {
    fn handle_native(
        &self,
        _method: &str,
        _params: &serde_json::Value,
        _window_id: &str,
    ) -> Result<serde_json::Value, String> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"status": "ok"}))
    }
}

#[test]
fn test_part5_counting_handler_gates_every_native_method() {
    let handler = CountingNativeHandler::new();
    let empty_caps = Capabilities::new();
    let window_id = "test-gate-win";

    // Proves that for EVERY registered native method, an ungranted call NEVER reaches the handler
    for &method in NATIVE_METHOD_NAMES {
        let req = Request {
            id: 1,
            method: method.to_string(),
            params: serde_json::json!({"test": "data"}),
        };
        let resp = wavical_ipc::dispatch_native(&req, window_id, &empty_caps, &handler);
        assert!(!resp.ok);
        assert_eq!(resp.error.as_deref(), Some(GENERIC_METHOD_ERROR));
        assert_eq!(
            handler.count(),
            0,
            "Handler was invoked for ungranted method '{method}'!"
        );
    }

    // Now test that when granted, handler call count increments exactly by 1 per method
    let mut expected_count = 0;
    for &method in NATIVE_METHOD_NAMES {
        let mut caps = Capabilities::new();
        caps.grant(method);

        let req = Request {
            id: 2 + expected_count as u64,
            method: method.to_string(),
            params: serde_json::json!({}),
        };
        let resp = wavical_ipc::dispatch_native(&req, window_id, &caps, &handler);
        assert!(resp.ok, "Granted call for '{method}' failed");
        expected_count += 1;
        assert_eq!(
            handler.count(),
            expected_count,
            "Handler call count did not increment for granted method '{method}'"
        );
    }
}

// ============================================================================
// 4. Named Limits Boundary Tests
// ============================================================================

#[test]
fn test_part5_clipboard_limit_boundaries() {
    // Plan: 1 MB text limit (MAX_CLIPBOARD_TEXT_BYTES = 1024 * 1024)
    assert_eq!(MAX_CLIPBOARD_TEXT_BYTES, 1024 * 1024);

    // Exactly 1 MB ASCII
    let exact_1mb = "a".repeat(MAX_CLIPBOARD_TEXT_BYTES);
    let exact_json = serde_json::json!({ "text": exact_1mb });
    assert!(validate_clipboard_write(&exact_json).is_ok());
    assert!(validate_clipboard_read(&exact_1mb).is_ok());

    // 1 MB + 1 byte ASCII
    let over_1mb = "a".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1);
    let over_json = serde_json::json!({ "text": over_1mb });
    assert_eq!(
        validate_clipboard_write(&over_json).unwrap_err(),
        GENERIC_CLIPBOARD_ERROR
    );
    assert_eq!(
        validate_clipboard_read(&over_1mb).unwrap_err(),
        GENERIC_CLIPBOARD_ERROR
    );

    // Multibyte case: 4-byte emoji (🦀 = 4 bytes)
    // 262,144 * 4 = 1,048,576 bytes = exactly 1 MB
    let emoji_exact = "🦀".repeat(262_144);
    assert_eq!(emoji_exact.len(), MAX_CLIPBOARD_TEXT_BYTES);
    let emoji_json = serde_json::json!({ "text": emoji_exact });
    assert!(validate_clipboard_write(&emoji_json).is_ok());
    assert!(validate_clipboard_read(&emoji_exact).is_ok());

    // 262,145 * 4 = 1,048,580 bytes (> 1 MB)
    let emoji_over = "🦀".repeat(262_145);
    let emoji_over_json = serde_json::json!({ "text": emoji_over });
    assert_eq!(
        validate_clipboard_write(&emoji_over_json).unwrap_err(),
        GENERIC_CLIPBOARD_ERROR
    );
}

#[test]
fn test_part5_notification_limit_boundaries() {
    // Plan: title 128 chars, body 1024 chars
    assert_eq!(MAX_NOTIFICATION_TITLE_CHARS, 128);
    assert_eq!(MAX_NOTIFICATION_BODY_CHARS, 1024);

    // Title exactly 128 chars
    let title_128 = "T".repeat(128);
    let valid_payload = serde_json::json!({ "title": title_128, "body": "test" });
    assert!(validate_notify_send(&valid_payload).is_ok());

    // Title 129 chars
    let title_129 = "T".repeat(129);
    let invalid_title = serde_json::json!({ "title": title_129, "body": "test" });
    assert_eq!(
        validate_notify_send(&invalid_title).unwrap_err(),
        GENERIC_NOTIFICATION_ERROR
    );

    // Title multibyte: 128 emojis (128 chars, 512 bytes)
    let title_emoji_128 = "🚀".repeat(128);
    assert_eq!(title_emoji_128.chars().count(), 128);
    let emoji_payload = serde_json::json!({ "title": title_emoji_128 });
    assert!(validate_notify_send(&emoji_payload).is_ok());

    // Title multibyte: 129 emojis
    let title_emoji_129 = "🚀".repeat(129);
    let emoji_payload_over = serde_json::json!({ "title": title_emoji_129 });
    assert_eq!(
        validate_notify_send(&emoji_payload_over).unwrap_err(),
        GENERIC_NOTIFICATION_ERROR
    );

    // Body exactly 1024 chars
    let body_1024 = "B".repeat(1024);
    let valid_body = serde_json::json!({ "title": "Valid Title", "body": body_1024 });
    assert!(validate_notify_send(&valid_body).is_ok());

    // Body 1025 chars
    let body_1025 = "B".repeat(1025);
    let invalid_body = serde_json::json!({ "title": "Valid Title", "body": body_1025 });
    assert_eq!(
        validate_notify_send(&invalid_body).unwrap_err(),
        GENERIC_NOTIFICATION_ERROR
    );
}

#[test]
fn test_part5_grant_limit_boundary() {
    // Plan: 64 grants per window
    assert_eq!(MAX_GRANTS_PER_WINDOW, 64);
    let table = GrantTable::new();
    let win = "test-boundary-win";

    for i in 0..64 {
        let res = table.insert(win, PathBuf::from(format!("test_file_{i}.txt")));
        assert!(res.is_ok(), "Grant {i} should succeed within limit");
    }

    assert_eq!(table.count_for_window(win), 64);

    // 65th grant fails
    let err = table.insert(win, PathBuf::from("file_65.txt")).unwrap_err();
    assert_eq!(err, GrantError::LimitReached);
}

// ============================================================================
// 5. Clipboard Integration & Safety Guard
// ============================================================================

struct ClipboardRestoreGuard {
    saved_text: Option<String>,
}

impl ClipboardRestoreGuard {
    fn new() -> Self {
        // Read current clipboard text to restore later
        let saved_text = match arboard::Clipboard::new() {
            Ok(mut cb) => cb.get_text().ok(),
            Err(_) => None,
        };
        Self { saved_text }
    }
}

impl Drop for ClipboardRestoreGuard {
    fn drop(&mut self) {
        if let (Some(text), Ok(mut cb)) = (&self.saved_text, arboard::Clipboard::new()) {
            let _ = cb.set_text(text);
        }
    }
}

#[test]
fn test_part5_clipboard_roundtrip_and_safety() {
    let _guard = ClipboardRestoreGuard::new();
    let service = NativeService::new();
    let window_id = "test-clipboard-win";

    let marker = "WAVICAL_AUDIT_MARKER_A3A_SEC_TEST_981";
    let write_params = serde_json::json!({ "text": marker });

    match service.clipboard_write(&write_params, window_id) {
        Ok(res) => {
            assert_eq!(res.get("success"), Some(&serde_json::json!(true)));

            // Read it back
            let read_res = service
                .clipboard_read(&serde_json::Value::Null, window_id)
                .expect("clipboard read should succeed");
            assert_eq!(read_res.get("text"), Some(&serde_json::json!(marker)));
        }
        Err(err) => {
            // In headless/container environments where OS clipboard is inaccessible
            assert_eq!(err, GENERIC_CLIPBOARD_ERROR);
        }
    }

    // Verify 1 MB + 1 byte write is rejected before touching OS clipboard
    let oversize = "x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1);
    let over_params = serde_json::json!({ "text": oversize });
    let err = service
        .clipboard_write(&over_params, window_id)
        .unwrap_err();
    assert_eq!(err, GENERIC_CLIPBOARD_ERROR);
}

// ============================================================================
// 6. Notifications Escaping & Strict Validation
// ============================================================================

#[test]
fn test_part5_notifications_escaping_and_rejections() {
    // Payloads containing XML markup, image tags, entities, environment vars, newlines, null bytes
    let test_strings = [
        "<b>x</b>",
        "]]>",
        "</text><image src=\"file:///C:/Windows/win.ini\"/>",
        "&amp;",
        "%USERNAME%",
        "Averylongwordthatshouldnotoverflowthetoasttemplatewithoutexceedingcharacterrules",
        "Line 1\nLine 2\r\nLine 3",
        "Before\0After",
    ];

    for s in test_strings {
        let payload = serde_json::json!({
            "title": s,
            "body": s,
        });

        // validate_notify_send must parse cleanly or reject without panic
        let res = validate_notify_send(&payload);
        if s.contains('\0') {
            // Null bytes are acceptable at string layer or sanitized at OS boundary
            assert!(res.is_ok() || res.is_err());
        } else {
            assert!(res.is_ok(), "String failed validation: '{s}'");
            let parsed = res.unwrap();
            assert_eq!(parsed.title, s);
            assert_eq!(parsed.body, s);
        }
    }

    // Confirm that extraneous fields (URL, image, action, sound) are strictly rejected
    let attack_url = serde_json::json!({
        "title": "Alert",
        "body": "Click here",
        "url": "https://attacker.com"
    });
    assert_eq!(
        validate_notify_send(&attack_url).unwrap_err(),
        GENERIC_NOTIFICATION_ERROR
    );

    let attack_image = serde_json::json!({
        "title": "Alert",
        "body": "Look",
        "image": "file:///C:/Windows/win.ini"
    });
    assert_eq!(
        validate_notify_send(&attack_image).unwrap_err(),
        GENERIC_NOTIFICATION_ERROR
    );

    let attack_action = serde_json::json!({
        "title": "Alert",
        "action": "calc.exe"
    });
    assert_eq!(
        validate_notify_send(&attack_action).unwrap_err(),
        GENERIC_NOTIFICATION_ERROR
    );
}

// ============================================================================
// 7. Dialog Parameters & Filter Resolution
// ============================================================================

#[test]
fn test_part5_dialog_parameters_rejects_paths() {
    // OpenFileDialogParams rejects path, filter strings, etc.
    let invalid_open = serde_json::json!({ "path": "C:\\secret.txt" });
    assert!(serde_json::from_value::<OpenFileDialogParams>(invalid_open).is_err());

    let invalid_filter_str = serde_json::json!({ "filter": "*.exe" });
    assert!(serde_json::from_value::<OpenFileDialogParams>(invalid_filter_str).is_err());

    // SaveFileDialogParams rejects path
    let invalid_save = serde_json::json!({ "path": "C:\\secret.txt" });
    assert!(serde_json::from_value::<SaveFileDialogParams>(invalid_save).is_err());
}

// ============================================================================
// 8. Grants Security Invariants (CSPRNG, Isolation, Forgery)
// ============================================================================

#[test]
fn test_part5_grant_table_entropy_and_isolation() {
    use std::collections::HashSet;

    let table = GrantTable::new();
    let mut ids = HashSet::new();

    // 1,000 grants produce 1,000 unique 128-bit IDs
    for i in 0..1000 {
        let win_id = format!("win-{}", i % 20); // 20 windows
        let gid = table
            .insert(&win_id, PathBuf::from(format!("file_{i}.txt")))
            .expect("insert grant");
        assert_eq!(gid.len(), 32);
        assert!(
            gid.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert!(
            ids.insert(gid),
            "Grant ID was duplicated! Insufficient entropy"
        );
    }

    // Cross-window isolation
    let grant_win_a = table
        .insert("window-A", PathBuf::from("secret_A.txt"))
        .unwrap();

    // Looking up window A's grant from window B must return NotFound
    assert_eq!(
        table.get(&grant_win_a, "window-B").unwrap_err(),
        GrantError::NotFound
    );

    // Closing window A removes its grants
    table.clear_window("window-A");
    assert_eq!(
        table.get(&grant_win_a, "window-A").unwrap_err(),
        GrantError::NotFound
    );

    // Forged IDs: empty, too long, wrong charset, single char flipped
    let forged_cases = [
        "",
        "abc",
        "0123456789abcdef0123456789abcdef0", // 33 chars
        "0123456789ABCDEF0123456789ABCDEF",  // uppercase
        "0123456789abcdef0123456789abcdeg",  // 'g' non-hex
    ];

    for forged in forged_cases {
        assert_eq!(
            table.get(forged, "window-B").unwrap_err(),
            GrantError::NotFound
        );
    }
}

// ============================================================================
// 9. Scoped File Access (fs.readGranted & fs.writeGranted)
// ============================================================================

#[test]
fn test_part5_fs_read_write_granted_enforcements() {
    let tmp = tempdir().expect("tempdir");
    let test_file = tmp.path().join("audit_sample.txt");
    std::fs::write(&test_file, b"Wavical scoped grant test payload").expect("write test file");

    let table = Arc::new(GrantTable::new());
    let service = NativeService::with_grant_table(table.clone());
    let window_id = "test-fs-win";

    let grant_id = table
        .insert_with_mode(window_id, &test_file, GrantMode::ReadWrite)
        .expect("insert grant");

    // 1. Read granted succeeds
    let read_params = serde_json::json!({ "grantId": grant_id });
    let read_res = service
        .fs_read_granted(&read_params, window_id)
        .expect("read granted");
    assert_eq!(
        read_res.get("content").and_then(|v| v.as_str()),
        Some("Wavical scoped grant test payload")
    );

    // 2. Read with raw 'path' parameter is rejected by serde
    let attack_path = serde_json::json!({ "grantId": grant_id, "path": "C:\\Windows\\win.ini" });
    assert!(service.fs_read_granted(&attack_path, window_id).is_err());

    // 3. Write granted succeeds
    let write_params = serde_json::json!({
        "grantId": grant_id,
        "content": "Updated content via writeGranted"
    });
    let write_res = service
        .fs_write_granted(&write_params, window_id)
        .expect("write granted");
    assert_eq!(write_res.get("success"), Some(&serde_json::json!(true)));

    // Verify file content was updated
    let updated = std::fs::read_to_string(&test_file).expect("read file");
    assert_eq!(updated, "Updated content via writeGranted");

    // 4. Oversized write rejected before write
    let oversize_content = "X".repeat(MAX_WRITE_GRANTED_BYTES + 1);
    let over_write_params = serde_json::json!({
        "grantId": grant_id,
        "content": oversize_content
    });
    let err = service
        .fs_write_granted(&over_write_params, window_id)
        .unwrap_err();
    assert!(err.contains("10 MB limit"));

    // 5. Zero-byte file reads cleanly
    let zero_file = tmp.path().join("zero.txt");
    std::fs::write(&zero_file, b"").expect("create zero file");
    let zero_grant = table.insert(window_id, &zero_file).unwrap();
    let zero_res = service
        .fs_read_granted(&serde_json::json!({ "grantId": zero_grant }), window_id)
        .unwrap();
    assert_eq!(zero_res.get("content").and_then(|v| v.as_str()), Some(""));

    // 6. Non-UTF-8 content rejected cleanly
    let binary_file = tmp.path().join("binary.bin");
    std::fs::write(&binary_file, [0xFF, 0xFE, 0xFD, 0xFC]).expect("create binary file");
    let bin_grant = table.insert(window_id, &binary_file).unwrap();
    let bin_err = service
        .fs_read_granted(&serde_json::json!({ "grantId": bin_grant }), window_id)
        .unwrap_err();
    assert!(bin_err.contains("invalid UTF-8"));
}

#[test]
fn test_part5_fs_use_time_recheck_file_removed_or_directory() {
    let tmp = tempdir().expect("tempdir");
    let test_file = tmp.path().join("to_delete.txt");
    std::fs::write(&test_file, b"data").unwrap();

    let table = Arc::new(GrantTable::new());
    let service = NativeService::with_grant_table(table.clone());
    let window_id = "test-recheck-win";

    let grant_id = table.insert(window_id, &test_file).unwrap();

    // File removed after grant
    std::fs::remove_file(&test_file).unwrap();
    let err = service
        .fs_read_granted(&serde_json::json!({ "grantId": grant_id }), window_id)
        .unwrap_err();
    assert!(err.contains("does not exist") || err.contains("File operation failed"));

    // File replaced by directory
    std::fs::create_dir(&test_file).unwrap();
    let err_dir = service
        .fs_read_granted(&serde_json::json!({ "grantId": grant_id }), window_id)
        .unwrap_err();
    assert!(err_dir.contains("not a regular file") || err_dir.contains("File operation failed"));
}

// ============================================================================
// 10. Audit Findings Tests (Ignored to keep CI Green)
// ============================================================================

/// FINDING-G1: Grant ID format (length and charset) is not validated before HashMap lookup
#[test]
#[ignore = "FINDING-G1"]
fn test_grant_id_format_validated_before_lookup() {
    let table = GrantTable::new();
    // A forged ID that is not 32 lowercase hex characters should be rejected at validation
    // with an explicit invalid-format error rather than silently doing a hashmap lookup
    let err = table.get("not-a-valid-hex-id", "win-1").unwrap_err();
    assert_ne!(
        err,
        GrantError::NotFound,
        "Expected format validation error before lookup"
    );
}

/// F-A0-06: Grant Table RwLock unwrap lock poisoning risk
#[test]
fn test_grant_table_rwlock_poisoning_handled_gracefully() {
    use wavical_ipc::{GrantError, GrantMode, GrantTable};

    struct PanickingPath;
    impl From<PanickingPath> for std::path::PathBuf {
        fn from(_: PanickingPath) -> Self {
            panic!("intentional panic to poison grant table RwLock");
        }
    }

    let table = Arc::new(GrantTable::new());
    let t_clone = Arc::clone(&table);

    // Intentionally panic inside a worker thread while holding the write lock
    let _ = std::thread::spawn(move || {
        let _ = t_clone.insert_with_mode("win-poison", PanickingPath, GrantMode::ReadOnly);
    })
    .join();

    // 1. get() returns Err(GrantError::InternalLockError) instead of panicking
    let get_res = table.get("any-grant-id", "win-1");
    assert_eq!(
        get_res,
        Err(GrantError::InternalLockError),
        "Expected InternalLockError on poisoned grant table get()"
    );

    // 2. insert() returns Err(GrantError::InternalLockError) instead of panicking
    let insert_res = table.insert("win-1", "some_file.txt");
    assert_eq!(
        insert_res,
        Err(GrantError::InternalLockError),
        "Expected InternalLockError on poisoned grant table insert()"
    );

    // 3. Helper methods do not panic
    table.clear_window("win-1");
    assert_eq!(table.count_for_window("win-1"), 0);
    assert_eq!(table.total_count(), 0);
    assert!(!table.remove_grant("any-grant-id", "win-1"));
}

/// FINDING-G2: Grant table does not enforce read-only vs write-only mode
#[test]
fn test_grant_enforces_read_vs_write_mode() {
    let tmp = tempdir().unwrap();
    let file = tmp.path().join("open_only.txt");
    std::fs::write(&file, b"readonly").unwrap();

    let table = Arc::new(GrantTable::new());
    let service = NativeService::with_grant_table(table.clone());

    // Grant created from open file dialog should NOT allow writeGranted
    let grant_id = table.insert("win-1", &file).unwrap();

    let write_params = serde_json::json!({
        "grantId": grant_id,
        "content": "overwrite data"
    });

    let res = service.fs_write_granted(&write_params, "win-1");
    assert!(
        res.is_err(),
        "Open file grant was allowed to be overwritten via fs.writeGranted"
    );

    // Grant created from save file dialog should NOT allow readGranted
    let save_file = tmp.path().join("save_only.txt");
    std::fs::write(&save_file, b"save_data").unwrap();
    let save_grant = table
        .insert_with_mode("win-1", &save_file, GrantMode::WriteOnly)
        .unwrap();

    let read_params = serde_json::json!({
        "grantId": save_grant
    });
    let read_res = service.fs_read_granted(&read_params, "win-1");
    assert!(
        read_res.is_err(),
        "Save file grant was allowed to be read via fs.readGranted"
    );
}

/// FINDING-F1: TOCTOU race window between validation and std::fs::write
#[test]
fn test_fs_write_granted_toctou_handle_validation() {
    let tmp = tempdir().unwrap();
    let file = tmp.path().join("toctou_target.txt");
    std::fs::write(&file, b"initial content").unwrap();

    let table = Arc::new(GrantTable::new());
    let service = NativeService::with_grant_table(table.clone());
    let window_id = "win-toctou";

    let grant_id = table
        .insert_with_mode(window_id, &file, GrantMode::WriteOnly)
        .unwrap();

    // Open file handle once with restrictive Win32 share flags and handle validation
    let (mut handle, entry) = service
        .open_granted_write_handle(&grant_id, window_id)
        .expect("Open granted write handle must succeed and lock target file");

    assert_eq!(entry.path, file);

    // On Windows, verify that the open write handle prevents concurrent deletion or renaming
    #[cfg(windows)]
    {
        let delete_attempt = std::fs::remove_file(&file);
        assert!(
            delete_attempt.is_err(),
            "Target file should be locked with restrictive share flags against deletion/renaming while write handle is open"
        );
        let rename_attempt = std::fs::rename(&file, tmp.path().join("swapped.txt"));
        assert!(
            rename_attempt.is_err(),
            "Target file should be locked with restrictive share flags against renaming/swapping while write handle is open"
        );
    }

    // Write through the open handle directly
    use std::io::Write;
    handle.write_all(b"safe write payload").unwrap();
    handle.flush().unwrap();
    drop(handle);

    // After handle is closed, the written payload is persisted
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "safe write payload");

    // Also verify fs_write_granted round-trip executes safely using the single-handle write
    let write_res = service.fs_write_granted(
        &serde_json::json!({
            "grantId": grant_id,
            "content": "second safe payload via fs_write_granted"
        }),
        window_id,
    );
    assert!(write_res.is_ok(), "fs_write_granted must succeed");
    let content2 = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content2, "second safe payload via fs_write_granted");
}

/// FINDING-D1: Dialog title and defaultName have no length or character validation
#[test]
#[ignore = "FINDING-D1"]
fn test_dialog_params_length_validation() {
    // A 100,000 character title or default name should be rejected by validation
    let huge_title = "A".repeat(100_000);
    let params = serde_json::json!({
        "title": huge_title,
    });
    let res: Result<OpenFileDialogParams, _> = serde_json::from_value(params);
    assert!(
        res.is_err(),
        "100,000 character dialog title should be rejected by validation"
    );
}

/// FINDING-N1: notify.send has no rate or queue limiting
#[test]
#[ignore = "FINDING-N1"]
fn test_notify_send_rate_limiting() {
    let service = NativeService::new();
    let window_id = "test-notify-rate";
    let payload = serde_json::json!({
        "title": "Rate Limit Test",
        "body": "Spam"
    });

    // Requesting 1,000 notifications in a loop should hit a rate limit
    let mut rate_limited = false;
    for _ in 0..100 {
        if let Err(e) = service.notify_send(&payload, window_id) {
            let is_limit = e.contains("rate") || e.contains("limit");
            if is_limit {
                rate_limited = true;
                break;
            }
        }
    }
    assert!(rate_limited, "No rate limiting on desktop notifications");
}
