import { useState, useEffect, useMemo } from "react";
import { toast } from "sonner";
import { motion, AnimatePresence } from "motion/react";
import {
  ShieldCheck,
  ShieldAlert,
  Clock,
  FolderOpen,
  RotateCcw,
  Trash2,
  Plus,
  Info,
  ChevronDown,
  Sparkles,
  HardDrive,
  User,
} from "lucide-react";
import type { BackupInfo, ConsolidateLegacyBackupsResult, SafeRestoreResult } from "../types";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { formatBytes, cn } from "@/lib/utils";

interface BackupsProps {
  addonsPath: string;
  onClose: () => void;
  /** True when a create/restore/delete/character-backup is in flight anywhere
   * in the app (Backups or Characters dialog), so destructive actions here
   * stay gated even if the user switches dialogs mid-operation. */
  sharedOpInFlight: boolean;
  onSharedOpInFlightChange: (busy: boolean) => void;
}

const STALE_AFTER_DAYS = 14;

function formatRelativeFromEpoch(epochSeconds: number): string {
  if (!epochSeconds) return "unknown time";
  const now = Math.floor(Date.now() / 1000);
  const diff = now - epochSeconds;
  if (diff < 60) return "just now";
  if (diff < 3600) {
    const m = Math.floor(diff / 60);
    return `${m} minute${m === 1 ? "" : "s"} ago`;
  }
  if (diff < 86400) {
    const h = Math.floor(diff / 3600);
    return `${h} hour${h === 1 ? "" : "s"} ago`;
  }
  if (diff < 86400 * 2) return "Yesterday";
  if (diff < 86400 * 30) {
    const d = Math.floor(diff / 86400);
    return `${d} day${d === 1 ? "" : "s"} ago`;
  }
  const date = new Date(epochSeconds * 1000);
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

function friendlyDefaultName(): string {
  const now = new Date();
  const date = now.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
  const time = now.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  // Backend rejects names with most punctuation; use safe-ish format.
  return `Manual backup ${date.replace(/,/g, "")} ${time.replace(/[: ]/g, "-")}`;
}

function describeBackup(b: BackupInfo): string {
  const filePart = `${b.fileCount} file${b.fileCount === 1 ? "" : "s"}`;
  return `${filePart} · ${formatBytes(b.totalSize)} of settings`;
}

function backupKey(b: BackupInfo): string {
  return `${b.storage}:${b.legacy}:${b.name}`;
}

function backupStoragePath(b: BackupInfo): string {
  switch (b.storage) {
    case "manual":
      return "KalpaBackups/Manual";
    case "automatic":
      return "KalpaBackups/Automatic";
    case "characters":
      return "KalpaBackups/Characters";
    case "legacy":
      return "kalpa-backups";
    case "shared":
      return "KalpaBackups";
  }
}

function computeProtection(latest: BackupInfo | null): "none" | "good" | "stale" {
  if (!latest) return "none";
  const ageDays = (Date.now() / 1000 - latest.createdAtEpoch) / 86400;
  return ageDays > STALE_AFTER_DAYS ? "stale" : "good";
}

function backupKindLabel(b: BackupInfo): { label: string; color: "violet" | "sky" | "muted" } {
  if (b.kind === "autoBeforeRestore") return { label: "Safety snapshot", color: "violet" };
  if (b.kind === "autoCleanup") return { label: "Cleanup backup", color: "muted" };
  if (b.kind === "character") return { label: "Character", color: "sky" };
  return { label: "Manual", color: "muted" };
}

export function Backups({
  addonsPath,
  onClose,
  sharedOpInFlight,
  onSharedOpInFlightChange,
}: BackupsProps) {
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [newName, setNewName] = useState("");
  const [showLabelField, setShowLabelField] = useState(false);
  const [showWhatExpanded, setShowWhatExpanded] = useState(false);
  const [creating, setCreating] = useState(false);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [confirmRestore, setConfirmRestore] = useState<BackupInfo | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [confirmConsolidate, setConfirmConsolidate] = useState(false);
  const [consolidating, setConsolidating] = useState(false);
  const [consolidationResult, setConsolidationResult] =
    useState<ConsolidateLegacyBackupsResult | null>(null);

  // Gates every destructive backup-surface button: local ops here, or an
  // in-flight op on the other backup surface (Characters) after a dialog switch.
  const anyBackupOpInFlight =
    sharedOpInFlight || creating || restoring !== null || deleting || consolidating;

  const loadBackups = async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const result = await invokeOrThrow<BackupInfo[]>("list_backups", { addonsPath });
      setBackups(result);
    } catch (e) {
      setLoadError(getTauriErrorMessage(e));
    } finally {
      setLoaded(true);
      setLoading(false);
    }
  };

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void loadBackups();
    setNewName(friendlyDefaultName());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const latest = useMemo(() => {
    return (
      [...backups]
        .filter((b) => b.kind !== "autoBeforeRestore" && b.kind !== "autoCleanup")
        .sort((a, b) => b.createdAtEpoch - a.createdAtEpoch)[0] ?? null
    );
  }, [backups]);

  // Only whole-account (manual) backups count toward the "protected" claim —
  // character and cleanup backups cover only selected settings.
  const latestManual = useMemo(() => {
    return (
      [...backups]
        .filter((b) => b.kind === "manual")
        .sort((a, b) => b.createdAtEpoch - a.createdAtEpoch)[0] ?? null
    );
  }, [backups]);

  const hasCharacterBackups = useMemo(() => backups.some((b) => b.kind === "character"), [backups]);

  const protection = computeProtection(latestManual);
  const hasEligibleLegacyBackups = backups.some(
    (b) =>
      b.legacy && b.format === "folder" && (b.kind === "manual" || b.kind === "autoBeforeRestore")
  );

  const handleConsolidate = async () => {
    if (anyBackupOpInFlight) return;
    setConsolidating(true);
    setConsolidationResult(null);
    onSharedOpInFlightChange(true);
    try {
      const result = await invokeOrThrow<ConsolidateLegacyBackupsResult>(
        "consolidate_legacy_backups",
        { addonsPath }
      );
      setConsolidationResult(result);
      setConfirmConsolidate(false);
      await loadBackups();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setConsolidating(false);
      onSharedOpInFlightChange(false);
    }
  };

  const handleCreate = async () => {
    // The button disables itself, but the label input's Enter handler calls this
    // directly — without the guard a second Enter (or an Enter during a backup
    // op started on the Characters dialog) starts a concurrent create.
    if (anyBackupOpInFlight) return;
    const name = newName.trim() || friendlyDefaultName();
    setCreating(true);
    onSharedOpInFlightChange(true);
    try {
      const info = await invokeOrThrow<BackupInfo>("create_backup", {
        addonsPath,
        backupName: name,
      });
      toast.success(
        `Backup saved · ${info.fileCount} file${info.fileCount === 1 ? "" : "s"} (${formatBytes(info.totalSize)})`
      );
      setNewName(friendlyDefaultName());
      setShowLabelField(false);
      void loadBackups();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setCreating(false);
      onSharedOpInFlightChange(false);
    }
  };

  const handleRestore = async (b: BackupInfo) => {
    if (anyBackupOpInFlight) return;
    setRestoring(backupKey(b));
    onSharedOpInFlightChange(true);
    try {
      const result = await invokeOrThrow<SafeRestoreResult>("restore_backup_safe", {
        addonsPath,
        backupName: b.name,
        legacy: b.legacy,
        storage: b.storage,
      });
      const snap = result.safetySnapshot;
      const undoNote = snap
        ? " Your previous settings were saved as a safety snapshot — look for it in the list to undo."
        : "";
      toast.success(
        `Restored ${result.restoredFiles} file${result.restoredFiles === 1 ? "" : "s"}.${undoNote}`,
        { duration: 7000 }
      );
      void loadBackups();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setRestoring(null);
      setConfirmRestore(null);
      onSharedOpInFlightChange(false);
    }
  };

  const handleDelete = async (b: BackupInfo) => {
    if (anyBackupOpInFlight) return;
    setDeleting(true);
    onSharedOpInFlightChange(true);
    try {
      await invokeOrThrow("delete_backup", {
        addonsPath,
        backupName: b.name,
        legacy: b.legacy,
        storage: b.storage,
      });
      toast.success("Backup deleted.");
      void loadBackups();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setDeleting(false);
      setConfirmDelete(null);
      onSharedOpInFlightChange(false);
    }
  };

  const handleRevealFolder = async () => {
    try {
      const folderPath = await invokeOrThrow<string>("get_backups_folder_path", { addonsPath });
      await invokeOrThrow("reveal_allowed_path", { path: folderPath });
    } catch (e) {
      toast.error(`Couldn't open folder: ${getTauriErrorMessage(e)}`);
    }
  };

  const totalSize = useMemo(() => backups.reduce((sum, b) => sum + b.totalSize, 0), [backups]);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] grid-cols-[minmax(0,1fr)] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Backup & Restore</DialogTitle>
          <DialogDescription>
            Save a copy of your addon settings so you can recover them if something goes wrong, or
            move them to a new PC.
          </DialogDescription>
        </DialogHeader>

        {/* Status hero */}
        {!loadError && <StatusHero protection={protection} latest={latestManual} loaded={loaded} />}
        {loaded && hasCharacterBackups && protection === "none" && (
          <InfoPill color="sky" className="w-fit">
            <User className="size-3" />
            Character backups do not cover all addon settings.
          </InfoPill>
        )}

        {/* Primary action */}
        <GlassPanel variant="subtle" className="p-4 space-y-3">
          <div className="flex items-start justify-between gap-3">
            <div>
              <div className="text-sm font-medium">Create a new backup</div>
              <div className="text-xs text-muted-foreground mt-0.5">
                Save all your addon settings in a compressed ZIP.
              </div>
            </div>
            <Button
              onClick={handleCreate}
              disabled={anyBackupOpInFlight}
              size="sm"
              className="shrink-0"
            >
              <Plus className="size-3.5" />
              {creating ? "Backing up…" : "Back Up Now"}
            </Button>
          </div>

          {!showLabelField ? (
            <button
              type="button"
              onClick={() => setShowLabelField(true)}
              disabled={anyBackupOpInFlight}
              className="text-xs text-muted-foreground hover:text-foreground transition-colors underline-offset-2 hover:underline disabled:opacity-50"
            >
              Add a custom label (optional)
            </button>
          ) : (
            <Fade>
              <div className="flex gap-2">
                <Input
                  placeholder="e.g. Before trying new addons"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && handleCreate()}
                  autoFocus
                />
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    setShowLabelField(false);
                    setNewName(friendlyDefaultName());
                  }}
                >
                  Cancel
                </Button>
              </div>
            </Fade>
          )}

          {/* What gets backed up — progressive disclosure */}
          <button
            type="button"
            onClick={() => setShowWhatExpanded((v) => !v)}
            className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
          >
            <Info className="size-3" />
            What gets backed up?
            <ChevronDown
              className={cn("size-3 transition-transform", showWhatExpanded && "rotate-180")}
            />
          </button>
          <AnimatePresence initial={false}>
            {showWhatExpanded && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.18 }}
                className="overflow-hidden"
              >
                <div className="text-xs text-muted-foreground space-y-1.5 pl-4 border-l border-structure-08">
                  <p>
                    Your <span className="text-foreground">addon settings</span> — every
                    customization, keybind, and configuration each addon remembers between sessions
                    (technically, the SavedVariables folder).
                  </p>
                  <p>
                    <span className="text-foreground">Not included:</span> addon files or ESO game
                    configuration. Automatic snapshots in the Safety Center can cover those too.
                  </p>
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </GlassPanel>

        <p className="text-xs text-muted-foreground leading-relaxed">
          New manual ZIPs are saved in <span className="text-foreground">KalpaBackups/Manual</span>.
          Automatic and pre-restore ZIPs use{" "}
          <span className="text-foreground">KalpaBackups/Automatic</span>; character folders use{" "}
          <span className="text-foreground">KalpaBackups/Characters</span>. Existing backups in{" "}
          <span className="text-foreground">KalpaBackups</span> and{" "}
          <span className="text-foreground">kalpa-backups</span> remain available below.
        </p>

        {/* Backup list */}
        <div className="space-y-2">
          {!loading && !loadError && hasEligibleLegacyBackups && (
            <GlassPanel variant="subtle" className="p-3 space-y-2">
              <p className="text-xs text-muted-foreground leading-relaxed">
                Copy older full settings backups to ZIPs in KalpaBackups/Manual, and pre-restore
                backups to KalpaBackups/Automatic. Your original folders stay in place.
              </p>
              <Button
                size="sm"
                variant="outline"
                disabled={anyBackupOpInFlight}
                onClick={() => setConfirmConsolidate(true)}
              >
                {consolidating ? "Copying older backups…" : "Review older backups"}
              </Button>
            </GlassPanel>
          )}
          {consolidationResult && (
            <div className="rounded-xl border border-structure-08 p-3 space-y-2" role="status">
              <p className="text-sm font-medium">
                {consolidationResult.copied} copied · {consolidationResult.alreadyPresent} already
                present · {consolidationResult.skipped.length} skipped
              </p>
              <p className="text-xs text-muted-foreground">Original folders were kept.</p>
              {consolidationResult.skipped.length > 0 && (
                <ul className="max-h-24 overflow-y-auto text-xs text-muted-foreground space-y-1">
                  {consolidationResult.skipped.map((item, index) => (
                    <li key={`${item.name}:${index}`} className="break-words">
                      <span className="text-foreground">{item.name}</span>: {item.reason}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
          <div className="flex items-center justify-between">
            <div className="text-xs uppercase tracking-wider text-muted-foreground font-semibold">
              Your backups
              {backups.length > 0 && (
                <span className="ml-2 normal-case tracking-normal font-normal text-muted-foreground">
                  {backups.length} · {formatBytes(totalSize)} of settings
                </span>
              )}
            </div>
          </div>

          <div className="max-h-[280px] overflow-y-auto space-y-1.5 pr-1">
            {loadError ? (
              <div className="rounded-xl border border-status-danger/20 p-4 space-y-2" role="alert">
                <p className="text-sm font-medium">Couldn't load backups</p>
                <p className="text-xs text-muted-foreground break-words">{loadError}</p>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void loadBackups()}
                  disabled={loading}
                >
                  Try again
                </Button>
              </div>
            ) : loading ? (
              <p className="py-4 text-sm text-muted-foreground" role="status">
                Loading backups…
              </p>
            ) : backups.length === 0 ? (
              <Fade>
                <GlassPanel variant="subtle" className="p-6 text-center">
                  <HardDrive className="size-8 mx-auto text-muted-foreground/60 mb-2" />
                  <div className="text-sm font-medium">No backups yet</div>
                  <div className="text-xs text-muted-foreground mt-1 max-w-sm mx-auto">
                    Create your first backup so you can recover your settings if anything ever goes
                    wrong.
                  </div>
                </GlassPanel>
              </Fade>
            ) : (
              backups.map((b, i) => (
                <Fade key={backupKey(b)} delay={i * 30}>
                  <BackupRow
                    backup={b}
                    isLatest={latest !== null && backupKey(latest) === backupKey(b)}
                    isRestoring={restoring === backupKey(b)}
                    confirmingDelete={confirmDelete === backupKey(b)}
                    onAskRestore={() => setConfirmRestore(b)}
                    onAskDelete={() => setConfirmDelete(backupKey(b))}
                    onCancelDelete={() => setConfirmDelete(null)}
                    onConfirmDelete={() => handleDelete(b)}
                    deleting={deleting}
                    anyOpInFlight={anyBackupOpInFlight}
                  />
                </Fade>
              ))
            )}
          </div>
        </div>

        <DialogFooter className="flex-row sm:justify-between gap-2">
          <Button variant="ghost" size="sm" onClick={handleRevealFolder}>
            <FolderOpen className="size-3.5" />
            Show in folder
          </Button>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>

      {/* Backup confirmation modals */}
      <Dialog
        open={confirmConsolidate}
        onOpenChange={(open) => !consolidating && setConfirmConsolidate(open)}
      >
        <DialogContent className="max-h-[calc(100dvh-2rem)] grid-cols-[minmax(0,1fr)] overflow-y-auto sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Copy older backups to KalpaBackups?</DialogTitle>
            <DialogDescription>
              Copy full settings folders from kalpa-backups into verified ZIPs in
              KalpaBackups/Manual, and pre-restore backups into KalpaBackups/Automatic. Your
              original folders are kept, and your current settings stay unchanged. Cleanup backups
              are not copied.
            </DialogDescription>
          </DialogHeader>
          <p className="text-xs text-muted-foreground leading-relaxed">
            Character backups remain supported in their original folders. They are not converted.
            Backups that cannot be copied safely are skipped with a reason.
          </p>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setConfirmConsolidate(false)}
              disabled={consolidating}
            >
              Cancel
            </Button>
            <Button onClick={handleConsolidate} disabled={anyBackupOpInFlight}>
              {consolidating ? "Copying…" : "Copy older backups"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <RestoreConfirmDialog
        backup={confirmRestore}
        restoring={restoring !== null}
        anyOpInFlight={anyBackupOpInFlight}
        onCancel={() => setConfirmRestore(null)}
        onConfirm={() => confirmRestore && handleRestore(confirmRestore)}
      />
    </Dialog>
  );
}

// ────────────────────────────────────────────────────────────────────────────

function StatusHero({
  protection,
  latest,
  loaded,
}: {
  protection: "none" | "good" | "stale";
  latest: BackupInfo | null;
  loaded: boolean;
}) {
  if (!loaded) {
    return <div className="h-[68px] rounded-xl bg-structure-02 border border-structure-04" />;
  }

  if (protection === "none") {
    return (
      <GlassPanel
        variant="subtle"
        className="p-4 border-status-warning/20 bg-status-warning/[0.04] flex items-center gap-3"
      >
        <ShieldAlert className="size-5 text-status-warning shrink-0" />
        <div className="flex-1">
          <div className="text-sm font-medium text-status-warning-faint">No full backup yet</div>
          <div className="text-xs text-status-warning-muted">
            Partial backups do not cover all addon settings. Create a full backup below.
          </div>
        </div>
      </GlassPanel>
    );
  }

  if (protection === "stale") {
    return (
      <GlassPanel
        variant="subtle"
        className="p-4 border-status-warning/20 bg-status-warning/[0.03] flex items-center gap-3"
      >
        <Clock className="size-5 text-status-warning shrink-0" />
        <div className="flex-1">
          <div className="text-sm font-medium">Last backup was a while ago</div>
          <div className="text-xs text-muted-foreground">
            Most recent: {latest && formatRelativeFromEpoch(latest.createdAtEpoch)}. Consider making
            a fresh one.
          </div>
        </div>
      </GlassPanel>
    );
  }

  return (
    <GlassPanel
      variant="subtle"
      className="p-4 border-status-success/15 bg-status-success/[0.03] flex items-center gap-3"
    >
      <ShieldCheck className="size-5 text-status-success shrink-0" />
      <div className="flex-1">
        <div className="text-sm font-medium">Your settings are protected</div>
        <div className="text-xs text-muted-foreground">
          Last backup {latest && formatRelativeFromEpoch(latest.createdAtEpoch)}
          {latest && ` · ${describeBackup(latest)}`}
        </div>
      </div>
    </GlassPanel>
  );
}

// ────────────────────────────────────────────────────────────────────────────

function BackupRow({
  backup,
  isLatest,
  isRestoring,
  confirmingDelete,
  onAskRestore,
  onAskDelete,
  onCancelDelete,
  onConfirmDelete,
  deleting,
  anyOpInFlight,
}: {
  backup: BackupInfo;
  isLatest: boolean;
  isRestoring: boolean;
  confirmingDelete: boolean;
  onAskRestore: () => void;
  onAskDelete: () => void;
  onCancelDelete: () => void;
  onConfirmDelete: () => void;
  deleting: boolean;
  anyOpInFlight: boolean;
}) {
  const kindInfo = backupKindLabel(backup);
  const isSafety = backup.kind === "autoBeforeRestore";

  return (
    <div
      className={cn(
        "flex items-center justify-between rounded-xl border bg-structure-02 p-3 transition-all duration-200 hover:border-structure-12",
        isSafety ? "border-status-library/15 bg-status-library/[0.025]" : "border-structure-06"
      )}
    >
      <div className="min-w-0 flex-1 pr-3">
        <div className="flex items-center gap-2 flex-wrap">
          <div className="font-medium text-sm truncate" title={backup.name}>
            {isSafety
              ? "Auto-saved before restore"
              : backup.kind === "character"
                ? backup.name.replace(/^char-/, "")
                : backup.name}
          </div>
          <InfoPill color={kindInfo.color}>
            {backup.kind === "character" ? <User className="size-3" /> : null}
            {isSafety ? <Sparkles className="size-3" /> : null}
            {kindInfo.label}
          </InfoPill>
          {isLatest && <InfoPill color="emerald">Latest</InfoPill>}
          <InfoPill color="muted">
            {backup.legacy ? "Legacy folder" : backup.format === "zip" ? "ZIP" : "Folder"}
          </InfoPill>
        </div>
        <div className="text-xs text-muted-foreground mt-1 flex flex-wrap items-center gap-1.5">
          <Clock className="size-3" />
          {formatRelativeFromEpoch(backup.createdAtEpoch)}
          <span className="text-muted-foreground">·</span>
          {describeBackup(backup)}
        </div>
        <div className="text-xs text-muted-foreground mt-1 break-all">
          {backupStoragePath(backup)}
        </div>
        {backup.kind === "autoCleanup" && (
          <p className="text-xs text-muted-foreground mt-1">
            Selected SavedVariables files saved before cleanup. Not a full settings backup.
          </p>
        )}
      </div>
      <div className="flex gap-1 shrink-0">
        <AnimatePresence mode="wait" initial={false}>
          {confirmingDelete ? (
            <motion.div
              key="confirm-delete"
              initial={{ opacity: 0, scale: 0.95 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.95 }}
              transition={{ duration: 0.12 }}
              className="flex items-center gap-1"
            >
              <span className="text-xs text-status-warning-soft/90 mr-1">Delete?</span>
              <Button
                size="sm"
                variant="destructive"
                disabled={anyOpInFlight}
                onClick={onConfirmDelete}
              >
                {deleting ? "…" : "Yes"}
              </Button>
              <Button size="sm" variant="outline" disabled={deleting} onClick={onCancelDelete}>
                No
              </Button>
            </motion.div>
          ) : (
            <motion.div
              key="actions"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.1 }}
              className="flex items-center gap-1"
            >
              <Button
                size="sm"
                onClick={onAskRestore}
                disabled={anyOpInFlight}
                title="Replace your current settings with this backup"
              >
                <RotateCcw className="size-3.5" />
                {isRestoring ? "Restoring…" : "Restore"}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={onAskDelete}
                disabled={anyOpInFlight}
                title="Delete this backup"
                aria-label={`Delete ${backup.name} (${backupStoragePath(backup)})`}
                className="text-muted-foreground hover:text-status-danger"
              >
                <Trash2 className="size-3.5" />
              </Button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
}

// ────────────────────────────────────────────────────────────────────────────

function RestoreConfirmDialog({
  backup,
  restoring,
  anyOpInFlight,
  onCancel,
  onConfirm,
}: {
  backup: BackupInfo | null;
  restoring: boolean;
  anyOpInFlight: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog open={backup !== null} onOpenChange={(open) => !open && !restoring && onCancel()}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] grid-cols-[minmax(0,1fr)] overflow-y-auto sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <RotateCcw className="size-4 text-primary" />
            Restore this backup?
          </DialogTitle>
          <DialogDescription>
            {backup?.kind === "character"
              ? (backup?.worldsSpanned ?? 0) > 1
                ? `This will restore ${backup.name.replace(/^char-/, "")}'s character settings — read the warning below first.`
                : `This will restore ${backup.name.replace(/^char-/, "")}'s character settings only — your other characters and account-wide settings are left untouched.`
              : backup?.kind === "autoCleanup"
                ? "This will restore only the selected SavedVariables files saved before cleanup. Other addon settings, addon files, and ESO game configuration stay unchanged."
                : backup?.kind === "autoBeforeRestore"
                  ? "This will restore your safety snapshot, replacing your current addon settings with the ones saved just before your last restore."
                  : "This will replace your current addon settings with the ones in this backup. Addon files and ESO game configuration stay unchanged."}
          </DialogDescription>
        </DialogHeader>

        {backup?.kind === "character" && (backup?.worldsSpanned ?? 0) > 1 && (
          <div className="rounded-xl border border-status-warning/20 bg-status-warning/[0.04] p-3 flex gap-2.5">
            <ShieldAlert className="size-4 text-status-warning shrink-0 mt-0.5" />
            <div className="text-xs text-status-warning-faint/90 leading-relaxed">
              <span className="font-semibold">
                This character name exists on more than one server.
              </span>{" "}
              Restoring it will overwrite the settings of every same-named character, not just one.
            </div>
          </div>
        )}

        {backup && (
          <div className="rounded-xl border border-structure-08 bg-structure-02 p-3 space-y-1">
            <div className="text-sm font-medium truncate" title={backup.name}>
              {backup.kind === "autoBeforeRestore"
                ? "Auto-saved before restore"
                : backup.kind === "character"
                  ? backup.name.replace(/^char-/, "")
                  : backup.name}
            </div>
            <div className="text-xs text-muted-foreground">
              {formatRelativeFromEpoch(backup.createdAtEpoch)} · {describeBackup(backup)}
            </div>
            <div className="text-xs text-muted-foreground">
              {backup.legacy ? "Legacy folder" : backup.format === "zip" ? "ZIP" : "Folder"} ·{" "}
              {backupStoragePath(backup)}
            </div>
          </div>
        )}

        <div className="rounded-xl border border-status-success/15 bg-status-success/[0.04] p-3 flex gap-2.5">
          <ShieldCheck className="size-4 text-status-success shrink-0 mt-0.5" />
          <div className="text-xs text-status-success-faint/90 leading-relaxed">
            <span className="font-semibold">Don't worry — this is reversible.</span> We'll save your
            current settings as a safety snapshot first, so you can undo by restoring it.
          </div>
        </div>

        <p className="text-xs text-muted-foreground">
          ESO must be closed for the new settings to load — restart the game after restoring.
        </p>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel} disabled={restoring}>
            Cancel
          </Button>
          <Button onClick={onConfirm} disabled={anyOpInFlight}>
            {restoring ? "Restoring…" : "Yes, Restore"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
