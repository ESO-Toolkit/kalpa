import { useCallback, useRef } from "react";

import { InfoPill } from "@/components/ui/info-pill";
import { cn } from "@/lib/utils";

import {
  LEVEL_META,
  SLOT_LABEL,
  SLOT_ORDER,
  findingsForSlot,
  slotLevel,
  slotSubLine,
} from "./slots";
import type { Slot } from "./slots";
import type { ClientStack } from "./types";

/**
 * The eight slots, as the panel's front door.
 *
 * This is the same list of rows the old pipeline rail drew, and it is still in
 * load order — but it is no longer a diagram. Each row is a place a choice can
 * be made, it carries its own status, and selecting it opens what is in it and
 * what could be instead. Three things had to change for that to work:
 *
 * - **Rows had to become obviously clickable.** They read as status cards
 *   before: no hover state, no pointer cursor, and eight buttons that looked
 *   like a report. The user's words on finding the actions inside them were
 *   "no idea i could click those".
 * - **The rail must not scroll.** Eight 40px rows fit in the body's 348px
 *   budget exactly, which is deliberate: the rail and the pane were once
 *   siblings in one scroller, and since the rail was much the taller, reaching
 *   a low row left the pane 1372px above the viewport. The pane now owns the
 *   only scroller.
 * - **The names had to be the user's.** "Injector" and "Super Resolution
 *   runtime" describe what a file does to a process. The filename beneath in
 *   `font-mono` is what keeps a forum guide greppable against this screen.
 *
 * The rail is deliberately always expanded. It was collapsed by default for a
 * healthy stack once, which reads well as a health report and fails completely
 * as a management panel: every action this feature adds lives inside these
 * rows, so collapsing them hid the entire feature behind a "Show layers"
 * button. There is no expand state here to get wrong a second time.
 */
export function SlotRail({
  stack,
  selected,
  onSelect,
}: {
  stack: ClientStack;
  selected: Slot | null;
  onSelect: (slot: Slot) => void;
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

  return (
    <div
      ref={railRef}
      role="listbox"
      aria-label="Stack slots"
      onKeyDown={handleKeyDown}
      className="w-[220px] shrink-0 space-y-1"
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
  const { Icon } = meta;
  const subLine = slotSubLine(slot, stack);
  const attention = findingsForSlot(slot, stack).filter((f) => f.level !== "info").length;

  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
      onClick={onSelect}
      className={cn(
        "flex h-10 w-full cursor-pointer items-center gap-2 rounded-lg border border-l-[3px] px-2 text-left",
        "transition-colors duration-150 focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
        selected
          ? "border-primary/30 border-l-primary bg-primary/[0.04]"
          : cn(meta.border, meta.tint, "hover:border-structure-10")
      )}
    >
      <Icon aria-hidden className={cn("size-3.5 shrink-0", meta.text)} />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-heading text-[12px] font-semibold leading-4">
          {SLOT_LABEL[slot]}
        </span>
        {/* The real filename, mono, beneath the friendly name — so a guide that
            says "rename dxgi.dll" still matches something on this screen. */}
        <span className="block truncate font-mono text-[10px] leading-3 text-muted-foreground">
          {subLine ?? "nothing here"}
        </span>
      </span>
      {attention > 0 && (
        <InfoPill color={level === "danger" ? "red" : "amber"}>{attention}</InfoPill>
      )}
      {/* The level word, for the themes where the colour carries nothing. */}
      <span className="sr-only">{meta.label}</span>
    </button>
  );
}
