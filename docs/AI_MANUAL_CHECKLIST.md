# Wavical Phase 3 AI Manual Verification Checklist (WP3 to WP9)

This checklist compiles all manual verification procedures from Work Packages WP3 through WP9 into a single end-to-end testing session for Windows.

---

## Session Prerequisites
- Windows 10/11 x64
- Terminal (PowerShell or Windows Terminal)
- Built binaries: `cargo build --workspace`
- Target binary path: `.\target\debug\wavical.exe` (or `.\target\release\wavical.exe`)

---

## Checklist Matrix

### WP3: Catalog, Doctor & Download Flow
- [ ] **Check 1: System Doctor AI Diagnostics**
  - **Command**: `.\target\debug\wavical doctor --ai`
  - **Expected Result**: Output displays:
    - CPU model and core count
    - Physical RAM size and free memory
    - Catalog status (embedded catalog parsed and valid)
    - Available models list
    - Cached engine and model count
    - Exit code 0.

- [ ] **Check 2: Interactive License Prompt & Download**
  - **Command**: `.\target\debug\wavical ai download bge-small-en-v1.5`
  - **Expected Result**:
    - Terminal displays license terms, model size, and responsibility disclaimer.
    - Prompts to confirm by typing model ID `bge-small-en-v1.5`.
    - Shows progress bar during transfer.
    - Computes SHA-256 hash and verifies against catalog.
    - Reports: `Model 'bge-small-en-v1.5' downloaded and verified successfully.`

- [ ] **Check 3: Non-Interactive License Flag Download**
  - **Command**: `.\target\debug\wavical ai download qwen2.5-0.5b-instruct --accept-license "Apache-2.0"`
  - **Expected Result**: Downloads without interactive prompt; SHA-256 integrity verified; placed in cache.

---

### WP4: Engine Lifecycle & Process Isolation
- [ ] **Check 4: Engine Status Check**
  - **Command**: `.\target\debug\wavical ai status`
  - **Expected Result**: Reports engine state: `Engine: Stopped` (or `Idle`), active port, and memory metrics.

- [ ] **Check 5: Windows Job Object Kill-on-Close Verification**
  - **Steps**:
    1. Start a local app that activates the local engine (`wavical dev examples/hello`).
    2. Open Windows Task Manager (or Process Explorer).
    3. Locate `wavical.exe` and child `llama-server.exe`.
    4. Kill `wavical.exe` via Task Manager.
  - **Expected Result**: `llama-server.exe` terminates immediately and automatically with zero orphaned processes remaining.

- [ ] **Check 6: Inactivity Idle Shutdown**
  - **Steps**:
    1. Set `idleMinutes: 1` in `wavical.config.js`.
    2. Start the app, send one generation, then leave idle for 70 seconds.
  - **Expected Result**: Engine automatically shuts down; memory is completely released back to the OS.

---

### WP5: Local Streaming Chat & Concurrency Limits
- [ ] **Check 7: Local Token Streaming**
  - **Steps**:
    1. Launch the example app: `.\target\debug\wavical dev examples/hello`.
    2. Enter a prompt (e.g. `"Explain quantum entanglement in two sentences"`).
    3. Click **Send Chat**.
  - **Expected Result**: Tokens appear progressively in the chat container in real-time.

- [ ] **Check 8: Cancel Mid-Stream**
  - **Steps**:
    1. Send a long prompt (e.g. `"Count from 1 to 500 with explanations"`).
    2. Click **Cancel** after 3 seconds.
  - **Expected Result**: Generation ceases immediately; UI reports cancellation; engine returns to idle without freezing.

- [ ] **Check 9: Generation Concurrency Gate**
  - **Steps**:
    1. Send a slow generation request.
    2. Immediately trigger another generation request from the browser devtools console: `window.wavical.invoke('ai.chat', { messages: [{ role: 'user', content: 'test' }] })`.
  - **Expected Result**: Second request fails immediately with generic error (`Method not allowed or not found`); active stream finishes cleanly.

---

### WP6: Semantic Indexation (RAG) & Vector Search
- [ ] **Check 10: Document Indexing**
  - **Steps**:
    1. From devtools or app UI, call `ai.index` with sample text documents across collection `"knowledge"`.
  - **Expected Result**:
    - SQLite database created at `%LOCALAPPDATA%\wavical\apps\<appId>\data\ai\knowledge.db`.
    - Documents chunked and embedded using `bge-small-en-v1.5`.

- [ ] **Check 11: Vector Similarity Search**
  - **Steps**:
    1. Execute `ai.search` with a conceptual query matching indexed text.
  - **Expected Result**: Returns top matching document chunks ranked by cosine similarity score.

---

### WP7: Cloud Fallback, Consent Dialog & Secret Management
- [ ] **Check 12: Native Consent Modal Dialog**
  - **Steps**:
    1. Set `ai.mode = "cloud"` in `wavical.config.js`.
    2. Configure a test endpoint in `ai.cloud.baseUrl`.
    3. Initiate an `ai.chat` call.
  - **Expected Result**:
    - Native Windows OS dialog appears with title: `Wavical - Cloud AI Service Consent`.
    - Shows host destination.
    - Clicking **Decline** aborts the call with generic error; zero network traffic sent.
    - On retry, clicking **Allow** persists consent and sends the request.

- [ ] **Check 13: Local-Mode No-Network Guarantee**
  - **Steps**:
    1. Set `ai.mode = "local"`.
    2. Disconnect Wi-Fi / Ethernet or block outbound connections via Windows Firewall.
    3. Send chat and search requests.
  - **Expected Result**: All local inference and vector searches function with 100% success offline.

- [ ] **Check 14: Windows Credential Manager Secret Storage**
  - **Steps**:
    1. Run: `.\target\debug\wavical secret set api.openai.com`
    2. Enter API key into the hidden terminal prompt (characters masked/hidden).
    3. Verify via Windows Credential Manager GUI (`control keymgr.dll`) that generic credential `wavical:api.openai.com` exists.
    4. Run: `.\target\debug\wavical secret clear api.openai.com`
    5. Verify the credential is removed.

---

### WP8: `wavical init --ai` Project Scaffolding
- [ ] **Check 15: AI App Generation & Capability Selection**
  - **Command**: `.\target\debug\wavical init my-ai-demo --ai "A markdown notes app with local vector search"`
  - **Expected Result**:
    - Generates safe files (`index.html`, `styles.css`, `app.js`).
    - Validates all generated files (no `.exe`, total size <= 1 MB, file count <= 20).
    - Inspects code and shows discovered capabilities: `ai.chat`, `ai.index`, `ai.search`.
    - Prompts developer to select capabilities (default none).
    - Writes safe `wavical.config.js`.

---

### WP9: Performance Benchmark Harness
- [ ] **Check 16: Automated Benchmark Execution**
  - **Command**: `.\target\debug\wavical ai bench --output bench_results.json`
  - **Expected Result**:
    - Executes warmup and 5 measured runs for each metric.
    - Measures cold start latency, tokens/sec, idle memory, engine working set, and RAG index latency (1k, 10k, 50k chunks).
    - Writes valid machine-readable JSON to `bench_results.json`.
    - Exits 0.

