# Audit Report A9: Phase 4.5 (Network Modes)

- **Target:** `crates/wavical-netpolicy`, `crates/wavical-shell`, `crates/wavical-cli`, `crates/wavical-config`, `crates/wavical-ai`, `docs/`
- **Specification:** `docs/phase 4.5/WAVICAL_PHASE_4_5_NETWORK_MODES_PLAN.md` (Sections 1–3)
- **Mode:** AUDIT ONLY (Black-box tests in `tests/audit/tests/part9_network_modes.rs`)
- **Date:** 2026-11-21
- **Auditor:** Independent Security Auditor & QA Engineer

---

## 1. Executive Summary

| Status | Count | Notes |
|---|:---:|---|
| **PASS** | **9** | Pure policy crate (`wavical-netpolicy`), shell mode lock & serve-time hash check, CSP per mode (`ultra-safe` == Phase 0 `DEFAULT_CSP`), inline script hashing & extraction, `wavical vendor` & `--check`, `allowlist suggest` & `--record`, baseline in all 3 modes, zero cert-error bypasses, 14/14 mutation proofs killed |
| **PARTIAL** | **3** | `dev.url` (pure validator & shell enforced, config schema missing), `ultra-safe` exfiltration (residual risks documented; WebRTC/preconnect confirmed unclosable in WebView2), docs (`NETWORK_MODES.md` documents WP3 build commands not yet implemented) |
| **FAIL** | **3** | WP3 CLI build subcommands (`build --allow-open`, `build-safe`, `build-ultra-safe`), WP3 `wavical.config.js` fields (`dev.url`, `build.frontendDir`, `ultraSafe.blockNetworkCapabilities`), WP3 manifest fields & build warnings |
| **NOT RUN** | **1** | Live GUI WebView2 walkthrough of the reference app across `open` / `safe` / `ultra-safe` and live `window.remoteUrl` |

---

## 2. Requirements Matrix

| Area | Requirement | Status | Evidence |
|---|---|---|---|
| **1. Drift & Certs** | Plan 1.1–1.15 alignment; zero certificate bypasses | **PARTIAL** | Grep for `ignore_certificate`, `danger_accept`, `accept_invalid_certs` = 0 hits. WP3 CLI build commands & config fields paused (`F-A9-01`..`F-A9-05`). |
| **2. Commands** | 6 commands + `wca` aliases; build refusal without `--allow-open` | **FAIL** | `dev`, `dev-safe`, `dev-ultra-safe`, `allowlist suggest`, `vendor` present; `build --allow-open`, `build-safe`, `build-ultra-safe` missing (`F-A9-01`). |
| **3. Mode Lock** | Built app cannot be loosened by flags/env/edits; missing manifest -> `ultra-safe` | **PASS** | `a9_03_mode_lock_and_serve_time_hash_check` (`part9_network_modes.rs`). Tampered `wavical.allowlist.json` falls back to `ultra-safe`. |
| **4. Allowlist Parser** | 80+ inputs covering all Plan 1.4 rules & Plan 1.9 limits | **PASS** | `a9_01_and_02_allowlist_80_plus_inputs_and_matcher_confusion` (81+ cases). |
| **5. Matcher** | Look-alikes, punycode, wildcards, ports, `ws`/`wss` | **PASS** | `policy_tests::matcher_table_at_least_60_cases` + `part9_network_modes.rs`. |
| **6. CSP per Mode** | Per-mode CSP; `ultra-safe` == Phase 0 `DEFAULT_CSP` | **PASS** | `ultra_safe_csp_matches_phase_0_default_csp_verbatim`. |
| **7. Inline Scripts** | `safe` hashing; `ultra-safe` extraction order & `type="module"` | **PASS** | `a9_04_inline_scripts_extraction_and_ultra_safe_exfiltration`. |
| **8. Vendor** | Path escapes, reserved names, 20MB/500 limits, unpinned refusal, `--check`, licenses | **PASS** | `a9_05_vendor_security_and_dev_url_and_suggest` + `vendor_and_extract_tests`. |
| **9. `dev.url`** | Loopback+port only; dev commands only; ignored in builds | **PARTIAL** | `validate_dev_url` & shell enforcement pass; `wavical-config` `dev.url` field missing (`F-A9-02`). |
| **10. Ultra-safe Exfil** | Exfil routes blocked; residual risks match docs; capability report | **PARTIAL** | Server receives 0 hits (`part9_network_modes.rs`); `ultraSafe.blockNetworkCapabilities` missing (`F-A9-02`). |
| **11. Serve-time Hash** | Tampered built file returns 404 and logs | **PASS** | `a9_03_mode_lock_and_serve_time_hash_check`. |
| **12. Baseline & Remote UI** | Baseline in all 3 modes; `remoteUi.bridge` subset; `allowlist suggest` | **PASS** | `a9_06_baseline_and_remote_ui_in_all_modes` + `a9_05`. |
| **13. Mutation Proofs** | 14/14 targeted mutations killed | **PASS** | All 14 WP8 mutations exited `101` (failed) and were restored. |
| **14. Docs** | `NETWORK_MODES.md`, `VENDOR.md`, `SECURITY_MODEL.md`, `CHANGELOG.md` | **PARTIAL** | Docs match implemented behavior except WP3 build/config features (`F-A9-06`). |

---

## 3. Findings by Severity

| ID | Severity | Title | Where | Recommendation |
|---|---|---|---|---|
| **F-A9-01** | **High** | WP3 Build Subcommands (`build --allow-open`, `build-safe`, `build-ultra-safe`) Not Implemented | `crates/wavical-cli/src/lib.rs` | Complete WP3 after resolving the Phase 1–4 `wavical build` test migration (`build-ultra-safe`). |
| **F-A9-02** | **High** | WP3 Config Fields (`dev.url`, `build.frontendDir`, `ultraSafe.blockNetworkCapabilities`) Missing | `crates/wavical-config/src/lib.rs` | Add fields with `deny_unknown_fields` and validation per Plan 1.12–1.13. |
| **F-A9-03** | **High** | WP3 Manifest Fields (`mode`, `allowlistSha256`, `inlineScriptHashes`, `networkCapabilities`) & Plan 1.14 Build Warnings Missing | `crates/wavical-cli/src/lib.rs` | Populate fields in `WavicalManifest` during `build*` and emit Plan 1.14 warnings. |
| **F-A9-04** | **Medium** | Missing `.wavical/` Last-Dev-Mode Mismatch Warning & One-Time `wavical dev` Notice | `crates/wavical-cli/src/lib.rs` | Track last dev mode in `.wavical/` and print one-time open-mode notice. |
| **F-A9-05** | **Medium** | Missing Example `wavical.allowlist.json` Error in `dev-safe` When File Absent | `crates/wavical-cli/src/lib.rs` | Validate `wavical.allowlist.json` existence in `dev-safe` and print Plan 1.4 example if missing. |
| **F-A9-06** | **Low** | Documentation Drift for Unimplemented WP3 Commands/Config | `docs/NETWORK_MODES.md` | Complete WP3 or mark WP3 commands as pending until WP3 lands. |

---

## 4. NOT RUN List (Manual GUI Checks)
- Live WebView2 rendering of the reference app in `open`, `safe` (`--record` -> `allowlist suggest`), and `ultra-safe` (`wavical vendor` + inline extraction).
- Live WebView2 `window.remoteUrl` navigation & untrusted self-signed certificate fail-closed UI verification.

Commands to run automated audit suite:
```powershell
cargo test -p wavical-audit --test part9_network_modes --offline
```

---

## 5. Top 5 Priorities
1. **Resolve Phase 1–4 `wavical build` test migration & complete WP3** (`F-A9-01`, `F-A9-02`, `F-A9-03`).
2. **Add `.wavical/` dev-mode tracking & one-time `wavical dev` open-mode notice** (`F-A9-04`).
3. **Require `wavical.allowlist.json` with example output in `dev-safe`** (`F-A9-05`).
4. **Run the live GUI manual checklist** on the reference app across all 3 modes.
5. **Resolve pre-existing `README.md` `FINDING-C5`/`FINDING-C6`** in `part4_cli` when touching docs.
