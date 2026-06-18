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

/// Raw `Encounter.log` line types our encoder reproduces byte-exactly, each
/// gated by a passing differential golden test. **Only add a type here in the
/// same change that adds its encoder AND a golden-diff test proving it.**
///
/// Currently empty: the encoder framework and differential harness exist, but no
/// event type has yet been proven byte-exact end-to-end. Until at least the full
/// set in the golden sample is proven, [`assess`] returns [`Coverage::Fallback`]
/// for every real log — i.e. the official uploader is used, which is correct and
/// safe. This is the honest state, enforced in code rather than assumed.
///
/// `COMBAT_EVENT` (segment code 1) status: nearly every per-field piece is proven
/// byte-exact (3733/3733 on the combat golden pair) and implemented in
/// [`super::encode`] — the state blocks (positions, championPoints), the crit
/// flag, the action-result-branched final field, the masks
/// ([`super::encode::ActorTable::code1_masks`]), and the subordinal *suffix*
/// ([`super::encode::ActorTable::code1_subordinal`]). It remains GATED out of this
/// list for ONE reason: the subordinal's leading allocation number `A` is a global
/// cross-code emission counter that cannot be minted from a single-code capture.
/// Until `A` is solved (needs a cross-code emission-ordered raw↔segment capture),
/// a whole code-1 line can't be reproduced byte-exact, so any log containing
/// `COMBAT_EVENT` must keep falling back to the official uploader.
pub const PROVEN_LINE_TYPES: &[&str] = &[
    // e.g. "ZONE_CHANGED", "MAP_CHANGED", ... added as each is proven.
    // "COMBAT_EVENT" — blocked solely on the subordinal `A` (see above).
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

    #[test]
    fn empty_proven_set_means_any_real_log_falls_back() {
        // The current honest state: nothing proven yet → real logs fall back.
        let log = ["0,BEGIN_LOG,123,15", "4,ZONE_CHANGED,1129,\"Hall\""];
        match assess(log) {
            Coverage::Fallback { unproven } => {
                assert!(unproven.contains(&"BEGIN_LOG".to_string()));
                assert!(unproven.contains(&"ZONE_CHANGED".to_string()));
            }
            Coverage::Native => panic!("nothing is proven yet; must fall back"),
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
