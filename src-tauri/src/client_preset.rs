//! Presets: switching between them, and fixing the one ordering mistake that
//! does not announce itself.
//!
//! # Why order is worth a feature
//!
//! `DLSS5_Feed` consumes motion vectors that another effect produces. ReShade
//! runs techniques in the order the preset lists them, so a preset that enables
//! the feed *above* its provider compiles cleanly, loads cleanly, errors
//! nothing — and feeds DLSS last frame's data. Nothing about either file is
//! defective. The image is just quietly wrong. That is the whole reason this
//! module offers a fix rather than only a report.
//!
//! # Which effect is the provider is not a constant
//!
//! It is resolved per preset by [`crate::client_stack`] from `DLSS5_MV_SOURCE`
//! and `MV_PROVIDER`: iMMERSE LaunchPad, or whichever enabled effect writes the
//! shared `texMotionVectors` texture. The fix moves *that* technique, read from
//! [`crate::client_stack::MvProvider`], never a hardcoded name — assuming
//! LaunchPad is what made the check silently not run in the first place.
//!
//! # Surgical edits only
//!
//! Both writes here change exactly one line of one file: `PresetPath` in
//! `ReShade.ini`, or `Techniques` in the preset. Everything else in the file
//! survives byte for byte, including comments, key order, per-effect uniform
//! blocks and the file's line endings. ReShade and its add-ons own these files;
//! Kalpa is editing one value in someone else's document, and a reserialised
//! file would be a whole-file diff for a one-value change.

use crate::client_stack::ClientStack;
use crate::client_write::AllowedGameInstallPath;
use serde::Serialize;
use std::path::Path;

/// One preset the user could switch to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetChoice {
    /// File name relative to the client directory.
    pub relative_path: String,
    /// The form that goes into `PresetPath`, e.g. `.\ReShadePreset.ini`.
    pub preset_path: String,
    pub is_active: bool,
    /// How many techniques it enables, so the list is more than a row of
    /// identical file names.
    pub technique_count: usize,
}

/// The `Techniques` reordering Kalpa would perform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrderFix {
    /// The technique that has to run first, resolved from the preset's own
    /// `MV_PROVIDER`.
    pub provider_technique: String,
    pub feed_technique: String,
    /// The `Techniques=` value as it is now.
    pub before: String,
    /// The `Techniques=` value the fix would write.
    pub after: String,
    /// One line the confirmation shows verbatim.
    pub summary: String,
}

/// Everything the preset panel needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetOptions {
    pub client_dir: String,
    /// The active preset's relative path, when `PresetPath` names a file that
    /// is actually there.
    pub active: Option<String>,
    /// Every preset found at the client root, sorted by file name, active first
    /// only if that is where sorting puts it — the list order must not encode a
    /// recommendation.
    pub choices: Vec<PresetChoice>,
    /// The fix, when the active preset runs the feed before its provider.
    /// `None` when the order is right, when there is no feed technique, or when
    /// no provider technique is enabled at all — that last case is a different
    /// problem (`stack-mv-provider-missing`) that reordering cannot solve, and
    /// offering a reorder for it would be a fix that changes nothing.
    pub fix: Option<OrderFix>,
}

/// The result of a preset switch or an order fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetChangeOutcome {
    /// The file that was edited, relative to the client directory.
    pub relative_path: String,
    /// Backup folder id holding the previous contents.
    pub backup_id: Option<String>,
    /// What changed, in one line, for the toast.
    pub summary: String,
}

/// Find the presets in a client directory.
///
/// A preset is a `.ini` at the client root that is not `ReShade.ini` and that
/// has a `Techniques` key. The key is the test rather than the file name:
/// ReShade does not enforce a naming convention and users rename presets freely,
/// so matching on `*Preset*.ini` would hide half of them.
pub fn find_presets(client_dir: &Path, active_relative: Option<&str>) -> Vec<PresetChoice> {
    let _ = (client_dir, active_relative);
    todo!("find_presets")
}

/// Turn a relative preset file name into the `PresetPath` form ReShade writes.
///
/// ReShade uses `.\Name.ini`. Kalpa writes the same shape rather than an
/// absolute path: an absolute path in this key would break the moment the user
/// moved or reinstalled the game.
pub fn to_preset_path(relative_path: &str) -> String {
    let _ = relative_path;
    todo!("to_preset_path")
}

/// The inverse of [`to_preset_path`], tolerant of `.\`, `./` and a bare name.
pub fn from_preset_path(preset_path: &str) -> String {
    let _ = preset_path;
    todo!("from_preset_path")
}

/// Work out the ordering fix for a stack, or `None` when there is nothing to
/// fix. See [`PresetOptions::fix`] for exactly when this is `None`.
///
/// The fix moves the provider technique to sit immediately before the feed,
/// leaving every other technique in its existing relative order. Techniques
/// keep their `name@source.fx` spelling exactly as the preset had them.
pub fn plan_order_fix(stack: &ClientStack, preset_contents: &str) -> Option<OrderFix> {
    let _ = (stack, preset_contents);
    todo!("plan_order_fix")
}

/// Rewrite one `key=value` line in an INI file, leaving the rest byte for byte.
///
/// `section` is `""` for the preset's headerless top block, where `Techniques`
/// lives. Preserves the line's original terminator and the file's; returns `Err`
/// when the key is not present, because inventing a `PresetPath` or a
/// `Techniques` line in a file that has none would be Kalpa writing
/// configuration rather than editing it.
pub fn replace_ini_value(
    contents: &str,
    section: &str,
    key: &str,
    value: &str,
) -> Result<String, String> {
    let _ = (contents, section, key, value);
    todo!("replace_ini_value")
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Read-only: the presets available and whether the active one is misordered.
#[tauri::command]
pub fn list_client_presets(client_dir: String) -> Result<PresetOptions, String> {
    let _ = client_dir;
    todo!("list_client_presets")
}

/// Switch the active preset by rewriting `PresetPath` in `ReShade.ini`.
///
/// Refuses a preset that is not one of the choices [`find_presets`] returns,
/// rather than trusting a path from the frontend — the write path would accept
/// any `.ini` under the client folder, and "the user picked it from a list" is
/// only true if the backend checks it against the same list.
#[tauri::command]
pub async fn set_client_preset(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    relative_path: String,
) -> Result<PresetChangeOutcome, String> {
    let _ = (app, state, client_dir, relative_path);
    todo!("set_client_preset")
}

/// Reorder the active preset's `Techniques` so the provider runs before the feed.
///
/// The fix is recomputed here from the folder rather than accepted from the
/// caller, for the same reason `adopt_stack` recomputes its own plan.
#[tauri::command]
pub async fn fix_client_technique_order(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
) -> Result<PresetChangeOutcome, String> {
    let _ = (app, state, client_dir);
    todo!("fix_client_technique_order")
}
