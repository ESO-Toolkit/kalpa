import { useId, useRef } from "react";
import { HexColorPicker } from "react-colorful";
import { Pipette } from "lucide-react";
import { cn } from "@/lib/utils";
import { Popover, PopoverTrigger, PopoverContent } from "@/components/ui/popover";
import { isHexColor, normalizeHex } from "@/lib/theme-color";

/**
 * Color picker control for the theme editor.
 *
 * - A swatch opens an inline visual picker (react-colorful — zero-dep, accessible).
 * - A hex text field for typing/pasting exact values.
 * - The EyeDropper API (sample any pixel on screen), feature-detected so it only
 *   appears where supported (evergreen Chromium / WebView2).
 *
 * Native `<input type="color">` is intentionally avoided: it's unstyleable and
 * opens a separate OS modal. An inline picker is the better fit for a theme tool.
 */
export function ColorInput({
  label,
  hint,
  value,
  onChange,
  className,
}: {
  label: string;
  hint?: string;
  value: string;
  onChange: (hex: string) => void;
  className?: string;
}) {
  const id = useId();
  const textRef = useRef<HTMLInputElement>(null);
  const normalized = isHexColor(value) ? normalizeHex(value) : "#000000";

  const supportsEyeDropper = typeof window !== "undefined" && "EyeDropper" in window;

  const pickWithEyeDropper = async () => {
    try {
      const EyeDropperCtor = (
        window as unknown as {
          EyeDropper: new () => { open: () => Promise<{ sRGBHex: string }> };
        }
      ).EyeDropper;
      const result = await new EyeDropperCtor().open();
      if (result?.sRGBHex && isHexColor(result.sRGBHex)) {
        onChange(normalizeHex(result.sRGBHex));
      }
    } catch {
      // User cancelled (AbortError) — no-op.
    }
  };

  const commitText = (raw: string) => {
    const trimmed = raw.trim();
    if (isHexColor(trimmed)) onChange(normalizeHex(trimmed));
  };

  return (
    <div className={cn("flex items-center gap-2", className)}>
      <Popover>
        <PopoverTrigger
          className="size-7 shrink-0 cursor-pointer rounded-md border border-white/[0.12] shadow-[inset_0_1px_0_rgba(255,255,255,0.1)] transition-transform duration-150 hover:scale-105"
          style={{ background: normalized }}
          aria-label={`${label} — open color picker`}
        />
        <PopoverContent className="w-auto p-2" side="bottom" align="start">
          <div className="kalpa-color-picker">
            <HexColorPicker color={normalized} onChange={(hex) => onChange(normalizeHex(hex))} />
          </div>
        </PopoverContent>
      </Popover>

      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-2">
          <label htmlFor={id} className="text-xs font-medium text-white/85">
            {label}
          </label>
          {hint && <span className="truncate text-[10px] text-muted-foreground">{hint}</span>}
        </div>
        <input
          id={id}
          ref={textRef}
          type="text"
          value={value}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => onChange(e.target.value)}
          onBlur={(e) => commitText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              commitText((e.target as HTMLInputElement).value);
              textRef.current?.blur();
            }
          }}
          className="mt-0.5 w-full rounded-md border border-white/[0.08] bg-white/[0.03] px-1.5 py-0.5 font-mono text-[11px] text-white/80 outline-none transition-colors duration-150 hover:border-white/15 focus:border-accent-sky/40 focus:bg-white/[0.05]"
        />
      </div>

      {supportsEyeDropper && (
        <button
          type="button"
          onClick={pickWithEyeDropper}
          className="flex size-7 shrink-0 items-center justify-center rounded-md border border-white/[0.08] bg-white/[0.03] text-muted-foreground transition-colors duration-150 hover:border-white/15 hover:text-white/80"
          aria-label={`Sample a color for ${label} from the screen`}
          title="Sample a color from the screen"
        >
          <Pipette className="size-3.5" />
        </button>
      )}
    </div>
  );
}
