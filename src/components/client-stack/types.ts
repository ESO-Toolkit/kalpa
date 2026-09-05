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
/* Data contract — the diagnostic report                                     */
/* -------------------------------------------------------------------------- */
/* These mirror `src-tauri/src/client_health.rs`.                              */

/** A DLL found next to `eso64.exe`. `version` is null when the version
 *  resource is missing or unreadable — unknown, not an error. */
export interface DllInfo {
  name: string;
  version: string | null;
}

/**
 * A log line that matched a known **fatal** failure signature.
 *
 * Only fatal matches reach this type. The six ERROR/WARN lines a *working*
 * RenoDX DLSS-NR setup emits on every launch are recognised as benign, counted
 * into `log_benign_suppressed`, and never turned into an excerpt — surfacing
 * them is how a healthy stack got triaged as a broken one.
 */
export interface LogExcerpt {
  /** Source file name, e.g. `ReShade.log`. */
  file: string;
  /** Slug of the log rule that matched. Deliberately **not** a
   *  `HealthFinding["id"]`: no finding is raised from a log line. */
  rule: string;
  line: string;
}

/**
 * Whether the log proves Neural Rendering is actually running.
 *
 * The three states are genuinely different and **must not be collapsed** — the
 * panel used to treat "no evidence" as "fine", which is the bug this union
 * exists to fix.
 *
 * - `running` — the `EvaluateFeature succeeded: evaluation=N` counter was found
 *   **and is climbing**. It advances once per rendered frame, so a rising
 *   sequence cannot be faked by an add-on that merely loaded. This is the only
 *   real proof.
 * - `stalled` — the counter was found but never advanced across the scanned
 *   window. Suspicious, and explicitly **not proof**. A single occurrence lands
 *   here too.
 * - `unknown` — no evaluation line at all. This is **unknown, not broken**: the
 *   log may be absent, the session may predate the add-on, or the signature may
 *   simply have scrolled out of the 400-line tail window. Never render it as
 *   working, and never render it as failing.
 */
export type NeuralRenderingState = "running" | "stalled" | "unknown";

/** Positive evidence, or the documented absence of it, that Neural Rendering
 *  ran during the logged session. */
export interface NeuralRenderingSignal {
  state: NeuralRenderingState;
  /** How many `EvaluateFeature succeeded` lines were seen. Not a frame count:
   *  the scanned window is capped, so this saturates on any real session. */
  samples: number;
  /** First and last counter values parsed, in file order, so the UI can say
   *  *how far* it climbed rather than only that it did. Null when none. */
  first_evaluation: number | null;
  last_evaluation: number | null;
}

export interface ClientHealthReport {
  location: EsoClientLocation;
  /** The ReShade/injector proxy DLL. `d3d11.dll` for a stock install, `dxgi.dll`
   *  for setups needing DXGI-level add-on hooks; which is correct is
   *  setup-dependent and deliberately not asserted. */
  injector: DllInfo | null;
  /** ESO's bundled DLSS super-resolution runtime (2.2.16, unchanged since 2021). */
  dlss: DllInfo | null;
  /** The D3D shader compiler ESO ships — a 2013 build that wins DLL search order. */
  d3dcompiler: DllInfo | null;
  reshade_preset: string | null;
  findings: HealthFinding[];
  /** Fatal log matches only. */
  log_excerpts: LogExcerpt[];
  /** The "everything agrees" claim has to be earned by `state === "running"`
   *  here, **not** by an empty findings list. */
  neural_rendering: NeuralRenderingSignal;
  /** How many lines matched a *benign* signature and were suppressed. Shown so
   *  the panel can say "6 known-harmless lines ignored" rather than silently
   *  discarding them — a suppressed line is still one somebody may go looking
   *  for in the raw log. */
  log_benign_suppressed: number;
}

/* -------------------------------------------------------------------------- */
/* Data contract — the stack                                                 */
/* -------------------------------------------------------------------------- */
/* These mirror `src-tauri/src/client_stack.rs`. A DLSS 5 Neural Rendering     */
/* setup is a pipeline of layers, not three DLLs — see that file's module      */
/* doc for why the cross-layer findings below exist.                          */
/*                                                                            */
/* There are TWO such pipelines and they are mutually exclusive: see           */
/* `ActivePath`. Rendering either one's layers as universally required is how  */
/* the panel came to report "Everything agrees" over a stack that could not    */
/* work, so `active_path` and `slots` have to be read before any row decides   */
/* what an empty slot means.                                                   */

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

/**
 * Whose hand parked a file.
 *
 * Re-enabling a user-parked file is not the same act as re-enabling a
 * Kalpa-parked one. `.kalpa-off` is a name only Kalpa writes, so a file
 * carrying it is one Kalpa moved and can move back. `.off` is the user's own
 * switch: Kalpa did not choose the name and cannot claim to know why it was
 * flipped, so it reports the state and offers nothing.
 */
export type ParkedBy = "kalpa" | "user";

/**
 * A file renamed aside so the stack does not load it.
 *
 * Not a `PreservedOriginal`. A backup suffix marks a displaced *original*; a
 * park suffix marks a **live file that was switched off** — the same bytes that
 * run again the moment the name goes back.
 */
export interface ParkedFile {
  file_name: string;
  restores: string;
  size_bytes: number;
  target_present: boolean;
  /** The suffix actually on disk, verbatim — `.kalpa-off` or `.off`. */
  suffix: string;
  parked_by: ParkedBy;
}

/**
 * Which of the two mutually exclusive Neural Rendering setups is live.
 *
 * - `direct` — `renodx-dlss.addon64` hooks `nvngx_dlssnr.dll` itself. No feed
 *   add-on, no host process, no motion-vector provider, and an **empty
 *   `Techniques=` is correct**.
 * - `feed` — `renodx-dlss5.addon64` + `dlss5-feed.addon64` +
 *   `dlss5-feed-host64.exe`, with `DLSS5_Feed` enabled and a motion-vector
 *   provider ordered above it.
 * - `both` — both add-ons are loaded. ReShade loads both, Kalpa does not guess
 *   which one wins, and **both paths' checks apply**: the feed pipeline is live
 *   here, so its technique order, provider and preset are all still checked. An
 *   earlier three-variant version reported this as `direct` and gated every
 *   feed finding away over a live feed — do not re-collapse it.
 * - `neither` — no Neural Rendering add-on is loaded. Entirely ordinary; a
 *   plain ReShade install is the common case and nothing about it is wrong.
 * - `unknown` — the client folder could not be read. **Not the same as
 *   `neither`**: that one asserts an empty slot is correct, this one says Kalpa
 *   could not look. Every tuning section reads as `unknown` provenance and
 *   nothing is writable, because a module that writes must never guess in the
 *   direction of writing.
 *
 * Computed from which add-on is *loaded*, never from what is on disk: liveness
 * is "a file named exactly like the add-on is present and not in
 * `disabled_addons`", so every rename aside — `.off`, `.kalpa-off`, anything —
 * is inert whether or not Kalpa has heard of the suffix.
 */
export type ActivePath = "direct" | "feed" | "both" | "neither" | "unknown";

/**
 * One panel row, named to match the `Slot` union in `slots.ts` exactly.
 *
 * Deliberately the same vocabulary rather than a parallel one. The need axis is
 * a backend answer — only the backend knows which path is live — but it has to
 * land on a row the frontend already renders, and a mapping table between two
 * spellings is exactly the thing that lets them drift.
 */
export type StackSlot =
  "reshade" | "addons" | "nr" | "sr" | "shaders" | "motion" | "preset" | "tuning";

/**
 * Whether a slot is *wanted* on the live path — the third axis beside present
 * and active.
 *
 * - `required` — the live path needs this; empty here is a real gap.
 * - `not_on_this_path` — correctly absent. Render `reason` affirmatively.
 *   **Never as an empty row and never as an Info-level finding**: an empty
 *   motion-vector slot on the direct path is the right answer, and painting it
 *   as an absence is the bug this axis exists to fix.
 * - `installed_unused` — present, and not used on this path. Never a fault and
 *   never advice to remove it; see `keep_because`.
 * - `unknown` — Kalpa could not read the client folder, so it cannot say
 *   whether this slot is wanted. Only reachable when `active_path` is
 *   `unknown`, and deliberately not `not_on_this_path`: that one *asserts* the
 *   slot is correctly empty, and asserting it from a folder Kalpa could not
 *   read is a guess wearing a verdict's clothes.
 */
export type SlotNeed = "required" | "not_on_this_path" | "installed_unused" | "unknown";

export interface SlotStatus {
  slot: StackSlot;
  need: SlotNeed;
  /** A complete sentence, shown verbatim. */
  reason: string;
  /**
   * Why to keep something installed but unused. Kalpa refetches none of this
   * stack's optional pieces — iMMERSE LaunchPad is link-only by licence, the
   * `renodx-dlss5` / `dlss5-feed` add-ons come from a Discord with no stable
   * URL — so a user's existing copy is their only fallback, and "you could
   * delete this" would be advice Kalpa cannot undo. Null when nothing needs
   * saying.
   */
  keep_because: string | null;
}

/**
 * Whether an INI section's contents are configuration **in force**, or
 * leftovers from a path that is no longer running.
 *
 * `[RenoDX.DLSS5]` is written by `renodx-dlss5.addon64`, the **feed** path's
 * add-on. On a direct-path install that add-on is parked, so the section is a
 * `fossil`: real values, saved by a real add-on, describing a configuration
 * that is not running. Presented as live tuning it misled the user once
 * already — a dead `NeuralUplift=0` read as this install's current setting.
 * `unknown` means liveness could not be determined at all (see
 * `ActivePath["unknown"]`), and is never writable.
 *
 * One union for both panels: the stack's `tuning_owner` and each tuning
 * section's `provenance` are the same verdict, and two spellings of it is how
 * they came to disagree.
 *
 * **Provenance is not presence.** There is deliberately no `absent` member —
 * "the section is not in the file" is carried separately, by
 * `ClientStack["tuning_section"]` being null and by `TuningSection["present"]`,
 * so the panel can state both facts at once ("no section, *and* the add-on that
 * would write one is parked").
 */
export type TuningProvenance = "live" | "fossil" | "unknown";

export interface Technique {
  name: string;
  source: string;
  source_present: boolean;
}

/** Which effect supplies the motion vectors DLSS5_Feed consumes. */
export interface MvProvider {
  kind: "shared_texture" | "launchpad" | "vort" | "lumenite_kernel" | "lumenite_quant_motion";
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
  /**
   * Files **Kalpa** parked — `.kalpa-off` and nothing else.
   *
   * `powerState()` in `slots.ts` reads "on" from this being empty and the
   * toggle plans an unpark for every entry, and both are statements about
   * Kalpa's own work. A file the user renamed by hand must never join this
   * list: it would report a working install as partly switched off.
   */
  parked: ParkedFile[];
  /** Files the **user** switched off themselves (`.off`). Read-only knowledge:
   *  Kalpa still parks only as `.kalpa-off` and removes only that suffix. */
  user_parked: ParkedFile[];
  /** True when **Kalpa** parked the injector — ESO is back to stock. Not fed by
   *  `user_parked`, because the copy attached to it claims work Kalpa did. */
  is_disabled: boolean;
  shaders: ShaderTree;
  preset: PresetInfo | null;
  tuning: TuningValue[];
  /** The ini section `tuning` came from, so no row hardcodes a section name
   *  that belongs to only one of the two paths. Null when there is none —
   *  this field, not a provenance member, is where absence lives. */
  tuning_section: string | null;
  /** Whether `[RenoDX.DLSS5]` is in force, answered whether or not the section
   *  is present: it is a fact about which add-on is loaded. */
  tuning_owner: TuningProvenance;
  disabled_addons: string[];
  /** `[ADDON] LoadFromDllMain` — the add-ons ReShade loads early enough for
   *  their hooks to land. See `stack-addon-not-in-dllmain`. */
  load_from_dll_main: string[];
  /** Read this before deciding what an empty slot means. */
  active_path: ActivePath;
  /** Always all eight, in slot order, so a row never falls back to "unknown". */
  slots: SlotStatus[];
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
  /** True when part of the stack is parked. Adoption is refused in that state:
   *  what is in the folder is the game's own files, not the stack. */
  stack_switched_off: boolean;
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

/* -------------------------------------------------------------------------- */
/* Data contract — switching the stack off and on                            */
/* -------------------------------------------------------------------------- */
/* Mirrors `src-tauri/src/client_toggle.rs`. Disable puts ESO back to stock:   */
/* the injector is parked, the two files the game loads itself get the user's  */
/* own originals put live, and everything else is left where it is.            */

export type ToggleAction = "disable" | "enable";

export type ToggleOpKind =
  "park" | "restore_original" | "unpark" | "remove_restored" | "leave_in_place";

export interface PlannedOp {
  kind: ToggleOpKind;
  file_name: string;
  summary: string;
  detail: string;
  /** The other file a two-file step involves. */
  partner: string | null;
}

export interface TogglePlan {
  client_dir: string;
  action: ToggleAction;
  is_disabled: boolean;
  /** In the order they will run. This list is the confirmation. */
  operations: PlannedOp[];
  /** Non-empty means the confirm button stays disabled. Shown verbatim. */
  blockers: string[];
}

export interface FileOpOutcome {
  applied: string[];
  skipped: string[];
  preserved: string[];
}

/* -------------------------------------------------------------------------- */
/* Data contract — tuning                                                    */
/* -------------------------------------------------------------------------- */
/* Mirrors `src-tauri/src/client_tuning.rs`. Every label and enum value comes  */
/* from the add-on's own binary; none of it is invented, and the float ranges  */
/* are display hints rather than limits, because the real limits are unknown.  */

export type TuningControl = "toggle" | "choice" | "float" | "key_code";

export type TuningGroup = "neural_rendering" | "detail" | "color" | "keys" | "advanced";

export interface ChoiceOption {
  value: number;
  label: string;
}

export interface TuningField {
  key: string;
  label: string;
  control: TuningControl;
  group: TuningGroup;
  choices: ChoiceOption[];
  decimals: number;
  help: string;
  /** Verbatim from the file. Null when the key is absent. Never clamped. */
  current: string | null;
  /** Display range only — the numeric box beside the slider is authoritative. */
  slider_min: number | null;
  slider_max: number | null;
}

/** Which of the two mutually exclusive RenoDX integrations a section belongs
 *  to. `direct` writes `[RENODX-DLSS]` and `[RENODX-DLSS-preset*]`; `feed`
 *  writes `[RenoDX.DLSS5]`. */
export type RenoDxPath = "direct" | "feed";

/** A raw `key=value` pair, exactly as `ReShade.ini` has it. */
export interface TuningEntry {
  key: string;
  value: string;
}

/**
 * One section of `ReShade.ini`.
 *
 * A fossil is never hidden and never deleted — the user may well switch paths
 * back, and silently dropping their saved settings would be the worse failure.
 * It is **labelled**, and it is not writable while it is a fossil.
 */
export interface TuningSection {
  /** The name as it appears in the file when present, else the canonical
   *  spelling. */
  section: string;
  path: RenoDxPath;
  /** The add-on file that writes this section. */
  owner: string;
  /** Whether `ReShade.ini` has this section at all. False means the add-on has
   *  never run here — say so rather than offering to write one from nothing. */
  present: boolean;
  provenance: TuningProvenance;
  /** True only when Kalpa has a verified field table for the section *and* its
   *  owning add-on is live *and* the section already exists. */
  writable: boolean;
  /** Why it is not writable, in the panel's own words. Empty when it is, and
   *  never empty when `writable` is false. Shown verbatim. */
  read_only_reason: string;
  /** Typed, verified controls. Empty for every section but `[RenoDX.DLSS5]`:
   *  the direct path's keys are closed-source and read-only by design, and
   *  inventing labels for them is how a working install gets corrupted. */
  fields: TuningField[];
  /** Every key no field spec describes, verbatim and in file order. For the
   *  direct path's sections that is all of them. */
  entries: TuningEntry[];
}

export interface TuningForm {
  client_dir: string;
  /** Decides every section's provenance. Read it before rendering any value as
   *  current. */
  active_path: ActivePath;
  /** Plain-English observations behind `active_path`, naming the files that
   *  were and were not found. Shown rather than asking the user to take the
   *  verdict on trust. */
  path_evidence: string[];
  sections: TuningSection[];
  /** Shown beside the apply button. ReShade reads these values when the add-on
   *  initialises, so a change lands at the *next* launch — the status after an
   *  apply is "Applies at next launch", never "Saved". */
  apply_note: string;
}

export interface TuningEdit {
  key: string;
  value: string;
}

export interface TuningApplyOutcome {
  changed: string[];
  backup_id: string | null;
  /** The same timing note. "Changed 3 settings" on its own reads as "done
   *  now", which is exactly what it is not. */
  note: string;
}

/* -------------------------------------------------------------------------- */
/* Data contract — runtime drift                                             */
/* -------------------------------------------------------------------------- */
/* Mirrors `src-tauri/src/client_runtime.rs`. Kalpa downloads nothing: the     */
/* user's own kept copy is the only thing a re-apply can restore from.         */

export type DriftState =
  | "unchanged"
  | "drifted_recoverable"
  | "drifted_unrecoverable"
  | "missing"
  | "parked"
  /** Changed, but ESO does not ship this file, so no update can have reverted
   *  it — the user replaced it. There is nothing to put back. */
  | "changed_not_by_update";

export interface RuntimeStatus {
  relative_path: string;
  role: StackRole;
  state: DriftState;
  current_version: string | null;
  kept_version: string | null;
  kept_backup_id: string | null;
  size_bytes: number;
  displaced_in_place: string | null;
}

export interface RuntimeReport {
  client_dir: string;
  runtimes: RuntimeStatus[];
  recoverable: string[];
  unrecoverable: string[];
}

/** What dropping an install's records did. */
export interface ForgetOutcome {
  forgotten: string[];
  /** Kept copies that stopped being referenced, and so become candidates for
   *  cleanup. Non-zero means "records and nothing else" is not the whole story. */
  released_copies: number;
}

export interface ReapplyOutcome {
  restored: string[];
  skipped: string[];
}

/* -------------------------------------------------------------------------- */
/* Data contract — presets                                                   */
/* -------------------------------------------------------------------------- */
/* Mirrors `src-tauri/src/client_preset.rs`.                                   */

export interface PresetChoice {
  relative_path: string;
  preset_path: string;
  is_active: boolean;
  technique_count: number;
}

export interface OrderFix {
  provider_technique: string;
  feed_technique: string;
  before: string;
  after: string;
  summary: string;
}

export interface PresetOptions {
  client_dir: string;
  active: string | null;
  choices: PresetChoice[];
  /** Null when the order is right, or when reordering could not fix it. */
  fix: OrderFix | null;
}

export interface PresetChangeOutcome {
  relative_path: string;
  backup_id: string | null;
  summary: string;
}

/* -------------------------------------------------------------------------- */
/* Data contract — the shader-pack library                                   */
/* -------------------------------------------------------------------------- */
/* Mirrors `src-tauri/src/client_shaders.rs`. Kalpa fetches every pack from    */
/* its author's own repository and mirrors nothing, so whether a pack can be   */
/* installed at all is a licensing fact rather than a UI state — which is why  */
/* `PackSource` is part of the data and not a rendering decision.              */

export type ArchiveLayout = "shaders_and_textures" | "flat_root";

/** Kalpa may fetch this pack from the author's own repository. */
export interface PackSourceFetchable {
  kind: "fetchable";
  owner: string;
  repo: string;
  branch: string;
}

/** Named and linked, never downloaded. `reason` is shown verbatim: "Kalpa
 *  can't install this" without a why reads as a missing feature rather than a
 *  deliberate limit. */
export interface PackSourceLinkOnly {
  kind: "link_only";
  url: string;
  reason: string;
}

export type PackSource = PackSourceFetchable | PackSourceLinkOnly;

/** A catalogue entry flattened together with whether it is actually here —
 *  the Rust side uses `#[serde(flatten)]`, so these are one object. */
export interface PackStatus {
  id: string;
  name: string;
  author: string;
  summary: string;
  licence: string;
  source: PackSource;
  layout: ArchiveLayout;
  /** Technique identifiers, never `ui_label`s — a preset stores identifiers. */
  techniques: string[];
  markers: string[];
  installed: boolean;
  /** The marker files actually found, so the panel can say what it matched on
   *  rather than asserting "installed" with nothing to show for it. */
  found: string[];
}

export interface ShaderLibrary {
  client_dir: string;
  shader_tree_present: boolean;
  packs: PackStatus[];
}

export interface PackInstallOutcome {
  pack_id: string;
  pack_name: string;
  /** The commit the archive was pinned to. These repositories publish no tags,
   *  so this is the only thing identifying which bytes were installed. */
  commit: string;
  files: string[];
}
