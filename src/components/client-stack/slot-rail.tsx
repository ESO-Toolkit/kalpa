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
 * more labels in a 200px column would cost more than they explain, and the
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
 * a button below meant paying 129px of chrome to say what three 32px rows say.
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
    <div
      ref={railRef}
      role="listbox"
      aria-label="Graphics stack views"
      onKeyDown={handleKeyDown}
      className="flex w-[200px] shrink-0 flex-col"
    >
      {/* The spine: a hairline behind the glyph nodes, so eight rows read as an
          ordered route rather than eight equal options — which is what the
          stack actually is. ReShade loads first, the preset picks the
          motion-vector provider, the feed consumes it. It says "ordered"
          without redrawing the pipeline diagram the redesign retired.

          Grouping is rhythm, not labels: binaries the game loads, then content
          ReShade runs, then configuration. Three more headings in a 200px
          column would cost more than they explain. */}
      <div role="presentation" className="min-h-0 flex-1 overflow-y-auto pr-1">
        {/* The spine hangs off this inner wrapper rather than the scroll
            container. On the container it stretched to the full flex height and
            ran on past the last node into empty space, which reads as a route
            that goes somewhere Kalpa is not showing you. Here it spans exactly
            first node to last. */}
        <div className="relative before:absolute before:top-4 before:bottom-4 before:left-[17px] before:w-px before:bg-structure-10">
          {SLOT_ORDER.map((slot, i) => (
            <div key={slot} className={i > 0 && SLOT_GROUP_START.has(slot) ? "mt-2" : "mt-0.5"}>
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

      <div className="my-2 shrink-0 border-t border-structure-06" />

      <div role="presentation" className="shrink-0 space-y-0.5 pr-1">
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
        <RailRow
          Icon={ScrollTextIcon}
          tone={logCount > 0 ? "text-status-warning" : "text-muted-foreground"}
          label="Log check"
          id={
            logCount > 0
              ? `${logCount} line${logCount === 1 ? "" : "s"} matched`
              : "Nothing matched"
          }
          selected={selected === "logs"}
          tabIndex={tabStop === "logs" ? 0 : -1}
          onSelect={() => onSelect("logs")}
        />
      </div>
    </div>
  );
}

/**
 * Shared row chrome. 32px: a 16px label line over a 12px sub-line.
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
 */
function RailRow({
  Icon,
  tone,
  label,
  id,
  meta,
  pill,
  selected,
  tabIndex,
  accent,
  node,
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
  tabIndex: 0 | -1;
  /** Severity border and tint, when this row carries a finding. */
  accent?: { border: string; tint: string };
  /** Whether the glyph sits on the pipeline spine. The pinned whole-stack rows
   *  deliberately do not — that absence is what says they are about the folder
   *  rather than stations on the route. */
  node?: boolean;
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
        "flex h-8 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left",
        "transition-colors duration-150 focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
        accent && cn("border-l-[3px] pl-1.5", accent.border, accent.tint),
        selected ? "bg-structure-08" : "hover:bg-structure-04"
      )}
    >
      {node ? (
        // The node punches through the spine, so the row reads as a station on
        // an ordered route. `bg-card` is what makes the line stop at the ring.
        <span
          className={cn(
            "relative z-10 grid size-5 shrink-0 place-items-center rounded-full bg-card ring-1",
            selected ? "ring-structure-30" : "ring-structure-12"
          )}
        >
          <Icon aria-hidden className={cn("size-3", tone)} />
        </span>
      ) : (
        <Icon aria-hidden className={cn("size-3.5 shrink-0", tone)} />
      )}
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "block truncate font-heading text-[13px] leading-4",
            selected ? "font-semibold text-foreground" : "font-medium"
          )}
        >
          {label}
        </span>
        {/* Mono is only for the identifier. It earns that face because a forum
            guide says "rename dxgi.dll" and the two should match on screen; a
            count does not, and setting everything in mono drains the signal
            out of it. */}
        <span className="block truncate text-[11px] leading-3 text-muted-foreground">
          <span className="font-mono">{id}</span>
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
      tabIndex={tabIndex}
      onSelect={onSelect}
    />
  );
}
