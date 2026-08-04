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

  it("keeps the guard latched when a refresh lands mid-batch", () => {
    // THE regression. `checkForUpdates` used to clear `updatingAll`, which
    // unlatched this guard while the batch was still extracting — a second
    // Update All could then start on top of the first. An update *check* is not
    // an update *run*: nothing on the refresh path may touch the latch.
    const latch = new BatchUpdateLatch();
    latch.tryEnterPreflight();
    latch.promoteToRunning();

    // A refresh completing mid-batch: it sets `checkingUpdates` and replaces
    // `updateResults`, and deliberately does not touch the batch latch.
    expect(latch.isRunning).toBe(true);
    expect(latch.tryEnterPreflight()).toBe(false);

    // Only the batch that set `running` clears it, via the `updatingAll` sync.
    latch.syncRunning(false);
    expect(latch.isRunning).toBe(false);
    expect(latch.tryEnterPreflight()).toBe(true);
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
