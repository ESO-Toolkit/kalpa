import { load } from "@tauri-apps/plugin-store";
import { invoke } from "@tauri-apps/api/core";

const STORE_PATH = "settings.json";

/** The `flush_settings` command returns this error when it reloaded settings from
 * disk after the store had opened over a transiently-unreadable file. The plugin
 * cache has been replaced, so a batch's rollback snapshot is stale — skip the
 * rollback. Keep in sync with `STORE_RELOADED_SIGNAL` in
 * src-tauri/src/settings_store.rs. */
const STORE_RELOADED_SIGNAL = "settings-store-reloaded";

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

/** Structural equality for JSON-serializable values. Store reads come back as
 * freshly deserialized values over IPC, so reference equality (===) would treat
 * every object/array entry as changed — making the rollback guard below skip them.
 */
function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) {
    return false;
  }
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ak = Object.keys(a as object);
  const bk = Object.keys(b as object);
  if (ak.length !== bk.length) return false;
  return ak.every(
    (k) =>
      Object.prototype.hasOwnProperty.call(b, k) &&
      deepEqual((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k])
  );
}

function getStore() {
  if (!storePromise) {
    // autoSave is OFF so persistence is fully explicit: every write below flushes
    // via the `flush_settings` command (atomic write-temp + fsync + rename in
    // settings_store.rs) instead of the plugin's non-atomic save(). A debounced
    // autosave could otherwise flush a key in the middle of a multi-key batch
    // (e.g. the forced-default migration marker before its active-theme reset),
    // leaving the store durably inconsistent.
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

/** Like {@link getSetting} but distinguishes a genuinely-absent key (`ok: true`,
 * value = fallback) from a store READ FAILURE (`ok: false`). `getSetting` returns
 * its fallback in both cases, which is wrong for a security/trust-boundary read
 * (e.g. the native-upload opt-out): a degraded store would look like "not opted
 * out" and silently route the unofficial path. Such callers can fail CLOSED by
 * treating `ok: false` as opted-out. Never throws. */
export async function getSettingChecked<T>(
  key: string,
  fallback: T
): Promise<{ value: T; ok: boolean }> {
  try {
    const store = await getStore();
    const val = await store.get<T>(key);
    return { value: val ?? fallback, ok: true };
  } catch (err) {
    console.warn(`[store] Failed to read "${key}":`, err);
    return { value: fallback, ok: false };
  }
}

/** Persist a setting. Never throws; returns true on success, false on failure
 * (so callers can surface "couldn't save" instead of falsely reporting success).
 * `true` means the write was published crash-atomically (it survives a process
 * kill or app crash and never leaves a partial file); it does not guarantee the
 * change survives an OS-level power cut in the brief window after the write.
 *
 * Implemented as a one-key batch so a failed write rolls the in-memory cache back
 * to its prior value — otherwise the failed value would linger in the cache and a
 * later unrelated flush could persist it despite this call returning false. */
export async function setSetting<T>(key: string, value: T): Promise<boolean> {
  return setSettings({ [key]: value });
}

/** Persist several settings as one unit: serialized against all other writes (so
 * no other flush interleaves the batch), every key is set in memory, then a
 * single `flush_settings` invoke writes the whole store file to disk
 * all-or-nothing (atomic temp+rename). On failure the in-memory cache is
 * restored to its exact pre-batch snapshot (set/delete per key), so a later
 * flush can't write a half-written batch — a `false` return means "nothing
 * changed." (store.reload() is unsuitable: it merges disk state and won't drop
 * keys this batch newly added.) Never throws. */
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
      await invoke("flush_settings");
      return true;
    } catch (err) {
      if (String(err).includes(STORE_RELOADED_SIGNAL)) {
        // The store was reloaded from disk (it had opened over a transiently
        // unreadable file). The plugin cache was replaced, so this batch's rollback
        // snapshot is stale — skip rollback. The write didn't persist; report
        // failure and let the caller retry against the freshly loaded state.
        return false;
      }
      console.warn("[store] Failed to write batch:", err);
      try {
        const store = await getStore();
        for (const [key, had] of prior) {
          // Compare-and-restore: only roll back a key that STILL holds this
          // batch's attempted value. If a concurrent write changed it since, that
          // newer value wins — the rollback must not clobber an unrelated write.
          // Use structural equality: reads return fresh deserialized values, so
          // === would skip rollback for every object/array entry.
          const current = await store.get(key);
          if (!deepEqual(current, entries[key])) continue;
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
