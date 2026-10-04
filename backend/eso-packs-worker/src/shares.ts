import type { Env, PackType, SharePackData, ShareCodeResponse, ValidationError } from "./types";
import { MAX_SHARES_PER_USER } from "./share-store";
import { corsHeaders } from "./cors";
import { readJsonBody, sanitizeAddons } from "./validate";

const CODE_PATTERN = /^[23456789ABCDEFGHJKMNPQRSTUVWXYZ]{6}$/;
const MAX_ADDONS = 200;
const MAX_NAME_LENGTH = 100;
const MAX_DESCRIPTION_LENGTH = 1000;
const VALID_TYPES = ["addon-pack", "build-pack", "roster-pack"];

const ESO_LOGS_API = "https://www.esologs.com/api/v2/user";
/** Workers' fetch has no default timeout, so a hung esologs.com connection
 *  would otherwise hold the whole invocation until the client gives up. */
const ESO_LOGS_TIMEOUT_MS = 10_000;

// ── Helpers ───────────────────────────────────────────────────────

function json(
  request: Request,
  data: unknown,
  status = 200,
  cacheMaxAge = 0,
  cacheScope: "public" | "private" = "private",
): Response {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...corsHeaders(request),
  };
  if (cacheMaxAge > 0) {
    headers["Cache-Control"] = `${cacheScope}, max-age=${cacheMaxAge}`;
  } else {
    headers["Cache-Control"] = "private, no-store";
  }
  return new Response(JSON.stringify(data), { status, headers });
}

export interface EsoLogsUser {
  id: number;
  name: string;
}

/**
 * Isolate-local memo of resolved tokens, keyed by a SHA-256 of the token so the
 * secret itself never sits in memory as a map key.
 *
 * Every authenticated operation used to hit esologs.com, so a single client
 * action (create → publish → list) cost three upstream round trips. Only
 * successful resolutions are memoized — a failure is never remembered, keeping
 * the fail-closed behaviour — and the TTL is short enough that a revoked token
 * stops working almost immediately.
 */
const TOKEN_CACHE_TTL_MS = 30_000;
const TOKEN_CACHE_LIMIT = 500;
const tokenCache = new Map<string, { user: EsoLogsUser; at: number }>();

/** Forget every memoized token identity. Tests resolve one token to several
 *  different identities, so they reset between cases. */
export function resetTokenCache(): void {
  tokenCache.clear();
}

async function tokenFingerprint(token: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export async function validateBearerToken(request: Request): Promise<EsoLogsUser | null> {
  const authHeader = request.headers.get("Authorization");
  if (!authHeader?.startsWith("Bearer ")) return null;

  const token = authHeader.slice(7);
  const fingerprint = await tokenFingerprint(token);
  const cached = tokenCache.get(fingerprint);
  if (cached && Date.now() - cached.at < TOKEN_CACHE_TTL_MS) {
    return cached.user;
  }

  try {
    const res = await fetch(ESO_LOGS_API, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({
        query: "{ userData { currentUser { id name } } }",
      }),
      signal: AbortSignal.timeout(ESO_LOGS_TIMEOUT_MS),
    });

    if (!res.ok) return null;

    const body = (await res.json()) as {
      data?: { userData?: { currentUser?: EsoLogsUser } };
    };
    const user = body.data?.userData?.currentUser ?? null;
    if (user) {
      if (tokenCache.size >= TOKEN_CACHE_LIMIT) tokenCache.clear();
      tokenCache.set(fingerprint, { user, at: Date.now() });
    }
    return user;
  } catch {
    return null;
  }
}

// ── Validation ───────────────────────────────────────────────────

function validateSharePayload(data: unknown): ValidationError[] {
  const errors: ValidationError[] = [];

  if (!data || typeof data !== "object" || Array.isArray(data)) {
    return [{ field: "body", message: "Body must be a JSON object" }];
  }

  const d = data as Record<string, unknown>;

  if (typeof d.title !== "string" || d.title.length === 0 || d.title.length > MAX_NAME_LENGTH) {
    errors.push({ field: "title", message: `title is required and must be 1-${MAX_NAME_LENGTH} characters` });
  }

  if (typeof d.description !== "string" || d.description.length > MAX_DESCRIPTION_LENGTH) {
    errors.push({ field: "description", message: `description must be a string under ${MAX_DESCRIPTION_LENGTH} characters` });
  }

  if (typeof d.packType !== "string" || !VALID_TYPES.includes(d.packType)) {
    errors.push({ field: "packType", message: `packType must be one of: ${VALID_TYPES.join(", ")}` });
  }

  if (!Array.isArray(d.tags)) {
    errors.push({ field: "tags", message: "tags must be an array" });
  } else if (d.tags.length > 10) {
    errors.push({ field: "tags", message: "tags must have at most 10 entries" });
  } else {
    for (let i = 0; i < d.tags.length; i++) {
      if (typeof d.tags[i] !== "string" || d.tags[i].length === 0 || d.tags[i].length > 50) {
        errors.push({ field: `tags[${i}]`, message: "each tag must be a non-empty string of at most 50 characters" });
        break;
      }
    }
  }

  if (!Array.isArray(d.addons) || d.addons.length === 0 || d.addons.length > MAX_ADDONS) {
    errors.push({ field: "addons", message: `addons must be an array with 1-${MAX_ADDONS} entries` });
  } else {
    for (let i = 0; i < d.addons.length; i++) {
      if (!d.addons[i] || typeof d.addons[i] !== "object" || Array.isArray(d.addons[i])) {
        errors.push({ field: `addons[${i}]`, message: "each addon must be a JSON object" });
        continue;
      }
      const addon = d.addons[i] as Record<string, unknown>;
      if (typeof addon.esouiId !== "number" || !Number.isInteger(addon.esouiId) || addon.esouiId <= 0) {
        errors.push({ field: `addons[${i}].esouiId`, message: "esouiId must be a positive number" });
      }
      if (typeof addon.name !== "string" || addon.name.length === 0 || addon.name.length > 200) {
        errors.push({ field: `addons[${i}].name`, message: "name is required and must be at most 200 characters" });
      }
      if (addon.note !== undefined && (typeof addon.note !== "string" || (addon.note as string).length > 500)) {
        errors.push({ field: `addons[${i}].note`, message: "note must be a string of at most 500 characters" });
      }
      if (typeof addon.required !== "boolean") {
        errors.push({ field: `addons[${i}].required`, message: "required must be a boolean" });
      }
      if (addon.defaultEnabled !== undefined && typeof addon.defaultEnabled !== "boolean") {
        errors.push({ field: `addons[${i}].defaultEnabled`, message: "defaultEnabled must be a boolean" });
      }
    }
  }

  return errors;
}

// ── Handlers ─────────────────────────────────────────────────────

export async function handleCreateShare(request: Request, env: Env): Promise<Response> {
  // Validate Bearer token
  const user = await validateBearerToken(request);
  if (!user) {
    return json(request, { error: "Invalid or missing authorization token" }, 401);
  }

  // Parse and validate body
  const parsed = await readJsonBody(request);
  if (!parsed.ok) {
    return parsed.reason === "too-large"
      ? json(request, { error: "Request body is too large" }, 413)
      : json(request, { error: "Invalid JSON" }, 400);
  }

  const errors = validateSharePayload(parsed.body);
  if (errors.length > 0) {
    return json(request, { error: "Validation failed", details: errors }, 400);
  }

  // Rebuild the payload from validated fields only. The record is served back
  // verbatim to anyone holding the code, so storing the raw body turned this
  // endpoint into an anonymously-readable blob host for whatever extra
  // properties the caller attached.
  const input = parsed.body as Record<string, unknown>;
  const packData: SharePackData = {
    title: input.title as string,
    description: input.description as string,
    packType: input.packType as PackType,
    tags: [...(input.tags as string[])],
    addons: sanitizeAddons(input.addons),
  };

  const index = env.PACK_INDEX.get(env.PACK_INDEX.idFromName("singleton"));
  const result = await index.createShare(user, packData);
  if (result.status === "limit") {
    return json(request, { error: `Maximum of ${MAX_SHARES_PER_USER} active share codes reached. Wait for existing codes to expire.` }, 429);
  }
  if (result.status !== "ok") {
    return json(request, { error: "Failed to generate unique share code. Please try again." }, 500);
  }
  const { code, expiresAt } = result.record;

  const response: ShareCodeResponse = {
    code,
    expiresAt,
    deepLink: `kalpa://share/${code}`,
  };

  return json(request, response, 201);
}

export async function handleResolveShare(request: Request, env: Env, code: string): Promise<Response> {
  // Validate code format
  if (!CODE_PATTERN.test(code)) {
    return json(request, { error: "Invalid share code format" }, 400);
  }

  const index = env.PACK_INDEX.get(env.PACK_INDEX.idFromName("singleton"));
  const record = await index.getShare(code);
  if (!record) {
    return json(request, { error: "Share code not found or expired" }, 404);
  }

  // The code grants public read access. Limit freshness to the remaining
  // lifetime; a cached response can outlive account deletion by up to 5 minutes.
  return json(request, {
    pack: record.pack,
    sharedBy: record.createdByName,
    sharedAt: record.createdAt,
    expiresAt: record.expiresAt,
  }, 200, Math.max(0, Math.min(300, Math.floor((Date.parse(record.expiresAt) - Date.now()) / 1000))), "public");
}
