# Changelog

## [1.1.0] - 2026-11-21

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
