# Wavical Vendor (`wavical vendor`)

`wavical vendor` downloads external CDN scripts, styles, fonts, and images into `src/vendor/` so your app works **offline** and in **`ultra-safe`** mode.

## Usage
- `wavical vendor` — Scans HTML and CSS in `src/`, verifies every external URL is pinned to an exact version, downloads through the Phase 4 verified downloader (HTTPS-only, 20 MB per-file / 200 MB total / 500 file limits), follows CSS `url()` and `@import` one level, rewrites HTML and CSS references to local `vendor/...` paths (saving `.bak` backups), and writes `vendor.lock.json`.
- `wavical vendor --allow-unpinned <url>` — Takes a one-time snapshot of an unpinned URL (such as `https://fonts.googleapis.com/css2?...` or `https://cdn.tailwindcss.com`) with a warning that it will never update automatically.
- `wavical vendor --check` — Verifies every vendored file on disk against its exact size and SHA-256 in `vendor.lock.json`.
- `wavical vendor --update` — Refetches pinned URLs, displays changed hashes, and asks for confirmation before writing.

## Version Pinning
- **Pinned (accepted):** `cdnjs.cloudflare.com/ajax/libs/<pkg>/<x.y.z>/...`, `cdn.jsdelivr.net/npm/<pkg>@<x.y.z>/...`, `unpkg.com/<pkg>@<x.y.z>/...`, `cdn.tailwindcss.com/<x.y.z>`.
- **Unpinned (refused unless `--allow-unpinned`):** `@latest`, major-only (`@3`), unversioned URLs (`https://cdn.tailwindcss.com`), and `https://fonts.googleapis.com/css2?...`.

## Licensing
Vendoring redistributes third-party libraries inside your application. `wavical vendor` records any detected `SPDX-License-Identifier` in `vendor.lock.json` and prints:
> *Vendoring redistributes these files inside your app. You are responsible for their licenses.*
