import {
  useDeferredValue,
  useEffect,
  useState,
  useCallback,
  useRef,
  useMemo,
  lazy,
  Suspense,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { useAppUpdate } from "./components/app-update";
import { AddonList } from "./components/addon-list";
import { AppBackground } from "./components/app-background";
import { AppDialogs } from "./components/app-dialogs";
import { AppHeader } from "./components/app-header";
import { DiscoverDetail } from "./components/discover-detail";
import { EsoRunningDialog } from "./components/eso-running-dialog";
import { EsoRunningProvider } from "@/lib/eso-running-context";
import { SetupWizard } from "./components/setup-wizard";
import { StatusBanners } from "./components/status-banners";
import { RosterPackInstall } from "./components/roster-pack-install";
import { UpdateBanner, type BannerUpdate } from "./components/update-banner";
import { UploaderIntroCard } from "./components/uploader-intro-card";
import { CfaGuidanceDialog } from "./components/cfa-guidance-dialog";
import { DependencyPickerDialog } from "./components/dependency-picker-dialog";
import { getSetting, setSetting } from "@/lib/store";
import {
  addSkippedDependencies,
  getDependencyPolicy,
  getSkippedDependencies,
  isDependencySkipped,
  setDependencyPolicy,
} from "@/lib/dependency-policy";
import { DependencyPromptProvider, type ResolvePendingDeps } from "@/lib/dependency-prompt-context";
import {
  getTauriErrorMessage,
  invokeOrThrow,
  invokeResult,
  warnIfSessionNotPersisted,
} from "@/lib/tauri";
import { filterAddons, isFilterMode, isSortMode } from "@/lib/addon-helpers";
import { pruneSelection, reconcileSelectedAddon } from "@/lib/addon-selection";
import { BatchUpdateLatch } from "@/lib/batch-update-latch";
import {
  RemovalQueue,
  hideAddon,
  hideUpdateResult,
  removeFolderFromSelection,
  restoreAddon,
  restoreUpdateResult,
  sameAddonsFolder,
  shouldRestoreAfterFailure,
  type PendingRemoval,
  type PendingRemovalGroup,
} from "@/lib/removal-queue";
import { isModKey, isWindows } from "@/lib/platform";
import { nextTextZoomStop, setTextZoom } from "@/lib/text-zoom";
import type {
  AddonManifest,
  AuthUser,
  BatchConflictAddon,
  BatchEnableResult,
  BatchRemoveResult,
  BatchTagResult,
  GameInstance,
  InstallResult,
  PendingDependency,
  StreamingBatchResult,
  UpdateCheckResult,
  WriteAccessStatus,
  EsouiSearchResult,
  SortMode,
  FilterMode,
  ViewMode,
  DiscoverTab,
} from "./types";
import type { LogPathDetection } from "@/types/uploader";

const AddonDetail = lazy(() =>
  import("./components/addon-detail").then((m) => ({ default: m.AddonDetail }))
);

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
  | "shortcuts"
  | "log-upload"
  | null;

interface PendingDeepLinkPayload {
  packId: string | null;
  shareCode: string | null;
  installPackId: string | null;
  appUpdate: boolean;
  logUpload: boolean;
  packHub: boolean;
}

type AddonPhase = "downloading" | "scanning" | "extracting" | "completed" | "failed";

/**
 * Merge duplicate entries so the picker asks about each library once, listing
 * every addon that wants it. Batch flows (Update All, pack installs) can report
 * the same library for several addons, and names are compared case-insensitively
 * because ESO resolves addon folders that way.
 */
function dedupeDependencies(pending: PendingDependency[]): PendingDependency[] {
  const byName = new Map<string, PendingDependency>();
  for (const dep of pending) {
    const key = dep.name.trim().toLowerCase();
    const existing = byName.get(key);
    if (!existing) {
      byName.set(key, { ...dep, requiredBy: [...dep.requiredBy] });
      continue;
    }
    // Required by ANY addon means required — the optional declaration loses.
    if (dep.required) existing.required = true;
    for (const requiredBy of dep.requiredBy) {
      if (!existing.requiredBy.includes(requiredBy)) existing.requiredBy.push(requiredBy);
    }
    // Satisfy the strictest constraint asked for.
    if (dep.minVersion !== null) {
      existing.minVersion =
        existing.minVersion === null
          ? dep.minVersion
          : Math.max(existing.minVersion, dep.minVersion);
    }
  }
  return [...byName.values()].sort(
    (a, b) => Number(b.required) - Number(a.required) || a.name.localeCompare(b.name)
  );
}

/** Ceiling on a queued folder's dependency list. The backend already bounds a
 * single response (MAX_PENDING_DEPENDENCIES in commands.rs); this bounds the
 * repeated arrivals that merge into one batch during a long Update All. */
const MAX_QUEUED_DEPENDENCIES = 200;

function App() {
  const [addonsPath, setAddonsPath] = useState("");
  const [addons, setAddons] = useState<AddonManifest[]>([]);
  const [selectedAddon, setSelectedAddon] = useState<AddonManifest | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [errorShowSettings, setErrorShowSettings] = useState(false);
  const [isOffline, setIsOffline] = useState(!navigator.onLine);
  const [activeDialog, setActiveDialog] = useState<ActiveDialog>(null);
  const [logUploaderMounted, setLogUploaderMounted] = useState(false);
  const [esoRunningPromptOpen, setEsoRunningPromptOpen] = useState(false);
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
  const [addonStatuses, setAddonStatuses] = useState<Map<string, AddonPhase>>(new Map());
  const [pendingConflicts, setPendingConflicts] = useState<Map<string, BatchConflictAddon>>(
    new Map()
  );
  // The app's single dependency picker (see DependencyPromptProvider). Open
  // state is kept apart from the list so the list survives the close animation.
  const [pendingDeps, setPendingDeps] = useState<PendingDependency[]>([]);
  const [depPromptOpen, setDepPromptOpen] = useState(false);
  // Controlled Folder Access / write-access guidance dialog. `exePath` is the
  // Kalpa executable the user must allow through Windows ransomware protection.
  const [cfaDialog, setCfaDialog] = useState<{ exePath: string; permissionDenied: boolean } | null>(
    null
  );
  const [sortMode, setSortMode] = useState<SortMode>("name");
  const [filterMode, setFilterMode] = useState<FilterMode>("all");
  const [authUser, setAuthUser] = useState<AuthUser | null>(null);
  const [authVerifying, setAuthVerifying] = useState(true);
  const [uploaderIntroDismissed, setUploaderIntroDismissed] = useState(true);
  const [uploaderIntroHasLog, setUploaderIntroHasLog] = useState(false);
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
  const checkForAppUpdateRef = useRef(checkForAppUpdate);
  const selectedAddonRef = useRef<AddonManifest | null>(null);
  const addonsPathRef = useRef("");
  const viewModeRef = useRef<ViewMode>("installed");
  const runBatchUpdatesRef = useRef<((updates: UpdateCheckResult[]) => Promise<void>) | null>(null);
  // Resolves the ESO-running confirm dialog: true = update anyway, false = cancel.
  const esoRunningResolveRef = useRef<((proceed: boolean) => void) | null>(null);
  // The single in-flight ESO-running prompt. Concurrent update paths share this one
  // promise instead of each opening a dialog and clobbering the resolver.
  const esoRunningPromptRef = useRef<Promise<boolean> | null>(null);
  // Re-entry guard for Update All: a synchronous preflight latch covering the
  // async preamble plus a `running` latch mirroring `updatingAll`. See
  // `lib/batch-update-latch.ts` for why they are separate.
  const batchLatchRef = useRef(new BatchUpdateLatch());
  // Synchronous mirror of `depPromptOpen`: the resolver must decide whether a
  // picker is already up before its own async store read lands, which state
  // read inside a stable callback can't tell it.
  const depPromptOpenRef = useRef(false);
  // Dependency batches that arrived while a picker was already up, drained when
  // it closes so a concurrent install's libraries are offered rather than
  // dropped. Each batch carries the AddOns folder it was resolved against, so a
  // batch cannot be installed into whichever instance is selected later.
  const queuedDepBatchesRef = useRef<{ addonsPath: string; deps: PendingDependency[] }[]>([]);
  // Latched while a decision is settling, so the picker cannot be submitted
  // twice in the window between the click and the close.
  const depDecisionInFlightRef = useRef(false);
  // The AddOns folder the open prompt's dependencies were resolved against.
  // Recorded to detect staleness, NOT to install elsewhere: the backend allows
  // exactly one registered AddOns path at a time (require_allowed_path in
  // commands.rs) and rejects anything else, so a prompt belonging to a folder
  // the user has since switched away from cannot be fulfilled at all. Such
  // prompts are discarded rather than silently installed into the wrong place.
  const depPromptPathRef = useRef("");
  const scanSeqRef = useRef(0);
  const checkSeqRef = useRef(0);
  const activeDialogRef = useRef<ActiveDialog>(null);
  const setupInstancesRef = useRef<GameInstance[] | null>(null);
  // Synchronous mirror of `addonStatuses` for the batch-progress listener:
  // events arrive faster than renders during Update All, and deriving the
  // progress counts inside the state updater would call a second setter from
  // within it (a Rules-of-React violation). The mirror lets the handler build
  // the next map and both setter calls stay siblings.
  const addonStatusesRef = useRef<Map<string, AddonPhase>>(new Map());

  useEffect(() => {
    checkForAppUpdateRef.current = checkForAppUpdate;
  }, [checkForAppUpdate]);

  useEffect(() => {
    selectedAddonRef.current = selectedAddon;
    addonsPathRef.current = addonsPath;
    viewModeRef.current = viewMode;
    batchLatchRef.current.syncRunning(updatingAll);
    activeDialogRef.current = activeDialog;
    setupInstancesRef.current = setupInstances;
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
        const next = new Map(addonStatusesRef.current);
        next.set(folderName, phase as AddonPhase);
        addonStatusesRef.current = next;
        let completed = 0;
        let failed = 0;
        for (const s of next.values()) {
          if (s === "completed") completed++;
          if (s === "failed") failed++;
        }
        setAddonStatuses(next);
        setUpdateProgress({ completed, failed, total, currentAddon: folderName });
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
        if (payload.appUpdate) {
          void checkForAppUpdateRef.current(false);
        }
        if (payload.logUpload) {
          setActiveDialog("log-upload");
        }
        if (payload.packHub) {
          setActiveDialog("packs");
        }
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

    // One-shot: the startup gate reverted out of native mode because the
    // native shell never finished booting. Reading clears the note.
    void invokeOrThrow<boolean>("native_boot_failure_pending")
      .then((failed) => {
        if (disposed || !failed) return;
        toast.error("The native performance UI couldn't start.", {
          duration: 10000,
          description:
            "Kalpa switched back to the standard UI. You can try native mode again from Settings.",
        });
      })
      .catch((invokeError) => {
        console.error("[tauri:native_boot_failure_pending]", invokeError);
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
        setSelectedAddon(reconcileSelectedAddon(selectedAddonRef.current, result));
      }
      // Prune, never reset: a dependency install rescans too, and clearing here
      // threw away a multi-select whose addons were all still installed.
      setSelectedFolders((prev) => pruneSelection(prev, result));
    } catch (scanError) {
      if (seq !== scanSeqRef.current) return;
      setError(getTauriErrorMessage(scanError));
      setAddons([]);
      setSelectedFolders(new Set());
    } finally {
      if (seq === scanSeqRef.current) {
        setLoading(false);
      }
    }
  }, []);

  const checkForUpdates = useCallback(
    async (path: string, autoUpdate = false, notifyOnError = false) => {
      const seq = ++checkSeqRef.current;
      // Deliberately does NOT clear updatingAll/updateProgress: those belong to
      // the batch path that set them, and a refresh landing mid-Update-All would
      // otherwise hide the progress UI and unlatch the batch re-entry guard.
      setCheckingUpdates(true);
      try {
        const results = await invokeOrThrow<UpdateCheckResult[]>("check_for_updates", {
          addonsPath: path,
        });
        if (seq !== checkSeqRef.current) return;

        setUpdateResults(results);

        // The check just wrote fresh esoui_last_update values to metadata, but the
        // live addon state still holds whatever was on disk at scan time (0 for a
        // just-installed addon). Merge the freshly observed timestamps in so the
        // "Recently Updated" sort is correct immediately, without a second scan.
        const freshUpdateTimes = new Map(
          results
            .filter((r) => r.remoteLastUpdate > 0)
            .map((r) => [r.folderName, r.remoteLastUpdate] as const)
        );
        if (freshUpdateTimes.size > 0) {
          setAddons((prev) => {
            let changed = false;
            const next = prev.map((addon) => {
              const ts = freshUpdateTimes.get(addon.folderName);
              if (ts !== undefined && ts !== addon.esouiLastUpdate) {
                changed = true;
                return { ...addon, esouiLastUpdate: ts };
              }
              return addon;
            });
            return changed ? next : prev;
          });
        }

        const updates = results.filter((result) => result.hasUpdate);

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

  const refreshUploaderIntroDetection = useCallback(async () => {
    const dismissed = await getSetting<boolean>("uploaderIntroDismissed", false);
    setUploaderIntroDismissed(dismissed === true);
    if (dismissed) {
      setUploaderIntroHasLog(false);
      return;
    }

    const detection = await invokeResult<LogPathDetection>("uploader_detect_path");
    setUploaderIntroHasLog(detection.ok ? detection.data.encounterLogExists === true : false);
  }, []);

  const initializeApp = useCallback(async () => {
    // Restore cached account identity first so an already-signed-in user never
    // sees a signed-out flash while the network verification runs.
    void invokeResult<AuthUser | null>("auth_cached_user")
      .then((cachedResult) => {
        if (cachedResult.ok) {
          setAuthUser(cachedResult.data ?? null);
        } else {
          console.error(`[tauri:auth_cached_user] ${cachedResult.error}`);
        }
        return invokeResult<AuthUser | null>("auth_get_user");
      })
      .then((authResult) => {
        if (authResult.ok) {
          setAuthUser(authResult.data ?? null);
          warnIfSessionNotPersisted(authResult.data);
        } else {
          setAuthUser(null);
          toast.error(`Could not restore sign-in: ${authResult.error}`);
        }
      })
      .finally(() => setAuthVerifying(false));

    // These settings reads are independent — fetch them in one batch instead
    // of four sequential awaits.
    const [savedSort, savedFilter, savedPath, autoUpdate, introDismissed] = await Promise.all([
      getSetting<string>("sortMode", "name"),
      getSetting<string>("filterMode", "all"),
      getSetting<string>("addonsPath", ""),
      getSetting<boolean>("autoUpdate", false),
      getSetting<boolean>("uploaderIntroDismissed", false),
    ]);

    const normalizedSort = isSortMode(savedSort) ? savedSort : "name";
    const normalizedFilter = isFilterMode(savedFilter) ? savedFilter : "all";
    setSortMode(normalizedSort);
    setFilterMode(normalizedFilter);
    setUploaderIntroDismissed(introDismissed === true);
    if (normalizedSort !== savedSort) {
      void setSetting("sortMode", normalizedSort);
    }
    if (normalizedFilter !== savedFilter) {
      void setSetting("filterMode", normalizedFilter);
    }

    if (savedPath) {
      // Saved path exists — use it directly
      try {
        setAddonsPath(savedPath);
        await invokeOrThrow("set_addons_path", { addonsPath: savedPath });
        void refreshUploaderIntroDetection();
        // Scan (disk) and update check (metadata + network) touch different
        // state and locks, so run them concurrently instead of in series.
        await Promise.all([scanAddons(savedPath), checkForUpdates(savedPath, autoUpdate, false)]);
        void runAutoLink(savedPath);
        // Populate knownInstances so the Settings instance switcher works for
        // returning users. Fire-and-forget — does not block startup.
        invokeOrThrow<GameInstance[]>("detect_game_instances")
          .then(setKnownInstances)
          .catch(console.error);
      } catch (initError) {
        setError(
          `Could not access saved AddOns folder — it may have been moved or deleted. ${getTauriErrorMessage(initError)}`
        );
        setErrorShowSettings(true);
        setLoading(false);
      }
    } else {
      // No saved path — run detection and show wizard or auto-select
      let instances: GameInstance[];
      try {
        instances = await invokeOrThrow<GameInstance[]>("detect_game_instances");
      } catch (detectError) {
        const message = `Could not detect game folders: ${getTauriErrorMessage(detectError)}`;
        setError(message);
        // The wizard tree does not mount StatusBanners, so `error` alone is
        // invisible during first-run onboarding — toast it as well.
        toast.error(message);
        setSetupInstances([]);
        setLoading(false);
        return;
      }
      setKnownInstances(instances);

      const singleClean = instances.length === 1 && !instances[0]!.isOnedrive;

      if (singleClean) {
        // One unambiguous instance with no OneDrive complication — auto-select
        try {
          const path = instances[0]!.addonsPath;
          setAddonsPath(path);
          await invokeOrThrow("set_addons_path", { addonsPath: path });
          void refreshUploaderIntroDetection();
          // Best-effort persist: this is auto-detection, so if the write fails it
          // self-heals — the same instance is re-detected and re-selected next launch.
          void setSetting("addonsPath", path);
          // Scan and update check are independent — run concurrently.
          await Promise.all([scanAddons(path), checkForUpdates(path, autoUpdate, false)]);
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
  }, [checkForUpdates, refreshUploaderIntroDetection, runAutoLink, scanAddons]);

  useEffect(() => {
    if (initRan.current) return;
    initRan.current = true;
    void initializeApp();
  }, [initializeApp]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.isComposing) return;

      // `event.key` reflects Caps Lock and Shift, so a latched Caps Lock would
      // deliver "R" and silently disable every letter shortcut.
      const key = event.key.toLowerCase();

      if (event.key === "?" && !event.ctrlKey && !event.metaKey && !event.altKey) {
        const active = document.activeElement;
        if (
          active instanceof HTMLInputElement ||
          active instanceof HTMLTextAreaElement ||
          (active instanceof HTMLElement && active.isContentEditable)
        ) {
          return;
        }
        if (activeDialogRef.current !== null || setupInstancesRef.current !== null) return;
        event.preventDefault();
        setActiveDialog("shortcuts");
      }

      if (isModKey(event) && key === "r") {
        event.preventDefault();
        if (addonsPathRef.current && !batchLatchRef.current.isRunning) {
          void scanAndCheck(addonsPathRef.current, true);
        }
      }

      if (isModKey(event) && key === "i") {
        event.preventDefault();
        setViewMode("discover");
        setDiscoverTab("url");
      }

      if (isModKey(event) && key === "b") {
        event.preventDefault();
        setViewMode("discover");
        setDiscoverTab("search");
      }

      if (isModKey(event) && (event.key === "+" || event.key === "=")) {
        event.preventDefault();
        setTextZoom(nextTextZoomStop(1));
      }

      if (isModKey(event) && event.key === "-") {
        event.preventDefault();
        setTextZoom(nextTextZoomStop(-1));
      }

      if (isModKey(event) && event.key === "0") {
        event.preventDefault();
        setTextZoom(1);
      }

      if (event.key === "Escape") {
        const active = document.activeElement;
        if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) {
          active.blur();
          return;
        }
        if (viewModeRef.current === "discover") {
          setViewMode("installed");
        } else {
          setSelectedFolders(new Set());
        }
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

  const effectiveTagFilter = useMemo(
    () =>
      activeTagFilter && addons.some((a) => a.tags.includes(activeTagFilter))
        ? activeTagFilter
        : null,
    [activeTagFilter, addons]
  );

  const handleSetupSelect = useCallback(
    async (selectedPath: string) => {
      const path = selectedPath.trim();
      if (!path) return;

      try {
        await invokeOrThrow("set_addons_path", { addonsPath: path });
        // Critical setting: don't commit the UI to a path that didn't persist.
        if (!(await setSetting("addonsPath", path))) {
          const message =
            "Could not save the AddOns folder location — free up disk space or check antivirus, then try again.";
          setError(message);
          setErrorShowSettings(true);
          // StatusBanners is not mounted while the wizard is up, so a toast is
          // the only feedback the user gets here.
          toast.error(message);
          return;
        }
        setAddonsPath(path);
        void refreshUploaderIntroDetection();
        setSetupInstances(null);
        setErrorShowSettings(false);
        setLoading(true);
        await scanAddons(path);
        const autoUpdate = await getSetting<boolean>("autoUpdate", false);
        await checkForUpdates(path, autoUpdate, false);
        void runAutoLink(path);
      } catch (pathError) {
        const message = `Could not set addons folder: ${getTauriErrorMessage(pathError)}`;
        setError(message);
        setErrorShowSettings(true);
        toast.error(message);
      }
    },
    [checkForUpdates, refreshUploaderIntroDetection, runAutoLink, scanAddons]
  );

  const handleRefresh = useCallback(() => {
    // Same guard the Ctrl+R path already has: a rescan mid-Update-All reads the
    // folder while the batch is still extracting into it, and replaces `addons`
    // wholesale — resurrecting rows the batch or an undo window had hidden.
    if (addonsPathRef.current && !batchLatchRef.current.isRunning) {
      void scanAndCheck(addonsPathRef.current, true);
    }
  }, [scanAndCheck]);

  // True when any of the given folders is declared as a dependency by another
  // installed addon. missing/outdatedDependencies are computed by the backend
  // against the *enabled* folder set, so the in-place disabled-flag patch can't
  // recompute them — when a dependency's enabled state changes we must rescan to
  // keep dependents' "N missing" badges correct. Returns false in the common
  // case (toggling a non-dependency addon), preserving the no-rescan perf win.
  const togglingAffectsDependencies = useCallback(
    (toggledFolders: Set<string>) =>
      addons.some((addon) => addon.dependsOn.some((dep) => toggledFolders.has(dep.name))),
    [addons]
  );

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
        // Patch the toggled addon's `disabled` flag in place rather than
        // triggering a full disk rescan + network update check (which blanks
        // the list and loses scroll position). Mirrors handleTagsChange.
        const nowDisabled = !currentlyDisabled;
        setAddons((prev) =>
          prev.map((addon) =>
            addon.folderName === folderName ? { ...addon, disabled: nowDisabled } : addon
          )
        );
        setSelectedAddon((prev) =>
          prev?.folderName === folderName ? { ...prev, disabled: nowDisabled } : prev
        );
        // If this addon is a dependency of another, its enabled-state change
        // affects dependents' missing/outdated badges, which only the backend
        // can recompute — rescan to keep them accurate.
        if (togglingAffectsDependencies(new Set([folderName]))) {
          void scanAddons(addonsPathRef.current);
        }
      } else {
        toast.error(result.error);
      }
    },
    [addonsPath, togglingAffectsDependencies, scanAddons]
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
        const { join } = await import("@tauri-apps/api/path");
        await revealItemInDir(await join(addonsPath, folderName));
      } catch (e) {
        toast.error(`Could not open folder: ${getTauriErrorMessage(e)}`);
      }
    },
    [addonsPath]
  );

  // Shared gate for every addon-update write path. Returns true to proceed.
  // Updating while ESO runs is safe on disk — the game just won't see the changes
  // until /reloadui or relog — so we warn (unless suppressed) instead of blocking.
  // Concurrent callers share one prompt so a second can't strand the first's resolver.
  const ensureEsoNotBlocking = useCallback(async (): Promise<boolean> => {
    let esoRunning: boolean;
    try {
      esoRunning = await invokeOrThrow<boolean>("is_eso_running");
    } catch {
      return true; // Non-critical — proceed if we can't check.
    }
    if (!esoRunning) return true;
    if (await getSetting<boolean>("suppressEsoRunningWarning", false)) return true;

    // A prompt is already open — join its decision instead of opening another.
    if (esoRunningPromptRef.current) return esoRunningPromptRef.current;

    const prompt = new Promise<boolean>((resolve) => {
      esoRunningResolveRef.current = resolve;
      setEsoRunningPromptOpen(true);
    }).finally(() => {
      esoRunningPromptRef.current = null;
    });
    esoRunningPromptRef.current = prompt;
    return prompt;
  }, []);

  // Present (or park) one batch of held-back dependencies. The AddOns folder
  // travels WITH the batch rather than being read from the live path when the
  // prompt opens: a batch resolved against one game instance can sit in the
  // queue while the user switches to another, and these libraries belong to the
  // addon that asked for them. Tracking the path separately re-pinned a queued
  // instance-A batch to instance B on drain and installed it into the wrong
  // folder.
  const presentDependencyBatch = useCallback(
    async (pending: PendingDependency[], sourceAddonsPath: string) => {
      if (pending.length === 0) return;

      // Drop anything the user has already declined, so Update All doesn't
      // re-ask for the same library on every patch day.
      //
      // Only optional entries are filtered. The skip list is keyed by name
      // alone, so a library turned down while it was OPTIONAL would otherwise
      // stay hidden when a different addon later declares it REQUIRED —
      // silently leaving that addon unable to load, with no prompt and no
      // warning. A required dependency is load-bearing enough to be worth
      // re-offering; declining it again is one click.
      // Refuse outright for a folder that is no longer the active one. A command
      // started before an instance switch can land after it, and the backend
      // registers exactly one AddOns path, so such a prompt could never be
      // fulfilled — its Install button would be refused. Say so once here rather
      // than opening a dialog whose primary action cannot work.
      if (!sameAddonsFolder(sourceAddonsPath, addonsPathRef.current)) {
        toast.info(
          `${pending.length} ${pending.length === 1 ? "library was" : "libraries were"} needed by the previous AddOns folder. Switch back and reinstall to add ${pending.length === 1 ? "it" : "them"}.`
        );
        return;
      }

      const skipped = await getSkippedDependencies();
      const candidates = dedupeDependencies(pending).filter(
        (dep) => dep.required || !isDependencySkipped(dep.name, skipped)
      );
      if (candidates.length === 0) return;

      // One picker at a time — a second would replace the first's list
      // mid-decision. Park the extras instead of dropping them, so a Discover
      // install landing mid-Update-All still gets its dependencies offered.
      // Queue while a decision is settling too: the picker closes the instant
      // it is submitted but the install runs on for seconds, and anything
      // arriving in that window would otherwise escape the filtering that
      // drops what the in-flight decision installs.
      if (depPromptOpenRef.current || depDecisionInFlightRef.current) {
        const batches = queuedDepBatchesRef.current;
        // Merge only within the same folder. Two instances can legitimately
        // need the same library name, and they are separate installs.
        const sameFolder = batches.find((batch) =>
          sameAddonsFolder(batch.addonsPath, sourceAddonsPath)
        );
        // Bounded: the backend caps a single response, but repeated arrivals
        // merge here, so without this the queue could grow without limit across
        // a long Update All. Say so when it bites — dropping libraries quietly
        // is the failure mode this whole feature exists to avoid, and dedupe
        // keeps required entries first so the cap sheds optional ones first.
        const merged = sameFolder
          ? dedupeDependencies([...sameFolder.deps, ...candidates])
          : candidates;
        if (merged.length > MAX_QUEUED_DEPENDENCIES) {
          toast.warning(
            `Too many pending libraries to offer at once; showing the first ${MAX_QUEUED_DEPENDENCIES}. Re-run the update afterwards to see the rest.`
          );
        }
        const capped = merged.slice(0, MAX_QUEUED_DEPENDENCIES);
        if (sameFolder) {
          sameFolder.deps = capped;
        } else {
          batches.push({ addonsPath: sourceAddonsPath, deps: capped });
        }
        return;
      }

      depPromptOpenRef.current = true;
      depPromptPathRef.current = sourceAddonsPath;
      setPendingDeps(candidates);
      setDepPromptOpen(true);
    },
    []
  );

  // Shared entry point for every install/update that came back with dependencies
  // the backend held for the user ("ask" policy). See dependency-prompt-context.
  // Called straight after that install resolved, so the live path is still the
  // one those dependencies were resolved against — capture it here, once.
  const resolvePendingDeps = useCallback<ResolvePendingDeps>(
    async (pending, addonsPath) => {
      await presentDependencyBatch(pending, addonsPath);
    },
    [presentDependencyBatch]
  );

  const handleSingleUpdate = useCallback(
    async (folderName: string) => {
      const ur = updateResults.find((r) => r.folderName === folderName && r.hasUpdate);
      if (!ur) return;
      if (!(await ensureEsoNotBlocking())) return;
      try {
        const result = await invokeOrThrow<InstallResult>("update_addon", {
          addonsPath,
          esouiId: ur.esouiId,
          // Record the version the update CHECK compared against (the filelist
          // string), not the one the filedetails endpoint reports. When the two
          // sources disagree the addon otherwise stays permanently "outdated".
          apiVersion: ur.remoteVersion,
          dependencyPolicy: await getDependencyPolicy(),
        });
        toast.success(`Updated ${folderName}`);
        srAnnounce(`Updated ${folderName}`);
        handleAddonUpdated(ur.esouiId);
        // Empty unless the policy is "ask"; the picker owns the rest. Pass the
        // same folder this update wrote to, not the live one — the user may
        // have switched instances while it ran.
        void resolvePendingDeps(result.pendingDeps, addonsPath);
      } catch (e) {
        toast.error(`Update failed: ${getTauriErrorMessage(e)}`);
      }
    },
    [
      addonsPath,
      updateResults,
      srAnnounce,
      handleAddonUpdated,
      ensureEsoNotBlocking,
      resolvePendingDeps,
    ]
  );

  // The optimistic-removal queue. Its group-timer and per-entry AddOns-folder
  // rules live in `lib/removal-queue.ts` so they can be tested without the app.
  const pendingRemovalsRef = useRef(new RemovalQueue());

  // Put an optimistically-hidden addon back, update row included, so a restored
  // addon keeps its Update badge, banner count and "Outdated" filter membership.
  const restorePendingRemoval = useCallback((entry: PendingRemoval) => {
    setAddons((prev) => restoreAddon(prev, entry.addon));
    setUpdateResults((prev) => restoreUpdateResult(prev, entry.updateResult));
  }, []);

  // Commit everything still inside its undo window. Each entry names the folder
  // it was queued against, so a removal queued before an instance switch runs
  // against the old folder (where the backend rejects it) instead of deleting
  // the same-named addon in the instance the user moved to.
  const flushPendingRemovals = useCallback(async () => {
    const entries = pendingRemovalsRef.current.drain();
    await Promise.all(
      entries.map(async (entry) => {
        const result = await invokeResult("remove_addon", {
          addonsPath: entry.addonsPath,
          folderName: entry.addon.folderName,
        });
        if (result.ok) return;
        toast.error(`Remove failed: ${result.error}`);
        // Only restore into the view the addon belongs to — after an instance
        // switch the row would reappear in a folder that never had it.
        if (shouldRestoreAfterFailure(entry, addonsPathRef.current)) {
          restorePendingRemoval(entry);
        }
      })
    );
  }, [restorePendingRemoval]);

  useEffect(() => {
    const handler = () => void flushPendingRemovals();
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [flushPendingRemovals]);

  const handleSingleRemove = useCallback(
    (folderName: string) => {
      const addon = addons.find((a) => a.folderName === folderName);
      if (!addon) return;
      const updateResult = updateResults.find((r) => r.folderName === folderName) ?? null;
      const queuedPath = addonsPath;

      // Optimistically hide from UI
      setAddons((prev) => hideAddon(prev, folderName));
      setUpdateResults((prev) => hideUpdateResult(prev, folderName));
      setSelectedFolders((prev) => removeFolderFromSelection(prev, folderName));
      if (selectedAddonRef.current?.folderName === folderName) {
        setSelectedAddon(null);
      }

      // Cancel any existing pending removal for this addon. Dropping the entry
      // rather than clearing its timer keeps a shared batch timer alive for the
      // folders still queued behind it.
      pendingRemovalsRef.current.drop(folderName);

      const groupFolders = new Set([folderName]);
      const timer = setTimeout(() => {
        const entry = pendingRemovalsRef.current.drop(folderName);
        void invokeOrThrow("remove_addon", { addonsPath: queuedPath, folderName }).catch((e) => {
          toast.error(`Remove failed: ${getTauriErrorMessage(e)}`);
          if (entry && sameAddonsFolder(queuedPath, addonsPathRef.current)) {
            restorePendingRemoval(entry);
          }
        });
      }, 3000);

      pendingRemovalsRef.current.add(folderName, {
        addon,
        updateResult,
        addonsPath: queuedPath,
        group: { timer, folderNames: groupFolders },
      });

      toast(`Removed ${addon.title}`, {
        action: {
          label: "Undo",
          onClick: () => {
            const entry = pendingRemovalsRef.current.drop(folderName);
            if (entry) {
              restorePendingRemoval(entry);
              toast.success(`Restored ${addon.title}`);
            }
          },
        },
        duration: 3000,
      });
      srAnnounce(`Removed ${addon.title}. Press undo to restore.`);
    },
    [addons, addonsPath, updateResults, srAnnounce, restorePendingRemoval]
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

      const switchingFolder = !sameAddonsFolder(nextPath, addonsPathRef.current);

      // Settle queued removals while the backend still authorizes the folder
      // they were queued against. Left running, their timers would fire against
      // a folder the backend no longer accepts — the removal never happens and
      // the failure handler re-inserts the old folder's addon into the new
      // instance's freshly scanned list.
      if (switchingFolder) await flushPendingRemovals();

      try {
        await invokeOrThrow("set_addons_path", { addonsPath: nextPath });
        // The AddOns path is the one setting the app can't function without, so
        // treat a persistence failure as hard: don't switch the UI to a path that
        // won't survive a restart. (Cosmetic prefs below tolerate silent failure.)
        if (!(await setSetting("addonsPath", nextPath))) {
          // The backend has already authorized nextPath, but the UI is staying
          // on the old folder — put the backend back where the UI is, or they
          // disagree about which folder is live and any command issued from the
          // old view is rejected as "does not match the configured path".
          if (addonsPathRef.current) {
            void invokeResult("set_addons_path", { addonsPath: addonsPathRef.current });
          }
          setError(
            "Could not save the AddOns folder location — free up disk space or check antivirus, then try again."
          );
          setErrorShowSettings(true);
          return;
        }
        // Only now that the switch has actually committed. Dependency prompts
        // belong to the folder they were resolved for, and the backend registers
        // exactly one AddOns path at a time, so nothing queued for the old
        // folder can be acted on. Discarding earlier would have thrown away the
        // current folder's prompts whenever set_addons_path or the settings
        // write failed and left the app where it was.
        if (switchingFolder) {
          const abandoned = queuedDepBatchesRef.current.length > 0 || depPromptOpenRef.current;
          queuedDepBatchesRef.current = [];
          // Inline rather than closeDependencyPrompt, which is declared further
          // down this component and would be in its temporal dead zone here.
          setDepPromptOpen(false);
          depPromptOpenRef.current = false;
          if (abandoned) {
            toast.info(
              "Pending dependency prompts were for the previous AddOns folder and have been dismissed."
            );
          }
        }

        setAddonsPath(nextPath);
        // Sync the ref here rather than waiting for the effect that mirrors it.
        // A dependency result from the old folder can land in between, and the
        // open-time folder guard reads this ref — against a stale value it would
        // wave through a batch for the folder we just left.
        addonsPathRef.current = nextPath;
        void refreshUploaderIntroDetection();
        setSelectedAddon(null);
        setSelectedFolders(new Set());
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
    [flushPendingRemovals, refreshUploaderIntroDetection, scanAndCheck]
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

  // The single place the tray tooltip is written. Every path that changes the
  // update set — full check, batch, single update, removal, undo, conflict
  // resolution — lands in `updateResults`, so deriving the count here is the
  // only way the tray and the banner cannot disagree.
  useEffect(() => {
    void invokeResult("update_tray_tooltip", { updateCount: updatesAvailable.length });
  }, [updatesAvailable.length]);

  const installedEsouiIds = useMemo(() => {
    const ids = new Set<number>();
    for (const addon of addons) {
      if (addon.esouiId != null) ids.add(addon.esouiId);
    }
    return ids;
  }, [addons]);

  // Enrich the available updates with display titles and sort them, so the
  // banner's "Choose" checklist reads like the addon list rather than raw
  // folder names.
  const bannerUpdates = useMemo<BannerUpdate[]>(() => {
    const titleByFolder = new Map(addons.map((a) => [a.folderName, a.title] as const));
    return updatesAvailable
      .map((u) => ({
        folderName: u.folderName,
        title: titleByFolder.get(u.folderName) ?? u.folderName,
        currentVersion: u.currentVersion,
        remoteVersion: u.remoteVersion,
      }))
      .sort((a, b) => a.title.localeCompare(b.title));
  }, [updatesAvailable, addons]);

  const runBatchUpdates = useCallback(
    async (updates: UpdateCheckResult[]) => {
      const path = addonsPathRef.current;
      if (!path || updates.length === 0) return;

      // Claim the preflight slot synchronously (before any await) so two rapid calls
      // can't both pass the game check / confirm dialog and start overlapping batches
      // against the same AddOns folder. Refused outright while a batch is already
      // running. Released on every preamble exit and once the `updatingAll` state
      // guard takes over below.
      const latch = batchLatchRef.current;
      if (!latch.tryEnterPreflight()) return;

      if (!(await ensureEsoNotBlocking())) {
        latch.abortPreflight();
        return;
      }

      // Proactively check that Kalpa can write to the AddOns folder. When
      // Windows Controlled Folder Access (or read-only/AV) blocks writes, every
      // update would fail; surface the guidance up front instead of after 14
      // failures. Fails open — a detection hiccup never blocks the update.
      const access = await invokeResult<WriteAccessStatus>("check_addons_write_access", {
        addonsPath: path,
      });
      if (access.ok && access.data.blocked) {
        latch.abortPreflight();
        if (isWindows()) {
          setCfaDialog({
            exePath: access.data.exePath,
            permissionDenied: access.data.permissionDenied,
          });
        } else {
          // The CFA dialog is Windows Security-specific; elsewhere a write
          // block is a plain permissions problem.
          toast.error("Kalpa can't write to your AddOns folder.", {
            description: "Check that the folder is writable by your user account.",
          });
        }
        return;
      }

      // Hand off from the preflight latch to the in-progress latch synchronously, so the
      // `isRunning` guard above covers the gap until the `updatingAll` state lands.
      latch.promoteToRunning();
      setUpdatingAll(true);
      setUpdateProgress({ completed: 0, failed: 0, total: updates.length });
      addonStatusesRef.current = new Map();
      setAddonStatuses(new Map());

      void invokeResult("create_pre_operation_snapshot", {
        addonsPath: path,
        operationLabel: "update-all",
      });

      // Resolve the conflict policy up front so the backend can auto-resolve
      // conflicts inline (keep_mine / take_update) and only defer the "ask"
      // case back to us for the interactive modal.
      const policy = await getSetting<"ask" | "keep_mine" | "take_update">("conflictPolicy", "ask");
      // Likewise for dependencies — read once for the whole batch, so a run that
      // pulls libraries prompts once at the end rather than once per addon.
      const dependencyPolicy = await getDependencyPolicy();

      // Single streaming call: parallel downloads, extract-as-each-finishes,
      // one kalpa.json load/save and one dependency-resolution pass for the
      // whole batch. Replaces the old scan-all → per-addon-decision loop, which
      // re-locked and re-saved metadata once per addon (the source of the
      // last-addon lag).
      const batch = await invokeResult<StreamingBatchResult>("update_batch_with_decisions", {
        addonsPath: path,
        conflictPolicy: policy,
        dependencyPolicy,
        updates: updates.map((u) => ({
          esouiId: u.esouiId,
          folderName: u.folderName,
          apiVersion: u.remoteVersion,
        })),
      });

      if (!batch.ok) {
        setUpdatingAll(false);
        setUpdateProgress(null);
        toast.error(`Batch update failed: ${batch.error}`);
        srAnnounce("Batch update failed");
        return;
      }

      const { completed, failed, errors: batchErrors, conflicts: remainingConflicts } = batch.data;

      // Final progress reflects the streamed batch-update-progress events; the
      // count here is just for the summary toast below.

      // Collect per-addon failure reasons. Backend errors come back as raw
      // strings (the command resolved ok), so they bypass invokeResult's
      // mapper — normalize them here so extraction/download failures get the
      // same friendly/permission/CFA guidance the UI already applies.
      const failureReasons = new Map<string, string>();
      for (const name of failed) {
        const raw = batchErrors?.[name];
        failureReasons.set(name, raw ? getTauriErrorMessage(raw) : "unknown error");
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
          // Full per-addon reasons to the console for diagnosis.
          const reasonLines = failed.map(
            (name) => `${name}: ${failureReasons.get(name) ?? "unknown error"}`
          );
          console.error(`Update failures (${failed.length}):\n${reasonLines.join("\n")}`);

          // Build the user-visible detail by grouping addons under a stable
          // label for their cause, then listing the affected addon names.
          // Controlled Folder Access messages embed the per-file path, so they
          // must be normalized to one canonical label — otherwise a batch-wide
          // CFA block would repeat the full multi-sentence instructions once
          // per addon. Names per group are bounded with an overflow count.
          // Collapse the permission-denied family to one hedged label. The
          // backend message embeds a per-file path, so without this every
          // addon would fragment into its own group. The label mirrors the
          // backend's hedge (CFA is the likely — not certain — cause) so we
          // don't send read-only/permission/antivirus failures down a
          // CFA-only path. Keeps the "controlled folder access" substring so
          // the tauri.ts passthrough still recognizes it.
          const canonicalReason = (reason: string): string =>
            /controlled folder access/i.test(reason)
              ? "Windows blocked Kalpa from writing — most often Controlled Folder Access " +
                "(ransomware protection), but possibly read-only files or antivirus. Fix the " +
                "common case in Windows Security → Virus & threat protection → Ransomware " +
                "protection → Allow an app through Controlled folder access."
              : reason;

          const byReason = new Map<string, string[]>();
          for (const name of failed) {
            const reason = canonicalReason(failureReasons.get(name) ?? "unknown error");
            const names = byReason.get(reason) ?? [];
            names.push(name);
            byReason.set(reason, names);
          }
          const MAX_NAMES = 5;
          let detail = [...byReason.entries()]
            .map(([reason, names]) => {
              const shown = names.slice(0, MAX_NAMES).join(", ");
              const overflow = names.length - Math.min(names.length, MAX_NAMES);
              const nameList = overflow > 0 ? `${shown} +${overflow} more` : shown;
              return `${reason}\n${nameList}`;
            })
            .join("\n\n");
          // Hard cap so a pathological batch can't produce an enormous toast;
          // the full per-addon detail is always in the console above.
          const MAX_DETAIL = 600;
          if (detail.length > MAX_DETAIL) {
            detail = detail.slice(0, MAX_DETAIL - 1).trimEnd() + "…";
          }
          toast.warning(msg, { description: detail });

          // If any failure was a write/permission block (CFA et al.), surface
          // the rich guidance dialog as a fallback — the proactive probe may
          // have passed but extraction still hit a protected file. Fetch the
          // exe path so the dialog can name the app to allow.
          const hasCfaFailure = [...failureReasons.values()].some((r) =>
            /controlled folder access/i.test(r)
          );
          if (hasCfaFailure && isWindows()) {
            const access = await invokeResult<WriteAccessStatus>("check_addons_write_access", {
              addonsPath: path,
            });
            // Only open if not already showing (proactive probe may have set it).
            setCfaDialog(
              (prev) =>
                prev ?? { exePath: access.ok ? access.data.exePath : "", permissionDenied: true }
            );
          }
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
        setUpdateResults((prev) => prev.filter((result) => !succeededNames.has(result.folderName)));

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

      // One prompt for the whole run: the backend returns a single list for the
      // batch, raised after the re-scan so the picker isn't competing with it.
      // `path` is the folder this whole batch ran against.
      void resolvePendingDeps(batch.data.pendingDeps, path);
    },
    [ensureEsoNotBlocking, resolvePendingDeps, scanAddons, srAnnounce]
  );

  useEffect(() => {
    runBatchUpdatesRef.current = runBatchUpdates;
  }, [runBatchUpdates]);

  const handleUpdateAll = useCallback(() => {
    void runBatchUpdates(updatesAvailable);
  }, [runBatchUpdates, updatesAvailable]);

  // Update only the addons chosen in the banner's "Choose" checklist. Reuses the
  // streaming batch path so a partial run gets the same pills, progress bar, and
  // conflict handling as Update All.
  const handleUpdateSelected = useCallback(
    (folderNames: string[]) => {
      const names = new Set(folderNames);
      const toUpdate = updatesAvailable.filter((update) => names.has(update.folderName));
      if (toUpdate.length === 0) return;
      void runBatchUpdates(toUpdate);
    },
    [runBatchUpdates, updatesAvailable]
  );

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
    const folderNames = removedAddons.map((a) => a.folderName);
    const count = removedAddons.length;
    const queuedPath = addonsPath;

    const removedSet = new Set(folderNames);
    const removedUpdates = new Map(
      updateResults
        .filter((r) => removedSet.has(r.folderName))
        .map((r) => [r.folderName, r] as const)
    );

    // Optimistically hide from all relevant state
    setAddons((prev) => prev.filter((a) => !removedSet.has(a.folderName)));
    setUpdateResults((prev) => prev.filter((r) => !removedSet.has(r.folderName)));
    if (selectedAddonRef.current && selectedFolders.has(selectedAddonRef.current.folderName)) {
      setSelectedAddon(null);
    }
    setSelectedFolders(new Set());

    for (const folderName of folderNames) pendingRemovalsRef.current.drop(folderName);

    const groupFolders = new Set(folderNames);
    const timer = setTimeout(() => {
      // Only the members still queued: a folder re-removed or undone in the
      // meantime has already left the group and must not be removed twice.
      const entries = [...groupFolders]
        .map((folderName) => pendingRemovalsRef.current.drop(folderName))
        .filter((entry): entry is PendingRemoval => entry !== null);
      if (entries.length === 0) return;
      void invokeOrThrow<BatchRemoveResult>("batch_remove_addons", {
        addonsPath: queuedPath,
        folderNames: entries.map((entry) => entry.addon.folderName),
      })
        .then((result) => {
          if (result.failed.length > 0) {
            // Restore only the addons that failed to remove
            const failedSet = new Set(result.failed);
            const details = result.failed
              .map((name) => `${name}: ${result.errors[name] ?? "unknown error"}`)
              .join("; ");
            toast.error(`Failed to remove ${result.failed.length} addon(s): ${details}`);
            if (!sameAddonsFolder(queuedPath, addonsPathRef.current)) return;
            for (const entry of entries) {
              if (failedSet.has(entry.addon.folderName)) restorePendingRemoval(entry);
            }
          }
        })
        .catch((e) => {
          toast.error(`Batch remove failed: ${getTauriErrorMessage(e)}`);
          if (!sameAddonsFolder(queuedPath, addonsPathRef.current)) return;
          for (const entry of entries) restorePendingRemoval(entry);
        });
    }, 3000);

    const group: PendingRemovalGroup = { timer, folderNames: groupFolders };
    for (const addon of removedAddons) {
      pendingRemovalsRef.current.add(addon.folderName, {
        addon,
        updateResult: removedUpdates.get(addon.folderName) ?? null,
        addonsPath: queuedPath,
        group,
      });
    }

    toast(`Removed ${count} addon${count !== 1 ? "s" : ""}`, {
      action: {
        label: "Undo",
        onClick: () => {
          const restored = folderNames
            .map((folderName) => pendingRemovalsRef.current.drop(folderName))
            .filter((entry): entry is PendingRemoval => entry !== null);
          if (restored.length === 0) return;
          for (const entry of restored) restorePendingRemoval(entry);
          toast.success(`Restored ${restored.length} addon${restored.length !== 1 ? "s" : ""}`);
        },
      },
      duration: 3000,
    });
    srAnnounce(`Removed ${count} addon${count !== 1 ? "s" : ""}. Press undo to restore.`);
  }, [addons, addonsPath, selectedFolders, updateResults, srAnnounce, restorePendingRemoval]);

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

    // Each selected addon toggles to the opposite of its current state.
    const entries = Array.from(selectedFolders)
      .map((folderName) => {
        const addon = addons.find((a) => a.folderName === folderName);
        return addon ? { folderName, enable: addon.disabled } : null;
      })
      .filter((e): e is { folderName: string; enable: boolean } => e !== null);

    if (entries.length === 0) {
      setSelectedFolders(new Set());
      return;
    }

    setBatchDisabling(true);
    try {
      // One command instead of one invoke per addon — single metadata lock.
      const result = await invokeOrThrow<BatchEnableResult>("batch_set_enabled", {
        addonsPath,
        entries,
      });

      const parts: string[] = [];
      if (result.disabled.length > 0) parts.push(`disabled ${result.disabled.length}`);
      if (result.enabled.length > 0) parts.push(`enabled ${result.enabled.length}`);
      if (result.failed.length > 0) parts.push(`${result.failed.length} failed`);
      if (parts.length > 0) toast.success(parts.join(", "));

      // Patch the successfully toggled addons in place instead of a full rescan.
      const toggled = new Map<string, boolean>();
      for (const folderName of result.enabled) toggled.set(folderName, false);
      for (const folderName of result.disabled) toggled.set(folderName, true);
      if (toggled.size > 0) {
        setAddons((prev) =>
          prev.map((addon) =>
            toggled.has(addon.folderName)
              ? { ...addon, disabled: toggled.get(addon.folderName)! }
              : addon
          )
        );
        setSelectedAddon((prev) =>
          prev && toggled.has(prev.folderName)
            ? { ...prev, disabled: toggled.get(prev.folderName)! }
            : prev
        );
        // Rescan if any toggled addon is a dependency of another, so dependents'
        // missing/outdated badges stay accurate (the backend recomputes them
        // against the enabled folder set; the in-place patch cannot).
        if (togglingAffectsDependencies(new Set(toggled.keys()))) {
          void scanAddons(addonsPathRef.current);
        }
      }
    } catch (batchError) {
      toast.error(`Failed to update addons: ${getTauriErrorMessage(batchError)}`);
    } finally {
      setBatchDisabling(false);
      setSelectedFolders(new Set());
    }
  }, [addons, addonsPath, selectedFolders, togglingAffectsDependencies, scanAddons]);

  const handleBatchTag = useCallback(
    async (tag: string) => {
      if (selectedFolders.size === 0) return;

      // Compute the new tag set per addon; skip ones that already have the tag.
      const entries = Array.from(selectedFolders)
        .map((folderName) => {
          const addon = addons.find((a) => a.folderName === folderName);
          if (!addon || addon.tags.includes(tag)) return null;
          return { folderName, tags: [...addon.tags, tag] };
        })
        .filter((e): e is { folderName: string; tags: string[] } => e !== null);

      if (entries.length === 0) {
        toast.info(`All selected addons already have the "${tag}" tag`);
        setSelectedFolders(new Set());
        return;
      }

      try {
        // One command instead of one invoke per addon — single metadata lock.
        const result = await invokeOrThrow<BatchTagResult>("batch_set_tags", {
          addonsPath,
          entries,
        });

        if (result.updated.length > 0) {
          toast.success(
            `Tagged ${result.updated.length} addon${result.updated.length !== 1 ? "s" : ""} as "${tag}"`
          );
        }

        // Patch the tagged addons in place instead of a full rescan.
        const tagged = new Map(entries.map((e) => [e.folderName, e.tags]));
        const updatedSet = new Set(result.updated);
        setAddons((prev) =>
          prev.map((addon) =>
            updatedSet.has(addon.folderName)
              ? { ...addon, tags: tagged.get(addon.folderName)! }
              : addon
          )
        );
        setSelectedAddon((prev) =>
          prev && updatedSet.has(prev.folderName)
            ? { ...prev, tags: tagged.get(prev.folderName)! }
            : prev
        );
      } catch (batchError) {
        toast.error(`Failed to tag addons: ${getTauriErrorMessage(batchError)}`);
      } finally {
        setSelectedFolders(new Set());
      }
    },
    [addons, addonsPath, selectedFolders]
  );

  // Filter against the deferred query so a keystroke's urgent render is just
  // the input update (the list re-renders with the PREVIOUS filtered array and
  // bails); the filter+sort catches up in a non-blocking render right after.
  const deferredSearchQuery = useDeferredValue(searchQuery);
  const filteredAddons = useMemo(
    () =>
      filterAddons(addons, {
        searchQuery: deferredSearchQuery,
        filterMode,
        sortMode,
        updatesSet,
        effectiveTagFilter,
      }),
    [effectiveTagFilter, addons, filterMode, deferredSearchQuery, sortMode, updatesSet]
  );

  const selectedUpdateResult = useMemo(
    () =>
      selectedAddon
        ? (updateResults.find((result) => result.folderName === selectedAddon.folderName) ?? null)
        : null,
    [selectedAddon, updateResults]
  );

  const batchMode = selectedFolders.size > 0 && viewMode === "installed";
  const showUploaderIntro =
    uploaderIntroHasLog && authUser === null && !uploaderIntroDismissed && !authVerifying;

  const dismissUploaderIntro = useCallback(() => {
    setUploaderIntroDismissed(true);
    setUploaderIntroHasLog(false);
    void setSetting("uploaderIntroDismissed", true);
  }, []);

  const handleAuthChange = useCallback(
    (user: AuthUser | null) => {
      setAuthUser(user);
      if (user) {
        dismissUploaderIntro();
        srAnnounce(`Signed in as ${user.userName}`);
      } else {
        srAnnounce("Signed out of ESO Logs");
      }
    },
    [dismissUploaderIntro, srAnnounce]
  );

  const handleOpenDialog = useCallback((dialog: Exclude<ActiveDialog, null>) => {
    if (dialog === "log-upload") setLogUploaderMounted(true);
    setActiveDialog(dialog);
  }, []);

  const handleCloseDialog = useCallback(() => {
    setActiveDialog(null);
    setDeepLinkPackId(null);
    setDeepLinkShareCode(null);
  }, []);

  // Stable identities for every callback prop below. The children are
  // memoized, so a fresh lambda per render would defeat their bailout and
  // re-render the whole tree on every keystroke / progress event / dialog
  // toggle.
  const handleBatchCancel = useCallback(() => setSelectedFolders(new Set()), []);
  const handleBatchDisableClick = useCallback(
    () => void handleBatchDisable(),
    [handleBatchDisable]
  );
  const handleBatchRemoveClick = useCallback(() => void handleBatchRemove(), [handleBatchRemove]);
  const handleBatchUpdateClick = useCallback(() => void handleBatchUpdate(), [handleBatchUpdate]);
  const handleOpenPacks = useCallback(() => setActiveDialog("packs"), []);
  const handleOpenProfiles = useCallback(() => setActiveDialog("profiles"), []);
  const handleOpenSavedVars = useCallback(() => setActiveDialog("saved-variables"), []);
  const handleOpenSettings = useCallback(() => setActiveDialog("settings"), []);
  const handleOpenLogUpload = useCallback(() => {
    dismissUploaderIntro();
    setLogUploaderMounted(true);
    setActiveDialog("log-upload");
  }, [dismissUploaderIntro]);
  const handleUpdateAddonClick = useCallback(
    (folderName: string) => void handleSingleUpdate(folderName),
    [handleSingleUpdate]
  );
  const handleRemoveAddonClick = useCallback(
    (folderName: string) => void handleSingleRemove(folderName),
    [handleSingleRemove]
  );
  const handleOpenFolderClick = useCallback(
    (folderName: string) => void handleOpenFolder(folderName),
    [handleOpenFolder]
  );
  const handleConflictResolved = useCallback(
    (folderName: string) => {
      setPendingConflicts((prev) => {
        const next = new Map(prev);
        next.delete(folderName);
        return next;
      });
      handleAddonUpdated(updateResults.find((r) => r.folderName === folderName)?.esouiId ?? 0);
    },
    [handleAddonUpdated, updateResults]
  );
  const handleCheckForAppUpdateClick = useCallback(
    () => void checkForAppUpdate(false),
    [checkForAppUpdate]
  );
  const handlePathChangeClick = useCallback(
    (path: string) => void handlePathChange(path),
    [handlePathChange]
  );
  const handleEsoRunningConfirm = useCallback((dontAskAgain: boolean) => {
    setEsoRunningPromptOpen(false);
    if (dontAskAgain) void setSetting("suppressEsoRunningWarning", true);
    esoRunningResolveRef.current?.(true);
    esoRunningResolveRef.current = null;
  }, []);
  const handleEsoRunningCancel = useCallback(() => {
    setEsoRunningPromptOpen(false);
    esoRunningResolveRef.current?.(false);
    esoRunningResolveRef.current = null;
  }, []);
  // Closing and draining are separate steps on purpose. Closing is synchronous
  // so the picker cannot be submitted twice while an async decision is still
  // settling; draining waits until that decision has fully landed.
  const closeDependencyPrompt = useCallback(() => {
    setDepPromptOpen(false);
    depPromptOpenRef.current = false;
  }, []);

  // Hand the picker back to anything that queued up behind it. Re-run through
  // `resolvePendingDeps` rather than shown directly, so the queue is re-filtered
  // against the skip list the decision may have just written to. `justInstalled`
  // drops entries the user already accepted: the queue holds a snapshot taken
  // before that install, so without this the same library is offered again
  // moments after being installed.
  const drainQueuedDeps = useCallback(
    async (justInstalled: readonly string[] = [], installedInPath = "") => {
      const batches = queuedDepBatchesRef.current;
      if (batches.length === 0) return;
      queuedDepBatchesRef.current = [];
      const handled = new Set(justInstalled.map((name) => name.toLowerCase()));
      for (const batch of batches) {
        // No folder filter here. Switching instances discards the queue up front
        // and tells the user, so anything still queued belongs to the current
        // folder. Skipping silently at this point would drop required libraries
        // on the floor with no message — the exact failure this feature exists
        // to prevent.
        //
        // An install only accounts for the folder it wrote to. The same library
        // name missing from a different instance is a different missing file.
        const remaining = sameAddonsFolder(batch.addonsPath, installedInPath)
          ? batch.deps.filter((dep) => !handled.has(dep.name.toLowerCase()))
          : batch.deps;
        if (remaining.length === 0) continue;
        await presentDependencyBatch(remaining, batch.addonsPath);
      }
    },
    [presentDependencyBatch]
  );

  // Only ever a dismissal now: the dialog's own buttons no longer close it, so
  // this cannot be confused with a decision. Nothing is installed and nothing is
  // declined, but required libraries must still not go by unmentioned.
  const handleDependencyPromptOpenChange = useCallback(
    (open: boolean) => {
      if (open) {
        setDepPromptOpen(true);
        return;
      }
      const stillMissing = pendingDeps.filter((dep) => dep.required);
      if (stillMissing.length > 0) {
        toast.warning(
          `${stillMissing.length} required ${
            stillMissing.length === 1 ? "library is" : "libraries are"
          } still missing: ${stillMissing.map((dep) => dep.name).join(", ")}. Install ${
            stillMissing.length === 1 ? "it" : "them"
          } from the addon's Details tab.`
        );
      }
      closeDependencyPrompt();
      void drainQueuedDeps();
    },
    [pendingDeps, closeDependencyPrompt, drainQueuedDeps]
  );
  const handleDependencyConfirm = useCallback(
    (selectedNames: string[], alwaysAutoInstall: boolean, rememberDeclines: boolean) => {
      // Synchronous latch, checked and set before any await. The dialog stays
      // interactive until the close below lands, so a fast second click would
      // otherwise run this whole handler twice — duplicate warnings and a
      // duplicate install call.
      if (depDecisionInFlightRef.current) return;
      depDecisionInFlightRef.current = true;

      // Fire-and-forget preference writes, exactly like conflictPolicy.
      if (alwaysAutoInstall) void setDependencyPolicy("auto");

      const selected = new Set(selectedNames);

      // "Skip all" gives the user no time to read anything in the dialog, so the
      // in-dialog warning about declining a required library never lands on that
      // path. Say it here, so no required dependency is turned down silently.
      const declinedRequired = pendingDeps.filter((dep) => dep.required && !selected.has(dep.name));
      if (declinedRequired.length > 0) {
        toast.warning(
          `Skipped ${declinedRequired.length} required ${
            declinedRequired.length === 1 ? "library" : "libraries"
          }: ${declinedRequired.map((dep) => dep.name).join(", ")}. Addons needing ${
            declinedRequired.length === 1 ? "it" : "them"
          } won't load.`
        );
      }

      // Read the destination before anything can await, and refuse if the user
      // has since switched instances. These libraries belong to the folder they
      // were resolved for, and the backend registers exactly one allowed AddOns
      // path at a time (require_allowed_path), so installing them elsewhere is
      // both wrong and impossible — the command would reject the call. Say so
      // rather than firing a request that can only fail.
      const targetAddonsPath = depPromptPathRef.current || addonsPathRef.current;
      if (!sameAddonsFolder(targetAddonsPath, addonsPathRef.current)) {
        toast.warning(
          "Those dependencies were for the previous AddOns folder. Switch back and reinstall the addon to add them."
        );
        closeDependencyPrompt();
        depDecisionInFlightRef.current = false;
        void drainQueuedDeps();
        return;
      }

      // Close now, synchronously — nothing below should leave the user looking
      // at a modal while an install runs, and it closes the double-click window.
      closeDependencyPrompt();

      // No ESO-running gate here: this prompt only ever follows an install or
      // update that just passed it and wrote to the same folder, so re-warning
      // would stack a second modal on the one the user just dismissed.
      void (async () => {
        // Filled from the install result below. Stays empty when nothing was
        // selected, and deliberately also when the call threw: the backend can
        // fail after writing folders (a metadata save at the end), so on an
        // error we genuinely do not know what landed. Leaving it empty re-offers
        // those libraries, which at worst reinstalls something already on disk.
        // Assuming the selection succeeded would be the other way round — a
        // required library silently never mentioned again. Do not "optimise"
        // this into filtering by selectedNames on the error path.
        let installedNames: string[] = [];
        try {
          // Persist the declines BEFORE the queue drains, or a name just turned
          // down gets offered straight back by whatever queued up behind this
          // prompt. Only remember them when the user asked for that: persisting
          // every untick would be a one-way door, since optional dependencies
          // arrive unticked and would vanish without ever being refused.
          if (rememberDeclines) {
            const declined = pendingDeps
              .filter((dep) => !selected.has(dep.name))
              .map((dep) => dep.name);
            // Report a failed write. Otherwise the user ticked "don't offer
            // these again", nothing was recorded, and the very next update
            // offers them right back with no explanation.
            if (declined.length > 0 && !(await addSkippedDependencies(declined))) {
              toast.warning("Could not save your choice — these libraries may be offered again.");
            }
          }

          if (selectedNames.length === 0) return;
          try {
            const result = await invokeOrThrow<InstallResult>("install_selected_dependencies", {
              addonsPath: targetAddonsPath,
              depNames: selectedNames,
            });
            // What actually landed, which is neither more nor less than what was
            // asked for: accepting one library pulls its own transitively, and a
            // requested one can fail outright. The queue must be filtered by this
            // rather than by the request, or a transitively-installed library is
            // offered again while a failed one is silently dropped.
            installedNames = result.installedDeps;
            const installed = result.installedDeps.length;
            if (installed > 0) {
              toast.success(
                `Installed ${installed} ${installed === 1 ? "dependency" : "dependencies"}`
              );
            }
            // `skippedDeps` means "not found on ESOUI" — as unusable as a failure.
            const unresolved = [...result.failedDeps, ...result.skippedDeps];
            if (unresolved.length > 0) {
              toast.warning(`Could not install: ${unresolved.join(", ")}`);
            }
          } catch (e) {
            toast.error(`Failed to install dependencies: ${getTauriErrorMessage(e)}`);
          } finally {
            // Refresh even after an error: a partially applied selection still
            // reached disk and must show up in the list.
            handleRefresh();
          }
        } finally {
          // Release the latch and only then drain, so the queue opens after this
          // decision has fully landed. In a `finally` so a throw in the
          // persistence step above cannot strand the picker shut forever.
          depDecisionInFlightRef.current = false;
          await drainQueuedDeps(installedNames, targetAddonsPath);
        }
      })();
    },
    [handleRefresh, pendingDeps, closeDependencyPrompt, drainQueuedDeps]
  );

  if (setupInstances !== null) {
    return (
      <div className="relative flex h-screen flex-col">
        <AppBackground />
        <SetupWizard instances={setupInstances} onSelect={(path) => void handleSetupSelect(path)} />
      </div>
    );
  }

  return (
    <EsoRunningProvider value={ensureEsoNotBlocking}>
      <DependencyPromptProvider value={resolvePendingDeps}>
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
            authUser={authUser}
            authVerifying={authVerifying}
            instances={knownInstances}
            activeAddonsPath={addonsPath}
            onSwitchInstance={handlePathChangeClick}
            onBatchCancel={handleBatchCancel}
            onBatchDisable={handleBatchDisableClick}
            onBatchRemove={handleBatchRemoveClick}
            onBatchTag={handleBatchTag}
            onBatchUpdate={handleBatchUpdateClick}
            onOpenPacks={handleOpenPacks}
            onOpenProfiles={handleOpenProfiles}
            onOpenSavedVars={handleOpenSavedVars}
            onOpenSettings={handleOpenSettings}
            onOpenLogUpload={handleOpenLogUpload}
            onAuthChange={handleAuthChange}
            onRefresh={handleRefresh}
          />

          <StatusBanners
            error={error}
            isOffline={isOffline}
            appUpdateState={appUpdateState}
            onDownload={downloadAndInstall}
            onRestart={restartApp}
            onOpenSettings={errorShowSettings ? handleOpenSettings : undefined}
          />

          <UpdateBanner
            availableCount={updatesAvailable.length}
            updatingAll={updatingAll}
            updateProgress={updateProgress}
            addonStatuses={addonStatuses}
            updates={bannerUpdates}
            onUpdateAll={handleUpdateAll}
            onUpdateSelected={handleUpdateSelected}
            isOffline={isOffline}
          />

          {showUploaderIntro && (
            <UploaderIntroCard
              onOpenLogUpload={handleOpenLogUpload}
              onDismiss={dismissUploaderIntro}
            />
          )}

          {pendingConflicts.size > 0 && (
            <div className="mx-4 mb-2 flex items-center gap-2 rounded-lg border border-status-warning-strong/20 bg-status-warning-strong/[0.04] px-3 py-2 text-xs text-status-warning">
              <span className="h-2 w-2 rounded-full bg-status-warning animate-pulse" />
              {pendingConflicts.size} addon{pendingConflicts.size !== 1 ? "s" : ""} need your
              attention — click one to review your edited files
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
              onUpdateAddon={handleUpdateAddonClick}
              onRemoveAddon={handleRemoveAddonClick}
              onToggleDisable={handleToggleDisable}
              onOpenFolder={handleOpenFolderClick}
              onToggleFavorite={handleTagsChange}
            />

            {viewMode === "installed" ? (
              <Suspense
                fallback={
                  <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
                    <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-structure-10 border-t-primary" />
                    Loading addon details...
                  </div>
                }
              >
                <AddonDetail
                  key={selectedAddon?.folderName ?? "none"}
                  addon={selectedAddon}
                  installedAddons={addons}
                  addonsPath={addonsPath}
                  onRefresh={handleRefresh}
                  onRemoveAddon={handleSingleRemove}
                  onToggleDisable={handleToggleDisable}
                  updateResult={selectedUpdateResult}
                  onAddonUpdated={handleAddonUpdated}
                  onTagsChange={handleTagsChange}
                  isOffline={isOffline}
                  pendingConflict={
                    selectedAddon ? pendingConflicts.get(selectedAddon.folderName) : undefined
                  }
                  onConflictResolved={handleConflictResolved}
                />
              </Suspense>
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
            authVerifying={authVerifying}
            deepLinkPackId={deepLinkPackId}
            deepLinkShareCode={deepLinkShareCode}
            knownInstances={knownInstances}
            logUploaderMounted={logUploaderMounted}
            onAuthChange={handleAuthChange}
            onCheckForAppUpdate={handleCheckForAppUpdateClick}
            onCloseDialog={handleCloseDialog}
            onInstancesDetected={setKnownInstances}
            onPathChange={handlePathChangeClick}
            onRefresh={handleRefresh}
            onShowDialog={handleOpenDialog}
          />

          <EsoRunningDialog
            open={esoRunningPromptOpen}
            onConfirm={handleEsoRunningConfirm}
            onCancel={handleEsoRunningCancel}
          />

          <DependencyPickerDialog
            open={depPromptOpen}
            onOpenChange={handleDependencyPromptOpenChange}
            pending={pendingDeps}
            onConfirm={handleDependencyConfirm}
          />

          {cfaDialog && (
            <CfaGuidanceDialog
              open
              onClose={() => setCfaDialog(null)}
              exePath={cfaDialog.exePath}
              permissionDenied={cfaDialog.permissionDenied}
            />
          )}
        </div>
      </DependencyPromptProvider>
    </EsoRunningProvider>
  );
}

export default App;
