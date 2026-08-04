import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { AlertTriangle, ChevronDown, ChevronRight, Scissors, Upload, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { Input } from "@/components/ui/input";
import { ProgressBar } from "@/components/ui/progress-bar";
import { SectionHeader } from "@/components/ui/section-header";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import type {
  FightSelection,
  FightSummary,
  LogPreflight,
  LogSession,
  SplitSelection,
  UploadDispatch,
  UploadProgressEvent,
  Visibility,
} from "@/types/uploader";
import { dominantZone, suggestFightName, suggestSplitName } from "./naming";
import {
  compactBytes,
  fightLabel,
  formatDuration,
  primaryReportUrlForOpen,
} from "./uploader-shared";

type CheckState = "none" | "some" | "all";
type CommitMode = "save" | "upload";
type WorkPhase = "idle" | "preparing" | "uploading" | "partial";
type WholeSelection = Set<number>;
type FightSelectionMap = Map<number, Set<number>>;

interface PlannedReport {
  id: string;
  session: LogSession;
  fights: FightSummary[];
  kind: "whole" | "single-fight" | "subset";
  fightCount: number;
  sizeBytes: number;
  suggestedName: string;
  zone: string | null;
  label: string;
}

export interface SplitUploadRequest {
  filePath: string;
  fightCount: number;
  zone: string | null;
  onProgress: (event: UploadProgressEvent) => void;
}

export function SlicePicker({
  filePath,
  fileName,
  preflight,
  isSignedIn,
  visibility,
  onSignIn,
  onRescan,
  onFilesWritten,
  onUploadPreparedLog,
  onBusyChange,
}: {
  filePath: string;
  fileName: string;
  preflight: LogPreflight;
  isSignedIn: boolean;
  visibility: Visibility;
  onSignIn: () => Promise<void> | void;
  onRescan: () => Promise<void> | void;
  onFilesWritten: () => Promise<void> | void;
  onUploadPreparedLog: (request: SplitUploadRequest) => Promise<UploadDispatch>;
  /** Raised while this picker is cutting or uploading. The workspace uses it to
   *  keep its own whole-log Upload disabled and to refuse a log switch, either of
   *  which would otherwise run concurrently with — or silently orphan — a batch
   *  that can take minutes on a multi-GB log. */
  onBusyChange?: (busy: boolean) => void;
}) {
  const sessions = preflight.sessions;
  const fights = preflight.fights;
  const omitted = preflight.fightsOmitted;
  const fightsBySession = useMemo(() => mapFightsBySession(sessions, fights), [sessions, fights]);
  const defaultState = useMemo(
    () => buildDefaultState(sessions, fightsBySession, omitted),
    [sessions, fightsBySession, omitted]
  );

  const [selectedFights, setSelectedFights] = useState<FightSelectionMap>(defaultState.fights);
  const [wholeSessions, setWholeSessions] = useState<WholeSelection>(defaultState.whole);
  const [expanded, setExpanded] = useState<Set<number>>(defaultState.expanded);
  const [renameOpen, setRenameOpen] = useState(false);
  const [nameDrafts, setNameDrafts] = useState<Record<string, string>>({});
  const [phase, setPhase] = useState<WorkPhase>("idle");
  const [progressValue, setProgressValue] = useState(0);
  const [progressMax, setProgressMax] = useState(1);
  const [progressText, setProgressText] = useState("");
  const [partialMessage, setPartialMessage] = useState<string | null>(null);
  const [partialFiles, setPartialFiles] = useState<{ path: string; plan: PlannedReport }[]>([]);
  const [partialMode, setPartialMode] = useState<CommitMode>("upload");
  /** Reports this run already published before it stopped. Non-zero means a full
   *  re-run would publish them a second time, so the "Try again" escape hatch is
   *  withdrawn (see the recovery buttons below). */
  const [partialPublished, setPartialPublished] = useState(0);
  const [stale, setStale] = useState(false);
  const stopRequestedRef = useRef(false);
  const stopButtonRef = useRef<HTMLButtonElement | null>(null);
  const commitButtonRef = useRef<HTMLButtonElement | null>(null);
  const partialActionsRef = useRef<HTMLDivElement | null>(null);
  const prevPhaseRef = useRef<WorkPhase>("idle");

  // Every phase change unmounts whatever had focus (Stop, or the recovery
  // buttons), which would otherwise drop the user to <body> at the exact moment
  // a decision appears. Move focus to the control that replaced it.
  useEffect(() => {
    const previous = prevPhaseRef.current;
    prevPhaseRef.current = phase;
    if (phase === "preparing" || phase === "uploading") {
      stopButtonRef.current?.focus();
      return;
    }
    if (phase === "partial") {
      partialActionsRef.current?.querySelector("button")?.focus();
      return;
    }
    if (previous !== "idle") commitButtonRef.current?.focus();
  }, [phase]);

  const busy = phase === "preparing" || phase === "uploading";
  useEffect(() => {
    onBusyChange?.(busy);
    // Unmounting mid-batch must not strand the workspace in a busy state.
    return () => onBusyChange?.(false);
  }, [busy, onBusyChange]);

  const noFightsAtAll = preflight.totalFights === 0 && !omitted;
  const plans = useMemo(
    () => buildPlan(sessions, fightsBySession, selectedFights, wholeSessions),
    [sessions, fightsBySession, selectedFights, wholeSessions]
  );
  const selectedFightCount = plans.reduce((sum, p) => sum + p.fightCount, 0);
  const selectedSize = plans.reduce((sum, p) => sum + p.sizeBytes, 0);
  const controlsDisabled = phase === "preparing" || phase === "uploading";
  const allOmitted = omitted && sessions.length > 0 && fights.length === 0;
  const bossFightsDisabled = allOmitted;
  // Two distinct questions, so two chips rather than one that guesses.
  //
  // A kill is positive evidence: a boss unit declared by UNIT_ADDED later died.
  // Validated against five real logs — a single-boss Asylum run reports exactly
  // one kill on "Saint Olms the Just", a Lucent Citadel clear reports three.
  //
  // The ABSENCE of a kill proves nothing. An abandoned run emits BEGIN_TRIAL with
  // no END_TRIAL at all, so a wipe and an unclassified pull look identical. That
  // is why "boss fights" cannot be derived by excluding wipes, and why an empty
  // kill set disables its chip instead of quietly selecting something else.
  const killCount = fights.filter((f) => f.outcome === "kill").length;
  const bossFightCount = fights.filter((f) => f.bossName).length;
  const bossKillsDisabled = allOmitted || killCount === 0;

  const resetToDefault = () => {
    setSelectedFights(cloneFightMap(defaultState.fights));
    setWholeSessions(new Set(defaultState.whole));
    setExpanded(new Set(defaultState.expanded));
    setNameDrafts({});
    setRenameOpen(false);
  };

  const setSessionChecked = (session: LogSession, checked: boolean) => {
    const inFights = fightsBySession.get(session.index) ?? [];
    if (omitted || inFights.length === 0) {
      setWholeSessions((prev) => toggleSet(prev, session.index, checked));
      return;
    }
    setSelectedFights((prev) => {
      const next = cloneFightMap(prev);
      if (checked) next.set(session.index, new Set(inFights.map((f) => f.index)));
      else next.delete(session.index);
      return next;
    });
  };

  const setFightChecked = (session: LogSession, fight: FightSummary, checked: boolean) => {
    setSelectedFights((prev) => {
      const next = cloneFightMap(prev);
      const set = new Set(next.get(session.index) ?? []);
      if (checked) set.add(fight.index);
      else set.delete(fight.index);
      if (set.size > 0) next.set(session.index, set);
      else next.delete(session.index);
      return next;
    });
  };
  const selectMatchingFights = (predicate: (fight: FightSummary) => boolean) => {
    const nextFights: FightSelectionMap = new Map();
    for (const session of sessions) {
      const selected = (fightsBySession.get(session.index) ?? []).filter(predicate);
      if (selected.length > 0) nextFights.set(session.index, new Set(selected.map((f) => f.index)));
    }
    setWholeSessions(new Set());
    setSelectedFights(nextFights);
    setExpanded(new Set(Array.from(nextFights.keys())));
  };

  /** Only fights we positively know ended in a kill — the clean run, without the
   *  resets before it. Disabled outright when the log has none, rather than
   *  falling back to a wider set the label does not promise. */
  const applyBossKills = () => {
    if (bossKillsDisabled) return;
    selectMatchingFights((f) => f.outcome === "kill");
  };

  /** Every pull at a named boss, kills and resets alike. */
  const applyBossFights = () => {
    if (bossFightsDisabled) return;
    selectMatchingFights((f) => Boolean(f.bossName));
  };

  const applyWholeLog = () => {
    const nextFights: FightSelectionMap = new Map();
    const nextWhole = new Set<number>();
    const nextExpanded = new Set<number>();
    for (const session of sessions) {
      const inFights = fightsBySession.get(session.index) ?? [];
      if (omitted || inFights.length === 0) nextWhole.add(session.index);
      else nextFights.set(session.index, new Set(inFights.map((f) => f.index)));
      nextExpanded.add(session.index);
    }
    setSelectedFights(nextFights);
    setWholeSessions(nextWhole);
    setExpanded(nextExpanded);
  };

  const clearAll = () => {
    setSelectedFights(new Map());
    setWholeSessions(new Set());
    setExpanded(new Set());
  };

  const nameForPlan = (plan: PlannedReport): string | null => {
    const draft = nameDrafts[plan.id];
    if (draft === undefined) return plan.suggestedName;
    return draft.trim() || null;
  };

  /** Enter the recovery state.
   *
   *  `files` is what is still OUTSTANDING, never what already succeeded. `mode`
   *  is the commit the user actually asked for: a run they started with Save
   *  must not be offered an upload as its recovery, because publishing a combat
   *  log is a decision they explicitly declined. `published` counts the reports
   *  this run already sent to ESO Logs (zero while only cutting files).  */
  const showPartial = (
    done: number,
    total: number,
    reason: string,
    files: { path: string; plan: PlannedReport }[],
    mode: CommitMode,
    published: number
  ) => {
    setPhase("partial");
    setPartialFiles(files);
    setPartialMode(mode);
    setPartialPublished(published);
    setPartialMessage(
      mode === "save"
        ? `Kalpa finished ${done} of ${total} file${total === 1 ? "" : "s"} before ${reason}. The finished files are safe in Kalpa Splits.`
        : `Kalpa finished ${done} of ${total} file${total === 1 ? "" : "s"} before ${reason}. The finished files are safe and ready to upload.`
    );
  };

  const uploadPartialFiles = async (files: { path: string; plan: PlannedReport }[]) => {
    if (files.length === 0) return;
    if (!isSignedIn) {
      await onSignIn();
      return;
    }
    stopRequestedRef.current = false;
    setPhase("uploading");
    setProgressMax(files.length);
    setProgressValue(0);
    let uploaded = 0;
    try {
      for (let i = 0; i < files.length; i++) {
        if (stopRequestedRef.current) break;
        const item = files[i]!;
        const uploadName = basename(item.path);
        setProgressText(`Uploading report ${i + 1} of ${files.length} - ${uploadName}`);
        const dispatch = await onUploadPreparedLog({
          filePath: item.path,
          fightCount: item.plan.fightCount,
          zone: item.plan.zone,
          onProgress: () => {
            setProgressText(`Uploading report ${i + 1} of ${files.length} - ${uploadName}`);
          },
        });
        uploaded += 1;
        setProgressValue(uploaded);
        if (dispatch.report) {
          toast.success(`Report ready - ${uploadName}`, {
            duration: 9000,
            action: {
              label: "Open report",
              onClick: () => void openReport(dispatch.report!, visibility),
            },
            cancel: { label: "Show in folder", onClick: () => void showInFolder(item.path) },
          });
        } else {
          toast.success(dispatch.detail || `Report handed off - ${uploadName}`, { duration: 9000 });
          // Same reason commit() parks here: an external uploader now owns the
          // exchange, so continuing would launch it once per remaining file and
          // claim success for uploads Kalpa never performed.
          if (uploaded < files.length) {
            showPartial(
              uploaded,
              files.length,
              "handing this file to the official uploader",
              outstandingAfter(files, uploaded),
              "upload",
              uploaded
            );
            return;
          }
          break;
        }
      }
      await onFilesWritten();
      // The recovery set is what is still OUTSTANDING, i.e. everything from the
      // first item that did not upload. Passing the successful prefix instead
      // would re-upload reports that already exist — esologs.com assigns a report
      // code per request with no client idempotency key, so a retry would publish
      // duplicates — while never retrying the item that actually failed.
      if (uploaded < files.length)
        showPartial(
          uploaded,
          files.length,
          "stopping",
          outstandingAfter(files, uploaded),
          "upload",
          uploaded
        );
      else setPhase("idle");
    } catch (error) {
      const message = getTauriErrorMessage(error);
      const outstanding = outstandingAfter(files, uploaded);
      if (outstanding.length > 0) {
        showPartial(uploaded, files.length, message, outstanding, "upload", uploaded);
      } else {
        // Nothing is left to retry, and re-sending what already published would
        // duplicate every report — report the failure instead of offering a rerun.
        setPhase("idle");
        toast.error(`Every remaining file was sent, but the run ended early. ${message}`);
      }
    }
  };
  const commit = async (mode: CommitMode) => {
    if (mode === "upload" && !isSignedIn) {
      await onSignIn();
      return;
    }
    if (plans.length === 0 || noFightsAtAll) return;
    stopRequestedRef.current = false;
    setPartialMessage(null);
    setStale(false);
    const writtenFiles: { path: string; plan: PlannedReport }[] = [];
    // Hoisted so the catch below can tell which files are still outstanding.
    // Scoped inside the try, a mid-upload throw could only offer every written
    // file again, re-publishing the reports that had already succeeded.
    let uploaded = 0;

    try {
      setPhase("preparing");
      const totalBytes = Math.max(
        1,
        plans.reduce((sum, p) => sum + p.sizeBytes, 0)
      );
      let copiedBytes = 0;
      setProgressMax(totalBytes);
      setProgressValue(0);
      for (let i = 0; i < plans.length; i++) {
        if (stopRequestedRef.current) break;
        const plan = plans[i]!;
        setProgressText(
          `Preparing report ${i + 1} of ${plans.length} - 0 of ${compactBytes(plan.sizeBytes)}`
        );
        const paths = await writePlan(filePath, preflight, plan, nameForPlan(plan));
        copiedBytes += plan.sizeBytes;
        setProgressValue(Math.min(totalBytes, copiedBytes));
        setProgressText(
          `Preparing report ${i + 1} of ${plans.length} - ${compactBytes(plan.sizeBytes)} of ${compactBytes(plan.sizeBytes)}`
        );
        if (paths[0]) writtenFiles.push({ path: paths[0], plan });
      }
      await onFilesWritten();

      if (stopRequestedRef.current || writtenFiles.length < plans.length) {
        // Nothing has uploaded yet at this point, so every written file is
        // outstanding. `mode` decides whether recovery offers an upload at all.
        showPartial(writtenFiles.length, plans.length, "stopping", writtenFiles, mode, 0);
        return;
      }

      if (mode === "save") {
        setPhase("idle");
        toast.success("Saved. Your files are in Kalpa Splits whenever you're ready.", {
          duration: 7000,
          action: writtenFiles[0]
            ? { label: "Show in folder", onClick: () => void showInFolder(writtenFiles[0]!.path) }
            : undefined,
        });
        return;
      }

      setPhase("uploading");
      setProgressMax(writtenFiles.length);
      setProgressValue(0);
      for (let i = 0; i < writtenFiles.length; i++) {
        if (stopRequestedRef.current) break;
        const item = writtenFiles[i]!;
        const uploadName = basename(item.path);
        setProgressText(`Uploading report ${i + 1} of ${writtenFiles.length} - ${uploadName}`);
        const dispatch = await onUploadPreparedLog({
          filePath: item.path,
          fightCount: item.plan.fightCount,
          zone: item.plan.zone,
          onProgress: () => {
            setProgressText(`Uploading report ${i + 1} of ${writtenFiles.length} - ${uploadName}`);
          },
        });
        uploaded += 1;
        setProgressValue(uploaded);
        if (dispatch.report) {
          toast.success(`Report ready - ${uploadName}`, {
            duration: 9000,
            action: {
              label: "Open report",
              onClick: () => void openReport(dispatch.report!, visibility),
            },
            cancel: { label: "Show in folder", onClick: () => void showInFolder(item.path) },
          });
        } else {
          toast.success(dispatch.detail || `Report handed off - ${uploadName}`, {
            duration: 9000,
            action: { label: "Show in folder", onClick: () => void showInFolder(item.path) },
          });
          // No report code means Kalpa handed this file to the official uploader,
          // an external app that now owns the rest of the exchange. Continuing the
          // loop would launch it once per remaining slice, stacking windows and
          // claiming success for uploads Kalpa never performed. Park the rest and
          // let the user send them once this one is done.
          if (uploaded < writtenFiles.length) {
            showPartial(
              uploaded,
              writtenFiles.length,
              "handing the first file to the official uploader",
              outstandingAfter(writtenFiles, uploaded),
              mode,
              uploaded
            );
            return;
          }
          break;
        }
      }
      await onFilesWritten();
      if (uploaded < writtenFiles.length) {
        showPartial(
          uploaded,
          writtenFiles.length,
          "stopping",
          outstandingAfter(writtenFiles, uploaded),
          mode,
          uploaded
        );
        return;
      }
      setPhase("idle");
      resetToDefault();
    } catch (error) {
      const message = getTauriErrorMessage(error);
      if (/log changed since it was scanned|log changed while/i.test(message)) setStale(true);
      // Outstanding = written but not yet uploaded. A throw partway through the
      // upload loop must not re-offer the slices that already published.
      if (writtenFiles.length > uploaded)
        showPartial(
          uploaded,
          plans.length,
          message,
          outstandingAfter(writtenFiles, uploaded),
          mode,
          uploaded
        );
      else if (writtenFiles.length > 0) {
        setPhase("idle");
        toast.error(`Every prepared file was sent, but the run ended early. ${message}`);
      } else {
        setPhase("idle");
        toast.error(`Kalpa couldn't finish cutting the file. Nothing was uploaded. ${message}`);
      }
    }
  };
  return (
    <div className="space-y-3">
      <GlassPanel
        variant="default"
        className={cn(
          "border-l-[3px] p-3",
          preflight.recommendSplit ? "border-l-status-warning" : "border-l-accent-sky"
        )}
      >
        <div className="space-y-3">
          <div className="min-w-0 space-y-1">
            <SectionHeader className="flex items-center gap-2 text-foreground">
              <Scissors className="size-3.5" aria-hidden /> Upload part of this log
            </SectionHeader>
            <p
              className={cn(
                "text-xs",
                preflight.recommendSplit ? "text-status-warning" : "text-muted-foreground"
              )}
            >
              {preflight.recommendSplit
                ? "This log is too big for one report - pick part of it. "
                : ""}
              {fileName} - {compactBytes(preflight.sizeBytes)} - {sessions.length} night
              {sessions.length === 1 ? "" : "s"} - {preflight.totalFights} fight
              {preflight.totalFights === 1 ? "" : "s"}
            </p>
            {stale && (
              <Alert className="mt-2 border-status-warning/30 bg-status-warning/[0.06]">
                <AlertTriangle className="size-4 text-status-warning" aria-hidden />
                <AlertTitle>This log changed while you were choosing</AlertTitle>
                <AlertDescription>
                  The game probably wrote new combat. Rescan before uploading this slice.
                </AlertDescription>
                <Button
                  variant="outline"
                  size="sm"
                  className="mt-2"
                  onClick={() => void onRescan()}
                >
                  Rescan
                </Button>
              </Alert>
            )}
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={resetToDefault}
              disabled={controlsDisabled}
            >
              Tonight's raid
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={applyBossKills}
              disabled={controlsDisabled || bossKillsDisabled}
              aria-label={
                killCount > 0
                  ? `Boss kills only, ${killCount} in this log`
                  : "Boss kills only, none recorded in this log"
              }
            >
              Boss kills{killCount > 0 ? ` (${killCount})` : ""}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={applyBossFights}
              disabled={controlsDisabled || bossFightsDisabled}
              aria-label={`Every pull at a named boss, kills and resets alike, ${bossFightCount} in this log`}
            >
              Boss fights{bossFightCount > 0 ? ` (${bossFightCount})` : ""}
            </Button>
            <Button variant="outline" size="sm" onClick={applyWholeLog} disabled={controlsDisabled}>
              Whole log
            </Button>
            <Button variant="outline" size="sm" onClick={clearAll} disabled={controlsDisabled}>
              Clear
            </Button>
            {bossFightsDisabled ? (
              <span className="text-xs text-muted-foreground">
                Boss filters are unavailable because fights were not listed.
              </span>
            ) : (
              killCount === 0 &&
              bossFightCount > 0 && (
                <span className="text-xs text-muted-foreground">
                  No kills were recorded in this log. A run that ends without finishing writes
                  nothing to say so, so use Boss fights and pick from there.
                </span>
              )
            )}
          </div>

          <div className="border-t border-structure-06 pt-2" aria-busy={controlsDisabled}>
            {noFightsAtAll ? (
              <Alert>
                <AlertTriangle className="size-4" aria-hidden />
                <AlertTitle>No fights in this file yet</AlertTitle>
                <AlertDescription>
                  In game, type /encounterlog to start recording combat, then check back.
                </AlertDescription>
              </Alert>
            ) : sessions.length === 0 ? (
              <Alert>
                <AlertTriangle className="size-4" aria-hidden />
                <AlertTitle>No logging sessions found</AlertTitle>
                <AlertDescription>
                  This file does not contain a session Kalpa can split.
                </AlertDescription>
              </Alert>
            ) : (
              <div className="space-y-0">
                {sessions.map((session) => {
                  const inFights = fightsBySession.get(session.index) ?? [];
                  return (
                    <SessionGroup
                      key={session.index}
                      session={session}
                      fights={inFights}
                      omitted={omitted}
                      expanded={expanded.has(session.index)}
                      disabled={controlsDisabled}
                      wholeSelected={wholeSessions.has(session.index)}
                      selected={selectedFights.get(session.index) ?? new Set()}
                      onToggleExpanded={() =>
                        setExpanded((prev) =>
                          toggleSet(prev, session.index, !prev.has(session.index))
                        )
                      }
                      onSessionChecked={(checked) => setSessionChecked(session, checked)}
                      onFightChecked={(fight, checked) => setFightChecked(session, fight, checked)}
                    />
                  );
                })}
              </div>
            )}
          </div>
        </div>
      </GlassPanel>

      <GlassPanel
        variant="primary"
        className="sticky bottom-0 z-10 p-3 shadow-[0_-10px_30px_-20px_var(--scrim-80)]"
      >
        <div className="space-y-2">
          <div className="space-y-1" aria-live="polite">
            {phase === "preparing" || phase === "uploading" ? (
              <>
                <p className="text-sm text-foreground">{progressText}</p>
                {/* The cut advances only when a whole plan finishes, so a single
                    multi-GB plan would hold a determinate bar at 0% for minutes
                    and read as a hang. */}
                <ProgressBar
                  value={progressValue}
                  max={progressMax}
                  label={progressText}
                  indeterminate={phase === "preparing"}
                />
                {phase === "preparing" && (
                  <p className="text-xs text-muted-foreground">
                    Big files take a minute - Kalpa is copying only the parts you picked.
                  </p>
                )}
              </>
            ) : phase === "partial" && partialMessage ? (
              <p className="text-sm text-foreground">{partialMessage}</p>
            ) : plans.length === 0 || noFightsAtAll ? (
              <p className="text-sm text-foreground">
                Nothing picked - tick a fight or a night above.
              </p>
            ) : (
              <p className="text-sm text-foreground">
                {plans.length} report{plans.length === 1 ? "" : "s"} - {selectedFightCount} fight
                {selectedFightCount === 1 ? "" : "s"} - ~{compactBytes(selectedSize)}
              </p>
            )}
            <p className="text-xs text-muted-foreground">
              Fights you pick from the same night go into one report.
            </p>
          </div>

          {plans.length > 0 && phase === "idle" && (
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <span
                className="min-w-0 flex-1 truncate"
                title={plans.map((p) => `${nameForPlan(p) ?? p.suggestedName}.log`).join(" - ")}
              >
                {plans.map((p) => `${nameForPlan(p) ?? p.suggestedName}.log`).join(" - ")}
              </span>
              <Button
                variant="ghost"
                size="sm"
                aria-expanded={renameOpen}
                onClick={() => setRenameOpen((open) => !open)}
              >
                Rename...
              </Button>
            </div>
          )}

          {renameOpen && phase === "idle" && plans.length > 0 && (
            <div className="grid gap-2 rounded-lg border border-structure-06 bg-structure-02 p-2 sm:grid-cols-2">
              {plans.map((plan) => (
                <Input
                  key={plan.id}
                  value={nameDrafts[plan.id] ?? plan.suggestedName}
                  onChange={(event) =>
                    setNameDrafts((prev) => ({ ...prev, [plan.id]: event.target.value }))
                  }
                  aria-label={`File name for ${plan.label}`}
                  className="h-8 text-xs"
                />
              ))}
            </div>
          )}

          <div className="flex flex-wrap items-center justify-between gap-2">
            {phase === "preparing" || phase === "uploading" ? (
              <Button
                ref={stopButtonRef}
                variant="ghost"
                size="sm"
                onClick={() => (stopRequestedRef.current = true)}
              >
                <X className="size-3.5" /> Stop
              </Button>
            ) : phase === "partial" ? (
              <div className="flex flex-wrap gap-2" ref={partialActionsRef}>
                {/* A full re-run re-cuts AND re-uploads every plan, so it is only
                    safe while nothing has published — esologs.com issues a report
                    code per request with no client idempotency key, so retrying a
                    slice that already succeeded publishes a duplicate. Past that
                    point the outstanding-only button below is the whole recovery. */}
                {partialPublished === 0 && (
                  <Button variant="outline" size="sm" onClick={() => void commit(partialMode)}>
                    Try again
                  </Button>
                )}
                {/* A run started with Save is never recovered by uploading:
                    publishing the log is a decision this user declined. */}
                {partialMode === "upload" ? (
                  <Button
                    size="sm"
                    onClick={() => void uploadPartialFiles(partialFiles)}
                    disabled={partialFiles.length === 0}
                  >
                    Upload the {partialFiles.length} remaining file
                    {partialFiles.length === 1 ? "" : "s"}
                  </Button>
                ) : (
                  partialFiles[0] && (
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => void showInFolder(partialFiles[0]!.path)}
                    >
                      Show in folder
                    </Button>
                  )
                )}
              </div>
            ) : (
              <>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={plans.length === 0 || noFightsAtAll}
                  onClick={() => void commit("save")}
                >
                  Save without uploading
                </Button>
                <Button
                  ref={commitButtonRef}
                  disabled={plans.length === 0 || noFightsAtAll}
                  onClick={() => void commit("upload")}
                >
                  {isSignedIn ? <Upload className="size-3.5" /> : null}
                  {isSignedIn
                    ? `Upload ${plans.length} report${plans.length === 1 ? "" : "s"}`
                    : "Sign in to upload"}
                </Button>
              </>
            )}
          </div>
        </div>
      </GlassPanel>
    </div>
  );
}
function SessionGroup({
  session,
  fights,
  omitted,
  expanded,
  disabled,
  wholeSelected,
  selected,
  onToggleExpanded,
  onSessionChecked,
  onFightChecked,
}: {
  session: LogSession;
  fights: FightSummary[];
  omitted: boolean;
  expanded: boolean;
  disabled: boolean;
  wholeSelected: boolean;
  selected: Set<number>;
  onToggleExpanded: () => void;
  onSessionChecked: (checked: boolean) => void;
  onFightChecked: (fight: FightSummary, checked: boolean) => void;
}) {
  const state = sessionCheckState(fights, omitted, wholeSelected, selected);
  const headerId = `slice-session-${session.index}`;
  const groupId = `slice-session-fights-${session.index}`;
  const realm = session.realm?.replace(/megaserver/i, "").trim() || null;
  const killCount = fights.filter((f) => f.outcome === "kill").length;
  const bossFights = fights.filter((f) => f.bossName);
  const allVeteran = bossFights.length > 0 && bossFights.every((f) => f.difficulty === "veteran");
  const dominant = dominantZone(fights);
  const hasChildren = !omitted && fights.length > 0;
  const noFights = !omitted && fights.length === 0;
  const attempts = attemptOrdinals(fights);

  return (
    <div className="border-t border-structure-06 first:border-t-0">
      <div className="flex min-h-11 items-start gap-2 py-2">
        <Checkbox
          checked={state === "all"}
          indeterminate={state === "some"}
          disabled={disabled}
          aria-label={`Include ${sessionLabel(session, fights)}`}
          onCheckedChange={() => onSessionChecked(state !== "all")}
          className="mt-1"
        />
        {hasChildren ? (
          <Button
            variant="ghost"
            size="icon-sm"
            disabled={disabled}
            aria-expanded={expanded}
            aria-controls={groupId}
            aria-label={`${expanded ? "Collapse" : "Expand"} ${sessionLabel(session, fights)}`}
            onClick={onToggleExpanded}
          >
            {expanded ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
          </Button>
        ) : (
          <span className="size-7 shrink-0" aria-hidden />
        )}
        <button
          type="button"
          disabled={!hasChildren || disabled}
          onClick={onToggleExpanded}
          className="min-w-0 flex-1 text-left disabled:cursor-default"
        >
          <div className="flex flex-wrap items-center gap-1.5">
            <span
              id={headerId}
              className="font-heading text-[11px] font-bold tracking-[0.05em] text-foreground uppercase"
            >
              {sessionLabel(session, fights)}
            </span>
            {dominant ? <span className="truncate text-xs text-foreground">{dominant}</span> : null}
            {noFights ? (
              <span className="text-xs text-muted-foreground">No fights recorded</span>
            ) : null}
            {allVeteran ? (
              <InfoPill color="gold" aria-label="Veteran">
                VET
              </InfoPill>
            ) : null}
            {realm ? <InfoPill color="muted">{realm}</InfoPill> : null}
          </div>
          <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
            <span>
              {session.fightCount} fight{session.fightCount === 1 ? "" : "s"}
            </span>
            <span>-</span>
            <span>
              <span className="text-foreground">
                {killCount} kill{killCount === 1 ? "" : "s"}
              </span>
            </span>
            <span>-</span>
            <span>{compactBytes(session.sizeBytes)}</span>
          </div>
          {omitted && (
            <div className="mt-1 text-xs text-muted-foreground">
              Too big to list every fight - uploads as one whole night.
            </div>
          )}
        </button>
      </div>
      {hasChildren && expanded && (
        <div id={groupId} role="group" aria-labelledby={headerId} className="pb-1">
          {fights.map((fight) => (
            <FightRow
              key={fight.index}
              fight={fight}
              selected={selected.has(fight.index)}
              disabled={disabled}
              ordinal={attempts.get(fight.index)}
              sessionStartMs={session.startTimeMs}
              onCheckedChange={(checked) => onFightChecked(fight, checked)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function FightRow({
  fight,
  selected,
  disabled,
  ordinal,
  sessionStartMs,
  onCheckedChange,
}: {
  fight: FightSummary;
  selected: boolean;
  disabled: boolean;
  ordinal?: { position: number; total: number };
  /** Wall-clock start of the enclosing session. `fight.startMs` is an offset from
   *  it, not an epoch, so the clock is only meaningful once the two are added. */
  sessionStartMs: number;
  onCheckedChange: (checked: boolean) => void;
}) {
  const outcome = fight.outcome ?? null;
  const isKill = outcome === "kill";
  const isWipe = outcome === "wipe";
  const isTrash = !fight.bossName;
  const durationMs = Math.max(0, fight.endMs - fight.startMs);
  const name = fightName(fight, ordinal);

  return (
    <div
      className={cn(
        "flex items-center gap-2 border-l-[3px] px-2 transition-colors duration-150",
        isTrash ? "min-h-7 py-1" : "min-h-10 py-2",
        selected ? "bg-accent-sky/[0.04]" : isKill ? "bg-status-success/[0.04]" : "bg-transparent",
        isKill
          ? "border-l-status-success"
          : isWipe
            ? "border-l-status-danger"
            : "border-l-transparent"
      )}
    >
      <Checkbox
        checked={selected}
        disabled={disabled}
        aria-label={fightAriaLabel(fight, name)}
        onCheckedChange={(checked) => onCheckedChange(checked === true)}
      />
      <span
        className={cn(
          "min-w-0 flex-1 truncate text-foreground",
          isTrash ? "text-xs" : "text-sm font-medium"
        )}
        title={name}
      >
        {name}
      </span>
      <span className="flex shrink-0 items-center gap-1.5">
        {fight.difficulty === "veteran" && !isTrash ? (
          <InfoPill color="gold" aria-label="Veteran">
            VET
          </InfoPill>
        ) : null}
        {isKill ? <InfoPill color="emerald">Kill</InfoPill> : null}
        {isWipe ? <InfoPill color="red">Wipe</InfoPill> : null}
      </span>
      <span className="hidden w-16 shrink-0 text-right text-xs tabular-nums text-muted-foreground sm:inline">
        {formatClock(sessionStartMs + fight.startMs)}
      </span>
      <span className="hidden w-12 shrink-0 text-right text-xs tabular-nums text-muted-foreground sm:inline">
        {compactBytes(Math.max(0, fight.endOffset - fight.startOffset))}
      </span>
      <span
        className={cn(
          "w-12 shrink-0 text-right text-xs tabular-nums",
          isKill ? "font-medium text-foreground" : "text-muted-foreground"
        )}
      >
        {formatDuration(durationMs)}
      </span>
    </div>
  );
}
/** Exported for test — the promote-to-whole rules it feeds decide whether a run
 *  writes a few pulls or an entire night. */
export function buildDefaultState(
  sessions: LogSession[],
  fightsBySession: Map<number, FightSummary[]>,
  omitted: boolean
) {
  const cluster = nightCluster(sessions, fightsBySession);
  const selected: FightSelectionMap = new Map();
  const whole = new Set<number>();
  const expanded = new Set<number>();
  for (const session of sessions) {
    if (!cluster.has(session.index)) continue;
    expanded.add(session.index);
    const fights = fightsBySession.get(session.index) ?? [];
    if (omitted || fights.length === 0) whole.add(session.index);
    else selected.set(session.index, new Set(fights.map((f) => f.index)));
  }
  return { fights: selected, whole, expanded };
}

/** The most recent run of sessions that belong to one sitting.
 *
 *  A crash-and-relaunch mid-raid is minutes apart, so those sessions must stay
 *  together. The first rule chained transitively on start-to-start distance,
 *  which merged anything within six hours of anything already in the set:
 *  sessions at 00:00, 05:00, 10:00, 15:00 and 20:00 all linked, and "tonight's
 *  raid" selected a twenty-hour span. It also never asked how long a session
 *  actually ran, so a four-hour raid and a session starting five hours after it
 *  began — one hour after it ended — were treated the same as two unrelated
 *  logins.
 *
 *  So: walk back from the latest session over a CONTIGUOUS run, and measure the
 *  gap from when a session actually stopped to when the next one started. Stop
 *  at the first real break rather than hopping over it. */
export function nightCluster(
  sessions: LogSession[],
  fightsBySession: Map<number, FightSummary[]>
): Set<number> {
  if (sessions.length === 0) return new Set();

  const GAP_MS = 3 * 60 * 60 * 1000;
  const ordered = [...sessions].sort((a, b) => a.startTimeMs - b.startTimeMs);

  // `endMs` is relative to the session's own BEGIN_LOG. When the fight list was
  // omitted (very large log) we can only fall back to the start time, which just
  // makes the measured gap larger — it never merges sessions that should stay apart.
  const endOf = (session: LogSession) => {
    let last = 0;
    for (const fight of fightsBySession.get(session.index) ?? []) {
      if (fight.endMs > last) last = fight.endMs;
    }
    return session.startTimeMs + last;
  };

  const out = new Set<number>([ordered[ordered.length - 1]!.index]);
  for (let i = ordered.length - 2; i >= 0; i--) {
    if (ordered[i + 1]!.startTimeMs - endOf(ordered[i]!) > GAP_MS) break;
    out.add(ordered[i]!.index);
  }
  return out;
}

function mapFightsBySession(
  sessions: LogSession[],
  fights: FightSummary[]
): Map<number, FightSummary[]> {
  const map = new Map<number, FightSummary[]>();
  for (const session of sessions) map.set(session.index, []);
  for (const fight of fights) {
    const session = sessions.find(
      (s) => fight.startOffset >= s.startOffset && fight.startOffset < s.endOffset
    );
    if (session) map.get(session.index)?.push(fight);
  }
  return map;
}

/** Exported for test — see the promotion guard below. */
export function buildPlan(
  sessions: LogSession[],
  fightsBySession: Map<number, FightSummary[]>,
  selectedFights: FightSelectionMap,
  wholeSessions: WholeSelection
): PlannedReport[] {
  const plans: PlannedReport[] = [];
  for (const session of sessions) {
    const fights = fightsBySession.get(session.index) ?? [];
    const selectedSet = selectedFights.get(session.index) ?? new Set<number>();
    const selected = fights.filter((f) => selectedSet.has(f.index));
    // Promoting "every listed fight" to "the whole session" is only sound when
    // the list is COMPLETE. A very large log ships a truncated fight list
    // (`fightsOmitted`), so ticking everything on screen can be a small subset of
    // the session — and promoting it would silently write and upload the entire
    // night instead of the few pulls that were chosen. `fightCount` is the
    // session's true total and is never truncated.
    const listIsComplete = fights.length > 0 && fights.length === session.fightCount;
    if (wholeSessions.has(session.index) || (listIsComplete && selected.length === fights.length)) {
      plans.push(planFor(session, fights, "whole"));
    } else if (selected.length === 1) {
      plans.push(planFor(session, selected, "single-fight"));
    } else if (selected.length > 1) {
      plans.push(planFor(session, selected, "subset"));
    }
  }
  return plans;
}

function planFor(
  session: LogSession,
  fights: FightSummary[],
  kind: PlannedReport["kind"]
): PlannedReport {
  const firstFight = fights[0];
  const suggestedName =
    kind === "single-fight" && firstFight
      ? suggestFightName(firstFight, 1)
      : suggestSplitName(session, fights);
  const sizeBytes =
    kind === "whole"
      ? session.sizeBytes
      : fights.reduce((sum, f) => sum + Math.max(0, f.endOffset - f.startOffset), 0);
  return {
    id: `${session.index}:${kind}:${fights.map((f) => f.index).join(",")}`,
    session,
    fights,
    kind,
    fightCount: kind === "whole" ? session.fightCount : fights.length,
    sizeBytes,
    suggestedName,
    zone: dominantZone(fights),
    label: sessionLabel(session, fights),
  };
}

async function writePlan(
  filePath: string,
  preflight: LogPreflight,
  plan: PlannedReport,
  name: string | null
): Promise<string[]> {
  if (plan.kind === "whole") {
    const selection: SplitSelection = {
      index: plan.session.index,
      name,
      startOffset: plan.session.startOffset,
      startTimeMs: plan.session.startTimeMs,
    };
    return invokeOrThrow<string[]>("uploader_split_to_disk_named", {
      filePath,
      sessions: preflight.sessions,
      selections: [selection],
    });
  }

  const fightSelections: FightSelection[] = plan.fights.map((fight) => ({
    index: fight.index,
    name: plan.kind === "single-fight" ? name : null,
    startOffset: fight.startOffset,
    startMs: fight.startMs,
  }));

  if (plan.kind === "single-fight") {
    return invokeOrThrow<string[]>("uploader_split_fights_to_disk", {
      filePath,
      sessions: preflight.sessions,
      fights: preflight.fights,
      selections: fightSelections,
    });
  }

  return invokeOrThrow<string[]>("uploader_split_session_fights_to_disk", {
    filePath,
    sessions: preflight.sessions,
    fights: preflight.fights,
    selection: {
      index: plan.session.index,
      name,
      startOffset: plan.session.startOffset,
      startTimeMs: plan.session.startTimeMs,
      fights: fightSelections,
    },
  });
}

/** Exported for test — the tri-state drives the session checkbox and its
 *  indeterminate flag, which is what tells a user a night is only partly picked. */
export function sessionCheckState(
  fights: FightSummary[],
  omitted: boolean,
  wholeSelected: boolean,
  selected: Set<number>
): CheckState {
  if (omitted || fights.length === 0) return wholeSelected ? "all" : "none";
  if (selected.size === 0) return "none";
  return selected.size >= fights.length ? "all" : "some";
}

function cloneFightMap(map: FightSelectionMap): FightSelectionMap {
  return new Map(Array.from(map.entries()).map(([key, value]) => [key, new Set(value)]));
}

function toggleSet<T>(set: Set<T>, value: T, present: boolean): Set<T> {
  const next = new Set(set);
  if (present) next.add(value);
  else next.delete(value);
  return next;
}

function attemptOrdinals(fights: FightSummary[]): Map<number, { position: number; total: number }> {
  const out = new Map<number, { position: number; total: number }>();
  let i = 0;
  while (i < fights.length) {
    const boss = fights[i]?.bossName;
    if (!boss) {
      i += 1;
      continue;
    }
    let j = i + 1;
    while (j < fights.length && fights[j]?.bossName === boss) j += 1;
    const total = j - i;
    if (total > 1) {
      for (let n = i; n < j; n++) out.set(fights[n]!.index, { position: n - i + 1, total });
    }
    i = j;
  }
  return out;
}

function fightName(fight: FightSummary, ordinal?: { position: number; total: number }): string {
  if (!fight.bossName) return `Trash - ${fight.zoneName || "Unknown zone"}`;
  if (ordinal) return `${fight.bossName} - pull ${ordinal.position}/${ordinal.total}`;
  return fightLabel(fight);
}

function fightAriaLabel(fight: FightSummary, name: string): string {
  const parts = [`Include ${name}`];
  if (fight.difficulty === "veteran") parts.push("veteran");
  if (fight.outcome === "kill") parts.push("kill");
  if (fight.outcome === "wipe") parts.push("wipe");
  parts.push(formatDuration(fight.endMs - fight.startMs));
  parts.push(compactBytes(Math.max(0, fight.endOffset - fight.startOffset)));
  return parts.join(", ");
}

function sessionLabel(session: LogSession, fights: FightSummary[] = []): string {
  const start = new Date(session.startTimeMs);
  const day = start
    .toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" })
    .toUpperCase();
  // `endMs` is an offset from the session's BEGIN_LOG, not an epoch, so it only
  // becomes a clock time once added to the session start — the same mistake the
  // fight rows made. Without a fight list there is no end to show at all, and
  // repeating the start produced a meaningless "12:25 AM-12:25 AM".
  if (fights.length === 0) return `${day} - ${formatClock(session.startTimeMs)}`;
  const endMs = session.startTimeMs + Math.max(...fights.map((f) => f.endMs));
  return `${day} - ${formatClock(session.startTimeMs)}-${formatClock(endMs)}`;
}

function formatClock(ms: number): string {
  if (!ms) return "--";
  return new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function basename(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

/** The slices still owed after `uploadedCount` of them succeeded.
 *
 *  Exported for test because the UI path is hard to reach on purpose — each
 *  report takes only a few seconds, so a Stop lands between two of them mostly by
 *  luck. Getting this backwards is expensive and silent: esologs.com assigns a
 *  report code per request with no client idempotency key, so re-sending a
 *  finished slice publishes a duplicate while the slice that actually failed is
 *  never retried. */
export function outstandingAfter<T>(files: T[], uploadedCount: number): T[] {
  return files.slice(Math.max(0, Math.min(uploadedCount, files.length)));
}

async function showInFolder(path: string): Promise<void> {
  try {
    const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
    await revealItemInDir(path);
  } catch {
    toast.error("Couldn't show the file in its folder.");
  }
}

async function openReport(
  report: NonNullable<UploadDispatch["report"]>,
  visibility: Visibility
): Promise<void> {
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(await primaryReportUrlForOpen(report, visibility));
  } catch {
    toast.error("Couldn't open the report - copy the link and open it manually.");
  }
}
