import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.toml" },
      miniflare: {
        bindings: {
          ADMIN_API_KEY: "test-api-key",
          ALLOW_SEED: "true",
        },
        // The real ADDON_INDEX binding stays commented out in wrangler.toml
        // until the database is provisioned; miniflare supplies a local one so
        // the index is fully exercised by tests either way.
        d1Databases: { ADDON_INDEX: "addon-index-test" },
      },
    }),
  ],
});
