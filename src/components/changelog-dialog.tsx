import { openUrl } from "@tauri-apps/plugin-opener";
import { ArrowRightIcon, ExternalLink, ScrollText } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { ChangelogView } from "@/components/changelog-view";
import { useEsouiDetail } from "@/hooks/use-esoui-detail";

interface ChangelogDialogProps {
  esouiId: number;
  title: string;
  currentVersion?: string;
  remoteVersion?: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Four shimmer lines standing in for the changelog body while it loads. */
function ChangelogSkeleton() {
  const widths = ["92%", "78%", "85%", "64%"];
  return (
    <div className="space-y-2.5 rounded-xl border border-structure-06 bg-gradient-to-b from-structure-03 to-structure-01 p-4">
      {widths.map((width, idx) => (
        <Skeleton
          key={width}
          className="h-3.5 rounded"
          style={{ width, "--shimmer-delay": `${idx * 80}ms` } as React.CSSProperties}
        />
      ))}
    </div>
  );
}

/**
 * Shows an addon's full ESOUI version history.
 *
 * Deliberately not a "changes since your version" delta: ESOUI changelogs are
 * author-freeform (some head each entry with a bold version, others with a bare
 * `2.0 r43` line), so there is no delimiter a slicer could trust — it would
 * silently truncate. The header carries the version context instead.
 */
export function ChangelogDialog({
  esouiId,
  title,
  currentVersion,
  remoteVersion,
  open,
  onOpenChange,
}: ChangelogDialogProps) {
  // Fetch only while open; the hook's shared cache usually makes this instant.
  const { detail, loading, error, refetch } = useEsouiDetail(esouiId, open);

  const showVersions = Boolean(currentVersion && remoteVersion);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ScrollText className="size-4 shrink-0 text-primary" />
            <span className="truncate">{title}</span>
          </DialogTitle>
          <DialogDescription>
            {showVersions ? (
              <span className="flex items-center gap-1 text-[11px] tabular-nums">
                <span className="max-w-[140px] truncate text-muted-foreground">
                  v{currentVersion}
                </span>
                <ArrowRightIcon className="h-3 w-3 text-muted-foreground" aria-hidden="true" />
                <span className="max-w-[140px] truncate font-medium text-primary">
                  v{remoteVersion}
                </span>
              </span>
            ) : (
              "Full version history from ESOUI, newest first."
            )}
          </DialogDescription>
        </DialogHeader>

        <div className="max-h-[55vh] overflow-y-auto pr-1">
          {loading && !detail ? (
            <ChangelogSkeleton />
          ) : error ? (
            <div className="flex flex-col items-start gap-3">
              <div className="w-full rounded-xl border border-status-danger/20 bg-status-danger/[0.04] p-4 text-sm text-status-danger">
                {error}
              </div>
              <Button variant="outline" onClick={refetch}>
                Retry
              </Button>
            </div>
          ) : detail && detail.changeLog ? (
            <ChangelogView
              changeLog={detail.changeLog}
              variant="dialog"
              installedVersion={currentVersion}
              archivedVersions={detail.archivedVersions}
              latestDate={detail.updated}
            />
          ) : detail ? (
            <div className="flex flex-col items-center gap-2 rounded-xl border border-structure-06 bg-gradient-to-b from-structure-03 to-structure-01 px-4 py-8 text-center">
              <ScrollText className="size-6 text-muted-foreground/60" aria-hidden="true" />
              <p className="text-sm text-muted-foreground">
                This author hasn&apos;t published a changelog.
              </p>
              <button
                onClick={() => openUrl(`https://www.esoui.com/downloads/info${esouiId}.html`)}
                className="flex cursor-pointer items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
              >
                <ExternalLink className="size-3" />
                View on ESOUI
              </button>
            </div>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
