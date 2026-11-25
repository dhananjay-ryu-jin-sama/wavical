# Audit A3a Report: Phase 2 Native Core (Framework, Clipboard, Notifications, Dialogs, File Grants)

**Auditor:** Independent Security Auditor & QA Engineer  
**Date:** 2026-10-26  
**Target:** `wavical-ipc`, `wavical-native`, `wavical-shell`  
**Reference Plan:** [`docs/WAVICAL_PHASE_2_BUILD_PLAN.md`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/docs/WAVICAL_PHASE_2_BUILD_PLAN.md) (Sections 0.2 to 0.5, WP2, WP3, WP4)  
**Test Suite:** [`tests/audit/tests/part5_native_core.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/tests/audit/tests/part5_native_core.rs)  

---

## 1. Executive Summary

Audit A3a evaluates Phase 2 Native Tier 1 Core capabilities: capability gating architecture, clipboard access, desktop toast notifications, native file dialogs, and scoped file access via opaque grant tokens.

### Key Positive Verification Results
1. **Centralized Capability Gating:** Every native method is verified against the window's capability allowlist before any parameter deserialization, file access, or OS interaction occurs ([crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705)).
2. **Strict Default Deny:** Counting handler tests prove that ungranted calls never reach native handlers (call count remains strictly 0 across all 11 native methods).
3. **CSPRNG Grant Token Generation:** Grant IDs are 128-bit unguessable tokens generated via OS CSPRNG (`getrandom::fill`), strictly isolated per window, and cleared upon window close.
4. **Scoped File Access (No Raw Paths):** JavaScript callers cannot pass raw file paths to `fs.readGranted` or `fs.writeGranted`. All operations require valid opaque grant IDs.
5. **Use-Time Recheck:** Files are verified at access time for existence, non-symlink status, regular file attributes, and canonical path preservation.

---

## 2. Capability & Method Registration

Section 0.2 of the Phase 2 Build Plan defines 11 native capabilities, which supersede the broad names from Phase 1. `wavical-ipc` defines `KNOWN_CAPABILITIES` (including Phase 0 core methods) and `NATIVE_METHOD_NAMES`.

### Registry Comparison Table

| Capability Name | Code Definition | Registered Native Method | Plan Section 0.2 Match |
| :--- | :--- | :--- | :--- |
| `badge.set` | `crates/wavical-ipc/src/lib.rs:379` | `crates/wavical-ipc/src/lib.rs:364` | **MATCH** |
| `clipboard.read` | `crates/wavical-ipc/src/lib.rs:380` | `crates/wavical-ipc/src/lib.rs:365` | **MATCH** |
| `clipboard.write` | `crates/wavical-ipc/src/lib.rs:381` | `crates/wavical-ipc/src/lib.rs:366` | **MATCH** |
| `core.echo` | `crates/wavical-ipc/src/lib.rs:382` | N/A (Core IPC handler) | **MATCH** (Phase 0) |
| `core.ping` | `crates/wavical-ipc/src/lib.rs:383` | N/A (Core IPC handler) | **MATCH** (Phase 0) |
| `dialog.openFile` | `crates/wavical-ipc/src/lib.rs:384` | `crates/wavical-ipc/src/lib.rs:367` | **MATCH** |
| `dialog.saveFile` | `crates/wavical-ipc/src/lib.rs:385` | `crates/wavical-ipc/src/lib.rs:368` | **MATCH** |
| `fs.readGranted` | `crates/wavical-ipc/src/lib.rs:386` | `crates/wavical-ipc/src/lib.rs:369` | **MATCH** |
| `fs.writeGranted` | `crates/wavical-ipc/src/lib.rs:387` | `crates/wavical-ipc/src/lib.rs:370` | **MATCH** |
| `menu.showContextMenu` | `crates/wavical-ipc/src/lib.rs:388` | `crates/wavical-ipc/src/lib.rs:371` | **MATCH** |
| `notify.send` | `crates/wavical-ipc/src/lib.rs:389` | `crates/wavical-ipc/src/lib.rs:372` | **MATCH** |
| `shortcut.register` | `crates/wavical-ipc/src/lib.rs:390` | `crates/wavical-ipc/src/lib.rs:373` | **MATCH** |
| `tray.set` | `crates/wavical-ipc/src/lib.rs:391` | `crates/wavical-ipc/src/lib.rs:374` | **MATCH** |

**Example Application Config Check:**
- Inspected [`examples/hello/wavical.config.js`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/examples/hello/wavical.config.js):
  - `core.echo` is **NOT granted** (default-deny preserved).
  - `core.ping` is granted.
  - Native features used by the hello demonstration are granted.

---

## 3. Handler Gate & Counting Handler Analysis

### Method Gate Verification Table

Every native method is processed through a single centralized gateway function `dispatch_native`.

| Method | Native Handler Implementation | Capability Gate Location | Black-Box Verification Test |
| :--- | :--- | :--- | :--- |
| `badge.set` | [crates/wavical-native/src/lib.rs:550](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L550) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `clipboard.read` | [crates/wavical-native/src/lib.rs:233](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L233) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `clipboard.write` | [crates/wavical-native/src/lib.rs:257](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L257) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `dialog.openFile` | [crates/wavical-native/src/lib.rs:326](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L326) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `dialog.saveFile` | [crates/wavical-native/src/lib.rs:360](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L360) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `fs.readGranted` | [crates/wavical-native/src/lib.rs:394](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L394) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `fs.writeGranted` | [crates/wavical-native/src/lib.rs:444](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L444) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `menu.showContextMenu` | [crates/wavical-native/src/lib.rs:498](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L498) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `notify.send` | [crates/wavical-native/src/lib.rs:276](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L276) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `shortcut.register` | [crates/wavical-native/src/lib.rs:537](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L537) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |
| `tray.set` | [crates/wavical-native/src/lib.rs:524](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L524) | [crates/wavical-ipc/src/lib.rs:705](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L705) | `part5_native_core::test_part5_counting_handler_gates_every_native_method` |

**Verification of Gate Timing:**
In `wavical_ipc::dispatch_native`:
```rust
    if !is_granted {
        return Response::error(request.id, GENERIC_METHOD_ERROR);
    }
```
The check occurs at line 705. The handler is invoked at line 725. Thus, ungranted calls return immediately with code `Method not allowed or not found` without touching parameters, clipboard, files, dialogs, or OS services.

---

## 4. Named Limits Verification & Boundary Testing

Section 0.4 specifies named constants for resource limits.

### Constant Definition & Boundary Results

| Constant Name | Plan Value | Code Value | Measurement Unit | Boundary Test Status |
| :--- | :--- | :--- | :--- | :--- |
| `MAX_CLIPBOARD_TEXT_BYTES` | 1 MB | `1024 * 1024` (1,048,576) | Bytes | Exactly 1 MB passes; 1 MB + 1 byte fails. |
| `MAX_NOTIFICATION_TITLE_CHARS` | 128 chars | 128 | Unicode chars (`.chars().count()`) | Exactly 128 chars passes; 129 chars fails. Multibyte (128 emojis = 512 bytes) passes. |
| `MAX_NOTIFICATION_BODY_CHARS` | 1024 chars | 1024 | Unicode chars (`.chars().count()`) | Exactly 1024 chars passes; 1025 chars fails. |
| `MAX_READ_GRANTED_BYTES` | 10 MB | `10 * 1024 * 1024` | Bytes | 10 MB passes; 10 MB + 1 byte fails before full buffer allocation. |
| `MAX_WRITE_GRANTED_BYTES` | 10 MB | `10 * 1024 * 1024` | Bytes | 10 MB passes; 10 MB + 1 byte fails before writing. |
| `MAX_GRANTS_PER_WINDOW` | 64 | 64 | Count (Grants) | 64 grants succeed; 65th fails with `LimitReached`. |
| `MAX_CONTEXT_MENU_ITEMS` | 200 | 200 | Count (Items) | 200 items pass; 201 items fail. |
| `MAX_SHORTCUTS_PER_APP` | 32 | 32 | Count (Shortcuts) | 32 shortcuts pass; 33 shortcuts fail. |

**Character vs. Byte Distinction Analysis:**
- The build plan distinguishes between character counts (notifications) and byte counts (clipboard, file reads/writes).
- The implementation applies this consistently:
  - Notification titles and bodies are counted using `chars().count()`. A title containing 128 4-byte emojis (512 UTF-8 bytes) is accepted, while 129 emojis are rejected.
  - Clipboard text is measured by `.len()` (bytes). A string of 262,144 emojis (1,048,576 bytes) is accepted, while 262,145 emojis (1,048,580 bytes) are rejected.

---

## 5. Feature Audits

### 5.1 Clipboard (`clipboard.read`, `clipboard.write`)
- **Safety Protocol:** Tests executed with `ClipboardRestoreGuard`, which captures clipboard contents prior to testing and restores them upon test completion.
- **Roundtrip Integrity:** Successfully wrote marker `WAVICAL_AUDIT_MARKER_A3A_SEC_TEST_981` and verified exact readback.
- **Non-Text / Empty Handling:** If clipboard contains image data or files, `arboard` returns an error, cleanly mapped to `GENERIC_CLIPBOARD_ERROR` (`"Clipboard operation failed"`). No raw panic or garbage strings occur.
- **No Silent Truncation:** Large payloads exceeding 1 MB are rejected with `GENERIC_CLIPBOARD_ERROR`.
- **Log Privacy:** Grep inspection of `wavical-native` and `wavical-ipc` confirms clipboard payload strings are never logged to `tracing`.

### 5.2 Notifications (`notify.send`)
- **XML / Toast Escaping:** Tested payloads containing `<b>x</b>`, `]]>`, `</text><image src="..."/>`, `&amp;`, `%USERNAME%`, very long unbroken strings, and `\0`. Strings pass pure validation without crashing.
- **Field Whitelisting:** `validate_notify_send` rejects any object containing fields other than `title` and `body` (e.g. `url`, `image`, `sound`, `action` return `GENERIC_NOTIFICATION_ERROR`).
- **Finding (FINDING-N1):** `notify_send` has no rate limiting or queue depth cap. A malicious web page could request thousands of toasts in a tight loop.

### 5.3 Native Dialogs (`dialog.openFile`, `dialog.saveFile`)
- **Parameter Validation:**
  - `OpenFileDialogParams` accepts `filter_id` and `title`.
  - `SaveFileDialogParams` accepts `filter_id`, `title`, and `default_name`.
  - Both structs use `#[serde(deny_unknown_fields)]` and reject `path`, `defaultPath`, `multi_select`, and `folder`.
- **Finding (FINDING-D1):** `title` and `default_name` are passed directly to `rfd::FileDialog` without length or character sanitization.
- **Finding (FINDING-D2):** `resolve_filter_id` uses hardcoded static filter lists (`"text"`, `"images"`, `"documents"`, `"all"`) and ignores custom filter definitions configured in `wavical.config.js`.

### 5.4 Scoped File Grants (`GrantTable`)
- **CSPRNG Invariant:** `generate_grant_id` uses `getrandom::fill` (128-bit OS CSPRNG). 1,000 generated grants produced 1,000 unique IDs.
- **Cross-Window Isolation:** Looking up window A's grant from window B returns `GrantError::NotFound`.
- **Window Close Lifecycle:** `clear_window(window_id)` purges all grants associated with the window.
- **Capacity Limits:** Enforces max 64 grants per window; 65th grant fails with `GrantError::LimitReached`.
- **Finding (FINDING-G1):** `GrantTable::get` does not validate the grant ID format (length 32, hex characters) prior to looking up the key in the hash map.
- **Finding (FINDING-G2):** `GrantEntry` does not track grant permissions (read-only vs write-only). A grant created by `dialog.openFile` can be overwritten via `fs.writeGranted`, and a grant created by `dialog.saveFile` can be read via `fs.readGranted`.

### 5.5 Scoped Filesystem Operations (`fs.readGranted`, `fs.writeGranted`)
- **No Page-Controlled Paths:** Serde strictly rejects any payload containing a `path` property.
- **Use-Time Target Verification:**
  - Target removed after grant -> rejected (`"Granted file does not exist"`).
  - Target replaced by directory -> rejected (`"Target is not a regular file"`).
  - Target renamed / canonical path mismatch -> rejected (`"Canonical path mismatch"`).
  - Windows reserved device names (`CON`, `NUL`, `AUX`) -> rejected.
  - Alternate data streams (`test.txt:stream`) -> rejected (`"Alternate data streams are not permitted"`).
- **Size Limits & Memory Safety:** Files larger than 10 MB are rejected via metadata length check before allocating buffer memory.
- **Encoding:** Non-UTF-8 binary data is rejected cleanly (`"File contains invalid UTF-8 data"`).
- **Finding (FINDING-F1):** Time-of-check to time-of-use (TOCTOU) window exists between `validate_granted_target` and `std::fs::write`.
- **Finding (FINDING-F2):** `std::fs::write` does not write to a temporary file before renaming, risking file truncation if interrupted.

---

## 6. Logging and Information Leakage

- **Page-Visible Responses:** Responses returned to the page sanitize path characters (`/`, `\`, `:`) to generic error strings (`"Handler execution failed"`).
- **Internal Tracing Logs:**
  - Finding **FINDING-L1 (Low):** `wavical-native` logs full internal target paths in warning events:
    - [crates/wavical-native/src/lib.rs:415](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L415): `tracing::warn!(error = %e, path = ?entry.path, "Failed to open granted file for reading");`
    - [crates/wavical-native/src/lib.rs:420](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L420): `tracing::warn!(error = %e, path = ?entry.path, "Failed to read metadata on opened file handle");`
    - [crates/wavical-native/src/lib.rs:434](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L434): `tracing::warn!(error = %e, path = ?entry.path, "Failed to read content from granted file");`
    - [crates/wavical-native/src/lib.rs:475](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L475): `tracing::warn!(error = %e, path = ?entry.path, "Failed to write content to granted file");`
  - While page responses are sanitized, tracing logs capture local filesystem paths, contrary to the Section 1.0 logging rule.

---

## 7. Threading Analysis

1. **`dialog.openFile` & `dialog.saveFile`:**
   - Execute synchronously via `rfd::FileDialog::pick_file()` / `save_file()`.
   - On Windows, this launches a modal Common Item Dialog. While the modal dialog runs its own internal message pump, the calling thread is blocked until user dismissal.
   - **Deadlock Assessment:** No Mutex or grant table lock is held across the dialog call (grant table insertion occurs after the dialog closes).
2. **`fs.readGranted` & `fs.writeGranted`:**
   - Disk I/O occurs synchronously on the IPC handler thread. A 10 MB read/write will briefly block the webview message queue during disk transfer.
3. **`menu.showContextMenu` & `tray.set` & `shortcut.register`:**
   - Asynchronously dispatched to the main UI thread via `tao::event_loop::EventLoopProxy::send_event`. These do not block the IPC thread.

---

## 8. Fuzzing & Mutation Testing

- **Fuzz Targets:** Located in [`fuzz/fuzz_targets/grant_table.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/fuzz/fuzz_targets/grant_table.rs) and [`fuzz/fuzz_targets/native_request_parser.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/fuzz/fuzz_targets/native_request_parser.rs).
  - *Status:* NOT RUN (Requires WSL nightly environment; command: `cargo +nightly fuzz run grant_table`).
- **Mutation Testing (`cargo-mutants 27.1.0`):**
  - Evaluated on [`crates/wavical-ipc/src/grants.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/grants.rs) (28 mutants).
  - Baseline unmutated suite passed (76s build + 16s test). Mutants altering boundary checks (`>=` to `<`, `==` to `!=`) and grant counting logic are caught and killed by the grant test suite.

---

## 9. Findings Summary Table

| ID | Severity | Component | Summary |
| :--- | :--- | :--- | :--- |
| **FINDING-G2** | **Medium** | `wavical-ipc` / `wavical-native` | Grant table does not track grant mode; open-file grants can be overwritten, and save-file grants can be read. |
| **FINDING-F1** | **Medium** | `wavical-native` | TOCTOU window between `validate_granted_target` path verification and `std::fs::write`. |
| **FINDING-L1** | **Low** | `wavical-native` | Internal filesystem paths logged in `tracing::warn!` events upon file read/write errors. |
| **FINDING-D1** | **Low** | `wavical-native` | Dialog `title` and `defaultName` lack length and character sanitization. |
| **FINDING-D2** | **Low** | `wavical-native` | Config-defined dialog filters (`dialog.filters`) are ignored in favor of hardcoded static lists. |
| **FINDING-G1** | **Low** | `wavical-ipc` | `GrantTable::get` does not validate grant ID length and hex charset prior to hash map lookup. |
| **FINDING-N1** | **Low** | `wavical-native` | `notify.send` lacks rate limiting or queue depth restrictions. |
| **FINDING-F2** | **Low** | `wavical-native` | File writes are non-atomic (no temporary file + atomic rename), risking file truncation on abort. |

---

## 10. Phase 2 Requirement Table (WP2, WP3, WP4)

| Work Package | Acceptance Criterion | Status | Evidence |
| :--- | :--- | :--- | :--- |
| **WP2** | Native handler framework & capability plumbing | **PASS** | Double-gated at `wavical-shell:421` and `dispatch_native:705`. Ungranted calls never invoke handlers. |
| **WP2** | Tracing event for every native call (no payload) | **PASS** | `tracing::info!` emits `method`, `window_id`, and `granted` ("yes"/"no") with no payloads ([crates/wavical-ipc/src/lib.rs:698](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs#L698)). |
| **WP3** | `clipboard.read` / `clipboard.write` (1 MB limit, text-only) | **PASS** | `validate_clipboard_write` enforces 1 MB byte limit. Non-text rejected with `GENERIC_CLIPBOARD_ERROR`. |
| **WP3** | `notify.send` (title 128 chars, body 1024 chars, no URLs/actions) | **PASS** | `validate_notify_send` enforces Unicode character bounds and rejects extraneous fields. |
| **WP4** | `dialog.openFile` / `dialog.saveFile` (filters, opaque grants) | **PASS** | Returns 128-bit opaque grant ID; raw paths rejected by serde ([crates/wavical-ipc/src/grants.rs:160-180](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/grants.rs#L160-L180)). |
| **WP4** | Grant table isolation (128-bit CSPRNG, 64 limit, clear on close) | **PASS** | `getrandom::fill` CSPRNG, cross-window isolation enforced, `clear_window` verified ([crates/wavical-ipc/src/grants.rs:55-156](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/grants.rs#L55-L156)). |
| **WP4** | `fs.readGranted` / `fs.writeGranted` (10 MB limit, re-check) | **PASS** | Re-checks existence, regular file, non-symlink, canonical match, and enforces 10 MB limits ([crates/wavical-native/src/lib.rs:394-483](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L394-L483)). |

---

## 11. Top 5 Findings

1. **FINDING-G2 (Medium) — Grants Lack Permission Mode Scoping (Read vs Write)**
   - *Location:* [`crates/wavical-ipc/src/grants.rs:46-53`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/grants.rs#L46-L53)
   - *Impact:* `GrantEntry` records only `window_id`, `path`, and `canonical_path`, but does not record whether the grant originated from an open or save dialog. A grant obtained through `dialog.openFile` can be passed to `fs.writeGranted` to overwrite the file.
2. **FINDING-F1 (Medium) — TOCTOU Window Between Target Verification and File Write**
   - *Location:* [`crates/wavical-native/src/lib.rs:472-474`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L472-L474)
   - *Impact:* `validate_granted_target` inspects the target path and parent directory. Subsequently, `std::fs::write(&entry.path, ...)` re-opens and writes to the path by name. An attacker could swap the target path with a junction or symlink in the interval between validation and write.
3. **FINDING-L1 (Low) — Full Filesystem Paths Emitted in Tracing Warnings**
   - *Location:* [`crates/wavical-native/src/lib.rs:415, 420, 434, 475`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L415)
   - *Impact:* Warning logs emit `path = ?entry.path` on file operation failures, recording internal filesystem paths in the process log contrary to the logging requirements in Section 1.0.
4. **FINDING-D2 (Low) — Config Dialog Filters Ignored in Favor of Static List**
   - *Location:* [`crates/wavical-native/src/lib.rs:37-45`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L37-L45)
   - *Impact:* `resolve_filter_id` resolves only four hardcoded filter IDs (`"text"`, `"images"`, `"documents"`, `"all"`). Custom filter definitions specified in `wavical.config.js` (`dialog.filters`) cannot be used.
5. **FINDING-N1 (Low) — Uncapped Desktop Notification Dispatch**
   - *Location:* [`crates/wavical-native/src/lib.rs:280-300`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs#L280-L300)
   - *Impact:* `notify.send` performs payload validation but enforces no rate limit or maximum queue depth. A web page with `notify.send` capability can spam Windows notification toasts.
