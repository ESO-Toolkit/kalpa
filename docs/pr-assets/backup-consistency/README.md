# Backup consistency review images

These images document the backup storage and recovery UX in this change. The
Backups and Safety Center images render the production React components with
mocked IPC and sample backup data. They do not show or modify a real ESO profile.

| Image                                                        | Review focus                                                                                                |
| ------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------- |
| [Backups, dark theme](backups-dark.png)                      | Manual, Automatic and Characters locations, storage format and origin labels, and the optional review action. |
| [Copy confirmation](consolidation-confirmation.png)          | Copying eligible older settings backups keeps the originals and current settings.                           |
| [Copy results, light theme](consolidation-results-light.png) | Copied and skipped counts, with an explanation for a destination collision.                                 |
| [Backups on a short screen](backups-short-screen-light.png)  | The dialog scrolls and its footer remains reachable at an 800 by 500 viewport.                              |
| [Safety Center, light theme](safety-center-light.png)        | Automatic snapshot coverage and recovery actions.                                                           |
| [Example folder layout](folder-layout-example.png)           | An illustrative folder diagram showing the canonical root, retained legacy folders, and addon edit backups. |

The folder diagram uses example names and dates. It is not an Explorer capture
or a complete directory listing. New backups use `KalpaBackups/Manual`,
`KalpaBackups/Automatic` and `KalpaBackups/Characters`. Existing shared-root
archives and legacy folders stay usable in place. Manual settings ZIPs and
automatic snapshots have different coverage; cleanup backups contain selected
files, character backups remain folders, and addon edit backups retain their
existing location.

The UI captures use the built-in Nordic Runestone dark theme and Paper White
light theme. The ignored `.codex/` preview fixtures are not part of the product.
