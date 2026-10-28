# Wavical — Phase 2 Build Plan: Native Tier 1 APIs

**Goal:** give apps a small set of safe native features (clipboard, file dialogs, notifications, tray, context menus, shortcuts, window state, single-instance, file associations, badges, crash recovery). Every native call is default-deny, capability-gated, validated in Rust, and tested.

**Scope rule:** each feature is an addition on top of Phase 0 (bridge, capabilities, protocol) and Phase 1 (config, CLI). No Node. No AI. No installers.

**Estimate:** 3–4 weeks part-time with AI. The research package (WP1) decides which crates are used. Several features (tray, shortcuts, badges) are uncertain, so they get their own packages.

---

## 0. Decisions locked before code

### 0.1 Where native code lives

| Crate | May depend on | Rule |
|---|---|---|
| `wavical-ipc` | serde, thiserror, tracing | Still pure. Owns the method registry, capability names and input validation. NO native crates. |
| `wavical-protocol` | (unchanged) | Still pure. |
| `wavical-native` (NEW) | tao, wry, rfd, arboard, notify-rust, and others chosen in WP1 | All OS-touching code. Handlers live here. |
| `wavical-shell` | tao, wry, wavical-native | Wires the bridge to native handlers. |

`#![forbid(unsafe_code)]` stays on the pure crates. `wavical-native` also starts with `forbid`. If a feature truly needs our own `unsafe` (for example, a raw Win32 call), the agent must **stop and ask**. It may use crates that wrap unsafe internally.

### 0.2 Capability names (supersedes examples in WAVICAL_PHASE_1_CONFIG_SCHEMA.md)

The Phase 1 schema doc listed broad names such as `fs.readFile` and `fs.writeFile`. Those would let a page read any file, so **they are replaced** by scoped names below.

| Capability | What it allows | Phase 2 WP |
|---|---|---|
| `clipboard.read` | read text from clipboard | WP3 |
| `clipboard.write` | write text to clipboard | WP3 |
| `notify.send` | show a notification | WP3 |
| `dialog.openFile` | show open dialog; returns a **grant**, not a path | WP4 |
| `dialog.saveFile` | show save dialog; returns a **grant** | WP4 |
| `fs.readGranted` | read a file via an existing grant | WP4 |
| `fs.writeGranted` | write a file via an existing grant | WP4 |
| `menu.showContextMenu` | show a context menu at the cursor | WP7 |
| `tray.set` | set tray icon, tooltip and menu | WP7 |
| `shortcut.register` | register global shortcuts | WP8 |
| `badge.set` | set taskbar overlay badge | WP8 |

**Config-only features (no runtime capability):** window state persistence, single-instance lock, file associations, crash recovery, WebView2 fallback UX. The app opts out or in through `wavical.config.js`. The page cannot call these.

**Default:** every capability is denied unless listed as `true` in config. The Phase 1 validator must accept the new names. Its list comes from `wavical-ipc`, so it is updated in one place.

### 0.3 Scoped file access (the most important rule in this phase)

- The page **never** passes a file path to `fs.*`. A path is never accepted from JavaScript.
- A file path only enters the system when the **user** picks it in a native dialog.
- Rust stores that path in a per-window grant table and returns an **opaque grant ID** (random, unguessable) to the page.
- `fs.readGranted` and `fs.writeGranted` take only a grant ID. Rust looks up the path, re-checks it (exists, is a file, is not a reserved name, canonical path still matches), and then acts.
- Grants are cleared when the window closes. Grants never move between windows.
- Reads and writes have size limits (named constants, see 0.4).

### 0.4 Named limits (constants in code, each with a test)

| Limit | Value |
|---|---|
| Clipboard text | 1 MB |
| Notification title | 128 chars |
| Notification body | 1024 chars |
| Context menu items | 200 |
| `fs.readGranted` max file size | 10 MB |
| `fs.writeGranted` max payload | 10 MB |
| Grants per window | 64 |
| Shortcuts per app | 32 |

### 0.5 Threading

Many native APIs must run on the main (tao) thread. The agent checks each crate's docs in WP1 and records the rule. Dispatch to the main thread uses tao's event loop proxy. Blocking dialogs must not freeze the webview forever (decide in WP4).

### 0.6 Platform

- Windows only for behavior. The `wavical-native` crate must build on Windows in CI.
- Linux CI keeps testing only the pure crates, as in Phase 0.
- Pure logic (validation, grant table, limits, config parsing) is tested on any OS.

### 0.7 File associations

Registered per user under `HKCU` only. No admin rights. Opt-in via config. Has a matching unregister command. Full uninstall cleanup is a Phase 5 (installer) task, and this plan records that gap.

---

## 1. Scope

**In scope**
- Research and decisions for every crate and feature (WP1)
- Native handler framework and capability plumbing (WP2)
- Clipboard, notifications (WP3)
- File dialogs and scoped file grants (WP4)
- Window state persistence, single-instance lock (WP5)
- WebView2 detection UX, renderer crash recovery (WP6)
- Tray and context menu (WP7)
- Global shortcuts, taskbar badges, file associations (WP8)
- Integration, extended test bench, self-review (WP9)

**Out of scope**
- AI runtime, Node/Bun, updater, installers, code signing (Phases 3–5)
- macOS and Linux behavior (compile only where it already does)
- Live IPC inspector, template marketplace (ecosystem list)
- Enterprise audit logging (Phase 6). Phase 2 adds a `tracing` event for each native call only, so the Phase 6 logger has a base.

---

## 2. Work packages (do in order, one per session)

### WP1: Research and decisions (NO feature code)

For each feature, the agent reads current docs.rs pages and repo examples and records, in `docs/WAVICAL_PHASE_2_DECISIONS.md`:
- crate name and the version `cargo add` picks
- the API used
- whether it must run on the main thread
- whether it needs our own `unsafe` (if yes, stop and ask)
- how tray, menu, and global shortcuts relate to `tao` (the checklist flagged this as unverified). Does `tao` ship them, or do `tray-icon`, `muda`, or `global-hotkey` need to be added?
- how window state and single-instance lock can be done with `tao` and small crates

**Done when:** the decisions file exists, each row has a source link, and the agent has stopped for review.

### WP2: Native framework and capability plumbing

- Create `wavical-native` (forbid unsafe, empty handlers).
- In `wavical-ipc`: add the capability names from 0.2 and register each native method name. Add the enumeration test from Phase 0 WP7 for the new list.
- Add a generic native-call path: capability check first, then handler, then response. Unknown or ungranted methods return the same generic error as Phase 0.
- Each native call emits a `tracing` event (method, window id, granted yes/no, no payload).
- Update the Phase 1 config validator to accept the new names. Unknown names still fail with a suggestion.

**Done when:** tests prove an ungranted native method never reaches a handler, and the config validator accepts and rejects the right names.

### WP3: Clipboard and notifications

- `clipboard.read` / `clipboard.write`: text only, 1 MB limit, rejects non-text and oversize payloads with a generic error.
- `notify.send`: title and body limits, no URLs or actions in v1.
- Both gated by capability; the hello example gets buttons for each.

**Done when:** tests cover limits and denial; manual Windows check (copy, paste, notification appears) is listed for the user.

### WP4: File dialogs and scoped grants

- `dialog.openFile` / `dialog.saveFile` via `rfd`. Filters come from config (fixed list), never from raw page input.
- Returned grant IDs follow 0.3. Implement the grant table as a pure type in `wavical-ipc` (testable on Linux) and the dialog glue in `wavical-native`.
- `fs.readGranted` / `fs.writeGranted` with size limits.
- Tests: forged grant IDs are rejected; grants from another window are rejected; grants are cleared on close; a path that becomes a symlink or reserved name after being granted is rejected; over-limit reads and writes fail.

**Done when:** all grant tests pass; manual Windows check (pick a file, read it, save a file) is listed.

### WP5: Window state and single-instance lock

- Window state (position, size, maximized) saved to the app data folder, JSON, opt-out in config. Corrupt or out-of-screen state falls back to defaults.
- Single-instance lock: a second launch passes its arguments to the first instance and exits with code 0. Lock mechanism chosen in WP1.
- Tests for the state file parser (corrupt input, missing fields, unknown fields) and lock behavior where testable.

**Done when:** parser tests pass; manual check (resize, relaunch, second launch focuses first window) is listed.

### WP6: WebView2 detection UX and renderer crash recovery

- Upgrade Phase 0 WP2: if WebView2 is missing or blocked, show a friendly native message with a link to install it. Still exit non-zero.
- If the WebView2 renderer process dies, reload the page once and log the event. If it dies again within 30 seconds, show a clear error and stop retrying.
- Tests: the retry-limit logic as a pure function.

**Done when:** retry logic is unit-tested; manual check (kill the renderer in Task Manager) is listed.

### WP7: Tray and context menu

- Tray (`tray.set`): icon, tooltip, menu items. Click events go to the page only as a generic event name, never raw Rust data.
- Context menu (`menu.showContextMenu`): menu built from a page-provided list, validated against the 200-item limit and label length. Item clicks return only the item id the page sent.
- Crate choice comes from WP1 decisions.

**Done when:** menu validation tests pass; manual check (tray icon, menu click, right-click menu) is listed.

### WP8: Global shortcuts, taskbar badges, file associations

- `shortcut.register`: shortcut strings validated against a fixed grammar. Conflicts return a generic error and are logged. Max 32 per app.
- `badge.set`: taskbar overlay. Count range 0–99, shows "99+" above that. Uses the approach chosen in WP1 (no unsafe written by us).
- File associations: opt-in config entry; `wavical register-filetypes` and `wavical unregister-filetypes` commands write `HKCU` only; tests check the key names are built from validated extensions only (no injection of registry paths).

**Done when:** validation and naming tests pass; manual checks (shortcut fires, badge shows, file type opens app) are listed.

### WP9: Integration, extended test bench, self-review

- Extend the Phase 0 attack page with: forged grant IDs, native calls without capability, oversized clipboard and notification payloads, shortcut injection strings, registry-path injection attempts.
- Extend fuzz targets: the native request parser and the grant table.
- Update the example app to demonstrate each feature behind a config flag.
- Self-review as a hostile reviewer for: grant reuse, TOCTOU between grant check and file access, label/size bypass, and anything that leaks paths or handles to the page. Each finding gets severity, exploit, fix and a test.
- Prove the tests are real: disable the capability check, grant check and size limit one at a time; show the matching test fails.
- CI: Windows job builds `wavical-native`. Linux job runs the pure-crate tests and the grant table tests.

**Done when:** everything passes, the review list is written, and the three tests-fail proofs are shown.

---

## 3. Manual Windows checks (the agent cannot test GUI in CI)

Each WP lists its manual checks. The user runs this checklist on a real Windows machine before Phase 2 is called done:

### Consolidated Phase 2 Manual Verification Checklist:
- [ ] **WP2 (File Dialogs & Grants):** Open file dialog, pick a file, read content. Cancel dialog. Save file. Expected: file read/write roundtrip works cleanly; cancelled dialog returns `{ cancelled: true }`.
- [ ] **WP3 (Clipboard & Notifications):** Read/write text to Windows clipboard; fire desktop toast notification. Expected: text copied/pasted correctly; toast appears in Windows notification center.
- [ ] **WP5 (Window State & Single-Instance):** Move/resize window, close, reopen. Launch second instance with `wavical dev examples/hello`. Expected: window position/size restored; second instance focuses existing window and exits 0.
- [ ] **WP6 (Missing Runtime & Renderer Crash):** Test `WAVICAL_SIMULATE_NO_WEBVIEW2=1`. Crash webview. Expected: friendly native message box on missing runtime; single crash reloads page, rapid repeat crashes stop retrying cleanly.
- [ ] **WP7 (Tray Icon & Context Menu):** Click "Setup Tray Icon"; right-click window for context menu. Expected: icon appears in notification tray, right-click opens native Win32 popup menu at cursor; clicks emit generic item events.
- [ ] **WP8 (Global Shortcuts):** Register `Ctrl+Shift+K` in config or via page. Press `Ctrl+Shift+K` while another app has focus. Expected: hello page receives `wavical:shortcut` event with `{ shortcut: "Ctrl+Shift+K" }`.
- [ ] **WP8 (Conflicting Shortcut):** Register a shortcut another app/system uses. Expected: generic error returned, app continues running without panic.
- [ ] **WP8 (Taskbar Overlay Badge):** **DEFERRED** (Open Exception). Call `badge.set(5)` and `badge.set(150)`. Expected: error returned to page ("Taskbar overlay badge is not supported on this platform"), tracing warning logged, nothing crashes.
- [ ] **WP8 (File Associations Registration):** Run `cargo run -p wavical-cli --bin wavical -- register-filetypes examples/hello`. Double-click a file with `.wavhello`. Expected: opens in Wavical shell.
- [ ] **WP8 (File Associations Unregistration):** Run `cargo run -p wavical-cli --bin wavical -- unregister-filetypes examples/hello`. Expected: association keys removed from `HKCU\Software\Classes`.

---

## 4. Not needed in Phase 2

- Node/Bun, AI models, updater, installers, code signing
- macOS and Linux native features
- Audit logger (only the event hooks are added now)
- Template marketplace, plugin registry, WebDriver tests
