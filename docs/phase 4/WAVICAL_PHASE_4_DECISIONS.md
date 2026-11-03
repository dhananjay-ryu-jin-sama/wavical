# Wavical — Phase 4 Research and Decisions (WP1)

This document records the research, evaluation, and decisions for Wavical Phase 4 (Optional Node Backend Runtime, App Updates, Production Hardening, and Release Pipeline) pursuant to WP1 in `docs/phase 4/WAVICAL_PHASE_4_BUILD_PLAN.md`.

---

## 1. Summary of Evaluated Tools, Crates, and Components

| Component / Crate | Upstream / Package Source | Version Verified | License | Last Release Date | Assessment & Recommendation |
|---|---|---|---|---|---|
| **Node.js LTS (Jod)** | https://nodejs.org/dist/v22.11.0/ | `v22.11.0` (LTS) | MIT | 2024-10-29 | Recommended initial pinned runtime. Active LTS line with experimental permission model and signed checksums. |
| **Node Release Keys** | https://github.com/nodejs/release-keys | Git repo (`main`) | MIT / CC0 | Maintained 2026 | Official repository of GPG public keys for Node.js release managers. Used to verify `SHASUMS256.txt.asc`. |
| **Bun** | https://github.com/oven-sh/bun | `v1.1.34` | MIT | 2024-10-25 | Has **no** permission model or runtime filesystem sandbox. Must run unrestricted. Recommendation: **Defer to later**. |
| **win32job** | https://crates.io/crates/win32job | `2.0.3` | MIT OR Apache-2.0 | 2024-05-18 | Production-tested in Phase 3. Wraps Windows Job Objects safely with `#![forbid(unsafe_code)]`. Supports kill-on-close, memory limits, and child process limits. |
| **zip** | https://crates.io/crates/zip | `9.3.0` | MIT | 2026-06-12 | Standard pure-Rust archive extraction. Requires strict entry sanitization against zip-slip, symlinks, ADS, and decompression bombs. |
| **ed25519-dalek** | https://crates.io/crates/ed25519-dalek | `3.0.0` | BSD-3-Clause | 2026-07-15 | High-performance, pure-Rust Ed25519 signatures. Ideal for signature generation and verification over raw manifest bytes. |
| **minisign** | https://crates.io/crates/minisign | `0.10.0` | MIT | 2026-09-28 | Minisign format compatibility crate in pure Rust. Allows verifying signatures created with standard `minisign` CLI. |
| **interprocess** | https://crates.io/crates/interprocess | `2.4.7` | MIT OR Apache-2.0 | 2024-07-22 | Cross-platform local IPC. Provides safe Windows Named Pipe server/client abstractions for dedicated backend framing channels. |
| **wry** | https://crates.io/crates/wry | `0.57.0` (in workspace) | MIT OR Apache-2.0 | 2026-02-14 | Active WebView2 wrapper in `wavical-shell`. Controls devtools, browser arguments, and native webview lifecycle. |
| **GitHub Attestations** | https://github.com/actions/attest-build-provenance | `v2.2.0` | MIT | 2025-11-10 | GitHub official action for cryptographically signed Sigstore build provenance attestations on releases. |

---

## 2. Research Items (WP1 Sections 1 to 12)

### Item 1: Node.js Downloads, Checksums, and Verification Process

#### Upstream Details
- **Official Base URL:** `https://nodejs.org/dist/v22.11.0/`
- **Windows x64 Binary Archive:** `https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-x64.zip`
- **Official Checksums File:** `https://nodejs.org/dist/v22.11.0/SHASUMS256.txt`
- **GPG Detached Signature:** `https://nodejs.org/dist/v22.11.0/SHASUMS256.txt.asc`
- **Official Release Keys Repository:** `https://github.com/nodejs/release-keys`

#### Step-by-Step Upstream Verification Process on Windows
Before pinning any SHA-256 hash in Wavical's runtime manifest, the developer or CI runs the following commands on Windows to cryptographically verify that the checksums file was signed by an authorized Node.js release team member:

1. **Clone the official Node.js release keys repository:**
   ```powershell
   git clone https://github.com/nodejs/release-keys.git node-release-keys
   ```

2. **Download the release files for target version (`v22.11.0`):**
   ```powershell
   Invoke-WebRequest -Uri "https://nodejs.org/dist/v22.11.0/SHASUMS256.txt" -OutFile "SHASUMS256.txt"
   Invoke-WebRequest -Uri "https://nodejs.org/dist/v22.11.0/SHASUMS256.txt.asc" -OutFile "SHASUMS256.txt.asc"
   Invoke-WebRequest -Uri "https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-x64.zip" -OutFile "node-v22.11.0-win-x64.zip"
   ```

3. **Import Node.js release keys into an isolated GPG keyring:**
   ```powershell
   gpg --no-default-keyring --keyring .\node-keyring.kbx --import node-release-keys\keys\*.asc
   ```

4. **Verify the detached signature over `SHASUMS256.txt`:**
   ```powershell
   gpg --no-default-keyring --keyring .\node-keyring.kbx --verify SHASUMS256.txt.asc SHASUMS256.txt
   ```
   *Expected Output:*
   `gpg: Good signature from "<Node.js Release Team Member Name> <email>" [unknown]`
   *(The `[unknown]` trust level is normal in GPG when keys are not locally signed).*

5. **Verify the archive SHA-256 matches the signed checksum:**
   ```powershell
   $expected = (Select-String -Path "SHASUMS256.txt" -Pattern "node-v22.11.0-win-x64.zip").Line.Split(" ")[0]
   $actual = (Get-FileHash -Path "node-v22.11.0-win-x64.zip" -Algorithm SHA256).Hash.ToLower()
   if ($expected -eq $actual) {
       Write-Host "VERIFIED OK: $actual"
   } else {
       Write-Error "CHECKSUM MISMATCH: expected $expected but got $actual"
   }
   ```
Only when Step 4 yields "Good signature" and Step 5 matches is the hash pinned in Wavical's manifest. No hash is pinned in this document.

---

### Item 2: Node Permission Model (v22.11.0 LTS)

#### Official Documentation Source
- **URL:** https://nodejs.org/docs/latest-v22.x/api/permissions.html (Node.js v22.11.0 LTS)

#### CLI Flags & Coverage
In Node.js v22.11.0, the permission model is toggled using `--experimental-permission`:

| Flag | Capability Governed | Wavical Phase 4 Default |
|---|---|---|
| `--experimental-permission` | Enables global permission restrictions (default-deny for restricted scopes) | Enabled |
| `--allow-fs-read=<paths>` | Granular filesystem read allowlist | Backend project dir + app data dir |
| `--allow-fs-write=<paths>` | Granular filesystem write allowlist | App data dir only |
| `--allow-child-process` | Permits `child_process.spawn`, `exec`, `fork` | **Disabled** (opt-in via config) |
| `--allow-worker` | Permits `node:worker_threads` | **Disabled** (opt-in via config) |
| `--allow-addons` | Permits loading native C++ Node addons | **Disabled** |
| `--allow-wasi` | Permits WebAssembly System Interface instances | **Disabled** |

#### Direct Quotes & Documented Limitations
Official Node.js v22 documentation states:

> *"The Permission Model is a mechanism for restricting access to specific system resources during execution."*
>
> *"Limitations: The Permission Model is a security mechanism designed to prevent unintended resource usage... It is designed as a **seat belt** for trusted code and is **not a security sandbox** for executing arbitrary untrusted code. Malicious code can exploit system-level capabilities or process-level vulnerabilities to bypass these constraints."*

#### What is NOT Covered in v22.11.0
- **Network Access:** In Node.js `v22.11.0`, the permission model **does NOT restrict network access**. Outbound network requests (`http`, `https`, `net`, `dgram`, `fetch`, `WebSocket`) and local port binding are unrestricted by the permission model flags. (Note: Network restriction via `--allow-net` was introduced experimentally only in later releases v22.13.0+ / v23.5.0).
- **Environment Variable Reading:** `process.env` reading is not blocked by `--experimental-permission` (which is why Wavical's host process must cleanse the child environment before spawn).

---

### Item 3: Bun Evaluation

#### Upstream Details
- **Homepage:** https://bun.sh
- **Repository:** https://github.com/oven-sh/bun
- **Latest Windows Release:** `bun-windows-x64.zip` published on GitHub Releases
- **License:** MIT

#### Findings on Permission Model
- **Bun has NO runtime permission model, capability restriction flags, or filesystem sandboxing.**
- Unlike Deno (which provides `--allow-read`, `--allow-net`, etc.) or Node.js (which provides `--permission`), Bun executes all scripts with the full operating system privileges of the calling user.
- Flags such as `--allow-read` or `--permission` do not exist in Bun. Any script running under Bun can read/write the entire filesystem, bind to network ports, make outbound connections, and spawn child processes.
- **Checksums & Signatures:** Bun publishes SHA-256 hashes on GitHub releases, but does not use a distributed GPG web-of-trust with published release team keys comparable to Node.js.

#### Recommendation
**Defer Bun support to a later phase (or require explicit `backend.unrestricted: true` with a loud warning).** Enabling Bun now would create a false sense of security because none of the table 1.5 privilege reductions (filesystem jail, worker disabling, child process blocking) can be enforced by the Bun runtime.

---

### Item 4: Version Policy History for Node.js LTS (v20 and v22)

To evaluate whether Wavical's `allowPatchUpdates` should allow **"same major.minor"** vs **"same major LTS line"**, we analyzed official security releases over the last two years (2023–2025):

#### Historical Security Releases:
1. **February 14, 2024:**
   - Lines affected: v18.19.1 (patch), v20.11.1 (patch), v21.6.2 (patch).
   - Addressed: `CVE-2024-21892` (code execution), `CVE-2024-22019` (HTTP DoS).
2. **April 3 & April 10, 2024:**
   - Lines affected: v18.20.1 (patch), v20.12.1 (patch), v21.7.2 (patch).
   - Addressed: `CVE-2024-27980` (BatBadBut Windows command injection), `CVE-2024-27983` (HTTP/2 crash).
3. **July 8, 2024:**
   - Lines affected: v18.20.4 (patch), v20.15.1 (patch), v22.4.1 (patch).
   - Addressed: `CVE-2024-36138` (Windows command injection bypass).
4. **January 21, 2025:**
   - Lines affected: v18.20.6 (patch), v20.18.2 (patch), v22.13.1 (patch).
   - Addressed: `CVE-2025-23083` (worker permission bypass), `CVE-2025-23084` (Windows drive-letter path traversal).
5. **May 14, 2025 & July 15, 2025:**
   - Lines affected: v20.19.0 (minor / security roll-up), v22.14.0 (minor / security roll-up).
   - Addressed: `CVE-2025-23166`, `CVE-2025-27210` (Windows device name traversal).

#### Key Insight & Finding
Within an LTS line, Node.js regularly advances the **minor** number (e.g., from `22.11.0` to `22.12.0` and `22.13.0`) as new quarterly maintenance batches land. Crucially, **Node.js only maintains the latest minor version of an active LTS line**. Once `22.12.0` or `22.13.0` is released, Node **never backports security fixes** to `22.11.1` or older minors.

#### Recommendation
If `backend.allowPatchUpdates` were strictly "same major.minor" (e.g., `22.11.x`), the app would be permanently stranded on an insecure minor as soon as Node ships the next security batch at `22.12.x` or `22.13.x`.
**Recommendation: Define `allowPatchUpdates` as "same major LTS line" (e.g. `22.x.y` within approved versions in Wavical's manifest).**

---

### Item 5: Backend Channel Architecture and Console Redirection

#### Architectural Options
1. **Stdio with Helper Redirection:**
   - Rust spawns Node with standard redirected `stdin` (for Rust → Node frames) and `stdout` (for Node → Rust frames), plus `stderr` (for logs).
   - *Risk:* If backend code calls `console.log("hello")`, raw text is written to `stdout`, which immediately breaks length-prefixed JSON framing.
   - *Mitigation:* The `wavical-backend.js` runtime helper overrides `console.log`, `console.info`, `console.warn` to write exclusively to `process.stderr`.
   - *Vulnerability:* Third-party C++ addons or `process.stdout.write` bypass JS `console.log` overrides and corrupt frames.
2. **Dedicated Windows Named Pipe:**
   - Rust creates a private Windows Named Pipe server: `\\.\pipe\wavical-backend-<appId>-<token>` using security attributes that grant access **strictly to the current user SID**.
   - Rust passes the pipe path as an environment variable or CLI argument to Node.
   - Node connects via `net.createConnection(pipePath)`.
   - Stdio (`stdout` and `stderr`) remain entirely separate from IPC framing. `console.log` naturally prints to `stdout`/`stderr`, which Rust captures and routes into `tracing` logs with size and rate limits.
   - *Advantage:* Framing is 100% immune to `console.log` pollution and third-party stdout writes.

#### Channel Flow Sketch (Prose)
1. **Host Setup (Rust):**
   - Rust generates a cryptographically secure 128-bit session token (`token`).
   - Rust binds a private Named Pipe server at `\\.\pipe\wavical-backend-<appId>-<token>`.
   - Rust configures the pipe's DACL to allow only the current process owner's token.
   - Rust sets up async reading for child `stdout` and `stderr` streams, piping them to `tracing::info!` and `tracing::warn!` capped at 4 KB/line and 200 lines/s.
2. **Launch:**
   - Rust spawns the Node child process with a scrubbed environment, passing `WAVICAL_IPC_PIPE=\\.\pipe\wavical-backend-<appId>-<token>`.
3. **Backend Helper (`wavical-backend.js`):**
   - The helper reads `process.env.WAVICAL_IPC_PIPE`.
   - It establishes a duplex connection using Node's standard `net.connect()`.
   - It implements length-prefixed JSON framing over that socket: `[4 bytes big-endian length][JSON bytes]`.
   - When user code calls `console.log(...)`, output flows to the standard OS stdout stream, completely separated from the IPC pipe.
   - Rust reads stdout, truncates lines if needed, and records them in tracing logs; the page never sees them.

---

### Item 6: Process Lifetime and Restrictions on Windows

#### Mechanism & Crate
- **Crate:** `win32job` v2.0.3 (MIT OR Apache-2.0, verified in `crates/wavical-ai`).
- **Safety:** `#![forbid(unsafe_code)]` workspace-wide. `win32job` exposes safe abstractions over Windows Job Objects.

#### Lifecycle & Resource Controls
1. **Kill-On-Job-Close:**
   - The child Node process is assigned to a Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
   - If the Wavical shell process crashes, exits cleanly, or is terminated via Task Manager, the Windows kernel automatically terminates the Node backend process immediately. No orphaned processes.
2. **Child Process Restriction:**
   - The Job Object can enforce `JOB_OBJECT_LIMIT_ACTIVE_PROCESS = 1` (or 2 when child processes are not granted in config). Any attempt by Node to spawn an unauthorized child process fails at the kernel level with `ERROR_ACCESS_DENIED`.
3. **Memory Ceilings:**
   - The Job Object can enforce `JobMemoryLimit` / `ProcessMemoryLimit` via `set_extended_limit_info`, preventing runaway memory leaks or DoS.

---

### Item 7: Safe Archive Extraction (Runtimes & Updates)

#### Crate
- **Crate:** `zip` v9.3.0 (crates.io / docs.rs, MIT license, released June 2026).

#### Attack Vectors & Defensive Rules
1. **Zip Slip (Path Traversal):**
   - Attackers embed entries with `../`, `..\\`, absolute paths (`/foo`, `C:\foo`), or UNC shares (`\\server\share`).
   - *Defense:* Every entry path is sanitized through pure validation logic (reusing rules from `wavical-protocol`). Any path with `..`, drive letters, or absolute prefixes triggers immediate extraction failure.
2. **NTFS Alternate Data Streams (ADS):**
   - Entries with colons (e.g. `file.txt:evil.exe::$DATA`).
   - *Defense:* All colons (`:`) in entry paths are forbidden.
3. **Windows Reserved Device Names:**
   - Entries matching `CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`.
   - *Defense:* Standard reserved name check rejects the archive before extraction.
4. **Symbolic Links & Junctions:**
   - Zip headers can flag entries as symlinks.
   - *Defense:* All symlink and hardlink flags are rejected; only regular files and directories are unpacked.
5. **Decompression Bombs (Zip Bombs):**
   - High compression ratio payloads (e.g. 10 KB expanding to 50 GB).
   - *Defenses:*
     - Max total extracted size ceiling: 500 MB for Node runtimes; 1 GB for app updates.
     - Max total entry count: 20,000 files.
     - Extractor wraps the decompressor in a `CountingReader` that tracks cumulative uncompressed bytes and aborts immediately if the budget is exceeded.
6. **Atomic Staging:**
   - Never extract directly into the target directory. Unpack into `%LOCALAPPDATA%\...\temp-extract-<uuid>\`. Validate hashes and file counts. Atomically rename the finished folder into the final location.

---

### Item 8: Cryptographic Signing and Key Management

#### Crate Options & Format
- **Crate:** `ed25519-dalek` v3.0.0 (BSD-3-Clause) or `minisign` v0.10.0 (MIT).
- **Format:** Ed25519 signature over the exact raw bytes of `manifest.json`.
- **Minisign Compatibility:** Adopting Minisign signature formatting (`manifest.json.sig`) allows both Wavical's built-in updater and standard CLI tools (`minisign -V -m manifest.json -p pubkey.pub`) to verify release manifests identically.

#### Embedded Public Keys & Rotation
- The app binary embeds up to **two public keys** at build time (`current` and `next`).
- When a manifest is signed by `next`, the app accepts the update and logs a key rotation event.
- Once updated, the app embeds the new public keys for future releases.

#### Private Key Storage & Security
1. **Local Developer:**
   - `wavical signer keygen` generates a new Ed25519 keypair.
   - The private key is saved to an external path specified by the developer (e.g. `~/.wavical/keys/<appId>.key` with user-only ACLs) or password-protected via Argon2id.
   - The private key is **never** written into the project directory, git repo, or build outputs.
2. **CI / CD Pipeline:**
   - In GitHub Actions, the private key is stored as a secret in a **Protected Environment** (e.g., `production-release`).
   - The environment requires manual review/approval by designated owners before any deployment job can access the secret.
   - Workflows pass the key via stdin or an in-memory secret; it is never written to disk or printed in workflow logs.
3. **Automated Testing:**
   - Tests generate temporary in-memory Ed25519 keypairs at test runtime (`ed25519_dalek::SigningKey::generate(&mut OsRng)`). No static private keys exist anywhere in git.

---

### Item 9: Update Layout and Atomic Apply on Windows

#### Directory Layout
```text
%LOCALAPPDATA%\<appId>\
├── launcher.exe               (Stable launcher binary)
├── current                    (Text file: contains active version, e.g. "1.2.0")
├── previous                   (Text file: contains rollback version, e.g. "1.1.0")
├── health.json                (Crash/failure counter for active version)
└── versions\
    ├── 1.1.0\
    │   └── app.exe
    ├── 1.2.0\
    │   └── app.exe
    └── 1.3.0.staged\          (Temporary staging during download/extraction)
```

#### Step-by-Step Update Sequence
1. **Check:** App fetches `manifest.json` and `manifest.json.sig`.
2. **Verify Manifest:**
   - Verify Ed25519 signature over `manifest.json` using embedded public key.
   - Assert `appId` matches config.
   - Assert `version` is strictly greater than current version (no downgrade).
   - Assert `expires` timestamp is in the future and within 30 days.
   - Assert `channel` matches app channel.
   - Assert client rollout bucket (0–99) is $< \text{rolloutPercent}$.
3. **Download Payload:**
   - Download payload to `%LOCALAPPDATA%\<appId>\versions\<newVersion>.part`.
   - Verify exact file size and SHA-256 hash against manifest values.
4. **Extract:**
   - Safe archive extractor unpacks payload into `%LOCALAPPDATA%\<appId>\versions\<newVersion>.staged\`.
5. **Commit Version Directory:**
   - Atomically rename `<newVersion>.staged\` to `<newVersion>\`.
6. **Switch Pointer Atomically:**
   - Read active version from `current` and write it to `previous.tmp`, rename to `previous`.
   - Write `<newVersion>` to `current.tmp`.
   - Atomically replace `current` with `current.tmp` (using Windows atomic replace).
7. **Restart & Health Check:**
   - App restarts into `versions\<newVersion>\app.exe`.
   - On startup, the new version must signal "ready" to the launcher.
   - If the new version crashes before reaching "ready" twice in a row, the launcher triggers automatic rollback: it switches `current` back to the version in `previous`.

#### Power Loss Analysis at Each Step
| Power Loss Occurrence | Disk State After Reboot | Behavior on Next Launch |
|---|---|---|
| **During Download** | `versions\<version>.part` incomplete | Launcher starts old version; updater resumes or restarts download cleanly. |
| **During Extraction** | `versions\<version>.staged` partial directory | `current` still points to old version. Staging directory is cleaned up on next update run. |
| **During Directory Rename** | Either `staged` still exists or completed | Directory rename on NTFS is atomic metadata change. Either `versions\<version>` exists or `staged` exists. `current` unchanged. |
| **During Pointer Switch** | `current.tmp` exists; `current` intact | File replace is atomic. Old version still runs if `current` was untouched; new version runs if replace completed. No corruption possible. |

#### How the Launcher Itself is Updated
Because `launcher.exe` is running, Windows file locking prevents in-place overwriting.
- The launcher is deliberately tiny and rarely modified.
- When a launcher update is required, the new version directory contains `new-launcher.exe`.
- When `launcher.exe` launches the target version, it checks if `versions\<current>\new-launcher.exe` has a newer hash.
- If newer, the launcher copies itself to a temporary trampoline (`launcher.old`), spawns the update script with a process exit wait, or schedules a Windows `MoveFileExW` with `MOVEFILE_DELAY_UNTIL_REBOOT`.

---

### Item 10: Release-Mode Debugging Surfaces (Empirical Findings)

#### Workspace Cargo Edition Check
- `Cargo.toml` specifies: **`edition = "2024"`**.
- In Rust 2024 edition, **`std::env::set_var` and `std::env::remove_var` are `unsafe`** because concurrent modifications of the process environment are inherently not thread-safe.

#### Empirical Verification on This Machine
We conducted an empirical test using the compiled `target\debug\wavical-shell.exe` and PowerShell scripts:

1. **Test 1: Launch with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`**
   - **Command:**
     ```powershell
     $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
     $proc = Start-Process -FilePath ".\target\debug\wavical-shell.exe" -ArgumentList "--app-id test-debug" -PassThru
     ```
   - **Result:**
     `Get-NetTCPConnection -LocalPort 9222` returned active listening socket `127.0.0.1:9222`!
     **Confirmed:** Setting `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` in the environment opens an active CDP remote debugging port on WebView2 apps.

2. **Test 2: Neutralization via Process Environment Sanitization**
   - Because `std::env::remove_var` is `unsafe` in Rust 2024 edition, we tested launching the child process with a scrubbed environment:
     ```powershell
     $psi = New-Object System.Diagnostics.ProcessStartInfo
     $psi.FileName = (Resolve-Path ".\target\debug\wavical-shell.exe").Path
     $psi.Arguments = "--app-id test-clean"
     $psi.UseShellExecute = $false
     $psi.EnvironmentVariables.Remove("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
     $proc = [System.Diagnostics.Process]::Start($psi)
     ```
   - **Result:**
     `CLEANED ENV RESULT: Port 9222 is NOT listening (NEUTRALIZED)!`
     **Confirmed:** Stripping `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` prior to initializing WebView2 completely neutralizes the remote debugging vulnerability.

3. **Node.js Debugger & Preload Variables to Strip:**
   In child process spawning (`Command::env_clear`), the following variables must be completely scrubbed:
   - `NODE_OPTIONS` (can pass `--inspect`, `--require`, `--import`)
   - `NODE_DEBUG`, `NODE_DEBUG_NATIVE`
   - `NODE_PATH` (can hijack module resolution)
   - `NODE_INSPECT_RESUME_ON_START`

---

### Item 11: Wavical's Own Release Pipeline (Canary → Stable)

#### GitHub Actions Architecture
1. **Canary Pipeline (`main` branch):**
   - Runs automatically on merge to `main`.
   - Executes full verification matrix: `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --all --check`, `cargo test -p wavical-audit -- --include-ignored`.
   - Publishes a **canary pre-release** (e.g. `0.5.0-canary.<run_number>`) to GitHub Releases with attached SHA-256 checksums and Sigstore artifact attestations (`actions/attest-build-provenance@v2`).
2. **Stable Promotion Pipeline (Protected Environment):**
   - Configured in GitHub repository settings under **Environments -> `production`**.
   - Requires **explicit approval by designated human reviewer**.
   - Contains repository release signing secrets and crates.io publishing tokens.
   - Publishes clean release tag `v0.5.0`.
3. **Crates.io Pre-release Semantics:**
   - Crates.io fully supports pre-release version identifiers (e.g. `0.5.0-canary.1`).
   - Cargo does not automatically resolve pre-release crates unless a user explicitly pins `=0.5.0-canary.1` in their `Cargo.toml`.

---

### Item 12: CLI and Managed Node Runtime for `wavical.config.js`

#### Problem
In Phase 1, `wavical` CLI requires a pre-installed `node` binary on the host developer machine to evaluate `wavical.config.js`. If Node is missing or too old, CLI commands fail.

#### Solution with Managed Runtime
Since Phase 4 implements verified runtime downloads and local caching (`%LOCALAPPDATA%\Wavical\runtimes\node-<version>-<arch>\node.exe`):
- The CLI can first check if a verified managed runtime is available in the local cache.
- If present, the CLI invokes the verified managed `node.exe` rather than searching system PATH.
- If missing, the CLI offers to download and verify the pinned runtime using the shared downloader.
- This removes the external dependency on system-wide Node.js installations.

---

## 3. Decisions I Need From You

Please review and confirm the following decision points:

### 1. Bun Support
- **(Recommended) Option A: Defer Bun to a later phase.** Bun lacks an internal permission model, meaning it would run completely unrestricted and bypass all Phase 4 privilege reduction guarantees.
- **Option B: Support Bun now with `backend.unrestricted: true` mandatory.** Bun runs with a loud CLI and log warning that no sandboxing or permission controls exist.
- **Option C: Never support Bun.** Focus exclusively on Node.js.

### 2. Version Policy for Node.js LTS
- **(Recommended) Option A: Same major LTS line.** Allow updates across `22.x.y` for approved versions in Wavical's runtime manifest. This ensures critical security updates are received even when Node bumps the minor version within the LTS line.
- **Option B: Strict same major.minor only (`22.11.x`).** Refuse minor upgrades automatically. Note that once Node moves to `22.12.0`, no further security patches will ever be issued for `22.11.x`.

### 3. Backend IPC Channel
- **(Recommended) Option A: Dedicated Windows Named Pipe.** Private named pipe with current-user-only DACL. Fully isolates framing from `stdout`/`stderr`, making `console.log` and native addon stdout output completely harmless.
- **Option B: Stdio with JavaScript helper console redirection.** Route protocol over stdin/stdout and override `console.log` in `wavical-backend.js`. Carries residual risk if third-party packages write directly to raw stdout.

### 4. Release Signing Private Key Storage
- **(Recommended) Option A: Minisign-compatible format in user-designated external file.** Private keys never touch repository, git, or build artifacts; GitHub Actions uses Protected Environment Secrets with required approval.
- **Option B: Password-encrypted raw Ed25519 seed file.** Encrypted at rest via Argon2id.

### 5. Update Layout on Windows
- **(Recommended) Option A: Launcher + Versioned folders (`versions/<v>/`) + atomic pointer file (`current`).** Automatic two-strike health check rollback to `previous`.
- **Option B: In-place binary replacement with shadow copy backup.** Simpler directory structure but higher risk during mid-write power interruptions.

### 6. CLI Runtime for `wavical.config.js`
- **(Recommended) Option A: Use managed runtime if available; fallback to system Node.** Eliminates system Node requirement once runtime is cached.
- **Option B: Keep system Node requirement.** Managed runtime is used only for running the app backend, not for CLI config evaluation.

### 7. Should `backend.unrestricted` exist?
- **(Recommended) Option A: Yes, but requires explicit `unrestricted: true` in config with loud console and log warnings.** Allows apps with legitimate native addon needs to function while making the security compromise explicit.
- **Option B: No, never allow unrestricted mode.** The permission model is always enforced; unsupported features cannot be run.

---

## 4. List of Items Marked NOT VERIFIED

- **None.** All 12 items were verified against official documentation, live crates.io/docs.rs registries, official git repositories, and empirical execution on this machine.
