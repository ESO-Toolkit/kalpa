# Privacy Policy

**Last updated:** 2026-07-03

Kalpa is an open-source desktop application for managing Elder Scrolls Online (ESO) addons. This policy explains what data Kalpa collects, how it is used, and your rights.

## Age Requirement

In accordance with the Elder Scrolls Online Terms of Service, users must be at least 18 years old or have parental/guardian consent. By using Kalpa, you confirm that you meet the age requirements of both the ESO Terms of Service and your local jurisdiction.

---

## Data We Collect

### Data stored on your computer

| Data | Location | Purpose |
|------|----------|---------|
| Addon metadata (ESOUI IDs, versions, tags) | `{AddOns folder}/kalpa.json` | Track installed addons |
| User preferences (sort mode, theme, paths) | `%LOCALAPPDATA%\com.kalpa.desktop\settings.json` | Remember your settings |
| Addon profiles | `{AddOns folder}/kalpa-profiles.json` | Addon profile switching |
| SavedVariables backups | `{ESO folder}/kalpa-backups/` | Backup/restore functionality |
| File hash manifests | `{AddOns folder}/.kalpa-hashes/` | Detect user-modified files |
| Manifest cache (SQLite) | `%LOCALAPPDATA%\com.kalpa.desktop\` | Speed up addon scanning |
| Auth tokens | Windows Credential Manager | Sign in to Pack Hub |
| Upload session cookie | Windows Credential Manager | Direct upload to ESO Logs |

**Auth tokens** (ESO Logs OAuth access and refresh tokens) and **Upload session cookie** (`wcl_session` for ESO Logs authentication) are stored in the Windows Credential Manager, which encrypts them using your Windows account credentials. They are not stored in plaintext files. The upload session cookie is removed when you sign out.

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
| Votes | Indefinite (until you remove your vote) |
| Share codes | 7 days (auto-deleted) |
| Install rate-limit keys (IP) | 1 hour (auto-deleted) |
| Pack Hub daily backups | 90 days (auto-deleted) |
| Build-evidence records (ESO Log Aggregator) | Indefinite — no automatic deletion yet (see *Your Rights*) |
| Local backups | Until you delete them manually |

---

## Your Rights

### Delete your Pack Hub data

You can delete all your data from the Pack Hub at any time:

1. Open Kalpa Settings
2. In the Account section, click **Delete My Pack Hub Data**
3. Confirm the deletion

This permanently removes all your packs, votes, and share codes from our servers.

### Remove build-evidence records

Build-evidence records published to the ESO Log Aggregator are keyed to the ESO Logs report code. In-app deletion is planned but not yet available; until then, to have a build-evidence record removed, contact us (see *Contact* below) with the report code. Note that build evidence is only ever published for reports you made **public or unlisted** on ESO Logs.

### Sign out

Signing out removes your auth tokens from the Windows Credential Manager. No tokens are retained after sign-out.

### Local data

All local data (addon metadata, backups, profiles, cache) is stored on your computer and can be deleted by uninstalling the app or removing the `%LOCALAPPDATA%\com.kalpa.desktop\` directory and the `kalpa-*` folders in your ESO AddOns directory.

---

## Third-Party Services

| Service | Purpose | Their Privacy Policy |
|---------|---------|---------------------|
| ESOUI | Addon catalog and downloads | [esoui.com](https://www.esoui.com) |
| ESO Logs | Authentication (OAuth) and log uploads | [esologs.com](https://www.esologs.com) |
| ESO Log Aggregator (esotk.com) | Build-evidence sidecar for public/unlisted direct uploads | [esotk.com](https://esotk.com) |
| Cloudflare | Pack Hub and ESO Log Aggregator hosting, rate limiting | [cloudflare.com/privacypolicy](https://www.cloudflare.com/privacypolicy/) |
| GitHub | App update distribution | [github.com/privacy](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement) |

---

## Changes to This Policy

We may update this privacy policy as features change. The "Last updated" date at the top will reflect the most recent revision. Significant changes will be noted in the changelog.

---

## Contact

For privacy questions or data deletion requests, reach out on Discord: **@spike_jones**

---

## Open Source

Kalpa is open source. You can audit exactly what data the app handles by reviewing the source code at [github.com/ESO-Toolkit/kalpa](https://github.com/ESO-Toolkit/kalpa).
