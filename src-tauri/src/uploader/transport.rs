//! Upload transport abstraction.
//!
//! The actual upload to ESO Logs is performed by the **official ESO Logs /
//! Archon uploader**, which Kalpa drives rather than re-implementing. The native
//! `/desktop-client/` REST protocol is private, version-coupled, and against
//! RPGLogs' Terms of Service for third-party clients, so we deliberately do not
//! speak it ourselves. Driving the official app keeps uploads legitimate and
//! stable across protocol changes.
//!
//! Two transports are provided behind one trait so the strategy can evolve:
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

/// Candidate install locations for the official ESO Logs / Archon uploader on
/// Windows. The app historically installs per-user under LocalAppData.
fn official_uploader_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    // Per-user (Squirrel/Electron default) and Program Files installs.
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from);
    let pf86 = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from);

    // Both the legacy "ESO Logs Uploader" and the rebranded "Archon" app.
    let exe_names = ["ESO Logs Uploader.exe", "Archon.exe", "ESO Logs.exe"];
    let app_dirs = ["ESO Logs Uploader", "Archon", "eso-logs-uploader"];

    for base in [local, pf, pf86].into_iter().flatten() {
        for app in app_dirs {
            for exe in exe_names {
                out.push(base.join(app).join(exe));
                // Electron Squirrel keeps the exe at the install root too.
                out.push(base.join("Programs").join(app).join(exe));
            }
        }
    }
    out
}

/// Find the official uploader executable, if installed.
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
                .map_err(|e| format!("Failed to launch the ESO Logs Uploader: {e}"))?;
            Ok(UploadOutcome::HandedOff {
                detail: "Opened the ESO Logs Uploader with your prepared log.".into(),
            })
        } else {
            // Not installed — open the official download page; the file is ready
            // on disk for the user to drag in.
            open_url(UPLOADER_DOWNLOAD_URL)?;
            Ok(UploadOutcome::HandedOff {
                detail: "The ESO Logs Uploader isn't installed. Opened the \
                         download page; your prepared log is ready on disk."
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

impl LogUploadTransport for CliTransport {
    fn name(&self) -> &'static str {
        "Official Uploader (CLI)"
    }

    fn is_available(&self) -> bool {
        self.exe.is_file()
    }

    fn upload_file(&self, log_path: &str, opts: &UploadOptions) -> Result<UploadOutcome, String> {
        if !Path::new(log_path).is_file() {
            return Err(format!("Prepared log not found: {log_path}"));
        }

        let mut cmd = std::process::Command::new(&self.exe);
        cmd.arg("--operation-name")
            .arg("uploadALog")
            .arg("--file-path")
            .arg(log_path)
            .arg("--region")
            .arg(opts.region.to_string());

        match &opts.guild_id {
            Some(g) if !g.is_empty() => {
                cmd.arg("--guild").arg(g);
            }
            _ => {
                cmd.arg("--guild").arg("null");
            }
        }
        if opts.real_time {
            cmd.arg("--enable-real-time-uploading");
        }
        if opts.include_entire_file {
            cmd.arg("--include-entire-file");
        }

        let status = cmd
            .status()
            .map_err(|e| format!("Failed to run the ESO Logs Uploader CLI: {e}"))?;

        if status.success() {
            Ok(UploadOutcome::Completed { report_code: None })
        } else {
            Err(format!(
                "The ESO Logs Uploader exited with status {}. Try the manual \
                 handoff instead.",
                status.code().unwrap_or(-1)
            ))
        }
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
