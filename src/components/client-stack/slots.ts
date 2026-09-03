/**
 * The slot model: what the graphics-stack panel is a list of.
 *
 * The panel used to be a *pipeline diagram* — eight layers in load order, each
 * reporting a status, with cross-layer findings drawn on the connectors between
 * them. That is the right model for diagnosis and it is the model
 * `client_stack.rs` actually computes, but it is the wrong front door: the user
 * asked to "manage and swap between all the possible options", and a diagram
 * answers none of the questions that come with. So the same eight rows are
 * re-read here as **slots** — places a choice can be made — and each one owns
 * the findings that concern it, the thing currently filling it, and the options
 * it could be filled with instead.
 *
 * Nothing about the backend model changed. `Slot` is deliberately a renaming of
 * the old `Stage` union with the same members in the same load order, so the
 * pipeline reading is still available to anyone who wants it; it is just no
 * longer what the screen leads with.
 *
 * Three rules hold this file together:
 *
 * 1. **Every finding `client_stack::build_findings` can emit has a home here.**
 *    A finding with no slot renders nowhere at all, which has happened once
 *    already — `stack-mv-provider-missing` was missing from both of the old
 *    lookup tables and so never appeared in the panel, despite being the
 *    "DLSS is silently upscaling a still image" diagnosis. `FINDING_SLOT` is
 *    total over the known ids and there is a test asserting it.
 * 2. **Copy states what the user will notice, in their terms.** The Rust
 *    `detail` explains the configuration; `FINDING_IMPACT` says what it looks
 *    like from the player's chair. Both are shown — the impact first, because
 *    it is what makes someone care.
 * 3. **A slot never implies a choice it cannot offer.** `SlotSource` is the
 *    honest column: whether Kalpa can fetch a thing at all is a licensing and
 *    supply fact, not a UI state, and it differs per slot. Kalpa hosts and
 *    mirrors nothing, and never downloads an NVIDIA binary.
 */

import { AlertCircleIcon, AlertTriangleIcon, ShieldAlertIcon, ShieldCheckIcon } from "lucide-react";

import type { ClientStack, HealthFinding, HealthLevel, StackRole } from "./types";

/* -------------------------------------------------------------------------- */
/* Level presentation                                                         */
/* -------------------------------------------------------------------------- */

/**
 * Every level pairs its colour with an icon **and** a word.
 *
 * Colour is never the only signal: three shipped themes are light and two are
 * high-contrast, and the `status-*` tokens are reseeded per theme rather than
 * being fixed palette values. A row that says "problem" only by being red says
 * nothing on half the themes Kalpa ships.
 *
 * Borders come from the `structure-*`/`status-*` ladders for the same reason —
 * a literal `border-white/[0.06]` is invisible the moment `--structure-rgb`
 * flips to black.
 */
export const LEVEL_META: Record<
  HealthLevel,
  {
    label: string;
    Icon: typeof ShieldCheckIcon;
    text: string;
    border: string;
    tint: string;
    line: string;
  }
> = {
  ok: {
    label: "OK",
    Icon: ShieldCheckIcon,
    text: "text-status-success",
    border: "border-status-success/20 border-l-status-success",
    tint: "bg-status-success/[0.04]",
    line: "border-status-success",
  },
  info: {
    label: "Info",
    Icon: AlertCircleIcon,
    text: "text-status-info",
    border: "border-status-info/20 border-l-status-info",
    tint: "bg-status-info/[0.04]",
    line: "border-status-info",
  },
  warning: {
    label: "Warning",
    Icon: AlertTriangleIcon,
    text: "text-status-warning",
    border: "border-status-warning/20 border-l-status-warning",
    tint: "bg-status-warning/[0.04]",
    line: "border-status-warning",
  },
  danger: {
    label: "Problem",
    Icon: ShieldAlertIcon,
    text: "text-status-danger",
    border: "border-status-danger/20 border-l-status-danger",
    tint: "bg-status-danger/[0.04]",
    line: "border-status-danger",
  },
};

/* -------------------------------------------------------------------------- */
/* The slots                                                                  */
/* -------------------------------------------------------------------------- */

/**
 * One place in the stack where a choice can be made, in ESO's load order.
 *
 * Order is load order and must not be re-sorted by severity: which thing loads
 * before which is the information that makes the technique-order finding
 * comprehensible, and a list that reshuffles itself as problems appear is one
 * the user cannot build a mental model of.
 *
 * `compiler` and `companion` are gone as rows of their own. They were layers in
 * the pipeline model because they load separately, but nobody chooses a
 * `d3dcompiler_47.dll` independently of ReShade, so as *slots* they fold into
 * the thing they ship beside.
 */
export type Slot = "reshade" | "addons" | "nr" | "sr" | "shaders" | "motion" | "preset" | "tuning";

export const SLOT_ORDER: Slot[] = [
  "reshade",
  "addons",
  "nr",
  "sr",
  "shaders",
  "motion",
  "preset",
  "tuning",
];

/**
 * The name on the rail row.
 *
 * These are the renames agreed in the naming pass: "Injector" told the user
 * what the file does to the process rather than what it is, and nobody
 * searching a forum guide is looking for a "Super Resolution runtime". The real
 * filename is never lost — it renders beneath, in `font-mono text-[11px]`, so
 * a guide that says "rename dxgi.dll" stays greppable against the screen.
 */
export const SLOT_LABEL: Record<Slot, string> = {
  reshade: "ReShade",
  addons: "ReShade add-ons",
  nr: "Neural Rendering",
  sr: "DLSS upscaling",
  shaders: "Shader packs",
  motion: "Motion vectors",
  preset: "Preset",
  tuning: "Tuning",
};

/**
 * Upstream's own name for each motion-vector provider.
 *
 * Kalpa does not invent wording here — these are what DLSS5-Feeder's own
 * provider table calls them. Note these are *provider* names for use in a
 * sentence, not technique identifiers: the identifier is what the preset
 * stores and what Kalpa matches on, and the two are not the same string.
 */
export const MV_PROVIDER_LABEL: Record<
  "shared_texture" | "launchpad" | "vort" | "lumenite_kernel" | "lumenite_quant_motion",
  string
> = {
  shared_texture: "the shared texMotionVectors texture",
  launchpad: "iMMERSE LaunchPad",
  vort: "VORT",
  lumenite_kernel: "LumeniteFX Kernel",
  lumenite_quant_motion: "LumeniteFX QuantMotion",
};

/** What one file in the stack does, for the card that describes it. */
export const ROLE_LABEL: Record<StackRole, string> = {
  injector: "ReShade",
  neural_rendering: "Neural Rendering runtime",
  super_sampling: "DLSS upscaling runtime",
  frame_generation: "Frame Generation runtime",
  shader_compiler: "Shader compiler",
  addon: "ReShade add-on",
  companion: "Companion process",
};

/** Which slot a stack item's role belongs to. */
export const ROLE_TO_SLOT: Record<StackRole, Slot> = {
  injector: "reshade",
  // Nobody picks a shader compiler on its own; it ships with ReShade and is
  // swapped for the same reason, so it is a sub-line of that row.
  shader_compiler: "reshade",
  neural_rendering: "nr",
  super_sampling: "sr",
  // Frame Generation has no slot of its own: ESO ships no Streamline modules,
  // so there is nothing to choose between. It rides with DLSS upscaling.
  frame_generation: "sr",
  addon: "addons",
  companion: "addons",
};

/* -------------------------------------------------------------------------- */
/* Where each finding lives                                                   */
/* -------------------------------------------------------------------------- */

/**
 * The slot each finding attaches to — chosen as **where the fix is**, not where
 * the symptom shows.
 *
 * `stack-technique-source-missing` is the instructive one: it fires because a
 * *preset* enables a technique, but what the user has to do about it is install
 * the shader pack that technique comes from, so it lives on Shader packs and
 * names the preset in its copy. Putting it on the preset would be filing it
 * where nothing can be done.
 *
 * `stack-disabled` is deliberately absent. It is not a per-slot problem, it is
 * the whole-stack power state, and the status strip states it with the action
 * attached — which is strictly more useful than a finding that describes it.
 */
export const FINDING_SLOT: Record<string, Slot> = {
  "stack-no-injector": "reshade",
  "stack-addon-disabled": "addons",
  "stack-feed-host-missing": "addons",
  "stack-nr-runtime-missing": "nr",
  "stack-dlss-reverted": "sr",
  "stack-technique-source-missing": "shaders",
  "stack-search-path-mismatch": "shaders",
  "stack-mv-provider-missing": "motion",
  "stack-preset-missing": "preset",
  "stack-technique-order": "preset",
  "stack-feed-technique-off": "preset",
};

/**
 * What the user will actually notice, in their own terms.
 *
 * The Rust `detail` on each finding explains the *configuration* — which file
 * points at what, which technique is above which. That is correct and it is
 * what makes the diagnosis checkable, but it does not tell someone whether it
 * explains the thing that made them open this panel. These lines do, and they
 * are drawn from the reasoning already written in the `client_stack.rs` module
 * doc and the finding bodies; none of them is invented.
 *
 * The hardest cases are the silent ones. `stack-technique-order` and
 * `stack-mv-provider-missing` both produce a stack where nothing errors, every
 * file loads, and the image is quietly wrong — so for those the impact line is
 * the *only* thing on screen that connects the diagnosis to what the user sees.
 */
export const FINDING_IMPACT: Record<string, string> = {
  "stack-no-injector":
    "ESO starts up looking exactly like stock: no ReShade banner, the Home key does nothing, and none of the add-ons run.",
  "stack-addon-disabled":
    "That add-on is missing from the overlay's Add-ons tab even though its file is sitting right there.",
  "stack-feed-host-missing":
    "The feed never starts, so DLSS is fed nothing. You get whatever ReShade alone produces.",
  "stack-nr-runtime-missing":
    "ReShade loads and the RenoDX menu appears, but turning Neural Rendering on changes nothing at all.",
  "stack-dlss-reverted":
    "DLSS is back on ESO's own build — noticeably softer than it looked before the last game update.",
  "stack-technique-source-missing":
    "ReShade reports a failed effect when the game starts, and that technique is missing from the overlay's list.",
  "stack-search-path-mismatch":
    "The overlay's effect list is empty even though the shader folder is full.",
  "stack-mv-provider-missing":
    "Fine standing still, ghosting or smearing the moment you move — DLSS is upscaling what it thinks is a still image.",
  "stack-preset-missing":
    "ReShade loads with no effects at all, and the overlay's technique list is empty.",
  "stack-technique-order":
    "Nothing errors, but the image ghosts or smears in motion: DLSS is working from where things were last frame.",
  "stack-feed-technique-off":
    "The feed add-on is loaded but idle, so Neural Rendering has nothing to work from.",
};

/* -------------------------------------------------------------------------- */
/* The honest column                                                          */
/* -------------------------------------------------------------------------- */

/**
 * Whether Kalpa can supply a thing itself, or the user has to bring it.
 *
 * This is a licensing and supply fact, not a feature gap, and saying so plainly
 * is what keeps the library a **directory rather than a store**. Three separate
 * constraints produce it:
 *
 * - **No NVIDIA binary is ever downloaded.** The DLSS and Neural Rendering
 *   runtimes are not licensed for redistribution and there is no upstream to
 *   fetch them from. They are `byo`, verified by Authenticode signer on the way
 *   in. (DLSS Swapper's hash-vetted community DLL manifest served malware in
 *   2026; a curated mirror is the failure mode, not the fix.)
 * - **Some shader packs forbid it.** iMMERSE's licence forbids "public
 *   propagation"; `renodx-dlss5.addon64` has no licence at all and is
 *   distributed only through a Discord, so it has no fetchable URL in any case.
 *   Those are `link_only` — Kalpa names them and links the author's page.
 * - **Kalpa hosts and mirrors nothing.** What is `fetchable` comes from the
 *   author's own upstream at install time, never from a Kalpa-owned copy.
 */
export type SlotSource = "fetchable" | "byo" | "link_only" | "yours" | "derived";

export const SLOT_SOURCE: Record<Slot, SlotSource> = {
  reshade: "fetchable",
  // The add-ons this stack needs live in the RenoDX Discord: no stable URL, no
  // licence, and no signer to check even if there were. Link out, always.
  addons: "link_only",
  nr: "byo",
  sr: "byo",
  // Per pack, not per slot — LumeniteFX and VORT are fetchable, iMMERSE is not.
  // Resolved on each row rather than here.
  shaders: "fetchable",
  // Not a thing you obtain; a setting that points at a shader you already have.
  motion: "derived",
  preset: "yours",
  tuning: "derived",
};

export const SOURCE_LABEL: Record<SlotSource, string | null> = {
  fetchable: "Kalpa can fetch",
  byo: "Bring your own",
  link_only: "Link only",
  yours: "Your files",
  derived: null,
};

export const SOURCE_PILL_COLOR: Record<SlotSource, "gold" | "muted" | "sky"> = {
  fetchable: "gold",
  byo: "muted",
  link_only: "muted",
  yours: "sky",
  derived: "muted",
};

/**
 * Provenance as a colour role: **whose hand is on this thing.**
 *
 * Colour in this panel used to mean exactly one thing — severity — while the
 * brand accent quietly meant three (selected, fetchable, and "needs
 * attention"). Two axes now, each with its own physical channel so they never
 * compete for the same pixel: severity owns the 3px left border and the level
 * word, provenance owns the *glyph tint* and the word beside it.
 *
 * Gold is the narrow one. It marks Kalpa's own hand and nothing else — a file
 * Kalpa placed or could fetch — which is why selection had to stop using it.
 * `link_only` is deliberately uncoloured: a tint there would claim an
 * involvement Kalpa does not have.
 *
 * The word is not optional. On the three light themes every status token
 * collapses towards one near-black hue, so hue separation is weak by
 * construction and the label is what actually carries the meaning.
 */
export const SOURCE_META: Record<SlotSource, { glyph: string; word: string }> = {
  fetchable: { glyph: "text-primary", word: "Kalpa can fetch" },
  byo: { glyph: "text-accent-sky", word: "Bring your own" },
  yours: { glyph: "text-accent-sky", word: "Your files" },
  link_only: { glyph: "text-muted-foreground", word: "Link only" },
  // Not a file you obtain — a setting other slots consume. `status-library` is
  // the one status token this panel does not otherwise use, so it arrives
  // without baggage, and the addon list already uses violet for "a thing other
  // things depend on rather than run".
  derived: { glyph: "text-status-library", word: "Setting" },
};

/* -------------------------------------------------------------------------- */
/* Derived state                                                              */
/* -------------------------------------------------------------------------- */

/** The findings that belong to one slot, worst first. */
export function findingsForSlot(slot: Slot, stack: ClientStack): HealthFinding[] {
  return stack.findings
    .filter((f) => FINDING_SLOT[f.id] === slot)
    .sort((a, b) => LEVEL_ORDER[a.level] - LEVEL_ORDER[b.level]);
}

export const LEVEL_ORDER: Record<HealthLevel, number> = {
  danger: 0,
  warning: 1,
  info: 2,
  ok: 3,
};

export function worstLevel(levels: HealthLevel[]): HealthLevel {
  return levels.reduce((worst, level) => (LEVEL_ORDER[level] < LEVEL_ORDER[worst] ? level : worst));
}

/** Is there anything filling this slot at all? */
export function slotFilled(slot: Slot, stack: ClientStack): boolean {
  switch (slot) {
    case "shaders":
      return stack.shaders.present;
    case "preset":
      return stack.preset?.exists ?? false;
    case "tuning":
      return stack.tuning.length > 0;
    case "motion":
      // Filled means something is actually producing vectors. A provider that
      // is *selected* but whose technique is not enabled is the failure this
      // slot exists to surface, not a filled slot.
      return Boolean(stack.preset?.mv_provider?.technique);
    default:
      return stack.items.some((item) => ROLE_TO_SLOT[item.role] === slot);
  }
}

/**
 * The row's status.
 *
 * A slot with a finding takes that finding's level. Otherwise a filled slot is
 * `ok` and an empty one is `info` — never a warning, because an empty slot is
 * usually a deliberate choice (most people run no add-ons at all) and painting
 * it amber would train the user to ignore the colour that means something.
 */
export function slotLevel(slot: Slot, stack: ClientStack): HealthLevel {
  const findings = findingsForSlot(slot, stack);
  if (findings.length > 0) return worstLevel(findings.map((f) => f.level));
  return slotFilled(slot, stack) ? "ok" : "info";
}

/**
 * What is in the slot right now: the identifier, and what else is worth saying
 * about it.
 *
 * Split into two fields rather than one joined string because **only `id` is
 * a real thing on disk.** Monospace was doing duty for filenames, counts,
 * states and prose alike, which drains it of meaning — the reason to set
 * `dxgi.dll` in mono is that a forum guide says "rename dxgi.dll" and the two
 * should match on screen. `28 effects` is not that; it is a count, and it
 * belongs in the sans face.
 *
 * Returns null when the slot is empty — the caller renders its own empty copy
 * rather than this rendering "none", so each slot can say something specific.
 */
export interface SlotSubLine {
  /** A string that exists on disk or in an INI file. Rendered in mono. */
  id: string;
  /** Counts, versions, states. Rendered in the sans face. */
  meta?: string;
}

export function slotSubLine(slot: Slot, stack: ClientStack): SlotSubLine | null {
  const items = stack.items.filter((item) => ROLE_TO_SLOT[item.role] === slot);
  switch (slot) {
    case "shaders":
      return stack.shaders.present
        ? {
            id: "reshade-shaders",
            meta: `${stack.shaders.effect_count} effect${
              stack.shaders.effect_count === 1 ? "" : "s"
            }`,
          }
        : null;
    case "preset": {
      const preset = stack.preset;
      if (!preset?.exists) return null;
      const name = preset.path.split(/[\\/]/).pop() ?? preset.path;
      return {
        id: name,
        meta: `${preset.techniques.length} technique${preset.techniques.length === 1 ? "" : "s"}`,
      };
    }
    case "tuning":
      return stack.tuning.length > 0
        ? {
            id: "[RenoDX.DLSS5]",
            meta: `${stack.tuning.length} value${stack.tuning.length === 1 ? "" : "s"}`,
          }
        : null;
    case "motion": {
      // The technique as the *preset* spells it: that is the identifier the
      // user can grep for in the file and the one Kalpa lists elsewhere.
      const technique = stack.preset?.mv_provider?.technique;
      return technique ? { id: technique } : null;
    }
    default: {
      if (items.length === 0) return null;
      const [item] = items;
      if (items.length === 1 && item) {
        return { id: item.file_name, meta: item.version ?? undefined };
      }
      return {
        id: items.map((entry) => entry.file_name).join(" · "),
      };
    }
  }
}

/* -------------------------------------------------------------------------- */
/* Whole-stack power state                                                    */
/* -------------------------------------------------------------------------- */

/**
 * Three states, not two.
 *
 * **A plain on/off toggle is forbidden here.** "Partly off" is real and
 * reachable — a switch-off that failed part way, or a file the user renamed by
 * hand — and it is precisely the state in which someone needs the panel to be
 * truthful. A boolean would have to round it to one of the other two, and both
 * roundings lie: "on" hides that ESO is loading half a stack, "off" invites a
 * re-enable of files that were never parked.
 *
 * Derived from `parked` rather than from `is_disabled`, because `is_disabled`
 * asks only whether an injector name is parked. Both `dxgi.dll` and `d3d11.dll`
 * are loaded by the DLL search order, so a folder with one parked and one live
 * still loads ReShade while reporting itself off.
 */
export type PowerState = "on" | "partly_off" | "off";

export function powerState(stack: ClientStack): PowerState {
  if (stack.parked.length === 0) return "on";
  return stack.is_disabled ? "off" : "partly_off";
}

export const POWER_COPY: Record<PowerState, { state: string; action: string }> = {
  on: { state: "Switched on · Modded", action: "Switch off…" },
  // "Finish" and not "Switch on", because the toggle always plans Enable while
  // anything is parked — there is no path from here to a switch-off, and a
  // button offering one would be describing an action that cannot happen.
  partly_off: { state: "Partly switched off", action: "Finish switching on…" },
  off: { state: "Switched off · Stock ESO", action: "Switch back on…" },
};
