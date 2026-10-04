import { useCallback, useRef, useState } from "react";
import { toast } from "sonner";

type StateUpdate<T> = T | ((current: T) => T);

/**
 * Optimistically renders a persisted setting without allowing an older load or
 * failed write to overwrite a newer user intent. The store serializes writes,
 * so every success advances `confirmedRef` in durable order; a failed latest
 * operation rolls back to that confirmed value instead of inverting its input.
 */
export function useOptimisticSetting<T>(
  initial: T,
  save: (value: T) => Promise<boolean>,
  errorMessage = "Couldn't save that setting — try again."
): {
  value: T;
  commit: (update: StateUpdate<T>) => Promise<boolean>;
  hydrate: (loaded: T) => void;
} {
  const [value, setValue] = useState(initial);
  const valueRef = useRef(initial);
  const confirmedRef = useRef(initial);
  const operationRef = useRef(0);
  const pendingRef = useRef(new Set<number>());
  const writeConfirmedRef = useRef(false);

  const hydrate = useCallback((loaded: T) => {
    // This mount-time read predates every user operation. It is still the best
    // rollback target while writes are pending, but can never replace a value
    // that a newer write has already confirmed.
    if (writeConfirmedRef.current) return;
    confirmedRef.current = loaded;
    if (pendingRef.current.size === 0) {
      valueRef.current = loaded;
      setValue(loaded);
    }
  }, []);

  const commit = useCallback(
    async (update: StateUpdate<T>) => {
      const operation = ++operationRef.current;
      const next =
        typeof update === "function" ? (update as (current: T) => T)(valueRef.current) : update;
      pendingRef.current.add(operation);
      valueRef.current = next;
      setValue(next);

      const ok = await save(next).catch(() => false);
      pendingRef.current.delete(operation);
      if (ok) {
        confirmedRef.current = next;
        writeConfirmedRef.current = true;
        return true;
      }

      if (operation === operationRef.current) {
        valueRef.current = confirmedRef.current;
        setValue(confirmedRef.current);
        toast.error(errorMessage);
      }
      return false;
    },
    [errorMessage, save]
  );

  return { value, commit, hydrate };
}
