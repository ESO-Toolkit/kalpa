# Protected Edits — Design Plan

> **Status**: Proposed (May 2026)  
> **Goal**: Let users edit addon files and keep their changes across updates.

---

## Problem Statement

When Kalpa updates an addon, it extracts the new ZIP directly over the existing folder, overwriting all files. Users who have manually edited `.lua` or `.xml` files (to change a color, tweak a font size, adjust a threshold, etc.) lose their changes silently.

No existing addon manager in any game ecosystem solves this well for script-based addons. This is a genuine innovation opportunity.

---

## Design: Hash-Based Edit Protection

Inspired by `dpkg`'s conffiles system. The core idea:

1. **At install/update time**, compute a SHA-256 hash for every file in the addon and store it.
2. **Before the next update**, re-hash all files on disk and compare against stored hashes.
3. **Apply a decision matrix** per file to determine what action to take.

### Decision Matrix

| User Modified? | Upstream Changed? | Action |
|:-:|:-:|:--|
| No | No | Skip (no change needed) |
| No | Yes | Overwrite silently |
| Yes | No | **Keep user's version** (upstream didn't touch it) |
| Yes | Yes | **Present to user for review** |

This is the same algorithm Linux package managers have used for decades. It's battle-tested and users intuitively understand it.

---

## Architecture

### New Data: File Hash Manifest

A **separate file** from `kalpa.json` to avoid bloating the existing metadata store.

**Location**: `<addons_dir>/.kalpa-hashes/<folder_name>.json`

**Schema**:
```json
{
  "addon_folder": "LibAddonMenu-2.0",
  "esoui_id": 7,
  "recorded_at": "2026-05-03T12:00:00Z",
  "installed_version": "2.26",
  "files": {
    "LibAddonMenu-2.0.txt": "b3a1f2c4d5e6...(64 hex chars)",
    "LibAddonMenu-2.0.lua": "d7e8a9b0c1d2...(64 hex chars)",
    "controls/dropdown.lua": "1a2b3c4d5e6f...(64 hex chars)",
    "controls/slider.lua": "5e6f7a8b9c0d...(64 hex chars)"
  },
  "modified_files": ["controls/slider.lua"]
}
```

- `files` keys are **relative paths** within the addon folder (forward-slash normalized).
- `files` values are **SHA-256 hex strings** (64 chars each).
- `modified_files` is a **cached list** of files whose current disk hash differs from the stored hash. Updated during update scans, explicit rescans, and after in-app saves. Used by the Files tab for instant display without re-hashing on every view.
- One file per addon folder (not one giant file for all addons).

**Why separate from kalpa.json?**
- A large addon collection (100+ addons, 10,000+ files) would add ~1-2MB of hash data.
- Keeps the existing metadata store clean and fast to parse.
- Hash files can be regenerated from disk if lost (non-critical data).

### Hash Algorithm: SHA-256

- Already a dependency (`sha2` crate) with a proven `sha256_file()` implementation in `safe_migration.rs`.
- No new crate needed — reuse existing infrastructure.
- Cryptographic collision resistance (important since addon ZIPs come from untrusted ESOUI sources).
- For small files (5-50KB Lua scripts), the difference between SHA-256 and faster alternatives is measured in microseconds — irrelevant when filesystem I/O dominates.

### Performance Budget

- **Target**: < 1 second for a full conflict scan during "Update All".
- **Reality**: 10,000 files × ~25KB average = ~250MB of I/O. On SSD, ~1-2 seconds worst case.
- **Optimization 1**: Use `rayon` (already a dependency) for parallel file I/O.
- **Optimization 2**: Only hash at update-apply time, not on every "check for updates" (ESOUI version comparison handles update detection).
- **Optimization 3**: Fast path — compare file size first. Only hash if size differs from original or file mtime is newer than install time.

---

## Update Flow (Modified)

### Current Flow
```
1. Download ZIP → 2. Extract over existing folder → 3. Update kalpa.json
```

### New Flow: Pre-Scan Architecture

The key constraint: `batch_update_addons_blocking` runs on a `spawn_blocking` thread and cannot pause for UI interaction. Therefore, conflict detection and user decisions happen **before** the update executes.

**Important**: The current extraction does NOT delete the addon folder first — it overwrites files in place. Files removed in new versions linger on disk. User-added files naturally survive. This means the selective extraction approach only needs to *skip* overwriting protected files — no backup/restore dance needed.

#### Single Addon Update

```
Frontend                          Rust Backend
───────                          ────────────
1. User clicks "Update"
   │
   ├─► scan_update_conflicts(addon, zip_url)
   │                              ├─ Download ZIP to temp
   │                              ├─ Load stored hashes
   │                              ├─ Hash current files on disk
   │                              ├─ Hash files inside ZIP (without extracting)
   │                              └─ Return ConflictReport:
   │                                   - safe_files: [...] (no user edits)
   │                                   - auto_kept: [...] (user edited, upstream unchanged)
   │                                   - conflicts: [...] (both changed)
   │◄─────────────────────────────────┘
   │
2. IF conflicts.is_empty():
   │    proceed immediately
   │ ELSE:
   │    show conflict dialog, collect per-file decisions
   │
   ├─► update_addon_with_decisions(addon, decisions)
   │                              ├─ Extract ZIP, skipping "keep mine" files
   │                              ├─ Back up overwritten user files
   │                              ├─ Record new hashes
   │                              └─ Return success
   │◄─────────────────────────────────┘
```

#### Batch Update ("Update All")

```
Frontend                          Rust Backend
───────                          ────────────
1. User clicks "Update All"
   │
   ├─► scan_batch_conflicts(updates)
   │                              ├─ Download ALL ZIPs (parallel, 4 threads)
   │                              ├─ keep() ALL temp files (one download, no re-fetch)
   │                              ├─ Store ALL in PendingUpdates by session UUID
   │                              ├─ For each addon with a hash manifest:
   │                              │    compare disk hashes vs stored vs ZIP
   │                              └─ Return:
   │                                   - no_conflict_addons: [{ session_id, folder }...]
   │                                   - conflicting_addons: [
   │                                       { session_id, folder, conflicts: [...] }
   │                                     ]
   │◄─────────────────────────────────┘
   │
2. For each no_conflict_addon: call update_addon_with_decisions
   │ with empty decisions array (extract everything)
   │ (progress bar: "Updating 97 of 100...")
   │
3. Show banner: "3 addons need your attention"
   │ User reviews each at their leisure
   │ (addons are flagged with amber "needs review" badge)
   │
4. For each conflicting addon, user opens review panel:
   │    per-file: "Keep my version" / "Take the update" / "View differences"
   │
   ├─► update_addon_with_decisions(session_id, decisions)
   │                              └─ (same as single update)
   │◄─────────────────────────────────┘
```

**Single download, no re-fetch**: The scan downloads every ZIP once and persists them all via `keep()`. The frontend then calls `update_addon_with_decisions` for every addon — non-conflicting ones pass an empty `decisions` array (meaning "extract everything"). This avoids the double-download problem.

**Backward compatibility**: The existing `batch_update_addons` command remains for addons without hash manifests (installed before this feature). Once all addons have hash manifests (after one update cycle), the new pipeline handles everything.

### Key Principle: Never Block the Happy Path

- 97% of addons in a typical "Update All" have no user modifications. They update instantly.
- The ~3% with conflicts are flagged for async review. The user is never forced to resolve them immediately.
- A global preference ("Always take updates, back up my files") auto-resolves all conflicts without any prompt.

---

## Tauri Commands (New)

### `scan_update_conflicts`

```rust
#[tauri::command]
pub async fn scan_update_conflicts(
    addons_path: String,
    folder_name: String,
    esoui_id: u32,
) -> Result<ConflictReport, String>
```

Returns:
```typescript
interface ConflictReport {
  session_id: string;          // UUID — opaque handle for subsequent commands
  folder_name: string;
  update_version: string;
  safe_files: string[];        // no user edits, will overwrite
  auto_kept_files: string[];   // user edited, upstream unchanged — auto-preserved
  conflicts: FileConflict[];   // both changed — needs decision
}

interface FileConflict {
  relative_path: string;
  user_hash: string;
  upstream_hash: string;
}
```

### `get_conflict_diff`

```rust
#[tauri::command]
pub async fn get_conflict_diff(
    state: tauri::State<'_, PendingUpdates>,
    addons_path: String,
    session_id: String,
    relative_path: String,
) -> Result<DiffData, String>
```

Looks up the session UUID in managed state to find the ZIP path. Reads the user's on-disk file and the upstream version directly from the ZIP (via `archive.by_name()` — no extraction needed). Returns both as strings for the frontend diff viewer.

```typescript
interface DiffData {
  user_content: string;
  upstream_content: string;
  is_binary: boolean;  // if true, frontend shows "Keep mine / Take update" only
}
```

### `update_addon_with_decisions`

```rust
#[tauri::command]
pub async fn update_addon_with_decisions(
    state: tauri::State<'_, PendingUpdates>,
    addons_path: String,
    session_id: String,
    esoui_id: u32,
    decisions: Vec<FileDecision>,
) -> Result<InstallResult, String>
```

```typescript
interface FileDecision {
  relative_path: string;
  action: "keep_mine" | "take_update";
}
```

Looks up session UUID in state to get the ZIP path. Extracts using selective extraction (skipping files where `action == "keep_mine"`), backs up overwritten user files, records new hashes, removes the session from state, and deletes the temp ZIP.

### `list_addon_files`

```rust
#[tauri::command]
pub async fn list_addon_files(
    addons_path: String,
    folder_name: String,
) -> Result<AddonFileTree, String>
```

Walks the addon folder, returns a flat list of files with extension, size, and modification status (from cached `modified_files` in the hash manifest). See File Browser section for `AddonFileTree` TypeScript interface.

### `read_addon_file`

```rust
#[tauri::command]
pub async fn read_addon_file(
    addons_path: String,
    folder_name: String,
    relative_path: String,
) -> Result<String, String>
```

Reads a single addon file as UTF-8 text. Rejects binary files (null bytes in first 512 bytes). Path-traversal validated: `folder_name` checked for `..`/`/`/`\`, final path canonicalized and verified within addons directory.

### `write_addon_file`

```rust
#[tauri::command]
pub async fn write_addon_file(
    addons_path: String,
    folder_name: String,
    relative_path: String,
    content: String,
) -> Result<(), String>
```

Writes content to an addon file. Same path-traversal validation as `read_addon_file`. After writing, re-hashes the file and updates the `modified_files` cache in the hash manifest.

---

## ZIP Temp File Lifecycle

Conflict scanning downloads the ZIP before the user makes decisions. The ZIP must survive until the user resolves conflicts (seconds to minutes later). Pattern:

1. Download ZIP using `tempfile::Builder::new().prefix("kalpa-pending-").suffix(".zip").tempfile()`
2. Call `.keep()` to detach auto-delete (returns `PathBuf`)
3. Store the `PathBuf` in Tauri managed state: `PendingUpdates(Mutex<HashMap<String, PendingUpdate>>)` keyed by UUID session ID
4. Frontend receives the UUID, not the file path (security: prevents webview from pointing at arbitrary files)
5. When user resolves conflicts, frontend sends UUID + decisions → Rust looks up the path, extracts, then deletes the file
6. On app startup, sweep `%TEMP%` for orphaned `kalpa-pending-*.zip` files (crash recovery)

This matches the existing state patterns in `lib.rs` (`AllowedAddonsPath`, `AuthState`, `TrayState`, `PendingDeepLink` — all `Mutex`-wrapped).

Windows does not auto-delete temp files (Storage Sense only targets files unchanged for 1+ days, disabled by default). Files are safe for the entire review session.

---

## Backup System for Overwritten Edits

When a user's modified file is overwritten (either by explicit choice or by the global preference):

**Location**: `<addons_dir>/.kalpa-backups/<folder_name>/<timestamp>/`

Contains:
- The user's version of the overwritten file(s), preserving relative path structure.
- A `manifest.json` with metadata:
  ```json
  {
    "addon_folder": "MyAddon",
    "backed_up_at": "2026-05-03T14:30:00Z",
    "update_from": "1.2.0",
    "update_to": "1.3.0",
    "files": ["MyAddon.lua", "config.lua"]
  }
  ```

Users can restore individual files from the addon detail view at any time.

**Retention**: Keep the last 5 backup snapshots per addon. Older ones are pruned automatically.

---

## Frontend UX

### Editing Philosophy

**Users edit files externally (or in Kalpa's built-in editor). Kalpa's value is discoverability and preservation.**

Research findings:
- r2modman's config editor is the gold standard in mod management UX — users love it for **discoverability** (seeing files grouped by mod), not for the editor itself (which is a basic textarea).
- ESO addon edits are overwhelmingly **single-value changes**: `local FONT_SIZE = 16` → `20`, color hex changes, threshold tweaks. Multi-line or structural edits are rare.
- The typical editor is a "confident copy-paster" — not a Lua programmer, but comfortable changing one value if shown where it is.
- No ESO tool currently shows users which files they can edit or which files they've already modified. This alone is a competitive advantage.

**Decision: Two-layer approach.**

| Layer | What | Effort | Value |
|---|---|---|---|
| **1. File Browser** | "Files" tab in addon detail — shows all files with modification status, "Open in Explorer" / "Open in Editor" buttons | Low | High (discoverability) |
| **2. Built-in Editor** | CodeMirror 6 inline editor with Lua/XML syntax highlighting, dark glass theme | Medium | Polish (premium feel) |

Layer 1 ships with Phase 4 (conflict UI). Layer 2 ships as Phase 6 (independent, non-blocking).

---

### File Browser (Layer 1)

A new **"Files" tab** on the addon detail panel. Entry point for all editing and modification visibility.

```
┌─────────────────────────────────────────────────────────┐
│  Bandit's UI                                            │
│  ┌──────────┬──────────┐                                │
│  │  Details  │  Files  │                                │
│  └──────────┴──────────┘                                │
│                                                         │
│  ┌ BanditsUI/                                           │
│  │  ├─ BanditsUI.txt               stock               │
│  │  ├─ BanditsUI.lua         ● modified                │
│  │  ├─ settings.lua          ● modified                │
│  │  ├─ modules/                                         │
│  │  │  ├─ combat.lua               stock               │
│  │  │  └─ unitframes.lua    ● modified                 │
│  │  └─ textures/                                        │
│  │     └─ bar.dds                  stock               │
│  │                                                      │
│  │  3 files edited · Protected on update                │
│  └──────────────────────────────────────────────────────│
│                                                         │
│  [Open Folder]  [Open in Editor]  [Rescan]              │
└─────────────────────────────────────────────────────────┘
```

**UI details:**
- File tree is a custom recursive component (no library needed — 2-3 levels max, ~50 lines of React).
- Status indicators: gold dot for "modified", no dot for "stock". Uses existing `InfoPill` color system.
- File type labels: small `LUA` / `XML` / `TXT` / `DDS` badges using `InfoPill` (sky for lua, amber for xml, muted for txt/other).
- **Clicking a file** in the tree opens it in the inline CodeMirror editor (Phase 6). Before Phase 6 ships, clicking falls back to `openPath` (system default editor).
- **"Open Folder"** button uses `revealItemInDir` from `@tauri-apps/plugin-opener` (already a dependency v2.5.3) — opens the addon folder in Windows Explorer.
- **"Open in Editor"** button always uses `openPath` from `@tauri-apps/plugin-opener` — opens the selected file in the system default editor. This remains available even after Phase 6 as a power-user escape hatch (e.g., user prefers VS Code for complex edits).
- "Rescan" button re-computes hashes for the addon and updates modification status.

**When is modification status computed?**
- During update scans (already in the flow — compare disk hashes vs stored hashes).
- The scan result is cached in the hash manifest as `modified_files: string[]`.
- The Files tab displays the cached state immediately (no hashing on every detail view open).
- The "Rescan" button triggers a fresh hash comparison and updates the cache.
- After the user saves a file from the built-in editor, re-hash that single file and update the cache.

**Tauri commands needed (security-critical):**

```rust
/// Read a single addon file. Path-traversal validated.
#[tauri::command]
pub async fn read_addon_file(
    addons_path: String,
    folder_name: String,
    relative_path: String,
) -> Result<String, String>

/// Write a single addon file. Path-traversal validated.
#[tauri::command]
pub async fn write_addon_file(
    addons_path: String,
    folder_name: String,
    relative_path: String,
    content: String,
) -> Result<(), String>

/// List all files in an addon folder with their modification status.
#[tauri::command]
pub async fn list_addon_files(
    addons_path: String,
    folder_name: String,
) -> Result<AddonFileTree, String>
```

Both `read_addon_file` and `write_addon_file` MUST validate:
1. `folder_name` contains no `..`, `/`, or `\` (same check as `remove_addon`).
2. `relative_path` contains no `..` components.
3. After joining, canonicalize the result and verify it `starts_with` the canonicalized addons directory.
4. Reject binary files for `read_addon_file` (check for null bytes in first 512 bytes).

```typescript
interface AddonFileTree {
  folder_name: string;
  files: AddonFileEntry[];
  modified_count: number;
}

interface AddonFileEntry {
  relative_path: string;      // "modules/combat.lua"
  is_directory: boolean;
  size_bytes: number;
  status: "stock" | "modified" | "unknown"; // unknown = no hash manifest yet
  extension: string;          // "lua", "xml", "txt", "dds"
}
```

---

### Built-in Editor (Layer 2)

An inline CodeMirror 6 editor embedded in the Files tab when a user clicks a file.

**Technology choice: CodeMirror 6 via `@uiw/react-codemirror`**
- ~100KB min+gzip total (core + lua + xml + theme + wrapper). Negligible for a desktop app.
- React 19 compatible (hooks-based, no class components).
- Fully customizable theming (translucent backgrounds, gold/sky accents match our glass morphism).
- Lua syntax highlighting via `@codemirror/legacy-modes` (stream mode, functional).
- XML syntax highlighting via `@codemirror/lang-xml` (first-class CM6 package).
- Read-only mode available for stock files (encourage intentional edits).

**Custom dark theme:**
```typescript
// Matches Kalpa's glass morphism design system
settings: {
  background: 'rgba(10, 12, 18, 0.6)',      // translucent glass
  foreground: '#e2e8f0',
  caret: '#38bdf8',                          // sky-blue cursor
  selection: 'rgba(56, 189, 248, 0.15)',
  lineHighlight: 'rgba(196, 164, 74, 0.08)', // gold line highlight
  gutterBackground: 'transparent',
  gutterForeground: 'rgba(255, 255, 255, 0.25)',
}
// Syntax colors: keywords=gold, strings=emerald, numbers=amber, variables=sky
```

**Editor UX flow:**
1. User clicks a file in the tree → editor opens below/beside the tree.
2. File loads via `read_addon_file` Tauri command.
3. Stock files open in **read-only mode** with a subtle banner: "This file hasn't been edited. Click 'Enable Editing' to modify it."
4. Modified files open in **edit mode** directly.
5. "Unsaved changes" indicator appears when content differs from disk.
6. Explicit **Save** button (not auto-save) — writes via `write_addon_file`, then re-hashes the single file.
7. **Revert** button restores the file to its last-saved state (from disk, not from original install).
8. Binary files (.dds, .ttf) show a "Binary file — cannot edit" message with file size info.

**Vite configuration required (CM6 singleton gotcha):**
```typescript
// vite.config.ts additions — prevents module duplication in production builds
optimizeDeps: {
  exclude: ['@codemirror/state', '@codemirror/view', '@codemirror/language'],
},
resolve: {
  dedupe: ['@codemirror/state', '@codemirror/view', '@codemirror/language'],
},
```

**Not building:**
- A structured settings GUI (VS Code-style key-value editor over Lua) — ESO addons have no standardized config format, would be fragile and low-coverage.
- Full Monaco editor (~2.4MB, overkill).
- Auto-save or live-reload — file editing in addon scripts isn't a live-preview workflow.

---

### Addon Card Badge

When Kalpa has cached modification status (from the last update scan or explicit rescan):

- A small badge appears on the addon card: **"2 edited files"** (muted, informational).
- Clicking it opens the Files tab showing which files were modified.

### Batch Update Flow

1. **Pre-scan** (1-2 seconds after ZIPs download): categorize all addons.
2. **Immediate progress**: non-conflicting addons start updating with the existing progress bar.
3. **Banner notification**: "3 addons need your attention" — persistent, not blocking.
4. **Amber badge** on conflicting addon cards in the list.
5. User clicks an amber-badged addon → opens the review panel.

### Conflict Review Panel

Shown inline in the addon detail view (not a modal that blocks other interaction):

```
┌─────────────────────────────────────────────────────┐
│  MyAddon v1.2.0 → v1.3.0                           │
│                                                     │
│  You've edited 2 files that this update also        │
│  changed. Choose what to do for each:               │
│                                                     │
│  ┌─────────────────────────────────────────────┐    │
│  │  MyAddon.lua                                │    │
│  │  [Keep my version]  [Take the update]       │    │
│  │  [View differences]                         │    │
│  └─────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────┐    │
│  │  config.lua                                 │    │
│  │  [Keep my version]  [Take the update]       │    │
│  └─────────────────────────────────────────────┘    │
│                                                     │
│  4 other files will update normally                 │
│  (you haven't modified them)                        │
│                                                     │
│         [Apply & Update]       [Skip Update]        │
└─────────────────────────────────────────────────────┘
```

### Diff Viewer

A custom split-view renderer built on the `diff` npm package (v9.0.0, zero dependencies, built-in TypeScript types, ~12KB gzipped).

**Technical approach:**
1. Call `structuredPatch('', '', userContent, upstreamContent, '', '', { context: 3 })` from the `diff` package.
2. Iterate the returned `hunks` array. Each hunk contains `lines[]` prefixed with `' '` (context), `'-'` (removed from user's version), `'+'` (added in update).
3. Render two columns: left (user's version) shows context + `-` lines; right (the update) shows context + `+` lines. Spacer rows align the sides.
4. Style with Tailwind + glass morphism: gold highlights for user's changes, sky-blue for upstream changes, `bg-white/[0.03]` for context lines.

**Why not an off-the-shelf React diff viewer:**
- `react-diff-viewer-continued` has an open React 19 compatibility issue (#63) and uses `emotion` (conflicts with our design system).
- `diff2html` bundles highlight.js and is overweight for our needs.
- A custom renderer on top of `structuredPatch` output is ~50-80 lines of JSX with full styling control.

**Rendering rules:**
- Line numbers, monospace font (`font-mono`).
- No merge editing — view only. User decides, then clicks a button.
- For binary files (.dds): show file size comparison only, no diff.
- Collapse hunks with >50 unchanged lines between them (expand on click).

### Language Guidelines

Use gamer-friendly language throughout:

| Instead of... | Say... |
|---|---|
| "Resolve conflicts" | "Choose which version to keep" |
| "Local modifications" | "Your changes" / "Your edits" |
| "Upstream/incoming" | "The update" |
| "Merge" | (don't use this word) |
| "Diff" | "Differences" / "What changed" |
| "Stash" | "Back up" / "Save a copy" |

---

## Implementation Plan

### Phase 1: Hash Infrastructure (Rust)

**Files to create/modify:**
- `src-tauri/src/file_hashes.rs` (new module)
  - `compute_addon_hashes(addon_path) -> HashMap<String, String>` — walks addon directory, SHA-256 hashes each file
  - `hash_zip_entries(zip_path, folder_name) -> HashMap<String, String>` — hashes files inside a ZIP without extracting (via `archive.by_name()`)
  - `save_hash_manifest(addons_dir, folder_name, manifest)` — writes to `.kalpa-hashes/`
  - `load_hash_manifest(addons_dir, folder_name) -> Option<HashManifest>` — reads from `.kalpa-hashes/`
  - `detect_modifications(addons_dir, folder_name) -> ModificationReport` — compares disk state against stored hashes
- `src-tauri/src/lib.rs` — register new module

**Infrastructure reused:**
- `sha2` crate (already a dependency)
- `sha256_file()` pattern from `safe_migration.rs`
- `rayon` (already a dependency) for parallel hashing
- `serde_json` + `save_json_with_backup()` pattern from `metadata.rs`

**No new crates required for this phase.**

**Integration points for hash recording:**
- `install_addon_blocking()` in `commands.rs` — after extraction, compute and save hashes for each installed folder.
- `update_addon_blocking()` — after extraction (existing path without conflict scanning), record hashes.
- Auto-installed dependency addons — same code path as above (they flow through `install_addon_blocking`).
- This ensures every newly installed addon gets a hash manifest from day one.

### Phase 2: Conflict Scanning Commands (Rust)

**Files to modify:**
- `src-tauri/src/lib.rs`
  - Add `PendingUpdates(Mutex<HashMap<String, PendingUpdate>>)` to managed state
  - Add startup cleanup for orphaned `kalpa-pending-*.zip` files in `%TEMP%`

- `src-tauri/src/commands.rs`
  - Add `scan_update_conflicts` — downloads ZIP (using `tempfile::Builder` with `kalpa-pending-` prefix), calls `.keep()`, stores `PathBuf` in `PendingUpdates` state under a UUID, compares three hash sets (stored, disk, ZIP), returns `ConflictReport` with session UUID
  - Add `scan_batch_conflicts` — parallel version for "Update All" (reuses rayon pool pattern from existing batch update)
  - Add `get_conflict_diff` — looks up session UUID in state, reads user's file from disk + upstream file from ZIP via `archive.by_name()`, returns both as strings
  - Add `update_addon_with_decisions` — looks up session UUID, performs selective extraction, backs up overwritten files, removes session from state, deletes temp ZIP

- `src-tauri/src/installer.rs`
  - Add `extract_addon_zip_selective(zip_path, addons_dir, skip_files: &HashSet<String>)` — identical to existing `extract_addon_zip` but adds a `continue` on line 36 for relative paths in `skip_files`. This is a 3-line change to the existing extraction loop

### Phase 3: Backup System (Rust)

**Files to create/modify:**
- `src-tauri/src/edit_backups.rs` (new module)
  - `backup_user_files(addons_dir, folder_name, files: &[String]) -> BackupManifest`
  - `restore_backup_file(addons_dir, folder_name, backup_timestamp, relative_path)`
  - `list_backups(addons_dir, folder_name) -> Vec<BackupManifest>`
  - `prune_old_backups(addons_dir, folder_name, keep: usize)`
- `src-tauri/src/commands.rs`
  - Add `list_edit_backups` command
  - Add `restore_edit_backup` command

### Phase 4: Frontend — Conflict UI + File Browser (React)

**Files to create/modify:**
- `src/components/update-conflict-panel.tsx` (new) — inline review panel for conflicting addons
- `src/components/diff-viewer.tsx` (new) — custom split-view diff renderer
- `src/components/addon-file-browser.tsx` (new) — file tree with status indicators + action buttons
- `src/components/addon-card.tsx` — add "edited files" badge + amber "needs review" state
- `src/components/addon-detail.tsx` — add "Files" tab, backup restore section
- `src/components/update-all-banner.tsx` (new) — "X addons need your attention" persistent banner

**Rust commands for file browser:**
- `list_addon_files` — walks addon folder, returns file tree with modification status (from cached hash manifest)
- `read_addon_file` — reads a single file with path-traversal validation + canonicalize check
- `write_addon_file` — writes a single file with same validation, then re-hashes and updates manifest cache

**Dependencies to add:**
- `diff` v9.0.0 (npm) — zero dependencies, built-in TypeScript types, ~12KB gzipped. Use `structuredPatch()` for hunk-based diff data

**File tree implementation:**
- Custom recursive React component (~50 lines). No library needed for 2-3 level trees.
- File type badges via `InfoPill`: `LUA` (sky), `XML` (amber), `TXT` (muted).
- Status: gold dot = modified, no dot = stock.
- Actions: "Open Folder" (`revealItemInDir`), "Open in Editor" (`openPath`), "Rescan".

### Phase 5: Settings & Preferences

- Add a global setting: "When my edited files conflict with an update"
  - Options: "Ask me each time" (default) / "Always keep my version" / "Always take the update (back up my files)"
- Wire into both single-update and batch-update flows.
- Setting stored in Kalpa's existing settings infrastructure.

### Phase 6: Built-in Editor (CodeMirror 6)

**Independent from Phases 1-5. Can ship separately as a polish release.**

**Files to create/modify:**
- `src/components/addon-file-editor.tsx` (new) — CodeMirror wrapper with Kalpa theme
- `src/lib/kalpa-codemirror-theme.ts` (new) — custom dark theme matching glass morphism
- `src/components/addon-file-browser.tsx` — wire file click → open in editor
- `vite.config.ts` — add `optimizeDeps.exclude` and `resolve.dedupe` for CM6 singletons

**npm dependencies to add:**
- `@uiw/react-codemirror` — React 19 compatible wrapper (~15KB gzip)
- `@codemirror/legacy-modes` — Lua stream mode (~3KB gzip)
- `@codemirror/lang-xml` — native XML package (~8KB gzip)
- `@uiw/codemirror-themes` — `createTheme` helper (~2KB gzip)
- `@lezer/highlight` — syntax tag definitions (peer dep)

Total bundle addition: ~100KB min+gzip. Acceptable for desktop app.

**Editor features:**
- Lua + XML syntax highlighting
- Read-only mode for stock files (with "Enable Editing" button)
- Dirty state indicator ("unsaved" label)
- Explicit Save button (calls `write_addon_file` + single-file re-hash)
- Revert button (re-reads from disk)
- Line numbers, bracket matching, search (Cmd/Ctrl+F)
- Binary file detection (show "Cannot edit" for .dds/.ttf)

**Vite config gotcha (required for production builds):**
CM6 uses module-level singletons. Without deduplication, Vite/Rollup may create multiple instances in production builds, causing silent failures. The `optimizeDeps.exclude` + `resolve.dedupe` config is mandatory.

---

## Edge Cases & Decisions

| Scenario | Decision |
|---|---|
| User deletes a file from the addon | Treat as "user modified" (hash mismatch = file missing). Don't re-create it on update unless user explicitly chooses "Take the update" |
| User adds a new file to the addon folder | Ignore — not tracked in hash manifest. ZIP extraction won't overwrite it (ZIPs don't delete extra files) |
| Hash manifest is missing/corrupted | Treat all files as unmodified. Record hashes after this update completes. Protection activates on the *next* update |
| Addon was installed before this feature existed | Same as above — first update with the feature records initial hashes |
| Auto-installed dependency addons | Hash them at install time (same code path — `record_installed_folders` already handles deps) |
| Binary files (.dds textures) in conflict | No diff view — just "Keep mine" / "Take update" buttons |
| User clicks "Skip Update" | Addon stays at current version. Badge remains. Offer again on next update check |
| ZIP contains new files not in old version | Extract them normally (no hash comparison needed for new files) |
| ZIP removes files that existed in old version | Leave them on disk — current behavior. ZIP extraction never deletes files. This is fine; stale files are harmless |
| Temp ZIP is cleaned up before user reviews conflicts | ZIPs are `keep()`-ed in `%TEMP%` with `kalpa-pending-` prefix, stored in Tauri managed state by UUID. Cleaned on resolution or on next app startup |
| Multiple addons from the same ZIP (multi-folder) | Track hashes per folder independently. Conflict scan checks each folder in the ZIP |
| User's file has different line endings than ZIP version | Hash raw bytes — CRLF/LF difference IS a modification. Diff viewer handles both gracefully |
| App crashes mid-extraction | Worst case: some files overwritten, some not. Pre-update hash manifest is still intact. User can re-run the update. No data loss because user's protected files were skipped (never overwritten) |
| User edits a file via built-in editor then updates | Save triggers single-file re-hash → `modified_files` cache updates immediately → next update scan correctly detects the modification |
| User edits a file externally (VS Code, Notepad) | Modification not detected until next "Rescan" or update scan. The Files tab shows cached state. "Rescan" button re-hashes and updates |
| User tries to read/write a binary file (.dds) via editor | `read_addon_file` rejects files with null bytes in first 512 bytes. Frontend shows "Binary file — cannot edit" |

---

## What This Is NOT

- **Not a merge tool.** No automatic three-way merging of Lua code. Users pick one version per file. (Automatic merging may be added in a future phase if there's demand, using the `diffy-imara` Rust crate.)
- **Not a "child addon" generator.** The hook-based child addon pattern (creating a separate addon with `DependsOn` that uses `ZO_PreHook`) is a separate, developer-oriented feature for a future release. It requires Lua knowledge and breaks when parent addons restructure their code.
- **Not a version control system.** One snapshot of hashes at install time, plus backup copies of overwritten files. Not a full git-style history.
- **Not a file watcher.** Hashes are computed on-demand (during update scans), not continuously monitored.

---

## Future Enhancements (Out of Scope for v1)

- **Automatic three-way merge**: For cases where user and upstream changes don't overlap, auto-merge using `diffy-imara` and only prompt on true conflicts.
- **Child Addon Generator**: Scaffold a separate `_Custom` addon with hooks for power users who want ESO-native overrides.
- **Edit annotations**: Let users mark specific lines/blocks as "protected" with comment markers, enabling finer-grained preservation.
- **Conflict prediction**: On the "updates available" screen, show which addons will have conflicts *before* the user initiates the update.

---

## Success Criteria

1. A user who edits `MyAddon.lua` to change a color value can update the addon and keep their change, without any technical knowledge.
2. The update flow adds < 1 second of overhead for hash comparison on a typical addon (< 200 files).
3. No data is ever silently lost — modified files are either preserved in place or backed up before overwriting.
4. The feature works retroactively for existing installations after one update cycle (first update records hashes, second update onward gets protection).
5. "Update All" for 100 addons where 97 have no conflicts completes without any user interaction for those 97.
6. The UX is approachable enough that a gamer who has never seen a diff viewer can make an informed choice.
7. A user can discover *which* files they've edited (and which they haven't) without leaving Kalpa — the Files tab makes this visible at a glance.
8. A user can make a simple edit (change a number, color, string) directly in Kalpa without needing to install an external editor or navigate file paths manually.

---

## Technical Validation (Completed)

The following was verified during research:

**Rust Backend:**
- **SHA-256 crate** (`sha2` v0.11) is already a dependency with a working `sha256_file()` in `safe_migration.rs` — no new crate needed.
- **rayon** (v1.12) is already a dependency — used in `batch_update_addons_blocking` for parallel downloads.
- **`zip` crate** (v8.6.0) supports `archive.by_name()` for reading individual files without full extraction — enables hashing ZIP contents and streaming file content for diffs.
- **`tempfile` crate** is already a dependency. `NamedTempFile::keep()` detaches auto-deletion, returning a `PathBuf` that persists until manually deleted.
- **Tauri managed state** pattern (`Mutex<HashMap<...>>`) is already used for `AllowedAddonsPath`, `AuthState`, `TrayState`, and `PendingDeepLink` — adding `PendingUpdates` is consistent.
- **Selective extraction** is trivial: add a `HashSet<String>` of paths to skip via `continue` in the existing `extract_addon_zip` loop (line 31-34 in `installer.rs`).
- **Current update flow does NOT delete addon folders** before extraction — it overwrites in place. User-added files and files not in the new ZIP naturally persist. This means the "skip protected files" approach has zero data-loss risk.
- **`serde_json`** + the `save_json_with_backup()` pattern from `metadata.rs` can be reused for hash manifest persistence.

**Frontend:**
- **`diff` npm package** (v9.0.0, April 2026) — zero dependencies, built-in TypeScript types, ~12KB gzipped. `structuredPatch()` returns hunk-based data ideal for custom split-view rendering.
- **No React 19 issues** with `diff` (pure computation library, no React dependency). The alternative `react-diff-viewer-continued` has an open React 19 compatibility issue — avoid it.
- **Custom diff renderer** on top of `structuredPatch` output is ~50-80 lines of JSX with full Tailwind styling control.
- **CodeMirror 6** via `@uiw/react-codemirror` — ~100KB total gzip addition. React 19 compatible (hooks-based). Custom theming API supports translucent backgrounds and arbitrary accent colors. Lua via `@codemirror/legacy-modes` (stream mode), XML via `@codemirror/lang-xml` (native). Known Vite production build gotcha (singleton duplication) has documented fix (`optimizeDeps.exclude` + `resolve.dedupe`).
- **File tree** — custom recursive React component, no library needed. 2-3 level depth max for addon folders.
- **`@tauri-apps/plugin-opener`** (already v2.5.3) — exports `revealItemInDir` for "Open Folder" and `openPath` for "Open in Default Editor". No new dependency.

**Performance:**
- File I/O is the bottleneck, not hashing. SHA-256 processes 300-400 MB/s; reading 10,000 small files from SSD takes ~1-2 seconds regardless of hash algorithm.
- Storage impact is ~1-2MB total for 10,000 files across all addons — negligible.

**Competitive landscape:**
- No game modding tool in any ecosystem (Minion, ESOLL, WowUp, Vortex, MO2) currently offers this feature for script-based addons. MO2's VFS works for binary assets only.
- ESO API version is 101049 (Update 49, March 2026). The addon ecosystem is active with ~4000 addons on ESOUI.
- ESO's load order is dependency-based and undefined beyond that — no user-configurable priority system exists.

**Total new dependencies:**
- Rust: 0 new crates
- npm (Phase 4): `diff` (~12KB gzip)
- npm (Phase 6): `@uiw/react-codemirror`, `@codemirror/legacy-modes`, `@codemirror/lang-xml`, `@uiw/codemirror-themes`, `@lezer/highlight` (~100KB gzip combined)
