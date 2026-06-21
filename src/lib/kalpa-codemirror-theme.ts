import { createTheme } from "@uiw/codemirror-themes";
import { tags as t } from "@lezer/highlight";

/**
 * CodeMirror theme for the addon-file editor.
 *
 * Colors are CSS `var()` / `color-mix()` tokens, not literals, so the editor
 * follows the active app theme LIVE — createTheme emits them as plain CSS values
 * and the cascade re-resolves them when the theme switches (no rebuild needed).
 * Syntax content hues reuse the theme's tinted status tokens so highlighting
 * harmonizes with the rest of the UI; chrome (bg/caret/selection/gutter) tracks
 * the theme surface and accents.
 */
export const kalpaTheme = createTheme({
  theme: "dark",
  settings: {
    background: "color-mix(in oklab, var(--card) 55%, transparent)",
    foreground: "var(--foreground)",
    caret: "var(--accent-sky)",
    selection: "color-mix(in oklab, var(--accent-sky) 22%, transparent)",
    selectionMatch: "color-mix(in oklab, var(--accent-sky) 12%, transparent)",
    lineHighlight: "color-mix(in oklab, var(--primary) 8%, transparent)",
    gutterBackground: "transparent",
    gutterForeground: "color-mix(in oklab, var(--foreground) 35%, transparent)",
    gutterBorder: "transparent",
  },
  styles: [
    { tag: [t.keyword, t.operatorKeyword], color: "var(--primary)" },
    { tag: [t.string, t.special(t.string)], color: "var(--status-success-strong)" },
    { tag: t.number, color: "var(--status-warning-strong)" },
    { tag: t.bool, color: "var(--primary)" },
    { tag: [t.variableName, t.self], color: "var(--accent-sky)" },
    { tag: [t.propertyName], color: "var(--foreground)" },
    { tag: [t.function(t.variableName)], color: "var(--status-library)" },
    {
      tag: [t.comment, t.lineComment, t.blockComment],
      color: "color-mix(in oklab, var(--foreground) 38%, transparent)",
      fontStyle: "italic",
    },
    {
      tag: [t.operator, t.punctuation],
      color: "color-mix(in oklab, var(--foreground) 55%, transparent)",
    },
    { tag: [t.bracket], color: "color-mix(in oklab, var(--foreground) 45%, transparent)" },
    { tag: [t.tagName], color: "var(--primary)" },
    { tag: [t.attributeName], color: "var(--accent-sky)" },
    { tag: [t.attributeValue], color: "var(--status-success-strong)" },
    { tag: t.null, color: "var(--status-error-strong)" },
  ],
});
