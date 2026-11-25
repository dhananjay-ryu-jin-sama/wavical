<div align="center">

# 🌊 Wavical

**A lightweight, security-first, AI-native desktop app framework built in Rust.**

*Build native desktop apps with HTML, CSS, and JavaScript using the OS WebView2 runtime, capability-gated IPC, local/cloud AI, optional sandboxed Node.js backends, and signed auto-updates.*

[![CI](https://github.com/dhananjay-ryu-jin-sama/Wavical/actions/workflows/ci.yml/badge.svg)](https://github.com/dhananjay-ryu-jin-sama/Wavical/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/rust-2024_edition-orange?logo=rust)
![Platform](https://img.shields.io/badge/platform-Windows-0078D6?logo=windows)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

[Installation](#-installation) •
[Quick Start](#-quick-start) •
[CLI Reference](#-cli-reference) •
[Configuration](#-configuration-wavicalconfigjs) •
[JavaScript API](#-javascript-api) •
[Roadmap](#-roadmap) •
[Documentation](#-documentation)

</div>

---

## 📦 Installation

### Prerequisites (Windows 10/11)
- **Node.js** `>= 14.8.0` (used to evaluate `wavical.config.js`)
- **Microsoft Edge WebView2 Runtime** (pre-installed on Windows 11)

### Option 1: Install via npm (Recommended)

```powershell
npm install -g wavical
```

Once installed, both `wavical` and the short alias `wca` are available globally:

```powershell
wavical --version
wca --help
```

### Option 2: Build from Source (Rust / Cargo)

Requires [Rust (stable)](https://rustup.rs/) and **Visual Studio C++ Build Tools**:

```powershell
git clone https://github.com/dhananjay-ryu-jin-sama/Wavical.git
cd Wavical

# Install the CLI binary (`wavical` and `wca`) into ~/.cargo/bin
cargo install --path crates/wavical-cli
```

### Verify What You Installed

Check that the CLI and shell binaries match the expected release version and git commit:

```powershell
npx wavical --version
npx wavical doctor --version-info
```

- `npx wavical --version` prints the release version and git commit (for example, `1.2.0 (<commit>)`).
- `npx wavical doctor --version-info` prints the CLI version/commit, resolved shell path, shell version/commit, and `Match: yes` when both binaries come from the same clean build.
- To inspect a built app's manifest `mode`, effective CSP, log path, and shell provenance, run `npx wavical doctor --app dist`.

### Dev Shell vs. Production Shell

Wavical separates developer conveniences from hardened production execution across two shell binaries:

| Binary | Used By | Capabilities & Hardening |
|---|---|---|
| `wavical-shell-dev.exe` (**dev shell**) | `wavical dev`, `wavical dev-safe`, `wavical dev-ultra-safe` | Dev-only conveniences: hot reload, live config reload, backend auto-restart, DevTools, `--record` event capture, `dev.url` support, and terminal blocked-resource diagnostics. Never copied into `dist/` builds. |
| `wavical-shell.exe` (**production shell**) | Built apps (`wavical build --allow-open`, `wavical build-safe`, `wavical build-ultra-safe`) | Phase 4 production hardening: no DevTools, no remote debugging, no `stdin` command channel, and no `dev.url`. Reads `mode` from `wavical-manifest.json` (verifies `allowlistSha256` before window creation in `safe` mode; falls closed to `ultra-safe` if `mode` is missing) and writes logs to `%LOCALAPPDATA%\Wavical\apps\<appId>\logs\shell.log`. |

### Network Modes & Browser Differences

Even in `open` mode, Wavical keeps baseline security boundaries that differ from a standard web browser (such as blocking plain `http://` unless listed in `localExceptions`, blocking `window.open` popups, and WebView2 tracking prevention for third-party embeds). See [Differences from a browser](docs/NETWORK_MODES.md#differences-from-a-browser) in [`docs/NETWORK_MODES.md`](docs/NETWORK_MODES.md).

---

## 🚀 Quick Start

### 1. Create a New App

```powershell
# Standard app template
wavical init my-app
cd my-app

# Or scaffold with AI assistance
wavical init my-ai-app --ai "Create a markdown note-taking app"

# Or scaffold with an optional Node.js backend (`backend/main.js`)
wavical init my-backend-app --backend
```

### 2. Run in Development Mode

Starts the hot-reload dev server and opens the native WebView2 shell:

```powershell
wavical dev
```

### 3. Build for Production

Bundles your frontend (and optional backend) into `dist/` with a SHA-256 asset manifest (`wavical-manifest.json`):

```powershell
wavical build --allow-open

# Or build with explicit allowlist / strict local-only isolation
wavical build-safe
wavical build-ultra-safe

# Bundle the verified Node.js backend runtime into the output directory
wavical build-safe --bundle-runtime
```

---

## 🛠️ CLI Reference

Both `wavical` and `wca` accept the same commands:

### Core Project Commands

| Command | Description |
|---|---|
| `wavical init <DIR> [--force] [--backend] [--ai "<prompt>"]` | Scaffold a new Wavical project in `<DIR>`. |
| `wavical dev [DIR]` | Start hot-reload development server in **open** mode and launch the desktop shell. |
| `wavical dev-safe [DIR] [--record]` | Start dev server in **safe** mode (`wavical.allowlist.json` + inline script hashes). |
| `wavical dev-ultra-safe [DIR] [--record]` | Start dev server in **ultra-safe** mode (strict local-only CSP; extracts inline scripts). |
| `wavical allowlist suggest [DIR]` | Review `--record` blocked events and selectively add origins to `wavical.allowlist.json`. |
| `wavical vendor [DIR] [--allow-unpinned <URL>] [--check] [--update]` | Vendor pinned CDN scripts, styles, fonts, and images into `src/vendor/` for offline & `ultra-safe` use. |
| `wavical build [DIR] --allow-open [--outdir dist] [--bundle-runtime]` | Build production assets in **open** mode (refuses without `--allow-open`) and generate `wavical-manifest.json`. |
| `wavical build-safe [DIR] [--outdir dist] [--bundle-runtime]` | Build production assets in **safe** mode (`wavical.allowlist.json` + SHA-256 integrity checks). |
| `wavical build-ultra-safe [DIR] [--outdir dist] [--bundle-runtime]` | Build production assets in **ultra-safe** mode (strict local-only CSP; extracts inline scripts). |
| `wavical doctor [--version-info] [--app <PATH>]` | Verify CLI/shell version & commit provenance or inspect a built app manifest. |
| `wavical register-filetypes [DIR]` | Register file extensions from `wavical.config.js` in Windows (`HKCU`). |
| `wavical unregister-filetypes [DIR]` | Remove registered Windows file associations for the project. |

### AI & Cloud Secret Commands

| Command | Description |
|---|---|
| `wavical ai doctor [DIR]` | Check CPU SIMD support (AVX2/FMA), RAM, disk space, and AI readiness. |
| `wavical ai bench [DIR] [--rag-only] [--runs 5]` | Run local chat and RAG vector store performance benchmarks. |
| `wavical model list` | List approved catalog GGUF models and local installation status. |
| `wavical model pull <ID> [--accept-license <ID>]` | Download and SHA-256 verify a model from Hugging Face. |
| `wavical model verify <ID>` | Verify SHA-256 hash and GGUF header of an installed model. |
| `wavical model remove <ID>` | Delete an installed model from local storage. |
| `wavical secret set <HOST>` | Save a cloud provider API key in the Windows Credential Store (e.g. `api.openai.com`). |
| `wavical secret clear <HOST>` | Remove a stored API key from the Windows Credential Store. |

### Node.js Backend Runtime Commands

| Command | Description |
|---|---|
| `wavical runtime list` | List approved pinned Node.js runtimes and local cache status. |
| `wavical runtime install <VERSION>` | Download, verify SHA-256, and cache a Node.js runtime (e.g. `22.11.0`). |
| `wavical runtime verify <VERSION>` | Verify cryptographic integrity of a cached runtime. |
| `wavical runtime remove <VERSION>` | Remove a cached runtime version. |
| `wavical runtime status [DIR]` | Inspect runtime cache status and project `backend` configuration. |

### Update Signing & Release Commands

| Command | Description |
|---|---|
| `wavical signer keygen --out <PATH>` | Generate an Ed25519 signing keypair (must be stored outside git repos). |
| `wavical release manifest --payload <FILE> --version <VER> --channel <CH> --expires <DAYS> --rollout <0-100> --notes <FILE>` | Generate an update `manifest.json` for a release payload. |
| `wavical release sign --key <KEY_PATH> <MANIFEST>` | Sign `manifest.json` with your private key to produce `manifest.json.sig`. |
| `wavical release verify --pubkey <BASE64_PUBKEY> <MANIFEST>` | Verify `manifest.json` and its `.sig` against an Ed25519 public key. |

---

## ⚙️ Configuration (`wavical.config.js`)

Every project is configured through a single `wavical.config.js` file in the project root. All capabilities are **default-deny** (omitted or `false` means blocked).

```javascript
/** @type {import('wavical-config').AppConfig} */
export default {
  appId: 'my-app',
  version: '0.1.0',

  window: {
    title: 'My App',
    width: 1200,
    height: 800,
    persistState: true,
  },

  singleInstance: true,

  // Explicitly grant only the IPC capabilities your app needs
  capabilities: {
    'core.ping': true,
    'clipboard.read': true,
    'clipboard.write': true,
    'notify.send': true,
    'dialog.openFile': true,
    'dialog.saveFile': true,
    'fs.readGranted': true,
    'fs.writeGranted': true,
    'menu.showContextMenu': true,
    'tray.set': true,
    'shortcut.register': true,

    // Requires `ai.enabled: true`
    'ai.chat': true,
    'ai.index': true,
    'ai.search': true,

    // Requires `backend.enabled: true`
    'backend.invoke': true,

    // Requires `updater.enabled: true`
    'updater.check': true,
    'updater.apply': true,
  },

  // Optional: Local / Cloud AI configuration
  ai: {
    enabled: true,
    mode: 'local', // 'local' | 'cloud' | 'local-first'
    local: {
      chatModel: 'qwen2.5-1.5b-instruct',
      embedModel: 'bge-small-en-v1.5',
      contextSize: 4096,
      maxTokens: 1024,
      idleMinutes: 10,
      threads: 'auto',
    },
  },

  // Optional: Sandboxed Node.js backend
  backend: {
    enabled: true,
    runtime: 'node',
    version: '22.11.0',
    entry: 'backend/main.js',
    methods: ['greet', 'compute'],
    timeoutSeconds: 30,
    permissions: {
      fsRead: [],
      fsWrite: [],
      childProcess: false,
      workers: false,
    },
  },

  // Optional: Signed auto-updates
  updater: {
    enabled: false,
    manifestUrl: 'https://updates.example.com/manifest.json',
    publicKeys: ['<BASE64_ED25519_PUBLIC_KEY>'],
    channel: 'stable', // 'stable' | 'beta' | 'canary'
    checkIntervalHours: 24,
    autoDownload: true,
  },

  dialog: {
    filters: {
      text: { name: 'Text Documents', extensions: ['txt', 'md', 'json'] },
    },
  },

  shortcuts: ['Ctrl+Shift+K'],
  fileAssociations: ['wca'],
  dev: {
    port: 7890,
  },
};
```

> [!WARNING]
> `wavical.config.js` is evaluated by Node.js during `wavical dev` and `wavical build`. Always inspect `wavical.config.js` before running CLI commands on untrusted third-party repositories.

---

## 🧩 JavaScript API

Wavical injects a frozen `window.wavical` object into your frontend.

### 1. Core & Native OS APIs

```javascript
// Health check
const ping = await window.wavical.invoke('core.ping');

// Clipboard
await window.wavical.invoke('clipboard.write', { text: 'Hello from Wavical!' });
const clip = await window.wavical.invoke('clipboard.read');

// Native desktop notification
await window.wavical.invoke('notify.send', {
  title: 'Export Complete',
  body: 'Your file has been saved.',
});

// Scoped File Access (Webview never sees raw file paths, only opaque grantIds)
const openRes = await window.wavical.invoke('dialog.openFile', {
  title: 'Select a note',
  filterId: 'text',
});

if (!openRes.cancelled) {
  const { content } = await window.wavical.invoke('fs.readGranted', {
    grantId: openRes.grantId,
  });
  console.log(content);
}

// Listen for native menu & global shortcut events
window.addEventListener('wavical:menu', (e) => console.log('Menu clicked:', e.detail.id));
window.addEventListener('wavical:shortcut', (e) => console.log('Shortcut pressed:', e.detail));
```

### 2. Local & Cloud AI (`ai.chat`, `ai.index`, `ai.search`)

```javascript
// Stream AI chat tokens
await window.wavical.invoke(
  'ai.chat',
  {
    messages: [{ role: 'user', content: 'Summarize Rust ownership in 2 sentences.' }],
    maxTokens: 256,
    temperature: 0.7,
    stream: true,
  },
  (chunk) => {
    // Always use textContent (never innerHTML) for model output
    document.getElementById('output').textContent += chunk;
  }
);

// Cancel an active generation in the current window
await window.wavical.cancel(1);

// Index documents into the embedded local SQLite vector store
await window.wavical.invoke('ai.index', {
  action: 'add',
  collection: 'notes',
  documents: [{ id: 'doc-1', text: 'Wavical uses WebView2 and default-deny capabilities.' }],
});

// Semantic similarity search
const results = await window.wavical.invoke('ai.search', {
  collection: 'notes',
  query: 'How are permissions handled?',
  topK: 3,
});
```

### 3. Invoking the Node.js Backend (`backend.invoke`)

```javascript
// Calls a method allowlisted in `backend.methods` in wavical.config.js
const res = await window.wavical.invoke('backend.invoke', {
  method: 'compute',
  params: { values: [10, 20, 30] },
});
```

### All Supported Capabilities

| Capability | Parameters | Description |
|---|---|---|
| `core.ping` | `none` | Health check (`{ message: "pong", version }`). |
| `core.echo` | `any` | Echoes input parameters back. |
| `clipboard.read` | `none` | Reads text from system clipboard (max 1 MB). |
| `clipboard.write` | `{ text: string }` | Writes text to system clipboard (max 1 MB). |
| `notify.send` | `{ title: string, body?: string }` | Sends native OS notification (rate-limited: 5 / 10s). |
| `dialog.openFile` | `{ title?: string, filterId?: string }` | Opens file dialog; returns `ReadOnly` `grantId`. |
| `dialog.saveFile` | `{ title?: string, defaultName?: string, filterId?: string }` | Opens save dialog; returns `WriteOnly` `grantId`. |
| `fs.readGranted` | `{ grantId: string }` | Reads UTF-8 file using a `ReadOnly` `grantId` (max 10 MB). |
| `fs.writeGranted` | `{ grantId: string, content: string }` | Atomically writes UTF-8 text using a `WriteOnly` `grantId` (max 10 MB). |
| `menu.showContextMenu` | `{ items: MenuItem[] }` | Displays native popup context menu at cursor (max 200 items). |
| `tray.set` | `{ tooltip?: string, items?: MenuItem[] }` | Configures system tray icon tooltip and menu items. |
| `shortcut.register` | `{ shortcut: string } \| { shortcuts: string[] }` | Registers global keyboard shortcuts (max 32). |
| `badge.set` | `{ count: number \| null }` | Sets taskbar badge count (`0..=99`). |
| `ai.chat` | `{ messages, maxTokens?, temperature?, stream? }` | Runs local GGUF or cloud chat completion (also enables `ai.cancel`). |
| `ai.index` | `{ action: 'add' \| 'replace' \| 'delete', collection, documents? }` | Indexes or deletes documents in the local RAG store. |
| `ai.search` | `{ collection: string, query: string, topK?: number }` | Searches local RAG collection by cosine similarity (`topK` 1–20). |
| `backend.invoke` | `{ method: string, params?: any }` | Invokes an allowlisted method in the Node.js backend process. |
| `updater.check` | `none` | Checks configured `manifestUrl` for a signed update. |
| `updater.apply` | `none` | Stages and applies a verified update with health-check rollback. |

---

## 🗺️ Roadmap

| Phase | Focus | Status |
|:-:|---|:-:|
| **0** | Foundation shell (`tao`/`wry`), `wavical://` protocol, secure IPC, CSP | ✅ Completed |
| **1** | `wavical` / `wca` CLI (`init`, `dev`, `build`), `wavical.config.js`, hot-reload | ✅ Completed |
| **2** | Native Tier 1 APIs: scoped file grants, clipboard, notifications, tray, menus, shortcuts | ✅ Completed |
| **3** | AI-native: local GGUF engine, cloud fallback + consent, local RAG (`ai.index`/`ai.search`) | ✅ Completed |
| **4** | Opt-in sandboxed Node.js backend runtime (`wavical runtime`), Ed25519 signed updates | ✅ Completed |
| **4.5** | Network Modes (`open` / `safe` / `ultra-safe`), `wavical vendor`, `allowlist suggest`, Remote UI (`window.remoteUrl`) | In progress |
| **5** | Windows installers (`.exe`/`.msi`), code signing, portable builds, public benchmarks | 📋 Planned |
| **6** | Enterprise: structured audit logging, SBOM generation, `wavical audit`, private registries | 📋 Planned |

Full breakdown: [`ROADMAP.md`](ROADMAP.md)

---

## 📄 License

Dual-licensed under either of **MIT License** or **Apache License, Version 2.0**, at your option.
