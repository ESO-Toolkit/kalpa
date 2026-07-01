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
const orbGradient = (color: string, corePct: number) =>
  `radial-gradient(circle closest-side at center, ${color} 0%, ${color} ${corePct}%, transparent 100%)`;

export function AppBackground() {
  return (
    <div className="fixed inset-0 -z-10 overflow-hidden bg-bg-base">
      {/* Material texture (art themes only; "none" by default) */}
      <div
        className="absolute inset-0"
        style={{ backgroundImage: "var(--app-texture)", backgroundSize: "var(--app-texture-size)" }}
      />
      {/* orb 1 — former 600px disk @ (-15%,-10%), blur-120px (gold) */}
      <div
        className="absolute [will-change:transform] animate-[orb-drift_25s_ease-in-out_infinite]"
        style={{
          left: "calc(-10% - 280px)",
          top: "calc(-15% - 280px)",
          width: "1160px",
          height: "1160px",
          backgroundImage: orbGradient("color-mix(in oklab, var(--orb-1) 22%, transparent)", 34),
        }}
      />
      {/* orb 2 — former 500px disk @ (bottom -20%, right -10%), blur-120px (sky) */}
      <div
        className="absolute [will-change:transform] animate-[orb-drift_20s_ease-in-out_infinite_reverse]"
        style={{
          right: "calc(-10% - 280px)",
          bottom: "calc(-20% - 280px)",
          width: "1060px",
          height: "1060px",
          backgroundImage: orbGradient("color-mix(in oklab, var(--orb-2) 17%, transparent)", 32),
        }}
      />
      {/* orb 3 — former 400px disk @ (30%,40%), blur-100px (indigo) */}
      <div
        className="absolute [will-change:transform] animate-[orb-drift_30s_ease-in-out_infinite]"
        style={{
          left: "calc(40% - 230px)",
          top: "calc(30% - 230px)",
          width: "860px",
          height: "860px",
          backgroundImage: orbGradient("color-mix(in oklab, var(--orb-3) 11%, transparent)", 32),
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
