import { load } from "@tauri-apps/plugin-store";

const STORE_PATH = "settings.json";

let storePromise: ReturnType<typeof load> | null = null;

/** Serializes all writes (set + save) so they never interleave. Because autoSave
 * is off, every write ends in an explicit save() that flushes the WHOLE store; if
 * a single write's save ran between a batch's per-key sets it could persist a
 * partial batch (e.g. the migration marker without its active-theme reset). The
 * mutex makes each setSetting/setSettings run to completion before the next. */
let writeChain: Promise<unknown> = Promise.resolve();

function enqueueWrite<T>(op: () => Promise<T>): Promise<T> {
  const run = writeChain.then(op, op);
  // Keep the chain alive regardless of this op's outcome (never reject the chain).
  writeChain = run.then(
    () => undefined,
    () => undefined
  );
  return run;
}

function getStore() {
  if (!storePromise) {
    // autoSave is OFF so persistence is fully explicit: every write below calls
    // save() itself. A debounced autosave could otherwise flush a key in the
    // middle of a multi-key batch (e.g. the forced-default migration marker
    // before its active-theme reset), leaving the store durably inconsistent.
    // NOTE: plugin-store caches one instance per path and the FIRST opener's
    // options win. The Rust side opens settings.json first (token_store.rs) and
    // also disables autosave, so this option must stay in sync with it.
    storePromise = load(STORE_PATH, { autoSave: false, defaults: {} }).catch((err) => {
      storePromise = null;
      throw err;
    });
  }
  return storePromise;
}

export async function getSetting<T>(key: string, fallback: T): Promise<T> {
  try {
    const store = await getStore();
    const val = await store.get<T>(key);
    return val ?? fallback;
  } catch (err) {
    console.warn(`[store] Failed to read "${key}":`, err);
    return fallback;
  }
}

/** Persist a setting. Never throws; returns true on success, false on failure
 * (so callers can surface "couldn't save" instead of falsely reporting success). */
export async function setSetting<T>(key: string, value: T): Promise<boolean> {
  return enqueueWrite(async () => {
    try {
      const store = await getStore();
      await store.set(key, value);
      await store.save();
      return true;
    } catch (err) {
      console.warn(`[store] Failed to write "${key}":`, err);
      return false;
    }
  });
}

/** Persist several settings as one unit: serialized against all other writes (so
 * no other save() interleaves the batch), every key is set in memory, then a
 * single explicit `save()` flushes the whole store file to disk all-or-nothing.
 * On failure the in-memory cache is restored to its exact pre-batch snapshot
 * (set/delete per key), so a later save can't flush a half-written batch — a
 * `false` return means "nothing changed." (store.reload() is unsuitable: it
 * merges disk state and won't drop keys this batch newly added.) Never throws. */
export async function setSettings(entries: Record<string, unknown>): Promise<boolean> {
  return enqueueWrite(async () => {
    const prior = new Map<string, unknown>();
    try {
      const store = await getStore();
      // Snapshot every key's prior value BEFORE mutating any, so the snapshot is
      // complete by the time the first set() runs.
      for (const key of Object.keys(entries)) {
        prior.set(key, await store.get(key));
      }
      for (const [key, value] of Object.entries(entries)) {
        await store.set(key, value);
      }
      await store.save();
      return true;
    } catch (err) {
      console.warn("[store] Failed to write batch:", err);
      try {
        const store = await getStore();
        for (const [key, had] of prior) {
          // Compare-and-restore: only roll back a key that STILL holds this
          // batch's attempted value. If a concurrent write changed it since, that
          // newer value wins — the rollback must not clobber an unrelated write.
          const current = await store.get(key);
          if (current !== entries[key]) continue;
          if (had === undefined) await store.delete(key);
          else await store.set(key, had);
        }
      } catch {
        /* best-effort rollback */
      }
      return false;
    }
  });
}
