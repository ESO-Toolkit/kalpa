# Native upload — Infinite Archive (IA) scoping

How to add Infinite Archive / Endless Archive support to the native
`/desktop-client/*` encoder. **Research-complete on inputs, blocked on outputs:**
the six IA log tokens and most of their *input* grammar are known from public game
facts, but the *wire encoding* each maps to can only be settled by one owner-run
golden capture (see Probe C in `native-upload-owner-verification.md`).

## Why IA hands off today (correct, not a bug)

The encoder ships against a closed, censused vocabulary of **20** line types
(`coverage::TARGET_LINE_TYPES`). Coverage is **all-or-nothing per log**: a log
routes native only if *every* line type in it is in `PROVEN_LINE_TYPES`, else it
falls back to the official uploader. IA logs introduce **six tokens** not in that
set, so `assess()` returns `Fallback` and every IA log hands off. That is the
safety guarantee working — IA support is purely additive.

## The six IA tokens

Token existence is settled by ZeniMax's own `ENCOUNTER_LOG_LINE_TYPE_*` enum
(official ESO API Patch Notes, Update 42 — the enum of the string written as field
index 1 of each `Encounter.log` line). These are game facts, independently
corroborated by multiple open-source community log parsers and a P42 API-globals
dump. Grammar is the comma fields *after* `<timeMs>,<TYPE>,`. Booleans are `T`/`F`,
timestamps Unix ms, `dungeonId` observed `1` (= Infinite Archive) in every sample.

| Token | Status | Input grammar (after `<timeMs>,<TYPE>,`) | Grammar conf. | Example line |
|---|---|---|---|---|
| `ENDLESS_DUNGEON_BEGIN` | token confirmed | `dungeonId, startTimeMs, <bool>` | high | `11147,ENDLESS_DUNGEON_BEGIN,1,1758511859000,T` |
| `ENDLESS_DUNGEON_END` | token confirmed; **fields disputed** | A: `dungeonId, durationMs, finalScore, <bool>` · B (looks synthetic): `dungeonId, result(str), roundsCompleted` | low — needs capture | `…,ENDLESS_DUNGEON_END,1,<durMs>,<score>,T` (A) |
| `ENDLESS_DUNGEON_STAGE_END` | token confirmed | `dungeonId, dungeonBeginStartTimeMs` (no stage/cycle/arc index in the line) | high | `53960,ENDLESS_DUNGEON_STAGE_END,1,1758511859000` |
| `ENDLESS_DUNGEON_BUFF_ADDED` | token confirmed | `dungeonId, abilityId` (the Verse/Vision buff) | high | `66452,ENDLESS_DUNGEON_BUFF_ADDED,1,200018` |
| `ENDLESS_DUNGEON_BUFF_REMOVED` | token confirmed | `dungeonId, abilityId` | high | `146276,ENDLESS_DUNGEON_BUFF_REMOVED,1,200020` |
| `ENDLESS_DUNGEON_INIT` | token confirmed; **fields undocumented** | unknown — handled by no parser, absent from all fixtures; likely an early-run setup line like `TRIAL_INIT` | none — needs capture | (none captured) |

**Out of scope — not log tokens.** The `EVENT_ENDLESS_DUNGEON_*` family
(`_INITIALIZED`, `_STARTED`, `_COMPLETED`, `_SCORE_UPDATED`,
`_COUNTER_VALUE_CHANGED`, …) are in-game Lua client events, never written to the
log. The real score/counter data (`ENDLESS_DUNGEON_POINT_REASON_*`,
`..._COUNTER_TYPE_*` = ARC/CYCLE/STAGE/WIPES_REMAINING) lives only in those events,
so the **log exposes no arc/cycle/stage index** — don't try to encode one.

## Output encoding — the part research cannot answer

Each known token either emits a wire-code line (like `END_TRIAL` → code `55`) or is
a no-op state marker (like `BEGIN_TRIAL`/`TRIAL_INIT` → `None`). A no-op type still
*must* be in the coverage vocabulary or its mere presence forces the whole log to
fall back. The table below is the research **hypothesis** — actual wire behavior is
capture-only.

| Token | Hypothesis | Analog | Reasoning |
|---|---|---|---|
| `ENDLESS_DUNGEON_BEGIN` | no-op marker (likely) | `BEGIN_TRIAL`→None | run-start, no combat payload |
| `ENDLESS_DUNGEON_INIT` | no-op marker (likely) | `TRIAL_INIT`→None | parallel position; fields unknown |
| `ENDLESS_DUNGEON_END` | **maybe emits a code** (like `END_TRIAL`→55) or no-op | `END_TRIAL`→55 | carries duration/score; but ESO Logs may have no IA run-end code — **the single most important unknown** |
| `ENDLESS_DUNGEON_STAGE_END` | no-op marker (likely) | — | no score field in samples |
| `ENDLESS_DUNGEON_BUFF_ADDED` | maybe effect-style, or no-op | `EFFECT_CHANGED` | carries abilityId; IA verses may be ignored by ESO Logs |
| `ENDLESS_DUNGEON_BUFF_REMOVED` | maybe effect-style, or no-op | `EFFECT_CHANGED` | removal side, same |

**Hard caveat (every row):** the input grammar says what fields *exist*; it says
nothing about the *output* — which wire code (if any) ESO Logs assigns, its field
order, or whether buff/stage lines are encoded at all. `END_TRIAL`→55 in this
codebase was established *only* by capturing the official uploader's wire output for
a real trial and matching byte-for-byte (server-accepted-but-didn't-render until the
codes matched). IA needs identical treatment; input research cannot substitute.

## Implementation plan (gated on the golden capture)

1. **Parse-only first (no capture needed).** Add the six tokens to the parser and
   log the *actual* field values for `ENDLESS_DUNGEON_END` and `_INIT` from a real
   raw IA log — this resolves their disputed/undocumented **input** grammar without
   needing an official-uploader capture.
2. **Golden capture (owner-run, Probe C).** Upload one real IA `Encounter.log`
   through the **Archon App** while capturing the exact ZIP segment with mitmproxy
   (proxy OFF during play). This is the only oracle for the output encoding.
3. **Diagnostic diff.** Run the captured IA segment through `native/differential.rs`
   against the encoder's output to pinpoint each token's true code/field layout.
4. **Implement** the confirmed handling in `native/events.rs::feed` (`match kind` —
   either `=> None` no-op arms or `emit_*` helpers for proven codes), then extend
   `coverage::TARGET_LINE_TYPES` (20 → 26; revise the "closed set of exactly 20"
   census comment + the `target_vocabulary_is_the_closed_set_of_20` test
   deliberately) and `STRUCTURALLY_READY_LINE_TYPES`.
5. **Promote last.** Only after a live IA upload renders correctly, add all six
   tokens to `PROVEN_LINE_TYPES`. The gate is all-or-nothing per log, so **all six**
   must land before any IA log routes native; until then IA falls back safely.

### Code touch-points
- `src-tauri/src/uploader/native/events.rs` — `EventEmitter::feed`, the `match kind`
  arm (add six arms; mirror the trial-marker pattern `BEGIN_TRIAL | TRIAL_INIT => None`
  and `END_TRIAL => self.emit_end_trial(...)`).
- `src-tauri/src/uploader/native/coverage.rs` — `TARGET_LINE_TYPES`, then
  `STRUCTURALLY_READY_LINE_TYPES`, then `PROVEN_LINE_TYPES`. The existing tests
  (`proven_types_are_all_valid_targets`, `coverage_progress`) update automatically;
  the count-pinning test and the "closed set of 20" comment are a deliberate edit.

## Bottom line
All six IA tokens are real (official ZOS enum) and four input grammars are solid
enough to parse today. The feature is **research-complete on inputs, blocked on
outputs** — the wire code each IA event maps to (especially `ENDLESS_DUNGEON_END`)
is obtainable only from one owner-run golden capture. Until then IA already falls
back safely: a low-risk, purely-additive feature whose entire critical path is "get
one good capture, then match the bytes."
