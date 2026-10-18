# Wavical Config Schema — Phase 1 Reference

The config file is a JavaScript module that exports a default object. It's parsed by the CLI when you run `wavical dev` or `wavical build`.

## File location and format

- **Location:** `wavical.config.js` in the project root (same directory as `src/` and `dist/`)
- **Format:** JavaScript (ES modules), not JSON or TOML
- **Execution:** The CLI spawns Node to parse it: `node -e "console.log(JSON.stringify(await import('./wavical.config.js').then(m => m.default)))"`

## Root object schema

```javascript
export default {
  // Required fields
  appId: 'my-app',

  // Optional fields
  window: { /* see below */ },
  capabilities: { /* see below */ },
  dev: { /* see below */ },
}
```

### `appId` (required, string)

A unique identifier for your app. Used in window titles, data directories, and logs.

- Must be a valid identifier (alphanumeric, hyphens, underscores)
- Lowercase recommended
- Examples: `my-app`, `pdf-reader`, `chat_bot`

```javascript
appId: 'my-awesome-app'
```

### `window` (optional, object)

Window properties. Defaults: 1200×800, title = appId.

```javascript
window: {
  width: 1200,      // pixels (default 1200)
  height: 800,      // pixels (default 800)
  title: 'My App',  // window title (default: appId)
  resizable: true,  // default true (Phase 2+)
  minWidth: 400,    // minimum width (Phase 2+)
  minHeight: 300,   // minimum height (Phase 2+)
}
```

All fields are optional; omitted fields use defaults.

### `capabilities` (optional, object)

Declares which APIs your app can call. Phase 0 shipped with a hardcoded set; Phase 1 moves this into the config.

```javascript
capabilities: {
  'core.ping': true,   // granted
  'core.echo': false,  // explicitly denied (or omit to deny by default)
  // Phase 2 will add native APIs here:
  // 'fs.readFile': true,
  // 'fs.writeFile': true,
  // 'clipboard.read': false,
}
```

- **Default:** empty object (deny all)
- **Validation:** the CLI rejects unknown capability names (e.g., `core.echo2` → error: "unknown capability; did you mean 'core.ping'?")
- **Enforcement:** the IPC bridge in `wavical-shell` denies any method not declared (capability model)

**Known capabilities (Phase 0–1):**
- `core.ping` — returns "pong" and the framework version (used to test the IPC bridge)
- `core.echo` — echoes back the params you send (used to test denied capabilities)

### `dev` (optional, object)

Development server settings.

```javascript
dev: {
  port: 7890,  // dev server port (default 7890, used for hot-reload and future features)
}
```

- **Default:** port 7890
- **Why a port?** Future phases will use this for WebSocket-based hot-reload and a devtools panel.

## Examples

### Minimal config

```javascript
export default {
  appId: 'hello-world',
}
```

This uses all defaults: 1200×800 window, no capabilities, dev port 7890.

### Full config

```javascript
export default {
  appId: 'photo-editor',
  
  window: {
    width: 1400,
    height: 900,
    title: 'Photo Editor v1.0',
  },
  
  capabilities: {
    'core.ping': true,
    'fs.readFile': true,    // Phase 2: read files
    'fs.writeFile': true,   // Phase 2: write files
    'clipboard.read': false,
  },
  
  dev: {
    port: 8000,
  },
}
```

### Dynamic values (example)

Because the config is JavaScript, you can use computed values:

```javascript
export default {
  appId: process.env.APP_ID || 'my-app',
  
  window: {
    width: process.env.WINDOW_WIDTH ? parseInt(process.env.WINDOW_WIDTH) : 1200,
    title: `My App v${process.env.VERSION || '0.1.0'}`,
  },
  
  capabilities: {
    'core.ping': !process.env.DISABLE_CORE_APIS,
  },
}
```

## Validation rules

The CLI validates the config and rejects it with a clear error if:

1. **Missing `appId`:** "error: missing required field 'appId' in wavical.config.js"
2. **Invalid `appId`:** "error: appId must be alphanumeric (a-z, 0-9, -, _), got 'My App'"
3. **Unknown capability:** "error: unknown capability 'core.echo2' in capabilities; known: core.ping"
4. **Invalid data type:** "error: window.width must be a number, got string"
5. **JSON parsing error (from Node):** "error: failed to parse wavical.config.js: SyntaxError: ..."
6. **Node timeout:** "error: config parsing took too long (>5 seconds); check for infinite loops in wavical.config.js"

All errors are actionable and suggest fixes.

## Hot-reload behavior (Phase 1 `wavical dev`)

When you edit `wavical.config.js` during `wavical dev`:

- If the change is valid:
  - The config is reloaded
  - The capability set is updated in the IPC bridge
  - Your app continues running (no restart needed)
  - Message: "config changed, reloading... ✓ Config reloaded"

- If the change is invalid:
  - The old config is kept in memory
  - A warning is printed: "warning: config change invalid; keeping previous config"
  - Your app continues running with the old capabilities
  - Fix the config and save again; it will reload when valid

## Build behavior (Phase 1 `wavical build`)

When you run `wavical build`:

1. The config is validated
2. `src/` is copied to `dist/`
3. A manifest (`dist/wavical-manifest.json`) is generated with the config and asset hashes
4. The manifest is used by installers and updaters (Phases 4–5) to verify integrity

## Phases 2–6: upcoming capabilities

Future phases will add capabilities to this schema:

**Phase 2 (Native APIs):**
- `fs.readFile`, `fs.writeFile` — file access (gated via config)
- `clipboard.read`, `clipboard.write` — clipboard
- `notify.send` — notifications
- `menu.showContextMenu` — context menus
- `window.getState`, `window.setState` — window position/size persistence

**Phase 3 (AI):**
- `ai.search` — semantic search / RAG
- `ai.chat` — local/cloud chat model

**Phase 4 (Backend Runtime):**
- `backend.invoke` — call a Node/Bun backend function (if opted into)

**Phase 6 (Enterprise):**
- `audit.log` — emit an audit event (enterprise tier only)

## TypeScript support (future)

A future version may support `wavical.config.ts` for TypeScript projects. For now, use JSDoc comments in `.js` for IDE autocomplete:

```javascript
/** @type {import('wavical-config').AppConfig} */
export default {
  appId: 'my-app',
  // ...
}
```

(This requires the CLI to publish type definitions; coming in a later phase.)

## Environment variables and secrets

The config is **NOT** a good place for secrets (API keys, credentials). Instead:

1. Read from environment variables:
   ```javascript
   export default {
     appId: process.env.APP_ID || 'my-app',
     // ...
   }
   ```

2. Load from a `.env` file (use `dotenv` in your config file):
   ```javascript
   // In your config
   import dotenv from 'dotenv';
   dotenv.config({ path: '.env.local' });
   
   export default {
     appId: process.env.APP_ID,
     // ...
   }
   ```

3. Pass secrets to your app at runtime (via the IPC bridge, Phase 2+):
   - The config declares which APIs your app can call
   - At runtime, your app calls `wavical.invoke('api.getSecret', { name: 'AUTH_TOKEN' })`
   - The bridge retrieves the secret from a secure store (environment, `.env`, etc.) and returns it to the app
   - Secrets never live in the config file

## Common mistakes

### Forgetting to export

```javascript
// ❌ Wrong
const config = { appId: 'my-app' };

// ✅ Right
export default { appId: 'my-app' };
```

### Using invalid field names

```javascript
// ❌ Wrong: typo in field name
export default {
  appId: 'my-app',
  windowTitle: 'My App',  // should be window.title
}

// ✅ Right
export default {
  appId: 'my-app',
  window: { title: 'My App' },
}
```

### Capabilities as an array instead of object

```javascript
// ❌ Wrong
export default {
  appId: 'my-app',
  capabilities: ['core.ping', 'core.echo'],
}

// ✅ Right
export default {
  appId: 'my-app',
  capabilities: { 'core.ping': true, 'core.echo': false },
}
```

### Hardcoding secrets

```javascript
// ❌ Wrong: never put secrets in the config
export default {
  appId: 'my-app',
  apiKey: 'sk-1234567890abcdef',  // Don't do this!
}

// ✅ Right: use environment variables
export default {
  appId: 'my-app',
  // At runtime, call wavical.invoke('api.getSecret', { name: 'API_KEY' })
}
```
