import { describe, expect, it } from "vitest";

import { sourceLocation, tidyLogLabel } from "../history-panel";

describe("tidyLogLabel", () => {
  it("keeps a readable date + session for a dated archive with a session number", () => {
    expect(tidyLogLabel("Archive-2025-06-20__03_46_03-Encounter-session02-1750394097660.log")).toBe(
      "Archive 2025-06-20 · session 2"
    );
  });

  it("falls back to a bare session label when a session archive has no date", () => {
    expect(tidyLogLabel("Archive-Encounter-session05-123.log")).toBe("Session 5");
  });

  it("renders an ISO-stamped archive as a short date", () => {
    // Format is locale-dependent; mirror the impl's Date formatting so the assertion
    // is locale-agnostic while still proving the ISO date parsed to 2026-06-14.
    const expected = `Archive · ${new Date(2026, 5, 14).toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
    })}`;
    expect(tidyLogLabel("Archive-20260614T190354Z-Encounter.log")).toBe(expected);
  });

  it("drops a trailing epoch-ms id from an otherwise plain name", () => {
    expect(tidyLogLabel("Encounter-1750394097660.log")).toBe("Encounter");
  });

  it("leaves ordinary and user-named split names intact", () => {
    expect(tidyLogLabel("Lucent Citadel — Jun 18.log")).toBe("Lucent Citadel — Jun 18");
    expect(tidyLogLabel("lucent-citadel-jun18.log")).toBe("lucent-citadel-jun18");
  });
});

describe("sourceLocation", () => {
  it("splits a Windows path into its parent folder and full directory", () => {
    expect(sourceLocation("C:\\Games\\ESO\\Logs\\Encounter.log")).toEqual({
      folder: "Logs",
      dir: "C:\\Games\\ESO\\Logs",
    });
  });

  it("splits a POSIX path into its parent folder and full directory", () => {
    expect(sourceLocation("/home/user/Documents/ESO/Logs/Encounter.log")).toEqual({
      folder: "Logs",
      dir: "/home/user/Documents/ESO/Logs",
    });
  });

  it("falls back to the raw value when the path has no separator", () => {
    expect(sourceLocation("Encounter.log")).toEqual({
      folder: "Encounter.log",
      dir: "Encounter.log",
    });
  });
});
