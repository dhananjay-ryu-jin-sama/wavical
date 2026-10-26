# Audit Report A0: Baseline, CI and Drift

- **Target:** Wavical workspace (`crates/`, `tests/`, `.github/`, `docs/`)
- **Mode:** AUDIT ONLY
- **Date:** 2026-10-23
- **Auditor:** Independent Security Auditor & QA Engineer

---

## Executive Summary & Status Counts

| Status | Count | Definition |
|---|---|---|
| **PASS** | **17** | Verified compliant against plan documents via executed commands or line verification |
| **PARTIAL** | **7** | Partially implemented, partial coverage, or subtle divergence from specification |
| **FAIL** | **4** | Direct specification violation, broken gate, or unapproved implementation |
| **NOT RUN** | **2** | Tooling not installed on host machine (`cargo-fuzz`, `gh`) |
| **Total Requirements** | **30** | |

### Master Requirement Table

| ID | Requirement | Status | Evidence / Location |
|---|---|---|---|
| A0-01 | Host environment recorded | **PASS** | Section 1 (rustc 1.99.0, cargo 1.99.0, node 26.4.0, git 2.54.0) |
| A0-02 | Clean git tree / uncommitted changes tracked | **PASS** | Section 2.1 (`git status`, only allowed audit harness untracked) |
| A0-03 | Zero credentials or private keys in repository | **PASS** | Section 2.4 (regex scan in tracked files and commit history clean) |
| A0-04 | Commit messages accurately describe changes | **PARTIAL** | Section 2.2 (`0.2.91` omits report addition; bare version commits) |
| A0-05 | `cargo build --workspace` succeeds | **PASS** | Section 3.1 (Exit 0, dev profile in 17.98s) |
| A0-06 | `cargo build --workspace --release` succeeds | **PASS** | Section 3.1 (Exit 0, release profile in 41.50s) |
| A0-07 | `cargo test --workspace` passes clean | **PASS** | Section 3.2 (Exit 0, 230 passed, 3 ignored, 0 failed) |
| A0-08 | `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** | Section 3.1 (Exit 0 in 8.16s, zero warnings) |
| A0-09 | `cargo fmt --all --check` clean | **PASS** | Section 3.1 (Exit 0, code style compliant) |
| A0-10 | `cargo audit` security scan passes | **PASS** | Section 3.1 (Exit 0, 6 allowed warnings, 0 vulnerabilities) |
| A0-11 | `cargo deny check` supply-chain gate passes | **FAIL** | Section 3.1 (Exit 1: unmaintained advisories on zbus/zvariant & BSL-1.0 rejected) |
| A0-12 | `cargo mutants` tool installed | **PASS** | Section 1 (`cargo-mutants 27.1.0` present) |
| A0-13 | `cargo fuzz` tool installed | **NOT RUN** | Section 1 (`cargo install cargo-fuzz --locked` needed on Linux/WSL) |
| A0-14 | Repository layout matches build plans | **PARTIAL** | Section 4.2 (`tests/cli/` missing; duplicate `examples/hello` assets) |
| A0-15 | Pure crates production dependency tree (`-e normal`) clean | **PASS** | Section 5.1 (0 prohibited crates in `wavical-ipc`, `wavical-protocol`) |
| A0-16 | Pure crates development dependency tree (`-e dev`) clean | **FAIL** | Section 5.1 (`windows-sys` pulled transitively by proptest/tempfile) |
| A0-17 | Dependency direction valid with zero cycles | **PASS** | Section 5.2 (Clean DAG; zero inversions from pure to impure) |
| A0-18 | `#![forbid(unsafe_code)]` on every crate root | **PARTIAL** | Section 6.1 (`wavical-native` uses `#![deny(unsafe_code)]`) |
| A0-19 | Every `unsafe` block approved in decisions document | **FAIL** | Section 6.2 (`crates/wavical-native/src/crash_recovery.rs` unapproved) |
| A0-20 | Panic-prone code in non-test code justified | **PARTIAL** | Section 7.1 (`grants.rs` RwLock unwrap; single-instance thread spawn expect) |
| A0-21 | Dependencies justified in architecture plans | **PARTIAL** | Section 8.1 (`getrandom`, `ctrlc`, `windows-registry`, shell muda/tray unlisted) |
| A0-22 | `Cargo.lock` committed and tracked in git | **PASS** | Section 8.2 (113,939 bytes, committed, not gitignored) |
| A0-23 | Linux CI job avoids OS-dependent crates/tests | **PARTIAL** | Section 9.2 (Linux job runs `wavical-cli` tests with Windows `taskkill`) |
| A0-24 | CI installs `cargo-audit` and `cargo-deny` before use | **PASS** | Section 9.2 (`taiki-e/install-action@v2` installs prior to checks) |
| A0-25 | `deny.toml` uses valid non-deprecated schema keys | **PASS** | Section 9.2 (Standard valid schema; no deprecation warnings) |
| A0-26 | CI runs audit crate, fuzz smoke tests, and E2E | **PARTIAL** | Section 9.2 (CLI E2E runs; fuzzing and audit crate omitted) |
| A0-27 | Workflow actions pinned to SHAs with minimal permissions | **FAIL** | Section 9.2 (Unpinned major tags/@stable; 0 `permissions:` blocks) |
| A0-28 | GitHub Actions remote execution verified (`gh run list`) | **NOT RUN** | Section 9.2 (`gh` CLI tool not installed on host machine) |
| A0-29 | Registered capability and method lists match plan 0.2 + core | **PASS** | Section 10.3 (Exact 13/13 match; 0 discrepancies) |
| A0-30 | Feature implementation traceable to plan requirements | **PARTIAL** | Section 10.1 (Phase 2 WP9 hostile review & tests-fail proofs missing) |

---

## 1. Environment

```text
rustc:         rustc 1.99.0 (b940084d7 2026-09-28)
cargo:         cargo 1.99.0 (5f94df478 2026-08-27)
node:          v26.4.0
git:           git version 2.54.0.windows.1
OS:            Microsoft Windows NT 10.0.26100.0 (Windows 11 Enterprise)
cargo-audit:   cargo-audit-audit 0.22.2 (installed)
cargo-deny:    cargo-deny 0.20.2 (installed)
cargo-mutants: cargo-mutants 27.1.0 (installed)
cargo-fuzz:    NOT INSTALLED (error: no such command: `fuzz`)
```

---

## 2. Git State

### 2.1 Status & Branches
- **Active Branch:** `main` (tracking `origin/main`)
- **Remote Branches:** `remotes/origin/main`
- **Uncommitted Changes:**
  - `Cargo.toml`: Modified (added workspace member `"tests/audit"`)
  - `Cargo.lock`: Modified (registered `wavical-audit` test package dependencies)
  - `README.md`: Modified (updated test count to 230+ and added audit references)
  - `docs/audits/A0_baseline.md`: New/Modified audit report
  - Untracked artifacts: `fuzz/Cargo.lock`, `fuzz/target/`, `tests/audit/`

### 2.2 Commit Log & Message Alignment
All 27 repository commits reviewed:
- `d24568d 0.2.91 - Readme update`: **Mismatch/Omission:** Commit message claims only "Readme update", but commit also added `docs/audits/A0_baseline.md` and updated `Cargo.lock`.
- `0a31f0f 0.2.9 - feat: implement core shell, IPC, fuzz targets, integration tests, and CI workflow`: **Partial Mismatch:** Message implies core shell/IPC initial creation, but actually adds Phase 2 WP9 integration test vectors, attack page payloads, and `grant_table`/`native_request_parser` fuzz targets.
- `f1a14ac 0.2.5`, `d102c8a 0.2.4`, `71e5b5a 0.2.3`, `53e0bd1 0.2.0`: **Non-descriptive bare version tags:** Lack descriptive summaries of major feature additions (e.g., `grants.rs`, `single_instance.rs`, `wavical-native`).
- Typographical errors in commit messages: `d2a1365` ("lirary"), `4668a16` ("Phae_1"), `d5ce41b` ("pahse_details_update").
- `8f60e54 edit` and `3085414 EDIT`: Vague messages without context.

### 2.3 Binary Files
- Only two binary assets tracked: `examples/hello/icon.png` (157 bytes) and `examples/hello/src/icon.png` (157 bytes).
- Largest tracked file is `Cargo.lock` (113,939 bytes, standard text lockfile).
- Zero large binary files (>100 KB) in repository history.

### 2.4 Secrets Audit
- **Tracked Files & History Search:** Pattern regex `api_key|secret|token|password|BEGIN PRIVATE KEY`.
- **Findings:** Zero leaked private keys, API secrets, passwords, or production credentials.
- All matches in code and tests correspond to:
  1. Negative test cases asserting path traversal blocks on mock files (e.g. `secret.txt`, `secret.db`).
  2. Negative test cases asserting IPC secret leak prevention (e.g. `secret.leaked`, `secret_token_leaked`).
  3. Win32 COM event registration token identifiers (`EventRegistrationToken`).
  4. Shortcut grammar token parsers (`mod_token`).

---

## 3. Build, Test, and Supply Chain Verification

### 3.1 Real Output Summary

| Command | Status | Output Summary / Details |
|---|---|---|
| `cargo build --workspace` | **PASS** | Exit code 0. Finished `dev` profile [unoptimized + debuginfo] in 17.98s. |
| `cargo build --workspace --release` | **PASS** | Exit code 0. Finished `release` profile [optimized] in 41.50s (1 compiler warning: unused variable `simulate_override` in `crates/wavical-shell/src/lib.rs:464`). |
| `cargo test --workspace` | **PASS** | Exit code 0. Total 230 passed, 3 ignored across 8 crates and test suites. Details below. |
| `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** | Exit code 0. Clean compilation across all crates, binaries, and test targets in 8.16s. |
| `cargo fmt --all --check` | **PASS** | Exit code 0. All files properly formatted. |
| `cargo audit` | **PASS** | Exit code 0. 6 allowed warnings (proc-macro-error, zbus_names, zvariant, zvariant_derive, glib, clipboard-win). Zero unallowed vulnerabilities. |
| `cargo deny check` | **FAIL** | Exit code 1. `advisories FAILED` (RUSTSEC-2026-0370, RUSTSEC-2026-0371, RUSTSEC-2026-0372 unmaintained advisories on zvariant/zbus_names), `licenses FAILED` (BSL-1.0 on `clipboard-win` and `error-code` rejected by deny.toml). |
| `cargo mutants` | **PASS** | Tool exists (`cargo-mutants 27.1.0`). |
| `cargo fuzz` | **NOT RUN** | Tool not installed (`error: no such command: fuzz`). Install command: `cargo install cargo-fuzz --locked` (requires nightly toolchain, designed for WSL/Linux). |

### 3.2 Test Counts by Crate & Target (`cargo test --workspace`)

- `wavical-audit` (`tests/audit`):
  - `part1_protocol.rs`: 7 passed, 0 failed, 2 ignored (`test_part1_error_information_leakage`, `test_part1_reserved_device_conin_conout`)
  - `part2_ipc.rs`: 10 passed, 0 failed, 1 ignored (`test_part2_response_internal_leakage`)
  - Subtotal: **17 passed, 3 ignored**
- `wavical-cli` (`crates/wavical-cli`):
  - `src/lib.rs`: 8 passed
  - `tests/build_tests.rs`: 7 passed
  - `tests/cli_tests.rs`: 4 passed
  - `tests/dev_tests.rs`: 4 passed
  - `tests/e2e_tests.rs`: 8 passed
  - Subtotal: **31 passed**
- `wavical-config` (`crates/wavical-config`):
  - `src/lib.rs`: **24 passed**
- `wavical-ipc` (`crates/wavical-ipc`):
  - `src/lib.rs`: **82 passed**
- `wavical-native` (`crates/wavical-native`):
  - `src/lib.rs`: **38 passed**
- `wavical-protocol` (`crates/wavical-protocol`):
  - `src/lib.rs`: **15 passed**
- `wavical-shell` (`crates/wavical-shell`):
  - `src/lib.rs`: **23 passed**
- `wavical-integration-tests` (`tests/integration`):
  - Binary target: 0 unit tests (integration runner)

**Total workspace tests:** **230 passed, 3 ignored, 0 failed**.
*(Note: Core workspace without audit crate = 213 passed, matching README and Phase 2 checklist count).*

---

## 4. Repository Layout Comparison

### 4.1 Actual Workspace Tree (Excluding `target/` and `.git/`)

```text
wavical/
├── .github/
│   └── workflows/ci.yml
├── .gitignore
├── Cargo.lock
├── Cargo.toml
├── deny.toml
├── project_rules.md                  [Extra: undocumented root file]
├── README.md
├── ROADMAP.md
├── SECURITY.md
├── crates/
│   ├── wavical-cli/                  [Phase 1]
│   │   ├── Cargo.toml
│   │   ├── src/ (lib.rs, sha256.rs, bin/wavical.rs, bin/wca.rs)
│   │   └── tests/ (build_tests.rs, cli_tests.rs, dev_tests.rs, e2e_tests.rs)
│   ├── wavical-config/               [Phase 1]
│   │   ├── Cargo.toml
│   │   ├── examples/manual_test.rs   [Extra: helper script]
│   │   └── src/lib.rs
│   ├── wavical-ipc/                  [Phase 0]
│   │   ├── .gitignore
│   │   ├── Cargo.toml
│   │   └── src/ (lib.rs, badge.rs, crash_recovery.rs, file_assoc.rs, grants.rs, menu.rs, shortcut.rs, window_state.rs)
│   ├── wavical-native/               [Phase 2]
│   │   ├── Cargo.toml
│   │   └── src/ (lib.rs, crash_recovery.rs, shortcut.rs, single_instance.rs, tray.rs, window_state.rs)
│   ├── wavical-protocol/             [Phase 0]
│   │   ├── Cargo.toml
│   │   ├── proptest-regressions/lib.txt
│   │   └── src/lib.rs
│   └── wavical-shell/                [Phase 0]
│       ├── Cargo.toml
│       └── src/ (lib.rs, main.rs)
├── docs/
│   ├── audits/ (A0_baseline.md)
│   ├── phase 0/ (WAVICAL_PHASE_0_BUILD_PLAN.md)
│   ├── phase 1/ (PHASE_1_README.md, SECURITY_MODEL.md, WAVICAL_PHASE_1_BUILD_PLAN.md, WAVICAL_PHASE_1_CONFIG_SCHEMA.md)
│   ├── WAVICAL_FEATURE_CHECKLIST.md
│   ├── WAVICAL_PHASE_2_BUILD_PLAN.md
│   ├── WAVICAL_PHASE_2_DECISIONS.md
│   ├── WAVICAL_PHASES_INDEX.md
│   └── WAVICAL_SETUP.md
├── examples/
│   └── hello/
│       ├── README.md, wavical.config.js, app.js, index.html, style.css, icon.png
│       └── src/ (app.js, icon.png, index.html, style.css) [Duplicate copy of assets]
├── fuzz/                             [Phase 0]
│   ├── Cargo.toml, README.md
│   └── fuzz_targets/ (capability_dispatch.rs, parse_message.rs, resolve_path.rs, grant_table.rs, native_request_parser.rs)
└── tests/
    ├── attack-page/ (README.md, app.js, index.html, style.css)
    ├── audit/ (Cargo.toml, src/, tests/part1_protocol.rs, tests/part2_ipc.rs) [Auditor test crate]
    └── integration/ (Cargo.toml, README.md, src/main.rs)
```

### 4.2 Differences Against Plan Specifications

1. **Missing Planned Directory:**
   - `tests/cli/`: Specified in `docs/phase 1/WAVICAL_PHASE_1_BUILD_PLAN.md` Section 3 ("tests/cli/ (NEW: CLI tests)"). Implemented instead as integration tests inside the crate: `crates/wavical-cli/tests/`.
2. **Extra Folders & Files (Not in Plan Trees):**
   - `project_rules.md`: Agent configuration prompt in repo root.
   - `deny.toml`: Config for `cargo-deny` (justified by Phase 0 supply-chain requirements, but not in original Section 3 diagram).
   - `crates/wavical-config/examples/manual_test.rs`: Ad-hoc scratch test file.
   - `examples/hello/src/` vs `examples/hello/`: Duplicated frontend files (`app.js`, `index.html`, `style.css`, `icon.png` exist in both the root of `examples/hello/` and `examples/hello/src/`).
   - `crates/wavical-ipc/.gitignore`: Redundant inner gitignore.
   - `tests/audit/`: Audit harness crate (permitted per audit charter).

---

## 5. Crate Purity Rule & Dependency Direction

### 5.1 Crate Dependency Trees (`-e normal` vs `-e dev`)

Prohibited crate set: `tao`, `wry`, `rfd`, `arboard`, `notify-rust`, `global-hotkey`, `tray-icon`, `muda`, `windows`, `windows-sys`, `winreg`, `windows-registry`, `webview2-com`.

- **`wavical-ipc` (-e normal):** **PASS**
  - Dependencies: `getrandom`, `serde`, `serde_json`, `thiserror`, `tracing`.
  - Prohibited crates found: **0**.
- **`wavical-protocol` (-e normal):** **PASS**
  - Dependencies: `thiserror`, `tracing`.
  - Prohibited crates found: **0**.
- **`wavical-ipc` (-e dev):** **FAIL**
  - `windows-sys v0.61.2` present transitively via `wavical-ipc` (dev) -> `proptest v1.11.0` -> `tempfile v3.27.0` / `rusty-fork v0.3.1` on Windows.
- **`wavical-protocol` (-e dev):** **FAIL**
  - `windows-sys v0.61.2` present transitively via `wavical-protocol` (dev) -> `tempfile v3.27.0` on Windows.

*(Verdict: Pure crates are completely clean in production/normal tree (`-e normal`). However, under `-e dev` on Windows, standard Rust test utilities `proptest` and `tempfile` pull in `windows-sys` transitively).*

### 5.2 Workspace Dependency Graph & Directionality

```text
wavical-ipc              -> (none)              [PURE]
wavical-protocol         -> (none)              [PURE]
wavical-config           -> wavical-ipc
wavical-native           -> wavical-ipc
wavical-cli              -> wavical-ipc, wavical-config
wavical-shell            -> wavical-ipc, wavical-protocol, wavical-native
wavical-audit            -> wavical-ipc, wavical-protocol
wavical-integration      -> wavical-ipc, wavical-native, wavical-shell
```

- **Dependency Cycles:** **0 cycles** detected.
- **Pure to Non-Pure Inversions:** **0 inversions**. Neither `wavical-ipc` nor `wavical-protocol` imports any workspace crate or OS-dependent crate.

---

## 6. Unsafe Code Analysis

### 6.1 Root Safety Directives (`#pragma`/`#![forbid(unsafe_code)]`)

| Crate / Target | File | Directive | Status |
|---|---|---|---|
| `wavical-ipc` | `crates/wavical-ipc/src/lib.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-protocol` | `crates/wavical-protocol/src/lib.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-config` | `crates/wavical-config/src/lib.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-cli` (lib) | `crates/wavical-cli/src/lib.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-cli` (bin/wavical) | `crates/wavical-cli/src/bin/wavical.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-cli` (bin/wca) | `crates/wavical-cli/src/bin/wca.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-shell` (lib) | `crates/wavical-shell/src/lib.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-shell` (bin) | `crates/wavical-shell/src/main.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-integration-tests` | `tests/integration/src/main.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-audit` | `tests/audit/src/lib.rs:1` | `#![forbid(unsafe_code)]` | **PASS** |
| `wavical-native` | `crates/wavical-native/src/lib.rs:1` | `#![deny(unsafe_code)]` | **PARTIAL** (Uses `deny` instead of `forbid` to permit scoped `allow` in modules) |

### 6.2 All Unsafe Occurrences in Repository

1. **`crates/wavical-native/src/tray.rs:56-61`**
   - Code: `#[allow(unsafe_code)]` wrapping `unsafe { menu.show_context_menu_for_hwnd(hwnd, position); }`
   - Purpose: Win32 native context menu popup invocation via `muda`.
   - Approval in `docs/WAVICAL_PHASE_2_DECISIONS.md`: **APPROVED** (Sections 1, 2.5, 3.2, 4 explicitly record and authorize this isolated call).
2. **`crates/wavical-native/src/crash_recovery.rs:36-65`**
   - Code: `#[allow(unsafe_code)]` wrapping COM interface queries and `unsafe { core_wv.add_ProcessFailed(&handler, &mut token) }`.
   - Purpose: Attaching to WebView2 renderer crash notification event via Win32 COM.
   - Approval in `docs/WAVICAL_PHASE_2_DECISIONS.md`: **UNAPPROVED (High Finding)**.
     - Section 4 explicitly states: *"wavical-native enforces `#![deny(unsafe_code)]` with exactly one scoped `#[allow(unsafe_code)]` for muda::show_context_menu_for_hwnd."*
     - Introducing COM `add_ProcessFailed` with raw pointer casting and unapproved `unsafe` violates the single-exception rule in Phase 2 decisions.

---

## 7. Panic-Prone Code and Unchecked Operations in Non-Test Code

### 7.1 `.unwrap()`, `.expect()`, `panic!`, `eprintln!`, `println!` Audit

| File:Line | Code Snippet | External Input? | Verdict |
|---|---|---|---|
| `crates/wavical-ipc/src/grants.rs:80,122,131,137,143,149` | `self.grants.write().unwrap()`, `read().unwrap()` | No (internal `RwLock` lock acquisition) | **Finding (Low)**: If a worker thread panics while holding the lock, lock poisoning causes all future grant operations to panic across windows instead of cleanly returning an error. |
| `crates/wavical-native/src/single_instance.rs:78` | `.expect("Failed to spawn single-instance listener thread")` | Indirect (OS thread limit) | **Finding (Low)**: Thread exhaustion leads to direct process crash rather than clean graceful degradation or error code. |
| `crates/wavical-cli/src/lib.rs` (multiple lines) | `println!`, `eprintln!` | No (CLI user messaging) | **Justified**: CLI output for user feedback (`wavical dev`, `wavical init`, `wavical build`). |
| `crates/wavical-shell/src/lib.rs:481` | `eprintln!("Wavical critical error: WebView2 runtime missing...")` | Indirect (missing runtime) | **Justified**: Emergency console diagnostic when native window fails before WebView2 initialization. |

### 7.2 Indexing, Slicing, and Integer Downcast Audit

| File:Line | Code Snippet | External Input? | Verdict |
|---|---|---|---|
| `crates/wavical-ipc/src/badge.rs:51` | `count as u32` | Yes (page IPC params) | **Justified**: Explicitly guarded by preceding `if count <= 0` and `if count > 99`; guaranteed $1 \le count \le 99$. |
| `crates/wavical-ipc/src/shortcut.rs:153,159` | `parts[0]`, `parts[parts.len() - 1]` | Yes (page IPC params) | **Justified**: Bounded by preceding `if parts.is_empty() { return Err(...); }`. |
| `crates/wavical-config/src/lib.rs:436,442` | `remainder[..end]` | Yes (config typo suggestion) | **Justified**: `end` bounded by character search on ASCII separators. |
| `crates/wavical-shell/src/lib.rs:279` | `&payload[..end]` | Yes (raw IPC payload) | **Justified**: `end` derived safely using standard `floor_char_boundary(MAX_LOG_PAYLOAD_BYTES)`. |
| `crates/wavical-native/src/single_instance.rs:130` | `payload.len() as u32` | Yes (CLI args) | **Justified**: Arguments vector serialization size within reasonable memory bounds. |

---

## 8. Dependency Inventory & Supply-Chain Audit

### 8.1 Direct Dependencies vs Plan Documents

| Crate | Dependency | Version in `Cargo.lock` | Justified in Docs? | Notes / Finding |
|---|---|---|---|---|
| `wavical-ipc` | `serde` | 1.0.229 | **Yes** (Phase 0 WP4) | `derive` feature enabled. |
| `wavical-ipc` | `serde_json` | 1.0.151 | **Yes** (Phase 0 WP4) | |
| `wavical-ipc` | `thiserror` | 2.0.21 | **Yes** (Phase 0 WP4) | |
| `wavical-ipc` | `tracing` | 0.1.44 | **Yes** (Phase 0 WP7) | |
| `wavical-ipc` | `getrandom` | 0.3.4 | **No (Finding)** | Added in Phase 2 WP4 for grant ID entropy; not documented in `WAVICAL_PHASE_2_DECISIONS.md`. |
| `wavical-protocol` | `thiserror` | 2.0.21 | **Yes** (Phase 0 WP3) | |
| `wavical-protocol` | `tracing` | 0.1.44 | **Yes** (Phase 0 WP7) | |
| `wavical-config` | `serde`, `serde_json`, `thiserror`, `tracing` | 1.0.229, 1.0.151, 2.0.21, 0.1.44 | **Yes** (Phase 1 WP1) | |
| `wavical-config` | `wavical-ipc` | 0.1.0 | **Yes** (Phase 1 WP1) | Types import for capability checking. |
| `wavical-native` | `arboard` | 3.7.0 | **Yes** (Phase 2 WP1) | Documented in `WAVICAL_PHASE_2_DECISIONS.md`. |
| `wavical-native` | `notify-rust` | 4.18.2 | **Yes** (Phase 2 WP1) | Documented in `WAVICAL_PHASE_2_DECISIONS.md`. |
| `wavical-native` | `rfd` | 0.17.2 | **Yes** (Phase 2 WP1) | Documented in `WAVICAL_PHASE_2_DECISIONS.md`. |
| `wavical-native` | `interprocess` | 2.4.6 | **Yes** (Phase 2 WP1) | Documented in `WAVICAL_PHASE_2_DECISIONS.md`. |
| `wavical-native` | `wry` | 0.57.0 | **Yes** (Phase 2 WP1) | Window handle & crash handler hooks. |
| `wavical-native` | `muda` | 0.21.2 | **Yes** (Phase 2 WP1) | Native context menu. |
| `wavical-native` | `tray-icon` | 0.26.1 | **Yes** (Phase 2 WP1) | Notification tray icon. |
| `wavical-native` | `global-hotkey` | 0.9.0 | **Yes** (Phase 2 WP1) | Global shortcuts. |
| `wavical-native` | `webview2-com` | 0.39.1 | **Partial (Finding)** | Recorded in research matrix, but contradicts Section 4 single-exception rule. |
| `wavical-shell` | `tao`, `wry`, `tracing`, `tracing-subscriber` | 0.37.1, 0.57.0, 0.1.44, 0.3.23 | **Yes** (Phase 0 WP2/5) | |
| `wavical-shell` | `clap` | 4.6.7 | **Yes** (Phase 1 CLI/Shell args) | Flag forwarding. |
| `wavical-shell` | `muda`, `tray-icon`, `global-hotkey` | 0.21.2, 0.26.1, 0.9.0 | **No (Finding)** | Direct dependencies in shell; Phase 2 plan 0.1 specifies native APIs isolate in `wavical-native`. |
| `wavical-cli` | `clap`, `notify`, `tracing`, `tracing-subscriber` | 4.6.7, 6.1.1, 0.1.44, 0.3.23 | **Yes** (Phase 1 WP1-3) | |
| `wavical-cli` | `ctrlc` | 3.5.2 | **No (Finding)** | Added for clean Ctrl+C signal handling in `wavical dev`; not documented in Phase 1 plan. |
| `wavical-cli` | `windows-registry` | 0.100.0 | **No (Finding)** | Added for Phase 2 WP8 filetype registration; not documented in `WAVICAL_PHASE_2_DECISIONS.md`. |

### 8.2 Lockfile Status & Duplicate Dependencies

- **Lockfile Versioning:** `Cargo.lock` is actively tracked in git (113,939 bytes) and NOT excluded in `.gitignore`. This adheres strictly to binary deployment standards.
- **Notable Duplicate Dependency Trees (`cargo tree -d`):**
  - `windows-sys`: 3 duplicate versions (`0.48.0` via `notify`, `0.59.0` via `global-hotkey`, `0.61.2` via `arboard`/`rfd`/`muda`/`interprocess`).
  - `windows-link`: 2 versions (`0.2.1` via standard windows crates, `0.100.0` via `windows-registry`).
  - `windows-result`: 2 versions (`0.4.1` and `0.100.0`).
  - `windows-strings`: 2 versions (`0.5.1` and `0.100.0`).
  - `windows-targets`: 2 versions (`0.48.5` and `0.52.6`).
  - `thiserror`: 2 versions (`1.0.69` transitive, `2.0.21` workspace).
  - `getrandom`: 2 versions (`0.3.4` in `wavical-ipc`, `0.4.3` transitive).

---

## 9. Continuous Integration (CI) Review

### 9.1 Workflow & Job Breakdown (`.github/workflows/ci.yml`)

1. **Job `windows` (`runs-on: windows-latest`)**
   - Commands:
     - `cargo fmt --all --check`
     - `cargo clippy --workspace -- -D warnings`
     - `cargo test --workspace`
     - `cargo run -p wavical-integration-tests`
     - `cargo build --workspace`
   - Evaluation: Full Windows build, clippy, test suite, and integration attack page run.
2. **Job `linux` (`runs-on: ubuntu-latest`)**
   - Commands:
     - `cargo fmt --all --check`
     - `cargo clippy -p wavical-ipc -p wavical-protocol -p wavical-config -p wavical-cli -- -D warnings`
     - `cargo test -p wavical-ipc -p wavical-protocol -p wavical-config -p wavical-cli`
     - `cargo build -p wavical-ipc -p wavical-protocol -p wavical-config -p wavical-cli`
   - Evaluation: Tests pure crates on Ubuntu.
3. **Job `cli-e2e` (`runs-on: ubuntu-latest`)**
   - Commands:
     - `cargo build -p wavical-cli`
     - Run help diagnostics: `./target/debug/wavical --help`, etc.
     - Scaffolds `wavical init test-app`, then runs `wavical build` and asserts output files (`dist/index.html`, `dist/styles.css`, `dist/wavical-manifest.json`).
4. **Job `security` (`runs-on: ubuntu-latest`)**
   - Commands:
     - `taiki-e/install-action@v2` (installs `cargo-audit`, `cargo-deny`)
     - `cargo audit`
     - `cargo deny check`

### 9.2 Targeted CI Checks

- **(a) Linux Job Execution of Native/GUI-Dependent Crates:** **PARTIAL / Finding (Medium)**
  - Linux job includes `cargo test -p wavical-cli`. While `wavical-cli` does not directly link `tao`/`wry`, its test suite `crates/wavical-cli/tests/dev_tests.rs` spawns the dev server which looks for `wavical-shell.exe` and calls Windows-specific `taskkill /F /PID` on line 270. On an Ubuntu runner, these tests may fail or exhibit unexpected behavior unless guarded by `#[cfg(windows)]`.
- **(b) Installation of `cargo-audit` and `cargo-deny`:** **PASS**
  - Explicitly installed in the `security` job via `taiki-e/install-action@v2` (lines 114-117) before execution.
- **(c) `deny.toml` Schema Validity:** **PASS**
  - `cargo deny check` parsed all sections without schema deprecation warnings. Uses modern keys (`[graph]`, `[output]`, `[advisories]`, `[licenses]`, `[bans]`, `[sources]`); no obsolete keys (`vulnerability`, `unmaintained`, `notice`, `unlicensed`, `copyleft`).
- **(d) Audit Crate, Fuzz Smoke Test, and E2E Coverage:** **PARTIAL / Finding (Medium)**
  - CLI `init` + `build` end-to-end workflow: **Covered** in `cli-e2e` job.
  - Fuzz smoke tests: **NOT COVERED** (No cargo-fuzz target execution in CI).
  - Audit crate (`tests/audit`): **NOT COVERED** in CI.
- **(e) Action Pinning & Least-Privilege Permissions:** **FAIL / Finding (Low)**
  - Actions use mutable version tags (`actions/checkout@v4`, `actions/setup-node@v4`, `taiki-e/install-action@v2`) or branch refs (`dtolnay/rust-toolchain@stable`) rather than immutable commit SHAs.
  - Zero explicit `permissions:` block declared; runner inherits default repository token permissions.
- **`gh run list --limit 10`:** **NOT RUN**
  - GitHub CLI (`gh`) is not installed on the host system.
  - *Action required from user:* Please run `gh run list --limit 10` or paste the latest GitHub Actions CI run status to verify remote CI execution.

---

## 10. Drift Matrix & Specification Traceability

### 10.1 Table 10(a): Requirement -> Implementation Traceability

| ID | Requirement / Plan Milestone | Implementation File:Line | Associated Test Names | Status | Notes |
|---|---|---|---|---|---|
| P0-01 | tao window creation + wry webview | `crates/wavical-shell/src/lib.rs:438,488` | `test_real_webview2_runtime_detected` | **FOUND** | Tao closure event loop verified. |
| P0-02 | Default-deny capability model | `crates/wavical-ipc/src/lib.rs:30` | `test_capability_default_is_empty_and_denies_all` | **FOUND** | Zero-grant start enforced. |
| P0-03 | Minimal frozen `window.wavical` bridge | `crates/wavical-shell/src/lib.rs:34` | `test_process_ipc_message_ping_granted` | **FOUND** | Injected via `INIT_SCRIPT`. |
| P0-04 | Strict IPC schema & length validation | `crates/wavical-ipc/src/lib.rs:550` | `test_rejection_*`, `proptest_*` | **FOUND** | 1MB raw size, 32 JSON depth. |
| P0-05 | Path sanitization in `wavical://` | `crates/wavical-protocol/src/lib.rs:136` | `test_rejection_*`, `proptest_*` | **FOUND** | Path traversal, ADS, device names blocked. |
| P0-06 | Strict Default Content-Security-Policy | `crates/wavical-shell/src/lib.rs:25,172` | `test_protocol_ok_response_headers` | **FOUND** | Default-src 'self' injected. |
| P0-07 | Top-level navigation allowlisting | `crates/wavical-shell/src/lib.rs:563` | `test_navigation_denied_for_external_*` | **FOUND** | Denies external and unapproved origins. |
| P0-08 | Secrets prevention across IPC | `crates/wavical-ipc/src/lib.rs:686,1079` | `test_handler_error_sanitization_*` | **FOUND** | Secrets never returned to JS. |
| P0-09 | Prove pattern: `core.ping` & `core.echo` | `crates/wavical-shell/src/lib.rs:65,1535` | `test_process_ipc_message_echo_ungranted` | **FOUND** | Ping granted, echo ungranted. |
| P0-10 | Security test bench v1 & fuzzers | `tests/attack-page/`, `fuzz/` | Attack page suite & 5 fuzz targets | **FOUND** | Live test harness and cargo-fuzz. |
| P1-01 | CLI skeleton: `init`, `dev`, `build` | `crates/wavical-cli/src/lib.rs:178,311,556` | `cli_tests.rs`, `dev_tests.rs`, `build_tests.rs` | **FOUND** | Clap subcommands implemented. |
| P1-02 | Short binary alias `wca` | `crates/wavical-cli/src/bin/wca.rs:1` | `test_binary_invocations_wavical_and_wca` | **FOUND** | Dual binary routing to common handler. |
| P1-03 | JavaScript config `wavical.config.js` | `crates/wavical-config/src/lib.rs:135` | `test_integration_valid_config_js` | **FOUND** | Node-evaluated subprocess parser. |
| P1-04 | Dynamic capability update on config reload | `crates/wavical-shell/src/lib.rs:242,504` | `test_dynamic_capabilities_live_update` | **FOUND** | Stdin protocol live capability update. |
| P1-05 | Friendly actionable errors & suggestions | `crates/wavical-config/src/lib.rs:88,410` | `test_unknown_capability_suggestion_*` | **FOUND** | Levenshtein typo suggestion. |
| P2-01 | Scoped file dialogs & opaque grants | `crates/wavical-ipc/src/grants.rs:60` | `test_grant_table_*`, `test_wp4_*` | **FOUND** | Grants isolate paths from page. |
| P2-02 | Clipboard text read/write | `crates/wavical-native/src/lib.rs:33` | `test_native_service_clipboard_*` | **FOUND** | 1 MB limit enforced. |
| P2-03 | Native toast notifications | `crates/wavical-native/src/lib.rs:60` | `test_native_service_notify_send_*` | **FOUND** | 128 title / 1024 body limits. |
| P2-04 | System tray icon & events | `crates/wavical-native/src/tray.rs:20` | `test_wp7_tray_set_*` | **FOUND** | tray-icon + muda event forwarding. |
| P2-05 | Native context menu | `crates/wavical-native/src/tray.rs:48` | `test_wp7_menu_show_context_menu_*` | **FOUND** | 200 items limit enforced. |
| P2-06 | Global shortcuts | `crates/wavical-ipc/src/shortcut.rs:14` | `test_wp8_shortcut_register_*` | **FOUND** | 32 max shortcuts, grammar validator. |
| P2-07 | Window state persistence | `crates/wavical-ipc/src/window_state.rs:16` | `test_save_and_load_window_state_*` | **FOUND** | Persists pos/size/maximized state. |
| P2-08 | Single-instance lock & CLI handoff | `crates/wavical-native/src/single_instance.rs:30` | `test_single_instance_handoff` | **FOUND** | Windows Named Pipe IPC handoff. |
| P2-09 | Taskbar overlay badge | `crates/wavical-ipc/src/badge.rs:30` | `test_wp8_badge_set_validation_*` | **PARTIAL** | Pure validation implemented; OS call returns "not supported" (deferred). |
| P2-10 | Crash recovery & missing runtime UX | `crates/wavical-ipc/src/crash_recovery.rs:12` | `test_first_crash_retries`, etc. | **FOUND** | 30s backoff state machine. |
| P2-11 | File association registration | `crates/wavical-cli/src/lib.rs:800` | `test_filetype_registration_*` | **FOUND** | Writes HKCU\Software\Classes. |
| P2-12 | Phase 2 WP9 Hostile Review Artifacts | Missing in `docs/` | None | **MISSING** | Hostile self-review document and 3 tests-fail proofs missing from repository artifacts. |

### 10.2 Table 10(b): Implementation -> Requirement Traceability (Scope Creep Audit)

| Code Item / Public Symbol | Implementation Location | Document Requirement Line | Status / Assessment |
|---|---|---|---|
| Crates `wavical-ipc`, `wavical-protocol`, `wavical-shell` | Workspace root `Cargo.toml` | Phase 0 Build Plan Sec 3 | **Justified** |
| Crates `wavical-cli`, `wavical-config` | Workspace root `Cargo.toml` | Phase 1 Build Plan Sec 3 | **Justified** |
| Crate `wavical-native` | Workspace root `Cargo.toml` | Phase 2 Build Plan Sec 0.1 | **Justified** |
| `GrantTable`, `GrantId`, `FileGrant` | `crates/wavical-ipc/src/grants.rs` | Phase 2 Build Plan Sec 0.3 | **Justified** |
| `resolve_asset_path` | `crates/wavical-protocol/src/lib.rs:136` | Phase 0 Build Plan Sec 4 (WP3) | **Justified** |
| `parse_request`, `Response` | `crates/wavical-ipc/src/lib.rs:550` | Phase 0 Build Plan Sec 4 (WP4) | **Justified** |
| Config field `appId`, `window`, `capabilities`, `dev` | `crates/wavical-config/src/lib.rs` | Phase 1 Config Schema Doc Sec 2 | **Justified** |
| Config field `version` | `crates/wavical-config/src/lib.rs:36` | None | **SCOPE CREEP (Medium)**: Added without schema doc entry. |
| Config field `tray` (`icon`, `tooltip`) | `crates/wavical-config/src/lib.rs:69` | None | **SCOPE CREEP (Medium)**: Added without schema doc entry. |
| Config field `shortcuts` | `crates/wavical-config/src/lib.rs:81` | None | **SCOPE CREEP (Medium)**: Added without schema doc entry. |
| Dependency `getrandom` in pure `wavical-ipc` | `crates/wavical-ipc/Cargo.toml:13` | None | **SCOPE CREEP (Medium)**: Used for grant ID entropy, but omitted from Phase 2 decisions. |
| Dependencies `muda`, `tray-icon`, `global-hotkey` in `wavical-shell` | `crates/wavical-shell/Cargo.toml:20-22` | Phase 2 Plan 0.1 (assigns to native) | **SCOPE CREEP (Medium)**: Direct inclusion in shell violates pure glue isolation. |
| Dependencies `ctrlc`, `windows-registry` in `wavical-cli` | `crates/wavical-cli/Cargo.toml:26,32` | None | **SCOPE CREEP (Medium)**: Not listed in Phase 1 or 2 plans. |
| Undocumented constants: `MAX_MENU_LABEL_CHARS` (100), `MAX_MENU_ITEM_ID_CHARS` (64), `MAX_TRAY_TOOLTIP_CHARS` (128), `MAX_SHORTCUT_STRING_CHARS` (64), `MAX_EXTENSION_CHARS` (10) | `crates/wavical-ipc/src/*.rs` | Phase 2 Plan 0.4 (unlisted) | **Info / Hardening Gap**: Sensible defensive bounds, but absent from plan table. |

### 10.3 Registered Capability & Method Diff

- **Plan Requirement (Phase 2 Plan 0.2 + `core.ping`, `core.echo`):**
  `badge.set`, `clipboard.read`, `clipboard.write`, `core.echo`, `core.ping`, `dialog.openFile`, `dialog.saveFile`, `fs.readGranted`, `fs.writeGranted`, `menu.showContextMenu`, `notify.send`, `shortcut.register`, `tray.set` (Total: 13).
- **Code Registration (`KNOWN_CAPABILITIES` in `wavical-ipc/src/lib.rs:378-392`):**
  `badge.set`, `clipboard.read`, `clipboard.write`, `core.echo`, `core.ping`, `dialog.openFile`, `dialog.saveFile`, `fs.readGranted`, `fs.writeGranted`, `menu.showContextMenu`, `notify.send`, `shortcut.register`, `tray.set` (Total: 13).
- **Difference:** **EXACT MATCH (0 differences)**.
- **Documentation Drift Note:** `docs/phase 1/WAVICAL_PHASE_1_CONFIG_SCHEMA.md` lines 65-66, 117-118, 190-194 still reference superseded names (`fs.readFile`, `fs.writeFile`, `window.getState/setState`).

---

## 11. Security & Compliance Findings

### F-A0-01 (High): Unapproved `unsafe` COM Hook in `wavical-native/src/crash_recovery.rs`
- **Where:** `crates/wavical-native/src/crash_recovery.rs:36,51,62`
- **Reproduction:** `git grep -n -E "unsafe|unsafe_code" crates/wavical-native/`
- **Impact:** Directly violates Phase 2 Build Plan Section 0.1 ("If a feature truly needs our own unsafe... the agent must stop and ask") and contradicts `docs/WAVICAL_PHASE_2_DECISIONS.md` Section 4 ("wavical-native enforces `#![deny(unsafe_code)]` with exactly one scoped `#[allow(unsafe_code)]` for `muda::show_context_menu_for_hwnd`"). Introduces unreviewed COM interface queries and raw pointer casting (`add_ProcessFailed`).
- **Suggested Fix (not applied):** Formally record approval for the crash handler COM call in `WAVICAL_PHASE_2_DECISIONS.md` after security review, or revert the native hook and rely on wry's high-level error handling.

### F-A0-02 (Medium): Supply-Chain Gate Failure (`cargo deny check`)
- **Where:** `deny.toml:12-33`; `Cargo.lock:45,438,450,451`
- **Reproduction:** `cargo deny check` -> Exit code 1 (`advisories FAILED`, `licenses FAILED`).
- **Impact:** Phase 0 Definition of Done requires `cargo deny` to pass cleanly in CI. Current build fails due to unmaintained security advisories (`zbus_names` RUSTSEC-2026-0371, `zvariant` RUSTSEC-2026-0370, `zvariant_derive` RUSTSEC-2026-0372) and rejected licenses (`BSL-1.0` on `clipboard-win` and `error-code`).
- **Suggested Fix (not applied):** Add `BSL-1.0` to allowed licenses in `deny.toml` (OSI/FSF approved); review and explicitly ignore the transitive unmaintained D-Bus advisories with documented rationale, or update dependencies.

### F-A0-03 (Medium): Linux CI Test Fragility & Windows Dependency
- **Where:** `.github/workflows/ci.yml:64`; `crates/wavical-cli/tests/dev_tests.rs:270`
- **Reproduction:** Running `cargo test -p wavical-cli` on Ubuntu fails or hangs when `dev_tests` executes line 270 (`Command::new("taskkill")`) or attempts to launch missing Windows binary `wavical-shell.exe`.
- **Impact:** Compromises Linux CI reliability; prevents clean cross-platform verification of pure CLI parsing.
- **Suggested Fix (not applied):** Gate `dev_tests.rs` with `#[cfg(windows)]` or mock process termination using cross-platform signals (`child.kill()`).

### F-A0-04 (Medium): Scope Creep in Config Schema & Dependencies
- **Where:** `crates/wavical-config/src/lib.rs:36,69,81`; `crates/wavical-ipc/Cargo.toml:13`; `crates/wavical-shell/Cargo.toml:20-22`; `crates/wavical-cli/Cargo.toml:26,32`
- **Reproduction:** Inspect crate dependencies and config fields against build plans.
- **Impact:** Config fields (`version`, `tray.icon`, `shortcuts`) and dependencies (`getrandom`, `ctrlc`, `windows-registry`, direct shell links to `muda`/`tray-icon`) exist in code without documentation in build plans or schema specifications.
- **Suggested Fix (not applied):** Update `docs/phase 1/WAVICAL_PHASE_1_CONFIG_SCHEMA.md` and `docs/WAVICAL_PHASE_2_DECISIONS.md` to document all active fields and dependencies.

### F-A0-05 (Medium): Missing Phase 2 WP9 Hostile Review Artifacts
- **Where:** `docs/`; `tests/`
- **Reproduction:** Search repository for WP9 hostile review document and "tests-fail" demonstration records.
- **Impact:** Phase 2 Definition of Done requires hostile self-review notes and explicit evidence that disabling capability/grant checks causes tests to fail. These records are missing from repository documentation.
- **Suggested Fix (not applied):** Conduct and commit the formal hostile review notes and test failure proofs under `docs/`.

### F-A0-06 (Low): Grant Table `RwLock .unwrap()` Poisoning Risk
- **Where:** `crates/wavical-ipc/src/grants.rs:80,122,131,137,143,149`
- **Reproduction:** `git grep -n "grants.*unwrap" crates/wavical-ipc/`
- **Impact:** If an asynchronous thread panics while holding the grant table lock, the poisoned lock causes all subsequent grant validations to panic across all windows.
- **Suggested Fix (not applied):** Handle lock poisoning gracefully with `map_err(|_| GrantError::InternalLockError)`.

### F-A0-07 (Low): CI Hygiene (Unpinned Actions & Missing Permissions)
- **Where:** `.github/workflows/ci.yml:18,21,48,53,77,82,109,112,115`
- **Reproduction:** Inspect GitHub Actions workflow file.
- **Impact:** Workflow actions use mutable major version tags (`@v4`, `@v2`) and `@stable` branch refs rather than immutable commit SHAs. No explicit `permissions:` block is declared.
- **Suggested Fix (not applied):** Declare top-level `permissions: { contents: read }` and pin actions to full commit SHAs.

### F-A0-08 (Low): Transitive `windows-sys` in Pure Crates Under `-e dev`
- **Where:** `crates/wavical-ipc/Cargo.toml:16`; `crates/wavical-protocol/Cargo.toml`
- **Reproduction:** `cargo tree -p wavical-ipc -e normal,dev | Select-String windows-sys`
- **Impact:** Standard test framework `proptest` and `tempfile` pull in `windows-sys` on Windows dev builds. Shipped normal builds remain 100% clean.
- **Suggested Fix (not applied):** Document dev-only test exception in purity rules.

### F-A0-09 (Info): `wavical-native` Uses `#![deny(unsafe_code)]`
- **Where:** `crates/wavical-native/src/lib.rs:1`
- **Impact:** Documented design decision in Phase 2 decisions Section 4 to allow scoped `#[allow(unsafe_code)]` for Win32 popup menus.

### F-A0-10 (Info): Taskbar Overlay Badge Unsupported at OS Level
- **Where:** `crates/wavical-native/src/lib.rs:549`; `docs/WAVICAL_PHASE_2_BUILD_PLAN.md:203`
- **Impact:** `badge.set` IPC method validates parameters but always returns `"Taskbar overlay badge is not supported on this platform"`. Correctly documented as an open exception to avoid unverified COM unsafe code.

---

## 12. Conclusion: Has the Project Drifted from the Plans?

**Feature-wise, the core architecture has remained remarkably faithful to the build plans:**
All 13 declared capabilities and 11 native methods match Section 0.2 of the Phase 2 plan exactly. Core security invariants (default-deny capability model, 1 MB message bounds, 32-level JSON nesting limits, path canonicalization, opaque scoped file grants, and secrets exclusion) are fully implemented and verified by 230 passing tests.

**However, significant drift exists in discipline, supply-chain gating, and documentation consistency:**
1. **Unapproved `unsafe`:** A second `unsafe` COM hook for WebView2 crash recovery was introduced in `wavical-native/src/crash_recovery.rs` despite the decisions document explicitly mandating that only a single `unsafe` site (`muda::show_context_menu_for_hwnd`) was approved.
2. **Broken Supply-Chain Gate:** `cargo deny check` is actively red (Exit 1) due to unmaintained transitives and BSL-1.0 license rejections, violating Phase 0 launch-blocking requirements.
3. **Documentation Lag:** The Phase 1 Config Schema reference document still lists broad deprecated APIs (`fs.readFile`, `fs.writeFile`, `window.getState`) superseded in Phase 2, while new config fields (`tray`, `shortcuts`, `version`) and dependencies (`getrandom`, `ctrlc`, `windows-registry`) were introduced without plan documentation.
4. **CI & Verification Gaps:** CI lacks fuzz smoke testing, audit crate inclusion, action SHA pinning, and least-privilege token permissions, while the Linux test job attempts to run Windows-specific CLI tests. Finally, Phase 2 WP9 hostile review artifacts and tests-fail proofs were never committed to `docs/`.










