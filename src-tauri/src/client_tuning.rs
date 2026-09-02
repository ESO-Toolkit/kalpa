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
    FIELDS.iter().find(|f| f.key.eq_ignore_ascii_case(key))
}

/// Split a raw line (as produced by `str::split_inclusive('\n')`) into its
/// content and its original terminator (`"\r\n"`, `"\n"`, or `""` for a final
/// line with none).
fn split_terminator(raw: &str) -> (&str, &str) {
    if let Some(stripped) = raw.strip_suffix("\r\n") {
        (stripped, "\r\n")
    } else if let Some(stripped) = raw.strip_suffix('\n') {
        (stripped, "\n")
    } else {
        (raw, "")
    }
}

/// Is this trimmed line a `[Section]` header, and if so, its name (trimmed)?
fn section_header(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(|name| name.trim())
}

/// Build the panel's data from the text of `ReShade.ini`.
///
/// Values are reported exactly as they appear. Nothing is normalised, defaulted
/// or clamped: a value Kalpa does not understand still belongs to the user.
pub fn read_form(reshade_ini: &str, client_dir: &str) -> TuningForm {
    let mut section_present = false;
    let mut in_section = false;
    // Last occurrence of a key wins, as it would when ReShade itself reads the
    // file; order of first appearance is kept for the `unknown` list.
    let mut found: Vec<(String, String)> = Vec::new();

    for raw in reshade_ini.split_inclusive('\n') {
        let (content, _) = split_terminator(raw);
        let trimmed = content.trim();
        if let Some(name) = section_header(trimmed) {
            in_section = name.eq_ignore_ascii_case(TUNING_SECTION);
            if in_section {
                section_present = true;
            }
            continue;
        }
        if !in_section {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim().to_string();
        if let Some(existing) = found.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&key)) {
            existing.1 = value;
        } else {
            found.push((key, value));
        }
    }

    let fields = FIELDS
        .iter()
        .map(|spec| {
            let current = found
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(spec.key))
                .map(|(_, v)| v.clone());
            let (slider_min, slider_max) = if spec.control == TuningControl::Float {
                let parsed = current
                    .as_deref()
                    .and_then(|s| s.trim().parse::<f64>().ok());
                let (min, max) = slider_range(parsed);
                (Some(min), Some(max))
            } else {
                (None, None)
            };
            TuningField {
                key: spec.key.to_string(),
                label: spec.label.to_string(),
                control: spec.control,
                group: spec.group,
                choices: spec
                    .choices
                    .iter()
                    .map(|(value, label)| ChoiceOption {
                        value: *value,
                        label: label.to_string(),
                    })
                    .collect(),
                decimals: spec.decimals,
                help: spec.help.to_string(),
                current,
                slider_min,
                slider_max,
            }
        })
        .collect();

    let unknown = found
        .into_iter()
        .filter(|(key, _)| field_for(key).is_none())
        .collect();

    TuningForm {
        client_dir: client_dir.to_string(),
        section_present,
        fields,
        unknown,
    }
}

/// The display range for a float slider, derived so it contains `current`.
///
/// Not a constraint — see [`TuningField::slider_min`]. The rule is: start at
/// `0.0`, or at the floor of `current` when that is negative; end at `1.0`, or
/// at the next whole 0.5 step above `current` when that is larger. Deterministic
/// so the slider does not jump around between reads of the same file.
pub fn slider_range(current: Option<f64>) -> (f64, f64) {
    let min = match current {
        Some(v) if v < 0.0 => v.floor(),
        _ => 0.0,
    };
    let max = match current {
        Some(v) if v > 1.0 => (((v / 0.5).floor()) + 1.0) * 0.5,
        _ => 1.0,
    };
    (min, max)
}

/// Check one edit against its field before anything is written.
///
/// Refuses: a key not in [`FIELDS`]; a toggle that is not `0` or `1`; a choice
/// value not among the field's own options; a float or key code that does not
/// parse. A refusal names the key and the reason.
pub fn validate_edit(edit: &TuningEdit) -> Result<(), String> {
    let spec = field_for(&edit.key)
        .ok_or_else(|| format!("{} is not a known RenoDX DLSS 5 setting.", edit.key))?;
    match spec.control {
        TuningControl::Toggle => {
            if edit.value != "0" && edit.value != "1" {
                return Err(format!("{} must be 0 or 1.", spec.key));
            }
        }
        TuningControl::Choice => {
            let parsed: i64 = edit
                .value
                .trim()
                .parse()
                .map_err(|_| format!("{} must be one of its listed values.", spec.key))?;
            if !spec.choices.iter().any(|(value, _)| *value == parsed) {
                return Err(format!("{} must be one of its listed values.", spec.key));
            }
        }
        TuningControl::Float => {
            edit.value
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("{} must be a number.", spec.key))?;
        }
        TuningControl::KeyCode => {
            edit.value
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("{} must be a key code.", spec.key))?;
        }
    }
    Ok(())
}

/// One physical line of the file, kept as content plus its original
/// terminator so the file can be reassembled byte for byte.
struct Line {
    content: String,
    terminator: String,
}

/// The line ending most of the file uses, for lines newly appended by
/// [`apply_edits`]. Falls back to `"\n"` for a file with no terminated lines
/// at all (a single-line file with no trailing newline).
fn dominant_line_ending(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| line.terminator.as_str())
        .find(|term| !term.is_empty())
        .unwrap_or("\n")
        .to_string()
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
    let mut lines: Vec<Line> = reshade_ini
        .split_inclusive('\n')
        .map(|raw| {
            let (content, terminator) = split_terminator(raw);
            Line {
                content: content.to_string(),
                terminator: terminator.to_string(),
            }
        })
        .collect();

    let headers: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| section_header(line.content.trim()).is_some())
        .map(|(i, _)| i)
        .collect();
    let section_start = *headers
        .iter()
        .find(|&&i| {
            section_header(lines[i].content.trim())
                .is_some_and(|name| name.eq_ignore_ascii_case(TUNING_SECTION))
        })
        .ok_or_else(|| format!("The [{TUNING_SECTION}] section was not found in {TUNING_FILE}."))?;
    // The section runs to the next header of any kind. Derived from the header
    // list rather than tracked in one pass so that a file with the section
    // written twice — which ReShade will not produce but a hand-edited file
    // can — still yields an end that is after the start, instead of a reversed
    // range that would append the edit above the section it belongs to.
    let section_end = headers
        .iter()
        .copied()
        .find(|&i| i > section_start)
        .unwrap_or(lines.len());

    let mut remaining: Vec<&TuningEdit> = edits.iter().collect();

    for line in lines.iter_mut().take(section_end).skip(section_start + 1) {
        let trimmed = line.content.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq_pos) = line.content.find('=') else {
            continue;
        };
        let key_trimmed = line.content[..eq_pos].trim();
        if let Some(pos) = remaining
            .iter()
            .position(|edit| edit.key.eq_ignore_ascii_case(key_trimmed))
        {
            let edit = remaining.remove(pos);
            let key_part = &line.content[..eq_pos];
            line.content = format!("{key_part}={}", edit.value);
        }
    }

    if !remaining.is_empty() {
        // Every remaining edit was already validated against `FIELDS`, so this
        // key must resolve — `apply_client_tuning` never reaches this function
        // with an unvalidated edit.
        let mut appended = Vec::new();
        for edit in &remaining {
            let key = field_for(&edit.key)
                .map(|spec| spec.key)
                .unwrap_or(&edit.key);
            appended.push(format!("{key}={}", edit.value));
        }

        // A key appended right at end-of-file needs the preceding line to end
        // with a newline first, or it would land on the same physical line.
        if section_end > 0 && lines[section_end - 1].terminator.is_empty() {
            lines[section_end - 1].terminator = dominant_line_ending(&lines);
        }
        let terminator = dominant_line_ending(&lines);
        for (offset, content) in appended.into_iter().enumerate() {
            lines.insert(
                section_end + offset,
                Line {
                    content,
                    terminator: terminator.clone(),
                },
            );
        }
    }

    let mut out = String::new();
    for line in &lines {
        out.push_str(&line.content);
        out.push_str(&line.terminator);
    }
    Ok(out)
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Read-only: the current tuning values.
#[tauri::command]
pub fn read_client_tuning(client_dir: String) -> Result<TuningForm, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    let ini_path = tuning_file_path(&location.client_dir);
    let contents = std::fs::read_to_string(&ini_path).unwrap_or_default();
    Ok(read_form(&contents, &location.client_dir.to_string_lossy()))
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
    for edit in &edits {
        validate_edit(edit)?;
    }

    let root = crate::client_write::begin_write(&state, &client_dir).await?;

    tokio::task::spawn_blocking(move || {
        let ini_path = tuning_file_path(root.path());
        let contents = std::fs::read_to_string(&ini_path)
            .map_err(|e| format!("Could not read {TUNING_FILE}: {e}"))?;
        let updated = apply_edits(&contents, &edits)?;
        let outcome = crate::client_backup::edit_managed_file(
            &app,
            &root,
            TUNING_FILE,
            crate::client_write::ManagedKind::ReShadeConfig,
            updated.as_bytes(),
        )?;
        Ok(TuningApplyOutcome {
            changed: edits.into_iter().map(|edit| edit.key).collect(),
            backup_id: outcome.backup_id,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Where `ReShade.ini` is, for a validated client directory.
pub fn tuning_file_path(client_dir: &Path) -> std::path::PathBuf {
    client_dir.join(TUNING_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── field_for ───────────────────────────────────────────────────────

    #[test]
    fn field_for_is_case_insensitive() {
        let spec = field_for("neuraluplift").expect("should match NeuralUplift");
        assert_eq!(spec.key, "NeuralUplift");
    }

    #[test]
    fn field_for_unknown_key_is_none() {
        assert!(field_for("NotARealKey").is_none());
    }

    // ── slider_range ────────────────────────────────────────────────────

    #[test]
    fn slider_range_defaults_to_zero_one_with_no_value() {
        assert_eq!(slider_range(None), (0.0, 1.0));
    }

    #[test]
    fn slider_range_floors_a_negative_value_for_the_minimum() {
        let (min, max) = slider_range(Some(-0.3));
        assert_eq!(min, -1.0);
        assert_eq!(max, 1.0);
    }

    /// `NRLocalStructure=1.4` is a value pulled from a real user's install
    /// (see the module doc). The slider range must contain it.
    #[test]
    fn slider_range_contains_a_real_world_value_above_one() {
        let (min, max) = slider_range(Some(1.4));
        assert!(
            min <= 1.4 && 1.4 <= max,
            "range [{min}, {max}] must contain 1.4"
        );
        // Also pinned exactly: next whole 0.5 step above 1.4 is 1.5.
        assert_eq!((min, max), (0.0, 1.5));
    }

    #[test]
    fn slider_range_keeps_default_max_for_values_within_zero_one() {
        assert_eq!(slider_range(Some(0.5)), (0.0, 1.0));
    }

    // ── read_form ───────────────────────────────────────────────────────

    #[test]
    fn read_form_reports_values_verbatim_and_never_defaults() {
        let ini = "[RenoDX.DLSS5]\nNeuralUplift=1\nNRLocalStructure=1.4\nUnknownFutureKey=42\n";
        let form = read_form(ini, "C:/client");

        assert!(form.section_present);

        let uplift = form
            .fields
            .iter()
            .find(|f| f.key == "NeuralUplift")
            .expect("field present");
        assert_eq!(uplift.current.as_deref(), Some("1"));

        let structure = form
            .fields
            .iter()
            .find(|f| f.key == "NRLocalStructure")
            .expect("field present");
        assert_eq!(structure.current.as_deref(), Some("1.4"));
        let (min, max) = (
            structure.slider_min.expect("float has a slider range"),
            structure.slider_max.expect("float has a slider range"),
        );
        assert!(min <= 1.4 && 1.4 <= max);

        let intensity = form
            .fields
            .iter()
            .find(|f| f.key == "NRIntensity")
            .expect("field present");
        assert_eq!(intensity.current, None, "an absent key must read as None");

        assert_eq!(
            form.unknown,
            vec![("UnknownFutureKey".to_string(), "42".to_string())]
        );
    }

    #[test]
    fn read_form_reports_section_absent_without_inventing_anything() {
        let form = read_form("[GENERAL]\nFoo=1\n", "C:/client");
        assert!(!form.section_present);
        assert!(form.fields.iter().all(|f| f.current.is_none()));
        assert!(form.unknown.is_empty());
    }

    // ── validate_edit ───────────────────────────────────────────────────

    #[test]
    fn validate_edit_refuses_an_unknown_key() {
        let err = validate_edit(&TuningEdit {
            key: "NotARealKey".to_string(),
            value: "1".to_string(),
        })
        .expect_err("must refuse");
        assert!(err.contains("NotARealKey"));
    }

    #[test]
    fn validate_edit_refuses_a_toggle_that_is_not_zero_or_one() {
        let err = validate_edit(&TuningEdit {
            key: "NeuralUplift".to_string(),
            value: "2".to_string(),
        })
        .expect_err("must refuse");
        assert!(err.contains("NeuralUplift"));
    }

    #[test]
    fn validate_edit_accepts_a_valid_toggle() {
        validate_edit(&TuningEdit {
            key: "NeuralUplift".to_string(),
            value: "1".to_string(),
        })
        .expect("must accept");
    }

    #[test]
    fn validate_edit_refuses_a_choice_value_outside_its_own_options() {
        let err = validate_edit(&TuningEdit {
            key: "NRStyle".to_string(),
            value: "5".to_string(),
        })
        .expect_err("must refuse");
        assert!(err.contains("NRStyle"));
    }

    #[test]
    fn validate_edit_refuses_a_non_numeric_choice() {
        validate_edit(&TuningEdit {
            key: "NRStyle".to_string(),
            value: "abc".to_string(),
        })
        .expect_err("must refuse");
    }

    #[test]
    fn validate_edit_accepts_a_valid_choice() {
        validate_edit(&TuningEdit {
            key: "NRStyle".to_string(),
            value: "1".to_string(),
        })
        .expect("must accept");
    }

    #[test]
    fn validate_edit_refuses_an_unparseable_float() {
        let err = validate_edit(&TuningEdit {
            key: "NRIntensity".to_string(),
            value: "abc".to_string(),
        })
        .expect_err("must refuse");
        assert!(err.contains("NRIntensity"));
    }

    #[test]
    fn validate_edit_accepts_a_valid_float() {
        validate_edit(&TuningEdit {
            key: "NRIntensity".to_string(),
            value: "0.5".to_string(),
        })
        .expect("must accept");
    }

    #[test]
    fn validate_edit_refuses_an_unparseable_key_code() {
        let err = validate_edit(&TuningEdit {
            key: "NRToggleKey".to_string(),
            value: "12.5".to_string(),
        })
        .expect_err("must refuse");
        assert!(err.contains("NRToggleKey"));
    }

    #[test]
    fn validate_edit_accepts_a_valid_key_code() {
        validate_edit(&TuningEdit {
            key: "NRToggleKey".to_string(),
            value: "65".to_string(),
        })
        .expect("must accept");
    }

    // ── apply_edits ─────────────────────────────────────────────────────

    /// A fixture shaped like a real `ReShade.ini`: CRLF terminators, a giant
    /// ImGui docking value containing `=` and commas, a comment, a blank
    /// line, the target section with an already-unknown key, and another
    /// section after it. Only the one edited value may change.
    fn crlf_fixture() -> String {
        concat!(
            "[GENERAL]\r\n",
            "DockingData=Layout=A,B=2,C=3\r\n",
            "; a comment\r\n",
            "\r\n",
            "[RenoDX.DLSS5]\r\n",
            "NeuralUplift=1\r\n",
            "UnknownFutureKey=42\r\n",
            "[OtherSection]\r\n",
            "Foo=bar\r\n",
        )
        .to_string()
    }

    #[test]
    fn apply_edits_touches_only_the_edited_value_and_keeps_crlf() {
        let original = crlf_fixture();
        let edits = vec![
            TuningEdit {
                key: "NeuralUplift".to_string(),
                value: "0".to_string(),
            },
            TuningEdit {
                key: "NRIntensity".to_string(),
                value: "0.75".to_string(),
            },
        ];

        let updated = apply_edits(&original, &edits).expect("section exists");

        let expected = concat!(
            "[GENERAL]\r\n",
            "DockingData=Layout=A,B=2,C=3\r\n",
            "; a comment\r\n",
            "\r\n",
            "[RenoDX.DLSS5]\r\n",
            "NeuralUplift=0\r\n",
            "UnknownFutureKey=42\r\n",
            "NRIntensity=0.75\r\n",
            "[OtherSection]\r\n",
            "Foo=bar\r\n",
        );
        assert_eq!(updated, expected);
        assert!(!updated.contains("\r\r"), "must not double up terminators");

        // Every line must still be CRLF-terminated.
        for line in updated.split_inclusive('\n') {
            assert!(line.ends_with("\r\n"), "line {line:?} lost its CRLF");
        }
    }

    #[test]
    fn apply_edits_appends_missing_keys_with_canonical_spelling() {
        let original = "[RenoDX.DLSS5]\nneuraluplift=1\n";
        let edits = vec![TuningEdit {
            key: "EnableHooks".to_string(),
            value: "1".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("section exists");
        assert_eq!(updated, "[RenoDX.DLSS5]\nneuraluplift=1\nEnableHooks=1\n");
    }

    #[test]
    fn apply_edits_edits_an_existing_key_in_place_preserving_its_own_spelling() {
        let original = "[RenoDX.DLSS5]\nneuraluplift=1\n";
        let edits = vec![TuningEdit {
            key: "NeuralUplift".to_string(),
            value: "0".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("section exists");
        assert_eq!(
            updated, "[RenoDX.DLSS5]\nneuraluplift=0\n",
            "the on-disk key spelling must survive, only the value changes"
        );
    }

    #[test]
    fn apply_edits_adds_a_missing_trailing_newline_before_appending() {
        // No trailing newline on the last line of the file.
        let original = "[RenoDX.DLSS5]\nNeuralUplift=1";
        let edits = vec![TuningEdit {
            key: "EnableHooks".to_string(),
            value: "1".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("section exists");
        assert_eq!(updated, "[RenoDX.DLSS5]\nNeuralUplift=1\nEnableHooks=1\n");
    }

    #[test]
    fn apply_edits_refuses_when_the_section_is_absent() {
        let err =
            apply_edits("[GENERAL]\nFoo=1\n", &[]).expect_err("must refuse an absent section");
        assert!(err.contains(TUNING_SECTION));
    }

    #[test]
    fn apply_edits_section_lookup_is_case_insensitive() {
        let original = "[renodx.dlss5]\nNeuralUplift=1\n";
        let edits = vec![TuningEdit {
            key: "NeuralUplift".to_string(),
            value: "0".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("section exists");
        assert_eq!(updated, "[renodx.dlss5]\nNeuralUplift=0\n");
    }

    /// A hand-edited file can name the section twice. The edit has to land
    /// inside a section, not above one.
    #[test]
    fn apply_edits_survives_the_section_appearing_twice() {
        let original =
            "[RenoDX.DLSS5]\nNRStyle=0\n\n[GENERAL]\nFoo=1\n\n[RenoDX.DLSS5]\nNRPreset=2\n";
        let edits = vec![TuningEdit {
            key: "NRIntensity".to_string(),
            value: "0.50".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("section exists");
        assert_eq!(
            updated,
            "[RenoDX.DLSS5]\nNRStyle=0\n\nNRIntensity=0.50\n[GENERAL]\nFoo=1\n\n\
             [RenoDX.DLSS5]\nNRPreset=2\n",
            "the appended key must sit inside a [RenoDX.DLSS5] block"
        );
    }
}
