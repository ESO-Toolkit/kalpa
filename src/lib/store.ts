import { load } from "@tauri-apps/plugin-store";

const STORE_PATH = "settings.json";

let storePromise: ReturnType<typeof load> | null = null;

function getStore() {
  if (!storePromise) {
    storePromise = load(STORE_PATH, { autoSave: true, defaults: {} }).catch((err) => {
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
    return true;
  } catch (err) {
    console.warn(`[store] Failed to write "${key}":`, err);
    return false;
  }
}
