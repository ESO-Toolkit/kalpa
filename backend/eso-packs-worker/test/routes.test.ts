import { env } from "cloudflare:workers";
import { createExecutionContext, createScheduledController, waitOnExecutionContext } from "cloudflare:test";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import worker, {
  invalidatePackListCache,
  RESTORE_MAX_PAGE_SIZE,
  SUBREQUESTS_PER_RECORD,
  SUBREQUEST_CEILING,
  SUBREQUEST_RESERVE,
} from "../src/index";
import { putPack, putVote } from "../src/kv";
import { resetTokenCache } from "../src/shares";
import type { Env, Pack, PackIndex, VoteRecord } from "../src/types";
import {
  TEST_USER,
  OTHER_USER,
  esoLogsResponse,
  esoLogsUnauthorized,
  validPackBody,
  authedRequest,
  apiKeyRequest,
  makePack,
} from "./helpers";

const BASE = "https://kalpa-pack-hub.eso-toolkit.workers.dev";
const e = env as unknown as Env;

function packIndexForTest() {
  return e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton"));
}

async function ensureD1MirrorTables(): Promise<void> {
  await e.ROSTER_HUB_DB!.prepare(
    "CREATE TABLE IF NOT EXISTS packs (id TEXT PRIMARY KEY, author_id TEXT, author_name TEXT, is_anonymous INTEGER, title TEXT, description TEXT, pack_type TEXT, addons TEXT, vote_count INTEGER, created_at TEXT, updated_at TEXT)"
  ).run();
  await e.ROSTER_HUB_DB!.prepare(
    "CREATE TABLE IF NOT EXISTS pack_tags (pack_id TEXT, tag TEXT)"
  ).run();
}

async function putPackIndex(testEnv: Env, index: PackIndex): Promise<void> {
  const id = testEnv.PACK_INDEX.idFromName("singleton");
  await testEnv.PACK_INDEX.get(id).replaceIndex(index);
}

async function getCanonicalIndex(testEnv: Env): Promise<PackIndex> {
  const id = testEnv.PACK_INDEX.idFromName("singleton");
  return testEnv.PACK_INDEX.get(id).getIndex();
}

let fetchSpy: ReturnType<typeof vi.fn>;
const originalFetch = globalThis.fetch;

beforeEach(async () => {
  fetchSpy = vi.fn((input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (url.includes("esologs.com")) {
      return Promise.resolve(esoLogsResponse(TEST_USER));
    }
    return originalFetch(input);
  });
  globalThis.fetch = fetchSpy as typeof fetch;
  // These cases resolve the same memoized token to different identities.
  resetTokenCache();
  await packIndexForTest().cancelActiveRestoreJob();
  await putPackIndex(e, { packs: [] });
  await invalidatePackListCache(new URL(BASE));
});

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

async function call(request: Request) {
  const ctx = createExecutionContext();
  const response = await worker.fetch(request, e, ctx);
  await waitOnExecutionContext(ctx);
  return response;
}

// ── Health ─────────────────────────────────────────────────────────

describe("GET /health", () => {
  it("returns ok status", async () => {
    const res = await call(new Request(`${BASE}/health`));
    expect(res.status).toBe(200);
    const body = await res.json<{ status: string; kv: boolean }>();
    expect(body.status).toBe("ok");
    expect(body.kv).toBe(true);
  });

  it("does not expose corpus size or backup timing publicly", async () => {
    await e.ESO_PACKS.put("backup:meta", JSON.stringify({ last_success: Date.now() }));
    await putPackIndex(e, { packs: [makePack("private-health-detail")] });

    const body = await (await call(new Request(`${BASE}/health`))).json<Record<string, unknown>>();
    expect(body).not.toHaveProperty("packCount");
    expect(body).not.toHaveProperty("last_backup_at");
    expect(body).not.toHaveProperty("last_backup_ok");
  });
});

// ── 404 ───────────────────────────────────────────────────────────

describe("unknown routes", () => {
  it("returns 404", async () => {
    const res = await call(new Request(`${BASE}/nonexistent`));
    expect(res.status).toBe(404);
  });
});

// ── OPTIONS ───────────────────────────────────────────────────────

describe("OPTIONS preflight", () => {
  it("returns 204", async () => {
    const res = await call(new Request(BASE, { method: "OPTIONS" }));
    expect(res.status).toBe(204);
  });
});

// ── GET /packs ────────────────────────────────────────────────────

describe("GET /packs", () => {
  it("does not populate an isolate-unsafe manual Cache API entry", async () => {
    const key = new Request(`${BASE}/packs?default=1`);
    await caches.default.delete(key);
    await call(new Request(`${BASE}/packs?sort=votes&page=1`));
    expect(await caches.default.match(key)).toBeUndefined();
  });

  it("keeps omitted-sort and votes-sort wire results distinct", async () => {
    await putPackIndex(e, {
      packs: [
        makePack("recent-low", { vote_count: 1, updated_at: "2026-08-26T00:00:00.000Z" }),
        makePack("old-high", { vote_count: 10, updated_at: "2026-01-01T00:00:00.000Z" }),
      ],
    });
    const omitted = await (await call(new Request(`${BASE}/packs`)))
      .json<{ packs: Array<{ id: string }>; sort: string }>();
    const votes = await (await call(new Request(`${BASE}/packs?sort=votes&page=1`)))
      .json<{ packs: Array<{ id: string }>; sort: string }>();
    expect([omitted.sort, omitted.packs[0].id]).toEqual(["updated", "recent-low"]);
    expect([votes.sort, votes.packs[0].id]).toEqual(["votes", "old-high"]);
  });

  it("returns empty list when no index", async () => {
    const res = await call(new Request(`${BASE}/packs`));
    expect(res.status).toBe(200);
    const body = await res.json<{ packs: unknown[]; page: number }>();
    expect(body.packs).toEqual([]);
    expect(body.page).toBe(1);
  });

  it("returns packs from index", async () => {
    const index: PackIndex = {
      packs: [makePack("pack-a"), makePack("pack-b")],
    };
    await putPackIndex(e, index);

    const res = await call(new Request(`${BASE}/packs`));
    const body = await res.json<{ packs: unknown[] }>();
    expect(body.packs).toHaveLength(2);
  });

  it("filters by type", async () => {
    await putPackIndex(e, {
      packs: [
        makePack("a", { pack_type: "addon-pack" }),
        makePack("b", { pack_type: "build-pack" }),
      ],
    });

    const res = await call(new Request(`${BASE}/packs?type=build-pack`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs).toHaveLength(1);
    expect(body.packs[0].id).toBe("b");
  });

  it("filters by search query", async () => {
    await putPackIndex(e, {
      packs: [makePack("a", { title: "PvP Build" }), makePack("b", { title: "Healing Setup" })],
    });

    const res = await call(new Request(`${BASE}/packs?q=pvp`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs).toHaveLength(1);
    expect(body.packs[0].id).toBe("a");
  });

  it("hides draft packs by default", async () => {
    await putPackIndex(e, {
      packs: [makePack("pub", { status: "published" }), makePack("drft", { status: "draft" })],
    });

    // Use author filter to bypass CDN cache from prior tests
    const authorId = String(TEST_USER.id);
    const res = await call(new Request(`${BASE}/packs?author=${authorId}`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs).toHaveLength(1);
    expect(body.packs[0].id).toBe("pub");
  });

  it("sorts by popular", async () => {
    await putPackIndex(e, {
      packs: [makePack("low", { vote_count: 1 }), makePack("high", { vote_count: 10 })],
    });

    const res = await call(new Request(`${BASE}/packs?sort=popular`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs[0].id).toBe("high");
  });

  it("sorts by votes (client default) by vote_count desc", async () => {
    // Use distinct updated_at to prove it is NOT falling through to updated_at order.
    await putPackIndex(e, {
      packs: [
        makePack("low", { vote_count: 1, updated_at: "2025-12-01T00:00:00.000Z" }),
        makePack("high", { vote_count: 10, updated_at: "2025-01-01T00:00:00.000Z" }),
      ],
    });

    const res = await call(new Request(`${BASE}/packs?sort=votes`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs[0].id).toBe("high");
  });

  it("sorts by newest by created_at desc", async () => {
    await putPackIndex(e, {
      packs: [
        makePack("older", { created_at: "2025-01-01T00:00:00.000Z" }),
        makePack("newer", { created_at: "2025-06-01T00:00:00.000Z" }),
      ],
    });

    const res = await call(new Request(`${BASE}/packs?sort=newest`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs[0].id).toBe("newer");
  });

  it("sorts by installs by install_count desc", async () => {
    await putPackIndex(e, {
      packs: [makePack("few", { install_count: 2 }), makePack("many", { install_count: 99 })],
    });

    const res = await call(new Request(`${BASE}/packs?sort=installs`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs[0].id).toBe("many");
  });

  it("paginates results", async () => {
    const packs = Array.from({ length: 25 }, (_, i) => makePack(`p-${i}`));
    await putPackIndex(e, { packs });

    const page1 = await call(new Request(`${BASE}/packs?page=1`));
    const body1 = await page1.json<{ packs: unknown[] }>();
    expect(body1.packs).toHaveLength(20);

    const page2 = await call(new Request(`${BASE}/packs?page=2`));
    const body2 = await page2.json<{ packs: unknown[] }>();
    expect(body2.packs).toHaveLength(5);
  });

  it("reports user_voted per pack for a signed-in viewer", async () => {
    await putPackIndex(e, {
      packs: [makePack("list-voted"), makePack("list-unvoted")],
    });
    await putVote(e, "list-voted", String(TEST_USER.id));

    const res = await call(authedRequest(`${BASE}/packs`));
    const body = await res.json<{
      packs: Array<{ id: string; user_voted?: boolean }>;
    }>();
    expect(body.packs.find((p) => p.id === "list-voted")!.user_voted).toBe(true);
    expect(body.packs.find((p) => p.id === "list-unvoted")!.user_voted).toBe(false);
    expect(res.headers.get("Cache-Control")).toBe("public, max-age=0");
    expect(res.headers.get("Vary")).toContain("Authorization");
  });

  it("omits user_voted for anonymous callers", async () => {
    await putPackIndex(e, { packs: [makePack("list-anon")] });

    const res = await call(new Request(`${BASE}/packs`));
    const body = await res.json<{ packs: Array<Record<string, unknown>> }>();
    expect(body.packs[0].user_voted).toBeUndefined();
    expect(res.headers.get("Cache-Control")).toBe("public, max-age=30");
  });

  it("does not cache an anonymous fallback for a failed bearer lookup", async () => {
    await putPackIndex(e, { packs: [makePack("list-auth-outage")] });
    fetchSpy.mockResolvedValueOnce(esoLogsUnauthorized());

    const res = await call(authedRequest(`${BASE}/packs`));

    expect(res.status).toBe(200);
    expect(res.headers.get("Cache-Control")).toBe("public, max-age=0");
  });
});

// ── POST /packs ───────────────────────────────────────────────────

describe("POST /packs", () => {
  it("returns retryable 503 and resumes the same create after a KV mirror failure", async () => {
    const originalPut = e.ESO_PACKS.put.bind(e.ESO_PACKS);
    const put = vi.spyOn(e.ESO_PACKS, "put").mockRejectedValueOnce(
      new Error("injected route detail put failure"),
    );
    const request = () => authedRequest(`${BASE}/packs`, {
      method: "POST",
      body: JSON.stringify(validPackBody({ id: "w1-route-create-retry" })),
    });

    const first = await call(request());
    expect(first.status).toBe(503);
    expect(first.headers.get("Retry-After")).toBe("5");
    const canonical = await getCanonicalIndex(e);
    const createdAt = canonical.packs.find(({ id }) => id === "w1-route-create-retry")!.created_at;
    put.mockImplementation(originalPut);

    const retry = await call(request());
    expect(retry.status).toBe(201);
    const body = await retry.json<{ pack: { created_at: string } }>();
    expect(body.pack.created_at).toBe(createdAt);
  });

  it("creates a pack with auth", async () => {
    // Reset index so prior tests' packs don't trigger the 25-pack-per-user limit
    await putPackIndex(e, { packs: [] });
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody()),
      })
    );
    expect(res.status).toBe(201);
    const body = await res.json<{ pack: { id: string; title: string; author_id: string } }>();
    expect(body.pack.title).toBe("Test Pack");
    expect(body.pack.author_id).toBe(String(TEST_USER.id));
  });

  it("rejects without auth", async () => {
    fetchSpy.mockImplementation((input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (url.includes("esologs.com")) return Promise.resolve(esoLogsUnauthorized());
      return originalFetch(input);
    });

    const res = await call(
      new Request(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody()),
      })
    );
    expect(res.status).toBe(401);
  });

  it("rejects invalid payload", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify({ title: "" }),
      })
    );
    expect(res.status).toBe(400);
  });

  it("generates id from title slug", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody({ title: "My Cool Pack!" })),
      })
    );
    const body = await res.json<{ pack: { id: string } }>();
    expect(body.pack.id).toMatch(/^my-cool-pack/);
  });

  it("honors a requested published status instead of forcing draft", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody({ title: "Published On Create", status: "published" })),
      })
    );
    expect(res.status).toBe(201);
    const body = await res.json<{ pack: { id: string; status: string } }>();
    expect(body.pack.status).toBe("published");

    // A draft would be invisible to everyone but its author.
    const detail = await call(new Request(`${BASE}/packs/${body.pack.id}`));
    expect(detail.status).toBe(200);
  });

  it("defaults to draft when no status is requested", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody({ title: "No Status Given" })),
      })
    );
    const body = await res.json<{ pack: { status: string } }>();
    expect(body.pack.status).toBe("draft");
  });

  it("falls back to a usable id when the title slugifies to nothing", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody({ title: "日本語のパック" })),
      })
    );
    expect(res.status).toBe(201);
    const body = await res.json<{ pack: { id: string } }>();
    expect(body.pack.id).not.toBe("");
    // Must satisfy the /packs/:id route pattern, or the pack is unreachable.
    expect(body.pack.id).toMatch(/^[a-z0-9-]+$/);
  });

  it("strips unknown properties from addon entries", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(
          validPackBody({
            title: "Sanitized Addons",
            addons: [
              {
                esouiId: 7,
                name: "Addon",
                required: true,
                note: "keep me",
                junk: "x".repeat(2000),
              },
            ],
          })
        ),
      })
    );
    expect(res.status).toBe(201);
    const body = await res.json<{ pack: { addons: Record<string, unknown>[] } }>();
    expect(body.pack.addons[0]).toEqual({
      esouiId: 7,
      name: "Addon",
      required: true,
      note: "keep me",
    });
  });

  it("rejects an oversized body before parsing it", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify({ junk: "x".repeat(300_000) }),
      })
    );
    expect(res.status).toBe(413);
  });

  it("enforces the per-user pack limit against the authoritative index", async () => {
    const packs = Array.from({ length: 25 }, (_, i) => makePack(`quota-${i}`));
    await putPackIndex(e, { packs });

    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody({ title: "One Too Many" })),
      })
    );
    expect(res.status).toBe(429);
  });

  it("returns one 409 when concurrent creates request the same id", async () => {
    const body = JSON.stringify(validPackBody({ id: "same-slug", title: "Same Slug" }));
    const responses = await Promise.all([
      call(authedRequest(`${BASE}/packs`, { method: "POST", body })),
      call(authedRequest(`${BASE}/packs`, { method: "POST", body })),
    ]);

    expect(responses.map(({ status }) => status).sort()).toEqual([201, 409]);
    expect((await e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton")).getIndex()).packs
      .filter(({ id }) => id === "same-slug")).toHaveLength(1);
  });
});

// ── GET /packs/:id ────────────────────────────────────────────────

describe("GET /packs/:id", () => {
  it("returns a pack", async () => {
    await putPackIndex(e, { packs: [makePack("get-test")] });
    const res = await call(new Request(`${BASE}/packs/get-test`));
    expect(res.status).toBe(200);
    const body = await res.json<{ pack: { id: string } }>();
    expect(body.pack.id).toBe("get-test");
  });

  it("returns 404 for missing pack", async () => {
    const res = await call(new Request(`${BASE}/packs/nope`));
    expect(res.status).toBe(404);
  });

  it("hides draft pack from unauthenticated user", async () => {
    await putPackIndex(e, { packs: [makePack("draft-test", { status: "draft" })] });

    fetchSpy.mockImplementation((input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (url.includes("esologs.com")) return Promise.resolve(esoLogsUnauthorized());
      return originalFetch(input);
    });

    const res = await call(new Request(`${BASE}/packs/draft-test`));
    expect(res.status).toBe(404);
  });

  it("shows draft pack to authenticated user", async () => {
    await putPackIndex(e, { packs: [makePack("draft-visible", { status: "draft" })] });
    const res = await call(authedRequest(`${BASE}/packs/draft-visible`));
    expect(res.status).toBe(200);
  });

  it("reports user_voted for a signed-in viewer who already voted", async () => {
    await putPackIndex(e, { packs: [makePack("voted-detail")] });
    await putVote(e, "voted-detail", String(TEST_USER.id));

    const res = await call(authedRequest(`${BASE}/packs/voted-detail`));
    const body = await res.json<{ pack: { user_voted: boolean } }>();
    expect(body.pack.user_voted).toBe(true);
    // Per-viewer state must never be cached.
    expect(res.headers.get("Cache-Control")).toBe("public, max-age=0");
    expect(res.headers.get("Vary")).toContain("Authorization");
  });

  it("reports user_voted false for a signed-in viewer who has not voted", async () => {
    await putPackIndex(e, { packs: [makePack("unvoted-detail")] });
    const res = await call(authedRequest(`${BASE}/packs/unvoted-detail`));
    const body = await res.json<{ pack: { user_voted: boolean } }>();
    expect(body.pack.user_voted).toBe(false);
  });

  it("omits user_voted for anonymous viewers and stays cacheable", async () => {
    await putPackIndex(e, { packs: [makePack("anon-detail")] });
    const res = await call(new Request(`${BASE}/packs/anon-detail`));
    const body = await res.json<{ pack: Record<string, unknown> }>();
    expect(body.pack.user_voted).toBeUndefined();
    expect(res.headers.get("Cache-Control")).toContain("max-age=300");
  });

  it("does not cache redacted detail after a failed bearer lookup", async () => {
    await putPackIndex(e, { packs: [makePack("detail-auth-outage")] });
    fetchSpy.mockResolvedValueOnce(esoLogsUnauthorized());

    const res = await call(authedRequest(`${BASE}/packs/detail-auth-outage`));

    expect(res.status).toBe(200);
    expect(res.headers.get("Cache-Control")).toBe("public, max-age=0");
  });
});

// ── Anonymity enforcement ─────────────────────────────────────────

describe("anonymous pack redaction", () => {
  const anon = () => makePack("anon-pack", { is_anonymous: true, title: "Secret Pack" });
  const named = () => makePack("named-pack");

  it("redacts author fields of anonymous packs in the list", async () => {
    await putPackIndex(e, { packs: [anon(), named()] });

    // sort=updated is a non-default view, so the worker cache never interferes.
    const res = await call(new Request(`${BASE}/packs?sort=updated`));
    const body = await res.json<{
      packs: Array<{ id: string; author_name: string; author_id: string }>;
    }>();
    const anonOut = body.packs.find((p) => p.id === "anon-pack")!;
    const namedOut = body.packs.find((p) => p.id === "named-pack")!;
    expect(anonOut.author_name).toBe("Anonymous");
    expect(anonOut.author_id).toBe("");
    expect(namedOut.author_name).toBe(TEST_USER.name);
    expect(namedOut.author_id).toBe(String(TEST_USER.id));
  });

  it("excludes anonymous packs from ?author= for unauthenticated callers", async () => {
    await putPackIndex(e, { packs: [anon(), named()] });

    const res = await call(new Request(`${BASE}/packs?author=${TEST_USER.id}`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs.map((p) => p.id)).toEqual(["named-pack"]);
  });

  it("excludes anonymous packs from ?author= for a different authenticated user", async () => {
    await putPackIndex(e, { packs: [anon(), named()] });

    fetchSpy.mockImplementation((input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (url.includes("esologs.com")) return Promise.resolve(esoLogsResponse(OTHER_USER));
      return originalFetch(input);
    });

    const res = await call(authedRequest(`${BASE}/packs?author=${TEST_USER.id}`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs.map((p) => p.id)).toEqual(["named-pack"]);
  });

  it("shows the author their own anonymous packs with real fields via ?author=", async () => {
    await putPackIndex(e, { packs: [anon(), named()] });

    const res = await call(authedRequest(`${BASE}/packs?author=${TEST_USER.id}`));
    const body = await res.json<{
      packs: Array<{ id: string; author_name: string; author_id: string }>;
    }>();
    const anonOut = body.packs.find((p) => p.id === "anon-pack")!;
    expect(anonOut.author_name).toBe(TEST_USER.name);
    expect(anonOut.author_id).toBe(String(TEST_USER.id));
  });

  it("redacts an anonymous pack's detail for unauthenticated callers and keeps it cacheable", async () => {
    await putPackIndex(e, { packs: [anon()] });
    const res = await call(new Request(`${BASE}/packs/anon-pack`));
    expect(res.status).toBe(200);
    const body = await res.json<{
      pack: { author_name: string; author_id: string };
    }>();
    expect(body.pack.author_name).toBe("Anonymous");
    expect(body.pack.author_id).toBe("");
    expect(res.headers.get("Cache-Control")).toContain("max-age=300");
  });

  it("returns real fields to the author on detail, uncached", async () => {
    await putPackIndex(e, { packs: [anon()] });
    const res = await call(authedRequest(`${BASE}/packs/anon-pack`));
    expect(res.status).toBe(200);
    const body = await res.json<{
      pack: { author_name: string; author_id: string };
    }>();
    expect(body.pack.author_name).toBe(TEST_USER.name);
    expect(body.pack.author_id).toBe(String(TEST_USER.id));
    expect(res.headers.get("Cache-Control")).toBe("public, max-age=0");
    expect(res.headers.get("Vary")).toContain("Authorization");
  });
});

// ── PUT /packs/:id ────────────────────────────────────────────────

describe("PUT /packs/:id", () => {
  it("updates own pack", async () => {
    const pack = makePack("update-me");
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const res = await call(
      authedRequest(`${BASE}/packs/update-me`, {
        method: "PUT",
        body: JSON.stringify(validPackBody({ title: "Updated Title" })),
      })
    );
    expect(res.status).toBe(200);
    const body = await res.json<{ pack: { title: string } }>();
    expect(body.pack.title).toBe("Updated Title");
  });

  it("uses DO-owned counters when the detail body read by update is stale", async () => {
    const stale = makePack("update-stale-counter", { vote_count: 0 });
    await putPackIndex(e, { packs: [{ ...stale, vote_count: 1 }] });
    await putPack(e, stale);

    const res = await call(
      authedRequest(`${BASE}/packs/update-stale-counter`, {
        method: "PUT",
        body: JSON.stringify(validPackBody({ title: "Fresh content" })),
      }),
    );

    expect(res.status).toBe(200);
    const body = await res.json<{ pack: { title: string; vote_count: number } }>();
    expect(body.pack).toMatchObject({ title: "Fresh content", vote_count: 1 });
    const detail = await e.ESO_PACKS.get<{ vote_count: number }>(
      "pack:update-stale-counter",
      "json",
    );
    expect(detail!.vote_count).toBe(1);
  });

  it("rejects update by different user", async () => {
    const pack = makePack("not-mine", { author_id: String(OTHER_USER.id) });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const res = await call(
      authedRequest(`${BASE}/packs/not-mine`, {
        method: "PUT",
        body: JSON.stringify(validPackBody()),
      })
    );
    expect(res.status).toBe(403);
  });

  it("rejects a stale former owner after the slug is recreated", async () => {
    const stale = makePack("reused-update");
    const canonical = makePack(stale.id, {
      author_id: String(OTHER_USER.id),
      author_name: OTHER_USER.name,
      created_at: "2026-08-26T00:00:00.000Z",
    });
    await putPackIndex(e, { packs: [canonical] });
    await putPack(e, stale);

    const res = await call(
      authedRequest(`${BASE}/packs/${stale.id}`, {
        method: "PUT",
        body: JSON.stringify(validPackBody()),
      }),
    );

    expect(res.status).toBe(403);
    expect(await e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton")).getPack(stale.id))
      .toMatchObject({ author_id: String(OTHER_USER.id) });
  });
});

// ── DELETE /packs/:id ─────────────────────────────────────────────

describe("DELETE /packs/:id", () => {
  it("returns retryable 503 and resumes detail cleanup after delete commits", async () => {
    const pack = makePack("w1-route-delete-retry");
    await putPackIndex(e, { packs: [pack] });
    const originalDelete = e.ESO_PACKS.delete.bind(e.ESO_PACKS);
    const remove = vi.spyOn(e.ESO_PACKS, "delete").mockRejectedValueOnce(
      new Error("injected route detail delete failure"),
    );

    const first = await call(
      authedRequest(`${BASE}/packs/${pack.id}`, { method: "DELETE" }),
    );
    expect(first.status).toBe(503);
    expect(first.headers.get("Retry-After")).toBe("5");
    expect((await call(new Request(`${BASE}/packs/${pack.id}`))).status).toBe(404);
    remove.mockImplementation(originalDelete);

    const retry = await call(
      authedRequest(`${BASE}/packs/${pack.id}`, { method: "DELETE" }),
    );
    expect(retry.status).toBe(200);
    expect(await e.ESO_PACKS.get(`pack:${pack.id}`)).toBeNull();
  });

  it("deletes own pack", async () => {
    const pack = makePack("delete-me");
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const res = await call(authedRequest(`${BASE}/packs/delete-me`, { method: "DELETE" }));
    expect(res.status).toBe(200);
    const body = await res.json<{ ok: boolean }>();
    expect(body.ok).toBe(true);
  });

  it("rejects delete by different user", async () => {
    const pack = makePack("not-mine-del", { author_id: String(OTHER_USER.id) });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const res = await call(authedRequest(`${BASE}/packs/not-mine-del`, { method: "DELETE" }));
    expect(res.status).toBe(403);
  });

  it("rejects deletion by a stale former owner after the slug is recreated", async () => {
    const stale = makePack("reused-delete");
    const canonical = makePack(stale.id, {
      author_id: String(OTHER_USER.id),
      author_name: OTHER_USER.name,
      created_at: "2026-08-26T00:00:00.000Z",
    });
    await putPackIndex(e, { packs: [canonical] });
    await putPack(e, stale);

    const res = await call(authedRequest(`${BASE}/packs/${stale.id}`, { method: "DELETE" }));

    expect(res.status).toBe(403);
    expect(await e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton")).getPack(stale.id))
      .toMatchObject({ author_id: String(OTHER_USER.id) });
  });

  it("returns 404 for nonexistent pack", async () => {
    const res = await call(authedRequest(`${BASE}/packs/ghost`, { method: "DELETE" }));
    expect(res.status).toBe(404);
  });

  it("deletes the pack's vote records so a recycled id starts clean", async () => {
    const pack = makePack("recyclable");
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });
    await putVote(e, "recyclable", String(TEST_USER.id));

    const res = await call(authedRequest(`${BASE}/packs/recyclable`, { method: "DELETE" }));
    expect(res.status).toBe(200);

    expect(await e.ESO_PACKS.get(`vote:recyclable:${TEST_USER.id}`)).toBeNull();
    expect(await e.ESO_PACKS.get(`user-votes:${TEST_USER.id}:recyclable`)).toBeNull();
  });
});

// ── POST /packs/:id/vote ──────────────────────────────────────────

describe("POST /packs/:id/vote", () => {
  it("toggles vote on then off", async () => {
    const pack = makePack("votable", { vote_count: 0 });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const vote1 = await call(authedRequest(`${BASE}/packs/votable/vote`, { method: "POST" }));
    const body1 = await vote1.json<{ voted: boolean; voteCount: number }>();
    expect(body1.voted).toBe(true);
    expect(body1.voteCount).toBe(1);

    const vote2 = await call(authedRequest(`${BASE}/packs/votable/vote`, { method: "POST" }));
    const body2 = await vote2.json<{ voted: boolean; voteCount: number }>();
    expect(body2.voted).toBe(false);
    expect(body2.voteCount).toBe(0);
  });

  it("requires auth", async () => {
    await putPackIndex(e, { packs: [makePack("noauth-vote")] });

    fetchSpy.mockImplementation((input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (url.includes("esologs.com")) return Promise.resolve(esoLogsUnauthorized());
      return originalFetch(input);
    });

    const res = await call(new Request(`${BASE}/packs/noauth-vote/vote`, { method: "POST" }));
    expect(res.status).toBe(401);
  });

  it("404s on a draft pack instead of revealing it via 401", async () => {
    await putPackIndex(e, { packs: [makePack("draft-vote", { status: "draft" })] });

    const anonymous = await call(new Request(`${BASE}/packs/draft-vote/vote`, { method: "POST" }));
    expect(anonymous.status).toBe(404);

    const authed = await call(authedRequest(`${BASE}/packs/draft-vote/vote`, { method: "POST" }));
    expect(authed.status).toBe(404);
  });

  it("does not double-apply a rapid vote/unvote", async () => {
    const pack = makePack("rapid-toggle", { vote_count: 0 });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    for (let i = 0; i < 4; i++) {
      await call(authedRequest(`${BASE}/packs/rapid-toggle/vote`, { method: "POST" }));
    }

    // vote, unvote, vote, unvote — the record is gone and the counter is back
    // to where it started, regardless of how stale the KV read was.
    const final = await call(authedRequest(`${BASE}/packs/rapid-toggle/vote`, { method: "POST" }));
    const body = await final.json<{ voted: boolean; voteCount: number }>();
    expect(body.voted).toBe(true);
    expect(body.voteCount).toBe(1);
  });

  it("does not let a vote that read stale detail state recreate a deleted pack", async () => {
    const pack = makePack("delete-while-voting");
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    let releaseVote: (() => void) | undefined;
    fetchSpy.mockImplementation((input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (!url.includes("esologs.com")) return originalFetch(input);
      const token = new Headers(init?.headers).get("Authorization");
      if (token === "Bearer slow-voter") {
        return new Promise<Response>((resolve) => {
          releaseVote = () => resolve(esoLogsResponse(OTHER_USER));
        });
      }
      return Promise.resolve(esoLogsResponse(TEST_USER));
    });

    const vote = call(
      new Request(`${BASE}/packs/delete-while-voting/vote`, {
        method: "POST",
        headers: { Authorization: "Bearer slow-voter" },
      }),
    );
    await vi.waitFor(() => expect(releaseVote).toBeTypeOf("function"));

    const deleted = await call(
      authedRequest(`${BASE}/packs/delete-while-voting`, { method: "DELETE" }),
    );
    expect(deleted.status).toBe(200);
    releaseVote!();

    expect((await vote).status).toBe(404);
    expect((await e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton")).getIndex()).packs
      .some(({ id }) => id === pack.id)).toBe(false);
    expect(await e.ESO_PACKS.get(`pack:${pack.id}`)).toBeNull();
    expect(await e.ESO_PACKS.get(`vote:${pack.id}:${OTHER_USER.id}`)).toBeNull();
  });
});

// ── POST /packs/:id/install ───────────────────────────────────────

describe("POST /packs/:id/install", () => {
  it("honors a live limiter key written by the previous release", async () => {
    const pack = makePack("legacy-install-limit", { install_count: 4 });
    const ip = "4.3.2.1";
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });
    await e.ESO_PACKS.put(`install-rate:${pack.id}:${ip}`, "1", { expirationTtl: 3600 });

    const res = await call(new Request(`${BASE}/packs/${pack.id}/install`, {
      method: "POST",
      headers: { "CF-Connecting-IP": ip },
    }));

    expect(await res.json<{ installCount: number }>()).toEqual({ installCount: 4 });
    expect(await packIndexForTest().getPack(pack.id)).toMatchObject({ install_count: 4 });
  });

  it("does not let a legacy limiter expose a tombstoned stale detail", async () => {
    const pack = makePack("legacy-install-deleted", { install_count: 4 });
    const ip = "4.3.2.2";
    await putPackIndex(e, { packs: [pack] });
    await packIndexForTest().removePack(pack.id);
    await putPack(e, pack);
    await e.ESO_PACKS.put(`install-rate:${pack.id}:${ip}`, "1", { expirationTtl: 3600 });

    const res = await call(new Request(`${BASE}/packs/${pack.id}/install`, {
      method: "POST",
      headers: { "CF-Connecting-IP": ip },
    }));

    expect(res.status).toBe(404);
  });

  it("does not carry a legacy limiter into a recreated slug lifecycle", async () => {
    const oldPack = makePack("legacy-install-recreated", { install_count: 9 });
    const newPack = makePack(oldPack.id, {
      install_count: 0,
      created_at: "2026-08-27T00:00:00.000Z",
      updated_at: "2026-08-27T00:00:00.000Z",
    });
    const ip = "4.3.2.3";
    await putPackIndex(e, { packs: [oldPack] });
    await packIndexForTest().removePack(oldPack.id);
    await packIndexForTest().addPack(newPack);
    await putPack(e, oldPack);
    await e.ESO_PACKS.put(`install-rate:${oldPack.id}:${ip}`, "1", { expirationTtl: 3600 });

    const res = await call(new Request(`${BASE}/packs/${oldPack.id}/install`, {
      method: "POST",
      headers: { "CF-Connecting-IP": ip },
    }));

    expect(await res.json<{ installCount: number }>()).toEqual({ installCount: 1 });
    expect(await packIndexForTest().getPack(oldPack.id)).toMatchObject({
      created_at: newPack.created_at,
      install_count: 1,
    });
  });

  it("increments install count", async () => {
    const pack = makePack("installable", { install_count: 0 });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const res = await call(
      new Request(`${BASE}/packs/installable/install`, {
        method: "POST",
        headers: { "CF-Connecting-IP": "1.2.3.4" },
      })
    );
    expect(res.status).toBe(200);
    const body = await res.json<{ installCount: number }>();
    expect(body.installCount).toBe(1);
  });

  it("rate limits same IP", async () => {
    const pack = makePack("rate-limited", { install_count: 0 });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    await call(
      new Request(`${BASE}/packs/rate-limited/install`, {
        method: "POST",
        headers: { "CF-Connecting-IP": "5.6.7.8" },
      })
    );

    const res2 = await call(
      new Request(`${BASE}/packs/rate-limited/install`, {
        method: "POST",
        headers: { "CF-Connecting-IP": "5.6.7.8" },
      })
    );
    const body2 = await res2.json<{ installCount: number }>();
    // Second call returns current count without incrementing
    expect(body2.installCount).toBe(1);
  });

  it("does not double-count concurrent requests from the same IP", async () => {
    const pack = makePack("concurrent-install", { install_count: 0 });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });
    const request = () => new Request(`${BASE}/packs/${pack.id}/install`, {
      method: "POST",
      headers: { "CF-Connecting-IP": "6.7.8.9" },
    });

    const responses = await Promise.all([call(request()), call(request())]);
    const bodies = await Promise.all(responses.map((response) => response.json<{ installCount: number }>()));

    expect(bodies.map(({ installCount }) => installCount)).toEqual([1, 1]);
    expect(await packIndexForTest().getPack(pack.id)).toMatchObject({ install_count: 1 });
  });

  it("404s on a draft pack rather than bumping and disclosing its count", async () => {
    const pack = makePack("draft-install", { status: "draft", install_count: 0 });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const res = await call(
      new Request(`${BASE}/packs/draft-install/install`, {
        method: "POST",
        headers: { "CF-Connecting-IP": "9.9.9.9" },
      })
    );
    expect(res.status).toBe(404);

    const stored = await e.ESO_PACKS.get<{ install_count: number }>("pack:draft-install", "json");
    expect(stored!.install_count).toBe(0);
  });
});

// ── POST /admin/seed ──────────────────────────────────────────────

describe("POST /admin/seed", () => {
  it("seeds with valid API key", async () => {
    const res = await call(apiKeyRequest(`${BASE}/admin/seed`, { method: "POST" }));
    expect(res.status).toBe(200);
    const body = await res.json<{ ok: boolean; seeded: number }>();
    expect(body.ok).toBe(true);
    expect(body.seeded).toBeGreaterThan(0);
  });

  it("rejects without API key", async () => {
    const res = await call(new Request(`${BASE}/admin/seed`, { method: "POST" }));
    expect(res.status).toBe(401);
  });
});

// ── POST /admin/restore ─────────────────────────────────────────────

describe("POST /admin/migration/authority", () => {
  it("rejects an oversized adjudication without changing authority", async () => {
    const index = packIndexForTest();
    await index.setAuthority("kv", []);
    const body = JSON.stringify({
      authority: "do",
      unowned_d1_ids: Array.from({ length: 100 }, (_, i) => `website-${i}`),
      padding: "x".repeat(256_000),
    });

    const res = await call(apiKeyRequest(`${BASE}/admin/migration/authority`, {
      method: "POST",
      body,
    }));

    expect(res.status).toBe(413);
    expect((await index.getReconciliationState()).authority).toBe("kv");
  });

  it("requires explicit adjudication before ignoring a shared-D1-only witness", async () => {
    await e.ROSTER_HUB_DB!.prepare(
      "CREATE TABLE IF NOT EXISTS packs (id TEXT PRIMARY KEY, author_id TEXT, author_name TEXT, is_anonymous INTEGER, title TEXT, description TEXT, pack_type TEXT, addons TEXT, vote_count INTEGER, created_at TEXT, updated_at TEXT)"
    ).run();
    await e.ROSTER_HUB_DB!.prepare(
      "CREATE TABLE IF NOT EXISTS pack_tags (pack_id TEXT, tag TEXT)"
    ).run();
    await e.ROSTER_HUB_DB!.prepare("DELETE FROM pack_tags").run();
    await e.ROSTER_HUB_DB!.prepare("DELETE FROM packs").run();
    await e.ROSTER_HUB_DB!.prepare(
      "INSERT INTO packs VALUES ('website-only', 'website', 'Website', 0, 'Website row', '', 'addon-pack', '[]', 0, datetime('now'), datetime('now'))"
    ).run();
    const details = await e.ESO_PACKS.list({ prefix: "pack:" });
    for (const { name } of details.keys) await e.ESO_PACKS.delete(name);
    await e.ESO_PACKS.delete("backup:latest");
    await putPackIndex(e, { packs: [] });

    const blocked = await call(
      apiKeyRequest(`${BASE}/admin/migration/authority`, {
        method: "POST",
        body: JSON.stringify({ authority: "do" }),
      })
    );
    expect(blocked.status).toBe(409);

    const adjudicated = await call(
      apiKeyRequest(`${BASE}/admin/migration/authority`, {
        method: "POST",
        body: JSON.stringify({ authority: "do", unowned_d1_ids: ["website-only"] }),
      })
    );
    expect(adjudicated.status).toBe(200);

    await e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton")).setAuthority("kv", []);
    await e.ROSTER_HUB_DB!.prepare("DELETE FROM pack_tags").run();
    await e.ROSTER_HUB_DB!.prepare("DELETE FROM packs").run();
  });
});

describe("POST /admin/restore", () => {
  it("rejects without API key", async () => {
    const res = await call(new Request(`${BASE}/admin/restore`, { method: "POST" }));
    expect(res.status).toBe(401);
  });

  it("rejects an oversized admin body before buffering it", async () => {
    const res = await call(apiKeyRequest(`${BASE}/admin/restore`, {
      method: "POST",
      body: JSON.stringify({ ignored: "😀".repeat(70_000) }),
    }));
    expect(res.status).toBe(413);
  });

  it("404s when the requested backup snapshot doesn't exist", async () => {
    await e.ESO_PACKS.delete("backup:latest");
    const res = await call(apiKeyRequest(`${BASE}/admin/restore`, { method: "POST" }));
    expect(res.status).toBe(404);
  });

  it("does not replay a vote for a pack absent from the snapshot corpus", async () => {
    const live = makePack("restore-live-vote");
    const orphanPackId = "restore-deleted-vote";
    const orphanUserId = "restore-orphan-voter";
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: new Date().toISOString(),
        packs: [live],
        packBodies: { [live.id]: live },
        votes: {
          [`${orphanPackId}:${orphanUserId}`]: {
            packId: orphanPackId,
            userId: orphanUserId,
            votedAt: "2026-08-27T00:00:00.000Z",
          },
        },
      }),
    );

    const res = await call(apiKeyRequest(`${BASE}/admin/restore`, { method: "POST" }));
    expect(res.status).toBe(200);
    expect(await e.ESO_PACKS.get(`vote:${orphanPackId}:${orphanUserId}`)).toBeNull();
    expect(await e.ESO_PACKS.get(`user-votes:${orphanUserId}:${orphanPackId}`)).toBeNull();
  });

  it("restores packs and index from a backup snapshot", async () => {
    const pack = makePack("restore-me");
    const snapshot = {
      created_at: new Date().toISOString(),
      packs: [pack],
      packBodies: { [pack.id]: pack },
      votes: {
        [`${pack.id}:${TEST_USER.id}`]: {
          userId: String(TEST_USER.id),
          packId: pack.id,
          votedAt: "2025-01-01T00:00:00.000Z",
        },
      },
    };
    await e.ESO_PACKS.put("backup:latest", JSON.stringify(snapshot));

    // Wipe current state so the test proves restore repopulates it.
    await e.ESO_PACKS.delete(`pack:${pack.id}`);
    await putPackIndex(e, { packs: [] });

    const res = await call(apiKeyRequest(`${BASE}/admin/restore`, { method: "POST" }));
    expect(res.status).toBe(200);
    const body = await res.json<{
      ok: boolean;
      restored_packs: number;
      restored_votes: number;
    }>();
    expect(body.ok).toBe(true);
    expect(body.restored_packs).toBe(1);
    expect(body.restored_votes).toBe(1);

    const restoredPack = await e.ESO_PACKS.get(`pack:${pack.id}`, "json");
    expect(restoredPack).toEqual(pack);

    const restoredVote = await e.ESO_PACKS.get(`vote:${pack.id}:${TEST_USER.id}`);
    expect(restoredVote).toBeTruthy();
  });

  it("pages a snapshot larger than one call and resumes from the cursor", async () => {
    // A restore used to walk the whole snapshot in one request, two subrequests
    // per pack, strictly serialized — so it fell over at exactly the corpus size
    // where an incident recovery matters, with no way to resume.
    const packs = Array.from({ length: 5 }, (_, i) => makePack(`paged-${i}`));
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: new Date().toISOString(),
        packs,
        packBodies: Object.fromEntries(packs.map((p) => [p.id, p])),
        votes: {},
      })
    );

    for (const pack of packs) await e.ESO_PACKS.delete(`pack:${pack.id}`);
    await putPackIndex(e, { packs: [] });

    const first = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 2 }),
      })
    );
    expect(first.status).toBe(200);
    const firstBody = await first.json<{
      done: boolean;
      cursor: number;
      token: string;
      total: number;
      restored_packs: number;
    }>();
    expect(firstBody.done).toBe(false);
    expect(firstBody.cursor).toBe(2);
    expect(firstBody.total).toBe(5);
    expect(firstBody.restored_packs).toBe(2);
    expect(firstBody.token).toBeTruthy();

    // The index must NOT have been swapped yet: publishing a half-restored
    // corpus would be worse than the drift the restore is repairing.
    const midIndex = await getCanonicalIndex(e);
    expect(midIndex?.packs ?? []).toHaveLength(0);
    expect(await e.ESO_PACKS.get(`pack:${packs[0]!.id}`)).toBeTruthy();
    expect(await e.ESO_PACKS.get(`pack:${packs[4]!.id}`)).toBeNull();
    expect((await call(new Request(`${BASE}/packs/${packs[0]!.id}`))).status).toBe(404);
    expect(
      await e.PACK_INDEX.get(e.PACK_INDEX.idFromName("singleton")).getPack(packs[0]!.id),
    ).toBeNull();

    let cursor: number | null = firstBody.cursor;
    let token = firstBody.token;
    let guard = 0;
    while (cursor !== null && guard++ < 10) {
      const res = await call(
        apiKeyRequest(`${BASE}/admin/restore`, {
          method: "POST",
          body: JSON.stringify({ limit: 2, cursor, token }),
        })
      );
      expect(res.status).toBe(200);
      const body = await res.json<{ done: boolean; cursor: number | null; token: string }>();
      cursor = body.done ? null : body.cursor;
      if (body.token) token = body.token;
    }

    // Only now is the whole corpus live and indexed.
    for (const pack of packs) {
      expect(await e.ESO_PACKS.get(`pack:${pack.id}`)).toBeTruthy();
    }
    const finalIndex = await getCanonicalIndex(e);
    expect((finalIndex?.packs ?? []).map((p) => p.id).sort()).toEqual(
      packs.map((p) => p.id).sort()
    );
  });

  it("resumes from the staged snapshot when backup:latest changes mid-restore", async () => {
    // The daily cron overwrites backup:latest at midnight UTC, so a paged
    // restore straddling midnight silently changes snapshots mid-run. Applying
    // the old offset to the new work list skips every record before it — and
    // the final page would still publish an index listing packs whose bodies
    // were never written.
    const packs = Array.from({ length: 4 }, (_, i) => makePack(`stale-${i}`));
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: "2026-01-01T00:00:00.000Z",
        packs,
        packBodies: Object.fromEntries(packs.map((p) => [p.id, p])),
        votes: {},
      })
    );
    for (const pack of packs) await e.ESO_PACKS.delete(`pack:${pack.id}`);
    await putPackIndex(e, { packs: [] });

    const first = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 2 }),
      })
    );
    const { cursor, token } = await first.json<{ cursor: number; token: string }>();

    // The snapshot is replaced underneath, exactly as the cron would.
    const replacement = Array.from({ length: 4 }, (_, i) => makePack(`fresh-${i}`));
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: "2026-01-02T00:00:00.000Z",
        packs: replacement,
        packBodies: Object.fromEntries(replacement.map((p) => [p.id, p])),
        votes: {},
      })
    );

    const resumed = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 2, cursor, token }),
      })
    );
    expect(resumed.status).toBe(200);
    const resumedBody = await resumed.json<{ done: boolean }>();
    expect(resumedBody.done).toBe(true);

    // Nothing from the replacement snapshot was written; the restore kept using
    // its staged snapshot.
    expect(await e.ESO_PACKS.get(`pack:${replacement[0]!.id}`)).toBeNull();
    const index = await getCanonicalIndex(e);
    expect((index?.packs ?? []).map((p) => p.id).sort()).toEqual(
      packs.map((p) => p.id).sort(),
    );
  });

  it("does not rate-limit an authenticated admin restore across many pages", async () => {
    // WRITE_LIMITER allows 10 writes/minute per IP and runs before routing, so a
    // paged restore — one POST per page — used to 429 partway through and never
    // reach the final page that swaps the index. That breaks exactly the
    // large-corpus recovery the paging exists for.
    const packs = Array.from({ length: 14 }, (_, i) => makePack(`limiter-${i}`));
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: "2026-07-01T00:00:00.000Z",
        packs,
        packBodies: Object.fromEntries(packs.map((p) => [p.id, p])),
        votes: {},
      })
    );
    for (const pack of packs) await e.ESO_PACKS.delete(`pack:${pack.id}`);
    await putPackIndex(e, { packs: [] });

    const ip = "203.0.113.7";
    const page = (body: Record<string, unknown>) =>
      call(
        apiKeyRequest(`${BASE}/admin/restore`, {
          method: "POST",
          headers: { "CF-Connecting-IP": ip },
          body: JSON.stringify(body),
        })
      );

    // 14 records at one per page = 14 sequential POSTs, comfortably past the
    // 10/minute write limit.
    let res = await page({ limit: 1 });
    expect(res.status).toBe(200);
    let state = await res.json<{ done: boolean; cursor: number; token: string }>();
    let pages = 1;
    while (!state.done && pages < 30) {
      res = await page({ limit: 1, cursor: state.cursor, token: state.token });
      expect(res.status, `page ${pages + 1} was rejected`).toBe(200);
      state = await res.json<{ done: boolean; cursor: number; token: string }>();
      pages++;
    }
    expect(state.done, "restore never completed").toBe(true);
    expect(pages).toBeGreaterThan(10);

    const index = await getCanonicalIndex(e);
    expect((index?.packs ?? []).length).toBe(packs.length);
  });

  it("still rate-limits an admin path without a valid key", async () => {
    // The exemption is for AUTHENTICATED admins only — it must not become a
    // way for an anonymous caller to sidestep the limiter by path prefix.
    const ip = "203.0.113.9";
    const unauthed = () =>
      call(
        new Request(`${BASE}/admin/restore`, {
          method: "POST",
          headers: { "CF-Connecting-IP": ip },
        })
      );
    // Accepting "401 or 429" proves nothing: handleRestore returns 401 as its
    // first act, so an unauthenticated caller gets 401 whether or not the
    // limiter counted it — the assertion passed identically with the auth check
    // deleted from the exemption, which is the regression it exists to catch.
    // WRITE_LIMITER allows 10 a minute, so drive past it and demand the 429.
    // Only a request that was actually COUNTED can produce one.
    const statuses: number[] = [];
    for (let i = 0; i < 12; i++) {
      statuses.push((await unauthed()).status);
    }

    expect(statuses[0]).toBe(401);
    expect(
      statuses,
      `an unauthenticated /admin/ POST must still be rate-limited, got ${statuses.join(",")}`
    ).toContain(429);
  });

  it("refuses a valid token paired with a cursor it was not issued for", async () => {
    // The token used to fingerprint only the snapshot, so it validated ANY
    // in-range cursor. A mistyped offset — or a 409 retried with the
    // expected_token the handler used to echo back — could jump the middle of
    // the corpus and still publish an index for bodies never replayed.
    const packs = Array.from({ length: 6 }, (_, i) => makePack(`skip-${i}`));
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: "2026-05-01T00:00:00.000Z",
        packs,
        packBodies: Object.fromEntries(packs.map((p) => [p.id, p])),
        votes: {},
      })
    );
    for (const pack of packs) await e.ESO_PACKS.delete(`pack:${pack.id}`);
    await putPackIndex(e, { packs: [] });

    const first = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 2 }),
      })
    );
    const { cursor, token } = await first.json<{ cursor: number; token: string }>();
    expect(cursor).toBe(2);

    // Same snapshot, genuine token, but jump to the last page.
    const skipped = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ cursor: 4, token, limit: 2 }),
      })
    );
    expect(skipped.status).toBe(409);
    // And the 409 must not hand back a token that would make the retry work.
    const body = await skipped.json<Record<string, unknown>>();
    expect(body.expected_token).toBeUndefined();
    expect(body.expected_cursor).toBe(2);

    // Records 2..3 were never written and the index was never published.
    expect(await e.ESO_PACKS.get(`pack:${packs[3]!.id}`)).toBeNull();
    const index = await getCanonicalIndex(e);
    expect(index?.packs ?? []).toHaveLength(0);
  });

  it("resumes a DATED snapshot from cursor and token alone", async () => {
    // The paged response carries cursor and token but not `date`, and the docs
    // say to pass the response straight back — so a dated restore used to fall
    // through to backup:latest on page 2 and 409 against its own token. A dated
    // multi-page restore is exactly the incident-recovery case.
    const packs = Array.from({ length: 4 }, (_, i) => makePack(`dated-${i}`));
    await e.ESO_PACKS.put(
      "backup:2026-02-14",
      JSON.stringify({
        created_at: "2026-02-14T00:00:00.000Z",
        packs,
        packBodies: Object.fromEntries(packs.map((p) => [p.id, p])),
        votes: {},
      })
    );
    // A DIFFERENT latest snapshot, so falling through to it is unmistakable.
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: "2026-06-01T00:00:00.000Z",
        packs: [makePack("wrong-snapshot")],
        packBodies: { "wrong-snapshot": makePack("wrong-snapshot") },
        votes: {},
      })
    );
    for (const pack of packs) await e.ESO_PACKS.delete(`pack:${pack.id}`);
    await e.ESO_PACKS.delete("pack:wrong-snapshot");
    await putPackIndex(e, { packs: [] });

    const first = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ date: "2026-02-14", limit: 2 }),
      })
    );
    expect(first.status).toBe(200);
    let { cursor, token, done } = await first.json<{
      cursor: number;
      token: string;
      done: boolean;
    }>();
    expect(done).toBe(false);

    // Only cursor + token, exactly what the response hands back.
    let guard = 0;
    while (!done && guard++ < 10) {
      const res = await call(
        apiKeyRequest(`${BASE}/admin/restore`, {
          method: "POST",
          body: JSON.stringify({ cursor, token, limit: 2 }),
        })
      );
      expect(res.status).toBe(200);
      const body = await res.json<{ done: boolean; cursor: number; token: string }>();
      done = body.done;
      if (!done) {
        cursor = body.cursor;
        token = body.token;
      }
    }
    expect(done).toBe(true);

    // The dated snapshot restored, and the unrelated latest one did not leak in.
    for (const pack of packs) {
      expect(await e.ESO_PACKS.get(`pack:${pack.id}`)).toBeTruthy();
    }
    expect(await e.ESO_PACKS.get("pack:wrong-snapshot")).toBeNull();
    const index = await getCanonicalIndex(e);
    expect((index?.packs ?? []).map((p) => p.id).sort()).toEqual(packs.map((p) => p.id).sort());
  });

  it("advances even when given a fractional limit", async () => {
    // 0 < limit < 1 floored to 0, so end === start: the page wrote nothing and
    // returned the same cursor with done:false — a caller looping until done
    // would spin forever.
    const packs = Array.from({ length: 2 }, (_, i) => makePack(`frac-${i}`));
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: "2026-04-01T00:00:00.000Z",
        packs,
        packBodies: Object.fromEntries(packs.map((p) => [p.id, p])),
        votes: {},
      })
    );
    for (const pack of packs) await e.ESO_PACKS.delete(`pack:${pack.id}`);

    const res = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 0.5 }),
      })
    );
    expect(res.status).toBe(200);
    const body = await res.json<{ done: boolean; cursor: number; restored_packs: number }>();
    expect(body.restored_packs, "a fractional limit restored nothing").toBeGreaterThan(0);
    expect(body.done ? Infinity : body.cursor, "cursor did not advance").toBeGreaterThan(0);
  });

  it("refuses a cursor equal to the total so forged completion cannot skip remaining writes", async () => {
    const packs = Array.from({ length: 3 }, (_, i) => makePack(`at-total-${i}`));
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: "2026-03-01T00:00:00.000Z",
        packs,
        packBodies: Object.fromEntries(packs.map((p) => [p.id, p])),
        votes: {},
      })
    );
    for (const pack of packs) await e.ESO_PACKS.delete(`pack:${pack.id}`);
    await putPackIndex(e, { packs: [] });

    const first = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 1 }),
      })
    );
    const { cursor, total, token } = await first.json<{ cursor: number; total: number; token: string; done: boolean }>();
    expect(cursor).toBe(1);
    expect(total).toBe(3);

    const res = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ cursor: total, token, limit: 1 }),
      })
    );
    expect(res.status).toBe(409);
    const body = await res.json<{ error: string; expected_cursor?: number; expected_token?: string }>();
    expect(body.error).toMatch(/server-owned job cursor/);
    expect(body.expected_cursor).toBe(1);
    expect(body.expected_token).toBeUndefined();

    expect(await e.ESO_PACKS.get(`pack:${packs[2]!.id}`)).toBeNull();
    const index = await getCanonicalIndex(e);
    expect(index?.packs ?? []).toHaveLength(0);
  });

  it("rejects legacy plaintext restore continuation tokens", async () => {
    const res = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({
          cursor: 1,
          token: "backup:latest|2026-03-01T00:00:00.000Z|1|1",
        }),
      })
    );

    expect(res.status).toBe(409);
    const body = await res.json<{ error: string }>();
    expect(body.error).toMatch(/invalid/);
  });

  it("keeps the page cap under the Worker subrequest ceiling", async () => {
    // Each published pack costs a KV put plus two D1 calls, and every binding
    // call counts against the same 1000-subrequest ceiling. A cap of 400 was
    // ~1200 — over the limit the paging exists to stay under.
    //
    // Seed a snapshot LARGER than the cap, out of vote records. An earlier
    // version of this test seeded an empty one, which proved nothing: with
    // `work.length === 0` the page ends at 0 whether `clampLimit` is honoured or
    // deleted outright, so a regression that stopped clamping real pages passed
    // here and would have failed only during a production restore.
    //
    // Pack records are required because restore deliberately rejects orphan
    // votes whose pack is absent from the snapshot corpus.
    await ensureD1MirrorTables();
    const overCap = RESTORE_MAX_PAGE_SIZE + 1;
    const packs = Array.from({ length: overCap }, (_, i) => makePack(`cap-pack-${i}`));
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: "2026-06-01T00:00:00.000Z",
        packs,
        packBodies: Object.fromEntries(packs.map((pack) => [pack.id, pack])),
        votes: {},
      })
    );

    const oversized = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 100000 }),
      })
    );
    expect(oversized.status).toBe(200);

    // The clamp, observed through the endpoint: an absurd limit must come back
    // having advanced exactly one capped page, with work still outstanding.
    const body = await oversized.json<{ done: boolean; cursor: number; total: number }>();
    expect(body.total).toBe(overCap);
    expect(body.done).toBe(false);
    expect(body.cursor).toBe(RESTORE_MAX_PAGE_SIZE);

    // And the budget the cap is derived from still holds. Note this pair is
    // weaker than it looks on its own — `derived * perRecord <= budget` is true
    // by construction — so the load-bearing half is that the cap is still
    // DERIVED rather than a literal, which is the "cap of 400 was ~1200
    // subrequests" bug this block exists for.
    expect(RESTORE_MAX_PAGE_SIZE).toBe(
      Math.floor((SUBREQUEST_CEILING - SUBREQUEST_RESERVE) / SUBREQUESTS_PER_RECORD)
    );
    expect(RESTORE_MAX_PAGE_SIZE * SUBREQUESTS_PER_RECORD).toBeLessThanOrEqual(
      SUBREQUEST_CEILING - SUBREQUEST_RESERVE
    );

    // Clean up the restored page so later full-corpus tests stay isolated.
    await Promise.all(
      Array.from({ length: RESTORE_MAX_PAGE_SIZE }, (_, i) =>
        e.ESO_PACKS.delete(`pack:cap-pack-${i}`)
      )
    );
    await e.ROSTER_HUB_DB!.prepare("DELETE FROM pack_tags WHERE pack_id LIKE 'cap-pack-%'").run();
    await e.ROSTER_HUB_DB!.prepare("DELETE FROM packs WHERE id LIKE 'cap-pack-%'").run();
    await e.ESO_PACKS.delete("backup:latest");
    // 120s: this case restores 300 pack records and mirrors each one to D1.
  }, 120_000);

  it("restores an empty snapshot without tripping the cursor guard", async () => {
    // start === 0 is always legitimate, including when there is nothing to do.
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: "2026-03-02T00:00:00.000Z",
        packs: [],
        packBodies: {},
        votes: {},
      })
    );
    const res = await call(apiKeyRequest(`${BASE}/admin/restore`, { method: "POST" }));
    expect(res.status).toBe(200);
    const body = await res.json<{ done: boolean; restored_packs: number }>();
    expect(body.done).toBe(true);
    expect(body.restored_packs).toBe(0);
  });

  it("refuses a cursor past the end instead of publishing an unwritten index", async () => {
    const packs = [makePack("past-end-0"), makePack("past-end-1")];
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: new Date().toISOString(),
        packs,
        packBodies: Object.fromEntries(packs.map((pack) => [pack.id, pack])),
        votes: {},
      })
    );
    for (const pack of packs) await e.ESO_PACKS.delete(`pack:${pack.id}`);
    await putPackIndex(e, { packs: [] });

    const first = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 1 }),
      })
    );
    expect(first.status).toBe(200);
    const firstBody = await first.json<{ cursor: number; token: string; done: boolean }>();
    expect(firstBody.cursor).toBe(1);
    expect(firstBody.done).toBe(false);

    const res = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ cursor: 9999, token: firstBody.token, limit: 1 }),
      })
    );
    expect(res.status).toBe(409);
    const body = await res.json<{ error: string; expected_cursor?: number }>();
    expect(body.error).toMatch(/server-owned job cursor/);
    expect(body.expected_cursor).toBe(1);
    expect(await e.ESO_PACKS.get(`pack:${packs[1]!.id}`)).toBeNull();
    const index = await getCanonicalIndex(e);
    expect(index?.packs ?? []).toHaveLength(0);
  });

  it.each([
    { label: "latest", backupKey: "backup:latest", restoreBody: {} },
    { label: "dated", backupKey: "backup:2026-04-18", restoreBody: { date: "2026-04-18" } },
  ])("filters GDPR tombstones while restoring a $label backup", async ({ label, backupKey, restoreBody }) => {
    await ensureD1MirrorTables();
    await e.ROSTER_HUB_DB!.prepare("DELETE FROM pack_tags").run();
    await e.ROSTER_HUB_DB!.prepare("DELETE FROM packs").run();

    const deletedAuthorId = `restore-${label}-deleted-author`;
    const deletedVoterId = `restore-${label}-deleted-voter`;
    const keptUserId = `restore-${label}-kept-user`;
    const deletedPack = makePack(`restore-${label}-deleted-pack`, {
      author_id: deletedAuthorId,
      author_name: "Deleted Author",
    });
    const keptPack = makePack(`restore-${label}-kept-pack`, {
      author_id: keptUserId,
      author_name: "Kept Author",
    });
    const keptVote: VoteRecord = {
      packId: keptPack.id,
      userId: keptUserId,
      votedAt: "2026-04-18T00:00:00.000Z",
    };
    const voteOnDeletedPack: VoteRecord = {
      packId: deletedPack.id,
      userId: keptUserId,
      votedAt: "2026-04-18T00:00:00.000Z",
    };
    const voteByDeletedUser: VoteRecord = {
      packId: keptPack.id,
      userId: deletedVoterId,
      votedAt: "2026-04-18T00:00:00.000Z",
    };
    const backup: { packs: Pack[]; packBodies: Record<string, Pack>; votes: Record<string, VoteRecord> } = {
      packs: [deletedPack, keptPack],
      packBodies: {
        [deletedPack.id]: deletedPack,
        [keptPack.id]: keptPack,
      },
      votes: {
        [`${deletedPack.id}:${keptUserId}`]: voteOnDeletedPack,
        [`${keptPack.id}:${deletedVoterId}`]: voteByDeletedUser,
        [`${keptPack.id}:${keptUserId}`]: keptVote,
      },
    };

    await e.ESO_PACKS.put(`deleted:${deletedAuthorId}`, "2026-04-18T00:00:00.000Z", {
      expirationTtl: 97 * 24 * 60 * 60,
    });
    await e.ESO_PACKS.put(`deleted:${deletedVoterId}`, "2026-04-18T00:00:00.000Z", {
      expirationTtl: 97 * 24 * 60 * 60,
    });
    await e.ESO_PACKS.put(backupKey, JSON.stringify(backup));
    await e.ESO_PACKS.delete(`pack:${deletedPack.id}`);
    await e.ESO_PACKS.delete(`pack:${keptPack.id}`);
    await putPackIndex(e, { packs: [] });

    const res = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 100, ...restoreBody }),
      })
    );
    expect(res.status).toBe(200);
    const result = await res.json<{ done: boolean; restored_packs: number; restored_votes: number }>();
    expect(result.done).toBe(true);
    expect(result.restored_packs).toBe(1);
    expect(result.restored_votes).toBe(1);

    expect(await e.ESO_PACKS.get(`pack:${deletedPack.id}`)).toBeNull();
    expect(await e.ESO_PACKS.get<Pack>(`pack:${keptPack.id}`, "json")).toMatchObject({ id: keptPack.id });
    expect(await e.ESO_PACKS.get(`vote:${deletedPack.id}:${keptUserId}`)).toBeNull();
    expect(await e.ESO_PACKS.get(`user-votes:${keptUserId}:${deletedPack.id}`)).toBeNull();
    expect(await e.ESO_PACKS.get(`vote:${keptPack.id}:${deletedVoterId}`)).toBeNull();
    expect(await e.ESO_PACKS.get(`user-votes:${deletedVoterId}:${keptPack.id}`)).toBeNull();
    expect(await e.ESO_PACKS.get<VoteRecord>(`vote:${keptPack.id}:${keptUserId}`, "json")).toMatchObject({
      packId: keptPack.id,
      userId: keptUserId,
    });

    const index = await getCanonicalIndex(e);
    expect((index?.packs ?? []).map((pack) => pack.id)).toEqual([keptPack.id]);
    const d1Rows = await e.ROSTER_HUB_DB!.prepare("SELECT id FROM packs WHERE id IN (?, ?) ORDER BY id")
      .bind(deletedPack.id, keptPack.id)
      .all<{ id: string }>();
    expect((d1Rows.results ?? []).map((row) => row.id)).toEqual([keptPack.id]);
  });

  it("ignores a nonsense cursor rather than skipping records", async () => {
    const pack = makePack("cursor-guard");
    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: new Date().toISOString(),
        packs: [pack],
        packBodies: { [pack.id]: pack },
        votes: {},
      })
    );
    await e.ESO_PACKS.delete(`pack:${pack.id}`);

    const res = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ cursor: -5 }),
      })
    );
    expect(res.status).toBe(200);
    const body = await res.json<{ done: boolean; restored_packs: number }>();
    expect(body.done).toBe(true);
    expect(body.restored_packs).toBe(1);
    expect(await e.ESO_PACKS.get(`pack:${pack.id}`)).toBeTruthy();
  });
});

// ── DELETE /account ────────────────────────────────────────────────

describe("DELETE /account", () => {
  it("rejects without a bearer token", async () => {
    const res = await call(new Request(`${BASE}/account`, { method: "DELETE" }));
    expect(res.status).toBe(401);
  });

  it("publishes the backup tombstone before deleting canonical packs", async () => {
    const mine = makePack("account-ordering");
    await putPackIndex(e, { packs: [mine] });
    const originalPut = e.ESO_PACKS.put.bind(e.ESO_PACKS);
    let releaseTombstone: (() => void) | undefined;
    let tombstoneOptions: unknown;
    const put = vi.spyOn(e.ESO_PACKS, "put").mockImplementation(async (key, value, options) => {
      if (key === `deleted:${TEST_USER.id}`) {
        tombstoneOptions = options;
        await new Promise<void>((resolve) => { releaseTombstone = resolve; });
      }
      return originalPut(key, value, options);
    });

    const deleting = call(authedRequest(`${BASE}/account`, { method: "DELETE" }));
    await vi.waitFor(() => expect(releaseTombstone).toBeTypeOf("function"));
    expect(tombstoneOptions).toMatchObject({ expirationTtl: 97 * 24 * 60 * 60 });
    expect((await getCanonicalIndex(e)).packs.map(({ id }) => id)).toContain(mine.id);
    releaseTombstone!();
    expect((await deleting).status).toBe(200);
    put.mockRestore();
  });

  it("scrubs the deleting user from the non-expiring backup:latest snapshot", async () => {
    const mine = makePack("mine-1");
    const theirs = makePack("theirs-1", {
      author_id: String(OTHER_USER.id),
      author_name: OTHER_USER.name,
    });
    const myVoteKey = `${theirs.id}:${TEST_USER.id}`;
    const theirVoteKey = `${mine.id}:${OTHER_USER.id}`;

    await e.ESO_PACKS.put(
      "backup:latest",
      JSON.stringify({
        created_at: new Date().toISOString(),
        packs: [mine, theirs],
        packBodies: { [mine.id]: mine, [theirs.id]: theirs },
        votes: {
          [myVoteKey]: {
            userId: String(TEST_USER.id),
            packId: theirs.id,
            votedAt: "2025-01-01T00:00:00.000Z",
          },
          [theirVoteKey]: {
            userId: String(OTHER_USER.id),
            packId: mine.id,
            votedAt: "2025-01-01T00:00:00.000Z",
          },
        },
      })
    );

    await putPack(e, mine);
    await putPack(e, theirs);
    await putPackIndex(e, { packs: [mine, theirs] });
    await putVote(e, mine.id, String(OTHER_USER.id));

    const res = await call(authedRequest(`${BASE}/account`, { method: "DELETE" }));
    expect(res.status).toBe(200);

    const snapshot = await e.ESO_PACKS.get<{
      packs: { id: string }[];
      packBodies: Record<string, unknown>;
      votes: Record<string, unknown>;
    }>("backup:latest", "json");

    // The deleting user's pack and their own vote are gone...
    expect(snapshot!.packs.map((p) => p.id)).toEqual([theirs.id]);
    expect(Object.keys(snapshot!.packBodies)).toEqual([theirs.id]);
    expect(snapshot!.votes[myVoteKey]).toBeUndefined();

    // Votes attached to the deleted pack are lifecycle data, even when another
    // user cast them, so neither live state nor a future restore may retain them.
    expect(snapshot!.votes[theirVoteKey]).toBeUndefined();
    expect(await e.ESO_PACKS.get(`vote:${mine.id}:${OTHER_USER.id}`)).toBeNull();
    expect(await e.ESO_PACKS.get(`user-votes:${OTHER_USER.id}:${mine.id}`)).toBeNull();
  });


  // 20s: this test chains a full delete-account, a create, a real scheduled
  // backup run, and an admin restore, each doing its own D1 mirror round trip.
  // Observed timing out at the 5s default on Windows CI while passing on
  // Linux and macOS, so it gets real headroom instead of a value that just
  // clears one bad run.
  it("includes new packs from a returning deleted author in backups and restores", async () => {
    await ensureD1MirrorTables();
    const oldPackId = "returning-author-old-pack";
    const newPackId = "returning-author-new-pack";
    const todayKey = `backup:${new Date().toISOString().slice(0, 10)}`;

    await e.ESO_PACKS.delete(todayKey);
    await e.ESO_PACKS.delete("backup:latest");
    await e.ESO_PACKS.delete("backup:meta");
    await e.ESO_PACKS.delete(`deleted:${TEST_USER.id}`);
    await e.ESO_PACKS.delete(`pack:${oldPackId}`);
    await e.ESO_PACKS.delete(`pack:${newPackId}`);

    const oldPack = makePack(oldPackId, { status: "published" });
    await putPack(e, oldPack);
    await putPackIndex(e, { packs: [oldPack] });

    const deleted = await call(authedRequest(`${BASE}/account`, { method: "DELETE" }));
    expect(deleted.status).toBe(200);
    expect(await e.ESO_PACKS.get(`deleted:${TEST_USER.id}`)).toBeTruthy();

    // The filters compare record timestamps against deletedAt with <=, so the
    // new pack must land strictly after the deletion instant.
    await new Promise((resolve) => setTimeout(resolve, 5));

    const created = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(
          validPackBody({
            id: newPackId,
            title: "Returning Author Pack",
            status: "published",
          }),
        ),
      }),
    );
    expect(created.status).toBe(201);
    const createdBody = await created.json<{ pack: Pack }>();
    // The tombstone deliberately SURVIVES reactivation: it is scoped by
    // timestamp, so the new pack passes every filter while the pre-deletion
    // corpus stays excluded — deleting it would let an admin restore of an
    // old dated backup republish what the user asked to erase.
    expect(await e.ESO_PACKS.get(`deleted:${TEST_USER.id}`)).toBeTruthy();

    const scheduledCtx = createExecutionContext();
    const scheduledController = createScheduledController({
      scheduledTime: Date.now(),
      cron: "0 0 * * *",
    });
    await worker.scheduled(scheduledController, e, scheduledCtx);
    await waitOnExecutionContext(scheduledCtx);

    const backup = await e.ESO_PACKS.get<{
      packs: Pack[];
      packBodies: Record<string, Pack>;
    }>("backup:latest", "json");
    expect(backup?.packs.map((pack) => pack.id)).toContain(newPackId);
    expect(backup?.packs.map((pack) => pack.id)).not.toContain(oldPackId);
    expect(backup?.packBodies[createdBody.pack.id]).toMatchObject({ author_id: String(TEST_USER.id) });

    await e.ESO_PACKS.delete(`pack:${newPackId}`);
    await putPackIndex(e, { packs: [] });

    const restored = await call(
      apiKeyRequest(`${BASE}/admin/restore`, {
        method: "POST",
        body: JSON.stringify({ limit: 100 }),
      }),
    );
    expect(restored.status).toBe(200);
    const restoredBody = await restored.json<{ done: boolean; restored_packs: number }>();
    expect(restoredBody.done).toBe(true);
    expect(restoredBody.restored_packs).toBe(1);
    expect(await e.ESO_PACKS.get(`pack:${newPackId}`, "json")).toMatchObject({
      id: newPackId,
      author_id: String(TEST_USER.id),
    });
    expect((await getCanonicalIndex(e)).packs.map((pack) => pack.id)).toContain(newPackId);
  }, 20_000);

  it("leaves backup:latest untouched when the user has nothing in it", async () => {
    const theirs = makePack("theirs-only", {
      author_id: String(OTHER_USER.id),
      author_name: OTHER_USER.name,
    });
    const original = JSON.stringify({
      created_at: "2025-01-01T00:00:00.000Z",
      packs: [theirs],
      packBodies: { [theirs.id]: theirs },
      votes: {},
    });
    await e.ESO_PACKS.put("backup:latest", original);
    await putPackIndex(e, { packs: [theirs] });

    const res = await call(authedRequest(`${BASE}/account`, { method: "DELETE" }));
    expect(res.status).toBe(200);

    expect(await e.ESO_PACKS.get("backup:latest")).toBe(original);
  });
});
