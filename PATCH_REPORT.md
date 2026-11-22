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

<!-- Future patches: copy this section structure for Patch F5, F6, ... -->
<!--
## Patch F5

**Status:** ...
**Baseline Commit:** ...

### 1. Introduction (What F5 Is)
### 2. What Was Done
### 3. Decisions Made
### 4. Decisions Needed from You
### 5. Other Info (Files, Verification & Manual Checks)
-->
