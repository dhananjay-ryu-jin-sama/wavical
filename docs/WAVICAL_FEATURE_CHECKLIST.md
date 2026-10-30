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

- [x] `wavical init`, `wavical dev`, `wavical build` commands (implemented in `crates/wavical-cli`, full lifecycle verified in e2e tests)
- [x] Short alias `wca` installed alongside `wavical` (same binary behavior, e.g. `wca dev`). Dual binary targets in Cargo.toml routing to identical CLI handler
- [x] Command-name collision check for `wavical` and `wca` on Windows and WSL/Linux before release (`where.exe` / `which` confirmed clean: 0 collisions on system PATH)
- [x] Single flat `wavical.config.js` config file (vs. Tauri/Electron's multi-file split)
- [x] Config schema validation (Node-evaluated subprocess with strict Rust deserialization, timeout kill switch, and typo suggestions)
- [x] Capability declarations live in config (ties into Phase 0's capability model; dynamic reload via stdin protocol without restart)
- [x] Dev-friendly error messages (no raw Rust panic traces surfaced to the dev; `thiserror`-backed structured error reporting)
- [x] JSON-schema-backed IntelliSense/autocomplete for `wavical.config.js` in VS Code (JSDoc `@type {import('wavical-config').AppConfig}` type annotation scaffolded by `wavical init`; schema specification in `docs/phase 1/WAVICAL_PHASE_1_CONFIG_SCHEMA.md`)

## Phase 2 — Native Tier 1 APIs (pure Rust, zero Node required)

- [x] File dialogs (`rfd`) — `dialog.openFile` and `dialog.saveFile` returning 128-bit opaque `GrantTable` tokens (`ReadOnly` / `WriteOnly` scoped) with use-time path re-validation (`fs.readGranted`, `fs.writeGranted`)
- [x] Clipboard (`arboard`) — `clipboard.read` and `clipboard.write` with 1 MB payload validation
- [x] Notifications (`notify-rust`) — `notify.send` with title/body length bounds and per-window sliding-window rate limiting (max 5 per 10s)
- [x] System tray — confirmed `tao` requires external crates; implemented via `tray-icon` and `muda` (`tray.set` with icon size cap, tooltip, and menu items)
- [x] Global shortcuts (`global-hotkey` if split out of `tao`) — implemented via `global-hotkey` (`shortcut.register`, max 32 shortcuts, OS-reserved key combination blocklist)
- [x] Native context menu builder (`wavical.menu.showContextMenu`) — implemented via `muda` (`menu.showContextMenu`, max 200 items, single audited Win32 popup hook)
- [x] Window state persistence (position/size/maximized, opt-out-able) — debounced JSON persistence with off-screen monitor fallback, 64 KB ingestion cap, and `window.persistState` config toggle
- [x] Single-instance lock (second launch focuses existing window) — implemented via `interprocess` Windows Named Pipes (`singleInstance` config toggle, forwards args without auto-granting file access)
- [x] File association registration (declare `.foo` file handling in config) — `wavical register-filetypes` and `wavical unregister-filetypes` under `HKCU\Software\Classes\.{ext}\OpenWithProgids`
- [ ] Taskbar overlay badges (Windows) — pure parameter validation (`badge.set`, `0..=99` and `99+`) implemented in `wavical-ipc`; OS-level Win32 `ITaskbarList3` COM rendering deferred as a documented open exception (`F-A0-10`) to avoid unapproved `unsafe` COM code
- [x] Webview crash recovery (auto-reload if WebView2 renderer process dies) — single-reload within 30s rate limit before surfacing native recovery dialog
- [x] WebView2 detection + fallback UX (not installed / blocked by IT policy) — runtime version probe at startup with actionable native error dialog and `winget` installation instructions

## Phase 3 — AI-Native Infrastructure (core differentiator)

- [x] Local AI model runtime (llama.cpp / GGUF), opt-in via config — `crates/wavical-ai` (`ai.enabled: true` gate, SHA-256 engine binary check before every launch, anonymous stdio pipe IPC with zero TCP ports, Windows Job Object lifecycle, idle auto-shutdown, streaming `ai.chat` & `ai.cancel`)
- [x] Cloud AI fallback (API key config switch) — `local`, `cloud`, and `local-first` modes, HTTPS-only host pinning, zero-redirect policy, interactive native user consent dialog (`rfd`), `apiKeyEnv` + Windows Credential Store (`wavical secret set/clear`), and full secret redaction
- [x] Local RAG / semantic search (`ai.index(...)`, `ai.search(...)`) — deterministic text chunker, local embedding pipeline (`bge-small-en-v1.5`), per-app tenant-isolated SQLite vector store, cosine similarity search, and collection/document/metadata boundary guards
- [x] Model manager CLI (`wavical model list`, `wavical model pull <name>`, `wavical model remove <name>`, `wavical model verify <name>`, plus `wavical ai doctor` and `wavical ai bench`)
- [x] License acknowledgment prompt before any model download (name + link, explicit confirm) — displays model name, license ID, and URL; requires typing `AGREE` interactively or `--accept-license <ID>` before any download traffic
- [x] Models fetched directly from upstream source (Hugging Face / official source) — never bundled/mirrored by Wavical itself (pinned catalog URLs in `models.json`, domain allowlist, `.part` atomic download, GGUF magic header check, and SHA-256 verification)
- [x] AI-assisted scaffolding (`wavical init --ai`) — growth/adoption tool, not a runtime feature (validates generated paths/extensions/sizes, scans for `wavical.invoke` calls, defaults to zero granted capabilities, and synthesizes `wavical.config.js` in Rust)
- [ ] Hardware benchmark on low-end (8GB RAM, no GPU) machine completed and documented before calling this phase done — `wavical ai bench` harness and `docs/AI_BENCHMARK.md` specification completed; awaiting physical 8 GB RAM target hardware run to populate final measured tables (`F-A7-05`)

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
- [x] Devtools toggle enforced off in production (verified, not just configured — `is_devtools_enabled()` gated on `cfg!(debug_assertions)` and verified in release audit tests)
- [ ] Security test bench expanded: update-tampering tests + runtime-download-tampering tests

## Phase 5 — Packaging & Distribution

- [ ] Installer pipeline (Windows first)
- [ ] Code signing on shipped `.exe`/`.msi`
- [ ] Portable/zip mode (`wavical build --portable`, no installer)
- [ ] Real benchmark published: reference app built identically in Wavical, Electron, and Tauri — installer size, on-disk size, idle RAM, cold-start time, time-to-interactive, with benchmark app source published alongside the numbers

## Ongoing / Ecosystem (not phase-locked, grows over time)

- [ ] Live IPC inspector panel (devtools-style overlay showing every `wavical.*` call in real time) in `wavical dev`
- [ ] Template marketplace (`wavical init --template react`, community-contributed templates)
- [ ] WebDriver/e2e testing story
- [ ] Structured opt-in crash/telemetry reporting (for you, the maintainer — roadmap prioritization)
- [ ] Accessibility baseline (screen reader support via WebView2's UIA bridge, focus order for frameless windows)
- [x] Public roadmap maintained in the git repo (`ROADMAP.md`)
- [ ] Linux support (webkit2gtk via `wry`, testable in VM) — target soon after Windows v1 stabilizes
- [ ] macOS support — explicitly deferred until real Apple hardware or a contributor is available; GitHub Actions hosted macOS runners used for compile-only CI in the meantime

---

## Business / Process Decisions (not features, but locked in)

- [ ] Open-core model: free core framework (MIT/Apache) maximizes adoption; enterprise features (Phase 6) are the paid tier
- [ ] Consider splitting enterprise-tier code into a separate source-available repo (e.g. BSL) before building it, to keep it defensible against forking
- [x] AI model licensing: Wavical is a platform, not an enforcer — developers/companies using a given model bear responsibility for that model's license, stated plainly in Wavical's ToS/docs (`README.md`, `docs/AI_PRIVACY.md`, and `wavical model pull` prompt)
- [ ] Security-critical modules (IPC bridge, capability enforcement, protocol handler, update verification) get expert Rust review before launch, given AI-assisted development is the primary build method
- [ ] Test-bench-before-release + staged rollout are both required, layered together — not a substitute for each other
- [x] Every real-world issue found post-release becomes a permanent regression test in the bench (`tests/audit` regression battery covering A0–A7 findings)

---

## Changed From the Original Build Guide

- [x] Renamed: "Nova" → ARKA → **Wavical**, with CLI alias **`wca`** (considered and dropped: `wav`, `wv`, `wcl`)
- [x] Rejected: bundling Node into every installer (too heavy, breaks the lightweight pitch) — replaced with shared, auto-downloaded, version-pinned runtime cache
- [x] Softened: "no Rust required" as the headline differentiator (Tauri 2's official plugins already cover this for most apps) — repositioned AI-native infrastructure as the primary pitch instead
- [x] Clarified: the original guide's closure-style event loop is fine with `tao` (wry's recommended crate); only winit moved to `ApplicationHandler`. Pick one windowing crate and don't mix their APIs
- [x] Flagged for verification: assumption that `tao` ships built-in system tray support — confirmed separate `tray-icon`/`muda`/`global-hotkey` crates are required and integrated in `wavical-native`
- [x] Moved up: security/capability model, originally filed as a "Phase 6 extra," now required in Phase 0
