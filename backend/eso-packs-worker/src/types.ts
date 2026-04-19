// ── Pack addon entry ────────────────────────────────────────────────
export interface PackAddonEntry {
  esouiId: number;
  name: string;
  required: boolean;
  defaultEnabled?: boolean;
  note?: string;
}

// ── Pack types ────────────────────────────────────────────────────────
export type PackType = "addon-pack" | "build-pack" | "roster-pack";
export type PackStatus = "draft" | "published";

// ── Full pack (snake_case to match Rust HubPack) ─────────────────────
export interface Pack {
  id: string;
  title: string;
  description: string;
  pack_type: PackType;
  author_id: string;
  author_name: string;
  is_anonymous: boolean;
  addons: PackAddonEntry[];
  tags: string[];
  vote_count: number;
  install_count: number;
  created_at: string;
  updated_at: string;
  status: PackStatus;
}

// ── Index (stores full packs for list queries) ────────────────────────
export interface PackIndex {
  packs: Pack[];
}

// ── Vote tracking ─────────────────────────────────────────────────────
export interface VoteRecord {
  userId: string;
  packId: string;
  votedAt: string;
}

export interface VoteResponse {
  voted: boolean;
  voteCount: number;
}

// ── Validation ────────────────────────────────────────────────────────
export interface ValidationError {
  field: string;
  message: string;
}

// ── Share types ───────────────────────────────────────────────────────
export interface SharePackData {
  title: string;
  description: string;
  packType: PackType;
  tags: string[];
  addons: PackAddonEntry[];
}

export interface ShareRecord {
  code: string;
  pack: SharePackData;
  createdBy: string;
  createdByName: string;
  createdAt: string;
  expiresAt: string;
}

export interface ShareCodeResponse {
  code: string;
  expiresAt: string;
  deepLink: string;
}

// ── Sync data sent to roster-hub-api via Service Binding RPC ─────────
export interface SyncPackData {
  id: string;
  title: string;
  description: string;
  pack_type: string;
  author_id: string;
  author_name: string;
  is_anonymous: boolean;
  addons: { esouiId: number; name: string; required?: boolean; note?: string }[];
  tags: string[];
}

export interface PackSyncBinding {
  syncPack(data: SyncPackData): Promise<void>;
  removePack(id: string): Promise<void>;
  reconcile(published: SyncPackData[]): Promise<{ synced: number; removed: number }>;
}

// ── Env bindings ──────────────────────────────────────────────────────
export interface Env {
  ESO_PACKS: KVNamespace;
  ADMIN_API_KEY: string;
  ALLOW_SEED?: string;
  /** Service Binding RPC to roster-hub-api's PackSyncService entrypoint */
  ROSTER_HUB?: PackSyncBinding;
}
