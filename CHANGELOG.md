# Changelog

## [1.2.0] - 2026-11-25

### Fixed
- **Stale binaries in `1.1.0`:** Replaced the copy-only `prepack` step with `scripts/prepack.js` and `scripts/verify-package.js`, which require a clean git working tree (unless `WAVICAL_ALLOW_DIRTY=1` is set for local dry-run `npm pack`), build fresh release binaries, verify CLI commands/flags, shell feature isolation, and docs parity, write `bin/checksums.txt`, and reject binaries byte-identical to previously published npm versions.
- **Version mismatch (`wavical --version` printed `0.1.0`):** Synchronized `Cargo.toml` (`[workspace.package].version`) and `package.json` (`1.2.0`) via `scripts/sync-version.js`, and embedded `<version> (<git-commit>)` at build time across `wavical.exe`, `wavical-shell.exe`, and `wavical-shell-dev.exe`.
- **Hot reload, config reload, and backend restart in npm builds:** Moved dev-loop automation into `wavical-shell-dev.exe` with an authenticated `stdin` command channel (`WAVICAL_DEV_SESSION_TOKEN`) and real shell acknowledgements before the CLI logs `Reloaded`, `Config reloaded`, or `Backend restarted`, while keeping production `wavical-shell.exe` hardened with `stdin` commands compiled out.
- **Silent blocking:** `wavical dev`, `wavical dev-safe`, and `wavical dev-ultra-safe` now forward deduplicated, rate-limited blocked resource/CSP/navigation/popup events (`[blocked]`) with fix hints to CLI `stderr`; built apps running `wavical-shell.exe` log startup and security events to `%LOCALAPPDATA%\Wavical\apps\<appId>\logs\shell.log`.
- **Missing build manifest `mode` and integrity fields:** Bare `wavical build` now refuses without `--allow-open`, alongside `wavical build-safe` and `wavical build-ultra-safe`. All three write `mode` (plus `allowlistSha256`, per-page `inlineScriptHashes`, `externalLinks`, and copied `wavical.allowlist.json` in `safe` mode) into `wavical-manifest.json`, and `wavical-shell.exe` verifies `allowlistSha256` before creating any window or webview in `safe` mode.
- **`mailto:` and `tel:` links:** Routed `mailto:` and `tel:` links through the validated OS opener (allowed by default in `open` mode, gated by `openExternalSchemes` in `safe` mode, blocked in `ultra-safe`) with length limits, control-character/multi-encoding rejection, and scheme-only logging.
- **External links leaving the app:** Added `window.externalLinks` (default `"browser"`, or `"in-app"`), opening user-initiated cross-origin `<a href>` clicks in the system browser while keeping the app on its page (while preserving redirect-based login flows and `navigate` allowlist destinations in-app).

### Added
- **Separate dev shell (`wavical-shell-dev.exe`):** Built in isolation (`target/dev-shell` with the `dev-shell` feature) for dev-only DevTools, hot reload, live config reload, backend auto-restart, `--record`, and terminal diagnostic forwarding—never included in `dist/` builds and never weakening production `wavical-shell.exe`.
- **Terminal diagnostics:** Real-time blocked resource, CSP, navigation, and popup diagnostics with actionable `wavical.allowlist.json` / `wavical vendor` hints printed to CLI `stderr` during dev commands.
- **Version and built-app provenance (`wavical doctor`):** Added `wavical doctor --version-info` (prints CLI and shell versions with git commits and verifies they match) and `wavical doctor --app <PATH>` (inspects a built app's manifest mode, CSP, log path, and shell provenance).

## [1.1.0] - 2026-11-21

> [!WARNING]
> **Known issue:** The `wavical@1.1.0` package published to npm mistakenly shipped stale pre-1.1 binaries (`0.1.0` CLI output and the strict local-only shell without Phase 4.5 network modes, hot reload, or build manifest `mode` support). Upgrade to `1.2.0` (recommended); `wavical@1.1.0` is superseded.

### Added — Phase 4.5: Network Modes (`open` / `safe` / `ultra-safe`)
- **Three network modes:**
  - `wavical dev` now runs in **open** mode (allows `https:` external content, inline scripts/styles, and `https:` external links with baseline protections kept).
  - `wavical dev-safe` enforces `wavical.allowlist.json` per resource type (`scripts`, `styles`, `fonts`, `images`, `media`, `connect`, `frames`, `navigate`, `openExternal`, `inlineStyles`, `dataImages`, `remoteUi`, `localExceptions`) and hashes static inline `<script>` blocks automatically.
  - `wavical dev-ultra-safe` keeps the strict Phase 0 local-only CSP unchanged, automatically extracts inline `<script>` blocks into `src/.wavical-extracted/` (preserving order and `type="module"`), reports inline event handlers with file and line, sets `X-DNS-Prefetch-Control: off`, and disables WebView2 `OptimizationHints` and `NetworkTimeServiceQuerying`.
- **`wavical vendor`:** Scans HTML and CSS in `src/`, enforces pinned CDN versions (with `--allow-unpinned <url>` for one-time snapshots), downloads through the Phase 4 verified downloader into `src/vendor/`, follows CSS `url()` and `@import` one level, writes `vendor.lock.json`, and supports `--check` and `--update`.
- **`wavical allowlist suggest` & `--record`:** Records blocked events (`{ type, origin, count }` only; never full URLs or queries) in `.wavical/recorded-blocks.json` and proposes safe `https:` additions while listing `inlineStyles`, `dataImages`, wildcards, and `localExceptions` separately for explicit user decision.
- **Remote UI (`window.remoteUrl`):** Loads a remote UI URL in `open` or `safe` mode with top-frame-only bridge exposure, `remoteUi.bridge` capability subsetting in `safe` mode, and fail-closed TLS behavior (no certificate-error bypass).
- **Serve-time integrity & mode lock:** Built apps read `mode` from `wavical-manifest.json` (falling back to `ultra-safe` if missing or invalid) and verify every served asset's SHA-256 against the manifest before serving.
- **Pure policy crate (`wavical-netpolicy`):** `#![forbid(unsafe_code)]` crate for allowlist parsing, parsed-URL origin matching, CSP generation, URL version detection, and `vendor.lock.json` validation.

### Breaking Changes
- `wavical dev` now runs in **open** mode instead of strict local-only mode. Use `wavical dev-ultra-safe` for the previous strict behavior, or `wavical dev-safe` for an explicit allowlist.
