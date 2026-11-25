# Wavical Security Fuzzing Guide (Nightly WSL)

Wavical maintains dedicated `cargo-fuzz` (libFuzzer) targets to continuously stress-test untrusted input boundaries across IPC parsing, capability evaluation, path resolution, native subsystem params, and AI processing components.

## Target Matrix

| Target | Subsystem | Invariants Verified |
|---|---|---|
| `parse_message` | `wavical-ipc` | Safe JSON parsing, length bounds, type sanitization without panic |
| `capability_dispatch` | `wavical-ipc` | Strict default-deny, capability routing without unauthorized dispatch |
| `resolve_path` | `wavical-protocol` | Path traversal prevention, UNC/device name blocking, safe root containment |
| `native_request_parser` | `wavical-ipc` | Menu parameter bounds, shortcut syntax, file extension sanitization |
| `grant_table` | `wavical-ipc` | Max grants per window (64), per-window isolation, random ID entropy |
| `ai_request_parser` | `wavical-ipc` | Chat message limits (32k chars, 50 messages, 128k total), index documents (100 docs, 100k chars), search query (4k chars, topK 1-20), collection names |
| `catalog_parser` | `wavical-ai` | Malformed JSON, extra fields, invalid URLs, bad sha256 checksum strings, enum bounds |
| `chunker` | `wavical-ai` | Arbitrary unicode, multibyte splits, zero/huge chunk sizes, extreme overlaps without panics or infinite loops |

---

## Environment Setup (WSL / Linux)

Run the following inside Ubuntu / WSL:

```bash
# 1. Install Rust nightly toolchain
rustup toolchain install nightly

# 2. Install cargo-fuzz
cargo +nightly install cargo-fuzz

# 3. Ensure build prerequisites (build-essential, pkg-config, libssl-dev)
sudo apt-get update && sudo apt-get install -y build-essential pkg-config libssl-dev
```

---

## Individual Fuzz Runs

To run an individual fuzzer from the repository root:

```bash
cargo +nightly fuzz run ai_request_parser
cargo +nightly fuzz run catalog_parser
cargo +nightly fuzz run chunker
cargo +nightly fuzz run parse_message
cargo +nightly fuzz run capability_dispatch
cargo +nightly fuzz run resolve_path
cargo +nightly fuzz run native_request_parser
cargo +nightly fuzz run grant_table
```

---

## Nightly Continuous WSL Fuzzing Execution

For continuous overnight fuzzing runs (e.g., 3600 seconds = 1 hour per target):

```bash
#!/usr/bin/env bash
set -euo pipefail

TARGETS=(
  "ai_request_parser"
  "catalog_parser"
  "chunker"
  "parse_message"
  "capability_dispatch"
  "resolve_path"
  "native_request_parser"
  "grant_table"
)

echo "Starting Wavical Nightly Fuzzing Cycle..."
for target in "${TARGETS[@]}"; do
  echo ">>> Running fuzzer: $target for 3600 seconds..."
  cargo +nightly fuzz run "$target" -- -max_total_time=3600 -workers=2 -jobs=2
done
echo "Wavical Nightly Fuzzing Cycle Complete."
```

---

## Reproducing & Triaging Crashes

When `cargo-fuzz` identifies an input causing an unexpected panic or memory violation, it automatically writes an artifact to:
`fuzz/artifacts/<target>/crash-<hash>`

To reproduce the exact execution path:
```bash
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/crash-<hash>
```

Every discovered edge case must be converted into a deterministic Rust unit test in the corresponding crate's `tests/` directory.

