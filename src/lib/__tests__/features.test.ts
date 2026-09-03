import { describe, it, expect } from "vitest";
import {
  FEATURES,
  DIALOG_LABELS,
  findFeature,
  visibleToolbar,
  toolsMenuFeatures,
  type FeatureId,
  type FeatureDef,
} from "../features";

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
    const result = visibleToolbar(FEATURES, []);
    expect(result.map((f) => f.id)).toEqual(["packs", "profiles", "saved-variables", "log-upload"]);
  });

  it("removes exactly the hidden id", () => {
    const result = visibleToolbar(FEATURES, ["profiles"]);
    expect(result.map((f) => f.id)).toEqual(["packs", "saved-variables", "log-upload"]);
  });

  it("ignores unknown ids in hidden rather than throwing", () => {
    expect(() => visibleToolbar(FEATURES, ["not-a-real-id" as FeatureId])).not.toThrow();
    const result = visibleToolbar(FEATURES, ["not-a-real-id" as FeatureId]);
    expect(result.map((f) => f.id)).toEqual(["packs", "profiles", "saved-variables", "log-upload"]);
  });

  it("is a no-op when hiding a non-pinnable id", () => {
    const result = visibleToolbar(FEATURES, ["backups"]);
    expect(result.map((f) => f.id)).toEqual(["packs", "profiles", "saved-variables", "log-upload"]);
  });

  it("is pure: repeated calls with the same inputs deep-equal each other", () => {
    const a = visibleToolbar(FEATURES, ["profiles"]);
    const b = visibleToolbar(FEATURES, ["profiles"]);
    expect(a).toEqual(b);
  });

  it("does not mutate the features or hidden arguments", () => {
    const features = FEATURES.map((f) => ({ ...f }));
    const featuresSnapshot = JSON.stringify(features.map((f) => f.id));
    const hidden: FeatureId[] = ["profiles"];
    const hiddenSnapshot = [...hidden];

    visibleToolbar(features as FeatureDef[], hidden);

    expect(JSON.stringify(features.map((f) => f.id))).toBe(featuresSnapshot);
    expect(hidden).toEqual(hiddenSnapshot);
  });

  it("never touches module state (calling it twice with different inputs doesn't cross-contaminate)", () => {
    const first = visibleToolbar(FEATURES, ["packs"]);
    const second = visibleToolbar(FEATURES, []);
    expect(first.map((f) => f.id)).toEqual(["profiles", "saved-variables", "log-upload"]);
    expect(second.map((f) => f.id)).toEqual(["packs", "profiles", "saved-variables", "log-upload"]);
  });
});

describe("toolsMenuFeatures", () => {
  it("excludes ids that are currently pinned to the toolbar", () => {
    const result = toolsMenuFeatures(FEATURES, [], { minionDetected: false });
    expect(result.map((f) => f.id)).not.toContain("packs");
    expect(result.map((f) => f.id)).not.toContain("profiles");
    expect(result.map((f) => f.id)).not.toContain("saved-variables");
    expect(result.map((f) => f.id)).not.toContain("log-upload");
    expect(result.map((f) => f.id)).toContain("backups");
    expect(result.map((f) => f.id)).toContain("characters");
  });

  it("re-surfaces a hidden pinnable id in the tools menu (it is no longer pinned)", () => {
    const result = toolsMenuFeatures(FEATURES, ["packs"], { minionDetected: false });
    expect(result.map((f) => f.id)).toContain("packs");
    expect(result.map((f) => f.id)).not.toContain("profiles");
  });

  it("excludes migration-wizard when minion is not detected", () => {
    const result = toolsMenuFeatures(FEATURES, [], { minionDetected: false });
    expect(result.map((f) => f.id)).not.toContain("migration-wizard");
  });

  it("includes migration-wizard when minion is detected", () => {
    const result = toolsMenuFeatures(FEATURES, [], { minionDetected: true });
    expect(result.map((f) => f.id)).toContain("migration-wizard");
  });
});
