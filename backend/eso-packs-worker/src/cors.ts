const ALLOWED_ORIGINS = [
  "http://localhost:1420", // Tauri dev (default)
  "http://localhost:1430", // Tauri dev (Kalpa override)
  "http://localhost:5173", // Vite dev (webapp)
  "http://localhost:3000", // Alt dev port
  "https://tauri.localhost", // Tauri production
  "http://tauri.localhost", // Tauri production (http)
  "tauri://localhost", // Tauri production (macOS/Linux webview)
];

function isAllowedOrigin(origin: string | null): boolean {
  if (!origin) return false;
  return ALLOWED_ORIGINS.includes(origin);
}

export function corsHeaders(request: Request): Record<string, string> {
  const origin = request.headers.get("Origin");
  const headers: Record<string, string> = {
    "Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, X-API-Key, Authorization",
    "Access-Control-Max-Age": "86400",
    // Allow-Origin below echoes the caller's Origin, while Authorization can
    // change redaction and viewer state. Shared caches must key on both.
    Vary: "Origin, Authorization",
  };

  if (isAllowedOrigin(origin)) {
    headers["Access-Control-Allow-Origin"] = origin!;
  }

  return headers;
}

export function handlePreflight(request: Request): Response {
  return new Response(null, {
    status: 204,
    headers: corsHeaders(request),
  });
}
