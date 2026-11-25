# Wavical App Updater Guide

Wavical's built-in updater (`updater.enabled: true`) provides cryptographically signed, staged, and automatically rolling-back updates for packaged Wavical apps.

---

## 1. Key Generation & Storage

Generate an Ed25519 signing keypair **outside** your project and git repository:

```powershell
wavical signer keygen --out "$env:USERPROFILE\.wavical\keys\myapp-release.key"
```

- `wavical signer keygen` refuses any path inside a git repository (`.git`) or Wavical project (`wavical.config.*`).
- It prints **only** the base64-encoded public key to stdout. Paste that public key into `updater.publicKeys` in `wavical.config.js`.
- **Never** commit the private key file, store it in app assets, or log it. Back it up in an encrypted password manager or HSM/protected CI environment.

---

## 2. Key Rotation (Current + Next Key)

`updater.publicKeys` accepts up to **2** embedded public keys (`[currentKey, nextKey]`):

1. Generate your next keypair offline with `wavical signer keygen --out ...`.
2. Ship an app update signed by `currentKey` whose `wavical.config.js` includes both `[currentKey, nextKey]`.
3. Once users have updated, begin signing new manifests with `nextKey`.
4. On the following release, drop the old key from `updater.publicKeys` (`[nextKey]`).

### Lost-Key Recovery
If a private key is **lost** (not compromised) and no `"next"` key was pre-embedded, existing installs will reject manifests signed by a new unknown key. Recovery requires distributing a new installer out-of-band (or via an OS/store update) that embeds the new public key. Always pre-embed a `"next"` key in `updater.publicKeys` for seamless recovery.

---

## 3. Building, Signing & Hosting Releases

1. Package your update archive (`.zip`) and plain-text release notes (max 8 KB).
2. Build `manifest.json`:
   ```powershell
   wavical release manifest `
     --payload .\myapp-1.2.0.zip `
     --version 1.2.0 `
     --channel stable `
     --expires 14 `
     --rollout 25 `
     --notes .\notes.txt `
     --url https://updates.example.com/myapp/stable/myapp-1.2.0.zip `
     --out .\manifest.json
   ```
3. Sign the exact bytes of `manifest.json`:
   ```powershell
   wavical release sign --key "$env:USERPROFILE\.wavical\keys\myapp-release.key" .\manifest.json
   ```
4. Verify before uploading:
   ```powershell
   wavical release verify --pubkey "<BASE64_PUBKEY>" .\manifest.json
   ```
5. Host `manifest.json`, `manifest.json.sig`, and `myapp-1.2.0.zip` over **HTTPS** on a host matching `updater.manifestUrl` (redirects off the allowlist or to `http://` are rejected).

---

## 4. Staged Rollout & Automatic Rollback

- **Rollout Bucket:** Each install generates a random persistent integer `0..=99` (`rollout-bucket`) once via OS CSPRNG and never transmits it. An install is eligible when `bucket < rolloutPercent`.
- **Atomic Staging:** Verified updates extract into `versions\<new>.staged\` (safe extractor, 1 GB / 20,000 file limit, no symlinks/traversal) and atomically rename to `versions\<new>\`.
- **Two-Strike Rollback:** On `updater.apply` or restart, `previous` is kept on disk and `current` switches atomically. If the new version fails to report ready for 2 consecutive launches (`MAX_UPDATE_HEALTH_FAILURES = 2`), the launcher automatically restores `current` to `previous` and logs `updater.rollback`.
- **Page API:** `updater.check` returns `{ available, version, notes }` and `updater.apply` applies a verified staged update. Neither accepts parameters from the page.

