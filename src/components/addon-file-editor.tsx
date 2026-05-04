import { useState, useEffect, useCallback } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { StreamLanguage } from "@codemirror/language";
import { lua } from "@codemirror/legacy-modes/mode/lua";
import { xml } from "@codemirror/lang-xml";
import { kalpaTheme } from "@/lib/kalpa-codemirror-theme";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { toast } from "sonner";
import { Save, Undo2, Pencil, X } from "lucide-react";

interface AddonFileEditorProps {
  addonsPath: string;
  folderName: string;
  relativePath: string;
  isModified: boolean;
  onClose: () => void;
  onSaved: () => void;
}

const BINARY_EXTENSIONS = new Set(["dds", "ttf", "otf", "png", "jpg", "jpeg", "gif", "bmp", "tga"]);

function getExtension(path: string): string {
  const dot = path.lastIndexOf(".");
  return dot >= 0 ? path.slice(dot + 1).toLowerCase() : "";
}

function getLanguageExtension(ext: string) {
  if (ext === "lua") return StreamLanguage.define(lua);
  if (ext === "xml") return xml();
  return undefined;
}

export function AddonFileEditor({
  addonsPath,
  folderName,
  relativePath,
  isModified,
  onClose,
  onSaved,
}: AddonFileEditorProps) {
  const [content, setContent] = useState<string | null>(null);
  const [originalContent, setOriginalContent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [editable, setEditable] = useState(isModified);

  const ext = getExtension(relativePath);
  const isBinary = BINARY_EXTENSIONS.has(ext);
  const fileName = relativePath.split("/").pop() ?? relativePath;
  const [loadState, setLoadState] = useState<"loading" | "done">(isBinary ? "done" : "loading");

  useEffect(() => {
    if (isBinary) return;
    let cancelled = false;
    invokeOrThrow<string>("read_addon_file", { addonsPath, folderName, relativePath })
      .then((text) => {
        if (!cancelled) {
          setContent(text);
          setOriginalContent(text);
          setLoadState("done");
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(getTauriErrorMessage(e));
          setLoadState("done");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [addonsPath, folderName, relativePath, isBinary]);

  const dirty = content !== null && originalContent !== null && content !== originalContent;

  const handleSave = useCallback(async () => {
    if (content === null) return;
    setSaving(true);
    try {
      await invokeOrThrow("write_addon_file", {
        addonsPath,
        folderName,
        relativePath,
        content,
      });
      setOriginalContent(content);
      toast.success(`Saved ${fileName}`);
      onSaved();
    } catch (e) {
      toast.error(`Failed to save: ${getTauriErrorMessage(e)}`);
    } finally {
      setSaving(false);
    }
  }, [addonsPath, folderName, relativePath, content, fileName, onSaved]);

  const handleRevert = useCallback(() => {
    if (originalContent !== null) {
      setContent(originalContent);
    }
  }, [originalContent]);

  const langExt = getLanguageExtension(ext);

  if (loadState === "loading") {
    return (
      <div className="flex items-center justify-center py-8 text-muted-foreground/50 text-sm">
        <div className="h-4 w-4 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a] mr-2" />
        Loading file...
      </div>
    );
  }

  if (isBinary) {
    return (
      <GlassPanel variant="subtle" className="p-4">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-medium">{fileName}</p>
            <p className="text-xs text-muted-foreground/60 mt-1">
              Binary file — cannot edit in Kalpa
            </p>
          </div>
          <Button variant="ghost" size="sm" onClick={onClose}>
            <X className="h-3.5 w-3.5" />
          </Button>
        </div>
      </GlassPanel>
    );
  }

  if (error) {
    return (
      <GlassPanel variant="subtle" className="p-4">
        <div className="flex items-center justify-between">
          <p className="text-sm text-red-400/80">{error}</p>
          <Button variant="ghost" size="sm" onClick={onClose}>
            <X className="h-3.5 w-3.5" />
          </Button>
        </div>
      </GlassPanel>
    );
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <span className="font-mono text-sm truncate flex-1">{relativePath}</span>
        {dirty && (
          <span className="text-[10px] font-semibold uppercase tracking-wider text-amber-400">
            unsaved
          </span>
        )}
        {!editable && (
          <Button variant="outline" size="sm" onClick={() => setEditable(true)}>
            <Pencil className="h-3.5 w-3.5 mr-1.5" />
            Enable Editing
          </Button>
        )}
        {editable && (
          <>
            <Button variant="outline" size="sm" onClick={handleRevert} disabled={!dirty}>
              <Undo2 className="h-3.5 w-3.5 mr-1.5" />
              Revert
            </Button>
            <Button size="sm" onClick={handleSave} disabled={!dirty || saving}>
              <Save className="h-3.5 w-3.5 mr-1.5" />
              {saving ? "Saving..." : "Save"}
            </Button>
          </>
        )}
        <Button variant="ghost" size="sm" onClick={onClose}>
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>

      {!editable && (
        <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-1.5 text-xs text-muted-foreground/50">
          This file hasn't been edited. Click "Enable Editing" to modify it.
        </div>
      )}

      <GlassPanel variant="subtle" className="overflow-hidden rounded-lg">
        <CodeMirror
          value={content ?? ""}
          onChange={editable ? setContent : undefined}
          theme={kalpaTheme}
          extensions={langExt ? [langExt] : []}
          readOnly={!editable}
          basicSetup={{
            lineNumbers: true,
            bracketMatching: true,
            closeBrackets: true,
            searchKeymap: true,
            foldGutter: true,
            highlightActiveLineGutter: true,
            highlightActiveLine: true,
          }}
          height="400px"
          className="text-xs"
        />
      </GlassPanel>
    </div>
  );
}
