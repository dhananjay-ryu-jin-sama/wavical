# Why `wavical dev` breaks `my-app` (and what else will break)

Tested on 2026-11-21, Windows 11, WebView2 runtime 157.0.4341.60, `wavical@1.1.0` from npm (installed in `my-app/node_modules`).

## TL;DR

**The page is fine. The npm package is broken.**

1. **Main root cause: `wavical@1.1.0` on npm ships stale binaries.** `node_modules/wavical/bin/wavical.exe` and `wavical-shell.exe` are byte-identical (same SHA-256) to `target/release/*.exe` built on **Nov 16**. That is *before* Phase 4.5 (network modes). The `prepack` script in `package.json` only **copies** `target/release/*.exe`; it never rebuilds them. So the published 1.1.0 is really the old ~1.0.x shell:
   - `wavical --version` prints `wavical 0.1.0`.
   - `wavical --help` has **no** `dev-safe`, `dev-ultra-safe`, `vendor` or `allowlist` commands, even though the README/CHANGELOG shipped in the same package document them.
   - `wavical-shell.exe --help` has **no** `--network-mode`, `--dev-url` or `--remote-url`.
   - The only CSP embedded in the shell binary is the old strict, local-only one:
     ```
     default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self';
     connect-src 'self'; frame-src 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self';
     ```
   - The CHANGELOG says "`wavical dev` now runs in **open** mode", but what you actually get from npm is the strict, ultra-safe-equivalent CSP. **Every `https://` resource the page uses is blocked.**

2. **Hot reload never works with the npm binaries (even after fixing #1).** `wavical-shell/src/main.rs` hard-codes `listen_stdin = false` in release builds (`#[cfg(not(debug_assertions))]`). The CLI still prints `info: asset changed, reloading... ✓ Reloaded in 0ms`, but the shell ignores stdin, so nothing happens.

3. **DevTools are compiled out of release builds** (`DEVTOOLS_COMPILED_ENABLED = cfg!(debug_assertions)`). With the npm binary you can't press F12 or inspect, and CSP errors print nowhere. That is why this has been so hard to diagnose: it fails silently.

4. **`wavical build` will break the same way, even in the current source.** `run_build` writes a `WavicalManifest` with **no `mode`** field (and no `allowlistSha256` or `inlineScriptHashes`). The shell treats a missing `mode` as **ultra-safe** (`resolve_shell_network_policy_with_remote`: "manifest missing mode field; falling back to ultra-safe"). So a production build of this app gets the same strict CSP and the same broken page.

Proof that the page itself is OK: running the same `my-app` with your **local, unpublished debug build** (`target/debug/wavical.exe dev`, which logs `mode="open"`) gives a result **identical to the browser**. I probed it inside the real WebView2: all libraries, fonts, logos, the intro, scroll effects and the form work (screenshot 4).

---

## How it was tested

| Run | How | Result |
|---|---|---|
| A. Browser | Headless Edge (same Chromium engine as WebView2), `src/` served at `http://wavical.localhost` (the origin Wavical uses), no CSP | ✅ Everything works |
| B. **`npx wavical dev` (npm 1.1.0)** | The real shell window, screenshotted | ❌ Broken (screenshot 2) |
| B'. Wavical 1.1.0 CSP, emulated | Same as A, plus the exact CSP header extracted from the npm shell binary, so I could probe the DOM | ❌ Identical to B; used for the per-feature results below |
| C. Open-mode CSP from current source | Same as A, plus the `generate_open_csp()` header | ✅ Identical to browser |
| D. Local debug build `target/debug/wavical.exe dev` | Real WebView2, probed over CDP | ✅ Identical to browser (only harmless warnings) |
| E. Hot-reload probe | Appended a red "RELOAD PROBE" div to `index.html` while B was running, then restored the file | ❌ CLI logged "Reloaded", window unchanged (screenshot 3) |

Screenshots are in `break-screenshots/`:
`1-browser.png`, `2-wavical-dev-npm-1.1.0.png`, `3-wavical-dev-after-edit-no-reload.png`, `4-wavical-dev-local-source-build.png`, `5-wavical-csp-services-section.png`, `6-browser-services-section.png`.

---

## Exactly what breaks under `wavical dev` (npm 1.1.0)

Five external resources are blocked by CSP. Everything else follows from those five:

| Blocked resource | Directive that blocks it |
|---|---|
| `https://cdn.tailwindcss.com` (script) | `script-src 'self'` |
| `https://cdnjs.cloudflare.com/.../gsap.min.js` | `script-src 'self'` |
| `https://cdnjs.cloudflare.com/.../ScrollTrigger.min.js` | `script-src 'self'` |
| `https://unpkg.com/lenis@1.1.18/dist/lenis.min.js` | `script-src 'self'` |
| `https://fonts.googleapis.com/css2?...` (stylesheet) | `style-src 'self'` (fonts.gstatic.com would also hit `font-src 'self'`) |
| `https://www.designarena.ai/u/...` (logo image + favicon) | `img-src 'self' data:` |

Measured in the browser (A) and under the Wavical 1.1.0 CSP (B'):

| # | Feature | Browser | `wavical dev` | Why |
|---|---|---|---|---|
| 1 | **Tailwind layout/utilities** (nav `md:flex`, `max-w-7xl`, `hidden`, grids, `gap-*`, spacing, colours `text-gold2`, `bg-ink-*`, `backdrop-blur`, `font-display`/`font-mono` classes) | nav `display:flex`, container `1280px` | nav `display:block`, `max-width:none` | Tailwind Play CDN blocked. `script.js` still sets `window.tailwind.config`, but nothing reads it. The nav collapses into an underlined link list (top-left of screenshot 2). |
| 2 | **Hamburger button shows on desktop** | hidden at ≥md | visible | `md:hidden` is a Tailwind class |
| 3 | **"Request Received" success panel always visible** | `display:none` | `display:block` | `.hidden` comes from Tailwind |
| 4 | **Custom fonts** (Syne, Space Grotesk, JetBrains Mono) | loaded | fallback serif/sans | Google Fonts CSS blocked by `style-src`, font files by `font-src` |
| 5 | **Hero title clipped off the right edge** ("…TECHNOLOGI") | fits | overflows | The serif fallback is wider than Syne, and `#hero-title` has `white-space:nowrap` |
| 6 | **Logo images** (preloader, nav, hero, about orbit, footer, favicon) | 5/5 loaded | 0/5. The `img`s are removed by the `data-remove-on-error` handler and the "GN" text fallback shows | `img-src 'self' data:` |
| 7 | **Preloader / lotus intro animation** (petals, swirl, glow, ring burst, flash, skip button) | plays (16 petals built) | never runs (0 petals); preloader is hard-hidden by `hardReveal()` | GSAP missing: `if(!window.gsap) return;` exits the whole interactive engine |
| 8 | **Hero reveal**: per-letter title split, underline grow, kicker/badge/CTA entrance, typewriter tagline | animated | static; tagline set instantly by the fallback | same (GSAP) |
| 9 | **Hero particle constellation canvas + mouse repulsion** | 1200×800 canvas animating | not initialised (default 300×150, blank) | code sits after the GSAP early `return` |
| 10 | **Custom cursor** (dot + trailing ring, grow on hover) | works | dot stuck at top-left (yellow dot in the corner of screenshot 2), no ring | same |
| 11 | **Lenis smooth scrolling** | smooth | native, jumpy scroll | Lenis blocked |
| 12 | **Scroll progress bar** | `scaleX(0.31)` after scrolling | stuck at `scaleX(0)` | same |
| 13 | **Nav "scrolled" glass background** | toggles at 40px | never | same |
| 14 | **Active nav-link highlight** (ScrollTrigger) | follows section | stuck on HOME | ScrollTrigger missing |
| 15 | **Smooth anchor scrolling with -64px offset**, closing the mobile menu on click | works | native jump; target hides under the fixed header | the JS anchor handler is never attached |
| 16 | **Mobile menu open/close** | works | button does nothing | listener never attached |
| 17 | **Back-to-top button** | appears after 600px | never appears | same |
| 18 | **Services "energy line" connector**: SVG path, comet, traveler glow, lit nodes, energized cards | path built, nodes light up | empty path, no comet, no lit nodes | `buildConnector()` is never called |
| 19 | **Reveal-on-scroll** (`[data-reveal]`, cards sliding in with 3D rotate, node pop-in) | animated | static | GSAP/ScrollTrigger |
| 20 | **Parallax** (hero content fade-out, hero orbs, service orbs/shapes, about orb) | works | none | same |
| 21 | **Stats counters** (50+, 30+, 99.9%, 24/7) | count up | **stuck at "0"** | the counter tween never runs. This one is actively *wrong* content, not just less pretty. |
| 22 | **Card 3D tilt + spotlight** | works | none | listener never attached |
| 23 | **"Discuss this service" → prefill quote form + gold pulse** | sets `q-service` = "Cyber Security" | does nothing (value `""`) | listener never attached |
| 24 | **Quote form validation** (inline errors, shake) | 4 invalid fields flagged | no validation | listener never attached |
| 25 | **Quote form submit → spinner → success burst** | success panel + particle burst | **native form submit: the whole page reloads** (URL becomes `index.html?`) and the input is lost | no `preventDefault`. The form has no `action`, so it GET-submits to itself. (`form-action 'self'` allows that.) |
| 26 | **"Send another request" reset** | works | dead | listener never attached |
| 27 | **Marquee seamless loop** | track duplicated (24 items) | 12 items, so a visible gap/jump at the loop point | `mq.innerHTML += mq.innerHTML` never runs |
| 28 | **Resize handling** (canvas resize, connector rebuild, ScrollTrigger refresh) | works | none | listener never attached |
| 29 | **Hot reload** | n/a | CLI says "✓ Reloaded in 0ms", **window does not change** | release shell has `listen_stdin = false` (see TL;DR #2) |
| 30 | **DevTools / F12 / console** | available | unavailable | release build compiles devtools out, so all of the above fails silently |

What still works under the strict CSP: `style.css` (it's local, so the dark background, gradients, noise, cards and orbit rings render), the static HTML text and SVG icons, the "GN" logo fallback, and `script.js` itself loading (it's same-origin). Only its interactive part is dead, because it bails out without GSAP.

### Note: `styles.css` is unused

`src/styles.css` (white background, `padding: 2rem`) is never linked; `index.html` links `style.css`. It's harmless, but if someone "fixes" the link to `styles.css` the page will turn white with padding.

---

## Related features that will also break (or are at risk)

Some of these I verified in the real shell using the local open-mode build (D). The rest come from reading the source.

### With the current npm package (strict CSP), anything external breaks

- **Any CDN** (jsDelivr, unpkg, cdnjs, esm.sh, Skypack), including `<script type="module">` imports from URLs and `import()` from CDNs.
- **Any web fonts** (Google Fonts, Adobe Fonts, Font Awesome kits), **icon fonts**, and remote CSS frameworks (Bootstrap CDN, etc.).
- **Remote images/video/audio** (`img-src 'self' data:`, `media-src` falls back to `default-src 'self'`). `blob:` images (e.g. `URL.createObjectURL` previews, canvas exports) are also blocked.
- **`fetch` / XHR / WebSocket / EventSource to any API** (`connect-src 'self'`). Any real backend for the quote form (Formspree, EmailJS, your own API) would fail.
- **Iframes/embeds** (YouTube, Google Maps, Calendly, reCAPTCHA): `frame-src 'none'`.
- **Inline `<script>` blocks and inline `onclick=` handlers**: `script-src 'self'` has no `'unsafe-inline'`. Your page avoided this because the config lives in `script.js`, but most website templates (Google Analytics snippets, inline Tailwind config) will break.
- **Inline `<style>` blocks and runtime-injected `<style>` tags** (CSS-in-JS: styled-components, emotion; and the Tailwind Play CDN itself): `style-src 'self'`.
- **`eval` / `new Function`** (some template engines, older libraries, Vue's runtime template compiler): no `'unsafe-eval'`.
- **HTML form posts to external services**: `form-action 'self'`.

### Even after shipping correct binaries (open mode), these still differ from a browser

- **`mailto:` and `tel:` links are silently blocked.** Verified: `navigation_blocked … origin=mailto: rule="baseline_blocked_scheme"` and the same for `tel:`. `decide_navigation` / `is_baseline_blocked_scheme` only allow `http|https|ws|wss|wavical`. Your page shows the email and phone as plain text today; turning them into links (very common) will do nothing in Wavical. Other schemes (`sms:`, `whatsapp:`, `skype:`, `ms-*`, `file:`, `data:`/`blob:` top-level navigation, downloads via `data:` URLs) are blocked too.
- **`http://` (non-TLS) resources are blocked** even in open mode. Verified: `http://example.com/x.png` was refused by `img-src 'self' https: data: blob:`. Navigation to `http://` also needs a `localExceptions` entry.
- **`target="_blank"` and `window.open` never open in-app.** `handle_new_window_with_policy` always returns `Deny`. `https://` URLs open in the system browser (rate-limited to 5 per 10s); anything else is silently dropped. Popup-based logins (OAuth "Sign in with Google", payment popups) won't work.
- **Third-party storage is blocked by WebView2 Tracking Prevention.** Verified: warnings for cdnjs and unpkg. Harmless for this page, but embeds that rely on third-party cookies/localStorage (YouTube embeds, reCAPTCHA, Disqus, analytics) can misbehave.
- **External links clicked in-page navigate the whole app window** to that https site (open mode allows https navigation), replacing your UI. `window.wavical` is removed on foreign origins, so there's no way back except via the page's own history. Your social links use `href="#"` today; giving them real URLs will hit this.
- **`frame-ancestors 'none'`**: the app can't be iframed (fine for a desktop app, but it matters if you reuse the same pages on the web).
- **`base-uri 'self'`**: a `<base href="https://…">` tag will be ignored.

### `dev-safe` / `dev-ultra-safe` / `vendor` (once they actually ship)

- **ultra-safe and safe will break this page's Tailwind setup even after `wavical vendor`.** The Tailwind Play CDN generates CSS at runtime and injects a `<style>` element. `style-src 'self'` (ultra-safe) blocks that, and safe mode allows it only with `inlineStyles: true`. For these modes, compile Tailwind to a static CSS file (Tailwind CLI) instead of vendoring the Play CDN.
- **`wavical vendor` will refuse `https://cdn.tailwindcss.com`** because it's unpinned (no version in the URL), unless you pass `--allow-unpinned`. The logo URL `https://www.designarena.ai/u/<uuid>` has no version either, so it will most likely need the same flag. GSAP 3.12.5 and Lenis 1.1.18 are pinned and should vendor fine. The Google Fonts CSS → `fonts.gstatic.com` woff2 chain relies on vendor's "follow `url()` one level" behaviour.
- In **safe** mode the logo image needs `images` plus `dataImages: true`, because `style.css` uses `data:` SVGs for the noise overlay and the select arrow.

### Packaging / dev-loop features affected by the release-build gating

- **Hot reload of HTML/CSS/JS**: broken in any release shell (`listen_stdin = false`).
- **Live config reload** ("✓ Config reloaded. Capability set updated."): also reported as successful but **not applied**. Same stdin channel.
- **Backend auto-restart on `backend/` changes**: same stdin channel, so it doesn't happen.
- **`wavical build` output runs in ultra-safe** because no `mode` is written to `wavical-manifest.json` (TL;DR #4). For safe mode, `allowlistSha256` and `inlineScriptHashes` are never written either, so a safe build would fall back to ultra-safe or fail its hash checks.
- **Version reporting**: npm says 1.1.0, `wavical --version` says 0.1.0 (the Cargo version), which makes bug reports confusing.

---

## How to fix

1. **Republish with fresh binaries.** Make `prepack` build before copying, e.g. in the root `package.json`:
   ```json
   "prepack": "cargo build --release --bin wavical --bin wavical-shell && node -e \"...copy target/release/*.exe to bin/...\""
   ```
   Also add a guard that fails the pack if `bin/wavical.exe --help` doesn't list `dev-safe` (or if the exe is older than the newest `crates/**/*.rs`). Then bump to 1.1.1 and publish. Consider `npm deprecate wavical@1.1.0 "ships pre-1.1 binaries; use 1.1.1"`.
2. **Make `wavical dev` usable with the shipped release shell.** Either allow the stdin command channel when the shell is launched with `--dev` (it's a local pipe owned by the CLI process), or ship a separate dev-shell binary. At minimum, stop printing "✓ Reloaded" when the shell can't reload.
3. **Allow DevTools for `--dev` runs** (or at least forward CSP violations / console errors to the CLI's stderr via the existing `securitypolicyviolation` hook) so failures aren't silent.
4. **Write `mode` (and, for safe, `allowlistSha256` + `inlineScriptHashes`) into `wavical-manifest.json`** in `run_build`, with a `--mode` flag that defaults to `open` to match `wavical dev`.
5. Consider routing `mailto:`/`tel:` (and optionally other user-approved schemes) through `open_external_validated`, opening them in the OS handler instead of silently dropping them. Also consider sending cross-origin top-level link clicks to the system browser instead of navigating the app window away.
6. Optional, on the app side: replace the Tailwind Play CDN with a compiled `tailwind.css` (the Play CDN itself warns "should not be used in production"). Make the email and phone plain `https://` links or keep them as text until (5) lands.

**Workaround today:** use a CLI *and* shell built from the current source together, e.g. run `..\..\target\debug\wavical.exe dev` from `my-app`. That's what run D used, and it also gives you hot reload and DevTools because it's a debug build. Swapping only the shell (`WAVICAL_SHELL_BIN`) under the old npm CLI does **not** help: the old CLI never passes `--network-mode`, and the new shell defaults to **ultra-safe** when that flag is missing (`dev_mode_override … unwrap_or(NetworkMode::UltraSafe)`). A `cargo install --path crates/wavical-cli` release build fixes the CSP, but hot reload still won't work (#2).
