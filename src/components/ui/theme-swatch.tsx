import { cn } from "@/lib/utils";
import type { ThemeColors } from "@/lib/theme-types";

/**
 * A miniature, at-a-glance preview of a theme rendered directly from its seed
 * colors (no CSS-variable application needed, so many can render at once in the
 * gallery). Approximates the real app chrome: ambient glow, a glass panel, text,
 * a primary button, and an accent marker.
 */
export function ThemeSwatch({
  colors,
  className,
  active = false,
}: {
  colors: ThemeColors;
  className?: string;
  active?: boolean;
}) {
  return (
    <div
      className={cn("relative aspect-[16/10] w-full overflow-hidden rounded-lg", className)}
      style={{ background: colors.bgBase }}
      aria-hidden
    >
      {/* Ambient orbs */}
      <div
        className="absolute -left-4 -top-6 h-16 w-16 rounded-full blur-2xl"
        style={{ background: colors.orb1, opacity: 0.5 }}
      />
      <div
        className="absolute -bottom-6 right-2 h-16 w-16 rounded-full blur-2xl"
        style={{ background: colors.orb2, opacity: 0.4 }}
      />
      <div
        className="absolute right-8 top-2 h-12 w-12 rounded-full blur-2xl"
        style={{ background: colors.orb3, opacity: 0.3 }}
      />

      {/* App background wash */}
      <div className="absolute inset-0" style={{ background: colors.background, opacity: 0.7 }} />

      {/* Glass panel */}
      <div
        className="absolute inset-2 rounded-md p-2"
        style={{
          background: colors.surface,
          border: `1px solid ${colors.border}`,
          boxShadow: active ? `0 0 0 1px ${colors.primary}55` : undefined,
        }}
      >
        {/* Title + muted line */}
        <div className="flex items-center gap-1">
          <div className="h-1.5 w-1.5 rounded-full" style={{ background: colors.primary }} />
          <div
            className="h-1.5 w-[42%] rounded-full"
            style={{ background: colors.foreground, opacity: 0.85 }}
          />
        </div>
        <div
          className="mt-1.5 h-1 w-[64%] rounded-full"
          style={{ background: colors.mutedForeground, opacity: 0.7 }}
        />
        <div
          className="mt-1 h-1 w-[52%] rounded-full"
          style={{ background: colors.mutedForeground, opacity: 0.5 }}
        />

        {/* Controls row */}
        <div className="absolute bottom-2 left-2 right-2 flex items-center gap-1.5">
          <div
            className="h-2.5 w-8 rounded"
            style={{ background: colors.primary }}
            title="primary"
          />
          <div
            className="h-2.5 w-2.5 rounded-full"
            style={{ background: colors.accent }}
            title="accent"
          />
          <div
            className="ml-auto h-2 w-2 rounded-full ring-1"
            style={{ background: colors.surface, borderColor: colors.border, color: colors.accent }}
          />
        </div>
      </div>
    </div>
  );
}

/** A compact horizontal strip of the key seed colors, for dense lists. */
export function ThemeColorStrip({
  colors,
  className,
}: {
  colors: ThemeColors;
  className?: string;
}) {
  const keys: (keyof ThemeColors)[] = ["background", "surface", "primary", "accent", "foreground"];
  return (
    <div className={cn("flex h-4 overflow-hidden rounded", className)} aria-hidden>
      {keys.map((k) => (
        <div key={k} className="flex-1" style={{ background: colors[k] }} />
      ))}
    </div>
  );
}
