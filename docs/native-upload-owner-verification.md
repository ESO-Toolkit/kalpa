# Native upload — owner verification probes

These are the **owner-run empirical checks** that gate moving the native
(`/desktop-client/*`) uploader from opt-in toward default. They cannot be settled
offline — each needs a real ESO Logs account, a real combat log, and (for two of
them) a wire capture. Run them in one sitting; they share a mitmproxy session.

Status this captures (2026-06-23):

- **clientVersion** is now `9.3.93` (Archon App), committed. ✅ no action.
- **parserVersion** is hard-coded `FORMAT_VERSION = 11` and was last confirmed
  against the *old* uploader (2026-06-17). Re-confirm it against the **Archon App**
  (Probe A) — low risk, but the new `desktop-client/list-versions` endpoint implies
  the server version-gates.
- **Native render gate** (`FORMAT_VERSION_CONFIRMED`, committed `false`): a
  *dungeon* log was confirmed to render (2026-06-19). The open question is whether a
  **real non-IA raid/combat log** renders end-to-end (Probe B). This is what gates
  flipping `FORMAT_VERSION_CONFIRMED` to `true` in a commit and making native the
  default.
- **Infinite Archive (IA)**: IA logs carry `ENDLESS_DUNGEON_*` event types not in
  `PROVEN_LINE_TYPES`, so they correctly hand off today. Probe C captures the
  **golden IA segment** needed to build + verify IA support byte-exactly.

> ⚠️ **Proxy OFF during live play.** Only run mitmproxy while *uploading a
> finished log*. Do not route the live game client through the proxy mid-combat —
> it can disrupt the connection and pollute the capture. Every probe below uploads
> an already-finished log.

---

## One-time setup (≈5 min)

1. `pip install mitmproxy` (if not already).
2. The capture addon already exists at `C:\Users\brayd\Desktop\eso-capture.py`. It
   watches `*.esologs.com/desktop-client/*`, unzips each `logfile` field, and writes
   decoded text to `.\eso-segments\` next to where you run it. It prints
   `clientVersion`/`parserVersion` from every `create-report`.
3. Start it: `cd %USERPROFILE%\Desktop && mitmweb -s eso-capture.py`
   (web UI on `:8081`, proxy on `:8080`).
4. Trust the mitmproxy CA so TLS interception works: browse to
   `http://mitm.it` through the proxy and install the Windows cert, **or** set the
   system proxy to `127.0.0.1:8080` and install via `mitm.it`. (Electron apps like
   the Archon App honor the Windows system proxy + cert store.)
5. **Remember to undo step 4 afterwards** — remove the system proxy and the CA
   cert when done. (The MITM CA is a security risk if left installed.)

Pick a **short** real log for Probes A/B (a single dungeon or one trial boss is
ideal — fast to upload, fast to eyeball). Copy it out of
`Documents\Elder Scrolls Online\live\Logs\esologsarchive\` to a non-Controlled-
Folder-Access dir (e.g. `C:\eso-verify\`) first — Documents is CFA-protected (see
the CFA note in the repo history) and a clean working dir avoids surprises.

---

## Probe A — confirm parserVersion (objective #5, low risk)

**Goal:** confirm the Archon App still sends `parserVersion: 11` (the value Kalpa
hard-codes as `FORMAT_VERSION`).

1. With mitmproxy + cert active, open the **Archon App** and upload the short log
   (any visibility; a throwaway/Unlisted report is fine).
2. Watch the mitmproxy console or read
   `eso-segments\*_create-report_request.json`. The addon prints:
   `clientVersion=… parserVersion=…  <-- VERSION FACTS`.

**Pass:** `parserVersion == 11`. → no code change; Kalpa already matches.
**Fail (≠ 11):** set `FORMAT_VERSION` in `src-tauri/src/uploader/native/format.rs`
to the captured value and re-prove the goldens (the differential tests pin the old
value). Note the new value here.

---

## Probe B — confirm native renders a real combat log (objective #3, THE GATE)

**Goal:** prove Kalpa's *own* native upload of a real non-IA combat log produces a
report that **renders** on esologs.com — not merely returns HTTP 200. (Server
acceptance ≠ rendering; that exact trap closed the gate once before.)

1. In Kalpa **Settings**, enable the direct-upload opt-in (`nativeUploadOptIn`).
2. Sign in to ESO Logs *uploads* inside Kalpa (the in-app ESO Logs login that
   captures the `wcl_session` cookie — distinct from the Pack Hub / profile login).
3. Build/run a Kalpa dev build whose `FORMAT_VERSION_CONFIRMED` is `true` (your
   local working-tree flip). The committed value stays `false`.
4. In the uploader, pick the short real log (all-proven types — a normal
   dungeon/trial log; **not** IA). The button should read **"Upload directly"** and
   the route chip **"Direct from Kalpa"**. If it instead says "Open the ESO Logs
   uploader," native declined — check sign-in + that every line type is in
   `PROVEN_LINE_TYPES` (the eprintln `native routing → official: …` says why).
5. Click **Upload directly**.

**Pass bar (all of):**
- The report appears in *your* esologs.com report list within ~1–2 min.
- It **renders**: fights are named, damage/healing tables populate, the player
  list is correct — no infinite "loading" spinner, no empty report.
- **Fidelity check (do this, don't skip):** upload the *same* log through the
  **Archon App** to a second report and compare headline numbers (total damage,
  per-fight durations, top players). They should match. A report that renders but
  shows wrong numbers is still a fail.

**If pass:** flip `FORMAT_VERSION_CONFIRMED = true` in a commit (it is the
belt-and-suspenders partner of the already-populated `PROVEN_LINE_TYPES`), and
native becomes the default for all-proven logs. Then proceed to branch integration
(objective #6).

**If fail:** keep `FORMAT_VERSION_CONFIRMED = false` (committed). Capture Kalpa's
segment via the proxy and diff it against the Archon golden with
`native/differential.rs` to find the first divergence. Do **not** ship native
default.

### Optional, stronger: byte-diff Kalpa vs Archon for the same log

With the proxy on, upload the same short log through **both** the Archon App and
Kalpa. The addon writes `*_fights-segment_log.txt` + `*_master-table_log.txt` for
each. Diff the pair — byte-identity is the strongest possible confirmation; any
divergence is a precise bug coordinate even if the report still renders.

---

## Probe C — capture the IA golden (objective #4)

**Goal:** obtain the official Archon App's segment for a **real Infinite Archive
run**, so we can build the `ENDLESS_DUNGEON_*` encoder and verify it byte-exactly
before adding those types to `PROVEN_LINE_TYPES`.

1. Play one IA run with `/encounterlog on` (proxy **OFF** during play). End the run;
   `/encounterlog off`.
2. Turn the proxy **ON** (system proxy + cert), then upload that IA log through the
   **Archon App**.
3. The addon writes the IA `create-report` body, `master-table`, and
   `fights-segment` to `eso-segments\`. **Keep these** — copy them somewhere named
   (e.g. `eso-segments\ia-golden\`). Also keep the matching **raw** IA log.

**What it gives us:** a matched raw↔segment pair for IA. With it we can answer the
only question research can't (see `docs/` IA scoping brief / the
`ia-event-grammar-research` workflow output): **does each `ENDLESS_DUNGEON_*` line
emit a segment event (like `END_TRIAL` → code 55) or is it a no-op state marker
(like `BEGIN_TRIAL`/`TRIAL_INIT`)?**

**Then (code work, gated on the golden):**
1. Add the `ENDLESS_DUNGEON_*` types to `coverage::TARGET_LINE_TYPES` (the
   vocabulary grows past 20 — update the `target_vocabulary_is_the_closed_set_of_20`
   test deliberately).
2. Implement their handling in `native/events.rs::feed` (likely no-op `=> None`
   arms, mirroring the trial markers — but **verified against the golden**, not
   assumed).
3. Add a `differential.rs` golden test proving our IA segment matches the captured
   one byte-for-byte.
4. **Only then** add the verified `ENDLESS_DUNGEON_*` types to
   `PROVEN_LINE_TYPES` (and `STRUCTURALLY_READY_LINE_TYPES`). Until that test is
   green, IA stays on the official handoff — which is correct, not a regression.

---

## Cleanup

- Remove the system proxy setting and uninstall the mitmproxy CA cert.
- `eso-segments\` and `eso-capture.py` are throwaway except the **IA golden** you
  deliberately saved for the encoder work.
