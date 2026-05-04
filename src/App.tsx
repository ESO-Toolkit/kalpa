import { useEffect, useState, useCallback, useRef, useMemo } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { useAppUpdate } from "./components/app-update";
import { AddonList } from "./components/addon-list";
import { AddonDetail } from "./components/addon-detail";
import { AppBackground } from "./components/app-background";
import { AppDialogs } from "./components/app-dialogs";
import { AppHeader } from "./components/app-header";
import { DiscoverDetail } from "./components/discover-detail";
import { SetupWizard } from "./components/setup-wizard";
import { StatusBanners } from "./components/status-banners";
import { RosterPackInstall } from "./components/roster-pack-install";
import { UpdateBanner } from "./components/update-banner";
import { getSetting, setSetting } from "@/lib/store";
import { getTauriErrorMessage, invokeOrThrow, invokeResult } from "@/lib/tauri";
import type {
  AddonManifest,
  AuthUser,
  BatchConflictAddon,
  BatchConflictResult,
  GameInstance,
  InstallResult,
  UpdateCheckResult,
  EsouiSearchResult,
  SortMode,
  FilterMode,
  ViewMode,
  DiscoverTab,
} from "./types";

type ActiveDialog =
  | "settings"
  | "profiles"
  | "packs"
  | "backups"
  | "api-compat"
  | "characters"
  | "saved-variables"
  | "migration-wizard"
  | "safety-center"
  | null;

interface PendingDeepLinkPayload {
  packId: string | null;
  shareCode: string | null;
  installPackId: string | null;
}

const VALID_FILTER_MODES: readonly FilterMode[] = [
  "all",
  "addons",
  "libraries",
  "outdated",
  "missing-deps",
  "favorites",
  "disabled",
];

function isFilterMode(value: string): value is FilterMode {
  return (VALID_FILTER_MODES as readonly string[]).includes(value);
}

function App() {
  const [addonsPath, setAddonsPath] = useState("");
  const [addons, setAddons] = useState<AddonManifest[]>([]);
  const [selectedAddon, setSelectedAddon] = useState<AddonManifest | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [errorShowSettings, setErrorShowSettings] = useState(false);
  const [isOffline, setIsOffline] = useState(!navigator.onLine);
  const [activeDialog, setActiveDialog] = useState<ActiveDialog>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [updateResults, setUpdateResults] = useState<UpdateCheckResult[]>([]);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [updatingAll, setUpdatingAll] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<{
    completed: number;
    failed: number;
    total: number;
    currentAddon?: string;
  } | null>(null);
  const [addonStatuses, setAddonStatuses] = useState<
    Map<string, "downloading" | "scanning" | "extracting" | "completed" | "failed">
  >(new Map());
  const [pendingConflicts, setPendingConflicts] = useState<Map<string, BatchConflictAddon>>(
    new Map()
  );
  const [sortMode, setSortMode] = useState<SortMode>("name");
  const [filterMode, setFilterMode] = useState<FilterMode>("all");
  const [authUser, setAuthUser] = useState<AuthUser | null>(null);
  const [deepLinkPackId, setDeepLinkPackId] = useState<string | null>(null);
  const [deepLinkShareCode, setDeepLinkShareCode] = useState<string | null>(null);
  const [rosterPackInstallId, setRosterPackInstallId] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("installed");
  const [discoverTab, setDiscoverTab] = useState<DiscoverTab>("search");
  const [selectedDiscoverResult, setSelectedDiscoverResult] = useState<EsouiSearchResult | null>(
    null
  );
  const [selectedFolders, setSelectedFolders] = useState<Set<string>>(new Set());
  const [batchDisabling, setBatchDisabling] = useState(false);
  const [activeTagFilter, setActiveTagFilter] = useState<string | null>(null);
  // null = not in setup mode; [] = in setup mode with no candidates found
  const [setupInstances, setSetupInstances] = useState<GameInstance[] | null>(null);
  // All detected instances; available after init for the instance switcher in Settings
  const [knownInstances, setKnownInstances] = useState<GameInstance[]>([]);

  const [srAnnouncement, setSrAnnouncement] = useState("");

  const srAnnounce = useCallback((message: string) => {
    setSrAnnouncement("");
    requestAnimationFrame(() => setSrAnnouncement(message));
  }, []);

  const {
    state: appUpdateState,
    checkForAppUpdate,
    downloadAndInstall,
    restartApp,
  } = useAppUpdate();

  const initRan = useRef(false);
  const autoLinkRan = useRef(false);
  const selectedAddonRef = useRef<AddonManifest | null>(null);
  const addonsPathRef = useRef("");
  const viewModeRef = useRef<ViewMode>("installed");
  const updatingAllRef = useRef(false);
  const runBatchUpdatesRef = useRef<((updates: UpdateCheckResult[]) => Promise<void>) | null>(null);
  const scanSeqRef = useRef(0);
  const checkSeqRef = useRef(0);

  useEffect(() => {
    selectedAddonRef.current = selectedAddon;
    addonsPathRef.current = addonsPath;
    viewModeRef.current = viewMode;
    updatingAllRef.current = updatingAll;
  });

  useEffect(() => {
    const goOffline = () => setIsOffline(true);
    const goOnline = () => setIsOffline(false);
    window.addEventListener("offline", goOffline);
    window.addEventListener("online", goOnline);
    return () => {
      window.removeEventListener("offline", goOffline);
      window.removeEventListener("online", goOnline);
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const cleanups: (() => void)[] = [];

    void listen<string>("deep-link-pack", (event) => {
      setDeepLinkPackId(event.payload);
      setActiveDialog("packs");
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        cleanups.push(unlisten);
      })
      .catch((listenError) => {
        console.error("[tauri:deep-link-pack]", listenError);
      });

    void listen<string>("roster-pack-install", (event) => {
      setRosterPackInstallId(event.payload);
      setActiveDialog(null); // close packs dialog if open to avoid stacking
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        cleanups.push(unlisten);
      })
      .catch((listenError) => {
        console.error("[tauri:roster-pack-install]", listenError);
      });

    void listen<string>("deep-link-share", (event) => {
      setDeepLinkShareCode(event.payload);
      setActiveDialog("packs");
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        cleanups.push(unlisten);
      })
      .catch((listenError) => {
        console.error("[tauri:deep-link-share]", listenError);
      });

    void listen<{ folderName: string; phase: string; index: number; total: number }>(
      "batch-update-progress",
      (event) => {
        const { folderName, phase, total } = event.payload;
        setAddonStatuses((prev) => {
          const next = new Map(prev);
          next.set(
            folderName,
            phase as "downloading" | "scanning" | "extracting" | "completed" | "failed"
          );
          return next;
        });
        // Keep legacy progress in sync
        let completed = 0;
        let failed = 0;
        setAddonStatuses((current) => {
          for (const s of current.values()) {
            if (s === "completed") completed++;
            if (s === "failed") failed++;
          }
          setUpdateProgress({ completed, failed, total, currentAddon: folderName });
          return current;
        });
      }
    )
      .then((unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        cleanups.push(unlisten);
      })
      .catch((listenError) => {
        console.error("[tauri:batch-update-progress]", listenError);
      });

    void invokeOrThrow<PendingDeepLinkPayload>("consume_initial_deep_link")
      .then((payload) => {
        if (disposed) return;
        if (payload.installPackId) {
          setRosterPackInstallId(payload.installPackId);
        } else if (payload.packId) {
          setDeepLinkPackId(payload.packId);
          setActiveDialog("packs");
        } else if (payload.shareCode) {
          setDeepLinkShareCode(payload.shareCode);
          setActiveDialog("packs");
        }
      })
      .catch((invokeError) => {
        console.error("[tauri:consume_initial_deep_link]", invokeError);
      });

    return () => {
      disposed = true;
      for (const fn of cleanups) fn();
    };
  }, []);

  const scanAddons = useCallback(async (path: string) => {
    const seq = ++scanSeqRef.current;
    setLoading(true);
    setError(null);
    setErrorShowSettings(false);

    try {
      const result = await invokeOrThrow<AddonManifest[]>("scan_installed_addons", {
        addonsPath: path,
      });
      if (seq !== scanSeqRef.current) return;

      setAddons(result);
      if (selectedAddonRef.current) {
        const updated = result.find(
          (addon) => addon.folderName === selectedAddonRef.current?.folderName
        );
        setSelectedAddon(updated ?? null);
      }
    } catch (scanError) {
      if (seq !== scanSeqRef.current) return;
      setError(getTauriErrorMessage(scanError));
      setAddons([]);
    } finally {
      if (seq === scanSeqRef.current) {
        setLoading(false);
      }
    }
  }, []);

  const checkForUpdates = useCallback(
    async (path: string, autoUpdate = false, notifyOnError = false) => {
      const seq = ++checkSeqRef.current;
      setUpdatingAll(false);
      setUpdateProgress(null);
      setCheckingUpdates(true);
      try {
        const results = await invokeOrThrow<UpdateCheckResult[]>("check_for_updates", {
          addonsPath: path,
        });
        if (seq !== checkSeqRef.current) return;

        setUpdateResults(results);
        const updates = results.filter((result) => result.hasUpdate);

        void invokeResult("update_tray_tooltip", { updateCount: updates.length });

        if (updates.length > 0) {
          srAnnounce(`${updates.length} update${updates.length !== 1 ? "s" : ""} available`);
        }

        if (autoUpdate && updates.length > 0) {
          toast.info(`Auto-updating ${updates.length} addon${updates.length > 1 ? "s" : ""}...`);
          await runBatchUpdatesRef.current?.(updates);
        }
      } catch (updateError) {
        if (seq !== checkSeqRef.current) return;
        console.error("[tauri:check_for_updates]", updateError);
        if (notifyOnError) {
          toast.error(`Failed to check for updates: ${getTauriErrorMessage(updateError)}`);
        }
      } finally {
        if (seq === checkSeqRef.current) {
          setCheckingUpdates(false);
        }
      }
    },
    [srAnnounce]
  );

  const scanAndCheck = useCallback(
    async (path: string, notifyOnUpdateError = false) => {
      await scanAddons(path);
      await checkForUpdates(path, false, notifyOnUpdateError);
    },
    [checkForUpdates, scanAddons]
  );

  const runAutoLink = useCallback(
    async (path: string) => {
      if (autoLinkRan.current) return;
      autoLinkRan.current = true;

      const result = await invokeResult<{ linked: string[]; notFound: string[] }>(
        "auto_link_addons",
        {
          addonsPath: path,
        }
      );

      if (!result.ok) {
        toast.error(`Auto-link failed: ${result.error}`);
        return;
      }

      if (result.data.linked.length > 0) {
        toast.success(
          `Auto-linked ${result.data.linked.length} addon${result.data.linked.length > 1 ? "s" : ""} to ESOUI`
        );
        await scanAddons(path);
      }
    },
    [scanAddons]
  );

  const initializeApp = useCallback(async () => {
    const savedSort = await getSetting<SortMode>("sortMode", "name");
    const savedFilter = await getSetting<string>("filterMode", "all");
    const normalizedFilter = isFilterMode(savedFilter) ? savedFilter : "all";
    setSortMode(savedSort);
    setFilterMode(normalizedFilter);
    if (normalizedFilter !== savedFilter) {
      void setSetting("filterMode", normalizedFilter);
    }

    const authResult = await invokeResult<AuthUser | null>("auth_get_user");
    if (authResult.ok) {
      setAuthUser(authResult.data ?? null);
    } else {
      toast.error(`Could not restore sign-in: ${authResult.error}`);
    }

    const savedPath = await getSetting<string>("addonsPath", "");

    if (savedPath) {
      // Saved path exists — use it directly
      try {
        setAddonsPath(savedPath);
        await invokeOrThrow("set_addons_path", { addonsPath: savedPath });
        await scanAddons(savedPath);
        const autoUpdate = await getSetting<boolean>("autoUpdate", false);
        await checkForUpdates(savedPath, autoUpdate, false);
        void runAutoLink(savedPath);
        // Populate knownInstances so the Settings instance switcher works for
        // returning users. Fire-and-forget — does not block startup.
        invokeOrThrow<GameInstance[]>("detect_game_instances")
          .then(setKnownInstances)
          .catch(() => {});
      } catch (initError) {
        setError(
          `Could not access saved AddOns folder — it may have been moved or deleted. ${getTauriErrorMessage(initError)}`
        );
        setErrorShowSettings(true);
        setLoading(false);
      }
    } else {
      // No saved path — run detection and show wizard or auto-select
      const instances = await invokeOrThrow<GameInstance[]>("detect_game_instances");
      setKnownInstances(instances);

      const singleClean = instances.length === 1 && !instances[0].isOnedrive;

      if (singleClean) {
        // One unambiguous instance with no OneDrive complication — auto-select
        try {
          const path = instances[0].addonsPath;
          setAddonsPath(path);
          await invokeOrThrow("set_addons_path", { addonsPath: path });
          await setSetting("addonsPath", path);
          await scanAddons(path);
          const autoUpdate = await getSetting<boolean>("autoUpdate", false);
          await checkForUpdates(path, autoUpdate, false);
          void runAutoLink(path);
        } catch (initError) {
          setError(`Could not access detected AddOns folder. ${getTauriErrorMessage(initError)}`);
          setErrorShowSettings(true);
          setLoading(false);
        }
      } else {
        // Multiple instances, OneDrive warning, or nothing found — show wizard
        setSetupInstances(instances);
        setLoading(false);
      }
    }
  }, [checkForUpdates, runAutoLink, scanAddons]);

  useEffect(() => {
    if (initRan.current) return;
    initRan.current = true;
    void initializeApp();
  }, [initializeApp]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.key === "r") {
        event.preventDefault();
        if (addonsPathRef.current && !updatingAllRef.current) {
          void scanAndCheck(addonsPathRef.current, true);
        }
      }

      if (event.ctrlKey && event.key === "i") {
        event.preventDefault();
        setViewMode("discover");
        setDiscoverTab("url");
      }

      if (event.ctrlKey && event.key === "b") {
        event.preventDefault();
        setViewMode("discover");
        setDiscoverTab("search");
      }

      if (event.key === "Escape") {
        if (viewModeRef.current === "discover") {
          setViewMode("installed");
        }
        setSelectedFolders(new Set());
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [scanAndCheck]);

  // Notify the user if a deep link is pending while the setup wizard is shown
  const pendingDeepLinkToastShown = useRef(false);
  useEffect(() => {
    if (setupInstances === null) return;
    if (pendingDeepLinkToastShown.current) return;
    if (rosterPackInstallId || deepLinkPackId || deepLinkShareCode) {
      pendingDeepLinkToastShown.current = true;
      toast.info("Finish setup to continue with the incoming link.");
    }
  }, [setupInstances, rosterPackInstallId, deepLinkPackId, deepLinkShareCode]);

  const effectiveTagFilter =
    activeTagFilter && addons.some((a) => a.tags.includes(activeTagFilter))
      ? activeTagFilter
      : null;

  const handleSetupSelect = useCallback(
    async (selectedPath: string) => {
      const path = selectedPath.trim();
      if (!path) return;

      try {
        await invokeOrThrow("set_addons_path", { addonsPath: path });
        await setSetting("addonsPath", path);
        setAddonsPath(path);
        setSetupInstances(null);
        setErrorShowSettings(false);
        setLoading(true);
        await scanAddons(path);
        const autoUpdate = await getSetting<boolean>("autoUpdate", false);
        await checkForUpdates(path, autoUpdate, false);
        void runAutoLink(path);
      } catch (pathError) {
        const message = getTauriErrorMessage(pathError);
        setError(`Could not set addons folder: ${message}`);
        setErrorShowSettings(true);
      }
    },
    [checkForUpdates, runAutoLink, scanAddons]
  );

  const handleRefresh = useCallback(() => {
    if (addonsPathRef.current) {
      void scanAndCheck(addonsPathRef.current, true);
    }
  }, [scanAndCheck]);

  const handleTagsChange = useCallback(
    async (folderName: string, tags: string[]) => {
      try {
        await invokeOrThrow("set_addon_tags", { addonsPath, folderName, tags });
        setAddons((prev) =>
          prev.map((addon) => (addon.folderName === folderName ? { ...addon, tags } : addon))
        );
        setSelectedAddon((prev) => (prev?.folderName === folderName ? { ...prev, tags } : prev));
      } catch (tagsError) {
        toast.error(`Failed to save tags: ${getTauriErrorMessage(tagsError)}`);
      }
    },
    [addonsPath]
  );

  const handleToggleDisable = useCallback(
    async (folderName: string, currentlyDisabled: boolean) => {
      const command = currentlyDisabled ? "enable_addon" : "disable_addon";
      const result = await invokeResult(command, { addonsPath, folderName });
      if (result.ok) {
        toast.success(currentlyDisabled ? `Enabled ${folderName}` : `Disabled ${folderName}`);
        handleRefresh();
      } else {
        toast.error(result.error);
      }
    },
    [addonsPath, handleRefresh]
  );

  const handleAddonUpdated = useCallback(
    (esouiId: number) => {
      if (addonsPathRef.current) {
        void scanAddons(addonsPathRef.current);
      }
      setUpdateResults((prev) => prev.filter((result) => result.esouiId !== esouiId));
    },
    [scanAddons]
  );

  const handleOpenFolder = useCallback(
    async (folderName: string) => {
      try {
        const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
        await revealItemInDir(`${addonsPath}\\${folderName}`);
      } catch (e) {
        toast.error(`Could not open folder: ${getTauriErrorMessage(e)}`);
      }
    },
    [addonsPath]
  );

  const handleSingleUpdate = useCallback(
    async (folderName: string) => {
      const ur = updateResults.find((r) => r.folderName === folderName && r.hasUpdate);
      if (!ur) return;
      try {
        await invokeOrThrow("update_addon", {
          addonsPath,
          esouiId: ur.esouiId,
        });
        toast.success(`Updated ${folderName}`);
        srAnnounce(`Updated ${folderName}`);
        handleAddonUpdated(ur.esouiId);
      } catch (e) {
        toast.error(`Update failed: ${getTauriErrorMessage(e)}`);
      }
    },
    [addonsPath, updateResults, srAnnounce, handleAddonUpdated]
  );

  const pendingRemovalsRef = useRef<
    Map<string, { timer: ReturnType<typeof setTimeout>; addon: AddonManifest }>
  >(new Map());

  const flushPendingRemovals = useCallback(() => {
    for (const [folderName, { timer }] of pendingRemovalsRef.current) {
      clearTimeout(timer);
      void invokeOrThrow("remove_addon", { addonsPath, folderName }).catch(() => {});
    }
    pendingRemovalsRef.current.clear();
  }, [addonsPath]);

  useEffect(() => {
    const handler = () => flushPendingRemovals();
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [flushPendingRemovals]);

  const handleSingleRemove = useCallback(
    (folderName: string) => {
      const addon = addons.find((a) => a.folderName === folderName);
      if (!addon) return;

      // Optimistically hide from UI
      setAddons((prev) => prev.filter((a) => a.folderName !== folderName));
      setUpdateResults((prev) => prev.filter((r) => r.folderName !== folderName));
      setSelectedFolders((prev) => {
        if (!prev.has(folderName)) return prev;
        const next = new Set(prev);
        next.delete(folderName);
        return next;
      });
      if (selectedAddonRef.current?.folderName === folderName) {
        setSelectedAddon(null);
      }

      // Cancel any existing pending removal for this addon
      const existing = pendingRemovalsRef.current.get(folderName);
      if (existing) clearTimeout(existing.timer);

      const timer = setTimeout(() => {
        pendingRemovalsRef.current.delete(folderName);
        void invokeOrThrow("remove_addon", { addonsPath, folderName }).catch((e) => {
          toast.error(`Remove failed: ${getTauriErrorMessage(e)}`);
          setAddons((prev) => [...prev, addon]);
        });
      }, 3000);

      pendingRemovalsRef.current.set(folderName, { timer, addon });

      toast(`Removed ${addon.title}`, {
        action: {
          label: "Undo",
          onClick: () => {
            const pending = pendingRemovalsRef.current.get(folderName);
            if (pending) {
              clearTimeout(pending.timer);
              pendingRemovalsRef.current.delete(folderName);
              setAddons((prev) => [...prev, addon]);
              toast.success(`Restored ${addon.title}`);
            }
          },
        },
        duration: 3000,
      });
      srAnnounce(`Removed ${addon.title}. Press undo to restore.`);
    },
    [addons, addonsPath, srAnnounce]
  );

  const handleRemoveByEsouiId = useCallback(
    (esouiId: number) => {
      for (const addon of addons.filter((a) => a.esouiId === esouiId)) {
        handleSingleRemove(addon.folderName);
      }
    },
    [addons, handleSingleRemove]
  );

  const handlePathChange = useCallback(
    async (newPath: string) => {
      const nextPath = newPath.trim();
      if (!nextPath) return;

      try {
        await invokeOrThrow("set_addons_path", { addonsPath: nextPath });
        await setSetting("addonsPath", nextPath);
        setAddonsPath(nextPath);
        setSelectedAddon(null);
        setUpdateResults([]);
        setError(null);
        setErrorShowSettings(false);
        await scanAndCheck(nextPath, true);
      } catch (pathError) {
        const message = getTauriErrorMessage(pathError);
        setError(`Could not set addons folder: ${message}`);
        setErrorShowSettings(true);
      }
    },
    [scanAndCheck]
  );

  const handleSortChange = useCallback((mode: SortMode) => {
    setSortMode(mode);
    void setSetting("sortMode", mode);
  }, []);

  const handleFilterChange = useCallback((mode: FilterMode) => {
    setFilterMode(mode);
    void setSetting("filterMode", mode);
  }, []);

  const { updatesAvailable, updatesSet } = useMemo(() => {
    const available = updateResults.filter((result) => result.hasUpdate);
    return {
      updatesAvailable: available,
      updatesSet: new Set(available.map((result) => result.folderName)),
    };
  }, [updateResults]);

  const installedEsouiIds = useMemo(() => {
    const ids = new Set<number>();
    for (const addon of addons) {
      if (addon.esouiId != null) ids.add(addon.esouiId);
    }
    return ids;
  }, [addons]);

  const runBatchUpdates = useCallback(
    async (updates: UpdateCheckResult[]) => {
      const path = addonsPathRef.current;
      if (!path || updates.length === 0) return;

      if (updatingAllRef.current) return;

      setUpdatingAll(true);
      setUpdateProgress({ completed: 0, failed: 0, total: updates.length });
      setAddonStatuses(new Map());

      void invokeResult("create_pre_operation_snapshot", {
        addonsPath: path,
        operationLabel: "update-all",
      });

      // Phase 1: Scan all addons for conflicts (downloads ZIPs in parallel on Rust side)
      const scanResult = await invokeResult<BatchConflictResult>("scan_batch_conflicts", {
        addonsPath: path,
        updates: updates.map((u) => ({
          esouiId: u.esouiId,
          folderName: u.folderName,
          apiVersion: u.remoteVersion,
        })),
      });

      if (!scanResult.ok) {
        setUpdatingAll(false);
        setUpdateProgress(null);
        toast.error(`Batch update failed: ${scanResult.error}`);
        srAnnounce("Batch update failed");
        return;
      }

      const { noConflictAddons, conflictingAddons, failed: scanFailed } = scanResult.data;
      const total = noConflictAddons.length + conflictingAddons.length + scanFailed.length;
      const completed: string[] = [];
      const failed: string[] = [...scanFailed];

      // Phase 2: Update non-conflicting addons sequentially
      for (let i = 0; i < noConflictAddons.length; i++) {
        const addon = noConflictAddons[i];
        setAddonStatuses((prev) => {
          const next = new Map(prev);
          next.set(addon.folderName, "extracting");
          return next;
        });

        const result = await invokeResult<InstallResult>("update_addon_with_decisions", {
          addonsPath: path,
          sessionId: addon.sessionId,
          decisions: [],
        });

        if (result.ok) {
          completed.push(addon.folderName);
          setAddonStatuses((prev) => {
            const next = new Map(prev);
            next.set(addon.folderName, "completed");
            return next;
          });
        } else {
          failed.push(addon.folderName);
          setAddonStatuses((prev) => {
            const next = new Map(prev);
            next.set(addon.folderName, "failed");
            return next;
          });
        }

        setUpdateProgress({
          completed: completed.length,
          failed: failed.length,
          total,
          currentAddon: addon.folderName,
        });
      }

      // Handle conflicting addons based on user's policy preference
      const remainingConflicts: typeof conflictingAddons = [];
      if (conflictingAddons.length > 0) {
        const policy = await getSetting<"ask" | "keep_mine" | "take_update">(
          "conflictPolicy",
          "ask"
        );

        if (policy !== "ask") {
          for (const ca of conflictingAddons) {
            const autoDecisions = ca.conflicts.map((c) => ({
              relativePath: c.relativePath,
              action: policy as "keep_mine" | "take_update",
            }));
            const result = await invokeResult<InstallResult>("update_addon_with_decisions", {
              addonsPath: path,
              sessionId: ca.sessionId,
              decisions: autoDecisions,
            });
            if (result.ok) {
              completed.push(ca.folderName);
            } else {
              failed.push(ca.folderName);
            }
          }
        } else {
          remainingConflicts.push(...conflictingAddons);
        }
      }

      setUpdatingAll(false);
      setUpdateProgress(null);

      if (remainingConflicts.length > 0) {
        setPendingConflicts((prev) => {
          const next = new Map(prev);
          for (const ca of remainingConflicts) {
            next.set(ca.folderName, ca);
          }
          return next;
        });
      }

      // Summary toast
      const conflictCount = remainingConflicts.length;
      if (completed.length > 0 || failed.length > 0) {
        let msg = `Updated ${completed.length} addon${completed.length !== 1 ? "s" : ""}`;
        if (failed.length > 0) msg += `, ${failed.length} failed`;
        if (conflictCount > 0)
          msg += `, ${conflictCount} need${conflictCount === 1 ? "s" : ""} your attention`;
        if (failed.length > 0) {
          toast.warning(msg);
        } else if (conflictCount > 0) {
          toast.info(msg);
        } else {
          toast.success(msg);
        }
        srAnnounce(msg);
      } else if (conflictCount > 0) {
        toast.info(`${conflictCount} addon${conflictCount !== 1 ? "s" : ""} need your attention`);
      }

      if (completed.length > 0) {
        const succeededNames = new Set(completed);
        setUpdateResults((prev) => {
          const remaining = prev.filter((result) => !succeededNames.has(result.folderName));
          const remainingUpdates = remaining.filter((r) => r.hasUpdate).length;
          void invokeResult("update_tray_tooltip", {
            updateCount: remainingUpdates,
          });
          return remaining;
        });

        void (async () => {
          try {
            const { getCurrentWindow } = await import("@tauri-apps/api/window");
            const isVisible = await getCurrentWindow().isVisible();
            if (!isVisible) {
              const { isPermissionGranted, sendNotification } =
                await import("@tauri-apps/plugin-notification");
              if (await isPermissionGranted()) {
                sendNotification({
                  title: "Kalpa",
                  body: `Updated ${completed.length} addon${completed.length !== 1 ? "s" : ""}${failed.length > 0 ? `, ${failed.length} failed` : ""}${conflictCount > 0 ? `, ${conflictCount} need review` : ""}`,
                });
              }
            }
          } catch {
            // Notification is best-effort
          }
        })();
      }

      await scanAddons(path);
    },
    [scanAddons, srAnnounce]
  );

  useEffect(() => {
    runBatchUpdatesRef.current = runBatchUpdates;
  }, [runBatchUpdates]);

  const handleUpdateAll = useCallback(() => {
    void runBatchUpdates(updatesAvailable);
  }, [runBatchUpdates, updatesAvailable]);

  const handleToggleSelect = useCallback((folderName: string) => {
    setSelectedFolders((prev) => {
      const next = new Set(prev);
      if (next.has(folderName)) {
        next.delete(folderName);
      } else {
        next.add(folderName);
      }
      return next;
    });
  }, []);

  const handleBatchRemove = useCallback(() => {
    if (selectedFolders.size === 0) return;

    const removedAddons = addons.filter((a) => selectedFolders.has(a.folderName));
    const folderNames = Array.from(selectedFolders);
    const count = removedAddons.length;

    const removedSet = new Set(folderNames);

    // Optimistically hide from all relevant state
    setAddons((prev) => prev.filter((a) => !removedSet.has(a.folderName)));
    setUpdateResults((prev) => prev.filter((r) => !removedSet.has(r.folderName)));
    if (selectedAddonRef.current && selectedFolders.has(selectedAddonRef.current.folderName)) {
      setSelectedAddon(null);
    }
    setSelectedFolders(new Set());

    for (const addon of removedAddons) {
      const existing = pendingRemovalsRef.current.get(addon.folderName);
      if (existing) clearTimeout(existing.timer);
    }

    const timer = setTimeout(() => {
      for (const fn of folderNames) pendingRemovalsRef.current.delete(fn);
      void invokeOrThrow<string[]>("batch_remove_addons", {
        addonsPath,
        folderNames,
      }).catch((e) => {
        toast.error(`Batch remove failed: ${getTauriErrorMessage(e)}`);
        setAddons((prev) => [...prev, ...removedAddons]);
      });
    }, 3000);

    for (const addon of removedAddons) {
      pendingRemovalsRef.current.set(addon.folderName, { timer, addon });
    }

    toast(`Removed ${count} addon${count !== 1 ? "s" : ""}`, {
      action: {
        label: "Undo",
        onClick: () => {
          clearTimeout(timer);
          for (const fn of folderNames) pendingRemovalsRef.current.delete(fn);
          setAddons((prev) => [...prev, ...removedAddons]);
          toast.success(`Restored ${count} addon${count !== 1 ? "s" : ""}`);
        },
      },
      duration: 3000,
    });
    srAnnounce(`Removed ${count} addon${count !== 1 ? "s" : ""}. Press undo to restore.`);
  }, [addons, addonsPath, selectedFolders, srAnnounce]);

  const handleBatchUpdate = useCallback(async () => {
    const toUpdate = updatesAvailable.filter((update) => selectedFolders.has(update.folderName));
    if (toUpdate.length === 0) {
      toast.info("No selected addons have updates available");
      return;
    }

    await runBatchUpdates(toUpdate);
    setSelectedFolders(new Set());
  }, [runBatchUpdates, selectedFolders, updatesAvailable]);

  const handleBatchDisable = useCallback(async () => {
    if (selectedFolders.size === 0) return;

    setBatchDisabling(true);
    let disabled = 0;
    let enabled = 0;
    let failed = 0;

    for (const folderName of selectedFolders) {
      const addon = addons.find((a) => a.folderName === folderName);
      if (!addon) continue;
      const command = addon.disabled ? "enable_addon" : "disable_addon";
      const result = await invokeResult(command, { addonsPath, folderName });
      if (result.ok) {
        if (addon.disabled) enabled++;
        else disabled++;
      } else {
        failed++;
      }
    }

    setBatchDisabling(false);

    const parts: string[] = [];
    if (disabled > 0) parts.push(`disabled ${disabled}`);
    if (enabled > 0) parts.push(`enabled ${enabled}`);
    if (failed > 0) parts.push(`${failed} failed`);
    toast.success(parts.join(", "));

    setSelectedFolders(new Set());
    handleRefresh();
  }, [addons, addonsPath, handleRefresh, selectedFolders]);

  const handleBatchTag = useCallback(
    async (tag: string) => {
      if (selectedFolders.size === 0) return;

      let applied = 0;
      for (const folderName of selectedFolders) {
        const addon = addons.find((a) => a.folderName === folderName);
        if (!addon) continue;
        if (addon.tags.includes(tag)) continue;
        try {
          await invokeOrThrow("set_addon_tags", {
            addonsPath,
            folderName,
            tags: [...addon.tags, tag],
          });
          applied++;
        } catch {
          // skip individual failures
        }
      }

      if (applied > 0) {
        toast.success(`Tagged ${applied} addon${applied !== 1 ? "s" : ""} as "${tag}"`);
      } else {
        toast.info(`All selected addons already have the "${tag}" tag`);
      }

      setSelectedFolders(new Set());
      handleRefresh();
    },
    [addons, addonsPath, handleRefresh, selectedFolders]
  );

  const filteredAddons = useMemo(
    () =>
      addons
        .filter((addon) => {
          if (searchQuery) {
            const query = searchQuery.toLowerCase();
            const matchesSearch =
              addon.title.toLowerCase().includes(query) ||
              addon.folderName.toLowerCase().includes(query) ||
              addon.author.toLowerCase().includes(query) ||
              addon.tags.some((tag) => tag.toLowerCase().includes(query));
            if (!matchesSearch) return false;
          }

          switch (filterMode) {
            case "addons":
              return !addon.isLibrary;
            case "libraries":
              return addon.isLibrary;
            case "outdated":
              return updatesSet.has(addon.folderName);
            case "missing-deps":
              return addon.missingDependencies.length > 0;
            case "favorites":
              return addon.tags.includes("favorite");
            case "disabled":
              return addon.disabled;
            default:
              if (effectiveTagFilter) return addon.tags.includes(effectiveTagFilter);
              return true;
          }
        })
        .sort((left, right) => {
          switch (sortMode) {
            case "author":
              return left.author.toLowerCase().localeCompare(right.author.toLowerCase());
            case "name":
            default:
              return left.title.toLowerCase().localeCompare(right.title.toLowerCase());
          }
        }),
    [effectiveTagFilter, addons, filterMode, searchQuery, sortMode, updatesSet]
  );

  const selectedUpdateResult = useMemo(
    () =>
      selectedAddon
        ? (updateResults.find((result) => result.folderName === selectedAddon.folderName) ?? null)
        : null,
    [selectedAddon, updateResults]
  );

  const batchMode = selectedFolders.size > 0;

  const handleOpenDialog = useCallback((dialog: Exclude<ActiveDialog, null>) => {
    setActiveDialog(dialog);
  }, []);

  const handleCloseDialog = useCallback(() => {
    setActiveDialog(null);
    setDeepLinkPackId(null);
    setDeepLinkShareCode(null);
  }, []);

  if (setupInstances !== null) {
    return (
      <div className="relative flex h-screen flex-col">
        <AppBackground />
        <SetupWizard instances={setupInstances} onSelect={(path) => void handleSetupSelect(path)} />
      </div>
    );
  }

  return (
    <div className="relative flex h-screen flex-col">
      <div className="sr-only" aria-live="assertive" aria-atomic="true" role="status">
        {srAnnouncement}
      </div>
      <AppBackground />

      <AppHeader
        addonsCount={addons.length}
        batchMode={batchMode}
        batchDisabling={batchDisabling}
        checkingUpdates={checkingUpdates}
        loading={loading}
        selectedCount={selectedFolders.size}
        updatingAll={updatingAll}
        isOffline={isOffline}
        onBatchCancel={() => setSelectedFolders(new Set())}
        onBatchDisable={() => void handleBatchDisable()}
        onBatchRemove={() => void handleBatchRemove()}
        onBatchTag={handleBatchTag}
        onBatchUpdate={() => void handleBatchUpdate()}
        onOpenPacks={() => setActiveDialog("packs")}
        onOpenSavedVars={() => setActiveDialog("saved-variables")}
        onOpenSettings={() => setActiveDialog("settings")}
        onRefresh={handleRefresh}
      />

      <StatusBanners
        error={error}
        isOffline={isOffline}
        appUpdateState={appUpdateState}
        onDownload={downloadAndInstall}
        onRestart={restartApp}
        onOpenSettings={errorShowSettings ? () => setActiveDialog("settings") : undefined}
      />

      <UpdateBanner
        availableCount={updatesAvailable.length}
        updatingAll={updatingAll}
        updateProgress={updateProgress}
        addonStatuses={addonStatuses}
        onUpdateAll={handleUpdateAll}
        isOffline={isOffline}
      />

      {pendingConflicts.size > 0 && (
        <div className="mx-4 mb-2 flex items-center gap-2 rounded-lg border border-amber-500/20 bg-amber-500/[0.04] px-3 py-2 text-xs text-amber-400">
          <span className="h-2 w-2 rounded-full bg-amber-400 animate-pulse" />
          {pendingConflicts.size} addon{pendingConflicts.size !== 1 ? "s" : ""} need your attention
          — click one to review your edited files
        </div>
      )}

      <div className="flex flex-1 overflow-hidden">
        <AddonList
          addons={filteredAddons}
          allAddons={addons}
          selectedAddon={selectedAddon}
          onSelect={setSelectedAddon}
          searchQuery={searchQuery}
          onSearchChange={setSearchQuery}
          loading={loading}
          updateResults={updateResults}
          sortMode={sortMode}
          onSortChange={handleSortChange}
          filterMode={filterMode}
          onFilterChange={handleFilterChange}
          activeTagFilter={effectiveTagFilter}
          onActiveTagFilterChange={setActiveTagFilter}
          selectedFolders={selectedFolders}
          onToggleSelect={handleToggleSelect}
          viewMode={viewMode}
          onViewModeChange={setViewMode}
          discoverTab={discoverTab}
          onDiscoverTabChange={setDiscoverTab}
          addonsPath={addonsPath}
          onInstalled={handleRefresh}
          onSelectDiscoverResult={setSelectedDiscoverResult}
          selectedDiscoverResultId={selectedDiscoverResult?.id ?? null}
          installedEsouiIds={installedEsouiIds}
          isOffline={isOffline}
          onUpdateAddon={(fn) => void handleSingleUpdate(fn)}
          onRemoveAddon={(fn) => void handleSingleRemove(fn)}
          onToggleDisable={handleToggleDisable}
          onOpenFolder={(fn) => void handleOpenFolder(fn)}
          onToggleFavorite={handleTagsChange}
        />

        {viewMode === "installed" ? (
          <AddonDetail
            key={selectedAddon?.folderName ?? "none"}
            addon={selectedAddon}
            installedAddons={addons}
            addonsPath={addonsPath}
            onRemove={() => {
              setSelectedAddon(null);
              handleRefresh();
            }}
            onRemoveAddon={handleSingleRemove}
            onToggleDisable={handleToggleDisable}
            updateResult={selectedUpdateResult}
            onAddonUpdated={handleAddonUpdated}
            onTagsChange={handleTagsChange}
            isOffline={isOffline}
            pendingConflict={
              selectedAddon ? pendingConflicts.get(selectedAddon.folderName) : undefined
            }
            onConflictResolved={(folderName) => {
              setPendingConflicts((prev) => {
                const next = new Map(prev);
                next.delete(folderName);
                return next;
              });
              handleAddonUpdated(
                updateResults.find((r) => r.folderName === folderName)?.esouiId ?? 0
              );
            }}
          />
        ) : (
          <DiscoverDetail
            key={selectedDiscoverResult?.id ?? "none"}
            result={selectedDiscoverResult}
            addonsPath={addonsPath}
            onInstalled={handleRefresh}
            onRemoveByEsouiId={handleRemoveByEsouiId}
            installedEsouiIds={installedEsouiIds}
            isOffline={isOffline}
          />
        )}
      </div>

      {rosterPackInstallId && addonsPath && (
        <RosterPackInstall
          packId={rosterPackInstallId}
          addonsPath={addonsPath}
          installedAddons={addons}
          onClose={() => setRosterPackInstallId(null)}
          onRefresh={handleRefresh}
        />
      )}

      <AppDialogs
        activeDialog={activeDialog}
        addons={addons}
        addonsPath={addonsPath}
        authUser={authUser}
        deepLinkPackId={deepLinkPackId}
        deepLinkShareCode={deepLinkShareCode}
        knownInstances={knownInstances}
        onAuthChange={setAuthUser}
        onCheckForAppUpdate={() => void checkForAppUpdate(false)}
        onCloseDialog={handleCloseDialog}
        onPathChange={(path) => void handlePathChange(path)}
        onRefresh={handleRefresh}
        onShowDialog={handleOpenDialog}
      />
    </div>
  );
}

export default App;
