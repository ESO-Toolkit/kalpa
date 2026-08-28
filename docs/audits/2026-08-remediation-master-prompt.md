# Kalpa Audit Remediation - Master Execution Prompt

## Mission

Remediate every verified finding from the August 2026 full-repository audit of Kalpa, starting from current `main`.

Kalpa is a public-beta Tauri v2 desktop application:

- React 19 + TypeScript + Tailwind v4 frontend in `src/`
- Rust/Tauri backend in `src-tauri/`
- Cloudflare Pack Hub Worker in `backend/eso-packs-worker/`
- Shipped Slint sidecar in `prototypes/slint-kalpa/`
- Shared code enters the sidecar through many `#[path]` module includes

Work must be safe, minimal, tested, reviewable, and grounded in the current source. Do not broadly refactor while fixing defects.

This effort will span multiple sessions. The repository tracker, not chat history or model memory, is the source of truth.

## Repository Rules

Read these before doing anything:

1. `claude.md`
2. `AGENTS.md`
3. `docs/audits/2026-08-remediation.md`, creating it from the template below if absent
4. `C:\Users\brayd\.claude\projects\C--Users-brayd-Desktop-Projects-Kalpa\memory\repo-audit-2026-08.md` when available

Treat cited line numbers as hints. Relocate code by symbol name before making any change.

Never:

- Push directly to `main`
- Force-push
- Use `git add .` or `git add -A`
- Add AI attribution
- Modify `.env*`
- Hand-edit lockfiles
- Change the Pack Hub Worker name from `kalpa-pack-hub`
- Deploy to `roster-hub-api`
- Run a real Worker deployment manually
- Modify shared D1 schema without explicit maintainer approval
- Re-report or "fix" refuted findings listed below
- Revert unrelated user or concurrent-agent changes

Use Conventional Commits and short-lived branches named `fix/audit-<id>-<slug>`.

## Persistent Tracker

Create `docs/audits/2026-08-remediation.md` in the first branch if it does not exist.

Use this table:

| ID | Status | Branch | PR | Merged SHA | Fable decision | Sol verdict | Tests | Notes |
|---|---|---|---|---|---|---|---|---|

Status values:

- `todo`
- `in-progress`
- `pr-open`
- `merged`
- `obsolete`
- `deferred`
- `blocked`

The tracker must also contain:

### Decisions

Record every significant architecture decision as `D-<finding>-<number>`, including the chosen design, rejected alternatives, and why.

### Session Log

Record date, executor, completed work, active branch, blockers, and the exact next action.

### Open Questions

Record decisions requiring the maintainer, especially shared-D1 changes, new native locking dependencies, or behavior changes affecting users.

Update the relevant tracker row in the same PR as its fix. If multiple isolated worktrees run concurrently, avoid editing the same tracker hunk; reconcile the index after each merge.

## Session Protocol

At the beginning of every session:

```powershell
git status --short
git fetch origin
git switch main
git pull --ff-only
```

Then:

1. Read the tracker.
2. Confirm the working tree state.
3. Resume a named `in-progress` branch or choose the first dependency-ready `todo`.
4. Re-run that branch's existing gates before making further changes.
5. Announce the finding being handled and its acceptance criteria.

At the end of every session:

1. Run the required gates.
2. Commit all intended work using specifically named files.
3. Push the branch if appropriate.
4. Open or update the PR.
5. Update tracker state.
6. Leave no unexplained uncommitted changes.

Do not run multiple code-writing agents in the same worktree. Parallel implementation is allowed only in isolated worktrees with non-overlapping files and dependency lanes. Advisors remain read-only.

# Advisor Protocol

The primary executor owns all code changes. Advisors inspect and recommend; they do not edit.

Every advisor claim must be checked against the actual code before adoption.

## Fable 5

Use Fable for architecture, concurrency, durability, and cross-process design.

Invoke through Claude Code:

```powershell
$prompt = Get-Content -Raw "<consult-file>.md"
claude -p $prompt --model fable
```

Required Fable consultations:

- Full P0 design before P0-A1 begins
- Worker consistency design before W1 begins
- Folder-ownership/conflict model before R4 begins
- Crash-safe installer transaction design before R6 begins
- Final review of P0 before P0-A3 merges

Every Fable consultation file must contain:

1. Finding and acceptance criteria
2. Current code excerpts, maximum roughly 150 lines per excerpt
3. Relevant repository constraints
4. Two or three candidate designs drafted by the executor
5. Specific failure modes to evaluate
6. Required output format below

Require Fable to return only:

```text
DECISION:
1. Chosen design and numbered implementation steps

REJECTED:
1. Alternative and the concrete failure that rejects it

CRASH_RECOVERY:
1. Behavior after process kill, power loss, stale marker, timeout, or partial write

TESTS:
1. Tests distinguishing a correct design from a plausible but incorrect one

RISKS:
1. Remaining risks and required human decisions
```

If a required section is missing, re-ask once for that section only.

## Sol / Codex

Use Sol as the adversarial reviewer for every implementation PR.

Invoke:

```powershell
codex exec --sandbox read-only -C "<repo>" "<review prompt>"
```

The review prompt must include:

- Finding text
- Acceptance criteria
- Relevant design decision IDs
- `git diff main...HEAD`
- New regression tests
- Explicit instruction to propagate-search the same bug class

Require this output:

```text
VERDICT: APPROVE | REVISE | REJECT

FINDINGS:
1. file:line | severity | concrete failure scenario

MISSING_TESTS:
1. Exact missing case

WIRE_CONTRACT:
OK | BROKEN - explanation

BUG_CLASS_SWEEP:
CLEAN | additional verified sites
```

Handling:

- `APPROVE`: proceed after personally verifying all observations.
- `REVISE`: fix verified findings, rerun gates, request one follow-up review.
- `REJECT`: stop and reassess the design.
- Two verified `REJECT` verdicts: mark the item `blocked`, close or pause the PR, and return to Fable with the rejection evidence.
- A Sol finding may be refuted only with specific code or test evidence recorded in the tracker.

## Kimi

Use Kimi when available for high-volume, long-context mechanical analysis:

- Inventory all cross-process writers
- Compare `commands.rs` orchestration against the 20k+ line Slint `main.rs`
- Find every folder-relative conflict/backup/diff path
- Produce exhaustive state-transition test matrices
- Sweep documentation against implementation

Require tabular output:

```text
file | symbol | current behavior | action required | confidence
```

Kimi is optional. If unavailable, use repository search and document that fallback.

## Luna

Use Luna for frontend and UX review only.

Provide:

- Component diff
- Relevant acceptance criteria
- `context/40-design-system.md`
- `context/41-component-patterns.md`
- `context/42-theme-tokens.md`

Require:

```text
VERDICT: PASS | FAIL
TOKEN_VIOLATIONS:
ACCESSIBILITY:
STATE_FEEDBACK:
RESPONSIVE_BEHAVIOR:
```

Check especially:

- Theme-aware `structure-*`, `scrim-*`, and `status-*` tokens
- No literal white-alpha surfaces or fixed palette colors
- Keyboard and focus behavior
- Error and optimistic-update rollback feedback
- Desktop and mobile-width layouts
- Reduced-motion behavior where relevant

# Refuted Findings - Never Reintroduce

Do not report or "fix" these:

1. Default slice-name collisions do not overwrite or duplicate uploads. Every split invocation gets a fresh `split-<timestamp>` directory and sequential awaits prevent timestamp reuse.
2. `text-amber-*` classes are not unreadable on light accessibility themes. Tailwind aliases remap them through theme status tokens.
3. Sidecar dual-writing manual/live official-uploader settings does not collapse separate user choices. Production semantics already OR both keys and deliberately normalize storage.
4. "Node is not pinned" is stale. `package.json`, `.nvmrc`, and `check-env.js` provide the supported pin and validation.
5. Do not loosen the addon manifest invariant that manifest name equals folder basename. A case-insensitive fallback on Linux may be valid, but changing the identity invariant is not.
6. Do not reintroduce the historical byte-based SavedVariables serializer, path backslash joins, token-refresh credential wiping, missing Tauri capabilities, or discover-panel response races. These are verified fixed.

# Dependency-Ordered Execution Plan

## Worker Lane

Implement worker consistency before low-level worker polish.

### W1 - Atomic Worker Consistency

Handle these in one production-safe PR because merging to `main` deploys the Worker:

- P1-D deleted-pack resurrection
- P1-F duplicate-slug race
- P2-N stale counter overwrite

Acceptance criteria:

- A stale KV detail body cannot recreate a deleted pack.
- ID uniqueness is enforced inside the Durable Object mutation, not by a cached pre-check.
- Updates preserve the latest DO-owned vote/install counters.
- Duplicate creation returns HTTP 409.
- Existing successful response JSON remains compatible with Rust `HubPack`.
- No new, removed, or renamed JSON fields without a paired Rust deserializer test.

Design constraints:

- Evaluate a DO-persisted tombstone or authoritative existence record with Fable.
- Never trust the KV seed as proof of existence.
- Keep mutation checks inside `blockConcurrencyWhile`.
- Do not split these changes into separately deployed intermediate states unless Fable and Sol explicitly establish a safe deployment order.

Tests:

- Delete then vote using a stale seed does not restore the pack.
- Delete then install using a stale seed does not restore the pack.
- Concurrent same-slug creates produce one pack and one 409.
- Update racing a vote preserves the fresh counter.
- Update racing an install preserves the fresh counter.
- Rust test parses representative list/detail/error responses if any response shape changes.

Likely files:

- `backend/eso-packs-worker/src/index.ts`
- `backend/eso-packs-worker/src/pack-index-do.ts`
- `backend/eso-packs-worker/test/routes.test.ts`
- `backend/eso-packs-worker/test/kv.test.ts`

### W2 - D1 Mirror Reliability

Finding:

D1 operations are awaited but failures are logged and swallowed. There is no reconciliation path, so failed deletes or updates can leave permanent zombie or stale rows in shared `roster-hub-db`.

Acceptance criteria:

- Mirror failures leave a durable diagnostic breadcrumb.
- Reconciliation compares authoritative Pack Hub state with only the owned `packs` and `pack_tags` rows.
- Reconciliation still runs when the authoritative pack index is validly empty.
- A read failure is never mistaken for an authoritative empty index.
- Reconciliation has mutation-count safety limits and fails closed on suspicious divergence.
- No schema change.
- No modification to unrelated shared tables.
- A dry-run/log-only mode is available for initial verification if practical.

Required review:

- Fable design review
- Sol diff review
- Explicit maintainer approval before merging reconciliation that can delete D1 rows

Tests:

- KV index has pack missing from D1: restored.
- D1 has zombie absent from authoritative index: removed.
- Empty authoritative index: owned D1 rows reconciled safely.
- KV/DO read failure: no D1 deletion occurs.
- Partial D1 failure records durable error state.
- Shared unrelated tables are untouched.

### W3 - Worker Low-Severity Hardening

Handle separately after W1/W2:

- Count request-body bytes rather than UTF-16 units and avoid unbounded buffering where Workers APIs permit
- Replace wholesale `voteMemo` clearing with bounded oldest-entry eviction
- Make install counting atomic or explicitly bounded/idempotent
- Prevent stale list responses from repopulating cache after invalidation
- Decide whether `/health` should expose corpus size and backup freshness publicly

# Rust and Cross-Process Lane

## P0-A1 - Shared Crash-Safe Atomic Writer

Finding:

`metadata::save_json_with_backup` uses a fixed `json.tmp` staging filename. Both the Tauri process and Slint sidecar can execute the same shared module, so concurrent writes can open and rename the same temporary path.

Acceptance criteria:

- One shared atomic-write implementation is used by both Rust crates.
- Temporary names are unique per process and operation.
- Replacement data is flushed before rename.
- Existing backup semantics are preserved.
- Failure cleanup never removes another writer's staging file.
- No fixed shared `json.tmp` filename remains.
- The helper does not claim to solve read-modify-write races; P0-A2 handles those.

Start from the tested behavior in `settings_store::atomic_write`.

Audit and migrate:

- `metadata.rs`
- `settings_store.rs`
- `safe_migration.rs`
- `saved_variables/io.rs`
- `edit_backups.rs`
- Slint writer paths sharing or duplicating these operations

Tests:

- Two threads repeatedly writing one target always leave parseable data.
- Unique temporary paths do not collide.
- Failed rename cleans only the operation's own staging file.
- Previous valid primary/backup remains recoverable.
- Both crates compile against the shared implementation.

## P0-A2 - Cross-Process Read-Modify-Write Locking

Atomic rename does not prevent lost updates. Hold an OS-level lock across the entire read -> mutate -> write transaction.

Protected stores include:

- AddOns metadata file `kalpa.json`
- App-data `settings.json`
- `kalpa-profiles.json`
- Any mirrored sidecar store confirmed by the writer inventory

Requirements:

- Consult Fable before choosing `fs4`, another safe crate, or direct platform APIs.
- Prefer a safe cross-platform crate unless it cannot satisfy Windows semantics.
- Add dependencies through Cargo tooling in both crates as required; do not edit lockfiles manually.
- Define lock acquisition ordering to prevent deadlocks when one operation touches multiple stores.
- Define timeout/cancellation behavior.
- Define crash behavior and stale-lock recovery.
- Never delete a lockfile merely because a process ID appears absent unless the locking API makes that safe.
- Apply the same protocol in Tauri and Slint.
- Preserve existing in-process mutexes where they still prevent local contention.

Tests:

- Spawn two processes targeting one store.
- Process B blocks or times out while A holds the transaction lock.
- Killing A releases the OS lock.
- No lost updates after repeated concurrent read-modify-write operations.
- Opposite multi-lock acquisition attempts cannot deadlock.
- Tauri and Slint test helpers use the same lock protocol.

## P0-A3 - Native Sidecar Ready Handshake

Do not invent a second unrelated marker. Inspect and deliberately extend or replace the existing `native-boot.pending` protocol.

Acceptance criteria:

- Parent exits only after the child proves its UI/runtime is ready.
- Existing stale-marker recovery still works.
- Failed or timed-out child startup keeps or restores the WebView path.
- No fixed `sleep(300ms)` determines correctness.
- Duplicate sidecars are rejected without resetting user settings incorrectly.
- Deep-link startup does not leave two independent writers active.
- Shutdown and retry behavior are observable in logs.

Tests:

- Ready arrives normally.
- Child exits before ready.
- Ready times out.
- Stale `native-boot.pending` exists.
- Duplicate sidecar starts.
- Deep link arrives while native shell is active.
- Manual Windows validation with a real built sidecar.

# Addon Update Pipeline Lane

Design P1-B and P1-C together with Fable, then implement as stacked PRs. They share folder ownership semantics.

The Slint sidecar contains parallel implementations. Search and patch or factor both paths; fixing only `commands.rs` is incomplete.

## R4 - Preserve Separately Tracked Sibling Ownership

Finding:

The install recording path assigns `esoui_id = 0` to every non-primary folder in an archive, overwriting separately tracked addon/library identity.

Acceptance invariant:

A separately tracked sibling folder must never be silently demoted, silently downgraded, or excluded from future update checks merely because another ZIP bundles the same folder.

Do not blindly preserve stale version metadata while overwriting files. Choose explicit ownership semantics with Fable.

Candidate designs to evaluate:

- Skip extraction of a bundled sibling that is separately tracked.
- Preserve ownership but classify and present the bundled overwrite as a conflict.
- Maintain explicit bundled ownership metadata and select the authoritative source.
- Reject ambiguous archive installation with actionable user feedback.

Tests:

- Primary plus sibling fixture ZIP.
- Existing sibling has nonzero `esoui_id`.
- Update cannot change that ID to zero silently.
- Older bundled sibling cannot silently downgrade the separately tracked folder.
- Future update checks still include the separately tracked sibling.
- Matching tests cover Tauri and Slint paths.

## R5 - Folder-Qualified Conflict Protection

Finding:

ZIP hashing strips only the primary folder prefix. Sibling entries never participate in conflict classification, backups, selective extraction, or diffs.

This is not only a hash-function change. Carry folder-qualified paths through:

- ZIP classification
- Stored hash manifests
- Pending conflict state
- Conflict diff generation
- Keep-mine / take-update decisions
- Edit backups
- Selective extraction skip keys
- Metadata recording
- Tauri serialization and frontend contracts
- Slint implementation

Acceptance criteria:

- Every top-level folder touched by an archive is classified independently.
- Modified sibling files produce conflicts.
- Keep-mine preserves the correct folder-qualified file.
- Take-update backs up the correct folder-qualified file.
- Unmodified sibling content still updates.
- Flat archives remain correctly wrapped and classified.
- Wire-format changes receive paired Rust and frontend updates and tests.

Tests:

- Modified sibling file.
- User-added sibling file that upstream begins shipping.
- Deleted sibling file.
- Same relative filename in primary and sibling folders.
- Flat archive.
- Symlink entry.
- Keep-mine and take-update for both folders.
- Batch update and single-addon update.
- Matching sidecar behavior.

## R6 - Crash-Safe Installer Transaction

Finding:

Existing folders are modified in place. A crash during copy can leave truncated files that later appear to be user edits and may become a permanent keep-mine baseline.

Do not implement an ad hoc directory swap without Fable review. Windows rename/open-file behavior and multi-folder archives must be designed explicitly.

Acceptance criteria:

- Interrupted extraction cannot leave a partially committed addon presented as healthy.
- Original files remain recoverable until commit.
- A failed multi-folder install has explicit rollback semantics.
- Hash baselines are promoted only after successful commit.
- Keep-mine cannot bless a known incomplete installation.
- Recovery after process restart is deterministic.
- Existing protected-edit backups remain compatible.

Consult Fable on:

- Per-folder staging
- Journal/transaction marker
- Commit order for multi-folder archives
- Windows open-file and antivirus behavior
- Rollback after partial rename
- Startup recovery for abandoned transactions

Tests:

- Failure before first commit.
- Failure after one of multiple folders commits.
- Failure during replacement.
- Restart with abandoned transaction marker.
- Existing files open or locked.
- Antivirus-style transient rename failure.
- Successful update leaves no staging residue.

## R7 - Native Build-Evidence Bound

Finding:

The uploaded report is limited to a vetted `scanned_len` prefix, but build-evidence extraction scans from offset zero through current EOF.

Acceptance criteria:

- Build evidence reads only bytes included in the uploaded report.
- Active-log appends after preflight cannot enter evidence.
- UTF-8 boundary handling remains correct.
- Full static-file uploads continue working.

Add focused tests around append-after-scan behavior.

## R8 - Manifest-Less Protected-Edits Disclosure

Finding:

Without a `.kalpa-hashes` baseline, the system cannot identify user modifications. Many migrated or pre-feature installs silently receive no Protected Edits coverage.

Minimum acceptable fix:

- Surface honest, specific disclosure before update.
- Do not claim protection exists when it does not.
- Do not silently seed a baseline from already-modified files without design review.
- Avoid blocking normal updates unless the maintainer explicitly chooses that policy.

Use Luna for copy, hierarchy, accessibility, and theme review.

## R9 - Downloaded-Version Recording

Finding:

Batch metadata can record the frontend's previously observed `api_version` rather than the version actually downloaded, producing phantom update loops.

Acceptance criteria:

- Metadata records the version associated with the fetched artifact.
- Update checks do not immediately offer the same update again.
- Publish-between-check-and-download receives deterministic handling.

# Frontend Lane

These may run independently in an isolated frontend worktree, except UI work dependent on R4/R5.

## F1 - Import Source Sequencing

`packs.tsx`: a late share-code resolution can overwrite a newer `.esopack` import while retaining unrelated imported settings.

Acceptance criteria:

- Every import source has a monotonically increasing operation ID.
- Starting any new import invalidates all previous import requests.
- Pack data and settings come from the same import operation.
- Switching methods cannot mix state.

Test with deferred promises resolving in reverse order.

## F2 - Uploader Log Directory Sequencing

`uploader-workspace.tsx`: `loadLogs(dir)` can apply folder-A results after switching to folder B.

Acceptance criteria:

- Result application checks both operation ID and current directory.
- Loading/error state belongs to the active directory only.
- File selection cannot point to an entry from a previous directory.

Test A -> B with A resolving last.

## F3 - Imported Log Uses Fresh List Data

After importing a log and awaiting list refresh, selection still reads the previous render's `logs` array and may assume size zero.

Acceptance criteria:

- Selection receives the imported file metadata directly or receives the refreshed list result.
- Large imported logs follow the intended deferred/full-preflight route.
- No render-timing dependency remains.

## F4 - Logout Invalidates Private Loads

`loadMyPacksSeqRef` is not invalidated during logout.

Acceptance criteria:

- Logout invalidates every in-flight authenticated pack request.
- Late results cannot repopulate signed-out state.
- Loading flags settle correctly.

## F5 - Controlled State Parent Veto

`useControlledState` updates internal state even when a controlled parent does not accept the change.

Acceptance criteria:

- Controlled mode renders only the supplied value.
- Uncontrolled mode retains current behavior.
- Parent veto cannot leave internal and external state divergent.
- Dialog behavior remains correct.

Add controlled, vetoed, and uncontrolled tests.

## F6 - Optimistic State Sequencing

Handle independently:

- Settings hydration racing user changes
- Quick toggle failures rolling back newer successful values
- Installed-pack reference rollback restoring stale arrays
- SavedVariables list refreshes resolving out of order

Use operation IDs or confirmed-store state rather than inverting submitted values.

Use Luna for user-visible failure states.

# Release and Hygiene Lane

Complete after functional remediation:

- Generate release `Changed:` copy from the matching CHANGELOG section rather than hand-maintained shared YAML text.
- Decide whether `kalpa-elder-scrolls-themes/` should be tracked or ignored; do not decide without inspecting provenance.
- Update stale Worker package version only if the project chooses to keep it synchronized.
- Update `claude.md` structure tree for `animate-ui/` and tests.
- Produce a proposed branch-pruning list; never delete local or remote branches without approval.
- Revisit ignored quick-xml advisories when the Tauri dependency graph permits it.

# Test-First Loop for Every Finding

For each branch:

1. Locate the current defect by symbol.
2. Write the smallest regression test reproducing the exact scenario.
3. Run it before implementation and record the failure.
4. Implement the minimum correct fix.
5. Run the focused test until green.
6. Run related subsystem tests.
7. Run full required gates.
8. Ask Sol to review.
9. Address verified review findings.
10. Request Luna or final Fable review where required.
11. Commit, push, open PR, and update the tracker.

If a failing-first test is mechanically impossible, document why and provide another objective before/after reproduction.

Do not weaken or delete assertions to make a gate pass.

# Verification Gates

## Root Frontend

```powershell
npm run check
npm test
```

## Main Rust Crate

On Windows, ensure the required sidecar placeholder exists before Tauri compilation:

```powershell
node scripts/ensure-slint-sidecar-placeholder.mjs
```

Then in `src-tauri/`:

```powershell
cargo clippy --fix --allow-dirty --allow-staged
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo fmt --check
```

Clippy runs before final formatting because clippy fixes can alter formatting.

## Slint Sidecar

Required whenever shared Rust modules or `prototypes/slint-kalpa/` change:

```powershell
cargo clippy --manifest-path prototypes/slint-kalpa/Cargo.toml --fix --allow-dirty --allow-staged
cargo fmt --manifest-path prototypes/slint-kalpa/Cargo.toml
cargo clippy --manifest-path prototypes/slint-kalpa/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path prototypes/slint-kalpa/Cargo.toml
cargo fmt --manifest-path prototypes/slint-kalpa/Cargo.toml --check
npm run build:native-slint
```

## Worker

In `backend/eso-packs-worker/`:

```powershell
npm run check
npm test
npx wrangler deploy --dry-run
```

Before merging, explicitly verify:

- `wrangler.toml` name is `kalpa-pack-hub`
- No real deployment command ran
- Worker/Rust wire-contract tests pass
- The PR is safe to deploy immediately because merge to `main` triggers deployment

## Security / Dependencies

Run when dependencies change:

```powershell
npm audit --omit=dev
cargo audit
```

If adding a Rust dependency, use Cargo tooling in each affected crate and allow Cargo to update lockfiles. Never edit lockfiles manually.

## Destructive Sandbox E2E

Required for P0, sibling-folder, conflict, backup, and installer transaction changes:

```powershell
npm run test:e2e:sandbox
```

Before running, read the runner header and `claude.md` caveats.

The sandbox only isolates the AddOns directory. It still uses real:

- App settings
- Manifest cache
- Uploader history
- Tokens
- WebView profile

It empties the real manifest-cache database when scanning the empty sandbox. Normalize or back up relevant developer state first. Never treat it as fully isolated.

## Final Main-Branch Verification

After all remediation PRs merge:

```powershell
npm run check
npm test
npm run check:versions
```

Then run main Rust, sidecar, Worker, and applicable sandbox gates above.

# Pull Request Requirements

Every PR description must include:

- Finding ID and concrete failure scenario
- Root cause
- Chosen design and decision IDs
- Scope and intentionally excluded work
- Failing-before test evidence
- Passing-after test evidence
- Full gate results
- Sol verdict and resolved findings
- Fable/Luna decision where applicable
- Wire-format or persisted-data compatibility statement
- Rollback plan
- Screenshots for visible UI changes

Do not mix unrelated cleanup into remediation PRs.

If a new defect is found, add it to the tracker as `NEW-<number>` with evidence. Do not silently expand the active branch.

If Sol twice rejects a design for verified correctness reasons:

1. Stop implementation.
2. Mark the tracker item `blocked`.
3. Preserve the branch.
4. Reconsult Fable with both reviews.
5. Do not fix-forward through architectural uncertainty.

If a merged change must be undone, use `git revert` on the merge commit. Never rewrite shared history.

# Definition of Done

The remediation is complete only when:

- Every tracker row is `merged`, `obsolete` with evidence, or `deferred` with maintainer approval.
- All P0 and P1 findings are merged.
- All P2 findings are merged or explicitly deferred.
- P3 and hygiene items are triaged.
- Every corrected defect has a regression test where mechanically possible.
- Tauri and Slint behavior remain aligned.
- Worker and Rust wire contracts are tested.
- Shared D1 tables remain compatible with `roster-hub-api`.
- Full main-branch gates pass.
- Required sandbox scenarios pass.
- CHANGELOG `[Unreleased]` summarizes the remediation.
- The final tracker session log lists merged PRs, deferred work, residual risks, and recommended release validation.
