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
 * nothing) and resumes on reveal. If the element has no animations — e.g.
 * `prefers-reduced-motion` styles set `animation: none` — this is a no-op.
 */
export function useCappedAnimationRate(ref: RefObject<HTMLElement | null>, intervalMs: number) {
  useEffect(() => {
    const el = ref.current;
    if (el === null) return;
    const anims = el.getAnimations();
    if (anims.length === 0) return;
    for (const a of anims) a.pause();

    let last = performance.now();
    let timer: number | null = null;
    const tick = () => {
      const now = performance.now();
      const dt = now - last;
      last = now;
      for (const a of anims) {
        const t = a.currentTime;
        if (typeof t === "number") a.currentTime = t + dt;
      }
    };
    const start = () => {
      if (timer === null && !document.hidden) {
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
    const onVisibilityChange = () => {
      if (document.hidden) stop();
      else start();
    };

    start();
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibilityChange);
      // If the element stays mounted (HMR/StrictMode re-run), hand the
      // timelines back to CSS. If it is being unmounted, CANCEL instead: a
      // played animation on a detached target keeps Blink requesting a main
      // frame every vsync forever (measured 240 main-frame requests/s leaked
      // after one dialog open/close) with nothing to show.
      for (const a of anims) {
        if (el.isConnected) a.play();
        else a.cancel();
      }
    };
  }, [ref, intervalMs]);
}
