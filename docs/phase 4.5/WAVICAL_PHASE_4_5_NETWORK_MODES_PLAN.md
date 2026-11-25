# Wavical — Phase 4.5 Build Plan: Network Modes (open / safe / ultra-safe)

**Goal:** let apps use external content (CDN scripts, styles and fonts, API calls, a whole UI loaded from a URL, links that open in the browser, inline scripts) by choosing one of three modes, without losing the protections that matter. Also: make CDN-based apps work offline and in ultra-safe mode through `wavical vendor`, and let React/Vite developers use their dev server.

**Why now:** a real app built on Wavical relied entirely on external content, and strict mode blocked it. The reference app (a single HTML page) used 6 external origins, 2 inline script blocks, 5 inline event handlers, runtime-injected styles and `data:` images. An allowlist of links alone would not have made it work. Phase 5 (installers) should ship with this in place.

**Estimate:** 3–4 weeks part-time with AI. It touches Phase 0 security code (CSP, navigation, origin check), so testing and the audit take most of the time.

---

## 0. Gate before starting

- [ ] Phase 4 audit (A8) has no open Critical or High findings, or each is accepted in writing.
- [ ] CI green, `Cargo.lock` committed.

---

## 1. Decisions (agreed with Dhananjay)

### 1.1 Commands

| Mode | Run | Build | Meaning |
|---|---|---|---|
| **open** | `wavical dev` | `wavical build --allow-open` | external content allowed (https), baseline protections kept |
| **safe** | `wavical dev-safe` | `wavical build-safe` | only what is listed in `wavical.allowlist.json`; your inline scripts are hashed automatically |
| **ultra-safe** | `wavical dev-ultra-safe` | `wavical build-ultra-safe` | today's strict CSP, unchanged; vendored files and extracted inline scripts are local, so they are allowed |

- `wavical build` without `--allow-open` refuses and explains the three choices. With the flag it prints a loud warning and records `"mode": "open"` in the build manifest.
- The mode is **fixed at build time** in the build manifest, which is part of the signed update payload (Phase 4).
- `wca` aliases work the same.
- **Behaviour change:** `wavical dev` was strict in Phases 0–3; it now means open. One-time notice per project, changelog entry, docs.
- **Mismatch warning** when the build mode differs from the last dev mode.

### 1.2 What each mode controls

| Content | open | safe | ultra-safe |
|---|---|---|---|
| Scripts, styles, fonts, images, media from other origins | any https | only listed origins, per type | blocked (use `wavical vendor`) |
| Inline `<script>` blocks | allowed | allowed only if the exact bytes match a hash computed at dev/build time | blocked at runtime; `build-ultra-safe` and `dev-ultra-safe` move them into local files automatically (CSP unchanged) |
| Inline event handlers (`onerror=`, `onclick=`) | allowed | blocked; reported with file and line | blocked; reported with file and line |
| Inline styles and runtime-injected `<style>` (for example Tailwind's CDN) | allowed | only with `"inlineStyles": true` | as today (see 1.13 for known limitations) |
| `data:` images (CSS backgrounds, icons) | allowed | only with `"dataImages": true` | as today |
| API calls (fetch, XHR, WebSocket, EventSource) | any https / wss | only listed in `connect` | blocked |
| iframes | any https | only listed in `frames` | blocked |
| Top-level navigation to other sites | any https | only listed in `navigate` | blocked |
| Links, `window.open`, `target=_blank` | system browser (https only) | system browser, only listed in `openExternal` | blocked |
| Whole UI from a URL (`window.remoteUrl`) | allowed | allowed if the origin is listed in `remoteUi` | not allowed |
| Bridge on the remote UI | configured `remoteUrl` origin only, top frame only, app's normal capabilities | only if `remoteUi.bridge` lists capabilities (subset) | n/a |
| Plain http, localhost, intranet hosts | only through `localExceptions` | only through `localExceptions` | blocked |
| Dev server (`dev.url`, see 1.12) | dev commands only | dev commands only | dev commands only |

### 1.3 Baseline kept in every mode (not negotiable)

1. Capability model (default-deny; config grants).
2. IPC origin check: the bridge answers only the app's own origin, the configured remote UI origin where allowed, and the `dev.url` origin in dev commands. Never other sites, iframes, popups or workers.
3. Blocked schemes: `file:`, `javascript:` navigation, `data:` and `blob:` top-level navigation, `ms-*` and other custom protocol launches, `ftp:`.
4. No devtools or remote debugging in release builds.
5. Certificate errors fail closed. **No "ignore certificate errors" option**, in any mode.
6. Plain `http` only through explicit `localExceptions` (or `dev.url` in dev commands).
7. External links open in the system browser through one validated function (https only, length-limited, rate-limited).
8. `object-src 'none'` and `base-uri 'self'` on Wavical's own pages in every mode.

### 1.4 The allowlist file (`wavical.allowlist.json`, project root)

```json
{
  "version": 1,
  "scripts":      ["https://cdnjs.cloudflare.com"],
  "styles":       ["https://fonts.googleapis.com"],
  "fonts":        ["https://fonts.gstatic.com"],
  "images":       ["https://*.company.com"],
  "media":        [],
  "connect":      ["https://api.company.com", "wss://live.company.com"],
  "frames":       [],
  "navigate":     ["https://login.company.com"],
  "openExternal": ["https://docs.company.com"],
  "inlineStyles": false,
  "dataImages":   false,
  "remoteUi": { "origin": "https://app.company.com", "bridge": ["core.ping"] },
  "localExceptions": [ { "origin": "http://localhost:3000", "reason": "local API" } ]
}
```

Rules, validated by the CLI and again by the shell:
- Entries are origins, not paths (paths are rejected with a message).
- `https` everywhere; `wss` in `connect` only; `http` and `ws` only in `localExceptions`.
- Wildcards: only a leading `*.` with at least two labels after it. `*`, `*.com`, `*.*`, `https://*` are rejected. WP1 decides on public-suffix checks (`*.co.uk`).
- Hosts lowercased and converted to punycode; CLI shows punycode so look-alikes are visible.
- No userinfo, no IP literals except in `localExceptions`, no duplicates, unknown keys rejected.
- `remoteUi.bridge` must be a subset of the capabilities granted in `wavical.config.js`.
- `localExceptions` entries need a `reason`; a warning prints on every dev and build.

Example for the reference app in safe mode:

```json
{
  "version": 1,
  "scripts": ["https://cdn.tailwindcss.com", "https://cdnjs.cloudflare.com", "https://unpkg.com"],
  "styles":  ["https://fonts.googleapis.com"],
  "fonts":   ["https://fonts.gstatic.com"],
  "images":  ["https://www.designarena.ai"],
  "inlineStyles": true,
  "dataImages": true
}
```

Its two inline scripts would be hashed automatically, and its five `onerror="this.remove()"` handlers would be reported for moving into the main script.

### 1.5 Internal hosts and local connectors (off by default)

- Nothing local or plain-http is reachable in any mode by default.
- Enable a local service or intranet host by adding it to `localExceptions` with a reason (open and safe; refused in ultra-safe).
- Company certificates: if IT installed the company CA in Windows, WebView2 trusts internal https automatically. No bypass for untrusted certificates; `docs/NETWORK_MODES.md` explains how IT installs the CA.

### 1.6 Remote UI

- `window.remoteUrl: 'https://app.company.com/'` in config.
- open: allowed; bridge only for that exact origin, top frame; other sites it navigates to get no bridge; loud warning in CLI and build.
- safe: origin must equal `remoteUi.origin`; bridge only with `remoteUi.bridge` capabilities.
- ultra-safe: config error.
- Friendly offline page with retry; certificate errors fail closed with a friendly page.
- Wavical cannot set a remote website's CSP. Enforcement there relies on the navigation handler, new-window handler, bridge origin check and, if wry supports it, request filtering (WP1). Docs say this plainly.

### 1.7 `wavical allowlist suggest`

- `wavical dev-safe --record` or `dev-ultra-safe --record` records blocked items as type, origin and count only (never full URLs, query strings or content).
- `wavical allowlist suggest` proposes additions grouped by type and writes them only after confirmation.
- Never proposes wildcards, `http`, `localExceptions`, `remoteUi`, bridge capabilities, `inlineStyles` or `dataImages`. Those are listed separately as "needs your decision".

### 1.8 Logging

Every block emits a tracing event: mode, type, origin (no full URL), window id, rule. Open-mode builds log a startup event saying so.

### 1.9 Named limits (constants with boundary tests)

| Limit | Value |
|---|---|
| allowlist file size | 256 KB |
| entries per list | 200 |
| `localExceptions` entries | 20 |
| origin length / host length | 2,048 / 253 characters |
| openExternal URL length | 2,048 characters |
| openExternal calls | 5 per 10 s per window |
| recorded events in `--record` | 10,000 per session |
| inline scripts hashed per page | 100 |
| vendored file size / total / file count | 20 MB / 200 MB / 500 |

### 1.10 Inline script hashing (safe mode)

- `dev-safe` (on every reload) and `build-safe` parse each HTML file with a proper HTML parser, compute SHA-256 of every inline `<script>` block's exact bytes, and add the hashes to that page's CSP.
- A script that is changed or injected later does not match and is blocked.
- Inline event handlers are not hashed. They are reported (file, line, attribute) with the one-line fix (`addEventListener`).
- Scripts created at runtime (`document.createElement('script')` with inline text) are blocked. Scripts loaded by `src` follow the allowlist.

### 1.11 `wavical vendor` (offline copies of CDN files)

- `wavical vendor` scans the HTML and CSS for external scripts, styles, fonts and images, downloads them through the shared verified downloader (Phase 4, https only), and stores them in `src/vendor/<host>/<path>`.
- It follows `url()` and `@import` inside downloaded CSS one level (needed for Google Fonts and their font files), and rewrites those references to the local copies.
- It rewrites the HTML references to the local copies (keeping a backup of each changed file and showing a diff).
- **Pinned versions only.** A URL without a recognizable version (for example `https://cdn.tailwindcss.com`) is refused with a message asking for a pinned URL. `--allow-unpinned <url>` takes a one-time snapshot, with a warning that it will never update by itself.
- `vendor.lock.json` records URL, SHA-256, size, fetched date and license (when the registry or file header states one). The CLI prints the licenses: vendoring redistributes these libraries inside your app, and you're responsible for their licenses.
- `wavical vendor --check` verifies every file against the lock. `wavical vendor --update` refetches pinned URLs and shows what changed.
- Vendoring happens only through the CLI, never at runtime.
- After vendoring, the app works **offline**, and its scripts are `'self'`, so ultra-safe accepts them.

### 1.12 `dev.url` (React / Vite dev servers)

- `dev: { url: 'http://localhost:5173' }` in config. Only `http://localhost:<port>`, `http://127.0.0.1:<port>` or the https versions are accepted.
- **Dev commands only** (`dev`, `dev-safe`, `dev-ultra-safe`): the window loads that URL, that one origin and its WebSocket (for hot reload) are allowed, and the bridge answers that origin.
- **Builds never use it.** A build loads the built files from the configured frontend folder (`build.frontendDir`, default `src`, for example `dist` for a Vite app).
- The mode's other rules still apply in dev (for example dev-safe still enforces the allowlist for everything else).

### 1.13 Ultra-safe isolation (Dhananjay's request: "even if a script is injected, data does not leave the app")

1. **Local only.** CSP stays exactly as today: no external origins, no inline scripts.
2. **Inline scripts are extracted, not allowed.** `build-ultra-safe` and `dev-ultra-safe` move inline `<script>` blocks into local files (`src/.wavical-extracted/` in dev, inside the build in builds) and rewrite the HTML. The CSP string doesn't change. Inline event handlers are reported, not rewritten.
3. **Vendored files** are served as local files, and each served file is checked against its SHA-256 in the build manifest (tamper detection on disk; a mismatch serves a 404 and logs).
4. **Leak channels CSP doesn't cover** are closed in ultra-safe if WP1 finds a reliable way, otherwise documented as known residual risks: WebRTC (ICE/STUN traffic), DNS prefetch and preconnect, and anything else WP1 finds.
5. **Capabilities that can send data off the machine.** An injected script could still use granted capabilities. `build-ultra-safe` lists every granted capability that can move data off the machine (`backend.invoke`, since the backend has unrestricted network; `ai.chat` when cloud or local-first mode is set; any later network capability) and records the list in the build manifest. With `ultraSafe.blockNetworkCapabilities: true` in config, the build fails if any such capability is granted.
6. **Known limitations, stated plainly:** runtime CSS generators like Tailwind's CDN, inline `style=""` attributes and inline event handlers can't run in ultra-safe. The fix for Tailwind is compiling it at build time (the normal Tailwind setup), which Wavical doesn't do for you in this phase.

**What this means for the reference app in ultra-safe:** after `wavical vendor` (with a pinned Tailwind URL), automatic inline script extraction and moving the five `onerror` handlers into the main script, the remaining blockers are Tailwind's runtime style injection and the `style="opacity:0"` attributes. It runs fully in safe mode with `inlineStyles: true`.

### 1.14 Build warnings (safe and open)

- External script with no version in its URL (unpinned).
- External script without an `integrity` attribute.
- "This app needs internet: N external files. Run `wavical vendor` to make it work offline."

### 1.15 Where the code goes

- **Pure crate** (`wavical-netpolicy`): allowlist parser, origin matcher, CSP generators, decision functions, HTML/CSS scanner results model, inline-script hashing, URL version detection, vendor lock parser. No tao, wry, network or OS code.
- **CLI:** commands, warnings, `allowlist suggest`, `vendor`, inline extraction.
- **Shell:** applies CSP and decisions, serve-time hash check, leak-channel closing, opener.

---

## 2. Scope

**In scope:** three modes for dev and build; allowlist; local exceptions; remote UI; system-browser links; inline script hashing (safe); inline extraction (ultra-safe); `wavical vendor`; `dev.url`; ultra-safe leak-channel work; capability exfiltration report; build warnings; `allowlist suggest`; docs; tests; audit.

**Out of scope:** per-window modes; certificate pinning; proxy settings; any certificate-error bypass (never); compiling Tailwind or other CSS frameworks at build time; rewriting inline event handlers automatically; changing the Phase 3 cloud AI or Phase 4 updater/download allowlists.

---

## 3. Work packages

### WP1: Research (no feature code)
Sources and versions in `docs/WAVICAL_PHASE_4_5_DECISIONS.md`:
1. wry CSP and control over remote pages (navigation, new window, request filtering).
2. Whether initialization scripts run in remote pages and frames; keeping the bridge inert where not allowed.
3. Opening URLs in the system browser safely (crate, unsafe?).
4. Collecting blocked events for `--record`.
5. URL parsing, punycode, public suffix list.
6. CSP needs of common CDN libraries; open and safe CSP strings.
7. HTML and CSS parser crates for the scanner, extraction and vendor rewriting (robust on real-world HTML).
8. How to recognise a pinned version in common CDN URLs (cdnjs, jsDelivr, unpkg, Google Fonts, Tailwind).
9. **Ultra-safe leak channels in WebView2:** WebRTC, DNS prefetch, preconnect, and others; what can reliably close each (browser arguments, permission handlers, headers) and what can't.
10. Local multi-origin test server approach (no hosts-file edits).
Stop with a decision list.

### WP2: Pure policy crate
Allowlist parser; origin matcher; CSP generators for the three modes (ultra-safe identical to today); decision functions; inline-script hashing; scanner result model; version detection; vendor lock parser; limits. Heavy tests and proptests.

### WP3: CLI commands, dev.url, build manifest, warnings
Six commands; `--allow-open`; manifest fields; mismatch warning; one-time notice; `window.remoteUrl` rules; `dev.url` and `build.frontendDir`; inline handler reports; build warnings from 1.14; `ultraSafe.blockNetworkCapabilities` and the capability report.

### WP4: Shell enforcement
Mode from manifest; CSP per mode on every response (with page-specific hashes in safe); handlers; opener; `localExceptions`; bridge origin rule including `dev.url` in dev; serve-time hash check; ultra-safe leak-channel closing from WP1; logging; baseline in every mode.

### WP5: Remote UI
As in 1.6.

### WP6: `allowlist suggest`
As in 1.7.

### WP7: `wavical vendor` and inline extraction
As in 1.11 and 1.13 items 2–3.

### WP8: Integration, attack pages, docs, self-review
Attack pages per mode, local multi-origin server, the reference app as a manual fixture, full regression in ultra-safe, `docs/NETWORK_MODES.md` and `docs/VENDOR.md`, SECURITY_MODEL update, changelog, hostile review, mutation proofs, combined manual checklist.
