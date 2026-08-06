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
export function sameAddonsFolder(a: string, b: string): boolean {
  const normalize = (value: string) =>
    value.trim().replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
  return normalize(a) === normalize(b);
}

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
   * Should this folder stay hidden from a freshly scanned list — either queued
   * for removal or with its delete already in flight? This, not `has`, is what
   * the scan mask asks.
   */
  isHidden(folderName: string): boolean {
    return this.entries.has(folderName) || this.committing.has(folderName);
  }

  /** The undo window has closed and the backend delete is starting. */
  beginCommit(folderName: string): void {
    this.committing.add(folderName);
  }

  /** The backend delete resolved — succeeded (folder gone) or failed (row
   *  restored). Either way it no longer needs masking. */
  endCommit(folderName: string): void {
    this.committing.delete(folderName);
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
 */
export function hidePendingRemovals<T extends { folderName: string }>(
  items: T[],
  queue: Pick<RemovalQueue, "isHidden">
): T[] {
  if (items.length === 0) return items;
  const masked = items.filter((item) => !queue.isHidden(item.folderName));
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
