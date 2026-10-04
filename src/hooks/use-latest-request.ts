import { useCallback, useEffect, useRef } from "react";

interface LatestRequestHandlers<T> {
  onSuccess?: (value: T) => void;
  onError?: (error: unknown) => void;
  onSettled?: () => void;
}

/** Runs overlapping refreshes while allowing only the newest to affect UI. */
export function useLatestRequest() {
  const sequenceRef = useRef(0);

  useEffect(
    () => () => {
      // Treat unmount like a newer request so late work cannot call component
      // setters or surface an error after its dialog has closed.
      sequenceRef.current += 1;
    },
    []
  );

  return useCallback(async function runLatest<T>(
    request: () => Promise<T>,
    handlers: LatestRequestHandlers<T>
  ): Promise<void> {
    const sequence = ++sequenceRef.current;
    try {
      const value = await request();
      if (sequence === sequenceRef.current) handlers.onSuccess?.(value);
    } catch (error) {
      if (sequence === sequenceRef.current) handlers.onError?.(error);
    } finally {
      if (sequence === sequenceRef.current) handlers.onSettled?.();
    }
  }, []);
}
