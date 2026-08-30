# H1 Sol Review Record

## Initial review

Verdict: `REVISE`

Verified finding:

1. `.github/scripts/release-body.cjs` stripped a trailing content-owned Markdown reference definition, so valid reference links in generated GitHub and Discord copy could become unresolved.

Missing test requested: a matching release section whose final non-empty lines are content-owned reference definitions.

Wire contract: `OK`. Bug-class sweep: `CLEAN`.

Resolution: changed section-boundary handling so a definition cannot truncate later content, retained trailing definitions referenced by the selected body, and added both shapes as regressions.

## Required follow-up

Verdict: `REVISE`

Verified finding:

1. Reference-label matching was case-sensitive and did not normalize whitespace, unlike Markdown. A body using `[Notes]` with `[notes]: ...`, or single versus repeated spaces, could still lose its definition.

Missing tests requested: case-different and whitespace-normalized labels.

Wire contract: `OK`. Bug-class sweep: `CLEAN`.

Resolution: normalized candidate and definition labels by trimming, collapsing whitespace, and lowercasing. Added a regression covering both requested cases. The prescribed single follow-up review is complete; all verified findings are addressed and the complete release/Discord suite passes 16/16.
