# Security Policy

Wavical is engineered with a security-first architecture. We take all vulnerability reports seriously and appreciate responsible disclosure.

---

## Supported Versions

Wavical is currently under active pre-release development. Security fixes are applied directly to the main development branch.

| Version | Supported | Notes |
|:---|:---|:---|
| Pre-release (Phase 0 / main) | :white_check_mark: | Active development |
| < 0.1.0 | :x: | Not yet released |

---

## Reporting a Vulnerability

Please **DO NOT** report security vulnerabilities via public GitHub issues, discussions, or pull requests.

### Preferred Reporting Method
1. **GitHub Private Vulnerability Reporting**: Use the "Report a vulnerability" button under the **Security** tab of the GitHub repository.
2. **Email**: If GitHub Private Vulnerability Reporting is unavailable, send an encrypted report to our security contact:
   - **Contact Email**: `security@example.com` <!-- ACTION REQUIRED: REPLACE THIS PLACEHOLDER WITH YOUR REAL SECURITY CONTACT EMAIL -->
   - If using PGP/GPG, include your public key for encrypted communications.

### What to Include in Your Report
To help us triage and resolve the issue quickly, please provide:
- A clear description of the vulnerability and potential impact.
- Affected component(s) (`wavical-ipc`, `wavical-protocol`, `wavical-shell`, etc.).
- Minimal reproducible proof of concept (PoC), script, or attack payload.
- Any known mitigations or workarounds.

---

## Response Process & Commitments
- **Initial Acknowledgment**: We aim to acknowledge vulnerability reports within 48 hours.
- **Triage & Assessment**: We will validate the severity and discuss the disclosure timeline.
- **Remediation**: A security patch will be prepared and tested against our security test bench.
- **Public Disclosure**: Once fixed and deployed, coordinated public disclosure will follow with attribution to the reporter (unless anonymity is requested).
