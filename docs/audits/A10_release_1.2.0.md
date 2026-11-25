# Audit Report A10: Release Artifact (`wavical-1.2.0.tgz`)

- **Target Artifact:** `wavical-1.2.0.tgz` (packed locally from clean `HEAD` commit `8b028510ff397b185d9ee514413895cf00880806`; `14,052,793` bytes; tarball SHA-256 `ea044855100687d3aa3eb7fc96e1452b9b9784f55c91177f70b55d2accb2651b`)
- **Registry Context:** `npm view wavical versions --json` shows `wavical@1.2.0` was already published to npm at `2026-11-25T14:24:56.052Z`
- **Mode:** AUDIT ONLY (independent inspection of unpacked `.tgz`, registry `.tgz`, clean-checkout rebuild, binary string scan, and installed E2E/build tests; no repository files modified)
- **Date:** 2026-11-25
- **Auditor:** Independent Release & Security Auditor

---

## 1. Summary Table

| # | Audit Item | Status | Summary |
|---|---|---|---|
| 1 | Tarball unpack & `bin/checksums.txt` | **PASS** | All 8 files unpacked cleanly; `bin/checksums.txt` matches all 3 `.exe` SHA-256 hashes verbatim. |
| 2 | Provenance (`--version`, git tag, clean rebuild) | **FAIL** | Binaries report `1.2.0 (8b02851)` and clean-checkout rebuild matches `--version`, `--help`, and embedded CSP strings (non-byte-reproducible PE builds), **but** `git tag -l` is empty (no `v1.2.0` / `1.2.0` tag exists). |
| 3 | Not stale (`npm pack wavical@<previous>`) | **FAIL** | Binaries differ from broken `wavical@1.1.0`, **but** `wavical@1.2.0` is already published on npm (`2026-11-25T14:24:56.052Z`), blocking any re-publish of `wavical-1.2.0.tgz`. |
| 4 | Features present & shell isolation | **PASS** | CLI `--help` lists all 18 commands; prod `wavical-shell.exe` embeds `open`/`safe`/`ultra-safe` CSP templates and excludes DevTools, stdin command channel, and `--dev-url`/`--dev`/`--record`; `wavical-shell-dev.exe` contains them. |
| 5 | Docs vs binary (`README.md` & `CHANGELOG.md`) | **FAIL** | 0 CLI/flag mismatches and 0 missing config/allowlist/capability keys, **but** the verbatim `wavical.config.js` example in `README.md` fails `wavical` config validation (`updater.apply` enabled while `updater.enabled: false`). |
| 6 | Install test & F6 reference-app harness | **FAIL** | `npm install wavical-1.2.0.tgz` succeeded; `e2e-reference-app.js` passed **6/7** checks offline and **6/7** `--online`, failing `dev (open) + hot reload` at the 15s startup timeout (cold-start `npx` + AV scan measured at `25,552ms` in standalone probe). |
| 7 | Built-app check (`build*` outputs) | **PASS** | Bare `wavical build` refuses; `build --allow-open`, `build-safe`, and `build-ultra-safe` produce valid `wavical-manifest.json` modes and exclude `wavical-shell-dev.exe`. |
| 8 | Package hygiene | **FAIL** | No source maps, `.env`, test fixtures, or private keys; `os`/`cpu`/`bin` correct and sizes reasonable (14.1 MB packed / 31.3 MB unpacked), **but** `LICENSE*` in `package.json` `"files"` matches 0 files (missing license file). |

---

## 2. Detailed Evidence per Item

### Item 1: Unpack & Checksums — **PASS**

Unpacked `wavical-1.2.0.tgz` into a fresh temp directory (`tar -xzf`):

| File | Size (bytes) | SHA-256 |
|---|---:|---|
| `CHANGELOG.md` | 6,202 | `6aa72e51ef5c214c13be8a385f0c58d853680177ec268f6f05173af34f79c542` |
| `README.md` | 17,086 | `c456bbb1d96a123556dfae8b6fffdf39612806e50af174d41e3b05701b79b8cc` |
| `bin/checksums.txt` | 250 | `528afdc68a7a2a0c5b98f22ddc10731e74194faaac1f7b4e733b469af745d05f` |
| `bin/wavical-shell-dev.exe` | 9,807,360 | `012586fa9060864e72a2ed8ba37cf8ee7e1b7194b8daa9e39da90a66dbc895dc` |
| `bin/wavical-shell.exe` | 9,489,920 | `3c577f6f66788bee27a7436683a9c20647b471254ed1ea7d7d0cb31c165042d1` |
| `bin/wavical.exe` | 11,981,312 | `4fcf9eedf2d9f9464d5ef9cf85a56e93ffc3bc1817dfb355fb095205e8e4c84a` |
| `bin/wavical.js` | 774 | `d470845cff5ee4215b8a128637bd9a23bfb77a41ef6d09c2e3a6703c3a92a6df` |
| `package.json` | 1,736 | `2b1cd835fe96bc7079bc304d31be268a6c12acf8b4f7aea1853e773c03635a59` |

`bin/checksums.txt` inside the tarball matched all three binary SHA-256 digests (`match=true` for `wavical.exe`, `wavical-shell.exe`, and `wavical-shell-dev.exe`).

### Item 2: Provenance & Clean-Checkout Rebuild — **FAIL**

- Unpacked binary `--version` outputs vs `package.json` (`"version": "1.2.0"`):
  - `bin/wavical.exe --version` -> `wavical 1.2.0 (8b02851)`
  - `bin/wavical-shell.exe --version` -> `wavical-shell 1.2.0 (8b02851)`
  - `bin/wavical-shell-dev.exe --version` -> `wavical-shell-dev 1.2.0 (8b02851)`
- Git tag check: `git tag -l` returned empty (`""`); no `v1.2.0` or `1.2.0` tag exists in the repository (`HEAD` is `8b028510ff397b185d9ee514413895cf00880806`). **Fails** the `git rev-parse <tag>` provenance requirement (`A10-02`).
- Clean-checkout rebuild (`git clone --no-hardlinks` at `8b028510ff397b185d9ee514413895cf00880806`, rebuilt with `cargo build --release --locked` in separate runs):

| Binary | Tarball SHA-256 | Clean-Rebuild SHA-256 | Byte-Reproducible | `--version` Match | `--help` Match | Embedded CSP Match |
|---|---|---|:---:|:---:|:---:|:---:|
| `wavical.exe` | `4fcf9eed...e4c84a` | `a352ade1...2199d5` | No | Yes | Yes | N/A |
| `wavical-shell.exe` | `3c577f6f...5042d1` | `17b99e45...6bb616` | No | Yes | Yes | Yes |
| `wavical-shell-dev.exe` | `012586fa...c895dc` | `ef96dc60...3c8cb1b` | No | Yes | Yes | Yes |

Note: Non-byte-reproducibility across MSVC PE release builds is noted as expected and does not fail behavioural equivalence.

### Item 3: Not Stale (`npm pack wavical@<previous>`) — **FAIL**

- `npm view wavical versions --json` returned: `["0.0.0-stage", "1.0.0", "1.0.1", "1.0.2", "1.1.0", "1.2.0"]`.
- Compared against `npm pack wavical@1.1.0`:
  - `wavical@1.1.0` `bin/wavical.exe` (`2a55e6f9729683ee3db1059a105ac29ecae49a842e113c741e8b9ade0efa4f43`) differs from `1.2.0` (`4fcf9eed...`).
  - `wavical@1.1.0` `bin/wavical-shell.exe` (`49e03fd3822044405821b886e57a80e6490b81f014e9f1ac276ad49edd7c62db`) differs from `1.2.0` (`3c577f6f...`).
- **Critical registry finding (`A10-01`):** `wavical@1.2.0` was already published to npm at `2026-11-25T14:24:56.052Z`. Unpacking `npm pack wavical@1.2.0` confirms the published `wavical@1.2.0` contains `8b02851` binaries (`wavical.exe` `4fcf9eed...` identical, `wavical-shell-dev.exe` `012586fa...` identical, `wavical-shell.exe` `33b7fdba...` reporting `wavical-shell 1.2.0 (8b02851)`). However, `wavical-1.2.0.tgz` cannot be published again under `1.2.0` on npm.

### Item 4: Features Present & Shell Isolation — **PASS**

- Unpacked `wavical.exe --help` lists all 18 commands: `init`, `dev`, `dev-safe`, `dev-ultra-safe`, `allowlist`, `vendor`, `build`, `build-safe`, `build-ultra-safe`, `register-filetypes`, `unregister-filetypes`, `model`, `ai`, `secret`, `runtime`, `signer`, `release`, `doctor`.
- Direct binary ASCII scan of unpacked `wavical-shell.exe` (in place of GNU `strings`, which is not installed on this Windows host):
  - **CSP templates present:**
    - `ultra-safe` verbatim: `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-src 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self';` (`true`)
    - `open`: `default-src 'self' https:` and `'unsafe-inline' 'unsafe-eval'` (`true`)
    - `safe`: `default-src 'self'; script-src ` and `frame-ancestors 'none'; form-action 'self';` (`true`)
  - **Dev/probe surfaces absent in `wavical-shell.exe`:**
    - DevTools/CDP: `--remote-debugging-port` (`false`), `WAVICAL_DEV_REMOTE_DEBUGGING_PORT` (`false`), `with_devtools` (`false`), `open_devtools` (`false`). Only `hardening.rs` release enforcement strings (`"devtools enabled in release build"`, `devtools_disabled`) and startup log message (`"Initializing webview (devtools enabled: "`) are present.
    - Stdin command channel: `WAVICAL_DEV_SHELL_CHANNEL_V1_MARKER` (`false`), `WAVICAL_DEV_SESSION_TOKEN` (`false`), `reload-config` (`false`), `restart-backend` (`false`), `--listen-stdin` (`false`), `listen_stdin` (`false`).
    - Test probe markers: `WAVICAL_TEST_PROBE_V1_MARKER` (`false`), `WAVICAL_TEST_PROBE_PORT_FILE` (`false`).
    - `dev.url` / dev flags: `--dev`, `--dev-url`, `--record` absent from `wavical-shell.exe --help` and PE strings (only shared `net_policy.rs` tracing event name `invalid_dev_url` is present; `is_dev_command = false` in `main.rs`).
- `wavical-shell-dev.exe` scan and `--help` confirm presence of `--dev`, `--dev-url`, `--record`, `--remote-debugging-port` (`WAVICAL_DEV_REMOTE_DEBUGGING_PORT`), `WAVICAL_DEV_SHELL_CHANNEL_V1_MARKER`, `WAVICAL_DEV_SESSION_TOKEN`, `reload-config`, and `restart-backend`.

### Item 5: Docs vs Binary (`README.md` & `CHANGELOG.md`) — **FAIL**

- Checked every CLI invocation and standalone flag in unpacked `README.md` (49 invocations, 2 standalone flags, excluding `Roadmap` rows marked `Planned`) and `CHANGELOG.md` (19 invocations, 5 standalone flags) against recursive `wavical.exe --help`: **0 mismatches**.
- Checked all 47 documented `wavical.config.js`, `wavical.allowlist.json`, and `wavical-manifest.json` keys and all 19 capability names in `README.md` against `wavical-config`, `wavical-netpolicy`, `wavical-cli`, and `wavical-ipc`: **0 missing keys or capabilities**.
- **Finding (`A10-05`, Low):** Validating the verbatim `wavical.config.js` snippet from `README.md` (`## Configuration (wavical.config.js)`) with unpacked `wavical.exe build-ultra-safe` fails with:
  `error: invalid config: in 'capabilities.updater.apply': capability 'updater.apply' requires 'updater.enabled: true'; set 'updater: { enabled: true }' in wavical.config.js to enable updater capabilities` (because the snippet grants `'updater.check': true` and `'updater.apply': true` while setting `updater.enabled: false`).

### Item 6: Install Test & F6 Reference-App Harness — **FAIL**

- `npm install wavical-1.2.0.tgz` in a temp project succeeded (`exit 0`).
- Ran `node scripts/e2e-reference-app.js --tarball wavical-1.2.0.tgz` (offline) and `--online`:

| Step | Offline Result | Online Result |
|---|---|---|
| `cli: npx wavical --version matches tarball version` | **PASS** (15,756 ms) | **PASS** (15,736 ms) |
| `dev (open) + hot reload` | **FAIL** (`Timed out after 15000ms waiting for npx wavical dev startup`) | **FAIL** (`Timed out after 15000ms waiting for npx wavical dev startup`) |
| `dev-safe` | **PASS** (12,774 ms) | **PASS** (17,059 ms) |
| `dev-ultra-safe` | **PASS** (11,885 ms) | **PASS** (12,094 ms) |
| `build --allow-open` | **PASS** (20,109 ms) | **PASS** (18,092 ms) |
| `build-safe` | **PASS** (13,913 ms) | **PASS** (18,293 ms) |
| `build-ultra-safe` | **PASS** (12,438 ms) | **PASS** (12,797 ms) |

- Independent cold-start probe (`a10_dev_probe2.js` with 45s timeout) confirmed `npx --no-install wavical dev` did **not** hang or fail functionally: first `stderr` output arrived at `+16,739 ms` (`npx` + Windows Defender cold-start scan of unpacked `wavical.exe`) and `Dev server running (PID: 35640)` (`mode=open | shell=1.2.0 (8b02851)`) printed at `+25,552 ms`. Because `scripts/e2e-reference-app.js:705` hardcodes a `15,000 ms` startup timeout, the first `dev` step times out while subsequent `dev-safe` / `dev-ultra-safe` steps pass after executable warm-up. (Note: `--online` also threw `EPERM` removing its temp directory during cleanup.)

### Item 7: Built-App Check — **PASS**

Installed `wavical-1.2.0.tgz` into a fresh temp copy of `tests/fixtures/reference-app` and ran all build modes:

- Bare `npx --no-install wavical build`: exited `1` with `error: refusing to run 'wavical build' without an explicit network mode.` listing `--allow-open`, `build-safe`, `build-ultra-safe`.
- `npx --no-install wavical build --allow-open --outdir dist-open`: exited `0`. Output files: `['index.html', 'script.js', 'style.css', 'wavical-manifest.json']`. Manifest: `"mode": "open"`, `"externalLinks": "browser"`, 3 assets hashed.
- `npx --no-install wavical build-safe --outdir dist-safe`: exited `0`. Output files: `['index.html', 'script.js', 'style.css', 'wavical-manifest.json', 'wavical.allowlist.json']`. Manifest: `"mode": "safe"`, `"allowlistSha256": "7472339c68d61eeb90d8b1d37b5b337b08ba2c5dafe0b9cba45a17e4cfac61b6"`, `"externalLinks": "browser"`, 3 assets hashed.
- `npx --no-install wavical build-ultra-safe --outdir dist-ultra-safe`: exited `0`. Output files: `['index.html', 'script.js', 'style.css', 'wavical-manifest.json']`. Manifest: `"mode": "ultra-safe"`, `"externalLinks": "browser"`, 3 assets hashed.
- Confirmed `wavical-shell-dev.exe` is absent from all three outputs (`wavical build*` outputs assets + manifest only, executed by production `wavical-shell.exe`).

### Item 8: Package Hygiene — **FAIL**

- Package contents: exactly 8 files, all matching `package.json` `"files"`. Zero `.map`, `.env`, test fixtures, or private key files.
- Size: `14,052,793` bytes (~14.1 MB) compressed, ~31.3 MB unpacked (3 Windows x64 Rust/WebView2/SQLite/AI-enabled binaries + docs/wrapper).
- `package.json` metadata: `"os": ["win32"]`, `"cpu": ["x64"]`, `"engines": { "node": ">=14.8.0" }`, `"bin": { "wavical": "bin/wavical.js", "wca": "bin/wavical.js" }` — all accurate.
- **Finding (`A10-04`, Medium):** `package.json` `"files"` declares `"LICENSE*"` and `"license": "MIT OR Apache-2.0"`, and `README.md` states dual MIT / Apache-2.0 licensing, but **no `LICENSE` / `LICENSE-MIT` / `LICENSE-APACHE` file exists in the repository or in `wavical-1.2.0.tgz`**.

---

## 3. Findings by Severity

| ID | Severity | Item | Title & Evidence | Recommended Fix |
|---|---|---|---|---|
| **A10-01** | **High** | #3 | **`wavical@1.2.0` Already Published on npm** — `npm view wavical versions` includes `"1.2.0"` (`2026-11-25T14:24:56.052Z`). While the published `1.2.0` contains `8b02851` binaries (not stale `1.1.0`), `wavical-1.2.0.tgz` cannot be re-published as `1.2.0`. | Cut `1.2.1` (if shipping fixes below) instead of attempting to re-publish `1.2.0`. |
| **A10-02** | **High** | #2 | **Missing Git Release Tag** — `git tag -l` is empty; no `v1.2.0` / `1.2.0` tag resolves via `git rev-parse <tag>` for `8b02851`. | Tag the exact release commit before packing/publishing. |
| **A10-03** | **Medium** | #6 | **F6 E2E `dev (open)` Cold-Start Timeout (`1/7` FAIL)** — `scripts/e2e-reference-app.js` fails `dev (open) + hot reload` offline and `--online` due to its `15,000 ms` `waitUntil` limit vs `25,552 ms` measured cold-start `npx` + AV scan time. | Warm the unpacked binary or increase the cold-start `waitUntil` budget in `scripts/e2e-reference-app.js` (and handle Windows temp-dir `EPERM` cleanup retry). |
| **A10-04** | **Medium** | #8 | **Missing `LICENSE*` File in Package** — `package.json` `"files"` lists `"LICENSE*"`, `"license": "MIT OR Apache-2.0"`, yet 0 license files exist in repo/tarball. | Add `LICENSE-MIT` and `LICENSE-APACHE` (or `LICENSE`) to the repository root so `npm pack` includes them. |
| **A10-05** | **Low** | #5 | **`README.md` Example `wavical.config.js` Fails Validation** — Snippet enables `'updater.check'` / `'updater.apply'` while `updater.enabled` is `false`. | Either comment out `updater.*` capabilities in the example or set `updater.enabled: true` in `README.md`. |

---

## 4. Verdict

do not publish
