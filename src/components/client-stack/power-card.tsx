import { useCallback, useState } from "react";
import {
  ArchiveIcon,
  CircleCheckIcon,
  MinusCircleIcon,
  PowerIcon,
  PowerOffIcon,
  ShieldAlertIcon,
} from "lucide-react";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { Button } from "@/components/ui/button";
import { approveClientWrites } from "@/components/client-stack/approve";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import type { StackPanelProps } from "@/components/client-stack/panel-props";
import type {
  FileOpOutcome,
  PlannedOp,
  ToggleOpKind,
  TogglePlan,
} from "@/components/client-stack/types";

const Spinner = ({ className }: { className?: string }) => (
  <span
    aria-hidden
    className={cn(
      "inline-block size-4 animate-spin rounded-full border-2 border-structure-10 border-t-primary",
      className
    )}
  />
);

/** Icon per operation kind. `leave_in_place` gets a quiet, reassuring mark;
 *  every other kind gets something that reads as "a file moved". */
const OP_ICON: Record<ToggleOpKind, typeof PowerIcon> = {
  park: ArchiveIcon,
  restore_original: ArchiveIcon,
  unpark: ArchiveIcon,
  remove_restored: MinusCircleIcon,
  leave_in_place: CircleCheckIcon,
};

function PlannedOpRow({ op }: { op: PlannedOp }) {
  const Icon = OP_ICON[op.kind];
  const quiet = op.kind === "leave_in_place";
  return (
    <li
      className={cn(
        "flex items-start gap-2 rounded-lg p-2",
        quiet ? "text-muted-foreground" : "bg-structure-03"
      )}
    >
      <Icon
        aria-hidden
        className={cn("mt-0.5 size-3.5 shrink-0", quiet ? "text-muted-foreground" : "text-primary")}
      />
      <div className="min-w-0 flex-1">
        <p className={cn("text-xs", quiet ? "text-muted-foreground" : "text-foreground")}>
          {op.summary}
        </p>
        <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{op.detail}</p>
      </div>
    </li>
  );
}

/**
 * Switch the whole stack off, or back on.
 *
 * The confirmation is the plan: one line per operation, in the order they run,
 * computed by the backend from what is actually on disk. The confirm button
 * stays disabled until that plan has loaded — a user cannot approve a list they
 * have not been shown.
 */
export function StackPowerCard({ clientDir, stack, onChanged }: StackPanelProps) {
  const [plan, setPlan] = useState<TogglePlan | null>(null);
  const [planLoading, setPlanLoading] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<FileOpOutcome | null>(null);

  const handleRequest = useCallback(async () => {
    setError(null);
    setOutcome(null);
    setPlanLoading(true);
    try {
      const next = await invokeOrThrow<TogglePlan>("plan_client_toggle", { clientDir });
      setPlan(next);
    } catch (e) {
      setPlan(null);
      setError(getTauriErrorMessage(e));
    } finally {
      setPlanLoading(false);
    }
  }, [clientDir]);

  const handleCancel = useCallback(() => {
    setPlan(null);
    setError(null);
  }, []);

  const handleConfirm = useCallback(async () => {
    if (!plan) return;
    setApplying(true);
    setError(null);
    try {
      await approveClientWrites(clientDir);
      const result = await invokeOrThrow<FileOpOutcome>("apply_client_toggle", {
        clientDir,
        expected: plan.action,
      });
      setOutcome(result);
      setPlan(null);
      await onChanged();
    } catch (e) {
      setError(getTauriErrorMessage(e));
    } finally {
      setApplying(false);
    }
  }, [clientDir, onChanged, plan]);

  if (stack.is_empty) return null;

  const disabled = stack.is_disabled;

  return (
    <GlassPanel variant="default" className="space-y-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <SectionHeader>Stack power</SectionHeader>
          <div className="mt-1 flex items-center gap-2">
            {disabled ? (
              <PowerOffIcon aria-hidden className="size-4 text-muted-foreground" />
            ) : (
              <PowerIcon aria-hidden className="size-4 text-status-success" />
            )}
            <span className="font-heading text-sm font-semibold">
              {disabled ? "Switched off" : "Switched on"}
            </span>
            <InfoPill color={disabled ? "muted" : "emerald"}>
              {disabled ? "Stock ESO" : "Modded"}
            </InfoPill>
          </div>
        </div>

        {!plan && (
          <Button
            variant="outline"
            size="sm"
            disabled={planLoading}
            onClick={() => void handleRequest()}
          >
            {planLoading ? (
              <Spinner className="size-3.5" />
            ) : disabled ? (
              <PowerIcon />
            ) : (
              <PowerOffIcon />
            )}
            {planLoading ? "Working out the plan..." : disabled ? "Switch on" : "Switch off"}
          </Button>
        )}
      </div>

      <p className="text-xs leading-relaxed text-muted-foreground">
        {disabled
          ? "The stack is parked. ESO is running with nothing from it loaded — the files are still in this folder, untouched."
          : "Switching off puts ESO back to stock: the injector is parked and the files ESO loads itself are put back to your own originals. Nothing is deleted, and everything else in the stack is left exactly where it is. ESO reads this at its next launch, not immediately."}
      </p>

      {plan && (
        <div className="space-y-2 border-t border-structure-06 pt-3">
          {plan.blockers.length > 0 && (
            <GlassPanel
              variant="subtle"
              className="flex items-start gap-2 border-status-danger/20 p-3 text-xs"
              role="alert"
            >
              <ShieldAlertIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-status-danger" />
              <ul className="space-y-1 text-status-danger">
                {plan.blockers.map((b, i) => (
                  <li key={i}>{b}</li>
                ))}
              </ul>
            </GlassPanel>
          )}

          <ul className="space-y-1">
            {plan.operations.map((op, i) => (
              <PlannedOpRow key={`${op.kind}-${op.file_name}-${i}`} op={op} />
            ))}
          </ul>

          <div className="flex items-center gap-2 pt-1">
            <Button
              size="sm"
              variant={plan.action === "disable" ? "destructive" : "default"}
              disabled={applying || plan.blockers.length > 0}
              onClick={() => void handleConfirm()}
            >
              {applying ? <Spinner className="size-3.5" /> : <PowerIcon />}
              {applying
                ? "Applying..."
                : plan.action === "disable"
                  ? "Confirm switch off"
                  : "Confirm switch on"}
            </Button>
            <Button size="sm" variant="outline" disabled={applying} onClick={handleCancel}>
              Cancel
            </Button>
          </div>
        </div>
      )}

      {error && (
        <p className="text-xs text-status-danger" role="alert">
          {error}
        </p>
      )}

      {outcome && (
        <GlassPanel variant="subtle" className="space-y-1 p-3 text-xs" role="status">
          <p className="text-status-success">
            Applied {outcome.applied.length} step{outcome.applied.length === 1 ? "" : "s"}.
          </p>
          {outcome.skipped.length > 0 && (
            <p className="text-muted-foreground">
              Skipped {outcome.skipped.length} step{outcome.skipped.length === 1 ? "" : "s"}:
              already in the state they needed to be in.
            </p>
          )}
          {outcome.preserved.length > 0 && (
            <div className="space-y-1 text-status-warning">
              <p>
                Moved {outcome.preserved.length} file
                {outcome.preserved.length === 1 ? "" : "s"} out of the game folder into a backup
                rather than deleting {outcome.preserved.length === 1 ? "it" : "them"} — the bytes
                did not match what Kalpa expected, so it would not assume they were its own.
              </p>
              {outcome.preserved.map((line) => (
                <p key={line} className="font-mono text-[11px]">
                  {line}
                </p>
              ))}
              <p className="text-muted-foreground">
                Those backups are not tied to a file Kalpa tracks, so its own cleanup may reclaim
                them later. Copy anything you want to keep out of that folder.
              </p>
            </div>
          )}
        </GlassPanel>
      )}
    </GlassPanel>
  );
}
