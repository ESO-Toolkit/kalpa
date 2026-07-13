import { useState, useEffect } from "react";
import { toast } from "sonner";
import { motion, AnimatePresence } from "motion/react";
import { Check, ChevronRight, Pencil, RefreshCw, X } from "lucide-react";
import type { ActivateProfileResult, AddonProfile, ProfilePlan } from "../types";
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
import { InfoPill } from "@/components/ui/info-pill";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { useEnsureEsoNotBlocking } from "@/lib/eso-running-context";
import { cn } from "@/lib/utils";

interface ProfilesProps {
  addonsPath: string;
  /** Folder names of currently ENABLED addons, from the latest scan. Used to
   * flag the active profile as "modified" when the setup has drifted. */
  enabledFolders: string[];
  onClose: () => void;
  onRefresh: () => void;
}

/** Set equality between a profile snapshot and the current enabled folders. */
function matchesSnapshot(snapshot: string[], enabled: string[]): boolean {
  if (snapshot.length !== enabled.length) return false;
  const set = new Set(snapshot);
  return enabled.every((name) => set.has(name));
}

function PreviewNameList({ names }: { names: string[] }) {
  return (
    <div className="mt-1 max-h-24 overflow-y-auto rounded-lg bg-white/[0.03] px-2 py-1.5 text-xs text-muted-foreground">
      {names.join(", ")}
    </div>
  );
}

export function Profiles({ addonsPath, enabledFolders, onClose, onRefresh }: ProfilesProps) {
  const [profiles, setProfiles] = useState<AddonProfile[]>([]);
  const [activeProfile, setActiveProfile] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);
  const [previewing, setPreviewing] = useState<string | null>(null);
  const [preview, setPreview] = useState<{ name: string; plan: ProfilePlan } | null>(null);
  const [activating, setActivating] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [confirmUpdate, setConfirmUpdate] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renameSaving, setRenameSaving] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const ensureEsoNotBlocking = useEnsureEsoNotBlocking();

  const busy =
    creating || deleting || updating || renameSaving || previewing !== null || activating !== null;

  const loadProfiles = async () => {
    try {
      const [profs, active] = await invokeOrThrow<[AddonProfile[], string | null]>(
        "list_profiles",
        {
          addonsPath,
        }
      );
      setProfiles(profs);
      setActiveProfile(active);
    } catch (e) {
      toast.error(`Failed to load profiles: ${getTauriErrorMessage(e)}`);
    } finally {
      setLoaded(true);
    }
  };

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    loadProfiles();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleCreate = async () => {
    // `busy` guard: the button disables itself, but the input's Enter
    // handler calls this directly and must not double-submit.
    if (busy || !newName.trim()) return;
    setCreating(true);
    try {
      await invokeOrThrow<AddonProfile>("create_profile", {
        addonsPath,
        profileName: newName.trim(),
      });
      toast.success(`Profile "${newName.trim()}" created`);
      setNewName("");
      loadProfiles();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setCreating(false);
    }
  };

  const handlePreview = async (name: string) => {
    if (busy) return;
    setPreviewing(name);
    try {
      const plan = await invokeOrThrow<ProfilePlan>("preview_profile", {
        addonsPath,
        profileName: name,
      });
      const hasChanges =
        plan.toEnable.length > 0 || plan.toDisable.length > 0 || plan.blocked.length > 0;
      if (hasChanges) {
        setPreview({ name, plan });
      } else {
        // Nothing to rename — activate directly so the profile still becomes
        // the recorded active one, without a pointless confirmation step.
        await runActivation(name);
      }
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setPreviewing(null);
    }
  };

  const runActivation = async (name: string) => {
    // Re-entry guard + busy state must land BEFORE the first await: a second
    // click during the async ESO check below would otherwise start a
    // concurrent activation with interleaved renames.
    if (activating !== null) return;
    setActivating(name);
    try {
      // Same gate as install/update/remove: renaming addon folders while ESO
      // runs desyncs disk state from what the game loaded, so warn first.
      if (!(await ensureEsoNotBlocking())) return;
      const result = await invokeOrThrow<ActivateProfileResult>("activate_profile", {
        addonsPath,
        profileName: name,
      });
      const parts: string[] = [];
      if (result.enabled.length > 0) parts.push(`${result.enabled.length} enabled`);
      if (result.disabled.length > 0) parts.push(`${result.disabled.length} disabled`);
      toast.success(
        `Profile "${name}" activated${parts.length > 0 ? `: ${parts.join(", ")}` : ""}`
      );
      if (result.failed.length > 0) {
        toast.error(
          `Failed to rename ${result.failed.length} addon(s): ${result.failed.join(", ")}`
        );
      }
      if (result.missing.length > 0) {
        toast.info(
          `${result.missing.length} addon(s) from this profile are no longer installed: ${result.missing.join(", ")}`
        );
      }
      if (result.keptDependencies.length > 0) {
        toast.info(
          `Kept ${result.keptDependencies.length} required librar${result.keptDependencies.length === 1 ? "y" : "ies"} enabled: ${result.keptDependencies.join(", ")}`
        );
      }
      setActiveProfile(name);
      setPreview(null);
      onRefresh();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setActivating(null);
    }
  };

  const handleUpdate = async (name: string) => {
    setUpdating(true);
    try {
      await invokeOrThrow<AddonProfile>("update_profile", {
        addonsPath,
        profileName: name,
      });
      toast.success(`Profile "${name}" updated to the current setup`);
      loadProfiles();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setUpdating(false);
    }
  };

  const handleRenameSave = async (oldName: string) => {
    const newProfileName = renameValue.trim();
    if (!newProfileName || newProfileName === oldName) {
      setRenaming(null);
      return;
    }
    setRenameSaving(true);
    try {
      await invokeOrThrow("rename_profile", {
        addonsPath,
        oldName,
        newName: newProfileName,
      });
      toast.success(`Profile renamed to "${newProfileName}"`);
      setRenaming(null);
      loadProfiles();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setRenameSaving(false);
    }
  };

  const handleDelete = async (name: string) => {
    setDeleting(true);
    try {
      await invokeOrThrow("delete_profile", { addonsPath, profileName: name });
      toast.success(`Profile "${name}" deleted`);
      loadProfiles();
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Addon Profiles</DialogTitle>
          <DialogDescription>
            Save and switch between addon configurations. Activating a profile enables/disables
            addons by renaming folders.
          </DialogDescription>
        </DialogHeader>

        {preview ? (
          <Fade>
            <div className="space-y-3 rounded-xl border border-primary/25 bg-primary/[0.04] p-3">
              <p className="text-sm font-medium">
                Activate <span className="text-primary">{preview.name}</span>?
              </p>
              {preview.plan.toEnable.length > 0 && (
                <div className="text-xs text-emerald-400">
                  {preview.plan.toEnable.length} addon
                  {preview.plan.toEnable.length !== 1 ? "s" : ""} will be enabled
                  <PreviewNameList names={preview.plan.toEnable} />
                </div>
              )}
              {preview.plan.toDisable.length > 0 && (
                <div className="text-xs text-amber-400">
                  {preview.plan.toDisable.length} addon
                  {preview.plan.toDisable.length !== 1 ? "s" : ""} will be disabled
                  <PreviewNameList names={preview.plan.toDisable} />
                </div>
              )}
              {preview.plan.keptDependencies.length > 0 && (
                <div className="text-xs text-muted-foreground">
                  {preview.plan.keptDependencies.length} required librar
                  {preview.plan.keptDependencies.length === 1 ? "y" : "ies"} not in this profile
                  will stay enabled: {preview.plan.keptDependencies.join(", ")}
                </div>
              )}
              {preview.plan.missing.length > 0 && (
                <div className="text-xs text-muted-foreground">
                  {preview.plan.missing.length} addon
                  {preview.plan.missing.length !== 1 ? "s" : ""} from this profile{" "}
                  {preview.plan.missing.length !== 1 ? "are" : "is"} no longer installed:{" "}
                  {preview.plan.missing.join(", ")}
                </div>
              )}
              {preview.plan.blocked.length > 0 && (
                <div className="text-xs text-red-400">
                  Cannot disable {preview.plan.blocked.join(", ")} — both the enabled and disabled
                  folder copies exist. Remove the stale copy first.
                </div>
              )}
              <div className="flex justify-end gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={activating !== null}
                  onClick={() => setPreview(null)}
                >
                  Cancel
                </Button>
                <Button
                  size="sm"
                  disabled={activating !== null}
                  onClick={() => void runActivation(preview.name)}
                >
                  {activating ? "Activating..." : "Activate"}
                </Button>
              </div>
            </div>
          </Fade>
        ) : (
          <>
            <div className="flex gap-2">
              <Input
                placeholder="New profile name..."
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && handleCreate()}
                autoFocus
              />
              <Button onClick={handleCreate} disabled={busy || !newName.trim()} size="sm">
                {creating ? "Creating..." : "Save Current"}
              </Button>
            </div>

            <div className="border-t border-white/[0.06]" />

            <div className="max-h-[300px] overflow-y-auto space-y-2">
              {!loaded ? (
                <div className="flex justify-center py-6">
                  <div className="size-5 animate-spin rounded-full border-2 border-white/[0.1] border-t-primary" />
                </div>
              ) : profiles.length === 0 ? (
                <Fade>
                  <p className="text-sm text-muted-foreground text-center py-4">
                    No profiles yet. Save your current addon setup as a profile.
                  </p>
                </Fade>
              ) : (
                profiles.map((p, i) => {
                  const isActive = activeProfile === p.name;
                  const isModified = isActive && !matchesSnapshot(p.enabledAddons, enabledFolders);
                  const isExpanded = expanded === p.name;
                  return (
                    <Fade key={p.name} delay={i * 50}>
                      <div
                        className={cn(
                          "rounded-xl border border-white/[0.06] bg-white/[0.02] p-3 transition-all duration-200",
                          isActive && "border-primary/30 bg-primary/[0.04]"
                        )}
                      >
                        <div className="flex items-center justify-between">
                          <div className="min-w-0">
                            {renaming === p.name ? (
                              <div className="flex items-center gap-1">
                                <Input
                                  value={renameValue}
                                  onChange={(e) => setRenameValue(e.target.value)}
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter") void handleRenameSave(p.name);
                                    if (e.key === "Escape") setRenaming(null);
                                  }}
                                  className="h-7 text-sm"
                                  autoFocus
                                />
                                <Button
                                  size="icon-sm"
                                  variant="ghost"
                                  disabled={renameSaving}
                                  onClick={() => void handleRenameSave(p.name)}
                                  aria-label="Save name"
                                >
                                  <Check className="size-3.5 text-emerald-400" />
                                </Button>
                                <Button
                                  size="icon-sm"
                                  variant="ghost"
                                  disabled={renameSaving}
                                  onClick={() => setRenaming(null)}
                                  aria-label="Cancel rename"
                                >
                                  <X className="size-3.5" />
                                </Button>
                              </div>
                            ) : (
                              <div className="flex items-center gap-2">
                                <button
                                  type="button"
                                  className="flex items-center gap-1 min-w-0 text-left"
                                  onClick={() => setExpanded(isExpanded ? null : p.name)}
                                  aria-expanded={isExpanded}
                                  aria-label={`Show addons in ${p.name}`}
                                >
                                  <ChevronRight
                                    className={cn(
                                      "size-3.5 shrink-0 text-muted-foreground transition-transform duration-150",
                                      isExpanded && "rotate-90"
                                    )}
                                  />
                                  <span className="truncate font-medium text-sm">{p.name}</span>
                                </button>
                                {isActive && (
                                  <InfoPill color={isModified ? "amber" : "gold"}>
                                    {isModified ? "Active · modified" : "Active"}
                                  </InfoPill>
                                )}
                                <SimpleTooltip content="Rename profile">
                                  <Button
                                    size="icon-sm"
                                    variant="ghost"
                                    className="size-6"
                                    disabled={busy}
                                    onClick={() => {
                                      setRenaming(p.name);
                                      setRenameValue(p.name);
                                    }}
                                    aria-label={`Rename ${p.name}`}
                                  >
                                    <Pencil className="size-3" />
                                  </Button>
                                </SimpleTooltip>
                              </div>
                            )}
                            <div className="text-xs text-muted-foreground mt-0.5">
                              {p.enabledAddons.length} addons &middot; {p.createdAt}
                            </div>
                          </div>
                          <div className="flex gap-1 shrink-0">
                            <AnimatePresence mode="wait">
                              {confirmDelete === p.name ? (
                                <motion.div
                                  key="confirm-delete"
                                  initial={{ opacity: 0, scale: 0.95 }}
                                  animate={{ opacity: 1, scale: 1 }}
                                  exit={{ opacity: 0, scale: 0.95 }}
                                  transition={{ duration: 0.15 }}
                                  className="flex items-center gap-1"
                                >
                                  <span className="text-xs text-amber-400 mr-1">Delete?</span>
                                  <Button
                                    size="sm"
                                    variant="destructive"
                                    disabled={deleting}
                                    onClick={() => {
                                      setConfirmDelete(null);
                                      handleDelete(p.name);
                                    }}
                                  >
                                    {deleting ? "..." : "Yes"}
                                  </Button>
                                  <Button
                                    size="sm"
                                    variant="outline"
                                    disabled={deleting}
                                    onClick={() => setConfirmDelete(null)}
                                  >
                                    No
                                  </Button>
                                </motion.div>
                              ) : confirmUpdate === p.name ? (
                                <motion.div
                                  key="confirm-update"
                                  initial={{ opacity: 0, scale: 0.95 }}
                                  animate={{ opacity: 1, scale: 1 }}
                                  exit={{ opacity: 0, scale: 0.95 }}
                                  transition={{ duration: 0.15 }}
                                  className="flex items-center gap-1"
                                >
                                  <span className="text-xs text-amber-400 mr-1">Overwrite?</span>
                                  <Button
                                    size="sm"
                                    disabled={updating}
                                    onClick={() => {
                                      setConfirmUpdate(null);
                                      void handleUpdate(p.name);
                                    }}
                                  >
                                    {updating ? "..." : "Yes"}
                                  </Button>
                                  <Button
                                    size="sm"
                                    variant="outline"
                                    disabled={updating}
                                    onClick={() => setConfirmUpdate(null)}
                                  >
                                    No
                                  </Button>
                                </motion.div>
                              ) : (
                                <motion.div
                                  key="action-btns"
                                  initial={{ opacity: 0 }}
                                  animate={{ opacity: 1 }}
                                  exit={{ opacity: 0 }}
                                  transition={{ duration: 0.1 }}
                                  className="flex gap-1"
                                >
                                  <SimpleTooltip content="Overwrite this profile with the current addon setup">
                                    <Button
                                      size="icon-sm"
                                      variant="ghost"
                                      disabled={busy}
                                      onClick={() => setConfirmUpdate(p.name)}
                                      aria-label={`Update ${p.name} from current setup`}
                                    >
                                      <RefreshCw className="size-3.5" />
                                    </Button>
                                  </SimpleTooltip>
                                  <Button
                                    size="sm"
                                    variant={isActive ? "outline" : "default"}
                                    onClick={() => void handlePreview(p.name)}
                                    disabled={busy}
                                  >
                                    {previewing === p.name || activating === p.name
                                      ? "Activating..."
                                      : "Activate"}
                                  </Button>
                                  <Button
                                    size="sm"
                                    variant="destructive"
                                    disabled={busy}
                                    onClick={() => setConfirmDelete(p.name)}
                                  >
                                    Delete
                                  </Button>
                                </motion.div>
                              )}
                            </AnimatePresence>
                          </div>
                        </div>
                        {isExpanded && (
                          <Fade>
                            <div className="mt-2 max-h-28 overflow-y-auto rounded-lg bg-white/[0.03] px-2 py-1.5 text-xs text-muted-foreground">
                              {p.enabledAddons.length > 0
                                ? p.enabledAddons.join(", ")
                                : "No addons in this profile."}
                            </div>
                          </Fade>
                        )}
                      </div>
                    </Fade>
                  );
                })
              )}
            </div>
          </>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
