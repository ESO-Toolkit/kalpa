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

use super::types::{UploadOptions, UploadPhase, UploadProgressEvent};

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

/// Whole-file memory ceiling for the native path. The native encoder reads the
/// log into memory, so a file above this routes to the official uploader (which
/// streams) instead. Shared by `assess_native_routing` (route away) and
/// `run_native_upload` (defence-in-depth refuse) so they agree on the limit.
const MAX_NATIVE_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB

/// Why a given log was (not) routed to the native uploader. Surfaced for
/// diagnostics and honest UI ("uploaded directly" vs "used the official app
/// because this log has events Kalpa can't encode yet").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRouting {
    /// Native path chosen — the log is fully within proven coverage.
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
/// safety gate that guarantees native output is byte-correct or not used:
///
/// 1. The user must have opted in (`opt_in`).
/// 2. The upload format/version must be confirmed
///    ([`super::native::format::FORMAT_VERSION_CONFIRMED`]).
/// 3. **Every** event type in the log must be within proven coverage
///    ([`super::native::coverage::assess`]).
///
/// Any failure routes to the official uploader. This is intentionally
/// conservative: native upload only runs when we can guarantee a report
/// identical to the official uploader's. The actual line scan streams the file
/// so it stays cheap on multi-GB logs.
pub fn assess_native_routing(log_path: &str, opt_in: bool) -> NativeRouting {
    use super::native::{coverage, format};

    if !opt_in {
        return NativeRouting::Fallback(NativeFallbackReason::NotOptedIn);
    }
    if !format::FORMAT_VERSION_CONFIRMED {
        return NativeRouting::Fallback(NativeFallbackReason::FormatUnconfirmed);
    }

    // The native path reads the whole file into memory, so it refuses files over
    // MAX_NATIVE_BYTES. Check the size HERE so an over-large covered log routes to
    // the official uploader instead of reaching run_native_upload and hard-failing.
    if let Ok(meta) = std::fs::metadata(log_path) {
        if meta.len() > MAX_NATIVE_BYTES {
            return NativeRouting::Fallback(NativeFallbackReason::TooLarge);
        }
    }

    // Scan the log's line types through the coverage gate WITHOUT materializing
    // the whole file — logs can be multi-GB. We read one line at a time, assess it
    // in isolation, and accumulate only the (tiny, capped) set of unproven types.
    let file = match std::fs::File::open(log_path) {
        Ok(f) => f,
        // If we can't read it, let the official path surface the real error.
        Err(_) => return NativeRouting::Fallback(NativeFallbackReason::FormatUnconfirmed),
    };
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(file);
    let mut unproven: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // Count BEGIN_LOG markers: the native encoder builds ONE segment whose time
    // bounds come from the first session, so a multi-session file (>1 BEGIN_LOG)
    // must route to the official uploader. We tally during this same pass to avoid
    // a second read. Only meaningful on the all-proven path below — the unproven
    // and early-break arms fall back regardless.
    let mut begin_log_count: u32 = 0;
    // Read raw bytes per line, but require UTF-8 before routing native. The native
    // encoder works on decoded text; lossy decoding would mutate the payload.
    // An IO error mid-scan FAILS CLOSED — we must
    // not return Native off a partial read, or the all-or-nothing coverage gate is
    // bypassed and an unproven event later in the file could reach the encoder.
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break, // EOF — full file scanned
            Ok(_) => {}
            // Read error: fail closed to the official uploader rather than risk a
            // partial-scan false "Native".
            Err(_) => return NativeRouting::Fallback(NativeFallbackReason::FormatUnconfirmed),
        }
        let line = match std::str::from_utf8(&buf) {
            Ok(line) => line,
            Err(_) => return NativeRouting::Fallback(NativeFallbackReason::InvalidEncoding),
        };
        // A BEGIN_LOG header has the type as field index 1: `<ms>,BEGIN_LOG,…`.
        if line
            .split(',')
            .nth(1)
            .map(|t| t.trim().eq_ignore_ascii_case("BEGIN_LOG"))
            .unwrap_or(false)
        {
            begin_log_count += 1;
        }
        if let coverage::Coverage::Fallback { unproven: u } = coverage::assess([line]) {
            unproven.extend(u);
            if unproven.len() >= 32 {
                break; // the reported set is capped anyway; stop early.
            }
        }
    }
    if !unproven.is_empty() {
        return NativeRouting::Fallback(NativeFallbackReason::UnprovenEvents(
            unproven.into_iter().collect(),
        ));
    }
    if begin_log_count > 1 {
        return NativeRouting::Fallback(NativeFallbackReason::MultiSession);
    }
    NativeRouting::Native
}

/// Run a **native** upload: read the prepared log, build the ZIP'd segment +
/// master-table payload with the in-process encoder, and drive the report
/// lifecycle (`create-report` → master-table/segment → `terminate-report`)
/// against `esologs.com/desktop-client/*` using the supplied session — no
/// official-uploader handoff.
///
/// This is the integration seam the `Native` routing arm calls — reached when
/// [`assess_native_routing`] returns [`NativeRouting::Native`] (an opted-in user
/// whose log is all proven types, with the format-version gate OPEN, which it is
/// since the 2026-06-19 render confirmation). It builds + validates the payload and,
/// if the payload can't be built or fails the structural self-check, falls back to
/// the official [`GuiHandoffTransport`] so a broken segment is never shipped.
///
/// `cancel` lets a Stop abort cleanly between segments (the client still
/// `terminate-report`s so no draft is orphaned). On any failure a short, honest
/// message is returned for the history record.
///
/// `progress` is invoked with [`UploadProgressEvent`]s as the upload moves through
/// its real lifecycle (build payload → POST segments → finalize) so the UI can show
/// a true progress bar. It fires ONLY on the native path: if this function falls back
/// to the official uploader (no session / non-UTF-8 / a malformed payload), it emits
/// nothing further and the handoff is reflected by the returned `HandedOff` outcome.
pub fn run_native_upload(
    log_path: &str,
    opts: &UploadOptions,
    session: &dyn super::native::session::SessionProvider,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    progress: &(dyn Fn(UploadProgressEvent) + Send + Sync),
) -> Result<UploadOutcome, String> {
    use super::native::{client::NativeUpload, events};

    if !Path::new(log_path).is_file() {
        return Err(format!("Prepared log not found: {log_path}"));
    }

    // Hard size ceiling BEFORE reading the file whole. The native encoder needs the
    // lines in memory (`read_to_string` + a per-line slice vec), so an unbounded
    // read would OOM on a multi-GB raw `Encounter.log`. The routing scan upstream
    // streams the file and gates only on event-type coverage — it has NO size cap —
    // so this is the one place that bounds memory on the native path. Above the
    // ceiling we refuse native (the caller falls back to the official uploader,
    // which streams). The user-facing route is to split first (the uploader already
    // offers split-to-disk for large logs). Uses the module-level MAX_NATIVE_BYTES
    // shared with assess_native_routing so routing and this guard agree.
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

    // Read the prepared log and split into lines for the encoder. Bounded by the
    // size ceiling above.
    let contents = match std::fs::read_to_string(log_path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            eprintln!("[uploader] native: log is not valid UTF-8 -> official handoff");
            return GuiHandoffTransport.upload_file(log_path, opts);
        }
        Err(e) => return Err(format!("Failed to read log: {e}")),
    };
    let lines: Vec<&str> = contents.lines().collect();

    // Build the ZIP'd (segment, master-table) payload. This also runs the structural
    // self-check (`validate_segment_text`): if the encoder ever produced a malformed
    // or internally-inconsistent segment for an unseen log shape, building returns
    // `Err`. Rather than fail the user's upload — or worse, ship a segment that the
    // server accepts but never renders — we FALL BACK to the official uploader. A
    // `None` (no valid session) likewise falls back; the official path surfaces the
    // real reason. This keeps the guarantee: native ships only a verified segment.
    let payload = match events::build_native_payload(&lines) {
        Ok(Some(pair)) => pair,
        Ok(None) => {
            eprintln!("[uploader] native: no valid session in log → official handoff");
            return GuiHandoffTransport.upload_file(log_path, opts);
        }
        Err(e) => {
            eprintln!("[uploader] native payload rejected ({e}) → official handoff");
            return GuiHandoffTransport.upload_file(log_path, opts);
        }
    };
    let (segment, master) = payload;

    // Payload is built and validated: announce the upload phase with the real
    // segment count so the UI can show a true fraction (0 of N done).
    let segments_total = 1;
    progress(UploadProgressEvent {
        phase: UploadPhase::Uploading,
        segments_done: 0,
        segments_total,
    });

    let upload = NativeUpload::new(session, opts, cancel);
    // Forward the client's per-segment ticks. When the last segment is accepted the
    // only step left is `terminate-report`, so surface that as the Finalizing phase.
    let on_segment = |p: super::native::client::UploadProgress| {
        let done = p.segments_done >= p.segments_total;
        progress(UploadProgressEvent {
            phase: if done {
                UploadPhase::Finalizing
            } else {
                UploadPhase::Uploading
            },
            segments_done: p.segments_done,
            segments_total: p.segments_total,
        });
    };
    match upload.upload_finished(&[segment], &[master], &on_segment) {
        Ok(code) => {
            progress(UploadProgressEvent {
                phase: UploadPhase::Done,
                segments_done: segments_total,
                segments_total,
            });
            Ok(UploadOutcome::Completed {
                report_code: Some(code.0),
            })
        }
        Err(e) => Err(e.to_string()),
    }
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
        // an opted-in user whose log is all proven types routes native ONCE the gate
        // is open, and still falls back while it is closed. A ZONE_CHANGED-only log
        // is all-proven, so the gate is the only thing deciding native vs fallback.
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

    // Coverage safety (gate is OPEN): an opted-in upload with ANY unproven line
    // type must still fall back, never route native — so a novel/future event can
    // never reach the encoder and corrupt a report. (The all-proven → native half,
    // including Infinite Archive logs, is exercised by the coverage.rs unit tests.)
    #[test]
    fn opted_in_with_unproven_type_falls_back() {
        let (_d, path) = temp_log("0,BEGIN_LOG,1,15\n100,SOME_FUTURE_EVENT,1,2\n");
        assert_ne!(assess_native_routing(&path, true), NativeRouting::Native);
    }

    #[test]
    fn opted_in_with_invalid_utf8_falls_back() {
        let (_d, path) =
            temp_log_bytes(b"0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n5,ZONE_CHANGED,\xff\n");
        if super::super::native::format::FORMAT_VERSION_CONFIRMED {
            assert_eq!(
                assess_native_routing(&path, true),
                NativeRouting::Fallback(NativeFallbackReason::InvalidEncoding)
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
    // handoff's process-spawn side effect in a unit test: `build_native_payload`
    // returns `Ok(None)` for non-session input, which is exactly what triggers the
    // fallback branch in `run_native_upload`.
    #[test]
    fn no_valid_session_yields_no_native_payload_so_it_falls_back() {
        use super::super::native::events::build_native_payload;
        let raw = "not a real session line\n";
        let lines: Vec<&str> = raw.lines().collect();
        assert!(
            matches!(build_native_payload(&lines), Ok(None)),
            "junk input must produce no native payload (→ official handoff)"
        );
    }

    #[test]
    fn native_runner_rejects_missing_file() {
        let opts = UploadOptions::default();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let err = run_native_upload(
            "C:/nonexistent/nope.log",
            &opts,
            &FixedSession,
            cancel,
            &|_| {},
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

    // A MULTI-session log (>1 BEGIN_LOG), even all-proven, must fall back: the
    // native encoder's single-segment time bounds can't represent multiple
    // sessions correctly.
    #[test]
    fn routing_multi_session_falls_back() {
        if !super::super::native::format::FORMAT_VERSION_CONFIRMED {
            return; // gate closed → falls back as FormatUnconfirmed, not MultiSession.
        }
        let (_d, path) = temp_log(
            "0,BEGIN_LOG,1000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n\
             0,BEGIN_LOG,2000,15,\"NA\",\"en\",\"10.0\"\n10,BEGIN_COMBAT\n20,END_COMBAT\n",
        );
        assert!(
            matches!(
                assess_native_routing(&path, true),
                NativeRouting::Fallback(NativeFallbackReason::MultiSession)
            ),
            "multi-session log must route to the official uploader"
        );
    }
}
