# Wavical Optional Node.js Backend Guide

Wavical supports an optional local Node.js backend process (`backend.enabled: true` in `wavical.config.js`), mediated strictly through the Rust shell over a length-prefixed named-pipe IPC channel.

---

## 1. Permission Model (Enabled by Default)

By default, Wavical launches the verified Node.js runtime with the **Node.js Permission Model** enabled (`--permission` on Node >= 23, `--experimental-permission` on Node < 23):

- **Filesystem Read (`backend.permissions.fsRead`):** Only the backend directory (read-only) and explicitly configured paths are readable.
- **Filesystem Write (`backend.permissions.fsWrite`):** Only `%LOCALAPPDATA%\<appId>\backend-data` (and explicitly configured paths) are writable.
- **Child Processes (`backend.permissions.childProcess`):** Disabled by default (`false`). Enable only if your backend must spawn trusted subprocesses.
- **Worker Threads (`backend.permissions.workers`):** Disabled by default (`false`).
- **Unrestricted Escape Hatch (`backend.unrestricted: true`):** Disables the permission model and emits a loud startup warning. Avoid in production.

---

## 2. What Is NOT Restricted (Important Threat-Model Boundaries)

Even with the Node.js Permission Model enabled, the following are **NOT** restricted by Node's permission flags:

1. **Outbound & Localhost Network Access:** Node's `--permission` model does **not** block `http`, `https`, `net`, `tls`, `dns`, or `fetch()` calls. A compromised backend script or malicious npm dependency can make arbitrary outbound network requests or connect to localhost services.
   - **Mitigation:** Keep untrusted dependencies out of `backend/package.json`, audit dependencies regularly, and never pass sensitive host credentials into `backend.env`.
2. **Native Addons (`*.node`) / FFI:** If `dlopen`/native addons are loaded, OS-level syscalls inside native C/C++ code bypass JS-level permission checks.
3. **CPU / Memory Exhaustion inside a Single Request:** While IPC enforces frame size (1 MB request / 4 MB response), concurrency (16 in-flight), and per-call timeouts (default 30s, max 300s), an infinite loop in JS can consume CPU until the process is terminated.

---

## 3. Environment & Debug Hardening

- **Cleaned Environment:** The backend spawns with `Command::env_clear()`, receiving only minimal OS variables (`SystemRoot`/`TEMP`/`PATH`), `WAVICAL_IPC_PIPE`, `WAVICAL_APP_ID`, and explicitly allowlisted `backend.env` variables.
- **Forbidden Variables & Flags:** `NODE_OPTIONS`, `NODE_PATH`, `NODE_DEBUG*`, `NODE_INSPECT*`, `--inspect*`, `--require`, and `--import` are stripped from the environment and **refused** in release builds.

---

## 4. Supply-Chain & Install-Script Advice

- **Lockfile Required:** `wavical build` refuses to build a backend without `backend/package-lock.json` and executes `npm ci --ignore-scripts` (never `npm install`).
- **Skip Install Scripts:** Lifecycle scripts (`preinstall`, `install`, `postinstall`) are blocked by default, and `wavical build` prints the list of packages whose install scripts were skipped. Only pass `--allow-install-scripts` if you have audited every package in the lockfile.
- **Build Exclusions:** `.env*`, `.git`, test directories, and source maps (unless `--include-source-maps`) are excluded from builds.
- **Bundled Runtime (`--bundle-runtime`):** Copies the SHA-256-verified Node runtime and its `LICENSE` into `dist/` so built apps run offline without relying on the shared user cache.

