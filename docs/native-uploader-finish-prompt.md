# New-session prompt — finish the native ESO Logs uploader

Copy the block below as the opening message of a fresh **ultracode** session in the
`log-uploader` worktree.

---

```
ultracode

Finish the native ESO Logs uploader in Kalpa (Tauri/Rust + React), branch
feat/log-uploader = PR #157, worktree .claude/worktrees/log-uploader. This is a
deep, in-progress effort — DO NOT restart or re-decode. FIRST read these IN FULL:
  - docs/native-uploader-next-steps.md  (the authoritative handoff: strategic
    facts, what's built, the remaining seams in order, gotchas)
  - memory note uploader-native-format-facts.md  (protocol + decode state)
  - memory note uploader-native-auth-probe-result.md  (auth context)

GROUND TRUTH (already decided/verified — do not relitigate):
- GOAL = fully NATIVE upload: Kalpa POSTs directly to esologs.com/desktop-client/*;
  NEVER spawn the official GUI uploader. The "handoff/fallback" transport is a
  temporary scaffold, not the end state.
- Byte-exact subordinal-A is NOT required. The working reference uploader uploads a
  valid-but-different segment and the server re-parses + accepts it. Our encoder
  (engine_v4 in gitignored .decode-samples/) already emits a valid segment. Do NOT
  grind byte-exact A — it is off the critical path.
- AUTH = embedded webview showing esologs.com's REAL login page (no password form in
  Kalpa; Kalpa reads the laravel_session cookie from its OWN webview's cookie jar —
  works because Kalpa owns that webview; an external browser / esotk proxy CANNOT,
  blocked by HttpOnly cross-origin/process isolation). This is the chosen approach.
- CLEAN-ROOM: build from protocol FACTS only (endpoints, multipart field names,
  version constants). NEVER read/port/name the reference project's conversion code.

ALREADY DONE + pushed (commits b69275b → 83fbf48, all CI-green, 266 lib tests):
- session.rs StoredSessionProvider (cookie + encrypted persist via token_store,
  invalidate on 401, store() returns durability bool).
- client.rs full wire-send (create-report JSON via serde, multipart add-segment +
  set-master-table, cookie header, 401/419 re-auth-retry, empty-input reject,
  best-effort terminate_report on ANY post-create error, no silent truncation).
- token_store generic fail-closed chunked storage + upload_session API.
- format.rs CLIENT_VERSION; reqwest multipart feature; AuthUser.sessionPersisted
  surfaced to the UI (warnIfSessionNotPersisted toast).
- Hardened through 6 adversarial-review rounds. Native path is still GATED
  (FORMAT_VERSION_CONFIRMED=false, native_opt_in hardcoded false) → zero behavior
  change today, no corruption risk.

YOUR TASK — complete the remaining seams IN ORDER (see the handoff doc §3-6 + 4b):
1. WEBVIEW LOGIN COMMAND: a Tauri command that opens a WebviewWindow at esologs.com's
   login, and after login reads laravel_session from THAT webview's cookie jar, then
   calls StoredSessionProvider::store(). VERIFY the exact Tauri v2/wry cookie API
   first (WebviewWindow::cookies()/cookies_for_url() — confirm the Cargo.toml tauri
   version supports it); do NOT build blind. Build to compiles-clean; this needs a
   real login to fully test (have me run it).
2. NATIVE TRANSPORT: a Transport impl wired into select_transport's Native arm that
   runs the converter (convert→encode→serialize) per fight → Segment +
   MasterTableBytes, ZIP-deflates each segment (single log.txt entry, zip crate
   DEFLATE-9), and calls NativeUpload::upload_finished. Behind opt-in; keep the
   handoff fallback. Add a test proving an eligible opted-in log bypasses the
   official uploader.
3. OPT-IN + DISCLOSURE: drive native_opt_in from a persisted Settings toggle; add a
   one-time honest ToS-risk disclosure (default OFF). (Operator okayed RE; still
   disclose to users.)
4b. LOGGED-OUT UPLOADER SIGN-IN: the uploader's logged-out state currently points to
   Settings, which has no auth_login control. Add a direct sign-in action (call
   onAuthChange + warnIfSessionNotPersisted on success). See uploader-workspace.tsx.
5. CONFIRM + FLIP THE GATE: with the above done, do ONE real upload of a short combat
   log to a test report. If the server accepts it and the report renders correctly →
   flip FORMAT_VERSION_CONFIRMED=true and change the coverage gate from
   "byte-exact-or-fallback" to "structurally-valid + server-accepts" (the byte-diff
   becomes a quality metric, not a ship gate).

CONSTRAINTS: keep CI green (cargo test + clippy -D warnings + fmt + npm run check +
cargo audit + npm audit --omit=dev). Stage files by name, conventional commits, no AI
attribution. Machine disk runs ~100% full — if builds fail with os error 112 /
LNK1318, run `rm -rf src-tauri/target/debug/incremental`. .gitattributes pins
testdata/** to LF. Never edit package-lock.json. Outward actions (running the live
login, the real upload round-trip, flipping the gate) need me present — build/test
autonomously, then hand me the one-line command to run.

Use workflows where the work fans out (e.g. verifying the wry cookie API across
versions, or a final adversarial review pass). Run /codex:adversarial-review in a
loop after each seam until clean.
```
