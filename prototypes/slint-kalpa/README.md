# Kalpa Native UI Sidecar (Slint)

This crate is Kalpa's shipped native performance UI, not an experiment. `npm run
build:native-slint` compiles it and installs the binary as
`src-tauri/binaries/kalpa-slint-<triple>` (run as part of `npm run
build:release-assets`); users opt into it from Settings on Windows, and the
launcher relaunches Kalpa into this shell instead of the WebView2 one.

It operates on real user data. It `#[path]`-imports the production `esoui.rs`,
`metadata.rs`, and `saved_variables` modules from `src-tauri/src`, and it
installs and updates addons, rewrites `.kalpa-metadata`, edits SavedVariables,
creates and restores backups, and signs in to ESO Logs against the user's live
game folders. There is no mock-data mode: every change here ships to users'
machines and touches their real data, so it needs the same input validation,
atomic-write, and locking discipline as `src-tauri`.

The directory name and the render-preset material below are historical — this
started as a port workspace for evaluating whether Kalpa could keep its dark
glass UI while removing WebView2 from the desktop shell, and the measurement
notes from that evaluation are still the reference for renderer choices.

With no environment variables the sidecar selects the `standard` preset and
Slint's `winit-femtovg` backend, matching the production launcher's default.
`low-memory` is the explicit opt-in to the software renderer.

Render presets:

- `KALPA_RENDER_PRESET=standard` (default): uses `winit-femtovg`, keeps the
  translucent sidebar/panes, and is the track for richer glass/motion fidelity
  checks. Use `KALPA_SLINT_BACKEND=winit-skia` only on a Slint build that exposes
  that renderer.
- `KALPA_RENDER_PRESET=low-memory`: uses `winit-software`, opaque sidebar/panes,
  simplified native glass, smaller pre-blurred orb sprites, and zero-duration
  transitions.

Ambient backdrop motion is off by default in both presets; `KALPA_AMBIENT_MOTION=1`
is the only way to turn it on.

The preset and the backend are not independent. Slint 1.17's software renderer
implements no drop shadows, no clip-to-border-radius and no transform-rotation,
and the `standard` preset's translucent panes expose the full-window SVG backdrop
that it cannot draw correctly. `low-memory` is that renderer's mitigation bundle
rather than a lighter skin, so a software backend is always paired with the
`low-memory` preset even when `standard` was requested.

`KALPA_SLINT_BACKEND` or `SLINT_BACKEND` still override the backend directly for
manual renderer checks, subject to that pairing.

Current measured renderer memory on July 27, 2026 (private working set, release
build, median of three cold launches after a 100s settle):

- `winit-software` at the `low-memory` preset: 20.3 MB open, 6.1 MB minimized.
- `winit-femtovg` at the `standard` preset: 78.6 MB open, 76.2 MB minimized.

Those minimized figures predate #303, which trims the working set on minimize.
Re-measured on the sidecar binary after that change, `winit-femtovg` holds
82.0 MB open and **10.9 MB minimized**. Treat 76.2 as historical: the sentence
that used to sit here — that nothing releases the working set on this branch —
stopped being true when #303 landed.

The gap that remains is therefore almost entirely in the window-open state
(82.0 vs 20.3 MB), not the minimized one (10.9 vs 6.1 MB). That matters when
weighing the renderers against each other, because minimized is the state a
player is in while raiding and open is the state where the software renderer's
missing shadows, clips and rotations are actually visible.
- `winit-skia` was not re-measured in this pass. The July 1, 2026 figure was
  about 86 MB working set / 132 MB private and predates every change since.

Measure with `scripts/measure-memory.ps1` from the repository root; never set
`KALPA_DEMO_DATA` while measuring, as it inflates the sidecar.

Porting is tracked in `PORTING.md`; the project-wide fidelity contract is in
`../../docs/native-ui-port.md`.

Run it with:

```powershell
cargo run --release
```

from this directory.

With no environment variables, the sidecar uses the persisted active theme if
one has been saved, and otherwise the app's current `DEFAULT_THEME_ID`
(`nordic-runestone`). To compare against the older ESO Gold
reference screenshot, launch with `KALPA_THEME=eso-gold`.

Useful launch options:

```powershell
$env:KALPA_THEME = "daedric-crimson"
$env:KALPA_RENDER_PRESET = "low-memory"
$env:KALPA_REDUCED_MOTION = "1"
cargo run --release
```

`KALPA_REDUCED_MOTION=1` disables the ambient backdrop drift and loading spinner
timers. The sidecar should eventually source that token from the app
settings/accessibility layer instead.

Built-in theme ids are generated from the React catalog in
`src/lib/theme-presets.ts`:

```powershell
npm run export:native-themes
```

The generated native catalog lives in `assets/themes/builtin-themes.json` and is
compiled into the sidecar with `include_str!`, so it has to be re-exported and
committed whenever a theme is added or changed in `theme-presets.ts` — otherwise
the native picker silently ships an older theme list than the WebView UI. The
theme bridge is deliberately seed-based
so production can load the real built-in and custom theme data without
per-component color rewrites.

Built-in skin SVGs are generated from the React skin source in
`src/lib/theme-skins.ts`:

```powershell
npm run export:native-skins
```

To regenerate both native theme inputs together:

```powershell
npm run export:native-assets
```

The generated skin assets preserve the existing texture SVG, motif SVG,
`patternSize`, and `patternOpacity` for each art theme. Slint still approximates
the CSS gradient stack, backdrop blur, and browser color-mix behavior with native
gradients and precomputed tokens.

The sidecar can also ingest the same 12-color seed shape used by the current
custom theme editor:

```powershell
$env:KALPA_THEME_JSON = '{"colors":{"bgBase":"#030608","background":"#071218","surface":"#0c1f29","foreground":"#dcf0f6","mutedForeground":"#84b0bd","primary":"#3fc6dd","primaryForeground":"#02141b","accent":"#67e2f5","border":"#173744","orb1":"#1592ac","orb2":"#2a8fa6","orb3":"#0e3a52"}}'
cargo run --release
```

For larger exported themes, point `KALPA_THEME_FILE` at a JSON file containing
either that direct seed object or a full theme object with a `colors` field.
Full theme objects may also include `skinId`, which is mapped to the native skin
material layer when the id matches a built-in skin.
