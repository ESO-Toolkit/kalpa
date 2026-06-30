# Native ESO Logs Uploader — Implementation Plan

> **Status:** Planning only. Build in a fresh session.
> **Decision owner:** project owner (informed, 2026-06-16).
> **Direction:** Replace the hand-off-to-official-uploader model with a native,
> in-process upload client so Kalpa owns the whole Start/Stop lifecycle — no
> second window, low memory, clean cancellation.

---

## 0. Why this exists (and the risk you're accepting)

Today Kalpa prepares a log and hands it to the **official ESO Logs Uploader**,
which performs the actual upload. That works but has three problems we proved are
**unfixable within the hand-off model** (all empirically tested 2026-06-16):

1. **It can't be controlled.** Kalpa can't stop the official uploader (no
   CLI/IPC/signal; the spawned PID is a self-exiting launcher).
2. **It's heavy.** The official app idles at **~1.4 GB across 11–13 processes**
   (measured on a real machine — the often-cited "550 MB" is optimistic), which
   undercuts "lightweight while gaming."
3. **It can't be hidden.** Every external launch trick fails: OS minimize hint,
   `start /MIN`, off-screen `config.json` bounds, and `CreateProcessW` +
   `SW_HIDE` are all ignored by Electron (Chromium detects-and-corrects a hidden
   launch; the app shows its window from its own JS). The only thing that works
   is `ShowWindow(SW_HIDE)` *after* a ~3–5 s flash, which we rejected (matches
   AV/Smart-App-Control heuristics for concealing a credentialed network binary,
   and removes the user's upload feedback).

The **only** way to get a clean, lightweight, single-window experience is to
**speak the ESO Logs upload protocol directly** instead of using the official
app. There is **no sanctioned path** to do this:

- The public v2 GraphQL API is read-only (no upload mutation).
- There is no web upload form, no headless uploader, no CLI-only build, no
  partner/upload API, no Overwolf-light surface. (All researched + verified.)
- The upload endpoints are the private `/desktop-client/*` REST API.

### The risk (must be surfaced to end users in-app)

Speaking the private protocol and presenting Kalpa as an upload client runs
against RPGLogs' API Terms of Service (notably: only access APIs as documented;
don't mask client identity; don't build a substantially-equivalent client for
third parties). **RPGLogs can suspend accounts — the user's and Kalpa users' —
without notice.** The owner accepts this risk based on a comparable widely-used
third-party tool that has not faced enforcement. **Because the risk lands on end
users, the feature MUST include an explicit, honest in-app disclosure and opt-in
before any native upload runs** (see §6).

### Hard constraints (non-negotiable)

- **Clean-room.** Implement everything from scratch. Do NOT copy code, comments,
  structure, or naming from any existing third-party implementation. Protocol
  facts (endpoint URLs, multipart shape, format-version constants) are facts
  about ESO Logs and may be used; implementation is ours.
- **Never name or allude to the reference project** anywhere — code, comments,
  commits, PRs, UI, docs.
- **No evasion.** No user-agent rotation to dodge detection, no hiding that the
  client is Kalpa beyond the unavoidable `clientVersion` the protocol requires,
  no detection-dodging. If it can't be done plainly, it doesn't ship.
- **Build must stay green** (`npm run check`, `cargo clippy`, `cargo fmt`, CI).

---

## 1. What already exists and is reused as-is

Kalpa's uploader backend already does most of the *local* work. Reuse, don't
rebuild:

| Module | Role | Reuse |
|---|---|---|
| `src-tauri/src/uploader/scanner.rs` | Streaming session/fight detection as byte ranges; handles multi-GB, invalid UTF-8, partial lines | ✅ as-is |
| `src-tauri/src/uploader/splitter.rs` | Per-session extraction, stale-offset revalidation | ✅ as-is (segment boundaries) |
| `src-tauri/src/uploader/watcher.rs` | Live tail of `Encounter.log` by byte offset; fight-boundary events | ✅ as-is (drives both UI timeline AND native live upload) |
| `src-tauri/src/uploader/history.rs` | Persistent upload history + stale reconcile | ✅ extend |
| `src-tauri/src/uploader/discovery.rs` | Find Logs dir, enumerate logs | ✅ as-is |
| `src-tauri/src/uploader/types.rs` | IPC types (`UploadOptions`, `Visibility`, etc.) | ✅ extend |

**Already shipped & correct this branch:** the `liveLog --directory-path` fix in
`transport.rs` (operation-aware command building) + its tests. The native path
will eventually supersede `transport.rs`, but keep it as the fallback (see §5).

---

## 2. The new work: the native protocol client

The genuinely new, substantial piece. Two layers.

### 2a. The log → ESO Logs format converter (the bulk of the effort)

The scanner produces *byte ranges*; the protocol needs ESO Logs' **internal
structured upload format** — a per-segment serialization plus a "master table"
of units/abilities/effects, NOT the raw log. **This is a clean-room
reimplementation** of that format. Plan it as its own module:
`src-tauri/src/uploader/native/format.rs` (+ `convert.rs`).

- Input: a session's raw log lines (from the scanner's byte ranges).
- Output: the compact segment + master-table representation the
  `/desktop-client/` endpoints accept, keyed to the current **format/parser
  version** (a version constant — must be kept current; uploads break if the
  server's expected version drifts).
- Build with **golden-file tests** against real (sanitized) `Encounter.log`
  samples: parse → serialize → assert byte-stable output.
- This is the riskiest/most labor-intensive part. Estimate the majority of the
  effort here. Consider scaffolding the test harness first.

### 2b. The protocol client

`src-tauri/src/uploader/native/client.rs`. Uses `reqwest` (already a dep — but
**add the `multipart` feature**; current features are `blocking, json, query,
form`).

Report lifecycle (all POST to `https://www.esologs.com/desktop-client/`):

1. `create-report` → returns a report code.
2. loop `add-report-segment/{code}` (multipart ZIP of a segment).
3. `set-report-master-table/{code}` (multipart ZIP).
4. `terminate-report/{code}` (close).

- Cancellable via an `Arc<AtomicBool>` checked between segments — reuse the exact
  cancellation pattern already in `commands.rs` (`upload_cancel`/`Starting`
  slot). **Stop becomes a clean in-process flag flip + final `terminate-report`
  — the original "Stop opens esologs" bug is gone because nothing foreign is
  spawned.**
- Live mode: the existing `watcher.rs` tail drives it — on each completed
  fight/segment, convert + POST. No 5 s busy-poll needed beyond the watcher's
  existing cadence.

---

## 3. Auth — reuse the existing ESO Logs sign-in (verify first)

**Open question that gates the client, resolve in the build session first.**

Kalpa already has ESO Logs auth (`src-tauri/src/auth.rs`): OAuth2/PKCE via the
esotk.com proxy → a **Bearer access_token** for the GraphQL API (client_id
`9fd28ffc-300a-44ce-8a0e-6167db47a7e1`), stored in `token_store.rs` (Windows
Credential Manager, chunked).

The `/desktop-client/*` upload endpoints are a **different auth domain** — the
protocol authenticates via a **session cookie** from a credential login, not the
API Bearer token. **It is UNTESTED whether the existing Bearer token is accepted
by `create-report`.** Three outcomes:

1. **Bearer accepted** → reuse existing sign-in, zero new credential UX. Best.
2. **Token→session bridge** exists (a valid Bearer can mint an upload session) →
   reuse sign-in + a small bridge step. Investigate this before any password UI.
3. **Cookie-from-password only** → would need a credential login. The owner
   prefers reusing the existing sign-in; if only #3 works, **stop and decide**
   (a new email/password form is a new security surface).

**First build step:** a throwaway probe (the reverted `native_probe.rs` from
commit 40b3d50 is a starting template — it POSTed `create-report` with the
existing Bearer and classified the response). Resolve which outcome we're in
before building the converter, since it shapes the client.

> A revertable spike for this was built and reverted on this branch
> (commits 40b3d50 → 75bf9e4); recover it from git history if useful.

---

## 4. Frontend / UX changes

`src/components/uploader/uploader-workspace.tsx` already has the live timeline,
status pill, history, and Stop wiring. With native upload:

- **Stop becomes real.** "Stop tracking" cancels the in-process upload (flag +
  terminate-report) — update the honest-but-now-inaccurate copy that says "the
  uploader keeps going in its own window." It no longer does.
- **No more handoff toasts / HandedOff status** for the native path.
- **Manual upload** shows real progress (segments POSTed) instead of "watch the
  uploader window."
- Keep the in-app live fight timeline as the front-of-house experience.

---

## 5. Migration / fallback strategy

- Put the native client behind the existing transport seam (`select_transport`
  in `transport.rs`). Add a `NativeTransport` implementing `LogUploadTransport`.
- Keep `GuiHandoffTransport`/`CliTransport` as a **fallback** (e.g. if auth
  fails, or as a user setting) so a protocol/version break doesn't brick uploads
  entirely — the official app still works.
- Gate the native path behind the §6 opt-in.

---

## 6. Required user-facing disclosure (ship-blocking)

Because the risk lands on end users, before the first native upload show a
one-time, explicit opt-in dialog, honestly worded, e.g.:

> **Upload directly from Kalpa (beta)**
> Kalpa can upload your logs to ESO Logs itself — faster, lighter, and without
> opening a separate app. This uses ESO Logs' upload service directly rather than
> the official uploader. It works, but it isn't an officially supported method,
> and in rare cases ESO Logs could restrict accounts that use unofficial
> uploaders. You can switch back to the official uploader anytime in Settings.
> [ Use Kalpa direct upload ] [ Use the official uploader instead ]

- Default the toggle in Settings; let users choose the official-uploader
  fallback.
- Do not overstate safety; do not hide that it's unofficial.

---

## 7. Suggested build order (fresh session)

1. **Resolve auth (§3)** — probe whether the existing Bearer works on
   `create-report`. Decide reuse vs bridge vs (escalate) password. *Gates
   everything.*
2. **Converter + golden tests (§2a)** — the bulk. Clean-room. Pin format version.
3. **Protocol client (§2b)** — create/add-segment/master-table/terminate, with
   `multipart` reqwest feature + `AtomicBool` cancel.
4. **Wire manual upload** through `NativeTransport`; real progress + real cancel.
5. **Wire live** via the existing watcher; clean Stop.
6. **Disclosure + Settings toggle (§6)** + fallback to official (§5).
7. **Honest UX copy pass** (§4); `npm run check`, clippy, fmt; CI green.

---

## 8. Durable facts (so the build session doesn't re-derive them)

- reqwest is `0.13`, features `blocking, json, query, form` — **add `multipart`**.
- `AuthState` is managed in `lib.rs`; `get_valid_token()` returns a fresh Bearer.
- Uploader commands register in `lib.rs` invoke_handler (~L412–422); debug-only
  commands use `#[cfg(debug_assertions)]` (see `dev_scrub_saved_variable`).
- The official uploader's CLI flag table (for the fallback path) lives in its
  `resources/app.asar` var `Ofu`: `--operation-name` ∈
  {liveLog, uploadALog, splitALog}; `--directory-path` (live), `--file-path`
  (upload/split), `--guild`, `--report-visibility` (Public=0/Private=1/
  Unlisted=2), `--region`, `--include-entire-file-in-report`,
  `--enable-real-time-uploading`.
- Format/parser version is a server-coupled constant — uploads break silently if
  it drifts; make it easy to update and surface failures clearly.

See also the memory notes: native-protocol-direction, tos-no-sanctioned-upload,
lightweight-live-design, uploader-stop-research.
