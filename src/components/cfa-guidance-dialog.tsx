import { useState } from "react";
import { ShieldAlert, ExternalLink, Copy, Check } from "lucide-react";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { invokeResult } from "@/lib/tauri";

interface CfaGuidanceDialogProps {
  open: boolean;
  onClose: () => void;
  /** Absolute path to the Kalpa exe the user must allow. */
  exePath: string;
  /** True if the block was a permission denial (lets us hedge wording). */
  permissionDenied: boolean;
}

/**
 * Explains that Windows is blocking Kalpa from writing to the AddOns folder
 * (most often Controlled Folder Access) and gives the user a one-click path to
 * the Windows Security page plus copy-pasteable steps. Shown proactively when a
 * write probe fails, and as a fallback after an update fails for this reason.
 */
export function CfaGuidanceDialog({
  open,
  onClose,
  exePath,
  permissionDenied,
}: CfaGuidanceDialogProps) {
  const [copied, setCopied] = useState(false);

  const openSettings = async () => {
    const result = await invokeResult("open_ransomware_protection_settings");
    if (!result.ok) {
      toast.error(`Could not open Windows Security: ${result.error}`);
    }
  };

  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(exePath);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  };

  const steps = [
    'Click "Open Windows Security" below.',
    "Go to Ransomware protection → Manage ransomware protection.",
    "Select Allow an app through Controlled folder access (you may need admin approval).",
    'Click Add an allowed app → Browse all apps, then add the path below ("kalpa.exe").',
    "Return to Kalpa and try updating again.",
  ];

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldAlert className="size-5 text-amber-400" />
            Windows is blocking Kalpa
          </DialogTitle>
          <DialogDescription>
            Kalpa can&apos;t write to your AddOns folder, so installs and updates will fail.
            {permissionDenied
              ? " This is most often Windows Controlled Folder Access (ransomware protection), but can also be a read-only file, restrictive permissions, or antivirus."
              : " Check that the folder isn't read-only and that antivirus isn't blocking it."}
          </DialogDescription>
        </DialogHeader>

        <ol className="space-y-2 text-sm text-foreground/80">
          {steps.map((step, i) => (
            <li key={i} className="flex gap-2.5">
              <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-white/[0.06] text-xs font-medium text-foreground/70">
                {i + 1}
              </span>
              <span>{step}</span>
            </li>
          ))}
        </ol>

        {exePath && (
          <div className="flex min-w-0 items-center gap-2 rounded-lg border border-white/[0.06] bg-white/[0.03] p-2">
            <code
              className="min-w-0 flex-1 truncate font-mono text-xs text-foreground/70"
              title={exePath}
            >
              {exePath}
            </code>
            <Button variant="ghost" size="icon-sm" onClick={copyPath} aria-label="Copy path">
              {copied ? <Check className="text-emerald-400" /> : <Copy />}
            </Button>
          </div>
        )}

        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
          <Button onClick={openSettings}>
            <ExternalLink data-icon="inline-start" />
            Open Windows Security
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
