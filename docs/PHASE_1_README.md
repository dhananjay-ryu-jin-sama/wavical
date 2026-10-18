# Wavical Phase 1 — Overview & Getting Started

**Phase 1 builds the developer experience:** a CLI with a JavaScript config file that lets developers initialize, develop, and build Wavical apps.

## What's included in this release

- **WAVICAL_PHASE_1_BUILD_PLAN.md** — Full technical plan with scope, tech choices, and work package descriptions
- **WAVICAL_PHASE_1_ANTIGRAVITY_PROMPTS.md** — AI-ready prompts for running each work package (WP1–WP5)
- **WAVICAL_PHASE_1_CONFIG_SCHEMA.md** — Reference documentation for the `wavical.config.js` config file format

## Quick start for AI agents

1. **Read the Master Brief** in `WAVICAL_PHASE_1_ANTIGRAVITY_PROMPTS.md` (copy it once per session)
2. **Pick a work package** (WP1, WP2, WP3, WP4, or WP5)
3. **Follow the WPx prompt** — each one is self-contained and has clear acceptance criteria
4. **One WP per session** — don't start the next one until this one passes acceptance
5. **Phase 0 must be complete** before starting Phase 1 (git repo initialized, CI green)

## Work package breakdown

| WP | Title | Focus | Est. Time |
|---|---|---|---|
| WP1 | Config schema & validation | Define AppConfig, validate via Node subprocess, reject unknown capabilities | 1 session |
| WP2 | CLI skeleton & `wavical init` | Scaffolding; clap parsing; directory creation; `wca` alias setup | 1 session |
| WP3 | `wavical dev` & dev server | Launch shell; filesystem watching; hot-reload on config/asset changes | 1 session |
| WP4 | `wavical build` | Copy assets; generate manifest with SHA256 hashes; `--outdir` flag | 1 session |
| WP5 | Integration & self-review | E2E testing; error case handling; security review; CI integration | 1 session |

**Total estimate:** 5 sessions (2–3 weeks part-time)

## Key decisions locked in Phase 1

1. **Config format is JavaScript**, not TOML or JSON
   - Executable (supports computed values via environment variables)
   - Familiar to web devs
   - Parsed via Node subprocess in the CLI

2. **Config is declarative**
   - Lists capabilities your app needs (not runtime-evaluated)
   - CLI validates declared capabilities at startup
   - Bridge enforces them (defense in depth)

3. **No Node backend in Phase 1**
   - Developers can build pure-Rust apps
   - Node/Bun runtime is opt-in in Phase 4
   - The CLI uses Node only to parse the config file

4. **Second binary name is `wca`**
   - Short alias for `wavical`
   - Same behavior under either name
   - Implemented as symlink, hardlink, or wrapper (TBD in WP2)

5. **Error messages are friendly**
   - No Rust panics or backtraces
   - Actionable suggestions ("did you mean 'core.ping'?")
   - Structured using `thiserror` enums

## Architecture: CLI → Config → Shell

```
User runs: wavical dev
    ↓
CLI (wavical-cli crate):
  1. Parse wavical.config.js (spawn Node, validate via WP1 logic)
  2. Launch wavical-shell with config values
  3. Watch src/ and wavical.config.js for changes
  4. On change: reload assets or revalidate config
    ↓
wavical-shell (Phase 0):
  1. Create window, load webview
  2. Serve files from src/ via wavical:// protocol
  3. Enforce capabilities from config in IPC bridge
  4. Deny undeclared methods
```

## Config file format (quick reference)

```javascript
export default {
  appId: 'my-app',                    // Required
  window: {
    width: 1200,                      // Optional; default 1200
    height: 800,                      // Optional; default 800
    title: 'My App',                  // Optional; default: appId
  },
  capabilities: {
    'core.ping': true,                // Grant core.ping
    'core.echo': false,               // Deny core.echo (or omit)
  },
  dev: {
    port: 7890,                       // Optional; default 7890
  },
}
```

See `WAVICAL_PHASE_1_CONFIG_SCHEMA.md` for full details, examples, and validation rules.

## Acceptance criteria summary

**Phase 1 is done when:**
- `wavical init my-app` creates a scaffolded project with a valid config
- `wavical dev` launches the shell and serves the app from src/
- Config/asset changes are detected and reloaded without restarting
- `wavical build` creates dist/ with a valid manifest
- Error messages are clear and actionable
- All tests pass
- CI runs the full CLI flow (init → dev → build)

## Relationship to Phase 0

Phase 0 (foundation shell) is complete. Phase 1 wraps it in a developer-friendly CLI and config system.

**Phase 0 provides:**
- Window creation (tao + wry)
- Custom protocol handler (wavical://)
- IPC bridge with default-deny capability model
- Security test bench

**Phase 1 adds:**
- Developer CLI (wavical, wca)
- JavaScript config file (wavical.config.js)
- Project scaffolding (`wavical init`)
- Dev server with hot-reload (`wavical dev`)
- Build command (`wavical build`)
- Capability validation against declared set

## What comes next (Phase 2 onwards)

- **Phase 2:** Native Tier 1 APIs (file dialogs, clipboard, notifications, etc.)
- **Phase 3:** AI-native infrastructure (local models, RAG)
- **Phase 4:** Optional Node/Bun backend + auto-updater
- **Phase 5:** Installers and packaging
- **Phase 6:** Enterprise audit logging

## Notes for the AI agent building Phase 1

1. **Dependency management:** Use `cargo add` to fetch current crate versions; don't hardcode from memory.
2. **API documentation:** Before writing code, read the docs for `clap`, `thiserror`, `serde`, and `notify` (or whatever filesystem watcher is chosen).
3. **Config parsing:** Always spawn Node in a subprocess, never call it from within Rust. Set a 5-second timeout.
4. **Error handling:** No `unwrap()` or `expect()` on user input, config, or Node output. Use `thiserror` enums.
5. **Testing:** Write tests as you go. Every validation rule needs a test that fails if the rule is removed.
6. **CI integration:** By WP5, the CI should run `wavical init` and `wavical build` in a temp project to verify the full flow works.

## Phase 1 prompts overview

- **Master Brief:** Read first, establishes ground rules and hard requirements
- **WP1 prompt:** Config schema, validation, capability checking
- **WP2 prompt:** CLI argument parsing, project scaffolding, init command
- **WP3 prompt:** Dev server, filesystem watching, hot-reload
- **WP4 prompt:** Build command, manifest generation, asset copying
- **WP5 prompt:** E2E testing, error cases, security review, CI integration

Each prompt is self-contained. When the AI finishes a work package, it should:
1. List what was built
2. Show real command output (not summaries)
3. Document any issues or simplifications
4. Stop and wait for approval before starting the next WP

---

## Questions?

Refer to the corresponding sections in:
- `WAVICAL_PHASE_1_BUILD_PLAN.md` for technical details
- `WAVICAL_PHASE_1_CONFIG_SCHEMA.md` for config format
- Phase 0 docs for context on the shell and security model
