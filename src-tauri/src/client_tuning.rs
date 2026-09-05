//! Tuning the RenoDX add-ons: the `ReShade.ini` sections they own, and which of
//! those sections is live.
//!
//! # Two paths, three sections
//!
//! There are two mutually exclusive RenoDX integrations for DLSS Neural
//! Rendering on ESO, and which one is installed decides which INI sections are
//! live configuration and which are leftovers:
//!
//! * **Direct** — `renodx-dlss.addon64` hooks `nvngx_dlssnr.dll` itself. It
//!   writes `[RENODX-DLSS]` and `[RENODX-DLSS-preset1]`.
//! * **Feed** — the older two-piece setup, `renodx-dlss5.addon64` alongside
//!   `dlss5-feed.addon64`. It writes `[RenoDX.DLSS5]`.
//!
//! This module used to read `[RenoDX.DLSS5]` and nothing else, and to present
//! it as *the* tuning. On a real install running the direct path — which is the
//! arrangement that actually works, and the one Kalpa's own load-order guidance
//! points at — `renodx-dlss5.addon64` sits on disk renamed aside, and
//! `[RenoDX.DLSS5]` is a **fossil**: whatever the user last had under a path
//! they no longer run. The panel showed `NeuralUplift=0` out of it, and both a
//! user and a debugging session concluded Neural Rendering was switched off
//! while it was in fact running fine on the other path. A stale value presented
//! as current is worse than no value at all, because it is *actionable*.
//!
//! So every section carries a [`TuningProvenance`]. A fossil is never hidden
//! and never deleted — the user may well switch paths back, and silently
//! dropping their saved settings would be the worse failure — it is
//! **labelled**, and it is not writable while it is a fossil.
//!
//! # Why one section is editable and two are not
//!
//! `[RenoDX.DLSS5]`'s keys have verified labels, enum wordings and controls in
//! [`FIELDS`] (see "Where the labels come from" below). The direct path's 30
//! keys have nothing of the sort: `renodx-dlss.addon64` is closed source, and
//! no label, range or enum meaning for `DirectNeuralRenderingEncoding=2` or
//! `StreamlineOutputPreset=2` has been recovered. So they are exposed
//! **read-only, as key and raw value**, with no invented label.
//!
//! This asymmetry is deliberate and must stay. Inferring a meaning from a key
//! name and then attaching it to a *writable* control is how a working install
//! gets corrupted: the user trusts the label, moves the control, and Kalpa
//! writes a number whose real effect nobody here knows. A key name and its
//! value, plainly presented, is the honest answer, and it is the same rule the
//! rest of this module already follows for float ranges and key codes. Do not
//! "finish" the direct-path sections by hand-mapping them to typed fields
//! unless someone has actually read the labels out of the binary, exactly as
//! was done for `[RenoDX.DLSS5]`.
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
//! [`FIELDS`] describes `[RenoDX.DLSS5]` and only `[RenoDX.DLSS5]`. The direct
//! path's sections have no such table because no such reading was done for
//! them, and until it is they stay read-only.
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

/// The one INI section Kalpa can write, as ReShade writes its name.
///
/// This constant is the *writable* section, not "the tuning section" — see
/// [`SECTIONS`] for the full set Kalpa reads. Every write helper in this module
/// is scoped to this name on purpose; see [`apply_edits`].
pub const TUNING_SECTION: &str = "RenoDX.DLSS5";

/// The file the sections live in.
pub const TUNING_FILE: &str = "ReShade.ini";

/// The add-on file that owns [`TUNING_SECTION`].
pub const FEED_NR_ADDON: &str = "renodx-dlss5.addon64";

/// The add-on file that owns the direct path's sections.
pub const DIRECT_NR_ADDON: &str = "renodx-dlss.addon64";

/// The sentence the panel shows beside the apply button, and after an apply.
///
/// Two facts, and each has already cost somebody an afternoon. ReShade reads
/// these values when the add-on initialises, so a change lands at the *next*
/// launch and not now — which is why the status afterwards is "Applies at next
/// launch" and never "Saved". And ReShade rewrites the whole of `ReShade.ini`
/// from memory when it exits, so an edit made while ESO is open is thrown away
/// on close, silently, with no error anywhere to explain it. The apply refuses
/// while the game or its launcher is running for precisely that reason; this is
/// the copy that makes the refusal read as protection rather than as an
/// arbitrary lock.
pub const APPLY_TIMING_NOTE: &str =
    "Applies at next launch, and only with ESO closed: ReShade rewrites ReShade.ini from \
     memory when the game exits, so anything changed while it is running is discarded.";

/// Which of the two mutually exclusive RenoDX integrations a section belongs
/// to. See the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenoDxPath {
    /// `renodx-dlss.addon64`, hooking `nvngx_dlssnr.dll` itself.
    Direct,
    /// `renodx-dlss5.addon64` + `dlss5-feed.addon64`.
    Feed,
}

/// Which path is live, whether a section is in force, and the one function that
/// decides — all owned by `client_stack` and used here, not restated.
///
/// This module briefly had its own `TuningActivePath` and its own
/// `TuningProvenance`, justified at the time by `client_stack` having an
/// `ActivePath` of its own with three variants and a slightly different job.
/// That justification does not survive contact with the panel they both feed:
/// two enums answering "which path is live?" by two rules, in one crate, is the
/// exact bug class this model exists to eliminate — and the three-variant
/// version was *wrong*, folding "both are loaded" into `Direct` and "I could
/// not look" into `Neither`. The merged [`ActivePath`] carries all five states,
/// and both of the extra ones are load-bearing here: this module **writes**, so
/// "both are loaded" and "I could not look" must never be silently folded into
/// a verdict. See [`writable_section_guard`].
pub use crate::client_stack::{detect_active_path, ActivePath, TuningProvenance};

/// A raw `key=value` pair, exactly as the file has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TuningEntry {
    pub key: String,
    pub value: String,
}

/// A section Kalpa knows about: which add-on writes it, and whether Kalpa has
/// a verified field table for it.
pub struct SectionSpec {
    /// Canonical spelling of the section name.
    pub name: &'static str,
    /// True when a section name matching `name` case-insensitively is not the
    /// only match: `RENODX-DLSS-preset1` is one of a family, and a user running
    /// preset 2 or 3 would have `RENODX-DLSS-preset2`. Matching by prefix keeps
    /// those visible instead of silently dropping them.
    pub prefix_match: bool,
    pub path: RenoDxPath,
    /// The add-on file whose presence makes this section live.
    pub owner: &'static str,
    /// True only for [`TUNING_SECTION`]. See the module doc for why the direct
    /// path's sections are read-only, and do not flip this without doing the
    /// binary string-table reading that earned [`FIELDS`].
    pub editable: bool,
}

/// Every section Kalpa reads out of `ReShade.ini`, in the order the panel
/// should show them.
///
/// The direct path comes first because it is the arrangement that works, and a
/// user with both will be looking at it. Only [`TUNING_SECTION`] is writable.
pub const SECTIONS: &[SectionSpec] = &[
    SectionSpec {
        name: "RENODX-DLSS",
        prefix_match: false,
        path: RenoDxPath::Direct,
        owner: DIRECT_NR_ADDON,
        editable: false,
    },
    SectionSpec {
        name: "RENODX-DLSS-preset",
        prefix_match: true,
        path: RenoDxPath::Direct,
        owner: DIRECT_NR_ADDON,
        editable: false,
    },
    SectionSpec {
        name: TUNING_SECTION,
        prefix_match: false,
        path: RenoDxPath::Feed,
        owner: FEED_NR_ADDON,
        editable: true,
    },
];

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

/// One section of `ReShade.ini`, with everything the panel needs to say where
/// its values came from and whether they are in force.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TuningSection {
    /// The section name as it appears in the file when present, otherwise the
    /// canonical spelling from [`SECTIONS`].
    pub section: String,
    pub path: RenoDxPath,
    /// The add-on file that writes this section.
    pub owner: String,
    /// True when `ReShade.ini` has this section at all. False means the add-on
    /// has never run here, and the panel should say so rather than offer to
    /// write a section from nothing.
    pub present: bool,
    pub provenance: TuningProvenance,
    /// True only when Kalpa has a verified field table for the section *and*
    /// its owning add-on is live. A fossil is never writable — see
    /// [`writable_section_guard`].
    pub writable: bool,
    /// Why the section is not writable, in the panel's own words. Empty when
    /// it is. Never blank when `writable` is false.
    pub read_only_reason: String,
    /// Typed, verified controls, in [`FIELDS`] order — the add-on's own order.
    /// Empty for a section with no field table, which is every section but
    /// [`TUNING_SECTION`].
    pub fields: Vec<TuningField>,
    /// Every key in the section that no [`FieldSpec`] describes, verbatim and
    /// in file order. For the direct path's sections that is *all* of them, by
    /// design. For `[RenoDX.DLSS5]` an entry here is a newer add-on build, and
    /// silently dropping it on the next write would delete a setting the user
    /// relies on.
    pub entries: Vec<TuningEntry>,
}

/// The whole panel's data.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TuningForm {
    pub client_dir: String,
    /// Which RenoDX integration is installed here. Decides every section's
    /// [`TuningProvenance`].
    pub active_path: ActivePath,
    /// Plain-English observations behind `active_path`, one per line, naming
    /// the files that were and were not found. The panel shows these rather
    /// than asking the user to take the verdict on trust.
    pub path_evidence: Vec<String>,
    /// In [`SECTIONS`] order.
    pub sections: Vec<TuningSection>,
    /// [`APPLY_TIMING_NOTE`], so the panel does not keep its own copy of copy
    /// that describes this module's write behaviour.
    pub apply_note: String,
}

impl TuningForm {
    /// The one section Kalpa may write, if it is present and live.
    pub fn writable_section(&self) -> Option<&TuningSection> {
        self.sections.iter().find(|s| s.writable)
    }
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
    /// [`APPLY_TIMING_NOTE`]. The panel shows it as the outcome's own status
    /// line, because "changed 3 settings" on its own reads as "done now".
    pub note: String,
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
];

// `EnableHooks` is deliberately NOT in this table. It only does anything in a
// Streamline title, and ESO ships no `sl.*.dll` modules at all — RenoDX itself
// logs `EnableHooks=2 (Streamline modules left unpatched)` against this game.
// Offering a switch whose own help text has to say "applies to other games"
// is the general-settings-screen failure mode: every setting Kalpa can edit
// exists because a finding points at it. The key still appears, read-only,
// under the unknown-settings list if the user's file carries it.

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

/// A UTF-8 BOM, if the file opens with one.
///
/// `str::trim` does not remove `U+FEFF` — it is a format character, not
/// whitespace — so a `ReShade.ini` saved by an editor that writes a BOM has a
/// first line of `\u{feff}[GENERAL]`, which fails every `starts_with('[')`
/// test. That is only ever the *first* line, but the section Kalpa needs can
/// be it, and a missed header reads as "the section is not there" rather than
/// as a parse problem. Stripped for matching only: `apply_edits` rebuilds the
/// file from the original line contents, so the BOM is written back untouched.
fn without_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Is this trimmed line a `[Section]` header, and if so, its name (trimmed)?
fn section_header(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(|name| name.trim())
}

/// Which [`SECTIONS`] entry, if any, owns a section name out of the file.
///
/// Exact matches are tried before prefix matches, because `RENODX-DLSS` is
/// itself a prefix of `RENODX-DLSS-preset1` and the wrong order would fold the
/// preset blocks into the base section.
///
/// An index rather than a reference because `SECTIONS` is a `const`: every use
/// of it is a fresh inlined copy, so `std::ptr::eq` on references into it is
/// not a reliable identity test.
pub fn spec_index_for_section(name: &str) -> Option<usize> {
    SECTIONS
        .iter()
        .position(|spec| !spec.prefix_match && spec.name.eq_ignore_ascii_case(name))
        .or_else(|| {
            SECTIONS.iter().position(|spec| {
                spec.prefix_match
                    && name.len() >= spec.name.len()
                    && name[..spec.name.len()].eq_ignore_ascii_case(spec.name)
            })
        })
}

/// Which [`SectionSpec`], if any, owns a section name out of the file.
pub fn spec_for_section(name: &str) -> Option<&'static SectionSpec> {
    spec_index_for_section(name).map(|index| &SECTIONS[index])
}

/// Every `key=value` pair Kalpa found in one logical section, plus the spelling
/// of the header as the file actually has it.
#[derive(Default)]
struct RawSection {
    header: Option<String>,
    /// Last occurrence of a key wins, as it would when ReShade itself reads the
    /// file; order of first appearance is kept.
    entries: Vec<(String, String)>,
}

impl RawSection {
    fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Collect `ReShade.ini` into one [`RawSection`] per [`SectionSpec`], plus
/// whatever `[ADDON]` holds.
///
/// A section written twice — which ReShade does not do but a hand-edited file
/// can — merges into one, last-value-wins, which is what ReShade's own reader
/// would resolve to. The prefix-matched preset family also merges: all of
/// `RENODX-DLSS-preset1..3` land in one bucket, because Kalpa shows them
/// read-only and has no way to know which preset the add-on will select.
fn collect_sections(reshade_ini: &str) -> (Vec<RawSection>, RawSection) {
    let mut sections: Vec<RawSection> = SECTIONS.iter().map(|_| RawSection::default()).collect();
    let mut addon = RawSection::default();
    // Index into `sections`, or `usize::MAX` for `[ADDON]`, or `None`.
    let mut current: Option<usize> = None;
    let mut in_addon = false;

    for raw in reshade_ini.split_inclusive('\n') {
        let (content, _) = split_terminator(raw);
        let trimmed = without_bom(content).trim();
        if let Some(name) = section_header(trimmed) {
            in_addon = name.eq_ignore_ascii_case("ADDON");
            current = spec_index_for_section(name).inspect(|&index| {
                if sections[index].header.is_none() {
                    sections[index].header = Some(name.to_string());
                }
            });
            continue;
        }
        let target = match (current, in_addon) {
            (Some(index), _) => &mut sections[index],
            (None, true) => &mut addon,
            (None, false) => continue,
        };
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim().to_string();
        if let Some(existing) = target
            .entries
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
        {
            existing.1 = value;
        } else {
            target.entries.push((key, value));
        }
    }

    (sections, addon)
}

/// The add-on file names ReShade has been told not to load, lower-cased.
///
/// A file can be sitting right there and still be inert because it is named in
/// `[ADDON] DisabledAddons`. Kalpa reads that out of the same `ReShade.ini` it
/// is already holding rather than asking another module, so that the liveness
/// verdict and the values it labels come from one read of one file.
pub fn disabled_addons(reshade_ini: &str) -> Vec<String> {
    let (_, addon) = collect_sections(reshade_ini);
    addon
        .get("DisabledAddons")
        .unwrap_or_default()
        .split(',')
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect()
}

// The path detection this module uses lives in `client_stack` — see the
// re-export at the top of this file. It is deliberately not restated here: the
// rule it applies ("live only when a file named *exactly* like the add-on is
// present, and not in `DisabledAddons`") is the one this module argued for and
// won, and it now decides liveness for the whole panel rather than for one
// half of it.

/// What a section's provenance is, given the active path.
///
/// Two of the five paths need saying out loud. [`ActivePath::Both`] makes
/// *every* section live — both add-ons are loaded, so both paths' saved
/// settings are in force, and Kalpa picking a winner would be Kalpa labelling
/// live configuration a fossil. [`ActivePath::Unknown`] makes every section
/// unknown, which is not-writable: the folder could not be read, and a module
/// that writes does not guess in the direction of writing.
fn provenance_for(spec: &SectionSpec, active: ActivePath) -> TuningProvenance {
    match (active, spec.path) {
        (ActivePath::Unknown, _) => TuningProvenance::Unknown,
        (ActivePath::Both, _) => TuningProvenance::Live,
        (ActivePath::Direct, RenoDxPath::Direct) => TuningProvenance::Live,
        (ActivePath::Feed, RenoDxPath::Feed) => TuningProvenance::Live,
        _ => TuningProvenance::Fossil,
    }
}

/// The single rule deciding whether Kalpa may write a section, and the sentence
/// it owes the user when it may not.
///
/// Three conditions, all required, and every one of them is a refusal this
/// module has learned the hard way:
///
/// 1. Kalpa has a verified field table for the section. Without one it cannot
///    vouch for what a key means, and writing a value whose effect is unknown
///    into a working install is the worst outcome available here.
/// 2. The owning add-on is live. Writing to a fossil would be Kalpa editing
///    configuration for something that is not running, then reporting success —
///    the same class of lie as showing a fossil as current tuning, which is the
///    bug this whole model exists to fix.
/// 3. The section already exists in the file. Writing one from nothing would be
///    Kalpa inventing configuration for an add-on that has never run.
///
/// `Ok(())` means writable. `Err` is the reason, phrased for the panel.
pub fn writable_section_guard(
    spec: &SectionSpec,
    provenance: TuningProvenance,
    present: bool,
) -> Result<(), String> {
    if !spec.editable {
        return Err(format!(
            "[{}] is written by {}, which is closed source and whose settings Kalpa has \
             not been able to verify. Kalpa shows them exactly as they are on disk and \
             will not change them.",
            spec.name, spec.owner
        ));
    }
    match provenance {
        TuningProvenance::Fossil => {
            return Err(format!(
                "[{}] belongs to {}, which is not loaded in this client folder. These are \
                 the settings you last used with it, kept for if you switch back — they \
                 are not in force, so Kalpa will not write to them.",
                spec.name, spec.owner
            ));
        }
        TuningProvenance::Unknown => {
            return Err(format!(
                "Kalpa could not read the client folder, so it cannot tell whether {} is \
                 loaded. It will not write settings it cannot vouch for.",
                spec.owner
            ));
        }
        TuningProvenance::Live => {}
    }
    if !present {
        return Err(format!(
            "{TUNING_FILE} has no [{}] section, so {} has never run here. Kalpa will not \
             write a section from nothing.",
            spec.name, spec.owner
        ));
    }
    Ok(())
}

/// Build the panel's data from the text of `ReShade.ini`.
///
/// `active` and `evidence` come from [`detect_active_path`]; this function is
/// pure over them so that the whole live/fossil split is testable from a string
/// fixture. Callers that cannot list the client directory pass
/// [`ActivePath::Unknown`], which makes everything read-only rather than
/// guessing.
///
/// Values are reported exactly as they appear. Nothing is normalised, defaulted
/// or clamped: a value Kalpa does not understand still belongs to the user.
pub fn read_form(
    reshade_ini: &str,
    client_dir: &str,
    active: ActivePath,
    evidence: Vec<String>,
) -> TuningForm {
    let (raw_sections, _) = collect_sections(reshade_ini);

    let sections = SECTIONS
        .iter()
        .zip(raw_sections.iter())
        .map(|(spec, raw)| build_section(spec, raw, active))
        .collect();

    TuningForm {
        client_dir: client_dir.to_string(),
        active_path: active,
        path_evidence: evidence,
        sections,
        apply_note: APPLY_TIMING_NOTE.to_string(),
    }
}

/// One section of the form: its provenance, its typed fields if it has a table,
/// and every key the table does not describe.
fn build_section(spec: &SectionSpec, raw: &RawSection, active: ActivePath) -> TuningSection {
    let present = raw.header.is_some();
    let provenance = provenance_for(spec, active);
    let guard = writable_section_guard(spec, provenance, present);

    // Only the section with a verified field table gets typed controls. The
    // direct path's sections deliberately have none — see the module doc.
    let fields = if spec.editable {
        FIELDS
            .iter()
            .map(|field| build_field(field, raw.get(field.key).map(str::to_string)))
            .collect()
    } else {
        Vec::new()
    };

    let entries = raw
        .entries
        .iter()
        .filter(|(key, _)| !spec.editable || field_for(key).is_none())
        .map(|(key, value)| TuningEntry {
            key: key.clone(),
            value: value.clone(),
        })
        .collect();

    TuningSection {
        section: raw.header.clone().unwrap_or_else(|| spec.name.to_string()),
        path: spec.path,
        owner: spec.owner.to_string(),
        present,
        provenance,
        writable: guard.is_ok(),
        read_only_reason: guard.err().unwrap_or_default(),
        fields,
        entries,
    }
}

/// One typed control: its static definition plus whatever the file holds.
fn build_field(spec: &FieldSpec, current: Option<String>) -> TuningField {
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
///
/// [`FIELDS`] holds only `[RenoDX.DLSS5]` keys, so this is also the guard that
/// keeps the direct path's undocumented keys out of the write path: a UI that
/// somehow offered `DirectNeuralRenderingIntensity` as editable would be
/// refused here, before any file is opened. That is not an accident of the
/// table's contents — [`fields_and_read_only_sections_stay_disjoint`] pins it.
pub fn validate_edit(edit: &TuningEdit) -> Result<(), String> {
    let spec = field_for(&edit.key).ok_or_else(|| {
        format!(
            "{} is not a setting Kalpa can vouch for, so it will not write it. Settings Kalpa \
             shows as read-only stay read-only.",
            edit.key
        )
    })?;
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
            // `"NaN"`, `"inf"` and `"-inf"` all parse as `f64`, so a bare
            // `parse` would let Kalpa write one of them into `ReShade.ini`.
            // The add-on reads these back with a C++ float parse and would
            // then run a non-finite strength through its own maths — and the
            // user has no way to see what went in, because the value Kalpa
            // shows them is the string it wrote.
            let parsed: f64 = edit
                .value
                .trim()
                .parse()
                .map_err(|_| format!("{} must be a number.", spec.key))?;
            if !parsed.is_finite() {
                return Err(format!("{} must be a finite number.", spec.key));
            }
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
/// **This function writes [`TUNING_SECTION`] and nothing else, ever.** It takes
/// no section argument, and it must not grow one. `[RENODX-DLSS]` and
/// `[RENODX-DLSS-preset*]` are read-only for reasons the module doc sets out at
/// length, and the surest way to keep them read-only is for the code that can
/// write to have no way to name them. Everything outside `[RenoDX.DLSS5]` —
/// including those two — is in the "must survive byte for byte" set below.
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
///
/// Liveness is *not* checked here, because this function has no view of the
/// client directory. [`apply_client_tuning`] checks it through
/// [`writable_section_guard`] before calling this, and that ordering is the
/// only thing standing between a user and Kalpa editing a parked add-on's
/// settings while reporting success.
pub fn apply_edits(reshade_ini: &str, edits: &[TuningEdit]) -> Result<String, String> {
    // Re-validated here rather than trusted from the caller. `apply_edits` is
    // `pub`, the cost is a string comparison per edit, and the failure it
    // prevents — an unvetted key landing in a working `ReShade.ini` — is not
    // one worth economising on.
    for edit in edits {
        validate_edit(edit)?;
    }

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
        .filter(|(_, line)| section_header(without_bom(&line.content).trim()).is_some())
        .map(|(i, _)| i)
        .collect();
    let section_start = *headers
        .iter()
        .find(|&&i| {
            section_header(without_bom(&lines[i].content).trim())
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

    // A key named twice in one edit list has no meaningful answer once every
    // occurrence is rewritten: the first edit would claim all the lines and the
    // second would be appended below them, which is the value ReShade would
    // then read. The panel sends one edit per field and cannot produce this,
    // so refusing costs nothing and keeps the rule below unambiguous.
    for (index, edit) in edits.iter().enumerate() {
        if edits[..index]
            .iter()
            .any(|earlier| earlier.key.eq_ignore_ascii_case(&edit.key))
        {
            return Err(format!("{} was given two values in one change.", edit.key));
        }
    }

    // Every occurrence of the key inside the section is rewritten, not just
    // the first. A section naming a key twice is not something ReShade writes,
    // but a hand-edited file has it, and ReShade resolves it to the *last*
    // occurrence — which is also what `read_form` reports, so the panel shows
    // the last value. Editing only the first would leave the value the user
    // was looking at unchanged and report success: the edit would silently do
    // nothing. Rewriting all of them makes the file agree with itself, and is
    // correct whichever occurrence the reader happens to take.
    let mut applied = vec![false; edits.len()];

    for line in lines.iter_mut().take(section_end).skip(section_start + 1) {
        let trimmed = line.content.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq_pos) = line.content.find('=') else {
            continue;
        };
        let key_trimmed = line.content[..eq_pos].trim();
        if let Some(pos) = edits
            .iter()
            .position(|edit| edit.key.eq_ignore_ascii_case(key_trimmed))
        {
            let key_part = &line.content[..eq_pos];
            line.content = format!("{key_part}={}", edits[pos].value);
            applied[pos] = true;
        }
    }

    let remaining: Vec<&TuningEdit> = edits
        .iter()
        .enumerate()
        .filter(|(index, _)| !applied[*index])
        .map(|(_, edit)| edit)
        .collect();

    if !remaining.is_empty() {
        // Every remaining edit was validated against `FIELDS` at the top of
        // this function, so this key resolves; the fallback is unreachable and
        // kept only so the lookup needs no panic.
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

/// The file names directly inside a client directory, or `None` when it cannot
/// be listed.
///
/// `None` and "listed, but empty" are deliberately different: an unreadable
/// folder means Kalpa knows nothing about which add-on is loaded, and that has
/// to become [`ActivePath::Unknown`] rather than [`ActivePath::Neither`], which
/// would label live settings as fossils.
fn list_file_names(dir: &Path) -> Option<Vec<String>> {
    let entries = std::fs::read_dir(dir).ok()?;
    Some(
        entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect(),
    )
}

/// Build the form for a real client directory: detect the path from the folder
/// and `ReShade.ini`'s own `DisabledAddons`, then read every section against it.
pub fn read_form_for_dir(client_dir: &Path, reshade_ini: &str) -> TuningForm {
    let disabled = disabled_addons(reshade_ini);
    let (active, evidence) = match list_file_names(client_dir) {
        Some(names) => detect_active_path(&names, &disabled),
        None => (
            ActivePath::Unknown,
            vec![format!(
                "Kalpa could not read {}, so it cannot tell which RenoDX add-on is loaded.",
                client_dir.display()
            )],
        ),
    };
    read_form(reshade_ini, &client_dir.to_string_lossy(), active, evidence)
}

/// Read-only: the current tuning values, with each section's provenance.
#[tauri::command(async)]
pub fn read_client_tuning(client_dir: String) -> Result<TuningForm, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    let ini_path = tuning_file_path(&location.client_dir);
    let contents = std::fs::read_to_string(&ini_path).unwrap_or_default();
    Ok(read_form_for_dir(&location.client_dir, &contents))
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
        crate::client_backup::run_managed_transaction(&app, &root, |transaction| {
            let ini_path = tuning_file_path(transaction.client_root());
            let contents = std::fs::read_to_string(&ini_path)
                .map_err(|e| format!("Could not read {TUNING_FILE}: {e}"))?;

            // The liveness guard, and the reason this read happens on the write
            // side rather than trusting whatever the panel last saw. The panel's
            // copy of the form can be minutes old; the add-on could have been
            // parked in between. Provenance is re-derived from the folder and the
            // file as they are *now*, and a fossil section is refused with the same
            // sentence the panel shows beside it.
            let form = read_form_for_dir(transaction.client_root(), &contents);
            if form.writable_section().is_none() {
                return Err(form
                    .sections
                    .iter()
                    .find(|section| section.section.eq_ignore_ascii_case(TUNING_SECTION))
                    .map(|section| section.read_only_reason.clone())
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| {
                        format!("Kalpa will not write [{TUNING_SECTION}] in this client folder.")
                    }));
            }

            let updated = apply_edits(&contents, &edits)?;
            let outcome = transaction.edit_file(
                TUNING_FILE,
                crate::client_write::ManagedKind::ReShadeConfig,
                updated.as_bytes(),
            )?;
            Ok(TuningApplyOutcome {
                changed: edits.into_iter().map(|edit| edit.key).collect(),
                backup_id: outcome.backup_id,
                note: APPLY_TIMING_NOTE.to_string(),
            })
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

    /// Read a form with the feed path live, which is what the pre-provenance
    /// tests assumed without ever saying so.
    fn feed_form(ini: &str) -> TuningForm {
        read_form(ini, "C:/client", ActivePath::Feed, Vec::new())
    }

    /// The one writable section, by name rather than by index.
    fn dlss5(form: &TuningForm) -> &TuningSection {
        section_named(form, TUNING_SECTION)
    }

    fn section_named<'a>(form: &'a TuningForm, name: &str) -> &'a TuningSection {
        form.sections
            .iter()
            .find(|section| section.section.eq_ignore_ascii_case(name))
            .unwrap_or_else(|| panic!("no section {name} in the form"))
    }

    #[test]
    fn read_form_reports_values_verbatim_and_never_defaults() {
        let ini = "[RenoDX.DLSS5]\nNeuralUplift=1\nNRLocalStructure=1.4\nUnknownFutureKey=42\n";
        let form = feed_form(ini);
        let section = dlss5(&form);

        assert!(section.present);

        let uplift = section
            .fields
            .iter()
            .find(|f| f.key == "NeuralUplift")
            .expect("field present");
        assert_eq!(uplift.current.as_deref(), Some("1"));

        let structure = section
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

        let intensity = section
            .fields
            .iter()
            .find(|f| f.key == "NRIntensity")
            .expect("field present");
        assert_eq!(intensity.current, None, "an absent key must read as None");

        assert_eq!(
            section.entries,
            vec![TuningEntry {
                key: "UnknownFutureKey".to_string(),
                value: "42".to_string(),
            }]
        );
    }

    #[test]
    fn read_form_reports_section_absent_without_inventing_anything() {
        let form = feed_form("[GENERAL]\nFoo=1\n");
        let section = dlss5(&form);
        assert!(!section.present);
        assert!(section.fields.iter().all(|f| f.current.is_none()));
        assert!(section.entries.is_empty());
        assert!(
            !section.writable,
            "a section that has never been written is not one Kalpa may create"
        );
    }

    // ── paths and provenance ────────────────────────────────────────────

    /// The real install's state: all three sections in one file, with the feed
    /// add-on parked as `.off`. Trimmed to the keys the assertions name, but
    /// verbatim in spelling, ordering and section headers.
    fn three_section_fixture() -> String {
        concat!(
            "[GENERAL]\r\n",
            "PresetPath=\r\n",
            "\r\n",
            "[ADDON]\r\n",
            "DisabledAddons=\r\n",
            "LoadFromDllMain=renodx-dlss.addon64\r\n",
            "\r\n",
            "[RENODX-DLSS]\r\n",
            "DirectNeuralRenderingDiffuseWhiteNits=101\r\n",
            "DirectNeuralRenderingEncoding=2\r\n",
            "DLSSQualityMode=0\r\n",
            "StreamlinePeakNits=1000\r\n",
            "\r\n",
            "[RENODX-DLSS-preset1]\r\n",
            "DirectNeuralRenderingIntensity=1\r\n",
            "DirectNeuralRenderingStyle=0\r\n",
            "\r\n",
            "[RenoDX.DLSS5]\r\n",
            "NeuralUplift=0\r\n",
            "NRIntensity=1.05\r\n",
            "NRLocalStructure=1.62\r\n",
            "\r\n",
            "[OtherSection]\r\n",
            "Foo=bar\r\n",
        )
        .to_string()
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    /// The bug this whole model exists for: on the user's real machine the
    /// direct add-on is loaded, the feed add-on is renamed to `.off`, and
    /// `NeuralUplift=0` out of the parked add-on's section was being shown as
    /// live tuning.
    #[test]
    fn the_real_install_splits_live_from_fossil() {
        let files = names(&[
            "ReShade.ini",
            "renodx-dlss.addon64",
            "renodx-dlss5.addon64.off",
            "nvngx_dlssnr.dll",
        ]);
        let ini = three_section_fixture();
        let (active, evidence) = detect_active_path(&files, &disabled_addons(&ini));
        assert_eq!(active, ActivePath::Direct);

        let form = read_form(&ini, "C:/client", active, evidence);

        assert_eq!(
            section_named(&form, "RENODX-DLSS").provenance,
            TuningProvenance::Live
        );
        assert_eq!(
            section_named(&form, "RENODX-DLSS-preset1").provenance,
            TuningProvenance::Live
        );

        let fossil = dlss5(&form);
        assert_eq!(
            fossil.provenance,
            TuningProvenance::Fossil,
            "renodx-dlss5.addon64 is parked, so its section is not live tuning"
        );
        assert!(!fossil.writable, "and a fossil is never writable");
        assert!(
            fossil.read_only_reason.contains(FEED_NR_ADDON),
            "the reason must name the add-on: {}",
            fossil.read_only_reason
        );

        // The fossil's values are still reported. Hiding them would lose
        // settings the user gets back the moment they switch paths.
        let uplift = fossil
            .fields
            .iter()
            .find(|f| f.key == "NeuralUplift")
            .expect("NeuralUplift is a known field");
        assert_eq!(uplift.current.as_deref(), Some("0"));

        // And the evidence names the file that decided it.
        assert!(
            form.path_evidence
                .iter()
                .any(|line| line.contains("renodx-dlss5.addon64.off")),
            "{:?}",
            form.path_evidence
        );
    }

    /// The mirror image: with the feed add-on restored and the direct one
    /// parked, the same file yields the opposite split.
    #[test]
    fn the_feed_path_makes_the_direct_sections_the_fossils() {
        let files = names(&[
            "ReShade.ini",
            "renodx-dlss.addon64.kalpa-off",
            "renodx-dlss5.addon64",
            "dlss5-feed.addon64",
        ]);
        let ini = three_section_fixture();
        let (active, evidence) = detect_active_path(&files, &disabled_addons(&ini));
        assert_eq!(active, ActivePath::Feed);

        let form = read_form(&ini, "C:/client", active, evidence);
        assert_eq!(
            section_named(&form, "RENODX-DLSS").provenance,
            TuningProvenance::Fossil
        );
        assert_eq!(
            section_named(&form, "RENODX-DLSS-preset1").provenance,
            TuningProvenance::Fossil
        );

        let live = dlss5(&form);
        assert_eq!(live.provenance, TuningProvenance::Live);
        assert!(live.writable, "the verified section is writable when live");
        assert!(live.read_only_reason.is_empty());
        assert_eq!(
            form.writable_section().map(|s| s.section.as_str()),
            Some(TUNING_SECTION)
        );
    }

    /// `RENODX-DLSS` is a prefix of `RENODX-DLSS-preset1`. Matching by prefix
    /// first would fold the presets into the base section and lose a
    /// distinction the add-on itself draws.
    #[test]
    fn the_preset_section_does_not_swallow_the_base_section() {
        let form = read_form(
            &three_section_fixture(),
            "C:/client",
            ActivePath::Direct,
            Vec::new(),
        );
        let base = section_named(&form, "RENODX-DLSS");
        assert!(
            base.entries.iter().any(|e| e.key == "DLSSQualityMode"),
            "{:?}",
            base.entries
        );
        assert!(
            !base
                .entries
                .iter()
                .any(|e| e.key == "DirectNeuralRenderingStyle"),
            "a preset key must not land in the base section: {:?}",
            base.entries
        );

        let preset = section_named(&form, "RENODX-DLSS-preset1");
        assert!(preset
            .entries
            .iter()
            .any(|e| e.key == "DirectNeuralRenderingStyle"));
    }

    /// Presets 2 and 3 exist in the add-on's own UI, so a user who moves off
    /// preset 1 must not have their values vanish from the panel.
    #[test]
    fn every_preset_block_is_read_not_just_preset_one() {
        let ini = "[RENODX-DLSS-preset3]\nDirectNeuralRenderingIntensity=2\n";
        let form = read_form(ini, "C:/client", ActivePath::Direct, Vec::new());
        let preset = section_named(&form, "RENODX-DLSS-preset3");
        assert!(preset.present);
        assert_eq!(preset.entries.len(), 1);
    }

    /// The direct path's keys are undocumented, so they appear as raw key and
    /// value with no invented label and no control. See the module doc — this
    /// asymmetry with `[RenoDX.DLSS5]` is the point, not an omission, and a
    /// later "tidy-up" that types these fields would be the regression.
    #[test]
    fn the_direct_sections_are_raw_key_values_and_never_typed_controls() {
        let form = read_form(
            &three_section_fixture(),
            "C:/client",
            ActivePath::Direct,
            Vec::new(),
        );
        for name in ["RENODX-DLSS", "RENODX-DLSS-preset1"] {
            let section = section_named(&form, name);
            assert!(
                section.fields.is_empty(),
                "{name} must offer no typed controls"
            );
            assert!(!section.entries.is_empty(), "{name} must still show values");
            assert!(
                !section.writable,
                "{name} is not a section Kalpa can vouch for, live or not"
            );
            assert!(
                !section.read_only_reason.is_empty(),
                "{name} must say why it is read-only"
            );
        }
    }

    /// The write path's outermost guard, stated as a property rather than as a
    /// list of examples: no key Kalpa offers as editable may also be a key of a
    /// section it refuses to write.
    #[test]
    fn fields_and_read_only_sections_stay_disjoint() {
        let read_only_keys = [
            "DirectNeuralRenderingDiffuseWhiteNits",
            "DirectNeuralRenderingEncoding",
            "DirectNeuralRenderingHookPoint",
            "DirectNeuralRenderingUiCorrectionMode",
            "DirectNeuralRenderingIntensity",
            "DirectNeuralRenderingStyle",
            "DirectNeuralRenderingAutoMask",
            "DLSSQualityMode",
            "FrameGenerationPresentationPath",
            "OptionsMode",
            "StreamlineOutputPreset",
            "StreamlinePeakNits",
        ];
        for key in read_only_keys {
            assert!(
                field_for(key).is_none(),
                "{key} belongs to a section Kalpa cannot vouch for and must never be editable"
            );
            let edit = TuningEdit {
                key: key.to_string(),
                value: "0".to_string(),
            };
            assert!(
                validate_edit(&edit).is_err(),
                "writing {key} must be refused"
            );
            assert!(
                apply_edits(&three_section_fixture(), &[edit]).is_err(),
                "and refused again at the file level, not only in the UI"
            );
        }
    }

    /// A section present in the file but whose add-on is nowhere is still the
    /// user's data. It reads as a fossil, not as absent.
    #[test]
    fn nothing_installed_makes_every_section_a_fossil() {
        let ini = three_section_fixture();
        let (active, evidence) = detect_active_path(&names(&["ReShade.ini"]), &[]);
        assert_eq!(active, ActivePath::Neither);
        let form = read_form(&ini, "C:/client", active, evidence);
        assert!(form
            .sections
            .iter()
            .all(|s| s.provenance == TuningProvenance::Fossil));
        assert!(form.sections.iter().all(|s| !s.writable));
        assert!(
            dlss5(&form).present,
            "the values are still reported, just not as live"
        );
    }

    /// An add-on can be sitting right there and still be inert.
    #[test]
    fn disabled_addons_beats_the_file_being_present() {
        let ini = three_section_fixture().replace(
            "DisabledAddons=\r\n",
            "DisabledAddons=renodx-dlss.addon64\r\n",
        );
        assert_eq!(disabled_addons(&ini), vec!["renodx-dlss.addon64"]);

        let files = names(&["renodx-dlss.addon64", "renodx-dlss5.addon64"]);
        let (active, evidence) = detect_active_path(&files, &disabled_addons(&ini));
        assert_eq!(active, ActivePath::Feed);
        assert!(
            evidence.iter().any(|line| line.contains("DisabledAddons")),
            "{evidence:?}"
        );
    }

    #[test]
    fn an_empty_disabled_addons_list_disables_nothing() {
        assert!(disabled_addons(&three_section_fixture()).is_empty());
    }

    /// Both add-ons live is a state nobody designed for, but it is not a state
    /// in which Kalpa gets to pick a winner and call the other one dead.
    #[test]
    fn both_addons_present_makes_both_paths_live() {
        let files = names(&["renodx-dlss.addon64", "renodx-dlss5.addon64"]);
        let (active, _) = detect_active_path(&files, &[]);
        assert_eq!(active, ActivePath::Both);
        let form = read_form(&three_section_fixture(), "C:/client", active, Vec::new());
        assert!(form
            .sections
            .iter()
            .all(|s| s.provenance == TuningProvenance::Live));
    }

    /// An unreadable client folder must not be reported as "nothing
    /// installed", which would label live settings as fossils. It reads as
    /// unknown, and unknown never writes.
    #[test]
    fn an_unknown_path_is_read_only_and_not_a_fossil() {
        let form = read_form(
            &three_section_fixture(),
            "C:/client",
            ActivePath::Unknown,
            Vec::new(),
        );
        assert!(form
            .sections
            .iter()
            .all(|s| s.provenance == TuningProvenance::Unknown));
        assert!(form.sections.iter().all(|s| !s.writable));
        assert!(form.writable_section().is_none());
    }

    /// `.off` is not one of Kalpa's own parking suffixes, and the liveness rule
    /// must not depend on knowing it: only the exact file name counts, because
    /// only the exact file name is what ReShade loads.
    #[test]
    fn any_renaming_aside_parks_an_addon_whatever_the_suffix() {
        for suffix in [".off", ".kalpa-off", ".disabled", ".bak", ".old"] {
            let files = names(&[&format!("renodx-dlss.addon64{suffix}")]);
            let (active, evidence) = detect_active_path(&files, &[]);
            assert_eq!(
                active,
                ActivePath::Neither,
                "renodx-dlss.addon64{suffix} must not count as loaded"
            );
            assert!(
                evidence
                    .iter()
                    .any(|line| line.contains(suffix) && line.contains("renamed aside")),
                "the user is told it is parked, not that it is missing: {evidence:?}"
            );
        }
    }

    /// "Parked" and "never installed" read differently to a user deciding what
    /// to do next.
    #[test]
    fn a_missing_addon_is_not_described_as_parked() {
        let (_, evidence) = detect_active_path(&names(&["ReShade.ini"]), &[]);
        assert!(
            evidence.iter().all(|line| !line.contains("renamed aside")),
            "{evidence:?}"
        );
        assert!(evidence
            .iter()
            .any(|line| line.contains("not in the client folder")));
    }

    /// File names on Windows are case-insensitive, and ReShade's own
    /// `DisabledAddons` is written however the user typed it.
    #[test]
    fn liveness_is_case_insensitive_on_both_sides() {
        let files = names(&["RenoDX-DLSS.Addon64"]);
        let (active, _) = detect_active_path(&files, &[]);
        assert_eq!(active, ActivePath::Direct);

        let (active, _) = detect_active_path(&files, &["RENODX-DLSS.ADDON64".to_string()]);
        assert_eq!(
            active,
            ActivePath::Neither,
            "`disabled_addons` lower-cases its output, but `client_stack` passes \
             ReShade.ini's list through as written, so the comparison itself has to be \
             case-insensitive: a caller passing raw mixed case must not silently fail to \
             disable anything"
        );
    }

    // ── the write guard ─────────────────────────────────────────────────

    #[test]
    fn writable_section_guard_refuses_a_section_with_no_verified_table() {
        let spec = SECTIONS
            .iter()
            .find(|spec| !spec.editable)
            .expect("there are read-only sections");
        let err = writable_section_guard(spec, TuningProvenance::Live, true)
            .expect_err("live is not enough; the table has to exist");
        assert!(err.contains(spec.owner), "{err}");
    }

    #[test]
    fn writable_section_guard_refuses_a_fossil_and_says_the_settings_are_kept() {
        let spec = SECTIONS
            .iter()
            .find(|spec| spec.editable)
            .expect("one section is editable");
        let err = writable_section_guard(spec, TuningProvenance::Fossil, true)
            .expect_err("a parked add-on's settings are not in force");
        assert!(err.contains("not in force"), "{err}");
        assert!(
            err.contains("switch back"),
            "the user must be told the values are kept, not lost: {err}"
        );
    }

    #[test]
    fn writable_section_guard_refuses_an_absent_section() {
        let spec = SECTIONS
            .iter()
            .find(|spec| spec.editable)
            .expect("one section is editable");
        writable_section_guard(spec, TuningProvenance::Live, false)
            .expect_err("Kalpa does not create a section from nothing");
    }

    #[test]
    fn writable_section_guard_accepts_only_the_verified_live_present_section() {
        let spec = SECTIONS
            .iter()
            .find(|spec| spec.editable)
            .expect("one section is editable");
        writable_section_guard(spec, TuningProvenance::Live, true).expect("all three conditions");
    }

    /// The copy shown for a write has to say both things: it lands at the next
    /// launch, and it has to be made with ESO closed because ReShade rewrites
    /// the whole file from memory when the game exits.
    #[test]
    fn the_apply_note_says_when_the_change_lands_and_when_it_may_be_made() {
        let note = APPLY_TIMING_NOTE.to_ascii_lowercase();
        assert!(note.contains("next launch"), "{APPLY_TIMING_NOTE}");
        assert!(note.contains("eso closed"), "{APPLY_TIMING_NOTE}");
        assert!(note.contains("rewrites reshade.ini"), "{APPLY_TIMING_NOTE}");
        assert_eq!(
            feed_form("[RenoDX.DLSS5]\nNeuralUplift=1\n").apply_note,
            APPLY_TIMING_NOTE,
            "the panel must not have to carry its own copy of it"
        );
    }

    // ── validate_edit ───────────────────────────────────────────────────

    /// `EnableHooks` only does anything in a Streamline title and ESO ships no
    /// `sl.*.dll` modules at all, so Kalpa offering a switch for it was
    /// offering a control that provably cannot do anything here. The key still
    /// belongs to the user, so it has to keep showing up — read-only, among
    /// the section's raw entries — rather than disappear from a file that
    /// contains it.
    #[test]
    fn enable_hooks_is_shown_but_not_offered_as_a_control() {
        assert!(
            field_for("EnableHooks").is_none(),
            "EnableHooks must not be an editable field"
        );

        let form = feed_form("[RenoDX.DLSS5]\nNeuralUplift=1\nEnableHooks=2\n");
        let section = dlss5(&form);
        assert!(
            !section.fields.iter().any(|f| f.key == "EnableHooks"),
            "it must not appear among the controls"
        );
        assert!(
            section
                .entries
                .iter()
                .any(|entry| entry.key == "EnableHooks" && entry.value == "2"),
            "it must still be reported, with the user's own value: {:?}",
            section.entries
        );

        let edit = TuningEdit {
            key: "EnableHooks".to_string(),
            value: "1".to_string(),
        };
        assert!(
            validate_edit(&edit).is_err(),
            "and writing it must be refused"
        );
    }

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
            key: "nruicorrection".to_string(),
            value: "1".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("section exists");
        assert_eq!(
            updated, "[RenoDX.DLSS5]\nneuraluplift=1\nNRUICorrection=1\n",
            "an appended key takes the table's spelling, not the caller's"
        );
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
            key: "NRUICorrection".to_string(),
            value: "1".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("section exists");
        assert_eq!(
            updated,
            "[RenoDX.DLSS5]\nNeuralUplift=1\nNRUICorrection=1\n"
        );
    }

    /// ReShade resolves a duplicated key to the last occurrence, and
    /// `read_form` reports that same one — so the panel shows the last value.
    /// Editing only the first would leave what the user was looking at
    /// unchanged while reporting success.
    #[test]
    fn apply_edits_rewrites_every_occurrence_of_a_duplicated_key() {
        let original = "[RenoDX.DLSS5]\nNRStyle=1\nNeuralUplift=1\nNRStyle=0\n";
        let edits = vec![TuningEdit {
            key: "NRStyle".to_string(),
            value: "1".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("section exists");
        assert_eq!(
            updated, "[RenoDX.DLSS5]\nNRStyle=1\nNeuralUplift=1\nNRStyle=1\n",
            "both occurrences must carry the new value, not just the first"
        );
    }

    /// The value the panel reads back must be the value that was written, for
    /// a duplicated key as much as any other.
    #[test]
    fn a_duplicated_key_reads_back_as_what_was_written() {
        let original = "[RenoDX.DLSS5]\nNRStyle=0\nNRStyle=1\n";
        let edits = vec![TuningEdit {
            key: "NRStyle".to_string(),
            value: "0".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("section exists");
        let form = feed_form(&updated);
        let field = dlss5(&form)
            .fields
            .iter()
            .find(|f| f.key == "NRStyle")
            .expect("NRStyle is a known field")
            .clone();
        assert_eq!(field.current.as_deref(), Some("0"));
    }

    /// `str::trim` does not strip `U+FEFF`, so a BOM on line 1 used to make the
    /// section unfindable — reported to the user as "the add-on has never run".
    #[test]
    fn a_utf8_bom_does_not_hide_the_section() {
        let original = "\u{feff}[RenoDX.DLSS5]\nNeuralUplift=1\n";
        let edits = vec![TuningEdit {
            key: "NeuralUplift".to_string(),
            value: "0".to_string(),
        }];

        let updated = apply_edits(original, &edits).expect("the BOM must not hide the section");
        assert_eq!(
            updated, "\u{feff}[RenoDX.DLSS5]\nNeuralUplift=0\n",
            "the BOM is stripped for matching only and must be written back"
        );
        assert!(dlss5(&feed_form(original)).present);
    }

    #[test]
    fn validate_edit_refuses_a_non_finite_float() {
        for value in ["NaN", "inf", "-inf", "infinity"] {
            let edit = TuningEdit {
                key: "NRIntensity".to_string(),
                value: value.to_string(),
            };
            let err =
                validate_edit(&edit).expect_err("a non-finite float must never reach the ini file");
            assert!(
                err.contains("finite"),
                "{value} was refused with an unhelpful message: {err}"
            );
        }
    }

    #[test]
    fn apply_edits_refuses_a_key_named_twice_in_one_change() {
        let edits = vec![
            TuningEdit {
                key: "NRStyle".to_string(),
                value: "0".to_string(),
            },
            TuningEdit {
                key: "nrstyle".to_string(),
                value: "1".to_string(),
            },
        ];
        let err = apply_edits("[RenoDX.DLSS5]\nNRStyle=0\n", &edits)
            .expect_err("two values for one key has no answer");
        assert!(err.contains("two values"), "{err}");
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
