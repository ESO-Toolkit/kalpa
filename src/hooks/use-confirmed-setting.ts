import { useCallback, useRef, useState } from "react";
import { toast } from "sonner";

/**
 * A settings control that shows what is stored, never what was merely clicked.
 *
 * Most toggles in Settings update optimistically and persist in the background.
 * That is fine when the displayed value is the only consumer, and wrong when
 * something else reads the SAME preference from storage and acts on it. The
 * dependency policy is the second kind: the install path reads it to decide
 * whether to offer a missing library at all, so a radio showing "ask" over a
 * stored "skip" means a required library is silently never offered, and the
 * user has every reason to believe it would be.
 *
 * The optimistic version of this was tried and abandoned. Rolling back on a
 * failed write needs a rollback target, which needs to distinguish values that
 * reached disk from values that did not, which still cannot repair a write that
 * never reaches EITHER outcome — a save queued behind a wedged one leaves the
 * control showing a value the store may never receive. No amount of rollback
 * bookkeeping fixes that, because the bug is displaying an unconfirmed value in
 * the first place.
 *
 * So the displayed value follows CONFIRMED STORAGE, never the last click:
 *
 *   - A confirmed write is displayed, and is remembered as what storage holds
 *     even if the user has clicked past it since.
 *   - A failed write shows whatever last landed, and says so. That is not
 *     always where the control started: clicking twice in a burst can land the
 *     first write and fail the second.
 *   - A write that never settles leaves the control where it was, silently —
 *     truthful, since the setting genuinely did not change.
 *   - Readers need no ordering against pending writes. Once the control shows a
 *     value, the store already holds it.
 *
 * The cost is that the control lags a click by one local file write. The
 * benefit is that it cannot lie.
 */
export function useConfirmedSetting<T>(
  initial: T,
  save: (value: T) => Promise<boolean>
): {
  /** The stored value — safe to render, and safe for other code to assume. */
  value: T;
  /** Persist, then display. Does nothing visible until the write confirms. */
  commit: (next: T) => void;
  /** Seed from storage on load. */
  hydrate: (loaded: T) => void;
} {
  const [value, setValue] = useState<T>(initial);
  // Best knowledge of what storage holds. Every confirmed write updates it,
  // including one the user has already clicked past — that write reached disk,
  // so it is the truth regardless of what was clicked afterwards. Starts at the
  // caller's default, which is only a guess until `hydrate` or a write lands.
  const confirmedRef = useRef<T>(initial);
  const writeConfirmedRef = useRef(false);
  // Identifies the newest click, so only that one decides what is displayed.
  const seqRef = useRef(0);

  const hydrate = useCallback((loaded: T) => {
    // The load reflects storage, so it updates the confirmed value — unless a
    // write has already landed, which is newer than this mount-time read.
    if (!writeConfirmedRef.current) confirmedRef.current = loaded;
    // It only reaches the display before the user has acted: the load is async,
    // so a click can land while it is in flight, and this read predates it.
    if (seqRef.current === 0) setValue(loaded);
  }, []);

  const commit = useCallback(
    (next: T) => {
      const seq = ++seqRef.current;
      void save(next)
        // A rejection is a failed write like any other. Both callers go through
        // `setSetting`, which is documented never to throw, but the guarantee
        // here must not rest on a convention the signature does not enforce.
        .catch(() => false)
        .then((ok) => {
          if (ok) {
            confirmedRef.current = next;
            writeConfirmedRef.current = true;
          }
          // Superseded: a later click is still in flight and will settle the
          // display itself. Its success or failure — not this one — is what the
          // control should end up reflecting.
          if (seq !== seqRef.current) return;
          // Always follow storage, never the click. On success that is `next`;
          // on failure it is whatever last landed, which may be an EARLIER
          // click of this same burst that succeeded while this one failed.
          // Leaving the display alone there was the bug: click skip, click ask
          // again, skip lands and ask fails, and the control kept saying "ask"
          // over a stored "skip" — the silent-suppression case.
          setValue(confirmedRef.current);
          if (!ok) toast.error("Couldn't save that setting — try again.");
        });
    },
    [save]
  );

  return { value, commit, hydrate };
}
