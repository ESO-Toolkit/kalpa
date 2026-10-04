# H2 Theme Screenshot Provenance

## Decision

Ignore the root-level `kalpa-elder-scrolls-themes/` directory. It is a local
set of visual-review captures, not source input or a distributable asset set.
The directory itself remains untouched.

## Evidence

- The directory contains exactly eight PNG screenshots, one for each Elder
  Scrolls art theme: Elder Scroll, Daedric Obsidian, Dwemer Brass, Ayleid
  Welkynd, Sithis, Hermaeus Mora, Clockwork City, and Nordic Runestone.
- All eight files are 1663x1227, untagged 8-bit truecolor RGB PNGs without
  alpha. Their only PNG chunks are `IHDR`, `IDAT`, and `IEND`, so they contain
  no embedded text, author, copyright, source-URL, gamma, chromaticity, sRGB,
  or ICC-profile metadata. There are no NTFS alternate streams such as
  `Zone.Identifier`.
- The files total 17,247,144 bytes. They have distinct SHA-256 hashes and none
  matches a tracked image in the repository.
- The first four files have filesystem creation times of 14:07 Pacific on
  2026-06-21 and modification times around 14:55; the final four have creation
  and modification times from 14:55:30 through 14:55:39. Those mutable NTFS
  timestamps and the eight matching theme names are consistent with the theme
  development sequence in `b47f3384` (the first four art themes) and
  `e0825246` (the refinement plus four additional themes), but do not prove
  that these exact bytes were used in that review.
- The screenshots visibly contain Kalpa's desktop UI with a "Version
  0.1.0-beta.8 available" banner. The art-theme branch still declared beta.7,
  so the banner identifies the offered update rather than the running build.
  Commit `e0825246` says all eight skins were "CDP-rendered + reviewed," and
  the merged PR #175 records that every theme was CDP-rendered and reviewed
  live.
- No product branch, remote, or tag tracks the directory, and no build script,
  package manifest, or product documentation consumes it. Local T3 checkpoint
  refs snapshot the same eight files as workspace state beginning in August
  2026; those tool-owned recovery refs are not product history or build input.
- The product sources for these visuals are already tracked as CSS gradients
  and inline SVG in `src/lib/theme-skins.ts`, with theme definitions in
  `src/lib/theme-presets.ts`. The Slint port derives its own tracked SVG skins
  from those React sources. The PNG captures are therefore outputs, not the
  editable or generated runtime inputs.
- The README's current theme documentation uses the tracked, optimized
  `.screenshots/themes.webp` image. The eight old full-window PNGs are neither
  referenced documentation media nor the canonical current screenshot.

## Policy

The narrow root-directory ignore prevents these historical/local review
captures from repeatedly appearing as repository residue. It does not ignore
PNG files generally, the tracked `.screenshots/` documentation media, or the
theme source files. If individual captures become useful documentation later,
they should be deliberately optimized, placed under `.screenshots/`, and
referenced from the relevant document rather than committing this workspace
artifact directory wholesale.

## Sol review

The initial review returned `REVISE` for an unanchored ignore rule, overstated
timestamp correlation, an absolute history claim that omitted T3 checkpoint
refs, and premature tracker wording. A focused follow-up verified all four
corrections and returned `REVISE` only for calling the untagged PNGs sRGB and
calling the running UI beta.8. Both factual labels are corrected above; neither
review disputed the track-versus-ignore decision after the evidence check.

## Integrity record

| File | SHA-256 |
|---|---|
| `01-elder-scroll.png` | `4F954A67D54735F6A6DC540CE8F64AAE73C247DCF9EBD28367F45BCC070877F0` |
| `02-daedric-obsidian.png` | `F4756E19A2C1E86C0DC9A2D664D126AF017EDEE453F0C985006CDBA9C281AA46` |
| `03-dwemer-brass.png` | `012574359CF06FE251A2A1259CB0027817A924CFDA394238CEDA670BCA804793` |
| `04-ayleid-welkynd.png` | `988C27C2FF1D34DBA4BFA001B7149736A826871C7AECB04CA4C6A99E58658E70` |
| `05-sithis.png` | `E2C046257CD7D0CC4E404EFAB27C7696BD6F73136AD087E06E78E7719CF5EC1E` |
| `06-hermaeus-mora.png` | `E8098D782A3321A976D38543DE0AFB4864D4FA0B260FF2E4A887B1F07E73AEA5` |
| `07-clockwork-city.png` | `71D38B5A72939E5F736CD4DCE2E620484DADAF199828F85A04A0C0948A6753B6` |
| `08-nordic-runestone.png` | `9CB857CDEF33C9A05F83F5BBEE4A90A4EDBB0066FF18F6DD3219790EE79064EF` |
