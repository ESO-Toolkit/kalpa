//! Native-upload coverage gate — the "never ship a wrong report" guarantee.
//!
//! We reimplement ESO Logs' encoder incrementally, so at any moment we can
//! correctly encode *some* of the log line types but not all. This module
//! decides, for a given raw log, whether the native path may be used at all:
//!
//! * Every raw line type our encoder has **proven** it reproduces byte-exactly
//!   (validated by a golden-pair differential test) is in [`PROVEN_LINE_TYPES`].
//! * [`assess`] scans a log's line types. If *all* of them are proven, native
//!   upload is allowed. If *any* unproven type appears, native is refused and the
//!   caller falls back to the official uploader.
//!
//! This is deliberately conservative: an unknown or not-yet-proven line type
//! means "we can't guarantee a byte-correct report," so we don't risk it. The
//! result is the core UX guarantee — **native upload only ever produces output
//! identical to the official uploader's, or it declines and hands off.** A user
//! never receives a silently-corrupted report from the native path.
//!
//! As the encoder grows (each line type added *with* a passing golden diff), its
//! type joins [`PROVEN_LINE_TYPES`] and more logs qualify for native upload.

use std::collections::BTreeSet;

/// Raw `Encounter.log` line types the native path is cleared to handle. [`assess`]
/// returns [`Coverage::Native`] only when **every** line type in a log is in this
/// list; any other type forces the official-uploader fallback.
///
/// **Currently empty — and intentionally so until a live round-trip confirms the
/// server accepts our segment.** The events encoder ([`super::events`]) now
/// structurally reproduces every line type (it rebuilds the golden sample segment
/// byte-for-byte except the optional `A` cast-ref, and assembles a full ~49k-event
/// raid capture with zero malformed lines — see [`STRUCTURALLY_READY_LINE_TYPES`]).
///
/// **CONFIRMED RENDERING (2026-06-19, owner-verified):** a native upload of a real
/// dungeon log (all the combat types below) was accepted by ESO Logs and rendered
/// a complete report. Native upload is therefore enabled for any log whose line
/// types are all in this set. The blocker had been a transport bug (the
/// `add-report-segment` request sent a zero-width `startTime`/`endTime`), not the
/// encoder — see [`super::client`].
///
/// The trial markers (`BEGIN_TRIAL`/`END_TRIAL`/`TRIAL_INIT`) are included so
/// trial/raid logs also route native: `END_TRIAL` emits a code-55 line and the
/// other two are no-op state lines (same as the reference). Any line type NOT in
/// this set still forces fallback to the official uploader (graceful degradation),
/// so a future game-patch event type can never corrupt a report.
///
/// This is belt-and-suspenders with [`super::format::FORMAT_VERSION_CONFIRMED`]
/// (`true`): both must agree for native to run.
pub const PROVEN_LINE_TYPES: &[&str] = &[
    // Always-present combat/state types (17) — every real log contains these, and
    // a dungeon log of exactly these rendered correctly (owner-confirmed).
    "BEGIN_LOG",
    "END_LOG",
    "BEGIN_COMBAT",
    "END_COMBAT",
    "ZONE_CHANGED",
    "MAP_CHANGED",
    "UNIT_ADDED",
    "UNIT_CHANGED",
    "UNIT_REMOVED",
    "ABILITY_INFO",
    "EFFECT_INFO",
    "EFFECT_CHANGED",
    "BEGIN_CAST",
    "END_CAST",
    "COMBAT_EVENT",
    "HEALTH_REGEN",
    "PLAYER_INFO",
    // Trial markers (3) — present in every raid/trial log. END_TRIAL → code 55;
    // BEGIN_TRIAL/TRIAL_INIT are no-op state lines. Included so trials route native.
    "BEGIN_TRIAL",
    "END_TRIAL",
    "TRIAL_INIT",
    // Infinite Archive markers (6) — present in IA logs. The official uploader emits
    // NO segment event for ANY of these (golden-confirmed 2026-06-24, Archon report
    // M6t4mDzFWyqraPdN): all six are pure no-op state markers, and IA fights encode
    // with the standard combat codes above. Included so IA logs route native.
    "ENDLESS_DUNGEON_BEGIN",
    "ENDLESS_DUNGEON_END",
    "ENDLESS_DUNGEON_STAGE_END",
    "ENDLESS_DUNGEON_BUFF_ADDED",
    "ENDLESS_DUNGEON_BUFF_REMOVED",
    "ENDLESS_DUNGEON_INIT",
];

/// Line types whose [`super::events`] encoder is **built and structurally tested**
/// — every emitted line has the correct field layout, a real allocated subordinal
/// `A`, well-formed masks/state blocks, and the segment's event count is
/// self-consistent. This is *not* the ship gate ([`PROVEN_LINE_TYPES`] is) — it is
/// the honest readiness report: these are one live round-trip away from native.
///
/// Proven by [`super::events`] tests: the golden sample (codes 41/51/5/7/10/12/15/16)
/// is reproduced byte-for-byte except the optional `A` cast-ref, and the full
/// combat capture exercises the damage/heal/dot/power/player-info/regen/combat-
/// boundary path (codes 1/2/3/26/44/4/52/53) with zero malformed lines.
///
/// The state-only line types (`UNIT_ADDED`/`UNIT_CHANGED`/`UNIT_REMOVED`/
/// `ABILITY_INFO`/`EFFECT_INFO`/`END_CAST`/`BEGIN_LOG`/`END_LOG`) emit no segment
/// event — they are consumed to maintain parser state (actor table, effect types,
/// timestamp offset). They are "handled" (a log containing them assembles cleanly),
/// so they belong here.
///
/// **Not yet listed:** the `*_TRIAL*` markers — the combat capture is a dungeon, so
/// trial-marker handling is unverified against a real capture. They are added once a
/// trial-log capture confirms they assemble cleanly (most likely also as
/// no-segment-event state lines, but verified rather than assumed).
///
/// **To enable native upload (owner-run):** upload a segment built from a short
/// real combat log to a TEST report via the native client; if ESO Logs accepts and
/// renders it, set [`super::format::FORMAT_VERSION_CONFIRMED`] = `true` and copy the
/// confirmed subset of these into [`PROVEN_LINE_TYPES`]. The gate is all-or-nothing
/// per log, so a log is native only once *every* type it contains is in
/// `PROVEN_LINE_TYPES`.
pub const STRUCTURALLY_READY_LINE_TYPES: &[&str] = &[
    "BEGIN_LOG",
    "END_LOG",
    "BEGIN_COMBAT",
    "END_COMBAT",
    "ZONE_CHANGED",
    "MAP_CHANGED",
    "UNIT_ADDED",
    "UNIT_CHANGED",
    "UNIT_REMOVED",
    "ABILITY_INFO",
    "EFFECT_INFO",
    "EFFECT_CHANGED",
    "BEGIN_CAST",
    "END_CAST",
    "COMBAT_EVENT",
    "HEALTH_REGEN",
    "PLAYER_INFO",
    // Trial markers: END_TRIAL → code 55; BEGIN_TRIAL/TRIAL_INIT → no-op state.
    "BEGIN_TRIAL",
    "END_TRIAL",
    "TRIAL_INIT",
    // Infinite Archive markers: all six emit no segment event (golden-confirmed).
    "ENDLESS_DUNGEON_BEGIN",
    "ENDLESS_DUNGEON_END",
    "ENDLESS_DUNGEON_STAGE_END",
    "ENDLESS_DUNGEON_BUFF_ADDED",
    "ENDLESS_DUNGEON_BUFF_REMOVED",
    "ENDLESS_DUNGEON_INIT",
];

/// The **complete, closed** set of ESO `Encounter.log` event types — the finite
/// target the encoder must cover for native upload to handle ~all real raid
/// logs. Established by an event-type census across real logs spanning 70K → 8.4M
/// lines (a 120× size range, a year of game patches): a closed set of 20 base
/// combat/trial types that does **not** grow with log size, plus the 6 Infinite
/// Archive `ENDLESS_DUNGEON_*` markers (a distinct game mode) = **26 total**. There
/// is no long tail — two types (`EFFECT_CHANGED` + `COMBAT_EVENT`) are 92–95% of
/// every file, so the cost is concentrated.
///
/// 17 appear in 100% of logs (the mandatory floor); the 3 `*_TRIAL*` markers appear
/// in every actual trial/raid; the 6 `ENDLESS_DUNGEON_*` markers appear in every
/// Infinite Archive run (and emit no segment event — golden-confirmed). Proving all
/// 26 reaches effectively 100% native coverage. A *future* game patch could add a new
/// type — that is the one scenario [`assess`] still falls back on, and it does so
/// automatically (any type not in [`PROVEN_LINE_TYPES`] → fallback), so a novel
/// type degrades gracefully to the official uploader instead of corrupting.
pub const TARGET_LINE_TYPES: &[&str] = &[
    // Always-present (17) — every real log contains all of these.
    "BEGIN_LOG",
    "END_LOG",
    "BEGIN_COMBAT",
    "END_COMBAT",
    "ZONE_CHANGED",
    "MAP_CHANGED",
    "UNIT_ADDED",
    "UNIT_CHANGED",
    "UNIT_REMOVED",
    "ABILITY_INFO",
    "EFFECT_INFO",
    "EFFECT_CHANGED",
    "BEGIN_CAST",
    "END_CAST",
    "COMBAT_EVENT",
    "HEALTH_REGEN",
    "PLAYER_INFO",
    // Trial markers (3) — present in every actual raid/trial log.
    "BEGIN_TRIAL",
    "END_TRIAL",
    "TRIAL_INIT",
    // Infinite Archive markers (6) — present in every IA log.
    "ENDLESS_DUNGEON_BEGIN",
    "ENDLESS_DUNGEON_END",
    "ENDLESS_DUNGEON_STAGE_END",
    "ENDLESS_DUNGEON_BUFF_ADDED",
    "ENDLESS_DUNGEON_BUFF_REMOVED",
    "ENDLESS_DUNGEON_INIT",
];

/// Whether the native path may handle a given log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// Every line type in the log is proven — native upload is safe to use.
    Native,
    /// At least one line type is not yet proven — fall back to the official
    /// uploader. Carries the offending types (sorted, capped) for diagnostics.
    Fallback { unproven: Vec<String> },
}

impl Coverage {
    pub fn is_native(&self) -> bool {
        matches!(self, Coverage::Native)
    }
}

/// The line type token of a raw ESO log line (`<ms>,<TYPE>,...`) — field index 1.
fn line_type(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    line.split(',').nth(1).map(str::trim)
}

/// Assess whether a raw log (as an iterator of lines) is fully within proven
/// coverage. Returns [`Coverage::Native`] only if every line type present is in
/// [`PROVEN_LINE_TYPES`]; otherwise [`Coverage::Fallback`] with the unproven set.
///
/// Streaming-friendly: takes any line iterator, so a caller can feed a scanner's
/// lines without materializing the whole multi-GB log.
pub fn assess<'a, I>(lines: I) -> Coverage
where
    I: IntoIterator<Item = &'a str>,
{
    let proven: BTreeSet<&str> = PROVEN_LINE_TYPES.iter().copied().collect();
    let mut unproven: BTreeSet<String> = BTreeSet::new();
    for line in lines {
        if let Some(t) = line_type(line) {
            if !proven.contains(t) {
                unproven.insert(t.to_string());
            }
        }
    }
    if unproven.is_empty() {
        Coverage::Native
    } else {
        // Cap the reported set so a pathological log can't produce a huge vec.
        Coverage::Fallback {
            unproven: unproven.into_iter().take(32).collect(),
        }
    }
}

/// How many of the [`TARGET_LINE_TYPES`] are currently proven. Progress toward
/// full native coverage: when this equals `TARGET_LINE_TYPES.len()` (26), native
/// upload handles ~all real raid logs.
pub fn coverage_progress() -> (usize, usize) {
    let target: BTreeSet<&str> = TARGET_LINE_TYPES.iter().copied().collect();
    let proven = PROVEN_LINE_TYPES
        .iter()
        .filter(|t| target.contains(**t))
        .count();
    (proven, TARGET_LINE_TYPES.len())
}

/// Structural readiness: how many of the [`TARGET_LINE_TYPES`] have a built +
/// structurally-tested encoder ([`STRUCTURALLY_READY_LINE_TYPES`]), and how many of
/// those are also *live-confirmed* ([`PROVEN_LINE_TYPES`]). Returns
/// `(ready, confirmed, total)`.
///
/// This is the honest progress report after the strategy pivot: `ready` counts
/// what the encoder can structurally produce now; `confirmed` counts what a live
/// round-trip has cleared for native upload. `ready > confirmed` means "built,
/// pending the owner-run server-acceptance test." Neither number alone enables
/// native — [`super::format::FORMAT_VERSION_CONFIRMED`] gates that too.
pub fn structural_readiness() -> (usize, usize, usize) {
    let target: BTreeSet<&str> = TARGET_LINE_TYPES.iter().copied().collect();
    let ready = STRUCTURALLY_READY_LINE_TYPES
        .iter()
        .filter(|t| target.contains(**t))
        .count();
    let (confirmed, total) = coverage_progress();
    (ready, confirmed, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every proven type must be a real target type — guards against typos or a
    // type being added to PROVEN that isn't in the known vocabulary (which would
    // silently never match and quietly disable a type's coverage).
    #[test]
    fn proven_types_are_all_valid_targets() {
        let target: BTreeSet<&str> = TARGET_LINE_TYPES.iter().copied().collect();
        for t in PROVEN_LINE_TYPES {
            assert!(
                target.contains(t),
                "proven type {t:?} is not in TARGET_LINE_TYPES — typo or unknown type"
            );
        }
    }

    // The target vocabulary is the closed 26-type set: the 20 base combat/trial
    // types from the census + the 6 Infinite Archive ENDLESS_DUNGEON_* markers. Pin
    // the count so an accidental edit is caught; changing it is a deliberate act
    // (e.g. a future patch genuinely adds a type).
    #[test]
    fn target_vocabulary_is_the_closed_set_of_26() {
        assert_eq!(
            TARGET_LINE_TYPES.len(),
            26,
            "20 base census types + 6 Infinite Archive markers = 26"
        );
        // No duplicates.
        let unique: BTreeSet<&str> = TARGET_LINE_TYPES.iter().copied().collect();
        assert_eq!(
            unique.len(),
            26,
            "TARGET_LINE_TYPES must have no duplicates"
        );
    }

    #[test]
    fn coverage_progress_reports_proven_over_total() {
        let (proven, total) = coverage_progress();
        assert_eq!(total, 26);
        assert_eq!(proven, PROVEN_LINE_TYPES.len());
    }

    // Every structurally-ready type must be a real target type (same guard as the
    // proven set) — catches typos that would silently never match.
    #[test]
    fn structurally_ready_types_are_all_valid_targets() {
        let target: BTreeSet<&str> = TARGET_LINE_TYPES.iter().copied().collect();
        let unique: BTreeSet<&str> = STRUCTURALLY_READY_LINE_TYPES.iter().copied().collect();
        assert_eq!(
            unique.len(),
            STRUCTURALLY_READY_LINE_TYPES.len(),
            "STRUCTURALLY_READY_LINE_TYPES must have no duplicates"
        );
        for t in STRUCTURALLY_READY_LINE_TYPES {
            assert!(
                target.contains(t),
                "structurally-ready type {t:?} is not in TARGET_LINE_TYPES"
            );
        }
    }

    // The readiness report: more types are structurally built than are live-
    // confirmed (the honest current state — encoders exist, server acceptance is
    // pending). Every confirmed (proven) type must also be structurally ready.
    #[test]
    fn structural_readiness_reports_ready_ge_confirmed() {
        let (ready, confirmed, total) = structural_readiness();
        assert_eq!(total, 26);
        assert_eq!(confirmed, PROVEN_LINE_TYPES.len());
        assert!(
            ready >= confirmed,
            "ready ({ready}) must be >= confirmed ({confirmed})"
        );
        // Anything confirmed must be a subset of structurally-ready (can't confirm
        // a type whose encoder isn't built).
        let ready_set: BTreeSet<&str> = STRUCTURALLY_READY_LINE_TYPES.iter().copied().collect();
        for t in PROVEN_LINE_TYPES {
            assert!(
                ready_set.contains(t),
                "proven type {t:?} must also be structurally ready"
            );
        }
    }

    #[test]
    fn an_unproven_line_type_forces_fallback() {
        // The gate is OPEN (native rendering confirmed). A log of only-proven types
        // routes native; a single UNPROVEN type still forces fallback so a novel
        // (e.g. future-patch) event can never reach the encoder and corrupt a report.
        let proven_only = ["0,BEGIN_LOG,123,15", "4,ZONE_CHANGED,1129,\"Hall\""];
        assert!(
            matches!(assess(proven_only), Coverage::Native),
            "a log of only-proven types must route native now that rendering is confirmed"
        );

        let with_unknown = [
            "0,BEGIN_LOG,123,15",
            "4,SOME_FUTURE_EVENT,1,2,3",
            "5,END_LOG",
        ];
        match assess(with_unknown) {
            Coverage::Fallback { unproven } => {
                assert!(unproven.contains(&"SOME_FUTURE_EVENT".to_string()));
            }
            Coverage::Native => panic!("an unproven line type must force fallback"),
        }
    }

    #[test]
    fn a_log_with_only_proven_types_is_native() {
        // Simulate a future state where these two are proven, to lock the gate
        // logic itself (independent of which types are actually proven now).
        fn assess_with(proven: &[&str], lines: &[&str]) -> Coverage {
            let proven: BTreeSet<&str> = proven.iter().copied().collect();
            let mut unproven = BTreeSet::new();
            for l in lines {
                if let Some(t) = super::line_type(l) {
                    if !proven.contains(t) {
                        unproven.insert(t.to_string());
                    }
                }
            }
            if unproven.is_empty() {
                Coverage::Native
            } else {
                Coverage::Fallback {
                    unproven: unproven.into_iter().collect(),
                }
            }
        }
        let log = ["4,ZONE_CHANGED,1129,x", "5,MAP_CHANGED,1576,y"];
        assert!(assess_with(&["ZONE_CHANGED", "MAP_CHANGED"], &log).is_native());
        // One unproven type → fallback.
        let log2 = ["4,ZONE_CHANGED,1129,x", "5,COMBAT_EVENT,1,2,3"];
        assert!(!assess_with(&["ZONE_CHANGED", "MAP_CHANGED"], &log2).is_native());
    }

    #[test]
    fn blank_lines_are_ignored() {
        assert!(matches!(assess(["", "   "]), Coverage::Native));
    }

    // An Infinite Archive log carries the six ENDLESS_DUNGEON_* markers (grammar
    // from the real golden, Archon report M6t4mDzFWyqraPdN) alongside standard
    // combat types. All are proven — the markers as golden-confirmed no-ops — so an
    // IA log routes native instead of falling back to the official uploader.
    #[test]
    fn an_infinite_archive_log_routes_native() {
        let ia = [
            "0,BEGIN_LOG,123,15,\"NA Megaserver\",\"en\",\"eso.live.12.0\"",
            "119777,ENDLESS_DUNGEON_BEGIN,1,1781928320125,T",
            "152543,ENDLESS_DUNGEON_INIT,1,1782074394312,F",
            "163488,ENDLESS_DUNGEON_STAGE_END,1,1781928320125",
            "183759,ENDLESS_DUNGEON_BUFF_ADDED,1,200204",
            "269960,ENDLESS_DUNGEON_BUFF_REMOVED,1,200204",
            "189697,ENDLESS_DUNGEON_END,1,1780500,26000,F",
            "300000,COMBAT_EVENT,DAMAGE,1,2,3",
            "400000,END_LOG",
        ];
        assert!(
            matches!(assess(ia), Coverage::Native),
            "an Infinite Archive log must route native (markers are proven no-ops)"
        );
    }
}
