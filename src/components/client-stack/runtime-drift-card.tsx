import { useCallback, useEffect, useState } from "react";
import {
  InfoIcon,
  AlertTriangleIcon,
  PowerOffIcon,
  RefreshCwIcon,
  ShieldAlertIcon,
  ShieldOffIcon,
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
  DriftState,
  ReapplyOutcome,
  RuntimeReport,
  RuntimeStatus,
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

/** Same color+icon+word discipline as `LEVEL_META` in `client-health.tsx`:
 *  color is never the only signal. Only the states worth reporting are here —
 *  `unchanged` never renders a row at all. */
const STATE_META: Record<
  Exclude<DriftState, "unchanged">,
  { label: string; Icon: typeof AlertTriangleIcon; text: string }
> = {
  drifted_recoverable: {
    label: "Reverted by an update",
    Icon: AlertTriangleIcon,
    text: "text-status-warning",
  },
  drifted_unrecoverable: {
    label: "Reverted by an update",
    Icon: ShieldAlertIcon,
    text: "text-status-warning",
  },
  missing: {
    label: "Missing",
    Icon: AlertTriangleIcon,
    text: "text-status-info",
  },
  parked: {
    label: "Switched off",
    Icon: PowerOffIcon,
    text: "text-muted-foreground",
  },
  changed_not_by_update: {
    label: "Changed",
    Icon: InfoIcon,
    text: "text-status-info",
  },
};

function RuntimeRow({
  runtime,
  onReapply,
  reapplying,
}: {
  runtime: RuntimeStatus;
  onReapply: (relativePath: string) => void;
  reapplying: boolean;
}) {
  const state = runtime.state;
  if (state === "unchanged") return null;
  const meta = STATE_META[state];

  return (
    <li className="space-y-2 rounded-lg bg-structure-03 p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <meta.Icon aria-hidden className={cn("size-4 shrink-0", meta.text)} />
          <span className="font-heading text-sm font-semibold">{runtime.relative_path}</span>
        </div>
        <InfoPill
          color={
            state === "parked"
              ? "muted"
              : state === "missing" || state === "changed_not_by_update"
                ? "sky"
                : "amber"
          }
        >
          {meta.label}
        </InfoPill>
      </div>

      {state === "drifted_recoverable" && (
        <>
          <p className="text-xs leading-relaxed text-muted-foreground">
            A game update put ESO&apos;s own build of this file back over the one you swapped in.
            Kalpa kept a copy of your bytes and can put it back.
          </p>
          {(runtime.current_version || runtime.kept_version) && (
            <div className="flex flex-wrap gap-4 text-xs">
              <span>
                <span className="text-muted-foreground">On disk now: </span>
                <span className="font-medium text-foreground">
                  {runtime.current_version ?? "unknown version"}
                </span>
              </span>
              <span>
                <span className="text-muted-foreground">Yours was: </span>
                <span className="font-medium text-foreground">
                  {runtime.kept_version ?? "unknown version"}
                </span>
              </span>
            </div>
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={reapplying}
            onClick={() => onReapply(runtime.relative_path)}
          >
            {reapplying ? <Spinner className="size-3.5" /> : <RefreshCwIcon />}
            {reapplying ? "Putting it back..." : "Put your copy back"}
          </Button>
        </>
      )}

      {state === "drifted_unrecoverable" && (
        <p className="text-xs leading-relaxed text-muted-foreground">
          A game update put ESO&apos;s own build of this file back, and Kalpa cannot put it back: it
          kept no copy of this file. The NVIDIA runtimes are not redistributable, so Kalpa never
          downloads one either. To make the next game update recoverable, put your own build back
          yourself, then use &quot;Stop managing&quot; in Kalpa&apos;s records and manage the stack
          again with &quot;keep copies&quot; turned on.
        </p>
      )}

      {state === "changed_not_by_update" && (
        <>
          <p className="text-xs leading-relaxed text-muted-foreground">
            This file has changed since Kalpa recorded it. ESO does not ship it, so no game update
            can have put its own build back — something replaced it deliberately, and the usual
            reason is that you installed a newer runtime.
          </p>
          {(runtime.current_version || runtime.kept_version) && (
            <div className="flex flex-wrap gap-4 text-xs">
              <span>
                <span className="text-muted-foreground">On disk now: </span>
                <span className="font-medium text-foreground">
                  {runtime.current_version ?? "unknown version"}
                </span>
              </span>
              <span>
                <span className="text-muted-foreground">Kalpa recorded: </span>
                <span className="font-medium text-foreground">
                  {runtime.kept_version ?? "unknown version"}
                </span>
              </span>
            </div>
          )}
          <p className="text-xs leading-relaxed text-muted-foreground">
            Kalpa offers nothing here on purpose: putting its older copy back would overwrite the
            file you are using, and these runtimes have no source it could fetch a replacement from.
            Manage the stack again if you want Kalpa to record the new one.
          </p>
        </>
      )}

      {state === "missing" && (
        <p className="text-xs leading-relaxed text-muted-foreground">
          This file is not in the folder at all.
        </p>
      )}

      {state === "parked" && (
        <p className="text-xs leading-relaxed text-muted-foreground">
          Not drift — the stack is switched off, so ESO&apos;s own file is supposed to be the one
          that loads here.
        </p>
      )}
    </li>
  );
}

/**
 * Whether a game update has put ESO's own runtimes back over the user's swap,
 * and — only when a copy was kept — the action that undoes it.
 */
export function RuntimeDriftCard({
  clientDir,
  onChanged,
  filePaths,
}: StackPanelProps & {
  /** File names shown on the surrounding stage, so the card reports only those. */
  filePaths: string[];
}) {
  const [report, setReport] = useState<RuntimeReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reapplyingPath, setReapplyingPath] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<ReapplyOutcome | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await invokeOrThrow<RuntimeReport>("inspect_client_runtimes", { clientDir });
      setReport(next);
    } catch (e) {
      setReport(null);
      setError(getTauriErrorMessage(e));
    } finally {
      setLoading(false);
    }
  }, [clientDir]);

  useEffect(() => {
    // Both the reset and `load` (which flips the loading flag before its
    // first await) are synchronous setState calls the rule flags. That is
    // the intended behaviour for an on-mount/on-clientDir-change fetch — see
    // the same pattern in `client-health.tsx`'s `detect` effect.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setOutcome(null);
    void load();
  }, [load]);

  const handleReapply = useCallback(
    async (relativePath: string) => {
      setReapplyingPath(relativePath);
      setError(null);
      setOutcome(null);
      try {
        await approveClientWrites(clientDir);
        const result = await invokeOrThrow<ReapplyOutcome>("reapply_client_runtimes", {
          clientDir,
          relativePaths: [relativePath],
        });
        setOutcome(result);
        await onChanged();
        await load();
      } catch (e) {
        setError(getTauriErrorMessage(e));
      } finally {
        setReapplyingPath(null);
      }
    },
    [clientDir, load, onChanged]
  );

  if (loading && !report) {
    return (
      <GlassPanel
        variant="subtle"
        className="flex items-center gap-2 p-3 text-xs text-muted-foreground"
      >
        <Spinner className="size-3.5" />
        Checking for runtime drift...
      </GlassPanel>
    );
  }

  if (error && !report) {
    return (
      <GlassPanel variant="subtle" className="p-3 text-xs text-status-danger" role="alert">
        {error}
      </GlassPanel>
    );
  }

  if (!report) return null;

  const matching = report.runtimes.filter((r) => filePaths.includes(r.relative_path));
  const reportable = matching.filter((r) => r.state !== "unchanged");
  if (reportable.length === 0) return null;

  return (
    <GlassPanel variant="subtle" className="space-y-3 p-4">
      <div className="flex items-center gap-2">
        <ShieldOffIcon aria-hidden className="size-4 text-status-warning" />
        <SectionHeader>Runtime drift</SectionHeader>
      </div>

      <ul className="space-y-2">
        {reportable.map((runtime) => (
          <RuntimeRow
            key={runtime.relative_path}
            runtime={runtime}
            reapplying={reapplyingPath === runtime.relative_path}
            onReapply={(path) => void handleReapply(path)}
          />
        ))}
      </ul>

      {error && (
        <p className="text-xs text-status-danger" role="alert">
          {error}
        </p>
      )}

      {outcome && (
        <div className="space-y-1 border-t border-structure-06 pt-2 text-xs">
          {outcome.restored.length > 0 && (
            <p className="text-status-success">
              Restored {outcome.restored.length} file{outcome.restored.length === 1 ? "" : "s"}.
            </p>
          )}
          {outcome.skipped.length > 0 && (
            <div className="text-status-warning">
              <p>Skipped:</p>
              <ul className="ml-4 list-disc">
                {outcome.skipped.map((line, i) => (
                  <li key={i}>{line}</li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </GlassPanel>
  );
}
