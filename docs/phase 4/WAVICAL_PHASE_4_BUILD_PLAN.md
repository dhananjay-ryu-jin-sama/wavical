# Wavical — Phase 4 Build Plan: Backend Runtime (optional) and Updates

**Goal:** two things.
1. Apps that need JavaScript backend code can opt into a **Node (or Bun) backend**, served from a **shared, version-pinned, verified runtime cache**, running with **reduced privileges**, reachable from the page only through the bridge.
2. Shipped apps can **update themselves safely**: signed update manifests, verified payloads, no downgrades, automatic rollback. Wavical's own releases go through a **canary → stable** pipeline.

Plus production hardening: no devtools, no remote debugging, no `--inspect` in release builds, verified rather than only configured.

**Why this phase matters for security:** it adds a second programming runtime with full OS access by default, archive downloads, code that updates itself, and a signing key. A mistake in the updater is a remote code execution channel into every installed app. This plan locks the rules first.

**Estimate:** 5–7 weeks part-time with AI. The updater (WP7) and the release pipeline (WP9) take the longest to get right.

---

## 0. Gate before starting

- [ ] Phase 3 audit (A7) has **no open Critical or High findings**, or each is accepted by you in writing.
- [ ] Phase 3 benchmark (WP9) was run on the 8 GB, no-GPU machine and budgets approved.
- [ ] `Cargo.lock` committed, CI green.

---

## 1. Decisions locked before code

### 1.1 Principles

1. **The backend is opt-in.** An app without `backend.enabled: true` downloads no runtime, starts no process, and has no backend code paths active.
2. **The page never talks to the backend directly.** Page → Rust bridge (capability `backend.invoke`, method on the config allowlist) → backend. Rust validates both directions.
3. **The backend is trusted app code, but its dependencies are not.** npm packages are a supply-chain risk. Defaults: install scripts off, Node's permission model on, cleaned environment, no debugger.
4. **Every downloaded executable or archive is verified before use and every runtime binary is hash-checked before every launch.** The same rule as the Phase 3 engine.
5. **A checksum fetched from the same server as the file is not enough.** Runtime hashes are pinned in Wavical's own source (a runtime manifest), after the agent has verified them against the upstream signed checksum files.
6. **Updates are accepted only when signed** by a key whose public half is embedded in the app at build time. HTTPS alone is never trusted. No downgrades. Manifests expire.
7. **The signing private key is never in the repo, never in logs, never on the page, never in a build artifact.**
8. **Release builds can't be debugged from outside**: devtools off, WebView2 remote-debugging arguments neutralized, Node `--inspect` and `NODE_OPTIONS` stripped. Tests prove it.
9. **No `unsafe` written by us.** If something needs it, stop and ask.

### 1.2 Runtime support

- **Node first.** Bun is a decision for WP1. Node has a permission model (`--permission` with fs, child-process and worker controls; WP1 confirms the exact flags for the pinned version). Bun had no equivalent as far as this plan knows. If WP1 confirms that, Bun would run **unrestricted**, and you decide whether that is acceptable or whether Bun waits.
- **Per-user cache:** `%LOCALAPPDATA%\Wavical\runtimes\<runtime>-<version>-<arch>\`. The checklist said "per machine", but a machine-wide folder needs admin rights and has harder access-control problems. Per user is the locked choice. Several apps of the same user share one copy.
- **Runtime manifest:** a file in Wavical's source listing approved runtime versions with URL, size and SHA-256 per architecture, and a source link to the upstream signed checksum file each value was checked against.

### 1.3 Version policy (resolves a contradiction in the checklist)

The checklist says both "exact version pinning" and "auto-update limited to patch level". Both are kept, as two separate settings:

- `backend.version: '22.11.0'` is an **exact pin**. By default exactly this version is used. Never anything else.
- `backend.allowPatchUpdates: true` (opt-in, default false) allows a **newer version with the same major and minor** from Wavical's runtime manifest only. Never a new minor or major. It is never silent: the CLI and the app log the change, and `wavical runtime status` shows it.
- A version not in Wavical's runtime manifest cannot be used, even if it exists upstream.

Note for WP1: Node sometimes ships security fixes as a **minor** bump inside an LTS line. "Patch only" could miss those. WP1 reports how often that happened recently, and you decide whether the policy should be "same major and minor" or "same major LTS line".

### 1.4 Backend channel and protocol

- The page-facing method is `backend.invoke({ method, params })`. Rust checks the capability and that `method` is on the config allowlist (`backend.methods`) before forwarding.
- **The protocol must not share stdout with `console.log`.** A backend that prints to stdout would corrupt the protocol. WP1 picks a dedicated channel (an extra pipe handle, a named pipe restricted to the current user, or stdio with the backend helper moving console output to stderr) and justifies it.
- Framing: length-prefixed JSON frames with a maximum frame size. Responses are matched to pending request ids. A frame from the backend that doesn't match a pending id is dropped and logged.
- Backend stdout/stderr logs go to `tracing` with a size and rate cap. They never reach the page.
- The backend gets a small JavaScript helper (`wavical-backend.js`) that registers handlers. It is copied into the project by `wavical init --backend`. No npm package is published in Phase 4.

### 1.5 Reduced privilege for the backend (defaults)

| Control | Default | Config can change? |
|---|---|---|
| Node permission model | on | no (off only by an explicit `backend.unrestricted: true` with a loud warning in CLI and logs) |
| File read | backend folder + app data folder | add paths in `backend.permissions.fsRead` |
| File write | app data folder | add paths in `backend.permissions.fsWrite` |
| Child processes | off | `backend.permissions.childProcess: true` |
| Worker threads | off | `backend.permissions.workers: true` |
| Network | **not restricted** by Node's permission model (WP1 confirms; documented plainly) | n/a |
| Environment | cleaned: no `NODE_OPTIONS`, `NODE_PATH`, `NODE_*` debug variables, no inherited secrets | add named variables in `backend.env` |
| Debugger | `--inspect*` refused in release builds | no |
| Process lifetime | dies with the app (job object or equivalent from Phase 3) | no |
| Install scripts | `npm ci --ignore-scripts` during build | `build --allow-install-scripts` with a warning |

### 1.6 Update system

- **Signing:** Ed25519 signatures over the **exact bytes** of `manifest.json` (no JSON canonicalization tricks). Signature in `manifest.json.sig`. WP1 picks the crate (a minisign-compatible format is an option so standard tools can verify too).
- **Keys:** the app embeds up to **two** public keys at build time (current and next) so keys can be rotated. The private key lives in a file outside the repo or in a CI secret. `wavical signer keygen`, `wavical release sign`.
- **Manifest fields:** app id, version, channel, minimum supported version, `expires` (date), `rolloutPercent` (0–100), payload URL, payload size, payload SHA-256, release notes (plain text, size-limited).
- **Checks, all required:** signature valid with an embedded key; app id matches; version strictly greater than installed (no downgrade, no replay of an old manifest); not expired (stops "freeze" attacks that hide new security updates); channel matches; payload size and SHA-256 match; download host on the allowlist; redirects checked each hop.
- **Install layout (side by side):** `%LOCALAPPDATA%\<appId>\versions\<version>\` plus a small stable **launcher** and a `current` pointer file written atomically. The previous version is kept for rollback. A new version that fails a health check on its first starts (for example, does not reach "ready" twice) is rolled back automatically.
- **Staged rollout:** each install has a random, stable bucket (0–99) stored locally. An update applies only if the bucket is below `rolloutPercent`. Channels: `canary`, `beta`, `stable`.
- **Page API:** capabilities `updater.check` (returns version and release notes as data) and `updater.apply` (restart into an update that is already downloaded and verified). The page can never supply a URL, version, key or channel.
- **Hosting:** any static HTTPS host (for example GitHub Releases). A hosted update service is Phase 6.
- **Phase 5 dependency:** the installer in Phase 5 must keep this layout (launcher + versioned folders + pointer). This is recorded so Phase 5 does not break the updater.

### 1.7 Wavical's own releases (canary → stable)

- crates.io and npm don't support percentage rollouts. So, for the framework itself, "canary" means **pre-release versions** (`0.5.0-canary.3`), a GitHub pre-release, and (if npm is used later) a non-default dist-tag. **Promotion to stable is a manual, reviewed step.**
- CI publishes canary builds from `main` only after all tests, audits and the attack page pass. Stable promotion uses a protected GitHub environment with a required reviewer (you).
- Signing secrets live only in that protected environment and are never echoed.
- Build provenance attestations are added if WP1 confirms the GitHub feature fits.

### 1.8 Capabilities (new)

| Capability | What it allows |
|---|---|
| `backend.invoke` | call a backend method listed in `backend.methods` |
| `updater.check` | check for an update; receive version and release notes as data |
| `updater.apply` | restart into an already-verified downloaded update |

Runtime management, signing and release are CLI-only.

### 1.9 Config sections (added to the Phase 1 validator)

```javascript
backend: {
  enabled: false,
  runtime: 'node',                 // 'node' (Bun only if approved in WP1)
  version: '22.11.0',              // exact; must exist in Wavical's runtime manifest
  allowPatchUpdates: false,
  entry: 'backend/main.js',        // inside the project; validated path
  methods: ['todos.list', 'todos.add'],   // allowlist; same grammar as IPC method names
  timeoutSeconds: 30,              // default; hard ceiling in code
  permissions: { fsRead: [], fsWrite: [], childProcess: false, workers: false },
  env: [],                         // names of extra environment variables to pass through
  unrestricted: false,             // turns off the permission model; loud warning
},
updater: {
  enabled: false,
  manifestUrl: 'https://example.com/myapp/stable/manifest.json',
  publicKeys: ['<current public key>', '<next public key>'],
  channel: 'stable',
  checkIntervalHours: 24,
  autoDownload: true,
},
```

Validator rules: exact version format; version must be in the runtime manifest; `entry` and permission paths validated (no traversal outside the allowed roots, no UNC, no reserved names); `manifestUrl` https only; public keys well-formed; `backend.invoke` requires `backend.enabled`; `updater.*` capabilities require `updater.enabled`.

### 1.10 Named limits (constants, each with a boundary test; initial values)

| Area | Limit | Value |
|---|---|---|
| Backend | request params size | 1 MB |
| Backend | response size / max frame | 4 MB |
| Backend | timeout default / ceiling | 30 s / 300 s |
| Backend | concurrent requests per window | 16 |
| Backend | declared methods | 256 |
| Backend | log line / log rate | 4 KB / 200 lines per second |
| Backend | restart retry | 3 within 60 s, then give up with a clear error |
| Runtime | archive download size | from the runtime manifest, exact |
| Runtime | extracted size / file count | 500 MB / 20,000 files |
| Updater | manifest size | 64 KB |
| Updater | release notes | 8 KB plain text |
| Updater | payload size | 1 GB |
| Updater | maximum manifest validity (`expires` minus now) | 30 days |
| Updater | failed health checks before rollback | 2 |

---

## 2. Scope

**In scope:** WP1 research and decisions; WP2 verified downloads and runtime cache; WP3 config and version policy; WP4 backend process manager and reduced privilege; WP5 `backend.invoke` bridge and helper; WP6 build and `--bundle-runtime`; WP7 app updater and signing tools; WP8 production hardening; WP9 Wavical release pipeline; WP10 integration, supply-chain test bench, self-review.

**Out of scope:** installers and code signing of `.exe`/`.msi` (Phase 5); hosted update server, SBOM, audit logger (Phase 6); backend-to-page push events; Deno; macOS and Linux; delta updates; per-machine runtime cache.

---

## 3. Work packages (in order, one per session)

### WP1: Research and decisions (NO feature code; decision gate)

Record in `docs/WAVICAL_PHASE_4_DECISIONS.md`, each row with a source link and the version shown by `cargo add`, docs.rs or the upstream page (never from memory):

1. **Node downloads:** official URLs for Windows x64 (and arm64 if relevant), archive format, where checksums are published, how the checksum file is signed, which release keys sign it, and how Wavical would verify that before pinning hashes.
2. **Node permission model** in the LTS line you propose: exact flags, what they cover, what they do not cover (network?), known bypasses or limitations listed by Node itself.
3. **Bun:** the same questions (downloads, checksums, signatures, any permission model). Recommendation: support now, later, or never.
4. **Version policy history:** in the last two years of the proposed LTS line, how many security fixes shipped as minor versus patch bumps. Recommend "same major.minor" or "same major LTS line".
5. **Backend channel:** dedicated pipe handle versus named pipe versus stdio with console redirection, and how each works from Rust (std::process) and from Node.
6. **Process lifetime and restrictions** on Windows: reuse of the Phase 3 mechanism; whether a job object can also cap child processes and memory through an existing crate (no unsafe of ours).
7. **Archive extraction:** crate for zip, protection against path traversal ("zip slip"), symlinks, absolute paths, alternate data streams, decompression bombs.
8. **Signing:** Ed25519 crate options and minisign compatibility; key file format; how CI would hold the private key.
9. **Update apply on Windows:** launcher plus versioned folders plus pointer file; how other frameworks or tools do it; how to replace the launcher itself rarely and safely; what happens if the PC loses power mid-update.
10. **Release-mode debugging surfaces:** how `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` and the WebView2 policy registry keys affect a wry app; whether wry lets you set browser arguments that override or block them; whether relaunching the app with a cleaned environment is a safe way to neutralize them (note: `std::env::remove_var` is `unsafe` in the Rust 2024 edition; check the workspace edition). For Node: which environment variables and flags can enable a debugger or preload code (`NODE_OPTIONS`, `--inspect`, `--require`, `--import`, and others).
11. **Wavical's release pipeline:** GitHub protected environments, required reviewers, artifact attestations, pre-release tags; what crates.io supports for pre-releases.
12. **CLI and Node:** whether the CLI can use the managed runtime to read `wavical.config.js`, so developers no longer need Node installed (fixes the Phase 1 dependency noted earlier).

End with "Decisions I need from you": items 3, 4, 5, 8 (key storage), 9 (layout), 12, and whether `backend.unrestricted` should exist at all. **Stop.**

### WP2: Shared verified downloads and runtime cache

- Turn the Phase 3 downloader into one shared module (used by models, the AI engine, runtimes and updates): HTTPS only, allowlisted hosts with per-hop redirect checks, exact size, SHA-256, `.part` files, resume, atomic rename, one download at a time per target.
- Safe archive extraction: reject entries with `..`, absolute paths, drive letters, UNC, alternate data streams, reserved names, symlinks; enforce extracted size and file-count limits; extract to a temp folder, then rename into the cache.
- Runtime manifest in source, with source links. `wavical runtime list | install <version> | remove <version> | verify <version> | status`.
- The runtime binary's hash is checked before every launch.
- Tests with a mock server and crafted archives: zip slip, absolute path entry, symlink entry, ADS name, reserved name, 10,000 empty files, a small file that expands to 2 GB, wrong hash, wrong size, off-list redirect.

### WP3: Config and version policy

- `backend` and `updater` config sections and rules from 1.9; capability rules from 1.8.
- Version resolver as a pure function: input pinned version, `allowPatchUpdates`, manifest versions; output exactly one version or an error. Property-tested: never returns a different major or minor; never returns a version not in the manifest; returns the pin when patch updates are off.
- If approved in WP1: the CLI reads `wavical.config.js` with the managed runtime instead of a system Node.

### WP4: Backend process manager and reduced privilege

- Spawn with the permission-model flags from 1.5, a cleaned environment, the backend folder as working directory, the lifetime mechanism from WP1, and no debugger flags in release.
- Lifecycle: start with the app when `backend.enabled`; restart with the retry limit; stop on exit; kill with the app on crash.
- Backend logs to `tracing` with caps. Never to the page.
- Tests: a fake backend written in Rust speaking the protocol for fast tests; plus Node integration tests (Windows CI runner with Node, or local) that prove: reading a file outside the allowed roots fails; spawning a child process fails when not allowed; `NODE_OPTIONS=--require=evil.js` set in the parent has no effect; `--inspect` is refused in release; killing the app leaves no Node process.

### WP5: `backend.invoke` bridge and helper

- Capability check, method allowlist, params validation and limits, then forward. Timeouts, concurrency per window, cancel pending calls when the window closes.
- Backend responses: size, depth and JSON validation; unknown ids dropped; errors to the page are generic.
- `wavical-backend.js` helper: `handle(name, fn)`, frames, errors mapped to a generic shape, console output kept off the protocol channel.
- `wavical init --backend` scaffolds `backend/main.js` with one example method and the config entries.
- Tests: malformed frames, oversize frames, partial frames, interleaved responses, a backend printing to stdout, a backend that never answers, a backend that answers twice, a method not on the allowlist, a page calling with a method name that differs only in case or whitespace.

### WP6: Build and `--bundle-runtime`

- `wavical build` with a backend: `npm ci --ignore-scripts` (from the lockfile, never `npm install`), copy backend code and dependencies, include their hashes in the build manifest, warn about install scripts that were skipped.
- `--bundle-runtime`: copy the verified pinned runtime into the build, include its license file, and make the app use the bundled copy instead of the shared cache. Offline start must work.
- `wavical dev` restarts the backend when backend files change.
- Tests: build without lockfile fails with a clear message; install scripts are not run (a test package with a postinstall that writes a marker file; the marker must not exist); bundled build starts with the network disabled (manual); runtime license present.

### WP7: App updater and signing tools

- `wavical signer keygen` (writes the private key to a file you choose, never into the project; prints only the public key), `wavical release sign <manifest>`, `wavical release manifest` (builds a manifest from a built payload).
- In the app: check on schedule and on `updater.check`; download through the shared module; verify everything in 1.6; stage into `versions\<new>\`; apply on restart by switching the pointer atomically; health check; rollback after the failure limit; keep the previous version.
- Page API behind `updater.check` and `updater.apply`.
- Tests with a mock server: tampered payload, tampered manifest, signature from an unknown key, valid signature for another app id, replayed older manifest, equal version, expired manifest, `expires` too far in the future, channel mismatch, rollout bucket below and above the percentage, size mismatch, off-list redirect, interrupted download, power loss simulated during the pointer switch (kill the process at each step), new version that fails health checks (rollback), key rotation (manifest signed by the "next" key accepted; after rotation the old key is rejected if the app was built with only the new key).

### WP8: Production hardening

- Release builds: devtools off (compile-time and checked at runtime), WebView2 remote-debugging arguments neutralized by the approach from WP1, Node debug flags and variables stripped, a startup self-check that logs (and in release refuses) unsafe debug settings.
- Tests: a release build started with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` must not answer on that port; `NODE_OPTIONS=--inspect` must not open a Node inspector port; a debug build still allows devtools.

### WP9: Wavical's own release pipeline

- GitHub Actions: on `main`, after all checks (tests, clippy, fmt, audit, deny, attack page where runnable, audit crate), publish a **canary** pre-release with signed artifacts and checksums. Promotion to **stable** is a separate workflow that needs your approval through a protected environment.
- Signing key only in the protected environment. Workflows declare minimal permissions. Third-party actions pinned to a commit SHA.
- `docs/RELEASING.md`: version scheme, how to cut a canary, how to promote, how to roll back, what to do if the signing key leaks.
- A dry-run mode that runs everything except publishing.

### WP10: Integration, supply-chain test bench, self-review

- Attack page: `backend.invoke` without capability, methods not on the allowlist, oversize and malformed calls, floods, attempts to reach the backend directly, attempts to call `updater.apply` with a URL or version.
- **Update-tampering bench** and **runtime-download-tampering bench** (required by the checklist): a malicious mock server in `tests/supply-chain/` that serves every bad case from WP2 and WP7, run in CI. Each case must be rejected.
- Fuzz targets: frame parser, update manifest parser, archive entry path checker, version resolver.
- Mutation proofs for the new rules. Full regression of Phases 0–3.
- Docs: SECURITY_MODEL.md (backend and updater sections), `docs/UPDATER.md` for app developers (key management, rotation, lost-key recovery, hosting), `docs/BACKEND.md` (permissions, what is not restricted, supply-chain advice).
- Hostile self-review of the downloader, extractor, updater, launcher, signing tools and backend launcher.
- Combined manual checklist.

---

## 4. Manual checks

Collected in WP10. The agent can't see the GUI, can't pull a real update from your server, and can't prove power-loss behaviour on real hardware. It must say which checks were automated and which were not run.

## 5. Not needed in Phase 4

Installers and code signing of executables, hosted update server, SBOM, audit logger, delta updates, backend push events, Deno, macOS and Linux.
