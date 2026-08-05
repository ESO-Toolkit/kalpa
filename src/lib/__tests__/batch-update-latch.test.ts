import { describe, expect, it } from "vitest";
import { BatchUpdateLatch } from "../batch-update-latch";

describe("BatchUpdateLatch", () => {
  it("refuses a second trigger during the async preamble", () => {
    // The preamble (ESO-running check, confirm dialog, write-access probe) runs
    // before any React state lands, so without a synchronous latch two rapid
    // triggers both cleared it and started overlapping batches extracting into
    // the same AddOns folder.
    const latch = new BatchUpdateLatch();

    expect(latch.tryEnterPreflight()).toBe(true);
    expect(latch.tryEnterPreflight()).toBe(false);
  });

  it("frees the slot when the preamble bails out", () => {
    const latch = new BatchUpdateLatch();
    latch.tryEnterPreflight();

    latch.abortPreflight();

    expect(latch.isPreflight).toBe(false);
    expect(latch.tryEnterPreflight()).toBe(true);
  });

  it("hands off from preflight to running without an unguarded gap", () => {
    const latch = new BatchUpdateLatch();
    latch.tryEnterPreflight();

    latch.promoteToRunning();

    expect(latch.isPreflight).toBe(false);
    expect(latch.isRunning).toBe(true);
    // The batch is guarded the moment it starts, before `updatingAll` renders.
    expect(latch.tryEnterPreflight()).toBe(false);
  });

  it("stays latched until syncRunning(false), and refuses re-entry until then", () => {
    // NOTE ON WHAT THIS DOES *NOT* COVER. The regression behind this class is
    // that `checkForUpdates` cleared `updatingAll` mid-batch, unlatching the
    // guard so a second Update All could start on top of the first. No test at
    // this boundary can catch that: the latch has no concept of a refresh, and
    // `syncRunning` is an unconditional setter the refresh path is simply
    // expected not to call. The property is held by `checkForUpdates` in
    // App.tsx — see the comment there — and that is where a regression would
    // have to be caught.
    //
    // An earlier version of this test was named after the regression and
    // claimed to cover it. It did not: it asserted state was unchanged after
    // calling nothing, and stayed green whether or not the refresh path cleared
    // `updatingAll`. A test named after a bug it cannot catch is worse than no
    // test, because it stops the next reader from looking.
    const latch = new BatchUpdateLatch();
    latch.tryEnterPreflight();
    latch.promoteToRunning();

    expect(latch.tryEnterPreflight(), "re-entry while running must be refused").toBe(false);

    latch.syncRunning(false);
    expect(latch.isRunning).toBe(false);
    expect(latch.tryEnterPreflight(), "a finished batch releases the guard").toBe(true);
  });

  it("mirrors the updatingAll state without clobbering a preflight claim", () => {
    // The render-sync effect writes `running` on every render. A single
    // tri-state enum would let that sync wipe a preflight claim made in the same
    // tick, reopening the double-run window the preflight latch exists to close.
    const latch = new BatchUpdateLatch();
    latch.tryEnterPreflight();

    latch.syncRunning(false);

    expect(latch.isPreflight).toBe(true);
    expect(latch.tryEnterPreflight()).toBe(false);
  });
});
