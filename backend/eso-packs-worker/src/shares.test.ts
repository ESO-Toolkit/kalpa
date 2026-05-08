import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { handleCreateShare, handleResolveShare, validateBearerToken } from "./shares";
import type { Env, ShareRecord } from "./types";

// ── Fake KV namespace ─────────────────────────────────────────────

interface KvEntry {
  value: string;
  ttl?: number;
}

function fakeKv(): KVNamespace {
  const store = new Map<string, KvEntry>();
  const kv = {
    get: vi.fn(async (key: string, type?: "json" | "text") => {
      const entry = store.get(key);
      if (!entry) return null;
      if (type === "json") return JSON.parse(entry.value);
      return entry.value;
    }),
    put: vi.fn(async (key: string, value: string, opts?: { expirationTtl?: number }) => {
      store.set(key, { value, ttl: opts?.expirationTtl });
    }),
    delete: vi.fn(async (key: string) => {
      store.delete(key);
    }),
    list: vi.fn(async ({ prefix }: { prefix?: string } = {}) => {
      const keys = [...store.keys()]
        .filter((k) => !prefix || k.startsWith(prefix))
        .map((name) => ({ name }));
      return { keys, list_complete: true, cursor: undefined };
    }),
    // expose internals for assertion
    _store: store,
  } as unknown as KVNamespace;
  return kv;
}

function makeEnv(): Env {
  return {
    ESO_PACKS: fakeKv(),
    ADMIN_API_KEY: "test-admin-key",
  };
}

const validSharePayload = {
  title: "Pack",
  description: "desc",
  packType: "addon-pack",
  tags: ["a"],
  addons: [{ esouiId: 1, name: "Foo", required: true }],
};

function authedRequest(body: unknown = validSharePayload, token = "user-token"): Request {
  return new Request("https://worker/api/shares", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
      Origin: "http://localhost:1420",
    },
    body: typeof body === "string" ? body : JSON.stringify(body),
  });
}

// Stub the global fetch used by validateBearerToken
function stubEsoLogsUser(user: { id: number; name: string } | null) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      if (!user) return new Response("nope", { status: 401 });
      return new Response(
        JSON.stringify({ data: { userData: { currentUser: user } } }),
        { status: 200, headers: { "Content-Type": "application/json" } }
      );
    })
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

// ── validateBearerToken ───────────────────────────────────────────

describe("validateBearerToken", () => {
  it("returns null when no Authorization header is present", async () => {
    const req = new Request("https://worker/", { method: "POST" });
    expect(await validateBearerToken(req)).toBeNull();
  });

  it("returns null when scheme is not Bearer", async () => {
    const req = new Request("https://worker/", {
      method: "POST",
      headers: { Authorization: "Basic abc" },
    });
    expect(await validateBearerToken(req)).toBeNull();
  });

  it("returns the user when ESO Logs returns currentUser", async () => {
    stubEsoLogsUser({ id: 7, name: "ada" });
    const user = await validateBearerToken(authedRequest());
    expect(user).toEqual({ id: 7, name: "ada" });
  });

  it("returns null when ESO Logs returns non-OK", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("forbidden", { status: 403 }))
    );
    expect(await validateBearerToken(authedRequest())).toBeNull();
  });

  it("returns null when ESO Logs response has no currentUser", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify({ data: {} }), { status: 200 }))
    );
    expect(await validateBearerToken(authedRequest())).toBeNull();
  });

  it("returns null when fetch throws", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => { throw new Error("network"); }));
    expect(await validateBearerToken(authedRequest())).toBeNull();
  });

  it("forwards the bearer token to the ESO Logs API", async () => {
    let capturedAuth: string | undefined;
    const fetchMock = vi.fn(async (_url: string, init?: { headers?: Record<string, string> }) => {
      capturedAuth = init?.headers?.Authorization;
      return new Response(
        JSON.stringify({ data: { userData: { currentUser: { id: 1, name: "a" } } } })
      );
    });
    vi.stubGlobal("fetch", fetchMock);
    await validateBearerToken(authedRequest(validSharePayload, "secret-token"));
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(capturedAuth).toBe("Bearer secret-token");
  });
});

// ── handleCreateShare ─────────────────────────────────────────────

describe("handleCreateShare", () => {
  it("rejects unauthenticated requests with 401", async () => {
    stubEsoLogsUser(null);
    const env = makeEnv();
    const req = new Request("https://worker/api/shares", {
      method: "POST",
      headers: { Authorization: "Bearer bad" },
      body: JSON.stringify(validSharePayload),
    });
    const res = await handleCreateShare(req, env);
    expect(res.status).toBe(401);
  });

  it("rejects invalid JSON with 400", async () => {
    stubEsoLogsUser({ id: 1, name: "a" });
    const env = makeEnv();
    const req = authedRequest("not-json");
    const res = await handleCreateShare(req, env);
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string };
    expect(body.error).toBe("Invalid JSON");
  });

  it("rejects payloads that fail validation with 400 and details", async () => {
    stubEsoLogsUser({ id: 1, name: "a" });
    const env = makeEnv();
    const res = await handleCreateShare(authedRequest({ title: "" }), env);
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string; details: unknown[] };
    expect(body.error).toBe("Validation failed");
    expect(Array.isArray(body.details)).toBe(true);
    expect(body.details.length).toBeGreaterThan(0);
  });

  it("rejects empty addon array (share validation requires ≥1)", async () => {
    stubEsoLogsUser({ id: 1, name: "a" });
    const env = makeEnv();
    const res = await handleCreateShare(
      authedRequest({ ...validSharePayload, addons: [] }),
      env
    );
    expect(res.status).toBe(400);
  });

  it("creates a share, stores both KV records with TTL, and returns 201", async () => {
    stubEsoLogsUser({ id: 42, name: "ada" });
    const env = makeEnv();
    const res = await handleCreateShare(authedRequest(), env);
    expect(res.status).toBe(201);

    const body = (await res.json()) as {
      code: string;
      expiresAt: string;
      deepLink: string;
    };
    expect(body.code).toMatch(/^[23456789ABCDEFGHJKMNPQRSTUVWXYZ]{6}$/);
    expect(body.deepLink).toBe(`kalpa://share/${body.code}`);
    expect(new Date(body.expiresAt).getTime()).toBeGreaterThan(Date.now());

    // Both KV records present with 7-day TTL
    const internal = (env.ESO_PACKS as unknown as { _store: Map<string, { value: string; ttl?: number }> })._store;
    const shareEntry = internal.get(`share:${body.code}`);
    const userEntry = internal.get(`share-user:42:${body.code}`);
    expect(shareEntry).toBeDefined();
    expect(userEntry).toBeDefined();
    expect(shareEntry?.ttl).toBe(604800);
    expect(userEntry?.ttl).toBe(604800);

    const record = JSON.parse(shareEntry!.value) as ShareRecord;
    expect(record.createdBy).toBe("42");
    expect(record.createdByName).toBe("ada");
    expect(record.pack.title).toBe("Pack");
  });

  it("blocks users at the per-user share limit with 429", async () => {
    stubEsoLogsUser({ id: 5, name: "spammy" });
    const env = makeEnv();
    // Pre-fill 10 existing user shares
    for (let i = 0; i < 10; i++) {
      await env.ESO_PACKS.put(`share-user:5:CODE${i}`, "1");
    }
    const res = await handleCreateShare(authedRequest(), env);
    expect(res.status).toBe(429);
  });

  it("returns 500 if all 3 code generation attempts collide", async () => {
    stubEsoLogsUser({ id: 1, name: "a" });
    const env = makeEnv();
    // Replace get to simulate every candidate code colliding
    const getMock = vi.fn(async (key: string) => (key.startsWith("share:") ? "occupied" : null));
    (env.ESO_PACKS as unknown as { get: typeof getMock }).get = getMock;
    const res = await handleCreateShare(authedRequest(), env);
    expect(res.status).toBe(500);
    expect(getMock).toHaveBeenCalledTimes(3);
  });
});

// ── handleResolveShare ───────────────────────────────────────────

describe("handleResolveShare", () => {
  function getRequest(): Request {
    return new Request("https://worker/api/shares/AAAA22", {
      headers: { Origin: "http://localhost:1420" },
    });
  }

  it.each([
    ["lowercase", "abc234"],
    ["too short", "ABC23"],
    ["too long", "ABC2345"],
    ["forbidden char (0)", "ABC230"],
    ["forbidden char (O)", "ABCO23"],
    ["forbidden char (I)", "ABCI23"],
    ["forbidden char (L)", "ABCL23"],
    ["forbidden char (1)", "ABC123"],
    ["empty", ""],
  ])("rejects malformed code (%s) with 400", async (_label, code) => {
    const env = makeEnv();
    const res = await handleResolveShare(getRequest(), env, code);
    expect(res.status).toBe(400);
  });

  it("returns 404 when code is not in KV", async () => {
    const env = makeEnv();
    const res = await handleResolveShare(getRequest(), env, "AAAA22");
    expect(res.status).toBe(404);
  });

  it("returns the stored pack with public cache header", async () => {
    const env = makeEnv();
    const record: ShareRecord = {
      code: "AAAA22",
      pack: {
        title: "T",
        description: "D",
        packType: "addon-pack",
        tags: [],
        addons: [{ esouiId: 1, name: "x", required: true }],
      },
      createdBy: "1",
      createdByName: "ada",
      createdAt: "2026-05-01T00:00:00.000Z",
      expiresAt: "2026-05-08T00:00:00.000Z",
    };
    await env.ESO_PACKS.put("share:AAAA22", JSON.stringify(record));

    const res = await handleResolveShare(getRequest(), env, "AAAA22");
    expect(res.status).toBe(200);
    expect(res.headers.get("Cache-Control")).toMatch(/max-age=300/);

    const body = (await res.json()) as {
      pack: { title: string };
      sharedBy: string;
      sharedAt: string;
      expiresAt: string;
    };
    expect(body.pack.title).toBe("T");
    expect(body.sharedBy).toBe("ada");
    expect(body.sharedAt).toBe(record.createdAt);
    expect(body.expiresAt).toBe(record.expiresAt);
  });
});
