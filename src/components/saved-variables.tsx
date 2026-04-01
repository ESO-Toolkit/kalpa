import { useState, useEffect, useCallback, useMemo, useRef, memo } from "react";
import { toast } from "sonner";
import type { AddonManifest, SavedVariableFile, SvTreeNode } from "../types";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import {
  RefreshCwIcon,
  ChevronRightIcon,
  ChevronDownIcon,
  FileTextIcon,
  CopyIcon,
  AlertTriangleIcon,
  HashIcon,
  ToggleLeftIcon,
  TypeIcon,
  BracesIcon,
  CircleSlashIcon,
  Trash2Icon,
  ShieldCheckIcon,
  HardDriveIcon,
  PackageXIcon,
  ArrowUpDownIcon,
  CheckIcon,
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
  for (const folder of installedFolders) {
    if (f.addonName.startsWith(folder) && f.addonName.length > folder.length) {
      return "installed";
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

// ─── Tree Node Renderer ──────────────────────────────────────

function TypeIcon_({ type: t }: { type: string }) {
  switch (t) {
    case "string":
      return <TypeIcon className="size-3 text-green-400" />;
    case "number":
      return <HashIcon className="size-3 text-blue-400" />;
    case "boolean":
      return <ToggleLeftIcon className="size-3 text-amber-400" />;
    case "nil":
      return <CircleSlashIcon className="size-3 text-muted-foreground" />;
    case "table":
      return <BracesIcon className="size-3 text-purple-400" />;
    default:
      return null;
  }
}

const EMPTY_PATH: string[] = [];

interface TreeNodeProps {
  node: SvTreeNode;
  depth: number;
  onEdit: (path: string[], value: string | number | boolean | null) => void;
  path: string[];
}

const TreeNode = memo(function TreeNode({ node, depth, onEdit, path }: TreeNodeProps) {
  const [expanded, setExpanded] = useState(depth < 2);
  const [editingValue, setEditingValue] = useState<string | null>(null);

  const isTable = node.valueType === "table" && node.children;
  const currentPath = useMemo(() => [...path, node.key], [path, node.key]);

  const handleSave = () => {
    if (editingValue === null) return;
    let parsed: string | number | boolean | null;
    if (node.valueType === "number") {
      parsed = Number(editingValue);
      if (isNaN(parsed)) {
        toast.error("Invalid number");
        return;
      }
    } else if (node.valueType === "boolean") {
      parsed = editingValue === "true";
    } else if (node.valueType === "nil") {
      parsed = null;
    } else {
      parsed = editingValue;
    }
    onEdit(currentPath, parsed);
    setEditingValue(null);
  };

  if (isTable) {
    return (
      <div className="select-text">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex w-full items-center gap-1 rounded px-1 py-0.5 text-left text-xs hover:bg-white/[0.04]"
          style={{ paddingLeft: `${depth * 12 + 4}px` }}
          aria-expanded={expanded}
          aria-label={`${expanded ? "Collapse" : "Expand"} ${node.key}`}
        >
          {expanded ? (
            <ChevronDownIcon className="size-3 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronRightIcon className="size-3 shrink-0 text-muted-foreground" />
          )}
          <TypeIcon_ type={node.valueType} />
          <span className="font-medium text-foreground/80">{node.key}</span>
          <span className="ml-1 text-muted-foreground/50">
            ({node.children?.length ?? 0} entries)
          </span>
        </button>
        {expanded && (
          <div>
            {node.children?.map((child, i) => (
              <TreeNode
                key={`${child.key}-${i}`}
                node={child}
                depth={depth + 1}
                onEdit={onEdit}
                path={currentPath}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  const displayValue =
    node.value === null || node.value === undefined
      ? "nil"
      : typeof node.value === "string"
        ? `"${node.value}"`
        : String(node.value);

  return (
    <div
      className="flex items-center gap-1 rounded px-1 py-0.5 text-xs hover:bg-white/[0.04]"
      style={{ paddingLeft: `${depth * 12 + 4}px` }}
    >
      <div className="size-3 shrink-0" />
      <TypeIcon_ type={node.valueType} />
      <span className="font-medium text-foreground/80">{node.key}</span>
      <span className="mx-1 text-muted-foreground/40">=</span>
      {editingValue !== null ? (
        <span className="flex items-center gap-1">
          {node.valueType === "boolean" ? (
            <select
              className="rounded border border-white/[0.1] bg-transparent px-1 py-0 text-xs"
              value={editingValue}
              onChange={(e) => setEditingValue(e.target.value)}
              onBlur={handleSave}
              aria-label={`Edit ${node.key}`}
              autoFocus
            >
              <option value="true">true</option>
              <option value="false">false</option>
            </select>
          ) : (
            <input
              type={node.valueType === "number" ? "number" : "text"}
              className="w-40 rounded border border-white/[0.1] bg-transparent px-1 py-0 text-xs"
              value={editingValue}
              onChange={(e) => setEditingValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleSave();
                if (e.key === "Escape") setEditingValue(null);
              }}
              onBlur={handleSave}
              aria-label={`Edit ${node.key}`}
              autoFocus
            />
          )}
        </span>
      ) : (
        <button
          className="cursor-pointer truncate text-left text-foreground/60 hover:text-foreground"
          onClick={() => {
            if (node.valueType !== "nil") {
              const raw = node.value === null || node.value === undefined ? "" : String(node.value);
              setEditingValue(raw);
            }
          }}
          title="Click to edit"
          aria-label={`Edit ${node.key}, current value: ${displayValue}`}
        >
          {displayValue}
        </button>
      )}
    </div>
  );
});

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

  const handleDelete = async () => {
    if (selected.size === 0) return;
    const fileNames = [...selected];

    const confirmed = window.confirm(
      `Delete ${fileNames.length} orphaned file${fileNames.length !== 1 ? "s" : ""} (${formatBytes(selectedSize)})?\n\nA backup will be created automatically before deletion.`
    );
    if (!confirmed) return;

    setDeleting(true);
    try {
      const deleted = await invokeOrThrow<number>("delete_saved_variables", {
        addonsPath,
        fileNames,
      });
      toast.success(
        `Cleaned up ${deleted} file${deleted !== 1 ? "s" : ""} (${formatBytes(selectedSize)}). Backup saved.`
      );
      setSelected(new Set());
      onRefresh();
    } catch (e) {
      toast.error(`Cleanup failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setDeleting(false);
    }
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
    </div>
  );
}

// ─── Editor Tab ──────────────────────────────────────────────

function EditorTab({
  files,
  addonsPath,
  initialFile,
  esoRunning,
  onDirtyChange,
}: {
  files: SavedVariableFile[];
  addonsPath: string;
  initialFile: string;
  esoRunning: boolean;
  onDirtyChange: (dirty: boolean) => void;
}) {
  const [selectedFile, setSelectedFile] = useState<string>(initialFile);
  const [tree, setTree] = useState<SvTreeNode | null>(null);
  const [loading, setLoading] = useState(false);
  const [dirty, setDirty] = useState(false);

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

  return (
    <div className="space-y-3">
      {esoRunning && (
        <div className="flex items-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-400">
          <AlertTriangleIcon className="size-4 shrink-0" />
          ESO is currently running. Changes to SavedVariables may be overwritten when you exit the
          game.
        </div>
      )}

      <div className="flex items-center gap-2">
        <select
          className="flex-1 rounded-lg border border-white/[0.1] bg-[#1a1a2e] px-2 py-1.5 text-sm"
          value={selectedFile}
          onChange={(e) => {
            if (dirty && !window.confirm("You have unsaved changes. Discard them?")) return;
            setSelectedFile(e.target.value);
          }}
          aria-label="Select SavedVariables file"
        >
          <option value="">Select a file...</option>
          {files.map((f) => (
            <option key={f.fileName} value={f.fileName}>
              {f.addonName}
            </option>
          ))}
        </select>
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

      <div className="max-h-[350px] overflow-y-auto rounded-lg border border-white/[0.06] bg-white/[0.02] p-2">
        {loading ? (
          <div className="flex items-center justify-center py-8">
            <span className="inline-block size-5 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
          </div>
        ) : !tree ? (
          <p className="py-8 text-center text-sm text-muted-foreground">
            Select a file to view its contents.
          </p>
        ) : (
          <div className="font-mono">
            {tree.children?.map((child, i) => (
              <TreeNode
                key={`${child.key}-${i}`}
                node={child}
                depth={0}
                onEdit={handleEdit}
                path={EMPTY_PATH}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// ─── Copy Profile Tab ────────────────────────────────────────

function CopyProfileTab({
  files,
  addonsPath,
  onRefresh,
}: {
  files: SavedVariableFile[];
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

  const charKeys = useMemo(() => currentFile?.characterKeys ?? [], [currentFile]);

  const destOptions = useMemo(() => charKeys.filter((k) => k !== sourceKey), [charKeys, sourceKey]);

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
        <select
          className="mt-1 w-full rounded-lg border border-white/[0.1] bg-[#1a1a2e] px-2 py-1.5 text-sm"
          value={selectedFile}
          onChange={(e) => {
            setSelectedFile(e.target.value);
            setSourceKey("");
            setDestKey("");
          }}
          aria-label="Select SavedVariables file"
        >
          <option value="">Choose a file...</option>
          {files
            .filter((f) => f.characterKeys.length > 0)
            .map((f) => (
              <option key={f.fileName} value={f.fileName}>
                {f.addonName} ({f.characterKeys.length} profiles)
              </option>
            ))}
        </select>
      </div>

      {/* Step 2: Source */}
      {selectedFile && (
        <div>
          <label className="text-xs text-muted-foreground">2. Source character</label>
          <select
            className="mt-1 w-full rounded-lg border border-white/[0.1] bg-[#1a1a2e] px-2 py-1.5 text-sm"
            value={sourceKey}
            onChange={(e) => {
              setSourceKey(e.target.value);
              setDestKey("");
            }}
            aria-label="Source character"
          >
            <option value="">Choose source...</option>
            {charKeys.map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
        </div>
      )}

      {/* Step 3: Destination */}
      {sourceKey && (
        <div>
          <label className="text-xs text-muted-foreground">3. Destination character</label>
          <select
            className="mt-1 w-full rounded-lg border border-white/[0.1] bg-[#1a1a2e] px-2 py-1.5 text-sm"
            value={destKey}
            onChange={(e) => setDestKey(e.target.value)}
            aria-label="Destination character"
          >
            <option value="">Choose destination...</option>
            {destOptions.map((k) => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
            <option value="__custom__">+ New character key...</option>
          </select>
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
    case "number":
      return String(node.value ?? 0);
    case "boolean":
      return String(node.value ?? false);
    case "nil":
      return "nil";
    default:
      return "nil";
  }
}

function escLua(s: string): string {
  return s
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\n/g, "\\n")
    .replace(/\r/g, "\\r")
    .replace(/\t/g, "\\t");
}

function isNumericKey(key: string): boolean {
  return /^-?\d+$/.test(key);
}

// ─── Main Component ──────────────────────────────────────────

export function SavedVariables({ addonsPath, installedAddons, onClose }: SavedVariablesProps) {
  const [files, setFiles] = useState<SavedVariableFile[]>([]);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<string>("overview");
  const [editorFile, setEditorFile] = useState<string>("");
  const [esoRunning, setEsoRunning] = useState(false);
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
  }, [loadFiles]);

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
    if (editorDirtyRef.current && !window.confirm("You have unsaved changes. Discard them?")) {
      return;
    }
    onClose();
  }, [onClose]);

  return (
    <Dialog open onOpenChange={(open) => !open && handleClose()}>
      <DialogContent className="sm:max-w-2xl">
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
    </Dialog>
  );
}
