# Wavical Network Modes (`open` / `safe` / `ultra-safe`)

Wavical controls what external content, inline scripts, APIs, and links an application may use via three build/run modes:

| Mode | Dev Command | Build Command | Summary |
|---|---|---|---|
| **open** | `wavical dev` | `wavical build --allow-open` | Allows any `https:` external content, inline scripts/styles, and `https:` system-browser links while keeping baseline protections. |
| **safe** | `wavical dev-safe` | `wavical build-safe` | Allows only origins listed in `wavical.allowlist.json` and hashes static inline `<script>` blocks automatically. |
| **ultra-safe** | `wavical dev-ultra-safe` | `wavical build-ultra-safe` | Strict local-only CSP (unchanged from Phase 0). Inline `<script>` blocks are extracted into local files automatically. |

> **Breaking change from Phases 0–3:** `wavical dev` now runs in **open** mode. Use `wavical dev-ultra-safe` for the strict Phase 0–3 behavior, or `wavical dev-safe` for allowlisted origins.

---

## `wavical.allowlist.json` Format

Place `wavical.allowlist.json` in the project root:

```json
{
  "version": 1,
  "scripts": ["https://cdn.tailwindcss.com", "https://cdnjs.cloudflare.com", "https://unpkg.com"],
  "styles":  ["https://fonts.googleapis.com"],
  "fonts":   ["https://fonts.gstatic.com"],
  "images":  ["https://www.designarena.ai", "https://*.company.com"],
  "media":   [],
  "connect": ["https://api.company.com", "wss://live.company.com"],
  "frames":  [],
  "navigate": ["https://login.company.com"],
  "openExternal": ["https://docs.company.com"],
  "openExternalSchemes": ["mailto", "tel"],
  "inlineStyles": true,
  "dataImages": true,
  "remoteUi": { "origin": "https://app.company.com", "bridge": ["core.ping"] },
  "localExceptions": [
    { "origin": "http://localhost:3000", "reason": "local backend API" }
  ]
}
```

### Rules
- Entries are **origins only** (paths, queries, fragments, userinfo, and duplicates are rejected).
- `https:` everywhere; `wss:` in `connect` only; `http:`/`ws:` and IP literals only in `localExceptions` (with a mandatory `reason`).
- `openExternalSchemes`: optional array accepting only `"mailto"` and `"tel"` (allowed by default in `open` mode; never allowed in `ultra-safe`).
- Wildcards: leading `*.` with at least two labels (`https://*.company.com`); public-suffix wildcards (e.g. `*.co.uk`, `*.github.io`) are rejected.
- `inlineStyles` and `dataImages` default to `false`.

---

## External Links (`window.externalLinks`, `mailto:`, `tel:`)
- **`window.externalLinks` (`"browser"` | `"in-app"`, default `"browser"`):**
  - By default (`"browser"`), user-initiated top-level link clicks (`<a href>`) to another `https:` origin open in the system browser while the app stays on its page. Redirect-based login flows and app-initiated navigation (e.g., to `navigate` origins in `safe` mode) still navigate in-app.
  - Set `window: { externalLinks: 'in-app' }` in `wavical.config.js` if an app in `open` mode intentionally wants external links to navigate inside the app window. (`window.wavical` and IPC remain strictly disabled on foreign origins.)
- **`mailto:` and `tel:` links:**
  - Routed through the same validated system opener as external `https:` links (never navigated in-app), with length limits, control-character/header-injection rejection, and scheme-only logging.
  - Allowed by default in `open` mode, allowed in `safe` mode only when `"mailto"` / `"tel"` is listed in `openExternalSchemes`, and never allowed in `ultra-safe`. Other schemes (`sms:`, `whatsapp:`, `skype:`, `ms-*`, `file:`) remain blocked in all modes.

---

## Differences from a browser
- **Plain `http://` stays blocked in `open` mode unless listed in `localExceptions`:** Even in `open` mode, `http://` resources and navigations are blocked unless the exact origin is added to `localExceptions` in `wavical.allowlist.json` with a `reason`.
- **Popup-based logins (`window.open` OAuth) are not supported yet:** `window.open` and `target="_blank"` never spawn child app windows (they route `https:` links to the system browser or are denied). Redirect-based OAuth/SSO login flows work in-app.
- **WebView2 tracking prevention can block third-party storage in embeds:** Embedded third-party frames or widgets that rely on third-party cookies or `localStorage` may be restricted by WebView2's built-in tracking prevention.

---

## Inline Scripts & Event Handlers per Mode
- **`open`**: Inline `<script>` blocks and inline event handlers (`onclick=`, `onerror=`) are allowed.
- **`safe`**: Static inline `<script>` blocks are parsed with `html5ever`, hashed (SHA-256), and added to that page's CSP. Runtime-created inline scripts and inline event handlers are blocked (and reported with file, line, and `addEventListener` fix).
- **`ultra-safe`**: Inline `<script>` blocks are extracted into local files (`src/.wavical-extracted/` in dev, inside the build output in builds) preserving order and `type="module"`. Inline event handlers are reported, not rewritten.

---

## `localExceptions` & Company Certificates
- Local/intranet `http://` or `ws://` hosts are blocked by default in all modes. Add them to `localExceptions` with a `reason` (allowed in `open` and `safe`; **refused** in `ultra-safe`).
- **Company Certificates:** Wavical never provides an "ignore certificate errors" option. If your company uses an internal CA, IT should install the root CA certificate into the Windows Trusted Root Certification Authorities store; WebView2 trusts it automatically.

---

## Remote UI (`window.remoteUrl`) Risks
- **`open`**: `window.remoteUrl` receives all app capabilities granted in `wavical.config.js` (top frame only).
- **`safe`**: `window.remoteUrl` origin must match `remoteUi.origin`, and its bridge is restricted to `remoteUi.bridge`.
- **`ultra-safe`**: `window.remoteUrl` is refused.
- **Important limitation:** Wavical cannot set or tighten a remote website's CSP header. Enforcement on remote pages relies on the navigation handler, new-window handler, and Rust IPC origin check.

---

## `dev.url` (React / Vite Dev Servers)
Configure `dev: { url: 'http://localhost:5173' }` in `wavical.config.js` (only `localhost` or `127.0.0.1` with an explicit port is accepted).
- Used **only** by `dev`, `dev-safe`, and `dev-ultra-safe` (and their WebSocket HMR).
- **Never** used by builds; built apps load from `build.frontendDir` (default `src`).

### Dev Shell Remote Debugging (`--remote-debugging-port`)
- `wavical-shell-dev` (`dev-shell` feature) keeps `--remote-debugging-port` (`WAVICAL_DEV_REMOTE_DEBUGGING_PORT`) for loopback-only (`127.0.0.1`), dev-only DevTools/CDP attachment.
- Production `wavical-shell.exe` and `dist/` builds never include `--remote-debugging-port`, DevTools, or test hooks (`test-hooks`).

---

## `wavical allowlist suggest`
Run `wavical dev-safe --record` or `wavical dev-ultra-safe --record` to record blocked items (`{ type, origin, count }` only, never full URLs or queries) into `.wavical/recorded-blocks.json`, then run `wavical allowlist suggest` to review and selectively add `https:` origins. Wildcards, `http`/`ws`, `localExceptions`, `remoteUi`, bridge capabilities, `inlineStyles`, and `dataImages` are never auto-proposed.

---

## `ultra-safe` Guarantees, Network-Capable Capabilities, and Residual Risks
- **Guarantees:** Strict Phase 0 CSP (`default-src 'self'`, no `'unsafe-inline'`, no external origins), serve-time SHA-256 verification of built assets, `X-DNS-Prefetch-Control: off`, `--dns-prefetch-disable`, and `OptimizationHints,NetworkTimeServiceQuerying` disabled.
- **Network-capable capabilities:** `backend.invoke` and `ai.chat` (in `cloud` or `local-first` mode) can still transmit data off the machine if granted. Set `ultraSafe.blockNetworkCapabilities: true` to fail `build-ultra-safe` if any are granted.
- **Known residual risks (verified in WP1):**
  1. **WebRTC (`RTCPeerConnection` ICE/STUN UDP):** Not governed by CSP `connect-src`; WebView2 CLI flags did not block STUN UDP packets in testing.
  2. **Runtime-injected `<link rel="preconnect">` / `<link rel="dns-prefetch">`:** Not blocked by CSP or `--disable-preconnect`.
  3. **Evergreen WebView2 runtime background traffic:** OS-managed Microsoft Edge WebView2 component/telemetry traffic occurs outside page CSP.
