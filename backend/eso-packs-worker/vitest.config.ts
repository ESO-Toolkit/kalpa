import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    cloudflareTest({
      // The [ai] binding in wrangler.toml is a REMOTE binding: by default the
      // pool opens a proxy session against the real Cloudflare API before any
      // test runs, which fails outright without credentials and would break the
      // whole worker suite in CI (ci.yml runs `vitest run` with no CF secrets).
      // Nothing here needs a live model — ask.test.ts injects its own AI stub —
      // so run fully local.
      remoteBindings: false,
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
