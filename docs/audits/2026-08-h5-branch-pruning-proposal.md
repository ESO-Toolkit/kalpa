# H5 Branch-Pruning Proposal

Snapshot: 2026-08-26 22:26 (America/Los_Angeles)

Comparison base: `origin/main` at `3ff99839`

Scope: local branches and `origin/*` remote-tracking branches after `git fetch origin --prune`

This is a proposal only. No local branch, remote branch, worktree, commit, or branch tip was deleted or moved.

## Decision rule

A branch is a **safe candidate** only when all of these are true:

1. It has zero commits unique to the branch relative to `origin/main`.
2. It has no open pull request.
3. It is not attached to a worktree.
4. It is not `main`, the current H5 branch, or the W1 stack base.

Branches with an open PR, a worktree attachment, or protected/current status are **retain**. A remote counterpart with the same name as a worktree-attached local branch is also retained conservatively; a remote-tracking ref is not itself worktree-attached. Every remaining branch with one or more unique commits is **needs human review**. This deliberately fails closed for squash- or rebase-merged work: a historical merged PR does not prove that the current branch tip is disposable.

The GitHub branches API reported no branch with `protected: true` at snapshot time. `main`, the current H5 branch, and W1 are nevertheless retained by repository workflow/current-stack policy.

The divergence notation below is `+unique/-missing` relative to `origin/main`. Evidence came from:

```powershell
git fetch origin --prune
git for-each-ref --format="%(refname)|%(objectname:short)|%(committerdate:short)|%(ahead-behind:origin/main)" refs/heads refs/remotes/origin
git worktree list --porcelain
gh pr list --state open --limit 500 --json number,headRefName,baseRefName,isDraft,url
```

## Inventory summary

| Scope | Safe candidate | Retain | Needs human review | Total |
|---|---:|---:|---:|---:|
| Local | 37 | 41 | 51 | 129 |
| Remote | 17 | 35 | 59 | 111 |
| Combined refs | 54 | 76 | 110 | 240 |

`origin/HEAD` is symbolic and excluded from the remote total. A local and remote ref with the same name are separate proposed actions and therefore counted separately.

## Proposed safe candidates

Every row has `+0`, no open PR, no worktree attachment, and no protected/current role.

### Local branches

| Branch | Divergence | Last commit | SHA |
|---|---:|---:|---|
| `chore/audit-residue` | +0/-58 | 2026-08-08 | `b8428759` |
| `chore/release-v0.1.0-beta.16` | +0/-25 | 2026-08-08 | `93dbd0f8` |
| `chore/release-v0.1.0-beta.17` | +0/-10 | 2026-08-13 | `adaf1e79` |
| `feat/ask-required-dependencies-only` | +0/-12 | 2026-08-13 | `b926cede` |
| `feat/eso-running-reloadui-warning` | +0/-287 | 2026-06-06 | `b38d6d29` |
| `feat/protected-edits` | +0/-381 | 2026-05-03 | `1927fb9c` |
| `feat/split-redesign-rust` | +0/-114 | 2026-07-30 | `dd7d5882` |
| `feat/split-redesign-ui` | +0/-101 | 2026-07-31 | `8f4810e7` |
| `feat/ux-polish` | +0/-404 | 2026-05-02 | `61ac0eaf` |
| `fix/audit-remediation-2026-08` | +0/-91 | 2026-08-03 | `33a91771` |
| `fix/dep-version-validation` | +0/-364 | 2026-05-07 | `f644dbdc` |
| `fix/md5-truncate-copy` | +0/-437 | 2026-05-01 | `b1aa392f` |
| `fix/savedvariables-eso-running-gate` | +0/-22 | 2026-08-09 | `a6d5c559` |
| `fix/security-audit-improvements` | +0/-317 | 2026-05-23 | `91c77bbf` |
| `fix/surface-update-errors` | +0/-270 | 2026-06-06 | `6dab45a7` |
| `refactor/app-state-reducers` | +0/-75 | 2026-08-08 | `b56e6537` |
| `release/v0.1.0-beta.1` | +0/-307 | 2026-05-23 | `1db130c1` |
| `test/destructive-e2e` | +0/-77 | 2026-08-08 | `e711179a` |
| `worktree-addon-filtering` | +0/-202 | 2026-06-24 | `e6441042` |
| `worktree-close-game` | +0/-295 | 2026-05-25 | `01941124` |
| `worktree-custom-themes` | +0/-225 | 2026-06-20 | `baa4d8d9` |
| `worktree-fix-3` | +0/-370 | 2026-05-05 | `9a6d3216` |
| `worktree-left-spacing` | +0/-226 | 2026-06-20 | `cb3fe518` |
| `worktree-libs` | +0/-226 | 2026-06-20 | `cb3fe518` |
| `worktree-log-uploader` | +0/-238 | 2026-06-11 | `3629c95f` |
| `worktree-log-uploader-ready` | +0/-195 | 2026-07-02 | `33e73474` |
| `worktree-log-uploader-ready2` | +0/-195 | 2026-07-02 | `33e73474` |
| `worktree-log-uploader-ux` | +0/-202 | 2026-06-24 | `e6441042` |
| `worktree-missing-characters` | +0/-226 | 2026-06-20 | `cb3fe518` |
| `worktree-new` | +0/-299 | 2026-05-24 | `a4c9c344` |
| `worktree-optimize` | +0/-185 | 2026-07-02 | `89815e36` |
| `worktree-pack-hub-cards` | +0/-216 | 2026-06-21 | `f669690c` |
| `worktree-performance-audit-2` | +0/-253 | 2026-06-06 | `814f41a2` |
| `worktree-performance-auidt` | +0/-253 | 2026-06-06 | `814f41a2` |
| `worktree-PR-218` | +0/-196 | 2026-06-29 | `4fcfe196` |
| `worktree-update-failed` | +0/-295 | 2026-05-25 | `01941124` |
| `worktree-vercel-security` | +0/-334 | 2026-05-11 | `bbc60e40` |

### Remote branches

| Branch | Divergence | Last commit | SHA |
|---|---:|---:|---|
| `origin/chore/audit-residue` | +0/-58 | 2026-08-08 | `b8428759` |
| `origin/chore/release-beta-4` | +0/-296 | 2026-05-25 | `a964ea33` |
| `origin/chore/release-v0.1.0-beta.16` | +0/-25 | 2026-08-08 | `93dbd0f8` |
| `origin/chore/release-v0.1.0-beta.17` | +0/-10 | 2026-08-13 | `adaf1e79` |
| `origin/claude/explain-child-addons-rMSRs` | +0/-379 | 2026-05-03 | `fff73def` |
| `origin/feat/ask-required-dependencies-only` | +0/-12 | 2026-08-13 | `b926cede` |
| `origin/feat/protected-edits` | +0/-381 | 2026-05-03 | `1927fb9c` |
| `origin/feat/split-redesign-rust` | +0/-114 | 2026-07-30 | `dd7d5882` |
| `origin/feat/split-redesign-ui` | +0/-101 | 2026-07-31 | `8f4810e7` |
| `origin/feat/ux-polish` | +0/-404 | 2026-05-02 | `61ac0eaf` |
| `origin/fix/audit-remediation-2026-08` | +0/-91 | 2026-08-03 | `33a91771` |
| `origin/fix/dep-version-validation` | +0/-364 | 2026-05-07 | `f644dbdc` |
| `origin/fix/savedvariables-eso-running-gate` | +0/-22 | 2026-08-09 | `a6d5c559` |
| `origin/fix/security-audit-improvements` | +0/-317 | 2026-05-23 | `91c77bbf` |
| `origin/refactor/app-state-reducers` | +0/-75 | 2026-08-08 | `b56e6537` |
| `origin/release/v0.1.0-beta.1` | +0/-307 | 2026-05-23 | `1db130c1` |
| `origin/test/destructive-e2e` | +0/-77 | 2026-08-08 | `e711179a` |

## Retain

Retain every branch in these groups regardless of merge appearance:

- Protected/current: local `main`, `fix/audit-h5-branch-pruning-proposal`, and `fix/audit-w1-worker-consistency`; remote `main` and `fix/audit-w1-worker-consistency`.
- Open PR heads: PRs #365-#384 that remain open, including every active audit stack and the four active Dependabot branches.
- Worktree attached: `chore/release-v0.1.0-beta.8`, `feat/addon-list-sort-options`, `feat/companion-sidecar-forward`, `feat/custom-themes`, `feat/uploader-header-history`, `feat/uploader-tier0`, `fix/audit-h1-release-copy`, `fix/audit-h2-theme-provenance`, `fix/audit-h3-worker-version-policy`, `fix/audit-h4-claude-structure-tree`, `fix/audit-p0-a1-atomic-writer`, `fix/audit-p0-a2-cross-process-locking`, `fix/audit-r8-protected-edits-disclosure`, `fix/audit-w2-d1-reconciliation`, `fix/audit-w3-worker-hardening`, `fix/backup-hardening`, `fix/batch-controls-discover-tab`, `fix/dep-install-transitive-resolution`, `fix/slint-native-ui-polish`, `fix/slow-large-addon-update`, `fix/uploader-audit-findings`, `perf/runtime-cpu-memory`, `spike/native-live`, `t3code/evaluate-skia-visual-memory`, `worktree-audit-faster-logging`, `worktree-fix-1`, `worktree-fix-4`, `worktree-log-uploader-log-tweak`, `worktree-merge`, and `worktree-webview-optimize`.

Some names belong to more than one group. The totals de-duplicate them by scope. Remote `fix/audit-h1-release-copy` is retained for both its local worktree counterpart and open draft PR #384. Remote `fix/audit-h5-branch-pruning-proposal` is retained as the current worktree's remote counterpart after publishing this proposal branch.

## Needs human review

These refs have unique commits and no affirmative retain signal. They are not pruning candidates. Before any later deletion, a maintainer should inspect the historical PR, compare patches or trees rather than names, and decide whether the unique commits are obsolete, superseded, or still valuable.

### Local (51)

`audit/pr184-work`, `audit-183`, `chore/pin-node-version`, `chore/release-beta-14`, `chore/release-v0.1.0-beta.9`, `claude/esopack-v2-settings-Zjo2b`, `claude/modernize-tooltips-6r5yC`, `claude/window-resize-header-2iPCB`, `codex/fix-pack-hub-auth-callback`, `codex/native-uploader-10x-proof`, `codex/pr157-audit-fixes`, `codex/slint-native-ui-port`, `dependabot/cargo/src-tauri/sha2-0.11.0`, `dependabot/cargo/src-tauri/zip-8.6.0`, `dependabot/npm_and_yarn/lucide-react-1.11.0`, `docs/light-mode-in-scope`, `docs/slint-renderer-default`, `feat/a11y-text-size`, `feat/account-visibility`, `feat/batch-update-stop`, `feat/default-theme-nord`, `feat/light-themes`, `feat/light-themes-v2`, `feat/log-uploader`, `feat/modal-redesign`, `feat/pack-hub-sync`, `feat/slint-prototype-land`, `feat/text-zoom`, `feat/text-zoom-v2`, `feat/uploader-ux`, `fix/atomic-settings-persistence`, `fix/bundled-dep-version-check`, `fix/ci-root-audit-omit-dev`, `fix/comprehensive-audit-round-3`, `fix/dev-port-conflict`, `fix/esoui-dep-install-search`, `fix/issue-199-text-contrast`, `fix/light-mode-residue`, `fix/live-uploader-background`, `fix/missing-characters`, `fix/native-encoder-code9`, `fix/native-encoder-fidelity`, `fix/outdated-dep-lookup`, `fix/pack-hub-500`, `fix/rust-audit-vulnerabilities`, `fix/snapshot-save-bugs`, `fix/startup-parallelism`, `fix/sv-manager-safety`, `fix/update-pipeline-hash-reuse`, `refactor/remove-dead-code`, `worktree-slint`.

Unique-commit range: 1-229 commits. Last-commit range: 2026-04-03 through 2026-07-29.

### Remote (59)

`origin/chore/pin-node-version`, `origin/chore/release-beta-14`, `origin/chore/release-v0.1.0-beta.9`, `origin/chore/worker-dev-deps`, `origin/claude/analyze-test-coverage-uWbOj`, `origin/claude/bump-vitest-pool-workers-rebased`, `origin/claude/dependabot-pr-review-aa759k`, `origin/claude/dependabot-prs-review-xnhwow`, `origin/claude/esopack-v2-settings-Zjo2b`, `origin/claude/fix-backup-conflicts-79Lmk`, `origin/claude/kalpa-pr-217-audit-os99bo`, `origin/claude/memory-cpu-audit-be4cki`, `origin/claude/modernize-tooltips-6r5yC`, `origin/claude/resolve-merge-conflicts-p0iNB`, `origin/claude/savedvariables-copy-audit-d9dbc4`, `origin/claude/savedvariables-manager-audit-4usgvz`, `origin/claude/update-branding-kalpa-qqeAo`, `origin/claude/update-readme-features-eZzRc`, `origin/claude/webview-power-live-session-end-ect67b`, `origin/claude/window-resize-header-2iPCB`, `origin/codex/fix-pack-hub-auth-callback`, `origin/codex/latest-session-split`, `origin/codex/native-build-evidence-sidecar`, `origin/codex/native-uploader-10x-proof`, `origin/codex/slint-native-ui-port`, `origin/docs/light-mode-in-scope`, `origin/docs/slint-renderer-default`, `origin/feat/a11y-text-size`, `origin/feat/account-visibility`, `origin/feat/batch-update-stop`, `origin/feat/brand-badge`, `origin/feat/light-themes`, `origin/feat/light-themes-v2`, `origin/feat/log-uploader`, `origin/feat/modal-redesign`, `origin/feat/pack-hub-sync`, `origin/feat/uploader-ux`, `origin/fix/atomic-settings-persistence`, `origin/fix/bundled-dep-version-check`, `origin/fix/ci-root-audit-omit-dev`, `origin/fix/comprehensive-audit-round-3`, `origin/fix/dep-resolution-consistency`, `origin/fix/esoui-dep-install-search`, `origin/fix/issue-199-text-contrast`, `origin/fix/light-mode-residue`, `origin/fix/live-uploader-background`, `origin/fix/md5-truncate-copy`, `origin/fix/missing-characters`, `origin/fix/native-encoder-fidelity`, `origin/fix/outdated-dep-lookup`, `origin/fix/pack-hub-500`, `origin/fix/pack-hub-anonymity`, `origin/fix/rust-audit-vulnerabilities`, `origin/fix/snapshot-save-bugs`, `origin/fix/startup-parallelism`, `origin/fix/sv-manager-safety`, `origin/fix/update-pipeline-hash-reuse`, `origin/refactor/remove-dead-code`, `origin/release/v0.1.0-beta.10`.

Unique-commit range: 1-233 commits. Last-commit range: 2026-04-03 through 2026-07-28.

## Approval and execution guard

The maintainer may approve all or a subset of the 54 safe-candidate rows. Approval should name the exact scope and branch names. Before a separate cleanup operation, re-fetch, recompute every guard, and stop if a SHA, PR state, worktree attachment, protection state, or divergence changed. This document does not authorize deletion.

Immediately before opening this proposal PR, the executor must run a final freshness guard: fetch/prune, compare the GitHub branches API with remote-tracking names, refresh open PR heads, and recompute all six category counts. Any mismatch with this document blocks the PR until the snapshot is updated. The same guard is mandatory again before any separately approved cleanup.
