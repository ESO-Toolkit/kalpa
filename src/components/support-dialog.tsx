import { useEffect, useMemo, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import {
  CheckCircle2,
  ClipboardCheck,
  ExternalLink,
  LifeBuoy,
  LoaderCircle,
  LockKeyhole,
} from "lucide-react";
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
import { FEEDBACK_DISCORD_SUPPORT_URL, openFeedbackUrl } from "@/lib/feedback";
import { osType } from "@/lib/platform";
import {
  buildSupportHandoffUrl,
  buildSupportTicketPayload,
  renderSupportReport,
  SUPPORT_ISSUES,
  type SupportIssueId,
} from "@/lib/support-report";
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
  const [consented, setConsented] = useState(false);
  const [openingHandoff, setOpeningHandoff] = useState(false);
  const [appVersion, setAppVersion] = useState("unknown");
  const [generatedAt] = useState(() => new Date());
  const [copiedReport, setCopiedReport] = useState<string | null>(null);

  useEffect(() => {
    void getVersion()
      .then(setAppVersion)
      .catch(() => setAppVersion("unknown"));
  }, []);

  const payload = useMemo(
    () =>
      buildSupportTicketPayload({
        issueId,
        description,
        appVersion,
        platform: osType(),
        generatedAt,
        isOffline,
        checkingUpdates,
        addonsPath,
        instanceLabel,
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
      instanceLabel,
      isOffline,
      issueId,
      lastError,
      updateResults,
    ]
  );
  const report = useMemo(() => renderSupportReport(payload), [payload]);
  const handoffUrl = useMemo(() => buildSupportHandoffUrl(payload), [payload]);
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

  async function createPrivateTicket() {
    if (!consented || openingHandoff) return;
    if (isOffline) {
      toast.error("Kalpa is offline. Your report is still ready to copy for later.");
      return;
    }
    if (!handoffUrl) {
      toast.error(
        "This report is too large for the secure handoff. Copy it and use the manual option."
      );
      return;
    }

    setOpeningHandoff(true);
    try {
      await openFeedbackUrl(handoffUrl);
      toast.info("Continue in your browser to sign in and create the ticket.");
    } catch {
      toast.error("Kalpa couldn't open the secure handoff. Your report is still available below.");
    } finally {
      setOpeningHandoff(false);
    }
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
                aria-describedby="support-description-count"
                value={description}
                onChange={(event) => {
                  setDescription(event.target.value);
                }}
                maxLength={500}
                rows={3}
                placeholder={selectedIssue.placeholder}
                className="w-full resize-none rounded-[10px] border border-structure-10 bg-structure-04 px-3 py-2 text-sm text-foreground shadow-[inset_0_1px_2px_var(--scrim-20),0_1px_0_var(--structure-02)] outline-none transition-all duration-150 placeholder:text-muted-foreground hover:border-structure-14 hover:bg-structure-05 focus-visible:border-accent-sky/40 focus-visible:shadow-[inset_0_1px_2px_var(--scrim-20),0_0_0_2px_color-mix(in_oklab,var(--accent-sky)_12%,transparent)]"
              />
              <span
                id="support-description-count"
                className="mt-1 block text-right text-[10px] tabular-nums text-muted-foreground"
              >
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
                  SavedVariables, file contents, account IDs, login tokens, and your full local
                  folder path are never included.
                </p>
              </div>
              <label className="flex cursor-pointer items-start gap-2 border-t border-structure-06 pt-3 text-xs">
                <Checkbox
                  nativeButton
                  checked={consented}
                  onCheckedChange={(checked) => {
                    setConsented(checked === true);
                  }}
                  aria-label="I agree to share the exact report shown below with ESO Toolkit support"
                />
                <span>
                  <span className="font-medium text-foreground">
                    I agree to share the exact report shown below
                  </span>
                  <span className="mt-0.5 block text-muted-foreground">
                    You will sign in with Discord in your browser before anything is sent.
                  </span>
                </span>
              </label>
            </GlassPanel>

            <details className="group rounded-xl border border-structure-06 bg-structure-02" open>
              <summary className="cursor-pointer px-3 py-2.5 font-heading text-xs font-semibold text-foreground marker:text-muted-foreground">
                Exact support report
              </summary>
              <pre
                aria-label="Exact support report that will be shared"
                tabIndex={0}
                className="max-h-56 overflow-auto whitespace-pre-wrap border-t border-structure-06 bg-scrim-10 px-3 py-2.5 font-mono text-xs leading-relaxed text-muted-foreground select-text"
              >
                {report}
              </pre>
            </details>
          </section>

          <div aria-live="polite">
            <p className="sr-only">
              Your support report is prepared. A ticket has not been created yet.
            </p>
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
                    If the secure handoff is unavailable, join the server, choose{" "}
                    <strong className="text-foreground">Create ticket</strong>, paste the report
                    into Description, and submit. The ticket is private to you and the support team.
                  </p>
                </div>
              </GlassPanel>
            )}
          </div>
        </div>

        <DialogFooter className="mx-0 mb-0 gap-2 px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex flex-wrap gap-1">
            <Button variant="ghost" size="sm" onClick={() => void copyReport()}>
              <ClipboardCheck />
              Copy report
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void openFeedbackUrl(FEEDBACK_DISCORD_SUPPORT_URL)}
            >
              Manual ticket desk
              <ExternalLink />
            </Button>
          </div>
          <Button
            disabled={!consented || isOffline || !handoffUrl || openingHandoff}
            onClick={() => void createPrivateTicket()}
          >
            {openingHandoff ? <LoaderCircle className="animate-spin" /> : <LockKeyhole />}
            {openingHandoff ? "Opening secure sign-in…" : "Create private ticket"}
            {!openingHandoff && <ExternalLink />}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
