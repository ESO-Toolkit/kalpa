import { useMemo, useState } from "react";
import { toast } from "sonner";
import { Check, Plus, ClipboardPaste, Pencil, CopyPlus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { SectionHeader } from "@/components/ui/section-header";
import { ThemeSwatch } from "@/components/ui/theme-swatch";
import { ThemeEditor } from "@/components/theme-editor";
import { useTheme } from "@/lib/use-theme";
import { newCustomThemeId, isBuiltin } from "@/lib/theme-manager";
import { CATEGORY_ORDER } from "@/lib/theme-presets";
import { THEME_COLOR_KEYS } from "@/lib/theme-types";
import { isHexColor, normalizeHex } from "@/lib/theme-color";
import type { Theme, ThemeColors } from "@/lib/theme-types";

type Mode = { view: "gallery" } | { view: "editor"; draft: Theme; isNew: boolean };

/** Validate + normalize a pasted theme JSON into a custom Theme, or null. */
function parseImportedTheme(raw: string): Theme | null {
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!data || typeof data !== "object") return null;
  const obj = data as Record<string, unknown>;
  const colorsIn = obj.colors as Record<string, unknown> | undefined;
  if (!colorsIn || typeof colorsIn !== "object") return null;

  const colors = {} as ThemeColors;
  for (const key of THEME_COLOR_KEYS) {
    const v = colorsIn[key];
    if (typeof v !== "string" || !isHexColor(v)) return null;
    colors[key] = normalizeHex(v);
  }
  const name = typeof obj.name === "string" && obj.name.trim() ? obj.name.trim() : "Imported theme";
  return {
    id: newCustomThemeId(),
    name,
    description: typeof obj.description === "string" ? obj.description : "Imported custom theme.",
    category: "Custom",
    colors,
    custom: true,
  };
}

export function AppearanceSettings() {
  const {
    activeThemeId,
    activeTheme,
    builtinThemes,
    customThemes,
    setActiveTheme,
    upsertCustomTheme,
    deleteCustomTheme,
  } = useTheme();
  const [mode, setMode] = useState<Mode>({ view: "gallery" });

  const grouped = useMemo(() => {
    const byCat = new Map<string, Theme[]>();
    for (const t of builtinThemes) {
      const list = byCat.get(t.category) ?? [];
      list.push(t);
      byCat.set(t.category, list);
    }
    const cats = [...byCat.keys()].sort((a, b) => {
      const ia = CATEGORY_ORDER.indexOf(a as (typeof CATEGORY_ORDER)[number]);
      const ib = CATEGORY_ORDER.indexOf(b as (typeof CATEGORY_ORDER)[number]);
      return (ia === -1 ? 99 : ia) - (ib === -1 ? 99 : ib);
    });
    return cats.map((c) => ({ category: c, themes: byCat.get(c)! }));
  }, [builtinThemes]);

  const startNew = () => {
    // Fork the current theme as a friendly starting point.
    setMode({
      view: "editor",
      isNew: true,
      draft: {
        id: newCustomThemeId(),
        name: `${activeTheme.name} Custom`,
        description: "My custom theme.",
        category: "Custom",
        colors: { ...activeTheme.colors },
        custom: true,
      },
    });
  };

  const forkTheme = (theme: Theme) => {
    setMode({
      view: "editor",
      isNew: true,
      draft: {
        id: newCustomThemeId(),
        name: `${theme.name} Copy`,
        description: theme.description,
        category: "Custom",
        colors: { ...theme.colors },
        custom: true,
      },
    });
  };

  const editTheme = (theme: Theme) => {
    setMode({ view: "editor", isNew: false, draft: theme });
  };

  const handleImport = async () => {
    try {
      const text = await navigator.clipboard.readText();
      const theme = parseImportedTheme(text);
      if (!theme) {
        toast.error("Clipboard doesn't contain a valid theme.");
        return;
      }
      upsertCustomTheme(theme);
      setActiveTheme(theme.id);
      toast.success(`Imported "${theme.name}".`);
    } catch {
      toast.error("Could not read the clipboard.");
    }
  };

  if (mode.view === "editor") {
    return (
      <ThemeEditor
        draft={mode.draft}
        isNew={mode.isNew}
        onSave={(theme) => {
          upsertCustomTheme(theme);
          setActiveTheme(theme.id);
          setMode({ view: "gallery" });
          toast.success(`Saved "${theme.name}".`);
        }}
        onDelete={(id) => deleteCustomTheme(id)}
        onClose={() => setMode({ view: "gallery" })}
      />
    );
  }

  return (
    <div className="space-y-4">
      {/* Action bar */}
      <div className="flex items-center gap-2">
        <Button size="sm" onClick={startNew}>
          <Plus className="size-3.5" />
          Create theme
        </Button>
        <Button variant="outline" size="sm" onClick={handleImport}>
          <ClipboardPaste className="size-3.5" />
          Import
        </Button>
      </div>

      {/* Custom themes */}
      {customThemes.length > 0 && (
        <section className="space-y-2">
          <SectionHeader>Your Themes</SectionHeader>
          <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-3">
            {customThemes.map((theme) => (
              <ThemeCard
                key={theme.id}
                theme={theme}
                active={activeThemeId === theme.id}
                onApply={() => setActiveTheme(theme.id)}
                onEdit={() => editTheme(theme)}
                onFork={() => forkTheme(theme)}
              />
            ))}
          </div>
        </section>
      )}

      {/* Built-in themes grouped by category */}
      {grouped.map(({ category, themes }) => (
        <section key={category} className="space-y-2">
          <SectionHeader>{category}</SectionHeader>
          <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-3">
            {themes.map((theme) => (
              <ThemeCard
                key={theme.id}
                theme={theme}
                active={activeThemeId === theme.id}
                onApply={() => setActiveTheme(theme.id)}
                onFork={() => forkTheme(theme)}
                onEdit={isBuiltin(theme.id) ? undefined : () => editTheme(theme)}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

function ThemeCard({
  theme,
  active,
  onApply,
  onEdit,
  onFork,
}: {
  theme: Theme;
  active: boolean;
  onApply: () => void;
  onEdit?: () => void;
  onFork: () => void;
}) {
  return (
    <div
      className={`group relative overflow-hidden rounded-xl border p-1.5 text-left transition-all duration-150 ${
        active
          ? "border-primary/50 bg-primary/[0.06] shadow-[0_0_0_1px_var(--primary-glow)]"
          : "border-white/[0.06] bg-white/[0.02] hover:-translate-y-px hover:border-white/[0.12] hover:bg-white/[0.04]"
      }`}
    >
      <button
        type="button"
        onClick={onApply}
        className="block w-full"
        aria-label={`Apply ${theme.name} theme`}
      >
        <ThemeSwatch colors={theme.colors} active={active} />
        <div className="flex items-center gap-1 px-1 pt-1.5">
          <span className="truncate text-xs font-medium text-white/90">{theme.name}</span>
          {active && <Check className="ml-auto size-3.5 shrink-0 text-primary" />}
        </div>
        <p className="line-clamp-1 px-1 pb-0.5 text-[10px] text-muted-foreground">
          {theme.description}
        </p>
      </button>

      {/* Hover actions */}
      <div className="absolute right-1.5 top-1.5 flex gap-1 opacity-0 transition-opacity duration-150 group-hover:opacity-100">
        {onEdit && (
          <button
            type="button"
            onClick={onEdit}
            className="flex size-6 items-center justify-center rounded-md border border-white/[0.1] bg-black/40 text-white/70 backdrop-blur-sm transition-colors hover:text-white"
            title="Edit theme"
          >
            <Pencil className="size-3" />
          </button>
        )}
        <button
          type="button"
          onClick={onFork}
          className="flex size-6 items-center justify-center rounded-md border border-white/[0.1] bg-black/40 text-white/70 backdrop-blur-sm transition-colors hover:text-white"
          title="Duplicate as custom theme"
        >
          <CopyPlus className="size-3" />
        </button>
      </div>
    </div>
  );
}
