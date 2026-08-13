import { getSetting, setSetting, settingsWritesSettled } from "@/lib/store";

/**
 * The "dependencyPolicy" preference: how Kalpa treats the libraries an addon
 * declares in its manifest. Modelled on the existing `conflictPolicy` key —
 * same three-way shape, same "ask" deferral flow, same plain-string storage in
 * settings.json — so there is only one way preferences of this kind work.
 *
 *   auto — install required dependencies transitively (Kalpa's original behaviour).
 *   ask  — install nothing up front; the backend returns the first round of
 *          missing dependencies and the UI prompts. Whatever the user accepts is
 *          then installed transitively.
 *   skip — install nothing and look at nothing (no disk walk, no network).
 *
 * The STORED default is "ask": dependency installs are downloads the user never
 * asked for, so they are opt-in. That is deliberately NOT the same as the wire
 * default — an absent/unrecognised `dependencyPolicy` argument makes the Rust
 * side behave as "auto", which keeps every existing caller and test working.
 * Only code that has actually READ this preference may send a policy.
 */
export type DependencyPolicy = "auto" | "ask" | "skip";

/** settings.json key holding the {@link DependencyPolicy}. */
export const DEPENDENCY_POLICY_KEY = "dependencyPolicy";

/** settings.json key holding the declined-dependency list (see below). */
export const SKIPPED_DEPENDENCIES_KEY = "skippedDependencies";

/** settings.json key holding the {@link getAskRequiredDependenciesOnly} opt-in. */
export const ASK_REQUIRED_ONLY_KEY = "askRequiredDependenciesOnly";

export const DEFAULT_DEPENDENCY_POLICY: DependencyPolicy = "ask";

const POLICIES: readonly DependencyPolicy[] = ["auto", "ask", "skip"];

/** Narrow an untrusted value (settings.json is user-editable and survives
 * downgrades, so it can hold anything) to a DependencyPolicy. Anything
 * unrecognised falls back to the default rather than throwing. */
export function parseDependencyPolicy(value: unknown): DependencyPolicy {
  return POLICIES.includes(value as DependencyPolicy)
    ? (value as DependencyPolicy)
    : DEFAULT_DEPENDENCY_POLICY;
}

/**
 * How long a dependency read will wait for pending settings writes before
 * giving up and reading anyway.
 *
 * The settle wait exists to avoid reading a policy the user just replaced, but
 * it waits on the GLOBAL write chain — any unrelated settings write that never
 * settles (a `flush_settings` invoke that never returns) would otherwise hang
 * these reads forever, and with them the install path and the dependency
 * prompt. A stuck prompt is a worse failure than a stale read: it is the same
 * "required library never offered" outcome, with no toast and no way out.
 *
 * On timeout the read simply proceeds, which is exactly the behaviour before
 * the ordering was added — degraded, not broken. Generous enough that a normal
 * flush (a local temp+rename) is never close to it.
 */
const SETTLE_TIMEOUT_MS = 3000;

/** Wait for queued settings writes, but never longer than
 * {@link SETTLE_TIMEOUT_MS}. Never rejects; the timer is always cleared, so a
 * fast settle does not leave a pending handle behind. */
async function settledOrTimeout(): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([
      settingsWritesSettled(),
      new Promise<void>((resolve) => {
        timer = setTimeout(resolve, SETTLE_TIMEOUT_MS);
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/**
 * Read the persisted policy. Never throws — a degraded store yields the default.
 *
 * Ordered behind any settings write still in flight, like the native-upload
 * opt-out. The Settings radio writes fire-and-forget, and writes are queued, so
 * a policy chosen moments before an install could still be sitting behind
 * another write when this reads — handing Rust the OLD policy. Reading a stale
 * "skip" is the damaging direction: the backend then reports no pending
 * dependencies at all, so a missing required library is silently never offered,
 * moments after the user selected the setting meant to offer it.
 */
export async function getDependencyPolicy(): Promise<DependencyPolicy> {
  await settledOrTimeout();
  return parseDependencyPolicy(await getSetting<unknown>(DEPENDENCY_POLICY_KEY, undefined));
}

/** Persist the policy. Returns false when the write failed (see `setSetting`). */
export function setDependencyPolicy(policy: DependencyPolicy): Promise<boolean> {
  return setSetting(DEPENDENCY_POLICY_KEY, policy);
}

/**
 * Narrows the "ask" policy to required (`DependsOn`) libraries only, dropping
 * optional (`OptionalDependsOn`) entries before the picker ever opens.
 *
 * Scope, not action: it modifies what "ask" asks about and is meaningless under
 * the other two policies, which never surface an optional dependency in the
 * first place — "auto" installs required entries only (see
 * `resolve_transitive_deps` in commands.rs, which filters on `d.required`) and
 * "skip" installs nothing. The settings UI only offers it alongside "ask" for
 * that reason.
 *
 * Turning it on costs no discoverability: an addon's optional libraries are
 * listed permanently in its detail panel, each with its own Install button, so
 * the prompt was never the only way to find them.
 *
 * Defaults to false — today's behaviour, where the picker lists both groups and
 * optional entries simply arrive unticked.
 */
// Annotated rather than inferred, matching DEFAULT_DEPENDENCY_POLICY above: a
// bare `false` infers the literal type, which then narrows every consumer's
// generic to `false` and rejects ever setting it true.
export const DEFAULT_ASK_REQUIRED_ONLY: boolean = false;

/** Read the opt-in. Never throws; a degraded store or a non-boolean value
 * (settings.json is user-editable) yields the default. Ordered behind pending
 * writes for the same reason as `getDependencyPolicy` — this one decides
 * whether a prompt is suppressed, so a stale read suppresses the wrong thing. */
export async function getAskRequiredDependenciesOnly(): Promise<boolean> {
  await settledOrTimeout();
  const raw = await getSetting<unknown>(ASK_REQUIRED_ONLY_KEY, undefined);
  return typeof raw === "boolean" ? raw : DEFAULT_ASK_REQUIRED_ONLY;
}

/** Persist the opt-in. Returns false when the write failed (see `setSetting`). */
export function setAskRequiredDependenciesOnly(enabled: boolean): Promise<boolean> {
  return setSetting(ASK_REQUIRED_ONLY_KEY, enabled);
}

/**
 * Declined-dependency list: names the user has already said "no" to, so the
 * prompt does not nag about the same library on every subsequent install.
 * Stored as a plain string array under one settings key — no separate store.
 */

/** ESO addon folder names are compared case-insensitively (Windows paths are,
 * and manifests are inconsistent about casing), so every lookup normalises. */
function normalizeName(name: string): string {
  return name.trim().toLowerCase();
}

/** Read the declined list. Tolerates a malformed/legacy value by ignoring it. */
export async function getSkippedDependencies(): Promise<string[]> {
  const raw = await getSetting<unknown>(SKIPPED_DEPENDENCIES_KEY, undefined);
  if (!Array.isArray(raw)) return [];
  return raw.filter((n): n is string => typeof n === "string" && n.trim() !== "");
}

/** True when `name` appears in a previously-read declined list. Pure, so a
 * caller can filter a whole batch after a single store read. */
export function isDependencySkipped(name: string, skipped: readonly string[]): boolean {
  const key = normalizeName(name);
  return skipped.some((entry) => normalizeName(entry) === key);
}

/** Add names to the declined list, preserving their original casing and
 * dropping duplicates. Read-modify-write is safe here because the prompt this
 * serves is modal — only one picker can be resolving at a time. */
export async function addSkippedDependencies(names: readonly string[]): Promise<boolean> {
  const current = await getSkippedDependencies();
  const seen = new Set(current.map(normalizeName));
  const merged = [...current];
  for (const name of names) {
    const key = normalizeName(name);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    merged.push(name.trim());
  }
  // Nothing new: skip the disk flush and report success.
  if (merged.length === current.length) return true;
  return setSetting(SKIPPED_DEPENDENCIES_KEY, merged);
}

/** Forget every declined dependency (a "start prompting me again" reset). */
export function clearSkippedDependencies(): Promise<boolean> {
  return setSetting(SKIPPED_DEPENDENCIES_KEY, []);
}
