# Wavical — Phase 3 Build Plan: AI-Native Infrastructure

**Goal:** let a Wavical app use AI safely: a local model (llama.cpp / GGUF), an optional cloud fallback, and local search over the app's own documents (RAG). All of it is opt-in, default-deny, capability-gated, and reachable from the page only through the Phase 0 bridge.

**Why this phase carries the most risk so far:** it adds untrusted model files, a native inference engine, outbound network traffic, API keys, and multi-gigabyte downloads. Every one of those needs a rule before code is written.

**Estimate:** 4–6 weeks part-time with AI. The two slowest parts are the decision gate in WP1 and the low-end hardware benchmark in WP9.

---

## 0. Gate before starting

Do not start Phase 3 until:
- [ ] The Phase 0–2 audit summary (A6) has **no open Critical or High findings** (or each is explicitly accepted by you in writing).
- [ ] `Cargo.lock` is committed and CI is green on GitHub.
- [ ] The Phase 2 manual Windows checklist has been run and recorded.

Phase 3 adds heavy dependencies. Starting on a shaky base makes every later problem harder to trace.

---

## 1. Decisions locked before code

### 1.1 Principles (not negotiable inside this phase)

1. **Model output is untrusted data.** Rust never interprets it as a command, path, URL, capability, SQL, or script. No tool calling, no function calling, no agents in Phase 3.
2. **The page never triggers a download, never picks a model file, never supplies a URL or an API key.** The page can only call the AI methods the config grants. Downloads happen only through the CLI, run by a developer.
3. **Local mode never touches the network.** If `mode` is `local`, there must be no outbound connection from Wavical's AI code. This is tested.
4. **Secrets never leave Rust.** API keys are never in the config file, the page, logs, command-line arguments, child-process environments, error text, window state, or the index.
5. **Prompts and outputs are never logged.** Logs hold sizes, ids, durations, backend and model id only. Prompts are also never put in process command lines (other processes can read those).
6. **User data leaves the machine only when `cloud` is explicitly enabled in config AND the first-use consent was given.** The page is told which backend answered.
7. **Wavical does not bundle, mirror or re-host model weights.** It ships a **catalog of pointers** (name, upstream URL, size, SHA-256, license name, license link). Weights come directly from the upstream source. The developer or company using a model carries that model's license responsibility, and the CLI states this plainly.
8. **No `unsafe` written by us.** Crates that wrap unsafe code internally are allowed. If a feature needs our own `unsafe`, stop and ask.

### 1.2 Process isolation (default)

GGUF files are untrusted input parsed by C++ code. The default architecture is: **the inference engine runs as a separate OS process** started by Wavical. It is:
- started lazily on the first AI call (not at app start),
- shut down after an idle period (config, default 10 minutes),
- killed when the app exits **even if the app crashes** (WP4 picks the mechanism, for example a Windows job object through a crate that wraps it),
- given a cleaned environment (no inherited secrets) and only the channel it needs,
- reachable only by Wavical (stdio or a named pipe restricted to the current user; if loopback TCP is the only option, it needs a random port plus a per-session random token, and WP1 must justify it).

WP1 compares this with running the engine inside the shell process through a Rust binding crate, and with a pure-Rust engine. **This is a decision gate: the agent presents the options and stops. You decide.** The recommendation is process isolation, unless WP1 finds a concrete blocker.

### 1.3 Capabilities (new; added to the registry in WP2)

| Capability | What it allows |
|---|---|
| `ai.chat` | send a chat request, receive a streamed answer, cancel your own request |
| `ai.index` | add documents to, replace, or delete a collection in this app's local index |
| `ai.search` | query this app's local index |

Model management (list, pull, remove, verify), cloud settings and key storage are **CLI and config only**, never a page capability.

Config validation rule: any `ai.*` capability is an error unless `ai.enabled` is `true`.

### 1.4 Config section (added to the Phase 1 validator in WP2)

```javascript
ai: {
  enabled: false,                       // default false; everything below is ignored when false
  mode: 'local',                        // 'local' | 'cloud' | 'local-first'  (default 'local')
  local: {
    chatModel: 'catalog-id',            // must exist in the catalog and be installed
    embedModel: 'catalog-id',           // needed for ai.index / ai.search
    contextSize: 4096,                  // hard ceiling in code (see 1.5)
    maxTokens: 1024,                    // default per answer, hard ceiling in code
    idleMinutes: 10,
    threads: 'auto',                    // 'auto' or a number within a sane range
  },
  cloud: {                              // used only by 'cloud' and 'local-first'
    protocol: 'openai-compatible',      // WP1 decides the supported protocols
    baseUrl: 'https://api.example.com/v1',   // https only; this host is the only host ever contacted
    model: 'provider-model-name',
    apiKeyEnv: 'MY_PROVIDER_API_KEY',   // NAME of an environment variable, never the key itself
    requireConsent: true,               // first-use native consent dialog (default true)
  },
}
```

Validator rules: unknown keys rejected; `baseUrl` must be `https://` with a host and no userinfo; a value that looks like an API key in `apiKeyEnv` is rejected (it must be a variable name, `[A-Z][A-Z0-9_]*`); models must exist in the catalog.

**Cloud fallback meaning:** `local-first` tries local; it falls back to cloud **only** when local is unavailable (model missing, not enough memory, engine failed), cloud is configured, and consent was given. A content refusal or a slow answer is not a reason to fall back. Every cloud request emits a tracing event.

### 1.5 Named limits (constants in code, each with a test; initial values, change only with your approval)

| Area | Limit | Value |
|---|---|---|
| Chat | messages per request | 50 |
| Chat | characters per message | 32,000 |
| Chat | total characters per request | 128,000 |
| Chat | roles allowed | `system`, `user`, `assistant` |
| Chat | `maxTokens` hard ceiling | 4,096 |
| Chat | temperature range | 0.0 to 2.0 |
| Chat | concurrent generations per window | 1 (extra requests are rejected, not queued) |
| Chat | wall-clock timeout | 120 s |
| Chat | streamed output cap | 100,000 characters |
| Local | `contextSize` hard ceiling | 8,192 |
| Index | collection name | `[a-z0-9._-]{1,64}` |
| Index | collections per app | 16 |
| Index | documents per `ai.index` call | 100 |
| Index | document id | `[A-Za-z0-9._:-]{1,128}` |
| Index | characters per document | 100,000 |
| Index | metadata per document | 4 KB, JSON, string/number/bool values only |
| Index | chunks per collection | 50,000 |
| Index | index file size per app | 512 MB |
| Search | query length | 4,000 characters |
| Search | `topK` | 1 to 20 |
| Cloud | request body | 1 MB |
| Cloud | response body (non-streamed) | 4 MB |
| Cloud | connect / total timeout | 10 s / 120 s |
| Cloud | redirects | none followed |
| Download | file size | must equal the catalog size exactly |
| Download | concurrent downloads | 1 |

### 1.6 Model catalog and storage

- Catalog is a JSON file in the repo. Each entry: id, display name, kind (`chat` or `embedding`), file name, upstream URL (HTTPS), size in bytes, SHA-256, license name, license URL, minimum RAM, notes. **Every value comes from the upstream page, with a source link recorded in `docs/WAVICAL_PHASE_3_DECISIONS.md`. Nothing from memory.**
- Models are stored in a shared per-user cache, `%LOCALAPPDATA%\Wavical\models\`, so several apps reuse one copy.
- Pull flow: show model name, size, license name and link, and the line "Wavical does not provide or endorse this model. You are responsible for its license." Require an explicit confirmation: typing the model id, or `--accept-license <exact-license-id>` for scripts. There is **no blanket `--yes`**. Record the acceptance (time, model id, license id, catalog version) in a local file.
- Download rules: HTTPS only; host allowlist taken from the research in WP1 (including the hosts upstream redirects to); no redirect to a host outside the allowlist; temp file with `.part`; free-disk check; resume allowed; SHA-256 verified before the file moves into the cache (atomic rename); on mismatch delete the file and fail; the target file name is built from the catalog id after validation (no path traversal).
- GGUF sanity: check the magic bytes and version header before first load. The engine process does the real parsing.
- Recorded size and modification time are checked at load. `wavical model verify` recomputes the hash.

### 1.7 Engine runtime provenance

The engine (for example llama.cpp's server binary) is code that Wavical runs. It must come from a pinned, verified source. WP1 proposes the exact approach (build as part of the workspace, or download a pinned official release with a pinned SHA-256). The binary's hash is checked **before every launch**. No engine binary is committed to git.

### 1.8 Storage and privacy of the index

The index lives in the app's data folder, per appId. It is **not encrypted at rest** in Phase 3 (document this plainly; encryption is a later item). Collections are per app. Another app cannot read them through Wavical because the path is built from the validated appId.

### 1.9 Platform and hardware target

Windows only. Target machine for the benchmark and for the default model choices: **8 GB RAM, no GPU**. GPU acceleration is out of scope. The default catalog entries must run on the target machine.

---

## 2. Scope

**In scope**
- Research and decisions (WP1)
- `wavical-ai` crate, config schema, capabilities, pure validation (WP2)
- Model catalog and `wavical model` CLI with license flow (WP3)
- Local engine process manager (WP4)
- `ai.chat` with streaming and cancel (WP5)
- RAG: `ai.index`, `ai.search` (WP6)
- Cloud fallback (WP7)
- `wavical init --ai` (WP8, optional, may be deferred)
- Benchmark on low-end hardware (WP9)
- Integration, test bench, self-review (WP10)

**Out of scope**
- GPU acceleration, multimodal (images/audio), fine-tuning, model training
- Tool calling, function calling, agents, code execution from model output
- Encryption of the index at rest
- In-app UI for end users to type an API key (Phase 3 supports an environment variable and the Windows credential store set by `wavical secret set`, if WP1 picks that crate; an end-user key-entry flow is a later item)
- Custom or private catalogs, `model add` for arbitrary local files
- Node/Bun backend, updater, installers (Phases 4–5)
- macOS and Linux behaviour

---

## 3. Work packages (do in order, one per session)

### WP1: Research and decisions (NO feature code, decision gate)

Record in `docs/WAVICAL_PHASE_3_DECISIONS.md`, each row with a source link and the version `cargo add` or docs.rs shows (never from memory):

1. **Engine integration options:** (a) separate llama.cpp server process, (b) Rust binding crate inside the shell process, (c) a pure-Rust engine. For each: how it is built or obtained, size, whether it needs a C++ toolchain, license, maintenance activity, how streaming works, how embeddings work, security notes. Recommend one.
2. **Engine provenance and pinning** (see 1.7). Exact release or commit, hash source, CPU variants (does a CPU-only build exist that runs on older CPUs?).
3. **Process lifetime control:** how to guarantee the engine dies with the app on Windows, using a crate that wraps it (no unsafe of ours).
4. **Channel to the engine:** stdio, named pipe or loopback TCP, with the security consequences of each.
5. **Embeddings:** which engine feature, which kind of model, vector dimension handling.
6. **Vector store:** a simple approach first (for example SQLite through a crate that bundles it, vectors as blobs, brute-force cosine in Rust) versus a vector crate. Judge by safety, size, speed at 50,000 chunks.
7. **HTTP client and TLS** for downloads and cloud calls (certificate verification on, no option that disables it, proxy handling).
8. **Credential storage:** a crate that wraps the Windows credential store, if any, and whether it needs our own unsafe.
9. **Cloud protocols:** which to support in Phase 3 (OpenAI-compatible, Anthropic Messages, others). Recommend the smallest set.
10. **Model candidates:** two or three chat models and two embedding models that run on 8 GB RAM, no GPU. For each: parameter size, quantization, file size, minimum RAM, license, upstream URL, SHA-256 from the upstream page, and the redirect hosts the download uses. **Prefer permissive licenses. Do not choose for me: propose and wait.**
11. **Benchmarks from upstream** (if they publish any) for the candidates on CPU-only machines, with links.

**Done when:** the decisions file is complete, the agent has listed the choices that need me (items 1, 4, 9, 10), and has **stopped**. Do not start WP2.

### WP2: `wavical-ai` crate, config, capabilities, pure validation

- Create `wavical-ai` (forbid unsafe; no engine yet).
- In `wavical-ipc`: add `ai.chat`, `ai.index`, `ai.search` to the registry and capability list; add pure request types and validators for chat, index and search with every limit in 1.5; the method-enumeration test is updated.
- Update the Phase 1 config validator for the `ai` section and the rule "ai.* capability requires ai.enabled". Include the `apiKeyEnv` and `baseUrl` rules.
- Add tracing event names and field names for AI calls (started, finished, denied, cancelled, failed) with window id, method, backend, model id, sizes, duration. Never content.

**Done when:** tests prove each limit at the boundary (limit passes, limit+1 fails, multibyte text), capability denial never reaches a handler, config rules reject bad input with clear messages.

### WP3: Catalog and `wavical model` CLI

- Catalog parser and validator (pure): unknown fields rejected, URL must be HTTPS and on the allowlist, ids and file names restricted to a safe charset, hash format checked.
- Commands: `wavical model list`, `pull <id>`, `remove <id>`, `verify <id>`. Add `wavical ai doctor` basics later in WP4.
- License acknowledgment flow exactly as in 1.6; acceptance record file.
- Download rules as in 1.6. Tests use a **local mock HTTP server and a tiny fake model file**. CI never downloads a real model.

**Done when:** tests cover tampered file, truncated file, wrong size, redirect to a non-allowlisted host, HTTP URL, path traversal in ids, license prompt bypass attempts, interrupted and resumed download. Manual check: one real pull on your machine.

### WP4: Local engine process manager

- Obtain/verify the engine binary per WP1 (hash checked before every launch).
- Lifecycle: lazy start, idle shutdown, crash recovery with a retry limiter (reuse the Phase 2 pattern: pure decision function, monotonic clock), kill on parent exit including a crash of the parent, clean environment, no prompts in command lines.
- Channel per WP1. Health check and version handshake.
- `wavical ai doctor`: reports RAM, CPU features, engine present and hash OK, models installed, and whether the target limits are met.
- Tests use a **fake engine executable** (a tiny test binary that speaks the same protocol) so lifecycle logic is tested without a real model.

**Done when:** tests cover start, idle stop, crash and retry limit, hash mismatch refuses to launch, orphan check (kill the parent hard, the engine must be gone). Manual: Task Manager check.

### WP5: `ai.chat` (local)

- Validate with the pure validators from WP2, then call the engine.
- Streaming to the page through the Phase 0 safe delivery (JSON-escaped data, never string concatenation). Cancel by request id, only for requests that window started.
- Enforce concurrency, timeout, token cap and output cap. Errors to the page are generic; details are logged.
- No content in logs.
- Hello example gets a chat box (text only, rendered with `textContent`, never `innerHTML`).

**Done when:** tests with the fake engine cover limits, cancel races, timeout, engine crash mid-stream, output containing `</script>` and HTML (page receives data only). Manual: a real chat with a real model.

### WP6: RAG (`ai.index`, `ai.search`)

- Pure chunker (deterministic, bounded, property-tested: never panics, chunks cover the text, no chunk over the limit).
- Embeddings through the engine; the index records embedding model id and dimension. If the configured embedding model changes, search returns a clear "reindex required" error instead of wrong results.
- Vector store per WP1; parameterized queries only (no SQL built from strings); corrupt or truncated store is detected and reported, not trusted.
- Replace and delete collection through `ai.index`. Size and count limits from 1.5.
- Search returns ids, scores, text and metadata of the top hits.

**Done when:** tests compare search against a simple reference implementation on random data, cover every limit, collection-name and id injection attempts (`../`, `con`, quotes, SQL), a corrupt store file, and two apps with different appIds not seeing each other's data.

### WP7: Cloud fallback

- HTTPS client per WP1 with certificate verification on. Only `baseUrl`'s host is ever contacted. No redirects. Size and time limits from 1.5. Proxy behaviour documented.
- Key source: the environment variable named in config (and the credential store if WP1 chose it, with `wavical secret set` and `clear`). Read in Rust at call time, never cached in the page, config, logs, errors or child environments.
- First-use native consent dialog (provider host, "your text will be sent to this service"); acceptance stored per app and host. A page cannot trigger or skip it.
- Modes `local`, `cloud`, `local-first` exactly as in 1.4. Response carries `backend: "local" | "cloud"`.
- Tests with a **mock HTTPS server**. Required tests: in `local` mode the mock sees zero requests; the key never appears in any page response, log, error, process arguments or child environment (use a marker key); redirect to another host is refused; HTTP base URL is refused; consent cannot be bypassed.

**Done when:** all of the above pass. Manual: one real cloud call with your own key.

### WP8: `wavical init --ai` (optional; may be deferred without blocking Phase 3)

- Generates `src/` files from a text description using the configured backend.
- Generated content is untrusted: only `.html`, `.css`, `.js`, `.json`, `.svg`, `.txt` files; every path validated with the same rules as `wavical init`; file count and size limits; nothing is executed; files are written only inside the new project folder.
- **The model never decides capabilities.** The CLI shows the capabilities the app seems to need and the developer selects them. Default is none.
- Tests: path traversal and reserved names in generated paths, disallowed extensions, oversized output, capability auto-grant attempts.

### WP9: Low-end hardware benchmark

- Add a benchmark harness (a small example app or CLI command) that measures: cold start to first token, tokens per second, peak working set of engine and shell, idle memory with AI enabled but unused, memory after idle shutdown, index build time and search latency at 1,000, 10,000 and 50,000 chunks.
- You run it on the **8 GB RAM, no GPU** machine. The agent writes `docs/AI_BENCHMARK.md` with the machine spec, method and results, and proposes budgets from the measured numbers. You approve the budgets.
- Required by the feature checklist before Phase 3 can be called done.

### WP10: Integration, test bench, self-review

- Extend the attack page for AI: calls without capability, oversize and malformed requests, forged collection names, cross-app collection names, flood of chat/index/search calls, cancel races, model output with markup, attempts to supply a model path, URL or key from the page.
- Fuzz targets: AI request parsers, catalog parser, chunker. Document the nightly WSL commands.
- Mutation proofs for the new rules (see the A7 audit prompt for the list).
- Run the full Phase 0–2 regression (attack page, audit crate).
- CI: Windows job builds `wavical-ai`; Linux job runs the pure-crate and catalog/chunker tests. No real model in CI.
- Update docs: SECURITY_MODEL.md (AI section), config schema, privacy notes (what leaves the machine, what is stored).
- Hostile self-review of the engine launcher, downloader, cloud client, key handling and index code.

**Done when:** everything passes and a combined manual Windows checklist is written.

---

## 4. Manual checks (collected in WP10)

Each WP lists its manual checks. The agent cannot run a real model in CI. State clearly which checks were automated and which were not run.

## 5. Not needed in Phase 3

GPU support, multimodal models, tools and agents, index encryption, end-user key-entry UI, custom catalogs, installers, updater, Node/Bun.
