/**
 * Data contracts for the ESO client-stack panel.
 *
 * Every type here mirrors a Rust struct in `src-tauri/src/client_*.rs`, field
 * for field and name for name — the backend serialises snake_case and nothing
 * renames it on the way through, so a mismatch here is a mismatch with the
 * wire format rather than a style choice.
 *
 * They live in their own module so the panel components under this folder can
 * share them without importing from `client-health.tsx`, which imports those
 * components in turn.
 */

/* -------------------------------------------------------------------------- */
/* Data contract — install discovery                                         */
/* -------------------------------------------------------------------------- */
/* These mirror the Rust structs behind `detect_eso_clients` and                */
/* `validate_eso_client`.                                                      */

export type ClientSource = "steam" | "zos_registry" | "proton" | "manual";

export interface EsoClientLocation {
  client_dir: string;
  exe_path: string;
  source: ClientSource;
}

export type HealthLevel = "ok" | "info" | "warning" | "danger";

export interface HealthFinding {
  id: string;
  level: HealthLevel;
  title: string;
  detail: string;
  guide_url: string | null;
}

export interface ClientHealthPanelProps {
  open: boolean;
  onClose: () => void;
}

/* -------------------------------------------------------------------------- */
/* Data contract — the stack                                                 */
/* -------------------------------------------------------------------------- */
/* These mirror `src-tauri/src/client_stack.rs`. A DLSS 5 Neural Rendering     */
/* setup is a pipeline of layers, not three DLLs — see that file's module      */
/* doc for why the cross-layer findings below exist.                          */

export type StackRole =
  | "injector"
  | "neural_rendering"
  | "super_sampling"
  | "frame_generation"
  | "shader_compiler"
  | "addon"
  | "companion";

export interface StackItem {
  role: StackRole;
  file_name: string;
  display_name: string | null;
  version: string | null;
  company: string | null;
  description: string | null;
  size_bytes: number;
}

export interface PreservedOriginal {
  file_name: string;
  backs_up: string | null;
  version: string | null;
  size_bytes: number;
}

/** A file Kalpa parked (`.kalpa-off`) so the stack does not load. */
export interface ParkedFile {
  file_name: string;
  restores: string;
  size_bytes: number;
  target_present: boolean;
}

export interface Technique {
  name: string;
  source: string;
  source_present: boolean;
}

/** Which effect supplies the motion vectors DLSS5_Feed consumes. */
export interface MvProvider {
  kind: "launchpad" | "shared_texture";
  /** The enabled technique producing them, or null when nothing does. */
  technique: string | null;
}

export interface PresetInfo {
  path: string;
  exists: boolean;
  techniques: Technique[];
  available: string[];
  /** Null when the preset does not enable the feed, so the question is moot. */
  mv_provider: MvProvider | null;
}

export interface TuningValue {
  key: string;
  value: string;
}

export interface ShaderTree {
  present: boolean;
  effect_count: number;
  texture_count: number;
  effect_search_paths: string | null;
}

export interface ClientStack {
  client_dir: string;
  items: StackItem[];
  preserved_originals: PreservedOriginal[];
  parked: ParkedFile[];
  /** True when the injector is parked — ESO is back to stock. */
  is_disabled: boolean;
  shaders: ShaderTree;
  preset: PresetInfo | null;
  tuning: TuningValue[];
  disabled_addons: string[];
  is_empty: boolean;
  findings: HealthFinding[];
}

/* -------------------------------------------------------------------------- */
/* Data contract — adoption                                                  */
/* -------------------------------------------------------------------------- */
/* These mirror `src-tauri/src/client_adopt.rs`.                               */

export interface AdoptionEntry {
  relative_path: string;
  kind: ManagedKind;
  role: StackRole;
  display_name: string | null;
  version: string | null;
  size_bytes: number;
  displaced_in_place: string | null;
  copyable: boolean;
}

export interface AdoptionPlan {
  client_dir: string;
  entries: AdoptionEntry[];
  copy_bytes: number;
  already_managed: boolean;
  is_empty: boolean;
}

export interface AdoptionOutcome {
  recorded: string[];
  copied: string[];
  skipped: string[];
}

/* -------------------------------------------------------------------------- */
/* Data contract — Kalpa's own records                                       */
/* -------------------------------------------------------------------------- */
/* These mirror `list_managed_client_files`, `uninstall_managed_client_files`  */
/* and `emergency_remove_injector` in `src-tauri/src/client_uninstall.rs`.     */

export type ManagedFileState = "present" | "modified" | "missing" | "parked";

export type ManagedKind =
  | "re_shade_core"
  | "re_shade_config"
  | "shader"
  | "preset"
  | "addon"
  | "nvidia_runtime"
  | "shader_compiler";

export interface ManagedFileStatus {
  relative_path: string;
  kind: ManagedKind;
  placed_at: string;
  state: ManagedFileState;
  restores_backup: boolean;
}

export interface OrphanInjector {
  file_name: string;
  product_name: string;
  version: string | null;
}

export interface ManagedInventory {
  client_dir: string;
  files: ManagedFileStatus[];
  orphan_injectors: OrphanInjector[];
}

export interface UninstallOutcome {
  removed: string[];
  skipped: string[];
}

export interface EmergencyRemoval {
  file_name: string;
  quarantine_path: string;
}
