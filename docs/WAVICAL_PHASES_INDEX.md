# Wavical Build Guide — Phase Index

This index helps you navigate the build documentation for Wavical, a lightweight desktop app framework built on Rust, WebView2, and a JavaScript config system.

## Document Map

### Core Setup
- **WAVICAL_SETUP.md** — Environment setup (Rust, C++, Node, WSL/fuzzing)
- **WAVICAL_FEATURE_CHECKLIST.md** — Master feature list across all 6 phases

### Phase 0 — Foundation Shell (COMPLETE ✅)
**Goal:** Minimal security-first shell with IPC bridge and custom protocol handler.

Documents:
- **WAVICAL_PHASE_0_BUILD_PLAN.md** — Technical plan (8 work packages)
- **WAVICAL_ANTIGRAVITY_PROMPTS.md** — AI prompts for WP1–WP8
- **ROADMAP.md** (in repo) — Public roadmap for all 6 phases
- **SECURITY.md** (in repo) — Vulnerability reporting policy

**What was built:**
- Workspace structure (wavical-ipc, wavical-protocol, wavical-shell)
- Default-deny capability model
- Custom `wavical://` protocol with path sanitization
- IPC schema validation
- Content Security Policy enforcement
- Navigation allowlisting
- Security test bench (unit tests, proptests, fuzzing, integration tests)

**Status:** Phase 0 is complete. All work packages (WP1–WP8) passed acceptance. CI is green.

---

### Phase 1 — Core CLI & Config (IN PROGRESS)
**Goal:** Developer-friendly CLI with JavaScript config file.

Documents:
- **PHASE_1_README.md** ← **Start here** for Phase 1 overview
- **WAVICAL_PHASE_1_BUILD_PLAN.md** — Technical plan (5 work packages)
- **WAVICAL_PHASE_1_ANTIGRAVITY_PROMPTS.md** — AI prompts for WP1–WP5
- **WAVICAL_PHASE_1_CONFIG_SCHEMA.md** — Reference for wavical.config.js format

**Work packages:**
1. **WP1:** Config schema & validation (Zod-style, Node subprocess)
2. **WP2:** CLI skeleton & `wavical init` (project scaffolding)
3. **WP3:** `wavical dev` (dev server with hot-reload)
4. **WP4:** `wavical build` (static build, manifest generation)
5. **WP5:** Integration & self-review (E2E testing, security review)

**Key decisions:**
- Config format is JavaScript (executable, familiar to web devs)
- Capability declarations in config (not hardcoded)
- Config is parsed via Node subprocess (CLI shells out, never runs Node code directly)
- Second binary name is `wca` (short alias for `wavical`)
- Error messages are user-friendly (no Rust panics)

**Status:** Phase 1 prompts and build plan are ready. Ready to start WP1.

---

### Phase 2 — Native Tier 1 APIs (NOT STARTED)
**Goal:** Pure Rust native APIs (file dialogs, clipboard, notifications, tray, shortcuts, etc.)

Will include:
- File dialogs (rfd crate)
- Clipboard (arboard)
- Notifications (notify-rust)
- System tray
- Global shortcuts
- Window state persistence
- Single-instance lock
- Webview crash recovery

*Plan: to be generated after Phase 1 is complete.*

---

### Phase 3 — AI-Native Infrastructure (NOT STARTED)
**Goal:** Local and cloud AI model integration (core differentiator).

Will include:
- Local AI model runtime (llama.cpp / GGUF)
- Cloud model fallback
- Semantic search / RAG
- Model manager CLI

*Plan: to be generated after Phase 2 is complete.*

---

### Phase 4 — Backend Runtime & Updates (NOT STARTED)
**Goal:** Optional Node/Bun backend, auto-updater, security hardening.

Will include:
- Shared runtime cache (not bundled per-app)
- Auto-download with hash verification
- Auto-updater with signature verification
- Reduced-privilege backend process
- Devtools locked in production

*Plan: to be generated after Phase 3 is complete.*

---

### Phase 5 — Packaging & Distribution (NOT STARTED)
**Goal:** Installers, code signing, portable builds.

Will include:
- Windows MSI installer
- Portable/zip mode
- Code signing
- Benchmark (Wavical vs. Electron vs. Tauri)

*Plan: to be generated after Phase 4 is complete.*

---

### Phase 6 — Enterprise Tier (NOT STARTED)
**Goal:** Audit logging, dependency scanning, SBOM, licensing primitives.

Will include:
- Enterprise audit logger
- Vulnerability scanner
- SBOM generation
- Licensing/entitlement checks

*Plan: to be generated after Phase 5 is complete.*

---

## How to use this guide

### For the first build (Phase 1)

1. **Read PHASE_1_README.md** for an overview of Phase 1's goals and structure
2. **Paste the Master Brief** from WAVICAL_PHASE_1_ANTIGRAVITY_PROMPTS.md (or save it as project rules)
3. **Start WP1** — follow the WP1 prompt in WAVICAL_PHASE_1_ANTIGRAVITY_PROMPTS.md
4. **One work package per session** — finish WP1, verify it passes acceptance, then start WP2 in a fresh session
5. **After WP5**, Phase 1 is complete; commit and push

### For subsequent phases

Each phase will have:
- A `PHASE_N_README.md` (overview)
- A `WAVICAL_PHASE_N_BUILD_PLAN.md` (technical details)
- A `WAVICAL_PHASE_N_ANTIGRAVITY_PROMPTS.md` (AI-ready prompts)
- Optionally, reference docs for new features (e.g., config schema for Phase 1, native API docs for Phase 2)

**Pattern:** After Phase N is complete, the Phase N+1 plan will be generated based on:
- What was learned in Phase N
- Feedback from the implementation
- Any API changes or new dependencies that appeared

### Questions about scope

**"Is X in Phase Y?"** Check the feature checklist at the top of the relevant phase's build plan, or consult:
- `WAVICAL_FEATURE_CHECKLIST.md` — lists all features by phase
- The "Out of scope" section in the phase's build plan

**"Why are we doing this before that?"** Phases are ordered by dependency:
- Phase 0 is the foundation (window, webview, IPC)
- Phase 1 wraps Phase 0 in a CLI (no Phase 0 changes needed)
- Phase 2 (native APIs) builds on Phases 0–1
- Phase 3 (AI) builds on Phases 0–2
- Phases 4–6 are orthogonal (can be worked on partly in parallel, but all depend on Phases 0–3)

---

## Key artifacts in the repo

After Phase 0 is complete, the repo contains:

```
wavical/
├── Cargo.toml (workspace)
├── crates/
│   ├── wavical-ipc/ (Phase 0: IPC types, capability model)
│   ├── wavical-protocol/ (Phase 0: path sanitization)
│   ├── wavical-shell/ (Phase 0: window + webview)
│   ├── wavical-cli/ (Phase 1: CLI entry point)
│   ├── wavical-config/ (Phase 1: config parsing)
│   └── wavical-* (Phase 2+: native APIs, AI runtime, etc.)
├── examples/hello/ (demo app, enhanced per phase)
├── tests/ (unit, property, integration, fuzz)
├── .github/workflows/ci.yml (grows with each phase)
├── docs/
│   ├── WAVICAL_PHASE_0_BUILD_PLAN.md
│   ├── WAVICAL_PHASE_1_BUILD_PLAN.md
│   ├── WAVICAL_PHASE_1_CONFIG_SCHEMA.md
│   └── SECURITY_MODEL.md (Phase 0, end of WP7)
├── ROADMAP.md (public-facing, updated per phase)
└── SECURITY.md (vulnerability reporting)
```

---

## Timeline estimate

- **Phase 0:** 1–2 weeks (COMPLETE ✅)
- **Phase 1:** 2–3 weeks (starting now)
- **Phase 2:** 2–3 weeks
- **Phase 3:** 3–4 weeks (more complex)
- **Phase 4:** 2–3 weeks
- **Phase 5:** 1–2 weeks (mostly packaging, not new features)
- **Phase 6:** 2–3 weeks (post-traction, optional)

**Total to v1.0 (Phases 0–5):** ~12–18 weeks part-time with AI assistance.

---

## Support and feedback

- **Technical questions:** Refer to the relevant phase's build plan section
- **Feature scope questions:** Check the feature checklist and the phase's "Out of scope" section
- **Implementation concerns:** Raise them in the work package acceptance step; don't work around them silently

Good luck building Wavical! 🚀
