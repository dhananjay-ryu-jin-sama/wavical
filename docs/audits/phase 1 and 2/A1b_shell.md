# Audit Report A1b: Phase 0 Shell (`wavical-shell`)

- **Target:** `crates/wavical-shell`, `tests/integration`, `tests/attack-page`, `tests/audit`
- **Specification:** `docs/phase 0/WAVICAL_PHASE_0_BUILD_PLAN.md` (WP2, WP5, WP6, WP7, WP8), `docs/phase 1/SECURITY_MODEL.md`
- **Mode:** AUDIT ONLY (No modifications to crates, examples, or existing workspace files)
- **Date:** 2026-10-26
- **Auditor:** Independent Security Auditor & QA Engineer

---

## Executive Summary & Status Counts

| Status | Count | Description |
|---|---|---|
| **PASS** | **18** | Verified strictly against build plan specifications via automated test execution and code analysis |
| **PARTIAL** | **4** | Partially compliant, divergence in specifications, or defense-in-depth gaps |
| **FAIL** | **3** | Direct security rule violation, runtime hang, or architectural gap caught by test suite |
| **Total Requirements** | **25** | Total items evaluated across WP2, WP5, WP6, WP7, and WP8 |

---

## Commands to Run Audit Tests

### Standard CI Run (All Green)
```powershell
cargo test -p wavical-audit --test part3_shell
```
> **Evidence:** `12 passed; 0 failed; 1 ignored; 0 measured; finished in 0.08s`
> Full workspace audit test suite (`part1_protocol`, `part2_ipc`, `part3_shell`):
> `cargo test -p wavical-audit` -> `29 passed; 0 failed; 4 ignored; finished in 45.6s`.

### Run Including Known Finding Regressions
```powershell
cargo test -p wavical-audit --test part3_shell -- --ignored
```
> **Evidence:**
> `test test_shell_bridge_script_only_exposes_invoke_and_no_other_globals ... FAILED`
> `panicked at tests\audit\tests\part3_shell.rs:98:5: Phase 0 spec requires only 'invoke' on window.wavical; 'onMenu' is exposed`

---

## 1. Requirements Matrix (WP2, WP5, WP6, WP7, WP8)

| Req ID | Phase 0 Spec / Component | Requirement Summary | Status | Evidence & Verification Reference |
| :--- | :--- | :--- | :---: | :--- |
| **WP2-01** | Window & Event Loop | tao closure-style event loop, one window, clean exit code 0 on close | **PASS** | Window creation in [`crates/wavical-shell/src/lib.rs:502-518`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L502-L518); Event loop cleanly exits on `WindowEvent::CloseRequested` at line 1115 (`*control_flow = ControlFlow::Exit`). |
| **WP2-02** | WebView2 Missing Error | Missing WebView2 runtime handled with friendly message, non-zero exit, no panic | **PASS** | Verified via simulation `WAVICAL_SIMULATE_NO_WEBVIEW2=1`: logs error, prints friendly install instructions, exits code 1 ([`crates/wavical-shell/src/lib.rs:479-499`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L479-L499)). Unit test: `test_shell_webview2_missing_runtime_simulation`. |
| **WP2-03** | User Data Isolation | WebView2 user data folder must be isolated and specific to application | **FAIL** | **FINDING-S3**: `WebViewBuilder::new()` in [`crates/wavical-shell/src/lib.rs:845`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L845) omits `.with_data_directory(...)`. Runtime defaults to shared or CWD `EBWebView` folder. |
| **WP5-01** | Bridge Script Invariants | Frozen `window.wavical`, non-writable, non-configurable, no eval/Function | **PASS** | `WAVICAL_INIT_SCRIPT` uses `Object.freeze` on API object and `Object.defineProperty` on `window.wavical` ([`crates/wavical-shell/src/lib.rs:136-147`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L136-L147)). Verified in `test_shell_bridge_script_immutability_and_safety`. |
| **WP5-02** | Minimal Bridge Exposure | Only `invoke` exposed on `window.wavical`; no extra globals added to window | **FAIL** | **FINDING-S1**: `WAVICAL_INIT_SCRIPT` exposes `onMenu` and `onShortcut` on `window.wavical`, and attaches `__wavical_receive__`, `__wavical_event__`, and `__wavical_shortcut_event__` to `window`. Verified by regression test `test_shell_bridge_script_only_exposes_invoke_and_no_other_globals`. |
| **WP5-03** | Request ID & Match | Unique request IDs up to `MAX_SAFE_INTEGER`, matched by Promise resolver | **PASS** | Encapsulated IIFE maintains `pending` Map and increments `nextId` ([`crates/wavical-shell/src/lib.rs:34-48,105-117`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L34-L48)). |
| **WP5-04** | Response Delivery Escaping | Response delivery must never concatenate untrusted strings into executable script | **PASS** | `build_delivery_script` uses double JSON serialization (`serde_json::to_string` of response JSON) and delivers via `JSON.parse` ([`crates/wavical-shell/src/lib.rs:359-367`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L359-L367)). Verified in `test_shell_response_delivery_escaping`. |
| **WP5-05** | Origin Verification | Check origin of every IPC message against allowlist; exact scheme+host+port check | **PASS** | `extract_origin` parses `Uri`, strips default ports (80/443), rejects userinfo (`@`), and compares via `eq_ignore_ascii_case` ([`crates/wavical-shell/src/lib.rs:288-314`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L288-L314)). Verified in `test_shell_origin_check_vectors`. |
| **WP5-06** | Subframe Isolation | IPC messages from cross-origin iframes, `srcdoc`, `about:blank`, or `data:` rejected | **PASS** | `is_origin_allowed` rejects non-allowlisted origins. `blob:`, `data:`, and `about:blank` lack matching host authorities and are rejected ([`crates/wavical-shell/src/lib.rs:384`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L384)). Verified in `test_shell_subframe_and_special_schemes`. |
| **WP5-07** | Raw Channel Independence | Host enforcement does not rely on JS wrapper; raw `window.ipc.postMessage` defended | **PASS** | All capability, size, and schema validations execute strictly in native Rust `process_ipc_message` ([`crates/wavical-shell/src/lib.rs:378-450`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L378-L450)). Verified in `test_shell_raw_ipc_channel_enforcement`. |
| **WP5-08** | Demo Methods Parity | `core.ping` granted and responds; `core.echo` ungranted and returns generic error | **PASS** | Default config grants `core.ping` only ([`crates/wavical-shell/src/lib.rs:226`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L226)); `core.echo` returns `GENERIC_METHOD_ERROR` ([`crates/wavical-shell/src/lib.rs:431-434`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L431-L434)). |
| **WP6-01** | Navigation Guard | Allow only application origin; deny external URLs, `javascript:`, `data:`, `file:` | **PASS** | `handle_navigation_request` verifies target URL origin against `allowed_origins` allowlist ([`crates/wavical-shell/src/lib.rs:325-341`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L325-L341)). Verified in `test_shell_navigation_and_popup_guard`. |
| **WP6-02** | Popup Blocker | Intercept `window.open()` and new-window requests; return `Deny` unconditionally | **PASS** | `handle_new_window_request` returns `NewWindowResponse::Deny` unconditionally ([`crates/wavical-shell/src/lib.rs:344-353`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L344-L353)). Verified in `test_shell_navigation_and_popup_guard`. |
| **WP6-03** | Default Strict CSP | CSP on every protocol response; zero `'unsafe-inline'` or `'unsafe-eval'` in script-src | **PASS** | `DEFAULT_CSP` enforced on all protocol responses ([`crates/wavical-shell/src/lib.rs:19,541,570`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L19)). Verified in `test_shell_csp_directives_strictness`. |
| **WP6-04** | Response Security Headers | Content-Type, X-Content-Type-Options: nosniff, and CSP on 200, 404, and rejections | **PASS** | `handle_protocol_request` and `build_safe_404_response` inject headers on all responses ([`crates/wavical-shell/src/lib.rs:539-543,568-571`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L539-L543)). Verified in `test_shell_protocol_response_headers_on_all_responses`. |
| **WP6-05** | Per-Window CSP Override | Override allowed via config; page JS cannot tamper with CSP | **PASS** | `WindowConfig.custom_csp` supported ([`crates/wavical-shell/src/lib.rs:200,252`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L200)). No webview IPC method exists to mutate CSP. |
| **WP6-06** | CSP Directive Completeness | Explicit declaration of all core fetch directives (`connect-src`, `frame-src`) | **PARTIAL** | **FINDING-S5**: `connect-src` and `frame-src` are not explicitly defined in `DEFAULT_CSP`. While they safely fall back to `default-src 'self'`, explicit declaration avoids browser fallback ambiguity. |
| **WP6-07** | Devtools Disabled in Release | Devtools enabled only in debug builds; strictly disabled in release | **PASS** | `is_devtools_enabled` binds directly to `cfg!(debug_assertions)` ([`crates/wavical-shell/src/lib.rs:597-599,846`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L597-L599)). Verified in `test_shell_devtools_disabled_in_release`. |
| **WP6-08** | Debug Argument Exposure | WebView2 additional browser arguments disabled or stripped in release builds | **PARTIAL** | Setting `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` exposes DevTools WebSocket interface even in release mode. Requires local environment variable access. |
| **WP7-01** | Secrets Rule & Method Enumeration | No bridge method returns credentials/secrets; registered methods test in place | **PASS** | `test_registered_methods_exact_allowlist` ([`crates/wavical-shell/src/lib.rs:1420-1429`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L1420-L1429)) asserts exact method allowlist. None of the registered methods returns secrets. |
| **WP7-02** | Dynamic Capability Stdin Guard | Dynamic capability update restricted to development/authenticated sessions | **PARTIAL** | **FINDING-S4**: CLI `--listen-stdin` flag is not gated by `cfg(debug_assertions)` in `main.rs:41`. If launched with `--listen-stdin`, unauthenticated stdin input can grant capabilities in release builds. |
| **WP7-03** | Structured Security Logging | Tracing events emitted for all violations: `capability_denied`, `malformed_message`, `oversize_message`, `origin_rejected`, `path_rejected`, `navigation_blocked`, `popup_blocked` | **PASS** | All 7 events emitted with structured fields ([`crates/wavical-shell/src/lib.rs:333,346,387,400,411,424,552`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L333)). Unit test: `test_all_security_events_emitted_on_violation`. |
| **WP7-04** | Log Payload Truncation | Truncate large payloads (max 64 bytes) without splitting UTF-8 codepoint boundaries | **PASS** | `truncate_payload` walks backward to valid `is_char_boundary` ([`crates/wavical-shell/src/lib.rs:269-281`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L269-L281)). Verified in `test_shell_truncate_payload_boundaries`. |
| **WP8-01** | Error Sanitization to Webview | Internal paths, rejection enums, or debug markers never leaked to webview | **PASS** | `build_safe_404_response` and IPC error handlers emit generic strings. Asset root with `SECRETMARKER_PATH_98765` never appears in response body or headers. Verified in `test_shell_secret_marker_never_leaked_to_page`. |
| **WP8-02** | Integration Attack Suite Harness | Harness loads `tests/attack-page`, executes attacks, and asserts clean pass | **FAIL** | **FINDING-S2**: Integration test harness hangs indefinitely on test 10 due to unreplied oversized payload, and runner assertion hardcodes `passed == 7` while attack page contains 13 tests. |

---

## 2. Detailed Findings

### [FINDING-S2] HIGH: Integration Harness Hangs on Oversized Payload & Fails on Hardcoded Suite Count
- **Crate / File:** `tests/integration/src/main.rs:280`, `tests/attack-page/app.js:370`
- **Severity:** High
- **Description:**
  1. In `tests/attack-page/app.js`, test 10 (`testOversizedPayloads`) executes:
     ```javascript
     const hugeText = "X".repeat(1024 * 1024 + 10240);
     await window.wavical.invoke("clipboard.write", { text: hugeText });
     ```
     In `crates/wavical-shell/src/lib.rs:398-408`, incoming messages exceeding 1 MB trigger `IpcError::MessageTooLarge` and return `Ok(None)`. When `Ok(None)` is returned, the host produces no response to the webview. Consequently, the Promise created in `window.wavical.invoke` never resolves or rejects. The entire attack test suite hangs indefinitely until the integration harness times out after 12 seconds:
     ```
     [Integration Test] TIMEOUT: Attack page did not complete in 12s!
     error: process didn't exit successfully: `target\debug\wavical-integration-tests.exe` (exit code: 1)
     ```
  2. In `tests/integration/src/main.rs:280`, the completion condition is hardcoded to:
     ```rust
     if report.all_passed && report.summary.passed == 7 && report.summary.failed == 0
     ```
     However, `tests/attack-page/app.js` registers **13** test suites. Even if the hang were resolved, `report.summary.passed` would be 13, causing `report.summary.passed == 7` to evaluate to `false` and trigger an exit with failure code 1.
- **Impact:** Automated CI integration testing for the shell security bench fails deterministically.
- **Reproduction:** Run `cargo run -p wavical-integration-tests`.
- **Recommended Remediation:**
  1. In `tests/attack-page/app.js`, send oversized payloads via the raw channel `window.ipc.postMessage(hugePayload)` instead of `await window.wavical.invoke(...)`. Alternatively, ensure `window.wavical.invoke` implements a client-side timeout or Rust emits an explicit oversize error response.
  2. In `tests/integration/src/main.rs:280`, check `report.all_passed && report.summary.passed == report.summary.total && report.summary.failed == 0`.

---

### [FINDING-S3] MEDIUM: Missing App-Specific WebView2 User Data Directory Isolation
- **Crate / File:** `crates/wavical-shell/src/lib.rs:845-896`
- **Severity:** Medium
- **Description:**
  `WebViewBuilder::new()` in `wavical-shell` initializes WebView2 without calling `.with_data_directory(...)`. By default on Windows, WebView2 stores browser state, cookies, DOM storage, and cache inside an `EBWebView` directory created in the executable directory or the current working directory.
- **Impact:**
  1. Multiple running instances or different applications using the shell share user data or pollute the application launch directory with runtime cache files.
  2. If the application directory is read-only (e.g. `C:\Program Files\...`), WebView2 initialization may fail or crash on startup.
- **Reproduction:** Inspect lines 845–896 of `crates/wavical-shell/src/lib.rs`. No `.with_data_directory(...)` call is configured.
- **Recommended Remediation:**
  Derive an app-specific storage path (e.g., `%LOCALAPPDATA%\wavical\<window_id>\EBWebView`) and configure `.with_data_directory(app_data_dir)` on `WebViewBuilder`.

---

### [FINDING-S1] MEDIUM: Bridge Script Invariant Violation & Namespace Pollution
- **Crate / File:** `crates/wavical-shell/src/lib.rs:77-96,136-140`
- **Severity:** Medium
- **Description:**
  Phase 0 WP5 and `SECURITY_MODEL.md` Invariant 1 specify that the injected bridge script defines a frozen `window.wavical` object with a single `invoke(method, params)` function, and that no other globals are attached to `window`.
  In `WAVICAL_INIT_SCRIPT`:
  1. `wavicalApi` exposes `invoke`, `onMenu`, and `onShortcut`.
  2. `Object.defineProperty` is called on `window` to attach `__wavical_receive__`, `__wavical_event__`, and `__wavical_shortcut_event__`.
- **Impact:**
  While these internal properties are configured as `writable: false, configurable: false, enumerable: false`, exposing them on `window` broadens the surface of the web runtime and violates the minimal-bridge invariant.
- **Reproduction:**
  Run `cargo test -p wavical-audit --test part3_shell test_shell_bridge_script_only_exposes_invoke_and_no_other_globals -- --exact`.
- **Recommended Remediation:**
  For Phase 0 strict compliance, encapsulate event dispatching within closure state or gate menu/shortcut helper properties behind dedicated feature configuration.

---

### [FINDING-S4] MEDIUM: Unauthenticated Dynamic Capability Reloading via Stdin in Release Binaries
- **Crate / File:** `crates/wavical-shell/src/main.rs:41`, `crates/wavical-shell/src/lib.rs:706-752`
- **Severity:** Medium
- **Description:**
  `ShellCli` exposes `--listen-stdin` as a public CLI argument. The listener thread accepts JSON input on `stdin`:
  ```rust
  "update_capabilities" => {
      let caps = val.get("capabilities")...;
      let _ = stdin_proxy.send_event(UserEvent::UpdateCapabilities(caps));
  }
  ```
  This code is compiled into release binaries and contains no authentication, token verification, or origin binding. If an application is launched in an environment where a local process or child process can write to standard input, it can elevate its capabilities to arbitrary methods (e.g. `["*"]`).
- **Impact:** Local privilege escalation from standard input to native shell capabilities.
- **Reproduction:** Launch `wavical-shell --listen-stdin` in release mode, write `{"action": "update_capabilities", "capabilities": ["*"]}` to stdin, and observe capability grant.
- **Recommended Remediation:**
  Guard `--listen-stdin` and `UpdateCapabilities` behind `#[cfg(debug_assertions)]` or require a cryptographically signed session token.

---

### [FINDING-S5] LOW: Omission of Explicit `connect-src` and `frame-src` Directives in Default CSP
- **Crate / File:** `crates/wavical-shell/src/lib.rs:19`
- **Severity:** Low
- **Description:**
  `DEFAULT_CSP` is defined as:
  ```
  default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self';
  ```
  `connect-src` (governing `fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource`) and `frame-src` (governing `<iframe>` sources) are not explicitly declared.
- **Impact:**
  While W3C CSP Level 2/3 specifications dictate fallback to `default-src 'self'`, defense-in-depth principles require explicit declarations (`connect-src 'self'; frame-src 'none';`) to eliminate browser interpretation discrepancies.
- **Recommended Remediation:**
  Update `DEFAULT_CSP` to explicitly specify `connect-src 'self'; frame-src 'none';`.

---

## 3. Top 5 Findings Deep Dive

```
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│                               TOP 5 AUDIT FINDINGS (PHASE 0 SHELL)                               │
├────┬──────────────┬──────────┬───────────────────────┬───────────────────────────────────────────┤
│ #  │ Finding ID   │ Severity │ Target Location       │ Title & Summary                           │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 1  │ FINDING-S2   │ HIGH     │ tests/integration     │ Attack Suite Hangs on Oversize Payload    │
│    │              │          │ tests/attack-page     │ & Hardcoded Assertion Fails CI Runner     │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 2  │ FINDING-S3   │ MEDIUM   │ crates/wavical-shell  │ Missing WebView2 User Data Directory      │
│    │              │          │ lib.rs:845            │ Isolation per Application ID              │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 3  │ FINDING-S4   │ MEDIUM   │ crates/wavical-shell  │ Unauthenticated Stdin Dynamic Capability  │
│    │              │          │ main.rs:41 / lib.rs   │ Grant Surface Active in Release Binaries  │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 4  │ FINDING-S1   │ MEDIUM   │ crates/wavical-shell  │ Bridge Script Specification Drift: Extra  │
│    │              │          │ lib.rs:77-96,136-140  │ API Properties and Window Globals Exposed │
├────┼──────────────┼──────────┼───────────────────────┼───────────────────────────────────────────┤
│ 5  │ FINDING-S5   │ LOW      │ crates/wavical-shell  │ Omission of Explicit connect-src and      │
│    │              │          │ lib.rs:19             │ frame-src Directives in Default CSP       │
└────┴──────────────┴──────────┴───────────────────────┴───────────────────────────────────────────┘
```

### 1. FINDING-S2 (High): Attack Suite Hangs Indefinitely on Oversized Message
- **Root Cause:** When `window.wavical.invoke` sends a message exceeding 1 MB, Rust drops the message without emitting a response (`Ok(None)`). In JavaScript, `window.wavical.invoke` waits on an unresolved Promise. The test suite stops making progress and triggers a 12s timeout.
- **Secondary Flaw:** `tests/integration/src/main.rs:280` asserts `report.summary.passed == 7`, whereas `tests/attack-page/app.js` runs 13 tests.
- **Evidence:** Real execution log:
  ```
  2026-10-26T07:01:30.245991Z WARN wavical_shell: Security check failed: oversize IPC message rejected security_event="oversize_message" ...
  [Integration Test] TIMEOUT: Attack page did not complete in 12s!
  error: process didn't exit successfully: `target\debug\wavical-integration-tests.exe` (exit code: 1)
  ```

### 2. FINDING-S3 (Medium): Missing WebView2 User Data Directory Isolation
- **Root Cause:** `WebViewBuilder::new()` in `crates/wavical-shell/src/lib.rs:845` omits `.with_data_directory(...)`.
- **Evidence:** In `crates/wavical-shell/src/lib.rs:845-896`, `WebViewBuilder` is configured with devtools, protocol, navigation, popups, and IPC handler, but never sets the user data folder.

### 3. FINDING-S4 (Medium): Unauthenticated Stdin Capability Grants in Release Builds
- **Root Cause:** `crates/wavical-shell/src/main.rs:41` allows passing `--listen-stdin` in release mode. The background thread parses JSON commands from stdin and updates window capabilities without credential verification.
- **Evidence:** [`crates/wavical-shell/src/lib.rs:725-737`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L725-L737).

### 4. FINDING-S1 (Medium): Bridge Script Specification Drift & Window Namespace Pollution
- **Root Cause:** Phase 0 spec mandates: "only `invoke` is exposed; no other globals are added". The injected initialization script exports `onMenu` and `onShortcut` on `window.wavical` and sets three non-enumerable properties on `window`.
- **Evidence:** Tested deterministically by `test_shell_bridge_script_only_exposes_invoke_and_no_other_globals` (marked `#[ignore = "FINDING-S1"]`).

### 5. FINDING-S5 (Low): Incomplete Directive Set in Default CSP
- **Root Cause:** `DEFAULT_CSP` relies on `default-src 'self'` fallback for `connect-src` and `frame-src`.
- **Evidence:** [`crates/wavical-shell/src/lib.rs:19`](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/crates/wavical-shell/src/lib.rs#L19).

---

## 4. Verification Evidence & Real Command Outputs

### Remote Debugging Port Test (`--remote-debugging-port=9222`)
```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
Start-Process "target\release\wavical-shell.exe"
Invoke-RestMethod -Uri "http://127.0.0.1:9222/json"
```
**Output:**
```json
{
    "description": "",
    "devtoolsFrontendUrl": "https://aka.ms/docs-landing-page/serve_rev/@aa3a7406ba8c53a83536df539f07124d73c899bd/inspector.html?ws=127.0.0.1:9222/devtools/page/D17EC4C5588CD07FD0B45C311C80FA12",
    "id": "D17EC4C5588CD07FD0B45C311C80FA12",
    "title": "Wavical - Hello World",
    "type": "page",
    "url": "http://wavical.localhost/index.html",
    "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/D17EC4C5588CD07FD0B45C311C80FA12"
}
```
*Assessment:* Setting `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` opens the Chromium remote debugging WebSocket port even on release builds, allowing local processes to connect to DevTools Protocol.

### Real Log Traces for Security Violations
- **Capability Denied:**
  `WARN wavical_shell: Security check failed: IPC capability denied security_event="capability_denied" window_id=integration-attack-window method=core.echo id=1 reason="Method not granted in window capability allowlist"`
- **Malformed Message:**
  `WARN wavical_shell: Security check failed: malformed IPC message rejected security_event="malformed_message" window_id=integration-attack-window payload={ invalid json syntax ... reason=JSON deserialization failed: key must be a string at line 1 column 3`
- **Oversize Message:**
  `WARN wavical_shell: Security check failed: oversize IPC message rejected security_event="oversize_message" window_id=integration-attack-window size=1058816 max_size=1048576 reason="Raw message exceeds maximum size limit"`
- **Popup Blocked:**
  `WARN wavical_shell: Security check failed: popup blocked security_event="popup_blocked" window_id=integration-attack-window target_url=https://external-phish.example.com/ reason="Popup and new-window creation is denied by default"`
- **Path Rejected:**
  `WARN wavical_shell: Security check failed: protocol path rejected security_event="path_rejected" window_id=integration-attack-window requested_path=/..%5CCargo.toml reason=Directory traversal sequence detected ('..')`
