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

use super::encode::{
    accumulate_ability_signals, combat_event_registers, join_lines, resolve_ability_section,
    split_csv_quoted_pub, AbilitySignals, ActorInfo,
};
use super::serialize::MasterTableDoc;

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

    /// Monotonically increasing counter bumped ONLY when `actor_map`/`ability_map`
    /// actually change contents (a first-registration `assign_actor`/`assign_ability`,
    /// which also covers the `HEALTH_RECOVERY` splice). The live driver reads it to
    /// skip the two full-map clones of `refresh_maps` on the ~92% of lines that leave
    /// the maps untouched (the L7 hot-path fix); an idempotent no-op never bumps it, so
    /// the maps the emitter holds stay byte-identical to a per-line clone.
    version: u64,
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
            version: 0,
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

    /// The map-mutation version (see [`Self::version`]'s field doc). Bumped only when
    /// `actor_map`/`ability_map` actually changed; equal values across two calls mean
    /// the maps are byte-identical between them.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Process one raw line, updating both maps. Idempotent semantics match the
    /// re-walk: an already-assigned identity/ability is never re-numbered.
    pub fn update(&mut self, line: &str) {
        let f = split_csv_quoted_pub(line);
        self.update_with_fields(line, &f);
    }

    /// [`Self::update`] with the quote-aware CSV split already performed by the caller
    /// (the live driver splits each line ONCE and threads the fields to every consumer
    /// — this state, the master state, and the emitter — instead of splitting three
    /// times per line). `f` must be exactly `split_csv_quoted_pub(line)`.
    pub fn update_with_fields(&mut self, line: &str, f: &[&str]) {
        let Some(kind) = f.get(1).map(|s| s.trim()) else {
            return;
        };
        match kind {
            "UNIT_ADDED" => self.on_unit_added(line, f),
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
            "COMBAT_EVENT" | "EFFECT_CHANGED" | "BEGIN_CAST" => self.on_registering_line(kind, f),
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
        // Real mutation of `actor_map` — bump so the live driver knows to re-push.
        self.version += 1;
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
        // Real mutation of `ability_map` (incl. the HEALTH_RECOVERY splice, which
        // routes through here) — bump so the live driver knows to re-push.
        self.version += 1;
    }
}

// ── Incremental MASTER-TABLE record state (Step 2b) ──────────────────────────────

/// An actor captured at its `UNIT_ADDED`, ready to render its master record once its
/// pinned index is known. Mirrors `build_master_table_inner`'s `PendingActor` —
/// `owner_is_player` is resolved AT the `UNIT_ADDED` (from the live unit→is-player
/// map at that moment), so it is captured once and never recomputed.
struct CapturedActor {
    actor: ActorInfo,
    owner_is_player: bool,
}

/// A pet candidate captured at a player-owned monster's `UNIT_ADDED`, in file order.
/// Both sides are stored as IDENTITIES (not indices) because a pet record's
/// `{petIdx}|{ownerIdx}` is rendered against the cut's FINAL pinned actor map
/// (matching `build_tuples_and_pets`, which resolves pet/owner indices from the
/// final `identity_to_actor`). De-dup is by the resolved `(petIdx, ownerIdx)` at
/// render time, exactly as the re-walk de-dups.
struct PetCandidate {
    pet_identity: String,
    owner_identity: String,
}

/// Maintains the cumulative MASTER-TABLE *record* state incrementally, line by line,
/// so the live driver renders each cut's master directly from this state +
/// `emitter.tuples()` instead of re-walking the unbounded `all_lines` buffer
/// (`build_master_table_with_tuples_forced`). Proven byte-identical to that re-walk
/// oracle at every cut by the differential test in [`super::live`].
///
/// What it maintains, mirroring `build_master_table_inner`:
///
/// * header: `log_version`, `server`, `begin_wall` from the FIRST `BEGIN_LOG`.
/// * actors: each registering/player identity's [`CapturedActor`] (the `ActorInfo`
///   plus `owner_is_player` resolved at its `UNIT_ADDED`), keyed by identity.
///   Records are rendered at cut time in PINNED-INDEX order using the index from
///   the companion [`IncrementalIndexState`]'s actor map (which already reproduces
///   the re-walk's registering-monster inclusion + append-only pin).
/// * abilities: first-write-wins [`AbilitySignals`] + the first-appearance id order,
///   re-rendered each cut (divergence #4: an ability's damageType can be learned in
///   a LATER segment than its `ABILITY_INFO`, so records cannot be frozen).
/// * pets: ordered [`PetCandidate`]s captured at each player-owned `UNIT_ADDED`.
///
/// It owns the SAME live unit-binding bookkeeping the re-walk recomputes per call
/// (`unit_is_player` for owner resolution; `pet_owner_live` for the time-aware
/// owner lookup). The tuple section is supplied externally (`emitter.tuples()`), so
/// no tuple state is kept here.
#[derive(Default)]
pub struct IncrementalMasterState {
    // ── Header (first BEGIN_LOG) ────────────────────────────────────────────────
    log_version: Option<String>,
    server: String,
    begin_wall: u64,

    // ── Actor records ───────────────────────────────────────────────────────────
    /// identity → its captured record inputs (first UNIT_ADDED for the identity).
    captured_actors: HashMap<String, CapturedActor>,
    /// Live raw unitId → is-player, for resolving whether a monster's owner is a
    /// player (mirrors `build_master_table_inner`'s `unit_is_player`). Last write
    /// wins (recycled unit ids rebind), never cleared — matching the re-walk, which
    /// only ever inserts into this map.
    unit_is_player: HashMap<String, bool>,

    // ── Ability records ─────────────────────────────────────────────────────────
    /// First-write-wins damage/heal/status/info signals (shared accumulator).
    ability_signals: AbilitySignals,
    /// Ability ids in first-appearance order (incl. the synthetic at first
    /// HEALTH_REGEN), the order `resolve_ability_section` assigns indices in.
    ordered_ability_ids: Vec<String>,
    /// Dedup guard for `ordered_ability_ids` (mirrors the re-walk's `seen` set).
    ability_seen: std::collections::BTreeSet<String>,
    /// Whether the synthetic HEALTH_RECOVERY has been placed (first HEALTH_REGEN).
    synthetic_placed: bool,

    // ── Pet records ─────────────────────────────────────────────────────────────
    /// Pet candidates in file (UNIT_ADDED) order.
    pet_candidates: Vec<PetCandidate>,
    /// Time-aware live raw unitId → (identity, is_player), set on UNIT_ADDED and
    /// cleared on UNIT_REMOVED — the `build_tuples_and_pets` `live` map, but storing
    /// the identity (indices are resolved at render time from the final pin).
    pet_owner_live: HashMap<String, (String, bool)>,
}

impl IncrementalMasterState {
    /// Process one raw line, folding it into the cumulative master record state.
    /// Idempotent semantics match `build_master_table_inner`'s single pass: a
    /// first-write-wins field is never overwritten; an already-captured actor /
    /// already-seen ability is not re-captured.
    pub fn update(&mut self, line: &str) {
        let f = split_csv_quoted_pub(line);
        self.update_with_fields(line, &f);
    }

    /// [`Self::update`] with the quote-aware CSV split already performed by the caller
    /// (the live driver splits each line ONCE and threads the fields to every
    /// consumer). `f` must be exactly `split_csv_quoted_pub(line)`.
    pub fn update_with_fields(&mut self, line: &str, f: &[&str]) {
        let Some(kind) = f.get(1).map(|s| s.trim()) else {
            return;
        };
        // Ability signals (damage element / heal / EFFECT_INFO status / ABILITY_INFO)
        // are folded for EVERY relevant line, exactly as the re-walk's pass 1.
        accumulate_ability_signals(&mut self.ability_signals, kind, f, line);

        match kind {
            "BEGIN_LOG" if self.log_version.is_none() => {
                // tail: <wallMs>,<logVersion>,"<server>",...
                let rest = line.splitn(3, ',').nth(2).unwrap_or("");
                let mut t = split_csv_quoted_pub(rest).into_iter();
                self.begin_wall = t.next().and_then(|w| w.trim().parse().ok()).unwrap_or(0);
                self.log_version = Some(t.next().unwrap_or("").trim().to_string());
                self.server = t.next().unwrap_or("").trim().to_string();
            }
            "UNIT_ADDED" => self.on_unit_added(line, f),
            "UNIT_REMOVED" => {
                if let Some(u) = f.get(2) {
                    self.pet_owner_live.remove(u.trim());
                }
            }
            "HEALTH_REGEN" => {
                if !self.synthetic_placed {
                    self.synthetic_placed = true;
                    if self.ability_seen.insert(HEALTH_RECOVERY_ID.to_string()) {
                        self.ordered_ability_ids
                            .push(HEALTH_RECOVERY_ID.to_string());
                    }
                }
            }
            "ABILITY_INFO" => {
                // First-appearance ordering: only place an id we have an ABILITY_INFO
                // for (the signal accumulate above stored it), mirroring the re-walk's
                // `info.contains_key` guard.
                let rest = line.splitn(3, ',').nth(2).unwrap_or("");
                let id = rest.split(',').next().unwrap_or("").trim().to_string();
                if !id.is_empty()
                    && self.ability_signals.info.contains_key(&id)
                    && self.ability_seen.insert(id.clone())
                {
                    self.ordered_ability_ids.push(id);
                }
            }
            _ => {}
        }
    }

    fn on_unit_added(&mut self, line: &str, f: &[&str]) {
        let rest = line.splitn(3, ',').nth(2).unwrap_or("");
        let Some(actor) = ActorInfo::parse(rest) else {
            return;
        };
        let identity = actor.identity();
        let is_player = matches!(actor, ActorInfo::Player { .. });

        // Track this unit's is-player status for owner resolution (mirrors
        // `unit_is_player`). Keyed on the raw unit id; last write wins.
        if let Some(unit_id) = f.get(2).map(|s| s.trim().to_string()) {
            // Owner-is-player is resolved NOW, against bindings set by EARLIER
            // UNIT_ADDEDs — so resolve before inserting this unit's own binding,
            // matching the re-walk (which inserts this unit then reads `owner_unit_id`
            // against the map that already holds prior units; a unit is never its own
            // owner, so insertion order vs. this read does not matter, but we keep the
            // re-walk's exact sequence: insert is below).
            // Pet-owner time-aware live map (build_tuples_and_pets `live`): the pet
            // record uses the owner's binding as it stood at THIS UNIT_ADDED.
            let owner_unit_id = match &actor {
                ActorInfo::Monster { owner_unit_id, .. }
                    if !owner_unit_id.is_empty() && owner_unit_id != "0" =>
                {
                    Some(owner_unit_id.clone())
                }
                _ => None,
            };
            let owner_is_player = owner_unit_id
                .as_deref()
                .and_then(|o| self.unit_is_player.get(o).copied())
                .unwrap_or(false);

            // Pet candidate: capture in file order if the owner unit is currently live
            // AND a player (build_tuples_and_pets resolves pet/owner indices at render
            // time from the final pin, so store identities). Mirror the re-walk's gate:
            // the owner must be in the live map (added, not yet removed) and is_player.
            if let Some(owner) = owner_unit_id.as_deref() {
                if let Some((owner_identity, owner_live_is_player)) =
                    self.pet_owner_live.get(owner).cloned()
                {
                    if owner_live_is_player {
                        self.pet_candidates.push(PetCandidate {
                            pet_identity: identity.clone(),
                            owner_identity,
                        });
                    }
                }
            }

            // Capture the actor record inputs (dedup by identity across sessions).
            // Mirror `build_master_table_inner`: prefer a RESOLVED player record over an
            // "Offline"/empty placeholder so an offline-first re-add upgrades IN PLACE,
            // keeping the live master byte-identical to the re-walk oracle (the live twin
            // of the one-shot charId-dedup fix — without this, native live would freeze
            // the "Offline" row for an offline-first player).
            if let Some(captured) = self.captured_actors.get_mut(&identity) {
                let new_is_resolved = matches!(
                    &actor,
                    ActorInfo::Player { name, account, .. }
                        if !name.is_empty() && name.as_str() != "Offline" && !account.is_empty()
                );
                let stored_is_placeholder = matches!(
                    &captured.actor,
                    ActorInfo::Player { name, .. } if name.is_empty() || name.as_str() == "Offline"
                );
                if new_is_resolved && stored_is_placeholder {
                    captured.actor = actor.clone();
                }
            } else {
                self.captured_actors.insert(
                    identity.clone(),
                    CapturedActor {
                        actor: actor.clone(),
                        owner_is_player,
                    },
                );
            }

            // Now record this unit's bindings for LATER lines.
            self.unit_is_player.insert(unit_id.clone(), is_player);
            self.pet_owner_live.insert(unit_id, (identity, is_player));
        }
    }

    /// The log version captured from the first `BEGIN_LOG` (its field `f[3]`), used
    /// to frame both the master header and the fights segment. `None` until a
    /// `BEGIN_LOG` has been seen.
    pub fn log_version(&self) -> Option<&str> {
        self.log_version.as_deref()
    }

    /// Render the FULL cumulative master text for a cut, byte-identical to
    /// `build_master_table_with_tuples_forced(&all_lines, tuples, pinned_actors,
    /// pinned_abilities)`. `pinned_actors`/`pinned_abilities` are the cut's frozen
    /// index maps (the emitter's, == the companion `IncrementalIndexState`'s maps);
    /// `tuples` is `emitter.tuples()`. Returns `None` only when no `BEGIN_LOG` has
    /// been seen (no log version), matching the oracle's `log_version?`.
    ///
    /// The returned `u64` is the number of tuple records embedded in the master
    /// (`tuple_records.len()`, == the doc's `last_assigned_tuple_id`). The live driver
    /// cross-checks it against `emitter.allocated()` before every POST — an exact,
    /// structural count rather than a fragile text re-parse of the rendered master (C5).
    pub fn render_master(
        &self,
        tuples: &[(u32, u32, u32)],
        pinned_actors: &HashMap<String, u32>,
        pinned_abilities: &HashMap<String, u32>,
    ) -> Option<(String, u64)> {
        let log_version = self.log_version.clone()?;

        // ── Actors: rendered in PINNED-INDEX order. The pinned actor map IS the set
        // of actors the master lists (every registering monster + every player; a
        // never-registering monster is absent from the map, exactly as the re-walk
        // drops it). Each captured actor renders at its pinned index. ───────────────
        let mut by_index: Vec<(u32, &CapturedActor)> = pinned_actors
            .iter()
            .filter_map(|(identity, &idx)| self.captured_actors.get(identity).map(|ca| (idx, ca)))
            .collect();
        by_index.sort_by_key(|(i, _)| *i);
        let actors: Vec<String> = by_index
            .iter()
            .map(|(index, ca)| {
                ca.actor.to_master_record(
                    *index as usize,
                    &self.server,
                    self.begin_wall,
                    ca.owner_is_player,
                )
            })
            .collect();

        // ── Abilities: re-render each cut from the first-write-wins signals + the
        // first-appearance id order, pinned (divergence #4). ──────────────────────
        let (ability_records, _ability_index) = resolve_ability_section(
            &self.ordered_ability_ids,
            &self.ability_signals,
            Some(pinned_abilities),
        );

        // ── Tuples: the emitter's table, formatted `{s}|{t}|{a}`. ─────────────────
        let tuple_records: Vec<String> = tuples
            .iter()
            .map(|(s, t, a)| format!("{s}|{t}|{a}"))
            .collect();

        // ── Pets: resolve each candidate's (petIdx, ownerIdx) from the final pinned
        // map, in capture order, de-duplicated — exactly `build_tuples_and_pets`. A
        // candidate whose pet identity is not in the pinned map is dropped (the
        // re-walk's `let Some(&idx) = identity_to_actor.get(&identity) else continue`).
        let mut pet_seen: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        let mut pets: Vec<String> = Vec::new();
        for cand in &self.pet_candidates {
            let Some(&pet_idx) = pinned_actors.get(&cand.pet_identity) else {
                continue;
            };
            let Some(&owner_idx) = pinned_actors.get(&cand.owner_identity) else {
                continue;
            };
            if pet_seen.insert((pet_idx, owner_idx)) {
                pets.push(format!("{pet_idx}|{owner_idx}"));
            }
        }

        let actors_string = join_lines(&actors);
        let abilities_string = join_lines(&ability_records);
        let tuples_string = join_lines(&tuple_records);
        let pets_string = join_lines(&pets);

        let doc = MasterTableDoc {
            log_version: &log_version,
            game_version: "1",
            log_file_details: "",
            last_assigned_actor_id: actors.len() as u64,
            actors_string: &actors_string,
            last_assigned_ability_id: ability_records.len() as u64,
            abilities_string: &abilities_string,
            last_assigned_tuple_id: tuple_records.len() as u64,
            tuples_string: &tuples_string,
            last_assigned_pet_id: pets.len() as u64,
            pets_string: &pets_string,
        };
        Some((doc.render(), tuple_records.len() as u64))
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
        // A named player now dedups by stable character id alone (not account+id), so
        // the same character logged later as "Offline","" merges instead of splitting.
        assert_eq!(a.get("p:820189967932710348").copied(), Some(1));
        assert!(
            idx_b == Some(2) && idx_a == Some(3),
            "under the advancing pin the FIRST registrant (Wisp B) takes the lower index: B={idx_b:?} A={idx_a:?}"
        );
    }

    // OFFLINE-FIRST in the LIVE path: a player added first as "Offline","" (out of
    // render range) then resolved on a later UNIT_ADDED must UPGRADE in the incremental
    // master too — byte-identical to the one-shot re-walk. Codex adversarial-review
    // finding: my one-shot charId-dedup + name-upgrade fix wasn't mirrored in
    // IncrementalMasterState, so native live would freeze the "Offline" row while a
    // finished-log upload resolved it.
    #[test]
    fn incremental_master_upgrades_offline_first_player_like_rewalk() {
        use crate::uploader::native::encode::build_master_table_with_tuples_forced;
        let lines = [
            "0,BEGIN_LOG,1700000000000,15,\"NA Megaserver\",\"en\",\"10.0\"",
            "5,UNIT_ADDED,1,PLAYER,T,1,0,F,3,9,\"Hero\",\"@hero\",111,50,1000,0,PLAYER_ALLY,T",
            // Character 222 appears OFFLINE first, then resolves to "Sidekick".
            "6,UNIT_ADDED,2,PLAYER,F,2,0,F,0,0,\"Offline\",\"\",222,0,0,0,PLAYER_ALLY,T",
            "100,UNIT_ADDED,3,PLAYER,F,2,0,F,4,9,\"Sidekick\",\"@side\",222,50,1100,0,PLAYER_ALLY,T",
        ];
        let mut idx = IncrementalIndexState::default();
        let mut mst = IncrementalMasterState::default();
        for l in lines {
            idx.update(l);
            mst.update(l);
        }
        let tuples: Vec<(u32, u32, u32)> = Vec::new();
        let (live, tuple_count) = mst
            .render_master(&tuples, idx.actor_map(), idx.ability_map())
            .expect("live master builds");
        assert_eq!(
            tuple_count,
            tuples.len() as u64,
            "render_master must return the embedded tuple count (== tuples.len())"
        );
        let refs: Vec<&str> = lines.to_vec();
        let oracle = build_master_table_with_tuples_forced(
            &refs,
            &tuples,
            idx.actor_map(),
            idx.ability_map(),
        )
        .expect("one-shot master builds");
        assert_eq!(live, oracle, "live master must match the one-shot re-walk");
        assert!(
            live.contains("Sidekick^@side^222^") && !live.contains("Offline"),
            "offline-first player must resolve to its real name in the live master; got:\n{live}"
        );
    }

    // C5: the tuple count `render_master` returns is exactly the number of tuples it was
    // handed (`tuples.len()`), for a non-empty table — the desync cross-check depends on
    // this equality, and it must not rely on re-parsing rendered text.
    #[test]
    fn render_master_returns_tuple_count_equal_to_tuples_len() {
        let lines = [
            "0,BEGIN_LOG,1700000000000,15,\"NA Megaserver\",\"en\",\"10.0\"",
            "5,UNIT_ADDED,1,PLAYER,T,1,0,F,3,9,\"Hero\",\"@hero\",111,50,1000,0,PLAYER_ALLY,T",
        ];
        let mut idx = IncrementalIndexState::default();
        let mut mst = IncrementalMasterState::default();
        for l in lines {
            idx.update(l);
            mst.update(l);
        }
        let tuples: Vec<(u32, u32, u32)> = vec![(0, 1, 2), (3, 4, 5), (6, 7, 8)];
        let (_text, tuple_count) = mst
            .render_master(&tuples, idx.actor_map(), idx.ability_map())
            .expect("live master builds");
        assert_eq!(
            tuple_count,
            tuples.len() as u64,
            "render_master must return tuples.len() as its embedded tuple count"
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
