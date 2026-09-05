import { describe, expect, it } from "vitest";

import { stackVerdict } from "@/components/client-health";
import type {
  ClientHealthReport,
  ClientStack,
  LogExcerpt,
  NeuralRenderingState,
} from "@/components/client-stack/types";

/**
 * "Everything agrees" has to be earned.
 *
 * The header badge used to read `attentionCount > 0 ? "n need attention" :
 * "Everything agrees"`, so the reassuring half was rendered by an *empty array*
 * — the absence of findings, never positive evidence that anything ran. That is
 * how it claimed agreement over a stack that could not work: no
 * `HealthFinding` is emitted from a log rule at all, so the empty list was
 * silent about the one signature that mattered.
 *
 * These cover both gates that were missing and all three evidence states. The
 * third state is the easy one to get wrong twice: `unknown` must read as
 * neither working nor failing, because a missing or truncated log is the common
 * case and rendering it as breakage is the same bug pointing the other way.
 */

type Evidence = Pick<
  ClientHealthReport,
  "log_excerpts" | "neural_rendering" | "log_benign_suppressed"
>;

function evidence(state: NeuralRenderingState, overrides: Partial<Evidence> = {}): Evidence {
  return {
    log_excerpts: [],
    neural_rendering: {
      state,
      samples: state === "unknown" ? 0 : 3,
      first_evaluation: state === "running" ? 1 : state === "stalled" ? 7 : null,
      last_evaluation: state === "running" ? 412 : state === "stalled" ? 7 : null,
    },
    log_benign_suppressed: 6,
    ...overrides,
  };
}

const FATAL: LogExcerpt = {
  file: "ReShade.log",
  rule: "addon-not-in-dllmain",
  line: "renodx-dlss.addon64 is not listed in ADDON.LoadFromDllMain.",
};

/** A stack that could plausibly be running: not disabled, direct path live.
 *  Tests that want to prove the happy path still works, or that want to
 *  isolate a different gate, start from this and override only what they are
 *  about. */
function liveStack(overrides: Partial<ClientStack> = {}): ClientStack {
  return {
    client_dir: "C:\\eso\\game\\client",
    items: [],
    preserved_originals: [],
    parked: [],
    user_parked: [],
    is_disabled: false,
    shaders: {
      present: true,
      effect_count: 1,
      texture_count: 0,
      effect_search_paths: null,
    },
    preset: null,
    tuning: [],
    tuning_section: null,
    tuning_owner: "unknown",
    tuning_blocks: [],
    disabled_addons: [],
    load_from_dll_main: [],
    active_path: "direct",
    slots: [],
    is_empty: false,
    findings: [],
    ...overrides,
  };
}

describe("stackVerdict", () => {
  it("claims agreement only when the evaluation counter was seen climbing", () => {
    expect(stackVerdict(0, evidence("running"), liveStack())).toMatchObject({
      label: "Everything agrees",
      color: "emerald",
    });
  });

  it("gives a stalled counter its own copy rather than an all-clear", () => {
    const verdict = stackVerdict(0, evidence("stalled"), liveStack());
    expect(verdict.label).toBe("Ran, then stopped");
    expect(verdict.label).not.toBe("Everything agrees");
    expect(verdict.color).toBe("amber");
  });

  it("renders no evidence as neither working nor failing", () => {
    const verdict = stackVerdict(0, evidence("unknown"), liveStack());
    expect(verdict.label).toBe("No proof it ran");
    // Not emerald: it is not an all-clear. Not red or amber either: an absent
    // or truncated log is not a fault, and saying so would be the same bug
    // inverted.
    expect(verdict.color).toBe("muted");
  });

  it("lets a fatal log line block the claim even with a climbing counter and no findings", () => {
    // The second gate, and the one easiest to miss: `log_excerpts` is
    // fatal-only and raises no finding, so a findings-only check sails straight
    // past the LoadFromDllMain line.
    const verdict = stackVerdict(0, evidence("running", { log_excerpts: [FATAL] }), liveStack());
    expect(verdict.label).toBe("1 log failure");
    expect(verdict.color).toBe("red");
  });

  it("pluralises fatal log lines", () => {
    expect(
      stackVerdict(0, evidence("running", { log_excerpts: [FATAL, FATAL] }), liveStack()).label
    ).toBe("2 log failures");
  });

  it("still puts findings first, because a diagnosis outranks evidence", () => {
    const verdict = stackVerdict(2, evidence("running", { log_excerpts: [FATAL] }), liveStack());
    expect(verdict.label).toBe("2 need attention");
    expect(verdict.color).toBe("amber");
  });

  it("gives every state an icon, so colour is never the only signal", () => {
    // Five shipped themes are light or high-contrast and `status-*` is reseeded
    // per theme, so a pill distinguished only by colour distinguishes nothing.
    const verdicts = [
      stackVerdict(1, evidence("running"), liveStack()),
      stackVerdict(0, evidence("running", { log_excerpts: [FATAL] }), liveStack()),
      stackVerdict(0, evidence("running"), liveStack()),
      stackVerdict(0, evidence("stalled"), liveStack()),
      stackVerdict(0, evidence("unknown"), liveStack()),
    ];
    for (const verdict of verdicts) {
      expect(verdict.Icon).toBeTypeOf("object");
      expect(verdict.label.length).toBeGreaterThan(0);
    }
    expect(new Set(verdicts.map((v) => v.label)).size).toBe(verdicts.length);
  });

  /**
   * The gate this file exists to add: `running` evidence is a fact about the
   * last time ReShade launched, and ReShade truncates its log on every
   * launch, so that fact survives long after the stack it describes changed.
   * A user who switches the stack off from the power card, or removes the
   * add-on by hand, leaves the log completely untouched — so `running` must
   * also be checked against what the stack looks like *right now*, not just
   * trusted on its own.
   */
  describe("stale 'running' evidence over a stack that cannot run", () => {
    it("does not claim agreement when the stack is switched off", () => {
      const verdict = stackVerdict(0, evidence("running"), liveStack({ is_disabled: true }));
      expect(verdict.label).not.toBe("Everything agrees");
      expect(verdict.label).toBe("No proof it ran");
      expect(verdict.color).toBe("muted");
    });

    it("does not claim agreement when no path is live", () => {
      const verdict = stackVerdict(0, evidence("running"), liveStack({ active_path: "neither" }));
      expect(verdict.label).toBe("No proof it ran");
      expect(verdict.color).toBe("muted");
    });

    it("does not claim agreement when the live path could not be determined", () => {
      const verdict = stackVerdict(0, evidence("running"), liveStack({ active_path: "unknown" }));
      expect(verdict.label).toBe("No proof it ran");
      expect(verdict.color).toBe("muted");
    });

    it("does not claim agreement when the stack has not loaded yet", () => {
      const verdict = stackVerdict(0, evidence("running"), null);
      expect(verdict.label).toBe("No proof it ran");
      expect(verdict.color).toBe("muted");
    });

    it("falls through to unknown, never to a failure colour, when the stack is off", () => {
      // The stack being off is a deliberate user choice, not a fault — this
      // must render as the same "neither working nor failing" copy as no
      // evidence at all, not as red or amber.
      const verdict = stackVerdict(0, evidence("running"), liveStack({ is_disabled: true }));
      expect(verdict.color).not.toBe("red");
      expect(verdict.color).not.toBe("amber");
    });

    it("still allows the happy path: live direct path, not disabled, running", () => {
      const verdict = stackVerdict(
        0,
        evidence("running"),
        liveStack({ is_disabled: false, active_path: "direct" })
      );
      expect(verdict.label).toBe("Everything agrees");
      expect(verdict.color).toBe("emerald");
    });

    it("still allows the happy path on the feed and both paths", () => {
      for (const active_path of ["feed", "both"] as const) {
        const verdict = stackVerdict(0, evidence("running"), liveStack({ active_path }));
        expect(verdict.label).toBe("Everything agrees");
      }
    });

    it("findings still win over a live path with running evidence", () => {
      const verdict = stackVerdict(1, evidence("running"), liveStack());
      expect(verdict.label).toBe("1 need attention");
      expect(verdict.color).toBe("amber");
    });

    it("a fatal log line still beats running evidence on a live path", () => {
      const verdict = stackVerdict(0, evidence("running", { log_excerpts: [FATAL] }), liveStack());
      expect(verdict.label).toBe("1 log failure");
      expect(verdict.color).toBe("red");
    });
  });
});
