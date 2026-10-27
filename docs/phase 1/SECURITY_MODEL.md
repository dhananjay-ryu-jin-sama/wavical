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
