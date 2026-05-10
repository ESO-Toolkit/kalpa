import type { Pack } from "../src/types";

export const TEST_USER = { id: 42, name: "testuser" };
export const OTHER_USER = { id: 99, name: "otheruser" };

export function esoLogsResponse(user: { id: number; name: string }) {
  return new Response(
    JSON.stringify({
      data: { userData: { currentUser: user } },
    }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  );
}

export function esoLogsUnauthorized() {
  return new Response("Unauthorized", { status: 401 });
}

export function validPackBody(overrides: Record<string, unknown> = {}) {
  return {
    title: "Test Pack",
    description: "A test pack",
    pack_type: "addon-pack",
    tags: ["pvp"],
    addons: [{ esouiId: 100, name: "TestAddon", required: true }],
    ...overrides,
  };
}

export function authedRequest(
  url: string,
  init: RequestInit = {},
): Request {
  const headers = new Headers(init.headers);
  headers.set("Authorization", "Bearer test-token");
  return new Request(url, { ...init, headers });
}

export function apiKeyRequest(
  url: string,
  init: RequestInit = {},
): Request {
  const headers = new Headers(init.headers);
  headers.set("X-API-Key", "test-api-key");
  return new Request(url, { ...init, headers });
}

export function makePack(id: string, overrides: Partial<Pack> = {}): Pack {
  return {
    id,
    title: `Pack ${id}`,
    description: "Test pack",
    pack_type: "addon-pack",
    author_id: String(TEST_USER.id),
    author_name: TEST_USER.name,
    is_anonymous: false,
    addons: [{ esouiId: 1, name: "Addon", required: true }],
    tags: [],
    vote_count: 0,
    install_count: 0,
    created_at: "2025-01-01T00:00:00.000Z",
    updated_at: "2025-01-01T00:00:00.000Z",
    status: "published",
    ...overrides,
  };
}
