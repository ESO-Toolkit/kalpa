import { useState, useRef, useEffect, useCallback, useMemo, memo } from "react";
import { useVirtualizer, type Virtualizer } from "@tanstack/react-virtual";
import { toast } from "sonner";
import type {
  AddonSearchPage,
  AddonSearchSource,
  AskResponse,
  AskRecommendation,
  BrowsePopularPage,
  DiscoverTab,
  EsouiSearchResult,
  EsouiCategory,
  EsouiAddonInfo,
  InstallResult,
} from "../types";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { InfoPill } from "@/components/ui/info-pill";
import { GlassPanel } from "@/components/ui/glass-panel";
import { ProgressBar } from "@/components/ui/progress-bar";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { getTauriErrorMessage, invokeOrThrow, invokeResult } from "@/lib/tauri";
import { getDependencyPolicy } from "@/lib/dependency-policy";
import { reportDependencyFailures } from "@/lib/dependency-failure";
import { useResolvePendingDeps } from "@/lib/dependency-prompt-context";
import { useEnsureEsoNotBlocking } from "@/lib/eso-running-context";
import { cn } from "@/lib/utils";
import {
  Download,
  Clock,
  TrendingUp,
  Search,
  FolderOpen,
  Link,
  Flame,
  Check,
  WifiOff,
  Sparkles,
  ChevronRight,
} from "lucide-react";
import { useInfiniteScroll } from "@/lib/use-infinite-scroll";
import { useInstallProgress } from "@/hooks/use-install-progress";
import { formatInstallProgress, type InstallProgress } from "@/lib/install-progress";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { AskAnswerSkeleton, DiscoverResultListSkeleton } from "@/components/ui/skeletons";
import { motion, AnimatePresence } from "motion/react";

const PAGE_SIZE = 25;

interface DiscoverPanelProps {
  activeTab: DiscoverTab;
  onTabChange: (tab: DiscoverTab) => void;
  addonsPath: string;
  onInstalled: () => void;
  onSelectResult: (result: EsouiSearchResult | null) => void;
  selectedResultId: number | null;
  installedEsouiIds: Set<number>;
  isOffline?: boolean;
}

/**
 * Exported for tests: the install-then-uninstall regression below is a property
 * of this hook's state, and reaching it through the whole panel would mean
 * standing up the virtualizer and the ESOUI catalog for a badge.
 */
export function useAddonInstall(
  addonsPath: string,
  onInstalled: () => void,
  persistedIds: Set<number>
) {
  const ensureEsoNotBlocking = useEnsureEsoNotBlocking();
  const resolvePendingDeps = useResolvePendingDeps();
  const [installingId, setInstallingId] = useState<number | null>(null);
  const { progress, beginOperation, endOperation } = useInstallProgress();

  /**
   * Ids installed this session that the scan behind `persistedIds` has not
   * reported yet. Bridges install-success -> rescan-lands, and nothing more.
   */
  const [sessionInstalledIds, setSessionInstalledIds] = useState<Set<number>>(new Set());

  // A new `persistedIds` identity is App publishing a freshly scanned addon
  // list — a better answer than this overlay, so the overlay retires.
  //
  // Retiring the STATE is the point, not just ignoring it when merging. This
  // was a plain set that was only ever added to, so an addon installed and then
  // UNINSTALLED in the same session stayed badged as Installed for the rest of
  // the session; the only thing that cleared it was the tab switch that
  // unmounts this panel. Holding a snapshot of `persistedIds` from install time
  // and merging only while it still matches does NOT fix that: an overlay
  // recorded when nothing was installed matches the empty set again the moment
  // the addon is removed, and revives the badge it was supposed to drop.
  //
  // The cost is that an `addons` change from a non-scan source (a tag edit, a
  // disable toggle) also retires it, which can flash "Install" on an addon
  // whose scan has not landed. That needs the user to act on the installed list
  // during the few hundred ms after an install they started from Discover, and
  // a momentary understatement beats a badge that is wrong until restart.
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setSessionInstalledIds((prev) => (prev.size === 0 ? prev : new Set()));
  }, [persistedIds]);

  const installedIds = useMemo(() => {
    if (sessionInstalledIds.size === 0) return persistedIds;
    const merged = new Set(persistedIds);
    for (const id of sessionInstalledIds) merged.add(id);
    return merged;
  }, [persistedIds, sessionInstalledIds]);

  const install = useCallback(
    async (id: number) => {
      setInstallingId(id);
      if (!(await ensureEsoNotBlocking())) {
        setInstallingId(null);
        return;
      }
      try {
        const info = await invokeOrThrow<EsouiAddonInfo>("resolve_esoui_addon", {
          input: String(id),
        });
        const res = await invokeOrThrow<InstallResult>("install_addon", {
          addonsPath,
          downloadUrl: info.downloadUrl,
          esouiId: id,
          esouiTitle: info.title,
          esouiVersion: info.version,
          dependencyPolicy: await getDependencyPolicy(),
          // Correlates the download/extract/dependency events streamed back on
          // `update-progress` with this row.
          operationId: beginOperation(),
        });
        setSessionInstalledIds((prev) => new Set(prev).add(id));
        toast.success(`Installed ${res.installedFolders.join(", ")}`);
        reportDependencyFailures(res.failedDeps);
        onInstalled();
        // Empty unless the policy is "ask"; the app-level picker owns the rest.
        void resolvePendingDeps(res.pendingDeps, addonsPath);
      } catch (e) {
        toast.error(getTauriErrorMessage(e));
      } finally {
        setInstallingId(null);
        endOperation();
      }
    },
    [
      addonsPath,
      onInstalled,
      ensureEsoNotBlocking,
      resolvePendingDeps,
      beginOperation,
      endOperation,
    ]
  );

  return { installingId, installProgress: progress, installedIds, install };
}

const DiscoverResultRow = memo(function DiscoverResultRow({
  result,
  selected,
  isInstalling,
  progress,
  anyInstalling,
  installed,
  onSelect,
  onInstall,
  showMeta = false,
  rank,
}: {
  result: EsouiSearchResult;
  selected: boolean;
  isInstalling: boolean;
  /** Live phase/counts for THIS row's install; null until the first event. */
  progress: InstallProgress | null;
  anyInstalling: boolean;
  installed: boolean;
  onSelect: (result: EsouiSearchResult) => void;
  onInstall: (id: number) => void;
  showMeta?: boolean;
  rank?: number;
}) {
  const isInstalled = installed;
  // Until the first event lands the backend is still resolving the addon, so
  // there is genuinely nothing to measure — say so rather than show 0%.
  const progressLabel = progress ? formatInstallProgress(progress) : "Preparing…";

  return (
    <div
      className={cn(
        "cursor-pointer border-l-3 border-l-transparent px-4 py-2.5 transition-all duration-200 hover:bg-structure-04 group",
        selected &&
          "bg-primary/[0.06] border-l-primary! shadow-[inset_4px_0_16px_-4px_color-mix(in_oklab,var(--primary)_15%,transparent),inset_0_0_0_1px_color-mix(in_oklab,var(--primary)_8%,transparent)]"
      )}
      onClick={() => onSelect(result)}
    >
      <div className="flex items-center gap-2.5">
        {rank != null && (
          <span
            className={cn(
              "shrink-0 size-6 flex items-center justify-center rounded-md text-[11px] font-bold font-heading tabular-nums",
              rank <= 3
                ? "bg-primary/12 text-primary border border-primary/20"
                : "bg-structure-03 text-muted-foreground border border-structure-06"
            )}
          >
            {rank}
          </span>
        )}
        <span className="flex-1 truncate text-sm font-medium">{result.title}</span>
        <Button
          size="xs"
          variant={isInstalled ? "ghost" : "default"}
          onClick={(e) => {
            e.stopPropagation();
            onInstall(result.id);
          }}
          disabled={anyInstalling}
          className={cn(
            "shrink-0 transition-all",
            isInstalling || isInstalled ? "opacity-100" : "opacity-0 group-hover:opacity-100"
          )}
        >
          {isInstalling ? (
            <span className="flex items-center gap-1">
              <span className="inline-block size-3 animate-spin rounded-full border-2 border-[var(--primary-foreground)]/20 border-t-[var(--primary-foreground)]" />
              Installing
            </span>
          ) : isInstalled ? (
            <span className="flex items-center gap-1 text-status-success">
              <Check className="size-3" />
              Installed
            </span>
          ) : (
            "Install"
          )}
        </Button>
      </div>
      {isInstalling && (
        <div className="mt-2 space-y-1">
          <ProgressBar
            value={progress?.done ?? 0}
            max={progress?.determinate ? progress.total : 100}
            indeterminate={!progress?.determinate}
            label={`${result.title}: ${progressLabel}`}
            className="h-1.5"
          />
          <div className="text-[11px] tabular-nums text-muted-foreground">{progressLabel}</div>
        </div>
      )}
      <div className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
        {result.author && <span className="truncate">by {result.author}</span>}
        {result.category && <InfoPill color="muted">{result.category}</InfoPill>}
      </div>
      {showMeta && (
        <div className="mt-1.5 flex items-center gap-3 text-xs text-muted-foreground">
          {result.downloads && (
            <span className="flex items-center gap-1">
              <Download className="size-3" />
              {result.downloads}
            </span>
          )}
          {result.updated && (
            <span className="flex items-center gap-1">
              <Clock className="size-3" />
              {result.updated}
            </span>
          )}
        </div>
      )}
    </div>
  );
});

// Shared virtualized body for the Search / Popular / Category result lists. Only
// the visible window of rows is mounted (mirrors the installed-addon list's
// @tanstack/react-virtual usage) so deep infinite-scroll browsing no longer keeps
// 1000+ heavy DOM rows alive. The parent owns the scroll container + sentinel; this
// renders the sized spacer and the absolutely-positioned visible rows.
function VirtualResultRows({
  rowVirtualizer,
  results,
  selectedResultId,
  installingId,
  installProgress,
  installedIds,
  onSelectResult,
  onInstall,
  showRank = false,
}: {
  rowVirtualizer: Virtualizer<HTMLDivElement, Element>;
  results: EsouiSearchResult[];
  selectedResultId: number | null;
  installingId: number | null;
  installProgress: InstallProgress | null;
  installedIds: Set<number>;
  onSelectResult: (result: EsouiSearchResult | null) => void;
  onInstall: (id: number) => void;
  showRank?: boolean;
}) {
  return (
    <div
      style={{
        height: `${rowVirtualizer.getTotalSize()}px`,
        width: "100%",
        position: "relative",
      }}
    >
      {rowVirtualizer.getVirtualItems().map((virtualRow) => {
        const r = results[virtualRow.index];
        if (!r) return null;
        return (
          <div
            key={r.id}
            data-result-row
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              transform: `translateY(${virtualRow.start}px)`,
            }}
            ref={rowVirtualizer.measureElement}
            data-index={virtualRow.index}
          >
            <DiscoverResultRow
              result={r}
              selected={selectedResultId === r.id}
              isInstalling={installingId === r.id}
              progress={installingId === r.id ? installProgress : null}
              anyInstalling={installingId !== null}
              installed={installedIds.has(r.id)}
              onSelect={onSelectResult}
              onInstall={onInstall}
              showMeta
              rank={showRank ? virtualRow.index + 1 : undefined}
            />
          </div>
        );
      })}
    </div>
  );
}

const DISCOVER_TABS: [DiscoverTab, string, React.FC<{ className?: string }>][] = [
  ["search", "Search", Search],
  ["popular", "Popular", Flame],
  ["categories", "Categories", FolderOpen],
  ["url", "URL / ID", Link],
];

export function DiscoverPanel({
  activeTab,
  onTabChange,
  addonsPath,
  onInstalled,
  onSelectResult,
  selectedResultId,
  installedEsouiIds,
  isOffline,
}: DiscoverPanelProps) {
  const {
    installingId,
    installProgress,
    installedIds,
    install: handleInstall,
  } = useAddonInstall(addonsPath, onInstalled, installedEsouiIds);

  if (isOffline) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        {/* Sub-tab selector (disabled) */}
        <div className="@container flex gap-1 px-3 pb-2" role="tablist" aria-label="Discover mode">
          {DISCOVER_TABS.map(([tab, label, Icon]) => (
            <button
              key={tab}
              role="tab"
              aria-selected={false}
              disabled
              title={label}
              aria-label={label}
              className="flex-1 min-w-0 rounded-lg px-2 py-1 text-xs font-medium flex items-center justify-center gap-1 text-muted-foreground border border-transparent cursor-not-allowed"
            >
              <Icon className="size-3 shrink-0" />
              <span className="truncate hidden @md:inline">{label}</span>
            </button>
          ))}
        </div>
        <EmptyState
          icon={<WifiOff className="size-8 text-muted-foreground/20" />}
          title="You're offline"
          subtitle="Discovery, search, and installs require an internet connection. Reconnect to browse addons."
        />
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Sub-tab selector */}
      {/* The tab labels never fit side by side in a 300-380px panel — they
          alone need well over the panel's width, so every one truncated to an
          ellipsis. Only the ACTIVE tab shows its label; the rest are icons with
          tooltips, which fits comfortably and still names where you are. A wide
          enough container (a future resizable panel) shows every label again. */}
      <div className="@container flex gap-1 px-3 pb-2" role="tablist" aria-label="Discover mode">
        {DISCOVER_TABS.map(([tab, label, Icon]) => (
          <button
            key={tab}
            role="tab"
            aria-selected={activeTab === tab}
            title={label}
            aria-label={label}
            className={cn(
              "relative min-w-0 rounded-lg px-2 py-1 text-xs font-medium transition-colors duration-150 flex items-center justify-center gap-1",
              // The active tab earns the room for its label; the others stay
              // icon-sized so nothing has to truncate.
              activeTab === tab ? "flex-1" : "flex-none @md:flex-1",
              activeTab === tab
                ? "text-primary"
                : "text-muted-foreground hover:text-foreground hover:bg-structure-05 border border-transparent"
            )}
            onClick={() => onTabChange(tab)}
          >
            {activeTab === tab && (
              <motion.span
                layoutId="discover-tab-indicator"
                className="absolute inset-0 rounded-lg bg-primary/15 border border-primary/25 shadow-[0_0_8px_color-mix(in_oklab,var(--primary)_10%,transparent),inset_0_1px_0_var(--structure-05)]"
                transition={{ type: "spring", stiffness: 400, damping: 30 }}
              />
            )}
            <span className="relative z-10 flex min-w-0 items-center justify-center gap-1">
              <Icon className="size-3 shrink-0" />
              <span className={cn("truncate", activeTab === tab ? "inline" : "hidden @md:inline")}>
                {label}
              </span>
            </span>
          </button>
        ))}
      </div>

      <AnimatePresence mode="wait">
        {activeTab === "search" && (
          <motion.div
            key="search"
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={{ duration: 0.08 }}
            className="flex min-h-0 flex-1 flex-col"
          >
            <SearchContent
              installingId={installingId}
              installProgress={installProgress}
              installedIds={installedIds}
              onInstall={handleInstall}
              onSelectResult={onSelectResult}
              selectedResultId={selectedResultId}
            />
          </motion.div>
        )}
        {activeTab === "popular" && (
          <motion.div
            key="popular"
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={{ duration: 0.08 }}
            className="flex min-h-0 flex-1 flex-col"
          >
            <PopularContent
              installingId={installingId}
              installProgress={installProgress}
              installedIds={installedIds}
              onInstall={handleInstall}
              onSelectResult={onSelectResult}
              selectedResultId={selectedResultId}
            />
          </motion.div>
        )}
        {activeTab === "categories" && (
          <motion.div
            key="categories"
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={{ duration: 0.08 }}
            className="flex min-h-0 flex-1 flex-col"
          >
            <CategoryContent
              installingId={installingId}
              installProgress={installProgress}
              installedIds={installedIds}
              onInstall={handleInstall}
              onSelectResult={onSelectResult}
              selectedResultId={selectedResultId}
            />
          </motion.div>
        )}
        {activeTab === "url" && (
          <motion.div
            key="url"
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={{ duration: 0.08 }}
            className="flex min-h-0 flex-1 flex-col"
          >
            <UrlContent
              addonsPath={addonsPath}
              onInstalled={onInstalled}
              installedEsouiIds={installedEsouiIds}
            />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* ── Search Tab (search + ask) ────────────────────────── */

/**
 * Shown in the empty state rather than the placeholder.
 *
 * The panel is ~380px, so any placeholder long enough to carry a real example
 * gets truncated mid-word — and the example is exactly the half that gets cut.
 * Here they wrap, stay fully readable, and are clickable, so discovering that
 * the box also takes a question costs no typing.
 */
const ASK_EXAMPLES = [
  "an addon that shows when I'm in combat",
  "something to manage my inventory and bank",
  "how do I track my dps",
];

/**
 * One box for both retrieval paths.
 *
 * Search and Ask used to be separate tabs, but they run the SAME retrieval —
 * the worker's /ask does a `limit: 20` search of the very same index before it
 * shows anything to a model. The split only made the user guess: a question
 * typed into Search got no answer, and keywords typed into Ask burned a model
 * call to rank what a free search would have ranked.
 *
 * So typing is always the free path (debounced `search_addon_index`), and the
 * assistant is an explicit act — the Ask button, or Shift+Enter. Its answer
 * stacks ABOVE the result list rather than replacing it, which is also what
 * makes a degraded assistant a non-event: the search results the user would
 * have got anyway are still sitting right underneath it.
 */
function SearchContent({
  installingId,
  installProgress,
  installedIds,
  onInstall,
  onSelectResult,
  selectedResultId,
}: {
  installingId: number | null;
  installProgress: InstallProgress | null;
  installedIds: Set<number>;
  onInstall: (id: number) => void;
  onSelectResult: (result: EsouiSearchResult | null) => void;
  selectedResultId: number | null;
}) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<EsouiSearchResult[]>([]);
  const [searchSource, setSearchSource] = useState<AddonSearchSource | null>(null);
  const [searching, setSearching] = useState(false);
  const [askResponse, setAskResponse] = useState<AskResponse | null>(null);
  const [asking, setAsking] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const searchIdRef = useRef(0);
  const askIdRef = useRef(0);
  const listRef = useRef<HTMLDivElement>(null);

  // eslint-disable-next-line react-hooks/incompatible-library
  const rowVirtualizer = useVirtualizer({
    count: results.length,
    getScrollElement: () => listRef.current,
    // Multi-line result rows (title + author/category + meta). measureElement
    // corrects the estimate once each row mounts.
    estimateSize: () => 84,
    overscan: 8,
  });

  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, []);

  const handleSearch = useCallback(async (searchQuery: string) => {
    if (!searchQuery.trim()) {
      setResults([]);
      setSearchSource(null);
      return;
    }
    setSearching(true);
    const id = ++searchIdRef.current;
    try {
      // Full-text search over titles AND descriptions, served by the Pack Hub
      // worker's addon index. The command falls back to the ESOUI scraper on
      // its own when the index cannot answer, so this never regresses to a
      // dead search — `source` just reports which backend replied.
      const page = await invokeOrThrow<AddonSearchPage>("search_addon_index", {
        query: searchQuery.trim(),
      });
      if (searchIdRef.current === id) {
        setResults(page.results);
        setSearchSource(page.source);
      }
    } catch (e) {
      if (searchIdRef.current === id) toast.error(getTauriErrorMessage(e));
    } finally {
      if (searchIdRef.current === id) setSearching(false);
    }
  }, []);

  const handleAsk = useCallback(async (raw: string) => {
    const trimmed = raw.trim();
    if (!trimmed) return;
    setAsking(true);
    const id = ++askIdRef.current;
    try {
      const result = await invokeOrThrow<AskResponse>("ask_addon_assistant", {
        question: trimmed,
      });
      if (askIdRef.current === id) setAskResponse(result);
    } catch (e) {
      if (askIdRef.current === id) {
        toast.error(getTauriErrorMessage(e));
        setAskResponse(null);
      }
    } finally {
      if (askIdRef.current === id) setAsking(false);
    }
  }, []);

  /** Retires an answer (and any in-flight one) that no longer matches the box. */
  const dismissAsk = useCallback(() => {
    askIdRef.current++;
    setAsking(false);
    setAskResponse(null);
  }, []);

  const handleInputChange = (value: string) => {
    setQuery(value);
    // An answer to the previous wording is worse than no answer, so editing the
    // box drops it rather than leaving it stranded above fresh results.
    if (askResponse || asking) dismissAsk();
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => handleSearch(value), 500);
  };

  const runExample = (example: string) => {
    setQuery(example);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    void handleSearch(example);
    void handleAsk(example);
  };

  // Keyboard navigation
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (results.length === 0) return;
      const currentIdx = results.findIndex((r) => r.id === selectedResultId);

      if (e.key === "ArrowDown") {
        e.preventDefault();
        const next = currentIdx < results.length - 1 ? currentIdx + 1 : 0;
        onSelectResult(results[next] ?? null);
        // Virtualized: scroll the (possibly unmounted) target into view via the
        // virtualizer rather than querying the DOM for a row that may not exist.
        rowVirtualizer.scrollToIndex(next, { align: "auto" });
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        const prev = currentIdx > 0 ? currentIdx - 1 : results.length - 1;
        onSelectResult(results[prev] ?? null);
        rowVirtualizer.scrollToIndex(prev, { align: "auto" });
      }
    },
    [results, selectedResultId, onSelectResult, rowVirtualizer]
  );

  const canAsk = query.trim().length > 0 && !asking;

  return (
    <>
      <div className="flex items-center gap-1.5 px-3 pb-2">
        <Input
          placeholder="Search or ask about addons…"
          aria-label="Search or ask about addons"
          value={query}
          onChange={(e) => handleInputChange(e.target.value)}
          onKeyDown={(e) => {
            // Shift+Enter is the assistant; plain Enter stays the free search.
            if (e.key === "Enter" && e.shiftKey) {
              e.preventDefault();
              if (canAsk) void handleAsk(query);
              return;
            }
            if (e.key === "Enter") void handleSearch(query);
            handleKeyDown(e);
          }}
          className="min-w-0 flex-1"
          autoFocus
        />
        <Button
          variant="outline"
          onClick={() => handleAsk(query)}
          disabled={!canAsk}
          title="Ask the assistant (Shift+Enter)"
          aria-label="Ask the assistant"
        >
          <Sparkles className="size-3.5" />
          Ask
        </Button>
      </div>

      {/* The assistant's answer sits ABOVE the results it was drawn from, and is
          capped so it can never push the result list off a 300px panel. */}
      {(asking || askResponse) && (
        <div className="flex max-h-[45%] shrink-0 flex-col overflow-hidden border-b border-structure-06 pb-2">
          <div className="flex items-center justify-between px-3 pb-1.5">
            <span className="text-[11px] font-heading font-bold uppercase tracking-[0.05em] text-muted-foreground">
              Assistant
            </span>
            {!asking && (
              <button
                type="button"
                onClick={dismissAsk}
                className="rounded px-1 text-xs text-muted-foreground transition-colors duration-150 hover:text-foreground"
              >
                Dismiss
              </button>
            )}
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto px-3">
            {asking ? (
              /* A centred spinner sat in the middle of an empty column and then
                 the answer appeared at the top — the whole panel jumped. The
                 skeleton occupies the same shape the cards will. */
              <AskAnswerSkeleton />
            ) : askResponse ? (
              <AskAnswer
                response={askResponse}
                onSelectResult={onSelectResult}
                selectedResultId={selectedResultId}
              />
            ) : null}
          </div>
        </div>
      )}

      {/* Results count bar */}
      {results.length > 0 && (
        <div className="flex items-center justify-between px-3 pt-1.5 pb-1.5">
          <span className="text-[11px] font-heading font-bold uppercase tracking-[0.05em] text-muted-foreground">
            {results.length} result{results.length !== 1 ? "s" : ""}
          </span>
          <span className="text-xs text-muted-foreground">&uarr;&darr; to navigate</span>
        </div>
      )}

      {/* The index searches descriptions; the scraper only matches titles. Say
          so when we fall back, otherwise a thinner result set looks like a bug. */}
      {results.length > 0 && searchSource === "esoui" && (
        <div className="px-3 pb-1.5">
          <span className="text-xs text-muted-foreground">
            Matching addon names only &mdash; description search is unavailable right now.
          </span>
        </div>
      )}

      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto">
        {searching ? (
          <DiscoverResultListSkeleton />
        ) : results.length === 0 && query.trim() ? (
          <EmptyState
            icon={<Search className="size-8 text-muted-foreground" />}
            title="No results found"
            subtitle={`Try different keywords for "${query}"`}
          />
        ) : results.length === 0 ? (
          <EmptyState
            icon={<Search className="size-8 text-muted-foreground/20" />}
            title="Search ESOUI"
            subtitleClassName="mt-2 w-full max-w-[260px]"
            subtitle={
              <span className="flex flex-col items-center gap-2">
                <span>Keywords search names and descriptions. For a question, press Ask. Try:</span>
                <span className="flex w-full flex-col items-stretch gap-1.5">
                  {ASK_EXAMPLES.map((example) => (
                    <button
                      key={example}
                      type="button"
                      onClick={() => runExample(example)}
                      className="rounded-lg border border-structure-06 px-2.5 py-1.5 text-left text-xs leading-snug text-foreground transition-colors duration-150 hover:border-primary/25 hover:bg-primary/[0.06]"
                    >
                      &ldquo;{example}&rdquo;
                    </button>
                  ))}
                </span>
              </span>
            }
          />
        ) : (
          <VirtualResultRows
            rowVirtualizer={rowVirtualizer}
            results={results}
            selectedResultId={selectedResultId}
            installingId={installingId}
            installProgress={installProgress}
            installedIds={installedIds}
            onSelectResult={onSelectResult}
            onInstall={onInstall}
          />
        )}
      </div>
    </>
  );
}

/**
 * The assistant's grounded answer, rendered above the plain search results.
 *
 * The worker does the retrieval and the grounding; every recommendation here
 * corresponds to a real indexed addon, and clicking one opens the same
 * DiscoverDetail pane (and Install button) the result rows use.
 */
function AskAnswer({
  response,
  onSelectResult,
  selectedResultId,
}: {
  response: AskResponse;
  onSelectResult: (result: EsouiSearchResult | null) => void;
  selectedResultId: number | null;
}) {
  // Only the ESOUI id is load-bearing: DiscoverDetail fetches everything else
  // itself, so a recommendation can open the full detail pane directly.
  const selectRecommendation = (rec: AskRecommendation) => {
    onSelectResult({
      id: rec.esoui_id,
      title: rec.title,
      author: rec.author,
      category: rec.category,
      downloads: "",
      updated: "",
    });
  };

  return (
    <div className="flex flex-col gap-2">
      {/* The per-addon reasons carry the useful information. A summary
          paragraph on top of them just restated the question back at the
          user, so it is shown only when there is nothing to recommend
          and the sentence has to do the whole job. */}
      {response.answer && response.no_good_match && (
        <GlassPanel variant="subtle" className="p-3">
          <p className="text-sm leading-relaxed text-foreground">{response.answer}</p>
        </GlassPanel>
      )}

      {/* Say plainly when the assistant itself did not run, rather than passing
          off raw search hits as an answer. It is a soft failure now — the plain
          search results are already on screen just below this block. */}
      {response.degraded && (
        <p className="text-xs text-muted-foreground">
          {response.recommendations.length > 0
            ? "The assistant is unavailable right now — showing the closest matches instead. Your search results below are unaffected."
            : "The assistant is unavailable right now. Your search results below are unaffected."}
        </p>
      )}

      {!response.degraded && response.no_good_match && response.recommendations.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No indexed addon looks like a good fit. Try describing it differently, or scan the search
          results below.
        </p>
      )}

      {response.recommendations.map((rec) => (
        <button
          key={rec.esoui_id}
          onClick={() => selectRecommendation(rec)}
          className={cn(
            "w-full rounded-lg border p-2.5 text-left transition-colors duration-150",
            selectedResultId === rec.esoui_id
              ? "border-primary/25 bg-primary/[0.06]"
              : "border-structure-06 hover:bg-structure-05"
          )}
        >
          {/* The pill used to share a row with the title and would wrap
              to two lines ("Graphic UI / Mods") whenever the title was
              long. The title now owns the row and truncates; the category
              sits with the reason, where it never competes for width. */}
          <div className="min-w-0">
            <span className="block truncate font-heading text-sm font-medium text-foreground">
              {rec.title}
            </span>
          </div>
          {rec.reason && (
            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{rec.reason}</p>
          )}
          {rec.category && (
            <InfoPill color="muted" className="mt-1.5 max-w-full whitespace-nowrap">
              <span className="truncate">{rec.category}</span>
            </InfoPill>
          )}
        </button>
      ))}

      {/* The assistant answers with a few picks, but retrieval found more.
          Showing the rest collapsed means a short answer never looks like
          it missed something — and it costs no extra model call. */}
      {response.also_considered.length > 0 && (
        <details className="group mt-1">
          <summary className="cursor-pointer list-none rounded-lg px-1 py-1 text-xs text-muted-foreground transition-colors duration-150 hover:text-foreground">
            <span className="inline-flex items-center gap-1">
              <ChevronRight className="size-3 shrink-0 transition-transform duration-150 group-open:rotate-90" />
              {response.also_considered.length} more{" "}
              {response.also_considered.length === 1 ? "match" : "matches"}
            </span>
          </summary>
          <div className="mt-1.5 flex flex-col gap-1">
            {response.also_considered.map((rec) => (
              <button
                key={rec.esoui_id}
                onClick={() => selectRecommendation(rec)}
                className={cn(
                  "w-full rounded-lg border px-2.5 py-1.5 text-left transition-colors duration-150",
                  selectedResultId === rec.esoui_id
                    ? "border-primary/25 bg-primary/[0.06]"
                    : "border-structure-06 hover:bg-structure-05"
                )}
              >
                {/* The tail is no longer relevance-capped, so it can trail
                    weak matches on a vague question. Bare titles gave no cue
                    which rows those were; the category is the cheapest signal
                    that "Deconstruction Junk Marker" is not a combat addon.

                    Inline rather than stacked, because this list is long and
                    its container is short. A second line took tail rows from
                    28px to 54px, and at 23 rows that is 1249px of scroll
                    inside a 189px region — expanding the disclosure pushed the
                    answer and the picks off screen entirely. */}
                <span className="flex items-baseline gap-2">
                  <span className="min-w-0 flex-1 truncate text-xs text-foreground">
                    {rec.title}
                  </span>
                  {rec.category && (
                    <InfoPill color="muted" className="shrink-0 whitespace-nowrap">
                      {rec.category}
                    </InfoPill>
                  )}
                </span>
              </button>
            ))}
          </div>
        </details>
      )}
    </div>
  );
}

/* ── Popular Tab ─────────────────────────────────────── */

type PopularSort = "downloads" | "newest";

function PopularContent({
  installingId,
  installProgress,
  installedIds,
  onInstall,
  onSelectResult,
  selectedResultId,
}: {
  installingId: number | null;
  installProgress: InstallProgress | null;
  installedIds: Set<number>;
  onInstall: (id: number) => void;
  onSelectResult: (result: EsouiSearchResult | null) => void;
  selectedResultId: number | null;
}) {
  const [sortBy, setSortBy] = useState<PopularSort>("downloads");
  const [results, setResults] = useState<EsouiSearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const pageRef = useRef(0);
  const loadSeqRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);

  // eslint-disable-next-line react-hooks/incompatible-library
  const rowVirtualizer = useVirtualizer({
    count: results.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 84,
    overscan: 8,
  });

  const loadPage = useCallback(async (p: number, sort: PopularSort, append: boolean) => {
    // A sort switch (or a loadMore overtaken by one) must not have its late
    // response applied over the newer list — same seq guard the search tab uses.
    const seq = ++loadSeqRef.current;
    setLoading(true);
    try {
      const page = await invokeOrThrow<BrowsePopularPage>("browse_esoui_popular", {
        page: p,
        sortBy: sort,
      });
      if (seq !== loadSeqRef.current) return;
      setResults((prev) => (append ? [...prev, ...page.results] : page.results));
      setHasMore(page.hasMore);
      pageRef.current = p;
    } catch (e) {
      if (seq !== loadSeqRef.current) return;
      toast.error(getTauriErrorMessage(e));
    } finally {
      if (seq === loadSeqRef.current) setLoading(false);
    }
  }, []);

  // Load on mount
  useEffect(() => {
    loadPage(0, "downloads", false);
  }, [loadPage]);

  const loadMore = useCallback(() => {
    const next = pageRef.current + 1;
    loadPage(next, sortBy, true);
  }, [sortBy, loadPage]);

  const sentinelRef = useInfiniteScroll(loadMore, { hasMore, isLoading: loading });

  const handleSortChange = (sort: string | null) => {
    if (!sort) return;
    setSortBy(sort as PopularSort);
    setHasMore(true);
    pageRef.current = 0;
    onSelectResult(null);
    loadPage(0, sort as PopularSort, false);
  };

  return (
    <>
      <div className="px-3 pb-2">
        <div className="flex gap-1.5">
          <button
            className={cn(
              "flex-1 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-all duration-150 flex items-center justify-center gap-1.5",
              sortBy === "downloads"
                ? "bg-primary/15 text-primary border border-primary/25"
                : "text-muted-foreground hover:text-foreground hover:bg-structure-05 border border-structure-06"
            )}
            onClick={() => handleSortChange("downloads")}
          >
            <TrendingUp className="size-3" />
            Most Popular
          </button>
          <button
            className={cn(
              "flex-1 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-all duration-150 flex items-center justify-center gap-1.5",
              sortBy === "newest"
                ? "bg-primary/15 text-primary border border-primary/25"
                : "text-muted-foreground hover:text-foreground hover:bg-structure-05 border border-structure-06"
            )}
            onClick={() => handleSortChange("newest")}
          >
            <Clock className="size-3" />
            Recently Updated
          </button>
        </div>
      </div>

      <div ref={scrollRef} className="flex-1 overflow-y-auto">
        {results.length === 0 && loading ? (
          <DiscoverResultListSkeleton />
        ) : results.length === 0 ? (
          <EmptyState
            icon={<Flame className="size-8 text-muted-foreground/20" />}
            title="No addons found"
            subtitle="Could not load popular addons"
          />
        ) : (
          <>
            <VirtualResultRows
              rowVirtualizer={rowVirtualizer}
              results={results}
              selectedResultId={selectedResultId}
              installingId={installingId}
              installProgress={installProgress}
              installedIds={installedIds}
              onSelectResult={onSelectResult}
              onInstall={onInstall}
              showRank
            />
            {hasMore && <div ref={sentinelRef} className="h-1" />}
            {loading && <DiscoverResultListSkeleton count={3} />}
          </>
        )}
      </div>
    </>
  );
}

/* ── Categories Tab ───────────────────────────────────── */

function CategoryContent({
  installingId,
  installProgress,
  installedIds,
  onInstall,
  onSelectResult,
  selectedResultId,
}: {
  installingId: number | null;
  installProgress: InstallProgress | null;
  installedIds: Set<number>;
  onInstall: (id: number) => void;
  onSelectResult: (result: EsouiSearchResult | null) => void;
  selectedResultId: number | null;
}) {
  const [categories, setCategories] = useState<EsouiCategory[]>([]);
  const [selectedCategory, setSelectedCategory] = useState<number | null>(null);
  const [sortBy, setSortBy] = useState("downloads");
  const [results, setResults] = useState<EsouiSearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [filterText, setFilterText] = useState("");
  const pageRef = useRef(0);
  const loadSeqRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void invokeResult<EsouiCategory[]>("get_esoui_categories").then((result) => {
      if (result.ok) {
        setCategories(result.data);
      } else {
        toast.error(`Failed to load categories: ${result.error}`);
      }
    });
  }, []);

  const loadPage = useCallback(async (catId: number, p: number, sort: string, append: boolean) => {
    // A category/sort switch (or a loadMore overtaken by one) must not have its
    // late response applied over the newer list, and must not leave pageRef
    // pointing at the previous category's page — same seq guard the search tab
    // uses.
    const seq = ++loadSeqRef.current;
    setLoading(true);
    try {
      const r = await invokeOrThrow<EsouiSearchResult[]>("browse_esoui_category", {
        categoryId: catId,
        page: p,
        sortBy: sort,
      });
      if (seq !== loadSeqRef.current) return;
      setResults((prev) => (append ? [...prev, ...r] : r));
      setHasMore(r.length >= PAGE_SIZE);
      pageRef.current = p;
    } catch (e) {
      if (seq !== loadSeqRef.current) return;
      toast.error(getTauriErrorMessage(e));
    } finally {
      if (seq === loadSeqRef.current) setLoading(false);
    }
  }, []);

  const loadMore = useCallback(() => {
    if (!selectedCategory) return;
    const next = pageRef.current + 1;
    loadPage(selectedCategory, next, sortBy, true);
  }, [selectedCategory, sortBy, loadPage]);

  const sentinelRef = useInfiniteScroll(loadMore, {
    hasMore: hasMore && !filterText,
    isLoading: loading,
  });

  const handleCategoryChange = (catId: string | null) => {
    if (!catId) return;
    const id = Number(catId);
    setSelectedCategory(id);
    setFilterText("");
    setHasMore(true);
    pageRef.current = 0;
    onSelectResult(null);
    loadPage(id, 0, sortBy, false);
  };

  const handleSortChange = (sort: string | null) => {
    if (!sort) return;
    setSortBy(sort);
    setFilterText("");
    if (selectedCategory) {
      setHasMore(true);
      loadPage(selectedCategory, 0, sort, false);
    }
  };

  const filteredResults = useMemo(() => {
    if (!filterText.trim()) return results;
    const q = filterText.toLowerCase();
    return results.filter(
      (r) => r.title.toLowerCase().includes(q) || r.author.toLowerCase().includes(q)
    );
  }, [results, filterText]);

  // eslint-disable-next-line react-hooks/incompatible-library
  const rowVirtualizer = useVirtualizer({
    count: filteredResults.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 84,
    overscan: 8,
  });

  const selectedCategoryName = useMemo(
    () => categories.find((c) => c.id === selectedCategory)?.name ?? null,
    [categories, selectedCategory]
  );

  return (
    <>
      <div className="space-y-2 px-3 pb-2">
        <Select onValueChange={handleCategoryChange}>
          <SelectTrigger className="w-full">
            <SelectValue placeholder="Select a category...">{selectedCategoryName}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            {categories.map((cat) => (
              <SelectItem key={cat.id} value={String(cat.id)}>
                {cat.depth > 0 ? `${"  ".repeat(cat.depth)}${cat.name}` : cat.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div className="flex gap-2">
          <Select value={sortBy} onValueChange={handleSortChange}>
            <SelectTrigger className="flex-1">
              <SelectValue>
                {{ downloads: "Most Popular", newest: "Recently Updated", name: "Name" }[sortBy]}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="downloads">Most Popular</SelectItem>
              <SelectItem value="newest">Recently Updated</SelectItem>
              <SelectItem value="name">Name</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {/* Inline filter for loaded results */}
        {results.length > 0 && (
          <Input
            placeholder={`Filter ${selectedCategoryName ?? "results"}...`}
            aria-label="Filter category results"
            value={filterText}
            onChange={(e) => setFilterText(e.target.value)}
            className="h-7 text-xs"
          />
        )}
      </div>

      {/* Results count */}
      {results.length > 0 && (
        <div className="flex items-center justify-between px-3 pb-1">
          <span className="text-[11px] font-heading font-bold uppercase tracking-[0.05em] text-muted-foreground">
            {filterText ? `${filteredResults.length} of ${results.length}` : results.length} addon
            {(filterText ? filteredResults.length : results.length) !== 1 ? "s" : ""}
          </span>
        </div>
      )}

      <div ref={scrollRef} className="flex-1 overflow-y-auto">
        {results.length === 0 && loading ? (
          <DiscoverResultListSkeleton />
        ) : filteredResults.length === 0 && filterText ? (
          <EmptyState
            icon={<Search className="size-8 text-muted-foreground/20" />}
            title="No matches"
            subtitle={`No addons matching "${filterText}"`}
          />
        ) : results.length === 0 ? (
          <EmptyState
            icon={<FolderOpen className="size-8 text-muted-foreground/20" />}
            title={selectedCategory ? "No addons in this category" : "Browse Categories"}
            subtitle={
              selectedCategory
                ? "Try a different category or sort order"
                : "Select a category above to explore addons"
            }
          />
        ) : (
          <>
            <VirtualResultRows
              rowVirtualizer={rowVirtualizer}
              results={filteredResults}
              selectedResultId={selectedResultId}
              installingId={installingId}
              installProgress={installProgress}
              installedIds={installedIds}
              onSelectResult={onSelectResult}
              onInstall={onInstall}
            />
            {hasMore && !filterText && <div ref={sentinelRef} className="h-1" />}
            {loading && <DiscoverResultListSkeleton count={3} />}
          </>
        )}
      </div>
    </>
  );
}

/* ── URL / ID Tab ─────────────────────────────────────── */

function UrlContent({
  addonsPath,
  onInstalled,
  installedEsouiIds,
}: {
  addonsPath: string;
  onInstalled: () => void;
  installedEsouiIds: Set<number>;
}) {
  const ensureEsoNotBlocking = useEnsureEsoNotBlocking();
  const resolvePendingDeps = useResolvePendingDeps();
  const [input, setInput] = useState("");
  const [state, setState] = useState<
    "idle" | "resolving" | "resolved" | "installing" | "installed" | "error"
  >("idle");
  const [addonInfo, setAddonInfo] = useState<EsouiAddonInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<InstallResult | null>(null);
  const { progress, beginOperation, endOperation } = useInstallProgress();

  const handleResolve = async () => {
    if (!input.trim()) return;
    setState("resolving");
    setError(null);
    try {
      const info = await invokeOrThrow<EsouiAddonInfo>("resolve_esoui_addon", {
        input: input.trim(),
      });
      setAddonInfo(info);
      setState("resolved");
    } catch (e) {
      setError(getTauriErrorMessage(e));
      setState("error");
    }
  };

  const handleInstall = async () => {
    if (!addonInfo) return;
    setState("installing");
    if (!(await ensureEsoNotBlocking())) {
      setState("resolved");
      return;
    }
    setError(null);
    try {
      const installResult = await invokeOrThrow<InstallResult>("install_addon", {
        addonsPath,
        downloadUrl: addonInfo.downloadUrl,
        esouiId: addonInfo.id,
        esouiTitle: addonInfo.title,
        esouiVersion: addonInfo.version,
        dependencyPolicy: await getDependencyPolicy(),
        operationId: beginOperation(),
      });
      setResult(installResult);
      setState("installed");
      toast.success(`Installed ${installResult.installedFolders.join(", ")}`);
      reportDependencyFailures(installResult.failedDeps);
      onInstalled();
      // Empty unless the policy is "ask"; the app-level picker owns the rest.
      void resolvePendingDeps(installResult.pendingDeps, addonsPath);
    } catch (e) {
      setError(getTauriErrorMessage(e));
      setState("error");
    } finally {
      endOperation();
    }
  };

  const busy = state === "resolving" || state === "installing";

  return (
    <div className="flex-1 overflow-y-auto px-3 space-y-3">
      <div>
        <label htmlFor="esoui-input" className="mb-1 block text-xs text-muted-foreground">
          ESOUI URL or Addon ID
        </label>
        <Input
          id="esoui-input"
          value={input}
          onChange={(e) => {
            setInput(e.target.value);
            if (state !== "idle" && state !== "error") {
              setState("idle");
              setAddonInfo(null);
              setResult(null);
            }
          }}
          placeholder="https://esoui.com/... or 123"
          disabled={busy}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (state === "idle" || state === "error")) handleResolve();
          }}
          autoFocus
        />
      </div>

      <div className="rounded-xl border border-structure-04 bg-structure-02 p-3 space-y-2">
        <div className="text-[11px] font-heading font-bold uppercase tracking-[0.05em] text-muted-foreground">
          Supported formats
        </div>
        <div className="space-y-1 text-xs text-muted-foreground">
          <div className="flex items-center gap-2">
            <span className="text-primary">1.</span>
            <code className="rounded bg-structure-04 px-1.5 py-0.5 text-[11px]">
              https://esoui.com/downloads/info123
            </code>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-primary">2.</span>
            <code className="rounded bg-structure-04 px-1.5 py-0.5 text-[11px]">123</code>
            <span className="text-muted-foreground">(addon ID)</span>
          </div>
        </div>
      </div>

      {(state === "idle" || state === "error") && (
        <Button onClick={handleResolve} disabled={!input.trim()} className="w-full" size="sm">
          Resolve
        </Button>
      )}

      {state === "resolving" && (
        <Button disabled className="w-full" size="sm">
          <span className="inline-block size-3 animate-spin rounded-full border-2 border-[var(--primary-foreground)]/20 border-t-[var(--primary-foreground)] mr-2" />
          Resolving...
        </Button>
      )}

      {addonInfo && state === "resolved" && (
        <div className="rounded-xl border border-primary/15 bg-primary/[0.03] p-3 space-y-2">
          <div className="font-heading font-medium bg-gradient-to-r from-primary to-primary-hover bg-clip-text text-transparent">
            {addonInfo.title}
          </div>
          <div className="flex items-center gap-3 text-xs text-muted-foreground">
            <span>ESOUI #{addonInfo.id}</span>
            {addonInfo.version && <span>v{addonInfo.version}</span>}
            {addonInfo.updated && (
              <span className="flex items-center gap-1">
                <Clock className="size-3" />
                {addonInfo.updated}
              </span>
            )}
          </div>
          {installedEsouiIds.has(addonInfo.id) && (
            <div className="flex items-center gap-1.5 text-xs text-status-success">
              <Check className="size-3" />
              Already installed
            </div>
          )}
          <Button onClick={handleInstall} className="w-full" size="sm">
            {installedEsouiIds.has(addonInfo.id) ? "Reinstall" : "Install"}
          </Button>
        </div>
      )}

      {state === "installing" && (
        <div className="space-y-2">
          <Button disabled className="w-full" size="sm">
            <span className="inline-block size-3 animate-spin rounded-full border-2 border-[var(--primary-foreground)]/20 border-t-[var(--primary-foreground)] mr-2" />
            Installing...
          </Button>
          <ProgressBar
            value={progress?.done ?? 0}
            max={progress?.determinate ? progress.total : 100}
            indeterminate={!progress?.determinate}
            label={progress ? formatInstallProgress(progress) : "Preparing…"}
            className="h-1.5"
          />
          <div className="text-[11px] tabular-nums text-muted-foreground text-center">
            {progress ? formatInstallProgress(progress) : "Preparing…"}
          </div>
        </div>
      )}

      {state === "installed" && result && (
        <div className="space-y-2">
          <div className="rounded-xl border border-status-success/20 bg-status-success/[0.04] p-3 text-sm text-status-success flex items-center gap-2">
            <Check className="size-4 shrink-0" />
            Installed: {result.installedFolders.join(", ")}
          </div>
          {result.installedDeps.length > 0 && (
            <div className="rounded-xl border border-status-success/20 bg-status-success/[0.04] p-3 text-sm text-status-success flex items-center gap-2">
              <Check className="size-4 shrink-0" />
              Deps: {result.installedDeps.join(", ")}
            </div>
          )}
        </div>
      )}

      {error && (
        <div className="rounded-xl border border-status-danger/20 bg-status-danger/[0.04] p-3 text-sm text-status-danger">
          {error}
        </div>
      )}
    </div>
  );
}

/* ── Shared Components ────────────────────────────────── */

function EmptyState({
  icon,
  title,
  subtitle,
  subtitleClassName,
}: {
  icon: React.ReactNode;
  title: string;
  subtitle: React.ReactNode;
  /** Overrides the default narrow measure. The 200px cap suits a sentence but
   *  squashes richer content such as the Ask tab's example buttons. */
  subtitleClassName?: string;
}) {
  return (
    <Fade transition={{ type: "spring", stiffness: 200, damping: 25 }}>
      <div className="flex flex-col items-center justify-center py-12 gap-3 px-6">
        <div className="rounded-2xl bg-structure-03 border border-structure-06 p-4 shadow-[0_0_30px_color-mix(in_oklab,var(--primary)_3%,transparent)]">
          {icon}
        </div>
        <div className="text-center">
          <p className="font-heading text-sm font-medium text-foreground">{title}</p>
          <div
            className={cn(
              "mt-1 text-xs text-muted-foreground",
              subtitleClassName ?? "max-w-[200px]"
            )}
          >
            {subtitle}
          </div>
        </div>
      </div>
    </Fade>
  );
}
