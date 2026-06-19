# Native Uploader — Next Steps (session handoff, 2026-06-18)

Self-contained state + plan for finishing **fully-native** ESO Logs upload in Kalpa
(Kalpa POSTs directly to `esologs.com/desktop-client/*`; the official-uploader
"handoff" is a temporary fallback, NOT the goal). Read this first in a new session.

## The big strategic facts (decided / verified this session)

1. **Byte-exact subordinal-A is NOT required.** The working reference uploader
   (sheumais/logs — clean-room, read for facts only, never port/name) does NOT
   reproduce ESO Logs' exact `A` counter; it uses a simpler self-consistent index
   and users upload successfully. ESO Logs' server **re-parses** the segment and
   accepts any structurally-valid one. The old "acid 3733/3733" bar was a
   self-imposed safety gate, stricter than reality.
   → Do NOT keep grinding byte-exact A. It is off the critical path. The A counter
   only needs to be **internally consistent** (dense, first-sight allocation in
   raw-line order), not byte-identical to the official uploader's.

   ⚠️ **CORRECTION (2026-06-18): a prior version of this doc claimed "Our encoder
   (engine_v4 in `.decode-samples/`) already emits a valid segment." That is an
   OVER-CLAIM.** `engine_v4.py` (and engine_final/engine_v3) are A-counter
   **scorers** — they compute and score the A allocation against ground truth and
   `print` a number; they do NOT serialize an uploadable segment (no `log.txt`,
   no `to_wire`, no events-string assembly). And there is **no fights-segment event
   encoder in Rust at all.** See "WHAT IS NOT BUILT" below and memory note
   `uploader-encoder-not-built.md`. The format is well-DECODED (in Python research
   scripts); the segment ASSEMBLY was never built. This is the real remaining work.

2. **Auth = embedded webview showing ESO Logs' REAL login page.** No password
   form in Kalpa. Why not other options:
   - External browser CANNOT hand Kalpa the cookie: `laravel_session` is HttpOnly +
     esologs.com-scoped, in a separate process. Browser/OS isolation blocks it
     (the very thing that makes it feel safe).
   - esotk.com CANNOT broker it: esotk (the eso-toolkit repo, GitHub Pages static
     SPA, NO backend) only ever sees the OAuth `code`, never the browser cookie;
     and cross-origin + HttpOnly is unreadable even by esologs' own JS.
   - Embedded webview WORKS because Kalpa owns that webview's cookie jar → it can
     read the cookie after the user logs in on esologs.com's actual page. Standard
     pattern (Spotify/Discord/Steam).

3. **Operator permission** for the native upload exists (ESO Logs dev okayed RE).
   Still ship behind a one-time opt-in ToS disclosure; keep handoff as fallback.

## DONE this session (committed on `feat/log-uploader`, all tests green)

- **commit b69275b** + **e6fb478** (Cargo.lock):
  - `session.rs`: `StoredSessionProvider` — serves the upload-session cookie,
    persists it encrypted via `token_store`, `invalidate()` clears on 401/419.
  - `token_store.rs`: generic fail-closed chunked blob helper +
    `save/load/clear_upload_session` (independent of OAuth tokens).
  - `client.rs::send`: FULL wire-send — JSON create-report body, multipart
    add-segment (`parameters`+`logfile`) + set-master-table (`segmentId`+
    `isRealTime`+`logfile`), cookie header, single re-auth-retry on 401/419,
    early-cancel short-circuit. `NativeUpload::new` now takes `&UploadOptions`.
  - `format.rs`: `CLIENT_VERSION = "8.20.113"` constant.
  - `Cargo.toml`: reqwest `multipart` feature; `[profile.dev] debug =
    "line-tables-only"` (fixes MSVC LNK1318 PDB-limit + cuts target/ disk).
- 261 lib tests pass; clippy + fmt clean.
- Native upload STILL GATED: `format::FORMAT_VERSION_CONFIRMED = false` and
  `coverage::PROVEN_LINE_TYPES` empty → zero behavior change for users.

## Adversarial review hardening (6 rounds, all fixed — commits a29bac6 → 9ef92c9)

Ran `/codex:adversarial-review` in a loop until it converged. 11 real issues
found and fixed across 6 rounds (all green: 266 lib tests, clippy/fmt, tsc):
- create-report body rebuilt with serde_json (was invalid JSON for any non-null
  description); dropped the hand-rolled escaper.
- add-segment: malformed/missing nextSegmentId now a hard error (was a silent
  end-of-upload that finalized incomplete reports); a terminal `0` before the
  last local segment is rejected (no silent truncation).
- token_store chunked write hardened to truly fail-closed (verify the count
  sentinel is gone before writing chunks; hard-fail on commit-write failure).
- credential persistence failures surfaced end-to-end: save_tokens/
  save_upload_session return a committed-bool; save_auth_tokens is #[must_use] +
  logs; AuthUser gains `sessionPersisted`; UI shows a toast (warnIfSessionNotPersisted)
  at auth_get_user + both auth_login sites.
- token migration deletes plaintext only after the chunked read-back EQUALS the
  source (AuthUser/AuthTokens gained PartialEq) — no stale-set false-verify.
- upload_finished rejects empty input and best-effort terminate_reports on ANY
  post-create error (extracted push_segments_and_terminate).

Round-7 verdict findings are NOT code bugs — they are the two items below
(native dispatch is intentionally gated; the logged-out-uploader → Settings sign-in
gap is a real UX item). The review loop stopped here: no remaining bugs in the
changed code.

Known follow-up test gaps (need a mock transport / HTTP layer, none exists yet):
forcing set_master_table/add_segment failures to assert terminate; the early
`nextSegmentId:0` multi-segment case; the credential-write-failure store path.

## DONE 2026-06-18 (the auth/login/opt-in/UI seams — committed, all green)

These were the "plumbing around the upload." All build clean (273 lib tests,
clippy -D warnings, fmt, npm run check) and are GATED OFF (no behavior change):

- **Seam #1 — Webview login.** `native/login.rs`: `run_login`/`poll_for_session`
  open a `WebviewWindow` at `esologs.com/login`, and — only once the webview has
  navigated to an **authenticated view** (off `/login,/register,/password,/oauth`
  on an esologs host) — read `laravel_session` via `WebviewWindow::cookies_for_url`
  (Tauri 2.11.2, sync, returns HttpOnly cookies, off a `spawn_blocking` to avoid
  the Windows WebView2 deadlock) and `StoredSessionProvider::store`. The auth-view
  gate is the guard against capturing Laravel's anonymous guest session (the
  cookie is set on `/login` load — presence ≠ logged in). Commands:
  `uploader_login_esologs` (async) / `uploader_has_session` / `uploader_logout_esologs`.
  `StoredSessionProvider` is now **managed Tauri state** (`lib.rs`, single shared
  instance the login writes + the upload reads). The global `on_window_event`
  close handler is now **label-discriminated** (`window.label() == "main"`) so the
  login window closes instead of hide-to-tray. **Needs a live login to fully test.**
- **Seam #4b — Uploader inline sign-in.** `uploader-workspace.tsx` `LoggedOut` now
  does inline `auth_login` (was a dead-end "Open Settings"); `onAuthChange` threaded
  through `app-dialogs.tsx`.
- **Seam #3 — Opt-in + ToS disclosure.** `settings.tsx` `nativeUploadOptIn` toggle
  + `NativeUploadDisclosure` dialog (default OFF, honest "unofficial method"
  copy, accept-to-enable). The frontend reads it per-upload and passes `nativeOptIn`
  to `uploader_upload_log`, which now takes `native_opt_in: Option<bool>` and drives
  `assess_native_routing` (was hardcoded false). Still gated by
  `FORMAT_VERSION_CONFIRMED=false`, so opted-in users still route to the official
  uploader — only the logged routing reason changes.

## ⚠️ 2026-06-18 — LIVE ROUND-TRIP RESULT: accepted but DOES NOT RENDER

The owner ran the round-trip. The native upload was **server-ACCEPTED** (report
`jAHXkRdzpGwxVQ1t` created) **but the report does NOT render** — esologs shows an
infinite loading screen and it never appears in the user's report list. So
**server-accept ≠ parseable**: the segment is structurally well-formed (zero
malformed *lines*) but the event *stream* is wrong, so the parser can't build a
report. The gate was reverted CLOSED (`FORMAT_VERSION_CONFIRMED=false`, empty
`PROVEN_LINE_TYPES`) — native is OFF again, no more broken reports.

### Root cause (from the diagnostic diff — see below)
A new diagnostic test `events::combat_fixture::diff_against_official_combat_segment`
(`#[ignore]`, run `cargo test -- --ignored --nocapture`) diffs OUR fights-segment
against the OFFICIAL captured segment for the SAME log (Archive-20260614T190354Z,
a Lair of Maarselok dungeon — NOT a trial). It found the encoder is materially
incomplete on a real log:

- **TOTAL EVENTS: ours 48121 vs official 49891** (−1770), and they are *different*
  events, not just a count.
- **Entire segment codes MISSING from our output** (we emit zero): code **6**
  (0/1050), **8** (0/93), **11** (0/187) — the UPDATED-effect family the encoder
  deliberately suppressed ("emit predicate underivable"); plus **9** (0/25),
  **14** (0/4), **19** (0/158), **22** (0/1), **27** (0/19), **28** (0/23), **38**
  (0/638) — codes the encoder does not model at all.
- **Wrong counts on modeled codes**: code 1 (4525/4665), 16 (2628/3120), 5
  (11969/11012), 3 (2442/2354), 4 (659/677) — over- AND under-producing.
- **Wrong base timestamp**: first event `1|41|...` vs official `0|41|...`
  (off-by-one base — `segment_ts`/offset bug; cascades since events are
  ts-ordered).

### What this means
The "structurally valid, zero malformed lines" gate was MISLEADING: every line is
well-formed, but the stream omits ~6–10 whole event codes (~2900+ events), has
wrong per-code counts, and a wrong ts base. The events encoder is **NOT done** —
the prior "BUILT" claim below means the skeleton + the easy codes, not a
report-correct encoder. Finishing it = real, multi-day work (implement codes
6/8/9/11/14/19/22/27/28/38, fix counts on 1/3/4/5/16, fix the ts base, then drive
the diff to ~zero deltas and confirm a RENDERING report — render, not just
accept). The diff test is the precise target/oracle.

### ENGINEER'S NOTE on the strategy pivot
The whole plan rested on "the server re-parses any structurally-valid segment, so
byte-exact isn't needed." The round-trip shows that's only half-true: the server
*accepts* a structurally-valid segment (mints a code) but the *renderer/parser*
needs the events to actually be there and correct. "Structurally valid line" was
too weak a bar; the real bar is "the event stream faithfully represents the fight,"
which is much closer to the byte-exact target we'd set aside.

### ⚙️ REPORT-CORRECTNESS PASS (2026-06-18, after the round-trip) — driven by the oracle
The diff oracle was used test-first to fix the encoder. Landed (commits
db65d1e → c57199b on `feat/log-uploader`):
- **TS BASE** anchored on the FIRST EMITTED EVENT (ts 0), not `BEGIN_LOG` — the
  cascade bug (`BEGIN_LOG@9 / ZONE@10` now → segTs 0, was 1). Hardened so a
  dropped line can't steal the anchor.
- **MASKS**: effect/cast/regen use OWN-SIDE masks (16/64/32 per unit); combat codes
  keep the proven earlier/later. (`ActorTable::side_mask`.)
- **UPDATED codes 6/8/11** implemented: emit only on a stack-count CHANGE (buff
  +→6, buff −→8, debuff→11); same-stack / orphan / already-active GAINED dropped.
- **DEATHS → code 19** (DIED/DIED_XP): combat prefix + S + T, no tail.

**Result: 13 of 25 codes now count-EXACT** vs the official segment (was 7):
2,6,8,10,11,12,15,19,26,44,52,53. A committed regression guard
(`per_code_counts_stay_within_known_bounds`) locks them + bounds the residuals.

**Deferred residuals** (owner chose high-confidence wins; underdetermined from one
capture): code 5 +260 (passive/aura wall), code 1 −140 (DAMAGE_SHIELDED 1-vs-2
split), code 16 −492 (status/QUEUED cast markers), rare codes 9/14/22/27/28/38
(~870 events, 1.7%), 41/51 +1 (post-fight zone-out needs fight-window bounding).

**→ NEXT: OWNER RE-TEST the live round-trip.** The structural rendering-blockers
are fixed. If the report RENDERS now, the long tail likely doesn't matter — flip
the gate. If not, the oracle's remaining deltas point to what else the parser
needs. Re-evaluate full-native vs. the official handoff only after this re-test.

---

## DONE 2026-06-18 — the EVENTS ENCODER (seam #2) is BUILT (skeleton + easy codes)

The fights-segment events encoder is implemented, structurally tested, and
committed on `feat/log-uploader`. **NOTE: "built" = the driver + the easier codes;
the round-trip above shows it is NOT yet report-correct on a real log.** The two
locks (`FORMAT_VERSION_CONFIRMED=false` + empty `PROVEN_LINE_TYPES`) keep it OFF,
so there is **zero behavior change**.

- **`native/zip_segment.rs`** — `zip_log_txt(text) -> Vec<u8>`: the `logfile`
  envelope (single `log.txt` entry, DEFLATE-9, deterministic — fixed ZIP-epoch
  mtime, no wall-clock read). `Segment::from_text` / `MasterTableBytes::from_text`
  build the wire bytes from rendered text.
- **`native/events.rs`** — `EventEmitter` walks the raw log in file order and emits
  a structurally-valid line per code, threading the actor table, championPoints,
  effect types and a **dense first-sight `A`** counter (keyed on the
  `(srcIdentity, abilityId, tgtIdentity)` triple). Codes covered: 41/51 (zone/map),
  5/7/10/12 (effect gained/faded), 15/16 (cast), 1/2/3/26 (damage/dot/heal/power),
  44 (player info), 4 (regen), 52/53 (combat boundaries). UPDATED (6/8/11) is
  suppressed (the official segment drops the vast majority; emit predicate
  underivable). `build_fights_segment(lines)` frames the full segment;
  `build_native_payload(lines)` returns the ZIP'd `(Segment, MasterTableBytes)` —
  the single seam a `NativeTransport` calls.
- **Two `encode.rs` byte-bugs the combat capture exposed, fixed**: zone difficulty
  (`NONE/NORMAL/VETERAN → 0/1/2`, was hardcoded 0) and map resource-path
  lowercasing. Plus `combat_noncode1_crit_flag` (heal/dot/power) and a public
  `master_index_of` (code 44). `encode_state_block` now rejects a non-numeric
  championPoints (fail-loud against field-index bugs).
- **Validation**: reproduces the golden sample segment byte-for-byte **except the
  optional `A` cast-ref** (deliberately omitted — not needed for validity), and
  assembles the full ~49k-event combat capture with **zero malformed lines** (test
  no-ops on a clean checkout since the fixture is gitignored).
- **Adversarial review** (`/codex`-style, two workflow rounds, 50 agents): round 1
  found + fixed a high-sev UNIT_CHANGED championPoints off-by-two and a code-44
  coverage gap; round 2 found zero bugs. A prefix-only-tail issue (code-1 status
  results) was caught and fixed (drop the event rather than emit a malformed line).

### Coverage posture (deliberate)
`coverage::PROVEN_LINE_TYPES` stays **empty** — the ship gate is honestly closed
until the live round-trip proves server acceptance. `STRUCTURALLY_READY_LINE_TYPES`
+ `structural_readiness()` report what is built-and-tested-but-not-yet-confirmed
(17 of the 20 target types; the 3 `*_TRIAL*` markers await a trial-log capture).

### 5. Confirm + flip the gate (OWNER-run live round-trip — the only thing left)
The encoder produces a candidate segment; the empirical proof is a real upload.
Cannot be run autonomously (needs the signed-in webview session + a live POST).

**Procedure for the owner:**
1. `npm run tauri dev`, sign in via the in-app ESO Logs login (seam #1 stores the
   `laravel_session` cookie).
2. Pick a SHORT real combat log (a few minutes of fight).
3. Build + upload a native payload to a **test** report. The building blocks are
   `events::build_native_payload(&lines)` → `(Segment, MasterTableBytes)` and
   `client::NativeUpload::new(provider, &opts, cancel).upload_finished(&[seg],
   &[master], &progress)`. A `NativeTransport` that wires these into
   `select_transport` behind the opt-in flag is the remaining integration glue (the
   payload + lifecycle are both built + tested; this is plumbing, not research).
4. If ESO Logs **accepts and renders** the report:
   - set `format::FORMAT_VERSION_CONFIRMED = true`, and
   - copy the confirmed subset of `STRUCTURALLY_READY_LINE_TYPES` into
     `PROVEN_LINE_TYPES` (the gate is all-or-nothing per log).
   - `differential.rs`'s byte-diff becomes a QUALITY metric, not the ship gate.
5. If it is rejected, the server's error pinpoints the wrong field — the encoder is
   structured per-code (`events::emit_*`) so a fix is localized; re-test.

## Gotchas / environment

- **Machine disk runs near-full** (C: was at ~900M free this session). Builds may
  fail with `os error 112` / linker `LNK1318`. Reclaim: `rm -rf
  src-tauri/target/debug/incremental` (safe, regenerates). NOTE: the **main repo's**
  `src-tauri/target/debug/incremental` (a separate checkout, ~700M) is the bigger
  reclaim — clearing both regenerable incremental caches freed ~750M this session.
  `cargo sweep --time 30` found nothing stale (all artifacts recent).
- `.gitattributes` pins `testdata/**` to LF — CRLF breaks byte-exact tests.
- Never edit `package-lock.json` (it shows modified from before this session —
  not ours). `Cargo.lock` IS ours to commit.
- Decode artifacts (engine_v4.py, datasets.json, BUILD_CONCLUSION.md, the 4 parsed
  segment captures) live in gitignored `.decode-samples/` — the byte-exact decode
  record, only needed if revisiting A (which is off the critical path now).

## Key files
- `src-tauri/src/uploader/native/{session,client,format,serialize,convert,encode,
  events,zip_segment,coverage,differential,login}.rs`
- `src-tauri/src/uploader/transport.rs` (`assess_native_routing` — the routing seam)
- `src-tauri/src/token_store.rs`
- Reference (facts only): sheumais/logs `cli/src/esologs_{convert,format}.rs`.
