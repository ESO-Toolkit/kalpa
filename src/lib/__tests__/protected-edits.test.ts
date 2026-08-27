import { describe, expect, it } from "vitest";
import { countUpdatesWithoutProtectedEditsBaseline } from "@/lib/protected-edits";
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
});
