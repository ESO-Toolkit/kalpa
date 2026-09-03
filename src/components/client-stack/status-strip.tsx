import { AlertTriangleIcon, PowerIcon, PowerOffIcon, ScrollTextIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { InfoPill } from "@/components/ui/info-pill";
import { cn } from "@/lib/utils";

import { POWER_COPY, powerState } from "./slots";
import type { ClientStack } from "./types";

/**
 * Whole-stack state, in one 36px row above the slots.
 *
 * Everything here is true of the stack rather than of any one slot, which is
 * exactly why it is not a rail row: is it on, is Kalpa managing it, how much
 * needs attention, and did the logs say anything. Putting these in the strip is
 * what lets every rail row be a *choice* and nothing else.
 *
 * `stack-disabled` has no slot for this reason. The strip is that finding,
 * stated with the action attached — which is strictly more useful than a
 * finding row describing the same state and then sending the user elsewhere to
 * act on it.
 *
 * **There is deliberately no on/off toggle.** "Partly switched off" is a real,
 * reachable state — a batch that died part way, or a `.kalpa-off` file the user
 * renamed by hand — and a boolean control would have to round it to on or off,
 * both of which lie. The button is a verb that opens a plan, never a switch
 * that flips.
 */
export function StatusStrip({
  stack,
  attention,
  isManaged,
  logCount,
  onOpenPower,
  onOpenAdoption,
  onOpenLogs,
}: {
  stack: ClientStack;
  /** Findings above `info`, across every slot. */
  attention: number;
  isManaged: boolean;
  logCount: number;
  onOpenPower: () => void;
  onOpenAdoption: () => void;
  onOpenLogs: () => void;
}) {
  const power = powerState(stack);
  const copy = POWER_COPY[power];
  const PowerGlyph =
    power === "on" ? PowerIcon : power === "off" ? PowerOffIcon : AlertTriangleIcon;
  const glyphTone =
    power === "on"
      ? "text-status-success"
      : power === "off"
        ? "text-muted-foreground"
        : "text-status-warning";

  return (
    <div className="flex h-9 shrink-0 items-center gap-2 rounded-lg border border-structure-06 bg-structure-02 px-2.5">
      <PowerGlyph aria-hidden className={cn("size-4 shrink-0", glyphTone)} />
      <span className="truncate font-heading text-[12px] font-semibold">{copy.state}</span>
      {power === "partly_off" && (
        <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
          {stack.parked.length} parked
        </span>
      )}

      <Button variant="outline" size="xs" className="shrink-0" onClick={onOpenPower}>
        {copy.action}
      </Button>

      <div className="ml-auto flex shrink-0 items-center gap-2">
        {logCount > 0 && (
          <Button variant="ghost" size="xs" onClick={onOpenLogs}>
            <ScrollTextIcon className="text-status-warning" />
            {logCount} log line{logCount === 1 ? "" : "s"}
          </Button>
        )}

        <InfoPill color={attention > 0 ? "amber" : "emerald"}>
          {attention > 0 ? `${attention} need attention` : "Everything agrees"}
        </InfoPill>

        {isManaged ? (
          <InfoPill color="emerald">Managed</InfoPill>
        ) : (
          <Button variant="outline" size="xs" onClick={onOpenAdoption}>
            <InfoPill color="gold" className="mr-1">
              Not managed
            </InfoPill>
            Manage…
          </Button>
        )}
      </div>
    </div>
  );
}
