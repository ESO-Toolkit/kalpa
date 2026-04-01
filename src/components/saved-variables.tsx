import { useState, useEffect, useCallback, useMemo, memo } from "react";
import { toast } from "sonner";
import type { SavedVariableFile, SvTreeNode } from "../types";
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
} from "lucide-react";

interface SavedVariablesProps {
  addonsPath: string;
  onClose: () => void;
}

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
  const currentPath = [...path, node.key];

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

// ─── Browse Tab ──────────────────────────────────────────────

function BrowseTab({
  files,
  loading,
  onRefresh,
  onSelectFile,
}: {
  files: SavedVariableFile[];
  loading: boolean;
  onRefresh: () => void;
  onSelectFile: (f: SavedVariableFile) => void;
}) {
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <p className="text-sm text-muted-foreground">
          {files.length} SavedVariables file{files.length !== 1 ? "s" : ""}
        </p>
        <Button size="sm" variant="outline" onClick={onRefresh} disabled={loading}>
          <RefreshCwIcon className={`mr-1 size-3 ${loading ? "animate-spin" : ""}`} />
          Refresh
        </Button>
      </div>

      <div className="max-h-[400px] overflow-y-auto space-y-1">
        {files.length === 0 ? (
          <div className="py-8 text-center">
            <FileTextIcon className="mx-auto mb-2 size-8 text-muted-foreground/30" />
            <p className="text-sm text-muted-foreground">No SavedVariables files found.</p>
            <p className="mt-1 text-xs text-muted-foreground/60">
              Make sure your ESO AddOns path is set correctly and you have launched the game at
              least once.
            </p>
          </div>
        ) : (
          files.map((f) => (
            <button
              key={f.fileName}
              onClick={() => onSelectFile(f)}
              className="flex w-full items-center justify-between rounded-xl border border-white/[0.06] bg-white/[0.02] p-3 text-left transition-all duration-200 hover:border-white/[0.1]"
            >
              <div className="min-w-0 flex-1">
                <div className="font-medium text-sm truncate">{f.addonName}</div>
                <div className="text-xs text-muted-foreground mt-0.5">
                  {formatBytes(f.sizeBytes)} &middot; {formatDate(f.lastModified)}
                </div>
              </div>
              <div className="flex items-center gap-2 ml-2 shrink-0">
                {f.characterKeys.length > 0 && (
                  <InfoPill color="sky">
                    {f.characterKeys.length} profile{f.characterKeys.length !== 1 ? "s" : ""}
                  </InfoPill>
                )}
                <ChevronRightIcon className="size-4 text-muted-foreground/40" />
              </div>
            </button>
          ))
        )}
      </div>
    </div>
  );
}

// ─── Editor Tab ──────────────────────────────────────────────

function EditorTab({
  files,
  addonsPath,
  initialFile,
}: {
  files: SavedVariableFile[];
  addonsPath: string;
  initialFile: string;
}) {
  const [selectedFile, setSelectedFile] = useState<string>(initialFile);
  const [tree, setTree] = useState<SvTreeNode | null>(null);
  const [loading, setLoading] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [esoRunning, setEsoRunning] = useState(false);

  // Sync when parent changes the initial file
  useEffect(() => {
    if (initialFile) {
      setSelectedFile(initialFile);
    }
  }, [initialFile]);

  useEffect(() => {
    invokeOrThrow<boolean>("is_eso_running")
      .then(setEsoRunning)
      .catch(() => {});
  }, []);

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
          className="flex-1 rounded-lg border border-white/[0.1] bg-transparent px-2 py-1.5 text-sm"
          value={selectedFile}
          onChange={(e) => setSelectedFile(e.target.value)}
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
                path={[]}
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
          className="mt-1 w-full rounded-lg border border-white/[0.1] bg-transparent px-2 py-1.5 text-sm"
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
            className="mt-1 w-full rounded-lg border border-white/[0.1] bg-transparent px-2 py-1.5 text-sm"
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
            className="mt-1 w-full rounded-lg border border-white/[0.1] bg-transparent px-2 py-1.5 text-sm"
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
            {" → "}
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

export function SavedVariables({ addonsPath, onClose }: SavedVariablesProps) {
  const [files, setFiles] = useState<SavedVariableFile[]>([]);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<string>("browse");
  const [editorFile, setEditorFile] = useState<string>("");

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

  const handleSelectFile = useCallback((f: SavedVariableFile) => {
    setEditorFile(f.fileName);
    setActiveTab("editor");
  }, []);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>SavedVariables Manager</DialogTitle>
        </DialogHeader>

        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList variant="line">
            <TabsTrigger value="browse">Browse</TabsTrigger>
            <TabsTrigger value="editor">Editor</TabsTrigger>
            <TabsTrigger value="copy">Copy Profile</TabsTrigger>
          </TabsList>

          <TabsContent value="browse">
            <BrowseTab
              files={files}
              loading={loading}
              onRefresh={() => void loadFiles()}
              onSelectFile={handleSelectFile}
            />
          </TabsContent>

          <TabsContent value="editor">
            <EditorTab files={files} addonsPath={addonsPath} initialFile={editorFile} />
          </TabsContent>

          <TabsContent value="copy">
            <CopyProfileTab
              files={files}
              addonsPath={addonsPath}
              onRefresh={() => void loadFiles()}
            />
          </TabsContent>
        </Tabs>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
