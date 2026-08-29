# Kalpa Audit Wrap-Up — Session Prompt

Paste this whole file as the first message of a new session.

---

You are finishing the August 2026 Kalpa audit remediation. Most of it is done.
Your job is to review what is unreviewed, implement the one remaining lane, and
land the stack.

## Read these first, in order

1. `claude.md` and `AGENTS.md`
2. `docs/audits/2026-08-remediation-master-prompt.md` — the governing rules
3. `docs/audits/2026-08-remediation.md` — the tracker (see the caveat below)
4. Memory: `audit-remediation-advisors-2026-08.md`

**Tracker caveat.** Every branch updates *only its own row*, deliberately, so
parallel worktrees do not fight over the same hunk. No single branch has the full
picture, and a row reading `todo` on the branch you happen to be on does **not**
mean the work is missing. Check the PR list before concluding anything is
unstarted. Reconcile the rows as PRs merge.

## Advisors

- **Fable** (design): `claude -p "$(cat consult.md)" --model fable`. Claude-side.
- **Sol** (adversarial review): `codex exec --sandbox read-only -C "<repo>" "<prompt>"`.
  Confirmed working. Use it on every PR that still says `pending` in the tracker.
- Verify every advisor claim against real code before adopting it. This is not a
  formality — Fable's final P0 review produced a genuine blocker, and Sol has a
  history of finding real defects in exactly the "safety machinery" code this
  audit adds.

## Current state — refreshed 2026-08-29

| PR | Lane | Base | Sol | Notes |
|---|---|---|---|---|
| #389 | P0-A3 sidecar handshake | `fix/audit-p0-a2-cross-process-locking` | **pending** | Fable's final P0 review found and fixed a blocker |
| #390 | R4/R5/R6 designs | **`main`** | **pending** | Design records only, no code; #391 must merge first |
| #391 | Reserved `.kalpa-` folders | **`main`** | **pending** | **Checks currently running (`check`, `check-linux`, `check-macos`, `check-slint`) after sync; prerequisite for #390** |
| #392 | R4 sibling ownership | `docs/audit-r-lane-designs` | **pending** | Behaviour change flagged for review |
| #393 | R5 folder-qualified conflicts | `fix/audit-r4-sibling-ownership` | *in progress* | Largest change; Sol was mid-review |

PR #369 is merged into `main`; #386 is retargeted to `main` and green. Earlier
PRs #370–#388 are already Sol-converged; do not re-review them.

CI is base-sensitive: the workflow triggers on `pull_request: branches: [main]`.
#386 has green checks after retargeting to `main`; #390 is docs-only and currently
has checks running; #391 also has four checks running after its latest sync. The still-stacked implementation PRs
have no checks until they are retargeted to `main`.

## Work remaining

### 1. Sol reviews (do these first — they may change the code)

Run Sol on #389, #391, #392 and #393 (if its review did not complete). The master
prompt specifies the review prompt contents and required output format; a worked
example is `.sol-r5.md` in the R5 worktree. Each prompt must carry the finding
text, acceptance criteria, the design decision ID, the branch diff, the new
regression tests, and an explicit instruction to propagate-search the bug class.

Handle verdicts per the master prompt: `REVISE` → fix verified findings, re-run
gates, request one follow-up. Two verified `REJECT`s → mark `blocked`, pause the
PR, return to Fable with the evidence.

### 2. R6 — crash-safe installer transaction

**Deliberately not started.** The design is settled in `D-R6-1`
(`docs/audits/consultations/r6-fable.md` and `r6-fable-decision.md`); implement
that, do not re-derive it.

It was held back because it is the highest-risk lane — directory swaps, Windows
open-file semantics, and the first startup recovery pass that touches
`addons_dir` — and because Sol was unavailable. Sol is available now, so it can
proceed.

Branch it off `fix/audit-p0-a2-cross-process-locking`: it needs both
`atomic_file.rs` (P0-A1) and `transaction_lock.rs` (P0-A2), and cannot merge
independently of them.

Two decisions in the design need a human answer before implementation. Ask the
maintainer:

- **Symlinked/junctioned addon folders** (developers pointing `AddOns/Foo` at a
  git checkout). A swap replaces the link with a real directory. Fable recommends
  refusing with an explanatory message over silently falling back to the legacy
  in-place path.
- **Directory rename fails on Windows if any file inside is open** without
  `FILE_SHARE_DELETE`. Behaviour becomes a clean refusal instead of a
  half-overwrite — strictly better, but users will see a **new error** where they
  previously saw a silently corrupting success. Needs a CFA-style explanatory
  message and sign-off.

Note `installer.rs:951` `cancel_midway_preserves_pre_existing_addon_files`
currently asserts the in-place semantics as a *requirement*. R6 supersedes that
test; rewrite it, do not preserve it.

### 3. Merge train

PR #369 is already merged into `main`; that merge auto-deployed the Pack Hub
Worker shadow phase. The remaining stack requires maintainer review, and per
`D-W1-2` the rollback path requires a verified backup restore — flipping the
authority flag alone is not a reconciliation strategy. Merge #391 before the
docs record #390 because #390 records #391's reserved-folder fix, then follow
each implementation dependency bottom-up.

When they green-light it: retarget stacked PRs to `main` and merge bottom-up, or
GitHub auto-closes the chain. Squashing a base PR always breaks its stacked child
(new SHA) — rebuild via a fresh worktree and cherry-pick rather than force-pushing.

Two follow-ups are recorded in the tracker's Open Questions and should not be lost:

- **P0-A1 rename budget.** Two P0-A2 concurrency tests flake under full-suite
  load; one failure was definitively inside `atomic_write`'s rename, whose budget
  is ~200ms (`RENAME_ATTEMPTS = 5` at a flat 40ms). Fable item 2a proposes a
  bounded geometric backoff (~2.5s) plus `ERROR_USER_MAPPED_FILE` (1224) in
  `is_transient_rename_error`. The same budget serves the real `settings.json`
  write path in both binaries, so this is worth root-causing rather than
  de-flaking. Needs a decision on acceptable worst-case save latency.
- **R4's demoted-user heal** lives in `auto_link`, which only the main app has.
  Sidecar-only users stay demoted until they open the main app once.

### 4. Final verification

After everything merges, on `main`:

```powershell
npm run check
npm test
npm run check:versions
```

Then run the local-only gates before any release: `npm run test:e2e:sandbox`
(required for P0, sibling-folder, conflict, backup and installer-transaction
changes) and `npm run test:packaged`. Neither runs in CI — WebView2 never binds
the CDP port on a GitHub runner, proven three times. Read the sandbox caveats in
`claude.md` first: it isolates **only** the AddOns folder and empties the real
manifest-cache database.

## Gates for any code you write

```powershell
node scripts/ensure-slint-sidecar-placeholder.mjs   # fresh worktrees need this
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path src-tauri/Cargo.toml
cargo fmt   --manifest-path src-tauri/Cargo.toml --check
# whenever shared modules or prototypes/slint-kalpa change:
cargo clippy --manifest-path prototypes/slint-kalpa/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path prototypes/slint-kalpa/Cargo.toml
cargo fmt   --manifest-path prototypes/slint-kalpa/Cargo.toml --check
npm run build:native-slint
npm run check && npm test                            # if src/ changed
```

Write the failing test **first**. If you write it afterwards, prove it
discriminates by re-running it against the old behaviour and recording the
output — a test that has never failed has demonstrated nothing.

## Machine constraints — these will bite you

- **Disk.** C: has hit 100% and fails Rust links with `os error 112`. Each Kalpa
  worktree build is 15–27 GB. Delete `src-tauri/target` and
  `prototypes/slint-kalpa/target` from worktrees whose lane is finished and
  pushed. `cargo sweep` reclaims nothing here (atimes stay fresh).
- **B: drive** has ~42 GB free and can host a shared cache via
  `CARGO_TARGET_DIR`, but it is the user's media/games drive — **do not delete
  anything on it**, and clean up any cache you create. Note
  `build:native-slint` hardcodes the default target path, so it fails under a
  `CARGO_TARGET_DIR` override; run that gate without it.
- Per-lane worktrees live at `C:\Users\brayd\Desktop\Projects\Kalpa-wt-<id>`.
- `src-tauri/Cargo.toml` shows as modified with an empty numstat. It is a
  CRLF-only phantom — exclude it from every commit, as prior sessions did.
  `kalpa-elder-scrolls-themes/` is untracked on purpose (H2 decides its fate).

## Rules that are easy to violate

- Never push to `main`, never force-push, never `git add .` or `-A`.
- No AI attribution anywhere — commits, PRs, or any other output.
- Never change the Worker name from `kalpa-pack-hub`; never deploy to
  `roster-hub-api`; never run a real Worker deployment.
- Do not re-report the refuted findings listed in the master prompt.
- Update the relevant tracker row in the same PR as its fix.
