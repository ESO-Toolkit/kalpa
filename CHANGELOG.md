# Changelog

All notable changes to Kalpa are documented here. This project uses [Conventional Commits](https://www.conventionalcommits.org/).

## [0.4.0] — 2026-03-30

First release under the **Kalpa** name on [ESO-Toolkit/kalpa](https://github.com/ESO-Toolkit/kalpa).

### Changed
- Rebranded from "ESO Addon Manager" to **Kalpa**
- Moved repository to the ESO-Toolkit organization
- License changed from MIT to BSL 1.1 (converts to Apache 2.0 after four years)
- Deep link scheme changed to `kalpa://`

### Added
- Upgraded discovery UI with Popular tab, better filters, and enhanced UX (#52)
- Multi-layer performance optimizations across Rust, React, and Worker (#43)
- Multi-candidate addon folder detection with setup wizard (#35)
- Code of Conduct (Contributor Covenant v2.1)
- Changelog

### Fixed
- CSP `connect-src` for production builds (#32)

## [0.3.0] — 2026-03-11

### Added
- Private pack sharing via share codes and `.esopack` file export (#30)
- Dynamic tag tabs replacing static Tagged/Untracked filters (#31)
- Pack upvote system (#19)
- ESOTK branding and JSON API migration (#17, #18)

### Fixed
- Exclude libraries from pack creation and surface dependency installs (#29)
- Hardened addon path validation across Tauri commands (#23, #24)
- Restored drag-region window permission (#27)
- Refactored app shell and centralized Tauri error handling (#26)

## [0.2.0] — 2026-02-15

### Added
- Addon packs system with Cloudflare Worker, UI, and deep linking (#15)
- Tags, filters, ESOUI API integration, and performance improvements (#13)
- Auto-update with signed GitHub Releases (#11)
- Install/remove buttons for dependencies in addon detail (#12)

### Fixed
- LIB indicator alignment in title line (#16)

## [0.1.0] — 2026-01-20

Initial release as ESO Addon Manager.

### Added
- Smart addon scanning with manifest parsing (`.txt` and `.addon` files)
- One-click install from ESOUI URL or addon ID
- Automatic dependency resolution (3 levels deep)
- Bulk update checking and one-click update all
- Browse and search ESOUI with addon detail view and screenshots
- Profiles for quick addon set switching
- Full and character-specific backups with restore
- Character management grouped by server (NA/EU)
- API compatibility checking
- Addon list export/import (JSON)
- Minion migration (one-click import)
- Custom window chrome with integrated Discover tab
- Keyboard navigation
- Offline detection

[0.4.0]: https://github.com/ESO-Toolkit/kalpa/releases/tag/v0.4.0
[0.3.0]: https://github.com/ESO-Toolkit/kalpa/releases/tag/v0.3.0
[0.2.0]: https://github.com/ESO-Toolkit/kalpa/releases/tag/v0.2.0
[0.1.0]: https://github.com/ESO-Toolkit/kalpa/releases/tag/v0.1.0
