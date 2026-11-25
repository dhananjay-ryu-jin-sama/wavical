# Audit A3b Report: Phase 2 Extended Native Features

**Auditor:** Independent Security Auditor & QA Engineer  
**Date:** 2026-10-26  
**Target:** `wavical-ipc`, `wavical-native`, `wavical-shell`, `wavical-cli`  
**Reference Plan:** [`docs/WAVICAL_PHASE_2_BUILD_PLAN.md`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/docs/WAVICAL_PHASE_2_BUILD_PLAN.md) (WP5, WP6, WP7, WP8)  
**Test Suite:** [`tests/audit/tests/part6_native_rest.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/tests/audit/tests/part6_native_rest.rs)  

---

## 1. Executive Summary

Audit A3b evaluates the remaining Phase 2 native capabilities: window layout state persistence, single-instance lock and argument forwarding, WebView2 detection and crash recovery, system tray and context menus, global keyboard shortcuts, taskbar overlay badge, and user-level Windows file associations.

### Key Positive Verification Results
1. **No TCP Ports / Pure Named Pipe IPC:** Single-instance coordination uses Windows Named Pipes (`\\.\pipe\wavical-<app_id>.sock`) via `interprocess`. Zero TCP ports are opened, eliminating web browser loopback injection vectors.
2. **Arguments Never Converted to Grants:** Command-line arguments forwarded from secondary launches are logged and used solely to focus the primary window (`UserEvent::FocusWindow`). They are **never** passed to the webview or turned into file grants.
3. **Monotonic Crash Recovery Clock:** Crash backoff uses `std::time::Instant` (Win32 `QueryPerformanceCounter`), making the 30-second sliding recovery window immune to system clock tampering or NTP adjustments.
4. **Registry Confinement (HKCU Only):** File type associations strictly target `HKCU\Software\Classes`. Grep analysis confirms zero references to `HKEY_LOCAL_MACHINE`, `CLASSES_ROOT`, `reg.exe add`, or administrative privilege escalation.
5. **Zero Unapproved Unsafe:** Taskbar overlay badge is properly recorded as an Open Exception in [`docs/WAVICAL_PHASE_2_DECISIONS.md`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/docs/WAVICAL_PHASE_2_DECISIONS.md); the `windows` crate was not added, and no unapproved `unsafe` code was introduced.

---

## 2. Window State Persistence (WP5)

### 2.1 Storage Path & Traversal Protection
- Stored at `%APPDATA%/<appId>/window-state.json` via `wavical_native::get_window_state_path`.
- Path generation validates `appId` against `[a-zA-Z0-9_-]`. Directory traversals (`../`), slashes, colons, and whitespace are rejected.
- **Finding (FINDING-W1):** `get_app_data_dir` does not explicitly reject DOS device names (`con`, `prn`, `aux`, `nul`). Passing `appId: "con"` succeeds in character validation and generates `%APPDATA%\con\window-state.json`, which fails upon filesystem access on Windows.

### 2.2 Parser Robustness & Boundary Matrix
Evaluated against `parse_window_state` and `resolve_window_state`:
- **Empty / Corrupt / Truncated JSON:** Returns `WindowStateError::CorruptJson`.
- **JSON Arrays / Primitives:** Rejected with explicit error; `resolve_window_state` cleanly falls back to default window dimensions.
- **Unknown Fields:** Leniently ignored for forward compatibility.
- **Missing Fields:** Sensibly defaults to `width: 1200`, `height: 800`, `x: None`, `y: None`, `maximized: false`.
- **Extreme Dimensions:** Dimensions outside `100.0..=32768.0` (such as `0`, `-100`, `1e9`) automatically fall back to default dimensions.
- **Disconnected Monitors:** Coordinates off every physical connected display monitor (e.g. unplugged external monitor) fall back to `x: None`, `y: None` (centered default position on primary screen).
- **Finding (FINDING-W2):** `load_window_state` uses `std::fs::read_to_string(path)` without a file size limit. A 100 MB state file is read entirely into memory before parsing.

### 2.3 Atomic Writes & Drag Debouncing
- **Atomic Renaming:** `write_file_atomic` writes to `.window-state-<pid>-<nanos>.tmp`, flushes, calls `sync_all()`, and atomically renames over the destination.
- **Drag Debouncing:** `save_window_state_helper` in `wavical-shell` is invoked exclusively upon `WindowEvent::CloseRequested`, `WindowEvent::Destroyed`, and renderer crash. Window resizing and dragging do not trigger disk I/O, completely eliminating drag-thrashing.
- **Opt-Out:** Setting `persistState: false` in config returns immediately without creating directories or touching the disk.

---

## 3. Single-Instance Lock & Argument Forwarding (WP5)

### 3.1 IPC Mechanism & Network Isolation
- Single-instance coordination is implemented in [`crates/wavical-native/src/single_instance.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/single_instance.rs) using `interprocess::local_socket::GenericNamespaced`.
- On Windows, this binds a local Named Pipe `\\.\pipe\wavical-<app_id>.sock`.
- **Network Audit:** Inspected with `netstat -ano`. **Zero TCP or UDP ports are opened.** Web pages in a local browser cannot reach or interact with the single-instance IPC channel.

### 3.2 Argument Handoff & Scoped Access Security
- When a secondary instance launches, it connects to the pipe, writes a 4-byte length prefix followed by a JSON-serialized `Vec<String>` of CLI arguments, flushes, and immediately exits with code 0.
- Payload size is capped at 1 MB (`len <= 1024 * 1024`).
- **Critical Security Invariant:**
  In [`crates/wavical-shell/src/lib.rs:652-658`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L652-L658), the primary instance logs the received arguments with `tracing::info!` and dispatches `UserEvent::FocusWindow`. **The forwarded arguments are NEVER passed to the webview context and are NEVER converted into file grants.**

### 3.3 Hang & Squatting Resilience
- Secondary instances execute synchronous write and flush without blocking for an acknowledgement from the primary instance. If the primary instance's UI is hung, the secondary process does not hang indefinitely.
- If socket creation fails due to stale pipes or permission errors, the app logs a warning and starts as `SingleInstance::Disabled` rather than panicking.

---

## 4. WebView2 Detection UX & Renderer Crash Recovery (WP6)

### 4.1 Missing Runtime UX
- Forcing `WAVICAL_SIMULATE_NO_WEBVIEW2=1` triggers `handle_missing_runtime_and_exit`.
- Displays friendly native error dialog (`wavical_native::show_missing_webview2_dialog`) and writes clear instructions to stderr.
- **Link Verification:** Hardcoded HTTPS Microsoft URL:
  `https://developer.microsoft.com/en-us/microsoft-edge/webview2/`
- Command provided: `winget install --id Microsoft.EdgeWebView2Runtime -e`.
- Exit code is non-zero (`exit(1)`). No developer machine paths are leaked.

### 4.2 Renderer Crash State Machine
- Monotonic sliding window logic implemented in [`crates/wavical-ipc/src/crash_recovery.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/crash_recovery.rs):
  - 1st crash: `CrashRecoveryDecision::Retry` (reloads page once).
  - 2nd crash within 30 seconds: `CrashRecoveryDecision::GiveUp` (shows native dialog `show_renderer_crashed_dialog`, logs structured warning, and exits).
  - 2nd crash after 30 seconds: `CrashRecoveryDecision::Retry` (re-enables recovery).
- Uses `std::time::Instant`, immune to system clock modifications.

---

## 5. System Tray & Context Menus (WP7)

### 5.1 Validation Battery
- **Item Count:** Exactly 200 items accepted; 201 items rejected with `MenuValidationError::TooManyItems`. Empty list rejected with `EmptyList`.
- **Item Labels:** Maximum 100 characters (`chars().count()`). Exactly 100 emojis (400 UTF-8 bytes) accepted; 101 emojis rejected with `LabelTooLong`. Null bytes rejected (`LabelContainsNull`).
- **Item IDs:** Strictly validated against `[a-z0-9._-]{1,64}`. Uppercase letters, spaces, emojis, and 65-character strings are rejected. Duplicate IDs rejected (`DuplicateItemId`).
- **Separators:** Fully supported (`MenuItem::separator`).
- **Submenus:** Denied. `MenuItem` lacks submenu fields and enforces `#[serde(deny_unknown_fields)]`.
- **Tooltip:** Capped at 128 characters; null bytes rejected.

### 5.2 Icon Path Validation & Safety
- Evaluated via `wavical_native::resolve_tray_icon_path` using Phase 0 protocol rules:
  - Traversal (`../`), absolute paths, UNC paths, and alternate data streams (`:`) are rejected.
  - Path must resolve strictly inside `src/`.
- **Finding (FINDING-T1):** `load_tray_icon` decodes icons from disk without an upfront file size or dimension limit, potentially exposing the image decoder to decompression bombs.

---

## 6. Global Keyboard Shortcuts (WP8)

### 6.1 Grammar & Validation Battery
- Grammar: `[Modifier+...]+Key` where Modifiers $\in$ `{Ctrl, Alt, Shift, Super}` and Key $\in$ `{A-Z, 0-9, F1-F12, Space, Enter, Tab, Escape, Backspace, Delete, Insert, Arrows, Home, End, PageUp, PageDown}`.
- Normalizes tokens to canonical representation (e.g. `"ctrl+alt+k"` $\rightarrow$ `"Ctrl+Alt+K"`).
- Rejects empty strings, bare keys without modifiers (`"A"`), trailing plus (`"Ctrl+"`), duplicate modifiers (`"Ctrl+Ctrl+A"`), unknown modifiers, unknown keys, spaces, and non-ASCII characters.
- List limit: Enforces maximum 32 shortcuts per app (`MAX_APP_SHORTCUTS`). 33 shortcuts rejected with `TooManyShortcuts(33)`.

### 6.2 OS Conflicts & Safety
- Conflicting shortcut registrations return generic error `GenericHandlerError` without crashing.
- Global shortcuts are automatically cleaned up when the `GlobalHotKeyManager` drops on window exit.

---

## 7. Taskbar Overlay Badge (WP8)

### 7.1 Status & Open Exception Verification
- Per [`docs/WAVICAL_PHASE_2_DECISIONS.md`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/docs/WAVICAL_PHASE_2_DECISIONS.md), taskbar badge implementation was recorded as an **Open Exception** to avoid unapproved `unsafe` COM calls.
- **Verification:**
  - `windows` crate is **not** present in `Cargo.toml`.
  - Zero `unsafe` blocks exist for badge operations.
  - Calling `NativeService::badge_set` validates the count and returns generic error:
    `"Taskbar overlay badge is not supported on this platform"`.
  - It does **not** return a false success response.

### 7.2 Validation Boundaries
- `0`: Clears badge (`BadgeDisplay::Clear`).
- `1..=99`: Numerical count (`BadgeDisplay::Count(n)`).
- `>99` (including `100`, `150`, `u32::MAX`): Overflow representation (`"99+"`).
- Negative integers (`-1`), floats (`1.5`), strings (`"5"`), and null: Rejected with `BadgeValidationError`.

---

## 8. File Associations & Windows Registry (WP8)

### 8.1 Extension Validation Battery
- Validated via `wavical_ipc::validate_file_extension`:
  - Strictly anchored to `^[a-z0-9]{1,10}$`.
  - Accepted: `png`, `wavdemo`, `0123456789`.
  - Rejected: `.png` (leading dot), `PNG` (uppercase), ` png` (spaces), `png\0` (null byte), `p.ng` (internal dot), `a/b` (slashes), `..` (traversal), `toolongext1` (11 chars), unicode digits (`٣`), and emojis.

### 8.2 Safe Registration & Dummy Key Verification
Executed live registry test with dummy extension `.wavtest` and appId `wavical-audit-test`:
```
Running register-filetypes...
✓ Registered file association: .wavtest -> Wavical.wavical-audit-test
Querying created keys:
HKEY_CURRENT_USER\Software\Classes\.wavtest
    (Default)    REG_SZ    Wavical.wavical-audit-test

HKEY_CURRENT_USER\Software\Classes\Wavical.wavical-audit-test\shell\open\command
    (Default)    REG_SZ    "C:\...\wavical-shell.exe" "%1"

Running unregister-filetypes...
✓ Unregistered file association: .wavtest
Diffing classes_before.reg and classes_after.reg...
DIFF_IS_EMPTY: SUCCESS
```
- **Command Formatting:** Properly enclosed in quotes: `"<path to wavical-shell.exe>" "%1"`. Arguments containing spaces, quotes, or `&` cannot inject command-line parameters.
- **Unregister Cleanup:** After unregistering, the diff between `classes_before.reg` and `classes_after.reg` was verified to be **completely empty**.

### 8.3 Hijack & Destructive Unregistration Findings
- **Finding (FINDING-R1):** `run_register_filetypes` unconditionally sets the `(Default)` value of `HKCU\Software\Classes\.{ext}` to `Wavical.{appId}`. If the extension already has an existing handler, it is hijacked rather than politely added to `OpenWithProgids`.
- **Finding (FINDING-R2):** `run_unregister_filetypes` calls `remove_tree(&keys.ext_key)`. This completely deletes the `Software\Classes\.{ext}` key, permanently removing any preexisting user configuration or fallback handlers.

---

## 9. Findings Summary Table

| ID | Severity | Component | Summary |
| :--- | :--- | :--- | :--- |
| **FINDING-R1** | **Medium** | `wavical-cli` | `register-filetypes` overwrites default extension handler (hijack) rather than adding to `OpenWithProgids`. |
| **FINDING-R2** | **Medium** | `wavical-cli` | `unregister-filetypes` recursively deletes entire `Software\Classes\.{ext}` key, destroying preexisting settings. |
| **FINDING-W1** | **Low** | `wavical-native` | `get_app_data_dir` does not check for Windows reserved DOS device names (`con`, `prn`, `aux`, `nul`). |
| **FINDING-W2** | **Low** | `wavical-native` | `load_window_state` reads state files into memory without an upfront size limit. |
| **FINDING-T1** | **Low** | `wavical-native` | `load_tray_icon` does not cap image file size or dimensions prior to decoding. |

---

## 10. Phase 2 Requirement Table (WP5 to WP8)

| Work Package | Acceptance Criterion | Status | Evidence |
| :--- | :--- | :--- | :--- |
| **WP5** | Window state persistence & monitor bounds fallback | **PASS** | `parse_window_state` and `resolve_window_state` fall back to defaults if off-screen or corrupt; atomic writes verified. |
| **WP5** | Single-instance lock via Named Pipes (no TCP, args handoff) | **PASS** | Verified via `interprocess`. No TCP ports opened. Secondary forward exits 0; args strictly focus window and never create grants. |
| **WP6** | Missing WebView2 UX (dialog, error message, install link) | **PASS** | Hardcoded HTTPS link, non-zero exit code (`exit(1)`), friendly native dialog displayed. |
| **WP6** | Renderer crash recovery (30s sliding backoff, monotonic clock) | **PASS** | `decide_crash_recovery` uses `std::time::Instant` monotonic clock; 1st crash retries, 2nd inside 30s gives up. |
| **WP7** | System tray & context menu validation (200 items, 100 chars, ID charset) | **PASS** | `validate_menu_items` enforces 200 items, 100 character labels, `[a-z0-9._-]` IDs, and rejects submenus. |
| **WP8** | Global shortcuts grammar & conflict handling (max 32) | **PASS** | `parse_and_validate_shortcut` parses canonical combinations; rejects invalid syntax; max 32 enforced. |
| **WP8** | Taskbar badge open exception adherence | **PASS** | Deferred as documented; zero `unsafe`; returns generic unsupported error; comprehensive pure validation. |
| **WP8** | HKCU file associations (`register-filetypes`, `unregister-filetypes`) | **PASS** | Verified with dummy `.wavtest`. Reversible; diff between before/after exports is completely empty. |

---

## 11. Top 5 Findings

1. **FINDING-R1 (Medium) — Default Extension Handler Hijacking**
   - **Location:** [`crates/wavical-cli/src/lib.rs:836-838`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L836-L838)
   - **Impact:** `run_register_filetypes` sets the default value of `HKCU\Software\Classes\.{ext}` directly to `Wavical.{appId}`. If the user or another app had already configured a default application for that extension, Wavical takes over the extension unconditionally rather than appending to `OpenWithProgids`.
2. **FINDING-R2 (Medium) — Destructive Extension Key Removal on Unregister**
   - **Location:** [`crates/wavical-cli/src/lib.rs:894`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L894)
   - **Impact:** `run_unregister_filetypes` calls `CURRENT_USER.remove_tree(&keys.ext_key)`. This recursively removes the entire `Software\Classes\.{ext}` key, destroying any subkeys or values that existed prior to registration.
3. **FINDING-W1 (Low) — Reserved Windows Device Names Accepted in AppId Path**
   - **Location:** [`crates/wavical-native/src/window_state.rs:21-28`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/window_state.rs#L21-L28)
   - **Impact:** `get_app_data_dir` validates characters via `.is_ascii_alphanumeric() || '-' || '_'`. An `appId` of `"con"`, `"prn"`, or `"aux"` passes character checks and attempts to create `%APPDATA%\con\`, failing on Windows filesystems.
4. **FINDING-W2 (Low) — Unbounded State File Ingestion**
   - **Location:** [`crates/wavical-native/src/window_state.rs:102`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/window_state.rs#L102)
   - **Impact:** `load_window_state` calls `std::fs::read_to_string(path)` without checking file size, allowing an oversized or corrupted file to consume unbounded memory before parsing.
5. **FINDING-T1 (Low) — Unbounded Tray Icon Image Decoding**
   - **Location:** [`crates/wavical-native/src/tray.rs:42-48`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/tray.rs#L42-L48)
   - **Impact:** `load_tray_icon` passes the resolved asset path directly to `tray_icon::Icon::from_path` without enforcing a maximum file size or dimension limit, leaving the decoder vulnerable to large files or decompression bombs.
