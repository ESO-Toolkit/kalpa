# Privacy Policy

**Last updated:** 2026-05-25

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

**Auth tokens** (ESO Logs OAuth access and refresh tokens) are stored in the Windows Credential Manager, which encrypts them using your Windows account credentials. They are not stored in plaintext files.

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

Sign-in uses OAuth via [esotk.com](https://esotk.com), which handles the authentication flow with ESO Logs. The only data retrieved from ESO Logs is your **numeric user ID** and **display name**. No combat logs, guild data, character stats, or other game data is accessed.

### Data sent to GitHub

Kalpa checks for app updates by fetching a public JSON file from GitHub Releases. No user data is sent — GitHub will see standard HTTP request metadata (your IP address and the Tauri updater User-Agent).

---

## Data We Do NOT Collect

- **No analytics or telemetry** — Kalpa contains zero tracking, analytics libraries, or usage metrics
- **No crash reporting** — no error data is sent to any server
- **No addon file contents** — your addon source code (.lua, .xml) is never uploaded
- **No ESO game data** — no combat logs, character stats, guild info, or inventory data
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
| Local backups | Until you delete them manually |

---

## Your Rights

### Delete your Pack Hub data

You can delete all your data from the Pack Hub at any time:

1. Open Kalpa Settings
2. In the Account section, click **Delete My Pack Hub Data**
3. Confirm the deletion

This permanently removes all your packs, votes, and share codes from our servers.

### Sign out

Signing out removes your auth tokens from the Windows Credential Manager. No tokens are retained after sign-out.

### Local data

All local data (addon metadata, backups, profiles, cache) is stored on your computer and can be deleted by uninstalling the app or removing the `%LOCALAPPDATA%\com.kalpa.desktop\` directory and the `kalpa-*` folders in your ESO AddOns directory.

---

## Third-Party Services

| Service | Purpose | Their Privacy Policy |
|---------|---------|---------------------|
| ESOUI | Addon catalog and downloads | [esoui.com](https://www.esoui.com) |
| ESO Logs | Authentication (OAuth) | [esologs.com](https://www.esologs.com) |
| Cloudflare | Pack Hub hosting and rate limiting | [cloudflare.com/privacypolicy](https://www.cloudflare.com/privacypolicy/) |
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
