import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { toast } from "sonner";
import type {
  Pack,
  PackPage,
  PackAddonEntry,
  InstallResult,
  EsouiAddonInfo,
  AddonManifest,
  AuthUser,
  ShareCodeResponse,
  SharedPack,
  EsoPackFile,
  InstalledPackRef,
} from "../types";
import { getSetting, setSetting } from "@/lib/store";
import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import type { PackTypeFilter, SortOption, TabMode } from "./pack-constants";

interface VoteResponse {
  voted: boolean;
  voteCount: number;
}
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { getTauriErrorMessage, invokeOrThrow, invokeResult } from "@/lib/tauri";
import { cn, decodeHtml } from "@/lib/utils";
import { PackageIcon, DownloadIcon, ArrowLeftIcon, Loader2Icon, ImportIcon } from "lucide-react";
import { motion, AnimatePresence } from "motion/react";

// Sub-components
import { PackListView } from "./pack-browse";
import { PackDetailView } from "./pack-detail";
import { PackCreateView } from "./pack-create";
import { PackImportView } from "./pack-import";
import { MyPacksView } from "./pack-my-packs";

// ── Main Packs Component ──────────────────────────────────────────────────

interface PacksProps {
  addonsPath: string;
  installedAddons: AddonManifest[];
  authUser: AuthUser | null;
  onAuthChange: (user: AuthUser | null) => void;
  onClose: () => void;
  onRefresh: () => void;
  initialPackId?: string | null;
  initialShareCode?: string | null;
}

export function Packs({
  addonsPath,
  installedAddons,
  authUser,
  onAuthChange,
  onClose,
  onRefresh,
  initialPackId,
  initialShareCode,
}: PacksProps) {
  const [tab, setTab] = useState<TabMode>("browse");
  const [showImportPanel, setShowImportPanel] = useState(!!initialShareCode);
  const [packs, setPacks] = useState<Pack[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [typeFilter, setTypeFilter] = useState<PackTypeFilter>("all");
  const [sortMode, setSortMode] = useState<SortOption>("votes");
  const [currentPage, setCurrentPage] = useState(1);
  const [hasMore, setHasMore] = useState(false);
  const [confirmInstall, setConfirmInstall] = useState(false);

  const installedEsouiIds = useMemo(
    () => new Set(installedAddons.filter((a) => a.esouiId && a.esouiId > 0).map((a) => a.esouiId!)),
    [installedAddons]
  );

  // Detail view
  const [selectedPack, setSelectedPack] = useState<Pack | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);

  // Create pack form state (lifted here so tab switches don't reset it)
  const [createStep, setCreateStep] = useState<"details" | "addons">("details");
  const [createTitle, setCreateTitle] = useState("");
  const [createDescription, setCreateDescription] = useState("");
  const [createPackType, setCreatePackType] = useState("addon-pack");
  const [createTags, setCreateTags] = useState<string[]>([]);
  const [createAddons, setCreateAddons] = useState<PackAddonEntry[]>([]);
  const [createAnonymous, setCreateAnonymous] = useState(false);
  const [editingPackId, setEditingPackId] = useState<string | null>(null);

  // Sharing state
  const [shareResult, setShareResult] = useState<ShareCodeResponse | null>(null);
  const [generatingShare, setGeneratingShare] = useState(false);
  const [showShareSection, setShowShareSection] = useState(false);
  const [copiedField, setCopiedField] = useState<"code" | "link" | null>(null);

  // Import state
  const [shareCodeInput, setShareCodeInput] = useState(initialShareCode ?? "");
  const [resolvingCode, setResolvingCode] = useState(false);
  const [importedPack, setImportedPack] = useState<SharedPack | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  // Installation — selected addons (esouiId set)
  const [installing, setInstalling] = useState(false);
  const [installProgress, setInstallProgress] = useState<{
    completed: number;
    failed: number;
    total: number;
  } | null>(null);
  const [selectedAddons, setSelectedAddons] = useState<Set<number>>(new Set());

  // My Packs state
  const [myPacks, setMyPacks] = useState<Pack[]>([]);
  const [myPacksLoading, setMyPacksLoading] = useState(false);
  const [myPacksLoadingMore, setMyPacksLoadingMore] = useState(false);
  const [myPacksPage, setMyPacksPage] = useState(1);
  const [myPacksHasMore, setMyPacksHasMore] = useState(false);
  const [duplicatingPackId, setDuplicatingPackId] = useState<string | null>(null);

  // Delete state
  const [deletingPack, setDeletingPack] = useState(false);

  // Install success flash state
  const [installSucceeded, setInstallSucceeded] = useState(false);

  // Installed packs library (persisted locally)
  const [installedPackRefs, setInstalledPackRefs] = useState<InstalledPackRef[]>([]);

  useEffect(() => {
    getSetting<InstalledPackRef[]>("installed_packs", []).then(setInstalledPackRefs);
  }, []);

  // When a pack is selected, pre-select all required addons
  useEffect(() => {
    if (selectedPack) {
      setSelectedAddons(
        new Set(selectedPack.addons.filter((a) => a.required).map((a) => a.esouiId))
      );
    }
  }, [selectedPack]);

  const searchQueryRef = useRef(searchQuery);
  searchQueryRef.current = searchQuery;

  const loadPacksSeqRef = useRef(0);
  const loadPacks = useCallback(
    async (q: string, page: number = 1) => {
      const seq = ++loadPacksSeqRef.current;
      if (page === 1) {
        setLoading(true);
      } else {
        setLoadingMore(true);
      }
      setError(null);
      try {
        const result = await invokeOrThrow<PackPage>("list_packs", {
          packType: typeFilter === "all" ? null : typeFilter,
          tag: null,
          query: q || null,
          sort: sortMode,
          page,
        });
        // Discard stale response if a newer request was fired
        if (seq !== loadPacksSeqRef.current) return;
        // Filter out drafts — browse should only show published packs
        const published = result.packs.filter((p) => p.status !== "draft");
        if (page === 1) {
          setPacks(published);
        } else {
          setPacks((prev) => [...prev, ...published]);
        }
        setCurrentPage(result.page);
        // If the API returned fewer results than the page size, there are no more pages
        const PAGE_SIZE = 10;
        setHasMore(result.packs.length >= PAGE_SIZE);
      } catch (e) {
        if (seq !== loadPacksSeqRef.current) return;
        const msg = getTauriErrorMessage(e);
        if (msg.includes("connect") || msg.includes("internet")) {
          setError("Could not reach Pack Hub. Check your internet connection and try again.");
        } else if (msg.includes("parse") || msg.includes("JSON")) {
          setError("Pack Hub returned an unexpected response. This may be a temporary issue.");
        } else {
          setError("Something went wrong loading packs. Please try again.");
        }
      } finally {
        if (seq === loadPacksSeqRef.current) {
          setLoading(false);
          setLoadingMore(false);
        }
      }
    },
    [typeFilter, sortMode]
  );

  const handleLoadMore = () => {
    loadPacks(searchQuery, currentPage + 1);
  };

  // Debounce search queries (400ms), but load immediately on type filter change
  const searchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (searchTimerRef.current) clearTimeout(searchTimerRef.current);
    searchTimerRef.current = setTimeout(
      () => {
        loadPacks(searchQuery, 1);
      },
      searchQuery ? 400 : 0
    );
    return () => {
      if (searchTimerRef.current) clearTimeout(searchTimerRef.current);
    };
  }, [searchQuery, loadPacks]);

  const selectPackSeqRef = useRef(0);
  const handleSelectPack = useCallback(async (id: string) => {
    const seq = ++selectPackSeqRef.current;
    setLoadingDetail(true);
    try {
      const pack = await invokeOrThrow<Pack>("get_pack", { id });
      if (seq !== selectPackSeqRef.current) return;
      setSelectedPack(pack);
    } catch (e) {
      if (seq !== selectPackSeqRef.current) return;
      toast.error(`Failed to load pack: ${getTauriErrorMessage(e)}`);
    } finally {
      if (seq === selectPackSeqRef.current) {
        setLoadingDetail(false);
      }
    }
  }, []);

  // ── My Packs loader ──────────────────────────────────────────────
  const loadMyPacksSeqRef = useRef(0);
  const authUserRef = useRef(authUser);
  authUserRef.current = authUser;

  const loadMyPacks = useCallback(
    async (page: number = 1) => {
      const currentUser = authUserRef.current;
      if (!currentUser) return;
      const seq = ++loadMyPacksSeqRef.current;
      if (page === 1) {
        setMyPacksLoading(true);
      } else {
        setMyPacksLoadingMore(true);
      }
      try {
        const result = await invokeOrThrow<PackPage>("list_packs", {
          packType: null,
          tag: null,
          query: null,
          sort: "newest",
          page,
          author: currentUser.userId,
          status: "all",
        });
        if (seq !== loadMyPacksSeqRef.current) return;
        if (page === 1) {
          setMyPacks(result.packs);
        } else {
          setMyPacks((prev) => [...prev, ...result.packs]);
        }
        setMyPacksPage(result.page);
        const PAGE_SIZE = 10;
        setMyPacksHasMore(result.packs.length >= PAGE_SIZE);
      } catch (e) {
        if (seq !== loadMyPacksSeqRef.current) return;
        toast.error(`Failed to load your packs: ${getTauriErrorMessage(e)}`);
      } finally {
        if (seq === loadMyPacksSeqRef.current) {
          setMyPacksLoading(false);
          setMyPacksLoadingMore(false);
        }
      }
    },
    [] // stable — reads authUser from ref
  );

  // Load my packs when tab is switched to "my-packs" or user signs in
  useEffect(() => {
    if (tab === "my-packs" && authUser) {
      loadMyPacks(1);
    }
  }, [tab, authUser, loadMyPacks]); // loadMyPacks is stable; authUser triggers reload on sign-in

  // ── Delete pack handler ──────────────────────────────────────────
  const handleDeletePack = async (packId: string) => {
    setDeletingPack(true);
    try {
      await invokeOrThrow("delete_pack", { id: packId });
      toast.success("Pack deleted");
      // Remove from local state
      setPacks((prev) => prev.filter((p) => p.id !== packId));
      setMyPacks((prev) => prev.filter((p) => p.id !== packId));
      // If we're in detail view for the deleted pack, go back.
      // We read from a functional updater to avoid stale closure.
      let wasViewing = false;
      setSelectedPack((prev) => {
        wasViewing = prev?.id === packId;
        return wasViewing ? null : prev;
      });
      if (wasViewing) {
        setConfirmInstall(false);
        setInstalling(false);
        setInstallProgress(null);
        setShowShareSection(false);
        setShareResult(null);
      }
      // Refresh browse list — use ref to avoid stale closure
      loadPacks(searchQueryRef.current, 1);
    } catch (e) {
      toast.error(`Failed to delete pack: ${getTauriErrorMessage(e)}`);
    } finally {
      setDeletingPack(false);
    }
  };

  // Auto-open a specific pack when triggered via deep link
  useEffect(() => {
    if (initialPackId) {
      handleSelectPack(initialPackId);
    }
  }, [initialPackId, handleSelectPack]);

  const handleBack = () => {
    setSelectedPack(null);
    setConfirmInstall(false);
    setInstalling(false);
    setInstallProgress(null);
    setShowShareSection(false);
    setShareResult(null);
  };

  const resetCreateForm = useCallback(() => {
    setCreateTitle("");
    setCreateDescription("");
    setCreatePackType("addon-pack");
    setCreateTags([]);
    setCreateAddons([]);
    setCreateAnonymous(false);
    setCreateStep("details");
    setEditingPackId(null);
  }, []);

  // ── Share code generation ──────────────────────────────────────────
  const handleGenerateShareCode = async (pack: Pack) => {
    setGeneratingShare(true);
    setShareResult(null);
    try {
      const result = await invokeOrThrow<ShareCodeResponse>("create_share_code", {
        payload: {
          title: pack.title,
          description: pack.description,
          packType: pack.packType,
          tags: pack.tags,
          addons: pack.addons,
        },
      });
      setShareResult(result);
    } catch (e) {
      toast.error(`Failed to generate share code: ${getTauriErrorMessage(e)}`);
    } finally {
      setGeneratingShare(false);
    }
  };

  const handleCopyToClipboard = async (text: string, field: "code" | "link") => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedField(field);
      setTimeout(() => setCopiedField(null), 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  };

  const handleExportPackFile = async (pack: Pack) => {
    const safeName = pack.title
      .replace(/[^a-zA-Z0-9-_ ]/g, "")
      .trim()
      .replace(/\s+/g, "-");
    const path = await saveFileDialog({
      defaultPath: `${safeName}.esopack`,
      filters: [{ name: "ESO Pack", extensions: ["esopack"] }],
    });
    if (!path) return;

    try {
      await invokeOrThrow("export_pack_file", {
        pack: {
          format: "esopack",
          version: 1,
          pack: {
            title: pack.title,
            description: pack.description,
            packType: pack.packType,
            tags: pack.tags,
            addons: pack.addons,
          },
          sharedAt: new Date().toISOString(),
          sharedBy: authUser?.userName ?? "Anonymous",
        },
        path,
      });
      toast.success("Pack exported successfully");
    } catch (e) {
      toast.error(`Failed to export pack: ${getTauriErrorMessage(e)}`);
    }
  };

  // ── Import handlers ──────────────────────────────────────────────
  const handleResolveShareCode = async (code: string) => {
    const trimmed = code.trim().toUpperCase();
    if (!trimmed) return;

    setResolvingCode(true);
    setImportError(null);
    setImportedPack(null);
    try {
      const pack = await invokeOrThrow<SharedPack>("resolve_share_code", { code: trimmed });
      setImportedPack(pack);
    } catch (e) {
      setImportError(getTauriErrorMessage(e));
    } finally {
      setResolvingCode(false);
    }
  };

  const handleImportFile = async () => {
    const path = await openFileDialog({
      filters: [{ name: "ESO Pack", extensions: ["esopack"] }],
      multiple: false,
    });
    if (!path) return;

    setImportError(null);
    setImportedPack(null);
    try {
      const result = await invokeOrThrow<EsoPackFile>("import_pack_file", { path });
      setImportedPack({
        title: result.pack.title,
        description: result.pack.description,
        packType: result.pack.packType,
        tags: result.pack.tags,
        addons: result.pack.addons,
        sharedBy: result.sharedBy,
        sharedAt: result.sharedAt,
        expiresAt: "",
      });
    } catch (e) {
      setImportError(getTauriErrorMessage(e));
    }
  };

  // Auto-resolve share code from deep link
  useEffect(() => {
    if (initialShareCode) {
      setShareCodeInput(initialShareCode);
      setShowImportPanel(true);
      handleResolveShareCode(initialShareCode);
    }
  }, [initialShareCode]);

  // Selected addons for imported pack preview
  const importedPackAddonsToInstall = useMemo(() => {
    if (!importedPack) return [];
    return importedPack.addons
      .filter((a) => a.required)
      .filter((a) => !installedEsouiIds.has(a.esouiId));
  }, [importedPack, installedEsouiIds]);

  const handleInstallImportedPack = async () => {
    if (!importedPack || importedPackAddonsToInstall.length === 0) return;

    setInstalling(true);
    setInstallProgress({ completed: 0, failed: 0, total: importedPackAddonsToInstall.length });

    let completed = 0;
    let failed = 0;
    const installed: string[] = [];

    for (const addon of importedPackAddonsToInstall) {
      const info = await invokeResult<EsouiAddonInfo>("resolve_esoui_addon", {
        input: String(addon.esouiId),
      });
      if (!info.ok) {
        failed++;
        setInstallProgress({ completed, failed, total: importedPackAddonsToInstall.length });
        continue;
      }

      const result = await invokeResult<InstallResult>("install_addon", {
        addonsPath,
        downloadUrl: info.data.downloadUrl,
        esouiId: addon.esouiId,
        esouiTitle: info.data.title,
        esouiVersion: info.data.version,
      });

      if (result.ok) {
        completed++;
        installed.push(...result.data.installedFolders);
      } else {
        failed++;
      }

      setInstallProgress({ completed, failed, total: importedPackAddonsToInstall.length });
    }

    setInstalling(false);
    setInstallProgress(null);

    if (installed.length > 0) {
      onRefresh();
      toast.success(`Installed ${installed.length} addon${installed.length !== 1 ? "s" : ""}`);
    }
    if (failed > 0) {
      toast.error(`${failed} addon${failed !== 1 ? "s" : ""} failed to install`);
    }
  };

  // Flash green on successful install completion (Task D)
  const prevInstallingRef = useRef(false);
  const lastInstallFailedRef = useRef(0);
  useEffect(() => {
    // Track failure count while installing
    if (installing && installProgress) {
      lastInstallFailedRef.current = installProgress.failed;
    }
    if (prevInstallingRef.current && !installing && installProgress === null) {
      // Install just finished — only flash green if no failures
      if (lastInstallFailedRef.current === 0) {
        setInstallSucceeded(true);
        const timer = setTimeout(() => setInstallSucceeded(false), 1500);
        return () => clearTimeout(timer);
      }
      lastInstallFailedRef.current = 0;
    }
    prevInstallingRef.current = installing;
  }, [installing, installProgress]);

  const handleStartEditing = useCallback((pack: Pack) => {
    setCreateTitle(decodeHtml(pack.title));
    setCreateDescription(decodeHtml(pack.description));
    setCreatePackType(pack.packType);
    setCreateTags([...pack.tags]);
    setCreateAddons(
      pack.addons.map((addon) => ({
        esouiId: addon.esouiId,
        name: decodeHtml(addon.name),
        required: addon.required,
        note: addon.note ? decodeHtml(addon.note) : undefined,
      }))
    );
    setCreateAnonymous(pack.isAnonymous);
    setCreateStep("details");
    setEditingPackId(pack.id);
    setSelectedPack(null);
    setTab("create");
  }, []);

  const canEditSelectedPack = !!(
    selectedPack &&
    authUser &&
    selectedPack.authorId &&
    selectedPack.authorId === authUser.userId
  );

  const handleToggleAddon = (esouiId: number, required: boolean) => {
    // Required addons can't be deselected
    if (required) return;
    setSelectedAddons((prev) => {
      const next = new Set(prev);
      if (next.has(esouiId)) {
        next.delete(esouiId);
      } else {
        next.add(esouiId);
      }
      return next;
    });
  };

  const newAddonsToInstall = useMemo(
    () =>
      selectedPack
        ? selectedPack.addons.filter(
            (a) => selectedAddons.has(a.esouiId) && !installedEsouiIds.has(a.esouiId)
          )
        : [],
    [selectedPack, selectedAddons, installedEsouiIds]
  );

  const handleInstallPack = async () => {
    if (!selectedPack) return;
    if (newAddonsToInstall.length === 0) {
      toast.info("All selected addons are already installed.");
      return;
    }

    setConfirmInstall(false);
    setInstalling(true);
    setInstallProgress({ completed: 0, failed: 0, total: newAddonsToInstall.length });

    let completed = 0;
    let failed = 0;
    const failedNames: string[] = [];
    let totalDepsInstalled = 0;

    for (const addon of newAddonsToInstall) {
      const info = await invokeResult<EsouiAddonInfo>("resolve_esoui_addon", {
        input: String(addon.esouiId),
      });
      if (!info.ok) {
        failed++;
        failedNames.push(addon.name);
        setInstallProgress({ completed, failed, total: newAddonsToInstall.length });
        continue;
      }

      const install = await invokeResult<InstallResult>("install_addon", {
        addonsPath,
        downloadUrl: info.data.downloadUrl,
        esouiId: addon.esouiId,
        esouiTitle: info.data.title,
        esouiVersion: info.data.version,
      });

      if (install.ok) {
        completed++;
        totalDepsInstalled += install.data.installedDeps.length;
      } else {
        failed++;
        failedNames.push(addon.name);
      }

      setInstallProgress({ completed, failed, total: newAddonsToInstall.length });
    }

    setInstalling(false);
    setInstallProgress(null);

    const depNote =
      totalDepsInstalled > 0
        ? ` (+${totalDepsInstalled} dependenc${totalDepsInstalled !== 1 ? "ies" : "y"})`
        : "";

    if (failed > 0) {
      toast.warning(
        `Installed ${completed} addon${completed !== 1 ? "s" : ""}${depNote}, ${failed} failed: ${failedNames.join(", ")}`
      );
    } else {
      toast.success(
        `Installed ${completed} addon${completed !== 1 ? "s" : ""}${depNote} from "${decodeHtml(selectedPack.title)}"`
      );
    }

    // Track the install count and save to installed packs library
    if (completed > 0 && selectedPack) {
      invokeResult("track_pack_install", { packId: selectedPack.id }).catch(() => {});

      // Save to local installed packs library
      const ref: InstalledPackRef = {
        packId: selectedPack.id,
        title: selectedPack.title,
        packType: selectedPack.packType as "addon-pack" | "build-pack" | "roster-pack",
        authorName: selectedPack.authorName,
        addonCount: selectedPack.addons.length,
        installedAt: new Date().toISOString(),
      };
      setInstalledPackRefs((prev) => {
        const updated = [ref, ...prev.filter((r) => r.packId !== ref.packId)];
        setSetting("installed_packs", updated);
        return updated;
      });
    }

    onRefresh();
  };

  // ── Voting ──────────────────────────────────────────────────────────
  const [votingPacks, setVotingPacks] = useState<Set<string>>(new Set());

  const handleVote = async (packId: string) => {
    if (!authUser) {
      toast.error("Sign in to vote on packs.");
      return;
    }
    if (votingPacks.has(packId)) return; // debounce

    // Optimistic update helper
    const applyVote = (pack: Pack): Pack => {
      const willVote = !pack.userVoted;
      return {
        ...pack,
        userVoted: willVote,
        voteCount: pack.voteCount + (willVote ? 1 : -1),
      };
    };

    // Optimistic: update list + detail
    setPacks((prev) => prev.map((p) => (p.id === packId ? applyVote(p) : p)));
    setSelectedPack((prev) => (prev?.id === packId ? applyVote(prev) : prev));

    setVotingPacks((prev) => new Set(prev).add(packId));
    try {
      const result = await invokeOrThrow<VoteResponse>("vote_pack", { packId });
      // Reconcile with server truth
      const reconcile = (pack: Pack): Pack => ({
        ...pack,
        userVoted: result.voted,
        voteCount: result.voteCount,
      });
      setPacks((prev) => prev.map((p) => (p.id === packId ? reconcile(p) : p)));
      setSelectedPack((prev) => (prev?.id === packId ? reconcile(prev) : prev));
    } catch (e) {
      // Revert optimistic update
      const revert = (pack: Pack): Pack => {
        const wasVoted = !pack.userVoted;
        return {
          ...pack,
          userVoted: wasVoted,
          voteCount: pack.voteCount + (wasVoted ? 1 : -1),
        };
      };
      setPacks((prev) => prev.map((p) => (p.id === packId ? revert(p) : p)));
      setSelectedPack((prev) => (prev?.id === packId ? revert(prev) : prev));
      const msg = getTauriErrorMessage(e);
      if (msg.includes("expired") || msg.includes("sign in") || msg.includes("Sign in")) {
        onAuthChange(null);
      }
      toast.error(msg);
    } finally {
      setVotingPacks((prev) => {
        const next = new Set(prev);
        next.delete(packId);
        return next;
      });
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-2xl h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {selectedPack && (
              <Button variant="ghost" size="icon-sm" onClick={handleBack} className="mr-1">
                <ArrowLeftIcon className="size-4" />
              </Button>
            )}
            <PackageIcon className="size-4 text-[#c4a44a]" />
            {selectedPack ? decodeHtml(selectedPack.title) : "Pack Hub"}
          </DialogTitle>

          {/* Tab bar with animated pill indicator */}
          {!selectedPack &&
            (() => {
              const tabs: TabMode[] = ["browse", "my-packs", "create"];
              const tabCount = tabs.length;
              const tabIndex = tabs.indexOf(tab);
              const tabLabels: Record<TabMode, string> = {
                browse: "Browse",
                "my-packs": "My Packs",
                create: editingPackId ? "Edit Pack" : "Create",
              };
              return (
                <div className="relative flex mt-2 p-0.5 rounded-lg bg-white/[0.03] border border-white/[0.06] shadow-[inset_0_1px_2px_rgba(0,0,0,0.12)]">
                  <div
                    className="absolute top-0.5 bottom-0.5 rounded-md bg-white/[0.1] border border-white/[0.06] shadow-[0_1px_3px_rgba(0,0,0,0.2),inset_0_1px_0_rgba(255,255,255,0.06)] transition-[left] duration-200 ease-out"
                    style={{
                      left: `calc(${(tabIndex / tabCount) * 100}% + 2px)`,
                      width: `calc(${100 / tabCount}% - 4px)`,
                    }}
                  />
                  {tabs.map((t) => (
                    <button
                      key={t}
                      onClick={() => {
                        setTab(t);
                        if (duplicatingPackId) setDuplicatingPackId(null);
                      }}
                      className={cn(
                        "relative z-10 flex-1 px-3 py-1.5 rounded-md text-xs font-semibold transition-colors duration-200",
                        tab === t
                          ? "text-foreground"
                          : "text-muted-foreground/60 hover:text-muted-foreground"
                      )}
                    >
                      {tabLabels[t]}
                    </button>
                  ))}
                </div>
              );
            })()}
        </DialogHeader>

        <div className="flex-1 min-h-0 overflow-y-auto">
          {selectedPack ? (
            <PackDetailView
              key={selectedPack.id}
              pack={selectedPack}
              loading={loadingDetail}
              installing={installing}
              installProgress={installProgress}
              installSucceeded={installSucceeded}
              selectedAddons={selectedAddons}
              installedEsouiIds={installedEsouiIds}
              votingPacks={votingPacks}
              onToggleAddon={handleToggleAddon}
              onSelectAllOptional={(select) => {
                if (!selectedPack) return;
                setSelectedAddons((prev) => {
                  const next = new Set(prev);
                  for (const a of selectedPack.addons) {
                    if (!a.required) {
                      if (select) next.add(a.esouiId);
                      else next.delete(a.esouiId);
                    }
                  }
                  return next;
                });
              }}
              onVote={handleVote}
              authUser={authUser}
              canEdit={canEditSelectedPack}
              onEdit={() => selectedPack && handleStartEditing(selectedPack)}
              onDelete={() => selectedPack && handleDeletePack(selectedPack.id)}
              deletingPack={deletingPack}
              showShareSection={showShareSection}
              onToggleShare={() => {
                setShowShareSection((prev) => !prev);
                setShareResult(null);
                setCopiedField(null);
              }}
              shareResult={shareResult}
              generatingShare={generatingShare}
              copiedField={copiedField}
              onGenerateShareCode={() => selectedPack && handleGenerateShareCode(selectedPack)}
              onRegenerateShareCode={() => {
                setShareResult(null);
                if (selectedPack) handleGenerateShareCode(selectedPack);
              }}
              onCopyToClipboard={handleCopyToClipboard}
              onExportFile={() => selectedPack && handleExportPackFile(selectedPack)}
            />
          ) : (
            <AnimatePresence initial={false}>
              {tab === "browse" && (
                <motion.div
                  key="browse"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.1 }}
                  className="flex flex-col gap-3 min-h-0"
                >
                  {/* Import toggle button */}
                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => setShowImportPanel((prev) => !prev)}
                      className={cn(
                        "flex items-center gap-1.5 text-xs font-semibold px-2.5 py-1 rounded-md border transition-all duration-200",
                        showImportPanel
                          ? "text-[#c4a44a] border-[#c4a44a]/30 bg-[#c4a44a]/[0.08] shadow-[0_0_10px_rgba(196,164,74,0.08),inset_0_1px_0_rgba(196,164,74,0.06)]"
                          : "text-muted-foreground/50 border-white/[0.06] bg-white/[0.02] shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] hover:text-muted-foreground hover:border-white/[0.12] hover:bg-white/[0.04]"
                      )}
                    >
                      <ImportIcon className="size-3.5" />
                      Import
                    </button>
                    <div className="flex-1" />
                  </div>

                  {/* Collapsible import panel */}
                  <AnimatePresence>
                    {showImportPanel && (
                      <motion.div
                        key="import-panel"
                        initial={{ opacity: 0, y: -8 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: -8 }}
                        transition={{ type: "spring", stiffness: 400, damping: 30 }}
                        className="rounded-xl border border-white/[0.08] bg-[rgba(15,23,42,0.5)] backdrop-blur-md p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_4px_16px_rgba(0,0,0,0.15)]"
                      >
                        <PackImportView
                          key={importedPack ? "resolved" : "empty"}
                          shareCodeInput={shareCodeInput}
                          onShareCodeInputChange={setShareCodeInput}
                          resolvingCode={resolvingCode}
                          importedPack={importedPack}
                          importError={importError}
                          installing={installing}
                          installProgress={installProgress}
                          installedEsouiIds={installedEsouiIds}
                          importedPackAddonsToInstall={importedPackAddonsToInstall}
                          onResolveCode={handleResolveShareCode}
                          onImportFile={handleImportFile}
                          onInstall={handleInstallImportedPack}
                          onClear={() => {
                            setImportedPack(null);
                            setImportError(null);
                            setShareCodeInput("");
                          }}
                        />
                      </motion.div>
                    )}
                  </AnimatePresence>

                  {/* Pack list */}
                  <PackListView
                    packs={packs}
                    loading={loading}
                    loadingMore={loadingMore}
                    hasMore={hasMore}
                    error={error}
                    searchQuery={searchQuery}
                    onSearchChange={setSearchQuery}
                    typeFilter={typeFilter}
                    onTypeFilterChange={setTypeFilter}
                    sortMode={sortMode}
                    onSortChange={setSortMode}
                    onSelectPack={handleSelectPack}
                    onLoadMore={handleLoadMore}
                    onRetry={() => loadPacks(searchQuery, 1)}
                    onVote={handleVote}
                    votingPacks={votingPacks}
                    authUser={authUser}
                  />
                </motion.div>
              )}
              {tab === "create" && (
                <motion.div
                  key="create"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.1 }}
                >
                  <PackCreateView
                    installedAddons={installedAddons}
                    authUser={authUser}
                    onAuthChange={onAuthChange}
                    step={createStep}
                    onStepChange={setCreateStep}
                    title={createTitle}
                    onTitleChange={setCreateTitle}
                    description={createDescription}
                    onDescriptionChange={setCreateDescription}
                    packType={createPackType}
                    onPackTypeChange={setCreatePackType}
                    selectedTags={createTags}
                    onTagsChange={setCreateTags}
                    addons={createAddons}
                    onAddonsChange={setCreateAddons}
                    isAnonymous={createAnonymous}
                    onAnonymousChange={setCreateAnonymous}
                    editingPackId={editingPackId}
                    onPublished={(pack) => {
                      resetCreateForm();
                      setSelectedPack(pack);
                      setTab("browse");
                      loadPacks(searchQuery, 1);
                      if (authUser) loadMyPacks(1);
                    }}
                    onCancelEdit={
                      editingPackId
                        ? () => {
                            resetCreateForm();
                            setTab("browse");
                            loadPacks(searchQuery, 1);
                            if (authUser) loadMyPacks(1);
                          }
                        : undefined
                    }
                  />
                </motion.div>
              )}
              {tab === "my-packs" && (
                <motion.div
                  key="my-packs"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.1 }}
                >
                  <MyPacksView
                    packs={myPacks}
                    loading={myPacksLoading}
                    loadingMore={myPacksLoadingMore}
                    hasMore={myPacksHasMore}
                    authUser={authUser}
                    onAuthChange={onAuthChange}
                    onSelectPack={handleSelectPack}
                    onLoadMore={() => loadMyPacks(myPacksPage + 1)}
                    installedPackRefs={installedPackRefs}
                    onRemoveInstalledRef={(packId) => {
                      setInstalledPackRefs((prev) => {
                        const updated = prev.filter((r) => r.packId !== packId);
                        setSetting("installed_packs", updated);
                        return updated;
                      });
                    }}
                    onEdit={(pack) => {
                      handleStartEditing(pack);
                    }}
                    onDuplicate={(pack) => {
                      if (duplicatingPackId) return;
                      setDuplicatingPackId(pack.id);
                      setCreateTitle(`Copy of ${decodeHtml(pack.title)}`);
                      setCreateDescription(decodeHtml(pack.description));
                      setCreatePackType(pack.packType);
                      setCreateTags([...pack.tags]);
                      setCreateAddons(
                        pack.addons.map((a) => ({
                          esouiId: a.esouiId,
                          name: decodeHtml(a.name),
                          required: a.required,
                          note: a.note ? decodeHtml(a.note) : undefined,
                        }))
                      );
                      setCreateAnonymous(pack.isAnonymous);
                      setCreateStep("details");
                      setEditingPackId(null);
                      setTab("create");
                      setDuplicatingPackId(null);
                    }}
                    onDelete={handleDeletePack}
                    onCreatePack={() => {
                      resetCreateForm();
                      setTab("create");
                    }}
                    onPublish={async (pack) => {
                      try {
                        await invokeOrThrow<Pack>("update_pack", {
                          payload: {
                            id: pack.id,
                            title: pack.title,
                            description: pack.description,
                            packType: pack.packType,
                            addons: pack.addons,
                            tags: pack.tags,
                            isAnonymous: pack.isAnonymous,
                            status: "published",
                          },
                        });
                        toast.success("Pack published!");
                        loadMyPacks(1);
                        loadPacks(searchQuery, 1);
                      } catch (e) {
                        toast.error(`Publish failed: ${getTauriErrorMessage(e)}`);
                      }
                    }}
                  />
                </motion.div>
              )}
            </AnimatePresence>
          )}
        </div>

        <DialogFooter>
          {selectedPack ? (
            <>
              <Button variant="outline" onClick={handleBack}>
                Back
              </Button>
              {confirmInstall ? (
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground/60">
                    Install {newAddonsToInstall.length} new addon
                    {newAddonsToInstall.length !== 1 ? "s" : ""}?
                  </span>
                  <Button variant="outline" size="sm" onClick={() => setConfirmInstall(false)}>
                    Cancel
                  </Button>
                  <Button size="sm" onClick={handleInstallPack}>
                    Confirm
                  </Button>
                </div>
              ) : (
                <Button
                  onClick={() => setConfirmInstall(true)}
                  disabled={installing || newAddonsToInstall.length === 0}
                >
                  {installing ? (
                    <>
                      <Loader2Icon className="size-4 animate-spin mr-1.5" />
                      Installing...
                    </>
                  ) : (
                    <>
                      <DownloadIcon className="size-4 mr-1.5" />
                      {newAddonsToInstall.length === 0
                        ? "All Installed"
                        : `Install ${newAddonsToInstall.length} New Addon${newAddonsToInstall.length !== 1 ? "s" : ""}`}
                    </>
                  )}
                </Button>
              )}
            </>
          ) : (
            <Button variant="outline" onClick={onClose}>
              Close
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
