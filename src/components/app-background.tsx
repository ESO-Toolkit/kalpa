// Ambient drifting orbs. Rendered as radial-gradient paints rather than a solid
// circle + `filter: blur()`. A large `filter: blur(120px)` forces Blink to
// allocate a compositor backing texture sized to the blur-EXPANDED bounds
// (~3× the blur radius on every side) for the whole app lifetime — the single
// biggest GPU allocation tied purely to the background. A radial gradient
// reproduces the same soft, low-opacity glow with no filter pass and a backing
// store bounded to the element box, removing that permanent GPU texture cost.
//
// Each orb keeps the FORMER blurred disk's geometry: the box is sized to the
// original disk plus ~2× its blur halo and centered on the original disk's
// center, and the gradient holds a flat saturated core out to the disk's radius
// fraction (`core%`) before fading to transparent — so the drifting glow reads
// the same as before. Drift + will-change are kept so each orb still animates on
// the compositor.
// `closest-side` so the fade terminates at the (invisible, clipped) box edge —
// no mid-canvas ring. A small flat core out to `corePct` keeps a saturated
// center, then a long gradual fall to transparent mimics the former Gaussian
// halo with no visible boundary.
import { useEffect, useRef, useState } from "react";

const orbGradient = (color: string, corePct: number) =>
  `radial-gradient(circle closest-side at center, ${color} 0%, ${color} ${corePct}%, transparent 100%)`;

const isVisibleAndFocused = () =>
  typeof document === "undefined" ? true : !document.hidden && document.hasFocus();

const prefersReducedMotion = () =>
  typeof window === "undefined"
    ? false
    : window.matchMedia("(prefers-reduced-motion: reduce)").matches;

// The drift is driven by seeking the (paused) CSS animations from ONE shared
// low-rate timer instead of letting them free-run on the compositor. A running
// compositor animation forces the compositor to produce a frame every vsync for
// the app's whole lifetime — measured 238 DrawFrames/s and ~0.9 CPU cores while
// the app sat idle-focused on a 240 Hz display, because every presented frame
// also re-executes each overlapping glass panel's backdrop-blur. A PAUSED
// animation cancels the compositor keyframe model entirely, so between seeks
// the pipeline is fully idle (zero frames, zero backdrop re-blurs); each seek
// costs one small style commit + ONE presented frame. At 10 Hz the orbs'
// worst-case step is ~1.5px of a glow whose falloff spans hundreds of pixels —
// visually indistinguishable from the vsync-rate drift, for ~1/24th of the
// frames. The eased trajectory is untouched (we seek the timeline, we don't
// re-time the keyframes).
const DRIFT_TICK_MS = 100;

export function AppBackground() {
  // Pause the ambient orb drift when Kalpa isn't visible/focused: the drift is
  // imperceptible while you're not looking — purely a power win. Freezing is
  // just "stop seeking"; the orbs hold position and resume on focus.
  // Initial value is derived lazily (not via a setState-in-effect) so the repo's
  // react-hooks/set-state-in-effect lint stays green.
  const [active, setActive] = useState(() => isVisibleAndFocused() && !prefersReducedMotion());
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

    let anims: Animation[] = [];
    let timer: number | null = null;
    let last = 0;

    // Pause the natively-running CSS animations and take ownership of their
    // timelines. Collected lazily again on tick in case style recalc replaced
    // them (e.g. dev HMR); pausing an already-paused animation is a no-op.
    const collect = () => {
      anims = root.getAnimations({ subtree: true });
      for (const a of anims) a.pause();
    };
    const tick = () => {
      const now = performance.now();
      const dt = now - last;
      last = now;
      // Re-collect if the styled animations were replaced (a canceled
      // CSSAnimation reports currentTime === null), not just on empty.
      if (anims.length === 0 || anims.every((a) => a.currentTime === null)) collect();
      for (const a of anims) {
        const t = a.currentTime;
        if (typeof t === "number") a.currentTime = t + dt;
      }
    };
    const start = () => {
      // Honor prefers-reduced-motion by never seeking: the ambient layer holds
      // still, which is exactly what a reduced-motion user asked for.
      if (timer !== null || reduceMotion.matches) return;
      last = performance.now();
      timer = window.setInterval(tick, DRIFT_TICK_MS);
    };
    const stop = () => {
      if (timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };

    // Drive the root `.app-hidden` class (window hidden: tray/minimized) and
    // `.app-unfocused` (visible on a second monitor while the game has focus).
    // CSS pauses the looping decorative animations under each; spinners stay
    // exempt from the unfocused pause so progress never looks hung mid-game.
    const syncHidden = () => {
      document.documentElement.classList.toggle("app-hidden", document.hidden);
      document.documentElement.classList.toggle(
        "app-unfocused",
        !document.hidden && !document.hasFocus()
      );
    };
    const update = () => {
      const on = isVisibleAndFocused() && !reduceMotion.matches;
      setActive(on);
      syncHidden();
      if (on) start();
      else stop();
    };

    collect();
    syncHidden(); // set the class from the current state without a mount setState
    if (isVisibleAndFocused() && !reduceMotion.matches) start();

    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    document.addEventListener("visibilitychange", update);
    reduceMotion.addEventListener("change", update);
    return () => {
      stop();
      // If the orbs stay mounted (HMR/StrictMode re-run), hand the timelines
      // back to CSS; if they're being detached, cancel — a played animation on
      // a detached target leaks per-vsync main-frame scheduling forever.
      // Canceled animations (playState "idle") must not be play()ed either:
      // that would restart drift the ambient-animations toggle removed.
      for (const a of anims) {
        if (root.isConnected && a.playState !== "idle") a.play();
        else a.cancel();
      }
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
      document.removeEventListener("visibilitychange", update);
      reduceMotion.removeEventListener("change", update);
      document.documentElement.classList.remove("app-hidden");
      document.documentElement.classList.remove("app-unfocused");
    };
  }, []);

  // Only pin the orb layers to their own GPU textures WHILE they're being
  // seeked. When frozen (unfocused/hidden/reduced-motion) drop will-change so
  // the 3 large backing textures are released — the drift can't jank if it
  // isn't advancing.
  const willChange = active ? "transform" : "auto";

  return (
    <div
      ref={rootRef}
      data-slot="app-background"
      className="fixed inset-0 -z-10 overflow-hidden bg-bg-base"
    >
      {/* Material texture (art themes only; "none" by default) */}
      <div
        className="absolute inset-0"
        style={{ backgroundImage: "var(--app-texture)", backgroundSize: "var(--app-texture-size)" }}
      />
      {/* orb 1 — former 600px disk @ (-15%,-10%), blur-120px (gold) */}
      <div
        className="absolute animate-[orb-drift_25s_ease-in-out_infinite]"
        style={{
          left: "calc(-10% - 280px)",
          top: "calc(-15% - 280px)",
          width: "1160px",
          height: "1160px",
          backgroundImage: orbGradient("color-mix(in oklab, var(--orb-1) 22%, transparent)", 34),
          willChange,
        }}
      />
      {/* orb 2 — former 500px disk @ (bottom -20%, right -10%), blur-120px (sky) */}
      <div
        className="absolute animate-[orb-drift_20s_ease-in-out_infinite_reverse]"
        style={{
          right: "calc(-10% - 280px)",
          bottom: "calc(-20% - 280px)",
          width: "1060px",
          height: "1060px",
          backgroundImage: orbGradient("color-mix(in oklab, var(--orb-2) 17%, transparent)", 32),
          willChange,
        }}
      />
      {/* orb 3 — former 400px disk @ (30%,40%), blur-100px (indigo) */}
      <div
        className="absolute animate-[orb-drift_30s_ease-in-out_infinite]"
        style={{
          left: "calc(40% - 230px)",
          top: "calc(30% - 230px)",
          width: "860px",
          height: "860px",
          backgroundImage: orbGradient("color-mix(in oklab, var(--orb-3) 11%, transparent)", 32),
          willChange,
        }}
      />
      {/* Ornamental motif tile (art themes only; "none" by default) */}
      <div
        className="absolute inset-0"
        style={{
          backgroundImage: "var(--app-pattern)",
          backgroundSize: "var(--app-pattern-size)",
          opacity: "var(--app-pattern-opacity)",
        }}
      />
    </div>
  );
}
