# Native uploader — what's left

The native ESO Logs uploader is **built end-to-end** and gated off. The auth/login,
opt-in, UI, and the fights-segment **events encoder** are all implemented, tested,
and committed on `feat/log-uploader`. See `native-uploader-next-steps.md` for the
full state.

**The only thing left is an owner-run live round-trip** to prove the server accepts
a Kalpa-produced segment, then flipping the gate. It cannot be done autonomously
(needs a signed-in session + a live POST). The exact procedure is in
`native-uploader-next-steps.md` → "5. Confirm + flip the gate". In short:

1. `npm run tauri dev`, sign in via the in-app ESO Logs login (the uploader's
   logged-out state now has a direct sign-in; the upload-session login command is
   `uploader_login_esologs`).
2. Enable "Upload logs directly" in Settings (accept the disclosure).
3. Temporarily flip `format::FORMAT_VERSION_CONFIRMED = true` and add the
   structurally-ready types to `coverage::PROVEN_LINE_TYPES` so routing chooses
   native, then upload a SHORT real combat log to a **test** report. The native path
   is already wired (`transport::run_native_upload` → `events::build_native_payload`
   → `client::NativeUpload::upload_finished`).
4. If ESO Logs accepts + renders it → keep the gate flip (commit it) and treat the
   `differential.rs` byte-diff as a quality metric, not a ship gate. If not, revert
   the flip (the gate stays closed = zero corruption) and debug the segment against
   the captured golden pair.

---

## If resuming the encoder (only if the round-trip reveals gaps)

The events encoder lives in `src-tauri/src/uploader/native/events.rs` (+
`zip_segment.rs`). Read `uploader-encoder-built.md` and `uploader-native-format-facts.md`
first. Byte-exact `A` is **not** required (the server re-parses; A only needs to be
internally consistent). The Python decode prototypes in `.decode-samples/`
(`engine_v4.py` etc.) are the spec for routing/mint logic — they are OUR research
scripts (fine to port); never read/port the reference project's conversion code.
