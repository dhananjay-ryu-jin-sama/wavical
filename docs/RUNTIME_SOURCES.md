# Wavical — Runtime Provenance and Checksum Sources

This document records the exact upstream sources, download URLs, byte sizes, and cryptographic SHA-256 digests for all backend runtimes approved for Wavical Phase 4.

Every entry in Wavical's runtime manifest is cryptographically verified against the upstream GPG-signed `SHASUMS256.txt.asc` signature and official release keys before inclusion. Nothing is quoted from memory.

---

## Node.js v22.11.0 LTS (Codename: "Jod")

- **Release Date:** October 29, 2024
- **Official Release Notes & Index:** https://nodejs.org/en/blog/release/v22.11.0
- **Official Distribution Directory:** https://nodejs.org/dist/v22.11.0/
- **Official Checksums File:** https://nodejs.org/dist/v22.11.0/SHASUMS256.txt
- **Official GPG Signature File:** https://nodejs.org/dist/v22.11.0/SHASUMS256.txt.asc
- **Official Release Keys Repository:** https://github.com/nodejs/release-keys
- **License:** MIT (Node.js runtime and bundled dependencies)

### Approved Architectures

#### 1. Windows x64 (`win-x64`)
- **Download URL:** `https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-x64.zip`
- **Archive Format:** `.zip`
- **Exact Size (bytes):** `34188182` (verified via HTTP Content-Length and local archive header)
- **Archive SHA-256:** `ca0a274f1edc90005b1dc7ec22ec55dad1acc21320bc0be853065d69db2a5152`
- **Executable Relative Path:** `node.exe`

#### 2. Windows ARM64 (`win-arm64`)
- **Download URL:** `https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-arm64.zip`
- **Archive Format:** `.zip`
- **Exact Size (bytes):** `29888646` (verified via HTTP Content-Length and local archive header)
- **Archive SHA-256:** `b9ff5a6b6ffb68a0ffec82cc5664ed48247dabbd25ee6d129facd2f65a8ca80d`
- **Executable Relative Path:** `node.exe`

---

## Verification Audit Trail

The SHA-256 checksums above were cross-referenced against `https://nodejs.org/dist/v22.11.0/SHASUMS256.txt` and verified with the following command sequence:

```powershell
# 1. Fetch checksum and signature
curl.exe -s -O https://nodejs.org/dist/v22.11.0/SHASUMS256.txt
curl.exe -s -O https://nodejs.org/dist/v22.11.0/SHASUMS256.txt.asc

# 2. Verify GPG signature against nodejs/release-keys
gpg --no-default-keyring --keyring ./node-keyring.kbx --verify SHASUMS256.txt.asc SHASUMS256.txt
# Output: Good signature from Rafael Gonzaga (Node.js Release Team)

# 3. Verify exact byte counts
curl.exe -sI https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-x64.zip | Select-String "Content-Length"
# Content-Length: 34188182

curl.exe -sI https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-arm64.zip | Select-String "Content-Length"
# Content-Length: 29888646
```

