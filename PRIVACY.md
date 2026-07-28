# Privacy Policy

**Last updated:** 2026-07-27

Kalpa is a source-available desktop application for managing Elder Scrolls Online (ESO) addons. This policy explains what data Kalpa collects, how it is used, and your rights.

## Age Requirement

In accordance with the Elder Scrolls Online Terms of Service, users must be at least 18 years old or have parental/guardian consent. By using Kalpa, you confirm that you meet the age requirements of both the ESO Terms of Service and your local jurisdiction.

---

## Data We Collect

### Data stored on your computer

Kalpa keeps its own files in a per-user application-data directory, referred to
below as **`{app data}`**. Its location depends on your operating system:

| Platform | `{app data}` |
|---|---|
| Windows | `%APPDATA%\com.kalpa.desktop\` |
| macOS | `~/Library/Application Support/com.kalpa.desktop/` |
| Linux | `~/.local/share/com.kalpa.desktop/` (or `$XDG_DATA_HOME/com.kalpa.desktop/`) |

| Data | Location | Purpose |
|------|----------|---------|
| Addon metadata (ESOUI IDs, versions, tags) | `{AddOns folder}/kalpa.json` | Track installed addons |
| User preferences (sort mode, theme, paths) | `{app data}/settings.json` | Remember your settings |
| Addon profiles | `{AddOns folder}/kalpa-profiles.json` | Addon profile switching |
| SavedVariables backups | `{ESO folder}/kalpa-backups/` | Backup/restore functionality |
| File hash manifests | `{AddOns folder}/.kalpa-hashes/` | Detect user-modified files |
| Manifest cache (SQLite) | `{app data}/manifest-cache.db` | Speed up addon scanning |
| Upload history | `{app data}/upload-history.json` | Show past uploads in the uploader panel |
| Auth tokens | OS credential store | Sign in to Pack Hub |
| Upload session cookie | OS credential store | Direct upload to ESO Logs |

**Upload history** records the uploads you have made from Kalpa: the local path
and file name of the log you uploaded, the resulting ESO Logs report code and
URL, the zone and fight count, and the build-evidence summary described below.
It stays on your computer — it is a local record of uploads you already made,
not a separate transmission — and is capped at the 200 most recent entries.

**Auth tokens** (ESO Logs OAuth access and refresh tokens) and **Upload session
cookie** (`wcl_session` for ESO Logs authentication) are stored in your
operating system's credential store rather than in plaintext files:

| Platform | Credential store |
|---|---|
| Windows | Credential Manager (encrypted with your Windows account credentials) |
| macOS | Keychain |
| Linux | Secret Service (GNOME Keyring / KWallet, via D-Bus) |

On Linux systems with no Secret Service daemon running, credential storage is
unavailable and Kalpa simply asks you to sign in again each launch — nothing is
written to disk as a fallback. The upload session cookie is removed when you
sign out.

### Data sent to ESOUI

When you search, browse, install, or update addons, Kalpa makes HTTPS requests to ESOUI's public API (`api.mmoui.com`, `www.esoui.com`, `cdn.esoui.com`). These requests include:

- Addon IDs and search queries
- A User-Agent header identifying the app (includes a standard browser-compatibility prefix and `Kalpa/{version}`)

No personal information, auth tokens, or machine identifiers are sent to ESOUI.

### Data sent to the Pack Hub

The Pack Hub (`kalpa-pack-hub.eso-toolkit.workers.dev`) powers community addon collections. When you sign in and use Pack Hub features, the following data is transmitted:

**When you create or edit a pack:**
- Your ESO Logs display name and user ID (as the pack author)
- Pack content: title, description, addon list (ESOUI IDs and names), tags

**When you vote on a pack:**
- Your ESO Logs user ID (to track your vote)

**When you share a pack via share code:**
- Your ESO Logs display name (visible to anyone with the share code)
- Pack content (title, description, addon list)

**When you install a pack (install count tracking):**
- Your IP address is stored in a rate-limiting key for **1 hour** to prevent duplicate counting, then automatically deleted

**When you export a `.esopack` file with settings:**
- SavedVariables data is scrubbed of personal information (account names, character names, character IDs, and world names are replaced with placeholders) before export

#### Published packs are also copied to the ESO Log Aggregator's database

The Pack Hub does not keep published packs to itself. When a pack's status is
**published**, the Pack Hub worker writes a copy of it into `roster-hub-db` —
the same Cloudflare D1 database that powers [esotk.com](https://esotk.com) — so
the website can list community packs alongside the desktop app. This happens
inline with the write, not as a separate opt-in.

**What is copied:** the pack's ID, title, description, pack type, addon list
(ESOUI IDs, names, required flags, notes), tags, and its author fields.

**What this means for anonymous packs:** if you mark a pack anonymous, your ESO
Logs **display name is not copied** — it is replaced with "Anonymous" before the
write, and esotk.com never renders a name for it. Your numeric ESO Logs **user
ID is still copied**, because the row is keyed to it for ownership. So an
anonymous pack is anonymous to readers of the site, but the copy is not
un-linkable from your account inside the database itself.

**What is not copied:** individual vote records stay in the Pack Hub's own
storage and are never written to the shared database. Neither are share codes or
the install-count rate-limit keys.

**Drafts and deletions:** a pack that is a draft, or that you switch back to
draft, is actively removed from the shared database rather than copied to it.
Deleting a pack, or deleting your Pack Hub data, deletes the copied row too.

### Data sent to ESO Logs

Sign-in uses OAuth via [esotk.com](https://esotk.com), which handles the authentication flow with ESO Logs. During sign-in, the only data retrieved from ESO Logs is your **numeric user ID** and **display name**.

**Uploading logs:** Kalpa also includes an opt-in log uploader. When you choose to upload an ESO encounter log, that log is sent to **ESO Logs** (esologs.com) — either through the official ESO Logs uploader or, if you enable direct upload, straight from Kalpa. This only happens for logs you explicitly upload; Kalpa never uploads combat logs in the background.

### Direct upload to ESO Logs (opt-in)

Kalpa includes an optional direct-upload feature for combat logs to ESO Logs. When enabled:

- **Combat-log contents are uploaded only on explicit user action** — you must click "Upload" for each log or session. No background or automatic uploads occur.
- **Report visibility is user-chosen** — you control whether a report is **Unlisted** (default, visible only via direct link), **Public** (listed on your profile), or **Private** (not visible to others). You choose the visibility in Kalpa before each upload; direct uploads apply it immediately, while the official-uploader handoff lets you confirm it there.
- **Upload session authentication** — a session cookie (`wcl_session`) is captured from ESO Logs' login page inside Kalpa, stored in the Windows Credential Manager, and used only for upload authentication. This cookie is removed when you sign out.
- **Alternative: handoff to official uploader** — if you disable direct upload or are not signed in, Kalpa can launch ESO Logs' standalone desktop uploader instead, which handles the upload in a separate application.

### Data sent to the ESO Log Aggregator (build evidence)

When you upload a log using Kalpa's **direct (in-app) uploader** and that ESO Logs report is **public or unlisted**, Kalpa also publishes a small "build evidence" record to the ESO Log Aggregator (`roster-hub-api.eso-toolkit.workers.dev`, the backend for [esotk.com](https://esotk.com)). ESO Logs does not retain these facts as report data, so this sidecar lets the Aggregator show richer build details next to your report.

**What is published** — recovered from the same encounter log you are uploading to ESO Logs, for every player in that log:

- Character name, `@account` name, and character ID
- Class, race, level, and champion-point total
- Champion-point passives and class-mastery choices
- Food/drink buff and scribed-skill abilities

**When it is published:**

- Only for **public or unlisted** reports uploaded via the direct uploader. **Private reports never publish build evidence.**
- The upload is authenticated with your ESO Logs OAuth token, and the Aggregator verifies you own the report before storing anything.

**Current limitations you should know about:**

- Publishing is keyed to the ESO Logs report code and is **not tied to ESO Logs' anonymization** — if you upload a public/unlisted report, the identities of everyone in your group are included in the build evidence.
- There is currently **no automatic deletion**: making the report private or deleting it on ESO Logs later does **not** automatically remove the stored build-evidence record. See *Your Rights* below for removal.

### Data sent to GitHub

Kalpa checks for app updates by fetching a public JSON file from GitHub Releases. No user data is sent — GitHub will see standard HTTP request metadata (your IP address and the Tauri updater User-Agent).

---

## Data We Do NOT Collect

- **No analytics or telemetry** — Kalpa contains zero tracking, analytics libraries, or usage metrics
- **No crash reporting** — no error data is sent to any server
- **No addon file contents** — your addon source code (.lua, .xml) is never uploaded
- **No background game-data collection** — Kalpa never reads or transmits your inventory, guild data, or gameplay on its own. The only game data that leaves your machine is a combat log **you** choose to upload (to ESO Logs), plus the build-evidence summary described above for public/unlisted direct uploads
- **No machine fingerprinting** — no hardware IDs, OS version telemetry, or device identifiers

---

## Data Retention

| Data | Retention |
|------|-----------|
| Published packs | Indefinite (until you delete them) |
| Copies of published packs in the ESO Log Aggregator database | Deleted together with the pack |
| Votes | Indefinite (until you remove your vote) |
| Share codes | 7 days (auto-deleted) |
| Install rate-limit keys (IP) | 1 hour (auto-deleted) |
| Pack Hub dated daily backups | 90 days (auto-deleted) |
| Pack Hub "latest" backup snapshot | Overwritten daily, no expiry; scrubbed of your data when you delete it |
| Build-evidence records (ESO Log Aggregator) | Indefinite — no automatic deletion yet (see *Your Rights*) |
| Local backups | Until you delete them manually |

---

## Your Rights

### Delete your Pack Hub data

You can delete all your data from the Pack Hub at any time:

1. Sign in to the Pack Hub (the option is only shown while signed in)
2. Open Kalpa Settings
3. In the **Pack Hub Data** section, click **Delete My Pack Hub Data**
4. Confirm the deletion

This immediately removes your packs, your votes, and your share codes from the
Pack Hub's live data, and deletes the copies of your published packs from the
ESO Log Aggregator's database.

**What happens to backups:** the Pack Hub takes a daily snapshot of its pack
data for disaster recovery. Deleting your data also scrubs you from the
non-expiring "latest" snapshot at the time of deletion, but the **dated daily
snapshots are not rewritten** — your packs and votes remain in those until they
expire on their own, within **90 days**. Those snapshots are only ever read to
restore the service after data loss.

Two further limits worth stating plainly:

- Votes **other people** cast on your packs are not deleted, since they are
  other users' records. They are left behind as orphans once your packs are gone.
- Pack vote totals shown elsewhere are denormalized counters and are not
  recalculated when your votes are removed.

### Remove build-evidence records

Build-evidence records published to the ESO Log Aggregator are keyed to the ESO Logs report code. In-app deletion is planned but not yet available; until then, to have a build-evidence record removed, contact us (see *Contact* below) with the report code. Note that build evidence is only ever published for reports you made **public or unlisted** on ESO Logs.

### Sign out

Signing out removes your auth tokens from your operating system's credential
store (Credential Manager, Keychain, or Secret Service). No tokens are retained
after sign-out.

### Local data

All local data (addon metadata, backups, profiles, cache, upload history) is
stored on your computer. To remove it, uninstall the app and delete the
`{app data}` directory for your platform — see the table under *Data stored on
your computer* — along with the `kalpa-*` files and folders in your ESO AddOns
directory and the `kalpa-backups` folder in your ESO folder.

On Windows, note that Kalpa's data lives in `%APPDATA%` (roaming), not
`%LOCALAPPDATA%`; the `%LOCALAPPDATA%\com.kalpa.desktop\` folder holds only the
WebView2 browser cache.

---

## Third-Party Services

| Service | Purpose | Their Privacy Policy |
|---------|---------|---------------------|
| ESOUI | Addon catalog and downloads | [esoui.com](https://www.esoui.com) |
| ESO Logs | Authentication (OAuth) and log uploads | [esologs.com](https://www.esologs.com) |
| ESO Log Aggregator (esotk.com) | Build-evidence sidecar for public/unlisted direct uploads; hosts the shared database that published Pack Hub packs are copied into | [esotk.com](https://esotk.com) |
| Cloudflare | Pack Hub and ESO Log Aggregator hosting, rate limiting | [cloudflare.com/privacypolicy](https://www.cloudflare.com/privacypolicy/) |
| GitHub | App update distribution | [github.com/privacy](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement) |

---

## Changes to This Policy

We may update this privacy policy as features change. The "Last updated" date at the top will reflect the most recent revision. Significant changes will be noted in the changelog.

---

## Contact

For privacy questions or data deletion requests, reach out on Discord: **@spike_jones**

---

## Source Availability

Kalpa's full source is public at [github.com/ESO-Toolkit/kalpa](https://github.com/ESO-Toolkit/kalpa), so you can audit exactly what data the app handles. It is licensed under the Business Source License 1.1, which permits non-production and non-commercial use and converts to Apache 2.0 four years after each release. That is source-available rather than open source in the OSI sense.
