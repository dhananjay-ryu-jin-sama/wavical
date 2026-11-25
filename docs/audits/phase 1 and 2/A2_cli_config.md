# Audit Report A2: Phase 1 CLI & Configuration (`wavical-cli` & `wavical-config`)

- **Target:** `crates/wavical-cli`, `crates/wavical-config`, `tests/audit`
- **Specification:** `docs/phase 1/WAVICAL_PHASE_1_BUILD_PLAN.md` (WP1 to WP5), `docs/phase 1/WAVICAL_PHASE_1_CONFIG_SCHEMA.md`, `docs/WAVICAL_SETUP.md`
- **Mode:** AUDIT ONLY (No modifications to crates, examples, or existing workspace files)
- **Date:** 2026-10-26
- **Auditor:** Independent Security Auditor & QA Engineer

---

## Executive Summary & Status Counts

| Status | Count | Description |
|---|---|---|
| **PASS** | **17** | Verified strictly against build plan specifications via automated test execution and code analysis |
| **PARTIAL** | **5** | Partially compliant or documentation/specification divergence |
| **FAIL** | **4** | Security vulnerabilities, data integrity gaps, or architectural issues caught by tests |
| **NOT RUN** | **1** | Historical Node LTS matrix testing (only host Node v26.4.0 is installed; no nvm/fnm on host) |
| **Total Requirements** | **27** | Total requirements evaluated across Phase 1 WP1 through WP5 |

---

## Commands to Run Audit Tests

### Standard CI Run (All Green)
```powershell
cargo test -p wavical-audit --test part4_cli
```
> **Evidence:** `6 passed; 0 failed; 4 ignored; 0 measured; finished in 2.96s`
> Full workspace audit test suite (`part1_protocol`, `part2_ipc`, `part3_shell`, `part4_cli`):
> `cargo test -p wavical-audit` -> `35 passed; 0 failed; 8 ignored; finished in 50.4s`.

### Run Including Known Finding Regressions
```powershell
cargo test -p wavical-audit --test part4_cli -- --ignored
```
> **Evidence:**
> `4 failed; 0 passed; finished in 0.96s`
> Confirms `FINDING-C1` (stdout pollution), `FINDING-C2` (stale file retention), `FINDING-C3` (dangerous outdir), and `FINDING-C4` (sensitive dotfile leak) deterministically.

---

## 1. Requirements Matrix: Phase 1 (WP1 to WP5)

| Req ID | Phase 1 Acceptance Item | Specification / Requirement | Status | Evidence & Verification Reference |
| :--- | :--- | :--- | :---: | :--- |
| **WP1-01** | Config Validator | `wavical-config` parses `.js` config via Node and rejects invalid configs with clear messages | **PASS** | `validate_config` shells out to `node -e` with timeout and validates schema ([`crates/wavical-config/src/lib.rs:613-633`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-config/src/lib.rs#L613-L633)). Test: `test_cli_config_validation_battery`. |
| **WP1-02** | Capability Integration | Capabilities validated against `wavical-ipc` registry types as single source of truth | **PASS** | [`crates/wavical-config/src/lib.rs:484`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-config/src/lib.rs#L484) calls `wavical_ipc::known_capabilities()`. Unknown capabilities rejected with Levenshtein suggestions. |
| **WP1-03** | Capability Typo Suggestion | Rejects unknown capabilities with helpful "did you mean" suggestion | **PASS** | `closest_match` suggests `core.echo` for `core.echo2` ([`crates/wavical-config/src/lib.rs:351-371`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-config/src/lib.rs#L351-L371)). Test: `test_cli_config_validation_battery`. |
| **WP1-04** | Node Execution Timeout | Subprocess execution terminates on timeout (>5 seconds) and kills child process | **PASS** | `execute_node_config_script` uses `mpsc::channel().recv_timeout(5s)` and calls `child.kill()` ([`crates/wavical-config/src/lib.rs:524-597`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-config/src/lib.rs#L524-L597)). Test: `test_cli_node_execution_battery`. |
| **WP1-05** | Stdout Isolation | User stdout pollution in config (`console.log`) must not break JSON extraction | **FAIL** | **FINDING-C1**: `execute_node_config_script` executes `console.log(JSON.stringify(...))`. Calling `console.log()` inside config corrupts stdout and triggers `MalformedJson`. Test: `test_cli_config_stdout_pollution_isolation`. |
| **WP2-01** | `wavical init <dir>` | Creates directory and scaffolds `wavical.config.js`, `src/index.html`, `src/styles.css`, `.gitignore` | **PASS** | [`crates/wavical-cli/src/lib.rs:221-285`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L221-L285); Scaffolded files verified in `test_cli_init_behavior_with_safety_sentinel`. |
| **WP2-02** | `wca` Alias Parity | `wca` functions identically to `wavical` | **PASS** | Both `wavical.rs` and `wca.rs` invoke `wavical_cli::main_entry()`. Subcommands and flags identical. Test: `test_cli_wca_alias_parity`. |
| **WP2-03** | Scaffold Validity | The scaffolded `wavical.config.js` is valid and passes WP1 validator | **PASS** | `wavical_config::validate_config` passes on newly scaffolded project ([`crates/wavical-cli/src/lib.rs:197-201`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L197-L201)). |
| **WP2-04** | Non-empty Init Protection | Refuses to overwrite non-empty directory unless `--force` is specified | **PASS** | [`crates/wavical-cli/src/lib.rs:232-234`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L232-L234); Refuses duplicate init; `--force` overwrites scaffold without deleting non-colliding files. Test: `test_cli_init_behavior_with_safety_sentinel`. |
| **WP2-05** | AppId Sanitization | Derived `appId` passes validation for odd directory names (spaces, symbols) | **PASS** | `sanitize_app_id` normalizes non-alphanumeric characters to hyphens ([`crates/wavical-cli/src/lib.rs:158-175`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L158-L175)). |
| **WP3-01** | `wavical dev` Startup | Starts shell and serves app from `src/` | **PASS** | Spawns `wavical-shell` with `--asset-root`, `--capabilities`, and window parameters ([`crates/wavical-cli/src/lib.rs:402-440`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L402-L440)). |
| **WP3-02** | Capability Enforcement | Declarations in config are passed and enforced by shell | **PASS** | `config.granted_capabilities()` serialized as JSON and passed via `--capabilities` flag to shell ([`crates/wavical-cli/src/lib.rs:400,412`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L400)). |
| **WP3-03** | Live Asset Hot Reload | Asset modifications in `src/` trigger live page reload in webview | **PASS** | `notify` watcher detects changes in `src/` and sends `{"action":"reload"}` via stdin to shell ([`crates/wavical-cli/src/lib.rs:530-542`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L530-L542)). |
| **WP3-04** | Live Config Reload | Edits to `wavical.config.js` re-evaluate config and update capability grants | **PASS** | Valid config changes send `{"action":"update_capabilities"}` via stdin ([`crates/wavical-cli/src/lib.rs:509-521`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L509-L521)). |
| **WP3-05** | Invalid Config Hot Guard | Invalid config edits log warning and preserve existing running config | **PASS** | When `validate_config` fails on reload, old config remains active with warning ([`crates/wavical-cli/src/lib.rs:522-525`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L522-L525)). |
| **WP3-06** | Ctrl+C Process Cleanup | Clean exit on Ctrl+C; no orphan shell or Node processes remain | **PASS** | `terminate_child` sends `{"action":"exit"}`, kills child shell, and calls `wait()` ([`crates/wavical-cli/src/lib.rs:551-558`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L551-L558)). |
| **WP3-07** | TCP Dev Port Inactive | `dev.port` in config is not bound to network socket in Phase 1 | **PASS** | Code analysis & `netstat -ano`: dev server uses stdin pipe to shell; no TCP socket is opened on port 7890. |
| **WP4-01** | `wavical build` Output | Copies `src/` to `dist/` and generates `wavical-manifest.json` | **PASS** | [`crates/wavical-cli/src/lib.rs:707-733`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L707-L733); Manifest verified in `test_cli_build_manifest_and_determinism`. |
| **WP4-02** | Manifest Format & Hashes | Valid JSON; SHA-256 for all assets; relative forward-slash paths; manifest omitted | **PASS** | Verified via `sha256_file` comparison against real file bytes ([`crates/wavical-cli/src/lib.rs:638-650`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L638-L650)). Test: `test_cli_build_manifest_and_determinism`. |
| **WP4-03** | Stale File Cleanup | Overwrite / cleanup of stale files in `dist/` on subsequent builds | **FAIL** | **FINDING-C2**: `run_build` copies files into `out_dir` without clearing old files. A file deleted from `src/` remains orphaned in `dist/`. Test: `test_cli_build_removes_stale_files_from_outdir`. |
| **WP4-04** | Dangerous `--outdir` Guard | Must refuse dangerous paths (`.`, project root, `src`, `..`, parent) | **FAIL** | **FINDING-C3**: `run_build` does not validate `outdir`. Building with `--outdir .` or `--outdir src` overwrites project files. Test: `test_cli_build_refuses_dangerous_outdir_paths`. |
| **WP4-05** | Sensitive File Exclusion | Must exclude `.env`, `.git`, `.DS_Store`, `Thumbs.db` from production `dist/` | **FAIL** | **FINDING-C4**: `copy_and_hash_assets` copies every file in `src/` indiscriminately. Adding `.env` inside `src/` copies secrets into `dist/`. Test: `test_cli_build_excludes_sensitive_files`. |
| **WP5-01** | E2E Workflow Pass | Full CLI workflow (`init` -> `dev` -> `build`) functions seamlessly | **PASS** | Verified via manual CLI execution and automated CI runner in `.github/workflows/ci.yml:95-100`. |
| **WP5-02** | Error Quality & Exit Codes | Clear error messages; non-zero exit codes; no panics or raw debug traces | **PASS** | Errors use `thiserror` enums and return `ExitCode::FAILURE`. Test: `test_cli_config_validation_battery`. |
| **WP5-03** | Trust Model Documentation | Document that `wavical.config.js` executes arbitrary code with user rights | **PARTIAL** | **FINDING-C5**: Discussed in `WAVICAL_PHASE_1_BUILD_PLAN.md` Section 5, but omitted from user-facing `README.md` and `SECURITY.md`. |
| **WP5-04** | Node Requirement Documentation | Document that Node.js is required for CLI config parsing | **PARTIAL** | **FINDING-C6**: `docs/WAVICAL_SETUP.md` and `README.md` omit Node.js from prerequisites, causing runtime failures when Node is not installed. |
| **WP5-05** | Config Schema Doc Drift | Deprecated capability names (`fs.readFile`, `window.getState`) in schema doc | **PARTIAL** | `WAVICAL_PHASE_1_CONFIG_SCHEMA.md` lists `fs.readFile` and `window.getState`, which fail validation in current code (`fs.readGranted`). |

---

## 2. Findings Summary

| ID | Severity | Component | Title | Remediation |
| :--- | :---: | :--- | :--- | :--- |
| **FINDING-C1** | **MEDIUM** | `wavical-config` | Config Stdout Pollution (`console.log`) Corrupts JSON Parsing | Wrap config evaluation in Node script with an isolated stdout channel (e.g. fd 3 or custom delimiter tokens) or evaluate in an isolated VM context. |
| **FINDING-C2** | **MEDIUM** | `wavical-cli` | Stale Asset Retention in Production `dist/` Directory | Clear existing output directory before copying, or remove files present in `dist/` that no longer exist in `src/`. |
| **FINDING-C3** | **MEDIUM** | `wavical-cli` | Dangerous `--outdir` Paths (`.`, `src`, `..`) Accepted Without Validation | Canonicalize `outdir` and reject if it equals project root, parent directory, or is inside `src/`. |
| **FINDING-C4** | **MEDIUM** | `wavical-cli` | Sensitive Files (`.env`, `.git`) Copied Into Production Distributable | Add exclusion filters in `copy_and_hash_assets` for `.env*`, `.git*`, `*.map`, `.DS_Store`, and `Thumbs.db`. |
| **FINDING-C5** | **LOW** | `docs`, `README.md` | Omission of Config Execution Trust Boundary Documentation | Add security disclosure in `README.md` and `SECURITY.md` warning that `wavical.config.js` executes with developer permissions. |
| **FINDING-C6** | **LOW** | `docs`, `README.md` | Node.js Prerequisite Omitted from Developer Setup Guides | Add Node.js (>= 14.8.0) to prerequisites in `docs/WAVICAL_SETUP.md` and `README.md`. |

---

## 3. Top 5 Findings Deep Dive

```
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│                               TOP 5 AUDIT FINDINGS (PHASE 1 CLI)                                 │
├────┬──────────────┬──────────┬───────────────────────┬───────────────────────────────────────────┤
│ #  │ Finding ID   │ Severity │ Target Location       │ Title & Summary                           │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 1  │ FINDING-C1   │ MEDIUM   │ crates/wavical-config │ User console.log in wavical.config.js     │
│    │              │          │ lib.rs:530-536        │ Pollutes Stdout & Breaks JSON Extraction  │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 2  │ FINDING-C2   │ MEDIUM   │ crates/wavical-cli    │ wavical build Retains Stale / Deleted     │
│    │              │          │ lib.rs:707-710        │ Files in dist/ Output Directory           │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 3  │ FINDING-C3   │ MEDIUM   │ crates/wavical-cli    │ wavical build Accepts Dangerous --outdir  │
│    │              │          │ lib.rs:696-705        │ Paths (., src, ..) Overwriting Sources    │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 4  │ FINDING-C4   │ MEDIUM   │ crates/wavical-cli    │ Sensitive Files (.env, .git) in src/      │
│    │              │          │ lib.rs:595-653        │ Copied Unconditionally into dist/         │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 5  │ FINDING-C5   │ LOW      │ README.md, docs       │ Missing Config Code Execution Trust       │
│    │              │          │ SECURITY.md           │ Model Disclosure in User Documentation    │
└────┴──────────────┴──────────┴───────────────────────┴───────────────────────────────────────────┘
```

### 1. FINDING-C1 (Medium): Config Stdout Pollution Breaks JSON Extraction
* **Vulnerability:** [`crates/wavical-config/src/lib.rs:530-536`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-config/src/lib.rs#L530-L536)
  ```rust
  cmd.arg("-e").arg(
      "console.log(JSON.stringify(await import('./wavical.config.js').then(m => m.default)))",
  )
  ```
* **Impact:** If `wavical.config.js` prints diagnostic logs via `console.log("...")` or a third-party module emits to standard output, the stdout buffer contains mixed text before the JSON payload. `parse_config_json` fails with `MalformedJson("expected value at line 1 column 1")`.
* **Reproduction:** Run `cargo test -p wavical-audit --test part4_cli test_cli_config_stdout_pollution_isolation -- --exact`.

### 2. FINDING-C2 (Medium): Stale File Retention in Production `dist/`
* **Vulnerability:** [`crates/wavical-cli/src/lib.rs:707-710`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L707-L710)
* **Impact:** `run_build` invokes `copy_and_hash_assets` into `out_dir` without wiping previous contents. When a sensitive or temporary file is deleted from `src/`, it remains present in `dist/` on subsequent builds. It will be packaged and shipped in the distributable, despite not appearing in `wavical-manifest.json`.
* **Reproduction:** Run `cargo test -p wavical-audit --test part4_cli test_cli_build_removes_stale_files_from_outdir -- --exact`.

### 3. FINDING-C3 (Medium): Dangerous `--outdir` Paths Accepted Without Validation
* **Vulnerability:** [`crates/wavical-cli/src/lib.rs:696-705`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L696-L705)
* **Impact:** Passing `--outdir .` or `--outdir src` causes `run_build` to copy assets over the project root or source directory. `wavical-manifest.json` is generated directly inside `src/` or the project root.
* **Reproduction:** Run `cargo test -p wavical-audit --test part4_cli test_cli_build_refuses_dangerous_outdir_paths -- --exact`.

### 4. FINDING-C4 (Medium): Sensitive Files (`.env`, `.git`) Copied into Distributable
* **Vulnerability:** [`crates/wavical-cli/src/lib.rs:595-653`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-cli/src/lib.rs#L595-L653)
* **Impact:** `copy_and_hash_assets` copies all files found by `read_dir`. If an author places a local `.env` or credential file inside `src/`, `wavical build` includes it in `dist/` and computes a hash for it in `wavical-manifest.json`, leaking production secrets.
* **Reproduction:** Run `cargo test -p wavical-audit --test part4_cli test_cli_build_excludes_sensitive_files -- --exact`.

### 5. FINDING-C5 (Low): Missing Trust Model Documentation for JavaScript Config
* **Vulnerability:** [`README.md`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/README.md), [`SECURITY.md`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/SECURITY.md)
* **Impact:** Because `wavical.config.js` is an executable JavaScript module executed during `dev` and `build`, downloading a third-party project and running `wavical dev` executes arbitrary code on the developer's machine with full user permissions. This threat model is discussed internally in `WAVICAL_PHASE_1_BUILD_PLAN.md` but is absent from user documentation.

---

## 4. Detailed Audit Area Analysis

### A. Environment & Setup
- **Installed Node Version:** `v26.4.0` (Host platform: Windows 11).
- **Supported Versions Claimed:** None declared in documentation or code. Top-level await in CLI evaluation (`await import(...)`) requires Node.js >= 14.8.0.
- **Node Missing from PATH:** Executing with empty `PATH` fails cleanly with non-zero exit code (1) and friendly error message: `error: invalid config: Node subprocess failed: Failed to spawn Node process: The system cannot find the file specified. (os error 2)`. Zero panics observed.

### B. ESM vs CommonJS & Scaffolding
- `wavical init` scaffolds `wavical.config.js` using `export default { ... }`.
- `package.json` is **not** scaffolded. Node.js evaluates the module via dynamic `import(...)` within `-e`, which successfully treats `.js` as an ES module in modern Node versions.

### C. Config Validation Battery Results
- **Missing `appId`:** Rejected with `missing required field 'appId'`.
- **Empty `appId`:** Rejected with `missing required field 'appId'`.
- **Invalid `appId` (`../x`, `a b`, `🦀`, `.`, `..`):** Rejected with `appId must be alphanumeric (a-z, 0-9, -, _), got '...'`.
- **Upper-case / Mixed-case `appId` (`A_B`):** Accepted (`is_ascii_alphanumeric` permits uppercase).
- **Unknown Top-Level Field:** Rejected by `deny_unknown_fields` with `unknown field '...'`.
- **Unknown Window Field:** Rejected with `unknown field '...'`.
- **Unknown Capability with Typo (`core.echo2`):** Rejected with `unknown capability 'core.echo2'; did you mean 'core.echo'?`.
- **Unknown Capability without Match (`unrelated.api`):** Rejected without suggestion: `unknown capability 'unrelated.api' (known: ...)`.
- **Invalid Width / Height / Port Types:** Rejected with typed deserialization errors (e.g. `invalid type: string "800", expected u32`).
- **Infinite Loop in Config:** Process killed after timeout (default 5s, configurable via `WAVICAL_NODE_TIMEOUT`).

### D. Build & Manifest Verification
- `wavical build` outputs `dist/wavical-manifest.json`.
- All asset hashes independently verified against real file SHA-256 byte digests.
- File paths in manifest are formatted with relative forward-slashes (`index.html`, `styles.css`).
- Build output is deterministic across repeated runs on identical source trees.

---

## 5. Summary

- **Total Test Cases Executed in `wavical-audit`:** 35 passing tests, 8 documented finding regressions, zero unexpected failures.
- **CI Status:** `cargo test -p wavical-audit` is clean green (`exit 0`).
- **Safety Verified:** All tests executed inside isolated `tempdir` instances with sentinel assertion verification (`SENTINEL_DO_NOT_DELETE.txt` preserved).
