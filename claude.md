# Kalpa — Claude Code Guide

You are Claude Code working in this repository. Optimize for **safety, clarity, and maintainability** while helping evolve this project.

---

## Mission & Current State

Kalpa is a source-available desktop app for managing Elder Scrolls Online addons, licensed under BSL 1.1 (not open source in the OSI sense — it converts to Apache 2.0 four years after each release). It is in **public beta** (see `package.json` for the current version) with:

- Addon scanning and installation
- Updates and dependency resolution
- Backups and profiles
- Character management and API compatibility checks
- Minion migration support
- Pack Hub for community addon collections

Your job is to improve this app without breaking existing functionality or the build.

---

## Tech Stack Snapshot

- **Desktop client**: Tauri v2 + React 19 + TypeScript + Tailwind v4 + shadcn-ui
- **Backend**: Cloudflare Workers + KV, mirrored into the website's shared D1 (Pack Hub)
- **CI/CD**: GitHub Actions with tag-triggered release builds (Windows NSIS, macOS universal dmg, Linux AppImage/deb/rpm)

When in doubt, prefer solutions that fit naturally into this stack.

---

## Core Principles & Constraints

Follow these rules unless explicitly directed otherwise:

- **No private APIs or hacks**
  - Only use public ESOUI pages and direct public download URLs.
- **Centralized scraping**
  - Keep all scraping logic in `src-tauri/src/esoui.rs`.
- **No background spam**
  - Do not implement hourly or aggressive background scraping.
  - Use "on-open" refresh plus an explicit **Refresh** button.
- **Maintainability over cleverness**
  - Prefer straightforward, well-documented code over overly abstract solutions.
- **Build must always pass**
  - Keep the repo buildable and tests/linters passing after each change.

---

## Project Structure

Use the existing architecture; extend it instead of inventing new patterns:

```text
src/                        # React frontend
  components/               # Feature components (addon list, packs, settings)
  components/ui/            # shadcn-ui primitives
  components/uploader/      # ESO Logs uploader workspace
  hooks/                    # Shared React hooks
  lib/                      # Utilities, Tauri bindings, store, theme presets
  types.ts                  # Shared TypeScript interfaces

src-tauri/src/              # Rust backend
  commands.rs               # Tauri command handlers (except the uploader's)
  esoui.rs                  # ESOUI API client and HTML scraping
  manifest.rs               # ESO addon manifest parser
  manifest_cache.rs         # SQLite-backed manifest cache
  installer.rs              # ZIP extraction and addon installation
  metadata.rs               # Metadata tracking and persistence
  file_hashes.rs            # File hashing for update conflict detection
  edit_backups.rs           # Backups for addon file edits
  safe_migration.rs         # Minion migration with dry-run and snapshots
  game_instances.rs         # Multi-instance detection (native/Steam)
  platform.rs               # Cross-platform helpers (Steam/Proton discovery, open_url)
  settings_store.rs         # Atomic app-settings persistence
  saved_variables/          # SavedVariables parsing, scrubbing, per-character backups
  uploader/                 # ESO Logs uploader (scan, split, encode, upload, live)
    commands.rs             # The uploader's own Tauri commands
  auth.rs                   # Authentication
  token_store.rs            # Credential storage (Credential Manager / Keychain / Secret Service)
  lib.rs                    # Module definitions, MetadataLock, Tauri app setup

backend/eso-packs-worker/   # Pack Hub Cloudflare Worker
  src/index.ts              # Router, handlers, scheduled backup
  src/kv.ts                 # KV read/write helpers
  src/pack-index-do.ts      # Durable Object for atomic index mutations
  src/types.ts              # Pack types (snake_case, matches Rust HubPack)
  src/validate.ts           # Input validation
  src/shares.ts             # Share code create/resolve, bearer-token validation
  src/redact.ts             # Anonymous-pack author redaction
  src/seed.ts               # Seed data for a fresh namespace
  src/cors.ts               # CORS config
  wrangler.toml             # Worker config — name MUST be "kalpa-pack-hub"

prototypes/slint-kalpa/     # Native (Slint) performance UI sidecar, shipped on Windows
```

Both `commands.rs` files register handlers into the single `generate_handler!` list in `lib.rs`. When adding new logic, pick the closest existing file that matches the concern before creating new modules — uploader and SavedVariables work belongs in `uploader/` and `saved_variables/`, not in the root `commands.rs`.

---

## Pack Hub Worker — Critical Rules

The Pack Hub is a **dedicated Cloudflare Worker** (`kalpa-pack-hub`), deployed separately from the ESO Toolkit website API (`roster-hub-api`) — but it is not isolated from it: the two share the `roster-hub-db` D1 database.

### NEVER do these:

- **NEVER deploy to `roster-hub-api`** — that is the ESO Toolkit website's full API (D1, Discord, AI). Deploying pack hub code there will overwrite the entire website API.
- **NEVER change the `name` field in `wrangler.toml`** from `kalpa-pack-hub`.
- **NEVER deploy to `eso-packs-worker`** — that was an old name and is now deleted.
- **NEVER run `wrangler deploy` without running `tsc --noEmit` first.**

### Architecture:

- **Worker URL**: `https://kalpa-pack-hub.eso-toolkit.workers.dev`
- **Primary store**: Cloudflare KV (`ESO_PACKS` namespace)
- **Shared D1 mirror**: every pack mutation is dual-written inline into the `packs`/`pack_tags` tables of `roster-hub-db` (binding `ROSTER_HUB_DB`) so esotk.com reflects the latest pack data. **These tables are shared with `roster-hub-api` — any schema or SQL change has to be coordinated with the website.**
- **Index serialization**: `PackIndexDO` (Durable Object binding `PACK_INDEX`) owns mutations of the `index:packs` value
- **Rate limiting**: three built-in limiter bindings — `READ_LIMITER` (60/min), `WRITE_LIMITER` (10/min), `VOTE_LIMITER` (20/min)
- **API format**: snake_case JSON matching Rust `HubPack` struct in `commands.rs`
- **Auth**: ESO Logs Bearer token via `validateBearerToken()` in `shares.ts`
- **Backup**: Daily cron at midnight UTC snapshots pack index to `backup:YYYY-MM-DD` keys (90-day TTL)
- **CI**: `.github/workflows/deploy-worker.yml` — auto-deploys on push to main, with typecheck + name guard + health check

### Rust integration:

- `commands.rs` calls `kalpa-pack-hub.eso-toolkit.workers.dev` (see `pack_hub_url()` and `share_worker_url()`)
- Response format: `{ packs: [...], page, sort }` for list, `{ pack: {...} }` for detail
- Pack fields are snake_case: `title`, `pack_type`, `author_id`, `author_name`, `is_anonymous`, `vote_count`, etc.

---

## Code Quality & Checks

**Rust**

- After editing Rust code, always run:
  1. `cargo clippy --fix --allow-dirty --allow-staged` (or similar clippy invocation)
  2. `cargo fmt`
- `cargo fmt` must run **after** clippy because clippy fixes can break formatting.

**Frontend**

- Run: `npm run check`
  - This runs TypeScript, ESLint, and Prettier.
- Fix all reported issues before considering the work complete.

**End-to-end**

E2E drives the real Tauri webview over CDP, so it is Windows-only (the debug port
comes from `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, which WebKitGTK/WKWebView do
not have). There are two flavours, and the difference matters:

- `npm run test:e2e` — attaches to whatever `npm run tauri dev` is already
  running, which is your REAL ESO install. Read-only specs only. Never add a spec
  here that installs, updates, removes, restores, migrates or applies a profile.
- `npm run test:e2e:sandbox` — builds the debug binary, launches it with
  `KALPA_ADDONS_DIR` pointed at a throwaway `AddOns` folder, and runs the
  `@sandbox` specs against it. This is where destructive coverage belongs. The
  override is `debug_addons_dir_override` in `commands.rs`, compiled out of
  release builds so no shipped binary can be aimed away from a user's real
  folder. Pass `--no-build` when iterating on the specs themselves.

**CI**

- GitHub Actions enforces Rust and frontend checks on every PR.
- Neither e2e flavour runs in CI, and `test:packaged` doesn't either. All three
  drive a real WebView2 window over CDP, and on a GitHub Windows runner the app
  launches and stays alive but **never binds the debug port** — verified three
  times with `netstat` showing no listener on 9222. Run them locally before a
  release. `ci.yml` records what was ruled out, so a future attempt starts from
  evidence instead of repeating it.
- Treat CI failures as blockers; update code until CI is green.

---

## Git Workflow & Releases

### Branching

Use **GitHub Flow**:

1. `main` is always releasable.
2. Create short-lived branches such as:
   - `feat/feature-name`
   - `fix/bug-name`
3. Open a PR, let CI pass, request review, then merge to `main`.
4. Tag releases from `main` (for example `v0.3.0`) to trigger release CI.

### Commit Messages

Use **Conventional Commits**:

- Format: `type(scope): description`
- Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`
- Use imperative mood, keep under ~50 characters, no trailing period.

### Release Process

When preparing a new release:

1. Bump the version. Six fields across five files carry it, and every one has to
   move together — listing only the first three is how nine consecutive tags
   (alpha.8 through beta.9) shipped with a stale `package-lock.json`:
   - `src-tauri/tauri.conf.json`
   - `src-tauri/Cargo.toml`
   - `package.json`
   - `package-lock.json` (**twice** — top level and `packages[""]`)
   - `src-tauri/Cargo.lock` (the `[[package]]` block named `kalpa`)

   The lockfiles are easiest to get right by tool rather than by hand:
   `npm version <v> --no-git-tag-version` updates `package.json` and both
   `package-lock.json` fields; `cargo update --workspace` rewrites only the
   local crate's `Cargo.lock` entry. Run `npm run check:versions` to confirm —
   CI runs the same check, and `release.yml` also compares the result to the tag.

2. Add a `## [<version>] — YYYY-MM-DD` section to `CHANGELOG.md` and a matching
   link-reference definition at the bottom of the file (a heading with no
   definition renders as literal bracketed text). Every GitHub release body
   opens with "See CHANGELOG.md for full details", so a missing entry sends
   users to a file that does not mention the release they just installed —
   which is how beta.15 shipped.
3. Rewrite the per-release "Changed:" section of `releaseBody` in
   `.github/workflows/release.yml`. It is shared by every tag, so it otherwise
   ships the previous release's headline.
4. Run the packaged build verification gate on Windows: `npm run test:packaged`.
   It is deliberately local-only because it needs WebView2, launches the debug
   packaged binary itself, and fails if it connects to the Vite dev server instead
   of `http://tauri.localhost/`.
5. Push a tag `v*` (for example `v0.3.0`).
6. `.github/workflows/release.yml` builds installers for all three platforms (Windows NSIS `.exe`, macOS universal `.dmg`, Linux `.AppImage`/`.deb`/`.rpm`) via a tauri-action matrix and attaches them — plus updater `.sig` files and a merged multi-platform `latest.json` — to one GitHub Release.

### Cross-Platform Notes

- Platform-divergent Rust helpers live in `src-tauri/src/platform.rs` (Steam root discovery, Proton prefix scanning, `open_url`, `pgrep`-based process detection). ESO detection injects Proton/CrossOver documents roots through `documents_candidates()` in `src-tauri/src/commands.rs`, which feeds every detection consumer (instances, addons dirs, log discovery).
- Frontend OS branching goes through `src/lib/platform.ts` (`osType()`, `isMac()`, `modKeyLabel()`, `isModKey()`) backed by `@tauri-apps/plugin-os`.
- Per-platform bundle/window overrides live in `src-tauri/tauri.macos.conf.json` (native traffic lights via `titleBarStyle: Overlay`) and `src-tauri/tauri.linux.conf.json`; the base `tauri.conf.json` stays Windows-shaped (`decorations: false` + custom buttons, also used on Linux).
- Token storage uses the OS credential store on every platform (Credential Manager / Keychain / Secret Service) through the same chunked layout in `src-tauri/src/token_store.rs`.

---

## Design System Essentials

The UI builds on the ESO Log Aggregator visual language, adapted to shadcn-ui and Tailwind v4. Respect the existing design system; do not introduce ad-hoc styles if a primitive exists.

### Reference Design Docs

Review these before UI work:

1. `context/40-design-system.md` — design principles, colors, glass morphism, typography, animations.
2. `context/41-component-patterns.md` — concrete shadcn component recipes.
3. `context/42-theme-tokens.md` — CSS variables, `@theme` inline mappings, Tailwind utilities.

Docs 40 and 41 were written before the light-theme token migration, so the literal `rgba(…)` / white-alpha snippets in them are historical. The shipped components (`src/components/ui/*.tsx`) and `src/index.css` are the authority for actual class names; the Visual Rules below say which tokens to use.

### Implemented UI Primitives

Use these components instead of re-rolling new ones:

- `GlassPanel` (`components/ui/glass-panel.tsx`)
  - Variants: `primary`, `default`, `subtle`
- `SectionHeader` (`components/ui/section-header.tsx`)
  - Uppercase micro-label (11px, Space Grotesk)
- `InfoPill` (`components/ui/info-pill.tsx`)
  - Colors: `gold`, `sky`, `emerald`, `amber`, `red`, `violet`, `muted`

### Overridden shadcn Components

- `Input` — glass styling (translucent background, sky-blue focus ring)
- `Dialog` — glass morphism overlay with gradient background and gold gradient titles
- `Toaster` — glass-styled toasts

### Visual Rules

- Dark-first; light themes are in scope. All colors come from theme tokens.
- Glass morphism panels:
  - Three tiers: `primary`, `default`, `subtle`.
- Typography:
  - `Space Grotesk` (`font-heading`) for headings.
  - `Geist` (`font-sans`) for body text.
- Addon list items:
  - 3px colored left border encoding status.
- Borders and dividers:
  - Surfaces: `border-structure-06` (not `border-border`, and **never** `border-white/[0.06]`).
  - Dividers: `<div className="border-t border-structure-06" />` instead of `<Separator />`.
  - The `structure-*` ladder (`structure-01` … `structure-70`) and the `scrim-*` ladder are theme-aware: `--structure-rgb` flips from white to black on light themes, so a literal white-alpha class renders invisible on the three light and two high-contrast themes.
- Spinners:
  - Use `border-structure-10 border-t-primary` (accent-colored top border, follows the theme).
- Motion:
  - Timing scale: fast 150ms, normal 250ms, slow 400ms.
- Colors:
  - `primary` is the brand accent (ESO gold `#c4a44a` on the default theme, but themes reseed it — use `text-primary` / `bg-primary/[0.04]`, never the hex).
  - `accent-sky` for interactive and focus states.
  - Status colors go through the `status-*` tokens (`status-success`, `status-warning`, `status-danger`, `status-info`, `status-library`), which are re-applied per theme by `theme-apply.ts` — they are not fixed palette values.
  - Overlay depth goes through `scrim-*`, not `rgba(0,0,0,…)`.

---

## How to Work in This Repo (Claude)

When performing changes, follow this workflow:

1. **Load context**
   - Skim the relevant `context/*.md` files for the area you are touching.
   - Always read `context/40-design-system.md` before any UI work.
2. **Clarify intent**
   - Restate the user's goal and constraints before proposing changes.
   - Prefer small, incremental improvements over broad refactors.
3. **Plan the change**
   - Identify which files you will touch (both Rust and React).
   - Check for existing patterns or utilities to reuse.
4. **Implement safely**
   - Keep changes small and reviewable.
   - Avoid introducing new dependencies unless necessary and clearly justified.
5. **Verify**
   - Run `npm run tauri dev` locally (or instruct the user) to ensure the app still starts.
   - Run `npm run check`, `cargo clippy`, and `cargo fmt`.
6. **Explain**
   - When done, summarize what changed, why, and any follow-up tasks or caveats.

---

## Dev Server Port

Kalpa's Vite dev server uses **port 1430** (overriding Tauri's default 1420) so it doesn't collide with other Tauri projects running on the same machine.

Port configuration lives in two places that must stay in sync:

- `.env.local` → `VITE_PORT=1430` (read by `vite.config.ts` via `loadEnv`)
- `src-tauri/tauri.conf.json` → `"devUrl": "http://127.0.0.1:1430"`

`VITE_PORT` lives only in `.env.local`, which is gitignored — copy `.env.example` to `.env.local` on a fresh clone, or `npm run tauri dev` waits forever for a dev server that never appears on 1430.

If you need to change the port:

1. Update `VITE_PORT` in `.env.local`
2. Update `devUrl` in `src-tauri/tauri.conf.json` to match
3. **Do not commit `.env.local`** — it is gitignored and machine-local.

---

## Available Tools & Commands

You can assume access (by the human developer) to:

- `gh` — GitHub operations (PRs, issues, releases).
- `wrangler` — Cloudflare Worker deployment.
- Local Rust/Node toolchain:
  - `npm install`
  - `npm run tauri dev` — run the desktop app in development.

When suggesting steps, prefer commands that fit this toolchain.

---

## Chrome DevTools MCP (Visual Debugging)

The Tauri WebView2 exposes Chrome DevTools Protocol (CDP) on **port 9222** via `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` in `src-tauri/src/lib.rs`, guarded by `#[cfg(debug_assertions)]` so it is enabled **only in debug builds**.

### Setup

1. Run `npm run tauri dev`.
2. CDP is automatically available at `http://localhost:9222`.
3. Production/release builds never expose this debug port.

### Capabilities

Use CDP-backed tools for visual debugging:

- `take_screenshot` — capture the current rendered UI.
- `evaluate_script` — run JavaScript in the webview to inspect state or trigger actions.
- `click` / `fill` / `hover` — interact with UI elements.
- `list_network_requests` / `get_network_request` — inspect ESOUI API calls.
- `list_console_messages` — read frontend logs.
- `take_snapshot` — capture the DOM accessibility tree.

### Typical Debugging Flow

1. The user starts `npm run tauri dev`.
2. Claude connects via:
   - `list_pages` -> `navigate_page` to `http://127.0.0.1:1430` -> `select_page`.
3. Use `take_screenshot` to see the current state of the app.
4. Use other CDP tools to inspect layout, state, network calls, and console messages.

Remember: CDP access must never leak into production builds.

---

## Context File Index

Before large changes, consult these:

- `context/00-overview.md` — Core vision and principles.
- `context/10-desktop-client.md` — Desktop client architecture.
- `context/20-metadata-worker.md` — Backend worker design.
- `context/30-mvp-plan.md` — Original phase roadmap.
- `context/40-design-system.md` — Design language and visual rules.
- `context/41-component-patterns.md` — Component patterns and best practices.
- `context/42-theme-tokens.md` — Theme tokens and Tailwind integration.
