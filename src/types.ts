interface Dependency {
  name: string;
  min_version: number | null;
}

export interface AddonManifest {
  folderName: string;
  title: string;
  author: string;
  version: string;
  addonVersion: number | null;
  apiVersion: number[];
  description: string;
  isLibrary: boolean;
  dependsOn: Dependency[];
  optionalDependsOn: Dependency[];
  missingDependencies: string[];
  outdatedDependencies: string[];
  missingOptionalDependencies: string[];
  esouiId: number | null;
  tags: string[];
  esouiLastUpdate: number;
  disabled: boolean;
  modifiedFileCount: number;
}

export interface EsouiAddonInfo {
  id: number;
  title: string;
  version: string;
  downloadUrl: string;
  updated: string;
}

export interface InstallResult {
  installedFolders: string[];
  installedDeps: string[];
  failedDeps: string[];
  skippedDeps: string[];
}

export interface UpdateCheckResult {
  folderName: string;
  esouiId: number;
  currentVersion: string;
  remoteVersion: string;
  downloadUrl: string;
  hasUpdate: boolean;
}

export interface BatchUpdateResult {
  completed: string[];
  failed: string[];
  errors: Record<string, string>;
}

export interface BatchRemoveResult {
  removed: string[];
  failed: string[];
  errors: Record<string, string>;
}

export interface BatchTagResult {
  updated: string[];
  failed: string[];
  errors: Record<string, string>;
}

export interface BatchEnableResult {
  enabled: string[];
  disabled: string[];
  failed: string[];
  errors: Record<string, string>;
}

export interface ImportResult {
  installed: string[];
  failed: string[];
  skipped: string[];
  errors?: Record<string, string>;
}

export interface PackInstallEntry {
  esouiId: number;
  label?: string;
}

export interface PackInstallResult {
  installed: number[];
  failed: number[];
  installedFolders: string[];
  installedDeps: string[];
  failedDeps: string[];
  skippedDeps: string[];
  errors?: Record<number, string>;
}

export interface PackInstallProgress {
  esouiId: number;
  label: string;
  phase: "downloading" | "extracting" | "completed" | "failed";
  index: number;
  total: number;
}

export interface EsouiSearchResult {
  id: number;
  title: string;
  author: string;
  category: string;
  downloads: string;
  updated: string;
}

export interface BrowsePopularPage {
  results: EsouiSearchResult[];
  /** True when the upstream ESOUI page was full before library filtering. */
  hasMore: boolean;
}

export interface EsouiAddonDetail {
  id: number;
  title: string;
  version: string;
  author: string;
  description: string;
  compatibility: string;
  md5: string;
  totalDownloads: string;
  monthlyDownloads: string;
  favorites: string;
  updated: string;
  created: string;
  screenshots: string[];
  downloadUrl: string;
}

export interface EsouiCategory {
  id: number;
  name: string;
  depth: number;
}

export interface ApiCompatInfo {
  gameApiVersion: number;
  outdatedAddons: string[];
  upToDateAddons: string[];
}

export type BackupKind = "manual" | "autoBeforeRestore" | "character";

export interface BackupInfo {
  name: string;
  createdAt: string;
  createdAtEpoch: number;
  fileCount: number;
  totalSize: number;
  kind: BackupKind;
}

export interface SafeRestoreResult {
  restoredFiles: number;
  safetySnapshot: BackupInfo | null;
}

export interface AddonProfile {
  name: string;
  enabledAddons: string[];
  createdAt: string;
}

export interface ActivateProfileResult {
  enabled: string[];
  disabled: string[];
  failed: string[];
  missing: string[];
}

export interface CharacterInfo {
  server: string;
  name: string;
  /** True when recovered from SavedVariables addon data rather than
   * AddOnSettings.txt (e.g. account-wide addon-settings mode). */
  recovered?: boolean;
}

export interface CharacterRoster {
  characters: CharacterInfo[];
  /** SavedVariables files the roster scan had to skip (too large, unreadable,
   * or unparseable); when > 0 the list may be incomplete. */
  skippedFiles: number;
}

// ── Game instance types (multi-region / launcher detection) ──────────────
type ClientType = "native" | "steam";
type ServerRegion = "na" | "eu" | "pts";

export interface GameInstance {
  /** Region env-folder name: "live" | "liveeu" | "pts" */
  id: string;
  clientType: ClientType;
  region: ServerRegion;
  /** Absolute path to the AddOns directory for this instance. */
  addonsPath: string;
  /** Number of valid addon manifests detected. */
  addonCount: number;
  isOnedrive: boolean;
  hasSavedVariables: boolean;
  hasAddonSettings: boolean;
  /** Human-readable label, e.g. "Steam · EU" or "Native · NA". */
  displayLabel: string;
}

// ── SavedVariables Manager types ─────────────────────────────────────────
export interface SavedVariableFile {
  fileName: string;
  addonName: string;
  lastModified: string;
  sizeBytes: number;
  characterKeys: string[];
}

export interface SvTreeNode {
  key: string;
  valueType: "string" | "number" | "boolean" | "nil" | "table";
  value?: string | number | boolean | null;
  children?: SvTreeNode[];
  rawLuaValue?: string;
}

export interface SvFileStamp {
  size: number;
  modifiedEpochMs: number;
}

export interface SvReadResponse {
  tree: SvTreeNode;
  stamp: SvFileStamp;
}

export interface SvChange {
  path: string[];
  changeType: "modified" | "added" | "removed";
  oldValue: string | null;
  newValue: string | null;
}

export interface SvDiffPreview {
  changes: SvChange[];
}

// ── SavedVariables Editor v2 types ──────────────────────────────────────
export type WidgetType =
  | "text"
  | "number"
  | "toggle"
  | "slider"
  | "color"
  | "dropdown"
  | "readonly"
  | "group"
  | "raw";

export type WidgetConfidence = "certain" | "inferred" | "ambiguous";
export type NodeContext = "account-wide" | "per-character" | "setting";

export interface WidgetProps {
  min?: number;
  max?: number;
  step?: number;
  options?: string[];
  multiline?: boolean;
}

export interface WidgetOverride {
  widget?: WidgetType;
  props?: Partial<WidgetProps>;
  hidden?: boolean;
  readOnly?: boolean;
  label?: string;
}

export interface SvSchemaOverlay {
  [addonName: string]: {
    [stablePathId: string]: WidgetOverride;
  };
}

export interface EffectiveField {
  nodeId: string;
  key: string;
  label: string;
  widget: WidgetType;
  confidence: WidgetConfidence;
  context: NodeContext;
  props: WidgetProps;
  hidden: boolean;
  readOnly: boolean;
  value: string | number | boolean | null;
  children?: EffectiveField[];
}

// App-level UI state types
export type SortMode = "name" | "author";
export type FilterMode =
  | "all"
  | "addons"
  | "libraries"
  | "outdated"
  | "missing-deps"
  | "favorites"
  | "disabled";

// Predefined tags users can apply to addons
export const PRESET_TAGS = ["favorite", "testing", "broken", "essential", "raid"] as const;
export type ViewMode = "installed" | "discover";
export type DiscoverTab = "search" | "popular" | "categories" | "url";

// ── Pack types (from roster-hub-api Pack Hub) ─────────────────────────────
export interface PackAddonEntry {
  esouiId: number;
  name: string;
  required: boolean;
  note?: string;
}

export type PackType = "addon-pack" | "build-pack" | "roster-pack";

type PackStatus = "draft" | "published";

export interface Pack {
  id: string;
  authorId: string;
  title: string;
  description: string;
  packType: PackType;
  authorName: string;
  isAnonymous: boolean;
  voteCount: number;
  installCount: number;
  userVoted: boolean;
  tags: string[];
  addons: PackAddonEntry[];
  createdAt: string;
  updatedAt: string;
  status: PackStatus;
}

export interface PackPage {
  packs: Pack[];
  page: number;
}

// ── Private sharing types ────────────────────────────────────────────────
export interface ShareCodeResponse {
  code: string;
  expiresAt: string;
  deepLink: string;
}

export interface SharedPack {
  title: string;
  description: string;
  packType: string;
  tags: string[];
  addons: PackAddonEntry[];
  sharedBy: string;
  sharedAt: string;
  expiresAt: string;
}

export interface ScrubContext {
  accounts: string[];
  characters: string[];
  characterIds: string[];
  extraWorlds: string[];
}

export interface ScrubSummary {
  dropCount: number;
  templateCount: number;
  originalBytes: number;
  scrubbedBytes: number;
}

export interface AddonSettings {
  encoding: string;
  lua: string;
  originalBytes: number;
  scrubbedBytes: number;
  /** True exported size after per-character data is stripped. 0 if absent (pre-v2 files). */
  finalBytes: number;
  scrubSummary: ScrubSummary;
}

export interface SvImportResult {
  applied: string[];
  skipped: string[];
  errors: string[];
}

export interface EsoPackFile {
  format: string;
  version: number;
  pack: EsoPackData;
  sharedAt: string;
  sharedBy: string;
  /** v2: per-addon scrubbed SavedVariables settings (keyed by addon folder name) */
  settings?: Record<string, AddonSettings>;
}

interface EsoPackData {
  title: string;
  description: string;
  packType: string;
  tags: string[];
  addons: PackAddonEntry[];
}

// ── Installed pack reference (persisted locally) ─────────────────────────
export interface InstalledPackRef {
  packId: string;
  title: string;
  packType: PackType;
  authorName: string;
  addonCount: number;
  installedAt: string;
}

// ── Safe Migration types ─────────────────────────────────────────────────

export interface PreconditionResult {
  esoRunning: boolean;
  minionRunning: boolean;
  minionFound: boolean;
  addonsPathValid: boolean;
  savedVariablesExists: boolean;
  warnings: string[];
}

export interface SnapshotManifest {
  id: string;
  label: string;
  createdAt: string;
  sourcePaths: string[];
  fileCount: number;
  totalSize: number;
  archiveSha256: string;
}

export interface DryRunAddon {
  folderName: string;
  esouiId: number;
  minionVersion: string;
  status: string;
}

export interface DryRunResult {
  willTrack: DryRunAddon[];
  alreadyTracked: DryRunAddon[];
  missingOnDisk: DryRunAddon[];
  unmanagedOnDisk: string[];
}

export interface SafeMigrationResult {
  imported: number;
  alreadyTracked: number;
  skippedMissing: number;
  addonCount: number;
}

export interface IntegrityResult {
  addonsFolderOk: boolean;
  savedVariablesOk: boolean;
  addonCount: number;
  issues: string[];
}

export interface OpLogEntry {
  operation: string;
  startedAt: string;
  finishedAt: string;
  status: string;
  snapshotId: string | null;
  filesCreated: string[];
  filesModified: string[];
  details: string;
}

// ── Roster pack install types (deep link: kalpa://install-pack/{id}) ─────
export interface RosterPack {
  id: string;
  title: string;
  addons: PackAddonEntry[];
}

// ── Auth types ────────────────────────────────────────────────────────────
export interface AuthUser {
  userId: string;
  userName: string;
}

// ── Protected Edits types ──────────────────────────────────────────────

export interface FileConflict {
  relativePath: string;
  userHash: string;
  upstreamHash: string;
}

export interface ConflictReport {
  sessionId: string;
  folderName: string;
  updateVersion: string;
  safeFiles: string[];
  autoKeptFiles: string[];
  conflicts: FileConflict[];
}

export interface DiffData {
  userContent: string;
  upstreamContent: string;
  isBinary: boolean;
}

export interface FileDecision {
  relativePath: string;
  action: "keep_mine" | "take_update";
}

export interface NoConflictAddon {
  sessionId: string;
  folderName: string;
  updateVersion: string;
  autoKeptFiles: string[];
}

export interface BatchConflictAddon {
  sessionId: string;
  folderName: string;
  updateVersion: string;
  conflicts: FileConflict[];
  autoKeptFiles: string[];
}

export interface BatchConflictResult {
  noConflictAddons: NoConflictAddon[];
  conflictingAddons: BatchConflictAddon[];
  failed: string[];
  errors: Record<string, string>;
}

/// Result of the streaming `update_batch_with_decisions` command: a single IPC
/// call that downloads in parallel, extracts as each download finishes, and
/// writes kalpa.json once. Addons with unresolved ("ask") conflicts come back
/// in `conflicts` for the interactive modal; everything else is already applied.
export interface StreamingBatchResult {
  completed: string[];
  failed: string[];
  errors: Record<string, string>;
  conflicts: BatchConflictAddon[];
  installedDeps: string[];
  failedDeps: string[];
  skippedDeps: string[];
}

export interface WriteAccessStatus {
  /** True when Kalpa cannot write into the AddOns folder. */
  blocked: boolean;
  /** True when the block is a permission denial (most often Controlled Folder Access). */
  permissionDenied: boolean;
  /** Absolute path to the Kalpa executable, to show which app to allow. */
  exePath: string;
}

export interface AddonFileEntry {
  relativePath: string;
  isDirectory: boolean;
  sizeBytes: number;
  status: "stock" | "modified" | "unknown";
  extension: string;
}

export interface AddonFileTree {
  folderName: string;
  files: AddonFileEntry[];
  modifiedCount: number;
}

export interface EditBackupManifest {
  addonFolder: string;
  backedUpAt: string;
  updateFrom: string;
  updateTo: string;
  files: string[];
}
