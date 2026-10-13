# Fuzzing Suite

`cargo-fuzz` targets for fuzz testing `wavical-ipc` message parsing, capability dispatch, and `wavical-protocol` path resolution.

## Requirements (Linux or WSL)
- Rust nightly toolchain: `rustup toolchain install nightly`
- `cargo-fuzz`: `cargo install cargo-fuzz`

## Running the Targets

Run in Linux or WSL from the repository root:

1. **Fuzz IPC Message Parser:**
   ```bash
   cargo +nightly fuzz run parse_message
   ```

2. **Fuzz Capability Model & Dispatch:**
   ```bash
   cargo +nightly fuzz run capability_dispatch
   ```

3. **Fuzz Path Resolver & Sanitizer:**
   ```bash
   cargo +nightly fuzz run resolve_path
   ```

### Running with Time Limits & Sanitizers

To run a fuzz target for a fixed time limit (e.g. 60 seconds):
```bash
cargo +nightly fuzz run parse_message -- -max_total_time=60
cargo +nightly fuzz run capability_dispatch -- -max_total_time=60
cargo +nightly fuzz run resolve_path -- -max_total_time=60
```

### Reproducing Crash Artifacts

If an input triggers a crash or panic, `cargo-fuzz` saves it to `fuzz/artifacts/<target>/`. Reproduce and debug with:
```bash
cargo +nightly fuzz run parse_message fuzz/artifacts/parse_message/crash-<hash>
```
Any crash inputs discovered should be committed as permanent regression unit tests.
