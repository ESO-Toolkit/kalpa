# Changelog

All notable changes to Kalpa are documented here. This project uses [Conventional Commits](https://www.conventionalcommits.org/).

## [Unreleased]

_Nothing yet._

## [0.1.0-beta.20] — 2026-08-28

Two changes from beta feedback: you can now read an addon's changelog without
leaving Kalpa, and the addon list no longer comes back empty after a visit to
Discover.

### Features

- **My Addons now has an ESOUI tab, and changelogs are readable in-app.** The
  tab shows the same rich remote view Discover does — screenshots, the full
  description and download stats — and both panes now read through a cached
  lookup, so Discover also stops refetching on every selection. Changelogs come
  from ESOUI's own file data, reachable from a "What's new" button in the update
  panel and in each Update All row. The tab is hidden for side-loaded addons
  with no ESOUI ID, and only fetches once you open it.
  ([#397](https://github.com/ESO-Toolkit/kalpa/pull/397))
- **Changelogs render as a scannable version list rather than one flat dump.**
  A long changelog could be tens of thousands of characters with no hierarchy
  and no way to find a given version — on AwesomeGuildStore it was 87% of the
  panel's text. Versions are now hairline-separated rows with the latest
  expanded, behind a "Show all N versions" affordance, and each row carries its
  release date taken from ESOUI's archived-files table (no extra request). Your
  installed version is marked but never used to hide older entries. When an
  author's changelog has no reliable structure, the previous plain view is used
  instead. ([#397](https://github.com/ESO-Toolkit/kalpa/pull/397))

### Bug Fixes

- **The addon list no longer renders empty after returning from Discover.**
  Switching to Discover unmounts the installed list, but its virtualizer lived
  further up the tree and outlived the scroll container it measured — so coming
  back mounted a fresh container that nothing was ever told about, leaving a
  correctly-sized but blank list until something forced a re-render. The
  virtualizer now shares a lifecycle with the element it observes.
  ([#398](https://github.com/ESO-Toolkit/kalpa/pull/398))
- **Fixed the wrong changelog entry being marked as installed.** Version
  matching used containment, so an installed version that is a numeric prefix of
  a newer one — 1.7 inside 1.7.8 — marked the newest entry as installed and
  collapsed the update delta to nothing. Matching is now exact on the version
  token. Versions written as "v2.5.49" also now match their archived release
  date, and entries with no notes no longer render at half strength as though
  broken. ([#397](https://github.com/ESO-Toolkit/kalpa/pull/397))
- **The per-row changelog button in the update chooser is now disabled while
  offline**, matching the update actions beside it, instead of opening a dialog
  that could only show a network error.
  ([#397](https://github.com/ESO-Toolkit/kalpa/pull/397))

### Maintenance

- Pack Hub's index is now authoritative for pack lifecycles, with mutations
  journaled through the Durable Object to close races between concurrent
  create, update and delete requests.
  ([#398](https://github.com/ESO-Toolkit/kalpa/pull/398))

## [0.1.0-beta.19] — 2026-08-28

A focused fix for addons updated outside Kalpa.

### Bug Fixes

- **Kalpa now recognizes addons updated by Minion or another external manager.**
  Refreshing reconciles Kalpa's stored version when the selected live or PTS
  AddOns folder contains the current ESOUI release. Minion records are matched
  by addon ID, folder, and selected game root, preventing stale, duplicate, or
  cross-root data from rewriting unrelated addon metadata.
  ([#394](https://github.com/ESO-Toolkit/kalpa/pull/394))

## [0.1.0-beta.18] — 2026-08-23

A focused security and dependency release. There are no feature changes.

### Security

- **Updated the HTTP/2 stack to address RUSTSEC-2026-0258.** Kalpa now ships
  `h2` 0.4.16 instead of 0.4.14, fixing an issue where a peer could consume
  unbounded resources by sending empty DATA frames. The crate is used by both
  Kalpa's HTTP client and the built-in updater. ([#361](https://github.com/ESO-Toolkit/kalpa/pull/361))

### Maintenance

- Refreshed the frontend production and development dependencies, including
  Base UI, Motion, Vite, Playwright, ESLint, and testing libraries.
  ([#359](https://github.com/ESO-Toolkit/kalpa/pull/359),
  [#360](https://github.com/ESO-Toolkit/kalpa/pull/360))
- Updated the Pack Hub's Cloudflare development and test tooling, the Rust
  `base64` crate, and the GitHub Actions Rust cache revision.
  ([#339](https://github.com/ESO-Toolkit/kalpa/pull/339),
  [#355](https://github.com/ESO-Toolkit/kalpa/pull/355),
  [#353](https://github.com/ESO-Toolkit/kalpa/pull/353))

## [0.1.0-beta.17] — 2026-08-14

A new dependency setting, and two fixes for cases where Kalpa could act on
something other than what it had told you.

### Features

- **You can now be asked about required libraries only.** An addon's manifest
  separates the libraries it needs from the ones it merely supports, but the
  prompt listed both — so anyone who only wanted the required ones had to untick
  the optional rows on every install. Settings → "When an addon needs other
  libraries" now has an "Only ask about required ones" option alongside "Ask me
  which ones to install". Optional libraries stay listed under each addon's
  Details tab, with an Install button, so nothing becomes harder to find.
  ([#357](https://github.com/ESO-Toolkit/kalpa/pull/357))
- **"Install them automatically" is now labelled "Install required ones
  automatically."** That is what it has always done — optional libraries are
  never installed without you ticking them — but the old wording read as "all of
  them" and steered people away from the setting they actually wanted.
  ([#357](https://github.com/ESO-Toolkit/kalpa/pull/357))

### Bug Fixes

- **The dependency setting can no longer show one thing while Kalpa does
  another.** The radio applied your click immediately and saved in the
  background without checking whether the save worked. If it failed — or was
  still queued when an install started — Settings could say "ask me" while the
  stored setting was still "never install them", and a missing required library
  was silently never offered. The setting now changes only once the save is
  confirmed, and tells you if it could not be saved.
  ([#357](https://github.com/ESO-Toolkit/kalpa/pull/357))
- **A deleted account can no longer be restored by a backup that was already in
  flight.** The nightly backup read the pack index and wrote it minutes later,
  sharing no ordering with account deletion, so a backup that started before a
  deletion could republish the deleted records into the one backup key that
  never expires. Deletions now leave a marker that the backup honours whichever
  way the two interleave. ([#352](https://github.com/ESO-Toolkit/kalpa/pull/352))
- **SavedVariables are no longer written while ESO is running.** Importing a
  pack's settings and restoring a snapshot both relied on a preference that only
  ever governed the addon reminder, so anyone who had dismissed that reminder
  got no warning at all — and the game, which rewrites SavedVariables from
  memory at every loading screen, discarded the result. Both now decline until
  you close ESO. Addon installs and updates are unaffected: writing addon files
  under a running client really is safe.
  ([#352](https://github.com/ESO-Toolkit/kalpa/pull/352))

## [0.1.0-beta.16] — 2026-08-08

Three follow-ups to the audit remediation in beta.15. Most of the work is
internal, but reviewing it turned up several ways settings and addons could go
missing quietly — including one that reported success while doing the opposite.

### Bug Fixes

- **Importing pack settings in the performance UI no longer overwrites your real
  settings with something the game cannot read.** When a pack carried a
  megaserver you do not play on, the performance UI wrote the file anyway — with
  the unmapped layer left as a placeholder key ESO never looks at — and reported
  the addon as applied. Your previous settings for that addon were replaced. The
  main window refused the same import correctly; now both refuse it, and say
  which addon could not be mapped and why.
  ([#344](https://github.com/ESO-Toolkit/kalpa/pull/344))
- **Pack settings are no longer applied while ESO is running.** The game holds
  SavedVariables in memory and rewrites them when you log out, so an import
  applied underneath a running client was discarded — after Kalpa had told you it
  worked. Kalpa now asks you to close ESO first, and re-checks immediately before
  writing rather than once when you clicked.
  ([#344](https://github.com/ESO-Toolkit/kalpa/pull/344))
- **Removing an addon and then refreshing no longer brings it back as a row for
  something that is gone.** A removal hides the row immediately and deletes the
  folder three seconds later, so a refresh in that window read the addon straight
  back off disk and restored it — and once the delete landed, nothing hid it
  again. Refreshes now mask anything queued for removal, and keep masking it
  across the delete itself. ([#342](https://github.com/ESO-Toolkit/kalpa/pull/342))
- **Importing pack settings no longer silently discards a megaserver's worth of
  them.** Every world layer in an exported pack was templated to the same
  placeholder, so a file holding both an NA and an EU layer emitted two identical
  keys on import and Lua's last-one-wins quietly dropped one. Layers now map
  one-to-one. ([#344](https://github.com/ESO-Toolkit/kalpa/pull/344))
- **Update All no longer acts on a list that went stale while it waited on you.**
  The "ESO is running" prompt has no time limit and the addon list stays live
  behind it, so an addon removed while that prompt was open could still be
  downloaded and extracted into a folder seconds from deletion — and switching
  your AddOns folder mid-prompt could start the batch against the instance you
  had just left. Both are re-checked at the last moment, and a folder switch
  stops the run rather than guessing.
  ([#342](https://github.com/ESO-Toolkit/kalpa/pull/342))
- **A SavedVariables edit saved while ESO is running now always warns you** that
  the game will overwrite it at logout. The warning was tied to the addon
  reminder setting, so anyone who had dismissed that never saw it.
  ([#344](https://github.com/ESO-Toolkit/kalpa/pull/344))

### Internal

- Addon removal, the Update All re-entry guard and the rescan's selection
  handling moved out of `App.tsx` into tested modules.
  ([#342](https://github.com/ESO-Toolkit/kalpa/pull/342))
- Destructive end-to-end tests, which previously could not exist: the suite ran
  against the developer's real ESO install. A debug-only AddOns override plus a
  runner that owns the app makes install, remove and restore testable — and the
  backend now refuses to register any other folder while that override is set, so
  a regression fails the boot instead of a later assertion.
  ([#343](https://github.com/ESO-Toolkit/kalpa/pull/343))
- Pack Hub restore is resumable. It walked the whole snapshot in one request and
  hit Cloudflare's subrequest ceiling, so a large corpus could not be restored at
  all. ([#344](https://github.com/ESO-Toolkit/kalpa/pull/344))
- Gates that were not running: the worker's tests had never been typechecked,
  `clippy` never linted either crate's test target, `prettier` never saw
  `public/`, `scripts/` or `e2e/`, and the guard against source files containing
  raw control bytes — the defect class that hid a whole module from review in
  beta.15 — only ever looked at the frontend. All now run across the repository.
  ([#344](https://github.com/ESO-Toolkit/kalpa/pull/344),
  [#342](https://github.com/ESO-Toolkit/kalpa/pull/342))

## [0.1.0-beta.15] — 2026-07-31

Splitting a log now finishes the job. The output used to land outside the folder every upload path reads from, so the one reason to split — uploading part of a session — was unreachable without dragging the file back in by hand.

### Features

- **Split output lands in your ESO Logs folder, appears in Kalpa's own log list, and uploads straight from the picker.** Nothing is auto-deleted either; each new split used to silently remove an older one. ([#335](https://github.com/ESO-Toolkit/kalpa/pull/335), [#336](https://github.com/ESO-Toolkit/kalpa/pull/336))
- **The picker is rebuilt around one rule: every session you tick fights in becomes one report.** The modal is gone — sessions and their fights are a single list, in place, with no mode toggle asking the same question twice. It opens on your latest raid night (the most recent session plus anything that ran within a few hours of it, so a mid-raid crash stays together), and "the boss kills, without the nine resets" is finally something you can express. Nine naming controls were removed; names were always derived anyway. ([#336](https://github.com/ESO-Toolkit/kalpa/pull/336))
- **Fights show difficulty and kills**, read from data the scanner was already parsing and throwing away. A kill is only shown when it can be proven — an absent badge means unknown, never "failed", because a wiped run writes nothing to say so. ([#335](https://github.com/ESO-Toolkit/kalpa/pull/335))

### Bug Fixes

- **Flat addon archives install where ESO can load them.** An addon zipped without its containing folder extracted loose into the AddOns root, so the game never saw it. Those archives are now wrapped in the folder the manifest names. ([#335](https://github.com/ESO-Toolkit/kalpa/pull/335))
- **A UTF-8 BOM in `settings.json` no longer resets every preference.** The file parsed as invalid and silently fell back to defaults. ([#336](https://github.com/ESO-Toolkit/kalpa/pull/336))
- **An expired ESO Logs upload session says so.** It reported itself as "Off", sending you to look for a switch you never touched. ([#336](https://github.com/ESO-Toolkit/kalpa/pull/336))
- **The text size setting applies before the window appears.** At 110%, 125% or 150% the interface painted once at 100% and then jumped, which is a poor first frame for the users the control was built for. ([#334](https://github.com/ESO-Toolkit/kalpa/pull/334))

## [0.1.0-beta.14] — 2026-07-28

A player with low vision reported that Kalpa's text was too faint and too small ([#199](https://github.com/ESO-Toolkit/kalpa/issues/199)). The faintness turned out to be a bug affecting every theme, and the rest of this release follows from investigating it.

### Bug Fixes

- **Text now keeps the contrast its theme promises.** Every theme defined muted text that met the WCAG AA 4.5:1 floor, and no theme delivered it: components faded that text with an opacity multiplier _after_ the theme had chosen the colour, so the value the contrast checker validated was never the value on screen. Measured across all 48 themes, 48 passed as defined and **0 passed as rendered** — the worst case painted at 1.9:1 in a theme reporting 5.1:1. That is why trying a different theme did not help: the fade sat on top of whichever one was active. 240 sites across 50 files now paint at full strength, hierarchy is carried by which token is used rather than by opacity, and a test fails the build if readable text starts fading again. ([#327](https://github.com/ESO-Toolkit/kalpa/pull/327))
- **A fatal error is now readable on a light theme.** The crash overlay printed its heading in hardcoded white, so an error on a light background showed an invisible message — the one screen that most needs to be legible. It renders before the theme loads and no single colour clears 4.5:1 against both white and near-black, so it now uses a fixed mid-slate that is readable either way. Found by the same sweep that caught a fifth class of dark-only hardcoding: raw brand hex written straight into text, bypassing the darker `primary` and `accent` the light themes deliberately define. Three of the four earlier classes had been found only after someone opened a screen and saw it broken. ([#331](https://github.com/ESO-Toolkit/kalpa/pull/331))

### Features

- **A text size control**, in Settings → Appearance at 100%, 110%, 125% or 150%, and on `Ctrl`/`⌘` with `+`, `-` and `0`. It scales the whole interface rather than body text alone, which matters here: 132 of Kalpa's 140 fixed-pixel type sizes are 11px or smaller, so a font-size slider would have enlarged the text that was already legible and left the smallest labels untouched — widening the gap it was meant to close. At 150% the minimum window grows to 1200 × 750 so the layout keeps the space it needs; that still fits a 1366 × 768 laptop.
- **Light and high-contrast themes.** A new Accessibility category, placed ahead of the decorative sets so it is not forty themes down, with two high-contrast dark themes clearing 12:1 on every pair the contrast checker measures, plus Paper White, Soft Grey and Warm Parchment. Light themes work because borders, dividers, panel fills, overlays and status colours now follow the active theme's lightness instead of assuming a dark background — 944 hardcoded values in all. Dark themes are unchanged: each token resolves to exactly the value it replaced. ([#329](https://github.com/ESO-Toolkit/kalpa/pull/329))
- **Every keyboard shortcut is now listed in the app**, on `?` or from Settings → Appearance. Kalpa had twelve and showed three, two of which only appeared in an empty state that renders when no addons are installed.
- **A way to report problems without leaving Kalpa.** Settings → Tools and the Accessibility theme section both link to the issue tracker and to Discord. There was previously no in-app route at all: the person who filed [#199](https://github.com/ESO-Toolkit/kalpa/issues/199) had to go and find the repository unaided. Accessibility reports also have their own issue template now — #199 arrived as a feature request and read as a nice-to-have for a month, while being a bug affecting all 48 themes.

### Internal

- **A verification gate that runs the packaged build.** CI produced the production bundle and never executed it, so every lazily-loaded dialog shipped unexercised. That is not hypothetical: the text size control passed type-checking, linting, formatting, `cargo check`, clippy and 400 tests while being completely inert, because zoom was applied before the WebView2 dispatcher existed. `npm run test:packaged` builds the app, owns the launch — necessary, because the single-instance plugin silently focuses an existing window, so a test that merely attaches can pass against a dev server — and proves every emitted chunk loads from the bundled origin.

## [0.1.0-beta.13] — 2026-07-28

Dependency installation becomes something you choose rather than something that happens to you, the native performance UI stops holding memory it isn't using, and a documentation audit replaced the claims that could not be substantiated.

### Features

- **You choose which libraries get installed.** Kalpa used to pull in every dependency an addon declared, transitively, with no say in the matter. It now asks: required libraries come pre-ticked, optional ones (`OptionalDependsOn`, which Kalpa previously ignored entirely) are listed unticked, and you take all, some, or none. Settings has an "install automatically / ask me / never" control, and a way to clear libraries you told it to stop offering. Skipping a required library warns you but never blocks. Update All asks once for the whole run rather than once per addon. ([#305](https://github.com/ESO-Toolkit/kalpa/pull/305))
- **Every release now ships SHA-256 checksums.** A `SHA256SUMS.txt` covering every file is generated and attached before the release goes live, so the checksum verification [the download docs](docs/verify-download.md) describe is now always available rather than occasionally. ([#301](https://github.com/ESO-Toolkit/kalpa/pull/301))

### Bug Fixes

- **The native performance UI now releases its memory when minimized.** It previously held its whole footprint while minimized — the state you leave it in while playing. Measured on a release build: 76.2 MB → 10.9 MB minimized, which now also beats the standard WebView UI's 18 MB. ([#303](https://github.com/ESO-Toolkit/kalpa/pull/303))
- **The native UI can no longer start in a broken rendering mode.** Slint's software renderer draws no shadows, ignores rounded-corner clipping and cannot rotate, so pairing it with the full-fidelity preset produced a visibly wrong window. That combination is now impossible to select, and three latent environment-variable bugs were fixed alongside it — including one where a set-but-empty backend variable stopped the native UI starting at all. ([#304](https://github.com/ESO-Toolkit/kalpa/pull/304))
- **`npm run check:env` now checks for the Node version the project actually needs.** It only failed below Node 18 while everything else requires 22, so it passed on versions Kalpa does not build against. ([#302](https://github.com/ESO-Toolkit/kalpa/pull/302))

### Documentation

- **The privacy policy now describes a data flow it had omitted.** Published Pack Hub packs are copied into the same database that powers esotk.com; the policy said only the Pack Hub. It now states exactly what crosses, including that an anonymous pack's display name is replaced but its numeric author ID is not. The "delete my data" claim was also overstated — deletion never touched the backup snapshots, so the non-expiring one is now scrubbed on deletion and the remaining 90-day window is stated plainly. Storage paths are documented per platform and the Windows path was corrected (`%APPDATA%`, not `%LOCALAPPDATA%`), which means the "delete your local data" instructions previously pointed at the wrong folder. ([#299](https://github.com/ESO-Toolkit/kalpa/pull/299))
- **Security reports now go somewhere that works.** `SECURITY.md` pointed at GitHub private vulnerability reporting, which is not enabled on this repository, with no fallback. It now points at Discord, drops a 48-hour response promise no single maintainer can keep, and cites the actual review write-ups instead of implying a third-party audit. ([#300](https://github.com/ESO-Toolkit/kalpa/pull/300))
- **Both package manifests use a valid SPDX license identifier** (`BUSL-1.1`; `BSL-1.1` is not registered). ([#302](https://github.com/ESO-Toolkit/kalpa/pull/302), [#319](https://github.com/ESO-Toolkit/kalpa/pull/319))
- **Kalpa is described as source-available, not open source.** BSL 1.1 is not an OSI-approved licence, and the README, privacy policy, both package manifests, project docs and this changelog's own alpha.1 entry all said otherwise. ([#294](https://github.com/ESO-Toolkit/kalpa/pull/294), [#297](https://github.com/ESO-Toolkit/kalpa/pull/297), [#319](https://github.com/ESO-Toolkit/kalpa/pull/319))
- **The README's security section no longer claims protections that do not exist.** There are two path validators, not one, and ZIP extraction has no recursion cap — zip bombs are stopped by a 500 MB cap and traversal by per-component validation. The installer size (the Linux AppImage is 84 MB, not 18 MB), the claim that every installer ships a `.sig` (the `.dmg` does not) and the dependency-audit cadence were corrected at the same time. ([#298](https://github.com/ESO-Toolkit/kalpa/pull/298))
- **The performance numbers are measured rather than asserted.** The idle CPU and memory figures were inherited from an older release and did not reproduce; the downloads badge was counting auto-updater polls (838 of 895 "downloads" were `latest.json` fetches) and is gone. The replacements use private working set — Task Manager's Memory column — because summing each process's total working set double-counts pages shared across the seven WebView2 processes and inflated the comparison roughly threefold. ([#295](https://github.com/ESO-Toolkit/kalpa/pull/295), [#296](https://github.com/ESO-Toolkit/kalpa/pull/296))
- **All 14 README screenshots were recaptured from the running app** (33 MB of PNGs down to 626 KB of WebP), and the ESO Logs uploader, custom themes and native UI — none of which the README mentioned — are now documented. ([#294](https://github.com/ESO-Toolkit/kalpa/pull/294))

### Under the Hood

- Dependabot updates are grouped per ecosystem, so a week's bumps arrive as one pull request per lane instead of a queue of individually-opened ones that all edit the same lockfile and conflict with each other. It also keeps version-locked pairs such as `react` and `react-dom` moving together. ([#315](https://github.com/ESO-Toolkit/kalpa/pull/315))

## [0.1.0-beta.12] — 2026-07-16

Adds an experimental **native performance UI** and polishes the cross-platform release from beta.11.

### Features

- **Native performance UI (beta, Windows).** A new opt-in mode in Settings relaunches Kalpa as a lightweight native app that uses noticeably less memory than the standard WebView UI. It has full addon management, the ESO Logs uploader, and Pack Hub, and you can switch back to the standard UI anytime from the native app's Settings. It's clearly marked **Beta** and is Windows-only for now — if the native UI ever fails to start, Kalpa automatically falls back to the standard UI. ([#274](https://github.com/ESO-Toolkit/kalpa/pull/274))

### Bug Fixes

- **Fixed blurry dialogs in the native UI on high-DPI displays.** The upload, appearance, characters, safety, and migration dialogs rendered soft/upscaled on scaled displays; they're now crisp. ([#276](https://github.com/ESO-Toolkit/kalpa/pull/276))
- **The native UI can't get stuck without a window.** If the native performance mode is enabled but its app is missing or won't start, Kalpa now reliably reverts to the standard UI and tells you why, instead of retrying a broken launch. ([#275](https://github.com/ESO-Toolkit/kalpa/pull/275))
- **macOS release builds are no longer blocked by signing.** Unsigned macOS beta builds now publish correctly; code signing activates automatically once an Apple Developer certificate is configured. ([#272](https://github.com/ESO-Toolkit/kalpa/pull/272))

## [0.1.0-beta.11] — 2026-07-14

Kalpa goes cross-platform: this release ships native **macOS** and **Linux** builds (beta) alongside Windows, plus a real progress bar for manual log uploads.

### Features

- **macOS and Linux support (beta).** Kalpa now builds, packages, and auto-updates on all three desktop platforms. macOS ships as a universal `.dmg` (Intel & Apple Silicon, macOS 10.15+) with native traffic-light window controls, ⌘-based shortcuts, Keychain-backed login persistence, and detection of both the native Mac client (`~/Documents/Elder Scrolls Online`) and CrossOver bottles. Linux ships as `.AppImage` (self-updating), `.deb`, and `.rpm`, stores your login in the Secret Service keyring, and automatically finds ESO under Steam Proton — including Flatpak/Snap Steam and secondary Steam libraries. Windows behavior is unchanged. ([#239](https://github.com/ESO-Toolkit/kalpa/pull/239))
- **Manual uploads now show real progress and a time estimate.** Uploading a log to ESO Logs displays a determinate progress bar driven by the actual backend lifecycle — Prepare → Upload → Finalize → Done — with a percentage, a phase stepper, and a live "about Xs left" estimate. Shown only for direct uploads (work Kalpa can actually observe); the official-uploader handoff keeps its existing behavior. ([#215](https://github.com/ESO-Toolkit/kalpa/pull/215))

### Bug Fixes

- **The Addon Profiles dialog is reachable again.** beta.10 shipped the profiles overhaul with no way to open it — the button was lost in a refactor. Profiles now live behind a dedicated header button (the layers icon, next to Saved Vars). ([#271](https://github.com/ESO-Toolkit/kalpa/pull/271))
- **"Show in folder" now works with any path separator.** Two spots joined paths with a hard-coded backslash, which produced invalid paths on macOS and Linux; they now use the platform-correct separator everywhere. ([#239](https://github.com/ESO-Toolkit/kalpa/pull/239))

### Under the Hood

- Releases are now built per-platform in a serialized pipeline and published only after an automated check confirms the auto-updater manifest covers Windows, macOS, and Linux — a release can never go live with a platform missing. ([#239](https://github.com/ESO-Toolkit/kalpa/pull/239))

## [0.1.0-beta.10] — 2026-07-13

The biggest release since launch, headlined by a full **ESO Logs uploader** built into Kalpa, a **profiles & multi-instance overhaul**, major **SavedVariables editor** upgrades, and deep **performance work** that cuts idle CPU and memory dramatically.

### Features

- **Upload combat logs to ESO Logs without leaving Kalpa.** The new uploader workspace (Logs button in the header) uploads past sessions or individual fights, live-streams a running session so your report fills in as you play, and can split a multi-session `Encounter.log` into separate files. Sign in with your ESO Logs account to upload directly from Kalpa, or hand off to the official uploader if you prefer — either way you choose report visibility (Unlisted/Public/Private) before anything is sent, and an upload history keeps links to every report. ([#157](https://github.com/ESO-Toolkit/kalpa/pull/157), [#202](https://github.com/ESO-Toolkit/kalpa/pull/202), [#256](https://github.com/ESO-Toolkit/kalpa/pull/256))
- **Huge log files no longer make the uploader crawl.** For multi-GB `Encounter.log` files, "Latest fights" and "Latest session" now anchor to the newest session instead of scanning the whole file — on a 3.7 GB archive that's ~0.1s instead of ~9s — and the full scan is deferred behind an explicit action for logs over 256 MiB. ([#256](https://github.com/ESO-Toolkit/kalpa/pull/256))
- **Richer player cards on the ESO Log Aggregator.** After a direct upload, Kalpa forwards a small "build evidence" sidecar that the raw log contains but ESO Logs doesn't keep: exact scribing scripts (deterministic, not inferred), Mundus stone, champion-point stars and passives, and — if you run the ESOTK Companion addon — its live-client snapshots. What's sent and how to remove it is documented in PRIVACY.md. ([#256](https://github.com/ESO-Toolkit/kalpa/pull/256))
- **See exactly what a profile switch will do before it happens.** Activating a profile now shows a preview first: which addons will be enabled, which will be disabled, which required libraries outside the profile stay on, and anything that can't be changed — so an older snapshot can never silently turn off addons you installed since. If nothing needs to change, the profile activates directly with no extra step. ([#251](https://github.com/ESO-Toolkit/kalpa/pull/251))
- **The app header now shows which ESO install you're managing.** A badge next to the logo displays the active instance (e.g. "Native · NA"), and when you have more than one install — live, EU, or PTS — it opens a quick-switch menu so you always know (and can change) where installs and updates are going. Switching instances in Settings now also applies in one click instead of requiring a separate Save, and newly installed instances are picked up automatically whenever Settings opens. ([#251](https://github.com/ESO-Toolkit/kalpa/pull/251))
- **Copy your addon setup to another instance.** Next to each other instance in Settings there's now a copy action that installs all of your enabled addons into that instance (e.g. set up PTS from your live loadout), including their update metadata and tags. Addons the target already has — enabled or disabled — are never touched. ([#251](https://github.com/ESO-Toolkit/kalpa/pull/251))
- **Profiles can be updated, renamed, and inspected.** Each profile row now has an update action that overwrites the snapshot with your current setup (no more delete-and-recreate), an inline rename, and an expandable list of the addons it contains. The active profile is marked "modified" when your current setup has drifted from its snapshot. ([#251](https://github.com/ESO-Toolkit/kalpa/pull/251))
- **Profiles survive an AddOns folder wipe.** Kalpa now keeps a mirror copy of each instance's profiles in its app data folder and restores from it automatically if the AddOns folder is deleted or reset (a common troubleshooting step, and routine on PTS). ([#251](https://github.com/ESO-Toolkit/kalpa/pull/251))
- **The SavedVariables editor now knows what your addon settings mean.** Kalpa scans installed addons' LibAddonMenu source and turns raw settings values into proper dropdowns with the addon's own labels — no more guessing that `2` means "Large". Nothing is executed: the scan is purely textual and capped for safety. ([#227](https://github.com/ESO-Toolkit/kalpa/pull/227))
- **Search every setting in the SavedVariables editor.** A search box finds settings anywhere in an addon's tree (with jump-to-group), and large dropdowns get type-to-filter. ([#229](https://github.com/ESO-Toolkit/kalpa/pull/229))
- **Sort addons by Recently Updated or Recently Installed.** Two new sort options put your latest changes on top, and the installed date now tracks the last download rather than freezing at first install. ([#201](https://github.com/ESO-Toolkit/kalpa/pull/201))
- **Pick exactly which addons to update.** The update banner now has a chooser to update a subset instead of all-or-nothing. ([#194](https://github.com/ESO-Toolkit/kalpa/pull/194))
- **Pack Hub cards got a visual identity.** Each pack shows a distinct monogram so collections are recognizable at a glance. ([#197](https://github.com/ESO-Toolkit/kalpa/pull/197))
- **Nordic Runestone is the default theme for new installs.** ([#193](https://github.com/ESO-Toolkit/kalpa/pull/193))

### Bug Fixes

- **Activating a profile no longer disables libraries its addons need.** A profile is a snapshot of your enabled addons at creation time, but addon updates can pull in new required libraries afterward — and activating an older profile would disable those libraries, leaving the profile's own addons erroring at the login screen. Activation now keeps required dependencies enabled (re-enabling them if needed), including libraries required indirectly through other libraries, matched case-insensitively the way ESO resolves them. A toast tells you which libraries were kept. ([#251](https://github.com/ESO-Toolkit/kalpa/pull/251))
- **Profile activation handles leftover duplicate addon folders gracefully.** If both `Foo` and `Foo.disabled` exist (e.g. left behind by another tool), activating a profile no longer shows a raw rename error: enabling such an addon is recognized as already done (the enabled copy is what the game loads), and disabling one now explains that the stale copy must be removed first. ([#251](https://github.com/ESO-Toolkit/kalpa/pull/251))
- **Profile activation now warns when ESO is running,** the same as installing, updating, or removing addons, since the game won't see the change until a relog or /reloadui. ([#251](https://github.com/ESO-Toolkit/kalpa/pull/251))
- **Anonymous packs are now anonymous at the API level.** The Pack Hub worker redacts the author's name and id from anonymous packs in every public response and excludes them from author searches — previously the real identity was visible to anyone querying the API directly. ([#254](https://github.com/ESO-Toolkit/kalpa/pull/254))
- **Backups, restore & snapshots hardened.** Character backups are now scoped to the right server (same-named characters on different servers no longer share a backup), writes are crash-safe, and a batch of edge cases found by a full audit were fixed. ([#230](https://github.com/ESO-Toolkit/kalpa/pull/230))
- **Settings can't be lost to a crash mid-save.** `settings.json` persistence is now atomic (temp file + rename), so a crash or power loss during save can't truncate your settings. ([#198](https://github.com/ESO-Toolkit/kalpa/pull/198))
- **SavedVariables copy-profile no longer risks data loss** on odd file formats, and a batch of editor audit findings were fixed. ([#224](https://github.com/ESO-Toolkit/kalpa/pull/224), [#223](https://github.com/ESO-Toolkit/kalpa/pull/223))
- **Large-addon updates are fast and stoppable.** Updating a big addon no longer hangs for minutes on single-threaded hashing; updates hash in parallel, show progress, and can be stopped. ([#183](https://github.com/ESO-Toolkit/kalpa/pull/183), [#184](https://github.com/ESO-Toolkit/kalpa/pull/184))

### Performance

- **Idle CPU cut from ~89% of a core to ~4%, memory from ~530 MB to ~142 MB** when the app sits focused but idle: ambient animations pause and resample instead of running the compositor at full rate, the webview deep-suspends when hidden, and V8 code caching speeds startup. ([#217](https://github.com/ESO-Toolkit/kalpa/pull/217), [#236](https://github.com/ESO-Toolkit/kalpa/pull/236))
- **Lower peak memory and CPU across the backend, uploader, and SavedVariables parsing.** ([#226](https://github.com/ESO-Toolkit/kalpa/pull/226))

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

First public alpha release of **Kalpa** — a source-available desktop addon manager for Elder Scrolls Online.

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

<!--
Version headings are link references; a heading without a definition below
renders as literal bracketed text, which is how beta.5 through beta.12 read
until this release.

beta.3 was never tagged (the repo goes v0.1.0-beta.2 → v0.1.0-beta.4), so its
changes are only reachable inside the beta.4 range and both headings resolve
to it.
-->

[Unreleased]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.20...HEAD
[0.1.0-beta.20]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.19...v0.1.0-beta.20
[0.1.0-beta.19]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.18...v0.1.0-beta.19
[0.1.0-beta.18]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.17...v0.1.0-beta.18
[0.1.0-beta.17]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.16...v0.1.0-beta.17
[0.1.0-beta.16]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.15...v0.1.0-beta.16
[0.1.0-beta.15]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.14...v0.1.0-beta.15
[0.1.0-beta.14]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.13...v0.1.0-beta.14
[0.1.0-beta.13]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.12...v0.1.0-beta.13
[0.1.0-beta.12]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.11...v0.1.0-beta.12
[0.1.0-beta.11]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.10...v0.1.0-beta.11
[0.1.0-beta.10]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.9...v0.1.0-beta.10
[0.1.0-beta.9]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.8...v0.1.0-beta.9
[0.1.0-beta.8]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.7...v0.1.0-beta.8
[0.1.0-beta.7]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.6...v0.1.0-beta.7
[0.1.0-beta.6]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.5...v0.1.0-beta.6
[0.1.0-beta.5]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.4...v0.1.0-beta.5
[0.1.0-beta.4]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.2...v0.1.0-beta.4
[0.1.0-beta.3]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.2...v0.1.0-beta.4
[0.1.0-beta.2]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-beta.1...v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-alpha.8...v0.1.0-beta.1
[0.1.0-alpha.8]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-alpha.3...v0.1.0-alpha.8
[0.1.0-alpha.3]: https://github.com/ESO-Toolkit/kalpa/compare/v0.1.0-alpha.2...v0.1.0-alpha.3
[0.1.0-alpha.1]: https://github.com/ESO-Toolkit/kalpa/releases/tag/v0.1.0-alpha.1
