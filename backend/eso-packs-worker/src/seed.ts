import type { Pack } from "./types";

/**
 * Sample packs with real ESOUI addon IDs.
 * Used by POST /admin/seed in dev only.
 */
export const SEED_PACKS: Pack[] = [
  {
    id: "trial-essentials",
    title: "Trial Essentials",
    description:
      "The core addons every trial group expects you to have. Covers mechanic alerts, DPS/ult sharing, and combat logging.",
    pack_type: "addon-pack",
    author_id: "system",
    author_name: "Kalpa",
    is_anonymous: false,
    tags: ["trial", "pve", "essential"],
    vote_count: 0,
    install_count: 0,
    status: "published",
    created_at: "2026-03-28T00:00:00.000Z",
    updated_at: "2026-03-28T00:00:00.000Z",
    addons: [
      { esouiId: 1573, name: "RaidNotifier", required: true },
      { esouiId: 2629, name: "Combat Metrics", required: true },
      { esouiId: 2698, name: "Hodor Reflexes", required: true },
      { esouiId: 1891, name: "Code's Combat Alerts", required: false, note: "Optional alternative to RaidNotifier" },
    ],
  },
  {
    id: "healer-toolkit",
    title: "Healer Toolkit",
    description:
      "Everything a healer needs: group frames, buff tracking, resource management, and mechanic timers.",
    pack_type: "addon-pack",
    author_id: "system",
    author_name: "Kalpa",
    is_anonymous: false,
    tags: ["healer", "pve", "support"],
    vote_count: 0,
    install_count: 0,
    status: "published",
    created_at: "2026-03-28T00:00:00.000Z",
    updated_at: "2026-03-28T00:00:00.000Z",
    addons: [
      { esouiId: 1573, name: "RaidNotifier", required: true },
      { esouiId: 2629, name: "Combat Metrics", required: true },
      { esouiId: 2698, name: "Hodor Reflexes", required: true },
      { esouiId: 1898, name: "Srendarr", required: false, note: "Buff/debuff tracking" },
      { esouiId: 2244, name: "Untaunted", required: false, note: "Group frame display" },
    ],
  },
  {
    id: "dps-starter",
    title: "DPS Starter Pack",
    description:
      "Get parsing-ready with combat logging, weave tracking, and essential raid callouts for damage dealers.",
    pack_type: "addon-pack",
    author_id: "system",
    author_name: "Kalpa",
    is_anonymous: false,
    tags: ["dps", "pve", "combat"],
    vote_count: 0,
    install_count: 0,
    status: "published",
    created_at: "2026-03-28T00:00:00.000Z",
    updated_at: "2026-03-28T00:00:00.000Z",
    addons: [
      { esouiId: 2629, name: "Combat Metrics", required: true },
      { esouiId: 1573, name: "RaidNotifier", required: true },
      { esouiId: 2698, name: "Hodor Reflexes", required: true },
      { esouiId: 1898, name: "Srendarr", required: false, note: "Buff tracking for DoT uptime" },
      { esouiId: 1891, name: "Code's Combat Alerts", required: false, note: "Extra combat warnings" },
    ],
  },
];
