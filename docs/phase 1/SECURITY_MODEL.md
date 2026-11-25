# Wavical Security Model

This document outlines the security architecture, trust boundaries, enforcement mechanisms, and security audit logging for the Wavical application runtime.

---

## 1. Trust Boundaries

Wavical enforces a strict two-tier security model with a clear isolation boundary between the web rendering environment and the native OS host:

```
┌─────────────────────────────────────────────────────────────┐
│                 Untrusted Webview Context                   │
│                                                             │
│   • HTML / CSS / JavaScript Runtime (WebView2)              │
│   • DOM, Workers, Iframes, and Third-Party Web Assets       │
│   • Threat assumptions: Potential XSS, malformed payloads,  │
│     prototype pollution, adversarial navigation attempts    │
└──────────────────────────────┬──────────────────────────────┘
                               │
                Strict IPC & Protocol Boundary
        (Only wavical:// protocol & length/type-checked IPC)
                               │
┌──────────────────────────────▼──────────────────────────────┐
│                  Privileged Native Host                     │
│                                                             │
│   • Rust Shell (tao + wry glue)                             │
│   • wavical-protocol (Safe path resolution, MIME, CSP)      │
│   • wavical-ipc (Default-deny capability router & handlers) │
│   • OS Filesystem, Process APIs, and Native Windows         │
└─────────────────────────────────────────────────────────────┘
```

### Core Security Invariants
1. **Never Trust the Frontend**: Client-side JavaScript wrappers (`window.wavical`) are conveniences. All capability enforcement, origin checking, size limits, and sanitization occur exclusively in compiled native Rust.
2. **Secrets Rule**: No method exposed across the IPC bridge may ever return credentials, secret tokens, private keys, or raw native OS handles/pointers to the webview context.
3. **Information Leakage Prevention**: Protocol rejections and IPC errors returned to the webview never leak internal OS paths, rejection enum reasons, or filesystem layouts.
4. **Default Deny Everywhere**: All capabilities, origins, popups, and external navigations are denied unless explicitly allowlisted.

### Developer Environment & Config Execution (`wavical.config.js`)
- **Trust Boundary**: `wavical.config.js` is an executable JavaScript module evaluated via Node.js during CLI execution (`wavical dev`, `wavical build`).
- **Permissions**: `wavical.config.js` executes with the local developer's user permissions and privileges.
- **Threat Assumption**: Running CLI commands on untrusted third-party repositories carries arbitrary code execution risks. Developers must review third-party configuration files before executing developer CLI commands.

---

## 2. Enforcement Architecture

### A. Custom Protocol & Path Sanitization (`wavical-protocol`)
- **Protocol**: `wavical://` (mapped to virtual origin `http://wavical.localhost` or `https://wavical.localhost` on Windows).
- **Sanitization Checks**:
  - Rejection of path traversal (`..`), single/nested percent-encoding (`%2e%2e`), and mixed slashes (`/` and `\`).
  - Rejection of drive letters (`C:`), absolute roots, and UNC network shares (`\\server\share`).
  - Rejection of NTFS Alternate Data Streams (`file:stream`, `::$DATA`).
  - Rejection of Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`).
  - Rejection of trailing dots/spaces, null bytes (`\0`), and paths exceeding 2048 bytes.
  - Canonicalization escape verification: Target canonical path must strictly start with canonical asset root (stopping symlinks and NTFS junction escapes).
- **Response**: Any rejection yields an identical minimal `404 Not Found` with zero path detail.

### B. IPC Framing & Parsing (`wavical-ipc`)
- **Raw Message Size**: Enforces a strict 1 MB (`MAX_MESSAGE_SIZE_BYTES`) limit *before* invoking any JSON parser.
- **Nesting Depth Limit**: Scans raw JSON and parsed parameters to enforce a maximum nesting depth of 32 levels (`MAX_JSON_DEPTH`), preventing stack overflow DoS.
- **Strict Typing & Schema**: Deserializes into `Request { id, method, params }` using `#[serde(deny_unknown_fields)]`. Duplicate keys, extraneous fields, invalid types, and negative/floating-point IDs are rejected.
- **Request ID Range**: Enforces $0 \le \text{id} \le 2^{53} - 1$ (JavaScript `Number.MAX_SAFE_INTEGER`).
- **Method Name Constraints**: Restricted to ASCII alphanumeric, periods, underscores, and hyphens (`[a-zA-Z0-9_.-]`), length 1–64 bytes, no consecutive periods (`..`), and no leading/trailing separators.

### C. Default-Deny Capability Model (`wavical-ipc`)
- **Window Capabilities**: Each window holds an explicit `Capabilities` set (default is strictly EMPTY).
- **Pre-Routing Capability Check**: Incoming methods are checked against the window's capability allowlist *before* dispatching to the handler registry.
- **Generic Error Invariant**: Both ungranted methods and unknown/non-existent methods return the exact same generic error string (`"Method not allowed or not found"`) to prevent capability enumeration.

### D. Shell-Level Security Controls (`wavical-shell`)
- **Origin Verification**: Every IPC request's URI origin is checked against `WindowConfig.allowed_origins`. Messages from iframes or unauthorized origins are dropped immediately.
- **Navigation Guard**: Top-level navigations are intercepted; only URLs matching the application origin allowlist are permitted. External sites are denied.
- **Popup & New-Window Blocking**: Intercepts `window.open()` and popup requests, returning `NewWindowResponse::Deny` by default.
- **Content-Security-Policy (CSP)**: Injected on every protocol response (default: `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self';`). Zero inline scripts (`'unsafe-inline'`) required.
- **Devtools Restrictions**: Devtools are compiled in only during debug builds (`cfg!(debug_assertions)`) and strictly disabled in release builds.

---

## 3. Structured Security Logging

All security violations and rejected actions emit structured `tracing` events with consistent field names:

| Security Event | Emitted When | Structured Fields |
|---|---|---|
| `capability_denied` | A webview requests an IPC method not granted in its capability allowlist. | `security_event`, `window_id`, `method`, `id`, `reason` |
| `malformed_message` | An IPC payload fails JSON syntax, schema, nesting depth, or type checks. | `security_event`, `window_id`, `payload` (truncated), `reason` |
| `oversize_message` | An incoming raw IPC message exceeds the 1 MB raw size limit. | `security_event`, `window_id`, `size`, `max_size`, `reason` |
| `origin_rejected` | An IPC message originates from an unauthorized URL or external iframe. | `security_event`, `window_id`, `origin`, `reason` |
| `path_rejected` | A `wavical://` protocol request fails path sanitization or canonical root containment. | `security_event`, `window_id`, `requested_path` (truncated), `reason` |
| `navigation_blocked` | The webview attempts to navigate top-level to an unauthorized external origin. | `security_event`, `window_id`, `target_url` (truncated), `reason` |
| `popup_blocked` | The webview requests opening a new window or popup. | `security_event`, `window_id`, `target_url` (truncated), `reason` |

### Log Sanitization & Truncation
- Payloads, requested paths, and navigation URLs in log events are automatically truncated via `truncate_payload` (max 64 bytes on valid UTF-8 boundaries) to prevent log injection and memory bloat.
- Secrets, tokens, and raw buffer contents are never written to logs.

---

## 4. Phase 3 AI Security Architecture & Trust Boundaries

Phase 3 introduces local and cloud AI inference (`ai.chat`), semantic indexation (`ai.index`), and similarity search (`ai.search`). AI capabilities operate across four distinct isolation boundaries:

```
┌─────────────────────────────────────────────────────────────┐
│                 Untrusted Webview Context                   │
│                                                             │
│   • Page JavaScript (window.wavical.invoke)                 │
│   • Threat assumptions: Untrusted inputs, model output XSS, │
│     attempted path injection, API key/URL parameter theft   │
└──────────────────────────────┬──────────────────────────────┘
                               │
                 Strict IPC & Pure Validator
             (ai.chat, ai.index, ai.search, ai.cancel)
                               │
┌──────────────────────────────▼──────────────────────────────┐
│                  Privileged Native Host                     │
│                                                             │
│   • Pure request validation (wavical-ipc)                   │
│   • Concurrency limiter (1 generation per window)           │
│   • Credential Manager / zeroizing memory (Zeroizing)       │
│   • Native OS consent dialogs & app-scoped consent store    │
│   • SQLite vector store & cosine search (wavical-ai)        │
└───────────────┬─────────────────────────────┬───────────────┘
                │ Local IPC                   │ HTTPS / TLS
                │ (Loopback TCP / Anonymous)  │ (Pinning, System Proxy)
┌───────────────▼─────────────┐ ┌─────────────▼───────────────┐
│     Engine Subprocess       │ │       Cloud AI Provider     │
│                             │ │                             │
│ • Child process (llama-srv) │ │ • Pinned host (baseUrl)     │
│ • Windows Job Object        │ │ • No cross-host redirects   │
│   (KILL_ON_JOB_CLOSE)       │ │ • Rustls certificate verif  │
│ • Cleared environment       │ │ • 1 MB req / 4 MB resp cap  │
│ • SHA-256 binary validation │ │ • Timeout 10s conn / 120s tot│
└─────────────────────────────┘ └─────────────────────────────┘
```

### 4.1 Four-Tier Trust Boundaries

1. **Untrusted Webview Context (Page)**:
   - May only invoke `ai.chat`, `ai.index`, `ai.search`, and `ai.cancel` if the respective capability is granted in `wavical.config.js`.
   - Cannot supply model paths, download URLs, base URLs, or API keys. Any extra parameters passed from the page are ignored or rejected.
   - Cannot trigger downloads or bypass consent prompts.
   - **Model Output Is Untrusted Data**: Generated text from local models or cloud endpoints is treated strictly as untrusted plain text. It is never executed as commands, evaluated as scripts, or interpolated into DOM without escaping.

2. **Privileged Native Host (`wavical-ipc`, `wavical-shell`, `wavical-ai`)**:
   - Enforces default-deny on `ai.*` methods; rejects unauthorized calls with `"Method not allowed or not found"`.
   - Validates all request payloads using pure validators:
     - `ai.chat`: max 50 messages, max 32,768 characters per message, max 128k characters total, valid roles (`system`, `user`, `assistant`), maxTokens 1–32768, temperature 0.0–2.0.
     - `ai.index`: collection names 1–64 characters `[a-z0-9_.-]`, no traversal, no Windows reserved names; max 100 docs per request, max 100,000 characters per document, metadata depth <= 5, max 20 metadata keys.
     - `ai.search`: query max 4,096 characters, topK between 1 and 20.
   - Concurrency limiter: enforces at most one active generation per window; excess concurrent requests are rejected immediately without queueing or thread starvation.
   - Enforces the `local-mode no-network guarantee`: when `ai.mode = "local"`, no outgoing network socket is ever opened.
   - Protects credentials using `Zeroizing<String>` in-memory storage, Windows Credential Manager (`wavical secret set/clear`), and complete redaction from logs, errors, and JSON IPC output.

3. **Isolated Engine Subprocess (`wavical-engine` / llama-server)**:
   - Spawned as an isolated child process tied to a Windows Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) to prevent orphaned processes on host crash or exit.
   - Subprocess environment variables are cleared, preventing inherited secrets or tokens.
   - Engine executable and GGUF model weights are strictly verified against SHA-256 hashes defined in the verified model catalog before execution.
   - Communication occurs solely over loopback localhost TCP (`127.0.0.1`) on dynamically assigned ephemeral ports.
   - Automatic idle shutdown kills the engine after `idleMinutes` of inactivity.

4. **External Network Boundary (Cloud API)**:
   - Certificate verification is permanently enforced via `rustls-platform-verifier` / `rustls`. There is no configuration option or code path to disable certificate verification.
   - Host pinning: only the host specified in `ai.cloud.baseUrl` is contacted.
   - HTTP redirects across different hosts are strictly disallowed.
   - Size limits: request body at most 1 MB; non-streamed response at most 4 MB; streamed output follows character caps.
   - Timeouts: 10 seconds connect timeout, 120 seconds total request timeout.
   - Native Consent Dialog: before any text leaves the machine to a cloud host, a native OS dialog is presented naming the service and destination. Consent is stored per-app and per-host, and cannot be triggered, suppressed, or inspected by the webview.

### 4.2 Enforcement Points Matrix

| Threat Vector | Enforcement Layer | Enforcement Mechanism |
|---|---|---|
| Unauthorized AI API access | `wavical-ipc` / `wavical-shell` | Default-deny capability check returning generic error |
| Excessive memory consumption (DoS) | `wavical-ipc` | Strict message length (32k), document length (100k), and array bounds |
| Model path traversal / local file read | `wavical-ai` | Model paths constructed exclusively from internal catalog & cache dir |
| Cross-app collection / SQL injection | `wavical-ai` | Strict collection regex (`^[a-z0-9_.-]{1,64}$`), parameterized SQLite queries |
| Rogue / tampered engine binary | `wavical-ai` | Cryptographic SHA-256 verification before child process spawn |
| Rogue / tampered model weights | `wavical-ai` | Cryptographic SHA-256 verification before engine invocation |
| Orphaned engine processes | `wavical-ai` | Windows Job Object with kill-on-close policy |
| Data exfiltration in local mode | `wavical-ai` | Local mode socket isolation; cloud module never instantiated |
| Secret leakage (API keys) | `wavical-ai` / `wavical-ipc` | `Zeroizing` zero-on-drop, `[REDACTED]` Debug/Display, no IPC exposure |
| Man-in-the-Middle on cloud calls | `wavical-ai` | Strict TLS verification; host pinning; redirect blocking |
| Silent cloud exfiltration | `wavical-ai` | Mandatory native OS consent prompt naming destination host |
| XSS via model generation | Webview frontend | Untrusted text handling; DOM node textContent binding |

### 4.3 Structured AI Security Audit Events

| Security Event | Emitted When | Structured Fields |
|---|---|---|
| `ai_capability_denied` | Webview invokes `ai.chat`, `ai.index`, or `ai.search` without capability grant. | `security_event`, `window_id`, `method`, `reason` |
| `ai_validation_failed` | An AI request payload violates schema limits (e.g. oversize text, malformed role). | `security_event`, `window_id`, `method`, `detail` |
| `ai_concurrency_rejected`| Window attempts concurrent AI generation while a generation is already active. | `security_event`, `window_id`, `reason` |
| `ai_consent_prompted` | Host displays native consent dialog for first cloud API request. | `security_event`, `app_id`, `host` |
| `ai_consent_denied` | User declines cloud transmission in native consent dialog. | `security_event`, `app_id`, `host` |
| `ai_engine_hash_mismatch`| Engine binary SHA-256 hash does not match expected catalog checksum. | `security_event`, `expected_hash`, `actual_hash` |
| `ai_model_hash_mismatch` | Model file SHA-256 hash does not match expected catalog checksum. | `security_event`, `model_id`, `expected_hash` |
| `ai_cloud_network_blocked`| Request attempted redirect to different host or network call in local mode. | `security_event`, `target_host`, `reason` |
