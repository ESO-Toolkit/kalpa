mod auth;
/// Heap high-water-mark allocator for the uploader perf benchmark. The
/// `#[global_allocator]` below is installed ONLY under the `bench-alloc` feature,
/// so normal builds are unaffected.
pub mod bench_alloc;
mod commands;
mod edit_backups;
mod esoui;
mod file_hashes;
pub mod game_instances;
mod installer;
mod manifest;
mod manifest_cache;
mod metadata;
pub mod platform;
mod safe_migration;
mod saved_variables;
mod settings_store;
mod token_store;
pub mod uploader;

// Benchmark-only heap tracker (see `bench_alloc`). Installed solely under the
// `bench-alloc` feature so the app/release/normal-test builds keep the system
// allocator. Used by the `cargo test --features bench-alloc … --ignored` perf
// benchmark to report peak heap.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: bench_alloc::TrackingAlloc = bench_alloc::TrackingAlloc;

use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

/// Stores the approved addons directory path. Commands that perform
/// write operations validate the caller-supplied path against this
/// value, preventing a compromised webview from targeting arbitrary
/// filesystem locations.
pub struct ApprovedAddonsPath {
    pub configured: PathBuf,
    pub canonical: PathBuf,
}

pub struct AllowedAddonsPath(pub Mutex<Option<ApprovedAddonsPath>>);

/// Guards all load_metadata → modify → save_metadata sequences against
/// concurrent access (TOCTOU). Wrap every read-modify-write cycle in
/// `let _guard = lock.0.lock()…;` before touching the metadata store.
pub struct MetadataLock(pub Arc<Mutex<()>>);

pub struct TrayState(pub Mutex<Option<tauri::tray::TrayIcon>>);

/// Actions that can be triggered by a deep link URL.
#[derive(Clone)]
enum DeepLinkAction {
    /// Open a pack by ID: `kalpa://pack/{id}`
    Pack(String),
    /// Import a shared pack by code: `kalpa://share/{code}`
    Share(String),
    /// Install a roster pack by ID: `kalpa://install-pack/{id}`
    InstallPack(String),
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDeepLinkPayload {
    pub pack_id: Option<String>,
    pub share_code: Option<String>,
    pub install_pack_id: Option<String>,
}

pub struct PendingDeepLink(pub Mutex<PendingDeepLinkPayload>);

#[derive(Clone)]
pub struct PendingUpdate {
    pub zip_path: PathBuf,
    pub folder_name: String,
    pub esoui_id: u32,
    pub update_version: String,
    /// The downloaded ZIP's hash/signature map, computed once during conflict
    /// detection (`build_conflict_report`) and reused as the post-extraction
    /// baseline so the apply step doesn't re-decompress and re-hash the whole
    /// archive a second time. Empty only for entries created before this field
    /// existed or via paths that didn't compute it; the apply step falls back to
    /// hashing the ZIP in that case.
    ///
    /// Wrapped in `Arc` so cloning a `PendingUpdate` out of the pending map (and
    /// the apply step's reuse of this map) is a refcount bump rather than a deep
    /// copy of a many-entry hash map.
    pub zip_hashes: Arc<HashMap<String, String>>,
}

pub struct PendingUpdates(pub Arc<Mutex<HashMap<String, PendingUpdate>>>);

/// Per-operation cancellation flags for in-flight addon updates, keyed by a
/// frontend-supplied operation id. An update command registers a flag at the
/// start of its blocking work and removes it on every exit; `cancel_update`
/// sets the flag so the extraction loop aborts cooperatively. Work runs in
/// `spawn_blocking` with blocking I/O, so it cannot be aborted by dropping the
/// future — polling a shared flag between files is the only safe mechanism.
pub struct UpdateCancels(pub Arc<Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>);

/// Extract an action from a deep link URL.
fn parse_deep_link(url: &str) -> Option<DeepLinkAction> {
    let url = url.trim();

    // Share codes: kalpa://share/{code}
    if let Some(rest) = url.strip_prefix("kalpa://share/") {
        let code = rest.split(['/', '?', '#']).next()?.trim();
        if !code.is_empty() {
            return Some(DeepLinkAction::Share(code.to_string()));
        }
    }

    // Roster pack install: kalpa://install-pack/{id}
    if let Some(rest) = url.strip_prefix("kalpa://install-pack/") {
        let id = rest.split(['/', '?', '#']).next()?.trim();
        if !id.is_empty() {
            return Some(DeepLinkAction::InstallPack(id.to_string()));
        }
    }

    // Pack IDs: kalpa://pack/{id} or kalpa://packs/{id}
    let path = url
        .strip_prefix("kalpa://pack/")
        .or_else(|| url.strip_prefix("kalpa://packs/"))?;
    let id = path.split(['/', '?', '#']).next()?.trim();
    if id.is_empty() {
        None
    } else {
        Some(DeepLinkAction::Pack(id.to_string()))
    }
}

/// Focus the main window and emit the appropriate deep-link event.
fn emit_deep_link(app: &tauri::AppHandle, action: &DeepLinkAction) {
    if let Some(window) = app.get_webview_window("main") {
        webview_power::on_shown(app); // resume before showing (flash-free)
        let _ = window.show();
        let _ = window.set_focus();
    }
    match action {
        DeepLinkAction::Pack(id) => {
            let _ = app.emit("deep-link-pack", id.as_str());
        }
        DeepLinkAction::Share(code) => {
            let _ = app.emit("deep-link-share", code.as_str());
        }
        DeepLinkAction::InstallPack(id) => {
            let _ = app.emit("roster-pack-install", id.as_str());
        }
    }
}

fn pending_deep_link_payload(action: &DeepLinkAction) -> PendingDeepLinkPayload {
    match action {
        DeepLinkAction::Pack(id) => PendingDeepLinkPayload {
            pack_id: Some(id.clone()),
            ..Default::default()
        },
        DeepLinkAction::Share(code) => PendingDeepLinkPayload {
            share_code: Some(code.clone()),
            ..Default::default()
        },
        DeepLinkAction::InstallPack(id) => PendingDeepLinkPayload {
            install_pack_id: Some(id.clone()),
            ..Default::default()
        },
    }
}

/// Clear the WebView2 cache when the app version changes.
///
/// The NSIS updater replaces the binary and bundled frontend assets, but
/// WebView2 keeps its own disk cache under `%LOCALAPPDATA%\{identifier}\EBWebView`.
/// Stale cached JS/CSS causes the UI to look outdated after an update.
/// We store the last-seen version in a marker file and nuke the cache dir
/// whenever it differs from the current build version.
///
/// Windows/WebView2-specific: WKWebView (macOS) and WebKitGTK (Linux) don't
/// exhibit the stale-cache-after-update bug this works around, and deleting
/// their cache directories would risk clobbering unrelated webview state.
#[cfg(windows)]
fn clear_webview_cache_on_upgrade() {
    let current = env!("CARGO_PKG_VERSION");
    let local_app_data = match std::env::var("LOCALAPPDATA") {
        Ok(v) => std::path::PathBuf::from(v),
        Err(_) => return,
    };
    let data_dir = local_app_data.join("com.kalpa.desktop");
    let marker = data_dir.join(".kalpa-version");

    let previous = std::fs::read_to_string(&marker).unwrap_or_default();
    if previous.trim() == current {
        return;
    }

    let cache_dir = data_dir.join("EBWebView");
    if cache_dir.exists() {
        let _ = std::fs::remove_dir_all(&cache_dir);
    }

    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::write(&marker, current);
}

#[cfg(not(windows))]
fn clear_webview_cache_on_upgrade() {}

fn cleanup_orphaned_pending_zips() {
    // Fire-and-forget: enumerating %TEMP% can take a while on cluttered
    // machines and nothing in the launch path depends on the sweep having
    // finished (no current code produces matching names during startup), so
    // keep it off the critical path to window creation.
    std::thread::spawn(|| {
        let temp_dir = std::env::temp_dir();
        if let Ok(entries) = std::fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("kalpa-pending-") && name.ends_with(".zip") {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    });
}

/// Windows-only WebView2 power management: shrink the renderer's memory while
/// Kalpa's main window is out of view, per Microsoft's guidance (two tiers that
/// compose exactly as documented):
///   - hidden + IDLE (no live-upload session running): `TrySuspend` the renderer
///     — pauses scripts/timers and lets the engine reclaim the most memory (the
///     renderer process can be parked). Resumed on show.
///   - hidden + LIVE (a live-upload session is streaming): only
///     `MemoryUsageTargetLevel = LOW` (evicts caches, nudges GC) so the webview
///     stays alive and the live feed keeps updating in the tray — TrySuspend
///     would freeze those event handlers.
///
/// `on_shown` resumes + restores NORMAL and is idempotent, so every path that
/// makes the window visible (tray, deep link, focus) can call it safely — this
/// guards against a dead/blank window on restore. It is also cheap when nothing
/// was saved: a `POWER_SAVED` flag short-circuits it to a no-op (no with_webview
/// / COM) unless a hide path actually reduced the webview first. TrySuspend requires the
/// controller be marked invisible first, which `window.hide()`/minimize does
/// NOT do for us, so it is set explicitly. All no-ops on non-Windows and on
/// WebView2 runtimes without the required interfaces (older than ~2022).
#[cfg(windows)]
mod webview_power {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::{AppHandle, Manager, WebviewWindow};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, ICoreWebView2_3, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use webview2_com::TrySuspendCompletedHandler;
    use windows::core::Interface; // brings `.cast()` into scope

    static SUSPENDED: AtomicBool = AtomicBool::new(false);
    // True whenever a hide path has actually reduced the webview (LOW or suspend)
    // and it has not yet been restored. Lets `on_shown` short-circuit to a cheap
    // no-op — skipping `with_webview` + COM entirely — when nothing was saved.
    static POWER_SAVED: AtomicBool = AtomicBool::new(false);

    fn set_memory_target(window: &WebviewWindow, low: bool) {
        let level = if low {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        };
        let _ = window.with_webview(move |webview| unsafe {
            if let Ok(core) = webview.controller().CoreWebView2() {
                if let Ok(core19) = core.cast::<ICoreWebView2_19>() {
                    let _ = core19.SetMemoryUsageTargetLevel(level);
                }
            }
        });
    }

    /// The main window was hidden (tray) or minimized. `live` = a live-upload
    /// session is active. COM calls run on the WebView2 UI thread via with_webview.
    pub fn on_hidden(app: &AppHandle, live: bool) {
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        if live {
            // Mark saved before touching webview state so `on_shown` will restore.
            POWER_SAVED.store(true, Ordering::SeqCst);
            set_memory_target(&window, true); // LOW: keep feed warm, trim caches
            return;
        }
        // Mark saved before the SUSPENDED early-return / SetIsVisible below.
        POWER_SAVED.store(true, Ordering::SeqCst);
        if SUSPENDED.swap(true, Ordering::SeqCst) {
            return; // already suspended
        }
        let _ = window.with_webview(|webview| {
            // SAFETY: runs on the WebView2 UI thread.
            unsafe {
                let controller = webview.controller();
                // TrySuspend requires IsVisible=false first, else ERROR_INVALID_STATE.
                if controller.SetIsVisible(false).is_err() {
                    return;
                }
                let Ok(core) = controller.CoreWebView2() else {
                    return;
                };
                if let Ok(core3) = core.cast::<ICoreWebView2_3>() {
                    let handler = TrySuspendCompletedHandler::create(Box::new(|_hr, _ok| Ok(())));
                    let _ = core3.TrySuspend(&handler); // best-effort, fire-and-forget
                }
            }
        });
    }

    /// A live-upload session just ended. At hide time a live session had only
    /// dropped the renderer to `MemoryUsageTargetLevel = LOW` (to keep its feed
    /// event handlers running in the tray) rather than deep-suspending it — and
    /// nothing re-runs that decision when the session ends while the window is
    /// still out of view. So if the main window is currently hidden (tray) or
    /// minimized, run the idle-hide path now to deep-suspend and reclaim the large
    /// idle-memory win. If the window is on-screen, do nothing: a later hide will
    /// suspend correctly. `on_hidden(_, false)` sets `SUSPENDED`, so a subsequent
    /// `on_shown` still resumes + restores NORMAL exactly as after a normal idle
    /// hide. No-op on non-Windows.
    pub fn on_live_session_ended(app: &AppHandle) {
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        // Only act when the window is genuinely out of view. Hidden-to-tray shows
        // up as `is_visible() == false`; a minimized window reports `is_visible()
        // == true` but `is_minimized() == true`. Both queries fail safe to
        // "on-screen" (do nothing) so a query error never suspends a live window.
        let hidden = !window.is_visible().unwrap_or(true);
        let minimized = window.is_minimized().unwrap_or(false);
        if hidden || minimized {
            on_hidden(app, false);
        }
    }

    /// The main window is being shown/focused. Resume if suspended and restore
    /// NORMAL. Idempotent — safe (and intended) to call from every show path.
    pub fn on_shown(app: &AppHandle) {
        // Nothing was ever reduced -> nothing to restore. Skip with_webview + COM.
        if !POWER_SAVED.swap(false, Ordering::SeqCst) {
            return;
        }
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let was_suspended = SUSPENDED.swap(false, Ordering::SeqCst);
        let _ = window.with_webview(move |webview| {
            // SAFETY: runs on the WebView2 UI thread.
            unsafe {
                let controller = webview.controller();
                let Ok(core) = controller.CoreWebView2() else {
                    return;
                };
                if was_suspended {
                    if let Ok(core3) = core.cast::<ICoreWebView2_3>() {
                        let _ = core3.Resume();
                    }
                    let _ = controller.SetIsVisible(true);
                }
                // Restore NORMAL (Resume already does this when suspended; this
                // also covers the LIVE/LOW-only path that never suspended).
                if let Ok(core19) = core.cast::<ICoreWebView2_19>() {
                    let _ = core19
                        .SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL);
                }
            }
        });
    }
}

#[cfg(not(windows))]
mod webview_power {
    use tauri::AppHandle;
    pub fn on_hidden(_app: &AppHandle, _live: bool) {}
    pub fn on_shown(_app: &AppHandle) {}
    pub fn on_live_session_ended(_app: &AppHandle) {}
}

pub fn run() {
    // msWebView2CodeCache: V8 bytecode caching for the app bundle. wry serves
    // the frontend through WebView2's WebResourceRequested interception, which
    // bypasses the HTTP-cache-backed code cache — without this feature the JS
    // bundle is re-parsed and re-compiled on every launch. The env var is
    // documented to APPEND to the environment's AdditionalBrowserArguments, so
    // wry's default args are preserved, and an unknown feature name is simply
    // ignored by the runtime, so this degrades gracefully if the flag ever
    // changes. (Debug builds also enable the Chrome DevTools Protocol here.)
    // WebView2 only exists on Windows, so don't set a dead env var elsewhere.
    #[cfg(all(windows, debug_assertions))]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--remote-debugging-port=9222 --enable-features=msWebView2CodeCache",
    );
    #[cfg(all(windows, not(debug_assertions)))]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--enable-features=msWebView2CodeCache",
    );

    clear_webview_cache_on_upgrade();
    cleanup_orphaned_pending_zips();

    tauri::Builder::default()
        .manage(AllowedAddonsPath(Mutex::new(None)))
        .manage(MetadataLock(Arc::new(Mutex::new(()))))
        .manage(auth::AuthState::new(None))
        .manage(TrayState(Mutex::new(None)))
        .manage(PendingDeepLink(Mutex::new(
            PendingDeepLinkPayload::default(),
        )))
        .manage(PendingUpdates(Arc::new(Mutex::new(HashMap::new()))))
        .manage(UpdateCancels(Arc::new(Mutex::new(HashMap::new()))))
        .manage(uploader::commands::UploaderState::default())
        // The native upload session provider, shared between the in-app login
        // (which captures the esologs cookie) and the upload path (which reads
        // it). `new()` rehydrates any cookie persisted on a prior run. Wrapped in
        // an `Arc` so the (blocking) native upload can clone an owned handle into
        // `spawn_blocking` while the login/status commands borrow it — all sharing
        // the one instance (so a mid-upload `invalidate` reaches the login path).
        .manage(std::sync::Arc::new(
            uploader::native::session::StoredSessionProvider::new(),
        ))
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // Focus the existing window when a duplicate instance is launched
            if let Some(window) = app.get_webview_window("main") {
                webview_power::on_shown(app); // resume before showing (flash-free)
                let _ = window.show();
                let _ = window.set_focus();
            }
            // Check argv for deep link URLs (Windows/Linux pass them as CLI args)
            for arg in &argv {
                if let Some(action) = parse_deep_link(arg) {
                    emit_deep_link(app, &action);
                    break;
                }
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            if let Some(action) = std::env::args().find_map(|arg| parse_deep_link(&arg)) {
                if let Ok(mut pending) = app.state::<PendingDeepLink>().0.lock() {
                    *pending = pending_deep_link_payload(&action);
                }
            }

            // Register the deep link scheme at runtime (for dev / non-installer builds)
            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let _ = app.deep_link().register_all();
            }

            let show_item = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let tray = TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .expect("default window icon must be set in tauri.conf.json"),
                )
                .tooltip("Kalpa")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            webview_power::on_shown(app); // resume before showing
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            webview_power::on_shown(app); // resume before showing
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            if let Ok(mut guard) = app.state::<TrayState>().0.lock() {
                *guard = Some(tray);
            }

            // Repair settings.json left partial/corrupt by an interrupted write,
            // BEFORE anything opens (and merge-loads) the store below: clear
            // uncommitted staging leftovers and quarantine a corrupt primary.
            settings_store::recover(app.handle());

            // Migrate auth tokens from plaintext store to credential manager
            // (one-time). This is also the first opener of the settings store.
            token_store::migrate_from_store(app.handle());

            // If that open swallowed a load error (plugin-store ignores them) and
            // left an empty cache while settings exist on disk, reload so a later
            // flush can't overwrite the user's settings with an empty store.
            settings_store::ensure_loaded(app.handle());

            // Load auth tokens from secure credential manager
            if let Some(tokens) = token_store::load_tokens() {
                if let Ok(mut guard) = app.state::<auth::AuthState>().tokens.lock() {
                    *guard = Some(tokens);
                }
            }

            // Note: settling upload-history records left in a transient state by
            // a previous run is deferred to first use of the uploader (see
            // `uploader_list_history`), so a user who never opens the uploader
            // pays no history read/parse at startup. It still runs at most once
            // per process, before the history panel first renders.

            Ok(())
        })
        .on_window_event(|window, event| {
            // Only the main window hides-to-tray + power-manages. Auxiliary
            // windows (e.g. the "esologs-login" sign-in webview) use default
            // behaviour — intercepting their close would leave them hidden but
            // alive, breaking flows that create a window, wait for the user, then
            // close it (the login flow detects cancel by the window disappearing).
            if window.label() != "main" {
                return;
            }
            let app = window.app_handle();
            let is_live = || {
                app.state::<uploader::commands::UploaderState>()
                    .has_active_live_session()
            };
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let _ = window.hide();
                    api.prevent_close();
                    // Backgrounded to tray. Idle -> deep-suspend the renderer;
                    // live-logging -> LOW so the feed keeps updating in the tray.
                    webview_power::on_hidden(app, is_live());
                }
                // Power-manage only when the window is genuinely out of view —
                // MINIMIZED — not merely unfocused. A multi-monitor player keeps
                // Kalpa visible on a second screen with the game focused and
                // watches the live feed there; that window must stay resumed at
                // NORMAL. Regaining focus resumes + restores NORMAL (also covers
                // restore-from-tray, which calls set_focus).
                tauri::WindowEvent::Focused(focused) => {
                    if *focused {
                        webview_power::on_shown(app);
                    } else if window.is_minimized().unwrap_or(false) {
                        webview_power::on_hidden(app, is_live());
                    }
                }
                // Closes the already-unfocused-minimize gap: if the window is
                // already unfocused (user clicked elsewhere) and is then minimized
                // from the taskbar, no `Focused` event fires, so the arm above
                // never runs. tao emits `Resized` on minimize/restore on Windows,
                // so we power-manage off the minimized state here. A visible
                // resize hits the `else` branch, and `on_shown` is a gated no-op
                // (see POWER_SAVED) when nothing was saved — so interactive
                // resizes of a visible window are effectively free and, per the
                // multi-monitor note above, correctly leave it resumed at NORMAL.
                tauri::WindowEvent::Resized(_) => {
                    if window.is_minimized().unwrap_or(false) {
                        webview_power::on_hidden(app, is_live());
                    } else {
                        webview_power::on_shown(app); // also covers restore-without-focus
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::set_addons_path,
            commands::check_addons_write_access,
            commands::open_ransomware_protection_settings,
            commands::detect_addons_folder,
            commands::detect_addons_folders,
            commands::scan_installed_addons,
            commands::set_addon_tags,
            commands::resolve_esoui_addon,
            commands::search_esoui_addons,
            commands::fetch_esoui_detail,
            commands::install_addon,
            commands::remove_addon,
            commands::disable_addon,
            commands::enable_addon,
            commands::install_dependency,
            commands::check_for_updates,
            commands::update_addon,
            commands::batch_update_addons,
            commands::export_addon_list,
            commands::import_addon_list,
            commands::auto_link_addons,
            commands::batch_remove_addons,
            commands::batch_set_tags,
            commands::batch_set_enabled,
            commands::batch_install_pack_addons,
            commands::get_esoui_categories,
            commands::browse_esoui_category,
            commands::browse_esoui_popular,
            commands::check_api_compatibility,
            commands::list_backups,
            commands::create_backup,
            commands::restore_backup_safe,
            commands::get_backups_folder_path,
            commands::delete_backup,
            commands::list_profiles,
            commands::create_profile,
            commands::activate_profile,
            commands::delete_profile,
            commands::list_characters,
            commands::backup_character_settings,
            commands::detect_minion,
            commands::migrate_from_minion,
            commands::migration_check_preconditions,
            commands::migration_create_snapshot,
            commands::migration_dry_run,
            commands::migration_execute,
            commands::migration_check_integrity,
            commands::list_snapshots,
            commands::restore_snapshot,
            commands::delete_snapshot,
            commands::create_pre_operation_snapshot,
            commands::read_ops_log,
            commands::backup_minion_config,
            commands::list_packs,
            commands::get_pack,
            commands::auth_login,
            commands::auth_logout,
            commands::auth_get_user,
            commands::consume_initial_deep_link,
            commands::create_pack,
            commands::update_pack,
            commands::delete_pack,
            commands::delete_pack_hub_account,
            commands::vote_pack,
            commands::track_pack_install,
            commands::create_share_code,
            commands::resolve_share_code,
            commands::export_pack_file,
            commands::import_pack_file,
            commands::export_sv_settings,
            commands::import_sv_settings,
            commands::detect_local_identities,
            commands::fetch_roster_pack,
            commands::get_saved_variables_path,
            commands::list_saved_variables,
            commands::read_saved_variable,
            commands::write_saved_variable,
            commands::copy_sv_profile,
            commands::is_eso_running,
            commands::is_portable_update_supported,
            commands::delete_saved_variables,
            commands::restore_sv_backup,
            commands::preview_sv_save,
            commands::detect_game_instances,
            commands::update_tray_tooltip,
            commands::scan_update_conflicts,
            commands::scan_batch_conflicts,
            commands::get_conflict_diff,
            commands::update_addon_with_decisions,
            commands::update_batch_with_decisions,
            commands::list_addon_files,
            commands::read_addon_file,
            commands::write_addon_file,
            commands::rescan_addon_hashes,
            commands::cancel_pending_update,
            commands::cancel_update,
            commands::list_edit_backups,
            commands::restore_edit_backup,
            uploader::commands::uploader_detect_path,
            uploader::commands::uploader_list_logs,
            uploader::commands::uploader_preflight,
            uploader::commands::uploader_probe_live_readiness,
            uploader::commands::uploader_split_to_disk,
            uploader::commands::uploader_split_to_disk_named,
            uploader::commands::uploader_split_fights_to_disk,
            uploader::commands::uploader_import_log,
            uploader::commands::uploader_delete_log,
            uploader::commands::uploader_restore_log,
            uploader::commands::uploader_transport_info,
            uploader::commands::uploader_login_esologs,
            uploader::commands::uploader_has_session,
            uploader::commands::uploader_logout_esologs,
            uploader::commands::uploader_upload_log,
            uploader::commands::uploader_start_live,
            uploader::commands::uploader_stop_live,
            uploader::commands::uploader_list_history,
            uploader::commands::uploader_delete_history,
            uploader::commands::uploader_attach_report,
            #[cfg(debug_assertions)]
            uploader::commands::uploader_run_native_live_spike,
            commands::flush_settings,
            commands::settings_tainted,
            #[cfg(debug_assertions)]
            commands::dev_scrub_saved_variable,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // On a real app exit, detach settings.json from the plugin registry so
            // tauri-plugin-store's own RunEvent::Exit handler can't truncate-write
            // it. ExitRequested fires before Exit (and before the plugin's exit
            // save), so detaching here neutralises that non-atomic write. Settings
            // are already persisted atomically on every write, so nothing is
            // flushed here. (Window close hides to tray and never reaches this.)
            if let tauri::RunEvent::ExitRequested { .. } = &event {
                settings_store::detach_on_exit(app);
            }
            // On any real process exit, signal every native live session to stop so
            // its terminate-report + abandoned POSTs settle promptly (the OS reaps
            // the driver threads; we don't join here, to avoid blocking exit on a
            // wedged network). A hard exit's correctness is covered by the L2 orphan
            // breadcrumb + next-launch recovery — this just closes reports faster.
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = &event {
                if let Some(state) = app.try_state::<uploader::commands::UploaderState>() {
                    state.signal_all_live_stop();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_install_pack() {
        match parse_deep_link("kalpa://install-pack/trial-essentials") {
            Some(DeepLinkAction::InstallPack(id)) => assert_eq!(id, "trial-essentials"),
            other => panic!("expected InstallPack, got {:?}", other.is_some()),
        }
    }

    #[test]
    fn parse_install_pack_strips_query_and_fragment() {
        match parse_deep_link("kalpa://install-pack/my-pack?ref=web#top") {
            Some(DeepLinkAction::InstallPack(id)) => assert_eq!(id, "my-pack"),
            other => panic!("expected InstallPack, got {:?}", other.is_some()),
        }
    }

    #[test]
    fn parse_install_pack_strips_trailing_slash() {
        match parse_deep_link("kalpa://install-pack/my-pack/") {
            Some(DeepLinkAction::InstallPack(id)) => assert_eq!(id, "my-pack"),
            other => panic!("expected InstallPack, got {:?}", other.is_some()),
        }
    }

    #[test]
    fn parse_install_pack_rejects_empty_id() {
        assert!(parse_deep_link("kalpa://install-pack/").is_none());
    }

    #[test]
    fn parse_pack() {
        match parse_deep_link("kalpa://pack/some-id") {
            Some(DeepLinkAction::Pack(id)) => assert_eq!(id, "some-id"),
            other => panic!("expected Pack, got {:?}", other.is_some()),
        }
    }

    #[test]
    fn parse_packs_alias() {
        match parse_deep_link("kalpa://packs/some-id") {
            Some(DeepLinkAction::Pack(id)) => assert_eq!(id, "some-id"),
            other => panic!("expected Pack, got {:?}", other.is_some()),
        }
    }

    #[test]
    fn parse_share() {
        match parse_deep_link("kalpa://share/abc123") {
            Some(DeepLinkAction::Share(code)) => assert_eq!(code, "abc123"),
            other => panic!("expected Share, got {:?}", other.is_some()),
        }
    }

    #[test]
    fn parse_unknown_scheme_returns_none() {
        assert!(parse_deep_link("kalpa://unknown/foo").is_none());
        assert!(parse_deep_link("https://example.com").is_none());
        assert!(parse_deep_link("").is_none());
    }

    #[test]
    fn parse_trims_whitespace() {
        match parse_deep_link("  kalpa://install-pack/my-pack  ") {
            Some(DeepLinkAction::InstallPack(id)) => assert_eq!(id, "my-pack"),
            other => panic!("expected InstallPack, got {:?}", other.is_some()),
        }
    }

    #[test]
    fn install_pack_does_not_match_pack_prefix() {
        // "kalpa://install-pack/x" must NOT match the "kalpa://pack/" branch
        match parse_deep_link("kalpa://install-pack/x") {
            Some(DeepLinkAction::InstallPack(_)) => {}
            other => panic!("expected InstallPack, got {:?}", other.is_some()),
        }
    }
}
