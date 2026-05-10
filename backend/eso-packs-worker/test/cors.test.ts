import { describe, it, expect } from "vitest";
import { corsHeaders, handlePreflight } from "../src/cors";

function requestWithOrigin(origin: string | null): Request {
  const headers = new Headers();
  if (origin) headers.set("Origin", origin);
  return new Request("https://example.com", { headers });
}

describe("corsHeaders", () => {
  it.each([
    "http://localhost:1420",
    "http://localhost:5173",
    "http://localhost:3000",
    "https://tauri.localhost",
    "http://tauri.localhost",
  ])("sets Access-Control-Allow-Origin for allowed origin %s", (origin) => {
    const headers = corsHeaders(requestWithOrigin(origin));
    expect(headers["Access-Control-Allow-Origin"]).toBe(origin);
  });

  it("does not set Allow-Origin for disallowed origin", () => {
    const headers = corsHeaders(requestWithOrigin("https://evil.com"));
    expect(headers["Access-Control-Allow-Origin"]).toBeUndefined();
  });

  it("does not set Allow-Origin when origin is null", () => {
    const headers = corsHeaders(requestWithOrigin(null));
    expect(headers["Access-Control-Allow-Origin"]).toBeUndefined();
  });

  it("always includes Allow-Methods", () => {
    const headers = corsHeaders(requestWithOrigin(null));
    expect(headers["Access-Control-Allow-Methods"]).toBe(
      "GET, POST, PUT, DELETE, OPTIONS",
    );
  });

  it("always includes Allow-Headers", () => {
    const headers = corsHeaders(requestWithOrigin(null));
    expect(headers["Access-Control-Allow-Headers"]).toBe(
      "Content-Type, X-API-Key, Authorization",
    );
  });

  it("sets max-age to 86400", () => {
    const headers = corsHeaders(requestWithOrigin(null));
    expect(headers["Access-Control-Max-Age"]).toBe("86400");
  });
});

describe("handlePreflight", () => {
  it("returns 204 with CORS headers", () => {
    const response = handlePreflight(
      requestWithOrigin("http://localhost:1420"),
    );
    expect(response.status).toBe(204);
    expect(response.headers.get("Access-Control-Allow-Origin")).toBe(
      "http://localhost:1420",
    );
  });

  it("returns empty body", async () => {
    const response = handlePreflight(requestWithOrigin(null));
    expect(await response.text()).toBe("");
  });
});
