//! The graphics-mod **stack** in an ESO client directory.
//!
//! `client_health` answers "are these three DLLs current?". That was the wrong
//! question. A DLSS 5 Neural Rendering setup is not three files, it is a
//! pipeline whose layers only work if every one of them agrees with the ones
//! around it, and the interesting failures live *between* layers — exactly what
//! a per-file report cannot see.
//!
//! # There are two paths, and they are mutually exclusive
//!
//! This module asserted the eight-layer feed pipeline as *the* shape until
//! 2026-09-03. It is one of two, and the one the primary user does **not** run.
//! Presenting the other path's layers as universally required is how the panel
//! came to report "Everything agrees" over a stack that could not work: an
//! empty motion-vector slot was read as an absence rather than as the correct
//! answer for the path in use. [`ActivePath`] is therefore computed first, and
//! every slot re-frames around it.
//!
//! ```text
//!   DIRECT — the current shape, and what the primary user runs
//!     injector (ReShade dxgi.dll)
//!       -> addon    (renodx-dlss.addon64)          hooks nvngx_dlssnr itself
//!         -> runtimes (nvngx_dlssnr.dll, nvngx_dlss.dll)
//!     no feed add-on · no host process · no motion-vector provider
//!     no preset technique — an EMPTY `Techniques=` is correct here
//!     tuning is [RENODX-DLSS] / [RENODX-DLSS-preset1], not [RenoDX.DLSS5]
//!
//!   FEED — the older two-piece shape, still the fallback
//!     injector (ReShade dxgi.dll)
//!       -> addons  (renodx-dlss5.addon64, dlss5-feed.addon64)
//!         -> companion (dlss5-feed-host64.exe + .cfg)
//!           -> runtimes (nvngx_dlssnr.dll, nvngx_dlss.dll)
//!             -> shaders (MartysMods_LAUNCHPAD.fx, DLSS5_Feed.fx)
//!               -> preset (ordered technique list — provider ABOVE the feed)
//!                 -> tuning ([RenoDX.DLSS5] in ReShade.ini)
//! ```
//!
//! The technique order is the sharpest cross-layer failure, and it belongs to
//! the feed path alone: Launchpad produces the motion vectors and normals the
//! feed consumes, so a preset listing `DLSS5_Feed` before
//! `MartysMods_Launchpad` leaves the whole thing running and silently wrong.
//! Nothing about either file is defective. On the direct path there is no feed
//! technique at all and the question does not arise — which is why the ordering
//! and provider checks are gated on the path rather than on file presence.
//!
//! # Tuning follows the path too
//!
//! The same reasoning reaches the last row, and it took a second pass to get
//! there. Until 2026-09-04 this module read `[RenoDX.DLSS5]` unconditionally —
//! the *feed* path's block — so on the primary user's direct install the panel
//! reported a parked add-on's saved settings and said they were "history, not
//! this install's live tuning", while ~30 live keys in `[RENODX-DLSS]` and
//! `[RENODX-DLSS-preset1]` went unread. Every word of that sentence was true
//! and the row as a whole was misleading: it implied there was no live tuning.
//! Worse, `client_tuning`'s form *did* read all three sections, so the two
//! panels disagreed about whether this install had live tuning at all.
//!
//! So the sections are now read by `client_tuning::read_form` — the tuning
//! panel's own reader, over `client_tuning::SECTIONS` — and the live one is
//! selected by [`ActivePath`]. Every block found is carried, live and fossil
//! alike, because the feed add-ons cannot be refetched and a user's parked
//! settings are their only copy. See [`TuningBlock`].
//!
//! # Present, active, needed
//!
//! Three questions, not one. A file can be **present** (on disk), **active**
//! (live and not switched off in `ReShade.ini`), and **needed** (by the path
//! this user is actually on), and every combination of the three occurs in real
//! installs. [`SlotNeed`] is the third axis: a slot that is
//! [`SlotNeed::NotOnThisPath`] is *correctly* empty and must be said so
//! affirmatively, and a slot that is [`SlotNeed::InstalledUnused`] is shown as
//! exactly that, **with the reason to keep it** — several pieces of this stack
//! are link-only or Discord-only, so Kalpa cannot refetch them and advice to
//! delete them would be advice Kalpa cannot undo.
//!
//! # Load order is not file presence
//!
//! An add-on can be present, listed nowhere in `DisabledAddons`, visible in the
//! ReShade overlay, and still do nothing: ESO creates a **D3D12** device first,
//! so ReShade loads add-ons during `D3D12CreateDevice` and hooks `d3d11.dll`
//! afterwards. An add-on that installs early DLSS hooks has to be named in
//! `[ADDON] LoadFromDllMain` or those hooks are not guaranteed. RenoDX names
//! its own fix in `ReShade.log`; `stack-addon-not-in-dllmain` is Kalpa noticing
//! it without needing the log.
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
// The section table, and the reader that turns `ReShade.ini` into sections, are
// `client_tuning`'s. Reading them from here rather than keeping a second copy
// is the whole point: the two panels answer "which tuning is live?" out of one
// function, so they cannot drift apart the way they had by 2026-09-03.
use crate::client_tuning::{read_form, RenoDxPath, TuningSection, SECTIONS};
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

/// Whose hand parked a file.
///
/// Re-enabling a user-parked file is **not the same act** as re-enabling a
/// Kalpa-parked one. `.kalpa-off` is a name only Kalpa writes, so a file
/// carrying it is one Kalpa moved and can move back with no further question.
/// `.off` is the user's own switch: Kalpa did not choose the name, did not
/// record what it displaced, and cannot claim to know why it was switched off.
/// Collapsing the two would have Kalpa offering to undo a decision it never
/// made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParkedBy {
    /// Kalpa parked it, under [`PARKED_SUFFIX`].
    Kalpa,
    /// The user switched it off themselves, under one of
    /// [`USER_PARK_SUFFIXES`].
    User,
}

/// A file that has been renamed aside so the stack does not load it.
///
/// Not the same thing as a [`PreservedOriginal`]. A backup suffix marks a
/// *displaced original* — the file that was there before something replaced it.
/// A park suffix marks a **live file that was switched off**: the same bytes
/// that would run again the moment the name goes back. `nvngx_dlss.dll.disabled-bak`
/// is the stock DLL a restore depends on; `renodx-dlss5.addon64.off` is a mod
/// the user turned off last week. Reading one as the other would have Kalpa
/// offering to "restore" a mod over a stock file, or to delete a stock file as
/// a stale mod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParkedFile {
    /// Name on disk, ending in [`PARKED_SUFFIX`] or one of
    /// [`USER_PARK_SUFFIXES`].
    pub file_name: String,
    /// The live name it goes back to.
    pub restores: String,
    pub size_bytes: u64,
    /// True when something already occupies the name it would restore, which
    /// means re-enabling has to displace it rather than just rename.
    pub target_present: bool,
    /// Which suffix it actually carries, verbatim, so the UI can show the user
    /// their own convention rather than Kalpa's word for it.
    pub suffix: String,
    pub parked_by: ParkedBy,
}

/// Which of the two mutually exclusive Neural Rendering setups is **live**.
///
/// Computed from which add-on is actually loaded, never from what is merely on
/// disk: a `.off` file is not live, and neither is a name sitting in
/// `ReShade.ini`'s `DisabledAddons`. [`detect_active_path`] is the only place
/// that answers this question, for `client_stack` and `client_tuning` alike —
/// see its doc for the rule, and see below for why the two modules may not
/// answer it separately.
///
/// This is a *framing* answer, not a verdict. `Neither` is entirely ordinary:
/// a plain ReShade install with no Neural Rendering at all is the common case,
/// and nothing about it is wrong.
///
/// # Five variants, and why none of them may be folded away
///
/// An earlier three-variant version of this enum tested Direct first and
/// returned it, so an install with **both** add-ons live reported `Direct` and
/// every feed-path check was then gated away by a path that was only half the
/// truth: real feed breakage went unreported, which is the same "gated into
/// silence" failure this whole model exists to end. And it had nowhere to put
/// "I could not look", so an unreadable client folder read as `Neither` —
/// which `client_tuning` would have taken as licence to call live settings
/// fossils, and to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivePath {
    /// `renodx-dlss.addon64` hooks `nvngx_dlssnr.dll` itself. No feed add-on,
    /// no host process, no motion-vector provider, and an empty `Techniques=`
    /// is correct.
    Direct,
    /// `renodx-dlss5.addon64` + `dlss5-feed.addon64` + `dlss5-feed-host64.exe`,
    /// with the `DLSS5_Feed` technique enabled and a motion-vector provider
    /// ordered above it.
    Feed,
    /// Both add-ons are loaded. ReShade will load both, Kalpa does not guess
    /// which one wins, and **both paths' checks apply**: the feed pipeline is
    /// live here, so its technique order, its provider and its preset are all
    /// still Kalpa's business. Picking a winner would silence exactly the
    /// findings this install most needs.
    Both,
    /// No Neural Rendering add-on is loaded. Not a fault.
    Neither,
    /// The client folder could not be listed, so nothing about liveness is
    /// known. Distinct from `Neither` on purpose and load-bearing on the write
    /// side: `client_tuning` treats every section as
    /// [`TuningProvenance::Unknown`] here and refuses to write, because a
    /// module that writes must never guess in the direction of writing. On the
    /// read side it makes each slot say "Kalpa could not look" rather than
    /// claim a slot is correctly empty.
    Unknown,
}

/// One row of the panel, named to match the frontend's `Slot` union exactly.
///
/// Deliberately a mirror rather than a re-derivation: the need axis is a
/// backend answer (only the backend knows which path is live and what is on
/// disk), but it has to land on a row the frontend already renders. A second
/// vocabulary here would need a mapping table, and a mapping table is where the
/// toolbar and the Settings list drifted apart the last time this repo grew
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StackSlot {
    Reshade,
    Addons,
    Nr,
    Sr,
    Shaders,
    Motion,
    Preset,
    Tuning,
}

/// Whether this slot is *wanted* on the live path.
///
/// The third axis, and the one whose absence produced "Everything agrees" over
/// a broken stack. Present/absent alone cannot tell a deliberate empty slot
/// from a missing piece, so the panel rendered a correct direct-path install as
/// six-eighths empty and said nothing was wrong with any of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotNeed {
    /// The live path needs this. Empty here is a real gap.
    Required,
    /// Correctly absent. Say so affirmatively — "nothing here" is a different
    /// and wrong sentence.
    NotOnThisPath,
    /// Present, and not used on this path. Never a fault, and never advice to
    /// remove it: see [`SlotStatus::keep_because`].
    InstalledUnused,
    /// Kalpa could not read the client folder, so it does not know which path
    /// is live and cannot say whether this slot is wanted. Distinct from
    /// [`SlotNeed::NotOnThisPath`] on purpose: that one *asserts* the slot is
    /// correctly empty, and asserting it from an unreadable folder would be a
    /// guess wearing a verdict's clothes. Only reachable under
    /// [`ActivePath::Unknown`].
    Unknown,
}

/// What one slot means on the live path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SlotStatus {
    pub slot: StackSlot,
    pub need: SlotNeed,
    /// A complete sentence, shown verbatim. Written affirmatively for
    /// [`SlotNeed::NotOnThisPath`] because the whole point is that the user
    /// reads a deliberate state rather than a hole.
    pub reason: String,
    /// Why to **keep** something that is installed but unused, when there is a
    /// reason beyond taste.
    ///
    /// Kalpa refetches none of this stack's optional pieces: iMMERSE LaunchPad
    /// is link-only by licence, and the `renodx-dlss5` / `dlss5-feed` add-ons
    /// are distributed through a Discord with no stable URL. A user's existing
    /// copy is therefore their only fallback if the live path stops working,
    /// and telling them to delete it would be advice Kalpa cannot undo. `None`
    /// when nothing needs saying.
    pub keep_because: Option<String>,
}

/// Whether an INI section's contents are configuration **in force**, or
/// leftovers from a path that is no longer running.
///
/// `[RenoDX.DLSS5]` is written by `renodx-dlss5.addon64` — the **feed** path's
/// add-on. On a direct-path install that add-on is parked, so the section is a
/// fossil: real values, saved by a real add-on, describing a configuration that
/// is not running. Presented as live tuning it misled both the user and the
/// agent diagnosing this install, which is why provenance is part of the data
/// rather than something the UI guesses.
///
/// # Provenance is not presence
///
/// There is deliberately no `Absent` variant. "The section is not in the file"
/// and "the section is here but I cannot tell whether it is in force" are
/// different facts, and squeezing both onto one enum makes them impossible to
/// state together. Presence is carried by a separate flag on each side —
/// [`ClientStack::tuning_section`] being `Some`, and
/// [`crate::client_tuning::TuningSection::present`] — and a section that is
/// absent still has a provenance, because whether its owning add-on is loaded
/// is a fact about the folder rather than about the file.
///
/// Defined here, and re-exported by `client_tuning`, because two enums spelling
/// one verdict is how the panel came to have two answers to the same question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningProvenance {
    /// The add-on that writes this section is present and enabled.
    Live,
    /// The owning add-on is absent, parked (renamed aside — `.off`,
    /// `.kalpa-off`, anything), or listed in `DisabledAddons`. The values are
    /// the user's and are kept and shown, but they are not in force.
    Fossil,
    /// Liveness could not be determined — see [`ActivePath::Unknown`]. Treated
    /// as not-writable.
    Unknown,
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
    /// The technique *identifiers* this provider declares, plus the spellings
    /// older presets use for the same effect.
    ///
    /// **These are identifiers, never `ui_label`s.** A ReShade preset's
    /// `Techniques=` line records the identifier a shader declares after the
    /// `technique` keyword; the `ui_label` annotation beside it is display text
    /// for the in-game overlay and never reaches the preset file. Upstream prose
    /// quotes the labels, because those are what a user reading the overlay
    /// sees — so DLSS5-Feeder's provider table says "enable `LUMENITE: Kernel
    /// 2.0`" for a technique whose identifier is `Lumenite_Kernel`. Matching a
    /// label against a preset can therefore never fire, which is exactly what
    /// this table did for VORT and both LumeniteFX entries: any user on those
    /// providers had a correctly configured stack reported as
    /// `stack-mv-provider-missing`. Each entry below was read from the
    /// shader source, not from upstream documentation.
    ///
    /// [`MvProviderKind::SharedTexture`] has no entry: it is a convention rather
    /// than a named effect, and is resolved by reading the shader sources.
    fn technique_names(self) -> &'static [&'static str] {
        match self {
            MvProviderKind::SharedTexture => &[],
            // `MartysMods_Launchpad` is the identifier (label: "iMMERSE:
            // Launchpad (enable and move to the top!)"), and is what the
            // primary user's preset contains. `Launchpad` is the bare spelling
            // DLSS5-Feeder's table uses, kept as an alias.
            MvProviderKind::Launchpad => &["MartysMods_Launchpad", "Launchpad"],
            // `vort_Motion` is the *file* name; the technique it declares is
            // `vort_MotionEffects`.
            MvProviderKind::Vort => &["vort_MotionEffects"],
            MvProviderKind::LumeniteKernel => &["Lumenite_Kernel"],
            MvProviderKind::LumeniteQuantMotion => &["Lumenite_QuantMotion"],
        }
    }

    /// The provider's `ui_label`, i.e. what the ReShade overlay calls it.
    ///
    /// Kept separate from [`MvProviderKind::technique_names`] on purpose: a
    /// finding that tells the user to enable something has to name what they
    /// will actually read in the overlay, while matching has to use what the
    /// preset actually stores. Conflating the two is what broke matching.
    fn technique_label(self) -> Option<&'static str> {
        match self {
            MvProviderKind::SharedTexture => None,
            MvProviderKind::Launchpad => Some("iMMERSE: Launchpad"),
            MvProviderKind::Vort => Some("vort_MotionEffects"),
            MvProviderKind::LumeniteKernel => Some("LUMENITE: Kernel 2.0"),
            MvProviderKind::LumeniteQuantMotion => Some("LUMENITE: QuantMotion"),
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

/// One tunable read out of a RenoDX block in `ReShade.ini`.
///
/// Which block is [`TuningBlock::section`]'s business, and there is more than
/// one: `renodx-dlss5.addon64` writes `[RenoDX.DLSS5]`, `renodx-dlss.addon64`
/// writes `[RENODX-DLSS]` and a `[RENODX-DLSS-preset*]` family. This module
/// used to read the first of those and nothing else, so on the primary user's
/// direct-path install a dead `NeuralUplift=0` was the only tuning the panel
/// knew about while ~30 live keys sat unread — the fossil bug, in the one place
/// it was hardest to see. Every known block is now read, by
/// `client_tuning`'s own reader; see [`ClientStack::tuning_blocks`].
///
/// Values are verbatim. Keys are the file's own spelling, except for the keys
/// `client_tuning` has a verified field table for, which come back in that
/// table's canonical spelling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TuningValue {
    pub key: String,
    pub value: String,
}

/// One RenoDX tuning section that is actually in `ReShade.ini`, labelled with
/// whether it is in force.
///
/// Every block Kalpa knows about and finds is carried, live and fossil alike.
/// That is deliberate and it is a data-loss rule, not a presentation one: the
/// feed add-ons are Discord-distributed with no stable URL, so a user's parked
/// `[RenoDX.DLSS5]` is the only copy of the settings they would come back to if
/// they ever switch paths. Kalpa showing only the live block would quietly turn
/// the panel into a claim that those settings no longer exist.
///
/// [`ClientStack::tuning`], [`ClientStack::tuning_section`] and
/// [`ClientStack::tuning_owner`] are the *headline* block picked out of this
/// list for the one-line rail; the list is what lets the UI also say "and this
/// one belongs to a parked add-on".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TuningBlock {
    /// The section name as `ReShade.ini` spells it, e.g. `RENODX-DLSS-preset1`.
    pub section: String,
    /// The add-on file that writes this section, so the UI can name the thing
    /// that is or is not loaded rather than only the block it left behind.
    pub owner: String,
    pub provenance: TuningProvenance,
    pub values: Vec<TuningValue>,
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
    /// Files **Kalpa** parked when the stack was switched off — `.kalpa-off`
    /// and nothing else.
    ///
    /// Kept separate from [`ClientStack::user_parked`] on purpose, and the
    /// separation is a safety boundary rather than a presentation one. The
    /// toggle planner reads this list and plans an unpark for every entry, and
    /// the panel's power state reads "on" from it being empty. Both are correct
    /// statements about *Kalpa's* work and would become false the moment a file
    /// the user renamed by hand appeared here: a working install would report
    /// itself partly switched off, and "switch back on" would offer to undo a
    /// decision Kalpa never made.
    pub parked: Vec<ParkedFile>,
    /// Files the **user** switched off themselves, under one of
    /// [`USER_PARK_SUFFIXES`].
    ///
    /// Read-only knowledge. Recognising these is what stops
    /// `renodx-dlss5.addon64.off` reading as "not installed" — which is exactly
    /// why the whole DLSS 5 findings branch used to be skipped on the primary
    /// user's real machine — but Kalpa still parks only as [`PARKED_SUFFIX`]
    /// and still removes only files carrying it.
    pub user_parked: Vec<ParkedFile>,
    /// True when **Kalpa** has parked the injector, i.e. Kalpa put ESO back to
    /// stock and it loads none of this.
    ///
    /// Deliberately not fed by [`ClientStack::user_parked`]. A user with a live
    /// `dxgi.dll` and a `renodx-dlss5.addon64.off` beside it has a running
    /// stack, not a switched-off one, and the `stack-disabled` finding this
    /// flag drives says "Kalpa has parked … and put the game's own files back",
    /// which would be untrue of a rename Kalpa did not perform. A user-parked
    /// injector with nothing live under the name still reports through
    /// `stack-no-injector`, where the copy fits.
    pub is_disabled: bool,
    pub shaders: ShaderTree,
    pub preset: Option<PresetInfo>,
    /// The **headline** block's values: the live one when there is a live one.
    ///
    /// This field used to be `[RenoDX.DLSS5]` unconditionally, which is the
    /// *feed* path's section. On the primary user's direct-path install that
    /// made the panel report a parked add-on's saved settings as this install's
    /// tuning while `[RENODX-DLSS]`'s live keys went unread — and made the
    /// stack panel and the tuning panel disagree about whether live tuning
    /// existed at all. The selection now follows [`ClientStack::active_path`],
    /// by the same rule and the same code as `client_tuning`; see
    /// [`ClientStack::tuning_blocks`].
    pub tuning: Vec<TuningValue>,
    /// The ini section [`ClientStack::tuning`] came from, so the UI does not
    /// hardcode a section name that belongs to only one of the two paths.
    /// `None` when there is no known tuning section in the file at all — this
    /// field, not a provenance variant, is where absence is recorded. See
    /// [`TuningProvenance`].
    pub tuning_section: Option<String>,
    /// Whether [`ClientStack::tuning_section`] is in force, answered whether or
    /// not any section is present: it is a fact about which add-on is loaded,
    /// and the panel needs it to say "no section, and the add-on that would
    /// write one is parked" rather than only half of that. With no section
    /// present it describes the live path's own add-on — see
    /// [`tuning_owner_without_a_section`].
    pub tuning_owner: TuningProvenance,
    /// Every known tuning section present in `ReShade.ini`, in
    /// `client_tuning::SECTIONS` order, each carrying its own provenance.
    ///
    /// The headline three fields above are one entry from this list; this is
    /// the rest, and it is how a fossil stays visible instead of being dropped
    /// the moment a live block exists. See [`TuningBlock`].
    pub tuning_blocks: Vec<TuningBlock>,
    /// Addon file names ReShade has been told not to load.
    pub disabled_addons: Vec<String>,
    /// `[ADDON] LoadFromDllMain` — the add-ons ReShade loads early enough for
    /// their hooks to land. Empty is the default and is fine for most add-ons;
    /// see `stack-addon-not-in-dllmain` for the ones it is not fine for.
    pub load_from_dll_main: Vec<String>,
    /// Which of the two Neural Rendering setups is live. Computed before every
    /// finding below, because most of them only make sense on one path.
    pub active_path: ActivePath,
    /// Per-slot need, in the frontend's own slot order. Always all eight, so a
    /// row never has to fall back to "unknown".
    pub slots: Vec<SlotStatus>,
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

/// Suffixes the **user** switches a live file off with, recognised on read only.
///
/// # This list classifies for display. It does not decide liveness.
///
/// The distinction is the whole reason the list is allowed to exist. A list of
/// the suffixes humans use cannot be exhaustive — `.disabled`, `.old`, `.bak2`,
/// a date, a word — and known bug 4 is exactly what a short one costs: Kalpa
/// knew only `.kalpa-off`, so the user's real `renodx-dlss5.addon64.off`
/// matched nothing at all and a whole findings branch was skipped.
///
/// So liveness no longer asks this list anything. [`detect_active_path`] takes
/// the opposite rule — an add-on is live only when a file named *exactly* like
/// it is present — under which every rename aside is inert whether or not
/// Kalpa has heard of the suffix. What this list is genuinely for is telling a
/// parked file apart from a stray one *for display*: it is what earns a file a
/// [`ParkedFile`] row and a [`ParkedBy::User`] label, so "switched off" reads
/// differently from "never installed". Adding a suffix here widens what Kalpa
/// can *name*; it can never widen or narrow what Kalpa considers loaded.
///
/// The invariant above is unchanged and stays load-bearing: Kalpa still parks
/// as [`PARKED_SUFFIX`] alone and still removes or restores only files carrying
/// it. What this adds is *sight*. Kalpa knowing only its own suffix meant
/// `renodx-dlss5.addon64.off` and `dlss5-feed.addon64.off` — the primary user's
/// real, deliberately switched-off feed add-ons — matched nothing at all: not a
/// live add-on, not a backup, not a park. `has_file` was false for both, so the
/// entire DLSS 5 findings branch was skipped on the one machine it had been
/// written for.
///
/// Deliberately **not** merged into [`BACKUP_SUFFIXES`], which mean something
/// else entirely: a backup suffix marks a displaced *original*, a park suffix
/// marks a live file switched off. `client_toggle` reads the two lists for
/// opposite purposes — the originals are what it puts back, the parks are what
/// it moves aside — so conflating them would have it restore a mod over a stock
/// DLL.
pub const USER_PARK_SUFFIXES: [&str; 1] = [".off"];

/// The direct path's add-on: it hooks `nvngx_dlssnr.dll` itself and needs
/// nothing else in the pipeline.
const DIRECT_ADDON_STEM: &str = "renodx-dlss";

/// The feed path's add-ons. `renodx-dlss5` is the Neural Rendering half,
/// `dlss5-feed` the motion-vector half; either one live means the feed path is
/// the one being configured.
const FEED_ADDON_STEMS: [&str; 2] = ["renodx-dlss5", "dlss5-feed"];

/// Add-ons whose hooks have to be installed at `DllMain` time, and which are
/// therefore useless unless named in `[ADDON] LoadFromDllMain`.
///
/// Scoped deliberately tightly, and to *observed* evidence rather than to a
/// plausible class. Most ReShade add-ons work fine loaded the ordinary way, so
/// firing this at every `.addon64` in the folder would turn a one-line fix into
/// noise the user learns to scroll past.
///
/// `renodx-dlss.addon64` is here because it was watched failing this exact way
/// on a real ESO install and because it **names its own diagnosis** in
/// `ReShade.log`: "renodx-dlss.addon64 is not listed in ADDON.LoadFromDllMain.
/// Early DLSS hooks are not guaranteed for this session." ESO creates a D3D12
/// device first, so ReShade loads add-ons during `D3D12CreateDevice` and hooks
/// `d3d11.dll` afterwards, by which point an NVNGX hook installed at the normal
/// time is too late.
///
/// `renodx-dlss5.addon64` is deliberately **not** here. It is the same author
/// and plausibly the same mechanism, but nobody has watched it fail this way
/// and it has never printed that line. Adding it on the strength of the family
/// resemblance would put a Danger-level finding on every feed-path install in
/// existence on a guess.
const EARLY_HOOK_ADDON_STEMS: [&str; 1] = ["renodx-dlss"];

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

/// The section the live path's add-on saves its tunables to, and the add-on
/// that writes it — or `None` on the paths where no add-on is loaded to write
/// one.
///
/// Taken from `client_tuning::SECTIONS` rather than restated here. A `const
/// TUNING_SECTION: &str = "RenoDX.DLSS5"` used to live in this module beside an
/// identical one over there, and this module then read *only* that section: two
/// copies of one name, one of which belonged to the path the primary user does
/// not run. One table, consulted by both panels, is what makes the two of them
/// unable to disagree.
///
/// [`ActivePath::Both`] answers `Direct` for the same reason `SECTIONS` lists
/// the direct path first: both add-ons are loaded and both sections are live,
/// so this is a choice of which one to *name first*, never a claim that the
/// other is a fossil. See [`build_slots`]'s tuning arm.
fn expected_tuning_section(path: ActivePath) -> Option<(&'static str, &'static str)> {
    let want = match path {
        ActivePath::Direct | ActivePath::Both => RenoDxPath::Direct,
        ActivePath::Feed => RenoDxPath::Feed,
        ActivePath::Neither | ActivePath::Unknown => return None,
    };
    SECTIONS
        .iter()
        .find(|spec| spec.path == want && !spec.prefix_match)
        .map(|spec| (spec.name, spec.owner))
}

/// What [`ClientStack::tuning_owner`] says when `ReShade.ini` holds no known
/// tuning section at all.
///
/// Provenance is a fact about whether the *owning add-on* is loaded, and it is
/// answered whether or not the section exists — so with no section the question
/// becomes "is the add-on that would write one loaded?". On a live path it is,
/// and the block is simply one the user has not caused it to save yet. On
/// `Neither` nothing is loaded, and on `Unknown` Kalpa did not look.
fn tuning_owner_without_a_section(path: ActivePath) -> TuningProvenance {
    match path {
        ActivePath::Unknown => TuningProvenance::Unknown,
        ActivePath::Neither => TuningProvenance::Fossil,
        ActivePath::Direct | ActivePath::Feed | ActivePath::Both => TuningProvenance::Live,
    }
}

/// LaunchPad's marker file in the shader tree, for the "installed but unused"
/// note on the direct path.
const LAUNCHPAD_LABEL: &str = "iMMERSE LaunchPad";

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

    // `str::trim` does not remove `U+FEFF` — it is a format character, not
    // whitespace — so a file saved with a BOM opens with `\u{feff}[GENERAL]`,
    // whose header match fails and whose keys then land in the headerless
    // section. Every reading of that file would be silently wrong.
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);

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

/// Is `name` a parked file, and if so whose park is it and what does it restore?
///
/// [`PARKED_SUFFIX`] is checked first and on its own terms: it is the one name
/// Kalpa writes, and a file carrying it is unambiguously Kalpa's own work.
fn park_target(name: &str) -> Option<(ParkedBy, String, String)> {
    let lower = name.to_ascii_lowercase();
    if let Some(stem) = lower.strip_suffix(PARKED_SUFFIX) {
        return Some((ParkedBy::Kalpa, PARKED_SUFFIX.to_string(), stem.to_string()));
    }
    for suffix in USER_PARK_SUFFIXES {
        if let Some(stem) = lower.strip_suffix(suffix) {
            // `.off` on its own is a whole file name, not a suffix on one, and
            // a park has to restore to *something*.
            if stem.is_empty() {
                continue;
            }
            return Some((ParkedBy::User, suffix.to_string(), stem.to_string()));
        }
    }
    None
}

/// The stem of an add-on file name: `renodx-dlss.addon64` -> `renodx-dlss`.
///
/// Matched as a whole stem rather than a prefix, because `renodx-dlss` is a
/// prefix of `renodx-dlss5` and the two are the *opposite* paths. A
/// `starts_with` here would report every feed install as running the direct
/// add-on.
fn addon_stem(file_name: &str) -> Option<String> {
    let lower = file_name.to_ascii_lowercase();
    let (stem, ext) = lower.rsplit_once('.')?;
    ADDON_EXTENSIONS
        .contains(&ext)
        .then(|| stem.to_string())
        .filter(|stem| !stem.is_empty())
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
    let mut user_parked: Vec<ParkedFile> = Vec::new();
    let mut live_names: Vec<String> = Vec::new();
    let mut addon_files: Vec<String> = Vec::new();
    // Listed-and-empty and could-not-be-listed are different facts, and only
    // one of them licenses a verdict about which path is live. See
    // [`ActivePath::Unknown`].
    let mut listed = false;

    if let Ok(entries) = std::fs::read_dir(client_dir) {
        listed = true;
        for entry in entries.flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            live_names.push(lower.clone());

            if let Some((parked_by, suffix, restores)) = park_target(&lower) {
                let file = ParkedFile {
                    file_name: name.clone(),
                    restores,
                    size_bytes: size_of(&entry.path()),
                    target_present: false,
                    suffix,
                    parked_by,
                };
                match parked_by {
                    ParkedBy::Kalpa => parked.push(file),
                    ParkedBy::User => user_parked.push(file),
                }
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

    for file in parked.iter_mut().chain(user_parked.iter_mut()) {
        file.target_present = live_names.contains(&file.restores);
    }
    parked.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    user_parked.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    // Kalpa's own parks only. See the field doc: a file the user renamed by
    // hand is not Kalpa having switched the stack off.
    let is_disabled = parked
        .iter()
        .any(|file| INJECTOR_NAMES.contains(&file.restores.as_str()));

    let shaders = read_shader_tree(client_dir, &ini);
    let preset = read_preset(client_dir, &ini);

    let disabled_addons: Vec<String> = comma_list(ini_get(&ini, "ADDON", "DisabledAddons"));
    let load_from_dll_main: Vec<String> = comma_list(ini_get(&ini, "ADDON", "LoadFromDllMain"));

    items.sort_by(|a, b| a.role.cmp(&b.role).then(a.file_name.cmp(&b.file_name)));

    // A folder holding only `*.off` files is not an empty folder — it is a
    // stack the user switched off, and saying "nothing installed" about it is
    // the mistake that hid the primary user's parked feed add-ons.
    let is_empty = items.is_empty()
        && preserved_originals.is_empty()
        && parked.is_empty()
        && user_parked.is_empty()
        && !shaders.present;

    let mut stack = ClientStack {
        client_dir: client_dir.to_string_lossy().to_string(),
        items,
        preserved_originals,
        parked,
        user_parked,
        is_disabled,
        shaders,
        preset,
        tuning: Vec::new(),
        tuning_section: None,
        tuning_owner: TuningProvenance::Unknown,
        tuning_blocks: Vec::new(),
        disabled_addons,
        load_from_dll_main,
        active_path: ActivePath::Unknown,
        slots: Vec::new(),
        is_empty,
        findings: Vec::new(),
    };
    // Path first: it is what every slot and most findings below re-frame
    // around, so nothing may be computed before it. `live_names` is every file
    // name in the folder, parked ones included, because the rule is about what
    // is named exactly like an add-on rather than about what Kalpa classified.
    stack.active_path = if listed {
        // The evidence lines belong to the tuning panel, which asks
        // `detect_active_path` for them directly; this stack carries the
        // verdict alone.
        detect_active_path(&live_names, &stack.disabled_addons).0
    } else {
        ActivePath::Unknown
    };
    // Tuning second, because which block is live is a function of the path —
    // and read through `client_tuning::read_form`, not through this module's
    // own `ini` map, so the stack panel and the tuning panel are looking at one
    // answer rather than two. `read_form` is pure over `(text, path)`, so this
    // costs a second parse of a file already in memory and buys agreement.
    let form = read_form(
        &reshade_ini,
        &stack.client_dir,
        stack.active_path,
        Vec::new(),
    );
    stack.tuning_blocks = form
        .sections
        .iter()
        .filter(|section| section.present)
        .map(|section| TuningBlock {
            section: section.section.clone(),
            owner: section.owner.clone(),
            provenance: section.provenance,
            values: section_values(section),
        })
        .collect();
    // The headline is the first **live** block, and only falls back to the
    // first present one when nothing is live — which is what keeps a fossil on
    // the rail when a fossil is all there is, without ever letting one outrank
    // live tuning. `SECTIONS` order decides ties, so `Both` names the direct
    // path's block first; see [`expected_tuning_section`].
    let headline = stack
        .tuning_blocks
        .iter()
        .find(|block| block.provenance == TuningProvenance::Live)
        .or_else(|| stack.tuning_blocks.first())
        .cloned();
    match headline {
        Some(block) => {
            stack.tuning_section = Some(block.section);
            stack.tuning_owner = block.provenance;
            stack.tuning = block.values;
        }
        // Presence is `tuning_section`; provenance is answered either way. See
        // [`TuningProvenance`].
        None => stack.tuning_owner = tuning_owner_without_a_section(stack.active_path),
    }
    stack.slots = build_slots(&stack);
    stack.findings = build_findings(&stack);
    stack
}

/// Flatten one of `client_tuning`'s sections back to plain `key=value` pairs.
///
/// That module splits a section in two: keys it has a verified field table for
/// become typed `fields`, everything else stays a raw `entry`. The stack panel
/// renders neither — it counts and lists — so both halves come back here, typed
/// ones first in the add-on's own order. A field whose `current` is `None` is a
/// key the file does not have, and is dropped: this list is what is *in*
/// `ReShade.ini`, never what could be.
fn section_values(section: &TuningSection) -> Vec<TuningValue> {
    section
        .fields
        .iter()
        .filter_map(|field| {
            field.current.as_ref().map(|value| TuningValue {
                key: field.key.clone(),
                value: value.clone(),
            })
        })
        .chain(section.entries.iter().map(|entry| TuningValue {
            key: entry.key.clone(),
            value: entry.value.clone(),
        }))
        .collect()
}

/// Split a ReShade comma list, dropping the empties `DisabledAddons=` produces.
fn comma_list(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
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
///
/// `source` is the right-hand side of a `Technique@Source.fx` entry in a preset
/// file, which the user or ReShade owns and Kalpa does not validate on the way
/// in. A bare `Path::join` with a segment like `C:pwned.fx` is drive-relative on
/// Windows: it discards `shader_dir` entirely and resolves against the process
/// cwd. Nothing here writes, so the worst case today is reporting a shader as
/// present because an unrelated file exists elsewhere — but it is the same
/// class as the two joins already fixed in this module, and the answer feeds
/// findings the user acts on. `safe_relative_join` refusing reads as "not
/// found", which is the safe direction for a presence test.
fn find_shader_source(shader_dir: &Path, source: &str) -> Option<std::path::PathBuf> {
    let direct = crate::client_write::safe_relative_join(shader_dir, source).ok()?;
    if direct.is_file() {
        return Some(direct);
    }
    if let Some(hit) = case_insensitive_file(shader_dir, source) {
        return Some(hit);
    }
    std::fs::read_dir(shader_dir).ok()?.flatten().find_map(|e| {
        let dir = e.path();
        if !dir.is_dir() {
            return None;
        }
        let nested = crate::client_write::safe_relative_join(&dir, source).ok()?;
        if nested.is_file() {
            return Some(nested);
        }
        case_insensitive_file(&dir, source)
    })
}

/// Find a file in `dir` whose name matches `file_name` ignoring case.
///
/// The lookup above asks the *filesystem* whether a name matches, which makes
/// the answer depend on which filesystem it is. That is not a portability
/// nicety here, it is a correctness bug with a real install behind it:
/// [`LAUNCHPAD_SOURCE`] is spelled lowercase in this file while the effect
/// iMMERSE actually ships is `MartysMods_LAUNCHPAD.fx`, so the join resolved on
/// Windows and macOS and failed on Linux — where Kalpa runs ESO through Proton.
/// LaunchPad went invisible there, and the Shaders slot silently downgraded
/// from `InstalledUnused` (keep this, Kalpa cannot refetch it) to
/// `NotOnThisPath` (nothing to see). Linux CI caught it; the two case-folding
/// platforms never could.
///
/// Only ever called with a bare file name — the caller's `safe_relative_join`
/// stays the guard against a preset pointing somewhere outside the shader tree,
/// and a `source` carrying any separator is refused here rather than walked.
fn case_insensitive_file(dir: &Path, file_name: &str) -> Option<std::path::PathBuf> {
    if file_name.contains(['/', '\\', ':']) {
        return None;
    }
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        (path.is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(file_name))
        .then_some(path)
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

// ── Which path is live ───────────────────────────────────────────────────

/// **The** liveness rule, and the only one in this crate. Returns the file name
/// an add-on stem is actually loaded under, or `None`.
///
/// Two things have to be true, and neither of them is a suffix lookup. A file
/// named *exactly* like the add-on has to be present — `addon_stem` accepts a
/// name only when its final extension is one ReShade loads, so
/// `renodx-dlss5.addon64.off`, `…​.kalpa-off`, `….disabled` and any other
/// rename aside all fail it without Kalpa having to know the suffix — and the
/// name must not be sitting in `DisabledAddons`, which is ReShade being told
/// not to load a file that is otherwise perfectly present.
///
/// This replaced a rule that asked [`USER_PARK_SUFFIXES`] whether a file was
/// parked. That rule was only ever as good as the list, and a list of the names
/// humans give a switched-off file cannot be finished; see known bug 4. This
/// one needs no list and cannot be defeated by a suffix Kalpa has not heard of.
///
/// `disabled` is compared case-insensitively in both directions: file names are
/// case-insensitive on Windows and `DisabledAddons` is written however the user
/// typed it, so a case difference must never be the reason a switched-off
/// add-on reads as loaded.
fn live_addon_file<'a>(names: &'a [String], disabled: &[String], stem: &str) -> Option<&'a String> {
    names.iter().find(|name| {
        addon_stem(name).is_some_and(|found| found == stem)
            && !disabled.iter().any(|off| off.eq_ignore_ascii_case(name))
    })
}

/// Is an add-on with this stem **loaded**, according to an already-inspected
/// stack?
///
/// A convenience over [`live_addon_file`], not a second rule: [`ClientStack::items`]
/// holds exactly the loadable names — everything renamed aside was routed to
/// [`ClientStack::parked`] or [`ClientStack::user_parked`] before it got here —
/// so this asks the same question of the same rule.
fn addon_is_live(stack: &ClientStack, stem: &str) -> bool {
    let names: Vec<String> = stack
        .items
        .iter()
        .filter(|item| item.role == StackRole::Addon)
        .map(|item| item.file_name.clone())
        .collect();
    live_addon_file(&names, &stack.disabled_addons, stem).is_some()
}

/// The canonical file name for an add-on stem: the 64-bit one, which is what
/// ESO loads and what every message about a missing add-on should name.
fn canonical_addon_file(stem: &str) -> String {
    format!("{stem}.{}", ADDON_EXTENSIONS[0])
}

/// One add-on's liveness, plus the plain-English observation behind it.
///
/// The evidence is not decoration. "Parked" and "never installed" are different
/// situations for the user, and a verdict with no observations under it asks
/// them to take Kalpa's word for something they can check in ten seconds.
fn addon_liveness(
    names: &[String],
    disabled: &[String],
    stem: &str,
    evidence: &mut Vec<String>,
) -> bool {
    if let Some(live) = live_addon_file(names, disabled, stem) {
        evidence.push(format!("{live} is present and not disabled."));
        return true;
    }

    if let Some(present) = names
        .iter()
        .find(|name| addon_stem(name).is_some_and(|found| found == stem))
    {
        evidence.push(format!(
            "{present} is present but listed in DisabledAddons, so ReShade will not load it."
        ));
        return false;
    }

    let canonical = canonical_addon_file(stem);
    let parked: Vec<&str> = names
        .iter()
        .filter(|name| {
            ADDON_EXTENSIONS
                .iter()
                .any(|ext| name.starts_with(&format!("{stem}.{ext}.")))
        })
        .map(|name| name.as_str())
        .collect();
    if parked.is_empty() {
        evidence.push(format!("{canonical} is not in the client folder."));
    } else {
        evidence.push(format!(
            "{canonical} is renamed aside as {}, so ReShade will not load it.",
            parked.join(", ")
        ));
    }
    false
}

/// Decide which RenoDX path is live, from a client directory's file names and
/// `ReShade.ini`'s `DisabledAddons` list.
///
/// The one detection function. `client_tuning` re-exports it rather than
/// keeping its own: two modules answering "is this add-on live?" by two rules,
/// in one crate, feeding one panel, is the bug class this whole model exists to
/// eliminate, and it had already grown back once.
///
/// Pure over its inputs, so the entire decision is testable without a
/// filesystem — and so a caller that could not list the folder has somewhere
/// honest to go instead of calling this with an empty slice, which would read
/// as [`ActivePath::Neither`]. That caller passes [`ActivePath::Unknown`].
///
/// Returns the verdict and the observations behind it, which the panel shows
/// verbatim. See [`live_addon_file`] for the rule itself.
pub fn detect_active_path(file_names: &[String], disabled: &[String]) -> (ActivePath, Vec<String>) {
    let lower: Vec<String> = file_names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();

    let mut evidence = Vec::new();
    let direct = addon_liveness(&lower, disabled, DIRECT_ADDON_STEM, &mut evidence);
    // `fold` rather than `any`, because `any` short-circuits and the evidence
    // for the second feed add-on would silently go missing whenever the first
    // one was live.
    let feed = FEED_ADDON_STEMS.iter().fold(false, |live, stem| {
        addon_liveness(&lower, disabled, stem, &mut evidence) || live
    });

    let path = match (direct, feed) {
        (true, false) => ActivePath::Direct,
        (false, true) => ActivePath::Feed,
        (true, true) => ActivePath::Both,
        (false, false) => ActivePath::Neither,
    };
    (path, evidence)
}

/// Is the feed pipeline live? True on [`ActivePath::Both`] as well, and that is
/// the point: a feed add-on that is loaded is a feed add-on whose technique
/// order, motion-vector provider and preset all still matter. Gating those
/// checks on `== Feed` is what let `Both` hide real feed breakage.
fn feed_is_live(path: ActivePath) -> bool {
    matches!(path, ActivePath::Feed | ActivePath::Both)
}

/// Every add-on name, on either park list, whose stem is a feed-path add-on.
///
/// These are the user's fallback and Kalpa's supply chain for them is a Discord
/// invite, so they are named in the copy that explains why to keep them rather
/// than being silently counted.
fn parked_feed_addons(stack: &ClientStack) -> Vec<String> {
    let mut names: Vec<String> = stack
        .parked
        .iter()
        .chain(stack.user_parked.iter())
        .filter(|file| {
            addon_stem(&file.restores).is_some_and(|stem| FEED_ADDON_STEMS.contains(&stem.as_str()))
        })
        .map(|file| file.restores.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Is iMMERSE LaunchPad in the shader tree?
fn launchpad_installed(stack: &ClientStack) -> bool {
    let shader_dir = Path::new(&stack.client_dir)
        .join("reshade-shaders")
        .join("Shaders");
    find_shader_source(&shader_dir, LAUNCHPAD_SOURCE).is_some()
}

/// The sentence explaining why a link-only piece is worth keeping.
///
/// One place rather than three, because the reasoning is the same each time and
/// it is the reasoning, not the wording, that has to stay true: Kalpa fetches
/// none of these, so the user's existing copy is the only one there is.
fn keep_link_only(what: &str, where_from: &str) -> String {
    format!(
        "{what} is not something Kalpa can fetch — {where_from} — so the copy you already \
         have is your only way back to the other path if this one ever stops working. \
         Kalpa will not suggest removing it."
    )
}

/// One sentence for every row on [`ActivePath::Unknown`], shared by
/// [`build_slots`] and [`tuning_slot`] so the two rows cannot say it
/// differently.
const UNREADABLE_ROW: &str = "Kalpa could not read this client folder, so it cannot tell which \
                              Neural Rendering path is live or whether anything is missing here.";

/// The need axis, one entry per frontend slot, always all eight.
///
/// Read the module doc first. The short version: a slot that is empty because
/// the live path does not want it is a *correct* state and has to read as one.
/// Rendering it as a gap — or as an Info-level finding, which is the same
/// mistake with a colour on it — is what made a working direct-path install
/// look six-eighths broken while the panel insisted everything agreed.
fn build_slots(stack: &ClientStack) -> Vec<SlotStatus> {
    let path = stack.active_path;
    let launchpad = launchpad_installed(stack);
    let parked_feed = parked_feed_addons(stack);
    let has_nr_runtime = has_role(stack, StackRole::NeuralRendering);

    let make = |slot: StackSlot, need: SlotNeed, reason: String, keep_because: Option<String>| {
        SlotStatus {
            slot,
            need,
            reason,
            keep_because,
        }
    };

    // The folder could not be read, so Kalpa knows nothing about this row — and
    // says so, rather than borrowing `Neither`'s copy, which asserts that an
    // empty slot is the correct answer. See [`UNREADABLE_ROW`].
    let unreadable =
        |slot: StackSlot| make(slot, SlotNeed::Unknown, UNREADABLE_ROW.to_string(), None);

    let launchpad_keep = || {
        launchpad.then(|| {
            keep_link_only(
                LAUNCHPAD_LABEL,
                "its licence forbids public propagation, so Kalpa links it and never downloads it",
            )
        })
    };

    // Named, not counted: these are the user's only copies, and a sentence that
    // says "two add-ons are parked" gives them nothing to check the folder
    // against.
    let parked_feed_keep = (!parked_feed.is_empty()).then(|| {
        let (is, they) = if parked_feed.len() == 1 {
            ("is", "it is")
        } else {
            ("are", "they are")
        };
        format!(
            "{} {is} switched off, not missing — {they} the feed path's fallback. {}",
            parked_feed.join(" and "),
            keep_link_only(
                "That add-on set",
                "it is distributed through a Discord with no stable URL and no licence"
            )
        )
    });

    vec![
        make(
            StackSlot::Reshade,
            SlotNeed::Required,
            "ReShade is the file the game loads. Both Neural Rendering paths, and every \
             effect below, run inside it."
                .to_string(),
            None,
        ),
        match path {
            ActivePath::Direct => make(
                StackSlot::Addons,
                SlotNeed::Required,
                "The direct path is a single add-on: renodx-dlss.addon64 hooks the Neural \
                 Rendering runtime itself, with no feed add-on and no host process."
                    .to_string(),
                parked_feed_keep,
            ),
            ActivePath::Feed => make(
                StackSlot::Addons,
                SlotNeed::Required,
                "The feed path needs renodx-dlss5.addon64 and dlss5-feed.addon64 together, \
                 with dlss5-feed-host64.exe beside them."
                    .to_string(),
                None,
            ),
            ActivePath::Both => make(
                StackSlot::Addons,
                SlotNeed::Required,
                "Both Neural Rendering add-ons are loaded: renodx-dlss.addon64 and the feed \
                 set. ReShade will load both and Kalpa does not guess which one wins, so \
                 every check for both paths applies until one of them is switched off."
                    .to_string(),
                None,
            ),
            ActivePath::Neither => make(
                StackSlot::Addons,
                SlotNeed::NotOnThisPath,
                "No Neural Rendering add-on is loaded here. ReShade runs perfectly well \
                 without one — this is an effects-only install."
                    .to_string(),
                None,
            ),
            ActivePath::Unknown => unreadable(StackSlot::Addons),
        },
        match path {
            ActivePath::Direct => make(
                StackSlot::Nr,
                SlotNeed::Required,
                "renodx-dlss.addon64 hooks nvngx_dlssnr.dll directly, so the runtime has to \
                 be in this folder for anything to happen."
                    .to_string(),
                None,
            ),
            ActivePath::Feed => make(
                StackSlot::Nr,
                SlotNeed::Required,
                "renodx-dlss5.addon64 drives nvngx_dlssnr.dll, so the runtime has to be in \
                 this folder."
                    .to_string(),
                None,
            ),
            ActivePath::Neither if has_nr_runtime => make(
                StackSlot::Nr,
                SlotNeed::InstalledUnused,
                "The Neural Rendering runtime is here, but no add-on is loaded to drive it, \
                 so nothing calls into it."
                    .to_string(),
                Some(
                    "This is an NVIDIA binary. Kalpa never downloads one, so if you remove it \
                     you have to find it again yourself."
                        .to_string(),
                ),
            ),
            ActivePath::Both => make(
                StackSlot::Nr,
                SlotNeed::Required,
                "Both loaded add-ons drive nvngx_dlssnr.dll, so the runtime has to be in \
                 this folder either way."
                    .to_string(),
                None,
            ),
            ActivePath::Neither => make(
                StackSlot::Nr,
                SlotNeed::NotOnThisPath,
                "Nothing loaded here uses the Neural Rendering runtime.".to_string(),
                None,
            ),
            ActivePath::Unknown => unreadable(StackSlot::Nr),
        },
        make(
            StackSlot::Sr,
            SlotNeed::Required,
            "ESO loads nvngx_dlss.dll itself for DLSS and DLAA. That is true on both paths \
             and with no path at all."
                .to_string(),
            None,
        ),
        match path {
            ActivePath::Direct if launchpad => make(
                StackSlot::Shaders,
                SlotNeed::InstalledUnused,
                "The direct path runs no ReShade technique, so nothing in the shader tree \
                 feeds Neural Rendering. LaunchPad here is the feed path's provider."
                    .to_string(),
                launchpad_keep(),
            ),
            ActivePath::Direct => make(
                StackSlot::Shaders,
                SlotNeed::NotOnThisPath,
                "Neural Rendering needs no shader effects on the direct path. Anything you \
                 install here is for ReShade's own effects."
                    .to_string(),
                None,
            ),
            ActivePath::Feed | ActivePath::Both => make(
                StackSlot::Shaders,
                SlotNeed::Required,
                "DLSS5_Feed.fx and the motion-vector provider that feeds it both live in the \
                 shader tree."
                    .to_string(),
                None,
            ),
            ActivePath::Neither => make(
                StackSlot::Shaders,
                SlotNeed::NotOnThisPath,
                "Shader packs are ReShade's own effects. No Neural Rendering path is live to \
                 depend on them."
                    .to_string(),
                None,
            ),
            ActivePath::Unknown => unreadable(StackSlot::Shaders),
        },
        match path {
            ActivePath::Direct => make(
                StackSlot::Motion,
                SlotNeed::NotOnThisPath,
                "renodx-dlss.addon64 gets motion vectors from the game through its own hooks. \
                 Nothing in ReShade has to produce them, so an empty slot here is correct."
                    .to_string(),
                launchpad_keep(),
            ),
            ActivePath::Feed | ActivePath::Both => make(
                StackSlot::Motion,
                SlotNeed::Required,
                "DLSS5_Feed reads its motion vectors from an effect enabled above it in the \
                 preset. With nothing above it the feed reads zeros."
                    .to_string(),
                None,
            ),
            ActivePath::Neither => make(
                StackSlot::Motion,
                SlotNeed::NotOnThisPath,
                "Nothing loaded here consumes motion vectors.".to_string(),
                None,
            ),
            ActivePath::Unknown => unreadable(StackSlot::Motion),
        },
        match path {
            ActivePath::Direct => make(
                StackSlot::Preset,
                SlotNeed::NotOnThisPath,
                "The direct path enables no technique of its own, so an empty `Techniques=` — \
                 or no preset at all — is the correct configuration, not a missing one."
                    .to_string(),
                None,
            ),
            ActivePath::Feed | ActivePath::Both => make(
                StackSlot::Preset,
                SlotNeed::Required,
                "The feed path is configured entirely in the preset: DLSS5_Feed enabled, with \
                 its motion-vector provider ordered above it."
                    .to_string(),
                None,
            ),
            ActivePath::Neither => make(
                StackSlot::Preset,
                SlotNeed::NotOnThisPath,
                "No Neural Rendering path is live, so the preset is yours to arrange. Kalpa \
                 checks its order only where the order changes the picture."
                    .to_string(),
                None,
            ),
            ActivePath::Unknown => unreadable(StackSlot::Preset),
        },
        tuning_slot(stack, path, make),
    ]
}

/// The Tuning row.
///
/// Lifted out of [`build_slots`]'s vector because it is the one row that has to
/// look up its own block first, and because its copy is where the fossil bug
/// used to be written down in words: this arm said `[RenoDX.DLSS5]` "is
/// history, not this install's live tuning" on a direct-path install that had
/// ~30 live keys in `[RENODX-DLSS]` sitting unmentioned. Every sentence in it
/// is now about the block Kalpa actually selected.
fn tuning_slot(
    stack: &ClientStack,
    path: ActivePath,
    make: impl Fn(StackSlot, SlotNeed, String, Option<String>) -> SlotStatus,
) -> SlotStatus {
    let headline = stack
        .tuning_section
        .as_deref()
        .and_then(|name| stack.tuning_blocks.iter().find(|b| b.section == name));

    // Named, never counted, and never advice to delete: the feed add-ons come
    // from a Discord with no stable URL, so a parked path's saved settings are
    // the user's only copy of them. See [`SlotStatus::keep_because`].
    let fossils: Vec<&str> = stack
        .tuning_blocks
        .iter()
        .filter(|block| block.provenance == TuningProvenance::Fossil)
        .map(|block| block.section.as_str())
        .collect();
    let fossil_keep = (!fossils.is_empty()).then(|| {
        let (is, they) = if fossils.len() == 1 {
            ("is", "it is")
        } else {
            ("are", "they are")
        };
        format!(
            "[{}] {is} also in ReShade.ini and left exactly as saved — {they} what the parked \
             add-on would come back to if you ever switch paths.",
            fossils.join("] and [")
        )
    });

    // Presence first, then provenance: a section that is not in the file at all
    // is a different row from one that is there and not in force, and
    // [`TuningProvenance`] deliberately has no variant for the former.
    match (headline, path) {
        (Some(block), _) => match block.provenance {
            TuningProvenance::Live => make(
                StackSlot::Tuning,
                SlotNeed::Required,
                format!(
                    "{} saves its Neural Rendering settings to [{}] in ReShade.ini, and that \
                     add-on is loaded.",
                    block.owner, block.section
                ),
                fossil_keep,
            ),
            TuningProvenance::Unknown => make(
                StackSlot::Tuning,
                SlotNeed::Unknown,
                format!(
                    "[{}] is in ReShade.ini, but Kalpa could not read the client folder and so \
                     cannot tell whether {} is loaded. The values are shown as they are on \
                     disk and nothing here is edited.",
                    block.section, block.owner
                ),
                None,
            ),
            // Reached whenever the only tuning block present belongs to a path
            // that is not the live one — not only when nothing is live. A
            // direct-live install that has not yet written `[RENODX-DLSS]`
            // still has only the feed's `[RenoDX.DLSS5]` block on disk, so it
            // lands here too: see
            // `a_tuning_block_left_by_a_parked_addon_is_marked_as_a_fossil`.
            TuningProvenance::Fossil => make(
                StackSlot::Tuning,
                SlotNeed::InstalledUnused,
                format!(
                    "[{}] belongs to {}, which is not loaded here. These values are history, \
                     not this install's live tuning.",
                    block.section, block.owner
                ),
                Some(
                    "Left exactly as saved. They are the settings that path would come back to \
                     if you ever switch to it."
                        .to_string(),
                ),
            ),
        },
        (None, ActivePath::Unknown) => make(
            StackSlot::Tuning,
            SlotNeed::Unknown,
            UNREADABLE_ROW.to_string(),
            None,
        ),
        // A live path with nothing saved yet is not a gap: these add-ons write
        // their section the first time a setting is changed in their own
        // overlay, so a fresh install correctly has no block at all.
        (None, ActivePath::Direct | ActivePath::Feed | ActivePath::Both) => {
            let (section, owner) = expected_tuning_section(path)
                .expect("every live path has a section owner in SECTIONS");
            make(
                StackSlot::Tuning,
                SlotNeed::NotOnThisPath,
                format!(
                    "{owner} saves its settings to [{section}] in ReShade.ini and has not \
                     written that block here yet, so there is correctly nothing to show."
                ),
                None,
            )
        }
        (None, ActivePath::Neither) => make(
            StackSlot::Tuning,
            SlotNeed::NotOnThisPath,
            "No add-on has saved a tuning block to ReShade.ini.".to_string(),
            None,
        ),
    }
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
///
/// Most of them are also **path-specific**, and reading
/// [`ClientStack::active_path`] before deciding is not optional. The feed
/// path's checks — technique order, motion-vector provider, an empty preset —
/// describe a pipeline the direct path does not have, and firing them there
/// would tell a working install it was broken. The converse mistake is the one
/// that shipped: nothing was gated, so every check that could not apply simply
/// did not fire, and "no findings" was read as "everything agrees".
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

    // Live, not merely present. `has_file` was the old test and it answered the
    // wrong question twice over: it said yes to a file ReShade has been told
    // not to load, and no to `renodx-dlss5.addon64.off`, which is a real,
    // deliberately switched-off add-on. The second miss is why this entire
    // branch never ran on the machine it was written for.
    let direct_addon = addon_is_live(stack, DIRECT_ADDON_STEM);
    let nr_addon = addon_is_live(stack, FEED_ADDON_STEMS[0]);
    let feed_addon = addon_is_live(stack, FEED_ADDON_STEMS[1]);

    // An add-on that installs early DLSS hooks and is not named in
    // `[ADDON] LoadFromDllMain`. ESO creates a D3D12 device first, so ReShade
    // loads the add-on during `D3D12CreateDevice` and hooks `d3d11.dll`
    // afterwards — by which point the NVNGX hooks are too late. Everything
    // loads, the add-on's menu appears in the overlay, nothing errors, and
    // Neural Rendering does nothing at all. RenoDX prints the diagnosis into
    // `ReShade.log` itself; this finding is Kalpa reaching it without the log.
    //
    // The fix is one ini line, and it has to be made with ESO **closed**:
    // ReShade rewrites `ReShade.ini` when the game exits and discards anything
    // edited underneath it.
    for item in stack
        .items
        .iter()
        .filter(|item| item.role == StackRole::Addon)
    {
        let Some(stem) = addon_stem(&item.file_name) else {
            continue;
        };
        if !EARLY_HOOK_ADDON_STEMS.contains(&stem.as_str()) {
            continue;
        }
        // Not loaded at all is a different finding — `stack-addon-disabled`
        // already says so, and telling someone to fix the load order of an
        // add-on ReShade is not loading would be advice with no effect.
        if !addon_is_live(stack, &stem) {
            continue;
        }
        if stack
            .load_from_dll_main
            .iter()
            .any(|listed| listed.eq_ignore_ascii_case(&item.file_name))
        {
            continue;
        }
        let existing = stack.load_from_dll_main.join(",");
        let line = if existing.is_empty() {
            format!("LoadFromDllMain={}", item.file_name)
        } else {
            format!("LoadFromDllMain={existing},{}", item.file_name)
        };
        out.push(finding(
            "stack-addon-not-in-dllmain",
            HealthLevel::Danger,
            "An add-on is loaded too late for its hooks to land",
            format!(
                "{} installs its DLSS hooks at load time, but it is not listed in \
                 LoadFromDllMain under [ADDON] in ReShade.ini. ESO creates a D3D12 device \
                 first, so ReShade loads the add-on and only then hooks d3d11.dll — the \
                 hooks miss, and the add-on runs with nothing behind its menu.\n\n\
                 Close ESO first: ReShade rewrites ReShade.ini when the game exits and will \
                 discard an edit made while it is running. Then put this line under the \
                 [ADDON] section:\n\n    {line}",
                item.file_name
            ),
        ));
    }

    if !has_role(stack, StackRole::Injector) && (direct_addon || nr_addon || feed_addon) {
        out.push(finding(
            "stack-no-injector",
            HealthLevel::Danger,
            "ReShade addons are present but nothing loads them",
            "The addon binaries are in the client folder, but there is no dxgi.dll or \
             d3d11.dll for the game to load, so ReShade never starts and none of them run."
                .to_string(),
        ));
    }

    // Both paths drive `nvngx_dlssnr.dll`; only the add-on in front of it
    // differs, so the finding names whichever one is actually loaded rather
    // than the feed path's, which is what it used to say unconditionally.
    if (direct_addon || nr_addon) && !has_role(stack, StackRole::NeuralRendering) {
        let addon = if direct_addon {
            "renodx-dlss.addon64"
        } else {
            "renodx-dlss5.addon64"
        };
        out.push(finding(
            "stack-nr-runtime-missing",
            HealthLevel::Danger,
            "Neural Rendering addon has no runtime",
            format!(
                "{addon} is installed, but nvngx_dlssnr.dll is not in the client folder. The \
                 addon will load and Neural Rendering will not work."
            ),
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
        // Both of the next two checks are about the preset being *empty of
        // technique*, and both are silent on the direct path — deliberately.
        // renodx-dlss.addon64 enables no ReShade technique at all, so a preset
        // that is missing, or present with an empty `Techniques=`, is the
        // correct configuration rather than a broken one. Reporting it would be
        // Kalpa calling a working install broken, which is the mirror of the
        // bug that made it call a broken install fine.
        //
        // `Both` is not `Direct`, so the feed's preset checks apply there —
        // deliberately. `Unknown` is excluded from the other end: Kalpa could
        // not read the folder, and a Danger-level finding about a preset that
        // may be entirely correct is a guess with a colour on it.
        let preset_matters = !matches!(stack.active_path, ActivePath::Direct | ActivePath::Unknown);

        if !preset.exists && preset_matters {
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

        // An empty `Techniques=` used to pass every check here, because
        // `stack-preset-missing` only ever asked whether the file existed. On
        // the feed path that preset is the entire configuration — DLSS5_Feed
        // and its provider are both enabled there and nowhere else — so an
        // empty one means the add-ons load, the host runs, and not a single
        // effect executes.
        // `feed_is_live`, not `== Feed`: an install running both add-ons has a
        // live feed pipeline, and gating this on the feed being the *only*
        // path is how a broken preset on such an install went unreported.
        if preset.exists && preset.techniques.is_empty() && feed_is_live(stack.active_path) {
            out.push(finding(
                "stack-preset-empty",
                HealthLevel::Danger,
                "The active preset enables nothing",
                format!(
                    "{} exists but its Techniques= list is empty, so ReShade runs no effects \
                     at all. The feed path lives in this list: DLSS5_Feed has to be enabled \
                     here, with a motion-vector provider above it. Nothing errors — the \
                     add-ons load and simply have no work to do.",
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
                    // The overlay's label, not the identifier: the user's next
                    // act is to find this in ReShade's technique list, and the
                    // list shows labels.
                    match provider.kind.technique_label() {
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
            // Gated on the list being non-empty: an empty preset is
            // `stack-preset-empty`'s to report, and it says it better. Two
            // findings for one line of one file is how a panel teaches people
            // to skim it.
            _ if feed_addon && !preset.techniques.is_empty() => out.push(finding(
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
#[tauri::command(async)]
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

    /// The primary user's **real** `ReShade.ini`, after the one-line fix that
    /// made Neural Rendering actually run, trimmed to the parts that matter.
    ///
    /// Read it against [`REAL_RESHADE_INI`] above: `DisabledAddons` is empty in
    /// both, and `OverlayCollapsed` still names the two feed add-ons, because
    /// ReShade remembers the overlay state of add-ons that are no longer
    /// loaded. Neither of those says anything about what is running. The line
    /// that does is `LoadFromDllMain`.
    const DIRECT_RESHADE_INI: &str = "\
[ADDON]
DisabledAddons=
LoadFromDllMain=renodx-dlss.addon64
OverlayCollapsed=DLSS 5 Neural Rendering@renodx-dlss5.addon64,DLSS 5 Feed 0.4.0@dlss5-feed.addon64

[GENERAL]
EffectSearchPaths=.\\reshade-shaders\\Shaders\\**
PresetPath=.\\ReShadePreset.ini
";

    /// The primary user's live, working install, file for file.
    ///
    /// Reproduced from a read of the real folder on 2026-09-03: the direct
    /// add-on live, the two feed add-ons and the feed config parked under the
    /// user's own `.off`, the stock DLSS kept as `.disabled-bak`, and a preset
    /// with **no `Techniques=` line at all**. Every one of those is correct,
    /// and every one of them used to read as either invisible or broken.
    fn direct_path_stack(dir: &Path) {
        write(dir, "eso64.exe", "");
        write(dir, "dxgi.dll", "");
        write(dir, "nvngx_dlss.dll", "");
        write(dir, "nvngx_dlssnr.dll", "");
        write(dir, "renodx-dlss.addon64", "");
        write(dir, "nvngx_dlss.dll.disabled-bak", "");
        write(dir, "renodx-dlss5.addon64.off", "");
        write(dir, "dlss5-feed.addon64.off", "");
        write(dir, "dlss5-feed.cfg.off", "");
        write(dir, "ReShade.ini", DIRECT_RESHADE_INI);
        // No `Techniques=` line. The direct path enables nothing.
        write(dir, "ReShadePreset.ini", "PreprocessorDefinitions=\n");
        let shaders = dir.join("reshade-shaders").join("Shaders");
        std::fs::create_dir_all(&shaders).expect("mkdir shaders");
        std::fs::write(shaders.join("MartysMods_LAUNCHPAD.fx"), "").unwrap();
    }

    fn ids(stack: &ClientStack) -> Vec<&str> {
        stack.findings.iter().map(|f| f.id.as_str()).collect()
    }

    fn slot(stack: &ClientStack, want: StackSlot) -> &SlotStatus {
        stack
            .slots
            .iter()
            .find(|entry| entry.slot == want)
            .expect("every slot is always present")
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
    ///
    /// The fixture said `LUMENITE: Kernel 2.0@lumenite_kernel.fx` until
    /// 2026-09-03, which ReShade would never write — that is the shader's
    /// `ui_label`, and a preset records the identifier. The invented fixture is
    /// what held the wrong provider table in place; see
    /// [`MvProviderKind::technique_names`].
    #[test]
    fn the_current_mv_provider_definition_is_understood() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=DLSS5_Feed@DLSS5_Feed.fx,Lumenite_Kernel@lumenite_Kernel.fx\n\n\
             [DLSS5_Feed.fx]\nPreprocessorDefinitions=DLSS5_MV_PROVIDER=3\n",
        );
        let stack = inspect_stack(tmp.path());

        let provider = stack
            .preset
            .as_ref()
            .and_then(|preset| preset.mv_provider.as_ref())
            .expect("provider");
        assert_eq!(provider.kind, MvProviderKind::LumeniteKernel);
        assert_eq!(provider.technique.as_deref(), Some("Lumenite_Kernel"));

        // The ordering finding names the technique as the *preset* spells it,
        // so it matches the technique list Kalpa shows beside it and stays
        // greppable against the file.
        let detail = &stack
            .findings
            .iter()
            .find(|f| f.id == "stack-technique-order")
            .expect("the ordering check must run for this provider too")
            .detail;
        assert!(detail.contains("Lumenite_Kernel"), "{detail}");
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

    /// A preset stores technique *identifiers*, so the provider table has to
    /// hold identifiers too.
    ///
    /// This is the regression for a table that held `ui_label`s
    /// (`LUMENITE: Kernel 2.0`) and a file name (`vort_Motion`) instead. Every
    /// case below is a correctly configured stack: the selected provider's
    /// technique is enabled and sits above the feed, so there is nothing to
    /// report. Against the old table none of them matched, the provider
    /// resolved to `technique: None`, and each was reported as
    /// `stack-mv-provider-missing` — Kalpa telling a working install it was
    /// broken.
    #[test]
    fn a_provider_is_matched_by_its_identifier_not_its_overlay_label() {
        // (DLSS5_MV_PROVIDER value, technique identifier, effect file)
        let cases = [
            (2, "vort_MotionEffects", "vort_Motion.fx"),
            (3, "Lumenite_Kernel", "lumenite_Kernel.fx"),
            (4, "Lumenite_QuantMotion", "lumenite_QuantMotion.fx"),
        ];
        for (value, technique, source) in cases {
            let tmp = tempfile::tempdir().unwrap();
            healthy_stack(tmp.path());
            write(tmp.path(), &format!("reshade-shaders/Shaders/{source}"), "");
            write(
                tmp.path(),
                "ReShadePreset.ini",
                &format!(
                    "Techniques={technique}@{source},DLSS5_Feed@DLSS5_Feed.fx\n\n\
                     [DLSS5_Feed.fx]\nPreprocessorDefinitions=DLSS5_MV_PROVIDER={value}\n"
                ),
            );
            let stack = inspect_stack(tmp.path());

            assert!(
                !ids(&stack).contains(&"stack-mv-provider-missing"),
                "{technique} is enabled above the feed, so nothing should be reported; \
                 got {:?}",
                ids(&stack)
            );
            let provider = stack.preset.unwrap().mv_provider.unwrap();
            assert_eq!(
                provider.technique.as_deref(),
                Some(technique),
                "{technique} should have been resolved as the supplying technique"
            );
        }
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

    /// A preset names its own shader sources, and `C:evil.fx` is
    /// drive-relative on Windows: a bare join would discard the shader
    /// directory and answer from the process cwd instead.
    #[test]
    fn a_drive_relative_shader_source_is_not_resolved_outside_the_shader_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let shaders = dir.path().join("reshade-shaders").join("Shaders");
        std::fs::create_dir_all(&shaders).expect("create shader dir");
        std::fs::write(shaders.join("Real.fx"), "technique").expect("write shader");
        // A real file outside the shader tree, so the escaping forms have
        // something to reach and the assertion is about the refusal rather
        // than about the target happening not to exist.
        std::fs::write(dir.path().join("Outside.fx"), "technique").expect("write outside");

        assert!(
            shader_source_exists(&shaders, "Real.fx"),
            "an ordinary source must still resolve"
        );
        assert!(
            dir.path().join("Outside.fx").is_file(),
            "the escape target must exist for this test to mean anything"
        );
        for source in ["../../Outside.fx", "C:Outside.fx", "/Outside.fx"] {
            assert!(
                !shader_source_exists(&shaders, source),
                "{source} must not resolve"
            );
        }
    }

    #[test]
    fn a_utf8_bom_does_not_hide_the_first_section() {
        let ini = parse_ini("\u{feff}[GENERAL]\nPresetPath=.\\a.ini\n");
        assert_eq!(
            ini_get(&ini, "GENERAL", "PresetPath"),
            Some(".\\a.ini"),
            "a BOM must not push the first section's keys into the headerless bucket"
        );
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

    /// Keys come back in `client_tuning`'s canonical spelling now that its
    /// reader is the one doing the reading. They used to be lower-cased by this
    /// module's own `parse_ini`, so the panel showed `nrlocalstructure` where
    /// the add-on's own overlay says `NRLocalStructure`.
    #[test]
    fn the_tuning_block_is_read() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        let structure = stack
            .tuning
            .iter()
            .find(|t| t.key == "NRLocalStructure")
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

    // ── The two paths ────────────────────────────────────────────────────

    /// The headline regression. Every finding this module can emit about the
    /// feed pipeline describes machinery the direct path does not have, and
    /// firing any of them here would be Kalpa telling a working install — the
    /// one it was debugged against — that it is broken.
    #[test]
    fn the_direct_path_is_detected_and_reports_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.active_path, ActivePath::Direct);
        assert!(!stack.is_empty);
        assert!(!stack.is_disabled, "the injector is live; this is not off");
        assert!(
            stack.findings.is_empty(),
            "the primary user's real working install must be quiet, got {:?}",
            ids(&stack)
        );
    }

    /// Each of these individually would have made the panel wrong about this
    /// install, so each is asserted by name rather than relying on the blanket
    /// "no findings" above.
    #[test]
    fn the_direct_path_raises_no_feed_or_motion_findings() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        for id in [
            "stack-mv-provider-missing",
            "stack-technique-order",
            "stack-feed-technique-off",
            "stack-feed-host-missing",
            "stack-preset-empty",
            "stack-preset-missing",
        ] {
            assert!(
                !ids(&stack).contains(&id),
                "{id} describes the feed path and must not fire on the direct path"
            );
        }
    }

    /// The feed fixture is the older shape and still has to behave exactly as
    /// it did: this whole change is a re-framing, not a replacement.
    #[test]
    fn the_feed_path_is_still_detected_and_still_checked() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.active_path, ActivePath::Feed);
        assert!(stack.findings.is_empty(), "got {:?}", ids(&stack));
        assert_eq!(slot(&stack, StackSlot::Motion).need, SlotNeed::Required);
        assert_eq!(slot(&stack, StackSlot::Preset).need, SlotNeed::Required);

        // And the feed path's checks still bite when the preset is wrong.
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=DLSS5_Feed@DLSS5_Feed.fx,MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx\n",
        );
        let broken = inspect_stack(tmp.path());
        assert!(ids(&broken).contains(&"stack-technique-order"));
    }

    /// A folder with neither add-on is the common case and is not a fault.
    #[test]
    fn no_neural_rendering_addon_is_neither_path() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "eso64.exe", "");
        write(tmp.path(), "dxgi.dll", "");
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.active_path, ActivePath::Neither);
        assert!(stack.findings.is_empty(), "got {:?}", ids(&stack));
    }

    /// `renodx-dlss` is a prefix of `renodx-dlss5`, and the two are opposite
    /// paths. Matching on the whole stem is the only thing stopping every feed
    /// install from being reported as running the direct add-on.
    #[test]
    fn the_feed_addon_is_not_mistaken_for_the_direct_one() {
        assert_eq!(
            addon_stem("renodx-dlss5.addon64").as_deref(),
            Some("renodx-dlss5")
        );
        assert_eq!(
            addon_stem("renodx-dlss.addon64").as_deref(),
            Some("renodx-dlss")
        );
        assert_eq!(addon_stem("renodx-dlss.addon64.off"), None);
        assert_eq!(addon_stem("dxgi.dll"), None);
    }

    /// A name in `DisabledAddons` is a file ReShade will not load, so it cannot
    /// be what defines the live path.
    #[test]
    fn an_addon_switched_off_in_reshade_is_not_the_live_path() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        write(
            tmp.path(),
            "ReShade.ini",
            &DIRECT_RESHADE_INI.replace("DisabledAddons=", "DisabledAddons=renodx-dlss.addon64"),
        );
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.active_path, ActivePath::Neither);
        assert!(ids(&stack).contains(&"stack-addon-disabled"));
        assert!(
            !ids(&stack).contains(&"stack-addon-not-in-dllmain"),
            "an add-on ReShade will not load has no load-order problem to fix"
        );
    }

    /// **Defect 1's regression test, and the most important one here.**
    ///
    /// An earlier `active_path` had three variants and tested Direct first, so
    /// an install running *both* add-ons reported `Direct` — and every
    /// feed-path check, all of which are gated on the path, quietly stood down
    /// over a live feed pipeline. Nothing errored, nothing was reported, and
    /// the feed was broken: the same "gated into silence" failure the whole
    /// path model exists to end.
    #[test]
    fn both_addons_live_is_both_and_does_not_gate_away_the_feed() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        // The direct add-on live *alongside* the feed set. Nobody designed
        // this, and it is not Kalpa's place to pick a winner.
        write(tmp.path(), "renodx-dlss.addon64", "");
        let stack = inspect_stack(tmp.path());
        assert_eq!(stack.active_path, ActivePath::Both);

        // The feed's rows still read as wanted, rather than as a direct-path
        // install's correctly-empty ones.
        assert_eq!(slot(&stack, StackSlot::Motion).need, SlotNeed::Required);
        assert_eq!(slot(&stack, StackSlot::Preset).need, SlotNeed::Required);
        assert_eq!(slot(&stack, StackSlot::Shaders).need, SlotNeed::Required);

        // And the feed's findings still bite. Each of these fired on `Feed`
        // and was silently skipped on the old `Direct`-wins-the-tie verdict.
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "Techniques=DLSS5_Feed@DLSS5_Feed.fx,MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx\n",
        );
        let misordered = inspect_stack(tmp.path());
        assert_eq!(misordered.active_path, ActivePath::Both);
        assert!(
            ids(&misordered).contains(&"stack-technique-order"),
            "got {:?}",
            ids(&misordered)
        );

        write(tmp.path(), "ReShadePreset.ini", "Techniques=\n");
        let empty = inspect_stack(tmp.path());
        assert!(
            ids(&empty).contains(&"stack-preset-empty"),
            "got {:?}",
            ids(&empty)
        );

        std::fs::remove_file(tmp.path().join("ReShadePreset.ini")).unwrap();
        assert!(ids(&inspect_stack(tmp.path())).contains(&"stack-preset-missing"));
    }

    /// The merged liveness rule, from the other end: it asks whether a file is
    /// named *exactly* like the add-on, never whether its suffix is one Kalpa
    /// recognises. A list of the names humans give a switched-off file cannot
    /// be finished — known bug 4 is what a short one costs — so a suffix Kalpa
    /// has never heard of must park an add-on just as thoroughly as `.off`.
    #[test]
    fn a_rename_aside_parks_an_addon_whatever_the_suffix() {
        for suffix in [".off", ".kalpa-off", ".disabled", ".bak", ".old", ".2026"] {
            let tmp = tempfile::tempdir().unwrap();
            direct_path_stack(tmp.path());
            std::fs::rename(
                tmp.path().join("renodx-dlss.addon64"),
                tmp.path().join(format!("renodx-dlss.addon64{suffix}")),
            )
            .unwrap();
            let stack = inspect_stack(tmp.path());

            assert_eq!(
                stack.active_path,
                ActivePath::Neither,
                "renodx-dlss.addon64{suffix} must not count as loaded"
            );
        }
    }

    /// One folder, two panels, and before the merge two rules that could
    /// disagree about it. The tuning panel's verdict and the stack panel's are
    /// now the same function's answer, and this is what says so.
    ///
    /// Two questions, not one, because agreeing on the *path* was never enough:
    /// the stack panel agreed the direct path was live and then read
    /// `[RenoDX.DLSS5]` anyway, so the two surfaces still disagreed about
    /// whether this install had live tuning. The second half pins which section
    /// each panel calls live, section by section, over a file that has all
    /// three.
    #[test]
    fn the_tuning_panel_and_the_stack_panel_agree_on_one_folder() {
        for build in [direct_path_stack as fn(&Path), healthy_stack as fn(&Path)] {
            let tmp = tempfile::tempdir().unwrap();
            build(tmp.path());
            let header = std::fs::read_to_string(tmp.path().join("ReShade.ini")).unwrap();
            write(tmp.path(), "ReShade.ini", &all_three_sections(&header));
            let ini = std::fs::read_to_string(tmp.path().join("ReShade.ini")).unwrap();

            let form = crate::client_tuning::read_form_for_dir(tmp.path(), &ini);
            let stack = inspect_stack(tmp.path());
            assert_eq!(form.active_path, stack.active_path);

            for section in form.sections.iter().filter(|section| section.present) {
                let block = block(&stack, &section.section);
                assert_eq!(
                    block.provenance, section.provenance,
                    "the two panels disagree about [{}]",
                    section.section
                );
                assert_eq!(block.owner, section.owner);
            }

            // And the one the stack panel puts on its rail is one the tuning
            // panel calls live.
            let headline = stack.tuning_section.as_deref().expect("a headline block");
            let matching = form
                .sections
                .iter()
                .find(|section| section.section == headline)
                .expect("the headline is one of the form's sections");
            assert_eq!(matching.provenance, TuningProvenance::Live);
        }
    }

    /// A folder that could not be listed is not a folder with nothing in it.
    /// `Neither` asserts that an empty slot is the correct answer; `Unknown`
    /// says Kalpa could not look, which is the only honest reading and the one
    /// `client_tuning` refuses to write against.
    #[test]
    fn an_unknown_path_says_so_on_every_slot() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        let mut stack = inspect_stack(tmp.path());
        stack.active_path = ActivePath::Unknown;
        stack.tuning_owner = TuningProvenance::Unknown;

        let slots = build_slots(&stack);
        assert_eq!(slots.len(), 8, "always all eight");
        for entry in &slots {
            // The two path-independent rows stay `Required` — ReShade is what
            // the game loads and ESO loads nvngx_dlss.dll itself, and neither
            // fact depends on reading the folder.
            if matches!(entry.slot, StackSlot::Reshade | StackSlot::Sr) {
                assert_eq!(entry.need, SlotNeed::Required);
                continue;
            }
            assert_eq!(
                entry.need,
                SlotNeed::Unknown,
                "{:?} must not claim to know anything: {}",
                entry.slot,
                entry.reason
            );
            assert!(entry.reason.contains("could not read"), "{}", entry.reason);
        }
    }

    // ── The user's own park suffix ───────────────────────────────────────

    /// Bug 4 of five: Kalpa knew only its own suffix, so the user's real
    /// `renodx-dlss5.addon64.off` matched nothing at all — not an add-on, not a
    /// backup, not a park — and the whole DLSS 5 branch was skipped.
    #[test]
    fn the_users_own_off_suffix_is_recognised_as_a_park() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        let names: Vec<&str> = stack
            .user_parked
            .iter()
            .map(|file| file.file_name.as_str())
            .collect();
        assert!(names.contains(&"renodx-dlss5.addon64.off"), "{names:?}");
        assert!(names.contains(&"dlss5-feed.addon64.off"), "{names:?}");
        assert!(names.contains(&"dlss5-feed.cfg.off"), "{names:?}");

        let feed = stack
            .user_parked
            .iter()
            .find(|file| file.restores == "dlss5-feed.addon64")
            .expect("a park says what it restores");
        assert_eq!(feed.parked_by, ParkedBy::User);
        assert_eq!(feed.suffix, ".off");
        assert!(!feed.target_present, "nothing occupies the live name");

        // A parked file is not a live add-on, and is not one of the user's
        // displaced originals either.
        assert!(!stack.items.iter().any(|i| i.file_name.ends_with(".off")));
        assert!(!stack
            .preserved_originals
            .iter()
            .any(|o| o.file_name.ends_with(".off")));
    }

    /// The distinction that keeps the toggle honest. `client_toggle` plans an
    /// unpark for every entry in `parked`, and the panel reads "switched on"
    /// from that list being empty — both are statements about Kalpa's own work,
    /// and a file the user renamed by hand must not join them.
    #[test]
    fn a_user_park_is_not_kalpas_park() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        assert!(
            stack.parked.is_empty(),
            "Kalpa parked nothing here; got {:?}",
            stack.parked
        );
        assert!(!stack.user_parked.is_empty());
        assert!(!stack.is_disabled);
    }

    /// A user-parked *injector* still must not read as Kalpa having switched
    /// the stack off — `stack-disabled` says "Kalpa has parked … and put the
    /// game's own files back", which would be a claim about work Kalpa did not
    /// do. The missing injector reports through its own finding instead.
    #[test]
    fn a_user_parked_injector_is_not_a_kalpa_disable() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        std::fs::rename(tmp.path().join("dxgi.dll"), tmp.path().join("dxgi.dll.off")).unwrap();
        let stack = inspect_stack(tmp.path());

        assert!(!stack.is_disabled);
        assert!(!ids(&stack).contains(&"stack-disabled"));
        assert!(ids(&stack).contains(&"stack-no-injector"));
    }

    /// Kalpa's write-side invariant is unchanged: it parks as, and removes,
    /// exactly one suffix. Recognising `.off` is a read-side act only, and
    /// `.off` is not a backup suffix either — a backup holds a displaced
    /// original, a park holds a live file switched off.
    #[test]
    fn recognising_the_users_suffix_does_not_make_it_kalpas() {
        assert!(!USER_PARK_SUFFIXES.contains(&PARKED_SUFFIX));
        for suffix in USER_PARK_SUFFIXES {
            assert!(!BACKUP_SUFFIXES.contains(&suffix), "{suffix}");
            assert!(!PARKED_SUFFIX.ends_with(suffix), "{suffix}");
        }
        assert_eq!(
            park_target("dxgi.dll.kalpa-off").map(|(by, ..)| by),
            Some(ParkedBy::Kalpa)
        );
        assert_eq!(
            park_target("dlss5-feed.addon64.off").map(|(by, ..)| by),
            Some(ParkedBy::User)
        );
        // Backups stay backups.
        assert_eq!(park_target("nvngx_dlss.dll.disabled-bak"), None);
        // `.off` on its own is a whole file name, not a suffix on one.
        assert_eq!(park_target(".off"), None);
    }

    /// A folder holding only switched-off files is not an empty folder.
    #[test]
    fn a_folder_of_only_user_parked_files_is_not_empty() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "eso64.exe", "");
        write(tmp.path(), "dlss5-feed.addon64.off", "");
        let stack = inspect_stack(tmp.path());

        assert!(!stack.is_empty);
        assert_eq!(stack.user_parked.len(), 1);
    }

    // ── LoadFromDllMain ──────────────────────────────────────────────────

    /// Bug 1 of five, and the highest-value one: an add-on present, enabled,
    /// visible in the overlay, and doing nothing, with a one-line fix nobody
    /// detects.
    #[test]
    fn an_early_hook_addon_missing_from_load_from_dll_main_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        write(
            tmp.path(),
            "ReShade.ini",
            &DIRECT_RESHADE_INI.replace("LoadFromDllMain=renodx-dlss.addon64\n", ""),
        );
        let stack = inspect_stack(tmp.path());

        let found = stack
            .findings
            .iter()
            .find(|f| f.id == "stack-addon-not-in-dllmain")
            .unwrap_or_else(|| panic!("expected the load-order finding, got {:?}", ids(&stack)));
        assert_eq!(found.level, HealthLevel::Danger);
        // The literal line is the whole fix, so it has to be in the copy.
        assert!(
            found.detail.contains("LoadFromDllMain=renodx-dlss.addon64"),
            "the finding must carry the exact ini line: {}",
            found.detail
        );
        assert!(
            found.detail.contains("[ADDON]"),
            "and say which section it goes under: {}",
            found.detail
        );
        // ReShade rewrites ReShade.ini on exit, so an edit made with the game
        // running is silently thrown away. Omitting this sends the user round
        // the loop twice.
        assert!(
            found.detail.contains("Close ESO"),
            "and say to close the game first: {}",
            found.detail
        );
    }

    #[test]
    fn an_addon_already_listed_in_load_from_dll_main_is_quiet() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.load_from_dll_main, vec!["renodx-dlss.addon64"]);
        assert!(!ids(&stack).contains(&"stack-addon-not-in-dllmain"));
    }

    /// An existing list is appended to, not replaced. Handing the user a line
    /// that drops their other early-loading add-on would be a fix that breaks
    /// something else.
    #[test]
    fn the_suggested_line_keeps_what_is_already_listed() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        write(
            tmp.path(),
            "ReShade.ini",
            &DIRECT_RESHADE_INI.replace(
                "LoadFromDllMain=renodx-dlss.addon64",
                "LoadFromDllMain=something-else.addon64",
            ),
        );
        let stack = inspect_stack(tmp.path());

        let detail = &stack
            .findings
            .iter()
            .find(|f| f.id == "stack-addon-not-in-dllmain")
            .expect("still missing from the list")
            .detail;
        assert!(
            detail.contains("LoadFromDllMain=something-else.addon64,renodx-dlss.addon64"),
            "{detail}"
        );
    }

    /// Scope. Firing this at every add-on in the folder would drown the one
    /// case it exists for, and nobody has watched the feed add-ons fail this
    /// way — see [`EARLY_HOOK_ADDON_STEMS`].
    #[test]
    fn ordinary_addons_are_not_asked_to_load_from_dll_main() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(tmp.path(), "some-overlay.addon64", "");
        let stack = inspect_stack(tmp.path());

        assert!(
            !ids(&stack).contains(&"stack-addon-not-in-dllmain"),
            "the feed fixture lists nothing in LoadFromDllMain and must stay quiet, got {:?}",
            ids(&stack)
        );
    }

    // ── An empty technique list ──────────────────────────────────────────

    /// Bug 5 of five: `stack-preset-missing` only ever asked whether the file
    /// existed, so a preset enabling nothing passed every check.
    #[test]
    fn an_empty_technique_list_is_reported_on_the_feed_path() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(tmp.path(), "ReShadePreset.ini", "Techniques=\n");
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.active_path, ActivePath::Feed);
        assert!(
            ids(&stack).contains(&"stack-preset-empty"),
            "got {:?}",
            ids(&stack)
        );
        // One finding for one fact. `stack-feed-technique-off` says a subset of
        // the same thing and stands down.
        assert!(!ids(&stack).contains(&"stack-feed-technique-off"));
    }

    /// A preset file with no `Techniques=` line at all is the same state, and
    /// is exactly what the primary user's `ReShadePreset.ini` looks like.
    #[test]
    fn an_absent_technique_line_is_the_same_as_an_empty_one() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShadePreset.ini",
            "PreprocessorDefinitions=\n",
        );
        let stack = inspect_stack(tmp.path());
        assert!(ids(&stack).contains(&"stack-preset-empty"));
    }

    /// And the other half of the rule: on the direct path an empty
    /// `Techniques=` is the *correct* configuration. Reporting it would be the
    /// same class of error in the opposite direction.
    #[test]
    fn an_empty_technique_list_is_correct_on_the_direct_path() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        write(tmp.path(), "ReShadePreset.ini", "Techniques=\n");
        let stack = inspect_stack(tmp.path());

        assert!(
            !ids(&stack).contains(&"stack-preset-empty"),
            "{:?}",
            ids(&stack)
        );
        assert!(!ids(&stack).contains(&"stack-preset-missing"));
    }

    /// A `PresetPath` pointing at nothing is a real problem on the feed path
    /// and a non-event on the direct one, where no preset is needed at all.
    #[test]
    fn a_missing_preset_file_only_matters_off_the_direct_path() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        std::fs::remove_file(tmp.path().join("ReShadePreset.ini")).unwrap();
        assert!(!ids(&inspect_stack(tmp.path())).contains(&"stack-preset-missing"));

        let feed = tempfile::tempdir().unwrap();
        healthy_stack(feed.path());
        std::fs::remove_file(feed.path().join("ReShadePreset.ini")).unwrap();
        assert!(ids(&inspect_stack(feed.path())).contains(&"stack-preset-missing"));
    }

    // ── The need axis ────────────────────────────────────────────────────

    /// The point of the whole exercise: an empty motion-vector slot on the
    /// direct path is a deliberate, correct state and has to read as one — not
    /// as a hole, and not as an Info-level finding, which is the same mistake
    /// with a colour on it.
    #[test]
    fn the_direct_path_frames_its_empty_slots_as_deliberate() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.slots.len(), 8, "every slot is always answered");
        for want in [StackSlot::Motion, StackSlot::Preset] {
            let entry = slot(&stack, want);
            assert_eq!(entry.need, SlotNeed::NotOnThisPath, "{:?}", entry.slot);
            assert!(
                !entry.reason.is_empty(),
                "{:?} needs a sentence",
                entry.slot
            );
        }
        assert_eq!(slot(&stack, StackSlot::Reshade).need, SlotNeed::Required);
        assert_eq!(slot(&stack, StackSlot::Nr).need, SlotNeed::Required);
        assert_eq!(slot(&stack, StackSlot::Sr).need, SlotNeed::Required);
        assert_eq!(slot(&stack, StackSlot::Addons).need, SlotNeed::Required);
    }

    /// A shader is found whatever case its name is written in.
    ///
    /// This test passes for free on Windows and macOS, whose filesystems fold
    /// case themselves — it earns its keep on Linux, where CI caught
    /// `find_shader_source` failing to see `MartysMods_LAUNCHPAD.fx` through
    /// the lowercase [`LAUNCHPAD_SOURCE`] and quietly downgrading the Shaders
    /// slot. Kalpa runs ESO through Proton there, so it is a real install.
    #[test]
    fn a_shader_is_found_whatever_case_its_name_is_written_in() {
        let tmp = tempfile::tempdir().unwrap();
        let shaders = tmp.path().join("reshade-shaders").join("Shaders");
        std::fs::create_dir_all(&shaders).unwrap();
        std::fs::write(shaders.join("MartysMods_LAUNCHPAD.fx"), "").unwrap();

        assert!(shader_source_exists(&shaders, LAUNCHPAD_SOURCE));
        assert!(shader_source_exists(&shaders, "MARTYSMODS_LAUNCHPAD.FX"));
        assert!(!shader_source_exists(&shaders, "NotHere.fx"));

        // The nested layout shader packs actually ship in.
        let nested = shaders.join("MartysMods");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("MartysMods_LAUNCHPAD.fx"), "").unwrap();
        assert!(shader_source_exists(&shaders, LAUNCHPAD_SOURCE));

        // The traversal guard is not weakened by the case-folding fallback.
        assert!(!shader_source_exists(
            &shaders,
            "../MartysMods_LAUNCHPAD.fx"
        ));
    }

    /// Installed-but-unused is shown as exactly that, **with the reason to keep
    /// it**. Kalpa can refetch none of these — LaunchPad's licence forbids
    /// redistribution and the feed add-ons come from a Discord — so "you could
    /// delete this" would be advice Kalpa cannot undo.
    #[test]
    fn installed_but_unused_things_come_with_a_reason_to_keep_them() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        let shaders = slot(&stack, StackSlot::Shaders);
        assert_eq!(shaders.need, SlotNeed::InstalledUnused);
        let keep = shaders
            .keep_because
            .as_deref()
            .expect("a reason to keep it");
        assert!(keep.contains("LaunchPad"), "{keep}");

        // The motion slot names it too: that is where a user looking at an
        // empty row would otherwise wonder what LaunchPad is still doing here.
        let motion_keep = slot(&stack, StackSlot::Motion)
            .keep_because
            .as_deref()
            .expect("a reason to keep it");
        assert!(motion_keep.contains("LaunchPad"), "{motion_keep}");

        // And the parked feed add-ons are named as the fallback they are.
        let addons_keep = slot(&stack, StackSlot::Addons)
            .keep_because
            .as_deref()
            .expect("the parked add-ons need explaining");
        assert!(
            addons_keep.contains("renodx-dlss5.addon64"),
            "{addons_keep}"
        );
        assert!(addons_keep.contains("dlss5-feed.addon64"), "{addons_keep}");
        assert!(addons_keep.contains("Discord"), "{addons_keep}");
    }

    // ── Tuning provenance ────────────────────────────────────────────────

    /// Bug 3 of five. `[RenoDX.DLSS5]` belongs to the *parked*
    /// `renodx-dlss5.addon64`; the live add-on writes `[RENODX-DLSS]`. A fossil
    /// presented as live tuning misled both the user and the agent diagnosing
    /// this install, so the provenance is data rather than a UI guess.
    #[test]
    fn a_tuning_block_left_by_a_parked_addon_is_marked_as_a_fossil() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        write(
            tmp.path(),
            "ReShade.ini",
            &format!("{DIRECT_RESHADE_INI}\n[RenoDX.DLSS5]\nNeuralUplift=0\n"),
        );
        let stack = inspect_stack(tmp.path());

        // The direct add-on has saved nothing here, so the feed's block is all
        // there is — and it is still shown, still named, and still labelled as
        // not in force. Compare
        // `the_direct_path_shows_its_own_live_tuning_not_the_feed_fossil`,
        // where a live `[RENODX-DLSS]` outranks it.
        assert_eq!(stack.tuning.len(), 1);
        assert_eq!(stack.tuning_owner, TuningProvenance::Fossil);
        assert_eq!(stack.tuning_section.as_deref(), Some("RenoDX.DLSS5"));
        assert_eq!(stack.tuning_blocks.len(), 1);

        let tuning = slot(&stack, StackSlot::Tuning);
        assert_eq!(tuning.need, SlotNeed::InstalledUnused);
        assert!(
            tuning.reason.contains("renodx-dlss5.addon64"),
            "{}",
            tuning.reason
        );
    }

    /// On the feed path the same section is live, and must not be disclaimed.
    #[test]
    fn the_tuning_block_is_live_when_its_addon_is() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.tuning_owner, TuningProvenance::Live);
        assert_eq!(slot(&stack, StackSlot::Tuning).need, SlotNeed::Required);
    }

    /// A live path that has simply never saved a block is not a gap.
    ///
    /// This fixture has the direct add-on live and no RenoDX section in
    /// `ReShade.ini` at all, so the row names `[RENODX-DLSS]` and the add-on
    /// that would write it. The provenance flipped from `Fossil` to `Live` when
    /// tuning started following the active path: with no section present,
    /// provenance answers "is the add-on that would write one loaded?", and on
    /// the direct path it is. It read `Fossil` before only because the field
    /// was hardcoded to ask about `renodx-dlss5.addon64` — the parked one.
    #[test]
    fn no_tuning_block_on_the_direct_path_is_not_a_gap() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        let stack = inspect_stack(tmp.path());

        assert!(stack.tuning.is_empty());
        assert!(stack.tuning_blocks.is_empty());
        // Absence is `tuning_section`, not a provenance variant: there is no
        // section here, *and* the add-on that would own one is loaded, and
        // those are two separate facts the panel is entitled to state
        // together. See [`TuningProvenance`].
        assert_eq!(stack.tuning_section, None);
        assert_eq!(stack.tuning_owner, TuningProvenance::Live);
        let tuning = slot(&stack, StackSlot::Tuning);
        assert_eq!(tuning.need, SlotNeed::NotOnThisPath);
        assert!(tuning.reason.contains("RENODX-DLSS"), "{}", tuning.reason);
        assert!(
            tuning.reason.contains("renodx-dlss.addon64"),
            "{}",
            tuning.reason
        );
    }

    // ── Tuning follows the active path ───────────────────────────────────

    /// The keys the direct add-on writes to `[RENODX-DLSS]`.
    ///
    /// The *count* is the load-bearing part and it is the real one: the primary
    /// user's `ReShade.ini` has 22 keys here and 8 more in
    /// `[RENODX-DLSS-preset1]`, none of which this module read before. The
    /// spellings are representative — `renodx-dlss.addon64` is closed-source,
    /// and Kalpa shows these read-only precisely because nobody has verified
    /// what each one does.
    const DIRECT_KEYS: [&str; 22] = [
        "DirectNeuralRendering",
        "DirectNeuralRenderingIntensity",
        "DirectNeuralRenderingStyle",
        "DirectNeuralRenderingEncoding",
        "DirectNeuralRenderingDiffuseWhiteNits",
        "DirectNeuralRenderingPeakNits",
        "DirectNeuralRenderingAutoMask",
        "DirectNeuralRenderingUICorrection",
        "DirectNeuralRenderingLocalTone",
        "DirectNeuralRenderingLocalStructure",
        "DirectNeuralRenderingSkinStructure",
        "DirectNeuralRenderingColorStrength",
        "DirectNeuralRenderingTransferStrength",
        "DirectNeuralRenderingDepthMode",
        "DirectNeuralRenderingToggleKey",
        "DLSSQualityMode",
        "DLSSPreset",
        "StreamlinePeakNits",
        "StreamlineDiffuseWhiteNits",
        "ToneMapType",
        "ColorGradeExposure",
        "SwapChainCustomColorSpace",
    ];

    const PRESET_KEYS: [&str; 8] = [
        "DirectNeuralRenderingIntensity",
        "DirectNeuralRenderingStyle",
        "DirectNeuralRenderingLocalTone",
        "DirectNeuralRenderingLocalStructure",
        "DLSSQualityMode",
        "ToneMapType",
        "ColorGradeExposure",
        "ColorGradeSaturation",
    ];

    /// Sixteen keys of `[RenoDX.DLSS5]`, all of them from `client_tuning`'s
    /// verified field table, because on the feed path this is the one section
    /// Kalpa may write and those typed fields are how it does it.
    const FEED_KEYS: [&str; 16] = [
        "NeuralUplift",
        "NREnableUpscaling",
        "NRPreset",
        "NRStyle",
        "NRIntensity",
        "NRLocalTone",
        "NRLocalStructure",
        "NRSkinStructure",
        "NRAutoMask",
        "NRUICorrection",
        "NRPaperWhiteScale",
        "NRTransferStrength",
        "NRColorStrength",
        "NRToggleKey",
        "NRScreenshotKey",
        "NRDepthMode",
    ];

    /// A `ReShade.ini` carrying all three RenoDX sections at once, which is the
    /// primary user's real file: they ran the feed path first, switched to the
    /// direct add-on, and the feed's saved block stayed behind.
    fn all_three_sections(header: &str) -> String {
        let mut out = header.to_string();
        for (name, keys) in [
            ("RENODX-DLSS", &DIRECT_KEYS[..]),
            ("RENODX-DLSS-preset1", &PRESET_KEYS[..]),
            ("RenoDX.DLSS5", &FEED_KEYS[..]),
        ] {
            out.push_str(&format!("\n[{name}]\n"));
            for (index, key) in keys.iter().enumerate() {
                out.push_str(&format!("{key}={index}\n"));
            }
        }
        out
    }

    fn block<'a>(stack: &'a ClientStack, section: &str) -> &'a TuningBlock {
        stack
            .tuning_blocks
            .iter()
            .find(|block| block.section == section)
            .unwrap_or_else(|| panic!("[{section}] should be carried: {:?}", stack.tuning_blocks))
    }

    /// The primary user's real shape, and the bug in one test: 22 live keys in
    /// `[RENODX-DLSS]`, 8 more in `[RENODX-DLSS-preset1]`, and a 16-key
    /// `[RenoDX.DLSS5]` left behind by the add-on they parked. The panel used
    /// to read only the last of those and call it "history, not this install's
    /// live tuning" — true of that block, and a silent claim that this install
    /// had no live tuning at all while 30 keys of it sat unread.
    #[test]
    fn the_direct_path_shows_its_own_live_tuning_not_the_feed_fossil() {
        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        write(
            tmp.path(),
            "ReShade.ini",
            &all_three_sections(DIRECT_RESHADE_INI),
        );
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.active_path, ActivePath::Direct);
        assert_eq!(stack.tuning_section.as_deref(), Some("RENODX-DLSS"));
        assert_eq!(stack.tuning_owner, TuningProvenance::Live);
        assert_eq!(stack.tuning.len(), DIRECT_KEYS.len());

        // Nothing is dropped to reach that headline: all three blocks are
        // carried, each labelled, and the preset family is live too.
        assert_eq!(stack.tuning_blocks.len(), 3);
        assert_eq!(
            block(&stack, "RENODX-DLSS").provenance,
            TuningProvenance::Live
        );
        let preset = block(&stack, "RENODX-DLSS-preset1");
        assert_eq!(preset.provenance, TuningProvenance::Live);
        assert_eq!(preset.values.len(), PRESET_KEYS.len());

        // And the fossil keeps every one of its values. They are the user's
        // only copy of the feed path's settings — see [`TuningBlock`].
        let fossil = block(&stack, "RenoDX.DLSS5");
        assert_eq!(fossil.provenance, TuningProvenance::Fossil);
        assert_eq!(fossil.values.len(), FEED_KEYS.len());
        assert_eq!(fossil.owner, crate::client_tuning::FEED_NR_ADDON);

        let tuning = slot(&stack, StackSlot::Tuning);
        assert_eq!(
            tuning.need,
            SlotNeed::Required,
            "live tuning is not an unused install: {}",
            tuning.reason
        );
        assert!(tuning.reason.contains("[RENODX-DLSS]"), "{}", tuning.reason);
        assert!(
            tuning.reason.contains("renodx-dlss.addon64"),
            "{}",
            tuning.reason
        );
        assert!(
            !tuning.reason.contains("history"),
            "the fossil's disclaimer must not be the live row's sentence: {}",
            tuning.reason
        );
        // The fossil is still spoken for, on the axis meant for it.
        let keep = tuning
            .keep_because
            .as_deref()
            .expect("the fossil needs saying");
        assert!(keep.contains("[RenoDX.DLSS5]"), "{keep}");
    }

    /// The feed path, given the same three-section file: now the direct add-on
    /// is the parked one and its two blocks are the fossils.
    #[test]
    fn the_feed_path_makes_its_own_section_the_live_one() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(
            tmp.path(),
            "ReShade.ini",
            &all_three_sections(REAL_RESHADE_INI),
        );
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.active_path, ActivePath::Feed);
        assert_eq!(stack.tuning_section.as_deref(), Some("RenoDX.DLSS5"));
        assert_eq!(stack.tuning_owner, TuningProvenance::Live);
        assert_eq!(stack.tuning.len(), FEED_KEYS.len());
        assert_eq!(
            block(&stack, "RENODX-DLSS").provenance,
            TuningProvenance::Fossil
        );
        assert_eq!(slot(&stack, StackSlot::Tuning).need, SlotNeed::Required);
    }

    /// Both add-ons loaded: **every** section is live, because both add-ons
    /// will read theirs. The headline names the direct path's block, which is a
    /// choice about what fits on a one-line rail and nothing more — no block is
    /// labelled a fossil here and `tuning_blocks` carries all three. Picking a
    /// winner and disclaiming the other would be Kalpa calling live
    /// configuration history, which is the bug this row was fixed for.
    #[test]
    fn both_addons_live_makes_every_tuning_block_live() {
        let tmp = tempfile::tempdir().unwrap();
        healthy_stack(tmp.path());
        write(tmp.path(), "renodx-dlss.addon64", "");
        write(
            tmp.path(),
            "ReShade.ini",
            &all_three_sections(REAL_RESHADE_INI),
        );
        let stack = inspect_stack(tmp.path());

        assert_eq!(stack.active_path, ActivePath::Both);
        assert!(
            stack
                .tuning_blocks
                .iter()
                .all(|block| block.provenance == TuningProvenance::Live),
            "{:?}",
            stack.tuning_blocks
        );
        assert_eq!(stack.tuning_section.as_deref(), Some("RENODX-DLSS"));
        assert_eq!(stack.tuning_owner, TuningProvenance::Live);

        let tuning = slot(&stack, StackSlot::Tuning);
        assert_eq!(tuning.need, SlotNeed::Required);
        assert_eq!(
            tuning.keep_because, None,
            "nothing here is a fossil to keep: {tuning:?}"
        );
    }

    /// An unreadable folder may not claim anything is in force. The provenance
    /// comes from `client_tuning`'s reader, so this pins it at the source as
    /// well as on the row.
    #[test]
    fn an_unknown_path_never_calls_tuning_live() {
        let ini = all_three_sections(DIRECT_RESHADE_INI);
        let form = read_form(&ini, "C:/client", ActivePath::Unknown, Vec::new());
        assert!(
            form.sections
                .iter()
                .all(|section| section.provenance == TuningProvenance::Unknown),
            "no section may be Live when Kalpa could not look"
        );

        let tmp = tempfile::tempdir().unwrap();
        direct_path_stack(tmp.path());
        write(tmp.path(), "ReShade.ini", &ini);
        let mut stack = inspect_stack(tmp.path());
        stack.active_path = ActivePath::Unknown;
        stack.tuning_owner = TuningProvenance::Unknown;
        for block in &mut stack.tuning_blocks {
            block.provenance = TuningProvenance::Unknown;
        }

        let slots = build_slots(&stack);
        let tuning = slots
            .iter()
            .find(|entry| entry.slot == StackSlot::Tuning)
            .expect("every slot is always present");
        assert_eq!(tuning.need, SlotNeed::Unknown);
        assert!(
            tuning.reason.contains("could not read"),
            "{}",
            tuning.reason
        );
    }
}
