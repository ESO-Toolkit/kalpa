import { describe, expect, it } from "vitest";

import { visibilityCaption } from "../upload-options";

describe("visibilityCaption", () => {
  it("says the direct path applies visibility immediately (regardless of CLI presence)", () => {
    const immediate = "Direct upload applies this visibility immediately.";
    expect(visibilityCaption(true, true)).toBe(immediate);
    expect(visibilityCaption(true, false)).toBe(immediate);
  });

  it("says the CLI forwards it (no confirm step) when the official uploader is installed", () => {
    expect(visibilityCaption(false, true)).toBe(
      "Applied when the official uploader runs — pick it here."
    );
  });

  it("keeps the confirm-in-uploader copy for the GUI fallback", () => {
    expect(visibilityCaption(false, false)).toBe(
      "You'll confirm visibility in the official ESO Logs uploader before the report goes live."
    );
  });
});
