# Wavical Phase 4.5 — WP1 Research & Architecture Decisions

This document records the WP1 research findings, crate versions, upstream source links, empirical WebView2/wry probe results, and open decisions required before implementing WP2–WP8 of [WAVICAL_PHASE_4_5_NETWORK_MODES_PLAN.md](file:///c:/Users/DHANANS/OneDrive%20-%20Daimler%20Truck/Documents/Dhananjay/coading/Arka-Wavical/docs/phase%204.5/WAVICAL_PHASE_4_5_NETWORK_MODES_PLAN.md).

---

## 1. `wry` CSP and Control Over Remote Pages

- **Crate & Version:** `wry` `0.57.0` ([docs.rs/wry/0.57.0](https://docs.rs/wry/0.57.0/wry/)), `tao` `0.37.1` ([docs.rs/tao/0.37.1](https://docs.rs/tao/0.37.1/tao/)) — already in workspace `Cargo.toml`.

### What Wavical CAN enforce on a remote UI page (`window.remoteUrl`)
1. **Top-level navigation (`WebViewBuilder::with_navigation_handler`)**: Intercepts top-level navigation requests. Can enforce `https:` only, block `file:`, `javascript:`, `data:`, `blob:`, `ftp:`, and custom schemes (`ms-*`), and restrict navigation to `remoteUi.origin` + `navigate` allowlist in `safe` mode (or any `https:` in `open` mode).
2. **Popups & new windows (`WebViewBuilder::with_new_window_req_handler`)**: Intercepts `window.open()` and `target="_blank"`. Returns `NewWindowResponse::Deny` or routes validated `https://` links to the system browser opener.
3. **IPC bridge origin enforcement (`WebViewBuilder::with_ipc_handler`)**: Every IPC message carries `req.uri()`. Rust parses the origin and drops/rejects messages from any origin other than the allowed bridge origin (top frame only).
4. **Permission requests (`ICoreWebView2::add_PermissionRequested`)**: WebView2 exposes permission prompts (camera, microphone, geolocation, notifications, clipboard). `wry` 0.57.0 registers a default `PermissionRequested` handler internally.

### What Wavical CANNOT enforce on a remote UI page
1. **Remote page's Content-Security-Policy (CSP)**: `wry` 0.57.0 registers WebView2's `WebResourceRequested` filter **only** for custom protocols (`wavical://` / `http(s)://wavical.localhost`), not for `http://` or `https://` remote origins (`wry-0.57.0/src/webview2/mod.rs:1003-1022`). Furthermore, WebView2's `WebResourceResponseReceived` event is read-only ([Microsoft Learn: WebView2 WebResourceResponseReceived](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2.webresourceresponsereceived)), so response headers (including `Content-Security-Policy`) cannot be mutated on live HTTPS responses without proxying every subresource through a custom Rust HTTP client (which would break browser cookies, TLS session semantics, and caching). Therefore, **a remote UI page's CSP is controlled solely by the remote server**, not by Wavical.
2. **Subresource request filtering on remote pages**: `wry` 0.57.0 does not expose a general `WebResourceRequested` callback for `https://` subresources (scripts, images, fetch, XHR) loaded by a remote page.

---

## 2. Initialization Scripts in Remote Pages & Iframes; Keeping the Bridge Inert

- **Crate & Version:** `wry` `0.57.0` — `WebViewBuilder::with_initialization_script` and `WebViewBuilder::with_initialization_script_for_main_only` ([docs.rs/wry/0.57.0/wry/struct.WebViewBuilder.html#method.with_initialization_script](https://docs.rs/wry/0.57.0/wry/struct.WebViewBuilder.html#method.with_initialization_script)).
- **Source Finding (`wry-0.57.0/src/lib.rs:1004, 1029`):**
  `wry`'s own documentation explicitly states: *"Windows: scripts are always added to subframes regardless of the `for_main_frame_only` option."*

### Empirical Throwaway Probe (`wavical-wp1-probe`)
Tested with a standalone throwaway `tao 0.37.1` + `wry 0.57.0` app in `%TEMP%\wavical-wp1-probe` (not real Wavical code) loading a local HTTP main page (`http://127.0.0.1:63953/`) embedding a cross-origin iframe (`http://127.0.0.1:<port2>/frame.html`):
- `with_initialization_script(...)` **ran on the remote HTTP top-level page** and posted `IPC|uri=http://127.0.0.1:63953/|body=INIT_RUN|isTop=true|origin=http://127.0.0.1:63953`.
- The iframe (`HTTP_HIT|frame|GET /frame.html HTTP/1.1`) loaded, and per `wry`'s Windows implementation, WebView2's `AddScriptToExecuteOnDocumentCreated` injects initialization scripts into all frames.

### Design Decision
1. **JS-side inertia (convenience):** `WAVICAL_INIT_SCRIPT` must immediately check `if (window.top !== window.self) return;` and check `window.location.origin` against the allowed bridge origin(s) before defining `window.wavical`. On disallowed origins or iframes, `window.wavical` is never created.
2. **Rust-side enforcement (security boundary):** `with_ipc_handler` in `wavical-shell` parses `req.uri()` via `url::Url` and accepts messages **only** from:
   - the app's own custom-protocol origin,
   - `window.remoteUrl` origin (when allowed by mode/allowlist, top frame only), or
   - `dev.url` origin (in dev commands only).
   All other origins, iframes, popups, and workers are dropped and logged as `origin_rejected`.

---

## 3. Opening URLs in the System Browser Safely

- **Candidate Crates:**
  - **`open` `5.4.5`** ([docs.rs/open/5.4.5](https://docs.rs/open/5.4.5/open/), [crates.io/crates/open](https://crates.io/crates/open)): On Windows, calls Win32 `ShellExecuteW` directly (avoiding `cmd.exe /c start` shell-injection risks). Uses `unsafe` FFI internally behind a safe Rust API.
  - **`webbrowser` `1.0.x`** ([docs.rs/webbrowser](https://docs.rs/webbrowser/)): Cross-platform browser launcher; heavier than `open` for Windows-first `ShellExecuteW`.
- **Decision:** Use **`open` `5.4.5`** in `wavical-shell` (not in `wavical-netpolicy`, keeping the pure crate `#![forbid(unsafe_code)]` with zero OS dependencies). Wrap `open::that_detached` in **one validated function** that:
  1. Parses the URL with `url::Url::parse`.
  2. Requires `scheme() == "https"` (rejects `http:`, `file:`, `javascript:`, `data:`, `blob:`, `ftp:`, `ms-*`).
  3. Rejects userinfo (`username`/`password`).
  4. Enforces length limit `<= 2,048` characters (Plan 1.9).
  5. Enforces mode policy (`open`: any `https:`; `safe`: origin in `openExternal`; `ultra-safe`: blocked).
  6. Enforces per-window rate limit: `5` calls per `10 s` (Plan 1.9).

---

## 4. Collecting Blocked Events for `--record`

- **Rust-side blocks** (navigation handler, new-window/popup handler, `wavical://` path/protocol rejections, IPC origin rejections): Recorded directly in `wavical-shell` into a bounded session buffer (`MAX_RECORDED_EVENTS = 10_000`, Plan 1.9) keyed by `(BlockType, origin)` with a count.
- **Browser CSP blocks** (scripts, styles, fonts, images, media, `connect-src`, `frame-src` blocked by WebView2's CSP engine on local pages): WebView2 does not emit a native Rust callback for CSP violations. In `--record` dev mode only, `WAVICAL_INIT_SCRIPT` (on Wavical's own origin) attaches a `document.addEventListener('securitypolicyviolation', ...)` listener that extracts **only** `effectiveDirective` (mapped to allowlist category) and `origin` (parsed from `blockedURI`, never full URLs, query strings, or content) and sends it to Rust via a dedicated internal/dev-only recorder path.
- Recorded output (`wavical-recorded.json` or session artifact) stores **type, origin, and count only** (Plan 1.7).

---

## 5. URL Parsing, Punycode, and Public Suffix List

- **`url` `2.5.8`** ([docs.rs/url/2.5.8](https://docs.rs/url/2.5.8/url/), [crates.io/crates/url](https://crates.io/crates/url)):
  - Pure Rust WHATWG URL Standard parser (`idna` crate for punycode/UTS #46 normalization).
  - Normalizes scheme, lowercases ASCII/punycode host, strips default ports (`https:443`, `http:80`, `wss:443`, `ws:80`), and exposes `username()`, `password()`, `host()` (`Host::Domain` vs `Host::Ipv4`/`Ipv6`).
- **`psl` `2.1.245`** ([docs.rs/psl/2.1.245](https://docs.rs/psl/2.1.245/psl/), [crates.io/crates/psl](https://crates.io/crates/psl)):
  - Compiles the Mozilla Public Suffix List into static Rust code (`no_std`, zero network calls, zero `unsafe`).
  - Used to reject public-suffix wildcards such as `*.co.uk`, `*.com.au`, `*.github.io` in `wavical.allowlist.json`.
- **Decision:** Add `url = "2.5.8"` and `psl = "2.1.245"` to `wavical-netpolicy`.

---

## 6. CSP Needs of Common CDN Libraries; Open and Safe CSP Strings

### Five Popular CDN Libraries / Design Systems & Their CSP Requirements

| Library / Design System | Typical CDN Origin(s) | Required CSP Directives | Notes & Sources |
|---|---|---|---|
| **1. Tailwind CSS Play CDN** | `https://cdn.tailwindcss.com` | `script-src https://cdn.tailwindcss.com 'unsafe-eval'`; `style-src 'unsafe-inline'` | Play CDN is a runtime JS compiler that evaluates config/utilities (`new Function` / `'unsafe-eval'`) and injects `<style>` tags (`'unsafe-inline'`). [Tailwind Play CDN docs](https://tailwindcss.com/docs/installation/play-cdn) |
| **2. Google Fonts** | `https://fonts.googleapis.com` (CSS), `https://fonts.gstatic.com` (woff2) | `style-src https://fonts.googleapis.com`; `font-src https://fonts.gstatic.com` | No `'unsafe-inline'` or `'unsafe-eval'` required. [Google Fonts CSS2 API](https://developers.google.com/fonts/docs/css2) |
| **3. Bootstrap 5** | `https://cdn.jsdelivr.net` or `https://cdnjs.cloudflare.com` | `style-src <cdn>`; `script-src <cdn>` | Bootstrap 5 uses Popper v2 (no `'unsafe-eval'`, unlike Bootstrap 4's Popper v1). Inline `element.style` positioning works without `'unsafe-inline'` in `style-src` unless `style-src-attr 'none'` is set. [Bootstrap CSP](https://getbootstrap.com/docs/5.3/getting-started/browsers-devices/#content-security-policy) |
| **4. Font Awesome** | `https://cdnjs.cloudflare.com` / `https://cdn.jsdelivr.net` (CSS) or `https://kit.fontawesome.com` (Kit) | CSS+WebFonts: `style-src <cdn>`; `font-src <cdn>`. JS Kit: `script-src https://kit.fontawesome.com`; `connect-src https://ka-f.fontawesome.com`; `style-src 'unsafe-inline'` (unless `data-auto-add-css="false"`) | [Font Awesome CSP docs](https://docs.fontawesome.com/web/dig-deeper/security) |
| **5. Alpine.js / htmx** | `https://unpkg.com` or `https://cdn.jsdelivr.net` | Standard Alpine: `script-src <cdn> 'unsafe-eval'`. `@alpinejs/csp`: `script-src <cdn>`. htmx: `script-src <cdn>`; `connect-src <api>` | Standard Alpine uses `new Function()` for `x-data` expressions; `@alpinejs/csp` avoids `'unsafe-eval'`. [Alpine CSP docs](https://alpinejs.dev/advanced/csp) |

### Recommended CSP Strings

- **`ultra-safe` (UNCHANGED — Plan 1.13 & Rule 4):**
  ```text
  default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-src 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self';
  ```

- **`open` mode (recommended):**
  ```text
  default-src 'self' https:; script-src 'self' https: 'unsafe-inline' 'unsafe-eval'; style-src 'self' https: 'unsafe-inline'; img-src 'self' https: data: blob:; font-src 'self' https: data:; media-src 'self' https:; connect-src 'self' https: wss:; frame-src 'self' https:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self' https:;
  ```
  *(Plus any `localExceptions` origins in their respective directives, and `dev.url` + its `ws:`/`wss:` origin in dev commands.)*

- **`safe` mode (recommended template):**
  ```text
  default-src 'self'; script-src 'self' <scripts...> <'sha256-...'>; style-src 'self' <styles...> <'unsafe-inline' if inlineStyles>; img-src 'self' <images...> <data: if dataImages>; font-src 'self' <fonts...>; media-src 'self' <media...>; connect-src 'self' <connect...>; frame-src <frames... or 'none'>; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self';
  ```
  *(Plus `localExceptions` origins, and `dev.url` + its `ws:`/`wss:` in dev commands. See Decision 1 below regarding `'unsafe-eval'` for Tailwind Play CDN in `safe` mode.)*

---

## 7. HTML and CSS Parser Crates

- **HTML: `lol_html` `3.0.3`** ([docs.rs/lol_html/3.0.3](https://docs.rs/lol_html/3.0.3/lol_html/), [crates.io/crates/lol_html](https://crates.io/crates/lol_html)):
  - Streaming, WHATWG-compliant HTML rewriter by Cloudflare in pure Rust.
  - Preserves untouched HTML formatting/whitespace when rewriting `<script>`, `<link>`, `<img>` for `wavical vendor` and inline script extraction (`dev-ultra-safe` / `build-ultra-safe`).
  - Exposes `source_location()` byte offsets, which map directly to 1-based `(line, column)` for inline event handler (`on*`) diagnostics.
  - Alternative considered: `html5ever` `0.39.0` + `scraper` (builds a full DOM tree, which alters formatting/whitespace upon re-serialization).
- **CSS: `cssparser` `0.38.0`** ([docs.rs/cssparser/0.38.0](https://docs.rs/cssparser/0.38.0/cssparser/), [crates.io/crates/cssparser](https://crates.io/crates/cssparser)):
  - Mozilla/Servo's pure Rust CSS tokenizer/parser.
  - Cleanly parses `@import "..."`, `@import url(...)`, and `url(...)` tokens with byte spans for one-level CSS vendoring and rewriting, without the heavy dependency tree of `lightningcss`.
- **Decision:** Use **`lol_html` `3.0.3`** and **`cssparser` `0.38.0`**.

---

## 8. Recognising Pinned Versions in Common CDN URLs

| CDN | Pinned Pattern | Unpinned Pattern | Pinned Example | Unpinned Example |
|---|---|---|---|---|
| **cdnjs** | `/ajax/libs/<pkg>/<exact-semver>/<file>` | `/latest/`, missing version | `https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.21/lodash.min.js` | `https://cdnjs.cloudflare.com/ajax/libs/lodash.js/latest/lodash.min.js` |
| **jsDelivr** | `/npm/<pkg>@<exact-semver>/<file>` or `/gh/<user>/<repo>@<exact-semver\|40-hex-sha>/<file>` | No `@ver`, `@latest`, `@3`, `@3.x`, `@^3.0.0`, `@~3.0.0`, `@main` | `https://cdn.jsdelivr.net/npm/jquery@3.7.1/dist/jquery.min.js` | `https://cdn.jsdelivr.net/npm/jquery@3/dist/jquery.min.js` |
| **unpkg** | `/<pkg>@<exact-semver>/<file>` | No `@ver`, `@latest`, `@18`, `@^18.0.0`, `@~18.0.0` | `https://unpkg.com/react@18.3.1/umd/react.production.min.js` | `https://unpkg.com/react@18/umd/react.production.min.js` |
| **Google Fonts** | Font binaries on `fonts.gstatic.com/s/<font>/v<N>/...` have `/v<N>/` | `https://fonts.googleapis.com/css2?family=...` (dynamic UA-dependent CSS) | `https://fonts.gstatic.com/s/roboto/v30/KFOmCnqEu92Fr1Mu4mxK.woff2` | `https://fonts.googleapis.com/css2?family=Roboto:wght@400;700&display=swap` |
| **Tailwind Play CDN** | `https://cdn.tailwindcss.com/<exact-semver>` or `https://cdn.tailwindcss.com@<exact-semver>` | `https://cdn.tailwindcss.com`, `@4`, `/3` | `https://cdn.tailwindcss.com/3.4.17` | `https://cdn.tailwindcss.com` |

*Exact-semver* means `v?\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?` (all three `major.minor.patch` numeric components present, no range operators `^`, `~`, `*`, `x`).

---

## 9. Ultra-Safe Leak Channels in WebView2

Note from `wry-0.57.0/src/webview2/mod.rs:300-303`: `wry` sets `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` by default. Calling `WebViewBuilderExtWindows::with_additional_browser_args` **replaces** `wry`'s default string, so Wavical must preserve `msWebOOUI,msPdfOOUI,msSmartScreenProtection` when appending flags.

| Leak Channel | Can WebView2/wry Close Reliably? | Mechanism & Empirical Probe Result (`wavical-wp1-probe`) | How to Test |
|---|---|---|---|
| **WebRTC (ICE/STUN UDP)** | **No** (browser-level) | Tested `--force-webrtc-ip-handling-policy=disable_non_proxied_udp --disable-features=...,WebRtcHideLocalIpsWithMdns`. **Probe result:** `UDP_STUN_HITS=4` in both default and hardened modes. CSP `connect-src 'self'` does not govern WebRTC. | Local `UdpSocket` listener receiving STUN binding requests from `new RTCPeerConnection({ iceServers: [{ urls: 'stun:127.0.0.1:<port>' }] })`. |
| **Preconnect (`<link rel="preconnect">`)** | **Static HTML only; No at runtime** | Tested `--disable-preconnect`. **Probe result:** `TCP_PRECONNECT_HITS=1` (default) and `2` (hardened). Static `<link rel="preconnect">` in HTML can be stripped/reported by `dev-ultra-safe`/`build-ultra-safe`, but JS-created `<link rel="preconnect">` is not blocked by CSP or WebView2 flags. | Local `TcpListener` counting incoming TCP connections from `<link rel="preconnect">`. |
| **DNS Prefetch (`<link rel="dns-prefetch">`)** | **Partially** | `--dns-prefetch-disable` disables Chromium's speculative DNS prefetching; `X-DNS-Prefetch-Control: off` response header + stripping `<link rel="dns-prefetch">` in `ultra-safe` HTML closes static tags. | Inspect response headers for `X-DNS-Prefetch-Control: off` and verify `<link rel="dns-prefetch">` is absent/rejected. |
| **Beacon / `fetch` / `XHR` / WebSocket / `<a ping>` / `<link rel="prefetch">`** | **Yes** | Covered by `DEFAULT_CSP` (`connect-src 'self'`, `default-src 'self'`). | Attack page attempting each against local test server; assert 0 hits. |
| **`<meta http-equiv="refresh">` / `window.open`** | **Yes** | Blocked by `with_navigation_handler` and `with_new_window_req_handler`. | Attack page attempting meta-refresh and `window.open`; assert blocked. |
| **WebView2 background services** | **Partially** | Adding `OptimizationHints,NetworkTimeServiceQuerying` to `--disable-features` reduces background queries, but Evergreen WebView2 runtime updates/telemetry are OS-managed. | Inspect `with_additional_browser_args` string and run local proxy/network monitor. |

### Known Residual Risks (to document in `SECURITY_MODEL.md` and `docs/NETWORK_MODES.md`)
1. **WebRTC (`RTCPeerConnection` ICE/STUN UDP):** Not blocked by CSP `connect-src 'self'` or WebView2 CLI flags (`UDP_STUN_HITS=4` observed in hardened probe).
2. **Runtime-injected `<link rel="preconnect">` / `<link rel="dns-prefetch">`:** Not blocked by CSP or `--disable-preconnect` (`TCP_PRECONNECT_HITS=2` observed in hardened probe).
3. **Evergreen WebView2 runtime background traffic:** Microsoft Edge WebView2 component/telemetry connections occur at the runtime level outside page CSP.

---

## 10. Local Multi-Origin Test Server Approach (No Hosts-File Edits)

- Bind multiple `std::net::TcpListener::bind("127.0.0.1:0")` listeners in Rust integration tests. Each distinct ephemeral port (`http://127.0.0.1:<portA>`, `http://127.0.0.1:<portB>`, `http://127.0.0.1:<portC>`) is a distinct **origin** per RFC 6454 (Same-Origin Policy, CSP, and Wavical's origin matcher).
- Zero hosts-file edits, zero admin privileges, zero internet dependency, and works deterministically on Windows and Linux CI.
- For `https://` origin unit/downloader tests (where `wavical://` / WebView2 is not involved), use `rcgen` + `rustls` on `127.0.0.1:0`; for WebView2 integration tests, use `http://127.0.0.1:<port>` via `localExceptions` (since WebView2 rejects self-signed certs by design and Wavical never disables cert verification).

---

## Decisions I Need From You

1. **Tailwind Play CDN (`'unsafe-eval'`) in `safe` mode:**
   In `wavical.allowlist.json`, `"inlineStyles": true` allows `'unsafe-inline'` in `style-src` for Tailwind Play CDN, but Tailwind Play CDN also requires `'unsafe-eval'` in `script-src` (which Plan 1.4 does not have a field for). How should `safe` mode handle Tailwind Play CDN?
   - **Option A (Recommended):** Keep `safe` mode free of `'unsafe-eval'` (do not add an allowlist field; document that Tailwind Play CDN requires `open` mode or a build-step CSS file in `safe`/`ultra-safe`).
   - **Option B:** Allow `'unsafe-eval'` automatically in `safe` mode whenever `https://cdn.tailwindcss.com` is in `scripts`.

2. **Google Fonts (`fonts.googleapis.com/css2?...`) in `wavical vendor`:**
   Google Fonts CSS2 URLs have no version number in the URL (they serve UA-specific CSS pointing to versioned `fonts.gstatic.com/s/.../v<N>/...` files). How should `wavical vendor` treat `fonts.googleapis.com`?
   - **Option A (Recommended):** Treat `fonts.googleapis.com` as unpinned per Plan 1.11 (requires `--allow-unpinned https://fonts.googleapis.com/...` with the one-time snapshot warning, and vendors the `@import`/`url()` `.woff2` files locally).
   - **Option B:** Whitelist `fonts.googleapis.com/css2` as a special-cased "pinned-equivalent" CSS snapshot without requiring `--allow-unpinned`.

3. **HTML & CSS parser crates (`lol_html` `3.0.3` and `cssparser` `0.38.0`):**
   Adding `lol_html` (`3.0.3`), `cssparser` (`0.38.0`), `url` (`2.5.8`), `psl` (`2.1.245`), and `open` (`5.4.5`) in WP2/WP4/WP7. Do you approve this crate set?
   - **Option A (Recommended):** Approve `lol_html 3.0.3`, `cssparser 0.38.0`, `url 2.5.8`, `psl 2.1.245`, and `open 5.4.5`.
   - **Option B:** Prefer `html5ever 0.39.0` + `scraper` instead of `lol_html`.
