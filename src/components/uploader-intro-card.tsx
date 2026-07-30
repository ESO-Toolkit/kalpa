import { CloudUpload, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";

interface UploaderIntroCardProps {
  onOpenLogUpload: () => void;
  onDismiss: () => void;
}

export function UploaderIntroCard({ onOpenLogUpload, onDismiss }: UploaderIntroCardProps) {
  const titleId = "uploader-intro-title";

  return (
    <section aria-labelledby={titleId} className="mx-4 mb-2">
      <GlassPanel
        variant="subtle"
        className="flex flex-wrap items-center justify-between gap-3 border-primary/20 bg-primary/[0.04] px-3 py-2"
      >
        <div className="flex min-w-0 items-start gap-2.5">
          <div className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-lg border border-primary/20 bg-primary/[0.08] text-primary">
            <CloudUpload className="size-3.5" aria-hidden />
          </div>
          <div className="min-w-0">
            <h2 id={titleId} className="text-sm font-medium text-foreground">
              You have ESO combat logs on this PC
            </h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Kalpa can turn your <span className="font-mono text-foreground">Encounter.log</span>{" "}
              into a shareable ESO Logs report - no separate uploader app needed. Sign in with ESO
              Logs when you're ready.
            </p>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button type="button" variant="outline" size="sm" onClick={onOpenLogUpload}>
            Open the log uploader
          </Button>
          <Button type="button" variant="ghost" size="sm" onClick={onDismiss}>
            <X className="size-3.5" />
            Not now
          </Button>
        </div>
      </GlassPanel>
    </section>
  );
}
