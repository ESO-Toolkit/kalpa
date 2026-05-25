# Desktop Client

> **Note:** This is an early planning document. The desktop client now includes many features beyond what's listed here: profiles, backups, protected edits, saved variables manager, migration wizard, Pack Hub, character management, and more. See `CLAUDE.md` for the current architecture.

Use Tauri for the desktop app unless there is a strong reason not to.

Client responsibilities:

- Detect and validate the ESO AddOns folder
- Scan installed addons by reading manifest `.txt` files
- Parse addon name, version, author, and dependency declarations
- Install ZIP files directly into the AddOns folder
- Update addons by replacing their folder contents
- Show installed addons, missing dependencies, and update status
- Refresh metadata on app open and with a manual button

Implementation notes:

- Keep filesystem logic in the Tauri backend
- Keep UI logic in the frontend
- Store only enough local state to remember installed addons and their source IDs
- Make path override possible for nonstandard installs

MVP UI:

- Installed addons list
- Addon detail panel
- Install by ESOUI URL or ID
- Refresh button
- Update all button
