# Wavical — Full Feature Checklist

Every feature discussed, categorized by build phase. Use this as the master checklist before locking the build plan. Check items off as each phase is implemented and tested.

---

## Phase 0 — Foundation Shell (launch-blocking, get this right before anything else)

- [x] Rust window creation via `tao` + `wry` using tao's own event loop style (do NOT mix in winit's `ApplicationHandler` API; check current docs.rs examples first)
- [x] Basic webview loading via `wry` (latest stable, don't hardcode a version)
- [x] IPC bridge (JSON-RPC style) with:
  - [x] Default-deny capability model (no API callable unless explicitly granted)
  - [x] Frozen, minimal bridge object exposed to page JS (no raw Rust/Node handles leaked)
  - [x] Schema validation on every IPC message (reject malformed/malicious payloads)
  - [x] Path sanitization in the custom protocol handler (block traversal attacks)
  - [x] Default CSP injected on every loaded page (overridable per-window)
  - [x] Navigation allowlisting (block arbitrary external origin navigation)
  - [x] Secrets (API keys etc.) never reachable from webview JS context — enforced, not just documented
- [x] One safe Tier 1 API wired end-to-end, to prove the full pattern works before building more on top of it
- [x] Security test bench v1 running in CI: IPC fuzzing + path traversal test suite (block release on failure)

## Phase 1 — Core CLI & Config

- [ ] `wavical init`, `wavical dev`, `wavical build` commands
- [ ] Short alias `wca` installed alongside `wavical` (same binary behavior, e.g. `wca dev`). Full name stays canonical in docs; `wca` is introduced once as shorthand
- [ ] Command-name collision check for `wavical` and `wca` on Windows and WSL/Linux before release (`where` / `which`)
- [ ] Single flat `wavical.config.js` config file (vs. Tauri/Electron's multi-file split)
- [ ] Config schema validation (zod or equivalent)
- [ ] Capability declarations live in config (ties into Phase 0's capability model)
- [ ] Dev-friendly error messages (no raw Rust panic traces surfaced to the dev)
- [ ] JSON-schema-backed IntelliSense/autocomplete for `wavical.config.js` in VS Code

## Phase 2 — Native Tier 1 APIs (pure Rust, zero Node required)

- [ ] File dialogs (`rfd`)
- [ ] Clipboard (`arboard`)
- [ ] Notifications (`notify-rust`)
- [ ] System tray — confirm whether `tao` still ships this or whether `tray-icon`/`muda` are needed; add explicitly if so
- [ ] Global shortcuts (`global-hotkey` if split out of `tao`)
- [ ] Native context menu builder (`wavical.menu.showContextMenu`)
- [ ] Window state persistence (position/size/maximized, opt-out-able)
- [ ] Single-instance lock (second launch focuses existing window)
- [ ] File association registration (declare `.foo` file handling in config)
- [ ] Taskbar overlay badges (Windows)
- [ ] Webview crash recovery (auto-reload if WebView2 renderer process dies)
- [ ] WebView2 detection + fallback UX (not installed / blocked by IT policy)

## Phase 3 — AI-Native Infrastructure (core differentiator)

- [ ] Local AI model runtime (llama.cpp / GGUF), opt-in via config
- [ ] Cloud AI fallback (API key config switch)
- [ ] Local RAG / semantic search (`ai.index(...)`, `ai.search(...)`)
- [ ] Model manager CLI (`wavical model list`, `wavical model pull <name>`)
- [ ] License acknowledgment prompt before any model download (name + link, explicit confirm)
- [ ] Models fetched directly from upstream source (Hugging Face / official source) — never bundled/mirrored by Wavical itself
- [ ] AI-assisted scaffolding (`wavical init --ai`) — growth/adoption tool, not a runtime feature
- [ ] Hardware benchmark on low-end (8GB RAM, no GPU) machine completed and documented before calling this phase done

## Phase 4 — Backend Runtime (optional) & Updates

- [ ] Node/Bun backend declared as opt-in in config (never required for apps that don't need it)
- [ ] Shared, version-pinned runtime cache per machine (`%LOCALAPPDATA%\Wavical\runtimes\`) — not bundled per-app
- [ ] Runtime auto-download with hash verification against published checksums
- [ ] Exact version pinning in config; auto-update limited to patch-level only, never silent major bumps
- [ ] `--bundle-runtime` build flag for offline/air-gapped/enterprise installs
- [ ] Auto-updater for shipped apps, with signature verification of the update payload (not just HTTPS transport trust)
- [ ] Canary/staged rollout pipeline for Wavical's own framework releases
- [ ] Reduced-privilege execution for the backend process where possible
- [ ] Remote debugging port disabled by default in production builds
- [ ] Devtools toggle enforced off in production (verified, not just configured)
- [ ] Security test bench expanded: update-tampering tests + runtime-download-tampering tests

## Phase 5 — Packaging & Distribution

- [ ] Installer pipeline (Windows first)
- [ ] Code signing on shipped `.exe`/`.msi`
- [ ] Portable/zip mode (`wavical build --portable`, no installer)
- [ ] Real benchmark published: reference app built identically in Wavical, Electron, and Tauri — installer size, on-disk size, idle RAM, cold-start time, time-to-interactive, with benchmark app source published alongside the numbers

## Phase 6 — Enterprise Tier (paid, post-traction)

- [ ] Enterprise audit logger — records capability grants + native API calls, exportable for IT/security review
- [ ] `wavical audit` — scans a dev's app dependencies for known vulnerabilities
- [ ] SBOM generation for Wavical's own releases
- [ ] Hosted/managed update server (so companies don't have to run their own)
- [ ] Private plugin registry
- [ ] Licensing/entitlement-check primitive for devs building paid apps on Wavical

## Ongoing / Ecosystem (not phase-locked, grows over time)

- [ ] Live IPC inspector panel (devtools-style overlay showing every `wavical.*` call in real time) in `wavical dev`
- [ ] Template marketplace (`wavical init --template react`, community-contributed templates)
- [ ] WebDriver/e2e testing story
- [ ] Structured opt-in crash/telemetry reporting (for you, the maintainer — roadmap prioritization)
- [ ] Accessibility baseline (screen reader support via WebView2's UIA bridge, focus order for frameless windows)
- [ ] Public roadmap maintained in the git repo
- [ ] Linux support (webkit2gtk via `wry`, testable in VM) — target soon after Windows v1 stabilizes
- [ ] macOS support — explicitly deferred until real Apple hardware or a contributor is available; GitHub Actions hosted macOS runners used for compile-only CI in the meantime

---

## Business / Process Decisions (not features, but locked in)

- [ ] Open-core model: free core framework (MIT/Apache) maximizes adoption; enterprise features (Phase 6) are the paid tier
- [ ] Consider splitting enterprise-tier code into a separate source-available repo (e.g. BSL) before building it, to keep it defensible against forking
- [ ] AI model licensing: Wavical is a platform, not an enforcer — developers/companies using a given model bear responsibility for that model's license, stated plainly in Wavical's ToS/docs
- [ ] Security-critical modules (IPC bridge, capability enforcement, protocol handler, update verification) get expert Rust review before launch, given AI-assisted development is the primary build method
- [ ] Test-bench-before-release + staged rollout are both required, layered together — not a substitute for each other
- [ ] Every real-world issue found post-release becomes a permanent regression test in the bench

---

## Changed From the Original Build Guide

- [ ] Renamed: "Nova" → ARKA → **Wavical**, with CLI alias **`wca`** (considered and dropped: `wav`, `wv`, `wcl`)
- [ ] Rejected: bundling Node into every installer (too heavy, breaks the lightweight pitch) — replaced with shared, auto-downloaded, version-pinned runtime cache
- [ ] Softened: "no Rust required" as the headline differentiator (Tauri 2's official plugins already cover this for most apps) — repositioned AI-native infrastructure as the primary pitch instead
- [ ] Clarified: the original guide's closure-style event loop is fine with `tao` (wry's recommended crate); only winit moved to `ApplicationHandler`. Pick one windowing crate and don't mix their APIs
- [ ] Flagged for verification: assumption that `tao` ships built-in system tray support — may need separate `tray-icon`/`muda`/`global-hotkey` crates
- [ ] Moved up: security/capability model, originally filed as a "Phase 6 extra," now required in Phase 0
