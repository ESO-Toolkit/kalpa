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
 * So the value moves only when the write says it landed:
 *
 *   - A failed write leaves the control where it was, and says so.
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
  // Identifies the newest write. Two clicks can be in flight at once and can
  // resolve out of order, so an older one settling last must not paint its
  // value over the newer choice.
  const seqRef = useRef(0);

  const hydrate = useCallback((loaded: T) => {
    // Ignored once the user has acted: the mount load is async, so a click can
    // land while it is in flight, and this read predates that click. There is
    // no rollback target to keep in step any more — the displayed value is
    // always a confirmed one — so dropping it outright is the whole handling.
    if (seqRef.current !== 0) return;
    setValue(loaded);
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
          // Superseded: the user has chosen again since. Neither the value nor
          // the toast belongs to the current state of the control any more.
          if (seq !== seqRef.current) return;
          if (ok) setValue(next);
          else toast.error("Couldn't save that setting — try again.");
        });
    },
    [save]
  );

  return { value, commit, hydrate };
}
