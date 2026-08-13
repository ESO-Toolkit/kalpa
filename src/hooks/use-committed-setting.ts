import { useCallback, useRef, useState } from "react";
import { toast } from "sonner";

/**
 * A settings control whose displayed value may never outlive what is actually
 * on disk.
 *
 * The controls in Settings update optimistically and persist in the background,
 * which reads well but has a failure mode: the install and update paths read
 * the STORED preference, so a control showing one thing while settings.json
 * holds another makes Kalpa behave in a way the user can see is wrong and
 * cannot explain. For the dependency policy specifically, "ask" on screen with
 * "skip" on disk means a missing required library is silently never offered.
 *
 * Reverting to a value captured at click time is not enough. Two rapid clicks
 * put two writes in flight, and if both fail the second rolls back to the first
 * click's value — which never persisted either. So this tracks two things
 * instead:
 *
 *   - `committed` — the last value known to have reached disk. Rollback targets
 *     this, never a per-click snapshot, so it can only ever land on a value the
 *     store actually holds.
 *   - `seq` — a monotonic id per write. Only the newest write may roll the UI
 *     back; an older failure landing late has already been superseded on screen
 *     and must not drag it backwards.
 *
 * Deliberately not solved by disabling the control while a write is pending: a
 * write that never settles would lock the setting permanently, which is a worse
 * failure than a rare toast.
 */
export function useCommittedSetting<T>(
  initial: T,
  save: (value: T) => Promise<boolean>
): {
  /** Current value to render. */
  value: T;
  /** Optimistically apply and persist; rolls back on a failed write. */
  commit: (next: T) => void;
  /** Seed from storage on load — marks the value as already persisted. */
  hydrate: (loaded: T) => void;
} {
  const [value, setValue] = useState<T>(initial);
  const committedRef = useRef<T>(initial);
  const seqRef = useRef(0);

  const hydrate = useCallback((loaded: T) => {
    // A load is ground truth, so it both displays and counts as committed.
    // It does NOT bump `seq`: hydration happens once on mount, before any
    // click, and claiming a write id here would let it cancel a real one.
    committedRef.current = loaded;
    setValue(loaded);
  }, []);

  const commit = useCallback(
    (next: T) => {
      const seq = ++seqRef.current;
      setValue(next);
      void save(next).then((ok) => {
        if (ok) {
          // Any successful write is now the truth on disk, even if a newer
          // write is still in flight — that one will overwrite this on success
          // or roll back to it on failure.
          committedRef.current = next;
          return;
        }
        // Superseded: a later click owns the display now. Rolling back here
        // would undo a selection the user made after this one.
        if (seq !== seqRef.current) return;
        setValue(committedRef.current);
        toast.error("Couldn't save that setting — try again.");
      });
    },
    [save]
  );

  return { value, commit, hydrate };
}
