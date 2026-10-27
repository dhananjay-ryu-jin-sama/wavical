# Wavical — Dev Environment Setup (Phase 0)

Everything needed to build and test Phase 0 on Windows. Run the commands yourself in PowerShell, or hand this file to Antigravity and tell it to follow it step by step and report each result.

> Rule for the agent: **never invent or pin crate versions from memory.** Use `cargo add <crate>` so Cargo fetches the current release, and check docs.rs for the current API before writing code.

---

## 1. System tools (Windows)

Run in PowerShell. `winget` ships with current Windows 10/11.

```powershell
# Git
winget install --id Git.Git -e

# Rust toolchain manager (installs rustc, cargo, rustfmt, clippy via rustup)
winget install --id Rustlang.Rustup -e

# C++ build tools (required by Rust on Windows, MSVC linker)
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

# WebView2 runtime (already on most Windows 11 machines; harmless to run)
winget install --id Microsoft.EdgeWebView2Runtime -e

# Node.js (>= 14.8.0, required for developer CLI evaluation of wavical.config.js)
winget install --id OpenJS.NodeJS -e
```

Close and reopen the terminal afterwards so PATH updates. If a newer Visual Studio Build Tools version exists, that works too. The key part is the **"Desktop development with C++"** workload.

Then set up Rust:

```powershell
rustup default stable
rustup component add rustfmt clippy
```

## 2. Cargo tools

```powershell
cargo install cargo-audit --locked     # checks dependencies for known vulnerabilities
cargo install cargo-deny --locked      # license, ban and advisory checks
cargo install cargo-nextest --locked   # optional: faster test runner
```

## 3. Fuzzing environment (Linux / WSL)

`cargo-fuzz` is much more reliable on Linux than on Windows, so run fuzzing in WSL or your Linux VM. Phase 0 keeps `wavical-ipc` and `wavical-protocol` free of window dependencies so they build there easily.

```powershell
# One time, from PowerShell (admin): installs WSL with Ubuntu
wsl --install
```

Inside Ubuntu:

```bash
sudo apt update
sudo apt install -y build-essential curl git pkg-config

# Rust inside WSL
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Fuzzing needs nightly
rustup toolchain install nightly
cargo install cargo-fuzz --locked
```

Only needed if you later build the shell itself on Linux (not Phase 0):

```bash
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev
```

## 4. Create the workspace

```powershell
mkdir wavical; cd wavical
git init

cargo new crates/wavical-ipc --lib
cargo new crates/wavical-protocol --lib
cargo new crates/wavical-shell
```

Then create the root `Cargo.toml` as a workspace:

```toml
[workspace]
resolver = "2"
members = ["crates/wavical-ipc", "crates/wavical-protocol", "crates/wavical-shell"]
```

## 5. Add dependencies (let Cargo pick current versions)

```powershell
# wavical-ipc
cargo add --package wavical-ipc serde --features derive
cargo add --package wavical-ipc serde_json
cargo add --package wavical-ipc thiserror
cargo add --package wavical-ipc tracing
cargo add --package wavical-ipc --dev proptest

# wavical-protocol
cargo add --package wavical-protocol thiserror
cargo add --package wavical-protocol tracing
cargo add --package wavical-protocol --dev proptest
cargo add --package wavical-protocol --dev tempfile

# wavical-shell
cargo add --package wavical-shell tao
cargo add --package wavical-shell wry
cargo add --package wavical-shell tracing
cargo add --package wavical-shell tracing-subscriber
cargo add --package wavical-shell serde_json
cargo add --package wavical-shell --path crates/wavical-ipc
cargo add --package wavical-shell --path crates/wavical-protocol
```

Add this at the top of `wavical-ipc/src/lib.rs`, `wavical-protocol/src/lib.rs`, and `wavical-shell/src/main.rs`:

```rust
#![forbid(unsafe_code)]
```

## 6. Check the command names are free

Before you build anything under these names, make sure nothing on your machine already uses them. Run in PowerShell, and again in WSL with `which`:

```powershell
where wavical
where wca
```

Both should say "could not find files". If `wca` is already taken by some installed tool, tell me and we'll pick another alias before the CLI is built in Phase 1. Also reserve the names you'll publish under: `wavical-*` on crates.io, the npm package, and the GitHub org.

---

## 7. Verify everything works

Run this block. Every line should succeed.

```powershell
rustc --version
cargo --version
git --version
cargo clippy --version
cargo audit --version
cargo deny --version
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all --check
cargo audit
```

In WSL (fuzzing):

```bash
cargo +nightly --version
cargo fuzz --version
```

If a step fails, paste the full error back to me and I'll help debug it. Common causes:
- `link.exe not found`: the C++ build tools workload isn't installed, or the terminal wasn't restarted
- `cargo audit` fails to install: update Rust (`rustup update`) and retry
- WebView2 window doesn't open later: check the runtime is installed (Settings > Apps, "Microsoft Edge WebView2 Runtime")

## 8. Not needed yet

Node, Bun, and any AI/model tooling are **not** needed for Phase 0. Don't let the agent install them. They arrive in Phases 3 and 4.
