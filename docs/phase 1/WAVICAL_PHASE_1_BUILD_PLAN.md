# Wavical — Phase 1 Build Plan: Core CLI & Config

**Goal:** build a developer-friendly CLI with a single config file that wires the Phase 0 capability model and sets up the foundation for Phases 2–6.

**Scope:** `wavical init`, `wavical dev`, `wavical build` commands; single flat `wavical.config.js`; config schema validation; capability declarations in config.

**Estimate:** 2–3 weeks part-time with AI. Most of the time goes into CLI ergonomics and error messages, not code generation.

---

## 0. Decisions locked before code

### Config file format: JavaScript, not TOML or JSON

**Why JavaScript?** Wavical positions itself as "Rust-free backend by default." A `.js` config file is:
- Already familiar to web devs (no new DSL)
- Executable (can do computed values: `appId: process.env.APP_ID || 'default'`)
- Easier for tooling to parse and validate (typed via Zod, JSDoc, TypeScript)
- Matches the pattern Vite, Nuxt, and other modern dev tools use

**Trade-off:** requires Node to *read* the config during `wavical dev` and `wavical build`, but does NOT require the app to use a Node backend. The CLI shells out to Node purely for config parsing; the app can be pure Rust/WebView2.

**Not settled yet:**
- Will the CLI itself be written in Rust (spawn Node as a subprocess), or will Phase 1 include a thin Node/Bun entry point that calls Rust binaries?
- Decision point: end of WP1 (architecture choice).

### Capability model integration

Phase 0 shipped with a hardcoded capability set per window. Phase 1 moves that into the config file:

```javascript
// wavical.config.js
export default {
  appId: 'my-app',
  capabilities: {
    'core.ping': true,      // explicitly granted
    'core.echo': false,      // explicitly denied (or omitted)
    // Phase 2 will add native APIs here
  },
}
```

The capability list is **declarative, not runtime-evaluated**. The CLI validates it at startup and rejects unknown capabilities.

---

## 1. Scope

**In scope**
- Three CLI commands: `wavical init`, `wavical dev`, `wavical build`
- `wavical.config.js` parsing and schema validation
- Capability declarations and validation against the Phase 0 registry
- Dev-friendly error messages (structured logging, no Rust panic traces)
- Zod schema for config (or equivalent TypeScript schema, if using a TS config)
- Integration with Phase 0's capability model (CLI validates declared capabilities exist)
- Second binary name: `wca` (symlink, hard link, or shell wrapper — TBD)

**Out of scope**
- Actual Node/Bun backend runtime (Phase 4)
- File dialogs, clipboard, notifications, tray, etc. (Phase 2)
- AI model integration (Phase 3)
- Installers, code signing, updaters (Phases 4–5)
- Enterprise audit logging (Phase 6)
- Template marketplace, marketplace auth, private registries (ecosystem, not phase-locked)

**Not required in Phase 1, but test framework structure**
- `wavical init --ai`: AI-assisted scaffolding can come in Phase 3 if it fits the schedule; skip for now
- JSON schema export for VS Code IntelliSense: can come as a separate tooling task post-Phase-1
- `--bundle-runtime` flag: Phase 4
- Devtools toggle, remote-debug disable: Phase 4 (backend process hardening)

---

## 2. Tech choices

| Area | Choice | Notes |
|---|---|---|
| CLI framework | `clap` | Structured argument parsing; generates help automatically |
| Config format | JavaScript (`.js`) | Executable, familiar to web devs, easy to validate with Zod |
| Config validation | Zod (or TypeScript + Zod) | Runtime-safe schema, clear error messages |
| Logging | `tracing` + `tracing-subscriber` | Structured logs; consistent with Phase 0 |
| Node interaction | `std::process::Command` | Spawn Node as subprocess to parse config |
| Error messages | Custom `thiserror` enums | Friendly, actionable errors; no raw Rust traces |
| Second binary | `wca` (TBD: symlink vs. wrapper) | Same CLI behavior under either name |

---

## 3. Repo layout (Phase 1 changes)

```
wavical/
├─ Cargo.toml                (workspace)
├─ crates/
│  ├─ wavical-ipc/              (Phase 0, unchanged)
│  ├─ wavical-protocol/         (Phase 0, unchanged)
│  ├─ wavical-shell/            (Phase 0, unchanged)
│  ├─ wavical-cli/              (NEW: CLI binary, Phase 1)
│  └─ wavical-config/           (NEW: config types & validation)
├─ examples/hello/           (enhanced with manifest, config)
├─ tests/
│  ├─ attack-page/           (Phase 0, unchanged)
│  ├─ integration/            (Phase 0, unchanged)
│  └─ cli/                     (NEW: CLI tests)
├─ fuzz/                     (Phase 0, unchanged)
├─ .github/workflows/ci.yml  (Phase 0, unchanged; will add Node to runner)
├─ ROADMAP.md               (Phase 0, unchanged)
├─ SECURITY.md              (Phase 0, unchanged)
└─ docs/
   ├─ WAVICAL_PHASE_0_BUILD_PLAN.md
   ├─ WAVICAL_PHASE_1_BUILD_PLAN.md (this file)
   ├─ WAVICAL_CONFIG_SCHEMA.md      (NEW: config file reference)
   └─ ...
```

**New crates:**
- `wavical-cli`: the `wavical` and `wca` binaries
- `wavical-config`: shared config types, Zod-style schema, validation logic

---

## 4. Work packages (do in order, one per session)

Each package has an acceptance test. Don't start the next until it passes.

### WP1: Config schema and validation

**Goal:** define the config file format, build a validator, wire it to Phase 0's capability model.

**Tasks:**
1. Create `wavical-config` crate (pure Rust, no `tao`/`wry`).
2. Define config types:
   - `AppConfig`: top-level struct
   - `Capabilities`: a map of capability name → bool (granted/denied)
   - Required fields: `appId` (string)
   - Optional fields: `window` (size, title, etc.), `capabilities`, `dev.port` (dev server port)
3. Create Zod-style validation: a `validate_config` function that parses a `.js` file via Node subprocess and returns `AppConfig` or a user-friendly error.
4. Validate capability names: check them against `wavical-ipc`'s registry (import the types). Reject unknown capabilities.
5. Config parsing flow:
   - Read `wavical.config.js` from the project root
   - Spawn `node -e "console.log(JSON.stringify(require('./wavical.config.js')))"` (or use dynamic import)
   - Parse the JSON output as `AppConfig`
   - Return errors in a human-readable format (list the unknown capabilities, etc.)
6. Tests: unit tests for validation (valid config, missing required fields, unknown capabilities, malformed JSON from Node). Property tests for config structs.

**Acceptance:**
- Config validator works, rejects invalid configs with clear messages
- `wavical-ipc` types are imported and used to validate capabilities
- Manual test: create a test `wavical.config.js` with a typo in a capability name; the validator rejects it with a helpful message

---

### WP2: CLI skeleton and `wavical init`

**Goal:** build the CLI entry point and the first command: `wavical init <dir>`.

**Tasks in `wavical-cli` crate:**
1. Set up `clap` for argument parsing. Three subcommands:
   - `wavical init <dir>` — initialize a new Wavical app
   - `wavical dev` — start dev server (stub for now, will be expanded in WP3)
   - `wavical build` — build for production (stub)
2. Implement `wavical init <dir>`:
   - Create the directory if it doesn't exist.
   - Scaffold a minimal app:
     - `wavical.config.js` with sensible defaults (appId = dir name, default window size, `capabilities: {}`)
     - `src/index.html` with a basic page
     - `src/styles.css` with boilerplate styles
     - `.gitignore`
   - Print a friendly message: "App created at `<dir>`. Run `wavical dev` to start."
3. Error handling: if the directory already exists and is not empty, ask for confirmation (or use `--force`).
4. The second binary name `wca`:
   - Figure out how to ship both `wavical` and `wca` as the same binary (symlink on Unix, hardlink on Windows, or a shell wrapper).
   - For now, assume it's a symlink. Document the approach.
5. Tests: create a temp directory, run `wavical init test-app`, verify the scaffolded files exist and have the right content.

**Acceptance:**
- `wavical init my-app` creates the directory and scaffolds the project
- `wca init my-app` works identically (alias functions correctly)
- The scaffolded `wavical.config.js` is valid (passes WP1's validator)
- All tests pass

---

### WP3: `wavical dev` and dev server

**Goal:** implement the dev command that starts the Phase 0 shell and watches for config/asset changes.

**Tasks:**
1. `wavical dev` (no args):
   - Parse `wavical.config.js` (uses WP1's validator).
   - Load the `src/` directory as assets.
   - Launch `wavical-shell` with the parsed config:
     - Pass appId, window title, initial window size, capabilities to the shell via command-line args or env vars (decision: which is cleaner?).
   - Bind the shell's custom protocol handler to serve files from `src/`.
   - Inject the capability set into the webview (the bridge should deny anything not in `capabilities`).
   - Print the PID and a message: "Dev server running. Press Ctrl+C to stop."
2. Filesystem watching:
   - Watch `src/` directory for changes (new files, edits, deletions).
   - On change, trigger a hot-reload in the webview (send a message to the page, or inject a script that polls, TBD).
   - Watch `wavical.config.js`:
     - On change, reload the config and update the capability set in the bridge.
     - On validation error, print a warning but keep the server running (friendly DX).
3. Error handling: if config is invalid, print a clear error and exit with code 1. If `src/` doesn't exist, create it or warn the user.
4. Tests: spawn `wavical dev` in a temp project, verify it starts and the page loads. Modify a file in `src/` and verify the change is reflected in the running shell.

**Acceptance:**
- `wavical dev` starts the shell and serves the app from `src/`
- Config changes are detected and applied without restarting
- Asset changes are reflected in the browser (hot-reload works)
- Ctrl+C exits cleanly with code 0
- All tests pass

---

### WP4: `wavical build` (static)

**Goal:** implement the build command for static deployment (no Node runtime in this phase).

**Tasks:**
1. `wavical build` (no args):
   - Parse `wavical.config.js`.
   - Copy `src/` files to `dist/`.
   - Generate a manifest file (`dist/wavical-manifest.json`) with:
     - `appId`, `version` (from config or package.json, TBD)
     - Capability list (for validation on app startup)
     - Asset hashes (for integrity checks)
   - Print: "Build complete. App is in `dist/`. To distribute, zip and ship."
2. Optional flags:
   - `--outdir` (default `dist/`)
   - `--release` or similar: compile `wavical-shell` in release mode (but doesn't affect assets)
3. Validation:
   - Reject if `src/` doesn't exist.
   - Warn if `src/index.html` is missing.
4. Tests: run build in a temp project, verify `dist/` contains the right files and `wavical-manifest.json` is valid JSON.

**Acceptance:**
- `wavical build` creates a `dist/` directory with the built app
- The manifest is valid JSON and contains the expected fields
- Subsequent `wavical build` overwrites `dist/` correctly
- `--outdir` flag works

---

### WP5: Integration and self-review

**Goal:** test the full CLI workflow and review for usability and security.

**Tasks:**
1. End-to-end workflow test:
   - `wavical init test-project`
   - `cd test-project && wavical dev` (run for 2 seconds, then kill)
   - Verify the shell started and served the page
   - `wavical build`
   - Verify `dist/` is valid
2. Test error cases:
   - Run `wavical init` in an existing non-empty directory (should ask for confirmation)
   - Run `wavical dev` with a malformed config (should print a clear error)
   - Run `wavical build` without a `src/` directory (should warn or error)
   - Run `wavical build --outdir /invalid/path` (should error)
3. Usability self-review:
   - Are error messages clear and actionable? (E.g., "unknown capability 'core.echo' in config; did you mean 'core.ping'?" or "missing required field 'appId'")
   - Does the help text (`wavical --help`, `wavical init --help`) clearly explain what each command does?
   - Is the scaffolded project immediately runnable (`wavical dev` works without edits)?
4. Security self-review:
   - Config parsing: does Node code run only in a subprocess, not in the Rust CLI process? (Defense in depth against config injection.)
   - Capability validation: is it impossible for a capability not in the declared set to be called?
   - Asset paths: are file paths from the config and asset directory validated to prevent directory traversal?
5. Wire it into CI: update `.github/workflows/ci.yml` to:
   - Run CLI tests in the Linux job
   - Add a job that runs `wavical init test-app && cd test-app && wavical dev` (backgrounded, with a timeout)

**Acceptance:**
- E2E workflow passes (init → dev → build)
- Error cases are handled with clear messages
- Self-review list is documented (with any issues and fixes)
- CI tests the full CLI flow
- All commands run without panics; errors are returned as typed enums, not unwrap/expect

---

## 5. Security & architecture decisions

### Config file execution

The config is a `.js` file that's executed via Node. This is a trust boundary:

- **Threat:** config injection (a malicious config file could run arbitrary code)
- **Defense:** the config is trusted (the developer writes it), but Node is untrusted. Spawn Node in a subprocess with a restricted environment:
  - No network access (or whitelist specific hosts only)
  - No filesystem access outside the project (or whitelist the project directory)
  - Timeout: if config parsing takes >5 seconds, kill the process
  - Capture only stdout; ignore stderr (to avoid side effects)

**Implementation note:** `std::process::Command` with a timeout via `std::time::timeout` or a similar mechanism.

### Capability validation at startup, not runtime

Phase 0 validated capabilities in the bridge (good). Phase 1 adds a second layer: the CLI validates that declared capabilities exist before launching the shell. This is defense in depth.

### Error handling: no Rust panics

Every error path returns a typed enum that's converted to a user-friendly message. Panics are caught and logged, not surfaced.

---

## 6. After WP5: shipping readiness

Once all work packages pass:

- [ ] Read the diff. Ask the agent to explain anything you don't understand.
- [ ] Run `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` yourself.
- [ ] Search the code for `unwrap(`, `expect(`, `unsafe`, `TODO`. Each one needs a reason.
- [ ] Check no new dependencies were added without discussion.
- [ ] Commit with a clear message, e.g., `Phase 1: CLI & Config (WP1-WP5)`.
- [ ] Optionally push to GitHub and verify CI is green.
- [ ] Only then start Phase 2 in a new session.

---

## 7. CLI error messages: examples

To guide the AI's error handling, here are target error messages:

```
$ wavical init
error: missing required argument <DIR>
  Usage: wavical init <DIR>

$ wavical init my-app
✓ Created app at ./my-app
  Run: cd my-app && wavical dev

$ cd my-app && wavical dev
error: invalid config: unknown capability 'core.echo2' in capabilities
  Known capabilities: core.ping
  Did you mean 'core.ping'?

$ wavical build --outdir /root
error: permission denied: cannot write to /root
  Check the path and try again, or use a subdirectory of the current project.

$ wavical dev
(config changes detected)
warning: config changed, reloading...
✓ Config reloaded. Capability set updated.

(asset changes detected)
info: src/index.html changed, reloading...
✓ Reloaded in 243ms
```

---

## 8. Not needed yet

- Node/Bun in the Phase 1 crate (Phase 4)
- Advanced CLI features: templates, plugins, telemetry
- JSON schema export for IntelliSense (can come as a separate task)
- Signed installers or code signing (Phase 5)
- Enterprise audit logging (Phase 6)
