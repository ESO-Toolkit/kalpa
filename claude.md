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
  __mocks__/                # Shared frontend test mocks
  __tests__/                # Frontend setup and source-hygiene tests
  components/               # Feature components (addon list, packs, settings)
  components/__tests__/     # Feature-component tests
  components/animate-ui/    # Motion primitives grouped by animate/base/buttons/effects/texts
  components/ui/            # shadcn-ui primitives
  components/uploader/      # ESO Logs uploader workspace
  components/uploader/__tests__/ # Uploader component and reducer tests
  hooks/                    # Shared React hooks
  hooks/__tests__/          # Shared hook tests
  lib/                      # Utilities, Tauri bindings, store, theme presets
  lib/features.ts           # App-shell feature registry (see below)
  lib/__tests__/            # Frontend utility and contract tests
  types.ts                  # Shared TypeScript interfaces

e2e/                       # Windows WebView2 read-only and sandbox Playwright specs

src-tauri/src/              # Rust backend
  commands.rs               # Tauri command handlers (except the uploader's and Pack Hub's)
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
  pack_hub/                 # Pack Hub worker client (packs, votes, shares, .esopack)
    commands.rs             # Pack Hub's own Tauri commands
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
  test/                     # Worker unit, route, Durable Object, and scheduled tests
  wrangler.toml             # Worker config — name MUST be "kalpa-pack-hub"

prototypes/slint-kalpa/     # Native (Slint) performance UI sidecar, shipped on Windows
```

### The shell feature registry

`src/lib/features.ts` is the single source of truth for Kalpa's app-shell surfaces:
what each feature is called, which icon it wears, and where the user finds it
(pinned in the header toolbar, listed in the Tools menu, or shown in
Settings > Tools). `App.tsx`, `app-header.tsx`, `app-dialogs.tsx`,
`settings.tsx` and `keyboard-shortcuts.tsx` all read from it.

**Add a new shell surface by adding a `FeatureDef` there — not by threading
another `onOpenX` prop through App -> AppHeader -> Settings.** That prop-drilling
is what the registry replaced, and it is how the toolbar and the Settings Tools
list drifted apart in the first place.

It is **not** a feature-flag system. No entry gates a Tauri command, a Cargo
feature, or a build variant; every feature is always registered and always
reachable by keyboard shortcut and deep link. The registry only decides where a
surface appears. `visibleToolbar()` and `toolsMenuFeatures()` are pure functions
of `(FEATURES, hidden, ctx)` so they can be unit-tested without a DOM.

Two strings per feature look redundant but are not: `label` is the menu/row text
("Backup & Restore") and `dialogTitle` is the dialog's loading-fallback title
("Backups"). Leave `dialogTitle` unset when they agree.

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
- **API format**: snake_case JSON matching the Rust `HubPack` struct in `pack_hub/commands.rs`
- **Auth**: ESO Logs Bearer token via `validateBearerToken()` in `shares.ts`
- **Backup**: Daily cron at midnight UTC snapshots pack index to `backup:YYYY-MM-DD` keys (90-day TTL)
- **CI**: `.github/workflows/deploy-worker.yml` — auto-deploys on push to main, with typecheck + name guard + health check

### Addon index (`/addons/*`)

The worker also owns a full-text index of the ESOUI catalogue, in a **separate**
D1 database (binding `ADDON_INDEX`, database `kalpa-addon-index`). It backs
Discover's search box, which previously matched addon _titles_ only because the
bulk filelist API carries no descriptions.

- `src/addon-index.ts` — D1 schema, FTS5 table, BM25 search. `expandIdentifier`
  splits CamelCase titles ("CombatIndicator" -> "Combat Indicator") because
  FTS5 tokenises the glued form as one token and would otherwise never match a
  spaced query against a title.
- `src/crawl.ts` — the ESOUI sync. `syncFilelist` is one bulk request;
  `crawlDetails` fetches `filedetails/{id}` only for entries whose `lastUpdate`
  moved.
- `src/addon-routes.ts` — `GET /addons/search`, `GET /addons/stats`, and the
  admin-only `POST /admin/index/sync`, `POST /admin/index/backfill` and
  `POST /admin/index/reprocess`.

`stripMarkup` is pure over its input, but what gets **stored** is its output —
so adding a rule leaves every existing row contaminated. `reprocess` re-applies
the current pipeline to stored text in place, with no upstream traffic. Reach
for it after any text-pipeline change rather than re-crawling ESOUI to work
around our own parser. Run the crawl with `npm run index:build` (needs
`ADMIN_API_KEY`), and **drive it against the deployed worker, not
`wrangler dev --remote`** — the remote preview session dies after ~20 minutes
and returns Cloudflare HTML error pages mid-run.

Two upstream quirks the crawl exists to absorb: `categorylist.json` returns
`id` as a **string** while `filelist.json` sends a number (a `typeof` check
here silently blanked every category), and ESOUI descriptions carry BBCode
whose attribute is often a full URL, so the tag pattern cannot be
length-capped tightly.

**Category 157, "Discontinued & Outdated", is excluded by default** — it is
~980 of ~4170 addons. Offering a retired addon as the answer to "is there an
addon that…" reads as a live recommendation, which is worse than no answer.
Pass `?discontinued=true` to include them.

D1's limits shape the write path and are easy to reintroduce: **100 bound
parameters per query** and **1000 queries per Worker invocation**. Metadata is
therefore written in multi-row statements via `upsertMetaBatch`, and the sync
tombstones with `sweepUnseen` (an `indexed_at < runStart` comparison) rather
than an id list. One statement per addon means ~4000 queries and fails.

`detail_stale` is the FTS rebuild trigger, and only `applyDetail` ever writes
an `addons_fts` row. Anything that invalidates the indexed text — a moved
`last_update`, a changed `category_name`, or **un-tombstoning** — has to
re-arm it, or the addons table and the search index quietly disagree.

**The nightly crawl is fail-closed.** It runs only when `ADDON_INDEX_SYNC` is
exactly `"enabled"` AND the `ADDON_INDEX` binding exists. Provision the database
and finish the backfill _before_ flipping the var — and note that this is the
one sanctioned exception to "no background spam": one bulk request plus a
bounded page of changed descriptions per day. Do not widen it to hourly, and do
not add other scheduled outbound fetches without the same kind of bound.

This does not violate "keep all scraping in `esoui.rs`". That rule governs the
desktop client, and the crawl is not scraping — it uses the same public
`api.mmoui.com` JSON API `esoui.rs` already calls, once on the server for all
users rather than once per user. Net ESOUI load falls, because search stops
hitting `esoui.com/downloads/search.php`.

### Ask (`POST /ask`)

`src/ask.ts` is a natural-language addon assistant built on the same index, and
its design is deliberately lopsided: **retrieval finds the addons, the model
only picks among them and writes one sentence.** The model never sees a URL and
never emits one.

Three layers keep answers honest, and the third is the one that holds:

1. Candidates are shown to the model as opaque keys (`C1`, `C2`, …), not IDs.
2. The prompt states the closed set of valid keys. This is only a prompt-level
   constraint: `@cf/meta/llama-3.1-8b-instruct-fp8` **rejects `json_schema`
   outright** (`AiError 5025: This model doesn't support JSON Schema`), which
   failed every call and silently degraded every answer until it was caught.
   The route uses `json_object`, which guarantees parseable JSON and nothing
   more. Verify any model change against `wrangler ai models` first.
3. `groundOutput()` re-checks every pick against the retrieved set and rebuilds
   `file_info_uri` from the index row. **This is the real boundary** — layer 2
   cannot be relied on at all, so never weaken layer 3.

Addon descriptions are third-party text and are treated as untrusted, and
`scrubProse()` additionally strips link-shaped text out of the model's `answer`
and `reason`, which the closed candidate set does not cover: a hostile
description can talk the model into writing a URL, and a non-degraded answer is
cached for seven days.

Runs on Workers AI (binding `AI`) — free allocation is 10k neurons/day and one
ask costs ~25, so ~400/day is free. `ASK_DAILY_BUDGET` (default 350) caps model
calls per UTC day. Past the cap, or when the model errors or returns
ungroundable output, the route **degrades** rather than failing: it returns the
ranked candidates with `degraded: true` and the UI says the assistant is
unavailable. Degraded answers are never cached, so an outage cannot be pinned
in KV for a week.

**Tests must run without Cloudflare credentials.** The `[ai]` binding is remote,
and by default `@cloudflare/vitest-pool-workers` opens a proxy session to the
real API before any test runs — which fails with no credentials and takes the
whole worker suite down in CI. `vitest.config.ts` sets `remoteBindings: false`
to prevent that. Do not remove it; `ask.test.ts` injects its own `AI` stub.

### Rust client

The Rust client is `src-tauri/src/pack_hub/addon_search.rs`. It treats the index
as an enhancement, never a dependency: on a 503, a network error, or zero hits
it falls back to `crate::esoui::search_esoui`, so search is never worse than it
was and still works offline-ish. `AddonSearchPage.source` reports which backend
answered so the UI can say when it is showing title-only results.

### Rust integration:

- `pack_hub/commands.rs` calls `kalpa-pack-hub.eso-toolkit.workers.dev` (see `pack_hub_url()` and `share_worker_url()`)
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
  override is `debug_addons_dir_override` in `commands.rs`; the env var is read
  only in debug builds, so no shipped binary can be aimed away from a user's
  real folder. Pass `--no-build` when iterating on the specs themselves.

  Two things it is **not**. It is not a CI gate — nothing runs it automatically
  on any platform, so a destructive regression can merge; treat it as local
  validation you run before a release, not a barrier. And its isolation is
  **partial**: only the AddOns folder is throwaway. `settings.json`, the manifest
  cache, uploader history, saved tokens and the WebView2 profile are the
  developer's real files, because Tauri resolves the app-data dir from the bundle
  identifier rather than any environment variable — including the WebView2
  profile, which Tauri always passes explicitly, so `WEBVIEW2_USER_DATA_FOLDER`
  is inert. Every run also empties the manifest-cache DB, because the first scan
  of an empty sandbox prunes it against zero folder names. The runner's header
  documents the exact line. Specs must normalise persisted state they depend on.

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
3. Confirm the release section contains the complete user-facing copy. The
   release workflow generates its `Changed:` section from the matching tagged
   section in `CHANGELOG.md` and fails closed if that section is missing, empty,
   malformed, or duplicated. Preview it with
   `node .github/scripts/release-body.cjs --version v<version>`.
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
