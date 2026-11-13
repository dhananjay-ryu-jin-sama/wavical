# Releasing Wavical

This document describes Wavical's release lifecycle, canary-to-stable promotion, verification commands, rollback procedure, and signing-key compromise response.

---

## 1. Version Scheme

Wavical follows Semantic Versioning (`MAJOR.MINOR.PATCH`):

- **Canary pre-releases:** `MAJOR.MINOR.PATCH-canary.<run_number>` (for example, `0.5.0-canary.42`), tagged as `v0.5.0-canary.42` and published as a GitHub **pre-release**.
- **Stable releases:** `MAJOR.MINOR.PATCH` (for example, `0.5.0`), tagged as `v0.5.0` and published as a standard GitHub Release.
- **Monotonicity:** Versions are strictly monotonic. Never delete or reuse a published version number or tag, even after a rollback.

---

## 2. Cutting a Canary Release

Canary releases are built by [`.github/workflows/release-canary.yml`](../.github/workflows/release-canary.yml):

1. Triggered automatically on push to `main`, or manually via **Actions -> Release Canary -> Run workflow** (with `dry_run` defaulting to `true`).
2. The `verify` job runs the full verification gate before any artifact is built:
   - `cargo fmt --all --check`
   - `cargo clippy --workspace -- -D warnings`
   - `cargo build --workspace`
   - `cargo test --workspace`
   - `cargo test -p wavical-audit -- --include-ignored`
   - Supply-chain & updater/hardening tests (`runtime_tests`, `signer_tests`, `updater_tests`, `hardening_tests`)
   - `cargo audit` and `cargo deny check`
3. The `build-and-sign-canary` job compiles release binaries (`wavical.exe`, `wca.exe`, `wavical-shell.exe`), packages `wavical-v<version>-windows-x86_64.zip`, computes `SHA256SUMS.txt`, builds and signs `manifest.json` (`manifest.json.sig`), and verifies the signature.
4. When `dry_run` is `false` (on `push` to `main` unless `vars.RELEASE_DRY_RUN == 'true'`), the workflow generates a Sigstore build provenance attestation (`actions/attest-build-provenance`) and publishes a GitHub **pre-release**.

### Dry-Run Mode
- Set repository variable `RELEASE_DRY_RUN=true` or trigger `workflow_dispatch` with `dry_run: true`.
- In dry-run mode, the workflow generates an ephemeral keypair outside the repository, signs and verifies the manifest, uploads workflow artifacts, and **skips** provenance attestation and `gh release create`.

---

## 3. Promoting Canary to Stable

Stable promotion is **manual only** via [`.github/workflows/release-stable.yml`](../.github/workflows/release-stable.yml):

1. **Protected Environment Setup (One-Time):**
   - In GitHub **Settings -> Environments**, create `production` (and `canary` for canary signing).
   - Under **Deployment protection rules** for `production`, enable **Required reviewers** and add the repository maintainer.
   - Store `WAVICAL_RELEASE_SIGNING_KEY` as an **Environment secret** in `production` (never as a repo-wide secret or in source code) and `WAVICAL_RELEASE_PUBKEY` / `WAVICAL_CANARY_PUBKEY` as **Environment variables**.
2. **Run Promotion:**
   - Open **Actions -> Release Stable -> Run workflow**.
   - Enter `canary_tag` (e.g., `v0.5.0-canary.12`), `stable_version` (e.g., `0.5.0`), and `dry_run` (`true` to test first, `false` to publish).
   - Approve the `production` environment deployment prompt when notified.
3. The workflow downloads the canary assets, **re-verifies** `SHA256SUMS.txt` and `manifest.json.sig` against `WAVICAL_CANARY_PUBKEY`, packages and signs the `stable` manifest, generates build provenance attestation, and publishes `v<stable_version>`.

---

## 4. Rolling Back a Release

### Framework Release Rollback
1. **Do not delete or overwrite the bad tag.** Mark the faulty GitHub Release as a pre-release (or edit its release notes with a clear `[RECALLED]` notice).
2. Fix the regression on `main` and cut a **higher-semver** patch release (e.g., `0.5.1`) through the canary -> stable pipeline.

### App Updater Rollback (End-User Installs)
- Wavical's in-app launcher (`wavical-shell`) keeps the `previous` version directory on disk and tracks startup health in `health.json`.
- If a newly applied update fails to report ready within the health-check window for 2 consecutive launches (`MAX_UPDATE_HEALTH_FAILURES = 2`), the launcher automatically switches `current` back to `previous` and logs an `updater.rollback` event.
- To halt an active rollout server-side immediately, publish a higher-version manifest or set `rolloutPercent: 0` (re-signed with the release key), or let the manifest expire (<= 30 days max).

---

## 5. Verifying a Release

Download the release archive, `SHA256SUMS.txt`, `manifest.json`, `manifest.json.sig`, and verify locally:

```powershell
# 1. Verify SHA-256 checksum (PowerShell)
Get-FileHash .\wavical-v0.5.0-windows-x86_64.zip -Algorithm SHA256
Get-Content .\SHA256SUMS.txt

# 2. Verify Ed25519 manifest signature over exact manifest bytes
wavical release verify --pubkey "<BASE64_PUBLIC_KEY>" .\manifest.json

# 3. Verify GitHub Sigstore build provenance attestation
gh attestation verify .\wavical-v0.5.0-windows-x86_64.zip --owner <OWNER>
```

---

## 6. Signing Key Compromise Response

If a signing private key is suspected leaked or compromised:

1. **Generate a replacement keypair outside any repo/project:**
   ```powershell
   wavical signer keygen --out "$env:USERPROFILE\.wavical\keys\wavical-next.key"
   ```
   Copy ONLY the printed base64 public key. Never commit or log the private key.
2. **Rotate via the `"next"` key slot:**
   - Apps configured with `updater.publicKeys: [currentPubkey, nextPubkey]` accept manifests signed by either key.
   - Update `WAVICAL_RELEASE_SIGNING_KEY` in the protected `production` GitHub environment to the new key and update `WAVICAL_RELEASE_PUBKEY`.
   - Ship a new release signed by the replacement key that updates embedded `publicKeys` to `[newPubkey]` (removing the compromised key).
3. **Revoke & Announce:**
   - Remove the compromised key from all CI environments and `updater.publicKeys` lists.
   - Publish a security advisory on GitHub describing the affected key fingerprint/public key, the cut-off version/date, and instructions for users to verify downloads against the new public key.

