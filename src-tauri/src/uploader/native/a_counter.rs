//! Subordinal **A** counter — the last unsolved piece of the code-1 line, parked
//! here as a **gated, non-shipping** module with a measured baseline.
//!
//! The code-1 (and all-code) subordinal `seg[2]` is `A.B.C`. Everything except
//! the leading `A` is proven byte-exact and lives in [`super::encode`]:
//! [`super::encode::ActorTable::code1_subordinal`] takes `A` as input and renders
//! the rest exactly. `A` itself is a **global allocation counter** assigned as the
//! official parser walks the raw log, and it is NOT yet reproducible byte-exact.
//!
//! ## What is known (verified)
//!
//! * `A` is a dense global counter in segment-emission order (the first ~81 are
//!   exactly `1,2,3,…`). Codes 41/51 (`ZONE_CHANGED`/`MAP_CHANGED`) don't use the
//!   counter — they write their literal zone/map id into the slot.
//! * The **allocation key** is the first-seen `(sourceIdentity, abilityId,
//!   targetIdentity)` triple, allocated in raw-line order, skipping a set of
//!   no-op `actionResult`s. This [`ACounter`] implements exactly that backbone.
//!
//! ## Why it does not ship (the two unsolved sub-problems)
//!
//! The backbone **over-allocates**: it mints ~4045 distinct `A` where the true
//! count is 3799 (and only 62 of those should be allocated-but-unemitted "gaps").
//! Two opposing errors remain, both confirmed by replay:
//!
//! 1. **Re-cast splits** — a genuine *re-cast* of the same triple should get a
//!    NEW `A`, but a DoT *tick* of that triple must reuse the old one. The
//!    cast-occurrence boundary (likely a fresh `BEGIN_CAST`/castTrackId generation
//!    after the prior cast `END_CAST`s) is not yet pinned.
//! 2. **Ability-family merges** — linked morph/synergy abilities share one `A`
//!    (e.g. ability ids `4730`↔`146311`), which the triple key splits. This needs
//!    an ability-link graph derived from `ABILITY_INFO`/`EFFECT_INFO`.
//!
//! Plus the 62 emission **gaps** (allocations whose segment line is filtered/merged
//! away) are not reproduced from raw-only data.
//!
//! Until a replay mints **exactly** the true count with the gaps at the right
//! positions, shipping `A` would silently drift on a longer log. So this module is
//! exercised ONLY by a baseline test (below) that records the current
//! known-broken accuracy; it is never used by the encoder, and `COMBAT_EVENT`
//! stays out of [`super::coverage::PROVEN_LINE_TYPES`].
//!
//! Clean-room: derived from our own matched-pair captures; no third-party code.

use std::collections::HashMap;

/// `actionResult` values that do NOT allocate an `A` (no-op / failed casts).
/// Verified against the combat capture — these never lead an emitted line.
pub const SKIP_ACTION_RESULTS: &[&str] = &[
    "QUEUED",
    "ABILITY_ON_COOLDOWN",
    "CANNOT_USE",
    "INSUFFICIENT_RESOURCE",
    "CANT_SWAP_HOTBAR_IS_OVERRIDDEN",
    "BAD_TARGET",
    "SPRINTING",
    "IN_AIR",
    "TARGET_OUT_OF_RANGE",
    "TARGET_NOT_IN_FRONT",
    "TARGET_TOO_CLOSE",
    "RECAST",
    "CASTER_DEAD",
    "TARGET_DEAD",
    "SOUL_GEM_RESURRECTION_ACCEPTED",
];

/// A unit's stable identity for the allocation key (players by account+char id,
/// monsters by monsterId+name) — coarser than the runtime unit id (which is
/// reused intra-session).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnitIdentity {
    Player {
        account: String,
        char_id: String,
    },
    Monster {
        monster_id: String,
        name: String,
    },
    /// Target id `0` (no target).
    None,
    /// An unresolved runtime unit id (not yet seen via `UNIT_ADDED`).
    Unknown(String),
}

/// The first-seen `(source, ability, target)` allocation backbone. Walk the raw
/// log in order, feeding `UNIT_ADDED`/`UNIT_REMOVED` to keep identities current
/// and the allocating events (`COMBAT_EVENT`/`EFFECT_CHANGED`/`BEGIN_CAST`) to
/// mint `A`. **Known-incomplete** (see module docs): no re-cast boundary, no
/// ability-family merge, no gap reproduction.
#[derive(Debug, Default)]
pub struct ACounter {
    unit2id: HashMap<String, UnitIdentity>,
    key2a: HashMap<(UnitIdentity, String, UnitIdentity), u32>,
    next_a: u32,
}

impl ACounter {
    pub fn new() -> Self {
        Self {
            next_a: 1,
            ..Default::default()
        }
    }

    /// How many distinct `A` values have been minted so far.
    pub fn allocated(&self) -> u32 {
        self.next_a - 1
    }

    /// Feed one raw line (the full line, including `<ts>,<TYPE>,…`). Returns the
    /// minted `A` if this line allocated/looked-up one (for an allocating event),
    /// else `None`.
    pub fn feed(&mut self, line: &str) -> Option<u32> {
        let f = super::encode::split_csv_quoted_pub(line);
        let kind = f.get(1).map(|s| s.trim())?;
        match kind {
            "UNIT_ADDED" => {
                self.on_unit_added(&f);
                None
            }
            "UNIT_REMOVED" => {
                if let Some(u) = f.get(2) {
                    self.unit2id.remove(u.trim());
                }
                None
            }
            "COMBAT_EVENT" => {
                let ar = f.get(2).map(|s| s.trim()).unwrap_or("");
                if SKIP_ACTION_RESULTS.contains(&ar) {
                    return None;
                }
                // src f[9], abilityId f[8], tgt f[19].
                let src = self.resolve(f.get(9));
                let ab = f.get(8).map(|s| s.trim().to_string()).unwrap_or_default();
                let tgt = self.resolve_target(f.get(19), &src);
                Some(self.alloc((src, ab, tgt)))
            }
            "EFFECT_CHANGED" => {
                // src f[6], abilityId f[5], tgt f[16].
                let src = self.resolve(f.get(6));
                let ab = f.get(5).map(|s| s.trim().to_string()).unwrap_or_default();
                let tgt = self.resolve_target(f.get(16), &src);
                Some(self.alloc((src, ab, tgt)))
            }
            "BEGIN_CAST" => {
                // src f[6], abilityId f[5], tgt f[16] (may be absent).
                let src = self.resolve(f.get(6));
                let ab = f.get(5).map(|s| s.trim().to_string()).unwrap_or_default();
                let tgt = self.resolve_target(f.get(16), &src);
                Some(self.alloc((src, ab, tgt)))
            }
            _ => None,
        }
    }

    fn on_unit_added(&mut self, f: &[&str]) {
        let Some(unit_id) = f.get(2).map(|s| s.trim().to_string()) else {
            return;
        };
        let unit_type = f.get(3).map(|s| s.trim()).unwrap_or("");
        let identity = if unit_type == "PLAYER" {
            UnitIdentity::Player {
                account: f
                    .get(11)
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .unwrap_or_default(),
                char_id: f.get(12).map(|s| s.trim().to_string()).unwrap_or_default(),
            }
        } else {
            UnitIdentity::Monster {
                monster_id: f.get(6).map(|s| s.trim().to_string()).unwrap_or_default(),
                name: f
                    .get(10)
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .unwrap_or_default(),
            }
        };
        self.unit2id.insert(unit_id, identity);
    }

    /// Resolve a unit-id field to an identity. `0`/missing → `None`; `*` → `None`
    /// (callers that fold self-targeting handle `*` before calling); an unseen id
    /// → `Unknown`.
    fn resolve(&self, unit_id: Option<&&str>) -> UnitIdentity {
        match unit_id.map(|s| s.trim()) {
            None | Some("0") | Some("*") => UnitIdentity::None,
            Some(u) => self
                .unit2id
                .get(u)
                .cloned()
                .unwrap_or_else(|| UnitIdentity::Unknown(u.to_string())),
        }
    }

    /// Resolve a target field, folding the `*` "same as source" token to `src`.
    fn resolve_target(&self, tgt: Option<&&str>, src: &UnitIdentity) -> UnitIdentity {
        if tgt.map(|s| s.trim()) == Some("*") {
            return src.clone();
        }
        self.resolve(tgt)
    }

    fn alloc(&mut self, key: (UnitIdentity, String, UnitIdentity)) -> u32 {
        if let Some(&a) = self.key2a.get(&key) {
            return a;
        }
        let a = self.next_a;
        self.next_a += 1;
        self.key2a.insert(key, a);
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_set_is_the_verified_15() {
        assert_eq!(SKIP_ACTION_RESULTS.len(), 15);
    }

    // Basic mechanics: the same triple reuses its A; a different triple gets the
    // next one. (This is the backbone; the not-yet-solved re-cast/family rules are
    // documented at the module level, not asserted here.)
    #[test]
    fn same_triple_reuses_a_distinct_triple_allocates() {
        let mut c = ACounter::new();
        c.feed("10,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@h\",111,50,1700,0,PLAYER_ALLY,T");
        c.feed("10,UNIT_ADDED,30,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F");
        // Hero hits Bear with ability 100 — first sight allocates A=1.
        let a1 = c
            .feed("20,COMBAT_EVENT,DAMAGE,FIRE,1,50,0,5000,100,1,1/1,0/0,0/0,0/0,0/0,0,0.5,0.5,0.0,30,1/1,0/0,0/0,0/0,0/0,0,0.5,0.5,0.0")
            .unwrap();
        // Same triple again (a tick) → reuse A=1.
        let a2 = c
            .feed("21,COMBAT_EVENT,DAMAGE,FIRE,1,50,0,5000,100,1,1/1,0/0,0/0,0/0,0/0,0,0.5,0.5,0.0,30,1/1,0/0,0/0,0/0,0/0,0,0.5,0.5,0.0")
            .unwrap();
        assert_eq!(a1, 1);
        assert_eq!(a2, 1, "same triple reuses its A");
        // Different ability → new A.
        let a3 = c
            .feed("22,COMBAT_EVENT,DAMAGE,FIRE,1,50,0,5001,200,1,1/1,0/0,0/0,0/0,0/0,0,0.5,0.5,0.0,30,1/1,0/0,0/0,0/0,0/0,0,0.5,0.5,0.0")
            .unwrap();
        assert_eq!(a3, 2);
    }

    #[test]
    fn skip_results_do_not_allocate() {
        let mut c = ACounter::new();
        c.feed("10,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@h\",111,50,1700,0,PLAYER_ALLY,T");
        let none = c.feed(
            "20,COMBAT_EVENT,QUEUED,GENERIC,0,0,0,5000,100,1,1/1,0/0,0/0,0/0,0/0,0,0.5,0.5,0.0,*",
        );
        assert_eq!(none, None, "QUEUED is in the skip set");
        assert_eq!(c.allocated(), 0);
    }

    // The BASELINE test: replay the real combat log and record the current
    // known-broken accuracy of the backbone against the golden code-1 A's. This is
    // NOT a pass/fail correctness gate — it is a measurable baseline so future work
    // on the re-cast/family/gap rules can show progress (and so a regression in the
    // backbone is caught). The golden fixture for this lives in the gitignored
    // decode samples; when absent (normal committed checkout) the test is a no-op.
    #[test]
    fn backbone_baseline_is_recorded_not_a_correctness_gate() {
        // The full 11MB combat log is a decode-only fixture, not committed. Only
        // run the heavy replay when it is present locally.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../.decode-samples/combat_raw_encounter.log"
        );
        let Ok(raw) = std::fs::read_to_string(path) else {
            // Not present in a clean checkout — nothing to measure, and that's fine.
            return;
        };
        let mut c = ACounter::new();
        for line in raw.lines() {
            c.feed(line);
        }
        let minted = c.allocated();
        // KNOWN BASELINE (documented, intentionally NOT byte-exact): the backbone
        // over-allocates (~4045 vs the true 3799). Assert only a loose envelope so
        // the test is a stable progress marker, not a brittle exact-count gate. The
        // real promotion bar (minted == 3799 AND the 62 gaps reproduced) is tracked
        // in the module docs and the project tasks, not here.
        assert!(
            (3700..=4200).contains(&minted),
            "A-counter backbone minted {minted}; expected the documented ~3799–4045 \
             envelope. A big move means the allocation rule changed — re-measure and \
             update the baseline (and check the gap/re-cast/family work)."
        );
    }
}
