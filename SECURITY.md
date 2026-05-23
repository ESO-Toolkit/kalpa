# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Kalpa, please report it responsibly.

**Do not open a public issue.** Instead, use [GitHub's private vulnerability reporting](https://github.com/ESO-Toolkit/kalpa/security/advisories/new) to submit your report.

Please include:

- A description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

You should receive a response within 48 hours. We will work with you to understand the issue and coordinate a fix before any public disclosure.

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Auditing and Dependencies

The 0.1.0 beta shipped after a comprehensive security audit covering path
validation, ZIP handling, CSP, and the Pack Hub worker. All dependencies are
regularly checked against current CVE databases via `npm audit` and
`cargo audit`; as of May 2026, no known vulnerabilities are present. A summary
of the hardening measures is in the [Security & privacy](README.md#security--privacy)
section of the README.

## Scope

The following are in scope:

- Path traversal or arbitrary file access via Tauri IPC commands
- ZIP extraction vulnerabilities (zip bombs, symlink attacks)
- Cross-site scripting (XSS) in the webview
- CSP bypasses
- Dependency vulnerabilities with known exploits

Out of scope:

- Issues requiring physical access to the machine
- Social engineering attacks
- Denial of service against ESOUI (rate limiting is already implemented)
