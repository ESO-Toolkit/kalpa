import { describe, it, expect } from "vitest";
import { getTauriErrorMessage } from "../tauri";

describe("getTauriErrorMessage — exhaustive error pattern matching", () => {
  describe("zip errors", () => {
    it("matches 'zip extraction aborted' with varying text", () => {
      expect(getTauriErrorMessage("Zip extraction aborted: suspected zip bomb")).toBe(
        "The archive is too large or may be corrupt. Try re-downloading it."
      );
    });

    it("matches 'zip archive contained no addon folders' case-insensitive", () => {
      expect(getTauriErrorMessage("ZIP ARCHIVE CONTAINED NO ADDON FOLDERS")).toBe(
        "This file doesn't look like a valid ESO addon archive."
      );
    });

    it("matches 'failed to open zip file'", () => {
      expect(getTauriErrorMessage("Failed to open zip file: io error")).toBe(
        "Could not open the downloaded file. It may be corrupt or incomplete — try again."
      );
    });

    it("matches 'failed to read zip archive'", () => {
      expect(getTauriErrorMessage("Failed to read ZIP archive at path /foo/bar")).toBe(
        "The downloaded file is not a valid ZIP. It may be corrupt — try re-downloading."
      );
    });
  });

  describe("filesystem errors", () => {
    it("matches 'addons folder not found'", () => {
      expect(getTauriErrorMessage("Addons folder not found at C:\\ESO\\...")).toBe(
        "Your AddOns folder could not be found. It may have been moved or the drive disconnected."
      );
    });

    it("matches os error 13 (permission denied)", () => {
      expect(getTauriErrorMessage("IO error: permission denied (os error 13)")).toBe(
        "Permission denied — antivirus or another program may be blocking the file."
      );
    });

    it("matches os error 5 (access denied)", () => {
      expect(getTauriErrorMessage("OS error 5: access is denied")).toBe(
        "Permission denied — antivirus or another program may be blocking the file."
      );
    });

    it("matches 'Access is denied' without os error number", () => {
      expect(getTauriErrorMessage("Access is denied to file path")).toBe(
        "Permission denied — antivirus or another program may be blocking the file."
      );
    });
  });

  describe("ESOUI network errors", () => {
    it("matches 'could not reach esoui'", () => {
      expect(getTauriErrorMessage("Could not reach ESOUI: connection timeout")).toBe(
        "ESOUI could not be reached. Check your internet connection and try again."
      );
    });

    it("matches 'too many requests to esoui'", () => {
      expect(getTauriErrorMessage("Too many requests to ESOUI (429)")).toBe(
        "ESOUI rate limit reached. Wait a moment and try again."
      );
    });

    it("matches 'esoui is currently unavailable'", () => {
      expect(getTauriErrorMessage("ESOUI is currently unavailable (503)")).toBe(
        "ESOUI appears to be down. Try again in a few minutes."
      );
    });

    it("matches 'addon not found on esoui'", () => {
      expect(getTauriErrorMessage("Addon not found on ESOUI (id: 12345)")).toBe(
        "This addon was not found on ESOUI — it may have been removed by its author."
      );
    });
  });

  describe("edge cases", () => {
    it("returns raw message for unrecognized error strings", () => {
      expect(getTauriErrorMessage("some random error message")).toBe("some random error message");
    });

    it("returns fallback for empty string", () => {
      expect(getTauriErrorMessage("")).toBe("Something went wrong");
    });

    it("returns fallback for whitespace-only string", () => {
      expect(getTauriErrorMessage("   ")).toBe("Something went wrong");
    });

    it("returns fallback for undefined", () => {
      expect(getTauriErrorMessage(undefined)).toBe("Something went wrong");
    });

    it("returns fallback for null", () => {
      expect(getTauriErrorMessage(null)).toBe("Something went wrong");
    });

    it("returns fallback for number", () => {
      expect(getTauriErrorMessage(42)).toBe("Something went wrong");
    });

    it("returns fallback for empty object", () => {
      expect(getTauriErrorMessage({})).toBe("Something went wrong");
    });

    it("handles Error with empty message", () => {
      expect(getTauriErrorMessage(new Error(""))).toBe("Something went wrong");
    });

    it("handles Error with matching message", () => {
      expect(getTauriErrorMessage(new Error("ESOUI is currently unavailable"))).toBe(
        "ESOUI appears to be down. Try again in a few minutes."
      );
    });

    it("first matching pattern wins", () => {
      // An error that could match multiple patterns — verify first match is returned
      expect(getTauriErrorMessage("permission denied (os error 13) while opening zip file")).toBe(
        "Permission denied — antivirus or another program may be blocking the file."
      );
    });
  });
});
