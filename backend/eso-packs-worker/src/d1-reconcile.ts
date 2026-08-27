import { ANONYMOUS_AUTHOR_NAME } from "./redact";
import type { Env, Pack } from "./types";

const LIMITS = { corpus: 2_000, upserts: 100, tags: 100, deletes: 25, emptyDeletes: 5, total: 150 };

export interface ReconciliationAuthority {
  packs: Pack[];
  tombstones: string[];
}
export interface D1PackRow {
  id: string;
  author_id: string;
  author_name: string;
  is_anonymous: number;
  title: string;
  description: string;
  pack_type: string;
  addons: string;
  vote_count: number;
}
interface D1TagRow {
  pack_id: string;
  tag: string;
}
export interface D1ReconciliationPlan {
  upserts: Pack[];
  tag_replacements: Pack[];
  deletes: string[];
  unowned_extra: string[];
}
type Mode = "off" | "dry-run" | "apply";
type Counts = { upserts: number; tag_replacements: number; deletes: number; total: number };
export interface D1ReconciliationResult {
  at: string;
  stage: "off" | "authority" | "d1-read" | "plan-rejected" | "apply" | "complete" | "no-d1";
  mode: Mode;
  mode_invalid?: string;
  authority_count: number;
  d1_count: number;
  planned: Counts;
  applied: Counts;
  unowned_extra: number;
  limit_hit?: string;
  message?: string;
}

export function toD1PackRow(pack: Pack): D1PackRow {
  return {
    id: pack.id,
    author_id: pack.author_id,
    author_name: pack.is_anonymous ? ANONYMOUS_AUTHOR_NAME : pack.author_name,
    is_anonymous: pack.is_anonymous ? 1 : 0,
    title: pack.title,
    description: pack.description,
    pack_type: pack.pack_type,
    addons: JSON.stringify(
      pack.addons.map(({ esouiId, name, required, note }) => ({ esouiId, name, required, note }))
    ),
    vote_count: pack.vote_count ?? 0,
  };
}
function rowsEqual(a: D1PackRow, b: D1PackRow): boolean {
  return (
    a.id === b.id &&
    a.author_id === b.author_id &&
    a.author_name === b.author_name &&
    Number(a.is_anonymous) === b.is_anonymous &&
    a.title === b.title &&
    a.description === b.description &&
    a.pack_type === b.pack_type &&
    a.addons === b.addons &&
    Number(a.vote_count) === b.vote_count
  );
}
function sameTags(a: string[], b: string[]): boolean {
  return [...a].sort().join("\0") === [...b].sort().join("\0");
}

export function buildD1ReconciliationPlan(
  authority: ReconciliationAuthority,
  d1Rows: D1PackRow[],
  d1Tags: D1TagRow[]
): D1ReconciliationPlan {
  const rows = new Map(d1Rows.map((row) => [row.id, row]));
  const tags = new Map<string, string[]>();
  for (const item of d1Tags) tags.set(item.pack_id, [...(tags.get(item.pack_id) ?? []), item.tag]);
  const expected = new Map(authority.packs.map((pack) => [pack.id, pack]));
  const owned = new Set([...expected.keys(), ...authority.tombstones]);
  const upserts: Pack[] = [],
    replacements: Pack[] = [],
    deletes = new Set<string>();
  for (const pack of [...authority.packs].sort((a, b) => a.id.localeCompare(b.id))) {
    const actual = rows.get(pack.id);
    if (pack.status !== "published") {
      if (actual) deletes.add(pack.id);
      continue;
    }
    if (!actual || !rowsEqual(actual, toD1PackRow(pack))) upserts.push(pack);
    if (!actual || !sameTags(tags.get(pack.id) ?? [], pack.tags)) replacements.push(pack);
  }
  const unowned = new Set<string>();
  for (const { id } of d1Rows) {
    if (expected.has(id)) continue;
    if (owned.has(id)) deletes.add(id);
    else unowned.add(id);
  }
  // Tags may survive a partially successful historical delete even when their
  // parent pack row is already gone. Reconcile those only with the same durable
  // ownership proof used for pack-row zombies.
  for (const { pack_id: id } of d1Tags) {
    const pack = expected.get(id);
    if (pack?.status === "published") continue;
    if (pack || owned.has(id)) deletes.add(id);
    else unowned.add(id);
  }
  return {
    upserts,
    tag_replacements: replacements,
    deletes: [...deletes].sort(),
    unowned_extra: [...unowned].sort(),
  };
}

function planCounts(plan: D1ReconciliationPlan): Counts {
  const value = {
    upserts: plan.upserts.length,
    tag_replacements: plan.tag_replacements.length,
    deletes: plan.deletes.length,
    total: 0,
  };
  value.total = value.upserts + value.tag_replacements + value.deletes;
  return value;
}
function zeros(): Counts {
  return { upserts: 0, tag_replacements: 0, deletes: 0, total: 0 };
}
function mode(raw?: string): { value: Mode; invalid?: string } {
  if (raw === "off" || raw === "dry-run" || raw === "apply") return { value: raw };
  return raw === undefined || raw === ""
    ? { value: "dry-run" }
    : { value: "dry-run", invalid: raw };
}
function validateAuthority(value: unknown): ReconciliationAuthority {
  if (!value || typeof value !== "object")
    throw new Error("Authority returned a non-object result");
  const result = value as Partial<ReconciliationAuthority>;
  if (!Array.isArray(result.packs) || !Array.isArray(result.tombstones))
    throw new Error("Authority result is incomplete");
  const ids = new Set<string>();
  for (const pack of result.packs) {
    if (
      !pack ||
      typeof pack.id !== "string" ||
      !pack.id ||
      ids.has(pack.id) ||
      typeof pack.title !== "string" ||
      typeof pack.description !== "string" ||
      !["addon-pack", "build-pack", "roster-pack"].includes(pack.pack_type) ||
      typeof pack.author_id !== "string" ||
      typeof pack.author_name !== "string" ||
      typeof pack.is_anonymous !== "boolean" ||
      !Array.isArray(pack.addons) ||
      pack.addons.some(
        (addon) =>
          !addon ||
          typeof addon.esouiId !== "number" ||
          typeof addon.name !== "string" ||
          typeof addon.required !== "boolean" ||
          (addon.note !== undefined && typeof addon.note !== "string")
      ) ||
      !Array.isArray(pack.tags) ||
      pack.tags.some((tag) => typeof tag !== "string") ||
      typeof pack.vote_count !== "number" ||
      typeof pack.install_count !== "number" ||
      typeof pack.created_at !== "string" ||
      typeof pack.updated_at !== "string" ||
      (pack.status !== "draft" && pack.status !== "published")
    ) {
      throw new Error("Authority returned a malformed pack");
    }
    ids.add(pack.id);
  }
  if (result.tombstones.some((id) => typeof id !== "string" || !id))
    throw new Error("Authority returned a malformed tombstone");
  return result as ReconciliationAuthority;
}
function limitHit(authorityCount: number, count: Counts): string | undefined {
  if (count.upserts > LIMITS.upserts) return "upserts";
  if (count.tag_replacements > LIMITS.tags) return "tag-replacements";
  if (count.deletes > LIMITS.deletes) return "deletes";
  if (count.total > LIMITS.total) return "total";
  if (authorityCount === 0 && count.deletes > LIMITS.emptyDeletes) return "empty-authority-deletes";
  if (authorityCount > 0 && count.deletes > Math.max(5, Math.ceil(authorityCount * 0.1)))
    return "delete-ratio";
  return undefined;
}
async function breadcrumb(env: Env, key: string, value: unknown): Promise<void> {
  try {
    await env.ESO_PACKS.put(key, JSON.stringify(value));
  } catch (error) {
    console.error(`Failed to write ${key}:`, error);
  }
}
export async function recordD1MirrorFailure(
  env: Env,
  op: string,
  packId: string,
  error: unknown
): Promise<void> {
  await breadcrumb(env, "d1-mirror:last_error", {
    at: new Date().toISOString(),
    op,
    pack_id: packId,
    message: error instanceof Error ? error.message : String(error),
  });
}
function initial(resolved: { value: Mode; invalid?: string }): D1ReconciliationResult {
  return {
    at: new Date().toISOString(),
    stage: "complete",
    mode: resolved.value,
    ...(resolved.invalid ? { mode_invalid: resolved.invalid } : {}),
    authority_count: 0,
    d1_count: 0,
    planned: zeros(),
    applied: zeros(),
    unowned_extra: 0,
  };
}
export async function reconcileD1(env: Env): Promise<D1ReconciliationResult> {
  const resolved = mode(env.D1_RECONCILIATION_MODE),
    result = initial(resolved);
  if (resolved.value === "off") {
    result.stage = "off";
    await breadcrumb(env, "d1-recon:last", result);
    return result;
  }
  if (!env.ROSTER_HUB_DB) {
    result.stage = "no-d1";
    await breadcrumb(env, "d1-recon:last", result);
    return result;
  }
  const stub = env.PACK_INDEX.get(env.PACK_INDEX.idFromName("singleton"));
  let authority: ReconciliationAuthority;
  try {
    authority = validateAuthority(await stub.getReconciliationState());
    result.authority_count = authority.packs.length;
    if (authority.packs.length > LIMITS.corpus)
      throw new Error(`Authority corpus exceeds ${LIMITS.corpus}`);
  } catch (error) {
    result.stage = "authority";
    result.message = error instanceof Error ? error.message : String(error);
    await breadcrumb(env, "d1-recon:last_error", result);
    return result;
  }
  let rows: D1PackRow[], tags: D1TagRow[];
  try {
    rows = (
      await env.ROSTER_HUB_DB.prepare(
        "SELECT id, author_id, author_name, is_anonymous, title, description, pack_type, addons, vote_count FROM packs"
      ).all<D1PackRow>()
    ).results;
    tags = (await env.ROSTER_HUB_DB.prepare("SELECT pack_id, tag FROM pack_tags").all<D1TagRow>())
      .results;
    result.d1_count = rows.length;
  } catch (error) {
    result.stage = "d1-read";
    result.message = error instanceof Error ? error.message : String(error);
    await breadcrumb(env, "d1-recon:last_error", result);
    return result;
  }
  const plan = buildD1ReconciliationPlan(authority, rows, tags);
  result.planned = planCounts(plan);
  result.unowned_extra = plan.unowned_extra.length;
  result.limit_hit = limitHit(authority.packs.length, result.planned);
  if (result.limit_hit) {
    result.stage = "plan-rejected";
    await breadcrumb(env, "d1-recon:last_error", result);
    return result;
  }
  if (resolved.value === "apply") {
    try {
      for (const pack of plan.upserts) {
        const replace = plan.tag_replacements.some(({ id }) => id === pack.id);
        const applied = await stub.reconcileWriteD1(pack.id, pack.created_at, true, replace);
        if (applied.upserted) {
          result.applied.upserts++;
          result.applied.total++;
        }
        if (applied.tags_replaced) {
          result.applied.tag_replacements++;
          result.applied.total++;
        }
      }
      const upsertIds = new Set(plan.upserts.map(({ id }) => id));
      for (const pack of plan.tag_replacements) {
        if (upsertIds.has(pack.id)) continue;
        const applied = await stub.reconcileWriteD1(pack.id, pack.created_at, false, true);
        if (applied.tags_replaced) {
          result.applied.tag_replacements++;
          result.applied.total++;
        }
      }
      for (const id of plan.deletes) {
        if (await stub.reconcileDeleteD1(id)) {
          result.applied.deletes++;
          result.applied.total++;
        }
      }
    } catch (error) {
      result.stage = "apply";
      result.message = error instanceof Error ? error.message : String(error);
      await breadcrumb(env, "d1-recon:last_error", result);
      return result;
    }
  }
  result.stage = "complete";
  await breadcrumb(env, "d1-recon:last", result);
  return result;
}
