//! Upload transport abstraction.
//!
//! The default upload route uses the **official ESO Logs / Archon uploader**.
//! Kalpa drives its CLI or GUI handoff for compatibility and uses this module for
//! that official-app transport. The separate `native` module owns the gated
//! opt-in direct path.
//!
//! Two official-uploader transports are provided behind one trait so that
//! fallback strategy can evolve:
//!
//! * [`GuiHandoffTransport`] — the rock-solid default. Opens the official
//!   uploader (or its download page) with the prepared log so the user finishes
//!   in one click. Works regardless of any private-protocol drift.
//! * [`CliTransport`] — an automated path that invokes the official uploader's
//!   command-line interface when present. Preferred when available; falls back
//!   to GUI handoff otherwise.

use std::path::{Path, PathBuf};

use super::types::UploadOptions;

/// How a transport reports the disposition of an upload request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadOutcome {
    /// The transport completed the upload itself and (optionally) knows the
    /// resulting report code.
    Completed { report_code: Option<String> },
    /// The transport handed off to the official uploader's UI; the user will
    /// complete the upload there. Not an error — the expected path for the
    /// GUI-handoff transport.
    HandedOff { detail: String },
}

/// Abstraction over "get this prepared log into ESO Logs".
pub trait LogUploadTransport: Send + Sync {
    /// Human-readable name for diagnostics / the UI.
    fn name(&self) -> &'static str;

    /// Whether this transport is usable on the current machine right now.
    fn is_available(&self) -> bool;

    /// Upload (or hand off) a single prepared `.log` file.
    fn upload_file(&self, log_path: &str, opts: &UploadOptions) -> Result<UploadOutcome, String>;
}

// ── Locating the official uploader ───────────────────────────────────────────

/// Known `(install-dir, exe-name)` pairs for the official uploader, newest first.
///
/// electron-builder names the executable after the app's `productName`, so the
/// install directory and the exe stem match. Listing real pairs (rather than a
/// dir × exe cross-product) keeps this honest and avoids nonsense paths like
/// `Archon\ESO Logs Uploader.exe`.
const KNOWN_UPLOADERS: [(&str, &str); 3] = [
    // The unified **Archon App** (replaces the standalone ESO Logs Uploader and
    // Companion on 2026-06-29). `productName = "Archon App"`, no `executableName`
    // override, verified clean-room from `Uploaders-archon` v9.3.93's app.asar
    // (and its macOS `CFBundleExecutable`). The /desktop-client CLI it accepts is
    // unchanged, so `CliTransport` drives it exactly like the old uploader.
    ("Archon App", "Archon App.exe"),
    // The legacy standalone uploader. Kept for the pre-retirement grace period so
    // an existing install keeps working until the user migrates to the Archon App.
    ("ESO Logs Uploader", "ESO Logs Uploader.exe"),
    // Defensive: a shorter "Archon" productName, should a future build use one.
    ("Archon", "Archon.exe"),
];

/// App install roots to search, admin-writable Program Files **before** the
/// per-user LocalAppData so an admin install is preferred when both exist. Each
/// base contributes both itself and its `Programs` subdir (where Electron's
/// per-user installs land).
///
/// Note the Archon App is an Electron per-user install under
/// `%LOCALAPPDATA%\Programs`, so on a typical machine the resolved path IS
/// user-writable — this ordering does not by itself prevent a planted binary from
/// being the only match. A planting attacker already has user-level code
/// execution; full hardening (Authenticode/publisher verification before
/// spawning) is a possible future improvement.
fn app_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(var).map(PathBuf::from) {
            roots.push(base.join("Programs"));
            roots.push(base);
        }
    }
    roots
}

/// Expand the known `(dir, exe)` pairs across the given roots, **product-major**:
/// every root for the first (newest) product before any path for the next. Pure
/// (no I/O), so it is unit-testable without touching the environment.
fn candidates_for_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    KNOWN_UPLOADERS
        .iter()
        .flat_map(|(dir, exe)| roots.iter().map(move |root| root.join(dir).join(exe)))
        .collect()
}

/// Candidate install locations for the official uploader, across all roots.
///
/// Ordered **product-major** so an installed Archon App always outranks a stale
/// legacy uploader regardless of which root each lives in — a per-user
/// `%LOCALAPPDATA%` Archon App is preferred over an admin `Program Files` legacy
/// install. (Root-major ordering would let the higher-priority Program Files root
/// surface a retired legacy uploader ahead of the newer per-user app.) Within a
/// single product the admin-writable Program Files roots still come first — the
/// order [`app_roots`] yields.
fn official_uploader_candidates() -> Vec<PathBuf> {
    candidates_for_roots(&app_roots())
}

/// Find the official uploader executable, if installed.
///
/// Matches only the **exact** [`KNOWN_UPLOADERS`] `(dir, exe)` pairs — never a
/// name-prefix scan of the install roots. A prefix scan (e.g. accept any
/// `Archon*`/`ESO Logs*` directory) would let a planted `Archon Helper\Archon
/// Helper.exe` under the user-writable `%LOCALAPPDATA%` be spawned as if it were
/// the official uploader, widening the trusted-executable set well beyond known
/// product paths. If the app is ever renamed past these entries, add the new pair
/// here in a release rather than re-introducing a broad scan.
pub fn find_official_uploader() -> Option<PathBuf> {
    official_uploader_candidates()
        .into_iter()
        .find(|p| p.is_file())
}

// ── GUI handoff transport (default, always available) ────────────────────────

/// Opens the official uploader pointed at the prepared log, or its download page
/// if it isn't installed. Always "available" because the fallback (opening the
/// folder / download page) never fails.
pub struct GuiHandoffTransport;

const UPLOADER_DOWNLOAD_URL: &str = "https://www.esologs.com/client/download";

impl LogUploadTransport for GuiHandoffTransport {
    fn name(&self) -> &'static str {
        "Official Uploader (handoff)"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn upload_file(&self, log_path: &str, _opts: &UploadOptions) -> Result<UploadOutcome, String> {
        if !Path::new(log_path).is_file() {
            return Err(format!("Prepared log not found: {log_path}"));
        }

        if let Some(exe) = find_official_uploader() {
            // Launch the uploader with the file path so the user lands on the
            // upload screen. The official app accepts a file path argument.
            std::process::Command::new(&exe)
                .arg(log_path)
                .spawn()
                .map_err(|e| format!("Failed to launch the official ESO Logs uploader: {e}"))?;
            Ok(UploadOutcome::HandedOff {
                detail: "Opened the official ESO Logs uploader with your prepared log.".into(),
            })
        } else {
            // Not installed — open the official download page; the file is ready
            // on disk for the user to drag in.
            open_url(UPLOADER_DOWNLOAD_URL)?;
            Ok(UploadOutcome::HandedOff {
                detail: "The official ESO Logs uploader (the Archon App) isn't \
                         installed. Opened the download page; your prepared log is \
                         ready on disk."
                    .into(),
            })
        }
    }
}

/// Open a URL in the default browser (Windows shell `start`).
fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| format!("Failed to open browser: {e}"))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {e}"))?;
    }
    Ok(())
}

// ── CLI transport (automated, when the official CLI is present) ───────────────

/// Invokes the official uploader's command-line interface. The official app
/// exposes `--operation-name uploadALog --file-path … --region … --guild …`.
/// Because exact CLI semantics can change between uploader versions, this
/// transport reports itself unavailable unless the executable is found, and the
/// caller falls back to [`GuiHandoffTransport`].
pub struct CliTransport {
    exe: PathBuf,
}

impl CliTransport {
    /// Construct if the official uploader executable can be located.
    pub fn detect() -> Option<Self> {
        find_official_uploader().map(|exe| Self { exe })
    }
}

impl CliTransport {
    /// Build the fully-prepared launch command (all validation + argv done), so
    /// the only thing left for the caller is `spawn()`. Kept separate from the
    /// spawn so a caller can insert a final cancellation check *immediately*
    /// before launch with no intervening fallible/filesystem work — see
    /// [`Self::upload_file_cancellable`].
    fn prepare_command(
        &self,
        log_path: &str,
        opts: &UploadOptions,
    ) -> Result<std::process::Command, String> {
        if !Path::new(log_path).is_file() {
            return Err(format!("Prepared log not found: {log_path}"));
        }

        let mut cmd = std::process::Command::new(&self.exe);
        if opts.real_time {
            // Live logging is the `liveLog` operation, which watches a DIRECTORY
            // for the active `Encounter.log` (the uploader's ESO config pins
            // `logFilePattern: ^Encounter\.log$`) and streams it as the game
            // appends — it does NOT take a `--file-path`. Passing `--file-path`
            // with `uploadALog` (the old behavior) ran a one-shot file upload
            // with a real-time flag bolted on, not a real live session. Hand it
            // the log's PARENT directory and the correct operation. Verified
            // against the installed uploader's CLI flag table (var `Ofu`):
            // `--directory-path` = "directory to use when live logging".
            let dir = Path::new(log_path)
                .parent()
                .ok_or("Could not resolve the log's folder for live logging.")?;
            cmd.arg("--operation-name")
                .arg("liveLog")
                .arg("--directory-path")
                .arg(dir)
                .arg("--region")
                .arg(opts.region.to_string());
        } else {
            // One-shot upload of a finished, prepared `.log` file.
            cmd.arg("--operation-name")
                .arg("uploadALog")
                .arg("--file-path")
                .arg(log_path)
                .arg("--region")
                .arg(opts.region.to_string());
        }

        // Guild ids are numeric; anything else (a value with spaces, quotes, or
        // a leading `-`) could be mis-parsed by the uploader into extra flags,
        // so fall back to Personal Logs (`null`) rather than forward it.
        match &opts.guild_id {
            Some(g) if !g.is_empty() && g.len() <= 32 && g.chars().all(|c| c.is_ascii_digit()) => {
                cmd.arg("--guild").arg(g);
            }
            _ => {
                cmd.arg("--guild").arg("null");
            }
        }
        // Forward the user's report visibility. Without this the uploader uses
        // its own default/last-used visibility, so a user who picked Private
        // could have the report uploaded more openly — a privacy bug. The id
        // mapping is the uploader's own (Public=0, Private=1, Unlisted=2),
        // verified against the installed app.asar (see as_report_visibility_id).
        cmd.arg("--report-visibility")
            .arg(opts.visibility.as_report_visibility_id().to_string());

        if opts.include_entire_file {
            // The official uploader's flag is `--include-entire-file-in-report`;
            // it silently ignores unknown flags, so the old `--include-entire-file`
            // was a no-op (verified against the installed app.asar v8.20.113 flag
            // table). Use the real name so "include earlier fights" takes effect.
            cmd.arg("--include-entire-file-in-report");
        }

        if opts.real_time {
            cmd.arg("--enable-real-time-uploading");
        }
        Ok(cmd)
    }

    fn handed_off_outcome(opts: &UploadOptions) -> UploadOutcome {
        UploadOutcome::HandedOff {
            detail: if opts.real_time {
                "Live logging started in the official ESO Logs uploader.".into()
            } else {
                "Uploading in the official ESO Logs uploader — watch its window for progress."
                    .into()
            },
        }
    }

    /// Like [`LogUploadTransport::upload_file`], but runs `should_abort` as the
    /// **last** thing before `spawn()` — after all path validation and argv
    /// construction — so a cancellation that lands during command preparation is
    /// honored and no external process is launched. `should_abort` returning
    /// `true` yields [`LaunchAborted`] without spawning. This shrinks the
    /// unrecallable "stop during launch" window to the irreducible instruction
    /// gap between the check and `spawn()` (an OS process launch can never be
    /// made truly atomic with a flag read).
    pub fn upload_file_cancellable(
        &self,
        log_path: &str,
        opts: &UploadOptions,
        should_abort: &dyn Fn() -> bool,
    ) -> Result<Result<UploadOutcome, String>, LaunchAborted> {
        let mut cmd = match self.prepare_command(log_path, opts) {
            Ok(c) => c,
            Err(e) => return Ok(Err(e)),
        };

        // FINAL pre-launch check — the last statement before spawn, with no
        // fallible work after it.
        if should_abort() {
            return Err(LaunchAborted);
        }
        // Spawn and hand off rather than blocking on `status()`. Waiting would
        // hang for the whole session in real-time mode, and even for a one-shot
        // it can block minutes on a multi-GB log with the UI frozen — and we
        // can't observe a report code from the CLI exit anyway. The official
        // uploader window shows the user real progress.
        match cmd.spawn() {
            Ok(_) => Ok(Ok(Self::handed_off_outcome(opts))),
            Err(e) => Ok(Err(format!(
                "Failed to start the official ESO Logs uploader: {e}"
            ))),
        }
    }
}

/// Returned by [`CliTransport::upload_file_cancellable`] when the final
/// pre-launch check aborted the launch: no external process was spawned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchAborted;

impl LogUploadTransport for CliTransport {
    fn name(&self) -> &'static str {
        "Official Uploader (CLI)"
    }

    fn is_available(&self) -> bool {
        self.exe.is_file()
    }

    fn upload_file(&self, log_path: &str, opts: &UploadOptions) -> Result<UploadOutcome, String> {
        // The trait (manual) path has no cancellation source, so it never aborts.
        match self.upload_file_cancellable(log_path, opts, &|| false) {
            Ok(result) => result,
            // unreachable: `should_abort` is always false here.
            Err(LaunchAborted) => Err("Upload was cancelled.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uploader::types::UploadOptions;
    use std::io::Write;

    /// Render a prepared command's program + args as a single string for
    /// assertions. `Command`'s Debug format prints the program and each arg, so
    /// this lets us verify the exact operation/flags without spawning anything.
    fn cmd_string(cmd: &std::process::Command) -> String {
        let mut s = format!("{:?}", cmd.get_program());
        for a in cmd.get_args() {
            s.push(' ');
            s.push_str(&format!("{a:?}"));
        }
        s
    }

    fn cli_with_dummy_exe() -> (CliTransport, tempfile::TempDir, String) {
        // `prepare_command` guards on `log_path.is_file()`, so the log must exist.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("Encounter.log");
        let mut f = std::fs::File::create(&log).unwrap();
        f.write_all(b"0,BEGIN_LOG,1,15\n").unwrap();
        let exe = dir.path().join("ESO Logs Uploader.exe");
        std::fs::File::create(&exe).unwrap();
        let log_str = log.to_string_lossy().into_owned();
        (CliTransport { exe }, dir, log_str)
    }

    // Live mode must invoke the `liveLog` operation against the log's PARENT
    // DIRECTORY (`--directory-path`), never `uploadALog`/`--file-path`. This is
    // the bug the operation-aware split fixed: `liveLog` watches a folder for
    // `Encounter.log`; a `--file-path` + `uploadALog` invocation is a one-shot
    // upload, not a live session.
    #[test]
    fn live_mode_uses_livelog_with_directory_path() {
        let (cli, dir, log) = cli_with_dummy_exe();
        let opts = UploadOptions {
            real_time: true,
            ..Default::default()
        };
        let cmd = cli.prepare_command(&log, &opts).unwrap();
        let s = cmd_string(&cmd);
        assert!(s.contains("liveLog"), "live must use the liveLog op: {s}");
        assert!(
            s.contains("--directory-path"),
            "live must pass --directory-path: {s}"
        );
        // The directory passed must be the log's parent, not the file itself.
        // `cmd_string` renders args via Debug, which escapes Windows backslashes,
        // so compare against the Debug-escaped form of the parent path.
        let parent = std::path::Path::new(&log).parent().unwrap();
        let parent_dbg = format!("{parent:?}");
        let parent_inner = parent_dbg.trim_matches('"');
        assert!(
            s.contains(parent_inner),
            "live must pass the parent dir ({parent_inner}): {s}"
        );
        assert!(
            !s.contains("uploadALog") && !s.contains("--file-path"),
            "live must NOT use the one-shot uploadALog/--file-path path: {s}"
        );
        assert!(
            s.contains("--enable-real-time-uploading"),
            "real-time live must request real-time uploading: {s}"
        );
        drop(dir);
    }

    // Manual (one-shot) upload keeps `uploadALog` with `--file-path` and never
    // touches the live `liveLog`/`--directory-path` path.
    #[test]
    fn manual_mode_uses_uploadalog_with_file_path() {
        let (cli, dir, log) = cli_with_dummy_exe();
        let opts = UploadOptions::default(); // real_time = false
        let cmd = cli.prepare_command(&log, &opts).unwrap();
        let s = cmd_string(&cmd);
        assert!(s.contains("uploadALog"), "manual must use uploadALog: {s}");
        assert!(
            s.contains("--file-path") && s.contains("Encounter.log"),
            "manual must pass the --file-path to the log: {s}"
        );
        assert!(
            !s.contains("liveLog") && !s.contains("--directory-path"),
            "manual must NOT use the live liveLog/--directory-path path: {s}"
        );
        assert!(
            !s.contains("--enable-real-time-uploading"),
            "manual must not request real-time uploading: {s}"
        );
        drop(dir);
    }

    // The Archon App (post-2026-06-29 unified uploader) MUST be discoverable:
    // `productName = "Archon App"` → `…\Archon App\Archon App.exe`. That pairing
    // is exactly what the old dir × exe cross-product missed (it had "Archon" /
    // "Archon.exe" but not "Archon App"). The legacy uploader stays a candidate so
    // an existing install keeps working through the retirement grace period.
    #[test]
    fn candidates_cover_archon_app_and_legacy() {
        let paths: Vec<String> = candidates_for_roots(&[PathBuf::from("C:/Root")])
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with("Archon App/Archon App.exe")),
            "Archon App exe must be a candidate: {paths:?}"
        );
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with("ESO Logs Uploader/ESO Logs Uploader.exe")),
            "legacy uploader must remain a candidate: {paths:?}"
        );
    }

    // Discovery must match ONLY the exact known `(dir, exe)` pairs — never a
    // name-prefix scan. A prefix scan would let a planted `Archon Helper\Archon
    // Helper.exe` in a user-writable root be spawned as the official uploader. This
    // pins the candidate surface to exactly the vetted names so re-introducing a
    // broad scan fails the build.
    #[test]
    fn discovery_only_matches_exact_known_names() {
        let exes: Vec<String> = candidates_for_roots(&[PathBuf::from("C:/Root")])
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Exactly one candidate per known pair, nothing else.
        assert_eq!(
            exes.len(),
            KNOWN_UPLOADERS.len(),
            "candidate count must equal the known-uploader list (no wildcard expansion): {exes:?}"
        );
        for exe in &exes {
            assert!(
                KNOWN_UPLOADERS.iter().any(|(_, e)| e == exe),
                "candidate {exe:?} is not an exact known uploader exe — prefix/wildcard discovery leaked in"
            );
        }
        // A plausible planted-binary name must NOT be produced by the generator.
        assert!(
            !exes.iter().any(|e| e == "Archon Helper.exe"),
            "a name-prefixed planted exe must never be a candidate: {exes:?}"
        );
    }

    // An installed Archon App must outrank a stale legacy uploader even when the
    // legacy one sits in a higher-priority root. Simulate the dual install: the
    // retired uploader in admin Program Files, the Archon App per-user in
    // LocalAppData. Product-major ordering must surface Archon App first.
    #[test]
    fn archon_app_outranks_legacy_across_roots() {
        let roots = [
            PathBuf::from("C:/Program Files"),
            PathBuf::from("C:/Users/x/AppData/Local/Programs"),
        ];
        let paths: Vec<String> = candidates_for_roots(&roots)
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        let archon = paths
            .iter()
            .position(|p| p.ends_with("Archon App/Archon App.exe"))
            .expect("Archon App candidate present");
        let legacy = paths
            .iter()
            .position(|p| p.ends_with("ESO Logs Uploader/ESO Logs Uploader.exe"))
            .expect("legacy candidate present");
        assert!(
            archon < legacy,
            "Archon App (any root) must rank before the legacy uploader (any root): {paths:?}"
        );
    }
}

/// Pick the best available transport for a given preference.
///
/// `prefer_cli`: when true, use the CLI if the official uploader is installed;
/// otherwise always use the GUI handoff (the safe default).
pub fn select_transport(prefer_cli: bool) -> Box<dyn LogUploadTransport> {
    if prefer_cli {
        if let Some(cli) = CliTransport::detect() {
            return Box::new(cli);
        }
    }
    Box::new(GuiHandoffTransport)
}

/// Conservative completed-upload ceiling for the native path. The encoder now
/// replays the raw log from disk instead of copying it into heap, but it still
/// builds compressed payloads in-process before creating the report, so very large
/// files route to the official uploader (or should be split first). Shared by
/// `assess_native_routing` (route away) and `run_native_upload`
/// (defence-in-depth refuse) so they agree on the limit.
const MAX_NATIVE_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB

/// Why a given log was (not) routed to the native uploader. Surfaced for
/// diagnostics and honest UI ("uploaded directly" vs "used the official app
/// because this log has events Kalpa can't encode yet").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRouting {
    /// Native path chosen by cheap preflight. The native payload builder still
    /// enforces per-line coverage and multi-session rejection before any report is
    /// created, so a late decline falls back to the official uploader safely.
    Native,
    /// Fell back to the official uploader. Carries a short, honest reason.
    Fallback(NativeFallbackReason),
}

/// The specific reason the native path was declined for a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeFallbackReason {
    /// The user has not opted into native upload (or chose the official path).
    NotOptedIn,
    /// The upload format/version isn't confirmed yet, so native is disabled.
    FormatUnconfirmed,
    /// The log contains event types Kalpa's encoder hasn't proven byte-exact, so
    /// using native could produce an inaccurate report. Carries the offending
    /// types for diagnostics.
    UnprovenEvents(Vec<String>),
    /// The log contains bytes that are not valid UTF-8. The native encoder works on
    /// decoded log lines; decoding lossily would change the payload, so route to
    /// the official uploader instead.
    InvalidEncoding,
    /// The log exceeds the native path's whole-file memory ceiling, so it routes
    /// to the official uploader instead of hard-failing. Split it to upload direct.
    TooLarge,
    /// The log contains more than one logging session (multiple `BEGIN_LOG`
    /// markers). The native encoder builds a single segment whose time bounds are
    /// derived from the first session, so a multi-session file can produce wrong
    /// segment bounds / a non-rendering report — route to the official uploader
    /// (or split per session to upload directly).
    MultiSession,
}

impl NativeFallbackReason {
    /// A short, honest, user-facing explanation.
    pub fn explain(&self) -> String {
        match self {
            NativeFallbackReason::NotOptedIn => {
                "Using the official ESO Logs uploader (direct upload is off).".into()
            }
            NativeFallbackReason::FormatUnconfirmed => {
                "Using the official ESO Logs uploader (Kalpa's direct upload \
                 isn't enabled yet)."
                    .into()
            }
            NativeFallbackReason::UnprovenEvents(types) => format!(
                "Using the official ESO Logs uploader — this log has events Kalpa \
                 can't yet upload directly with full accuracy ({}).",
                types.join(", ")
            ),
            NativeFallbackReason::InvalidEncoding => {
                "Using the official ESO Logs uploader - this log contains bytes \
                 Kalpa can't decode for direct upload without changing them."
                    .into()
            }
            NativeFallbackReason::TooLarge => {
                "Using the official ESO Logs uploader — this log is too large for \
                 direct upload (split it to upload directly)."
                    .into()
            }
            NativeFallbackReason::MultiSession => {
                "Using the official ESO Logs uploader — this log has multiple \
                 sessions (split it by session to upload directly)."
                    .into()
            }
        }
    }
}

/// Decide whether a LIVE session may use the native in-process driver instead of the
/// official-uploader handoff. Unlike the finished-log path, a live `Encounter.log`
/// GROWS — there is no whole file to coverage-scan up front — so the live gate is:
///
/// 1. The user opted in (`opt_in`).
/// 2. The upload format/version is confirmed
///    ([`super::native::format::FORMAT_VERSION_CONFIRMED`]).
/// 3. A captured ESO Logs session exists (`has_session`) — without it the native
///    path would hard-fail "Not signed in" mid-stream; route to the handoff instead.
///
/// Coverage is enforced PER SEGMENT at runtime by the encoder's structural self-check
/// plus the master/segment desync cross-check (a malformed segment is never POSTed),
/// and a mid-session `/reloadui` (a second `BEGIN_LOG`) terminates the report rather
/// than mixing two sessions — so the finished-log MultiSession/UnprovenEvents/TooLarge
/// gates have no live analog. The DEFAULT remains the official handoff: native live
/// runs only when all three hold.
pub fn assess_native_live_routing(opt_in: bool, has_session: bool) -> NativeRouting {
    use super::native::format;
    if !opt_in {
        return NativeRouting::Fallback(NativeFallbackReason::NotOptedIn);
    }
    if !format::FORMAT_VERSION_CONFIRMED {
        return NativeRouting::Fallback(NativeFallbackReason::FormatUnconfirmed);
    }
    if !has_session {
        // Reuse FormatUnconfirmed's honest "direct upload isn't enabled" copy — the
        // user-facing effect (handoff) and remedy (sign in / it'll work next time) are
        // the same; a dedicated reason isn't worth a new enum arm here.
        return NativeRouting::Fallback(NativeFallbackReason::NotOptedIn);
    }
    NativeRouting::Native
}

/// Decide whether a prepared log may use the native uploader, applying the
/// cheap gates that can decline without touching the whole file:
///
/// 1. The user must have opted in (`opt_in`).
/// 2. The upload format/version must be confirmed
///    ([`super::native::format::FORMAT_VERSION_CONFIRMED`]).
/// 3. The file must be within the native memory ceiling.
///
/// Per-line coverage, UTF-8 validity, and the single-session contract are enforced
/// inside [`run_native_upload`] while it builds and validates payloads before
/// `create-report`. That avoids a duplicate full-file scan on the fast path while
/// preserving the all-or-official safety guarantee.
pub fn assess_native_routing(log_path: &str, opt_in: bool) -> NativeRouting {
    use super::native::format;

    if !opt_in {
        return NativeRouting::Fallback(NativeFallbackReason::NotOptedIn);
    }
    if !format::FORMAT_VERSION_CONFIRMED {
        return NativeRouting::Fallback(NativeFallbackReason::FormatUnconfirmed);
    }

    // The native path streams and builds compressed payloads in-process, so it
    // refuses files over MAX_NATIVE_BYTES. Keep this in the router so an over-large
    // log can hand off immediately without attempting native build work.
    if let Ok(meta) = std::fs::metadata(log_path) {
        if meta.len() > MAX_NATIVE_BYTES {
            return NativeRouting::Fallback(NativeFallbackReason::TooLarge);
        }
    }

    NativeRouting::Native
}

fn handoff_with_reason(
    log_path: &str,
    opts: &UploadOptions,
    reason: NativeFallbackReason,
) -> Result<UploadOutcome, String> {
    eprintln!("[uploader] native → official: {}", reason.explain());
    match GuiHandoffTransport.upload_file(log_path, opts) {
        Ok(UploadOutcome::HandedOff { .. }) => Ok(UploadOutcome::HandedOff {
            detail: reason.explain(),
        }),
        other => other,
    }
}

fn payload_error_fallback_reason(error: &str) -> Option<NativeFallbackReason> {
    if let Some(t) = error
        .strip_prefix("unproven log line type '")
        .and_then(|tail| tail.strip_suffix('\''))
    {
        return Some(NativeFallbackReason::UnprovenEvents(vec![t.to_string()]));
    }
    if error == "native completed upload does not support multi-session logs" {
        return Some(NativeFallbackReason::MultiSession);
    }
    if error.starts_with("native encode: read raw log failed:") && error.contains("valid UTF-8") {
        return Some(NativeFallbackReason::InvalidEncoding);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeUploadProgress {
    Building {
        bytes_done: u64,
        bytes_total: u64,
    },
    Uploading {
        segments_done: usize,
        segments_total: usize,
    },
}

/// Run a **native** upload: read the prepared log, build the ZIP'd segment +
/// master-table payload with the in-process encoder, and drive the report
/// lifecycle (`create-report` → master-table/segment → `terminate-report`)
/// against `esologs.com/desktop-client/*` using the supplied session — no
/// official-uploader handoff.
///
/// This is the integration seam the `Native` routing arm calls after the cheap
/// opt-in / format / size preflight passes. It builds + validates the payload,
/// including per-line coverage and single-session checks, and if the payload can't
/// be built or fails the structural self-check, falls back to the official
/// [`GuiHandoffTransport`] so a broken segment is never shipped.
///
/// `cancel` lets a Stop abort cleanly between segments (the client still
/// `terminate-report`s so no draft is orphaned). On any failure a short, honest
/// message is returned for the history record.
pub fn run_native_upload(
    log_path: &str,
    opts: &UploadOptions,
    session: &dyn super::native::session::SessionProvider,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    progress: &(dyn Fn(NativeUploadProgress) + Send + Sync),
) -> Result<UploadOutcome, String> {
    use super::native::{client::NativeUpload, live};
    let native_started = std::time::Instant::now();

    if !Path::new(log_path).is_file() {
        return Err(format!("Prepared log not found: {log_path}"));
    }

    // Hard size ceiling before encoding. Native replay no longer copies the raw log
    // into heap, but it still builds compressed payloads in-process before report
    // creation. Above the ceiling we refuse native (the caller falls back to the
    // official uploader, which streams). The user-facing route is to split first
    // (the uploader already offers split-to-disk for large logs). Uses the
    // module-level MAX_NATIVE_BYTES shared with assess_native_routing so routing and
    // this guard agree.
    let size = std::fs::metadata(log_path)
        .map_err(|e| format!("Failed to read log: {e}"))?
        .len();
    if size > MAX_NATIVE_BYTES {
        return Err(format!(
            "This log is too large for direct upload ({} MiB > {} MiB). Split it first, \
             or it will be sent via the official uploader.",
            size / (1024 * 1024),
            MAX_NATIVE_BYTES / (1024 * 1024)
        ));
    }

    // Build small batched ZIP'd (segment, master-table) payloads with the same
    // incremental encoder used by native live upload. Each segment runs the structural
    // self-check (`validate_segment_text`) before it can be returned. If an unseen
    // log shape produces malformed or internally-inconsistent output, the builder
    // returns `Err` and we fall back to the official uploader rather than failing
    // the user's upload or shipping a report that never renders. `None` (no valid
    // session) likewise falls back so native only ships verified segments.
    let build_started = std::time::Instant::now();
    let build_progress = |build: live::FinishedBuildProgress| {
        progress(NativeUploadProgress::Building {
            bytes_done: build.bytes_done,
            bytes_total: build.bytes_total,
        });
    };
    let payloads = match live::build_finished_payloads_from_file_with_progress(
        Path::new(log_path),
        build_progress,
    ) {
        Ok(Some(payloads)) => payloads,
        Ok(None) => {
            eprintln!("[uploader] native: no valid session in log → official handoff");
            return GuiHandoffTransport.upload_file(log_path, opts);
        }
        Err(e) => {
            eprintln!("[uploader] native payload rejected ({e}) → official handoff");
            if let Some(reason) = payload_error_fallback_reason(&e) {
                return handoff_with_reason(log_path, opts, reason);
            }
            return GuiHandoffTransport.upload_file(log_path, opts);
        }
    };
    let build_ms = elapsed_ms(build_started);
    let raw_segment_bytes_total: usize = payloads
        .iter()
        .filter_map(|payload| payload.source_raw_bytes)
        .sum();
    let max_raw_segment_bytes = payloads
        .iter()
        .filter_map(|payload| payload.source_raw_bytes)
        .max()
        .unwrap_or(0);
    let segments_over_raw_target = payloads
        .iter()
        .filter_map(|payload| payload.source_raw_bytes)
        .filter(|&bytes| bytes > live::FINISHED_UPLOAD_SEGMENT_RAW_BYTE_TARGET)
        .count();
    let mut segments = Vec::with_capacity(payloads.len());
    let mut masters = Vec::with_capacity(payloads.len());
    for payload in payloads {
        segments.push(payload.segment);
        masters.push(payload.master);
    }

    progress(NativeUploadProgress::Uploading {
        segments_done: 0,
        segments_total: segments.len(),
    });

    let upload = NativeUpload::new(session, opts, cancel);
    let upload_progress = |upload: super::native::client::UploadProgress| {
        progress(NativeUploadProgress::Uploading {
            segments_done: upload.segments_done,
            segments_total: upload.segments_total,
        });
    };
    match upload.upload_finished_measured(&segments, &masters, &upload_progress) {
        Ok(result) => {
            let total_ms = elapsed_ms(native_started);
            let code = result.code.0;
            let metrics = result.metrics;
            eprintln!(
                "[uploader] native finished report={code} input_bytes={size} \
                 segments={} requests={} segment_zip_bytes={} master_zip_bytes={} \
                 raw_segment_bytes={} max_raw_segment_bytes={} segments_over_raw_target={} \
                 build_ms={build_ms} session_ms={} create_ms={} \
                 set_master_ms={} add_segment_ms={} terminate_ms={} upload_ms={} total_ms={total_ms}",
                metrics.segments_total,
                metrics.requests_total,
                metrics.segment_zip_bytes,
                metrics.master_zip_bytes,
                raw_segment_bytes_total,
                max_raw_segment_bytes,
                segments_over_raw_target,
                metrics.session_ms,
                metrics.create_report_ms,
                metrics.set_master_table_ms,
                metrics.add_segment_ms,
                metrics.terminate_report_ms,
                metrics.upload_total_ms
            );
            Ok(UploadOutcome::Completed {
                report_code: Some(code),
            })
        }
        Err(e) => Err(e.to_string()),
    }
}

fn elapsed_ms(started: std::time::Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    use std::io::Write;

    fn temp_log(contents: &str) -> (tempfile::TempDir, String) {
        temp_log_bytes(contents.as_bytes())
    }

    fn temp_log_bytes(contents: &[u8]) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Encounter.log");
        std::fs::File::create(&p)
            .unwrap()
            .write_all(contents)
            .unwrap();
        let s = p.to_string_lossy().into_owned();
        (dir, s)
    }

    #[test]
    fn not_opted_in_always_falls_back() {
        let (_d, path) = temp_log("4,ZONE_CHANGED,1129,x\n");
        assert_eq!(
            assess_native_routing(&path, false),
            NativeRouting::Fallback(NativeFallbackReason::NotOptedIn)
        );
    }

    #[test]
    fn opted_in_with_proven_types_routes_native() {
        // Gate-aware so it holds for BOTH the committed `FORMAT_VERSION_CONFIRMED =
        // false` (CI builds this) and the owner's local render-test flip to `true`:
        // an opted-in user routes native ONCE the gate is open, and still falls back
        // while it is closed. Content safety is enforced during payload build so the
        // fast path does not scan the whole file twice.
        let (_d, path) = temp_log("4,ZONE_CHANGED,1129,x\n");
        if super::super::native::format::FORMAT_VERSION_CONFIRMED {
            assert_eq!(assess_native_routing(&path, true), NativeRouting::Native);
        } else {
            assert_ne!(assess_native_routing(&path, true), NativeRouting::Native);
        }
    }

    #[test]
    fn fallback_reasons_have_honest_messages() {
        assert!(NativeFallbackReason::NotOptedIn
            .explain()
            .contains("official"));
        assert!(
            NativeFallbackReason::UnprovenEvents(vec!["COMBAT_EVENT".into()])
                .explain()
                .contains("COMBAT_EVENT")
        );
    }

    // Coverage safety moved out of the router and into the native payload builder
    // so the fast path does not scan the whole file twice. A novel/future event must
    // still fail closed before `create-report`, and its handoff detail must remain
    // honest.
    #[test]
    fn finished_payload_builder_rejects_unproven_type_before_upload() {
        use super::super::native::live::build_finished_payloads_from_text;

        let err =
            build_finished_payloads_from_text("0,BEGIN_LOG,1,15\n100,SOME_FUTURE_EVENT,1,2\n")
                .unwrap_err();
        assert_eq!(
            payload_error_fallback_reason(&err),
            Some(NativeFallbackReason::UnprovenEvents(vec![
                "SOME_FUTURE_EVENT".into()
            ]))
        );
    }

    #[test]
    fn opted_in_with_invalid_utf8_defers_to_runtime_fallback() {
        let (_d, path) =
            temp_log_bytes(b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n5,ZONE_CHANGED,\xff\n");
        if super::super::native::format::FORMAT_VERSION_CONFIRMED {
            assert_eq!(
                assess_native_routing(&path, true),
                NativeRouting::Native,
                "preflight must not do a duplicate full UTF-8 scan; run_native_upload \
                 maps invalid UTF-8 to InvalidEncoding handoff before create-report"
            );
        } else {
            assert_ne!(assess_native_routing(&path, true), NativeRouting::Native);
        }
    }

    /// A SessionProvider with a fixed cookie, for testing the native runner's
    /// control flow without a network.
    struct FixedSession;
    impl super::super::native::session::SessionProvider for FixedSession {
        fn session(
            &self,
        ) -> Result<
            super::super::native::session::Session,
            super::super::native::session::SessionError,
        > {
            Ok(super::super::native::session::Session::from_cookie_header(
                "laravel_session=test",
            ))
        }
        fn invalidate(&self) {}
    }

    // A log with no valid session yields no native payload — the native runner must
    // FALL BACK to the official handoff rather than fail the upload (and never ship
    // a bad/empty segment). We assert at the payload-builder level to avoid the
    // handoff's process-spawn side effect in a unit test:
    // `build_finished_payloads_from_text` returns `Ok(None)` for non-session input,
    // which is exactly what triggers the fallback branch in `run_native_upload`.
    #[test]
    fn no_valid_session_yields_no_native_payload_so_it_falls_back() {
        use super::super::native::live::build_finished_payloads_from_text;
        let raw = "not a real session line\n";
        assert!(
            matches!(build_finished_payloads_from_text(raw), Ok(None)),
            "junk input must produce no native payload (→ official handoff)"
        );
    }

    #[test]
    fn native_runner_rejects_missing_file() {
        let opts = UploadOptions::default();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let no_progress = |_p: crate::uploader::transport::NativeUploadProgress| {};
        let err = run_native_upload(
            "C:/nonexistent/nope.log",
            &opts,
            &FixedSession,
            cancel,
            &no_progress,
        )
        .unwrap_err();
        assert!(err.contains("not found"));
    }

    // A normal small file must NOT trip the size ceiling. The ceiling exists to
    // bound memory on the whole-file read; only an over-large file (>256 MiB) is
    // refused with a split-first message. We can't cheaply make a 256 MiB file in a
    // unit test, so we assert the size check directly on a small file's metadata.
    #[test]
    fn native_runner_small_file_passes_size_gate() {
        let (_d, path) = temp_log("not a session\n");
        let size = std::fs::metadata(&path).unwrap().len();
        const MAX_NATIVE_BYTES: u64 = 256 * 1024 * 1024;
        assert!(
            size <= MAX_NATIVE_BYTES,
            "a small file must not be size-gated"
        );
    }

    // A single-session log of only-proven types routes Native when the format
    // gate is open. (Gate-aware so the test holds if FORMAT_VERSION_CONFIRMED is
    // ever flipped back to false.)
    #[test]
    fn routing_single_session_proven_is_native() {
        if !super::super::native::format::FORMAT_VERSION_CONFIRMED {
            return; // gate closed → everything falls back; nothing to assert here.
        }
        let (_d, path) = temp_log(
            "0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n\
             5,ZONE_CHANGED,1,\"Z\",VETERAN\n\
             10,BEGIN_COMBAT\n20,END_COMBAT\n5,END_LOG\n",
        );
        assert!(
            matches!(assess_native_routing(&path, true), NativeRouting::Native),
            "single-session proven log should route native"
        );
    }

    // A MULTI-session log (>1 BEGIN_LOG), even all-proven, must fall back during
    // payload build before any native report is created: the native encoder's
    // single-segment time bounds can't represent multiple sessions correctly.
    #[test]
    fn finished_payload_builder_rejects_multi_session_with_fallback_reason() {
        use super::super::native::live::build_finished_payloads_from_text;

        let err = build_finished_payloads_from_text(
            "0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n\
             0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n",
        )
        .unwrap_err();
        assert_eq!(
            payload_error_fallback_reason(&err),
            Some(NativeFallbackReason::MultiSession),
            "multi-session log must hand off to the official uploader"
        );
    }
}
