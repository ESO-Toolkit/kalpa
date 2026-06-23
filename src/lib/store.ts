import { load } from "@tauri-apps/plugin-store";

const STORE_PATH = "settings.json";

let storePromise: ReturnType<typeof load> | null = null;

function getStore() {
  if (!storePromise) {
    // autoSave is OFF so persistence is fully explicit: every write below calls
    // save() itself. A debounced autosave could otherwise flush a key in the
    // middle of a multi-key batch (e.g. the forced-default migration marker
    // before its active-theme reset), leaving the store durably inconsistent.
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
  try {
    const store = await getStore();
    await store.set(key, value);
    await store.save();
    return true;
  } catch (err) {
    console.warn(`[store] Failed to write "${key}":`, err);
    return false;
  }
}

/** Persist several settings as one unit: every key is set in memory, then a
 * single explicit `save()` flushes the whole store file to disk. autoSave is
 * debounced (100ms), so the explicit save lands before any per-set autosave could
 * fire — making the batch atomic in the normal path, and a mid-batch crash
 * flushes nothing (clean retry next launch). On failure the in-memory cache is
 * restored to its exact pre-batch snapshot (set/delete per key), so a later
 * autosave can't flush a half-written batch — a `false` return means "nothing
 * changed." (store.reload() is unsuitable: it merges disk state and won't drop
 * keys this batch newly added.) Never throws. */
export async function setSettings(entries: Record<string, unknown>): Promise<boolean> {
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
        // Compare-and-restore: only roll back a key that STILL holds this batch's
        // attempted value. If a concurrent write changed it since, that newer
        // value wins — the rollback must not clobber an unrelated write.
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
}
