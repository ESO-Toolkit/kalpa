import {
  Archive,
  CloudUpload,
  FileSliders,
  Keyboard,
  Layers,
  PackageIcon,
  Shield,
  ShieldCheck,
  Sparkles,
  Stethoscope,
  Users,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

/**
 * The app-shell feature registry.
 *
 * Kalpa's shell is modular at the SURFACE level only: this file is the single
 * place that says what a feature is called, which icon it wears, and where it
 * may appear (pinned in the header toolbar, or listed in the Settings > Tools
 * tab). It is deliberately NOT a feature-flag system — no
 * entry here gates a Tauri command, a Cargo feature, or a build variant. Every
 * feature is always present and reachable; the registry only decides where the
 * user finds it.
 *
 * Add new shell surfaces HERE rather than by threading another `onOpenX` prop
 * through App -> AppHeader -> Settings.
 */
export type FeatureId =
  | "packs"
  | "profiles"
  | "saved-variables"
  | "log-upload"
  | "backups"
  | "characters"
  | "api-compat"
  | "safety-center"
  | "client-health"
  | "migration-wizard"
  | "shortcuts";

/**
 * Every dialog the shell can host. `settings` and `support` are always-present
 * shell surfaces with their own dedicated header buttons, so they are not
 * registry features — but they are still dialog ids.
 */
export type DialogId = FeatureId | "settings" | "support";
export type ActiveDialog = DialogId | null;

/** Inputs a feature's `visibleWhen` / `pinnedWhen` predicates may consult. */
export interface FeatureContext {
  /** A Minion install was detected on this machine. */
  minionDetected: boolean;
  /**
   * The selected ESO client has a recognisable graphics-mod stack.
   *
   * Sourced once at startup like [`FeatureContext.minionDetected`], never
   * polled — an "on open plus an explicit Refresh" feature must not grow a
   * background probe just to decide where its own button goes.
   */
  graphicsStackDetected: boolean;
}

/**
 * Which block of the Settings > Tools tab a `placement: "tools"` feature renders
 * in. The tab interleaves two static entries ("Check for App Updates" and the
 * feedback group) between the two blocks, so the registry cannot be mapped as
 * one contiguous list.
 *
 * `"appearance"` means the feature is reachable from the Appearance tab instead
 * (currently only the keyboard-shortcuts link) and must NOT be rendered as a
 * ToolItem.
 */
export type ToolsGroup = "primary" | "secondary" | "appearance";

export interface FeatureDef {
  id: FeatureId;
  /** Lifted verbatim from the shipped UI — changing these changes e2e selectors. */
  label: string;
  /**
   * Title for the dialog's Suspense loading fallback, when it differs from the
   * menu/row `label`. Two surfaces, two strings: the Tools row says "Backup &
   * Restore" while the dialog chrome says "Backups". Defaults to `label`.
   */
  dialogTitle?: string;
  description: string;
  icon: LucideIcon;
  placement: "toolbar" | "tools";
  pinnableToToolbar: boolean;
  /** Header button `aria-label`. Pinned in `e2e/settings-dialog.spec.ts` siblings. */
  ariaLabel?: string;
  /** Header button tooltip copy. */
  tooltip?: string;
  toolsGroup?: ToolsGroup;
  /** Renders the gold `ToolItem` accent / gold menu row. */
  accent?: "gold";
  shortcut?: { key: string; mod: boolean; spoken: string };
  visibleWhen?: (ctx: FeatureContext) => boolean;
  /**
   * Whether a pinnable feature actually earns its toolbar slot right now.
   *
   * `pinnableToToolbar` says a feature *may* live in the header; this says it
   * is worth the space *today*. The distinction exists because the toolbar is
   * the one surface with a hard budget — every button there is permanent prime
   * real estate — and some features only matter to the subset of users who have
   * the thing they manage. The graphics stack is the case in point: most ESO
   * players run no ReShade at all, and `client_stack.rs` treats that as the
   * common case, so pinning it unconditionally would spend a header slot to
   * open an empty state.
   *
   * Falling out of the toolbar does NOT make a feature unreachable: it lands in
   * the Settings > Tools catalog, which lists unpinned pinnables first. That is
   * what makes this safe to be false.
   *
   * Deliberately separate from `visibleWhen`, which decides whether a feature is
   * offered at all. Conflating them would hide the panel from someone whose
   * stack Kalpa merely failed to detect — exactly the "absence read as a
   * verdict" mistake the stack panel itself was built to stop making.
   */
  pinnedWhen?: (ctx: FeatureContext) => boolean;
}

export const FEATURES: readonly FeatureDef[] = [
  {
    id: "packs",
    label: "Pack Hub",
    description: "Browse and share community addon collections",
    icon: PackageIcon,
    placement: "toolbar",
    pinnableToToolbar: true,
    ariaLabel: "Addon Packs",
    tooltip: "Addon Packs",
  },
  {
    id: "profiles",
    label: "Profiles",
    description: "Switch between saved sets of enabled addons",
    icon: Layers,
    placement: "toolbar",
    pinnableToToolbar: true,
    ariaLabel: "Profiles",
    tooltip: "Addon Profiles",
  },
  {
    id: "saved-variables",
    label: "Saved Variables",
    description: "Inspect, scrub, and back up SavedVariables files",
    icon: FileSliders,
    placement: "toolbar",
    pinnableToToolbar: true,
    ariaLabel: "Saved Vars",
    tooltip: "SavedVariables Manager",
  },
  {
    id: "log-upload",
    label: "Log Uploader",
    description: "Split and upload Encounter.log to ESO Logs",
    icon: CloudUpload,
    placement: "toolbar",
    pinnableToToolbar: true,
    ariaLabel: "Upload to ESO Logs",
    tooltip: "Upload to ESO Logs",
  },
  {
    id: "backups",
    label: "Backup & Restore",
    dialogTitle: "Backups",
    description: "Save and recover your addon settings",
    icon: Archive,
    placement: "tools",
    pinnableToToolbar: false,
    toolsGroup: "primary",
  },
  {
    id: "characters",
    label: "Characters",
    description: "View and manage your ESO characters",
    icon: Users,
    placement: "tools",
    pinnableToToolbar: false,
    toolsGroup: "primary",
  },
  {
    id: "api-compat",
    label: "API Compatibility",
    description: "Check addons against current API version",
    icon: ShieldCheck,
    placement: "tools",
    pinnableToToolbar: false,
    toolsGroup: "primary",
  },
  {
    id: "migration-wizard",
    label: "Minion Migration",
    dialogTitle: "Migration",
    description: "Import tracking data from Minion with backup and preview",
    icon: Sparkles,
    placement: "tools",
    pinnableToToolbar: false,
    toolsGroup: "secondary",
    accent: "gold",
    visibleWhen: (ctx) => ctx.minionDetected,
  },
  {
    id: "safety-center",
    label: "Safety Center",
    description: "Snapshots, integrity checks, and operation log",
    icon: Shield,
    placement: "tools",
    pinnableToToolbar: false,
    toolsGroup: "secondary",
  },
  {
    // Named for what the user is looking at, not for what the code checks.
    // The registry called this "Client Health" while the dialog it opens is
    // titled "Graphics stack", so someone hunting for their ReShade setup had
    // to already know the two were the same thing — the likeliest reason the
    // panel felt buried. `dialogTitle` is unset because the two now agree.
    id: "client-health",
    label: "Graphics stack",
    description: "Manage the ReShade and DLSS setup in your ESO game folder",
    icon: Stethoscope,
    // Pinnable, but only pinned once there is a stack to manage — see
    // `pinnedWhen` on FeatureDef for why that is a toolbar-budget decision
    // rather than a visibility one.
    placement: "toolbar",
    pinnableToToolbar: true,
    ariaLabel: "Graphics stack",
    tooltip: "Graphics stack",
    pinnedWhen: (ctx) => ctx.graphicsStackDetected,
  },
  {
    id: "shortcuts",
    label: "Keyboard Shortcuts",
    description: "See every keyboard shortcut Kalpa understands",
    icon: Keyboard,
    placement: "tools",
    pinnableToToolbar: false,
    // Reachable from Settings > Appearance as an inline link, not a ToolItem.
    toolsGroup: "appearance",
    shortcut: { key: "?", mod: false, spoken: "Question mark" },
  },
] as const;

/** Loading-fallback / dialog titles for every dialog the shell can host. */
export const DIALOG_LABELS: Record<DialogId, string> = {
  ...(Object.fromEntries(FEATURES.map((f) => [f.id, f.dialogTitle ?? f.label])) as Record<
    FeatureId,
    string
  >),
  settings: "Settings",
  support: "Help",
};

export function findFeature(id: FeatureId): FeatureDef | undefined {
  return FEATURES.find((f) => f.id === id);
}

/**
 * The pinned header buttons, in registry order.
 *
 * Pure: same inputs -> same output, no reads of settings or DOM. `hidden` is the
 * persisted `toolbarHidden` preference; unknown ids in it are ignored, so a
 * preference written by a newer build cannot break an older one.
 *
 * Two different reasons a pinnable feature is absent, and they compose: the user
 * unpinned it (`hidden`), or it has not earned the slot right now
 * (`pinnedWhen`). Either way it falls through to the Settings > Tools catalog
 * rather than out of the app — `toolsMenuFeatures` derives its list from this
 * one, so the two surfaces cannot disagree about what is pinned.
 */
export function visibleToolbar(
  features: readonly FeatureDef[],
  hidden: readonly FeatureId[],
  ctx: FeatureContext
): FeatureDef[] {
  const hiddenSet = new Set<string>(hidden);
  return features.filter(
    (f) => f.pinnableToToolbar && !hiddenSet.has(f.id) && (f.pinnedWhen?.(ctx) ?? true)
  );
}

/**
 * Everything NOT pinned to the toolbar, filtered by each entry's `visibleWhen`.
 *
 * This is the catalog rendered by the Settings > Tools tab: unpinning a feature
 * moves it here rather than making it unreachable. There is deliberately no
 * header overflow menu — a customisable toolbar removes TO the catalog, it does
 * not sprout a second permanent control.
 */
export function toolsMenuFeatures(
  features: readonly FeatureDef[],
  hidden: readonly FeatureId[],
  ctx: FeatureContext
): FeatureDef[] {
  // Derived from `visibleToolbar` rather than re-deriving "is it pinned?" here,
  // so a feature held back by `pinnedWhen` lands in this catalog automatically.
  // Two copies of that rule would be two chances for the toolbar and this list
  // to disagree about the same feature — which is how they drifted apart before.
  const pinned = new Set(visibleToolbar(features, hidden, ctx).map((f) => f.id));
  return features.filter((f) => !pinned.has(f.id) && (f.visibleWhen?.(ctx) ?? true));
}

/**
 * Narrow a persisted `toolbarHidden` value into known ids.
 *
 * `settings.json` is user-editable and survives downgrades, so this must
 * tolerate anything: a non-array, nulls, numbers, or ids written by a NEWER
 * build that this one has never heard of. Unknown entries are dropped rather
 * than thrown on, so a preference from a newer Kalpa cannot brick an older
 * one's toolbar — it just falls back to showing those buttons.
 */
export function sanitizeHiddenIds(value: unknown): FeatureId[] {
  if (!Array.isArray(value)) return [];
  const known = new Set<string>(FEATURES.map((f) => f.id));
  const seen = new Set<string>();
  const out: FeatureId[] = [];
  for (const entry of value) {
    if (typeof entry !== "string" || !known.has(entry) || seen.has(entry)) continue;
    seen.add(entry);
    out.push(entry as FeatureId);
  }
  return out;
}
