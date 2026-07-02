#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

slint::include_modules!();

use serde::{Deserialize, Serialize};
use slint::{
    Color, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel,
};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ThemeSeed {
    bg_base: String,
    background: String,
    surface: String,
    foreground: String,
    muted_foreground: String,
    primary: String,
    primary_foreground: String,
    accent: String,
    border: String,
    orb1: String,
    orb2: String,
    orb3: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThemeEnvelope {
    colors: ThemeSeed,
    skin_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThemeCatalog {
    default_theme_id: String,
    root_theme_id: Option<String>,
    themes: Vec<CatalogTheme>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogTheme {
    id: String,
    name: String,
    category: String,
    description: String,
    colors: ThemeSeed,
    skin_id: Option<String>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeCustomThemeStore {
    themes: Vec<CatalogTheme>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeSettings {
    auto_update: bool,
    warn_eso_running: bool,
    official_uploader: bool,
    auto_open_analysis: bool,
    conflict_policy: i32,
}

impl Default for NativeSettings {
    fn default() -> Self {
        Self {
            auto_update: false,
            warn_eso_running: true,
            official_uploader: false,
            auto_open_analysis: false,
            conflict_policy: 0,
        }
    }
}

struct ThemeSelection {
    seed: ThemeSeed,
    skin_kind: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ContrastLevel {
    Fail,
    Ok,
    Great,
}

struct NativeContrastCheck {
    label: &'static str,
    ratio: f64,
    level: ContrastLevel,
}

impl ThemeSelection {
    fn colors_only(seed: ThemeSeed) -> Self {
        Self { seed, skin_kind: 0 }
    }

    fn with_skin(seed: ThemeSeed, skin_id: Option<&str>) -> Self {
        Self {
            seed,
            skin_kind: skin_kind(skin_id),
        }
    }
}

const BUILTIN_THEME_CATALOG: &str = include_str!("../assets/themes/builtin-themes.json");

thread_local! {
    static COLLAPSED_FILE_FOLDERS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    static ORB_SKIN_CACHE: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
}

#[derive(Clone)]
struct AddonModels {
    all: Rc<RefCell<Vec<AddonEntry>>>,
    visible: Rc<VecModel<AddonEntry>>,
    view_key: Rc<RefCell<Option<AddonViewKey>>>,
}

#[derive(Clone, PartialEq, Eq)]
struct AddonViewKey {
    query: String,
    filter_mode: i32,
    sort_mode: i32,
}

struct AddonFilterCounts {
    total: i32,
    addons: i32,
    libraries: i32,
    favorites: i32,
    outdated: i32,
    issues: i32,
    disabled: i32,
    testing: i32,
    broken: i32,
    essential: i32,
    raid: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifestDraft {
    #[serde(alias = "addon_folder")]
    addon_folder: String,
    #[serde(alias = "backed_up_at")]
    backed_up_at: String,
    #[serde(alias = "update_from")]
    update_from: String,
    #[serde(alias = "update_to")]
    update_to: String,
    files: Vec<String>,
}

fn main() -> Result<(), slint::PlatformError> {
    let backend = std::env::var("KALPA_SLINT_BACKEND")
        .or_else(|_| std::env::var("SLINT_BACKEND"))
        .unwrap_or_else(|_| "winit-software".to_string());

    slint::BackendSelector::new()
        .backend_name(backend.into())
        .select()?;

    let ui = KalpaWindow::new()?;
    place_demo_window(&ui);
    let custom_themes = Rc::new(RefCell::new(read_custom_themes()));
    set_theme_gallery(&ui, &custom_themes.borrow());

    let addon_models = apply_mock_data(&ui);
    let active_theme_id = apply_initial_theme(&ui);
    ui.set_active_theme_id(active_theme_id.into());
    seed_initial_theme_draft(&ui, &custom_themes.borrow());
    apply_initial_native_settings(&ui);
    apply_runtime_flags(&ui);
    apply_addon_view(&ui, &addon_models);
    let discover_installed_ids = Rc::new(RefCell::new(installed_discover_ids(
        &addon_models.all.borrow(),
    )));
    let discover_model = Rc::new(RefCell::new(apply_discover_data(
        &ui,
        ui.get_discover_tab(),
        &discover_installed_ids.borrow(),
    )));
    refresh_file_browser(&ui);

    wire_window_controls(&ui);
    wire_file_browser(&ui);
    wire_addon_filters(&ui, addon_models.clone());
    wire_header_actions(&ui, addon_models.clone());
    wire_tag_editor(&ui, addon_models.clone());
    wire_batch_actions(&ui, addon_models.clone());
    wire_context_actions(&ui, addon_models.clone());
    wire_detail_actions(&ui, addon_models);
    wire_discover(&ui, discover_model, discover_installed_ids);
    wire_theme_actions(&ui, custom_themes);
    wire_settings_actions(&ui);
    ui.run()
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MonitorPlacement {
    work_left: i32,
    work_top: i32,
    primary: bool,
}

#[cfg(target_os = "windows")]
fn place_demo_window(ui: &KalpaWindow) {
    if std::env::var("KALPA_NATIVE_AUTO_PLACE")
        .map(|value| matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
        .unwrap_or(false)
    {
        return;
    }

    if let Some(monitor) = preferred_demo_monitor(&windows_monitor_placements()) {
        ui.window().set_position(slint::PhysicalPosition::new(
            monitor.work_left,
            monitor.work_top,
        ));
    }
}

#[cfg(not(target_os = "windows"))]
fn place_demo_window(_ui: &KalpaWindow) {}

#[cfg(target_os = "windows")]
fn preferred_demo_monitor(monitors: &[MonitorPlacement]) -> Option<MonitorPlacement> {
    let primary = monitors
        .iter()
        .copied()
        .find(|monitor| monitor.primary)
        .or_else(|| monitors.first().copied())?;
    monitors
        .iter()
        .copied()
        .filter(|monitor| !monitor.primary && monitor.work_left > primary.work_left)
        .max_by_key(|monitor| monitor.work_left)
        .or(Some(primary))
}

#[cfg(target_os = "windows")]
fn windows_monitor_placements() -> Vec<MonitorPlacement> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WinRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MonitorInfo {
        cb_size: u32,
        rc_monitor: WinRect,
        rc_work: WinRect,
        dw_flags: u32,
    }

    type Bool = i32;
    type Hdc = *mut std::ffi::c_void;
    type Hmonitor = *mut std::ffi::c_void;
    type Lparam = isize;

    extern "system" {
        fn EnumDisplayMonitors(
            hdc: Hdc,
            lprc_clip: *const WinRect,
            lpfn_enum: Option<
                unsafe extern "system" fn(Hmonitor, Hdc, *mut WinRect, Lparam) -> Bool,
            >,
            dw_data: Lparam,
        ) -> Bool;
        fn GetMonitorInfoW(hmonitor: Hmonitor, lpmi: *mut MonitorInfo) -> Bool;
    }

    unsafe extern "system" fn enum_monitor(
        monitor: Hmonitor,
        _hdc: Hdc,
        _rect: *mut WinRect,
        data: Lparam,
    ) -> Bool {
        let monitors = &mut *(data as *mut Vec<MonitorPlacement>);
        let mut info = MonitorInfo {
            cb_size: std::mem::size_of::<MonitorInfo>() as u32,
            rc_monitor: WinRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rc_work: WinRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dw_flags: 0,
        };

        if GetMonitorInfoW(monitor, &mut info) != 0 {
            monitors.push(MonitorPlacement {
                work_left: info.rc_work.left,
                work_top: info.rc_work.top,
                primary: info.dw_flags & 1 == 1,
            });
        }

        1
    }

    let mut monitors = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(enum_monitor),
            &mut monitors as *mut Vec<MonitorPlacement> as Lparam,
        );
    }
    monitors
}

fn apply_mock_data(ui: &KalpaWindow) -> AddonModels {
    if let Some(addons_root) = addons_source_root() {
        if let Ok(addons) = real_addon_entries(&addons_root) {
            if !addons.is_empty() {
                return addon_models_from_entries(ui, addons);
            }
        }
    }

    let mut addons = vec![
        addon_entry(
            "BSC's How To Kynes Aegis",
            "BSCHowToKynesAegis",
            "3387",
            "BloodStainChild666",
            "2.1.3",
            "101048, 101049",
            "Addon",
            "1/28/2026",
            "Trial helper and callout package for Kyne's Aegis encounters.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "Calamath's BookFont Stylist",
            "CalamathsBookFontStylist",
            "3604",
            "Calamath",
            "5.0.1",
            "101048, 101049",
            "Library",
            "2/18/2026",
            "Shared font styling helper used by Calamath addons.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "Caro's Skill Point Saver",
            "CarosSkillPointSaver",
            "2840",
            "Irniben",
            "6.0.0",
            "101048, 101049",
            "Addon",
            "12/14/2025",
            "Stores and restores skill point setups for fast character swaps.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "Code's Combat Alerts",
            "CodesCombatAlerts",
            "3520",
            "@code65536",
            "2.4.10",
            "101048, 101049",
            "Addon",
            "2/9/2026",
            "Encounter alerts and timers for combat mechanics.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "Combat Metronome",
            "CombatMetronome",
            "2572",
            "Darianopolis, barny",
            "1.7.4",
            "101048, 101049",
            "Addon",
            "3/1/2026",
            "GCD and weave timing display for combat rotations.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "CombatMetrics",
            "CombatMetrics",
            "1360",
            "Solinur",
            "1.7.7",
            "101048, 101049",
            "Addon",
            "3/3/2026",
            "CombatMetrics is a tool to analyse your performance in fights.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "Cooldowns",
            "Cooldowns",
            "2463",
            "@g4rr3t (NA)",
            "1.6.1",
            "101048, 101049",
            "Addon",
            "1/4/2026",
            "Tracks important cooldown timers in compact combat widgets.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "Coral Aerie Helper",
            "CoralAerieHelper",
            "3417",
            "Branddi",
            "1.0.2",
            "101048, 101049",
            "Addon",
            "12/2/2025",
            "Dungeon helper for Coral Aerie mechanics.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "CraftStore Fixed and Improved 1.1",
            "CraftStoreFixedAndImproved",
            "1590",
            "AlphaLemming, continued by Vladislav",
            "11.0.9",
            "101048, 101049",
            "Addon",
            "2/22/2026",
            "Crafting, research, motifs, recipes, and character knowledge tracking.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "CrutchAlerts",
            "CrutchAlerts",
            "3218",
            "Kyzeragon",
            "2.14.0",
            "101048, 101049",
            "Addon",
            "3/6/2026",
            "Advanced raid alerts and quality-of-life combat helpers.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "LibAddonMenu-2.0",
            "LibAddonMenu-2.0",
            "7",
            "Seerah, sirinsidiator",
            "38",
            "101048, 101049",
            "Library",
            "2/14/2026",
            "Shared settings panel library used by many ESO addons.",
            false,
            true,
            false,
            3,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "LibAsync",
            "LibAsync",
            "2125",
            "sirinsidiator",
            "2.1",
            "101048, 101049",
            "Library",
            "10/7/2025",
            "Asynchronous task helper library for ESO addons.",
            false,
            true,
            false,
            3,
            "Ready",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "LibCombat",
            "LibCombat",
            "82",
            "ESOUI Community",
            "82",
            "101048, 101049",
            "Library",
            "1/22/2026",
            "Shared combat data library for combat-analysis addons.",
            false,
            true,
            false,
            3,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "LibCustomMenu",
            "LibCustomMenu",
            "730",
            "Shadowfen",
            "730",
            "101048, 101049",
            "Library",
            "11/11/2025",
            "Context menu helper library for ESO UI extensions.",
            false,
            true,
            false,
            3,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "Map Pins",
            "MapPins",
            "1881",
            "Hoft",
            "1.0.12",
            "101048, 101049",
            "Addon",
            "1/16/2026",
            "Adds map pins for lorebooks, skyshards, surveys, and other collectibles.",
            false,
            true,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "RaidNotifier Updated",
            "RaidNotifierUpdated",
            "1355",
            "Raid Tools Team",
            "4.2.3",
            "101048, 101049",
            "Addon",
            "2/25/2026",
            "Raid notifications and encounter warnings for group content.",
            false,
            false,
            false,
            1,
            "Update",
            1,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "Srendarr",
            "Srendarr",
            "655",
            "Phinix",
            "2.5.8",
            "101048, 101049",
            "Addon",
            "1/30/2026",
            "Aura, buff, and debuff tracking with configurable displays.",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        ),
        addon_entry(
            "Wizard's Wardrobe",
            "WizardsWardrobe",
            "3170",
            "Dolgubon",
            "1.19.6",
            "101048, 101049",
            "Addon",
            "2/27/2026",
            "Build and gear-set management for dungeons, arenas, and trials.",
            false,
            false,
            false,
            5,
            "Ready",
            0,
            "",
            0,
            "",
            0,
        ),
    ];

    if let Some(combat_metrics) = addons
        .iter_mut()
        .find(|addon| addon.folder_name.as_str() == "CombatMetrics")
    {
        combat_metrics.required_dependencies = dependency_model(vec![
            dependency_entry("CombatMetricsFightData", "v22+", true, false, true),
            dependency_entry("LibCombat", "v82+", false, false, false),
            dependency_entry("LibAddonMenu-2.0", "v38+", false, false, false),
            dependency_entry("LibCustomMenu", "v730+", false, false, false),
        ]);
        combat_metrics.optional_dependencies = dependency_model(vec![
            dependency_entry("LibDebugLogger", "v1+", true, false, true),
            dependency_entry("LibDataEncode", "v1+", false, false, false),
        ]);
    }

    addon_models_from_entries(ui, addons)
}

fn addon_models_from_entries(ui: &KalpaWindow, mut addons: Vec<AddonEntry>) -> AddonModels {
    populate_dependent_summaries(&mut addons);
    let all = Rc::new(RefCell::new(addons.clone()));
    let model = Rc::new(VecModel::from(addons));
    ui.set_addons(model.clone().into());
    set_addon_counts(ui, &all.borrow());
    AddonModels {
        all,
        visible: model,
        view_key: Rc::new(RefCell::new(None)),
    }
}

fn populate_dependent_summaries(addons: &mut [AddonEntry]) {
    let dependency_names = addons
        .iter()
        .map(|addon| {
            (
                addon.title.to_string(),
                addon.folder_name.to_ascii_lowercase(),
                addon_dependencies(addon),
            )
        })
        .collect::<Vec<_>>();

    for addon in addons {
        let folder_name = addon.folder_name.to_ascii_lowercase();
        let title = addon.title.to_ascii_lowercase();
        let dependents = dependency_names
            .iter()
            .filter(|(dependent_title, _, dependencies)| {
                dependent_title.as_str() != addon.title.as_str()
                    && dependencies
                        .iter()
                        .any(|dependency| dependency == &folder_name || dependency == &title)
            })
            .map(|(dependent_title, _, _)| dependent_title.as_str())
            .take(3)
            .collect::<Vec<_>>();

        addon.dependent_summary = dependent_warning_summary(&dependents).into();
    }
}

fn addon_dependencies(addon: &AddonEntry) -> Vec<String> {
    let mut dependencies = Vec::new();
    for model in [
        addon.required_dependencies.clone(),
        addon.optional_dependencies.clone(),
    ] {
        for index in 0..model.row_count() {
            if let Some(dependency) = model.row_data(index) {
                dependencies.push(dependency.name.to_ascii_lowercase());
            }
        }
    }
    dependencies
}

fn dependent_warning_summary(dependents: &[&str]) -> String {
    match dependents {
        [] => String::new(),
        [one] => format!("{one} depends on this addon and may not work."),
        [first, second] => {
            format!("{first} and {second} depend on this addon and may not work.")
        }
        [first, second, ..] => {
            let extra = dependents.len().saturating_sub(2);
            format!("{first}, {second}, and {extra} more depend on this addon and may not work.")
        }
    }
}

struct RealAddonDraft {
    entry: AddonEntry,
    required_dependencies: Vec<(String, String)>,
    optional_dependencies: Vec<(String, String)>,
}

fn real_addon_entries(addons_root: &Path) -> Result<Vec<AddonEntry>, String> {
    let mut addon_dirs = fs::read_dir(addons_root)
        .map_err(|error| format!("Failed to read AddOns folder: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .chars()
                .next()
                .is_some_and(|first| first != '.')
        })
        .collect::<Vec<_>>();

    addon_dirs.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());

    let folder_names = addon_dirs
        .iter()
        .map(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase())
        .flat_map(|name| {
            [
                name.clone(),
                name.strip_suffix(".disabled")
                    .map(str::to_string)
                    .unwrap_or(name),
            ]
        })
        .collect::<BTreeSet<_>>();

    let mut drafts = addon_dirs
        .into_iter()
        .filter_map(|dir| real_addon_draft(&dir.path()).ok())
        .collect::<Vec<_>>();

    drafts.sort_by(|left, right| {
        left.entry
            .title
            .to_ascii_lowercase()
            .cmp(&right.entry.title.to_ascii_lowercase())
    });

    let addons = drafts
        .into_iter()
        .map(|mut draft| {
            draft.entry.required_dependencies =
                dependency_model_from_specs(draft.required_dependencies, &folder_names);
            draft.entry.optional_dependencies =
                dependency_model_from_specs(draft.optional_dependencies, &folder_names);
            draft.entry
        })
        .collect::<Vec<_>>();

    Ok(addons)
}

fn real_addon_draft(addon_dir: &Path) -> Result<RealAddonDraft, String> {
    let disk_folder_name = addon_dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| "Addon folder has no name.".to_string())?;
    let disabled = disk_folder_name.ends_with(".disabled");
    let folder_name = disk_folder_name
        .strip_suffix(".disabled")
        .unwrap_or(&disk_folder_name)
        .to_string();

    let manifest = manifest_path(addon_dir, &folder_name)
        .ok_or_else(|| format!("No manifest found for {folder_name}"))?;
    let content = fs::read_to_string(&manifest)
        .map_err(|error| format!("Failed to read manifest for {folder_name}: {error}"))?;

    let title = manifest_field(&content, "Title").unwrap_or_else(|| folder_name.clone());
    let author = manifest_field(&content, "Author").unwrap_or_default();
    let version = manifest_field(&content, "Version")
        .or_else(|| manifest_field(&content, "AddOnVersion"))
        .unwrap_or_default();
    let api_version = manifest_field(&content, "APIVersion").unwrap_or_default();
    let description = manifest_field(&content, "Description").unwrap_or_default();
    let is_library = manifest_bool(&content, "IsLibrary")
        || folder_name.to_ascii_lowercase().starts_with("lib")
        || title.to_ascii_lowercase().starts_with("lib");
    let esoui_id = manifest_field(&content, "X-Website")
        .or_else(|| manifest_field(&content, "Website"))
        .and_then(|url| esoui_id_from_input(&url))
        .unwrap_or_default();

    let required_dependencies = dependency_specs(&manifest_field(&content, "DependsOn"));
    let optional_dependencies = dependency_specs(&manifest_field(&content, "OptionalDependsOn"));

    let mut entry = addon_entry(
        &title,
        &folder_name,
        &esoui_id,
        &author,
        &version,
        &api_version,
        if is_library { "Library" } else { "Addon" },
        "",
        &description,
        false,
        is_library,
        disabled,
        if is_library { 3 } else { 0 },
        "",
        0,
        "",
        0,
        "",
        0,
    );

    if !description.is_empty() {
        entry.description = description.into();
    }
    if disabled {
        entry.badge3 = "Disabled".into();
        entry.badge3_kind = 5;
    }

    Ok(RealAddonDraft {
        entry,
        required_dependencies,
        optional_dependencies,
    })
}

fn manifest_path(addon_dir: &Path, folder_name: &str) -> Option<PathBuf> {
    let expected = addon_dir.join(format!("{folder_name}.txt"));
    if expected.is_file() {
        return Some(expected);
    }

    let mut txt_files = fs::read_dir(addon_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        })
        .collect::<Vec<_>>();

    txt_files.sort();
    txt_files.into_iter().next()
}

fn manifest_field(content: &str, key: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("##")?.trim();
        let (field, value) = line.split_once(':')?;
        if field.trim().eq_ignore_ascii_case(key) {
            let cleaned = clean_manifest_value(value);
            (!cleaned.is_empty()).then_some(cleaned)
        } else {
            None
        }
    })
}

fn manifest_bool(content: &str, key: &str) -> bool {
    manifest_field(content, key)
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
        .unwrap_or(false)
}

fn clean_manifest_value(value: &str) -> String {
    strip_eso_markup(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn strip_eso_markup(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut out = String::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index] == '|' && index + 1 < chars.len() {
            let marker = chars[index + 1].to_ascii_lowercase();
            if marker == 'c' && index + 8 <= chars.len() {
                index += 8;
                continue;
            }
            if marker == 'r' {
                index += 2;
                continue;
            }
            if marker == 't' || marker == 'u' {
                index += 2;
                while index + 1 < chars.len()
                    && !(chars[index] == '|' && chars[index + 1].to_ascii_lowercase() == marker)
                {
                    index += 1;
                }
                index = (index + 2).min(chars.len());
                continue;
            }
        }

        out.push(chars[index]);
        index += 1;
    }

    out
}

fn dependency_specs(field: &Option<String>) -> Vec<(String, String)> {
    field
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .filter_map(|raw| {
            let raw = raw.trim_matches([',', ';']);
            if raw.is_empty() {
                return None;
            }

            let (name, version) = raw
                .split_once(">=")
                .or_else(|| raw.split_once('='))
                .map(|(name, version)| (name, version))
                .unwrap_or((raw, ""));
            let name = name.trim_matches([',', ';']).trim();
            (!name.is_empty()).then(|| (name.to_string(), version.trim().to_string()))
        })
        .collect()
}

fn dependency_model_from_specs(
    dependencies: Vec<(String, String)>,
    folder_names: &BTreeSet<String>,
) -> ModelRc<DependencyEntry> {
    dependency_model(
        dependencies
            .into_iter()
            .map(|(name, version)| {
                let present = folder_names.contains(&name.to_ascii_lowercase());
                dependency_entry(
                    &name,
                    if version.is_empty() { "" } else { &version },
                    !present,
                    false,
                    !present,
                )
            })
            .collect(),
    )
}

fn installed_discover_ids(addons: &[AddonEntry]) -> BTreeSet<String> {
    addons
        .iter()
        .map(|addon| addon.esoui_id.to_string())
        .filter(|id| !id.is_empty())
        .collect()
}

fn addon_filter_counts(addons: &[AddonEntry]) -> AddonFilterCounts {
    AddonFilterCounts {
        total: addons.len() as i32,
        addons: addons.iter().filter(|addon| !addon.is_library).count() as i32,
        libraries: addons.iter().filter(|addon| addon.is_library).count() as i32,
        favorites: addons.iter().filter(|addon| addon.favorite).count() as i32,
        outdated: addons
            .iter()
            .filter(|addon| addon_has_update(addon))
            .count() as i32,
        issues: addons
            .iter()
            .filter(|addon| addon_has_required_dependency_issue(addon))
            .count() as i32,
        disabled: addons.iter().filter(|addon| addon.disabled).count() as i32,
        testing: addons
            .iter()
            .filter(|addon| addon_has_tag(addon, "testing"))
            .count() as i32,
        broken: addons
            .iter()
            .filter(|addon| addon_has_tag(addon, "broken"))
            .count() as i32,
        essential: addons
            .iter()
            .filter(|addon| addon_has_tag(addon, "essential"))
            .count() as i32,
        raid: addons
            .iter()
            .filter(|addon| addon_has_tag(addon, "raid"))
            .count() as i32,
    }
}

fn set_addon_counts(ui: &KalpaWindow, addons: &[AddonEntry]) -> AddonFilterCounts {
    let counts = addon_filter_counts(addons);
    ui.set_total_addon_count(counts.total);
    ui.set_addon_kind_count(counts.addons);
    ui.set_library_kind_count(counts.libraries);
    ui.set_favorite_addon_count(counts.favorites);
    ui.set_outdated_addon_count(counts.outdated);
    ui.set_issue_addon_count(counts.issues);
    ui.set_disabled_addon_count(counts.disabled);
    ui.set_testing_tag_count(counts.testing);
    ui.set_broken_tag_count(counts.broken);
    ui.set_essential_tag_count(counts.essential);
    ui.set_raid_tag_count(counts.raid);
    counts
}

fn apply_addon_view(ui: &KalpaWindow, models: &AddonModels) {
    let selected_folder = selected_visible_addon_folder(ui);
    let all = models.all.borrow();
    let counts = set_addon_counts(ui, &all);
    let filter_mode = normalized_filter_mode(ui.get_filter_mode(), &counts);
    if filter_mode != ui.get_filter_mode() {
        ui.set_filter_mode(filter_mode);
    }

    let rows = visible_addons(
        &all,
        ui.get_addon_search_query().as_str(),
        filter_mode,
        ui.get_sort_mode(),
    );
    *models.view_key.borrow_mut() = Some(AddonViewKey {
        query: ui.get_addon_search_query().trim().to_string(),
        filter_mode,
        sort_mode: ui.get_sort_mode(),
    });

    let next_index = selected_folder
        .and_then(|folder| {
            rows.iter()
                .position(|addon| addon.folder_name.as_str() == folder.as_str())
        })
        .unwrap_or(0)
        .min(rows.len().saturating_sub(1));

    drop(all);
    models.visible.set_vec(rows);
    ui.set_selected_index(next_index as i32);
    set_batch_state(ui, models);

    if models.visible.row_count() == 0 {
        clear_editor(ui);
    } else {
        refresh_file_browser(ui);
    }
}

fn apply_addon_view_if_key_changed(ui: &KalpaWindow, models: &AddonModels) {
    let next_key = current_addon_view_key(ui);
    if models
        .view_key
        .borrow()
        .as_ref()
        .is_some_and(|current| current == &next_key)
    {
        return;
    }

    apply_addon_view(ui, models);
}

fn current_addon_view_key(ui: &KalpaWindow) -> AddonViewKey {
    AddonViewKey {
        query: ui.get_addon_search_query().trim().to_string(),
        filter_mode: ui.get_filter_mode(),
        sort_mode: ui.get_sort_mode(),
    }
}

fn visible_addons(
    addons: &[AddonEntry],
    search_query: &str,
    filter_mode: i32,
    sort_mode: i32,
) -> Vec<AddonEntry> {
    let query = search_query.trim().to_ascii_lowercase();
    let mut rows = addons
        .iter()
        .filter(|addon| {
            if !query.is_empty() && !addon_matches_search(addon, &query) {
                return false;
            }

            match filter_mode {
                1 => !addon.is_library,
                2 => addon.is_library,
                3 => addon.favorite,
                4 => addon_has_update(addon),
                5 => addon_has_required_dependency_issue(addon),
                6 => addon.disabled,
                7 => addon_has_tag(addon, "testing"),
                8 => addon_has_tag(addon, "broken"),
                9 => addon_has_tag(addon, "essential"),
                10 => addon_has_tag(addon, "raid"),
                _ => true,
            }
        })
        .cloned()
        .collect::<Vec<_>>();

    sort_addons(&mut rows, sort_mode);
    rows
}

fn normalized_filter_mode(filter_mode: i32, counts: &AddonFilterCounts) -> i32 {
    match filter_mode {
        0..=2 => filter_mode,
        3 if counts.favorites > 0 => 3,
        4 if counts.outdated > 0 => 4,
        5 if counts.issues > 0 => 5,
        6 if counts.disabled > 0 => 6,
        7 if counts.testing > 0 => 7,
        8 if counts.broken > 0 => 8,
        9 if counts.essential > 0 => 9,
        10 if counts.raid > 0 => 10,
        _ => 0,
    }
}

fn addon_has_update(addon: &AddonEntry) -> bool {
    addon.state == 1 || addon.badge.as_str() == "Update"
}

fn addon_has_required_dependency_issue(addon: &AddonEntry) -> bool {
    (0..addon.required_dependencies.row_count())
        .filter_map(|index| addon.required_dependencies.row_data(index))
        .any(|dependency| dependency.missing || dependency.outdated)
}

fn addon_has_tag(addon: &AddonEntry, tag_id: &str) -> bool {
    (0..addon.tags.row_count())
        .filter_map(|index| addon.tags.row_data(index))
        .any(|tag| tag.id.as_str() == tag_id && tag.active)
}

fn addon_matches_search(addon: &AddonEntry, query: &str) -> bool {
    [
        addon.title.as_str(),
        addon.folder_name.as_str(),
        addon.author.as_str(),
        addon.addon_type.as_str(),
    ]
    .iter()
    .any(|value| value.to_ascii_lowercase().contains(query))
        || active_tag_labels(addon)
            .iter()
            .any(|tag| tag.to_ascii_lowercase().contains(query))
}

fn active_tag_labels(addon: &AddonEntry) -> Vec<String> {
    (0..addon.tags.row_count())
        .filter_map(|index| addon.tags.row_data(index))
        .filter(|tag| tag.active)
        .map(|tag| tag.id.to_string())
        .collect()
}

fn sort_addons(addons: &mut [AddonEntry], sort_mode: i32) {
    addons.sort_by(|left, right| {
        let by_name = || {
            left.title
                .to_ascii_lowercase()
                .cmp(&right.title.to_ascii_lowercase())
        };

        match sort_mode {
            1 => {
                let by_author = left
                    .author
                    .to_ascii_lowercase()
                    .cmp(&right.author.to_ascii_lowercase());
                by_author.then_with(by_name)
            }
            2 => date_sort_key(right.last_updated.as_str())
                .cmp(&date_sort_key(left.last_updated.as_str()))
                .then_with(by_name),
            3 => date_sort_key(right.installed_at.as_str())
                .cmp(&date_sort_key(left.installed_at.as_str()))
                .then_with(by_name),
            _ => by_name(),
        }
    });
}

fn date_sort_key(value: &str) -> i32 {
    let parts = value
        .split('/')
        .filter_map(|part| part.parse::<i32>().ok())
        .collect::<Vec<_>>();

    if parts.len() == 3 {
        parts[2] * 10_000 + parts[0] * 100 + parts[1]
    } else {
        0
    }
}

fn selected_visible_addon_folder(ui: &KalpaWindow) -> Option<String> {
    let index = ui.get_selected_index().max(0) as usize;
    ui.get_addons()
        .row_data(index)
        .map(|addon| addon.folder_name.to_string())
}

fn apply_discover_data(
    ui: &KalpaWindow,
    tab: i32,
    installed_ids: &BTreeSet<String>,
) -> Rc<VecModel<DiscoverEntry>> {
    let entries = discover_entries_for_tab(
        tab,
        installed_ids,
        ui.get_discover_query().as_str(),
        ui.get_discover_url_input().as_str(),
    );
    let selected_index = if entries.is_empty() {
        0
    } else {
        ui.get_selected_discover_index().max(0) as usize
    };
    let selected_index = selected_index.min(entries.len().saturating_sub(1));
    let model = Rc::new(VecModel::from(entries));
    ui.set_selected_discover_index(selected_index as i32);
    ui.set_discover_results(model.clone().into());
    model
}

fn discover_entries_for_tab(
    tab: i32,
    installed_ids: &BTreeSet<String>,
    query: &str,
    url_input: &str,
) -> Vec<DiscoverEntry> {
    match normalize_discover_tab(tab) {
        1 => popular_discover_entries(installed_ids),
        2 => category_discover_entries(installed_ids),
        3 => url_discover_entries(installed_ids, url_input),
        _ => filter_discover_entries(search_discover_entries(installed_ids), query),
    }
}

fn normalize_discover_tab(tab: i32) -> i32 {
    tab.clamp(0, 3)
}

fn filter_discover_entries(entries: Vec<DiscoverEntry>, query: &str) -> Vec<DiscoverEntry> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return entries;
    }

    entries
        .into_iter()
        .filter(|entry| {
            [
                entry.esoui_id.as_str(),
                entry.title.as_str(),
                entry.author.as_str(),
                entry.category.as_str(),
                entry.description.as_str(),
            ]
            .iter()
            .any(|value| value.to_ascii_lowercase().contains(&query))
        })
        .enumerate()
        .map(|(index, mut entry)| {
            entry.rank = index as i32 + 1;
            entry
        })
        .collect()
}

fn search_discover_entries(installed_ids: &BTreeSet<String>) -> Vec<DiscoverEntry> {
    vec![
        discover_entry(
            "3520",
            "Code's Combat Alerts",
            "@code65536",
            "Combat",
            "2.4.10",
            "1.9M",
            "84K",
            "3.6K",
            "2/9/2026",
            "6/21/2021",
            "a23f91c8d71e",
            "101048, 101049",
            "Encounter alerts, timers, interrupt prompts, and boss mechanic callouts for veteran content.",
            1,
            installed_ids,
        ),
        discover_entry(
            "1360",
            "CombatMetrics",
            "Solinur",
            "Combat",
            "1.7.7",
            "5.2M",
            "213K",
            "8.8K",
            "3/3/2026",
            "8/5/2014",
            "8df19ab412e0",
            "101048, 101049",
            "Combat log analysis with fight summaries, damage breakdowns, buffs, debuffs, and encounter data.",
            2,
            installed_ids,
        ),
        discover_entry(
            "3218",
            "CrutchAlerts",
            "Kyzeragon",
            "Combat",
            "2.14.0",
            "2.4M",
            "96K",
            "4.1K",
            "3/6/2026",
            "3/14/2020",
            "4bd802e91f02",
            "101048, 101049",
            "Advanced raid alerts and compact quality-of-life helpers for organized group content.",
            3,
            installed_ids,
        ),
        discover_entry(
            "2572",
            "Combat Metronome",
            "Darianopolis, barny",
            "Combat",
            "1.7.4",
            "1.1M",
            "42K",
            "2.7K",
            "3/1/2026",
            "4/10/2018",
            "f8225f4b6314",
            "101048, 101049",
            "GCD, weaving, and rotation timing display for players who want consistent combat rhythm.",
            4,
            installed_ids,
        ),
    ]
}

fn popular_discover_entries(installed_ids: &BTreeSet<String>) -> Vec<DiscoverEntry> {
    vec![
        discover_entry(
            "57",
            "HarvestMap",
            "Shinni",
            "Map",
            "3.19.2",
            "18.4M",
            "522K",
            "18.6K",
            "2/28/2026",
            "4/1/2014",
            "1f5ce7bd9a21",
            "101048, 101049",
            "Resource node, chest, fishing, and survey pins with account-wide map data.",
            1,
            installed_ids,
        ),
        discover_entry(
            "3170",
            "Wizard's Wardrobe",
            "Dolgubon",
            "Raid",
            "1.19.6",
            "7.7M",
            "286K",
            "9.3K",
            "2/27/2026",
            "9/12/2020",
            "67fd90c43ba1",
            "101048, 101049",
            "Build and gear-set management for trials, dungeons, arenas, and fast role swaps.",
            2,
            installed_ids,
        ),
        discover_entry(
            "3520",
            "Code's Combat Alerts",
            "@code65536",
            "Combat",
            "2.4.10",
            "1.9M",
            "84K",
            "3.6K",
            "2/9/2026",
            "6/21/2021",
            "a23f91c8d71e",
            "101048, 101049",
            "Encounter alerts, timers, interrupt prompts, and boss mechanic callouts for veteran content.",
            3,
            installed_ids,
        ),
        discover_entry(
            "1881",
            "Map Pins",
            "Hoft",
            "Map",
            "1.9.8",
            "8.1M",
            "311K",
            "11.2K",
            "2/12/2026",
            "7/22/2016",
            "0b85dd8841ad",
            "101048, 101049",
            "Map markers for lorebooks, skyshards, delves, dungeons, mundus stones, and other destinations.",
            4,
            installed_ids,
        ),
    ]
}

fn category_discover_entries(installed_ids: &BTreeSet<String>) -> Vec<DiscoverEntry> {
    vec![
        discover_entry(
            "1360",
            "CombatMetrics",
            "Solinur",
            "Combat",
            "1.7.7",
            "5.2M",
            "213K",
            "8.8K",
            "3/3/2026",
            "8/5/2014",
            "8df19ab412e0",
            "101048, 101049",
            "Combat log analysis with fight summaries, damage breakdowns, buffs, debuffs, and encounter data.",
            1,
            installed_ids,
        ),
        discover_entry(
            "1355",
            "RaidNotifier Updated",
            "Raid Tools Team",
            "Combat",
            "4.2.3",
            "3.8M",
            "119K",
            "6.4K",
            "2/25/2026",
            "10/13/2014",
            "d26173caae09",
            "101048, 101049",
            "Raid notifications and encounter warnings for group content.",
            2,
            installed_ids,
        ),
        discover_entry(
            "655",
            "Srendarr",
            "Phinix",
            "Buffs",
            "2.5.8",
            "6.6M",
            "147K",
            "7.1K",
            "1/30/2026",
            "5/19/2014",
            "433b9023ce4a",
            "101048, 101049",
            "Aura, buff, and debuff tracking with configurable displays.",
            3,
            installed_ids,
        ),
        discover_entry(
            "2463",
            "Cooldowns",
            "@g4rr3t (NA)",
            "Combat",
            "1.6.1",
            "930K",
            "36K",
            "2.1K",
            "1/4/2026",
            "1/22/2018",
            "c032c10f61d5",
            "101048, 101049",
            "Tracks important cooldown timers in compact combat widgets.",
            4,
            installed_ids,
        ),
    ]
}

fn url_discover_entries(installed_ids: &BTreeSet<String>, input: &str) -> Vec<DiscoverEntry> {
    let Some(esoui_id) = esoui_id_from_input(input) else {
        return Vec::new();
    };

    all_known_discover_entries(installed_ids)
        .into_iter()
        .find(|entry| entry.esoui_id.as_str() == esoui_id)
        .map(|mut entry| {
            entry.rank = 1;
            entry.description = format!(
                "Resolved from URL / ID input. {}",
                entry.description.as_str()
            )
            .into();
            vec![entry]
        })
        .unwrap_or_default()
}

fn all_known_discover_entries(installed_ids: &BTreeSet<String>) -> Vec<DiscoverEntry> {
    let mut entries = Vec::new();
    entries.extend(search_discover_entries(installed_ids));
    entries.extend(popular_discover_entries(installed_ids));
    entries.extend(category_discover_entries(installed_ids));

    let mut seen = BTreeSet::new();
    entries
        .into_iter()
        .filter(|entry| seen.insert(entry.esoui_id.to_string()))
        .collect()
}

fn esoui_id_from_input(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(info_index) = trimmed.to_ascii_lowercase().find("info") {
        let after_info = &trimmed[info_index + 4..];
        let id = after_info
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>();
        if !id.is_empty() {
            return Some(id);
        }
    }

    let digits = trimmed
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then_some(digits)
}

fn discover_entry(
    esoui_id: &str,
    title: &str,
    author: &str,
    category: &str,
    version: &str,
    downloads: &str,
    monthly_downloads: &str,
    favorites: &str,
    updated: &str,
    created: &str,
    md5: &str,
    compatibility: &str,
    description: &str,
    rank: i32,
    installed_ids: &BTreeSet<String>,
) -> DiscoverEntry {
    DiscoverEntry {
        esoui_id: esoui_id.into(),
        title: title.into(),
        author: author.into(),
        category: category.into(),
        version: version.into(),
        downloads: downloads.into(),
        monthly_downloads: monthly_downloads.into(),
        favorites: favorites.into(),
        updated: updated.into(),
        created: created.into(),
        md5: md5.into(),
        compatibility: compatibility.into(),
        description: description.into(),
        installed: installed_ids.contains(esoui_id),
        rank,
    }
}

fn dependency_model(dependencies: Vec<DependencyEntry>) -> ModelRc<DependencyEntry> {
    Rc::new(VecModel::from(dependencies)).into()
}

fn empty_dependency_model() -> ModelRc<DependencyEntry> {
    dependency_model(Vec::new())
}

fn dependency_entry(
    name: &str,
    version: &str,
    missing: bool,
    outdated: bool,
    install_action: bool,
) -> DependencyEntry {
    DependencyEntry {
        name: name.into(),
        version: version.into(),
        missing,
        outdated,
        install_action,
    }
}

fn addon_entry(
    title: &str,
    folder_name: &str,
    esoui_id: &str,
    author: &str,
    version: &str,
    api_version: &str,
    addon_type: &str,
    last_updated: &str,
    description: &str,
    favorite: bool,
    is_library: bool,
    disabled: bool,
    state: i32,
    badge: &str,
    badge_kind: i32,
    badge2: &str,
    badge2_kind: i32,
    badge3: &str,
    badge3_kind: i32,
) -> AddonEntry {
    AddonEntry {
        title: title.into(),
        meta: format!("{version}  \u{00b7} {author}").into(),
        folder_name: folder_name.into(),
        esoui_id: esoui_id.into(),
        author: author.into(),
        version: version.into(),
        api_version: api_version.into(),
        addon_type: addon_type.into(),
        last_updated: last_updated.into(),
        installed_at: last_updated.into(),
        description: description.into(),
        favorite,
        selected: false,
        is_library,
        disabled,
        state,
        badge: badge.into(),
        badge_kind,
        badge2: badge2.into(),
        badge2_kind,
        badge3: badge3.into(),
        badge3_kind,
        dependent_summary: "".into(),
        tags: tag_model(initial_tags(folder_name, favorite, state)),
        required_dependencies: empty_dependency_model(),
        optional_dependencies: empty_dependency_model(),
    }
}

fn wire_addon_filters(ui: &KalpaWindow, models: AddonModels) {
    let search_ui = ui.as_weak();
    let search_models = models.clone();
    ui.on_addon_search_edited(move |_| {
        if let Some(ui) = search_ui.upgrade() {
            apply_addon_view_if_key_changed(&ui, &search_models);
        }
    });

    let filter_ui = ui.as_weak();
    let filter_models = models.clone();
    ui.on_filter_selected(move |_| {
        if let Some(ui) = filter_ui.upgrade() {
            apply_addon_view_if_key_changed(&ui, &filter_models);
        }
    });

    let sort_ui = ui.as_weak();
    ui.on_sort_selected(move |_| {
        if let Some(ui) = sort_ui.upgrade() {
            apply_addon_view_if_key_changed(&ui, &models);
        }
    });
}

fn wire_header_actions(ui: &KalpaWindow, models: AddonModels) {
    let refresh_ui = ui.as_weak();
    ui.on_refresh_requested(move || {
        let Some(ui) = refresh_ui.upgrade() else {
            return;
        };

        let active_theme_id = apply_initial_theme(&ui);
        ui.set_active_theme_id(active_theme_id.into());

        if let Some(addons_root) = addons_source_root() {
            match real_addon_entries(&addons_root) {
                Ok(addons) if !addons.is_empty() => {
                    *models.all.borrow_mut() = addons;
                    apply_addon_view(&ui, &models);
                    ui.set_status_error_message("".into());
                }
                Ok(_) => {
                    ui.set_status_error_message("No addons were found in the configured AddOns folder.".into());
                }
                Err(error) => {
                    ui.set_status_error_message(error.into());
                }
            }
        } else {
            ui.set_status_error_message("AddOns folder was not found. Set KALPA_ADDONS_PATH or configure the ESO AddOns path.".into());
        }
    });
}

fn apply_initial_native_settings(ui: &KalpaWindow) {
    apply_native_settings(ui, &read_native_settings());
}

fn apply_native_settings(ui: &KalpaWindow, settings: &NativeSettings) {
    ui.set_settings_auto_update(settings.auto_update);
    ui.set_settings_warn_eso_running(settings.warn_eso_running);
    ui.set_settings_official_uploader(settings.official_uploader);
    ui.set_settings_auto_open_analysis(settings.auto_open_analysis);
    ui.set_settings_conflict_policy(settings.conflict_policy.clamp(0, 2));
}

fn native_settings_from_ui(ui: &KalpaWindow) -> NativeSettings {
    NativeSettings {
        auto_update: ui.get_settings_auto_update(),
        warn_eso_running: ui.get_settings_warn_eso_running(),
        official_uploader: ui.get_settings_official_uploader(),
        auto_open_analysis: ui.get_settings_auto_open_analysis(),
        conflict_policy: ui.get_settings_conflict_policy().clamp(0, 2),
    }
}

fn wire_settings_actions(ui: &KalpaWindow) {
    let settings_ui = ui.as_weak();
    ui.on_settings_changed(move || {
        let Some(ui) = settings_ui.upgrade() else {
            return;
        };
        persist_native_settings(&native_settings_from_ui(&ui));
    });
}

fn seed_initial_theme_draft(ui: &KalpaWindow, custom_themes: &[CatalogTheme]) {
    let active_id = ui.get_active_theme_id().to_string();
    let draft = theme_by_id(&active_id, custom_themes)
        .or_else(default_catalog_theme)
        .map(|theme| custom_theme_draft_from_base(&theme, "Custom"))
        .unwrap_or_else(fallback_custom_theme_draft);
    set_theme_draft(ui, &draft, true);
}

fn open_theme_draft_editor(ui: &KalpaWindow, draft: &CatalogTheme, is_new: bool) {
    set_theme_draft(ui, draft, is_new);
    apply_theme_selection(ui, &ThemeSelection::colors_only(draft.colors.clone()));
    ui.set_settings_open(true);
    ui.set_settings_tab(1);
    ui.set_settings_editor_open(true);
}

fn set_theme_draft(ui: &KalpaWindow, draft: &CatalogTheme, is_new: bool) {
    ui.set_draft_theme(theme_entry_from_catalog_theme(
        draft,
        0,
        0,
        0,
        String::new(),
    ));
    ui.set_draft_theme_id(draft.id.clone().into());
    ui.set_draft_theme_name(draft.name.clone().into());
    ui.set_draft_theme_description(draft.description.clone().into());
    ui.set_draft_theme_new(is_new);
    set_theme_draft_color_fields(ui, &draft.colors);
    set_theme_draft_contrast(ui, &draft.colors);
}

fn set_theme_draft_color_fields(ui: &KalpaWindow, colors: &ThemeSeed) {
    ui.set_draft_bg_base(colors.bg_base.clone().into());
    ui.set_draft_background(colors.background.clone().into());
    ui.set_draft_surface(colors.surface.clone().into());
    ui.set_draft_foreground(colors.foreground.clone().into());
    ui.set_draft_muted_foreground(colors.muted_foreground.clone().into());
    ui.set_draft_primary(colors.primary.clone().into());
    ui.set_draft_primary_foreground(colors.primary_foreground.clone().into());
    ui.set_draft_accent(colors.accent.clone().into());
    ui.set_draft_border(colors.border.clone().into());
    ui.set_draft_orb1(colors.orb1.clone().into());
    ui.set_draft_orb2(colors.orb2.clone().into());
    ui.set_draft_orb3(colors.orb3.clone().into());
}

fn draft_colors_from_ui(ui: &KalpaWindow, fallback: &ThemeSeed) -> ThemeSeed {
    ThemeSeed {
        bg_base: normalize_hex_color(&ui.get_draft_bg_base(), &fallback.bg_base),
        background: normalize_hex_color(&ui.get_draft_background(), &fallback.background),
        surface: normalize_hex_color(&ui.get_draft_surface(), &fallback.surface),
        foreground: normalize_hex_color(&ui.get_draft_foreground(), &fallback.foreground),
        muted_foreground: normalize_hex_color(
            &ui.get_draft_muted_foreground(),
            &fallback.muted_foreground,
        ),
        primary: normalize_hex_color(&ui.get_draft_primary(), &fallback.primary),
        primary_foreground: normalize_hex_color(
            &ui.get_draft_primary_foreground(),
            &fallback.primary_foreground,
        ),
        accent: normalize_hex_color(&ui.get_draft_accent(), &fallback.accent),
        border: normalize_hex_color(&ui.get_draft_border(), &fallback.border),
        orb1: normalize_hex_color(&ui.get_draft_orb1(), &fallback.orb1),
        orb2: normalize_hex_color(&ui.get_draft_orb2(), &fallback.orb2),
        orb3: normalize_hex_color(&ui.get_draft_orb3(), &fallback.orb3),
    }
}

fn normalize_hex_color(value: &str, fallback: &str) -> String {
    normalized_hex_color(value).unwrap_or_else(|| fallback.to_string())
}

fn normalized_hex_color(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let raw = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let expanded = if raw.len() == 3 && raw.chars().all(|ch| ch.is_ascii_hexdigit()) {
        raw.chars().flat_map(|ch| [ch, ch]).collect::<String>()
    } else if raw.len() == 6 && raw.chars().all(|ch| ch.is_ascii_hexdigit()) {
        raw.to_string()
    } else {
        return None;
    };

    Some(format!("#{}", expanded.to_ascii_uppercase()))
}

fn normalize_theme_seed(colors: &ThemeSeed) -> Option<ThemeSeed> {
    Some(ThemeSeed {
        bg_base: normalized_hex_color(&colors.bg_base)?,
        background: normalized_hex_color(&colors.background)?,
        surface: normalized_hex_color(&colors.surface)?,
        foreground: normalized_hex_color(&colors.foreground)?,
        muted_foreground: normalized_hex_color(&colors.muted_foreground)?,
        primary: normalized_hex_color(&colors.primary)?,
        primary_foreground: normalized_hex_color(&colors.primary_foreground)?,
        accent: normalized_hex_color(&colors.accent)?,
        border: normalized_hex_color(&colors.border)?,
        orb1: normalized_hex_color(&colors.orb1)?,
        orb2: normalized_hex_color(&colors.orb2)?,
        orb3: normalized_hex_color(&colors.orb3)?,
    })
}

fn set_theme_draft_contrast(ui: &KalpaWindow, colors: &ThemeSeed) {
    let checks = evaluate_theme_contrast(colors);
    let failing = checks
        .iter()
        .filter(|check| check.level == ContrastLevel::Fail)
        .count();
    let warning = if failing == 0 {
        String::new()
    } else {
        format!(
            "{} pair{} below the recommended contrast. Some text may be hard to read.",
            failing,
            if failing == 1 { "" } else { "s" }
        )
    };
    let entries = checks
        .into_iter()
        .map(|check| ContrastEntry {
            label: check.label.into(),
            ratio: format!("{:.2}:1", check.ratio).into(),
            level: match check.level {
                ContrastLevel::Fail => 0,
                ContrastLevel::Ok => 1,
                ContrastLevel::Great => 2,
            },
        })
        .collect::<Vec<_>>();

    ui.set_draft_contrast_checks(Rc::new(VecModel::from(entries)).into());
    ui.set_draft_contrast_warning(warning.into());
}

fn evaluate_theme_contrast(colors: &ThemeSeed) -> Vec<NativeContrastCheck> {
    vec![
        contrast_check(
            "Text on background",
            &colors.foreground,
            &colors.background,
            4.5,
            7.0,
        ),
        contrast_check(
            "Muted text on panels",
            &colors.muted_foreground,
            &colors.surface,
            4.5,
            7.0,
        ),
        contrast_check(
            "Primary as text",
            &colors.primary,
            &colors.background,
            4.5,
            7.0,
        ),
        contrast_check(
            "Accent on panels",
            &colors.accent,
            &colors.surface,
            3.0,
            4.5,
        ),
        contrast_check(
            "Label on primary button",
            &colors.primary_foreground,
            &colors.primary,
            4.5,
            7.0,
        ),
    ]
}

fn contrast_check(
    label: &'static str,
    foreground: &str,
    background: &str,
    min: f64,
    good: f64,
) -> NativeContrastCheck {
    let ratio = (contrast_ratio(foreground, background) * 100.0).round() / 100.0;
    let level = if ratio < min {
        ContrastLevel::Fail
    } else if ratio < good {
        ContrastLevel::Ok
    } else {
        ContrastLevel::Great
    };

    NativeContrastCheck {
        label,
        ratio,
        level,
    }
}

fn custom_theme_draft_from_base(base: &CatalogTheme, suffix: &str) -> CatalogTheme {
    CatalogTheme {
        id: new_custom_theme_id(),
        name: format!("{} {suffix}", base.name).trim().to_string(),
        category: "Custom".to_string(),
        description: if base.description.trim().is_empty() {
            "My custom theme.".to_string()
        } else {
            base.description.clone()
        },
        colors: base.colors.clone(),
        skin_id: None,
    }
}

fn fallback_custom_theme_draft() -> CatalogTheme {
    CatalogTheme {
        id: new_custom_theme_id(),
        name: "New Custom Theme".to_string(),
        category: "Custom".to_string(),
        description: "My custom theme.".to_string(),
        colors: ThemeSeed {
            bg_base: "#02050a".to_string(),
            background: "#07111f".to_string(),
            surface: "#0f1a2e".to_string(),
            foreground: "#f4f7fb".to_string(),
            muted_foreground: "#96a2b7".to_string(),
            primary: "#d7a84a".to_string(),
            primary_foreground: "#120d05".to_string(),
            accent: "#38bdf8".to_string(),
            border: "#21324d".to_string(),
            orb1: "#d7a84a".to_string(),
            orb2: "#38bdf8".to_string(),
            orb3: "#8b5cf6".to_string(),
        },
        skin_id: None,
    }
}

fn new_custom_theme_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("custom-{millis:x}")
}

fn upsert_custom_theme(custom_themes: &mut Vec<CatalogTheme>, mut theme: CatalogTheme) {
    theme.category = "Custom".to_string();
    theme.skin_id = None;
    match custom_themes
        .iter()
        .position(|existing| existing.id == theme.id)
    {
        Some(index) => custom_themes[index] = theme,
        None => custom_themes.push(theme),
    }
}

fn parse_imported_custom_theme(json: &str) -> Option<CatalogTheme> {
    serde_json::from_str::<CatalogTheme>(json)
        .ok()
        .and_then(|mut theme| {
            theme.colors = normalize_theme_seed(&theme.colors)?;
            if theme.id.trim().is_empty() {
                theme.id = new_custom_theme_id();
            }
            if theme.name.trim().is_empty() {
                theme.name = "Imported Theme".to_string();
            }
            if theme.description.trim().is_empty() {
                theme.description = "Imported custom theme.".to_string();
            }
            theme.category = "Custom".to_string();
            theme.skin_id = None;
            Some(theme)
        })
}

fn restore_active_theme_preview(ui: &KalpaWindow, custom_themes: &[CatalogTheme]) {
    let active_id = ui.get_active_theme_id().to_string();
    if let Some(selection) = theme_selection_by_id(&active_id, custom_themes) {
        apply_theme_selection(ui, &selection);
        return;
    }

    if let Some(fallback) = default_catalog_theme() {
        apply_theme_selection(
            ui,
            &ThemeSelection::with_skin(fallback.colors, fallback.skin_id.as_deref()),
        );
        ui.set_active_theme_id(fallback.id.clone().into());
        persist_active_theme_id(&fallback.id);
    }
}

fn wire_theme_actions(ui: &KalpaWindow, custom_themes: Rc<RefCell<Vec<CatalogTheme>>>) {
    let current_draft = Rc::new(RefCell::new(
        default_catalog_theme()
            .map(|theme| custom_theme_draft_from_base(&theme, "Custom"))
            .unwrap_or_else(fallback_custom_theme_draft),
    ));

    let theme_ui = ui.as_weak();
    let selected_custom_themes = custom_themes.clone();
    ui.on_theme_selected(move |theme_id| {
        let Some(ui) = theme_ui.upgrade() else {
            return;
        };

        let theme_id = theme_id.to_string();
        match theme_selection_by_id(&theme_id, &selected_custom_themes.borrow()) {
            Some(selection) => {
                apply_theme_selection(&ui, &selection);
                ui.set_active_theme_id(theme_id.clone().into());
                persist_active_theme_id(&theme_id);
            }
            None => {
                ui.set_status_error_message(format!("Theme not found: {theme_id}").into());
            }
        }
    });

    let create_ui = ui.as_weak();
    let create_custom_themes = custom_themes.clone();
    let create_draft = current_draft.clone();
    ui.on_theme_create(move || {
        let Some(ui) = create_ui.upgrade() else {
            return;
        };

        let active_id = ui.get_active_theme_id().to_string();
        let custom_themes = create_custom_themes.borrow();
        let Some(base) = theme_by_id(&active_id, &custom_themes).or_else(default_catalog_theme)
        else {
            return;
        };
        let draft = custom_theme_draft_from_base(&base, "Custom");
        *create_draft.borrow_mut() = draft.clone();
        open_theme_draft_editor(&ui, &draft, true);
    });

    let fork_ui = ui.as_weak();
    let fork_custom_themes = custom_themes.clone();
    let fork_draft = current_draft.clone();
    ui.on_theme_fork(move |theme_id| {
        let Some(ui) = fork_ui.upgrade() else {
            return;
        };

        let theme_id = theme_id.to_string();
        let custom_themes = fork_custom_themes.borrow();
        let Some(base) = theme_by_id(&theme_id, &custom_themes) else {
            ui.set_status_error_message(format!("Theme not found: {theme_id}").into());
            return;
        };
        let draft = custom_theme_draft_from_base(&base, "Copy");
        *fork_draft.borrow_mut() = draft.clone();
        open_theme_draft_editor(&ui, &draft, true);
    });

    let edit_ui = ui.as_weak();
    let edit_custom_themes = custom_themes.clone();
    let edit_draft = current_draft.clone();
    ui.on_theme_edit(move |theme_id| {
        let Some(ui) = edit_ui.upgrade() else {
            return;
        };

        let theme_id = theme_id.to_string();
        let Some(theme) = edit_custom_themes
            .borrow()
            .iter()
            .find(|theme| theme.id == theme_id)
            .cloned()
        else {
            ui.set_status_error_message("Only saved custom themes can be edited.".into());
            return;
        };
        *edit_draft.borrow_mut() = theme.clone();
        open_theme_draft_editor(&ui, &theme, false);
    });

    let save_ui = ui.as_weak();
    let save_custom_themes = custom_themes.clone();
    let save_draft = current_draft.clone();
    ui.on_theme_save(move |name| {
        let Some(ui) = save_ui.upgrade() else {
            return;
        };

        let normalized_name = name.trim();
        if normalized_name.is_empty() {
            ui.set_status_error_message("Give your theme a name.".into());
            return;
        }

        let mut draft = save_draft.borrow().clone();
        draft.name = normalized_name.to_string();
        draft.category = "Custom".to_string();
        draft.skin_id = None;
        draft.colors = draft_colors_from_ui(&ui, &draft.colors);
        set_theme_draft_color_fields(&ui, &draft.colors);
        set_theme_draft_contrast(&ui, &draft.colors);

        let mut custom_themes = save_custom_themes.borrow_mut();
        upsert_custom_theme(&mut custom_themes, draft.clone());
        persist_custom_themes(&custom_themes);
        set_theme_gallery(&ui, &custom_themes);
        set_theme_draft(&ui, &draft, false);
        *save_draft.borrow_mut() = draft.clone();
        apply_theme_selection(&ui, &ThemeSelection::colors_only(draft.colors.clone()));
        ui.set_active_theme_id(draft.id.clone().into());
        persist_active_theme_id(&draft.id);
        ui.set_settings_editor_open(false);
    });

    let preview_ui = ui.as_weak();
    let preview_draft = current_draft.clone();
    ui.on_theme_draft_updated(move || {
        let Some(ui) = preview_ui.upgrade() else {
            return;
        };

        let mut draft = preview_draft.borrow().clone();
        let next_colors = draft_colors_from_ui(&ui, &draft.colors);
        if next_colors == draft.colors {
            return;
        }
        draft.colors = next_colors;
        *preview_draft.borrow_mut() = draft.clone();
        ui.set_draft_theme(theme_entry_from_catalog_theme(
            &draft,
            0,
            0,
            0,
            String::new(),
        ));
        set_theme_draft_contrast(&ui, &draft.colors);
        apply_theme_selection(&ui, &ThemeSelection::colors_only(draft.colors));
    });

    let delete_ui = ui.as_weak();
    let delete_custom_themes = custom_themes.clone();
    ui.on_theme_delete(move |theme_id| {
        let Some(ui) = delete_ui.upgrade() else {
            return;
        };

        let theme_id = theme_id.to_string();
        let mut custom_themes = delete_custom_themes.borrow_mut();
        custom_themes.retain(|theme| theme.id != theme_id);
        persist_custom_themes(&custom_themes);
        set_theme_gallery(&ui, &custom_themes);
        ui.set_settings_editor_open(false);

        if ui.get_active_theme_id().as_str() == theme_id {
            if let Some(fallback) = default_catalog_theme() {
                apply_theme_selection(
                    &ui,
                    &ThemeSelection::with_skin(fallback.colors, fallback.skin_id.as_deref()),
                );
                ui.set_active_theme_id(fallback.id.clone().into());
                persist_active_theme_id(&fallback.id);
            }
        }
    });

    let cancel_ui = ui.as_weak();
    let cancel_custom_themes = custom_themes.clone();
    ui.on_theme_editor_cancelled(move || {
        let Some(ui) = cancel_ui.upgrade() else {
            return;
        };

        restore_active_theme_preview(&ui, &cancel_custom_themes.borrow());
        ui.set_settings_editor_open(false);
    });

    let export_custom_themes = custom_themes.clone();
    ui.on_theme_export(move |theme_id| {
        let theme_id = theme_id.to_string();
        let Some(theme) = theme_by_id(&theme_id, &export_custom_themes.borrow()) else {
            return;
        };
        let Ok(json) = serde_json::to_string_pretty(&theme) else {
            return;
        };
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(json);
        }
    });

    let import_ui = ui.as_weak();
    let import_custom_themes = custom_themes;
    ui.on_theme_import(move || {
        let Some(ui) = import_ui.upgrade() else {
            return;
        };
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            ui.set_status_error_message("Could not open the system clipboard.".into());
            return;
        };
        let Ok(text) = clipboard.get_text() else {
            ui.set_status_error_message("Clipboard does not contain theme JSON.".into());
            return;
        };
        let Some(theme) = parse_imported_custom_theme(&text) else {
            ui.set_status_error_message("Clipboard does not contain a valid theme.".into());
            return;
        };

        let mut custom_themes = import_custom_themes.borrow_mut();
        upsert_custom_theme(&mut custom_themes, theme.clone());
        persist_custom_themes(&custom_themes);
        set_theme_gallery(&ui, &custom_themes);
        apply_theme_selection(&ui, &ThemeSelection::colors_only(theme.colors.clone()));
        ui.set_active_theme_id(theme.id.clone().into());
        persist_active_theme_id(&theme.id);
        set_theme_draft(&ui, &theme, false);
    });
}

fn wire_batch_actions(ui: &KalpaWindow, models: AddonModels) {
    let toggle_ui = ui.as_weak();
    let toggle_models = models.clone();
    ui.on_addon_selection_toggled(move |index| {
        let Some(ui) = toggle_ui.upgrade() else {
            return;
        };
        let Some(addon) = toggle_models.visible.row_data(index.max(0) as usize) else {
            return;
        };

        let folder_name = addon.folder_name.to_string();
        with_master_addon_mut(&toggle_models, &folder_name, |entry| {
            entry.selected = !entry.selected;
        });
        apply_addon_view(&ui, &toggle_models);
    });

    let clear_ui = ui.as_weak();
    let clear_models = models.clone();
    ui.on_batch_clear(move || {
        if let Some(ui) = clear_ui.upgrade() {
            for addon in clear_models.all.borrow_mut().iter_mut() {
                addon.selected = false;
            }
            apply_addon_view(&ui, &clear_models);
        }
    });

    let disable_ui = ui.as_weak();
    let disable_models = models.clone();
    ui.on_batch_disable(move || {
        if let Some(ui) = disable_ui.upgrade() {
            for addon in disable_models
                .all
                .borrow_mut()
                .iter_mut()
                .filter(|addon| addon.selected)
            {
                addon.disabled = true;
            }
            apply_addon_view(&ui, &disable_models);
        }
    });

    let tag_ui = ui.as_weak();
    let tag_models = models.clone();
    ui.on_batch_tag(move || {
        if let Some(ui) = tag_ui.upgrade() {
            for addon in tag_models
                .all
                .borrow_mut()
                .iter_mut()
                .filter(|addon| addon.selected)
            {
                addon.tags = set_tag_active(&addon.tags, "testing", true);
            }
            apply_addon_view(&ui, &tag_models);
        }
    });

    let update_ui = ui.as_weak();
    let update_models = models.clone();
    ui.on_batch_update(move || {
        if let Some(ui) = update_ui.upgrade() {
            for addon in update_models
                .all
                .borrow_mut()
                .iter_mut()
                .filter(|addon| addon.selected)
            {
                addon.state = 1;
                addon.badge = "Update".into();
                addon.badge_kind = 1;
            }
            apply_addon_view(&ui, &update_models);
        }
    });

    let remove_ui = ui.as_weak();
    ui.on_batch_remove(move || {
        if let Some(ui) = remove_ui.upgrade() {
            models.all.borrow_mut().retain(|addon| !addon.selected);
            apply_addon_view(&ui, &models);
        }
    });
}

fn wire_context_actions(ui: &KalpaWindow, models: AddonModels) {
    let folder_models = models.clone();
    ui.on_addon_context_open_folder(move |index| {
        let Some(addon) = folder_models.visible.row_data(index.max(0) as usize) else {
            return;
        };
        if let Some(addons_root) = addons_source_root() {
            open_path(&addon_disk_path(
                &addons_root,
                addon.folder_name.as_str(),
                addon.disabled,
            ));
        }
    });

    let esoui_models = models.clone();
    ui.on_addon_context_open_esoui(move |index| {
        let Some(addon) = esoui_models.visible.row_data(index.max(0) as usize) else {
            return;
        };
        if !addon.esoui_id.is_empty() {
            open_url(&format!(
                "https://www.esoui.com/downloads/info{}",
                addon.esoui_id.as_str()
            ));
        }
    });

    let favorite_ui = ui.as_weak();
    let favorite_models = models.clone();
    ui.on_addon_context_favorite(move |index| {
        let Some(ui) = favorite_ui.upgrade() else {
            return;
        };
        let Some(mut addon) = favorite_models.visible.row_data(index.max(0) as usize) else {
            return;
        };
        let folder_name = addon.folder_name.to_string();
        let next_favorite = !addon.favorite;
        addon.favorite = next_favorite;
        addon.tags = set_tag_active(&addon.tags, "favorite", next_favorite);
        update_master_addon(&favorite_models, &folder_name, addon);
        apply_addon_view(&ui, &favorite_models);
    });

    let disable_ui = ui.as_weak();
    let disable_models = models.clone();
    ui.on_addon_context_toggle_disable(move |index| {
        let Some(ui) = disable_ui.upgrade() else {
            return;
        };
        let Some(mut addon) = disable_models.visible.row_data(index.max(0) as usize) else {
            return;
        };
        let folder_name = addon.folder_name.to_string();
        addon.disabled = !addon.disabled;
        update_master_addon(&disable_models, &folder_name, addon);
        apply_addon_view(&ui, &disable_models);
    });

    let remove_ui = ui.as_weak();
    ui.on_addon_context_remove(move |index| {
        let Some(ui) = remove_ui.upgrade() else {
            return;
        };
        let Some(addon) = models.visible.row_data(index.max(0) as usize) else {
            return;
        };
        let folder_name = addon.folder_name.to_string();
        remove_master_addon(&models, &folder_name);
        apply_addon_view(&ui, &models);
    });
}

fn set_batch_state(ui: &KalpaWindow, models: &AddonModels) {
    let selected_count = models
        .all
        .borrow()
        .iter()
        .filter(|addon| addon.selected)
        .count() as i32;
    ui.set_selected_addon_count(selected_count);
    ui.set_batch_active(selected_count > 0);
}

fn with_master_addon_mut(
    models: &AddonModels,
    folder_name: &str,
    update: impl FnOnce(&mut AddonEntry),
) {
    if let Some(addon) = models
        .all
        .borrow_mut()
        .iter_mut()
        .find(|entry| entry.folder_name.as_str() == folder_name)
    {
        update(addon);
    }
}

fn wire_tag_editor(ui: &KalpaWindow, models: AddonModels) {
    let tag_ui = ui.as_weak();
    let tag_models = models.clone();
    ui.on_tag_clicked(move |tag_id| {
        let Some(ui) = tag_ui.upgrade() else {
            return;
        };

        let index = ui.get_selected_index().max(0) as usize;
        let Some(mut addon) = tag_models.visible.row_data(index) else {
            return;
        };

        let next_tags = toggled_tags(&addon.tags, tag_id.as_str());
        addon.favorite = tag_is_active(&next_tags, "favorite");
        addon.tags = tag_model_from_entries(next_tags);
        let folder_name = addon.folder_name.to_string();
        tag_models.visible.set_row_data(index, addon.clone());
        update_master_addon(&tag_models, &folder_name, addon);
        apply_addon_view(&ui, &tag_models);
    });

    let add_ui = ui.as_weak();
    let add_models = models.clone();
    ui.on_add_tag_clicked(move || {
        let Some(ui) = add_ui.upgrade() else {
            return;
        };

        let index = ui.get_selected_index().max(0) as usize;
        let Some(mut addon) = add_models.visible.row_data(index) else {
            return;
        };
        let Some(next_tags) = add_next_preset_tag(&addon.tags) else {
            return;
        };

        addon.favorite = tag_model_has_active(&next_tags, "favorite");
        addon.tags = next_tags;
        let folder_name = addon.folder_name.to_string();
        add_models.visible.set_row_data(index, addon.clone());
        update_master_addon(&add_models, &folder_name, addon);
        apply_addon_view(&ui, &add_models);
    });

    let custom_ui = ui.as_weak();
    let custom_models = models.clone();
    ui.on_custom_tag_submitted(move |raw_tag| {
        let Some(ui) = custom_ui.upgrade() else {
            return;
        };

        let index = ui.get_selected_index().max(0) as usize;
        let Some(mut addon) = custom_models.visible.row_data(index) else {
            return;
        };
        let Some(next_tags) = add_custom_tag(&addon.tags, raw_tag.as_str()) else {
            return;
        };

        addon.favorite = tag_model_has_active(&next_tags, "favorite");
        addon.tags = next_tags;
        let folder_name = addon.folder_name.to_string();
        custom_models.visible.set_row_data(index, addon.clone());
        update_master_addon(&custom_models, &folder_name, addon);
        ui.set_custom_tag_draft("".into());
        apply_addon_view(&ui, &custom_models);
    });
}

fn wire_detail_actions(ui: &KalpaWindow, models: AddonModels) {
    ui.on_open_esoui(move |esoui_id| {
        if !esoui_id.is_empty() {
            open_url(&format!(
                "https://www.esoui.com/downloads/info{}",
                esoui_id.as_str()
            ));
        }
    });

    let toggle_ui = ui.as_weak();
    let toggle_models = models.clone();
    ui.on_toggle_addon_disabled(move || {
        let Some(ui) = toggle_ui.upgrade() else {
            return;
        };

        let index = ui.get_selected_index().max(0) as usize;
        let Some(mut addon) = toggle_models.visible.row_data(index) else {
            return;
        };

        addon.disabled = !addon.disabled;
        let folder_name = addon.folder_name.to_string();
        toggle_models.visible.set_row_data(index, addon.clone());
        update_master_addon(&toggle_models, &folder_name, addon);
        apply_addon_view(&ui, &toggle_models);
    });

    let remove_ui = ui.as_weak();
    let remove_models = models.clone();
    ui.on_remove_addon(move || {
        let Some(ui) = remove_ui.upgrade() else {
            return;
        };

        let index = ui.get_selected_index().max(0) as usize;
        let Some(addon) = remove_models.visible.row_data(index) else {
            return;
        };

        let folder_name = addon.folder_name.to_string();
        remove_master_addon(&remove_models, &folder_name);
        apply_addon_view(&ui, &remove_models);
    });

    let install_ui = ui.as_weak();
    let install_models = models.clone();
    ui.on_install_dependency(move |name, optional| {
        if let Some(ui) = install_ui.upgrade() {
            update_selected_dependency(&ui, &install_models, name.as_str(), optional, true);
        }
    });

    let remove_dep_ui = ui.as_weak();
    ui.on_remove_dependency(move |name, optional| {
        if let Some(ui) = remove_dep_ui.upgrade() {
            update_selected_dependency(&ui, &models, name.as_str(), optional, false);
        }
    });
}

fn wire_discover(
    ui: &KalpaWindow,
    discover_model: Rc<RefCell<Rc<VecModel<DiscoverEntry>>>>,
    installed_ids: Rc<RefCell<BTreeSet<String>>>,
) {
    let tab_ui = ui.as_weak();
    let tab_model = discover_model.clone();
    let tab_installed_ids = installed_ids.clone();
    ui.on_discover_tab_selected(move |tab| {
        let Some(ui) = tab_ui.upgrade() else {
            return;
        };

        let model = apply_discover_data(&ui, tab, &tab_installed_ids.borrow());
        *tab_model.borrow_mut() = model;
    });

    let query_ui = ui.as_weak();
    let query_model = discover_model.clone();
    let query_installed_ids = installed_ids.clone();
    ui.on_discover_query_edited(move |_| {
        let Some(ui) = query_ui.upgrade() else {
            return;
        };

        if ui.get_discover_tab() != 0 {
            return;
        }

        let model = apply_discover_data(&ui, 0, &query_installed_ids.borrow());
        *query_model.borrow_mut() = model;
    });

    let url_ui = ui.as_weak();
    let url_model = discover_model.clone();
    let url_installed_ids = installed_ids.clone();
    ui.on_discover_url_edited(move |_| {
        let Some(ui) = url_ui.upgrade() else {
            return;
        };

        if ui.get_discover_tab() != 3 {
            return;
        }

        let model = apply_discover_data(&ui, 3, &url_installed_ids.borrow());
        *url_model.borrow_mut() = model;
    });

    let selected_ui = ui.as_weak();
    let selected_model = discover_model.clone();
    ui.on_discover_selected(move |index| {
        let Some(ui) = selected_ui.upgrade() else {
            return;
        };

        let row_count = selected_model.borrow().row_count();
        if row_count == 0 {
            ui.set_selected_discover_index(0);
            return;
        }

        let next_index = (index.max(0) as usize).min(row_count.saturating_sub(1));
        ui.set_selected_discover_index(next_index as i32);
    });

    let install_model = discover_model.clone();
    let install_ids = installed_ids;
    ui.on_discover_install(move |index| {
        let model = install_model.borrow();
        let Some(entry) = model.row_data(index.max(0) as usize) else {
            return;
        };

        let esoui_id = entry.esoui_id.to_string();
        install_ids.borrow_mut().insert(esoui_id.clone());
        mark_discover_installed(&model, &esoui_id);
    });

    ui.on_discover_open_esoui(move |esoui_id| {
        if !esoui_id.is_empty() {
            open_url(&format!(
                "https://www.esoui.com/downloads/info{}",
                esoui_id.as_str()
            ));
        }
    });
}

fn mark_discover_installed(discover_model: &Rc<VecModel<DiscoverEntry>>, esoui_id: &str) {
    for index in 0..discover_model.row_count() {
        let Some(mut entry) = discover_model.row_data(index) else {
            continue;
        };

        if entry.esoui_id.as_str() == esoui_id {
            entry.installed = true;
            discover_model.set_row_data(index, entry);
        }
    }
}

fn update_selected_dependency(
    ui: &KalpaWindow,
    models: &AddonModels,
    dependency_name: &str,
    optional: bool,
    install: bool,
) {
    let index = ui.get_selected_index().max(0) as usize;
    let Some(mut addon) = models.visible.row_data(index) else {
        return;
    };

    if optional {
        addon.optional_dependencies =
            updated_dependency_model(&addon.optional_dependencies, dependency_name, install);
    } else {
        addon.required_dependencies =
            updated_dependency_model(&addon.required_dependencies, dependency_name, install);
    }

    let folder_name = addon.folder_name.to_string();
    models.visible.set_row_data(index, addon.clone());
    update_master_addon(models, &folder_name, addon);
    apply_addon_view(ui, models);
}

fn update_master_addon(models: &AddonModels, folder_name: &str, addon: AddonEntry) {
    if let Some(existing) = models
        .all
        .borrow_mut()
        .iter_mut()
        .find(|entry| entry.folder_name.as_str() == folder_name)
    {
        *existing = addon;
    }
}

fn remove_master_addon(models: &AddonModels, folder_name: &str) {
    models
        .all
        .borrow_mut()
        .retain(|addon| addon.folder_name.as_str() != folder_name);
}

fn updated_dependency_model(
    dependencies: &ModelRc<DependencyEntry>,
    dependency_name: &str,
    install: bool,
) -> ModelRc<DependencyEntry> {
    let next = (0..dependencies.row_count())
        .filter_map(|index| dependencies.row_data(index))
        .map(|mut dependency| {
            if dependency.name.as_str() == dependency_name {
                dependency.missing = !install;
                dependency.outdated = false;
                dependency.install_action = !install;
            }
            dependency
        })
        .collect::<Vec<_>>();

    dependency_model(next)
}

fn initial_tags(folder_name: &str, favorite: bool, state: i32) -> Vec<&'static str> {
    let mut tags = Vec::new();
    if favorite {
        tags.push("favorite");
    }
    if folder_name == "CombatMetrics" || folder_name == "WizardsWardrobe" {
        tags.push("raid");
    }
    if state == 2 {
        tags.push("broken");
    }
    tags
}

fn tag_model(active_tags: Vec<&str>) -> ModelRc<TagEntry> {
    let active = active_tags
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    tag_model_from_entries(preset_tag_entries(&active))
}

fn tag_model_from_entries(tags: Vec<TagEntry>) -> ModelRc<TagEntry> {
    Rc::new(VecModel::from(tags)).into()
}

fn preset_tag_entries(active_tags: &[String]) -> Vec<TagEntry> {
    PRESET_TAGS
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let active = active_tags.iter().any(|tag| tag == id);
            TagEntry {
                id: (*id).into(),
                label: tag_label(id, active).into(),
                kind: index as i32,
                active,
                preset: true,
            }
        })
        .collect()
}

fn toggled_tags(tags: &ModelRc<TagEntry>, tag_id: &str) -> Vec<TagEntry> {
    (0..tags.row_count())
        .filter_map(|index| tags.row_data(index))
        .filter_map(|mut tag| {
            if !tag.preset && tag.id.as_str() == tag_id {
                return None;
            }
            if tag.id.as_str() == tag_id {
                tag.active = !tag.active;
            }
            tag.label = tag_label(tag.id.as_str(), tag.active).into();
            Some(tag)
        })
        .collect()
}

fn set_tag_active(tags: &ModelRc<TagEntry>, tag_id: &str, active: bool) -> ModelRc<TagEntry> {
    let next = (0..tags.row_count())
        .filter_map(|index| tags.row_data(index))
        .map(|mut tag| {
            if tag.id.as_str() == tag_id {
                tag.active = active;
            }
            tag.label = tag_label(tag.id.as_str(), tag.active).into();
            tag
        })
        .collect::<Vec<_>>();

    tag_model_from_entries(next)
}

fn add_next_preset_tag(tags: &ModelRc<TagEntry>) -> Option<ModelRc<TagEntry>> {
    PRESET_TAGS
        .iter()
        .skip(1)
        .find(|tag_id| !tag_model_has_active(tags, tag_id))
        .map(|tag_id| set_tag_active(tags, tag_id, true))
}

fn add_custom_tag(tags: &ModelRc<TagEntry>, raw_tag: &str) -> Option<ModelRc<TagEntry>> {
    let tag_id = sanitize_custom_tag(raw_tag)?;
    let mut next = (0..tags.row_count())
        .filter_map(|index| tags.row_data(index))
        .collect::<Vec<_>>();

    if PRESET_TAGS.iter().any(|preset| *preset == tag_id) {
        return Some(set_tag_active(tags, &tag_id, true));
    }

    if next.iter().any(|tag| tag.id.as_str() == tag_id) {
        return None;
    }

    next.push(TagEntry {
        id: tag_id.as_str().into(),
        label: tag_id.as_str().into(),
        kind: 5,
        active: true,
        preset: false,
    });
    Some(tag_model_from_entries(next))
}

fn sanitize_custom_tag(raw_tag: &str) -> Option<String> {
    let normalized = raw_tag
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn tag_model_has_active(tags: &ModelRc<TagEntry>, tag_id: &str) -> bool {
    (0..tags.row_count())
        .filter_map(|index| tags.row_data(index))
        .any(|tag| tag.id.as_str() == tag_id && tag.active)
}

fn tag_is_active(tags: &[TagEntry], tag_id: &str) -> bool {
    tags.iter()
        .any(|tag| tag.id.as_str() == tag_id && tag.active)
}

fn tag_label(tag_id: &str, active: bool) -> String {
    if tag_id == "favorite" {
        let star = if active { '\u{2605}' } else { '\u{2606}' };
        format!("{star} favorite")
    } else {
        tag_id.to_string()
    }
}

const PRESET_TAGS: [&str; 5] = ["favorite", "testing", "broken", "essential", "raid"];

fn wire_window_controls(ui: &KalpaWindow) {
    let minimize_ui = ui.as_weak();
    ui.on_minimize_requested(move || {
        if let Some(ui) = minimize_ui.upgrade() {
            ui.window().set_minimized(true);
        }
    });

    let maximize_ui = ui.as_weak();
    ui.on_maximize_requested(move || {
        if let Some(ui) = maximize_ui.upgrade() {
            let next = !ui.window().is_maximized();
            ui.window().set_maximized(next);
        }
    });

    let close_ui = ui.as_weak();
    ui.on_close_requested(move || {
        if let Some(ui) = close_ui.upgrade() {
            let _ = ui.hide();
        }
        let _ = slint::quit_event_loop();
    });
}

fn wire_file_browser(ui: &KalpaWindow) {
    let selected_ui = ui.as_weak();
    ui.on_addon_selected(move |_| {
        if let Some(ui) = selected_ui.upgrade() {
            refresh_file_browser(&ui);
        }
    });

    let file_ui = ui.as_weak();
    ui.on_file_selected(move |relative_path| {
        if let Some(ui) = file_ui.upgrade() {
            let folder_name = selected_addon_folder(&ui);
            open_file_in_editor(&ui, &folder_name, relative_path.as_str());
        }
    });

    let folder_toggle_ui = ui.as_weak();
    ui.on_folder_toggled(move |relative_path| {
        if let Some(ui) = folder_toggle_ui.upgrade() {
            let folder_name = selected_addon_folder(&ui);
            toggle_collapsed_file_folder(&folder_name, relative_path.as_str());
            refresh_file_browser(&ui);
        }
    });

    let save_ui = ui.as_weak();
    ui.on_save_file(move |relative_path, content| {
        if let Some(ui) = save_ui.upgrade() {
            save_file_from_editor(&ui, relative_path.as_str(), content.as_str());
        }
    });

    let edit_ui = ui.as_weak();
    ui.on_file_content_edited(move |content| {
        if let Some(ui) = edit_ui.upgrade() {
            ui.set_editor_line_numbers(line_numbers_for_content(content.as_str()).into());
        }
    });

    let rescan_ui = ui.as_weak();
    ui.on_rescan_files(move || {
        if let Some(ui) = rescan_ui.upgrade() {
            refresh_file_browser(&ui);
        }
    });

    let backups_ui = ui.as_weak();
    ui.on_load_edit_backups(move || {
        if let Some(ui) = backups_ui.upgrade() {
            refresh_edit_backups(&ui);
        }
    });

    let restore_ui = ui.as_weak();
    ui.on_restore_edit_backup(move |index| {
        let Some(ui) = restore_ui.upgrade() else {
            return;
        };
        let backups = ui.get_edit_backups();
        let Some(backup) = backups.row_data(index.max(0) as usize) else {
            return;
        };
        let Some(addons_root) = addons_source_root() else {
            ui.set_editor_error(true);
            ui.set_editor_message("AddOns folder was not found.".into());
            return;
        };
        let folder_name = selected_addon_folder(&ui);
        match restore_edit_backup_file(&addons_root, &folder_name, &backup) {
            Ok(()) => {
                refresh_file_browser(&ui);
                refresh_edit_backups(&ui);
                open_file_in_editor(&ui, &folder_name, backup.relative_path.as_str());
                ui.set_editor_message(format!("Restored {}", backup.relative_path.as_str()).into());
            }
            Err(error) => {
                ui.set_editor_error(true);
                ui.set_editor_message(format!("Restore failed: {error}").into());
            }
        }
    });

    let folder_ui = ui.as_weak();
    ui.on_open_folder(move || {
        if let Some(ui) = folder_ui.upgrade() {
            if let Some(addons_root) = addons_source_root() {
                if let Some(addon) = selected_addon(&ui) {
                    open_path(&addon_disk_path(
                        &addons_root,
                        addon.folder_name.as_str(),
                        addon.disabled,
                    ));
                }
            }
        }
    });

    let external_ui = ui.as_weak();
    ui.on_open_external(move |relative_path| {
        if let Some(ui) = external_ui.upgrade() {
            if let Some(addons_root) = addons_source_root() {
                let folder_name = selected_addon_folder(&ui);
                if let Ok(path) =
                    addon_file_path(&addons_root, &folder_name, relative_path.as_str())
                {
                    open_path(&path);
                }
            }
        }
    });
}

fn refresh_file_browser(ui: &KalpaWindow) {
    let folder_name = selected_addon_folder(ui);
    let source = addons_source_root();
    let files = source
        .as_deref()
        .and_then(|addons_root| real_file_entries(addons_root, &folder_name).ok())
        .unwrap_or_else(|| mock_file_entries(&folder_name));
    let collapsed = collapsed_file_folders();
    let files = apply_collapsed_file_folders(files, &folder_name, &collapsed);

    let selected_path =
        preferred_file_selection(&files, ui.get_selected_file_path().as_str()).unwrap_or_default();

    let modified_count = files.iter().filter(|entry| entry.modified).count() as i32;

    ui.set_addon_files(Rc::new(VecModel::from(files)).into());
    ui.set_file_modified_count(modified_count);
    ui.set_file_tree_scroll_y(file_tree_scroll_y_for_selection(
        ui.get_addon_files(),
        &selected_path,
        !selected_path.is_empty(),
    ));

    if selected_path.is_empty() {
        clear_editor(ui);
    } else {
        open_file_in_editor(ui, &folder_name, &selected_path);
    }

    if ui.get_show_file_backups() {
        refresh_edit_backups(ui);
    }
}

fn preferred_file_selection(files: &[FileEntry], current_path: &str) -> Option<String> {
    let current_path = current_path.trim();
    if !current_path.is_empty() {
        if let Some(entry) = files
            .iter()
            .find(|entry| !entry.folder && entry.relative_path.as_str() == current_path)
        {
            return Some(entry.relative_path.to_string());
        }
    }

    files
        .iter()
        .find(|entry| !entry.folder && entry.modified)
        .or_else(|| files.iter().find(|entry| !entry.folder && !entry.binary))
        .or_else(|| files.iter().find(|entry| !entry.folder))
        .map(|entry| entry.relative_path.to_string())
}

fn file_tree_scroll_y_for_selection(
    files: ModelRc<FileEntry>,
    selected_path: &str,
    editor_open: bool,
) -> f32 {
    let selected_path = selected_path.trim();
    if selected_path.is_empty() {
        return 0.0;
    }

    let row_count = files.row_count();
    let Some(selected_index) = (0..row_count).position(|index| {
        files
            .row_data(index)
            .is_some_and(|entry| !entry.folder && entry.relative_path.as_str() == selected_path)
    }) else {
        return 0.0;
    };

    file_tree_scroll_y_for_row(row_count, selected_index, editor_open)
}

fn file_tree_scroll_y_for_row(row_count: usize, selected_index: usize, editor_open: bool) -> f32 {
    const ROW_HEIGHT: f32 = 30.0;
    const CONTENT_PADDING: f32 = 16.0;
    const SELECTED_CONTEXT: f32 = 60.0;

    let viewport_height = if editor_open { 224.0 } else { 314.0 };
    let content_height = row_count as f32 * ROW_HEIGHT + CONTENT_PADDING;
    let max_offset = (content_height - viewport_height).max(0.0);
    if max_offset <= 0.0 {
        return 0.0;
    }

    let row_y = 8.0 + selected_index as f32 * ROW_HEIGHT;
    -(row_y - SELECTED_CONTEXT).clamp(0.0, max_offset)
}

fn collapsed_file_folders() -> BTreeSet<String> {
    COLLAPSED_FILE_FOLDERS.with(|folders| folders.borrow().clone())
}

fn toggle_collapsed_file_folder(folder_name: &str, relative_path: &str) {
    let key = collapsed_file_folder_key(folder_name, relative_path);
    COLLAPSED_FILE_FOLDERS.with(|folders| {
        let mut folders = folders.borrow_mut();
        if !folders.insert(key.clone()) {
            folders.remove(&key);
        }
    });
}

fn collapsed_file_folder_key(folder_name: &str, relative_path: &str) -> String {
    format!("{folder_name}::{relative_path}")
}

fn apply_collapsed_file_folders(
    rows: Vec<FileEntry>,
    folder_name: &str,
    collapsed: &BTreeSet<String>,
) -> Vec<FileEntry> {
    let mut hidden_prefixes = Vec::<String>::new();
    let mut visible = Vec::new();

    for mut row in rows {
        let relative_path = row.relative_path.to_string();
        if hidden_prefixes
            .iter()
            .any(|prefix| is_descendant_file_path(&relative_path, prefix))
        {
            continue;
        }

        if row.folder {
            let is_collapsed =
                collapsed.contains(&collapsed_file_folder_key(folder_name, &relative_path));
            row.expanded = !is_collapsed;
            if is_collapsed {
                hidden_prefixes.push(relative_path);
            }
        }

        visible.push(row);
    }

    visible
}

fn is_descendant_file_path(relative_path: &str, folder_path: &str) -> bool {
    if folder_path.is_empty() {
        return !relative_path.is_empty();
    }

    relative_path
        .strip_prefix(folder_path)
        .is_some_and(|remainder| remainder.starts_with('/'))
}

fn refresh_edit_backups(ui: &KalpaWindow) {
    let folder_name = selected_addon_folder(ui);
    let backups = addons_source_root()
        .as_deref()
        .map(|addons_root| edit_backup_entries(addons_root, &folder_name))
        .unwrap_or_default();
    ui.set_edit_backups(Rc::new(VecModel::from(backups)).into());
}

fn selected_addon_folder(ui: &KalpaWindow) -> String {
    selected_addon(ui)
        .map(|addon| addon.folder_name.to_string())
        .unwrap_or_default()
}

fn selected_addon(ui: &KalpaWindow) -> Option<AddonEntry> {
    let index = ui.get_selected_index().max(0) as usize;
    ui.get_addons().row_data(index)
}

fn open_file_in_editor(ui: &KalpaWindow, folder_name: &str, relative_path: &str) {
    let file_entry = current_file_entry(ui, relative_path);
    let file_name = file_name_from_path(relative_path);
    let is_binary = file_entry
        .as_ref()
        .map(|entry| entry.binary)
        .unwrap_or_else(|| is_binary_extension(extension_from_path(relative_path).as_str()));

    ui.set_selected_file_path(relative_path.into());
    ui.set_selected_file_name(file_name.into());
    ui.set_editor_open(true);
    ui.set_editor_error(false);
    ui.set_editor_binary(is_binary);
    ui.set_file_tree_scroll_y(file_tree_scroll_y_for_selection(
        ui.get_addon_files(),
        relative_path,
        true,
    ));

    if is_binary {
        ui.set_selected_file_content("".into());
        ui.set_selected_original_content("".into());
        ui.set_editor_line_numbers("".into());
        ui.set_editor_editable(false);
        ui.set_editor_message("Binary file - cannot edit in Kalpa.".into());
        return;
    }

    let content = addons_source_root()
        .as_deref()
        .map(|addons_root| read_text_file(addons_root, folder_name, relative_path))
        .unwrap_or_else(|| Ok(mock_file_content(folder_name, relative_path)));

    match content {
        Ok(content) => {
            let editable = file_entry.as_ref().is_some_and(|entry| entry.modified);
            ui.set_selected_file_content(content.clone().into());
            ui.set_editor_line_numbers(line_numbers_for_content(&content).into());
            ui.set_selected_original_content(content.into());
            ui.set_editor_editable(editable);
            ui.set_editor_message("".into());
        }
        Err(error) => {
            ui.set_selected_file_content("".into());
            ui.set_selected_original_content("".into());
            ui.set_editor_line_numbers("".into());
            ui.set_editor_editable(false);
            ui.set_editor_error(true);
            ui.set_editor_message(error.into());
        }
    }
}

fn save_file_from_editor(ui: &KalpaWindow, relative_path: &str, content: &str) {
    let folder_name = selected_addon_folder(ui);
    if let Some(addons_root) = addons_source_root() {
        if let Err(error) = write_text_file(&addons_root, &folder_name, relative_path, content) {
            ui.set_editor_error(true);
            ui.set_editor_message(format!("Failed to save: {error}").into());
            return;
        }
    }

    ui.set_selected_original_content(content.into());
    ui.set_editor_line_numbers(line_numbers_for_content(content).into());
    ui.set_editor_error(false);
    ui.set_editor_binary(false);
    ui.set_editor_editable(true);
    ui.set_editor_message(format!("Saved {}", file_name_from_path(relative_path)).into());
    mark_file_modified(ui, relative_path);
}

fn clear_editor(ui: &KalpaWindow) {
    ui.set_selected_file_path("".into());
    ui.set_selected_file_name("".into());
    ui.set_selected_file_content("".into());
    ui.set_selected_original_content("".into());
    ui.set_editor_line_numbers("".into());
    ui.set_editor_message("No editable files found.".into());
    ui.set_editor_open(false);
    ui.set_editor_editable(false);
    ui.set_editor_binary(false);
    ui.set_editor_error(false);
}

fn line_numbers_for_content(content: &str) -> String {
    let line_count = if content.is_empty() {
        1
    } else {
        content.split('\n').count()
    };
    (1..=line_count)
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn edit_backup_entries(addons_root: &Path, folder_name: &str) -> Vec<BackupEntry> {
    let addon_backup_dir = addons_root.join(".kalpa-backups").join(folder_name);
    if !addon_backup_dir.is_dir() {
        return Vec::new();
    }

    let mut entries = fs::read_dir(&addon_backup_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    entries.reverse();

    let mut backups = Vec::new();
    for entry in entries {
        let manifest_path = entry.path().join("manifest.json");
        let Ok(content) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<BackupManifestDraft>(&content) else {
            continue;
        };
        if manifest.addon_folder != folder_name {
            continue;
        }

        for file in manifest.files {
            backups.push(BackupEntry {
                backed_up_at: manifest.backed_up_at.as_str().into(),
                update_from: manifest.update_from.as_str().into(),
                update_to: manifest.update_to.as_str().into(),
                relative_path: file.into(),
            });
        }
    }

    backups
}

fn restore_edit_backup_file(
    addons_root: &Path,
    folder_name: &str,
    backup: &BackupEntry,
) -> Result<(), String> {
    if backup.backed_up_at.contains("..")
        || backup.backed_up_at.contains('/')
        || backup.backed_up_at.contains('\\')
    {
        return Err("Invalid backup timestamp.".to_string());
    }

    let timestamp_dir = backup.backed_up_at.replace(':', "-");
    let source_root = addons_root
        .join(".kalpa-backups")
        .join(folder_name)
        .join(timestamp_dir);
    let source = safe_relative_path(&source_root, backup.relative_path.as_str())?;
    if !source.is_file() {
        return Err(format!(
            "Backup file not found: {}",
            backup.relative_path.as_str()
        ));
    }

    let destination = addon_file_path(addons_root, folder_name, backup.relative_path.as_str())?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create directory: {error}"))?;
    }
    fs::copy(&source, &destination).map_err(|error| format!("Failed to restore file: {error}"))?;
    Ok(())
}

fn current_file_entry(ui: &KalpaWindow, relative_path: &str) -> Option<FileEntry> {
    let files = ui.get_addon_files();
    (0..files.row_count())
        .filter_map(|index| files.row_data(index))
        .find(|entry| entry.relative_path.as_str() == relative_path)
}

fn mark_file_modified(ui: &KalpaWindow, relative_path: &str) {
    let files = ui.get_addon_files();
    let rows = (0..files.row_count())
        .filter_map(|index| files.row_data(index))
        .collect::<Vec<_>>();
    let (rows, modified_count) = mark_modified_file_rows(rows, relative_path);
    ui.set_addon_files(Rc::new(VecModel::from(rows)).into());
    ui.set_file_modified_count(modified_count);
}

fn mark_modified_file_rows(mut rows: Vec<FileEntry>, relative_path: &str) -> (Vec<FileEntry>, i32) {
    for entry in &mut rows {
        if !entry.folder && entry.relative_path.as_str() == relative_path {
            entry.modified = true;
        }
    }

    let modified_count = rows.iter().filter(|entry| entry.modified).count() as i32;
    (rows, modified_count)
}

fn addons_source_root() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KALPA_ADDONS_PATH")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
    {
        return Some(path);
    }

    default_addons_root().filter(|path| path.is_dir())
}

fn default_addons_root() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE").map(|home| {
            PathBuf::from(home)
                .join("Documents")
                .join("Elder Scrolls Online")
                .join("live")
                .join("AddOns")
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

fn real_file_entries(addons_root: &Path, folder_name: &str) -> Result<Vec<FileEntry>, String> {
    let addon_root = resolve_addon_disk_path(addons_root, folder_name)
        .ok_or_else(|| format!("Addon folder not found: {folder_name}"))?;
    if !addon_root.is_dir() {
        return Err(format!("Addon folder not found: {folder_name}"));
    }

    let mut files = vec![folder_entry(folder_name, "", 0, true)];
    walk_file_entries(&addon_root, &addon_root, 1, &mut files)?;
    Ok(files)
}

fn walk_file_entries(
    base: &Path,
    current: &Path,
    depth: i32,
    files: &mut Vec<FileEntry>,
) -> Result<(), String> {
    if depth > 32 {
        return Err("Directory tree too deep (> 32 levels).".to_string());
    }

    let mut entries = fs::read_dir(current)
        .map_err(|error| format!("Failed to read directory: {error}"))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();

    entries.sort_by(|a, b| {
        let a_dir = a.path().is_dir();
        let b_dir = b.path().is_dir();
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });

    for entry in entries {
        let path = entry.path();
        if path
            .symlink_metadata()
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
        {
            continue;
        }

        let relative_path = path
            .strip_prefix(base)
            .map_err(|error| format!("Path prefix error: {error}"))?
            .components()
            .map(|component| component.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");

        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| relative_path.clone());

        if path.is_dir() {
            files.push(folder_entry(&name, &relative_path, depth, depth < 3));
            walk_file_entries(base, &path, depth + 1, files)?;
        } else {
            let extension = extension_from_path(&relative_path);
            let size = path.metadata().map(|meta| meta.len()).unwrap_or(0);
            files.push(file_entry(
                &name,
                &relative_path,
                &format_size(size),
                &extension,
                depth,
                false,
            ));
        }
    }

    Ok(())
}

fn mock_file_entries(folder_name: &str) -> Vec<FileEntry> {
    vec![
        folder_entry(folder_name, "", 0, true),
        file_entry(
            &format!("{folder_name}.lua"),
            &format!("{folder_name}.lua"),
            "22.4 KB",
            "lua",
            1,
            true,
        ),
        file_entry(
            &format!("{folder_name}.xml"),
            &format!("{folder_name}.xml"),
            "4.8 KB",
            "xml",
            1,
            false,
        ),
        folder_entry("lang", "lang", 1, true),
        file_entry("en.lua", "lang/en.lua", "6.2 KB", "lua", 2, false),
        file_entry("de.lua", "lang/de.lua", "6.0 KB", "lua", 2, false),
        folder_entry("textures", "textures", 1, false),
        file_entry("icon.dds", "textures/icon.dds", "48.0 KB", "dds", 2, false),
    ]
}

fn folder_entry(name: &str, relative_path: &str, indent_level: i32, expanded: bool) -> FileEntry {
    FileEntry {
        name: name.into(),
        relative_path: relative_path.into(),
        size: "".into(),
        extension: "".into(),
        extension_kind: 0,
        indent_level,
        folder: true,
        expanded,
        modified: false,
        binary: false,
    }
}

fn file_entry(
    name: &str,
    relative_path: &str,
    size: &str,
    extension: &str,
    indent_level: i32,
    modified: bool,
) -> FileEntry {
    FileEntry {
        name: name.into(),
        relative_path: relative_path.into(),
        size: size.into(),
        extension: extension.to_uppercase().into(),
        extension_kind: extension_kind(extension),
        indent_level,
        folder: false,
        expanded: false,
        modified,
        binary: is_binary_extension(extension),
    }
}

fn extension_kind(extension: &str) -> i32 {
    match extension.to_ascii_lowercase().as_str() {
        "xml" => 1,
        "dds" | "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tga" => 2,
        _ => 0,
    }
}

fn extension_from_path(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_binary_extension(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "dds" | "ttf" | "otf" | "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tga"
    )
}

fn file_name_from_path(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn read_text_file(
    addons_root: &Path,
    folder_name: &str,
    relative_path: &str,
) -> Result<String, String> {
    let file_path = addon_file_path(addons_root, folder_name, relative_path)?;
    const MAX_EDITOR_SIZE: u64 = 5 * 1024 * 1024;
    let meta = fs::metadata(&file_path).map_err(|error| format!("Failed to read file: {error}"))?;
    if meta.len() > MAX_EDITOR_SIZE {
        return Err(format!(
            "File is too large to edit ({:.1} MB). Maximum is 5 MB.",
            meta.len() as f64 / (1024.0 * 1024.0)
        ));
    }

    let bytes = fs::read(&file_path).map_err(|error| format!("Failed to read file: {error}"))?;
    if bytes.iter().take(512).any(|&byte| byte == 0) {
        return Err("Cannot read binary file.".to_string());
    }

    String::from_utf8(bytes).map_err(|_| "File contains invalid UTF-8.".to_string())
}

fn write_text_file(
    addons_root: &Path,
    folder_name: &str,
    relative_path: &str,
    content: &str,
) -> Result<(), String> {
    let file_path = addon_file_path(addons_root, folder_name, relative_path)?;
    fs::write(&file_path, content).map_err(|error| format!("Failed to write file: {error}"))
}

fn addon_file_path(
    addons_root: &Path,
    folder_name: &str,
    relative_path: &str,
) -> Result<PathBuf, String> {
    if relative_path.contains("..")
        || relative_path.starts_with('/')
        || relative_path.starts_with('\\')
        || Path::new(relative_path).is_absolute()
    {
        return Err("Invalid relative file path.".to_string());
    }

    let addon_root = resolve_addon_disk_path(addons_root, folder_name)
        .ok_or_else(|| format!("Addon folder not found: {folder_name}"))?;
    let mut path = addon_root.clone();
    for segment in relative_path.split('/') {
        if !segment.is_empty() {
            path.push(segment);
        }
    }
    ensure_contained_path(&addon_root, &path)
}

fn safe_relative_path(base: &Path, relative_path: &str) -> Result<PathBuf, String> {
    if relative_path.contains("..")
        || relative_path.starts_with('/')
        || relative_path.starts_with('\\')
        || Path::new(relative_path).is_absolute()
    {
        return Err("Invalid relative file path.".to_string());
    }

    let mut path = base.to_path_buf();
    for segment in relative_path.split('/') {
        if !segment.is_empty() {
            path.push(segment);
        }
    }
    ensure_contained_path(base, &path)
}

fn ensure_contained_path(base: &Path, path: &Path) -> Result<PathBuf, String> {
    let canonical_base =
        fs::canonicalize(base).map_err(|error| format!("Failed to resolve base path: {error}"))?;

    let path_to_check = if path.exists() {
        fs::canonicalize(path).map_err(|error| format!("Failed to resolve file path: {error}"))?
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| "File path has no parent directory.".to_string())?;
        let canonical_parent = fs::canonicalize(parent)
            .map_err(|error| format!("Failed to resolve parent path: {error}"))?;
        let file_name = path
            .file_name()
            .ok_or_else(|| "File path has no file name.".to_string())?;
        canonical_parent.join(file_name)
    };

    if !path_to_check.starts_with(&canonical_base) {
        return Err("File path escapes the addon directory.".to_string());
    }

    Ok(path.to_path_buf())
}

fn addon_disk_path(addons_root: &Path, folder_name: &str, disabled: bool) -> PathBuf {
    if disabled {
        let disabled_path = addons_root.join(format!("{folder_name}.disabled"));
        if disabled_path.is_dir() {
            return disabled_path;
        }
    }

    resolve_addon_disk_path(addons_root, folder_name)
        .unwrap_or_else(|| addons_root.join(folder_name))
}

fn resolve_addon_disk_path(addons_root: &Path, folder_name: &str) -> Option<PathBuf> {
    let enabled_path = addons_root.join(folder_name);
    if enabled_path.is_dir() {
        return Some(enabled_path);
    }

    let disabled_path = addons_root.join(format!("{folder_name}.disabled"));
    if disabled_path.is_dir() {
        return Some(disabled_path);
    }

    None
}

fn mock_file_content(folder_name: &str, relative_path: &str) -> String {
    if relative_path.ends_with(".xml") {
        return format!(
            "<GuiXml>\n  <Controls>\n    <TopLevelControl name=\"{folder_name}Window\" hidden=\"true\">\n      <Dimensions x=\"420\" y=\"320\" />\n    </TopLevelControl>\n  </Controls>\n</GuiXml>\n"
        );
    }

    if relative_path.starts_with("lang/") {
        return "local strings = {\n  settings = \"Settings\",\n  enabled = \"Enabled\",\n  disabled = \"Disabled\",\n}\n\nreturn strings\n".to_string();
    }

    format!(
        "local addonName = \"{folder_name}\"\nlocal version = \"2.1.3\"\n\nlocal function OnAddonLoaded(eventCode, name)\n  if name ~= addonName then return end\n  EVENT_MANAGER:UnregisterForEvent(addonName, EVENT_ADD_ON_LOADED)\n  d(addonName .. \" loaded\")\nend\n\nEVENT_MANAGER:RegisterForEvent(addonName, EVENT_ADD_ON_LOADED, OnAddonLoaded)\n"
    )
}

fn open_path(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(url).spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

fn apply_runtime_flags(ui: &KalpaWindow) {
    let reduced_motion = std::env::var("KALPA_REDUCED_MOTION")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);

    ui.global::<Tokens>().set_reduced_motion(reduced_motion);
    ui.global::<Tokens>()
        .set_ambient_motion(env_flag("KALPA_AMBIENT_MOTION"));

    let detail_files_active = std::env::var("KALPA_DETAIL_TAB")
        .map(|value| value.eq_ignore_ascii_case("files"))
        .unwrap_or(false);

    ui.set_detail_files_active(detail_files_active);
    ui.set_settings_open(env_flag("KALPA_SETTINGS_OPEN"));
    ui.set_settings_editor_open(env_flag("KALPA_SETTINGS_EDITOR"));
    let settings_tab = std::env::var("KALPA_SETTINGS_TAB")
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "appearance" | "theme" | "themes" => 1,
            "tools" => 2,
            "data" => 3,
            _ => 0,
        })
        .unwrap_or(0);
    ui.set_settings_tab(settings_tab);

    let discover_active = std::env::var("KALPA_VIEW")
        .map(|value| value.eq_ignore_ascii_case("discover"))
        .unwrap_or(false);
    ui.set_discover_active(discover_active);

    let discover_tab = std::env::var("KALPA_DISCOVER_TAB")
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "popular" => 1,
            "categories" => 2,
            "url" | "id" => 3,
            _ => 0,
        })
        .unwrap_or(0);
    ui.set_discover_tab(discover_tab);

    let discover_query =
        std::env::var("KALPA_DISCOVER_QUERY").unwrap_or_else(|_| "combat alerts".to_string());
    ui.set_discover_query(discover_query.into());

    let discover_url = std::env::var("KALPA_DISCOVER_URL")
        .unwrap_or_else(|_| "https://www.esoui.com/downloads/info3520.html".to_string());
    ui.set_discover_url_input(discover_url.into());

    ui.set_offline_active(env_flag("KALPA_OFFLINE"));
    if std::env::var("KALPA_SELECTED_INDEX").is_ok() {
        ui.set_selected_index(env_i32("KALPA_SELECTED_INDEX"));
    }
    if let Ok(message) = std::env::var("KALPA_STATUS_ERROR") {
        ui.set_status_error_message(message.into());
    }
    if let Ok(message) = std::env::var("KALPA_APP_UPDATE") {
        ui.set_app_update_message(message.into());
    }
    ui.set_update_available_count(env_i32("KALPA_UPDATE_COUNT"));
    ui.set_pending_conflict_count(env_i32("KALPA_PENDING_CONFLICTS"));
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn env_i32(name: &str) -> i32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(0)
        .max(0)
}

fn apply_theme_selection(ui: &KalpaWindow, selection: &ThemeSelection) {
    let tokens = ui.global::<Tokens>();
    tokens.set_skin_kind(selection.skin_kind);

    let seed = &selection.seed;
    tokens.set_bg_base(hex_color(&seed.bg_base));
    tokens.set_bg_base_60(hex_alpha(&seed.bg_base, 0x99));
    tokens.set_bg_base_85(hex_alpha(&seed.bg_base, 0xd9));
    tokens.set_background(hex_color(&seed.background));
    tokens.set_surface(hex_color(&seed.surface));
    let glass_soft_alpha = if selection.skin_kind == 0 { 0xa8 } else { 0xe0 };
    let glass_alpha = if selection.skin_kind == 0 { 0xd6 } else { 0xf0 };
    tokens.set_surface_66(hex_alpha(&seed.surface, glass_soft_alpha));
    tokens.set_surface_84(hex_alpha(&seed.surface, glass_alpha));
    tokens.set_foreground(hex_color(&seed.foreground));
    tokens.set_foreground_80(hex_alpha(&seed.foreground, 0xcc));
    tokens.set_muted_foreground(hex_color(&seed.muted_foreground));
    tokens.set_muted_foreground_30(hex_alpha(&seed.muted_foreground, 0x4d));
    tokens.set_muted_foreground_50(hex_alpha(&seed.muted_foreground, 0x80));
    tokens.set_muted_foreground_70(hex_alpha(&seed.muted_foreground, 0xb3));
    tokens.set_primary(hex_color(&seed.primary));
    tokens.set_primary_hover(color_from_rgb(primary_hover_rgb(&seed.primary)));
    tokens.set_primary_foreground(hex_color(&seed.primary_foreground));
    tokens.set_primary_04(hex_alpha(&seed.primary, 0x0a));
    tokens.set_primary_06(hex_alpha(&seed.primary, 0x10));
    tokens.set_primary_10(hex_alpha(&seed.primary, 0x1a));
    tokens.set_primary_15(hex_alpha(&seed.primary, 0x26));
    tokens.set_primary_20(hex_alpha(&seed.primary, 0x33));
    tokens.set_primary_25(hex_alpha(&seed.primary, 0x40));
    tokens.set_primary_30(hex_alpha(&seed.primary, 0x4d));
    tokens.set_accent(hex_color(&seed.accent));
    tokens.set_accent_04(hex_alpha(&seed.accent, 0x0a));
    tokens.set_accent_06(hex_alpha(&seed.accent, 0x0f));
    tokens.set_accent_10(hex_alpha(&seed.accent, 0x1a));
    tokens.set_accent_15(hex_alpha(&seed.accent, 0x26));
    tokens.set_accent_16(hex_alpha(&seed.accent, 0x28));
    tokens.set_accent_20(hex_alpha(&seed.accent, 0x33));
    tokens.set_accent_22(hex_alpha(&seed.accent, 0x35));
    tokens.set_accent_25(hex_alpha(&seed.accent, 0x40));
    tokens.set_accent_scroll(hex_alpha(&seed.accent, 0x2b));
    tokens.set_accent_scroll_thumb(hex_alpha(&seed.accent, 0x75));
    tokens.set_border(hex_color(&seed.border));
    tokens.set_orb1(hex_color(&seed.orb1));
    tokens.set_orb1_20(hex_alpha(&seed.orb1, 0x33));
    tokens.set_orb1_08(hex_alpha(&seed.orb1, 0x14));
    tokens.set_orb2(hex_color(&seed.orb2));
    tokens.set_orb2_15(hex_alpha(&seed.orb2, 0x26));
    tokens.set_orb2_06(hex_alpha(&seed.orb2, 0x0f));
    tokens.set_orb3(hex_color(&seed.orb3));
    tokens.set_orb3_10(hex_alpha(&seed.orb3, 0x1a));
    tokens.set_orb3_04(hex_alpha(&seed.orb3, 0x0a));
    let success = mix_oklab("#34d399", &seed.primary, 0.22);
    let warning = mix_oklab("#fbbf24", &seed.primary, 0.18);
    let error = mix_oklab("#f87171", &seed.primary, 0.14);
    let library = mix_oklab("#a78bfa", &seed.primary, 0.22);

    tokens.set_status_success(color_from_rgb(success));
    tokens.set_status_success_04(color_from_argb(0x0a, success));
    tokens.set_status_success_15(color_from_argb(0x26, success));
    tokens.set_status_success_20(color_from_argb(0x33, success));
    tokens.set_status_success_25(color_from_argb(0x40, success));
    tokens.set_status_warning(color_from_rgb(warning));
    tokens.set_status_warning_04(color_from_argb(0x0a, warning));
    tokens.set_status_warning_15(color_from_argb(0x26, warning));
    tokens.set_status_warning_20(color_from_argb(0x33, warning));
    tokens.set_status_warning_25(color_from_argb(0x40, warning));
    tokens.set_status_error(color_from_rgb(error));
    tokens.set_status_error_04(color_from_argb(0x0a, error));
    tokens.set_status_error_15(color_from_argb(0x26, error));
    tokens.set_status_error_20(color_from_argb(0x33, error));
    tokens.set_status_error_25(color_from_argb(0x40, error));
    tokens.set_status_library(color_from_rgb(library));
    tokens.set_status_library_04(color_from_argb(0x0a, library));
    tokens.set_status_library_15(color_from_argb(0x26, library));
    tokens.set_status_library_20(color_from_argb(0x33, library));
    tokens.set_status_library_25(color_from_argb(0x40, library));

    apply_backdrop_skins(ui, seed);
}

fn apply_backdrop_skins(ui: &KalpaWindow, seed: &ThemeSeed) {
    let backdrop = ui.global::<BackdropSkins>();
    backdrop.set_orb_one(cached_blurred_orb_skin(600, rgb_from_hex(&seed.orb1), 0.20));
    backdrop.set_orb_two(cached_blurred_orb_skin(500, rgb_from_hex(&seed.orb2), 0.15));
    backdrop.set_orb_three(cached_blurred_orb_skin(400, rgb_from_hex(&seed.orb3), 0.10));
}

fn cached_blurred_orb_skin(size: u32, color: (u8, u8, u8), opacity: f32) -> Image {
    let key = format!(
        "{size}:{}:{}:{}:{}",
        color.0,
        color.1,
        color.2,
        (opacity * 1000.0).round() as u16
    );
    ORB_SKIN_CACHE.with(|cache| {
        if let Some(image) = cache.borrow().get(&key).cloned() {
            return image;
        }

        let image = blurred_orb_skin(size, color, opacity);
        cache.borrow_mut().insert(key, image.clone());
        image
    })
}

fn blurred_orb_skin(size: u32, color: (u8, u8, u8), opacity: f32) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(size, size);
    let pixels = buffer.make_mut_slice();
    let center = size as f32 / 2.0;
    let core_radius = size as f32 * 0.18;
    let fade_radius = size as f32 * 0.50;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let alpha = orb_alpha(distance, core_radius, fade_radius, opacity);
            pixels[(y * size + x) as usize] = premul_pixel(color, alpha);
        }
    }

    Image::from_rgba8_premultiplied(buffer)
}

fn orb_alpha(distance: f32, core_radius: f32, fade_radius: f32, opacity: f32) -> f32 {
    let edge_fade = 1.0 - smoothstep(core_radius, fade_radius, distance);
    (opacity * edge_fade).clamp(0.0, 1.0)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn premul_pixel((r, g, b): (u8, u8, u8), alpha: f32) -> Rgba8Pixel {
    Rgba8Pixel::new(
        (r as f32 * alpha).round().clamp(0.0, 255.0) as u8,
        (g as f32 * alpha).round().clamp(0.0, 255.0) as u8,
        (b as f32 * alpha).round().clamp(0.0, 255.0) as u8,
        (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

fn runtime_theme_selection() -> Option<ThemeSelection> {
    if let Ok(path) = std::env::var("KALPA_THEME_FILE") {
        match fs::read_to_string(&path)
            .ok()
            .and_then(|json| parse_theme_selection(&json).ok())
        {
            Some(selection) => return Some(selection),
            None => eprintln!("KALPA_THEME_FILE could not be parsed: {path}"),
        }
    }

    if let Ok(json) = std::env::var("KALPA_THEME_JSON") {
        match parse_theme_selection(&json) {
            Ok(selection) => return Some(selection),
            Err(error) => eprintln!("KALPA_THEME_JSON could not be parsed: {error}"),
        }
    }

    None
}

fn apply_initial_theme(ui: &KalpaWindow) -> String {
    if let Some(selection) = runtime_theme_selection() {
        apply_theme_selection(ui, &selection);
        return std::env::var("KALPA_THEME").unwrap_or_else(|_| "custom-json".to_string());
    }

    let fallback_theme_id =
        default_catalog_theme_id().unwrap_or_else(|| "nordic-runestone".to_string());
    let theme_id = std::env::var("KALPA_THEME")
        .ok()
        .or_else(read_persisted_active_theme_id)
        .unwrap_or(fallback_theme_id.clone());
    if let Some(selection) = theme_selection_by_id_from_store(&theme_id) {
        apply_theme_selection(ui, &selection);
        return theme_id;
    }

    if let Some(selection) = theme_selection_by_id_from_store(&fallback_theme_id) {
        apply_theme_selection(ui, &selection);
        return fallback_theme_id;
    }

    "custom-json".to_string()
}

fn catalog_theme_selection_by_id(theme_id: &str) -> Option<ThemeSelection> {
    catalog_theme_by_id(theme_id)
        .map(|theme| ThemeSelection::with_skin(theme.colors, theme.skin_id.as_deref()))
}

fn theme_selection_by_id_from_store(theme_id: &str) -> Option<ThemeSelection> {
    let custom_themes = read_custom_themes();
    theme_selection_by_id(theme_id, &custom_themes)
}

fn theme_selection_by_id(theme_id: &str, custom_themes: &[CatalogTheme]) -> Option<ThemeSelection> {
    custom_themes
        .iter()
        .find(|theme| theme.id == theme_id)
        .map(|theme| ThemeSelection::colors_only(theme.colors.clone()))
        .or_else(|| catalog_theme_selection_by_id(theme_id))
}

fn theme_by_id(theme_id: &str, custom_themes: &[CatalogTheme]) -> Option<CatalogTheme> {
    custom_themes
        .iter()
        .find(|theme| theme.id == theme_id)
        .cloned()
        .or_else(|| catalog_theme_by_id(theme_id))
}

fn catalog_theme_by_id(theme_id: &str) -> Option<CatalogTheme> {
    parse_theme_catalog()?
        .themes
        .into_iter()
        .find(|theme| theme.id == theme_id)
}

fn default_catalog_theme() -> Option<CatalogTheme> {
    let catalog = parse_theme_catalog()?;
    let default_id = catalog
        .root_theme_id
        .as_ref()
        .unwrap_or(&catalog.default_theme_id)
        .clone();
    catalog
        .themes
        .into_iter()
        .find(|theme| theme.id == default_id)
}

fn default_catalog_theme_id() -> Option<String> {
    let catalog = parse_theme_catalog()?;
    Some(
        catalog
            .root_theme_id
            .as_ref()
            .unwrap_or(&catalog.default_theme_id)
            .clone(),
    )
}

fn native_state_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KALPA_NATIVE_STATE_DIR") {
        return Some(PathBuf::from(path));
    }

    std::env::var("APPDATA")
        .ok()
        .map(PathBuf::from)
        .map(|path| path.join("Kalpa"))
}

fn active_theme_store_path() -> Option<PathBuf> {
    let filename = if std::env::var("KALPA_NATIVE_STATE_DIR").is_ok() {
        "active-theme.txt"
    } else {
        "native-active-theme.txt"
    };
    native_state_dir().map(|path| path.join(filename))
}

fn custom_theme_store_path() -> Option<PathBuf> {
    let filename = if std::env::var("KALPA_NATIVE_STATE_DIR").is_ok() {
        "custom-themes.json"
    } else {
        "native-custom-themes.json"
    };
    native_state_dir().map(|path| path.join(filename))
}

fn native_settings_store_path() -> Option<PathBuf> {
    let filename = if std::env::var("KALPA_NATIVE_STATE_DIR").is_ok() {
        "settings.json"
    } else {
        "native-settings.json"
    };
    native_state_dir().map(|path| path.join(filename))
}

fn read_persisted_active_theme_id() -> Option<String> {
    read_active_theme_id_from_path(&active_theme_store_path()?)
}

fn read_active_theme_id_from_path(path: &Path) -> Option<String> {
    let theme_id = fs::read_to_string(path).ok()?.trim().to_string();
    if theme_id.is_empty() {
        None
    } else {
        Some(theme_id)
    }
}

fn persist_active_theme_id(theme_id: &str) {
    let Some(path) = active_theme_store_path() else {
        return;
    };

    if let Err(error) = persist_active_theme_id_to_path(&path, theme_id) {
        eprintln!("Failed to persist native theme selection: {error}");
    }
}

fn persist_active_theme_id_to_path(path: &Path, theme_id: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return Err(format!(
                "Failed to create native theme state directory: {error}"
            ));
        }
    }

    fs::write(path, theme_id).map_err(|error| format!("Failed to write active theme id: {error}"))
}

fn read_custom_themes() -> Vec<CatalogTheme> {
    let Some(path) = custom_theme_store_path() else {
        return Vec::new();
    };
    read_custom_themes_from_path(&path).unwrap_or_default()
}

fn read_custom_themes_from_path(path: &Path) -> Result<Vec<CatalogTheme>, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Failed to read custom themes: {error}")),
    };

    if contents.trim().is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str::<NativeCustomThemeStore>(&contents)
        .map(|store| store.themes)
        .or_else(|_| serde_json::from_str::<Vec<CatalogTheme>>(&contents))
        .map(|themes| {
            themes
                .into_iter()
                .map(|mut theme| {
                    theme.category = "Custom".to_string();
                    theme.skin_id = None;
                    theme
                })
                .collect()
        })
        .map_err(|error| format!("Failed to parse custom themes: {error}"))
}

fn persist_custom_themes(custom_themes: &[CatalogTheme]) {
    let Some(path) = custom_theme_store_path() else {
        return;
    };

    if let Err(error) = persist_custom_themes_to_path(&path, custom_themes) {
        eprintln!("Failed to persist native custom themes: {error}");
    }
}

fn persist_custom_themes_to_path(
    path: &Path,
    custom_themes: &[CatalogTheme],
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create custom theme directory: {error}"))?;
    }

    let store = NativeCustomThemeStore {
        themes: custom_themes.to_vec(),
    };
    let json = serde_json::to_string_pretty(&store)
        .map_err(|error| format!("Failed to serialize custom themes: {error}"))?;
    fs::write(path, json).map_err(|error| format!("Failed to write custom themes: {error}"))
}

fn read_native_settings() -> NativeSettings {
    let Some(path) = native_settings_store_path() else {
        return NativeSettings::default();
    };
    read_native_settings_from_path(&path).unwrap_or_default()
}

fn read_native_settings_from_path(path: &Path) -> Result<NativeSettings, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NativeSettings::default());
        }
        Err(error) => return Err(format!("Failed to read native settings: {error}")),
    };

    if contents.trim().is_empty() {
        return Ok(NativeSettings::default());
    }

    serde_json::from_str::<NativeSettings>(&contents)
        .map(|mut settings| {
            settings.conflict_policy = settings.conflict_policy.clamp(0, 2);
            settings
        })
        .map_err(|error| format!("Failed to parse native settings: {error}"))
}

fn persist_native_settings(settings: &NativeSettings) {
    let Some(path) = native_settings_store_path() else {
        return;
    };

    if let Err(error) = persist_native_settings_to_path(&path, settings) {
        eprintln!("Failed to persist native settings: {error}");
    }
}

fn persist_native_settings_to_path(path: &Path, settings: &NativeSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create native settings directory: {error}"))?;
    }

    let mut settings = settings.clone();
    settings.conflict_policy = settings.conflict_policy.clamp(0, 2);
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|error| format!("Failed to serialize native settings: {error}"))?;
    fs::write(path, json).map_err(|error| format!("Failed to write native settings: {error}"))
}

fn parse_theme_catalog() -> Option<ThemeCatalog> {
    let catalog = match serde_json::from_str::<ThemeCatalog>(BUILTIN_THEME_CATALOG) {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!("Built-in native theme catalog could not be parsed: {error}");
            return None;
        }
    };

    if catalog.themes.is_empty() {
        eprintln!("Built-in native theme catalog is empty");
        None
    } else {
        Some(catalog)
    }
}

fn set_theme_gallery(ui: &KalpaWindow, custom_themes: &[CatalogTheme]) {
    ui.set_themes(
        Rc::new(VecModel::from(theme_gallery_entries_from_custom(
            custom_themes,
        )))
        .into(),
    );
}

#[cfg(test)]
fn theme_gallery_entries() -> Vec<ThemeEntry> {
    theme_gallery_entries_from_custom(&read_custom_themes())
}

fn theme_gallery_entries_from_custom(custom_themes: &[CatalogTheme]) -> Vec<ThemeEntry> {
    let mut themes = Vec::new();
    themes.extend(custom_themes.iter().cloned());
    if let Some(catalog) = parse_theme_catalog() {
        themes.extend(catalog.themes);
    }
    theme_entries_from_catalog_themes(themes)
}

fn theme_entries_from_catalog_themes(themes: Vec<CatalogTheme>) -> Vec<ThemeEntry> {
    let mut current_category = String::new();
    let mut col = 0usize;
    let mut row = 0usize;
    let mut gallery_y = 22i32;

    themes
        .into_iter()
        .map(|theme| {
            let category_heading = if theme.category != current_category {
                if !current_category.is_empty() {
                    if col != 0 {
                        gallery_y += 152;
                    }
                    gallery_y += 34;
                    col = 0;
                    row += 1;
                }
                current_category = theme.category.clone();
                theme_category_heading(&theme.category).to_string()
            } else {
                String::new()
            };

            let entry = theme_entry_from_catalog_theme(
                &theme,
                row as i32,
                col as i32,
                gallery_y,
                category_heading,
            );

            if col == 2 {
                col = 0;
                row += 1;
                gallery_y += 152;
            } else {
                col += 1;
            }

            entry
        })
        .collect()
}

fn theme_category_heading(category: &str) -> &str {
    if category == "Custom" {
        "Your Themes"
    } else {
        category
    }
}

fn theme_entry_from_catalog_theme(
    theme: &CatalogTheme,
    row: i32,
    col: i32,
    gallery_y: i32,
    category_heading: String,
) -> ThemeEntry {
    ThemeEntry {
        id: theme.id.clone().into(),
        name: theme.name.clone().into(),
        category: theme.category.clone().into(),
        description: theme.description.clone().into(),
        bg_base: hex_color(&theme.colors.bg_base),
        background: hex_color(&theme.colors.background),
        surface: hex_color(&theme.colors.surface),
        foreground: hex_color(&theme.colors.foreground),
        muted_foreground: hex_color(&theme.colors.muted_foreground),
        primary: hex_color(&theme.colors.primary),
        primary_foreground: hex_color(&theme.colors.primary_foreground),
        accent: hex_color(&theme.colors.accent),
        border: hex_color(&theme.colors.border),
        orb1: hex_color(&theme.colors.orb1),
        orb2: hex_color(&theme.colors.orb2),
        orb3: hex_color(&theme.colors.orb3),
        row,
        col,
        gallery_y,
        category_heading: category_heading.into(),
    }
}

fn parse_theme_selection(json: &str) -> Result<ThemeSelection, serde_json::Error> {
    serde_json::from_str::<ThemeSeed>(json)
        .map(ThemeSelection::colors_only)
        .or_else(|_| {
            serde_json::from_str::<ThemeEnvelope>(json).map(|theme| {
                let skin_kind = skin_kind(theme.skin_id.as_deref());
                ThemeSelection {
                    seed: theme.colors,
                    skin_kind,
                }
            })
        })
}

fn skin_kind(skin_id: Option<&str>) -> i32 {
    match skin_id {
        Some("elder-scroll-ancient-tome") => 1,
        Some("daedric-obsidian") => 2,
        Some("dwemer-brass") => 3,
        Some("ayleid-welkynd") => 4,
        Some("sithis-brotherhood") => 5,
        Some("apocrypha-mora") => 6,
        Some("clockwork-city") => 7,
        Some("nordic-runestone") => 8,
        _ => 0,
    }
}

fn hex_color(hex: &str) -> Color {
    let (r, g, b) = rgb_from_hex(hex);
    Color::from_rgb_u8(r, g, b)
}

fn hex_alpha(hex: &str, alpha: u8) -> Color {
    let (r, g, b) = rgb_from_hex(hex);
    Color::from_argb_u8(alpha, r, g, b)
}

fn relative_luminance(hex: &str) -> f64 {
    let (r, g, b) = rgb_from_hex(hex);
    let channel = |value: u8| {
        let srgb = value as f64 / 255.0;
        if srgb <= 0.03928 {
            srgb / 12.92
        } else {
            ((srgb + 0.055) / 1.055).powf(2.4)
        }
    };

    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

fn contrast_ratio(foreground: &str, background: &str) -> f64 {
    let foreground = relative_luminance(foreground);
    let background = relative_luminance(background);
    let lighter = foreground.max(background);
    let darker = foreground.min(background);
    (lighter + 0.05) / (darker + 0.05)
}

#[cfg(test)]
fn mix_rgb(base: &str, overlay: &str, overlay_weight: f32) -> (u8, u8, u8) {
    let (br, bg, bb) = rgb_from_hex(base);
    let (or, og, ob) = rgb_from_hex(overlay);
    let weight = overlay_weight.clamp(0.0, 1.0);
    let inv = 1.0 - weight;
    (
        ((br as f32 * inv) + (or as f32 * weight)).round() as u8,
        ((bg as f32 * inv) + (og as f32 * weight)).round() as u8,
        ((bb as f32 * inv) + (ob as f32 * weight)).round() as u8,
    )
}

#[derive(Clone, Copy)]
struct Oklab {
    l: f32,
    a: f32,
    b: f32,
}

fn mix_oklab(base: &str, overlay: &str, overlay_weight: f32) -> (u8, u8, u8) {
    let base = oklab_from_rgb(rgb_from_hex(base));
    let overlay = oklab_from_rgb(rgb_from_hex(overlay));
    let weight = overlay_weight.clamp(0.0, 1.0);
    let inv = 1.0 - weight;

    rgb_from_oklab(Oklab {
        l: base.l * inv + overlay.l * weight,
        a: base.a * inv + overlay.a * weight,
        b: base.b * inv + overlay.b * weight,
    })
}

fn primary_hover_rgb(hex: &str) -> (u8, u8, u8) {
    let lab = oklab_from_rgb(rgb_from_hex(hex));
    let chroma = (lab.a * lab.a + lab.b * lab.b).sqrt();
    let hue = lab.b.atan2(lab.a);
    let next_chroma = (chroma * 1.05).clamp(0.0, 0.37);

    rgb_from_oklab(Oklab {
        l: (lab.l + 0.06).clamp(0.0, 1.0),
        a: next_chroma * hue.cos(),
        b: next_chroma * hue.sin(),
    })
}

fn oklab_from_rgb((r, g, b): (u8, u8, u8)) -> Oklab {
    let r = srgb_to_linear(r);
    let g = srgb_to_linear(g);
    let b = srgb_to_linear(b);

    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    Oklab {
        l: 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        a: 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        b: 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
    }
}

fn rgb_from_oklab(lab: Oklab) -> (u8, u8, u8) {
    let l_ = lab.l + 0.396_337_78 * lab.a + 0.215_803_76 * lab.b;
    let m_ = lab.l - 0.105_561_346 * lab.a - 0.063_854_17 * lab.b;
    let s_ = lab.l - 0.089_484_18 * lab.a - 1.291_485_5 * lab.b;

    let l = l_.powi(3);
    let m = m_.powi(3);
    let s = s_.powi(3);

    let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
    let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
    let b = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;

    (
        linear_to_srgb_u8(r),
        linear_to_srgb_u8(g),
        linear_to_srgb_u8(b),
    )
}

fn srgb_to_linear(value: u8) -> f32 {
    let value = value as f32 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb_u8(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };

    (srgb * 255.0).round().clamp(0.0, 255.0) as u8
}

fn color_from_rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::from_rgb_u8(r, g, b)
}

fn color_from_argb(alpha: u8, (r, g, b): (u8, u8, u8)) -> Color {
    Color::from_argb_u8(alpha, r, g, b)
}

fn rgb_from_hex(hex: &str) -> (u8, u8, u8) {
    let raw = hex.trim_start_matches('#');
    if raw.len() < 6 {
        return (128, 128, 128);
    }

    let r = u8::from_str_radix(&raw[0..2], 16).unwrap_or(128);
    let g = u8::from_str_radix(&raw[2..4], 16).unwrap_or(128);
    let b = u8::from_str_radix(&raw[4..6], 16).unwrap_or(128);
    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    #[test]
    fn embedded_theme_catalog_contains_default_and_exported_themes() {
        let catalog = serde_json::from_str::<ThemeCatalog>(BUILTIN_THEME_CATALOG)
            .expect("embedded native theme catalog parses");

        assert!(
            catalog.themes.len() >= 40,
            "expected full built-in theme catalog"
        );
        assert!(
            catalog
                .themes
                .iter()
                .any(|theme| theme.id == catalog.default_theme_id),
            "default theme id must exist in catalog"
        );
        assert!(
            catalog
                .themes
                .iter()
                .any(|theme| theme.id == "apocrypha-ink"),
            "catalog should include non-hardcoded React theme ids"
        );

        let default_theme = catalog
            .themes
            .iter()
            .find(|theme| theme.id == catalog.default_theme_id)
            .expect("default theme exists");
        assert_eq!(default_theme.skin_id.as_deref(), Some("nordic-runestone"));
        assert!(skin_kind(default_theme.skin_id.as_deref()) > 0);
        assert!(
            catalog
                .themes
                .iter()
                .filter(|theme| theme.skin_id.is_some())
                .count()
                >= 8,
            "catalog should preserve built-in skin ids"
        );
    }

    #[test]
    fn native_contrast_ratio_matches_wcag_reference_points() {
        assert_eq!(
            (contrast_ratio("#000000", "#ffffff") * 100.0).round() / 100.0,
            21.0
        );
        assert_eq!(
            (contrast_ratio("#ffffff", "#ffffff") * 100.0).round() / 100.0,
            1.0
        );
    }

    #[test]
    fn native_contrast_checks_flag_failed_theme_pairs() {
        let mut colors = sample_custom_theme("bad-contrast", "Bad Contrast").colors;
        colors.background = "#111111".to_string();
        colors.surface = "#111111".to_string();
        colors.foreground = "#111111".to_string();
        colors.muted_foreground = "#111111".to_string();
        colors.primary = "#111111".to_string();
        colors.primary_foreground = "#111111".to_string();
        colors.accent = "#111111".to_string();

        let checks = evaluate_theme_contrast(&colors);
        assert_eq!(checks.len(), 5);
        assert!(checks
            .iter()
            .all(|check| check.level == ContrastLevel::Fail));
        assert!(checks.iter().all(|check| check.ratio < 3.0));
    }

    #[test]
    fn embedded_themes_meet_native_contrast_minimums() {
        let catalog = serde_json::from_str::<ThemeCatalog>(BUILTIN_THEME_CATALOG)
            .expect("embedded native theme catalog parses");

        for theme in catalog.themes {
            for check in evaluate_theme_contrast(&theme.colors) {
                assert!(
                    check.level != ContrastLevel::Fail,
                    "{} {} = {:.2}:1",
                    theme.id,
                    check.label,
                    check.ratio
                );
            }
        }
    }

    #[test]
    fn theme_gallery_entries_preserve_catalog_metadata_and_grid_positions() {
        let entries = theme_gallery_entries();
        assert!(
            entries.len() >= 40,
            "native theme gallery should render the full catalog"
        );

        let first = entries.first().expect("first theme exists");
        assert_eq!(first.id.as_str(), "eso-gold");
        assert_eq!(first.name.as_str(), "ESO Gold");
        assert_eq!(first.row, 0);
        assert_eq!(first.col, 0);

        let fourth = entries.get(3).expect("fourth theme exists");
        assert_eq!(fourth.row, 1);
        assert_eq!(fourth.col, 0);

        assert!(
            entries
                .iter()
                .any(|entry| entry.id.as_str() == "apocrypha-ink" && !entry.description.is_empty()),
            "gallery entries should keep descriptions for theme cards"
        );
    }

    #[test]
    fn active_theme_id_round_trips_through_native_store_file() {
        let root = test_temp_dir("active-theme-id");
        let path = root.join("active-theme.txt");

        persist_active_theme_id_to_path(&path, "apocrypha-ink").expect("persist theme id");
        assert_eq!(
            read_active_theme_id_from_path(&path).as_deref(),
            Some("apocrypha-ink")
        );

        fs::write(&path, "   \n").expect("write blank theme id");
        assert_eq!(read_active_theme_id_from_path(&path), None);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn custom_theme_store_round_trips_native_json() {
        let root = test_temp_dir("custom-theme-store");
        let path = root.join("custom-themes.json");
        let theme = sample_custom_theme("custom-one", "Custom One");

        persist_custom_themes_to_path(&path, std::slice::from_ref(&theme))
            .expect("persist custom themes");
        let themes = read_custom_themes_from_path(&path).expect("read custom themes");

        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].id, "custom-one");
        assert_eq!(themes[0].name, "Custom One");
        assert_eq!(themes[0].category, "Custom");
        assert_eq!(themes[0].skin_id, None);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn theme_gallery_places_custom_themes_in_your_themes_section() {
        let custom = sample_custom_theme("custom-one", "Custom One");
        let entries = theme_gallery_entries_from_custom(std::slice::from_ref(&custom));
        let first = entries.first().expect("custom theme first");

        assert_eq!(first.id.as_str(), "custom-one");
        assert_eq!(first.category.as_str(), "Custom");
        assert_eq!(first.category_heading.as_str(), "Your Themes");
        assert!(
            entries
                .iter()
                .any(|entry| entry.id.as_str() == "eso-gold"
                    && entry.category_heading.as_str() == "ESO"),
            "built-in categories should follow custom themes"
        );
    }

    #[test]
    fn normalize_hex_color_accepts_hashless_values_and_rejects_bad_input() {
        assert_eq!(normalize_hex_color("aabbcc", "#000000"), "#AABBCC");
        assert_eq!(normalize_hex_color("#AaBbCc", "#000000"), "#AABBCC");
        assert_eq!(normalize_hex_color("#abc", "#000000"), "#AABBCC");
        assert_eq!(normalize_hex_color("not-a-color", "#123456"), "#123456");
    }

    #[test]
    fn imported_custom_theme_requires_valid_colors_and_normalizes_short_hex() {
        let mut theme = sample_custom_theme("imported-one", "Imported One");
        theme.colors.accent = "#abc".to_string();
        let json = serde_json::to_string(&theme).expect("serialize sample theme");
        let imported = parse_imported_custom_theme(&json).expect("parse imported theme");

        assert_eq!(imported.colors.accent, "#AABBCC");
        assert_eq!(imported.category, "Custom");
        assert_eq!(imported.skin_id, None);

        let mut invalid = theme;
        invalid.colors.primary = "not-a-color".to_string();
        let json = serde_json::to_string(&invalid).expect("serialize invalid theme");
        assert!(parse_imported_custom_theme(&json).is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn preferred_demo_monitor_uses_right_hand_secondary_when_available() {
        let monitors = [
            MonitorPlacement {
                work_left: 0,
                work_top: 0,
                primary: true,
            },
            MonitorPlacement {
                work_left: 2560,
                work_top: 0,
                primary: false,
            },
        ];

        assert_eq!(
            preferred_demo_monitor(&monitors),
            Some(MonitorPlacement {
                work_left: 2560,
                work_top: 0,
                primary: false,
            })
        );
    }

    #[test]
    fn native_settings_round_trip_and_clamp_conflict_policy() {
        let root = test_temp_dir("native-settings");
        let path = root.join("settings.json");
        let settings = NativeSettings {
            auto_update: true,
            warn_eso_running: false,
            official_uploader: true,
            auto_open_analysis: true,
            conflict_policy: 9,
        };

        persist_native_settings_to_path(&path, &settings).expect("persist settings");
        let restored = read_native_settings_from_path(&path).expect("read settings");

        assert!(restored.auto_update);
        assert!(!restored.warn_eso_running);
        assert!(restored.official_uploader);
        assert!(restored.auto_open_analysis);
        assert_eq!(restored.conflict_policy, 2);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn line_numbers_follow_editor_content_line_count() {
        assert_eq!(line_numbers_for_content(""), "1");
        assert_eq!(line_numbers_for_content("one"), "1");
        assert_eq!(line_numbers_for_content("one\ntwo\nthree"), "1\n2\n3");
        assert_eq!(line_numbers_for_content("one\n"), "1\n2");
    }

    #[test]
    fn oklab_status_mix_differs_from_srgb_mix() {
        assert_ne!(
            mix_oklab("#34d399", "#c4a44a", 0.22),
            mix_rgb("#34d399", "#c4a44a", 0.22)
        );
    }

    #[test]
    fn primary_hover_uses_oklch_adjustment() {
        assert_ne!(
            primary_hover_rgb("#c4a44a"),
            mix_rgb("#c4a44a", "#ffffff", 0.16)
        );
    }

    #[test]
    fn backdrop_orb_alpha_fades_to_transparent_edge() {
        let size = 600.0;
        let core = size * 0.18;
        let edge_radius = size * 0.50;
        let center = orb_alpha(0.0, core, edge_radius, 0.20);
        let core_edge = orb_alpha(core, core, edge_radius, 0.20);
        let mid = orb_alpha(size * 0.34, core, edge_radius, 0.20);
        let edge = orb_alpha(edge_radius, core, edge_radius, 0.20);

        assert_eq!(center, 0.20);
        assert_eq!(core_edge, 0.20);
        assert!(core_edge > mid);
        assert!(mid > edge);
        assert_eq!(edge, 0.0);
    }

    #[test]
    fn dependency_model_preserves_dependency_rows() {
        let required = dependency_model(vec![
            dependency_entry("CombatMetricsFightData", "v22+", true, false, true),
            dependency_entry("LibCombat", "v82+", false, false, false),
        ]);

        assert_eq!(required.row_count(), 2);
        let first = required.row_data(0).expect("first dependency exists");
        assert_eq!(first.name.as_str(), "CombatMetricsFightData");
        assert_eq!(first.version.as_str(), "v22+");
        assert!(first.missing);
        assert!(first.install_action);

        let second = required.row_data(1).expect("second dependency exists");
        assert_eq!(second.name.as_str(), "LibCombat");
        assert!(!second.missing);
        assert!(!second.install_action);

        let optional = dependency_model(vec![
            dependency_entry("LibDebugLogger", "v1+", true, false, true),
            dependency_entry("LibDataEncode", "v1+", false, false, false),
        ]);

        assert_eq!(optional.row_count(), 2);
        let missing_optional = optional.row_data(0).expect("optional dependency exists");
        assert_eq!(missing_optional.name.as_str(), "LibDebugLogger");
        assert!(missing_optional.missing);
        assert!(missing_optional.install_action);

        let present_optional = optional
            .row_data(1)
            .expect("present optional dependency exists");
        assert_eq!(present_optional.name.as_str(), "LibDataEncode");
        assert!(!present_optional.missing);
        assert!(!present_optional.install_action);
    }

    #[test]
    fn mock_file_model_preserves_editor_states() {
        let files = mock_file_entries("CombatMetrics");

        let root = files.first().expect("root folder row exists");
        assert!(root.folder);
        assert_eq!(root.name.as_str(), "CombatMetrics");

        let lua = files
            .iter()
            .find(|entry| entry.relative_path.as_str() == "CombatMetrics.lua")
            .expect("mock lua file exists");
        assert!(lua.modified);
        assert!(!lua.binary);
        assert_eq!(lua.extension.as_str(), "LUA");

        let texture = files
            .iter()
            .find(|entry| entry.relative_path.as_str() == "textures/icon.dds")
            .expect("mock binary texture exists");
        assert!(texture.binary);
        assert_eq!(texture.extension_kind, 2);
    }

    #[test]
    fn saving_file_marks_file_tree_row_modified() {
        let files = mock_file_entries("CombatMetrics");
        let (files, modified_count) = mark_modified_file_rows(files, "CombatMetrics.xml");

        let xml = files
            .iter()
            .find(|entry| entry.relative_path.as_str() == "CombatMetrics.xml")
            .expect("xml file exists");
        assert!(xml.modified);
        assert_eq!(modified_count, 2);
    }

    #[test]
    fn refresh_selection_keeps_current_visible_file_before_modified_fallback() {
        let files = mock_file_entries("CombatMetrics");
        let selected = preferred_file_selection(&files, "lang/en.lua").expect("selection exists");
        assert_eq!(selected, "lang/en.lua");
    }

    #[test]
    fn refresh_selection_falls_back_when_current_file_is_hidden() {
        let files = mock_file_entries("CombatMetrics");
        let collapsed = BTreeSet::from([collapsed_file_folder_key("CombatMetrics", "lang")]);
        let visible = apply_collapsed_file_folders(files, "CombatMetrics", &collapsed);
        let selected = preferred_file_selection(&visible, "lang/en.lua").expect("selection exists");
        assert_eq!(selected, "CombatMetrics.lua");
    }

    #[test]
    fn file_tree_scroll_keeps_deep_selection_visible() {
        let mut files = vec![folder_entry("ManyFiles", "", 0, true)];
        for index in 0..20 {
            files.push(file_entry(
                &format!("file-{index}.lua"),
                &format!("file-{index}.lua"),
                "1.0 KB",
                "lua",
                1,
                false,
            ));
        }
        let model: ModelRc<FileEntry> = Rc::new(VecModel::from(files)).into();

        let scroll_y = file_tree_scroll_y_for_selection(model, "file-14.lua", true);

        assert!(scroll_y < 0.0);
        assert_eq!(scroll_y, -398.0);
    }

    #[test]
    fn file_tree_scroll_stays_zero_for_visible_selection() {
        let files = mock_file_entries("CombatMetrics");
        let model: ModelRc<FileEntry> = Rc::new(VecModel::from(files)).into();

        assert_eq!(
            file_tree_scroll_y_for_selection(model, "CombatMetrics.lua", true),
            0.0
        );
    }

    #[test]
    fn collapsed_file_folder_hides_descendants() {
        let files = mock_file_entries("CombatMetrics");
        let collapsed = BTreeSet::from([collapsed_file_folder_key("CombatMetrics", "lang")]);
        let visible = apply_collapsed_file_folders(files, "CombatMetrics", &collapsed);

        let lang = visible
            .iter()
            .find(|entry| entry.relative_path.as_str() == "lang")
            .expect("collapsed folder row remains visible");
        assert!(lang.folder);
        assert!(!lang.expanded);
        assert!(visible
            .iter()
            .all(|entry| !entry.relative_path.as_str().starts_with("lang/")));
    }

    #[test]
    fn collapsed_root_file_folder_hides_every_child() {
        let files = mock_file_entries("CombatMetrics");
        let collapsed = BTreeSet::from([collapsed_file_folder_key("CombatMetrics", "")]);
        let visible = apply_collapsed_file_folders(files, "CombatMetrics", &collapsed);

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].relative_path.as_str(), "");
        assert!(visible[0].folder);
        assert!(!visible[0].expanded);
    }

    #[test]
    fn file_path_helpers_match_editor_guards() {
        assert_eq!(file_name_from_path("lang/en.lua"), "en.lua");
        assert_eq!(extension_from_path("textures/icon.dds"), "dds");
        assert!(is_binary_extension("dds"));
        assert!(!is_binary_extension("lua"));

        let root = test_temp_dir("file-path-helpers");
        fs::create_dir_all(root.join("CombatMetrics/lang")).expect("create temp addon folder");
        assert!(addon_file_path(root, "CombatMetrics", "../bad.lua").is_err());
        assert!(addon_file_path(root, "CombatMetrics", "/bad.lua").is_err());
        assert!(addon_file_path(root, "CombatMetrics", "lang/en.lua").is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mock_file_content_uses_selected_addon_name() {
        let lua = mock_file_content("FancyAddon", "FancyAddon.lua");
        assert!(lua.contains("local addonName = \"FancyAddon\""));

        let xml = mock_file_content("FancyAddon", "FancyAddon.xml");
        assert!(xml.contains("FancyAddonWindow"));
    }

    #[test]
    fn preset_tag_model_tracks_active_labels() {
        let tags = tag_model(vec!["favorite", "raid"]);

        assert_eq!(tags.row_count(), 5);
        let favorite = tags.row_data(0).expect("favorite tag exists");
        assert_eq!(favorite.id.as_str(), "favorite");
        assert!(favorite.active);
        assert!(favorite.label.as_str().contains("favorite"));

        let raid = tags.row_data(4).expect("raid tag exists");
        assert_eq!(raid.id.as_str(), "raid");
        assert!(raid.active);
        assert_eq!(raid.kind, 4);
    }

    #[test]
    fn toggled_tags_updates_model_state() {
        let tags = tag_model(vec!["favorite"]);
        let next = toggled_tags(&tags, "favorite");

        assert!(!tag_is_active(&next, "favorite"));
        assert_eq!(
            next.iter()
                .find(|tag| tag.id.as_str() == "favorite")
                .expect("favorite tag exists")
                .label
                .as_str(),
            "\u{2606} favorite"
        );

        let next = tag_model_from_entries(next);
        let next = toggled_tags(&next, "testing");
        assert!(tag_is_active(&next, "testing"));
    }

    #[test]
    fn custom_tags_are_added_and_removed_as_rows() {
        let tags = tag_model(vec![]);
        let tags = add_custom_tag(&tags, "  PvP Build  ").expect("custom tag added");

        let custom = (0..tags.row_count())
            .filter_map(|index| tags.row_data(index))
            .find(|tag| tag.id.as_str() == "pvp-build")
            .expect("custom tag row exists");
        assert!(custom.active);
        assert!(!custom.preset);
        assert_eq!(custom.kind, 5);

        let next = toggled_tags(&tags, "pvp-build");
        assert!(next.iter().all(|tag| tag.id.as_str() != "pvp-build"));
    }

    #[test]
    fn custom_tag_submit_promotes_presets_and_ignores_duplicates() {
        let tags = tag_model(vec![]);
        let tags = add_custom_tag(&tags, "testing").expect("preset tag activated");
        assert!(tag_model_has_active(&tags, "testing"));

        let first_count = tags.row_count();
        let duplicate = add_custom_tag(&tags, "testing");
        assert!(duplicate.is_some());
        assert_eq!(
            duplicate
                .expect("preset duplicate returns model")
                .row_count(),
            first_count
        );

        let tags = add_custom_tag(&tags, "trial").expect("custom tag added");
        assert_eq!(tags.row_count(), first_count + 1);
        assert!(add_custom_tag(&tags, "trial").is_none());
    }

    #[test]
    fn dependent_summaries_are_computed_from_dependency_rows() {
        let mut addons = vec![
            addon_entry(
                "LibCombat",
                "LibCombat",
                "",
                "Library Author",
                "1.0",
                "101049",
                "Library",
                "",
                "",
                false,
                true,
                false,
                0,
                "",
                0,
                "",
                0,
                "",
                0,
            ),
            addon_entry(
                "Combat Metrics",
                "CombatMetrics",
                "999",
                "Addon Author",
                "2.0",
                "101049",
                "Addon",
                "",
                "",
                false,
                false,
                false,
                0,
                "",
                0,
                "",
                0,
                "",
                0,
            ),
        ];
        addons[1].required_dependencies = dependency_model(vec![dependency_entry(
            "LibCombat",
            "v1+",
            false,
            false,
            false,
        )]);

        populate_dependent_summaries(&mut addons);

        assert!(addons[0]
            .dependent_summary
            .as_str()
            .contains("Combat Metrics depends"));
        assert_eq!(addons[1].dependent_summary.as_str(), "");
    }

    #[test]
    fn dependency_action_updates_model_state() {
        let dependencies = dependency_model(vec![dependency_entry(
            "LibCombat",
            "v82+",
            true,
            false,
            true,
        )]);

        let installed = updated_dependency_model(&dependencies, "LibCombat", true);
        let installed_dep = installed.row_data(0).expect("installed dependency exists");
        assert!(!installed_dep.missing);
        assert!(!installed_dep.outdated);
        assert!(!installed_dep.install_action);

        let removed = updated_dependency_model(&installed, "LibCombat", false);
        let removed_dep = removed.row_data(0).expect("removed dependency exists");
        assert!(removed_dep.missing);
        assert!(!removed_dep.outdated);
        assert!(removed_dep.install_action);
    }

    #[test]
    fn installed_addon_view_filters_search_and_kind() {
        let mut addons = vec![
            addon_entry(
                "CombatMetrics",
                "CombatMetrics",
                "1360",
                "Solinur",
                "1.7.7",
                "101048",
                "Addon",
                "3/3/2026",
                "Combat analysis.",
                false,
                false,
                false,
                0,
                "",
                0,
                "",
                0,
                "",
                0,
            ),
            addon_entry(
                "LibCombat",
                "LibCombat",
                "82",
                "ESOUI Community",
                "82",
                "101048",
                "Library",
                "1/22/2026",
                "Combat data library.",
                false,
                true,
                false,
                3,
                "",
                0,
                "",
                0,
                "",
                0,
            ),
            addon_entry(
                "Wizard's Wardrobe",
                "WizardsWardrobe",
                "3170",
                "Dolgubon",
                "1.19.6",
                "101048",
                "Addon",
                "2/27/2026",
                "Build manager.",
                false,
                false,
                false,
                5,
                "",
                0,
                "",
                0,
                "",
                0,
            ),
        ];
        addons[0].required_dependencies =
            dependency_model(vec![dependency_entry("MissingLib", "1", true, false, true)]);
        addons[1].favorite = true;
        addons[1].tags = set_tag_active(&addons[1].tags, "favorite", true);
        addons[1].tags = set_tag_active(&addons[1].tags, "essential", true);
        addons[2].state = 1;
        addons[2].badge = "Update".into();
        addons[2].badge_kind = 1;
        addons[2].disabled = true;

        let search = visible_addons(&addons, "dolgubon", 0, 0);
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].folder_name.as_str(), "WizardsWardrobe");

        let tag_search = visible_addons(&addons, "raid", 0, 0);
        assert_eq!(tag_search.len(), 2);
        assert!(tag_search
            .iter()
            .any(|addon| addon.folder_name.as_str() == "WizardsWardrobe"));
        assert!(tag_search
            .iter()
            .any(|addon| addon.folder_name.as_str() == "CombatMetrics"));

        let libraries = visible_addons(&addons, "", 2, 0);
        assert_eq!(libraries.len(), 1);
        assert!(libraries[0].is_library);

        let favorites = visible_addons(&addons, "", 3, 0);
        assert_eq!(favorites.len(), 1);
        assert!(favorites[0].favorite);

        let outdated = visible_addons(&addons, "", 4, 0);
        assert_eq!(outdated.len(), 1);
        assert_eq!(outdated[0].folder_name.as_str(), "WizardsWardrobe");

        let issues = visible_addons(&addons, "", 5, 0);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].folder_name.as_str(), "CombatMetrics");

        let disabled = visible_addons(&addons, "", 6, 0);
        assert_eq!(disabled.len(), 1);
        assert_eq!(disabled[0].folder_name.as_str(), "WizardsWardrobe");

        let essential = visible_addons(&addons, "", 9, 0);
        assert_eq!(essential.len(), 1);
        assert_eq!(essential[0].folder_name.as_str(), "LibCombat");

        let raid = visible_addons(&addons, "", 10, 0);
        assert_eq!(raid.len(), 2);

        let counts = addon_filter_counts(&addons);
        assert_eq!(counts.favorites, 1);
        assert_eq!(counts.outdated, 1);
        assert_eq!(counts.issues, 1);
        assert_eq!(counts.disabled, 1);
        assert_eq!(counts.essential, 1);
        assert_eq!(counts.raid, 2);
        assert_eq!(normalized_filter_mode(6, &counts), 6);
        assert_eq!(normalized_filter_mode(10, &counts), 10);

        let empty_special_counts = addon_filter_counts(&addons[..1]);
        assert_eq!(normalized_filter_mode(6, &empty_special_counts), 0);
        assert_eq!(normalized_filter_mode(9, &empty_special_counts), 0);
    }

    #[test]
    fn real_manifest_helpers_clean_eso_markup_and_dependencies() {
        let content = "\
## Title: |c00FF2BBSC's How To Kynes Aegis|r
## Author: |cFF0000BloodStainChild666
## Version: 2.1.3
## APIVersion: 101038 101039
## IsLibrary: false
## DependsOn: LibAddonMenu-2.0>=38 LibCombat
## OptionalDependsOn: OdySupportIcons
";

        assert_eq!(
            manifest_field(content, "Title").as_deref(),
            Some("BSC's How To Kynes Aegis")
        );
        assert_eq!(
            manifest_field(content, "Author").as_deref(),
            Some("BloodStainChild666")
        );
        assert!(!manifest_bool(content, "IsLibrary"));

        let required = dependency_specs(&manifest_field(content, "DependsOn"));
        assert_eq!(
            required,
            vec![
                ("LibAddonMenu-2.0".to_string(), "38".to_string()),
                ("LibCombat".to_string(), "".to_string())
            ]
        );
    }

    #[test]
    fn dependency_specs_mark_missing_against_real_folders() {
        let folders = BTreeSet::from(["libcombat".to_string()]);
        let model = dependency_model_from_specs(
            vec![
                ("LibCombat".to_string(), "".to_string()),
                ("MissingLib".to_string(), "1".to_string()),
            ],
            &folders,
        );

        let present = model.row_data(0).expect("present dependency exists");
        assert_eq!(present.name.as_str(), "LibCombat");
        assert!(!present.missing);
        assert!(!present.install_action);

        let missing = model.row_data(1).expect("missing dependency exists");
        assert_eq!(missing.name.as_str(), "MissingLib");
        assert_eq!(missing.version.as_str(), "1");
        assert!(missing.missing);
        assert!(missing.install_action);
    }

    #[test]
    fn real_addon_loader_handles_disabled_folder_suffix() {
        let root = test_temp_dir("disabled-addon-loader");
        let addon_dir = root.join("DisabledAddon.disabled");
        fs::create_dir_all(&addon_dir).expect("create disabled addon folder");
        fs::write(
            addon_dir.join("DisabledAddon.txt"),
            "## Title: Disabled Addon\n## Author: Tester\n## Version: 1.0\n",
        )
        .expect("write addon manifest");
        fs::write(
            addon_dir.join("DisabledAddon.lua"),
            "local addonName = \"DisabledAddon\"\n",
        )
        .expect("write addon lua");

        let addons = real_addon_entries(&root).expect("load real addon entries");
        assert_eq!(addons.len(), 1);
        assert_eq!(addons[0].folder_name.as_str(), "DisabledAddon");
        assert!(addons[0].disabled);
        assert_eq!(addons[0].badge3.as_str(), "Disabled");

        let resolved =
            resolve_addon_disk_path(&root, "DisabledAddon").expect("resolve disabled folder");
        assert!(resolved.ends_with("DisabledAddon.disabled"));
        assert!(addon_file_path(&root, "DisabledAddon", "DisabledAddon.lua")
            .expect("resolve disabled addon file")
            .ends_with("DisabledAddon.lua"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn edit_backup_entries_flatten_manifest_files_and_restore() {
        let root = test_temp_dir("edit-backups");
        let addon_dir = root.join("BackupAddon");
        let backup_dir = root
            .join(".kalpa-backups")
            .join("BackupAddon")
            .join("2026-07-01 12-00-00");
        fs::create_dir_all(addon_dir.join("lang")).expect("create addon folders");
        fs::create_dir_all(backup_dir.join("lang")).expect("create backup folders");
        fs::write(addon_dir.join("lang/en.lua"), "current").expect("write current file");
        fs::write(backup_dir.join("lang/en.lua"), "restored").expect("write backup file");
        fs::write(
            backup_dir.join("manifest.json"),
            r#"{
                "addonFolder": "BackupAddon",
                "backedUpAt": "2026-07-01 12:00:00",
                "updateFrom": "1.0",
                "updateTo": "2.0",
                "files": ["lang/en.lua"]
            }"#,
        )
        .expect("write backup manifest");

        let backups = edit_backup_entries(&root, "BackupAddon");
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].relative_path.as_str(), "lang/en.lua");
        assert_eq!(backups[0].update_from.as_str(), "1.0");

        restore_edit_backup_file(&root, "BackupAddon", &backups[0]).expect("restore backup");
        assert_eq!(
            fs::read_to_string(addon_dir.join("lang/en.lua")).expect("read restored file"),
            "restored"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn visible_addons_preserves_batch_selection_state() {
        let mut addons = vec![addon_entry(
            "CombatMetrics",
            "CombatMetrics",
            "1360",
            "Solinur",
            "1.7.7",
            "101048",
            "Addon",
            "3/3/2026",
            "",
            false,
            false,
            false,
            0,
            "",
            0,
            "",
            0,
            "",
            0,
        )];
        addons[0].selected = true;

        let visible = visible_addons(&addons, "combat", 0, 0);
        assert_eq!(visible.len(), 1);
        assert!(visible[0].selected);
    }

    #[test]
    fn set_tag_active_marks_existing_preset_without_toggling() {
        let tags = tag_model(vec![]);
        let tags = set_tag_active(&tags, "testing", true);
        let testing = tags.row_data(1).expect("testing preset exists");

        assert_eq!(testing.id.as_str(), "testing");
        assert!(testing.active);
        assert_eq!(testing.label.as_str(), "testing");
    }

    #[test]
    fn add_next_preset_tag_activates_first_available_non_favorite_tag() {
        let tags = tag_model(vec![]);
        let tags = add_next_preset_tag(&tags).expect("first tag added");
        assert!(tag_model_has_active(&tags, "testing"));

        let tags = add_next_preset_tag(&tags).expect("second tag added");
        assert!(tag_model_has_active(&tags, "broken"));
    }

    #[test]
    fn installed_addon_view_sorts_by_updated_desc() {
        let addons = vec![
            addon_entry(
                "Older", "Older", "1", "B", "1", "101048", "Addon", "1/1/2025", "", false, false,
                false, 0, "", 0, "", 0, "", 0,
            ),
            addon_entry(
                "Newer", "Newer", "2", "A", "1", "101048", "Addon", "3/1/2026", "", false, false,
                false, 0, "", 0, "", 0, "", 0,
            ),
        ];

        let sorted = visible_addons(&addons, "", 0, 2);
        assert_eq!(sorted[0].title.as_str(), "Newer");
        assert_eq!(date_sort_key("3/1/2026"), 20260301);
    }

    #[test]
    fn installed_addon_view_sorts_by_downloaded_desc() {
        let mut older = addon_entry(
            "Older", "Older", "1", "B", "1", "101048", "Addon", "3/1/2026", "", false, false,
            false, 0, "", 0, "", 0, "", 0,
        );
        older.installed_at = "1/1/2025".into();
        let mut newer = addon_entry(
            "Newer", "Newer", "2", "A", "1", "101048", "Addon", "1/1/2025", "", false, false,
            false, 0, "", 0, "", 0, "", 0,
        );
        newer.installed_at = "3/1/2026".into();

        let sorted = visible_addons(&[older, newer], "", 0, 3);
        assert_eq!(sorted[0].title.as_str(), "Newer");
    }

    fn test_temp_dir(name: &str) -> &'static Path {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "kalpa-slint-{name}-{}-{suffix}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        Box::leak(path.into_boxed_path())
    }

    fn sample_custom_theme(id: &str, name: &str) -> CatalogTheme {
        CatalogTheme {
            id: id.to_string(),
            name: name.to_string(),
            category: "Custom".to_string(),
            description: "Test custom theme".to_string(),
            colors: ThemeSeed {
                bg_base: "#010203".to_string(),
                background: "#111213".to_string(),
                surface: "#212223".to_string(),
                foreground: "#F1F2F3".to_string(),
                muted_foreground: "#A1A2A3".to_string(),
                primary: "#C1A24A".to_string(),
                primary_foreground: "#090807".to_string(),
                accent: "#38BDF8".to_string(),
                border: "#313233".to_string(),
                orb1: "#C1A24A".to_string(),
                orb2: "#38BDF8".to_string(),
                orb3: "#8B5CF6".to_string(),
            },
            skin_id: Some("nordic-runestone".to_string()),
        }
    }

    #[test]
    fn discover_tabs_have_model_backed_rows() {
        let installed = BTreeSet::new();

        for tab in 0..=3 {
            let entries = discover_entries_for_tab(
                tab,
                &installed,
                "combat",
                "https://www.esoui.com/downloads/info3520.html",
            );
            assert!(!entries.is_empty(), "tab {tab} should have discover rows");
            assert!(
                entries.iter().all(|entry| !entry.esoui_id.is_empty()),
                "tab {tab} rows should carry ESOUI ids"
            );
        }
    }

    #[test]
    fn discover_entries_reflect_installed_ids() {
        let installed = BTreeSet::from(["3520".to_string()]);
        let entries = popular_discover_entries(&installed);

        let code_alerts = entries
            .iter()
            .find(|entry| entry.esoui_id.as_str() == "3520")
            .expect("popular list includes Code's Combat Alerts");
        assert!(code_alerts.installed);

        let harvest_map = entries
            .iter()
            .find(|entry| entry.esoui_id.as_str() == "57")
            .expect("popular list includes HarvestMap");
        assert!(!harvest_map.installed);
    }

    #[test]
    fn discover_install_marks_matching_rows_installed() {
        let installed = BTreeSet::new();
        let model = Rc::new(VecModel::from(search_discover_entries(&installed)));

        mark_discover_installed(&model, "3520");

        let code_alerts = model.row_data(0).expect("first discover row exists");
        assert_eq!(code_alerts.esoui_id.as_str(), "3520");
        assert!(code_alerts.installed);

        let combat_metrics = model.row_data(1).expect("second discover row exists");
        assert_eq!(combat_metrics.esoui_id.as_str(), "1360");
        assert!(!combat_metrics.installed);
    }

    #[test]
    fn discover_search_filters_by_query() {
        let installed = BTreeSet::new();
        let entries = discover_entries_for_tab(0, &installed, "metrics", "");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title.as_str(), "CombatMetrics");
        assert_eq!(entries[0].rank, 1);
    }

    #[test]
    fn discover_url_input_resolves_esoui_id() {
        assert_eq!(
            esoui_id_from_input("https://www.esoui.com/downloads/info3520.html").as_deref(),
            Some("3520")
        );

        let installed = BTreeSet::new();
        let entries = discover_entries_for_tab(3, &installed, "", "57");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title.as_str(), "HarvestMap");
        assert_eq!(entries[0].rank, 1);
    }
}
