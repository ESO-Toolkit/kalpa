// Shared constants for Pack Hub components

export type PackTypeFilter = "all" | "addon-pack" | "build-pack" | "roster-pack";
export type SortOption = "votes" | "newest" | "updated";
export type TabMode = "browse" | "create" | "my-packs";
export type ShareMode = "private-link" | "export-file";
export type ImportMode = "enter-code" | "import-file";
export type MyPacksSubTab = "created" | "installed";

export const TYPE_LABELS: Record<string, string> = {
  "addon-pack": "Addon Pack",
  "build-pack": "Build Pack",
  "roster-pack": "Roster Pack",
};

export const TAG_COLORS: Record<
  string,
  "gold" | "sky" | "emerald" | "amber" | "red" | "violet" | "muted"
> = {
  essential: "gold",
  trial: "red",
  pve: "emerald",
  pvp: "red",
  healer: "sky",
  dps: "amber",
  tank: "violet",
  beginner: "emerald",
  utility: "muted",
};

export const PACK_TYPE_ACCENT: Record<
  string,
  { border: string; bg: string; hoverBg: string; text: string; hoverGlow: string }
> = {
  "addon-pack": {
    border: "border-l-[#c4a44a]/70",
    bg: "bg-[#c4a44a]/[0.03]",
    hoverBg: "hover:bg-[#c4a44a]/[0.08]",
    text: "text-[#c4a44a]",
    hoverGlow:
      "hover:shadow-[0_6px_24px_rgba(196,164,74,0.1),inset_0_1px_0_rgba(255,255,255,0.06)]",
  },
  "build-pack": {
    border: "border-l-sky-400/70",
    bg: "bg-sky-400/[0.03]",
    hoverBg: "hover:bg-sky-400/[0.08]",
    text: "text-sky-400",
    hoverGlow:
      "hover:shadow-[0_6px_24px_rgba(56,189,248,0.1),inset_0_1px_0_rgba(255,255,255,0.06)]",
  },
  "roster-pack": {
    border: "border-l-violet-400/70",
    bg: "bg-violet-400/[0.03]",
    hoverBg: "hover:bg-violet-400/[0.08]",
    text: "text-violet-400",
    hoverGlow:
      "hover:shadow-[0_6px_24px_rgba(167,139,250,0.1),inset_0_1px_0_rgba(255,255,255,0.06)]",
  },
};

export const PACK_TYPE_PILL_COLOR: Record<string, "gold" | "sky" | "violet" | "muted"> = {
  "addon-pack": "gold",
  "build-pack": "sky",
  "roster-pack": "violet",
};

export const PRESET_TAGS = [
  "trial",
  "pvp",
  "beginner",
  "healer",
  "tank",
  "dps",
  "utility",
] as const;

export const PACK_TYPE_DESCRIPTIONS: Record<string, string> = {
  "addon-pack": "A collection of addons",
  "build-pack": "A skill build or loadout",
  "roster-pack": "A group or raid roster",
};
