import { useEffect, useState } from "react";
import { toast } from "sonner";
import { ArrowLeft, Check, Copy, Trash2, AlertTriangle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SectionHeader } from "@/components/ui/section-header";
import { GlassPanel } from "@/components/ui/glass-panel";
import { ColorInput } from "@/components/ui/color-input";
import { ThemeSwatch } from "@/components/ui/theme-swatch";
import { previewThemeColors, stopPreview } from "@/lib/theme-manager";
import { evaluateContrast } from "@/lib/theme-contrast";
import { isHexColor, normalizeHex } from "@/lib/theme-color";
import { THEME_COLOR_KEYS, THEME_COLOR_META } from "@/lib/theme-types";
import type { Theme, ThemeColors } from "@/lib/theme-types";

/** Normalize valid hex; replace any malformed entry with a safe black so a bad
 * value can't be persisted/exported as raw text. */
function sanitizeColors(c: ThemeColors): ThemeColors {
  const out = {} as ThemeColors;
  for (const key of THEME_COLOR_KEYS) {
    out[key] = isHexColor(c[key]) ? normalizeHex(c[key]) : "#000000";
  }
  return out;
}

const GROUPS: { title: string; keys: (keyof ThemeColors)[] }[] = [
  { title: "Base & Surfaces", keys: ["bgBase", "background", "surface", "border"] },
  { title: "Text", keys: ["foreground", "mutedForeground"] },
  { title: "Accents", keys: ["primary", "primaryForeground", "accent"] },
  { title: "Ambient Glow", keys: ["orb1", "orb2", "orb3"] },
];

export function ThemeEditor({
  draft,
  isNew,
  onSave,
  onDelete,
  onClose,
}: {
  draft: Theme;
  isNew: boolean;
  onSave: (theme: Theme) => void;
  onDelete?: (id: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(draft.name);
  const [colors, setColors] = useState<ThemeColors>(draft.colors);
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Live full-app preview: push the in-progress colors onto the document while
  // editing (on every change), and restore the active theme once on unmount.
  useEffect(() => {
    previewThemeColors(colors);
  }, [colors]);
  useEffect(() => () => stopPreview(), []);

  const checks = evaluateContrast(colors);
  const failing = checks.filter((c) => c.level === "fail");

  const setColor = (key: keyof ThemeColors, value: string) =>
    setColors((prev) => ({ ...prev, [key]: value }));

  const handleSave = () => {
    const trimmed = name.trim();
    if (!trimmed) {
      toast.error("Give your theme a name.");
      return;
    }
    onSave({ ...draft, name: trimmed, colors: sanitizeColors(colors), custom: true });
  };

  const handleExport = async () => {
    try {
      const payload = JSON.stringify(
        { ...draft, name: name.trim() || draft.name, colors: sanitizeColors(colors), custom: true },
        null,
        2
      );
      await navigator.clipboard.writeText(payload);
      toast.success("Theme JSON copied to clipboard.");
    } catch {
      toast.error("Could not copy to clipboard.");
    }
  };

  return (
    <div className="space-y-3">
      {/* Header */}
      <div className="flex items-center gap-2">
        <Button variant="ghost" size="icon-sm" onClick={onClose} aria-label="Back to themes">
          <ArrowLeft className="size-4" />
        </Button>
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Theme name"
          className="h-8 flex-1"
          aria-label="Theme name"
        />
        <Button variant="outline" size="sm" onClick={handleExport}>
          <Copy className="size-3.5" />
          Export
        </Button>
      </div>

      {/* Live preview */}
      <GlassPanel variant="subtle" className="p-3">
        <ThemeSwatch colors={colors} className="max-w-[280px]" active />
        <p className="mt-2 text-[11px] text-muted-foreground">
          Changes preview across the whole app instantly. Nothing is saved until you click Save.
        </p>
      </GlassPanel>

      {/* Color groups */}
      {GROUPS.map((group) => (
        <GlassPanel key={group.title} variant="subtle" className="space-y-2.5 p-3">
          <SectionHeader>{group.title}</SectionHeader>
          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
            {group.keys.map((key) => (
              <ColorInput
                key={key}
                label={THEME_COLOR_META[key].label}
                hint={THEME_COLOR_META[key].hint}
                value={colors[key]}
                onChange={(hex) => setColor(key, hex)}
              />
            ))}
          </div>
        </GlassPanel>
      ))}

      {/* Contrast feedback */}
      <GlassPanel variant="subtle" className="space-y-2 p-3">
        <SectionHeader>Readability (WCAG)</SectionHeader>
        <div className="space-y-1.5">
          {checks.map((c) => (
            <div key={c.key} className="flex items-center gap-2 text-xs">
              <span
                className={`flex size-4 shrink-0 items-center justify-center rounded-full text-[9px] font-bold ${
                  c.level === "fail"
                    ? "bg-red-500/15 text-red-400"
                    : c.level === "ok"
                      ? "bg-amber-400/15 text-amber-400"
                      : "bg-emerald-400/15 text-emerald-400"
                }`}
              >
                {c.level === "fail" ? "!" : c.level === "ok" ? "~" : "✓"}
              </span>
              <span className="flex-1 text-white/75">{c.label}</span>
              <span className="font-mono text-[11px] text-muted-foreground">
                {c.ratio.toFixed(2)}:1
              </span>
            </div>
          ))}
        </div>
        {failing.length > 0 && (
          <div className="flex items-start gap-1.5 rounded-md border border-amber-400/20 bg-amber-400/[0.05] p-2 text-[11px] text-amber-300">
            <AlertTriangle className="mt-px size-3.5 shrink-0" />
            <span>
              {failing.length} pair{failing.length !== 1 ? "s" : ""} below the recommended contrast.
              You can still save, but some text may be hard to read.
            </span>
          </div>
        )}
      </GlassPanel>

      {/* Actions */}
      <div className="flex items-center gap-2 pb-1">
        <Button size="sm" onClick={handleSave}>
          <Check className="size-3.5" />
          {isNew ? "Save theme" : "Save changes"}
        </Button>
        <Button variant="outline" size="sm" onClick={onClose}>
          Cancel
        </Button>
        {!isNew && onDelete && (
          <div className="ml-auto">
            {confirmDelete ? (
              <div className="flex items-center gap-1.5">
                <span className="text-[11px] text-muted-foreground">Delete?</span>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={() => {
                    onDelete(draft.id);
                    onClose();
                  }}
                >
                  Yes
                </Button>
                <Button variant="outline" size="sm" onClick={() => setConfirmDelete(false)}>
                  No
                </Button>
              </div>
            ) : (
              <Button
                variant="outline"
                size="sm"
                className="border-red-500/30 text-red-400 hover:border-red-500/50 hover:bg-red-500/10"
                onClick={() => setConfirmDelete(true)}
              >
                <Trash2 className="size-3.5" />
                Delete
              </Button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
