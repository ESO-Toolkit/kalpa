//! The shader-pack library: what you have, what exists, and installing it.
//!
//! This is the **first production caller of the install path**. Everything the
//! client manager shipped before it either read a folder or rearranged files
//! already in it; `client_backup::apply_placements`,
//! `client_download::download_to_temp` and the signature verifier had no
//! callers at all. Shader packs go first deliberately: they are the lowest
//! blast radius in the whole build order — no DLL, nothing next to `eso64.exe`,
//! everything confined to `reshade-shaders/` — and installing one is what
//! finally gives the motion-vector slot something to choose between.
//!
//! # Why this is a directory and not a store
//!
//! Kalpa hosts and mirrors nothing. Every pack is fetched from its author's own
//! GitHub at install time, which keeps users on current versions and means
//! there is no Kalpa-owned mirror to become a supply-chain target — the exact
//! failure mode that had DLSS Swapper's hash-vetted community manifest serving
//! malware in 2026.
//!
//! It also means **licensing decides what can be listed and what can be
//! fetched, and those are different questions.** [`PackSource::LinkOnly`]
//! exists because several of the shaders a real DLSS 5 stack uses cannot be
//! fetched at all:
//!
//! * **iMMERSE** is `All rights reserved` plus a prohibition — *"Public
//!   propagation of this project or parts of it is strictly forbidden"* —
//!   scoped to independently hosting a copy, with execution and private copies
//!   carved out. That arguably permits fetching from Gilcher's own repository,
//!   the same act his README tells users to perform by hand, but the licence
//!   never affirmatively grants anything and reserves the right to change at
//!   any time. Kalpa is BSL 1.1 converting to Apache 2.0, so the conservative
//!   reading wins until the author says otherwise. This is a one-line change
//!   to reverse if he does.
//! * **qUINT** has no licence file at all, and separately ships no motion
//!   vectors on any branch — the shader people mean by that name lives in a
//!   Gist (not an allowlisted host) and in `ReShade-Optical-Flow`.
//! * **DRME** is CC BY-NC 4.0. NonCommercial is not compatible with a
//!   commercially-intended product fetching it on the user's behalf, and its
//!   author has been inactive since 2023, so assume no sign-off is available.
//!
//! Listing a pack Kalpa cannot fetch is still worth doing: the user needs to
//! know the option exists and where it comes from, which is what makes this a
//! directory. Pretending it does not exist would be less honest, and offering
//! a disabled button would be a promise.
//!
//! # Pinning
//!
//! **None of these repositories has a single tag or release.** The GitHub
//! Releases API would be useless even if `api.github.com` were on the allowlist
//! (it is not). So a version is a commit SHA, resolved from
//! `github.com/<owner>/<repo>/commits/<branch>.atom` — an allowlisted host —
//! and the archive is then fetched by that exact SHA rather than by branch
//! name. Fetching `refs/heads/<branch>` directly would mean the bytes installed
//! are whatever the branch pointed at that second, unrecorded and
//! irreproducible; pinning makes an install describable after the fact.
//!
//! # What gets written
//!
//! Only files whose destination `client_write::validate_placement` accepts for
//! [`ManagedKind::Shader`], under `reshade-shaders/`. Archives carry plenty
//! that must not be written — `.git*`, CI config, sometimes an addon binary —
//! and the filter is an allowlist, not a denylist, so an archive gaining a new
//! kind of file cannot smuggle it through.
//!
//! Every archive path goes through [`client_write::safe_relative_join`] before
//! it is used. A zip entry name is attacker-controlled input in the same sense
//! a config value is: `../../eso64.exe` and `C:\Windows\System32\evil.dll` are
//! both things a malicious archive can contain, and on Windows the second is
//! the more dangerous, because `Path::join` treats a segment starting with a
//! drive letter as *drive-relative* and silently discards the base.

use crate::client_backup::Placement;
use crate::client_download::{download_to_temp, DownloadSpec};
use crate::client_write::{safe_relative_join, validate_placement, ManagedKind};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

/// Where a pack's shaders sit inside its downloaded archive.
///
/// GitHub zips everything under one top-level directory named
/// `<repo>-<ref>`, which is stripped first in both cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveLayout {
    /// `Shaders/` and `Textures/` at the archive root, mapping onto the same
    /// names under `reshade-shaders/`. Every catalogued pack but DRME is this
    /// shape.
    ShadersAndTextures,
    /// No `Shaders/` directory: the `.fx` and `.fxh` files sit at the archive
    /// root and all map into `reshade-shaders/Shaders/`. DRME is like this, and
    /// its README says so explicitly.
    FlatRoot,
}

/// Whether Kalpa may fetch a pack, and if not, the reason in the user's terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackSource {
    Fetchable {
        owner: &'static str,
        repo: &'static str,
        branch: &'static str,
    },
    /// Named, described and linked — never downloaded. `reason` is shown
    /// verbatim, because "Kalpa can't install this" without a why reads as a
    /// missing feature rather than a deliberate limit.
    LinkOnly {
        url: &'static str,
        reason: &'static str,
    },
}

/// One shader pack in the directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShaderPack {
    pub id: &'static str,
    pub name: &'static str,
    pub author: &'static str,
    pub summary: &'static str,
    /// The licence, as a **badge**: short enough to sit in a pill beside four
    /// others without setting the row width.
    ///
    /// "All rights reserved (see the repository's LICENSE)" was the honest long
    /// form, and it was half the width of the pack row — so the action buttons
    /// down the right edge stopped aligning and the list lost its spine. The
    /// full wording belongs in the pack's own LICENSE file, which the install
    /// path now copies into the shader tree anyway.
    pub licence: &'static str,
    pub source: PackSource,
    pub layout: ArchiveLayout,
    /// Technique **identifiers** this pack declares — never `ui_label`s.
    ///
    /// A ReShade preset records the identifier after the `technique` keyword;
    /// the label beside it is overlay display text and never reaches the file.
    /// Matching on labels is what made `MvProviderKind` unable to resolve three
    /// of its five providers, so these were read from the shader sources.
    pub techniques: &'static [&'static str],
    /// Files whose presence under the shader tree means this pack is here.
    ///
    /// Matched by file name anywhere in the tree rather than by exact relative
    /// path, because users install shaders by hand into layouts of their own
    /// and a pack installed differently is still installed.
    pub markers: &'static [&'static str],
}

/// The directory.
///
/// Ordered so the two upstream recommends for DLSS 5 come first. The order is
/// not a ranking of quality — Kalpa has no basis for one — it is "most likely
/// to be what you came here for".
pub const PACKS: &[ShaderPack] = &[
    ShaderPack {
        id: "lumenite",
        name: "LumeniteFX",
        author: "Afzaal (Kaidō)",
        summary: "Motion vectors, temporal AA and ambient occlusion. Upstream's \
                  recommended motion-vector provider for DLSS 5.",
        licence: "AGNYA 1.4",
        source: PackSource::Fetchable {
            owner: "umar-afzaal",
            repo: "LumeniteFX",
            // Not `main`. The default branch is `mainline`, and a zip request
            // for a branch that does not exist is a 404, not a fallback.
            branch: "mainline",
        },
        layout: ArchiveLayout::ShadersAndTextures,
        techniques: &["Lumenite_Kernel", "Lumenite_QuantMotion"],
        markers: &["lumenite_Kernel.fx", "lumenite_QuantMotion.fx"],
    },
    ShaderPack {
        id: "vort",
        name: "VORT",
        author: "Vortigern",
        summary: "Motion blur, temporal AA and motion estimation.",
        licence: "MIT",
        source: PackSource::Fetchable {
            owner: "vortigern11",
            repo: "vort_Shaders",
            branch: "main",
        },
        layout: ArchiveLayout::ShadersAndTextures,
        techniques: &["vort_MotionEffects", "vort_StaticEffects"],
        markers: &["vort_Motion.fx", "vort_Static.fx"],
    },
    ShaderPack {
        id: "dh_uber",
        name: "dh_uber_motion",
        author: "AlucardDH",
        summary: "Optical-flow motion vectors written to the shared \
                  texMotionVectors texture.",
        licence: "GPL-2.0",
        source: PackSource::Fetchable {
            owner: "AlucardDH",
            repo: "dh-reshade-shaders",
            branch: "master",
        },
        layout: ArchiveLayout::ShadersAndTextures,
        // The version is embedded in the technique name upstream, so this will
        // need updating when the pack does. Deliberately not pattern-matched:
        // guessing which `DH_UBER_MOTION_*` a preset means is the kind of
        // inference that produces a confident wrong answer.
        techniques: &["DH_UBER_MOTION_020"],
        markers: &["dh_uber_motion.fx"],
    },
    ShaderPack {
        id: "immerse",
        name: "iMMERSE",
        author: "Pascal Gilcher (Marty McFly)",
        summary: "Launchpad, MXAO, anti-aliasing and more. Launchpad is a \
                  DLSS 5 motion-vector provider and is in the free repository.",
        licence: "All rights reserved",
        source: PackSource::LinkOnly {
            url: "https://github.com/martymcmodding/iMMERSE",
            reason: "This licence forbids redistributing the project and grants \
                     nothing explicitly, so Kalpa will not fetch it for you. \
                     Download it from the author's page and unzip it into \
                     reshade-shaders yourself.",
        },
        layout: ArchiveLayout::ShadersAndTextures,
        techniques: &["MartysMods_Launchpad"],
        markers: &["MartysMods_LAUNCHPAD.fx"],
    },
    ShaderPack {
        id: "drme",
        name: "ReShade Motion Estimation (DRME)",
        author: "Jakob Wapenhensch",
        summary: "The original shared texMotionVectors provider. Superseded, \
                  but still what several older presets expect.",
        licence: "CC BY-NC 4.0",
        source: PackSource::LinkOnly {
            url: "https://github.com/JakobPCoder/ReshadeMotionEstimation",
            reason: "The NonCommercial licence means Kalpa will not fetch this \
                     for you. Download it from the author's page and copy every \
                     .fx and .fxh file into reshade-shaders/Shaders.",
        },
        layout: ArchiveLayout::FlatRoot,
        techniques: &["DRME"],
        markers: &["MotionEstimation.fx"],
    },
];

pub fn pack_by_id(id: &str) -> Option<&'static ShaderPack> {
    PACKS.iter().find(|pack| pack.id == id)
}

/// A pack as the panel needs to render it: the catalogue entry plus whether it
/// is actually here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackStatus {
    #[serde(flatten)]
    pub pack: &'static ShaderPack,
    pub installed: bool,
    /// Marker files found, so the panel can say what it matched on rather than
    /// asserting "installed" with nothing to show for it.
    pub found: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShaderLibrary {
    pub client_dir: String,
    pub shader_tree_present: bool,
    pub packs: Vec<PackStatus>,
}

/// Every file name under `reshade-shaders/`, lowercased.
///
/// Walks the whole tree rather than the two directories the layouts imply:
/// a hand-installed pack is still installed, and reporting "not installed" for
/// shaders the user is demonstrably running would make the panel wrong about
/// the one thing it can see directly.
fn shader_file_names(client_dir: &Path) -> BTreeSet<String> {
    fn walk(dir: &Path, out: &mut BTreeSet<String>, depth: usize) {
        // Shader trees are shallow. The bound is here so a symlink loop or a
        // pathological tree cannot turn a panel refresh into a hang.
        if depth > 8 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out, depth + 1);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                out.insert(name.to_ascii_lowercase());
            }
        }
    }

    let mut out = BTreeSet::new();
    walk(&client_dir.join("reshade-shaders"), &mut out, 0);
    out
}

/// Read-only: which packs are in this client folder, and what else exists.
pub fn read_library(client_dir: &Path) -> ShaderLibrary {
    let names = shader_file_names(client_dir);
    let packs = PACKS
        .iter()
        .map(|pack| {
            let found: Vec<String> = pack
                .markers
                .iter()
                .filter(|marker| names.contains(&marker.to_ascii_lowercase()))
                .map(|marker| (*marker).to_string())
                .collect();
            PackStatus {
                pack,
                installed: !found.is_empty(),
                found,
            }
        })
        .collect();

    ShaderLibrary {
        client_dir: client_dir.to_string_lossy().to_string(),
        shader_tree_present: client_dir.join("reshade-shaders").is_dir(),
        packs,
    }
}

// ── Archive planning ─────────────────────────────────────────────────────

/// Where one archive entry should land, relative to the client directory.
///
/// Returns `None` for anything that should not be written at all, which is the
/// common case: an archive is mostly not shaders.
///
/// `entry` is the path inside the zip **after** the single top-level directory
/// GitHub adds has been stripped.
fn destination_for(entry: &str, layout: ArchiveLayout) -> Option<String> {
    let normalized = entry.replace('\\', "/");
    // A directory entry, or something that escaped the strip. Both are nothing
    // to write; the containment check below is what makes that safe rather than
    // merely tidy.
    if normalized.is_empty() || normalized.ends_with('/') {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    let file_name = lower.rsplit('/').next()?;
    let extension = file_name.rsplit('.').next().unwrap_or_default();

    // Allowlist, not denylist. An archive that starts shipping a new kind of
    // file gets it dropped rather than written.
    let is_shader = matches!(extension, "fx" | "fxh");
    let is_texture = matches!(extension, "png" | "jpg");
    // Attribution has to travel with the files: LumeniteFX's NOTICE carries the
    // MIT notices for Glamarye's Fast Effects and Alan Wolfe's blue-noise
    // texture, and MIT requires those accompany copies. Installing the shaders
    // and dropping the notice would be a licence breach by omission.
    let is_notice = matches!(
        file_name,
        "license" | "license.md" | "license.txt" | "notice" | "notice.md" | "notice.txt"
    );
    if !(is_shader || is_texture || is_notice) {
        return None;
    }

    let rest = match layout {
        ArchiveLayout::FlatRoot => {
            // Flat archives have no directories worth preserving; everything
            // lands directly under Shaders/.
            let name = normalized.rsplit('/').next()?;
            format!("Shaders/{name}")
        }
        ArchiveLayout::ShadersAndTextures => {
            if is_notice {
                // Notices live at the archive root, which maps nowhere on its
                // own. Park them beside the shaders they belong to.
                let name = normalized.rsplit('/').next()?;
                format!("Shaders/{name}")
            } else if lower.starts_with("shaders/") || lower.starts_with("textures/") {
                normalized.clone()
            } else {
                // Anything outside those two directories is not part of the
                // shader tree — docs, CI config, screenshots.
                return None;
            }
        }
    };

    Some(format!("reshade-shaders/{rest}"))
}

/// Strip the single top-level directory GitHub wraps an archive in.
///
/// Returns `None` when the entry is that directory itself, or when it does not
/// live under the expected prefix — a zip whose entries do not share one root
/// is not the shape this function is being asked about, and guessing would mean
/// writing files from an archive that is not what it claimed to be.
fn strip_archive_root<'a>(entry: &'a str, root: &str) -> Option<&'a str> {
    let rest = entry.strip_prefix(root)?.trim_start_matches('/');
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// The common top-level directory of every entry, if there is exactly one.
fn archive_root(names: &[String]) -> Option<String> {
    let mut roots = BTreeSet::new();
    for name in names {
        let normalized = name.replace('\\', "/");
        let first = normalized.split('/').next()?.to_string();
        if first.is_empty() {
            return None;
        }
        roots.insert(first);
    }
    if roots.len() == 1 {
        roots.into_iter().next()
    } else {
        None
    }
}

/// What an install would write, derived from the archive's entry names alone.
///
/// Split out from the extraction so the mapping is testable without building a
/// zip: this is where a path escape would happen, and it is worth being able to
/// assert on it directly.
pub fn plan_destinations(
    entry_names: &[String],
    layout: ArchiveLayout,
    client_root: &Path,
) -> Result<Vec<(String, String)>, String> {
    let root = archive_root(entry_names)
        .ok_or_else(|| "This archive does not have the single top-level folder a GitHub source archive has. Refusing to install it.".to_string())?;

    let mut out = Vec::new();
    for name in entry_names {
        let normalized = name.replace('\\', "/");
        let Some(rest) = strip_archive_root(&normalized, &root) else {
            continue;
        };

        // Refuse the whole archive on a traversal segment, before any layout
        // mapping happens.
        //
        // Containment alone would not catch this for `ArchiveLayout::FlatRoot`,
        // which keeps only the file name — so `../../evil.fx` *flattens* into a
        // perfectly valid `Shaders/evil.fx` and installs. The write would be
        // safe, but silently normalising a traversal into a benign path throws
        // away the only evidence that the archive is not what it claims to be.
        // A GitHub source archive never contains `..`; one that does is not a
        // file layout question, it is a reason to stop.
        if rest.split('/').any(|segment| segment == "..") {
            return Err(format!(
                "This archive contains a path that tries to escape its own folder ({normalized}). \
                 Kalpa will not install it."
            ));
        }

        let Some(destination) = destination_for(rest, layout) else {
            continue;
        };

        // Both gates, on every entry, before it counts as a destination.
        //
        // `safe_relative_join` rejects `..`, absolute paths, UNC prefixes and
        // Windows reserved device names — and, critically on Windows, a segment
        // beginning with a drive letter, which `Path::join` would treat as
        // drive-relative and resolve against the current directory instead of
        // the client folder. `validate_placement` then requires the destination
        // to be somewhere a shader may actually go, which is what stops a file
        // called `dxgi.dll` inside a shader archive from being written as one.
        safe_relative_join(client_root, &destination)?;
        validate_placement(ManagedKind::Shader, &destination)?;

        out.push((normalized, destination));
    }

    if out.is_empty() {
        return Err("This archive contained no shader files Kalpa could install.".to_string());
    }
    Ok(out)
}

// ── Fetching ─────────────────────────────────────────────────────────────

/// Resolve a branch to the commit SHA it currently points at.
///
/// Uses the repository's Atom commit feed, which lives on `github.com` and is
/// therefore already on the download allowlist — unlike `api.github.com`, which
/// is not, and which would be no use anyway because none of these repositories
/// publishes releases or tags.
///
/// The SHA appears as `tag:github.com,2008:Grit::Commit/<sha>`.
pub fn resolve_head_sha(owner: &str, repo: &str, branch: &str) -> Result<String, String> {
    let url = format!("https://github.com/{owner}/{repo}/commits/{branch}.atom");
    let spec = DownloadSpec {
        url,
        expected_sha256: None,
        // A commit feed is a few kilobytes. Anything remotely near this cap is
        // not the document being asked for.
        max_bytes: Some(2 * 1024 * 1024),
    };
    let file = download_to_temp(&spec, &|_| {})?;
    let text = std::fs::read_to_string(file.path())
        .map_err(|e| format!("Could not read the commit feed: {e}"))?;

    const MARKER: &str = "Grit::Commit/";
    let start = text
        .find(MARKER)
        .ok_or_else(|| format!("Could not find a commit id for {owner}/{repo} on {branch}."))?
        + MARKER.len();
    let sha: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    if sha.len() != 40 {
        return Err(format!(
            "The commit id for {owner}/{repo} on {branch} was not a 40-character hash."
        ));
    }
    Ok(sha)
}

/// The pinned source archive for a resolved commit.
pub fn archive_url(owner: &str, repo: &str, sha: &str) -> String {
    format!("https://codeload.github.com/{owner}/{repo}/zip/{sha}")
}

/// Shader repositories are small; the largest of the five is about 4 MB.
/// A cap well above that but far below the global one keeps a surprise
/// multi-hundred-megabyte body from being streamed to disk at all.
pub const MAX_PACK_BYTES: u64 = 64 * 1024 * 1024;

// ── Placements ───────────────────────────────────────────────────────────

/// Extract the planned entries into `staging` and return the placements.
///
/// Writes into a staging directory rather than the client folder: everything
/// that reaches `apply_placements` must already exist as a real file, because
/// that is what lets the batch be transactional. A failure part-way through
/// extraction leaves the client folder untouched, and the staging directory is
/// a `TempDir` the caller drops.
pub fn stage_entries(
    archive: &Path,
    plan: &[(String, String)],
    staging: &Path,
) -> Result<Vec<Placement>, String> {
    let file = std::fs::File::open(archive)
        .map_err(|e| format!("Could not open the downloaded archive: {e}"))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("The download was not a valid zip archive: {e}"))?;

    let mut placements = Vec::new();
    for (entry_name, destination) in plan {
        let mut entry = zip
            .by_name(entry_name)
            .map_err(|e| format!("Could not read {entry_name} from the archive: {e}"))?;
        if !entry.is_file() {
            continue;
        }

        // The staging path is derived from the *destination*, which has already
        // been through `safe_relative_join` and `validate_placement` — never
        // from the archive entry name, which has not.
        let staged = safe_relative_join(staging, destination)?;
        if let Some(parent) = staged.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create a staging folder: {e}"))?;
        }
        let mut out = std::fs::File::create(&staged)
            .map_err(|e| format!("Could not write a staged file: {e}"))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("Could not extract {entry_name}: {e}"))?;

        placements.push(Placement {
            relative_path: destination.clone(),
            kind: ManagedKind::Shader,
            source: staged,
        });
    }

    if placements.is_empty() {
        return Err("Nothing in this archive could be extracted.".to_string());
    }
    Ok(placements)
}

// ── Commands ─────────────────────────────────────────────────────────────

/// What an install actually did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackInstallOutcome {
    pub pack_id: String,
    pub pack_name: String,
    /// The commit the archive was pinned to. Recorded because these
    /// repositories have no tags, so this is the only thing that identifies
    /// which bytes were installed.
    pub commit: String,
    pub files: Vec<String>,
}

/// Read-only: the shader library for this client folder.
///
/// `async` so the directory walk runs on a worker thread. A non-async
/// `#[tauri::command]` runs on the **main** thread and freezes the window for
/// its whole duration — the cause of the panel's 5-10 second stall on open.
#[tauri::command(async)]
pub fn list_shader_packs(client_dir: String) -> Result<ShaderLibrary, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    Ok(read_library(&location.client_dir))
}

/// Fetch a shader pack from its author's repository and install it.
///
/// The order is deliberate and each step gates the next:
///
/// 1. [`crate::client_write::begin_write`] — sandbox, approved root, filename
///    policy and the ESO-not-running check, returning a token the placement
///    path requires. It cannot be skipped, because `apply_placements` takes an
///    `&ApprovedRoot` rather than a path.
/// 2. Refuse `LinkOnly` packs *before* any network call. The licence answer and
///    the download answer are the same answer.
/// 3. Resolve the branch to a commit, and fetch that commit's archive.
/// 4. Plan every destination from the entry names alone, refusing the whole
///    archive on a traversal or an unexpected shape — before a single byte is
///    extracted.
/// 5. Extract into a staging `TempDir`, so a failure part-way leaves the client
///    folder untouched.
/// 6. Hand the batch to `apply_placements`, which re-asserts the running check
///    inside the manifest lock and rolls back on failure.
#[tauri::command]
pub async fn install_shader_pack(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::client_write::AllowedGameInstallPath>,
    client_dir: String,
    pack_id: String,
) -> Result<PackInstallOutcome, String> {
    let root = crate::client_write::begin_write(&state, &client_dir).await?;

    tokio::task::spawn_blocking(move || {
        let pack = pack_by_id(&pack_id)
            .ok_or_else(|| format!("{pack_id} is not a shader pack Kalpa knows about."))?;

        // Before the network, not after: a pack Kalpa may not redistribute is
        // one it must not fetch either, and finding that out after downloading
        // would mean it had already been fetched.
        let (owner, repo, branch) = match &pack.source {
            PackSource::Fetchable {
                owner,
                repo,
                branch,
            } => (*owner, *repo, *branch),
            PackSource::LinkOnly { reason, .. } => {
                return Err(format!("Kalpa cannot install {}. {reason}", pack.name));
            }
        };

        let commit = resolve_head_sha(owner, repo, branch)?;
        let spec = DownloadSpec {
            url: archive_url(owner, repo, &commit),
            // No pinned hash: these repositories publish none, and inventing an
            // expected digest for a moving branch would be a check that always
            // fails or always passes. The commit pin is what makes the download
            // reproducible; TLS plus the host allowlist is what makes it
            // authentic. Nothing here is executable — no DLL is written by this
            // path, which is why shader packs go first in the build order.
            expected_sha256: None,
            max_bytes: Some(MAX_PACK_BYTES),
        };
        let archive = download_to_temp(&spec, &|_| {})?;

        let entry_names = {
            let file = std::fs::File::open(archive.path())
                .map_err(|e| format!("Could not open the downloaded archive: {e}"))?;
            let mut zip = zip::ZipArchive::new(file)
                .map_err(|e| format!("The download was not a valid zip archive: {e}"))?;
            (0..zip.len())
                .map(|i| {
                    zip.by_index(i)
                        .map(|entry| entry.name().to_string())
                        .map_err(|e| format!("Could not read the archive: {e}"))
                })
                .collect::<Result<Vec<_>, _>>()?
        };

        let client_root = root.path().to_path_buf();
        let plan = plan_destinations(&entry_names, pack.layout, &client_root)?;

        let staging =
            tempfile::tempdir().map_err(|e| format!("Could not create a staging folder: {e}"))?;
        let placements = stage_entries(archive.path(), &plan, staging.path())?;

        let files: Vec<String> = placements
            .iter()
            .map(|placement| placement.relative_path.clone())
            .collect();
        crate::client_backup::apply_placements(&app, &root, placements)?;

        Ok(PackInstallOutcome {
            pack_id: pack.id.to_string(),
            pack_name: pack.name.to_string(),
            commit,
            files,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn every_pack_id_is_unique_and_resolvable() {
        let mut seen = BTreeSet::new();
        for pack in PACKS {
            assert!(seen.insert(pack.id), "duplicate pack id {}", pack.id);
            assert_eq!(pack_by_id(pack.id).map(|p| p.id), Some(pack.id));
        }
    }

    /// The provider table and this catalogue name the same techniques. A pack
    /// that claims to supply a provider Kalpa cannot recognise would install
    /// happily and leave the motion-vector slot still empty.
    #[test]
    fn advertised_techniques_are_real_identifiers() {
        for pack in PACKS {
            for technique in pack.techniques {
                assert!(
                    !technique.contains(' ') && !technique.contains(':'),
                    "{} advertises {technique:?}, which looks like a ui_label rather than a \
                     technique identifier — a preset never contains labels",
                    pack.name
                );
            }
        }
    }

    #[test]
    fn a_github_archive_root_is_stripped() {
        let entries = names(&[
            "LumeniteFX-mainline/",
            "LumeniteFX-mainline/README.md",
            "LumeniteFX-mainline/NOTICE",
            "LumeniteFX-mainline/Shaders/lumenite_Kernel.fx",
            "LumeniteFX-mainline/Shaders/include/lumenite_Helpers.fxh",
            "LumeniteFX-mainline/Textures/lumenite_bluenoise256.png",
        ]);
        let plan = plan_destinations(
            &entries,
            ArchiveLayout::ShadersAndTextures,
            Path::new("C:/game"),
        )
        .expect("plan");
        let destinations: Vec<&str> = plan.iter().map(|(_, d)| d.as_str()).collect();

        assert!(destinations.contains(&"reshade-shaders/Shaders/lumenite_Kernel.fx"));
        assert!(destinations.contains(&"reshade-shaders/Shaders/include/lumenite_Helpers.fxh"));
        assert!(destinations.contains(&"reshade-shaders/Textures/lumenite_bluenoise256.png"));
        // The MIT attributions have to travel with the shaders.
        assert!(destinations.contains(&"reshade-shaders/Shaders/NOTICE"));
        // A README is not a shader and not a notice.
        assert!(!destinations.iter().any(|d| d.ends_with("README.md")));
    }

    /// DRME has no `Shaders/` directory; its README says to copy every file
    /// straight in. Applying the common layout to it would install nothing.
    #[test]
    fn a_flat_archive_maps_its_root_into_shaders() {
        let entries = names(&[
            "ReshadeMotionEstimation-main/",
            "ReshadeMotionEstimation-main/MotionEstimation.fx",
            "ReshadeMotionEstimation-main/MotionVectors.fxh",
            "ReshadeMotionEstimation-main/README.md",
        ]);
        let plan = plan_destinations(&entries, ArchiveLayout::FlatRoot, Path::new("C:/game"))
            .expect("plan");
        let destinations: Vec<&str> = plan.iter().map(|(_, d)| d.as_str()).collect();

        assert_eq!(
            destinations,
            vec![
                "reshade-shaders/Shaders/MotionEstimation.fx",
                "reshade-shaders/Shaders/MotionVectors.fxh",
            ]
        );
    }

    /// The whole reason every entry goes through `safe_relative_join`.
    ///
    /// **Every path here ends in `.fx` on purpose.** An escape spelled
    /// `../../eso64.exe` is refused by the extension allowlist before the path
    /// guard is ever consulted, so a test using one passes just as happily with
    /// `safe_relative_join` deleted — it proves nothing about the guard it is
    /// named after. Giving each case an extension the allowlist accepts is what
    /// forces the containment check to be the thing doing the refusing.
    #[test]
    fn an_archive_cannot_escape_the_shader_tree() {
        for evil in [
            // Traversal out of the shader tree and into the client folder.
            "pack-main/Shaders/../../evil.fx",
            "pack-main/Shaders/../../../../../../Windows/System32/evil.fx",
            // Drive-relative on Windows: `Path::join` discards the base for a
            // segment starting with a drive letter and resolves against the
            // current directory instead. This is the trap that once escaped a
            // test tempdir and wrote into the repository.
            "pack-main/Shaders/C:evil.fx",
            "pack-main/C:/Windows/System32/evil.fx",
            // Absolute and UNC.
            "pack-main//absolute/evil.fx",
        ] {
            let entries = names(&["pack-main/", evil]);
            let result = plan_destinations(
                &entries,
                ArchiveLayout::ShadersAndTextures,
                Path::new("C:/game"),
            );
            assert!(
                result.is_err(),
                "{evil} should have been refused, got {result:?}"
            );
        }
    }

    /// The flat layout takes only the file name, so traversal cannot survive
    /// it — but a drive-relative *name* still has to be refused, and the file
    /// still has to be a shader.
    #[test]
    fn a_flat_archive_cannot_escape_either() {
        for evil in ["pack-main/../../evil.fx", "pack-main/C:evil.fx"] {
            let entries = names(&["pack-main/", evil]);
            let result = plan_destinations(&entries, ArchiveLayout::FlatRoot, Path::new("C:/game"));
            assert!(
                result.is_err(),
                "{evil} should have been refused, got {result:?}"
            );
        }
    }

    /// A shader archive containing a DLL must not be able to place it, whatever
    /// it is called or wherever it claims to go.
    #[test]
    fn only_shader_files_are_written() {
        let entries = names(&[
            "pack-main/",
            "pack-main/Shaders/real.fx",
            "pack-main/Shaders/evil.dll",
            "pack-main/Shaders/evil.addon64",
            "pack-main/Shaders/setup.exe",
            "pack-main/.github/workflows/ci.yml",
            "pack-main/.gitignore",
        ]);
        let plan = plan_destinations(
            &entries,
            ArchiveLayout::ShadersAndTextures,
            Path::new("C:/game"),
        )
        .expect("plan");
        let destinations: Vec<&str> = plan.iter().map(|(_, d)| d.as_str()).collect();
        assert_eq!(destinations, vec!["reshade-shaders/Shaders/real.fx"]);
    }

    /// An archive whose entries share no single root is not a GitHub source
    /// archive, and stripping "the root" from it would mean writing files from
    /// something other than what was asked for.
    #[test]
    fn an_archive_without_one_root_is_refused() {
        let entries = names(&["a/Shaders/one.fx", "b/Shaders/two.fx"]);
        let result = plan_destinations(
            &entries,
            ArchiveLayout::ShadersAndTextures,
            Path::new("C:/game"),
        );
        assert!(result.is_err(), "got {result:?}");
    }

    #[test]
    fn an_archive_with_no_shaders_is_refused() {
        let entries = names(&["pack-main/", "pack-main/README.md"]);
        let result = plan_destinations(
            &entries,
            ArchiveLayout::ShadersAndTextures,
            Path::new("C:/game"),
        );
        assert!(result.is_err(), "got {result:?}");
    }

    #[test]
    fn a_missing_shader_tree_reports_nothing_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let library = read_library(tmp.path());
        assert!(!library.shader_tree_present);
        assert!(library.packs.iter().all(|p| !p.installed));
        assert_eq!(library.packs.len(), PACKS.len());
    }

    /// Markers are matched by name anywhere in the tree: a hand-installed pack
    /// is still installed, and saying otherwise would make the panel wrong
    /// about shaders the user is demonstrably running.
    #[test]
    fn a_pack_is_found_wherever_it_was_put() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp
            .path()
            .join("reshade-shaders")
            .join("Shaders")
            .join("mine");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("lumenite_Kernel.fx"), "").unwrap();

        let library = read_library(tmp.path());
        let lumenite = library
            .packs
            .iter()
            .find(|p| p.pack.id == "lumenite")
            .expect("lumenite");
        assert!(lumenite.installed);
        assert_eq!(lumenite.found, vec!["lumenite_Kernel.fx"]);
        assert!(library
            .packs
            .iter()
            .find(|p| p.pack.id == "vort")
            .is_some_and(|p| !p.installed));
    }

    #[test]
    fn archive_urls_stay_on_an_allowed_host() {
        let url = archive_url("umar-afzaal", "LumeniteFX", &"a".repeat(40));
        assert!(crate::client_download::host_allowed(&url), "{url}");
    }

    /// A link-only pack must never grow a fetchable URL by accident: the
    /// licence answer and the download answer are the same answer.
    #[test]
    fn link_only_packs_have_no_fetch_route() {
        for pack in PACKS {
            if let PackSource::LinkOnly { url, reason } = &pack.source {
                assert!(url.starts_with("https://"), "{} url", pack.name);
                assert!(!reason.is_empty(), "{} needs a reason", pack.name);
            }
        }
    }
}
