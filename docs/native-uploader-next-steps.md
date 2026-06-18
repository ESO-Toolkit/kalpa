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
   self-imposed safety gate, stricter than reality. Our encoder (engine_v4 in
   `.decode-samples/`) already emits a valid segment (A reproduced to 3/3803).
   → Do NOT keep grinding byte-exact A. It is off the critical path.

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

## NEXT (in order) — the remaining seams

### 3. Webview login command (the gating piece — needs the app running + a live login)
Build a Tauri command that opens a `WebviewWindow` pointed at esologs.com's login,
and after login reads `laravel_session` from THAT webview's cookie jar, then calls
`StoredSessionProvider::store(cookie_header)`.
- **Verify the exact Tauri v2 / wry cookie API first** (it's version-specific):
  `WebviewWindow::cookies()` / `cookies_for_url()` exist in recent Tauri 2.x —
  confirm the version in `Cargo.toml` supports it; may need a tauri feature or a
  wry-level call. Do NOT build blind.
- Filter to the `laravel_session` (and any XSRF cookie the upload needs) for the
  esologs.com origin; assemble the `Cookie:` header string.
- This CANNOT be fully tested without running `npm run tauri dev` + a real login
  (outward-facing) — build to compiles-clean, then have the user run it once.

### 4. NativeTransport (wire into `select_transport`)
- New `Transport` impl in `transport.rs` that: builds `UploadOptions`, constructs
  `NativeUpload::new(&StoredSessionProvider, &opts, cancel)`, runs the converter
  (`convert.rs` → `encode.rs` → `serialize.rs`) to produce `Segment` +
  `MasterTableBytes` per fight, ZIP-deflates each segment (single `log.txt` entry;
  `zip` crate, DEFLATE-9), and calls `upload_finished`.
- Behind the opt-in flag; keep `GuiHandoffTransport` as fallback for not-opted-in
  / not-logged-in / errors. `assess_native_routing` already exists in transport.rs.
- The segment bytes passed to `client.rs` must be the ZIP blob (the wire-send sends
  them as-is in the `logfile` part) — confirm where zipping happens (serialize vs
  transport) and do it once.

### 5. Opt-in ToS disclosure UI (ship-blocking)
- One-time, honest in-app disclosure: native upload uses an unofficial method;
  default OFF; user opts in. Settings toggle drives the native_opt_in flag that
  `assess_native_routing` reads (currently hardcoded false).

### 6. Confirm + flip the gate (user-run round-trip)
- With seams 3–5 done, do ONE real upload of a short real combat log to a test
  report. If the server accepts it and the report renders correctly → flip
  `FORMAT_VERSION_CONFIRMED = true` and change the coverage gate from
  "byte-exact-or-fallback" to "structurally-valid + server-accepts" (the byte-diff
  in `differential.rs`/`coverage.rs` becomes a QUALITY metric, not a ship gate).

## Gotchas / environment

- **Machine disk is 100% full** (~1.5G free on C:). Builds may fail with `os error
  112` / linker `LNK1318`. Reclaim: `rm -rf src-tauri/target/debug/incremental`
  (safe, regenerates) or `cargo sweep --time 30`. Don't clean outside the worktree.
- `.gitattributes` pins `testdata/**` to LF — CRLF breaks byte-exact tests.
- Never edit `package-lock.json` (it shows modified from before this session —
  not ours). `Cargo.lock` IS ours to commit.
- Decode artifacts (engine_v4.py, datasets.json, BUILD_CONCLUSION.md, the 4 parsed
  segment captures) live in gitignored `.decode-samples/` — the byte-exact decode
  record, only needed if revisiting A (which is off the critical path now).

## Key files
- `src-tauri/src/uploader/native/{session,client,format,serialize,convert,encode,
  coverage,differential,transport}.rs`
- `src-tauri/src/token_store.rs`
- Reference (facts only): sheumais/logs `cli/src/esologs_{convert,format}.rs`.
