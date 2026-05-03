import { useState, useEffect } from "react";
import { toast } from "sonner";
import { motion, AnimatePresence } from "motion/react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Tabs, TabsIndicator, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";
import type { SnapshotManifest, IntegrityResult, OpLogEntry } from "@/types";

interface SafetyCenterProps {
  addonsPath: string;
  onClose: () => void;
  onRefresh: () => void;
}

type SafetyTab = "snapshots" | "integrity" | "log";

export function SafetyCenter({ addonsPath, onClose, onRefresh }: SafetyCenterProps) {
  const [activeTab, setActiveTab] = useState<SafetyTab>("snapshots");

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg h-[70vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Safety Center</DialogTitle>
        </DialogHeader>

        <Tabs
          value={activeTab}
          onValueChange={(v) => setActiveTab(v as SafetyTab)}
          className="flex-1 min-h-0 flex flex-col"
        >
          <TabsList className="w-full shrink-0">
            <TabsIndicator />
            <TabsTrigger value="snapshots" className="flex-1">
              Snapshots
            </TabsTrigger>
            <TabsTrigger value="integrity" className="flex-1">
              Integrity
            </TabsTrigger>
            <TabsTrigger value="log" className="flex-1">
              Activity Log
            </TabsTrigger>
          </TabsList>

          <div className="flex-1 min-h-0 overflow-y-auto">
            <AnimatePresence mode="wait">
              {activeTab === "snapshots" && (
                <motion.div
                  key="snapshots"
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: 0.08 }}
                >
                  <SnapshotsTab addonsPath={addonsPath} onRefresh={onRefresh} />
                </motion.div>
              )}
              {activeTab === "integrity" && (
                <motion.div
                  key="integrity"
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: 0.08 }}
                >
                  <IntegrityTab addonsPath={addonsPath} />
                </motion.div>
              )}
              {activeTab === "log" && (
                <motion.div
                  key="log"
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ duration: 0.08 }}
                >
                  <LogTab addonsPath={addonsPath} />
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        </Tabs>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function SnapshotsTab({ addonsPath, onRefresh }: { addonsPath: string; onRefresh: () => void }) {
  const [snapshots, setSnapshots] = useState<SnapshotManifest[]>([]);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [confirmRestore, setConfirmRestore] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const loadSnapshots = async () => {
    try {
      const result = await invokeOrThrow<SnapshotManifest[]>("list_snapshots", { addonsPath });
      setSnapshots(result);
    } catch (e) {
      toast.error(`Failed to load snapshots: ${getTauriErrorMessage(e)}`);
    }
  };

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const result = await invokeOrThrow<SnapshotManifest[]>("list_snapshots", { addonsPath });
        if (!cancelled) setSnapshots(result);
      } catch (e) {
        if (!cancelled) toast.error(`Failed to load snapshots: ${getTauriErrorMessage(e)}`);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [addonsPath]);

  const handleRestore = async (id: string) => {
    setRestoring(id);
    try {
      const count = await invokeOrThrow<number>("restore_snapshot", {
        addonsPath,
        snapshotId: id,
      });
      toast.success(`Restored ${count} files from snapshot`);
      onRefresh();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setRestoring(null);
      setConfirmRestore(null);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await invokeOrThrow("delete_snapshot", { addonsPath, snapshotId: id });
      toast.success("Snapshot deleted");
      loadSnapshots();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setConfirmDelete(null);
    }
  };

  return (
    <div className="space-y-3 mt-2">
      <p className="text-xs text-muted-foreground">
        Snapshots are created automatically before migrations and bulk operations. You can restore
        to any snapshot to undo changes.
      </p>

      <div className="max-h-[300px] overflow-y-auto space-y-2">
        {snapshots.length === 0 ? (
          <Fade>
            <p className="text-sm text-muted-foreground text-center py-4">No snapshots yet.</p>
          </Fade>
        ) : (
          snapshots.map((s, i) => (
            <Fade key={s.id} delay={i * 50}>
              <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-3 transition-all duration-200 hover:border-white/[0.1]">
                <div className="flex items-start justify-between">
                  <div>
                    <div className="font-medium text-sm">{s.label}</div>
                    <div className="text-xs text-muted-foreground mt-0.5">
                      {s.fileCount} files &middot; {formatBytes(s.totalSize)} &middot; {s.createdAt}
                    </div>
                    <div className="text-[10px] text-muted-foreground mt-0.5">
                      Includes: {s.sourcePaths.join(", ")}
                    </div>
                  </div>
                  <div className="flex gap-1 shrink-0">
                    <AnimatePresence mode="wait">
                      {confirmRestore === s.id ? (
                        <motion.div
                          key="confirm-restore"
                          initial={{ opacity: 0, scale: 0.95 }}
                          animate={{ opacity: 1, scale: 1 }}
                          exit={{ opacity: 0, scale: 0.95 }}
                          transition={{ duration: 0.15 }}
                          className="flex items-center gap-1"
                        >
                          <span className="text-xs text-amber-400 mr-1">Restore?</span>
                          <Button
                            size="sm"
                            variant="destructive"
                            onClick={() => handleRestore(s.id)}
                            disabled={restoring !== null}
                          >
                            {restoring === s.id ? "Restoring..." : "Yes"}
                          </Button>
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => setConfirmRestore(null)}
                          >
                            No
                          </Button>
                        </motion.div>
                      ) : (
                        <motion.div
                          key="restore-btn"
                          initial={{ opacity: 0 }}
                          animate={{ opacity: 1 }}
                          exit={{ opacity: 0 }}
                          transition={{ duration: 0.1 }}
                        >
                          <Button
                            size="sm"
                            onClick={() => setConfirmRestore(s.id)}
                            disabled={restoring !== null}
                          >
                            Restore
                          </Button>
                        </motion.div>
                      )}
                    </AnimatePresence>
                    <AnimatePresence mode="wait">
                      {confirmDelete === s.id ? (
                        <motion.div
                          key="confirm-delete"
                          initial={{ opacity: 0, scale: 0.95 }}
                          animate={{ opacity: 1, scale: 1 }}
                          exit={{ opacity: 0, scale: 0.95 }}
                          transition={{ duration: 0.15 }}
                          className="flex items-center gap-1"
                        >
                          <Button
                            size="sm"
                            variant="destructive"
                            onClick={() => handleDelete(s.id)}
                          >
                            Delete
                          </Button>
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => setConfirmDelete(null)}
                          >
                            Keep
                          </Button>
                        </motion.div>
                      ) : (
                        <motion.div
                          key="delete-btn"
                          initial={{ opacity: 0 }}
                          animate={{ opacity: 1 }}
                          exit={{ opacity: 0 }}
                          transition={{ duration: 0.1 }}
                        >
                          <Button
                            size="sm"
                            variant="destructive"
                            onClick={() => setConfirmDelete(s.id)}
                          >
                            Delete
                          </Button>
                        </motion.div>
                      )}
                    </AnimatePresence>
                  </div>
                </div>
              </div>
            </Fade>
          ))
        )}
      </div>
    </div>
  );
}

function IntegrityTab({ addonsPath }: { addonsPath: string }) {
  const [result, setResult] = useState<IntegrityResult | null>(null);
  const [loading, setLoading] = useState(false);

  const runCheck = async () => {
    setLoading(true);
    try {
      const r = await invokeOrThrow<IntegrityResult>("migration_check_integrity", { addonsPath });
      setResult(r);
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-3 mt-2">
      <p className="text-xs text-muted-foreground">
        Check that your AddOns and SavedVariables folders are intact and all tracked addons have
        their expected files.
      </p>

      <Button onClick={runCheck} disabled={loading} size="sm">
        {loading ? "Checking..." : "Run Integrity Check"}
      </Button>

      {result && (
        <Fade>
          <div className="space-y-2">
            <div className="flex gap-3 text-sm">
              <StatusBadge ok={result.addonsFolderOk} label="AddOns" />
              <StatusBadge ok={result.savedVariablesOk} label="SavedVariables" />
              <span className="text-muted-foreground">
                {result.addonCount} tracked addon{result.addonCount !== 1 ? "s" : ""}
              </span>
            </div>
            {result.issues.length > 0 ? (
              <div className="space-y-1 max-h-[200px] overflow-y-auto">
                {result.issues.map((issue) => (
                  <div
                    key={issue}
                    className="rounded-lg border border-amber-400/20 bg-amber-400/[0.04] px-3 py-1.5 text-xs text-amber-400"
                  >
                    {issue}
                  </div>
                ))}
              </div>
            ) : (
              <div className="rounded-lg border border-emerald-400/20 bg-emerald-400/[0.04] px-3 py-2 text-xs text-emerald-400">
                All checks passed. No issues found.
              </div>
            )}
          </div>
        </Fade>
      )}
    </div>
  );
}

function StatusBadge({ ok, label }: { ok: boolean; label: string }) {
  return (
    <span
      className={`inline-flex items-center gap-1 text-xs ${ok ? "text-emerald-400" : "text-red-400"}`}
    >
      {ok ? "\u2713" : "\u2717"} {label}
    </span>
  );
}

function LogTab({ addonsPath }: { addonsPath: string }) {
  const [entries, setEntries] = useState<OpLogEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    void invokeOrThrow<OpLogEntry[]>("read_ops_log", { addonsPath })
      .then((data) => {
        if (!cancelled) setEntries(data);
      })
      .catch((e) => {
        if (!cancelled) toast.error(`Failed to load log: ${getTauriErrorMessage(e)}`);
      });
    return () => {
      cancelled = true;
    };
  }, [addonsPath]);

  return (
    <div className="space-y-3 mt-2">
      <p className="text-xs text-muted-foreground">
        A record of all Kalpa operations on your ESO data.
      </p>

      <div className="max-h-[300px] overflow-y-auto space-y-1.5">
        {entries.length === 0 ? (
          <Fade>
            <p className="text-sm text-muted-foreground text-center py-4">No operations logged.</p>
          </Fade>
        ) : (
          [...entries].reverse().map((entry, i) => (
            <Fade key={`${entry.startedAt}-${i}`} delay={i * 40}>
              <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-2 text-xs">
                <div className="flex items-center justify-between">
                  <span className="font-medium text-white/80">{entry.operation}</span>
                  <span
                    className={entry.status === "success" ? "text-emerald-400" : "text-red-400"}
                  >
                    {entry.status}
                  </span>
                </div>
                <div className="text-muted-foreground mt-0.5">{entry.details}</div>
                <div className="text-[10px] text-white/30 mt-0.5">
                  {entry.startedAt} &rarr; {entry.finishedAt}
                  {entry.snapshotId && ` | Snapshot: ${entry.snapshotId}`}
                </div>
              </div>
            </Fade>
          ))
        )}
      </div>
    </div>
  );
}
