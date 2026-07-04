# Native Slint ↔ WebView Parity Backlog

Status snapshot for `codex/slint-native-ui-port`. Produced from a full WebView-vs-native
comparison of every surface (Discover, main list/detail, Pack Hub, SavedVariables, theme
editor, uploader, backup/restore, safety, migration, characters).

Anchors are `ui/kalpa.slint` and `src/main.rs` in this crate unless noted; React anchors are
`../../src/…`.

## Already shipped (PRs on this branch)

- **Input/click, scroll, drag, discover, refresh** — PR #234 commit `ef864c9c`. Root cause
  was a Slint 1.17 gotcha: a `TouchArea` with a computed width but no explicit `x` gets a
  misplaced hit region. Fixed the "All" filter tab, the addon-row checkbox rail, and pack
  cards; added draggable scrollbars; winit window drag; `cdn-eso.mmoui.com` screenshots;
  rebuilt the Discover detail header; refresh spinner feedback.
- **Parity polish** — PR #234 commit `78b0898a`. Uploader route-label honesty + wording;
  Pack Hub selected-card overlap, detail description width, author cap; main description wrap.
- **Safety Center delete confirmation** — `5244c47d`. One-click snapshot delete now confirms.
- **SavedVariables value editors** — `89134e36` (MUST-FIX #1 below, now DONE). Visible
  `TextInput`s that commit on Enter/blur; no per-keystroke reset, no partial-number rejection.
- **Theme skin picker** — the theme editor now exposes all 8 Elder Scrolls skins + None;
  picking one live-applies and it persists on save/export (the "theme skin picker" feature
  below, now DONE). Verified: picker renders, selection applies.
- **Uploader fight note** — the preview no longer silently drops fights; an honest line
  clarifies the uploader hands off the entire log.
- **Uploader richer fight preview** — `scan_native_uploader_log` now tracks the zone from
  `ZONE_CHANGED` and titles each fight by zone, adds quick-reset / long-pull duration hints,
  and raises the preview cap 4 → 6 (`main.rs`). Tested.
- **Uploader split workbench (ToS-safe)** — the production splitter/scanner
  (`src-tauri/src/uploader/{types,tail_io,scanner,splitter}.rs`) are now `#[path]`-included
  and driven by a native Split modal (`SplitWorkbench` in `kalpa.slint`): scan once on open,
  By-fight / By-session modes, per-item toggle, select-all/clear, and byte-range writes to
  CFA-safe app-data (`%APPDATA%\Kalpa\log-splits`). Upload still hands off; only the local
  slice is native. Verified end-to-end (scan → select → split → valid single-fight logs on
  disk) and unit-tested (`split_plan_*`, `uploader_split_fight_title_*`). Brought 35 of the
  production splitter/scanner tests into the prototype suite (now 404 pass).

## Ship-readiness (all green)

- Prototype: `cargo build`, `cargo test` (364 pass / 0 fail), `cargo clippy`, `cargo fmt`.
- Main app: `src-tauri` `cargo check` clean; frontend `npm run check` (tsc/eslint/prettier) clean.
- Visual QA: every overlay renders real data with no dead buttons. One recurring cosmetic
  issue below (button-label truncation).
- Performance toggle: smooth (Standard/femtovg) is already the default; low-memory
  (`winit-software`) is env-only/advanced — matches the intended final decision.

---

## MUST-FIX (real bugs / data loss)

> **Update:** #1, #2, #3 (discard-on-close/switch), #5, #6, #7 are now **DONE** on this
> branch and verified. Still **OPEN**: #4 (Pack Hub create tags — low-value, publish is a
> handoff), and the ESO-running-*save* guard from #3 (the discard-on-close/switch confirm is
> done; a "SavedVariables save while ESO is running" warning is not yet wired).

1. **SavedVariables string/number editors are effectively unusable.** kind 1/2 rows overlay
   an invisible `TextEdit` (`opacity: 0.01`, `kalpa.slint:9406,9432`) and fire `value-edited`
   per keystroke; `apply_svm_editor_state` (`main.rs:14821-14853`) rebuilds the whole model
   each time, resetting the caret, and partial numeric input (`""`, `"-"`, `"1."`) is rejected
   mid-typing (`main.rs:15180`). Fix: render a *visible* field, commit on Enter/blur (Slint
   `LineEdit.accepted`, not `TextEdit.edited`), and stop rebuilding `svm-editor-settings` on
   scalar edits (update the row in place). React commits on blur (`sv-controls.tsx:57-61`).
2. **SavedVariables branch tree: no search + hard 80-row cap** hides deep branches with no way
   to reach them (`svm_editor_tree_entries` returns at `rows.len() >= 80`, `main.rs:14902,14910`;
   only Expand/Collapse exist, `kalpa.slint:9859`). Add a filter field bound to the tree
   (React `matchesSearch`/`hasMatchingDescendant`, `saved-variables.tsx:609-632`) and lift the
   cap for the filtered set.
3. **SavedVariables: unsaved edits silently discarded** on file switch (`on_svm_editor_select_file`
   → `select_next_file`, `main.rs:4533`) and on close (`kalpa.slint:9540`) with no dirty check;
   **no ESO-running guard on save** (`save_svm_editor_file`, `main.rs:15209`). Both risk data
   loss. React confirms discard (`saved-variables.tsx:1496`) and warns on `esoRunning`
   (`:1484`). Reuse `is_eso_running_blocking()` (`main.rs:106`).
4. **Pack Hub "Create → Details" preset tags are dead controls** — seven `PackHubTag` with no
   `clicked`/active state and a static `"0/5"` counter (`kalpa.slint:6756-6762`); tags never
   reach `export_pack_hub_create_file` (`main.rs:4293`), so saved `.esopack` tags are always
   empty. Wire a toggle + counter + export (mirror the addon toggles at `main.rs:4240`).
5. **No single-addon update path.** `DetailPane` has no "update available" banner/button and
   `AddonRow`'s context menu (`kalpa.slint:1434-1497`) has no "Update" item, so an addon with
   the Update badge (`main.rs:2332`) can only be updated via the global batch button. React
   shows "v1 → v2 [Update]" with progress (`addon-detail.tsx:409-441`) and an "Update" context
   item (`addon-list.tsx:366`).
6. **Safety Center delete/restore fire with no confirmation** (`SafetySnapshotRow`,
   `kalpa.slint:5221-5222`) — one click deletes a snapshot. React gates both behind a confirm
   (`safety-center.tsx:189-276`). The inline-confirm pattern already exists in `BackupListRow`
   (`kalpa.slint:7987-8016`); reuse it.
7. **Discover URL/ID tab is a stub** — a malformed/unknown URL shows the same generic empty
   card as "no input", with no "couldn't resolve" feedback (`DiscoverModePanel` tab==3,
   `kalpa.slint:2135`; auto-resolve `main.rs:3096`). React `UrlContent` has a resolve button,
   confirmation card, and error state (`discover-panel.tsx:815-991`).

## PARITY GAPS (missing states / affordances)

- **Missing async states everywhere** (cross-cutting): loading skeletons + error panels for
  Discover detail (`kalpa.slint:11670`), Files tab (`2909`), Characters (`5100`), Backups
  (`8265`); per-action in-progress/confirm feedback for installs, restores, backups. React has
  all of these.
- **Discover detail:** no install-success banner; MD5 not click-to-copy (`kalpa.slint:11888`);
  search tab has no "no results" state (`2140`); RichDescription/BBCode rendered as raw text.
- **Main list/detail:** description shows 2 lines now but React shows full via RichDescription;
  row badges capped at 3 vs React's up to 7 (`kalpa.slint:1363`); dependency rows can't "Update"
  an outdated-but-satisfied dep (`2446`); empty-list state is generic (no onboarding CTAs / clear
  filters, `12677`).
- **Pack Hub:** browse description is 1 line vs React 2 (`5961`); import meta concatenates
  author+date (`7259`); install list is flat (no Required/Optional grouping or Select-all,
  `7020`); per-addon notes/ESOUI links not rendered (`6499`); no install progress bar; create
  step 2 is installed-only (no "Search ESOUI" source, `6802`); single `tag` per pack vs React's
  `tags[]` (`kalpa.slint:80`); sort label "Votes" vs React "Top Voted" (`main.rs:4913`, has a
  test at `20330`).
- **Backup/Safety/Migration/Characters:** dead "What gets backed up?" control (`kalpa.slint:8111`);
  migration lacks a distinct "importing" phase + categorized preview diff (`5620`,`5539`);
  Characters not grouped by megaserver (React `characters.tsx:132`).
- **Button-label truncation** at capped modal widths — icon+label buttons crammed in fixed rows
  shrink and elide ("Save Chan…", "Rest…", "Re-c…", "Refre…") in the SVM editor toolbar, Safety,
  migration, characters. `SmallAction`/`LinkAction` auto-size (`kalpa.slint:943,985`) but the
  containing rows constrain them; give those rows more width or shorter labels / icon-only at
  narrow widths.

## LARGER FEATURES (deliberate builds; some need a product call)

- ~~**Uploader split workbench (ToS-safe).**~~ **DONE** (see "Already shipped"). Built on the
  production splitter/scanner rather than the prototype's own scan, so it gets real per-session
  offsets/size/realm + per-fight offsets + zone/boss names for free; writes to app-data (not the
  Logs folder) to dodge Controlled Folder Access.
- **Uploader fight richness + history (ToS-safe).** Zone titling + duration hints + raised cap are
  **DONE** (see "Already shipped"). Still open, low-value under the handoff model: *boss*-name
  titling in the *inline* preview (the split modal already does boss > zone > ordinal via the
  production scanner); local handoff history + paste-report-link reopen (`parseReportCode`) — worth
  little because the handoff never returns a report code to Kalpa; pass guild through the CLI
  (currently `--guild null`, `main.rs`).
- **Theme skin picker (new feature for BOTH apps).** Neither editor exposes skins today; custom
  themes are color-only by design in React and native. Native is cheap to add — the skin runtime
  (`Tokens.skin-kind`, 8 `assets/skins/*.svg`, `skin_id` model, import preservation) is already
  built. Add a `draft-skin-id` + picker (`AppearanceOverlay`), wire `on_theme_draft_updated`/
  `save`/`export` (`main.rs:8198,8176,8285`). This is a product decision, not a native regression.
- **SavedVariables polish:** color-table picker (`main.rs:14964`), multiline/long-string handling
  (`main.rs:15052`), file dropdown instead of cycle (`kalpa.slint:9803`), visual diff preview
  (data already computed, `main.rs:15229`).

## ARCHITECTURE-GATED (needs a Kalpa-owned WebView, or a product decision)

- **Native ESO Logs upload / sign-in / live direct streaming.** CORRECTION to an earlier note in
  this file: production *does* ship a native uploader (`src-tauri/src/uploader/native/*` +
  `uploader_login_esologs` / `uploader_upload_log` / `uploader_start_live`), opt-in with the Archon
  handoff as the default. It is NOT ToS-blocked. What the Slint prototype lacks is the **sign-in
  surface**: `uploader_login_esologs` opens "a webview Kalpa owns" for ESO Logs' OAuth, and a
  WebView-less Slint shell has none — so there is no session cookie, and `uploader_upload_log` /
  `uploader_start_live` are gated downstream of it. The encoder + transport
  (`native::{encode,events,format,client}`) are pure Rust and portable, but wiring them for a
  "signed-in-elsewhere-only" path (reading the shared `StoredSessionProvider` cookie) is a large
  port that yields a partial UX; do it only on a product decision. Until then the upload stays the
  external Archon App handoff (`launch_external_uploader`, `main.rs`). The **split workbench above**
  is the shipped ToS-safe slice of "uploader parity".
- **Authenticated Pack Hub mutations** — edit/delete ("Manage"), publish/draft, private
  share-codes, voting. These correctly defer to the signed-in WebView Pack Hub
  (`return_to_webview_shell`, `main.rs:4046,4138`). Local `.esopack` export stays native.
- **Full raw SavedVariables text editor** — deliberately out of the first shippable editor; the
  "Raw → clipboard" action (`main.rs:15260`) is the intended inspection path.

## Corrections to stale assumptions (verified during this pass)

- Theme editor **already** directly edits custom themes and forks built-ins (PORTING note is
  stale) — not a gap.
- Pack Hub **import preview already matches React** (read-only, required-only) — the only cleanup
  is removing the unreachable `pack_hub_import_addon_selection_toggle` handler (`main.rs:3861`).
- Native theme **import preserves `skinId`** (ahead of React, which drops it).
- Backup "stale" status, character skipped-files warning, and the 5-item migration precondition
  checklist are all correctly wired — not gaps.
