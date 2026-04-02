import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { toast } from "sonner";
import type {
  AddonManifest,
  CharacterInfo,
  SavedVariableFile,
  SvTreeNode,
  EffectiveField,
  SvSchemaOverlay,
  WidgetType,
  WidgetOverride,
  WidgetProps,
} from "../types";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Popover, PopoverTrigger, PopoverContent, PopoverTitle } from "@/components/ui/popover";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { getSetting, setSetting } from "@/lib/store";
import { classifyContext, humanizeKey, getTableChildren, getLeafChildren } from "@/lib/sv-nodes";
import { resolveEffectiveField } from "@/lib/sv-widgets";
import {
  RefreshCwIcon,
  ChevronRightIcon,
  ChevronDownIcon,
  FileTextIcon,
  CopyIcon,
  AlertTriangleIcon,
  BracesIcon,
  Trash2Icon,
  ShieldCheckIcon,
  HardDriveIcon,
  PackageXIcon,
  ArrowUpDownIcon,
  CheckIcon,
  SearchIcon,
  SettingsIcon,
  EyeOffIcon,
  LockIcon,
  RotateCcwIcon,
  MinusIcon,
  PlusIcon,
} from "lucide-react";

interface SavedVariablesProps {
  addonsPath: string;
  installedAddons: AddonManifest[];
  onClose: () => void;
}

// ─── ESO system SV files (no addon folder, but not orphaned) ─
const SYSTEM_SV_NAMES = new Set([
  "ZO_Ingame",
  "ZO_InternalIngame",
  "ZO_Pregame",
  "AccountSettings",
  "GuildHistoryCache",
]);

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDate(iso: string): string {
  if (!iso) return "";
  try {
    const d = new Date(iso);
    return d.toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  } catch {
    return iso;
  }
}

/** Classify a SV file against the installed addons list. */
function classifyFile(
  f: SavedVariableFile,
  installedFolders: Set<string>
): "installed" | "system" | "orphaned" {
  if (SYSTEM_SV_NAMES.has(f.addonName)) return "system";
  // Exact match: "LibAddonMenu-2.0.lua" -> "LibAddonMenu-2.0" folder
  if (installedFolders.has(f.addonName)) return "installed";
  // Prefix match: "HarvestMapAD.lua" -> "HarvestMap" folder
  // Also handles "CombatMetricsFightData.lua" -> "CombatMetrics" folder
  // Require the folder name to be at least 4 chars to prevent short names
  // like "Lib" from over-matching, and require the character at the boundary
  // to be uppercase (sub-file naming convention) or non-alphanumeric.
  for (const folder of installedFolders) {
    if (
      folder.length >= 4 &&
      f.addonName.startsWith(folder) &&
      f.addonName.length > folder.length
    ) {
      const boundaryChar = f.addonName[folder.length];
      if (!boundaryChar || /[A-Z_-]/.test(boundaryChar)) {
        return "installed";
      }
    }
  }
  return "orphaned";
}

type SizeCategory = "small" | "medium" | "large";

function sizeCategory(bytes: number): SizeCategory {
  if (bytes >= 5 * 1024 * 1024) return "large";
  if (bytes >= 1024 * 1024) return "medium";
  return "small";
}

const SIZE_COLORS: Record<SizeCategory, string> = {
  small: "text-emerald-400",
  medium: "text-amber-400",
  large: "text-red-400",
};

const SIZE_BAR_COLORS: Record<SizeCategory, string> = {
  small: "bg-emerald-500/40",
  medium: "bg-amber-500/40",
  large: "bg-red-500/40",
};

// ─── Tree update helper ──────────────────────────────────────

function updateTreeNode(
  tree: SvTreeNode,
  path: string[],
  value: string | number | boolean | null,
  depth = 0
): SvTreeNode {
  if (depth >= path.length || !tree.children) return tree;

  const targetKey = path[depth];
  const isLeaf = depth === path.length - 1;

  return {
    ...tree,
    children: tree.children.map((child) => {
      if (child.key !== targetKey) return child;
      if (isLeaf) {
        return { ...child, value: value };
      }
      return updateTreeNode(child, path, value, depth + 1);
    }),
  };
}

// ─── Overview Tab ───────────────────────────────────────────

type OverviewSort = "name" | "size" | "date";
type OverviewFilter = "all" | "orphaned" | "large";

function OverviewTab({
  files,
  loading,
  installedFolders,
  onRefresh,
  onSelectFile,
  onSwitchToCleanup,
}: {
  files: SavedVariableFile[];
  loading: boolean;
  installedFolders: Set<string>;
  onRefresh: () => void;
  onSelectFile: (f: SavedVariableFile) => void;
  onSwitchToCleanup: () => void;
}) {
  const [sortBy, setSortBy] = useState<OverviewSort>("size");
  const [filter, setFilter] = useState<OverviewFilter>("all");

  const classified = useMemo(
    () =>
      files.map((f) => ({
        file: f,
        status: classifyFile(f, installedFolders),
        sizeCategory: sizeCategory(f.sizeBytes),
      })),
    [files, installedFolders]
  );

  const totalSize = useMemo(() => files.reduce((sum, f) => sum + f.sizeBytes, 0), [files]);
  const maxSize = useMemo(() => Math.max(...files.map((f) => f.sizeBytes), 1), [files]);
  const orphaned = useMemo(() => classified.filter((c) => c.status === "orphaned"), [classified]);
  const orphanedSize = useMemo(
    () => orphaned.reduce((sum, c) => sum + c.file.sizeBytes, 0),
    [orphaned]
  );
  const largeFiles = useMemo(
    () => classified.filter((c) => c.sizeCategory === "large"),
    [classified]
  );

  const filtered = useMemo(() => {
    let items = classified;
    if (filter === "orphaned") items = items.filter((c) => c.status === "orphaned");
    if (filter === "large") items = items.filter((c) => c.sizeCategory === "large");
    return items;
  }, [classified, filter]);

  const sorted = useMemo(() => {
    const copy = [...filtered];
    switch (sortBy) {
      case "size":
        copy.sort((a, b) => b.file.sizeBytes - a.file.sizeBytes);
        break;
      case "name":
        copy.sort((a, b) =>
          a.file.addonName.toLowerCase().localeCompare(b.file.addonName.toLowerCase())
        );
        break;
      case "date":
        copy.sort((a, b) => b.file.lastModified.localeCompare(a.file.lastModified));
        break;
    }
    return copy;
  }, [filtered, sortBy]);

  return (
    <div className="space-y-3">
      {/* Summary stats */}
      <div className="grid grid-cols-3 gap-2">
        <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-2.5 text-center">
          <div className="text-lg font-heading font-semibold">{files.length}</div>
          <div className="text-[10px] uppercase tracking-wider text-muted-foreground">Files</div>
        </div>
        <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-2.5 text-center">
          <div className="text-lg font-heading font-semibold">{formatBytes(totalSize)}</div>
          <div className="text-[10px] uppercase tracking-wider text-muted-foreground">
            Total Size
          </div>
        </div>
        <button
          onClick={orphaned.length > 0 ? onSwitchToCleanup : undefined}
          className={`rounded-lg border p-2.5 text-center transition-colors ${
            orphaned.length > 0
              ? "border-amber-500/20 bg-amber-500/[0.05] hover:border-amber-500/30 cursor-pointer"
              : "border-white/[0.06] bg-white/[0.02]"
          }`}
        >
          <div
            className={`text-lg font-heading font-semibold ${orphaned.length > 0 ? "text-amber-400" : ""}`}
          >
            {orphaned.length}
          </div>
          <div className="text-[10px] uppercase tracking-wider text-muted-foreground">Orphaned</div>
        </button>
      </div>

      {/* Actionable insight banner */}
      {orphaned.length > 0 && (
        <button
          onClick={onSwitchToCleanup}
          className="flex w-full items-center gap-2 rounded-lg border border-amber-500/20 bg-amber-500/[0.06] p-2.5 text-left text-xs text-amber-300 transition-colors hover:border-amber-500/30"
        >
          <PackageXIcon className="size-4 shrink-0" />
          <span>
            <strong>{orphaned.length} orphaned files</strong> ({formatBytes(orphanedSize)}) from
            uninstalled addons.{" "}
            <span className="text-amber-400/80 underline underline-offset-2">Clean up &rarr;</span>
          </span>
        </button>
      )}

      {largeFiles.length > 0 && orphaned.length === 0 && (
        <div className="flex items-center gap-2 rounded-lg border border-white/[0.06] bg-white/[0.02] p-2.5 text-xs text-muted-foreground">
          <HardDriveIcon className="size-4 shrink-0 text-red-400" />
          <span>
            <strong className="text-red-400">{largeFiles.length} large files</strong> (&gt;5 MB) may
            slow down your game loading times.
          </span>
        </div>
      )}

      {/* Sort & filter controls */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1">
          {(["all", "orphaned", "large"] as const).map((f) => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              className={`rounded-md px-2 py-0.5 text-xs transition-colors ${
                filter === f
                  ? "bg-white/[0.1] text-foreground"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              {f === "all"
                ? "All"
                : f === "orphaned"
                  ? `Orphaned (${orphaned.length})`
                  : `Large (${largeFiles.length})`}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() =>
              setSortBy((prev) => (prev === "size" ? "name" : prev === "name" ? "date" : "size"))
            }
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
          >
            <ArrowUpDownIcon className="size-3" />
            {sortBy === "size" ? "Size" : sortBy === "name" ? "Name" : "Date"}
          </button>
          <Button size="sm" variant="outline" onClick={onRefresh} disabled={loading}>
            <RefreshCwIcon className={`mr-1 size-3 ${loading ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        </div>
      </div>

      {/* File list */}
      <div className="max-h-[320px] overflow-y-auto space-y-1">
        {sorted.length === 0 ? (
          <div className="py-8 text-center">
            <FileTextIcon className="mx-auto mb-2 size-8 text-muted-foreground/30" />
            <p className="text-sm text-muted-foreground">
              {filter !== "all" ? "No files match this filter." : "No SavedVariables files found."}
            </p>
            {filter === "all" && (
              <p className="mt-1 text-xs text-muted-foreground/60">
                Make sure your ESO AddOns path is set correctly and you have launched the game at
                least once.
              </p>
            )}
          </div>
        ) : (
          sorted.map(({ file: f, status, sizeCategory: sc }) => (
            <button
              key={f.fileName}
              onClick={() => onSelectFile(f)}
              className="flex w-full items-center gap-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-2.5 text-left transition-all duration-200 hover:border-white/[0.1]"
            >
              {/* Size bar */}
              <div className="w-14 shrink-0">
                <div className="h-1.5 w-full rounded-full bg-white/[0.06] overflow-hidden">
                  <div
                    className={`h-full rounded-full ${SIZE_BAR_COLORS[sc]}`}
                    style={{ width: `${Math.max((f.sizeBytes / maxSize) * 100, 2)}%` }}
                  />
                </div>
                <div className={`mt-0.5 text-[10px] text-center ${SIZE_COLORS[sc]}`}>
                  {formatBytes(f.sizeBytes)}
                </div>
              </div>

              {/* Name & meta */}
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5">
                  <span className="truncate text-sm font-medium">{f.addonName}</span>
                  {status === "orphaned" && <InfoPill color="amber">Orphaned</InfoPill>}
                  {status === "system" && <InfoPill color="muted">System</InfoPill>}
                </div>
                <div className="mt-0.5 text-[11px] text-muted-foreground">
                  {formatDate(f.lastModified)}
                  {f.characterKeys.length > 0 && (
                    <span>
                      {" "}
                      &middot; {f.characterKeys.length} profile
                      {f.characterKeys.length !== 1 ? "s" : ""}
                    </span>
                  )}
                </div>
              </div>

              <ChevronRightIcon className="size-4 shrink-0 text-muted-foreground/40" />
            </button>
          ))
        )}
      </div>
    </div>
  );
}

// ─── Cleanup Tab ────────────────────────────────────────────

function CleanupTab({
  files,
  installedFolders,
  addonsPath,
  onRefresh,
}: {
  files: SavedVariableFile[];
  installedFolders: Set<string>;
  addonsPath: string;
  onRefresh: () => void;
}) {
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState(false);
  const [confirmState, setConfirmState] = useState<ConfirmState | null>(null);

  const orphaned = useMemo(
    () => files.filter((f) => classifyFile(f, installedFolders) === "orphaned"),
    [files, installedFolders]
  );

  const selectedSize = useMemo(
    () => orphaned.filter((f) => selected.has(f.fileName)).reduce((s, f) => s + f.sizeBytes, 0),
    [orphaned, selected]
  );

  const toggleFile = (fileName: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(fileName)) next.delete(fileName);
      else next.add(fileName);
      return next;
    });
  };

  const toggleAll = () => {
    if (selected.size === orphaned.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(orphaned.map((f) => f.fileName)));
    }
  };

  const doDelete = async (fileNames: string[], size: number) => {
    setDeleting(true);
    try {
      const deleted = await invokeOrThrow<number>("delete_saved_variables", {
        addonsPath,
        fileNames,
      });
      toast.success(
        `Cleaned up ${deleted} file${deleted !== 1 ? "s" : ""} (${formatBytes(size)}). Backup saved.`
      );
      setSelected(new Set());
      onRefresh();
    } catch (e) {
      toast.error(`Cleanup failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setDeleting(false);
    }
  };

  const handleDelete = () => {
    if (selected.size === 0) return;
    const fileNames = [...selected];
    const size = selectedSize;
    setConfirmState({
      title: "Delete orphaned files?",
      description: `Delete ${fileNames.length} orphaned file${fileNames.length !== 1 ? "s" : ""} (${formatBytes(size)})? A backup will be created automatically before deletion.`,
      confirmLabel: "Delete",
      onConfirm: () => void doDelete(fileNames, size),
    });
  };

  const largeFiles = useMemo(
    () =>
      files.filter((f) => f.sizeBytes >= 5 * 1024 * 1024).sort((a, b) => b.sizeBytes - a.sizeBytes),
    [files]
  );

  if (orphaned.length === 0 && largeFiles.length === 0) {
    return (
      <div className="py-10 text-center">
        <ShieldCheckIcon className="mx-auto mb-3 size-10 text-emerald-500/60" />
        <p className="text-sm font-medium text-emerald-400">Your SavedVariables are clean</p>
        <p className="mt-1 text-xs text-muted-foreground">
          No orphaned files or oversized data detected.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Orphaned files section */}
      {orphaned.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-2">
            <div className="flex items-center gap-2">
              <PackageXIcon className="size-4 text-amber-400" />
              <span className="text-sm font-medium">
                Orphaned Files
                <span className="ml-1 text-muted-foreground font-normal">
                  ({orphaned.length} files,{" "}
                  {formatBytes(orphaned.reduce((s, f) => s + f.sizeBytes, 0))})
                </span>
              </span>
            </div>
            <button
              onClick={toggleAll}
              className="text-xs text-muted-foreground hover:text-foreground"
            >
              {selected.size === orphaned.length ? "Deselect all" : "Select all"}
            </button>
          </div>

          <p className="text-xs text-muted-foreground mb-2">
            These files belong to addons that are no longer installed. They waste disk space and
            slow down game loading.
          </p>

          <div className="max-h-[220px] overflow-y-auto space-y-1">
            {orphaned.map((f) => (
              <label
                key={f.fileName}
                className="flex items-center gap-2.5 rounded-lg border border-white/[0.06] bg-white/[0.02] p-2 cursor-pointer transition-colors hover:border-white/[0.1]"
              >
                <div
                  className={`flex size-4 shrink-0 items-center justify-center rounded border transition-colors ${
                    selected.has(f.fileName)
                      ? "border-[#c4a44a] bg-[#c4a44a]"
                      : "border-white/[0.2] bg-transparent"
                  }`}
                >
                  {selected.has(f.fileName) && <CheckIcon className="size-3 text-black" />}
                </div>
                <input
                  type="checkbox"
                  className="sr-only"
                  checked={selected.has(f.fileName)}
                  onChange={() => toggleFile(f.fileName)}
                />
                <div className="min-w-0 flex-1">
                  <div className="text-sm truncate">{f.addonName}</div>
                  <div className="text-[11px] text-muted-foreground">
                    {formatBytes(f.sizeBytes)} &middot; {formatDate(f.lastModified)}
                  </div>
                </div>
              </label>
            ))}
          </div>

          {/* Delete action */}
          <div className="mt-3 flex items-center justify-between rounded-lg border border-white/[0.06] bg-white/[0.02] p-2.5">
            <div className="text-xs text-muted-foreground">
              {selected.size > 0 ? (
                <span>
                  <strong className="text-foreground">{selected.size}</strong> selected &middot;{" "}
                  <strong className="text-[#c4a44a]">{formatBytes(selectedSize)}</strong>{" "}
                  reclaimable
                </span>
              ) : (
                "Select files to clean up"
              )}
            </div>
            <Button
              size="sm"
              onClick={() => void handleDelete()}
              disabled={selected.size === 0 || deleting}
            >
              <Trash2Icon className="mr-1 size-3" />
              {deleting
                ? "Cleaning..."
                : `Clean Up${selected.size > 0 ? ` (${formatBytes(selectedSize)})` : ""}`}
            </Button>
          </div>
        </div>
      )}

      {/* Large files advisory */}
      {largeFiles.length > 0 && (
        <div>
          <div className="flex items-center gap-2 mb-2">
            <HardDriveIcon className="size-4 text-red-400" />
            <span className="text-sm font-medium">
              Large Files
              <span className="ml-1 text-muted-foreground font-normal">
                (&gt;5 MB, may slow loading)
              </span>
            </span>
          </div>

          <div className="space-y-1">
            {largeFiles.map((f) => (
              <div
                key={f.fileName}
                className="flex items-center justify-between rounded-lg border border-white/[0.06] bg-white/[0.02] p-2"
              >
                <div className="min-w-0">
                  <div className="text-sm truncate">{f.addonName}</div>
                  {f.characterKeys.length > 0 && (
                    <div className="text-[11px] text-muted-foreground">
                      {f.characterKeys.length} profile{f.characterKeys.length !== 1 ? "s" : ""}
                    </div>
                  )}
                </div>
                <span className="text-sm font-medium text-red-400 shrink-0 ml-2">
                  {formatBytes(f.sizeBytes)}
                </span>
              </div>
            ))}
          </div>

          <p className="mt-2 text-xs text-muted-foreground">
            Large SavedVariables are loaded into memory every time you log in. If loading feels
            slow, consider clearing data in-game or checking if these addons have a "purge old data"
            option.
          </p>
        </div>
      )}

      <ConfirmDialog state={confirmState} onClose={() => setConfirmState(null)} />
    </div>
  );
}

// ─── Editor Tab v2 — Two-panel layout with smart controls ───

const OVERLAY_STORE_KEY = "sv-schema-overlay";

// ── Nav Tree Item ────────────────────────────────────────────

function NavTreeItem({
  node,
  depth,
  selectedPath,
  onSelect,
  searchQuery,
  knownCharacters,
  expandedPaths,
  toggleExpanded,
}: {
  node: SvTreeNode;
  depth: number;
  selectedPath: string[];
  onSelect: (path: string[], node: SvTreeNode) => void;
  searchQuery: string;
  knownCharacters: Set<string>;
  expandedPaths: Set<string>;
  toggleExpanded: (pathKey: string) => void;
}) {
  const pathKey = [...selectedPath.slice(0, depth), node.key].join("/");
  const isExpanded = expandedPaths.has(pathKey);
  const tableChildren = getTableChildren(node);
  const entryCount = node.children?.length ?? 0;
  const isSelected =
    selectedPath.length > depth &&
    selectedPath[depth] === node.key &&
    selectedPath.length === depth + 1;

  const context = classifyContext(node.key, depth, knownCharacters);
  const matchesSearch = !searchQuery || node.key.toLowerCase().includes(searchQuery.toLowerCase());

  // Also check if any descendant matches
  const hasMatchingDescendant = useMemo(() => {
    if (!searchQuery) return true;
    const check = (n: SvTreeNode): boolean => {
      if (n.key.toLowerCase().includes(searchQuery.toLowerCase())) return true;
      return (n.children ?? []).some(check);
    };
    return check(node);
  }, [node, searchQuery]);

  if (!matchesSearch && !hasMatchingDescendant) return null;

  const currentPath = [...selectedPath.slice(0, depth), node.key];

  return (
    <div>
      <button
        onClick={() => {
          onSelect(currentPath, node);
          if (tableChildren.length > 0) toggleExpanded(pathKey);
        }}
        className={`flex w-full items-center gap-1.5 rounded-lg px-2 py-1 text-left text-xs transition-colors ${
          isSelected
            ? "bg-white/[0.08] text-foreground"
            : "text-foreground/70 hover:bg-white/[0.04] hover:text-foreground"
        }`}
        style={{ paddingLeft: `${depth * 14 + 8}px` }}
      >
        {tableChildren.length > 0 ? (
          isExpanded ? (
            <ChevronDownIcon className="size-3 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronRightIcon className="size-3 shrink-0 text-muted-foreground" />
          )
        ) : (
          <div className="size-3 shrink-0" />
        )}
        <span className="truncate font-medium">{node.key}</span>
        <span className="ml-auto shrink-0 text-[10px] text-muted-foreground/50">{entryCount}</span>
        {context === "account-wide" && (
          <InfoPill color="sky" className="ml-1 !text-[9px] !px-1 !py-0">
            Account
          </InfoPill>
        )}
        {context === "per-character" && (
          <InfoPill color="emerald" className="ml-1 !text-[9px] !px-1 !py-0">
            Char
          </InfoPill>
        )}
      </button>
      {isExpanded &&
        tableChildren.map((child, i) => (
          <NavTreeItem
            key={`${child.key}-${i}`}
            node={child}
            depth={depth + 1}
            selectedPath={selectedPath}
            onSelect={onSelect}
            searchQuery={searchQuery}
            knownCharacters={knownCharacters}
            expandedPaths={expandedPaths}
            toggleExpanded={toggleExpanded}
          />
        ))}
    </div>
  );
}

// ── Widget Controls ──────────────────────────────────────────

function ToggleControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: boolean) => void;
}) {
  const checked = field.value === true;
  return (
    <button
      onClick={() => !field.readOnly && onChange(!checked)}
      className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors ${
        field.readOnly ? "opacity-50 cursor-not-allowed" : "cursor-pointer"
      } ${checked ? "bg-[#c4a44a]" : "bg-white/[0.12]"}`}
      aria-label={`${field.label}: ${checked ? "on" : "off"}`}
    >
      <span
        className={`inline-block size-3.5 rounded-full bg-white shadow transition-transform ${
          checked ? "translate-x-[18px]" : "translate-x-[3px]"
        }`}
      />
    </button>
  );
}

function NumberControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: number) => void;
}) {
  const fieldVal = String(field.value ?? 0);
  const [localValue, setLocalValue] = useState(fieldVal);
  const [prevFieldVal, setPrevFieldVal] = useState(fieldVal);
  if (prevFieldVal !== fieldVal) {
    setPrevFieldVal(fieldVal);
    setLocalValue(fieldVal);
  }

  const commit = () => {
    const num = Number(localValue);
    if (!isNaN(num)) onChange(num);
    else setLocalValue(String(field.value ?? 0));
  };

  const step = field.props.step ?? 1;

  return (
    <div className="flex items-center gap-1">
      <button
        onClick={() => onChange((Number(field.value) || 0) - step)}
        className="flex size-6 items-center justify-center rounded border border-white/[0.08] bg-white/[0.04] text-muted-foreground hover:bg-white/[0.08] hover:text-foreground"
        disabled={field.readOnly}
      >
        <MinusIcon className="size-3" />
      </button>
      <input
        type="number"
        className="w-20 rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none focus:border-[#38bdf8]/50 focus:ring-1 focus:ring-[#38bdf8]/30"
        value={localValue}
        onChange={(e) => setLocalValue(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
        disabled={field.readOnly}
        step={step}
        min={field.props.min}
        max={field.props.max}
      />
      <button
        onClick={() => onChange((Number(field.value) || 0) + step)}
        className="flex size-6 items-center justify-center rounded border border-white/[0.08] bg-white/[0.04] text-muted-foreground hover:bg-white/[0.08] hover:text-foreground"
        disabled={field.readOnly}
      >
        <PlusIcon className="size-3" />
      </button>
    </div>
  );
}

function SliderControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: number) => void;
}) {
  const min = field.props.min ?? 0;
  const max = field.props.max ?? 100;
  const step = field.props.step ?? 1;
  const value = Number(field.value) || min;

  return (
    <div className="flex items-center gap-2">
      <input
        type="range"
        className="h-1.5 flex-1 cursor-pointer appearance-none rounded-full bg-white/[0.1] accent-[#c4a44a]"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        disabled={field.readOnly}
      />
      <span className="w-10 text-right text-xs text-muted-foreground tabular-nums">{value}</span>
    </div>
  );
}

function ColorControl({
  field,
  originalNode,
  onChangeColor,
}: {
  field: EffectiveField;
  originalNode: SvTreeNode | null;
  onChangeColor: (r: number, g: number, b: number, a?: number) => void;
}) {
  // Extract r,g,b,a from children
  const children = originalNode?.children ?? [];
  const getVal = (key: string) => {
    const c = children.find((ch) => ch.key === key);
    return c ? Number(c.value ?? 0) : 0;
  };
  const r = getVal("r");
  const g = getVal("g");
  const b = getVal("b");
  const a = children.some((ch) => ch.key === "a") ? getVal("a") : undefined;

  const hexColor = `#${Math.round(r * 255)
    .toString(16)
    .padStart(2, "0")}${Math.round(g * 255)
    .toString(16)
    .padStart(2, "0")}${Math.round(b * 255)
    .toString(16)
    .padStart(2, "0")}`;

  return (
    <div className="flex items-center gap-2">
      <input
        type="color"
        value={hexColor}
        onChange={(e) => {
          const hex = e.target.value;
          const nr = parseInt(hex.slice(1, 3), 16) / 255;
          const ng = parseInt(hex.slice(3, 5), 16) / 255;
          const nb = parseInt(hex.slice(5, 7), 16) / 255;
          onChangeColor(nr, ng, nb, a);
        }}
        className="size-7 cursor-pointer rounded border border-white/[0.1] bg-transparent p-0"
        disabled={field.readOnly}
      />
      <span className="text-xs text-muted-foreground font-mono">{hexColor}</span>
      {a !== undefined && (
        <span className="text-xs text-muted-foreground/60">a: {a.toFixed(2)}</span>
      )}
    </div>
  );
}

function TextControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: string) => void;
}) {
  const fieldVal = String(field.value ?? "");
  const [localValue, setLocalValue] = useState(fieldVal);
  const [prevFieldVal, setPrevFieldVal] = useState(fieldVal);
  if (prevFieldVal !== fieldVal) {
    setPrevFieldVal(fieldVal);
    setLocalValue(fieldVal);
  }

  const commit = () => onChange(localValue);

  if (field.props.multiline) {
    return (
      <textarea
        className="w-full rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1.5 text-xs text-foreground outline-none focus:border-[#38bdf8]/50 focus:ring-1 focus:ring-[#38bdf8]/30 resize-y"
        rows={3}
        value={localValue}
        onChange={(e) => setLocalValue(e.target.value)}
        onBlur={commit}
        disabled={field.readOnly}
      />
    );
  }

  return (
    <input
      type="text"
      className="w-full max-w-xs rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none focus:border-[#38bdf8]/50 focus:ring-1 focus:ring-[#38bdf8]/30"
      value={localValue}
      onChange={(e) => setLocalValue(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && commit()}
      disabled={field.readOnly}
    />
  );
}

function DropdownControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: string) => void;
}) {
  const options = field.props.options ?? [];
  return (
    <select
      className="rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none focus:border-[#38bdf8]/50"
      value={String(field.value ?? "")}
      onChange={(e) => onChange(e.target.value)}
      disabled={field.readOnly}
    >
      {options.map((opt) => (
        <option key={opt} value={opt}>
          {opt}
        </option>
      ))}
      {!options.includes(String(field.value ?? "")) && (
        <option value={String(field.value ?? "")}>{String(field.value ?? "")}</option>
      )}
    </select>
  );
}

function ReadonlyControl({ field }: { field: EffectiveField }) {
  return (
    <span className="text-xs text-muted-foreground/60 italic">
      {field.value === null ? "nil" : String(field.value)}
    </span>
  );
}

function RawControl({
  field,
  onChange,
}: {
  field: EffectiveField;
  onChange: (val: string) => void;
}) {
  const fieldVal = String(field.value ?? "");
  const [localValue, setLocalValue] = useState(fieldVal);
  const [prevFieldVal, setPrevFieldVal] = useState(fieldVal);
  if (prevFieldVal !== fieldVal) {
    setPrevFieldVal(fieldVal);
    setLocalValue(fieldVal);
  }

  return (
    <textarea
      className="w-full rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 py-1.5 font-mono text-xs text-foreground outline-none focus:border-[#38bdf8]/50 focus:ring-1 focus:ring-[#38bdf8]/30 resize-y"
      rows={2}
      value={localValue}
      onChange={(e) => setLocalValue(e.target.value)}
      onBlur={() => onChange(localValue)}
      disabled={field.readOnly}
    />
  );
}

// ── Customization Popover ────────────────────────────────────

function WidgetCustomizer({
  field,
  overlay,
  addonName,
  onSave,
}: {
  field: EffectiveField;
  overlay: SvSchemaOverlay;
  addonName: string;
  onSave: (overlay: SvSchemaOverlay) => void;
}) {
  const existing = overlay[addonName]?.[field.nodeId];
  const [widgetType, setWidgetType] = useState<WidgetType | "">(existing?.widget ?? "");
  const [label, setLabel] = useState(existing?.label ?? "");
  const [hidden, setHidden] = useState(existing?.hidden ?? false);
  const [readOnly, setReadOnly] = useState(existing?.readOnly ?? false);
  const [min, setMin] = useState(String(existing?.props?.min ?? ""));
  const [max, setMax] = useState(String(existing?.props?.max ?? ""));
  const [step, setStep] = useState(String(existing?.props?.step ?? ""));
  const [options, setOptions] = useState((existing?.props?.options ?? []).join(", "));

  const handleSave = () => {
    const override: WidgetOverride = {};
    if (widgetType) override.widget = widgetType;
    if (label.trim()) override.label = label.trim();
    if (hidden) override.hidden = true;
    if (readOnly) override.readOnly = true;

    const props: Partial<WidgetProps> = {};
    if (min !== "") props.min = Number(min);
    if (max !== "") props.max = Number(max);
    if (step !== "") props.step = Number(step);
    if (options.trim())
      props.options = options
        .split(",")
        .map((o) => o.trim())
        .filter(Boolean);
    if (Object.keys(props).length > 0) override.props = props;

    const next = { ...overlay };
    if (!next[addonName]) next[addonName] = {};
    if (Object.keys(override).length > 0) {
      next[addonName] = { ...next[addonName], [field.nodeId]: override };
    } else {
      const { [field.nodeId]: _, ...rest } = next[addonName];
      next[addonName] = rest;
    }

    onSave(next);
  };

  const handleReset = () => {
    const next = { ...overlay };
    if (next[addonName]) {
      const { [field.nodeId]: _, ...rest } = next[addonName];
      next[addonName] = rest;
    }
    onSave(next);
  };

  return (
    <div className="space-y-2.5">
      <PopoverTitle>Customize Widget</PopoverTitle>

      <div>
        <label className="text-[10px] uppercase tracking-wider text-muted-foreground">
          Widget Type
        </label>
        <select
          className="mt-0.5 w-full rounded border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none"
          value={widgetType}
          onChange={(e) => setWidgetType((e.target.value || "") as WidgetType | "")}
        >
          <option value="">Auto-detect</option>
          <option value="text">Text</option>
          <option value="number">Number</option>
          <option value="toggle">Toggle</option>
          <option value="slider">Slider</option>
          <option value="color">Color</option>
          <option value="dropdown">Dropdown</option>
          <option value="readonly">Read-only</option>
          <option value="raw">Raw</option>
        </select>
      </div>

      {(widgetType === "slider" || widgetType === "") && (
        <div className="flex gap-2">
          <div className="flex-1">
            <label className="text-[10px] uppercase tracking-wider text-muted-foreground">
              Min
            </label>
            <input
              type="number"
              className="mt-0.5 w-full rounded border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none"
              value={min}
              onChange={(e) => setMin(e.target.value)}
              placeholder="—"
            />
          </div>
          <div className="flex-1">
            <label className="text-[10px] uppercase tracking-wider text-muted-foreground">
              Max
            </label>
            <input
              type="number"
              className="mt-0.5 w-full rounded border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none"
              value={max}
              onChange={(e) => setMax(e.target.value)}
              placeholder="—"
            />
          </div>
          <div className="flex-1">
            <label className="text-[10px] uppercase tracking-wider text-muted-foreground">
              Step
            </label>
            <input
              type="number"
              className="mt-0.5 w-full rounded border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none"
              value={step}
              onChange={(e) => setStep(e.target.value)}
              placeholder="1"
            />
          </div>
        </div>
      )}

      {widgetType === "dropdown" && (
        <div>
          <label className="text-[10px] uppercase tracking-wider text-muted-foreground">
            Options (comma-separated)
          </label>
          <input
            type="text"
            className="mt-0.5 w-full rounded border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none"
            value={options}
            onChange={(e) => setOptions(e.target.value)}
            placeholder="option1, option2, option3"
          />
        </div>
      )}

      <div>
        <label className="text-[10px] uppercase tracking-wider text-muted-foreground">
          Label Override
        </label>
        <input
          type="text"
          className="mt-0.5 w-full rounded border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-xs text-foreground outline-none"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder={humanizeKey(field.key)}
        />
      </div>

      <div className="flex items-center gap-3">
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer">
          <input
            type="checkbox"
            checked={hidden}
            onChange={(e) => setHidden(e.target.checked)}
            className="size-3.5 accent-[#c4a44a]"
          />
          <EyeOffIcon className="size-3" />
          Hide
        </label>
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer">
          <input
            type="checkbox"
            checked={readOnly}
            onChange={(e) => setReadOnly(e.target.checked)}
            className="size-3.5 accent-[#c4a44a]"
          />
          <LockIcon className="size-3" />
          Read-only
        </label>
      </div>

      <div className="flex items-center justify-between border-t border-white/[0.06] pt-2">
        <button
          onClick={handleReset}
          className="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
        >
          <RotateCcwIcon className="size-3" />
          Reset to auto
        </button>
        <Button size="sm" onClick={handleSave}>
          Apply
        </Button>
      </div>
    </div>
  );
}

// ── Field Row (renders one EffectiveField with its widget) ───

function FieldRow({
  field,
  originalNode,
  overlay,
  addonName,
  onEdit,
  onOverlayChange,
}: {
  field: EffectiveField;
  originalNode: SvTreeNode | null;
  overlay: SvSchemaOverlay;
  addonName: string;
  onEdit: (path: string[], value: string | number | boolean | null) => void;
  onOverlayChange: (overlay: SvSchemaOverlay) => void;
}) {
  if (field.hidden) return null;

  const pathSegments = field.nodeId.split("/").slice(1); // remove addon name prefix

  const handleChange = (val: string | number | boolean | null) => {
    onEdit(pathSegments, val);
  };

  const handleColorChange = (r: number, g: number, b: number, a?: number) => {
    onEdit([...pathSegments, "r"], r);
    onEdit([...pathSegments, "g"], g);
    onEdit([...pathSegments, "b"], b);
    if (a !== undefined) onEdit([...pathSegments, "a"], a);
  };

  const renderWidget = () => {
    switch (field.widget) {
      case "toggle":
        return <ToggleControl field={field} onChange={(v) => handleChange(v)} />;
      case "number":
        return <NumberControl field={field} onChange={(v) => handleChange(v)} />;
      case "slider":
        return <SliderControl field={field} onChange={(v) => handleChange(v)} />;
      case "color":
        return (
          <ColorControl
            field={field}
            originalNode={originalNode}
            onChangeColor={handleColorChange}
          />
        );
      case "text":
        return <TextControl field={field} onChange={(v) => handleChange(v)} />;
      case "dropdown":
        return <DropdownControl field={field} onChange={(v) => handleChange(v)} />;
      case "readonly":
        return <ReadonlyControl field={field} />;
      case "raw":
        return <RawControl field={field} onChange={(v) => handleChange(v)} />;
      default:
        return <ReadonlyControl field={field} />;
    }
  };

  return (
    <div className="flex items-center gap-3 rounded-lg px-2.5 py-1.5 hover:bg-white/[0.02] group">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="text-xs font-medium text-foreground/80">{field.label}</span>
          <span className="text-[10px] font-mono text-muted-foreground/40">{field.key}</span>
        </div>
      </div>
      <div className="shrink-0">{renderWidget()}</div>
      {field.confidence !== "certain" && (
        <Popover>
          <PopoverTrigger className="opacity-0 group-hover:opacity-100 transition-opacity">
            <SettingsIcon className="size-3.5 text-muted-foreground/50 hover:text-[#38bdf8]" />
          </PopoverTrigger>
          <PopoverContent align="end" className="w-72">
            <WidgetCustomizer
              field={field}
              overlay={overlay}
              addonName={addonName}
              onSave={onOverlayChange}
            />
          </PopoverContent>
        </Popover>
      )}
    </div>
  );
}

// ── Breadcrumb Bar ───────────────────────────────────────────

function Breadcrumbs({
  path,
  knownCharacters,
  onNavigate,
}: {
  path: string[];
  knownCharacters: Set<string>;
  onNavigate: (depth: number) => void;
}) {
  return (
    <div className="flex items-center gap-1 text-xs overflow-x-auto pb-1">
      <button
        onClick={() => onNavigate(0)}
        className="shrink-0 text-muted-foreground hover:text-foreground transition-colors"
      >
        Root
      </button>
      {path.map((segment, i) => {
        const ctx = classifyContext(segment, i, knownCharacters);
        return (
          <span key={i} className="flex items-center gap-1">
            <ChevronRightIcon className="size-3 text-muted-foreground/40" />
            <button
              onClick={() => onNavigate(i + 1)}
              className={`shrink-0 transition-colors ${
                i === path.length - 1
                  ? "text-foreground font-medium"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              {segment}
            </button>
            {ctx === "account-wide" && (
              <InfoPill color="sky" className="!text-[9px] !px-1 !py-0">
                Account
              </InfoPill>
            )}
            {ctx === "per-character" && (
              <InfoPill color="emerald" className="!text-[9px] !px-1 !py-0">
                Character
              </InfoPill>
            )}
          </span>
        );
      })}
    </div>
  );
}

// ── Detail Panel (right side) ────────────────────────────────

function DetailPanel({
  selectedNode,
  selectedPath,
  tree,
  overlay,
  knownCharacters,
  onNavigate,
  onEdit,
  onOverlayChange,
  onSelectPath,
}: {
  selectedNode: SvTreeNode | null;
  selectedPath: string[];
  tree: SvTreeNode | null;
  overlay: SvSchemaOverlay;
  knownCharacters: Set<string>;
  onNavigate: (depth: number) => void;
  onEdit: (path: string[], value: string | number | boolean | null) => void;
  onOverlayChange: (overlay: SvSchemaOverlay) => void;
  onSelectPath: (path: string[], node: SvTreeNode) => void;
}) {
  const addonName = selectedPath[0] ?? "";
  const leafChildren = useMemo(
    () => (selectedNode ? getLeafChildren(selectedNode) : []),
    [selectedNode]
  );
  const tableChildren = useMemo(
    () => (selectedNode ? getTableChildren(selectedNode) : []),
    [selectedNode]
  );

  // Build effective fields for leaf children
  const effectiveFields = useMemo(() => {
    return leafChildren.map((child) => {
      const childPath = [...selectedPath, child.key];
      const ctx = classifyContext(child.key, selectedPath.length, knownCharacters);
      return resolveEffectiveField(child, childPath, ctx, overlay, addonName, knownCharacters);
    });
  }, [leafChildren, selectedPath, knownCharacters, overlay, addonName]);

  // Color table children also get form rendering
  const colorTableFields = useMemo(() => {
    return tableChildren
      .filter((child) => {
        if (!child.children || child.children.length < 3 || child.children.length > 4) return false;
        const keys = new Set(child.children.map((c) => c.key));
        return keys.has("r") && keys.has("g") && keys.has("b");
      })
      .map((child) => {
        const childPath = [...selectedPath, child.key];
        const ctx = classifyContext(child.key, selectedPath.length, knownCharacters);
        return {
          field: resolveEffectiveField(child, childPath, ctx, overlay, addonName, knownCharacters),
          node: child,
        };
      });
  }, [tableChildren, selectedPath, knownCharacters, overlay, addonName]);

  if (!selectedNode || !tree) {
    return (
      <div className="flex flex-1 items-center justify-center py-12">
        <p className="text-sm text-muted-foreground">
          Select a node from the tree to view its settings.
        </p>
      </div>
    );
  }

  // Non-color table children (navigable groups)
  const groupChildren = tableChildren.filter((child) => {
    if (!child.children || child.children.length < 3 || child.children.length > 4) return true;
    const keys = new Set(child.children.map((c) => c.key));
    return !(keys.has("r") && keys.has("g") && keys.has("b"));
  });

  // Find original nodes for field rendering
  const findOriginalNode = (key: string) =>
    selectedNode.children?.find((c) => c.key === key) ?? null;

  const visibleFields = effectiveFields.filter((f) => !f.hidden);
  const visibleColorFields = colorTableFields.filter((f) => !f.field.hidden);

  return (
    <div className="flex flex-1 flex-col min-w-0 overflow-hidden">
      <Breadcrumbs path={selectedPath} knownCharacters={knownCharacters} onNavigate={onNavigate} />

      <div className="mt-2 flex-1 overflow-y-auto space-y-3">
        {/* Leaf settings */}
        {visibleFields.length > 0 && (
          <div>
            <div className="mb-1.5 text-[10px] font-heading font-bold uppercase tracking-[0.05em] text-muted-foreground">
              Settings
            </div>
            <div className="space-y-0.5">
              {visibleFields.map((field) => (
                <FieldRow
                  key={field.nodeId}
                  field={field}
                  originalNode={findOriginalNode(field.key)}
                  overlay={overlay}
                  addonName={addonName}
                  onEdit={onEdit}
                  onOverlayChange={onOverlayChange}
                />
              ))}
            </div>
          </div>
        )}

        {/* Color fields from table children */}
        {visibleColorFields.length > 0 && (
          <div>
            <div className="mb-1.5 text-[10px] font-heading font-bold uppercase tracking-[0.05em] text-muted-foreground">
              Colors
            </div>
            <div className="space-y-0.5">
              {visibleColorFields.map(({ field, node }) => (
                <FieldRow
                  key={field.nodeId}
                  field={field}
                  originalNode={node}
                  overlay={overlay}
                  addonName={addonName}
                  onEdit={onEdit}
                  onOverlayChange={onOverlayChange}
                />
              ))}
            </div>
          </div>
        )}

        {/* Sub-groups (navigable) */}
        {groupChildren.length > 0 && (
          <div>
            <div className="mb-1.5 text-[10px] font-heading font-bold uppercase tracking-[0.05em] text-muted-foreground">
              Groups
            </div>
            <div className="grid grid-cols-2 gap-1.5">
              {groupChildren.map((child, i) => {
                const ctx = classifyContext(child.key, selectedPath.length, knownCharacters);
                const entries = child.children?.length ?? 0;
                return (
                  <button
                    key={`${child.key}-${i}`}
                    onClick={() => onSelectPath([...selectedPath, child.key], child)}
                    className="flex items-center gap-2 rounded-xl border border-white/[0.06] bg-white/[0.02] p-2.5 text-left transition-all hover:border-white/[0.1] hover:bg-white/[0.04]"
                  >
                    <BracesIcon className="size-3.5 shrink-0 text-purple-400/70" />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1">
                        <span className="truncate text-xs font-medium">{child.key}</span>
                        {ctx === "account-wide" && (
                          <InfoPill color="sky" className="!text-[9px] !px-1 !py-0">
                            Account
                          </InfoPill>
                        )}
                        {ctx === "per-character" && (
                          <InfoPill color="emerald" className="!text-[9px] !px-1 !py-0">
                            Char
                          </InfoPill>
                        )}
                      </div>
                      <span className="text-[10px] text-muted-foreground/50">
                        {entries} {entries === 1 ? "entry" : "entries"}
                      </span>
                    </div>
                    <ChevronRightIcon className="size-3.5 shrink-0 text-muted-foreground/30" />
                  </button>
                );
              })}
            </div>
          </div>
        )}

        {/* Empty state */}
        {visibleFields.length === 0 &&
          visibleColorFields.length === 0 &&
          groupChildren.length === 0 && (
            <div className="py-8 text-center">
              <EyeOffIcon className="mx-auto mb-2 size-6 text-muted-foreground/30" />
              <p className="text-xs text-muted-foreground">No visible settings in this node.</p>
            </div>
          )}
      </div>
    </div>
  );
}

// ── Editor Tab ──────────────────────────────────────────────

function EditorTab({
  files,
  addonsPath,
  initialFile,
  esoRunning,
  characters,
  onDirtyChange,
}: {
  files: SavedVariableFile[];
  addonsPath: string;
  initialFile: string;
  esoRunning: boolean;
  characters: CharacterInfo[];
  onDirtyChange: (dirty: boolean) => void;
}) {
  const [selectedFile, setSelectedFile] = useState<string>(initialFile);
  const [tree, setTree] = useState<SvTreeNode | null>(null);
  const [loading, setLoading] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [confirmState, setConfirmState] = useState<ConfirmState | null>(null);
  const [overlay, setOverlay] = useState<SvSchemaOverlay>({});

  // Navigation state
  const [selectedPath, setSelectedPath] = useState<string[]>([]);
  const [selectedNode, setSelectedNode] = useState<SvTreeNode | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());

  const knownCharacters = useMemo(() => new Set(characters.map((c) => c.name)), [characters]);

  // Load overlay from store
  useEffect(() => {
    void getSetting<SvSchemaOverlay>(OVERLAY_STORE_KEY, {}).then(setOverlay);
  }, []);

  const handleOverlayChange = useCallback((next: SvSchemaOverlay) => {
    setOverlay(next);
    void setSetting(OVERLAY_STORE_KEY, next);
  }, []);

  // Notify parent of dirty state changes
  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);

  // Sync when parent changes the initial file
  useEffect(() => {
    if (initialFile) {
      setSelectedFile(initialFile);
    }
  }, [initialFile]);

  const loadFile = useCallback(
    async (fileName: string) => {
      if (!fileName) return;
      setLoading(true);
      setDirty(false);
      setSelectedPath([]);
      setSelectedNode(null);
      try {
        const result = await invokeOrThrow<SvTreeNode>("read_saved_variable", {
          addonsPath,
          fileName,
        });
        setTree(result);
      } catch (e) {
        toast.error(`Failed to read file: ${getTauriErrorMessage(e)}`);
        setTree(null);
      } finally {
        setLoading(false);
      }
    },
    [addonsPath]
  );

  useEffect(() => {
    if (selectedFile) {
      void loadFile(selectedFile);
    }
  }, [selectedFile, loadFile]);

  const handleEdit = useCallback((path: string[], value: string | number | boolean | null) => {
    setDirty(true);
    setTree((prev) => {
      if (!prev) return prev;
      return updateTreeNode(prev, path, value);
    });
  }, []);

  // Re-resolve selectedNode when tree changes (edits)
  useEffect(() => {
    if (!tree || selectedPath.length === 0) return;
    let current: SvTreeNode | null = tree;
    for (const segment of selectedPath) {
      current = current?.children?.find((c) => c.key === segment) ?? null;
      if (!current) break;
    }
    if (current) setSelectedNode(current);
  }, [tree, selectedPath]);

  const handleSave = useCallback(async () => {
    if (!tree || !selectedFile) return;

    const lua = serializeTreeToLua(tree);
    try {
      await invokeOrThrow("write_saved_variable", {
        addonsPath,
        fileName: selectedFile,
        content: lua,
      });
      toast.success("Saved successfully");
      setDirty(false);
    } catch (e) {
      toast.error(`Failed to save: ${getTauriErrorMessage(e)}`);
    }
  }, [tree, selectedFile, addonsPath]);

  const handleSelectPath = useCallback((path: string[], node: SvTreeNode) => {
    setSelectedPath(path);
    setSelectedNode(node);
  }, []);

  const handleBreadcrumbNavigate = useCallback(
    (depth: number) => {
      if (depth === 0) {
        setSelectedPath([]);
        setSelectedNode(null);
        return;
      }
      const newPath = selectedPath.slice(0, depth);
      let current: SvTreeNode | null = tree;
      for (const segment of newPath) {
        current = current?.children?.find((c) => c.key === segment) ?? null;
        if (!current) break;
      }
      if (current) {
        setSelectedPath(newPath);
        setSelectedNode(current);
      }
    },
    [selectedPath, tree]
  );

  const toggleExpanded = useCallback((pathKey: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(pathKey)) next.delete(pathKey);
      else next.add(pathKey);
      return next;
    });
  }, []);

  const expandAll = useCallback(() => {
    if (!tree?.children) return;
    const paths = new Set<string>();
    const walk = (node: SvTreeNode, prefix: string) => {
      const key = prefix ? `${prefix}/${node.key}` : node.key;
      if (node.valueType === "table" && node.children) {
        paths.add(key);
        node.children.forEach((c) => walk(c, key));
      }
    };
    tree.children.forEach((c) => walk(c, ""));
    setExpandedPaths(paths);
  }, [tree]);

  const collapseAll = useCallback(() => {
    setExpandedPaths(new Set());
  }, []);

  return (
    <div className="space-y-3">
      {esoRunning && (
        <div className="flex items-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-400">
          <AlertTriangleIcon className="size-4 shrink-0" />
          ESO is running. Changes may be overwritten when you exit the game.
        </div>
      )}

      <div className="flex items-center gap-2">
        <Select
          value={selectedFile}
          onValueChange={(v) => {
            if (!v) return;
            if (dirty) {
              setConfirmState({
                title: "Unsaved changes",
                description: "You have unsaved changes. Discard them?",
                confirmLabel: "Discard",
                onConfirm: () => setSelectedFile(v),
              });
            } else {
              setSelectedFile(v);
            }
          }}
        >
          <SelectTrigger className="flex-1">
            <SelectValue placeholder="Select a file..." />
          </SelectTrigger>
          <SelectContent>
            {files.map((f) => (
              <SelectItem key={f.fileName} value={f.fileName}>
                {f.addonName}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button size="sm" onClick={() => void handleSave()} disabled={!dirty}>
          Save Changes
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            if (selectedFile) void loadFile(selectedFile);
          }}
          disabled={!dirty}
        >
          Discard
        </Button>
      </div>

      <ConfirmDialog state={confirmState} onClose={() => setConfirmState(null)} />

      {/* Two-panel layout */}
      <div
        className="flex gap-0 rounded-xl border border-white/[0.06] bg-white/[0.02] overflow-hidden"
        style={{ height: "380px" }}
      >
        {loading ? (
          <div className="flex flex-1 items-center justify-center">
            <span className="inline-block size-5 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
          </div>
        ) : !tree ? (
          <div className="flex flex-1 items-center justify-center">
            <p className="text-sm text-muted-foreground">Select a file to view its contents.</p>
          </div>
        ) : (
          <>
            {/* Left panel — Nav tree */}
            <div className="flex w-[200px] shrink-0 flex-col border-r border-white/[0.06]">
              <div className="p-2">
                <div className="relative">
                  <SearchIcon className="absolute left-2 top-1/2 size-3 -translate-y-1/2 text-muted-foreground/50" />
                  <input
                    type="text"
                    placeholder="Search..."
                    className="w-full rounded-lg border border-white/[0.08] bg-white/[0.04] py-1 pl-7 pr-2 text-xs text-foreground outline-none placeholder:text-muted-foreground/40 focus:border-[#38bdf8]/50"
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                  />
                </div>
              </div>
              <div className="flex-1 overflow-y-auto px-1 pb-1">
                {tree.children?.map((child, i) => (
                  <NavTreeItem
                    key={`${child.key}-${i}`}
                    node={child}
                    depth={0}
                    selectedPath={selectedPath}
                    onSelect={handleSelectPath}
                    searchQuery={searchQuery}
                    knownCharacters={knownCharacters}
                    expandedPaths={expandedPaths}
                    toggleExpanded={toggleExpanded}
                  />
                ))}
              </div>
              <div className="flex gap-1 border-t border-white/[0.06] p-1.5">
                <button
                  onClick={expandAll}
                  className="flex-1 rounded px-1.5 py-0.5 text-[10px] text-muted-foreground hover:bg-white/[0.06] hover:text-foreground"
                >
                  Expand All
                </button>
                <button
                  onClick={collapseAll}
                  className="flex-1 rounded px-1.5 py-0.5 text-[10px] text-muted-foreground hover:bg-white/[0.06] hover:text-foreground"
                >
                  Collapse All
                </button>
              </div>
            </div>

            {/* Right panel — Detail / Form */}
            <div className="flex flex-1 flex-col overflow-hidden p-3">
              <DetailPanel
                selectedNode={selectedNode}
                selectedPath={selectedPath}
                tree={tree}
                overlay={overlay}
                knownCharacters={knownCharacters}
                onNavigate={handleBreadcrumbNavigate}
                onEdit={handleEdit}
                onOverlayChange={handleOverlayChange}
                onSelectPath={handleSelectPath}
              />
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// ─── Copy Profile Tab ────────────────────────────────────────

function CopyProfileTab({
  files,
  characters,
  addonsPath,
  onRefresh,
}: {
  files: SavedVariableFile[];
  characters: CharacterInfo[];
  addonsPath: string;
  onRefresh: () => void;
}) {
  const [selectedFile, setSelectedFile] = useState<string>("");
  const [sourceKey, setSourceKey] = useState<string>("");
  const [destKey, setDestKey] = useState<string>("");
  const [customDest, setCustomDest] = useState<string>("");
  const [copying, setCopying] = useState(false);

  const currentFile = useMemo(
    () => files.find((f) => f.fileName === selectedFile),
    [files, selectedFile]
  );

  // Known character names from AddOnSettings.txt (authoritative source)
  const knownNames = useMemo(() => new Set(characters.map((c) => c.name)), [characters]);

  // Filter this file's keys to only real characters (match known names)
  const charKeys = useMemo(
    () => (currentFile?.characterKeys ?? []).filter((k) => knownNames.has(k)),
    [currentFile, knownNames]
  );

  // All known character names for the destination (even if not in this file yet)
  const allCharNames = useMemo(() => [...knownNames].sort(), [knownNames]);

  // Destination: characters in this file first, then others not yet in the file
  const destInFile = useMemo(() => charKeys.filter((k) => k !== sourceKey), [charKeys, sourceKey]);
  const destFromOtherFiles = useMemo(
    () => allCharNames.filter((k) => k !== sourceKey && !charKeys.includes(k)),
    [allCharNames, sourceKey, charKeys]
  );

  const actualDest = destKey === "__custom__" ? customDest : destKey;

  const handleCopy = async () => {
    if (!selectedFile || !sourceKey || !actualDest) return;
    setCopying(true);
    try {
      await invokeOrThrow("copy_sv_profile", {
        addonsPath,
        fileName: selectedFile,
        fromKey: sourceKey,
        toKey: actualDest,
      });
      toast.success(`Copied "${sourceKey}" to "${actualDest}" in ${currentFile?.addonName}`);
      onRefresh();
      setSourceKey("");
      setDestKey("");
      setCustomDest("");
    } catch (e) {
      toast.error(`Copy failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setCopying(false);
    }
  };

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Copy addon settings from one character to another within the same SavedVariables file.
      </p>

      {/* Step 1: File */}
      <div>
        <label className="text-xs text-muted-foreground">1. Select SavedVariables file</label>
        <Select
          value={selectedFile}
          onValueChange={(v) => {
            if (!v) return;
            setSelectedFile(v);
            setSourceKey("");
            setDestKey("");
          }}
        >
          <SelectTrigger className="mt-1 w-full">
            <SelectValue placeholder="Choose a file..." />
          </SelectTrigger>
          <SelectContent>
            {files
              .filter((f) => f.characterKeys.some((k) => knownNames.has(k)))
              .map((f) => {
                const count = f.characterKeys.filter((k) => knownNames.has(k)).length;
                return (
                  <SelectItem key={f.fileName} value={f.fileName}>
                    {f.addonName} ({count} {count === 1 ? "character" : "characters"})
                  </SelectItem>
                );
              })}
          </SelectContent>
        </Select>
      </div>

      {/* Step 2: Source */}
      {selectedFile && (
        <div>
          <label className="text-xs text-muted-foreground">2. Source character</label>
          <Select
            value={sourceKey}
            onValueChange={(v) => {
              if (!v) return;
              setSourceKey(v);
              setDestKey("");
            }}
          >
            <SelectTrigger className="mt-1 w-full">
              <SelectValue placeholder="Choose source..." />
            </SelectTrigger>
            <SelectContent>
              {charKeys.map((k) => (
                <SelectItem key={k} value={k}>
                  {k}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}

      {/* Step 3: Destination */}
      {sourceKey && (
        <div>
          <label className="text-xs text-muted-foreground">3. Destination character</label>
          <Select value={destKey} onValueChange={(v) => v && setDestKey(v)}>
            <SelectTrigger className="mt-1 w-full">
              <SelectValue placeholder="Choose destination..." />
            </SelectTrigger>
            <SelectContent>
              {destInFile.map((k) => (
                <SelectItem key={k} value={k}>
                  {k}
                </SelectItem>
              ))}
              {destFromOtherFiles.map((k) => (
                <SelectItem key={k} value={k}>
                  <span className="text-muted-foreground">{k}</span>
                  <span className="ml-1 text-[10px] text-muted-foreground/50">(other file)</span>
                </SelectItem>
              ))}
              <SelectItem value="__custom__">+ Custom key...</SelectItem>
            </SelectContent>
          </Select>
          {destKey === "__custom__" && (
            <Input
              className="mt-2"
              placeholder='e.g. "CharName^NA"'
              value={customDest}
              onChange={(e) => setCustomDest(e.target.value)}
            />
          )}
        </div>
      )}

      {/* Step 4: Confirm */}
      {sourceKey && actualDest && (
        <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3">
          <p className="text-sm">
            Copy <span className="font-medium text-[#c4a44a]">{sourceKey}</span>
            {" \u2192 "}
            <span className="font-medium text-[#4dc2e6]">{actualDest}</span>
            {" in "}
            <span className="font-medium">{currentFile?.addonName}.lua</span>
          </p>
          <Button
            className="mt-2"
            size="sm"
            onClick={() => void handleCopy()}
            disabled={copying || !actualDest.trim()}
          >
            <CopyIcon className="mr-1 size-3" />
            {copying ? "Copying..." : "Copy Profile"}
          </Button>
        </div>
      )}
    </div>
  );
}

// ─── Lua Serializer ──────────────────────────────────────────

function serializeTreeToLua(root: SvTreeNode): string {
  const lines: string[] = [];
  if (root.children) {
    for (const child of root.children) {
      lines.push(`${child.key} = ${serializeNode(child, 0, true)}`);
      lines.push("");
    }
  }
  return lines.join("\n");
}

function serializeNode(node: SvTreeNode, depth: number, isTopLevel = false): string {
  const indent = "\t".repeat(depth);

  if (node.valueType === "table" && node.children) {
    const lines: string[] = [];
    lines.push(`${indent}{`);
    for (const child of node.children) {
      const keyPart = isNumericKey(child.key) ? `[${child.key}]` : `["${escLua(child.key)}"]`;
      if (child.valueType === "table") {
        lines.push(`${indent}\t${keyPart} = ${serializeNode(child, depth + 1)}`);
      } else {
        lines.push(`${indent}\t${keyPart} = ${serializeLeaf(child)},`);
      }
    }
    // Top-level assignments must not have a trailing comma (Lua syntax error)
    lines.push(isTopLevel ? `${indent}}` : `${indent}},`);
    return lines.join("\n");
  }

  return `${indent}${serializeLeaf(node)}`;
}

function serializeLeaf(node: SvTreeNode): string {
  switch (node.valueType) {
    case "string":
      return `"${escLua(String(node.value ?? ""))}"`;
    case "number": {
      const v = node.value;
      // Guard against NaN/Infinity that came through as null from serde_json
      if (v === null || v === undefined || (typeof v === "number" && !isFinite(v))) return "0";
      return String(v);
    }
    case "boolean":
      return String(node.value ?? false);
    case "nil":
      return "nil";
    default:
      return "nil";
  }
}

function escLua(s: string): string {
  let out = "";
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    const code = s.charCodeAt(i);
    if (c === "\\") out += "\\\\";
    else if (c === '"') out += '\\"';
    else if (c === "\n") out += "\\n";
    else if (c === "\r") out += "\\r";
    else if (c === "\t") out += "\\t";
    else if (code === 7) out += "\\a";
    else if (code === 8) out += "\\b";
    else if (code === 11) out += "\\v";
    else if (code === 12) out += "\\f";
    else if (code < 32 || code === 127) out += `\\${code}`;
    else out += c;
  }
  return out;
}

function isNumericKey(key: string): boolean {
  return /^-?\d+$/.test(key);
}

// ─── Confirm Dialog ─────────────────────────────────────────

interface ConfirmState {
  title: string;
  description: string;
  confirmLabel?: string;
  onConfirm: () => void;
}

function ConfirmDialog({ state, onClose }: { state: ConfirmState | null; onClose: () => void }) {
  if (!state) return null;
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-sm" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>{state.title}</DialogTitle>
          <DialogDescription>{state.description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={() => {
              state.onConfirm();
              onClose();
            }}
          >
            {state.confirmLabel ?? "Confirm"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ─── Main Component ──────────────────────────────────────────

export function SavedVariables({ addonsPath, installedAddons, onClose }: SavedVariablesProps) {
  const [files, setFiles] = useState<SavedVariableFile[]>([]);
  const [characters, setCharacters] = useState<CharacterInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<string>("overview");
  const [editorFile, setEditorFile] = useState<string>("");
  const [esoRunning, setEsoRunning] = useState(false);
  const [confirmState, setConfirmState] = useState<ConfirmState | null>(null);
  const editorDirtyRef = useRef(false);

  const installedFolders = useMemo(
    () => new Set(installedAddons.map((a) => a.folderName)),
    [installedAddons]
  );

  const loadFiles = useCallback(async () => {
    setLoading(true);
    try {
      const result = await invokeOrThrow<SavedVariableFile[]>("list_saved_variables", {
        addonsPath,
      });
      setFiles(result);
    } catch (e) {
      toast.error(`Failed to load SavedVariables: ${getTauriErrorMessage(e)}`);
    } finally {
      setLoading(false);
    }
  }, [addonsPath]);

  useEffect(() => {
    void loadFiles();
    invokeOrThrow<CharacterInfo[]>("list_characters", { addonsPath })
      .then(setCharacters)
      .catch(() => {});
  }, [loadFiles, addonsPath]);

  useEffect(() => {
    invokeOrThrow<boolean>("is_eso_running")
      .then(setEsoRunning)
      .catch(() => {});
  }, []);

  const handleSelectFile = useCallback((f: SavedVariableFile) => {
    setEditorFile(f.fileName);
    setActiveTab("editor");
  }, []);

  const handleDirtyChange = useCallback((d: boolean) => {
    editorDirtyRef.current = d;
  }, []);

  const handleClose = useCallback(() => {
    if (editorDirtyRef.current) {
      setConfirmState({
        title: "Unsaved changes",
        description: "You have unsaved changes. Discard them?",
        confirmLabel: "Discard",
        onConfirm: () => onClose(),
      });
      return;
    }
    onClose();
  }, [onClose]);

  return (
    <Dialog open onOpenChange={(open) => !open && handleClose()}>
      <DialogContent className="sm:max-w-4xl">
        <DialogHeader>
          <DialogTitle>SavedVariables Manager</DialogTitle>
        </DialogHeader>

        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList variant="line">
            <TabsTrigger value="overview">Overview</TabsTrigger>
            <TabsTrigger value="cleanup">Cleanup</TabsTrigger>
            <TabsTrigger value="copy">Copy Profile</TabsTrigger>
            <TabsTrigger value="editor">Editor</TabsTrigger>
          </TabsList>

          <TabsContent value="overview">
            <OverviewTab
              files={files}
              loading={loading}
              installedFolders={installedFolders}
              onRefresh={() => void loadFiles()}
              onSelectFile={handleSelectFile}
              onSwitchToCleanup={() => setActiveTab("cleanup")}
            />
          </TabsContent>

          <TabsContent value="cleanup">
            <CleanupTab
              files={files}
              installedFolders={installedFolders}
              addonsPath={addonsPath}
              onRefresh={() => void loadFiles()}
            />
          </TabsContent>

          <TabsContent value="copy">
            <CopyProfileTab
              files={files}
              characters={characters}
              addonsPath={addonsPath}
              onRefresh={() => void loadFiles()}
            />
          </TabsContent>

          <TabsContent value="editor">
            <EditorTab
              files={files}
              addonsPath={addonsPath}
              initialFile={editorFile}
              esoRunning={esoRunning}
              characters={characters}
              onDirtyChange={handleDirtyChange}
            />
          </TabsContent>
        </Tabs>

        <DialogFooter>
          <Button variant="outline" onClick={handleClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>

      <ConfirmDialog state={confirmState} onClose={() => setConfirmState(null)} />
    </Dialog>
  );
}
