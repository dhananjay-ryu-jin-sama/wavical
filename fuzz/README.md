# Fuzzing Suite

`cargo-fuzz` targets for fuzz testing `wavical-ipc` message parsing, capability dispatch, `wavical-protocol` path resolution, native request parsing, grant table isolation, AI request validation, catalog parsing, and text chunking.

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

4. **Fuzz Native Request Parameter Parser (Phase 2):**
   ```bash
   cargo +nightly fuzz run native_request_parser
   ```

5. **Fuzz Grant Table Isolation & Limits (Phase 2):**
   ```bash
   cargo +nightly fuzz run grant_table
   ```

6. **Fuzz AI Request Parsers & Validators (Phase 3 WP10):**
   ```bash
   cargo +nightly fuzz run ai_request_parser
   ```

7. **Fuzz Model Catalog Parser (Phase 3 WP10):**
   ```bash
   cargo +nightly fuzz run catalog_parser
   ```

8. **Fuzz RAG Text Chunker (Phase 3 WP10):**
   ```bash
   cargo +nightly fuzz run chunker
   ```

### Nightly WSL Continuous / Long-Running Fuzzing Commands

To run nightly continuous fuzzing in WSL across all targets:
```bash
# In WSL:
rustup toolchain install nightly
cargo +nightly install cargo-fuzz

# Overnight runs (e.g. 1 hour / 3600 seconds per target):
cargo +nightly fuzz run parse_message -- -max_total_time=3600
cargo +nightly fuzz run capability_dispatch -- -max_total_time=3600
cargo +nightly fuzz run resolve_path -- -max_total_time=3600
cargo +nightly fuzz run native_request_parser -- -max_total_time=3600
cargo +nightly fuzz run grant_table -- -max_total_time=3600
cargo +nightly fuzz run ai_request_parser -- -max_total_time=3600
cargo +nightly fuzz run catalog_parser -- -max_total_time=3600
cargo +nightly fuzz run chunker -- -max_total_time=3600
```

### Reproducing Crash Artifacts

If an input triggers a crash or panic, `cargo-fuzz` saves it to `fuzz/artifacts/<target>/`. Reproduce and debug with:
```bash
cargo +nightly fuzz run ai_request_parser fuzz/artifacts/ai_request_parser/crash-<hash>
```
Any crash inputs discovered should be committed as permanent regression unit tests.
