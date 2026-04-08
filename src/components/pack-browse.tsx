import type { Pack, AuthUser } from "../types";
import type { PackTypeFilter, SortOption } from "./pack-constants";
import { TYPE_LABELS, TAG_COLORS, PACK_TYPE_ACCENT, PACK_TYPE_PILL_COLOR } from "./pack-constants";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { cn, decodeHtml } from "@/lib/utils";
import {
  SearchIcon,
  PackageIcon,
  AlertCircleIcon,
  RefreshCwIcon,
  SparklesIcon,
  ArrowUpIcon,
  DownloadIcon,
} from "lucide-react";

export function PackListView({
  packs,
  loading,
  loadingMore,
  hasMore,
  error,
  searchQuery,
  onSearchChange,
  typeFilter,
  onTypeFilterChange,
  sortMode,
  onSortChange,
  onSelectPack,
  onLoadMore,
  onRetry,
  onVote,
  votingPacks,
  authUser,
}: {
  packs: Pack[];
  loading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  error: string | null;
  searchQuery: string;
  onSearchChange: (q: string) => void;
  typeFilter: PackTypeFilter;
  onTypeFilterChange: (f: PackTypeFilter) => void;
  sortMode: SortOption;
  onSortChange: (s: SortOption) => void;
  onSelectPack: (id: string) => void;
  onLoadMore: () => void;
  onRetry: () => void;
  onVote: (packId: string) => void;
  votingPacks: Set<string>;
  authUser: AuthUser | null;
}) {
  return (
    <div className="flex flex-col gap-3 min-h-0">
      <div className="flex gap-2">
        <div className="relative flex-1">
          <SearchIcon className="absolute left-3 top-1/2 -translate-y-1/2 size-3.5 text-muted-foreground/40" />
          <Input
            placeholder="Search packs..."
            value={searchQuery}
            onChange={(e) => onSearchChange(e.target.value)}
            className="pl-9"
            autoFocus
          />
        </div>
        <Select
          value={typeFilter}
          onValueChange={(v) => v && onTypeFilterChange(v as PackTypeFilter)}
        >
          <SelectTrigger className="w-[130px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All Types</SelectItem>
            <SelectItem value="addon-pack">Addon Packs</SelectItem>
            <SelectItem value="build-pack">Build Packs</SelectItem>
            <SelectItem value="roster-pack">Roster Packs</SelectItem>
          </SelectContent>
        </Select>
        <Select value={sortMode} onValueChange={(v) => v && onSortChange(v as SortOption)}>
          <SelectTrigger className="w-[110px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="votes">Top Voted</SelectItem>
            <SelectItem value="newest">Newest</SelectItem>
            <SelectItem value="updated">Updated</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <div className="flex-1 overflow-y-auto space-y-2 min-h-0 max-h-[400px] px-1 -mx-1 py-1 -my-1">
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <div className="inline-block size-6 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
          </div>
        ) : error ? (
          <div className="flex flex-col items-center justify-center gap-3 py-12 text-center">
            <div className="rounded-xl bg-red-500/[0.08] border border-red-500/[0.15] p-4 shadow-[0_0_24px_rgba(239,68,68,0.08),inset_0_1px_0_rgba(239,68,68,0.06)]">
              <AlertCircleIcon className="size-8 text-red-400/70" />
            </div>
            <p className="font-heading text-sm font-medium text-red-400">Could not load packs</p>
            <p className="text-xs text-muted-foreground/60 max-w-[280px]">{error}</p>
            <Button variant="outline" size="sm" onClick={onRetry} className="mt-1">
              <RefreshCwIcon className="size-3.5 mr-1.5" />
              Retry
            </Button>
          </div>
        ) : packs.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-3 py-12 text-center">
            <div className="rounded-xl bg-[#c4a44a]/[0.08] border border-[#c4a44a]/[0.15] p-4 shadow-[0_0_24px_rgba(196,164,74,0.1),inset_0_1px_0_rgba(196,164,74,0.08)]">
              <SparklesIcon className="size-8 text-[#c4a44a]/60" />
            </div>
            <p className="font-heading text-sm font-medium">
              {searchQuery ? "No packs match your search" : "The Pack Hub is empty"}
            </p>
            <p className="text-xs text-muted-foreground/60 max-w-[260px]">
              {searchQuery
                ? "Try different keywords or clear filters"
                : "Be the first to share an addon pack with the community!"}
            </p>
          </div>
        ) : (
          packs.map((pack) => {
            const accent = PACK_TYPE_ACCENT[pack.packType] ?? PACK_TYPE_ACCENT["addon-pack"];
            const pillColor = PACK_TYPE_PILL_COLOR[pack.packType] ?? "muted";
            return (
              <div
                key={pack.id}
                role="button"
                tabIndex={0}
                onClick={() => onSelectPack(pack.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSelectPack(pack.id);
                  }
                }}
                className={cn(
                  "group w-full text-left rounded-xl border border-white/[0.06] p-3",
                  "border-l-[3px] transition-all duration-200 cursor-pointer",
                  "shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]",
                  accent.border,
                  accent.bg,
                  accent.hoverBg,
                  accent.hoverGlow,
                  "hover:border-white/[0.12] hover:-translate-y-[1px]",
                  "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sky-400/50"
                )}
              >
                {/* Top row: title + vote button */}
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="font-heading text-sm font-semibold truncate group-hover:text-[#c4a44a] transition-colors duration-200">
                        {decodeHtml(pack.title)}
                      </span>
                      <InfoPill color={pillColor}>
                        {TYPE_LABELS[pack.packType] ?? pack.packType}
                      </InfoPill>
                    </div>
                  </div>
                  {/* Vote button — right-aligned */}
                  <SimpleTooltip
                    content={
                      authUser ? (pack.userVoted ? "Remove vote" : "Upvote") : "Sign in to vote"
                    }
                  >
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        onVote(pack.id);
                      }}
                      disabled={votingPacks.has(pack.id)}
                      className={cn(
                        "group/vote relative flex flex-col items-center gap-0.5 text-xs font-semibold rounded-lg px-2 py-1.5 transition-all duration-200 border shrink-0",
                        votingPacks.has(pack.id) && "opacity-60 pointer-events-none",
                        pack.userVoted
                          ? "text-[#c4a44a] bg-[#c4a44a]/[0.15] border-[#c4a44a]/40 hover:bg-[#c4a44a]/[0.22] shadow-[0_0_12px_rgba(196,164,74,0.25),inset_0_1px_0_rgba(196,164,74,0.1)]"
                          : "text-muted-foreground/50 bg-white/[0.03] border-white/[0.06] hover:text-[#c4a44a] hover:border-[#c4a44a]/25 hover:bg-[#c4a44a]/[0.08] hover:shadow-[0_0_8px_rgba(196,164,74,0.08)]"
                      )}
                    >
                      <ArrowUpIcon
                        className={cn(
                          "size-3.5 transition-all duration-200",
                          pack.userVoted
                            ? "-translate-y-[1px]"
                            : "group-hover/vote:-translate-y-[1px]"
                        )}
                        strokeWidth={pack.userVoted ? 2.5 : 2}
                      />
                      <span className="tabular-nums leading-none">
                        {pack.voteCount > 0 ? pack.voteCount : 0}
                      </span>
                    </button>
                  </SimpleTooltip>
                </div>

                {/* Description */}
                {pack.description && (
                  <p className="mt-1.5 text-xs text-muted-foreground/70 line-clamp-2 leading-relaxed">
                    {decodeHtml(pack.description)}
                  </p>
                )}

                {/* Bottom row: tags + meta */}
                <div className="mt-2.5 flex items-center gap-1.5 flex-wrap">
                  {pack.tags.map((tag) => (
                    <InfoPill key={tag} color={TAG_COLORS[tag] ?? "muted"}>
                      {tag}
                    </InfoPill>
                  ))}
                  {pack.tags.length > 0 && pack.addons.length > 0 && (
                    <span className="text-muted-foreground/20 mx-0.5">·</span>
                  )}
                  <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/50">
                    <PackageIcon className="size-3" />
                    {pack.addons.length} addon{pack.addons.length !== 1 ? "s" : ""}
                  </span>
                  {(pack.installCount ?? 0) > 0 && (
                    <>
                      <span className="text-muted-foreground/20 mx-0.5">·</span>
                      <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/50">
                        <DownloadIcon className="size-3" />
                        {pack.installCount}
                      </span>
                    </>
                  )}
                  {!pack.isAnonymous && pack.authorName && (
                    <span className="text-[11px] text-muted-foreground/40 ml-auto inline-flex items-center gap-1.5">
                      <span
                        className={cn(
                          "inline-flex items-center justify-center size-4 rounded-full text-[8px] font-bold uppercase leading-none",
                          "bg-gradient-to-b from-white/[0.14] to-white/[0.06] text-muted-foreground/70 shadow-[inset_0_1px_0_rgba(255,255,255,0.1)]"
                        )}
                      >
                        {[...decodeHtml(pack.authorName)][0]}
                      </span>
                      {decodeHtml(pack.authorName)}
                    </span>
                  )}
                </div>
              </div>
            );
          })
        )}
        {!loading && hasMore && (
          <button
            onClick={onLoadMore}
            disabled={loadingMore}
            className={cn(
              "w-full py-2.5 rounded-xl border border-white/[0.06] bg-white/[0.02] text-xs font-semibold",
              "shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] transition-all duration-200 hover:bg-white/[0.05] hover:border-white/[0.12] hover:shadow-[inset_0_1px_0_rgba(255,255,255,0.05),0_2px_8px_rgba(0,0,0,0.15)]",
              "text-muted-foreground/60 hover:text-muted-foreground",
              loadingMore && "opacity-60 cursor-wait"
            )}
          >
            {loadingMore ? (
              <span className="inline-flex items-center gap-1.5">
                <span className="inline-block size-3 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
                Loading...
              </span>
            ) : (
              "Load More"
            )}
          </button>
        )}
      </div>
    </div>
  );
}
