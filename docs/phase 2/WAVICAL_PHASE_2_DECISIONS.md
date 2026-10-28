# Wavical Phase 2 — Architectural & Crate Decisions (WP1)

This document records the research, evaluation, and decisions for all native Tier 1 APIs planned for Wavical Phase 2. All versions were obtained from `cargo search` and current `docs.rs` documentation. No feature code has been written and no dependencies have been added to `Cargo.toml`.

---

## 1. Summary Matrix

| Feature / Domain | Crate | Version | API Used | Main Thread Required? | Our Own `unsafe` Needed? | Source URL |
|---|---|---|---|---|---|---|
| **File Dialogs** | `rfd` | `0.17.2` | `rfd::FileDialog::new().pick_file()`, `save_file()`, `add_filter()` | No (can run on background worker thread to avoid blocking GUI event loop) | **No** (pure safe Rust API) | [docs.rs/rfd/0.17.2](https://docs.rs/rfd/0.17.2/rfd/struct.FileDialog.html) |
| **Clipboard** | `arboard` | `3.7.0` | `arboard::Clipboard::new()`, `.get_text()`, `.set_text()` | No (can run on any thread on Windows) | **No** (pure safe Rust API) | [docs.rs/arboard/3.7.0](https://docs.rs/arboard/3.7.0/arboard/struct.Clipboard.html) |
| **Notifications** | `notify-rust` | `4.18.2` | `notify_rust::Notification::new().appname().summary().body().show()` | No (can run on any thread on Windows) | **No** (pure safe Rust API) | [docs.rs/notify-rust/4.18.2](https://docs.rs/notify-rust/4.18.2/notify_rust/struct.Notification.html) |
| **System Tray** | `tray-icon` | `0.26.1` | `tray_icon::TrayIconBuilder::new().with_menu().with_tooltip().build()`, `TrayIconEvent::set_event_handler` | Yes (creation and event dispatch must live with tao event loop) | **No** (tray creation and event forwarding are safe) | [docs.rs/tray-icon/0.26.1](https://docs.rs/tray-icon/0.26.1/tray_icon/struct.TrayIconBuilder.html) |
| **Context Menu** | `muda` | `0.21.2` | `muda::Menu::new()`, `muda::MenuItem::new()`, `muda::ContextMenu::show_context_menu_for_hwnd` | Yes (must run on window thread) | **YES** (`show_context_menu_for_hwnd` is marked `unsafe fn`) | [docs.rs/muda/0.21.2](https://docs.rs/muda/0.21.2/muda/trait.ContextMenu.html#tymethod.show_context_menu_for_hwnd) |
| **Global Shortcuts** | `global-hotkey` | `0.9.0` | `global_hotkey::GlobalHotKeyManager::new()`, `.register(hotkey)`, `GlobalHotKeyEvent::receiver()` | Yes (Windows requires manager on thread with active Win32 message loop) | **No** (pure safe Rust API) | [docs.rs/global-hotkey/0.9.0](https://docs.rs/global-hotkey/0.9.0/global_hotkey/struct.GlobalHotKeyManager.html) |
| **Window State** | `tao` | `0.37.1` (existing) | `window.outer_position()`, `inner_size()`, `is_maximized()`; `WindowEvent::Moved`, `Resized` | Yes (calls on `tao::window::Window`) | **No** (built-in tao methods) | [docs.rs/tao/0.37.1](https://docs.rs/tao/0.37.1/tao/window/struct.Window.html) |
| **Single-Instance Lock & Handoff** | `interprocess` | `2.4.6` | `interprocess::local_socket::ListenerOptions::new().create_sync()`, `Stream::connect()` | No (can be evaluated at app startup before event loop starts) | **No** (pure safe Rust cross-platform IPC) | [docs.rs/interprocess/2.4.6](https://docs.rs/interprocess/2.4.6/interprocess/local_socket/index.html) |
| **Taskbar Overlay Badge** | **NONE** (safe wrapper) | N/A | Windows COM `ITaskbarList3::SetOverlayIcon` | Yes (tied to window `HWND`) | **YES — CRITICAL: No safe wrapper exists on crates.io without our own `unsafe`** | [MSDN ITaskbarList3](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-itaskbarlist3) |
| **WebView2 Detection** | `wry` | `0.57.0` (existing) | `wry::webview_version() -> Result<String, wry::Error>` | No (can be called prior to window creation) | **No** (safe public function in `wry`) | [docs.rs/wry/0.57.0](https://docs.rs/wry/0.57.0/wry/fn.webview_version.html) |
| **WebView2 Renderer Crash Recovery** | `wry` / `webview2-com` | `0.57.0` / `0.39.0` | Underlying COM `ICoreWebView2::add_ProcessFailed` via `wry::WebViewExtWindows::webview()` | Yes (event dispatch on main UI thread) | **YES** (`wry` exposes no safe high-level event; hooking COM requires `unsafe`) | [docs.rs/wry/0.57.0 (WebViewExtWindows)](https://docs.rs/wry/0.57.0/wry/trait.WebViewExtWindows.html) |
| **CSPRNG / Entropy** | `getrandom` | `0.3.4` | `getrandom::fill()` for CSP nonces and grant IDs | No | **No** (pure safe Rust API) | [docs.rs/getrandom/0.3.4](https://docs.rs/getrandom/0.3.4) |
| **Signal Handling** | `ctrlc` | `3.4.7` | `ctrlc::set_handler()` for graceful CLI dev cleanup | No | **No** (pure safe Rust API) | [docs.rs/ctrlc/3.4.7](https://docs.rs/ctrlc/3.4.7) |
| **User Registry** | `windows-registry` | `0.6.0` | `windows_registry::CURRENT_USER` for file associations | No | **No** (pure safe Rust API) | [docs.rs/windows-registry/0.6.0](https://docs.rs/windows-registry/0.6.0) |

---

## 2. Detailed Findings & Architectural Analysis

### 2.1 File Dialogs: `rfd` (v0.17.2)
- **Crate:** `rfd = "0.17.2"`
- **API:**
  - `rfd::FileDialog::new()`
  - `.set_title("Select File")`
  - `.add_filter("Text Files", &["txt", "md"])` (fixed filters configured in app config, never page input)
  - `.pick_file() -> Option<PathBuf>` (open dialog)
  - `.save_file() -> Option<PathBuf>` (save dialog)
  - `.set_parent(&window)` (anchors dialog to tao window via `raw_window_handle`)
- **Main Thread Requirement:** On Windows, `IFileDialog` calls COM. Calling `.pick_file()` synchronously on the main tao thread blocks the thread and freezes webview repaints during dialog interaction. Running the dialog invocation on a dedicated worker thread (`std::thread::spawn`) prevents UI freeze while remaining fully thread-safe on Windows (`COINIT_APARTMENTTHREADED`).
- **Safety:** Pure safe Rust. No `unsafe` required from our application.
- **Source:** https://docs.rs/rfd/0.17.2/rfd/struct.FileDialog.html

### 2.2 Clipboard: `arboard` (v3.7.0)
- **Crate:** `arboard = "3.7.0"`
- **API:**
  - `arboard::Clipboard::new() -> Result<Clipboard, arboard::Error>`
  - `clipboard.get_text() -> Result<String, arboard::Error>`
  - `clipboard.set_text(text: impl Into<String>) -> Result<(), arboard::Error>`
- **Main Thread Requirement:** On Windows, Win32 clipboard functions (`OpenClipboard`, `GetClipboardData`, `CloseClipboard`) can be called from any thread that initializes access. It does not need to be on the tao event loop thread.
- **Safety:** Pure safe Rust. No `unsafe` required from our application.
- **Limits:** Clipboard text bounded by the 1 MB named constant (`MAX_CLIPBOARD_TEXT_BYTES = 1024 * 1024`).
- **Source:** https://docs.rs/arboard/3.7.0/arboard/struct.Clipboard.html

### 2.3 Notifications: `notify-rust` (v4.18.2)
- **Crate:** `notify-rust = "4.18.2"`
- **API:**
  - `notify_rust::Notification::new()`
  - `.appname(&str)`
  - `.summary(&str)` (bounded by 128 chars named constant)
  - `.body(&str)` (bounded by 1024 chars named constant)
  - `.show() -> Result<NotificationHandle, notify_rust::error::Error>`
- **Main Thread Requirement:** On Windows, toasts are issued via WinRT/PowerShell fallback. Can be executed on worker threads or main thread without blocking the event loop.
- **Safety:** Pure safe Rust. No `unsafe` required from our application.
- **Source:** https://docs.rs/notify-rust/4.18.2/notify_rust/struct.Notification.html

### 2.4 System Tray: `tray-icon` (v0.26.1) vs `tao` vs `muda`
- **Does `tao` ship tray support?**
  - **No.** System tray support was decoupled and removed from `tao` in upstream versions (around tao 0.22+) to modularize window management.
- **Comparison & Selection:**
  - `tray-icon` (v0.26.1) is the official Tauri-ecosystem crate designed to replace tao's old internal tray.
  - `muda` (v0.21.2) provides menu structures used by `tray-icon` (`tray-icon` takes `Box<dyn ContextMenu>`, which `muda::Menu` implements).
- **Connecting to the `tao` Event Loop:**
  - `tray-icon` uses an event channel. In `tao`, we create an `EventLoopBuilder::<UserEvent>::with_user_event().build()`.
  - We call `tray_icon::TrayIconEvent::set_event_handler(Some(move |event| { let _ = proxy.send_event(UserEvent::Tray(event)); }))`.
  - When the user clicks the tray icon, `tao`'s event loop wakes up and receives `Event::UserEvent(UserEvent::Tray(event))`.
- **Safety:** `TrayIconBuilder` and `TrayIconEvent` have safe Rust APIs. No `unsafe` needed for system tray creation or event forwarding.
- **Source:** https://docs.rs/tray-icon/0.26.1/tray_icon/struct.TrayIconBuilder.html

### 2.5 Context Menu: `muda` (v0.21.2) vs `tao`
- **Does `tao` ship menu support?**
  - **No.** `tao 0.37.1` contains no `tao::menu` module. All menu functionality in Tauri/tao applications has migrated to `muda`.
- **API:**
  - Menu structure created with `muda::Menu::new()`, `muda::MenuItem::new(label, enabled, None)`.
  - Displaying context menu: `menu.show_context_menu_for_hwnd(hwnd: isize, position: Option<Position>)`.
- **Main Thread Requirement:** Must run on the thread that owns the Win32 window (`HWND`).
- **Safety Issue:**
  - `muda::ContextMenu::show_context_menu_for_hwnd` is explicitly declared as:
    ```rust
    unsafe fn show_context_menu_for_hwnd(&self, hwnd: isize, position: Option<Position>)
    ```
  - It is marked `unsafe` in `muda` because it operates directly on a raw HWND integer handle.
  - Therefore, using `muda`'s window context menu on Windows **requires an `unsafe` block** unless another abstraction is chosen.
- **Source:** https://docs.rs/muda/0.21.2/muda/trait.ContextMenu.html#tymethod.show_context_menu_for_hwnd

### 2.6 Global Shortcuts: `global-hotkey` (v0.9.0) vs `tao`
- **Does `tao` provide global shortcuts?**
  - **No.** Removed from `tao` in favor of the standalone `global-hotkey` crate.
- **Crate & Version:** `global-hotkey = "0.9.0"`
- **API:**
  - `let manager = GlobalHotKeyManager::new()?;`
  - `let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyK);`
  - `manager.register(hotkey)?;`
- **Event Loop & Thread Requirement:**
  - On Windows, `GlobalHotKeyManager::new()` must be created on the thread that runs the Win32 message loop (the `tao` event loop thread).
  - Events are handled via `GlobalHotKeyEvent::receiver()` or `GlobalHotKeyEvent::set_event_handler(Some(move |event| { proxy.send_event(UserEvent::HotKey(event)); }))` to forward directly into `tao`'s event loop.
- **Safety:** Pure safe Rust API. No `unsafe` required from our application.
- **Source:** https://docs.rs/global-hotkey/0.9.0/global_hotkey/struct.GlobalHotKeyManager.html

### 2.7 Window State Persistence in `tao` (v0.37.1)
- **APIs on `tao::window::Window`:**
  - Position: `window.outer_position() -> Result<PhysicalPosition<i32>, NotSupportedError>`
  - Size: `window.inner_size() -> PhysicalSize<u32>`
  - Maximized: `window.is_maximized() -> bool`
  - Restoring: `window.set_outer_position(pos)`, `window.set_inner_size(size)`, `window.set_maximized(bool)`
- **Event Loop Change Signals:**
  - Position change: `tao::event::WindowEvent::Moved(PhysicalPosition<i32>)`
  - Size change: `tao::event::WindowEvent::Resized(PhysicalSize<u32>)`
  - Maximized change: checked via `window.is_maximized()` whenever `WindowEvent::Resized` fires or upon window exit/close.
- **Main Thread Requirement:** All `Window` methods must run on the main tao thread.
- **Safety:** Pure safe Rust.
- **Source:** https://docs.rs/tao/0.37.1/tao/window/struct.Window.html

### 2.8 Single-Instance Lock & Argument Handoff: `interprocess` (v2.4.6)
- **Requirement:** A second launch passes its command-line arguments to the first running instance and exits with code 0.
- **Comparison:**
  - `single-instance = "0.3.3"` creates a Win32 Mutex, but **cannot** pass arguments between processes.
  - `interprocess = "2.4.6"` provides cross-platform local IPC (`LocalSocketListener` / `LocalSocketStream`), backed by Windows Named Pipes (`\\.\pipe\...`) and Unix domain sockets.
- **API & Flow:**
  1. On launch, attempt to bind `LocalSocketListener::bind(pipe_name)`.
  2. If binding succeeds: this process is the **primary instance**. Spawn a listener thread to receive forwarded CLI arguments from subsequent launches and signal the main window (`window.set_focus()`).
  3. If binding fails with `AddrInUse` (or `LocalSocketStream::connect(pipe_name)` succeeds): this process is a **secondary instance**. Write the CLI arguments (JSON-encoded) to the stream, flush, and exit with code 0.
- **Safety:** `interprocess` wraps Windows Named Pipes internally with 100% safe Rust interfaces. No `unsafe` needed in our codebase.
- **Source:** https://docs.rs/interprocess/2.4.6/interprocess/local_socket/index.html

### 2.9 Auxiliary Dependencies & Toolchain Decisions (F-A0-04)
- **`getrandom` (v0.3.4 in `wavical-ipc`):** Cryptographically secure CSP nonce generation and 128-bit high-entropy grant ID generation for opaque file access tokens. Pure safe Rust.
- **`ctrlc` (v3.4.7 in `wavical-cli`):** Cross-platform SIGINT/Ctrl+C handler ensuring child processes (development server and child webview shell) are cleanly terminated upon exit.
- **`windows-registry` (v0.6.0 in `wavical-native`):** Pure safe Windows registry manipulation scoped strictly to user hive (`HKEY_CURRENT_USER\Software\Classes`) for file association registration and unregistration.
- **`muda` (v0.21.2) & `tray-icon` (v0.26.1 in `wavical-shell`):** Direct dependencies in the application shell for event-loop integration, linking tray icon clicks and context menus with the tao event loop.

---

## 3. Critical Findings Requiring Escalation

### 3.1 Taskbar Overlay Badge (`ITaskbarList3::SetOverlayIcon`) — OPEN EXCEPTION
- **Question:** Which crate, if any, wraps `ITaskbarList3` without our own `unsafe`?
- **Finding:** **NO crate exists on crates.io that wraps `ITaskbarList3::SetOverlayIcon` without requiring `unsafe`.**
  - `tao 0.37.1` has no `set_overlay_icon` or `set_badge_label` method on Windows (`WindowExtWindows` only offers `set_skip_taskbar` and `has_undecorated_shadow`).
  - The official Microsoft `windows` crate (`windows::Win32::UI::Shell::ITaskbarList3`) exposes `SetOverlayIcon(hwnd, hicon, description)`, but calling COM methods and `CoCreateInstance` is inherently marked `unsafe`.
  - The `winsafe = "0.0.29"` crate wraps `ITaskbarList3::SetOverlayIcon`, but constructing `winsafe::HWND` from tao's window handle requires `unsafe { HWND::from_ptr(window.hwnd()) }`, and creating the `HICON` badge bitmap in memory also requires Win32 GDI calls that involve `unsafe`.
- **Decision & Architecture in WP8:**
  - **Open Exception:** Per explicit user directive, zero `unsafe` was written for taskbar badges and the `windows` crate was not added.
  - **Pure Validation:** Full parameter and range validation is maintained in `wavical-ipc` (`0` clears, `1..=99` exact count, `>99` renders `"99+"`, negative values rejected) with comprehensive unit tests.
  - **Backend Stub:** In `wavical-native`, `NativeService::badge_set` validates the count, logs a structured `tracing::warn!` event, and returns a generic `"Taskbar overlay badge is not supported on this platform"` error (does NOT return success).
  - **Deferred Implementation:** The actual native Windows `ITaskbarList3` rendering is deferred to a future hardening phase where an isolated, human-reviewed `unsafe` change can be performed.

### 3.2 Context Menu (`muda::ContextMenu::show_context_menu_for_hwnd`)
- `muda`'s popup menu method on Windows takes a raw `isize` window handle and is defined as `unsafe fn show_context_menu_for_hwnd(...)`.
- Isolated safely in `wavical-native` (`crates/wavical-native/src/tray.rs`) under scoped `#[allow(unsafe_code)]`, keeping all other crates (and `wavical-shell`) strictly `#![forbid(unsafe_code)]`.

### 3.3 WebView2 Detection & Renderer Crash Recovery in `wry`
- **Detection:** `wry::webview_version() -> Result<String, wry::Error>` is completely safe and already used in `wavical-shell`.
- **Renderer Crash Events:** Pure retry state machine in `wavical-ipc` with 30-second backoff window and single-reload recovery.

---

## 4. Status

- WP1 through WP8 implementations complete.
- All crates (`wavical-ipc`, `wavical-config`, `wavical-protocol`, `wavical-cli`, `wavical-shell`) strictly maintain `#![forbid(unsafe_code)]`.
- `wavical-native` enforces `#![deny(unsafe_code)]` with exactly one scoped `#[allow(unsafe_code)]` for `muda::show_context_menu_for_hwnd`.
- Taskbar overlay badge recorded as an open exception: pure validation enforced, OS COM calls deferred to human-reviewed hardening.

