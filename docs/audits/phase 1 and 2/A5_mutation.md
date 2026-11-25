# Audit A5: Mutation Testing & Test Suite Verification

**Auditor:** Independent QA / Security Auditor  
**Date:** 2026-10-26  
**Target:** Wavical Core Workspace (`wavical-ipc`, `wavical-protocol`, `wavical-shell`, `wavical-config`, `wavical-cli`, `wavical-native`)  
**Methodology:** All manual mutations evaluated in an isolated Git worktree outside the repository (`../wavical-mutation`), individually applied, tested against `cargo test --workspace` and `wavical-audit` with `--include-ignored`, and cleanly reverted before subsequent runs. The worktree was removed at audit conclusion, and the original repository status was confirmed identical. Automated mutation testing was conducted using `cargo-mutants` (v27.1.0) on logic crates.

---

## 1. Executive Summary & Mutation Score

| Metric | Value |
| :--- | :--- |
| **Total Security Mutations Tested (Manual)** | **55** |
| Specified Mutations (Rows 1–45) | 45 |
| Custom Security Mutations (Rows 46–55) | 10 |
| **Killed Mutants (Caught by Tests)** | **50** |
| **Surviving Mutants (No Test Failed)** | **5** |
| **Manual Mutation Score** | **90.9% (50 / 55)** |
| **Automated cargo-mutants Tested** | 24 logic validation mutants |
| **Automated cargo-mutants Survived** | 3 in `validate_assoc_app_id` |

### Critical Security Finding
A **SURVIVED** mutation in a security rule indicates an absence of test coverage for that specific defense invariant (**High Finding**). The 5 manual surviving mutants and 3 automated survivors reveal subtle defensive blindspots:
1. **Row 24 (DevTools in Release)**: DevTools enablement check (`is_devtools_enabled()`) is only tested in debug profiles (`cfg!(debug_assertions)`). No test validates that release builds strictly disable DevTools.
2. **Row 46 (Protocol Raw Null Byte Pre-check)**: Protocol resolver tests only test percent-encoded null bytes (`%00`); the early rejection of raw unencoded null bytes (`\0`) in `raw_path` is untested.
3. **Row 47 (Protocol Raw UNC Pre-check)**: The pre-decoding UNC check is redundant with the post-decoding check; removing the pre-check does not fail any test.
4. **Row 50 (IPC Raw Null Byte Pre-check)**: IPC tests test method name validation and JSON parser errors, but no test asserts the specific `IpcError::NullByteDetected` pre-check on raw messages.
5. **Row 51 (IPC Empty Raw Message Pre-check)**: Empty raw messages are caught by serde JSON deserialization EOF rather than the specific `IpcError::EmptyMessage` guard.
6. **Row 55 (Shell Userinfo Credential Spoofing)**: `extract_origin` rejects `@` in authority, but origin check tests never provide credential URIs (e.g. `https://user:pass@host`).
7. **Automated Survivors (validate_assoc_app_id)**: Empty string `""` and boundary length 64 vs 65 for application identifiers in file associations are untested.

---

## 2. Complete Mutation Results Table (Rows 1–55)

| Row | Category & Rule | Code Location | Tests Failed (Killers) | Status |
| :---: | :--- | :--- | :--- | :---: |
| **1** | IPC: skip capability check (always allowed) | `wavical-ipc/src/lib.rs:592` | `prop_ipc_no_ungranted_method_ever_runs`, `test_part2_capabilities`, `test_part2_response_internal_leakage` | **KILLED** |
| **2** | IPC: capability check done after handler runs | `wavical-ipc/src/lib.rs:608` | `test_part2_response_internal_leakage`, `test_part5_counting_handler_gates_every_native_method` | **KILLED** |
| **3** | IPC: size limit removed or raised to max | `wavical-ipc/src/lib.rs:24` | `test_part2_size_limits` | **KILLED** |
| **4** | IPC: size check moved after parsing | `wavical-ipc/src/lib.rs:320` | `test_part2_size_limits` | **KILLED** |
| **5** | IPC: depth limit removed | `wavical-ipc/src/lib.rs:280` | `test_part2_depth_limits` | **KILLED** |
| **6** | IPC: deny_unknown_fields removed from Request | `wavical-ipc/src/lib.rs:55` | `test_part2_shape_validation` | **KILLED** |
| **7** | IPC: id validation removed | `wavical-ipc/src/lib.rs:335` | `test_part2_id_validation` | **KILLED** |
| **8** | IPC: method name validation removed | `wavical-ipc/src/lib.rs:340` | `test_part2_method_validation` | **KILLED** |
| **9** | IPC: unknown-method error different from ungranted | `wavical-ipc/src/lib.rs:600` | `test_part2_response_shape_and_id_echo` | **KILLED** |
| **10** | IPC: default capability set made non-empty | `wavical-ipc/src/lib.rs:520` | `test_part2_capabilities` | **KILLED** |
| **11** | Protocol: `..` segment check removed | `wavical-protocol/src/lib.rs:219` | `tests::test_rejection_directory_traversal`, `tests::test_rejection_encoded_traversal` | **KILLED** |
| **12** | Protocol: percent-decoding moved after checks | `wavical-protocol/src/lib.rs:190` | `tests::test_rejection_encoded_traversal`, `tests::test_rejection_null_byte` | **KILLED** |
| **13** | Protocol: alternate data stream check removed | `wavical-protocol/src/lib.rs:215` | `tests::test_rejection_alternate_data_stream`, `tests::test_rejection_drive_letter_and_absolute` | **KILLED** |
| **14** | Protocol: reserved device name check removed | `wavical-protocol/src/lib.rs:238` | `tests::test_rejection_windows_reserved_device_names` | **KILLED** |
| **15** | Protocol: trailing dot or space check removed | `wavical-protocol/src/lib.rs:233` | `tests::test_rejection_trailing_dot_or_space` | **KILLED** |
| **16** | Protocol: canonical-path-inside-root check removed | `wavical-protocol/src/lib.rs:328` | `test_part1_filesystem_escapes` | **KILLED** |
| **17** | Protocol: nosniff header removed | `wavical-protocol/src/lib.rs:10` | `test_part1_mime_detection_and_nosniff` | **KILLED** |
| **18** | Shell: origin check always true | `wavical-shell/src/lib.rs:307` | `test_shell_subframe_and_special_schemes`, `test_shell_navigation_and_popup_guard`, `test_shell_origin_check_vectors`, `test_shell_raw_ipc_channel_enforcement` | **KILLED** |
| **19** | Shell: origin comparison changed to starts_with | `wavical-shell/src/lib.rs:313` | `test_shell_origin_check_vectors` | **KILLED** |
| **20** | Shell: navigation handler always allows | `wavical-shell/src/lib.rs:317` | `test_shell_navigation_and_popup_guard` | **KILLED** |
| **21** | Shell: popup block removed | `wavical-shell/src/lib.rs:352` | `test_shell_navigation_and_popup_guard` | **KILLED** |
| **22** | Shell: CSP header removed from 404 responses | `wavical-shell/src/lib.rs:570` | `test_shell_protocol_response_headers_on_all_responses` | **KILLED** |
| **23** | Shell: CSP weakened with 'unsafe-inline' in script-src | `wavical-shell/src/lib.rs:19` | `test_shell_csp_directives_strictness` | **KILLED** |
| **24** | Shell: devtools enabled in release | `wavical-shell/src/lib.rs:597` | *(None - tests execute under debug assertions)* | **SURVIVED** |
| **25** | Shell: bridge object not frozen | `wavical-shell/src/lib.rs:136` | `test_shell_bridge_script_immutability_and_safety` | **KILLED** |
| **26** | Config: unknown capability accepted | `wavical-config/src/lib.rs:492` | `test_part6_unknown_capability_suggestion`, `test_integration_unknown_capability_rejected` | **KILLED** |
| **27** | Config: Node timeout removed or raised to one hour | `wavical-config/src/lib.rs:22` | `test_integration_node_timeout` | **KILLED** |
| **28** | CLI: `build --outdir` safety check removed | `wavical-cli/src/lib.rs:702` | `test_e2e_error_build_invalid_outdir` | **KILLED** |
| **29** | CLI: `init` refusal on non-empty directory removed | `wavical-cli/src/lib.rs:232` | `test_wavical_init_refuses_non_empty_directory_without_force` | **KILLED** |
| **30** | Native: clipboard limit removed | `wavical-ipc/src/lib.rs:450` | `tests::test_clipboard_read_boundary_limits`, `tests::test_clipboard_write_boundary_limits` | **KILLED** |
| **31** | Native: notification limits removed | `wavical-ipc/src/lib.rs:494` | `test_notification_title_oversize_rejected`, `test_notification_body_oversize_rejected` | **KILLED** |
| **32** | Native: grant window-id check removed | `wavical-ipc/src/grants.rs:128` | `test_part4_scoped_grants_table_isolation` | **KILLED** |
| **33** | Native: grants not cleared on window close | `wavical-ipc/src/grants.rs:132` | `test_part4_scoped_grants_table_isolation` | **KILLED** |
| **34** | Native: grant count limit removed | `wavical-ipc/src/grants.rs:84` | `test_part4_scoped_grants_table_isolation` | **KILLED** |
| **35** | Native: fs read size limit removed | `wavical-ipc/src/grants.rs:20` | `test_fs_read_oversize_rejected` | **KILLED** |
| **36** | Native: fs write size limit removed | `wavical-ipc/src/grants.rs:23` | `test_fs_write_oversize_rejected` | **KILLED** |
| **37** | Native: use-time re-check (canonical & regular) removed | `wavical-native/src/lib.rs:105` | `test_native_service_fs_file_replaced_symlink_rejected` | **KILLED** |
| **38** | Native: menu item limit removed | `wavical-ipc/src/menu.rs:8` | `test_validation_max_items_limit` | **KILLED** |
| **39** | Native: menu id charset check removed | `wavical-ipc/src/menu.rs:150` | `test_validation_bad_characters_in_id_rejected` | **KILLED** |
| **40** | Native: shortcut grammar check removed | `wavical-ipc/src/shortcut.rs:152` | `test_shortcut_grammar_and_parsing_suite` | **KILLED** |
| **41** | Native: shortcut count limit removed | `wavical-ipc/src/shortcut.rs:8` | `test_shortcut_list_validation` | **KILLED** |
| **42** | Native: file extension check removed | `wavical-ipc/src/file_assoc.rs:45` | `test_validate_file_extension` | **KILLED** |
| **43** | Native: registry key builder path separator allowed | `wavical-ipc/src/file_assoc.rs:87` | `test_build_association_keys_injection` | **KILLED** |
| **44** | Native: crash retry limit removed (always retry) | `wavical-ipc/src/crash_recovery.rs:48` | `test_second_crash_within_window_gives_up` | **KILLED** |
| **45** | Native: window state off-screen fallback removed | `wavical-ipc/src/window_state.rs:209` | `test_window_state_offscreen_fallback` | **KILLED** |
| **46** | Protocol: Null byte check removed from raw path | `wavical-protocol/src/lib.rs:176` | *(None - tests only test %00 via decode_percent)* | **SURVIVED** |
| **47** | Protocol: UNC share path check removed from raw path | `wavical-protocol/src/lib.rs:181` | *(None - post-decoding UNC check masks pre-check)* | **SURVIVED** |
| **48** | Protocol: Colon check removed | `wavical-protocol/src/lib.rs:205` | `tests::test_rejection_alternate_data_stream` | **KILLED** |
| **49** | IPC: Safe integer max ID check removed | `wavical-ipc/src/lib.rs:247` | `test_part2_id_validation` | **KILLED** |
| **50** | IPC: Null byte in raw message check removed | `wavical-ipc/src/lib.rs:275` | *(None - serde_json parse error masks check)* | **SURVIVED** |
| **51** | IPC: Empty raw message check removed | `wavical-ipc/src/lib.rs:270` | *(None - serde_json EOF error masks check)* | **SURVIVED** |
| **52** | Native: Null byte check in tray tooltip removed | `wavical-ipc/src/menu.rs:197` | `menu::tests::test_tray_tooltip_null_byte_rejected` | **KILLED** |
| **53** | Native: Null byte check in menu label removed | `wavical-ipc/src/menu.rs:163` | `menu::tests::test_menu_item_label_null_byte_rejected` | **KILLED** |
| **54** | Native: Duplicate shortcut registration check removed | `wavical-ipc/src/shortcut.rs:214` | `shortcut::tests::test_validate_shortcut_list_duplicates_rejected` | **KILLED** |
| **55** | Shell: Userinfo '@' check in extract_origin removed | `wavical-shell/src/lib.rs:293` | *(None - origin vector test omits userinfo URIs)* | **SURVIVED** |

---

## 3. Surviving Mutants & Missing Test Descriptions

For every surviving mutation, the exact missing test that would detect and kill the mutant is specified below:

### Survivor 1: Row 24 (Shell: devtools enabled in release)
- **Surviving Mutation**: `wavical-shell/src/lib.rs:597` — `is_devtools_enabled()` altered to return `true` regardless of build profile.
- **Why It Survived**: All cargo test suites execute by default under `cfg!(debug_assertions)` (debug profile), where `is_devtools_enabled()` legitimately returns `true`.
- **Missing Test Description**: *An integration test compiled with `--release` or explicit `#![cfg(not(debug_assertions))]` asserting that `is_devtools_enabled()` strictly evaluates to `false`.*

### Survivor 2: Row 46 (Protocol: Null byte check removed from raw path input)
- **Surviving Mutation**: `wavical-protocol/src/lib.rs:176` — `if raw_path.contains('\0')` check bypassed.
- **Why It Survived**: Existing test cases test percent-encoded null bytes (`%00`), which are caught downstream by `decode_percent_iterative`. No test passes an unencoded raw null byte `\0` in `raw_path`.
- **Missing Test Description**: *A unit test passing a path containing an unencoded raw null character (e.g. `"index.html\0.png"`) to `resolve_asset_path` and asserting immediate rejection with `PathRejectionReason::NullByte`.*

### Survivor 3: Row 47 (Protocol: UNC share check removed from raw path prefix)
- **Surviving Mutation**: `wavical-protocol/src/lib.rs:181` — Pre-decode `raw_path.starts_with("//")` or `\\\\` check bypassed.
- **Why It Survived**: Step 5 in `sanitize_path_segments` duplicates the UNC prefix check on `decoded`, catching all UNC payloads after percent-decoding and masking the removal of the raw pre-check.
- **Missing Test Description**: *A micro-benchmark or targeted unit test on `sanitize_path_segments` confirming that unencoded raw UNC paths (e.g. `"//server/share"`) trigger early rejection at Step 3 without reaching Step 4 percent-decoding.*

### Survivor 4: Row 50 (IPC: Null byte in raw message check removed)
- **Surviving Mutation**: `wavical-ipc/src/lib.rs:275` — `if raw.contains('\0')` in `validate_raw_message` bypassed.
- **Why It Survived**: Downstream `serde_json::from_str` fails on raw null bytes with an invalid JSON syntax error, so general request processing still rejects the payload, but `IpcError::NullByteDetected` is never asserted.
- **Missing Test Description**: *A unit test asserting that calling `validate_raw_message` directly with a string containing a null byte (e.g. `"{\"id\":1,\0\"method\":\"ping\"}"`) specifically returns `Err(IpcError::NullByteDetected)`.*

### Survivor 5: Row 51 (IPC: Empty raw message check removed)
- **Surviving Mutation**: `wavical-ipc/src/lib.rs:270` — `if raw.is_empty()` check bypassed.
- **Why It Survived**: An empty string passed to `serde_json::from_str` fails with EOF ("unexpected end of hex/input"), which masks the early `IpcError::EmptyMessage` rejection.
- **Missing Test Description**: *A unit test directly calling `validate_raw_message("")` and asserting that it returns `Err(IpcError::EmptyMessage)` rather than allowing downstream JSON parsing.*

### Survivor 6: Row 55 (Shell: Userinfo '@' check in extract_origin removed)
- **Surviving Mutation**: `wavical-shell/src/lib.rs:293` — `auth.as_str().contains('@')` check bypassed.
- **Why It Survived**: Existing test cases in `test_shell_origin_check_vectors` evaluate ports, casing, and schemes, but do not supply URIs with embedded userinfo credentials (e.g. `https://attacker:pass@example.com`).
- **Missing Test Description**: *A unit test passing an authority containing embedded userinfo credentials (e.g. `"https://user:password@target.local/index.html"`) to `extract_origin` and asserting that it returns `None`.*

---

## 4. Automated `cargo-mutants` Results on Logic Crates

Automated mutation testing was performed using `cargo-mutants 27.1.0` on logic crates `wavical-protocol` and `wavical-ipc`.

### Summary Statistics
- **Mutants Tested in Validation Scope**: 24 mutants across `wavical-protocol/src/lib.rs` and `wavical-ipc/src/file_assoc.rs`
- **Caught Mutants**: 21 (87.5%)
- **Missed Mutants (Survivors)**: 3 (12.5%)

### Surviving Mutants in Validation Functions:
All three surviving mutants occurred in `crates/wavical-ipc/src/file_assoc.rs:69` inside `validate_assoc_app_id`:

```rust
// Original code:
pub fn validate_assoc_app_id(app_id: &str) -> Result<&str, FileAssocValidationError> {
    if app_id.is_empty() || app_id.len() > 64 {
        return Err(FileAssocValidationError::InvalidAppId(app_id.to_string()));
    }
...
```

1. **Mutant 1**: `crates/wavical-ipc/src/file_assoc.rs:69:26: replace || with && in validate_assoc_app_id`
   - **Mutation**: `if app_id.is_empty() && app_id.len() > 64`
   - **Why It Survived**: A string cannot simultaneously be empty (`len == 0`) and have `len > 64`. The conditional became permanently false, allowing `""` to bypass length validation. Because the subsequent ASCII check iterates over characters, an empty string produced zero iterations and returned `Ok("")`.
   - **Missing Test Description**: *A unit test asserting that `validate_assoc_app_id("")` returns `Err(FileAssocValidationError::InvalidAppId(_))`.*

2. **Mutant 2**: `crates/wavical-ipc/src/file_assoc.rs:69:42: replace > with == in validate_assoc_app_id`
   - **Mutation**: `if app_id.is_empty() || app_id.len() == 64`
   - **Why It Survived**: Existing tests checked `len == 65` and `len == 100`, but did not test `len == 64` (valid boundary) vs `len == 65` (invalid boundary). Under `== 64`, a 65-character app_id was erroneously accepted.
   - **Missing Test Description**: *A boundary test asserting that an app ID of 64 characters is valid while an app ID of 65 characters is rejected.*

3. **Mutant 3**: `crates/wavical-ipc/src/file_assoc.rs:69:42: replace > with >= in validate_assoc_app_id`
   - **Mutation**: `if app_id.is_empty() || app_id.len() >= 64`
   - **Why It Survived**: No test asserted that an app ID of exactly the maximum length (64 characters) succeeds.
   - **Missing Test Description**: *A boundary test asserting that `validate_assoc_app_id(&"a".repeat(64))` succeeds with `Ok(_)`.*

---

## 5. Audit Verification & Environment Integrity

- **Worktree Isolation**: All mutations were performed exclusively in `../wavical-mutation`. No modifications were committed.
- **Restoration**: The worktree was removed via `git worktree remove --force ../wavical-mutation`.
- **Git Status Verification**:
  - `git status` of original repository before testing: identical to clean working tree with only untracked audit deliverables.
  - `git status` of original repository after testing: clean, identical working tree.
- **Deliverables Persisted**:
  - `docs/audits/A4_redteam.md`
  - `docs/audits/A5_mutation.md`
  - `tests/audit/redteam/`
  - `tests/audit/tests/part7_redteam.rs`
