import { memo, useCallback, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type {
  AddonManifest,
  UpdateCheckResult,
  EsouiSearchResult,
  SortMode,
  FilterMode,
  ViewMode,
  DiscoverTab,
} from "../types";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { GlassPanel } from "@/components/ui/glass-panel";
import { Logo } from "@/components/ui/logo";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import {
  Search,
  Globe,
  X,
  RefreshCw,
  FolderOpen,
  ExternalLink,
  Copy,
  Star,
  Power,
  Trash2,
  CheckSquare,
  Square,
} from "lucide-react";
import { ContextMenu, type ContextMenuEntry } from "@/components/ui/context-menu";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsList, TabsIndicator, TabsTrigger } from "@/components/ui/tabs";
import { DiscoverPanel } from "@/components/discover-panel";
import { cn } from "@/lib/utils";
import { motion, AnimatePresence } from "motion/react";

interface AddonListProps {
  addons: AddonManifest[];
  allAddons: AddonManifest[];
  selectedAddon: AddonManifest | null;
  onSelect: (addon: AddonManifest) => void;
  searchQuery: string;
  onSearchChange: (query: string) => void;
  loading: boolean;
  updateResults: UpdateCheckResult[];
  sortMode: SortMode;
  onSortChange: (mode: SortMode) => void;
  filterMode: FilterMode;
  onFilterChange: (mode: FilterMode) => void;
  activeTagFilter: string | null;
  onActiveTagFilterChange: (tag: string | null) => void;
  selectedFolders: Set<string>;
  onToggleSelect: (folderName: string) => void;
  viewMode: ViewMode;
  onViewModeChange: (mode: ViewMode) => void;
  discoverTab: DiscoverTab;
  onDiscoverTabChange: (tab: DiscoverTab) => void;
  addonsPath: string;
  onInstalled: () => void;
  onSelectDiscoverResult: (result: EsouiSearchResult | null) => void;
  selectedDiscoverResultId: number | null;
  installedEsouiIds: Set<number>;
  isOffline?: boolean;
  onUpdateAddon?: (folderName: string) => void;
  onRemoveAddon?: (folderName: string) => void;
  onToggleDisable?: (folderName: string, disabled: boolean) => void;
  onOpenFolder?: (folderName: string) => void;
  onToggleFavorite?: (folderName: string, tags: string[]) => void;
}

interface AddonListItemProps {
  addon: AddonManifest;
  isCurrent: boolean;
  isSelected: boolean;
  batchMode: boolean;
  hasUpdate: boolean;
  onSelect: (addon: AddonManifest) => void;
  onToggleSelect: (folderName: string) => void;
  onRightClick: (addon: AddonManifest, pos: { x: number; y: number }) => void;
}

const AddonListItem = memo(function AddonListItem({
  addon,
  isCurrent,
  isSelected,
  batchMode,
  hasUpdate,
  onSelect,
  onToggleSelect,
  onRightClick,
}: AddonListItemProps) {
  return (
    <div
      id={`addon-${addon.folderName}`}
      role="option"
      aria-selected={batchMode ? isSelected : isCurrent}
      aria-label={`${addon.title}${addon.author ? `, by ${addon.author}` : ""}${addon.isLibrary ? ", Library" : ""}${hasUpdate ? ", Update available" : ""}${addon.disabled ? ", Disabled" : ""}${addon.missingDependencies.length > 0 ? `, ${addon.missingDependencies.length} missing dependencies` : ""}`}
      className={cn(
        "cursor-pointer border-l-3 border-l-transparent px-4 py-2.5 transition-all duration-200 ease-[cubic-bezier(0.4,0,0.2,1)] hover:bg-white/[0.04] hover:shadow-[inset_0_0_20px_rgba(196,164,74,0.02)] group",
        addon.disabled
          ? "border-l-zinc-500 opacity-50"
          : addon.missingDependencies.length > 0
            ? "border-l-red-500 shadow-[inset_4px_0_12px_-4px_rgba(239,68,68,0.1)]"
            : addon.isLibrary
              ? "border-l-emerald-400 shadow-[inset_4px_0_12px_-4px_rgba(52,211,153,0.08)]"
              : hasUpdate
                ? "border-l-amber-500 shadow-[inset_4px_0_12px_-4px_rgba(245,158,11,0.1)]"
                : "border-l-transparent",
        isCurrent &&
          !batchMode &&
          "bg-[#c4a44a]/[0.06] border-l-[#c4a44a]! shadow-[inset_4px_0_16px_-4px_rgba(196,164,74,0.15),inset_0_0_0_1px_rgba(196,164,74,0.08)]",
        isSelected && "bg-[#c4a44a]/[0.04] border-l-[#c4a44a]!"
      )}
      onClick={(e) => {
        if (e.ctrlKey || e.metaKey) {
          onToggleSelect(addon.folderName);
        } else {
          const rect = e.currentTarget.getBoundingClientRect();
          const localX = e.clientX - rect.left;
          if (localX < 40) {
            onToggleSelect(addon.folderName);
          } else {
            onSelect(addon);
          }
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onRightClick(addon, { x: e.clientX, y: e.clientY });
      }}
    >
      <div className="flex items-start gap-2">
        <div
          className={cn(
            "shrink-0 mt-0.5 transition-opacity duration-150",
            isSelected || batchMode ? "opacity-100" : "opacity-0 group-hover:opacity-70"
          )}
        >
          <Checkbox checked={isSelected} tabIndex={-1} className="pointer-events-none" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="truncate text-sm font-medium">
            {addon.tags.includes("favorite") && (
              <span className="text-[#c4a44a] mr-1">{"\u2605"}</span>
            )}
            {addon.isLibrary && (
              <span className="text-emerald-400 mr-1 text-[10px] font-medium uppercase tracking-wide">
                LIB
              </span>
            )}
            {addon.title}
          </div>
          <div className="mt-0.5 flex items-center gap-1.5">
            <span className="text-xs text-muted-foreground/50">
              {addon.version || `v${addon.addonVersion ?? "?"}`}
            </span>
            {addon.author && (
              <span className="text-xs text-muted-foreground/40">&middot; {addon.author}</span>
            )}
            <div className="flex-1" />
            {hasUpdate && (
              <Badge
                variant="outline"
                className="border-amber-400/20 bg-amber-400/[0.04] text-amber-400 text-[10px]"
              >
                Update
              </Badge>
            )}
            {addon.disabled && (
              <Badge
                variant="outline"
                className="border-zinc-400/20 bg-zinc-400/[0.04] text-zinc-400 text-[10px]"
              >
                Disabled
              </Badge>
            )}
            {addon.missingDependencies.length > 0 && (
              <Badge
                variant="outline"
                className="border-red-400/20 bg-red-400/[0.04] text-red-400 text-[10px]"
              >
                {addon.missingDependencies.length} missing
              </Badge>
            )}
            {addon.tags.includes("broken") && (
              <Badge
                variant="outline"
                className="border-red-400/20 bg-red-400/[0.04] text-red-400 text-[10px]"
              >
                Broken
              </Badge>
            )}
            {addon.tags.includes("testing") && (
              <Badge
                variant="outline"
                className="border-amber-400/20 bg-amber-400/[0.04] text-amber-400 text-[10px]"
              >
                Testing
              </Badge>
            )}
          </div>
        </div>
      </div>
    </div>
  );
});

const FILTERS: [FilterMode, string][] = [
  ["all", "All"],
  ["addons", "Addons"],
  ["libraries", "Libs"],
  ["favorites", "\u2605 Favorites"],
  ["outdated", "Outdated"],
  ["missing-deps", "Issues"],
  ["disabled", "Disabled"],
];

export function AddonList({
  addons,
  allAddons,
  selectedAddon,
  onSelect,
  searchQuery,
  onSearchChange,
  loading,
  updateResults,
  sortMode,
  onSortChange,
  filterMode,
  onFilterChange,
  activeTagFilter,
  onActiveTagFilterChange,
  selectedFolders,
  onToggleSelect,
  viewMode,
  onViewModeChange,
  discoverTab,
  onDiscoverTabChange,
  addonsPath,
  onInstalled,
  onSelectDiscoverResult,
  selectedDiscoverResultId,
  installedEsouiIds,
  isOffline,
  onUpdateAddon,
  onRemoveAddon,
  onToggleDisable,
  onOpenFolder,
  onToggleFavorite,
}: AddonListProps) {
  const updatesMap = useMemo(
    () => new Map(updateResults.filter((r) => r.hasUpdate).map((r) => [r.folderName, r] as const)),
    [updateResults]
  );

  const filterCounts = useMemo<Record<FilterMode, number>>(
    () => ({
      all: allAddons.length,
      addons: allAddons.filter((a) => !a.isLibrary).length,
      libraries: allAddons.filter((a) => a.isLibrary).length,
      favorites: allAddons.filter((a) => a.tags.includes("favorite")).length,
      outdated: allAddons.filter((a) => updatesMap.has(a.folderName)).length,
      "missing-deps": allAddons.filter((a) => a.missingDependencies.length > 0).length,
      disabled: allAddons.filter((a) => a.disabled).length,
    }),
    [allAddons, updatesMap]
  );

  // Collect all unique tags with counts — each becomes its own tab
  const tagCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const addon of allAddons) {
      for (const tag of addon.tags) {
        // "favorite" is already handled by the dedicated Favorites filter
        if (tag === "favorite") continue;
        counts.set(tag, (counts.get(tag) ?? 0) + 1);
      }
    }
    return counts;
  }, [allAddons]);

  const batchMode = selectedFolders.size > 0;

  const [ctxMenu, setCtxMenu] = useState<{
    addon: AddonManifest;
    pos: { x: number; y: number };
  } | null>(null);

  const handleRightClick = useCallback((addon: AddonManifest, pos: { x: number; y: number }) => {
    setCtxMenu({ addon, pos });
  }, []);

  const ctxMenuItems = useMemo<ContextMenuEntry[]>(() => {
    if (!ctxMenu) return [];
    const { addon } = ctxMenu;
    const hasUpdate = updatesMap.has(addon.folderName);
    const items: ContextMenuEntry[] = [];

    if (hasUpdate && onUpdateAddon) {
      items.push({
        label: "Update",
        icon: RefreshCw,
        onClick: () => onUpdateAddon(addon.folderName),
        disabled: isOffline,
      });
      items.push({ separator: true });
    }

    if (onOpenFolder) {
      items.push({
        label: "Open Folder",
        icon: FolderOpen,
        onClick: () => onOpenFolder(addon.folderName),
      });
    }

    if (addon.esouiId) {
      items.push({
        label: "View on ESOUI",
        icon: ExternalLink,
        onClick: () => {
          void import("@tauri-apps/plugin-opener").then((m) =>
            m.openUrl(`https://www.esoui.com/downloads/info${addon.esouiId}`)
          );
        },
      });
      items.push({
        label: "Copy ESOUI Link",
        icon: Copy,
        onClick: () => {
          void navigator.clipboard.writeText(
            `https://www.esoui.com/downloads/info${addon.esouiId}`
          );
        },
      });
    }

    items.push({ separator: true });

    if (onToggleFavorite) {
      const isFav = addon.tags.includes("favorite");
      items.push({
        label: isFav ? "Unfavorite" : "Favorite",
        icon: Star,
        onClick: () => {
          const next = isFav
            ? addon.tags.filter((t) => t !== "favorite")
            : [...addon.tags, "favorite"];
          onToggleFavorite(addon.folderName, next);
        },
      });
    }

    if (onToggleDisable) {
      items.push({
        label: addon.disabled ? "Enable" : "Disable",
        icon: Power,
        onClick: () => onToggleDisable(addon.folderName, addon.disabled),
      });
    }

    items.push({ separator: true });

    const addonSelected = selectedFolders.has(addon.folderName);
    items.push({
      label: addonSelected ? "Deselect" : "Select",
      icon: addonSelected ? CheckSquare : Square,
      onClick: () => onToggleSelect(addon.folderName),
    });

    if (onRemoveAddon) {
      items.push({ separator: true });
      items.push({
        label: "Remove",
        icon: Trash2,
        destructive: true,
        onClick: () => onRemoveAddon(addon.folderName),
      });
    }

    return items;
  }, [
    ctxMenu,
    updatesMap,
    isOffline,
    selectedFolders,
    onUpdateAddon,
    onOpenFolder,
    onToggleFavorite,
    onToggleDisable,
    onToggleSelect,
    onRemoveAddon,
  ]);

  const scrollContainerRef = useRef<HTMLDivElement>(null);

  const rowVirtualizer = useVirtualizer({
    count: addons.length,
    getScrollElement: () => scrollContainerRef.current,
    // 52px = 48px row content + 4px gap. Rows are single-line (title truncated
    // via CSS), so height is stable. measureElement corrects any deviation.
    estimateSize: () => 52,
    overscan: 10,
  });

  const handleListKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (addons.length === 0) return;

      const currentIndex = selectedAddon
        ? addons.findIndex((a) => a.folderName === selectedAddon.folderName)
        : -1;

      if (e.key === "ArrowDown") {
        e.preventDefault();
        const nextIndex = currentIndex < addons.length - 1 ? currentIndex + 1 : 0;
        onSelect(addons[nextIndex]);
        rowVirtualizer.scrollToIndex(nextIndex, { align: "auto" });
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        const prevIndex = currentIndex > 0 ? currentIndex - 1 : addons.length - 1;
        onSelect(addons[prevIndex]);
        rowVirtualizer.scrollToIndex(prevIndex, { align: "auto" });
      } else if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        if (currentIndex >= 0) {
          onSelect(addons[currentIndex]);
        }
      } else if (e.key === "Home") {
        e.preventDefault();
        onSelect(addons[0]);
        rowVirtualizer.scrollToIndex(0, { align: "start" });
      } else if (e.key === "End") {
        e.preventDefault();
        onSelect(addons[addons.length - 1]);
        rowVirtualizer.scrollToIndex(addons.length - 1, { align: "end" });
      }
    },
    [addons, selectedAddon, onSelect, rowVirtualizer]
  );

  return (
    <div className="flex min-h-0 w-[380px] min-w-[300px] flex-col border-r border-white/[0.06] bg-[rgba(10,18,36,0.6)] backdrop-blur-xl backdrop-saturate-[1.2] shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]">
      {/* Mode switcher */}
      <div className="px-3 pt-3 pb-2">
        <Tabs value={viewMode} onValueChange={(v) => onViewModeChange(v as ViewMode)}>
          <TabsList className="w-full bg-white/[0.04] border border-white/[0.06] [&_[data-slot=tabs-trigger]]:data-active:text-[#c4a44a]">
            <TabsIndicator className="bg-[#c4a44a]/[0.1] border-[#c4a44a]/20" />
            <TabsTrigger value="installed" className="flex-1">
              My Addons
            </TabsTrigger>
            <TabsTrigger value="discover" className="flex-1">
              Discover
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </div>

      <AnimatePresence mode="wait" initial={false}>
        {viewMode === "installed" ? (
          <motion.div
            key="installed"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.12 }}
            className="flex min-h-0 flex-1 flex-col"
          >
            {/* Search */}
            <div className="px-3 pb-2">
              <Input
                type="search"
                placeholder="Search addons..."
                aria-label="Search addons"
                value={searchQuery}
                onChange={(e) => onSearchChange(e.target.value)}
              />
            </div>

            {/* Filter tabs */}
            <div
              className="flex gap-1 px-3 pb-2 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
              role="tablist"
              aria-label="Filter addons"
            >
              {FILTERS.map(([mode, label]) => {
                const hideIfZero = ["outdated", "missing-deps", "favorites", "disabled"];
                if (hideIfZero.includes(mode) && filterCounts[mode] === 0) return null;
                const isActive = filterMode === mode && !activeTagFilter;
                return (
                  <button
                    key={mode}
                    role="tab"
                    aria-selected={isActive}
                    aria-label={`Filter by ${label}`}
                    className={cn(
                      "shrink-0 rounded-lg px-2.5 py-1 text-xs font-medium transition-all duration-150",
                      isActive
                        ? "bg-[#c4a44a]/15 text-[#c4a44a] shadow-[0_0_8px_rgba(196,164,74,0.1),inset_0_1px_0_rgba(255,255,255,0.05)] border border-[#c4a44a]/25"
                        : "text-muted-foreground/70 hover:text-foreground hover:bg-white/[0.05] border border-transparent"
                    )}
                    onClick={() => {
                      onFilterChange(mode);
                      onActiveTagFilterChange(null);
                    }}
                  >
                    {label}
                    <span className="ml-1 opacity-50">({filterCounts[mode]})</span>
                  </button>
                );
              })}

              {/* Dynamic tag tabs — one per tag in use */}
              {[...tagCounts.entries()]
                .sort(([a], [b]) => a.localeCompare(b))
                .map(([tag, count]) => {
                  const isActive = activeTagFilter === tag;
                  return (
                    <button
                      key={`tag:${tag}`}
                      role="tab"
                      aria-selected={isActive}
                      aria-label={`Filter by tag: ${tag}`}
                      className={cn(
                        "shrink-0 rounded-lg px-2.5 py-1 text-xs font-medium transition-all duration-150",
                        isActive
                          ? "bg-sky-500/15 text-sky-400 shadow-[0_0_8px_rgba(56,189,248,0.1),inset_0_1px_0_rgba(255,255,255,0.05)] border border-sky-500/25"
                          : "text-muted-foreground/70 hover:text-foreground hover:bg-white/[0.05] border border-transparent"
                      )}
                      onClick={() => {
                        onFilterChange("all");
                        onActiveTagFilterChange(tag);
                      }}
                    >
                      {tag}
                      <span className="ml-1 opacity-50">({count})</span>
                    </button>
                  );
                })}
            </div>

            {/* Sort + count bar */}
            <div className="flex items-center justify-between border-y border-white/[0.06] px-3 py-1.5">
              <span className="text-[11px] font-heading font-bold uppercase tracking-[0.05em] text-muted-foreground/50">
                {addons.length} {addons.length === 1 ? "addon" : "addons"}
                {batchMode && (
                  <span className="text-[#c4a44a] font-medium normal-case tracking-normal">
                    {" "}
                    &middot; {selectedFolders.size} selected
                  </span>
                )}
              </span>
              <Select value={sortMode} onValueChange={(v) => onSortChange(v as SortMode)}>
                <SelectTrigger
                  size="sm"
                  className="h-6 w-auto gap-1 border-0 bg-transparent text-[11px] text-muted-foreground/50 hover:text-muted-foreground px-1.5"
                  aria-label="Sort by"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="name">Name</SelectItem>
                  <SelectItem value="author">Author</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div
              ref={scrollContainerRef}
              role="listbox"
              aria-label="Installed addons"
              aria-roledescription="addon list"
              aria-rowcount={addons.length}
              aria-activedescendant={
                selectedAddon ? `addon-${selectedAddon.folderName}` : undefined
              }
              tabIndex={0}
              onKeyDown={handleListKeyDown}
              className="flex-1 overflow-y-auto focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50 focus-visible:ring-inset"
            >
              {loading ? (
                <div className="flex h-full items-center justify-center text-muted-foreground">
                  <div className="size-5 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
                </div>
              ) : addons.length === 0 ? (
                <div className="flex h-full flex-col items-center justify-center px-5">
                  {searchQuery || activeTagFilter || filterMode !== "all" ? (
                    <Fade className="flex flex-col items-center gap-3 text-center">
                      <div className="rounded-xl bg-white/[0.03] border border-white/[0.06] p-3">
                        <Search className="size-6 text-muted-foreground/30" />
                      </div>
                      <div>
                        <p className="text-sm text-muted-foreground">No addons match</p>
                        <p className="mt-0.5 text-xs text-muted-foreground/50">
                          {searchQuery
                            ? `"${searchQuery}"`
                            : activeTagFilter
                              ? `tag: ${activeTagFilter}`
                              : filterMode}
                        </p>
                      </div>
                      <button
                        className="rounded-lg border border-white/[0.08] bg-white/[0.04] px-3 py-1.5 text-xs font-medium text-muted-foreground hover:bg-white/[0.08] hover:text-foreground transition-colors"
                        onClick={() => {
                          onSearchChange("");
                          onFilterChange("all");
                          onActiveTagFilterChange(null);
                        }}
                      >
                        <X className="size-3 inline mr-1 -mt-px" />
                        Clear filters
                      </button>
                    </Fade>
                  ) : (
                    <Fade className="w-full">
                      <GlassPanel variant="subtle" className="relative p-5 overflow-hidden">
                        <div className="absolute -top-10 -right-10 h-[120px] w-[120px] rounded-full bg-[#c4a44a]/[0.03] blur-[40px]" />
                        <div className="flex flex-col items-center gap-4 relative">
                          <Logo size={36} className="opacity-60" />
                          <div className="text-center">
                            <p className="font-heading text-sm font-medium text-foreground/80">
                              No addons installed yet
                            </p>
                            <p className="mt-1 text-xs text-muted-foreground/50">
                              Get started with one of these options
                            </p>
                          </div>
                          <div className="w-full space-y-2">
                            <button
                              onClick={() => {
                                onViewModeChange("discover");
                                onDiscoverTabChange("search");
                              }}
                              className="flex w-full items-center gap-3 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2.5 text-left transition-colors hover:bg-white/[0.06] hover:border-white/[0.1] group"
                            >
                              <div className="flex size-7 items-center justify-center rounded-md bg-[#c4a44a]/10 text-[#c4a44a]">
                                <Globe className="size-3.5" />
                              </div>
                              <div className="flex-1 min-w-0">
                                <p className="text-xs font-medium text-foreground/80 group-hover:text-foreground">
                                  Browse the ESOUI catalog
                                </p>
                                <p className="text-[10px] text-muted-foreground/40">
                                  Search and install from thousands of addons
                                </p>
                              </div>
                            </button>
                            <button
                              onClick={() => {
                                onViewModeChange("discover");
                                onDiscoverTabChange("url");
                              }}
                              className="flex w-full items-center gap-3 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2.5 text-left transition-colors hover:bg-white/[0.06] hover:border-white/[0.1] group"
                            >
                              <div className="flex size-7 items-center justify-center rounded-md bg-sky-500/10 text-sky-400">
                                <Search className="size-3.5" />
                              </div>
                              <div className="flex-1 min-w-0">
                                <p className="text-xs font-medium text-foreground/80 group-hover:text-foreground">
                                  Paste an ESOUI URL
                                </p>
                                <p className="text-[10px] text-muted-foreground/40">
                                  Install directly from a link
                                </p>
                              </div>
                            </button>
                          </div>
                          <p className="text-[10px] text-muted-foreground/30 tabular-nums">
                            Ctrl+B to browse &middot; Ctrl+I to install by URL
                          </p>
                        </div>
                      </GlassPanel>
                    </Fade>
                  )}
                </div>
              ) : (
                <div
                  style={{
                    height: `${rowVirtualizer.getTotalSize()}px`,
                    width: "100%",
                    position: "relative",
                  }}
                >
                  {rowVirtualizer.getVirtualItems().map((virtualRow) => {
                    const addon = addons[virtualRow.index];
                    return (
                      <div
                        key={addon.folderName}
                        style={{
                          position: "absolute",
                          top: 0,
                          left: 0,
                          width: "100%",
                          transform: `translateY(${virtualRow.start}px)`,
                        }}
                        ref={rowVirtualizer.measureElement}
                        data-index={virtualRow.index}
                        aria-rowindex={virtualRow.index + 1}
                      >
                        <AddonListItem
                          addon={addon}
                          isCurrent={selectedAddon?.folderName === addon.folderName}
                          isSelected={selectedFolders.has(addon.folderName)}
                          batchMode={batchMode}
                          hasUpdate={updatesMap.has(addon.folderName)}
                          onSelect={onSelect}
                          onToggleSelect={onToggleSelect}
                          onRightClick={handleRightClick}
                        />
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          </motion.div>
        ) : (
          <motion.div
            key="discover"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.12 }}
            className="flex min-h-0 flex-1 flex-col"
          >
            <DiscoverPanel
              activeTab={discoverTab}
              onTabChange={onDiscoverTabChange}
              addonsPath={addonsPath}
              onInstalled={onInstalled}
              onSelectResult={onSelectDiscoverResult}
              selectedResultId={selectedDiscoverResultId}
              installedEsouiIds={installedEsouiIds}
              isOffline={isOffline}
            />
          </motion.div>
        )}
      </AnimatePresence>

      {ctxMenu && ctxMenuItems.length > 0 && (
        <ContextMenu items={ctxMenuItems} position={ctxMenu.pos} onClose={() => setCtxMenu(null)} />
      )}
    </div>
  );
}
