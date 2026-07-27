# Kalpa

[![CI](https://github.com/ESO-Toolkit/kalpa/actions/workflows/ci.yml/badge.svg)](https://github.com/ESO-Toolkit/kalpa/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/ESO-Toolkit/kalpa?color=c4a44a&label=release)](https://github.com/ESO-Toolkit/kalpa/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/ESO-Toolkit/kalpa/total?color=c4a44a&label=downloads)](https://github.com/ESO-Toolkit/kalpa/releases)
[![License: BSL 1.1](https://img.shields.io/badge/License-BSL%201.1-blue.svg)](LICENSE)

A fast, open-source addon manager for **The Elder Scrolls Online**. Built with Tauri, React, and Rust — designed as a modern alternative to Minion with community features, better dependency handling, and a native desktop experience.

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#features">Features</a> ·
  <a href="#screenshots">Screenshots</a> ·
  <a href="#security--privacy">Security</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <img src=".screenshots/main-desktop.webp" alt="Kalpa's main window: the addon list on the left with smart filters, and the selected addon's details, tags, and dependencies on the right" width="800" />
</p>

> [!NOTE]
> **Beta release.** Kalpa is stable and feature-complete for daily use. Grab the latest build from the [Releases](https://github.com/ESO-Toolkit/kalpa/releases/latest) page — it auto-updates from there. Bug reports and feedback are welcome via [GitHub Issues](https://github.com/ESO-Toolkit/kalpa/issues).

> [!TIP]
> **Recently shipped:** a built-in [ESO Logs uploader](#eso-logs-uploader), [49 themes](#themes-and-appearance) with a custom theme builder, an opt-in [native performance UI](#additional-features) on Windows, and macOS and Linux support. Full history in the [changelog](CHANGELOG.md).

---

## Why Kalpa?

Minion has served the ESO community well, but it hasn't kept pace with modern expectations. Kalpa is built from scratch to be **fast, lightweight, and community-driven**:

- **Native performance** — a Rust backend in a compact installer (~18 MB) instead of a bundled Java runtime
- **Light on your system** — sitting idle, Kalpa uses roughly 4% of one CPU core and ~140 MB of memory, so it can stay open while you raid
- **Automatic dependency resolution** — installs missing libraries without manual hunting, including transitive deps and version validation
- **Update conflict resolution** — see file-level diffs when an update would overwrite your local edits, and choose per-file what to keep
- **Pack Hub** — share curated addon collections with the community (no other manager has this)
- **ESO Logs uploader** — turn a session into a report on esologs.com without leaving the app
- **SavedVariables manager** — view and edit addon settings directly in the app
- **Multi-instance support** — handles native and Steam clients across NA, EU, and PTS servers
- **Open source** — community contributions welcome, with regular releases

---

## Features

### Addon Management
- **Smart scanning** — auto-detects your ESO AddOns folder and parses every addon manifest, including embedded libraries up to 3 levels deep
- **One-click install** — paste an ESOUI URL or addon ID to install instantly, with automatic dependency resolution
- **Bulk updates** — check for updates on startup and update everything at once, or pick exactly which addons to update; large updates show per-addon progress and can be stopped mid-run
- **Update conflict resolution** — when an update would overwrite locally edited files, Kalpa shows a file-level diff so you can choose per-file whether to keep your changes or accept the update
- **Safe removal** — remove addons with dependency warnings so you don't break other addons
- **Safety Center** — see dependency warnings and conflicts at a glance before making changes
- **Plays nice with a running game** — updating while ESO is open warns instead of blocking (changes apply on `/reloadui` or relog), and if Windows Controlled Folder Access silently blocks writes, Kalpa detects it and walks you through the fix
- **ESOUI integration** — uses ESOUI's public JSON API and public pages for reliable metadata, versions, and download links

### Discovery
- **Search ESOUI** — find new addons by keyword directly in the app
- **Browse by category** — explore addons organized by category with sorting and pagination
- **Popular addons** — browse the ESOUI Popular tab with filters and enhanced UX
- **Addon details** — view descriptions, screenshots, download stats, compatibility info, and more before installing

### ESO Logs Uploader
- **Upload without leaving Kalpa** — sign in with your ESO Logs account and send a finished session straight to esologs.com, or hand off to the official uploader if you'd rather
- **Live logging** — stream an ongoing raid so your report fills in as you play
- **Session splitting** — split a multi-session `Encounter.log` into separate files, or upload only the fights you pick
- **Fast on huge logs** — multi-gigabyte archives anchor to the newest session instead of rescanning the whole file
- **Progress you can trust** — direct uploads show a phase stepper, a determinate progress bar, and a live time-remaining estimate
- **You stay in control** — choose report visibility before anything is sent, and an upload history keeps a link to every report

### Pack Hub (Community Addon Collections)
- **Browse packs** — discover curated addon collections shared by the community
- **Create and publish** — build your own packs with required/optional addons and descriptions
- **Pack types** — addon packs, build packs, and roster packs for different use cases
- **Upvote system** — vote on packs to surface the best collections
- **Share codes** — generate temporary 6-character codes to share packs with friends
- **File export** — save packs as `.esopack` files for offline sharing, with optional account-wide addon settings (v2 format, automatically scrubbed for privacy)
- **Deep links** — open packs directly via `kalpa://pack/` URLs, including roster pack installs from the ESO Toolkit website
- **One-click install** — install all addons from a pack with a single click, including shared addon settings from v2 packs

### Themes and Appearance
- **49 built-in themes** — recolour the whole app from Settings → Appearance: eight Elder Scrolls art skins (Nordic Runestone, Daedric Obsidian, Dwemer Brass, Hermaeus Mora, and more), ESO faction palettes, and editor classics like Dracula, Nord, and Catppuccin
- **Custom theme builder** — pick 12 seed colours with live preview; a built-in WCAG AA contrast check keeps your theme legible, and themes copy and paste as text for sharing
- **No theme flash** — your theme is applied before the window paints, so the app opens directly in your colours

### Addon File Browser
- **Browse source files** — explore the file tree of any installed addon
- **In-app editing** — open and edit addon Lua, XML, and text files directly in Kalpa
- **Edit backups** — automatic backups before edits so you can always restore the original

### Tagging and Organization
- **Custom tags** — create and assign your own tags to organize addons
- **Preset tags** — quick-access tags for favorite, essential, utility, and more
- **Dynamic filters** — filter your addon list by any tag with live counts
- **Smart filters** — built-in filters for All, Addons, Libraries, Favorites, Outdated, and Issues
- **Sort by recency** — order by Recently Updated or Recently Downloaded; the download date refreshes on every update, not just first install

### Profiles
- **Save configurations** — snapshot your current addon setup as a named profile
- **Quick switching** — swap between profiles (e.g., "PvP", "Raiding", "Casual")
- **Activation preview** — before a switch, see exactly which addons will be enabled or disabled and which required libraries stay on, so an old snapshot can never silently disable a library your addons need
- **Update, rename, inspect** — overwrite a profile with your current setup, rename it inline, and expand it to see what's inside, with a "modified" marker when your setup has drifted
- **Survives an AddOns wipe** — profiles are mirrored in Kalpa's app data and restored automatically if the AddOns folder is deleted or reset

### Backups and Characters
- **Full backups** — back up all SavedVariables with one click; custom label is optional
- **Character-specific backups** — back up settings for individual characters
- **Safe restore** — automatic safety snapshot taken before every restore so you can always undo
- **Protection status** — at-a-glance indicator shows when you last backed up and whether you're covered
- **Finds every character** — even in ESO's default Account-Wide settings mode, which hides most of your roster from other tools, Kalpa recovers character names from SavedVariables so your whole roster is listed
- **Surgical per-character restore** — backing up or restoring one character touches only that character's data, keeps same-named characters on NA and EU separate, and is crash-safe end to end

### SavedVariables Manager
- **Browse settings** — view all addon SavedVariables files, with size stats and orphan detection
- **Settings that mean something** — Kalpa reads installed addons' LibAddonMenu source and turns raw values into labelled toggles and dropdowns (the scan is purely textual — no addon code is executed)
- **Search every setting** — find settings anywhere in an addon's tree with jump-to-group
- **Profile management** — copy and delete SavedVariables profiles
- **Auto-backups** — automatic backups before edits so you can always restore

### Multi-Instance and Migration
- **Game instance detection** — automatically finds native and Steam ESO installations
- **Region support** — handles NA, EU, and PTS servers
- **Instance switcher** — a header badge shows which install you're managing and opens a quick-switch menu when you have more than one, so you always know where installs and updates are going
- **Copy your setup to another instance** — install all of your enabled addons into another instance in one click (setting up PTS from your live loadout, for example), including update metadata and tags, without touching what the target already has
- **Setup wizard** — guided first-run setup with multi-candidate addon folder detection
- **Minion migration** — import your existing Minion addon tracking data with dry-run preview, integrity checks, and snapshots before changes. Kalpa never deletes your original Minion data, so if something looks wrong you can always roll back from a backup

### Additional Features
- **Native performance UI (beta, Windows)** — an opt-in mode that relaunches Kalpa as a lightweight fully native app using noticeably less memory, with addon management, the uploader, and Pack Hub; switch back anytime, and Kalpa falls back to the standard UI automatically if native mode can't start
- **API compatibility checking** — identify addons that are outdated for the current game API version
- **Addon list export/import** — share your installed addon list as JSON and import on another machine
- **Deep link scheme** — `kalpa://` URLs for packs, share codes, and addon installs
- **Auto-update** — the app checks for and installs its own updates via signed GitHub Releases
- **System tray** — hides to the system tray on window close with a Show/Quit context menu
- **Custom window chrome** — native-feeling desktop experience with a custom title bar
- **Offline detection** — graceful handling when you're not connected
- **Keyboard navigation** — navigate the addon list with arrow keys

---

## Security & privacy

Kalpa is built to be trustworthy with your game files and your data:

- **Allowlisted downloads** — addon downloads are restricted to ESOUI's official hosts; arbitrary URLs are rejected
- **Hardened file handling** — every Tauri IPC command runs through centralized path validation, and ZIP extraction uses streaming hashing with recursion caps to resist zip bombs and path-traversal
- **Locked-down webview** — a strict Content-Security-Policy with `frame-ancestors 'none'` blocks clickjacking and untrusted embedding
- **DoS-resistant Pack Hub** — the Cloudflare Worker backend uses rate limiting and a Durable Object for atomic pack-index mutations
- **Audited dependencies** — `npm audit` (production dependencies) and `cargo audit` run in CI on every push. Advisories with no available upstream fix are assessed individually and documented in [`ci.yml`](.github/workflows/ci.yml); today that's two quick-xml DoS advisories that none of Kalpa's code paths can reach
- **Signed auto-updates** — updates are delivered through signed GitHub Releases; see [Verify your download](docs/verify-download.md)

**Privacy of shared settings:** when you export account-wide addon settings in a `.esopack` v2 pack, Kalpa automatically scrubs personal data (account handles, character names and IDs, chat logs, mail, friends/roster lists, trade history, and similar) before the file is written, and re-maps the placeholders to *your* identity on import. See [What's scrubbed in `.esopack` v2](docs/settings-export.md) for the full list of what is removed, what is kept, and the caveats.

**Log uploads:** uploading to ESO Logs sends your combat log to esologs.com under your own account, and you choose the report's visibility before anything is sent. See [PRIVACY.md](PRIVACY.md) for what each upload path transmits.

To report a vulnerability, see [SECURITY.md](SECURITY.md).

---

## Screenshots

### Discover and install

Browse and search ESOUI in the app — descriptions, screenshots, stats, and compatibility, with dependencies resolved on install.

<p align="center">
  <img src=".screenshots/discover.webp" alt="Discover panel showing ESOUI's most popular addons, with the selected addon's stats, screenshot gallery, and description" width="800" />
</p>

### ESO Logs uploader

Send a finished session to esologs.com, or live-log an ongoing raid — without leaving Kalpa.

<p align="center">
  <img src=".screenshots/log-uploader.webp" alt="Upload to ESO Logs window with the Upload a Log and Live Log modes, a list of detected log files, and the signed-in account" width="800" />
</p>

### Pack Hub

Share curated addon collections with the community. Browse, create, vote, and install packs.

<p align="center">
  <img src=".screenshots/pack-hub.webp" alt="Pack Hub browse tab listing community addon packs with vote counts and type filters" width="800" />
</p>

<details>
<summary>More Pack Hub screenshots — pack details and the create flow</summary>

<p align="center">
  <img src=".screenshots/pack-hub-detail.webp" alt="Pack detail view listing the pack's addons with install status and a one-click install action" width="800" />
</p>
<p align="center">
  <img src=".screenshots/pack-hub-create.webp" alt="Pack Hub create form with pack name, description, type, and tags" width="800" />
</p>
<p align="center">
  <img src=".screenshots/pack-hub-create-addons.webp" alt="Second step of pack creation, selecting installed addons and marking each required or optional" width="800" />
</p>

</details>

### Themes

49 built-in themes plus a custom theme builder, all in Settings → Appearance.

<p align="center">
  <img src=".screenshots/themes.webp" alt="Appearance settings showing the theme gallery with Elder Scrolls skins and the active Nordic Runestone theme" width="800" />
</p>

### SavedVariables manager

Browse, search, and edit addon settings — with real labels read from each addon's own LibAddonMenu definitions.

<p align="center">
  <img src=".screenshots/saved-variables.webp" alt="SavedVariables editor showing an addon's settings tree beside labelled toggles and numeric inputs" width="800" />
</p>

<details>
<summary>More SavedVariables screenshots — overview and orphan detection</summary>

<p align="center">
  <img src=".screenshots/saved-variables-overview.webp" alt="SavedVariables overview listing addon settings files by size with orphaned-file detection" width="800" />
</p>

</details>

### Backups

Full and character-specific backups, with a safety snapshot taken before every restore.

<p align="center">
  <img src=".screenshots/backups.webp" alt="Backup and Restore window showing protection status and saved backups with restore actions" width="800" />
</p>

### Settings

Point Kalpa at your AddOns folder, switch between game instances, and opt into the native performance UI.

<p align="center">
  <img src=".screenshots/settings.webp" alt="General settings with the AddOns folder path, the NA, EU, and PTS instance switcher, and the native performance UI toggle" width="800" />
</p>

<details>
<summary>More screenshots — profiles and the tools menu</summary>

<p align="center">
  <img src=".screenshots/profiles.webp" alt="Addon Profiles window showing a saved profile scoped to the active game instance" width="800" />
</p>
<p align="center">
  <img src=".screenshots/settings-tools.webp" alt="Tools settings listing backups, characters, API compatibility, app updates, Minion migration, and Safety Center" width="800" />
</p>

</details>

---

## Install

### Platform support

| Platform | Status | Download | Notes |
|---|---|---|---|
| **Windows** 10 (1803+) / 11 | Stable | `.exe` (NSIS) | WebView2 pre-installed on Win 11, bootstrapped automatically on Win 10 |
| **macOS** 10.15+ | Beta | `.dmg` (universal) | Intel & Apple Silicon; see [macOS first launch](#macos-first-launch) below |
| **Linux** x86_64 | Beta | `.AppImage` / `.deb` / `.rpm` | AppImage recommended (it self-updates); detects ESO under Steam Proton |

> [!IMPORTANT]
> Every release ships each installer alongside a `.sig` (auto-updater signature) and a shared `latest.json`. See [Verify your download](docs/verify-download.md) to check the integrity of the file you downloaded.

### Pre-built (recommended)

Download the latest installer from the [Releases](https://github.com/ESO-Toolkit/kalpa/releases/latest) page. Kalpa auto-updates after install — you'll see a banner when a new version is available. (`.deb`/`.rpm` installs are the exception: they don't self-update, so grab new versions from the Releases page or your package manager.)

#### macOS first launch

macOS builds are not yet notarized with Apple, so Gatekeeper needs a nudge the first time: **right-click Kalpa.app → Open → Open**. If macOS reports the app as "damaged", clear the quarantine flag instead:

```bash
xattr -dr com.apple.quarantine /Applications/Kalpa.app
```

ESO's native Mac client stores addons in `~/Documents/Elder Scrolls Online/live/AddOns`, which Kalpa detects automatically (CrossOver bottles are scanned too).

#### Linux notes

ESO runs on Linux through Steam Proton; Kalpa automatically finds your AddOns folder inside the Proton prefix (`steamapps/compatdata/306130/pfx/...`), including Flatpak/Snap Steam installs and secondary Steam libraries. Staying logged in to ESO Logs requires a Secret Service keyring (GNOME Keyring or KWallet — present on stock GNOME/KDE); without one, Kalpa still works but asks you to log in each launch.

### Build from source

**Prerequisites (all platforms):**
- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 22+

**Windows:**
- **MSVC** toolchain, [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **"Desktop development with C++"** workload
- [WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) runtime (pre-installed on Windows 11)

**macOS:**
- Xcode Command Line Tools: `xcode-select --install`

**Linux (Debian/Ubuntu — adjust for your distro):**
```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf libssl-dev libxdo-dev build-essential curl wget file
```

```bash
git clone https://github.com/ESO-Toolkit/kalpa.git
cd kalpa
npm install
npm run check:env       # verify prerequisites
npm run tauri dev       # development mode
npm run tauri build     # production build
```

The production build outputs installers to `src-tauri/target/release/bundle/` — NSIS `.exe` on Windows, `.app`/`.dmg` on macOS, `.AppImage`/`.deb`/`.rpm` on Linux.

### Troubleshooting

| Problem | Solution |
|---|---|
| **"MSVC toolchain not found"** | Run `rustup default stable-x86_64-pc-windows-msvc` to switch toolchains |
| **Build fails with linker errors** | Install Visual Studio Build Tools with the "Desktop development with C++" workload |
| **WebView2 not found at runtime** | Download the [Evergreen Bootstrapper](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) and run it |
| **App blocked by antivirus** | Add an exception for `kalpa.exe` or the install directory in your antivirus software |
| **Updates fail with "access denied", or addons vanish in-game** | Windows Controlled Folder Access is likely blocking writes. Kalpa detects this and offers a guided fix, or allow `kalpa.exe` yourself under Windows Security → Ransomware protection |
| **npm run tauri dev fails** | Run `npm run check:env` to identify which prerequisite is missing |
| **macOS: "Kalpa is damaged and can't be opened"** | The build isn't notarized yet — run `xattr -dr com.apple.quarantine /Applications/Kalpa.app` |
| **Linux: login doesn't persist between launches** | Install/enable a Secret Service keyring (GNOME Keyring or KWallet) |
| **Linux: ESO install not detected** | Kalpa scans Steam Proton prefixes (native, Flatpak, Snap Steam). Launch ESO once so the prefix exists, or set the AddOns path manually in Settings |
| **White screen on launch** | Ensure WebView2 is installed and up to date; try reinstalling it |

---

## How It Works

| Layer | What it does |
|---|---|
| **Manifest parser** | Reads `.txt` and `.addon` files from each addon folder — extracts title, version, author, dependencies, API version |
| **Manifest cache** | SQLite-backed cache for fast rescans without re-parsing every file |
| **Dependency resolver** | Scans the full AddOns tree (up to 3 levels deep) to find installed libraries, including those embedded inside other addons |
| **ESOUI client** | Fetches addon metadata via ESOUI's public JSON API and public pages — no private APIs |
| **Metadata tracker** | Persists ESOUI IDs, versions, tags, and install dates in `kalpa.json` inside your AddOns folder |
| **File hash tracker** | Tracks file hashes to detect local edits and power update conflict resolution |
| **Pack Hub worker** | Cloudflare Worker + KV that powers community pack sharing, voting, and share codes |
| **SavedVariables parser** | Reads and writes ESO's Lua-based SavedVariables files with change tracking |
| **Log uploader** | Scans, splits, encodes, and uploads ESO `Encounter.log` sessions to ESO Logs, including live streaming |

---

## Tech Stack

- **Desktop app**: [Tauri v2](https://v2.tauri.app/) — Rust backend plus the system webview (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux)
- **Frontend**: React 19 + TypeScript + Vite
- **Styling**: Tailwind CSS v4 + shadcn/ui
- **Native performance UI** (beta, Windows): [Slint](https://slint.dev/) sidecar (`kalpa-slint`), opt-in from Settings
- **Backend**: Cloudflare Workers + KV (Pack Hub)
- **HTTP**: reqwest
- **HTML parsing**: scraper
- **ZIP handling**: zip crate
- **Local database**: rusqlite (bundled SQLite)
- **SavedVariables**: Custom Lua parser

---

## Project Structure

```
src/                        # React frontend
  components/               # Feature components (addon list, packs, settings, etc.)
  components/ui/            # shadcn-ui primitives
  components/uploader/      # ESO Logs uploader workspace
  hooks/                    # Shared React hooks
  lib/                      # Utilities, Tauri bindings, store, theme presets
  types.ts                  # Shared TypeScript interfaces

src-tauri/src/              # Rust backend
  commands.rs               # All Tauri command handlers
  esoui.rs                  # ESOUI API client and HTML scraping
  manifest.rs               # ESO addon manifest parser
  manifest_cache.rs         # SQLite-backed manifest cache
  installer.rs              # ZIP extraction and addon installation
  metadata.rs               # Metadata tracking and persistence
  file_hashes.rs            # File hashing for update conflict detection
  edit_backups.rs           # Backup system for addon file edits
  safe_migration.rs         # Minion migration with dry-run and snapshots
  game_instances.rs         # Multi-instance detection (native/Steam)
  platform.rs               # Cross-platform helpers (Steam/Proton discovery, open_url)
  settings_store.rs         # Atomic app-settings persistence
  saved_variables/          # SavedVariables parser, editor, scrubbing, per-character backups
  uploader/                 # ESO Logs uploader (scan, split, encode, direct + live upload)
  auth.rs                   # Authentication
  token_store.rs            # Secure credential storage (Credential Manager / Keychain / Secret Service)
  lib.rs                    # Module definitions and app setup

backend/                    # Cloudflare Workers
  eso-packs-worker/         # Pack Hub API (packs, votes, shares)
    src/index.ts            # Router and handlers
    src/kv.ts               # KV read/write helpers
    src/types.ts            # Pack types (snake_case)
    src/validate.ts         # Input validation
    src/shares.ts           # Share code generation/resolution
    src/cors.ts             # CORS config
    src/seed.ts             # Dev seed data
    src/pack-index-do.ts    # Durable Object for atomic index mutations

prototypes/slint-kalpa/     # Native (Slint) performance UI sidecar
context/                    # Architecture and design documentation
docs/                       # User-facing and design docs
```

---

## Contributing

Contributions are welcome! Please read our [Code of Conduct](CODE_OF_CONDUCT.md) and [Contributing Guide](CONTRIBUTING.md) before opening a PR.

## Security

To report a vulnerability, see [SECURITY.md](SECURITY.md).

## License

[BSL 1.1](LICENSE) — converts to Apache 2.0 four years after each release.
