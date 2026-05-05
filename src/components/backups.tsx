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
import type { BackupInfo, SafeRestoreResult } from "../types";
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
  return `${filePart} · ${formatBytes(b.totalSize)}`;
}

function computeProtection(latest: BackupInfo | null): "none" | "good" | "stale" {
  if (!latest) return "none";
  const ageDays = (Date.now() / 1000 - latest.createdAtEpoch) / 86400;
  return ageDays > STALE_AFTER_DAYS ? "stale" : "good";
}

function backupKindLabel(b: BackupInfo): { label: string; color: "violet" | "sky" | "muted" } {
  if (b.kind === "autoBeforeRestore") return { label: "Safety snapshot", color: "violet" };
  if (b.kind === "character") return { label: "Character", color: "sky" };
  return { label: "Manual", color: "muted" };
}

export function Backups({ addonsPath, onClose }: BackupsProps) {
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

  const loadBackups = async () => {
    try {
      const result = await invokeOrThrow<BackupInfo[]>("list_backups", { addonsPath });
      setBackups(result);
    } catch (e) {
      toast.error(`Couldn't load backups: ${getTauriErrorMessage(e)}`);
    } finally {
      setLoaded(true);
    }
  };

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void loadBackups();
    setNewName(friendlyDefaultName());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const latest = useMemo(() => {
    return [...backups].sort((a, b) => b.createdAtEpoch - a.createdAtEpoch)[0] ?? null;
  }, [backups]);

  const protection = computeProtection(latest);

  const handleCreate = async () => {
    const name = newName.trim() || friendlyDefaultName();
    setCreating(true);
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
    }
  };

  const handleRestore = async (b: BackupInfo) => {
    setRestoring(b.name);
    try {
      const result = await invokeOrThrow<SafeRestoreResult>("restore_backup_safe", {
        addonsPath,
        backupName: b.name,
      });
      const snap = result.safetySnapshot;
      const undoNote = snap
        ? ` Your previous settings were saved as "${snap.name}" — restore it to undo.`
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
    }
  };

  const handleDelete = async (name: string) => {
    setDeleting(true);
    try {
      await invokeOrThrow("delete_backup", { addonsPath, backupName: name });
      toast.success("Backup deleted.");
      void loadBackups();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setDeleting(false);
      setConfirmDelete(null);
    }
  };

  const handleRevealFolder = async () => {
    try {
      const folderPath = await invokeOrThrow<string>("get_backups_folder_path", { addonsPath });
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(folderPath);
    } catch (e) {
      toast.error(`Couldn't open folder: ${getTauriErrorMessage(e)}`);
    }
  };

  const totalSize = useMemo(() => backups.reduce((sum, b) => sum + b.totalSize, 0), [backups]);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Backup &amp; Restore</DialogTitle>
          <DialogDescription>
            Save a copy of your addon settings so you can recover them if something goes wrong, or
            move them to a new PC.
          </DialogDescription>
        </DialogHeader>

        {/* Status hero */}
        <StatusHero protection={protection} latest={latest} loaded={loaded} />

        {/* Primary action */}
        <GlassPanel variant="subtle" className="p-4 space-y-3">
          <div className="flex items-start justify-between gap-3">
            <div>
              <div className="text-sm font-medium">Create a new backup</div>
              <div className="text-xs text-muted-foreground mt-0.5">
                Takes a snapshot of your current addon settings. Safe to do anytime.
              </div>
            </div>
            <Button onClick={handleCreate} disabled={creating} size="sm" className="shrink-0">
              <Plus className="size-3.5" />
              {creating ? "Backing up…" : "Back Up Now"}
            </Button>
          </div>

          {!showLabelField ? (
            <button
              type="button"
              onClick={() => setShowLabelField(true)}
              className="text-xs text-muted-foreground hover:text-white/80 transition-colors underline-offset-2 hover:underline"
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
            className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-white/80 transition-colors"
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
                <div className="text-xs text-muted-foreground space-y-1.5 pl-4 border-l border-white/[0.08]">
                  <p>
                    Your <span className="text-white/80">addon settings</span> — every
                    customization, keybind, and configuration each addon remembers between sessions
                    (technically, the SavedVariables folder).
                  </p>
                  <p>
                    <span className="text-white/80">Not included:</span> the addons themselves
                    (those come from ESOUI and can be reinstalled), or your ESO game saves.
                  </p>
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </GlassPanel>

        {/* Backup list */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <div className="text-xs uppercase tracking-wider text-muted-foreground font-semibold">
              Your backups
              {backups.length > 0 && (
                <span className="ml-2 normal-case tracking-normal font-normal text-white/40">
                  {backups.length} · {formatBytes(totalSize)} total
                </span>
              )}
            </div>
          </div>

          <div className="max-h-[280px] overflow-y-auto space-y-1.5 pr-1">
            {!loaded ? null : backups.length === 0 ? (
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
                <Fade key={b.name} delay={i * 30}>
                  <BackupRow
                    backup={b}
                    isLatest={latest?.name === b.name}
                    isRestoring={restoring === b.name}
                    confirmingRestore={confirmRestore?.name === b.name}
                    confirmingDelete={confirmDelete === b.name}
                    onAskRestore={() => setConfirmRestore(b)}
                    onAskDelete={() => setConfirmDelete(b.name)}
                    onCancelRestore={() => setConfirmRestore(null)}
                    onCancelDelete={() => setConfirmDelete(null)}
                    onConfirmRestore={() => handleRestore(b)}
                    onConfirmDelete={() => handleDelete(b.name)}
                    deleting={deleting}
                    anyRestoring={restoring !== null}
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

      {/* Restore confirmation modal */}
      <RestoreConfirmDialog
        backup={confirmRestore}
        restoring={restoring !== null}
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
    return <div className="h-[68px] rounded-xl bg-white/[0.02] border border-white/[0.04]" />;
  }

  if (protection === "none") {
    return (
      <GlassPanel
        variant="subtle"
        className="p-4 border-amber-400/20 bg-amber-400/[0.04] flex items-center gap-3"
      >
        <ShieldAlert className="size-5 text-amber-400 shrink-0" />
        <div className="flex-1">
          <div className="text-sm font-medium text-amber-100">No backup yet</div>
          <div className="text-xs text-amber-200/70">
            Your addon settings aren't protected. Create your first backup below.
          </div>
        </div>
      </GlassPanel>
    );
  }

  if (protection === "stale") {
    return (
      <GlassPanel
        variant="subtle"
        className="p-4 border-amber-400/20 bg-amber-400/[0.03] flex items-center gap-3"
      >
        <Clock className="size-5 text-amber-400 shrink-0" />
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
      className="p-4 border-emerald-400/15 bg-emerald-400/[0.03] flex items-center gap-3"
    >
      <ShieldCheck className="size-5 text-emerald-400 shrink-0" />
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
  anyRestoring,
}: {
  backup: BackupInfo;
  isLatest: boolean;
  isRestoring: boolean;
  confirmingRestore: boolean;
  confirmingDelete: boolean;
  onAskRestore: () => void;
  onAskDelete: () => void;
  onCancelRestore: () => void;
  onCancelDelete: () => void;
  onConfirmRestore: () => void;
  onConfirmDelete: () => void;
  deleting: boolean;
  anyRestoring: boolean;
}) {
  const kindInfo = backupKindLabel(backup);
  const isSafety = backup.kind === "autoBeforeRestore";

  return (
    <div
      className={cn(
        "flex items-center justify-between rounded-xl border bg-white/[0.02] p-3 transition-all duration-200 hover:border-white/[0.12]",
        isSafety ? "border-violet-400/15 bg-violet-400/[0.025]" : "border-white/[0.06]"
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
        </div>
        <div className="text-xs text-muted-foreground mt-1 flex items-center gap-1.5">
          <Clock className="size-3" />
          {formatRelativeFromEpoch(backup.createdAtEpoch)}
          <span className="text-white/20">·</span>
          {describeBackup(backup)}
        </div>
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
              <span className="text-xs text-amber-300/90 mr-1">Delete?</span>
              <Button size="sm" variant="destructive" disabled={deleting} onClick={onConfirmDelete}>
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
                disabled={anyRestoring}
                title="Replace your current settings with this backup"
              >
                <RotateCcw className="size-3.5" />
                {isRestoring ? "Restoring…" : "Restore"}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={onAskDelete}
                disabled={anyRestoring}
                title="Delete this backup"
                className="text-muted-foreground hover:text-red-400"
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
  onCancel,
  onConfirm,
}: {
  backup: BackupInfo | null;
  restoring: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog open={backup !== null} onOpenChange={(open) => !open && !restoring && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <RotateCcw className="size-4 text-primary" />
            Restore this backup?
          </DialogTitle>
          <DialogDescription>
            This will replace your current addon settings with the ones in this backup.
          </DialogDescription>
        </DialogHeader>

        {backup && (
          <div className="rounded-xl border border-white/[0.08] bg-white/[0.02] p-3 space-y-1">
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
          </div>
        )}

        <div className="rounded-xl border border-emerald-400/15 bg-emerald-400/[0.04] p-3 flex gap-2.5">
          <ShieldCheck className="size-4 text-emerald-400 shrink-0 mt-0.5" />
          <div className="text-xs text-emerald-100/90 leading-relaxed">
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
          <Button onClick={onConfirm} disabled={restoring}>
            {restoring ? "Restoring…" : "Yes, Restore"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
