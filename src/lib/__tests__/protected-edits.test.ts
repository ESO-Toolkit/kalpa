import { describe, expect, it } from "vitest";
import {
  countUpdatesWithoutProtectedEditsBaseline,
  shouldPublishProtectedEditsCoverage,
} from "@/lib/protected-edits";
import type { AddonManifest, UpdateCheckResult } from "@/types";

const update = (folderName: string) => ({ folderName }) as UpdateCheckResult;
const addon = (folderName: string, hasProtectedEditsBaseline?: boolean) =>
  ({ folderName, hasProtectedEditsBaseline }) as AddonManifest;

describe("Protected Edits update disclosure", () => {
  it("counts missing and unknown baselines as unavailable", () => {
    expect(
      countUpdatesWithoutProtectedEditsBaseline(
        [update("Missing"), update("Unknown"), update("Protected")],
        [addon("Missing", false), addon("Unknown"), addon("Protected", true)]
      )
    ).toBe(2);
  });

  it("fails closed when an update has no matching scan result", () => {
    expect(countUpdatesWithoutProtectedEditsBaseline([update("NotScanned")], [])).toBe(1);
  });

  it("publishes only the current instance when coverage resolves in reverse", () => {
    const published: string[] = [];
    const settle = (generation: number, path: string) => {
      if (shouldPublishProtectedEditsCoverage(generation, 2, path === "B:/AddOns")) {
        published.push(path);
      }
    };

    settle(2, "B:/AddOns");
    settle(1, "A:/AddOns");

    expect(published).toEqual(["B:/AddOns"]);
  });
});
