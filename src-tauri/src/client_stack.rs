//! The graphics-mod **stack** in an ESO client directory.
//!
//! `client_health` answers "are these three DLLs current?". That was the wrong
//! question. A DLSS 5 Neural Rendering setup is not three files, it is a
//! pipeline of roughly eight layers that only works if every layer agrees with
//! the ones around it:
//!
//! ```text
//!   injector (ReShade dxgi.dll)
//!     -> addons  (renodx-dlss5.addon64, dlss5-feed.addon64)
//!       -> runtimes (nvngx_dlssnr.dll, nvngx_dlss.dll)
//!         -> shaders (MartysMods_LAUNCHPAD.fx, DLSS5_Feed.fx)
//!           -> preset (ordered technique list)
//!             -> tuning ([RenoDX.DLSS5] in ReShade.ini)
//! ```
//!
//! The interesting failures live *between* layers, which is exactly what a
//! per-file report cannot see. The technique order is the sharpest example:
//! Launchpad produces the motion vectors and normals the feed consumes, so a
//! preset listing `DLSS5_Feed` before `MartysMods_Launchpad` leaves the whole
//! thing running and silently wrong. Nothing about either file is defective.
//!
//! # Read-only
//!
//! Everything here opens files for reading. This module is the inventory; any
//! write goes through `client_write`/`client_backup` like every other client
//! directory change. Keeping it read-only is what lets it run on open without
//! any of the write gates.
//!
//! # Honest about what it cannot identify
//!
//! Real binaries in this stack carry no version resource at all
//! (`dlss5-feed-host64.exe`) or an empty one (`renodx-dlss5.addon64` has a
//! version but no `ProductName`). The types here model that: a missing name is
//! `None` and the UI is expected to say "unidentified" rather than invent one.
//! [`addon_display_name`] recovers what it can from ReShade's own
//! `OverlayCollapsed` mapping, which is the only place some addon names exist.

use crate::client_health::{file_version, version_string, HealthFinding, HealthLevel};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Where a file sits in the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StackRole {
    /// The proxy DLL the game loads: ReShade as `dxgi.dll` or `d3d11.dll`.
    Injector,
    /// `nvngx_dlssnr.dll` — the Neural Rendering runtime.
    NeuralRendering,
    /// `nvngx_dlss.dll` — DLSS super resolution / DLAA.
    SuperSampling,
    /// `nvngx_dlssg.dll` — frame generation.
    FrameGeneration,
    /// `d3dcompiler_47.dll`.
    ShaderCompiler,
    /// A ReShade addon binary.
    Addon,
    /// A helper the stack needs but which is not itself loaded by the game,
    /// e.g. `dlss5-feed-host64.exe` and its `.cfg`.
    Companion,
}

/// One file in the stack, with whatever its version resource would admit to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StackItem {
    pub role: StackRole,
    /// File name as found on disk.
    pub file_name: String,
    /// Best human name available: `ProductName`, else ReShade's own addon
    /// mapping, else `None`. Never invented from the file name — a UI that
    /// wants a fallback should make that choice visibly.
    pub display_name: Option<String>,
    /// Four-part file version, `None` when there is no version resource.
    pub version: Option<String>,
    pub company: Option<String>,
    pub description: Option<String>,
    pub size_bytes: u64,
}

/// A pre-existing copy of a file the user (or Kalpa) displaced.
///
/// The primary user's own convention is `nvngx_dlss.dll.disabled-bak` and
/// `d3dcompiler_47.dll.eso-orig-bak`. Recognising these matters for adoption:
/// they are the displaced originals, and a stack Kalpa adopts should point at
/// them rather than declaring the original lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreservedOriginal {
    pub file_name: String,
    /// The live file this appears to be a backup of, when that file exists.
    pub backs_up: Option<String>,
    pub version: Option<String>,
    pub size_bytes: u64,
}

/// A file Kalpa has parked so the stack does not load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParkedFile {
    /// Name on disk, ending in [`PARKED_SUFFIX`].
    pub file_name: String,
    /// The live name it goes back to.
    pub restores: String,
    pub size_bytes: u64,
    /// True when something already occupies the name it would restore, which
    /// means re-enabling has to displace it rather than just rename.
    pub target_present: bool,
}

/// One technique in a preset's ordered list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Technique {
    /// e.g. `MartysMods_Launchpad`.
    pub name: String,
    /// Source file it comes from, e.g. `MartysMods_LAUNCHPAD.fx`.
    pub source: String,
    /// Whether that source file was found under the shader search path.
    pub source_present: bool,
}

/// Which motion-vector provider `DLSS5_Feed.fx` is configured to read.
///
/// The effect chooses on two levels, and both have to be read to get the
/// answer right. `DLSS5_MV_SOURCE` is a *preprocessor* definition: at `1` the
/// LaunchPad code path is not compiled in at all, so the `MV_PROVIDER`
/// dropdown does not exist and the shared-texture convention is the only
/// option. At `0` (the default) `MV_PROVIDER` picks between them at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MvProviderKind {
    /// Anything writing the shared `texMotionVectors` texture — qUINT,
    /// `dh_uber_motion`, DRME, ReshadeMotionEstimation. There is no name to
    /// match on, so which effect it is can only be answered by looking at which
    /// enabled one declares that texture.
    SharedTexture,
    /// iMMERSE LaunchPad (MartysMods).
    Launchpad,
    /// VORT.
    Vort,
    /// LumeniteFX Kernel — upstream's current recommendation.
    LumeniteKernel,
    /// LumeniteFX QuantMotion.
    LumeniteQuantMotion,
}

impl MvProviderKind {
    /// The technique name upstream says to enable for this provider, plus the
    /// spellings older presets use for the same effect.
    ///
    /// Matching is by name because that is what upstream documents per provider.
    /// [`MvProviderKind::SharedTexture`] has no entry: it is a convention rather
    /// than a named effect, and is resolved by reading the shader sources.
    fn technique_names(self) -> &'static [&'static str] {
        match self {
            MvProviderKind::SharedTexture => &[],
            // `Launchpad` is what current DLSS5-Feeder documents;
            // `MartysMods_Launchpad` is what iMMERSE presets of the 0.4.x era
            // actually contain, including the primary user's.
            MvProviderKind::Launchpad => &["Launchpad", "MartysMods_Launchpad"],
            MvProviderKind::Vort => &["vort_Motion"],
            MvProviderKind::LumeniteKernel => &["LUMENITE: Kernel 2.0"],
            MvProviderKind::LumeniteQuantMotion => &["LUMENITE: QuantMotion"],
        }
    }

    /// Upstream's own name for the provider, for use in a sentence.
    fn label(self) -> &'static str {
        match self {
            MvProviderKind::SharedTexture => "the shared texMotionVectors texture",
            MvProviderKind::Launchpad => "iMMERSE LaunchPad",
            MvProviderKind::Vort => "VORT",
            MvProviderKind::LumeniteKernel => "LumeniteFX Kernel",
            MvProviderKind::LumeniteQuantMotion => "LumeniteFX QuantMotion",
        }
    }

    /// `DLSS5_MV_PROVIDER`'s value, in current DLSS5-Feeder.
    fn from_mv_provider_definition(value: i64) -> Option<Self> {
        match value {
            0 => Some(MvProviderKind::SharedTexture),
            1 => Some(MvProviderKind::Launchpad),
            2 => Some(MvProviderKind::Vort),
            3 => Some(MvProviderKind::LumeniteKernel),
            4 => Some(MvProviderKind::LumeniteQuantMotion),
            _ => None,
        }
    }
}

/// The resolved provider, plus the enabled technique that actually supplies
/// the vectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MvProvider {
    pub kind: MvProviderKind,
    /// The enabled technique producing the vectors, when one was found. `None`
    /// means nothing in the preset feeds the runtime — the feed reads zeros.
    pub technique: Option<String>,
}

/// The active preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetInfo {
    /// As written in `ReShade.ini`, e.g. `.\ReShadePreset.ini`.
    pub path: String,
    pub exists: bool,
    /// Enabled techniques, **in the order ReShade will run them**. Order is
    /// load-bearing; see the module doc.
    pub techniques: Vec<Technique>,
    /// Everything the preset knows about, enabled or not.
    pub available: Vec<String>,
    /// Resolved motion-vector provider. `None` when the preset does not enable
    /// the feed technique at all, so the question does not arise.
    pub mv_provider: Option<MvProvider>,
}

/// One tunable from the `[RenoDX.DLSS5]` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TuningValue {
    pub key: String,
    pub value: String,
}

/// The shader tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShaderTree {
    pub present: bool,
    pub effect_count: usize,
    pub texture_count: usize,
    /// `EffectSearchPaths` from `ReShade.ini`, so a mismatch between where
    /// shaders are and where ReShade looks is visible.
    pub effect_search_paths: Option<String>,
}

/// Everything Kalpa can see about one client directory's stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientStack {
    pub client_dir: String,
    /// Ordered by [`StackRole`], so the UI can render a pipeline without
    /// re-deriving the order.
    pub items: Vec<StackItem>,
    pub preserved_originals: Vec<PreservedOriginal>,
    /// Files Kalpa parked when the stack was switched off.
    pub parked: Vec<ParkedFile>,
    /// True when the injector itself is parked, i.e. ESO is back to stock and
    /// loads none of this.
    pub is_disabled: bool,
    pub shaders: ShaderTree,
    pub preset: Option<PresetInfo>,
    pub tuning: Vec<TuningValue>,
    /// Addon file names ReShade has been told not to load.
    pub disabled_addons: Vec<String>,
    /// True when nothing recognisable is installed — the common case, and the
    /// signal for the UI to stay quiet rather than render an empty pipeline.
    pub is_empty: bool,
    pub findings: Vec<HealthFinding>,
}

// ── Known names ──────────────────────────────────────────────────────────

const INJECTOR_NAMES: [&str; 2] = ["dxgi.dll", "d3d11.dll"];
const ADDON_EXTENSIONS: [&str; 3] = ["addon64", "addon32", "addon"];
/// Suffixes that mark a displaced original rather than a live file. The first
/// two are the primary user's own hand-rolled convention.
const BACKUP_SUFFIXES: [&str; 4] = [".disabled-bak", ".eso-orig-bak", ".bak", ".orig"];

/// The suffix Kalpa parks a live file under when the stack is switched off.
///
/// Deliberately **not** one of [`BACKUP_SUFFIXES`]. Those are the user's own
/// names for their own originals: in a real install `nvngx_dlss.dll.disabled-bak`
/// *is* the stock DLL the fallback depends on, so parking a live file under that
/// name would overwrite the one thing disable exists to restore. Kalpa only ever
/// writes this suffix, and only ever removes files carrying it.
pub const PARKED_SUFFIX: &str = ".kalpa-off";

/// Files that are companions to a known addon: not loaded by the game, but the
/// stack does not work without them.
const COMPANION_NAMES: [&str; 3] = [
    "dlss5-feed-host64.exe",
    "dlss5-feed.cfg",
    "dlss5-feed-host32.exe",
];

/// The technique that consumes the motion vectors, and the effect file it
/// lives in.
const FEED_TECHNIQUE: &str = "dlss5_feed";
const FEED_SOURCE: &str = "dlss5_feed.fx";

/// LaunchPad's effect file. Its *technique* names live in
/// [`MvProviderKind::technique_names`] alongside every other provider's; the
/// source is matched as well because a preset can rename a technique but not
/// the file it comes from.
const LAUNCHPAD_SOURCE: &str = "martysmods_launchpad.fx";

/// The shared texture every non-LaunchPad provider writes. `DLSS5_Feed.fx`
/// declares it too, which is why the feed's own source is excluded when
/// searching for who supplies it.
const SHARED_MV_TEXTURE: &str = "texmotionvectors";

// ── INI parsing ──────────────────────────────────────────────────────────

/// A minimal INI reader: `section -> key -> value`, comparisons
/// case-insensitive on both.
///
/// ReShade writes plain `key=value` under `[SECTION]` headers with no quoting
/// and no escapes, and values routinely contain commas, backslashes and `=`
/// (the docking blobs), so only the *first* `=` separates. Lines before any
/// header land in the empty-string section, which is how `dlss5-feed.cfg`
/// (headerless) is read by the same code.
fn parse_ini(contents: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section = String::new();

    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        out.entry(section.clone())
            .or_default()
            .insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    out
}

fn ini_get<'a>(
    ini: &'a BTreeMap<String, BTreeMap<String, String>>,
    section: &str,
    key: &str,
) -> Option<&'a str> {
    ini.get(&section.to_ascii_lowercase())?
        .get(&key.to_ascii_lowercase())
        .map(|s| s.as_str())
}

/// Split a ReShade `name@source.fx` pair.
fn split_technique(entry: &str) -> Option<(String, String)> {
    let (name, source) = entry.split_once('@')?;
    let name = name.trim();
    let source = source.trim();
    if name.is_empty() || source.is_empty() {
        return None;
    }
    Some((name.to_string(), source.to_string()))
}

/// Recover an addon's human name from ReShade's `OverlayCollapsed`, which is
/// `Display Name@file.addon64` pairs and, for some addons, the only place a
/// display name exists at all.
fn addon_display_name(overlay_collapsed: Option<&str>, file_name: &str) -> Option<String> {
    let entries = overlay_collapsed?;
    for entry in entries.split(',') {
        if let Some((name, source)) = split_technique(entry) {
            if source.eq_ignore_ascii_case(file_name) {
                return Some(name);
            }
        }
    }
    None
}

// ── Probing ──────────────────────────────────────────────────────────────

fn size_of(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn probe(dir: &Path, file_name: &str, role: StackRole) -> Option<StackItem> {
    let path = dir.join(file_name);
    if !path.is_file() {
        return None;
    }
    Some(StackItem {
        role,
        file_name: file_name.to_string(),
        display_name: version_string(&path, "ProductName"),
        version: file_version(&path),
        company: version_string(&path, "CompanyName"),
        description: version_string(&path, "FileDescription"),
        size_bytes: size_of(&path),
    })
}

/// Is `name` one of the backup suffixes, and if so what does it back up?
fn backup_target(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    for suffix in BACKUP_SUFFIXES {
        if let Some(stem) = lower.strip_suffix(suffix) {
            return Some(stem.to_string());
        }
    }
    None
}

/// Read the whole stack for one client directory.
pub fn inspect_stack(client_dir: &Path) -> ClientStack {
    let reshade_ini = std::fs::read_to_string(client_dir.join("ReShade.ini")).unwrap_or_default();
    let ini = parse_ini(&reshade_ini);
    let overlay_collapsed = ini_get(&ini, "ADDON", "OverlayCollapsed");

    let mut items: Vec<StackItem> = Vec::new();

    for name in INJECTOR_NAMES {
        if let Some(item) = probe(client_dir, name, StackRole::Injector) {
            items.push(item);
        }
    }
    for (name, role) in [
        ("nvngx_dlssnr.dll", StackRole::NeuralRendering),
        ("nvngx_dlss.dll", StackRole::SuperSampling),
        ("nvngx_dlssg.dll", StackRole::FrameGeneration),
        ("d3dcompiler_47.dll", StackRole::ShaderCompiler),
    ] {
        if let Some(item) = probe(client_dir, name, role) {
            items.push(item);
        }
    }

    // Addons and preserved originals both come from one directory walk.
    let mut preserved_originals: Vec<PreservedOriginal> = Vec::new();
    let mut parked: Vec<ParkedFile> = Vec::new();
    let mut live_names: Vec<String> = Vec::new();
    let mut addon_files: Vec<String> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(client_dir) {
        for entry in entries.flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            live_names.push(lower.clone());

            if let Some(restores) = lower.strip_suffix(PARKED_SUFFIX) {
                parked.push(ParkedFile {
                    file_name: name.clone(),
                    restores: restores.to_string(),
                    size_bytes: size_of(&entry.path()),
                    target_present: false,
                });
                continue;
            }
            if let Some(target) = backup_target(&lower) {
                preserved_originals.push(PreservedOriginal {
                    file_name: name.clone(),
                    backs_up: Some(target),
                    version: file_version(&entry.path()),
                    size_bytes: size_of(&entry.path()),
                });
                continue;
            }
            if let Some(ext) = lower.rsplit('.').next() {
                if ADDON_EXTENSIONS.contains(&ext) {
                    addon_files.push(name);
                }
            }
        }
    }

    addon_files.sort();
    for name in &addon_files {
        let path = client_dir.join(name);
        items.push(StackItem {
            role: StackRole::Addon,
            file_name: name.clone(),
            // The version resource first, then ReShade's own mapping. Some
            // addons genuinely have neither.
            display_name: version_string(&path, "ProductName")
                .or_else(|| addon_display_name(overlay_collapsed, name)),
            version: file_version(&path),
            company: version_string(&path, "CompanyName"),
            description: version_string(&path, "FileDescription"),
            size_bytes: size_of(&path),
        });
    }

    for name in COMPANION_NAMES {
        if let Some(item) = probe(client_dir, name, StackRole::Companion) {
            items.push(item);
        }
    }

    // A backup whose live counterpart is gone is worth showing differently, so
    // resolve `backs_up` against what is actually present.
    for original in &mut preserved_originals {
        if let Some(target) = &original.backs_up {
            if !live_names.contains(target) {
                original.backs_up = None;
            }
        }
    }
    preserved_originals.sort_by(|a, b| a.file_name.cmp(&b.file_name));

    for file in &mut parked {
        file.target_present = live_names.contains(&file.restores);
    }
    parked.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    let is_disabled = parked
        .iter()
        .any(|file| INJECTOR_NAMES.contains(&file.restores.as_str()));

    let shaders = read_shader_tree(client_dir, &ini);
    let preset = read_preset(client_dir, &ini);

    let tuning: Vec<TuningValue> = ini
        .get("renodx.dlss5")
        .map(|block| {
            block
                .iter()
                .map(|(key, value)| TuningValue {
                    key: key.clone(),
                    value: value.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let disabled_addons: Vec<String> = ini_get(&ini, "ADDON", "DisabledAddons")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    items.sort_by(|a, b| a.role.cmp(&b.role).then(a.file_name.cmp(&b.file_name)));

    let is_empty =
        items.is_empty() && preserved_originals.is_empty() && parked.is_empty() && !shaders.present;

    let mut stack = ClientStack {
        client_dir: client_dir.to_string_lossy().to_string(),
        items,
        preserved_originals,
        parked,
        is_disabled,
        shaders,
        preset,
        tuning,
        disabled_addons,
        is_empty,
        findings: Vec::new(),
    };
    stack.findings = build_findings(&stack);
    stack
}

fn read_shader_tree(
    client_dir: &Path,
    ini: &BTreeMap<String, BTreeMap<String, String>>,
) -> ShaderTree {
    let root = client_dir.join("reshade-shaders");
    let effect_count = count_files(&root.join("Shaders"));
    let texture_count = count_files(&root.join("Textures"));
    ShaderTree {
        present: root.is_dir(),
        effect_count,
        texture_count,
        effect_search_paths: ini_get(ini, "GENERAL", "EffectSearchPaths").map(str::to_string),
    }
}

/// Count files one level deep plus immediate subdirectories. Shader packs nest
/// (`Shaders/MartysMods/...`), and a full recursive walk of a game directory is
/// not worth the I/O for a number shown in a summary line.
fn count_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            count += 1;
        } else if path.is_dir() {
            count += std::fs::read_dir(&path)
                .map(|inner| inner.flatten().filter(|e| e.path().is_file()).count())
                .unwrap_or(0);
        }
    }
    count
}

fn read_preset(
    client_dir: &Path,
    ini: &BTreeMap<String, BTreeMap<String, String>>,
) -> Option<PresetInfo> {
    let raw = ini_get(ini, "GENERAL", "PresetPath")?;
    let relative = raw.trim().trim_matches('"');
    if relative.is_empty() {
        return None;
    }
    // Presets are written relative to the client dir as `.\Name.ini`. The value
    // is text out of someone else's config file, so it goes through the
    // containment check rather than a bare `join`: `..\..\elsewhere.ini` and
    // `C:elsewhere.ini` both escape a plain join, and this module would then
    // read and report a file outside the game folder as if it were the preset.
    let path = crate::client_write::safe_relative_join(
        client_dir,
        relative.trim_start_matches(r".\").trim_start_matches("./"),
    )
    .ok();
    let exists = path.as_ref().is_some_and(|p| p.is_file());
    let contents = match &path {
        Some(path) if exists => std::fs::read_to_string(path).unwrap_or_default(),
        _ => String::new(),
    };
    let preset_ini = parse_ini(&contents);

    let shader_dir = client_dir.join("reshade-shaders").join("Shaders");
    let technique_entries = ini_get(&preset_ini, "", "Techniques").unwrap_or_default();
    let techniques: Vec<Technique> = technique_entries
        .split(',')
        .filter_map(split_technique)
        .map(|(name, source)| Technique {
            source_present: shader_source_exists(&shader_dir, &source),
            name,
            source,
        })
        .collect();

    let available = ini_get(&preset_ini, "", "TechniqueSorting")
        .unwrap_or_default()
        .split(',')
        .filter_map(split_technique)
        .map(|(name, _)| name)
        .collect();

    let mv_provider = resolve_mv_provider(&preset_ini, ini, &shader_dir, &techniques);

    Some(PresetInfo {
        path: raw.to_string(),
        exists,
        techniques,
        available,
        mv_provider,
    })
}

/// Shader packs nest one level (`Shaders/MartysMods/...`), so look in the root
/// and in immediate subdirectories.
fn find_shader_source(shader_dir: &Path, source: &str) -> Option<std::path::PathBuf> {
    let direct = shader_dir.join(source);
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(shader_dir).ok()?.flatten().find_map(|e| {
        let nested = e.path().join(source);
        (e.path().is_dir() && nested.is_file()).then_some(nested)
    })
}

fn shader_source_exists(shader_dir: &Path, source: &str) -> bool {
    find_shader_source(shader_dir, source).is_some()
}

/// Read one entry out of a ReShade `PreprocessorDefinitions=A=1,B=2` list.
fn preprocessor_definition<'a>(
    ini: &'a BTreeMap<String, BTreeMap<String, String>>,
    section: &str,
    name: &str,
) -> Option<&'a str> {
    let list = ini_get(ini, section, "PreprocessorDefinitions")?;
    list.split(',').find_map(|entry| {
        let (key, value) = entry.split_once('=')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

/// Work out who feeds `DLSS5_Feed`.
///
/// Kalpa used to assume LaunchPad. It is only ever one option, and which
/// options exist depends on which generation of `DLSS5_Feed.fx` is installed —
/// so both are read, newest first:
///
/// * **Current DLSS5-Feeder** uses one preprocessor definition,
///   `DLSS5_MV_PROVIDER`, taking `0`–`4`: shared `texMotionVectors`, iMMERSE
///   LaunchPad, VORT, LumeniteFX Kernel (upstream's recommendation), LumeniteFX
///   QuantMotion. Each names the technique to enable, so the provider is matched
///   by name.
/// * **0.4.x**, which is what the primary user runs, uses two levels instead:
///   `DLSS5_MV_SOURCE` decides at compile time whether the LaunchPad path is
///   linked in at all — its headers cannot coexist with `ReShade.fxh` — and the
///   runtime `MV_PROVIDER` uniform picks between LaunchPad (`0`) and the shared
///   texture (`1`).
///
/// Assuming LaunchPad did not produce a wrong answer, it produced *no* answer:
/// the ordering check looked for a technique that was not in the preset and
/// silently never ran. Reading only the 0.4.x scheme would do the same thing
/// again to anyone on a current build.
fn resolve_mv_provider(
    preset_ini: &BTreeMap<String, BTreeMap<String, String>>,
    reshade_ini: &BTreeMap<String, BTreeMap<String, String>>,
    shader_dir: &Path,
    techniques: &[Technique],
) -> Option<MvProvider> {
    if !techniques.iter().any(is_feed_technique) {
        return None;
    }

    // Per-effect definitions win over the global list; absent both, the effect's
    // own `#ifndef` default applies.
    let definition = |name: &str| {
        preprocessor_definition(preset_ini, FEED_SOURCE, name)
            .or_else(|| preprocessor_definition(reshade_ini, "GENERAL", name))
    };

    let kind = match definition("DLSS5_MV_PROVIDER") {
        // Current scheme. An unrecognised value is a build newer than this
        // table, and guessing which provider it means is exactly the mistake
        // this function exists to stop making — fall back to the convention
        // that needs no name.
        Some(value) => value
            .trim()
            .parse::<i64>()
            .ok()
            .and_then(MvProviderKind::from_mv_provider_definition)
            .unwrap_or(MvProviderKind::SharedTexture),
        None => {
            let launchpad_compiled_in = definition("DLSS5_MV_SOURCE").unwrap_or("0").trim() == "0";
            let selected = ini_get(preset_ini, FEED_SOURCE, "MV_PROVIDER")
                .and_then(|v| v.trim().parse::<i64>().ok())
                .unwrap_or(0);
            if launchpad_compiled_in && selected == 0 {
                MvProviderKind::Launchpad
            } else {
                MvProviderKind::SharedTexture
            }
        }
    };

    let names = kind.technique_names();
    let technique = techniques
        .iter()
        .find(|t| {
            if names.is_empty() {
                // A convention, not a named effect: ask the shader files
                // themselves which enabled one deals in the shared texture. The
                // feed declares it too — as the consumer — so its own source is
                // excluded.
                !t.source.eq_ignore_ascii_case(FEED_SOURCE)
                    && source_mentions_shared_mv(shader_dir, &t.source)
            } else {
                names.iter().any(|name| t.name.eq_ignore_ascii_case(name))
                    || (kind == MvProviderKind::Launchpad
                        && t.source.eq_ignore_ascii_case(LAUNCHPAD_SOURCE))
            }
        })
        .map(|t| t.name.clone());

    Some(MvProvider { kind, technique })
}

/// Does this effect file deal in `texMotionVectors` at all?
///
/// A declaration is not proof the effect *writes* the texture — a second
/// consumer would match too. It is the strongest signal available without
/// parsing HLSL, and naming the wrong enabled effect is a far smaller error
/// than the check not running.
fn source_mentions_shared_mv(shader_dir: &Path, source: &str) -> bool {
    let Some(path) = find_shader_source(shader_dir, source) else {
        return false;
    };
    std::fs::read_to_string(&path)
        .map(|text| text.to_ascii_lowercase().contains(SHARED_MV_TEXTURE))
        .unwrap_or(false)
}

fn is_feed_technique(t: &Technique) -> bool {
    t.name.eq_ignore_ascii_case(FEED_TECHNIQUE)
}

// ── Findings ─────────────────────────────────────────────────────────────

fn finding(id: &str, level: HealthLevel, title: &str, detail: String) -> HealthFinding {
    HealthFinding {
        id: id.to_string(),
        level,
        title: title.to_string(),
        detail,
        guide_url: None,
    }
}

fn has_role(stack: &ClientStack, role: StackRole) -> bool {
    stack.items.iter().any(|item| item.role == role)
}

fn has_file(stack: &ClientStack, name: &str) -> bool {
    stack
        .items
        .iter()
        .any(|item| item.file_name.eq_ignore_ascii_case(name))
}

/// Cross-layer checks. Every one of these is a disagreement *between* layers —
/// the kind a per-file report cannot see, and the reason this module exists.
pub fn build_findings(stack: &ClientStack) -> Vec<HealthFinding> {
    let mut out = Vec::new();
    if stack.is_empty {
        return out;
    }

    // A switched-off stack loads nothing, so every cross-layer check below
    // would be describing a configuration that is not running. Reporting a
    // missing injector — which disable deliberately created — or a DLSS that
    // has "reverted" — which is disable putting the stock file back — would be
    // Kalpa alarming the user about its own work. Say the one true thing and
    // stop; the rest becomes relevant again on re-enable.
    if stack.is_disabled {
        out.push(finding(
            "stack-disabled",
            HealthLevel::Info,
            "This stack is switched off",
            format!(
                "Kalpa has parked {} so ESO loads none of it, and put the game's own files \
                 back. Re-enable to reverse that.",
                stack
                    .parked
                    .iter()
                    .map(|file| file.restores.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
        return out;
    }

    let nr_addon = has_file(stack, "renodx-dlss5.addon64");
    let feed_addon = has_file(stack, "dlss5-feed.addon64");

    if !has_role(stack, StackRole::Injector) && (nr_addon || feed_addon) {
        out.push(finding(
            "stack-no-injector",
            HealthLevel::Danger,
            "ReShade addons are present but nothing loads them",
            "The addon binaries are in the client folder, but there is no dxgi.dll or \
             d3d11.dll for the game to load, so ReShade never starts and none of them run."
                .to_string(),
        ));
    }

    if nr_addon && !has_role(stack, StackRole::NeuralRendering) {
        out.push(finding(
            "stack-nr-runtime-missing",
            HealthLevel::Danger,
            "Neural Rendering addon has no runtime",
            "renodx-dlss5.addon64 is installed, but nvngx_dlssnr.dll is not in the client \
             folder. The addon will load and Neural Rendering will not work."
                .to_string(),
        ));
    }

    if feed_addon && !has_file(stack, "dlss5-feed-host64.exe") {
        out.push(finding(
            "stack-feed-host-missing",
            HealthLevel::Warning,
            "DLSS 5 Feed is missing its host process",
            "dlss5-feed.addon64 is installed but dlss5-feed-host64.exe is not next to it. \
             The feed cannot start without its host."
                .to_string(),
        ));
    }

    for name in &stack.disabled_addons {
        out.push(finding(
            "stack-addon-disabled",
            HealthLevel::Warning,
            "An addon is switched off in ReShade",
            format!(
                "{name} is listed in DisabledAddons in ReShade.ini, so ReShade will not load \
                 it even though the file is present."
            ),
        ));
    }

    if let Some(preset) = &stack.preset {
        if !preset.exists {
            out.push(finding(
                "stack-preset-missing",
                HealthLevel::Danger,
                "The active preset file does not exist",
                format!(
                    "ReShade.ini points PresetPath at {}, but there is no file there. No \
                     effects will run.",
                    preset.path
                ),
            ));
        }

        for technique in &preset.techniques {
            if !technique.source_present {
                out.push(finding(
                    "stack-technique-source-missing",
                    HealthLevel::Danger,
                    "An enabled effect has no shader file",
                    format!(
                        "The preset enables {} from {}, but that file is not in the shader \
                         tree. ReShade will fail to compile the preset.",
                        technique.name, technique.source
                    ),
                ));
            }
        }

        // The ordering rule. This is the failure worth catching: everything
        // loads, nothing errors, and the output is quietly wrong. Which
        // technique has to be above the feed depends on MV_PROVIDER, so the
        // check is driven by the resolved provider rather than by a name.
        let position = |needle: &str| {
            preset
                .techniques
                .iter()
                .position(|t| t.name.eq_ignore_ascii_case(needle))
        };
        match (position(FEED_TECHNIQUE), preset.mv_provider.as_ref()) {
            (Some(feed), Some(provider)) => match &provider.technique {
                Some(name) if position(name).is_some_and(|at| at > feed) => {
                    out.push(finding(
                        "stack-technique-order",
                        HealthLevel::Danger,
                        "Effects are in the wrong order",
                        format!(
                            "DLSS5_Feed runs before {name} in this preset. {name} produces the \
                             motion vectors the feed consumes, so with this order the feed reads \
                             last frame's data. Nothing errors — the image is just quietly wrong."
                        ),
                    ));
                }
                Some(_) => {}
                None => out.push(finding(
                    "stack-mv-provider-missing",
                    HealthLevel::Danger,
                    "Nothing is producing motion vectors",
                    match provider.kind.technique_names().first() {
                        Some(technique) => format!(
                            "DLSS5_Feed is set to read motion vectors from {}, but this preset \
                             does not enable its technique ({technique}). The feed reads zeros, \
                             so DLSS sees a still image.",
                            provider.kind.label()
                        ),
                        None => "DLSS5_Feed is set to read motion vectors from the shared \
                             texMotionVectors texture, but no enabled effect in this preset \
                             writes it. The feed reads zeros, so DLSS sees a still image."
                            .to_string(),
                    },
                )),
            },
            _ if feed_addon => out.push(finding(
                "stack-feed-technique-off",
                HealthLevel::Warning,
                "DLSS 5 Feed is installed but not enabled",
                "The feed addon is present, but the active preset does not enable the \
                 DLSS5_Feed technique, so nothing feeds the runtime."
                    .to_string(),
            )),
            _ => {}
        }
    }

    if stack.shaders.present {
        if let Some(paths) = &stack.shaders.effect_search_paths {
            if !paths.to_ascii_lowercase().contains("reshade-shaders") {
                out.push(finding(
                    "stack-search-path-mismatch",
                    HealthLevel::Warning,
                    "ReShade is not looking where the shaders are",
                    format!(
                        "There is a reshade-shaders folder, but EffectSearchPaths is set to \
                         {paths}, which does not include it."
                    ),
                ));
            }
        }
    }

    // Drift: the ZOS patcher rewrites the client folder on every game update,
    // which puts ESO's own 2.2.16 DLSS back over a user's swap.
    if let Some(dlss) = stack
        .items
        .iter()
        .find(|item| item.role == StackRole::SuperSampling)
    {
        let reverted = dlss
            .version
            .as_deref()
            .and_then(|v| v.split('.').next())
            .and_then(|major| major.parse::<u32>().ok())
            .is_some_and(|major| major < 3);
        let has_swapped_backup = stack.preserved_originals.iter().any(|original| {
            original
                .file_name
                .to_ascii_lowercase()
                .contains("nvngx_dlss")
        });
        if reverted && has_swapped_backup {
            out.push(finding(
                "stack-dlss-reverted",
                HealthLevel::Warning,
                "DLSS has reverted to the version ESO ships",
                "nvngx_dlss.dll is back to ESO's bundled build, but a backup of a newer one \
                 is still here. A game update almost certainly overwrote your swap."
                    .to_string(),
            ));
        }
    }

    out
}

// ── Command ──────────────────────────────────────────────────────────────

/// Read-only inventory of the mod stack in a client directory.
#[tauri::command]
pub fn inspect_client_stack(client_dir: String) -> Result<ClientStack, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    Ok(inspect_stack(&location.client_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).expect("write fixture");
    }

    /// The primary user's real ReShade.ini, trimmed to the parts that matter.
    const REAL_RESHADE_INI: &str = "\
[ADDON]
DisabledAddons=
OverlayCollapsed=DLSS 5 Neural Rendering@renodx-dlss5.addon64,DLSS 5 Feed 0.4.0@dlss5-feed.addon64

[GENERAL]
EffectSearchPaths=.\\reshade-shaders\\Shaders\\**
PresetPath=.\\ReShadePreset.ini

[RenoDX.DLSS5]
NRLocalStructure=1.4
NRLocalTone=0.33
NRStyle=1
NRToggleKey=56
";

    const REAL_PRESET: &str = "\
Techniques=MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,DLSS5_Feed@DLSS5_Feed.fx
TechniqueSorting=Daltonize@Daltonize.fx,MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,DLSS5_Feed@DLSS5_Feed.fx

[DLSS5_Feed.fx]
DEBUG_VIEW=1
";

    /// Build the primary user's stack shape: injector, NR runtime, both
    /// addons, host, shaders and a correctly-ordered preset.
    fn healthy_stack(dir: &Path) {
        write(dir, "eso64.exe", "");
        write(dir, "dxgi.dll", "");
        write(dir, "nvngx_dlssnr.dll", "");
        write(dir, "nvngx_dlss.dll", "");
        write(dir, "renodx-dlss5.addon64", "");
        write(dir, "dlss5-feed.addon64", "");
        write(dir, "dlss5-feed-host64.exe", "");
        write(dir, "ReShade.ini", REAL_RESHADE_INI);
        write(dir, "ReShadePreset.ini", REAL_PRESET);
        let shaders = dir.join("reshade-shaders").join("Shaders");
        std::fs::create_dir_all(&shaders).expect("mkdir shaders");
        std::fs::write(shaders.join("MartysMods_LAUNCHPAD.fx"), "").unwrap();
        std::fs::write(shaders.join("DLSS5_Feed.fx"), "").unwrap();
    }

    fn ids(stack: &ClientStack) -> Vec<&str> {
        stack.findings.iter().map(|f| f.id.as_str()).collect()
    }

    #[test]
    fn a_healthy_stack_reports_no_problems() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        assert!(!stack.is_empty);
        assert!(
            stack.findings.is_empty(),
            "a correct stack should be quiet, got {:?}",
            ids(&stack)
        );
    }

    #[test]
    fn an_empty_client_folder_is_empty_and_silent() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "eso64.exe", "");
        let stack = inspect_stack(tmp.path());

        assert!(stack.is_empty);
        assert!(stack.findings.is_empty());
        assert!(stack.items.is_empty());
    }

    /// The headline case: everything present, nothing errors, output silently
    /// wrong because the feed reads data Launchpad has not produced yet.
    #[test]
    fn feed_before_launchpad_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=DLSS5_Feed@DLSS5_Feed.fx,MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx\n",
        );
        let stack = inspect_stack(tmp.path());

        assert!(
            ids(&stack).contains(&"stack-technique-order"),
            "expected the ordering finding, got {:?}",
            ids(&stack)
        );
    }

    /// Write a `texMotionVectors` provider effect into the shader tree and
    /// point the preset at it with `MV_PROVIDER=1`.
    fn shared_texture_preset(dir: &Path, techniques: &str) {
        std::fs::write(
            dir.join("reshade-shaders")
                .join("Shaders")
                .join("MotionEstimation.fx"),
            "texture texMotionVectors { Format = RG16F; };\n",
        )
        .unwrap();
        write(
            dir,
            "ReShadePreset.ini",
            &format!("Techniques={techniques}\n\n[DLSS5_Feed.fx]\nMV_PROVIDER=1\n"),
        );
    }

    /// The bug this fixes: with a non-LaunchPad provider the ordering check
    /// used to look for a technique that is not in the preset, so it never ran.
    #[test]
    fn a_shared_texture_provider_below_the_feed_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        shared_texture_preset(
            tmp.path(),
            "DLSS5_Feed@DLSS5_Feed.fx,MotionEstimation@MotionEstimation.fx",
        );
        let stack = inspect_stack(tmp.path());

        assert!(
            ids(&stack).contains(&"stack-technique-order"),
            "expected the ordering finding, got {:?}",
            ids(&stack)
        );
        let detail = &stack
            .findings
            .iter()
            .find(|f| f.id == "stack-technique-order")
            .unwrap()
            .detail;
        assert!(
            detail.contains("MotionEstimation"),
            "the finding must name the real provider, got {detail}"
        );
    }

    /// Current DLSS5-Feeder replaced the two-level `DLSS5_MV_SOURCE` +
    /// `MV_PROVIDER` scheme with a single `DLSS5_MV_PROVIDER` definition taking
    /// 0–4, and recommends LumeniteFX Kernel (3). Reading only the older scheme
    /// would leave the ordering check looking for a technique that is not in
    /// the preset — the exact silent no-op this whole resolver exists to stop.
    #[test]
    fn the_current_mv_provider_definition_is_understood() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=DLSS5_Feed@DLSS5_Feed.fx,LUMENITE: Kernel 2.0@lumenite_kernel.fx\n\n\
             [DLSS5_Feed.fx]\nPreprocessorDefinitions=DLSS5_MV_PROVIDER=3\n",
        );
        let stack = inspect_stack(tmp.path());

        let provider = stack
            .preset
            .as_ref()
            .and_then(|preset| preset.mv_provider.as_ref())
            .expect("provider");
        assert_eq!(provider.kind, MvProviderKind::LumeniteKernel);
        assert_eq!(provider.technique.as_deref(), Some("LUMENITE: Kernel 2.0"));

        let detail = &stack
            .findings
            .iter()
            .find(|f| f.id == "stack-technique-order")
            .expect("the ordering check must run for this provider too")
            .detail;
        assert!(detail.contains("LUMENITE: Kernel 2.0"), "{detail}");
    }

    /// The numbering is not the same as the old runtime combo: `1` is LaunchPad
    /// under `DLSS5_MV_PROVIDER`, where `0` was LaunchPad under `MV_PROVIDER`.
    /// Getting that backwards would name the wrong effect.
    #[test]
    fn the_two_provider_schemes_number_launchpad_differently() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,DLSS5_Feed@DLSS5_Feed.fx\n\n\
             [DLSS5_Feed.fx]\nPreprocessorDefinitions=DLSS5_MV_PROVIDER=1\n",
        );
        let stack = inspect_stack(tmp.path());
        let provider = stack.preset.unwrap().mv_provider.unwrap();

        assert_eq!(provider.kind, MvProviderKind::Launchpad);
        assert_eq!(
            provider.technique.as_deref(),
            Some("MartysMods_Launchpad"),
            "the preset's own spelling is matched, not just upstream's"
        );
    }

    /// A build newer than this table must not be guessed at. Falling back to the
    /// convention that needs no name is the honest answer.
    #[test]
    fn an_unknown_provider_value_falls_back_rather_than_guessing() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,DLSS5_Feed@DLSS5_Feed.fx\n\n\
             [DLSS5_Feed.fx]\nPreprocessorDefinitions=DLSS5_MV_PROVIDER=9\n",
        );
        let stack = inspect_stack(tmp.path());
        let provider = stack.preset.unwrap().mv_provider.unwrap();

        assert_eq!(provider.kind, MvProviderKind::SharedTexture);
    }

    #[test]
    fn a_shared_texture_provider_above_the_feed_is_quiet() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        shared_texture_preset(
            tmp.path(),
            "MotionEstimation@MotionEstimation.fx,DLSS5_Feed@DLSS5_Feed.fx",
        );
        let stack = inspect_stack(tmp.path());

        assert!(stack.findings.is_empty(), "got {:?}", ids(&stack));
        let provider = stack.preset.unwrap().mv_provider.unwrap();
        assert_eq!(provider.kind, MvProviderKind::SharedTexture);
        assert_eq!(provider.technique.as_deref(), Some("MotionEstimation"));
    }

    /// Selecting the shared texture with nothing enabled to write it is the
    /// silent still-image case the effect's own tooltip warns about.
    #[test]
    fn a_provider_nobody_supplies_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,DLSS5_Feed@DLSS5_Feed.fx\n\n\
             [DLSS5_Feed.fx]\nMV_PROVIDER=1\n",
        );
        let stack = inspect_stack(tmp.path());

        assert!(
            ids(&stack).contains(&"stack-mv-provider-missing"),
            "got {:?}",
            ids(&stack)
        );
    }

    /// LaunchPad selected but not enabled is the same failure from the other
    /// side, and also used to go unreported.
    #[test]
    fn launchpad_selected_but_not_enabled_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=DLSS5_Feed@DLSS5_Feed.fx\n",
        );
        let stack = inspect_stack(tmp.path());

        assert!(
            ids(&stack).contains(&"stack-mv-provider-missing"),
            "got {:?}",
            ids(&stack)
        );
    }

    /// `DLSS5_MV_SOURCE=1` means LaunchPad is not compiled into the effect at
    /// all, so a stale `MV_PROVIDER=0` in the preset must not be believed.
    #[test]
    fn mv_source_one_forces_the_shared_texture_path() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShade.ini",
            &REAL_RESHADE_INI.replace(
                "[GENERAL]",
                "[GENERAL]\nPreprocessorDefinitions=DLSS5_MV_SOURCE=1,RESHADE_DEPTH_INPUT_IS_REVERSED=0",
            ),
        );
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,DLSS5_Feed@DLSS5_Feed.fx\n\n\
             [DLSS5_Feed.fx]\nMV_PROVIDER=0\n",
        );
        let stack = inspect_stack(tmp.path());

        let provider = stack.preset.unwrap().mv_provider.unwrap();
        assert_eq!(provider.kind, MvProviderKind::SharedTexture);
        assert_eq!(provider.technique, None);
    }

    #[test]
    fn a_preset_without_the_feed_has_no_provider_question() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx\n",
        );
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.preset.as_ref().unwrap().mv_provider, None);
        assert!(ids(&stack).contains(&"stack-feed-technique-off"));
    }

    #[test]
    fn correct_order_is_not_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        let stack = inspect_stack(tmp.path());
        assert!(!ids(&stack).contains(&"stack-technique-order"));
    }

    #[test]
    fn nr_addon_without_its_runtime_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        std::fs::remove_file(tmp.path().join("nvngx_dlssnr.dll")).unwrap();
        let stack = inspect_stack(tmp.path());
        assert!(ids(&stack).contains(&"stack-nr-runtime-missing"));
    }

    #[test]
    fn addons_without_an_injector_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        std::fs::remove_file(tmp.path().join("dxgi.dll")).unwrap();
        let stack = inspect_stack(tmp.path());
        assert!(ids(&stack).contains(&"stack-no-injector"));
    }

    #[test]
    fn a_missing_shader_source_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        std::fs::remove_file(
            tmp.path()
                .join("reshade-shaders")
                .join("Shaders")
                .join("DLSS5_Feed.fx"),
        )
        .unwrap();
        let stack = inspect_stack(tmp.path());
        assert!(ids(&stack).contains(&"stack-technique-source-missing"));
    }

    #[test]
    fn a_disabled_addon_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShade.ini",
            &REAL_RESHADE_INI.replace("DisabledAddons=", "DisabledAddons=dlss5-feed.addon64"),
        );
        let stack = inspect_stack(tmp.path());
        assert!(ids(&stack).contains(&"stack-addon-disabled"));
        assert_eq!(stack.disabled_addons, vec!["dlss5-feed.addon64"]);
    }

    #[test]
    fn the_feed_host_being_absent_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        std::fs::remove_file(tmp.path().join("dlss5-feed-host64.exe")).unwrap();
        let stack = inspect_stack(tmp.path());
        assert!(ids(&stack).contains(&"stack-feed-host-missing"));
    }

    #[test]
    fn the_ordered_technique_list_is_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        let stack = inspect_stack(tmp.path());
        let preset = stack.preset.expect("preset");

        assert!(preset.exists);
        let names: Vec<&str> = preset.techniques.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["MartysMods_Launchpad", "DLSS5_Feed"]);
        assert!(preset.techniques.iter().all(|t| t.source_present));
        assert!(preset.available.len() >= 3);
    }

    #[test]
    fn the_tuning_block_is_read() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        let structure = stack
            .tuning
            .iter()
            .find(|t| t.key == "nrlocalstructure")
            .expect("NRLocalStructure should be read");
        assert_eq!(structure.value, "1.4");
        assert_eq!(stack.tuning.len(), 4);
    }

    /// `renodx-dlss5.addon64` really does ship with no ProductName, so the
    /// only place its name exists is ReShade's own OverlayCollapsed mapping.
    #[test]
    fn an_addon_name_falls_back_to_reshades_own_mapping() {
        let overlay =
            "DLSS 5 Neural Rendering@renodx-dlss5.addon64,DLSS 5 Feed 0.4.0@dlss5-feed.addon64";
        assert_eq!(
            addon_display_name(Some(overlay), "renodx-dlss5.addon64").as_deref(),
            Some("DLSS 5 Neural Rendering")
        );
        assert_eq!(
            addon_display_name(Some(overlay), "RENODX-DLSS5.ADDON64").as_deref(),
            Some("DLSS 5 Neural Rendering"),
            "ReShade's own casing must not decide whether a name is found"
        );
        // Something genuinely unidentifiable stays unidentified.
        assert_eq!(addon_display_name(Some(overlay), "mystery.addon64"), None);
        assert_eq!(addon_display_name(None, "renodx-dlss5.addon64"), None);
    }

    /// The user's own hand-rolled backup convention must be recognised as
    /// displaced originals, not mistaken for live stack files.
    #[test]
    fn hand_made_backups_are_recognised_as_originals() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(tmp.path(), "nvngx_dlss.dll.disabled-bak", "");
        write(tmp.path(), "d3dcompiler_47.dll.eso-orig-bak", "");
        write(tmp.path(), "d3dcompiler_47.dll", "");
        let stack = inspect_stack(tmp.path());

        let names: Vec<&str> = stack
            .preserved_originals
            .iter()
            .map(|o| o.file_name.as_str())
            .collect();
        assert!(names.contains(&"nvngx_dlss.dll.disabled-bak"), "{names:?}");
        assert!(
            names.contains(&"d3dcompiler_47.dll.eso-orig-bak"),
            "{names:?}"
        );

        // A backup is not a stack item.
        assert!(!stack.items.iter().any(|i| i.file_name.ends_with("-bak")));

        let dlss_backup = stack
            .preserved_originals
            .iter()
            .find(|o| o.file_name.starts_with("nvngx_dlss.dll."))
            .unwrap();
        assert_eq!(dlss_backup.backs_up.as_deref(), Some("nvngx_dlss.dll"));
    }

    #[test]
    fn a_backup_whose_live_file_is_gone_reports_no_target() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "eso64.exe", "");
        write(tmp.path(), "nvngx_dlssg.dll.bak", "");
        let stack = inspect_stack(tmp.path());

        let orphan = stack
            .preserved_originals
            .iter()
            .find(|o| o.file_name == "nvngx_dlssg.dll.bak")
            .expect("backup should be listed");
        assert_eq!(orphan.backs_up, None);
    }

    /// `PresetPath` is text out of a config file Kalpa does not own. A value
    /// pointing outside the client folder must read as "no preset there",
    /// never as a preset whose contents get reported — and later edited.
    #[test]
    fn a_preset_path_pointing_outside_the_client_folder_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let client = tmp.path().join("client");
        std::fs::create_dir_all(&client).unwrap();
        healthy_stack(&client);
        std::fs::write(
            tmp.path().join("outside.ini"),
            "Techniques=DLSS5_Feed@DLSS5_Feed.fx\n",
        )
        .unwrap();

        for escape in ["..\\outside.ini", "../outside.ini", "C:outside.ini"] {
            write(
                &client,
                "ReShade.ini",
                &REAL_RESHADE_INI.replace(
                    "PresetPath=.\\ReShadePreset.ini",
                    &format!("PresetPath={escape}"),
                ),
            );
            let stack = inspect_stack(&client);
            let preset = stack.preset.expect("the key is still reported");
            assert!(
                !preset.exists,
                "{escape} must not resolve to a file outside the client folder"
            );
            assert!(preset.techniques.is_empty(), "{escape}");
        }
    }

    #[test]
    fn a_parked_injector_reads_as_a_switched_off_stack() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        std::fs::rename(
            tmp.path().join("dxgi.dll"),
            tmp.path().join("dxgi.dll.kalpa-off"),
        )
        .unwrap();
        let stack = inspect_stack(tmp.path());

        assert!(stack.is_disabled);
        assert!(!stack.is_empty);
        assert_eq!(ids(&stack), vec!["stack-disabled"]);

        let parked = stack.parked.first().expect("the parked injector");
        assert_eq!(parked.file_name, "dxgi.dll.kalpa-off");
        assert_eq!(parked.restores, "dxgi.dll");
        assert!(!parked.target_present, "disable freed the live name");

        // A parked file is neither a live stack item nor one of the user's own
        // originals.
        assert!(!stack
            .items
            .iter()
            .any(|i| i.file_name.contains("kalpa-off")));
        assert!(!stack
            .preserved_originals
            .iter()
            .any(|o| o.file_name.contains("kalpa-off")));
    }

    #[test]
    fn a_parked_file_whose_name_is_occupied_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(tmp.path(), "nvngx_dlss.dll.kalpa-off", "the modded one");
        let stack = inspect_stack(tmp.path());

        let parked = stack
            .parked
            .iter()
            .find(|p| p.restores == "nvngx_dlss.dll")
            .expect("parked runtime");
        assert!(
            parked.target_present,
            "the stock file is live under that name, so re-enable must displace it"
        );
        // Parking a runtime is not parking the injector.
        assert!(!stack.is_disabled);
    }

    /// The trap this suffix exists to avoid: Kalpa must never write, and never
    /// treat as its own, any of the names a user uses for their originals.
    #[test]
    fn kalpas_parking_suffix_is_not_one_of_the_users_own() {
        for suffix in BACKUP_SUFFIXES {
            assert_ne!(PARKED_SUFFIX, suffix);
            assert!(!PARKED_SUFFIX.ends_with(suffix));
        }
        assert_eq!(backup_target("dxgi.dll.kalpa-off"), None);
    }

    #[test]
    fn ini_values_may_contain_equals_and_commas() {
        // ReShade's docking blobs are full of both; only the first `=` splits.
        let ini = parse_ini("[A]\nDocking=ID=0x1,Pos=8,,8\nPlain=x\n");
        assert_eq!(ini_get(&ini, "A", "Docking"), Some("ID=0x1,Pos=8,,8"));
        assert_eq!(ini_get(&ini, "a", "plain"), Some("x"));
    }

    #[test]
    fn a_headerless_config_parses_into_the_empty_section() {
        // dlss5-feed.cfg has no [SECTION] headers at all.
        let ini = parse_ini("enabled=1\nmode=2\n; a comment\n");
        assert_eq!(ini_get(&ini, "", "enabled"), Some("1"));
        assert_eq!(ini_get(&ini, "", "mode"), Some("2"));
    }

    #[test]
    fn shader_sources_are_found_one_level_deep() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        let nested = tmp
            .path()
            .join("reshade-shaders")
            .join("Shaders")
            .join("MartysMods");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::remove_file(
            tmp.path()
                .join("reshade-shaders")
                .join("Shaders")
                .join("MartysMods_LAUNCHPAD.fx"),
        )
        .unwrap();
        std::fs::write(nested.join("MartysMods_LAUNCHPAD.fx"), "").unwrap();

        let stack = inspect_stack(tmp.path());
        assert!(
            !ids(&stack).contains(&"stack-technique-source-missing"),
            "a shader in a subfolder is still found, got {:?}",
            ids(&stack)
        );
    }
}
