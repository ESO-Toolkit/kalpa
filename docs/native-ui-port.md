# Native UI Port Plan

Kalpa can move off WebView only if the native UI is rebuilt as a component-by-component
port of the current React/Tailwind UI. Freehand approximations are not acceptable.

## Fidelity Target

The target is visually indistinguishable in normal use from the current Kalpa UI.
Literal pixel identity is not guaranteed because native rendering differs from
Chromium in text antialiasing, blur, shadows, and fractional layout. The working
standard is:

- Same layout structure, density, hierarchy, colors, typography scale, and motion intent.
- Same visible states for each component: idle, hover, active, selected, disabled,
  loading, focus, empty, error, and success where applicable.
- Screenshot comparison against the current WebView app before moving to the next
  surface.
- No invented product UI. If it is not in the current app or a planned state, it
  does not belong in the native prototype.

## Reference Source

Primary visual reference:

- `.screenshots/main-desktop.png`

Primary implementation reference:

- `src/components/app-header.tsx`
- `src/components/addon-list.tsx`
- `src/components/addon-detail.tsx`
- `src/components/discover-panel.tsx`
- `src/components/discover-detail.tsx`
- `src/components/app-background.tsx`
- `src/components/ui/button.tsx`
- `src/components/ui/tabs.tsx`
- `src/components/ui/input.tsx`
- `src/components/ui/glass-panel.tsx`
- `src/components/ui/info-pill.tsx`
- `src/components/ui/logo.tsx`

## Process

1. Port one component family at a time.
2. Use current React source for measurements, colors, spacing, and states.
3. Use logical CSS pixels from the React layout, not raw screenshot pixels.
   The current `main-desktop.png` reference is approximately 1.75x scaled:
   the 48 CSS px header appears around 84 bitmap px, and the 380 CSS px
   sidebar appears around 665 bitmap px.
4. Render only enough native UI to inspect that component in context.
5. Capture the current WebView reference and native prototype at the same window size.
6. Compare by eye first, then use image diff once component boundaries are stable.
7. Do not proceed to the next surface until the current component family is acceptable.

## Renderer Strategy

Kalpa has two competing native-renderer goals:

- Standard preset: use Slint's `winit-femtovg` by default in this prototype,
  with `winit-skia` allowed when the active Slint build exposes it. This is the
  fidelity track for richer glass, stronger shadows, more motion, and
  renderer-specific antialiasing comparisons.
- Low-memory preset: use Slint's `winit-software` renderer. This has measured
  much lower memory, but it makes faithful shadows and glass harder. It uses
  pre-blurred/static assets, translucency, and a stricter animation budget.

Do not assume the low-memory software renderer is the final renderer for the
native app. Do not keep measuring memory while a surface is still visibly or
functionally wrong; first make the surface acceptable, then measure both fidelity
and memory if the renderer choice affects the result.

The prototype defaults to `KALPA_RENDER_PRESET=low-memory` to protect the memory
target. Launch with `KALPA_RENDER_PRESET=standard` for the standard/fidelity
track. `KALPA_SLINT_BACKEND=winit-femtovg`, `KALPA_SLINT_BACKEND=winit-skia`, or
`KALPA_SLINT_BACKEND=winit-software` can still override the backend directly for
manual renderer checks.

Native glass must be implemented as a small design system instead of ad hoc
per-component approximations. The accepted low-memory path is:

- generated/pre-blurred backdrop and orb assets;
- low-alpha surface fills and subtle image/backplate highlights instead of
  software-rendered hairline borders on tiny rounded chips;
- static/translucent panel materials where CSS `backdrop-filter` would otherwise
  be too expensive or unavailable;
- OS window blur/mica/acrylic only as an additive shell-level enhancement after
  confirming the platform hook. The current Slint API exposes access to the
  underlying winit window, but this prototype does not yet wire a stable native
  blur implementation.

Measured on July 1, 2026 for the current main-screen prototype:

- `winit-software`: about 39.5-43.4 MB working set, 21.1-23.5 MB private memory
  after a clean release restart/wait with theme-aware nine-slice chip/pill
  backplates, downsized animated pre-blurred orb sprites, and the shared sidebar
  filter indicator. Repeated desktop capture/visual-QA can warm the process to
  about 55 MB working set, so workload memory should be measured separately from
  capture tooling.
- `winit-femtovg`: about 84 MB working set, 132 MB private memory.
- `winit-skia`: about 86 MB working set, 132 MB private memory. A later chip-AA
  spot-check measured about 104 MB working set, 159 MB private memory after
  launch, so Skia is not currently the default renderer.

## Theme Requirements

The current WebView app treats a theme as 12 seed colors plus an optional skin.
The CSS layer then derives surfaces, hover colors, status colors, glass tints,
scrollbars, chart colors, and backdrop orbs using OKLab/OKLCH color math. The
native port must preserve that model:

- Keep the same seed fields as `src/lib/theme-types.ts`.
- Generate native theme tokens from the seed instead of hardcoding per-component
  colors.
- Preserve built-in theme ids, custom theme persistence, and the pre-paint/no-flash
  behavior when the native app becomes the primary shell.
- Treat Elder Scrolls skins as a separate material layer. If Slint cannot render
  the exact CSS texture/pattern stack, record the gap and implement the closest
  native background layer before accepting the theme surface.
- Check every accepted component under at least the current reference theme, one
  warm/red theme, one cold/blue theme, and one textured art theme.

The Slint prototype now exposes a `Tokens` global, a Rust-side seed bridge, a
generated built-in theme catalog from `src/lib/theme-presets.ts`, generated
native skin SVGs from `src/lib/theme-skins.ts`, and runtime import for the same
12-color custom theme shape via `KALPA_THEME_JSON` or `KALPA_THEME_FILE`. Full
theme objects with `skinId` can activate the native skin layer. That proves the
native path does not need per-component hardcoded themes, but production still
needs app-store integration for persisted active/custom theme selection.

The prototype now launches with the older ESO Gold/root palette when no theme env
var is set so the native demo starts from the same clean palette as
`.screenshots/main-desktop.png`. The app factory default remains available with
`KALPA_THEME=nordic-runestone`, and all generated built-in/custom seed paths still
work through `KALPA_THEME`, `KALPA_THEME_JSON`, and `KALPA_THEME_FILE`.

The addon list is now model-backed in the Slint prototype: Rust keeps a full
`AddonEntry` source list plus a visible list, the sidebar renders rows from the
visible model, and the list uses a Slint `ScrollView` for real wheel scrolling
while retaining the custom Kalpa scrollbar overlay. The prototype loads real
local ESO addon manifests from `KALPA_ADDONS_PATH` or the default Windows
`Documents/Elder Scrolls Online/live/AddOns` path when available, with mock data
only as fallback. Row clicks now drive the selected-row highlight and detail pane
from the same model entry. Installed search is an editable native field that
filters title/folder/author/type/active tags, `All / Addons / Libs` filters
rebuild the visible model, sort cycling reorders it, and empty results show a
safe native empty state. The remaining list work is connecting this to the
production addon store for update/disabled/persisted tag state, dynamic tag
filters, the real sort popup, keyboard focus, context menus, batch selection, and
large-list virtualization/perf validation.
Row selection checkboxes now mutate native model state, survive search/filter/sort
rebuilds, and switch the header into a batch action strip. The batch
update/disable/tag/remove/clear controls are still prototype-only model actions;
production command wiring, confirmation flows, and backend persistence remain.
Rows also expose a native right-click context menu for Open Folder, View on
ESOUI, Favorite/Unfavorite, Enable/Disable, and Remove. The actions are wired to
prototype model/open helpers, but the menu uses Slint/native menu styling rather
than the custom React glass context-menu surface.
The installed list has basic native keyboard handling for ArrowUp/ArrowDown,
Home/End, and Space-to-toggle-selection. Auto-scroll-to-selected, complete focus
rings, and the exact React listbox accessibility model remain open.

The detail pane is also wrapped in a Slint `ScrollView` with the custom Kalpa
scrollbar bound to the scroll viewport. Its top-level body now uses a Slint
`VerticalLayout` with separate section headers, panels, tag chips, description,
and dependency-row bodies. Required and optional dependencies now flow through
`AddonEntry` dependency models instead of fixed Slint rows. The Files tab is now
model-backed in the native prototype with nested rows, selected-file state,
binary/read-error handling, edit-enable state, dirty tracking, revert/save, and
an edit-backups panel state. It can read/write real files when launched with
`KALPA_ADDONS_PATH=<AddOns path>`, but production still needs shared Tauri command
wiring and CodeMirror-level syntax highlighting parity. The next fidelity step
is replacing the remaining mock detail data with production models for long
descriptions, banners, dependency actions, Discover backend payloads, and
persisted tag writes. The Discover shell and right-pane detail are now backed by
a native `DiscoverEntry` model with clickable rows, selected detail state,
install/reinstall state, editable native search and URL inputs, and
`View on ESOUI`; production still needs network search/browse/detail wiring,
screenshot payloads, and the existing backend install/remove command path.
Tag chips now render from `AddonEntry` tag models and native clicks update the
selected addon's in-memory model state; persisted tag writes still need
production store/backend integration.
The detail action footer now has native model behavior for enable/disable and
in-memory mock row removal, and `View on ESOUI` opens the selected addon's ESOUI
URL. Production remove still needs the existing backend confirmation/delete flow.
Dependency install/remove controls now update the selected addon's in-memory
dependency models; production install/remove still needs the existing backend and
network command path.
The detail facts panel now uses the same subtle glass model as the React source:
flat low-alpha fill, faint `white/4%` outline, inset top highlight, and padded
`View on ESOUI` action spacing.
Small chips/pills avoid software-rendered 1px outlines because those looked
pixelated compared with Chromium's antialiased borders. The prototype now uses
low-alpha fills and subtle highlights for the `All`, folder-name, ESOUI, and tag
chips.

## Motion Requirements

The React app uses CSS transitions, Framer Motion springs, hover shines, spinners,
counting numbers, shimmer, staggered dialog entrances, and reduced-motion rules.
Native equivalents must be built component by component:

- Use Slint `states` and `animate` blocks for cheap hover, active, focus, selected,
  disabled, and expanded states.
- Use `Timer` only for bounded or state-gated animations such as loading spinners;
  timers must not run while idle.
- Enforce the low-memory preset as a hard animation budget: no ambient backdrop
  drift by default, no always-running decorative timers, and only cheap property
  animations on visible/interactive surfaces.
- Use the standard preset for richer motion checks after the static visual
  structure is acceptable.
- Wire reduced-motion through a native token before accepting each animated
  surface. The prototype currently supports `KALPA_REDUCED_MOTION=1` for ambient
  backdrop drift and loading spinner timers.
- Recreate spring/layout motion intent with tuned native durations and easing, then
  compare by eye against the WebView behavior.
- Add a reduced-motion switch before accepting settings, dialogs, upload/live
  logging, or any long-running animated surface.
- Capture motion-sensitive components in short clips or repeated screenshots before
  marking them accepted.

## Known Hard Limits

- Slint does not provide a browser-equivalent `backdrop-filter: blur(...) saturate(...)`.
  Glass panels must be approximated with tuned translucent fills, pre-blurred
  backgrounds, gradients, and static background layers.
- Elder Scrolls skin SVG textures/patterns are generated from the React source,
  and the prototype now adds a native skin-tint gradient stack for the large
  warm/cool washes. Per-skin CSS gradient details and browser blend behavior are
  still approximations.
- The default non-skin backdrop now follows the React `AppBackground` structure:
  theme base color plus three ambient theme-token orbs. The old native-only bottom
  darkening layer was removed because it made the lower-right surface too blocky.
  The native prototype now generates low-resolution pre-blurred orb sprites from
  the active theme in Rust and animates those sprites in Slint, which is closer
  to Chromium's blurred-orb look than live Slint radial gradients while staying
  well under the 50 MB prototype target.
- Chromium text rendering will not be exactly reproduced. Bundle and use the
  current app fonts (`Geist Variable` and `Space Grotesk Variable`) before judging
  typography.
- Framer Motion spring/layout animations do not map directly to Slint. Recreate
  the motion intent using Slint state/property animations.
- CSS `color-mix()` and OKLCH-derived theme tokens must be precomputed or manually
  represented in native theme data. The prototype now uses OKLab mixing for
  semantic status colors and the OKLCH lightness/chroma adjustment for
  `primary-hover`, but chart colors and remaining CSS-derived tokens still need
  parity work.
- Tiny rounded chips/pills currently avoid 1px outlines and use low-alpha theme
  fills instead. The remaining software-renderer corner antialiasing gap should
  be solved with a theme-aware image backplate or renderer-specific path after
  the current visual structure is accepted.
- Custom titlebar behavior needs native window glue for drag, double-click
  maximize, minimize, close, snap, and resize hit zones.

## Main Screen Component Order

1. App background and window chrome
2. App header
3. Header icon buttons and window controls
4. Left rail frame
5. My Addons/Discover segmented tabs
6. Search input
7. Filter tabs
8. Count/sort bar
9. Addon list rows
10. Selected addon row
11. Detail title and info pills
12. Detail glass panel
13. Tags
14. Description panel
15. Dependency rows
16. Scrollbars

## Later Surfaces

After the main addon manager is accepted:

- Discover production search/browse/detail data
- Settings dialog and settings tabs
- Addon packs
- SavedVariables manager
- Backup restore screens
- Upload/live logging workspace
- Dialogs, menus, tooltips, toasts, conflict panels

## Animation Inventory

The native port must reproduce the feel of these motion patterns:

- Header counters: counting number transition.
- Refresh/loading: spinner.
- Tabs: spring-like active indicator transition.
- Sidebar filter tabs and Discover sub-tabs now use shared animated native
  backplates instead of per-tab active repainting, matching the React
  `layoutId` indicator behavior more closely.
- Addon rows: hover background, selected rail, checkbox reveal.
- Buttons: hover brightness, active press offset/scale, gold shine.
- Dialog/popover: fade/slide/blur entrance.
- Live upload: pulse indicators and progress transitions.

## Acceptance Gate

A surface is ready only when:

- The current WebView screenshot and native screenshot are compared at the same size.
- Obvious visual mismatches are either fixed or written down as renderer limitations.
- Memory is measured in release mode.
- The component state checklist for that surface is complete.

## Current Native Prototype

The Slint prototype under `prototypes/slint-kalpa` is a working low-memory test bed,
not a completed design port. It should be judged only after each component family
has gone through the fidelity process above.

Current P0 native gaps:

- Detail body is scrollable and layout-backed, but it still needs production data
  models for banners, dependency actions, final file-editor backend integration,
  footer command wiring, Discover backend payload/actions, and the real addon
  store.
- Addon rows now read local manifests, but still need production data plumbing for
  ESOUI/update/disabled/persisted tag state, dynamic tag filters, context menus,
  full keyboard focus/auto-scroll behavior, custom context-menu styling,
  production context/batch command behavior, and virtualization/perf validation.
- Theme skins now have generated native SVG support and generated blurred orb
  sprites, but per-skin CSS gradient stack, remaining OKLCH token derivation, and
  backdrop-filter parity still need closer native equivalents.
- Source/screenshot disagreement around `Details / Files` tabs must be resolved
  against the running WebView before accepting the detail surface.
