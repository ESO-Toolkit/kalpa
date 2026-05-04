import { createTheme } from "@uiw/codemirror-themes";
import { tags as t } from "@lezer/highlight";

export const kalpaTheme = createTheme({
  theme: "dark",
  settings: {
    background: "rgba(10, 12, 18, 0.6)",
    foreground: "#e2e8f0",
    caret: "#38bdf8",
    selection: "rgba(56, 189, 248, 0.15)",
    selectionMatch: "rgba(56, 189, 248, 0.08)",
    lineHighlight: "rgba(196, 164, 74, 0.08)",
    gutterBackground: "transparent",
    gutterForeground: "rgba(255, 255, 255, 0.25)",
    gutterBorder: "transparent",
  },
  styles: [
    { tag: [t.keyword, t.operatorKeyword], color: "#c4a44a" },
    { tag: [t.string, t.special(t.string)], color: "#34d399" },
    { tag: t.number, color: "#fbbf24" },
    { tag: t.bool, color: "#c4a44a" },
    { tag: [t.variableName, t.self], color: "#38bdf8" },
    { tag: [t.propertyName], color: "#e2e8f0" },
    { tag: [t.function(t.variableName)], color: "#818cf8" },
    {
      tag: [t.comment, t.lineComment, t.blockComment],
      color: "rgba(255, 255, 255, 0.3)",
      fontStyle: "italic",
    },
    { tag: [t.operator, t.punctuation], color: "rgba(255, 255, 255, 0.5)" },
    { tag: [t.bracket], color: "rgba(255, 255, 255, 0.4)" },
    { tag: [t.tagName], color: "#c4a44a" },
    { tag: [t.attributeName], color: "#38bdf8" },
    { tag: [t.attributeValue], color: "#34d399" },
    { tag: t.null, color: "#f87171" },
  ],
});
