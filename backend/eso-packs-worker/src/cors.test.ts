import { describe, expect, it } from "vitest";
import { corsHeaders, handlePreflight } from "./cors";

function withOrigin(origin: string | null): Request {
  const headers = new Headers();
  if (origin !== null) headers.set("Origin", origin);
  return new Request("https://worker/", { headers });
}

describe("corsHeaders", () => {
  it("always sets Methods, Headers, and Max-Age", () => {
    const h = corsHeaders(withOrigin(null));
    expect(h["Access-Control-Allow-Methods"]).toContain("GET");
    expect(h["Access-Control-Allow-Methods"]).toContain("POST");
    expect(h["Access-Control-Allow-Methods"]).toContain("OPTIONS");
    expect(h["Access-Control-Allow-Headers"]).toContain("Content-Type");
    expect(h["Access-Control-Allow-Headers"]).toContain("Authorization");
    expect(h["Access-Control-Max-Age"]).toBe("86400");
  });

  it.each([
    "http://localhost:1420",
    "http://localhost:5173",
    "http://localhost:3000",
    "https://tauri.localhost",
    "http://tauri.localhost",
  ])("echoes allowed origin %s back", (origin) => {
    const h = corsHeaders(withOrigin(origin));
    expect(h["Access-Control-Allow-Origin"]).toBe(origin);
  });

  it.each([
    "https://evil.example.com",
    "http://localhost:9999",
    "https://localhost:1420", // wrong scheme
    "", // empty
  ])("does not set Allow-Origin for disallowed origin %j", (origin) => {
    const h = corsHeaders(withOrigin(origin));
    expect(h["Access-Control-Allow-Origin"]).toBeUndefined();
  });

  it("does not set Allow-Origin when no Origin header is present", () => {
    const h = corsHeaders(withOrigin(null));
    expect(h["Access-Control-Allow-Origin"]).toBeUndefined();
  });
});

describe("handlePreflight", () => {
  it("returns 204 with CORS headers", () => {
    const res = handlePreflight(withOrigin("http://localhost:1420"));
    expect(res.status).toBe(204);
    expect(res.headers.get("Access-Control-Allow-Origin")).toBe("http://localhost:1420");
    expect(res.headers.get("Access-Control-Allow-Methods")).toContain("OPTIONS");
  });

  it("returns 204 even without an allowed origin (browser will reject the response itself)", () => {
    const res = handlePreflight(withOrigin("https://evil.example.com"));
    expect(res.status).toBe(204);
    expect(res.headers.get("Access-Control-Allow-Origin")).toBeNull();
  });
});
