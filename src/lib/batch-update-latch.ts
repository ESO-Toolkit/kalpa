/**
 * The re-entry guard for Update All.
 *
 * A batch update has an async preamble — the ESO-running check, its confirm
 * dialog, and the write-access probe — that runs before any React state lands.
 * Two rapid triggers (the button, an auto-update, a keyboard shortcut) could
 * both clear that preamble and start overlapping batches against the same
 * AddOns folder, each extracting over the other's files.
 *
 * So there are two latches, not one:
 *
 * * `preflight` is claimed synchronously on entry, before the first `await`, and
 *   released on every preamble exit. It covers the window where no state exists
 *   yet.
 * * `running` mirrors the `updatingAll` React state and covers the batch itself.
 *
 * They are deliberately independent booleans rather than one tri-state enum:
 * `running` is re-synced from React state on every render, and a single enum
 * would let that sync clobber a preflight claim made in the same tick.
 *
 * The bug this file exists to prevent: a refresh landing mid-batch used to clear
 * `updatingAll`, which unlatched the guard and let a second batch start on top
 * of the first. Nothing on the refresh path may touch this latch — an update
 * *check* is not an update *run*, and only the batch that set `running` may
 * clear it.
 */

export class BatchUpdateLatch {
  private running = false;
  private preflight = false;

  /** Is a batch update in progress? Read by the auto-refresh paths, which skip
   * their rescan rather than yanking the list out from under a running batch. */
  get isRunning(): boolean {
    return this.running;
  }

  /** Has the preamble been claimed but not yet promoted or aborted? */
  get isPreflight(): boolean {
    return this.preflight;
  }

  /**
   * Mirror the `updatingAll` React state, exactly as the render-sync effect
   * does. This is the ONLY way `running` is cleared, which is why the refresh
   * path must never call `setUpdatingAll(false)`.
   */
  syncRunning(updatingAll: boolean): void {
    this.running = updatingAll;
  }

  /**
   * Claim the preamble slot. Returns false — meaning "abandon this trigger" —
   * when a batch is already running or another trigger is already in its
   * preamble.
   */
  tryEnterPreflight(): boolean {
    if (this.running) return false;
    if (this.preflight) return false;
    this.preflight = true;
    return true;
  }

  /** Release the preamble slot without starting a batch (user cancelled, ESO
   * running, folder not writable). */
  abortPreflight(): void {
    this.preflight = false;
  }

  /**
   * Hand off from the preamble latch to the in-progress latch synchronously, so
   * the `running` guard covers the gap until the `updatingAll` state lands.
   */
  promoteToRunning(): void {
    this.running = true;
    this.preflight = false;
  }
}
