# Wavical AI Subsystem — Low-End Hardware Benchmark (Phase 3 WP9)

**Document Version:** 1.0.0  
**Phase:** Phase 3 (AI-Native Infrastructure)  
**Work Package:** WP9 (Low-End Hardware Benchmark)  
**Status:** Awaiting User Benchmark Execution on Target 8 GB RAM Machine  

---

## 1. Objective & Target Machine Constraints

This benchmark measures the real-world performance, latency, and memory footprint of the Wavical AI subsystem on resource-constrained commodity personal computers:
- **Target Memory:** 8 GB total system RAM (with zero dedicated GPU VRAM).
- **Inference Mode:** CPU-only AVX2 execution without CUDA, ROCm, or discrete acceleration.
- **Operating System:** Windows 10 / Windows 11 (64-bit).
- **Core Principle:** AI features must never cause page swapping, system freezes, or exhaust the 8 GB machine budget.

---

## 2. Machine Specification Under Test

> *Note: This section will be recorded from the target 8 GB RAM machine upon benchmark execution.*

| Specification Parameter | Value / Model |
| :--- | :--- |
| **Device Manufacturer & Model** | *[Pending Run]* |
| **Processor (CPU)** | *[Pending Run]* |
| **CPU Cores & Threads** | *[Pending Run]* |
| **Base & Boost Clock Frequency** | *[Pending Run]* |
| **CPU SIMD Instruction Extensions** | *[Pending Run]* (AVX2, FMA, AVX, SSE4.2) |
| **System Memory (RAM)** | *[Pending Run]* GB (Channel / Speed / Type) |
| **Storage Type & Interface** | *[Pending Run]* (NVMe SSD / SATA SSD / HDD) |
| **Operating System & Build** | *[Pending Run]* (Windows 11 / Windows 10 x64) |

---

## 3. Benchmark Methodology & Repeatability

To ensure reproducibility across different machines and runs, the benchmark harness (`wavical ai bench`) adheres to strict invariants:

1. **Deterministic Prompts & Seeds:**
   - Standard Prompt: `"Explain the concept of virtual memory in operating systems in exactly two paragraphs."`
   - Fixed RNG Seed: `42` (used for pseudo-random synthetic chunk vectors and sampling where the engine permits).
2. **Warm-Up Runs:**
   - Exactly 1 unmeasured warm-up cycle is executed before measurements to prime CPU frequency scaling, disk caches, and thread pools.
3. **Statistical Rigor (At Least 5 Runs):**
   - Each metric is sampled across at least 5 consecutive measured runs.
   - The harness calculates and outputs **Min**, **Median**, **Max**, and **Mean**.
4. **Synthetic RAG Datasets (Zero External Documents):**
   - RAG indexing and search benchmarks generate synthetic, deterministic text and 384-dimensional unit-normalized embedding vectors in-memory.
   - Tested across three scales: **1,000 chunks**, **10,000 chunks**, and **50,000 chunks** (the maximum collection limit).
   - Zero confidential or real documents are required or stored.
5. **Memory & Working Set Tracking:**
   - Process working set (resident memory) is queried via `sysinfo` APIs.
   - Tracks peak working set during generation, idle memory when unused, and memory reclamation following idle engine shutdown.

---

## 4. Exact Execution Commands

### A. Run Full Local Benchmark Suite (Local Chat + RAG)
```powershell
# Ensure local chat model is installed
cargo run -p wavical-cli --bin wavical -- model pull qwen2.5-1.5b-instruct --accept-license Apache-2.0

# Run 5 measured runs with 1 warmup run and output machine-readable JSON
cargo run -p wavical-cli --bin wavical -- ai bench --runs 5 --warmup 1 --output docs/benchmark_results.json
```

### B. Run Vector Store / RAG Benchmark Only (1k, 10k, 50k Chunks)
```powershell
cargo run -p wavical-cli --bin wavical -- ai bench --rag-only --runs 5 --warmup 1 --output docs/benchmark_rag.json
```

### C. Run Cloud Path Benchmark (Optional)
```powershell
$env:OPENAI_API_KEY="your-api-key"
cargo run -p wavical-cli --bin wavical -- ai bench --cloud --runs 5 --warmup 1 --output docs/benchmark_cloud.json
```

---

## 5. Measured Results

> *Notice: Table entries are left EMPTY until execution on the 8 GB machine. Do not invent numbers.*

### 5.1 Local Chat Inference (`qwen2.5-1.5b-instruct`, CPU)

| Metric | Minimum | Median | Maximum | Mean |
| :--- | :---: | :---: | :---: | :---: |
| **Cold Start to First Token (ms)** | — | — | — | — |
| **Tokens per Second (TPS)** | — | — | — | — |
| **Peak Engine Working Set (MB)** | — | — | — | — |
| **Peak Shell Working Set (MB)** | — | — | — | — |
| **Idle Memory (Engine / Shell MB)** | — | — | — | — |
| **Memory Post-Shutdown (Engine / Shell MB)** | — | — | — | — |

---

### 5.2 RAG Vector Store & Retrieval (384-Dimensional Embeddings, SQLite)

| Dataset Scale | Chunks Indexed | Index Build Time (ms) | Search Latency Min (ms) | Search Latency Median (ms) | Search Latency Max (ms) |
| :--- | :---: | :---: | :---: | :---: | :---: |
| **Small Scale** | 1,000 | — | — | — | — |
| **Medium Scale** | 10,000 | — | — | — | — |
| **Max Capacity** | 50,000 | — | — | — | — |

---

### 5.3 Cloud Chat Inference (Optional / If Configured)

| Metric | Minimum | Median | Maximum | Mean |
| :--- | :---: | :---: | :---: | :---: |
| **Cold Start to First Token (ms)** | — | — | — | — |
| **Tokens per Second (TPS)** | — | — | — | — |
| **Peak Shell Working Set (MB)** | — | — | — | — |

---

## 6. Proposed Performance Budgets (Awaiting User Approval)

The following baseline performance budgets are proposed for standard operation on an 8 GB RAM Windows device without GPU acceleration:

1. **RAG Vector Search Latency:**
   - 1,000 chunks: $\le 10.0\text{ ms}$
   - 10,000 chunks: $\le 25.0\text{ ms}$
   - 50,000 chunks: $\le 60.0\text{ ms}$
2. **RAG 50k Index Build Time:**
   - 50,000 chunks full indexing: $\le 30.0\text{ seconds}$ ($30,000\text{ ms}$)
3. **Memory Footprint Ceiling:**
   - Shell working set: $\le 250\text{ MB}$
   - Engine peak working set: $\le 4,096\text{ MB}$ ($4.0\text{ GB}$, leaving $\ge 4.0\text{ GB}$ for OS and other applications)
   - Engine post-shutdown working set: $0.0\text{ MB}$ (engine process terminated)
4. **Local Text Generation Throughput:**
   - Cold start to first token: $\le 1,500\text{ ms}$
   - Generation speed: $\ge 12.0\text{ tokens/sec}$ on AVX2-capable CPU

*Status: Awaiting user benchmark numbers from the 8 GB machine before final budget ratification.*

