You are building Wavical, a lightweight, security-first desktop app framework (an alternative to Electron and Tauri). Full command: `wavical`, short alias: `wca`. Stack: Rust, tao + wry (WebView2 on Windows). I am not a Rust expert, so you must be careful, explicit and honest.

BEFORE ANY CODE, read these files in docs/:
- WAVICAL_PHASE_0_BUILD_PLAN.md (the plan you must follow)
- WAVICAL_FEATURE_CHECKLIST.md (overall scope)
- WAVICAL_SETUP.md (environment)

HARD RULES
1. Work on ONE work package (WP) at a time, only the one I name. Do not start the next one. Do not add features outside Phase 0 scope (see "Out of scope" in the plan).
2. Windowing crate is `tao` with `wry`. Use tao's own event loop style. NEVER mix in winit's ApplicationHandler API. Before writing any tao/wry code, read the current examples and docs on docs.rs and the crates' repositories and follow them.
3. Never invent or pin crate versions from memory. Use `cargo add`. Never invent API names. If you are unsure an API exists, check the docs or tell me you are unsure.
4. `#![forbid(unsafe_code)]` stays in wavical-ipc, wavical-protocol and wavical-shell. No `unsafe`.
5. No `unwrap()`, `expect()` or indexing that can panic on anything derived from external input (IPC messages, URLs, file paths, JSON). Use proper error handling with `thiserror`.
6. The webview page is UNTRUSTED. All security enforcement (capabilities, origin checks, validation, path sanitization) happens in Rust. JavaScript-side wrappers are convenience only.
7. Security-critical logic lives in the pure crates (wavical-ipc, wavical-protocol), which must not depend on tao or wry.
8. Return generic errors to the page. Log details with `tracing`. Never leak internal paths or error text to the webview.
9. Write tests together with the code, in the same session. Every security rule needs a test that fails if the rule is removed.
10. Write complete files, never partial snippets, "..." placeholders or TODO stubs for required behavior. If you change an existing file, give me the full updated file.
11. Do not add dependencies beyond what the plan lists without asking me first and explaining why.
12. Run the real commands and show me the real output. Never claim something passes unless you ran it. Required at the end of every WP: `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --all --check`.
13. If the plan and reality disagree (an API changed, a step is impossible), STOP and tell me. Do not silently work around it.

WHEN A WORK PACKAGE IS DONE, reply with:
- what you built (short)
- files created or changed
- the real output of the commands above
- anything you were unsure about, skipped, or simplified
- security shortcuts or assumptions you made
Then stop and wait for me.

Reply "Ready" once you have read the three docs, and list in two or three lines the main rules you will follow.