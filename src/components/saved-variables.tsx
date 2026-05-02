import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { toast } from "sonner";
import type {
  AddonManifest,
  CharacterInfo,
  SavedVariableFile,
  SvTreeNode,
  SvFileStamp,
  SvReadResponse,
  SvDiffPreview,
  SvChange,
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
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import { Tabs, TabsIndicator, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { motion, AnimatePresence } from "motion/react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Popover,
  PopoverTrigger,
  PopoverContent,
  PopoverClose,
  PopoverTitle,
} from "@/components/ui/popover";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { getSetting, setSetting } from "@/lib/store";
import { classifyContext, humanizeKey, getTableChildren, getLeafChildren } from "@/lib/sv-nodes";
import { resolveEffectiveField } from "@/lib/sv-widgets";
import {
  ToggleControl,
  NumberControl,
  SliderControl,
  ColorControl,
  TextControl,
  DropdownControl,
  ReadonlyControl,
  RawControl,
} from "@/components/sv-controls";
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
  SearchIcon,
  SettingsIcon,
  EyeOffIcon,
  LockIcon,
  RotateCcwIcon,
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
        <div className="rounded-xl border border-white/[0.06] bg-white/[0.03] p-2.5 text-center shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_2px_8px_rgba(0,0,0,0.12)]">
          <div className="text-lg font-heading font-semibold">{files.length}</div>
          <div className="text-[10px] uppercase tracking-wider text-muted-foreground">Files</div>
        </div>
        <div className="rounded-xl border border-white/[0.06] bg-white/[0.03] p-2.5 text-center shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_2px_8px_rgba(0,0,0,0.12)]">
          <div className="text-lg font-heading font-semibold">{formatBytes(totalSize)}</div>
          <div className="text-[10px] uppercase tracking-wider text-muted-foreground">
            Total Size
          </div>
        </div>
        <button
          onClick={orphaned.length > 0 ? onSwitchToCleanup : undefined}
          className={`rounded-xl border p-2.5 text-center transition-all duration-200 ${
            orphaned.length > 0
              ? "border-amber-500/25 bg-amber-500/[0.06] hover:border-amber-500/35 hover:shadow-[0_0_16px_rgba(245,158,11,0.1),inset_0_1px_0_rgba(245,158,11,0.06)] shadow-[inset_0_1px_0_rgba(245,158,11,0.04),0_2px_8px_rgba(0,0,0,0.12)] cursor-pointer"
              : "border-white/[0.06] bg-white/[0.03] shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_2px_8px_rgba(0,0,0,0.12)]"
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
          className="flex w-full items-center gap-2 rounded-xl border border-amber-500/25 bg-amber-500/[0.06] p-2.5 text-left text-xs text-amber-300 transition-all duration-200 hover:border-amber-500/35 shadow-[inset_0_1px_0_rgba(245,158,11,0.04),0_2px_8px_rgba(0,0,0,0.1)] hover:shadow-[0_0_16px_rgba(245,158,11,0.08),inset_0_1px_0_rgba(245,158,11,0.06)]"
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
        <div className="flex items-center gap-2 rounded-xl border border-red-500/20 bg-red-500/[0.04] p-2.5 text-xs text-muted-foreground shadow-[inset_0_1px_0_rgba(239,68,68,0.03),0_2px_8px_rgba(0,0,0,0.1)]">
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
              className={`rounded-lg px-2.5 py-1 text-xs font-medium transition-all duration-200 ${
                filter === f
                  ? "bg-white/[0.1] text-foreground shadow-[0_1px_3px_rgba(0,0,0,0.2),inset_0_1px_0_rgba(255,255,255,0.06)] border border-white/[0.06]"
                  : "text-muted-foreground hover:text-foreground hover:bg-white/[0.04]"
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
            <div className="mx-auto mb-3 flex size-12 items-center justify-center rounded-2xl bg-[#c4a44a]/[0.08] shadow-[0_0_24px_rgba(196,164,74,0.1),inset_0_1px_0_rgba(196,164,74,0.08)]">
              <FileTextIcon className="size-6 text-[#c4a44a]/60" />
            </div>
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
              className="flex w-full items-center gap-3 rounded-xl border border-white/[0.06] bg-white/[0.03] p-2.5 text-left transition-all duration-200 hover:border-white/[0.1] hover:bg-white/[0.05] shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] hover:shadow-[0_4px_12px_rgba(0,0,0,0.15),inset_0_1px_0_rgba(255,255,255,0.05)]"
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
        <div className="mx-auto mb-3 flex size-14 items-center justify-center rounded-2xl bg-emerald-500/[0.08] shadow-[0_0_32px_rgba(34,197,94,0.1),inset_0_1px_0_rgba(34,197,94,0.08)]">
          <ShieldCheckIcon className="size-7 text-emerald-400" />
        </div>
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
            These files belong to addons that are no longer installed.
          </p>

          <div className="max-h-[220px] overflow-y-auto space-y-1">
            {orphaned.map((f) => (
              <label
                key={f.fileName}
                className="flex items-center gap-2.5 rounded-xl border border-white/[0.06] bg-white/[0.03] p-2 cursor-pointer transition-all duration-200 hover:border-white/[0.1] hover:bg-white/[0.05] shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] hover:shadow-[0_2px_8px_rgba(0,0,0,0.12),inset_0_1px_0_rgba(255,255,255,0.05)]"
              >
                <Checkbox
                  checked={selected.has(f.fileName)}
                  onCheckedChange={() => toggleFile(f.fileName)}
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
          <div className="mt-3 flex items-center justify-between rounded-xl border border-white/[0.06] bg-white/[0.03] p-2.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_2px_8px_rgba(0,0,0,0.12)]">
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
                className="flex items-center justify-between rounded-xl border border-red-500/15 bg-red-500/[0.03] p-2 shadow-[inset_0_1px_0_rgba(239,68,68,0.03)]"
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
const EMPTY_PATH: string[] = [];

// ── Nav Tree Item ────────────────────────────────────────────

function NavTreeItem({
  node,
  depth,
  structuralDepth,
  parentPath,
  selectedPath,
  onSelect,
  searchQuery,
  knownCharacters,
  expandedPaths,
  toggleExpanded,
}: {
  node: SvTreeNode;
  depth: number;
  structuralDepth: number;
  parentPath: string[];
  selectedPath: string[];
  onSelect: (path: string[], node: SvTreeNode) => void;
  searchQuery: string;
  knownCharacters: Set<string>;
  expandedPaths: Set<string>;
  toggleExpanded: (pathKey: string) => void;
}) {
  const currentPath = useMemo(() => [...parentPath, node.key], [parentPath, node.key]);
  const pathKey = currentPath.map((s) => s.replace(/\0/g, "\\0")).join("\0");
  const isExpanded = expandedPaths.has(pathKey);
  const tableChildren = getTableChildren(node);
  const entryCount = node.children?.length ?? 0;

  // Compare full path for accurate selection highlighting
  const isSelected =
    selectedPath.length === currentPath.length &&
    currentPath.every((seg, i) => selectedPath[i] === seg);

  const context = classifyContext(node.key, structuralDepth, knownCharacters);
  const matchesSearch = !searchQuery || node.key.toLowerCase().includes(searchQuery.toLowerCase());

  // Detect context wrapper nodes that should be auto-expanded and rendered as chips.
  // These are structural containers (Default, @account, $AccountWide, server names)
  // that users rarely need to interact with — the interesting data is inside them.
  const isContextWrapper =
    structuralDepth <= 2 &&
    tableChildren.length > 0 &&
    (node.key === "Default" ||
      node.key.startsWith("@") ||
      node.key === "$AccountWide" ||
      node.key.includes("Megaserver"));

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

  // Context wrappers: render as a compact chip divider and auto-show children
  if (isContextWrapper) {
    const chipColor =
      node.key === "$AccountWide"
        ? "sky"
        : node.key.startsWith("@")
          ? "sky"
          : node.key.includes("Megaserver")
            ? "violet"
            : "muted";

    return (
      <div>
        <div
          className="flex items-center gap-1.5 px-2 py-0.5"
          style={{ paddingLeft: `${depth * 14 + 8}px` }}
        >
          <div className="h-px flex-1 bg-white/[0.06]" />
          <InfoPill color={chipColor} className="!text-[9px] !px-1.5 !py-0">
            {node.key}
          </InfoPill>
          <div className="h-px flex-1 bg-white/[0.06]" />
        </div>
        {tableChildren.map((child) => (
          <NavTreeItem
            key={child.key}
            node={child}
            depth={depth}
            structuralDepth={structuralDepth + 1}
            parentPath={currentPath}
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

  return (
    <div>
      <button
        onClick={() => {
          onSelect(currentPath, node);
          if (tableChildren.length > 0) toggleExpanded(pathKey);
        }}
        className={`flex w-full items-center gap-1.5 rounded-lg px-2 py-1 text-left text-xs transition-all duration-150 ${
          isSelected
            ? "bg-white/[0.1] text-foreground shadow-[inset_0_1px_0_rgba(255,255,255,0.06),0_1px_3px_rgba(0,0,0,0.15)] border border-white/[0.06]"
            : "text-foreground/70 hover:bg-white/[0.04] hover:text-foreground border border-transparent"
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
        tableChildren.map((child) => (
          <NavTreeItem
            key={child.key}
            node={child}
            depth={depth + 1}
            structuralDepth={structuralDepth + 1}
            parentPath={currentPath}
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
          <Checkbox checked={hidden} onCheckedChange={(checked) => setHidden(checked === true)} />
          <EyeOffIcon className="size-3" />
          Hide
        </label>
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer">
          <Checkbox
            checked={readOnly}
            onCheckedChange={(checked) => setReadOnly(checked === true)}
          />
          <LockIcon className="size-3" />
          Read-only
        </label>
      </div>

      <div className="flex items-center justify-between border-t border-white/[0.06] pt-2.5 mt-0.5">
        <PopoverClose
          render={
            <button
              onClick={handleReset}
              className="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
            >
              <RotateCcwIcon className="size-3" />
              Reset to auto
            </button>
          }
        />
        <PopoverClose
          render={
            <Button size="sm" onClick={handleSave}>
              Apply
            </Button>
          }
        />
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

  const pathSegments = field.nodeId.split("/");

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
    <div className="flex items-center gap-3 rounded-lg px-2.5 py-1.5 transition-colors duration-150 hover:bg-white/[0.03] group">
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
    <div className="flex items-center gap-1 text-xs overflow-x-auto pb-1 rounded-lg bg-white/[0.02] px-2 py-1.5 border border-white/[0.04]">
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
                    className="flex items-center gap-2 rounded-xl border border-white/[0.06] bg-white/[0.03] p-2.5 text-left transition-all duration-200 hover:border-white/[0.12] hover:bg-white/[0.05] shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] hover:shadow-[0_4px_12px_rgba(0,0,0,0.15),inset_0_1px_0_rgba(255,255,255,0.05)]"
                  >
                    <BracesIcon className="size-3.5 shrink-0 text-violet-400/70" />
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
              <div className="mx-auto mb-3 flex size-10 items-center justify-center rounded-xl bg-white/[0.04] shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]">
                <EyeOffIcon className="size-5 text-muted-foreground/40" />
              </div>
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
  const [fileStamp, setFileStamp] = useState<SvFileStamp | null>(null);
  const [loading, setLoading] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [confirmState, setConfirmState] = useState<ConfirmState | null>(null);
  const [overlay, setOverlay] = useState<SvSchemaOverlay>({});
  const [rawMode, setRawMode] = useState(false);

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
        const result = await invokeOrThrow<SvReadResponse>("read_saved_variable", {
          addonsPath,
          fileName,
        });
        setTree(result.tree);
        setFileStamp(result.stamp);
      } catch (e) {
        toast.error(`Failed to read file: ${getTauriErrorMessage(e)}`);
        setTree(null);
        setFileStamp(null);
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
    if (!tree || !selectedFile || !fileStamp) return;

    try {
      const newStamp = await invokeOrThrow<SvFileStamp>("write_saved_variable", {
        addonsPath,
        fileName: selectedFile,
        tree,
        stamp: fileStamp,
      });
      setFileStamp(newStamp);
      toast.success("Saved successfully");
      setDirty(false);
    } catch (e) {
      toast.error(`Failed to save: ${getTauriErrorMessage(e)}`);
    }
  }, [tree, selectedFile, addonsPath, fileStamp]);

  const handleRestoreBackup = useCallback(async () => {
    if (!selectedFile) return;
    try {
      const newStamp = await invokeOrThrow<SvFileStamp>("restore_sv_backup", {
        addonsPath,
        fileName: selectedFile,
      });
      setFileStamp(newStamp);
      // Reload the file to get the restored content
      await loadFile(selectedFile);
      toast.success("Backup restored successfully");
    } catch (e) {
      toast.error(`Failed to restore: ${getTauriErrorMessage(e)}`);
    }
  }, [selectedFile, addonsPath, loadFile]);

  const [diffPreview, setDiffPreview] = useState<SvDiffPreview | null>(null);

  const handlePreview = useCallback(async () => {
    if (!tree || !selectedFile) return;
    try {
      const preview = await invokeOrThrow<SvDiffPreview>("preview_sv_save", {
        addonsPath,
        fileName: selectedFile,
        tree,
      });
      setDiffPreview(preview);
    } catch (e) {
      toast.error(`Failed to generate preview: ${getTauriErrorMessage(e)}`);
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
    const walk = (node: SvTreeNode, parentPath: string[]) => {
      const currentPath = [...parentPath, node.key];
      const key = currentPath.map((s) => s.replace(/\0/g, "\\0")).join("\0");
      if (node.valueType === "table" && node.children) {
        paths.add(key);
        node.children.forEach((c) => walk(c, currentPath));
      }
    };
    tree.children.forEach((c) => walk(c, []));
    setExpandedPaths(paths);
  }, [tree]);

  const collapseAll = useCallback(() => {
    setExpandedPaths(new Set());
  }, []);

  return (
    <div className="space-y-3">
      {esoRunning && (
        <div className="flex items-center gap-2 rounded-xl border border-amber-500/25 bg-amber-500/[0.06] p-2.5 text-xs text-amber-400 shadow-[0_0_16px_rgba(245,158,11,0.06),inset_0_1px_0_rgba(245,158,11,0.04)]">
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
        <Button size="sm" variant="outline" onClick={() => void handlePreview()} disabled={!dirty}>
          Preview
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
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            setConfirmState({
              title: "Restore Backup",
              description:
                "This will restore the .bak file created before the last save. Any unsaved changes will be lost.",
              confirmLabel: "Restore",
              onConfirm: () => void handleRestoreBackup(),
            });
          }}
          disabled={!selectedFile}
        >
          <RotateCcwIcon className="mr-1 size-3.5" />
          Restore
        </Button>
        <div className="ml-auto flex items-center gap-1 rounded-xl border border-white/[0.06] bg-white/[0.03] p-0.5 shadow-[inset_0_1px_2px_rgba(0,0,0,0.15)]">
          <button
            onClick={() => setRawMode(false)}
            className={`rounded-lg px-2.5 py-1 text-xs font-medium transition-all duration-200 ${
              !rawMode
                ? "bg-white/[0.1] text-foreground border border-white/[0.06] shadow-[0_1px_3px_rgba(0,0,0,0.2),inset_0_1px_0_rgba(255,255,255,0.06)]"
                : "text-muted-foreground/60 hover:text-muted-foreground"
            }`}
          >
            <SettingsIcon className="mr-1 inline-block size-3" />
            Settings
          </button>
          <button
            onClick={() => setRawMode(true)}
            className={`rounded-lg px-2.5 py-1 text-xs font-medium transition-all duration-200 ${
              rawMode
                ? "bg-white/[0.1] text-foreground border border-white/[0.06] shadow-[0_1px_3px_rgba(0,0,0,0.2),inset_0_1px_0_rgba(255,255,255,0.06)]"
                : "text-muted-foreground/60 hover:text-muted-foreground"
            }`}
          >
            <BracesIcon className="mr-1 inline-block size-3" />
            Raw
          </button>
        </div>
      </div>

      <ConfirmDialog state={confirmState} onClose={() => setConfirmState(null)} />

      {/* Diff Preview Dialog */}
      {diffPreview && (
        <DiffPreviewDialog
          preview={diffPreview}
          onClose={() => setDiffPreview(null)}
          onConfirmSave={() => {
            setDiffPreview(null);
            void handleSave();
          }}
        />
      )}

      {/* Two-panel layout */}
      <div
        className="flex gap-0 rounded-xl border border-white/[0.08] bg-[rgba(15,23,42,0.4)] backdrop-blur-sm overflow-hidden shadow-[0_4px_16px_rgba(0,0,0,0.2),inset_0_1px_0_rgba(255,255,255,0.04)]"
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
        ) : rawMode ? (
          <div className="flex-1 overflow-y-auto p-3 font-mono text-xs">
            <RawTreeView node={tree} depth={0} />
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
                    className="w-full rounded-lg border border-white/[0.1] bg-white/[0.04] py-1 pl-7 pr-2 text-xs text-foreground outline-none placeholder:text-muted-foreground/40 shadow-[inset_0_1px_2px_rgba(0,0,0,0.15)] focus:border-[#38bdf8]/50 focus:shadow-[0_0_0_3px_rgba(56,189,248,0.1),inset_0_1px_2px_rgba(0,0,0,0.1)]"
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
                    structuralDepth={0}
                    parentPath={EMPTY_PATH}
                    selectedPath={selectedPath}
                    onSelect={handleSelectPath}
                    searchQuery={searchQuery}
                    knownCharacters={knownCharacters}
                    expandedPaths={expandedPaths}
                    toggleExpanded={toggleExpanded}
                  />
                ))}
              </div>
              <div className="flex gap-1 border-t border-white/[0.06] bg-white/[0.02] p-1.5">
                <button
                  onClick={expandAll}
                  className="flex-1 rounded-md px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground transition-all duration-150 hover:bg-white/[0.06] hover:text-foreground"
                >
                  Expand All
                </button>
                <button
                  onClick={collapseAll}
                  className="flex-1 rounded-md px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground transition-all duration-150 hover:bg-white/[0.06] hover:text-foreground"
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

// ─── Raw Tree View ──────────────────────────────────────────

function RawTreeView({ node, depth }: { node: SvTreeNode; depth: number }) {
  const [expanded, setExpanded] = useState(depth < 2);
  const isTable = node.valueType === "table" && node.children;
  const indent = depth * 16;

  const valueDisplay = () => {
    if (isTable) return `{${node.children?.length ?? 0} entries}`;
    if (node.valueType === "string") return `"${String(node.value ?? "")}"`;
    if (node.valueType === "nil") return "nil";
    return String(node.value ?? "");
  };

  const valueColor = () => {
    switch (node.valueType) {
      case "string":
        return "text-emerald-400";
      case "number":
        return "text-sky-400";
      case "boolean":
        return "text-amber-400";
      case "nil":
        return "text-muted-foreground/50";
      case "table":
        return "text-muted-foreground/60";
      default:
        return "text-foreground";
    }
  };

  return (
    <div>
      <div
        className="flex items-center gap-1 py-0.5 hover:bg-white/[0.03] rounded cursor-default"
        style={{ paddingLeft: indent }}
      >
        {isTable ? (
          <button
            onClick={() => setExpanded(!expanded)}
            className="flex size-4 items-center justify-center text-muted-foreground hover:text-foreground"
          >
            {expanded ? (
              <ChevronDownIcon className="size-3" />
            ) : (
              <ChevronRightIcon className="size-3" />
            )}
          </button>
        ) : (
          <span className="inline-block size-4" />
        )}
        <span className="text-foreground/70">{node.key}</span>
        <span className="text-muted-foreground/40 mx-0.5">=</span>
        <span className={valueColor()}>{valueDisplay()}</span>
      </div>
      {isTable &&
        expanded &&
        node.children?.map((child, i) => (
          <RawTreeView key={`${child.key}-${i}`} node={child} depth={depth + 1} />
        ))}
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
        <div className="rounded-xl border border-[#c4a44a]/20 bg-[#c4a44a]/[0.04] p-3 shadow-[0_0_16px_rgba(196,164,74,0.06),inset_0_1px_0_rgba(196,164,74,0.04)]">
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

// ─── Confirm Dialog ─────────────────────────────────────────

// ─── Diff Preview Dialog ────────────────────────────────────

/** Format a key path like ["Default", "@Account", "setting"] for display */
function formatChangePath(change: SvChange): { setting: string; location: string } {
  const path = change.path;
  const setting = path[path.length - 1] ?? "unknown";
  const location = path.length > 1 ? path.slice(0, -1).join(" > ") : "";
  return { setting, location };
}

function DiffPreviewDialog({
  preview,
  onClose,
  onConfirmSave,
}: {
  preview: SvDiffPreview;
  onClose: () => void;
  onConfirmSave: () => void;
}) {
  const { changes } = preview;
  const modified = changes.filter((c) => c.changeType === "modified");
  const added = changes.filter((c) => c.changeType === "added");
  const removed = changes.filter((c) => c.changeType === "removed");

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-2xl max-h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>Review Changes</DialogTitle>
          <DialogDescription>
            {changes.length === 0
              ? "No changes detected."
              : `${changes.length} change${changes.length === 1 ? "" : "s"} will be saved`}
          </DialogDescription>
        </DialogHeader>

        {changes.length > 0 && (
          <div className="flex gap-3 text-xs">
            {modified.length > 0 && (
              <span className="text-sky-400">{modified.length} modified</span>
            )}
            {added.length > 0 && <span className="text-emerald-400">{added.length} added</span>}
            {removed.length > 0 && <span className="text-red-400">{removed.length} removed</span>}
          </div>
        )}

        <div className="flex-1 overflow-auto space-y-1.5 pr-1">
          {changes.map((change, i) => {
            const { setting, location } = formatChangePath(change);
            return (
              <div
                key={i}
                className={`rounded-lg border px-3 py-2 text-xs ${
                  change.changeType === "added"
                    ? "border-emerald-500/20 bg-emerald-500/[0.06] shadow-[inset_0_1px_0_rgba(34,197,94,0.04)]"
                    : change.changeType === "removed"
                      ? "border-red-500/20 bg-red-500/[0.06] shadow-[inset_0_1px_0_rgba(239,68,68,0.04)]"
                      : "border-white/[0.06] bg-white/[0.03] shadow-[inset_0_1px_0_rgba(255,255,255,0.03)]"
                }`}
              >
                <div className="flex items-center gap-2 mb-1">
                  <span className="font-medium text-muted-foreground font-mono truncate">
                    {setting}
                  </span>
                  {change.changeType === "added" && (
                    <span className="shrink-0 text-[10px] font-medium uppercase tracking-wider text-emerald-400/80">
                      Added
                    </span>
                  )}
                  {change.changeType === "removed" && (
                    <span className="shrink-0 text-[10px] font-medium uppercase tracking-wider text-red-400/80">
                      Removed
                    </span>
                  )}
                </div>

                {location && (
                  <div className="text-[10px] text-muted-foreground/50 mb-1.5 truncate">
                    {location}
                  </div>
                )}

                {change.changeType === "modified" ? (
                  <div className="flex items-center gap-2 font-mono">
                    <SimpleTooltip content={change.oldValue ?? ""}>
                      <span className="rounded bg-red-500/10 px-1.5 py-0.5 text-red-400 truncate max-w-[45%]">
                        {change.oldValue}
                      </span>
                    </SimpleTooltip>
                    <ChevronRightIcon className="size-3 shrink-0 text-muted-foreground/40" />
                    <SimpleTooltip content={change.newValue ?? ""}>
                      <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-400 truncate max-w-[45%]">
                        {change.newValue}
                      </span>
                    </SimpleTooltip>
                  </div>
                ) : (
                  <div className="font-mono">
                    <SimpleTooltip
                      content={
                        (change.changeType === "added" ? change.newValue : change.oldValue) ?? ""
                      }
                    >
                      <span
                        className={`rounded px-1.5 py-0.5 truncate inline-block max-w-full ${
                          change.changeType === "added"
                            ? "bg-emerald-500/10 text-emerald-400"
                            : "bg-red-500/10 text-red-400"
                        }`}
                      >
                        {change.changeType === "added" ? change.newValue : change.oldValue}
                      </span>
                    </SimpleTooltip>
                  </div>
                )}
              </div>
            );
          })}
        </div>

        <DialogFooter>
          <Button size="sm" variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" onClick={onConfirmSave} disabled={changes.length === 0}>
            Save Changes
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

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
      <DialogContent className="sm:max-w-4xl h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>SavedVariables Manager</DialogTitle>
        </DialogHeader>

        <Tabs
          value={activeTab}
          onValueChange={setActiveTab}
          className="flex-1 min-h-0 flex flex-col"
        >
          <TabsList variant="line" className="shrink-0">
            <TabsIndicator />
            <TabsTrigger value="overview">Overview</TabsTrigger>
            <TabsTrigger value="cleanup">Cleanup</TabsTrigger>
            <TabsTrigger value="copy">Copy Profile</TabsTrigger>
            <TabsTrigger value="editor">Editor</TabsTrigger>
          </TabsList>

          <div className="flex-1 min-h-0 overflow-y-auto">
            <AnimatePresence mode="wait">
              {activeTab === "overview" && (
                <motion.div
                  key="overview"
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ type: "spring", stiffness: 300, damping: 30, duration: 0.15 }}
                  className="h-full"
                >
                  <OverviewTab
                    files={files}
                    loading={loading}
                    installedFolders={installedFolders}
                    onRefresh={() => void loadFiles()}
                    onSelectFile={handleSelectFile}
                    onSwitchToCleanup={() => setActiveTab("cleanup")}
                  />
                </motion.div>
              )}
              {activeTab === "cleanup" && (
                <motion.div
                  key="cleanup"
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ type: "spring", stiffness: 300, damping: 30, duration: 0.15 }}
                  className="h-full"
                >
                  <CleanupTab
                    files={files}
                    installedFolders={installedFolders}
                    addonsPath={addonsPath}
                    onRefresh={() => void loadFiles()}
                  />
                </motion.div>
              )}
              {activeTab === "copy" && (
                <motion.div
                  key="copy"
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ type: "spring", stiffness: 300, damping: 30, duration: 0.15 }}
                  className="h-full"
                >
                  <CopyProfileTab
                    files={files}
                    characters={characters}
                    addonsPath={addonsPath}
                    onRefresh={() => void loadFiles()}
                  />
                </motion.div>
              )}
              {activeTab === "editor" && (
                <motion.div
                  key="editor"
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  transition={{ type: "spring", stiffness: 300, damping: 30, duration: 0.15 }}
                  className="h-full"
                >
                  <EditorTab
                    files={files}
                    addonsPath={addonsPath}
                    initialFile={editorFile}
                    esoRunning={esoRunning}
                    characters={characters}
                    onDirtyChange={handleDirtyChange}
                  />
                </motion.div>
              )}
            </AnimatePresence>
          </div>
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
