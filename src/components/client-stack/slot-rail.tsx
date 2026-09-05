import { useCallback, useEffect, useRef } from "react";
import {
  ActivityIcon,
  AlertTriangleIcon,
  HardDriveIcon,
  LayersIcon,
  PaletteIcon,
  PowerIcon,
  PowerOffIcon,
  PuzzleIcon,
  ScanSearchIcon,
  ScrollTextIcon,
  Settings2Icon,
  SlidersHorizontalIcon,
  SparklesIcon,
} from "lucide-react";

import { InfoPill } from "@/components/ui/info-pill";
import { cn } from "@/lib/utils";

import {
  LEVEL_META,
  NEED_META,
  PATH_META,
  POWER_COPY,
  SLOT_LABEL,
  SLOT_ORDER,
  SLOT_SOURCE,
  SOURCE_META,
  findingsForSlot,
  powerState,
  slotLevel,
  slotNeed,
  slotRailLine,
} from "./slots";
import type { Slot } from "./slots";
import type { ClientStack, NeuralRenderingState } from "./types";

/**
 * The sub-line beside "No known failures".
 *
 * Only `running` is good news, and it is the one state earned by positive
 * evidence rather than by an absence: a climbing `EvaluateFeature` counter.
 * `unknown` deliberately does not borrow that tone — a log Kalpa could not
 * read, or one predating the add-on, is not a working stack.
 */
const NR_RAIL_META: Record<NeuralRenderingState, string> = {
  running: "Neural Rendering ran",
  stalled: "ran, then stopped",
  unknown: "no proof it ran",
};

/**
 * Per-slot iconography.
 *
 * The rail used to draw the *level* icon on every row, so a healthy stack was
 * eight identical shields and read as a list of text rather than a set of
 * distinct things. Severity has two other carriers already — the 3px left
 * border and the count pill — so the glyph is free to say what the slot *is*.
 * The level is still announced to screen readers on every row.
 */
const SLOT_ICON: Record<Slot, typeof LayersIcon> = {
  reshade: LayersIcon,
  addons: PuzzleIcon,
  nr: SparklesIcon,
  sr: ScanSearchIcon,
  shaders: PaletteIcon,
  motion: ActivityIcon,
  preset: SlidersHorizontalIcon,
  tuning: Settings2Icon,
};

/**
 * Where a visual group begins.
 *
 * The stack is three kinds of thing: binaries the game loads, content ReShade
 * runs, and configuration. Spacing says so. There are no group headings — three
 * more labels in a 240px column would cost more than they explain, and the
 * spine already carries the ordering.
 */
const SLOT_GROUP_START = new Set<Slot>(["shaders", "preset"]);

/** The three views that are about the whole folder rather than one slot. */
export type StackView = "power" | "records" | "logs";

/**
 * The slots, plus the whole-stack views pinned beneath them.
 *
 * This rail is the panel's only navigation. It replaced a pipeline diagram —
 * same eight rows, re-read as places a choice can be made — and it has now
 * absorbed the status strip and the dialog footer as well, because power,
 * Kalpa's records and the log check were always *views*: they were already
 * `SelectionKey`s routed to the same pane. Rendering them as a strip above and
 * a button below meant paying 129px of chrome to say what three 36px rows say.
 *
 * The whole-stack group is `shrink-0` and sits outside the scrolling slot list
 * on purpose. Uninstall, "Stop managing" and emergency removal all live behind
 * "Kalpa's records", and they are the recovery path for everything the panel
 * can do — so the one thing that must never scroll out of reach is that row.
 *
 * Above all of it sits `PathHeader`, which is not a row and not selectable. It
 * names which of the two mutually exclusive Neural Rendering shapes this stack
 * is, because the rows below cannot be read without it: the same empty motion
 * slot is correct on one path and a silent image-quality failure on the other.
 *
 * The rail is also deliberately always expanded. It was collapsed by default
 * for a healthy stack once, which reads well as a health report and fails
 * completely as a management panel: every action lives inside these rows, so
 * collapsing them hid the whole feature behind a "Show layers" button.
 */
export function SlotRail({
  stack,
  selected,
  onSelect,
  isManaged,
  trackedCount,
  logCount,
  nrState,
}: {
  stack: ClientStack;
  /** A slot, or one of the whole-stack views, or nothing. */
  selected: Slot | StackView | null;
  onSelect: (key: Slot | StackView) => void;
  isManaged: boolean;
  trackedCount: number | null;
  /** Fatal-only excerpt count. Benign lines are counted elsewhere, not here. */
  logCount: number;
  /** Whether the logs show Neural Rendering actually ran. */
  nrState: NeuralRenderingState;
}) {
  const railRef = useRef<HTMLDivElement>(null);

  // Refreshes replace the stack data but not the slot model. If selection is
  // adjusted while keyboard focus is in the rail, keep focus on the new roving
  // tab stop rather than stranding it on an option that is no longer selected.
  useEffect(() => {
    const container = railRef.current;
    if (!container || !container.contains(document.activeElement)) return;
    const tabStop = container.querySelector<HTMLElement>('[role="option"][tabindex="0"]');
    tabStop?.focus();
  }, [selected]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const container = railRef.current;
    if (!container) return;
    const items = Array.from(container.querySelectorAll<HTMLElement>('[role="option"]'));
    if (items.length === 0) return;
    e.preventDefault();
    const idx = items.indexOf(document.activeElement as HTMLElement);
    let nextIdx = idx === -1 ? 0 : idx;
    if (e.key === "Home") nextIdx = 0;
    else if (e.key === "End") nextIdx = items.length - 1;
    else if (e.key === "ArrowDown") nextIdx = Math.min(nextIdx + 1, items.length - 1);
    else nextIdx = Math.max(nextIdx - 1, 0);

    const next = items[nextIdx];
    next?.focus();
    next?.click();
  }, []);

  const power = powerState(stack);
  const tabStop = selected ?? SLOT_ORDER[0];

  return (
    // 240px, not 200. At 200 the sub-line had 152px to work with and three of
    // the eleven rows truncated at once — `d3dcompiler_4…`, `renodx-dlss.addon64
    // · dl…`, `ReShadePreset.ini · 0 tech…`. A file name that ends in an ellipsis
    // is not an identifier any more, and this rail's whole job is to show
    // identifiers. 240 gives the sub-line 192px and leaves only the genuinely
    // two-file add-ons row clipped; the pane gives up 40px of a column that was
    // already ending in empty space.
    <div
      ref={railRef}
      role="listbox"
      aria-label="Graphics stack views"
      onKeyDown={handleKeyDown}
      className="flex w-[240px] shrink-0 flex-col"
    >
      <PathHeader stack={stack} />

      {/* The spine: a hairline behind the glyph nodes, so eight rows read as an
          ordered route rather than eight equal options — which is what the
          stack actually is. ReShade loads first, the preset picks the
          motion-vector provider, the feed consumes it. It says "ordered"
          without redrawing the pipeline diagram the redesign retired.

          Grouping is rhythm, not labels: binaries the game loads, then content
          ReShade runs, then configuration. Three more headings in a 240px
          column would cost more than they explain. */}
      <div role="presentation" className="min-h-0 flex-1 overflow-y-auto pr-1">
        {/* The spine hangs off this inner wrapper rather than the scroll
            container. On the container it stretched to the full flex height and
            ran on past the last node into empty space, which reads as a route
            that goes somewhere Kalpa is not showing you. Here it spans exactly
            first node to last. */}
        {/* The inset is the node's own centre line: 4px of leading margin plus
            half a 36px row. Anything rounder leaves a stub of spine above the
            first glyph or below the last. */}
        <div className="relative before:absolute before:top-[22px] before:bottom-[18px] before:left-[17px] before:w-px before:bg-structure-10">
          {SLOT_ORDER.map((slot, i) => (
            <div key={slot} className={i > 0 && SLOT_GROUP_START.has(slot) ? "mt-3" : "mt-1"}>
              <SlotRow
                slot={slot}
                stack={stack}
                selected={selected === slot}
                tabIndex={tabStop === slot ? 0 : -1}
                onSelect={() => onSelect(slot)}
              />
            </div>
          ))}
        </div>
      </div>

      <div className="my-3 shrink-0 border-t border-structure-06" />

      <div role="presentation" className="shrink-0 space-y-1 pr-1">
        <RailRow
          Icon={power === "on" ? PowerIcon : power === "off" ? PowerOffIcon : AlertTriangleIcon}
          tone={
            power === "on"
              ? "text-status-success"
              : power === "off"
                ? "text-muted-foreground"
                : "text-status-warning"
          }
          label="Power"
          // Prose, all three of these rows. Their sub-lines are states rather
          // than filenames, and mono had been claiming otherwise — the same
          // category error the `not on this path` line would have made.
          id={POWER_COPY[power].state}
          idIsProse
          emphasis={power !== "on" && power !== "off"}
          selected={selected === "power"}
          tabIndex={tabStop === "power" ? 0 : -1}
          onSelect={() => onSelect("power")}
        />
        <RailRow
          Icon={HardDriveIcon}
          tone="text-muted-foreground"
          label="Kalpa's records"
          // The count is withheld until the inventory has loaded: "0 files" for
          // an adopted stack is a false statement, not a pending one.
          id={isManaged ? "Managed" : "Not managed yet"}
          idIsProse
          meta={
            isManaged && trackedCount !== null
              ? `${trackedCount} file${trackedCount === 1 ? "" : "s"}`
              : undefined
          }
          // Amber and a word, not a gold "!". Gold means Kalpa's own hand in
          // this panel, and a bare glyph would make colour the only signal.
          pill={isManaged ? null : <InfoPill color="amber">Not managed</InfoPill>}
          selected={selected === "records"}
          tabIndex={tabStop === "records" ? 0 : -1}
          onSelect={() => onSelect("records")}
        />
        {/* "Nothing matched" was the same sentence over a clean log, an
            unreadable one, and a log from before the add-on was installed —
            an all-clear derived from an empty array, which is the header
            badge's old bug one row further down. `logCount` is fatal-only
            now, so absence of matches is the absence of a *known* failure and
            nothing more; the NR state is what says whether anything actually
            ran, and it is the only line here that can carry good news. */}
        <RailRow
          Icon={ScrollTextIcon}
          tone={
            logCount > 0
              ? "text-status-warning"
              : nrState === "stalled"
                ? "text-status-warning"
                : "text-muted-foreground"
          }
          label="Log check"
          id={
            logCount > 0
              ? `${logCount} line${logCount === 1 ? "" : "s"} matched`
              : "No known failures"
          }
          meta={logCount > 0 ? undefined : NR_RAIL_META[nrState]}
          idIsProse
          emphasis={logCount > 0 || nrState === "stalled"}
          selected={selected === "logs"}
          tabIndex={tabStop === "logs" ? 0 : -1}
          onSelect={() => onSelect("logs")}
        />
      </div>
    </div>
  );
}

/**
 * Which of the two shapes this stack is, stated once, above everything.
 *
 * This is the sentence the panel was missing. Six of the eight rows below mean
 * different things on the direct path than on the feed path, and without
 * naming the path first, "not on this path" on three consecutive rows is three
 * separate riddles rather than one obvious consequence. The user's own
 * confusion was exactly this: not knowing which shape their stack was supposed
 * to be, and therefore unable to tell a correct empty slot from a broken one.
 *
 * Not a rail row and not selectable. It is a caption for the list, not an entry
 * in it — putting it inside the `listbox` would give arrow-key navigation a
 * stop that opens nothing, and the path has no pane of its own. The evidence
 * behind the verdict lives in the Tuning pane, which is where a user who
 * disbelieves it will go.
 *
 * Uncoloured on purpose; see `PATH_META`. None of the five states is a
 * severity, and the ones that look like they might be (`both`, `unknown`)
 * already raise their own findings on their own rows.
 */
function PathHeader({ stack }: { stack: ClientStack }) {
  const path = PATH_META[stack.active_path];
  return (
    <div className="mb-2 shrink-0 rounded-md bg-structure-03 px-2 py-1.5">
      <p className="font-heading text-[10px] font-bold uppercase leading-[14px] tracking-[0.05em] text-muted-foreground">
        Neural Rendering path
      </p>
      <p className="flex min-w-0 items-baseline gap-1.5 text-[11px] leading-[14px]">
        <span className="shrink-0 font-heading font-semibold text-foreground">{path.label}</span>
        <span className="truncate text-muted-foreground">{path.blurb}</span>
      </p>
    </div>
  );
}

/**
 * Shared row chrome. 36px: a 16px label line over a 14px sub-line.
 *
 * The row is a **bonded pair**, and the leading has to say so. It used to be
 * 32px holding a 16px label over an 11px/12px sub-line — a sub-1.0 line height,
 * with 2px to spare and 2px of margin between rows. Label-to-sub was then
 * *tighter* than row-to-row by about a pixel, so eleven rows read as
 * twenty-two lines. Now the pair costs 30px inside a 36px row (3px of internal
 * leading each side) and rows are 4px apart, so the whitespace between a label
 * and its identifier is a third of the whitespace between one row and the next.
 *
 * Rows have **no box.** Eight bordered, filled pills said "eight equal
 * options" and gave the eye nothing to travel along; without them the only
 * bordered row is one carrying a finding, which is therefore the row you look
 * at.
 *
 * Selection is a fill and a weight, never a colour. It used to paint the row
 * gold *in place of* the severity border, so selecting a slot with a problem
 * hid the red while you were looking at it. Severity owns the left border
 * unconditionally now, and gold is reserved for Kalpa's own hand.
 *
 * Weight is the third carrier of severity, alongside that border and the count
 * pill. Eight labels at one weight is a wall with no way in, and the rail has
 * exactly one real ranking to offer: a row with a finding outranks a row
 * without. So `emphasis` promotes the label the same way selection does. The
 * two can coincide harmlessly — the fill says which row you are on, the border
 * and pill say which row is shouting.
 *
 * **Provenance is a word here, not a colour.** It was a tint on the glyph and
 * nothing else, which fails outright on the user's own theme: Elsweyr Moons
 * reseeds `--primary` to #ccd7f2, within a couple of dE of `--foreground`, so
 * every "Kalpa can fetch" row carried no visible mark at all. `SOURCE_META`
 * has had a `word` for each source since it was written — and a note saying
 * "the word is not optional" — and nothing displayed it.
 *
 * It is `sr-only` rather than visible, and that is a measurement rather than a
 * preference. The row has 190px of text width (240 rail − 4 scrollbar gutter −
 * 16 padding − 20 glyph − 10 gap). "Neural Rendering" at 13px/600 is ~105px and
 * "BRING YOUR OWN" as a 10px uppercase micro-label is ~70px, so the pair
 * overflows before any gap — and the fix would be to truncate the label, in a
 * column whose stated job is showing identifiers whole. A wider rail or shorter
 * provenance words would change that answer; both are decisions above this
 * component. The pane header shows the same word full size, which is where a
 * sighted user gets it today, and `link_only` — the one source deliberately
 * left uncoloured, because a tint would claim an involvement Kalpa does not
 * have — is the case that most needed saying out loud at all.
 */
function RailRow({
  Icon,
  tone,
  label,
  id,
  idIsProse,
  meta,
  pill,
  selected,
  tabIndex,
  accent,
  node,
  emphasis,
  notes,
  onSelect,
}: {
  Icon: typeof LayersIcon;
  tone: string;
  label: string;
  /** A string that exists on disk or in an INI file. Set in mono. */
  id: string;
  /** True when `id` is prose about the row's state rather than something on
   *  disk. Mono is reserved for the latter — a guide says "rename dxgi.dll",
   *  and no guide anywhere says "not on this path". */
  idIsProse?: boolean;
  /** Counts, versions, states. Set in the sans face. */
  meta?: string;
  pill?: React.ReactNode;
  selected: boolean;
  tabIndex: 0 | -1;
  /** Severity border and tint, when this row carries a finding. */
  accent?: { border: string; tint: string };
  /** Whether the glyph sits on the pipeline spine. The pinned whole-stack rows
   *  deliberately do not — that absence is what says they are about the folder
   *  rather than stations on the route. */
  node?: boolean;
  /** Promote the label. Set when the row carries a finding — weight is the
   *  third carrier of severity, after the left border and the count pill. */
  emphasis?: boolean;
  /** Words that carry meaning colour alone cannot: the severity level, the
   *  provenance, whether the live path wants this slot. Announced after the
   *  label and identifier so the row reads as a sentence, and rendered
   *  `sr-only` because none of them fits the 240px rail — see the note above. */
  notes?: (string | null | undefined)[];
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
      tabIndex={tabIndex}
      onClick={onSelect}
      className={cn(
        "flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-md px-2 text-left",
        "transition-colors duration-150 focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
        accent && cn("border-l-[3px] pl-1.5", accent.border, accent.tint),
        selected ? "bg-structure-08" : "hover:bg-structure-04"
      )}
    >
      {/* Both treatments occupy the same 20px advance, so every label in the
          rail starts on one left edge. They used to differ — a `size-5` node
          against a bare `size-3.5` glyph, both followed by the same gap — which
          put the eight slot labels 6px right of the three pinned ones and gave
          a single column two ragged left edges. The circled/bare distinction is
          the point (a station on the route versus the whole folder) and it
          survives intact; only the box it sits in was made consistent. */}
      <span className="relative z-10 grid size-5 shrink-0 place-items-center">
        {node ? (
          // The node punches through the spine, so the row reads as a station
          // on an ordered route. `bg-card` is what makes the line stop at the
          // ring.
          <span
            className={cn(
              "grid size-5 place-items-center rounded-full bg-card ring-1",
              selected ? "ring-structure-30" : "ring-structure-12"
            )}
          >
            <Icon aria-hidden className={cn("size-3", tone)} />
          </span>
        ) : (
          <Icon aria-hidden className={cn("size-3.5", tone)} />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "block truncate font-heading text-[13px] leading-4 tracking-[-0.005em]",
            selected || emphasis ? "font-semibold text-foreground" : "font-medium"
          )}
        >
          {label}
        </span>
        {/* Mono is only for the identifier. It earns that face because a forum
            guide says "rename dxgi.dll" and the two should match on screen; a
            count does not, and setting everything in mono drains the signal
            out of it. */}
        <span className="block truncate text-[11px] leading-[14px] text-muted-foreground">
          <span className={idIsProse ? "font-sans" : "font-mono tracking-[-0.01em]"}>{id}</span>
          {meta && <span className="font-sans"> &middot; {meta}</span>}
        </span>
      </span>
      {pill}
      {notes && notes.some(Boolean) && (
        <span className="sr-only">
          {notes
            .filter((note): note is string => Boolean(note))
            .map((note) => `${note}.`)
            .join(" ")}
        </span>
      )}
    </button>
  );
}

function SlotRow({
  slot,
  stack,
  selected,
  tabIndex,
  onSelect,
}: {
  slot: Slot;
  stack: ClientStack;
  selected: boolean;
  tabIndex: 0 | -1;
  onSelect: () => void;
}) {
  const level = slotLevel(slot, stack);
  const meta = LEVEL_META[level];
  const attention = findingsForSlot(slot, stack).filter((f) => f.level !== "info").length;
  const hasProblem = attention > 0;
  // The sub-line reads the need axis, so an empty row states *why* it is empty
  // rather than falling back to "nothing here" — which is a sentence about
  // absence, and on the direct path three of these rows are correctly empty.
  const sub = slotRailLine(slot, stack);
  const need = slotNeed(slot, stack);
  const source = SOURCE_META[SLOT_SOURCE[slot]];

  return (
    <RailRow
      Icon={SLOT_ICON[slot]}
      // Resting, the glyph carries provenance — whose hand is on this slot. It
      // only takes the severity colour when there is severity to report.
      tone={hasProblem ? meta.text : source.glyph}
      label={SLOT_LABEL[slot]}
      id={sub.id}
      idIsProse={sub.prose}
      meta={sub.meta}
      node
      emphasis={hasProblem}
      accent={hasProblem ? { border: meta.border, tint: meta.tint } : undefined}
      pill={
        attention > 0 ? (
          <InfoPill color={level === "danger" ? "red" : "amber"}>{attention}</InfoPill>
        ) : null
      }
      // Level, provenance, need — the three things this row says with colour or
      // with nothing at all. `NEED_META.required.word` is null, so the ordinary
      // row announces two notes rather than a redundant third.
      notes={[meta.label, source.word, NEED_META[need].word]}
      selected={selected}
      tabIndex={tabIndex}
      onSelect={onSelect}
    />
  );
}
