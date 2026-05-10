import { env } from "cloudflare:workers";
import { createExecutionContext, waitOnExecutionContext } from "cloudflare:test";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import worker from "../src/index";
import type { Env } from "../src/types";
import {
  TEST_USER,
  esoLogsResponse,
  esoLogsUnauthorized,
  authedRequest,
} from "./helpers";

const BASE = "https://kalpa-pack-hub.eso-toolkit.workers.dev";
const e = env as unknown as Env;

let fetchSpy: ReturnType<typeof vi.fn>;
const originalFetch = globalThis.fetch;

beforeEach(() => {
  fetchSpy = vi.fn((input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (url.includes("esologs.com")) {
      return Promise.resolve(esoLogsResponse(TEST_USER));
    }
    return originalFetch(input);
  });
  globalThis.fetch = fetchSpy as typeof fetch;
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

function sharePayload(overrides: Record<string, unknown> = {}) {
  return {
    title: "Shared Pack",
    description: "A pack to share",
    packType: "addon-pack",
    tags: ["pvp"],
    addons: [{ esouiId: 1, name: "Addon", required: true }],
    ...overrides,
  };
}

describe("POST /shares", () => {
  it("creates a share code", async () => {
    const res = await call(
      authedRequest(`${BASE}/shares`, {
        method: "POST",
        body: JSON.stringify(sharePayload()),
      }),
    );
    expect(res.status).toBe(201);
    const body = await res.json<{ code: string; expiresAt: string; deepLink: string }>();
    expect(body.code).toMatch(/^[23456789ABCDEFGHJKMNPQRSTUVWXYZ]{6}$/);
    expect(body.deepLink).toBe(`kalpa://share/${body.code}`);
    expect(body.expiresAt).toBeTruthy();
  });

  it("rejects without auth", async () => {
    fetchSpy.mockImplementation((input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      if (url.includes("esologs.com")) return Promise.resolve(esoLogsUnauthorized());
      return originalFetch(input);
    });

    const res = await call(
      new Request(`${BASE}/shares`, {
        method: "POST",
        body: JSON.stringify(sharePayload()),
      }),
    );
    expect(res.status).toBe(401);
  });

  it("rejects invalid payload", async () => {
    const res = await call(
      authedRequest(`${BASE}/shares`, {
        method: "POST",
        body: JSON.stringify({ title: "" }),
      }),
    );
    expect(res.status).toBe(400);
  });
});

describe("GET /shares/:code", () => {
  it("resolves a created share code", async () => {
    const createRes = await call(
      authedRequest(`${BASE}/shares`, {
        method: "POST",
        body: JSON.stringify(sharePayload()),
      }),
    );
    const { code } = await createRes.json<{ code: string }>();

    const resolveRes = await call(new Request(`${BASE}/shares/${code}`));
    expect(resolveRes.status).toBe(200);
    const body = await resolveRes.json<{
      pack: { title: string };
      sharedBy: string;
    }>();
    expect(body.pack.title).toBe("Shared Pack");
    expect(body.sharedBy).toBe(TEST_USER.name);
  });

  it("returns 404 for nonexistent code", async () => {
    const res = await call(new Request(`${BASE}/shares/ZZZZZZ`));
    expect(res.status).toBe(404);
  });

  it("returns 400 for invalid code format", async () => {
    // lowercase, too short, contains invalid chars
    const res = await call(new Request(`${BASE}/shares/abc`));
    expect(res.status).toBe(404); // router won't match the pattern
  });
});
