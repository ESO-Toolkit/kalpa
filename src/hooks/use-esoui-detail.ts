import { useCallback, useEffect, useRef, useState } from "react";
import type { EsouiAddonDetail } from "@/types";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";

/** How long a fetched detail stays fresh before a background refresh. */
const TTL_MS = 10 * 60 * 1000;

/** Session-only cache — cleared when the webview reloads. */
const cache = new Map<number, { detail: EsouiAddonDetail; fetchedAt: number }>();

/** In-flight requests, so rapid re-selection doesn't stack duplicate calls. */
const inFlight = new Map<number, Promise<EsouiAddonDetail>>();

function isFresh(entry: { fetchedAt: number } | undefined): boolean {
  return entry !== undefined && Date.now() - entry.fetchedAt < TTL_MS;
}

function fetchDetail(esouiId: number): Promise<EsouiAddonDetail> {
  const existing = inFlight.get(esouiId);
  if (existing) return existing;

  const request = invokeOrThrow<EsouiAddonDetail>("fetch_esoui_detail", { esouiId })
    .then((detail) => {
      cache.set(esouiId, { detail, fetchedAt: Date.now() });
      return detail;
    })
    .finally(() => {
      inFlight.delete(esouiId);
    });

  inFlight.set(esouiId, request);
  return request;
}

export interface UseEsouiDetail {
  detail: EsouiAddonDetail | null;
  loading: boolean;
  error: string | null;
  refetch: () => void;
}

interface State {
  /** Identifies the request this state belongs to; `null` while disabled. */
  key: string | null;
  detail: EsouiAddonDetail | null;
  loading: boolean;
  error: string | null;
}

/** The state a given request starts from, seeded from the cache when possible. */
function initialState(key: string | null, esouiId: number | undefined): State {
  if (key === null || esouiId === undefined) {
    return { key, detail: null, loading: false, error: null };
  }
  const cached = cache.get(esouiId);
  // Only show a spinner when there is nothing to display — a stale value
  // refreshes underneath the rendered detail instead of flashing a skeleton.
  return { key, detail: cached?.detail ?? null, loading: cached === undefined, error: null };
}

/**
 * Loads an ESOUI addon detail, shared across panes via a module-level cache.
 *
 * A cached value is served immediately; if it is past its TTL the refresh runs
 * in the background so the stale-but-good detail stays on screen.
 */
export function useEsouiDetail(esouiId: number | undefined, enabled: boolean): UseEsouiDetail {
  // Bumped by refetch() to re-run the effect while bypassing the TTL.
  const [reloadToken, setReloadToken] = useState(0);
  const forceRef = useRef(false);

  const active = enabled && esouiId !== undefined;
  const key = active ? `${esouiId}:${reloadToken}` : null;

  const [state, setState] = useState<State>(() => initialState(key, esouiId));

  // Resetting during render rather than from an effect: switching addons must
  // not paint one frame of the previous addon's detail, and a synchronous
  // setState in an effect body would cascade an extra render.
  if (state.key !== key) {
    setState(initialState(key, esouiId));
  }

  const refetch = useCallback(() => {
    forceRef.current = true;
    setReloadToken((token) => token + 1);
  }, []);

  useEffect(() => {
    const force = forceRef.current;
    forceRef.current = false;

    if (!active || esouiId === undefined) return;

    const cached = cache.get(esouiId);
    if (cached !== undefined && isFresh(cached) && !force) return;

    let disposed = false;
    // Scoped by key so a response that lands after the selection changed is
    // dropped instead of overwriting the newer addon's state.
    const settle = (patch: Partial<State>) => {
      if (disposed) return;
      setState((prev) => (prev.key === key ? { ...prev, ...patch } : prev));
    };

    void fetchDetail(esouiId)
      .then((detail) => settle({ detail, error: null, loading: false }))
      .catch((e) => {
        // Keep a stale detail on screen rather than replacing it with an error
        // panel — the user still has something usable.
        settle(
          cached === undefined
            ? { error: getTauriErrorMessage(e), loading: false }
            : { loading: false }
        );
      });

    return () => {
      disposed = true;
    };
  }, [esouiId, active, key, reloadToken]);

  return { detail: state.detail, loading: state.loading, error: state.error, refetch };
}
