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
        // wrangler.toml binds ADDON_INDEX to the real kalpa-addon-index D1
        // database. Tests must never touch it, so miniflare supplies a local
        // throwaway under the same binding name.
        d1Databases: { ADDON_INDEX: "addon-index-test" },
      },
    }),
  ],
});
