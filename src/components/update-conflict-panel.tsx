import { useState, useCallback } from "react";
import type { FileConflict, FileDecision, DiffData } from "../types";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { DiffViewer } from "@/components/diff-viewer";
import { invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { Eye, Check, SkipForward } from "lucide-react";

interface UpdateConflictPanelProps {
  folderName: string;
  currentVersion: string;
  updateVersion: string;
  conflicts: FileConflict[];
  autoKeptFiles: string[];
  safeFileCount: number;
  sessionId: string;
  addonsPath: string;
  onResolve: (decisions: FileDecision[]) => void;
  onSkip: () => void;
}

export function UpdateConflictPanel({
  folderName,
  currentVersion,
  updateVersion,
  conflicts,
  autoKeptFiles,
  safeFileCount,
  sessionId,
  addonsPath,
  onResolve,
  onSkip,
}: UpdateConflictPanelProps) {
  const [decisions, setDecisions] = useState<Record<string, "keep_mine" | "take_update">>({});
  const [viewingDiff, setViewingDiff] = useState<string | null>(null);
  const [diffData, setDiffData] = useState<DiffData | null>(null);
  const [loadingDiff, setLoadingDiff] = useState(false);

  const setDecision = useCallback((path: string, action: "keep_mine" | "take_update") => {
    setDecisions((prev) => ({ ...prev, [path]: action }));
  }, []);

  const handleViewDiff = async (relativePath: string) => {
    if (viewingDiff === relativePath) {
      setViewingDiff(null);
      setDiffData(null);
      return;
    }
    setViewingDiff(relativePath);
    setLoadingDiff(true);
    try {
      const data = await invokeOrThrow<DiffData>("get_conflict_diff", {
        addonsPath,
        sessionId,
        relativePath,
      });
      setDiffData(data);
    } catch {
      setDiffData(null);
    } finally {
      setLoadingDiff(false);
    }
  };

  const allDecided = conflicts.every((c) => decisions[c.relativePath]);

  const handleApply = () => {
    const fileDecisions: FileDecision[] = conflicts.map((c) => ({
      relativePath: c.relativePath,
      action: decisions[c.relativePath] || "take_update",
    }));
    onResolve(fileDecisions);
  };

  return (
    <div className="space-y-4">
      <GlassPanel variant="subtle" className="p-4 border-amber-500/20! bg-amber-500/[0.04]!">
        <h3 className="font-heading text-sm font-semibold text-amber-400">
          {folderName} {currentVersion} → {updateVersion}
        </h3>
        <p className="mt-1.5 text-xs text-muted-foreground/70">
          You've edited {conflicts.length} file{conflicts.length !== 1 ? "s" : ""} that this update
          also changed. Choose which version to keep for each:
        </p>
      </GlassPanel>

      {autoKeptFiles.length > 0 && (
        <div className="text-xs text-emerald-400/70 flex items-center gap-1.5">
          <Check className="h-3.5 w-3.5" />
          {autoKeptFiles.length} of your edited files are unchanged in this update — automatically
          preserved
        </div>
      )}

      <div className="space-y-2">
        {conflicts.map((conflict) => {
          const decision = decisions[conflict.relativePath];
          const isViewingThis = viewingDiff === conflict.relativePath;

          return (
            <div key={conflict.relativePath}>
              <GlassPanel variant="subtle" className="p-3">
                <div className="flex items-center gap-2 mb-2">
                  <span className="font-mono text-sm flex-1 truncate">{conflict.relativePath}</span>
                  {decision && (
                    <span
                      className={cn(
                        "text-[10px] font-semibold uppercase tracking-wider",
                        decision === "keep_mine" ? "text-[#c4a44a]" : "text-sky-400"
                      )}
                    >
                      {decision === "keep_mine" ? "keeping yours" : "taking update"}
                    </span>
                  )}
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    variant={decision === "keep_mine" ? "default" : "outline"}
                    size="sm"
                    onClick={() => setDecision(conflict.relativePath, "keep_mine")}
                  >
                    Keep my version
                  </Button>
                  <Button
                    variant={decision === "take_update" ? "default" : "outline"}
                    size="sm"
                    onClick={() => setDecision(conflict.relativePath, "take_update")}
                  >
                    Take the update
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleViewDiff(conflict.relativePath)}
                  >
                    <Eye className="h-3.5 w-3.5 mr-1.5" />
                    {isViewingThis ? "Hide" : "View"} differences
                  </Button>
                </div>
              </GlassPanel>
              {isViewingThis && (
                <div className="mt-1">
                  {loadingDiff ? (
                    <div className="flex items-center justify-center py-4 text-muted-foreground/50 text-sm">
                      <div className="h-4 w-4 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a] mr-2" />
                      Loading differences...
                    </div>
                  ) : diffData?.isBinary ? (
                    <GlassPanel
                      variant="subtle"
                      className="p-3 text-sm text-muted-foreground/60 text-center"
                    >
                      Binary file — cannot show differences
                    </GlassPanel>
                  ) : diffData ? (
                    <DiffViewer
                      userContent={diffData.userContent}
                      upstreamContent={diffData.upstreamContent}
                      fileName={conflict.relativePath}
                    />
                  ) : null}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {safeFileCount > 0 && (
        <div className="text-xs text-muted-foreground/50">
          {safeFileCount} other file{safeFileCount !== 1 ? "s" : ""} will update normally (you
          haven't modified them)
        </div>
      )}

      <div className="flex items-center gap-3 pt-2">
        <Button onClick={handleApply} disabled={!allDecided}>
          <Check className="h-3.5 w-3.5 mr-1.5" />
          Apply & Update
        </Button>
        <Button variant="outline" onClick={onSkip}>
          <SkipForward className="h-3.5 w-3.5 mr-1.5" />
          Skip Update
        </Button>
      </div>
    </div>
  );
}
