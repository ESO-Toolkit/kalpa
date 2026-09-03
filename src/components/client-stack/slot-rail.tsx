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
    <div ref={railRef} onKeyDown={handleKeyDown} className="flex w-[200px] shrink-0 flex-col">
      <div
        role="listbox"
        aria-label="Stack slots"
        className="min-h-0 flex-1 space-y-0.5 overflow-y-auto pr-1"
      >
        {SLOT_ORDER.map((slot) => (
          <SlotRow
            key={slot}
            slot={slot}
            stack={stack}
            selected={selected === slot}
            onSelect={() => onSelect(slot)}
          />
        ))}
      </div>

      <div className="my-2 shrink-0 border-t border-structure-06" />

      <div role="listbox" aria-label="Whole stack" className="shrink-0 space-y-0.5 pr-1">
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
          sub={POWER_COPY[power].state}
          selected={selected === "power"}
          onSelect={() => onSelect("power")}
        />
        <RailRow
          Icon={HardDriveIcon}
          tone="text-muted-foreground"
          label="Kalpa's records"
          // The count is withheld until the inventory has loaded: "0 files" for
          // an adopted stack is a false statement, not a pending one.
          sub={
            isManaged
              ? trackedCount === null
                ? "Managed"
                : `Managed · ${trackedCount} file${trackedCount === 1 ? "" : "s"}`
              : "Not managed yet"
          }
          pill={isManaged ? null : <InfoPill color="gold">!</InfoPill>}
          selected={selected === "records"}
          onSelect={() => onSelect("records")}
        />
        <RailRow
          Icon={ScrollTextIcon}
          tone={logCount > 0 ? "text-status-warning" : "text-muted-foreground"}
          label="Log check"
          sub={
            logCount > 0
              ? `${logCount} line${logCount === 1 ? "" : "s"} matched`
              : "Nothing matched"
          }
          selected={selected === "logs"}
          onSelect={() => onSelect("logs")}
        />
      </div>
    </div>
  );
}

/** Shared row chrome. 32px: a 16px label line over a 12px mono sub-line. */
function RailRow({
  Icon,
  tone,
  label,
  sub,
  pill,
  selected,
  accent,
  onSelect,
}: {
  Icon: typeof LayersIcon;
  tone: string;
  label: string;
  sub: string;
  pill?: React.ReactNode;
  selected: boolean;
  /** Border/tint for a slot carrying a finding. Absent means neutral. */
  accent?: { border: string; tint: string };
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
      onClick={onSelect}
      className={cn(
        "flex h-8 w-full cursor-pointer items-center gap-2 rounded-lg border border-l-[3px] px-2 text-left",
        "transition-colors duration-150 focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
        selected
          ? "border-primary/30 border-l-primary bg-primary/[0.04]"
          : accent
            ? cn(accent.border, accent.tint, "hover:border-structure-10")
            : "border-structure-06 border-l-structure-10 bg-structure-02 hover:border-structure-10"
      )}
    >
      <Icon aria-hidden className={cn("size-3.5 shrink-0", tone)} />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-heading text-[12px] font-semibold leading-4">
          {label}
        </span>
        {/* The real filename, mono, beneath the friendly name — so a guide that
            says "rename dxgi.dll" still matches something on this screen. */}
        <span className="block truncate font-mono text-[10px] leading-3 text-muted-foreground">
          {sub}
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

  return (
    <RailRow
      Icon={SLOT_ICON[slot]}
      // The glyph says what the slot is; it only takes the level's colour when
      // there is a level worth reporting.
      tone={hasProblem ? meta.text : "text-muted-foreground"}
      label={SLOT_LABEL[slot]}
      sub={slotSubLine(slot, stack) ?? "nothing here"}
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
