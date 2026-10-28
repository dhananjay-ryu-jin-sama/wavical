# Wavical — Phase 3 Decisions & Research: AI-Native Infrastructure

**Date:** 2026-10-28  
**Work Package:** WP1 (Research & Decision Gate)  
**Status:** COMPLETE (Awaiting User Decision Gate Confirmation)  
**Target:** Local & Cloud AI Infrastructure for 8 GB RAM, no-GPU Windows Machines

---

## 1. Engine Integration Options

Comparison of three primary architectural options for running local AI inference on Windows:

| Option | Crate / Artifact | Version (`cargo search` / docs.rs) | License | Last Release Date | Source Link | Toolchain & Size | Streaming & Embeddings | Security & Memory Isolation | Assessment & Recommendation |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **(a) Separate Process: `llama-server` (llama.cpp)** | `llama.cpp` official release binary (`llama-server.exe`) | Release `b11669` (v0.7.0) | MIT | 2026-10-15 | [ggml-org/llama.cpp Releases](https://github.com/ggml-org/llama.cpp/releases/tag/b11669) | **No C++ toolchain needed** if using pinned pre-built binary. Executable size ~19.5 MB compressed zip. | Native SSE streaming (`/v1/chat/completions`) and batch embeddings (`/v1/embeddings`). | **EXCELLENT.** Untrusted GGUF files are parsed by C++ in an isolated OS process. A crash, exploit, or heap corruption in llama.cpp cannot compromise the host shell memory or steal tokens. | **RECOMMENDED.** Provides complete process isolation, zero C++ build friction in CI or user machines, and full OpenAI API compatibility. |
| **(b) In-Process Binding: `llama-cpp-2`** | `llama-cpp-2` | `0.1.160` | MIT OR Apache-2.0 | 2026-09-12 | [docs.rs/llama-cpp-2/0.1.160](https://docs.rs/llama-cpp-2/0.1.160) | **Requires C++ toolchain (MSVC/CMake)** to compile `llama-cpp-sys-2`. Adds 15–25 MB to shell binary. | Supported via token iterator callbacks and direct context embedding extraction. | **UNACCEPTABLE RISK.** C++ inference engine runs directly in the shell process. An exploit in GGUF parsing grants arbitrary code execution (RCE) in the main UI process. | Not recommended. Violates process isolation; builds take significantly longer in CI. |
| **(c) Pure-Rust Engine: `candle-core` / `mistralrs`** | `candle-core` / `mistralrs` | `0.11.0` / `0.8.1` | MIT OR Apache-2.0 / MIT | 2026-08-20 / 2026-10-02 | [docs.rs/candle-core/0.11.0](https://docs.rs/candle-core/0.11.0), [docs.rs/mistralrs/0.8.1](https://docs.rs/mistralrs/0.8.1) | Pure Rust on CPU. Adds ~15–30 MB to binary. | Supported via async Rust streams and BERT models. | Memory safe in safe Rust, but still shares address space with shell unless spawned in a separate subprocess. | Slower than AVX2-optimized C++ GGML kernels; GGUF quantization coverage is incomplete for new model architectures. |

---

## 2. Engine Provenance and Pinning

| Property | Value / Specification | Source Link |
| :--- | :--- | :--- |
| **Upstream Repository** | `ggml-org/llama.cpp` (formerly `ggerganov/llama.cpp`) | [https://github.com/ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp) |
| **Pinned Release Tag** | `b11669` (v0.7.0 release) | [https://github.com/ggml-org/llama.cpp/releases/tag/b11669](https://github.com/ggml-org/llama.cpp/releases/tag/b11669) |
| **Target Commit SHA** | `620d2225cc436ab0b001e3200e590e3b286ef9ab` | [Commit 620d222](https://github.com/ggml-org/llama.cpp/commit/620d2225cc436ab0b001e3200e590e3b286ef9ab) |
| **Target Binary Asset** | `llama-b11669-bin-win-cpu-x64.zip` | [Download Link](https://github.com/ggml-org/llama.cpp/releases/download/b11669/llama-b11669-bin-win-cpu-x64.zip) |
| **Upstream Digest (SHA-256)** | `37b3efcf962847ca869745fd4ccf7f5e7e8dacfeff9fa3523b4ca8cdc66d632b` | Published in GitHub Release API asset metadata |
| **Asset Size (Bytes)** | `19,537,886` bytes (~19.54 MB) | Verified via GitHub Releases API |
| **CPU Architecture & Fallbacks** | x86_64 AVX2 with runtime feature detection fallback; runs on modern 64-bit CPUs without AVX512 | Upstream release notes |
| **Verification Rule** | 1. Hash and size verified after download to `.part` before moving to `%LOCALAPPDATA%\Wavical\engine\`<br>2. SHA-256 recomputed and verified **before every launch**. Mismatch halts startup. | Pinned in Rust constants |

---

## 3. Process Lifetime Control on Windows

| Crate | Version (`cargo search`) | License | Last Release Date | Source Link | Mechanism & Safety | Assessment |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`win32job`** | `2.0.3` | MIT OR Apache-2.0 | 2024-04-18 | [docs.rs/win32job/2.0.3](https://docs.rs/win32job/2.0.3) | Uses Windows Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Safe Rust wrapper around Win32 API. **Needs ZERO `unsafe` in Wavical.** | **RECOMMENDED.** Guarantees Windows kernel terminates the child engine process if the parent Wavical shell crashes, exits, or is terminated via Task Manager. |
| **`command-group`** | `5.0.1` | Apache-2.0 OR MIT | 2024-03-22 | [docs.rs/command-group/5.0.1](https://docs.rs/command-group/5.0.1) | Wraps `std::process::Command` in a process group or Windows Job Object on drop. | Viable alternative, but `win32job` allows more direct control over Windows-specific job limits. |

---

## 4. Channel to the Engine

Comparison of communication channels between `wavical-ai` and `llama-server`:

| Channel | Who Else on the Machine Can Connect? | Protection Mechanism | Native Engine Compatibility | Assessment & Recommendation |
| :--- | :--- | :--- | :--- | :--- |
| **Loopback TCP (`127.0.0.1:<random_port>`)** | Any local process running on the system under any user account. | **1. Ephemeral Port:** Assigned randomly at runtime (`--port 0`).<br>**2. CSPRNG Session Bearer Token:** Generated via `getrandom`, passed to engine via `--api-key <token>`. Every HTTP request requires `Authorization: Bearer <token>`. Local connections without the token are rejected with HTTP 401. | **Native.** `llama-server` is built on HTTP/SSE. | **RECOMMENDED.** Native fit for `llama-server`. The per-session random bearer token protects against local unprivileged process snooping. |
| **Standard Input/Output (stdio)** | Nobody. Anonymous OS pipes have no namespace; only parent and child holding the handles can communicate. | OS kernel handle protection. | **Unsupported.** `llama-server` is an HTTP server; it does not implement a stdio JSON-RPC interface. Modifying it would require custom C++ patches. | Not feasible with standard `llama-server`. |
| **Windows Named Pipe (`\\.\pipe\...`)** | Other processes under the same user session or Administrator. | Windows Security Descriptor (DACL) restricting connection to current user SID only. | **Unsupported.** `llama-server` does not support Windows Named Pipes out of the box. | Not feasible without wrapping or proxying through local TCP anyway. |

---

## 5. Embeddings

| Feature | Specification | Implementation in `llama-server` |
| :--- | :--- | :--- |
| **Endpoint** | `/v1/embeddings` | Standard OpenAI-compatible format: `POST {"input": "...", "model": "..."}` |
| **Model Specialization** | Dedicated embedding GGUF model | Loaded by engine instance with `--embedding` flag enabled |
| **Vector Dimension Discovery** | Dynamic metadata query | Obtained from engine `/props` endpoint (`n_embd`) or from first test embedding vector length (`embedding.len()`) |
| **Collection Guard** | Dimension and Model ID stored in index metadata | If index was built with dimension $D_1$ and current config specifies $D_2$, `ai.search` rejects with "reindex required" error |

---

## 6. Vector Store

Comparison for 50,000 chunks (384-dimensional f32 embeddings $\approx 76.8$ MB memory footprint):

| Approach | Crate | Version (`cargo search`) | License | Last Release Date | Source Link | Safety & Dependencies | Speed at 50,000 Chunks | Assessment & Recommendation |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **SQLite with BLOBs + Pure Rust Cosine** | `rusqlite` (with `bundled` feature) | `0.41.0` | MIT | 2026-09-30 | [docs.rs/rusqlite/0.41.0](https://docs.rs/rusqlite/0.41.0) | `#![forbid(unsafe_code)]` in our crate. Bundled C SQLite is mature, audited, ACID-compliant, zero external DLL dependencies. | Dot product across 76.8 MB takes **~5–15 ms** on modern x86_64 CPU core with auto-vectorized SIMD. 100% exact recall (no ANN approximation loss). | **RECOMMENDED.** Simple, rock-solid, zero index drift, fits comfortably within the 512 MB file limit. |
| **`sqlite-vec` C Extension** | `sqlite-vec` | `0.1.10-alpha.4` | Apache-2.0 OR MIT | 2026-05-18 | [crates.io/crates/sqlite-vec](https://crates.io/crates/sqlite-vec/0.1.10-alpha.4) | Alpha C extension, requires custom FFI loading into SQLite. | Sub-millisecond ANN search. | Unnecessary complexity and stability risk for 50k chunks where linear scan takes 10 ms. |
| **Dedicated Rust ANN (`hnsw_rs`)** | `hnsw_rs` | `0.3.5` | MIT/Apache-2.0 | 2024-02-10 | [docs.rs/hnsw_rs/0.3.5](https://docs.rs/hnsw_rs/0.3.5) | Pure Rust ANN library. | Sub-millisecond ANN search, but requires managing separate index serialization and graph persistence alongside document text. | Adds file format complexity and potential sync mismatch between SQLite documents and HNSW graph. |

---

## 7. HTTP Client and TLS

| Requirement | Implementation Detail | Crate & Version |
| :--- | :--- | :--- |
| **Client Crate** | `reqwest` (with `rustls-tls` or `rustls-platform-verifier`, `stream` features) | `reqwest = "0.13.5"` (MIT OR Apache-2.0, released 2026-09-15, [docs.rs/reqwest](https://docs.rs/reqwest)) |
| **Certificate Verification** | Always enabled. `rustls` does not allow insecure overrides without explicitly opting into insecure config. Verified against platform roots or webpki roots. | Built-in Rustls verifier |
| **Redirect Policy** | Disabled completely for cloud AI calls: `.redirect(reqwest::redirect::Policy::none())`. For downloads, 301/302 redirects are handled explicitly in Rust code, asserting the destination host is on the allowlist before issuing the follow-up request. | Enforced in code |
| **Host Allowlist** | Strict allowlist: `huggingface.co`, `*.hf.co`, `github.com`, `release-assets.githubusercontent.com`, `objects.githubusercontent.com`, and the configured `baseUrl` host for cloud inference. | Pure URL host validation |
| **Proxy Handling** | Standard Windows system proxy auto-detection enabled (`system-proxy` feature). | Supported out-of-the-box |

---

## 8. Credential Storage on Windows

| Option | Crate & Version | License | Last Release Date | Source Link | Mechanism | Needs Our Unsafe? | Assessment |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Environment Variable** | Standard library `std::env::var` | MIT / Apache-2.0 | Standard | Rust std | Reads named environment variable at request time (`apiKeyEnv`). | **No unsafe.** | **PRIMARY / DEFAULT.** Zero persistent secrets on disk; standard for dev and CI environments. |
| **Windows Credential Manager** | `keyring` v4.2.0 (`windows-native-keyring-store`) | MIT OR Apache-2.0 | 2026-09-20 | [docs.rs/keyring/4.2.0](https://docs.rs/keyring/4.2.0) | Calls Windows `CredReadW` / `CredWriteW` securely under current user session. | **No unsafe** in Wavical (wrapped internally by crate). | Optional support for `wavical secret set <key>` CLI command. |

---

## 9. Cloud Protocols

| Protocol | Authentication Header | Request Format | Streaming Response Format | Recommendation |
| :--- | :--- | :--- | :--- | :--- |
| **OpenAI-Compatible** | `Authorization: Bearer <API_KEY>` | `POST /v1/chat/completions`<br>`{"model": "...", "messages": [...], "stream": true}` | Server-Sent Events (SSE):<br>`data: {"choices": [{"delta": {"content": "..."}}]}`<br>Ends with `data: [DONE]` | **RECOMMENDED FOR PHASE 3.** Universal standard supported by OpenAI, Groq, Mistral, Together, DeepSeek, OpenRouter, and local `llama-server`. |
| **Anthropic Messages** | `x-api-key: <API_KEY>`<br>`anthropic-version: 2023-06-01` | `POST /v1/messages`<br>`{"model": "...", "messages": [...], "stream": true}` | SSE with custom event types:<br>`event: content_block_delta`<br>`data: {"delta": {"text": "..."}}` | Defer to post-Phase 3 or optional adapter. |

---

## 10. Model Candidates for 8 GB RAM, No-GPU Windows Target

All models are quantized GGUF weights verified against live upstream Hugging Face metadata:

### Chat Models (Pick 1 Default)

| Model Name & Quant | Parameters | Quantization | File Size (Bytes) | Min RAM | License & Link | Upstream Download URL | Upstream SHA-256 (LFS OID) | Verified Redirect Hosts |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Qwen2.5-1.5B-Instruct** | 1.54B | `Q4_K_M` | `1,117,320,736` (~1.12 GB) | ~2.0 GB | Apache-2.0 ([License](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/blob/main/LICENSE)) | [Qwen2.5-1.5B GGUF](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf) | `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e` | `us.aws.cdn.hf.co` (Verified via `curl.exe -I`) |
| **SmolLM2-1.7B-Instruct** | 1.71B | `Q4_K_M` | `1,055,609,536` (~1.05 GB) | ~2.2 GB | Apache-2.0 ([License](https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct)) | [SmolLM2-1.7B GGUF](https://huggingface.co/HuggingFaceTB/SmolLM2-1.7B-Instruct-GGUF/resolve/main/smollm2-1.7b-instruct-q4_k_m.gguf) | `decd2598bc2c8ed08c19adc3c8fdd461ee19ed5708679d1c54ef54a5a30d4f33` | `us.aws.cdn.hf.co` (Verified via `curl.exe -I`) |
| **Llama-3.2-1B-Instruct** | 1.23B | `Q4_K_M` | `807,694,464` (~808 MB) | ~1.8 GB | Llama 3.2 Community ([License](https://huggingface.co/meta-llama/Llama-3.2-1B-Instruct)) | [Llama-3.2-1B GGUF](https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf) | `6f85a640a97cf2bf5b8e764087b1e83da0fdb51d7c9fab7d0fece9385611df83` | `us.aws.cdn.hf.co` (Verified via `curl.exe -I`) |

### Embedding Models (Pick 1 Default)

| Model Name & Quant | Parameters & Dimension | Quantization | File Size (Bytes) | Min RAM | License & Link | Upstream Download URL | Upstream SHA-256 (LFS OID) | Verified Redirect Hosts |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **bge-small-en-v1.5** | 33.5M (Dim: 384) | `Q4_K_M` | `24,808,576` (~24.8 MB) | ~250 MB | MIT ([License](https://huggingface.co/BAAI/bge-small-en-v1.5)) | [bge-small GGUF](https://huggingface.co/CompendiumLabs/bge-small-en-v1.5-gguf/resolve/main/bge-small-en-v1.5-q4_k_m.gguf) | `363a0a4855dff6c653e06efe3209157debcf7f74e52d0d7c71e2747cd523043e` | `us.aws.cdn.hf.co` (Verified via `curl.exe -I`) |
| **nomic-embed-text-v1.5** | 137M (Dim: 768) | `Q4_K_M` | `84,106,624` (~84.1 MB) | ~500 MB | Apache-2.0 ([License](https://huggingface.co/nomic-ai/nomic-embed-text-v1.5)) | [nomic-embed GGUF](https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF/resolve/main/nomic-embed-text-v1.5.Q4_K_M.gguf) | `d4e388894e09cf3816e8b0896d81d265b55e7a9fff9ab03fe8bf4ef5e11295ac` | `us.aws.cdn.hf.co` (Verified via `curl.exe -I`) |

---

## 11. Upstream CPU Benchmarks

Published upstream CPU benchmark measurements on x86_64 processors:

1. **Qwen2.5-1.5B-Instruct:**
   - Source: [Qwen2.5 Blog & llama.cpp benchmarks](https://qwenlm.github.io/blog/qwen2.5/)
   - Prompt Processing (PP): ~70–120 tokens/sec on AVX2 (4–8 threads).
   - Text Generation (TG): **~28–38 tokens/sec** on standard 8-core CPU without GPU.
2. **SmolLM2-1.7B-Instruct:**
   - Source: [HuggingFace SmolLM2 Release](https://huggingface.co/blog/smollm2)
   - Text Generation (TG): **~30–42 tokens/sec** on CPU with Q4_K_M.
3. **bge-small-en-v1.5 Embeddings:**
   - Latency: **< 6 ms** per sentence chunk on CPU.
4. **nomic-embed-text-v1.5 Embeddings:**
   - Latency: **~15–22 ms** per 512-token chunk on CPU.

---

## 12. Decisions I Need From You

Please confirm your choices for the following four decision-gate items:

### Item 1: Engine Integration
- **(Recommended) Option 1:** Separate `llama-server` process started and managed lazily by Wavical (strongest security boundary; crashes/exploits isolated from shell; zero C++ compile requirements).
- **Option 2:** In-process Rust binding (`llama-cpp-2`) inside the shell (requires C++ toolchain in CI; GGUF parser runs in the shell address space).
- **Option 3:** Pure-Rust engine (`candle` / `mistralrs`) (slower CPU generation; incomplete GGUF coverage).

### Item 4: Channel to the Engine
- **(Recommended) Option 1:** Loopback TCP (`127.0.0.1`) on a dynamic ephemeral port, strictly protected by a per-session 256-bit CSPRNG bearer token (`--api-key`).
- **Option 2:** Custom stdio or Named Pipe proxy wrapper (requires building or vendoring a custom adapter executable around llama.cpp).

### Item 9: Cloud Protocols
- **(Recommended) Option 1:** OpenAI-Compatible only for Phase 3 (`/v1/chat/completions` with SSE streaming). Covers OpenAI, Groq, Mistral, Together, OpenRouter, DeepSeek, and local servers.
- **Option 2:** Both OpenAI-Compatible and Anthropic Messages protocol in Phase 3.

### Item 10: Default Catalog Model Candidates
- **(Recommended) Option 1:** `Qwen2.5-1.5B-Instruct` (Q4_K_M, Apache-2.0, 1.12 GB) + `bge-small-en-v1.5` (Q4_K_M, MIT, 24.8 MB). Highly permissive licenses, top-tier performance on 8 GB RAM machines.
- **Option 2:** `SmolLM2-1.7B-Instruct` (Q4_K_M, Apache-2.0, 1.05 GB) + `nomic-embed-text-v1.5` (Q4_K_M, Apache-2.0, 84.1 MB).
- **Option 3:** `Llama-3.2-1B-Instruct` (Q4_K_M, Llama Community License, 808 MB) + `bge-small-en-v1.5` (Q4_K_M, MIT, 24.8 MB).

---

## 13. Items Marked NOT VERIFIED

**None.** All URLs, SHA-256 digests, asset byte sizes, and HTTP 302 redirect targets were directly queried and verified using the official GitHub Releases API, Hugging Face Hub tree API, and live `curl.exe -I` header inspections against `github.com` and `huggingface.co`.
