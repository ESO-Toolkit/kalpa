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
/// But "structurally valid" is only *proven* by uploading a real segment and
/// confirming ESO Logs renders it. That empirical step is the owner-run ship gate;
/// it has not happened yet, so this list stays empty and native upload stays off.
///
/// This is belt-and-suspenders with [`super::format::FORMAT_VERSION_CONFIRMED`]
/// (also `false`): either lock alone keeps native off. The flip to enable native
/// is deliberate and documented in [`STRUCTURALLY_READY_LINE_TYPES`].
///
/// **Promotion rule (updated for the structural-validity bar):** add a type here
/// only once (a) its [`super::events`] encoder + a passing *structural* test
/// exist — both true today for the ready set — AND (b) a live upload of a segment
/// containing it has been confirmed accepted by the server. Step (b) is what's
/// pending. The byte-level [`super::differential`] is now a *quality metric*, not
/// the promotion bar (the server re-parses and accepts any structurally-valid
/// segment, so byte-identity with the official uploader is not required).
pub const PROVEN_LINE_TYPES: &[&str] = &[
    // EMPTY again (2026-06-18): the live round-trip was server-ACCEPTED (report
    // jAHXkRdzpGwxVQ1t) but the report does NOT RENDER (infinite loading, absent
    // from the user's list). Server-accept ≠ parseable. So nothing is proven for
    // the real bar (renders correctly), and the gate is closed until the encoded
    // segment is confirmed to produce a working report. The encoders for
    // STRUCTURALLY_READY_LINE_TYPES exist + are structurally tested, but
    // "structurally valid" did NOT imply "the parser builds a report from it".
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
];

/// The **complete, closed** set of ESO `Encounter.log` event types — the finite
/// target the encoder must cover for native upload to handle ~all real raid
/// logs. Established by an event-type census across real logs spanning 70K → 8.4M
/// lines (a 120× size range, a year of game patches): the vocabulary is a closed
/// set of exactly these 20 types and does **not** grow with log size. There is no
/// long tail — two types (`EFFECT_CHANGED` + `COMBAT_EVENT`) are 92–95% of every
/// file, so the cost is concentrated.
///
/// 17 of these appear in 100% of logs (the mandatory floor); the 3 `*_TRIAL*`
/// markers appear in every actual trial/raid. Proving all 20 reaches effectively
/// 100% native coverage for real raid logs. A *future* game patch could add a new
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
/// full native coverage: when this equals `TARGET_LINE_TYPES.len()` (20), native
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

    // The target vocabulary is the closed 20-type set from the census. Pin the
    // count so an accidental edit is caught; changing it is a deliberate act
    // (e.g. a future patch genuinely adds a type).
    #[test]
    fn target_vocabulary_is_the_closed_set_of_20() {
        assert_eq!(
            TARGET_LINE_TYPES.len(),
            20,
            "the census established a closed set of 20 event types"
        );
        // No duplicates.
        let unique: BTreeSet<&str> = TARGET_LINE_TYPES.iter().copied().collect();
        assert_eq!(
            unique.len(),
            20,
            "TARGET_LINE_TYPES must have no duplicates"
        );
    }

    #[test]
    fn coverage_progress_reports_proven_over_total() {
        let (proven, total) = coverage_progress();
        assert_eq!(total, 20);
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
        assert_eq!(total, 20);
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
    fn empty_proven_set_means_any_real_log_falls_back() {
        // Gate closed again (the round-trip rendered broken): nothing is proven
        // for the real bar, so every real log falls back to the official uploader.
        let log = ["0,BEGIN_LOG,123,15", "4,ZONE_CHANGED,1129,\"Hall\""];
        match assess(log) {
            Coverage::Fallback { unproven } => {
                assert!(unproven.contains(&"BEGIN_LOG".to_string()));
                assert!(unproven.contains(&"ZONE_CHANGED".to_string()));
            }
            Coverage::Native => panic!("nothing is proven (renders); must fall back"),
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
}
