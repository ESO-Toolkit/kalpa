# Log Uploader Audit — Findings & Implementation Plan

> **Status (2026-07-13): all 25 findings implemented.** PR #220 (merged
> 2026-07-02) shipped every fix in this plan. The document is retained as a
> historical record — for the "verified sound" list, the must-not-change
> invariants, and the test-gap catalog. `file:line` references target commit
> `4fcfe19` and no longer match the current tree.

Audit of the ESO Logs uploader workspace (added in `4fcfe19`, PR #157): the core
Rust uploader (`src-tauri/src/uploader/`), the native direct-upload path
(`src-tauri/src/uploader/native/`), and the React frontend
(`src/components/uploader/`). Every finding carries exact `file:line` evidence
against the tree at commit `4fcfe19`; the four highest-severity findings were
independently re-verified by hand. This document is self-contained: an
implementer needs only this file and the repository checkout.

Audited: 2026-07-01.

## Summary

| ID | Title | Sev | Category | Conf | Effort |
|---|---|---|---|---|---|
| A1 | Native live tail loop is not cancel-aware — Stop can hang ~30 min | High | Cancellation | Confirmed | M |
| A2 | `uploader_stop_live` is sync and joins threads on the main thread — UI freeze | High | Concurrency | Confirmed | S |
| A3 | Esc/backdrop/X during native live silently stops upload & closes report | High | UX/Reliability | Confirmed | S |
| B1 | Logout leaves a valid ESO Logs session in the webview cookie jar | Medium | Security | Confirmed | M |
| B2 | `split_by_session` accepts unbounded caller session list (disk-fill DoS) + no name de-dup | Medium | Security | Confirmed | S |
| B3 | PRIVACY.md contradicts the shipped uploader (found twice independently) | Medium | Privacy/docs | Confirmed | S |
| B4 | Import extension check runs pre-canonicalization (symlink bypass) | Low | Security | Plausible | S |
| C1 | Manual native routing ignores sign-in state → Failed instead of handoff | Low | Reliability | Confirmed | S |
| C2 | One-shot native upload has no crash-recovery orphan breadcrumb | Low | Reliability | Confirmed | M |
| C3 | Default reqwest redirect policy can misclassify auth expiry as fatal | Low | Reliability | Plausible | S |
| C4 | Fixed 120s request timeout too small for large segments on slow uplinks | Low | Reliability | Plausible | S |
| C5 | `count_master_tuples` text heuristic can fatally kill a healthy live session | Low | Reliability | Confirmed | S |
| C6 | `uploader_attach_report` mutates history outside `MUTATION_LOCK` (lost-update race) | Low | Concurrency | Confirmed | S |
| D1 | Live emitter's per-cast maps grow unbounded over multi-hour sessions; dead `segment_lines` buffer | Medium | Memory | Confirmed | M |
| D2 | One-shot upload double-buffers the entire events output | Med-Low | Memory | Confirmed | S |
| D3 | Native routing scan (≤256 MiB read) runs inline on the async executor (found twice) | Low | Performance | Confirmed | S |
| D4 | Every keystroke in Report-name re-renders the whole ~4,100-line workspace | Medium | Performance | Confirmed | S |
| E1 | Boss-name heuristic matches `isLocalPlayer=T` → fights named after the player | Low | Correctness | Confirmed | S |
| E2 | Live watcher drops zone/boss context between reads → most live fights unnamed | Low | Correctness | Confirmed | M |
| E3 | Live mask classification missing two friend/foe clauses vs one-shot (+ stale comment) | Low | Correctness | Confirmed | M |
| F1 | "You'll confirm visibility in the official uploader" copy false on CLI transport | Low | UX copy | Confirmed | S |
| F2 | Drag-drop listener leaks on unmount race | Low | Correctness | Confirmed | S |
| F3 | Duplicated `refreshNativeState` copies already drifted (error-handling split-brain) | Low | Quality | Confirmed | S |
| F4 | Dead `StatusPill`, write-only `liveStatus`, never-produced `UploaderStatus` variants | Low | Quality | Confirmed | S |
| F5 | Decompose `uploader-workspace.tsx` at three clean seams (enables the missing tests) | Medium | Testability | — | L |

## Suggested implementation order

- **Batch 1 (A1→A2→A3)** — Stop/close correctness. A1 before A2 (A2's async join is only prompt once the tail cancels promptly). A3 is frontend-only, independent, but ships with these.
- **Batch 2 (B1–B4)** — security/privacy. Independent of each other.
- **Batch 3 (C1–C6)** — reliability hardening. Independent.
- **Batch 4 (D1–D4)** — performance/memory. D1/D2 must be validated byte-for-byte against golden tests. D3 is trivial.
- **Batch 5 (E1–E3)** — naming/classification correctness. E2 builds on E1's tests.
- **Batch 6 (F1–F5 + test gaps)** — polish and decomposition. F5 last; do not mix with behavior fixes.

Baseline check before starting: `cargo test` in `src-tauri/`, `npm run test` at repo root; note pre-existing failures.

## Findings

### A1: Native live tail loop is not cancel-aware — Stop can hang up to 30 minutes
- **High / Cancellation / Confirmed / M**
- **Location**: `src-tauri/src/uploader/native/live.rs:1574-1657` (`NotifyTail::next_lines`), `live.rs:889-918` (`LiveDriver::run` checks cancel only between polls), `live.rs:450` (`IDLE_DEADLINE = 30min`), `src-tauri/src/uploader/commands.rs:69-90` (`NativeLiveHandle::shutdown` sets cancel then `join()`)
- **Problem**: `next_lines` returns only on `Lines`/`Idle`/`Ended`/`Error`; its `recv_timeout(POLL_INTERVAL)` loop never reads the driver's cancel flag (it has no reference to it). Stop joins the driver thread, which is parked in `next_lines` until the file grows or the 30-minute idle deadline fires. The ~250ms cancel-aware POST machinery is defeated by the tail. Existing tests use only `ScriptedTail` (returns immediately), so this is untested.
- **Failure scenario**: Start native live before launching the game (or idle in town so `Encounter.log` stops growing), click Stop → the Stop command (and any new start on the same path, which stops the previous handle first at `commands.rs:1384`) hangs up to 30 minutes.
- **Fix**: Add `cancel: Arc<AtomicBool>` to `NotifyTail` (pass into `NotifyTail::new(...)` from the start branch, cloning the same `Arc` as `driver_cancel`). Add `TailOutcome::Cancelled`; check `self.cancel.load(SeqCst)` at the top of each loop iteration (right after `recv_timeout`). In `LiveDriver::run`, map `Cancelled => return EndReason::Stopped` — the existing exit path then runs `terminate_report_and_settle` on its fresh cancel flag; that discipline must not change. Apply the same check to the debug `FileTail` loop (`live.rs:1299-1382`). Do not change `wait_for_send_or_cancel`/terminate semantics.
- **Verification**: Unit test: `NotifyTail` on a temp file that never grows; trip cancel from another thread after ~100ms; assert `Cancelled` in <1s. Driver-level: stop against a static file, assert total stop latency <2s and terminate-report attempted.

### A2: `uploader_stop_live` is sync and joins live threads on the Tauri main thread
- **High / Concurrency / Confirmed / S**
- **Location**: `commands.rs:2227-2264` (sync `pub fn uploader_stop_live`), `commands.rs:2151-2159` (`stop_handle_blocking` joins inline), `commands.rs:2142-2149` (correct async helper `stop_handle_off_executor`, used by `uploader_start_live` but not by Stop)
- **Problem**: In Tauri 2, non-`async` commands run on the main event-loop thread. The command's own comment ("already off the async executor") is wrong about which thread it protects — it blocks the main thread, freezing rendering, all IPC, and the tray. Even after A1, the join covers the driver's exit terminate (up to `TERMINATE_DEADLINE` 5s, plus `CREATE_REPORT_GRACE` 10s if stop lands during create-report) — up to ~15s of UI freeze; before A1, up to 30 minutes.
- **Fix**: Make the command `async fn` and replace `stop_handle_blocking(handle)` with `stop_handle_off_executor(handle).await`. Keep the locked `stop_slot_in_map` block exactly as-is (the store-cancel-before-remove invariant at `commands.rs:2108-2135` must not change), keep the `!was_native → settle_live` tail. Delete the now-dead `stop_handle_blocking`. No frontend change (invoke is promise-based).
- **Verification**: Extract `async fn stop_live_impl(...)` if needed for testing; keep `stop_slot_in_map` tests green. Manual: stop a native session with the network blackholed; the window keeps repainting.

### A3: Accidental dialog close silently kills a native live upload and closes its report
- **High / UX-Reliability / Confirmed / S**
- **Location**: `src/components/uploader/uploader-workspace.tsx:1364` (`onOpenChange={(o) => !o && onClose()}`), `:514-547` (unconditional unmount cleanup → `uploader_stop_live`), `:1445-1453` (Manual-tab switch → `void handleStopLive()`), `src/components/ui/dialog.tsx:34-73` (Esc/backdrop dismiss + X always rendered)
- **Failure scenario**: Mid-raid with 12 fights posted, user hits Esc or misclicks the backdrop → report closed on esologs.com. Restarting opens a NEW report and live skips completed fights (`:3695-3705`), so the night is permanently split across two reports. Only feedback is an 8-second toast after the fact.
- **Fix**: Gate `onOpenChange`: if `liveSessionIdRef.current !== null`, show a confirm dialog instead of closing (reuse the `DeleteLogConfirm` pattern at `:1678-1721`; branch copy on `liveHandedOffRef.current` — native: "Stop the live upload and close the report on ESO Logs?"; handoff: "Stop tracking in Kalpa?"). Apply the same gate to the Manual `ModeTab` onClick at `:1452`. Keep the unmount cleanup exactly as-is (last-resort teardown for app quit). Do not change `handleStopLive` semantics.
- **Verification**: e2e (CDP pattern from `e2e/settings-dialog.spec.ts`): with a session active, Escape → confirm appears, `uploader_stop_live` not invoked until confirmed. Unit-test the extracted guard.

### B1: "Sign out of uploads" leaves a valid ESO Logs web session in the webview profile
- **Medium / Security-Privacy / Confirmed / M**
- **Location**: `commands.rs:994-1000` (`uploader_logout_esologs` → only `session.invalidate()`), `native/session.rs:194-198` (clears memory + credential store only), `native/login.rs:235-249` (login webview uses the persistent WebView2 profile; jar never cleared), `PRIVACY.md:106` ("No tokens are retained after sign-out")
- **Problem**: WebView2 persists the login webview's cookie jar (`wcl_session`, long-lived `remember_web_*`) on disk. Logout clears only the Credential-Manager copy; nothing clears the profile and no server-side revocation happens. Clicking "Sign in" after sign-out auto-completes with zero interaction — sign-out is effectively cosmetic.
- **Fix**: In `uploader_logout_esologs` (or a `login.rs` helper): after `session.invalidate()`, get/build the `LOGIN_WINDOW_LABEL` window (hidden, `.visible(false)` if absent), call `WebviewWindow::clear_all_browsing_data()`, close it. Optionally best-effort `POST https://www.esologs.com/logout` with the old cookie first. **Must not change**: `invalidate()` on 401/419 mid-upload must NOT clear the webview jar (that would break the reauth pause→re-login UX); only explicit user sign-out clears it. Update PRIVACY.md per B3.
- **Verification**: Manual: sign in → sign out → sign in shows the login form, not auto-auth. Automated: seam/mock asserting logout calls the clearing path.

### B2: `split_by_session` copies an unbounded caller-supplied session list; no name de-dup
- **Medium / Security (DoS) / Confirmed / S**
- **Location**: `commands.rs:592-614` (IPC `sessions: Option<Vec<LogSession>>` passed through), `splitter.rs:269-308` (`split_by_session`: no cap, no `unique_name`), `splitter.rs:314-352` (`resolve_sessions` trust gate validates fingerprint/`max_end` but not count/overlap). Precedent: `splitter.rs:389-395` (`split_selected` caps at `MAX_SELECTIONS = 256` citing "a compromised webview"), `splitter.rs:626-639` (fights: 1024 + de-dup)
- **Failure scenario**: Compromised webview sends `[genuine session 0, then 10⁴ crafted whole-file sessions]` for a 2 GiB log → backend writes up to 10⁴ × 2 GiB into app data with no cancel path. Also `session_file_name` (`splitter.rs:185-192`) keys on caller-controlled `index`/`start_time_ms` — duplicates silently overwrite (`File::create` truncates). Secondary: `resolve_scan`'s caller `fights` vec is unbounded → O(selections × fights) CPU stall.
- **Fix**: In `resolve_sessions`, reject caller lists `> 256` inside the `Some(s) if !s.is_empty()` arm before the trust checks. Route `split_by_session` names through `unique_name` (`splitter.rs:478`) like `split_selected` does. Cap the caller `fights` list in `resolve_scan` (e.g. 100 000). **Must not change**: trust-gate semantics (length/fingerprint/appended-`BEGIN_LOG` rescan) and the final-session extend-to-EOF clamp.
- **Verification**: 257 crafted sessions → `Err`; two sessions with identical index/start_time → two distinct files (`-2` suffix); existing splitter tests stay green.

### B3: PRIVACY.md contradicts the shipped uploader
- **Medium / Privacy-docs / Confirmed (twice independently) / S**
- **Location**: `PRIVACY.md:59-61` ("No combat logs … is accessed"), `:69-76` ("No ESO game data"), `:17-27` (credential table omits the upload session cookie), vs `commands.rs:1026-1060` (uploads combat-log contents), `native/login.rs:80-91` + `token_store.rs:244-268` (session cookie captured and persisted), `native/format.rs:74` (`FORMAT_VERSION_CONFIRMED = true` — the native path is live)
- **Fix**: Add a "Direct upload to ESO Logs (opt-in)" section: combat-log contents are uploaded only on user action; visibility is user-chosen, default **Unlisted**; the upload session cookie is stored in the Windows Credential Manager and removed on sign-out (align wording with post-B1 behavior); the handoff path launches a third-party app. Amend the "Do NOT Collect" bullet to "never uploaded without your action"; add the `upload_session` cookie row to the local-data table; bump "Last updated".
- **Verification**: Doc review only.

### B4: Import `.log` extension check runs before canonicalization — symlink bypass
- **Low / Security hardening / Plausible / S**
- **Location**: `commands.rs:631-648` (`is_log` computed on raw `src`, then `dunce::canonicalize`)
- **Failure scenario**: symlink `report.log → <arbitrary readable file>` passes the extension gate on the link name; contents get copied into the Logs sandbox (and are then preflightable/uploadable if they scan as a valid log). Kept Low: Windows symlink creation needs admin/dev-mode. Contrast: `confine_log_path` already checks containment on the canonical target.
- **Fix**: After canonicalization, re-check the extension on `canonical_src` with the same closure; reject on mismatch. Keep the raw-path fast-fail. Factor into a pure `fn validate_import_source(raw: &Path, canonical: &Path) -> Result<(), String>` for testability.
- **Verification**: unit test the pure fn with mismatched-extension inputs; `#[cfg(unix)]` symlink test.

### C1: Manual native routing ignores sign-in state — fails instead of handing off
- **Low / Reliability / Confirmed / S**
- **Location**: `transport.rs:679-687` (`assess_native_routing(log_path, opt_in)` — no session param, unlike `assess_native_live_routing(opt_in, has_session)` at `:649-664`), `commands.rs:1126-1128` (call site has `session` in scope), `native/client.rs:224-225` (hard error "Not signed in")
- **Failure scenario**: Stored cookie invalidated by a prior 401 while the UI still shows signed-in → next manual upload routes Native, fails fast, history record settles **Failed** — where the design (plan §5) and the live path hand off to the official uploader instead.
- **Fix**: In `uploader_upload_log`, mirror the live gate: `native_opt_in && session.has_session() && matches!(assess_native_routing(...), Native)`; on missing session take the official path with a fallback note (reuse the live gate's copy). Or add `has_session: bool` to `assess_native_routing` to keep the decision in one place (preferred — one seam).
- **Verification**: unit test: opted-in + no session ⇒ Fallback routing, not a Failed record.

### C2: One-shot native upload has no crash-recovery orphan breadcrumb
- **Low / Reliability / Confirmed / M**
- **Location**: `native/client.rs:190-243` (`upload_finished` — best-effort terminate covers error paths only), `native/orphans.rs` (writers are live-only)
- **Failure scenario**: Kill/panic/power-loss between `create-report` and `terminate-report` during a manual upload → open draft orphaned server-side with no `{code}` breadcrumb; next-launch recovery can't close it (the L2 hazard, solved for live only).
- **Fix**: Thread an orphan sink into `NativeUpload::upload_finished`: `record_open(code, 1)` right after `create_report()`, `note_segment` per accepted segment, `clear` only after a definitive terminate (reuse `is_definitively_closed` semantics exactly as `terminate_report_and_settle` does). `run_native_upload` gains the sink param from `uploader_upload_log`; a `NoopOrphanSink` keeps existing tests unchanged.
- **Verification**: fake-sink unit test asserting record-open-before-first-POST and clear-only-on-confirmed-close ordering (mirror `orphans.rs` tests).

### C3: Default reqwest redirect policy can misclassify auth expiry as fatal
- **Low / Reliability / Plausible / S**
- **Location**: `native/client.rs:411-419` (`send_once` client, no `.redirect(...)`), `:794-801` (`live_send_once`, same)
- **Problem**: If an expired session ever surfaces as `302 → /login` instead of 401/419, reqwest follows it (POST→GET), fetches login HTML with 200; JSON extraction fails → classified `Server{status:0}` → `RetryClass::Fatal`, bypassing the re-auth machinery. (No cookie-leak risk: reqwest 0.13 strips `Cookie` cross-host; all URLs from `DESKTOP_CLIENT_BASE`.)
- **Fix**: Build both clients with `.redirect(reqwest::redirect::Policy::none())`; classify any 3xx as `SendResult::AuthRejected` (conservatively, or when `Location` points at a login page) so the single re-auth retry and live pause-reauth engage. Keep 401/419 handling unchanged.
- **Verification**: refactor status classification into a pure fn over `(status, headers)`; test 302+Location=/login ⇒ AuthRejected.

### C4: Fixed 120s total request timeout too small for large one-shot segments on slow uplinks
- **Low / Reliability / Plausible / S**
- **Location**: `native/client.rs:411-414`, `:794-797` (`.timeout(120s)` covers the whole multipart body upload)
- **Failure scenario**: Near-ceiling upload → tens-of-MB segment ZIP; at ≤2 Mbps the POST exceeds 120s ⇒ `Transport` error ⇒ one-shot path (no retry) terminates the report and fails; retrying repeats. Live segments are small, unaffected.
- **Fix**: Scale the timeout with payload size for segment/master-table POSTs (e.g. `120s + bytes/(64 KiB/s)`, capped ~15 min), keep 120s for create/terminate. Put the computation in a pure fn.
- **Verification**: unit-test the timeout computation.

### C5: `count_master_tuples` text heuristic can fatally kill a healthy live session
- **Low / Reliability / Confirmed-fragility / S**
- **Location**: `native/live.rs:1670-1681` (counts lines matching `int|int|int`), desync check at `live.rs:344-351` (mismatch ⇒ `PostOutcome::Fatal` ⇒ terminate)
- **Problem**: The cross-check re-derives the tuple count by pattern-matching rendered master text. Today no other record renders as exactly three numeric `|`-fields, but the invariant is implicit; any future record-shape change silently over-counts and terminates a healthy live session as "master/segment desync".
- **Fix**: Have `IncrementalMasterState::render_master` return the embedded tuple count it already computes (`incremental.rs:581`, `tuple_records.len()`) — return `(String, u64)` — compare that to `emitter.allocated()`; delete the text heuristic.
- **Verification**: existing `incremental_master_matches_rewalk_*` tests + a unit test that the returned count equals `tuples.len()`.

### C6: `uploader_attach_report` load→mutate→upsert outside `MUTATION_LOCK`
- **Low / Concurrency / Confirmed / S**
- **Location**: `commands.rs:2302-2313`, vs `history.rs:20-23` (`MUTATION_LOCK` exists to serialize exactly this) and `history.rs:148-165` (`upsert` replaces whole record)
- **Failure scenario**: attach races the native driver's settle: load(Live) → driver upsert(Completed) → attach upsert(stale Live + report) → record stuck Live; next-launch `reconcile_stale` mislabels it Failed (`history.rs:116-124`).
- **Fix**: Move the mutation into `history.rs` as a locked helper `attach_report(app, id, report)` (take `MUTATION_LOCK`, load, find by id — optionally restrict to `HandedOff | Completed | Failed` — set, save). Command keeps its URL validation (`commands.rs:2290-2300`) and calls the helper.
- **Verification**: unit test the pure `apply_attach_report(records, id, report)` helper.

### D1: Live emitter per-cast maps grow unbounded; dead `segment_lines` buffer
- **Medium / Memory / Confirmed / M**
- **Location**: `native/events.rs:149-198` (fields); insert-only sites `events.rs:1092` (`cast_id_units`), `:1105` (`instant_cast_ids`), `:1111` (`cast_track_to_a`), `:1122` (`timed_cast`), `:1311` (`reflect_casts`). Dead buffer: `native/live.rs:80,138` (`LiveSegmenter::segment_lines` — every line cloned, never read; confirmed write-only by the comment at `live.rs:409-412`)
- **Problem**: One emitter lives for the whole session (by design), but every `BEGIN_CAST` inserts permanently, keyed by unique `castTrackId`. A 4–6h raid at tens of casts/sec accumulates hundreds of thousands of entries (tens of MB + rehash churn) — against the "lightweight while gaming" goal. `segment_lines` is pure dead weight with per-line allocation in the hot path.
- **Fix**: (a) Delete `segment_lines` outright (pushes at `live.rs:138`, clears at `:225-230,291-303,373,416`). (b) Bound the cast maps — prefer a generation-based cap (keep the most recent ~50k cast ids, evict oldest) over eviction-on-END_CAST, because `cast_track_to_a` is read by later `EFFECT_CHANGED` lines and `cast_id_units` gates code-28 refs. **Gate**: any eviction must pass the full golden/differential suite byte-for-byte, including `one_emitter_with_fight_cuts_matches_one_shot_build_byte_for_byte`.
- **Verification**: golden tests + a synthetic long-session test (1M casts, assert bounded map sizes).

### D2: One-shot upload double-buffers the entire events output
- **Med-Low / Memory / Confirmed / S**
- **Location**: `native/events.rs:513-524` (`feed` unconditionally appends to `self.segment_events`), `transport.rs:798-820` (whole-file read + line vec), `events.rs:391-430` (`build` assembles its own `out`)
- **Problem**: One-shot never calls `open_segment()`, yet `segment_events` accumulates a second full copy of the events text. Peak memory for a 256 MiB log ≈ 4–5× input (>1 GiB transient).
- **Fix**: Gate the append behind a `live_tracking: bool` enabled only by the live path (e.g. a one-time `enable_segment_tracking()` called by `LiveSegmenter`). Byte-identical for live; one-shot never read the buffer.
- **Verification**: golden + live differential tests stay green; compare peak RSS via `src-tauri/src/uploader/bench.rs:160` on a large fixture.

### D3: Native routing scan (up to 256 MiB read) inline on the async executor
- **Low / Performance / Confirmed (twice independently) / S**
- **Location**: `commands.rs:1127` (`assess_native_routing` called directly in `async fn uploader_upload_log`), `transport.rs:679-759` (streams the whole file). Convention: module header `commands.rs:3-4`; the neighboring fight-count scan at `commands.rs:1069-1075` already uses `spawn_blocking`, as does the live equivalent at `:1424`.
- **Fix**: Wrap in `tokio::task::spawn_blocking` (clone `dispatch_path` into the closure). `fallback_note` derivation (`commands.rs:1140-1148`) only needs the returned value — unchanged.
- **Verification**: compile + existing `routing_tests` in `transport.rs`.

### D4: Every keystroke in Report-name re-renders the entire workspace tree
- **Medium / Performance / Confirmed / S**
- **Location**: `uploader-workspace.tsx:1536-1543` (`options` at workspace root, `onChange` per keystroke from `upload-options.tsx:62`), `:370-376` (localStorage persist per keystroke), `:1524-1525` (`rowsFromSummaries(fights)` re-mapped every render, up to 500 rows), `:1490` (inline `onRefresh` closure)
- **Fix**: (1) `useMemo` the `rowsFromSummaries(fights)` result; (2) wrap `FightList`, `HistoryPanel`, `LogPicker`, `Preflight` in `React.memo` (make `onRefresh` a stable `useCallback` first); (3) debounce the localStorage persist (~300ms). Do **not** add list virtualization — live is capped at `MAX_LIVE_FIGHTS = 150` (`:192`) and preflight fights at 500 (`commands.rs:491-496`).
- **Verification**: React Profiler while typing with a 500-fight preflight — only the options panel commits.

### E1: Boss-name heuristic matches `isLocalPlayer=T` — fights named after the player
- **Low / Correctness / Confirmed / S**
- **Location**: `scanner.rs:393-403` (`if line.contains(",T,")` then name from field 10)
- **Problem**: In `UNIT_ADDED`, `isBoss` is field 7 and `isLocalPlayer` field 4. The local player line (present in every session) contains `,T,` and field 10 is the character name (no `@` prefix), so `pending_boss` gets set to the player's name. The repo's own fixture proves the shape: `commands.rs:2409` — `0,UNIT_ADDED,1,PLAYER,T,1,0,F,3,9,"H","@h",…`.
- **Failure scenario**: `BEGIN_LOG` → player `UNIT_ADDED` → trash fight with no boss unit → `FightSummary.boss_name = Some("CharName")`, rendered by `f.bossName || f.zoneName` (`uploader-workspace.tsx:1214,2346`).
- **Fix**: Positional check: `if field(line, 7).is_some_and(|f| f.eq_ignore_ascii_case("T"))`; keep the field-10 read and `@` filter as defense in depth. Naming only — no offsets change.
- **Verification**: scanner unit tests: realistic player line + fight ⇒ `boss_name == None`; monster line with field-7 `T` ⇒ `Some(...)`. (The `UnitAdded` arm currently has zero test coverage.)

### E2: Live watcher drops zone/boss naming context between read passes
- **Low / Correctness (live UX) / Confirmed / M**
- **Location**: `watcher.rs:330` (fresh `Detector` per pass via `scan_chunk_for_fights`), `scanner.rs:515-521` (`pending_zone`/`pending_boss` start `None` every chunk), `watcher.rs:354-369` (`consumed` advances past dispatched fights, discarding naming lines)
- **Failure scenario**: Go live, zone in, pull 3× — fights 2..n carry `zoneName: None` (zoning happens minutes before pulls; the bytes holding `ZONE_CHANGED` were consumed in an earlier pass). The native path documents this as a follow-up (`commands.rs:1858-1863`); the official-watcher path has the same hole undocumented.
- **Fix**: Persist naming state across passes in `tail_loop`: extend `ChunkScan` with the detector's final `pending_zone`/`pending_boss` and add seed parameters to `scan_chunk_for_fights` (the `Detector` already has the fields); keep them as locals in `tail_loop` next to `session_open`; a `BEGIN_LOG` must clear the carry (mirroring `Detector::feed`'s BeginLog arm at `scanner.rs:358-360`). Preserve partial-line deferral, offsets, `new_session_at` semantics.
- **Verification**: carry test (chunk 1 = `ZONE_CHANGED` only; chunk 2 = fight ⇒ named) + reset test (carry cleared across `BEGIN_LOG`).

### E3: Live mask classification missing two friend/foe clauses vs one-shot
- **Low / Correctness (report quality) / Confirmed / M**
- **Location**: `native/events.rs:400-402` (one-shot `build()` installs `classify_monsters`), `events.rs:1284-1300` (live analog `note_raid_damage` — rfp-clause only), `native/encode.rs:1697-1723` (full rule: `rfp>0 || (d2p>0 && d2m==0)` force-hostile + `attacks_players` per-event override), stale comment at `events.rs:400` referencing a nonexistent `set_force_hostile`
- **Problem**: Live never runs `classify_monsters`, so a friendly-tagged enemy that attacks players but takes no raid damage is never forced hostile, and the per-event `attacks_players` mask flip never fires — a live report diverges from the same log uploaded one-shot. The byte-for-byte differential passes only because no fixture contains such a monster.
- **Fix**: Maintain the classification incrementally — extend `note_raid_damage` (or a sibling) to keep per-monster `d2p`/`d2m` counters and evaluate the same predicate per event; pin with a new fixture. Fix the stale comment regardless.
- **Verification**: add a fixture with a friendly-tagged player-attacking monster; extend the live-vs-one-shot differential — must fail before, pass after.

### F1: Visibility-confirm copy is false on the CLI transport
- **Low / UX copy / Confirmed / S**
- **Location**: `upload-options.tsx:186-190` vs `transport.rs:248-276` (headless CLI forwards `--report-visibility`, no confirm step)
- **Fix**: Pass `officialInstalled` (already on `transport` in the workspace) into `UploadOptionsControl`; branch the caption: CLI → "Applied when the official uploader runs — pick it here."; GUI fallback → keep current; native → unchanged.
- **Verification**: caption unit test per (willUseNative, officialInstalled).

### F2: Drag-drop listener leaks when unmount races `onDragDropEvent` resolution
- **Low / Correctness / Confirmed / S**
- **Location**: `uploader-workspace.tsx:780-809`; deterministic under StrictMode (`src/main.tsx:107`), effect also re-runs per `logsDir` change
- **Fix**: standard Tauri idiom — after the await: `if (!active) { fn(); return; } unlisten = fn;`.
- **Verification**: unit test with mocked `onDragDropEvent` resolving after cleanup; assert the unlisten runs.

### F3: Duplicated `refreshNativeState` copies drifted
- **Low / Quality / Confirmed / S**
- **Location**: `uploader-workspace.tsx:459-479` vs `:481-501` (mount effect is a verbatim-then-drifted copy: it wraps `uploader_has_session` in `.catch(() => false)` at `:486`; the named fn at `:467` does not, so a session-check failure skips ALL fail-closed state updates)
- **Fix**: add the `.catch(() => false)` to `refreshNativeState` and replace the mount-effect body with `void refreshNativeState()`.
- **Verification**: unit test with a rejecting `uploader_has_session` mock ⇒ `hasNativeSession=false`, opt-outs still applied.

### F4: Dead `StatusPill`; write-only `liveStatus`; never-produced status variants
- **Low / Quality / Confirmed / S**
- **Location**: `uploader-shared.tsx:23-49` (`StatusPill`, zero importers), `uploader-workspace.tsx:310` (`liveStatus` written 9×, read once as `=== "attention"` at `:1322`), `src/types/uploader.ts:164-170` (`"uploading"`/`"retrying"` never set)
- **Fix**: delete `StatusPill`; reduce `liveStatus` to a `needsAttention` boolean (or prune `UploaderStatus` to produced values). Purely mechanical.

### F5: Decompose `uploader-workspace.tsx` (4,095 lines) at three clean seams
- **Medium / Testability / L — do last, no behavior change**
- **Seams** (already prop/callback-only): (1) `useLiveSession` hook — state/refs `:293-352` + handlers `:862-1266`; extract the channel `onmessage` (`:932-1042`) into a pure `applyLiveEvent(ev, ctx)` reducer so dedup/StrictMode double-count/session-reset/stale-event-guard/stopped-eviction logic becomes unit-testable (the most intricate untested logic in the file); (2) `log-picker.tsx` (`:2738-3156`); (3) `history-panel.tsx` (`:3716-4095`, exporting `tidyLogLabel`/`sourceLocation` for tests).
- **Must not change**: the ref-based session-ownership protocol (`liveSessionIdRef`/`startingRef`/`liveActiveRef` semantics, exact ref-null-then-await order in `handleStopLive`), the unmount cleanup's empty-deps contract, and `handleStartLive`'s position-sensitive pre-start abort checks (don't split it across modules).

## Test-coverage gaps (add alongside the relevant batches)

1. `Detector` `UnitAdded`/`ZoneChanged` naming — zero coverage (E1/E2).
2. `watcher::tail_loop` — zero direct tests (truncation reset, mid-chunk `BEGIN_LOG` dispatch, failure-streak teardown, re-anchor ladder `watcher.rs:354-405`).
3. Stop latency against a blocked tail (A1) — only `ScriptedTail` exists today.
4. `split_by_session` bound/dedup (B2); `split_selected`'s existing cap is also asserted nowhere.
5. `uploader_upload_log`: a `spawn_blocking` panic returns `Err` before the record settle (`commands.rs:1161-1171`), leaving the record `Uploading` until next-launch reconcile mislabels it — one-line settle-on-task-error fix, untested.
6. Mock-HTTP for the one-shot path (terminate-on-error still untested; acknowledged in `native-uploader-next-steps.md`).
7. Frontend: LiveEvent reducer sequences (after F5), `loadSavedOptions` validation, `naming.ts`, `tidyLogLabel`/`sourceLocation`, a TS-side LiveEvent casing pin, and a first `e2e/uploader.spec.ts` (open dialog, assert visibility defaults to **Unlisted**, exercise the A3 confirm).

## Verified sound — do NOT "fix" these

TS/Rust type mirror + all 19 invoke call-sites (no drift); path confinement incl. UNC/device-namespace rejection and TOCTOU-closing canonical IO; visibility-id mapping (pinned twice); cookie redaction and logging hygiene; login capture gate (exact-host + path blocklist); token_store fail-closed chunked scheme; live-slot cancel-ordering invariants (store-cancel-before-remove — A2's fix must preserve this); client.rs terminate-on-error + breadcrumb clear-only-on-confirmed-close; splitter stale-offset trust gate; `MAX_NATIVE_BYTES` whole-file read is a documented design decision (route-away + defense-in-depth), not a bug — D2/D3 reduce its cost without changing the policy.
