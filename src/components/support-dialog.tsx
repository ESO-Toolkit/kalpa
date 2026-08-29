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
import { collectSupportEnvironment } from "@/lib/support-environment";
import {
  buildSupportHandoffUrl,
  buildSupportTicketPayload,
  renderSupportReport,
  SUPPORT_ISSUES,
  UNKNOWN_SUPPORT_ENVIRONMENT,
  type SupportEnvironment,
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
  const [handoffOpened, setHandoffOpened] = useState(false);
  const [appVersion, setAppVersion] = useState("unknown");
  const [environment, setEnvironment] = useState<SupportEnvironment>(UNKNOWN_SUPPORT_ENVIRONMENT);
  const [generatedAt] = useState(() => new Date());
  const [copiedReport, setCopiedReport] = useState<string | null>(null);

  useEffect(() => {
    void getVersion()
      .then(setAppVersion)
      .catch(() => setAppVersion("unknown"));
    // Every field degrades to "unknown" on its own, so a rejection here can
    // only mean the whole collection failed; keep the all-unknown default.
    void collectSupportEnvironment()
      .then(setEnvironment)
      .catch(() => setEnvironment(UNKNOWN_SUPPORT_ENVIRONMENT));
  }, []);

  const payload = useMemo(
    () =>
      buildSupportTicketPayload({
        issueId,
        description,
        appVersion,
        platform: osType(),
        environment,
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
      environment,
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
  // A disabled primary action has to say what would enable it.
  const blockedReason = !consented
    ? "Tick the consent box above to enable this."
    : isOffline
      ? "Kalpa is offline. Reconnect, or copy the report and use the manual ticket desk."
      : !handoffUrl
        ? "This report is too large for the secure handoff. Copy it and use the manual ticket desk."
        : null;
  const selectedIssue = SUPPORT_ISSUES.find((issue) => issue.id === issueId)!;

  async function copyReport(): Promise<boolean> {
    try {
      await navigator.clipboard.writeText(report);
      // The status panel below announces this; a toast as well would make a
      // screen reader read the same result twice.
      setCopiedReport(report);
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
      const opened = await openFeedbackUrl(handoffUrl, { toastOnError: false });
      if (opened) {
        // The stage line owns this announcement for the same reason.
        setHandoffOpened(true);
      } else {
        toast.error(
          "Kalpa couldn't open the secure handoff. Your report is still available below."
        );
      }
    } finally {
      setOpeningHandoff(false);
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      {/* Short viewports are the norm here, not an edge case: Kalpa's minimum
          window is 800x500, and at 200% zoom that is a 400x250 CSS viewport.
          The header and footer are compacted so the review region keeps a
          usable height, and the popup itself scrolls as a last resort so a
          footer action can never be clipped out of reach. */}
      <DialogContent className="flex max-h-[calc(100dvh-1rem)] flex-col gap-0 overflow-y-auto p-0 sm:max-w-2xl">
        <DialogHeader className="mx-0 mt-0 shrink-0 px-5 pt-5 pb-4 max-h-short:pt-3 max-h-short:pb-2">
          <DialogTitle className="flex items-center gap-2">
            <LifeBuoy className="size-4 text-primary" />
            Help me with Kalpa
          </DialogTitle>
          <DialogDescription>
            Tell us what went wrong. Kalpa will prepare the useful details so you do not have to
            know what logs to find.
          </DialogDescription>
        </DialogHeader>

        {/* Below ~420px of viewport height the inner scroller would collapse to
            a few pixels, so the whole dialog becomes the single scroll
            container instead of nesting a useless one inside it — it must also
            stop shrinking there, or its content spills out of a collapsed box
            and paints over the footer. Above that the sticky footer is worth
            keeping: the primary action stays in view. */}
        <div className="min-h-0 space-y-4 overflow-y-auto px-5 py-4 max-h-short:space-y-3 max-h-short:py-2 max-h-tiny:shrink-0 max-h-tiny:overflow-visible">
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
                    <span className="flex items-start gap-2 font-heading text-xs font-semibold text-foreground">
                      {selected && (
                        <CheckCircle2 className="mt-px size-3.5 shrink-0 text-accent-sky" />
                      )}
                      <span className="min-w-0">{issue.label}</span>
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
                className="max-h-56 overflow-y-auto overflow-x-hidden border-t border-structure-06 bg-scrim-10 px-3 py-2.5 font-mono text-xs leading-relaxed whitespace-pre-wrap wrap-anywhere text-muted-foreground select-text"
              >
                {report}
              </pre>
            </details>
          </section>

          <div className="space-y-2">
            {/* One polite status owns the prepared/continuing distinction. The
                panels below carry their own alert/status roles, so nesting them
                inside another live region would announce everything twice. */}
            <p
              id="support-stage"
              role="status"
              className="text-[11px] leading-relaxed text-muted-foreground"
            >
              {handoffOpened
                ? "Continuing in your browser. No ticket exists until ESO Toolkit confirms it there."
                : "Your report is prepared. No ticket has been created yet."}
            </p>
            {!handoffUrl && (
              <GlassPanel
                variant="subtle"
                className="border-status-warning/20 bg-status-warning/[0.04] p-3"
                role="alert"
              >
                <p className="text-xs font-semibold text-foreground">
                  The secure browser handoff could not be prepared.
                </p>
                <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
                  Nothing has been sent. Copy the report and use the manual ticket desk instead.
                </p>
              </GlassPanel>
            )}
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

        <DialogFooter className="mx-0 mb-0 shrink-0 flex-col gap-3 px-5 py-4 max-h-short:gap-2 max-h-short:py-2 sm:flex-row sm:items-end sm:justify-between">
          <div className="flex min-w-0 flex-wrap gap-1">
            <Button
              variant="ghost"
              size="sm"
              className="h-auto min-h-8 py-1.5 whitespace-normal"
              onClick={() => void copyReport()}
            >
              <ClipboardCheck />
              Copy report
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-auto min-h-8 py-1.5 whitespace-normal"
              onClick={() => void openFeedbackUrl(FEEDBACK_DISCORD_SUPPORT_URL)}
            >
              Manual ticket desk
              <ExternalLink />
            </Button>
          </div>
          <div className="flex min-w-0 flex-col items-stretch gap-1 sm:items-end">
            <Button
              aria-busy={openingHandoff}
              aria-describedby={blockedReason ? "support-create-blocked" : "support-stage"}
              // Busy is advertised, not enforced with `disabled`: Chromium blurs
              // a focused element the moment it is disabled, so a keyboard user
              // who activated this would lose their place. createPrivateTicket
              // already refuses a second run while one is in flight.
              aria-disabled={openingHandoff}
              className="h-auto min-h-9 py-2 whitespace-normal aria-disabled:pointer-events-none aria-disabled:opacity-50"
              disabled={blockedReason !== null}
              onClick={() => void createPrivateTicket()}
            >
              {openingHandoff ? <LoaderCircle className="animate-spin" /> : <LockKeyhole />}
              {openingHandoff
                ? "Opening secure sign-in…"
                : handoffOpened
                  ? "Reopen secure sign-in"
                  : "Create private ticket"}
              {!openingHandoff && <ExternalLink />}
            </Button>
            {blockedReason && (
              <p
                id="support-create-blocked"
                className="text-[11px] leading-relaxed text-muted-foreground sm:text-right"
              >
                {blockedReason}
              </p>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
