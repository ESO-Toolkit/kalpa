# Changelog

All notable changes to Kalpa are documented here. This project uses [Conventional Commits](https://www.conventionalcommits.org/).

## [Unreleased]

## [0.1.0-beta.3] — 2026-05-25

### Bug Fixes
- Add crash recovery for metadata writes — if the app crashes mid-save, the completed `.tmp` file is now recovered on next load instead of falling back to stale data
- Add missing `addon.required` validation in Pack Hub share code creation
- Log warnings instead of silently ignoring metadata save failures during scan and update check

### Improvements
- Wrap `DiscoverResultRow` in `React.memo` for smoother list scrolling
- Add `aria-hidden` to decorative SVGs for screen reader accessibility

### Documentation
- Add "Security & privacy" section to README with full trust story
- Add download verification guide and `.esopack` v2 settings-export documentation
- Add beta feedback issue template
- Expand changelog with security hardening, feature, and testing highlights

### Testing & CI
- Add worker `npm audit` to CI pipeline (was only running for frontend)
- Add crash-recovery unit tests for metadata `.tmp` file promotion
- Fix clippy `approx_constant` deny in parser test

## [0.1.0-beta.2] — 2026-05-23

### Bug Fixes
- Hide batch action controls in the Discover tab and harden list selection state

### Documentation
- Mark the README as Beta and add a "Security & privacy" section
- Document `.esopack` v2 privacy scrubbing and how to verify downloads
- Expand the beta changelog and add a beta-feedback issue template

### Internal
- Bump CI Node.js 20 → 22 for wrangler 4.93 compatibility

## [0.1.0-beta.1] — 2026-05-23

First beta release. Graduating from alpha after a comprehensive security audit, 491-test verification, and 3 rounds of independent code review. The highlights below consolidate the headline work that made Kalpa beta-ready; see the alpha entries for per-change detail.

### Security & Hardening
- Allowlisted ESOUI download URLs and centralized path validation across all Tauri IPC commands
- Recursion caps and streaming ZIP hashing to bound resource use during install
- DoS-resistant Pack Hub: native rate limiting plus a Durable Object for atomic pack-index mutations
- CSP hardening, including `frame-ancestors 'none'`
- Dependencies verified against May 2026 CVE databases — zero `npm audit` / `cargo audit` vulnerabilities

### Features
- Protected edits — file-level diff and per-file choice when an update would overwrite your local changes, with automatic edit backups
- `.esopack` v2 — optional account-wide addon settings in shared packs, automatically scrubbed of personal data on export and re-mapped to the importer on install (see [docs/settings-export.md](docs/settings-export.md))
- Redesigned backup & restore UX with a protection-status indicator and an automatic safety snapshot before every restore
- Dependency resolution — auto-install new transitive dependencies after updates and validate version constraints against installed addons

### Testing & CI
- 491 tests across Vitest (frontend + worker) and Rust unit test suites
- Worker tests run in CI and before every deploy
- Pinned Rust 1.88.0 and cargo-audit 0.22.1

### Dependencies
- Bump tauri 2.11.1 → 2.11.2, tauri-build 2.6.1 → 2.6.2
- Bump lucide-react 1.14 → 1.16, @base-ui/react 1.4.1 → 1.5.0
- Bump @fontsource-variable/geist 5.2.8 → 5.2.9
- Bump wrangler 4.90 → 4.93, @cloudflare/workers-types
- Bump dev-dependencies group (6 updates)

## [0.1.0-alpha.8] — 2026-05-23

### Security & Hardening
- Harden path validation and centralize download URL allowlist
- Deny-by-default pack ownership check in worker
- Add native rate limiting and Durable Object for atomic pack index mutations
- Harden auth state, file editor limits, and async cleanup
- Add CSP `frame-ancestors` directive
- Harden Pack Hub worker and resolve dependency vulnerabilities
- Improve keyboard handling, accessibility, and error visibility

### Features
- `.esopack` v2 — per-addon SavedVariables export/import (Phase 1 backend)
- Protected edits — preserve user changes across addon updates (hash infrastructure, conflict scanning, file browser, diff viewer, batch conflict flow, CodeMirror editor, backup restore)
- Improve backup UX for non-technical users (redesigned backup & restore flow)
- Skeleton loading states, discover detail polish, and ESOUI browse fixes
- Auto-install new dependencies after addon updates
- Validate dependency version constraints against installed addons

### Bug Fixes
- Filter ESOUI search summary row and deduplicate results
- Use subfolder-aware resolution in all dependency install paths
- Resolve transitive deps on manual dependency install
- Show skipped deps in install success banner
- Improve network resilience, pagination, and UI state management
- Include bundled sub-library versions in outdated check
- Abort restore when safety snapshot copy fails

### Testing & CI
- Add Vitest unit tests and Playwright E2E testing infrastructure
- Add Vitest tests for Pack Hub Cloudflare Worker
- Run worker tests in CI and before deploy
- Pin Rust 1.88.0 and cargo-audit 0.22.1

### Dependencies
- Bump lucide-react 1.11 → 1.14, Vite 8.0, TypeScript 6.0
- Upgrade wrangler to v4
- Bump Rust deps: tokio, reqwest, winreg, zip

## [0.1.0-alpha.3] — 2026-05-02

### UI & Animations
- Add animate-ui primitives for dialog, tooltip, popover, and checkbox
- Complete animation coverage across all components (slide-fade tab transitions, entrance animations)
- Add UX polish, animations, and accessibility improvements across the app
- Add animate-ui animation enhancements to pack components
- Add context menu component
- Add animated checkmark component

### Bug Fixes
- Decode HTML entities in addon descriptions
- Fix updater endpoint by stopping releases from being marked as prerelease
- Truncate MD5 hash with click-to-copy in Discover
- Persist batch removals on `beforeunload`
- Fix DialogPortal gracefully handling missing context
- Resolve `rand` and `rustls-webpki` audit failures

### Dependencies
- Bump tokio 1.51 → 1.52, reqwest 0.13.2 → 0.13.3, winreg 0.55 → 0.56, zip 8.5 → 8.6
- Bump lucide-react 1.8 → 1.11, @tanstack/react-virtual 3.13.23 → 3.13.24, @base-ui/react 1.4 → 1.4.1
- Bump actions/setup-node 6.3 → 6.4

## [0.1.0-alpha.1] — 2026-04-03

First public alpha release of **Kalpa** — an open-source desktop addon manager for Elder Scrolls Online.

### Core Features
- Smart addon scanning with manifest parsing (`.txt` and `.addon` files)
- One-click install from ESOUI URL or addon ID
- Automatic dependency resolution (3 levels deep)
- Bulk update checking and one-click update all
- Browse and search ESOUI with addon detail view and screenshots

### Addon Management
- Profiles for quick addon set switching
- Full and character-specific backups with restore
- Character management grouped by server (NA/EU)
- API compatibility checking
- Addon list export/import (JSON)
- Minion migration with snapshots, dry-run preview, and integrity checks

### Pack Hub
- Community addon collections (packs) with Cloudflare Worker backend
- Pack sharing via share codes and `.esopack` file export
- Roster pack install via deep links
- Pack upvote system

### Discovery
- Browse ESOUI Popular tab with filters and enhanced UX
- Dynamic tag tabs for addon categorization

### SavedVariables Manager
- View and edit addon settings files

### Desktop Experience
- Tauri v2 desktop app with custom window chrome
- Auto-update with signed GitHub Releases
- Deep link scheme (`kalpa://`)
- Keyboard navigation
- Offline detection with graceful degradation
- Multi-candidate addon folder detection with setup wizard

### Infrastructure
- BSL 1.1 license (converts to Apache 2.0 after four years)
- GitHub Actions CI/CD with tag-triggered Windows release builds
- Code of Conduct (Contributor Covenant v2.1)

[Unreleased]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.3...HEAD
[0.1.0-beta.3]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.2...v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.1...v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-alpha.8...v0.1.0-beta.1
[0.1.0-alpha.8]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-alpha.3...v0.1.0-alpha.8
[0.1.0-alpha.3]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-alpha.2...v0.1.0-alpha.3
[0.1.0-alpha.1]: https://github.com/ESO-Toolkit/kalpa/releases/tag/v0.1.0-alpha.1
