import { useState, useEffect, useRef, useMemo } from "react";
import type { Pack, PackAddonEntry, EsouiSearchResult, AddonManifest, AuthUser } from "../types";
import {
  PACK_TYPE_ACCENT,
  PACK_TYPE_PILL_COLOR,
  PACK_TYPE_DESCRIPTIONS,
  PRESET_TAGS,
  TYPE_LABELS,
} from "./pack-constants";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import {
  SearchIcon,
  PackageIcon,
  ArrowLeftIcon,
  CheckIcon,
  PlusIcon,
  XIcon,
  ArrowUpIcon,
  Loader2Icon,
  SparklesIcon,
  FileDownIcon,
} from "lucide-react";

type AddonSource = "search" | "installed";

interface CreateViewProps {
  installedAddons: AddonManifest[];
  authUser: AuthUser | null;
  onAuthChange: (user: AuthUser | null) => void;
  step: "details" | "addons";
  onStepChange: (s: "details" | "addons") => void;
  title: string;
  onTitleChange: (v: string) => void;
  description: string;
  onDescriptionChange: (v: string) => void;
  packType: string;
  onPackTypeChange: (v: string) => void;
  selectedTags: string[];
  onTagsChange: (v: string[] | ((prev: string[]) => string[])) => void;
  addons: PackAddonEntry[];
  onAddonsChange: (v: PackAddonEntry[] | ((prev: PackAddonEntry[]) => PackAddonEntry[])) => void;
  isAnonymous: boolean;
  onAnonymousChange: (v: boolean) => void;
  editingPackId: string | null;
  onPublished: (pack: Pack) => void;
  onCancelEdit?: () => void;
}

export function PackCreateView({
  installedAddons,
  authUser,
  onAuthChange,
  step,
  onStepChange: setStep,
  title,
  onTitleChange: setTitle,
  description,
  onDescriptionChange: setDescription,
  packType,
  onPackTypeChange: setPackType,
  selectedTags,
  onTagsChange: setSelectedTags,
  addons,
  onAddonsChange: setAddons,
  isAnonymous,
  onAnonymousChange: setIsAnonymous,
  editingPackId,
  onPublished,
  onCancelEdit,
}: CreateViewProps) {
  // Search
  const [addonSource, setAddonSource] = useState<AddonSource>("search");
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<EsouiSearchResult[]>([]);
  const [searching, setSearching] = useState(false);
  const createSearchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const createSearchSeqRef = useRef(0);

  // Clean up search timer on unmount
  useEffect(() => {
    return () => {
      if (createSearchTimerRef.current) clearTimeout(createSearchTimerRef.current);
    };
  }, []);

  // Installed addons filter
  const [installedFilter, setInstalledFilter] = useState("");

  const handleTagToggle = (tag: string) => {
    setSelectedTags((prev) => {
      if (prev.includes(tag)) return prev.filter((t) => t !== tag);
      if (prev.length >= 5) return prev;
      return [...prev, tag];
    });
  };

  const handleSearchChange = (query: string) => {
    setSearchQuery(query);
    if (createSearchTimerRef.current) clearTimeout(createSearchTimerRef.current);
    if (query.trim().length < 2) {
      setSearchResults([]);
      setSearching(false);
      return;
    }
    setSearching(true);
    createSearchTimerRef.current = setTimeout(async () => {
      const seq = ++createSearchSeqRef.current;
      try {
        const results = await invokeOrThrow<EsouiSearchResult[]>("search_esoui_addons", {
          query: query.trim(),
        });
        if (seq !== createSearchSeqRef.current) return;
        setSearchResults(results);
      } catch (e) {
        if (seq !== createSearchSeqRef.current) return;
        setSearchResults([]);
        toast.error(`Search failed: ${getTauriErrorMessage(e)}`);
      } finally {
        if (seq === createSearchSeqRef.current) {
          setSearching(false);
        }
      }
    }, 400);
  };

  const handleAddAddon = (entry: PackAddonEntry) => {
    let added = false;
    setAddons((prev) => {
      if (prev.some((a) => a.esouiId === entry.esouiId)) return prev;
      added = true;
      return [...prev, entry];
    });
    // Toast after updater so we know the outcome
    // React processes the updater synchronously within setState, so `added` is set by here
    if (added) {
      toast.success(`Added "${entry.name}"`);
    } else {
      toast.error(`"${entry.name}" is already in the pack.`);
    }
  };

  const handleRemoveAddon = (esouiId: number) => {
    setAddons((prev) => prev.filter((a) => a.esouiId !== esouiId));
  };

  const handleToggleRequired = (esouiId: number) => {
    setAddons((prev) =>
      prev.map((a) => (a.esouiId === esouiId ? { ...a, required: !a.required } : a))
    );
  };

  const [publishing, setPublishing] = useState(false);
  const [savingToFile, setSavingToFile] = useState(false);
  const [loggingIn, setLoggingIn] = useState(false);

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

  const handleLogout = async () => {
    try {
      await invokeOrThrow("auth_logout");
      onAuthChange(null);
      toast.success("Signed out");
    } catch (e) {
      toast.error(`Sign out failed: ${getTauriErrorMessage(e)}`);
    }
  };

  const handleSaveDraft = async () => {
    if (!title.trim()) {
      toast.error("Pack needs a title.");
      return;
    }
    if (addons.length === 0) {
      toast.error("Add at least one addon.");
      return;
    }
    setPublishing(true);
    try {
      const pack = editingPackId
        ? await invokeOrThrow<Pack>("update_pack", {
            payload: {
              id: editingPackId,
              title: title.trim(),
              description: description.trim(),
              packType,
              addons,
              tags: selectedTags,
              isAnonymous,
              status: "draft",
            },
          })
        : await invokeOrThrow<Pack>("create_pack", {
            payload: {
              title: title.trim(),
              description: description.trim(),
              packType,
              addons,
              tags: selectedTags,
              isAnonymous,
              status: "draft",
            },
          });
      toast.success("Pack saved as draft");
      onPublished(pack);
    } catch (e) {
      const msg = getTauriErrorMessage(e);
      if (msg.includes("expired") || msg.includes("sign in")) {
        onAuthChange(null);
      }
      if (msg.includes("Maximum")) {
        toast.error(msg);
      } else {
        toast.error(`Save failed: ${msg}`);
      }
    } finally {
      setPublishing(false);
    }
  };

  const handlePublish = async () => {
    if (!title.trim()) {
      toast.error("Pack needs a title.");
      return;
    }
    if (addons.length === 0) {
      toast.error("Add at least one addon.");
      return;
    }
    setPublishing(true);
    try {
      const pack = editingPackId
        ? await invokeOrThrow<Pack>("update_pack", {
            payload: {
              id: editingPackId,
              title: title.trim(),
              description: description.trim(),
              packType,
              addons,
              tags: selectedTags,
              isAnonymous,
              status: "published",
            },
          })
        : await invokeOrThrow<Pack>("create_pack", {
            payload: {
              title: title.trim(),
              description: description.trim(),
              packType,
              addons,
              tags: selectedTags,
              isAnonymous,
              status: "published",
            },
          });
      toast.success(editingPackId ? "Pack updated!" : "Pack published!");
      onPublished(pack);
    } catch (e) {
      const msg = getTauriErrorMessage(e);
      if (msg.includes("expired") || msg.includes("sign in")) {
        onAuthChange(null);
      }
      if (msg.includes("Maximum")) {
        toast.error(msg);
      } else {
        toast.error(`Publish failed: ${msg}`);
      }
    } finally {
      setPublishing(false);
    }
  };

  const handleSaveToFile = async () => {
    if (!title.trim()) {
      toast.error("Pack needs a title.");
      return;
    }
    if (addons.length === 0) {
      toast.error("Add at least one addon.");
      return;
    }
    const safeName = title
      .trim()
      .replace(/[^a-zA-Z0-9-_ ]/g, "")
      .trim()
      .replace(/\s+/g, "-");
    const path = await saveFileDialog({
      defaultPath: `${safeName}.esopack`,
      filters: [{ name: "ESO Pack", extensions: ["esopack"] }],
    });
    if (!path) return;

    setSavingToFile(true);
    try {
      await invokeOrThrow("export_pack_file", {
        pack: {
          format: "esopack",
          version: 1,
          pack: {
            title: title.trim(),
            description: description.trim(),
            packType,
            tags: selectedTags,
            addons,
          },
          sharedAt: new Date().toISOString(),
          sharedBy: authUser?.userName ?? "Anonymous",
        },
        path,
      });
      toast.success("Pack saved to file");
    } catch (e) {
      toast.error(`Failed to save pack: ${getTauriErrorMessage(e)}`);
    } finally {
      setSavingToFile(false);
    }
  };

  // Filtered installed addons (only non-library addons with ESOUI IDs)
  const filteredInstalled = useMemo(
    () =>
      installedAddons
        .filter((a) => a.esouiId && a.esouiId > 0 && !a.isLibrary)
        .filter(
          (a) =>
            !installedFilter ||
            a.title.toLowerCase().includes(installedFilter.toLowerCase()) ||
            a.folderName.toLowerCase().includes(installedFilter.toLowerCase())
        ),
    [installedAddons, installedFilter]
  );

  const canProceed = !!title.trim();

  return (
    <div className="flex flex-col gap-3 min-h-0">
      {/* Header — shows auth state and edit controls */}
      <div className="flex items-center justify-between text-xs">
        {authUser ? (
          <span className="text-muted-foreground/60">
            {editingPackId ? "Editing as " : "Creating as "}
            <span className="text-[#c4a44a] font-semibold">{authUser.userName}</span>
          </span>
        ) : (
          <span className="text-muted-foreground/60">Creating a pack</span>
        )}
        <div className="flex items-center gap-3">
          {editingPackId && onCancelEdit && (
            <button
              onClick={onCancelEdit}
              className="text-muted-foreground/40 hover:text-muted-foreground transition-colors"
            >
              Cancel edit
            </button>
          )}
          {authUser && (
            <button
              onClick={handleLogout}
              className="text-muted-foreground/40 hover:text-muted-foreground transition-colors"
            >
              Sign out
            </button>
          )}
        </div>
      </div>

      {/* Step indicator */}
      <div className="flex items-center gap-2">
        {[
          { num: 1, label: "Details", key: "details" as const },
          { num: 2, label: "Addons", key: "addons" as const },
        ].map((s, i) => (
          <button
            key={s.key}
            onClick={() => {
              if (s.key === "addons" && !canProceed) return;
              setStep(s.key);
            }}
            disabled={s.key === "addons" && !canProceed}
            className={cn(
              "flex items-center gap-1.5 text-xs font-semibold transition-all duration-200",
              step === s.key
                ? "text-[#c4a44a]"
                : s.key === "addons" && !canProceed
                  ? "text-muted-foreground/30 cursor-not-allowed"
                  : "text-muted-foreground/50 hover:text-muted-foreground cursor-pointer"
            )}
          >
            <span
              className={cn(
                "inline-flex items-center justify-center size-5 rounded-full text-[10px] font-bold leading-none transition-all duration-200",
                step === s.key
                  ? "bg-[#c4a44a]/20 text-[#c4a44a] border border-[#c4a44a]/40"
                  : "bg-white/[0.04] text-muted-foreground/40 border border-white/[0.08]"
              )}
            >
              {s.num}
            </span>
            {s.label}
            {i === 0 && <span className="text-muted-foreground/20 mx-1">›</span>}
          </button>
        ))}
      </div>

      {step === "details" ? (
        /* ── Step 1: Pack Details ── */
        <div className="flex flex-col gap-3 overflow-y-auto max-h-[420px] px-3 -mx-3 pr-1">
          <p className="text-sm text-muted-foreground">
            {editingPackId
              ? "Update your pack details, then review the addon list before saving."
              : "Create an addon pack — save it locally for personal use, or publish it to the community."}
          </p>

          {/* Title */}
          <div>
            <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground/60 mb-1 block">
              Pack Name <span className="text-red-400">*</span>
            </label>
            <Input
              placeholder="e.g. Trial Essentials, PvP Toolkit"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              maxLength={100}
              autoFocus
            />
            <div className="mt-1 flex items-center gap-2">
              <div className="flex-1 h-0.5 rounded bg-white/[0.04] overflow-hidden">
                <div
                  className="h-full rounded bg-[#c4a44a] transition-all duration-300"
                  style={{ width: `${Math.min((title.length / 100) * 100, 100)}%` }}
                />
              </div>
              <span className="text-[10px] text-muted-foreground/40 tabular-nums">
                {title.length}/100
              </span>
            </div>
          </div>

          {/* Description */}
          <div>
            <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground/60 mb-1 block">
              Description
            </label>
            <textarea
              placeholder="What is this pack for? Who should use it?"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              maxLength={500}
              rows={3}
              className="w-full rounded-lg border border-input bg-transparent px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground/40 outline-none transition-colors focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 resize-none dark:bg-input/30 dark:hover:bg-input/50"
            />
            <div className="mt-1 flex items-center gap-2">
              <div className="flex-1 h-0.5 rounded bg-white/[0.04] overflow-hidden">
                <div
                  className="h-full rounded bg-[#c4a44a] transition-all duration-300"
                  style={{ width: `${Math.min((description.length / 500) * 100, 100)}%` }}
                />
              </div>
              <span className="text-[10px] text-muted-foreground/40 tabular-nums">
                {description.length}/500
              </span>
            </div>
          </div>

          {/* Pack type — clickable cards */}
          <div>
            <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground/60 mb-1.5 block">
              Pack Type
            </label>
            <div className="grid grid-cols-3 gap-2">
              {(["addon-pack", "build-pack", "roster-pack"] as const).map((pt) => {
                const accent = PACK_TYPE_ACCENT[pt];
                const pillColor = PACK_TYPE_PILL_COLOR[pt] ?? "muted";
                const isSelected = packType === pt;
                return (
                  <button
                    key={pt}
                    onClick={() => setPackType(pt)}
                    className={cn(
                      "relative flex flex-col items-start gap-1 rounded-lg border p-2.5 text-left transition-all duration-200",
                      isSelected
                        ? `${accent.border} border-l-[3px] bg-white/[0.08] border-white/[0.15] ring-1 ring-white/[0.08]`
                        : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.1] hover:bg-white/[0.04]",
                      "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sky-400/50"
                    )}
                  >
                    {isSelected && (
                      <span
                        className={cn(
                          "absolute top-1.5 right-1.5 flex items-center justify-center size-4 rounded-full",
                          "bg-white/[0.1] border border-white/[0.15]"
                        )}
                      >
                        <CheckIcon className={cn("size-2.5", accent.text)} />
                      </span>
                    )}
                    <InfoPill color={pillColor}>{TYPE_LABELS[pt]}</InfoPill>
                    <span
                      className={cn(
                        "text-[10px] leading-tight transition-colors duration-200",
                        isSelected ? "text-muted-foreground/70" : "text-muted-foreground/50"
                      )}
                    >
                      {PACK_TYPE_DESCRIPTIONS[pt]}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Tags */}
          <div>
            <div className="flex items-baseline justify-between mb-1">
              <label className="text-xs font-semibold uppercase tracking-wider text-muted-foreground/60">
                Tags
              </label>
              <span
                className={cn(
                  "text-[10px] tabular-nums",
                  selectedTags.length >= 5 ? "text-amber-400" : "text-muted-foreground/40"
                )}
              >
                {selectedTags.length}/5
              </span>
            </div>
            <div className="flex flex-wrap gap-1.5">
              {PRESET_TAGS.map((tag) => {
                const isSelected = selectedTags.includes(tag);
                const isDisabled = !isSelected && selectedTags.length >= 5;
                return (
                  <button
                    key={tag}
                    onClick={() => !isDisabled && handleTagToggle(tag)}
                    disabled={isDisabled}
                    className={cn(
                      "px-2.5 py-1 rounded-md text-xs font-semibold transition-all duration-150",
                      isSelected
                        ? "bg-[#c4a44a]/20 text-[#c4a44a] border border-[#c4a44a]/40"
                        : "bg-white/[0.03] text-muted-foreground/60 border border-white/[0.06] hover:border-white/[0.12] hover:text-muted-foreground",
                      isDisabled && "opacity-30 cursor-not-allowed"
                    )}
                  >
                    {tag}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Next button */}
          <Button onClick={() => setStep("addons")} disabled={!canProceed} className="mt-1">
            Next: Add Addons
            <ArrowLeftIcon className="size-4 ml-1.5 rotate-180" />
          </Button>
        </div>
      ) : (
        /* ── Step 2: Addon Search & Selection ── */
        <div className="flex flex-col gap-3 min-h-0">
          {/* Back + addon count */}
          <div className="flex items-center justify-between">
            <button
              onClick={() => setStep("details")}
              className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
            >
              <ArrowLeftIcon className="size-3" />
              Back to details
            </button>
            {addons.length > 0 && (
              <span className="text-xs text-[#c4a44a] font-semibold">
                {addons.length} addon{addons.length !== 1 ? "s" : ""} selected
              </span>
            )}
          </div>

          {/* Source toggle with animated pill */}
          <div className="relative flex p-0.5 rounded-lg bg-white/[0.03] border border-white/[0.06]">
            <div
              className="absolute top-0.5 bottom-0.5 rounded-md bg-white/[0.08] shadow-sm transition-all duration-300 ease-[cubic-bezier(0.34,1.56,0.64,1)]"
              style={{
                left: addonSource === "search" ? "2px" : "calc(50% + 2px)",
                width: "calc(50% - 4px)",
              }}
            />
            {(["search", "installed"] as AddonSource[]).map((src) => (
              <button
                key={src}
                onClick={() => setAddonSource(src)}
                className={cn(
                  "relative z-10 flex-1 px-3 py-1.5 rounded-md text-xs font-semibold transition-colors duration-200",
                  addonSource === src
                    ? "text-foreground"
                    : "text-muted-foreground/60 hover:text-muted-foreground"
                )}
              >
                {src === "search" ? (
                  <>
                    <SearchIcon className="size-3 inline mr-1" />
                    Search ESOUI
                  </>
                ) : (
                  <>
                    <PackageIcon className="size-3 inline mr-1" />
                    My Installed
                  </>
                )}
              </button>
            ))}
          </div>

          {/* Search / Filter input */}
          <div className="relative">
            <SearchIcon className="absolute left-3 top-1/2 -translate-y-1/2 size-3.5 text-muted-foreground/40" />
            {addonSource === "search" ? (
              <Input
                placeholder="Search ESOUI addons..."
                value={searchQuery}
                onChange={(e) => handleSearchChange(e.target.value)}
                className="pl-9"
                autoFocus
              />
            ) : (
              <Input
                placeholder="Filter installed addons..."
                value={installedFilter}
                onChange={(e) => setInstalledFilter(e.target.value)}
                className="pl-9"
                autoFocus
              />
            )}
          </div>

          {/* Two-pane layout: results + selected */}
          <div className="flex gap-2 min-h-0 flex-1 overflow-hidden" style={{ maxHeight: 300 }}>
            {/* Left: search results or installed addons */}
            <div className="flex-1 overflow-y-auto space-y-1 min-w-0">
              {addonSource === "search" ? (
                searching ? (
                  <div className="flex items-center justify-center py-8">
                    <div className="inline-block size-5 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
                  </div>
                ) : searchResults.length === 0 ? (
                  <div className="text-center py-8">
                    <SearchIcon className="size-6 mx-auto text-muted-foreground/20 mb-2" />
                    <p className="text-xs text-muted-foreground/50">
                      {searchQuery.length < 2 ? "Type to search ESOUI addons" : "No results found"}
                    </p>
                  </div>
                ) : (
                  searchResults.map((result) => {
                    const alreadyAdded = addons.some((a) => a.esouiId === result.id);
                    return (
                      <button
                        key={result.id}
                        disabled={alreadyAdded}
                        onClick={() =>
                          handleAddAddon({
                            esouiId: result.id,
                            name: result.title,
                            required: true,
                          })
                        }
                        className={cn(
                          "w-full text-left rounded-lg p-2 transition-all duration-150",
                          "border border-transparent",
                          alreadyAdded
                            ? "opacity-40 cursor-not-allowed bg-white/[0.02]"
                            : "hover:bg-white/[0.04] hover:border-white/[0.08] cursor-pointer"
                        )}
                      >
                        <div className="flex items-center gap-2">
                          {alreadyAdded ? (
                            <CheckIcon className="size-3.5 text-[#c4a44a] shrink-0" />
                          ) : (
                            <PlusIcon className="size-3.5 text-[#c4a44a] shrink-0" />
                          )}
                          <span className="text-sm font-medium truncate">{result.title}</span>
                          <span className="text-[10px] text-muted-foreground/30 tabular-nums shrink-0">
                            #{result.id}
                          </span>
                        </div>
                        <p className="text-[11px] text-muted-foreground/50 mt-0.5 truncate ml-5">
                          by {result.author}
                          {result.category ? ` · ${result.category}` : ""}
                          {result.downloads ? ` · ${result.downloads} downloads` : ""}
                        </p>
                      </button>
                    );
                  })
                )
              ) : filteredInstalled.length === 0 ? (
                <div className="text-center py-8">
                  <PackageIcon className="size-6 mx-auto text-muted-foreground/20 mb-2" />
                  <p className="text-xs text-muted-foreground/50">
                    {installedFilter
                      ? "No matching installed addons"
                      : "No installed addons with ESOUI IDs"}
                  </p>
                </div>
              ) : (
                filteredInstalled.map((addon) => {
                  const alreadyAdded = addons.some((a) => a.esouiId === addon.esouiId);
                  return (
                    <button
                      key={addon.folderName}
                      disabled={alreadyAdded}
                      onClick={() =>
                        handleAddAddon({
                          esouiId: addon.esouiId!,
                          name: addon.title || addon.folderName,
                          required: true,
                        })
                      }
                      className={cn(
                        "w-full text-left rounded-lg p-2 transition-all duration-150",
                        "border border-transparent",
                        alreadyAdded
                          ? "opacity-40 cursor-not-allowed bg-white/[0.02]"
                          : "hover:bg-white/[0.04] hover:border-white/[0.08] cursor-pointer"
                      )}
                    >
                      <div className="flex items-center gap-2">
                        {alreadyAdded ? (
                          <CheckIcon className="size-3.5 text-[#c4a44a] shrink-0" />
                        ) : (
                          <PlusIcon className="size-3.5 text-[#c4a44a] shrink-0" />
                        )}
                        <span className="text-sm font-medium truncate">
                          {addon.title || addon.folderName}
                        </span>
                        <span className="text-[10px] text-muted-foreground/30 tabular-nums shrink-0">
                          #{addon.esouiId}
                        </span>
                      </div>
                      <p className="text-[11px] text-muted-foreground/50 mt-0.5 truncate ml-5">
                        by {addon.author} · v{addon.version}
                      </p>
                    </button>
                  );
                })
              )}
            </div>

            {/* Right: selected addons */}
            <div className="w-[220px] shrink-0 overflow-y-auto border-l border-white/[0.06] pl-2">
              <SectionHeader className="mb-1.5 sticky top-0 bg-background/80 backdrop-blur-sm pb-1">
                Selected ({addons.length})
              </SectionHeader>
              {addons.length === 0 ? (
                <div className="flex flex-col items-center justify-center gap-2 py-6 text-center">
                  <div className="rounded-xl bg-[#c4a44a]/[0.06] border border-[#c4a44a]/[0.1] p-3">
                    <SparklesIcon className="size-5 text-[#c4a44a]/40" />
                  </div>
                  <p className="text-[11px] text-muted-foreground/50 max-w-[160px] leading-relaxed">
                    No addons yet — search or pick from your installed addons above.
                  </p>
                </div>
              ) : (
                <div className="space-y-1.5">
                  {addons.map((addon) => (
                    <div
                      key={addon.esouiId}
                      className={cn(
                        "group/item rounded-lg p-2 transition-all duration-150",
                        "border border-white/[0.04] bg-white/[0.02]",
                        "hover:bg-white/[0.04] hover:border-white/[0.08]"
                      )}
                    >
                      <div className="flex items-center gap-1.5 mb-1.5">
                        <p className="text-[11px] font-medium truncate flex-1">{addon.name}</p>
                        <button
                          onClick={() => handleRemoveAddon(addon.esouiId)}
                          className="text-muted-foreground/20 hover:text-red-400 transition-colors p-0.5 opacity-0 group-hover/item:opacity-100"
                          title="Remove"
                        >
                          <XIcon className="size-3" />
                        </button>
                      </div>
                      {/* Required / Optional toggle pill */}
                      <div className="relative flex p-0.5 rounded-md bg-white/[0.03] border border-white/[0.06]">
                        <div
                          className={cn(
                            "absolute top-0.5 bottom-0.5 rounded-[5px] transition-all duration-200 ease-[cubic-bezier(0.34,1.56,0.64,1)]",
                            addon.required
                              ? "left-0.5 w-[calc(50%-2px)] bg-[#c4a44a]/20 border border-[#c4a44a]/30"
                              : "left-[calc(50%)] w-[calc(50%-2px)] bg-white/[0.06] border border-white/[0.08]"
                          )}
                        />
                        <button
                          onClick={() => handleToggleRequired(addon.esouiId)}
                          className={cn(
                            "relative z-10 flex-1 text-[10px] font-semibold py-0.5 rounded-[5px] transition-colors duration-150 text-center",
                            addon.required
                              ? "text-[#c4a44a]"
                              : "text-muted-foreground/40 hover:text-muted-foreground/60"
                          )}
                        >
                          Required
                        </button>
                        <button
                          onClick={() => handleToggleRequired(addon.esouiId)}
                          className={cn(
                            "relative z-10 flex-1 text-[10px] font-semibold py-0.5 rounded-[5px] transition-colors duration-150 text-center",
                            !addon.required
                              ? "text-foreground/70"
                              : "text-muted-foreground/40 hover:text-muted-foreground/60"
                          )}
                        >
                          Optional
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>

          {/* Save / Publish actions */}
          <div className="flex flex-col gap-2 mt-1">
            {/* Save to file — always available, no auth needed */}
            <Button
              variant="outline"
              onClick={handleSaveToFile}
              disabled={addons.length === 0 || savingToFile}
              className="w-full"
            >
              {savingToFile ? (
                <>
                  <Loader2Icon className="size-4 animate-spin mr-1.5" />
                  Saving...
                </>
              ) : (
                <>
                  <FileDownIcon className="size-4 mr-1.5" />
                  Save to File
                </>
              )}
            </Button>

            {/* Divider */}
            <div className="flex items-center gap-3">
              <div className="flex-1 border-t border-white/[0.06]" />
              <span className="text-[10px] text-muted-foreground/40 uppercase tracking-wider">
                or
              </span>
              <div className="flex-1 border-t border-white/[0.06]" />
            </div>

            {/* Save as Draft + Publish — both require auth */}
            {authUser ? (
              <>
                <label className="flex items-center gap-2 text-xs text-muted-foreground/60 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={isAnonymous}
                    onChange={(e) => setIsAnonymous(e.target.checked)}
                    className="rounded border-white/20 bg-white/[0.03] accent-[#c4a44a]"
                  />
                  Publish anonymously
                </label>
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    onClick={handleSaveDraft}
                    disabled={addons.length === 0 || publishing}
                    className="flex-1"
                  >
                    {publishing ? <Loader2Icon className="size-4 animate-spin" /> : "Save as Draft"}
                  </Button>
                  <Button
                    onClick={handlePublish}
                    disabled={addons.length === 0 || publishing}
                    className="flex-1"
                  >
                    {publishing ? (
                      <Loader2Icon className="size-4 animate-spin" />
                    ) : editingPackId ? (
                      "Save Changes"
                    ) : (
                      <>
                        <ArrowUpIcon className="size-4 mr-1.5" />
                        Publish
                      </>
                    )}
                  </Button>
                </div>
              </>
            ) : (
              <Button
                variant="outline"
                onClick={handleLogin}
                disabled={loggingIn}
                className="w-full"
              >
                {loggingIn ? (
                  <>
                    <Loader2Icon className="size-4 animate-spin mr-1.5" />
                    Signing in...
                  </>
                ) : (
                  "Sign in to save or publish"
                )}
              </Button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
