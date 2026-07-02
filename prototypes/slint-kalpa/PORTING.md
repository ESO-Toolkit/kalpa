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
  batch action strip. Batch update/disable/tag/remove/clear are in-memory model
  actions only; production still needs the real backend command and confirmation
  paths.
- Rows now expose a native right-click context menu with Open Folder, View on
  ESOUI, Favorite/Unfavorite, Enable/Disable, and Remove actions. The actions are
  wired to the prototype model/open helpers; the menu is currently Slint/native
  menu styled, not the custom React glass context-menu surface.
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
  selected addon's model state plus the favorite row flag. Production tag
  persistence still needs backend/store wiring.
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
- The detail action footer now has the React divider and native model actions:
  enable/disable toggles the selected addon's model state, and remove deletes the
  selected mock row in memory only. Production remove still needs the backend
  command/confirmation flow.
- `View on ESOUI` now opens the selected addon's ESOUI URL from the native
  prototype.
- The sidebar `My Addons / Discover` switch is now stateful. Discover mode has
  native search/popular/category/url sub-tabs backed by a `DiscoverEntry` model,
  editable native search and URL inputs, clickable result rows, selected-row
  detail state, install/reinstall state, and `View on ESOUI` actions. Search and
  URL currently resolve against prototype data; production Discover still needs
  network/search/detail payloads, screenshots, and backend install/remove command
  wiring before the right pane can be accepted.
- The header Pack Hub action now opens a native Slint Pack Hub overlay covering
  the reference Browse, Create details, Create addons, and install-detail flows.
  The flow is still prototype-data backed; production needs real pack storage,
  import/export, publish, voting, install, and account/session wiring before it
  can replace the React Pack Hub implementation.
- Detail dependency install/remove affordances now mutate the selected addon's
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
powershell -ExecutionPolicy Bypass -File .\tools\capture-states.ps1 -Build -OutputDir .\captures\verify -State main,discover-popular,files,files-editing,settings-general,settings-theme-editor,packhub-browse,packhub-create1,packhub-create2,packhub-install
```

The harness launches a fresh prototype process per state, uses the low-memory
renderer preset by default, captures the largest visible Slint-owned window, and
writes ignored PNGs under `captures/`. Supported states include `main`,
`discover-popular`, `discover-search`, `discover-category`, `discover-url`,
`files`, `files-editing`, `settings-general`, `settings-appearance`,
`settings-theme-editor`, `settings-tools`, `settings-data`, `packhub-browse`,
`packhub-create1`, `packhub-create2`, `packhub-install`, `theme-crimson`, and
`theme-frost`.

From `prototypes/slint-kalpa`, compare a captured native prototype PNG against
the current WebView reference:

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\screenshot-diff.ps1 .\captures\native-main.png
```

The default baseline is `../../.screenshots/main-desktop.png`. The images must
have identical pixel dimensions; recapture the native window at the same nominal
size if the script reports a mismatch. By default, the diff PNG is written under
`tools/diff-output/`.
