# Patch Report

Ongoing log of F-series fixes for Wavical (`1.1.x`). Each patch uses a consistent structure so subsequent `Patch F<N>` entries can be appended in the same file.

---

## Patch F4

**Status:** Implemented and verified (`db660f5` + `be17d5f` on `main`; manual GUI launch on `test_app/my-app` remaining)  
**Baseline Commit:** `e1b51e4` (`1.1.3 - Network mode patch F3`) on `main` (`Cargo.toml` / `package.json` at `1.1.1`)  
**Commits:**
- `db660f5` — `F4: write build mode/integrity fields in manifest, enforce safe allowlist startup, add shell file logging`
- `be17d5f` — `F4: migrate Phase 1-4.5 wavical build tests to build-ultra-safe (closes F-A9-01)`

### 1. Introduction (What F4 Is)

`run_build` previously wrote `wavical-manifest.json` without `mode`, `allowlistSha256`, or per-page `inlineScriptHashes`, and did not copy `wavical.allowlist.json` into `dist/`. Because the shell treats a missing `mode` as `ultra-safe` (and safe mode checks `allowlistSha256`), every built app fell back to `ultra-safe` or failed safe-mode hash checks.

F4 fixes the build-to-shell network-mode pipeline by:
- Separating builds into `wavical build --allow-open` (`mode: "open"`), `wavical build-safe` (`mode: "safe"`, `allowlistSha256`, per-page `inlineScriptHashes`, copied `wavical.allowlist.json`), and `wavical build-ultra-safe` (`mode: "ultra-safe"` with inline `<script>` extraction in `dist/`), while refusing bare `wavical build` without `--allow-open`.
- Writing the mode and integrity fields through one shared function (`write_wavical_manifest`) covered by the Phase 4 signed update payload.
- Adding production file-only logging (`%LOCALAPPDATA%\Wavical\apps\<appId>\logs\shell.log`, 5 MB rotation) to `wavical-shell`, refusing to start tampered safe-mode apps before any window/webview creation, and aligning missing-mode diagnostics (`"manifest has no mode: app runs in ultra-safe"`) in `shell.log` and `wavical doctor --app`.
- Migrating existing Phase 1–4.5 `wavical build` test calls to `build-ultra-safe` (closing audit finding `F-A9-01`) and adding black-box tests modeled on `test_app/my-app`.

### 2. What Was Done

1. **Shared Validated App Log Path (`crates/wavical-config/src/lib.rs`)**
   - Added `MAX_APP_ID_LEN` (64), `is_safe_app_id_for_path`, and `app_log_file_path(local_appdata, app_id)` reusing `is_valid_app_id` and `wavical_ipc::ai::is_reserved_name` (rejecting traversal, slashes, dots, and Windows reserved device names).
   - Added unit test `test_app_log_file_path_validation`.
2. **Native Tampering Dialog (`crates/wavical-native/src/crash_recovery.rs`)**
   - Added `show_tampered_files_dialog()` using `rfd::MessageDialog` in the same style as `show_missing_webview2_dialog()`.
3. **Shell Startup Integrity & Shared Policy Helpers (`crates/wavical-shell/src/net_policy.rs`)**
   - Added `app_id` to `ShellBuildManifest` and `resolve_manifest_app_id` so standalone built apps resolve their `appId` from `wavical-manifest.json`.
   - Extracted single shared `verify_allowlist_sha256` and `find_allowlist_path` helpers used by both `resolve_shell_network_policy_with_remote` and `verify_safe_mode_startup_integrity`.
   - Updated missing-mode warning log to emit `"manifest has no mode: app runs in ultra-safe"`.
   - Fixed `handle_protocol_request_with_policy` asset hash verification to accept `"sha256:<hex>"` (strip prefix) as well as raw `"<hex>"`, enabling real CLI-built manifests (`"sha256:..."`) to serve `200 OK`.
4. **Production Shell File-Only Logging & Startup Refusal (`crates/wavical-shell/src/lib.rs`, `crates/wavical-shell/src/main.rs`)**
   - Added `SHELL_LOG_MAX_BYTES` (`5 * 1024 * 1024`), `prepare_rotated_log_file` (renames `shell.log` > 5 MB to `shell.log.1`, replacing prior backup), `init_production_file_logging_in_dir`, and `init_production_file_logging` using `tracing_subscriber::fmt().with_ansi(false).with_writer(Mutex::new(file)).try_init()`.
   - Removed stdout `tracing_subscriber::fmt::init()` from `wavical-shell`; if `LOCALAPPDATA` is missing or unwritable, the shell continues without file logging and without console printing.
   - In `wavical-shell` `main.rs`, initialized file logging and ran `verify_safe_mode_startup_integrity` **before** any window or WebView2 creation: on safe-mode allowlist mismatch/missing/invalid, logs `allowlist_integrity_refused` to `shell.log`, shows `show_tampered_files_dialog()` (unless `WAVICAL_HEADLESS_TEST` is set), and exits with code `1`.
   - Added `test_production_log_rotation_and_unwritable_fallback`.
5. **CLI Build Modes, Shared Manifest Writer & `doctor --app` (`crates/wavical-cli/src/lib.rs`)**
   - Added `Commands::BuildSafe`, `Commands::BuildUltraSafe`, `BuildArgs::allow_open`, and `CliError::{BuildModeRequired, BuildSafeAllowlistError, BuildPolicyError}`.
   - Extended `WavicalManifest` with `mode`, `allowlistSha256`, and `inlineScriptHashes`, written in one shared function `write_wavical_manifest` used by all three build modes (verified via repo-wide grep that no other production code writes `WavicalManifest`).
   - `wavical build` without `--allow-open` refuses and lists all three build choices (`--allow-open`, `build-safe`, `build-ultra-safe`).
   - `wavical build-safe` validates `wavical.allowlist.json`, writes `allowlistSha256`, copies `wavical.allowlist.json` verbatim into `dist/`, scans HTML files in `dist/` for per-page `inlineScriptHashes` (enforcing `MAX_INLINE_SCRIPTS_HASHED_PER_PAGE = 100`), and warns on inline handlers.
   - `wavical build-ultra-safe` refuses `window.remoteUrl`, copies `src/` to `dist/`, runs existing `html_scan::extract_frontend_inline_scripts` on `dist/` (leaving `src/` untouched), warns on inline handlers, and hashes rewritten HTML + `.wavical-extracted/*.js` into `manifest.assets`.
   - Updated `inspect_built_app` and `format_doctor_app_report` (`wavical doctor --app`) to report `"manifest has no mode: app runs in ultra-safe"` and print `Log file: <path>`.
6. **Documentation (`docs/SECURITY_MODEL.md`)**
   - Documented Section 7.1 safe-mode startup allowlist check, the pre-Phase-5 local-file modification boundary for the same Windows user, and Ed25519 signature protection of updates (`wavical-manifest.json` + `wavical.allowlist.json` inside `payloadSha256`). Note: `docs/` is listed in `.gitignore`, so `docs/SECURITY_MODEL.md` was updated on disk outside git commits.
7. **New F4 Tests & Dedicated Phase 1–4.5 Migration Commit**
   - Added `crates/wavical-cli/tests/f4_build_modes_tests.rs` (4 black-box tests covering bare `build` refusal, `--allow-open` / `build-safe` / `build-ultra-safe` manifest fields & shell enforcement & tampered safe-mode exit/no-webview, missing-mode `shell.log` + `doctor --app`, and signed update payload archive containing both `wavical-manifest.json` and `wavical.allowlist.json`).
   - Added `crates/wavical-shell/tests/f4_shell_manifest_tests.rs` (tests `resolve_shell_network_policy_with_remote`, `verify_safe_mode_startup_integrity`, and `handle_protocol_request_with_policy` directly against real CLI-built `dist/` outputs).
   - Migrated all existing Phase 1–4.5 `wavical build` test calls to `build-ultra-safe` in dedicated commit `be17d5f` (args/command variant only; zero assertions changed):
     - `crates/wavical-cli/tests/build_tests.rs` (10 tests, 12 calls)
     - `crates/wavical-cli/tests/backend_build_tests.rs` (4 tests, 5 calls)
     - `crates/wavical-cli/tests/e2e_tests.rs` (4 tests, 4 calls; `build --help` kept)
     - `crates/wavical-cli/tests/f2_dev_shell_tests.rs` (`test_build_excludes_all_shells_ignores_dev_url_and_prod_shell_ignores_stdin`, 1 call)
     - `tests/audit/tests/part4_cli.rs` (5 tests, 8 calls; `test_cli_wca_alias_parity` kept)
     - `crates/wavical-cli/src/lib.rs` (`test_cli_parsing_subcommands`)

### 3. Decisions Made

- **No new dependencies:** Used `tracing-subscriber`'s existing `MakeWriter` support for `Mutex<std::fs::File>` with `std::fs::OpenOptions` and manual 5 MB rotation (`shell.log` -> `shell.log.1`).
- **Keep `resolve_shell_network_policy*` contract intact:** Kept `resolve_shell_network_policy_with_remote` returning `NetworkMode::UltraSafe` on `allowlistSha256` mismatch so existing Phase 4.5 tests (`part9_network_modes.rs`, `network_modes_tests.rs`) pass unchanged, while `wavical-shell`'s `main.rs` calls `verify_safe_mode_startup_integrity` (sharing `verify_allowlist_sha256`) before window/webview creation and refuses to start.
- **Reuse existing inline extraction:** Confirmed Case 1 (`html_scan::extract_frontend_inline_scripts` already existed) and ran it exclusively on `out_dir` (`dist/.wavical-extracted/`), never touching `src/`.
- **Serve-time hash compatibility:** Stripped optional `"sha256:"` prefix in `handle_protocol_request_with_policy` because `wavical-cli` writes `"sha256:<hex>"` in `manifest.assets` while existing shell unit tests pass raw `<hex>`.

### 4. Decisions Needed from You

- **None remaining for F4.** (All F4 design questions from earlier turns were resolved.)
- **Pre-existing README audit failures noted (outside F4 scope):** `tests/audit/tests/part4_cli.rs` (`test_config_execution_trust_model_documented` / `FINDING-C5` and `test_setup_documentation_declares_nodejs_prerequisite` / `FINDING-C6`) and `tests/audit/tests/part8_phase3.rs` (`test_audit_f_a7_06_readme_model_license_disclaimer` / `F-A7-06`) fail against existing `README.md` content. Untouched per Rule 1.

### 5. Other Info (Files, Verification & Manual Checks)

#### Files Changed in `db660f5` (F4 Implementation + Tests) and `be17d5f` (Test Migration)
- `crates/wavical-config/src/lib.rs`
- `crates/wavical-native/src/crash_recovery.rs`
- `crates/wavical-shell/src/net_policy.rs`
- `crates/wavical-shell/src/lib.rs`
- `crates/wavical-shell/src/main.rs`
- `crates/wavical-cli/src/lib.rs`
- `crates/wavical-cli/tests/f4_build_modes_tests.rs` (new)
- `crates/wavical-shell/tests/f4_shell_manifest_tests.rs` (new)
- `crates/wavical-cli/tests/build_tests.rs` (migration commit `be17d5f`)
- `crates/wavical-cli/tests/backend_build_tests.rs` (migration commit `be17d5f`)
- `crates/wavical-cli/tests/e2e_tests.rs` (migration commit `be17d5f`)
- `crates/wavical-cli/tests/f2_dev_shell_tests.rs` (migration commit `be17d5f`)
- `tests/audit/tests/part4_cli.rs` (migration commit `be17d5f`)
- `docs/SECURITY_MODEL.md` (updated on disk; gitignored by `.gitignore`)
- `PATCH_REPORT.md` (this report)

#### Real Command Output Summary (after final code edit `be17d5f`)
1. `cargo build --workspace`: **EXIT=0** (`Finished dev profile in 27.69s`)
2. `cargo fmt --all --check`: **EXIT=0**
3. `cargo clippy --workspace -- -D warnings`: **EXIT=0**
4. `cargo test --workspace`:
   - `wavical-ai`: 57 lib + 14 `chat_tests` + 10 `cloud_tests` + 6 `engine_lifecycle_tests` + 12 `mock_tests` + 13 `rag_tests` — **all passed (0 failed)**
   - `wavical-audit` halted `cargo test --workspace` at `part4_cli` (`EXIT=101`) due to the 2 pre-existing `README.md` failures (`test_config_execution_trust_model_documented`, `test_setup_documentation_declares_nodejs_prerequisite`; 16/18 passed including all 5 migrated build tests); `part1_protocol` (12/12), `part2_ipc` (13/13), `part3_shell` (18/18) passed.
   - All remaining crates run individually after `be17d5f`:
     - `wavical-config` (51/51), `wavical-ipc` (107/107), `wavical-native`, `wavical-netpolicy`, `wavical-protocol`, `wavical-shell` (28 lib + 1 `f4_shell_manifest_tests` + 6 `hardening_tests` [1 ignored release-only test] + 7 `network_modes_tests` + 3 `node_backend_integration_tests` + 2 `remote_ui_tests` + 3 `updater_tests` + 1 `wp8_attack_suite`), `wavical-supply-chain-bench` (1/1) — **EXIT=0**
     - `wavical-cli`: `lib` (9/9), `allowlist_suggest_tests` (1/1), `backend_build_tests` (4/4), `build_tests` (10/10), `cli_tests` (5/5), `e2e_tests` (8/8), `f2_dev_shell_tests` (5/5), `f3_diagnostics_tests` (3/3), `f4_build_modes_tests` (4/4), `init_ai_tests` (13/13), `runtime_tests` (18/18), `signer_tests` (2/2), `vendor_and_extract_tests` (3/3) — **all passed**. `dev_tests` initially had 2 timeout failures (`test_dev_auto_creates_missing_src_dir`, `test_dev_server_lifecycle_and_hot_reload`) when racing on `target/dev-shell` with parallel `e2e_tests`; re-run standalone passed **4/4** (`EXIT=0`).
5. `cargo test -p wavical-audit -- --include-ignored`:
   - `part1_protocol` (12/12), `part2_ipc` (13/13), `part3_shell` (18/18), `part5_native_core` (20/20), `part6_native_rest` (20/20), `part7_redteam` (20/20), `part9_network_modes` (5/5) — **all passed**.
   - `part4_cli`: 16/18 passed (2 pre-existing `README.md` failures: `FINDING-C5`, `FINDING-C6`).
   - `part8_phase3`: 20/21 passed (1 pre-existing `README.md` failure: `F-A7-06`).

#### Manual Checks (Not Run)
- Run `wavical build --allow-open` on `test_app/my-app`, launch the built `wavical-shell.exe` against `dist/`, and visually verify in the WebView2 GUI that fonts, Tailwind styles, logos, preloader, scroll effects, and form submission behave identically to `wavical dev`.
- Visually verify the native OS error dialog (`show_tampered_files_dialog`) when launching a `build-safe` app after modifying `dist/wavical.allowlist.json` without `WAVICAL_HEADLESS_TEST=1`.

#### Security Assumptions
- Until Phase 5 adds Authenticode code signing and restrictive install-directory ACLs, a local process running as the same Windows user can modify files on disk (including `wavical-manifest.json`, `wavical.allowlist.json`, or `wavical-shell.exe`). App updates are protected end-to-end by Ed25519 signature verification over `manifest.json` (`payloadSha256`).
- `wavical-shell` builds `%LOCALAPPDATA%\Wavical\apps\<appId>\logs\shell.log` only after `is_safe_app_id_for_path` validates `appId`, and falls back silently to no file logging if `LOCALAPPDATA` is missing/unwritable (never writing logs to stdout/stderr).

---

## Patch F5

**Status:** Implemented and verified in working tree (uncommitted per user instruction; manual GUI checks on `my-app` remaining)  
**Baseline Commit:** `45fee8d` (`1.1.4 - Network mode patch F4 report`)  

### 1. Introduction (What F5 Is)

After F4 fixed build modes and manifest integrity, two real-app usability/security behaviors remained: `mailto:` and `tel:` links were rejected by the system opener, and clicking a cross-origin `<a href="https://...">` link in `open` mode navigated the entire app window away from the app instead of opening the system browser.

F5 fixes both and documents WebView/browser differences:
- **Validated `mailto:` and `tel:` system opener:** Extends the single validated external opener so `mailto:` and `tel:` launch in the OS handler (mail app, phone/Teams) — allowed by default in `open`, allowed in `safe` only via validated `"openExternalSchemes": ["mailto", "tel"]` in `wavical.allowlist.json`, never allowed in `ultra-safe`, capped by URL length and control-character/multi-encoding rejection, logged by scheme only (never the address), while keeping all other schemes (`sms:`, `whatsapp:`, `skype:`, `ms-*`, `file:`, etc.) blocked.
- **Default external top-level link handling (`window.externalLinks`):** Defaults user-initiated top-level cross-origin link clicks (`<a href>`) to open in the system browser while keeping the app on its page, preserving redirect-based login flows and `navigate` allowlist destinations in-app, and adding `window.externalLinks: "browser" | "in-app"` (default `"browser"`) to `wavical.config.js`.
- **Browser differences documented:** Documents in `docs/NETWORK_MODES.md` under `## Differences from a browser` that plain `http://` stays blocked in `open` mode unless in `localExceptions`, popup-based `window.open` OAuth is not supported yet, and WebView2 tracking prevention can block third-party storage in embeds.

### 2. What Was Done

1. **`openExternalSchemes` Allowlist Validation & Policy Decisions (`crates/wavical-netpolicy`)**
   - `src/lib.rs`: Added `ExternalLinksMode` (`Browser` [default], `InApp`, `#[serde(rename_all = "kebab-case")]`).
   - `src/allowlist.rs`: Added `openExternalSchemes` (`open_external_schemes`) to `RawAllowlist` and `Allowlist`, validated by `parse_external_schemes` to accept only `"mailto"` and `"tel"` (lowercased, no duplicates, max `MAX_ENTRIES_PER_LIST`), returning `AllowlistError::InvalidExternalScheme` on invalid entries. Added `Allowlist::allows_external_scheme`.
   - `src/decisions.rs`: Added `NavigationDecision { AllowInApp, OpenExternal, Deny(&'static str) }`, `decide_top_level_navigation`, `validate_mailto_or_tel_url`, and `contains_encoded_control_bytes` (unwraps up to 4 layers of `%25` hex encoding, rejects C0/DEL control bytes `<= 0x1F`, `0x7F`, and `char::is_control`). Updated `is_baseline_blocked_scheme` and `decide_open_external` to enforce scheme, length, and encoding rules per mode. Preserved existing `decide_navigation` signature by delegating to `decide_top_level_navigation(..., false, ExternalLinksMode::Browser)`.
   - `src/diagnostics.rs`: Updated `BlockedNavigation` / `BlockedNewWindow` hints in `format_diagnostic_hint` for `open_external_control_chars` and `open_external_scheme_not_allowlisted`.
   - `tests/policy_tests.rs`: Added `f5_open_external_schemes_allowlist_and_top_level_navigation_decisions`.

2. **Config Schema Support for `window.externalLinks` (`crates/wavical-config/src/lib.rs`)**
   - Added `ExternalLinksMode`, `WindowConfig.external_links` (`rename = "externalLinks"`, `alias = "external_links"`), `WindowConfig::external_links_or_default()`, and `AppConfig::external_links()`.
   - Updated `arb_app_config()` proptest generator and added `test_window_external_links_config_validation`.

3. **Shell Opener Logging, Click Interception & IPC Enforcement (`crates/wavical-shell`)**
   - `src/net_policy.rs`:
     - Added `external_links` to `ShellBuildManifest` and `ShellNetworkPolicy`, `parse_external_links_mode`, and `resolve_shell_network_policy_with_options` (keeping `resolve_shell_network_policy` and `resolve_shell_network_policy_with_remote` intact; production builds read `manifest.external_links` strictly and default to `Browser`).
     - Updated `open_external_validated` to log `tracing::info!(security_event = "open_external_allowed", ..., scheme = %parsed.scheme(), ...)` (scheme only, never address/recipient) and pass `parsed.as_str()` once to the opener.
     - Added `ExternalLinkClickOutcome`, `handle_external_link_click`, `handle_navigation_with_policy_and_opener` (routes `mailto:`/`tel:` to `open_external_validated` and returns `false`), `INTERNAL_OPEN_EXTERNAL_IPC_METHOD` (`"internal.openExternal"`), `InternalOpenExternalParams`, and `process_bridge_report_message_with_opener` (gates on `id == 0`, top frame, and authorized bridge origin).
     - Extended `build_init_script` via `build_external_link_click_hook_js` to intercept trusted primary clicks on `a[href], area[href]` (excluding `target="_blank"`, `download`, `#...`) for `mailto:`/`tel:` and cross-origin `http:`/`https:` when `externalLinks == "browser"` and target does not match in-app `navigate` allowlist entries.
   - `src/dev_channel.rs`: Added `external_links: Option<String>` to `DevCommand::ReloadConfig` and `DevLoopAction::ReloadConfig`; updated `test_parse_valid_commands`.
   - `src/bin/wavical_shell_dev.rs`: Added `--external-links` to `DevShellCli` (not added to production `src/main.rs`).
   - `src/lib.rs`: Shared `popup_rate_limiter` across new-window, navigation, and internal IPC handlers; wired `with_navigation_handler` to `handle_navigation_with_policy_and_opener`, `with_ipc_handler` to `process_bridge_report_message_with_opener`, and `ReloadConfig` to update `live_policy.external_links`.
   - `tests/network_modes_tests.rs`: Added `f5_mailto_tel_and_external_links_and_foreign_bridge_isolation` and `f5_open_external_logs_scheme_only_never_address`.

4. **CLI Manifest, Dev Channel & Allowlist Suggestions (`crates/wavical-cli`)**
   - `src/lib.rs`: Updated `summarize_config_changes`, passed `--external-links` to `wavical-shell-dev` in `run_dev_with_mode`, included `"externalLinks"` in `reload-config` JSON, and added `external_links` to `WavicalManifest` in `run_build_with_mode`.
   - `src/allowlist_suggest.rs`: Preserved `open_external_schemes` in `EditableAllowlist` and added `needs_decision` hints for `"mailto:"` and `"tel:"`.
   - `tests/f4_build_modes_tests.rs`: Added `f5_build_manifest_records_external_links_and_allowlist_suggest_preserves_schemes`.

5. **Documentation (`docs/NETWORK_MODES.md`)**
   - Documented `openExternalSchemes`, `External Links (window.externalLinks, mailto:, tel:)`, and `## Differences from a browser` (plain `http://` blocked in `open` unless in `localExceptions`, `window.open` popup OAuth unsupported, and WebView2 tracking prevention on third-party embed storage). Note: `docs/NETWORK_MODES.md` is ignored by `.gitignore` line 37.

### 3. Decisions Made

- **Click hook + navigation handler split for `wry` `0.57.3`:** `wry::WebViewBuilder::with_navigation_handler` only passes `uri: String` (dropping WebView2 `IsUserInitiated()` / `IsRedirected()` flags), and `wavical-shell` uses `#![forbid(unsafe_code)]`. To avoid affecting redirect-based login flows or app-initiated navigation, user-initiated top-level `<a href>` / `<area href>` clicks (`event.isTrusted`) are intercepted in `build_init_script` and validated server-side via `internal.openExternal`, while redirects and programmatic navigations flow through `with_navigation_handler`.
- **Preserve existing Phase 0–4.5 struct/API contracts:** Kept `PolicyContext` struct fields unchanged (avoiding breakage across Phase 4.5 struct literals) and kept existing `decide_navigation` / `resolve_shell_network_policy` / `resolve_shell_network_policy_with_remote` signatures intact.
- **No git commits:** Left all F5 changes uncommitted in the working tree as requested.

### 4. Decisions Needed from You

- **None for F5** unless you want `wry` forked/extended in `wavical-native` later to expose native `IsUserInitiated()`/`IsRedirected()` flags on `NavigationStarting` instead of relying on the trusted JS click hook.
- **Pre-existing `README.md` audit failures (outside F5 scope):** Same 3 `README.md` checks noted in F4 (`FINDING-C5`, `FINDING-C6` in `part4_cli`; `F-A7-06` in `part8_phase3`) continue to fail on `HEAD` (`45fee8d`).

### 5. Other Info (Files, Verification & Manual Checks)

#### Files Changed (Uncommitted in Working Tree)
- `crates/wavical-netpolicy/src/lib.rs`
- `crates/wavical-netpolicy/src/allowlist.rs`
- `crates/wavical-netpolicy/src/decisions.rs`
- `crates/wavical-netpolicy/src/diagnostics.rs`
- `crates/wavical-netpolicy/tests/policy_tests.rs`
- `crates/wavical-config/src/lib.rs`
- `crates/wavical-shell/src/net_policy.rs`
- `crates/wavical-shell/src/dev_channel.rs`
- `crates/wavical-shell/src/bin/wavical_shell_dev.rs`
- `crates/wavical-shell/src/lib.rs`
- `crates/wavical-shell/tests/network_modes_tests.rs`
- `crates/wavical-cli/src/lib.rs`
- `crates/wavical-cli/src/allowlist_suggest.rs`
- `crates/wavical-cli/tests/f4_build_modes_tests.rs`
- `docs/NETWORK_MODES.md` (on disk; ignored by `.gitignore`)
- `PATCH_REPORT.md` (this report)

#### Real Command Output Summary (after final code edit)
1. `cargo build --workspace`: **EXIT=0** (`Finished 'dev' profile in 22.00s`)
2. `cargo build -p wavical-shell --bin wavical-shell-dev --features dev-shell`: **EXIT=0** (`Finished 'dev' profile in 8.23s`)
3. `cargo clippy --workspace -- -D warnings`: **EXIT=0** (`Finished 'dev' profile in 16.10s`)
4. `cargo fmt --all --check`: **EXIT=0**
5. `cargo test --workspace` / `cargo test --workspace --no-fail-fast`: **EXIT=101**
   - `wavical-audit` pre-existing `README.md` failures: `part4_cli` (`16 passed; 2 failed`: `test_config_execution_trust_model_documented`, `test_setup_documentation_declares_nodejs_prerequisite`) and `part8_phase3` (`20 passed; 1 failed`: `test_audit_f_a7_06_readme_model_license_disclaimer`).
   - Under parallel `--workspace --no-fail-fast` load, two `wavical-cli` dev-startup timeout tests timed out at 10s (`f2_dev_shell_tests::test_cli_waits_for_shell_ack_reports_error_and_times_out_after_5s` and `f3_diagnostics_tests::dev_startup_line_and_diagnostic_forwarding_dedup_and_rate_limit`). Both passed cleanly when run directly (`cargo test -p wavical-cli --test f2_dev_shell_tests --test f3_diagnostics_tests -- --nocapture`: `5/5` in `27.59s` and `3/3` in `7.39s`; `cargo test -p wavical-cli --test dev_tests -- --nocapture`: `4/4` in `17.90s`).
   - All other crates and test binaries passed, including `wavical-netpolicy` (`4` unit + `14` `policy_tests`), `wavical-config` (`52` unit), `wavical-shell` (`9/9` `network_modes_tests`, `6` passed / `1` ignored release check, `2/2` `remote_ui_tests`, `3/3` `node_backend_integration_tests`, `3/3` `updater_tests`, `1/1` `wp8_attack_suite`), `wavical-cli` (`9` unit, `10/10` `build_tests`, `5/5` `f4_build_modes_tests`, `1/1` `allowlist_suggest_tests`, etc.), `wavical-ai`, `wavical-ipc` (`107`), `wavical-native` (`41`), `wavical-protocol` (`42`), and `wavical-supply-chain-bench` (`1`).
6. `cargo test -p wavical-audit --no-fail-fast -- --include-ignored`: **EXIT=101**
   - Only `part4_cli` (`16 passed; 2 failed`) and `part8_phase3` (`20 passed; 1 failed`) failed on the pre-existing `README.md` checks; `part1_protocol` (`12/12`), `part2_ipc` (`13/13`), `part3_shell` (`18/18`), `part5*`, `part6*`, `part7_redteam` (`20/20`), and `part9_network_modes` (`5/5`) passed.

#### Manual Checks (Not Run)
- Add `<a href="mailto:hello@example.com">` and `<a href="https://example.com">` to `my-app` and run `wavical dev`: verify clicking `mailto:` opens the OS mail client, clicking `https://example.com` opens the system browser, and `my-app` stays on its page.
- Verify `<a href="tel:...">`, redirect-based OAuth navigation staying in-app, `window: { externalLinks: 'in-app' }` allowing click navigation in-app in `open` mode while keeping `window.wavical`/IPC disabled on foreign origins, and `wavical dev-safe` gating `mailto:`/`tel:` on `openExternalSchemes`.

#### Security Assumptions
- Client-side `internal.openExternal` requests undergo identical server-side policy enforcement (`process_bridge_report_message_with_opener` top-frame/origin check + shared `popup_rate_limiter` + `handle_external_link_click` -> `decide_top_level_navigation` -> `open_external_validated`), preventing arbitrary scheme, oversized URL, or CRLF/header-injection abuse from a compromised renderer.
- Validated URLs are handed to the OS opener as one URL string (never split) and logged by scheme only (`tracing::info!`).
- When `window.externalLinks` is `"in-app"`, `window.wavical` and Rust IPC handlers remain bound to the configured bridge origin and refuse foreign origins.

---

## Patch F6

**Status:** Implemented and verified in working tree (uncommitted; manual GUI visual inspection and clean-tree `npm pack` without `WAVICAL_ALLOW_DIRTY=1` remaining)  
**Baseline Commit:** `0c81ab7` (`1.1.5 - Network mode patch F5`) on `main` (`Cargo.toml` / `package.json` at `1.1.1`)  

### 1. Introduction (What F6 Is)

`wavical@1.1.0` passed repository-level unit/integration tests and still shipped broken on npm. F6 closes that gap by testing what users actually install from the packed `.tgz` tarball across all six network-mode commands (`dev`, `dev-safe`, `dev-ultra-safe`, `build --allow-open`, `build-safe`, `build-ultra-safe`) against the real-world reference app:

- **Reference-app fixture (`tests/fixtures/reference-app/`):** Preserves `test_app/my-app` (`wavical.config.js`, `src/index.html`, `src/style.css`, `src/script.js`) verbatim and adds `wavical.allowlist.json` for `safe` mode.
- **Offline multi-origin test server (`scripts/reference-app-mock-server.js`):** Serves stand-in assets for all six external origins (`cdn.tailwindcss.com`, `cdnjs.cloudflare.com`, `unpkg.com`, `fonts.googleapis.com`, `fonts.gstatic.com`, `www.designarena.ai`) on six distinct local `127.0.0.1:0` HTTPS servers behind a loopback HTTP `CONNECT` proxy (`127.0.0.1:0`) with an ephemeral multi-SAN cert and SPKI pin (zero hosts-file edits).
- **Dev-shell CDP + test-only built-app probe (`crates/wavical-shell`):** Adds validated loopback CDP (`WAVICAL_DEV_REMOTE_DEBUGGING_PORT`) and offline proxy/SPKI pinning to `wavical-shell-dev` (`dev-shell` feature only), and adds a separate `wavical-shell-test-probe` binary (`test-probe` feature, built into `target/test-probe`, never compiled into or shipped with production `wavical-shell.exe`) for probing built `dist/` outputs.
- **Packed-tarball E2E harness & release gating (`scripts/e2e-reference-app.js`, `scripts/verify-package.js`, `.github/workflows/release-canary.yml`):** Installs the packed `.tgz` into a temp copy of the fixture, probes live DOM/Tailwind/GSAP/Lenis/stats/quote-form/hot-reload/F3-diagnostics/0-external-requests across all 6 modes (offline by default in `verify-package.js --tarball`, plus `--online` against real CDNs that fails loudly if unreachable in `release-canary.yml`), and is verified to pass on `wavical-1.1.1.tgz` while failing 7/7 checks on `wavical@1.1.0`.

### 2. What Was Done

1. **Reference App Fixture (`tests/fixtures/reference-app/`)**
   - Copied `wavical.config.js`, `src/index.html`, `src/style.css`, and `src/script.js` verbatim from `test_app/my-app/`.
   - Added `wavical.allowlist.json` (`scripts`: `cdn.tailwindcss.com`, `cdnjs.cloudflare.com`, `unpkg.com`; `styles`: `fonts.googleapis.com`; `fonts`: `fonts.gstatic.com`; `images`: `www.designarena.ai`, `dvxrhkgloakisnvqoxgl.supabase.co` [the observed `302` redirect target of `www.designarena.ai/u/c1aecdca-7fe3-4bb3-a796-046c584cbb35`]; `inlineStyles: true`; `dataImages: true`).

2. **Dev-Shell CDP & Test-Only Built-App Probe (`crates/wavical-shell`)**
   - `Cargo.toml`: Added `test-probe = []` feature (off by default) and `[[bin]] name = "wavical-shell-test-probe"` (`required-features = ["test-probe"]`).
   - `src/lib.rs`: Added `validate_test_proxy_url` (`http://127.0.0.1:<port>` loopback only), `validate_test_spki_pin` (44-char base64 SHA-256 SPKI pin), `build_effective_browser_args` (preserving `ULTRA_SAFE_BROWSER_ARGS` in `ultra-safe` mode), `WindowConfig` test/dev browser-arg fields, `UserEvent::EvalProbe`, and `run_test_probe_with_config`, all strictly gated under `#[cfg(any(feature = "dev-shell", feature = "test-probe"))]` / `#[cfg(feature = "test-probe")]` so production `wavical-shell.exe` is unchanged.
   - `src/bin/wavical_shell_dev.rs`: Added `--remote-debugging-port` (`WAVICAL_DEV_REMOTE_DEBUGGING_PORT`), `--test-proxy` (`WAVICAL_DEV_TEST_PROXY`), and `--test-spki` (`WAVICAL_DEV_TEST_SPKI`).
   - `src/test_probe.rs` & `src/bin/wavical_shell_test_probe.rs` (new): Mirrors production `main.rs` startup (`verify_safe_mode_startup_integrity`, `init_production_file_logging`, `resolve_shell_network_policy_with_remote`, `listen_stdin = false`, `effective_devtools = false` in release) and exposes loopback `GET /health` + `POST /eval` via `webview.evaluate_script_with_callback` with binary marker `WAVICAL_TEST_PROBE_V1_MARKER`.

3. **Offline Multi-Origin Mock Server & Tarball E2E Harness (`scripts/`)**
   - `scripts/reference-app-mock-server.js` (new): Zero-dependency Node.js ASN.1 DER X.509 v3 cert generator + 6 per-origin HTTPS servers (`127.0.0.1:0`) + loopback `CONNECT` proxy (`127.0.0.1:0`) tracking per-origin request counts. Stand-in payloads set `window.__stub_tailwind`, `window.__stub_gsap`, `window.__stub_scrolltrigger`, `window.__stub_lenis`, `--stub-google-fonts: 1` (with minimal valid WOFF font + `Access-Control-Allow-Origin: *`), and a 1x1 PNG.
   - `scripts/e2e-reference-app.js` (new): Zero-dependency RFC 6455 WebSocket CDP client + HTTP `/eval` probe client running all 7 checks against `npm install <tgz>`:
     - `cli`: `npx --no-install wavical --version` matches tarball `package.json` version.
     - `dev` (`open`) + hot reload: all 6 origins load, `#site-nav nav` `display === 'flex'`, GSAP/ScrollTrigger/Lenis present, 5 logo images loaded, stats counters reach `["50+", "30+", "99.9%", "24/7"]` after scroll, quote form submits `#quote-success` without reload, and editing `src/index.html` logs real `Reloaded in <N>ms` (`N >= 1`) and updates the DOM.
     - `dev-safe`, `build --allow-open`, `build-safe`: same full-page DOM/stats/form checks (plus `dist/` shell exclusion and manifest checks).
     - `dev-ultra-safe`, `build-ultra-safe`: `gsap`/`Lenis` undefined, `hardReveal` fallback active, 0 external HTTP hits in offline mode, and F3 blocked-origin diagnostics with hints on `dev-ultra-safe` stderr.
     - `--online`: preflights live CDN reachability (fails loudly if offline) and executes all 7 checks against real CDNs.

4. **Package Verification & Release Workflow (`scripts/verify-package.js`, `scripts/scripts.test.js`, `.github/workflows/release-canary.yml`)**
   - Added `--remote-debugging-port`, `--probe-port-file`, `--test-proxy`, `--test-spki` to `FORBIDDEN_PROD_SHELL_FLAGS` and added `TEST_PROBE_BINARY_MARKERS` (`WAVICAL_TEST_PROBE_V1_MARKER`, `WAVICAL_TEST_PROBE_PORT_FILE`) checked negatively against both `wavical-shell.exe` and `wavical-shell-dev.exe`.
   - Made `verifyTarball` in `scripts/verify-package.js` run `runReferenceAppE2E` as a required step (with `--online` / `--skip-e2e` / `--test-probe-bin` CLI options).
   - Added unit tests in `scripts/scripts.test.js` proving `verifyPackageDir` rejects `--remote-debugging-port` and `WAVICAL_TEST_PROBE_V1_MARKER` in production `wavical-shell.exe`.
   - Updated `.github/workflows/release-canary.yml` to build `wavical-shell-test-probe` (`--target-dir target/test-probe --features test-probe`) and run both `verify-package.js --tarball $tgzName` (offline E2E) and `e2e-reference-app.js --tarball $tgzName --online` (real-network E2E).

### 3. Decisions Made

- **Separate `test-probe` binary + `--target-dir target/test-probe`:** Built `wavical-shell-test-probe` in its own target directory with `--features test-probe` (mirroring F2's `target/dev-shell` isolation) so Cargo feature unification can never leak probe endpoints or test browser flags into production `wavical-shell.exe`.
- **Loopback `CONNECT` proxy + SPKI pin for offline `https://` testing:** Used an ephemeral `127.0.0.1:0` HTTP `CONNECT` proxy routing to 6 per-origin `https` listeners paired with `--ignore-certificate-errors-spki-list=<44-char-base64-sha256>` (validated in Rust, dev/test-probe only, never `--ignore-certificate-errors`) instead of binding privileged port `443`, editing the system `hosts` file, or modifying the Windows cert store.
- **Included `dvxrhkgloakisnvqoxgl.supabase.co` in fixture allowlist:** Online testing caught that `https://www.designarena.ai/u/c1aecdca-7fe3-4bb3-a796-046c584cbb35` returns `302` to `https://dvxrhkgloakisnvqoxgl.supabase.co/...`, which Chromium CSP checks on redirect in `safe` mode.

### 4. Decisions Needed from You

- **None for F6.** (Pre-existing 3 `README.md` audit failures noted in F4/F5 remain untouched outside F6 scope.)

### 5. Other Info (Files, Verification & Manual Checks)

#### Files Changed / Added
- `tests/fixtures/reference-app/wavical.config.js` (new)
- `tests/fixtures/reference-app/wavical.allowlist.json` (new)
- `tests/fixtures/reference-app/src/index.html` (new)
- `tests/fixtures/reference-app/src/style.css` (new)
- `tests/fixtures/reference-app/src/script.js` (new)
- `crates/wavical-shell/Cargo.toml`
- `crates/wavical-shell/src/lib.rs`
- `crates/wavical-shell/src/bin/wavical_shell_dev.rs`
- `crates/wavical-shell/src/test_probe.rs` (new)
- `crates/wavical-shell/src/bin/wavical_shell_test_probe.rs` (new)
- `scripts/reference-app-mock-server.js` (new)
- `scripts/e2e-reference-app.js` (new)
- `scripts/verify-package.js`
- `scripts/scripts.test.js`
- `.github/workflows/release-canary.yml`
- `PATCH_REPORT.md` (this report)

#### Real Command Output Summary
1. `cargo fmt --all --check`: **EXIT=0**
2. `cargo clippy --workspace -- -D warnings` (+ `--bin wavical-shell-dev --features dev-shell` + `--bin wavical-shell-test-probe --features test-probe`): **EXIT=0**
3. `cargo build --workspace`: **EXIT=0**
4. `cargo test --workspace --no-fail-fast`: **EXIT=101** — all workspace crates passed except the 3 pre-existing `README.md` audit failures (`part4_cli`: 2, `part8_phase3`: 1) and `wavical-cli` `dev_tests::test_dev_auto_creates_missing_src_dir` under parallel `target/dev-shell` contention; standalone `cargo test -p wavical-cli --test dev_tests` passed **4/4** (`EXIT=0`).
5. `cargo test -p wavical-audit --no-fail-fast -- --include-ignored`: **EXIT=101** — `part1_protocol` (12/12), `part2_ipc` (13/13), `part3_shell` (18/18), `part5_native_core` (20/20), `part6_native_rest` (20/20), `part7_redteam` (20/20), `part9_network_modes` (5/5) passed; only pre-existing `README.md` checks failed (`part4_cli` 16/18, `part8_phase3` 20/21).
6. `node --test scripts/scripts.test.js`: **EXIT=0** (**8/8** passed).
7. New tarball (`wavical-1.1.1.tgz`, dry-run pack with `WAVICAL_ALLOW_DIRTY=1`):
   - `node scripts/verify-package.js --tarball wavical-1.1.1.tgz` (offline E2E): **EXIT=0** (**7/7** E2E checks passed + version/help/markers/docs/checksums/vs-`1.1.0` checks passed).
   - `node scripts/e2e-reference-app.js --tarball wavical-1.1.1.tgz --online` (real-network E2E): **EXIT=0** (**7/7** E2E checks passed against live CDNs).
8. Broken `wavical@1.1.0` (`npm pack wavical@1.1.0`):
   - `node scripts/verify-package.js --tarball wavical-1.1.0.tgz`: **EXIT=1** (`Missing required binary: ... wavical-shell-dev.exe`).
   - `node scripts/e2e-reference-app.js --tarball wavical-1.1.0.tgz`: **EXIT=1** (**7/7** steps failed: `wavical 0.1.0` version mismatch, `dev` startup timeout, `dev-safe`/`dev-ultra-safe`/`build-safe`/`build-ultra-safe` unrecognized subcommands, `build --allow-open` unexpected argument).

#### Manual Checks (Not Run)
- Visually inspect the rendered WebView2 GUI window of `tests/fixtures/reference-app` in `open`/`safe` vs `ultra-safe` (fonts, lotus intro animation, particle constellation, scroll parallax, logo rendering).
- Commit F6 changes and run a clean-tree `npm pack` + `node scripts/verify-package.js --tarball wavical-1.1.1.tgz` without `WAVICAL_ALLOW_DIRTY=1`.

#### Security Assumptions
- Production `wavical-shell.exe` never compiles `dev-shell` or `test-probe` features, and `verify-package.js` enforces negative marker and CLI flag scans on `wavical-shell.exe`.
- `WAVICAL_DEV_TEST_PROXY` / `WAVICAL_TEST_PROBE_PROXY` accept only `http://127.0.0.1:<port>`, and `WAVICAL_DEV_TEST_SPKI` / `WAVICAL_TEST_PROBE_SPKI` accept only a 44-character base64 SHA-256 SPKI pin (never `--ignore-certificate-errors`).

---

<!-- Future patches: copy this section structure for Patch F7, ... -->
<!--
## Patch F6

**Status:** ...
**Baseline Commit:** ...

### 1. Introduction (What F6 Is)
### 2. What Was Done
### 3. Decisions Made
### 4. Decisions Needed from You
### 5. Other Info (Files, Verification & Manual Checks)
-->
