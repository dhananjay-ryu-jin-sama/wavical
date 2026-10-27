# Wavical — Phase 0 Build Plan: Foundation Shell

**Goal:** a minimal, security-first Windows shell that opens a native window, loads a local page through a custom protocol, and talks to Rust through a locked-down IPC bridge. Nothing else.

**Why it's small on purpose:** every later phase sits on top of this. It is also the code that matters most for security, so it has to be small enough to review properly.

> Estimate: roughly 1–2 weeks part-time with AI-assisted coding. Most of the time goes into reviewing and testing, not generating. Treat that as a guess, not a promise.

---
---


## 0. Decision to lock before writing code (correction to earlier advice)

Earlier I said the `event_loop.run(move |event, _, control_flow| ...)` closure pattern was deprecated and that you should use `ApplicationHandler`. **That was only half right.**

- `ApplicationHandler` / `run_app` is the **winit** API. winit deprecated the closure style.
- **tao** (the fork wry recommends, and the one Tauri uses) still uses the closure style, and wry's own tao example uses it.

**Decision for Wavical:** use **tao + wry**. wry's docs recommend tao for maximum compatibility. Use tao's closure-style event loop and don't mix in winit's `ApplicationHandler`. Before the AI writes any code, have it read the current `docs.rs/tao` and `docs.rs/wry` examples so it doesn't mix the two APIs.

---

> **Naming:** project is Wavical, full command `wavical`, short alias `wca`. Phase 0 only needs the crate names (`wavical-*`) and the `wavical://` protocol to be right. The alias is built in Phase 1 (as a second binary or an npm `bin` entry pointing at the same code).

---

## 1. Scope

**In scope**
- Cargo workspace and repo skeleton
- Window + webview creation (tao + wry), Windows only
- Custom `wavical://` protocol serving local assets
- IPC bridge with a default-deny capability model
- All P0 security items (listed in section 5)
- Two demo methods: `core.ping` (granted) and `core.echo` (not granted, used to prove default-deny)
- Security test bench v1 (unit, property tests, fuzzing, integration "attack page")
- CI pipeline

**Out of scope (do not let the AI wander into these)**
- File dialogs, clipboard, tray, notifications (Phase 2)
- CLI and config file (Phase 1), including the `wca` alias for `wavical`. Phase 0 uses a small hardcoded Rust config struct.
- Node/Bun, AI runtime, updater, installers (later phases)
- macOS and Linux builds of the shell (the `wavical-ipc` crate must still compile and test on Linux, see below)

---

## 2. Tech choices

| Area | Choice | Notes |
|---|---|---|
| Windowing | `tao` | Closure-style event loop |
| Webview | `wry` | Latest stable. Check docs.rs, don't pin from this doc |
| Serialization | `serde`, `serde_json` | Typed structs/enums with `#[serde(deny_unknown_fields)]`. Typed parsing is stronger than loose schema validation |
| Errors | `thiserror` | No `unwrap()` on anything that touches external input |
| Logging | `tracing` | Needed now, because the audit logger in Phase 6 builds on it |
| Tests | built-in tests, `proptest`, `cargo-fuzz` | cargo-fuzz works best on Linux, so use WSL or your Linux VM |
| Supply chain | `cargo audit`, `cargo deny` | Run in CI from day one |
| Safety rule | `#![forbid(unsafe_code)]` in `wavical-ipc` and `wavical-core` | Any `unsafe` has to be an explicit, reviewed exception |

---

## 3. Repo layout

```
wavical/
├─ Cargo.toml                (workspace)
├─ crates/
│  ├─ wavical-ipc/              (pure Rust: message types, parsing, capability checks. NO wry/tao dependency)
│  ├─ wavical-protocol/         (pure Rust: path resolution/sanitization for the wavical:// handler)
│  └─ wavical-shell/            (tao + wry glue: window, webview, handlers)
├─ examples/hello/           (tiny HTML/JS app used for manual testing)
├─ tests/
│  ├─ attack-page/           (adversarial HTML/JS used by integration tests)
│  └─ integration/
├─ fuzz/                     (cargo-fuzz targets)
├─ .github/workflows/ci.yml
├─ ROADMAP.md
└─ SECURITY.md
```

**Key design rule:** keep `wavical-ipc` and `wavical-protocol` free of any windowing or webview dependency. That makes them testable and fuzzable on Linux and fast in CI, and it keeps the security-critical logic small and separate from the glue code. (Dev-only test exception F-A0-08: standard dev-dependencies `proptest` and `tempfile` pull `windows-sys` transitively on Windows dev builds; production builds `-e normal` strictly contain zero prohibited/OS dependencies).

---

## 4. Work packages (do in this order, one per AI session)

Each package has an acceptance test. Don't start the next one until it passes.

### WP1: Skeleton and CI
- Create the workspace, empty crates, `ROADMAP.md`, `SECURITY.md`
- CI: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo audit`, `cargo deny`
- Windows runner builds the workspace, Linux runner tests the pure crates
- **Done when:** CI is green on an empty project

### WP2: Window and webview
- tao event loop, one window, wry webview loading a hardcoded HTML string
- Handle "WebView2 runtime missing" with a clear error message, not a panic
- **Done when:** `cargo run` opens a window showing text, and closing the window exits cleanly

### WP3: Custom protocol and path sanitization (`wavical-protocol`)
- Serve files from a configured asset directory via `wavical://`
- Note: on Windows, wry maps custom protocols to a `http://<scheme>.localhost` style URL. Verify this against current wry docs, since it affects your CSP and navigation allowlist
- Resolution rules: decode first, reject `..`, absolute paths, drive letters, UNC paths (`\\server\share`), alternate data streams (`file.txt:stream`), reserved device names (`CON`, `NUL`, ...), and any resolved path that escapes the asset root after canonicalization (this catches symlinks and junctions)
- Return a safe 404 for anything rejected. Do not leak the real path in the error
- Add MIME types and `X-Content-Type-Options: nosniff`
- **Done when:** the path traversal test suite (section 6) passes with zero escapes

### WP4: IPC core (`wavical-ipc`)
- Message types: request `{ id, method, params }` and response `{ id, ok, result | error }`
- Enforce a maximum message size **before** parsing (start at 1 MB), plus a JSON nesting depth limit
- Parse into typed structs with `deny_unknown_fields`. Unknown method returns a generic error
- Capability check: each window has an explicit allowlist of method names. Anything not on the list is rejected. Default is an empty list
- Return generic errors to the page. Log the detail with `tracing`, never send internals to the webview
- **Done when:** unit tests plus 10k+ proptest cases pass, and no input can panic the parser

### WP5: Bridge in the shell (`wavical-shell`)
- Inject an initialization script that defines a frozen `window.wavical` with a single `invoke(method, params)` returning a Promise (match responses by `id`)
- Wire wry's IPC handler to `wavical-ipc`. Send responses back with `evaluate_script`
- **Check the origin of every IPC message** (the request URI) against the allowlist. This blocks messages from iframes or unexpected pages
- Remember: page JS can bypass your `window.wavical` wrapper and call `window.ipc.postMessage` directly. The frozen object is a convenience, **all enforcement must happen in Rust**
- Implement `core.ping` (granted) and `core.echo` (left ungranted in the test config)
- **Done when:** a page can call `wavical.invoke("core.ping")` and gets a reply, while `core.echo` is rejected

### WP6: Navigation, popups, CSP, devtools
- Navigation handler: allow only the app origin, deny everything else
- Block new-window/popup requests by default
- Add a `Content-Security-Policy` header to every protocol response. Start strict (`default-src 'self'`) and loosen only with a reason
- Devtools: enabled in debug builds only, off in release
- **Done when:** the attack page cannot navigate away, open popups, or load a remote script

### WP7: Secrets rule and logging
- No method in the bridge may return secrets. Add a test that lists every exposed method so any new one has to be a deliberate change
- `tracing` setup with structured events for: capability denied, malformed message, oversize message, origin rejected, path rejected
- **Done when:** each of those events shows up in logs during the attack tests

### WP8: Security test bench v1 and review
- Section 6 below, wired into CI
- Run `cargo clippy` at the strictest practical level and `cargo audit`
- Send `wavical-ipc`, `wavical-protocol` and the IPC handler in `wavical-shell` for **human expert review** before calling Phase 0 done

---

## 5. P0 security items mapped to work packages

| P0 item | Where it's implemented |
|---|---|
| Default-deny capability model | WP4 |
| Frozen minimal bridge object | WP5 (plus Rust-side enforcement) |
| Schema/type validation on every IPC message | WP4 |
| Path sanitization in custom protocol | WP3 |
| Default CSP | WP6 |
| Navigation allowlisting | WP6 |
| Secrets never reachable from webview | WP7 |
| Runtime download hash verification | Phase 4 (not in Phase 0) |

---

## 6. Security test bench v1

**Unit / property tests (Linux and Windows CI)**
- Path traversal: `../`, `..\`, `%2e%2e%2f`, double-encoding, mixed slashes, absolute paths, `C:\`, UNC paths, `file.txt::$DATA`, `CON`/`NUL`, trailing dots and spaces, very long paths, symlink/junction escape
- IPC parser: empty message, invalid JSON, huge message, deeply nested JSON, wrong types, missing fields, extra fields, duplicate keys, huge `id`, unicode edge cases, null bytes
- Capability: method not granted, method granted to another window, case-variant method names, method names with odd characters

**Fuzzing (Linux/WSL)**
- Fuzz target for the IPC message parser and capability check
- Fuzz target for the path resolver
- Run for a fixed time in CI, store any crash inputs as permanent regression tests

**Integration "attack page" (Windows)**
The harness opens the app, loads `tests/attack-page`, runs attacks via `evaluate_script_with_callback` and asserts the results. The page tries:
- calling `core.echo` without the capability
- calling `window.ipc.postMessage` directly with malformed and oversized payloads
- messaging from an iframe on another origin
- navigating `top` to an external URL and opening popups
- fetching `wavical://` URLs with traversal tricks
- loading a remote script (CSP should block it)
- overwriting or deleting `window.wavical` and checking it still can't gain extra access

**Rule:** every bug found later becomes a permanent test here.

---

## 7. Definition of Done for Phase 0

- [ ] All 8 work packages' acceptance checks pass
- [ ] CI green on Windows and Linux, including audit and deny
- [ ] Attack page integration tests pass, fuzzers run clean for the agreed duration
- [ ] No `unsafe` in `wavical-ipc` / `wavical-core` / `wavical-protocol`, no `unwrap()` on external input
- [ ] Expert review of the three security-critical areas done and findings fixed
- [ ] `SECURITY.md` explains how to report a vulnerability
- [ ] `ROADMAP.md` published
- [ ] Hello example runs on a clean Windows machine (no dev tools installed)

---

## 8. Skills required

**What you personally need (to direct the AI and judge its output)**

| Skill | Level needed | Why |
|---|---|---|
| Rust reading ability: ownership, enums, `Result`, traits, cargo workspaces | Intermediate (read and question, not write from scratch) | You must be able to tell when generated code looks wrong or hides a shortcut |
| Basic threat modeling | Beginner+ | To know what "the page is untrusted" means and to write good attack cases |
| Web basics: Promises, `postMessage`, iframes, same-origin policy, CSP | Intermediate | The bridge and attack page are web code |
| Git and GitHub Actions | Beginner+ | Repo, CI, branch hygiene |
| Windows dev setup: MSVC build tools, WebView2 runtime | Beginner | Needed to build and run |
| Running and reading test output (`cargo test`, `clippy`, fuzzers) | Beginner+ | This is your main quality check |

**What the AI tools cover**
- Writing the Rust code, tests, CI config and attack page (Claude and Antigravity)
- Explaining unfamiliar crates and APIs (ask it to cite the current docs.rs page)

**What you should buy or borrow**
- **A Rust developer with security experience** for a focused review of `wavical-ipc`, `wavical-protocol` and the IPC handler. A few hours on those files is worth more than anything else you can spend on Phase 0.

**AI workflow rules that matter for this phase**
1. One work package per session. Never "build Phase 0" in one prompt.
2. Ask for complete files, with tests written alongside the code.
3. Tell it to read current docs first, and to say so if it's unsure about an API instead of guessing.
4. After each package: run `cargo test`, `cargo clippy -- -D warnings`, and read the diff yourself.
5. Ask the AI to explain every `unsafe`, every `unwrap`, and every place it skipped validation. Then ask for a second pass where it tries to attack its own code.
6. Don't use ChatGPT, Claude and Antigravity on the same file in parallel. Pick one owner per package to avoid conflicting styles.

---

## 9. Risks specific to Phase 0

| Risk | Mitigation |
|---|---|
| AI mixes winit and tao APIs | Decision in section 0, plus tell it to read current docs |
| Security checks done in JS instead of Rust | Rule: JS is untrusted, enforcement lives only in Rust |
| Scope creep (adding "just one more API") | Out-of-scope list in section 1 |
| Windows-only path edge cases missed | The test list in section 6, plus expert review |
| CI runners lacking WebView2 for integration tests | Verify on the runner early in WP1/WP2 and plan a fallback if it's missing |
| Tests that pass but don't test anything | Review the attack tests by deliberately breaking a check and confirming the test fails |
