# Native live-streaming upload — FINDINGS

**Branch:** `spike/native-live` (off `feat/log-uploader`) · **Status:** lifecycle hardening BUILT + wired (gated, opt-in) · **Date:** 2026-06-21

This is the deliverable for the spike: *"can Kalpa natively live-stream a growing
`Encounter.log` to esologs.com — pushing segments incrementally while the report stays
open — instead of handing off to the official uploader?"*

The answer is **FEASIBLE — the live round-trip is confirmed and the ToS gate is cleared
(owner, 2026-06-20).** The deferred lifecycle hardening (L2/L3/L4/L7/L8) + the gated
opt-in wiring are now **BUILT** — see the production-status section immediately below.
The remaining gate is an owner-run live re-test against a REAL archived combat log.

---

## ✅ PRODUCTION STATUS (2026-06-21) — lifecycle hardening built, gated + opt-in

The deferred items are implemented and the path is wired behind the same opt-in as the
native one-shot. **The official-uploader handoff is still the DEFAULT**; native live
runs only when the user has opted in AND the format is confirmed AND a session exists.
Everything is verified offline (187 uploader tests green; `cargo build --release`
succeeds, proving the debug-only spike command + `FileTail` gate out of release;
`npm run check` + clippy + fmt clean). What shipped, by deferred item:

| Item | What was built | Where | Proven by |
| --- | --- | --- | --- |
| **L7** perf (was O(events×lines)) | Incremental actor/ability index maps (`IncrementalIndexState`) + incremental cumulative master (`IncrementalMasterState`); `all_lines` buffer DROPPED. | `native/incremental.rs`, `native/live.rs`, `native/encode.rs` (shared `render_ability_record`/`resolve_ability_section`) | Differential tests assert byte-identity to the retained re-walk oracle at EVERY line/cut — incl. the real combat capture, late-registration reorder (F1), TARGET_DEAD exclusion (F2), late damage-type (F3), recycled unitId (F4), regen-before-ability (F5), intra-event tie (F7). |
| **L4** in-flight cancel | `LiveSender` runs each POST on a detached worker, polling cancel every 250ms → Stop returns ~250ms, not the 120s timeout. Additive; the one-shot `send` is untouched. | `native/client.rs` | `live_wait_returns_cancelled_fast_*`, `driver_cancel_mid_stream_*`. |
| **L4** retry/backoff | `post_with_retry`: 3 attempts, cancel-interruptible `1s/2s/4s` backoff; classifies 5xx/408/429/network = retry, 4xx/malformed = fatal, 401/419-survived = pause. | `native/live.rs` | `post_with_retry_*`, `cancel_aware_backoff_*`, `driver_retries_transient_*`. |
| **L4** reauth pause-resume | On `Session(Expired)` the driver PAUSES (report stays open, held payload retained), polls for a fresh session, re-POSTs on resume; `REAUTH_TIMEOUT` 15m guard; UI `ReauthRequired`/`ReauthResolved`. | `native/live.rs`, `watcher.rs`, `types.rs` (`UploadStatus::Paused`) | `driver_pauses_on_lost_session_then_resumes_*`, `driver_pause_times_out_*`. |
| **L3** idle deadline | `NotifyTail` signals `Idle` after `IDLE_DEADLINE` (30m) of no growth, distinct from a clean `END_LOG` `Ended`; `nextSegmentId=0` logged; 2nd `BEGIN_LOG` → terminate (single-session). | `native/live.rs` | `driver_stops_on_server_next_segment_id_zero`, `driver_terminates_on_second_begin_log`, line-assembler tests. |
| **L-tail** production tail | `NotifyTail` reuses `watcher.rs`'s read machinery via the shared `tail_io` module (byte-offset `read_range`, MAX_READ, failure-streak reset-on-READ-only, truncation reset) and yields raw lines; `LineAssembler` (partial carry, `\r\n` strip, 1MiB cap, discard-until-BEGIN_LOG). No second watcher. | `uploader/tail_io.rs`, `native/live.rs` | `line_assembler_*` (6 tests). |
| **L2** orphan recovery | Persist `{reportCode,segmentId}` before the first POST; best-effort terminate leftover codes on next launch / panel open; clear only on a confirmed 404/410 or success. | `native/orphans.rs`, `commands.rs` (`CommandOrphanSink`, `recover_orphans_once`) | `record_upserts_by_code`, `is_definitively_closed_only_for_404_410`, etc. |
| **L8** single-instance + mutual exclusion | `LiveSlot::NativeRunning` third variant; `StoppedHandle{Watch,Native}`; store-before-remove cancellation-race UNCHANGED; native `.stop()` joins via `spawn_blocking` (uploader_start_live is async); same-path single-instance guard inside the live_sessions critical section; native self-settles its record by exact id. | `commands.rs` | (concurrency review + the LiveSlot protocol comments). |
| **Gated opt-in wiring** | `assess_native_live_routing(opt_in, has_session)`; `uploader_start_live` routes native-vs-handoff; Settings disclosure extended to cover live; debug `*_live` client methods promoted out of `#[cfg(debug_assertions)]` (driver de-gated; spike command stays gated). | `transport.rs`, `commands.rs`, `settings.tsx`, `uploader-workspace.tsx` | `cargo build --release` (gate-out). |

**Remaining gate:** an owner-run live re-test against a REAL archived combat log (copy
one from `Documents\...\esologsarchive` to a path OUTSIDE CFA-protected folders and
point the debug `uploader_run_native_live_spike` command at it, OR opt in + go live on
a real session) to confirm a multi-fight raid report renders correctly — not just the
synthetic case. The encoder + cumulative pinned master + per-segment time bounds are
unchanged from the confirmed round-trip; this re-test validates the production tail +
lifecycle end-to-end.

---

### (original spike framing follows)

> ## ✅ ROUND-TRIP CONFIRMED (2026-06-20, owner-run)
> The debug-only driver streamed a synthetic 2-fight session to a real esologs report,
> holding the report OPEN across both fights and terminating on `END_LOG`. **`segments=2`
> POSTed, server accepted both, and the report RENDERED** — a streamed segment's events
> resolved into a real, named, timestamped fight ("Tenmar Lynx" trash fight). This settles
> the one unknown the spike could not answer offline: **esologs.com DOES incrementally
> render an open, multi-segment report fed under `isLiveLog:true`/`isRealTime:true`.** The
> native encoder + cumulative pinned master (H1 fix) + per-segment time bounds + open-report
> lifecycle all work end-to-end against the live service. Evidence reports (owner account,
> Unlisted): `nRxYJKqBWNmkTc1f` (the clean 2-segment render), plus `DYnwBNG1Tb7xVLFm` /
> `TAFLDmw18v49yVj3` (earlier runs — the latter `segments=0` from a tester-timing race, NOT
> an encoder fault; once fed correctly it rendered).
>
> Gotchas learned: (a) the `FileTail` idle deadline must exceed the manual feed cadence
> (bumped 10s→120s); a too-short deadline terminates with `segments=0` before fights arrive.
> (b) Pass the path with FORWARD SLASHES through the CDP/JS/Rust layers (`C:/eso-live-spike/...`)
> — backslashes get eaten by the escaping layers. (c) `__TAURI__` is not global (no
> `withGlobalTauri`); invoke via `window.__TAURI_INTERNALS__.invoke(...)`.

It was produced from: a 9-agent analysis workflow (5 analysts → 3 adversarial reviewers →
synthesis, `wf_bdf27e62-24b`), **cross-checked against the actual code**, plus four
empirical probes run against the real encoder (`events::tests::spike_probe_state_continuation_across_a_session_boundary`,
`#[ignore]`d, run with `cargo test -- --ignored --nocapture spike_probe`).

---

## TL;DR

| Question | Answer |
| --- | --- |
| Does carrying encoder state across headerless segments produce a **rendering** multi-segment report? | **YES — confirmed by a live round-trip** (report `nRxYJKqBWNmkTc1f`, 2 segments, rendered a real fight). The core model is sound and largely *forced* by the code; the server-behavior unknown is now resolved positively. |
| Time-base continuation | **Already works in the existing code** for the segment *body*; only the per-segment *wall window* needs new (stateful) logic. |
| Master-table model | **Cumulative** (forced — a delta master dangles the majority of A-refs). But a *naive* cumulative rebuild has an actor-index-stability bug (found by probe, see Hazard H1). |
| Cut policy | **Strictly at fight boundaries (`END_COMBAT`) and at every `BEGIN_LOG`** — never a timer / every-N-events window. |
| Effort to production | **~13–19 dev-days**, the bulk of it lifecycle/crash-safety, *not* the encoder. |
| ToS | **Ask the operator.** Live is a distinct, higher-conspicuousness server operation (`liveLog`) from the already-authorized one-shot. |

---

## The one unknown — NOW RESOLVED (live round-trip, 2026-06-20)

**Does the esologs.com `/desktop-client/*` server keep ONE report OPEN and incrementally
re-render it** when fed many `add-report-segment` POSTs under `isLiveLog:true` /
`isRealTime:true` *without* `terminate-report`?

**ANSWER: YES.** The debug driver opened a report, streamed 2 segments (one per fight) while
holding it open, terminated on `END_LOG`, and the report **rendered a real fight** (report
`nRxYJKqBWNmkTc1f`). The live params (`isLiveLog:true`/`isRealTime:true`, sent only by the
debug `*_live` seam — the one-shot params are untouched) are accepted, and the server stitches
the segments into one report timeline. `nextSegmentId=0` was not observed as a mid-stream
terminal in this run (the driver treats any `0` as session-end defensively); confirm its exact
semantics on a longer multi-segment session if pursued.

The *conditional-fatal* this section once flagged (server finalizes each segment as a closed
slice, or rejects a growing cumulative master) **did not occur** — feasibility is established.
The only remaining ship gate is operator ToS sign-off (next section).

---

## (a) Feasibility: state continuation across headerless segments

**Verdict: feasible-with-caveats.** The hard problem the brief names ("carry the encoder's
running state across segments that have no `BEGIN_LOG`") is **substantially already solved
in the existing one-shot encoder**, because `build()` already keeps one `EventEmitter` for
a whole multi-session file. The empirical probes confirm this directly:

**Probe observation 1 — the tuple table is monotonic and reuses across the boundary.**
Feeding a synthetic two-session log through *one* long-lived emitter, the shared tuple table
grew `2 → 3`: session 2's reused player ability **re-used its session-1 tuple index `A=1`**
rather than allocating a duplicate; only the new monster minted a fresh tuple (`A=3`).

```
[spike] shared tuple table (src,tgt,ability) — index = A:
[spike]   A=1 (1, 1, 1)  (first allocated in s1)
[spike]   A=2 (1, 2, 1)  (first allocated in s1)
[spike]   A=3 (1, 3, 1)  (first allocated in s2)
[spike] session 2 added exactly 1 new tuple(s) — a reused ability does NOT re-allocate
```

This is the make-or-break property: with a **cumulative master** (see (e)), a later segment's
`A`-refs resolve into a tuple table the server already holds. A real-capture measurement from
the adversarial review put numbers on it: **64% (28,714/44,889) of effect references reuse a
tuple first allocated in an earlier fight**; 1,013 `UPDATED` + 1,620 `FADED` + 9 timed-cast
completions cross an `END_COMBAT`. So cross-segment A-refs are the dominant case — a
delta/per-segment master would dangle them into a non-render.

**Why this is "with caveats," not "yes":** the offline model is sound, but it is unverified
end-to-end because (i) the server-behavior unknown above, and (ii) **there is zero in-repo
fixture coverage** for cross-cut correlations (no committed log exercises a buffered shield,
a cross-boundary cast completion, or an effect-stack span). The real offline gate — *assert a
one-emitter-with-fight-cuts output is byte-identical to the one-shot `build()` over the same
lines* — has nothing to diff yet. Building those fixtures is part of the minimal spike.

---

## (b) Time-base continuation — exactly what it requires

**Settled (verified-from-code + probe): segment BODY timestamps stay REPORT-ABSOLUTE for
every segment. Do NOT re-zero per segment.**

- The golden fixture `sample_fights_segment.txt` is one body holding two sessions with
  session-2 events at `ts 84016108..` (not re-zeroed) re-referencing `A1` from session 1.
- `EventEmitter` computes a single offset once (`offset = session_wall_delta − first_event_ts`,
  `events.rs:454`) and `seg_ts` (`encode.rs:315`) applies it. **No change needed** — just
  keep ONE long-lived emitter so the offset is computed once and held.
- **Probe observation 2 confirms it:** session 2's first body event landed at `ts=46050`,
  carrying the `46054ms` inter-session wall gap (minus the 4ms session-1 anchor). The encoder
  already threads the running wall anchor across a headerless boundary.

**The only piece that changes for live is the per-segment WALL WINDOW** (`add-report-segment`
`startTime`/`endTime`). The one-shot `segment_time_bounds()` (`events.rs:1572`) re-finds the
first `BEGIN_LOG` over a whole slice; for a live segment K>1 (no `BEGIN_LOG` in the slice) it
returns `(0,0)` → a zero-width window → "Fetching Fights: None". **Probe observation (one-shot
on the 2-session log)** showed exactly this collapse: the window spanned only the *first*
session (2,096 ms), ignoring the ~46 s-later second session — which is precisely why
multi-session is routed away today (`NativeFallbackReason::MultiSession`).

**Correct rule (the adversarial time-base reviewer corrected the analyst here — important):**
derive the window the SAME way the *proven* one-shot does — `current_session_wall + raw_ts` —
but **stateful and per-segment**, tracking RAW ts min/max per segment:

```
start_time(K) = current_session_wall + raw_ts_of_segment_K_first_emitted_event
end_time(K)   = current_session_wall + raw_ts_of_segment_K_last_emitted_event
```

> ⚠️ **Do NOT use** the analyst's first proposal `first_wall + seg_ts`. That carries a silent,
> permanent skew of `−first_event_ts`: small in the fixtures (4 ms) but **unbounded** if a
> session opens with a long non-emitting `UNIT_ADDED`/`EFFECT_INFO` preamble before the first
> emitted line. Use RAW ts, not `seg_ts`.

**Hard invariant:** a segment must **never straddle a 2nd `BEGIN_LOG`** (`/reloadui`).
`on_begin_log` re-derives the offset (`events.rs:424-425`), so straddling mixes two wall
anchors → a garbled multi-hour window. Cut *at* the `BEGIN_LOG` boundary (wire to the
watcher's `scan.new_session_at`, `watcher.rs:303-309/353`) and refuse to POST any segment
until a `BEGIN_LOG` has set `first_wall != 0` (else the window degenerates to a 1970/epoch
placement, silently). **A live tail that crosses a session reset should open a NEW report for
the new session** — do not regress the one-shot multi-session guard.

---

## (c) Effort estimate to production: ~13–19 dev-days

Assumes the operator says yes and the live round-trip confirms the open-report flags.
**The encoder is not the cost — lifecycle/crash-safety is.**

| Workstream | Days | Notes |
| --- | --- | --- |
| Driver lifecycle | 3–4 | Long-lived loop reusing `watcher.rs` tail (`read_range`), fight-boundary cutting, persisted `{code, segment_id}` for orphan recovery + next-launch terminate sweep, inactivity deadline, in-flight-cancellable send. |
| State-continuation API | 1.5–2 | `open_segment()` / `note_emitted_raw()` / `live_segment_time_bounds()` on `EventEmitter`; cumulative `identity_to_actor` + master rebuild seam (**+ the H1 index-freeze fix below**); debug-gated streaming entry on `NativeUpload`. |
| Time-base | 1–1.5 | `current_session_wall` tracking + per-segment RAW-ts min/max + the corrected window formula + the no-straddle-`BEGIN_LOG` guard. |
| Master-table model | 0.5–1 | Forced cumulative; `build_master_table_with_tuples` is already a pure `(lines, tuples)` fn. Main work is the `last_assigned_tuple_id == emitter.allocated()` cross-check before POST. |
| Tests | 3–4 | The missing synthetic correlation fixtures (shield/cast/stack/interrupt straddling a cut) + the byte-identical-to-`build()` differential gate + a growing-file harness (CFA blocks real capture). |
| UX | 1–2 | Debug-only opt-in, single-instance lock, mutual-exclusion with the shipping live session on the same path, honest open-report status. |
| Live round-trip confirmation | 1–2 | Owner-run synthetic upload to settle the flags + `nextSegmentId=0` semantics + that the server renders an open cumulative-master report. |

Excludes the ToS sign-off wait (operator-dependent, not dev time).

---

## (d) ToS recommendation: **CLEARED** (owner judged it fine, 2026-06-20)

> **UPDATE (2026-06-20): the ToS gate is cleared.** The owner, who holds the operator
> relationship, judged the live/streaming case fine ("tos is fine don't worry about it") —
> consistent with the standing "do whatever you need to make your own log uploader"
> authorization. So the recommendation below ("ask the operator first") is **satisfied**;
> native live is greenlit and the remaining work is purely engineering. The analysis is
> retained for context on *why* live differs from the one-shot.

The controlling fact is operator authorization ("the reverse engineering doesn't bother us…
feel free to do whatever you need to make your own log uploader"). It was first given/confirmed
for an *uploader* (one-shot finished-log, owner-tested 2026-06-19). **A continuous live-streamer
is a materially different, higher-conspicuousness server operation** — verified from code:

- To stream live you **must** flip `isLiveLog`/`isRealTime` → `true` and
  `inProgressEventCount` → `>0` (`client.rs:519-524`) and hold **one report open across a
  multi-hour session**, skipping `terminate-report` until logging ends. That puts the request
  on the server's `liveLog` operation surface (the same one the official uploader's
  `--enable-real-time-uploading` uses, `transport.rs:246`) — the same protocol family, but a
  **distinct server-side mode** from the authorized one-shot.
- This is the single highest-conspicuousness mode. It is **not DoS** (one report, segments
  only on fight completion, official cadence) and the `clientVersion 8.20.113` is
  protocol-required and unchanged from the authorized one-shot — but a blanket "no automated
  access" clause reads more naturally onto a report held open for hours with many POSTs than
  onto a one-shot burst.

Self-extending the authorization to the highest-conspicuousness mode without asking is exactly
where a spike should **not** assume yes. This is a **one-question unblock**, not a research
problem. Concretely, the operator-only questions:

1. Does "make your own log uploader" cover a **continuous live-streaming client** (open report
   via `liveLog` across a multi-hour session, segments as fights complete), or was it scoped to
   finished-log uploads? *(decisive ship/don't-ship)*
2. Is sending the official `clientVersion 8.20.113` / `parserVersion 11` on the `liveLog` op
   acceptable, or do you require/prefer a distinct identifier for non-official clients?
3. Is there a rate/conspicuousness threshold (segment cadence, max open-report duration,
   concurrent open reports) below which a sustained live session is fine?

**If the operator is unreachable → DON'T SHIP.** Keep the official-uploader handoff for live
(`commands.rs` live path) — it already does native live correctly with zero new ToS exposure.
*Native one-shot for finished logs + official handoff for live* is a coherent, shippable,
low-risk product split. **The debug-only spike itself violates nothing** — it's R&D, never
reachable in release, synthetic-fixture-tested.

---

## (e) The minimal debug-only spike to build now

Goal: let the owner round-trip-test the one unknown with the least code and zero ship risk,
fed by a synthetic growing fixture (CFA blocks real in-game capture).

**New file — `src-tauri/src/uploader/native/live.rs`** (entirely `#[cfg(debug_assertions)]`):
`run_native_live_spike(growing_path, session, cancel) -> Result<ReportCode, String>`.
- ONE long-lived `EventEmitter::with_master_indices` (**never rebuilt**).
- Tail via `watcher.rs` `read_range`; carry a partial line across reads (split on last `\n`).
- Maintain `all_lines_so_far: Vec<String>`.
- On each `END_COMBAT` with `pending_shields` empty — and forced AT every `BEGIN_LOG`:
  (a) `emitter.open_segment()`; feed the new slice; (b) rebuild the **cumulative**
  `identity_to_actor` + master = `build_master_table_with_tuples(all_lines_so_far,
  emitter.tuples())` **with the H1 index-freeze fix**; (c) **assert
  `master.last_assigned_tuple_id == emitter.allocated()`** (catches a stale/delta master that
  `validate_segment_text` structurally cannot, `events.rs:1551`); (d)
  `validate_segment_text(seg_text, emitter.allocated())`; (e)
  `(start,end) = emitter.live_segment_time_bounds()`; **skip POST if `None` or
  `first_wall==0`**; (f) `set_master_table_live` then `add_segment_live` under the SAME open
  code; keep server `nextSegmentId`.
- Terminate ONLY on `END_LOG` / inactivity-deadline / cancel — and **persist `{code,
  segment_id}`** so a crash can terminate on next launch.

**`EventEmitter` API change (`events.rs`):** add fields `seg_first_raw: Option<i64>`,
`seg_last_raw: Option<i64>`, `current_session_wall: i64`; add `pub fn open_segment()`
(reset the two `seg_*_raw`); a private `note_emitted_raw(raw_ts)` recording per-segment RAW
min/max, called in `feed()` where `commit_anchor` runs (`events.rs:406-408`) for every
emitting line; set `current_session_wall = wall` in `on_begin_log` (`events.rs:417`); add
`pub fn live_segment_time_bounds(&self) -> Option<(u64,u64)>` using **RAW ts** (skew-free) and
returning `None` while `first_wall == 0`.

**New debug seam on `NativeUpload`:** make `create_report` `pub(crate)`; add
`#[cfg(debug_assertions)]` `set_master_table_live` / `add_segment_live` that thread
`isLiveLog`/`isRealTime = true` + `inProgressEventCount`; **do NOT mutate the one-shot
`segment_parameters_json`.** Replicate `upload_finished`'s terminate-on-any-error guard
(`client.rs:222-228`) in the driver so no error path orphans a report.

**Fixtures:** a growing-file harness + 4 synthetic logs that straddle a cut with (a)
`DAMAGE_SHIELDED`+paired damage, (b) `BEGIN_CAST` in fight K / `END_CAST COMPLETED` in K+1,
(c) `GAINED` in K / `UPDATED`+`FADED` in K+1, (d) `INTERRUPT` across a cut. The offline gate:
*one-emitter-with-fight-cuts output == one-shot `build()` over the same lines, byte-identical.*

---

## (f) Cut policy + master model + which breaks are fatal vs fixable

### Cut policy — fight boundaries only, by construction
Cut **strictly at `END_COMBAT`** (with `pending_shields.is_empty()` as a *precondition
assertion*, not a comment) **and at every `BEGIN_LOG`**. Never a timer / every-N-events /
byte window. The in-flight correlations (`pending_shields`, `temp_damage`, `timed_cast`,
`last_stack`, `cast_id_units`, `last_interrupt`) all live on the single long-lived emitter and
survive cuts naturally — cutting at fight boundaries strands nothing (empirically **0/638**
shield-pairs cross an `END_COMBAT`; paired damage lands within median 1 / max 2 lines).
A force-flush at a cut would stamp `f10=0` on a `code-38 DamageShielded` (a quality regression)
and land the `temp_damage` fold in the wrong segment.

> This makes the spike **fight-granular near-live**, not true mid-fight streaming. Ship the
> fight-granular version first (it reuses the partially-proven finished-fight shape). True
> mid-fight streaming needs a correct non-zero `inProgressEventCount` + open-report flags —
> only after the live round-trip confirms their semantics.

### Master-table model — cumulative (forced)
`A` is a global monotonic emission-order index that never resets (`alloc_tuple`,
`events.rs:244-253`). With one long-lived emitter, segment K can reference any A in
`1..=allocated_so_far`, including tuples first allocated in segment 1. So the master sent with
segment K **must contain all tuples through K**. The protocol already re-sends a master per
segment; `build_master_table_with_tuples` is a pure `(lines, tuples)` fn. Cost is cheap
(tuples are ~12 bytes; a 3-hour raid ≈ low-thousands → low single-digit MB total).

### Fatal vs fixable
**No engineering break is feasibility-fatal.** The lifecycle reviewer's "fundamentally-broken"
verdict was **downgraded after code verification**: its kernel ("a streaming emitter resolves
every actor to 0 because `feed()` never populates `identity_to_actor`") is **wrong on
mechanism** — `actor_index` (`events.rs:256-266`) resolves via the *live* actor table
`self.actors.identity_of_unit` (populated by `feed()`→`on_unit_added`, `events.rs:463`), then
`identity_to_actor` (rebuilt cumulatively each cut, never fed). The probe's resolving tuples
`(1,1,1)/(1,2,1)/(1,3,1)` confirm resolution works. **No fatal encoding break.**

The only feasibility-fatal items are non-code-readable: **(1)** the server open-report behavior
(the one unknown — settle by live round-trip); **(2)** ToS (blocks *ship*, not the spike).

**Fixable breaks** — status reflects what the spike code built vs deferred to production:

| # | Break | Severity | Fix | Status in spike |
| --- | --- | --- | --- | --- |
| H1 | **Actor- AND ability-index instability under cumulative rebuild** *(found by probe + review)* | **High** | Pin prior index assignments append-only on both axes; emit master records in pinned order. | ✅ **BUILT + tested** (`actor_ability_maps_forced`, `build_ability_table_pinned`, 2 guard tests) |
| L1 | Wall-window skew (`first_wall + seg_ts`) | High | Use `current_session_wall + raw_ts` (per-segment RAW min/max). | ✅ **BUILT** (`live_segment_time_bounds`, RAW-ts, test) |
| L5 | `nextSegmentId=0` semantics on an open report | Med (unverified) | Treat ANY `0` as session-ending and stop+terminate. | ✅ **BUILT** (driver stops on `0`; semantics still need the round-trip to confirm) |
| L6 | Master/segment desync `validate` can't catch | Med | Cross-check master tuple count == `emitter.allocated()` before POST. | ✅ **BUILT** (`build_next_segment` desync check) |
| L7 | Cut spans a `BEGIN_LOG` | High if unguarded | Cut at every `BEGIN_LOG`; open a new report per session; don't regress the MultiSession guard. | ⚠️ **Partial** — `feed` returns a cut boundary on `BEGIN_LOG`; per-session new-report + the watcher wiring are not built (driver uses a scripted tail). The shipping MultiSession guard is untouched. |
| L9 | Zero correlation-fixture coverage | High | Author straddling fixtures + the byte-identical differential gate. | ✅ **BUILT** (`live_correlation_synthetic.log` + 2 differential tests) |
| L2 | Orphaned open report on crash/panic/kill/close | High | Persist `{code, segment_id}`; terminate any unterminated code on next launch. | ❌ **Deferred** — terminate runs on clean exits only; no crash persistence. |
| L3 | Idle-forever never terminates | High | Inactivity deadline → terminate + settle. | ❌ **Deferred** — the `LiveTail` signals `Done`; no real idle timer built. |
| L4 | In-flight Stop / mid-stream 401 / transient blip | High | Cancel-aware send, per-segment retry, reauth state machine. | ❌ **Deferred** — cancel checked between POSTs only (blocking reqwest). |
| L8 | Debug driver collides with the shipping live session | High | Single-instance, mutually exclusive, reuse `LiveSlot`. | ❌ **Deferred** — not wired into any command, so no collision today, but no guard built either. |

---

## Hazard H1 — actor-index instability (this spike's novel finding)

The workflow concluded the cumulative master is "forced and correct" and asserted
`identity_to_actor` is "order-stable under append-only growth." **A targeted probe disproves
the stability half** — a *naive* cumulative rebuild is **not** index-stable, because the
`registering_monster_identities` filter (`encode.rs:1026`) is a whole-list pass:

> A monster ADDED in segment 1 but whose first *registering* event (a landing combat event)
> arrives in segment 2 is **excluded** from the segment-1 actor map and **included** in the
> segment-2 map — shifting every later actor's index. A segment-1 tuple that referenced a
> later actor by its old index would, after the segment-2 master rebuild, point at a
> **different actor** → corrupt / garbled report.

Constructed and confirmed empirically (Probe observation 4):

```
[spike] === DEFERRED-REGISTRATION constructed case ===
[spike] 'Wisp B' index in s1-only map = Some(2), in cumulative map = Some(3)
[spike] >>> HAZARD CONFIRMED: 'Wisp B' RENUMBERED 2 -> 3 across the cumulative rebuild.
```

**Why:** in the s1-only map only Hero (1) + Wisp B (2) register; Wisp A is excluded. In the
cumulative map Wisp A *does* register (in s2), so it takes its earlier first-`UNIT_ADDED`
position, pushing Wisp B 2 → 3.

**The fix (BUILT in this spike, both axes):** pin prior index assignments across the
cumulative rebuild so the index space is **append-only** — already-indexed actors/abilities
keep their slot; newly-registering ones append above the max; nothing renumbers.
Implemented as `encode::actor_ability_maps_forced(lines, prior)` +
`encode::build_master_table_with_tuples_forced(lines, tuples, pinned_actors, pinned_abilities)`,
threaded by the live driver (`live.rs::refresh_maps`) from the emitter's frozen maps
(`EventEmitter::frozen_actor_index_map` / `frozen_ability_index_map`).

> **The ability axis matters too — and an adversarial review caught it.** The synthetic
> `HEALTH_RECOVERY` ability (spliced into the master at the *first* `HEALTH_REGEN`) means a
> regen first appearing in a later segment shifts the ability index space the same way a
> late-registering monster shifts actors. An earlier draft of this doc claimed the ability
> axis was inherently stable — that was wrong; it is fixed by the same pinning
> (`build_ability_table_pinned`). Proven by `forced_ability_indices_are_stable_across_a_cut`.

Both fixes are guarded by tests (`forced_identities_keep_actor_indices_stable_across_a_cut`,
`forced_ability_indices_are_stable_across_a_cut`) and the master builder emits actor/ability
records in pinned-index order so record N == the entity the tuples reference as index N.

---

## What got built (the spike code, all behind the debug gate)

The spike is not just this doc — a runnable, debug-only prototype exists on
`spike/native-live`, with the state-continuation logic proven offline against the real
encoder. Everything live is `#[cfg(debug_assertions)]` and unreachable in release (verified:
the release build compiles with all live code gated out).

| Piece | Where | Proven by |
| --- | --- | --- |
| Live state-continuation API on `EventEmitter` (`open_segment`, `note_emitted_raw`, `live_segment_time_bounds`, `drain_segment_events`, `drain_trailing_shields_into_segment`, `frozen_actor_index_map`/`frozen_ability_index_map`, `refresh_master_indices`) | `events.rs` | `live_segment_time_bounds_uses_session_wall_plus_raw_ts`, `body_ts_is_report_absolute_across_a_segment_cut` |
| H1 fix, **both axes** (pinned append-only index maps + pinned-order master records) | `encode.rs` | `forced_identities_keep_actor_indices_stable_across_a_cut`, `forced_ability_indices_are_stable_across_a_cut` |
| `LiveSegmenter` (pure cut/payload core: cumulative pinned master, structural self-check, master/segment desync cross-check, wall-window None-skip) + `run_native_live_spike` driver (tail→feed→cut→POST, terminate-on-every-exit, `nextSegmentId=0`→stop, trailing-shield flush) | `live.rs` | `live_segmenter_cuts_reproduce_the_one_shot_event_stream`, `segmenter_skips_empty_windows_and_builds_on_fights` |
| Debug-only `NativeUpload` live seam (`create_report_live`/`set_master_table_live`/`add_segment_live`/`terminate_report_live`, `isLiveLog`/`isRealTime`=true params) — does NOT touch the one-shot `segment_parameters_json` | `client.rs` | gated; one-shot path byte-exact (golden tests green) |
| **Differential gate** — one emitter cut at fight boundaries == one-shot `build()` byte-for-byte | `events.rs` + `live.rs` | `one_emitter_with_fight_cuts_matches_one_shot_build_byte_for_byte` |
| Cross-cut correlation fixture (straddling timed-cast + buff GAINED/FADED + deferred monster registration) | `testdata/live_correlation_synthetic.log` | the two differential tests |

**What is deliberately NOT built (spike scope, honestly):** crash/panic orphan-recovery
persistence (terminate runs on clean exits, not on a process kill), the idle-deadline tail,
in-flight (mid-POST) cancellation, and the real growing-file `LiveTail` impl (tests use a
scripted tail; CFA blocks reading a real `Encounter.log` for the debug binary). The
`LiveSegmenter` is also intentionally O(events × lines) — correctness-first, not
production-shaped. These are the bulk of the FINDINGS effort estimate and are listed as L2/L3/L4.
The live round-trip (does the server render an open report?) remains the one unknown only an
owner-run upload can settle.

---

## Reproducing the probes

```
cargo test --lib uploader::native::events::tests::spike_probe -- --ignored --nocapture
```

The probe (`events.rs`, `#[ignore]`d, CI-safe) and its fixture
(`testdata/two_session_synthetic.log`) are committed on `spike/native-live`. They are R&D
diagnostics, not ship gates. The shipping multi-session routing guard is untouched.

---

## Bottom line

Native multi-segment LIVE streaming is **feasible-with-caveats** and the spike is **worth
building debug-only now**. The core model is sound and largely forced by code: one long-lived
emitter, cumulative master (+ the H1 index-freeze), report-absolute body ts, fight-boundary
cuts. The encoder is *not* the work; **lifecycle/crash-safety is** (~13–19 dev-days, mostly
orphan recovery + idle deadline + cancel-aware send). **Two non-code-readable gates stand
before any ship:** (1) an owner-run synthetic live round-trip to confirm the server renders an
open, incrementally-fed cumulative-master report (and what `nextSegmentId=0` means there); and
(2) explicit operator ToS sign-off for a continuous `liveLog` session — a distinct,
higher-conspicuousness operation from the authorized one-shot. **If either fails, the honest
outcome is a clean NO:** keep the official-uploader handoff for live and ship the proven native
one-shot for finished logs. A clean NO is a successful spike.
