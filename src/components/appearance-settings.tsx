import { useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import { Check, Plus, ClipboardPaste, Pencil, CopyPlus, Bug, MessageCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import { invokeResult } from "@/lib/tauri";
import { ambientAnimationsEnabled, setAmbientAnimations } from "@/lib/ambient-animations";
import { FEATURES, type FeatureId } from "@/lib/features";
import {
  TEXT_ZOOM_CHANGE_EVENT,
  TEXT_ZOOM_STOPS,
  setTextZoom,
  textZoomFactor,
} from "@/lib/text-zoom";
import { ThemeSwatch } from "@/components/ui/theme-swatch";
import { ThemeEditor } from "@/components/theme-editor";
import { useTheme } from "@/lib/use-theme";
import { newCustomThemeId, isBuiltin } from "@/lib/theme-manager";
import { CATEGORY_ORDER } from "@/lib/theme-presets";
import { FEEDBACK_DISCORD_URL, FEEDBACK_ISSUES_URL } from "@/lib/feedback";
import { THEME_COLOR_KEYS } from "@/lib/theme-types";
import { isHexColor, normalizeHex } from "@/lib/theme-color";
import { modKeyLabel } from "@/lib/platform";
import { Kbd } from "@/components/ui/kbd";
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

export function AppearanceSettings({
  onShowShortcuts,
  toolbarHidden,
  onToolbarHiddenChange,
}: {
  onShowShortcuts: () => void;
  toolbarHidden: FeatureId[];
  onToolbarHiddenChange: (next: FeatureId[]) => void;
}) {
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
  // The root class is the synchronous source of truth (hydrated at startup),
  // so no load effect is needed.
  const [ambientOn, setAmbientOn] = useState(ambientAnimationsEnabled);
  const [textZoom, setTextZoomState] = useState(textZoomFactor);
  // Guards unpinning the log uploader while a live session is streaming — the
  // check is async (a Tauri round trip), so the row disables itself while in
  // flight to avoid a second click racing the first.
  const [checkingLogUpload, setCheckingLogUpload] = useState(false);

  const pinnableFeatures = useMemo(() => FEATURES.filter((f) => f.pinnableToToolbar), []);

  const handleToolbarPinChange = async (id: FeatureId, pinned: boolean) => {
    if (!pinned && id === "log-upload") {
      setCheckingLogUpload(true);
      try {
        const liveActive = await invokeResult<boolean>("uploader_live_active");
        if (liveActive.ok && liveActive.data) {
          toast.error("A live log upload is running.", {
            description:
              "Stop live logging (or let the session finish) before moving the uploader out of the toolbar.",
          });
          return;
        }
      } finally {
        setCheckingLogUpload(false);
      }
    }
    const next = pinned
      ? toolbarHidden.filter((hiddenId) => hiddenId !== id)
      : [...toolbarHidden, id];
    onToolbarHiddenChange(next);
  };

  useEffect(() => {
    const syncTextZoom = () => setTextZoomState(textZoomFactor());
    window.addEventListener(TEXT_ZOOM_CHANGE_EVENT, syncTextZoom);
    return () => window.removeEventListener(TEXT_ZOOM_CHANGE_EVENT, syncTextZoom);
  }, []);

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
      const ok = await upsertCustomTheme(theme);
      setActiveTheme(theme.id);
      if (ok) toast.success(`Imported "${theme.name}".`);
      else toast.error(`Imported "${theme.name}", but saving it didn't persist.`);
    } catch {
      toast.error("Could not read the clipboard.");
    }
  };

  if (mode.view === "editor") {
    return (
      <ThemeEditor
        draft={mode.draft}
        isNew={mode.isNew}
        onSave={async (theme) => {
          const ok = await upsertCustomTheme(theme);
          setActiveTheme(theme.id);
          setMode({ view: "gallery" });
          if (ok) toast.success(`Saved "${theme.name}".`);
          else toast.error(`Couldn't save "${theme.name}" — changes may be lost on restart.`);
        }}
        onDelete={(id) => deleteCustomTheme(id)}
        onClose={() => setMode({ view: "gallery" })}
      />
    );
  }

  return (
    <div className="space-y-4">
      {/* Text size */}
      <section className="space-y-2">
        <SectionHeader>Text Size</SectionHeader>
        <GlassPanel variant="subtle" className="space-y-3 p-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <p className="text-sm font-medium text-foreground">Interface scale</p>
              <p className="text-xs text-muted-foreground">
                Enlarges all Kalpa text and controls together.
              </p>
            </div>
            <div className="grid min-w-[248px] grid-cols-4 rounded-lg border border-structure-06 bg-structure-03 p-1">
              {TEXT_ZOOM_STOPS.map((factor) => {
                const active = Math.abs(textZoom - factor) < 0.001;
                return (
                  <button
                    key={factor}
                    type="button"
                    aria-pressed={active}
                    onClick={() => {
                      setTextZoomState(factor);
                      setTextZoom(factor);
                    }}
                    className={`rounded-md px-2 py-1.5 text-xs font-semibold transition-colors duration-150 ${
                      active
                        ? "bg-primary text-primary-foreground shadow-[0_1px_6px_var(--primary-glow)]"
                        : "text-muted-foreground hover:bg-structure-04 hover:text-foreground"
                    }`}
                  >
                    {Math.round(factor * 100)}%
                  </button>
                );
              })}
            </div>
          </div>
          <p className="text-xs leading-relaxed text-foreground">
            Press <Kbd>{modKeyLabel()}</Kbd> with <Kbd>+</Kbd> or <Kbd>{"\u2212"}</Kbd> to change
            this anywhere in Kalpa, or <Kbd>{modKeyLabel()}</Kbd> with <Kbd>0</Kbd> to reset it.{" "}
            <button
              type="button"
              onClick={onShowShortcuts}
              className="font-medium text-accent-sky underline underline-offset-2 transition-colors duration-150 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-sky"
            >
              See all keyboard shortcuts
            </button>
          </p>
          {textZoom >= 1.5 && (
            <p className="rounded-lg border border-amber-400/20 bg-amber-400/[0.05] px-3 py-2 text-xs text-amber-200">
              At 150%, Kalpa widens its minimum window to 1200 x 750 so the addon list stays usable.
            </p>
          )}
        </GlassPanel>
      </section>

      {/* Effects */}
      <section className="space-y-2">
        <SectionHeader>Effects</SectionHeader>
        <GlassPanel variant="subtle" className="p-3">
          <label className="flex items-center gap-3 cursor-pointer">
            <Checkbox
              checked={ambientOn}
              onCheckedChange={(checked) => {
                const value = checked === true;
                setAmbientOn(value);
                setAmbientAnimations(value);
              }}
            />
            <div>
              <p className="text-sm font-medium text-foreground">Ambient animations</p>
              <p className="text-xs text-muted-foreground">
                Background orb drift and dialog shimmer. Turn off for the lowest possible CPU and
                GPU use while playing — the decorative layer goes fully static. Either way,
                animations already pause automatically whenever Kalpa isn&apos;t the focused window.
              </p>
            </div>
          </label>
        </GlassPanel>
      </section>

      {/* Toolbar */}
      <section className="space-y-2">
        <SectionHeader>Toolbar</SectionHeader>
        <GlassPanel variant="subtle" className="p-3 space-y-3">
          <p className="text-xs text-muted-foreground">
            Choose which of these appear as buttons in the header. Hiding one doesn&apos;t disable
            it — it stays listed under Settings › Tools, and its deep links keep working exactly as
            before.
          </p>
          <div className="space-y-2">
            {pinnableFeatures.map((feature, index) => {
              const Icon = feature.icon;
              const hidden = toolbarHidden.includes(feature.id);
              const rowBusy = feature.id === "log-upload" && checkingLogUpload;
              return (
                <label
                  key={feature.id}
                  className={`flex items-center gap-3 ${
                    index > 0 ? "border-t border-structure-06 pt-2" : ""
                  } ${rowBusy ? "cursor-wait opacity-70" : "cursor-pointer"}`}
                >
                  <Checkbox
                    checked={!hidden}
                    disabled={rowBusy}
                    onCheckedChange={(checked) => {
                      void handleToolbarPinChange(feature.id, checked === true);
                    }}
                  />
                  <Icon className="size-4 shrink-0 text-muted-foreground" />
                  <div className="flex-1">
                    <div className="flex items-center gap-2">
                      <p className="text-sm font-medium text-foreground">{feature.label}</p>
                      {rowBusy && (
                        <span
                          className="size-3 shrink-0 animate-spin rounded-full border-2 border-structure-10 border-t-primary"
                          aria-hidden
                        />
                      )}
                      {!rowBusy && hidden && <InfoPill color="muted">In Settings › Tools</InfoPill>}
                    </div>
                    <p className="text-xs text-muted-foreground">{feature.description}</p>
                  </div>
                </label>
              );
            })}
          </div>
        </GlassPanel>
      </section>

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
          {category === "Accessibility" && (
            <div className="rounded-lg border border-structure-06 bg-structure-02 p-2">
              <p className="text-xs text-foreground">If a theme is hard to see or use:</p>
              <div className="mt-2 flex flex-wrap gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="xs"
                  onClick={() => void openUrl(FEEDBACK_ISSUES_URL)}
                >
                  <Bug className="size-3" />
                  GitHub Issues
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  size="xs"
                  onClick={() => void openUrl(FEEDBACK_DISCORD_URL)}
                >
                  <MessageCircle className="size-3" />
                  Discord
                </Button>
              </div>
            </div>
          )}
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
          : "border-structure-06 bg-structure-02 hover:-translate-y-px hover:border-structure-12 hover:bg-structure-04"
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
          <span className="truncate text-xs font-medium text-foreground">{theme.name}</span>
          {active && <Check className="ml-auto size-3.5 shrink-0 text-primary" />}
        </div>
        <p className="line-clamp-1 px-1 pb-0.5 text-[11px] text-muted-foreground">
          {theme.description}
        </p>
      </button>

      {/* Hover/focus actions (keyboard-accessible via group-focus-within) */}
      <div className="absolute right-1.5 top-1.5 flex gap-1 opacity-0 transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100">
        {onEdit && (
          <button
            type="button"
            onClick={onEdit}
            className="flex size-6 items-center justify-center rounded-md border border-structure-10 bg-scrim-40 text-foreground backdrop-blur-sm transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-sky/60 focus-visible:text-foreground"
            aria-label={`Edit ${theme.name} theme`}
          >
            <Pencil className="size-3" />
          </button>
        )}
        <button
          type="button"
          onClick={onFork}
          className="flex size-6 items-center justify-center rounded-md border border-structure-10 bg-scrim-40 text-foreground backdrop-blur-sm transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-sky/60 focus-visible:text-foreground"
          aria-label={`Duplicate ${theme.name} as a custom theme`}
        >
          <CopyPlus className="size-3" />
        </button>
      </div>
    </div>
  );
}
