// The manual-upload progress experience: a determinate, phase-driven progress bar
// with a live "time remaining" estimate. It is fed by REAL backend lifecycle ticks
// (UploadProgressEvent over a per-upload Channel) — Preparing → Uploading →
// Finalizing → Done — and eases between those checkpoints with a gentle time-based
// creep so the bar never looks frozen while the (single, large) segment is in flight.
//
// Honest by construction: the ETA is derived from elapsed time and the displayed
// fraction (the standard progress-derived estimate), and is clearly labelled an
// estimate. The official-uploader handoff path emits no ticks, so this panel only
// renders for the native direct route; the handoff keeps its own indeterminate copy.

import { useEffect, useRef, useState } from "react";
import { CloudUpload, FileArchive, Loader2, Send, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";
import type { UploadPhase } from "@/types/uploader";

/** The live progress signal the panel animates from. `startMs` anchors the ETA. */
export interface UploadProgressState {
  phase: UploadPhase;
  segmentsDone: number;
  segmentsTotal: number;
  startMs: number;
}

/** Lower/upper bounds of the Uploading band — completed-segment progress fills the
 *  space between them; Preparing owns everything below, Finalizing/Done above. */
const UPLOAD_BAND_START = 0.15;
const UPLOAD_BAND_END = 0.93;

/** The settled (checkpoint) fraction a given progress state maps to, in [0, 1].
 *  Phases own contiguous bands so the bar advances monotonically: Preparing fills
 *  the first ~15%, Uploading the middle (driven by real segment counts), Finalizing
 *  the last sliver, Done completes it.
 *
 *  `phaseElapsedMs` is time spent in the CURRENT phase. It lets each phase creep
 *  toward — but never past — its ceiling while no new backend tick has arrived, so a
 *  long payload build, or the (single, large) segment whose POST is in flight, still
 *  shows motion instead of freezing at a checkpoint. The key case: the native manual
 *  path emits `uploading {done: 0, total: 1}` before the POST and only ticks again
 *  once the segment is accepted, so `done/total = 0` must creep across that segment's
 *  band rather than sit at its floor. Exported for unit tests. */
export function uploadTargetFraction(state: UploadProgressState, phaseElapsedMs: number): number {
  const approach = (cap: number, base: number, tauMs: number) =>
    base + (cap - base) * (1 - Math.exp(-Math.max(0, phaseElapsedMs) / tauMs));
  switch (state.phase) {
    case "preparing":
      // 0 → 0.15, most of it in the first few seconds.
      return approach(UPLOAD_BAND_START, 0, 3000);
    case "uploading": {
      // Treat a not-yet-known count as a single in-flight segment.
      const total = Math.max(1, state.segmentsTotal);
      const done = Math.min(total, Math.max(0, state.segmentsDone));
      const span = UPLOAD_BAND_END - UPLOAD_BAND_START;
      // Floor = segments already ACCEPTED. Ceiling = where the next accepted tick
      // will land. The in-flight segment's true byte progress is unobservable, so
      // creep from floor toward its checkpoint but stop just short (0.92 of the way)
      // — the real "accepted" event then visibly completes that segment.
      const floor = UPLOAD_BAND_START + span * (done / total);
      const ceil = UPLOAD_BAND_START + span * Math.min(1, (done + 1) / total);
      const cap = floor + (ceil - floor) * 0.92;
      return approach(cap, floor, 5000);
    }
    case "finalizing":
      return 0.96;
    case "done":
      return 1;
  }
}

/** Format a millisecond ETA as a compact "time remaining" string. */
export function formatEta(ms: number): string {
  if (ms <= 900) return "<1s";
  const sec = Math.round(ms / 1000);
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Derive the "time remaining" line from elapsed time and the displayed fraction —
 *  the standard progress-derived estimate (eta = elapsed · (1 − f) / f). Returns soft
 *  copy at the boundaries where a number would be noise. Exported for unit tests. */
export function etaLabel(phase: UploadPhase, fraction: number, elapsedMs: number): string {
  if (phase === "done" || fraction >= 0.999) return "Done";
  if (phase === "finalizing" || fraction >= 0.92) return "Finishing up…";
  if (fraction < 0.06 || elapsedMs < 400) return "Estimating…";
  const remaining = (elapsedMs * (1 - fraction)) / fraction;
  return `about ${formatEta(remaining)} left`;
}

const PHASE_LABEL: Record<UploadPhase, string> = {
  preparing: "Preparing your log",
  uploading: "Uploading combat data",
  finalizing: "Finalizing report",
  done: "Upload complete",
};

const PHASE_ICON: Record<UploadPhase, typeof CloudUpload> = {
  preparing: FileArchive,
  uploading: Send,
  finalizing: Sparkles,
  done: CloudUpload,
};

/** The ordered phase steps shown as a stepper under the bar. */
const STEPS: { key: UploadPhase; label: string }[] = [
  { key: "preparing", label: "Prepare" },
  { key: "uploading", label: "Upload" },
  { key: "finalizing", label: "Finalize" },
];

const STEP_ORDER: Record<UploadPhase, number> = {
  preparing: 0,
  uploading: 1,
  finalizing: 2,
  done: 3,
};

/**
 * The animated manual-upload progress panel. Runs its own rAF loop so the fill and
 * ETA stay smooth independent of how often backend ticks arrive, easing the displayed
 * fraction toward the current phase's target and never moving backward. Reduced-motion
 * users get the same final values without the per-frame easing.
 */
export function UploadProgressPanel({
  state,
  fileName,
  className,
}: {
  state: UploadProgressState;
  fileName?: string | null;
  className?: string;
}) {
  // Displayed fraction (0..1) and the ETA line are animated in a rAF loop; mirror
  // them into refs so the loop reads the latest without re-subscribing each frame.
  const [fraction, setFraction] = useState(0);
  const [eta, setEta] = useState("Estimating…");
  const fractionRef = useRef(0);
  const maxTargetRef = useRef(0);
  const stateRef = useRef(state);
  // When the current phase began (performance.now), so the creep is anchored to
  // time-in-phase, not total elapsed — the in-flight segment must creep from the
  // instant Uploading starts, regardless of how long Preparing took.
  const phaseRef = useRef<UploadPhase | null>(null);
  const phaseStartRef = useRef(0);
  // Keep the rAF loop's view of `state` current without re-subscribing each tick.
  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  useEffect(() => {
    const prefersReduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

    let raf = 0;
    let last = performance.now();

    const frame = (now: number) => {
      const dt = Math.min(100, now - last);
      last = now;
      const s = stateRef.current;
      const elapsed = Date.now() - s.startMs;

      // Reset the per-phase clock whenever the phase changes so each phase's creep
      // starts from zero.
      if (phaseRef.current !== s.phase) {
        phaseRef.current = s.phase;
        phaseStartRef.current = now;
      }
      const phaseElapsed = now - phaseStartRef.current;

      // Targets are monotonic non-decreasing: a time-creep target from one phase must
      // never be undercut by a fresh-but-lower checkpoint from the next.
      const rawTarget = uploadTargetFraction(s, phaseElapsed);
      const target = Math.max(rawTarget, maxTargetRef.current);
      maxTargetRef.current = target;

      // Ease toward the target (exponential smoothing); snap hard once done so the bar
      // visibly completes. Never decreases.
      const next =
        s.phase === "done" || prefersReduced
          ? target
          : fractionRef.current + (target - fractionRef.current) * (1 - Math.exp(-dt / 220));
      fractionRef.current = Math.max(fractionRef.current, Math.min(1, next));

      setFraction((prev) =>
        Math.abs(prev - fractionRef.current) > 0.0005 ? fractionRef.current : prev
      );
      setEta(etaLabel(s.phase, fractionRef.current, elapsed));

      if (!(s.phase === "done" && fractionRef.current >= 0.999)) {
        raf = requestAnimationFrame(frame);
      }
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, []);

  const pct = Math.round(fraction * 100);
  const Icon = PHASE_ICON[state.phase];
  const done = state.phase === "done";
  const activeStep = STEP_ORDER[state.phase];

  return (
    <div
      className={cn(
        // The active climax surface: warm gold glass to match the upload action it
        // replaces, with a strong lift so it reads as the live focus of the flow.
        "relative w-full overflow-hidden rounded-2xl border border-primary/25 bg-gradient-to-b from-primary/[0.1] to-primary/[0.02] p-5 shadow-[0_16px_44px_-16px_rgba(0,0,0,0.75),inset_0_1px_0_rgba(255,255,255,0.08)]",
        className
      )}
      role="status"
      aria-live="polite"
    >
      <span
        className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-primary/0 via-primary/60 to-primary/0"
        aria-hidden
      />

      {/* Header row: phase + glanceable percent. */}
      <div className="flex items-center justify-between gap-3">
        <span className="flex min-w-0 items-center gap-2.5">
          <span className="relative flex size-8 shrink-0 items-center justify-center rounded-full bg-primary/15 text-primary">
            {done ? (
              <Icon className="size-4" aria-hidden />
            ) : (
              <>
                <Loader2 className="absolute size-8 animate-spin text-primary/40" aria-hidden />
                <Icon className="size-4" aria-hidden />
              </>
            )}
          </span>
          <span className="flex min-w-0 flex-col">
            <span className="truncate text-sm font-medium text-foreground">
              {PHASE_LABEL[state.phase]}
            </span>
            {fileName && (
              <span className="truncate text-xs text-muted-foreground" title={fileName}>
                {fileName}
              </span>
            )}
          </span>
        </span>
        <span className="shrink-0 text-right">
          <span className="font-heading text-lg tabular-nums text-foreground">{pct}%</span>
        </span>
      </div>

      {/* The bar. */}
      <div
        className="relative mt-3.5 h-2 overflow-hidden rounded-full bg-black/40 ring-1 ring-inset ring-white/[0.04]"
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={`Upload progress: ${pct}%`}
      >
        <div
          className="absolute inset-y-0 left-0 rounded-full bg-gradient-to-r from-primary to-primary-hover"
          style={{ width: `${Math.max(fraction * 100, 2)}%` }}
        >
          {/* Travelling sheen — only while still working. */}
          {!done && (
            <div className="absolute inset-0 overflow-hidden rounded-full">
              <div className="h-full w-full animate-[shimmer_1.5s_ease-in-out_infinite] bg-gradient-to-r from-transparent via-white/25 to-transparent motion-reduce:hidden" />
            </div>
          )}
        </div>
      </div>

      {/* Footer row: phase stepper + ETA. */}
      <div className="mt-3 flex items-center justify-between gap-3">
        <div className="flex items-center gap-1.5">
          {STEPS.map((step, i) => {
            const reached = activeStep > i || done;
            const current = activeStep === i && !done;
            return (
              <span key={step.key} className="flex items-center gap-1.5">
                <span
                  className={cn(
                    "size-1.5 rounded-full transition-colors duration-300",
                    reached ? "bg-primary" : current ? "bg-primary/70" : "bg-white/15",
                    current && "animate-pulse"
                  )}
                  aria-hidden
                />
                <span
                  className={cn(
                    "text-[11px] transition-colors duration-300",
                    reached || current ? "text-foreground/80" : "text-muted-foreground/60"
                  )}
                >
                  {step.label}
                </span>
                {i < STEPS.length - 1 && (
                  <span
                    className={cn(
                      "h-px w-3 transition-colors duration-300",
                      activeStep > i || done ? "bg-primary/50" : "bg-white/10"
                    )}
                    aria-hidden
                  />
                )}
              </span>
            );
          })}
        </div>
        <span className="shrink-0 text-xs tabular-nums text-muted-foreground" aria-live="polite">
          {eta}
        </span>
      </div>
    </div>
  );
}
