import { env } from "cloudflare:workers";
import { createExecutionContext, waitOnExecutionContext } from "cloudflare:test";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import worker, { invalidatePackListCache } from "../src/index";
import { putPack, putPackIndex, putVote } from "../src/kv";
import { resetTokenCache } from "../src/shares";
import type { Env, PackIndex } from "../src/types";
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
  // The worker memoizes resolved tokens per isolate, but these cases resolve
  // the same token to different identities, and every spelling of the default
  // list view now shares one cache entry.
  resetTokenCache();
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
    const res = await call(
      new Request(BASE, { method: "OPTIONS" }),
    );
    expect(res.status).toBe(204);
  });
});

// ── GET /packs ────────────────────────────────────────────────────

describe("GET /packs", () => {
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
      packs: [
        makePack("a", { title: "PvP Build" }),
        makePack("b", { title: "Healing Setup" }),
      ],
    });

    const res = await call(new Request(`${BASE}/packs?q=pvp`));
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs).toHaveLength(1);
    expect(body.packs[0].id).toBe("a");
  });

  it("hides draft packs by default", async () => {
    await putPackIndex(e, {
      packs: [
        makePack("pub", { status: "published" }),
        makePack("drft", { status: "draft" }),
      ],
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
      packs: [
        makePack("low", { vote_count: 1 }),
        makePack("high", { vote_count: 10 }),
      ],
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
      packs: [
        makePack("few", { install_count: 2 }),
        makePack("many", { install_count: 99 }),
      ],
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
  });

  it("omits user_voted for anonymous callers", async () => {
    await putPackIndex(e, { packs: [makePack("list-anon")] });

    const res = await call(new Request(`${BASE}/packs`));
    const body = await res.json<{ packs: Array<Record<string, unknown>> }>();
    expect(body.packs[0].user_voted).toBeUndefined();
  });
});

// ── POST /packs ───────────────────────────────────────────────────

describe("POST /packs", () => {
  it("creates a pack with auth", async () => {
    // Reset index so prior tests' packs don't trigger the 25-pack-per-user limit
    await putPackIndex(e, { packs: [] });
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody()),
      }),
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
      }),
    );
    expect(res.status).toBe(401);
  });

  it("rejects invalid payload", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify({ title: "" }),
      }),
    );
    expect(res.status).toBe(400);
  });

  it("generates id from title slug", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody({ title: "My Cool Pack!" })),
      }),
    );
    const body = await res.json<{ pack: { id: string } }>();
    expect(body.pack.id).toMatch(/^my-cool-pack/);
  });

  it("honors a requested published status instead of forcing draft", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(
          validPackBody({ title: "Published On Create", status: "published" }),
        ),
      }),
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
      }),
    );
    const body = await res.json<{ pack: { status: string } }>();
    expect(body.pack.status).toBe("draft");
  });

  it("falls back to a usable id when the title slugifies to nothing", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs`, {
        method: "POST",
        body: JSON.stringify(validPackBody({ title: "日本語のパック" })),
      }),
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
          }),
        ),
      }),
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
      }),
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
      }),
    );
    expect(res.status).toBe(429);
  });
});

// ── GET /packs/:id ────────────────────────────────────────────────

describe("GET /packs/:id", () => {
  it("returns a pack", async () => {
    await putPack(e, makePack("get-test"));
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
    await putPack(e, makePack("draft-test", { status: "draft" }));

    fetchSpy.mockImplementation((input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (url.includes("esologs.com")) return Promise.resolve(esoLogsUnauthorized());
      return originalFetch(input);
    });

    const res = await call(new Request(`${BASE}/packs/draft-test`));
    expect(res.status).toBe(404);
  });

  it("shows draft pack to authenticated user", async () => {
    await putPack(e, makePack("draft-visible", { status: "draft" }));
    const res = await call(
      authedRequest(`${BASE}/packs/draft-visible`),
    );
    expect(res.status).toBe(200);
  });

  it("reports user_voted for a signed-in viewer who already voted", async () => {
    await putPack(e, makePack("voted-detail"));
    await putVote(e, "voted-detail", String(TEST_USER.id));

    const res = await call(authedRequest(`${BASE}/packs/voted-detail`));
    const body = await res.json<{ pack: { user_voted: boolean } }>();
    expect(body.pack.user_voted).toBe(true);
    // Per-viewer state must never be cached.
    expect(res.headers.get("Cache-Control")).toBeNull();
  });

  it("reports user_voted false for a signed-in viewer who has not voted", async () => {
    await putPack(e, makePack("unvoted-detail"));
    const res = await call(authedRequest(`${BASE}/packs/unvoted-detail`));
    const body = await res.json<{ pack: { user_voted: boolean } }>();
    expect(body.pack.user_voted).toBe(false);
  });

  it("omits user_voted for anonymous viewers and stays cacheable", async () => {
    await putPack(e, makePack("anon-detail"));
    const res = await call(new Request(`${BASE}/packs/anon-detail`));
    const body = await res.json<{ pack: Record<string, unknown> }>();
    expect(body.pack.user_voted).toBeUndefined();
    expect(res.headers.get("Cache-Control")).toContain("max-age=300");
  });
});

// ── Anonymity enforcement ─────────────────────────────────────────

describe("anonymous pack redaction", () => {
  const anon = () =>
    makePack("anon-pack", { is_anonymous: true, title: "Secret Pack" });
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

    const res = await call(
      new Request(`${BASE}/packs?author=${TEST_USER.id}`),
    );
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

    const res = await call(
      authedRequest(`${BASE}/packs?author=${TEST_USER.id}`),
    );
    const body = await res.json<{ packs: Array<{ id: string }> }>();
    expect(body.packs.map((p) => p.id)).toEqual(["named-pack"]);
  });

  it("shows the author their own anonymous packs with real fields via ?author=", async () => {
    await putPackIndex(e, { packs: [anon(), named()] });

    const res = await call(
      authedRequest(`${BASE}/packs?author=${TEST_USER.id}`),
    );
    const body = await res.json<{
      packs: Array<{ id: string; author_name: string; author_id: string }>;
    }>();
    const anonOut = body.packs.find((p) => p.id === "anon-pack")!;
    expect(anonOut.author_name).toBe(TEST_USER.name);
    expect(anonOut.author_id).toBe(String(TEST_USER.id));
  });

  it("redacts an anonymous pack's detail for unauthenticated callers and keeps it cacheable", async () => {
    await putPack(e, anon());
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
    await putPack(e, anon());
    const res = await call(authedRequest(`${BASE}/packs/anon-pack`));
    expect(res.status).toBe(200);
    const body = await res.json<{
      pack: { author_name: string; author_id: string };
    }>();
    expect(body.pack.author_name).toBe(TEST_USER.name);
    expect(body.pack.author_id).toBe(String(TEST_USER.id));
    expect(res.headers.get("Cache-Control")).toBeNull();
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
      }),
    );
    expect(res.status).toBe(200);
    const body = await res.json<{ pack: { title: string } }>();
    expect(body.pack.title).toBe("Updated Title");
  });

  it("rejects update by different user", async () => {
    await putPack(
      e,
      makePack("not-mine", { author_id: String(OTHER_USER.id) }),
    );

    const res = await call(
      authedRequest(`${BASE}/packs/not-mine`, {
        method: "PUT",
        body: JSON.stringify(validPackBody()),
      }),
    );
    expect(res.status).toBe(403);
  });
});

// ── DELETE /packs/:id ─────────────────────────────────────────────

describe("DELETE /packs/:id", () => {
  it("deletes own pack", async () => {
    const pack = makePack("delete-me");
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const res = await call(
      authedRequest(`${BASE}/packs/delete-me`, { method: "DELETE" }),
    );
    expect(res.status).toBe(200);
    const body = await res.json<{ ok: boolean }>();
    expect(body.ok).toBe(true);
  });

  it("rejects delete by different user", async () => {
    await putPack(
      e,
      makePack("not-mine-del", { author_id: String(OTHER_USER.id) }),
    );

    const res = await call(
      authedRequest(`${BASE}/packs/not-mine-del`, { method: "DELETE" }),
    );
    expect(res.status).toBe(403);
  });

  it("returns 404 for nonexistent pack", async () => {
    const res = await call(
      authedRequest(`${BASE}/packs/ghost`, { method: "DELETE" }),
    );
    expect(res.status).toBe(404);
  });

  it("deletes the pack's vote records so a recycled id starts clean", async () => {
    const pack = makePack("recyclable");
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });
    await putVote(e, "recyclable", String(TEST_USER.id));

    const res = await call(
      authedRequest(`${BASE}/packs/recyclable`, { method: "DELETE" }),
    );
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

    const vote1 = await call(
      authedRequest(`${BASE}/packs/votable/vote`, { method: "POST" }),
    );
    const body1 = await vote1.json<{ voted: boolean; voteCount: number }>();
    expect(body1.voted).toBe(true);
    expect(body1.voteCount).toBe(1);

    const vote2 = await call(
      authedRequest(`${BASE}/packs/votable/vote`, { method: "POST" }),
    );
    const body2 = await vote2.json<{ voted: boolean; voteCount: number }>();
    expect(body2.voted).toBe(false);
    expect(body2.voteCount).toBe(0);
  });

  it("requires auth", async () => {
    await putPack(e, makePack("noauth-vote"));

    fetchSpy.mockImplementation((input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (url.includes("esologs.com")) return Promise.resolve(esoLogsUnauthorized());
      return originalFetch(input);
    });

    const res = await call(
      new Request(`${BASE}/packs/noauth-vote/vote`, { method: "POST" }),
    );
    expect(res.status).toBe(401);
  });

  it("404s on a draft pack instead of revealing it via 401", async () => {
    await putPack(e, makePack("draft-vote", { status: "draft" }));

    const anonymous = await call(
      new Request(`${BASE}/packs/draft-vote/vote`, { method: "POST" }),
    );
    expect(anonymous.status).toBe(404);

    const authed = await call(
      authedRequest(`${BASE}/packs/draft-vote/vote`, { method: "POST" }),
    );
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
    const final = await call(
      authedRequest(`${BASE}/packs/rapid-toggle/vote`, { method: "POST" }),
    );
    const body = await final.json<{ voted: boolean; voteCount: number }>();
    expect(body.voted).toBe(true);
    expect(body.voteCount).toBe(1);
  });
});

// ── POST /packs/:id/install ───────────────────────────────────────

describe("POST /packs/:id/install", () => {
  it("increments install count", async () => {
    const pack = makePack("installable", { install_count: 0 });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const res = await call(
      new Request(`${BASE}/packs/installable/install`, {
        method: "POST",
        headers: { "CF-Connecting-IP": "1.2.3.4" },
      }),
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
      }),
    );

    const res2 = await call(
      new Request(`${BASE}/packs/rate-limited/install`, {
        method: "POST",
        headers: { "CF-Connecting-IP": "5.6.7.8" },
      }),
    );
    const body2 = await res2.json<{ installCount: number }>();
    // Second call returns current count without incrementing
    expect(body2.installCount).toBe(1);
  });

  it("404s on a draft pack rather than bumping and disclosing its count", async () => {
    const pack = makePack("draft-install", { status: "draft", install_count: 0 });
    await putPack(e, pack);
    await putPackIndex(e, { packs: [pack] });

    const res = await call(
      new Request(`${BASE}/packs/draft-install/install`, {
        method: "POST",
        headers: { "CF-Connecting-IP": "9.9.9.9" },
      }),
    );
    expect(res.status).toBe(404);

    const stored = await e.ESO_PACKS.get<{ install_count: number }>(
      "pack:draft-install",
      "json",
    );
    expect(stored!.install_count).toBe(0);
  });
});

// ── POST /admin/seed ──────────────────────────────────────────────

describe("POST /admin/seed", () => {
  it("seeds with valid API key", async () => {
    const res = await call(
      apiKeyRequest(`${BASE}/admin/seed`, { method: "POST" }),
    );
    expect(res.status).toBe(200);
    const body = await res.json<{ ok: boolean; seeded: number }>();
    expect(body.ok).toBe(true);
    expect(body.seeded).toBeGreaterThan(0);
  });

  it("rejects without API key", async () => {
    const res = await call(
      new Request(`${BASE}/admin/seed`, { method: "POST" }),
    );
    expect(res.status).toBe(401);
  });
});

// ── POST /admin/restore ─────────────────────────────────────────────

describe("POST /admin/restore", () => {
  it("rejects without API key", async () => {
    const res = await call(
      new Request(`${BASE}/admin/restore`, { method: "POST" }),
    );
    expect(res.status).toBe(401);
  });

  it("404s when the requested backup snapshot doesn't exist", async () => {
    await e.ESO_PACKS.delete("backup:latest");
    const res = await call(
      apiKeyRequest(`${BASE}/admin/restore`, { method: "POST" }),
    );
    expect(res.status).toBe(404);
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

    const res = await call(
      apiKeyRequest(`${BASE}/admin/restore`, { method: "POST" }),
    );
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

    const restoredVote = await e.ESO_PACKS.get(
      `vote:${pack.id}:${TEST_USER.id}`,
    );
    expect(restoredVote).toBeTruthy();
  });
});

// ── DELETE /account ────────────────────────────────────────────────

describe("DELETE /account", () => {
  it("rejects without a bearer token", async () => {
    const res = await call(new Request(`${BASE}/account`, { method: "DELETE" }));
    expect(res.status).toBe(401);
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
      }),
    );

    await putPack(e, mine);
    await putPack(e, theirs);
    await putPackIndex(e, { packs: [mine, theirs] });

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

    // ...but the other user's records survive. Votes others cast on the
    // deleted user's packs are deliberately kept, mirroring how the live
    // deletion path leaves them in place.
    expect(snapshot!.votes[theirVoteKey]).toBeDefined();
  });

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
