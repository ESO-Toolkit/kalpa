# Metadata Worker

> **Superseded:** This metadata caching worker was never built. Instead, the desktop client fetches ESOUI data directly, and the Cloudflare Worker (`kalpa-pack-hub`) serves as the Pack Hub for community addon collections. See `CLAUDE.md` → "Pack Hub Worker — Critical Rules" for the actual backend architecture.

Use Cloudflare Workers + KV only if metadata caching is needed.

Worker responsibilities:

- Accept an ESOUI addon ID
- Return normalized metadata JSON
- Cache metadata in KV
- Revalidate on a TTL
- Fetch public ESOUI pages when cache is stale

Response should include:

- ESOUI ID
- Addon name
- Current version
- Direct download URL
- Compatibility info if available
- Last checked timestamp

Rules:

- No private APIs
- No broad crawling
- No hourly scraping
- Cache aggressively
- Keep request volume low

Suggested endpoint:

- GET /v1/addon/:id
