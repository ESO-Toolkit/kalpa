//! Tuning the RenoDX DLSS 5 add-on: the `[RenoDX.DLSS5]` block of `ReShade.ini`.
//!
//! # Draft, then apply — one write
//!
//! ReShade owns `ReShade.ini`. It reads it at startup and **rewrites the whole
//! file itself** when it exits or when a setting changes in its overlay. A UI
//! that saved on every slider drag would be writing into a file another process
//! rewrites wholesale, dozens of times per session, each write racing the last.
//! So the panel edits a draft in memory and applies it once, and the status
//! afterwards is **"Applies at next launch"** — not "Saved". The add-on reads
//! these values when it initialises; changing the file under a running game
//! changes nothing until it starts again, and saying "Saved" would imply
//! otherwise.
//!
//! The apply refuses outright while ESO or its launcher is running. Not a
//! warning: the launcher's patcher and ReShade both write this folder, and a
//! warning the user can click past is an invitation to lose the file.
//!
//! # Where the labels come from
//!
//! `renodx-dlss5.addon64` is closed source. Every label and enum value in
//! [`FIELDS`] was read out of the add-on binary's own string table, where an
//! ImGui label sits immediately before the config key it belongs to. **Nothing
//! here is invented.** A key whose meaning was not recoverable would have the
//! raw key as its label rather than a guess.
//!
//! # What is deliberately not known
//!
//! The float minimums and maximums are numeric immediates in the add-on's code,
//! not strings, and there is no public source. They are **not guessed**. Each
//! float field carries a *display* range derived to contain the value already in
//! the file (`NRLocalStructure=1.4` in a real install proves these are not all
//! 0–1), and the UI shows a numeric box beside the slider which is authoritative:
//! the slider range re-derives from whatever the box holds. Nothing is ever
//! clamped on load — silently rewriting a working value to fit a range Kalpa
//! made up would be the worst possible failure here.
//!
//! `NRToggleKey` and `NRScreenshotKey` are rendered as raw numbers labelled
//! "key code", because which key-code convention the add-on uses is unconfirmed.
//! There is no key-binding capture; offering one would mean claiming to know the
//! mapping.

use crate::client_write::AllowedGameInstallPath;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The INI section these settings live in, as ReShade writes it.
pub const TUNING_SECTION: &str = "RenoDX.DLSS5";

/// The file the section lives in.
pub const TUNING_FILE: &str = "ReShade.ini";

/// How the UI should render one setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningControl {
    /// `0`/`1`.
    Toggle,
    /// A fixed set of integer values, each with the add-on's own wording.
    Choice,
    /// A float, with a numeric box and a derived slider range.
    Float,
    /// A raw key code. Shown as a number, never as a key name.
    KeyCode,
}

/// Which part of the add-on's own UI a setting belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningGroup {
    /// The master switch and the overall look.
    NeuralRendering,
    /// Per-region detail strengths.
    Detail,
    /// Colour and HDR handling.
    Color,
    /// Hotkeys.
    Keys,
    /// The add-on's own section header for these reads "Guide overrides (leave
    /// at defaults unless diagnostics require them)", so they are tucked away
    /// rather than mixed into the settings a user is meant to touch.
    Advanced,
}

/// One value of a [`TuningControl::Choice`] field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceOption {
    pub value: i64,
    /// The add-on's own wording for this value.
    pub label: String,
}

/// A setting's static definition — everything that does not depend on the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSpec {
    /// Canonical spelling of the key, as it should be written back.
    pub key: &'static str,
    /// The add-on's own label.
    pub label: &'static str,
    pub control: TuningControl,
    pub group: TuningGroup,
    /// Empty except for [`TuningControl::Choice`].
    pub choices: &'static [(i64, &'static str)],
    /// Decimal places the add-on itself displays: 2 for `%.2f`, 3 for `%.3f`.
    /// Ignored for non-float controls.
    pub decimals: u8,
    /// A sentence of context, or empty when the label says it all. Never a
    /// guess about what a value does.
    pub help: &'static str,
}

/// One setting as the panel shows it: its definition plus what is in the file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TuningField {
    pub key: String,
    pub label: String,
    pub control: TuningControl,
    pub group: TuningGroup,
    pub choices: Vec<ChoiceOption>,
    pub decimals: u8,
    pub help: String,
    /// The value exactly as it appears in `ReShade.ini`, or `None` when the key
    /// is absent. Never normalised, never clamped.
    pub current: Option<String>,
    /// Display range for a float slider, derived to contain `current`. Both
    /// `None` for non-float controls.
    ///
    /// This is **not a constraint**. The real limits are unknown, the numeric
    /// box beside the slider accepts anything that parses, and this range
    /// re-derives from whatever it holds.
    pub slider_min: Option<f64>,
    pub slider_max: Option<f64>,
}

/// The whole panel's data.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TuningForm {
    pub client_dir: String,
    /// True when `ReShade.ini` has a `[RenoDX.DLSS5]` section at all. False
    /// means the add-on has never run here, and the panel should say so rather
    /// than offer to write a section from nothing.
    pub section_present: bool,
    /// In [`FIELDS`] order, which is the add-on's own order.
    pub fields: Vec<TuningField>,
    /// Keys found in the section that [`FIELDS`] does not describe, verbatim.
    /// Shown read-only: an unknown key is a newer add-on build, and silently
    /// dropping it on the next write would delete a setting the user relies on.
    pub unknown: Vec<(String, String)>,
}

/// One change the user made in the draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuningEdit {
    pub key: String,
    /// The value to write, already formatted. Validated against the field's
    /// control before anything is written.
    pub value: String,
}

/// The result of applying a draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TuningApplyOutcome {
    /// Keys actually changed, in the order they were written.
    pub changed: Vec<String>,
    /// Backup folder id holding the previous `ReShade.ini`.
    pub backup_id: Option<String>,
}

/// Every setting the add-on exposes, in the add-on's own order.
///
/// Extracted from the add-on binary's string table. See the module doc: this
/// table is transcription, not design, and a key that is not here is reported
/// as unknown rather than assumed.
pub const FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "NeuralUplift",
        label: "Enable DLSS Neural Rendering",
        control: TuningControl::Toggle,
        group: TuningGroup::NeuralRendering,
        choices: &[],
        decimals: 0,
        help: "The master switch for Neural Rendering.",
    },
    FieldSpec {
        key: "NREnableUpscaling",
        label: "Enable Upscaling (WIP)",
        control: TuningControl::Toggle,
        group: TuningGroup::NeuralRendering,
        choices: &[],
        decimals: 0,
        help: "",
    },
    FieldSpec {
        key: "NRPreset",
        label: "NR Preset",
        control: TuningControl::Choice,
        group: TuningGroup::NeuralRendering,
        choices: &[
            (0, "Default"),
            (1, "Preset #1"),
            (2, "Preset #2"),
            (3, "Preset #3"),
        ],
        decimals: 0,
        help: "",
    },
    FieldSpec {
        key: "NRStyle",
        label: "NR Style",
        control: TuningControl::Choice,
        group: TuningGroup::NeuralRendering,
        choices: &[(0, "Natural"), (1, "Cinematic")],
        decimals: 0,
        help: "",
    },
    FieldSpec {
        key: "NRIntensity",
        label: "NR Intensity",
        control: TuningControl::Float,
        group: TuningGroup::NeuralRendering,
        choices: &[],
        decimals: 2,
        help: "",
    },
    FieldSpec {
        key: "NRLocalTone",
        label: "Local Tone Strength",
        control: TuningControl::Float,
        group: TuningGroup::Detail,
        choices: &[],
        decimals: 2,
        help: "",
    },
    FieldSpec {
        key: "NRLocalStructure",
        label: "Local Structure Strength",
        control: TuningControl::Float,
        group: TuningGroup::Detail,
        choices: &[],
        decimals: 2,
        help: "",
    },
    FieldSpec {
        key: "NRSkinStructure",
        label: "Skin Structure Strength",
        control: TuningControl::Float,
        group: TuningGroup::Detail,
        choices: &[],
        decimals: 2,
        help: "",
    },
    FieldSpec {
        key: "NRAutoMask",
        label: "Automatic Mask",
        control: TuningControl::Toggle,
        group: TuningGroup::Detail,
        choices: &[],
        decimals: 0,
        help: "",
    },
    FieldSpec {
        key: "NRUICorrection",
        label: "NR UI Correction",
        control: TuningControl::Toggle,
        group: TuningGroup::Color,
        choices: &[],
        decimals: 0,
        help: "",
    },
    FieldSpec {
        key: "NRPaperWhiteScale",
        label: "Scene Paper-White Scale",
        control: TuningControl::Float,
        group: TuningGroup::Color,
        choices: &[],
        decimals: 3,
        help: "",
    },
    FieldSpec {
        key: "NRTransferStrength",
        label: "HDR Transfer Strength",
        control: TuningControl::Float,
        group: TuningGroup::Color,
        choices: &[],
        decimals: 2,
        help: "",
    },
    FieldSpec {
        key: "NRColorStrength",
        label: "Color Strength",
        control: TuningControl::Float,
        group: TuningGroup::Color,
        choices: &[],
        decimals: 2,
        help: "",
    },
    FieldSpec {
        key: "NRToggleKey",
        label: "NR Toggle Key",
        control: TuningControl::KeyCode,
        group: TuningGroup::Keys,
        choices: &[],
        decimals: 0,
        help: "Raw key code. Which convention the add-on uses is unconfirmed, \
               so Kalpa shows the number rather than naming a key.",
    },
    FieldSpec {
        key: "NRScreenshotKey",
        label: "Screenshot Key",
        control: TuningControl::KeyCode,
        group: TuningGroup::Keys,
        choices: &[],
        decimals: 0,
        help: "Raw key code. Which convention the add-on uses is unconfirmed, \
               so Kalpa shows the number rather than naming a key.",
    },
    FieldSpec {
        key: "NRDepthMode",
        label: "Depth Convention",
        control: TuningControl::Choice,
        group: TuningGroup::Advanced,
        choices: &[
            (0, "Use game NGX flag"),
            (1, "Force normal depth"),
            (2, "Force inverted depth"),
        ],
        decimals: 0,
        help: "",
    },
    FieldSpec {
        key: "NRMVecScaleX",
        label: "Motion Scale X Multiplier",
        control: TuningControl::Float,
        group: TuningGroup::Advanced,
        choices: &[],
        decimals: 2,
        help: "",
    },
    FieldSpec {
        key: "NRMVecScaleY",
        label: "Motion Scale Y Multiplier",
        control: TuningControl::Float,
        group: TuningGroup::Advanced,
        choices: &[],
        decimals: 2,
        help: "",
    },
    FieldSpec {
        key: "EnableHooks",
        label: "EnableHooks",
        control: TuningControl::Toggle,
        group: TuningGroup::Advanced,
        choices: &[],
        decimals: 0,
        help: "Applies to Streamline titles only.",
    },
];

/// Look up a field by key, case-insensitively — ReShade does not preserve case.
pub fn field_for(key: &str) -> Option<&'static FieldSpec> {
    let _ = key;
    todo!("field_for")
}

/// Build the panel's data from the text of `ReShade.ini`.
///
/// Values are reported exactly as they appear. Nothing is normalised, defaulted
/// or clamped: a value Kalpa does not understand still belongs to the user.
pub fn read_form(reshade_ini: &str, client_dir: &str) -> TuningForm {
    let _ = (reshade_ini, client_dir);
    todo!("read_form")
}

/// The display range for a float slider, derived so it contains `current`.
///
/// Not a constraint — see [`TuningField::slider_min`]. The rule is: start at
/// `0.0`, or at the floor of `current` when that is negative; end at `1.0`, or
/// at the next whole 0.5 step above `current` when that is larger. Deterministic
/// so the slider does not jump around between reads of the same file.
pub fn slider_range(current: Option<f64>) -> (f64, f64) {
    let _ = current;
    todo!("slider_range")
}

/// Check one edit against its field before anything is written.
///
/// Refuses: a key not in [`FIELDS`]; a toggle that is not `0` or `1`; a choice
/// value not among the field's own options; a float or key code that does not
/// parse. A refusal names the key and the reason.
pub fn validate_edit(edit: &TuningEdit) -> Result<(), String> {
    let _ = edit;
    todo!("validate_edit")
}

/// Produce the new text of `ReShade.ini` with `edits` applied to the
/// `[RenoDX.DLSS5]` section.
///
/// **Everything outside that section must survive byte for byte**, including
/// comments, blank lines, key order, unknown keys, and the file's line endings
/// (ReShade writes CRLF on Windows; rewriting the file as LF would be a
/// gratuitous whole-file diff and would confuse any tool diffing it). Within the
/// section, a key that is already present is edited in place; a key that is not
/// is appended at the end of the section, before the next `[header]`.
///
/// Returns `Err` when the section does not exist — writing one from nothing
/// would be Kalpa inventing configuration for an add-on that has never run.
pub fn apply_edits(reshade_ini: &str, edits: &[TuningEdit]) -> Result<String, String> {
    let _ = (reshade_ini, edits);
    todo!("apply_edits")
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Read-only: the current tuning values.
#[tauri::command]
pub fn read_client_tuning(client_dir: String) -> Result<TuningForm, String> {
    let _ = client_dir;
    todo!("read_client_tuning")
}

/// Apply a draft: one validated, backed-up, atomic write of `ReShade.ini`.
///
/// Refuses while ESO or its launcher is running — that is `begin_write`'s job
/// and it is a refusal, not a warning.
#[tauri::command]
pub async fn apply_client_tuning(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    edits: Vec<TuningEdit>,
) -> Result<TuningApplyOutcome, String> {
    let _ = (app, state, client_dir, edits);
    todo!("apply_client_tuning")
}

/// Where `ReShade.ini` is, for a validated client directory.
pub fn tuning_file_path(client_dir: &Path) -> std::path::PathBuf {
    client_dir.join(TUNING_FILE)
}
