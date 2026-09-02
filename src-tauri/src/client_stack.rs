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

/// Files that are companions to a known addon: not loaded by the game, but the
/// stack does not work without them.
const COMPANION_NAMES: [&str; 3] = [
    "dlss5-feed-host64.exe",
    "dlss5-feed.cfg",
    "dlss5-feed-host32.exe",
];

/// The technique that must run *before* `DLSS5_Feed`, because it produces the
/// motion vectors and normals the feed consumes.
const FEED_PREREQUISITE: &str = "martysmods_launchpad";
const FEED_TECHNIQUE: &str = "dlss5_feed";

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

    let is_empty = items.is_empty() && preserved_originals.is_empty() && !shaders.present;

    let mut stack = ClientStack {
        client_dir: client_dir.to_string_lossy().to_string(),
        items,
        preserved_originals,
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
    // Presets are written relative to the client dir as `.\Name.ini`.
    let path = client_dir.join(relative.trim_start_matches(r".\").trim_start_matches("./"));
    let exists = path.is_file();
    let contents = if exists {
        std::fs::read_to_string(&path).unwrap_or_default()
    } else {
        String::new()
    };
    let preset_ini = parse_ini(&contents);

    let shader_dir = client_dir.join("reshade-shaders").join("Shaders");
    let technique_entries = ini_get(&preset_ini, "", "Techniques").unwrap_or_default();
    let techniques = technique_entries
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

    Some(PresetInfo {
        path: raw.to_string(),
        exists,
        techniques,
        available,
    })
}

/// Shader packs nest one level (`Shaders/MartysMods/...`), so look in the root
/// and in immediate subdirectories.
fn shader_source_exists(shader_dir: &Path, source: &str) -> bool {
    if shader_dir.join(source).is_file() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(shader_dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.path().is_dir() && entry.path().join(source).is_file())
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
        // loads, nothing errors, and the output is quietly wrong.
        let position = |needle: &str| {
            preset
                .techniques
                .iter()
                .position(|t| t.name.to_ascii_lowercase() == needle)
        };
        if let (Some(feed), Some(launchpad)) =
            (position(FEED_TECHNIQUE), position(FEED_PREREQUISITE))
        {
            if launchpad > feed {
                out.push(finding(
                    "stack-technique-order",
                    HealthLevel::Danger,
                    "Effects are in the wrong order",
                    "DLSS5_Feed runs before MartysMods_Launchpad in this preset. Launchpad \
                     produces the motion vectors and normals the feed consumes, so with this \
                     order the feed reads last frame's data. Nothing errors — the image is \
                     just quietly wrong."
                        .to_string(),
                ));
            }
        } else if feed_addon && position(FEED_TECHNIQUE).is_none() {
            out.push(finding(
                "stack-feed-technique-off",
                HealthLevel::Warning,
                "DLSS 5 Feed is installed but not enabled",
                "The feed addon is present, but the active preset does not enable the \
                 DLSS5_Feed technique, so nothing feeds the runtime."
                    .to_string(),
            ));
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
