import { describe, expect, it } from "vitest";

import { stackVerdict } from "@/components/client-health";
import type {
  ClientHealthReport,
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

describe("stackVerdict", () => {
  it("claims agreement only when the evaluation counter was seen climbing", () => {
    expect(stackVerdict(0, evidence("running"))).toMatchObject({
      label: "Everything agrees",
      color: "emerald",
    });
  });

  it("gives a stalled counter its own copy rather than an all-clear", () => {
    const verdict = stackVerdict(0, evidence("stalled"));
    expect(verdict.label).toBe("Ran, then stopped");
    expect(verdict.label).not.toBe("Everything agrees");
    expect(verdict.color).toBe("amber");
  });

  it("renders no evidence as neither working nor failing", () => {
    const verdict = stackVerdict(0, evidence("unknown"));
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
    const verdict = stackVerdict(0, evidence("running", { log_excerpts: [FATAL] }));
    expect(verdict.label).toBe("1 log failure");
    expect(verdict.color).toBe("red");
  });

  it("pluralises fatal log lines", () => {
    expect(stackVerdict(0, evidence("running", { log_excerpts: [FATAL, FATAL] })).label).toBe(
      "2 log failures"
    );
  });

  it("still puts findings first, because a diagnosis outranks evidence", () => {
    const verdict = stackVerdict(2, evidence("running", { log_excerpts: [FATAL] }));
    expect(verdict.label).toBe("2 need attention");
    expect(verdict.color).toBe("amber");
  });

  it("gives every state an icon, so colour is never the only signal", () => {
    // Five shipped themes are light or high-contrast and `status-*` is reseeded
    // per theme, so a pill distinguished only by colour distinguishes nothing.
    const verdicts = [
      stackVerdict(1, evidence("running")),
      stackVerdict(0, evidence("running", { log_excerpts: [FATAL] })),
      stackVerdict(0, evidence("running")),
      stackVerdict(0, evidence("stalled")),
      stackVerdict(0, evidence("unknown")),
    ];
    for (const verdict of verdicts) {
      expect(verdict.Icon).toBeTypeOf("object");
      expect(verdict.label.length).toBeGreaterThan(0);
    }
    expect(new Set(verdicts.map((v) => v.label)).size).toBe(verdicts.length);
  });
});
