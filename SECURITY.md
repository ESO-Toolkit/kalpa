# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Kalpa, please report it responsibly.

**Do not open a public issue.** Report privately on Discord — join at
<https://discord.gg/cMumdw6cSE> and message **`@spike_jones`**.

GitHub's private vulnerability reporting is **not currently enabled** on this
repository, so the "Report a vulnerability" button and the
`/security/advisories/new` URL will not work. Previous versions of this file
pointed there; Discord is the channel that actually reaches a maintainer.

Please include:

- A description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

Kalpa is maintained by one person in their spare time, so please treat the
timeline as best-effort rather than a guarantee: expect an acknowledgement
within about a week. If you have heard nothing after that, ping on Discord —
it means the report was missed, not ignored. We will work with you to
understand the issue and coordinate a fix before any public disclosure.

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Auditing and Dependencies

Kalpa has been reviewed internally, and the write-ups are in the repository so
you can judge them yourself rather than take a claim on trust:

- [`docs/audit-2026-07.md`](docs/audit-2026-07.md) — a full-repository review
  (July 2026) covering the Rust backend, the React frontend, the Pack Hub
  worker, and CI/CD. Includes the path-validation, ZIP-extraction and CSP
  analysis, with `file:line` evidence for every finding.
- [`docs/audits/log-uploader-audit.md`](docs/audits/log-uploader-audit.md) — a
  review of the ESO Logs uploader (July 2026). All 25 findings were implemented
  in PR #220.

These are **internal reviews by the project, not a third-party security audit**,
and they postdate the first 0.1.0 beta rather than gating it. Treat them as
evidence of what has been looked at and what was found — not as external
assurance.

Dependencies are audited
in CI on every push and pull request: `npm audit --omit=dev --audit-level=high`
for both the desktop client and the Pack Hub worker (production dependencies
only — the flagged advisories are in dev/build tooling that never ships), and
`cargo audit --file src-tauri/Cargo.lock` for the Rust crate graph.

Advisories that cannot yet be fixed upstream are assessed individually and
recorded, with their justification, in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml). At the time of writing
that is two transitive `quick-xml` advisories — `--ignore RUSTSEC-2026-0194
--ignore RUSTSEC-2026-0195`, a quadratic-runtime DoS on duplicate attribute
names — which are pinned below the fixed version by upstream Tauri crates and
are not reachable from Kalpa's code paths (build-time plist parsing and Windows
toast notifications, neither of which sees untrusted XML). The suppression list
is revisited whenever those crates bump.

A summary of the hardening measures is in the
[Security & privacy](README.md#security--privacy) section of the README.

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
