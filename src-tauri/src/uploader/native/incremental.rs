//! Incremental live-streaming index maps (the L7 perf fix).
//!
//! The live driver needs the cumulative `identity → actorIndex` and
//! `abilityId → abilityIndex` maps current before each line that allocates a
//! tuple. The original spike re-derived them from the whole cumulative line
//! buffer on every registering line ([`super::encode::actor_ability_maps_forced`]
//! re-walks all lines), which is O(events × lines) and pins an unbounded
//! `all_lines` buffer in memory.
//!
//! [`IncrementalIndexState`] maintains the SAME map contents in O(1) amortized per
//! line by processing each line once as it arrives. It is the system-under-test;
//! [`super::encode::actor_ability_maps_forced`] is retained as the oracle, and a
//! differential test (`incremental_maps_match_rewalk_*`) asserts byte-identical
//! map CONTENTS at every line against the re-walk evaluated **with an
//! advancing pin** — the exact freeze the live driver applies on every refresh.
//!
//! ## What the live driver actually does (the semantics this reproduces)
//!
//! The original `LiveSegmenter::refresh_maps` re-derived the maps on *every*
//! registering line, seeding the re-walk with the emitter's CURRENT frozen map as
//! the pin, then re-freezing. So the operative oracle is the **pinned re-walk
//! advanced per line**, not the unpinned single-pass re-walk. Under the advancing
//! pin, once an entity is assigned an index that index is frozen by the next
//! refresh — so a monster is assigned its index **the moment it first registers**
//! (registration-fire order), and a never-registering monster is never assigned.
//! This is exactly what the original spike produced; the incremental updater
//! preserves it bit-for-bit while dropping the O(events × lines) re-walk.
//!
//! Concretely (two monsters A,B added in that order; B registers before A):
//! B registers first → B appends above the current max; A registers later →
//! A appends above the new max. So with the advancing pin the assignment order is
//! registration-fire order, NOT `UNIT_ADDED` order — the pin makes the first
//! assignment sticky. The differential test pins this exactly (fixture F1).
//!
//! A monster can never register before its `UNIT_ADDED` (a registering event
//! resolves the unit through the time-aware `live_monster_unit` binding, only set
//! on `UNIT_ADDED`), so assign-on-register never references an unknown identity.

use std::collections::HashMap;

use super::encode::{combat_event_registers, split_csv_quoted_pub, ActorInfo};

/// The synthetic `HEALTH_RECOVERY` ability id spliced into the ability table at the
/// first `HEALTH_REGEN` (mirrors `encode::build_ability_table_pinned`). Kept in sync
/// with that constant; both must agree or the ability index diverges.
const HEALTH_RECOVERY_ID: &str = "61322";

/// Maintains the cumulative `identity → actorIndex` and `abilityId → abilityIndex`
/// maps incrementally, line by line, reproducing the live driver's
/// pinned-advancing re-walk ([`super::encode::actor_ability_maps_forced`] applied
/// per line) without re-walking the whole buffer.
///
/// Seed with the prior frozen maps (the live driver's append-only pin) via
/// [`Self::with_pins`]; an empty seed reproduces the unpinned-from-scratch path.
pub struct IncrementalIndexState {
    // ── Actor axis ────────────────────────────────────────────────────────────
    /// The authoritative `identity → 1-based actor index` (mirrors
    /// `actor_ability_maps_forced`'s first return under the advancing pin).
    identity_to_actor: HashMap<String, u32>,
    /// Next actor index to assign (`max(values) + 1`).
    actor_next: u32,
    /// Time-aware live `unitId → monster identity` (set on `UNIT_ADDED`, cleared on
    /// `UNIT_REMOVED`) — exactly `registering_monster_identities`' `live` map. A
    /// monster is assigned its actor index the first time a registering event
    /// resolves through this binding.
    live_monster_unit: HashMap<String, String>,
    /// Per monster identity, its FIRST `UNIT_ADDED` sequence number (a monotonic
    /// counter incremented on every monster `UNIT_ADDED`). Used to break the
    /// intra-event tie: when a single registering event first-registers BOTH src and
    /// tgt, the re-walk assigns them in `UNIT_ADDED` order, not src-then-tgt, so the
    /// updater orders the newly-registering pair by this sequence (fixture F7).
    monster_added_seq: HashMap<String, u64>,
    /// The next monster `UNIT_ADDED` sequence number.
    added_seq_next: u64,

    // ── Ability axis ──────────────────────────────────────────────────────────
    /// `abilityId → 1-based ability index` (includes `"0" → 0`).
    ability_to_index: HashMap<String, u32>,
    /// Next ability index to assign.
    ability_next: u32,
    /// Whether the synthetic `HEALTH_RECOVERY` has been spliced (at the first
    /// `HEALTH_REGEN`).
    health_recovery_spliced: bool,
}

impl Default for IncrementalIndexState {
    fn default() -> Self {
        Self::with_pins(&HashMap::new(), &HashMap::new())
    }
}

impl IncrementalIndexState {
    /// Build seeded with the prior frozen maps (the live driver's append-only pin).
    /// A pinned identity keeps its exact index and is never re-assigned; new
    /// identities append above the prior maximum.
    pub fn with_pins(
        prior_actors: &HashMap<String, u32>,
        prior_abilities: &HashMap<String, u32>,
    ) -> Self {
        let identity_to_actor = prior_actors.clone();
        let actor_next = identity_to_actor.values().copied().max().unwrap_or(0) + 1;
        let mut ability_to_index = prior_abilities.clone();
        // `"0" → 0` is always present (the from-scratch path inserts it; a pin
        // carries it forward). `ability_next` skips index 0.
        ability_to_index.insert("0".to_string(), 0);
        let ability_next = ability_to_index
            .values()
            .copied()
            .filter(|&v| v != 0)
            .max()
            .unwrap_or(0)
            + 1;
        Self {
            identity_to_actor,
            actor_next,
            live_monster_unit: HashMap::new(),
            monster_added_seq: HashMap::new(),
            added_seq_next: 0,
            ability_to_index,
            ability_next,
            health_recovery_spliced: false,
        }
    }

    /// The current `identity → actorIndex` map (content-identical to the re-walk).
    pub fn actor_map(&self) -> &HashMap<String, u32> {
        &self.identity_to_actor
    }

    /// The current `abilityId → abilityIndex` map (content-identical to the re-walk).
    pub fn ability_map(&self) -> &HashMap<String, u32> {
        &self.ability_to_index
    }

    /// Process one raw line, updating both maps. Idempotent semantics match the
    /// re-walk: an already-assigned identity/ability is never re-numbered.
    pub fn update(&mut self, line: &str) {
        let f = split_csv_quoted_pub(line);
        let Some(kind) = f.get(1).map(|s| s.trim()) else {
            return;
        };
        match kind {
            "UNIT_ADDED" => self.on_unit_added(line, &f),
            "UNIT_REMOVED" => {
                if let Some(u) = f.get(2) {
                    self.live_monster_unit.remove(u.trim());
                }
            }
            "ABILITY_INFO" => {
                // id is the first field after `<ts>,ABILITY_INFO,` (mirrors
                // build_ability_table_pinned: ai.ability_id from rest).
                if let Some(id) = f.get(2).map(|s| s.trim()) {
                    if !id.is_empty() {
                        self.assign_ability(id);
                    }
                }
            }
            "HEALTH_REGEN" => {
                if !self.health_recovery_spliced {
                    self.health_recovery_spliced = true;
                    self.assign_ability(HEALTH_RECOVERY_ID);
                }
            }
            "COMBAT_EVENT" | "EFFECT_CHANGED" | "BEGIN_CAST" => self.on_registering_line(kind, &f),
            _ => {}
        }
    }

    fn on_unit_added(&mut self, line: &str, f: &[&str]) {
        let rest = line.splitn(3, ',').nth(2).unwrap_or("");
        let Some(actor) = ActorInfo::parse(rest) else {
            return;
        };
        let identity = actor.identity();
        match actor {
            // Players always register → assign immediately, in file order (matches
            // the re-walk, which assigns players at their UNIT_ADDED unconditionally).
            ActorInfo::Player { .. } => {
                self.assign_actor(identity);
            }
            ActorInfo::Monster { .. } => {
                // Record the time-aware live binding (mirrors
                // registering_monster_identities' `live`). Assignment is DEFERRED to
                // the monster's first registering event (fire order under the pin).
                // Stamp the FIRST UNIT_ADDED sequence for this identity so an
                // intra-event tie (both src+tgt first-register together) assigns in
                // UNIT_ADDED order, matching the re-walk (fixture F7).
                self.monster_added_seq
                    .entry(identity.clone())
                    .or_insert_with(|| {
                        let s = self.added_seq_next;
                        self.added_seq_next += 1;
                        s
                    });
                if let Some(unit_id) = f.get(2).map(|s| s.trim().to_string()) {
                    self.live_monster_unit.insert(unit_id, identity);
                }
            }
        }
    }

    fn on_registering_line(&mut self, kind: &str, f: &[&str]) {
        // Field positions match registering_monster_identities (encode.rs:1225-1237).
        let (result, src, tgt) = if kind == "COMBAT_EVENT" {
            (
                f.get(2).map(|s| s.trim()).unwrap_or(""),
                f.get(9).map(|s| s.trim()).unwrap_or(""),
                f.get(19).map(|s| s.trim()).unwrap_or("0"),
            )
        } else {
            (
                "",
                f.get(6).map(|s| s.trim()).unwrap_or(""),
                f.get(16).map(|s| s.trim()).unwrap_or("0"),
            )
        };
        let self_target = tgt == "*";
        if kind == "COMBAT_EVENT" && !combat_event_registers(result, self_target) {
            return;
        }
        // Collect the monster identities this event newly-registers (src, then tgt),
        // de-duplicated and excluding any already assigned. Assign them in ascending
        // UNIT_ADDED sequence order — NOT src-then-tgt — because the re-walk assigns
        // a registering event's actors by UNIT_ADDED file order, so when ONE event
        // first-registers both src and tgt the earlier-added unit takes the lower
        // index regardless of which side it is (fixture F7). When only one side is
        // new this collapses to a single assignment; the ordering is moot.
        let mut to_assign: Vec<String> = Vec::new();
        let consider = |unit: &str, this: &Self, out: &mut Vec<String>| {
            if let Some(id) = this.live_monster_unit.get(unit) {
                if !this.identity_to_actor.contains_key(id) && !out.contains(id) {
                    out.push(id.clone());
                }
            }
        };
        consider(src, self, &mut to_assign);
        if !self_target && tgt != "0" {
            consider(tgt, self, &mut to_assign);
        }
        // Stable sort by first-UNIT_ADDED sequence (a monster always has one here —
        // it registered through a live binding set at its UNIT_ADDED).
        to_assign.sort_by_key(|id| self.monster_added_seq.get(id).copied().unwrap_or(u64::MAX));
        for id in to_assign {
            self.assign_actor(id);
        }
    }

    /// Assign `identity` the next actor index if it has none. Idempotent — a pinned
    /// or already-assigned identity keeps its index (the append-only pin).
    fn assign_actor(&mut self, identity: String) {
        if self.identity_to_actor.contains_key(&identity) {
            return;
        }
        self.identity_to_actor.insert(identity, self.actor_next);
        self.actor_next += 1;
    }

    /// Append-only ability assignment (matches the ability-axis H1 pin). A pinned or
    /// already-seen ability keeps its index.
    fn assign_ability(&mut self, id: &str) {
        if self.ability_to_index.contains_key(id) {
            return;
        }
        self.ability_to_index
            .insert(id.to_string(), self.ability_next);
        self.ability_next += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uploader::native::encode::actor_ability_maps_forced;

    /// The core differential gate: drive [`IncrementalIndexState`] line by line and,
    /// after EACH line, assert its two maps equal the re-walk oracle
    /// ([`actor_ability_maps_forced`]) over the cumulative prefix, with the pin
    /// advanced exactly as the live driver freezes it on each refresh. `HashMap` `==`
    /// is content equality (order-independent), which is the right comparison —
    /// both downstream consumers read the maps by key, not by iteration order.
    ///
    /// Asserting at every line (not just at cuts) is strictly stronger and pins the
    /// exact registering semantics, including the assignment order under the pin.
    fn assert_incremental_matches_rewalk(lines: &[&str]) {
        let mut inc = IncrementalIndexState::default();
        let mut frozen_actors: HashMap<String, u32> = HashMap::new();
        let mut frozen_abilities: HashMap<String, u32> = HashMap::new();
        for end in 1..=lines.len() {
            let prefix = &lines[..end];
            let last = lines[end - 1];
            inc.update(last);
            let (rw_a, rw_b) =
                actor_ability_maps_forced(prefix, Some((&frozen_actors, &frozen_abilities)));
            assert_eq!(
                inc.actor_map(),
                &rw_a,
                "actor map diverged from re-walk after line {end}: {last}"
            );
            assert_eq!(
                inc.ability_map(),
                &rw_b,
                "ability map diverged from re-walk after line {end}: {last}"
            );
            // Advance the pin exactly as the live emitter does on refresh: the
            // current cumulative result becomes the next line's frozen prior.
            frozen_actors = rw_a;
            frozen_abilities = rw_b;
        }
    }

    fn fixture(name: &str) -> Vec<String> {
        let text = match name {
            "live_correlation" => include_str!("testdata/live_correlation_synthetic.log"),
            "two_session" => include_str!("testdata/two_session_synthetic.log"),
            "f1_late_registration" => include_str!("testdata/inc_f1_late_registration.log"),
            "f2_added_never_registers" => {
                include_str!("testdata/inc_f2_added_never_registers.log")
            }
            "f4_recycled_unit" => include_str!("testdata/inc_f4_recycled_unit.log"),
            "f5_regen_before_ability" => include_str!("testdata/inc_f5_regen_before_ability.log"),
            "f7_intra_event_both_register" => {
                include_str!("testdata/inc_f7_intra_event_both_register.log")
            }
            other => panic!("unknown fixture {other}"),
        };
        text.lines().map(str::to_string).collect()
    }

    fn final_state(lines: &[&str]) -> IncrementalIndexState {
        let mut inc = IncrementalIndexState::default();
        for l in lines {
            inc.update(l);
        }
        inc
    }

    #[test]
    fn incremental_maps_match_rewalk_live_correlation_fixture() {
        let lines = fixture("live_correlation");
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        assert_incremental_matches_rewalk(&refs);
    }

    #[test]
    fn incremental_maps_match_rewalk_two_session_fixture() {
        let lines = fixture("two_session");
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        assert_incremental_matches_rewalk(&refs);
    }

    // F1 — late-registration ORDER (the assignment-order trap): two monsters A,B
    // added in file order; B registers (a landing combat event) BEFORE A. Under the
    // advancing pin the live driver assigns in REGISTRATION-FIRE order (B before A),
    // because the first refresh that assigns B pins it. The incremental updater must
    // reproduce that — NOT UNIT_ADDED order. The differential assert is the gate; the
    // explicit ordering check below documents the (perhaps surprising) fire-order.
    #[test]
    fn incremental_maps_match_rewalk_f1_late_registration() {
        let lines = fixture("f1_late_registration");
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        assert_incremental_matches_rewalk(&refs);
        let inc = final_state(&refs);
        let a = inc.actor_map();
        let idx_b = a.get("m:7002:Wisp B").copied();
        let idx_a = a.get("m:7001:Wisp A").copied();
        assert_eq!(a.get("p:@hero:820189967932710348").copied(), Some(1));
        assert!(
            idx_b == Some(2) && idx_a == Some(3),
            "under the advancing pin the FIRST registrant (Wisp B) takes the lower index: B={idx_b:?} A={idx_a:?}"
        );
    }

    // F2 — a monster added but only ever the target of a non-landing TARGET_DEAD
    // (combat_event_registers → false) is excluded from the actor map, matching the
    // re-walk's registering filter.
    #[test]
    fn incremental_maps_match_rewalk_f2_added_never_registers() {
        let lines = fixture("f2_added_never_registers");
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        assert_incremental_matches_rewalk(&refs);
        let inc = final_state(&refs);
        assert!(
            !inc.actor_map().contains_key("m:9001:Dead On Arrival"),
            "a monster only ever TARGET_DEAD-targeted must be excluded from the actor map"
        );
        assert!(
            inc.actor_map().contains_key("m:9002:Real Target"),
            "a monster that lands a real event must be included"
        );
    }

    // F4 — recycled unitId: unit 30 is monster M1 (registers), UNIT_REMOVED 30,
    // then UNIT_ADDED 30 = monster M2 (registers). Two distinct identities, two
    // indices, correct time-aware live binding at each registering event.
    #[test]
    fn incremental_maps_match_rewalk_f4_recycled_unit() {
        let lines = fixture("f4_recycled_unit");
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        assert_incremental_matches_rewalk(&refs);
        let inc = final_state(&refs);
        let a = inc.actor_map();
        assert!(
            a.contains_key("m:8001:Recycled One") && a.contains_key("m:8002:Recycled Two"),
            "both distinct monster identities sharing a recycled unit id must be indexed: {a:?}"
        );
    }

    // F5 — HEALTH_REGEN before any ABILITY_INFO: the synthetic 61322 must get the
    // first ability index, matching the re-walk's chronological splice.
    #[test]
    fn incremental_maps_match_rewalk_f5_regen_before_ability() {
        let lines = fixture("f5_regen_before_ability");
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        assert_incremental_matches_rewalk(&refs);
        let inc = final_state(&refs);
        assert_eq!(
            inc.ability_map().get(HEALTH_RECOVERY_ID).copied(),
            Some(1),
            "the synthetic HEALTH_RECOVERY must take ability index 1 when HEALTH_REGEN precedes every ABILITY_INFO"
        );
    }

    // F7 — intra-event order trap: a single COMBAT_EVENT where BOTH src and tgt are
    // monsters first-registering in THAT event, with the UNIT_ADDED order (tgt 6001
    // then src 6002) OPPOSITE to src-before-tgt. The re-walk assigns by UNIT_ADDED
    // order within a single registering event (6001→2, 6002→3), so the incremental
    // updater must NOT naively assign src-before-tgt. The differential gate enforces it.
    #[test]
    fn incremental_maps_match_rewalk_f7_intra_event_both_register() {
        let lines = fixture("f7_intra_event_both_register");
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        assert_incremental_matches_rewalk(&refs);
        let inc = final_state(&refs);
        let a = inc.actor_map();
        assert_eq!(
            a.get("m:6001:Target First").copied(),
            Some(2),
            "the earlier-UNIT_ADDED monster takes the lower index even when it is the event TARGET"
        );
        assert_eq!(a.get("m:6002:Source Second").copied(), Some(3));
    }

    // A continuation of the SAME long-lived updater equals a from-scratch run — the
    // invariant the live driver relies on (one updater across all segments) — and the
    // freeze at a mid-point is an append-only subset of the final maps (no renumber).
    #[test]
    fn continuation_equals_from_scratch_and_preserves_freeze() {
        let lines = fixture("two_session");
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let split = refs.len() / 2;

        let whole = final_state(&refs);

        let mut cont = IncrementalIndexState::default();
        for l in &refs[..split] {
            cont.update(l);
        }
        let frozen_a = cont.actor_map().clone();
        let frozen_b = cont.ability_map().clone();
        for l in &refs[split..] {
            cont.update(l);
        }
        assert_eq!(cont.actor_map(), whole.actor_map());
        assert_eq!(cont.ability_map(), whole.ability_map());
        for (id, &i) in &frozen_a {
            assert_eq!(
                whole.actor_map().get(id),
                Some(&i),
                "pin not preserved: {id}"
            );
        }
        for (id, &i) in &frozen_b {
            assert_eq!(
                whole.ability_map().get(id),
                Some(&i),
                "ability pin not preserved: {id}"
            );
        }
    }
}
