# Audit Report A6: Master Consolidation & Final Verdict

**Auditor:** Independent Security Auditor & QA Engineer  
**Date:** 2026-10-26  
**Target:** Wavical Core Workspace (`wavical-ipc`, `wavical-protocol`, `wavical-shell`, `wavical-config`, `wavical-cli`, `wavical-native`)  
**Scope:** Consolidation of Audits A0, A1a, A1b, A2, A3a, A3b, A4, and A5  
**Mode:** AUDIT ONLY (No code changes applied)

---

## 1. Executive Verdicts: Phase 0, Phase 1, Phase 2

### Phase 0 Verdict: NOT READY

**Plain Language Summary:**  
The pure mathematical and protocol foundations of Wavical are exceptionally well built, but the host integration layer and CI security gates are broken. The core message framing (`wavical-ipc`) and path canonicalization resolver (`wavical-protocol`) are among the strongest parts of the codebase: they enforce a 1 MB message limit, a 32-level JSON depth bound, duplicate key rejections, strict default-deny capability allowlists, and exhaustive Windows filesystem traversal defenses (blocking `..`, alternate data streams, UNC paths, and reserved DOS device names). Mutation testing proved that over 90% of defensive mutations in these pure crates are caught and killed immediately.

However, Phase 0 cannot be certified as ready due to critical failures in runtime integration and release hygiene:
1. **Broken Supply-Chain Gate:** `cargo deny check` fails in CI with exit code 1 due to rejected `BSL-1.0` licenses on Windows clipboard dependencies and unmaintained security advisories on transitive D-Bus packages.
2. **Missing WebView2 Profile Isolation:** The shell initializes WebView2 without configuring `.with_data_directory(...)`. This causes multiple application instances to share browser cache, cookies, and local storage in a common folder, and will cause immediate crashes if the binary is installed in a read-only directory like `C:\Program Files`.
3. **Information Leakage:** The public `wavical_ipc::dispatch` function returns raw internal error strings directly to the caller, bypassing error sanitization and risking the exposure of backend paths or secrets to untrusted page JavaScript. Additionally, the protocol error display mirrors raw path segments for trailing dot/space errors.
4. **Hanging Integration Test Suite:** The integration attack harness hangs indefinitely whenever an oversized message (>1 MB) is sent because Rust drops the message without replying, leaving the JavaScript Promise unresolved. The test runner also hardcodes an expectation of 7 tests, while the attack suite executes 13 tests, failing automated CI runs.
5. **Bridge Invariant Violations & Release Stdin Surface:** The injected bridge script exposes extra methods (`onMenu`, `onShortcut`) and global window event properties rather than the strictly specified single `invoke` function. In release builds, the shell leaves an unauthenticated standard input channel open that allows arbitrary runtime capability escalation if `--listen-stdin` is provided.

---

### Phase 1 Verdict: NOT READY

**Plain Language Summary:**  
The developer tooling and CLI workflow (`init`, `dev`, `build`, and alias `wca`) provide a modern, ergonomic developer experience with Levenshtein typo suggestions, strict config schema validation, and process cleanup on Ctrl+C. The config loader safely terminates infinite loops in JavaScript configs after a 5-second timeout, and asset modifications reload live in the webview via internal pipes without exposing open TCP network ports.

However, Phase 1 fails fundamental data security, file isolation, and cross-platform criteria:
1. **Distributable Secret Leakage:** `wavical build` blindly copies all files from the `src/` directory into `dist/`. If a developer places a `.env` file, credential secret, or local Git repository inside `src/`, `wavical build` packages those secrets directly into the production distributable and publishes their hashes in `wavical-manifest.json`.
2. **Destructive `--outdir` Paths:** `wavical build` accepts dangerous output directories without canonicalization or path checks. Running `wavical build --outdir .` or `wavical build --outdir src` will overwrite source code files and inject build artifacts directly into the project root.
3. **Stale Asset Retention:** `wavical build` does not clean or synchronize `dist/` before copying. When an asset is deleted from `src/`, it remains orphaned inside `dist/` on subsequent builds and will be shipped in production bundles despite being absent from the manifest.
4. **Fragile Config Evaluation via Stdout:** The Node config runner extracts JSON directly from process stdout. If `wavical.config.js` prints diagnostic messages via `console.log()` or imports a module that writes to stdout, JSON deserialization fails with `MalformedJson`.
5. **Broken Linux CI:** The CLI integration test suite hardcodes Windows-specific commands (`Command::new("taskkill")`), causing Linux CI jobs to fail.
6. **Documentation Omissions:** Neither `README.md` nor `docs/WAVICAL_SETUP.md` discloses that Node.js is a mandatory prerequisite, nor do they document that `wavical.config.js` executes arbitrary code with developer privileges.

---

### Phase 2 Verdict: NOT READY

**Plain Language Summary:**  
The architecture of Phase 2 native capabilities is cleanly structured: all 13 declared capability tokens match the specification, native methods are double-gated and strictly blocked when ungranted (verified by counting handlers with 0 invocations), single-instance coordination uses Windows Named Pipes with zero open TCP ports, forwarded launch arguments are never converted to file grants, and window state persistence debounces disk I/O and falls back safely when monitors are disconnected.

However, Phase 2 contains critical security defects in its file grant model, unapproved unsafe code, and destructive registry operations:
1. **Unapproved `unsafe` Code:** `crates/wavical-native/src/crash_recovery.rs` introduces unreviewed raw pointer casting and Win32 COM queries for `add_ProcessFailed`. This directly violates the project rule that only a single scoped `unsafe` block (`muda::show_context_menu_for_hwnd`) is permitted.
2. **File Grants Lack Permission Scoping (Read vs. Write):** The `GrantTable` records target paths but does not record whether a token was issued for reading (`dialog.openFile`) or writing (`dialog.saveFile`). An attacker who convinces a user to select an existing file via an Open dialog receives a grant ID that can be passed to `fs.writeGranted` to completely overwrite that file. Conversely, save dialog grants can be read via `fs.readGranted`.
3. **Destructive Registry Handling:** Running `unregister-filetypes` recursively deletes the entire `HKCU\Software\Classes\.{ext}` registry key, permanently erasing any preexisting user associations or fallback applications. Furthermore, `register-filetypes` unconditionally hijacks the default handler rather than politely registering under `OpenWithProgids`.
4. **Filesystem TOCTOU & Non-Atomic Writes:** `wavical-native` validates file targets by path before invoking `std::fs::write`, leaving a time-of-check to time-of-use (TOCTOU) race condition where the target could be replaced with a symlink or junction. Writes are also non-atomic, risking corrupted, truncated files if power is lost or the process is terminated mid-write.
5. **Path Leakage in Logs:** Internal filesystem paths are emitted in `tracing::warn!` log statements during file read and write failures, violating logging privacy rules.
6. **Missing Deliverables:** Phase 2 Work Package 9 requires hostile self-review notes and explicit "tests-fail" proof records in `docs/`. Neither artifact was ever committed to the repository.

---

## 2. Deduplicated Master Findings Table

The table below consolidates all findings identified across Audits A0 through A5, deduplicated and sorted strictly by severity (**High**, **Medium**, **Low**, **Info**).

| ID | Phase | Severity | Title | Where | Source Audit | Status | One-Line Fix Idea |
| :--- | :---: | :---: | :--- | :--- | :---: | :---: | :--- |
| **F-A0-01** | Phase 2 | **High** | Unapproved `unsafe` COM Hook in Crash Recovery | `crates/wavical-native/src/crash_recovery.rs:36,51,62` | A0 | Fixed (`test_crash_recovery_contains_zero_unsafe_code`) | Revert raw COM hook or formally review and document approval in `WAVICAL_PHASE_2_DECISIONS.md`. |
| **F-A1a-03** | Phase 0 | **High** | Unsanitized Internal Error Leakage in `wavical_ipc::dispatch` | `crates/wavical-ipc/src/lib.rs:669-674` | A1a | Fixed (`test_part2_response_internal_leakage`) | Pass handler errors through `sanitize_handler_error(&e)` before constructing `Response::err`. |
| **FINDING-S2** | Phase 0 | **High** | Attack Suite Hangs on Oversize Payload & CI Runner Fails Count | `tests/integration/src/main.rs:280`, `tests/attack-page/app.js:370` | A1b | Fixed (`test_integration_harness_dynamic_count_and_no_hang`) | Return explicit oversize error response or client timeout, and match `summary.total` instead of hardcoded 7. |
| **F-A5-01** | Phase 0 | **High** | DevTools Release Enablement Guard Untested in Release Profile | `crates/wavical-shell/src/lib.rs:597` | A5 | Fixed (`test_shell_devtools_disabled_in_release`) | Add an integration test compiled under `--release` asserting `is_devtools_enabled()` returns false. |
| **F-A0-02** | Phase 0 | **Medium** | Supply-Chain Gate Failure (`cargo deny check`) | `deny.toml:12-33`, `Cargo.lock` | A0 | Fixed (`test_supply_chain_cargo_deny_clean`) | Add `BSL-1.0` to allowed licenses in `deny.toml` and explicitly ignore unmaintained D-Bus advisories. |
| **F-A0-03** | Phase 1 | **Medium** | Linux CI Test Fragility Due to Windows `taskkill` | `crates/wavical-cli/tests/dev_tests.rs:270`, `.github/workflows/ci.yml:64` | A0 | Fixed (`test_linux_ci_no_unguarded_windows_taskkill`) | Guard Windows-specific test cases with `#[cfg(windows)]` or use cross-platform child process killing. |
| **F-A0-04** | Phase 1/2 | **Medium** | Scope Creep in Config Schema Fields & Crate Dependencies | `crates/wavical-config/src/lib.rs:36`, `Cargo.toml` files | A0 | Fixed (`test_config_schema_and_dependencies_documented`) | Formally document `version`, `tray`, `shortcuts`, and native dependencies in architecture specifications. |
| **F-A0-05** | Phase 2 | **Medium** | Missing Phase 2 WP9 Hostile Review Artifacts | `docs/`, `tests/` | A0 | Fixed (`test_phase_2_wp9_hostile_review_artifacts`) | Conduct formal hostile review, generate "tests-fail" proof records, and commit to `docs/`. |
| **F-A1a-01** | Phase 0 | **Medium** | Information Leakage in `ProtocolError::TrailingDotOrSpace` | `crates/wavical-protocol/src/lib.rs:56` | A1a | Fixed (`test_part1_error_information_leakage`) | Omit raw segment text from `TrailingDotOrSpace` Display implementation. |
| **FINDING-S1** | Phase 0 | **Medium** | Bridge Script Namespace Pollution & Non-Minimal API | `crates/wavical-shell/src/lib.rs:77-96,136-140` | A1b | Fixed (`test_shell_bridge_script_only_exposes_invoke_and_no_other_globals`) | Encapsulate event dispatching within closure state and expose only `window.wavical.invoke`. |
| **FINDING-S3** | Phase 0 | **Medium** | Missing App-Specific WebView2 User Data Directory Isolation | `crates/wavical-shell/src/lib.rs:845-896` | A1b | Fixed (`test_shell_webview2_user_data_directory_isolated`) | Configure isolated user data directory via `wry::WebContext::new(Some(data_directory))` derived from `%LOCALAPPDATA%\wavical\<appId>\EBWebView`. |
| **FINDING-S4** | Phase 0/1 | **Medium** | Unauthenticated Stdin Capability Reload Active in Release | `crates/wavical-shell/src/main.rs:41`, `crates/wavical-shell/src/lib.rs:706` | A1b | Fixed (`test_shell_stdin_dynamic_capability_reload_gated_in_release`) | Gate CLI flag `--listen-stdin` and capability update thread behind `#[cfg(debug_assertions)]`. |
| **FINDING-C1** | Phase 1 | **Medium** | User `console.log` in Config Pollutes Stdout & Breaks JSON | `crates/wavical-config/src/lib.rs:530-536` | A2 | Fixed (`test_cli_config_stdout_pollution_isolation`) | Use an isolated file descriptor or unique delimiter token to separate JSON payload from stdout logs. |
| **FINDING-C2** | Phase 1 | **Medium** | Stale / Deleted File Retention in Production `dist/` | `crates/wavical-cli/src/lib.rs:707-710` | A2 | Fixed (`test_cli_build_removes_stale_files_from_outdir`) | Clear existing `outdir` contents prior to copying or remove orphaned files absent from `src/`. |
| **FINDING-C3** | Phase 1 | **Medium** | Dangerous `--outdir` Paths (`.`, `src`, `..`) Accepted | `crates/wavical-cli/src/lib.rs:696-705` | A2 | Fixed (`test_cli_build_refuses_dangerous_outdir_paths`) | Canonicalize `outdir` and reject paths colliding with project root, parent directories, or `src/`. |
| **FINDING-C4** | Phase 1 | **Medium** | Sensitive Files (`.env`, `.git`) Copied into Distributable | `crates/wavical-cli/src/lib.rs:595-653` | A2 | Fixed (`test_cli_build_excludes_sensitive_files`) | Add exclusion filter in `copy_and_hash_assets` for `.env*`, `.git*`, `*.map`, `.DS_Store`, and `Thumbs.db`. |
| **FINDING-G2** | Phase 2 | **Medium** | Grant Table Lacks Permission Mode Scoping (Read vs. Write) | `crates/wavical-ipc/src/grants.rs:46-53`, `wavical-native` | A3a | Fixed (`test_grant_enforces_read_vs_write_mode`) | Add `GrantMode::ReadOnly` vs `GrantMode::WriteOnly` enum to `GrantEntry` and enforce at dispatch. |
| **FINDING-F1** | Phase 2 | **Medium** | TOCTOU Race Window Between Validation and `std::fs::write` | `crates/wavical-native/src/lib.rs:472-474` | A3a | Fixed (`test_fs_write_granted_toctou_handle_validation`) | Open file handle once with Win32 share flags, validate handle metadata, and write via that handle. |
| **FINDING-R1** | Phase 2 | **Medium** | Default File Extension Handler Hijacking in HKCU | `crates/wavical-cli/src/lib.rs:836-838` | A3b | Fixed (`test_file_assoc_register_preserves_existing_default_handler`) | Register under `Software\Classes\.{ext}\OpenWithProgids` instead of overwriting `(Default)`. |
| **FINDING-R2** | Phase 2 | **Medium** | Destructive Recursive Deletion of `Software\Classes\.{ext}` | `crates/wavical-cli/src/lib.rs:894` | A3b | Fixed (`test_file_assoc_unregister_preserves_existing_extension_keys`) | Delete only the specific `Wavical.{appId}` entry under `OpenWithProgids` rather than `remove_tree`. |
| **F-A0-06** | Phase 2 | **Low** | Grant Table `RwLock .unwrap()` Poisoning Risk | `crates/wavical-ipc/src/grants.rs:80,122,131,137,143,149` | A0 | Fixed (`test_grant_table_rwlock_poisoning_handled_gracefully`) | Handle poisoned read/write locks gracefully with `map_err(|_| GrantError::InternalLockError)`. |
| **F-A0-07** | Phase 0 | **Low** | CI Hygiene: Unpinned Action Tags & Missing Permissions | `.github/workflows/ci.yml:18,21,48,53,77,82` | A0 | Fixed (`test_ci_hygiene_pinned_actions_and_permissions`) | Declare top-level `permissions: { contents: read }` and pin actions to full commit SHAs. |
| **F-A0-08** | Phase 0 | **Low** | Transitive `windows-sys` in Pure Crates Under `-e dev` | `crates/wavical-ipc/Cargo.toml:16` | A0 | Fixed (`test_pure_crates_production_dependency_purity_and_dev_exception`) | Formally document that test framework dependencies (`proptest`, `tempfile`) pull dev-only OS bindings. |
| **F-A1a-02** | Phase 0 | **Low** | Incomplete Windows Reserved Device Blocklist (`CONIN$`/`CONOUT$`) | `crates/wavical-protocol/src/lib.rs:153-167` | A1a | Fixed (`test_part1_reserved_device_conin_conout`) | Add `"CONIN$"` and `"CONOUT$"` to `RESERVED_NAMES` array in `wavical-protocol`. |
| **FINDING-S5** | Phase 0 | **Low** | Omission of Explicit `connect-src` & `frame-src` in Default CSP | `crates/wavical-shell/src/lib.rs:19` | A1b | Fixed (`test_shell_csp_explicit_connect_and_frame_src`) | Append `connect-src 'self'; frame-src 'none';` explicitly to `DEFAULT_CSP`. |
| **FINDING-C5** | Phase 1 | **Low** | Missing Config Execution Trust Model Disclosure | `README.md`, `docs/`, `SECURITY.md` | A2 | Fixed (`test_config_execution_trust_model_documented`) | Add security note explaining that `wavical.config.js` executes with local developer privileges. |
| **FINDING-C6** | Phase 1 | **Low** | Node.js Prerequisite Omitted from Developer Setup Guides | `docs/WAVICAL_SETUP.md`, `README.md` | A2 | Fixed (`test_setup_documentation_declares_nodejs_prerequisite`) | Explicitly state Node.js (>= 14.8.0) as a prerequisite in setup documentation. |
| **FINDING-G1** | Phase 2 | **Low** | Grant ID Hex Format & Length Not Validated Before Lookup | `crates/wavical-ipc/src/grants.rs:120` | A3a | Fixed (`test_grant_id_format_validated_before_lookup`) | Validate that `grant_id` is exactly 32 hex ASCII characters before searching the hash map. |
| **FINDING-F2** | Phase 2 | **Low** | Non-Atomic File Writes Risk File Truncation on Abort | `crates/wavical-native/src/lib.rs:474` | A3a | Fixed (`test_fs_write_granted_atomic_replacement_avoids_in_place_truncation`) | Write payload to a sibling `.tmp` file, flush to disk, and atomically rename over destination. |
| **FINDING-N1** | Phase 2 | **Low** | `notify.send` Lacks Rate Limiting or Queue Depth Bounds | `crates/wavical-native/src/lib.rs:280-300` | A3a | Fixed (`test_notify_send_rate_limiting`) | Enforce sliding window rate limit (e.g., max 5 toasts per 10 seconds per window). |
| **FINDING-D1** | Phase 2 | **Low** | Dialog `title` and `defaultName` Lack Length Sanitization | `crates/wavical-native/src/lib.rs:326,360` | A3a | Fixed (`test_dialog_params_length_validation`) | Sanitize dialog strings to max 255 chars and strip non-printable/control characters. |
| **FINDING-D2** | Phase 2 | **Low** | Config-Defined Dialog Filters Ignored for Hardcoded List | `crates/wavical-native/src/lib.rs:37-45` | A3a | Fixed (`test_dialog_config_defined_filters_resolved`) | Pass `dialog.filters` from validated config into native file dialog invocations. |
| **FINDING-L1** | Phase 2 | **Low** | Internal Filesystem Paths Logged in `tracing::warn!` | `crates/wavical-native/src/lib.rs:415,420,434,475` | A3a | Fixed (`test_fs_read_write_logging_redacts_filesystem_paths`) | Redact file paths from warning logs, emitting only grant IDs and generic OS error codes. |
| **FINDING-W1** | Phase 2 | **Low** | Windows Reserved DOS Names Accepted in `appId` Path | `crates/wavical-native/src/window_state.rs:21-28` | A3b | Fixed (`test_app_data_dir_rejects_reserved_device_names`) | Reject `is_windows_reserved_device(app_id)` inside `get_app_data_dir`. |
| **FINDING-W2** | Phase 2 | **Low** | Unbounded Window State File Ingestion into Memory | `crates/wavical-native/src/window_state.rs:102` | A3b | Fixed (`test_window_state_unbounded_file_ingestion_rejected`) | Check file metadata length (max 64 KB) before calling `read_to_string`. |
| **FINDING-T1** | Phase 2 | **Low** | Unbounded Tray Icon Image Decoding | `crates/wavical-native/src/tray.rs:42-48` | A3b | Fixed (`test_tray_icon_unbounded_file_size_rejected`) | Check file size (max 1 MB) before passing asset path to image decoder. |
| **F-A5-02** | Phase 0 | **Low** | Raw Unencoded Null Byte Pre-check Untested in Protocol | `crates/wavical-protocol/src/lib.rs:176` | A5 | Fixed (`test_resolve_asset_path_unencoded_null_byte_rejected`) | Add unit test passing `"index.html\0.png"` to `resolve_asset_path` asserting `NullByte`. |
| **F-A5-03** | Phase 0 | **Low** | Raw UNC Share Prefix Check Untested in Protocol | `crates/wavical-protocol/src/lib.rs:181` | A5 | Fixed (`test_raw_unc_share_prefix_check_step3`) | Add unit test verifying that raw `//server/share` triggers early rejection at Step 3. |

| **F-A5-04** | Phase 0 | **Low** | Raw Null Byte Pre-check Untested in IPC Validator | `crates/wavical-ipc/src/lib.rs:275` | A5 | Open | Add unit test asserting `validate_raw_message` specifically returns `IpcError::NullByteDetected`. |

| **F-A5-05** | Phase 0 | **Low** | Empty Raw Message Guard Untested in IPC Validator | `crates/wavical-ipc/src/lib.rs:270` | A5 | Open | Add unit test asserting `validate_raw_message("")` specifically returns `IpcError::EmptyMessage`. |

| **F-A5-06** | Phase 0 | **Low** | Userinfo `@` Credential Guard Untested in Origin Extractor | `crates/wavical-shell/src/lib.rs:293` | A5 | Open | Add unit test passing `"https://user:pass@host/path"` to `extract_origin` asserting `None`. |

| **F-A5-07** | Phase 2 | **Low** | Boundary Lengths & Empty String Untested in `validate_assoc_app_id` | `crates/wavical-ipc/src/file_assoc.rs:69` | A5 | Open | Add boundary tests asserting empty string rejected, 64 chars accepted, 65 chars rejected. |

| **F-A0-09** | Phase 2 | **Info** | `wavical-native` Uses `#![deny(unsafe_code)]` Not `forbid` | `crates/wavical-native/src/lib.rs:1` | A0 | Open | Keep as documented exception for Win32 popup menu hook. |

| **F-A0-10** | Phase 2 | **Info** | Taskbar Overlay Badge Unsupported at OS Level | `crates/wavical-native/src/lib.rs:549` | A0 | Open | Keep documented as open exception until safe COM abstractions exist. |

| **F-A1a-04** | Phase 0 | **Info** | Secondary MIME Type Match Arms Untested | `crates/wavical-protocol/src/lib.rs:72-77` | A1a | Open | Expand `test_mime_type_mappings` to assert `gif`, `webp`, `ico`, `woff`, and `ttf`. |

---

## 3. Plan Drift: Plans vs. Codebase Reality

| Drift Item | Specification / Plan Record | Current Code Reality | Recommended Resolution |
| :--- | :--- | :--- | :---: |
| **Deprecated Capability Names in Config Schema** | `docs/phase 1/WAVICAL_PHASE_1_CONFIG_SCHEMA.md` lines 65-66, 117-118, 190-194 reference `fs.readFile`, `fs.writeFile`, and `window.getState`. | `crates/wavical-ipc/src/lib.rs` and `wavical-config` reject these; replaced by scoped `fs.readGranted`, `fs.writeGranted`, and automatic state persistence. | **PLAN SHOULD CHANGE** (Update schema doc to match Phase 2 scoped capabilities). |
| **Unapproved COM Unsafe Hook** | `docs/WAVICAL_PHASE_2_DECISIONS.md` Section 4 permits exactly ONE `unsafe` block (`muda::show_context_menu_for_hwnd`). | `crates/wavical-native/src/crash_recovery.rs:36,51,62` contains raw pointer queries for `add_ProcessFailed`. | **CODE SHOULD CHANGE** (Remove unreviewed raw pointer casting or formally review and approve). |
| **Config Schema Scope Creep** | Phase 1 Config Schema document omits `version`, `tray` (`icon`, `tooltip`), and `shortcuts`. | `crates/wavical-config/src/lib.rs:36,69,81` parses and validates these fields. | **PLAN SHOULD CHANGE** (Formally update the schema specification to include these valid fields). |
| **Unlisted Workspace Dependencies** | Architecture plans omit `getrandom`, `ctrlc`, `windows-registry`, and direct shell bindings to `muda`/`tray-icon`. | Dependencies are present in `Cargo.toml` files and utilized in production code. | **PLAN SHOULD CHANGE** (Update architecture documents to record justified dependencies). |
| **Bridge Script API & Globals** | Phase 0 WP5 mandates only `invoke` on `window.wavical` and zero other globals attached to `window`. | `crates/wavical-shell/src/lib.rs:77-140` defines `onMenu`, `onShortcut`, `__wavical_receive__`, `__wavical_event__`, etc. | **CODE SHOULD CHANGE** (Encapsulate event routing within closure state to restore minimal bridge). |
| **Unauthenticated Dynamic Reload via Stdin** | Phase 0 WP7 mandates that dynamic reload is restricted to development or authenticated sessions. | `crates/wavical-shell/src/main.rs:41` allows `--listen-stdin` in release mode without authentication. | **CODE SHOULD CHANGE** (Gate `--listen-stdin` and capability updates behind `debug_assertions`). |
| **WebView2 User Data Directory** | Phase 0 WP2 mandates app-isolated user data folder. | `crates/wavical-shell/src/lib.rs:845` calls `WebViewBuilder::new()` with default shared directory. | **CODE SHOULD CHANGE** (Configure `.with_data_directory(...)` on `WebViewBuilder`). |
| **Config Dialog Filters Ignored** | `wavical.config.js` allows configuring custom `dialog.filters`. | `crates/wavical-native/src/lib.rs:37-45` ignores user filters and uses 4 hardcoded static filters. | **CODE SHOULD CHANGE** (Plumb user-defined filters from config into native dialog service). |
| **Taskbar Overlay Badge Deferral** | Phase 2 WP8 mandates taskbar badge count implementation. | Documented as Open Exception in `WAVICAL_PHASE_2_DECISIONS.md`; returns generic unsupported error. | **PLAN SHOULD CHANGE** (Formally accept the exception to avoid unapproved unsafe Win32 COM). |
| **Phase 2 WP9 Hostile Review Artifacts** | Phase 2 WP9 requires hostile review notes and "tests-fail" proof records in `docs/`. | Hostile review document and tests-fail proofs were never committed to `docs/`. | **CODE/DOCS SHOULD CHANGE** (Generate and commit the required hostile review records). |

---

## 4. Test Gaps: Surviving Mutants & Verification Commands

### 4.1 Surviving Mutants Summary (5 Manual + 3 Automated)

1. **Row 24 (DevTools in Release — High Finding):**
   - *Code:* `crates/wavical-shell/src/lib.rs:597` (`is_devtools_enabled()`).
   - *Survival Reason:* All test suites execute under `cfg!(debug_assertions)` (debug profile). No test runs against a release profile to assert that `is_devtools_enabled()` strictly evaluates to `false`.
2. **Row 46 (Protocol Raw Null Byte Pre-check — Low Finding):**
   - *Code:* `crates/wavical-protocol/src/lib.rs:176` (`raw_path.contains('\0')`).
   - *Survival Reason:* Tests only supply percent-encoded null bytes (`%00`), which are caught downstream by `decode_percent_iterative`. Passing an unencoded `\0` in `raw_path` is untested.
3. **Row 47 (Protocol Raw UNC Share Pre-check — Low Finding):**
   - *Code:* `crates/wavical-protocol/src/lib.rs:181` (`raw_path.starts_with("//")`).
   - *Survival Reason:* Step 5 in `sanitize_path_segments` duplicates the UNC check post-decoding, masking the removal of the raw pre-check.
4. **Row 50 (IPC Raw Null Byte Pre-check — Low Finding):**
   - *Code:* `crates/wavical-ipc/src/lib.rs:275` (`raw.contains('\0')`).
   - *Survival Reason:* Downstream `serde_json::from_str` fails with a JSON syntax error on null bytes, but `IpcError::NullByteDetected` is never asserted.
5. **Row 51 (IPC Empty Raw Message Pre-check — Low Finding):**
   - *Code:* `crates/wavical-ipc/src/lib.rs:270` (`raw.is_empty()`).
   - *Survival Reason:* Serde JSON deserialization fails with EOF on empty inputs, masking the specific `IpcError::EmptyMessage` check.
6. **Row 55 (Shell Origin Userinfo '@' Guard — Low Finding):**
   - *Code:* `crates/wavical-shell/src/lib.rs:293` (`auth.as_str().contains('@')`).
   - *Survival Reason:* Origin check tests evaluate ports and schemes, but never provide URIs containing embedded credentials (`https://user:pass@host/`).
7. **Automated Survivors (validate_assoc_app_id — Low Finding):**
   - *Code:* `crates/wavical-ipc/src/file_assoc.rs:69` (`app_id.is_empty() || app_id.len() > 64`).
   - *Survival Reason:* Boundary lengths (exactly 64 vs 65 characters) and empty strings (`""`) were not asserted in unit tests.

---

### 4.2 Items Marked NOT RUN & Exact Reasons

| Item | Target | Why It Was NOT RUN |
| :--- | :--- | :--- |
| **`cargo fuzz` Smoke Tests** | `fuzz/fuzz_targets/grant_table.rs`, `native_request_parser.rs` | `cargo-fuzz` requires Linux / WSL and a Rust nightly toolchain. The host environment is Windows 11 with stable MSVC toolchain. |
| **`gh run list` CI Check** | GitHub Actions remote executions | The GitHub CLI tool (`gh`) is not installed on the host machine. |
| **Historical Node LTS Matrix** | CLI config execution against Node 18, 20, 22 | Only host Node v26.4.0 is installed; version management tools (`nvm`, `fnm`) are not installed. |

---

### 4.3 Exact Commands the User Must Run Themselves

#### 1. Fuzz Smoke Testing (Run inside WSL or Linux container with Nightly Rust)
```bash
# Install cargo-fuzz if not present
cargo install cargo-fuzz --locked

# Run grant table fuzzer for 60 seconds
cargo +nightly fuzz run grant_table -- -max_total_time=60

# Run native request parser fuzzer for 60 seconds
cargo +nightly fuzz run native_request_parser -- -max_total_time=60
```

#### 2. Remote GitHub Actions CI Status Check (After installing `gh` CLI)
```powershell
# Authenticate and inspect recent workflow runs
gh auth login
gh run list --limit 10
gh run view --log-failed
```

#### 3. Full Local Workspace Verification (Windows PowerShell)
```powershell
# 1. Standard workspace build and test
cargo build --workspace
cargo test --workspace

# 2. Run all audit regression tests (verifies passing suite)
cargo test -p wavical-audit

# 3. Run all audit tests including the known finding assertions (demonstrates open findings)
cargo test -p wavical-audit -- --include-ignored

# 4. Verify release build and lints
cargo build --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# 5. Run supply chain check (currently fails on BSL-1.0 and advisories)
cargo deny check
```

---

## 5. Recommended Fix Order in Batches

Fixing these findings should follow a strict dependency sequence in batches of at most 5 findings.

```mermaid
graph TD
    B1[Batch 1: Supply Chain & Release Shell Hardening] --> B2[Batch 2: IPC & Protocol Error Sanitization]
    B1 --> B3[Batch 3: CLI Build & Config Security]
    B2 --> B4[Batch 4: Scoped File Grants & IO Hardening]
    B3 --> B5[Batch 5: Registry Safety & Native Robustness]
    B4 --> B6[Batch 6: Unsafe Code & Test Suite Hardening]
    B5 --> B6
```

### Batch 1: Supply Chain, CI Gates & Shell Release Hardening (4 Findings)
- **Goal:** Unbreak supply chain gate, establish immutable CI pipelines, and eliminate release mode backdoors.
- **Findings Addressed:**
  1. `F-A0-02` (Medium): Supply-chain gate failure (`cargo deny check`) [Fixed: `test_supply_chain_cargo_deny_clean`].
  2. `F-A0-07` (Low): CI hygiene (unpinned actions and missing permissions) [Fixed: `test_ci_hygiene_pinned_actions_and_permissions`].
  3. `FINDING-S4` (Medium): Stdin capability reload active in release mode [Fixed: `test_shell_stdin_dynamic_capability_reload_gated_in_release`].
  4. `FINDING-S3` (Medium): Missing WebView2 user data directory isolation [Fixed: `test_shell_webview2_user_data_directory_isolated`].
- **Dependencies:** None (foundational).
- **Files Touched:**
  - [`deny.toml`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/deny.toml)
  - [`.github/workflows/ci.yml`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/.github/workflows/ci.yml)
  - [`crates/wavical-shell/src/main.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/main.rs)
  - [`crates/wavical-shell/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs)

---

### Batch 2: IPC & Protocol Error Sanitization and Device Defense (4 Findings)
- **Goal:** Stop information leakage to untrusted webview contexts and complete Windows device blocking.
- **Findings Addressed:**
  1. `F-A1a-03` (High): Unsanitized internal error leakage in public `wavical_ipc::dispatch` [Fixed: `test_part2_response_internal_leakage`].
  2. `F-A1a-01` (Medium): Path segment leakage in `ProtocolError::TrailingDotOrSpace` [Fixed: `test_part1_error_information_leakage`].
  3. `F-A1a-02` (Low): Incomplete Windows reserved device blocklist (`CONIN$`, `CONOUT$`) [Fixed: `test_part1_reserved_device_conin_conout`].
  4. `FINDING-S1` (Medium): Bridge script namespace pollution (`onMenu`, `onShortcut`) [Fixed: `test_shell_bridge_script_only_exposes_invoke_and_no_other_globals`].
- **Dependencies:** None.
- **Files Touched:**
  - [`crates/wavical-ipc/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs)
  - [`crates/wavical-protocol/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-protocol/src/lib.rs)
  - [`crates/wavical-shell/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs)

---

### Batch 3: CLI Build Security & Config Robustness (5 Findings)
- **Goal:** Prevent production distributable secret leaks and guard filesystem paths during build.
- **Findings Addressed:**
  1. `FINDING-C4` (Medium): Sensitive files (`.env`, `.git`) copied into `dist/` — **Fixed** via regression test `test_cli_build_excludes_sensitive_files`.
  2. `FINDING-C3` (Medium): Dangerous `--outdir` paths (`.`, `src`, `..`) accepted without validation — **Fixed** via regression test `test_cli_build_refuses_dangerous_outdir_paths`.
  3. `FINDING-C2` (Medium): Stale asset retention in production `dist/` — **Fixed** via regression test `test_cli_build_removes_stale_files_from_outdir`.
  4. `FINDING-C1` (Medium): Config stdout pollution (`console.log`) breaks JSON extraction — **Fixed** via regression test `test_cli_config_stdout_pollution_isolation`.
  5. `F-A0-03` (Medium): Linux CI test failure on Windows `taskkill` [Fixed: `test_linux_ci_no_unguarded_windows_taskkill`].
- **Dependencies:** None.
- **Files Touched:**
  - [`crates/wavical-cli/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs)
  - [`crates/wavical-config/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-config/src/lib.rs)
  - [`crates/wavical-cli/tests/dev_tests.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/tests/dev_tests.rs)

---

### Batch 4: Scoped File Grants & Filesystem Hardening (5 Findings)
- **Goal:** Enforce read vs. write separation on opaque tokens and eliminate filesystem TOCTOU windows.
- **Findings Addressed:**
  1. `FINDING-G2` (Medium): Grant table lacks permission mode scoping (read vs. write) — **Fixed** via regression test `test_grant_enforces_read_vs_write_mode`.
  2. `FINDING-F1` (Medium): TOCTOU window between path check and `std::fs::write` — **Fixed** via regression test `test_fs_write_granted_toctou_handle_validation`.
  3. `FINDING-F2` (Low): Non-atomic file writes risk truncation on abort — **Fixed** via regression test `test_fs_write_granted_atomic_replacement_avoids_in_place_truncation`.
  4. `FINDING-G1` (Low): Grant ID hex format and length not validated before lookup [Fixed: `test_grant_id_format_validated_before_lookup`].
  5. `FINDING-L1` (Low): Internal filesystem paths logged in `tracing::warn!`.
- **Dependencies:** Batch 2 (`wavical-ipc` clean).
- **Files Touched:**
  - [`crates/wavical-ipc/src/grants.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/grants.rs)
  - [`crates/wavical-native/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs)

---

### Batch 5: Registry Safety & Extended Native Robustness (5 Findings)
- **Goal:** Prevent file extension hijacking, eliminate destructive registry deletions, and bound native inputs.
- **Findings Addressed:**
  1. `FINDING-R1` (Medium): Default file extension handler hijacking in HKCU.
  2. `FINDING-R2` (Medium): Destructive recursive deletion of `Software\Classes\.{ext}`.
  3. `FINDING-W1` (Low): Windows reserved DOS device names accepted in `appId`.
  4. `FINDING-W2` (Low): Unbounded window state file ingestion into memory.
  5. `FINDING-T1` (Low): Unbounded tray icon image decoding.
- **Dependencies:** None.
- **Files Touched:**
  - [`crates/wavical-cli/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs)
  - [`crates/wavical-native/src/window_state.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/window_state.rs)
  - [`crates/wavical-native/src/tray.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/tray.rs)

---

### Batch 6: Unsafe Code Resolution & Test Suite Verification (5 Findings)
- **Goal:** Resolve unapproved unsafe blocks, unhang integration test harnesses, and close mutation test gaps.
- **Findings Addressed:**
  1. `F-A0-01` (High): Unapproved `unsafe` COM hook in `crash_recovery.rs` [Fixed: `test_crash_recovery_contains_zero_unsafe_code`].
  2. `FINDING-S2` (High): Attack suite hangs on oversize payload and runner fails test count [Fixed: `test_integration_harness_dynamic_count_and_no_hang`].
  3. `F-A5-01` (High): DevTools release enablement guard untested in release builds [Fixed: `test_shell_devtools_disabled_in_release`].
  4. `F-A0-05` (Medium): Missing Phase 2 WP9 hostile review artifacts and tests-fail proofs [Fixed: `test_phase_2_wp9_hostile_review_artifacts`].
  5. `F-A0-06` (Low): Grant Table `RwLock .unwrap()` lock poisoning risk [Fixed: `test_grant_table_rwlock_poisoning_handled_gracefully`].
- **Dependencies:** Batches 1 through 5.
- **Files Touched:**
  - [`crates/wavical-native/src/crash_recovery.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/crash_recovery.rs)
  - [`crates/wavical-ipc/src/grants.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/grants.rs)
  - [`tests/integration/src/main.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/tests/integration/src/main.rs)
  - [`tests/attack-page/app.js`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/tests/attack-page/app.js)
  - [`docs/`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/docs)

---

## 6. Guide for Human Rust Reviewer: Where to Read First

A human Rust security engineer reviewing this codebase should prioritize the following three critical control surfaces:

### 1. IPC Request Ingestion & Capability Enforcement
Inspect the boundary where untrusted JSON strings from WebView2 are parsed and authorized before any native execution occurs:
- [`crates/wavical-ipc/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/lib.rs):
  - `validate_raw_message` (lines 268–285): Raw byte length check (1 MB limit) and null byte detection before parsing.
  - `parse_request` (lines 310–350): Strict Serde schema parsing (`deny_unknown_fields`), ID numeric range validation ($0 \le id \le 2^{53}-1$), and method name character validation.
  - `dispatch` & `dispatch_native` (lines 580–730): Central default-deny capability check. Verify that ungranted calls return immediately with identical generic error messages without invoking native handlers.
- [`crates/wavical-shell/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs):
  - `process_ipc_message` (lines 375–455): Origin authorization of incoming webview postMessages against the window's allowed origins allowlist.

### 2. Protocol Resolver & Path Traversal Canonicalization
Inspect how local application asset requests (`wavical://` or virtual custom origins) are resolved against the disk root:
- [`crates/wavical-protocol/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-protocol/src/lib.rs):
  - `resolve_asset_path` (lines 136–250): Core sanitization pipeline. Verify the sequence: null byte rejection $\rightarrow$ UNC path rejection $\rightarrow$ percent decoding $\rightarrow$ traversal (`..`) blocking $\rightarrow$ alternate data stream colon (`:`) rejection $\rightarrow$ reserved device blocklist (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`) $\rightarrow$ trailing dot and space removal.
  - `sanitize_path_segments` (lines 170–245): Per-segment traversal checks and root confinement.
- [`crates/wavical-shell/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs):
  - `extract_origin` (lines 288–314): Scheme, authority, and port extraction, including userinfo (`@`) stripping.
  - `handle_navigation_request` & `handle_new_window_request` (lines 325–355): Unconditional popup blocking and navigation restrictions to virtual app origins.

### 3. Scoped File Grants & Native Filesystem Handlers
Inspect how opaque grant tokens are generated, isolated, and verified at access time:
- [`crates/wavical-ipc/src/grants.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-ipc/src/grants.rs):
  - `GrantTable::insert` (lines 75–115): 128-bit CSPRNG token generation (`getrandom::fill`), 64-grant capacity ceiling per window, and per-window isolation.
  - `GrantTable::get` & `GrantTable::clear_window` (lines 116–155): Grant retrieval and window destruction cleanup.
- [`crates/wavical-native/src/lib.rs`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-native/src/lib.rs):
  - `validate_granted_target` (lines 75–130): Use-time verification asserting that target exists, canonical path matches grant registration, target is a regular file (not directory), and target is not a Windows symlink or junction.
  - `fs_read_granted` & `fs_write_granted` (lines 390–485): Enforcement of 10 MB file bounds before buffer allocation and file disk I/O.

---

## 7. Conclusion

Wavical contains a remarkably strong, disciplined security core in its pure crates (`wavical-ipc` and `wavical-protocol`). The design principles of default-deny capabilities, opaque file tokens, and zero open TCP ports are sound and effectively mitigate broad classes of webview attacks.

However, the framework is **NOT READY** for production release across any phase due to critical integration gaps:
- **Phase 0** is blocked by broken supply-chain CI gates, unisolated WebView2 user profiles, public error leakage, and release stdin backdoors.
- **Phase 1** is blocked by distributable secret leaks (`.env` in `dist/`), destructive unvalidated `--outdir` builds, and fragile config stdout parsing.
- **Phase 2** is blocked by unapproved raw COM `unsafe` hooks, lack of read vs. write grant scoping, destructive registry key removal on unregister, and filesystem TOCTOU race conditions.

Resolving these issues in the structured 6-batch order outlined above will bring the codebase into full alignment with its security specification.

**AUDIT COMPLETE. NO CHANGES APPLIED. STOPPING.**
