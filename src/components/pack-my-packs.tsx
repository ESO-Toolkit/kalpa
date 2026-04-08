import { useState, useMemo } from "react";
import type { Pack, AuthUser, InstalledPackRef } from "../types";
import {
  type PackTypeFilter,
  type MyPacksSubTab,
  TYPE_LABELS,
  TAG_COLORS,
  PACK_TYPE_ACCENT,
  PACK_TYPE_PILL_COLOR,
} from "./pack-constants";
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
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { cn, decodeHtml, formatRelativeDate } from "@/lib/utils";
import { toast } from "sonner";
import {
  PackageIcon,
  PlusIcon,
  SparklesIcon,
  PencilIcon,
  CopyIcon,
  TrashIcon,
  ArrowUpIcon,
  Loader2Icon,
  SearchIcon,
  XIcon,
  DownloadIcon,
} from "lucide-react";

export function MyPacksView({
  packs,
  loading,
  loadingMore,
  hasMore,
  authUser,
  onAuthChange,
  onSelectPack,
  onLoadMore,
  onEdit,
  onDuplicate,
  onDelete,
  onCreatePack,
  onPublish,
  installedPackRefs,
  onRemoveInstalledRef,
}: {
  packs: Pack[];
  loading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  authUser: AuthUser | null;
  onAuthChange: (user: AuthUser | null) => void;
  onSelectPack: (id: string) => void;
  onLoadMore: () => void;
  onEdit: (pack: Pack) => void;
  onDuplicate: (pack: Pack) => void;
  onDelete: (packId: string) => void;
  onCreatePack: () => void;
  onPublish: (pack: Pack) => void;
  installedPackRefs: InstalledPackRef[];
  onRemoveInstalledRef: (packId: string) => void;
}) {
  const [loggingIn, setLoggingIn] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [mySearchQuery, setMySearchQuery] = useState("");
  const [myTypeFilter, setMyTypeFilter] = useState<PackTypeFilter>("all");
  const [subTab, setSubTab] = useState<MyPacksSubTab>("created");

  const filteredPacks = useMemo(() => {
    let result = packs;
    if (myTypeFilter !== "all") {
      result = result.filter((p) => p.packType === myTypeFilter);
    }
    if (mySearchQuery.trim()) {
      const q = mySearchQuery.toLowerCase();
      result = result.filter(
        (p) => p.title.toLowerCase().includes(q) || p.description.toLowerCase().includes(q)
      );
    }
    return result;
  }, [packs, myTypeFilter, mySearchQuery]);

  const handleLogin = async () => {
    setLoggingIn(true);
    try {
      const user = await invokeOrThrow<AuthUser>("auth_login");
      onAuthChange(user);
      toast.success(`Signed in as ${user.userName}`);
    } catch (e) {
      toast.error(`Sign in failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setLoggingIn(false);
    }
  };

  // Auth gate
  if (!authUser) {
    return (
      <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
        <div className="rounded-xl bg-[#c4a44a]/[0.06] border border-[#c4a44a]/[0.1] p-5">
          <PackageIcon className="size-10 text-[#c4a44a]/50" />
        </div>
        <div>
          <p className="font-heading text-sm font-semibold">Sign in to manage your packs</p>
          <p className="mt-1 text-xs text-muted-foreground/60 max-w-[260px]">
            Sign in with your ESO Logs account to view, edit, and manage your packs and drafts.
          </p>
        </div>
        <Button onClick={handleLogin} disabled={loggingIn} className="mt-1">
          {loggingIn ? (
            <>
              <Loader2Icon className="size-4 animate-spin mr-1.5" />
              Signing in...
            </>
          ) : (
            "Sign in with ESO Logs"
          )}
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 min-h-0">
      {/* Sub-tab toggle: Created / Installed */}
      <div className="relative flex p-0.5 rounded-lg bg-white/[0.03] border border-white/[0.06]">
        <div
          className="absolute top-0.5 bottom-0.5 rounded-md bg-white/[0.08] shadow-sm transition-all duration-300 ease-[cubic-bezier(0.34,1.56,0.64,1)]"
          style={{
            left: subTab === "created" ? "2px" : "calc(50% + 2px)",
            width: "calc(50% - 4px)",
          }}
        />
        {(["created", "installed"] as MyPacksSubTab[]).map((st) => (
          <button
            key={st}
            onClick={() => setSubTab(st)}
            className={cn(
              "relative z-10 flex-1 px-3 py-1.5 rounded-md text-xs font-semibold transition-colors duration-200",
              subTab === st
                ? "text-foreground"
                : "text-muted-foreground/60 hover:text-muted-foreground"
            )}
          >
            {st === "created"
              ? `Created (${packs.length})`
              : `Installed (${installedPackRefs.length})`}
          </button>
        ))}
      </div>

      {subTab === "created" ? (
        <>
          {/* Header */}
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <span className="text-xs text-muted-foreground/60">
                Your packs as{" "}
                <span className="text-[#c4a44a] font-semibold">{authUser.userName}</span>
              </span>
              <span className="text-[10px] text-muted-foreground/40 tabular-nums">
                {packs.length} / 25
              </span>
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={onCreatePack}
              disabled={packs.length >= 25}
            >
              <PlusIcon className="size-3.5 mr-1.5" />
              Create Pack
            </Button>
          </div>

          {/* Search & filter */}
          {packs.length > 0 && (
            <div className="flex gap-2">
              <div className="relative flex-1">
                <SearchIcon className="absolute left-3 top-1/2 -translate-y-1/2 size-3.5 text-muted-foreground/40" />
                <Input
                  placeholder="Search your packs..."
                  value={mySearchQuery}
                  onChange={(e) => setMySearchQuery(e.target.value)}
                  className="pl-9"
                />
              </div>
              <Select
                value={myTypeFilter}
                onValueChange={(v) => v && setMyTypeFilter(v as PackTypeFilter)}
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
            </div>
          )}

          <div className="flex-1 overflow-y-auto space-y-2 min-h-0 max-h-[400px] px-1 -mx-1 py-1 -my-1">
            {loading ? (
              <div className="flex items-center justify-center py-12">
                <div className="inline-block size-6 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
              </div>
            ) : packs.length === 0 ? (
              <div className="flex flex-col items-center justify-center gap-3 py-12 text-center">
                <div className="rounded-xl bg-[#c4a44a]/[0.06] border border-[#c4a44a]/[0.1] p-4">
                  <SparklesIcon className="size-8 text-[#c4a44a]/50" />
                </div>
                <p className="font-heading text-sm font-medium">No packs yet</p>
                <p className="text-xs text-muted-foreground/60 max-w-[260px]">
                  You haven&apos;t created any packs yet. Share your favourite addon collections
                  with the community!
                </p>
                <Button size="sm" onClick={onCreatePack} className="mt-1">
                  <PlusIcon className="size-3.5 mr-1.5" />
                  Create your first pack
                </Button>
              </div>
            ) : filteredPacks.length === 0 ? (
              <div className="flex flex-col items-center justify-center gap-3 py-12 text-center">
                <p className="font-heading text-sm font-medium">No packs match your filters</p>
                <p className="text-xs text-muted-foreground/60 max-w-[260px]">
                  Try different keywords or clear your filters.
                </p>
              </div>
            ) : (
              filteredPacks.map((pack) => {
                const accent = PACK_TYPE_ACCENT[pack.packType] ?? PACK_TYPE_ACCENT["addon-pack"];
                const pillColor = PACK_TYPE_PILL_COLOR[pack.packType] ?? "muted";
                const isConfirmingDelete = confirmDeleteId === pack.id;
                return (
                  <div key={pack.id} className="relative">
                    <div
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
                        accent.border,
                        accent.bg,
                        accent.hoverBg,
                        "hover:border-white/[0.12] hover:-translate-y-[1px] hover:shadow-[0_4px_16px_rgba(0,0,0,0.2)]",
                        "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sky-400/50"
                      )}
                    >
                      {/* Top row: title + quick actions */}
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2">
                            <span className="font-heading text-sm font-semibold truncate group-hover:text-[#c4a44a] transition-colors duration-200">
                              {decodeHtml(pack.title)}
                            </span>
                            <InfoPill color={pillColor}>
                              {TYPE_LABELS[pack.packType] ?? pack.packType}
                            </InfoPill>
                            {pack.status === "draft" && <InfoPill color="muted">Draft</InfoPill>}
                          </div>
                        </div>
                        {/* Quick actions */}
                        <div className="flex items-center gap-1 shrink-0">
                          {pack.status === "draft" && (
                            <SimpleTooltip content="Publish">
                              <button
                                onClick={(e) => {
                                  e.stopPropagation();
                                  onPublish(pack);
                                }}
                                className="rounded-md p-1.5 text-muted-foreground/40 hover:text-emerald-400 hover:bg-emerald-400/[0.08] transition-all duration-150"
                              >
                                <ArrowUpIcon className="size-3.5" />
                              </button>
                            </SimpleTooltip>
                          )}
                          <SimpleTooltip content="Edit">
                            <button
                              onClick={(e) => {
                                e.stopPropagation();
                                onEdit(pack);
                              }}
                              className="rounded-md p-1.5 text-muted-foreground/40 hover:text-[#c4a44a] hover:bg-[#c4a44a]/[0.08] transition-all duration-150"
                            >
                              <PencilIcon className="size-3.5" />
                            </button>
                          </SimpleTooltip>
                          <SimpleTooltip content="Duplicate">
                            <button
                              onClick={(e) => {
                                e.stopPropagation();
                                onDuplicate(pack);
                              }}
                              className="rounded-md p-1.5 text-muted-foreground/40 hover:text-sky-400 hover:bg-sky-400/[0.08] transition-all duration-150"
                            >
                              <CopyIcon className="size-3.5" />
                            </button>
                          </SimpleTooltip>
                          {pack.authorId === authUser.userId && (
                            <SimpleTooltip content="Delete">
                              <button
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setConfirmDeleteId(isConfirmingDelete ? null : pack.id);
                                }}
                                className={cn(
                                  "rounded-md p-1.5 transition-all duration-150",
                                  isConfirmingDelete
                                    ? "text-red-400 bg-red-400/[0.1]"
                                    : "text-muted-foreground/40 hover:text-red-400 hover:bg-red-400/[0.08]"
                                )}
                              >
                                <TrashIcon className="size-3.5" />
                              </button>
                            </SimpleTooltip>
                          )}
                        </div>
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
                        {pack.updatedAt && formatRelativeDate(pack.updatedAt) && (
                          <span className="text-[10px] text-muted-foreground/30 ml-auto">
                            Updated {formatRelativeDate(pack.updatedAt)}
                          </span>
                        )}
                      </div>
                    </div>

                    {/* Inline delete confirmation */}
                    {isConfirmingDelete && (
                      <div className="mt-1 flex items-center justify-between rounded-lg border border-red-500/20 bg-red-500/[0.06] px-3 py-2 overflow-hidden transition-all duration-200">
                        <span className="text-xs text-red-400 font-medium">Delete this pack?</span>
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => setConfirmDeleteId(null)}
                            className="text-xs text-muted-foreground/60 hover:text-muted-foreground transition-colors px-2 py-0.5"
                          >
                            Cancel
                          </button>
                          <button
                            onClick={() => {
                              setConfirmDeleteId(null);
                              onDelete(pack.id);
                            }}
                            className="text-xs font-semibold text-red-400 hover:text-red-300 bg-red-500/10 hover:bg-red-500/20 rounded-md px-2.5 py-0.5 transition-all duration-150"
                          >
                            Delete
                          </button>
                        </div>
                      </div>
                    )}
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
                  "transition-all duration-200 hover:bg-white/[0.04] hover:border-white/[0.1]",
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
        </>
      ) : (
        /* -- Installed packs sub-tab -- */
        <div className="flex-1 overflow-y-auto space-y-2 min-h-0 max-h-[400px] px-1 -mx-1 py-1 -my-1">
          {installedPackRefs.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-3 py-12 text-center">
              <div className="rounded-xl bg-sky-400/[0.06] border border-sky-400/[0.1] p-4">
                <DownloadIcon className="size-8 text-sky-400/50" />
              </div>
              <p className="font-heading text-sm font-medium">No installed packs yet</p>
              <p className="text-xs text-muted-foreground/60 max-w-[260px]">
                Packs you install from the Browse tab will appear here for easy reference.
              </p>
            </div>
          ) : (
            installedPackRefs.map((ref) => {
              const accent = PACK_TYPE_ACCENT[ref.packType] ?? PACK_TYPE_ACCENT["addon-pack"];
              const pillColor = PACK_TYPE_PILL_COLOR[ref.packType] ?? "muted";
              return (
                <div
                  key={ref.packId}
                  role="button"
                  tabIndex={0}
                  onClick={() => onSelectPack(ref.packId)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      onSelectPack(ref.packId);
                    }
                  }}
                  className={cn(
                    "group w-full text-left rounded-xl border border-white/[0.06] p-3",
                    "border-l-[3px] transition-all duration-200 cursor-pointer",
                    accent.border,
                    accent.bg,
                    accent.hoverBg,
                    "hover:border-white/[0.12] hover:-translate-y-[1px] hover:shadow-[0_4px_16px_rgba(0,0,0,0.2)]",
                    "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sky-400/50"
                  )}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="font-heading text-sm font-semibold truncate group-hover:text-[#c4a44a] transition-colors duration-200">
                          {decodeHtml(ref.title)}
                        </span>
                        <InfoPill color={pillColor}>
                          {TYPE_LABELS[ref.packType] ?? ref.packType}
                        </InfoPill>
                      </div>
                    </div>
                    <SimpleTooltip content="Remove from library">
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          onRemoveInstalledRef(ref.packId);
                        }}
                        className="rounded-md p-1.5 text-muted-foreground/40 hover:text-red-400 hover:bg-red-400/[0.08] transition-all duration-150"
                      >
                        <XIcon className="size-3.5" />
                      </button>
                    </SimpleTooltip>
                  </div>
                  <div className="mt-2 flex items-center gap-1.5 flex-wrap">
                    <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/50">
                      <PackageIcon className="size-3" />
                      {ref.addonCount} addon{ref.addonCount !== 1 ? "s" : ""}
                    </span>
                    {ref.authorName && (
                      <>
                        <span className="text-muted-foreground/20 mx-0.5">·</span>
                        <span className="text-[11px] text-muted-foreground/40">
                          by {decodeHtml(ref.authorName)}
                        </span>
                      </>
                    )}
                    {ref.installedAt && formatRelativeDate(ref.installedAt) && (
                      <span className="text-[10px] text-muted-foreground/30 ml-auto">
                        Installed {formatRelativeDate(ref.installedAt)}
                      </span>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}
