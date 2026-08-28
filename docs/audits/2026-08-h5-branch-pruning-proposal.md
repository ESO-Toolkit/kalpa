# H5 Branch-Pruning Proposal

Snapshot: 2026-08-28 14:34 (America/Los_Angeles)

Comparison base: `origin/main` at `fb92cb92`

Scope: local branches and `origin/*` remote-tracking branches after `git fetch origin --prune`.

This is a proposal only. No local or remote branch, worktree, commit, or branch tip was deleted or moved.

## Decision rule

A branch is a safe candidate only when it has zero commits unique to `origin/main`, no open pull request, no worktree attachment, and is not `main`, the current H5 branch, or the W1 stack base. Open-PR, worktree, protected, and current-stack refs are retained. Every remaining ref with one or more unique commits is needs-human-review. This fails closed for squash- or rebase-merged work.

The GitHub branches API reported no protected branches at this snapshot. `origin/HEAD` is symbolic and excluded. Local and remote refs are counted separately.

Evidence commands:

```powershell
git fetch origin --prune
git for-each-ref --format="%(refname)|%(objectname:short)|%(committerdate:short)|%(ahead-behind:origin/main)" refs/heads refs/remotes/origin
git worktree list --porcelain
gh pr list --state open --limit 500 --json number,headRefName,baseRefName,isDraft,url
gh api repos/ESO-Toolkit/kalpa/branches --paginate --jq '.[]|select(.protected==true)|.name'
```

## Inventory summary

| Scope | Safe | Retain | Needs human review | Total |
|---|---:|---:|---:|---:|
| Local | 38 | 57 | 56 | 151 |
| Remote | 17 | 47 | 59 | 123 |
| Combined refs | 55 | 104 | 115 | 274 |

## Safe candidates

All rows below were `+0` relative to `origin/main`, had no open PR, no worktree attachment, and no protected/current role at the snapshot. Divergence is `+unique/-missing`.

### Local

`chore/audit-residue` (+0/-77, `b8428759`), `chore/release-v0.1.0-beta.16` (+0/-44, `93dbd0f8`), `chore/release-v0.1.0-beta.17` (+0/-29, `adaf1e79`), `feat/ask-required-dependencies-only` (+0/-31, `b926cede`), `feat/eso-running-reloadui-warning` (+0/-306, `b38d6d29`), `feat/protected-edits` (+0/-400, `1927fb9c`), `feat/split-redesign-rust` (+0/-133, `dd7d5882`), `feat/split-redesign-ui` (+0/-120, `8f4810e7`), `feat/ux-polish` (+0/-423, `61ac0eaf`), `fix/audit-remediation-2026-08` (+0/-110, `33a91771`), `fix/dep-version-validation` (+0/-383, `f644dbdc`), `fix/md5-truncate-copy` (+0/-456, `b1aa392f`), `fix/savedvariables-eso-running-gate` (+0/-41, `a6d5c559`), `fix/security-audit-improvements` (+0/-336, `91c77bbf`), `fix/surface-update-errors` (+0/-289, `6dab45a7`), `refactor/app-state-reducers` (+0/-94, `b56e6537`), `release/v0.1.0-beta.1` (+0/-326, `1db130c1`), `t3code/evaluate-skia-visual-memory` (+0/-28, `e528c43c`), `test/destructive-e2e` (+0/-96, `e711179a`), `worktree-PR-218` (+0/-215, `4fcfe196`), `worktree-addon-filtering` (+0/-221, `e6441042`), `worktree-close-game` (+0/-314, `01941124`), `worktree-custom-themes` (+0/-244, `baa4d8d9`), `worktree-fix-3` (+0/-389, `9a6d3216`), `worktree-left-spacing` (+0/-245, `cb3fe518`), `worktree-libs` (+0/-245, `cb3fe518`), `worktree-log-uploader` (+0/-257, `3629c95f`), `worktree-log-uploader-ready` (+0/-214, `33e73474`), `worktree-log-uploader-ready2` (+0/-214, `33e73474`), `worktree-log-uploader-ux` (+0/-221, `e6441042`), `worktree-missing-characters` (+0/-245, `cb3fe518`), `worktree-new` (+0/-318, `a4c9c344`), `worktree-optimize` (+0/-204, `89815e36`), `worktree-pack-hub-cards` (+0/-235, `f669690c`), `worktree-performance-audit-2` (+0/-272, `814f41a2`), `worktree-performance-auidt` (+0/-272, `814f41a2`), `worktree-update-failed` (+0/-314, `01941124`), `worktree-vercel-security` (+0/-353, `bbc60e40`).

### Remote

`origin/chore/audit-residue` (+0/-77, `b8428759`), `origin/chore/release-beta-4` (+0/-315, `a964ea33`), `origin/chore/release-v0.1.0-beta.16` (+0/-44, `93dbd0f8`), `origin/chore/release-v0.1.0-beta.17` (+0/-29, `adaf1e79`), `origin/claude/explain-child-addons-rMSRs` (+0/-398, `fff73def`), `origin/feat/ask-required-dependencies-only` (+0/-31, `b926cede`), `origin/feat/protected-edits` (+0/-400, `1927fb9c`), `origin/feat/split-redesign-rust` (+0/-133, `dd7d5882`), `origin/feat/split-redesign-ui` (+0/-120, `8f4810e7`), `origin/feat/ux-polish` (+0/-423, `61ac0eaf`), `origin/fix/audit-remediation-2026-08` (+0/-110, `33a91771`), `origin/fix/dep-version-validation` (+0/-383, `f644dbdc`), `origin/fix/savedvariables-eso-running-gate` (+0/-41, `a6d5c559`), `origin/fix/security-audit-improvements` (+0/-336, `91c77bbf`), `origin/refactor/app-state-reducers` (+0/-94, `b56e6537`), `origin/release/v0.1.0-beta.1` (+0/-326, `1db130c1`), `origin/test/destructive-e2e` (+0/-96, `e711179a`).

## Retain and review

Retain counts include `main`, current H5, W1, all attached worktrees, and every open PR head (including draft PRs). The exact retain set is regenerated from the commands above; no retain ref is a deletion candidate.

The 56 local and 59 remote review refs have unique commits but no affirmative retain signal. They are not pruning candidates; a maintainer must inspect historical PRs and compare patches/trees before any action. The generated review sets are recorded in the H5 audit session evidence and must be regenerated before cleanup.

## Approval and execution guard

This document authorizes no deletion. A maintainer must name exact branches and scope. Before any separately approved cleanup, fetch/prune again, compare branch/API/worktree/protection/open-PR state, and stop on any change. Recompute all six category counts; any mismatch blocks cleanup until this proposal is refreshed.
