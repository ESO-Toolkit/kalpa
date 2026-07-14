# Kalpa Slint Prototype

This is a contained native UI port workspace for evaluating whether Kalpa can
keep its current dark glass UI while removing WebView2 from the desktop shell.

It is intentionally mock-data only. The goal is to port the current UI
component by component, compare against WebView screenshots, and measure native
UI memory before porting application behavior.

The prototype selects the `low-memory` render preset by default, which maps to
Slint's `winit-software` backend. On this machine that is materially smaller
than the GPU-backed renderers and is a better match for Kalpa's mostly static,
dense desktop UI while visual parity is still being built.

Render presets:

- `KALPA_RENDER_PRESET=low-memory`: uses `winit-software`, simplified native
  glass, pre-blurred/static assets, and ambient motion disabled unless explicitly
  enabled.
- `KALPA_RENDER_PRESET=standard`: uses `winit-femtovg` by default for this
  prototype, enables ambient motion by default, and is the track for richer
  glass/motion fidelity checks. Use `KALPA_SLINT_BACKEND=winit-skia` only on a
  Slint build that exposes that renderer.

`KALPA_SLINT_BACKEND` or `SLINT_BACKEND` still override the backend directly for
manual renderer checks.

Current measured renderer memory on June 30, 2026:

- `winit-software`: about 36-38 MB working set, 18-20 MB private memory after the
  animated backdrop, generated theme catalog, generated skin assets, tags, and
  row-state pass. Latest local run: 36.66 MB working set / 19.18 MB private.
- `winit-femtovg`: about 84 MB working set, 132 MB private memory.
- `winit-skia`: about 86 MB working set, 132 MB private memory.

Porting is tracked in `PORTING.md`; the project-wide fidelity contract is in
`../../docs/native-ui-port.md`.

Run it with:

```powershell
cargo run --release
```

from this directory.

With no environment variables, the prototype uses the app's current
`DEFAULT_THEME_ID` (`nordic-runestone`). To compare against the older ESO Gold
reference screenshot, launch with `KALPA_THEME=eso-gold`.

Useful launch options:

```powershell
$env:KALPA_THEME = "daedric-crimson"
$env:KALPA_RENDER_PRESET = "low-memory"
$env:KALPA_REDUCED_MOTION = "1"
cargo run --release
```

`KALPA_REDUCED_MOTION=1` disables the ambient backdrop drift and loading spinner
timers in the prototype. Production should eventually source that token from the
app settings/accessibility layer.

Built-in prototype theme ids are generated from the React catalog in
`src/lib/theme-presets.ts`:

```powershell
npm run export:native-themes
```

The generated native catalog currently includes 49 built-in themes in
`assets/themes/builtin-themes.json`. The theme bridge is deliberately seed-based
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

The prototype can also ingest the same 12-color seed shape used by the current
custom theme editor:

```powershell
$env:KALPA_THEME_JSON = '{"colors":{"bgBase":"#030608","background":"#071218","surface":"#0c1f29","foreground":"#dcf0f6","mutedForeground":"#84b0bd","primary":"#3fc6dd","primaryForeground":"#02141b","accent":"#67e2f5","border":"#173744","orb1":"#1592ac","orb2":"#2a8fa6","orb3":"#0e3a52"}}'
cargo run --release
```

For larger exported themes, point `KALPA_THEME_FILE` at a JSON file containing
either that direct seed object or a full theme object with a `colors` field.
Full theme objects may also include `skinId`, which is mapped to the native skin
material layer when the id matches a built-in skin.
