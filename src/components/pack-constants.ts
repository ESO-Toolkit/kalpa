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
  { border: string; bg: string; hoverBg: string; text: string }
> = {
  "addon-pack": {
    border: "border-l-[#c4a44a]/60",
    bg: "bg-[#c4a44a]/[0.02]",
    hoverBg: "hover:bg-[#c4a44a]/[0.06]",
    text: "text-[#c4a44a]",
  },
  "build-pack": {
    border: "border-l-sky-400/60",
    bg: "bg-sky-400/[0.02]",
    hoverBg: "hover:bg-sky-400/[0.06]",
    text: "text-sky-400",
  },
  "roster-pack": {
    border: "border-l-violet-400/60",
    bg: "bg-violet-400/[0.02]",
    hoverBg: "hover:bg-violet-400/[0.06]",
    text: "text-violet-400",
  },
};

export const PACK_TYPE_PILL_COLOR: Record<string, "gold" | "sky" | "violet" | "muted"> = {
  "addon-pack": "gold",
  "build-pack": "sky",
  "roster-pack": "violet",
};

export const PRESET_TAGS = ["trial", "pvp", "beginner", "healer", "tank", "dps", "utility"] as const;

export const PACK_TYPE_DESCRIPTIONS: Record<string, string> = {
  "addon-pack": "A collection of addons",
  "build-pack": "A skill build or loadout",
  "roster-pack": "A group or raid roster",
};
