import { describe, it, expect } from "vitest";
import {
  FEATURES,
  DIALOG_LABELS,
  findFeature,
  visibleToolbar,
  toolsMenuFeatures,
  sanitizeHiddenIds,
  type FeatureId,
  type FeatureDef,
} from "../features";

/**
 * A context in which every conditional feature is on.
 *
 * The toolbar assertions below are about `hidden` and registry order, not about
 * `pinnedWhen`, so they need a context that does not silently drop a pinnable
 * feature out from under them. `pinnedWhen`'s own behaviour is asserted
 * separately, with a context that turns it off.
 */
const CTX = { minionDetected: true, graphicsStackDetected: true };

// Exhaustiveness map: adding an id to the `FeatureId` union without adding a
// matching `FEATURES` entry fails to type-check here.
const ID_EXHAUSTIVENESS_MAP: Record<FeatureId, true> = {
  packs: true,
  profiles: true,
  "saved-variables": true,
  "log-upload": true,
  backups: true,
  characters: true,
  "api-compat": true,
  "safety-center": true,
  "client-health": true,
  "migration-wizard": true,
  shortcuts: true,
};

describe("FEATURES", () => {
  it("has unique ids", () => {
    const ids = FEATURES.map((f) => f.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("has exactly one entry per FeatureId in the union", () => {
    const expectedIds = Object.keys(ID_EXHAUSTIVENESS_MAP) as FeatureId[];
    const actualIds = FEATURES.map((f) => f.id);
    expect(new Set(actualIds)).toEqual(new Set(expectedIds));
    expect(actualIds.length).toBe(expectedIds.length);
  });

  it("gives every entry a non-empty label and description", () => {
    for (const f of FEATURES) {
      expect(f.label.trim().length, `${f.id} label`).toBeGreaterThan(0);
      expect(f.description.trim().length, `${f.id} description`).toBeGreaterThan(0);
    }
  });

  it("gives every pinnable entry an icon", () => {
    for (const f of FEATURES) {
      if (f.pinnableToToolbar) {
        expect(f.icon, `${f.id} icon`).toBeTruthy();
      }
    }
  });
});

describe("DIALOG_LABELS", () => {
  it("has a non-empty entry for every DialogId, including settings and support", () => {
    const expectedIds = [...FEATURES.map((f) => f.id), "settings", "support"];
    for (const id of expectedIds) {
      const label = DIALOG_LABELS[id as keyof typeof DIALOG_LABELS];
      expect(label, `label for ${id}`).toBeTruthy();
      expect(label.trim().length, `label for ${id} non-empty`).toBeGreaterThan(0);
    }
  });

  it("pins the backups dialogTitle override distinct from its Tools-row label", () => {
    expect(DIALOG_LABELS.backups).toBe("Backups");
    expect(findFeature("backups")?.label).toBe("Backup & Restore");
  });

  it("pins the migration-wizard dialogTitle override", () => {
    expect(DIALOG_LABELS["migration-wizard"]).toBe("Migration");
  });
});

describe("findFeature", () => {
  it("returns the matching entry", () => {
    expect(findFeature("packs")?.label).toBe("Pack Hub");
  });

  it("returns undefined for an id not present in the registry", () => {
    expect(findFeature("does-not-exist" as FeatureId)).toBeUndefined();
  });
});

describe("visibleToolbar", () => {
  it("returns exactly the pinnable entries, in registry order, when nothing is hidden", () => {
    const result = visibleToolbar(FEATURES, [], CTX);
    expect(result.map((f) => f.id)).toEqual([
      "packs",
      "profiles",
      "saved-variables",
      "log-upload",
      "client-health",
    ]);
  });

  it("removes exactly the hidden id", () => {
    const result = visibleToolbar(FEATURES, ["profiles"], CTX);
    expect(result.map((f) => f.id)).toEqual([
      "packs",
      "saved-variables",
      "log-upload",
      "client-health",
    ]);
  });

  it("ignores unknown ids in hidden rather than throwing", () => {
    expect(() => visibleToolbar(FEATURES, ["not-a-real-id" as FeatureId], CTX)).not.toThrow();
    const result = visibleToolbar(FEATURES, ["not-a-real-id" as FeatureId], CTX);
    expect(result.map((f) => f.id)).toEqual([
      "packs",
      "profiles",
      "saved-variables",
      "log-upload",
      "client-health",
    ]);
  });

  it("is a no-op when hiding a non-pinnable id", () => {
    const result = visibleToolbar(FEATURES, ["backups"], CTX);
    expect(result.map((f) => f.id)).toEqual([
      "packs",
      "profiles",
      "saved-variables",
      "log-upload",
      "client-health",
    ]);
  });

  it("is pure: repeated calls with the same inputs deep-equal each other", () => {
    const a = visibleToolbar(FEATURES, ["profiles"], CTX);
    const b = visibleToolbar(FEATURES, ["profiles"], CTX);
    expect(a).toEqual(b);
  });

  it("does not mutate the features or hidden arguments", () => {
    const features = FEATURES.map((f) => ({ ...f }));
    const featuresSnapshot = JSON.stringify(features.map((f) => f.id));
    const hidden: FeatureId[] = ["profiles"];
    const hiddenSnapshot = [...hidden];

    visibleToolbar(features as FeatureDef[], hidden, CTX);

    expect(JSON.stringify(features.map((f) => f.id))).toBe(featuresSnapshot);
    expect(hidden).toEqual(hiddenSnapshot);
  });

  it("never touches module state (calling it twice with different inputs doesn't cross-contaminate)", () => {
    const first = visibleToolbar(FEATURES, ["packs"], CTX);
    const second = visibleToolbar(FEATURES, [], CTX);
    expect(first.map((f) => f.id)).toEqual([
      "profiles",
      "saved-variables",
      "log-upload",
      "client-health",
    ]);
    expect(second.map((f) => f.id)).toEqual([
      "packs",
      "profiles",
      "saved-variables",
      "log-upload",
      "client-health",
    ]);
  });
});

describe("pinnedWhen", () => {
  const NO_STACK = { minionDetected: false, graphicsStackDetected: false };
  const STACK = { minionDetected: false, graphicsStackDetected: true };

  it("keeps a conditional feature out of the toolbar until it earns the slot", () => {
    expect(visibleToolbar(FEATURES, [], NO_STACK).map((f) => f.id)).not.toContain("client-health");
    expect(visibleToolbar(FEATURES, [], STACK).map((f) => f.id)).toContain("client-health");
  });

  /**
   * The safety property. Falling out of the toolbar must never make a feature
   * unreachable — an unpinned pinnable belongs to the Settings > Tools catalog,
   * and for the graphics stack that is the ONLY surface a stock-client user
   * would ever see it on.
   */
  it("moves it to the Tools catalog rather than out of the app", () => {
    expect(toolsMenuFeatures(FEATURES, [], NO_STACK).map((f) => f.id)).toContain("client-health");
  });

  it("still honours an explicit unpin when the condition holds", () => {
    const ids = visibleToolbar(FEATURES, ["client-health"], STACK).map((f) => f.id);
    expect(ids).not.toContain("client-health");
    expect(toolsMenuFeatures(FEATURES, ["client-health"], STACK).map((f) => f.id)).toContain(
      "client-health"
    );
  });

  it("leaves features without the predicate pinned in every context", () => {
    for (const ctx of [NO_STACK, STACK]) {
      expect(visibleToolbar(FEATURES, [], ctx).map((f) => f.id)).toContain("packs");
    }
  });

  /**
   * `pinnedWhen` is about the toolbar budget; `visibleWhen` is about whether a
   * feature is offered at all. Conflating them would hide the panel from anyone
   * whose stack Kalpa merely failed to detect.
   */
  it("is not a visibility gate", () => {
    const feature = findFeature("client-health");
    expect(feature?.pinnedWhen).toBeTypeOf("function");
    expect(feature?.visibleWhen).toBeUndefined();
  });
});

describe("toolsMenuFeatures", () => {
  it("excludes ids that are currently pinned to the toolbar", () => {
    const result = toolsMenuFeatures(FEATURES, [], {
      minionDetected: false,
      graphicsStackDetected: true,
    });
    expect(result.map((f) => f.id)).not.toContain("packs");
    expect(result.map((f) => f.id)).not.toContain("profiles");
    expect(result.map((f) => f.id)).not.toContain("saved-variables");
    expect(result.map((f) => f.id)).not.toContain("log-upload");
    expect(result.map((f) => f.id)).toContain("backups");
    expect(result.map((f) => f.id)).toContain("characters");
  });

  it("re-surfaces a hidden pinnable id in the tools menu (it is no longer pinned)", () => {
    const result = toolsMenuFeatures(FEATURES, ["packs"], {
      minionDetected: false,
      graphicsStackDetected: true,
    });
    expect(result.map((f) => f.id)).toContain("packs");
    expect(result.map((f) => f.id)).not.toContain("profiles");
  });

  it("excludes migration-wizard when minion is not detected", () => {
    const result = toolsMenuFeatures(FEATURES, [], {
      minionDetected: false,
      graphicsStackDetected: true,
    });
    expect(result.map((f) => f.id)).not.toContain("migration-wizard");
  });

  it("includes migration-wizard when minion is detected", () => {
    const result = toolsMenuFeatures(FEATURES, [], {
      minionDetected: true,
      graphicsStackDetected: true,
    });
    expect(result.map((f) => f.id)).toContain("migration-wizard");
  });
});

describe("sanitizeHiddenIds", () => {
  it("round-trips a valid array of known ids unchanged", () => {
    expect(sanitizeHiddenIds(["packs", "profiles"])).toEqual(["packs", "profiles"]);
  });

  it("drops unrecognised id strings (a newer build's ids)", () => {
    expect(sanitizeHiddenIds(["packs", "some-future-feature"])).toEqual(["packs"]);
  });

  it("collapses duplicates while preserving first-seen order", () => {
    expect(sanitizeHiddenIds(["packs", "profiles", "packs"])).toEqual(["packs", "profiles"]);
  });

  it.each([
    ["numbers", [1, 2]],
    ["null entries", [null]],
    ["plain objects", [{ id: "packs" }]],
    ["nested arrays", [["packs"]]],
    ["booleans", [true, false]],
  ])("drops non-string entries: %s", (_label, input) => {
    expect(sanitizeHiddenIds(input)).toEqual([]);
  });

  it("mixes valid and invalid entries, keeping only the valid known ids", () => {
    expect(sanitizeHiddenIds(["packs", 42, null, "not-real", "profiles", {}])).toEqual([
      "packs",
      "profiles",
    ]);
  });

  it.each([
    ["null", null],
    ["undefined", undefined],
    ["a string", "packs"],
    ["an object", { packs: true }],
    ["a number", 5],
  ])("returns [] for non-array input: %s", (_label, input) => {
    expect(sanitizeHiddenIds(input)).toEqual([]);
  });

  it("contains only real FeatureIds", () => {
    const knownIds = new Set(FEATURES.map((f) => f.id));
    const result = sanitizeHiddenIds(["packs", "bogus", "profiles"]);
    for (const id of result) {
      expect(knownIds.has(id)).toBe(true);
    }
  });

  it("does not mutate its input array", () => {
    const input = ["packs", "bogus", "packs"];
    const snapshot = [...input];
    sanitizeHiddenIds(input);
    expect(input).toEqual(snapshot);
  });
});
