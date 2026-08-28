import { useEffect, useMemo, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { CheckCircle2, ClipboardCheck, ExternalLink, LifeBuoy, LockKeyhole } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import {
  FEEDBACK_DISCORD_SUPPORT_URL,
  FEEDBACK_DISCORD_URL,
  openFeedbackUrl,
} from "@/lib/feedback";
import { osType } from "@/lib/platform";
import { buildSupportReport, SUPPORT_ISSUES, type SupportIssueId } from "@/lib/support-report";
import { cn } from "@/lib/utils";
import type { AddonManifest, UpdateCheckResult } from "@/types";

interface SupportDialogProps {
  addons: AddonManifest[];
  addonsPath: string;
  checkingUpdates: boolean;
  instanceLabel: string | null;
  isOffline: boolean;
  lastError: string | null;
  onClose: () => void;
  updateResults: UpdateCheckResult[];
}

export function SupportDialog({
  addons,
  addonsPath,
  checkingUpdates,
  instanceLabel,
  isOffline,
  lastError,
  onClose,
  updateResults,
}: SupportDialogProps) {
  const [issueId, setIssueId] = useState<SupportIssueId>("addon-status");
  const [description, setDescription] = useState("");
  const [includeAddonsPath, setIncludeAddonsPath] = useState(false);
  const [appVersion, setAppVersion] = useState("unknown");
  const [generatedAt] = useState(() => new Date());
  const [copiedReport, setCopiedReport] = useState<string | null>(null);

  useEffect(() => {
    void getVersion()
      .then(setAppVersion)
      .catch(() => setAppVersion("unknown"));
  }, []);

  const report = useMemo(
    () =>
      buildSupportReport({
        issueId,
        description,
        appVersion,
        platform: osType(),
        generatedAt,
        isOffline,
        checkingUpdates,
        addonsPath,
        instanceLabel,
        includeAddonsPath,
        addons,
        updateResults,
        lastError,
      }),
    [
      addons,
      addonsPath,
      appVersion,
      checkingUpdates,
      description,
      generatedAt,
      includeAddonsPath,
      instanceLabel,
      isOffline,
      issueId,
      lastError,
      updateResults,
    ]
  );
  const copied = copiedReport === report;
  const selectedIssue = SUPPORT_ISSUES.find((issue) => issue.id === issueId)!;

  async function copyReport(): Promise<boolean> {
    try {
      await navigator.clipboard.writeText(report);
      setCopiedReport(report);
      toast.success("Support report copied");
      return true;
    } catch {
      toast.error("Kalpa couldn't copy the report. Select the preview and copy it manually.");
      return false;
    }
  }

  async function continueInDiscord() {
    if (!(await copyReport())) return;
    await openFeedbackUrl(FEEDBACK_DISCORD_SUPPORT_URL);
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex max-h-[92vh] flex-col gap-0 p-0 sm:max-w-2xl">
        <DialogHeader className="mx-0 mt-0 px-5 pt-5">
          <DialogTitle className="flex items-center gap-2">
            <LifeBuoy className="size-4 text-primary" />
            Help me with Kalpa
          </DialogTitle>
          <DialogDescription>
            Tell us what went wrong. Kalpa will prepare the useful details so you do not have to
            know what logs to find.
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 space-y-4 overflow-y-auto px-5 py-4">
          <section className="space-y-2">
            <SectionHeader id="support-issue-heading">
              1 · What do you need help with?
            </SectionHeader>
            <div
              className="grid gap-2 sm:grid-cols-2"
              role="radiogroup"
              aria-labelledby="support-issue-heading"
            >
              {SUPPORT_ISSUES.map((issue) => {
                const selected = issueId === issue.id;
                return (
                  <label
                    key={issue.id}
                    className={cn(
                      "cursor-pointer rounded-xl border p-3 text-left transition-colors duration-150 focus-within:outline-none focus-within:ring-2 focus-within:ring-accent-sky/30",
                      selected
                        ? "border-accent-sky/35 bg-accent-sky/[0.07] shadow-[inset_3px_0_0_var(--accent-sky)]"
                        : "border-structure-06 bg-structure-02 hover:border-structure-12 hover:bg-structure-04"
                    )}
                  >
                    <input
                      type="radio"
                      name="support-issue"
                      value={issue.id}
                      checked={selected}
                      onChange={() => setIssueId(issue.id)}
                      className="sr-only"
                    />
                    <span className="flex items-center gap-2 font-heading text-xs font-semibold text-foreground">
                      {selected && <CheckCircle2 className="size-3.5 text-accent-sky" />}
                      {issue.label}
                    </span>
                    <span className="mt-1 block text-[11px] leading-relaxed text-muted-foreground">
                      {issue.description}
                    </span>
                  </label>
                );
              })}
            </div>

            <label className="block pt-1">
              <span className="mb-1.5 block text-xs font-medium text-foreground">
                What happened? <span className="font-normal text-muted-foreground">(optional)</span>
              </span>
              <textarea
                value={description}
                onChange={(event) => {
                  setDescription(event.target.value);
                }}
                maxLength={500}
                rows={3}
                placeholder={selectedIssue.placeholder}
                className="w-full resize-none rounded-[10px] border border-structure-10 bg-structure-04 px-3 py-2 text-sm text-foreground shadow-[inset_0_1px_2px_var(--scrim-20),0_1px_0_var(--structure-02)] outline-none transition-all duration-150 placeholder:text-muted-foreground hover:border-structure-14 hover:bg-structure-05 focus-visible:border-accent-sky/40 focus-visible:shadow-[inset_0_1px_2px_var(--scrim-20),0_0_0_2px_color-mix(in_oklab,var(--accent-sky)_12%,transparent)]"
              />
              <span className="mt-1 block text-right text-[10px] tabular-nums text-muted-foreground">
                {description.length}/500
              </span>
            </label>
          </section>

          <section className="space-y-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <SectionHeader>2 · Review what will be shared</SectionHeader>
              <div className="flex flex-wrap gap-1.5">
                <InfoPill color={isOffline ? "amber" : "emerald"}>
                  {isOffline ? "Offline" : "Online"}
                </InfoPill>
                <InfoPill color="muted">{addons.length} addons</InfoPill>
                <InfoPill color="amber">
                  {updateResults.filter((result) => result.hasUpdate).length} updates seen
                </InfoPill>
              </div>
            </div>

            <GlassPanel variant="subtle" className="p-3">
              <div className="mb-3 flex items-start gap-2 text-xs text-muted-foreground">
                <LockKeyhole className="mt-0.5 size-3.5 shrink-0 text-status-success" />
                <p className="leading-relaxed">
                  SavedVariables, file contents, account IDs, and login tokens are never included.
                  Your local folder path stays hidden unless you choose to share it.
                </p>
              </div>
              <label className="flex cursor-pointer items-start gap-2 border-t border-structure-06 pt-3 text-xs">
                <Checkbox
                  nativeButton
                  checked={includeAddonsPath}
                  onCheckedChange={(checked) => {
                    setIncludeAddonsPath(checked === true);
                  }}
                  aria-label="Include my full AddOns folder path"
                />
                <span>
                  <span className="font-medium text-foreground">
                    Include my full AddOns folder path
                  </span>
                  <span className="mt-0.5 block text-muted-foreground">
                    This can reveal your computer account name. It is usually not needed.
                  </span>
                </span>
              </label>
            </GlassPanel>

            <details className="group rounded-xl border border-structure-06 bg-structure-02" open>
              <summary className="cursor-pointer px-3 py-2.5 font-heading text-xs font-semibold text-foreground marker:text-muted-foreground">
                Exact support report
              </summary>
              <pre className="max-h-56 overflow-auto whitespace-pre-wrap border-t border-structure-06 bg-scrim-10 px-3 py-2.5 font-mono text-xs leading-relaxed text-muted-foreground select-text">
                {report}
              </pre>
            </details>
          </section>

          <div aria-live="polite">
            {copied && (
              <GlassPanel
                variant="subtle"
                className="flex items-start gap-2 border-status-success/20 bg-status-success/[0.04] p-3"
                role="status"
              >
                <ClipboardCheck className="mt-0.5 size-4 shrink-0 text-status-success" />
                <div>
                  <p className="text-xs font-semibold text-foreground">Your report is copied.</p>
                  <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
                    1. Join the server if Discord asks. 2. Choose{" "}
                    <strong className="text-foreground">Create ticket</strong>. 3. Paste the report
                    into Description and submit. The ticket is private to you and the support team.
                  </p>
                </div>
              </GlassPanel>
            )}
          </div>
        </div>

        <DialogFooter className="mx-0 mb-0 items-center justify-between px-5 py-4 sm:justify-between">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void openFeedbackUrl(FEEDBACK_DISCORD_URL)}
          >
            New to the server? Join first
          </Button>
          <div className="flex flex-col-reverse gap-2 sm:flex-row">
            <Button variant="outline" onClick={() => void copyReport()}>
              <ClipboardCheck />
              Copy only
            </Button>
            <Button onClick={() => void continueInDiscord()}>
              Copy &amp; open ticket desk
              <ExternalLink />
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
