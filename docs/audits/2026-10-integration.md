# October audit integration record

## Disposition and integrated scope

The integration corrects 32 of the 33 original October audit findings in code
or configuration. Finding 12 remains partially mitigated: logout and a new OAuth
login invalidate uploader credentials and login-webview cookie state, closing
the traced stale-account reuse path. Cookie sessions are still not bound to a
verified OAuth-account identity. Completing that protection requires a confirmed
cookie-authenticated identity contract and live account-switch/upload acceptance.

The integrated changes preserve SavedVariables key types and Lua values, bind
desktop callbacks to their nonce, reject stale session work, repair editor and
addon/pack actions, and harden installation, extraction, and restore rollback.
Pack Hub persists lifecycle changes with durable mirror repair, reserves share
codes, bounds cleanup, and retains account-deletion cutoffs through backup and
restore. HTTP validation, dependency tooling, platform policy, and privacy
documentation are updated. The unused shadcn CLI is replaced by its licensed CSS.

## Merge and deployment evidence

- Kalpa integration: [PR #492](https://github.com/ESO-Toolkit/kalpa/pull/492).
- Tested PR head: `c6699f97`; merged commit: `7af95e5f13cc5774e4b2b07219449be872988ef2`.
- Exact-head checks: [CI 37179930677](https://github.com/ESO-Toolkit/kalpa/actions/runs/37179930677)
  and [Slint CI 37179930661](https://github.com/ESO-Toolkit/kalpa/actions/runs/37179930661).
  Both completed successfully against `c6699f97`.
- Worker deployment: [deployment 37180613544](https://github.com/ESO-Toolkit/kalpa/actions/runs/37180613544); deployed version: `7e869fd7-a1fe-414f-ab8b-fbf7398330cb`.
- Production verification timestamp (UTC): `2026-10-04T05:45:30.919Z`.
- All six public GET checks passed: `/health`, `/packs`, `/packs?sort=updated`,
  `/packs?sort=votes&page=1`, `/packs?status=draft`, and `/addons/stats`.
  The four list responses contained two published packs each, with the expected
  ordering and cache policy (30 seconds for default/updated/votes; zero for the
  filtered request). No anonymous pack was present, so production redaction was
  not exercised. The index reported 4,253 total and 4,229 live/described addons,
  zero pending descriptions, and a 19.9-hour-old sync. These checks establish
  response behavior at the recorded time, not authenticated or recovery behavior.

The website dependency is verified separately: [website PR #1636](https://github.com/ESO-Toolkit/eso-toolkit/pull/1636)
merged as `b37488db`, and [production deployment 37179478228](https://github.com/ESO-Toolkit/eso-toolkit/actions/runs/37179478228)
succeeded. Verified live AppAuth and OAuthRedirect assets preserve and echo the
desktop callback nonce. This does not establish an end-to-end desktop OAuth test.

Live Worker authority has not been verified. The parity-gated switch to `do`
remains a separate operator step; deployment alone does not establish the current
authority value or prove that the switch occurred. No desktop release tag or
shared production D1 schema change is included in this integration.

## Verification evidence

Exact-head CI and retained local checks establish the following. They do not
replace live runtime acceptance:

- Frontend check/build passed; 951 tests across 93 files passed.
- Native formatting and all-target clippy with warnings denied passed;
  library tests: 1,488 passed, 18 ignored.
- Worker typechecks passed; 483 tests across 15 files passed.
- Root JavaScript checks: 32 passed; release/updater checks: 24 passed.
- Slint formatting and all-target clippy passed; 943 tests passed, with 16 ignored.
  The release sidecar was built, packaged, and verified (45,485,056 bytes).
  All 16 active lock tests also passed locally.
- Windows/macOS/Linux CI passed against the final PR head. macOS native tests:
  1,472 passed, 18 ignored; Linux: 1,472 passed, 19 ignored.
- Fresh full root and Worker npm audits reported zero vulnerabilities, including
  development dependencies. Both Cargo audits reported zero vulnerability entries
  with no ignored advisories, using RustSec database commit
  `ef6173cbc5c50ec8166f9a5b28f07834144373ee`.

## Material residuals and acceptance limits

Both Rust graphs retain `glib 0.18.5` [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html):
unsound `VariantStrIter` iterator implementations. This is a residual Linux GTK
memory-safety concern, not merely a maintenance warning. The compatible GTK line
requires an upstream migration to adopt fixed `glib >=0.20`.

Maintenance advisories also remain. Zero vulnerability entries do not erase
these warnings or the unsoundness advisory above.

| Graph | Crate/version | RustSec advisory |
| --- | --- | --- |
| Both | `proc-macro-error 1.0.4` | RUSTSEC-2024-0370 |
| Native | `unic-char-property 0.9.0` | RUSTSEC-2025-0081 |
| Native | `unic-char-range 0.9.0` | RUSTSEC-2025-0075 |
| Native | `unic-common 0.9.0` | RUSTSEC-2025-0080 |
| Native | `unic-ucd-ident 0.9.0` | RUSTSEC-2025-0100 |
| Native | `unic-ucd-version 0.9.0` | RUSTSEC-2025-0098 |
| Slint | `bincode 2.0.1` | RUSTSEC-2025-0141 |
| Slint | `paste 1.0.15` | RUSTSEC-2024-0436 |
| Slint | `rustybuzz 0.20.1` | RUSTSEC-2026-0206 |
| Slint | `ttf-parser 0.25.1` | RUSTSEC-2026-0192 |

Native packaged/sandbox E2E was not run against the user's running app because
the harness shares real settings and credentials. Live desktop OAuth, account
switching/uploads, token rotation, real credential-store failure/restart,
macOS/Linux runtime, minimum macOS behavior, packaged Slint runtime, assistive
technology, and missing real-capture fixtures remain outside established acceptance.
Build/package success does not establish those runtime properties.

Rollback tests cover operation failures, not every power-loss or hostile concurrent
filesystem substitution. Worker scale/load and retry fairness under persistent
faults remain unverified. Deletion cutoffs retain user IDs/timestamps indefinitely;
dated backups expire after 90 days, `backup:latest` has no TTL, and failed cleanup
requires retries. Recovery must preserve canonical records, tombstones, and
deletion cutoffs; changing authority or restoring KV alone is not a rollback plan.

The original October report and August remediation log remain historical evidence;
their old test counts and no-deployment statements must not be rewritten to imply
that those earlier sessions performed this integration or production deployment.
