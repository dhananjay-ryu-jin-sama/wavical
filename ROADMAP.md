# Wavical Roadmap

Wavical is a lightweight, security-first, AI-native desktop application framework built with Rust, `tao`, and `wry`. This document summarizes the phased delivery plan from foundation to enterprise tier.

---

## Phase 0 — Foundation Shell *(Completed)*
- **Window & Webview**: Native Windows window creation via `tao` (closure event loop) and `wry` (WebView2) with per-app profile isolation.
- **Custom Protocol**: `wavical://` asset handler with strict path traversal sanitization and MIME detection.
- **Security-First IPC**: JSON-RPC IPC bridge with default-deny capability enforcement and frozen bridge object.
- **Security Policies**: Strict Content Security Policy (`default-src 'self'`), navigation allowlisting, and popup blocking.
- **Security Test Bench v1**: Unit tests, property tests (`proptest`), cargo-fuzz harness, and adversarial attack page.

## Phase 1 — Core CLI & Configuration *(Completed)*
- **CLI Tooling**: `wavical` executable with short alias `wca` (`init`, `dev`, `build`).
- **Configuration**: Single flat `wavical.config.js` file with schema validation and IDE autocomplete.
- **Capability Declarations**: Declarative security capability manifest in config; live dynamic updates over stdin in debug builds.
- **Asset Bundling**: Recursive copy to `dist/` with sensitive file exclusions and NIST FIPS 180-4 SHA-256 asset hash manifest generation.
- **Developer Experience**: Actionable error reporting without raw panic dumps (`thiserror` enums).
- **CI / E2E Automation**: End-to-end integration test suite and CI workflow running full CLI lifecycle.

## Phase 2 — Native Tier 1 APIs (Pure Rust) *(Completed)*
- **Dialogs & Scoped File Access**: File dialogs (`rfd`) returning mode-scoped (`ReadOnly` / `WriteOnly`) 128-bit grant tokens (`fs.readGranted`, `fs.writeGranted`) and clipboard integration (`arboard`).
- **System Integration**: Rate-limited notifications (`notify-rust`), system tray (`tray-icon`), context menus (`muda`), and global shortcuts (`global-hotkey`).
- **Shell UX & Lifecycle**: Window state persistence, single-instance lock (`interprocess`), Windows `HKCU` file association registration (`register-filetypes` / `unregister-filetypes`), WebView2 availability check, and rate-limited crash recovery.

## Phase 3 — AI-Native Infrastructure (Core Differentiator) *(Core Complete — 8 GB Hardware Benchmark Run Pending)*
- **Local AI Runtimes**: Opt-in GGUF inference (`wavical-ai`) over anonymous stdio pipes with SHA-256 binary integrity checks, Windows Job Object containment, idle shutdown, and zero network connections in local mode.
- **Cloud Fallback & Credentials**: OpenAI-compatible HTTPS client (`local`, `cloud`, `local-first` modes) with zero-redirect pinning, interactive native user consent (`rfd`), and Windows Credential Store secret management (`wavical secret set` / `clear`).
- **Local RAG**: Embedded chunker, local embeddings (`bge-small-en-v1.5`), and per-app tenant-isolated SQLite semantic vector search (`ai.index`, `ai.search`).
- **Model Management & Diagnostics**: CLI commands (`wavical model list`, `pull`, `remove`, `verify`, `wavical ai doctor`, `wavical ai bench`) with mandatory upstream license acknowledgment prompts.
- **AI Scaffolding**: `wavical init <dir> --ai "<prompt>"` project generator with untrusted output validation and default-deny capability synthesis.

## Phase 4 — Optional Backend Runtime & Updates *(Next Up)*
- **Opt-in Shared Runtime**: Centralized version-pinned Node/Bun runtime cache (`%LOCALAPPDATA%\Wavical\runtimes\`).
- **Integrity Verification**: Checksum verification on all runtime downloads.
- **Secure Auto-Updates**: Cryptographically signed delta updates with payload signature verification.

## Phase 5 — Packaging & Distribution *(Planned)*
- **Installers**: Windows installer generation (`.exe` / `.msi`) and code signing pipeline.
- **Portable Distribution**: Standalone portable archive builds (`wavical build --portable`).
- **Benchmarks**: Publicly reproducible benchmarks vs Electron and Tauri (size, idle RAM, cold start).

## Phase 6 — Enterprise Tier *(Planned)*
- **Compliance & Auditing**: Structured enterprise audit logging, SBOM generation, `wavical audit`, and private model/runtime registries.
