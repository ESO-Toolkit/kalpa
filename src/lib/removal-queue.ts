/**
 * The optimistic-removal queue that backs Kalpa's "Removed X — Undo" toasts.
 *
 * Removing an addon hides its row immediately and schedules the real
 * `remove_addon` call 3 seconds later, so the undo affordance is instant. That
 * makes the queue a small state machine with three ways out — the timer fires,
 * the user hits Undo, or the app flushes everything (window close, AddOns-path
 * switch) — and every one of them has shipped a bug:
 *
 * * A batch removal shares ONE timer across many folders. Clearing that timer
 *   when the first folder leaves stranded the rest: still queued, timer dead,
 *   then silently deleted by the close-time flush. Hence {@link RemovalQueue.drop}
 *   only clears the timer once the group's last member has left.
 * * The AddOns folder travels WITH each entry. The backend authorizes exactly
 *   one folder at a time, so a removal queued before an instance switch must run
 *   against the folder it was queued for — otherwise it deletes a same-named
 *   addon in the instance the user moved to, and the failure path resurrects a
 *   ghost row in a folder that never had it.
 * * The update row rides along too, so Undo restores the addon's "Update
 *   available" badge, banner count and "Outdated" filter membership rather than
 *   silently downgrading it to up-to-date.
 *
 * Everything here is pure or explicitly injected, so those rules can be tested
 * without mounting the app. `App.tsx` owns the React state; this module owns the
 * decisions.
 */

import type { AddonManifest, UpdateCheckResult } from "../types";

export type TimerHandle = ReturnType<typeof setTimeout>;

/** A set of folders removed together, sharing one undo timer. */
export interface PendingRemovalGroup {
  timer: TimerHandle;
  folderNames: Set<string>;
}

/**
 * One optimistically-hidden addon awaiting its undo window. See the module
 * comment for why `addonsPath` and `updateResult` are carried per entry.
 */
export interface PendingRemoval {
  addon: AddonManifest;
  updateResult: UpdateCheckResult | null;
  addonsPath: string;
  group: PendingRemovalGroup;
}

/**
 * Compare AddOns folders leniently.
 *
 * Paths reach us from settings as a bare trim, so the same physical folder can
 * arrive with different casing, with `/` or `\`, or with a trailing separator.
 * Raw equality would split one folder into two and defeat the staleness checks
 * that keep a queued removal pointed at its own instance. This is a staleness
 * check, not a security boundary — the backend authorizes by canonical path.
 */
function normalizeAddonsFolder(value: string): string {
  return value.trim().replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
}

export function sameAddonsFolder(a: string, b: string): boolean {
  return normalizeAddonsFolder(a) === normalizeAddonsFolder(b);
}

/**
 * Identity of a removal: the folder AND the instance it belongs to.
 *
 * Folder names are not unique across game instances — the same addon is
 * installed under the same name in live, PTS and every Steam copy. Keying
 * hidden state by name alone means an in-flight delete in one instance hides a
 * real, untouched addon of the same name in another. `PendingRemoval` already
 * carries `addonsPath` for exactly this reason; the mask has to respect it too.
 */
function maskKey(addonsPath: string, folderName: string): string {
  return `${normalizeAddonsFolder(addonsPath)}\u0000${folderName}`;
}

/**
 * How many confirmed removals the queue remembers.
 *
 * The log only has to outlive the requests that were in flight when a removal
 * landed, so this is a bound against unbounded growth over a long session
 * rather than a tuning knob. Anything evicted simply falls back to the
 * pre-existing masking answer.
 */
const MAX_COMPLETED_REMOVALS = 256;

/**
 * The queue itself. `clearTimer` is injectable so tests can assert *when* a
 * group's timer is cancelled without running real timers.
 */
export class RemovalQueue {
  private readonly entries = new Map<string, PendingRemoval>();

  /**
   * Folders whose undo window has closed and whose `remove_addon` call is
   * in flight.
   *
   * Leaving the queue is not the same as being gone from disk. The timer drops
   * the entry (so undo can no longer apply) and only THEN starts the async
   * delete, and during that gap the folder is still on disk while nothing marks
   * it as going away — a scan landing there reads it back and restores the row,
   * which nothing hides again once the delete succeeds. That is the same ghost
   * row the scan mask exists to prevent, one layer down.
   *
   * So visibility is tracked separately from undo eligibility: an entry is
   * hidden from scans from the moment it is queued until the backend either
   * confirms the delete or fails and the row is restored.
   */
  private readonly committing = new Set<string>();

  /**
   * Removals the backend has CONFIRMED, stamped with the generation at which
   * they landed.
   *
   * `committing` only covers the delete while it is in flight. It cannot cover
   * the request that enumerated the folder BEFORE the delete started and is
   * delivered after it finished — by then nothing is queued and nothing is
   * committing, so the row goes back into the list and stays there until the
   * next scan. Closing that window needs the queue to remember removals that
   * completed after a given request began, which is what this log plus
   * {@link RemovalQueue.stamp} provide: a caller snapshots the generation when
   * it issues the request and masks anything whose removal was confirmed at a
   * LATER generation.
   *
   * Comparing generations rather than just membership is what keeps a reinstall
   * visible. A scan issued after the addon came back carries a stamp at or above
   * the removal's generation, so the log no longer masks it and the fresh row
   * shows through.
   */
  private readonly completed = new Map<string, number>();

  /** Bumped once per confirmed removal; see {@link RemovalQueue.completed}. */
  private generation = 0;

  private readonly clearTimer: (handle: TimerHandle) => void;

  /**
   * The default MUST wrap `clearTimeout` rather than being it.
   *
   * `clearTimeout` is a Window method, and storing it on an instance means it is
   * later invoked as `this.clearTimer(handle)` — a call whose receiver is a
   * RemovalQueue, not a Window. WebIDL brand-checks that receiver, so the call
   * throws `TypeError: Illegal invocation` in Chromium/WebView2, which aborts
   * `drop()` before it returns and takes the removal (or the undo) with it.
   *
   * Verified in the shipped engine (WebView2, Chrome 151): a bare
   * `clearTimeout(h)` succeeds, `this.clearTimer(h)` with the raw global throws,
   * and this wrapper succeeds.
   *
   * jsdom does NOT brand-check, and every test here injects a mock, so neither
   * the unit suite nor the type checker can catch a regression to the bare
   * global — only the real browser can. Do not "simplify" this back.
   */
  constructor(clearTimer: (handle: TimerHandle) => void = (handle) => clearTimeout(handle)) {
    this.clearTimer = clearTimer;
  }

  get size(): number {
    return this.entries.size;
  }

  /** Is this folder still inside its undo window? Undo eligibility only. */
  has(folderName: string): boolean {
    return this.entries.has(folderName);
  }

  /**
   * Should this folder stay hidden from a scan of `addonsPath` — either queued
   * for removal there or with its delete already in flight there? This, not
   * `has`, is what the scan mask asks.
   *
   * Scoped to the instance. A delete in progress against the live folder must
   * not hide a same-named addon the user just switched to in PTS, which nothing
   * would restore: `endCommit` only lifts the mask, it does not re-run the scan
   * that was filtered while it was up.
   */
  isHidden(folderName: string, addonsPath: string, since = Number.POSITIVE_INFINITY): boolean {
    const queued = this.entries.get(folderName);
    if (queued && sameAddonsFolder(queued.addonsPath, addonsPath)) return true;
    const key = maskKey(addonsPath, folderName);
    if (this.committing.has(key)) return true;
    // Confirmed gone AFTER the caller issued its request, so the folder the
    // backend enumerated no longer exists by the time the answer is applied.
    // A folder with no log entry is NOT hidden — the absent case has to read as
    // "never removed", not as an unbounded generation.
    const removedAt = this.completed.get(key);
    return removedAt !== undefined && removedAt > since;
  }

  /**
   * Snapshot the generation for a request about to be issued.
   *
   * Pair it with the `since` argument of {@link RemovalQueue.isHidden} (or
   * {@link hidePendingRemovals}) when the result is applied.
   */
  stamp(): number {
    return this.generation;
  }

  /**
   * The backend confirmed the folder is gone. Records it so requests already in
   * flight do not deliver it back as a phantom row.
   *
   * Call this on the SUCCESS path only, before {@link RemovalQueue.endCommit}
   * lifts the in-flight mask — a failed delete leaves the folder on disk and its
   * row must come back.
   */
  markRemoved(folderName: string, addonsPath: string): void {
    const key = maskKey(addonsPath, folderName);
    // Re-setting an existing key keeps its ORIGINAL insertion slot, which would
    // make the oldest-first eviction below evict a fresh entry. Delete first so
    // insertion order and generation order stay the same ordering.
    this.completed.delete(key);
    this.completed.set(key, ++this.generation);
    // Only requests in flight can still consult the log, so a small window of
    // history is enough. Map iterates in insertion order and generations only
    // ever increase, so the first key is always the oldest.
    while (this.completed.size > MAX_COMPLETED_REMOVALS) {
      const oldest = this.completed.keys().next();
      if (oldest.done) break;
      this.completed.delete(oldest.value);
    }
  }

  /** The undo window has closed and the backend delete is starting. */
  beginCommit(folderName: string, addonsPath: string): void {
    this.committing.add(maskKey(addonsPath, folderName));
  }

  /** The backend delete resolved — succeeded (folder gone) or failed (row
   *  restored). Either way it no longer needs masking. */
  endCommit(folderName: string, addonsPath: string): void {
    this.committing.delete(maskKey(addonsPath, folderName));
  }

  /** Queue (or re-queue) a folder. Callers drop an existing entry first. */
  add(folderName: string, entry: PendingRemoval): void {
    this.entries.set(folderName, entry);
  }

  /**
   * Take one folder out of the queue, cancelling its group's timer only once the
   * last member has left — clearing it per-entry would kill a shared batch timer
   * and strand the batch's other folders.
   */
  drop(folderName: string): PendingRemoval | null {
    const entry = this.entries.get(folderName);
    if (!entry) return null;
    this.entries.delete(folderName);
    entry.group.folderNames.delete(folderName);
    if (entry.group.folderNames.size === 0) this.clearTimer(entry.group.timer);
    return entry;
  }

  /**
   * Remove and return every queued entry, leaving the queue empty and every
   * group timer cancelled.
   *
   * The key list is snapshotted before dropping so mutation during iteration
   * cannot skip an entry — a skipped entry is one the close-time flush would
   * later delete with no undo path left.
   */
  drain(): PendingRemoval[] {
    return [...this.entries.keys()]
      .map((folderName) => this.drop(folderName))
      .filter((entry): entry is PendingRemoval => entry !== null);
  }
}

/**
 * Drop anything still inside its undo window from a freshly scanned list.
 *
 * A queued removal has hidden its row but has NOT deleted the folder yet — the
 * real `remove_addon` is 3 seconds away. So a rescan landing inside that window
 * reads the addon straight back off disk and puts the row back, and when the
 * timer then fires and the delete succeeds nothing hides it again: the list
 * shows an addon that no longer exists until the next scan.
 *
 * Masking here rather than at each call site keeps "what the user can see" and
 * "what the queue is about to delete" in one place. Undo and failure-restore
 * both drop the entry from the queue first, so the row reappears the moment it
 * is genuinely back.
 *
 * Generic over anything folder-keyed, so the addon list and its update rows
 * filter through the same rule.
 *
 * The mask is read when the request RESOLVES, not when the backend read the
 * folder, so "still queued or still committing" is not enough on its own. The
 * race is between the backend ENUMERATING the addon and this code applying the
 * result:
 *
 *     t=0.0  Refresh issues scan_installed_addons
 *     t=0.1  the backend enumerates the folder — Foo is still on disk
 *     t=0.2  user removes Foo — row hidden, entry queued
 *     t=3.2  timer fires, remove_addon starts (beginCommit masks Foo)
 *     t=3.3  delete succeeds, endCommit lifts the mask
 *     t=3.4  the scan resolves — nothing is queued, nothing is committing
 *
 * At t=3.4 the queue-only mask says "visible" and Foo goes back into `addons`
 * as a phantom row — an addon that exists in neither the AddOns folder nor
 * `kalpa.json`, sitting in the list until something else triggers a scan. That
 * is the shipped "I uninstalled it and it still shows installed" report, and
 * why it looked arbitrary: it needs a scan in flight across the delete, so two
 * removals seconds apart can behave differently.
 *
 * `since` closes it. Callers snapshot {@link RemovalQueue.stamp} BEFORE issuing
 * the request and pass it here; anything whose removal was confirmed at a later
 * generation stays masked no matter what the backend reported. Omitting `since`
 * keeps the old queue-only behaviour, which is what callers that are not
 * applying an in-flight result (the pre-invoke re-mask in `runBatchUpdates`)
 * actually want.
 *
 * The window is "enumerated before the delete, delivered after it", NOT the
 * whole request duration. A request that has not read the folder yet when the
 * delete lands sees Foo already gone and reports nothing, which is why
 * `check_for_updates` is the WEAKER example despite being the slower call: it
 * fetches the full ESOUI filelist first and only then compares local metadata,
 * so a delete during that fetch is simply reflected. Its exposure is the
 * narrower gap between the metadata phase and delivery.
 *
 * On `main` this window was not narrower, it was total: nothing masked these
 * lists at all, so every scan inside the undo window resurrected the row.
 */
export function hidePendingRemovals<T extends { folderName: string }>(
  items: T[],
  queue: Pick<RemovalQueue, "isHidden">,
  addonsPath: string,
  since?: number
): T[] {
  if (items.length === 0) return items;
  const masked = items.filter((item) => !queue.isHidden(item.folderName, addonsPath, since));
  return masked.length === items.length ? items : masked;
}

/**
 * Should a failed removal put the row back?
 *
 * Only when the entry belongs to the folder currently on screen. After an
 * instance switch the restored row would appear in a folder that never had the
 * addon.
 */
export function shouldRestoreAfterFailure(entry: PendingRemoval, livePath: string): boolean {
  return sameAddonsFolder(entry.addonsPath, livePath);
}

/**
 * Put an optimistically-hidden addon back into the list. Idempotent: a restore
 * racing the rescan that already re-added the row must not duplicate it.
 */
export function restoreAddon(prev: AddonManifest[], addon: AddonManifest): AddonManifest[] {
  return prev.some((a) => a.folderName === addon.folderName) ? prev : [...prev, addon];
}

/**
 * Put the addon's update row back alongside it, so a restored addon keeps its
 * Update badge, banner count and "Outdated" filter membership. A null
 * `updateResult` means the addon had no update row to begin with.
 */
export function restoreUpdateResult(
  prev: UpdateCheckResult[],
  updateResult: UpdateCheckResult | null
): UpdateCheckResult[] {
  if (!updateResult) return prev;
  return prev.some((r) => r.folderName === updateResult.folderName)
    ? prev
    : [...prev, updateResult];
}

/** Optimistically hide a row. */
export function hideAddon(prev: AddonManifest[], folderName: string): AddonManifest[] {
  return prev.filter((a) => a.folderName !== folderName);
}

/** Optimistically hide the row's update entry alongside it. */
export function hideUpdateResult(
  prev: UpdateCheckResult[],
  folderName: string
): UpdateCheckResult[] {
  return prev.filter((r) => r.folderName !== folderName);
}

/**
 * Drop a removed folder from the multi-select set, preserving referential
 * identity when it was not selected so React can skip the re-render.
 */
export function removeFolderFromSelection(prev: Set<string>, folderName: string): Set<string> {
  if (!prev.has(folderName)) return prev;
  const next = new Set(prev);
  next.delete(folderName);
  return next;
}
