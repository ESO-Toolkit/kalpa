export function AppBackground() {
  return (
    <div className="fixed inset-0 -z-10 overflow-hidden bg-bg-base">
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
