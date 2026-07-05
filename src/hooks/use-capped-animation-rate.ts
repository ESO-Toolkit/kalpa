import { useEffect, type RefObject } from "react";

/**
 * Drives an element's CSS animations by seeking them from a low-rate timer
 * instead of letting them free-run on the compositor.
 *
 * A running compositor animation forces a presented frame every vsync — on a
 * 240 Hz display that is 240 frames/s for as long as the element is on
 * screen, and every frame re-executes the backdrop-blur of any glass surface
 * the element sits in (measured: the dialog accent sweep alone cost ~42% of a
 * core in the GPU process). A PAUSED animation produces zero frames between
 * seeks, so capping the seek rate caps the whole pipeline's frame rate while
 * preserving the animation's timing function exactly (the timeline is seeked,
 * not re-timed).
 *
 * The timer stops while the document is hidden (a hidden window renders
 * nothing) or unfocused (Kalpa on a second monitor while the game has focus —
 * decorative motion freezes in place) and resumes on focus. If the element has
 * no animations — e.g. `prefers-reduced-motion` or the ambient-animations
 * toggle set `animation: none` — ticks are no-ops; animations recreated later
 * (toggle flipped back on) are re-collected and re-owned on the next tick.
 */
export function useCappedAnimationRate(ref: RefObject<HTMLElement | null>, intervalMs: number) {
  useEffect(() => {
    const el = ref.current;
    if (el === null) return;
    let anims = el.getAnimations();
    for (const a of anims) a.pause();

    let last = performance.now();
    let timer: number | null = null;
    const tick = () => {
      const now = performance.now();
      const dt = now - last;
      last = now;
      // Re-collect if the styled animations were replaced or removed (a
      // canceled CSSAnimation reports currentTime === null).
      if (anims.length === 0 || anims.every((a) => a.currentTime === null)) {
        anims = el.getAnimations();
        for (const a of anims) a.pause();
      }
      for (const a of anims) {
        const t = a.currentTime;
        if (typeof t === "number") a.currentTime = t + dt;
      }
    };
    const start = () => {
      if (timer === null && !document.hidden && document.hasFocus()) {
        last = performance.now();
        timer = window.setInterval(tick, intervalMs);
      }
    };
    const stop = () => {
      if (timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
    const onStateChange = () => {
      if (document.hidden || !document.hasFocus()) stop();
      else start();
    };

    start();
    document.addEventListener("visibilitychange", onStateChange);
    window.addEventListener("focus", onStateChange);
    window.addEventListener("blur", onStateChange);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onStateChange);
      window.removeEventListener("focus", onStateChange);
      window.removeEventListener("blur", onStateChange);
      // If the element stays mounted (HMR/StrictMode re-run), hand the
      // timelines back to CSS. If it is being unmounted, CANCEL instead: a
      // played animation on a detached target keeps Blink requesting a main
      // frame every vsync forever (measured 240 main-frame requests/s leaked
      // after one dialog open/close) with nothing to show.
      // (Canceled animations — playState "idle" — must not be play()ed either:
      // that would restart a sweep the ambient-animations toggle removed.)
      for (const a of anims) {
        if (el.isConnected && a.playState !== "idle") a.play();
        else a.cancel();
      }
    };
  }, [ref, intervalMs]);
}
