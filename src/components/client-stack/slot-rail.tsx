import { useCallback, useRef } from "react";
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
  POWER_COPY,
  SLOT_LABEL,
  SLOT_ORDER,
  SLOT_SOURCE,
  SOURCE_META,
  findingsForSlot,
  powerState,
  slotLevel,
  slotSubLine,
} from "./slots";
import type { Slot } from "./slots";
import type { ClientStack } from "./types";

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
}: {
  stack: ClientStack;
  /** A slot, or one of the whole-stack views, or nothing. */
  selected: Slot | StackView | null;
  onSelect: (key: Slot | StackView) => void;
  isManaged: boolean;
  trackedCount: number | null;
  logCount: number;
}) {
  const railRef = useRef<HTMLDivElement>(null);

  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const container = railRef.current;
    if (!container) return;
    const items = Array.from(container.querySelectorAll<HTMLElement>('[role="option"]'));
    if (items.length === 0) return;
    e.preventDefault();
    const idx = items.indexOf(document.activeElement as HTMLElement);
    const nextIdx =
      idx === -1
        ? 0
        : e.key === "ArrowDown"
          ? Math.min(idx + 1, items.length - 1)
          : Math.max(idx - 1, 0);
    items[nextIdx]?.focus();
  }, []);

  const power = powerState(stack);

  return (
    // 240px, not 200. At 200 the sub-line had 152px to work with and three of
    // the eleven rows truncated at once — `d3dcompiler_4…`, `renodx-dlss.addon64
    // · dl…`, `ReShadePreset.ini · 0 tech…`. A file name that ends in an ellipsis
    // is not an identifier any more, and this rail's whole job is to show
    // identifiers. 240 gives the sub-line 192px and leaves only the genuinely
    // two-file add-ons row clipped; the pane gives up 40px of a column that was
    // already ending in empty space.
    <div ref={railRef} onKeyDown={handleKeyDown} className="flex w-[240px] shrink-0 flex-col">
      {/* The spine: a hairline behind the glyph nodes, so eight rows read as an
          ordered route rather than eight equal options — which is what the
          stack actually is. ReShade loads first, the preset picks the
          motion-vector provider, the feed consumes it. It says "ordered"
          without redrawing the pipeline diagram the redesign retired.

          Grouping is rhythm, not labels: binaries the game loads, then content
          ReShade runs, then configuration. Three more headings in a 240px
          column would cost more than they explain. */}
      <div role="listbox" aria-label="Stack slots" className="min-h-0 flex-1 overflow-y-auto pr-1">
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
                onSelect={() => onSelect(slot)}
              />
            </div>
          ))}
        </div>
      </div>

      <div className="my-3 shrink-0 border-t border-structure-06" />

      <div role="listbox" aria-label="Whole stack" className="shrink-0 space-y-1 pr-1">
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
          id={POWER_COPY[power].state}
          emphasis={power !== "on" && power !== "off"}
          selected={selected === "power"}
          onSelect={() => onSelect("power")}
        />
        <RailRow
          Icon={HardDriveIcon}
          tone="text-muted-foreground"
          label="Kalpa's records"
          // The count is withheld until the inventory has loaded: "0 files" for
          // an adopted stack is a false statement, not a pending one.
          id={isManaged ? "Managed" : "Not managed yet"}
          meta={
            isManaged && trackedCount !== null
              ? `${trackedCount} file${trackedCount === 1 ? "" : "s"}`
              : undefined
          }
          // Amber and a word, not a gold "!". Gold means Kalpa's own hand in
          // this panel, and a bare glyph would make colour the only signal.
          pill={isManaged ? null : <InfoPill color="amber">Not managed</InfoPill>}
          selected={selected === "records"}
          onSelect={() => onSelect("records")}
        />
        <RailRow
          Icon={ScrollTextIcon}
          tone={logCount > 0 ? "text-status-warning" : "text-muted-foreground"}
          label="Log check"
          id={
            logCount > 0
              ? `${logCount} line${logCount === 1 ? "" : "s"} matched`
              : "Nothing matched"
          }
          emphasis={logCount > 0}
          selected={selected === "logs"}
          onSelect={() => onSelect("logs")}
        />
      </div>
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
 */
function RailRow({
  Icon,
  tone,
  label,
  id,
  meta,
  pill,
  selected,
  accent,
  node,
  emphasis,
  onSelect,
}: {
  Icon: typeof LayersIcon;
  tone: string;
  label: string;
  /** A string that exists on disk or in an INI file. Set in mono. */
  id: string;
  /** Counts, versions, states. Set in the sans face. */
  meta?: string;
  pill?: React.ReactNode;
  selected: boolean;
  /** Severity border and tint, when this row carries a finding. */
  accent?: { border: string; tint: string };
  /** Whether the glyph sits on the pipeline spine. The pinned whole-stack rows
   *  deliberately do not — that absence is what says they are about the folder
   *  rather than stations on the route. */
  node?: boolean;
  /** Promote the label. Set when the row carries a finding — weight is the
   *  third carrier of severity, after the left border and the count pill. */
  emphasis?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
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
          <span className="font-mono tracking-[-0.01em]">{id}</span>
          {meta && <span className="font-sans"> &middot; {meta}</span>}
        </span>
      </span>
      {pill}
    </button>
  );
}

function SlotRow({
  slot,
  stack,
  selected,
  onSelect,
}: {
  slot: Slot;
  stack: ClientStack;
  selected: boolean;
  onSelect: () => void;
}) {
  const level = slotLevel(slot, stack);
  const meta = LEVEL_META[level];
  const attention = findingsForSlot(slot, stack).filter((f) => f.level !== "info").length;
  const hasProblem = attention > 0;
  const sub = slotSubLine(slot, stack);
  const source = SOURCE_META[SLOT_SOURCE[slot]];

  return (
    <RailRow
      Icon={SLOT_ICON[slot]}
      // Resting, the glyph carries provenance — whose hand is on this slot. It
      // only takes the severity colour when there is severity to report.
      tone={hasProblem ? meta.text : source.glyph}
      label={SLOT_LABEL[slot]}
      id={sub?.id ?? "nothing here"}
      meta={sub?.meta}
      node
      emphasis={hasProblem}
      accent={hasProblem ? { border: meta.border, tint: meta.tint } : undefined}
      pill={
        <>
          {attention > 0 && (
            <InfoPill color={level === "danger" ? "red" : "amber"}>{attention}</InfoPill>
          )}
          {/* The level word, for the themes where colour carries nothing. */}
          <span className="sr-only">{meta.label}</span>
        </>
      }
      selected={selected}
      onSelect={onSelect}
    />
  );
}
