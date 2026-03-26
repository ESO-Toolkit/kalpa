# ESO Addon Manager

You are Claude Code working in this repo.

## Project Overview

An open-source ESO addon manager desktop app. Current state: **functional alpha** with addon scanning, installation, updates, dependency resolution, backups, profiles, character management, API compatibility checks, and Minion migration.

### Stack
- **Desktop client**: Tauri v2 + React 19 + TypeScript + Tailwind v4 + shadcn-ui
- **Backend** (planned): Cloudflare Workers + KV, metadata caching only
- **CI/CD**: GitHub Actions — tag-triggered release builds (Windows NSIS/MSI)

## Important Rules

- Do not use private APIs or hacks
- Prefer public ESOUI pages and direct public download URLs
- Keep scraping centralized and cached (all in `esoui.rs`)
- Do not implement hourly background scraping — use on-open refresh + manual Refresh button
- Optimize for maintainability and simplicity over cleverness

## Architecture

```
src/                    # React frontend
  components/           # Feature components (addon-list, settings, etc.)
  components/ui/        # shadcn-ui primitives
  lib/                  # Utilities (store, utils)
  types.ts              # Shared TypeScript interfaces
src-tauri/src/          # Rust backend
  commands.rs           # All Tauri command handlers
  esoui.rs              # ESOUI HTTP client & HTML scraping
  manifest.rs           # Addon manifest (.txt) parsing
  installer.rs          # ZIP extraction & addon installation
  metadata.rs           # Metadata caching & management
  lib.rs                # Module defs & Tauri app setup
```

## Git Workflow

Use **GitHub Flow**:
1. `master` is always releasable
2. Create short-lived branches: `feat/feature-name`, `fix/bug-name`
3. Open a PR, let CI pass, merge to `master`
4. Tag releases from `master` (e.g., `v0.2.0`) — triggers release CI

### Commits
- Conventional Commits: `type(scope): description`
- Types: feat, fix, docs, style, refactor, test, chore
- Imperative mood, <50 chars, no period

### Releases
- Bump version in 3 files: `tauri.conf.json`, `Cargo.toml`, `package.json`
- Push tag `v*` to trigger `.github/workflows/release.yml`
- Release CI builds Windows NSIS/MSI installers and uploads to GitHub Releases

## Design System

The UI follows the ESO Log Aggregator visual language adapted for shadcn + Tailwind v4.
Before building or modifying any UI, read these context files:

1. `context/40-design-system.md` — Design principles, colors, glass morphism, typography, animations
2. `context/41-component-patterns.md` — Concrete shadcn component recipes
3. `context/42-theme-tokens.md` — CSS variables, @theme inline mappings, Tailwind utilities

Key rules:
- Glass morphism panels (three tiers: primary, default, subtle)
- Space Grotesk for headings, Geist for body text
- 3px colored left-border on cards for addon status
- Animation scale: fast (150ms), normal (250ms), slow (400ms)
- ESO gold (#c4a44a) as primary accent, sky-blue (#38bdf8) for interactive/focus

## How to Work

1. Read relevant context files before starting work
2. Read `context/40-design-system.md` before any UI work
3. Make small, reviewable changes
4. Keep the repo buildable after each change
5. Ask before making large architecture changes

## Available Tools

- `gh` for GitHub operations (PRs, issues, releases)
- `wrangler` for Cloudflare Worker deployment (when backend phase begins)
- Local Rust/Node toolchain (`npm run tauri dev` for development)

## Context Files

- `context/00-overview.md` — Core vision and principles
- `context/10-desktop-client.md` — Desktop client architecture
- `context/20-metadata-worker.md` — Backend worker design
- `context/30-mvp-plan.md` — Original phase roadmap (phases 1-3 complete)
