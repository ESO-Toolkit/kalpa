# Slint Native Port Checklist

This prototype is now a component port workspace. It is not accepted as a
visual match until each component below has been checked against the current
WebView UI.

## Active Surface

Main addon manager.

Reference screenshot:

- `../../.screenshots/main-desktop.png`

Scale note:

- Treat the React/Tailwind layout as logical CSS pixels. The screenshot is roughly
  1.75x scaled, so do not copy raw bitmap pixel measurements into Slint.

## Current Component Target

Main screen P0 fidelity cleanup.

Reference source:

- `../../src/components/app-header.tsx`
- `../../src/components/ui/button.tsx`
- `../../src/components/ui/logo.tsx`
- `../../src/components/app-background.tsx`

States to port:

- normal count display
- checking-updates spinner
- icon button idle/hover/active/focus
- upload/settings/saved-vars/addon-packs buttons
- batch-mode action set
- window controls
- frameless drag region behavior

Theme states to check:

- current WebView reference palette
- prototype no-env reference palette (`eso-gold`)
- app default palette (`KALPA_THEME=nordic-runestone`)
- warm/red palette (`KALPA_THEME=daedric-crimson`)
- cold/blue palette (`KALPA_THEME=coldharbour-frost`)
- textured art themes via generated native skin assets
- custom theme persistence once the prototype is wired into the app store
- runtime custom seed import via `KALPA_THEME_JSON` / `KALPA_THEME_FILE`
- built-in catalog parity via `assets/themes/builtin-themes.json`
- generated native skin parity via `assets/skins/*.svg`

Motion states to check:

- hover fade duration
- active press offset/scale intent
- selected indicator transition
- ambient orb drift
- loading spinner only while loading/checking
- reduced-motion behavior before accepting dialog/upload/live surfaces

## Main Screen Order

- [x] App background and window chrome
- [x] App header
- [x] Header icon buttons and window controls
- [x] Left rail frame
- [x] My Addons/Discover segmented tabs
- [x] Search input
- [x] Filter tabs
- [x] Count/sort bar
- [x] Addon list rows
- [x] Selected addon row
- [x] Detail title and info pills
- [x] Detail tabs
- [x] Detail glass panel
- [x] Tags
- [x] Description panel
- [x] Dependency rows
- [x] Scrollbars

Current row status:

- Visual scaffold exists for favorite, library, disabled, broken, update, edited,
  and ready states.
- Rows are now driven by an exported `AddonEntry` model instead of
  hand-duplicated `AddonRow` markup. The prototype loads real local ESO addon
  manifests from `KALPA_ADDONS_PATH` or the default Windows
  `Documents/Elder Scrolls Online/live/AddOns` path when available, and falls
  back to mock data only when no local AddOns folder can be read.
- The list now uses a Slint `ScrollView` with native wheel scrolling while the
  visible Kalpa scrollbar remains custom-styled and bound to `viewport-y`.
- Clicking a model row updates the selected row highlight and the detail pane's
  title, metadata, facts, and description from the same `AddonEntry`.
- Installed search is now a native editable field backed by the full addon model.
  Search matches title, folder, author, type, and active tags, then rebuilds the
  visible list without losing the full source data.
- The `All / Addons / Libs` filters and compact sort trigger now rebuild the
  visible native addon model from the full source list. Counts come from the full
  model instead of fixed placeholder numbers, and empty search/filter results
  show native empty states instead of indexing a missing detail row.
- Row selection checkboxes now mutate `AddonEntry.selected` state, preserve that
  state through search/filter/sort rebuilds, and switch the native header into a
  batch action strip. Batch disable and remove now use the real AddOns folder
  rename/delete paths when a row is backed by disk, then fall back to in-memory
  prototype behavior for mock rows. Batch tag now persists the active tag model
  for disk-backed rows, but still needs the React tag menu/custom choice. Batch
  update remains a native model action pending the production update path.
- Rows now expose a native right-click context menu with Open Folder, View on
  ESOUI, Favorite/Unfavorite, Enable/Disable, and Remove actions. Enable/disable
  and remove use the same real folder operations as the detail footer for
  disk-backed rows. The menu is currently Slint/native menu styled, not the
  custom React glass context-menu surface.
- The installed list now has a native `FocusScope` for ArrowUp/ArrowDown,
  Home/End, and Space-to-toggle-selection keyboard handling. Auto-scrolling the
  selected row into view and full ARIA/focus-ring parity remain open.
- The count/sort bar now uses a compact native select-trigger scaffold with the
  same rounded hover affordance as the React sidebar.
- The default mock row state has been quieted to match the main reference
  screenshot; status badges and rails remain implemented but are not shown in the
  baseline demo unless mock data enables them.
- Row badges now use the current outline-pill sizing/tone model, including a
  distinct disabled/zinc style.
- Rows can now show multiple right-aligned status pills in the same row for the
  React list's combined update/broken/testing/edited states.
- Status colors are now mixed in OKLab to match the React theme-token model more
  closely than the earlier RGB blend.
- `primary-hover` now uses the React OKLCH lightness/chroma rule instead of RGB
  lightening.
- Not accepted yet: real local manifest loading is not the same as the production
  addon store; ESOUI IDs, update state, disabled state, persisted tags, dynamic
  tag filter tabs, the real select popup, full keyboard focus/auto-scroll
  behavior, custom context-menu styling, production context/batch commands, and
  virtualized large-list behavior are not fully ported.

Current detail status:

- Tag chips now have per-tag active colors and the selected-addon mock has been
  quieted back to the inactive chip state used by the main reference.
- Tag chips now render from `AddonEntry` tag models, and native clicks update the
  selected addon's model state plus the favorite row flag. Disk-backed rows now
  persist tag changes to Kalpa metadata using the production store shape; mock
  rows still use in-memory prototype behavior.
- Tag chips now include the React-style active glow/inset highlight while keeping
  the inactive main-reference chip state quiet.
- Small chips/pills now avoid 1px stroked outlines in the software renderer.
  Slint software AA made the old `All`, folder-name, ESOUI, and tag chip borders
  look pixelated; the prototype now uses low-alpha fills plus subtle highlights
  instead. Skia improves AA only slightly here and measured too high for the
  memory target.
- Detail title has a native two-layer approximation of the React gradient text.
- Detail glass panels now use the textured-theme opacity rule so skin motifs do
  not sit directly behind body text.
- The detail facts panel now follows the React subtle glass values more closely:
  flat `white/2%` fill, `white/4%` border, faint inset top line, and a real
  bottom gutter below the `View on ESOUI` action.
- Detail content is now wrapped in a Slint `ScrollView` with the custom Kalpa
  scrollbar bound to `viewport-y`.
- Detail sections now stack in a real `VerticalLayout`: section headers, spacing,
  facts panel, tag chips, description card, and dependency rows are separate
  layout items instead of one fixed-coordinate sheet.
- Textured themes now have a native token-driven tint wash over the generated
  texture/pattern layer, but exact per-skin CSS gradient composition is still a
  fidelity gap.
- Required and optional dependency rows now render from `AddonEntry`
  dependency models instead of fixed Slint markup, with hover wash, status fills,
  install action, neutral optional-missing rows, and a trash icon scaffold
  preserved.
- The `Details / Files` tab strip is now stateful in the native prototype. The
  Files tab is model-backed with toolbar actions, nested rows, extension pills,
  selected row, edited-file marker, binary guard, edit-enable state, dirty
  tracking, revert/save actions, and an edit-backups panel state. The prototype
  can read/write real files when launched with `KALPA_ADDONS_PATH=<AddOns path>`;
  production still needs shared backend command wiring and CodeMirror-level
  syntax highlighting parity.
- The detail action footer now has the React divider and native actions:
  enable/disable renames disk-backed addon folders to and from `.disabled`, and
  remove deletes enabled/disabled copies plus Kalpa metadata. Mock rows still
  use in-memory behavior so the prototype works without a local ESO folder.
- `View on ESOUI` now opens the selected addon's ESOUI URL from the native
  prototype.
- The sidebar `My Addons / Discover` switch is now stateful. Discover mode has
  native search/popular/category/url sub-tabs backed by a `DiscoverEntry` model,
  editable native search and URL inputs, clickable result rows, selected-row
  detail state, install/reinstall state, and `View on ESOUI` actions. Search,
  popular, category, and URL/ID flows now load ESOUI data, hydrate details in the
  background, and install addons through the shared download/extract/hash/metadata
  path. Popular and category browse chips drive native ESOUI requests for
  downloads/newest/category/sort choices. Popular and category browse modes now
  track loading, page state, and load-more pagination, and the native result rows
  use clearer card-style selection/rank/meta hierarchy to avoid the previous
  stripe-like selected state and cramped metadata. Discover detail hydration now
  pulls trusted ESOUI screenshot URLs into a bounded temp cache and renders the
  native detail gallery with selectable thumbnails, previous/next controls, and
  dynamic screenshot counts. Remaining Discover gaps are category picker polish,
  richer skeleton/error states, and remove/update flows.
- The header Pack Hub action now opens a native Slint Pack Hub overlay covering
  the reference Browse, Create details, Create addons, and install-detail flows.
  Browse now loads published packs from the public Pack Hub `/packs` endpoint
  into native cards with live title, author, type, tag, vote, and addon-count
  metadata, and selecting a pack hydrates the native detail/install addon rows
  from `/packs/{id}`. Browse search, type filters, sort selection, and load-more
  pagination now drive native Pack Hub requests. The install footer now installs
  missing pack addons through the native ESOUI download/extract/hash/metadata
  path, but still lacks the React batch-progress UI, transitive dependency pass,
  installed-pack library tracking, and install-count tracking. Pack detail
  Edit/Delete/Share actions now hand off to the full WebView Pack Hub with the
  selected pack id because those flows still require the React account/session
  and share-code surfaces. Pack browse cards now use the React-style
  deterministic identity model: type accent, hash-derived monogram tile, dynamic
  author initial, and a denser title/type/description/meta hierarchy. The native
  Create flow now has editable title/description/type state and the Addons step
  is backed by real installed addons with ESOUI ids, filter text, selected-addon
  rows, remove actions, and required/optional toggles. My Packs, native
  create/save/publish/export, share/import links, voting, and account/session
  wiring still need production parity before this can replace the React Pack Hub
  implementation.
- The header SavedVariables action now opens a native Slint SavedVariables
  Manager overlay covering the reference Overview, Cleanup, Copy Profile, and
  Editor surfaces. Overview and Cleanup now load real SavedVariables files,
  classify installed/orphaned/system data against the native addon model, show
  real size/profile counts, and delete orphaned files through the shared
  auto-backup cleanup path. Copy Profile now cycles real file/source/destination
  profile choices and calls the shared raw-Lua profile copy command. The Editor
  now loads real files through the shared parser, renders native tree/settings
  rows, toggles boolean settings, previews diffs, saves through the shared write
  path, and restores `.bak` files. Remaining Editor gaps are full text/number
  editing, explicit tree row selection, search/filter behavior, raw mode,
  schema customization, ESO-running guards, and richer error states.
- The Settings > Tools Backup & Restore row now opens a native Slint backup
  overlay covering the main, custom-label, and restore-confirmation states. The
  overlay now lists real settings backups, creates manual backups, restores with
  safety snapshots, deletes backups, reveals the backups folder, and classifies
  character backups as restorable/refused through the shared native backup path.
  Remaining gaps are ESO-running guards, richer failure states, and final visual
  comparison against the WebView dialog.
- The Settings > Tools Characters row now opens a dedicated native Characters
  overlay instead of incorrectly routing to SavedVariables. It loads the roster
  from `AddOnSettings.txt` plus the bounded-memory SavedVariables roster scanner,
  warns when files are unreadable/malformed, and writes server-scoped v2
  per-character backups that preserve account-wide data and same-name NA/EU
  twins. Remaining gaps are exact React grouping/animation polish, per-row busy
  states, and restore cross-linking back into Backup & Restore.
- The Settings > Tools Safety Center row now opens a native overlay backed by
  the shared safe-migration module. It lists snapshots, restores and deletes
  snapshots, runs integrity checks, and displays the operation log. Remaining
  gaps are restore/delete confirmation animations, row busy states, and final
  visual comparison against the WebView dialog.
- The Settings > Tools Minion Migration row now opens a native safe-migration
  wizard backed by the shared migration module. It checks preconditions, creates
  a pre-migration restore point, previews Minion metadata imports, executes the
  metadata-only import, refreshes the native addon model, and writes to the
  Safety Center operation log. Remaining gaps are exact React phase animation,
  explicit per-step busy states, and final visual comparison.
- Settings > Tools app updates now run a native manifest check against the
  Tauri updater `latest.json`, compare prerelease versions correctly, and hand
  off to the WebView host only for the signed updater install/restart flow. This
  preserves Tauri's signature verification while avoiding a dead placeholder in
  native mode. Remaining gap is a fully Slint-hosted signed updater, if we decide
  to reimplement signature verification outside Tauri.
- The header Log Uploader action now opens a native Slint uploader shell backed
  by real ESO log discovery. Manual mode lists recent combat logs from the
  production Logs folder, streams the selected file for bounded-memory preflight
  counts, and previews recent fights without loading the whole log into memory.
  Upload and Live Logging still hand off to the signed WebView uploader flow
  until the report/session transport is ported natively; the native surface is
  intentionally honest about that route instead of showing fake progress.
- Detail dependency install/remove affordances still mutate the selected addon's
  dependency models in memory. Production install/remove still needs the existing
  backend/network command path.
- Detail body still needs conflict/update banners, production-store wiring,
  production Discover backend integration, and final native file-editor backend
  integration before it can replace the React surface.
- Current React source includes `Details / Files` tabs; the older
  `.screenshots/main-desktop.png` reference does not. Verify against the running
  WebView before accepting that surface.
- The right-side header actions now render from the current app action set and
  are anchored to the live window width. Do not tune header placement against a
  DPI-virtualized `PrintWindow` crop.

Current backdrop status:

- The native backdrop now follows the React `AppBackground` structure more
  closely: theme base color, optional generated skin layer, three animated
  ambient theme-token orbs, and a light diagonal wash. The orb layers now use
  low-resolution pre-blurred image sprites generated from the active theme in
  Rust, which avoids the blocky/pixelated look of Slint radial-gradient stacks
  while preserving the slow drift animation.
- The previous native-only bottom darkening layer was removed because it created
  a blockier lower-right surface than the WebView background.
- Tiny rounded chips and pills use theme-aware nine-slice backplates generated
  from the active theme in Rust. This keeps custom themes intact while avoiding
  the worst software-rendered hairline artifacts on chip borders.
- The sidebar `All / Addons / Libs` filter now uses one shared animated
  backplate that slides/resizes between tabs, matching the React shared-layout
  indicator pattern more closely than per-tab active repainting.
- The Discover `Search / Popular / Categories / URL` sub-tabs now use the same
  shared animated backplate approach. The `Popular` tab was verified to move the
  indicator and switch the native Discover list state.

## Rules

- Use real app assets where possible.
- Do not use placeholder letters for icons in accepted components.
- Do not add UI that is not present in the current app.
- Capture and compare after each component family.
- Defer release memory measurements until the active surface is visually and
  functionally acceptable.
- Test renderer tradeoffs explicitly. `KALPA_RENDER_PRESET=low-memory` maps to
  `winit-software`; `KALPA_RENDER_PRESET=standard` maps to `winit-femtovg` in
  this prototype unless `KALPA_SLINT_BACKEND` overrides it for a manual renderer
  check. Use `winit-skia` only when the active Slint build exposes it.
- Treat glass/border work as one native material system: pre-blurred assets,
  generated nine-slice/backplate assets, low-alpha fills, and carefully limited
  highlights. Do not keep chasing CSS `backdrop-filter` with per-component hacks.
- OS window blur/mica/acrylic remains a follow-up via the Slint winit-window
  access path; do not fake it inside component code.
- Historical renderer memory notes from July 1, 2026, for later re-check after
  visual/feature parity:
  `winit-software` ~39.5-43.4 MB working set / 21.1-23.5 MB private after
  theme-aware nine-slice chip backplates, downsized animated pre-blurred orb
  sprites, and the shared sidebar filter indicator during a clean release
  restart/wait. Repeated desktop capture/visual-QA can temporarily warm the
  process to ~55 MB working set, so workload memory should be measured
  separately from capture tooling. A Skia renderer spot-check for chip AA
  measured ~104 MB working set / 159 MB private and is not the default
  low-memory path.
  `winit-femtovg`
  ~84 MB / 132 MB, `winit-skia` ~86 MB / 132 MB.
- Keep timers state-gated; idle UI must not run animation timers. The low-memory
  preset disables ambient backdrop motion by default, while the standard preset
  enables it unless `KALPA_AMBIENT_MOTION=0`.
- Launch with `KALPA_REDUCED_MOTION=1` to inspect the native reduced-motion token.
- Launch with `KALPA_ADDONS_PATH=<AddOns path>` to inspect a specific local addon
  folder. Without the env var, the Windows default live AddOns path is used when
  present.
- Launch with `KALPA_DETAIL_TAB=files` to inspect the native Files-tab scaffold.
- Launch with `KALPA_VIEW=discover` and optional
  `KALPA_DISCOVER_TAB=popular|categories|url`, `KALPA_DISCOVER_QUERY=<query>`,
  or `KALPA_DISCOVER_URL=<url-or-id>` to inspect Discover scaffolds.
- Launch with `KALPA_PACK_HUB_OPEN=1` and optional
  `KALPA_PACK_HUB_VIEW=browse|create-details|create-addons|install-detail` to
  inspect native Pack Hub scaffolds.
- Launch with `KALPA_SVM_OPEN=1` and optional
  `KALPA_SVM_VIEW=overview|cleanup|copy-profile|editor` to inspect native
  SavedVariables Manager scaffolds.
- Launch with `KALPA_BACKUP_RESTORE_OPEN=1` and optional
  `KALPA_BACKUP_RESTORE_VIEW=main|custom-label|restore-confirm` to inspect
  native Backup & Restore scaffolds.
- Launch with `KALPA_CHARACTERS_OPEN=1` to inspect the native Characters
  overlay.
- Launch with `KALPA_SAFETY_OPEN=1` to inspect the native Safety Center overlay.
- Launch with `KALPA_MIGRATION_OPEN=1` to inspect the native Minion Migration
  overlay.
- Launch with `KALPA_UPLOADER_OPEN=1` and optional
  `KALPA_UPLOADER_VIEW=manual|live` to inspect the native Log Uploader overlay.
- Launch with `KALPA_RENDER_PRESET=standard` for visual-fidelity checks, or
  `KALPA_SLINT_BACKEND=winit-skia` / `winit-femtovg` for direct backend checks
  on Slint builds that support those renderer names.
- Launch with `KALPA_THEME=<theme-id>` to inspect supported native seed themes.
  The prototype launches with `eso-gold` when no theme env var is set so the
  saved main-screen reference and native demo start from the same clean palette.
- Regenerate built-in native theme seeds with `npm run export:native-themes`.
- Regenerate built-in native skin assets with `npm run export:native-skins`.
- Regenerate both with `npm run export:native-assets`.
- Launch with `KALPA_THEME_JSON=<seed-json>` or `KALPA_THEME_FILE=<path>` to inspect
  exported custom theme seeds.
- Bundle/use the current app fonts before accepting typography-sensitive work.

## Screenshot Diff Harness

For repeatable full-window state captures on Windows, use the DPI-aware capture
harness from `prototypes/slint-kalpa`:

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\capture-states.ps1 -Build -OutputDir .\captures\verify -State main,discover-popular,files,files-editing,settings-general,settings-theme-editor,packhub-browse,packhub-create1,packhub-create2,packhub-install,svm-overview,svm-cleanup,svm-copy,svm-editor,backup-restore-main,backup-restore-label,backup-restore-confirm,characters,safety,migration
```

The harness launches a fresh prototype process per state, uses the low-memory
renderer preset by default, captures the largest visible Slint-owned window, and
writes ignored PNGs under `captures/`. Supported states include `main`,
`discover-popular`, `discover-search`, `discover-category`, `discover-url`,
`files`, `files-editing`, `settings-general`, `settings-appearance`,
`settings-theme-editor`, `settings-tools`, `settings-data`, `packhub-browse`,
`packhub-create1`, `packhub-create2`, `packhub-install`, `svm-overview`,
`svm-cleanup`, `svm-copy`, `svm-editor`, `backup-restore-main`,
`backup-restore-label`, `backup-restore-confirm`, `characters`, `safety`,
`migration`, `theme-crimson`, and `theme-frost`.

From `prototypes/slint-kalpa`, compare a captured native prototype PNG against
the current WebView reference:

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\screenshot-diff.ps1 .\captures\native-main.png
```

The default baseline is `../../.screenshots/main-desktop.png`. The images must
have identical pixel dimensions; recapture the native window at the same nominal
size if the script reports a mismatch. By default, the diff PNG is written under
`tools/diff-output/`.
