// Prettier configuration for the Pack Hub worker.
//
// Everything is inherited from the repository root's .prettierrc; only
// trailingComma differs, and only because this directory was never written to
// the root's "es5". src/ is authored trailing-comma-"all" (Prettier 3's own
// default) while test/ is authored "es5", so a single setting makes one of them
// dirty no matter which is chosen: under the root config `prettier --check`
// reports src/index.ts and src/pack-index-do.ts -- the router and the Durable
// Object -- as unformatted on a tree where every gate is green, and forcing
// "all" instead would newly dirty test/routes.test.ts and
// test/pack-index-do.test.ts. Pin each to what it already is, so format-on-save
// stops rewriting the trailing comma of every multi-line call it touches and
// unrelated edits stop arriving with comma churn attached.
//
// The alternative offered was reformatting the worker instead. That is ~880
// changed lines across 25 files for no functional change, concentrated in the
// files whose diffs most need to stay readable, and it would not have been
// finished by a config choice anyway: this directory is not fully Prettier-clean
// under EITHER setting. Closing that out is a formatting-only pass, deliberately
// not mixed into a behavioural change.
//
// Nothing enforced "es5" here in the first place -- the root format:check globs
// only {src,public,scripts,e2e} from the repository root, none of which reach
// backend/.
//
// The shared options are read from the root file rather than restated, so they
// cannot drift apart silently.
import { readFileSync } from "node:fs";

const root = JSON.parse(readFileSync(new URL("../../.prettierrc", import.meta.url), "utf8"));

export default {
  ...root,
  trailingComma: "all",
  overrides: [{ files: "test/**", options: { trailingComma: root.trailingComma } }],
};
