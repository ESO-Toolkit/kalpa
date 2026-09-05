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
 * the findings that concern it, the thing currently filling it, whether the
 * live path wants it at all, and the options it could be filled with instead.
 *
 * Nothing about the backend model changed. `Slot` is deliberately a renaming of
 * the old `Stage` union with the same members in the same load order, so the
 * pipeline reading is still available to anyone who wants it; it is just no
 * longer what the screen leads with.
 *
 * Four rules hold this file together:
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
 * 4. **Empty is not a verdict until `active_path` has been read.** There are
 *    two mutually exclusive Neural Rendering paths, and six of these eight
 *    slots mean different things on each. A correct direct-path install has
 *    nothing in motion, preset or tuning, and the panel used to render all
 *    three as absences and then announce "Everything agrees" — the two halves
 *    of one bug. So presence (`slotFilled`) and want (`slotNeed`) are separate
 *    questions here, and no row states a verdict from the first alone.
 */

import {
  AlertCircleIcon,
  AlertTriangleIcon,
  ArchiveIcon,
  CircleCheckIcon,
  HelpCircleIcon,
  ShieldAlertIcon,
  ShieldCheckIcon,
} from "lucide-react";

import type {
  ActivePath,
  ClientStack,
  HealthFinding,
  HealthLevel,
  SlotNeed,
  SlotStatus,
  StackRole,
} from "./types";

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
 *
 * `stack-addon-not-in-dllmain` files under Add-ons for the same
 * where-the-fix-is reason. The symptom is that Neural Rendering does nothing,
 * which points the eye at the NR row, but the repair is one `LoadFromDllMain=`
 * line under `[ADDON]` in `ReShade.ini` — an add-on load-order edit, which is
 * what `stack-addon-disabled` already is and where it already lives.
 */
export const FINDING_SLOT: Record<string, Slot> = {
  "stack-no-injector": "reshade",
  "stack-addon-disabled": "addons",
  "stack-addon-not-in-dllmain": "addons",
  "stack-feed-host-missing": "addons",
  "stack-nr-runtime-missing": "nr",
  "stack-dlss-reverted": "sr",
  "stack-technique-source-missing": "shaders",
  "stack-search-path-mismatch": "shaders",
  "stack-mv-provider-missing": "motion",
  "stack-preset-missing": "preset",
  "stack-preset-empty": "preset",
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
 * The hardest cases are the silent ones. `stack-technique-order`,
 * `stack-mv-provider-missing` and `stack-addon-not-in-dllmain` all produce a
 * stack where nothing errors, every file loads, and the image is quietly wrong
 * — so for those the impact line is the *only* thing on screen that connects
 * the diagnosis to what the user sees. The last of the three is the one that
 * cost a real evening: the add-on's menu is right there in the overlay, so
 * "the settings do nothing" is genuinely the whole observable symptom.
 */
export const FINDING_IMPACT: Record<string, string> = {
  "stack-no-injector":
    "ESO starts up looking exactly like stock: no ReShade banner, the Home key does nothing, and none of the add-ons run.",
  "stack-addon-disabled":
    "That add-on is missing from the overlay's Add-ons tab even though its file is sitting right there.",
  "stack-addon-not-in-dllmain":
    "The add-on's menu is right there in the ReShade overlay and nothing you set in it changes the picture — Neural Rendering is off whatever the settings say.",
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
  "stack-preset-empty":
    "ReShade loads, the add-ons load, nothing errors — and the game looks exactly like stock, because not a single effect is running.",
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
/* Which shape this stack is                                                  */
/* -------------------------------------------------------------------------- */

/**
 * The active path, said in the rail's own width.
 *
 * The user's whole confusion was not knowing which of the two shapes their
 * stack was *supposed* to be, so six empty rows read as six holes rather than
 * as the correct answer for the path they were running. Naming the path once,
 * at the top of the rail, is what makes the rest of the column legible — every
 * "not on this path" row below it is then an obvious consequence rather than a
 * separate mystery.
 *
 * Deliberately uncoloured, all five of them. There is no severity here: `both`
 * is a state nobody designed but it is not itself a fault (its feed findings
 * fire on their own rows, in their own colours), and `neither` is the ordinary
 * plain-ReShade install. Painting either amber would spend the attention
 * budget on a shape rather than on a problem.
 *
 * `blurb` is capped at roughly 32 characters because it renders at 11px into
 * the 190px of text width a 240px rail leaves — see the width note on the rail
 * itself. A path summary that ends in an ellipsis has summarised nothing.
 */
export const PATH_META: Record<ActivePath, { label: string; blurb: string }> = {
  direct: { label: "Direct", blurb: "One add-on, no feed, no preset" },
  feed: { label: "Feed", blurb: "Feed add-on set plus its host" },
  both: { label: "Both loaded", blurb: "Both add-on sets are running" },
  neither: { label: "None", blurb: "ReShade effects only" },
  unknown: { label: "Couldn't tell", blurb: "Kalpa could not read the folder" },
};

/* -------------------------------------------------------------------------- */
/* The need axis                                                              */
/* -------------------------------------------------------------------------- */

/**
 * How each `SlotNeed` presents — a word, a glyph, and whether it borrows the
 * severity palette.
 *
 * Read the two-axis note on `SOURCE_META` first: severity owns the 3px left
 * border and the level word, provenance owns the glyph tint and the word beside
 * it. Need is a **third** thing and it deliberately does not open a third
 * colour channel. Three of these four states carry no status hue at all, for a
 * reason each:
 *
 * - `not_on_this_path` is *correct*. Colour would make it an event. It gets a
 *   check glyph and a plain structural plate, which is what "nothing to do
 *   here" looks like when it is true rather than when it is being excused.
 * - `installed_unused` is also correct, and additionally must never read as
 *   "you can delete this" — the pieces it describes (iMMERSE LaunchPad, the
 *   parked `renodx-dlss5` / `dlss5-feed` add-ons) are link-only or Discord-only,
 *   so Kalpa cannot fetch them back and the user's copy is their only fallback.
 *   An archive glyph, and `keep_because` shown beside it, not hidden.
 * - `required` says nothing extra at all. It is the default reading of a row
 *   and adding a badge for it would put a mark on all eight.
 *
 * `unknown` is the exception and takes `info`, because it is the one state that
 * is genuinely a gap in what Kalpa knows. It must not look like
 * `not_on_this_path`: that one asserts a slot is correctly empty, and asserting
 * that from a folder Kalpa could not read is a guess wearing a verdict's
 * clothes.
 */
export const NEED_META: Record<
  SlotNeed,
  {
    /** Null for `required`: the default row says nothing extra. */
    word: string | null;
    Icon: typeof ShieldCheckIcon;
    glyph: string;
    /** Plate colours. Structural for the two correct states, `info` for the
     *  one that is an absence of knowledge. */
    plate: string;
  }
> = {
  required: {
    word: null,
    Icon: CircleCheckIcon,
    glyph: "text-muted-foreground",
    plate: "bg-structure-03",
  },
  not_on_this_path: {
    word: "Not on this path",
    Icon: CircleCheckIcon,
    glyph: "text-muted-foreground",
    plate: "bg-structure-03",
  },
  installed_unused: {
    word: "Installed, not used here",
    Icon: ArchiveIcon,
    glyph: "text-muted-foreground",
    plate: "bg-structure-03",
  },
  unknown: {
    word: "Kalpa could not check",
    Icon: HelpCircleIcon,
    glyph: "text-status-info",
    plate: "border border-l-[3px] border-status-info/20 border-l-status-info bg-status-info/[0.04]",
  },
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

/**
 * What the backend says about this slot's place on the live path.
 *
 * `stack.slots` is contractually all eight rows in slot order, so a miss is a
 * contract violation rather than a state. It falls back to `required` and not
 * to `unknown`, deliberately: `required` is exactly the behaviour every row had
 * before this axis existed, so a backend that has not been rebuilt renders the
 * old panel rather than eight rows announcing that Kalpa could not look — which
 * would be a claim about the client folder made on the strength of a missing
 * JSON field.
 */
export function slotStatus(slot: Slot, stack: ClientStack): SlotStatus | null {
  return stack.slots?.find((entry) => entry.slot === slot) ?? null;
}

export function slotNeed(slot: Slot, stack: ClientStack): SlotNeed {
  return slotStatus(slot, stack)?.need ?? "required";
}

/**
 * Whether anything is **present** in this slot.
 *
 * Presence only, and that is the whole point of keeping it a separate question
 * from `slotNeed`. Neither answer is a verdict on its own: an empty motion
 * slot is correct on the direct path and a real gap on the feed path, and the
 * same `false` is returned in both cases. Anything rendering a judgement has to
 * read both.
 */
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
 * A slot with a finding takes that finding's level. Otherwise the reasoning
 * that used to end at "an empty slot is `info`" now goes one step further, and
 * the step changes the answer.
 *
 * The old rule was: an empty slot is `info`, never a warning, because an empty
 * slot is usually a deliberate choice and painting it amber would train the
 * user to ignore the colour that means something. Every clause of that is still
 * true — and it was understating itself. If an empty slot is usually a
 * deliberate choice, then `info` is *also* too loud: it is the colour of "look
 * at this", spent on six rows of a correct direct-path install, which is
 * precisely how the panel came to say "Everything agrees" while looking like a
 * list of holes. Now that `active_path` says which empties are deliberate,
 * Kalpa can stop guessing and stop tinting them:
 *
 * - `not_on_this_path` → `ok`. Not a gap. It is the right answer for the path
 *   that is running, and the rail states it in words on the row.
 * - `installed_unused` → `ok`. Present and correct; not a fault and not a
 *   suggestion to remove anything.
 * - `unknown` → `info`. The one case that really is an absence — of knowledge,
 *   not of files — and the only empty row still worth a colour.
 * - `required` and empty → `info`, exactly as before. Still never `warning`:
 *   whether a missing required piece is actually broken is a question for a
 *   finding, which has the detail to answer it.
 */
export function slotLevel(slot: Slot, stack: ClientStack): HealthLevel {
  const findings = findingsForSlot(slot, stack);
  if (findings.length > 0) return worstLevel(findings.map((f) => f.level));
  if (slotFilled(slot, stack)) return "ok";
  switch (slotNeed(slot, stack)) {
    case "not_on_this_path":
    case "installed_unused":
      return "ok";
    default:
      return "info";
  }
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
 * For the rail that caller is `slotRailLine` below, which knows what an empty
 * slot *means* on the live path; the pane has richer copy of its own.
 */
export interface SlotSubLine {
  /** A string that exists on disk or in an INI file. Rendered in mono. */
  id: string;
  /** Counts, versions, states. Rendered in the sans face. */
  meta?: string;
  /**
   * True when `id` is a sentence about the slot rather than a string on disk.
   *
   * Only `slotRailLine` sets it, for the rows that hold nothing: "not on this
   * path" is prose and setting it in mono would be the same category error the
   * `id`/`meta` split exists to prevent. It also stops the eye reading it as a
   * filename that failed to load.
   */
  prose?: boolean;
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
    case "tuning": {
      // The section name comes from `tuning_section`, never from a literal.
      // It used to be hardcoded `[RenoDX.DLSS5]`, which is the **feed** path's
      // add-on's block — so on a direct-path install the rail put a parked
      // add-on's section name under the word "Tuning" and a `NeuralUplift=0`
      // that had not been in force for months read as this install's current
      // setting. It misled the user and the diagnosis both.
      //
      // Whether those values are live is `tuning_owner`, and it is not said
      // here: it lands on the need axis (a fossil is `installed_unused`), so
      // `slotRailLine` overwrites `meta` with "not used here" and the pane
      // states the whole reason. Saying it twice in 190px would cost the
      // section name its width.
      if (stack.tuning.length === 0) return null;
      const count = `${stack.tuning.length} value${stack.tuning.length === 1 ? "" : "s"}`;
      return {
        id: stack.tuning_section ? `[${stack.tuning_section}]` : "unnamed section",
        meta: count,
      };
    }
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

/**
 * What an **empty** rail row says, per need.
 *
 * "nothing here" was the single answer for all four, and for three of them it
 * is the wrong sentence rather than a terse one. It reads as absence — as a
 * thing that ought to be there and is not — which is exactly the reading that
 * made a correct direct-path install look like six failures.
 *
 * Each of these is a statement, not a shrug, and each is short enough to live
 * in the rail's 190px of text width without an ellipsis.
 */
const EMPTY_RAIL_LINE: Record<SlotNeed, SlotSubLine> = {
  required: { id: "nothing here", prose: true },
  not_on_this_path: { id: "not on this path", prose: true },
  // Unreachable in practice — "installed" and "nothing here" cannot both hold —
  // but the record is total so a future backend cannot fall through to a blank
  // sub-line by adding one branch.
  installed_unused: { id: "nothing here", prose: true },
  unknown: { id: "not checked", prose: true },
};

/**
 * The sub-line for one rail row: what is in the slot, read through what the
 * live path wants there.
 *
 * The two overwrites are deliberate losses of detail, both paid for by width.
 * A row that is present-but-unused says *that* rather than its count, and a row
 * Kalpa could not vouch for says so rather than reporting a count it cannot
 * stand behind; in both cases the pane still shows the full figures. The rail's
 * job is the identifier and one true thing about it, and
 * `reshade-shaders · 28 effects · not used here` is neither.
 */
export function slotRailLine(slot: Slot, stack: ClientStack): SlotSubLine {
  const need = slotNeed(slot, stack);
  const sub = slotSubLine(slot, stack);
  if (!sub) return EMPTY_RAIL_LINE[need];
  if (need === "installed_unused") return { ...sub, meta: "not used here" };
  if (need === "unknown") return { ...sub, meta: "not checked" };
  return sub;
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
