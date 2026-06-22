import { describe, it, expect } from "vitest";
import { UNKNOWN_SERVER, serverTag, defaultCharacterBackupName } from "../character-backup";

describe("serverTag", () => {
  it("strips the Megaserver suffix", () => {
    expect(serverTag("NA Megaserver")).toBe("NA");
    expect(serverTag("EU Megaserver")).toBe("EU");
  });

  it("keeps non-suffixed servers verbatim", () => {
    expect(serverTag("PTS")).toBe("PTS");
  });

  it("returns null for the unknown bucket", () => {
    expect(serverTag(UNKNOWN_SERVER)).toBeNull();
    expect(serverTag("")).toBeNull();
  });
});

describe("defaultCharacterBackupName", () => {
  it("gives same-name NA/EU twins distinct default names", () => {
    const na = defaultCharacterBackupName("Bob", "NA Megaserver");
    const eu = defaultCharacterBackupName("Bob", "EU Megaserver");
    expect(na).toBe("Bob-NA-backup");
    expect(eu).toBe("Bob-EU-backup");
    expect(na).not.toBe(eu);
  });

  it("omits the tag for unknown-server characters", () => {
    expect(defaultCharacterBackupName("Bob", UNKNOWN_SERVER)).toBe("Bob-backup");
  });

  it("preserves hyphenated and spaced names", () => {
    expect(defaultCharacterBackupName("Jodynn-Jo", "NA Megaserver")).toBe("Jodynn-Jo-NA-backup");
    expect(defaultCharacterBackupName("Alt Ego", "PTS")).toBe("Alt Ego-PTS-backup");
  });
});
