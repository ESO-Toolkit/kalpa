import { useState, useEffect, useCallback } from "react";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Alert } from "@/components/ui/alert";
import { SectionHeader } from "@/components/ui/section-header";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";
import type {
  PreconditionResult,
  SnapshotManifest,
  DryRunResult,
  SafeMigrationResult,
} from "@/types";

type MigrationPhase =
  | "preconditions"
  | "snapshot"
  | "dry-run"
  | "confirm"
  | "migrating"
  | "complete";

interface MigrationWizardProps {
  addonsPath: string;
  onClose: () => void;
  onRefresh: () => void;
}

export function MigrationWizard({ addonsPath, onClose, onRefresh }: MigrationWizardProps) {
  const [phase, setPhase] = useState<MigrationPhase>("preconditions");
  const [preconditions, setPreconditions] = useState<PreconditionResult | null>(null);
  const [snapshot, setSnapshot] = useState<SnapshotManifest | null>(null);
  const [dryRun, setDryRun] = useState<DryRunResult | null>(null);
  const [result, setResult] = useState<SafeMigrationResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [includeAddons, setIncludeAddons] = useState(true);

  const checkPreconditions = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await invokeOrThrow<PreconditionResult>("migration_check_preconditions", {
        addonsPath,
      });
      setPreconditions(result);
    } catch (e) {
      setError(getTauriErrorMessage(e));
    } finally {
      setLoading(false);
    }
  }, [addonsPath]);

  useEffect(() => {
    checkPreconditions();
  }, [checkPreconditions]);

  const handleCreateSnapshot = async () => {
    setLoading(true);
    setError(null);
    try {
      // Also backup Minion config (read-only copy)
      try {
        await invokeOrThrow<number>("backup_minion_config", { addonsPath });
      } catch {
        // Non-fatal — Minion config backup is optional
      }

      const snap = await invokeOrThrow<SnapshotManifest>("migration_create_snapshot", {
        addonsPath,
        includeAddons,
      });
      setSnapshot(snap);
      toast.success(`Snapshot created: ${snap.fileCount} files (${formatBytes(snap.totalSize)})`);
      setPhase("dry-run");
      // Automatically run dry-run — don't reset loading between the two operations
      // to prevent a brief flash where buttons could be clicked
      await runDryRun();
    } catch (e) {
      setError(getTauriErrorMessage(e));
      setLoading(false);
    }
  };

  const runDryRun = async () => {
    setLoading(true);
    setError(null);
    try {
      const dr = await invokeOrThrow<DryRunResult>("migration_dry_run", { addonsPath });
      setDryRun(dr);
      setPhase("confirm");
    } catch (e) {
      setError(getTauriErrorMessage(e));
      setPhase("dry-run");
    } finally {
      setLoading(false);
    }
  };

  const handleExecute = async () => {
    setPhase("migrating");
    setLoading(true);
    setError(null);
    try {
      const res = await invokeOrThrow<SafeMigrationResult>("migration_execute", { addonsPath });
      setResult(res);
      setPhase("complete");
      onRefresh();
    } catch (e) {
      setError(getTauriErrorMessage(e));
      setPhase("confirm");
    } finally {
      setLoading(false);
    }
  };

  const canProceedFromPreconditions =
    preconditions && preconditions.minionFound && preconditions.addonsPathValid;

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Safe Minion Migration</DialogTitle>
        </DialogHeader>

        <PhaseIndicator current={phase} />

        <div className="min-h-[200px]">
          {error && (
            <Alert variant="destructive" className="mb-3">
              {error}
            </Alert>
          )}

          {phase === "preconditions" && (
            <PreconditionsPhase
              preconditions={preconditions}
              loading={loading}
              canProceed={!!canProceedFromPreconditions}
              onProceed={() => setPhase("snapshot")}
              onRetry={checkPreconditions}
            />
          )}

          {phase === "snapshot" && (
            <SnapshotPhase
              loading={loading}
              includeAddons={includeAddons}
              onIncludeAddonsChange={setIncludeAddons}
              onCreateSnapshot={handleCreateSnapshot}
            />
          )}

          {(phase === "dry-run" || phase === "confirm") && (
            <DryRunPhase
              dryRun={dryRun}
              loading={loading}
              confirmed={phase === "confirm"}
              onExecute={handleExecute}
            />
          )}

          {phase === "migrating" && (
            <div className="flex flex-col items-center justify-center py-8">
              <div className="text-sm text-muted-foreground">Importing metadata...</div>
              <p className="mt-2 text-xs text-muted-foreground">
                Only writing kalpa.json. No addon files or SavedVariables will be modified.
              </p>
            </div>
          )}

          {phase === "complete" && result && <CompletePhase result={result} snapshot={snapshot} />}
        </div>

        <DialogFooter>
          {phase === "complete" ? (
            <Button onClick={onClose}>Done</Button>
          ) : (
            <Button variant="outline" onClick={onClose} disabled={phase === "migrating"}>
              Cancel
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function PhaseIndicator({ current }: { current: MigrationPhase }) {
  const phases: { key: MigrationPhase; label: string }[] = [
    { key: "preconditions", label: "Check" },
    { key: "snapshot", label: "Backup" },
    { key: "dry-run", label: "Preview" },
    { key: "migrating", label: "Import" },
    { key: "complete", label: "Done" },
  ];

  const currentIndex = phases.findIndex(
    (p) => p.key === current || (current === "confirm" && p.key === "dry-run")
  );

  return (
    <div className="flex items-center gap-1 mb-2">
      {phases.map((p, i) => (
        <div key={p.key} className="flex items-center gap-1">
          <div
            className={`flex h-6 w-6 items-center justify-center rounded-full text-xs font-medium ${
              i < currentIndex
                ? "bg-emerald-500/20 text-emerald-400"
                : i === currentIndex
                  ? "bg-[#4dc2e6]/20 text-[#4dc2e6]"
                  : "bg-white/[0.06] text-muted-foreground"
            }`}
          >
            {i < currentIndex ? "\u2713" : i + 1}
          </div>
          <span
            className={`text-xs ${i === currentIndex ? "text-white" : "text-muted-foreground"}`}
          >
            {p.label}
          </span>
          {i < phases.length - 1 && <div className="mx-1 h-px w-4 bg-white/[0.1]" />}
        </div>
      ))}
    </div>
  );
}

function PreconditionsPhase({
  preconditions,
  loading,
  canProceed,
  onProceed,
  onRetry,
}: {
  preconditions: PreconditionResult | null;
  loading: boolean;
  canProceed: boolean;
  onProceed: () => void;
  onRetry: () => void;
}) {
  if (loading || !preconditions) {
    return <div className="py-8 text-center text-sm text-muted-foreground">Checking...</div>;
  }

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        Kalpa will import addon tracking data from Minion. Your addons, SavedVariables, and game
        settings will not be modified.
      </p>

      <div className="space-y-1.5">
        <CheckItem ok={preconditions.minionFound} label="Minion installation detected" />
        <CheckItem ok={preconditions.addonsPathValid} label="AddOns folder accessible" />
        <CheckItem ok={preconditions.savedVariablesExists} label="SavedVariables folder found" />
        <CheckItem ok={!preconditions.esoRunning} label="ESO is not running" />
        <CheckItem ok={!preconditions.minionRunning} label="Minion is not running" warn />
      </div>

      {preconditions.warnings.length > 0 && (
        <div className="space-y-1">
          {preconditions.warnings.map((w, i) => (
            <div
              key={i}
              className="rounded-lg border border-amber-400/20 bg-amber-400/[0.04] px-3 py-1.5 text-xs text-amber-400"
            >
              {w}
            </div>
          ))}
        </div>
      )}

      <div className="flex gap-2 pt-1">
        <Button onClick={onProceed} disabled={!canProceed}>
          Continue to Backup
        </Button>
        <Button variant="outline" size="sm" onClick={onRetry}>
          Re-check
        </Button>
      </div>
    </div>
  );
}

function CheckItem({ ok, label, warn }: { ok: boolean; label: string; warn?: boolean }) {
  return (
    <div className="flex items-center gap-2 text-sm">
      <span className={ok ? "text-emerald-400" : warn ? "text-amber-400" : "text-red-400"}>
        {ok ? "\u2713" : warn ? "!" : "\u2717"}
      </span>
      <span className={ok ? "text-white/80" : warn ? "text-amber-400" : "text-red-400"}>
        {label}
      </span>
    </div>
  );
}

function SnapshotPhase({
  loading,
  includeAddons,
  onIncludeAddonsChange,
  onCreateSnapshot,
}: {
  loading: boolean;
  includeAddons: boolean;
  onIncludeAddonsChange: (v: boolean) => void;
  onCreateSnapshot: () => void;
}) {
  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        Before proceeding, Kalpa will create a complete backup of your ESO data. This snapshot can
        be used to restore your exact pre-migration state at any time.
      </p>

      <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3 space-y-2">
        <SectionHeader className="text-xs">Snapshot will include:</SectionHeader>
        <div className="space-y-1 text-xs text-muted-foreground">
          <div className="flex items-center gap-2">
            <span className="text-emerald-400">{"\u2713"}</span>
            SavedVariables (all addon settings)
          </div>
          <div className="flex items-center gap-2">
            <span className="text-emerald-400">{"\u2713"}</span>
            UserSettings.txt &amp; AddOnSettings.txt
          </div>
          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={includeAddons}
              onChange={(e) => onIncludeAddonsChange(e.target.checked)}
              className="accent-[var(--primary)]"
            />
            AddOns folder (larger backup, recommended for first migration)
          </label>
        </div>
      </div>

      <p className="text-xs text-muted-foreground">
        Kalpa never modifies your ESO config or addon settings without creating a restore point
        first.
      </p>

      <Button onClick={onCreateSnapshot} disabled={loading}>
        {loading ? "Creating snapshot..." : "Create Snapshot & Continue"}
      </Button>
    </div>
  );
}

function DryRunPhase({
  dryRun,
  loading,
  confirmed,
  onExecute,
}: {
  dryRun: DryRunResult | null;
  loading: boolean;
  confirmed: boolean;
  onExecute: () => void;
}) {
  if (loading || !dryRun) {
    return (
      <div className="py-8 text-center text-sm text-muted-foreground">Analyzing Minion data...</div>
    );
  }

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        Dry-run complete. No files have been changed. Review what will happen:
      </p>

      <div className="max-h-[250px] overflow-y-auto space-y-2">
        {dryRun.willTrack.length > 0 && (
          <DiffSection
            title={`Will be tracked in Kalpa (${dryRun.willTrack.length})`}
            color="emerald"
            items={dryRun.willTrack.map((a) => `${a.folderName} (ESOUI #${a.esouiId})`)}
          />
        )}
        {dryRun.alreadyTracked.length > 0 && (
          <DiffSection
            title={`Already tracked (${dryRun.alreadyTracked.length})`}
            color="sky"
            items={dryRun.alreadyTracked.map((a) => a.folderName)}
          />
        )}
        {dryRun.unmanagedOnDisk.length > 0 && (
          <DiffSection
            title={`On disk but unmanaged (will remain as-is) (${dryRun.unmanagedOnDisk.length})`}
            color="white"
            items={dryRun.unmanagedOnDisk}
          />
        )}
        {dryRun.missingOnDisk.length > 0 && (
          <DiffSection
            title={`In Minion but not on disk (metadata only) (${dryRun.missingOnDisk.length})`}
            color="amber"
            items={dryRun.missingOnDisk.map((a) => `${a.folderName} (ESOUI #${a.esouiId})`)}
          />
        )}
      </div>

      {dryRun.willTrack.length === 0 && dryRun.missingOnDisk.length === 0 && (
        <p className="text-sm text-muted-foreground">
          All Minion addons are already tracked. Nothing to import.
        </p>
      )}

      {(dryRun.willTrack.length > 0 || dryRun.missingOnDisk.length > 0) && confirmed && (
        <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3">
          <p className="text-xs text-muted-foreground mb-2">
            This will only write to <code className="text-white/70">kalpa.json</code>. No addon
            folders, SavedVariables, or game settings will be touched.
          </p>
          <Button onClick={onExecute} disabled={loading}>
            {loading ? "Importing..." : "Import Metadata Now"}
          </Button>
        </div>
      )}
    </div>
  );
}

function DiffSection({ title, color, items }: { title: string; color: string; items: string[] }) {
  const colorMap: Record<string, string> = {
    emerald: "border-emerald-400/20 bg-emerald-400/[0.04] text-emerald-400",
    sky: "border-sky-400/20 bg-sky-400/[0.04] text-sky-400",
    amber: "border-amber-400/20 bg-amber-400/[0.04] text-amber-400",
    white: "border-white/[0.08] bg-white/[0.02] text-white/60",
  };
  const classes = colorMap[color] ?? colorMap.white;

  return (
    <div className={`rounded-lg border p-2 ${classes}`}>
      <div className="text-xs font-medium mb-1">{title}</div>
      <div className="text-xs opacity-80 space-y-0.5 max-h-[80px] overflow-y-auto">
        {items.map((item, i) => (
          <div key={i}>{item}</div>
        ))}
      </div>
    </div>
  );
}

function CompletePhase({
  result,
  snapshot,
}: {
  result: SafeMigrationResult;
  snapshot: SnapshotManifest | null;
}) {
  return (
    <div className="space-y-3">
      <div className="rounded-lg border border-emerald-400/20 bg-emerald-400/[0.04] p-3 text-sm text-emerald-400">
        Migration complete!
      </div>

      <div className="space-y-1 text-sm text-muted-foreground">
        <div>
          Imported: <span className="text-white">{result.imported}</span> addon
          {result.imported !== 1 ? "s" : ""}
        </div>
        <div>
          Already tracked: <span className="text-white">{result.alreadyTracked}</span>
        </div>
        {result.skippedMissing > 0 && (
          <div>
            Skipped (not on disk): <span className="text-white">{result.skippedMissing}</span>
          </div>
        )}
      </div>

      {snapshot && (
        <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3 text-xs text-muted-foreground">
          <div className="font-medium text-white/80 mb-1">Restore point saved</div>
          <div>
            Snapshot: {snapshot.fileCount} files ({formatBytes(snapshot.totalSize)})
          </div>
          <div className="mt-1 text-[10px] font-mono text-white/40 break-all">
            SHA-256: {snapshot.archiveSha256}
          </div>
          <p className="mt-1">
            You can restore this snapshot from Settings &rarr; Safety Center at any time.
          </p>
        </div>
      )}
    </div>
  );
}
