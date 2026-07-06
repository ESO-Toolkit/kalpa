# Changelog

All notable changes to Kalpa are documented here. This project uses [Conventional Commits](https://www.conventionalcommits.org/).

## [Unreleased]

### Features

- **macOS and Linux support (beta).** Kalpa now builds, packages, and auto-updates on all three desktop platforms. macOS ships as a universal `.dmg` (Intel & Apple Silicon, macOS 10.15+) with native traffic-light window controls, ⌘-based shortcuts, Keychain-backed login persistence, and detection of both the native Mac client (`~/Documents/Elder Scrolls Online`) and CrossOver bottles. Linux ships as `.AppImage` (self-updating), `.deb`, and `.rpm`, stores your login in the Secret Service keyring, and automatically finds ESO under Steam Proton — including Flatpak/Snap Steam and secondary Steam libraries. Windows behavior is unchanged.

### Bug Fixes

- **"Show in folder" now works with any path separator.** Two spots joined paths with a hard-coded backslash, which produced invalid paths on macOS and Linux; they now use the platform-correct separator everywhere.

## [0.1.0-beta.9] — 2026-06-21

A feature-and-fixes release headlined by a full **theming system** for the app, paired with a fix that brings back characters most players were missing from the **Characters** list.

The marquee addition lives in **Settings → Appearance**: 49 built-in dark themes — Elder Scrolls art skins, ESO palettes, and editor classics — plus a custom theme builder with live preview and contrast checking. Alongside it, the Characters list now surfaces characters that ESO's default Account-Wide settings mode previously hid, per-character backups are now crash-safe, and Pack Hub sign-in works again after the ESO Toolkit auth site changed its callback.

### Features

- **Theme system with 49 built-in skins.** Settings now has an Appearance section where you can recolor the entire app. There are 49 dark themes to choose from, including eight Elder Scrolls art skins with their own textures and patterns (Elder Scroll, Daedric Obsidian, Dwemer Brass, Ayleid Welkynd, Sithis, Hermaeus Mora, Clockwork City, Nordic Runestone), ESO faction and lore palettes, editor classics like Dracula, Nord, Tokyo Night, Catppuccin and Gruvbox, plus Neon, Nature, Gemstone, Metal and Minimal sets. Switching is instant and applies everywhere, and the original gold default is still there if you want to leave things as they were. ([#175](https://github.com/ESO-Toolkit/kalpa/pull/175))
- **Custom theme builder.** Beyond the presets, you can build your own theme by picking 12 colors and watching the whole app update live as you go. A built-in readability check flags any color combination that falls below WCAG AA contrast, so your custom theme stays legible, and you can copy a finished theme to the clipboard or paste one in to share it. The color picker is powered by one small new dependency, `react-colorful` (~2.8 KB). ([#175](https://github.com/ESO-Toolkit/kalpa/pull/175))
- **No theme flash on startup.** Your chosen theme is applied before the window paints, so Kalpa opens directly in the right colors instead of briefly flashing the default theme first. ([#175](https://github.com/ESO-Toolkit/kalpa/pull/175))

### Bug Fixes

- **Missing characters now appear in the Characters list.** When ESO runs in its default Account-Wide Addon Settings mode, every character's data collapses into a single shared block in `AddOnSettings.txt`, so Kalpa could only see characters that had per-character headers and most of your roster never showed up. The Characters list now also recovers character names from your SavedVariables files and merges them in, so characters that were previously invisible are listed (with their real megaserver when it can be determined, otherwise grouped under Unknown). ([#180](https://github.com/ESO-Toolkit/kalpa/pull/180))
- **Characters hidden inside very large SavedVariables files now show up.** The previous scan loaded each addon data file into memory and skipped any file larger than 64 MiB, which meant a character whose only data lived in a big file would silently go missing. Opening the Characters panel now streams through files of any size, so the size limit is gone and no character is left out. ([#180](https://github.com/ESO-Toolkit/kalpa/pull/180))
- **Per-character backup and restore is now safe and precise.** Backing up or restoring a single character now surgically touches only that character's data, leaving every other character and your account-wide settings untouched, and same-named characters on NA and EU are kept separate. The process is crash-safe: an interrupted backup rolls back cleanly, restores take a safety snapshot first, and an unreadable backup fails safely instead of risking an overwrite of the whole file. ([#180](https://github.com/ESO-Toolkit/kalpa/pull/180))
- **Clearer errors when character data can't be read.** If `AddOnSettings.txt` can't be read (for example due to file permissions), the Characters list now surfaces the real error instead of quietly showing an incomplete roster, and files it has to skip during the scan are counted and reported. ([#180](https://github.com/ESO-Toolkit/kalpa/pull/180))
- **Fixed garbled emoji and special characters in Pack Hub content.** Community pack titles, descriptions, author names, and the addon names shown inside packs could appear as corrupted glyphs when they contained numeric HTML entities for emoji or other upper-Unicode characters, because those characters were being truncated during decoding. Kalpa now decodes these entities correctly so the right characters render, and malformed or invalid entities (including null, lone surrogate values, and uppercase-hex forms like `&#X1F600;`) are handled safely instead of turning into broken text. ([#171](https://github.com/ESO-Toolkit/kalpa/pull/171))
- **Pack Hub sign-in works again.** The ESO Toolkit sign-in site changed how it hands tokens back to the desktop app — it now sends them as a JSON `POST` to Kalpa's local callback instead of a query string — so browser sign-in could complete while the app never received usable tokens. Kalpa now accepts the new callback (including the cross-origin preflight and private-network permission the browser requires) while still understanding the legacy one, so signing in to the Pack Hub works again. ([#182](https://github.com/ESO-Toolkit/kalpa/pull/182))
- **Pack Hub account strip and dismissible panel loading.** The Pack Hub now shows the account you're signed in as with a one-click sign-out at the top, and panels that open on demand show a labeled loading dialog you can close instead of briefly flashing a blank screen. ([#182](https://github.com/ESO-Toolkit/kalpa/pull/182))

## [0.1.0-beta.8] — 2026-06-21

A dependency-resolution and addon-install fix release. It makes required/optional dependency status accurate and the in-app **Install/Update** buttons actually work for libraries ESOUI serves via a redirect — most visibly LuiExtended's `LuiData` and `LuiMedia`.

### Bug Fixes

- **Dependency Install/Update no longer fails with `not_found`.** ESOUI redirects a precise-name search straight to the addon's page (e.g. `info4373-LuiData.html`), which carries none of the result-list links Kalpa's search scraped — so the dependency Install/Update buttons returned `not_found` for libraries like `LuiData` and `LuiMedia`. Kalpa now recovers the addon id from the redirected URL. The Discover-tab search was fixed the same way, so an exact-name query no longer comes back empty. ([#174](https://github.com/ESO-Toolkit/kalpa/pull/174))
- **Dependency status is resolved case-insensitively, matching the game.** ESO loads addon folders case-insensitively, but Kalpa compared names exactly, so a folder cased differently than a `## DependsOn:` entry was falsely flagged "missing". Names are now matched case-insensitively, a folder counts as installed only if it actually has a matching manifest (a partly-extracted folder no longer masks a missing library), and `DependsOn` parsing tolerates spaces around `>=` and stray invisible characters. ([#173](https://github.com/ESO-Toolkit/kalpa/pull/173))
- **Optional dependencies show present/absent correctly,** including libraries bundled inside another addon, and the Remove button now targets the real on-disk folder rather than the dependency's declared name. ([#173](https://github.com/ESO-Toolkit/kalpa/pull/173))
- **Dependency install surfaces the real error** — e.g. a Controlled Folder Access / permission block, with the fix steps — instead of a generic `extract_failed`. ([#174](https://github.com/ESO-Toolkit/kalpa/pull/174))

### Features

- **Slide-reveal selection rail in the addon list.** ([#172](https://github.com/ESO-Toolkit/kalpa/pull/172))

### CI

- **Scope the root npm audit to production dependencies** (`--omit=dev`), mirroring the worker audit. It was failing on high-severity advisories in dev-only tooling (`jsdom`→`undici`, `shadcn`→`hono`/`js-yaml`) that never ship in the app. ([#176](https://github.com/ESO-Toolkit/kalpa/pull/176))

## [0.1.0-beta.7] — 2026-06-14

A correctness and data-integrity hardening release following the recent performance work. An audit of the batched install/update/toggle paths surfaced several cases where the speedups skipped a safety step; these fixes restore the guarantees while keeping the performance gains.

### Bug Fixes

- **Pack installs now record an edit-protection baseline.** `batch_install_pack_addons` and the addon-list import path extracted addons and recorded them in `kalpa.json` but never wrote a `.kalpa-hashes` baseline. Without it, the next update saw every file as unmodified and would silently overwrite a user's edits — the exact case the hash system exists to prevent. Both paths now record the baseline after extraction and fail the addon (rather than tracking it unprotected) if the baseline can't be written, matching the invariant the update paths already enforce. ([#159](https://github.com/ESO-Toolkit/kalpa/pull/159))
- **Surface metadata-save failures during install instead of reporting a blanket failure.** If `kalpa.json` couldn't be saved after addons were extracted (e.g. Controlled Folder Access, read-only or full disk), the whole batch was reported as failed with no list refresh — even though the addons were on disk. The installed addons are now moved into the failed set with the save error, partial state is surfaced, and the addon list refreshes whenever anything reached disk. ([#159](https://github.com/ESO-Toolkit/kalpa/pull/159))
- **Install result reconciliation.** Per-addon status pills in the roster pack installer now reconcile against the command's authoritative result rather than trusting the streamed progress events alone, so a late save failure no longer leaves green "installed" pills next to a "failed" toast. ([#159](https://github.com/ESO-Toolkit/kalpa/pull/159))
- **Dependency badges refresh when toggling a depended-on addon.** Enabling/disabling an addon patched its own state in place but left other addons' "N missing dependencies" badges stale, so disabling a shared library (e.g. LibAddonMenu-2.0) left dependents looking healthy until a manual refresh. A toggle now rescans only when the toggled addon is actually a dependency of another installed addon, keeping the badges accurate without paying the rescan cost in the common case. ([#159](https://github.com/ESO-Toolkit/kalpa/pull/159))

### Pack Hub (Worker)

- **Stop reverting pack edits via stale-cached counter writes.** Voting or installing a pack read the whole pack through a 300s edge-cached path, bumped one counter, and wrote the entire object back — so a vote or install landing shortly after an author edited their pack could silently roll that edit back, and concurrent votes could be lost. Counter updates now run inside the pack-index Durable Object against a fresh, single-threaded read that touches only the counter; the other mutating paths read fresh. ([#160](https://github.com/ESO-Toolkit/kalpa/pull/160))

### CI

- **Scope the worker npm audit to production dependencies** (`--omit=dev`). The worker ships no runtime dependencies; the audit was failing on an unfixable `esbuild` advisory pulled in only by dev/test tooling that never reaches the deployed worker. ([#158](https://github.com/ESO-Toolkit/kalpa/pull/158))

## [0.1.0-beta.6] — 2026-06-06

### Dependencies
A maintenance release rolling up batched dependency updates. No user-facing behavior changes.

- **`md-5` 0.10 → 0.11** — the bump pulls `digest` 0.11, which dropped the `io::Write` impl and `LowerHex` output; the download checksum verification in `esoui.rs` was adapted to chunked `update()` + manual hex encoding (verified against known MD5 vectors).
- **`react` / `react-dom` 19.2.6 → 19.2.7** (kept in lockstep to satisfy the exact-version peer requirement)
- **`@tanstack/react-virtual` 3.13.24 → 3.14.2**
- **`lucide-react` 1.16.0 → 1.17.0**
- **`motion` 12.38.0 → 12.40.0**
- **`@uiw/react-codemirror` and `@uiw/codemirror-themes` 4.25.9 → 4.25.10**
- **Worker (`backend/eso-packs-worker`)**: `wrangler` 4.94 → 4.98, `@cloudflare/workers-types` and `@cloudflare/vitest-pool-workers`, `vitest` 4.1.6 → 4.1.8
- **CI**: `actions/checkout` 6.0.2 → 6.0.3
- **Dev dependencies**: grouped bump across `@types/node`, `eslint`, `shadcn`, `typescript-eslint`, `vite`, `vitest`, and `@types/react`

> **Deferred:** `rusqlite` 0.39 → 0.40 ([#120](https://github.com/ESO-Toolkit/kalpa/pull/120)) is held back — it pulls a newer `libsqlite3-sys` whose build script uses the `cfg_select` macro, which requires a rustc newer than our pinned `1.88.0`. It will land alongside an intentional Rust toolchain bump.

## [0.1.0-beta.5] — 2026-06-06

### Features
- **Update while ESO is running** — addon batch updates no longer hard-block when the game is open. A confirm dialog explains that files will update but ESO won't see changes until `/reloadui` or a relog (the same workflow Minion uses). Includes a "Don't show again" option and a "Warn when ESO is running" toggle in Settings.
- **Controlled Folder Access guidance** — when Windows Controlled Folder Access (CFA) silently blocks Kalpa from writing to the AddOns folder, a glass modal now explains the cause with numbered remediation steps, a copy-path button for `kalpa.exe`, and a one-click "Open Windows Security" button. Shown proactively before Update All when a block is detected, and as a fallback after a failure.

### Bug Fixes
- **Surface per-addon update failures** — batch updates previously reported only "Updated 0 addons, N failed" with no explanation. Failures are now captured per addon (scan and decision phases) and shown grouped by cause with affected addon names in the summary toast.
- Map `PermissionDenied` write errors to an actionable message naming CFA as the likely cause plus exact Windows Security steps, instead of a raw `Access is denied (os error 5)`.
- Distinguish CFA-blocked writes from corrupt-archive errors during extraction.
- Claim busy state before the ESO-running check in pack/roster installs to close double-submit and stale-gate gaps.
- Prevent overlapping batch updates during the ESO-running preamble; reset the opt-out checkbox between prompts.
- Stream Update All through a single batched command (one metadata write).
- Prevent CFA modal content from overflowing.

## [0.1.0-beta.4] — 2026-05-25

### Security Fixes
- **Draft packs were visible to unauthenticated users** via `?status=all` — now requires auth and ownership
- **Any authenticated user could view other users' drafts** by ID — added ownership check
- Add pack ID validation to generic `/packs/:id` route
- Validate `defaultEnabled` field in pack and share payloads
- Add `DELETE /account` endpoint for user data deletion (GDPR compliance)
- Secure token storage and observability opt-out with privacy policy link

### Bug Fixes
- Fix `BackupManifest` missing `#[serde(rename_all = "camelCase")]` — all edit backup fields were `undefined` in the frontend
- Fix `SvTreeNode.rawLuaValue` missing from TypeScript — caused silent data corruption on round-trip SavedVariables save
- Fix stuck loading spinner when `detect_game_instances` fails
- Fix `decodeHtml` innerHTML-based decoder — replaced with regex to eliminate DOM dependency
- Fix timer cleanup in discover-detail and packs to prevent setState on unmounted components
- Fix `useCallback`/`useEffect` dependency for share code handler
- Merge batch update progress double setState into single updater
- Fix library addon color from emerald to violet per design system spec
- Fix tag menu dropdown rounding (`rounded-md` → `rounded-xl`)
- Fix custom tag input to use glass input styling
- Gate duplicate dependents warning on `!addon.disabled`
- Fix `BackupManifest` serde aliases for backward compatibility
- Fix `auto_link_addons` filelist fetch moved outside MetadataLock to prevent deadlock

### Rust Backend Hardening
- Add `MetadataLock` mutex to prevent TOCTOU race conditions on `kalpa.json` (12 commands protected)
- Narrow MetadataLock scope to exclude network I/O for better concurrency
- Add partial extraction cleanup — removes newly-created folders on ZIP extraction failure
- Add bounded ZIP read (5 MB cap) in conflict diff viewer to prevent OOM
- Add recursion depth limit (32) and symlink skip in `walk_files` and `compute_addon_hashes`
- Add retry logic to `fetch_filelist_entries` and `download_addon` for transient HTTP errors
- `batch_remove_addons` now reports per-addon failures instead of silently dropping them
- MD5 verification and path hardening across installer

### Frontend Improvements
- Enable `noUncheckedIndexedAccess` in TypeScript — all array/record index access is now type-safe
- Add ErrorBoundary "Try Again" recovery button
- Add Windows error hints for file lock (os error 32/33) and disk space (os error 112)
- Show loading spinner in Profiles dialog instead of flashing "No profiles yet"

### CI/CD
- Align Node.js version in release workflow (20 → 22)
- Add concurrency control and timeout-minutes to all workflow jobs
- Add npm cache to worker deploy workflow
- Harden CSP with explicit `object-src`, `base-uri`, `form-action` directives
- Fix timestamp URL to HTTPS

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

[Unreleased]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.4...HEAD
[0.1.0-beta.4]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.3...v0.1.0-beta.4
[0.1.0-beta.3]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.2...v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.1...v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-alpha.8...v0.1.0-beta.1
[0.1.0-alpha.8]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-alpha.3...v0.1.0-alpha.8
[0.1.0-alpha.3]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-alpha.2...v0.1.0-alpha.3
[0.1.0-alpha.1]: https://github.com/ESO-Toolkit/kalpa/releases/tag/v0.1.0-alpha.1
