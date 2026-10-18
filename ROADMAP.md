# Wavical Roadmap

Wavical is a lightweight, security-first desktop application framework built with Rust, `tao`, and `wry`. This document summarizes the phased delivery plan from foundation to enterprise tier.

---

## Phase 0 — Foundation Shell *(Completed)*
- **Window & Webview**: Native Windows window creation via `tao` (closure event loop) and `wry` (WebView2).
- **Custom Protocol**: `wavical://` asset handler with strict path traversal sanitization and MIME detection.
- **Security-First IPC**: JSON-RPC IPC bridge with default-deny capability enforcement and frozen bridge object.
- **Security Policies**: Strict Content Security Policy (`default-src 'self'`), navigation allowlisting, and popup blocking.
- **Security Test Bench v1**: Unit tests, property tests (`proptest`), cargo-fuzz harness, and adversarial attack page.

## Phase 1 — Core CLI & Configuration *(Completed)*
- **CLI Tooling**: `wavical` executable with short alias `wca` (`init`, `dev`, `build`).
- **Configuration**: Single flat `wavical.config.js` file with schema validation and IDE autocomplete.
- **Capability Declarations**: Declarative security capability manifest in config; live dynamic updates over stdin.
- **Asset Bundling**: Recursive copy to `dist/` with NIST FIPS 180-4 SHA-256 asset hash manifest generation.
- **Developer Experience**: Actionable error reporting without raw panic dumps (`thiserror` enums).
- **CI / E2E Automation**: End-to-end integration test suite and CI workflow running full CLI lifecycle.

## Phase 2 — Native Tier 1 APIs (Pure Rust) *(Next Up)*
- **Dialogs & Clipboard**: File dialogs (`rfd`) and clipboard integration (`arboard`).
- **System Integration**: Notifications (`notify-rust`), system tray, and global shortcuts.
- **Shell UX**: Window state persistence, single-instance lock, native context menus, and crash recovery.

## Phase 3 — AI-Native Infrastructure (Core Differentiator)
- **Local AI Runtimes**: Embedded llama.cpp / GGUF execution with cloud fallback switches.
- **Local RAG**: Embedded semantic search and vector indexing APIs (`ai.index`, `ai.search`).
- **Model Management**: CLI commands (`wavical model pull <name>`) with license acknowledgment prompts.
- **AI Scaffolding**: `wavical init --ai` project generator.

## Phase 4 — Optional Backend Runtime & Updates
- **Opt-in Shared Runtime**: Centralized version-pinned Node/Bun runtime cache (`%LOCALAPPDATA%\Wavical\runtimes\`).
- **Integrity Verification**: Checksum verification on all runtime downloads.
- **Secure Auto-Updates**: Cryptographically signed delta updates with payload signature verification.

## Phase 5 — Packaging & Distribution
- **Installers**: Windows installer generation (.exe / .msi) and code signing pipeline.
- **Portable Distribution**: Standalone portable archive builds (`wavical build --portable`).
- **Benchmarks**: Publicly reproducible benchmarks vs Electron and Tauri (size, idle RAM, cold start).

## Phase 6 — Enterprise Tier
- **Audit Logging**: Structured capability and native API call auditing for enterprise compliance.
- **Security Audits & SBOM**: Dependency vulnerability scanning (`wavical audit`) and SBOM generation.
- **Enterprise Services**: Managed update server and private plugin registry.
