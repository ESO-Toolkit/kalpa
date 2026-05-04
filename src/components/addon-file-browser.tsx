import { useState, useEffect } from "react";
import type { AddonFileTree, AddonFileEntry, EditBackupManifest } from "../types";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import {
  FolderOpen,
  RotateCw,
  ChevronRight,
  ChevronDown,
  FileText,
  ExternalLink,
  History,
  Undo2,
} from "lucide-react";
import { AddonFileEditor } from "@/components/addon-file-editor";

interface AddonFileBrowserProps {
  addonsPath: string;
  folderName: string;
}

const EXT_COLORS: Record<string, "sky" | "amber" | "emerald" | "muted"> = {
  lua: "sky",
  xml: "amber",
  txt: "muted",
  dds: "emerald",
  ttf: "muted",
};

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

interface TreeNode {
  name: string;
  entry?: AddonFileEntry;
  children: Map<string, TreeNode>;
}

function buildTree(files: AddonFileEntry[]): TreeNode {
  const root: TreeNode = { name: "", children: new Map() };
  for (const file of files) {
    if (file.isDirectory) continue;
    const parts = file.relativePath.split("/");
    let current = root;
    for (let i = 0; i < parts.length - 1; i++) {
      if (!current.children.has(parts[i])) {
        current.children.set(parts[i], { name: parts[i], children: new Map() });
      }
      current = current.children.get(parts[i])!;
    }
    const fileName = parts[parts.length - 1];
    current.children.set(fileName, { name: fileName, entry: file, children: new Map() });
  }
  return root;
}

function FileTreeNode({
  node,
  depth,
  onOpenFile,
  selectedPath,
}: {
  node: TreeNode;
  depth: number;
  onOpenFile: (path: string) => void;
  selectedPath: string | null;
}) {
  const [expanded, setExpanded] = useState(depth < 2);
  const isDir = !node.entry && node.children.size > 0;
  const file = node.entry;

  if (isDir) {
    const sorted = [...node.children.values()].sort((a, b) => {
      const aDir = !a.entry && a.children.size > 0;
      const bDir = !b.entry && b.children.size > 0;
      if (aDir !== bDir) return aDir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });

    return (
      <div>
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex w-full items-center gap-1.5 rounded px-1.5 py-1 text-left text-sm hover:bg-white/[0.04] transition-colors"
          style={{ paddingLeft: `${depth * 16 + 6}px` }}
        >
          {expanded ? (
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground/50 shrink-0" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 text-muted-foreground/50 shrink-0" />
          )}
          <span className="text-muted-foreground/80">{node.name}/</span>
        </button>
        {expanded &&
          sorted.map((child) => (
            <FileTreeNode
              key={child.name}
              node={child}
              depth={depth + 1}
              onOpenFile={onOpenFile}
              selectedPath={selectedPath}
            />
          ))}
      </div>
    );
  }

  if (!file) return null;

  const ext = file.extension.toUpperCase();
  const pillColor = EXT_COLORS[file.extension] || "muted";
  const isSelected = file.relativePath === selectedPath;

  return (
    <button
      onClick={() => onOpenFile(file.relativePath)}
      className={cn(
        "flex w-full items-center gap-2 rounded px-1.5 py-1 text-left text-sm transition-colors group",
        isSelected ? "bg-sky-400/[0.08] border-l-2 border-sky-400/40" : "hover:bg-white/[0.04]"
      )}
      style={{ paddingLeft: `${depth * 16 + 6}px` }}
    >
      <FileText className="h-3.5 w-3.5 text-muted-foreground/40 shrink-0" />
      <span className="flex-1 truncate">{node.name}</span>
      {file.status === "modified" && (
        <span className="h-2 w-2 shrink-0 rounded-full bg-[#c4a44a]" title="Modified" />
      )}
      {ext && (
        <InfoPill
          color={pillColor}
          className="text-[10px] py-0 px-1.5 opacity-60 group-hover:opacity-100"
        >
          {ext}
        </InfoPill>
      )}
      <span className="text-[10px] text-muted-foreground/30 tabular-nums">
        {formatSize(file.sizeBytes)}
      </span>
    </button>
  );
}

export function AddonFileBrowser({ addonsPath, folderName }: AddonFileBrowserProps) {
  const [fileTree, setFileTree] = useState<AddonFileTree | null>(null);
  const [rescanning, setRescanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [loadState, setLoadState] = useState<"loading" | "done" | "error">("loading");
  const [editingFile, setEditingFile] = useState<string | null>(null);
  const [backups, setBackups] = useState<EditBackupManifest[]>([]);
  const [showBackups, setShowBackups] = useState(false);
  const [restoringFile, setRestoringFile] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invokeOrThrow<AddonFileTree>("list_addon_files", { addonsPath, folderName })
      .then((tree) => {
        if (!cancelled) {
          setFileTree(tree);
          setLoadState("done");
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(String(e));
          setLoadState("error");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [addonsPath, folderName, refreshKey]);

  const handleRescan = async () => {
    setRescanning(true);
    try {
      await invokeOrThrow<string[]>("rescan_addon_hashes", { addonsPath, folderName });
      setRefreshKey((k) => k + 1);
    } catch (e) {
      setError(String(e));
    } finally {
      setRescanning(false);
    }
  };

  const handleOpenFolder = async () => {
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(`${addonsPath}\\${folderName}`);
    } catch {
      // best-effort
    }
  };

  const handleOpenExternal = async (relativePath: string) => {
    try {
      const { openPath } = await import("@tauri-apps/plugin-opener");
      const fullPath = `${addonsPath}\\${folderName}\\${relativePath.replace(/\//g, "\\")}`;
      await openPath(fullPath);
    } catch {
      // best-effort
    }
  };

  const handleOpenFile = (relativePath: string) => {
    setEditingFile(relativePath);
  };

  const handleLoadBackups = async () => {
    setShowBackups(!showBackups);
    if (!showBackups) {
      try {
        const list = await invokeOrThrow<EditBackupManifest[]>("list_edit_backups", {
          addonsPath,
          folderName,
        });
        setBackups(list);
      } catch {
        setBackups([]);
      }
    }
  };

  const handleRestore = async (backedUpAt: string, relativePath: string) => {
    setRestoringFile(relativePath);
    try {
      await invokeOrThrow("restore_edit_backup", {
        addonsPath,
        folderName,
        backedUpAt,
        relativePath,
      });
      toast.success(`Restored ${relativePath}`);
      setRefreshKey((k) => k + 1);
    } catch (e) {
      toast.error(`Restore failed: ${String(e)}`);
    } finally {
      setRestoringFile(null);
    }
  };

  if (loadState === "loading" && !fileTree) {
    return (
      <div className="flex items-center justify-center py-12 text-muted-foreground/50 text-sm">
        <div className="h-4 w-4 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a] mr-2" />
        Loading files...
      </div>
    );
  }

  if (error) {
    return (
      <GlassPanel variant="subtle" className="p-4 text-sm text-red-400/80">
        Failed to load files: {error}
      </GlassPanel>
    );
  }

  if (!fileTree) return null;

  const tree = buildTree(fileTree.files);
  const sorted = [...tree.children.values()].sort((a, b) => {
    const aDir = !a.entry && a.children.size > 0;
    const bDir = !b.entry && b.children.size > 0;
    if (aDir !== bDir) return aDir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

  const editingEntry = editingFile
    ? fileTree.files.find((f) => f.relativePath === editingFile)
    : null;

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2 flex-wrap">
        <Button variant="outline" size="sm" onClick={handleOpenFolder}>
          <FolderOpen className="h-3.5 w-3.5 mr-1.5" />
          Open Folder
        </Button>
        {editingFile && (
          <Button variant="outline" size="sm" onClick={() => handleOpenExternal(editingFile)}>
            <ExternalLink className="h-3.5 w-3.5 mr-1.5" />
            Open in Editor
          </Button>
        )}
        <Button variant="ghost" size="sm" onClick={handleRescan} disabled={rescanning}>
          <RotateCw className={cn("h-3.5 w-3.5 mr-1.5", rescanning && "animate-spin")} />
          Rescan
        </Button>
        <Button variant="ghost" size="sm" onClick={handleLoadBackups}>
          <History className="h-3.5 w-3.5 mr-1.5" />
          Backups
        </Button>
      </div>

      <GlassPanel variant="subtle" className="p-2">
        <div className="max-h-[300px] overflow-y-auto">
          {sorted.map((child) => (
            <FileTreeNode
              key={child.name}
              node={child}
              depth={0}
              onOpenFile={handleOpenFile}
              selectedPath={editingFile}
            />
          ))}
        </div>
      </GlassPanel>

      {fileTree.modifiedCount > 0 && (
        <div className="flex items-center gap-2 text-xs text-muted-foreground/60">
          <span className="h-2 w-2 rounded-full bg-[#c4a44a]" />
          {fileTree.modifiedCount} file{fileTree.modifiedCount !== 1 ? "s" : ""} edited
          <span className="text-muted-foreground/30">·</span>
          Protected on update
        </div>
      )}

      {editingFile && (
        <AddonFileEditor
          addonsPath={addonsPath}
          folderName={folderName}
          relativePath={editingFile}
          isModified={editingEntry?.status === "modified"}
          onClose={() => setEditingFile(null)}
          onSaved={() => setRefreshKey((k) => k + 1)}
        />
      )}

      {showBackups && (
        <div className="space-y-2">
          <SectionHeader>Edit Backups</SectionHeader>
          {backups.length === 0 ? (
            <p className="text-xs text-muted-foreground/50">
              No backups yet. Backups are created when your edited files are overwritten by an
              update.
            </p>
          ) : (
            backups.map((backup) => (
              <GlassPanel key={backup.backedUpAt} variant="subtle" className="p-3">
                <div className="flex items-center justify-between mb-1.5">
                  <span className="text-xs font-medium text-muted-foreground/80">
                    {backup.updateFrom} → {backup.updateTo}
                  </span>
                  <span className="text-[10px] text-muted-foreground/40">{backup.backedUpAt}</span>
                </div>
                <div className="space-y-1">
                  {backup.files.map((file) => (
                    <div key={file} className="flex items-center gap-2 text-xs">
                      <span className="flex-1 font-mono truncate text-muted-foreground/70">
                        {file}
                      </span>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-6 px-2 text-[10px]"
                        onClick={() => handleRestore(backup.backedUpAt, file)}
                        disabled={restoringFile === file}
                      >
                        <Undo2 className="h-3 w-3 mr-1" />
                        {restoringFile === file ? "Restoring..." : "Restore"}
                      </Button>
                    </div>
                  ))}
                </div>
              </GlassPanel>
            ))
          )}
        </div>
      )}
    </div>
  );
}
