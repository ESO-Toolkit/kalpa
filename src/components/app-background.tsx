import { useEffect } from "react";

export function AppBackground() {
  // Pause the ambient orb/shimmer animations while the window is hidden (e.g.
  // minimized to the tray). Toggling a root class lets CSS set
  // animation-play-state: paused, so the compositor stops spending frames on the
  // three large blurred orbs. Animations resume exactly where they left off when
  // the window is shown again — the visible appearance is unchanged.
  useEffect(() => {
    const sync = () => {
      document.documentElement.classList.toggle("app-hidden", document.hidden);
    };
    sync();
    document.addEventListener("visibilitychange", sync);
    return () => {
      document.removeEventListener("visibilitychange", sync);
      document.documentElement.classList.remove("app-hidden");
    };
  }, []);

  return (
    <div data-slot="app-background" className="fixed inset-0 -z-10 overflow-hidden bg-bg-base">
      {/* Material texture (art themes only; "none" by default) */}
      <div
        className="absolute inset-0"
        style={{ backgroundImage: "var(--app-texture)", backgroundSize: "var(--app-texture-size)" }}
      />
      <div className="absolute -top-[15%] -left-[10%] h-[600px] w-[600px] rounded-full bg-[var(--orb-1)]/20 blur-[120px] [will-change:transform] animate-[orb-drift_25s_ease-in-out_infinite]" />
      <div className="absolute -bottom-[20%] -right-[10%] h-[500px] w-[500px] rounded-full bg-[var(--orb-2)]/15 blur-[120px] [will-change:transform] animate-[orb-drift_20s_ease-in-out_infinite_reverse]" />
      <div className="absolute top-[30%] left-[40%] h-[400px] w-[400px] rounded-full bg-[var(--orb-3)]/10 blur-[100px] [will-change:transform] animate-[orb-drift_30s_ease-in-out_infinite]" />
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
