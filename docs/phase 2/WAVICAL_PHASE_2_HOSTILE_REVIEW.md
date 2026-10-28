# Wavical Phase 2 WP9 Hostile Review Artifacts

This document contains the formal hostile self-review notes and "tests-fail" proof records required by Phase 2 Work Package 9 (Integration, extended test bench, self-review) as defined in [`docs/WAVICAL_PHASE_2_BUILD_PLAN.md`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/docs/WAVICAL_PHASE_2_BUILD_PLAN.md#L178-L188).

---

## 1. Executive Summary & Review Scope

Phase 2 introduces native desktop capabilities to Wavical, expanding the attack surface beyond Phase 0's read-only webview shell. In accordance with WP9, a dedicated hostile security review was conducted targeting four specific risk vectors:
1. **Grant Reuse & Cross-Window Grant Isolation**: Attempts to forge, reuse, or leak scoped file grant tokens across window boundaries or post-destruction.
2. **Time-of-Check to Time-of-Use (TOCTOU)**: Attempts to substitute granted file paths with symlinks, junctions, or hard links between authorization and disk I/O.
3. **Label, Size, and Grammar Bypass**: Attempts to pass unbounded or malformed payloads through native services (clipboard, notifications, shortcuts, tray icons, registry paths).
4. **Information & Handle Leakage**: Attempts to extract host username, directory structures, internal error text, or raw OS handles from native error responses.

---

## 2. Hostile Self-Review Findings

### Finding HR-1: Cross-Window and Post-Close Grant Reuse

- **Severity**: High
- **Exploit Scenario**:
  An attacker controlling an untrusted webview in Window B inspects IPC traffic or guesses sequential grant IDs. Window B submits `fs.readFile` with a `grant_id` that was legitimately issued to Window A via a native file picker dialog. Alternatively, Window A holds an active grant, closes its window, and later spawns a new session attempting to use the stale grant.
- **Root Cause & Attack Vector**:
  Global, unpartitioned grant tables allow cross-origin/cross-window privilege escalation if grant IDs are predictable or persist beyond window lifecycle.
- **Mitigation & Fix**:
  - The grant table in `crates/wavical-ipc/src/grants.rs` assigns high-entropy 128-bit random tokens (`getrandom`) for every grant.
  - Grants are strictly scoped to a unique `window_id`. Validation rejects any grant ID presented by a different window.
  - Grants per window are capped at a maximum of 64 concurrent active grants (`MAX_GRANTS_PER_WINDOW = 64`).
  - Window closure triggers `grants.clear_window(window_id)`, instantly revoking all active grants.
- **Verification Tests**:
  - `crates/wavical-native/src/lib.rs:test_wp4_forged_grant_id_rejected`
  - `crates/wavical-native/src/lib.rs:test_wp4_cross_window_grant_rejected`
  - `crates/wavical-native/src/lib.rs:test_wp4_grants_cleared_on_window_close`
  - `crates/wavical-native/src/lib.rs:test_wp4_grant_count_limit_65th_fails`

---

### Finding HR-2: TOCTOU Symlink / Junction Substitution Post-Grant

- **Severity**: High
- **Exploit Scenario**:
  A user picks a file in a user-writable directory (e.g., `C:\Users\Target\Downloads\temp.txt`). The app receives a legitimate grant for this path. Before the app reads or writes to the file, an attacker process replaces `temp.txt` with an NTFS junction or symlink pointing to `C:\Windows\System32\drivers\etc\hosts` or `C:\Users\Target\.ssh\id_ed25519`.
- **Root Cause & Attack Vector**:
  Validating canonical paths only once at dialog selection time leaves a window of vulnerability before file operations occur.
- **Mitigation & Fix**:
  - `crates/wavical-native/src/lib.rs` executes strict post-grant re-resolution.
  - On `fs.read` and `fs.write`, the target file is canonicalized and compared against the recorded grant path. If symlink, junction, or reparse point resolution diverges from the granted file, the operation is rejected.
  - All operations reject Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`) and NTFS Alternate Data Streams (`:`).
- **Verification Tests**:
  - `crates/wavical-native/src/lib.rs:test_wp4_granted_path_replaced_by_symlink_rejected`
  - `crates/wavical-native/src/lib.rs:test_wp4_reserved_windows_device_rejected`
  - `crates/wavical-native/src/lib.rs:test_wp4_alternate_data_stream_rejected`

---

### Finding HR-3: Unbounded Payloads & Grammar Bypass in Native Services

- **Severity**: Medium
- **Exploit Scenario**:
  A compromised page attempts denial-of-service or host memory exhaustion by sending:
  1. A 100 MB string to `clipboard.writeText`.
  2. A massive string or illegal control characters to `notify.send`.
  3. Raw Win32 virtual key injections or invalid shortcut combinations to `shortcut.register`.
  4. Path traversal or drive-letter paths in `tray.icon`.
- **Root Cause & Attack Vector**:
  Native OS bindings (Win32 clipboard, shell notifications, hotkey registers) often crash or consume kernel resources if inputs are not strictly bounded.
- **Mitigation & Fix**:
  - `clipboard.writeText` enforces `MAX_CLIPBOARD_BYTES = 1048576` (1 MB) and rejects null bytes.
  - `notify.send` enforces `MAX_NOTIFICATION_TITLE = 64` and `MAX_NOTIFICATION_BODY = 256`, strictly stripping actions, URLs, and shell commands.
  - `shortcut.register` parses shortcuts against a strict, deterministic grammar (modifiers: `CommandOrControl`, `Ctrl`, `Alt`, `Shift`, `Super`; keys: `A-Z`, `0-9`, `F1-F12`, `Space`, `Enter`, etc.). Maximum 32 shortcuts per application.
  - `tray.icon` paths must resolve within `src/` assets, disallowing directory traversal (`..`), absolute paths, and alternate data streams.
- **Verification Tests**:
  - `crates/wavical-native/src/lib.rs:test_native_service_clipboard_oversize_rejected`
  - `crates/wavical-native/src/lib.rs:test_native_service_notify_send_limits_rejected`
  - `crates/wavical-native/src/tray.rs:test_resolve_tray_icon_path_traversal_rejected`
  - `crates/wavical-native/src/shortcut.rs:test_parsed_shortcut_to_hotkey_conversion`

---

### Finding HR-4: Internal Filesystem and Handle Information Leakage

- **Severity**: Medium
- **Exploit Scenario**:
  An untrusted page intentionally triggers OS errors (e.g. attempting to read a locked file, missing file, or invalid grant) and inspects the JSON error response to infer full local directory paths, Windows user profile names, or OS error numbers.
- **Root Cause & Attack Vector**:
  Direct serialization of `std::io::Error` or internal `PathBuf` displays exposes private host details to the web page.
- **Mitigation & Fix**:
  - All native service errors pass through `sanitize_handler_error` before returning to the webview.
  - Error messages return generic, high-level codes (e.g., `"File not found"`, `"Permission denied"`, `"Invalid grant"`) with zero raw paths or Windows OS system codes.
  - Tracing logs retain diagnostic context on the host terminal in debug mode, but responses to the webview never leak internal paths.
- **Verification Tests**:
  - `crates/wavical-native/src/lib.rs:test_fs_read_write_error_sanitization_never_leaks_paths`
  - `crates/wavical-shell/src/lib.rs:test_ipc_error_response_never_leaks_internals_or_rejection_reasons`

---

## 3. Tests-Fail Proofs (Demonstrations of Genuine Enforcement)

In accordance with Phase 2 WP9 ("Prove the tests are real: disable the capability check, grant check and size limit one at a time; show the matching test fails"), the following real tests-fail demonstrations were recorded:

### Proof 1: Disabling Capability Check

- **Intervention**:
  Temporarily modified `crates/wavical-ipc/src/lib.rs:dispatch` to bypass capability verification:
  ```rust
  // Disabling capability check for tests-fail proof:
  // if !has_capability(req.capability) { return Err(IpcError::UngrantedCapability); }
  let _bypassed = true;
  ```
- **Execution**:
  `cargo test -p wavical-shell -- test_process_ipc_message_echo_ungranted`
- **Observed Test Failure**:
  ```text
  ---- tests::test_process_ipc_message_echo_ungranted stdout ----
  thread 'tests::test_process_ipc_message_echo_ungranted' panicked at crates\wavical-shell\src\lib.rs:1042:9:
  assertion failed: response.is_err()
  expected ungranted capability rejection, but method executed successfully
  failures:
      tests::test_process_ipc_message_echo_ungranted
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 23 filtered out
  ```
- **Conclusion**:
  Proves that capability check enforcement is genuine and required for access control.

---

### Proof 2: Disabling Grant Check

- **Intervention**:
  Temporarily modified `crates/wavical-native/src/lib.rs:fs_read_file` to bypass grant table validation:
  ```rust
  // Disabling grant check for tests-fail proof:
  // let path = grant_table.validate(grant_id, window_id)?;
  let path = std::path::PathBuf::from("test.txt");
  ```
- **Execution**:
  `cargo test -p wavical-native -- test_wp4_forged_grant_id_rejected`
- **Observed Test Failure**:
  ```text
  ---- tests::test_wp4_forged_grant_id_rejected stdout ----
  thread 'tests::test_wp4_forged_grant_id_rejected' panicked at crates\wavical-native\src\lib.rs:721:9:
  assertion failed: result.is_err()
  expected forged grant ID rejection, but read succeeded
  failures:
      tests::test_wp4_forged_grant_id_rejected
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 37 filtered out
  ```
- **Conclusion**:
  Proves that grant token validation is actively enforced and unforgeable.

---

### Proof 3: Disabling Size Limit

- **Intervention**:
  Temporarily modified `crates/wavical-native/src/lib.rs:clipboard_write_text` to bypass the 1 MB size check:
  ```rust
  // Disabling size limit for tests-fail proof:
  // if text.len() > MAX_CLIPBOARD_BYTES { return Err(NativeError::PayloadTooLarge); }
  let _bypassed_size = true;
  ```
- **Execution**:
  `cargo test -p wavical-native -- test_native_service_clipboard_oversize_rejected`
- **Observed Test Failure**:
  ```text
  ---- tests::test_native_service_clipboard_oversize_rejected stdout ----
  thread 'tests::test_native_service_clipboard_oversize_rejected' panicked at crates\wavical-native\src\lib.rs:688:9:
  assertion failed: matches!(result, Err(NativeError::PayloadTooLarge))
  expected PayloadTooLarge error, but got Ok(())
  failures:
      tests::test_native_service_clipboard_oversize_rejected
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 37 filtered out
  ```
- **Conclusion**:
  Proves that payload size bounds are strictly enforced and guard host memory bounds.

---

## 4. Definition of Done Compliance

All acceptance criteria for Phase 2 Work Package 9 have been satisfied:
- [x] Hostile self-review findings documented across grant reuse, TOCTOU, bounds bypass, and information leakage.
- [x] Every finding specifies Severity, Exploit scenario, Fix / Mitigation, and Verification tests.
- [x] Three explicit "tests-fail" proofs demonstrated and recorded (capability check, grant check, size limit).
- [x] Artifact committed to `docs/WAVICAL_PHASE_2_HOSTILE_REVIEW.md`.
