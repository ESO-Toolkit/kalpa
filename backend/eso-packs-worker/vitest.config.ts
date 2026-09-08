import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

export default defineConfig({
  // Every test here is a real round trip into workerd, and the restore and
  // account-deletion specs drive dozens of sequential Durable Object calls
  // apiece. Several sit just past vitest's 5s default, so the suite failed
  // intermittently on whichever spec happened to land over the line -- a real
  // regression was indistinguishable from a busy machine. Individual specs
  // that need more still say so inline (see the vote-budget deletion test).
  test: { testTimeout: 30_000 },
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.toml" },
      miniflare: {
        bindings: {
          ADMIN_API_KEY: "test-api-key",
          ALLOW_SEED: "true",
        },
      },
    }),
  ],
});
