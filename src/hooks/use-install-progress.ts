import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { InstallProgressEvent } from "@/types";
import { installProgressFromEvent, type InstallProgress } from "@/lib/install-progress";

export interface UseInstallProgress {
  /** Latest progress for the operation this hook started, or null when idle. */
  progress: InstallProgress | null;
  /** Mint an operation id to hand to `install_addon` / `install_dependency`. */
  beginOperation: () => string;
  /** Clear progress once the command settles (success, failure or cancel). */
  endOperation: () => void;
}

/**
 * Subscribes to `update-progress` for ONE install at a time and exposes the
 * latest phase/counts for it.
 *
 * Events are correlated by operation id, not by addon: a concurrent update
 * started elsewhere in the app emits on the same channel, and rendering its
 * bytes under a Discover row would be a lie.
 *
 * The backend already throttles (a byte stride while downloading, a file stride
 * while extracting), so the flood is bounded — but a large archive still emits
 * faster than a label can usefully change, so payloads are coalesced to one
 * setState per animation frame. The flush re-checks the operation id because a
 * frame can land after `endOperation` has already reset the UI (the command
 * resolving in the same frame, or the rAF frozen while the window was hidden)
 * and must not resurrect stale progress.
 */
export function useInstallProgress(): UseInstallProgress {
  const [progress, setProgress] = useState<InstallProgress | null>(null);
  const operationIdRef = useRef<string | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let pending: { progress: InstallProgress; opId: string } | null = null;
    let rafId: number | null = null;

    const flush = () => {
      rafId = null;
      if (pending !== null && pending.opId === operationIdRef.current) {
        setProgress(pending.progress);
      }
      pending = null;
    };

    void listen<InstallProgressEvent>("update-progress", (event) => {
      const { operationId } = event.payload;
      if (!operationId || operationId !== operationIdRef.current) return;
      pending = { progress: installProgressFromEvent(event.payload), opId: operationId };
      rafId ??= requestAnimationFrame(flush);
    })
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      })
      .catch((e) => console.error("[tauri:update-progress]", e));

    return () => {
      disposed = true;
      if (rafId !== null) cancelAnimationFrame(rafId);
      unlisten?.();
    };
  }, []);

  const beginOperation = useCallback(() => {
    const id = crypto.randomUUID();
    operationIdRef.current = id;
    setProgress(null);
    return id;
  }, []);

  const endOperation = useCallback(() => {
    operationIdRef.current = null;
    setProgress(null);
  }, []);

  return { progress, beginOperation, endOperation };
}
