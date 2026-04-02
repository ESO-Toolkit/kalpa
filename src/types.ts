export interface Dependency {
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
  esouiId: number | null;
  tags: string[];
  esouiLastUpdate: number;
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

export interface ImportResult {
  installed: string[];
  failed: string[];
  skipped: string[];
}

export interface EsouiSearchResult {
  id: number;
  title: string;
  author: string;
  category: string;
  downloads: string;
  updated: string;
}

export interface EsouiAddonDetail {
  id: number;
  title: string;
  version: string;
  author: string;
  description: string;
  compatibility: string;
  fileSize: string;
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

export interface BackupInfo {
  name: string;
  createdAt: string;
  fileCount: number;
  totalSize: number;
}

export interface AddonProfile {
  name: string;
  enabledAddons: string[];
  createdAt: string;
}

export interface CharacterInfo {
  server: string;
  name: string;
}

export interface MinionMigrationResult {
  found: boolean;
  addonCount: number;
  imported: number;
  alreadyTracked: number;
}

// ── Addon folder detection types ─────────────────────────────────────────
export interface DetectedCandidate {
  path: string;
  serverEnv: string;
  addonCount: number;
  isOnedrive: boolean;
}

export interface AddonsDetectionResult {
  primary: string | null;
  candidates: DetectedCandidate[];
  warnings: string[];
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
}

export interface SvFileStamp {
  size: number;
  modifiedEpochMs: number;
}

export interface SvReadResponse {
  tree: SvTreeNode;
  stamp: SvFileStamp;
}

export interface SvDiffPreview {
  original: string;
  serialized: string;
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
export type FilterMode = "all" | "addons" | "libraries" | "outdated" | "missing-deps" | "favorites";

// Predefined tags users can apply to addons
export const PRESET_TAGS = ["favorite", "testing", "broken", "essential", "raid"] as const;
export type PresetTag = (typeof PRESET_TAGS)[number];
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

export interface Pack {
  id: string;
  authorId: string;
  title: string;
  description: string;
  packType: PackType;
  authorName: string;
  isAnonymous: boolean;
  voteCount: number;
  userVoted: boolean;
  tags: string[];
  addons: PackAddonEntry[];
  createdAt: string;
  updatedAt: string;
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

export interface EsoPackFile {
  format: string;
  version: number;
  pack: EsoPackData;
  sharedAt: string;
  sharedBy: string;
}

export interface EsoPackData {
  title: string;
  description: string;
  packType: string;
  tags: string[];
  addons: PackAddonEntry[];
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
