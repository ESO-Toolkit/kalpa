/**
 * Small, dependency-free color helpers. NOTE: the real seed→token expansion
 * happens in CSS (index.css derivations via color-mix(in oklab)/OKLCH relative
 * color), applied by theme-apply.ts — NOT here. These are plain sRGB utilities
 * used only off the render path: WCAG contrast scoring (theme-contrast.ts),
 * gallery swatch preview, and hex validation/normalization in the editor.
 * `mix`/`lighten`/`darken` are naive sRGB blends (test-covered) — prefer the
 * CSS oklab path for anything user-visible to avoid muddy mid-tones.
 */

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

/** Parse `#rgb` / `#rrggbb` (with or without leading `#`) into 0–255 channels. */
export function hexToRgb(hex: string): Rgb {
  let h = hex.trim().replace(/^#/, "");
  if (h.length === 3) {
    h = h
      .split("")
      .map((c) => c + c)
      .join("");
  }
  if (h.length !== 6 || /[^0-9a-fA-F]/.test(h)) {
    // Fall back to a neutral grey rather than throwing — keeps theming resilient
    // to a malformed custom color.
    return { r: 128, g: 128, b: 128 };
  }
  return {
    r: parseInt(h.slice(0, 2), 16),
    g: parseInt(h.slice(2, 4), 16),
    b: parseInt(h.slice(4, 6), 16),
  };
}

const clamp = (n: number) => Math.max(0, Math.min(255, Math.round(n)));

export function rgbToHex({ r, g, b }: Rgb): string {
  const toHex = (n: number) => clamp(n).toString(16).padStart(2, "0");
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

/** Linear per-channel blend. `t = 0` → `a`, `t = 1` → `b`. */
export function mix(a: string, b: string, t: number): string {
  const ca = hexToRgb(a);
  const cb = hexToRgb(b);
  return rgbToHex({
    r: ca.r + (cb.r - ca.r) * t,
    g: ca.g + (cb.g - ca.g) * t,
    b: ca.b + (cb.b - ca.b) * t,
  });
}

/** Blend toward white by `amount` (0–1). */
export const lighten = (hex: string, amount: number) => mix(hex, "#ffffff", amount);

/** Blend toward black by `amount` (0–1). */
export const darken = (hex: string, amount: number) => mix(hex, "#000000", amount);

/** `rgba(r, g, b, a)` string from a hex color + alpha (0–1). */
export function rgba(hex: string, alpha: number): string {
  const { r, g, b } = hexToRgb(hex);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** WCAG relative luminance (0–1) of a hex color. */
export function relativeLuminance(hex: string): number {
  const { r, g, b } = hexToRgb(hex);
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast ratio between two hex colors (1–21). */
export function contrastRatio(a: string, b: string): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const lighter = Math.max(la, lb);
  const darker = Math.min(la, lb);
  return (lighter + 0.05) / (darker + 0.05);
}

/** True if a string looks like a valid 3- or 6-digit hex color. */
export function isHexColor(value: string): boolean {
  return /^#?([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.test(value.trim());
}

/** Normalize any accepted hex form to `#rrggbb` lowercase. */
export function normalizeHex(value: string): string {
  return rgbToHex(hexToRgb(value)).toLowerCase();
}
