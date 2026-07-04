#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

slint::include_modules!();

#[path = "native_char_backup.rs"]
mod char_backup;

#[allow(dead_code)]
#[path = "../../../src-tauri/src/esoui.rs"]
mod esoui;

#[allow(dead_code)]
#[path = "../../../src-tauri/src/edit_backups.rs"]
mod edit_backups;

#[allow(dead_code)]
#[path = "../../../src-tauri/src/file_hashes.rs"]
mod file_hashes;

#[allow(dead_code)]
#[path = "../../../src-tauri/src/installer.rs"]
mod installer;

#[allow(dead_code)]
#[path = "../../../src-tauri/src/manifest.rs"]
mod manifest;

#[allow(dead_code)]
#[path = "../../../src-tauri/src/metadata.rs"]
mod metadata;

#[allow(dead_code)]
mod commands {
    use regex::Regex;
    use serde::Serialize;
    use std::{path::PathBuf, sync::OnceLock};

    #[derive(Debug, Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct MinionAddon {
        pub uid: u32,
        pub version: String,
        pub folders: Vec<String>,
    }

    pub fn find_minion_xml() -> Option<PathBuf> {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)?;
        let path = home.join(".minion").join("minion.xml");
        path.exists().then_some(path)
    }

    pub fn parse_minion_addons(xml_content: &str) -> Vec<MinionAddon> {
        let mut addons = Vec::new();
        static RE_ADDON: OnceLock<Regex> = OnceLock::new();
        let re_addon = RE_ADDON.get_or_init(|| {
            Regex::new(r#"<addon[^>]*uid="(\d+)"[^>]*ui-version="([^"]*)"[^>]*>"#).unwrap()
        });
        static RE_DIR: OnceLock<Regex> = OnceLock::new();
        let re_dir = RE_DIR.get_or_init(|| Regex::new(r"<dir>([^<]+)</dir>").unwrap());

        let mut current_uid = None;
        let mut current_version = String::new();
        let mut current_dirs = Vec::new();

        for line in xml_content.lines() {
            let line = line.trim();
            if let Some(caps) = re_addon.captures(line) {
                if let Some(uid) = current_uid {
                    if !current_dirs.is_empty() {
                        addons.push(MinionAddon {
                            uid,
                            version: current_version.clone(),
                            folders: current_dirs.clone(),
                        });
                    }
                }
                current_uid = caps[1].parse::<u32>().ok();
                current_version = caps[2].to_string();
                current_dirs.clear();
            } else if let Some(caps) = re_dir.captures(line) {
                current_dirs.push(caps[1].to_string());
            } else if line.contains("</addon>") {
                if let Some(uid) = current_uid {
                    if !current_dirs.is_empty() {
                        addons.push(MinionAddon {
                            uid,
                            version: current_version.clone(),
                            folders: current_dirs.clone(),
                        });
                    }
                }
                current_uid = None;
                current_dirs.clear();
            }
        }

        addons
    }
}

const ESO_RUNNING_ADDON_NOTICE: &str =
    "ESO is running; these addon changes will load after /reloadui or relog.";

fn addon_write_eso_running_warning_active(ui: &KalpaWindow) -> bool {
    ui.get_settings_warn_eso_running() && is_eso_running_blocking().unwrap_or(false)
}

fn addon_write_status_message(message: impl AsRef<str>, eso_running: bool) -> String {
    let message = message.as_ref();
    if !eso_running {
        return message.to_string();
    }

    if message.is_empty() {
        ESO_RUNNING_ADDON_NOTICE.to_string()
    } else {
        format!("{ESO_RUNNING_ADDON_NOTICE} {message}")
    }
}

fn is_eso_running_blocking() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        is_eso_running_windows()
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(false)
    }
}

#[cfg(target_os = "windows")]
fn is_eso_running_windows() -> Result<bool, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct ProcessEntry32W {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u16; 260],
    }

    const TH32CS_SNAPPROCESS: u32 = 0x00000002;
    const INVALID_HANDLE_VALUE: isize = -1;

    extern "system" {
        fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> isize;
        fn Process32FirstW(hSnapshot: isize, lppe: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(hSnapshot: isize, lppe: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(hObject: isize) -> i32;
    }

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err("Failed to create process snapshot".to_string());
        }

        let mut entry: ProcessEntry32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<ProcessEntry32W>() as u32;

        let mut found = false;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
                let name = OsString::from_wide(&entry.szExeFile[..len])
                    .to_string_lossy()
                    .to_ascii_lowercase();
                if name == "eso64.exe" || name == "eso.exe" {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snap);
        Ok(found)
    }
}

#[allow(dead_code)]
#[path = "../../../src-tauri/src/safe_migration.rs"]
mod safe_migration;

#[allow(dead_code, unused_imports)]
#[path = "../../../src-tauri/src/saved_variables/mod.rs"]
mod saved_variables;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slint::{
    Color, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel,
};
use std::{
    cell::RefCell,
    cmp::Ordering as CmpOrdering,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportedCustomTheme {
    name: Option<String>,
    description: Option<String>,
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
    native_performance_mode: bool,
    official_uploader: bool,
    auto_open_analysis: bool,
    conflict_policy: i32,
    uploader_region: i32,
    uploader_visibility: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct NativeInstalledPackRef {
    pack_id: String,
    title: String,
    pack_type: String,
    author_name: String,
    addon_count: usize,
    installed_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportEntry {
    esoui_id: u32,
    folder_name: String,
    version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportData {
    version: u32,
    addons: Vec<ExportEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct NativePackAddonEntry {
    esoui_id: u32,
    name: String,
    #[serde(default = "default_true")]
    required: bool,
    note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
struct NativeHubPack {
    id: String,
    #[serde(default)]
    author_id: String,
    author_name: String,
    #[serde(default)]
    is_anonymous: bool,
    title: String,
    description: String,
    pack_type: String,
    addons: serde_json::Value,
    vote_count: i64,
    #[serde(default)]
    install_count: i64,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    user_voted: Option<bool>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct NativePackListResponse {
    packs: Vec<NativeHubPack>,
    page: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct NativePackSingleResponse {
    pack: NativeHubPack,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeSharedPackResponse {
    pack: NativeSharedPackBody,
    shared_by: String,
    shared_at: String,
    #[serde(rename = "expiresAt")]
    _expires_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeSharedPackBody {
    title: String,
    description: String,
    pack_type: String,
    tags: Vec<String>,
    addons: Vec<NativePackAddonEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeEsoPackFile {
    format: String,
    version: u32,
    pack: NativeSharedPackBody,
    shared_at: String,
    shared_by: String,
    #[serde(default)]
    settings: HashMap<String, NativeAddonSettings>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeAddonSettings {
    encoding: String,
    lua: String,
    #[serde(default)]
    #[serde(rename = "originalBytes")]
    _original_bytes: usize,
    #[serde(default)]
    #[serde(rename = "scrubbedBytes")]
    _scrubbed_bytes: usize,
    #[serde(default)]
    #[serde(rename = "finalBytes")]
    _final_bytes: usize,
}

#[derive(Debug, Clone)]
struct NativePackDetailData {
    entry: PackHubEntry,
    addons: Vec<PackHubAddonEntry>,
}

#[derive(Debug, Clone)]
struct NativePackFileImportData {
    detail: NativePackDetailData,
    settings: HashMap<String, NativeAddonSettings>,
}

#[derive(Debug, Clone)]
struct NativePackPageData {
    entries: Vec<PackHubEntry>,
    page: i64,
    has_more: bool,
}

#[derive(Debug, Clone)]
struct NativePackInstallResult {
    rows: Vec<PackHubAddonEntry>,
    installed: usize,
    failed: usize,
    folders: usize,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct NativeSvImportResult {
    applied: Vec<String>,
    skipped: Vec<String>,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct NativeAppUpdateManifest {
    version: String,
    platforms: HashMap<String, NativeAppUpdatePlatform>,
}

#[derive(Debug, Clone, Deserialize)]
struct NativeAppUpdatePlatform {
    url: String,
    signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeAppUpdateInfo {
    version: String,
    url: String,
    signature: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NativeAddonUpdateCheck {
    folder_name: String,
    remote_version: String,
    has_update: bool,
    remote_last_update: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeAddonUpdateTarget {
    folder_name: String,
    esoui_id: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NativeAddonUpdateApplyResult {
    checks: Vec<NativeAddonUpdateCheck>,
    completed: Vec<String>,
    conflicts: Vec<NativePendingConflict>,
    failed: Vec<String>,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NativeConflictReport {
    safe_file_count: usize,
    auto_kept_files: Vec<String>,
    conflicts: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NativePendingConflict {
    folder_name: String,
    esoui_id: u32,
    update_version: String,
    title: String,
    download_url: String,
    safe_file_count: usize,
    auto_kept_files: Vec<String>,
    conflicts: Vec<String>,
    decisions: HashMap<String, i32>,
    zip_path: PathBuf,
    zip_hashes: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
struct NativeImportResult {
    installed: Vec<String>,
    failed: Vec<String>,
    skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeApiCompatInfo {
    game_api_version: u32,
    outdated_addons: Vec<String>,
    up_to_date_addons: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NativeUploaderLog {
    path: PathBuf,
    file_name: String,
    size_bytes: u64,
    modified_epoch: u64,
    active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NativeUploaderPreflight {
    sessions: usize,
    fights: Vec<NativeUploaderFight>,
    total_fights: usize,
    truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NativeUploaderFight {
    index: usize,
    start_ms: u64,
    end_ms: u64,
}

fn default_true() -> bool {
    true
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct NativeHashManifest {
    addon_folder: String,
    #[serde(default)]
    esoui_ids: Vec<u32>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    esoui_id: u32,
    recorded_at: String,
    installed_version: String,
    files: HashMap<String, String>,
    #[serde(default)]
    modified_files: Vec<String>,
}

impl Default for NativeSettings {
    fn default() -> Self {
        Self {
            auto_update: false,
            warn_eso_running: true,
            native_performance_mode: true,
            official_uploader: false,
            auto_open_analysis: false,
            conflict_policy: 0,
            uploader_region: 1,
            uploader_visibility: 2,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeRenderPreset {
    LowMemory,
    Standard,
}

const STORE_KEY_ACTIVE_THEME: &str = "appearance.activeThemeId";
const STORE_KEY_ADDONS_PATH: &str = "addonsPath";
const STORE_KEY_CUSTOM_THEMES: &str = "appearance.customThemes";
const STORE_KEY_INSTALLED_PACKS: &str = "installed_packs";
const STORE_KEY_PERFORMANCE_MODE: &str = "performanceMode";
const APP_UPDATE_MANIFEST_URL: &str =
    "https://github.com/ESO-Toolkit/kalpa/releases/latest/download/latest.json";
const TAURI_CONF_JSON: &str = include_str!("../../../src-tauri/tauri.conf.json");

#[derive(Debug, PartialEq, Eq)]
struct NativeRenderConfig {
    backend: String,
    preset: NativeRenderPreset,
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
    static FILE_ENTRY_CACHE: RefCell<HashMap<String, Vec<FileEntry>>> = RefCell::new(HashMap::new());
    static ORB_SKIN_CACHE: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
}

type NativePendingConflictStore = Arc<Mutex<HashMap<String, NativePendingConflict>>>;

static PENDING_NATIVE_CONFLICTS: OnceLock<NativePendingConflictStore> = OnceLock::new();

#[derive(Clone)]
struct AddonModels {
    all: Rc<RefCell<Vec<AddonEntry>>>,
    visible: Rc<VecModel<AddonEntry>>,
    view_key: Rc<RefCell<Option<AddonViewKey>>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SvmProfileFile {
    file_name: String,
    addon_name: String,
    profiles: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SvmCopySelection {
    file_name: String,
    addon_name: String,
    source_key: String,
    dest_key: String,
}

#[derive(Clone, Debug, Default)]
struct SvmCopyState {
    files: Vec<SvmProfileFile>,
    file_index: usize,
    source_index: usize,
    dest_index: usize,
    status: String,
}

impl SvmCopyState {
    fn replace_files(&mut self, files: Vec<SvmProfileFile>) {
        self.files = files;
        self.file_index = 0;
        self.source_index = 0;
        self.dest_index = 0;
        self.status.clear();
        self.normalize();
    }

    fn normalize(&mut self) {
        if self.files.is_empty() {
            self.file_index = 0;
            self.source_index = 0;
            self.dest_index = 0;
            return;
        }

        self.file_index %= self.files.len();
        let source_count = self
            .selected_file()
            .map(|file| file.profiles.len())
            .unwrap_or_default();
        if source_count == 0 {
            self.source_index = 0;
            self.dest_index = 0;
            return;
        }

        self.source_index %= source_count;
        let dest_count = self.destination_choices().len();
        if dest_count == 0 {
            self.dest_index = 0;
        } else {
            self.dest_index %= dest_count;
        }
    }

    fn select_next_file(&mut self) {
        if !self.files.is_empty() {
            self.file_index = (self.file_index + 1) % self.files.len();
        }
        self.source_index = 0;
        self.dest_index = 0;
        self.status.clear();
        self.normalize();
    }

    fn select_next_source(&mut self) {
        let profile_count = self
            .selected_file()
            .map(|file| file.profiles.len())
            .unwrap_or_default();
        if profile_count > 0 {
            self.source_index = (self.source_index + 1) % profile_count;
        }
        self.dest_index = 0;
        self.status.clear();
        self.normalize();
    }

    fn select_next_dest(&mut self) {
        let dest_count = self.destination_choices().len();
        if dest_count > 0 {
            self.dest_index = (self.dest_index + 1) % dest_count;
        }
        self.status.clear();
        self.normalize();
    }

    fn selected_file(&self) -> Option<&SvmProfileFile> {
        self.files.get(self.file_index)
    }

    fn source_key(&self) -> Option<&str> {
        self.selected_file()
            .and_then(|file| file.profiles.get(self.source_index))
            .map(String::as_str)
    }

    fn destination_choices(&self) -> Vec<String> {
        let Some(source) = self.source_key() else {
            return Vec::new();
        };
        self.files
            .iter()
            .flat_map(|file| file.profiles.iter())
            .filter(|profile| profile.as_str() != source)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn dest_key(&self) -> Option<String> {
        self.destination_choices().get(self.dest_index).cloned()
    }

    fn selection(&self) -> Option<SvmCopySelection> {
        let file = self.selected_file()?;
        let source_key = self.source_key()?.to_string();
        let dest_key = self.dest_key()?;
        Some(SvmCopySelection {
            file_name: file.file_name.clone(),
            addon_name: file.addon_name.clone(),
            source_key,
            dest_key,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SvmEditorFile {
    file_name: String,
    addon_name: String,
}

#[derive(Clone, Debug, Default)]
struct SvmEditorState {
    files: Vec<SvmEditorFile>,
    file_index: usize,
    tree: Option<saved_variables::SvTreeNode>,
    stamp: Option<saved_variables::SvFileStamp>,
    selected_path: Vec<String>,
    tree_expanded_all: bool,
    tree_filter: String,
    dirty: bool,
    message: String,
}

impl SvmEditorState {
    fn replace_files(&mut self, files: Vec<SvmEditorFile>) {
        let previous_file = self.selected_file().map(|file| file.file_name.clone());
        self.files = files;
        self.file_index = previous_file
            .and_then(|name| self.files.iter().position(|file| file.file_name == name))
            .unwrap_or(0);
        self.normalize_file_index();
    }

    fn normalize_file_index(&mut self) {
        if self.files.is_empty() {
            self.file_index = 0;
        } else {
            self.file_index %= self.files.len();
        }
    }

    fn selected_file(&self) -> Option<&SvmEditorFile> {
        self.files.get(self.file_index)
    }

    fn select_next_file(&mut self) {
        if !self.files.is_empty() {
            self.file_index = (self.file_index + 1) % self.files.len();
        }
    }

    fn selected_file_name(&self) -> Option<String> {
        self.selected_file().map(|file| file.file_name.clone())
    }
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

#[derive(Clone, Debug, Default)]
struct DiscoverBrowseState {
    popular_sort: i32,
    popular_page: u32,
    popular_has_more: bool,
    categories: Vec<esoui::EsouiCategory>,
    selected_category_index: usize,
    category_sort: i32,
    category_page: u32,
    category_has_more: bool,
}

impl DiscoverBrowseState {
    fn normalize(&mut self) {
        self.popular_sort = self.popular_sort.clamp(0, 1);
        self.category_sort = self.category_sort.clamp(0, 2);
        if self.categories.is_empty() {
            self.selected_category_index = 0;
        } else {
            self.selected_category_index %= self.categories.len();
        }
    }

    fn reset_popular_page(&mut self) {
        self.popular_page = 0;
        self.popular_has_more = false;
        self.normalize();
    }

    fn reset_category_page(&mut self) {
        self.category_page = 0;
        self.category_has_more = false;
        self.normalize();
    }

    fn next_popular_page_snapshot(&self) -> Self {
        let mut next = self.clone();
        next.popular_page = next.popular_page.saturating_add(1);
        next.normalize();
        next
    }

    fn next_category_page_snapshot(&self) -> Self {
        let mut next = self.clone();
        next.category_page = next.category_page.saturating_add(1);
        next.normalize();
        next
    }

    fn selected_category(&self) -> Option<&esoui::EsouiCategory> {
        self.categories.get(self.selected_category_index)
    }

    fn replace_categories(&mut self, categories: Vec<esoui::EsouiCategory>) {
        let previous_id = self.selected_category().map(|category| category.id);
        self.categories = categories;
        self.selected_category_index = previous_id
            .and_then(|id| {
                self.categories
                    .iter()
                    .position(|category| category.id == id)
            })
            .or_else(|| {
                default_discover_category_id(&self.categories).and_then(|id| {
                    self.categories
                        .iter()
                        .position(|category| category.id == id)
                })
            })
            .unwrap_or(0);
        self.normalize();
    }

    fn select_next_category(&mut self) {
        if !self.categories.is_empty() {
            self.selected_category_index =
                (self.selected_category_index + 1) % self.categories.len();
        }
        self.reset_category_page();
        self.normalize();
    }

    fn select_category_index(&mut self, index: usize) {
        if index < self.categories.len() {
            self.selected_category_index = index;
        }
        self.reset_category_page();
        self.normalize();
    }

    fn select_next_category_sort(&mut self) {
        self.category_sort = (self.category_sort + 1) % 3;
        self.reset_category_page();
        self.normalize();
    }

    fn select_category_sort(&mut self, sort: i32) {
        self.category_sort = sort.clamp(0, 2);
        self.reset_category_page();
        self.normalize();
    }
}

#[derive(Clone, Debug, Default)]
struct PackHubBrowseState {
    query: String,
    type_filter: i32,
    sort: i32,
    page: i64,
}

impl PackHubBrowseState {
    fn normalize(&mut self) {
        self.type_filter = self.type_filter.clamp(0, 3);
        self.sort = self.sort.clamp(0, 2);
        self.page = self.page.max(1);
    }

    fn next_type_filter(&mut self) {
        self.type_filter = (self.type_filter + 1) % 4;
        self.page = 1;
        self.normalize();
    }

    fn next_sort(&mut self) {
        self.sort = (self.sort + 1) % 3;
        self.page = 1;
        self.normalize();
    }

    fn reset_page(&mut self) {
        self.page = 1;
        self.normalize();
    }

    fn next_page_snapshot(&self) -> Self {
        let mut next = self.clone();
        next.page = next.page.saturating_add(1).max(1);
        next.normalize();
        next
    }
}

#[derive(Default)]
struct PackHubCreateState {
    filter: String,
    selected: Vec<PackHubCreateAddonEntry>,
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
    let render_config = native_render_config();

    slint::BackendSelector::new()
        .backend_name(render_config.backend.clone().into())
        .select()?;

    let ui = KalpaWindow::new()?;
    place_demo_window(&ui);
    let custom_themes = Rc::new(RefCell::new(read_custom_themes()));
    set_theme_gallery(&ui, &custom_themes.borrow());
    clear_discover_screenshots(&ui);

    let addon_models = apply_mock_data(&ui);
    let pending_conflicts = Arc::new(Mutex::new(HashMap::new()));
    let _ = PENDING_NATIVE_CONFLICTS.set(pending_conflicts);
    apply_initial_native_settings(&ui);
    apply_runtime_flags(&ui, render_config.preset);
    let active_theme_id = apply_initial_theme(&ui);
    ui.set_active_theme_id(active_theme_id.into());
    seed_initial_theme_draft(&ui, &custom_themes.borrow());
    apply_backup_restore_model(&ui);
    apply_pack_hub_model(&ui, fallback_pack_hub_entries());
    apply_installed_pack_refs(&ui, read_installed_pack_refs());
    clear_pack_hub_import_model(&ui);
    apply_addon_view(&ui, &addon_models);
    let discover_installed_ids = Arc::new(Mutex::new(installed_discover_ids(
        &addon_models.all.borrow(),
    )));
    let discover_model = Rc::new(RefCell::new(apply_discover_data(
        &ui,
        ui.get_discover_tab(),
        &discover_installed_snapshot(&discover_installed_ids),
    )));
    refresh_file_browser(&ui);

    wire_window_controls(&ui);
    wire_file_browser(&ui);
    wire_addon_filters(&ui, addon_models.clone());
    wire_header_actions(&ui, addon_models.clone());
    wire_tag_editor(&ui, addon_models.clone());
    wire_batch_actions(&ui, addon_models.clone());
    wire_context_actions(&ui, addon_models.clone());
    let settings_models = addon_models.clone();
    let safety_models = addon_models.clone();
    let migration_models = addon_models.clone();
    let discover_addon_models = addon_models.clone();
    wire_detail_actions(&ui, addon_models);
    wire_discover(
        &ui,
        discover_model,
        discover_installed_ids,
        discover_addon_models,
    );
    wire_theme_actions(&ui, custom_themes);
    wire_settings_actions(&ui, settings_models);
    wire_uploader_actions(&ui);
    wire_backup_restore_actions(&ui);
    wire_character_actions(&ui);
    wire_safety_actions(&ui, safety_models);
    wire_migration_actions(&ui, migration_models);
    if ui.get_pack_hub_open() {
        ui.invoke_open_pack_hub();
    }
    if ui.get_characters_open() {
        ui.invoke_open_characters();
    }
    if ui.get_safety_open() {
        ui.invoke_open_safety();
    }
    if ui.get_migration_open() {
        ui.invoke_open_migration();
    }
    start_native_app_update_check(ui.as_weak(), true);
    ui.run()
}

fn native_render_config() -> NativeRenderConfig {
    render_config_from_inputs(
        std::env::var("KALPA_RENDER_PRESET").ok().as_deref(),
        std::env::var("KALPA_SLINT_BACKEND").ok().as_deref(),
        std::env::var("SLINT_BACKEND").ok().as_deref(),
    )
}

fn render_config_from_inputs(
    preset_env: Option<&str>,
    backend_env: Option<&str>,
    slint_backend_env: Option<&str>,
) -> NativeRenderConfig {
    let explicit_preset = preset_env.and_then(parse_render_preset);
    let backend = backend_env
        .or(slint_backend_env)
        .map(str::to_string)
        .unwrap_or_else(|| default_backend_for_preset(explicit_preset.unwrap_or_default()).into());
    let preset = explicit_preset.unwrap_or_else(|| render_preset_for_backend(&backend));

    NativeRenderConfig { backend, preset }
}

impl Default for NativeRenderPreset {
    fn default() -> Self {
        Self::Standard
    }
}

fn parse_render_preset(value: &str) -> Option<NativeRenderPreset> {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" | "low-memory" | "low_memory" | "memory" | "software" => {
            Some(NativeRenderPreset::LowMemory)
        }
        "standard" | "fidelity" | "quality" | "skia" | "femtovg" => {
            Some(NativeRenderPreset::Standard)
        }
        _ => None,
    }
}

fn default_backend_for_preset(preset: NativeRenderPreset) -> &'static str {
    match preset {
        NativeRenderPreset::LowMemory => "winit-software",
        NativeRenderPreset::Standard => "winit-femtovg",
    }
}

fn render_preset_for_backend(backend: &str) -> NativeRenderPreset {
    match backend.to_ascii_lowercase().as_str() {
        "winit-skia" | "skia" | "winit-femtovg" | "femtovg" => NativeRenderPreset::Standard,
        _ => NativeRenderPreset::LowMemory,
    }
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
    let auto_place = std::env::var("KALPA_NATIVE_AUTO_PLACE").ok();
    if !native_auto_place_enabled(auto_place.as_deref()) {
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
fn native_auto_place_enabled(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "true" | "TRUE" | "yes" | "YES"))
}

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
    apply_saved_variables_model(ui, &all.borrow());
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
    let store = metadata::load_metadata(addons_root);
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
            if let Some(meta) = store.addons.get(draft.entry.folder_name.as_str()) {
                hydrate_addon_from_metadata(&mut draft.entry, meta);
            }
            draft.entry.required_dependencies =
                dependency_model_from_specs(draft.required_dependencies, &folder_names);
            draft.entry.optional_dependencies =
                dependency_model_from_specs(draft.optional_dependencies, &folder_names);
            draft.entry
        })
        .collect::<Vec<_>>();

    Ok(addons)
}

fn hydrate_addon_from_metadata(entry: &mut AddonEntry, meta: &metadata::AddonMetadata) {
    if meta.esoui_id > 0 {
        entry.esoui_id = meta.esoui_id.to_string().into();
    }
    if entry.version.is_empty() && !meta.installed_version.trim().is_empty() {
        entry.version = meta.installed_version.as_str().into();
        entry.meta = addon_meta(entry.version.as_str(), entry.author.as_str()).into();
    }
    if meta.esoui_last_update > 0 {
        entry.last_updated = date_label_from_epoch_millis(meta.esoui_last_update).into();
    }
    if let Some(installed_at) = pack_iso_date_label(&meta.installed_at) {
        entry.installed_at = installed_at.into();
    }
    if !meta.tags.is_empty() {
        entry.tags = tag_model_from_ids(&meta.tags);
        entry.favorite = tag_model_has_active(&entry.tags, "favorite");
    }
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

fn toggle_visible_addon_selection(
    ui: &KalpaWindow,
    models: &AddonModels,
    visible_index: usize,
) -> bool {
    let Some(mut addon) = models.visible.row_data(visible_index) else {
        return false;
    };

    addon.selected = !addon.selected;
    let folder_name = addon.folder_name.to_string();
    let selected = addon.selected;
    let mut updated_master = false;
    {
        let mut all = models.all.borrow_mut();
        if let Some(master) = all
            .iter_mut()
            .find(|entry| entry.folder_name.as_str() == folder_name)
        {
            master.selected = selected;
            updated_master = true;
        }
    }

    if !updated_master {
        return false;
    }

    models.visible.set_row_data(visible_index, addon);
    set_batch_state(ui, models);
    true
}

fn clear_addon_selection(ui: &KalpaWindow, models: &AddonModels) {
    {
        let mut all = models.all.borrow_mut();
        for addon in all.iter_mut() {
            addon.selected = false;
        }
    }

    for index in 0..models.visible.row_count() {
        let Some(mut addon) = models.visible.row_data(index) else {
            continue;
        };
        if addon.selected {
            addon.selected = false;
            models.visible.set_row_data(index, addon);
        }
    }

    set_batch_state(ui, models);
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

fn clear_addon_update_state(addon: &mut AddonEntry) {
    if addon.state == 1 {
        addon.state = if addon.is_library { 3 } else { 0 };
    }
    if addon.badge.as_str() == "Update" {
        addon.badge = "".into();
        addon.badge_kind = 0;
    }
}

fn addon_update_entries_from_model(
    updates: &ModelRc<AddonUpdateCheckEntry>,
) -> Vec<AddonUpdateCheckEntry> {
    (0..updates.row_count())
        .filter_map(|index| updates.row_data(index))
        .collect()
}

fn apply_addon_update_check_results(
    models: &AddonModels,
    updates: &[AddonUpdateCheckEntry],
) -> usize {
    let update_by_folder = updates
        .iter()
        .map(|update| (update.folder_name.to_string(), update.clone()))
        .collect::<HashMap<_, _>>();

    let mut available = 0usize;
    for addon in models.all.borrow_mut().iter_mut() {
        clear_addon_update_state(addon);

        let Some(update) = update_by_folder.get(addon.folder_name.as_str()) else {
            continue;
        };

        if !update.last_updated.is_empty() {
            addon.last_updated = update.last_updated.clone();
        }
        if update.has_update {
            available += 1;
            addon.state = 1;
            addon.badge = "Update".into();
            addon.badge_kind = 1;
        }
    }

    available
}

fn native_update_targets(models: &AddonModels) -> Vec<NativeAddonUpdateTarget> {
    let all = models.all.borrow();
    let selected_count = all.iter().filter(|addon| addon.selected).count();
    all.iter()
        .filter(|addon| addon_has_update(addon))
        .filter(|addon| selected_count == 0 || addon.selected)
        .filter_map(|addon| {
            let esoui_id = addon.esoui_id.parse::<u32>().ok()?;
            Some(NativeAddonUpdateTarget {
                folder_name: addon.folder_name.to_string(),
                esoui_id,
            })
        })
        .collect()
}

fn update_apply_status_message(result: &NativeAddonUpdateApplyResult) -> String {
    let completed = result.completed.len();
    let conflicts = result.conflicts.len();
    let failed = result.failed.len();

    let mut parts = Vec::new();
    if completed > 0 {
        parts.push(format!(
            "Updated {completed} addon{}",
            if completed == 1 { "" } else { "s" }
        ));
    }
    if conflicts > 0 {
        parts.push(format!(
            "{conflicts} need{} conflict review",
            if conflicts == 1 { "s" } else { "" }
        ));
    }
    if failed > 0 {
        parts.push(format!(
            "{failed} failed{}",
            result
                .errors
                .first()
                .map(|error| format!(" ({error})"))
                .unwrap_or_default()
        ));
    }

    if parts.is_empty() {
        "No safe addon updates were applied.".to_string()
    } else {
        format!("{}.", parts.join("; "))
    }
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
    if let Some(key) = slash_date_sort_key(value) {
        return key;
    }
    if let Some(key) = iso_date_sort_key(value) {
        return key;
    }
    named_date_sort_key(value).unwrap_or(0)
}

fn slash_date_sort_key(value: &str) -> Option<i32> {
    let parts = value
        .split('/')
        .filter_map(|part| part.parse::<i32>().ok())
        .collect::<Vec<_>>();

    if parts.len() == 3 {
        Some(parts[2] * 10_000 + parts[0] * 100 + parts[1])
    } else {
        None
    }
}

fn iso_date_sort_key(value: &str) -> Option<i32> {
    let date = value.get(0..10)?;
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<i32>().ok()?;
    let day = parts.next()?.parse::<i32>().ok()?;
    Some(year * 10_000 + month * 100 + day)
}

fn named_date_sort_key(value: &str) -> Option<i32> {
    let cleaned = value.replace(',', "");
    let mut parts = cleaned.split_whitespace();
    let month = month_number(parts.next()?)?;
    let day = parts.next()?.parse::<i32>().ok()?;
    let year = parts.next()?.parse::<i32>().ok()?;
    Some(year * 10_000 + month * 100 + day)
}

fn month_number(month: &str) -> Option<i32> {
    match month {
        "Jan" | "January" => Some(1),
        "Feb" | "February" => Some(2),
        "Mar" | "March" => Some(3),
        "Apr" | "April" => Some(4),
        "May" => Some(5),
        "Jun" | "June" => Some(6),
        "Jul" | "July" => Some(7),
        "Aug" | "August" => Some(8),
        "Sep" | "September" => Some(9),
        "Oct" | "October" => Some(10),
        "Nov" | "November" => Some(11),
        "Dec" | "December" => Some(12),
        _ => None,
    }
}

fn date_label_from_epoch_millis(epoch_millis: u64) -> String {
    format_short_date(epoch_millis / 1_000)
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

fn append_discover_results(ui: &KalpaWindow, entries: Vec<DiscoverEntry>) {
    let mut rows = (0..ui.get_discover_results().row_count())
        .filter_map(|index| ui.get_discover_results().row_data(index))
        .collect::<Vec<_>>();
    rows.extend(entries);
    ui.set_discover_results(Rc::new(VecModel::from(rows)).into());
}

fn filter_category_discover_entries(entries: &[DiscoverEntry], query: &str) -> Vec<DiscoverEntry> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return entries.to_vec();
    }

    entries
        .iter()
        .filter(|entry| {
            entry
                .title
                .to_string()
                .to_ascii_lowercase()
                .contains(&query)
                || entry
                    .author
                    .to_string()
                    .to_ascii_lowercase()
                    .contains(&query)
        })
        .cloned()
        .collect()
}

fn apply_category_discover_results(ui: &KalpaWindow, entries: &[DiscoverEntry]) {
    let rows =
        filter_category_discover_entries(entries, ui.get_discover_category_filter().as_str());
    let selected_index = ui
        .get_selected_discover_index()
        .max(0)
        .min(rows.len().saturating_sub(1) as i32);
    ui.set_selected_discover_index(selected_index);
    ui.set_discover_results(Rc::new(VecModel::from(rows)).into());
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

fn discover_popular_sort_key(sort: i32) -> &'static str {
    match sort {
        1 => "newest",
        _ => "downloads",
    }
}

fn discover_category_sort_key(sort: i32) -> &'static str {
    match sort {
        1 => "newest",
        2 => "name",
        _ => "downloads",
    }
}

fn discover_category_sort_label(sort: i32) -> &'static str {
    match sort {
        1 => "Recently Updated",
        2 => "Name",
        _ => "Most Popular",
    }
}

fn discover_category_label(state: &DiscoverBrowseState) -> String {
    state
        .selected_category()
        .map(|category| category.name.clone())
        .unwrap_or_else(|| "Combat".to_string())
}

fn discover_category_options(state: &DiscoverBrowseState) -> Vec<DiscoverSelectOption> {
    state
        .categories
        .iter()
        .enumerate()
        .map(|(index, category)| DiscoverSelectOption {
            label: category.name.clone().into(),
            depth: category.depth.min(3) as i32,
            selected: index == state.selected_category_index,
        })
        .collect()
}

fn discover_category_sort_options(state: &DiscoverBrowseState) -> Vec<DiscoverSelectOption> {
    (0..3)
        .map(|sort| DiscoverSelectOption {
            label: discover_category_sort_label(sort).into(),
            depth: 0,
            selected: sort == state.category_sort,
        })
        .collect()
}

fn apply_discover_browse_state(ui: &KalpaWindow, state: &DiscoverBrowseState) {
    ui.set_discover_popular_sort(state.popular_sort);
    ui.set_discover_category_label(discover_category_label(state).into());
    ui.set_discover_category_sort_label(discover_category_sort_label(state.category_sort).into());
    ui.set_discover_category_options(
        Rc::new(VecModel::from(discover_category_options(state))).into(),
    );
    ui.set_discover_category_sort_options(
        Rc::new(VecModel::from(discover_category_sort_options(state))).into(),
    );
    let tab = ui.get_discover_tab();
    ui.set_discover_browse_has_more(match tab {
        1 => state.popular_has_more,
        2 => state.category_has_more,
        _ => false,
    });
}

fn load_discover_popular_page(
    mut state: DiscoverBrowseState,
    installed_ids: &BTreeSet<String>,
) -> Result<(DiscoverBrowseState, Vec<DiscoverEntry>, bool), String> {
    state.normalize();
    let page = esoui::browse_popular(
        state.popular_page,
        discover_popular_sort_key(state.popular_sort),
    )?;
    let entries = discover_entries_from_search_results_with_offset(
        page.results,
        installed_ids,
        state.popular_page as usize * 25,
    );
    state.popular_has_more = page.has_more;
    Ok((state, entries, page.has_more))
}

fn load_discover_category_page(
    mut state: DiscoverBrowseState,
    installed_ids: &BTreeSet<String>,
) -> Result<(DiscoverBrowseState, Vec<DiscoverEntry>, bool), String> {
    const CATEGORY_PAGE_SIZE: usize = 20;
    if state.categories.is_empty() {
        state.replace_categories(esoui::fetch_categories()?);
    } else {
        state.normalize();
    }

    if state.selected_category().is_none() {
        return Err("ESOUI did not return any addon categories.".to_string());
    }

    let category_id = state
        .selected_category()
        .map(|category| category.id)
        .ok_or_else(|| "ESOUI did not return any addon categories.".to_string())?;
    let results = esoui::browse_category(
        category_id,
        state.category_page,
        discover_category_sort_key(state.category_sort),
    )?;
    let has_more = results.len() >= CATEGORY_PAGE_SIZE;
    let entries = discover_entries_from_search_results_with_offset(
        results,
        installed_ids,
        state.category_page as usize * CATEGORY_PAGE_SIZE,
    );
    state.category_has_more = has_more;
    Ok((state, entries, has_more))
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
    let mut entries = vec![
        discover_entry(
            "1346",
            "Dolgubon's Lazy Writ Crafter",
            "Dolgubon",
            "TradeSkill Mods",
            "4.0.5.6.4",
            "9,238,875",
            "224,924",
            "2,167",
            "04/07/26 04:22 AM",
            "04/16/16 07:34 PM",
            "ca9e42fe27c133b66051d959a23161f0",
            "Season Zero (11.3.0)",
            "Crafting writ automation, rewards tracking, and compact workflow helpers for daily crafting.",
            1,
            installed_ids,
        ),
        discover_entry(
            "3317",
            "Character Knowledge (Research Assistant)",
            "Dolgubon",
            "Character Advancement",
            "2.1.9",
            "6,310,011",
            "198,402",
            "1,684",
            "04/06/26 09:10 PM",
            "06/21/20 10:14 PM",
            "3e2c04a97821d",
            "Season Zero (11.3.0)",
            "Account-wide research, recipe, motif, and collectible knowledge tracking.",
            2,
            installed_ids,
        ),
        discover_entry(
            "2045",
            "Action Duration Reminder",
            "Cloudor",
            "Action Bar Mods",
            "4.3.1",
            "5,904,102",
            "181,556",
            "1,391",
            "04/05/26 01:44 PM",
            "09/04/17 06:22 PM",
            "9a7d0cb338a41",
            "Season Zero (11.3.0)",
            "Action bar duration overlays and reminders for buffs, dots, and ability timers.",
            3,
            installed_ids,
        ),
        discover_entry(
            "818",
            "Lui Extended",
            "ArtOfShred",
            "Graphic UI Mods",
            "6.9.8",
            "5,184,339",
            "154,818",
            "1,012",
            "04/03/26 02:17 AM",
            "11/21/14 03:33 AM",
            "c33841af5a6d0",
            "Season Zero (11.3.0)",
            "Modular UI extensions for combat text, unit frames, buff displays, and alerts.",
            4,
            installed_ids,
        ),
        discover_entry(
            "3287",
            "Advanced Filters - Updated",
            "Votan",
            "Bags, Bank, Inventory",
            "1.6.8",
            "4,812,490",
            "142,104",
            "944",
            "04/01/26 08:31 PM",
            "03/09/20 11:20 AM",
            "58a88c32bd3e",
            "Season Zero (11.3.0)",
            "Inventory filter extensions for bags, bank, crafting, housing, and trading workflows.",
            5,
            installed_ids,
        ),
        discover_entry(
            "2194",
            "EsoTW Traditional Chinese",
            "EsoTW Team",
            "ESO Tools & Utilities",
            "12.4.0",
            "4,307,220",
            "119,887",
            "801",
            "03/28/26 10:54 PM",
            "08/02/18 02:42 AM",
            "ce1d4593b1e4",
            "Season Zero (11.3.0)",
            "Traditional Chinese localization data and utility support for Elder Scrolls Online.",
            6,
            installed_ids,
        ),
        discover_entry(
            "1725",
            "PersonalAssistant (Banking, Junk, Loot, Repair)",
            "Klingo",
            "Bags, Bank, Inventory",
            "2026.03.29",
            "4,106,775",
            "116,204",
            "739",
            "03/26/26 07:13 AM",
            "01/11/17 06:45 PM",
            "13a77a91b54e",
            "Season Zero (11.3.0)",
            "Automation helpers for banking, junk handling, repairs, loot rules, and inventory routines.",
            7,
            installed_ids,
        ),
        discover_entry(
            "2739",
            "TDAddon",
            "TrollDoll",
            "PvP",
            "3.2.6",
            "3,904,618",
            "104,230",
            "681",
            "03/22/26 01:18 AM",
            "05/14/19 08:20 PM",
            "719f8eac49db",
            "Season Zero (11.3.0)",
            "PvP utility helpers and compact combat quality-of-life tools.",
            8,
            installed_ids,
        ),
        discover_entry(
            "3860",
            "Hermes - Tools Tome",
            "Calamath",
            "ESO Tools & Utilities",
            "1.9.3",
            "3,612,904",
            "97,514",
            "623",
            "03/20/26 04:12 PM",
            "11/03/21 09:54 AM",
            "a6d3af8bb05d",
            "Season Zero (11.3.0)",
            "A broad utility collection for menus, slash commands, debug aids, and addon authors.",
            9,
            installed_ids,
        ),
        discover_entry(
            "2878",
            "Tamriel Trade Centre",
            "cyxui",
            "TradeSkill Mods",
            "4.3.2",
            "3,480,771",
            "91,006",
            "598",
            "03/18/26 05:33 AM",
            "07/12/19 05:15 PM",
            "9cba018e617c",
            "Season Zero (11.3.0)",
            "Market price lookups and trade data helpers for guild trader workflows.",
            10,
            installed_ids,
        ),
    ];

    if let Some(first) = entries.first_mut() {
        first.installed = true;
    }
    if let Some(lui) = entries
        .iter_mut()
        .find(|entry| entry.esoui_id.as_str() == "818")
    {
        lui.installed = true;
    }
    if let Some(esotw) = entries
        .iter_mut()
        .find(|entry| entry.esoui_id.as_str() == "2194")
    {
        esotw.installed = true;
    }

    entries
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

fn discover_entry_from_esoui_detail(
    detail: esoui::EsouiAddonDetail,
    installed: bool,
    rank: i32,
) -> DiscoverEntry {
    DiscoverEntry {
        esoui_id: detail.id.to_string().into(),
        title: detail.title.into(),
        author: detail.author.into(),
        category: "".into(),
        version: detail.version.into(),
        downloads: detail.total_downloads.into(),
        monthly_downloads: detail.monthly_downloads.into(),
        favorites: detail.favorites.into(),
        updated: detail.updated.into(),
        created: detail.created.into(),
        md5: detail.md5.into(),
        compatibility: detail.compatibility.into(),
        description: detail.description.into(),
        installed,
        rank,
    }
}

fn discover_entry_from_search_result(
    result: esoui::EsouiSearchResult,
    installed_ids: &BTreeSet<String>,
    rank: i32,
) -> DiscoverEntry {
    let esoui_id = result.id.to_string();
    DiscoverEntry {
        esoui_id: esoui_id.clone().into(),
        title: result.title.into(),
        author: result.author.into(),
        category: result.category.into(),
        version: "".into(),
        downloads: result.downloads.into(),
        monthly_downloads: "".into(),
        favorites: "".into(),
        updated: result.updated.into(),
        created: "".into(),
        md5: "".into(),
        compatibility: "".into(),
        description: "Select this addon to load its ESOUI description.".into(),
        installed: installed_ids.contains(&esoui_id),
        rank,
    }
}

fn discover_entries_from_search_results(
    results: Vec<esoui::EsouiSearchResult>,
    installed_ids: &BTreeSet<String>,
) -> Vec<DiscoverEntry> {
    discover_entries_from_search_results_with_offset(results, installed_ids, 0)
}

fn discover_entries_from_search_results_with_offset(
    results: Vec<esoui::EsouiSearchResult>,
    installed_ids: &BTreeSet<String>,
    rank_offset: usize,
) -> Vec<DiscoverEntry> {
    results
        .into_iter()
        .enumerate()
        .map(|(index, result)| {
            discover_entry_from_search_result(
                result,
                installed_ids,
                (rank_offset + index + 1) as i32,
            )
        })
        .collect()
}

fn default_discover_category_id(categories: &[esoui::EsouiCategory]) -> Option<u32> {
    categories
        .iter()
        .find(|category| category.name.eq_ignore_ascii_case("Combat"))
        .or_else(|| categories.iter().find(|category| category.depth == 0))
        .or_else(|| categories.first())
        .map(|category| category.id)
}

fn merge_discover_detail(
    mut entry: DiscoverEntry,
    detail: esoui::EsouiAddonDetail,
) -> DiscoverEntry {
    entry.esoui_id = detail.id.to_string().into();
    entry.title = detail.title.into();
    entry.author = detail.author.into();
    entry.version = detail.version.into();
    entry.downloads = detail.total_downloads.into();
    entry.monthly_downloads = detail.monthly_downloads.into();
    entry.favorites = detail.favorites.into();
    entry.updated = detail.updated.into();
    entry.created = detail.created.into();
    entry.md5 = detail.md5.into();
    entry.compatibility = detail.compatibility.into();
    entry.description = detail.description.into();
    entry
}

fn discover_entry_needs_detail(entry: &DiscoverEntry) -> bool {
    entry.version.is_empty()
        || entry.description.as_str() == "Select this addon to load its ESOUI description."
}

const DISCOVER_SCREENSHOT_LIMIT: usize = 4;

fn clear_discover_screenshots(ui: &KalpaWindow) {
    if ui.get_discover_screenshot_index() == 0 && ui.get_discover_screenshots().row_count() == 0 {
        return;
    }

    ui.set_discover_screenshot_index(0);
    ui.set_discover_screenshots(
        Rc::new(VecModel::from(Vec::<DiscoverScreenshotEntry>::new())).into(),
    );
}

fn discover_screenshot_entries(
    paths: &[PathBuf],
    selected_index: usize,
) -> Vec<DiscoverScreenshotEntry> {
    paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            Image::load_from_path(path)
                .ok()
                .map(|image| DiscoverScreenshotEntry {
                    image,
                    selected: index == selected_index,
                })
        })
        .collect()
}

fn apply_discover_screenshot_selection(ui: &KalpaWindow, selected_index: usize) {
    let screenshot_count = ui.get_discover_screenshots().row_count();
    if screenshot_count == 0 {
        ui.set_discover_screenshot_index(0);
        return;
    }

    let selected_index = selected_index.min(screenshot_count.saturating_sub(1));
    let rows = (0..screenshot_count)
        .filter_map(|index| ui.get_discover_screenshots().row_data(index))
        .enumerate()
        .map(|(index, mut shot)| {
            shot.selected = index == selected_index;
            shot
        })
        .collect::<Vec<_>>();
    ui.set_discover_screenshot_index(selected_index as i32);
    ui.set_discover_screenshots(Rc::new(VecModel::from(rows)).into());
}

fn discover_screenshot_cache_dir() -> PathBuf {
    std::env::temp_dir().join("kalpa-slint-discover-screenshots")
}

fn discover_screenshot_cache_path(addon_id: &str, index: usize, url: &str) -> PathBuf {
    let extension = url
        .split(['?', '#'])
        .next()
        .and_then(|value| value.rsplit('.').next())
        .filter(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp"
            )
        })
        .unwrap_or("jpg");
    discover_screenshot_cache_dir().join(format!("{addon_id}-{index}.{extension}"))
}

fn download_discover_screenshot(
    addon_id: &str,
    index: usize,
    url: &str,
) -> Result<PathBuf, String> {
    if !url.starts_with("https://cdn.esoui.com/")
        && !url.starts_with("https://www.esoui.com/")
        && !url.starts_with("https://cdn-eso.mmoui.com/")
    {
        return Err("Ignoring screenshot from an untrusted host.".to_string());
    }

    let path = discover_screenshot_cache_path(addon_id, index, url);
    if path.is_file() {
        return Ok(path);
    }

    fs::create_dir_all(discover_screenshot_cache_dir())
        .map_err(|error| format!("Could not create screenshot cache: {error}"))?;
    let bytes = reqwest::blocking::get(url)
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Could not fetch screenshot: {error}"))?
        .bytes()
        .map_err(|error| format!("Could not read screenshot: {error}"))?;
    fs::write(&path, bytes).map_err(|error| format!("Could not cache screenshot: {error}"))?;
    Ok(path)
}

fn request_discover_screenshots(
    ui: &KalpaWindow,
    addon_id: String,
    urls: Vec<String>,
    request_counter: Arc<AtomicU64>,
) {
    clear_discover_screenshots(ui);
    if urls.is_empty() {
        return;
    }

    let request_id = request_counter.fetch_add(1, Ordering::SeqCst) + 1;
    let ui_weak = ui.as_weak();
    std::thread::spawn(move || {
        let paths = urls
            .into_iter()
            .take(DISCOVER_SCREENSHOT_LIMIT)
            .enumerate()
            .filter_map(|(index, url)| download_discover_screenshot(&addon_id, index, &url).ok())
            .collect::<Vec<_>>();

        let _ = slint::invoke_from_event_loop(move || {
            if request_counter.load(Ordering::SeqCst) != request_id {
                return;
            }
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let entries = discover_screenshot_entries(&paths, 0);
            ui.set_discover_screenshot_index(0);
            ui.set_discover_screenshots(Rc::new(VecModel::from(entries)).into());
        });
    });
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

fn addon_meta(version: &str, author: &str) -> String {
    match (version.trim(), author.trim()) {
        ("", "") => String::new(),
        (version, "") => version.to_string(),
        ("", author) => author.to_string(),
        (version, author) => format!("{version}  \u{00b7} {author}"),
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
        meta: addon_meta(version, author).into(),
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
    let refresh_models = models.clone();
    ui.on_refresh_requested(move || {
        let Some(ui) = refresh_ui.upgrade() else {
            return;
        };

        let active_theme_id = apply_initial_theme(&ui);
        ui.set_active_theme_id(active_theme_id.into());

        match reload_real_addon_models(&ui, &refresh_models) {
            Ok(()) => ui.set_status_error_message("".into()),
            Err(error) => {
                apply_saved_variables_model(&ui, &refresh_models.all.borrow());
                ui.set_status_error_message(error.into());
            }
        }

        // Acknowledge the refresh even when the reloaded data is unchanged:
        // spin the header refresh icon for a short beat so the click clearly
        // does something instead of appearing inert.
        if !ui.get_checking_updates() {
            ui.set_checking_updates(true);
            let spinner_ui = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(650), move || {
                if let Some(ui) = spinner_ui.upgrade() {
                    ui.set_checking_updates(false);
                }
            });
        }
    });

    let pack_hub_browse_state = Arc::new(Mutex::new(PackHubBrowseState::default()));
    let pack_hub_browse_counter = Arc::new(AtomicU64::new(0));
    let pack_hub_create_state = Rc::new(RefCell::new(PackHubCreateState::default()));

    let pack_hub_ui = ui.as_weak();
    let pack_hub_state = pack_hub_browse_state.clone();
    let pack_hub_counter = pack_hub_browse_counter.clone();
    let pack_hub_create_models = models.clone();
    let pack_hub_create_state_open = pack_hub_create_state.clone();
    ui.on_open_pack_hub(move || {
        let Some(ui) = pack_hub_ui.upgrade() else {
            return;
        };
        {
            let mut state = pack_hub_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.reset_page();
            apply_pack_hub_browse_state(&ui, &state);
        }
        apply_pack_hub_create_state(
            &ui,
            &pack_hub_create_models,
            &pack_hub_create_state_open.borrow(),
        );
        apply_installed_pack_refs(&ui, read_installed_pack_refs());
        request_pack_hub_browse_page(&ui, pack_hub_state.clone(), pack_hub_counter.clone(), false);
    });

    let pack_query_ui = ui.as_weak();
    let pack_query_state = pack_hub_browse_state.clone();
    let pack_query_counter = pack_hub_browse_counter.clone();
    ui.on_pack_hub_browse_query_edited(move |query| {
        let Some(ui) = pack_query_ui.upgrade() else {
            return;
        };
        {
            let mut state = pack_query_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.query = query.to_string();
            state.reset_page();
            apply_pack_hub_browse_state(&ui, &state);
        }
        request_pack_hub_browse_page(
            &ui,
            pack_query_state.clone(),
            pack_query_counter.clone(),
            false,
        );
    });

    let pack_type_ui = ui.as_weak();
    let pack_type_state = pack_hub_browse_state.clone();
    let pack_type_counter = pack_hub_browse_counter.clone();
    ui.on_pack_hub_browse_type_next(move || {
        let Some(ui) = pack_type_ui.upgrade() else {
            return;
        };
        {
            let mut state = pack_type_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.next_type_filter();
            apply_pack_hub_browse_state(&ui, &state);
        }
        request_pack_hub_browse_page(
            &ui,
            pack_type_state.clone(),
            pack_type_counter.clone(),
            false,
        );
    });

    let pack_sort_ui = ui.as_weak();
    let pack_sort_state = pack_hub_browse_state.clone();
    let pack_sort_counter = pack_hub_browse_counter.clone();
    ui.on_pack_hub_browse_sort_next(move || {
        let Some(ui) = pack_sort_ui.upgrade() else {
            return;
        };
        {
            let mut state = pack_sort_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.next_sort();
            apply_pack_hub_browse_state(&ui, &state);
        }
        request_pack_hub_browse_page(
            &ui,
            pack_sort_state.clone(),
            pack_sort_counter.clone(),
            false,
        );
    });

    let pack_more_ui = ui.as_weak();
    let pack_more_state = pack_hub_browse_state.clone();
    let pack_more_counter = pack_hub_browse_counter.clone();
    ui.on_pack_hub_browse_load_more(move || {
        let Some(ui) = pack_more_ui.upgrade() else {
            return;
        };
        if ui.get_pack_hub_browse_loading() || !ui.get_pack_hub_browse_has_more() {
            return;
        }
        {
            let next = pack_more_state
                .lock()
                .map(|state| state.next_page_snapshot())
                .unwrap_or_default();
            let mut state = pack_more_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *state = next;
            apply_pack_hub_browse_state(&ui, &state);
        }
        request_pack_hub_browse_page(
            &ui,
            pack_more_state.clone(),
            pack_more_counter.clone(),
            true,
        );
    });

    let pack_retry_ui = ui.as_weak();
    let pack_retry_state = pack_hub_browse_state.clone();
    let pack_retry_counter = pack_hub_browse_counter.clone();
    ui.on_pack_hub_browse_retry(move || {
        let Some(ui) = pack_retry_ui.upgrade() else {
            return;
        };
        request_pack_hub_browse_page(
            &ui,
            pack_retry_state.clone(),
            pack_retry_counter.clone(),
            false,
        );
    });

    let import_file_settings = Arc::new(Mutex::new(HashMap::<String, NativeAddonSettings>::new()));

    let import_code_ui = ui.as_weak();
    ui.on_pack_hub_import_share_code_edited(move |code| {
        let Some(ui) = import_code_ui.upgrade() else {
            return;
        };
        let normalized = normalize_share_code(code.as_str());
        if normalized != code.as_str() {
            ui.set_pack_hub_import_share_code(normalized.into());
        }
        ui.set_pack_hub_import_message("".into());
    });

    let import_file_path_ui = ui.as_weak();
    ui.on_pack_hub_import_file_path_edited(move |path| {
        let Some(ui) = import_file_path_ui.upgrade() else {
            return;
        };
        ui.set_pack_hub_import_file_path(path.to_string().into());
        ui.set_pack_hub_import_message("".into());
    });

    let import_clear_ui = ui.as_weak();
    let import_clear_settings = import_file_settings.clone();
    ui.on_pack_hub_import_clear(move || {
        let Some(ui) = import_clear_ui.upgrade() else {
            return;
        };
        import_clear_settings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        ui.set_pack_hub_import_loading(false);
        ui.set_pack_hub_import_share_code("".into());
        ui.set_pack_hub_import_file_path("".into());
        clear_pack_hub_import_model(&ui);
    });

    let import_resolve_ui = ui.as_weak();
    let import_resolve_models = models.clone();
    let import_resolve_settings = import_file_settings.clone();
    ui.on_pack_hub_import_resolve_share_code(move || {
        let Some(ui) = import_resolve_ui.upgrade() else {
            return;
        };
        if ui.get_pack_hub_import_loading() {
            return;
        }

        let code = match validate_share_code(ui.get_pack_hub_import_share_code().as_str()) {
            Ok(code) => code,
            Err(error) => {
                ui.set_pack_hub_import_message(error.into());
                return;
            }
        };
        ui.set_pack_hub_import_share_code(code.as_str().into());
        ui.set_pack_hub_import_loading(true);
        ui.set_pack_hub_import_message("Resolving share code...".into());
        import_resolve_settings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        apply_pack_hub_import_model(&ui, empty_pack_hub_entry(), Vec::new(), false);

        let installed_ids = installed_discover_ids(&import_resolve_models.all.borrow());
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = fetch_shared_pack_blocking(&code, &installed_ids);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                ui.set_pack_hub_import_loading(false);
                match result {
                    Ok(detail) => {
                        apply_pack_hub_import_model(&ui, detail.entry, detail.addons, false);
                        ui.set_pack_hub_import_message("".into());
                    }
                    Err(error) => {
                        apply_pack_hub_import_model(&ui, empty_pack_hub_entry(), Vec::new(), false);
                        ui.set_pack_hub_import_message(error.into());
                    }
                }
            });
        });
    });

    let import_file_ui = ui.as_weak();
    let import_file_models = models.clone();
    let import_file_settings_state = import_file_settings.clone();
    ui.on_pack_hub_import_file(move || {
        let Some(ui) = import_file_ui.upgrade() else {
            return;
        };
        if ui.get_pack_hub_import_loading() {
            return;
        }

        let path = ui.get_pack_hub_import_file_path().to_string();
        if path.trim().is_empty() {
            ui.set_pack_hub_import_message("Enter a .esopack file path.".into());
            return;
        }

        ui.set_pack_hub_import_loading(true);
        ui.set_pack_hub_import_message("Reading .esopack file...".into());
        ui.set_pack_hub_import_share_code("".into());
        apply_pack_hub_import_model(&ui, empty_pack_hub_entry(), Vec::new(), false);

        let installed_ids = installed_discover_ids(&import_file_models.all.borrow());
        let ui_weak = ui.as_weak();
        let settings_state = import_file_settings_state.clone();
        std::thread::spawn(move || {
            let result = import_esopack_file_blocking(Path::new(&path), &installed_ids);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                ui.set_pack_hub_import_loading(false);
                match result {
                    Ok(imported) => {
                        let has_settings = !imported.settings.is_empty();
                        *settings_state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = imported.settings;
                        apply_pack_hub_import_model(
                            &ui,
                            imported.detail.entry,
                            imported.detail.addons,
                            has_settings,
                        );
                        ui.set_pack_hub_import_message("".into());
                    }
                    Err(error) => {
                        settings_state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .clear();
                        apply_pack_hub_import_model(&ui, empty_pack_hub_entry(), Vec::new(), false);
                        ui.set_pack_hub_import_message(error.into());
                    }
                }
            });
        });
    });

    let import_select_ui = ui.as_weak();
    ui.on_pack_hub_import_addon_selection_toggle(move |_| {
        let Some(ui) = import_select_ui.upgrade() else {
            return;
        };
        ui.set_pack_hub_import_message("Shared pack import installs required addons only.".into());
    });

    let import_install_ui = ui.as_weak();
    let import_install_settings = import_file_settings;
    ui.on_pack_hub_import_install(move || {
        let Some(ui) = import_install_ui.upgrade() else {
            return;
        };
        if ui.get_pack_hub_import_loading() {
            return;
        }

        let rows = pack_hub_import_addons(&ui);
        let pending = rows
            .iter()
            .filter(|row| !row.installed && row.selected)
            .count();
        let settings = import_install_settings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if pending == 0 && settings.is_empty() {
            ui.set_status_error_message(
                "All required addons in this shared pack are already installed.".into(),
            );
            return;
        }

        let addons_dir = match configured_addons_path() {
            Some(path) => path,
            None => {
                ui.set_status_error_message(
                    "Configure the ESO AddOns folder before installing from Pack Hub.".into(),
                );
                return;
            }
        };

        let pack = ui.get_pack_hub_import_pack();
        let eso_running = pending > 0 && addon_write_eso_running_warning_active(&ui);
        ui.set_pack_hub_import_loading(true);
        ui.set_pack_hub_import_install_label(
            if pending == 0 {
                "Applying Settings".to_string()
            } else {
                format!("Installing {pending}...")
            }
            .into(),
        );
        let status = if pending == 0 {
            "Applying shared pack settings...".to_string()
        } else {
            format!("Installing {pending} shared pack addon(s)...")
        };
        ui.set_status_error_message(addon_write_status_message(status, eso_running).into());

        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = if pending == 0 {
                NativePackInstallResult {
                    rows,
                    installed: 0,
                    failed: 0,
                    folders: 0,
                    errors: Vec::new(),
                }
            } else {
                install_pack_hub_addons_blocking(&addons_dir, rows)
            };
            let settings_result = (!settings.is_empty())
                .then(|| apply_imported_pack_settings_blocking(&addons_dir, settings));
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                let summary = pack_hub_install_summary(&result);
                let installed = result.installed;
                ui.set_pack_hub_import_loading(false);
                let has_settings = ui.get_pack_hub_import_has_settings();
                apply_pack_hub_import_model(&ui, pack, result.rows, has_settings);
                if installed > 0 {
                    ui.invoke_refresh_requested();
                }
                let summary = settings_result
                    .as_ref()
                    .map(|settings| pack_hub_import_combined_summary(&summary, settings, pending))
                    .unwrap_or(summary);
                ui.set_status_error_message(
                    addon_write_status_message(summary, eso_running).into(),
                );
            });
        });
    });

    let pack_detail_ui = ui.as_weak();
    let pack_detail_models = models.clone();
    ui.on_pack_hub_open_detail(move |index| {
        let Some(ui) = pack_detail_ui.upgrade() else {
            return;
        };

        let index = index.max(0) as usize;
        ui.set_pack_hub_selected_index(index as i32);
        ui.set_pack_hub_detail_loading(true);
        ui.set_pack_hub_detail_message("".into());
        ui.set_pack_hub_detail_sharing(false);
        ui.set_pack_hub_detail_share_status("".into());
        apply_pack_hub_detail_model(&ui, Vec::new());

        let Some(pack) = ui.get_pack_hub_packs().row_data(index) else {
            ui.set_pack_hub_detail_loading(false);
            ui.set_pack_hub_detail_message("Select a pack to view its addons.".into());
            return;
        };

        let installed_ids = installed_discover_ids(&pack_detail_models.all.borrow());
        request_pack_hub_detail(&ui, index, pack.id.to_string(), installed_ids);
    });

    let installed_pack_open_ui = ui.as_weak();
    let installed_pack_open_models = models.clone();
    ui.on_pack_hub_open_installed_pack(move |index| {
        let Some(ui) = installed_pack_open_ui.upgrade() else {
            return;
        };

        let index = index.max(0) as usize;
        let Some(entry) = pack_hub_installed_entries_from_model(&ui)
            .get(index)
            .cloned()
        else {
            ui.set_status_error_message("Select an installed pack to open.".into());
            return;
        };
        if entry.pack_id.trim().is_empty() {
            ui.set_status_error_message("Installed pack has no Pack Hub id.".into());
            return;
        }

        let pack_index = ensure_pack_hub_entry_for_installed(&ui, &entry);
        ui.set_pack_hub_selected_index(pack_index as i32);
        ui.set_pack_hub_view(3);
        ui.set_pack_hub_detail_loading(true);
        ui.set_pack_hub_detail_message("".into());
        ui.set_pack_hub_detail_sharing(false);
        ui.set_pack_hub_detail_share_status("".into());
        apply_pack_hub_detail_model(&ui, Vec::new());

        let installed_ids = installed_discover_ids(&installed_pack_open_models.all.borrow());
        request_pack_hub_detail(&ui, pack_index, entry.pack_id.to_string(), installed_ids);
    });

    let installed_pack_remove_ui = ui.as_weak();
    ui.on_pack_hub_remove_installed_pack(move |index| {
        let Some(ui) = installed_pack_remove_ui.upgrade() else {
            return;
        };

        let index = index.max(0) as usize;
        let Some(entry) = pack_hub_installed_entries_from_model(&ui)
            .get(index)
            .cloned()
        else {
            return;
        };
        match remove_installed_pack_ref(entry.pack_id.as_str()) {
            Ok(refs) => {
                apply_installed_pack_refs(&ui, refs);
                ui.set_status_error_message("Removed pack from local library.".into());
            }
            Err(error) => {
                ui.set_status_error_message(
                    format!("Could not update installed pack library: {error}").into(),
                );
            }
        }
    });

    let pack_hub_full_ui = ui.as_weak();
    ui.on_pack_hub_open_full_hub(move || {
        let Some(ui) = pack_hub_full_ui.upgrade() else {
            return;
        };

        match return_to_webview_shell(false, false, true, None) {
            Ok(()) => {
                ui.set_status_error_message("Opening full Pack Hub...".into());
                let _ = slint::quit_event_loop();
            }
            Err(error) => {
                ui.set_status_error_message(
                    format!("Failed to open full Pack Hub: {error}").into(),
                );
            }
        }
    });

    let pack_install_ui = ui.as_weak();
    ui.on_pack_hub_install_detail(move || {
        let Some(ui) = pack_install_ui.upgrade() else {
            return;
        };

        let rows = pack_hub_detail_addons(&ui);
        let pending = rows
            .iter()
            .filter(|row| !row.installed && row.selected)
            .count();
        if pending == 0 {
            ui.set_status_error_message("Select at least one missing addon to install.".into());
            return;
        }

        let addons_dir = match configured_addons_path() {
            Some(path) => path,
            None => {
                ui.set_status_error_message(
                    "Configure the ESO AddOns folder before installing from Pack Hub.".into(),
                );
                return;
            }
        };

        let eso_running = addon_write_eso_running_warning_active(&ui);
        ui.set_pack_hub_install_label(format!("Installing {pending}...").into());
        ui.set_status_error_message(
            addon_write_status_message(
                format!("Installing {pending} Pack Hub addon(s)..."),
                eso_running,
            )
            .into(),
        );

        let installed_pack_entry = selected_pack_hub_entry(&ui);
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = install_pack_hub_addons_blocking(&addons_dir, rows);
            if result.installed > 0 {
                if let Some(entry) = installed_pack_entry.as_ref() {
                    track_pack_install_blocking(entry.id.as_str());
                }
            }
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                let mut summary = pack_hub_install_summary(&result);
                let installed = result.installed;
                apply_pack_hub_detail_model(&ui, result.rows);
                if installed > 0 {
                    ui.invoke_refresh_requested();
                    if let Some(entry) = installed_pack_entry.as_ref() {
                        match upsert_installed_pack_ref(entry) {
                            Ok(refs) => apply_installed_pack_refs(&ui, refs),
                            Err(error) => {
                                summary =
                                    format!("{summary} Could not update pack library: {error}");
                            }
                        }
                    }
                }

                ui.set_status_error_message(
                    addon_write_status_message(summary, eso_running).into(),
                );
            });
        });
    });

    let pack_manage_ui = ui.as_weak();
    ui.on_pack_hub_manage_detail(move || {
        let Some(ui) = pack_manage_ui.upgrade() else {
            return;
        };

        let pack_id = selected_pack_hub_id(&ui);
        match return_to_webview_shell(false, false, true, pack_id.as_deref()) {
            Ok(()) => {
                ui.set_status_error_message("Opening full Pack Hub...".into());
                let _ = slint::quit_event_loop();
            }
            Err(error) => {
                ui.set_pack_hub_detail_message(
                    format!(
                        "Full Pack Hub is needed for edit and delete actions. Failed to open WebView: {error}"
                    )
                    .into(),
                );
                ui.set_status_error_message(format!("Failed to open full Pack Hub: {error}").into());
            }
        }
    });

    let pack_share_ui = ui.as_weak();
    ui.on_pack_hub_share_detail(move || {
        let Some(ui) = pack_share_ui.upgrade() else {
            return;
        };

        let Some(entry) = selected_pack_hub_entry(&ui) else {
            ui.set_pack_hub_detail_share_status("Select a pack first.".into());
            return;
        };
        let addons = pack_hub_detail_addons(&ui);
        if addons.is_empty() {
            ui.set_pack_hub_detail_share_status("Load pack addons before sharing.".into());
            return;
        }

        ui.set_pack_hub_detail_sharing(true);
        ui.set_pack_hub_detail_share_status("Preparing .esopack...".into());
        ui.set_status_error_message("Exporting Pack Hub share file...".into());

        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = export_pack_hub_detail_file(&entry, &addons, None).map(|path| {
                let clipboard = write_clipboard_text(path.display().to_string());
                (path, clipboard)
            });
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                ui.set_pack_hub_detail_sharing(false);
                match result {
                    Ok((path, Ok(()))) => {
                        ui.set_pack_hub_detail_share_status("Exported and copied path.".into());
                        ui.set_status_error_message(
                            format!(
                                "Pack exported to {} and copied to clipboard.",
                                path.display()
                            )
                            .into(),
                        );
                    }
                    Ok((path, Err(error))) => {
                        ui.set_pack_hub_detail_share_status("Exported. Clipboard failed.".into());
                        ui.set_status_error_message(
                            format!(
                                "Pack exported to {}. Could not copy path: {error}",
                                path.display()
                            )
                            .into(),
                        );
                    }
                    Err(error) => {
                        ui.set_pack_hub_detail_share_status("Share export failed.".into());
                        ui.set_status_error_message(
                            format!("Pack Hub share export failed: {error}").into(),
                        );
                    }
                }
            });
        });
    });

    let create_filter_ui = ui.as_weak();
    let create_filter_models = models.clone();
    let create_filter_state = pack_hub_create_state.clone();
    ui.on_pack_hub_create_filter_edited(move |filter| {
        let Some(ui) = create_filter_ui.upgrade() else {
            return;
        };
        let mut state = create_filter_state.borrow_mut();
        state.filter = filter.to_string();
        apply_pack_hub_create_state(&ui, &create_filter_models, &state);
    });

    let create_toggle_ui = ui.as_weak();
    let create_toggle_models = models.clone();
    let create_toggle_state = pack_hub_create_state.clone();
    ui.on_pack_hub_create_addon_toggle(move |index| {
        let Some(ui) = create_toggle_ui.upgrade() else {
            return;
        };
        let Some(row) = ui
            .get_pack_hub_create_addons()
            .row_data(index.max(0) as usize)
        else {
            return;
        };
        let mut state = create_toggle_state.borrow_mut();
        toggle_pack_hub_create_row(&mut state, row);
        apply_pack_hub_create_state(&ui, &create_toggle_models, &state);
    });

    let create_remove_ui = ui.as_weak();
    let create_remove_models = models.clone();
    let create_remove_state = pack_hub_create_state.clone();
    ui.on_pack_hub_create_selected_remove(move |index| {
        let Some(ui) = create_remove_ui.upgrade() else {
            return;
        };
        let Some(row) = ui
            .get_pack_hub_create_selected_addons()
            .row_data(index.max(0) as usize)
        else {
            return;
        };
        let mut state = create_remove_state.borrow_mut();
        remove_pack_hub_create_selected(&mut state, row);
        apply_pack_hub_create_state(&ui, &create_remove_models, &state);
    });

    let create_required_ui = ui.as_weak();
    let create_required_models = models.clone();
    let create_required_state = pack_hub_create_state.clone();
    ui.on_pack_hub_create_selected_required_toggle(move |index| {
        let Some(ui) = create_required_ui.upgrade() else {
            return;
        };
        let Some(row) = ui
            .get_pack_hub_create_selected_addons()
            .row_data(index.max(0) as usize)
        else {
            return;
        };
        let mut state = create_required_state.borrow_mut();
        toggle_pack_hub_create_required(&mut state, row);
        apply_pack_hub_create_state(&ui, &create_required_models, &state);
    });

    let create_save_ui = ui.as_weak();
    let create_save_state = pack_hub_create_state;
    ui.on_pack_hub_create_save_to_file(move || {
        let Some(ui) = create_save_ui.upgrade() else {
            return;
        };
        let title = ui.get_pack_hub_create_title().to_string();
        let description = ui.get_pack_hub_create_description().to_string();
        let pack_type = ui.get_pack_hub_create_pack_type();
        let state = create_save_state.borrow();
        match export_pack_hub_create_file(
            title.as_str(),
            description.as_str(),
            pack_type,
            &state.selected,
            None,
        ) {
            Ok(path) => {
                ui.set_status_error_message(format!("Pack saved to {}.", path.display()).into());
            }
            Err(error) => {
                ui.set_status_error_message(format!("Pack export failed: {error}").into())
            }
        }
    });

    let detail_select_ui = ui.as_weak();
    ui.on_pack_hub_detail_addon_selection_toggle(move |index| {
        let Some(ui) = detail_select_ui.upgrade() else {
            return;
        };
        let model = ui.get_pack_hub_detail_addons();
        let index = index.max(0) as usize;
        let Some(mut row) = model.row_data(index) else {
            return;
        };
        if row.required || row.installed {
            return;
        }
        row.selected = !row.selected;
        model.set_row_data(index, row);
        apply_pack_hub_detail_model(&ui, pack_hub_detail_addons(&ui));
    });

    let svm_copy_state = Rc::new(RefCell::new(SvmCopyState::default()));
    let svm_editor_state = Rc::new(RefCell::new(SvmEditorState::default()));

    let svm_open_ui = ui.as_weak();
    let svm_open_models = models.clone();
    let svm_open_copy_state = svm_copy_state.clone();
    let svm_open_editor_state = svm_editor_state.clone();
    ui.on_open_svm(move || {
        let Some(ui) = svm_open_ui.upgrade() else {
            return;
        };
        refresh_saved_variables_overlay_model(
            &ui,
            &svm_open_models,
            &svm_open_copy_state,
            &svm_open_editor_state,
        );
    });

    let svm_refresh_ui = ui.as_weak();
    let svm_refresh_models = models.clone();
    let svm_refresh_copy_state = svm_copy_state.clone();
    let svm_refresh_editor_state = svm_editor_state.clone();
    ui.on_svm_refresh(move || {
        let Some(ui) = svm_refresh_ui.upgrade() else {
            return;
        };
        refresh_saved_variables_overlay_model(
            &ui,
            &svm_refresh_models,
            &svm_refresh_copy_state,
            &svm_refresh_editor_state,
        );
    });

    let svm_clean_ui = ui.as_weak();
    let svm_clean_models = models.clone();
    let svm_clean_copy_state = svm_copy_state.clone();
    let svm_clean_editor_state = svm_editor_state.clone();
    ui.on_svm_clean_orphans(move || {
        let Some(ui) = svm_clean_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before cleaning SavedVariables.".into(),
            );
            return;
        };

        let orphaned_model = ui.get_svm_orphaned_files();
        let orphaned = (0..orphaned_model.row_count())
            .filter_map(|index| orphaned_model.row_data(index))
            .filter(|entry| entry.selected)
            .collect::<Vec<_>>();
        if orphaned.is_empty() {
            ui.set_status_error_message("Select orphaned SavedVariables files to clean.".into());
            return;
        }

        match clean_saved_variable_orphans(&addons_root, &orphaned) {
            Ok(deleted) => {
                refresh_saved_variables_overlay_model(
                    &ui,
                    &svm_clean_models,
                    &svm_clean_copy_state,
                    &svm_clean_editor_state,
                );
                ui.set_status_error_message(
                    format!(
                        "Deleted {deleted} orphaned SavedVariables file{} and created an automatic backup.",
                        if deleted == 1 { "" } else { "s" }
                    )
                    .into(),
                );
            }
            Err(error) => {
                ui.set_status_error_message(format!("SavedVariables cleanup failed: {error}").into())
            }
        }
    });

    let svm_orphan_toggle_ui = ui.as_weak();
    ui.on_svm_orphan_selection_toggled(move |index| {
        let Some(ui) = svm_orphan_toggle_ui.upgrade() else {
            return;
        };
        let model = ui.get_svm_orphaned_files();
        let index = index.max(0) as usize;
        let Some(mut row) = model.row_data(index) else {
            return;
        };
        row.selected = !row.selected;
        model.set_row_data(index, row);
        update_svm_cleanup_selected_count(&ui);
    });

    let svm_orphan_select_all_ui = ui.as_weak();
    ui.on_svm_orphan_select_all(move |selected| {
        let Some(ui) = svm_orphan_select_all_ui.upgrade() else {
            return;
        };
        let model = ui.get_svm_orphaned_files();
        for index in 0..model.row_count() {
            if let Some(mut row) = model.row_data(index) {
                row.selected = selected;
                model.set_row_data(index, row);
            }
        }
        update_svm_cleanup_selected_count(&ui);
    });

    let svm_copy_file_ui = ui.as_weak();
    let svm_copy_file_state = svm_copy_state.clone();
    ui.on_svm_copy_select_file(move || {
        let Some(ui) = svm_copy_file_ui.upgrade() else {
            return;
        };
        svm_copy_file_state.borrow_mut().select_next_file();
        apply_svm_copy_state(&ui, &svm_copy_file_state.borrow());
    });

    let svm_copy_source_ui = ui.as_weak();
    let svm_copy_source_state = svm_copy_state.clone();
    ui.on_svm_copy_select_source(move || {
        let Some(ui) = svm_copy_source_ui.upgrade() else {
            return;
        };
        svm_copy_source_state.borrow_mut().select_next_source();
        apply_svm_copy_state(&ui, &svm_copy_source_state.borrow());
    });

    let svm_copy_dest_ui = ui.as_weak();
    let svm_copy_dest_state = svm_copy_state.clone();
    ui.on_svm_copy_select_dest(move || {
        let Some(ui) = svm_copy_dest_ui.upgrade() else {
            return;
        };
        svm_copy_dest_state.borrow_mut().select_next_dest();
        apply_svm_copy_state(&ui, &svm_copy_dest_state.borrow());
    });

    let svm_copy_action_ui = ui.as_weak();
    let svm_copy_action_models = models.clone();
    let svm_copy_action_state = svm_copy_state;
    let svm_copy_action_editor_state = svm_editor_state.clone();
    ui.on_svm_copy_profile(move || {
        let Some(ui) = svm_copy_action_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before copying SavedVariables profiles.".into(),
            );
            return;
        };
        let Some(selection) = svm_copy_action_state.borrow().selection() else {
            ui.set_status_error_message("Choose a source and destination profile first.".into());
            return;
        };

        match copy_svm_profile_selection(&addons_root, &selection) {
            Ok(()) => {
                refresh_saved_variables_overlay_model(
                    &ui,
                    &svm_copy_action_models,
                    &svm_copy_action_state,
                    &svm_copy_action_editor_state,
                );
                {
                    let mut state = svm_copy_action_state.borrow_mut();
                    state.status = format!(
                        "Copied {} to {} in {}.",
                        selection.source_key, selection.dest_key, selection.addon_name
                    );
                }
                apply_svm_copy_state(&ui, &svm_copy_action_state.borrow());
                ui.set_status_error_message(
                    format!(
                        "Copied \"{}\" to \"{}\" in {}.",
                        selection.source_key, selection.dest_key, selection.addon_name
                    )
                    .into(),
                );
            }
            Err(error) => {
                {
                    let mut state = svm_copy_action_state.borrow_mut();
                    state.status = format!("Copy failed: {error}");
                }
                apply_svm_copy_state(&ui, &svm_copy_action_state.borrow());
                ui.set_status_error_message(format!("SavedVariables copy failed: {error}").into());
            }
        }
    });

    let svm_editor_select_ui = ui.as_weak();
    let svm_editor_select_state = svm_editor_state.clone();
    ui.on_svm_editor_select_file(move || {
        let Some(ui) = svm_editor_select_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before editing SavedVariables.".into(),
            );
            return;
        };
        {
            let mut state = svm_editor_select_state.borrow_mut();
            state.select_next_file();
            if let Err(error) = load_svm_editor_selected_file(&addons_root, &mut state) {
                state.tree = None;
                state.stamp = None;
                state.selected_path.clear();
                state.dirty = false;
                state.message = error;
            }
        }
        apply_svm_editor_state(&ui, &svm_editor_select_state.borrow());
    });

    let svm_editor_toggle_ui = ui.as_weak();
    let svm_editor_toggle_state = svm_editor_state.clone();
    ui.on_svm_editor_toggle_setting(move |index| {
        let Some(ui) = svm_editor_toggle_ui.upgrade() else {
            return;
        };
        let result = {
            let mut state = svm_editor_toggle_state.borrow_mut();
            toggle_svm_editor_setting(&mut state, index as usize)
        };
        match result {
            Ok(()) => apply_svm_editor_state(&ui, &svm_editor_toggle_state.borrow()),
            Err(error) => {
                {
                    let mut state = svm_editor_toggle_state.borrow_mut();
                    state.message = error.clone();
                }
                apply_svm_editor_state(&ui, &svm_editor_toggle_state.borrow());
                ui.set_status_error_message(error.into());
            }
        }
    });

    let svm_editor_select_path_ui = ui.as_weak();
    let svm_editor_select_path_state = svm_editor_state.clone();
    ui.on_svm_editor_select_path(move |path_json| {
        let Some(ui) = svm_editor_select_path_ui.upgrade() else {
            return;
        };
        let result = {
            let mut state = svm_editor_select_path_state.borrow_mut();
            select_svm_editor_tree_path(&mut state, path_json.as_str())
        };
        match result {
            Ok(()) => apply_svm_editor_state(&ui, &svm_editor_select_path_state.borrow()),
            Err(error) => {
                {
                    let mut state = svm_editor_select_path_state.borrow_mut();
                    state.message = error.clone();
                }
                apply_svm_editor_state(&ui, &svm_editor_select_path_state.borrow());
                ui.set_status_error_message(error.into());
            }
        }
    });

    let svm_editor_edit_ui = ui.as_weak();
    let svm_editor_edit_state = svm_editor_state.clone();
    ui.on_svm_editor_edit_setting(move |index, value| {
        let Some(ui) = svm_editor_edit_ui.upgrade() else {
            return;
        };
        let result = {
            let mut state = svm_editor_edit_state.borrow_mut();
            edit_svm_editor_setting(&mut state, index as usize, value.as_str())
        };
        match result {
            Ok(()) => apply_svm_editor_state(&ui, &svm_editor_edit_state.borrow()),
            Err(error) => {
                {
                    let mut state = svm_editor_edit_state.borrow_mut();
                    state.message = error.clone();
                }
                apply_svm_editor_state(&ui, &svm_editor_edit_state.borrow());
                ui.set_status_error_message(error.into());
            }
        }
    });

    let svm_editor_expand_ui = ui.as_weak();
    let svm_editor_expand_state = svm_editor_state.clone();
    ui.on_svm_editor_expand_all(move || {
        let Some(ui) = svm_editor_expand_ui.upgrade() else {
            return;
        };
        {
            let mut state = svm_editor_expand_state.borrow_mut();
            state.tree_expanded_all = true;
            state.message.clear();
        }
        apply_svm_editor_state(&ui, &svm_editor_expand_state.borrow());
    });

    let svm_editor_collapse_ui = ui.as_weak();
    let svm_editor_collapse_state = svm_editor_state.clone();
    ui.on_svm_editor_collapse_all(move || {
        let Some(ui) = svm_editor_collapse_ui.upgrade() else {
            return;
        };
        {
            let mut state = svm_editor_collapse_state.borrow_mut();
            state.tree_expanded_all = false;
            state.message.clear();
        }
        apply_svm_editor_state(&ui, &svm_editor_collapse_state.borrow());
    });

    let svm_editor_filter_ui = ui.as_weak();
    let svm_editor_filter_state = svm_editor_state.clone();
    ui.on_svm_editor_tree_filter_edited(move |value| {
        let Some(ui) = svm_editor_filter_ui.upgrade() else {
            return;
        };
        {
            let mut state = svm_editor_filter_state.borrow_mut();
            state.tree_filter = value.to_string();
        }
        apply_svm_editor_state(&ui, &svm_editor_filter_state.borrow());
    });

    let svm_editor_save_ui = ui.as_weak();
    let svm_editor_save_state = svm_editor_state.clone();
    ui.on_svm_editor_save(move || {
        let Some(ui) = svm_editor_save_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before saving SavedVariables.".into(),
            );
            return;
        };
        let result = {
            let mut state = svm_editor_save_state.borrow_mut();
            save_svm_editor_file(&addons_root, &mut state)
        };
        match result {
            Ok(()) => {
                apply_svm_editor_state(&ui, &svm_editor_save_state.borrow());
                ui.set_status_error_message("Saved SavedVariables changes.".into());
            }
            Err(error) => {
                {
                    let mut state = svm_editor_save_state.borrow_mut();
                    state.message = format!("Save failed: {error}");
                }
                apply_svm_editor_state(&ui, &svm_editor_save_state.borrow());
                ui.set_status_error_message(format!("SavedVariables save failed: {error}").into());
            }
        }
    });

    let svm_editor_preview_ui = ui.as_weak();
    let svm_editor_preview_state = svm_editor_state.clone();
    ui.on_svm_editor_preview(move || {
        let Some(ui) = svm_editor_preview_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before previewing SavedVariables.".into(),
            );
            return;
        };
        let result = {
            let mut state = svm_editor_preview_state.borrow_mut();
            preview_svm_editor_file(&addons_root, &mut state)
        };
        match result {
            Ok(()) => apply_svm_editor_state(&ui, &svm_editor_preview_state.borrow()),
            Err(error) => {
                {
                    let mut state = svm_editor_preview_state.borrow_mut();
                    state.message = format!("Preview failed: {error}");
                }
                apply_svm_editor_state(&ui, &svm_editor_preview_state.borrow());
                ui.set_status_error_message(
                    format!("SavedVariables preview failed: {error}").into(),
                );
            }
        }
    });

    let svm_editor_discard_ui = ui.as_weak();
    let svm_editor_discard_state = svm_editor_state.clone();
    ui.on_svm_editor_discard(move || {
        let Some(ui) = svm_editor_discard_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            return;
        };
        {
            let mut state = svm_editor_discard_state.borrow_mut();
            if let Err(error) = load_svm_editor_selected_file(&addons_root, &mut state) {
                state.message = format!("Discard failed: {error}");
            } else {
                state.message = "Discarded unsaved changes.".to_string();
            }
        }
        apply_svm_editor_state(&ui, &svm_editor_discard_state.borrow());
    });

    let svm_editor_restore_ui = ui.as_weak();
    let svm_editor_restore_state = svm_editor_state.clone();
    ui.on_svm_editor_restore(move || {
        let Some(ui) = svm_editor_restore_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before restoring SavedVariables.".into(),
            );
            return;
        };
        let result = {
            let mut state = svm_editor_restore_state.borrow_mut();
            restore_svm_editor_backup(&addons_root, &mut state)
        };
        match result {
            Ok(()) => {
                apply_svm_editor_state(&ui, &svm_editor_restore_state.borrow());
                ui.set_status_error_message("Restored SavedVariables backup.".into());
            }
            Err(error) => {
                {
                    let mut state = svm_editor_restore_state.borrow_mut();
                    state.message = format!("Restore failed: {error}");
                }
                apply_svm_editor_state(&ui, &svm_editor_restore_state.borrow());
                ui.set_status_error_message(
                    format!("SavedVariables restore failed: {error}").into(),
                );
            }
        }
    });

    let svm_editor_raw_ui = ui.as_weak();
    let svm_editor_raw_state = svm_editor_state;
    ui.on_svm_editor_copy_raw(move || {
        let Some(ui) = svm_editor_raw_ui.upgrade() else {
            return;
        };
        let result = {
            let mut state = svm_editor_raw_state.borrow_mut();
            copy_svm_editor_raw_to_clipboard(&mut state)
        };
        match result {
            Ok(message) => {
                ui.set_status_error_message(message.into());
                apply_svm_editor_state(&ui, &svm_editor_raw_state.borrow());
            }
            Err(error) => {
                {
                    let mut state = svm_editor_raw_state.borrow_mut();
                    state.message = format!("Raw copy failed: {error}");
                }
                apply_svm_editor_state(&ui, &svm_editor_raw_state.borrow());
                ui.set_status_error_message(
                    format!("SavedVariables raw copy failed: {error}").into(),
                );
            }
        }
    });
}

fn apply_pack_hub_model(ui: &KalpaWindow, entries: Vec<PackHubEntry>) {
    ui.set_pack_hub_packs(Rc::new(VecModel::from(entries)).into());
}

fn pack_hub_entries_from_model(ui: &KalpaWindow) -> Vec<PackHubEntry> {
    let model = ui.get_pack_hub_packs();
    (0..model.row_count())
        .filter_map(|index| model.row_data(index))
        .collect()
}

fn append_pack_hub_model(ui: &KalpaWindow, entries: Vec<PackHubEntry>) {
    let mut rows = pack_hub_entries_from_model(ui);
    rows.extend(entries);
    apply_pack_hub_model(ui, rows);
}

fn apply_installed_pack_refs(ui: &KalpaWindow, refs: Vec<NativeInstalledPackRef>) {
    let entries = refs
        .into_iter()
        .map(pack_hub_installed_entry_from_ref)
        .collect::<Vec<_>>();
    ui.set_pack_hub_installed_packs(Rc::new(VecModel::from(entries)).into());
}

fn pack_hub_installed_entries_from_model(ui: &KalpaWindow) -> Vec<PackHubInstalledPackEntry> {
    let model = ui.get_pack_hub_installed_packs();
    (0..model.row_count())
        .filter_map(|index| model.row_data(index))
        .collect()
}

fn pack_hub_installed_entry_from_ref(
    reference: NativeInstalledPackRef,
) -> PackHubInstalledPackEntry {
    let pack_type = normalize_pack_type_key(&reference.pack_type);
    PackHubInstalledPackEntry {
        pack_id: reference.pack_id.as_str().into(),
        title: reference.title.as_str().into(),
        pack_type_label: pack_type_label(&pack_type).into(),
        author: reference.author_name.as_str().into(),
        addon_count: addon_count_label(reference.addon_count).into(),
        installed_label: installed_pack_date_label(&reference.installed_at).into(),
        monogram: pack_monogram(&reference.title).into(),
        identity_kind: pack_identity_kind(&reference.pack_id, &reference.title, &pack_type),
        type_kind: pack_type_kind(&pack_type),
    }
}

fn ensure_pack_hub_entry_for_installed(
    ui: &KalpaWindow,
    entry: &PackHubInstalledPackEntry,
) -> usize {
    let mut rows = pack_hub_entries_from_model(ui);
    if let Some(index) = rows
        .iter()
        .position(|row| row.id.as_str() == entry.pack_id.as_str())
    {
        return index;
    }

    let pack_type = pack_type_key_from_kind(entry.type_kind);
    rows.push(PackHubEntry {
        id: entry.pack_id.clone(),
        title: entry.title.clone(),
        description: "Installed locally from Pack Hub.".into(),
        tag: fallback_pack_tag(pack_type).into(),
        addon_count: entry.addon_count.clone(),
        vote_count: "0".into(),
        author: entry.author.clone(),
        pack_type_label: entry.pack_type_label.clone(),
        updated_label: entry.installed_label.clone(),
        monogram: entry.monogram.clone(),
        author_initial: author_initial(entry.author.as_str()).into(),
        identity_kind: entry.identity_kind,
        type_kind: entry.type_kind,
        trial: false,
    });
    let index = rows.len() - 1;
    apply_pack_hub_model(ui, rows);
    index
}

fn pack_hub_type_filter_key(filter: i32) -> Option<&'static str> {
    match filter {
        1 => Some("addon-pack"),
        2 => Some("build-pack"),
        3 => Some("roster-pack"),
        _ => None,
    }
}

fn pack_hub_type_filter_label(filter: i32) -> &'static str {
    match filter {
        1 => "Addon Pack",
        2 => "Build Pack",
        3 => "Roster Pack",
        _ => "All",
    }
}

fn pack_hub_sort_key(sort: i32) -> &'static str {
    match sort {
        1 => "newest",
        2 => "updated",
        _ => "votes",
    }
}

fn pack_hub_sort_label(sort: i32) -> &'static str {
    match sort {
        1 => "Newest",
        2 => "Updated",
        _ => "Votes",
    }
}

fn apply_pack_hub_browse_state(ui: &KalpaWindow, state: &PackHubBrowseState) {
    ui.set_pack_hub_browse_query(state.query.clone().into());
    ui.set_pack_hub_browse_type_label(pack_hub_type_filter_label(state.type_filter).into());
    ui.set_pack_hub_browse_sort_label(pack_hub_sort_label(state.sort).into());
}

fn apply_pack_hub_detail_model(ui: &KalpaWindow, addons: Vec<PackHubAddonEntry>) {
    ui.set_pack_hub_install_label(pack_hub_install_label(&addons).into());
    ui.set_pack_hub_detail_addons(Rc::new(VecModel::from(addons)).into());
}

fn apply_pack_hub_import_model(
    ui: &KalpaWindow,
    entry: PackHubEntry,
    addons: Vec<PackHubAddonEntry>,
    has_settings: bool,
) {
    ui.set_pack_hub_import_has_settings(has_settings);
    ui.set_pack_hub_import_install_label(
        pack_hub_import_install_label(&addons, has_settings).into(),
    );
    ui.set_pack_hub_import_pack(entry);
    ui.set_pack_hub_import_addons(Rc::new(VecModel::from(addons)).into());
}

fn clear_pack_hub_import_model(ui: &KalpaWindow) {
    ui.set_pack_hub_import_message("".into());
    apply_pack_hub_import_model(ui, empty_pack_hub_entry(), Vec::new(), false);
}

fn apply_pack_hub_create_state(ui: &KalpaWindow, models: &AddonModels, state: &PackHubCreateState) {
    let filter = state.filter.trim().to_ascii_lowercase();
    let rows = models
        .all
        .borrow()
        .iter()
        .filter(|addon| pack_hub_create_filter_matches(addon, &filter))
        .filter_map(|addon| pack_hub_create_entry_from_addon(addon, state))
        .take(80)
        .collect::<Vec<_>>();

    ui.set_pack_hub_create_filter(state.filter.clone().into());
    ui.set_pack_hub_create_addons(Rc::new(VecModel::from(rows)).into());
    ui.set_pack_hub_create_selected_addons(Rc::new(VecModel::from(state.selected.clone())).into());
}

fn pack_hub_create_filter_matches(addon: &AddonEntry, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {} {} {}",
        addon.title.as_str(),
        addon.folder_name.as_str(),
        addon.author.as_str(),
        addon.esoui_id.as_str()
    )
    .to_ascii_lowercase();
    haystack.contains(filter)
}

fn pack_hub_create_entry_from_addon(
    addon: &AddonEntry,
    state: &PackHubCreateState,
) -> Option<PackHubCreateAddonEntry> {
    if addon.is_library {
        return None;
    }

    let esoui_id = normalized_pack_hub_create_id(addon.esoui_id.as_str())?;
    let title = if addon.title.as_str().trim().is_empty() {
        addon.folder_name.as_str()
    } else {
        addon.title.as_str()
    };
    let meta = pack_hub_create_addon_meta(addon, &esoui_id);
    let selected = state
        .selected
        .iter()
        .find(|entry| pack_hub_create_row_id(entry).as_deref() == Some(esoui_id.as_str()));

    Some(PackHubCreateAddonEntry {
        title: title.into(),
        meta: meta.into(),
        esoui_id: format!("#{esoui_id}").into(),
        selected: selected.is_some(),
        required: selected.map(|entry| entry.required).unwrap_or(true),
    })
}

fn pack_hub_create_addon_meta(addon: &AddonEntry, esoui_id: &str) -> String {
    let mut parts = Vec::new();
    if !addon.author.as_str().trim().is_empty() {
        parts.push(format!("by {}", addon.author.as_str()));
    }
    if !addon.version.as_str().trim().is_empty() {
        parts.push(format!("v{}", addon.version.as_str()));
    }
    parts.push(format!("#{esoui_id}"));
    parts.join(" - ")
}

fn normalized_pack_hub_create_id(value: &str) -> Option<String> {
    let id = value.trim().trim_start_matches('#').trim();
    if id.is_empty() || id == "0" || !id.chars().all(|ch| ch.is_ascii_digit()) {
        None
    } else {
        Some(id.to_string())
    }
}

fn pack_hub_create_row_id(row: &PackHubCreateAddonEntry) -> Option<String> {
    normalized_pack_hub_create_id(row.esoui_id.as_str())
}

fn toggle_pack_hub_create_row(state: &mut PackHubCreateState, mut row: PackHubCreateAddonEntry) {
    let Some(esoui_id) = pack_hub_create_row_id(&row) else {
        return;
    };
    if let Some(index) = state
        .selected
        .iter()
        .position(|entry| pack_hub_create_row_id(entry).as_deref() == Some(esoui_id.as_str()))
    {
        state.selected.remove(index);
    } else {
        row.selected = true;
        row.required = true;
        state.selected.push(row);
    }
}

fn remove_pack_hub_create_selected(state: &mut PackHubCreateState, row: PackHubCreateAddonEntry) {
    let Some(esoui_id) = pack_hub_create_row_id(&row) else {
        return;
    };
    state
        .selected
        .retain(|entry| pack_hub_create_row_id(entry).as_deref() != Some(esoui_id.as_str()));
}

fn toggle_pack_hub_create_required(state: &mut PackHubCreateState, row: PackHubCreateAddonEntry) {
    let Some(esoui_id) = pack_hub_create_row_id(&row) else {
        return;
    };
    if let Some(entry) = state
        .selected
        .iter_mut()
        .find(|entry| pack_hub_create_row_id(entry).as_deref() == Some(esoui_id.as_str()))
    {
        entry.required = !entry.required;
    }
}

fn export_pack_hub_create_file(
    title: &str,
    description: &str,
    pack_type_kind: i32,
    selected: &[PackHubCreateAddonEntry],
    export_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Pack needs a title.".to_string());
    }
    if selected.is_empty() {
        return Err("Add at least one addon.".to_string());
    }

    let pack_type = pack_type_key_from_kind(pack_type_kind).to_string();
    let addons = selected
        .iter()
        .map(pack_hub_create_export_addon)
        .collect::<Result<Vec<_>, _>>()?;
    let pack = NativeEsoPackFile {
        format: "esopack".to_string(),
        version: 1,
        pack: NativeSharedPackBody {
            title: title.to_string(),
            description: description.trim().to_string(),
            pack_type: pack_type.clone(),
            tags: vec![fallback_pack_tag(&pack_type)],
            addons,
        },
        shared_at: current_iso_utc(),
        shared_by: "Kalpa Native".to_string(),
        settings: HashMap::new(),
    };

    let export_dir = export_dir
        .map(PathBuf::from)
        .unwrap_or_else(default_pack_export_dir);
    fs::create_dir_all(&export_dir)
        .map_err(|error| format!("Failed to create export folder: {error}"))?;
    let path = unique_export_path(&export_dir, safe_pack_file_stem(title));
    let json = serde_json::to_string_pretty(&pack)
        .map_err(|error| format!("Failed to serialize pack: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("Failed to write pack file: {error}"))?;
    Ok(path)
}

fn pack_hub_create_export_addon(
    row: &PackHubCreateAddonEntry,
) -> Result<NativePackAddonEntry, String> {
    let esoui_id = pack_hub_create_row_id(row)
        .ok_or_else(|| format!("Invalid ESOUI id for {}.", row.title.as_str()))?
        .parse::<u32>()
        .map_err(|_| format!("Invalid ESOUI id for {}.", row.title.as_str()))?;
    Ok(NativePackAddonEntry {
        esoui_id,
        name: row.title.to_string(),
        required: row.required,
        note: None,
    })
}

fn export_pack_hub_detail_file(
    entry: &PackHubEntry,
    addons: &[PackHubAddonEntry],
    export_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    let title = entry.title.as_str().trim();
    if title.is_empty() {
        return Err("Pack needs a title.".to_string());
    }
    if addons.is_empty() {
        return Err("Pack has no addons to share.".to_string());
    }

    let pack_type = pack_type_key_from_kind(entry.type_kind).to_string();
    let tag = entry.tag.as_str().trim();
    let tags = if tag.is_empty() {
        vec![fallback_pack_tag(&pack_type)]
    } else {
        vec![tag.to_string()]
    };
    let export_addons = addons
        .iter()
        .map(pack_hub_detail_export_addon)
        .collect::<Result<Vec<_>, _>>()?;
    let shared_by = entry.author.as_str().trim();
    let pack = NativeEsoPackFile {
        format: "esopack".to_string(),
        version: 1,
        pack: NativeSharedPackBody {
            title: title.to_string(),
            description: entry.description.as_str().trim().to_string(),
            pack_type,
            tags,
            addons: export_addons,
        },
        shared_at: current_iso_utc(),
        shared_by: if shared_by.is_empty() {
            "Kalpa Native".to_string()
        } else {
            shared_by.to_string()
        },
        settings: HashMap::new(),
    };

    let export_dir = export_dir
        .map(PathBuf::from)
        .unwrap_or_else(default_pack_export_dir);
    fs::create_dir_all(&export_dir)
        .map_err(|error| format!("Failed to create export folder: {error}"))?;
    let path = unique_export_path(&export_dir, safe_pack_file_stem(title));
    let json = serde_json::to_string_pretty(&pack)
        .map_err(|error| format!("Failed to serialize pack: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("Failed to write pack file: {error}"))?;
    Ok(path)
}

fn pack_hub_detail_export_addon(row: &PackHubAddonEntry) -> Result<NativePackAddonEntry, String> {
    let note = row.note.as_str().trim().to_string();
    Ok(NativePackAddonEntry {
        esoui_id: pack_hub_row_esoui_id(row)?,
        name: row.title.to_string(),
        required: row.required,
        note: (!note.is_empty()).then_some(note),
    })
}

fn default_pack_export_dir() -> PathBuf {
    user_documents_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Kalpa")
        .join("Exports")
}

fn user_documents_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join("Documents"))
}

fn safe_pack_file_stem(title: &str) -> String {
    let mut stem = String::new();
    let mut last_dash = false;
    for ch in title.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            stem.push(ch);
            last_dash = false;
        } else if ch.is_whitespace() && !last_dash && !stem.is_empty() {
            stem.push('-');
            last_dash = true;
        }
    }
    let stem = stem.trim_matches('-');
    if stem.is_empty() {
        "kalpa-pack".to_string()
    } else {
        stem.to_string()
    }
}

fn unique_export_path(dir: &Path, stem: String) -> PathBuf {
    let mut path = dir.join(format!("{stem}.esopack"));
    let mut suffix = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{suffix}.esopack"));
        suffix += 1;
    }
    path
}

fn empty_pack_hub_entry() -> PackHubEntry {
    PackHubEntry {
        id: "".into(),
        title: "".into(),
        description: "".into(),
        tag: "".into(),
        addon_count: "0 addons".into(),
        vote_count: "0".into(),
        author: "".into(),
        pack_type_label: "Addon Pack".into(),
        updated_label: "".into(),
        monogram: "?".into(),
        author_initial: "?".into(),
        identity_kind: 0,
        type_kind: 0,
        trial: false,
    }
}

fn fallback_pack_hub_entries() -> Vec<PackHubEntry> {
    vec![
        PackHubEntry {
            id: "demo-spikes-utilities".into(),
            title: "Spike's Utilities".into(),
            description: "just some addons I always use, i really like Caro's!!".into(),
            tag: "utility".into(),
            addon_count: "22 addons".into(),
            vote_count: "1".into(),
            author: "Spike'jo".into(),
            pack_type_label: "Addon Pack".into(),
            updated_label: "Updated recently".into(),
            monogram: pack_monogram("Spike's Utilities").into(),
            author_initial: author_initial("Spike'jo").into(),
            identity_kind: pack_identity_kind(
                "demo-spikes-utilities",
                "Spike's Utilities",
                "addon-pack",
            ),
            type_kind: pack_type_kind("addon-pack"),
            trial: false,
        },
        PackHubEntry {
            id: "demo-spikes-trial-necessities".into(),
            title: "Spike's Trial Necessities".into(),
            description: "just some addons i never play without".into(),
            tag: "trial".into(),
            addon_count: "18 addons".into(),
            vote_count: "1".into(),
            author: "Spike'jo".into(),
            pack_type_label: "Addon Pack".into(),
            updated_label: "Updated recently".into(),
            monogram: pack_monogram("Spike's Trial Necessities").into(),
            author_initial: author_initial("Spike'jo").into(),
            identity_kind: pack_identity_kind(
                "demo-spikes-trial-necessities",
                "Spike's Trial Necessities",
                "addon-pack",
            ),
            type_kind: pack_type_kind("addon-pack"),
            trial: true,
        },
    ]
}

fn request_pack_hub_browse_page(
    ui: &KalpaWindow,
    browse_state: Arc<Mutex<PackHubBrowseState>>,
    request_counter: Arc<AtomicU64>,
    append: bool,
) {
    let request_id = request_counter.fetch_add(1, Ordering::SeqCst) + 1;
    let state_snapshot = {
        let mut state = browse_state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.normalize();
        apply_pack_hub_browse_state(ui, &state);
        state.clone()
    };

    ui.set_pack_hub_browse_loading(true);
    ui.set_pack_hub_browse_message("".into());
    ui.set_status_error_message("Loading Pack Hub packs...".into());

    let ui_weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = fetch_pack_hub_packs_blocking(&state_snapshot);
        let _ = slint::invoke_from_event_loop(move || {
            if request_counter.load(Ordering::SeqCst) != request_id {
                return;
            }
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            ui.set_pack_hub_browse_loading(false);
            match result {
                Ok(page) => {
                    {
                        let mut state = browse_state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        *state = state_snapshot;
                        state.page = page.page.max(1);
                        state.normalize();
                        apply_pack_hub_browse_state(&ui, &state);
                    }
                    ui.set_pack_hub_browse_has_more(page.has_more);

                    if append {
                        if page.entries.is_empty() {
                            ui.set_status_error_message("No more Pack Hub packs to load.".into());
                        } else {
                            append_pack_hub_model(&ui, page.entries);
                            ui.set_pack_hub_browse_message("".into());
                            ui.set_status_error_message("".into());
                        }
                    } else {
                        let empty = page.entries.is_empty();
                        apply_pack_hub_model(&ui, page.entries);
                        ui.set_pack_hub_selected_index(0);
                        ui.set_pack_hub_browse_message("".into());
                        ui.set_status_error_message(
                            if empty {
                                "No Pack Hub packs match the current filters."
                            } else {
                                ""
                            }
                            .into(),
                        );
                    }
                }
                Err(error) => {
                    ui.set_pack_hub_browse_has_more(false);
                    if !append {
                        apply_pack_hub_model(&ui, Vec::new());
                        ui.set_pack_hub_browse_message(error.clone().into());
                    }
                    ui.set_status_error_message(
                        format!("Could not load Pack Hub packs: {error}").into(),
                    );
                }
            }
        });
    });
}

fn request_pack_hub_detail(
    ui: &KalpaWindow,
    index: usize,
    pack_id: String,
    installed_ids: BTreeSet<String>,
) {
    let ui_weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = fetch_pack_hub_detail_blocking(&pack_id, &installed_ids);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            ui.set_pack_hub_detail_loading(false);
            match result {
                Ok(detail) => {
                    let packs = ui.get_pack_hub_packs();
                    if index < packs.row_count() {
                        packs.set_row_data(index, detail.entry);
                    }
                    let empty = detail.addons.is_empty();
                    apply_pack_hub_detail_model(&ui, detail.addons);
                    ui.set_pack_hub_detail_message(
                        if empty {
                            "No addons are listed for this pack."
                        } else {
                            ""
                        }
                        .into(),
                    );
                }
                Err(error) => {
                    apply_pack_hub_detail_model(&ui, Vec::new());
                    ui.set_pack_hub_detail_message(
                        format!("Could not load pack details: {error}").into(),
                    );
                }
            }
        });
    });
}

fn pack_hub_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("PACK_HUB_API_URL")
            .unwrap_or_else(|_| "https://kalpa-pack-hub.eso-toolkit.workers.dev".to_string())
    })
}

fn pack_hub_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent("Kalpa Slint Prototype")
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("failed to build Pack Hub HTTP client")
    })
}

fn share_worker_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("SHARE_WORKER_URL")
            .or_else(|_| std::env::var("PACK_HUB_API_URL"))
            .unwrap_or_else(|_| "https://kalpa-pack-hub.eso-toolkit.workers.dev".to_string())
    })
}

fn normalize_share_code(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .take(6)
        .collect::<String>()
        .to_ascii_uppercase()
}

fn validate_share_code(value: &str) -> Result<String, String> {
    let code = normalize_share_code(value);
    let valid = code.len() == 6
        && code
            .chars()
            .all(|ch| matches!(ch, '2'..='9' | 'A'..='H' | 'J'..='N' | 'P'..='Z'));
    if valid {
        Ok(code)
    } else {
        Err("Invalid share code format.".to_string())
    }
}

fn fetch_pack_hub_packs_blocking(state: &PackHubBrowseState) -> Result<NativePackPageData, String> {
    const PAGE_SIZE: usize = 10;
    let mut query_params = vec![
        ("sort", pack_hub_sort_key(state.sort).to_string()),
        ("page", state.page.max(1).to_string()),
    ];
    if let Some(pack_type) = pack_hub_type_filter_key(state.type_filter) {
        query_params.push(("type", pack_type.to_string()));
    }
    let query = state.query.trim();
    if !query.is_empty() {
        query_params.push(("q", query.to_string()));
    }

    let response = pack_hub_client()
        .get(format!("{}/packs", pack_hub_url()))
        .query(&query_params)
        .send()
        .map_err(|error| {
            if error.is_connect() || error.is_timeout() {
                "Could not connect to Pack Hub. Check your internet connection.".to_string()
            } else {
                format!("Network error: {error}")
            }
        })?;

    if !response.status().is_success() {
        return Err(format!("Pack Hub returned HTTP {}", response.status()));
    }

    let body: NativePackListResponse = response
        .json()
        .map_err(|error| format!("Failed to parse packs response: {error}"))?;
    let NativePackListResponse { packs, page } = body;
    let has_more = packs.len() >= PAGE_SIZE;
    let entries = packs
        .into_iter()
        .filter(|pack| pack.status.as_deref() != Some("draft"))
        .map(pack_hub_entry_from_hub)
        .collect();

    Ok(NativePackPageData {
        entries,
        page,
        has_more,
    })
}

fn fetch_pack_hub_detail_blocking(
    pack_id: &str,
    installed_ids: &BTreeSet<String>,
) -> Result<NativePackDetailData, String> {
    let response = pack_hub_client()
        .get(format!("{}/packs/{}", pack_hub_url(), pack_id))
        .send()
        .map_err(|error| {
            if error.is_connect() || error.is_timeout() {
                "Could not connect to Pack Hub. Check your internet connection.".to_string()
            } else {
                format!("Network error: {error}")
            }
        })?;

    match response.status().as_u16() {
        200 => {}
        404 => return Err(format!("Pack \"{pack_id}\" was not found.")),
        status => return Err(format!("Pack Hub returned HTTP {status}")),
    }

    let body: NativePackSingleResponse = response
        .json()
        .map_err(|error| format!("Failed to parse pack detail response: {error}"))?;

    Ok(pack_hub_detail_from_hub(body.pack, installed_ids))
}

fn track_pack_install_blocking(pack_id: &str) {
    let pack_id = pack_id.trim();
    if pack_id.is_empty() || pack_id.contains('/') || pack_id.contains('\\') {
        return;
    }
    drop(
        pack_hub_client()
            .post(format!("{}/packs/{pack_id}/install", pack_hub_url()))
            .send(),
    );
}

fn fetch_shared_pack_blocking(
    code: &str,
    installed_ids: &BTreeSet<String>,
) -> Result<NativePackDetailData, String> {
    let code = validate_share_code(code)?;
    let response = pack_hub_client()
        .get(format!("{}/shares/{}", share_worker_url(), code))
        .send()
        .map_err(|error| {
            if error.is_connect() || error.is_timeout() {
                "Could not connect to share service. Check your internet connection.".to_string()
            } else {
                format!("Network error: {error}")
            }
        })?;

    match response.status().as_u16() {
        200 => {}
        400 => return Err("Invalid share code format.".to_string()),
        404 => return Err("Share code not found or expired.".to_string()),
        status => return Err(format!("Share service returned HTTP {status}")),
    }

    let body: NativeSharedPackResponse = response
        .json()
        .map_err(|error| format!("Failed to parse share response: {error}"))?;

    Ok(pack_hub_detail_from_shared_pack(&code, body, installed_ids))
}

fn import_esopack_file_blocking(
    path: &Path,
    installed_ids: &BTreeSet<String>,
) -> Result<NativePackFileImportData, String> {
    if path.extension().and_then(|extension| extension.to_str()) != Some("esopack") {
        return Err("Only .esopack files can be imported.".to_string());
    }

    let path = path
        .canonicalize()
        .map_err(|_| "File not found.".to_string())?;
    let metadata = fs::metadata(&path).map_err(|error| format!("Failed to read file: {error}"))?;
    if metadata.len() > 10 * 1024 * 1024 {
        return Err("File is too large (max 10 MB).".to_string());
    }

    let contents =
        fs::read_to_string(&path).map_err(|error| format!("Failed to read file: {error}"))?;
    let pack: NativeEsoPackFile = serde_json::from_str(&contents)
        .map_err(|error| format!("Invalid .esopack file: {error}"))?;

    if pack.format != "esopack" {
        return Err("Not a valid .esopack file (wrong format field).".to_string());
    }
    if pack.version != 1 && pack.version != 2 {
        return Err(format!(
            "Unsupported .esopack version {}. Please update the app.",
            pack.version
        ));
    }

    let id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("imported-pack");
    let detail = pack_hub_detail_from_imported_pack(
        &format!("file:{id}"),
        pack.pack,
        pack.shared_by,
        pack.shared_at,
        installed_ids,
    );

    Ok(NativePackFileImportData {
        detail,
        settings: pack.settings,
    })
}

fn pack_hub_detail_from_hub(
    hub: NativeHubPack,
    installed_ids: &BTreeSet<String>,
) -> NativePackDetailData {
    let addons = native_pack_addons(&hub.addons);
    let entry = pack_hub_entry_from_hub_with_count(&hub, addons.len());
    let addon_rows = addons
        .into_iter()
        .map(|addon| pack_hub_addon_entry(addon, installed_ids))
        .collect();

    NativePackDetailData {
        entry,
        addons: addon_rows,
    }
}

fn pack_hub_detail_from_shared_pack(
    code: &str,
    shared: NativeSharedPackResponse,
    installed_ids: &BTreeSet<String>,
) -> NativePackDetailData {
    let NativeSharedPackResponse {
        pack,
        shared_by,
        shared_at,
        _expires_at: _,
    } = shared;
    pack_hub_detail_from_imported_pack(
        &format!("share:{code}"),
        pack,
        shared_by,
        shared_at,
        installed_ids,
    )
}

fn pack_hub_detail_from_imported_pack(
    id: &str,
    pack: NativeSharedPackBody,
    shared_by: String,
    shared_at: String,
    installed_ids: &BTreeSet<String>,
) -> NativePackDetailData {
    let addon_count = pack.addons.len();
    let tag = pack
        .tags
        .first()
        .cloned()
        .filter(|tag| !tag.trim().is_empty())
        .unwrap_or_else(|| fallback_pack_tag(&pack.pack_type));
    let trial = pack
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case("trial"))
        || pack.pack_type.eq_ignore_ascii_case("trial");
    let author = if shared_by.trim().is_empty() {
        "Friend".to_string()
    } else {
        shared_by
    };
    let updated_label = pack_iso_date_label(&shared_at)
        .map(|date| format!("Shared {date}"))
        .unwrap_or_else(|| "Shared pack".to_string());

    let entry = PackHubEntry {
        id: id.into(),
        title: pack.title.as_str().into(),
        description: pack.description.as_str().into(),
        tag: tag.into(),
        addon_count: addon_count_label(addon_count).into(),
        vote_count: "0".into(),
        author: author.as_str().into(),
        pack_type_label: pack_type_label(&pack.pack_type).into(),
        updated_label: updated_label.into(),
        monogram: pack_monogram(&pack.title).into(),
        author_initial: author_initial(&author).into(),
        identity_kind: pack_identity_kind(id, &pack.title, &pack.pack_type),
        type_kind: pack_type_kind(&pack.pack_type),
        trial,
    };
    let addons = pack
        .addons
        .into_iter()
        .map(|addon| pack_hub_addon_entry(addon, installed_ids))
        .collect();

    NativePackDetailData { entry, addons }
}

fn pack_hub_entry_from_hub(hub: NativeHubPack) -> PackHubEntry {
    let addons = native_pack_addons(&hub.addons);
    pack_hub_entry_from_hub_with_count(&hub, addons.len())
}

fn pack_hub_entry_from_hub_with_count(hub: &NativeHubPack, addon_count: usize) -> PackHubEntry {
    let tag = hub
        .tags
        .first()
        .cloned()
        .filter(|tag| !tag.trim().is_empty())
        .unwrap_or_else(|| fallback_pack_tag(&hub.pack_type));
    let trial = hub.tags.iter().any(|tag| tag.eq_ignore_ascii_case("trial"))
        || hub.pack_type.eq_ignore_ascii_case("trial");

    let author = if hub.is_anonymous {
        "Anonymous".to_string()
    } else {
        hub.author_name.clone()
    };

    PackHubEntry {
        id: hub.id.as_str().into(),
        title: hub.title.as_str().into(),
        description: hub.description.as_str().into(),
        tag: tag.into(),
        addon_count: addon_count_label(addon_count).into(),
        vote_count: hub.vote_count.max(0).to_string().into(),
        author: author.as_str().into(),
        pack_type_label: pack_type_label(&hub.pack_type).into(),
        updated_label: pack_updated_label(&hub.created_at, &hub.updated_at).into(),
        monogram: pack_monogram(&hub.title).into(),
        author_initial: author_initial(&author).into(),
        identity_kind: pack_identity_kind(&hub.id, &hub.title, &hub.pack_type),
        type_kind: pack_type_kind(&hub.pack_type),
        trial,
    }
}

fn pack_hub_addon_entry(
    addon: NativePackAddonEntry,
    installed_ids: &BTreeSet<String>,
) -> PackHubAddonEntry {
    let esoui_id = addon.esoui_id.to_string();
    PackHubAddonEntry {
        title: addon.name.into(),
        esoui_id: format!("#{esoui_id}").into(),
        required: addon.required,
        installed: installed_ids.contains(&esoui_id),
        selected: addon.required,
        note: addon.note.unwrap_or_default().into(),
    }
}

fn native_pack_addons(addons: &serde_json::Value) -> Vec<NativePackAddonEntry> {
    match addons {
        serde_json::Value::String(value) => serde_json::from_str(value).unwrap_or_default(),
        serde_json::Value::Array(_) => serde_json::from_value(addons.clone()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn pack_hub_install_label(addons: &[PackHubAddonEntry]) -> String {
    let missing = addons.iter().filter(|addon| !addon.installed).count();
    if missing == 0 {
        return "All Addons Installed".to_string();
    }

    let selected_missing = addons
        .iter()
        .filter(|addon| !addon.installed && addon.selected)
        .count();
    match selected_missing {
        0 => "Select Addons to Install".to_string(),
        1 => "Install 1 New Addon".to_string(),
        count => format!("Install {count} New Addons"),
    }
}

fn pack_hub_import_install_label(addons: &[PackHubAddonEntry], has_settings: bool) -> String {
    match addons
        .iter()
        .filter(|addon| !addon.installed && addon.selected)
        .count()
    {
        0 if has_settings => "Apply Settings".to_string(),
        0 => "All Addons Installed".to_string(),
        1 => "Install 1 New Addon".to_string(),
        count => format!("Install {count} New Addons"),
    }
}

fn hash_pack_id(input: &str) -> u32 {
    input.bytes().fold(0x811c9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x01000193)
    })
}

fn pack_monogram(title: &str) -> String {
    let words = title
        .split(|ch: char| !(ch.is_alphanumeric() || ch.is_whitespace()))
        .flat_map(str::split_whitespace)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();

    match words.as_slice() {
        [] => "?".to_string(),
        [word] => word.chars().take(2).collect::<String>().to_uppercase(),
        [first, .., last] => {
            let mut mono = String::new();
            if let Some(ch) = first.chars().next() {
                mono.push(ch);
            }
            if let Some(ch) = last.chars().next() {
                mono.push(ch);
            }
            if mono.is_empty() {
                "?".to_string()
            } else {
                mono.to_uppercase()
            }
        }
    }
}

fn author_initial(author: &str) -> String {
    author
        .chars()
        .find(|ch| ch.is_alphanumeric())
        .map(|ch| ch.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_string())
}

fn pack_type_kind(pack_type: &str) -> i32 {
    match pack_type {
        "build" | "build-pack" => 1,
        "roster" | "roster-pack" => 2,
        _ => 0,
    }
}

fn pack_identity_kind(pack_id: &str, title: &str, pack_type: &str) -> i32 {
    const IDENTITY_COUNT: u32 = 7;
    let seed = if pack_id.trim().is_empty() {
        title
    } else {
        pack_id
    };
    let mut kind = hash_pack_id(seed) % IDENTITY_COUNT;
    let type_kind = match pack_type_kind(pack_type) {
        0 => 0,
        1 => 1,
        2 => 6,
        _ => 0,
    };
    if kind as i32 == type_kind {
        kind = (kind + 1) % IDENTITY_COUNT;
    }
    kind as i32
}

fn pack_hub_detail_addons(ui: &KalpaWindow) -> Vec<PackHubAddonEntry> {
    let model = ui.get_pack_hub_detail_addons();
    (0..model.row_count())
        .filter_map(|index| model.row_data(index))
        .collect()
}

fn pack_hub_import_addons(ui: &KalpaWindow) -> Vec<PackHubAddonEntry> {
    let model = ui.get_pack_hub_import_addons();
    (0..model.row_count())
        .filter_map(|index| model.row_data(index))
        .collect()
}

fn selected_pack_hub_entry(ui: &KalpaWindow) -> Option<PackHubEntry> {
    let index = ui.get_pack_hub_selected_index().max(0) as usize;
    ui.get_pack_hub_packs().row_data(index)
}

fn selected_pack_hub_id(ui: &KalpaWindow) -> Option<String> {
    selected_pack_hub_entry(ui)
        .map(|pack| pack.id.to_string())
        .filter(|id| !id.trim().is_empty())
}

fn install_pack_hub_addons_blocking(
    addons_dir: &Path,
    rows: Vec<PackHubAddonEntry>,
) -> NativePackInstallResult {
    let mut result = NativePackInstallResult {
        rows: Vec::with_capacity(rows.len()),
        installed: 0,
        failed: 0,
        folders: 0,
        errors: Vec::new(),
    };

    for mut row in rows {
        if row.installed || !row.selected {
            result.rows.push(row);
            continue;
        }

        match pack_hub_row_esoui_id(&row)
            .and_then(|esoui_id| install_pack_hub_addon_blocking(addons_dir, esoui_id))
        {
            Ok(folders) => {
                row.installed = true;
                result.installed += 1;
                result.folders += folders.len();
            }
            Err(error) => {
                result.failed += 1;
                result
                    .errors
                    .push(format!("{}: {error}", row.title.as_str()));
            }
        }

        result.rows.push(row);
    }

    result
}

fn pack_hub_row_esoui_id(row: &PackHubAddonEntry) -> Result<u32, String> {
    let id = row.esoui_id.as_str().trim().trim_start_matches('#').trim();
    id.parse::<u32>()
        .map_err(|_| format!("Invalid ESOUI id {}", row.esoui_id.as_str()))
}

fn install_pack_hub_addon_blocking(
    addons_dir: &Path,
    esoui_id: u32,
) -> Result<Vec<String>, String> {
    let detail = esoui::fetch_addon_detail(esoui_id)?;
    let expected_md5 = (!detail.md5.trim().is_empty()).then_some(detail.md5.as_str());
    let tmp_file = esoui::download_addon(&detail.download_url, expected_md5)?;
    install_discover_download_blocking(addons_dir, tmp_file.path(), &detail)
}

fn pack_hub_install_summary(result: &NativePackInstallResult) -> String {
    if result.failed == 0 {
        return format!(
            "Installed {} Pack Hub addon{} ({} folder{} added).",
            result.installed,
            if result.installed == 1 { "" } else { "s" },
            result.folders,
            if result.folders == 1 { "" } else { "s" },
        );
    }

    let first_error = result.errors.first().cloned().unwrap_or_default();
    format!(
        "Installed {} Pack Hub addon{}, {} failed. {}",
        result.installed,
        if result.installed == 1 { "" } else { "s" },
        result.failed,
        first_error
    )
}

fn pack_hub_settings_summary(result: &NativeSvImportResult) -> String {
    let mut summary = format!(
        "Applied settings for {} addon{}",
        result.applied.len(),
        if result.applied.len() == 1 { "" } else { "s" }
    );
    if !result.skipped.is_empty() {
        summary.push_str(&format!(", {} skipped", result.skipped.len()));
    }
    if !result.errors.is_empty() {
        summary.push_str(&format!(
            ", {} failed: {}",
            result.errors.len(),
            result.errors.join("; ")
        ));
    }
    summary.push('.');
    summary
}

fn pack_hub_import_combined_summary(
    install_summary: &str,
    settings: &NativeSvImportResult,
    install_count: usize,
) -> String {
    if install_count == 0 {
        return pack_hub_settings_summary(settings);
    }
    format!("{install_summary} {}", pack_hub_settings_summary(settings))
}

fn apply_imported_pack_settings_blocking(
    addons_dir: &Path,
    settings: HashMap<String, NativeAddonSettings>,
) -> NativeSvImportResult {
    if settings.is_empty() {
        return NativeSvImportResult::default();
    }

    let sv_dir = settings_saved_variables_dir(addons_dir);
    if let Err(error) = fs::create_dir_all(&sv_dir) {
        return NativeSvImportResult {
            errors: vec![format!(
                "Failed to create SavedVariables directory: {error}"
            )],
            ..Default::default()
        };
    }

    let ctx = collect_import_saved_variable_identities(addons_dir);
    let mut result = NativeSvImportResult::default();
    let mut folders = settings.keys().cloned().collect::<Vec<_>>();
    folders.sort();

    for folder in folders {
        if let Err(error) = validate_addon_folder_name(&folder) {
            result
                .errors
                .push(format!("{folder}: invalid folder name: {error}"));
            continue;
        }

        let Some(entry) = settings.get(folder.as_str()) else {
            result.skipped.push(folder);
            continue;
        };

        if entry.encoding != "lua-text" {
            result.errors.push(format!(
                "{}: unsupported encoding '{}'",
                folder, entry.encoding
            ));
            continue;
        }

        let substituted = saved_variables::scrub::substitute_placeholders(
            &entry.lua,
            &ctx,
            saved_variables::scrub::WELL_KNOWN_WORLDS,
        );
        if has_unresolved_identity_placeholders(&substituted) {
            result.errors.push(format!(
                "{folder}: unresolved identity placeholders - launch ESO at least once to establish your identity"
            ));
            continue;
        }

        let file_name = format!("{folder}.lua");
        if let Err(error) = saved_variables::parser::parse_sv_file(&substituted, &file_name) {
            result.errors.push(format!(
                "{folder}: settings file failed validation: {error}"
            ));
            continue;
        }

        let destination = sv_dir.join(&file_name);
        if destination.is_file() {
            let backup = destination.with_extension("lua.bak");
            if let Err(error) = fs::copy(&destination, &backup) {
                result
                    .errors
                    .push(format!("{folder}: failed to create backup: {error}"));
                continue;
            }
        }

        let temp = sv_dir.join(format!("{folder}.lua.tmp"));
        if let Err(error) = fs::write(&temp, &substituted) {
            result
                .errors
                .push(format!("{folder}: failed to write: {error}"));
            continue;
        }
        if destination.exists() {
            if let Err(error) = fs::remove_file(&destination) {
                let _ = fs::remove_file(&temp);
                result.errors.push(format!(
                    "{folder}: failed to replace existing file: {error}"
                ));
                continue;
            }
        }
        if let Err(error) = fs::rename(&temp, &destination) {
            let _ = fs::remove_file(&temp);
            result
                .errors
                .push(format!("{folder}: failed to finalize write: {error}"));
            continue;
        }

        result.applied.push(folder);
    }

    result
}

fn collect_import_saved_variable_identities(
    addons_dir: &Path,
) -> saved_variables::scrub::ScrubContext {
    let sv_dir = settings_saved_variables_dir(addons_dir);
    let mut merged = saved_variables::scrub::ScrubContext::default();
    let Ok(entries) = fs::read_dir(&sv_dir) else {
        return merged;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("lua") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("SavedVariables.lua");
        let Ok(tree) = saved_variables::parser::parse_sv_file(&content, file_name) else {
            continue;
        };
        let ctx = saved_variables::scrub::detect_identities_from_tree(&tree);
        merge_unique_strings(&mut merged.accounts, ctx.accounts);
        merge_unique_strings(&mut merged.characters, ctx.characters);
        merge_unique_strings(&mut merged.character_ids, ctx.character_ids);
        merge_unique_strings(&mut merged.extra_worlds, ctx.extra_worlds);
    }

    merged
}

fn merge_unique_strings(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn has_unresolved_identity_placeholders(lua: &str) -> bool {
    lua.contains("${ACCOUNT}")
        || lua.contains("${ACCOUNT:")
        || lua.contains("${ACCOUNT_NAME}")
        || lua.contains("${ACCOUNT_NAME:")
        || lua.contains("${CHAR:")
        || lua.contains("${CHAR_ID:")
}

fn addon_count_label(count: usize) -> String {
    format!("{count} addon{}", if count == 1 { "" } else { "s" })
}

fn pack_type_label(pack_type: &str) -> String {
    match pack_type {
        "build" | "build-pack" => "Build Pack",
        "roster" | "roster-pack" => "Roster Pack",
        _ => "Addon Pack",
    }
    .to_string()
}

fn normalize_pack_type_key(pack_type: &str) -> String {
    match pack_type {
        "build" | "build-pack" | "Build Pack" => "build-pack",
        "roster" | "roster-pack" | "Roster Pack" => "roster-pack",
        _ => "addon-pack",
    }
    .to_string()
}

fn pack_type_key_from_kind(kind: i32) -> &'static str {
    match kind {
        1 => "build-pack",
        2 => "roster-pack",
        _ => "addon-pack",
    }
}

fn fallback_pack_tag(pack_type: &str) -> String {
    match pack_type {
        "build" | "build-pack" => "build",
        "roster" | "roster-pack" => "roster",
        _ => "addon",
    }
    .to_string()
}

fn pack_updated_label(created_at: &str, updated_at: &str) -> String {
    let (prefix, source) = if !updated_at.trim().is_empty() && updated_at != created_at {
        ("Updated", updated_at)
    } else {
        ("Created", created_at)
    };

    pack_iso_date_label(source)
        .map(|date| format!("{prefix} {date}"))
        .unwrap_or_default()
}

fn installed_pack_date_label(installed_at: &str) -> String {
    pack_iso_date_label(installed_at)
        .map(|date| format!("Installed {date}"))
        .unwrap_or_else(|| "Installed locally".to_string())
}

fn pack_iso_date_label(value: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let date = value.get(0..10)?;
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<usize>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    let month_name = MONTHS.get(month.saturating_sub(1))?;
    Some(format!("{month_name} {day}, {year}"))
}

fn apply_initial_native_settings(ui: &KalpaWindow) {
    apply_native_settings(ui, &read_native_settings());
    ui.set_settings_addons_path(configured_addons_path_display().into());
}

fn apply_native_settings(ui: &KalpaWindow, settings: &NativeSettings) {
    ui.set_settings_auto_update(settings.auto_update);
    ui.set_settings_warn_eso_running(settings.warn_eso_running);
    ui.set_settings_native_performance_mode(settings.native_performance_mode);
    ui.set_settings_official_uploader(settings.official_uploader);
    ui.set_settings_auto_open_analysis(settings.auto_open_analysis);
    ui.set_settings_conflict_policy(settings.conflict_policy.clamp(0, 2));
    ui.set_uploader_region(settings.uploader_region.clamp(1, 2));
    ui.set_uploader_visibility(settings.uploader_visibility.clamp(0, 2));
}

fn native_settings_from_ui(ui: &KalpaWindow) -> NativeSettings {
    NativeSettings {
        auto_update: ui.get_settings_auto_update(),
        warn_eso_running: ui.get_settings_warn_eso_running(),
        native_performance_mode: ui.get_settings_native_performance_mode(),
        official_uploader: ui.get_settings_official_uploader(),
        auto_open_analysis: ui.get_settings_auto_open_analysis(),
        conflict_policy: ui.get_settings_conflict_policy().clamp(0, 2),
        uploader_region: ui.get_uploader_region().clamp(1, 2),
        uploader_visibility: ui.get_uploader_visibility().clamp(0, 2),
    }
}

fn current_app_version() -> &'static str {
    static CURRENT: OnceLock<String> = OnceLock::new();
    CURRENT
        .get_or_init(|| {
            serde_json::from_str::<serde_json::Value>(TAURI_CONF_JSON)
                .ok()
                .and_then(|value| {
                    value
                        .get("version")
                        .and_then(|version| version.as_str())
                        .map(str::to_string)
                })
                .filter(|version| !version.trim().is_empty())
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
        })
        .as_str()
}

fn normalize_app_version(value: &str) -> &str {
    value.trim().trim_start_matches(['v', 'V'])
}

fn split_app_version(value: &str) -> (&str, Option<&str>) {
    let value = normalize_app_version(value)
        .split_once('+')
        .map(|(base, _)| base)
        .unwrap_or_else(|| normalize_app_version(value));
    value
        .split_once('-')
        .map(|(core, pre)| (core, Some(pre)))
        .unwrap_or((value, None))
}

fn compare_numeric_identifier(left: &str, right: &str) -> CmpOrdering {
    let left_number = left.parse::<u64>();
    let right_number = right.parse::<u64>();
    match (left_number, right_number) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        (Ok(_), Err(_)) => CmpOrdering::Less,
        (Err(_), Ok(_)) => CmpOrdering::Greater,
        (Err(_), Err(_)) => left.cmp(right),
    }
}

fn compare_prerelease(left: &str, right: &str) -> CmpOrdering {
    let mut left_parts = left.split('.');
    let mut right_parts = right.split('.');
    loop {
        match (left_parts.next(), right_parts.next()) {
            (Some(left), Some(right)) => {
                let ordering = compare_numeric_identifier(left, right);
                if ordering != CmpOrdering::Equal {
                    return ordering;
                }
            }
            (Some(_), None) => return CmpOrdering::Greater,
            (None, Some(_)) => return CmpOrdering::Less,
            (None, None) => return CmpOrdering::Equal,
        }
    }
}

fn compare_app_versions(left: &str, right: &str) -> CmpOrdering {
    let (left_core, left_pre) = split_app_version(left);
    let (right_core, right_pre) = split_app_version(right);
    let mut left_core_parts = left_core.split('.');
    let mut right_core_parts = right_core.split('.');
    loop {
        match (left_core_parts.next(), right_core_parts.next()) {
            (Some(left), Some(right)) => {
                let ordering = left
                    .parse::<u64>()
                    .unwrap_or_default()
                    .cmp(&right.parse::<u64>().unwrap_or_default());
                if ordering != CmpOrdering::Equal {
                    return ordering;
                }
            }
            (Some(left), None) => {
                let ordering = left.parse::<u64>().unwrap_or_default().cmp(&0);
                if ordering != CmpOrdering::Equal {
                    return ordering;
                }
            }
            (None, Some(right)) => {
                let ordering = 0.cmp(&right.parse::<u64>().unwrap_or_default());
                if ordering != CmpOrdering::Equal {
                    return ordering;
                }
            }
            (None, None) => break,
        }
    }

    match (left_pre, right_pre) {
        (None, None) => CmpOrdering::Equal,
        (None, Some(_)) => CmpOrdering::Greater,
        (Some(_), None) => CmpOrdering::Less,
        (Some(left), Some(right)) => compare_prerelease(left, right),
    }
}

fn app_version_is_newer(remote: &str, current: &str) -> bool {
    compare_app_versions(remote, current) == CmpOrdering::Greater
}

fn native_app_update_platform_keys() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["windows-x86_64-nsis", "windows-x86_64"]
    }
    #[cfg(target_os = "macos")]
    {
        &["darwin-x86_64", "darwin-aarch64"]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &["linux-x86_64", "linux-x86_64-appimage"]
    }
}

fn native_app_update_info_from_manifest(
    manifest: NativeAppUpdateManifest,
    current_version: &str,
) -> Option<NativeAppUpdateInfo> {
    if !app_version_is_newer(&manifest.version, current_version) {
        return None;
    }

    native_app_update_platform_keys()
        .iter()
        .find_map(|key| manifest.platforms.get(*key))
        .map(|platform| NativeAppUpdateInfo {
            version: manifest.version,
            url: platform.url.clone(),
            signature: platform.signature.clone(),
        })
}

fn fetch_native_app_update_info() -> Result<Option<NativeAppUpdateInfo>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("Kalpa/{} native-slint", current_app_version()))
        .build()
        .map_err(|error| format!("Failed to build update client: {error}"))?;
    let manifest = client
        .get(APP_UPDATE_MANIFEST_URL)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Failed to fetch update manifest: {error}"))?
        .json::<NativeAppUpdateManifest>()
        .map_err(|error| format!("Failed to parse update manifest: {error}"))?;
    Ok(native_app_update_info_from_manifest(
        manifest,
        current_app_version(),
    ))
}

fn set_app_update_banner(ui: &KalpaWindow, message: &str, action_label: &str, action_kind: i32) {
    ui.set_app_update_message(message.into());
    ui.set_app_update_action_label(action_label.into());
    ui.set_app_update_action_kind(action_kind);
}

fn start_native_app_update_check(ui_weak: slint::Weak<KalpaWindow>, silent: bool) {
    if !silent {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_settings_open(false);
            set_app_update_banner(&ui, "Checking for Kalpa updates...", "", 0);
        }
    }

    std::thread::spawn(move || {
        let result = fetch_native_app_update_info();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            match result {
                Ok(Some(update)) => {
                    debug_assert!(!update.url.is_empty());
                    debug_assert!(!update.signature.is_empty());
                    set_app_update_banner(
                        &ui,
                        &format!(
                            "Version {} available - open the signed updater to install.",
                            update.version
                        ),
                        "Install",
                        1,
                    );
                }
                Ok(None) if !silent => {
                    set_app_update_banner(
                        &ui,
                        &format!("Kalpa is up to date ({}).", current_app_version()),
                        "Dismiss",
                        2,
                    );
                }
                Ok(None) => {}
                Err(error) if !silent => {
                    set_app_update_banner(
                        &ui,
                        &format!("Update check failed: {error}"),
                        "Dismiss",
                        2,
                    );
                }
                Err(_) => {}
            }
        });
    });
}

const UPLOADER_ACTIVE_WINDOW_SECS: u64 = 90;
const UPLOADER_FIGHT_PREVIEW_LIMIT: usize = 4;

fn native_uploader_logs_dir() -> (Option<PathBuf>, String) {
    let addons_root = configured_addons_path().or_else(default_addons_root);
    if let Some(addons_root) = addons_root {
        if let Some(parent) = addons_root.parent() {
            let logs = parent.join("Logs");
            if logs.is_dir() {
                return (
                    Some(logs),
                    "Log directory found next to the configured AddOns folder.".to_string(),
                );
            }
            return (
                Some(logs),
                "Expected the ESO Logs folder next to AddOns, but it does not exist yet. Enable /encounterlog in-game to create it.".to_string(),
            );
        }
    }

    (
        None,
        "Configure the ESO AddOns folder so Kalpa can find the sibling Logs folder.".to_string(),
    )
}

fn list_native_uploader_logs(logs_dir: &Path) -> Result<Vec<NativeUploaderLog>, String> {
    if !logs_dir.is_dir() {
        return Ok(Vec::new());
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let mut logs = Vec::new();
    for entry in fs::read_dir(logs_dir).map_err(|error| format!("Failed to read Logs: {error}"))? {
        let entry = entry.map_err(|error| format!("Failed to enumerate Logs: {error}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_log = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("log"))
            .unwrap_or(false);
        if !is_log || is_noncombat_log_name(&path) {
            continue;
        }

        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified_epoch = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let active = modified_epoch > 0
            && modified_epoch <= now
            && now.saturating_sub(modified_epoch) <= UPLOADER_ACTIVE_WINDOW_SECS;

        logs.push(NativeUploaderLog {
            file_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Encounter.log")
                .to_string(),
            path,
            size_bytes: metadata.len(),
            modified_epoch,
            active,
        });
    }

    logs.sort_by(|left, right| {
        right
            .modified_epoch
            .cmp(&left.modified_epoch)
            .then_with(|| left.file_name.cmp(&right.file_name))
    });
    Ok(logs)
}

fn is_noncombat_log_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.eq_ignore_ascii_case("Interface.log") || name.eq_ignore_ascii_case("client.log")
        })
        .unwrap_or(false)
}

fn selected_uploader_path(ui: &KalpaWindow) -> Option<String> {
    (0..ui.get_uploader_logs().row_count())
        .filter_map(|index| ui.get_uploader_logs().row_data(index))
        .find(|entry| entry.selected)
        .map(|entry| entry.path.to_string())
}

fn uploader_log_entry(log: &NativeUploaderLog, selected_path: Option<&Path>) -> UploaderLogEntry {
    let selected = selected_path
        .map(|selected| selected == log.path.as_path())
        .unwrap_or(false);
    UploaderLogEntry {
        title: log.file_name.clone().into(),
        meta: uploader_log_meta(log).into(),
        path: log.path.to_string_lossy().into_owned().into(),
        selected,
        active: log.active,
    }
}

fn uploader_log_meta(log: &NativeUploaderLog) -> String {
    let date = if log.modified_epoch == 0 {
        "Unknown date".to_string()
    } else {
        format_short_date(log.modified_epoch)
    };
    let active = if log.active { " - active" } else { "" };
    format!("{} - {}{}", date, format_size(log.size_bytes), active)
}

fn apply_uploader_log_model(
    ui: &KalpaWindow,
    logs: Vec<NativeUploaderLog>,
    logs_summary: String,
    selected_path: Option<PathBuf>,
) {
    let selected_path = selected_path.or_else(|| logs.first().map(|log| log.path.clone()));
    let entries = logs
        .iter()
        .map(|log| uploader_log_entry(log, selected_path.as_deref()))
        .collect::<Vec<_>>();
    ui.set_uploader_logs(Rc::new(VecModel::from(entries)).into());
    ui.set_uploader_logs_summary(logs_summary.into());

    if let Some(selected) = logs
        .iter()
        .find(|log| Some(log.path.as_path()) == selected_path.as_deref())
    {
        ui.set_uploader_selected_log_label(format_size(selected.size_bytes).into());
        ui.set_uploader_live_log_label(selected.file_name.clone().into());
        ui.set_uploader_status_title("Scanning selected log...".into());
        ui.set_uploader_status_detail(
            "Reading combat boundaries without loading the whole file.".into(),
        );
        ui.set_uploader_fight_count_label("Scanning".into());
        ui.set_uploader_fights(Rc::new(VecModel::from(Vec::<UploaderFightEntry>::new())).into());
    } else {
        ui.set_uploader_selected_log_label("No log selected".into());
        ui.set_uploader_live_log_label("No Encounter.log".into());
        ui.set_uploader_status_title("No log selected".into());
        ui.set_uploader_status_detail(
            "Select a log file to inspect fights before uploading.".into(),
        );
        ui.set_uploader_fight_count_label("0 ready".into());
        ui.set_uploader_fights(Rc::new(VecModel::from(Vec::<UploaderFightEntry>::new())).into());
    }
}

fn refresh_native_uploader(ui: &KalpaWindow, preflight_counter: Arc<AtomicU64>) {
    let selected = selected_uploader_path(ui).map(PathBuf::from);
    let (logs_dir, message) = native_uploader_logs_dir();
    let logs = logs_dir
        .as_deref()
        .map(list_native_uploader_logs)
        .unwrap_or_else(|| Ok(Vec::new()));
    match logs {
        Ok(logs) => {
            let summary = if logs.is_empty() {
                message
            } else {
                format!(
                    "{} log file{} found.",
                    logs.len(),
                    if logs.len() == 1 { "" } else { "s" }
                )
            };
            apply_uploader_log_model(ui, logs, summary, selected);
            request_native_uploader_preflight(ui, preflight_counter);
        }
        Err(error) => {
            ui.set_uploader_logs(Rc::new(VecModel::from(Vec::<UploaderLogEntry>::new())).into());
            ui.set_uploader_fights(
                Rc::new(VecModel::from(Vec::<UploaderFightEntry>::new())).into(),
            );
            ui.set_uploader_logs_summary(error.clone().into());
            ui.set_uploader_status_title("Could not read Logs".into());
            ui.set_uploader_status_detail(error.into());
            ui.set_uploader_selected_log_label("No log selected".into());
            ui.set_uploader_fight_count_label("0 ready".into());
        }
    }
}

fn request_native_uploader_preflight(ui: &KalpaWindow, preflight_counter: Arc<AtomicU64>) {
    let Some(path) = selected_uploader_path(ui) else {
        return;
    };
    let sequence = preflight_counter.fetch_add(1, Ordering::Relaxed) + 1;
    let ui_weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = scan_native_uploader_log(Path::new(&path));
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            if preflight_counter.load(Ordering::Relaxed) != sequence {
                return;
            }
            if selected_uploader_path(&ui).as_deref() != Some(path.as_str()) {
                return;
            }
            apply_native_uploader_preflight(&ui, result);
        });
    });
}

fn scan_native_uploader_log(path: &Path) -> Result<NativeUploaderPreflight, String> {
    let file = fs::File::open(path).map_err(|error| format!("Failed to open log: {error}"))?;
    let reader = BufReader::new(file);
    let mut preflight = NativeUploaderPreflight::default();
    let mut in_fight: Option<(usize, u64)> = None;

    for line in reader.lines() {
        let line = line.map_err(|error| format!("Failed to read log: {error}"))?;
        match uploader_line_type(&line) {
            "BEGIN_LOG" => preflight.sessions += 1,
            "BEGIN_COMBAT" => {
                if in_fight.is_none() {
                    in_fight = Some((preflight.total_fights, uploader_line_ms(&line)));
                }
            }
            "END_COMBAT" => {
                if let Some((index, start_ms)) = in_fight.take() {
                    let end_ms = uploader_line_ms(&line).max(start_ms);
                    preflight.total_fights += 1;
                    if preflight.fights.len() < UPLOADER_FIGHT_PREVIEW_LIMIT {
                        preflight.fights.push(NativeUploaderFight {
                            index,
                            start_ms,
                            end_ms,
                        });
                    } else {
                        preflight.truncated = true;
                    }
                }
            }
            _ => {}
        }
    }

    Ok(preflight)
}

fn uploader_line_type(line: &str) -> &str {
    line.split(',').nth(1).map(str::trim).unwrap_or("")
}

fn uploader_line_ms(line: &str) -> u64 {
    line.split(',')
        .next()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

fn apply_native_uploader_preflight(
    ui: &KalpaWindow,
    result: Result<NativeUploaderPreflight, String>,
) {
    match result {
        Ok(preflight) => {
            let fights = preflight
                .fights
                .iter()
                .map(uploader_fight_entry)
                .collect::<Vec<_>>();
            let fight_label = match preflight.total_fights {
                0 => "0 ready".to_string(),
                1 => "1 ready".to_string(),
                count => format!("{count} ready"),
            };
            let session_label = match preflight.sessions {
                0 => "no logging sessions".to_string(),
                1 => "1 logging session".to_string(),
                count => format!("{count} logging sessions"),
            };
            let detail = if preflight.total_fights == 0 {
                format!(
                    "No completed fights found in {session_label}. Live mode can still watch for new fights."
                )
            } else {
                format!(
                    "{} completed fight{} across {session_label}. Upload launches the external ESO Logs uploader from the native Slint shell, so Kalpa does not reopen the WebView for this flow.",
                    preflight.total_fights,
                    if preflight.total_fights == 1 { "" } else { "s" }
                )
            };
            ui.set_uploader_fights(Rc::new(VecModel::from(fights)).into());
            ui.set_uploader_fight_count_label(fight_label.clone().into());
            ui.set_uploader_live_fight_label(fight_label.into());
            ui.set_uploader_status_title(
                if preflight.total_fights == 0 {
                    "No completed fights detected"
                } else {
                    "Ready to upload"
                }
                .into(),
            );
            ui.set_uploader_status_detail(detail.into());
        }
        Err(error) => {
            ui.set_uploader_fights(
                Rc::new(VecModel::from(Vec::<UploaderFightEntry>::new())).into(),
            );
            ui.set_uploader_fight_count_label("0 ready".into());
            ui.set_uploader_status_title("Could not scan log".into());
            ui.set_uploader_status_detail(error.into());
        }
    }
}

fn uploader_fight_entry(fight: &NativeUploaderFight) -> UploaderFightEntry {
    UploaderFightEntry {
        title: format!("Fight {}", fight.index + 1).into(),
        meta: format!(
            "{} start - {}",
            format_relative_ms(fight.start_ms),
            format_duration_ms(fight.end_ms.saturating_sub(fight.start_ms))
        )
        .into(),
        result: "READY".into(),
        live: false,
    }
}

fn format_relative_ms(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn format_duration_ms(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

const OFFICIAL_UPLOADER_PRODUCTS: [(&str, &str); 3] = [
    ("Archon App", "Archon App.exe"),
    ("ESO Logs Uploader", "ESO Logs Uploader.exe"),
    ("Archon", "Archon.exe"),
];

#[derive(Clone, Copy)]
struct UploaderLaunchOptions {
    region: u8,
    visibility: u8,
}

fn uploader_launch_options_from_ui(ui: &KalpaWindow) -> UploaderLaunchOptions {
    UploaderLaunchOptions {
        region: match ui.get_uploader_region() {
            2 => 2,
            _ => 1,
        },
        visibility: match ui.get_uploader_visibility() {
            0 => 0,
            1 => 1,
            _ => 2,
        },
    }
}

fn official_uploader_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(var).map(PathBuf::from) {
            roots.push(base.join("Programs"));
            roots.push(base);
        }
    }
    roots
}

fn official_uploader_candidates_from_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    OFFICIAL_UPLOADER_PRODUCTS
        .iter()
        .flat_map(|(dir, exe)| roots.iter().map(move |root| root.join(dir).join(exe)))
        .collect()
}

fn find_official_uploader() -> Option<PathBuf> {
    official_uploader_candidates_from_roots(&official_uploader_roots())
        .into_iter()
        .find(|path| path.is_file())
}

fn launch_external_uploader(
    log_path: &Path,
    live: bool,
    options: UploaderLaunchOptions,
) -> Result<String, String> {
    if !log_path.is_file() {
        return Err("Selected log file was not found.".to_string());
    }

    if let Some(exe) = find_official_uploader() {
        let mut command = std::process::Command::new(&exe);
        if live {
            let Some(log_dir) = log_path.parent() else {
                return Err("Could not resolve the log folder for live logging.".to_string());
            };
            command
                .arg("--operation-name")
                .arg("liveLog")
                .arg("--directory-path")
                .arg(log_dir)
                .arg("--region")
                .arg(options.region.to_string())
                .arg("--guild")
                .arg("null")
                .arg("--report-visibility")
                .arg(options.visibility.to_string())
                .arg("--enable-real-time-uploading");
        } else {
            command
                .arg("--operation-name")
                .arg("uploadALog")
                .arg("--file-path")
                .arg(log_path)
                .arg("--region")
                .arg(options.region.to_string())
                .arg("--guild")
                .arg("null")
                .arg("--report-visibility")
                .arg(options.visibility.to_string());
        }

        command
            .spawn()
            .map_err(|error| format!("Failed to launch the external ESO Logs uploader: {error}"))?;
        return Ok(if live {
            "Live logging started in the external ESO Logs uploader. Kalpa stayed in the native Slint shell.".to_string()
        } else {
            "Opened the external ESO Logs uploader with the selected log. Kalpa stayed in the native Slint shell.".to_string()
        });
    }

    if live {
        open_url("https://www.esologs.com/client/download");
        Err(
            "The Archon App / ESO Logs uploader is not installed, so live logging cannot start yet. Opened the download page."
                .to_string(),
        )
    } else {
        if let Some(parent) = log_path.parent() {
            open_path(parent);
        }
        open_url("https://www.esologs.com/client/download");
        Ok(
            "The Archon App / ESO Logs uploader is not installed. Opened the download page and the Logs folder."
                .to_string(),
        )
    }
}

fn wire_uploader_actions(ui: &KalpaWindow) {
    let preflight_counter = Arc::new(AtomicU64::new(0));

    let open_ui = ui.as_weak();
    let open_counter = preflight_counter.clone();
    ui.on_open_uploader(move || {
        let Some(ui) = open_ui.upgrade() else {
            return;
        };
        ui.set_uploader_route_label(
            if find_official_uploader().is_some() {
                "Archon App handoff"
            } else {
                "Install uploader"
            }
            .into(),
        );
        ui.set_uploader_live_status_label("Ready".into());
        ui.set_uploader_live_detail(
            "Upload and Go Live launch the external ESO Logs uploader directly, without reopening the WebView shell."
                .into(),
        );
        refresh_native_uploader(&ui, open_counter.clone());
    });

    let refresh_ui = ui.as_weak();
    let refresh_counter = preflight_counter.clone();
    ui.on_uploader_refresh(move || {
        if let Some(ui) = refresh_ui.upgrade() {
            refresh_native_uploader(&ui, refresh_counter.clone());
        }
    });

    let select_ui = ui.as_weak();
    let select_counter = preflight_counter.clone();
    ui.on_uploader_select_log(move |index| {
        let Some(ui) = select_ui.upgrade() else {
            return;
        };
        let index = index.max(0) as usize;
        let rows = (0..ui.get_uploader_logs().row_count())
            .filter_map(|row| ui.get_uploader_logs().row_data(row))
            .enumerate()
            .map(|(row, mut entry)| {
                entry.selected = row == index;
                entry
            })
            .collect::<Vec<_>>();
        ui.set_uploader_logs(Rc::new(VecModel::from(rows)).into());
        ui.set_uploader_status_title("Scanning selected log...".into());
        ui.set_uploader_status_detail(
            "Reading combat boundaries without loading the whole file.".into(),
        );
        ui.set_uploader_fight_count_label("Scanning".into());
        request_native_uploader_preflight(&ui, select_counter.clone());
    });

    let upload_ui = ui.as_weak();
    ui.on_uploader_upload(move || {
        let Some(ui) = upload_ui.upgrade() else {
            return;
        };
        let Some(path) = selected_uploader_path(&ui) else {
            ui.set_status_error_message("Select a log before uploading.".into());
            return;
        };
        ui.set_uploader_view(1);
        ui.set_uploader_status_title("Opening uploader...".into());
        ui.set_uploader_status_detail(
            "Launching the external ESO Logs uploader from the native Slint shell.".into(),
        );
        let options = uploader_launch_options_from_ui(&ui);
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = launch_external_uploader(Path::new(&path), false, options);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                ui.set_uploader_view(0);
                match result {
                    Ok(detail) => {
                        ui.set_uploader_status_title("Uploader opened".into());
                        ui.set_uploader_status_detail(detail.into());
                    }
                    Err(error) => {
                        ui.set_uploader_status_title("Upload could not start".into());
                        ui.set_uploader_status_detail(error.clone().into());
                        ui.set_status_error_message(error.into());
                    }
                }
            });
        });
    });

    let live_ui = ui.as_weak();
    ui.on_uploader_live_toggle(move || {
        let Some(ui) = live_ui.upgrade() else {
            return;
        };
        if ui.get_uploader_view() == 3 {
            ui.set_uploader_view(2);
            ui.set_uploader_live_status_label("Ready".into());
            ui.set_uploader_live_detail("Live handoff stopped in Kalpa. If the external uploader is still streaming, stop it there too.".into());
            return;
        }
        let Some(path) = selected_uploader_path(&ui) else {
            ui.set_status_error_message(
                "Select Encounter.log or refresh after enabling /encounterlog.".into(),
            );
            return;
        };
        ui.set_uploader_view(3);
        ui.set_uploader_live_status_label("Starting".into());
        ui.set_uploader_live_detail(
            "Launching the external ESO Logs live uploader without leaving the native shell."
                .into(),
        );
        let options = uploader_launch_options_from_ui(&ui);
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = launch_external_uploader(Path::new(&path), true, options);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                match result {
                    Ok(detail) => {
                        ui.set_uploader_view(3);
                        ui.set_uploader_live_status_label("Live handoff".into());
                        ui.set_uploader_live_detail(detail.into());
                    }
                    Err(error) => {
                        ui.set_uploader_view(2);
                        ui.set_uploader_live_status_label("Ready".into());
                        ui.set_uploader_live_detail(error.clone().into());
                        ui.set_status_error_message(error.into());
                    }
                }
            });
        });
    });

    let options_ui = ui.as_weak();
    ui.on_uploader_options_changed(move || {
        let Some(ui) = options_ui.upgrade() else {
            return;
        };
        persist_native_settings(&native_settings_from_ui(&ui));
    });
}

fn wire_settings_actions(ui: &KalpaWindow, models: AddonModels) {
    let settings_ui = ui.as_weak();
    ui.on_settings_changed(move || {
        let Some(ui) = settings_ui.upgrade() else {
            return;
        };
        persist_native_settings(&native_settings_from_ui(&ui));
    });

    let performance_ui = ui.as_weak();
    ui.on_settings_performance_mode_changed(move |native_mode| {
        let Some(ui) = performance_ui.upgrade() else {
            return;
        };
        persist_native_settings(&native_settings_from_ui(&ui));
        if native_mode {
            return;
        }

        match return_to_webview_shell(false, false, false, None) {
            Ok(()) => {
                let _ = slint::quit_event_loop();
            }
            Err(error) => {
                ui.set_settings_native_performance_mode(true);
                persist_native_settings(&native_settings_from_ui(&ui));
                ui.set_status_error_message(format!("Failed to return to WebView: {error}").into());
            }
        }
    });

    let open_addons_path_ui = ui.as_weak();
    ui.on_settings_addons_path_opened(move || {
        let Some(ui) = open_addons_path_ui.upgrade() else {
            return;
        };
        let path = ui.get_settings_addons_path().to_string();
        let path = PathBuf::from(path.trim());
        if path.is_dir() {
            open_path(&path);
        } else {
            ui.set_status_error_message("Configured AddOns folder was not found.".into());
        }
    });

    let redetect_addons_path_ui = ui.as_weak();
    ui.on_settings_addons_path_redetected(move || {
        let Some(ui) = redetect_addons_path_ui.upgrade() else {
            return;
        };
        if let Some(path) = default_addons_root().filter(|path| path.is_dir()) {
            ui.set_settings_addons_path(path.to_string_lossy().into_owned().into());
            ui.set_status_error_message(
                "Found the default ESO AddOns folder. Click Apply to save it.".into(),
            );
        } else {
            ui.set_status_error_message("No default ESO AddOns folder was found.".into());
        }
    });

    let apply_addons_path_ui = ui.as_weak();
    ui.on_settings_addons_path_applied(move |path| {
        let Some(ui) = apply_addons_path_ui.upgrade() else {
            return;
        };
        let path = path.trim();
        if path.is_empty() {
            ui.set_status_error_message("Choose an AddOns folder before applying.".into());
            return;
        }

        let path_buf = PathBuf::from(path);
        if !path_buf.is_dir() {
            ui.set_status_error_message("AddOns folder was not found.".into());
            return;
        }

        match persist_addons_path(path) {
            Ok(()) => match reload_real_addon_models(&ui, &models) {
                Ok(()) => {
                    ui.set_status_error_message("Saved AddOns folder and loaded addons.".into())
                }
                Err(error) => {
                    ui.set_status_error_message(format!("Saved AddOns folder, but {error}").into())
                }
            },
            Err(error) => {
                ui.set_status_error_message(format!("Failed to save AddOns folder: {error}").into())
            }
        }
    });

    let api_compat_ui = ui.as_weak();
    ui.on_settings_api_compat(move || {
        let Some(ui) = api_compat_ui.upgrade() else {
            return;
        };
        let Some(addons_dir) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before checking API compatibility.".into(),
            );
            return;
        };

        ui.set_status_error_message("Checking addon API compatibility...".into());
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = check_native_api_compatibility(&addons_dir);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                match result {
                    Ok(info) => ui.set_status_error_message(api_compat_summary(&info).into()),
                    Err(error) => ui.set_status_error_message(
                        format!("API compatibility check failed: {error}").into(),
                    ),
                }
            });
        });
    });

    let app_update_ui = ui.as_weak();
    ui.on_settings_app_update(move || {
        let Some(ui) = app_update_ui.upgrade() else {
            return;
        };
        start_native_app_update_check(ui.as_weak(), false);
    });

    let app_update_action_ui = ui.as_weak();
    ui.on_app_update_action(move || {
        let Some(ui) = app_update_action_ui.upgrade() else {
            return;
        };

        match ui.get_app_update_action_kind() {
            1 => match return_to_webview_shell(true, false, false, None) {
                Ok(()) => {
                    set_app_update_banner(&ui, "Opening the signed updater...", "", 0);
                    let _ = slint::quit_event_loop();
                }
                Err(error) => {
                    ui.set_status_error_message(
                        format!("Failed to open signed updater: {error}").into(),
                    );
                }
            },
            2 => set_app_update_banner(&ui, "", "", 0),
            _ => {}
        }
    });

    let migration_ui = ui.as_weak();
    ui.on_settings_minion_migration(move || {
        let Some(ui) = migration_ui.upgrade() else {
            return;
        };
        ui.set_migration_open(true);
        apply_migration_preconditions(&ui);
    });

    let safety_ui = ui.as_weak();
    ui.on_settings_safety_center(move || {
        let Some(ui) = safety_ui.upgrade() else {
            return;
        };
        ui.set_safety_open(true);
        apply_safety_center_model(&ui);
    });

    let export_ui = ui.as_weak();
    ui.on_settings_addon_list_export(move || {
        let Some(ui) = export_ui.upgrade() else {
            return;
        };
        let Some(addons_dir) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before exporting the addon list.".into(),
            );
            return;
        };

        match export_addon_list_json(&addons_dir).and_then(write_clipboard_text) {
            Ok(()) => ui.set_status_error_message("Addon list copied to clipboard.".into()),
            Err(error) => {
                ui.set_status_error_message(format!("Addon list export failed: {error}").into())
            }
        }
    });

    let import_ui = ui.as_weak();
    ui.on_settings_addon_list_import(move || {
        let Some(ui) = import_ui.upgrade() else {
            return;
        };
        let Some(addons_dir) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before importing an addon list.".into(),
            );
            return;
        };

        let json_data = match read_clipboard_text() {
            Ok(text) if !text.trim().is_empty() => text,
            Ok(_) => {
                ui.set_status_error_message(
                    "Clipboard is empty. Copy an addon-list export first.".into(),
                );
                return;
            }
            Err(error) => {
                ui.set_status_error_message(format!("Addon list import failed: {error}").into());
                return;
            }
        };

        ui.set_status_error_message("Importing addon list from clipboard...".into());
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = import_addon_list_json(&addons_dir, &json_data);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                match result {
                    Ok(result) => {
                        ui.invoke_refresh_requested();
                        ui.set_status_error_message(import_result_summary(&result).into());
                    }
                    Err(error) => ui.set_status_error_message(
                        format!("Addon list import failed: {error}").into(),
                    ),
                }
            });
        });
    });
}

fn reload_real_addon_models(ui: &KalpaWindow, models: &AddonModels) -> Result<(), String> {
    let addons_root = addons_source_root().ok_or_else(|| {
        "AddOns folder was not found. Set KALPA_ADDONS_PATH or configure the ESO AddOns path."
            .to_string()
    })?;
    clear_file_entry_cache();
    let addons = real_addon_entries(&addons_root)?;
    if addons.is_empty() {
        return Err("No addons were found in the configured AddOns folder.".into());
    }

    *models.all.borrow_mut() = addons;
    apply_addon_view(ui, models);
    apply_saved_variables_model(ui, &models.all.borrow());
    refresh_file_browser(ui);
    Ok(())
}

fn wire_backup_restore_actions(ui: &KalpaWindow) {
    let open_ui = ui.as_weak();
    ui.on_open_backup_restore(move || {
        let Some(ui) = open_ui.upgrade() else {
            return;
        };
        apply_backup_restore_model(&ui);
    });

    let refresh_ui = ui.as_weak();
    ui.on_backup_restore_refresh(move || {
        let Some(ui) = refresh_ui.upgrade() else {
            return;
        };
        apply_backup_restore_model(&ui);
    });

    let create_ui = ui.as_weak();
    ui.on_backup_restore_create(move |label| {
        let Some(ui) = create_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = addons_source_root() else {
            ui.set_status_error_message(
                "AddOns folder was not found. Configure it before creating a backup.".into(),
            );
            return;
        };
        match create_settings_backup(&addons_root, label.as_str()) {
            Ok(summary) => {
                ui.set_status_error_message(format!("Backup saved - {summary}.").into());
                ui.set_backup_label_draft("".into());
                ui.set_backup_restore_view(0);
                apply_backup_restore_model(&ui);
            }
            Err(error) => ui.set_status_error_message(format!("Backup failed: {error}").into()),
        }
    });

    let restore_ui = ui.as_weak();
    ui.on_backup_restore_restore(move |index| {
        let Some(ui) = restore_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = addons_source_root() else {
            ui.set_status_error_message(
                "AddOns folder was not found. Configure it before restoring a backup.".into(),
            );
            return;
        };
        let backups = ui.get_settings_backups();
        let Some(backup) = backups.row_data(index.max(0) as usize) else {
            ui.set_status_error_message("Choose a backup to restore.".into());
            return;
        };
        match restore_settings_backup(&addons_root, backup.name.as_str()) {
            Ok(summary) => {
                ui.set_backup_restore_view(0);
                ui.set_status_error_message(format!("Restored backup - {summary}.").into());
                apply_backup_restore_model(&ui);
                if let Some(addons_root) = addons_source_root() {
                    if let Ok(addons) = real_addon_entries(&addons_root) {
                        apply_saved_variables_model(&ui, &addons);
                    }
                }
            }
            Err(error) => ui.set_status_error_message(format!("Restore failed: {error}").into()),
        }
    });

    let delete_ui = ui.as_weak();
    ui.on_backup_restore_delete(move |index| {
        let Some(ui) = delete_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = addons_source_root() else {
            ui.set_status_error_message(
                "AddOns folder was not found. Configure it before deleting a backup.".into(),
            );
            return;
        };
        let backups = ui.get_settings_backups();
        let Some(backup) = backups.row_data(index.max(0) as usize) else {
            ui.set_status_error_message("Choose a backup to delete.".into());
            return;
        };
        match delete_settings_backup(&addons_root, backup.name.as_str()) {
            Ok(()) => {
                ui.set_status_error_message("Backup deleted.".into());
                apply_backup_restore_model(&ui);
            }
            Err(error) => ui.set_status_error_message(format!("Delete failed: {error}").into()),
        }
    });

    let reveal_ui = ui.as_weak();
    ui.on_backup_restore_reveal_folder(move || {
        let Some(ui) = reveal_ui.upgrade() else {
            return;
        };
        if let Some(addons_root) = addons_source_root() {
            let path = settings_backups_dir(&addons_root);
            if let Err(error) = fs::create_dir_all(&path) {
                ui.set_status_error_message(
                    format!("Failed to create backups folder: {error}").into(),
                );
                return;
            }
            open_path(&path);
        } else {
            ui.set_status_error_message(
                "AddOns folder was not found. Configure it before opening backups.".into(),
            );
        }
    });
}

fn wire_character_actions(ui: &KalpaWindow) {
    let open_ui = ui.as_weak();
    ui.on_open_characters(move || {
        let Some(ui) = open_ui.upgrade() else {
            return;
        };
        apply_character_roster_model(&ui);
    });

    let refresh_ui = ui.as_weak();
    ui.on_characters_refresh(move || {
        let Some(ui) = refresh_ui.upgrade() else {
            return;
        };
        apply_character_roster_model(&ui);
    });

    let backup_ui = ui.as_weak();
    ui.on_character_backup(move |index, label| {
        let Some(ui) = backup_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = addons_source_root() else {
            ui.set_status_error_message(
                "AddOns folder was not found. Configure it before backing up a character.".into(),
            );
            return;
        };
        let characters = ui.get_characters();
        let Some(character) = characters.row_data(index.max(0) as usize) else {
            ui.set_status_error_message("Choose a character to back up.".into());
            return;
        };

        match create_character_settings_backup(
            &addons_root,
            character.name.as_str(),
            character.server.as_str(),
            label.as_str(),
        ) {
            Ok(count) => {
                ui.set_status_error_message(
                    format!(
                        "Backed up {}'s settings ({} addon file{}).",
                        character.name.as_str(),
                        count,
                        if count == 1 { "" } else { "s" }
                    )
                    .into(),
                );
                ui.set_character_backup_label_draft("".into());
                apply_backup_restore_model(&ui);
                apply_character_roster_model(&ui);
            }
            Err(error) => {
                ui.set_status_error_message(format!("Character backup failed: {error}").into())
            }
        }
    });
}

fn wire_safety_actions(ui: &KalpaWindow, models: AddonModels) {
    let open_ui = ui.as_weak();
    ui.on_open_safety(move || {
        let Some(ui) = open_ui.upgrade() else {
            return;
        };
        apply_safety_center_model(&ui);
    });

    let refresh_ui = ui.as_weak();
    ui.on_safety_refresh(move || {
        let Some(ui) = refresh_ui.upgrade() else {
            return;
        };
        apply_safety_center_model(&ui);
    });

    let integrity_ui = ui.as_weak();
    ui.on_safety_run_integrity(move || {
        let Some(ui) = integrity_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before running integrity checks.".into(),
            );
            return;
        };
        let result = safe_migration::check_integrity(&addons_root);
        apply_safety_integrity_result(&ui, result);
    });

    let restore_ui = ui.as_weak();
    let restore_models = models.clone();
    ui.on_safety_restore_snapshot(move |index| {
        let Some(ui) = restore_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before restoring snapshots.".into(),
            );
            return;
        };
        let snapshots = ui.get_safety_snapshots();
        let Some(snapshot) = snapshots.row_data(index.max(0) as usize) else {
            ui.set_status_error_message("Choose a snapshot to restore.".into());
            return;
        };
        match safe_migration::restore_snapshot(&addons_root, snapshot.id.as_str()) {
            Ok(count) => {
                ui.set_status_error_message(
                    format!("Restored {count} files from snapshot.").into(),
                );
                let _ = reload_real_addon_models(&ui, &restore_models);
                apply_safety_center_model(&ui);
                apply_backup_restore_model(&ui);
            }
            Err(error) => {
                ui.set_status_error_message(format!("Snapshot restore failed: {error}").into())
            }
        }
    });

    let delete_ui = ui.as_weak();
    ui.on_safety_delete_snapshot(move |index| {
        let Some(ui) = delete_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before deleting snapshots.".into(),
            );
            return;
        };
        let snapshots = ui.get_safety_snapshots();
        let Some(snapshot) = snapshots.row_data(index.max(0) as usize) else {
            ui.set_status_error_message("Choose a snapshot to delete.".into());
            return;
        };
        match safe_migration::delete_snapshot(&addons_root, snapshot.id.as_str()) {
            Ok(()) => {
                ui.set_status_error_message("Snapshot deleted.".into());
                apply_safety_center_model(&ui);
            }
            Err(error) => {
                ui.set_status_error_message(format!("Snapshot delete failed: {error}").into())
            }
        }
    });
}

fn wire_migration_actions(ui: &KalpaWindow, models: AddonModels) {
    let open_ui = ui.as_weak();
    ui.on_open_migration(move || {
        let Some(ui) = open_ui.upgrade() else {
            return;
        };
        apply_migration_preconditions(&ui);
    });

    let recheck_ui = ui.as_weak();
    ui.on_migration_recheck(move || {
        let Some(ui) = recheck_ui.upgrade() else {
            return;
        };
        apply_migration_preconditions(&ui);
    });

    let continue_ui = ui.as_weak();
    ui.on_migration_continue(move || {
        let Some(ui) = continue_ui.upgrade() else {
            return;
        };
        if ui.get_migration_can_proceed() {
            ui.set_migration_phase(1);
            ui.set_migration_status("Create a restore point before previewing the import.".into());
        } else {
            ui.set_status_error_message(
                "Resolve migration precheck blockers before continuing.".into(),
            );
        }
    });

    let snapshot_ui = ui.as_weak();
    ui.on_migration_create_snapshot(move |include_addons| {
        let Some(ui) = snapshot_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before creating a migration snapshot.".into(),
            );
            return;
        };
        match safe_migration::create_pre_migration_snapshot(&addons_root, include_addons) {
            Ok(snapshot) => {
                let summary = format!(
                    "Restore point saved: {} files, {}.",
                    snapshot.file_count,
                    format_size(snapshot.total_size)
                );
                ui.set_migration_snapshot_summary(summary.into());
                apply_safety_center_model(&ui);
                match safe_migration::dry_run_migration(&addons_root) {
                    Ok(dry_run) => {
                        apply_migration_dry_run(&ui, dry_run);
                        ui.set_migration_phase(2);
                    }
                    Err(error) => ui.set_status_error_message(
                        format!("Migration preview failed: {error}").into(),
                    ),
                }
            }
            Err(error) => {
                ui.set_status_error_message(format!("Migration snapshot failed: {error}").into())
            }
        }
    });

    let execute_ui = ui.as_weak();
    let execute_models = models.clone();
    ui.on_migration_execute(move || {
        let Some(ui) = execute_ui.upgrade() else {
            return;
        };
        let Some(addons_root) = configured_addons_path() else {
            ui.set_status_error_message(
                "Configure the ESO AddOns folder before executing migration.".into(),
            );
            return;
        };
        match safe_migration::execute_migration(&addons_root) {
            Ok(result) => {
                ui.set_migration_result_summary(
                    format!(
                        "Imported {} addon{}, {} already tracked, {} missing on disk.",
                        result.imported,
                        if result.imported == 1 { "" } else { "s" },
                        result.already_tracked,
                        result.skipped_missing
                    )
                    .into(),
                );
                ui.set_migration_phase(3);
                ui.set_status_error_message("Minion metadata import complete.".into());
                let _ = reload_real_addon_models(&ui, &execute_models);
                apply_safety_center_model(&ui);
            }
            Err(error) => {
                ui.set_status_error_message(format!("Migration import failed: {error}").into())
            }
        }
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
    apply_theme_selection(
        ui,
        &ThemeSelection::with_skin(draft.colors.clone(), draft.skin_id.as_deref()),
    );
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
    ui.set_draft_skin_id(draft.skin_id.clone().unwrap_or_default().into());
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
    theme.skin_id = normalize_skin_id(theme.skin_id);
    match custom_themes
        .iter()
        .position(|existing| existing.id == theme.id)
    {
        Some(index) => custom_themes[index] = theme,
        None => custom_themes.push(theme),
    }
}

fn parse_imported_custom_theme(json: &str) -> Option<CatalogTheme> {
    let imported = serde_json::from_str::<ImportedCustomTheme>(json).ok()?;
    let colors = normalize_theme_seed(&imported.colors)?;
    let name = imported
        .name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Imported Theme".to_string());
    let description = imported
        .description
        .map(|description| description.trim().to_string())
        .filter(|description| !description.is_empty())
        .unwrap_or_else(|| "Imported custom theme.".to_string());
    Some(CatalogTheme {
        id: new_custom_theme_id(),
        name,
        category: "Custom".to_string(),
        description,
        colors,
        skin_id: normalize_skin_id(imported.skin_id),
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
        draft.description = ui.get_draft_theme_description().trim().to_string();
        if draft.description.is_empty() {
            draft.description = "My custom theme.".to_string();
        }
        draft.category = "Custom".to_string();
        draft.skin_id = normalize_skin_id(draft.skin_id);
        draft.colors = draft_colors_from_ui(&ui, &draft.colors);
        set_theme_draft_color_fields(&ui, &draft.colors);
        set_theme_draft_contrast(&ui, &draft.colors);

        let mut custom_themes = save_custom_themes.borrow_mut();
        upsert_custom_theme(&mut custom_themes, draft.clone());
        persist_custom_themes(&custom_themes);
        set_theme_gallery(&ui, &custom_themes);
        set_theme_draft(&ui, &draft, false);
        *save_draft.borrow_mut() = draft.clone();
        apply_theme_selection(
            &ui,
            &ThemeSelection::with_skin(draft.colors.clone(), draft.skin_id.as_deref()),
        );
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
        let next_name = ui.get_draft_theme_name().trim().to_string();
        let next_description = ui.get_draft_theme_description().trim().to_string();
        let next_skin = normalize_skin_id(Some(ui.get_draft_skin_id().to_string()));
        let description_changed = next_description != draft.description;
        let name_changed = !next_name.is_empty() && next_name != draft.name;
        let skin_changed = next_skin != draft.skin_id;
        if next_colors == draft.colors && !description_changed && !name_changed && !skin_changed {
            return;
        }
        if name_changed {
            draft.name = next_name;
        }
        if description_changed {
            draft.description = next_description;
        }
        draft.colors = next_colors;
        draft.skin_id = next_skin;
        *preview_draft.borrow_mut() = draft.clone();
        ui.set_draft_theme(theme_entry_from_catalog_theme(
            &draft,
            0,
            0,
            0,
            String::new(),
        ));
        set_theme_draft_contrast(&ui, &draft.colors);
        apply_theme_selection(
            &ui,
            &ThemeSelection::with_skin(draft.colors, draft.skin_id.as_deref()),
        );
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

    let export_ui = ui.as_weak();
    let export_custom_themes = custom_themes.clone();
    let export_draft = current_draft.clone();
    ui.on_theme_export(move |theme_id| {
        let theme = if let Some(ui) = export_ui.upgrade() {
            if ui.get_settings_editor_open() {
                let mut draft = export_draft.borrow().clone();
                let normalized_name = ui.get_draft_theme_name().trim().to_string();
                if !normalized_name.is_empty() {
                    draft.name = normalized_name;
                }
                draft.description = ui.get_draft_theme_description().to_string();
                draft.category = "Custom".to_string();
                draft.colors = draft_colors_from_ui(&ui, &draft.colors);
                draft.skin_id = normalize_skin_id(draft.skin_id);
                Some(draft)
            } else {
                let theme_id = theme_id.to_string();
                theme_by_id(&theme_id, &export_custom_themes.borrow())
            }
        } else {
            return;
        };

        let Some(theme) = theme else {
            return;
        };
        let json = match serde_json::to_string_pretty(&theme) {
            Ok(json) => json,
            Err(error) => {
                if let Some(ui) = export_ui.upgrade() {
                    ui.set_status_error_message(format!("Theme export failed: {error}").into());
                }
                return;
            }
        };
        match write_clipboard_text(json) {
            Ok(()) => {
                if let Some(ui) = export_ui.upgrade() {
                    ui.set_status_error_message(
                        format!("Copied theme '{}' to clipboard.", theme.name).into(),
                    );
                }
            }
            Err(error) => {
                if let Some(ui) = export_ui.upgrade() {
                    ui.set_status_error_message(format!("Theme export failed: {error}").into());
                }
            }
        };
    });

    let import_ui = ui.as_weak();
    let import_custom_themes = custom_themes;
    ui.on_theme_import(move || {
        let Some(ui) = import_ui.upgrade() else {
            return;
        };
        let text = match read_clipboard_text() {
            Ok(text) => text,
            Err(error) => {
                ui.set_status_error_message(format!("Theme import failed: {error}").into());
                return;
            }
        };
        let Some(theme) = parse_imported_custom_theme(&text) else {
            ui.set_status_error_message("Clipboard does not contain a valid theme.".into());
            return;
        };

        let mut custom_themes = import_custom_themes.borrow_mut();
        upsert_custom_theme(&mut custom_themes, theme.clone());
        persist_custom_themes(&custom_themes);
        set_theme_gallery(&ui, &custom_themes);
        apply_theme_selection(
            &ui,
            &ThemeSelection::with_skin(theme.colors.clone(), theme.skin_id.as_deref()),
        );
        ui.set_active_theme_id(theme.id.clone().into());
        persist_active_theme_id(&theme.id);
        set_theme_draft(&ui, &theme, false);
        ui.set_status_error_message(format!("Imported theme '{}'.", theme.name).into());
    });
}

fn wire_batch_actions(ui: &KalpaWindow, models: AddonModels) {
    let update_finished_ui = ui.as_weak();
    let update_finished_models = models.clone();
    ui.on_addon_update_check_finished(move |updates, message| {
        let Some(ui) = update_finished_ui.upgrade() else {
            return;
        };
        let update_rows = addon_update_entries_from_model(&updates);
        let available = apply_addon_update_check_results(&update_finished_models, &update_rows);
        ui.set_checking_updates(false);
        ui.set_update_available_count(available as i32);
        apply_addon_view(&ui, &update_finished_models);
        ui.set_status_error_message(message);
    });

    let apply_finished_ui = ui.as_weak();
    let apply_finished_models = models.clone();
    ui.on_addon_update_apply_finished(move |updates, message, conflict_count| {
        let Some(ui) = apply_finished_ui.upgrade() else {
            return;
        };
        let update_rows = addon_update_entries_from_model(&updates);
        let available = apply_addon_update_check_results(&apply_finished_models, &update_rows);
        ui.set_checking_updates(false);
        ui.set_pending_conflict_count(conflict_count);
        ui.set_update_available_count(available as i32);
        apply_addon_view(&ui, &apply_finished_models);
        ui.set_status_error_message(message);
    });

    let review_ui = ui.as_weak();
    let review_models = models.clone();
    ui.on_review_conflicts(move || {
        let Some(ui) = review_ui.upgrade() else {
            return;
        };

        ui.set_discover_active(false);
        ui.set_addon_search_query("".into());
        ui.set_filter_mode(4);
        apply_addon_view(&ui, &review_models);

        let pending_folders = pending_conflict_store()
            .and_then(|store| {
                store
                    .lock()
                    .ok()
                    .map(|pending| pending.keys().cloned().collect::<HashSet<_>>())
            })
            .unwrap_or_default();

        let first_update = (0..review_models.visible.row_count())
            .filter_map(|index| {
                review_models
                    .visible
                    .row_data(index)
                    .map(|addon| (index, addon))
            })
            .find(|(_, addon)| pending_folders.contains(addon.folder_name.as_str()))
            .or_else(|| {
                (0..review_models.visible.row_count())
                    .filter_map(|index| {
                        review_models
                            .visible
                            .row_data(index)
                            .map(|addon| (index, addon))
                    })
                    .find(|(_, addon)| addon_has_update(addon))
            })
            .map(|(index, _)| index)
            .unwrap_or(0);

        if review_models.visible.row_count() > 0 {
            ui.set_selected_index(first_update as i32);
            refresh_file_browser(&ui);
            ui.set_status_error_message(
                "Selected addons that still need update conflict review.".into(),
            );
        } else {
            ui.set_pending_conflict_count(0);
            ui.set_status_error_message("No addon update conflicts are pending.".into());
        }
    });

    let decision_ui = ui.as_weak();
    ui.on_conflict_decision_selected(move |index, decision| {
        let Some(ui) = decision_ui.upgrade() else {
            return;
        };
        let Some(store) = pending_conflict_store() else {
            return;
        };
        let folder_name = selected_addon_folder(&ui);
        if let Ok(mut pending) = store.lock() {
            if let Some(conflict) = pending.get_mut(&folder_name) {
                if let Some(relative_path) = conflict.conflicts.get(index.max(0) as usize).cloned()
                {
                    if matches!(decision, 1 | 2) {
                        conflict.decisions.insert(relative_path, decision);
                    }
                }
            }
        }
        refresh_active_conflict_panel(&ui);
    });

    let diff_ui = ui.as_weak();
    ui.on_conflict_view_diff(move |index| {
        let Some(ui) = diff_ui.upgrade() else {
            return;
        };
        let Some(pending) = selected_pending_conflict(&ui) else {
            clear_active_conflict_diff(&ui);
            return;
        };
        let Some(relative_path) = pending.conflicts.get(index.max(0) as usize).cloned() else {
            return;
        };

        if ui.get_detail_conflict_diff_file().as_str() == relative_path.as_str()
            && !ui.get_detail_conflict_diff_loading()
        {
            clear_active_conflict_diff(&ui);
            return;
        }

        let Some(addons_dir) = addons_source_root() else {
            ui.set_status_error_message("AddOns folder was not found.".into());
            return;
        };

        ui.set_detail_conflict_diff_file(relative_path.clone().into());
        ui.set_detail_conflict_diff_user_preview("".into());
        ui.set_detail_conflict_diff_upstream_preview("".into());
        ui.set_detail_conflict_diff_binary(false);
        ui.set_detail_conflict_diff_loading(true);

        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = native_pending_conflict_diff_blocking(
                &addons_dir,
                &pending,
                relative_path.as_str(),
            );
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                if ui.get_detail_conflict_diff_file().as_str() != relative_path.as_str() {
                    return;
                }
                ui.set_detail_conflict_diff_loading(false);
                match result {
                    Ok(diff) => {
                        ui.set_detail_conflict_diff_user_preview(diff.user_preview.into());
                        ui.set_detail_conflict_diff_upstream_preview(diff.upstream_preview.into());
                        ui.set_detail_conflict_diff_binary(diff.binary);
                    }
                    Err(error) => {
                        clear_active_conflict_diff(&ui);
                        ui.set_status_error_message(format!("Diff failed: {error}").into());
                    }
                }
            });
        });
    });

    let apply_conflict_ui = ui.as_weak();
    ui.on_conflict_apply(move || {
        let Some(ui) = apply_conflict_ui.upgrade() else {
            return;
        };
        let Some(pending) = selected_pending_conflict(&ui) else {
            ui.set_status_error_message("No pending conflict is selected.".into());
            return;
        };
        if !pending_conflict_all_decided(&pending) {
            ui.set_status_error_message("Choose keep or update for every conflicted file.".into());
            return;
        }
        let Some(addons_dir) = addons_source_root() else {
            ui.set_status_error_message("AddOns folder was not found.".into());
            return;
        };
        ui.set_checking_updates(true);
        ui.set_status_error_message("Applying conflict decisions...".into());
        let eso_running = addon_write_eso_running_warning_active(&ui);
        start_native_pending_conflict_apply(ui.as_weak(), addons_dir, pending, eso_running);
    });

    let skip_conflict_ui = ui.as_weak();
    ui.on_conflict_skip(move || {
        let Some(ui) = skip_conflict_ui.upgrade() else {
            return;
        };
        let folder_name = selected_addon_folder(&ui);
        let Some(store) = pending_conflict_store() else {
            return;
        };
        let removed = store
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&folder_name));
        if let Some(conflict) = removed {
            cleanup_pending_conflict_zip(&conflict);
            sync_pending_conflict_count(&ui);
            refresh_active_conflict_panel(&ui);
            ui.set_status_error_message(format!("Skipped update for {}.", folder_name).into());
        }
    });

    let toggle_ui = ui.as_weak();
    let toggle_models = models.clone();
    ui.on_addon_selection_toggled(move |index| {
        let Some(ui) = toggle_ui.upgrade() else {
            return;
        };
        let _ = toggle_visible_addon_selection(&ui, &toggle_models, index.max(0) as usize);
    });

    let clear_ui = ui.as_weak();
    let clear_models = models.clone();
    ui.on_batch_clear(move || {
        if let Some(ui) = clear_ui.upgrade() {
            clear_addon_selection(&ui, &clear_models);
        }
    });

    let disable_ui = ui.as_weak();
    let disable_models = models.clone();
    ui.on_batch_disable(move || {
        if let Some(ui) = disable_ui.upgrade() {
            let selected_folders = disable_models
                .all
                .borrow()
                .iter()
                .filter(|addon| addon.selected)
                .map(|addon| addon.folder_name.to_string())
                .collect::<Vec<_>>();

            let mut changed = 0usize;
            let mut failed = Vec::new();
            let eso_running = addon_write_eso_running_warning_active(&ui);
            for folder_name in selected_folders {
                if let Some(addons_root) = disk_root_for_addon(&folder_name) {
                    match set_addon_disabled_on_disk(&addons_root, &folder_name, true) {
                        Ok(()) => {
                            with_master_addon_mut(&disable_models, &folder_name, |addon| {
                                mark_addon_disabled(addon, true);
                            });
                            changed += 1;
                        }
                        Err(error) => failed.push(format!("{folder_name}: {error}")),
                    }
                } else {
                    with_master_addon_mut(&disable_models, &folder_name, |addon| {
                        mark_addon_disabled(addon, true);
                    });
                    changed += 1;
                }
            }

            apply_addon_view(&ui, &disable_models);
            let message = if failed.is_empty() {
                format!("Disabled {changed} selected addons.")
            } else {
                format!(
                    "Disabled {changed} selected addons; {} failed: {}",
                    failed.len(),
                    failed.join("; ")
                )
            };
            ui.set_status_error_message(addon_write_status_message(message, eso_running).into());
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
                let next_tags = set_tag_active(&addon.tags, "testing", true);
                if let Some(addons_root) = disk_root_for_addon(addon.folder_name.as_str()) {
                    if let Err(error) = persist_addon_tag_model(
                        &addons_root,
                        addon.folder_name.as_str(),
                        &next_tags,
                    ) {
                        ui.set_status_error_message(
                            format!(
                                "Failed to save tags for {}: {error}",
                                addon.folder_name.as_str()
                            )
                            .into(),
                        );
                        return;
                    }
                }
                addon.tags = next_tags;
            }
            apply_addon_view(&ui, &tag_models);
            ui.set_status_error_message("Tagged selected addons as testing.".into());
        }
    });

    let update_ui = ui.as_weak();
    let update_models = models.clone();
    ui.on_batch_update(move || {
        if let Some(ui) = update_ui.upgrade() {
            if ui.get_checking_updates() {
                return;
            }

            let Some(addons_root) = addons_source_root() else {
                ui.set_status_error_message(
                    "AddOns folder was not found. Set it before checking for updates.".into(),
                );
                return;
            };

            let targets = native_update_targets(&update_models);
            if targets.is_empty() {
                ui.set_checking_updates(true);
                ui.set_pending_conflict_count(0);
                ui.set_status_error_message("Checking ESOUI for addon updates...".into());
                start_native_addon_update_check(ui.as_weak(), addons_root);
            } else {
                let eso_running = addon_write_eso_running_warning_active(&ui);
                ui.set_checking_updates(true);
                let message = format!(
                    "Applying {} safe addon update{}...",
                    targets.len(),
                    if targets.len() == 1 { "" } else { "s" }
                );
                ui.set_status_error_message(
                    addon_write_status_message(message, eso_running).into(),
                );
                start_native_addon_update_apply(
                    ui.as_weak(),
                    addons_root,
                    targets,
                    ui.get_settings_conflict_policy().clamp(0, 2),
                    eso_running,
                );
            }
        }
    });

    let remove_ui = ui.as_weak();
    ui.on_batch_remove(move || {
        if let Some(ui) = remove_ui.upgrade() {
            let selected_folders = models
                .all
                .borrow()
                .iter()
                .filter(|addon| addon.selected)
                .map(|addon| addon.folder_name.to_string())
                .collect::<Vec<_>>();

            let mut removed = 0usize;
            let mut failed = Vec::new();
            let eso_running = addon_write_eso_running_warning_active(&ui);
            for folder_name in selected_folders {
                if let Some(addons_root) = disk_root_for_addon(&folder_name) {
                    match remove_addon_from_disk(&addons_root, &folder_name) {
                        Ok(()) => {
                            remove_master_addon(&models, &folder_name);
                            removed += 1;
                        }
                        Err(error) => failed.push(format!("{folder_name}: {error}")),
                    }
                } else {
                    remove_master_addon(&models, &folder_name);
                    removed += 1;
                }
            }

            apply_addon_view(&ui, &models);
            let message = if failed.is_empty() {
                format!("Removed {removed} selected addons.")
            } else {
                format!(
                    "Removed {removed} selected addons; {} failed: {}",
                    failed.len(),
                    failed.join("; ")
                )
            };
            ui.set_status_error_message(addon_write_status_message(message, eso_running).into());
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
        if let Some(addons_root) = disk_root_for_addon(&folder_name) {
            if let Err(error) = persist_addon_tag_model(&addons_root, &folder_name, &addon.tags) {
                ui.set_status_error_message(
                    format!("Failed to save tags for {folder_name}: {error}").into(),
                );
                return;
            }
        }
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
        let next_disabled = !addon.disabled;
        let eso_running = addon_write_eso_running_warning_active(&ui);
        if let Some(addons_root) = disk_root_for_addon(&folder_name) {
            match set_addon_disabled_on_disk(&addons_root, &folder_name, next_disabled) {
                Ok(()) => {}
                Err(error) => {
                    ui.set_status_error_message(error.into());
                    return;
                }
            }
        }

        mark_addon_disabled(&mut addon, next_disabled);
        update_master_addon(&disable_models, &folder_name, addon);
        apply_addon_view(&ui, &disable_models);
        let action = if next_disabled { "Disabled" } else { "Enabled" };
        ui.set_status_error_message(
            addon_write_status_message(format!("{action} {folder_name}."), eso_running).into(),
        );
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
        let eso_running = addon_write_eso_running_warning_active(&ui);
        if let Some(addons_root) = disk_root_for_addon(&folder_name) {
            match remove_addon_from_disk(&addons_root, &folder_name) {
                Ok(()) => {}
                Err(error) => {
                    ui.set_status_error_message(error.into());
                    return;
                }
            }
        }

        remove_master_addon(&models, &folder_name);
        apply_addon_view(&ui, &models);
        ui.set_status_error_message(
            addon_write_status_message(format!("Removed {folder_name}."), eso_running).into(),
        );
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
        if let Some(addons_root) = disk_root_for_addon(&folder_name) {
            if let Err(error) = persist_addon_tag_model(&addons_root, &folder_name, &addon.tags) {
                ui.set_status_error_message(
                    format!("Failed to save tags for {folder_name}: {error}").into(),
                );
                return;
            }
        }
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
        if let Some(addons_root) = disk_root_for_addon(&folder_name) {
            if let Err(error) = persist_addon_tag_model(&addons_root, &folder_name, &addon.tags) {
                ui.set_status_error_message(
                    format!("Failed to save tags for {folder_name}: {error}").into(),
                );
                return;
            }
        }
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
        if let Some(addons_root) = disk_root_for_addon(&folder_name) {
            if let Err(error) = persist_addon_tag_model(&addons_root, &folder_name, &addon.tags) {
                ui.set_status_error_message(
                    format!("Failed to save tags for {folder_name}: {error}").into(),
                );
                return;
            }
        }
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

        let folder_name = addon.folder_name.to_string();
        let next_disabled = !addon.disabled;
        let eso_running = addon_write_eso_running_warning_active(&ui);
        if let Some(addons_root) = disk_root_for_addon(&folder_name) {
            match set_addon_disabled_on_disk(&addons_root, &folder_name, next_disabled) {
                Ok(()) => {}
                Err(error) => {
                    ui.set_status_error_message(error.into());
                    return;
                }
            }
        }

        mark_addon_disabled(&mut addon, next_disabled);
        toggle_models.visible.set_row_data(index, addon.clone());
        update_master_addon(&toggle_models, &folder_name, addon);
        apply_addon_view(&ui, &toggle_models);
        let action = if next_disabled { "Disabled" } else { "Enabled" };
        ui.set_status_error_message(
            addon_write_status_message(format!("{action} {folder_name}."), eso_running).into(),
        );
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
        let eso_running = addon_write_eso_running_warning_active(&ui);
        if let Some(addons_root) = disk_root_for_addon(&folder_name) {
            match remove_addon_from_disk(&addons_root, &folder_name) {
                Ok(()) => {}
                Err(error) => {
                    ui.set_status_error_message(error.into());
                    return;
                }
            }
        }

        remove_master_addon(&remove_models, &folder_name);
        apply_addon_view(&ui, &remove_models);
        ui.set_status_error_message(
            addon_write_status_message(format!("Removed {folder_name}."), eso_running).into(),
        );
    });

    let update_addon_ui = ui.as_weak();
    let update_addon_models = models.clone();
    ui.on_update_addon(move || {
        let Some(ui) = update_addon_ui.upgrade() else {
            return;
        };
        if ui.get_checking_updates() {
            return;
        }
        let Some(addons_root) = addons_source_root() else {
            ui.set_status_error_message(
                "AddOns folder was not found. Set it before updating.".into(),
            );
            return;
        };
        let index = ui.get_selected_index().max(0) as usize;
        let Some(addon) = update_addon_models.visible.row_data(index) else {
            return;
        };
        if !addon_has_update(&addon) {
            return;
        }
        let Ok(esoui_id) = addon.esoui_id.parse::<u32>() else {
            return;
        };
        let target = NativeAddonUpdateTarget {
            folder_name: addon.folder_name.to_string(),
            esoui_id,
        };
        let eso_running = addon_write_eso_running_warning_active(&ui);
        ui.set_checking_updates(true);
        ui.set_status_error_message(
            addon_write_status_message(format!("Updating {}...", addon.title), eso_running).into(),
        );
        start_native_addon_update_apply(
            ui.as_weak(),
            addons_root,
            vec![target],
            ui.get_settings_conflict_policy().clamp(0, 2),
            eso_running,
        );
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
    installed_ids: Arc<Mutex<BTreeSet<String>>>,
    addon_models: AddonModels,
) {
    let tab_ui = ui.as_weak();
    let tab_model = discover_model.clone();
    let tab_installed_ids = installed_ids.clone();
    let browse_state = Arc::new(Mutex::new(DiscoverBrowseState::default()));
    let category_loaded_entries = Arc::new(Mutex::new(Vec::<DiscoverEntry>::new()));
    let popular_request_counter = Arc::new(AtomicU64::new(0));
    let tab_popular_request_counter = popular_request_counter.clone();
    let category_request_counter = Arc::new(AtomicU64::new(0));
    let tab_category_request_counter = category_request_counter.clone();
    let tab_browse_state = browse_state.clone();
    let tab_category_loaded_entries = category_loaded_entries.clone();
    ui.on_discover_tab_selected(move |tab| {
        let Some(ui) = tab_ui.upgrade() else {
            return;
        };

        let normalized_tab = normalize_discover_tab(tab);
        let model = apply_discover_data(&ui, tab, &discover_installed_snapshot(&tab_installed_ids));
        *tab_model.borrow_mut() = model;
        if let Ok(mut state) = tab_browse_state.lock() {
            state.normalize();
            apply_discover_browse_state(&ui, &state);
        }
        if normalized_tab != 1 && normalized_tab != 2 {
            ui.set_discover_browse_loading(false);
            ui.set_discover_browse_has_more(false);
        }
        if normalized_tab != 2 {
            ui.set_discover_category_filter("".into());
        }

        if normalized_tab == 1 {
            let request_id = tab_popular_request_counter.fetch_add(1, Ordering::SeqCst) + 1;
            let request_counter = tab_popular_request_counter.clone();
            let installed_snapshot = discover_installed_snapshot(&tab_installed_ids);
            let state_snapshot = {
                let mut state = tab_browse_state.lock().unwrap_or_else(|e| e.into_inner());
                state.reset_popular_page();
                apply_discover_browse_state(&ui, &state);
                state.clone()
            };
            ui.set_discover_browse_loading(true);
            ui.set_discover_browse_message("".into());
            let browse_state = tab_browse_state.clone();
            let ui_weak = ui.as_weak();
            std::thread::spawn(move || {
                let result = load_discover_popular_page(state_snapshot, &installed_snapshot);

                let _ = slint::invoke_from_event_loop(move || {
                    if request_counter.load(Ordering::SeqCst) != request_id {
                        return;
                    }

                    let Some(ui) = ui_weak.upgrade() else {
                        return;
                    };

                    if ui.get_discover_tab() != 1 {
                        return;
                    }

                    ui.set_discover_browse_loading(false);
                    match result {
                        Ok((next_state, entries, has_more)) if !entries.is_empty() => {
                            if let Ok(mut state) = browse_state.lock() {
                                *state = next_state;
                                state.normalize();
                                apply_discover_browse_state(&ui, &state);
                            }
                            ui.set_discover_browse_has_more(has_more);
                            let model = Rc::new(VecModel::from(entries));
                            ui.set_selected_discover_index(0);
                            ui.set_discover_results(model.into());
                            ui.set_discover_browse_message("".into());
                            ui.set_status_error_message("".into());
                        }
                        Ok((next_state, _, has_more)) => {
                            if let Ok(mut state) = browse_state.lock() {
                                *state = next_state;
                                state.normalize();
                                apply_discover_browse_state(&ui, &state);
                            }
                            ui.set_discover_browse_has_more(has_more);
                            ui.set_discover_browse_message("".into());
                        }
                        Err(error) => {
                            ui.set_discover_browse_has_more(false);
                            let message = format!("Could not load popular ESOUI addons: {error}");
                            ui.set_discover_browse_message(message.clone().into());
                            ui.set_status_error_message(message.into());
                        }
                    }
                });
            });
        }

        if normalized_tab == 2 {
            let request_id = tab_category_request_counter.fetch_add(1, Ordering::SeqCst) + 1;
            let request_counter = tab_category_request_counter.clone();
            let installed_snapshot = discover_installed_snapshot(&tab_installed_ids);
            let browse_state = tab_browse_state.clone();
            let loaded_entries = tab_category_loaded_entries.clone();
            let state_snapshot = {
                let mut state = browse_state.lock().unwrap_or_else(|e| e.into_inner());
                state.reset_category_page();
                apply_discover_browse_state(&ui, &state);
                state.clone()
            };
            ui.set_discover_browse_loading(true);
            ui.set_discover_browse_message("".into());
            ui.set_discover_category_filter("".into());
            let ui_weak = ui.as_weak();
            std::thread::spawn(move || {
                let result = load_discover_category_page(state_snapshot, &installed_snapshot);

                let _ = slint::invoke_from_event_loop(move || {
                    if request_counter.load(Ordering::SeqCst) != request_id {
                        return;
                    }

                    let Some(ui) = ui_weak.upgrade() else {
                        return;
                    };

                    if ui.get_discover_tab() != 2 {
                        return;
                    }

                    ui.set_discover_browse_loading(false);
                    match result {
                        Ok((next_state, entries, has_more)) if !entries.is_empty() => {
                            if let Ok(mut state) = browse_state.lock() {
                                *state = next_state;
                                state.normalize();
                                apply_discover_browse_state(&ui, &state);
                            }
                            ui.set_discover_browse_has_more(has_more);
                            if let Ok(mut loaded) = loaded_entries.lock() {
                                *loaded = entries.clone();
                            }
                            let model = Rc::new(VecModel::from(entries));
                            ui.set_selected_discover_index(0);
                            ui.set_discover_results(model.into());
                            ui.set_discover_browse_message("".into());
                            ui.set_status_error_message("".into());
                        }
                        Ok((next_state, _, has_more)) => {
                            if let Ok(mut state) = browse_state.lock() {
                                *state = next_state;
                                state.normalize();
                                apply_discover_browse_state(&ui, &state);
                            }
                            ui.set_discover_browse_has_more(has_more);
                            if let Ok(mut loaded) = loaded_entries.lock() {
                                loaded.clear();
                            }
                            ui.set_discover_browse_message("".into());
                        }
                        Err(error) => {
                            ui.set_discover_browse_has_more(false);
                            let message = format!("Could not load ESOUI category addons: {error}");
                            ui.set_discover_browse_message(message.clone().into());
                            ui.set_status_error_message(message.into());
                        }
                    }
                });
            });
        }
    });

    let popular_sort_ui = ui.as_weak();
    let popular_sort_state = browse_state.clone();
    let popular_sort_installed_ids = installed_ids.clone();
    let popular_sort_counter = popular_request_counter.clone();
    ui.on_discover_popular_sort_selected(move |sort| {
        let Some(ui) = popular_sort_ui.upgrade() else {
            return;
        };
        {
            let mut state = popular_sort_state.lock().unwrap_or_else(|e| e.into_inner());
            state.popular_sort = sort.clamp(0, 1);
            state.reset_popular_page();
            apply_discover_browse_state(&ui, &state);
        }
        if ui.get_discover_tab() != 1 {
            return;
        }

        let request_id = popular_sort_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let request_counter = popular_sort_counter.clone();
        let installed_snapshot = discover_installed_snapshot(&popular_sort_installed_ids);
        let state_snapshot = popular_sort_state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        ui.set_discover_browse_loading(true);
        ui.set_discover_browse_message("".into());
        let browse_state = popular_sort_state.clone();
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = load_discover_popular_page(state_snapshot, &installed_snapshot);

            let _ = slint::invoke_from_event_loop(move || {
                if request_counter.load(Ordering::SeqCst) != request_id {
                    return;
                }
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                if ui.get_discover_tab() != 1 {
                    return;
                }
                ui.set_discover_browse_loading(false);
                match result {
                    Ok((next_state, entries, has_more)) if !entries.is_empty() => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        let model = Rc::new(VecModel::from(entries));
                        ui.set_selected_discover_index(0);
                        ui.set_discover_results(model.into());
                        ui.set_discover_browse_message("".into());
                        ui.set_status_error_message("".into());
                    }
                    Ok((next_state, _, has_more)) => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        ui.set_discover_browse_message("".into());
                    }
                    Err(error) => {
                        ui.set_discover_browse_has_more(false);
                        let message = format!("Could not load popular ESOUI addons: {error}");
                        ui.set_discover_browse_message(message.clone().into());
                        ui.set_status_error_message(message.into());
                    }
                }
            });
        });
    });

    let category_next_ui = ui.as_weak();
    let category_next_state = browse_state.clone();
    let category_next_installed_ids = installed_ids.clone();
    let category_next_counter = category_request_counter.clone();
    let category_next_loaded_entries = category_loaded_entries.clone();
    ui.on_discover_category_next(move || {
        let Some(ui) = category_next_ui.upgrade() else {
            return;
        };
        {
            let mut state = category_next_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state.select_next_category();
            apply_discover_browse_state(&ui, &state);
        }
        if ui.get_discover_tab() != 2 {
            return;
        }

        let request_id = category_next_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let request_counter = category_next_counter.clone();
        let state_snapshot = category_next_state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        let installed_snapshot = discover_installed_snapshot(&category_next_installed_ids);
        let browse_state = category_next_state.clone();
        let loaded_entries = category_next_loaded_entries.clone();
        ui.set_discover_browse_loading(true);
        ui.set_discover_browse_message("".into());
        ui.set_discover_category_filter("".into());
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = load_discover_category_page(state_snapshot, &installed_snapshot);

            let _ = slint::invoke_from_event_loop(move || {
                if request_counter.load(Ordering::SeqCst) != request_id {
                    return;
                }
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                if ui.get_discover_tab() != 2 {
                    return;
                }
                ui.set_discover_browse_loading(false);
                match result {
                    Ok((next_state, entries, has_more)) if !entries.is_empty() => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        if let Ok(mut loaded) = loaded_entries.lock() {
                            *loaded = entries.clone();
                        }
                        let model = Rc::new(VecModel::from(entries));
                        ui.set_selected_discover_index(0);
                        ui.set_discover_results(model.into());
                        ui.set_discover_browse_message("".into());
                        ui.set_status_error_message("".into());
                    }
                    Ok((next_state, _, has_more)) => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        if let Ok(mut loaded) = loaded_entries.lock() {
                            loaded.clear();
                        }
                        ui.set_discover_browse_message("".into());
                    }
                    Err(error) => {
                        ui.set_discover_browse_has_more(false);
                        let message = format!("Could not load ESOUI category addons: {error}");
                        ui.set_discover_browse_message(message.clone().into());
                        ui.set_status_error_message(message.into());
                    }
                }
            });
        });
    });

    let category_select_ui = ui.as_weak();
    let category_select_state = browse_state.clone();
    let category_select_installed_ids = installed_ids.clone();
    let category_select_counter = category_request_counter.clone();
    let category_select_loaded_entries = category_loaded_entries.clone();
    ui.on_discover_category_selected(move |index| {
        let Some(ui) = category_select_ui.upgrade() else {
            return;
        };
        {
            let mut state = category_select_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state.select_category_index(index.max(0) as usize);
            apply_discover_browse_state(&ui, &state);
        }
        if ui.get_discover_tab() != 2 {
            return;
        }

        let request_id = category_select_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let request_counter = category_select_counter.clone();
        let state_snapshot = category_select_state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        let installed_snapshot = discover_installed_snapshot(&category_select_installed_ids);
        let browse_state = category_select_state.clone();
        let loaded_entries = category_select_loaded_entries.clone();
        ui.set_discover_browse_loading(true);
        ui.set_discover_browse_message("".into());
        ui.set_discover_category_filter("".into());
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = load_discover_category_page(state_snapshot, &installed_snapshot);

            let _ = slint::invoke_from_event_loop(move || {
                if request_counter.load(Ordering::SeqCst) != request_id {
                    return;
                }
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                if ui.get_discover_tab() != 2 {
                    return;
                }
                ui.set_discover_browse_loading(false);
                match result {
                    Ok((next_state, entries, has_more)) if !entries.is_empty() => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        if let Ok(mut loaded) = loaded_entries.lock() {
                            *loaded = entries.clone();
                        }
                        let model = Rc::new(VecModel::from(entries));
                        ui.set_selected_discover_index(0);
                        ui.set_discover_results(model.into());
                        ui.set_discover_browse_message("".into());
                        ui.set_status_error_message("".into());
                    }
                    Ok((next_state, _, has_more)) => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        if let Ok(mut loaded) = loaded_entries.lock() {
                            loaded.clear();
                        }
                        ui.set_discover_browse_message("".into());
                    }
                    Err(error) => {
                        ui.set_discover_browse_has_more(false);
                        let message = format!("Could not load ESOUI category addons: {error}");
                        ui.set_discover_browse_message(message.clone().into());
                        ui.set_status_error_message(message.into());
                    }
                }
            });
        });
    });

    let category_sort_ui = ui.as_weak();
    let category_sort_state = browse_state.clone();
    let category_sort_installed_ids = installed_ids.clone();
    let category_sort_counter = category_request_counter.clone();
    let category_sort_loaded_entries = category_loaded_entries.clone();
    ui.on_discover_category_sort_next(move || {
        let Some(ui) = category_sort_ui.upgrade() else {
            return;
        };
        {
            let mut state = category_sort_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state.select_next_category_sort();
            apply_discover_browse_state(&ui, &state);
        }
        if ui.get_discover_tab() != 2 {
            return;
        }

        let request_id = category_sort_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let request_counter = category_sort_counter.clone();
        let state_snapshot = category_sort_state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        let installed_snapshot = discover_installed_snapshot(&category_sort_installed_ids);
        let browse_state = category_sort_state.clone();
        let loaded_entries = category_sort_loaded_entries.clone();
        ui.set_discover_browse_loading(true);
        ui.set_discover_browse_message("".into());
        ui.set_discover_category_filter("".into());
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = load_discover_category_page(state_snapshot, &installed_snapshot);

            let _ = slint::invoke_from_event_loop(move || {
                if request_counter.load(Ordering::SeqCst) != request_id {
                    return;
                }
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                if ui.get_discover_tab() != 2 {
                    return;
                }
                ui.set_discover_browse_loading(false);
                match result {
                    Ok((next_state, entries, has_more)) if !entries.is_empty() => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        if let Ok(mut loaded) = loaded_entries.lock() {
                            *loaded = entries.clone();
                        }
                        let model = Rc::new(VecModel::from(entries));
                        ui.set_selected_discover_index(0);
                        ui.set_discover_results(model.into());
                        ui.set_discover_browse_message("".into());
                        ui.set_status_error_message("".into());
                    }
                    Ok((next_state, _, has_more)) => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        if let Ok(mut loaded) = loaded_entries.lock() {
                            loaded.clear();
                        }
                        ui.set_discover_browse_message("".into());
                    }
                    Err(error) => {
                        ui.set_discover_browse_has_more(false);
                        let message = format!("Could not load ESOUI category addons: {error}");
                        ui.set_discover_browse_message(message.clone().into());
                        ui.set_status_error_message(message.into());
                    }
                }
            });
        });
    });

    let category_sort_select_ui = ui.as_weak();
    let category_sort_select_state = browse_state.clone();
    let category_sort_select_installed_ids = installed_ids.clone();
    let category_sort_select_counter = category_request_counter.clone();
    let category_sort_select_loaded_entries = category_loaded_entries.clone();
    ui.on_discover_category_sort_selected(move |sort| {
        let Some(ui) = category_sort_select_ui.upgrade() else {
            return;
        };
        {
            let mut state = category_sort_select_state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state.select_category_sort(sort);
            apply_discover_browse_state(&ui, &state);
        }
        if ui.get_discover_tab() != 2 {
            return;
        }

        let request_id = category_sort_select_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let request_counter = category_sort_select_counter.clone();
        let state_snapshot = category_sort_select_state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        let installed_snapshot = discover_installed_snapshot(&category_sort_select_installed_ids);
        let browse_state = category_sort_select_state.clone();
        let loaded_entries = category_sort_select_loaded_entries.clone();
        ui.set_discover_browse_loading(true);
        ui.set_discover_browse_message("".into());
        ui.set_discover_category_filter("".into());
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = load_discover_category_page(state_snapshot, &installed_snapshot);

            let _ = slint::invoke_from_event_loop(move || {
                if request_counter.load(Ordering::SeqCst) != request_id {
                    return;
                }
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };
                if ui.get_discover_tab() != 2 {
                    return;
                }
                ui.set_discover_browse_loading(false);
                match result {
                    Ok((next_state, entries, has_more)) if !entries.is_empty() => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        if let Ok(mut loaded) = loaded_entries.lock() {
                            *loaded = entries.clone();
                        }
                        let model = Rc::new(VecModel::from(entries));
                        ui.set_selected_discover_index(0);
                        ui.set_discover_results(model.into());
                        ui.set_discover_browse_message("".into());
                        ui.set_status_error_message("".into());
                    }
                    Ok((next_state, _, has_more)) => {
                        if let Ok(mut state) = browse_state.lock() {
                            *state = next_state;
                            state.normalize();
                            apply_discover_browse_state(&ui, &state);
                        }
                        ui.set_discover_browse_has_more(has_more);
                        if let Ok(mut loaded) = loaded_entries.lock() {
                            loaded.clear();
                        }
                        ui.set_discover_browse_message("".into());
                    }
                    Err(error) => {
                        ui.set_discover_browse_has_more(false);
                        let message = format!("Could not load ESOUI category addons: {error}");
                        ui.set_discover_browse_message(message.clone().into());
                        ui.set_status_error_message(message.into());
                    }
                }
            });
        });
    });

    let load_more_ui = ui.as_weak();
    let load_more_state = browse_state.clone();
    let load_more_installed_ids = installed_ids.clone();
    let load_more_popular_counter = popular_request_counter.clone();
    let load_more_category_counter = category_request_counter.clone();
    let load_more_category_entries = category_loaded_entries.clone();
    ui.on_discover_load_more(move || {
        let Some(ui) = load_more_ui.upgrade() else {
            return;
        };
        if ui.get_discover_browse_loading() || !ui.get_discover_browse_has_more() {
            return;
        }

        match ui.get_discover_tab() {
            1 => {
                let request_id = load_more_popular_counter.fetch_add(1, Ordering::SeqCst) + 1;
                let request_counter = load_more_popular_counter.clone();
                let state_snapshot = load_more_state
                    .lock()
                    .map(|state| state.next_popular_page_snapshot())
                    .unwrap_or_default();
                let installed_snapshot = discover_installed_snapshot(&load_more_installed_ids);
                let browse_state = load_more_state.clone();
                ui.set_discover_browse_loading(true);
                ui.set_discover_browse_message("".into());
                let ui_weak = ui.as_weak();
                std::thread::spawn(move || {
                    let result = load_discover_popular_page(state_snapshot, &installed_snapshot);
                    let _ = slint::invoke_from_event_loop(move || {
                        if request_counter.load(Ordering::SeqCst) != request_id {
                            return;
                        }
                        let Some(ui) = ui_weak.upgrade() else {
                            return;
                        };
                        if ui.get_discover_tab() != 1 {
                            return;
                        }

                        ui.set_discover_browse_loading(false);
                        match result {
                            Ok((next_state, entries, has_more)) => {
                                if let Ok(mut state) = browse_state.lock() {
                                    *state = next_state;
                                    state.normalize();
                                    apply_discover_browse_state(&ui, &state);
                                }
                                ui.set_discover_browse_has_more(has_more);
                                if entries.is_empty() {
                                    ui.set_discover_browse_message("".into());
                                    ui.set_status_error_message(
                                        "No more popular ESOUI addons to load.".into(),
                                    );
                                } else {
                                    append_discover_results(&ui, entries);
                                    ui.set_discover_browse_message("".into());
                                    ui.set_status_error_message("".into());
                                }
                            }
                            Err(error) => {
                                ui.set_discover_browse_has_more(false);
                                ui.set_status_error_message(
                                    format!("Could not load more popular ESOUI addons: {error}")
                                        .into(),
                                );
                            }
                        }
                    });
                });
            }
            2 => {
                let request_id = load_more_category_counter.fetch_add(1, Ordering::SeqCst) + 1;
                let request_counter = load_more_category_counter.clone();
                let state_snapshot = load_more_state
                    .lock()
                    .map(|state| state.next_category_page_snapshot())
                    .unwrap_or_default();
                let installed_snapshot = discover_installed_snapshot(&load_more_installed_ids);
                let browse_state = load_more_state.clone();
                let loaded_entries = load_more_category_entries.clone();
                ui.set_discover_browse_loading(true);
                ui.set_discover_browse_message("".into());
                let ui_weak = ui.as_weak();
                std::thread::spawn(move || {
                    let result = load_discover_category_page(state_snapshot, &installed_snapshot);
                    let _ = slint::invoke_from_event_loop(move || {
                        if request_counter.load(Ordering::SeqCst) != request_id {
                            return;
                        }
                        let Some(ui) = ui_weak.upgrade() else {
                            return;
                        };
                        if ui.get_discover_tab() != 2 {
                            return;
                        }

                        ui.set_discover_browse_loading(false);
                        match result {
                            Ok((next_state, entries, has_more)) => {
                                if let Ok(mut state) = browse_state.lock() {
                                    *state = next_state;
                                    state.normalize();
                                    apply_discover_browse_state(&ui, &state);
                                }
                                ui.set_discover_browse_has_more(has_more);
                                if entries.is_empty() {
                                    ui.set_discover_browse_message("".into());
                                    ui.set_status_error_message(
                                        "No more ESOUI category addons to load.".into(),
                                    );
                                } else {
                                    if let Ok(mut loaded) = loaded_entries.lock() {
                                        loaded.extend(entries);
                                        apply_category_discover_results(&ui, &loaded);
                                    } else {
                                        append_discover_results(&ui, entries);
                                    }
                                    ui.set_discover_browse_message("".into());
                                    ui.set_status_error_message("".into());
                                }
                            }
                            Err(error) => {
                                ui.set_discover_browse_has_more(false);
                                ui.set_status_error_message(
                                    format!("Could not load more ESOUI category addons: {error}")
                                        .into(),
                                );
                            }
                        }
                    });
                });
            }
            _ => {}
        }
    });

    let retry_ui = ui.as_weak();
    let retry_state = browse_state.clone();
    let retry_installed_ids = installed_ids.clone();
    let retry_popular_counter = popular_request_counter.clone();
    let retry_category_counter = category_request_counter.clone();
    let retry_category_entries = category_loaded_entries.clone();
    ui.on_discover_browse_retry(move || {
        let Some(ui) = retry_ui.upgrade() else {
            return;
        };

        match ui.get_discover_tab() {
            1 => {
                let request_id = retry_popular_counter.fetch_add(1, Ordering::SeqCst) + 1;
                let request_counter = retry_popular_counter.clone();
                let installed_snapshot = discover_installed_snapshot(&retry_installed_ids);
                let state_snapshot = {
                    let mut state = retry_state.lock().unwrap_or_else(|e| e.into_inner());
                    state.reset_popular_page();
                    apply_discover_browse_state(&ui, &state);
                    state.clone()
                };
                let browse_state = retry_state.clone();
                ui.set_discover_browse_loading(true);
                ui.set_discover_browse_message("".into());
                let ui_weak = ui.as_weak();
                std::thread::spawn(move || {
                    let result = load_discover_popular_page(state_snapshot, &installed_snapshot);
                    let _ = slint::invoke_from_event_loop(move || {
                        if request_counter.load(Ordering::SeqCst) != request_id {
                            return;
                        }
                        let Some(ui) = ui_weak.upgrade() else {
                            return;
                        };
                        if ui.get_discover_tab() != 1 {
                            return;
                        }

                        ui.set_discover_browse_loading(false);
                        match result {
                            Ok((next_state, entries, has_more)) if !entries.is_empty() => {
                                if let Ok(mut state) = browse_state.lock() {
                                    *state = next_state;
                                    state.normalize();
                                    apply_discover_browse_state(&ui, &state);
                                }
                                ui.set_discover_browse_has_more(has_more);
                                let model = Rc::new(VecModel::from(entries));
                                ui.set_selected_discover_index(0);
                                ui.set_discover_results(model.into());
                                ui.set_discover_browse_message("".into());
                                ui.set_status_error_message("".into());
                            }
                            Ok((next_state, _, has_more)) => {
                                if let Ok(mut state) = browse_state.lock() {
                                    *state = next_state;
                                    state.normalize();
                                    apply_discover_browse_state(&ui, &state);
                                }
                                ui.set_discover_browse_has_more(has_more);
                                ui.set_discover_browse_message("".into());
                            }
                            Err(error) => {
                                ui.set_discover_browse_has_more(false);
                                let message =
                                    format!("Could not load popular ESOUI addons: {error}");
                                ui.set_discover_browse_message(message.clone().into());
                                ui.set_status_error_message(message.into());
                            }
                        }
                    });
                });
            }
            2 => {
                let request_id = retry_category_counter.fetch_add(1, Ordering::SeqCst) + 1;
                let request_counter = retry_category_counter.clone();
                let installed_snapshot = discover_installed_snapshot(&retry_installed_ids);
                let state_snapshot = {
                    let mut state = retry_state.lock().unwrap_or_else(|e| e.into_inner());
                    state.reset_category_page();
                    apply_discover_browse_state(&ui, &state);
                    state.clone()
                };
                let browse_state = retry_state.clone();
                let loaded_entries = retry_category_entries.clone();
                ui.set_discover_browse_loading(true);
                ui.set_discover_browse_message("".into());
                ui.set_discover_category_filter("".into());
                let ui_weak = ui.as_weak();
                std::thread::spawn(move || {
                    let result = load_discover_category_page(state_snapshot, &installed_snapshot);
                    let _ = slint::invoke_from_event_loop(move || {
                        if request_counter.load(Ordering::SeqCst) != request_id {
                            return;
                        }
                        let Some(ui) = ui_weak.upgrade() else {
                            return;
                        };
                        if ui.get_discover_tab() != 2 {
                            return;
                        }

                        ui.set_discover_browse_loading(false);
                        match result {
                            Ok((next_state, entries, has_more)) if !entries.is_empty() => {
                                if let Ok(mut state) = browse_state.lock() {
                                    *state = next_state;
                                    state.normalize();
                                    apply_discover_browse_state(&ui, &state);
                                }
                                ui.set_discover_browse_has_more(has_more);
                                if let Ok(mut loaded) = loaded_entries.lock() {
                                    *loaded = entries.clone();
                                }
                                let model = Rc::new(VecModel::from(entries));
                                ui.set_selected_discover_index(0);
                                ui.set_discover_results(model.into());
                                ui.set_discover_browse_message("".into());
                                ui.set_status_error_message("".into());
                            }
                            Ok((next_state, _, has_more)) => {
                                if let Ok(mut state) = browse_state.lock() {
                                    *state = next_state;
                                    state.normalize();
                                    apply_discover_browse_state(&ui, &state);
                                }
                                ui.set_discover_browse_has_more(has_more);
                                if let Ok(mut loaded) = loaded_entries.lock() {
                                    loaded.clear();
                                }
                                ui.set_discover_browse_message("".into());
                            }
                            Err(error) => {
                                ui.set_discover_browse_has_more(false);
                                let message =
                                    format!("Could not load ESOUI category addons: {error}");
                                ui.set_discover_browse_message(message.clone().into());
                                ui.set_status_error_message(message.into());
                            }
                        }
                    });
                });
            }
            _ => {}
        }
    });

    let category_filter_ui = ui.as_weak();
    let category_filter_entries = category_loaded_entries.clone();
    let category_filter_state = browse_state.clone();
    ui.on_discover_category_filter_edited(move |value| {
        let Some(ui) = category_filter_ui.upgrade() else {
            return;
        };
        if ui.get_discover_tab() != 2 {
            return;
        }

        ui.set_discover_category_filter(value.clone());
        if let Ok(entries) = category_filter_entries.lock() {
            apply_category_discover_results(&ui, &entries);
        }

        if value.trim().is_empty() {
            if let Ok(state) = category_filter_state.lock() {
                apply_discover_browse_state(&ui, &state);
            }
        } else {
            ui.set_discover_browse_has_more(false);
        }
    });

    let query_ui = ui.as_weak();
    let query_model = discover_model.clone();
    let query_installed_ids = installed_ids.clone();
    let query_request_counter = Arc::new(AtomicU64::new(0));
    ui.on_discover_query_edited(move |_| {
        let Some(ui) = query_ui.upgrade() else {
            return;
        };

        if ui.get_discover_tab() != 0 {
            return;
        }

        let model = apply_discover_data(&ui, 0, &discover_installed_snapshot(&query_installed_ids));
        *query_model.borrow_mut() = model;

        let query = ui.get_discover_query().to_string();
        if query.trim().is_empty() {
            ui.set_status_error_message("".into());
            return;
        }

        let request_id = query_request_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let request_counter = query_request_counter.clone();
        let installed_snapshot = discover_installed_snapshot(&query_installed_ids);
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = esoui::search_esoui(query.as_str())
                .map(|results| discover_entries_from_search_results(results, &installed_snapshot));

            let _ = slint::invoke_from_event_loop(move || {
                if request_counter.load(Ordering::SeqCst) != request_id {
                    return;
                }

                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                if ui.get_discover_tab() != 0 {
                    return;
                }

                match result {
                    Ok(entries) => {
                        let selected_index = if entries.is_empty() {
                            0
                        } else {
                            ui.get_selected_discover_index()
                                .max(0)
                                .min(entries.len().saturating_sub(1) as i32)
                        };
                        let model = Rc::new(VecModel::from(entries));
                        ui.set_selected_discover_index(selected_index);
                        ui.set_discover_results(model.into());
                        clear_discover_screenshots(&ui);
                        ui.set_status_error_message("".into());
                    }
                    Err(error) => {
                        ui.set_status_error_message(
                            format!("Could not search ESOUI addons: {error}").into(),
                        );
                    }
                }
            });
        });
    });

    let screenshot_request_counter = Arc::new(AtomicU64::new(0));

    let url_ui = ui.as_weak();
    let url_model = discover_model.clone();
    let url_installed_ids = installed_ids.clone();
    let url_request_counter = Arc::new(AtomicU64::new(0));
    let url_screenshot_counter = screenshot_request_counter.clone();
    ui.on_discover_url_edited(move |_| {
        let Some(ui) = url_ui.upgrade() else {
            return;
        };

        if ui.get_discover_tab() != 3 {
            return;
        }

        let model = apply_discover_data(&ui, 3, &discover_installed_snapshot(&url_installed_ids));
        *url_model.borrow_mut() = model;

        let input = ui.get_discover_url_input().to_string();
        if input.trim().is_empty() {
            ui.set_discover_browse_message("".into());
            ui.set_status_error_message("".into());
            return;
        }

        let request_id = url_request_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let request_counter = url_request_counter.clone();
        let screenshot_counter = url_screenshot_counter.clone();
        let installed_snapshot = discover_installed_snapshot(&url_installed_ids);
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = esoui::parse_esoui_input(input.as_str())
                .and_then(esoui::fetch_addon_detail)
                .map(|detail| {
                    let screenshots = detail.screenshots.clone();
                    let addon_id = detail.id.to_string();
                    let installed = installed_snapshot.contains(&detail.id.to_string());
                    (
                        discover_entry_from_esoui_detail(detail, installed, 0),
                        addon_id,
                        screenshots,
                    )
                });

            let _ = slint::invoke_from_event_loop(move || {
                if request_counter.load(Ordering::SeqCst) != request_id {
                    return;
                }

                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                match result {
                    Ok((entry, addon_id, screenshots)) => {
                        let model = Rc::new(VecModel::from(vec![entry]));
                        ui.set_selected_discover_index(0);
                        ui.set_discover_results(model.into());
                        request_discover_screenshots(
                            &ui,
                            addon_id,
                            screenshots,
                            screenshot_counter.clone(),
                        );
                        ui.set_discover_browse_message("".into());
                        ui.set_status_error_message("".into());
                    }
                    Err(error) => {
                        // Surface the failure inline in the Discover body (with a
                        // retry), not just in the global status line, so a bad
                        // URL/ID doesn't look like an empty search.
                        let message = format!("Could not resolve ESOUI addon: {error}");
                        ui.set_discover_results(Rc::new(VecModel::from(Vec::<DiscoverEntry>::new())).into());
                        ui.set_discover_browse_message(message.clone().into());
                        ui.set_status_error_message(message.into());
                    }
                }
            });
        });
    });

    let selected_ui = ui.as_weak();
    let detail_request_counter = Arc::new(AtomicU64::new(0));
    ui.on_discover_selected(move |index| {
        let Some(ui) = selected_ui.upgrade() else {
            return;
        };

        let model = ui.get_discover_results();
        let row_count = model.row_count();
        if row_count == 0 {
            ui.set_selected_discover_index(0);
            clear_discover_screenshots(&ui);
            return;
        }

        let next_index = (index.max(0) as usize).min(row_count.saturating_sub(1));
        let Some(entry) = model.row_data(next_index) else {
            return;
        };
        let current_index = ui.get_selected_discover_index().max(0) as usize;
        if next_index == current_index && !discover_entry_needs_detail(&entry) {
            return;
        }

        ui.set_selected_discover_index(next_index as i32);
        clear_discover_screenshots(&ui);
        screenshot_request_counter.fetch_add(1, Ordering::SeqCst);

        let Ok(esoui_id) = entry.esoui_id.parse::<u32>() else {
            return;
        };
        let request_id = detail_request_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let request_counter = detail_request_counter.clone();
        let screenshot_counter = screenshot_request_counter.clone();
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = esoui::fetch_addon_detail(esoui_id);

            let _ = slint::invoke_from_event_loop(move || {
                if request_counter.load(Ordering::SeqCst) != request_id {
                    return;
                }

                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                let model = ui.get_discover_results();
                let selected_index = ui.get_selected_discover_index().max(0) as usize;
                let Some(entry) = model.row_data(selected_index) else {
                    return;
                };

                if entry.esoui_id.as_str() != esoui_id.to_string() {
                    return;
                }

                match result {
                    Ok(detail) => {
                        let screenshots = detail.screenshots.clone();
                        model.set_row_data(selected_index, merge_discover_detail(entry, detail));
                        request_discover_screenshots(
                            &ui,
                            esoui_id.to_string(),
                            screenshots,
                            screenshot_counter.clone(),
                        );
                        ui.set_status_error_message("".into());
                    }
                    Err(error) => {
                        ui.set_status_error_message(
                            format!("Could not load ESOUI addon details: {error}").into(),
                        );
                    }
                }
            });
        });
    });

    let screenshot_prev_ui = ui.as_weak();
    ui.on_discover_screenshot_prev(move || {
        let Some(ui) = screenshot_prev_ui.upgrade() else {
            return;
        };
        let screenshot_count = ui.get_discover_screenshots().row_count();
        if screenshot_count <= 1 {
            return;
        }
        let current = ui.get_discover_screenshot_index().max(0) as usize;
        let next = if current == 0 {
            screenshot_count.saturating_sub(1)
        } else {
            current.saturating_sub(1)
        };
        apply_discover_screenshot_selection(&ui, next);
    });

    let screenshot_next_ui = ui.as_weak();
    ui.on_discover_screenshot_next(move || {
        let Some(ui) = screenshot_next_ui.upgrade() else {
            return;
        };
        let screenshot_count = ui.get_discover_screenshots().row_count();
        if screenshot_count <= 1 {
            return;
        }
        let current = ui.get_discover_screenshot_index().max(0) as usize;
        apply_discover_screenshot_selection(&ui, (current + 1) % screenshot_count);
    });

    let screenshot_select_ui = ui.as_weak();
    ui.on_discover_screenshot_select(move |index| {
        let Some(ui) = screenshot_select_ui.upgrade() else {
            return;
        };
        let screenshot_count = ui.get_discover_screenshots().row_count();
        if screenshot_count == 0 {
            return;
        }
        let next = (index.max(0) as usize).min(screenshot_count.saturating_sub(1));
        apply_discover_screenshot_selection(&ui, next);
    });

    let install_ui = ui.as_weak();
    let install_ids = installed_ids.clone();
    ui.on_discover_install(move |index| {
        let Some(ui) = install_ui.upgrade() else {
            return;
        };
        let model = ui.get_discover_results();
        let Some(entry) = model.row_data(index.max(0) as usize) else {
            return;
        };

        let esoui_id = entry.esoui_id.to_string();
        let addons_dir = match configured_addons_path() {
            Some(path) => path,
            None => {
                ui.set_status_error_message(
                    "Configure the ESO AddOns folder before installing from Discover.".into(),
                );
                return;
            }
        };

        let eso_running = addon_write_eso_running_warning_active(&ui);
        ui.set_status_error_message(
            addon_write_status_message(
                format!("Installing {}...", entry.title.as_str()),
                eso_running,
            )
            .into(),
        );

        let installed_ids = install_ids.clone();
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = install_discover_entry_blocking(&addons_dir, entry).map(|installed| {
                (
                    installed,
                    format!("Installed ESOUI addon {esoui_id}."),
                    esoui_id,
                )
            });

            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui_weak.upgrade() else {
                    return;
                };

                match result {
                    Ok((installed_folders, message, esoui_id)) => {
                        if let Ok(mut ids) = installed_ids.lock() {
                            ids.insert(esoui_id.clone());
                        }
                        mark_discover_installed_model(&ui.get_discover_results(), &esoui_id);
                        ui.invoke_refresh_requested();
                        let message = format!(
                            "{message} {} folder{} added.",
                            installed_folders.len(),
                            if installed_folders.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        );
                        ui.set_status_error_message(
                            addon_write_status_message(message, eso_running).into(),
                        );
                    }
                    Err(error) => {
                        ui.set_status_error_message(format!("Install failed: {error}").into());
                    }
                }
            });
        });
    });

    let remove_ui = ui.as_weak();
    let remove_models = addon_models;
    let remove_ids = installed_ids;
    ui.on_discover_remove(move |index| {
        let Some(ui) = remove_ui.upgrade() else {
            return;
        };

        let model = ui.get_discover_results();
        let Some(entry) = model.row_data(index.max(0) as usize) else {
            return;
        };
        let esoui_id = entry.esoui_id.to_string();
        let eso_running = addon_write_eso_running_warning_active(&ui);

        match remove_addons_by_esoui_id(&remove_models, &esoui_id) {
            Ok(removed) => {
                if let Ok(mut ids) = remove_ids.lock() {
                    ids.remove(&esoui_id);
                }
                mark_discover_uninstalled_model(&model, &esoui_id);
                apply_addon_view(&ui, &remove_models);
                let message = format!(
                    "Removed {} addon folder{} for ESOUI {}.",
                    removed.len(),
                    if removed.len() == 1 { "" } else { "s" },
                    esoui_id
                );
                ui.set_status_error_message(
                    addon_write_status_message(message, eso_running).into(),
                );
            }
            Err(error) => {
                ui.set_status_error_message(format!("Remove failed: {error}").into());
            }
        }
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

#[cfg(test)]
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

fn mark_discover_installed_model(discover_model: &ModelRc<DiscoverEntry>, esoui_id: &str) {
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

fn mark_discover_uninstalled_model(discover_model: &ModelRc<DiscoverEntry>, esoui_id: &str) {
    for index in 0..discover_model.row_count() {
        let Some(mut entry) = discover_model.row_data(index) else {
            continue;
        };

        if entry.esoui_id.as_str() == esoui_id {
            entry.installed = false;
            discover_model.set_row_data(index, entry);
        }
    }
}

fn remove_addons_by_esoui_id(models: &AddonModels, esoui_id: &str) -> Result<Vec<String>, String> {
    let targets = models
        .all
        .borrow()
        .iter()
        .filter(|addon| addon.esoui_id.as_str() == esoui_id)
        .map(|addon| addon.folder_name.to_string())
        .collect::<Vec<_>>();

    if targets.is_empty() {
        return Err(format!("No installed addon found for ESOUI {esoui_id}."));
    }

    for folder_name in &targets {
        if let Some(addons_root) = disk_root_for_addon(folder_name) {
            remove_addon_from_disk(&addons_root, folder_name)?;
        }
    }

    for folder_name in &targets {
        remove_master_addon(models, folder_name);
    }

    Ok(targets)
}

fn discover_installed_snapshot(installed_ids: &Arc<Mutex<BTreeSet<String>>>) -> BTreeSet<String> {
    installed_ids
        .lock()
        .map(|ids| ids.clone())
        .unwrap_or_default()
}

fn install_discover_entry_blocking(
    addons_dir: &Path,
    entry: DiscoverEntry,
) -> Result<Vec<String>, String> {
    let esoui_id = entry
        .esoui_id
        .parse::<u32>()
        .map_err(|_| "Selected Discover row does not have a valid ESOUI id.".to_string())?;
    let detail = esoui::fetch_addon_detail(esoui_id)?;
    let expected_md5 = (!detail.md5.trim().is_empty()).then_some(detail.md5.as_str());
    let tmp_file = esoui::download_addon(&detail.download_url, expected_md5)?;
    install_discover_download_blocking(addons_dir, tmp_file.path(), &detail)
}

fn install_discover_download_blocking(
    addons_dir: &Path,
    zip_path: &Path,
    detail: &esoui::EsouiAddonDetail,
) -> Result<Vec<String>, String> {
    install_downloaded_addon_blocking(
        addons_dir,
        zip_path,
        detail.id,
        &detail.title,
        &detail.version,
        &detail.download_url,
    )
}

fn install_downloaded_addon_blocking(
    addons_dir: &Path,
    zip_path: &Path,
    esoui_id: u32,
    title: &str,
    version: &str,
    download_url: &str,
) -> Result<Vec<String>, String> {
    let installed_folders = installer::extract_addon_zip(zip_path, addons_dir)?;
    file_hashes::record_hashes_for_folders(addons_dir, &installed_folders, esoui_id, version)?;

    let mut store = metadata::load_metadata(addons_dir);
    record_native_installed_folders(
        &mut store,
        addons_dir,
        &installed_folders,
        esoui_id,
        version,
        title,
        download_url,
    );
    metadata::save_metadata(addons_dir, &store)?;

    Ok(installed_folders)
}

fn record_native_installed_folders(
    store: &mut metadata::MetadataStore,
    addons_dir: &Path,
    installed_folders: &[String],
    esoui_id: u32,
    esoui_version: &str,
    esoui_title: &str,
    download_url: &str,
) {
    let primary = determine_primary_folder(installed_folders, esoui_title);
    for folder in installed_folders {
        let is_primary = *folder == primary;
        let version = if is_primary && !esoui_version.is_empty() {
            esoui_version.to_string()
        } else {
            read_local_version(addons_dir, folder)
        };
        metadata::record_install_ext(
            store,
            folder,
            if is_primary { esoui_id } else { 0 },
            &version,
            download_url,
            0,
        );
    }
}

fn determine_primary_folder(installed_folders: &[String], esoui_title: &str) -> String {
    installed_folders
        .iter()
        .find(|folder| esoui_title.contains(folder.as_str()))
        .or(installed_folders.first())
        .cloned()
        .unwrap_or_default()
}

fn read_local_version(addons_dir: &Path, folder: &str) -> String {
    find_manifest(addons_dir, folder)
        .and_then(|path| manifest::parse_manifest(folder, &path))
        .map(|manifest| manifest.version)
        .unwrap_or_default()
}

fn normalized_addon_version(version: &str) -> &str {
    version
        .trim()
        .strip_prefix('v')
        .or_else(|| version.trim().strip_prefix('V'))
        .unwrap_or(version.trim())
}

fn check_native_addon_updates_blocking(
    addons_dir: &Path,
) -> Result<Vec<NativeAddonUpdateCheck>, String> {
    let api_lookup = esoui::fetch_filelist_lookup()?;
    let mut store = metadata::load_metadata(addons_dir);
    let folder_names = store.addons.keys().cloned().collect::<Vec<_>>();
    let mut metadata_changed = false;
    let mut results = Vec::new();

    for folder_name in folder_names {
        if !addons_dir.join(&folder_name).is_dir() {
            continue;
        }

        let Some(meta) = store.addons.get(&folder_name).cloned() else {
            continue;
        };
        if meta.esoui_id == 0 {
            continue;
        }

        let Some(api_entry) = api_lookup.get(&folder_name) else {
            continue;
        };

        let local_version = normalized_addon_version(&meta.installed_version);
        let remote_version = normalized_addon_version(&api_entry.version);
        let has_update = !remote_version.is_empty()
            && !local_version.is_empty()
            && remote_version != local_version;

        if let Some(entry) = store.addons.get_mut(&folder_name) {
            if !has_update && entry.installed_version != api_entry.version {
                entry.installed_version = api_entry.version.clone();
                metadata_changed = true;
            }
            if entry.esoui_last_update != api_entry.last_update {
                entry.esoui_last_update = api_entry.last_update;
                metadata_changed = true;
            }
        }

        results.push(NativeAddonUpdateCheck {
            folder_name,
            remote_version: api_entry.version.clone(),
            has_update,
            remote_last_update: api_entry.last_update,
        });
    }

    if metadata_changed {
        metadata::save_metadata(addons_dir, &store)?;
    }

    Ok(results)
}

fn update_check_status_message(results: &[NativeAddonUpdateCheck]) -> String {
    let checked = results.len();
    let updates = results.iter().filter(|result| result.has_update).count();

    if checked == 0 {
        "No ESOUI-linked addons were found to check.".to_string()
    } else if updates == 0 {
        format!("Checked {checked} ESOUI addons; no updates available.")
    } else {
        format!(
            "Checked {checked} ESOUI addons; {updates} update{} available.",
            if updates == 1 { "" } else { "s" }
        )
    }
}

fn slint_update_check_entry(result: NativeAddonUpdateCheck) -> AddonUpdateCheckEntry {
    AddonUpdateCheckEntry {
        folder_name: result.folder_name.into(),
        remote_version: result.remote_version.into(),
        has_update: result.has_update,
        last_updated: if result.remote_last_update > 0 {
            date_label_from_epoch_millis(result.remote_last_update).into()
        } else {
            "".into()
        },
    }
}

fn slint_update_check_model(
    results: Vec<NativeAddonUpdateCheck>,
) -> ModelRc<AddonUpdateCheckEntry> {
    Rc::new(VecModel::from(
        results
            .into_iter()
            .map(slint_update_check_entry)
            .collect::<Vec<_>>(),
    ))
    .into()
}

fn start_native_addon_update_check(ui_weak: slint::Weak<KalpaWindow>, addons_dir: PathBuf) {
    std::thread::spawn(move || {
        let result = check_native_addon_updates_blocking(&addons_dir);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            match result {
                Ok(results) => {
                    let message = update_check_status_message(&results);
                    ui.invoke_addon_update_check_finished(
                        slint_update_check_model(results),
                        message.into(),
                    );
                }
                Err(error) => {
                    ui.set_checking_updates(false);
                    ui.set_status_error_message(format!("Update check failed: {error}").into());
                }
            }
        });
    });
}

fn start_native_addon_update_apply(
    ui_weak: slint::Weak<KalpaWindow>,
    addons_dir: PathBuf,
    targets: Vec<NativeAddonUpdateTarget>,
    conflict_policy: i32,
    eso_running: bool,
) {
    let pending_store = pending_conflict_store();
    std::thread::spawn(move || {
        let result = apply_native_addon_updates_blocking(&addons_dir, targets, conflict_policy);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            match result {
                Ok(result) => {
                    if let Some(store) = pending_store.as_ref() {
                        replace_pending_conflicts(store, result.conflicts.clone());
                    }
                    let message = addon_write_status_message(
                        update_apply_status_message(&result),
                        eso_running,
                    );
                    let conflict_count = result.conflicts.len() as i32;
                    ui.invoke_addon_update_apply_finished(
                        slint_update_check_model(result.checks),
                        message.into(),
                        conflict_count,
                    );
                }
                Err(error) => {
                    ui.set_checking_updates(false);
                    ui.set_status_error_message(format!("Update failed: {error}").into());
                }
            }
        });
    });
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NativeConflictDiffPreview {
    user_preview: String,
    upstream_preview: String,
    binary: bool,
}

fn start_native_pending_conflict_apply(
    ui_weak: slint::Weak<KalpaWindow>,
    addons_dir: PathBuf,
    pending: NativePendingConflict,
    eso_running: bool,
) {
    let pending_store = pending_conflict_store();
    std::thread::spawn(move || {
        let folder_name = pending.folder_name.clone();
        let result = apply_native_pending_conflict_blocking(&addons_dir, &pending);
        let conflict_count = if result.is_ok() {
            if let Some(store) = pending_store.as_ref() {
                if let Ok(mut conflicts) = store.lock() {
                    conflicts.remove(&folder_name);
                    conflicts.len()
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            pending_store
                .as_ref()
                .and_then(|store| store.lock().ok().map(|conflicts| conflicts.len()))
                .unwrap_or_default()
        };

        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            match result {
                Ok(result) => {
                    let message = addon_write_status_message(
                        update_apply_status_message(&result),
                        eso_running,
                    );
                    ui.invoke_addon_update_apply_finished(
                        slint_update_check_model(result.checks),
                        message.into(),
                        conflict_count as i32,
                    );
                    refresh_active_conflict_panel(&ui);
                }
                Err(error) => {
                    ui.set_checking_updates(false);
                    ui.set_status_error_message(format!("Conflict update failed: {error}").into());
                    refresh_active_conflict_panel(&ui);
                }
            }
        });
    });
}

fn apply_native_pending_conflict_blocking(
    addons_dir: &Path,
    pending: &NativePendingConflict,
) -> Result<NativeAddonUpdateApplyResult, String> {
    apply_native_pending_conflict_files_blocking(addons_dir, pending)?;

    let checks = check_native_addon_updates_blocking(addons_dir).unwrap_or_else(|_| {
        vec![NativeAddonUpdateCheck {
            folder_name: pending.folder_name.clone(),
            remote_version: pending.update_version.clone(),
            has_update: false,
            remote_last_update: 0,
        }]
    });

    Ok(NativeAddonUpdateApplyResult {
        checks,
        completed: vec![pending.folder_name.clone()],
        conflicts: Vec::new(),
        failed: Vec::new(),
        errors: Vec::new(),
    })
}

fn apply_native_pending_conflict_files_blocking(
    addons_dir: &Path,
    pending: &NativePendingConflict,
) -> Result<Vec<String>, String> {
    if !pending_conflict_all_decided(pending) {
        return Err("Every conflicted file needs a decision.".to_string());
    }

    let mut kept_files = pending.auto_kept_files.clone();
    let mut files_to_backup = Vec::new();
    for relative_path in &pending.conflicts {
        match pending.decisions.get(relative_path).copied() {
            Some(1) => kept_files.push(relative_path.clone()),
            Some(2) => files_to_backup.push(relative_path.clone()),
            _ => return Err(format!("Missing conflict decision for {relative_path}.")),
        }
    }
    kept_files.sort();
    kept_files.dedup();

    if !files_to_backup.is_empty() {
        let from_version = file_hashes::load_hash_manifest(addons_dir, &pending.folder_name)
            .map(|manifest| manifest.installed_version)
            .unwrap_or_default();
        edit_backups::backup_user_files(
            addons_dir,
            &pending.folder_name,
            &files_to_backup,
            &from_version,
            &pending.update_version,
        )?;
    }

    let skip_files = kept_files
        .iter()
        .map(|path| format!("{}/{}", pending.folder_name, path))
        .collect::<HashSet<_>>();

    let installed_folders = if skip_files.is_empty() {
        installer::extract_addon_zip(&pending.zip_path, addons_dir)?
    } else {
        installer::extract_addon_zip_selective(&pending.zip_path, addons_dir, &skip_files)?
    };

    let zip_hashes = if pending.zip_hashes.is_empty() {
        file_hashes::hash_zip_entries(&pending.zip_path, &pending.folder_name)?
    } else {
        pending.zip_hashes.clone()
    };
    let hash_overrides = native_hash_overrides(&kept_files, &zip_hashes);
    file_hashes::record_hashes_with_zip_baseline(
        addons_dir,
        &pending.zip_path,
        &installed_folders,
        &pending.folder_name,
        &zip_hashes,
        pending.esoui_id,
        &pending.update_version,
        hash_overrides.as_ref(),
    )?;

    let mut store = metadata::load_metadata(addons_dir);
    remove_stale_native_metadata(&mut store, pending.esoui_id, &installed_folders);
    record_native_installed_folders(
        &mut store,
        addons_dir,
        &installed_folders,
        pending.esoui_id,
        &pending.update_version,
        &pending.title,
        &pending.download_url,
    );
    let _ = native_resolve_transitive_deps(addons_dir, &installed_folders, &mut store);
    metadata::save_metadata(addons_dir, &store)?;
    cleanup_pending_conflict_zip(pending);

    Ok(installed_folders)
}

fn native_pending_conflict_diff_blocking(
    addons_dir: &Path,
    pending: &NativePendingConflict,
    relative_path: &str,
) -> Result<NativeConflictDiffPreview, String> {
    if !pending.conflicts.iter().any(|path| path == relative_path) {
        return Err("Conflict file was not found.".to_string());
    }

    let user_path = addon_file_path(addons_dir, &pending.folder_name, relative_path)?;
    let user_bytes = fs::read(&user_path).unwrap_or_default();
    if bytes_look_binary(&user_bytes) {
        return Ok(NativeConflictDiffPreview {
            binary: true,
            ..NativeConflictDiffPreview::default()
        });
    }

    let file = fs::File::open(&pending.zip_path)
        .map_err(|error| format!("Failed to open pending update ZIP: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("Failed to read update ZIP: {error}"))?;
    let zip_entry_name = format!("{}/{}", pending.folder_name, relative_path);
    let mut entry = archive
        .by_name(&zip_entry_name)
        .map_err(|error| format!("File was not found in update ZIP: {error}"))?;

    const MAX_DIFF_BYTES: u64 = 2 * 1024 * 1024;
    if entry.size() > MAX_DIFF_BYTES {
        return Err("File is too large to preview differences.".to_string());
    }

    let mut upstream_bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut upstream_bytes)
        .map_err(|error| format!("Failed to read update ZIP entry: {error}"))?;
    if bytes_look_binary(&upstream_bytes) {
        return Ok(NativeConflictDiffPreview {
            binary: true,
            ..NativeConflictDiffPreview::default()
        });
    }

    Ok(NativeConflictDiffPreview {
        user_preview: text_preview(&String::from_utf8_lossy(&user_bytes)),
        upstream_preview: text_preview(&String::from_utf8_lossy(&upstream_bytes)),
        binary: false,
    })
}

fn bytes_look_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(512).any(|byte| *byte == 0)
}

fn text_preview(text: &str) -> String {
    const MAX_LINES: usize = 6;
    const MAX_CHARS: usize = 900;
    let mut preview = text.lines().take(MAX_LINES).collect::<Vec<_>>().join("\n");
    if preview.chars().count() > MAX_CHARS {
        preview = preview.chars().take(MAX_CHARS).collect::<String>();
        preview.push_str("\n...");
    } else if text.lines().count() > MAX_LINES {
        preview.push_str("\n...");
    }
    preview
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeSingleUpdateOutcome {
    Applied,
    Pending(NativePendingConflict),
}

fn apply_native_addon_updates_blocking(
    addons_dir: &Path,
    targets: Vec<NativeAddonUpdateTarget>,
    conflict_policy: i32,
) -> Result<NativeAddonUpdateApplyResult, String> {
    let initial_checks = check_native_addon_updates_blocking(addons_dir)?;
    let target_folders = targets
        .iter()
        .map(|target| target.folder_name.as_str())
        .collect::<HashSet<_>>();
    let remote_versions = initial_checks
        .iter()
        .filter(|check| check.has_update && target_folders.contains(check.folder_name.as_str()))
        .map(|check| (check.folder_name.clone(), check.remote_version.clone()))
        .collect::<HashMap<_, _>>();

    let mut result = NativeAddonUpdateApplyResult::default();
    let mut completed_set = HashSet::new();

    for target in targets {
        let Some(remote_version) = remote_versions.get(&target.folder_name) else {
            continue;
        };

        match apply_native_single_addon_update(addons_dir, &target, remote_version, conflict_policy)
        {
            Ok(NativeSingleUpdateOutcome::Applied) => {
                completed_set.insert(target.folder_name.clone());
                result.completed.push(target.folder_name);
            }
            Ok(NativeSingleUpdateOutcome::Pending(conflict)) => result.conflicts.push(conflict),
            Err(error) => {
                result.failed.push(target.folder_name.clone());
                result
                    .errors
                    .push(format!("{}: {error}", target.folder_name));
            }
        }
    }

    let mut fallback_checks = initial_checks;
    for check in &mut fallback_checks {
        if completed_set.contains(&check.folder_name) {
            check.has_update = false;
        }
    }
    result.checks = check_native_addon_updates_blocking(addons_dir).unwrap_or(fallback_checks);
    Ok(result)
}

fn apply_native_single_addon_update(
    addons_dir: &Path,
    target: &NativeAddonUpdateTarget,
    remote_version: &str,
    conflict_policy: i32,
) -> Result<NativeSingleUpdateOutcome, String> {
    let (zip, info) = native_fetch_and_download_with_retry(target.esoui_id)?;
    let (report, zip_hashes) =
        build_native_conflict_report(addons_dir, &target.folder_name, zip.path())?;

    if !report.conflicts.is_empty() && conflict_policy == 0 {
        let (_, zip_path) = zip
            .keep()
            .map_err(|error| format!("Failed to preserve update ZIP: {error}"))?;
        return Ok(NativeSingleUpdateOutcome::Pending(NativePendingConflict {
            folder_name: target.folder_name.clone(),
            esoui_id: target.esoui_id,
            update_version: remote_version.to_string(),
            title: info.title,
            download_url: info.download_url,
            safe_file_count: report.safe_file_count,
            auto_kept_files: report.auto_kept_files,
            conflicts: report.conflicts,
            decisions: HashMap::new(),
            zip_path,
            zip_hashes,
        }));
    }

    let Some(kept_files) = native_kept_files_for_policy(&report, conflict_policy) else {
        return Ok(NativeSingleUpdateOutcome::Pending(NativePendingConflict {
            folder_name: target.folder_name.clone(),
            esoui_id: target.esoui_id,
            update_version: remote_version.to_string(),
            title: info.title,
            download_url: info.download_url,
            safe_file_count: report.safe_file_count,
            auto_kept_files: report.auto_kept_files,
            conflicts: report.conflicts,
            decisions: HashMap::new(),
            zip_path: PathBuf::new(),
            zip_hashes,
        }));
    };
    if !report.conflicts.is_empty() && conflict_policy == 2 {
        backup_native_conflicting_files(addons_dir, target, remote_version, &report.conflicts)?;
    }
    let skip_files = kept_files
        .iter()
        .map(|path| format!("{}/{}", target.folder_name, path))
        .collect::<HashSet<_>>();

    let installed_folders = if skip_files.is_empty() {
        installer::extract_addon_zip(zip.path(), addons_dir)?
    } else {
        installer::extract_addon_zip_selective(zip.path(), addons_dir, &skip_files)?
    };

    let hash_overrides = native_hash_overrides(&kept_files, &zip_hashes);
    file_hashes::record_hashes_with_zip_baseline(
        addons_dir,
        zip.path(),
        &installed_folders,
        &target.folder_name,
        &zip_hashes,
        target.esoui_id,
        remote_version,
        hash_overrides.as_ref(),
    )?;

    let mut store = metadata::load_metadata(addons_dir);
    remove_stale_native_metadata(&mut store, target.esoui_id, &installed_folders);
    record_native_installed_folders(
        &mut store,
        addons_dir,
        &installed_folders,
        target.esoui_id,
        remote_version,
        &info.title,
        &info.download_url,
    );
    let _ = native_resolve_transitive_deps(addons_dir, &installed_folders, &mut store);
    metadata::save_metadata(addons_dir, &store)?;

    Ok(NativeSingleUpdateOutcome::Applied)
}

fn native_kept_files_for_policy(
    report: &NativeConflictReport,
    conflict_policy: i32,
) -> Option<Vec<String>> {
    if !report.conflicts.is_empty() && conflict_policy == 0 {
        return None;
    }

    let mut kept_files = report.auto_kept_files.clone();
    if !report.conflicts.is_empty() && conflict_policy == 1 {
        kept_files.extend(report.conflicts.clone());
        kept_files.sort();
        kept_files.dedup();
    }
    Some(kept_files)
}

fn backup_native_conflicting_files(
    addons_dir: &Path,
    target: &NativeAddonUpdateTarget,
    remote_version: &str,
    conflicts: &[String],
) -> Result<(), String> {
    let from_version = file_hashes::load_hash_manifest(addons_dir, &target.folder_name)
        .map(|manifest| manifest.installed_version)
        .unwrap_or_default();
    edit_backups::backup_user_files(
        addons_dir,
        &target.folder_name,
        conflicts,
        &from_version,
        remote_version,
    )
}

fn native_hash_overrides(
    kept_files: &[String],
    zip_hashes: &HashMap<String, String>,
) -> Option<HashMap<String, String>> {
    let overrides = kept_files
        .iter()
        .filter_map(|path| {
            zip_hashes
                .get(path)
                .map(|hash| (path.clone(), hash.clone()))
        })
        .collect::<HashMap<_, _>>();
    (!overrides.is_empty()).then_some(overrides)
}

fn remove_stale_native_metadata(
    store: &mut metadata::MetadataStore,
    esoui_id: u32,
    installed_folders: &[String],
) {
    let old_folders = store
        .addons
        .iter()
        .filter(|(_, meta)| meta.esoui_id == esoui_id)
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    for old in old_folders {
        if !installed_folders.contains(&old) {
            metadata::remove_entry(store, &old);
        }
    }
}

fn build_native_conflict_report(
    addons_dir: &Path,
    folder_name: &str,
    zip_path: &Path,
) -> Result<(NativeConflictReport, HashMap<String, String>), String> {
    let stored = file_hashes::load_hash_manifest(addons_dir, folder_name);
    let addon_path = addons_dir.join(folder_name);
    let disk_hashes = if stored.is_some() && addon_path.is_dir() {
        file_hashes::compute_addon_hashes(&addon_path)?
    } else {
        HashMap::new()
    };
    let zip_hashes = file_hashes::hash_zip_entries(zip_path, folder_name)?;
    let stored_files = stored.as_ref().map(|manifest| &manifest.files);

    let mut safe_file_count = 0;
    let mut auto_kept_files = Vec::new();
    let mut conflicts = Vec::new();

    for (relative_path, zip_hash) in &zip_hashes {
        let stored_hash = stored_files.and_then(|files| files.get(relative_path));
        let disk_hash = disk_hashes.get(relative_path);
        let user_modified = match (stored_hash, disk_hash) {
            (Some(stored), Some(disk)) => !file_hashes::signatures_match(stored, disk),
            (Some(_), None) => true,
            (None, _) => false,
        };
        let upstream_changed = match stored_hash {
            Some(stored) => stored != zip_hash,
            None => true,
        };

        match (user_modified, upstream_changed) {
            (false, _) => safe_file_count += 1,
            (true, false) => auto_kept_files.push(relative_path.clone()),
            (true, true) => conflicts.push(relative_path.clone()),
        }
    }

    auto_kept_files.sort();
    conflicts.sort();

    Ok((
        NativeConflictReport {
            safe_file_count,
            auto_kept_files,
            conflicts,
        },
        zip_hashes,
    ))
}

fn native_is_rate_limited(error: &str) -> bool {
    error.contains("Too many requests") || error.contains("HTTP 429")
}

fn native_fetch_and_download_with_retry(
    esoui_id: u32,
) -> Result<(tempfile::NamedTempFile, esoui::EsouiAddonInfo), String> {
    let mut last_error = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(1_000 * (1 << (attempt - 1))));
        }

        let info = match esoui::fetch_addon_info(esoui_id) {
            Ok(info) => info,
            Err(error) => {
                last_error = error.clone();
                if native_is_rate_limited(&error) {
                    continue;
                }
                return Err(error);
            }
        };

        match esoui::download_addon(&info.download_url, None) {
            Ok(zip) => return Ok((zip, info)),
            Err(error) => {
                last_error = error.clone();
                if native_is_rate_limited(&error) {
                    continue;
                }
                return Err(error);
            }
        }
    }

    Err(last_error)
}

fn native_normalize_addon_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn native_find_manifest_in(dir: &Path, base_name: &str) -> Option<PathBuf> {
    let txt = dir.join(format!("{base_name}.txt"));
    if txt.exists() {
        return Some(txt);
    }
    let addon = dir.join(format!("{base_name}.addon"));
    if addon.exists() {
        return Some(addon);
    }
    None
}

fn native_record_installed_name(dir: &Path, name: &str, installed: &mut HashSet<String>) {
    if native_find_manifest_in(dir, name).is_some() {
        installed.insert(native_normalize_addon_name(name));
    }
}

fn native_collect_subfolder_names(folder_path: &Path, installed: &mut HashSet<String>, depth: u8) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(folder_path) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            native_record_installed_name(&path, name, installed);
            native_collect_subfolder_names(&path, installed, depth - 1);
        }
    }
}

fn native_build_installed_set(addons_dir: &Path) -> HashSet<String> {
    let mut installed = HashSet::new();
    let Ok(entries) = fs::read_dir(addons_dir) else {
        return installed;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.ends_with(".disabled") {
            continue;
        }
        native_record_installed_name(&path, name, &mut installed);
        native_collect_subfolder_names(&path, &mut installed, 2);
    }

    installed
}

fn native_resolve_transitive_deps(
    addons_dir: &Path,
    installed_folders: &[String],
    store: &mut metadata::MetadataStore,
) -> NativeImportResult {
    let mut all_installed = native_build_installed_set(addons_dir);
    let mut result = NativeImportResult::default();
    let mut folders_to_scan = installed_folders.to_vec();
    let mut seen = HashSet::new();

    while !folders_to_scan.is_empty() {
        let mut missing_deps = Vec::new();
        for folder in &folders_to_scan {
            let addon = find_manifest(addons_dir, folder)
                .and_then(|path| manifest::parse_manifest(folder, &path));
            if let Some(addon) = addon {
                for dep in &addon.depends_on {
                    let key = native_normalize_addon_name(&dep.name);
                    if !all_installed.contains(&key) && seen.insert(key) {
                        missing_deps.push(dep.name.clone());
                    }
                }
            }
        }

        if missing_deps.is_empty() {
            break;
        }

        let mut newly_installed = Vec::new();
        for (index, dep_name) in missing_deps.iter().enumerate() {
            if index > 0 {
                std::thread::sleep(Duration::from_millis(200));
            }
            match native_try_install_dep(dep_name, addons_dir, store) {
                Ok(dep_folders) => {
                    for folder in &dep_folders {
                        if find_manifest(addons_dir, folder).is_some() {
                            all_installed.insert(native_normalize_addon_name(folder));
                        }
                        native_collect_subfolder_names(
                            &addons_dir.join(folder),
                            &mut all_installed,
                            2,
                        );
                        newly_installed.push(folder.clone());
                    }
                    result.installed.push(dep_name.clone());
                }
                Err("not_found") => result.skipped.push(dep_name.clone()),
                Err(_) => result.failed.push(dep_name.clone()),
            }
        }

        folders_to_scan = newly_installed;
    }

    result
}

fn native_try_install_dep(
    dep_name: &str,
    addons_dir: &Path,
    store: &mut metadata::MetadataStore,
) -> Result<Vec<String>, &'static str> {
    let dep_id = if let Some(meta) = store.addons.get(dep_name) {
        meta.esoui_id
    } else {
        match esoui::search_addon_by_name(dep_name) {
            Ok(Some(id)) => id,
            Ok(None) => return Err("not_found"),
            Err(_) => return Err("search_failed"),
        }
    };
    let dep_info = esoui::fetch_addon_info(dep_id).map_err(|_| "fetch_failed")?;
    let dep_zip =
        esoui::download_addon(&dep_info.download_url, None).map_err(|_| "download_failed")?;
    let dep_folders =
        installer::extract_addon_zip(dep_zip.path(), addons_dir).map_err(|_| "extract_failed")?;
    file_hashes::record_hashes_for_folders(addons_dir, &dep_folders, dep_id, &dep_info.version)
        .map_err(|_| "hash_record_failed")?;

    for folder in &dep_folders {
        let version = read_local_version(addons_dir, folder);
        metadata::record_install(store, folder, dep_id, &version, &dep_info.download_url);
    }

    Ok(dep_folders)
}

fn find_manifest(addons_dir: &Path, folder_name: &str) -> Option<PathBuf> {
    let folder_dir = addons_dir.join(folder_name);
    let txt = folder_dir.join(format!("{folder_name}.txt"));
    if txt.exists() {
        return Some(txt);
    }

    let addon = folder_dir.join(format!("{folder_name}.addon"));
    if addon.exists() {
        return Some(addon);
    }

    None
}

fn export_addon_list_json(addons_dir: &Path) -> Result<String, String> {
    let store = metadata::load_metadata(addons_dir);
    let mut entries = store
        .addons
        .iter()
        .filter(|(folder, _)| addons_dir.join(folder).is_dir())
        .map(|(folder, meta)| ExportEntry {
            esoui_id: meta.esoui_id,
            folder_name: folder.clone(),
            version: meta.installed_version.clone(),
        })
        .collect::<Vec<_>>();

    entries.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));

    let mut seen_ids = BTreeSet::new();
    entries.retain(|entry| entry.esoui_id == 0 || seen_ids.insert(entry.esoui_id));

    serde_json::to_string_pretty(&ExportData {
        version: 1,
        addons: entries,
    })
    .map_err(|error| format!("Failed to export addon list: {error}"))
}

fn import_addon_list_json(
    addons_dir: &Path,
    json_data: &str,
) -> Result<NativeImportResult, String> {
    let export = serde_json::from_str::<ExportData>(json_data)
        .map_err(|error| format!("Invalid addon list export: {error}"))?;
    let mut result = NativeImportResult::default();

    for entry in export.addons {
        if addons_dir.join(&entry.folder_name).is_dir() {
            result.skipped.push(entry.folder_name);
            continue;
        }

        if entry.esoui_id == 0 {
            result.failed.push(entry.folder_name);
            continue;
        }

        match import_addon_entry_blocking(addons_dir, &entry) {
            Ok(()) => result.installed.push(entry.folder_name),
            Err(_) => result.failed.push(entry.folder_name),
        }
    }

    Ok(result)
}

fn import_addon_entry_blocking(addons_dir: &Path, entry: &ExportEntry) -> Result<(), String> {
    let info = esoui::fetch_addon_info(entry.esoui_id)?;
    let tmp = esoui::download_addon(&info.download_url, None)?;
    install_downloaded_addon_blocking(
        addons_dir,
        tmp.path(),
        entry.esoui_id,
        &info.title,
        &info.version,
        &info.download_url,
    )
    .map(|_| ())
}

fn import_result_summary(result: &NativeImportResult) -> String {
    format!(
        "Import complete: {} installed, {} skipped, {} failed.",
        result.installed.len(),
        result.skipped.len(),
        result.failed.len()
    )
}

fn write_clipboard_text(text: String) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("Clipboard unavailable: {error}"))?;
    clipboard
        .set_text(text)
        .map_err(|error| format!("Failed to write clipboard: {error}"))
}

fn read_clipboard_text() -> Result<String, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("Clipboard unavailable: {error}"))?;
    clipboard
        .get_text()
        .map_err(|error| format!("Failed to read clipboard: {error}"))
}

fn check_native_api_compatibility(addons_dir: &Path) -> Result<NativeApiCompatInfo, String> {
    let game_api_version = read_game_api_version(addons_dir)?;
    let entries = fs::read_dir(addons_dir)
        .map_err(|error| format!("Failed to read AddOns folder: {error}"))?;
    let mut outdated_addons = Vec::new();
    let mut up_to_date_addons = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let Some(folder_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(manifest) = find_manifest(addons_dir, folder_name)
            .and_then(|path| manifest::parse_manifest(folder_name, &path))
        else {
            continue;
        };

        if manifest.api_version.is_empty() {
            continue;
        }

        if manifest.api_version.contains(&game_api_version) {
            up_to_date_addons.push(manifest.title);
        } else {
            outdated_addons.push(manifest.title);
        }
    }

    outdated_addons.sort();
    up_to_date_addons.sort();

    Ok(NativeApiCompatInfo {
        game_api_version,
        outdated_addons,
        up_to_date_addons,
    })
}

fn read_game_api_version(addons_dir: &Path) -> Result<u32, String> {
    let settings_path = addons_dir
        .parent()
        .map(|path| path.join("AddOnSettings.txt"))
        .ok_or_else(|| "Could not find AddOnSettings.txt.".to_string())?;
    let content = fs::read_to_string(&settings_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "AddOnSettings.txt not found. Launch ESO at least once.".to_string()
        } else {
            format!("Failed to read AddOnSettings.txt: {error}")
        }
    })?;

    content
        .lines()
        .find(|line| line.starts_with("#Version"))
        .and_then(|line| line.strip_prefix("#Version").map(str::trim))
        .and_then(|version| version.parse::<u32>().ok())
        .filter(|version| *version != 0)
        .ok_or_else(|| "Could not determine game API version.".to_string())
}

fn api_compat_summary(info: &NativeApiCompatInfo) -> String {
    if info.outdated_addons.is_empty() {
        return format!(
            "API {}: all {} checked addons are compatible.",
            info.game_api_version,
            info.up_to_date_addons.len()
        );
    }

    let sample = info
        .outdated_addons
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if info.outdated_addons.len() > 3 {
        format!(" and {} more", info.outdated_addons.len() - 3)
    } else {
        String::new()
    };

    format!(
        "API {}: {} compatible, {} outdated ({sample}{suffix}).",
        info.game_api_version,
        info.up_to_date_addons.len(),
        info.outdated_addons.len()
    )
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

fn validate_addon_folder_name(folder_name: &str) -> Result<(), String> {
    if folder_name.is_empty()
        || folder_name.contains("..")
        || folder_name.contains('/')
        || folder_name.contains('\\')
    {
        return Err("Invalid addon folder name.".to_string());
    }
    Ok(())
}

fn set_addon_disabled_on_disk(
    addons_root: &Path,
    folder_name: &str,
    disabled: bool,
) -> Result<(), String> {
    validate_addon_folder_name(folder_name)?;
    if disabled {
        let src = addons_root.join(folder_name);
        if !src.is_dir() {
            return Err(format!("Addon folder not found: {folder_name}"));
        }
        let dst = addons_root.join(format!("{folder_name}.disabled"));
        if dst.exists() {
            return Err(format!("{folder_name} is already disabled."));
        }
        fs::rename(&src, &dst).map_err(|error| format!("Failed to disable {folder_name}: {error}"))
    } else {
        let src = addons_root.join(format!("{folder_name}.disabled"));
        if !src.is_dir() {
            return Err(format!("Disabled addon folder not found: {folder_name}"));
        }
        let dst = addons_root.join(folder_name);
        if dst.exists() {
            return Err(format!("A folder named {folder_name} already exists."));
        }
        fs::rename(&src, &dst).map_err(|error| format!("Failed to enable {folder_name}: {error}"))
    }
}

fn remove_addon_from_disk(addons_root: &Path, folder_name: &str) -> Result<(), String> {
    validate_addon_folder_name(folder_name)?;

    let enabled_exists = addons_root.join(folder_name).is_dir();
    let disabled_name = format!("{folder_name}.disabled");
    let disabled_exists = addons_root.join(&disabled_name).is_dir();

    if enabled_exists {
        installer::remove_addon(addons_root, folder_name)?;
    }
    if disabled_exists {
        installer::remove_addon(addons_root, &disabled_name)?;
    }
    if !enabled_exists && !disabled_exists {
        return Err(format!("Addon folder not found: {folder_name}"));
    }

    let mut store = metadata::load_metadata(addons_root);
    metadata::remove_entry(&mut store, folder_name);
    metadata::save_metadata(addons_root, &store)
}

fn active_tag_ids(tags: &ModelRc<TagEntry>) -> Vec<String> {
    (0..tags.row_count())
        .filter_map(|index| tags.row_data(index))
        .filter(|tag| tag.active)
        .map(|tag| tag.id.to_string())
        .collect()
}

fn persist_addon_tags(
    addons_root: &Path,
    folder_name: &str,
    tags: Vec<String>,
) -> Result<(), String> {
    validate_addon_folder_name(folder_name)?;
    let mut store = metadata::load_metadata(addons_root);
    match store.addons.get_mut(folder_name) {
        Some(meta) => meta.tags = tags,
        None => {
            store.addons.insert(
                folder_name.to_string(),
                metadata::AddonMetadata {
                    esoui_id: 0,
                    installed_version: String::new(),
                    download_url: String::new(),
                    installed_at: String::new(),
                    tags,
                    esoui_last_update: 0,
                },
            );
        }
    }
    metadata::save_metadata(addons_root, &store)
}

fn persist_addon_tag_model(
    addons_root: &Path,
    folder_name: &str,
    tags: &ModelRc<TagEntry>,
) -> Result<(), String> {
    persist_addon_tags(addons_root, folder_name, active_tag_ids(tags))
}

fn disk_root_for_addon(folder_name: &str) -> Option<PathBuf> {
    configured_addons_path().filter(|root| resolve_addon_disk_path(root, folder_name).is_some())
}

fn mark_addon_disabled(entry: &mut AddonEntry, disabled: bool) {
    entry.disabled = disabled;
    if disabled {
        entry.badge3 = "Disabled".into();
        entry.badge3_kind = 5;
    } else if entry.badge3.as_str() == "Disabled" {
        entry.badge3 = "".into();
        entry.badge3_kind = 0;
    }
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

fn tag_model_from_ids(active_tags: &[String]) -> ModelRc<TagEntry> {
    let mut entries = preset_tag_entries(active_tags);
    let mut seen = PRESET_TAGS
        .iter()
        .map(|tag| (*tag).to_string())
        .collect::<BTreeSet<_>>();

    for raw_tag in active_tags {
        let Some(tag_id) = sanitize_custom_tag(raw_tag) else {
            continue;
        };
        if seen.contains(&tag_id) {
            continue;
        }
        seen.insert(tag_id.clone());
        entries.push(TagEntry {
            id: tag_id.as_str().into(),
            label: tag_id.as_str().into(),
            kind: 5,
            active: true,
            preset: false,
        });
    }

    tag_model_from_entries(entries)
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

    // Frameless window: initiate a native OS-level window move when the user
    // presses the header drag region. Without this the borderless window can
    // not be repositioned at all.
    let drag_ui = ui.as_weak();
    ui.on_window_drag_requested(move || {
        if let Some(ui) = drag_ui.upgrade() {
            use i_slint_backend_winit::WinitWindowAccessor;
            ui.window().with_winit_window(|winit_window| {
                let _ = winit_window.drag_window();
            });
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
            if guard_unsaved_editor(&ui) {
                return;
            }
            if needs_file_browser_refresh_on_addon_selection(
                ui.get_detail_files_active(),
                ui.get_editor_open(),
                ui.get_show_file_backups(),
            ) {
                refresh_file_browser(&ui);
            } else {
                refresh_active_conflict_panel(&ui);
            }
        }
    });

    let file_ui = ui.as_weak();
    ui.on_file_selected(move |relative_path| {
        if let Some(ui) = file_ui.upgrade() {
            if guard_unsaved_editor(&ui) {
                return;
            }
            let folder_name = selected_addon_folder(&ui);
            open_file_in_editor(&ui, &folder_name, relative_path.as_str());
        }
    });

    let folder_toggle_ui = ui.as_weak();
    ui.on_folder_toggled(move |relative_path| {
        if let Some(ui) = folder_toggle_ui.upgrade() {
            if guard_unsaved_editor(&ui) {
                return;
            }
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
            if guard_unsaved_editor(&ui) {
                return;
            }
            if let Some(addons_root) = addons_source_root() {
                let folder_name = selected_addon_folder(&ui);
                invalidate_file_entry_cache(&addons_root, &folder_name);
            }
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
                invalidate_file_entry_cache(&addons_root, &folder_name);
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
        .and_then(|addons_root| cached_file_entries(addons_root, &folder_name).ok())
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

    refresh_active_conflict_panel(ui);
}

fn needs_file_browser_refresh_on_addon_selection(
    files_active: bool,
    editor_open: bool,
    show_backups: bool,
) -> bool {
    files_active || editor_open || show_backups
}

fn pending_conflict_store() -> Option<NativePendingConflictStore> {
    PENDING_NATIVE_CONFLICTS.get().cloned()
}

fn selected_pending_conflict(ui: &KalpaWindow) -> Option<NativePendingConflict> {
    let folder_name = selected_addon_folder(ui);
    pending_conflict_store().and_then(|store| {
        store
            .lock()
            .ok()
            .and_then(|map| map.get(&folder_name).cloned())
    })
}

fn refresh_active_conflict_panel(ui: &KalpaWindow) {
    let Some(pending) = selected_pending_conflict(ui) else {
        clear_active_conflict_panel_if_needed(ui);
        return;
    };

    ui.set_detail_conflict_files(conflict_file_model(&pending));
    ui.set_detail_conflict_auto_kept_count(pending.auto_kept_files.len() as i32);
    ui.set_detail_conflict_safe_file_count(pending.safe_file_count as i32);
    ui.set_detail_conflict_update_version(pending.update_version.clone().into());
    ui.set_detail_conflict_all_decided(pending_conflict_all_decided(&pending));

    let diff_file = ui.get_detail_conflict_diff_file().to_string();
    if diff_file.is_empty() {
        return;
    }
    if !pending.conflicts.iter().any(|path| path == &diff_file) {
        clear_active_conflict_diff(ui);
    }
}

fn clear_active_conflict_panel_if_needed(ui: &KalpaWindow) {
    let has_conflict_state = ui.get_detail_conflict_files().row_count() > 0
        || ui.get_detail_conflict_auto_kept_count() != 0
        || ui.get_detail_conflict_safe_file_count() != 0
        || !ui.get_detail_conflict_update_version().is_empty()
        || ui.get_detail_conflict_all_decided()
        || !ui.get_detail_conflict_diff_file().is_empty()
        || !ui.get_detail_conflict_diff_user_preview().is_empty()
        || !ui.get_detail_conflict_diff_upstream_preview().is_empty()
        || ui.get_detail_conflict_diff_binary()
        || ui.get_detail_conflict_diff_loading();

    if has_conflict_state {
        clear_active_conflict_panel(ui);
    }
}

fn clear_active_conflict_panel(ui: &KalpaWindow) {
    ui.set_detail_conflict_files(Rc::new(VecModel::from(Vec::<ConflictFileEntry>::new())).into());
    ui.set_detail_conflict_auto_kept_count(0);
    ui.set_detail_conflict_safe_file_count(0);
    ui.set_detail_conflict_update_version("".into());
    ui.set_detail_conflict_all_decided(false);
    clear_active_conflict_diff(ui);
}

fn clear_active_conflict_diff(ui: &KalpaWindow) {
    ui.set_detail_conflict_diff_file("".into());
    ui.set_detail_conflict_diff_user_preview("".into());
    ui.set_detail_conflict_diff_upstream_preview("".into());
    ui.set_detail_conflict_diff_binary(false);
    ui.set_detail_conflict_diff_loading(false);
}

fn conflict_file_model(pending: &NativePendingConflict) -> ModelRc<ConflictFileEntry> {
    Rc::new(VecModel::from(
        pending
            .conflicts
            .iter()
            .map(|relative_path| ConflictFileEntry {
                relative_path: relative_path.clone().into(),
                decision: pending.decisions.get(relative_path).copied().unwrap_or(0),
            })
            .collect::<Vec<_>>(),
    ))
    .into()
}

fn pending_conflict_all_decided(pending: &NativePendingConflict) -> bool {
    !pending.conflicts.is_empty()
        && pending
            .conflicts
            .iter()
            .all(|path| matches!(pending.decisions.get(path), Some(1 | 2)))
}

fn sync_pending_conflict_count(ui: &KalpaWindow) {
    let count = pending_conflict_store()
        .and_then(|store| store.lock().ok().map(|map| map.len()))
        .unwrap_or_default();
    ui.set_pending_conflict_count(count as i32);
}

fn cleanup_pending_conflict_zip(pending: &NativePendingConflict) {
    if !pending.zip_path.as_os_str().is_empty() {
        let _ = fs::remove_file(&pending.zip_path);
    }
}

fn replace_pending_conflicts(
    store: &NativePendingConflictStore,
    conflicts: Vec<NativePendingConflict>,
) {
    if let Ok(mut pending) = store.lock() {
        for old in pending.drain().map(|(_, old)| old) {
            cleanup_pending_conflict_zip(&old);
        }
        for conflict in conflicts {
            pending.insert(conflict.folder_name.clone(), conflict);
        }
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
            let capture_dirty = env_flag("KALPA_FILE_EDITOR_DIRTY");
            let selected_content = if capture_dirty {
                format!("{content}\n-- unsaved local edit")
            } else {
                content.clone()
            };
            let editable = capture_dirty
                || env_flag("KALPA_FILE_EDITOR_EDITABLE")
                || file_entry.as_ref().is_some_and(|entry| entry.modified);
            ui.set_selected_file_content(selected_content.clone().into());
            ui.set_editor_line_numbers(line_numbers_for_content(&selected_content).into());
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
    let Some(addons_root) = addons_source_root() else {
        ui.set_editor_error(true);
        ui.set_editor_message(
            "Cannot save demo file. Configure the ESO AddOns folder to edit real addon files."
                .into(),
        );
        return;
    };

    if let Err(error) = write_text_file(&addons_root, &folder_name, relative_path, content) {
        ui.set_editor_error(true);
        ui.set_editor_message(format!("Failed to save: {error}").into());
        return;
    }

    invalidate_file_entry_cache(&addons_root, &folder_name);
    ui.set_selected_original_content(content.into());
    ui.set_editor_line_numbers(line_numbers_for_content(content).into());
    ui.set_editor_error(false);
    ui.set_editor_binary(false);
    ui.set_editor_editable(true);
    ui.set_editor_message(format!("Saved {}", file_name_from_path(relative_path)).into());
    mark_file_modified(ui, relative_path);
}

fn editor_has_unsaved_changes(ui: &KalpaWindow) -> bool {
    ui.get_editor_open()
        && !ui.get_editor_binary()
        && !ui.get_editor_error()
        && ui.get_selected_file_content() != ui.get_selected_original_content()
}

fn guard_unsaved_editor(ui: &KalpaWindow) -> bool {
    if !editor_has_unsaved_changes(ui) {
        return false;
    }

    ui.set_editor_message("Save or revert this file before switching away.".into());
    true
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

    if let Some(path) = read_persisted_addons_path().filter(|path| path.is_dir()) {
        return Some(path);
    }

    default_addons_root().filter(|path| path.is_dir())
}

fn configured_addons_path() -> Option<PathBuf> {
    addons_source_root().filter(|path| path.is_dir())
}

fn configured_addons_path_display() -> String {
    if let Some(path) = std::env::var_os("KALPA_ADDONS_PATH")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    {
        return path.to_string_lossy().into_owned();
    }

    if let Some(path) = read_persisted_addons_path() {
        return path.to_string_lossy().into_owned();
    }

    default_addons_root()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn read_persisted_addons_path() -> Option<PathBuf> {
    for path in native_settings_store_paths() {
        match read_addons_path_from_settings_path(&path) {
            Ok(Some(addons_path)) => return Some(addons_path),
            Ok(None) => {}
            Err(error) => eprintln!("Failed to read AddOns path from {path:?}: {error}"),
        }
    }

    None
}

fn read_addons_path_from_settings_path(path: &Path) -> Result<Option<PathBuf>, String> {
    let Some(value) = read_settings_store_key_from_path(path, STORE_KEY_ADDONS_PATH)? else {
        return Ok(None);
    };

    Ok(value
        .as_str()
        .map(str::trim)
        .filter(|addons_path| !addons_path.is_empty())
        .map(PathBuf::from))
}

fn persist_addons_path(addons_path: &str) -> Result<(), String> {
    let Some(path) = native_settings_store_path() else {
        return Err("settings store path was not available".to_string());
    };
    persist_addons_path_to_settings_path(&path, addons_path)
}

fn persist_addons_path_to_settings_path(path: &Path, addons_path: &str) -> Result<(), String> {
    let mut object = read_settings_store_object_from_path(path)?;
    object.insert(
        STORE_KEY_ADDONS_PATH.to_string(),
        serde_json::Value::String(addons_path.to_string()),
    );
    write_settings_store_object_to_path(path, object)
}

fn read_installed_pack_refs() -> Vec<NativeInstalledPackRef> {
    for path in native_settings_store_paths() {
        match read_installed_pack_refs_from_settings_path(&path) {
            Ok(Some(refs)) => return refs,
            Ok(None) => {}
            Err(error) => eprintln!("Failed to read installed packs from {path:?}: {error}"),
        }
    }

    Vec::new()
}

fn read_installed_pack_refs_from_settings_path(
    path: &Path,
) -> Result<Option<Vec<NativeInstalledPackRef>>, String> {
    let Some(value) = read_settings_store_key_from_path(path, STORE_KEY_INSTALLED_PACKS)? else {
        return Ok(None);
    };

    serde_json::from_value::<Vec<NativeInstalledPackRef>>(value)
        .map(normalize_installed_pack_refs)
        .map(Some)
        .map_err(|error| format!("Failed to parse installed packs: {error}"))
}

fn persist_installed_pack_refs(refs: &[NativeInstalledPackRef]) -> Result<(), String> {
    let Some(path) = native_settings_store_path() else {
        return Err("settings store path was not available".to_string());
    };
    persist_installed_pack_refs_to_settings_path(&path, refs)
}

fn persist_installed_pack_refs_to_settings_path(
    path: &Path,
    refs: &[NativeInstalledPackRef],
) -> Result<(), String> {
    let mut object = read_settings_store_object_from_path(path)?;
    object.insert(
        STORE_KEY_INSTALLED_PACKS.to_string(),
        serde_json::to_value(normalize_installed_pack_refs(refs.to_vec()))
            .map_err(|error| format!("Failed to serialize installed packs: {error}"))?,
    );
    write_settings_store_object_to_path(path, object)
}

fn normalize_installed_pack_refs(refs: Vec<NativeInstalledPackRef>) -> Vec<NativeInstalledPackRef> {
    let mut seen = HashSet::new();
    refs.into_iter()
        .filter_map(|mut reference| {
            reference.pack_id = reference.pack_id.trim().to_string();
            reference.title = reference.title.trim().to_string();
            if reference.pack_id.is_empty()
                || reference.title.is_empty()
                || !seen.insert(reference.pack_id.clone())
            {
                return None;
            }
            reference.pack_type = normalize_pack_type_key(&reference.pack_type);
            reference.author_name = reference.author_name.trim().to_string();
            Some(reference)
        })
        .collect()
}

fn upsert_installed_pack_ref(entry: &PackHubEntry) -> Result<Vec<NativeInstalledPackRef>, String> {
    if entry.id.trim().is_empty() {
        return Err("Pack Hub pack has no id.".to_string());
    }

    let reference = NativeInstalledPackRef {
        pack_id: entry.id.trim().to_string(),
        title: entry.title.trim().to_string(),
        pack_type: pack_type_key_from_kind(entry.type_kind).to_string(),
        author_name: entry.author.trim().to_string(),
        addon_count: parse_addon_count_label(entry.addon_count.as_str()),
        installed_at: current_iso_utc(),
    };

    let mut refs = read_installed_pack_refs();
    refs.retain(|existing| existing.pack_id != reference.pack_id);
    refs.insert(0, reference);
    persist_installed_pack_refs(&refs)?;
    Ok(refs)
}

fn remove_installed_pack_ref(pack_id: &str) -> Result<Vec<NativeInstalledPackRef>, String> {
    let mut refs = read_installed_pack_refs();
    let before = refs.len();
    refs.retain(|reference| reference.pack_id != pack_id);
    if refs.len() != before {
        persist_installed_pack_refs(&refs)?;
    }
    Ok(refs)
}

fn parse_addon_count_label(label: &str) -> usize {
    label
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
}

fn current_iso_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let days = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
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

fn cached_file_entries(addons_root: &Path, folder_name: &str) -> Result<Vec<FileEntry>, String> {
    let key = file_entry_cache_key(addons_root, folder_name);
    if let Some(files) = FILE_ENTRY_CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return Ok(files);
    }

    let files = real_file_entries(addons_root, folder_name)?;
    FILE_ENTRY_CACHE.with(|cache| {
        cache.borrow_mut().insert(key, files.clone());
    });
    Ok(files)
}

fn invalidate_file_entry_cache(addons_root: &Path, folder_name: &str) {
    let key = file_entry_cache_key(addons_root, folder_name);
    FILE_ENTRY_CACHE.with(|cache| {
        cache.borrow_mut().remove(&key);
    });
}

fn clear_file_entry_cache() {
    FILE_ENTRY_CACHE.with(|cache| cache.borrow_mut().clear());
}

fn file_entry_cache_key(addons_root: &Path, folder_name: &str) -> String {
    format!(
        "{}\n{}",
        addons_root.to_string_lossy(),
        folder_name.to_ascii_lowercase()
    )
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

struct SettingsBackupSnapshot {
    created_epoch: u64,
    total_size: u64,
    kind: i32,
    entry: SettingsBackupEntry,
}

fn apply_backup_restore_model(ui: &KalpaWindow) {
    let mut snapshots = addons_source_root()
        .and_then(|addons_root| settings_backup_snapshots(&addons_root).ok())
        .unwrap_or_default();
    let total_size = snapshots
        .iter()
        .map(|snapshot| snapshot.total_size)
        .sum::<u64>();
    let latest_index = snapshots
        .iter()
        .position(|snapshot| snapshot.kind != BACKUP_KIND_SAFETY);
    let now = unix_now_secs();

    let (status_kind, status_title, status_subtitle) = match latest_index {
        None => (
            0,
            "No backup yet".to_string(),
            "Your addon settings aren't protected. Create your first backup below.".to_string(),
        ),
        Some(index) => {
            snapshots[index].entry.latest = true;
            let latest = &snapshots[index];
            let age_secs = now.saturating_sub(latest.created_epoch);
            if age_secs > 14 * 86_400 {
                (
                    1,
                    "Last backup was a while ago".to_string(),
                    format!(
                        "Most recent: {}. Consider making a fresh one.",
                        latest.entry.meta
                    ),
                )
            } else {
                (
                    2,
                    "Your settings are protected".to_string(),
                    format!("Last backup {}", latest.entry.meta),
                )
            }
        }
    };

    let count = snapshots.len();
    let entries = snapshots
        .into_iter()
        .map(|snapshot| snapshot.entry)
        .collect::<Vec<_>>();
    ui.set_backup_status_kind(status_kind);
    ui.set_backup_status_title(status_title.into());
    ui.set_backup_status_subtitle(status_subtitle.into());
    ui.set_backup_list_summary(format!("{count} - {} total", format_size(total_size)).into());
    ui.set_settings_backups(Rc::new(VecModel::from(entries)).into());
}

#[derive(Debug, Clone)]
struct NativeCharacterInfo {
    server: String,
    name: String,
    recovered: bool,
}

#[derive(Debug, Clone)]
struct NativeCharacterRoster {
    characters: Vec<NativeCharacterInfo>,
    skipped_files: u32,
}

const UNKNOWN_SERVER: &str = "Unknown";

fn apply_character_roster_model(ui: &KalpaWindow) {
    let Some(addons_root) = configured_addons_path() else {
        ui.set_characters(Rc::new(VecModel::from(Vec::<CharacterEntry>::new())).into());
        ui.set_characters_summary(
            "Configure the ESO AddOns folder before loading characters.".into(),
        );
        ui.set_characters_warning("".into());
        return;
    };

    match native_character_roster(&addons_root) {
        Ok(mut roster) => {
            roster.characters.sort_by(|left, right| {
                let left_unknown = left.server == UNKNOWN_SERVER;
                let right_unknown = right.server == UNKNOWN_SERVER;
                left_unknown
                    .cmp(&right_unknown)
                    .then_with(|| left.server.cmp(&right.server))
                    .then_with(|| left.name.cmp(&right.name))
            });

            let summary = character_roster_summary(&roster.characters);
            let warning = if roster.skipped_files == 0 {
                String::new()
            } else {
                format!(
                    "{} SavedVariables file{} could not be fully read; a character may be missing.",
                    roster.skipped_files,
                    if roster.skipped_files == 1 { "" } else { "s" }
                )
            };
            let entries = roster
                .characters
                .into_iter()
                .map(|character| CharacterEntry {
                    backup_label: default_character_backup_name(&character.name, &character.server)
                        .into(),
                    server: character.server.into(),
                    name: character.name.into(),
                    recovered: character.recovered,
                })
                .collect::<Vec<_>>();
            ui.set_characters_summary(summary.into());
            ui.set_characters_warning(warning.into());
            ui.set_characters(Rc::new(VecModel::from(entries)).into());
        }
        Err(error) => {
            ui.set_characters(Rc::new(VecModel::from(Vec::<CharacterEntry>::new())).into());
            ui.set_characters_summary("Characters could not be loaded.".into());
            ui.set_characters_warning("".into());
            ui.set_status_error_message(format!("Failed to load characters: {error}").into());
        }
    }
}

fn character_roster_summary(characters: &[NativeCharacterInfo]) -> String {
    if characters.is_empty() {
        return "No characters found.".to_string();
    }
    let servers = characters
        .iter()
        .map(|character| character.server.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    format!(
        "{} character{} across {} server{}",
        characters.len(),
        if characters.len() == 1 { "" } else { "s" },
        servers,
        if servers == 1 { "" } else { "s" }
    )
}

fn native_character_roster(addons_root: &Path) -> Result<NativeCharacterRoster, String> {
    let addon_settings = match addons_root
        .parent()
        .map(|parent| parent.join("AddOnSettings.txt"))
    {
        Some(path) => match fs::read_to_string(&path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("Failed to read AddOnSettings.txt: {error}")),
        },
        None => None,
    };
    let (sv_chars, skipped_files) = collect_native_roster_characters(addons_root);
    Ok(NativeCharacterRoster {
        characters: build_native_character_list(addon_settings.as_deref(), &sv_chars),
        skipped_files: skipped_files.min(u32::MAX as usize) as u32,
    })
}

fn normalize_character_name(name: &str) -> String {
    name.split('^').next().unwrap_or(name).trim().to_string()
}

fn build_native_character_list(
    addon_settings: Option<&str>,
    sv_chars: &[(String, Option<String>)],
) -> Vec<NativeCharacterInfo> {
    let mut characters = Vec::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
    let mut known_names: HashSet<String> = HashSet::new();

    if let Some(content) = addon_settings {
        let skip_prefixes = [
            "Version",
            "Acknowledged",
            "AddOnsEnabled",
            "LoadOutOfDateAddOns",
        ];
        for line in content.lines() {
            let Some(line) = line.strip_prefix('#') else {
                continue;
            };
            if line.starts_with('$') || skip_prefixes.iter().any(|prefix| line.starts_with(prefix))
            {
                continue;
            }
            let Some(pos) = line.find('-') else {
                continue;
            };
            let server = line[..pos].trim().to_string();
            let name = line[pos + 1..].trim().to_string();
            if server.is_empty() || name.is_empty() {
                continue;
            }
            let normalized = normalize_character_name(&name);
            if seen_pairs.insert((server.clone(), normalized.clone())) {
                known_names.insert(normalized);
                characters.push(NativeCharacterInfo {
                    server,
                    name,
                    recovered: false,
                });
            }
        }
    }

    let mut recovered = Vec::new();
    for (raw, world) in sv_chars {
        let name = normalize_character_name(raw);
        if name.is_empty()
            || name.starts_with('$')
            || name.bytes().all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let server = world
            .as_deref()
            .filter(|world| char_backup::WELL_KNOWN_WORLDS.contains(world))
            .map(str::to_string);
        recovered.push((name, server));
    }
    recovered.sort_by_key(|(_, server)| server.is_none());

    for (name, server) in recovered {
        match server {
            Some(server) => {
                if seen_pairs.insert((server.clone(), name.clone())) {
                    known_names.insert(name.clone());
                    characters.push(NativeCharacterInfo {
                        server,
                        name,
                        recovered: true,
                    });
                }
            }
            None => {
                if known_names.insert(name.clone()) {
                    characters.push(NativeCharacterInfo {
                        server: UNKNOWN_SERVER.to_string(),
                        name,
                        recovered: true,
                    });
                }
            }
        }
    }

    characters
}

fn collect_native_roster_characters(addons_root: &Path) -> (Vec<(String, Option<String>)>, usize) {
    const ROSTER_TOTAL_MAX: usize = 100_000;

    let sv_dir = settings_saved_variables_dir(addons_root);
    let entries = match fs::read_dir(&sv_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), 0),
        Err(_) => return (Vec::new(), 1),
    };

    let mut known_worlds: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut all_names: BTreeSet<String> = BTreeSet::new();
    let mut skipped = 0usize;
    let mut aggregate_truncated = false;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("lua") {
            continue;
        }
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let scan = match saved_variables::roster_stream::extract_roster_characters_streaming(
            std::io::BufReader::new(file),
        ) {
            Ok(scan) => scan,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if scan.malformed {
            skipped += 1;
        }
        for (name, world) in scan.characters {
            if all_names.contains(&name) {
                if let Some(world) = world {
                    known_worlds.entry(name).or_default().insert(world);
                }
            } else if all_names.len() < ROSTER_TOTAL_MAX {
                all_names.insert(name.clone());
                if let Some(world) = world {
                    known_worlds.entry(name).or_default().insert(world);
                }
            } else {
                aggregate_truncated = true;
            }
        }
    }

    if aggregate_truncated {
        skipped += 1;
    }

    let mut out = Vec::new();
    for name in all_names {
        match known_worlds.get(&name) {
            Some(worlds) if !worlds.is_empty() => {
                for world in worlds {
                    out.push((name.clone(), Some(world.clone())));
                }
            }
            _ => out.push((name, None)),
        }
    }
    (out, skipped)
}

fn apply_safety_center_model(ui: &KalpaWindow) {
    let Some(addons_root) = configured_addons_path() else {
        ui.set_safety_snapshots(Rc::new(VecModel::from(Vec::<SafetySnapshotEntry>::new())).into());
        ui.set_safety_logs(Rc::new(VecModel::from(Vec::<SafetyLogEntry>::new())).into());
        ui.set_safety_snapshot_summary(
            "Configure the ESO AddOns folder before loading Safety Center.".into(),
        );
        return;
    };

    let snapshots = safe_migration::list_snapshots(&addons_root);
    let snapshot_summary = if snapshots.is_empty() {
        "No snapshots yet.".to_string()
    } else {
        let total_size = snapshots
            .iter()
            .map(|snapshot| snapshot.total_size)
            .sum::<u64>();
        format!(
            "{} snapshot{} - {} total",
            snapshots.len(),
            if snapshots.len() == 1 { "" } else { "s" },
            format_size(total_size)
        )
    };
    let snapshot_entries = snapshots
        .into_iter()
        .map(safety_snapshot_entry)
        .collect::<Vec<_>>();
    ui.set_safety_snapshot_summary(snapshot_summary.into());
    ui.set_safety_snapshots(Rc::new(VecModel::from(snapshot_entries)).into());

    let mut logs = safe_migration::read_ops_log(&addons_root);
    logs.reverse();
    let log_entries = logs.into_iter().map(safety_log_entry).collect::<Vec<_>>();
    ui.set_safety_logs(Rc::new(VecModel::from(log_entries)).into());
}

fn safety_snapshot_entry(snapshot: safe_migration::SnapshotManifest) -> SafetySnapshotEntry {
    SafetySnapshotEntry {
        id: snapshot.id.into(),
        label: snapshot.label.into(),
        meta: format!(
            "{} file{} - {} - {}",
            snapshot.file_count,
            if snapshot.file_count == 1 { "" } else { "s" },
            format_size(snapshot.total_size),
            snapshot.created_at
        )
        .into(),
        sources: if snapshot.source_paths.is_empty() {
            "Includes: no source paths recorded".into()
        } else {
            format!("Includes: {}", snapshot.source_paths.join(", ")).into()
        },
    }
}

fn safety_log_entry(entry: safe_migration::OpLogEntry) -> SafetyLogEntry {
    SafetyLogEntry {
        operation: entry.operation.into(),
        status: entry.status.clone().into(),
        details: entry.details.into(),
        timing: format!("{} -> {}", entry.started_at, entry.finished_at).into(),
        snapshot_id: entry.snapshot_id.unwrap_or_default().into(),
        success: entry.status == "success",
    }
}

fn apply_safety_integrity_result(ui: &KalpaWindow, result: safe_migration::IntegrityResult) {
    let issues = result
        .issues
        .iter()
        .map(|issue| SafetyIssueEntry {
            message: issue.clone().into(),
        })
        .collect::<Vec<_>>();
    let summary = if issues.is_empty() {
        "All checks passed. No issues found.".to_string()
    } else {
        format!(
            "{} issue{} found.",
            issues.len(),
            if issues.len() == 1 { "" } else { "s" }
        )
    };
    ui.set_safety_integrity_addons_ok(result.addons_folder_ok);
    ui.set_safety_integrity_sv_ok(result.saved_variables_ok);
    ui.set_safety_integrity_addon_count(result.addon_count.to_string().into());
    ui.set_safety_integrity_summary(summary.into());
    ui.set_safety_issues(Rc::new(VecModel::from(issues)).into());
}

fn apply_migration_preconditions(ui: &KalpaWindow) {
    let Some(addons_root) = configured_addons_path() else {
        ui.set_migration_phase(0);
        ui.set_migration_can_proceed(false);
        ui.set_migration_status(
            "Configure the ESO AddOns folder before running Minion migration.".into(),
        );
        ui.set_migration_checks(Rc::new(VecModel::from(Vec::<MigrationCheckEntry>::new())).into());
        ui.set_migration_preview(
            Rc::new(VecModel::from(Vec::<MigrationPreviewEntry>::new())).into(),
        );
        return;
    };

    let preconditions = safe_migration::check_preconditions(&addons_root);
    let checks = vec![
        migration_check(
            "Minion installation detected",
            preconditions.minion_found,
            false,
        ),
        migration_check(
            "AddOns folder accessible",
            preconditions.addons_path_valid,
            false,
        ),
        migration_check(
            "SavedVariables folder found",
            preconditions.saved_variables_exists,
            false,
        ),
        migration_check("ESO is not running", !preconditions.eso_running, false),
        migration_check("Minion is not running", !preconditions.minion_running, true),
    ];
    let can_proceed = preconditions.minion_found
        && preconditions.addons_path_valid
        && preconditions.saved_variables_exists
        && !preconditions.eso_running;
    let status = if can_proceed {
        if preconditions.minion_running {
            "Ready, but close Minion first if you want the cleanest migration.".to_string()
        } else {
            "Ready to create the migration restore point.".to_string()
        }
    } else if preconditions.warnings.is_empty() {
        "Migration precheck has blockers.".to_string()
    } else {
        preconditions.warnings.join(" ")
    };

    ui.set_migration_phase(0);
    ui.set_migration_can_proceed(can_proceed);
    ui.set_migration_status(status.into());
    ui.set_migration_snapshot_summary("".into());
    ui.set_migration_result_summary("".into());
    ui.set_migration_checks(Rc::new(VecModel::from(checks)).into());
    ui.set_migration_preview(Rc::new(VecModel::from(Vec::<MigrationPreviewEntry>::new())).into());
}

fn migration_check(label: &str, ok: bool, warn: bool) -> MigrationCheckEntry {
    MigrationCheckEntry {
        label: label.into(),
        ok,
        warn,
    }
}

fn apply_migration_dry_run(ui: &KalpaWindow, dry_run: safe_migration::DryRunResult) {
    let will_track = dry_run.will_track.len();
    let already_tracked = dry_run.already_tracked.len();
    let missing = dry_run.missing_on_disk.len();
    let unmanaged = dry_run.unmanaged_on_disk.len();
    let mut entries = Vec::new();

    for addon in dry_run.will_track {
        entries.push(MigrationPreviewEntry {
            title: addon.folder_name.into(),
            meta: format!(
                "Will be tracked - ESOUI #{} - {}",
                addon.esoui_id, addon.minion_version
            )
            .into(),
            kind: 0,
        });
    }
    for addon in dry_run.already_tracked {
        entries.push(MigrationPreviewEntry {
            title: addon.folder_name.into(),
            meta: "Already tracked in Kalpa".into(),
            kind: 1,
        });
    }
    for addon in dry_run.missing_on_disk {
        entries.push(MigrationPreviewEntry {
            title: addon.folder_name.into(),
            meta: format!("In Minion but not on disk - ESOUI #{}", addon.esoui_id).into(),
            kind: 2,
        });
    }
    for folder in dry_run.unmanaged_on_disk {
        entries.push(MigrationPreviewEntry {
            title: folder.into(),
            meta: "On disk but unmanaged; will remain as-is".into(),
            kind: 3,
        });
    }

    ui.set_migration_status(
        format!(
            "Preview complete: {will_track} to import, {already_tracked} already tracked, {missing} missing, {unmanaged} unmanaged."
        )
        .into(),
    );
    ui.set_migration_preview(Rc::new(VecModel::from(entries)).into());
}

const BACKUP_KIND_MANUAL: i32 = 0;
const BACKUP_KIND_SAFETY: i32 = 1;
const BACKUP_KIND_CHARACTER: i32 = 2;
const CHAR_BACKUP_MARKER: &str = ".kalpa-char-backup";
const CHAR_BACKUP_MARKER_V2_BODY: &[u8] = b"kalpa character backup v2\n";
const CHAR_BACKUP_MARKER_V2_PREFIX: &[u8] = b"kalpa character backup v2";
const CHAR_BACKUP_META: &str = ".kalpa-char-backup.json";
const CHAR_BACKUP_VERSION: u32 = 2;
static CHARACTER_BACKUP_SEQ: AtomicU64 = AtomicU64::new(0);
static CHARACTER_BACKUP_FINALIZE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CharBackupMeta {
    version: u32,
    character: String,
    server: String,
}

enum CharRestoreMode {
    Merge(CharBackupMeta),
    WholeFile,
    Refuse(String),
}

fn settings_backup_snapshots(addons_root: &Path) -> Result<Vec<SettingsBackupSnapshot>, String> {
    let backups_dir = settings_backups_dir(addons_root);
    if !backups_dir.is_dir() {
        return Ok(Vec::new());
    }
    recover_orphaned_backups(&backups_dir);

    let now = unix_now_secs();
    let mut snapshots = fs::read_dir(&backups_dir)
        .map_err(|error| format!("Failed to read backups folder: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| settings_backup_snapshot(&entry.path(), now))
        .collect::<Vec<_>>();

    snapshots.sort_by(|left, right| {
        right
            .created_epoch
            .cmp(&left.created_epoch)
            .then_with(|| left.entry.name.cmp(&right.entry.name))
    });
    Ok(snapshots)
}

fn settings_backup_snapshot(path: &Path, now: u64) -> Option<SettingsBackupSnapshot> {
    let name = path.file_name()?.to_string_lossy().to_string();
    if name.starts_with('.') {
        return None;
    }

    let (file_count, total_size) = backup_file_count_and_size(path);
    let created_epoch = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let kind = backup_kind(&name);
    let display_name = backup_display_name(&name, kind);
    let detail = backup_detail(file_count, total_size);
    let restorable = !matches!(
        classify_backup_for_restore(path),
        CharRestoreMode::Refuse(_)
    );

    Some(SettingsBackupSnapshot {
        created_epoch,
        total_size,
        kind,
        entry: SettingsBackupEntry {
            name: name.into(),
            display_name: display_name.into(),
            kind_label: backup_kind_label(kind).into(),
            kind,
            meta: format!(
                "{} - {}",
                format_backup_relative_time(created_epoch, now),
                detail
            )
            .into(),
            detail: detail.into(),
            file_count: file_count.min(i32::MAX as u32) as i32,
            total_size_label: format_size(total_size).into(),
            latest: false,
            restorable,
        },
    })
}

fn backup_kind(name: &str) -> i32 {
    if name.starts_with("auto-before-restore-") {
        BACKUP_KIND_SAFETY
    } else if name.starts_with("char-") {
        BACKUP_KIND_CHARACTER
    } else {
        BACKUP_KIND_MANUAL
    }
}

fn backup_kind_label(kind: i32) -> &'static str {
    match kind {
        BACKUP_KIND_SAFETY => "Safety snapshot",
        BACKUP_KIND_CHARACTER => "Character",
        _ => "Manual",
    }
}

fn backup_display_name(name: &str, kind: i32) -> String {
    match kind {
        BACKUP_KIND_SAFETY => "Auto-saved before restore".to_string(),
        BACKUP_KIND_CHARACTER => name.strip_prefix("char-").unwrap_or(name).to_string(),
        _ => name.to_string(),
    }
}

fn backup_detail(file_count: u32, total_size: u64) -> String {
    format!(
        "{} file{} - {}",
        file_count,
        if file_count == 1 { "" } else { "s" },
        format_size(total_size)
    )
}

fn backup_file_count_and_size(path: &Path) -> (u32, u64) {
    let mut file_count = 0u32;
    let mut total_size = 0u64;
    let Ok(entries) = fs::read_dir(path) else {
        return (file_count, total_size);
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let dotfile = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with('.'))
            .unwrap_or(false);
        if dotfile {
            continue;
        }
        file_count += 1;
        total_size += fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
    }

    (file_count, total_size)
}

fn settings_backups_dir(addons_root: &Path) -> PathBuf {
    addons_root
        .parent()
        .unwrap_or(addons_root)
        .join("kalpa-backups")
}

fn settings_saved_variables_dir(addons_root: &Path) -> PathBuf {
    addons_root
        .parent()
        .unwrap_or(addons_root)
        .join("SavedVariables")
}

fn create_settings_backup(addons_root: &Path, requested_name: &str) -> Result<String, String> {
    let sv_dir = settings_saved_variables_dir(addons_root);
    if !sv_dir.is_dir() {
        return Err("SavedVariables folder not found.".to_string());
    }

    let backups_dir = settings_backups_dir(addons_root);
    fs::create_dir_all(&backups_dir)
        .map_err(|error| format!("Failed to create backups folder: {error}"))?;

    let backup_name = requested_name.trim();
    let backup_name = if backup_name.is_empty() {
        next_available_backup_name(&backups_dir, &friendly_backup_name(unix_now_secs()))
    } else {
        validate_backup_name(backup_name)?;
        backup_name.to_string()
    };
    validate_backup_name(&backup_name)?;
    let backup_path = backups_dir.join(&backup_name);
    if backup_path.exists() {
        return Err(format!("Backup '{backup_name}' already exists."));
    }

    fs::create_dir_all(&backup_path)
        .map_err(|error| format!("Failed to create backup: {error}"))?;
    let (file_count, total_size) = copy_directory_files(&sv_dir, &backup_path, false)?;
    Ok(backup_detail(file_count, total_size))
}

fn create_character_settings_backup(
    addons_root: &Path,
    character_name: &str,
    server: &str,
    requested_name: &str,
) -> Result<u32, String> {
    let character_name = character_name.trim();
    if character_name.is_empty() {
        return Err("Character name cannot be empty.".to_string());
    }
    if character_name.len() < 3 {
        return Err("Character name must be at least 3 characters.".to_string());
    }

    let backup_name = requested_name.trim();
    let backup_name = if backup_name.is_empty() {
        default_character_backup_name(character_name, server)
    } else {
        validate_character_backup_name(backup_name)?;
        backup_name.to_string()
    };
    validate_character_backup_name(&backup_name)?;

    let sv_dir = settings_saved_variables_dir(addons_root);
    if !sv_dir.is_dir() {
        return Err("SavedVariables folder not found.".to_string());
    }
    let world = if char_backup::WELL_KNOWN_WORLDS.contains(&server) {
        Some(server)
    } else {
        None
    };

    let backups_root = settings_backups_dir(addons_root);
    fs::create_dir_all(&backups_root)
        .map_err(|error| format!("Failed to create backups folder: {error}"))?;
    recover_orphaned_backups(&backups_root);

    let effective_name =
        resolve_character_backup_name(&backups_root, &backup_name).ok_or_else(|| {
            "Too many existing backups with this name; choose a different name.".to_string()
        })?;
    let final_dir = backups_root.join(format!("char-{effective_name}"));
    let seq = CHARACTER_BACKUP_SEQ.fetch_add(1, Ordering::Relaxed);
    let staging = backups_root.join(format!(".tmp-char-{effective_name}-{seq}"));
    let tombstone = backups_root.join(format!(".old-char-{effective_name}-{seq}"));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)
        .map_err(|error| format!("Failed to create backup folder: {error}"))?;

    let (matched, copied, last_copy_error) = match stage_character_subtrees(
        &sv_dir,
        character_name,
        world,
        &staging,
    ) {
        Ok(counts) => counts,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!(
                    "Could not read all SavedVariables files while backing up \"{character_name}\" ({error}). Close ESO and try again."
                ));
        }
    };

    if matched == 0 {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!(
            "No per-character SavedVariables data found for \"{character_name}\". This character may only use account-wide addon settings."
        ));
    }

    if copied < matched {
        let _ = fs::remove_dir_all(&staging);
        let detail = last_copy_error
            .map(|error| format!(" ({error})"))
            .unwrap_or_default();
        return Err(format!(
            "Backed up only {copied} of {matched} SavedVariables files for \"{character_name}\"; some files could not be saved{detail}."
        ));
    }

    if let Err(error) = fs::write(staging.join(CHAR_BACKUP_MARKER), CHAR_BACKUP_MARKER_V2_BODY) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("Failed to write backup marker: {error}"));
    }

    let meta = CharBackupMeta {
        version: CHAR_BACKUP_VERSION,
        character: character_name.to_string(),
        server: server.to_string(),
    };
    match serde_json::to_vec_pretty(&meta) {
        Ok(json) => {
            if let Err(error) = fs::write(staging.join(CHAR_BACKUP_META), json) {
                let _ = fs::remove_dir_all(&staging);
                return Err(format!("Failed to write backup metadata: {error}"));
            }
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("Failed to serialize backup metadata: {error}"));
        }
    }

    finalize_character_backup_replace(&staging, &final_dir, &tombstone)
        .map_err(|error| format!("Failed to finalize backup: {error}"))?;
    Ok(copied)
}

fn validate_character_backup_name(name: &str) -> Result<(), String> {
    validate_backup_name_for_lookup(name)?;
    if name.starts_with('.') || name.starts_with("auto-before-restore-") {
        return Err("Backup name uses a reserved prefix. Choose another name.".to_string());
    }
    Ok(())
}

fn default_character_backup_name(name: &str, server: &str) -> String {
    let safe_name = safe_backup_name_component(name);
    if server == UNKNOWN_SERVER {
        format!("{safe_name}-backup")
    } else {
        format!("{}-{}-backup", safe_name, character_server_tag(server))
    }
}

fn character_server_tag(server: &str) -> String {
    match server {
        "NA Megaserver" => "NA".to_string(),
        "EU Megaserver" => "EU".to_string(),
        "PTS" => "PTS".to_string(),
        other => safe_backup_name_component(other)
            .split_whitespace()
            .next()
            .filter(|tag| !tag.is_empty())
            .unwrap_or("Server")
            .to_string(),
    }
}

fn safe_backup_name_component(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '-'
            } else {
                character
            }
        })
        .collect::<String>();
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    if out.trim().is_empty() {
        "Character".to_string()
    } else {
        out
    }
}

fn stage_character_subtrees(
    sv_dir: &Path,
    character_name: &str,
    world: Option<&str>,
    staging: &Path,
) -> Result<(u32, u32, Option<String>), String> {
    let base = char_backup::char_base(character_name.as_bytes()).to_vec();
    let entries = fs::read_dir(sv_dir)
        .map_err(|error| format!("Failed to read SavedVariables folder: {error}"))?;
    let mut matched = 0u32;
    let mut copied = 0u32;
    let mut last_error = None;

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Failed to enumerate SavedVariables: {error}"))?;
        let path = entry.path();
        if !path.is_file()
            || path.extension().and_then(|extension| extension.to_str()) != Some("lua")
        {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("?")
            .to_string();
        let bytes =
            fs::read(&path).map_err(|error| format!("Could not read {file_name}: {error}"))?;
        let blocks = char_backup::extract_character_blocks(&bytes, &base, world);
        if blocks.is_empty() {
            continue;
        }
        matched += 1;
        match char_backup::build_backup_file(&blocks) {
            Ok(content) => match fs::write(staging.join(&file_name), &content) {
                Ok(()) => copied += 1,
                Err(error) => last_error = Some(error.to_string()),
            },
            Err(error) => last_error = Some(error),
        }
    }

    Ok((matched, copied, last_error))
}

fn character_backup_replaceable(final_dir: &Path) -> bool {
    !final_dir.exists() || final_dir.join(CHAR_BACKUP_MARKER).is_file()
}

fn resolve_character_backup_name(backups_root: &Path, backup_name: &str) -> Option<String> {
    if character_backup_replaceable(&backups_root.join(format!("char-{backup_name}"))) {
        return Some(backup_name.to_string());
    }
    (2..=999).find_map(|index| {
        let candidate = format!("{backup_name}-{index}");
        character_backup_replaceable(&backups_root.join(format!("char-{candidate}")))
            .then_some(candidate)
    })
}

fn finalize_character_backup_replace(
    staging: &Path,
    final_dir: &Path,
    tombstone: &Path,
) -> std::io::Result<()> {
    let _guard = CHARACTER_BACKUP_FINALIZE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let had_previous = final_dir.exists();
    if had_previous {
        let _ = fs::remove_dir_all(tombstone);
        fs::rename(final_dir, tombstone)?;
    }
    match fs::rename(staging, final_dir) {
        Ok(()) => {
            if had_previous {
                let _ = fs::remove_dir_all(tombstone);
            }
            Ok(())
        }
        Err(error) => {
            if had_previous {
                match fs::rename(tombstone, final_dir) {
                    Ok(()) => {
                        let _ = fs::remove_dir_all(staging);
                    }
                    Err(_) => return Err(error),
                }
            } else {
                let _ = fs::remove_dir_all(staging);
            }
            Err(error)
        }
    }
}

fn next_available_backup_name(backups_dir: &Path, base_name: &str) -> String {
    let mut backup_name = base_name.to_string();
    let mut suffix = 2;
    while backups_dir.join(&backup_name).exists() {
        backup_name = format!("{base_name} {suffix}");
        suffix += 1;
    }
    backup_name
}

fn restore_settings_backup(addons_root: &Path, backup_name: &str) -> Result<String, String> {
    validate_backup_name_for_lookup(backup_name)?;

    let backup_path = settings_backups_dir(addons_root).join(backup_name);
    if !backup_path.is_dir() {
        return Err(format!("Backup '{backup_name}' not found."));
    }
    let restore_mode = classify_backup_for_restore(&backup_path);
    if let CharRestoreMode::Refuse(reason) = &restore_mode {
        return Err(reason.clone());
    }

    let sv_dir = settings_saved_variables_dir(addons_root);
    if sv_dir.is_dir() && directory_has_files(&sv_dir) {
        let snapshot_name = format!("auto-before-restore-{}", unix_now_secs());
        let snapshot_path = settings_backups_dir(addons_root).join(snapshot_name);
        fs::create_dir_all(&snapshot_path)
            .map_err(|error| format!("Failed to create safety snapshot folder: {error}"))?;
        copy_directory_files(&sv_dir, &snapshot_path, false).map_err(|error| {
            format!(
                "Failed to create safety snapshot. Restore aborted to prevent data loss: {error}"
            )
        })?;
        prune_auto_snapshots(&settings_backups_dir(addons_root), 3);
    }

    fs::create_dir_all(&sv_dir)
        .map_err(|error| format!("Failed to create SavedVariables folder: {error}"))?;
    match restore_mode {
        CharRestoreMode::Refuse(reason) => Err(reason),
        CharRestoreMode::WholeFile => {
            let (file_count, total_size) = copy_directory_files(&backup_path, &sv_dir, true)?;
            Ok(backup_detail(file_count, total_size))
        }
        CharRestoreMode::Merge(meta) => {
            let (file_count, failed) =
                restore_character_subtrees_merge(&backup_path, &sv_dir, &meta);
            if failed.is_empty() {
                Ok(format!(
                    "{} character file{}",
                    file_count,
                    if file_count == 1 { "" } else { "s" }
                ))
            } else {
                Err(format!(
                    "Restore incomplete - {} file(s) failed: {}",
                    failed.len(),
                    failed.join(", ")
                ))
            }
        }
    }
}

fn classify_backup_for_restore(backup_path: &Path) -> CharRestoreMode {
    let refuse = |message: &str| CharRestoreMode::Refuse(message.to_string());
    let marker_path = backup_path.join(CHAR_BACKUP_MARKER);
    let meta_path = backup_path.join(CHAR_BACKUP_META);

    let marker = match fs::read(&marker_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            return refuse(
                "This character backup's marker is present but unreadable, so it can't be restored safely.",
            )
        }
    };
    let marker_is_v2 = marker
        .as_ref()
        .is_some_and(|content| content.starts_with(CHAR_BACKUP_MARKER_V2_PREFIX));

    match fs::read(&meta_path) {
        Ok(bytes) => match serde_json::from_slice::<CharBackupMeta>(&bytes) {
            Ok(meta) if meta.version == CHAR_BACKUP_VERSION => CharRestoreMode::Merge(meta),
            Ok(meta) => CharRestoreMode::Refuse(format!(
                "This character backup uses an unsupported format (version {}). Update Kalpa before restoring it.",
                meta.version
            )),
            Err(_) => CharRestoreMode::Refuse(
                "This character backup's metadata is corrupt, so it can't be restored safely."
                    .to_string(),
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if marker_is_v2 {
                refuse(
                    "This character backup is missing its metadata, so it can't be restored safely.",
                )
            } else {
                CharRestoreMode::WholeFile
            }
        }
        Err(_) => refuse(
            "This character backup's metadata is present but unreadable, so it can't be restored safely.",
        ),
    }
}

fn restore_character_subtrees_merge(
    backup_path: &Path,
    sv_dir: &Path,
    meta: &CharBackupMeta,
) -> (u32, Vec<String>) {
    let mut restored = 0u32;
    let mut failed = Vec::new();
    let base = char_backup::char_base(meta.character.as_bytes()).to_vec();
    let world = if char_backup::WELL_KNOWN_WORLDS.contains(&meta.server.as_str()) {
        Some(meta.server.as_str())
    } else {
        None
    };

    let entries = match fs::read_dir(backup_path) {
        Ok(entries) => entries,
        Err(error) => {
            failed.push(format!("backup: {error}"));
            return (restored, failed);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        let name_string = name.to_string_lossy().to_string();
        if name_string.starts_with('.')
            || path.extension().and_then(|ext| ext.to_str()) != Some("lua")
        {
            continue;
        }
        let stored = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                failed.push(format!("{name_string}: {error}"));
                continue;
            }
        };
        let blocks = char_backup::extract_character_blocks(&stored, &base, world);
        if blocks.is_empty() {
            failed.push(format!(
                "{name_string}: no character subtree found in backup"
            ));
            continue;
        }

        let live_path = sv_dir.join(&name_string);
        let mut live = if live_path.is_file() {
            match fs::read(&live_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    failed.push(format!("{name_string}: {error}"));
                    continue;
                }
            }
        } else {
            Vec::new()
        };

        let mut ok = true;
        for block in &blocks {
            match char_backup::merge_character_block(&live, block) {
                Ok(merged) => live = merged,
                Err(error) => {
                    failed.push(format!("{name_string}: {error}"));
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        match write_raw_backup_bytes(sv_dir, &name_string, &live) {
            Ok(()) => restored += 1,
            Err(error) => failed.push(format!("{name_string}: {error}")),
        }
    }

    (restored, failed)
}

fn write_raw_backup_bytes(sv_dir: &Path, file_name: &str, content: &[u8]) -> Result<(), String> {
    let file_path = sv_dir.join(file_name);
    let tmp_path = sv_dir.join(format!("{file_name}.tmp"));
    fs::write(&tmp_path, content).map_err(|error| format!("Failed to write temp file: {error}"))?;
    fs::rename(&tmp_path, &file_path).map_err(|error| {
        let _ = fs::remove_file(&tmp_path);
        format!("Failed to finalize write: {error}")
    })
}

fn prune_auto_snapshots(backups_dir: &Path, keep: usize) {
    let prefix = "auto-before-restore-";
    let Ok(entries) = fs::read_dir(backups_dir) else {
        return;
    };
    let mut dirs = entries
        .flatten()
        .filter(|entry| {
            entry.path().is_dir()
                && entry
                    .file_name()
                    .to_str()
                    .map(|name| name.starts_with(prefix))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    if dirs.len() <= keep {
        return;
    }
    dirs.sort_by_key(|entry| entry.file_name());
    let remove_count = dirs.len() - keep;
    for entry in dirs.into_iter().take(remove_count) {
        let _ = fs::remove_dir_all(entry.path());
    }
}

fn recover_orphaned_backups(backups_root: &Path) {
    let Ok(entries) = fs::read_dir(backups_root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix(".old-char-") else {
            continue;
        };
        let Some((base, seq)) = rest.rsplit_once('-') else {
            continue;
        };
        if base.is_empty() {
            continue;
        }
        let final_dir = backups_root.join(format!("char-{base}"));
        let staging = backups_root.join(format!(".tmp-char-{base}-{seq}"));
        if final_dir.exists() {
            let _ = fs::remove_dir_all(&path);
        } else if staging.exists() {
            if fs::rename(&path, &final_dir).is_ok() {
                let _ = fs::remove_dir_all(&staging);
            }
        } else {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

fn delete_settings_backup(addons_root: &Path, backup_name: &str) -> Result<(), String> {
    validate_backup_name_for_lookup(backup_name)?;
    let backups_dir = settings_backups_dir(addons_root);
    recover_orphaned_backups(&backups_dir);
    let backup_path = backups_dir.join(backup_name);
    if !backup_path.is_dir() {
        return Err(format!("Backup '{backup_name}' not found."));
    }
    fs::remove_dir_all(&backup_path).map_err(|error| format!("Failed to delete backup: {error}"))
}

fn copy_directory_files(
    source_dir: &Path,
    destination_dir: &Path,
    skip_dotfiles: bool,
) -> Result<(u32, u64), String> {
    let entries =
        fs::read_dir(source_dir).map_err(|error| format!("Failed to read folder: {error}"))?;
    let mut file_count = 0u32;
    let mut total_size = 0u64;
    let mut failed = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        if skip_dotfiles
            && name
                .to_str()
                .map(|name| name.starts_with('.'))
                .unwrap_or(false)
        {
            continue;
        }
        let destination = destination_dir.join(name);
        match fs::copy(&path, &destination) {
            Ok(_) => {
                file_count += 1;
                total_size += fs::metadata(&destination)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
            }
            Err(error) => failed.push(format!("{}: {error}", name.to_string_lossy())),
        }
    }

    if failed.is_empty() {
        Ok((file_count, total_size))
    } else {
        Err(format!(
            "{} file(s) failed to copy: {}",
            failed.len(),
            failed.join(", ")
        ))
    }
}

fn directory_has_files(path: &Path) -> bool {
    fs::read_dir(path)
        .map(|mut entries| {
            entries.any(|entry| entry.map(|entry| entry.path().is_file()).unwrap_or(false))
        })
        .unwrap_or(false)
}

fn friendly_backup_name(epoch_secs: u64) -> String {
    let seconds = epoch_secs % 86_400;
    let hour = seconds / 3600;
    let minute = (seconds % 3600) / 60;
    let second = seconds % 60;
    let date = format_short_date(epoch_secs).replace(',', "");
    format!("Manual backup {date} {hour:02}-{minute:02}-{second:02}")
}

fn validate_backup_name(name: &str) -> Result<(), String> {
    validate_backup_name_for_lookup(name)?;
    if name.starts_with('.')
        || name.starts_with("char-")
        || name.starts_with("auto-before-restore-")
    {
        return Err("Backup name uses a reserved prefix. Choose another name.".to_string());
    }
    Ok(())
}

fn validate_backup_name_for_lookup(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Name cannot be empty.".to_string());
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("Name contains invalid characters.".to_string());
    }
    let forbidden: &[char] = &['<', '>', ':', '"', '|', '?', '*'];
    if name.contains(forbidden) {
        return Err("Name contains a forbidden character.".to_string());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("Name must not end with a dot or space.".to_string());
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return Err(format!(
            "\"{stem}\" is a Windows reserved name and cannot be used."
        ));
    }
    Ok(())
}

fn format_backup_relative_time(epoch_secs: u64, now_secs: u64) -> String {
    if epoch_secs == 0 {
        return "unknown time".to_string();
    }
    let diff = now_secs.saturating_sub(epoch_secs);
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        let minutes = diff / 60;
        format!(
            "{minutes} minute{} ago",
            if minutes == 1 { "" } else { "s" }
        )
    } else if diff < 86_400 {
        let hours = diff / 3600;
        format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" })
    } else if diff < 86_400 * 2 {
        "Yesterday".to_string()
    } else if diff < 86_400 * 30 {
        let days = diff / 86_400;
        format!("{days} day{} ago", if days == 1 { "" } else { "s" })
    } else {
        format_short_date(epoch_secs)
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn apply_saved_variables_model(ui: &KalpaWindow, addons: &[AddonEntry]) {
    let entries = addons_source_root()
        .and_then(|addons_root| saved_variable_entries(&addons_root, addons).ok())
        .unwrap_or_default();

    let orphaned = entries
        .iter()
        .filter(|entry| entry.orphaned)
        .cloned()
        .map(|mut entry| {
            entry.selected = true;
            entry
        })
        .collect::<Vec<_>>();
    let total_size = entries
        .iter()
        .map(|entry| entry.size_bytes.max(0) as u64)
        .sum::<u64>();
    let orphan_size = orphaned
        .iter()
        .map(|entry| entry.size_bytes.max(0) as u64)
        .sum::<u64>();
    let large_count = entries
        .iter()
        .filter(|entry| entry.size_bytes >= (5 * 1024 * 1024))
        .count();

    ui.set_svm_file_count_label(entries.len().to_string().into());
    ui.set_svm_total_size_label(format_size(total_size).into());
    ui.set_svm_orphan_count_label(orphaned.len().to_string().into());
    ui.set_svm_orphan_size_label(format_size(orphan_size).into());
    ui.set_svm_large_count_label(large_count.to_string().into());
    ui.set_svm_has_orphans(!orphaned.is_empty());
    ui.set_svm_has_files(!entries.is_empty());
    ui.set_svm_files(Rc::new(VecModel::from(entries)).into());
    ui.set_svm_cleanup_selected_count(orphaned.len() as i32);
    ui.set_svm_orphaned_files(Rc::new(VecModel::from(orphaned)).into());
}

fn update_svm_cleanup_selected_count(ui: &KalpaWindow) {
    let model = ui.get_svm_orphaned_files();
    let selected = (0..model.row_count())
        .filter_map(|index| model.row_data(index))
        .filter(|entry| entry.selected)
        .count();
    ui.set_svm_cleanup_selected_count(selected as i32);
}

fn refresh_saved_variables_overlay_model(
    ui: &KalpaWindow,
    models: &AddonModels,
    copy_state: &Rc<RefCell<SvmCopyState>>,
    editor_state: &Rc<RefCell<SvmEditorState>>,
) {
    let addons = models.all.borrow();
    apply_saved_variables_model(ui, &addons);
    refresh_svm_copy_state(ui, &addons, copy_state);
    refresh_svm_editor_state(ui, &addons, editor_state);
}

fn refresh_svm_copy_state(
    ui: &KalpaWindow,
    addons: &[AddonEntry],
    copy_state: &Rc<RefCell<SvmCopyState>>,
) {
    let result = addons_source_root()
        .map(|addons_root| saved_variable_profile_files(&addons_root, addons))
        .unwrap_or_else(|| Ok(Vec::new()));

    {
        let mut state = copy_state.borrow_mut();
        match result {
            Ok(files) => state.replace_files(files),
            Err(error) => {
                state.replace_files(Vec::new());
                state.status = error;
            }
        }
    }

    apply_svm_copy_state(ui, &copy_state.borrow());
}

fn apply_svm_copy_state(ui: &KalpaWindow, state: &SvmCopyState) {
    let selected_file = state.selected_file();
    let source = state.source_key().unwrap_or("");
    let dest = state.dest_key().unwrap_or_default();
    let file_label = selected_file
        .map(|file| {
            format!(
                "{} ({} profile{})",
                file.addon_name,
                file.profiles.len(),
                if file.profiles.len() == 1 { "" } else { "s" }
            )
        })
        .unwrap_or_else(|| "No profile files".to_string());
    let source_label = if source.is_empty() {
        "No source profile".to_string()
    } else {
        source.to_string()
    };
    let dest_label = if dest.is_empty() {
        "No destination profile".to_string()
    } else {
        dest.clone()
    };
    let status = if !state.status.is_empty() {
        state.status.clone()
    } else if state.files.is_empty() {
        "No SavedVariables files with character profiles were found.".to_string()
    } else if source.is_empty() {
        "This file has no copyable character profiles.".to_string()
    } else if dest.is_empty() {
        "A second character profile is needed before copying.".to_string()
    } else {
        "Click a field to cycle through available choices.".to_string()
    };
    let ready = state.selection().is_some();
    let addon_name = selected_file
        .map(|file| format!("{}.lua", file.addon_name))
        .unwrap_or_default();

    ui.set_svm_copy_file_label(file_label.into());
    ui.set_svm_copy_source_label(source_label.into());
    ui.set_svm_copy_dest_label(dest_label.into());
    ui.set_svm_copy_source_name(source.into());
    ui.set_svm_copy_dest_name(dest.into());
    ui.set_svm_copy_addon_name(addon_name.into());
    ui.set_svm_copy_status_label(status.into());
    ui.set_svm_copy_ready(ready);
    ui.set_svm_copy_has_file(selected_file.is_some());
    ui.set_svm_copy_has_source(!source.is_empty());
    ui.set_svm_copy_has_dest(!state.destination_choices().is_empty());
}

fn saved_variable_profile_files(
    addons_root: &Path,
    addons: &[AddonEntry],
) -> Result<Vec<SvmProfileFile>, String> {
    let sv_dir = addons_root
        .parent()
        .unwrap_or(addons_root)
        .join("SavedVariables");
    let files = saved_variable_entries(addons_root, addons)?
        .into_iter()
        .filter_map(|entry| {
            let file_name = entry.file_name.to_string();
            let profiles = extract_saved_variable_profiles_from_path(&sv_dir.join(&file_name))
                .into_iter()
                .collect::<Vec<_>>();
            if profiles.is_empty() {
                return None;
            }
            Some(SvmProfileFile {
                file_name,
                addon_name: entry.addon_name.to_string(),
                profiles,
            })
        })
        .collect();

    Ok(files)
}

fn validate_svm_profile_key(key: &str) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("Character profile keys cannot be empty.".to_string());
    }
    if key.contains('"') || key.contains('\'') || key.contains('\\') {
        return Err("Character profile keys must not contain quotes or backslashes.".to_string());
    }
    if key.chars().any(|character| character.is_control()) {
        return Err("Character profile keys must not contain control characters.".to_string());
    }
    Ok(())
}

fn copy_svm_profile_selection(
    addons_root: &Path,
    selection: &SvmCopySelection,
) -> Result<(), String> {
    validate_svm_profile_key(&selection.source_key)?;
    validate_svm_profile_key(&selection.dest_key)?;
    saved_variables::profile::copy_sv_profile_blocking(
        addons_root,
        &selection.file_name,
        &selection.source_key,
        &selection.dest_key,
    )
}

fn refresh_svm_editor_state(
    ui: &KalpaWindow,
    addons: &[AddonEntry],
    editor_state: &Rc<RefCell<SvmEditorState>>,
) {
    let result = addons_source_root().map(|addons_root| {
        let files = saved_variable_editor_files(&addons_root, addons)?;
        Ok((addons_root, files))
    });

    {
        let mut state = editor_state.borrow_mut();
        match result {
            Some(Ok((addons_root, files))) => {
                state.replace_files(files);
                if let Err(error) = load_svm_editor_selected_file(&addons_root, &mut state) {
                    state.tree = None;
                    state.stamp = None;
                    state.selected_path.clear();
                    state.dirty = false;
                    state.message = error;
                }
            }
            Some(Err(error)) => {
                state.replace_files(Vec::new());
                state.tree = None;
                state.stamp = None;
                state.selected_path.clear();
                state.dirty = false;
                state.message = error;
            }
            None => {
                state.replace_files(Vec::new());
                state.tree = None;
                state.stamp = None;
                state.selected_path.clear();
                state.dirty = false;
                state.message =
                    "Configure the ESO AddOns folder to edit SavedVariables.".to_string();
            }
        }
    }

    apply_svm_editor_state(ui, &editor_state.borrow());
}

fn saved_variable_editor_files(
    addons_root: &Path,
    addons: &[AddonEntry],
) -> Result<Vec<SvmEditorFile>, String> {
    Ok(saved_variable_entries(addons_root, addons)?
        .into_iter()
        .map(|entry| SvmEditorFile {
            file_name: entry.file_name.to_string(),
            addon_name: entry.addon_name.to_string(),
        })
        .collect())
}

fn load_svm_editor_selected_file(
    addons_root: &Path,
    state: &mut SvmEditorState,
) -> Result<(), String> {
    let Some(file_name) = state.selected_file_name() else {
        state.tree = None;
        state.stamp = None;
        state.selected_path.clear();
        state.dirty = false;
        state.message = "No SavedVariables files found.".to_string();
        return Ok(());
    };

    let response = saved_variables::io::read_saved_variable_blocking(addons_root, &file_name)?;
    let selected_path = default_svm_editor_path(&response.tree).unwrap_or_default();
    state.tree = Some(response.tree);
    state.stamp = Some(response.stamp);
    state.selected_path = selected_path;
    state.tree_expanded_all = false;
    state.dirty = false;
    state.message.clear();
    Ok(())
}

fn apply_svm_editor_state(ui: &KalpaWindow, state: &SvmEditorState) {
    let file_label = state
        .selected_file()
        .map(|file| file.file_name.clone())
        .unwrap_or_else(|| "No file selected".to_string());
    let tree_entries = state
        .tree
        .as_ref()
        .map(|tree| {
            svm_editor_tree_entries(
                tree,
                &state.selected_path,
                state.tree_expanded_all,
                &state.tree_filter,
            )
        })
        .unwrap_or_default();
    let setting_entries = svm_editor_setting_entries(state);
    let path_label = if state.selected_path.is_empty() {
        "Root".to_string()
    } else {
        format!("Root  >  {}", state.selected_path.join("  >  "))
    };
    let message = if !state.message.is_empty() {
        state.message.clone()
    } else if state.tree.is_none() {
        "No SavedVariables file loaded.".to_string()
    } else if setting_entries.is_empty() {
        "No editable leaf settings in the selected branch.".to_string()
    } else {
        String::new()
    };

    ui.set_svm_editor_file_label(file_label.into());
    ui.set_svm_editor_path_label(path_label.into());
    ui.set_svm_editor_message_label(message.into());
    ui.set_svm_editor_has_file(state.selected_file().is_some());
    ui.set_svm_editor_dirty(state.dirty);
    ui.set_svm_editor_tree(Rc::new(VecModel::from(tree_entries)).into());
    ui.set_svm_editor_settings(Rc::new(VecModel::from(setting_entries)).into());
}

fn default_svm_editor_path(tree: &saved_variables::SvTreeNode) -> Option<Vec<String>> {
    fn walk(node: &saved_variables::SvTreeNode, path: &[String]) -> Option<Vec<String>> {
        let children = node.children.as_ref()?;
        for child in children {
            let mut child_path = path.to_vec();
            child_path.push(child.key.clone());
            if matches!(child.value_type, saved_variables::types::SvValueType::Table)
                && child
                    .children
                    .as_ref()
                    .map(|inner| {
                        inner.iter().any(|entry| {
                            !matches!(entry.value_type, saved_variables::types::SvValueType::Table)
                        })
                    })
                    .unwrap_or(false)
            {
                return Some(child_path);
            }
            if let Some(found) = walk(child, &child_path) {
                return Some(found);
            }
        }
        children.first().map(|child| {
            let mut child_path = path.to_vec();
            child_path.push(child.key.clone());
            child_path
        })
    }

    walk(tree, &[])
}

fn svm_editor_tree_entries(
    tree: &saved_variables::SvTreeNode,
    selected_path: &[String],
    expand_all: bool,
    filter: &str,
) -> Vec<SvmEditorTreeEntry> {
    let query = filter.trim().to_ascii_lowercase();

    fn node_matches(node: &saved_variables::SvTreeNode, query: &str) -> bool {
        if node.key.to_ascii_lowercase().contains(query) {
            return true;
        }
        node.children
            .as_ref()
            .map(|children| children.iter().any(|child| node_matches(child, query)))
            .unwrap_or(false)
    }

    fn collect(
        node: &saved_variables::SvTreeNode,
        depth: i32,
        path: Vec<String>,
        selected_path: &[String],
        expand_all: bool,
        query: &str,
        rows: &mut Vec<SvmEditorTreeEntry>,
    ) {
        if rows.len() >= 500 {
            return;
        }
        let Some(children) = node.children.as_ref() else {
            return;
        };

        for child in children {
            if rows.len() >= 500 {
                return;
            }
            // While filtering, drop branches that neither match nor contain a match.
            if !query.is_empty() && !node_matches(child, query) {
                continue;
            }
            let mut child_path = path.clone();
            child_path.push(child.key.clone());
            let child_count = child.children.as_ref().map(Vec::len).unwrap_or(0);
            // Filtering auto-expands surviving branches so matches are visible.
            let expanded = child_count > 0
                && (!query.is_empty()
                    || expand_all
                    || depth == 0
                    || path_is_prefix(&child_path, selected_path));
            let path_json = serde_json::to_string(&child_path).unwrap_or_else(|_| "[]".to_string());
            rows.push(SvmEditorTreeEntry {
                label: child.key.clone().into(),
                path: path_json.into(),
                count: if child_count == 0 {
                    String::new().into()
                } else {
                    child_count.to_string().into()
                },
                indent: depth,
                active: child_path == selected_path,
                expanded,
            });
            if expanded {
                collect(
                    child,
                    depth + 1,
                    child_path,
                    selected_path,
                    expand_all,
                    query,
                    rows,
                );
            }
        }
    }

    let mut rows = Vec::new();
    collect(
        tree,
        0,
        Vec::new(),
        selected_path,
        expand_all,
        &query,
        &mut rows,
    );
    rows
}

fn path_is_prefix(path: &[String], selected_path: &[String]) -> bool {
    path.len() <= selected_path.len()
        && path
            .iter()
            .zip(selected_path.iter())
            .all(|(left, right)| left == right)
}

fn svm_editor_setting_entries(state: &SvmEditorState) -> Vec<SvmEditorSettingEntry> {
    let Some(tree) = state.tree.as_ref() else {
        return Vec::new();
    };
    let Some(selected) = sv_node_at_path(tree, &state.selected_path) else {
        return Vec::new();
    };
    selected
        .children
        .as_ref()
        .into_iter()
        .flatten()
        .filter(|child| !matches!(child.value_type, saved_variables::types::SvValueType::Table))
        .map(|child| SvmEditorSettingEntry {
            label: humanize_sv_key(&child.key).into(),
            key_name: child.key.clone().into(),
            kind: svm_setting_kind(child),
            value: svm_value_label(child).into(),
            checked: child
                .value
                .as_ref()
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
        })
        .collect()
}

fn select_svm_editor_tree_path(state: &mut SvmEditorState, path_json: &str) -> Result<(), String> {
    let path: Vec<String> = serde_json::from_str(path_json)
        .map_err(|_| "Could not read the SavedVariables branch path.".to_string())?;
    let tree = state
        .tree
        .as_ref()
        .ok_or_else(|| "No SavedVariables file loaded.".to_string())?;
    let node = sv_node_at_path(tree, &path)
        .ok_or_else(|| "SavedVariables branch not found.".to_string())?;
    if !matches!(node.value_type, saved_variables::types::SvValueType::Table) {
        return Err("Select a table branch to inspect its settings.".to_string());
    }
    state.selected_path = path;
    state.message.clear();
    Ok(())
}

fn svm_editor_setting_paths(state: &SvmEditorState) -> Vec<Vec<String>> {
    let Some(tree) = state.tree.as_ref() else {
        return Vec::new();
    };
    let Some(selected) = sv_node_at_path(tree, &state.selected_path) else {
        return Vec::new();
    };
    selected
        .children
        .as_ref()
        .into_iter()
        .flatten()
        .filter(|child| !matches!(child.value_type, saved_variables::types::SvValueType::Table))
        .map(|child| {
            let mut path = state.selected_path.clone();
            path.push(child.key.clone());
            path
        })
        .collect()
}

fn sv_node_at_path<'a>(
    tree: &'a saved_variables::SvTreeNode,
    path: &[String],
) -> Option<&'a saved_variables::SvTreeNode> {
    let mut current = tree;
    for segment in path {
        current = current
            .children
            .as_ref()?
            .iter()
            .find(|child| child.key == *segment)?;
    }
    Some(current)
}

fn sv_node_at_path_mut<'a>(
    tree: &'a mut saved_variables::SvTreeNode,
    path: &[String],
) -> Option<&'a mut saved_variables::SvTreeNode> {
    let mut current = tree;
    for segment in path {
        current = current
            .children
            .as_mut()?
            .iter_mut()
            .find(|child| child.key == *segment)?;
    }
    Some(current)
}

fn svm_setting_kind(node: &saved_variables::SvTreeNode) -> i32 {
    match node.value_type {
        saved_variables::types::SvValueType::Boolean => 0,
        saved_variables::types::SvValueType::Number => 2,
        _ => 1,
    }
}

fn svm_value_label(node: &saved_variables::SvTreeNode) -> String {
    match node.value_type {
        saved_variables::types::SvValueType::Boolean => node
            .value
            .as_ref()
            .and_then(|value| value.as_bool())
            .map(|value| if value { "true" } else { "false" }.to_string())
            .unwrap_or_else(|| "false".to_string()),
        saved_variables::types::SvValueType::Number => node
            .value
            .as_ref()
            .map(|value| {
                if let Some(number) = value.as_f64() {
                    if number == (number as i64) as f64 {
                        (number as i64).to_string()
                    } else {
                        number.to_string()
                    }
                } else {
                    value.to_string()
                }
            })
            .unwrap_or_else(|| "0".to_string()),
        saved_variables::types::SvValueType::String => node
            .value
            .as_ref()
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string(),
        saved_variables::types::SvValueType::Nil => "nil".to_string(),
        saved_variables::types::SvValueType::Table => node
            .children
            .as_ref()
            .map(|children| format!("{} entries", children.len()))
            .unwrap_or_else(|| "0 entries".to_string()),
    }
}

fn humanize_sv_key(key: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for character in key.chars() {
        if character == '_' || character == '-' {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            previous_lower = false;
            continue;
        }
        if character.is_ascii_uppercase() && previous_lower && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push(character);
        previous_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
    }
    if out.is_empty() {
        key.to_string()
    } else {
        let mut chars = out.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
            None => key.to_string(),
        }
    }
}

fn toggle_svm_editor_setting(state: &mut SvmEditorState, index: usize) -> Result<(), String> {
    let path = svm_editor_setting_paths(state)
        .get(index)
        .cloned()
        .ok_or_else(|| "Setting row not found.".to_string())?;
    let tree = state
        .tree
        .as_mut()
        .ok_or_else(|| "No SavedVariables file loaded.".to_string())?;
    let node =
        sv_node_at_path_mut(tree, &path).ok_or_else(|| "Setting node not found.".to_string())?;
    if !matches!(
        node.value_type,
        saved_variables::types::SvValueType::Boolean
    ) {
        return Err("Edit this setting in its value field instead of toggling it.".to_string());
    }
    let current = node
        .value
        .as_ref()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    node.value = Some(serde_json::Value::Bool(!current));
    state.dirty = true;
    state.message = "Unsaved SavedVariables change.".to_string();
    Ok(())
}

fn edit_svm_editor_setting(
    state: &mut SvmEditorState,
    index: usize,
    value: &str,
) -> Result<(), String> {
    let path = svm_editor_setting_paths(state)
        .get(index)
        .cloned()
        .ok_or_else(|| "Setting row not found.".to_string())?;
    let tree = state
        .tree
        .as_mut()
        .ok_or_else(|| "No SavedVariables file loaded.".to_string())?;
    let node =
        sv_node_at_path_mut(tree, &path).ok_or_else(|| "Setting node not found.".to_string())?;

    match node.value_type {
        saved_variables::types::SvValueType::Boolean => {
            let lowered = value.trim().to_ascii_lowercase();
            let parsed = match lowered.as_str() {
                "true" | "1" | "yes" | "on" => true,
                "false" | "0" | "no" | "off" => false,
                _ => return Err("Use true or false for this SavedVariables setting.".to_string()),
            };
            node.value = Some(serde_json::Value::Bool(parsed));
        }
        saved_variables::types::SvValueType::Number => {
            let trimmed = value.trim();
            let parsed = trimmed
                .parse::<f64>()
                .map_err(|_| "Enter a valid number for this SavedVariables setting.".to_string())?;
            let number = serde_json::Number::from_f64(parsed).ok_or_else(|| {
                "Enter a finite number for this SavedVariables setting.".to_string()
            })?;
            node.value = Some(serde_json::Value::Number(number));
        }
        saved_variables::types::SvValueType::String => {
            node.value = Some(serde_json::Value::String(value.to_string()));
            node.raw_lua_value = None;
        }
        saved_variables::types::SvValueType::Nil => {
            node.value = Some(serde_json::Value::String(value.to_string()));
            node.value_type = saved_variables::types::SvValueType::String;
            node.raw_lua_value = None;
        }
        saved_variables::types::SvValueType::Table => {
            return Err("Select a child setting before editing a value.".to_string());
        }
    }

    state.dirty = true;
    state.message = "Unsaved SavedVariables change.".to_string();
    Ok(())
}

fn save_svm_editor_file(addons_root: &Path, state: &mut SvmEditorState) -> Result<(), String> {
    let file_name = state
        .selected_file_name()
        .ok_or_else(|| "No SavedVariables file selected.".to_string())?;
    let tree = state
        .tree
        .as_ref()
        .ok_or_else(|| "No SavedVariables file loaded.".to_string())?;
    let stamp = state
        .stamp
        .as_ref()
        .ok_or_else(|| "No file stamp available. Reload the file before saving.".to_string())?;
    let new_stamp =
        saved_variables::io::write_saved_variable_blocking(addons_root, &file_name, tree, stamp)?;
    state.stamp = Some(new_stamp);
    state.dirty = false;
    state.message = "Saved changes and wrote a .bak backup.".to_string();
    Ok(())
}

fn preview_svm_editor_file(addons_root: &Path, state: &mut SvmEditorState) -> Result<(), String> {
    let file_name = state
        .selected_file_name()
        .ok_or_else(|| "No SavedVariables file selected.".to_string())?;
    let tree = state
        .tree
        .as_ref()
        .ok_or_else(|| "No SavedVariables file loaded.".to_string())?;
    let changes = saved_variables::io::preview_save(addons_root, &file_name, tree)?;
    state.message = if changes.is_empty() {
        "No changes detected.".to_string()
    } else {
        format!(
            "{} pending change{}.",
            changes.len(),
            if changes.len() == 1 { "" } else { "s" }
        )
    };
    Ok(())
}

fn restore_svm_editor_backup(addons_root: &Path, state: &mut SvmEditorState) -> Result<(), String> {
    let file_name = state
        .selected_file_name()
        .ok_or_else(|| "No SavedVariables file selected.".to_string())?;
    saved_variables::io::restore_backup_file(addons_root, &file_name)?;
    load_svm_editor_selected_file(addons_root, state)?;
    state.message = "Restored the latest .bak file.".to_string();
    Ok(())
}

fn copy_svm_editor_raw_to_clipboard(state: &mut SvmEditorState) -> Result<String, String> {
    let raw_lua = svm_editor_raw_lua(state)?;
    let size = raw_lua.len() as u64;
    write_clipboard_text(raw_lua)?;
    let message = format!(
        "Copied raw SavedVariables Lua ({}) to clipboard.",
        format_size(size)
    );
    state.message = message.clone();
    Ok(message)
}

fn svm_editor_raw_lua(state: &SvmEditorState) -> Result<String, String> {
    let tree = state
        .tree
        .as_ref()
        .ok_or_else(|| "No SavedVariables file loaded.".to_string())?;
    Ok(saved_variables::serializer::serialize_to_lua(tree))
}

fn clean_saved_variable_orphans(
    addons_root: &Path,
    orphaned: &[SavedVariableEntry],
) -> Result<u32, String> {
    let file_names = orphaned
        .iter()
        .map(|entry| entry.file_name.to_string())
        .collect::<Vec<_>>();
    if file_names.is_empty() {
        return Ok(0);
    }

    saved_variables::io::delete_saved_variables_blocking(addons_root, &file_names)
}

fn saved_variable_entries(
    addons_root: &Path,
    addons: &[AddonEntry],
) -> Result<Vec<SavedVariableEntry>, String> {
    let sv_dir = addons_root
        .parent()
        .unwrap_or(addons_root)
        .join("SavedVariables");
    if !sv_dir.is_dir() {
        return Ok(Vec::new());
    }

    let installed = addons
        .iter()
        .map(|addon| addon.folder_name.to_string())
        .collect::<HashSet<_>>();
    let mut raw = fs::read_dir(&sv_dir)
        .map_err(|error| format!("Failed to read SavedVariables folder: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| saved_variable_entry(&entry.path(), &installed))
        .collect::<Vec<_>>();

    let max_size = raw.iter().map(|entry| entry.1).max().unwrap_or(1);
    raw.sort_by(|left, right| {
        right.1.cmp(&left.1).then_with(|| {
            left.0
                .title
                .to_ascii_lowercase()
                .cmp(&right.0.title.to_ascii_lowercase())
        })
    });

    Ok(raw
        .into_iter()
        .map(|(mut entry, size)| {
            let ratio = if max_size == 0 {
                0.0
            } else {
                size as f32 / max_size as f32
            };
            entry.meter_width = (10.0 + ratio * 46.0).clamp(10.0, 56.0).round() as i32;
            entry
        })
        .collect())
}

fn saved_variable_entry(
    path: &Path,
    installed: &HashSet<String>,
) -> Option<(SavedVariableEntry, u64)> {
    let file_name = path.file_name()?.to_string_lossy().to_string();
    if !file_name.ends_with(".lua") {
        return None;
    }

    let addon_name = file_name
        .strip_suffix(".lua")
        .unwrap_or(&file_name)
        .to_string();
    let metadata = fs::metadata(path).ok();
    let size = metadata
        .as_ref()
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let modified = metadata
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| format_short_date(duration.as_secs()))
        .unwrap_or_default();
    let profile_count = extract_saved_variable_profile_count(path);
    let meta = if profile_count > 0 {
        format!(
            "{} - {} profile{}",
            modified,
            profile_count,
            if profile_count == 1 { "" } else { "s" }
        )
    } else {
        modified
    };
    let status = classify_saved_variable(&addon_name, installed);

    Some((
        SavedVariableEntry {
            file_name: file_name.into(),
            addon_name: addon_name.clone().into(),
            title: addon_name.into(),
            meta: meta.into(),
            size_label: format_size(size).into(),
            size_bytes: size.min(i32::MAX as u64) as i32,
            meter_width: 10,
            system: status == SavedVariableStatus::System,
            orphaned: status == SavedVariableStatus::Orphaned,
            selected: false,
        },
        size,
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SavedVariableStatus {
    Installed,
    System,
    Orphaned,
}

fn classify_saved_variable(addon_name: &str, installed: &HashSet<String>) -> SavedVariableStatus {
    const SYSTEM_NAMES: &[&str] = &[
        "ZO_Ingame",
        "ZO_InternalIngame",
        "ZO_Pregame",
        "AccountSettings",
        "GuildHistoryCache",
    ];

    if SYSTEM_NAMES.contains(&addon_name) {
        return SavedVariableStatus::System;
    }
    if installed.contains(addon_name) {
        return SavedVariableStatus::Installed;
    }

    for folder in installed {
        if folder.len() < 4 || !addon_name.starts_with(folder) || addon_name.len() <= folder.len() {
            continue;
        }
        let boundary = addon_name[folder.len()..].chars().next();
        if boundary
            .map(|character| character == '_' || character == '-' || character.is_ascii_uppercase())
            .unwrap_or(true)
        {
            return SavedVariableStatus::Installed;
        }
    }

    SavedVariableStatus::Orphaned
}

fn extract_saved_variable_profile_count(path: &Path) -> usize {
    extract_saved_variable_profiles_from_path(path).len()
}

fn extract_saved_variable_profiles_from_path(path: &Path) -> BTreeSet<String> {
    let Ok(mut file) = fs::File::open(path) else {
        return BTreeSet::new();
    };
    let mut buffer = vec![0u8; 256 * 1024];
    let Ok(count) = std::io::Read::read(&mut file, &mut buffer) else {
        return BTreeSet::new();
    };
    buffer.truncate(count);
    let content = String::from_utf8_lossy(&buffer);
    extract_saved_variable_profiles(&content)
}

fn extract_saved_variable_profiles(content: &str) -> BTreeSet<String> {
    let mut profiles = BTreeSet::new();
    let mut depth: i32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if depth == 3 {
            if let Some(key) = saved_variable_key(trimmed) {
                if key != "$AccountWide" {
                    profiles.insert(key.to_string());
                }
            }
        }
        depth += brace_delta_ignoring_strings(line);
    }

    profiles
}

fn saved_variable_key(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("[\"")?;
    let end = rest.find("\"]")?;
    let after = rest[end + 2..].trim_start();
    if !after.starts_with('=') {
        return None;
    }
    Some(&rest[..end])
}

fn brace_delta_ignoring_strings(line: &str) -> i32 {
    let bytes = line.as_bytes();
    let mut index = 0usize;
    let mut delta = 0i32;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => {
                let quote = bytes[index];
                index += 1;
                while index < bytes.len() && bytes[index] != quote {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    index += 1;
                }
                index += 1;
            }
            b'-' if index + 1 < bytes.len() && bytes[index + 1] == b'-' => break,
            b'{' => {
                delta += 1;
                index += 1;
            }
            b'}' => {
                delta -= 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    delta
}

fn format_short_date(epoch_secs: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = epoch_secs / 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let month_name = MONTHS
        .get(month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("Jan");
    format!("{month_name} {day}, {year}")
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
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
    fs::write(&file_path, content).map_err(|error| format!("Failed to write file: {error}"))?;
    update_hash_manifest_for_file(addons_root, folder_name, relative_path, &file_path)
}

fn update_hash_manifest_for_file(
    addons_root: &Path,
    folder_name: &str,
    relative_path: &str,
    file_path: &Path,
) -> Result<(), String> {
    let Some(mut manifest) = load_hash_manifest(addons_root, folder_name) else {
        return Ok(());
    };
    let key = relative_path.replace('\\', "/");
    let signature = file_signature(&key, file_path)?;
    let is_modified = manifest
        .files
        .get(&key)
        .map(|stored| !signatures_match(stored, &signature))
        .unwrap_or(true);
    if is_modified && !manifest.modified_files.contains(&key) {
        manifest.modified_files.push(key);
        manifest.modified_files.sort();
    } else if !is_modified {
        manifest.modified_files.retain(|file| file != &key);
    }
    save_hash_manifest(addons_root, &manifest)
}

fn hash_manifest_path(addons_root: &Path, folder_name: &str) -> PathBuf {
    addons_root
        .join(".kalpa-hashes")
        .join(format!("{folder_name}.json"))
}

fn load_hash_manifest(addons_root: &Path, folder_name: &str) -> Option<NativeHashManifest> {
    let path = hash_manifest_path(addons_root, folder_name);
    if !path.exists() {
        return None;
    }
    let contents = fs::read_to_string(&path).ok()?;
    let mut manifest =
        serde_json::from_str::<NativeHashManifest>(json_without_bom(&contents)).ok()?;
    if manifest.esoui_ids.is_empty() && manifest.esoui_id != 0 {
        manifest.esoui_ids = vec![manifest.esoui_id];
    }
    Some(manifest)
}

fn save_hash_manifest(addons_root: &Path, manifest: &NativeHashManifest) -> Result<(), String> {
    let path = hash_manifest_path(addons_root, &manifest.addon_folder);
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|error| format!("Failed to serialize hash manifest: {error}"))?;
    write_string_atomic(&path, &json)
        .map_err(|error| format!("Failed to save hash manifest: {error}"))
}

fn file_signature(key: &str, path: &Path) -> Result<String, String> {
    if hashes_file_contents(key) {
        hash_file_sha256(path)
    } else {
        let size = fs::metadata(path)
            .map_err(|error| format!("Failed to read file metadata for signature: {error}"))?
            .len();
        Ok(format!("size:{size}"))
    }
}

fn hash_file_sha256(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Failed to open file for hashing: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Failed to read file for hashing: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn hashes_file_contents(key: &str) -> bool {
    let ext = key
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some(
            "lua"
                | "xml"
                | "txt"
                | "addon"
                | "md"
                | "json"
                | "toc"
                | "def"
                | "lang"
                | "csv"
                | "cfg"
                | "ini"
                | "html"
                | "htm"
        )
    )
}

fn signatures_match(stored: &str, current: &str) -> bool {
    stored == current || (stored.starts_with("size:") != current.starts_with("size:"))
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

fn return_to_webview_shell(
    start_app_update: bool,
    start_log_uploader: bool,
    start_pack_hub: bool,
    pack_hub_pack_id: Option<&str>,
) -> Result<(), String> {
    let exe = std::env::var_os("KALPA_WEBVIEW_EXE")
        .map(PathBuf::from)
        .ok_or_else(|| "webview launcher path was not provided".to_string())?;
    if !exe.is_file() {
        return Err(format!(
            "webview launcher was not found at {}",
            exe.display()
        ));
    }

    let mut command = std::process::Command::new(&exe);
    command.env("KALPA_FORCE_WEBVIEW", "1");
    if start_app_update {
        command.env("KALPA_START_APP_UPDATE", "1");
    }
    if start_log_uploader {
        command.env("KALPA_START_LOG_UPLOADER", "1");
    }
    if start_pack_hub {
        command.env("KALPA_START_PACK_HUB", "1");
    }
    if let Some(pack_id) = pack_hub_pack_id.filter(|value| !value.trim().is_empty()) {
        command.env("KALPA_START_PACK_HUB_ID", pack_id);
    }

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to launch webview shell: {error}"))
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

fn apply_runtime_flags(ui: &KalpaWindow, render_preset: NativeRenderPreset) {
    let reduced_motion = std::env::var("KALPA_REDUCED_MOTION")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);

    let tokens = ui.global::<Tokens>();
    tokens.set_low_memory_preset(render_preset == NativeRenderPreset::LowMemory);
    tokens.set_reduced_motion(reduced_motion);
    tokens.set_ambient_motion(env_flag_with_default("KALPA_AMBIENT_MOTION", false));

    let detail_files_active = std::env::var("KALPA_DETAIL_TAB")
        .map(|value| value.eq_ignore_ascii_case("files"))
        .unwrap_or(false);

    ui.set_detail_files_active(detail_files_active);
    ui.set_settings_open(env_flag("KALPA_SETTINGS_OPEN"));
    ui.set_settings_editor_open(env_flag("KALPA_SETTINGS_EDITOR"));
    ui.set_settings_minion_detected(env_flag("KALPA_MINION_DETECTED"));
    ui.set_settings_pack_hub_authenticated(
        env_flag("KALPA_PACK_HUB_AUTHENTICATED") || env_flag("KALPA_AUTH_USER"),
    );
    ui.set_pack_hub_open(env_flag("KALPA_PACK_HUB_OPEN"));
    ui.set_uploader_open(env_flag("KALPA_UPLOADER_OPEN"));
    ui.set_svm_open(env_flag("KALPA_SVM_OPEN"));
    ui.set_backup_restore_open(env_flag("KALPA_BACKUP_RESTORE_OPEN"));
    ui.set_characters_open(env_flag("KALPA_CHARACTERS_OPEN"));
    ui.set_safety_open(env_flag("KALPA_SAFETY_OPEN"));
    ui.set_migration_open(env_flag("KALPA_MIGRATION_OPEN"));
    let pack_hub_view = std::env::var("KALPA_PACK_HUB_VIEW")
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "create" | "create-details" | "details" => 1,
            "2" | "create-addons" | "addons" => 2,
            "3" | "install" | "detail" | "install-detail" => 3,
            "4" | "my" | "my-packs" | "installed" => 4,
            "5" | "import" | "share" | "share-code" => 5,
            _ => 0,
        })
        .unwrap_or(0);
    ui.set_pack_hub_view(pack_hub_view);
    if let Ok(path) = std::env::var("KALPA_PACK_HUB_IMPORT_FILE") {
        if !path.trim().is_empty() {
            ui.set_pack_hub_open(true);
            ui.set_pack_hub_view(5);
            ui.set_pack_hub_import_file_path(path.into());
        }
    }
    let uploader_view = std::env::var("KALPA_UPLOADER_VIEW")
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "uploading" | "manual-uploading" => 1,
            "2" | "live" | "live-ready" => 2,
            "3" | "live-running" | "running" | "streaming" => 3,
            _ => 0,
        })
        .unwrap_or(0);
    ui.set_uploader_view(uploader_view);
    let svm_view = std::env::var("KALPA_SVM_VIEW")
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "cleanup" => 1,
            "2" | "copy" | "copy-profile" => 2,
            "3" | "editor" => 3,
            _ => 0,
        })
        .unwrap_or(0);
    ui.set_svm_view(svm_view);
    let backup_restore_view = std::env::var("KALPA_BACKUP_RESTORE_VIEW")
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "label" | "custom-label" | "backup-label" => 1,
            "2" | "confirm" | "restore-confirm" | "restore" => 2,
            _ => 0,
        })
        .unwrap_or(0);
    ui.set_backup_restore_view(backup_restore_view);
    let settings_tab = std::env::var("KALPA_SETTINGS_TAB")
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "appearance" | "theme" | "themes" => 1,
            "2" | "tools" => 2,
            "3" | "data" => 3,
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
    env_flag_with_default(name, false)
}

fn env_flag_with_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(default)
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
    let low_memory = ui.global::<Tokens>().get_low_memory_preset();
    let (orb_one, orb_two, orb_three) = if low_memory {
        ((420, 0.18), (340, 0.13), (260, 0.09))
    } else {
        ((600, 0.20), (500, 0.15), (400, 0.10))
    };
    backdrop.set_orb_one(cached_blurred_orb_skin(
        orb_one.0,
        rgb_from_hex(&seed.orb1),
        orb_one.1,
    ));
    backdrop.set_orb_two(cached_blurred_orb_skin(
        orb_two.0,
        rgb_from_hex(&seed.orb2),
        orb_two.1,
    ));
    backdrop.set_orb_three(cached_blurred_orb_skin(
        orb_three.0,
        rgb_from_hex(&seed.orb3),
        orb_three.1,
    ));
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
        .map(|theme| ThemeSelection::with_skin(theme.colors.clone(), theme.skin_id.as_deref()))
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
    native_settings_store_paths().into_iter().next()
}

fn native_settings_store_paths() -> Vec<PathBuf> {
    if let Ok(path) = std::env::var("KALPA_NATIVE_STATE_DIR") {
        return vec![PathBuf::from(path).join("settings.json")];
    }

    let Some(appdata) = std::env::var("APPDATA").ok().map(PathBuf::from) else {
        return Vec::new();
    };

    vec![
        appdata.join("com.kalpa.desktop").join("settings.json"),
        appdata.join("Kalpa").join("native-settings.json"),
    ]
}

fn read_persisted_active_theme_id() -> Option<String> {
    for path in native_settings_store_paths() {
        match read_active_theme_id_from_settings_path(&path) {
            Ok(Some(theme_id)) => return Some(theme_id),
            Ok(None) => {}
            Err(error) => eprintln!("Failed to read active theme from {path:?}: {error}"),
        }
    }

    read_active_theme_id_from_path(&active_theme_store_path()?)
}

fn read_active_theme_id_from_settings_path(path: &Path) -> Result<Option<String>, String> {
    let Some(value) = read_settings_store_key_from_path(path, STORE_KEY_ACTIVE_THEME)? else {
        return Ok(None);
    };

    Ok(value
        .as_str()
        .map(str::trim)
        .filter(|theme_id| !theme_id.is_empty())
        .map(str::to_string))
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
    if let Some(path) = native_settings_store_path() {
        if let Err(error) = persist_active_theme_id_to_settings_path(&path, theme_id) {
            eprintln!("Failed to persist native theme selection: {error}");
        }
        return;
    }

    if let Some(path) = active_theme_store_path() {
        if let Err(error) = persist_active_theme_id_to_path(&path, theme_id) {
            eprintln!("Failed to persist native theme selection: {error}");
        }
    }
}

fn persist_active_theme_id_to_settings_path(path: &Path, theme_id: &str) -> Result<(), String> {
    let mut object = read_settings_store_object_from_path(path)?;
    object.insert(
        STORE_KEY_ACTIVE_THEME.to_string(),
        serde_json::Value::String(theme_id.to_string()),
    );
    write_settings_store_object_to_path(path, object)
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
    for path in native_settings_store_paths() {
        match read_custom_themes_from_settings_path(&path) {
            Ok(Some(themes)) => return themes,
            Ok(None) => {}
            Err(error) => eprintln!("Failed to read custom themes from {path:?}: {error}"),
        }
    }

    let Some(path) = custom_theme_store_path() else {
        return Vec::new();
    };
    read_custom_themes_from_path(&path).unwrap_or_default()
}

fn read_custom_themes_from_settings_path(path: &Path) -> Result<Option<Vec<CatalogTheme>>, String> {
    let Some(value) = read_settings_store_key_from_path(path, STORE_KEY_CUSTOM_THEMES)? else {
        return Ok(None);
    };

    serde_json::from_value::<Vec<CatalogTheme>>(value)
        .map(normalize_custom_themes)
        .map(Some)
        .map_err(|error| format!("Failed to parse production custom themes: {error}"))
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

    let contents = json_without_bom(&contents);
    serde_json::from_str::<NativeCustomThemeStore>(contents)
        .map(|store| store.themes)
        .or_else(|_| serde_json::from_str::<Vec<CatalogTheme>>(contents))
        .map(normalize_custom_themes)
        .map_err(|error| format!("Failed to parse custom themes: {error}"))
}

fn persist_custom_themes(custom_themes: &[CatalogTheme]) {
    if let Some(path) = native_settings_store_path() {
        if let Err(error) = persist_custom_themes_to_settings_path(&path, custom_themes) {
            eprintln!("Failed to persist native custom themes: {error}");
        }
        return;
    }

    if let Some(path) = custom_theme_store_path() {
        if let Err(error) = persist_custom_themes_to_path(&path, custom_themes) {
            eprintln!("Failed to persist native custom themes: {error}");
        }
    }
}

fn persist_custom_themes_to_settings_path(
    path: &Path,
    custom_themes: &[CatalogTheme],
) -> Result<(), String> {
    let mut object = read_settings_store_object_from_path(path)?;
    object.insert(
        STORE_KEY_CUSTOM_THEMES.to_string(),
        serde_json::to_value(normalize_custom_themes(custom_themes.to_vec()))
            .map_err(|error| format!("Failed to serialize production custom themes: {error}"))?,
    );
    write_settings_store_object_to_path(path, object)
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

fn normalize_custom_themes(themes: Vec<CatalogTheme>) -> Vec<CatalogTheme> {
    themes
        .into_iter()
        .map(|mut theme| {
            theme.category = "Custom".to_string();
            theme.skin_id = normalize_skin_id(theme.skin_id);
            theme
        })
        .collect()
}

fn read_settings_store_key_from_path(
    path: &Path,
    key: &str,
) -> Result<Option<serde_json::Value>, String> {
    Ok(read_settings_store_object_from_path(path)?.remove(key))
}

fn read_settings_store_object_from_path(
    path: &Path,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(format!("Failed to read settings store: {error}")),
    };

    let contents = json_without_bom(&contents);
    if contents.trim().is_empty() {
        return Ok(Default::default());
    }

    serde_json::from_str::<serde_json::Value>(contents)
        .map_err(|error| format!("Failed to parse settings store: {error}"))
        .map(|value| value.as_object().cloned().unwrap_or_default())
}

fn write_settings_store_object_to_path(
    path: &Path,
    object: serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&serde_json::Value::Object(object))
        .map_err(|error| format!("Failed to serialize settings store: {error}"))?;
    write_string_atomic(path, &json)
}

fn read_native_settings() -> NativeSettings {
    for path in native_settings_store_paths() {
        match read_native_settings_from_path(&path) {
            Ok(settings) => return settings,
            Err(error) => eprintln!("Failed to read native settings from {path:?}: {error}"),
        }
    }

    NativeSettings::default()
}

fn read_native_settings_from_path(path: &Path) -> Result<NativeSettings, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NativeSettings::default());
        }
        Err(error) => return Err(format!("Failed to read native settings: {error}")),
    };

    let contents = json_without_bom(&contents);
    if contents.trim().is_empty() {
        return Ok(NativeSettings::default());
    }

    let value = serde_json::from_str::<serde_json::Value>(contents)
        .map_err(|error| format!("Failed to parse native settings: {error}"))?;
    Ok(native_settings_from_store_value(&value))
}

fn json_without_bom(contents: &str) -> &str {
    contents.strip_prefix('\u{feff}').unwrap_or(contents)
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
    let mut settings = settings.clone();
    settings.conflict_policy = settings.conflict_policy.clamp(0, 2);
    settings.uploader_region = settings.uploader_region.clamp(1, 2);
    settings.uploader_visibility = settings.uploader_visibility.clamp(0, 2);
    let existing = fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok());
    let store_value = native_settings_to_store_value(&settings, existing);
    let json = serde_json::to_string_pretty(&store_value)
        .map_err(|error| format!("Failed to serialize native settings: {error}"))?;
    write_string_atomic(path, &json)
        .map_err(|error| format!("Failed to write native settings: {error}"))
}

fn bool_from_store_object(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<bool> {
    object.get(key).and_then(serde_json::Value::as_bool)
}

fn conflict_policy_from_store_value(value: &serde_json::Value) -> Option<i32> {
    if let Some(index) = value.as_i64() {
        return Some((index as i32).clamp(0, 2));
    }

    match value.as_str()? {
        "keep_mine" => Some(1),
        "take_update" => Some(2),
        "ask" => Some(0),
        _ => None,
    }
}

fn conflict_policy_to_store_value(index: i32) -> serde_json::Value {
    serde_json::Value::String(
        match index.clamp(0, 2) {
            1 => "keep_mine",
            2 => "take_update",
            _ => "ask",
        }
        .to_string(),
    )
}

fn native_performance_mode_from_store_value(value: &serde_json::Value) -> Option<bool> {
    match value.as_str()? {
        "native-slint" | "slint" | "native" | "low-memory" => Some(true),
        "webview" | "web" | "standard" => Some(false),
        _ => None,
    }
}

fn native_performance_mode_to_store_value(enabled: bool) -> serde_json::Value {
    serde_json::Value::String(if enabled { "native-slint" } else { "webview" }.to_string())
}

fn native_settings_from_store_value(value: &serde_json::Value) -> NativeSettings {
    let Some(object) = value.as_object() else {
        return NativeSettings::default();
    };

    let defaults = NativeSettings::default();
    let manual_official = bool_from_store_object(object, "manualUseOfficialUploader");
    let live_official = bool_from_store_object(object, "liveUseOfficialUploader");

    NativeSettings {
        auto_update: bool_from_store_object(object, "autoUpdate").unwrap_or(defaults.auto_update),
        warn_eso_running: bool_from_store_object(object, "warnEsoRunning")
            .or_else(|| {
                bool_from_store_object(object, "suppressEsoRunningWarning").map(|value| !value)
            })
            .unwrap_or(defaults.warn_eso_running),
        native_performance_mode: object
            .get(STORE_KEY_PERFORMANCE_MODE)
            .and_then(native_performance_mode_from_store_value)
            .unwrap_or(defaults.native_performance_mode),
        official_uploader: bool_from_store_object(object, "officialUploader")
            .or_else(|| {
                manual_official
                    .or(live_official)
                    .map(|manual| manual || live_official.unwrap_or(false))
            })
            .unwrap_or(defaults.official_uploader),
        auto_open_analysis: bool_from_store_object(object, "autoOpenAnalysis")
            .unwrap_or(defaults.auto_open_analysis),
        conflict_policy: object
            .get("conflictPolicy")
            .and_then(conflict_policy_from_store_value)
            .unwrap_or(defaults.conflict_policy)
            .clamp(0, 2),
        uploader_region: object
            .get("uploaderRegion")
            .and_then(serde_json::Value::as_i64)
            .map(|value| value as i32)
            .unwrap_or(defaults.uploader_region)
            .clamp(1, 2),
        uploader_visibility: object
            .get("uploaderVisibility")
            .and_then(serde_json::Value::as_i64)
            .map(|value| value as i32)
            .unwrap_or(defaults.uploader_visibility)
            .clamp(0, 2),
    }
}

fn native_settings_to_store_value(
    settings: &NativeSettings,
    existing: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut object = existing
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();

    object.remove("warnEsoRunning");
    object.remove("officialUploader");
    object.insert(
        "autoUpdate".to_string(),
        serde_json::Value::Bool(settings.auto_update),
    );
    object.insert(
        "suppressEsoRunningWarning".to_string(),
        serde_json::Value::Bool(!settings.warn_eso_running),
    );
    object.insert(
        STORE_KEY_PERFORMANCE_MODE.to_string(),
        native_performance_mode_to_store_value(settings.native_performance_mode),
    );
    object.insert(
        "manualUseOfficialUploader".to_string(),
        serde_json::Value::Bool(settings.official_uploader),
    );
    object.insert(
        "liveUseOfficialUploader".to_string(),
        serde_json::Value::Bool(settings.official_uploader),
    );
    object.insert(
        "autoOpenAnalysis".to_string(),
        serde_json::Value::Bool(settings.auto_open_analysis),
    );
    object.insert(
        "conflictPolicy".to_string(),
        conflict_policy_to_store_value(settings.conflict_policy),
    );
    object.insert(
        "uploaderRegion".to_string(),
        serde_json::Value::Number(settings.uploader_region.clamp(1, 2).into()),
    );
    object.insert(
        "uploaderVisibility".to_string(),
        serde_json::Value::Number(settings.uploader_visibility.clamp(0, 2).into()),
    );

    serde_json::Value::Object(object)
}

fn write_string_atomic(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create settings directory: {error}"))?;
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.json");
    let staging = path.with_file_name(format!("{file_name}.tmp-{}-{unique}", std::process::id()));

    let write_result = (|| -> Result<(), String> {
        let mut file = fs::File::create(&staging)
            .map_err(|error| format!("Failed to stage settings: {error}"))?;
        file.write_all(contents.as_bytes())
            .map_err(|error| format!("Failed to write staged settings: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Failed to sync staged settings: {error}"))?;
        drop(file);
        fs::rename(&staging, path).map_err(|error| format!("Failed to publish settings: {error}"))
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&staging);
    }

    write_result
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
        skin_id: theme.skin_id.clone().unwrap_or_default().into(),
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

fn normalize_skin_id(skin_id: Option<String>) -> Option<String> {
    let skin_id = skin_id?.trim().to_string();
    if skin_kind(Some(&skin_id)) == 0 {
        None
    } else {
        Some(skin_id)
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
    fn addon_meta_omits_empty_separators() {
        assert_eq!(addon_meta("1.2.3", "Author"), "1.2.3  \u{00b7} Author");
        assert_eq!(addon_meta("1.2.3", ""), "1.2.3");
        assert_eq!(addon_meta("", "Author"), "Author");
        assert_eq!(addon_meta("", ""), "");
        assert_eq!(addon_meta(" 1.2.3 ", " Author "), "1.2.3  \u{00b7} Author");
    }

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
    fn active_theme_id_round_trips_through_production_settings_store() {
        let root = test_temp_dir("active-theme-production");
        let path = root.join("settings.json");
        fs::create_dir_all(&root).expect("create settings directory");
        fs::write(&path, r#"{"autoUpdate":true}"#).expect("seed settings store");

        persist_active_theme_id_to_settings_path(&path, "apocrypha-ink")
            .expect("persist active theme");

        assert_eq!(
            read_active_theme_id_from_settings_path(&path)
                .expect("read active theme")
                .as_deref(),
            Some("apocrypha-ink")
        );

        let object = read_settings_store_object_from_path(&path).expect("read settings object");
        assert_eq!(
            object
                .get("autoUpdate")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

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
        assert_eq!(themes[0].skin_id.as_deref(), Some("nordic-runestone"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn custom_theme_store_round_trips_production_settings_key() {
        let root = test_temp_dir("custom-theme-production");
        let path = root.join("settings.json");
        fs::create_dir_all(&root).expect("create settings directory");
        fs::write(&path, r#"{"conflictPolicy":"ask"}"#).expect("seed settings store");
        let theme = sample_custom_theme("custom-one", "Custom One");

        persist_custom_themes_to_settings_path(&path, std::slice::from_ref(&theme))
            .expect("persist custom themes");
        let themes = read_custom_themes_from_settings_path(&path)
            .expect("read production custom themes")
            .expect("custom themes key exists");

        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].id, "custom-one");
        assert_eq!(themes[0].category, "Custom");
        assert_eq!(themes[0].skin_id.as_deref(), Some("nordic-runestone"));

        let object = read_settings_store_object_from_path(&path).expect("read settings object");
        assert_eq!(
            object
                .get("conflictPolicy")
                .and_then(serde_json::Value::as_str),
            Some("ask")
        );
        assert!(object
            .get(STORE_KEY_CUSTOM_THEMES)
            .and_then(serde_json::Value::as_array)
            .is_some());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn addons_path_reads_production_settings_key() {
        let root = test_temp_dir("addons-path-production");
        let path = root.join("settings.json");
        fs::create_dir_all(&root).expect("create settings directory");
        fs::write(
            &path,
            serde_json::json!({
                "addonsPath": "C:/Users/Example/Documents/Elder Scrolls Online/live/AddOns"
            })
            .to_string(),
        )
        .expect("seed settings store");

        let addons_path = read_addons_path_from_settings_path(&path)
            .expect("read addons path")
            .expect("addons path exists");
        assert_eq!(
            addons_path.to_string_lossy(),
            "C:/Users/Example/Documents/Elder Scrolls Online/live/AddOns"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn addons_path_persists_production_settings_key() {
        let root = test_temp_dir("addons-path-persist");
        let path = root.join("settings.json");
        fs::create_dir_all(&root).expect("create settings directory");
        fs::write(&path, r#"{"autoUpdate":true,"conflictPolicy":"ask"}"#)
            .expect("seed settings store");

        persist_addons_path_to_settings_path(&path, "D:/ESO/live/AddOns")
            .expect("persist addons path");

        let object = read_settings_store_object_from_path(&path).expect("read settings object");
        assert_eq!(
            object
                .get(STORE_KEY_ADDONS_PATH)
                .and_then(serde_json::Value::as_str),
            Some("D:/ESO/live/AddOns")
        );
        assert_eq!(
            object
                .get("autoUpdate")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            object
                .get("conflictPolicy")
                .and_then(serde_json::Value::as_str),
            Some("ask")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn installed_pack_refs_round_trip_through_production_settings_key() {
        let root = test_temp_dir("installed-packs-production");
        let path = root.join("settings.json");
        fs::create_dir_all(&root).expect("create settings directory");
        fs::write(&path, r#"{"autoUpdate":true}"#).expect("seed settings store");
        let refs = vec![
            NativeInstalledPackRef {
                pack_id: "pack-1".to_string(),
                title: "Trial Essentials".to_string(),
                pack_type: "build".to_string(),
                author_name: "Spike'jo".to_string(),
                addon_count: 12,
                installed_at: "2026-07-02T18:45:00Z".to_string(),
            },
            NativeInstalledPackRef {
                pack_id: "pack-1".to_string(),
                title: "Duplicate".to_string(),
                pack_type: "addon-pack".to_string(),
                author_name: "Ignored".to_string(),
                addon_count: 1,
                installed_at: "2026-07-03T00:00:00Z".to_string(),
            },
        ];

        persist_installed_pack_refs_to_settings_path(&path, &refs)
            .expect("persist installed packs");
        let restored = read_installed_pack_refs_from_settings_path(&path)
            .expect("read installed packs")
            .expect("installed packs key exists");

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].pack_id, "pack-1");
        assert_eq!(restored[0].pack_type, "build-pack");
        assert_eq!(restored[0].addon_count, 12);

        let object = read_settings_store_object_from_path(&path).expect("read settings object");
        assert_eq!(
            object
                .get("autoUpdate")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(object
            .get(STORE_KEY_INSTALLED_PACKS)
            .and_then(serde_json::Value::as_array)
            .is_some());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn installed_pack_ref_maps_to_native_card_entry() {
        let entry = pack_hub_installed_entry_from_ref(NativeInstalledPackRef {
            pack_id: "build-pack-7".to_string(),
            title: "Cloudrest Build".to_string(),
            pack_type: "build-pack".to_string(),
            author_name: "Alyx".to_string(),
            addon_count: 5,
            installed_at: "2026-07-02T18:45:00Z".to_string(),
        });

        assert_eq!(entry.pack_id.as_str(), "build-pack-7");
        assert_eq!(entry.pack_type_label.as_str(), "Build Pack");
        assert_eq!(entry.addon_count.as_str(), "5 addons");
        assert_eq!(entry.installed_label.as_str(), "Installed Jul 2, 2026");
        assert_eq!(entry.monogram.as_str(), "CB");
        assert_eq!(entry.type_kind, 1);
    }

    #[test]
    fn settings_store_reader_accepts_utf8_bom() {
        let root = test_temp_dir("settings-bom");
        let path = root.join("settings.json");
        fs::create_dir_all(&root).expect("create settings directory");
        fs::write(&path, "\u{feff}{\"addonsPath\":\"D:/ESO/live/AddOns\"}")
            .expect("seed bom settings store");

        assert_eq!(
            read_addons_path_from_settings_path(&path)
                .expect("read addons path")
                .expect("addons path exists")
                .to_string_lossy(),
            "D:/ESO/live/AddOns"
        );

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
        assert_eq!(first.skin_id.as_str(), "nordic-runestone");
        assert!(
            entries
                .iter()
                .any(|entry| entry.id.as_str() == "eso-gold"
                    && entry.category_heading.as_str() == "ESO"),
            "built-in categories should follow custom themes"
        );
    }

    #[test]
    fn custom_theme_drafts_are_color_only() {
        let base = sample_custom_theme("skinned-base", "Skinned Base");
        let draft = custom_theme_draft_from_base(&base, "Copy");

        assert_eq!(draft.category, "Custom");
        assert_eq!(draft.colors.primary, base.colors.primary);
        assert_eq!(draft.colors.background, base.colors.background);
        assert_eq!(draft.skin_id, None);
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

        assert_ne!(imported.id, "imported-one");
        assert_eq!(imported.name, "Imported One");
        assert_eq!(imported.colors.accent, "#AABBCC");
        assert_eq!(imported.category, "Custom");
        assert_eq!(imported.skin_id.as_deref(), Some("nordic-runestone"));

        let colors_only = serde_json::json!({ "colors": theme.colors.clone() }).to_string();
        let imported = parse_imported_custom_theme(&colors_only).expect("parse colors-only theme");
        assert_eq!(imported.name, "Imported Theme");
        assert_eq!(imported.description, "Imported custom theme.");

        let mut invalid = theme;
        invalid.colors.primary = "not-a-color".to_string();
        let json = serde_json::to_string(&invalid).expect("serialize invalid theme");
        assert!(parse_imported_custom_theme(&json).is_none());

        let mut unknown_skin = sample_custom_theme("unknown-skin", "Unknown Skin");
        unknown_skin.skin_id = Some("missing-skin".to_string());
        let json = serde_json::to_string(&unknown_skin).expect("serialize unknown skin theme");
        let imported = parse_imported_custom_theme(&json).expect("parse unknown skin theme");
        assert_eq!(imported.skin_id, None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_auto_place_is_opt_in() {
        assert!(!native_auto_place_enabled(None));
        assert!(!native_auto_place_enabled(Some("0")));
        assert!(!native_auto_place_enabled(Some("false")));
        assert!(native_auto_place_enabled(Some("1")));
        assert!(native_auto_place_enabled(Some("true")));
        assert!(native_auto_place_enabled(Some("YES")));
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
    fn render_config_defaults_to_smooth_femtovg_preset() {
        assert_eq!(
            render_config_from_inputs(None, None, None),
            NativeRenderConfig {
                backend: "winit-femtovg".to_string(),
                preset: NativeRenderPreset::Standard,
            }
        );
    }

    #[test]
    fn render_config_standard_preset_selects_femtovg_unless_backend_is_explicit() {
        assert_eq!(
            render_config_from_inputs(Some("standard"), None, None),
            NativeRenderConfig {
                backend: "winit-femtovg".to_string(),
                preset: NativeRenderPreset::Standard,
            }
        );
        assert_eq!(
            render_config_from_inputs(Some("standard"), Some("winit-femtovg"), None),
            NativeRenderConfig {
                backend: "winit-femtovg".to_string(),
                preset: NativeRenderPreset::Standard,
            }
        );
    }

    #[test]
    fn render_config_derives_preset_from_backend_without_explicit_preset() {
        assert_eq!(
            render_config_from_inputs(None, Some("winit-skia"), None).preset,
            NativeRenderPreset::Standard
        );
        assert_eq!(
            render_config_from_inputs(None, Some("winit-software"), None).preset,
            NativeRenderPreset::LowMemory
        );
    }

    #[test]
    fn current_app_version_comes_from_tauri_config() {
        assert_eq!(current_app_version(), "0.1.0-beta.9");
    }

    #[test]
    fn app_version_compare_handles_beta_identifiers() {
        assert!(app_version_is_newer("0.1.0-beta.10", "0.1.0-beta.9"));
        assert!(app_version_is_newer("0.1.0", "0.1.0-beta.10"));
        assert!(app_version_is_newer("v0.1.1-beta.1", "0.1.0"));
        assert!(!app_version_is_newer("0.1.0-beta.9", "0.1.0-beta.9"));
        assert!(!app_version_is_newer("0.1.0-beta.8", "0.1.0-beta.9"));
    }

    #[test]
    fn app_update_manifest_selects_windows_nsis_and_ignores_same_version() {
        let manifest = NativeAppUpdateManifest {
            version: "0.1.0-beta.10".to_string(),
            platforms: HashMap::from([
                (
                    "windows-x86_64".to_string(),
                    NativeAppUpdatePlatform {
                        url: "https://example.invalid/plain.exe".to_string(),
                        signature: "plain-sig".to_string(),
                    },
                ),
                (
                    "windows-x86_64-nsis".to_string(),
                    NativeAppUpdatePlatform {
                        url: "https://example.invalid/nsis.exe".to_string(),
                        signature: "nsis-sig".to_string(),
                    },
                ),
            ]),
        };

        let update =
            native_app_update_info_from_manifest(manifest, "0.1.0-beta.9").expect("new update");
        assert_eq!(update.version, "0.1.0-beta.10");
        assert_eq!(update.url, "https://example.invalid/nsis.exe");
        assert_eq!(update.signature, "nsis-sig");

        let same_version = NativeAppUpdateManifest {
            version: "0.1.0-beta.9".to_string(),
            platforms: HashMap::new(),
        };
        assert!(native_app_update_info_from_manifest(same_version, "0.1.0-beta.9").is_none());
    }

    #[test]
    fn native_uploader_log_list_skips_noncombat_logs() {
        let root = test_temp_dir("uploader-logs");
        let logs_dir = root.join("Logs");
        fs::create_dir_all(&logs_dir).expect("create logs dir");
        fs::write(logs_dir.join("Encounter.log"), "0,BEGIN_LOG,1,15\n")
            .expect("write encounter log");
        fs::write(logs_dir.join("client.log"), "diagnostics").expect("write client log");
        fs::write(logs_dir.join("Interface.log"), "lua errors").expect("write interface log");

        let logs = list_native_uploader_logs(&logs_dir).expect("list logs");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].file_name, "Encounter.log");
        assert!(logs[0].active);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_uploader_preflight_counts_sessions_and_fights() {
        let root = test_temp_dir("uploader-preflight");
        fs::create_dir_all(&root).expect("create temp dir");
        let path = root.join("Encounter.log");
        fs::write(
            &path,
            concat!(
                "0,BEGIN_LOG,1780641553946,15,\"NA Megaserver\"\n",
                "1000,BEGIN_COMBAT\n",
                "62000,END_COMBAT\n",
                "70000,BEGIN_LOG,1780641623946,15,\"NA Megaserver\"\n",
                "72000,BEGIN_COMBAT\n",
                "76000,END_COMBAT\n",
            ),
        )
        .expect("write log");

        let preflight = scan_native_uploader_log(&path).expect("scan log");
        assert_eq!(preflight.sessions, 2);
        assert_eq!(preflight.total_fights, 2);
        assert_eq!(preflight.fights.len(), 2);
        assert_eq!(preflight.fights[0].start_ms, 1000);
        assert_eq!(preflight.fights[0].end_ms, 62000);
        assert!(!preflight.truncated);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn official_uploader_candidates_prefer_archon_app() {
        let roots = [
            PathBuf::from("C:/Program Files"),
            PathBuf::from("C:/Users/me/AppData/Local/Programs"),
        ];
        let candidates = official_uploader_candidates_from_roots(&roots)
            .into_iter()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>();

        let archon = candidates
            .iter()
            .position(|path| path.ends_with("Archon App/Archon App.exe"))
            .expect("Archon App candidate is present");
        let legacy = candidates
            .iter()
            .position(|path| path.ends_with("ESO Logs Uploader/ESO Logs Uploader.exe"))
            .expect("legacy uploader candidate is present");

        assert!(
            archon < legacy,
            "Archon App should be preferred: {candidates:?}"
        );
    }

    #[test]
    fn addon_write_status_message_adds_eso_reload_notice() {
        assert_eq!(
            addon_write_status_message("Updated 1 addon.", true),
            format!("{ESO_RUNNING_ADDON_NOTICE} Updated 1 addon.")
        );
        assert_eq!(
            addon_write_status_message("", true),
            ESO_RUNNING_ADDON_NOTICE
        );
        assert_eq!(
            addon_write_status_message("Updated 1 addon.", false),
            "Updated 1 addon."
        );
    }

    #[test]
    fn native_settings_round_trip_and_clamp_conflict_policy() {
        let root = test_temp_dir("native-settings");
        let path = root.join("settings.json");
        fs::create_dir_all(&root).expect("create temp settings directory");
        fs::write(
            &path,
            serde_json::json!({
                "addonsPath": "C:/Games/Elder Scrolls Online/live/AddOns",
                "appearance.activeThemeId": "nordic-runestone",
                "warnEsoRunning": false,
                "officialUploader": false
            })
            .to_string(),
        )
        .expect("seed settings");
        let settings = NativeSettings {
            auto_update: true,
            warn_eso_running: false,
            native_performance_mode: false,
            official_uploader: true,
            auto_open_analysis: true,
            conflict_policy: 9,
            uploader_region: 7,
            uploader_visibility: -1,
        };

        persist_native_settings_to_path(&path, &settings).expect("persist settings");
        let restored = read_native_settings_from_path(&path).expect("read settings");

        assert!(restored.auto_update);
        assert!(!restored.warn_eso_running);
        assert!(!restored.native_performance_mode);
        assert!(restored.official_uploader);
        assert!(restored.auto_open_analysis);
        assert_eq!(restored.conflict_policy, 2);
        assert_eq!(restored.uploader_region, 2);
        assert_eq!(restored.uploader_visibility, 0);

        let value = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(&path).expect("read settings json"),
        )
        .expect("parse settings json");
        let object = value.as_object().expect("settings object");
        assert_eq!(
            object.get("addonsPath").and_then(serde_json::Value::as_str),
            Some("C:/Games/Elder Scrolls Online/live/AddOns")
        );
        assert_eq!(
            object
                .get("appearance.activeThemeId")
                .and_then(serde_json::Value::as_str),
            Some("nordic-runestone")
        );
        assert_eq!(
            object
                .get("suppressEsoRunningWarning")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            object
                .get("performanceMode")
                .and_then(serde_json::Value::as_str),
            Some("webview")
        );
        assert_eq!(
            object
                .get("manualUseOfficialUploader")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            object
                .get("liveUseOfficialUploader")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            object
                .get("conflictPolicy")
                .and_then(serde_json::Value::as_str),
            Some("take_update")
        );
        assert_eq!(
            object
                .get("uploaderRegion")
                .and_then(serde_json::Value::as_i64),
            Some(2)
        );
        assert_eq!(
            object
                .get("uploaderVisibility")
                .and_then(serde_json::Value::as_i64),
            Some(0)
        );
        assert!(object.get("warnEsoRunning").is_none());
        assert!(object.get("officialUploader").is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_settings_read_production_store_keys() {
        let root = test_temp_dir("native-settings-production");
        let path = root.join("settings.json");
        fs::create_dir_all(&root).expect("create temp settings directory");
        fs::write(
            &path,
            serde_json::json!({
                "autoUpdate": true,
                "suppressEsoRunningWarning": true,
                "manualUseOfficialUploader": false,
                "liveUseOfficialUploader": true,
                "performanceMode": "native-slint",
                "autoOpenAnalysis": true,
                "conflictPolicy": "keep_mine",
                "uploaderRegion": 2,
                "uploaderVisibility": 1
            })
            .to_string(),
        )
        .expect("seed production settings");

        let restored = read_native_settings_from_path(&path).expect("read settings");
        assert!(restored.auto_update);
        assert!(!restored.warn_eso_running);
        assert!(restored.native_performance_mode);
        assert!(restored.official_uploader);
        assert!(restored.auto_open_analysis);
        assert_eq!(restored.conflict_policy, 1);
        assert_eq!(restored.uploader_region, 2);
        assert_eq!(restored.uploader_visibility, 1);

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
    fn addon_selection_defers_file_browser_until_visible_or_editing() {
        assert!(!needs_file_browser_refresh_on_addon_selection(
            false, false, false
        ));
        assert!(needs_file_browser_refresh_on_addon_selection(
            true, false, false
        ));
        assert!(needs_file_browser_refresh_on_addon_selection(
            false, true, false
        ));
        assert!(needs_file_browser_refresh_on_addon_selection(
            false, false, true
        ));
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
    fn native_tag_persistence_writes_metadata_tags() {
        let root = test_temp_dir("native-tags");
        fs::create_dir_all(root).expect("create addons root");

        let tags = tag_model(vec!["favorite"]);
        let tags = add_custom_tag(&tags, "PvP Build").expect("add custom tag");
        let active = active_tag_ids(&tags);
        assert_eq!(
            active,
            vec!["favorite".to_string(), "pvp-build".to_string()]
        );

        persist_addon_tag_model(root, "TaggedAddon", &tags).expect("persist tags");
        let store = metadata::load_metadata(root);
        let meta = store.addons.get("TaggedAddon").expect("metadata entry");
        assert_eq!(meta.tags, active);

        let cleared = tag_model(vec![]);
        persist_addon_tag_model(root, "TaggedAddon", &cleared).expect("clear tags");
        let store = metadata::load_metadata(root);
        let meta = store.addons.get("TaggedAddon").expect("metadata entry");
        assert!(meta.tags.is_empty());
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
    fn native_disable_enable_renames_real_addon_folder() {
        let root = test_temp_dir("native-disable-enable");
        let addon_dir = root.join("ToggleAddon");
        fs::create_dir_all(&addon_dir).expect("create addon folder");
        fs::write(
            addon_dir.join("ToggleAddon.txt"),
            "## Title: Toggle Addon\n",
        )
        .expect("write manifest");

        set_addon_disabled_on_disk(root, "ToggleAddon", true).expect("disable addon");
        assert!(!root.join("ToggleAddon").exists());
        assert!(root.join("ToggleAddon.disabled").is_dir());

        set_addon_disabled_on_disk(root, "ToggleAddon", false).expect("enable addon");
        assert!(root.join("ToggleAddon").is_dir());
        assert!(!root.join("ToggleAddon.disabled").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_disable_enable_rejects_invalid_folder_names() {
        let root = test_temp_dir("native-disable-invalid");
        fs::create_dir_all(root).expect("create root");

        assert!(set_addon_disabled_on_disk(root, "../Bad", true).is_err());
        assert!(set_addon_disabled_on_disk(root, "Bad/Name", true).is_err());
        assert!(remove_addon_from_disk(root, "Bad\\Name").is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_remove_deletes_enabled_and_disabled_copies_and_metadata() {
        let root = test_temp_dir("native-remove-addon");
        fs::create_dir_all(root.join("DuplicateAddon")).expect("create enabled addon");
        fs::create_dir_all(root.join("DuplicateAddon.disabled")).expect("create disabled addon");

        let mut store = metadata::MetadataStore::default();
        metadata::record_install(
            &mut store,
            "DuplicateAddon",
            1360,
            "1.0",
            "https://example.test",
        );
        metadata::save_metadata(root, &store).expect("save metadata");

        remove_addon_from_disk(root, "DuplicateAddon").expect("remove addon");

        assert!(!root.join("DuplicateAddon").exists());
        assert!(!root.join("DuplicateAddon.disabled").exists());
        assert!(!metadata::load_metadata(root)
            .addons
            .contains_key("DuplicateAddon"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn real_file_editor_reads_writes_nested_text_files() {
        let root = test_temp_dir("real-file-editor");
        let addon_dir = root.join("EditableAddon");
        fs::create_dir_all(addon_dir.join("lang")).expect("create addon folders");
        fs::write(
            addon_dir.join("EditableAddon.txt"),
            "## Title: Editable Addon\n## Author: Tester\n## Version: 1.0\n",
        )
        .expect("write addon manifest");
        fs::write(addon_dir.join("lang/en.lua"), "local value = 1\n").expect("write lua file");
        let baseline_hash =
            hash_file_sha256(&addon_dir.join("lang/en.lua")).expect("hash baseline file");
        fs::create_dir_all(root.join(".kalpa-hashes")).expect("create hash dir");
        fs::write(
            hash_manifest_path(root, "EditableAddon"),
            serde_json::to_string_pretty(&NativeHashManifest {
                addon_folder: "EditableAddon".to_string(),
                recorded_at: "2026-07-01".to_string(),
                installed_version: "1.0".to_string(),
                files: HashMap::from([("lang/en.lua".to_string(), baseline_hash)]),
                ..Default::default()
            })
            .expect("serialize hash manifest"),
        )
        .expect("write hash manifest");

        let files = real_file_entries(root, "EditableAddon").expect("load real file tree");
        assert!(files
            .iter()
            .any(|entry| !entry.folder && entry.relative_path.as_str() == "lang/en.lua"));

        let original = read_text_file(root, "EditableAddon", "lang/en.lua").expect("read lua file");
        assert_eq!(original, "local value = 1\n");

        write_text_file(root, "EditableAddon", "lang/en.lua", "local value = 2\n")
            .expect("write lua file");
        assert_eq!(
            fs::read_to_string(addon_dir.join("lang/en.lua")).expect("read saved lua file"),
            "local value = 2\n"
        );
        let manifest = load_hash_manifest(root, "EditableAddon").expect("read hash manifest");
        assert_eq!(manifest.modified_files, vec!["lang/en.lua".to_string()]);

        write_text_file(root, "EditableAddon", "lang/en.lua", "local value = 1\n")
            .expect("write original lua file");
        let manifest = load_hash_manifest(root, "EditableAddon").expect("read hash manifest");
        assert!(manifest.modified_files.is_empty());
        assert!(write_text_file(root, "EditableAddon", "../escape.lua", "bad").is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn saved_variable_classification_matches_installed_boundaries() {
        let installed = HashSet::from([
            "CombatMetrics".to_string(),
            "HarvestMap".to_string(),
            "LibAddonMenu-2.0".to_string(),
        ]);

        assert!(matches!(
            classify_saved_variable("CombatMetrics", &installed),
            SavedVariableStatus::Installed
        ));
        assert!(matches!(
            classify_saved_variable("CombatMetricsFightData", &installed),
            SavedVariableStatus::Installed
        ));
        assert!(matches!(
            classify_saved_variable("CombatMetrics_Data", &installed),
            SavedVariableStatus::Installed
        ));
        assert!(matches!(
            classify_saved_variable("HarvestMap-Extra", &installed),
            SavedVariableStatus::Installed
        ));
        assert!(matches!(
            classify_saved_variable("AccountSettings", &installed),
            SavedVariableStatus::System
        ));
        assert!(matches!(
            classify_saved_variable("CombatMetricsextra", &installed),
            SavedVariableStatus::Orphaned
        ));
        assert!(matches!(
            classify_saved_variable("RemovedAddon", &installed),
            SavedVariableStatus::Orphaned
        ));
    }

    #[test]
    fn saved_variable_profile_parser_ignores_accountwide_and_nested_keys() {
        let profiles = extract_saved_variable_profiles(
            r#"
CombatMetrics_SavedVariables = {
    ["Default"] = {
        ["@Account"] = {
            ["$AccountWide"] = {
                ["nested"] = "{ not a profile }",
            },
            ["Main"] = {
                ["note"] = "contains } in a string",
            },
            ["Alt"] = {
                -- comment with { braces } should be ignored
            },
        },
    },
}
"#,
        );

        assert_eq!(
            profiles,
            BTreeSet::from(["Alt".to_string(), "Main".to_string()])
        );
    }

    #[test]
    fn short_date_uses_readable_month_names() {
        assert_eq!(format_short_date(0), "Jan 1, 1970");
        assert_eq!(format_short_date(1_782_864_000), "Jul 1, 2026");
    }

    #[test]
    fn saved_variable_entries_sort_and_mark_statuses() {
        let root = test_temp_dir("saved-variable-entries");
        let addons_root = root.join("AddOns");
        let sv_dir = root.join("SavedVariables");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&sv_dir).expect("create saved variables root");
        fs::write(
            sv_dir.join("CombatMetricsFightData.lua"),
            format!("CombatMetricsFightData = {{}}\n{}", "x".repeat(4096)),
        )
        .expect("write installed saved variable");
        fs::write(
            sv_dir.join("OldAddon.lua"),
            "OldAddon = {\n    [\"Default\"] = {\n    },\n}\n",
        )
        .expect("write orphaned saved variable");
        fs::write(sv_dir.join("AccountSettings.lua"), "AccountSettings = {}\n")
            .expect("write system saved variable");
        fs::write(sv_dir.join("readme.txt"), "ignored").expect("write ignored file");

        let addons = vec![addon_entry(
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
        let entries = saved_variable_entries(&addons_root, &addons).expect("load saved variables");

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].title.as_str(), "CombatMetricsFightData");
        assert_eq!(entries[0].meter_width, 56);
        assert!(!entries[0].orphaned);
        assert!(!entries[0].system);
        assert!(!entries
            .iter()
            .any(|entry| entry.file_name.as_str() == "readme.txt"));

        let account_settings = entries
            .iter()
            .find(|entry| entry.title.as_str() == "AccountSettings")
            .expect("system entry exists");
        assert!(account_settings.system);
        assert!(!account_settings.orphaned);

        let orphaned = entries
            .iter()
            .find(|entry| entry.title.as_str() == "OldAddon")
            .expect("orphaned entry exists");
        assert!(orphaned.orphaned);
        assert!(!orphaned.system);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_svm_copy_state_uses_profiles_across_files() {
        let mut state = SvmCopyState::default();
        state.replace_files(vec![
            SvmProfileFile {
                file_name: "One.lua".to_string(),
                addon_name: "One".to_string(),
                profiles: vec!["Main".to_string()],
            },
            SvmProfileFile {
                file_name: "Two.lua".to_string(),
                addon_name: "Two".to_string(),
                profiles: vec!["Alt".to_string(), "Bank".to_string()],
            },
        ]);

        assert_eq!(state.source_key(), Some("Main"));
        assert_eq!(
            state.destination_choices(),
            vec!["Alt".to_string(), "Bank".to_string()]
        );
        assert_eq!(
            state.selection().map(|selection| selection.dest_key),
            Some("Alt".to_string())
        );

        state.select_next_dest();
        assert_eq!(
            state.selection().map(|selection| selection.dest_key),
            Some("Bank".to_string())
        );

        state.select_next_file();
        assert_eq!(state.source_key(), Some("Alt"));
        assert_eq!(
            state.destination_choices(),
            vec!["Bank".to_string(), "Main".to_string()]
        );
    }

    #[test]
    fn native_svm_profile_copy_writes_destination_profile() {
        let root = test_temp_dir("native-svm-profile-copy");
        let addons_root = root.join("AddOns");
        let sv_dir = root.join("SavedVariables");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&sv_dir).expect("create saved variables root");
        fs::write(
            sv_dir.join("TestAddon.lua"),
            r#"TestAddon_SavedVariables = {
    ["Default"] = {
        ["@Account"] = {
            ["Main"] = {
                ["enabled"] = true,
            },
        },
    },
}
"#,
        )
        .expect("write saved variable");

        let selection = SvmCopySelection {
            file_name: "TestAddon.lua".to_string(),
            addon_name: "TestAddon".to_string(),
            source_key: "Main".to_string(),
            dest_key: "Alt".to_string(),
        };

        copy_svm_profile_selection(&addons_root, &selection).expect("copy profile");
        let updated =
            fs::read_to_string(sv_dir.join("TestAddon.lua")).expect("read copied saved variable");
        assert!(updated.contains("[\"Main\"]"));
        assert!(updated.contains("[\"Alt\"]"));
        assert!(sv_dir.join("TestAddon.lua.bak").is_file());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_svm_editor_loads_toggles_previews_and_saves_boolean() {
        let root = test_temp_dir("native-svm-editor");
        let addons_root = root.join("AddOns");
        let sv_dir = root.join("SavedVariables");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&sv_dir).expect("create saved variables root");
        fs::write(
            sv_dir.join("TestAddon.lua"),
            r#"TestAddon_SavedVariables = {
    ["Default"] = {
        ["@Account"] = {
                ["Main"] = {
                ["enabled"] = false,
                ["size"] = 12,
                ["theme"] = "classic",
            },
        },
    },
}
"#,
        )
        .expect("write saved variable");

        let mut state = SvmEditorState::default();
        state.replace_files(vec![SvmEditorFile {
            file_name: "TestAddon.lua".to_string(),
            addon_name: "TestAddon".to_string(),
        }]);
        load_svm_editor_selected_file(&addons_root, &mut state).expect("load editor file");

        assert_eq!(
            state.selected_path,
            vec![
                "TestAddon_SavedVariables".to_string(),
                "Default".to_string(),
                "@Account".to_string(),
                "Main".to_string()
            ]
        );
        assert!(
            svm_editor_tree_entries(state.tree.as_ref().unwrap(), &state.selected_path, false, "")
                .iter()
                .any(|entry| entry.active && entry.label.as_str() == "Main")
        );
        assert!(
            svm_editor_tree_entries(state.tree.as_ref().unwrap(), &state.selected_path, true, "")
                .len()
                >= svm_editor_tree_entries(
                    state.tree.as_ref().unwrap(),
                    &state.selected_path,
                    false,
                    ""
                )
                .len()
        );

        let settings = svm_editor_setting_entries(&state);
        assert!(settings
            .iter()
            .any(|entry| entry.key_name.as_str() == "enabled" && !entry.checked));
        assert!(settings
            .iter()
            .any(|entry| entry.key_name.as_str() == "theme" && entry.value.as_str() == "classic"));
        let enabled_index = settings
            .iter()
            .position(|entry| entry.key_name.as_str() == "enabled")
            .expect("enabled setting exists");

        toggle_svm_editor_setting(&mut state, enabled_index).expect("toggle enabled");
        assert!(state.dirty);

        let settings = svm_editor_setting_entries(&state);
        let theme_index = settings
            .iter()
            .position(|entry| entry.key_name.as_str() == "theme")
            .expect("theme setting exists");
        edit_svm_editor_setting(&mut state, theme_index, "dark").expect("edit theme string");

        let settings = svm_editor_setting_entries(&state);
        let size_index = settings
            .iter()
            .position(|entry| entry.key_name.as_str() == "size")
            .expect("size setting exists");
        edit_svm_editor_setting(&mut state, size_index, "18").expect("edit size number");

        let settings = svm_editor_setting_entries(&state);
        assert!(settings
            .iter()
            .any(|entry| entry.key_name.as_str() == "enabled" && entry.checked));
        assert!(settings
            .iter()
            .any(|entry| entry.key_name.as_str() == "theme" && entry.value.as_str() == "dark"));
        assert!(settings
            .iter()
            .any(|entry| entry.key_name.as_str() == "size" && entry.value.as_str() == "18"));

        let account_path = serde_json::to_string(&vec![
            "TestAddon_SavedVariables".to_string(),
            "Default".to_string(),
            "@Account".to_string(),
        ])
        .expect("serialize path");
        select_svm_editor_tree_path(&mut state, &account_path).expect("select account branch");
        assert_eq!(
            state.selected_path,
            vec![
                "TestAddon_SavedVariables".to_string(),
                "Default".to_string(),
                "@Account".to_string()
            ]
        );

        preview_svm_editor_file(&addons_root, &mut state).expect("preview editor change");
        assert!(state.message.contains("pending change"));
        let raw_preview = svm_editor_raw_lua(&state).expect("serialize raw editor lua");
        assert!(
            raw_preview.contains("enabled = true") || raw_preview.contains("[\"enabled\"] = true")
        );
        assert!(
            raw_preview.contains("theme = \"dark\"")
                || raw_preview.contains("[\"theme\"] = \"dark\"")
        );
        assert!(raw_preview.contains("size = 18") || raw_preview.contains("[\"size\"] = 18"));

        save_svm_editor_file(&addons_root, &mut state).expect("save editor change");
        assert!(!state.dirty);
        let updated =
            fs::read_to_string(sv_dir.join("TestAddon.lua")).expect("read saved variable");
        assert!(updated.contains("enabled = true") || updated.contains("[\"enabled\"] = true"));
        assert!(updated.contains("theme = \"dark\"") || updated.contains("[\"theme\"] = \"dark\""));
        assert!(updated.contains("size = 18") || updated.contains("[\"size\"] = 18"));
        assert!(sv_dir.join("TestAddon.lua.bak").is_file());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_saved_variable_cleanup_deletes_orphans_and_keeps_installed_files() {
        let root = test_temp_dir("saved-variable-cleanup");
        let addons_root = root.join("AddOns");
        let sv_dir = root.join("SavedVariables");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&sv_dir).expect("create saved variables root");
        fs::write(sv_dir.join("InstalledAddon.lua"), "InstalledAddon = {}\n")
            .expect("write installed saved variable");
        fs::write(sv_dir.join("RemovedAddon.lua"), "RemovedAddon = {}\n")
            .expect("write orphaned saved variable");

        let addons = vec![addon_entry(
            "Installed Addon",
            "InstalledAddon",
            "100",
            "Tester",
            "1.0",
            "101048",
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
        )];
        let entries = saved_variable_entries(&addons_root, &addons).expect("load saved variables");
        let orphaned = entries
            .iter()
            .filter(|entry| entry.orphaned)
            .cloned()
            .collect::<Vec<_>>();

        let deleted =
            clean_saved_variable_orphans(&addons_root, &orphaned).expect("clean orphaned files");

        assert_eq!(deleted, 1);
        assert!(sv_dir.join("InstalledAddon.lua").is_file());
        assert!(!sv_dir.join("RemovedAddon.lua").exists());
        let backup_root = root.join("kalpa-backups");
        let backup_files = fs::read_dir(&backup_root)
            .expect("backup root exists")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("RemovedAddon.lua"))
            .collect::<Vec<_>>();
        assert!(backup_files.iter().any(|path| path.is_file()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn settings_backup_snapshots_classify_sort_and_exclude_dotfiles() {
        let root = test_temp_dir("settings-backup-list");
        let addons_root = root.join("AddOns");
        let backups_root = root.join("kalpa-backups");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(backups_root.join("Manual One")).expect("create manual backup");
        fs::create_dir_all(backups_root.join("auto-before-restore-100"))
            .expect("create safety backup");
        fs::create_dir_all(backups_root.join("char-Alt-NA-backup"))
            .expect("create character backup");
        fs::write(backups_root.join("Manual One").join("A.lua"), "manual")
            .expect("write manual file");
        fs::write(
            backups_root
                .join("auto-before-restore-100")
                .join("Safety.lua"),
            "safety",
        )
        .expect("write safety file");
        fs::write(
            backups_root
                .join("char-Alt-NA-backup")
                .join(".kalpa-char-backup.json"),
            "{}",
        )
        .expect("write character marker");
        fs::write(
            backups_root
                .join("char-Alt-NA-backup")
                .join("Character.lua"),
            "character",
        )
        .expect("write character file");

        let snapshots = settings_backup_snapshots(&addons_root).expect("list backups");
        assert_eq!(snapshots.len(), 3);
        assert!(snapshots
            .iter()
            .any(|snapshot| snapshot.kind == BACKUP_KIND_MANUAL
                && snapshot.entry.kind_label.as_str() == "Manual"
                && snapshot.entry.file_count == 1));
        assert!(snapshots
            .iter()
            .any(|snapshot| snapshot.kind == BACKUP_KIND_SAFETY
                && snapshot.entry.display_name.as_str() == "Auto-saved before restore"));
        let character = snapshots
            .iter()
            .find(|snapshot| snapshot.kind == BACKUP_KIND_CHARACTER)
            .expect("character backup exists");
        assert_eq!(character.entry.display_name.as_str(), "Alt-NA-backup");
        assert_eq!(character.entry.file_count, 1);
        assert!(!character.entry.restorable);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn settings_backup_create_restore_and_delete_round_trip() {
        let root = test_temp_dir("settings-backup-roundtrip");
        let addons_root = root.join("AddOns");
        let sv_dir = root.join("SavedVariables");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&sv_dir).expect("create saved variables root");
        fs::write(sv_dir.join("Live.lua"), "current").expect("write live file");
        fs::write(sv_dir.join("Other.lua"), "other").expect("write other file");

        let summary = create_settings_backup(&addons_root, "").expect("create backup");
        assert!(summary.contains("2 files"));
        let snapshots = settings_backup_snapshots(&addons_root).expect("list backups");
        let manual = snapshots
            .iter()
            .find(|snapshot| snapshot.kind == BACKUP_KIND_MANUAL)
            .expect("manual backup exists");
        let manual_name = manual.entry.name.to_string();
        create_settings_backup(&addons_root, "Raid UI Snapshot").expect("create labeled backup");
        assert!(settings_backups_dir(&addons_root)
            .join("Raid UI Snapshot")
            .is_dir());
        assert!(create_settings_backup(&addons_root, "char-reserved").is_err());

        fs::write(sv_dir.join("Live.lua"), "changed").expect("modify live file");
        let restore_summary =
            restore_settings_backup(&addons_root, &manual_name).expect("restore backup");
        assert!(restore_summary.contains("2 files"));
        assert_eq!(
            fs::read_to_string(sv_dir.join("Live.lua")).expect("read restored file"),
            "current"
        );
        assert!(settings_backup_snapshots(&addons_root)
            .expect("list backups after restore")
            .iter()
            .any(|snapshot| snapshot.kind == BACKUP_KIND_SAFETY));

        delete_settings_backup(&addons_root, &manual_name).expect("delete backup");
        assert!(!settings_backups_dir(&addons_root)
            .join(&manual_name)
            .exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_character_roster_merges_addon_settings_and_saved_variables() {
        let root = test_temp_dir("native-character-roster");
        let addons_root = root.join("AddOns");
        let sv_dir = root.join("SavedVariables");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&sv_dir).expect("create saved variables root");
        fs::write(
            root.join("AddOnSettings.txt"),
            "#Version 101046\n#NA Megaserver-Main Character\n",
        )
        .expect("write AddOnSettings");
        fs::write(
            sv_dir.join("Roster.lua"),
            concat!(
                "Roster =\n{\n",
                "\t[\"Default\"] =\n\t{\n",
                "\t\t[\"EU Megaserver\"] =\n\t\t{\n",
                "\t\t\t[\"@me\"] =\n\t\t\t{\n",
                "\t\t\t\t[\"Recovered Alt\"] = { [\"enabled\"] = true },\n",
                "\t\t\t},\n",
                "\t\t},\n",
                "\t},\n}\n"
            ),
        )
        .expect("write roster saved variable");

        let roster = native_character_roster(&addons_root).expect("load roster");
        assert_eq!(roster.skipped_files, 0);
        assert!(roster.characters.iter().any(|character| {
            character.name == "Main Character"
                && character.server == "NA Megaserver"
                && !character.recovered
        }));
        assert!(roster.characters.iter().any(|character| {
            character.name == "Recovered Alt"
                && character.server == "EU Megaserver"
                && character.recovered
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_character_backup_writes_v2_subtree_backup() {
        let root = test_temp_dir("native-character-backup");
        let addons_root = root.join("AddOns");
        let sv_dir = root.join("SavedVariables");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&sv_dir).expect("create saved variables root");
        fs::write(
            sv_dir.join("TestAddon.lua"),
            concat!(
                "TestAddon =\n{\n",
                "\t[\"Default\"] =\n\t{\n",
                "\t\t[\"NA Megaserver\"] =\n\t\t{\n",
                "\t\t\t[\"@me\"] =\n\t\t\t{\n",
                "\t\t\t\t[\"Bob\"] = { [\"hp\"] = 1, [\"loc\"] = \"NA\" },\n",
                "\t\t\t},\n",
                "\t\t},\n",
                "\t\t[\"EU Megaserver\"] =\n\t\t{\n",
                "\t\t\t[\"@me\"] =\n\t\t\t{\n",
                "\t\t\t\t[\"Bob\"] = { [\"hp\"] = 2, [\"loc\"] = \"EU\" },\n",
                "\t\t\t},\n",
                "\t\t},\n",
                "\t},\n}\n"
            ),
        )
        .expect("write saved variable");

        let copied = create_character_settings_backup(&addons_root, "Bob", "NA Megaserver", "")
            .expect("create character backup");
        assert_eq!(copied, 1);

        let backup_dir = settings_backups_dir(&addons_root).join("char-Bob-NA-backup");
        assert!(backup_dir.is_dir());
        assert!(fs::read(backup_dir.join(CHAR_BACKUP_MARKER))
            .expect("read marker")
            .starts_with(CHAR_BACKUP_MARKER_V2_PREFIX));
        let meta = serde_json::from_slice::<CharBackupMeta>(
            &fs::read(backup_dir.join(CHAR_BACKUP_META)).expect("read metadata"),
        )
        .expect("parse metadata");
        assert_eq!(meta.character, "Bob");
        assert_eq!(meta.server, "NA Megaserver");
        let backup = fs::read_to_string(backup_dir.join("TestAddon.lua")).expect("read backup");
        assert!(backup.contains("[\"hp\"] = 1"));
        assert!(!backup.contains("[\"hp\"] = 2"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn settings_backup_restore_refuses_corrupt_character_metadata() {
        let root = test_temp_dir("settings-backup-character-refuse");
        let addons_root = root.join("AddOns");
        let char_backup = root.join("kalpa-backups").join("char-Alt-NA-backup");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&char_backup).expect("create character backup");
        fs::write(
            char_backup.join(CHAR_BACKUP_MARKER),
            b"kalpa character backup v2\n",
        )
        .expect("write character marker");
        fs::write(char_backup.join("Character.lua"), "character").expect("write character file");

        let error = restore_settings_backup(&addons_root, "char-Alt-NA-backup")
            .expect_err("corrupt character restore should be refused");
        assert!(error.contains("missing its metadata"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn settings_backup_restore_merges_character_backup_without_touching_twins() {
        let root = test_temp_dir("settings-backup-character-merge");
        let addons_root = root.join("AddOns");
        let sv_dir = root.join("SavedVariables");
        let backup_dir = root.join("kalpa-backups").join("char-Bob-backup");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&sv_dir).expect("create saved variables root");
        fs::create_dir_all(&backup_dir).expect("create character backup");

        let live = concat!(
            "TestAddon =\n{\n",
            "\t[\"Default\"] =\n\t{\n",
            "\t\t[\"NA Megaserver\"] =\n\t\t{\n",
            "\t\t\t[\"@me\"] =\n\t\t\t{\n",
            "\t\t\t\t[\"Bob\"] = { [\"hp\"] = 1, [\"loc\"] = \"NA\" },\n",
            "\t\t\t},\n",
            "\t\t},\n",
            "\t\t[\"EU Megaserver\"] =\n\t\t{\n",
            "\t\t\t[\"@me\"] =\n\t\t\t{\n",
            "\t\t\t\t[\"Bob\"] = { [\"hp\"] = 2, [\"loc\"] = \"EU\" },\n",
            "\t\t\t},\n",
            "\t\t},\n",
            "\t\t[\"@me\"] =\n\t\t{\n",
            "\t\t\t[\"$AccountWide\"] = { [\"gold\"] = 9 },\n",
            "\t\t},\n",
            "\t},\n}\n"
        );
        let backup_source = live.replace("[\"hp\"] = 1", "[\"hp\"] = 100");
        let blocks = char_backup::extract_character_blocks(
            backup_source.as_bytes(),
            b"Bob",
            Some("NA Megaserver"),
        );
        let backup_file = char_backup::build_backup_file(&blocks).expect("build backup file");

        fs::write(sv_dir.join("TestAddon.lua"), live).expect("write live saved variable");
        fs::write(backup_dir.join("TestAddon.lua"), backup_file).expect("write backup file");
        fs::write(
            backup_dir.join(CHAR_BACKUP_MARKER),
            b"kalpa character backup v2\n",
        )
        .expect("write marker");
        fs::write(
            backup_dir.join(CHAR_BACKUP_META),
            serde_json::to_vec(&CharBackupMeta {
                version: CHAR_BACKUP_VERSION,
                character: "Bob".to_string(),
                server: "NA Megaserver".to_string(),
            })
            .expect("serialize meta"),
        )
        .expect("write meta");

        let summary =
            restore_settings_backup(&addons_root, "char-Bob-backup").expect("restore character");
        assert!(summary.contains("1 character file"));
        let restored =
            fs::read_to_string(sv_dir.join("TestAddon.lua")).expect("read restored saved variable");
        assert!(restored.contains("[\"Bob\"] = { [\"hp\"] = 100, [\"loc\"] = \"NA\" }"));
        assert!(restored.contains("[\"Bob\"] = { [\"hp\"] = 2, [\"loc\"] = \"EU\" }"));
        assert!(restored.contains("[\"$AccountWide\"] = { [\"gold\"] = 9 }"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn settings_backup_listing_recovers_orphaned_character_backup() {
        let root = test_temp_dir("settings-backup-recover-char");
        let addons_root = root.join("AddOns");
        let backups_root = root.join("kalpa-backups");
        let old = backups_root.join(".old-char-Bob-backup-42");
        let staging = backups_root.join(".tmp-char-Bob-backup-42");
        fs::create_dir_all(&addons_root).expect("create addon root");
        fs::create_dir_all(&old).expect("create tombstone backup");
        fs::create_dir_all(&staging).expect("create staging backup");
        fs::write(old.join(CHAR_BACKUP_MARKER), b"kalpa character backup\n").expect("write marker");
        fs::write(old.join("TestAddon.lua"), "backup").expect("write backup file");

        let snapshots = settings_backup_snapshots(&addons_root).expect("list backups");
        assert!(backups_root.join("char-Bob-backup").is_dir());
        assert!(!old.exists());
        assert!(!staging.exists());
        assert!(snapshots
            .iter()
            .any(|snapshot| snapshot.entry.name.as_str() == "char-Bob-backup"));

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
    fn addon_date_sort_key_accepts_metadata_date_formats() {
        assert_eq!(date_sort_key("2026-07-02T18:45:00Z"), 20260702);
        assert_eq!(date_sort_key("Jul 2, 2026"), 20260702);
        assert_eq!(date_sort_key("July 2 2026"), 20260702);
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

    #[test]
    fn metadata_hydration_restores_esoui_dates_and_tags() {
        let mut entry = addon_entry(
            "Tracked Addon",
            "TrackedAddon",
            "",
            "Author",
            "",
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
        );
        let meta = metadata::AddonMetadata {
            esoui_id: 42,
            installed_version: "v2.0.0".to_string(),
            download_url: "https://example.test/addon.zip".to_string(),
            installed_at: "2026-07-02T18:45:00Z".to_string(),
            tags: vec!["favorite".to_string(), "pvp-build".to_string()],
            esoui_last_update: 1_782_864_000_000,
        };

        hydrate_addon_from_metadata(&mut entry, &meta);

        assert_eq!(entry.esoui_id.as_str(), "42");
        assert_eq!(entry.version.as_str(), "v2.0.0");
        assert!(entry.meta.as_str().contains("v2.0.0"));
        assert_eq!(entry.last_updated.as_str(), "Jul 1, 2026");
        assert_eq!(entry.installed_at.as_str(), "Jul 2, 2026");
        assert!(entry.favorite);
        assert!(tag_model_has_active(&entry.tags, "favorite"));
        assert!(tag_model_has_active(&entry.tags, "pvp-build"));
    }

    #[test]
    fn addon_update_check_results_replace_stale_badges() {
        let current = addon_entry(
            "Current",
            "CurrentAddon",
            "1",
            "Author",
            "1.0",
            "101049",
            "Addon",
            "",
            "",
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
        );
        let mut stale = addon_entry(
            "Stale",
            "StaleAddon",
            "2",
            "Author",
            "1.0",
            "101049",
            "Addon",
            "",
            "",
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
        );
        stale.last_updated = "Jan 1, 2025".into();
        let models = test_addon_models(vec![current, stale]);
        let updates = vec![
            AddonUpdateCheckEntry {
                folder_name: "CurrentAddon".into(),
                remote_version: "1.0".into(),
                has_update: false,
                last_updated: "Jul 1, 2026".into(),
            },
            AddonUpdateCheckEntry {
                folder_name: "StaleAddon".into(),
                remote_version: "2.0".into(),
                has_update: true,
                last_updated: "Jul 2, 2026".into(),
            },
        ];

        let available = apply_addon_update_check_results(&models, &updates);
        let addons = models.all.borrow();

        assert_eq!(available, 1);
        assert_eq!(addons[0].badge.as_str(), "");
        assert_eq!(addons[0].state, 0);
        assert_eq!(addons[0].last_updated.as_str(), "Jul 1, 2026");
        assert_eq!(addons[1].state, 1);
        assert_eq!(addons[1].badge.as_str(), "Update");
        assert_eq!(addons[1].last_updated.as_str(), "Jul 2, 2026");
    }

    #[test]
    fn native_conflict_report_flags_user_and_upstream_edits() {
        let root = test_temp_dir("native-conflict-report");
        let folder = "ConflictAddon";
        let addon_dir = root.join(folder);
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::write(
            addon_dir.join(format!("{folder}.txt")),
            "## Title: Conflict Addon\n## Version: 1.0\n",
        )
        .expect("write manifest");
        fs::write(addon_dir.join("main.lua"), "d('old upstream')\n").expect("write baseline");
        file_hashes::record_hashes_for_folders(root, &[folder.to_string()], 42, "1.0")
            .expect("record baseline");
        fs::write(addon_dir.join("main.lua"), "d('user edit')\n").expect("write user edit");

        let zip_path = root.join("update.zip");
        write_test_addon_zip_with_lua(&zip_path, folder, "2.0", "d('new upstream')\n");

        let (report, _) =
            build_native_conflict_report(root, folder, &zip_path).expect("build report");

        assert_eq!(report.conflicts, vec!["main.lua".to_string()]);
        assert!(report.auto_kept_files.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_conflict_report_auto_keeps_user_only_edits() {
        let root = test_temp_dir("native-auto-keep-report");
        let folder = "AutoKeepAddon";
        let addon_dir = root.join(folder);
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::write(
            addon_dir.join(format!("{folder}.txt")),
            "## Title: Auto Keep Addon\n## Version: 1.0\n",
        )
        .expect("write manifest");
        fs::write(addon_dir.join("main.lua"), "d('old upstream')\n").expect("write baseline");
        file_hashes::record_hashes_for_folders(root, &[folder.to_string()], 42, "1.0")
            .expect("record baseline");
        fs::write(addon_dir.join("main.lua"), "d('user edit')\n").expect("write user edit");

        let zip_path = root.join("update.zip");
        write_test_addon_zip_with_lua(&zip_path, folder, "2.0", "d('old upstream')\n");

        let (report, _) =
            build_native_conflict_report(root, folder, &zip_path).expect("build report");

        assert!(report.conflicts.is_empty());
        assert_eq!(report.auto_kept_files, vec!["main.lua".to_string()]);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_conflict_policy_decides_kept_files() {
        let report = NativeConflictReport {
            safe_file_count: 0,
            auto_kept_files: vec!["unchanged-user-edit.lua".to_string()],
            conflicts: vec!["changed-user-edit.lua".to_string()],
        };

        assert!(native_kept_files_for_policy(&report, 0).is_none());
        assert_eq!(
            native_kept_files_for_policy(&report, 1).expect("keep mine resolves"),
            vec![
                "changed-user-edit.lua".to_string(),
                "unchanged-user-edit.lua".to_string()
            ]
        );
        assert_eq!(
            native_kept_files_for_policy(&report, 2).expect("take update resolves"),
            vec!["unchanged-user-edit.lua".to_string()]
        );
    }

    #[test]
    fn native_pending_conflict_apply_keeps_selected_user_file() {
        let root = test_temp_dir("native-pending-conflict-keep");
        let folder = "KeepConflictAddon";
        seed_conflicted_addon(root, folder);

        let zip_path = root.join("update.zip");
        write_test_addon_zip_with_lua(&zip_path, folder, "2.0", "d('new upstream')\n");
        let pending = pending_conflict_for_test(root, folder, &zip_path, 1);

        let installed = apply_native_pending_conflict_files_blocking(root, &pending)
            .expect("apply pending conflict");

        assert!(installed.contains(&folder.to_string()));
        assert_eq!(
            fs::read_to_string(root.join(folder).join("main.lua")).expect("read kept file"),
            "d('user edit')\n"
        );
        assert_eq!(
            file_hashes::load_hash_manifest(root, folder)
                .expect("hash manifest")
                .installed_version,
            "2.0"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_pending_conflict_apply_takes_update_and_backs_up_user_file() {
        let root = test_temp_dir("native-pending-conflict-take");
        let folder = "TakeConflictAddon";
        seed_conflicted_addon(root, folder);

        let zip_path = root.join("update.zip");
        write_test_addon_zip_with_lua(&zip_path, folder, "2.0", "d('new upstream')\n");
        let pending = pending_conflict_for_test(root, folder, &zip_path, 2);

        apply_native_pending_conflict_files_blocking(root, &pending)
            .expect("apply pending conflict");

        assert_eq!(
            fs::read_to_string(root.join(folder).join("main.lua")).expect("read updated file"),
            "d('new upstream')\n"
        );
        let backups = edit_backups::list_backups(root, folder);
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].files, vec!["main.lua".to_string()]);

        let _ = fs::remove_dir_all(root);
    }

    fn seed_conflicted_addon(root: &Path, folder: &str) {
        let addon_dir = root.join(folder);
        fs::create_dir_all(&addon_dir).expect("create addon dir");
        fs::write(
            addon_dir.join(format!("{folder}.txt")),
            "## Title: Conflict Addon\n## Version: 1.0\n",
        )
        .expect("write manifest");
        fs::write(addon_dir.join("main.lua"), "d('old upstream')\n").expect("write baseline");
        file_hashes::record_hashes_for_folders(root, &[folder.to_string()], 42, "1.0")
            .expect("record baseline");
        fs::write(addon_dir.join("main.lua"), "d('user edit')\n").expect("write user edit");
    }

    fn pending_conflict_for_test(
        root: &Path,
        folder: &str,
        zip_path: &Path,
        decision: i32,
    ) -> NativePendingConflict {
        let (report, zip_hashes) =
            build_native_conflict_report(root, folder, zip_path).expect("build report");
        assert_eq!(report.conflicts, vec!["main.lua".to_string()]);

        NativePendingConflict {
            folder_name: folder.to_string(),
            esoui_id: 42,
            update_version: "2.0".to_string(),
            title: folder.to_string(),
            download_url: "https://example.invalid/update.zip".to_string(),
            safe_file_count: report.safe_file_count,
            auto_kept_files: report.auto_kept_files,
            conflicts: report.conflicts,
            decisions: HashMap::from([("main.lua".to_string(), decision)]),
            zip_path: zip_path.to_path_buf(),
            zip_hashes,
        }
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

    fn test_addon_models(addons: Vec<AddonEntry>) -> AddonModels {
        AddonModels {
            all: Rc::new(RefCell::new(addons.clone())),
            visible: Rc::new(VecModel::from(addons)),
            view_key: Rc::new(RefCell::new(None)),
        }
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

    fn write_test_addon_zip(path: &Path, folder: &str, version: &str) {
        write_test_addon_zip_with_lua(path, folder, version, "d('installed')\n");
    }

    fn write_test_addon_zip_with_lua(path: &Path, folder: &str, version: &str, lua: &str) {
        let file = fs::File::create(path).expect("create test zip");
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive
            .start_file(format!("{folder}/{folder}.txt"), options)
            .expect("start manifest file");
        archive
            .write_all(
                format!(
                    "## Title: {folder}\n## Author: Kalpa\n## Version: {version}\n## APIVersion: 101048\n"
                )
                .as_bytes(),
            )
            .expect("write manifest");
        archive
            .start_file(format!("{folder}/main.lua"), options)
            .expect("start lua file");
        archive.write_all(lua.as_bytes()).expect("write lua file");
        archive.finish().expect("finish test zip");
    }

    fn sample_native_hub_pack(addons: serde_json::Value) -> NativeHubPack {
        NativeHubPack {
            id: "pack-1".to_string(),
            author_id: "author-1".to_string(),
            author_name: "Spike'jo".to_string(),
            is_anonymous: false,
            title: "Trial Essentials".to_string(),
            description: "Core addons for trial nights".to_string(),
            pack_type: "addon".to_string(),
            addons,
            vote_count: 7,
            install_count: 3,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
            tags: vec!["trial".to_string(), "healer".to_string()],
            user_voted: None,
            status: Some("published".to_string()),
        }
    }

    fn sample_pack_hub_addon_row(
        title: &str,
        esoui_id: &str,
        installed: bool,
    ) -> PackHubAddonEntry {
        PackHubAddonEntry {
            title: title.into(),
            esoui_id: esoui_id.into(),
            required: true,
            installed,
            selected: true,
            note: "".into(),
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
    fn discover_browse_state_tracks_pages_and_rank_offsets() {
        let mut state = DiscoverBrowseState {
            popular_has_more: true,
            category_has_more: true,
            category_page: 2,
            categories: vec![esoui::EsouiCategory {
                id: 1,
                name: "Combat".to_string(),
                depth: 0,
            }],
            ..Default::default()
        };

        state.popular_page = 2;
        let next_popular = state.next_popular_page_snapshot();
        assert_eq!(next_popular.popular_page, 3);

        state.select_next_category_sort();
        assert_eq!(state.category_page, 0);
        assert!(!state.category_has_more);

        let entries = discover_entries_from_search_results_with_offset(
            vec![esoui::EsouiSearchResult {
                id: 42,
                title: "Combat Metrics".to_string(),
                author: "Solinur".to_string(),
                category: "Combat".to_string(),
                downloads: "5.2M".to_string(),
                updated: "04/01/26".to_string(),
            }],
            &BTreeSet::new(),
            25,
        );
        assert_eq!(entries[0].rank, 26);
    }

    #[test]
    fn pack_hub_entry_maps_worker_json_string_shape() {
        let pack = sample_native_hub_pack(serde_json::Value::String(
            r#"[{"esouiId":4061,"name":"Ability Icons Framework","required":true},{"esouiId":1161,"name":"Addon Selector","required":false,"note":"Optional profile helper"}]"#
                .to_string(),
        ));

        let entry = pack_hub_entry_from_hub(pack);

        assert_eq!(entry.title.as_str(), "Trial Essentials");
        assert_eq!(entry.addon_count.as_str(), "2 addons");
        assert_eq!(entry.vote_count.as_str(), "7");
        assert_eq!(entry.tag.as_str(), "trial");
        assert_eq!(entry.author.as_str(), "Spike'jo");
        assert_eq!(entry.pack_type_label.as_str(), "Addon Pack");
        assert_eq!(entry.updated_label.as_str(), "Updated Jan 2, 2026");
        assert_eq!(entry.monogram.as_str(), "TE");
        assert_eq!(entry.author_initial.as_str(), "S");
        assert_eq!(entry.type_kind, 0);
        assert!(entry.trial);
    }

    #[test]
    fn pack_hub_entry_maps_array_shape_and_anonymous_author() {
        let mut pack = sample_native_hub_pack(serde_json::json!([
            { "esouiId": 4061, "name": "Ability Icons Framework" }
        ]));
        pack.is_anonymous = true;
        pack.author_name = "Hidden Author".to_string();
        pack.pack_type = "build".to_string();
        pack.tags.clear();
        pack.vote_count = -5;

        let entry = pack_hub_entry_from_hub(pack);

        assert_eq!(entry.addon_count.as_str(), "1 addon");
        assert_eq!(entry.vote_count.as_str(), "0");
        assert_eq!(entry.author.as_str(), "Anonymous");
        assert_eq!(entry.author_initial.as_str(), "A");
        assert_eq!(entry.pack_type_label.as_str(), "Build Pack");
        assert_eq!(entry.tag.as_str(), "build");
        assert_eq!(entry.type_kind, 1);
    }

    #[test]
    fn pack_hub_identity_derives_monograms_and_distinct_type_color() {
        assert_eq!(pack_monogram("Trial Essentials"), "TE");
        assert_eq!(pack_monogram("CombatMetrics"), "CO");
        assert_eq!(pack_monogram("!!!"), "?");
        assert_eq!(author_initial("@code65536"), "C");

        assert_eq!(pack_type_kind("addon-pack"), 0);
        assert_eq!(pack_type_kind("build-pack"), 1);
        assert_eq!(pack_type_kind("roster-pack"), 2);

        let addon_identity = pack_identity_kind("pack-1", "Trial Essentials", "addon-pack");
        assert!((0..7).contains(&addon_identity));
        assert_ne!(addon_identity, 0);
    }

    #[test]
    fn pack_hub_detail_maps_addons_and_install_label() {
        let pack = sample_native_hub_pack(serde_json::Value::String(
            r#"[{"esouiId":4061,"name":"Ability Icons Framework","required":true},{"esouiId":1161,"name":"Addon Selector","required":false,"note":"Optional profile helper"}]"#
                .to_string(),
        ));
        let installed_ids = BTreeSet::from(["4061".to_string()]);

        let detail = pack_hub_detail_from_hub(pack, &installed_ids);

        assert_eq!(detail.entry.addon_count.as_str(), "2 addons");
        assert_eq!(detail.addons.len(), 2);
        assert_eq!(detail.addons[0].title.as_str(), "Ability Icons Framework");
        assert_eq!(detail.addons[0].esoui_id.as_str(), "#4061");
        assert!(detail.addons[0].required);
        assert!(detail.addons[0].installed);
        assert_eq!(detail.addons[1].note.as_str(), "Optional profile helper");
        assert!(!detail.addons[1].required);
        assert!(!detail.addons[1].installed);
        assert!(!detail.addons[1].selected);
        assert_eq!(
            pack_hub_install_label(&detail.addons),
            "Select Addons to Install"
        );

        let mut selected_addons = detail.addons.clone();
        selected_addons[1].selected = true;
        assert_eq!(
            pack_hub_install_label(&selected_addons),
            "Install 1 New Addon"
        );
    }

    #[test]
    fn pack_hub_share_code_normalizes_and_validates() {
        assert_eq!(normalize_share_code(" hk7m3p "), "HK7M3P");
        assert_eq!(validate_share_code("hk7m3p").unwrap(), "HK7M3P");
        assert!(validate_share_code("HK7M3").is_err());
        assert!(validate_share_code("HK7M1P").is_err());
        assert!(validate_share_code("HK7MOP").is_err());
    }

    #[test]
    fn pack_hub_shared_pack_maps_required_only_import_label() {
        let shared = NativeSharedPackResponse {
            pack: NativeSharedPackBody {
                title: "Shared Trial Pack".to_string(),
                description: "Required addon plus optional helper".to_string(),
                pack_type: "addon-pack".to_string(),
                tags: vec!["trial".to_string()],
                addons: vec![
                    NativePackAddonEntry {
                        esoui_id: 4061,
                        name: "Ability Icons Framework".to_string(),
                        required: true,
                        note: None,
                    },
                    NativePackAddonEntry {
                        esoui_id: 1161,
                        name: "Addon Selector".to_string(),
                        required: false,
                        note: Some("Optional profile helper".to_string()),
                    },
                ],
            },
            shared_by: "Spike'jo".to_string(),
            shared_at: "2026-01-03T00:00:00Z".to_string(),
            _expires_at: "2026-01-10T00:00:00Z".to_string(),
        };

        let installed_required = BTreeSet::from(["4061".to_string()]);
        let detail =
            pack_hub_detail_from_shared_pack("HK7M3P", shared.clone(), &installed_required);
        assert_eq!(detail.entry.title.as_str(), "Shared Trial Pack");
        assert_eq!(detail.entry.author.as_str(), "Spike'jo");
        assert_eq!(detail.entry.updated_label.as_str(), "Shared Jan 3, 2026");
        assert!(detail.addons[0].selected);
        assert!(detail.addons[0].installed);
        assert!(!detail.addons[1].required);
        assert!(!detail.addons[1].selected);
        assert_eq!(
            pack_hub_import_install_label(&detail.addons, false),
            "All Addons Installed"
        );
        assert_eq!(
            pack_hub_import_install_label(&detail.addons, true),
            "Apply Settings"
        );

        let missing_required = pack_hub_detail_from_shared_pack("HK7M3P", shared, &BTreeSet::new());
        assert_eq!(
            pack_hub_import_install_label(&missing_required.addons, true),
            "Install 1 New Addon"
        );
    }

    #[test]
    fn pack_hub_esopack_file_import_parses_v2_settings() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("trial.esopack");
        let json = serde_json::json!({
            "format": "esopack",
            "version": 2,
            "pack": {
                "title": "File Trial Pack",
                "description": "Imported from disk",
                "packType": "addon-pack",
                "tags": ["trial"],
                "addons": [
                    { "esouiId": 4061, "name": "Ability Icons Framework", "required": true },
                    { "esouiId": 1161, "name": "Addon Selector", "required": false }
                ]
            },
            "sharedAt": "2026-01-04T00:00:00Z",
            "sharedBy": "Guildmate",
            "settings": {
                "AddonSelector": {
                    "encoding": "lua-text",
                    "lua": "AddonSelector_SavedVariables = { }\n",
                    "originalBytes": 32,
                    "scrubbedBytes": 32,
                    "finalBytes": 32
                }
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).expect("write esopack");

        let imported = import_esopack_file_blocking(&path, &BTreeSet::new()).expect("import file");

        assert_eq!(imported.detail.entry.title.as_str(), "File Trial Pack");
        assert_eq!(imported.detail.entry.author.as_str(), "Guildmate");
        assert_eq!(
            imported.detail.entry.updated_label.as_str(),
            "Shared Jan 4, 2026"
        );
        assert_eq!(imported.detail.addons.len(), 2);
        assert!(imported.settings.contains_key("AddonSelector"));
    }

    #[test]
    fn pack_hub_esopack_settings_apply_writes_saved_variables() {
        let temp = tempfile::tempdir().expect("tempdir");
        let addons_root = temp.path().join("live").join("AddOns");
        fs::create_dir_all(&addons_root).expect("addons root");
        let mut settings = HashMap::new();
        settings.insert(
            "AddonSelector".to_string(),
            NativeAddonSettings {
                encoding: "lua-text".to_string(),
                lua: "AddonSelector_SavedVariables = { }\n".to_string(),
                _original_bytes: 32,
                _scrubbed_bytes: 32,
                _final_bytes: 32,
            },
        );

        let result = apply_imported_pack_settings_blocking(&addons_root, settings);

        assert_eq!(result.applied, vec!["AddonSelector".to_string()]);
        assert!(result.errors.is_empty());
        let written = settings_saved_variables_dir(&addons_root).join("AddonSelector.lua");
        assert!(written.is_file());
        assert_eq!(
            fs::read_to_string(written).expect("written settings"),
            "AddonSelector_SavedVariables = { }\n"
        );
    }

    #[test]
    fn pack_hub_install_helpers_parse_ids_and_summarize_results() {
        let installed = sample_pack_hub_addon_row("Addon Selector", "#1161", true);
        let missing = sample_pack_hub_addon_row("Combat Metrics", "1360", false);

        assert_eq!(pack_hub_row_esoui_id(&installed).unwrap(), 1161);
        assert_eq!(pack_hub_row_esoui_id(&missing).unwrap(), 1360);
        assert_eq!(
            pack_hub_install_label(&[installed.clone(), missing.clone()]),
            "Install 1 New Addon"
        );
        assert_eq!(
            pack_hub_install_label(&[installed.clone()]),
            "All Addons Installed"
        );

        let summary = pack_hub_install_summary(&NativePackInstallResult {
            rows: vec![installed, missing],
            installed: 1,
            failed: 1,
            folders: 2,
            errors: vec!["Combat Metrics: download failed".to_string()],
        });

        assert!(summary.contains("Installed 1 Pack Hub addon, 1 failed."));
        assert!(summary.contains("Combat Metrics: download failed"));
    }

    #[test]
    fn pack_hub_create_rows_filter_and_track_selection() {
        let mut state = PackHubCreateState::default();
        let mut addon = addon_entry(
            "Combat Metrics",
            "CombatMetrics",
            "1360",
            "Solinur",
            "1.0",
            "101048",
            "Addon",
            "3/1/2026",
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
        );

        let row = pack_hub_create_entry_from_addon(&addon, &state).expect("create row");
        assert_eq!(row.esoui_id.as_str(), "#1360");
        assert!(!row.selected);
        assert!(pack_hub_create_filter_matches(&addon, "combat"));
        assert!(pack_hub_create_filter_matches(&addon, "1360"));
        assert!(!pack_hub_create_filter_matches(&addon, "writ"));

        toggle_pack_hub_create_row(&mut state, row);
        let selected = pack_hub_create_entry_from_addon(&addon, &state).expect("selected row");
        assert!(selected.selected);
        assert!(selected.required);

        toggle_pack_hub_create_required(&mut state, selected);
        assert!(!state.selected[0].required);

        addon.is_library = true;
        assert!(pack_hub_create_entry_from_addon(&addon, &state).is_none());
    }

    #[test]
    fn pack_hub_create_export_round_trips_as_esopack() {
        let root = test_temp_dir("pack-hub-create-export");
        let selected = vec![
            PackHubCreateAddonEntry {
                title: "Combat Metrics".into(),
                meta: "by Solinur - v1.0 - #1360".into(),
                esoui_id: "#1360".into(),
                selected: true,
                required: true,
            },
            PackHubCreateAddonEntry {
                title: "Optional Helper".into(),
                meta: "#2468".into(),
                esoui_id: "#2468".into(),
                selected: true,
                required: false,
            },
        ];

        let path = export_pack_hub_create_file(
            "Trial Essentials",
            "Core raid addons",
            0,
            &selected,
            Some(root),
        )
        .expect("export pack");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("Trial-Essentials.esopack")
        );
        let imported = import_esopack_file_blocking(&path, &BTreeSet::new()).expect("import pack");
        assert_eq!(imported.detail.entry.title.as_str(), "Trial Essentials");
        assert_eq!(imported.detail.addons.len(), 2);
        assert!(imported.detail.addons[0].required);
        assert!(!imported.detail.addons[1].required);
        assert!(imported.settings.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pack_hub_detail_export_round_trips_as_esopack() {
        let root = test_temp_dir("pack-hub-detail-export");
        let entry = PackHubEntry {
            id: "pack-1".into(),
            title: "Trial Essentials".into(),
            description: "Core raid addons".into(),
            tag: "trial".into(),
            addon_count: "2 addons".into(),
            vote_count: "7".into(),
            author: "Spike'jo".into(),
            pack_type_label: "Addon Pack".into(),
            updated_label: "Updated Jan 2, 2026".into(),
            monogram: "TE".into(),
            author_initial: "S".into(),
            identity_kind: 1,
            type_kind: 0,
            trial: true,
        };
        let addons = vec![
            PackHubAddonEntry {
                title: "Combat Metrics".into(),
                esoui_id: "#1360".into(),
                required: true,
                installed: false,
                selected: true,
                note: "".into(),
            },
            PackHubAddonEntry {
                title: "Optional Helper".into(),
                esoui_id: "#2468".into(),
                required: false,
                installed: false,
                selected: false,
                note: "Optional profile helper".into(),
            },
        ];

        let path =
            export_pack_hub_detail_file(&entry, &addons, Some(root)).expect("export detail pack");
        let imported = import_esopack_file_blocking(&path, &BTreeSet::new()).expect("import pack");

        assert_eq!(imported.detail.entry.title.as_str(), "Trial Essentials");
        assert_eq!(imported.detail.entry.author.as_str(), "Spike'jo");
        assert_eq!(imported.detail.addons.len(), 2);
        assert_eq!(imported.detail.addons[0].esoui_id.as_str(), "#1360");
        assert!(imported.detail.addons[0].required);
        assert!(!imported.detail.addons[1].required);
        assert_eq!(
            imported.detail.addons[1].note.as_str(),
            "Optional profile helper"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pack_hub_browse_state_cycles_filters_and_pages() {
        let mut state = PackHubBrowseState::default();
        assert_eq!(pack_hub_type_filter_label(state.type_filter), "All");
        assert_eq!(pack_hub_type_filter_key(state.type_filter), None);
        assert_eq!(pack_hub_sort_label(state.sort), "Votes");
        assert_eq!(pack_hub_sort_key(state.sort), "votes");

        state.next_type_filter();
        assert_eq!(pack_hub_type_filter_label(state.type_filter), "Addon Pack");
        assert_eq!(
            pack_hub_type_filter_key(state.type_filter),
            Some("addon-pack")
        );
        state.next_type_filter();
        assert_eq!(
            pack_hub_type_filter_key(state.type_filter),
            Some("build-pack")
        );
        state.next_type_filter();
        assert_eq!(
            pack_hub_type_filter_key(state.type_filter),
            Some("roster-pack")
        );
        state.next_type_filter();
        assert_eq!(pack_hub_type_filter_key(state.type_filter), None);

        state.next_sort();
        assert_eq!(pack_hub_sort_label(state.sort), "Newest");
        assert_eq!(pack_hub_sort_key(state.sort), "newest");
        state.next_sort();
        assert_eq!(pack_hub_sort_label(state.sort), "Updated");
        assert_eq!(pack_hub_sort_key(state.sort), "updated");

        let page_two = state.next_page_snapshot();
        assert_eq!(state.page, 1);
        assert_eq!(page_two.page, 2);
    }

    #[test]
    fn discover_entries_reflect_installed_ids() {
        let installed = BTreeSet::from(["1346".to_string(), "3317".to_string()]);
        let entries = popular_discover_entries(&installed);

        let lazy_writ = entries
            .iter()
            .find(|entry| entry.esoui_id.as_str() == "1346")
            .expect("popular list includes Lazy Writ Crafter");
        assert!(lazy_writ.installed);

        let character_knowledge = entries
            .iter()
            .find(|entry| entry.esoui_id.as_str() == "3317")
            .expect("popular list includes Character Knowledge");
        assert!(character_knowledge.installed);

        let action_duration = entries
            .iter()
            .find(|entry| entry.esoui_id.as_str() == "2045")
            .expect("popular list includes Action Duration Reminder");
        assert!(!action_duration.installed);
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
    fn discover_search_result_conversion_preserves_row_metadata() {
        let installed = BTreeSet::from(["1360".to_string()]);
        let entry = discover_entry_from_search_result(
            esoui::EsouiSearchResult {
                id: 1360,
                title: "CombatMetrics".to_string(),
                author: "Solinur".to_string(),
                category: "Combat".to_string(),
                downloads: "5.2M".to_string(),
                updated: "3/3/2026".to_string(),
            },
            &installed,
            4,
        );

        assert_eq!(entry.esoui_id.as_str(), "1360");
        assert_eq!(entry.title.as_str(), "CombatMetrics");
        assert_eq!(entry.category.as_str(), "Combat");
        assert_eq!(entry.rank, 4);
        assert!(entry.installed);
    }

    #[test]
    fn discover_search_result_still_needs_first_click_detail_load() {
        let installed = BTreeSet::new();
        let search_entry = discover_entry_from_search_result(
            esoui::EsouiSearchResult {
                id: 1360,
                title: "CombatMetrics".to_string(),
                author: "Solinur".to_string(),
                category: "Combat".to_string(),
                downloads: "5.2M".to_string(),
                updated: "3/3/2026".to_string(),
            },
            &installed,
            1,
        );

        assert!(discover_entry_needs_detail(&search_entry));

        let detailed_entry = discover_entry(
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
            "abc123",
            "101048",
            "Full detail",
            1,
            &installed,
        );

        assert!(!discover_entry_needs_detail(&detailed_entry));
    }

    #[test]
    fn discover_category_default_prefers_combat() {
        let categories = vec![
            esoui::EsouiCategory {
                id: 10,
                name: "Libraries".to_string(),
                depth: 0,
            },
            esoui::EsouiCategory {
                id: 20,
                name: "Combat".to_string(),
                depth: 1,
            },
        ];

        assert_eq!(default_discover_category_id(&categories), Some(20));
    }

    #[test]
    fn discover_browse_state_cycles_category_and_sort_labels() {
        let mut state = DiscoverBrowseState::default();
        state.replace_categories(vec![
            esoui::EsouiCategory {
                id: 10,
                name: "Libraries".to_string(),
                depth: 0,
            },
            esoui::EsouiCategory {
                id: 20,
                name: "Combat".to_string(),
                depth: 0,
            },
            esoui::EsouiCategory {
                id: 30,
                name: "Tradeskills".to_string(),
                depth: 0,
            },
        ]);

        assert_eq!(discover_category_label(&state), "Combat");
        state.select_next_category();
        assert_eq!(discover_category_label(&state), "Tradeskills");

        assert_eq!(discover_category_sort_key(state.category_sort), "downloads");
        assert_eq!(
            discover_category_sort_label(state.category_sort),
            "Most Popular"
        );
        state.select_next_category_sort();
        assert_eq!(discover_category_sort_key(state.category_sort), "newest");
        assert_eq!(
            discover_category_sort_label(state.category_sort),
            "Recently Updated"
        );
        state.select_next_category_sort();
        assert_eq!(discover_category_sort_key(state.category_sort), "name");
        assert_eq!(discover_category_sort_label(state.category_sort), "Name");

        assert_eq!(discover_popular_sort_key(0), "downloads");
        assert_eq!(discover_popular_sort_key(1), "newest");
    }

    #[test]
    fn discover_detail_merge_keeps_selection_state() {
        let installed = BTreeSet::from(["1360".to_string()]);
        let entry = discover_entry_from_search_result(
            esoui::EsouiSearchResult {
                id: 1360,
                title: "CombatMetrics".to_string(),
                author: "Solinur".to_string(),
                category: "Combat".to_string(),
                downloads: "5.2M".to_string(),
                updated: "3/3/2026".to_string(),
            },
            &installed,
            2,
        );

        let merged = merge_discover_detail(
            entry,
            esoui::EsouiAddonDetail {
                id: 1360,
                title: "CombatMetrics".to_string(),
                version: "1.7.7".to_string(),
                author: "Solinur".to_string(),
                description: "Full detail".to_string(),
                compatibility: "101048".to_string(),
                md5: "abc123".to_string(),
                total_downloads: "5,200,000".to_string(),
                monthly_downloads: "213,000".to_string(),
                favorites: "8,800".to_string(),
                updated: "03/03/26".to_string(),
                created: "08/05/14".to_string(),
                screenshots: Vec::new(),
                download_url: "https://cdn.esoui.com/downloads/file1360.zip".to_string(),
            },
        );

        assert_eq!(merged.category.as_str(), "Combat");
        assert_eq!(merged.rank, 2);
        assert!(merged.installed);
        assert_eq!(merged.version.as_str(), "1.7.7");
        assert_eq!(merged.description.as_str(), "Full detail");
    }

    #[test]
    fn native_discover_install_extracts_hashes_and_records_metadata() {
        let root = test_temp_dir("discover-install");
        let addons_root = root.join("AddOns");
        fs::create_dir_all(&addons_root).expect("create AddOns root");
        let zip_path = root.join("CombatMetrics.zip");
        write_test_addon_zip(&zip_path, "CombatMetrics", "1.7.7");

        let detail = esoui::EsouiAddonDetail {
            id: 1360,
            title: "CombatMetrics".to_string(),
            version: "1.7.7".to_string(),
            author: "Solinur".to_string(),
            description: "Full detail".to_string(),
            compatibility: "101048".to_string(),
            md5: String::new(),
            total_downloads: "5,200,000".to_string(),
            monthly_downloads: "213,000".to_string(),
            favorites: "8,800".to_string(),
            updated: "03/03/26".to_string(),
            created: "08/05/14".to_string(),
            screenshots: Vec::new(),
            download_url: "https://cdn.esoui.com/downloads/file1360.zip".to_string(),
        };

        let installed =
            install_discover_download_blocking(&addons_root, &zip_path, &detail).unwrap();

        assert_eq!(installed, vec!["CombatMetrics".to_string()]);
        assert!(addons_root.join("CombatMetrics").join("main.lua").is_file());
        assert!(addons_root
            .join(".kalpa-hashes")
            .join("CombatMetrics.json")
            .is_file());

        let store = metadata::load_metadata(&addons_root);
        let meta = store
            .addons
            .get("CombatMetrics")
            .expect("metadata entry recorded");
        assert_eq!(meta.esoui_id, 1360);
        assert_eq!(meta.installed_version, "1.7.7");
        assert_eq!(meta.download_url, detail.download_url);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discover_uninstall_marks_matching_rows_not_installed() {
        let installed = BTreeSet::from(["1360".to_string(), "3520".to_string()]);
        let model: ModelRc<DiscoverEntry> = Rc::new(VecModel::from(vec![
            discover_entry(
                "1360",
                "CombatMetrics",
                "Solinur",
                "Combat",
                "1.7.7",
                "5.2M",
                "",
                "",
                "3/3/2026",
                "",
                "",
                "",
                "",
                0,
                &installed,
            ),
            discover_entry(
                "3520",
                "Wizards Wardrobe",
                "Dolgubon",
                "Utility",
                "1.0.0",
                "2.1M",
                "",
                "",
                "4/1/2026",
                "",
                "",
                "",
                "",
                0,
                &installed,
            ),
        ]))
        .into();

        mark_discover_uninstalled_model(&model, "1360");

        assert!(!model.row_data(0).expect("first row").installed);
        assert!(model.row_data(1).expect("second row").installed);
    }

    #[test]
    fn remove_addons_by_esoui_id_deletes_matching_folders_and_metadata() {
        static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = TEST_ENV_LOCK.lock().expect("test env lock");
        let previous = std::env::var_os("KALPA_ADDONS_PATH");
        let root = test_temp_dir("discover-remove-esoui");
        let addons_root = root.join("AddOns");
        fs::create_dir_all(addons_root.join("CombatMetrics")).expect("create addon folder");
        fs::create_dir_all(addons_root.join("CombatMetricsHelper"))
            .expect("create bundled addon folder");
        fs::create_dir_all(addons_root.join("OtherAddon")).expect("create other addon folder");
        std::env::set_var("KALPA_ADDONS_PATH", &addons_root);

        let mut store = metadata::MetadataStore::default();
        metadata::record_install_ext(
            &mut store,
            "CombatMetrics",
            1360,
            "1.7.7",
            "https://cdn.esoui.com/downloads/file1360.zip",
            0,
        );
        metadata::record_install_ext(
            &mut store,
            "CombatMetricsHelper",
            1360,
            "1.7.7",
            "https://cdn.esoui.com/downloads/file1360.zip",
            0,
        );
        metadata::record_install_ext(
            &mut store,
            "OtherAddon",
            3520,
            "1.0.0",
            "https://cdn.esoui.com/downloads/file3520.zip",
            0,
        );
        metadata::save_metadata(&addons_root, &store).expect("save metadata");

        let models = test_addon_models(vec![
            addon_entry(
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
            ),
            addon_entry(
                "CombatMetrics Helper",
                "CombatMetricsHelper",
                "1360",
                "Solinur",
                "1.7.7",
                "101048",
                "Library",
                "3/3/2026",
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
                "Other Addon",
                "OtherAddon",
                "3520",
                "Author",
                "1.0.0",
                "101048",
                "Addon",
                "4/1/2026",
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
        ]);

        let removed = remove_addons_by_esoui_id(&models, "1360").expect("remove by esoui id");

        assert_eq!(
            removed,
            vec![
                "CombatMetrics".to_string(),
                "CombatMetricsHelper".to_string()
            ]
        );
        assert!(!addons_root.join("CombatMetrics").exists());
        assert!(!addons_root.join("CombatMetricsHelper").exists());
        assert!(addons_root.join("OtherAddon").exists());
        assert_eq!(models.all.borrow().len(), 1);
        assert_eq!(models.all.borrow()[0].folder_name.as_str(), "OtherAddon");

        let store = metadata::load_metadata(&addons_root);
        assert!(!store.addons.contains_key("CombatMetrics"));
        assert!(!store.addons.contains_key("CombatMetricsHelper"));
        assert!(store.addons.contains_key("OtherAddon"));

        if let Some(previous) = previous {
            std::env::set_var("KALPA_ADDONS_PATH", previous);
        } else {
            std::env::remove_var("KALPA_ADDONS_PATH");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_addon_list_export_matches_production_shape() {
        let root = test_temp_dir("addon-list-export");
        let addons_root = root.join("AddOns");
        fs::create_dir_all(addons_root.join("CombatMetrics")).expect("create addon folder");
        fs::create_dir_all(addons_root.join("LibCombat")).expect("create bundled folder");
        fs::create_dir_all(&addons_root).expect("create AddOns root");

        let mut store = metadata::MetadataStore::default();
        metadata::record_install_ext(
            &mut store,
            "CombatMetrics",
            1360,
            "1.7.7",
            "https://cdn.esoui.com/downloads/file1360.zip",
            0,
        );
        metadata::record_install_ext(
            &mut store,
            "LibCombat",
            1360,
            "1.7.7",
            "https://cdn.esoui.com/downloads/file1360.zip",
            0,
        );
        metadata::record_install_ext(
            &mut store,
            "MissingAddon",
            9999,
            "0.0.1",
            "https://cdn.esoui.com/downloads/missing.zip",
            0,
        );
        metadata::save_metadata(&addons_root, &store).expect("save metadata");

        let export = serde_json::from_str::<ExportData>(
            &export_addon_list_json(&addons_root).expect("export addon list"),
        )
        .expect("parse export json");

        assert_eq!(export.version, 1);
        assert_eq!(export.addons.len(), 1);
        assert_eq!(export.addons[0].folder_name, "CombatMetrics");
        assert_eq!(export.addons[0].esoui_id, 1360);
        assert_eq!(export.addons[0].version, "1.7.7");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_api_compatibility_reads_game_version_and_manifests() {
        let root = test_temp_dir("api-compat");
        let addons_root = root.join("AddOns");
        fs::create_dir_all(addons_root.join("CurrentAddon")).expect("create current addon");
        fs::create_dir_all(addons_root.join("OldAddon")).expect("create old addon");
        fs::write(root.join("AddOnSettings.txt"), "#Version 101048\n").expect("write settings");
        fs::write(
            addons_root.join("CurrentAddon").join("CurrentAddon.txt"),
            "## Title: Current Addon\n## APIVersion: 101048 101049\n",
        )
        .expect("write current manifest");
        fs::write(
            addons_root.join("OldAddon").join("OldAddon.txt"),
            "## Title: Old Addon\n## APIVersion: 101038\n",
        )
        .expect("write old manifest");

        let info = check_native_api_compatibility(&addons_root).expect("check compat");

        assert_eq!(info.game_api_version, 101048);
        assert_eq!(info.up_to_date_addons, vec!["Current Addon".to_string()]);
        assert_eq!(info.outdated_addons, vec!["Old Addon".to_string()]);
        assert_eq!(
            api_compat_summary(&info),
            "API 101048: 1 compatible, 1 outdated (Old Addon)."
        );

        let _ = fs::remove_dir_all(root);
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
    fn discover_category_filter_matches_title_and_author() {
        let installed = BTreeSet::new();
        let entries = vec![
            discover_entry(
                "1",
                "Combat Metrics",
                "Solinur",
                "Combat",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                1,
                &installed,
            ),
            discover_entry(
                "2",
                "Inventory Grid",
                "Crafty",
                "Bags",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                2,
                &installed,
            ),
        ];

        let title_matches = filter_category_discover_entries(&entries, "combat");
        assert_eq!(title_matches.len(), 1);
        assert_eq!(title_matches[0].title.as_str(), "Combat Metrics");

        let author_matches = filter_category_discover_entries(&entries, "crafty");
        assert_eq!(author_matches.len(), 1);
        assert_eq!(author_matches[0].title.as_str(), "Inventory Grid");
    }

    #[test]
    fn discover_url_input_resolves_esoui_id() {
        assert_eq!(
            esoui_id_from_input("https://www.esoui.com/downloads/info3520.html").as_deref(),
            Some("3520")
        );

        let installed = BTreeSet::new();
        let entries = discover_entries_for_tab(3, &installed, "", "1346");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title.as_str(), "Dolgubon's Lazy Writ Crafter");
        assert_eq!(entries[0].rank, 1);
    }
}
