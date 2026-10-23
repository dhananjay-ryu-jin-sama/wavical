# A0: Baseline, CI and Drift Audit

Auditor: independent (did not write the code). Date: 2026-10-23. Mode: audit only, no source changes.
Commit audited: `main` at `0a31f0f` (clean tree). Machine: Windows NT 10.0.26100.

## Summary

| Status | Count |
|---|---|
| PASS | 14 |
| FAIL | 4 |
| PARTIAL | 8 |
| NOT RUN | 3 |

Findings: 0 Critical, 1 High, 5 Medium, 6 Low, 3 Info.

Top results: `cargo deny check` fails (exit 5). Unapproved `unsafe` in `crash_recovery.rs`. The first `cargo test --workspace` run failed (exit 101) on a flaky dev test and leaked two processes. CI has no `permissions:`, no pinned SHAs, no fuzz job, no audit-crate job. Linux CI probably cannot pass `wavical-cli` dev tests.

## Step 1: Environment (PASS)
```
rustc 1.99.0 (b940084d7 2026-09-28)
cargo 1.99.0 (5f94df478 2026-08-27)
node v26.4.0
git version 2.54.0.windows.1
Microsoft Windows NT 10.0.26100.0
cargo subcommands: audit, deny, clippy, fmt, nextest present; mutants, fuzz ABSENT
```
- `cargo mutants`: NOT RUN. Install: `cargo install cargo-mutants --locked`
- `cargo fuzz`: NOT RUN. Needs nightly, run in WSL: `cargo install cargo-fuzz --locked`, then `cd fuzz; cargo +nightly fuzz run parse_message -- -max_total_time=60`
- `gh`: not installed (`gh not installed`). `gh run list --limit 10`: NOT RUN. Please paste your latest CI result.

## Step 2: Git state
- `git status --short`: empty. `git branch -a`: `* main`, `remotes/origin/main`. 26 commits, 78 tracked files, none over 200 KB.
- Secrets: `git grep -i -E "api_key|secret|token|password|BEGIN PRIVATE KEY"` hits are only test fixtures, doc text, parser variable names. `git log --all -S"BEGIN PRIVATE KEY"` and `git grep -a "BEGIN PRIVATE KEY" HEAD`: no output. History hits for `api_key|password` (3cdcbc2, 1d0bb75) are one doc comment and `'api.getSecret', { name: 'API_KEY' }` in the schema doc. No `.env/.pem/.key/.pfx/.p12` ever tracked. PASS.
- Commit/message mismatches (PARTIAL):
  - `53e0bd1 0.2.0`, `71e5b5a 0.2.3`, `d102c8a 0.2.4`, `f1a14ac 0.2.5`: message is only a version number.
  - `d2a1365 0.2.71` sits after `11bee1f 0.2.7`.
  - `0a31f0f 0.2.9` says "implement core shell, IPC, fuzz targets, integration tests, and CI workflow" but changes 15 files, 573 insertions. `d2126a3 0.2.8` ("add IPC, native modules, and CLI tests") changes 30 files, 2405 insertions. Messages do not describe the content.
  - `21ac1d7` deleted the Phase 0 plan; it exists again at HEAD.

## Step 3: Build, test, lint (outputs in session scratch logs)
| Command | Exit | Result |
|---|---|---|
| `cargo build --workspace` | 0 | PASS |
| `cargo build --workspace --release` | 0 | PASS |
| `cargo test --workspace` (first run) | 101 | FAIL: `test_dev_auto_creates_missing_src_dir` panicked at `crates/wavical-cli/tests/dev_tests.rs:263:5`: "dev server should start even if src/ was missing". Stopped at first failing binary, later crates did not run. |
| `cargo test --workspace --no-fail-fast` (rerun) | 0 | 213 passed, 0 failed |
| `cargo test -p wavical-cli --test dev_tests test_dev_auto_creates_missing_src_dir` | 0 | passed in 14.42s (the test waits at most 10s for startup) |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | PASS |
| `cargo fmt --all --check` | 0 | PASS (no output) |
| `cargo audit` | 0 | 0 vulnerabilities, 6 allowed warnings (see below) |
| `cargo deny check` | 5 | FAIL: `advisories FAILED, bans ok, licenses FAILED, sources ok` |

Test counts per binary (rerun):
| Crate | Binary | Tests |
|---|---|---|
| wavical-cli | lib 8, build_tests 7, cli_tests 4, dev_tests 4, e2e_tests 8, bins 0 | 31 |
| wavical-config | lib | 24 |
| wavical-ipc | lib | 82 |
| wavical-native | lib | 38 |
| wavical-protocol | lib | 15 |
| wavical-shell | lib 23, main 0 | 23 |
| wavical-integration-tests | main | 0 |
| doc-tests (all) | | 0 |
| Total | | 213 |

Test-run side effect: the failed test left `wavical.exe` and `wavical-shell.exe` running (started 14:33). They held the output pipe and hung the suite until I stopped those two PIDs. The test only kills them after its asserts (dev_tests.rs:269-275), so any failure leaks processes.

`cargo audit` warnings: RUSTSEC-2024-0370 (proc-macro-error, unmaintained, ignored in deny.toml:14), RUSTSEC-2026-0370/0371/0372 (zvariant, zbus_names, zvariant_derive, unmaintained), RUSTSEC-2024-0429 (glib 0.18.5, unsound), clipboard-win 5.4.1 (yanked).

`cargo deny check` errors:
- `licenses`: `clipboard-win 5.4.1` and `error-code 3.4.0` are `BSL-1.0`, not in the allow list (deny.toml:20-33). Path: arboard -> wavical-native.
- `advisories`: `zbus_names`, `zvariant`, `zvariant_derive` unmaintained (not in the ignore list, deny.toml:12-15).
- Warnings: 26 duplicate-version groups (bitflags, getrandom, syn, thiserror, windows-sys x4, toml_edit x3 and others), one yanked crate.
- No deprecation warnings or config-key errors were printed (`Select-String deprecat` on the deny log returned nothing). The old keys (`vulnerability`, `notice`, `unlicensed`, `copyleft`) are not used in deny.toml.

## Step 4: Layout versus plans
Plan section 3 (Phase 0), Phase 1 layout, Phase 2 table 0.1.
- Present as planned: `crates/wavical-{ipc,protocol,shell,cli,config,native}`, `examples/hello`, `tests/attack-page`, `tests/integration`, `fuzz/`, `.github/workflows/ci.yml`, `ROADMAP.md`, `SECURITY.md`, `docs/`.
- Missing: `tests/cli/` (Phase 1 layout, CLI tests live in `crates/wavical-cli/tests/` instead); `wavical-core` (named in Phase 0 section 2 safety rule, never in the layout; doc inconsistency); `WAVICAL_*_ANTIGRAVITY_PROMPTS.md` (referenced by the index, not in repo); `tests/audit/` and `docs/audits/` did not exist before this audit.
- Extra: `project_rules.md`, `README.md` (root); `examples/hello/{app.js,index.html,style.css,icon.png}` duplicated at root and under `examples/hello/src/`; `crates/wavical-config/examples/manual_test.rs`; `crates/wavical-protocol/proptest-regressions/lib.txt` (expected).
- `fuzz/` is excluded from the workspace (Cargo.toml `exclude`), consistent with the plan (Linux/WSL).
- Command names: `where.exe wavical` and `where.exe wca`: "Could not find files" (checklist claim holds on this machine; WSL not checked).

## Step 5: Purity
- `cargo tree -p wavical-ipc -e normal`: 21 unique crates (serde, serde_json, thiserror, tracing, getrandom and their support crates). No banned name. PASS.
- `cargo tree -p wavical-protocol -e normal`: 12 crates (thiserror, tracing and support). No banned name. PASS.
- `-e dev` alone shows only `proptest` (and `tempfile` for protocol), because `-e dev` hides transitive normal edges. With `-e normal,dev`: both crates pull `windows-sys` (v0.61.2) via `proptest -> tempfile -> windows-sys` and `rusty-fork -> tempfile`. Literal rule says FAIL for any depth. It is target-conditional and dev-only, not linked into shipped binaries; Low.
- Workspace dependency direction (normal edges):
  - wavical-ipc -> (none); wavical-protocol -> (none)
  - wavical-config -> ipc; wavical-cli -> config, ipc
  - wavical-native -> ipc, protocol
  - wavical-shell -> ipc, native, protocol
  - wavical-integration-tests -> ipc, native, protocol, shell
  - No cycles. No arrow from a pure crate (ipc, protocol) to a non-pure one. config and cli (pure-ish) do not depend on tao/wry; cli depends on `windows-registry` (Windows only).

## Step 6: unsafe
| Location | Hit | Approved in WAVICAL_PHASE_2_DECISIONS.md? |
|---|---|---|
| `crates/wavical-native/src/lib.rs:1` | `#![deny(unsafe_code)]` | Plan 0.1 says `forbid`; decisions line 162 documents `deny` plus one scoped allow. Info. |
| `crates/wavical-native/src/shortcut.rs:1` | `#![deny(unsafe_code)]` | same |
| `crates/wavical-native/src/tray.rs:56,59` | `#[allow(unsafe_code)]`, `unsafe { menu.show_context_menu_for_hwnd(..) }` | Yes (decisions 3.2, line 150). |
| `crates/wavical-native/src/crash_recovery.rs:36,51,62` | `#[allow(unsafe_code)]`, two `unsafe` blocks (`ProcessFailedKind`, `add_ProcessFailed`) | NO. Table row 21 says unsafe is needed but no stop-and-ask or approval is recorded; 3.3 describes a pure state machine; section 4 says "exactly one scoped allow" (muda). Contradiction. |
- `forbid(unsafe_code)` at crate root: present in wavical-ipc, protocol, shell (lib+main), config, cli (lib + both bins), integration main. NOT present: wavical-native (deny), fuzz targets, `wavical-config/examples/manual_test.rs`. `wavical-audit` does not exist yet: N/A.

## Step 7: Panic-prone code in non-test code
Scope: everything before the first `#[cfg(test)]` in each file, plus native `lib.rs` lines 27-594 (a second cfg(test) item at line 26 hides them from a naive scan; rescanned).
| File:line | Hit | External input? | Verdict |
|---|---|---|---|
| `wavical-ipc/src/grants.rs:80,122,131,137,143,149` | `RwLock .unwrap()` | Indirect: panics only if the lock is poisoned by an earlier panic | Finding, Low (F-A0-07) |
| `wavical-native/src/single_instance.rs:78` | `.expect("Failed to spawn ... thread")` | OS resource failure | Finding, Low |
| `wavical-cli/src/lib.rs:443-903` many | `println!`/`eprintln!` | no | Justified (CLI output) |
| `wavical-shell/src/lib.rs:481` | `eprintln!` | no (native message path) | Justified |
| `tests/integration/src/main.rs` | println/eprintln | no | Justified (test harness) |
| `wavical-config/src/lib.rs:434-442` | `&msg[start + len..]`, `remainder[..end]` | serde error text | Justified (indices from `find`) |
| `wavical-ipc/src/shortcut.rs:153,159,164` | `parts[0]`, `parts[len-1]` | page | Justified (`parts.len() >= 2` checked at :152) |
| `wavical-ipc/src/badge.rs:51` | `count as u32` | page | Justified (range checked 0..=99 first) |
| `wavical-ipc/src/window_state.rs:47-50,134,139` | `as i32`, `as u32` | file | Justified (`saturating_add`, range-checked 100..=32768) |
| `wavical-native/src/single_instance.rs:56,59,130` | `len as usize`, `vec![0; len]`, `payload.len() as u32` | local process | Justified (len <= 1 MB checked before alloc); send side not capped |
| `wavical-shell/src/lib.rs:279` | `&payload[..end]` | page | Justified (UTF-8 boundary logic, unit-tested `test_truncate_payload`) |
| `wavical-shell/src/lib.rs:1004` | `window.hwnd() as isize` | OS | Justified |
| `wavical-cli/src/sha256.rs:37-96` | fixed-size array indexing | file contents, but fixed block size | Justified |
No `todo!`, `unimplemented!`, `unreachable!`, `dbg!`, `TODO`, `FIXME`, `HACK` in non-test code. No `.unwrap()`/`.expect()` on page, config or file input found.

## Step 8: Dependencies (versions from Cargo.lock via `cargo metadata --locked`)
| Crate | Direct deps (lock version) | In decisions/docs? |
|---|---|---|
| wavical-ipc | serde 1.0.229, serde_json 1.0.151, thiserror 2.0.21, tracing 0.1.44, getrandom 0.3.4; dev proptest 1.11.0 | all but getrandom documented (needed for 0.3 random grant ids, undocumented) |
| wavical-protocol | thiserror, tracing; dev proptest, tempfile 3.27.0 | yes (setup doc) |
| wavical-config | serde, serde_json, thiserror, tracing, wavical-ipc; dev proptest, tempfile | yes |
| wavical-cli | clap 4.6.7, ctrlc 3.5.2, notify 6.1.1, serde, serde_json, thiserror, tracing, tracing-subscriber 0.3.23, windows-registry 0.100.0 (cfg windows); dev tempfile | clap/notify named in Phase 1 plan; ctrlc and windows-registry not in any decision doc |
| wavical-native | arboard 3.7.0, notify-rust 4.18.2, rfd 0.17.2, interprocess 2.4.6, global-hotkey 0.9.0, tray-icon 0.26.1, muda 0.21.2, wry 0.57.0, webview2-com 0.39.1 (cfg windows), serde, serde_json, thiserror, tracing, ipc, protocol | versions match decisions (manifest says `interprocess = "2.2"`, lock 2.4.6; decisions 2.4.6); webview2-com 0.39.1 vs doc 0.39.0 |
| wavical-shell | tao 0.37.1, wry 0.57.0, muda 0.21.2, tray-icon 0.26.1, global-hotkey 0.9.0, clap 4.6.7, serde, serde_json, thiserror, tracing, tracing-subscriber, ipc, native, protocol | plan 0.1 lists only tao, wry, wavical-native for the shell; muda, tray-icon, global-hotkey direct in shell are not justified |
| wavical-integration-tests | tao, wry, serde, serde_json, tracing, tracing-subscriber, ipc, native, protocol, shell | plan |
- `cargo tree -d` duplicates (notable): getrandom 0.3.4 and 0.4.3, syn 2/3, miniz_oxide 0.8.9/0.9.1, windows-sys x3 (0.48, 0.59, 0.61), windows-link/result/strings x2. Mostly platform crates via GUI stack.
- `Cargo.lock` is tracked (`git ls-files Cargo.lock` prints it) and `.gitignore` does not ignore it. PASS.

## Step 9: CI review (`.github/workflows/ci.yml`, the only workflow)
| Job | OS | Commands |
|---|---|---|
| windows | windows-latest | checkout@v4, dtolnay/rust-toolchain@stable, `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, `cargo run -p wavical-integration-tests`, `cargo build --workspace` |
| linux | ubuntu-latest | checkout, setup-node@v4 (20), toolchain, fmt, `cargo clippy -p wavical-ipc -p wavical-protocol -p wavical-config -p wavical-cli`, `cargo test` (same), `cargo build` (same) |
| cli-e2e | ubuntu-latest | build wavical-cli, `--help` checks, `wavical init test-app`, `wavical build`, `test -f dist/{index.html,styles.css,wavical-manifest.json}` |
| security | ubuntu-latest | `taiki-e/install-action@v2` (cargo-audit, cargo-deny), `cargo audit`, `cargo deny check` |
(a) Linux jobs do not build wavical-shell, wavical-native or tao/wry/rfd/arboard/muda/tray-icon/global-hotkey crates. But the linux job runs `cargo test -p wavical-cli`, whose `dev_tests` spawn `wavical dev`, which needs a built `wavical-shell` (`find_wavical_shell`, cli lib.rs:311-396) and uses `taskkill` (dev_tests.rs:270). On plain Ubuntu these are likely to fail. NOT RUN on Linux. To verify: `wsl -e bash -lc "cargo test -p wavical-cli"`. PARTIAL.
(b) audit/deny installed before use via install-action (ci.yml:114-117). PASS (read, not executed here).
(c) deny.toml accepted by the installed cargo-deny: it ran with no config or deprecation messages. PASS. But the job will fail on the licence and advisory errors above, so `security` is red unless something changed upstream.
(d) Audit crate job: none. Fuzz smoke job: none (`git grep -i fuzz .github` empty). Phase 1 init+build e2e: present (`cli-e2e`). PARTIAL.
(e) Actions pinned to tags/branch (`@v4`, `@v2`, `@stable`), no SHAs. No `permissions:` block in the workflow. FAIL.
Also: Windows clippy lacks `--all-targets` (differs from the plan's strictest-practical rule); the integration harness prints `SKIPPED: WebView2 runtime is not available` (tests/integration/src/main.rs:86) and may pass without running attacks; its summary says "All 7 adversarial attack vectors" (main.rs:281) while `tests/attack-page/app.js` defines 12.

## Step 10: Drift matrix

### 10(a) Requirement -> implementation (locate only, no PASS)
| Requirement | Implementation | Tests | Status | Note |
|---|---|---|---|---|
| P0 window via tao+wry closure loop | shell lib.rs:602 `run_with_config` | none unit (manual) | FOUND | |
| P0 webview via wry | shell lib.rs:846-889 | none unit | FOUND | |
| P0 JSON-RPC bridge | ipc lib.rs:54-103, 288 | `test_valid_request_deserialization:782` | FOUND | |
| P0 default-deny capabilities | ipc lib.rs:306-343, 626 | `test_capability_default_is_empty_and_denies_all:961`, `test_counting_handler_proves_ungranted_methods_never_reach_handler:1183` | FOUND | |
| P0 frozen bridge object | shell lib.rs:136-137 `Object.freeze` | attack page only | PARTIAL | no Rust unit test |
| P0 schema validation | ipc lib.rs:288-301 | `test_rejection_*:800-929` | FOUND | |
| P0 path sanitisation | protocol lib.rs:169, 257 | `test_rejection_*:361-523` | FOUND | |
| P0 default CSP | shell lib.rs:19, 252, 521 | `test_protocol_ok_response_headers:1188`, `test_window_config_custom_csp_override:1141` | FOUND | |
| P0 navigation allowlist | shell lib.rs:317-344, 856 | `test_navigation_*:1361,1379` | FOUND | |
| P0 secrets never reachable | ipc lib.rs ~1079 (rule text) | `test_registered_methods_exact_allowlist:1086` | FOUND | |
| P0 one Tier 1 API e2e | ipc lib.rs:350 `handle_core_ping` | `test_process_ipc_message_ping_granted:1432` | FOUND | |
| P0 test bench in CI | tests/integration/src/main.rs; fuzz/; ci.yml:34-35 | proptest 10_000 cases (ipc lib.rs:1600) | PARTIAL | fuzz not in CI; harness can skip |
| P1 init/dev/build | cli lib.rs:32-40 | build_tests 7, cli_tests 4, dev_tests 4, e2e 8 | FOUND | |
| P1 `wca` alias | cli src/bin/wca.rs | cli_tests | FOUND | |
| P1 collision check | none in repo | none | PARTIAL | manual `where.exe` clean today |
| P1 flat config | config lib.rs:102 | config tests (24) | FOUND | |
| P1 Node subprocess validation, timeout, suggestions | config lib.rs:22, 525, 613, 351 | config tests | FOUND | |
| P1 capabilities in config, dynamic reload | config lib.rs:114; cli lib.rs:506-523 | `test_dynamic_capabilities_live_update_without_restart:1555` | FOUND | |
| P1 thiserror errors | cli lib.rs:96 | cli tests | FOUND | |
| P1 JSON-schema IntelliSense | none (no schema file tracked) | none | PARTIAL | checklist ticks it |
| P2 file dialogs | native lib.rs:306, 350 | `test_wp4_*:704-1056` | FOUND | |
| P2 clipboard | native lib.rs:233, 257 | `test_native_service_clipboard_*:612,634` | FOUND | |
| P2 notifications | native lib.rs:280 | `test_native_service_notify_send_limits_rejected:645` | FOUND | |
| P2 tray | native tray.rs; lib.rs:505 | `test_wp7_tray_*:1166,1181`, tray.rs tests | FOUND | |
| P2 global shortcuts | native shortcut.rs:91-143; lib.rs:523 | `test_wp8_shortcut_*:1314` | FOUND | |
| P2 context menu | native lib.rs:486; tray.rs:57 | `test_wp7_menu_*:1100-1224` | FOUND | |
| P2 window state | ipc window_state.rs; native window_state.rs | ipc `:230-356`; native `:111-154` | FOUND | |
| P2 single instance | native single_instance.rs:106 | `test_single_instance_handoff:165` | FOUND | |
| P2 file associations | ipc file_assoc.rs:82; cli lib.rs ~810-905 | `test_build_association_keys*:178,191` | FOUND | CLI falls back to its own exe path (lib.rs:822) |
| P2 taskbar badge | native lib.rs:550 (stub) | `test_wp8_badge_set_validation_and_not_supported_error:1261` | PARTIAL | documented deferral, always errors |
| P2 crash recovery | native crash_recovery.rs; ipc crash_recovery.rs | ipc `:60-157` | FOUND | unsafe, see F-A0-01 |
| P2 WebView2 detection UX | shell lib.rs:454-500 | `test_simulate_missing_webview2_runtime:1162` | FOUND | |
| P0 WP3 traversal suite zero escapes | protocol lib.rs | `:361-523`, proptests `:584,590` | FOUND | |
| P0 WP4 10k+ proptest | ipc lib.rs:1600 | `proptest_*:1603,1623` | FOUND | |
| P0 WP7 events in logs | shell | `test_all_security_events_emitted_on_violation:1489` | FOUND | |
| P0 WP8 expert review done | none | none | MISSING | no review record in repo |
| P0 CI green incl. audit/deny | ci.yml | n/a | PARTIAL | deny fails locally; CI result NOT RUN |
| P1 acceptance (init, build, errors) | cli lib.rs | build/cli/e2e tests | FOUND | |
| P1 CI runs init->dev->build | ci.yml cli-e2e | n/a | PARTIAL | `dev` not run in CI |
| P2 WP2 ungranted never reaches handler | ipc lib.rs:688 | `:1183` | FOUND | |
| P2 WP3 limits and denial tests | ipc lib.rs | `:1366-1552` | FOUND | |
| P2 WP4 grant tests (forged, cross-window, clear, symlink, limits) | native lib.rs:58, 399, 455 | `:704-940` | FOUND | |
| P2 WP5 parser and lock tests | ipc window_state.rs | `:248-356`, `:165` | FOUND | |
| P2 WP6 retry logic pure | ipc crash_recovery.rs:33 | `:60-157` | FOUND | |
| P2 WP7 menu validation | ipc menu.rs:122 | `:233-421` | FOUND | |
| P2 WP8 validation/naming | ipc shortcut.rs, badge.rs, file_assoc.rs | listed above | FOUND | |
| P2 WP9 attack page extended | tests/attack-page/app.js:17-20 | harness | FOUND | harness text says 7 vectors |
| P2 WP9 fuzz native parser and grant table | fuzz/fuzz_targets/native_request_parser.rs, grant_table.rs | not run | PARTIAL | not run in CI |
| P2 WP9 hostile review list + 3 tests-fail proofs | none | none | MISSING | no document in repo |
| P2 WP9 CI Windows builds native; Linux grant tests | ci.yml:37-38, 63-64 | n/a | FOUND | |

### 10(b) Implementation -> requirement
| Item | Document line | Result |
|---|---|---|
| Crates ipc, protocol, shell, cli, config, native, integration-tests | Phase 0 sec 3; Phase 1 sec 3; Phase 2 0.1 | documented |
| Pure-crate public API: Request/Response, Capabilities, dispatch*, parse_request, limits | Phase 0 WP4; Phase 2 WP2 | documented |
| ipc modules grants, window_state, crash_recovery, menu, badge, shortcut, file_assoc | Phase 2 0.3, WP4-WP8 | documented |
| protocol `resolve_asset_path`, `sanitize_path_segments`, `get_mime_type`, HEADER_NOSNIFF | Phase 0 WP3 | documented |
| GrantTable (insert/get/clear_window/count_for_window/total_count/remove_grant), `generate_grant_id` | Phase 2 0.3, WP4 | documented; `total_count`, `remove_grant` not mentioned (Info) |
| CLI: init (`--force`), dev, build (`--outdir`, `--release`), register-filetypes, unregister-filetypes | Phase 1 WP2-4; Phase 2 WP8 | documented |
| Config fields appId, window.{width,height,title,persistState}, capabilities, dev.port | Phase 1 schema | documented |
| Config fields `version`, `dialog.filters`, `singleInstance`, `tray.icon`, `shortcuts`, `fileAssociations` | Phase 2 plan 0.2/WP4/WP5/WP8 (implicit); schema doc silent | SCOPE CREEP (Medium, doc missing): `tray.icon`, `shortcuts`, `version` have no document line; others only implicitly in plan |
| `appId` serde alias `app_id`; method `AppConfig::appId` (config lib.rs:105, 228) | none | SCOPE CREEP (Low) |
| Hand-written SHA-256 (cli/src/sha256.rs) | Phase 1 WP4 "SHA256 hashes" | required feature, but custom crypto instead of a vetted crate (Low) |
| Constants MAX_MENU_LABEL_CHARS 100, MAX_MENU_ITEM_ID_CHARS 64, MAX_TRAY_TOOLTIP_CHARS 128, MAX_SHORTCUT_STRING_CHARS 64, MAX_EXTENSION_CHARS 10 | Phase 2 0.4 (not listed) | undocumented limits (Info) |
| Dependencies getrandom, ctrlc, windows-registry; shell direct muda/tray-icon/global-hotkey | none | SCOPE CREEP (Medium) |
| `project_rules.md`, duplicate `examples/hello` root files | none | SCOPE CREEP (Low) |
| WindowConfig fields beyond schema doc | doc lists resizable, minWidth, minHeight ("Phase 2+"); code has only 4 fields, `deny_unknown_fields` rejects them | doc says supported, code rejects |

Capability and method diff against Phase 2 plan 0.2 plus `core.ping`, `core.echo`:
- Plan: badge.set, clipboard.read, clipboard.write, dialog.openFile, dialog.saveFile, fs.readGranted, fs.writeGranted, menu.showContextMenu, notify.send, shortcut.register, tray.set + core.ping, core.echo = 13.
- Code `KNOWN_CAPABILITIES` (ipc lib.rs:378-392) = the same 13. `NATIVE_METHOD_NAMES` (lib.rs:363-375) = the 11 native names. Differences: none.
- Stale doc: Phase 1 schema still lists `fs.readFile`, `fs.writeFile`, `window.getState/setState` (schema doc lines 65-66, 117-118, 190-194); plan 0.2 supersedes them.

## Findings

### F-A0-01 High: Unapproved `unsafe` in `wavical-native/src/crash_recovery.rs`
- Where: `crates/wavical-native/src/crash_recovery.rs:36,51,62`.
- Repro: `git grep -n -E "unsafe|unsafe_code" -- '*.rs'`.
- Impact: Phase 2 plan 0.1 says stop and ask before own `unsafe`; decisions doc section 4 says exactly one allow (muda). A second allow with COM calls exists without a recorded approval.
- Suggested fix (not applied): record an explicit approval or remove it; fix the contradiction between decisions 3.3 and the code.

### F-A0-02 Medium: `cargo deny check` fails (licence and advisories)
- Where: `deny.toml:12-33`; CI `security` job.
- Repro: `cargo deny check` -> exit 5: BSL-1.0 on clipboard-win and error-code; zvariant, zbus_names, zvariant_derive unmaintained.
- Impact: the supply-chain gate is red; Phase 0 definition of done requires deny in CI.
- Fix: add BSL-1.0 to the allow list (if accepted) and review/ignore the advisories explicitly with reasons.

### F-A0-03 Medium: Flaky dev test that leaks processes
- Where: `crates/wavical-cli/tests/dev_tests.rs:245-275`.
- Repro: `cargo test --workspace` first run failed at line 263; rerun passed.
- Impact: unreliable baseline; failure leaves `wavical.exe` and `wavical-shell.exe` running and hangs redirected output.
- Fix: kill children in a drop guard; raise or condition the startup timeout.

### F-A0-04 Medium: Linux CI probably runs Windows-dependent CLI tests
- Where: `ci.yml:60-67` with `dev_tests.rs:270` (`taskkill`) and `cli lib.rs:311-396`.
- Impact: likely red Linux job, or tests silently skipped. Unverified (no `gh`).
- Fix: gate the dev tests with `#[cfg(windows)]` or build the shell first.

### F-A0-05 Medium: Scope creep without document lines
- Where: config fields `tray.icon`, `shortcuts`, `version`; dependencies getrandom, ctrlc, windows-registry; shell direct deps muda, tray-icon, global-hotkey (see 10(b)).
- Fix: document or remove.

### F-A0-06 Medium: Stale or missing plan artefacts for WP9
- No hostile-review list, no "tests-fail" proofs, no expert-review record for Phase 0 WP8 in the repo. Checklist Phase 2 boxes are unticked, the phase index says Phase 2 "NOT STARTED", while the decisions doc says WP1-WP8 are done.
- Fix: write the artefacts; update the docs.

### F-A0-07 Low: `RwLock .unwrap()` in grant table
- `crates/wavical-ipc/src/grants.rs:80,122,131,137,143,149`. A poisoned lock turns into a panic on every later grant call.

### F-A0-08 Low: Single-instance listener hardening
- `crates/wavical-native/src/single_instance.rs:48-67,78`. Sequential accept with no read timeout: a client that connects and stalls blocks all later handoffs. Forwarded args are unauthenticated. `.expect` on thread spawn.

### F-A0-09 Low: File-association registration falls back to the CLI's own exe
- `crates/wavical-cli/src/lib.rs:822-824`. If the shell is not found, `register-filetypes` registers `current_exe()` (the CLI) as the opener.

### F-A0-10 Low: CI hygiene
- No `permissions:`, actions not pinned to SHAs, `@stable` branch ref, Windows clippy without `--all-targets`, no fuzz job, no audit-crate job, integration harness may skip silently and reports "7 vectors" for a 12-vector page.

### F-A0-11 Low: Dev-only `windows-sys` in pure crates; `getrandom` in a pure crate
- Literal breach of the purity rule in `-e normal,dev`; shipped tree is clean.

### F-A0-12 Low: Hand-rolled SHA-256 for manifest hashes
- `crates/wavical-cli/src/sha256.rs`. Prefer a reviewed crate.

### F-A0-13 Info: `wavical-native` uses `deny` rather than `forbid`; documented.
### F-A0-14 Info: Git hygiene (bare-number commit messages, `0.2.71` ordering, `0.2.9` message mismatch).
### F-A0-15 Info: `cargo audit` allowed warnings: glib unsound (RUSTSEC-2024-0429, GTK path, Linux), clipboard-win 5.4.1 yanked (`cargo update -p clipboard-win`).

## Requirement table (A0 checks)
| Id | Requirement | Status | Evidence |
|---|---|---|---|
| A0-01 | Environment recorded | PASS | Step 1 |
| A0-02 | Clean git tree | PASS | `git status --short` empty |
| A0-03 | No secrets in files or history | PASS | Step 2 |
| A0-04 | Commits match messages | PARTIAL | Step 2 |
| A0-05 | `cargo build --workspace` | PASS | exit 0 |
| A0-06 | `cargo build --workspace --release` | PASS | exit 0 |
| A0-07 | `cargo test --workspace` | PARTIAL | first run exit 101; rerun `--no-fail-fast` exit 0, 213 tests |
| A0-08 | clippy `--all-targets -D warnings` | PASS | exit 0 |
| A0-09 | `cargo fmt --all --check` | PASS | exit 0 |
| A0-10 | `cargo audit` | PASS | exit 0, 6 warnings |
| A0-11 | `cargo deny check` | FAIL | exit 5 |
| A0-12 | cargo mutants available | NOT RUN | install given |
| A0-13 | cargo fuzz available | NOT RUN | install given |
| A0-14 | Layout matches plans | PARTIAL | Step 4 |
| A0-15 | Pure crates: normal tree clean | PASS | Step 5 |
| A0-16 | Pure crates: dev tree clean | FAIL | windows-sys via proptest/tempfile |
| A0-17 | Dependency direction, no cycles | PASS | Step 5 |
| A0-18 | `forbid(unsafe_code)` on every crate root | PARTIAL | native uses deny |
| A0-19 | Every `unsafe` approved | FAIL | crash_recovery.rs |
| A0-20 | Panic-prone code justified | PARTIAL | Step 7 |
| A0-21 | Dependencies justified in docs | PARTIAL | Step 8 |
| A0-22 | Cargo.lock committed | PASS | tracked |
| A0-23 | Linux CI avoids GUI crates | PARTIAL | cli dev tests |
| A0-24 | audit/deny installed before use | PASS | ci.yml:114-117 (read) |
| A0-25 | deny.toml keys accepted | PASS | no config warnings when run |
| A0-26 | CI runs audit crate, fuzz, init+build | PARTIAL | e2e only |
| A0-27 | Actions pinned, minimal permissions | FAIL | Step 9 |
| A0-28 | `gh run list` | NOT RUN | `gh` not installed |
| A0-29 | Capability list equals plan 0.2 + core | PASS | zero differences |

## Has the project drifted from the plans?
Largely no on features: the 13 capabilities and 11 native methods match the Phase 2 plan exactly, all crate boundaries respect the purity rule in shipped code, and every checklist feature has code and tests behind it. The drift is in discipline and bookkeeping. A second `unsafe` site appeared without the required approval and the decisions doc contradicts itself on it. The supply-chain gate (`cargo deny`) is red, CI is weaker than the plans imply (no fuzz, no permissions, unpinned actions, a Linux job that probably cannot run the CLI dev tests), and the baseline test suite is not reliably green. Several config fields and dependencies exist with no document line, while key WP9 artefacts (hostile review, tests-fail proofs, expert-review record) and the doc status pages are missing or stale. The taskbar badge is a documented deferral (always returns an error), so that checklist item is not truly delivered.
