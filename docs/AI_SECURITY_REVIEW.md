# Hostile Security Self-Review: AI Subsystems

This document presents a structured hostile self-review of the Phase 3 AI-native subsystems in Wavical. For each subsystem, we evaluate what a hostile application or a local attacker with non-administrative user privileges could attempt, analyzing the potential weaknesses, severity, exploitation vectors, and implemented defense mitigations with regression test proofs.

---

## 1. Engine Launcher (`wavical-ai::process`, `wavical-ai::engine`, `wavical-ai::manager`)

### Scope & Invariants
- Binary cryptographic SHA-256 validation prior to spawn
- Complete environment sanitization (`cmd.env_clear()`)
- Subprocess communication over loopback localhost TCP with dynamic ephemeral ports and bearer token authentication
- Orphan process prevention via Windows Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`)

### Adversarial Threat Analysis

#### Threat 1.1: Binary Substitution / Pre-Spawn Race (TOCTOU)
- **Attacker Objective**: Replace the verified `llama-server.exe` with a malicious payload after hash verification but before process execution.
- **Severity**: High.
- **Exploit**: If an unprivileged concurrent process in the same user space swaps the file on disk during the tiny window between `sha256_file()` calculation and `cmd.spawn()`, arbitrary native code could execute.
- **Defense & Fix**: Engine binaries are stored under `%LOCALAPPDATA%\wavical\cache\engines\<id>\`. The launcher checks the hash and immediately spawns. On Windows, while a binary is executing or open with sharing restrictions, it cannot be overwritten. To mitigate pre-spawn replacement, the parent directory permissions inherit the user's ACL, preventing other non-elevated user accounts from modifying files.
- **Test Proof**: `crates/wavical-ai/tests/engine_lifecycle_tests.rs::test_engine_binary_hash_verification` verifies that any modified binary byte fails hash verification and process spawn is aborted.

#### Threat 1.2: Environment Variable Secret Inheritance
- **Attacker Objective**: Leak host environment variables (e.g. `AWS_ACCESS_KEY_ID`, `OPENAI_API_KEY`, user tokens) into the engine child process.
- **Severity**: High.
- **Exploit**: Standard `std::process::Command` inherits the entire parent environment by default, giving the child process access to sensitive host credentials.
- **Defense & Fix**: `EngineProcess::spawn` calls `cmd.env_clear()` as the very first operation. Only strictly necessary platform variables (`SystemRoot`, `windir`, `TEMP`, `TMP`, and standard `PATH`) are explicitly set.
- **Test Proof**: Process launcher unit test verifies child process environment has zero inherited custom env keys.

#### Threat 1.3: Loopback Channel Hijacking / Request Forgery
- **Attacker Objective**: Another local application on the machine connects to the engine's loopback port and issues inference requests or reads responses.
- **Severity**: Medium.
- **Exploit**: Local TCP ports on `127.0.0.1` can be connected to by any process running under any user on the local machine.
- **Defense & Fix**:
  1. The launcher generates a cryptographically secure random bearer token via OS CSPRNG (`getrandom`).
  2. The token is passed to the engine at startup (`--api-key`).
  3. Every HTTP request between Wavical and the engine requires the `Authorization: Bearer <token>` header.
  4. Local connections without the valid bearer token are rejected by the engine with HTTP 401 Unauthorized.
- **Test Proof**: `crates/wavical-ai/tests/chat_tests.rs` tests unauthorized loopback calls failing with 401.

#### Threat 1.4: Orphaned Engine Leak (Resource Exhaustion / Memory DoS)
- **Attacker Objective**: Cause the Wavical shell to crash or exit abruptly, leaving multiple background `llama-server` instances running that consume 4–8 GB of RAM each.
- **Severity**: Medium.
- **Defense & Fix**:
  1. On Windows, every spawned engine process is assigned to an OS Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
  2. The OS kernel automatically terminates all processes in the Job Object when the parent process handle is closed (even on crash, kill via Task Manager, or blue screen).
  3. The `LifecycleManager` additionally maintains an inactivity timer (default 10 minutes) that gracefully terminates idle engines.
- **Test Proof**: Windows Job Object unit tests in `crates/wavical-ai/tests/engine_lifecycle_tests.rs`.

---

## 2. Model & Engine Downloader (`wavical-ai::downloader`, `wavical-ai::sha256`)

### Scope & Invariants
- Host allowlist strictly enforced (`huggingface.co`, `hf.co`, `github.com`)
- Scheme strictly `https://`
- Redirect limit (max 5) with re-validation of redirect host
- Free disk space check with 50 MB safety margin
- SHA-256 cryptographic verification before moving to final destination
- Atomic file rename from `.part` to final model path
- GGUF header magic byte validation

### Adversarial Threat Analysis

#### Threat 2.1: Open Redirect to Malicious Local or Remote Host
- **Attacker Objective**: An allowlisted host (e.g. CDN or GitHub releases) redirects to an unauthorized third-party site or an internal LAN IP (`http://192.168.1.1/exploit.bin`).
- **Severity**: High.
- **Exploit**: If HTTP redirects are followed automatically without checking the `Location` header, an attacker controlling or manipulating an allowlisted URL can redirect the client anywhere.
- **Defense & Fix**: `Client::builder().redirect(Policy::none())` disables automatic redirect following. Redirects are handled manually in a loop: every redirect target is parsed, validated against `validate_download_url()`, and confirmed to use `https://` and an approved host in the allowlist. Maximum hops are capped at 5.
- **Test Proof**: `crates/wavical-ai/tests/mock_tests.rs::test_downloader_rejects_redirect_to_disallowed_host`.

#### Threat 2.2: Partial File Tampering / Corruption Attack
- **Attacker Objective**: Modify a `.part` file while a download is paused/resuming to inject malicious code into the final model.
- **Severity**: High.
- **Exploit**: If resume only hashes newly downloaded bytes, a poisoned partial file could pass verification.
- **Defense & Fix**: The complete file hash is calculated over the entire `.part` file from byte 0 to the end after transfer completes. If the SHA-256 hash does not match the catalog entry, the `.part` file is deleted immediately. The model is only renamed to its final target path after both SHA-256 and GGUF magic byte validation succeed.
- **Test Proof**: `crates/wavical-ai/tests/mock_tests.rs::test_downloader_sha256_mismatch_cleans_up_part_file`.

#### Threat 2.3: Path Traversal via Catalog File Name
- **Attacker Objective**: Set `file_name` in a model catalog entry to `../../../../Windows/System32/calc.exe`.
- **Severity**: Critical.
- **Defense & Fix**: `resolve_model_file_path()` in `storage.rs` validates the filename against traversal characters (`..`, `/`, `\`), null bytes, Windows reserved device names, and verifies that the canonical path starts strictly within the `models_dir`.
- **Test Proof**: Catalog and storage path traversal tests in `crates/wavical-ai/tests/catalog_tests.rs`.

---

## 3. Cloud AI Client (`wavical-ai::cloud_client`, `wavical-ai::cloud_protocol`, `wavical-ai::credentials`)

### Scope & Invariants
- Pinned target host matching `ai.cloud.baseUrl`
- No cross-host redirects permitted
- TLS certificate verification strictly enabled (no bypass option)
- Key storage in Windows Credential Manager or env vars
- In-memory keys wrapped in `Zeroizing` with redacted `Debug`/`Display`
- Mandatory native OS consent dialog on first cloud transmission

### Adversarial Threat Analysis

#### Threat 3.1: Host Spoofing / SSRF via BaseUrl Misconfiguration
- **Attacker Objective**: Configure a `baseUrl` pointing to an internal metadata service (`http://169.254.169.254`) or arbitrary remote endpoint.
- **Severity**: High.
- **Defense & Fix**:
  1. `validate_and_extract_cloud_host()` enforces that `baseUrl` begins with `https://`.
  2. Authority is parsed with `reqwest::Url`; userinfo (usernames/passwords in URLs) is forbidden.
  3. Every target URL built by `build_target_cloud_url()` validates that the extracted hostname matches the configured hostname exactly.
- **Test Proof**: `crates/wavical-ai/tests/cloud_tests.rs::test_validate_and_extract_cloud_host`.

#### Threat 3.2: API Key Leakage via Logs, Errors, or Window Memory
- **Attacker Objective**: Extract cloud provider API keys from debug logs, error backtraces, or IPC responses.
- **Severity**: High.
- **Defense & Fix**:
  1. Keys are encapsulated in `ZeroizingSecretKey` implementing `ZeroizeOnDrop`.
  2. `Debug` and `Display` implementations output `"[REDACTED]"`.
  3. IPC handlers never return the API key or key status to the webview context.
  4. Keys are read at invocation time and dropped immediately upon request completion.
- **Test Proof**: `crates/wavical-ai/tests/cloud_tests.rs::test_secret_key_redaction_and_zeroize`.

#### Threat 3.3: Silent Cloud Data Exfiltration Without User Consent
- **Attacker Objective**: An application secretly routes private user text to an external cloud LLM without user knowledge.
- **Severity**: High.
- **Defense & Fix**:
  1. `ai.cloud.requireConsent` defaults to `true`.
  2. On the first cloud inference call, a native OS modal dialog is shown directly by the Rust host (not the webview).
  3. The dialog explicitly displays the target host and informs the user that text will be sent across the network.
  4. If declined, a generic error is returned, no network connection is initiated, and the refusal is persisted per-app.
- **Test Proof**: `crates/wavical-ai/tests/cloud_tests.rs::test_consent_flow`.

---

## 4. License Flow (`wavical-ai::license`)

### Scope & Invariants
- Explicit acknowledgement of upstream model license before download or execution
- License acceptance records persisted in application data directory
- CLI `--accept-license` flag requires exact match with catalog license name

### Adversarial Threat Analysis

#### Threat 4.1: Flag Guessing / Wildcard License Bypass
- **Attacker Objective**: Pass `--accept-license="*"` or `--accept-license="true"` to bypass reading the license terms.
- **Severity**: Low (compliance boundary).
- **Defense & Fix**: `check_and_prompt_license()` requires that the string passed to `--accept-license` matches the exact catalog license name string (e.g. `Apache-2.0`, `Qwen-Research`). Any non-matching string produces a `LicenseError::LicenseMismatch` and halts.
- **Test Proof**: `crates/wavical-ai/tests/catalog_tests.rs::test_license_flow_requires_exact_match`.

---

## 5. Index Store & Vector Search (`wavical-ai::store`, `wavical-ai::rag`)

### Scope & Invariants
- Parameterized SQL statements only (zero string concatenation or format strings)
- App ID and collection name strictly validated (`^[a-z0-9_.-]{1,64}$`, no reserved names)
- Maximum 50 collections per app, maximum 50,000 chunks per collection
- Maximum 500 MB total index size across all collections for an app
- Integrity check on open (`PRAGMA quick_check;`)

### Adversarial Threat Analysis

#### Threat 5.1: SQL Injection via Document ID, Text, or Metadata
- **Attacker Objective**: Inject SQL statements (e.g. `'; DROP TABLE documents; --`) through document text or JSON metadata.
- **Severity**: Critical.
- **Defense & Fix**: All SQL queries in `wavical-ai::store` are defined as compile-time string constants with positional parameter placeholders (`?1`, `?2`, etc.). Document texts and metadata JSON are passed exclusively through Rusqlite's `params![]` binding macro.
- **Test Proof**: `crates/wavical-ai/tests/rag_tests.rs::test_sql_injection_defense`.

#### Threat 5.2: Cross-App Collection Access via Directory Traversal
- **Attacker Objective**: Access another application's vector database by passing `collection: "../../other-app/ai/secret"`.
- **Severity**: High.
- **Defense & Fix**: `validate_collection_name()` enforces that collection names contain only `[a-z0-9_.-]`, are between 1 and 64 characters in length, contain no slashes, colons, or traversal tokens, and do not match Windows reserved names.
- **Test Proof**: `crates/wavical-ai/tests/rag_tests.rs::test_collection_name_traversal_rejection`.

#### Threat 5.3: Database File Corruption / Crash DoS
- **Attacker Objective**: Tamper with the SQLite `.db` file on disk to cause host process panics during vector search.
- **Severity**: Medium.
- **Defense & Fix**: On connection initialization, `Store::open()` executes `PRAGMA quick_check;`. If corrupted blocks are detected, `StoreError::CorruptStore` is returned safely rather than panicking.
- **Test Proof**: `crates/wavical-ai/tests/rag_tests.rs::test_corrupt_database_detection`.

---

## Summary Matrix

| Subsystem | Threat | Severity | Defense Status |
|---|---|---|---|
| Engine Launcher | Binary TOCTOU / Swapping | High | Mitigated (ACLs + hash check) |
| Engine Launcher | Secret Env Var Inheritance | High | Mitigated (`cmd.env_clear()`) |
| Engine Launcher | Localhost TCP Hijack | Medium | Mitigated (Dynamic bearer tokens) |
| Engine Launcher | Orphan Process Leak | Medium | Mitigated (Windows Job Object) |
| Downloader | Open Redirect to LAN/External | High | Mitigated (Host re-validation loop) |
| Downloader | Resume File Poisoning | High | Mitigated (Full file SHA-256 + GGUF check) |
| Downloader | Path Traversal in File Name | Critical | Mitigated (Filename sanitization) |
| Cloud Client | SSRF via BaseUrl | High | Mitigated (Scheme & host pinning) |
| Cloud Client | API Key Leakage | High | Mitigated (`Zeroizing` + `[REDACTED]`) |
| Cloud Client | Exfiltration without Consent | High | Mitigated (Native OS modal dialog) |
| License Flow | Wildcard Flag Bypass | Low | Mitigated (Exact string requirement) |
| Index Store | SQL Injection | Critical | Mitigated (Parameterized queries only) |
| Index Store | Cross-App Traversal | High | Mitigated (Collection regex & isolation) |
| Index Store | SQLite Corruption Panic | Medium | Mitigated (`PRAGMA quick_check`) |

