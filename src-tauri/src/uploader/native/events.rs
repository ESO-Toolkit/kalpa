//! Fights-segment **events** assembler — the driver that turns a raw session's
//! lines into the segment's events string.
//!
//! This is the missing analog of [`super::encode::build_master_table`] for the
//! *events* half of the upload. [`super::encode`] proved every per-field piece
//! byte-exact (state blocks, masks, the subordinal suffix, timestamps); this
//! module is the **whole-log driver** that walks the raw log once, in file order,
//! routes each line to its segment code, and emits a structurally-valid event
//! line — feeding the per-event encoders the running parser state (the actor
//! table, championPoints, and the allocation counter `A`).
//!
//! ## Emission model (verified)
//!
//! * **Emission order = raw-file line order.** A single forward pass; no intra-
//!   timestamp re-sort. Each routable line emits its segment line in place.
//! * **`A` is a global first-sight allocation counter.** Minted on the first
//!   sighting of the `(sourceIdentity, abilityId, targetIdentity)` triple and
//!   reused for that triple thereafter. It leads the subordinal field
//!   (`A.srcOrd.tgtOrd`).
//!
//! ## Byte-exact `A` is **not** the bar (and is not reached)
//!
//! ESO Logs' server re-parses the segment and accepts any *structurally valid*
//! one — the official-uploader's exact `A` allocation is not a server
//! requirement (a working third-party uploader emits a different, self-consistent
//! `A` and uploads succeed). So this assembler aims for a **dense, self-consistent
//! first-sight `A`**, not byte-identity with a capture. The byte-level
//! [`super::differential`] check is a *quality metric*, not the ship gate.
//!
//! What this module guarantees is **structural** correctness: each emitted line
//! has the right field layout for its code, every subordinal `A` is a real
//! allocated counter value, masks/state blocks are well-formed, and the segment's
//! declared event count equals the number of emitted lines. The
//! [`super::coverage`] gate still keeps any log whose line types are not all
//! proven-structural on the official uploader.
//!
//! Clean-room: routing/mint logic derived from our own matched-pair captures and
//! research prototypes; no third-party code.

use std::collections::HashMap;

use super::encode::{
    combat_noncode1_crit_flag, encode_map_changed, encode_state_block, encode_zone_changed,
    segment_ts, session_offset, split_csv_quoted_pub, ActorTable,
};

/// `actionResult` values that never emit a segment line (no-op / failed casts).
/// Mirrors [`super::a_counter::SKIP_ACTION_RESULTS`] — kept local so the assembler
/// owns its routing table.
const SKIP_ACTION_RESULTS: &[&str] = &[
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

/// `COMBAT_EVENT` actionResults that are *status* effects (stuns, taunts…). They
/// carry no damage/heal value and do not emit a code-1/2/3/26 line.
const STATUS_ACTION_RESULTS: &[&str] = &[
    "STUNNED",
    "TAUNTED",
    "SNARED",
    "ROOTED",
    "FEARED",
    "OFFBALANCE",
    "STAGGERED",
    "KNOCKBACK",
    "DISORIENTED",
    "CHARMED",
    "SILENCED",
    "INTERRUPT",
    "BLOCKED",
];

/// `actionResults` that emit a code-1 (damage) line. (`DIED`/`DIED_XP` are NOT
/// here — a death emits a code-19 line, handled separately.)
const CODE1_ACTION_RESULTS: &[&str] = &[
    "DAMAGE",
    "CRITICAL_DAMAGE",
    "IMMUNE",
    "BLOCKED_DAMAGE",
    "DODGED",
    "FALL_DAMAGE",
];

/// `actionResults` that emit a code-19 (unit death) line. The line is the combat
/// prefix + S + T state blocks with **no trailing crit/final tail** (the target's
/// state shows 0 health). Verified on the capture: `DIED`/`DIED_XP` → code 19.
const CODE19_ACTION_RESULTS: &[&str] = &["DIED", "DIED_XP"];

/// A buffered code-38 (DamageShielded) line awaiting its damaging-ability index.
/// `prefix` is the full line up to (but excluding) the trailing `|{f10}` field; the
/// target unit lets the flushing damage event confirm the run belongs to it.
struct PendingShield {
    /// The line text through field 9 (`...|0|{hit}`); `|{f10}` is appended on flush.
    prefix: String,
    /// The shielded (damage target) raw unit id — the back-patch run's owner.
    target_unit: String,
}

/// The running parser state for one log's event assembly.
///
/// Holds the actor table (master indices, reactions, owners — for masks and the
/// subordinal suffix), the per-unit current championPoints (for state blocks), the
/// effect-type table (`abilityId → BUFF/DEBUFF`, for effect-code routing), the
/// unit-id → identity map and the first-sight `A` allocator.
#[derive(Default)]
pub struct EventEmitter {
    actors: ActorTable,
    /// Raw unit id → current championPoints (from `UNIT_ADDED`/`UNIT_CHANGED`).
    champion_points: HashMap<String, String>,
    /// abilityId → effectType (`BUFF`/`DEBUFF`) from `EFFECT_INFO`.
    effect_type: HashMap<String, String>,
    /// Per effect instance `(srcUnit, abilityId, tgtUnit)` → its last stack count.
    /// An `EFFECT_CHANGED UPDATED` emits (codes 6/8/11) only when the stack count
    /// *changes* from this value; a re-application with the same stack is dropped.
    last_stack: HashMap<(String, String, String), String>,
    /// `identity → 1-based actor master index` and `abilityId → 1-based ability
    /// index`, supplied from the master-table build. The event subordinal `A` is
    /// the index of the event's `(srcActorIndex, tgtActorIndex, abilityIndex)`
    /// **tuple** in the master tuple/effects table — so these maps tie the segment
    /// to the master.
    identity_to_actor: HashMap<String, u32>,
    ability_to_index: HashMap<String, u32>,
    /// The tuple/effects table, built in event-emission order: a tuple key →
    /// its 1-based index (== the event's `A`). `tuple_order` keeps the records for
    /// the master's tuples section. Same `(src,tgt,ability)` → same index → same A,
    /// which is what makes a uploaded report *render* (events resolve to real
    /// effects).
    tuple_to_index: HashMap<(u32, u32, u32), u32>,
    tuple_order: Vec<(u32, u32, u32)>,
    /// `castTrackId → the tuple A of the BEGIN_CAST that opened it`. An effect
    /// event applied by a tracked cast emits a trailing `A{castA}` linking the buff
    /// to its cast (the reference's `source_cast_index`). Set when a cast emits.
    cast_track_to_a: HashMap<String, u32>,
    /// `castTrackId → (tupleA, src_unit, tgt_unit)` for TIMED casts (those that
    /// emitted a code-15 CastWithCastTime). A later `END_CAST COMPLETED` for that id
    /// emits a thin code-16 `Cast` line reusing the original cast's tuple A + units.
    /// Only timed casts are recorded (instant casts already emitted their code-16).
    timed_cast: HashMap<String, (u32, String, String)>,
    /// Per-unit last-seen shield pool (`unit_id → shield`). A buff GAINED carries a
    /// trailing shield magnitude only when the unit's shield *changed* from this
    /// stored value (the reference's `shield_values` history): `source_shield`/
    /// `target_shield` are the new values iff they differ from what was stored,
    /// else 0. The display then emits the trailing iff one is non-zero AND they
    /// differ from each other.
    shield_values: HashMap<String, u32>,
    /// Whether we are inside a BEGIN_COMBAT/END_COMBAT window. A damage-class
    /// COMBAT_EVENT outside combat allocates NO tuple (the only pre-allocation skip
    /// the parser applies); every other combat event allocates its tuple regardless
    /// of whether its line is emitted.
    in_combat: bool,
    /// `castTrackId → (sourceUnit, targetUnit)` recorded at every `BEGIN_CAST`. An
    /// `END_CAST INTERRUPTED` looks up the interrupted cast's caster (source) here to
    /// build the code-27 interrupt line (the reference's `cast_id_source_unit_id` /
    /// `cast_id_target_unit_id`). The fall-back when a cast id is unknown is
    /// [`Self::last_interrupt`].
    cast_id_units: HashMap<String, (String, String)>,
    /// The unit id of the most recent `INTERRUPT`-status COMBAT_EVENT target — the
    /// reference's `last_interrupt`, used as the interrupted-cast caster when the
    /// `END_CAST INTERRUPTED`'s cast id isn't in [`Self::cast_id_units`].
    last_interrupt: Option<String>,
    /// Buffered code-38 (DamageShielded) lines awaiting their damaging-ability index
    /// (`f10`). A DAMAGE_SHIELDED carries the SHIELD ability (146311), not the
    /// damaging one; the damaging ability arrives on the paired real DAMAGE/DOT event
    /// (same target) that immediately follows. Each entry is the line up to the f10
    /// slot. They flush — in order, before the damage line — when that event arrives.
    pending_shields: Vec<PendingShield>,
    /// Per-target accumulated **absorbed** shield damage (`target unit → sum of
    /// DAMAGE_SHIELDED hit values not yet folded`). The paired real DAMAGE/DOT event
    /// for that target folds this into its `overflow` (the reference's
    /// `temporary_damage_buffer`) and resets it to 0. This is why a hit-0 damage
    /// event whose damage was fully absorbed is still EMITTED (with the absorbed
    /// total in overflow) rather than dropped — the official segment keeps those.
    temp_damage: HashMap<String, u64>,
    /// The current session's segment-timestamp offset (`segTs = rawTs + offset`).
    offset: i64,
    /// Whether the first `BEGIN_LOG` has been seen (anchors `first_wall`).
    first_seen: bool,
    /// First session's `BEGIN_LOG` wall-clock ms (anchors all sessions' offsets).
    first_wall: i64,
    /// The current session's wall-clock delta from the first session
    /// (`wall − first_wall`). Combined with [`Self::first_event_ts`] to form the
    /// offset: `offset = session_wall_delta − first_event_ts`.
    session_wall_delta: i64,
    /// The raw ts of the **first emitted event** (anchors all segment timestamps to
    /// 0). The official segment maps its first event to ts 0, and that first event
    /// is the first *emittable* line (a `ZONE_CHANGED`), whose raw ts is **not**
    /// necessarily the `BEGIN_LOG` ts (e.g. `BEGIN_LOG@9`, first `ZONE_CHANGED@10`
    /// → official ts 0, so the anchor is 10, not 9). Set lazily on the first
    /// emitted event; `None` until then.
    first_event_ts: Option<i64>,

    // ── Live-streaming continuation state (the `spike/native-live` spike) ─────
    // These fields exist ONLY to support the debug-only native live-streaming
    // driver (`super::live`). In the one-shot path nothing reads them: `build()`
    // never opens a segment and the master is built once over the whole file, so
    // they stay at their defaults and have ZERO effect on the proven one-shot
    // output. See docs/native-live-streaming-spike-FINDINGS.md.
    /// The active session's `BEGIN_LOG` wall-clock ms (updated on EVERY `BEGIN_LOG`,
    /// unlike [`Self::first_wall`] which freezes on the first). The live wall window
    /// for a segment is `current_session_wall + raw_ts` — the same `begin_wall + rel`
    /// formula the proven one-shot [`segment_time_bounds`] uses, made stateful so a
    /// headerless segment (no `BEGIN_LOG` of its own) still gets a real window. Using
    /// RAW ts (not `seg_ts`) avoids the unbounded `−first_event_ts` skew.
    current_session_wall: i64,
    /// The RAW ts of the first / last event EMITTED in the segment currently being
    /// assembled (since the last [`Self::open_segment`]). `live_segment_time_bounds`
    /// turns these into the segment's `(startTime, endTime)` wall window. `None`
    /// until the segment emits its first line.
    seg_first_raw: Option<i64>,
    seg_last_raw: Option<i64>,
    /// The events emitted since the last [`Self::open_segment`] (the live segment
    /// body) and their count. `feed` appends here in addition to returning the line;
    /// the live driver [`Self::drain_segment_events`] takes them to frame a segment.
    /// The one-shot `build()` path ignores these (it assembles its own string).
    segment_events: String,
    segment_event_count: u64,
}

impl EventEmitter {
    pub fn new() -> Self {
        Self {
            actors: ActorTable::new(),
            ..Default::default()
        }
    }

    /// Construct with the master-table index maps (so the segment's `A` is the
    /// master tuple index — the reference that makes a report render).
    pub fn with_master_indices(
        identity_to_actor: HashMap<String, u32>,
        ability_to_index: HashMap<String, u32>,
    ) -> Self {
        Self {
            actors: ActorTable::new(),
            identity_to_actor,
            ability_to_index,
            ..Default::default()
        }
    }

    /// How many distinct tuples (== `A` values) have been allocated.
    pub fn allocated(&self) -> u32 {
        self.tuple_order.len() as u32
    }

    /// The ordered tuple/effects table (`(srcActorIndex, tgtActorIndex,
    /// abilityIndex)` records) built during assembly — this becomes the master
    /// table's tuples section, guaranteeing the segment's `A` references resolve.
    pub fn tuples(&self) -> &[(u32, u32, u32)] {
        &self.tuple_order
    }

    /// The set of actor identities currently in the frozen index map — the live
    /// driver passes these forward as `forced_identities` so a per-segment master
    /// rebuild keeps every already-indexed actor at its slot (the H1 fix). See
    /// [`super::encode::build_master_table_with_tuples_forced`].
    pub fn frozen_actor_identities(&self) -> std::collections::HashSet<String> {
        self.identity_to_actor.keys().cloned().collect()
    }

    /// Replace the master index maps with refreshed ones (the live driver rebuilds
    /// them from `all_lines_so_far` each cut, forcing the prior frozen identities so
    /// indices only ever grow). The caller is responsible for passing maps that
    /// PRESERVE every prior assignment — refreshing with a map that renumbers an
    /// already-emitted actor would dangle earlier segments' `A` refs (exactly hazard
    /// H1). The append-only guarantee comes from building the maps via
    /// [`super::encode::actor_ability_maps_forced`] with this emitter's
    /// [`Self::frozen_actor_identities`].
    pub fn refresh_master_indices(
        &mut self,
        identity_to_actor: HashMap<String, u32>,
        ability_to_index: HashMap<String, u32>,
    ) {
        self.identity_to_actor = identity_to_actor;
        self.ability_to_index = ability_to_index;
    }

    /// Allocate (or look up) the tuple index for an event's `(srcActorIndex,
    /// tgtActorIndex, abilityIndex)`. The 1-based index IS the event's subordinal
    /// `A`. First sight of a tuple appends it (event-emission order); repeats reuse
    /// the same index.
    fn alloc_tuple(&mut self, src_actor: u32, tgt_actor: u32, ability_idx: u32) -> u32 {
        let key = (src_actor, tgt_actor, ability_idx);
        if let Some(&a) = self.tuple_to_index.get(&key) {
            return a;
        }
        let a = self.tuple_order.len() as u32 + 1;
        self.tuple_order.push(key);
        self.tuple_to_index.insert(key, a);
        a
    }

    /// Resolve a raw unit id to its 1-based actor master index (0 = unknown).
    fn actor_index(&self, unit_id: &str) -> u32 {
        let u = unit_id.trim();
        if u.is_empty() || u == "0" || u == "*" {
            return 0;
        }
        // Resolve via the live actor table → identity → master index.
        self.actors
            .identity_of_unit(u)
            .and_then(|id| self.identity_to_actor.get(&id).copied())
            .unwrap_or(0)
    }

    /// Resolve an ability id to its 1-based ability master index (0 if unknown).
    fn ability_index(&self, ability_id: &str) -> u32 {
        self.ability_to_index
            .get(ability_id.trim())
            .copied()
            .unwrap_or(0)
    }

    /// The event's subordinal `A`: the tuple index for `(srcActor, tgtActor,
    /// abilityIndex)`. `src_unit`/`tgt_unit` are raw unit ids (the folded `*`
    /// target already resolved to the source). This is what ties the segment to
    /// the master tuple table.
    fn alloc_for(&mut self, src_unit: &str, ability_id: &str, tgt_unit: &str) -> u32 {
        let src = self.actor_index(src_unit);
        let tgt = self.actor_index(tgt_unit);
        let ab = self.ability_index(ability_id);
        self.alloc_tuple(src, tgt, ab)
    }

    /// The current championPoints for a unit id (`"0"` if unknown — the state-block
    /// encoder still produces a well-formed block).
    fn cp_of(&self, unit_id: &str) -> String {
        self.champion_points
            .get(unit_id.trim())
            .cloned()
            .unwrap_or_else(|| "0".to_string())
    }

    /// Assemble the whole log's events into one [`EventsOutput`] (the single-fight
    /// case — every event in one events string). Walks lines in file order.
    ///
    /// If the emitter was built without the master index maps
    /// ([`EventEmitter::new`]), they are derived from `lines` here so a standalone
    /// `EventEmitter::new().build(lines)` still produces correct tuple-indexed `A`
    /// values. The production path supplies the maps up front
    /// ([`EventEmitter::with_master_indices`]) so the segment and master share one
    /// tuple numbering.
    pub fn build(&mut self, lines: &[&str]) -> EventsOutput {
        if self.identity_to_actor.is_empty() && self.ability_to_index.is_empty() {
            let (id2a, ab2i) = super::encode::actor_ability_maps(lines);
            self.identity_to_actor = id2a;
            self.ability_to_index = ab2i;
        }
        let mut out = String::new();
        let mut count: u64 = 0;
        for line in lines {
            if let Some(ev) = self.feed(line) {
                // `feed` may return MULTIPLE `\n`-separated lines (a damage event
                // flushes its preceding buffered code-38 DamageShielded run); count
                // each emitted line.
                for l in ev.split('\n') {
                    out.push_str(l);
                    out.push('\n');
                    count += 1;
                }
            }
        }
        // Any code-38 lines still pending at end of stream (no following real
        // damage to back-patch them) are flushed with f10 = 0 (the reference's
        // un-patched placeholder).
        let tail = self.flush_pending_shields(None, None);
        for l in tail.iter() {
            out.push_str(l);
            out.push('\n');
            count += 1;
        }
        EventsOutput {
            events_string: out,
            event_count: count,
        }
    }

    /// Feed one raw line. Updates parser state and returns the emitted segment
    /// event line (without the trailing newline) if this line emits one, else
    /// `None` (state-only lines and dropped events).
    ///
    /// `pub(crate)` so the debug-only live driver ([`super::live`]) can feed lines
    /// one at a time across segment cuts; the one-shot path uses [`Self::build`].
    pub(crate) fn feed(&mut self, line: &str) -> Option<String> {
        let f = split_csv_quoted_pub(line);
        let kind = f.get(1).map(|s| s.trim())?;
        let raw_ts: i64 = f.first().and_then(|s| s.trim().parse().ok())?;
        let emitted = match kind {
            "BEGIN_LOG" => {
                self.on_begin_log(&f);
                None
            }
            "UNIT_ADDED" => {
                self.on_unit_added(&f, line);
                None
            }
            "UNIT_CHANGED" => {
                self.on_unit_changed(&f, line);
                None
            }
            "UNIT_REMOVED" => {
                // The actor index map keeps the latest unit→actor binding (a
                // recycled id is rebound on the next UNIT_ADDED), so no cleanup is
                // needed here.
                None
            }
            "EFFECT_INFO" => {
                if let (Some(ab), Some(ty)) = (f.get(2), f.get(3)) {
                    self.effect_type
                        .insert(ab.trim().to_string(), ty.trim().to_string());
                }
                None
            }
            "ZONE_CHANGED" => encode_zone_changed(self.seg_ts(raw_ts), tail(line)),
            "MAP_CHANGED" => encode_map_changed(self.seg_ts(raw_ts), tail(line)),
            "PLAYER_INFO" => self.emit_player_info(raw_ts, &f, line),
            "HEALTH_REGEN" => self.emit_health_regen(raw_ts, &f),
            "EFFECT_CHANGED" => self.emit_effect_changed(raw_ts, &f),
            "BEGIN_CAST" => self.emit_begin_cast(raw_ts, &f),
            "END_CAST" => self.emit_end_cast(raw_ts, &f),
            "COMBAT_EVENT" => self.emit_combat_event(raw_ts, &f),
            // Combat boundaries: codes 52/53 are a bare `{segTs}|52|` / `{segTs}|53|`
            // (a single trailing empty field). Verified 1:1 with BEGIN/END_COMBAT
            // counts in the capture. No subordinal/mask/state — pure markers.
            "BEGIN_COMBAT" => {
                self.in_combat = true;
                Some(format!("{}|52|", self.seg_ts(raw_ts)))
            }
            "END_COMBAT" => {
                self.in_combat = false;
                Some(format!("{}|53|", self.seg_ts(raw_ts)))
            }
            // END_TRIAL → code 55: `{segTs}|55|{trialId}|{duration}|{success}|{score}`.
            // Raw layout: `ts,END_TRIAL,id,duration,success(T/F),finalScore`.
            "END_TRIAL" => self.emit_end_trial(raw_ts, &f),
            // BEGIN_TRIAL / TRIAL_INIT carry no segment event (the reference treats
            // them as unknown/no-op) — they only need to be *covered* so a trial log
            // routes native instead of falling back.
            "BEGIN_TRIAL" | "TRIAL_INIT" => None,
            _ => None,
        };
        // Anchor the timestamp base on the first line that ACTUALLY emits (a
        // dropped line must never steal the ts-0 anchor). `seg_ts` already computed
        // this line's ts with the provisional anchor (== the real anchor for the
        // first emitted line), so committing here only fixes the base for the
        // following lines.
        if let Some(ev) = &emitted {
            self.commit_anchor(raw_ts);
            self.note_emitted_raw(raw_ts);
            // Accumulate the live segment body. `feed` may return multiple
            // \n-separated lines (a damage event flushing buffered code-38s); count
            // each, mirroring `build()`'s split. One-shot ignores this buffer.
            for l in ev.split('\n') {
                self.segment_events.push_str(l);
                self.segment_events.push('\n');
                self.segment_event_count += 1;
            }
        }
        emitted
    }

    /// Record the RAW ts of an emitted line for the current live segment's wall
    /// window. First emit of a segment sets both bounds; later emits advance the
    /// last. No-op semantics in one-shot (nothing reads `seg_*_raw` there).
    fn note_emitted_raw(&mut self, raw_ts: i64) {
        if self.seg_first_raw.is_none() {
            self.seg_first_raw = Some(raw_ts);
        }
        self.seg_last_raw = Some(raw_ts);
    }

    /// Begin a new live segment: clear the per-segment RAW-ts window so the NEXT
    /// emitted line anchors a fresh `(startTime, endTime)`. The report-scoped state
    /// (actor/ability/tuple tables, the offset/anchor, all in-flight correlations)
    /// is deliberately untouched — that is what makes a headerless segment encode
    /// correctly. Used only by the live driver; the one-shot path never calls this.
    pub fn open_segment(&mut self) {
        self.seg_first_raw = None;
        self.seg_last_raw = None;
        self.segment_events.clear();
        self.segment_event_count = 0;
    }

    /// Take the events emitted since the last [`Self::open_segment`] — the live
    /// segment body (`events_string` + `event_count`) ready to frame into a
    /// fights-segment. Does NOT reset the buffer (the live driver calls
    /// [`Self::open_segment`] after a successful build); the report-scoped encoder
    /// state (tuples, offset, correlations) is untouched. Returns an empty body if
    /// nothing was emitted this segment.
    pub fn drain_segment_events(&self) -> EventsOutput {
        EventsOutput {
            events_string: self.segment_events.clone(),
            event_count: self.segment_event_count,
        }
    }

    /// Flush any `DAMAGE_SHIELDED` lines still buffered at end-of-stream (no following
    /// real damage event arrived to back-patch them) into the CURRENT segment, with
    /// `f10 = 0` — mirroring what one-shot [`Self::build`] does at the end of the file.
    /// The live driver calls this once when logging ends, so a fully-absorbed final hit
    /// is not silently dropped. (Mid-stream the cut policy never cuts with pending
    /// shields, so this only fires on the terminal segment.)
    pub fn drain_trailing_shields_into_segment(&mut self) {
        let tail = self.flush_pending_shields(None, None);
        for l in tail {
            // These lines carry no fresh ts of their own; keep the segment's existing
            // window (they belong to the final fight already accounted for).
            self.segment_events.push_str(&l);
            self.segment_events.push('\n');
            self.segment_event_count += 1;
        }
    }

    /// Whether no `DAMAGE_SHIELDED` is buffered awaiting its paired damage event — a
    /// SAFE point to cut a live segment (cutting with a shield pending would strand
    /// the back-patch across the boundary). See [`PendingShield`].
    pub fn pending_shields_is_empty(&self) -> bool {
        self.pending_shields.is_empty()
    }

    /// The emitter's current frozen `identity → actor index` map (a clone), for
    /// pinning the next cumulative master rebuild (the H1 fix). Same data as
    /// [`Self::frozen_actor_identities`] but with the indices, which is what
    /// [`super::encode::actor_ability_maps_forced`] needs.
    pub fn frozen_actor_index_map(&self) -> HashMap<String, u32> {
        self.identity_to_actor.clone()
    }

    /// The emitter's current frozen `abilityId → ability index` map (a clone), the
    /// ability-axis companion to [`Self::frozen_actor_index_map`]. The live driver
    /// pins this into the next cumulative rebuild so a late `ABILITY_INFO` or the
    /// synthetic `HEALTH_RECOVERY` splice can't renumber prior abilities (the
    /// ability-axis half of the H1 fix).
    pub fn frozen_ability_index_map(&self) -> HashMap<String, u32> {
        self.ability_to_index.clone()
    }

    /// The current live segment's `(startTime, endTime)` wall-clock window for the
    /// `add-report-segment` request. Computed as `current_session_wall + raw_ts` of
    /// the segment's first/last EMITTED event — the proven one-shot `begin_wall +
    /// rel` formula ([`segment_time_bounds`]) made stateful and per-segment, using
    /// RAW ts (not `seg_ts`) to avoid the `−first_event_ts` skew.
    ///
    /// Returns `None` (so the driver SKIPS the POST rather than placing a segment at
    /// the 1970 epoch) when either no `BEGIN_LOG` has set the session wall yet
    /// (`current_session_wall == 0`) or the segment has emitted nothing
    /// (`seg_first_raw == None`).
    pub fn live_segment_time_bounds(&self) -> Option<(u64, u64)> {
        if self.current_session_wall <= 0 {
            return None;
        }
        let base = self.current_session_wall as u64;
        let first = self.seg_first_raw?;
        let last = self.seg_last_raw.unwrap_or(first);
        // Clamp negatives to 0 (a malformed line could carry a negative rel ts);
        // saturating_add keeps the window monotonic and never wraps.
        let start = base.saturating_add(first.max(0) as u64);
        let end = base.saturating_add(last.max(0) as u64);
        Some((start, end))
    }

    /// Apply a `BEGIN_LOG`: record the session's wall-clock delta from the first
    /// session. The first session anchors `first_wall`. The full offset is only
    /// finalized once the first event's raw ts is known (see [`Self::seg_ts`]):
    /// `offset = (wall − first_wall) − first_event_ts`.
    fn on_begin_log(&mut self, f: &[&str]) {
        let wall: i64 = f.get(2).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        if !self.first_seen {
            self.first_seen = true;
            self.first_wall = wall;
        }
        // The ACTIVE session's wall (every BEGIN_LOG, not just the first) — the live
        // wall-window base. One-shot ignores it.
        self.current_session_wall = wall;
        self.session_wall_delta = wall - self.first_wall;
        // Re-derive the offset for this session if the anchor event ts is known.
        if let Some(anchor) = self.first_event_ts {
            self.offset = session_offset(wall, self.first_wall, anchor);
        }
    }

    /// Segment timestamp for a raw relative ms, anchoring the first emitted event
    /// at ts 0. The anchor is the **first emitted event's raw ts** (not
    /// `BEGIN_LOG`'s), so a `BEGIN_LOG@9 → first ZONE@10` log maps that ZONE to 0
    /// (offset −10), matching the official segment. Subsequent sessions keep their
    /// `(wall − first_wall)` separation.
    ///
    /// **Pure** (`&self`): until the anchor is committed it computes the ts *as if*
    /// `raw_ts` were the anchor (i.e. `session_wall_delta`, which is 0 for the first
    /// session). The anchor is committed by [`Self::commit_anchor`], which `feed`
    /// calls only AFTER a line has actually produced output — so a *dropped* line
    /// can never steal the anchor and shift every later timestamp.
    fn seg_ts(&self, raw_ts: i64) -> u64 {
        let offset = match self.first_event_ts {
            Some(_) => self.offset,
            None => self.session_wall_delta - raw_ts,
        };
        segment_ts(raw_ts, offset)
    }

    /// Commit the timestamp anchor to a raw ts if not already set. Called by
    /// [`Self::feed`] on the FIRST line that actually emits, so the anchor is the
    /// first *emitted* event (dropped lines never anchor).
    fn commit_anchor(&mut self, raw_ts: i64) {
        if self.first_event_ts.is_none() {
            self.first_event_ts = Some(raw_ts);
            self.offset = self.session_wall_delta - raw_ts;
        }
    }

    /// Apply a `UNIT_ADDED`: update the actor table (master indices, reactions,
    /// owners — for masks, the subordinal suffix, and actor-index resolution) and
    /// the per-unit championPoints.
    fn on_unit_added(&mut self, f: &[&str], line: &str) {
        let rest = tail(line);
        self.actors.on_unit_added(rest);
        let Some(unit_id) = f.get(2).map(|s| s.trim().to_string()) else {
            return;
        };
        // championPoints: UNIT_ADDED field [12] (after the `<ts>,UNIT_ADDED,`
        // header these are f[2+0]=unitId … f[2+12]=champ → absolute index 14).
        if let Some(cp) = f.get(14) {
            self.champion_points.insert(unit_id, cp.trim().to_string());
        }
    }

    /// Apply a `UNIT_CHANGED`: update the actor table and championPoints. The
    /// `UNIT_CHANGED` tail (after `<ts>,UNIT_CHANGED,`) is
    /// `unitId,class,race,name,account,charId,level,CP,owner,reaction,grouped`, so
    /// tail-relative championPoints is index 7 → absolute index 9 (the 2-field
    /// `<ts>,UNIT_CHANGED,` header shifts every tail index by 2). Owner is at
    /// absolute 10, reaction at 11 — reading 11 here would store the reaction token
    /// (e.g. "HOSTILE") as championPoints and corrupt every later state block for
    /// the unit. Cross-checked against [`ActorTable::on_unit_changed`]'s layout.
    fn on_unit_changed(&mut self, f: &[&str], line: &str) {
        self.actors.on_unit_changed(tail(line));
        let Some(unit_id) = f.get(2).map(|s| s.trim().to_string()) else {
            return;
        };
        if let Some(cp) = f.get(9) {
            self.champion_points.insert(unit_id, cp.trim().to_string());
        }
    }

    /// Emit a code-44 `PLAYER_INFO` line: `{ts}|44|{masterIndex}|{arrays…}`. The
    /// master index is the unit's 1-based actor index (dense, from the actor
    /// table); the five raw arrays are passed through verbatim.
    fn emit_player_info(&mut self, raw_ts: i64, f: &[&str], line: &str) -> Option<String> {
        let unit_id = f.get(2)?.trim();
        let master_index = self.actors.master_index_of(unit_id)?;
        // The arrays are everything after `<ts>,PLAYER_INFO,<unitId>,`. They contain
        // commas inside `[...]`, so slice the raw line rather than re-join fields.
        let arrays = nth_comma_tail(line, 3);
        let ts = self.seg_ts(raw_ts);
        Some(format!("{ts}|44|{master_index}|{arrays}"))
    }

    /// Emit a code-4 `HEALTH_REGEN` line:
    /// `{ts}|4|{A}|{srcMask}|{tgtMask}|S{state}|T{state}|1|{effectiveRegen}`. The
    /// unit is both source and target (self), so S and T are the same block.
    /// Emit a code-55 `END_TRIAL` line: `{ts}|55|{trialId}|{duration}|{success}|
    /// {finalScore}`. Raw layout: `ts,END_TRIAL,id,duration,success(T/F),score`.
    /// `success` is the `T`/`F` flag mapped to `1`/`0`. A pure trailing marker (no
    /// subordinal/mask/state) like the combat boundaries.
    fn emit_end_trial(&mut self, raw_ts: i64, f: &[&str]) -> Option<String> {
        let trial_id = f.get(2)?.trim();
        let duration = f.get(3)?.trim();
        let success = if f.get(4).map(|s| s.trim()) == Some("T") {
            "1"
        } else {
            "0"
        };
        let final_score = f.get(5).map(|s| s.trim()).unwrap_or("0");
        Some(format!(
            "{ts}|55|{trial_id}|{duration}|{success}|{final_score}",
            ts = self.seg_ts(raw_ts),
        ))
    }

    fn emit_health_regen(&mut self, raw_ts: i64, f: &[&str]) -> Option<String> {
        let effective_regen = f.get(2)?.trim();
        let unit_id = f.get(3)?.trim().to_string();
        // State: the 9 fields after the unit id (raw f[4..=12] → absolute 4..=12).
        let state: Vec<&str> = f.get(4..13)?.iter().map(|s| s.trim()).collect();
        let cp = self.cp_of(&unit_id);
        let block = encode_state_block(&state, &cp)?;
        // HEALTH_REGEN has no abilityId of its own — the parser models it as the
        // synthetic HEALTH_RECOVERY buff (id 61322, spliced into the master ability
        // table). Its tuple is (unit, unit, indexOf(61322)), NOT (unit,unit,0). Using
        // 0 here put every regen at ability-index 0, desyncing the whole tuple table.
        let a = self.alloc_for(&unit_id, "61322", &unit_id);
        // Self-target (src == tgt): both masks are the unit's own side, S == T.
        let (src_mask, tgt_mask) = self.masks(&unit_id, &unit_id);
        let sub = self
            .actors
            .code1_subordinal(&a.to_string(), &unit_id, &unit_id);
        Some(format!(
            "{ts}|4|{sub}|{src_mask}|{tgt_mask}|S{block}|T{block}|1|{effective_regen}",
            ts = self.seg_ts(raw_ts),
        ))
    }

    /// Route + emit an `EFFECT_CHANGED` line. GAINED/FADED/UPDATED × BUFF/DEBUFF →
    /// codes 5/7/6 (buff) or 10/12/11 (debuff). These are *thin* lines: no state
    /// block, `{ts}|{code}|{sub}|{srcMask}|{tgtMask}` (+ a stack count for the
    /// UPDATED codes). The optional trailing `A{ref}` of the official capture is
    /// omitted — it is the unsolved byte-exact global counter and is not needed for
    /// a structurally-valid line.
    fn emit_effect_changed(&mut self, raw_ts: i64, f: &[&str]) -> Option<String> {
        let change_type = f.get(2)?.trim();
        let stack = f.get(3)?.trim().to_string();
        let cast_track_id = f.get(4)?.trim().to_string();
        let ability = f.get(5)?.trim().to_string();
        let src_unit = f.get(6)?.trim().to_string();
        let tgt_field = f.get(16).map(|s| s.trim()).unwrap_or("0");
        let tgt_unit = if tgt_field == "*" {
            src_unit.clone()
        } else {
            tgt_field.to_string()
        };
        // Unit-state shields. Source state is f[7..16]
        // (health,magicka,stamina,ultimate,werewolf,SHIELD,x,y,heading) → shield at
        // f[12]. Target id at f[16], its state at f[17..26] → shield at f[22] (`*`
        // collapses target onto source). A buff carries a trailing shield only when
        // the unit's pool *changed*, per `update_shield_history` below.
        let src_shield: u32 = f.get(12).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let tgt_shield: u32 = if tgt_field == "*" {
            src_shield
        } else {
            f.get(22).and_then(|s| s.trim().parse().ok()).unwrap_or(0)
        };
        // Ability id (numeric) for the frost-safeguard shield exception.
        let ability_id: u32 = ability.parse().unwrap_or(0);

        let is_buff = self
            .effect_type
            .get(&ability)
            .map(|t| t != "DEBUFF")
            .unwrap_or(true); // default BUFF when no EFFECT_INFO seen

        // Stack tracking key (raw unit ids, like the line's masks/subordinal).
        let stack_key = (src_unit.clone(), ability.clone(), tgt_unit.clone());

        // Allocate the tuple at FIRST SIGHT — BEFORE the emit decision. The parser
        // reserves a tuple/effects-table slot the moment an effect is seen, even for
        // an orphan UPDATED (no prior GAINED) or an already-active GAINED whose LINE
        // is suppressed. Allocating here (not after the routing's early returns)
        // keeps our tuple index == the official's: a dropped LINE still consumes its
        // A. `alloc_for` is idempotent on (src,ability,tgt), so this never
        // double-counts a later real emission of the same effect.
        let a = self.alloc_for(&src_unit, &ability, &tgt_unit);

        // Route the change type to a code (and decide whether it emits at all).
        // UPDATED emits ONLY when the stack count *changed* from the instance's
        // previous value, split by direction: a buff increase → 6, a buff decrease
        // → 8, any debuff change → 11. A same-stack re-application (the common
        // case, ~90% of UPDATED) is dropped. The new stack is the trailing field.
        let code = match change_type {
            "GAINED" => {
                // An effect GAINED while it is ALREADY active (re-applied before it
                // faded) does not re-emit — the official segment records one GAINED
                // per activation, not per re-application. `last_stack` holds exactly
                // the currently-active instances (GAINED inserts, FADED removes), so
                // a key already present means "already active". Verified on the
                // capture: this removes 697 spurious code-5 GAINEDs.
                let already_active = self.last_stack.contains_key(&stack_key);
                self.last_stack.insert(stack_key.clone(), stack.clone());
                if already_active {
                    return None;
                }
                if is_buff {
                    "5"
                } else {
                    "10"
                }
            }
            "FADED" => {
                self.last_stack.remove(&stack_key);
                if is_buff {
                    "7"
                } else {
                    "12"
                }
            }
            "UPDATED" => {
                let prev = self.last_stack.insert(stack_key.clone(), stack.clone());
                // An ORPHAN UPDATED (no prior GAINED for this instance — the effect
                // was active before the segment window opened) is not emitted: the
                // official segment has no record to update. Verified on the capture:
                // dropping orphans makes code 6 exact (1050) without regressing 8/11.
                let prev = prev?;
                if prev == stack {
                    return None; // same-stack re-application → not emitted
                }
                if is_buff {
                    // Direction: increase → 6, decrease → 8 (compare numerically; a
                    // non-numeric prior is treated as an increase).
                    let increased = match (prev.parse::<i64>().ok(), stack.parse::<i64>().ok()) {
                        (Some(p), Some(n)) => n > p,
                        _ => true,
                    };
                    if increased {
                        "6"
                    } else {
                        "8"
                    }
                } else {
                    "11"
                }
            }
            _ => return None,
        };

        let (src_mask, tgt_mask) = self.masks(&src_unit, &tgt_unit);
        let sub = self
            .actors
            .code1_subordinal(&a.to_string(), &src_unit, &tgt_unit);
        let ts = self.seg_ts(raw_ts);

        // Shield history. On GAINED/FADED both unit pools are reconciled against the
        // stored history; a side "registers" a shield value only when it *changed*
        // (or ability 146311, frost safeguard, which always registers). The
        // registered value (else 0) is what may print. Runs on GAINED and FADED so
        // the stored pool stays accurate; UPDATED never touches shields.
        // Both updates are guarded on the *source* unit being known (the reference
        // gates `update_target` on `source.unit_id != 0` too): a sourceless effect
        // registers no shield on either side.
        let (reg_src, reg_tgt) =
            if matches!(code, "5" | "10" | "7" | "12") && !src_unit.is_empty() && src_unit != "0" {
                let us = self.update_shield_history(&src_unit, src_shield, ability_id);
                let ut = self.update_shield_history(&tgt_unit, tgt_shield, ability_id);
                (
                    if us { src_shield } else { 0 },
                    if ut { tgt_shield } else { 0 },
                )
            } else {
                (0, 0)
            };

        // A buff/debuff applied by a tracked cast carries a trailing `A{castA}`
        // (the reference's `source_cast_index`) linking it to that cast's tuple.
        // Only GAINED (5/10) and UPDATED-debuff (11) carry it — a FADED has no
        // causing cast. The shield trailing is gated on the cast-ref being present.
        let cast_a = if matches!(code, "5" | "10" | "11") {
            self.cast_track_to_a.get(&cast_track_id).copied()
        } else {
            None
        };
        let cast_ref = cast_a.map(|a| format!("|A{a}")).unwrap_or_default();

        // Shield trailing, only when a cast-ref is present and the two registered
        // pools differ (matching the reference's display gate). Forms:
        //   both ≠ 0           → `|A{cast}|{src}|{tgt}`
        //   only source ≠ 0    → `|A{cast}|{src}`
        //   only target ≠ 0    → `|A{cast}|{tgt}`
        let shield_tail =
            if cast_a.is_some() && (reg_src != 0 || reg_tgt != 0) && reg_src != reg_tgt {
                if reg_src != 0 {
                    if reg_tgt != 0 {
                        format!("|{reg_src}|{reg_tgt}")
                    } else {
                        format!("|{reg_src}")
                    }
                } else {
                    format!("|{reg_tgt}")
                }
            } else {
                String::new()
            };

        // GAINED/FADED (5/7/10/12) are the thin 5-field line; UPDATED (6/8/11)
        // appends the new stack count. Cast-ref then the optional shield follow.
        match code {
            "6" | "8" => Some(format!("{ts}|{code}|{sub}|{src_mask}|{tgt_mask}|{stack}")),
            "11" => Some(format!(
                "{ts}|{code}|{sub}|{src_mask}|{tgt_mask}|{stack}{cast_ref}"
            )),
            _ => Some(format!(
                "{ts}|{code}|{sub}|{src_mask}|{tgt_mask}{cast_ref}{shield_tail}"
            )),
        }
    }

    /// Reconcile a unit's shield pool against history. Returns `true` (the value
    /// "registers", i.e. may print) iff it *changed* from the stored value, or the
    /// ability is frost safeguard (146311) which always registers. Updates the
    /// stored value. A `unit_id` of "0"/empty never registers.
    fn update_shield_history(&mut self, unit_id: &str, shield: u32, ability_id: u32) -> bool {
        if unit_id.is_empty() || unit_id == "0" {
            return false;
        }
        let stored = self.shield_values.get(unit_id).copied().unwrap_or(0);
        if shield != stored || ability_id == 146311 {
            self.shield_values.insert(unit_id.to_string(), shield);
            return true;
        }
        false
    }

    /// Route + emit a `BEGIN_CAST` line. Channeled casts (`channeled == T`) are
    /// dropped; instant casts (`duration == 0`) → code 16, timed → code 15. Form:
    /// `{ts}|{code}|{sub}|{srcMask}|{tgtMask}|C{ctid}|S{srcState}[|T{tgtState}]`,
    /// with the T block present iff the target side is not absent (mask ≠ 32).
    fn emit_begin_cast(&mut self, raw_ts: i64, f: &[&str]) -> Option<String> {
        let duration = f.get(2)?.trim();
        let channeled = f.get(3)?.trim();
        if channeled == "T" {
            return None; // channeled casts do not emit a line
        }
        let cast_track_id = f.get(4)?.trim();
        let ability = f.get(5)?.trim().to_string();
        let src_unit = f.get(6)?.trim().to_string();
        // Source state: 9 fields after the source unit id (f[7..=15]).
        let src_state: Vec<&str> = f.get(7..16)?.iter().map(|s| s.trim()).collect();
        // Target: id at f[16], state at f[17..=25] (may be `*` collapsed).
        let tgt_field = f.get(16).map(|s| s.trim()).unwrap_or("*");
        let (tgt_unit, tgt_state) = self.parse_cast_target(f, tgt_field, &src_unit, &src_state);

        let code = if duration == "0" || duration.is_empty() {
            "16"
        } else {
            "15"
        };
        let a = self.alloc_for(&src_unit, &ability, &tgt_unit);
        // Record this cast's ABILITY INDEX against its track id. A buff applied by
        // this cast carries a trailing `A{n}` where n is this index — the reference's
        // `source_cast_index` is the cast ability's 1-based master index (from
        // `buffs_hashmap[cast_id] = buff_index`), NOT a tuple index. (Storing the
        // tuple A here was the render bug: cast-refs pointed into the wrong table.)
        if !cast_track_id.is_empty() && cast_track_id != "0" {
            // Record this cast's source/target so an END_CAST INTERRUPTED can resolve
            // the interrupted cast's caster (the code-27 line's target).
            self.cast_id_units.insert(
                cast_track_id.to_string(),
                (src_unit.clone(), tgt_unit.clone()),
            );
            let cast_ability_index = self.ability_index(&ability);
            if cast_ability_index != 0 {
                self.cast_track_to_a
                    .insert(cast_track_id.to_string(), cast_ability_index);
            }
            // A TIMED cast (code 15) WITH A PRESENT TARGET gets a thin code-16 line
            // when it later completes (END_CAST COMPLETED). Record its units so that
            // line can reuse this cast's tuple/units. A cast with an absent target
            // (own-side mask 32) emits no completion — verified: the golden sample's
            // timed casts all target `*` and produce zero completions, and on the
            // full capture this gate lands 502 vs the official 494. Instant casts
            // (16) need no record either.
            if code == "15" && self.own_side(&tgt_unit) != "32" {
                self.timed_cast.insert(
                    cast_track_id.to_string(),
                    (a, src_unit.clone(), tgt_unit.clone()),
                );
            }
        }
        let (src_mask, tgt_mask) = self.masks(&src_unit, &tgt_unit);
        let sub = self
            .actors
            .code1_subordinal(&a.to_string(), &src_unit, &tgt_unit);

        let src_cp = self.cp_of(&src_unit);
        let s_block = encode_state_block(&src_state, &src_cp)?;
        let mut line = format!(
            "{ts}|{code}|{sub}|{src_mask}|{tgt_mask}|C{cast_track_id}|S{s_block}",
            ts = self.seg_ts(raw_ts),
        );
        // T block present iff the target side is not absent (mask 32 omits it).
        if tgt_mask != "32" {
            let tgt_cp = self.cp_of(&tgt_unit);
            let t_block = encode_state_block(&tgt_state, &tgt_cp)?;
            line.push_str(&format!("|T{t_block}"));
        }
        Some(line)
    }

    /// Emit a thin code-16 `Cast` line for a COMPLETED `END_CAST` of a TIMED cast.
    /// A timed cast (one that emitted a code-15 `CastWithCastTime` at BEGIN_CAST)
    /// produces a second, thin line when it finishes:
    /// `{ts}|16|{A.inst}|{srcMask}|{tgtMask}` — reusing the original cast's tuple
    /// (same src/tgt/ability) and own-side masks, with NO state blocks. Layout:
    /// `<ts>,END_CAST,<endReason>,<abilityCastId>,...`. Only `COMPLETED` of a
    /// recorded timed cast emits; PLAYER_CANCELLED / INTERRUPTED do not.
    fn emit_end_cast(&mut self, raw_ts: i64, f: &[&str]) -> Option<String> {
        let end_reason = f.get(2)?.trim();
        if end_reason == "INTERRUPTED" {
            return self.emit_interrupt(raw_ts, f);
        }
        if end_reason != "COMPLETED" {
            return None;
        }
        let cast_track_id = f.get(3)?.trim();
        // Reuse the original BEGIN_CAST's tuple A + units (the reference reuses its
        // buff_event verbatim — no new tuple is allocated for the completion line).
        let (a, src_unit, tgt_unit) = self.timed_cast.get(cast_track_id)?.clone();
        let (src_mask, tgt_mask) = self.masks(&src_unit, &tgt_unit);
        let sub = self
            .actors
            .code1_subordinal(&a.to_string(), &src_unit, &tgt_unit);
        Some(format!(
            "{ts}|16|{sub}|{src_mask}|{tgt_mask}",
            ts = self.seg_ts(raw_ts),
        ))
    }

    /// Emit a code-27 `Interrupted` line for an `END_CAST INTERRUPTED`. Raw layout:
    /// `{ts},END_CAST,INTERRUPTED,{interruptedCastId},{interruptedAbility},{interruptingAbility},{interruptingUnit}`.
    ///
    /// The line is `{ts}|27|{sub}|{srcMask}|{tgtMask}|{interruptedAbilityIndex}`:
    /// * source = the **interrupting** unit, target = the **interrupted** cast's
    ///   caster (resolved via [`Self::cast_id_units`], falling back to
    ///   [`Self::last_interrupt`]).
    /// * the subordinal `A` is the tuple `(interruptingUnit, interruptedCaster,
    ///   interruptingAbility)` — which must already exist (the reference only emits
    ///   when its `effects_hashmap` has that key). If it doesn't, the interrupt is
    ///   dropped (matches the official, which omits ~3 of the raw interrupts).
    /// * the trailing field is the interrupted ability's 1-based master index.
    ///
    /// Dropped (returns `None`, no line) when the interrupting unit is 0, the
    /// interrupting and interrupted abilities are identical (a cast can't interrupt
    /// itself), the caster is unknown, or the interrupting tuple was never allocated.
    fn emit_interrupt(&mut self, raw_ts: i64, f: &[&str]) -> Option<String> {
        let interrupted_cast_id = f.get(3)?.trim();
        let interrupted_ability = f.get(4)?.trim();
        let interrupting_ability = f.get(5)?.trim();
        let interrupting_unit = f.get(6)?.trim();
        if interrupting_unit == "0" || interrupting_unit.is_empty() {
            return None;
        }
        if interrupting_ability == interrupted_ability {
            return None; // an ability cannot interrupt itself
        }
        // The interrupted cast's caster is the line's target: look it up by the
        // interrupted cast id, falling back to the last INTERRUPT-status target.
        let caster = self
            .cast_id_units
            .get(interrupted_cast_id)
            .map(|(src, _)| src.clone())
            .or_else(|| self.last_interrupt.clone())?;
        // The tuple must already exist for (interruptingUnit, caster,
        // interruptingAbility) — the reference reuses the existing effect tuple.
        let src_actor = self.actor_index(interrupting_unit);
        let tgt_actor = self.actor_index(&caster);
        let ability_idx = self.ability_index(interrupting_ability);
        let &a = self
            .tuple_to_index
            .get(&(src_actor, tgt_actor, ability_idx))?;
        // Own-side masks (the reference's allegiance_from_reaction per unit): the
        // interrupter and caster are usually cross-faction (16|64) but can be same
        // side (16|16 / 64|64), which the earlier/later ordering can't produce.
        let (src_mask, tgt_mask) = self.masks(interrupting_unit, &caster);
        let sub = self
            .actors
            .code1_subordinal(&a.to_string(), interrupting_unit, &caster);
        let interrupted_idx = self.ability_index(interrupted_ability);
        Some(format!(
            "{ts}|27|{sub}|{src_mask}|{tgt_mask}|{interrupted_idx}",
            ts = self.seg_ts(raw_ts),
        ))
    }

    /// Resolve a cast/combat target's unit id + state, folding the `*` self-target
    /// (which duplicates the source state).
    fn parse_cast_target<'a>(
        &self,
        f: &'a [&'a str],
        tgt_field: &str,
        src_unit: &str,
        src_state: &[&'a str],
    ) -> (String, Vec<&'a str>) {
        if tgt_field == "*" {
            return (src_unit.to_string(), src_state.to_vec());
        }
        let tgt_state: Vec<&str> = f
            .get(17..26)
            .map(|s| s.iter().map(|x| x.trim()).collect())
            .unwrap_or_default();
        (tgt_field.to_string(), tgt_state)
    }

    /// Route + emit a `COMBAT_EVENT` line. Damage-class → code 1, `DOT_TICK*` →
    /// code 2, heals → code 3, `POWER_*` → code 26; status/skip results are
    /// dropped. The prefix is the same as the cast codes plus per-code trailing
    /// fields (crit flag + final for 1/2/3; the power tuple for 26).
    fn emit_combat_event(&mut self, raw_ts: i64, f: &[&str]) -> Option<String> {
        let action_result = f.get(2)?.trim();

        // Raw COMBAT_EVENT layout (absolute indices after the `<ts>,COMBAT_EVENT,`
        // header): [2]actionResult [3]damageType [4]powerType [5]hitValue
        // [6]overflow [7]castTrackId [8]abilityId [9]srcUnit [10..=18]srcState
        // [19]tgtUnit [20..=28]tgtState.
        let power_type = f.get(4)?.trim();
        let hit_value = f.get(5)?.trim();
        let overflow = f.get(6)?.trim();
        let cast_track_id = f.get(7)?.trim();
        let mut ability = f.get(8)?.trim().to_string();
        let src_unit = f.get(9)?.trim().to_string();
        let src_state: Vec<&str> = f.get(10..19)?.iter().map(|s| s.trim()).collect();
        let tgt_field = f.get(19).map(|s| s.trim()).unwrap_or("*");
        let (tgt_unit, tgt_state) = self.parse_combat_target(f, tgt_field, &src_unit, &src_state);

        // A soul-gem resurrection accept carries abilityId 0; the parser maps it to
        // the rez ability 26770 for tuple purposes.
        if ability == "0" && action_result == "SOUL_GEM_RESURRECTION_ACCEPTED" {
            ability = "26770".to_string();
        }

        // SOUL_GEM_RESURRECTION_ACCEPTED → a code-22 Resurrect line: the combat
        // prefix (own-side masks, no `C` cast field) + S + T state blocks, no
        // trailing tail. A resurrect can only ever target a player/companion, whose
        // session instance is 0, so the subordinal is the bare tuple `A`.
        if action_result == "SOUL_GEM_RESURRECTION_ACCEPTED" {
            return self.emit_resurrect(
                raw_ts, &ability, &src_unit, &src_state, &tgt_unit, &tgt_state,
            );
        }

        // A fully-ineffective IN-COMBAT heal (hit 0 AND overflow 0 — healed nothing,
        // no overheal) is dropped: the official segment omits it. Verified EXACT on
        // the Maarselok capture (88 such raw heals, all in-combat, official drops
        // exactly 88 → code-3 2354/2354) and within ~2 on the Ossein trial capture.
        // The in-combat gate matters — out-of-combat zero heals are kept (the Ossein
        // capture keeps several). This mirrors the zero-zero drop the damage path
        // already applies.
        if self.in_combat
            && matches!(action_result, "HEAL" | "CRITICAL_HEAL")
            && hit_value == "0"
            && overflow == "0"
        {
            return None;
        }

        // DAMAGE_SHIELDED → a code-38 line (NOT code-1). It carries the SHIELD
        // ability (e.g. 146311) and references the SHIELD's tuple; its damaging
        // ability is unknown until the paired real DAMAGE/DOT event arrives, so the
        // line is buffered and flushed (back-patched with f10) then.
        if action_result == "DAMAGE_SHIELDED" {
            self.buffer_damage_shielded(raw_ts, hit_value, &ability, &src_unit, &tgt_unit);
            return None; // emitted later via the flush
        }

        // A real damage/dot event flushes any buffered code-38 lines for its target
        // FIRST (stamping their f10 with this event's damaging ability), then emits
        // its own line. The flushed shield lines precede the damage line in output.
        let flushed = if matches!(
            action_result,
            "DAMAGE" | "CRITICAL_DAMAGE" | "DOT_TICK" | "DOT_TICK_CRITICAL" | "BLOCKED_DAMAGE"
        ) {
            self.flush_pending_shields(Some(&tgt_unit), Some(&ability))
        } else {
            Vec::new()
        };

        // An INTERRUPT-status combat event records its target as the last interrupted
        // unit (the reference's `last_interrupt`), the fall-back caster for a later
        // END_CAST INTERRUPTED whose cast id we never saw. It emits no line, but it
        // DOES register its `(src, ability, tgt)` tuple — the reference allocates a
        // buff event for every combat event at the top of the handler, and the later
        // END_CAST INTERRUPTED reuses this exact tuple as its code-27 subordinal `A`.
        // Without it the interrupting ability (e.g. Bash 21973) owns no tuple and
        // both the tuple and its interrupt line are lost.
        if action_result == "INTERRUPT" {
            self.last_interrupt = Some(tgt_unit.clone());
            self.alloc_for(&src_unit, &ability, &tgt_unit);
        }

        // Filter the LINE before allocating: a status/skip/unmodeled combat event
        // neither emits a line NOR registers a tuple here (empirically the official
        // tuple set matches the registering-result rule, not an allocate-for-all
        // rule — allocating for every status event over-produces). The registering
        // rule lives in `combat_event_registers` (used by the master); the routing
        // tables below are its line-level mirror.
        if SKIP_ACTION_RESULTS.contains(&action_result)
            || STATUS_ACTION_RESULTS.contains(&action_result)
        {
            return None;
        }
        let code = if CODE1_ACTION_RESULTS.contains(&action_result) {
            "1"
        } else if CODE19_ACTION_RESULTS.contains(&action_result) {
            "19"
        } else if matches!(action_result, "DOT_TICK" | "DOT_TICK_CRITICAL") {
            "2"
        } else if matches!(action_result, "HEAL" | "CRITICAL_HEAL") {
            "3"
        } else if matches!(action_result, "HOT_TICK" | "HOT_TICK_CRITICAL") {
            // Heal-over-time tick → code 4, same heal-style tail as code 3.
            "4"
        } else if matches!(action_result, "POWER_ENERGIZE" | "POWER_DRAIN") {
            "26"
        } else {
            return None; // unmodeled actionResult → dropped (not guessed)
        };

        // Prepend any flushed code-38 lines (back-patched above) to whatever this
        // damage event itself emits, so the shield run precedes the damage line.
        let prepend = |emitter_line: Option<String>| -> Option<String> {
            match (flushed.is_empty(), emitter_line) {
                (true, l) => l,
                (false, Some(l)) => Some(format!("{}\n{}", flushed.join("\n"), l)),
                (false, None) => Some(flushed.join("\n")),
            }
        };

        // For an actual damage/dot hit (DAMAGE/CRITICAL_DAMAGE/BLOCKED_DAMAGE and the
        // DOT_TICK family), fold any accumulated absorbed shield damage for this
        // target into the overflow and reset the buffer (the reference's
        // temporary_damage_buffer). IMMUNE/DODGED never carry damage, so they don't
        // fold. A fully-absorbed (hit 0) hit is dropped ONLY when nothing was absorbed
        // either — otherwise it is emitted with the absorbed total in overflow.
        let folds_damage = matches!(
            action_result,
            "DAMAGE" | "CRITICAL_DAMAGE" | "BLOCKED_DAMAGE" | "DOT_TICK" | "DOT_TICK_CRITICAL"
        );
        let mut folded_overflow = overflow.parse::<u64>().unwrap_or(0);
        if matches!(code, "1" | "2") && folds_damage {
            let absorbed = self.temp_damage.remove(&tgt_unit).unwrap_or(0);
            folded_overflow += absorbed;
            // Only code-1 (direct damage) drops a fully-zero event; the official keeps
            // every DOT_TICK (code-2 is count-exact at 2938) and just folds overflow.
            if code == "1" {
                let hit = hit_value.parse::<u64>().unwrap_or(0);
                if hit == 0 && folded_overflow == 0 {
                    return prepend(None); // nothing happened — dropped (matches official)
                }
            }
        }

        let a = self.alloc_for(&src_unit, &ability, &tgt_unit);
        // Mask rule is code-dependent (verified on the capture): the DAMAGE codes
        // (1 damage, 2 dot) use the earlier/later ordering (16/64 by master index —
        // a cross-faction attacker/victim pair), while the heal/power codes (3 heal,
        // 4 hot, 26 power) use OWN-SIDE masks (source and target are always the same
        // faction — you heal/energize allies — so both slots are that side: 16|16 for
        // friendlies, 64|64 for hostiles, never crossed).
        let (src_mask, tgt_mask) = if matches!(code, "3" | "4" | "26") {
            self.masks(&src_unit, &tgt_unit)
        } else {
            self.combat_masks(&src_unit, &tgt_unit)
        };
        let sub = self
            .actors
            .code1_subordinal(&a.to_string(), &src_unit, &tgt_unit);

        let mut line = format!(
            "{ts}|{code}|{sub}|{src_mask}|{tgt_mask}|C{cast_track_id}",
            ts = self.seg_ts(raw_ts),
        );
        // S block present iff src side present (mask != 32); same for T.
        if src_mask != "32" {
            let s_block = match encode_state_block(&src_state, &self.cp_of(&src_unit)) {
                Some(b) => b,
                None => return prepend(None),
            };
            line.push_str(&format!("|S{s_block}"));
        }
        if tgt_mask != "32" {
            let t_block = match encode_state_block(&tgt_state, &self.cp_of(&tgt_unit)) {
                Some(b) => b,
                None => return prepend(None),
            };
            line.push_str(&format!("|T{t_block}"));
        }
        // Code 19 (death) is the combat prefix + S + T with NO trailing tail (the
        // target's state already shows 0 health) — emit it as-is.
        if code == "19" {
            return prepend(Some(line));
        }
        // Code 1 (damage/immune/dodged/blocked) and code 2 (dot) share the unified
        // tail, derived from the capture. It folds the absorbed shield damage into
        // overflow (computed above) and uses the killing-blow form when the target's
        // health is 0.
        if matches!(code, "1" | "2") {
            let target_dead = tgt_state
                .first()
                .and_then(|s| s.split('/').next())
                .map(|h| h.trim() == "0")
                .unwrap_or(false);
            let hit = hit_value.parse::<u64>().unwrap_or(0);
            match super::encode::code1_tail(action_result, hit, folded_overflow, target_dead) {
                Some(tail) => {
                    line.push('|');
                    line.push_str(&tail);
                    return prepend(Some(line));
                }
                None => return prepend(None),
            }
        }
        // Append the per-code trailing fields. If the required tail can't be
        // formed (an actionResult whose crit/final we don't model byte-safely),
        // DROP the whole event rather than emit a structurally-incomplete line —
        // a code-1/2/3/26 line missing its trailing fields is malformed. The
        // dropped event is rare and out-of-combat-ish; the coverage gate keeps the
        // log off native regardless until the live round-trip confirms the format.
        // The code-26 pool max is the TARGET unit's max for the energized resource
        // (magicka(1)→state[1], stamina(4)→state[2], ultimate(8)→state[3], `cur/MAX`
        // → MAX), with the resource index remapped (1→0, 4→1, 8→2). Resolve it here
        // (where the target state is in scope) and pass the formed power tail.
        let power_tail = if code == "26" {
            let (power_idx, slot) = match power_type.trim() {
                "1" => ("0", 1),
                "4" => ("1", 2),
                "8" => ("2", 3),
                _ => ("0", 1),
            };
            let power_max = tgt_state
                .get(slot)
                .and_then(|s| s.split('/').nth(1))
                .unwrap_or("0");
            Some(format!("{power_idx}|{power_max}"))
        } else {
            None
        };
        if !self.append_combat_tail(
            &mut line,
            code,
            action_result,
            hit_value,
            overflow,
            power_tail.as_deref(),
        ) {
            return prepend(None);
        }
        prepend(Some(line))
    }

    /// Emit a code-22 `Resurrect` line for a `SOUL_GEM_RESURRECTION_ACCEPTED`
    /// combat event. Format (verified on the capture):
    /// `{ts}|22|{A}|{srcMask}|{tgtMask}|S{srcState}|T{tgtState}` — the combat prefix
    /// with **own-side** masks and **no `C` cast field** (the reference uses
    /// `cast_id_origin: 0`), and **no trailing crit/final tail** (`cast_information:
    /// None`). The subordinal is the bare tuple `A` (a resurrect targets a
    /// player/companion, whose session instance ordinal is 0). The tuple is keyed on
    /// `(src, 26770, tgt)` — the rez ability the parser substitutes for ability 0.
    fn emit_resurrect(
        &mut self,
        raw_ts: i64,
        ability: &str,
        src_unit: &str,
        src_state: &[&str],
        tgt_unit: &str,
        tgt_state: &[&str],
    ) -> Option<String> {
        let a = self.alloc_for(src_unit, ability, tgt_unit);
        let (src_mask, tgt_mask) = self.masks(src_unit, tgt_unit);
        let mut line = format!(
            "{ts}|22|{a}|{src_mask}|{tgt_mask}",
            ts = self.seg_ts(raw_ts)
        );
        if src_mask != "32" {
            let s_block = encode_state_block(src_state, &self.cp_of(src_unit))?;
            line.push_str(&format!("|S{s_block}"));
        }
        if tgt_mask != "32" {
            let t_block = encode_state_block(tgt_state, &self.cp_of(tgt_unit))?;
            line.push_str(&format!("|T{t_block}"));
        }
        Some(line)
    }

    /// Buffer a code-38 DamageShielded line from a DAMAGE_SHIELDED combat event. The
    /// line references the SHIELD's tuple and the damage source, but its damaging
    /// ability (`f10`) is unknown until the paired real DAMAGE/DOT event; the line is
    /// held until [`Self::flush_pending_shields`] back-patches and emits it.
    ///
    /// Fields (from the empirically-verified layout
    /// `{ts}|38|{A}|{f4}|{f5}|{f6}|{f7}|{f8}|0|{hit}|{f10}`):
    /// * `{A}` = the shield tuple `(target, shieldAbility, target)` — a self-shield
    ///   on the damage target (the absorbing ward). Allocated like any tuple.
    /// * `{f4}` = `{f5}` = the damage target's (shield owner's) own-side mask.
    /// * `{f6}` = the damage source's 1-based actor index.
    /// * `{f7}` = the damage source's per-fight session/instance index (0 if none).
    /// * `{f8}` = the damage source's own-side mask.
    /// * `{hit}` = the absorbed hit value (raw, not accumulated).
    ///
    /// A zero hit absorbs nothing → no line.
    fn buffer_damage_shielded(
        &mut self,
        raw_ts: i64,
        hit_value: &str,
        shield_ability: &str,
        src_unit: &str,
        tgt_unit: &str,
    ) {
        if hit_value == "0" || hit_value.is_empty() {
            return;
        }
        // Accumulate the absorbed amount for this target so the paired real damage
        // event folds it into its overflow (the reference's temporary_damage_buffer).
        if let Ok(v) = hit_value.parse::<u64>() {
            *self.temp_damage.entry(tgt_unit.to_string()).or_insert(0) += v;
        }
        // The DAMAGE_SHIELDED combat event allocates its OWN tuple first — keyed on
        // (damageSource, shieldAbility, damageTarget), e.g. `9|3|210` — exactly like
        // any combat event (the reference allocates buff_event before the
        // DamageShielded arm). This is distinct from the shield's self-tuple the line
        // references, and must be minted at the event's position so the A numbering
        // stays aligned with the official table.
        self.alloc_for(src_unit, shield_ability, tgt_unit);
        // The shield's tuple: a self-shield on the damage target (target absorbs with
        // its own ward). alloc_for is idempotent, so this mints the shield self-tuple
        // (e.g. Frost Safeguard `3|3|210`) if not already present.
        let a = self.alloc_for(tgt_unit, shield_ability, tgt_unit);
        let sub = self
            .actors
            .code1_subordinal(&a.to_string(), tgt_unit, tgt_unit);
        let shield_mask = self.own_side(tgt_unit); // f4 == f5 (shield owner)
        let dmg_src_actor = self.actor_index(src_unit); // f6
        let dmg_src_session = self.actors.session_index(src_unit); // f7
        let dmg_src_mask = self.own_side(src_unit); // f8
        let prefix = format!(
            "{ts}|38|{sub}|{shield_mask}|{shield_mask}|{dmg_src_actor}|{dmg_src_session}|{dmg_src_mask}|0|{hit_value}",
            ts = self.seg_ts(raw_ts),
        );
        self.pending_shields.push(PendingShield {
            prefix,
            target_unit: tgt_unit.to_string(),
        });
    }

    /// Flush buffered code-38 lines, appending the damaging-ability index (`f10`) to
    /// each. Called by the paired real damage event (with `tgt`/`ability` set) — only
    /// the pending lines whose target matches are flushed — and at end of stream
    /// (both `None`) to drain any leftover lines with `f10 = 0`. Returns the finished
    /// lines in buffered (timestamp) order.
    fn flush_pending_shields(&mut self, tgt: Option<&str>, ability: Option<&str>) -> Vec<String> {
        if self.pending_shields.is_empty() {
            return Vec::new();
        }
        // The damaging ability's 1-based master index (+0 placeholder at end of
        // stream, matching the reference's un-patched `usize::MAX.wrapping_add(1)`).
        let f10 = ability.map(|ab| self.ability_index(ab)).unwrap_or(0);
        let mut out = Vec::new();
        let mut kept = Vec::new();
        // `take` so we can re-borrow self while iterating.
        for p in std::mem::take(&mut self.pending_shields) {
            let matches_target = tgt.map(|t| t == p.target_unit).unwrap_or(true);
            if matches_target {
                out.push(format!("{}|{f10}", p.prefix));
            } else {
                kept.push(p);
            }
        }
        self.pending_shields = kept;
        out
    }

    /// Resolve a combat target's unit id + state (target state at raw f[20..=28]),
    /// folding the `*` self-target.
    fn parse_combat_target<'a>(
        &self,
        f: &'a [&'a str],
        tgt_field: &str,
        src_unit: &str,
        src_state: &[&'a str],
    ) -> (String, Vec<&'a str>) {
        if tgt_field == "*" {
            return (src_unit.to_string(), src_state.to_vec());
        }
        let tgt_state: Vec<&str> = f
            .get(20..29)
            .map(|s| s.iter().map(|x| x.trim()).collect())
            .unwrap_or_default();
        (tgt_field.to_string(), tgt_state)
    }

    /// Append the per-code trailing fields after the state blocks. **Returns
    /// `true` iff a complete, structurally-valid tail was appended** — the caller
    /// drops the event when this is `false`, so a code-1/2/3/26 line is never
    /// emitted with a missing/partial tail.
    ///
    /// * code 3 (heal): `|{critFlag}|{final}` with the heal crit scheme; heals append
    ///   the overflow (overheal) when nonzero.
    /// * code 26 (power): `|{hitValue}|{overflow}|{powerTypeIdx}|{powerMax}`.
    ///
    /// Codes 1 (damage) and 2 (dot) are formed by the unified [`super::encode::code1_tail`]
    /// at the call site (they need the folded overflow + the target-dead flag), so
    /// they never reach here.
    fn append_combat_tail(
        &self,
        line: &mut String,
        code: &str,
        action_result: &str,
        hit_value: &str,
        overflow: &str,
        power_tail: Option<&str>,
    ) -> bool {
        match code {
            // Heal (3) and heal-over-time tick (4) share the same heal-style tail.
            "3" | "4" => match combat_noncode1_crit_flag(action_result) {
                Some(crit) => {
                    if overflow == "0" {
                        line.push_str(&format!("|{crit}|{hit_value}"));
                    } else {
                        // Overheal: the effective heal (hit + overflow) then the
                        // overflow amount.
                        let effective = hit_value
                            .parse::<i64>()
                            .ok()
                            .zip(overflow.parse::<i64>().ok())
                            .map(|(h, o)| (h + o).to_string())
                            .unwrap_or_else(|| hit_value.to_string());
                        line.push_str(&format!("|{crit}|{effective}|{overflow}"));
                    }
                    true
                }
                None => false,
            },
            "26" => match power_tail {
                // `{hit}|{overflow}|{resourceIdx}|{poolMax}` — the resource index +
                // target pool max are resolved by the caller (target state in scope).
                Some(pt) => {
                    line.push_str(&format!("|{hit_value}|{overflow}|{pt}"));
                    true
                }
                None => false,
            },
            // No other code reaches append_combat_tail (only 1/2/3/26 do).
            _ => false,
        }
    }

    /// `(srcMask, tgtMask)` for any event with two unit sides. For two distinct
    /// units the proven code-1 ordering (earlier→16, later→64, absent→32) applies.
    /// For a **self-targeted** event (src == tgt, including the folded `*` target)
    /// both slots are the unit's OWN side mask — `16|16` for a friendly unit,
    /// `64|64` for a hostile one — which `code1_masks` deliberately gates (equal
    /// keys). A `16|16` fallback is only correct for friendly self-events; a
    /// hostile monster's self-buff is `64|64` (verified on the combat capture,
    /// e.g. `33|5|7.1.1`).
    ///
    /// **Effect/cast/regen masks are OWN-SIDE**, not the code-1 earlier/later
    /// ordering: each slot is that unit's own reaction side (`16` friendly, `64`
    /// hostile), and an absent unit (id `0`/`*`/unknown) is `32`. Verified on the
    /// combat capture — a buff from player 6 to player 5 is `16|16` (both
    /// friendly), where the earlier/later rule would wrongly give `16|64`. The
    /// earlier/later relative ordering is specific to the combat codes
    /// ([`Self::combat_masks`]).
    fn masks(&self, src_unit: &str, tgt_unit: &str) -> (&'static str, &'static str) {
        (self.own_side(src_unit), self.own_side(tgt_unit))
    }

    /// A unit's own-side mask: `16` friendly, `64` hostile, `32` absent/unknown.
    fn own_side(&self, unit_id: &str) -> &'static str {
        let u = unit_id.trim();
        if u.is_empty() || u == "0" || u == "*" {
            return "32";
        }
        self.actors.side_mask(u).unwrap_or("32")
    }

    /// `(srcMask, tgtMask)` for the **combat** codes (1/2/3/26), which use the
    /// proven earlier/later relative ordering ([`ActorTable::code1_masks`]):
    /// earlier→16, later→64, absent→32, with self/co-located falling back to the
    /// own side. Byte-exact 3733/3733 on the code-1 golden pair.
    fn combat_masks(&self, src_unit: &str, tgt_unit: &str) -> (&'static str, &'static str) {
        if let Some(masks) = self.actors.code1_masks(src_unit, tgt_unit) {
            return masks;
        }
        let side = self.actors.side_mask(src_unit).unwrap_or("16");
        (side, side)
    }
}

/// Build the complete **fights-segment text** for a raw log — the analog of
/// [`super::encode::build_master_table`] for the events half. Frames the assembled
/// events with the segment header
/// (`{logVersion}|{gameVersion}\n{totalEventCount}\n{events}`) via
/// [`super::serialize::FightsSegmentDoc`].
///
/// Single-fight framing (the whole log's events as one fight); live/multi-fight
/// segmentation layers on top of this. Returns `None` if the log has no
/// `BEGIN_LOG` (not a valid session). The result is the text to ZIP via
/// [`super::zip_segment::zip_log_txt`] into the `logfile` upload field.
pub fn build_fights_segment(lines: &[&str]) -> Option<String> {
    // Run the emitter with the master-table index maps so its `A` is the tuple
    // index (the standalone path — tests/diagnostics; the production path shares
    // the tuple table via `build_native_payload`).
    let (id2a, ab2i) = super::encode::actor_ability_maps(lines);
    let mut emitter = EventEmitter::with_master_indices(id2a, ab2i);
    render_segment(lines, &mut emitter)
}

/// Frame an emitter's assembled events into the fights-segment text. Returns
/// `None` if the log has no `BEGIN_LOG`.
fn render_segment(lines: &[&str], emitter: &mut EventEmitter) -> Option<String> {
    use super::serialize::FightsSegmentDoc;
    let log_version = lines.iter().find_map(|l| {
        let f = split_csv_quoted_pub(l);
        if f.get(1).map(|s| s.trim()) == Some("BEGIN_LOG") {
            f.get(3).map(|s| s.trim().to_string())
        } else {
            None
        }
    })?;
    let out = emitter.build(lines);
    let doc = FightsSegmentDoc {
        log_version: &log_version,
        game_version: "1", // observed constant, matches the master table
        fights: &[(out.event_count, &out.events_string)],
    };
    Some(doc.render())
}

/// Build the complete, ready-to-upload native payload for a raw log: the ZIP'd
/// fights segment + the ZIP'd master table, paired for one `add-report-segment`
/// call. This is the single seam a native [`super::super::transport`] impl calls —
/// it turns scanner-provided raw lines into the exact
/// ([`super::client::Segment`], [`super::client::MasterTableBytes`]) the
/// [`super::client::NativeUpload::upload_finished`] lifecycle consumes.
///
/// Returns `None` if the log is not a valid session (no `BEGIN_LOG`), and `Err`
/// only on an internal ZIP failure. The segment and master table are built from
/// the *same* lines so their interned ids line up.
pub fn build_native_payload(
    lines: &[&str],
) -> Result<Option<(super::client::Segment, super::client::MasterTableBytes)>, String> {
    use super::client::{MasterTableBytes, Segment};
    use super::encode::{actor_ability_maps, build_master_table_with_tuples};

    // Build the segment with an emitter that holds the master index maps, so its
    // `A` values are tuple indices. Then build the master table using THAT
    // emitter's tuple table — the segment's `A` references and the master's tuple
    // section share one numbering, which is what lets the uploaded report render.
    let (id2a, ab2i) = actor_ability_maps(lines);
    let mut emitter = EventEmitter::with_master_indices(id2a, ab2i);
    let Some(segment_text) = render_segment(lines, &mut emitter) else {
        return Ok(None); // not a valid session
    };
    let Some(master_text) = build_master_table_with_tuples(lines, emitter.tuples()) else {
        return Ok(None);
    };
    // Structural self-check BEFORE we ZIP and upload: a malformed segment (a short/
    // non-numeric line, a wrong declared count, or an `A` that points past the tuple
    // table) would be ACCEPTED by the server but never render — the exact "loads
    // forever" failure. The encoder is proven on two captures, but this guards
    // against an unseen log shape slipping a broken segment to a real user's account.
    // On failure the caller falls back to the official uploader (never ships native).
    validate_segment_text(&segment_text, emitter.allocated())?;
    // The segment's wall-clock time bounds — the `add-report-segment` request sends
    // these so the server can place the segment on the timeline and extract fights.
    let (start_time, end_time) = segment_time_bounds(lines);
    let segment = Segment::from_text(&segment_text, start_time, end_time)?;
    let master = MasterTableBytes::from_text(&master_text)?;
    Ok(Some((segment, master)))
}

/// Codes whose subordinal field LEADS with the tuple index `A` (so the leading
/// `A.b.c` must reference a real master tuple). The pure markers (41/51/52/53/55)
/// and the trial/zone lines carry literal ids or nothing in that slot, so they are
/// not range-checked. Mirrors the `a_bearing` set the structural test asserts.
const A_BEARING_CODES: &[&str] = &[
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "15", "16", "19", "22", "26",
    "27", "28", "38", "44",
];

/// Validate that a built fights-segment is **structurally uploadable**: the server
/// re-parses any well-formed segment but the renderer needs the events present and
/// internally consistent, so a structurally-broken segment renders as an infinite
/// load. This is the runtime gate the production upload path runs before sending —
/// a cheap O(lines) pass that mirrors the invariants the test suite proves offline.
///
/// `max_a` is the number of allocated tuples (`EventEmitter::allocated()`); the
/// master tuples section has exactly this many records, so every event's leading
/// `A` must be in `1..=max_a` or it references a tuple that does not exist.
///
/// Returns `Err` (not a panic) so the caller can fall back to the official uploader
/// rather than ship a segment that would not render.
pub(crate) fn validate_segment_text(segment_text: &str, max_a: u32) -> Result<(), String> {
    let mut lines = segment_text.lines();
    // Line 1: `{logVersion}|{gameVersion}` (>=2 fields). Line 2: the event count.
    let header = lines.next().ok_or("empty segment")?;
    if header.split('|').count() < 2 {
        return Err(format!("segment header malformed: {header:?}"));
    }
    let declared: u64 = lines
        .next()
        .ok_or("segment missing event-count line")?
        .trim()
        .parse()
        .map_err(|_| "segment event-count line is not a number".to_string())?;

    let mut body_count: u64 = 0;
    for line in lines {
        body_count += 1;
        let mut f = line.split('|');
        let ts = f.next().unwrap_or("");
        let code = f
            .next()
            .ok_or_else(|| format!("line too short: {line:?}"))?;
        if ts.parse::<u64>().is_err() {
            return Err(format!("non-numeric timestamp: {line:?}"));
        }
        if A_BEARING_CODES.contains(&code) {
            // The subordinal is `A.b.c`; only the leading `A` is the tuple index.
            let sub = f
                .next()
                .ok_or_else(|| format!("missing subordinal: {line:?}"))?;
            let a: u32 = sub
                .split('.')
                .next()
                .unwrap_or("")
                .parse()
                .map_err(|_| format!("non-numeric subordinal A: {line:?}"))?;
            if a < 1 || a > max_a {
                return Err(format!(
                    "subordinal A={a} out of range 1..={max_a} (dangling tuple ref): {line:?}"
                ));
            }
        }
    }
    if body_count != declared {
        return Err(format!(
            "declared event count {declared} != {body_count} emitted lines"
        ));
    }
    Ok(())
}

/// The segment's `(startTime, endTime)` in **absolute wall-clock ms** — the values
/// the `add-report-segment` request needs. `startTime` is the `BEGIN_LOG` wall-clock
/// (its field-2 ms) plus its own relative timestamp (field 0); `endTime` is that
/// same wall anchor plus the LAST event's relative timestamp. Mirrors the reference
/// uploader: `first = beginLogWall + beginLogRelTs`, `last = first + lastEventRelTs`.
/// Without these the server receives a zero-width segment and finds no fights.
fn segment_time_bounds(lines: &[&str]) -> (u64, u64) {
    let mut begin_wall: u64 = 0;
    let mut begin_rel: u64 = 0;
    let mut found_begin = false;
    let mut last_rel: u64 = 0;
    for line in lines {
        let f = split_csv_quoted_pub(line);
        let kind = f.get(1).map(|s| s.trim()).unwrap_or("");
        let rel: u64 = f.first().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        if kind == "BEGIN_LOG" && !found_begin {
            begin_wall = f.get(2).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            begin_rel = rel;
            found_begin = true;
        }
        // Track the last line's relative ts (the segment ends at the last event).
        last_rel = rel;
    }
    if !found_begin {
        return (0, 0);
    }
    let start = begin_wall + begin_rel;
    let end = begin_wall + last_rel;
    (start, end)
}

/// Map a raw `powerType` to the segment's `(powerTypeIdx, powerMax)` pair for a
/// code-26 (power) line. `powerType` is an ESO power-mechanic ordinal; the segment
/// remaps it to a small index and pairs it with the relevant pool max. Only the
/// observed mappings are encoded; an unknown type falls back to a structurally
/// valid `(0, 0)`.
/// The comma tail after `<ts>,<TYPE>,` (i.e. everything the inner encoders parse).
fn tail(line: &str) -> &str {
    line.splitn(3, ',').nth(2).unwrap_or("")
}

/// The substring after the `n`-th comma (0-based count of commas to skip past the
/// leading fields). Used for `PLAYER_INFO` where the trailing arrays contain
/// commas inside `[...]` and must be passed through verbatim.
fn nth_comma_tail(line: &str, skip: usize) -> &str {
    let mut idx = 0;
    let mut seen = 0;
    for (i, b) in line.bytes().enumerate() {
        if b == b',' {
            seen += 1;
            if seen == skip {
                idx = i + 1;
                break;
            }
        }
    }
    &line[idx..]
}

/// The result of assembling one session's events: the events string (each line
/// `\n`-terminated) plus the number of event lines emitted (the per-fight
/// `eventCount` the segment header sums).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventsOutput {
    pub events_string: String,
    pub event_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_and_status_sets_are_the_verified_sizes() {
        assert_eq!(SKIP_ACTION_RESULTS.len(), 15);
        assert_eq!(STATUS_ACTION_RESULTS.len(), 13);
        // DAMAGE_SHIELDED is no longer a code-1 result — it routes to code-38.
        assert_eq!(CODE1_ACTION_RESULTS.len(), 6);
        assert_eq!(CODE19_ACTION_RESULTS.len(), 2);
    }

    // Robustness: a DROPPED line preceding the first emitted line must NOT steal
    // the ts-0 anchor. A DAMAGE_SHIELDED (dropped — no byte-safe tail) before the
    // first ZONE_CHANGED must still leave that ZONE at segTs 0.
    #[test]
    fn dropped_line_does_not_steal_the_ts_anchor() {
        let state = "18400/18400,9971/12868,8488/28700,198/500,0/1000,0,0.5,0.7,5.9";
        let tgt = "40000/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0";
        // DAMAGE_SHIELDED at raw ts 7 is dropped; ZONE at raw ts 10 is the first emit.
        let shielded =
            format!("7,COMBAT_EVENT,DAMAGE_SHIELDED,FIRE,1,500,0,5000,100,1,{state},30,{tgt}");
        let lines = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            "0,UNIT_ADDED,30,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F",
            &shielded,
            "10,ZONE_CHANGED,1129,\"Hall\",NORMAL",
        ];
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        let first = out.events_string.lines().next().expect("an emitted line");
        assert!(
            first.starts_with("0|41|"),
            "the first emitted event (the ZONE) must anchor at segTs 0, got: {first}"
        );
    }

    // A DIED / DIED_XP COMBAT_EVENT emits a code-19 line: the combat prefix +
    // S + T state blocks, with NO trailing crit/final tail (the target shows 0 HP).
    #[test]
    fn death_emits_code_19_without_a_tail() {
        let state = "18400/18400,9971/12868,8488/28700,198/500,0/1000,0,0.5,0.7,5.9";
        let dead = "0/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0";
        let death_line =
            format!("6,COMBAT_EVENT,DIED_XP,DISEASE,1,92,0,5000,100,1,{state},30,{dead}");
        let lines = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            "0,UNIT_ADDED,30,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F",
            "5,ZONE_CHANGED,1129,\"Hall\",NORMAL",
            &death_line,
        ];
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        let code19 = out
            .events_string
            .lines()
            .find(|l| l.split('|').nth(1) == Some("19"))
            .expect("DIED_XP emits a code-19 line");
        // Combat prefix + C + S + T, ending at the T block's last field (a heading),
        // with NO crit/final tail and NO trailing pipe.
        assert!(
            code19.contains("|C5000|S"),
            "must carry C + S block: {code19}"
        );
        assert!(
            code19.contains("|T0/45000"),
            "must carry the dead T block: {code19}"
        );
        // The last field is the target heading (0), not a crit/final pair.
        assert!(code19.ends_with("|0"), "no crit/final tail: {code19}");
        // It must NOT be routed to code 1.
        assert!(
            !out.events_string
                .lines()
                .any(|l| l.split('|').nth(1) == Some("1")),
            "a death must not emit a code-1 line"
        );
    }

    // The tuple allocation: A is the 1-based index of an event's (srcActor,
    // tgtActor, abilityIndex) tuple, in first-emission order; the same tuple reuses
    // its index. This is what ties the segment's A to the master tuple table.
    #[test]
    fn tuple_alloc_is_first_sight_and_dense() {
        let mut e = EventEmitter::new();
        // (srcActor=4, tgtActor=5, ability=150)
        let a1 = e.alloc_tuple(4, 5, 150);
        let a2 = e.alloc_tuple(4, 5, 150); // same tuple
        let a3 = e.alloc_tuple(4, 6, 150); // different target → new tuple
        let a4 = e.alloc_tuple(4, 5, 146); // different ability → new tuple
        assert_eq!(a1, 1);
        assert_eq!(a2, 1, "same tuple reuses its index (== A)");
        assert_eq!(a3, 2);
        assert_eq!(a4, 3);
        assert_eq!(e.allocated(), 3);
        assert_eq!(e.tuples(), &[(4, 5, 150), (4, 6, 150), (4, 5, 146)]);
    }

    #[test]
    fn nth_comma_tail_passes_through_arrays() {
        // PLAYER_INFO: skip `<ts>,PLAYER_INFO,<unitId>,` (3 commas) → arrays verbatim.
        let line = "1,PLAYER_INFO,1,[10,20],[1,1],[[A,B]],[5],[6]";
        assert_eq!(nth_comma_tail(line, 3), "[10,20],[1,1],[[A,B]],[5],[6]");
    }

    // The full fights-segment text frames the events with the right header: the
    // log version from BEGIN_LOG, the event count, then the events. The body must
    // match the assembled events (modulo the optional A-ref).
    #[test]
    fn full_segment_frames_header_count_and_events() {
        let raw = include_str!("testdata/sample_raw_encounter.log");
        let lines: Vec<&str> = raw.lines().collect();
        let segment = build_fights_segment(&lines).expect("build segment");

        let mut it = segment.splitn(3, '\n');
        let header = it.next().unwrap();
        let count_line = it.next().unwrap();
        let body = it.next().unwrap();

        // header = {logVersion}|{gameVersion}; the sample's BEGIN_LOG version is 15.
        assert_eq!(
            header, "15|1",
            "segment header must be logVersion|gameVersion"
        );
        // count = number of emitted events (45 for this sample).
        let count: u64 = count_line.parse().unwrap();
        assert_eq!(count, 45, "event count must be the number of emitted lines");
        assert_eq!(
            body.lines().count() as u64,
            count,
            "the body must contain exactly `count` event lines"
        );
        // The body's first line is the first event (the zone change).
        assert!(body.starts_with("0|41|1129|Hall of the Lunar Champion|0\n"));
    }

    #[test]
    fn build_fights_segment_needs_a_begin_log() {
        // No BEGIN_LOG → not a valid session → None.
        let lines = vec!["4,ZONE_CHANGED,1129,\"Hall\",NONE"];
        assert!(build_fights_segment(&lines).is_none());
    }

    // Code-44 (PLAYER_INFO) end-to-end: the line is `{segTs}|44|{masterIndex}|`
    // followed by the raw bracketed arrays passed through verbatim. The master
    // index is the unit's dense 1-based actor index (NOT the raw unit id), and the
    // nested equipment array (which contains commas) must survive intact.
    #[test]
    fn player_info_emits_master_index_and_verbatim_arrays() {
        let lines = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA Megaserver\",\"en\",\"eso.live.11.3\"",
            // Two players: raw unit ids 1 and 7 → master indices 1 and 2.
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            "0,UNIT_ADDED,7,PLAYER,F,2,0,F,3,9,\"Ally\",\"@ally\",222,50,1500,0,PLAYER_ALLY,T",
            // A zone change is the first emitted event (anchors segTs 0 at raw ts 5).
            "5,ZONE_CHANGED,1129,\"Hall\",NORMAL",
            // PLAYER_INFO for raw unit 7 (master index 2). Arrays contain commas,
            // including a nested [[...]] equipment list. Raw ts 8 → segTs 8−5 = 3.
            "8,PLAYER_INFO,7,[142210,86673],[1,1],[[HEAD,94773,T,16,ARMOR_DIVINES,LEGENDARY]],[63046],[40382]",
        ];
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        let code44 = out
            .events_string
            .lines()
            .find(|l| l.split('|').nth(1) == Some("44"))
            .expect("a code-44 line");
        // {segTs}|44|{masterIndex=2}|{arrays verbatim}. The first emitted event (the
        // ZONE_CHANGED at raw ts 5) anchors segTs 0, so PLAYER_INFO at raw ts 8 → 3.
        assert_eq!(
            code44,
            "3|44|2|[142210,86673],[1,1],[[HEAD,94773,T,16,ARMOR_DIVINES,LEGENDARY]],[63046],[40382]",
            "code-44 must carry the dense master index and verbatim arrays"
        );
    }

    // A code-1 event whose tail can't be byte-safely formed (e.g. DAMAGE_SHIELDED)
    // is DROPPED, never emitted as a structurally-incomplete prefix-only line. A
    // status result with a constant final (IMMUNE) IS emitted, complete.
    #[test]
    fn incomplete_combat_tail_drops_the_event() {
        let prelude = [
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            "0,UNIT_ADDED,30,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F",
        ];
        let state = "18400/18400,9971/12868,8488/28700,198/500,0/1000,0,0.5,0.7,5.9";
        let tgt = "40000/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0";

        // DAMAGE_SHIELDED → combat_final_field returns None → event dropped.
        let mut e = EventEmitter::new();
        let shielded =
            format!("6,COMBAT_EVENT,DAMAGE_SHIELDED,FIRE,1,500,0,5000,100,1,{state},30,{tgt}");
        let mut lines: Vec<&str> = prelude.to_vec();
        lines.push(&shielded);
        let out = e.build(&lines);
        assert!(
            !out.events_string
                .lines()
                .any(|l| l.split('|').nth(1) == Some("1")),
            "DAMAGE_SHIELDED must not emit a (prefix-only) code-1 line: {:?}",
            out.events_string
        );

        // IMMUNE → a complete code-1 line with the single result flag `10` (the
        // official segment emits a bare `|10`, NOT `|1|10` — IMMUNE carries no hit
        // value, so hit/overflow/blocked are zeroed and stripped).
        let mut e2 = EventEmitter::new();
        let immune = format!("6,COMBAT_EVENT,IMMUNE,FIRE,1,0,0,5000,100,1,{state},30,{tgt}");
        let mut lines2: Vec<&str> = prelude.to_vec();
        lines2.push(&immune);
        let out2 = e2.build(&lines2);
        let code1 = out2
            .events_string
            .lines()
            .find(|l| l.split('|').nth(1) == Some("1"))
            .expect("IMMUNE emits a complete code-1 line");
        // It must end with the single IMMUNE result flag (10), no crit prefix.
        assert!(
            code1.ends_with("|10") && !code1.ends_with("|1|10"),
            "IMMUNE code-1 line must end with the bare result flag 10: {code1}"
        );
    }

    #[test]
    fn soul_gem_resurrection_emits_a_code_22_line() {
        // A player (unit 1) resurrects another player (unit 2). The accept carries
        // abilityId 0 (mapped to 26770). The line is the combat prefix with own-side
        // masks (16|16, both allies), NO `C` cast field, and NO trailing tail.
        let st = "18400/18400,9971/12868,8488/28700,198/500,0/1000,0,0.5,0.7,5.9";
        // src=1 resurrects tgt=2, abilityId 0, hit 0.
        let rez = format!(
            "2,COMBAT_EVENT,SOUL_GEM_RESURRECTION_ACCEPTED,GENERIC,0,0,0,0,0,1,{st},2,{st}"
        );
        let lines = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            "0,UNIT_ADDED,2,PLAYER,F,2,0,F,1,3,\"Ally\",\"@ally\",222,50,1740,0,PLAYER_ALLY,T",
            "1,BEGIN_COMBAT,",
            &rez,
        ];
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        let c22 = out
            .events_string
            .lines()
            .find(|l| l.split('|').nth(1) == Some("22"))
            .expect("a soul-gem resurrection must emit a code-22 line");
        let f: Vec<&str> = c22.split('|').collect();
        assert_eq!(f[1], "22");
        assert_eq!((f[3], f[4]), ("16", "16"), "own-side ally masks: {c22}");
        // No `C{cast}` field (cast_id 0) and no trailing crit/final tail: after the
        // masks come straight to S/T blocks, and the line ends with the T block.
        assert!(
            !c22.contains("|C"),
            "resurrect carries no cast field: {c22}"
        );
        assert!(
            c22.contains("|S") && c22.contains("|T"),
            "S+T blocks: {c22}"
        );
    }

    #[test]
    fn immune_code_1_uses_the_bare_result_flag_end_to_end() {
        // Regression for the headline fix: IMMUNE emits a single trailing `|10`, not
        // `|1|10`. Exercised end-to-end through emit_combat_event (not just the tail
        // helper), so the integration with masks/state can't silently regress it.
        let st = "18400/18400,9971/12868,8488/28700,198/500,0/1000,0,0.5,0.7,5.9";
        // IMMUNE with a non-zero raw hit (162) — the official zeroes it → bare `|10`.
        let immune = format!("2,COMBAT_EVENT,IMMUNE,FIRE,1,162,0,5000,100,1,{st},30,{st}");
        let lines = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            "0,UNIT_ADDED,30,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F",
            "1,BEGIN_COMBAT,",
            &immune,
        ];
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        let c1 = out
            .events_string
            .lines()
            .find(|l| l.split('|').nth(1) == Some("1"))
            .expect("IMMUNE emits a code-1 line");
        assert!(
            c1.ends_with("|10") && !c1.ends_with("|1|10"),
            "IMMUNE must end with the bare result flag 10 (raw hit ignored): {c1}"
        );
    }

    #[test]
    fn end_cast_interrupted_emits_code_27() {
        // A player (unit 1) casts an interrupting ability (61665) at a monster (30)
        // who is mid-cast (cast 5000, ability 88330). The interrupting cast registers
        // its tuple via its BEGIN_CAST; the END_CAST INTERRUPTED then emits code-27.
        let st = "18400/18400,9971/12868,8488/28700,198/500,0/1000,0,0.5,0.7,5.9";
        let prelude = [
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            "0,UNIT_ADDED,30,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F",
            "1,BEGIN_COMBAT,",
            // The monster begins the cast that will be interrupted (cast id 5000).
            &format!("2,BEGIN_CAST,1000,F,5000,88330,30,{st},*"),
            // The player begins the interrupting cast (ability 61665) targeting the
            // monster — this registers the (player, monster, 61665) tuple.
            &format!("3,BEGIN_CAST,0,F,5001,61665,1,{st},30,{st}"),
        ];
        // END_CAST INTERRUPTED of cast 5000: interruptedAbility 88330, interrupting
        // ability 61665, interrupting unit 1.
        let interrupted = "4,END_CAST,INTERRUPTED,5000,88330,61665,1";
        let mut lines: Vec<&str> = prelude.to_vec();
        lines.push(interrupted);
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        let c27 = out
            .events_string
            .lines()
            .find(|l| l.split('|').nth(1) == Some("27"));
        let c27 = c27.expect("END_CAST INTERRUPTED must emit a code-27 line");
        let f: Vec<&str> = c27.split('|').collect();
        // {ts}|27|{sub}|{srcMask}|{tgtMask}|{interruptedAbilityIndex}
        assert_eq!(f[1], "27");
        // Player source is friendly (16), monster caster is hostile (64).
        assert_eq!((f[3], f[4]), ("16", "64"), "own-side masks: 16|64 ({c27})");
        // A self-interrupt (same interrupting + interrupted ability) is dropped.
        let mut lines2: Vec<&str> = prelude.to_vec();
        let self_int = "4,END_CAST,INTERRUPTED,5000,61665,61665,1";
        lines2.push(self_int);
        let mut e2 = EventEmitter::new();
        let out2 = e2.build(&lines2);
        assert!(
            !out2
                .events_string
                .lines()
                .any(|l| l.split('|').nth(1) == Some("27")),
            "an ability interrupting itself must not emit a code-27 line"
        );
    }

    #[test]
    fn zero_zero_in_combat_heal_is_dropped() {
        let st = "18400/18400,9971/12868,8488/28700,198/500,0/1000,0,0.5,0.7,5.9";
        let prelude = [
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            "1,BEGIN_COMBAT,",
        ];
        // A heal that healed nothing (hit 0, overflow 0) in combat → dropped.
        let zero = format!("2,COMBAT_EVENT,HEAL,GENERIC,8,0,0,5000,1000,1,{st},*");
        let mut lines: Vec<&str> = prelude.to_vec();
        lines.push(&zero);
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        assert!(
            !out.events_string
                .lines()
                .any(|l| l.split('|').nth(1) == Some("3")),
            "a zero-zero in-combat heal must not emit a code-3 line: {:?}",
            out.events_string
        );
        // A heal with a real value still emits.
        let real = format!("3,COMBAT_EVENT,HEAL,GENERIC,8,500,0,5000,1000,1,{st},*");
        let mut lines2: Vec<&str> = prelude.to_vec();
        lines2.push(&real);
        let mut e2 = EventEmitter::new();
        let out2 = e2.build(&lines2);
        assert!(
            out2.events_string
                .lines()
                .any(|l| l.split('|').nth(1) == Some("3")),
            "a real heal must still emit a code-3 line"
        );
    }

    #[test]
    fn validate_segment_text_accepts_a_well_formed_segment() {
        // header | count | two well-formed body lines (an A-bearing code-1 and a
        // pure marker code-52). max_a = 2 → A=2 is in range.
        let seg = "15|1\n2\n100|1|2|16|64|C5|S1|T1|1|50\n200|52|";
        assert!(validate_segment_text(seg, 2).is_ok());
    }

    #[test]
    fn validate_segment_text_rejects_a_dangling_tuple_ref() {
        // A code-1 line with A=5 but only 2 tuples allocated → out-of-range ref, the
        // exact "accepts but never renders" failure. Must be rejected so the caller
        // falls back to the official uploader.
        let seg = "15|1\n1\n100|1|5|16|64|C5|S1|T1|1|50";
        let err = validate_segment_text(seg, 2).unwrap_err();
        assert!(err.contains("out of range"), "got: {err}");
    }

    #[test]
    fn validate_segment_text_rejects_a_wrong_event_count() {
        // Declared 3 but only 1 body line.
        let seg = "15|1\n3\n100|52|";
        let err = validate_segment_text(seg, 1).unwrap_err();
        assert!(err.contains("event count"), "got: {err}");
    }

    #[test]
    fn validate_segment_text_rejects_a_non_numeric_timestamp() {
        let seg = "15|1\n1\nXX|52|";
        let err = validate_segment_text(seg, 1).unwrap_err();
        assert!(err.contains("timestamp"), "got: {err}");
    }

    #[test]
    fn full_combat_payload_passes_the_structural_self_check() {
        // The real combat capture (when present) must pass the runtime self-check —
        // proving the gate the production path runs never rejects a good segment.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../.decode-samples/combat_raw_encounter.log"
        );
        let Ok(raw) = std::fs::read_to_string(path) else {
            return; // golden pair not present — nothing to check.
        };
        let lines: Vec<&str> = raw.lines().collect();
        let (id2a, ab2i) = super::super::encode::actor_ability_maps(&lines);
        let mut e = EventEmitter::with_master_indices(id2a, ab2i);
        let seg = render_segment(&lines, &mut e).expect("segment builds");
        validate_segment_text(&seg, e.allocated())
            .expect("the real combat segment must pass the runtime self-check");
    }

    // The end-to-end seam: raw lines → ready-to-upload ZIP'd segment + master
    // table. The segment bytes must unzip back to the exact fights-segment text,
    // confirming the full assemble → frame → ZIP pipeline.
    #[test]
    fn native_payload_zips_segment_and_master() {
        let raw = include_str!("testdata/sample_raw_encounter.log");
        let lines: Vec<&str> = raw.lines().collect();
        let (segment, master) = build_native_payload(&lines)
            .expect("build payload")
            .expect("valid session");
        assert!(!segment.bytes.is_empty(), "segment must have ZIP bytes");
        assert!(!master.bytes.is_empty(), "master must have ZIP bytes");

        // The segment ZIP must unzip to the same text build_fights_segment renders.
        let expected = build_fights_segment(&lines).unwrap();
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(segment.bytes)).expect("open segment zip");
        let mut file = archive.by_index(0).unwrap();
        assert_eq!(file.name(), "log.txt");
        use std::io::Read;
        let mut got = String::new();
        file.read_to_string(&mut got).unwrap();
        assert_eq!(
            got, expected,
            "unzipped segment must match the rendered text"
        );
    }

    #[test]
    fn native_payload_needs_a_valid_session() {
        let lines = vec!["4,ZONE_CHANGED,1129,\"Hall\",NONE"];
        assert!(build_native_payload(&lines).unwrap().is_none());
    }

    // Regression: a UNIT_CHANGED updates championPoints from the CORRECT field
    // (tail index 7 → absolute 9), not the reaction field (absolute 11). The
    // emitted state block must carry the new numeric CP, never a reaction token.
    #[test]
    fn unit_changed_updates_champion_points_not_reaction() {
        // UNIT_CHANGED tail layout: unitId,class,race,name,account,charId,level,CP,
        // owner,reaction,grouped → CP at tail index 7 (here 1741), reaction at 9.
        let lines = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA Megaserver\",\"en\",\"eso.live.11.3\"",
            // Player unit 1, initial CP 1740.
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            // A hostile target so a damage event has both sides.
            "0,UNIT_ADDED,30,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F",
            // CP increments to 1741 (reaction stays PLAYER_ALLY).
            "5,UNIT_CHANGED,1,3,1,\"Hero\",\"@hero\",111,50,1741,0,PLAYER_ALLY,T",
            // A damage event from the player carries the player's CURRENT CP (1741)
            // in its source state block.
            "6,COMBAT_EVENT,DAMAGE,FIRE,1,500,0,5000,100,1,18400/18400,9971/12868,8488/28700,198/500,0/1000,0,0.5760,0.7050,5.9698,30,40000/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0",
        ];
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        // Find the code-1 line; its source state block's 7th field must be 1741.
        let code1 = out
            .events_string
            .lines()
            .find(|l| l.split('|').nth(1) == Some("1"))
            .expect("a code-1 line");
        // Locate the S block and read its championPoints (7th block field).
        let fields: Vec<&str> = code1.split('|').collect();
        let s_idx = fields.iter().position(|f| f.starts_with('S')).unwrap();
        let s_cp = fields[s_idx + 6];
        assert_eq!(
            s_cp, "1741",
            "source state block must carry the updated numeric CP, got {s_cp:?} in {code1}"
        );
        // And definitely not a reaction token.
        assert!(
            !code1.contains("PLAYER_ALLY") && !code1.contains("HOSTILE"),
            "a reaction token must never leak into a state block: {code1}"
        );
    }

    /// Strip the optional trailing `|A{n}` reference (the unsolved byte-exact
    /// cast-ref) from a code-5/10 effect line so the rest can be compared.
    fn strip_a_ref(line: &str) -> &str {
        // Only effect GAINED lines (codes 5/10) ever carry the trailing |A{n}.
        match line.rsplit_once('|') {
            Some((head, tail))
                if tail.starts_with('A') && tail[1..].chars().all(|c| c.is_ascii_digit()) =>
            {
                head
            }
            _ => line,
        }
    }

    // The MILESTONE structural test: assemble the events for the matched golden
    // sample and assert we reproduce the captured fights-segment event lines —
    // EXCEPT the optional `|A{n}` cast-reference on GAINED effect lines, which is
    // the unsolved byte-exact global counter and is deliberately omitted (a
    // structurally-valid segment does not need it). Everything else — codes,
    // timestamps, masks, the C cast field, every state block, the dense
    // first-sight subordinal — must match byte-for-byte.
    #[test]
    fn sample_events_reproduce_golden_modulo_optional_a_ref() {
        let raw = include_str!("testdata/sample_raw_encounter.log");
        let golden = include_str!("testdata/sample_fights_segment.txt");
        let lines: Vec<&str> = raw.lines().collect();
        // The emitter needs the master index maps so its `A` is the tuple index.
        let (id2a, ab2i) = super::super::encode::actor_ability_maps(&lines);
        let mut e = EventEmitter::with_master_indices(id2a, ab2i);
        let out = e.build(&lines);

        // Golden events are the file minus the 2-line header (version|game, count).
        let golden_events: Vec<&str> = golden.lines().skip(2).collect();
        let ours: Vec<&str> = out.events_string.lines().collect();

        assert_eq!(
            ours.len(),
            golden_events.len(),
            "event count must match the golden segment ({} vs {})",
            ours.len(),
            golden_events.len()
        );
        assert_eq!(
            out.event_count as usize,
            golden_events.len(),
            "reported event_count must equal the number of emitted lines"
        );
        for (i, (got, want)) in ours.iter().zip(golden_events.iter()).enumerate() {
            // Strip the optional cast-ref from BOTH sides: whether a GAINED carries
            // `|A{n}` depends on having seen its causing BEGIN_CAST inside this short
            // sample window, which is incidental to structural correctness.
            assert_eq!(
                strip_a_ref(got),
                strip_a_ref(want),
                "event line {i} must reproduce the golden line (modulo optional A-ref)"
            );
        }
    }

    // Structural-validity invariants every emitted line must satisfy, independent
    // of byte-identity: a numeric timestamp, a known code, and a dense subordinal
    // whose leading A is a real allocated counter value (1..=allocated).
    #[test]
    fn every_emitted_line_is_structurally_valid() {
        let raw = include_str!("testdata/sample_raw_encounter.log");
        let lines: Vec<&str> = raw.lines().collect();
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        let max_a = e.allocated();

        // Codes that lead with a subordinal A.B.C (effect/cast/combat/regen); the
        // zone/map codes 41/51 put a literal id in that slot instead.
        let a_bearing = [
            "1", "2", "3", "4", "5", "6", "7", "8", "10", "11", "12", "15", "16", "26",
        ];
        for line in out.events_string.lines() {
            let f: Vec<&str> = line.split('|').collect();
            assert!(f.len() >= 3, "line too short: {line}");
            assert!(
                f[0].parse::<u64>().is_ok(),
                "timestamp must be numeric: {line}"
            );
            let code = f[1];
            if a_bearing.contains(&code) {
                // Leading subordinal component is the A; must be a real counter.
                let a: u32 = f[2]
                    .split('.')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap_or_else(|_| panic!("subordinal A not numeric: {line}"));
                assert!(
                    a >= 1 && a <= max_a,
                    "subordinal A={a} out of allocated range 1..={max_a}: {line}"
                );
            }
        }
    }

    // ── LIVE STATE-CONTINUATION API tests (spike/native-live) ───────────────
    // These exercise the additive EventEmitter live API + the H1 (actor-index
    // stability) fix. They are REAL unit tests (not #[ignore]d): the live API is
    // committed code and its correctness gates the spike, so it must stay green in
    // CI even though the driver that uses it is debug-only.

    // open_segment + live_segment_time_bounds: a fresh segment's wall window is
    // `current_session_wall + raw_ts` of its first/last EMITTED event — the proven
    // one-shot begin_wall+rel formula made per-segment. Before any BEGIN_LOG (or
    // before any emit) it returns None so the driver skips the POST (never a 1970
    // epoch placement).
    #[test]
    fn live_segment_time_bounds_uses_session_wall_plus_raw_ts() {
        let mut e = EventEmitter::new();
        // No BEGIN_LOG yet, no emits → no window.
        assert_eq!(e.live_segment_time_bounds(), None);

        let lines = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1740,0,PLAYER_ALLY,T",
            "10,ZONE_CHANGED,1129,\"Hall\",NORMAL", // first EMITTED event, raw ts 10
            "510,MAP_CHANGED,1576,\"Rimmen\",\"a/b\"", // last EMITTED event, raw ts 510
        ];
        e.open_segment();
        for l in &lines {
            let _ = e.feed(l);
        }
        // Window = session wall (BEGIN_LOG field 2) + first/last emitted RAW ts.
        assert_eq!(
            e.live_segment_time_bounds(),
            Some((1700000000000 + 10, 1700000000000 + 510)),
            "wall window must be current_session_wall + raw_ts of first/last emit"
        );

        // A new segment re-anchors the window to its OWN first/last emit, while the
        // report-scoped offset/anchor (and thus body ts) stay continuous.
        e.open_segment();
        assert_eq!(
            e.live_segment_time_bounds(),
            None,
            "a freshly opened segment that hasn't emitted has no window yet"
        );
        let _ = e.feed("900,ZONE_CHANGED,1130,\"Other\",NORMAL");
        assert_eq!(
            e.live_segment_time_bounds(),
            Some((1700000000000 + 900, 1700000000000 + 900)),
            "second segment's window anchors on its own first emit"
        );
    }

    // The body timestamps stay REPORT-ABSOLUTE across an open_segment cut — the
    // offset/anchor is report-scoped, so segment 2's events keep counting from the
    // session's first emitted event, never re-zeroing. (The wall window resets; the
    // body ts does not.)
    #[test]
    fn body_ts_is_report_absolute_across_a_segment_cut() {
        let mut e = EventEmitter::new();
        let s1 = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "10,ZONE_CHANGED,1129,\"Hall\",NORMAL", // anchors segTs 0 at raw 10
        ];
        e.open_segment();
        let out1 = e.build(&s1);
        assert!(
            out1.events_string.starts_with("0|41|"),
            "first emit at segTs 0"
        );

        // Cut, then feed a later event in the SAME session: its body ts must be
        // raw−anchor (1010−10 = 1000), NOT re-zeroed to 0.
        e.open_segment();
        let out2 = e.build(&["1010,MAP_CHANGED,1576,\"Rimmen\",\"a/b\""]);
        assert!(
            out2.events_string.starts_with("1000|51|"),
            "segment 2 body ts must be report-absolute (1000), got: {}",
            out2.events_string.lines().next().unwrap_or("")
        );
    }

    // HAZARD H1 FIX: a monster ADDED in segment 1 but first REGISTERING in segment 2
    // keeps its frozen actor index across a cumulative rebuild when the prior frozen
    // identities are forced. Without the fix (plain actor_ability_maps) the index
    // shifts; with actor_ability_maps_forced it is stable. This is the make-or-break
    // correctness property for the live cumulative-master model.
    #[test]
    fn forced_identities_keep_actor_indices_stable_across_a_cut() {
        use super::super::encode::{actor_ability_maps, actor_ability_maps_forced};

        let player =
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,3,9,\"Hero\",\"@hero\",111,50,1735,0,PLAYER_ALLY,T";
        // Wisp A added FIRST, registers only in s2. Wisp B added second, registers in s1.
        let mon_a = "0,UNIT_ADDED,40,MONSTER,F,0,90001,F,0,0,\"Wisp A\",\"\",0,50,160,0,HOSTILE,F";
        let mon_b = "0,UNIT_ADDED,41,MONSTER,F,0,90002,F,0,0,\"Wisp B\",\"\",0,50,160,0,HOSTILE,F";
        let state = "16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0";
        let tgt = "40000/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0";
        let dmg_b = format!("500,COMBAT_EVENT,DAMAGE,FIRE,1,500,0,5000,28549,1,{state},41,{tgt}");
        let dmg_a = format!("600,COMBAT_EVENT,DAMAGE,FIRE,1,700,0,5000,28549,1,{state},40,{tgt}");

        let s1: Vec<&str> = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,ZONE_CHANGED,1129,\"Hall\",NONE",
            player,
            mon_a,
            mon_b,
            "10,BEGIN_COMBAT",
            "10,ABILITY_INFO,28549,\"Roll\",\"/esoui/art/icons/x.dds\",F,T",
            &dmg_b,
            "1500,END_COMBAT",
        ];
        let mut full = s1.clone();
        full.extend_from_slice(&["600,BEGIN_COMBAT", &dmg_a, "1600,END_COMBAT"]);

        let b_id = "m:90002:Wisp B";
        let (id_s1, ab_s1) = actor_ability_maps(&s1);
        let idx_b_s1 = id_s1.get(b_id).copied();

        // WITHOUT the fix: a plain cumulative rebuild renumbers Wisp B (Wisp A now
        // registers and takes its earlier UNIT_ADDED slot).
        let (id_full_plain, _) = actor_ability_maps(&full);
        assert_ne!(
            idx_b_s1,
            id_full_plain.get(b_id).copied(),
            "plain rebuild SHOULD renumber Wisp B — this is the H1 hazard we're fixing"
        );

        // WITH the fix: pin the s1-frozen assignments → Wisp B keeps its index and
        // the late-registering Wisp A appends ABOVE the prior max.
        let (id_full_pinned, _) = actor_ability_maps_forced(&full, Some((&id_s1, &ab_s1)));
        assert_eq!(
            idx_b_s1,
            id_full_pinned.get(b_id).copied(),
            "pinned rebuild must KEEP Wisp B at its frozen index (H1 fix)"
        );
        // Every prior s1 actor is stable.
        for (identity, &idx) in &id_s1 {
            assert_eq!(
                Some(idx),
                id_full_pinned.get(identity).copied(),
                "pinned rebuild must preserve every frozen actor index: {identity}"
            );
        }
        // The newly-registering Wisp A appends above the prior max (no collision).
        let prior_max = id_s1.values().copied().max().unwrap();
        assert_eq!(
            id_full_pinned.get("m:90001:Wisp A").copied(),
            Some(prior_max + 1),
            "a late-registering actor must append above the frozen max, not insert mid-table"
        );

        // And the master builder must render actor records in PINNED-INDEX order so
        // record N == the actor index N references. Build a pinned master and assert
        // the actor section lists Wisp B (idx 2) before Wisp A (idx 3).
        let tuples = vec![(1u32, idx_b_s1.unwrap(), 1u32)];
        let master = super::super::encode::build_master_table_with_tuples_forced(
            &full, &tuples, &id_s1, &ab_s1,
        )
        .expect("pinned master builds");
        let wisp_b_pos = master.find("Wisp B");
        let wisp_a_pos = master.find("Wisp A");
        assert!(
            matches!((wisp_b_pos, wisp_a_pos), (Some(b), Some(a)) if b < a),
            "pinned master must list Wisp B (idx 2) before Wisp A (idx 3): B@{wisp_b_pos:?} A@{wisp_a_pos:?}"
        );
    }

    // HAZARD H1, ABILITY AXIS: the synthetic HEALTH_RECOVERY splice (emitted at the
    // first HEALTH_REGEN) can shift ability indices across a cumulative rebuild. An
    // ability whose ABILITY_INFO is seen in s1 (before any regen) gets an early index;
    // when s2 adds a HEALTH_REGEN, the unpinned rebuild splices the synthetic and
    // shifts every later ability. Pinning the prior ability map must keep s1 abilities
    // at their frozen indices and append the synthetic above the max.
    #[test]
    fn forced_ability_indices_are_stable_across_a_cut() {
        use super::super::encode::{actor_ability_maps, actor_ability_maps_forced};

        let player =
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,3,9,\"Hero\",\"@hero\",111,50,1735,0,PLAYER_ALLY,T";
        // s1: two abilities declared, NO health regen yet (so no synthetic spliced).
        let s1: Vec<&str> = vec![
            "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live.11.3\"",
            "0,ZONE_CHANGED,1129,\"Hall\",NONE",
            player,
            "10,ABILITY_INFO,1001,\"Alpha\",\"/esoui/art/icons/a.dds\",F,T",
            "10,ABILITY_INFO,1002,\"Beta\",\"/esoui/art/icons/b.dds\",F,T",
        ];
        // full: s1 + a HEALTH_REGEN (splices the synthetic) + a new ability.
        let mut full = s1.clone();
        full.extend_from_slice(&[
            "100,HEALTH_REGEN,500,1,16000/16000,12000/12000,8000/12000,53/500,0/1000,0,0.5,0.5,4.0",
            "110,ABILITY_INFO,1003,\"Gamma\",\"/esoui/art/icons/c.dds\",F,T",
        ]);

        let (_, ab_s1) = actor_ability_maps(&s1);
        let alpha_s1 = ab_s1.get("1001").copied();
        let beta_s1 = ab_s1.get("1002").copied();

        // Unpinned cumulative rebuild SHIFTS Beta: the synthetic (61322) splices at the
        // regen position (after Alpha/Beta in line order here it actually appends, so
        // construct the shift by checking the synthetic took an index that pushes the
        // ability count). The decisive check is the PINNED path preserving s1 indices.
        let (_, ab_full_pinned) = actor_ability_maps_forced(&full, Some((&id_dummy(), &ab_s1)));
        assert_eq!(
            ab_s1.get("1001").copied(),
            ab_full_pinned.get("1001").copied(),
            "pinned rebuild must preserve Alpha's index"
        );
        assert_eq!(
            beta_s1,
            ab_full_pinned.get("1002").copied(),
            "pinned rebuild must preserve Beta's index across the synthetic splice"
        );
        // The synthetic HEALTH_RECOVERY (61322) and the new Gamma both append above the
        // prior max — never colliding with a frozen index.
        let prior_max = ab_s1.values().copied().max().unwrap();
        let synthetic = ab_full_pinned.get("61322").copied().unwrap();
        let gamma = ab_full_pinned.get("1003").copied().unwrap();
        assert!(
            synthetic > prior_max && gamma > prior_max,
            "late abilities/synthetic must append above the frozen max ({prior_max}): \
             synthetic={synthetic}, gamma={gamma}"
        );
        assert_ne!(synthetic, gamma, "appended indices must be distinct");
        let _ = alpha_s1;
    }

    /// An empty actor map for ability-only pinning tests (the ability path ignores it).
    fn id_dummy() -> std::collections::HashMap<String, u32> {
        std::collections::HashMap::new()
    }

    // ── DIFFERENTIAL GATE: one-emitter-with-cuts == one-shot build() ─────────
    // The core offline correctness gate for the live spike: assembling a log with a
    // long-lived emitter that is CUT at fight boundaries (open_segment per END_COMBAT)
    // must produce the SAME event lines, in the same order, as the proven one-shot
    // build() over the whole log. If they diverge, state continuation across a cut is
    // corrupting something. This is what lets us trust the live driver offline; only
    // the server's open-report rendering behaviour then remains (a live round-trip).
    //
    // Simulates the live driver's cut loop minus the network: one emitter, feed every
    // line, open_segment() at each END_COMBAT boundary, collect emitted lines. The
    // master index maps are supplied up front (as the live driver does on first
    // create) and never change here because no actor registers late in this fixture —
    // the H1 pin path is exercised separately by
    // forced_identities_keep_actor_indices_stable_across_a_cut.
    fn assemble_with_fight_cuts(lines: &[&str]) -> Vec<String> {
        let (id2a, ab2i) = super::super::encode::actor_ability_maps(lines);
        let mut e = EventEmitter::with_master_indices(id2a, ab2i);
        let mut all_emitted: Vec<String> = Vec::new();
        e.open_segment();
        for line in lines {
            if let Some(ev) = e.feed(line) {
                for l in ev.split('\n') {
                    all_emitted.push(l.to_string());
                }
            }
            // Cut at a fight boundary: the live driver POSTs the segment here and
            // opens the next. open_segment() only resets the per-segment WALL window;
            // the report-scoped encoder state (tuples, offset, correlations) persists.
            let kind = split_csv_quoted_pub(line);
            if kind.get(1).map(|s| s.trim()) == Some("END_COMBAT") {
                e.open_segment();
            }
        }
        // Drain any pending code-38 shields at end of stream (the one-shot build()
        // does this too).
        for l in e.flush_pending_shields(None, None) {
            all_emitted.push(l);
        }
        all_emitted
    }

    #[test]
    fn one_emitter_with_fight_cuts_matches_one_shot_build_byte_for_byte() {
        let raw = include_str!("testdata/live_correlation_synthetic.log");
        let lines: Vec<&str> = raw.lines().collect();

        // One-shot: the proven path, all lines through one build().
        let (id2a, ab2i) = super::super::encode::actor_ability_maps(&lines);
        let mut one_shot = EventEmitter::with_master_indices(id2a, ab2i);
        let out = one_shot.build(&lines);
        let one_shot_lines: Vec<&str> = out.events_string.lines().collect();

        // Live: same lines through one emitter cut at every END_COMBAT.
        let live_lines = assemble_with_fight_cuts(&lines);

        assert_eq!(
            live_lines.len(),
            one_shot_lines.len(),
            "live-with-cuts emitted {} lines vs one-shot {}",
            live_lines.len(),
            one_shot_lines.len()
        );
        for (i, (live, shot)) in live_lines.iter().zip(one_shot_lines.iter()).enumerate() {
            assert_eq!(
                live, shot,
                "event line {i} differs across a cut:\n  live: {live}\n  shot: {shot}"
            );
        }
        // And the tuple table built by the cut path must equal the one-shot table
        // (same A numbering → the master resolves the same way).
        let (id2a, ab2i) = super::super::encode::actor_ability_maps(&lines);
        let mut cut_emitter = EventEmitter::with_master_indices(id2a, ab2i);
        cut_emitter.open_segment();
        for line in &lines {
            let _ = cut_emitter.feed(line);
            if split_csv_quoted_pub(line).get(1).map(|s| s.trim()) == Some("END_COMBAT") {
                cut_emitter.open_segment();
            }
        }
        let _ = cut_emitter.flush_pending_shields(None, None);
        assert_eq!(
            cut_emitter.tuples(),
            one_shot.tuples(),
            "the cut path's tuple table must match the one-shot table exactly"
        );
    }

    // ── SPIKE PROBE (debug R&D, not a ship gate) ────────────────────────────
    // Empirically measures whether the existing `EventEmitter` carries its state
    // coherently across a HEADERLESS session/segment boundary — the core question
    // of the native live-streaming spike (`spike/native-live`). It is `#[ignore]`d
    // so it never runs in CI; run with `cargo test -- --ignored --nocapture
    // spike_probe`. NOTE: a two-session log is intentional input here — in
    // production these route to the official uploader (the MultiSession guard),
    // and that guard is NOT changed by this probe.
    #[test]
    #[ignore = "spike R&D diagnostic; run with --ignored --nocapture"]
    fn spike_probe_state_continuation_across_a_session_boundary() {
        let raw = include_str!("testdata/two_session_synthetic.log");
        let all: Vec<&str> = raw.lines().collect();

        // Find the second BEGIN_LOG = the headerless-boundary point a live cut
        // would straddle. Everything before it is "session 1 / segment(s) so far",
        // everything after is the new session with NO fresh actor/ability context
        // of its own beyond what it re-declares.
        let second_begin = all
            .iter()
            .enumerate()
            .filter(|(_, l)| split_csv_quoted_pub(l).get(1).map(|s| s.trim()) == Some("BEGIN_LOG"))
            .nth(1)
            .map(|(i, _)| i)
            .expect("fixture must have two BEGIN_LOG sessions");
        let (s1, s2) = all.split_at(second_begin);

        // MODEL A — ONE long-lived emitter fed BOTH sessions (the "carry state
        // across the boundary" live model). We supply the master maps for the WHOLE
        // log so A is the shared tuple index, mirroring build_native_payload.
        let (id2a, ab2i) = super::super::encode::actor_ability_maps(&all);
        let mut shared = EventEmitter::with_master_indices(id2a.clone(), ab2i.clone());
        let out_s1 = shared.build(s1);
        let tuples_after_s1 = shared.allocated();
        let offset_after_s1 = shared.offset;
        let wall_delta_after_s1 = shared.session_wall_delta;
        let out_s2 = shared.build(s2);
        let tuples_after_s2 = shared.allocated();

        eprintln!("[spike] === MODEL A: one emitter across the boundary ===");
        eprintln!(
            "[spike] s1: {} events, tuples allocated so far = {}",
            out_s1.event_count, tuples_after_s1
        );
        eprintln!(
            "[spike] after s1: offset={offset_after_s1}, session_wall_delta={wall_delta_after_s1}"
        );
        eprintln!(
            "[spike] s2: {} events, tuples allocated so far = {}",
            out_s2.event_count, tuples_after_s2
        );
        eprintln!(
            "[spike] after s2: offset={}, session_wall_delta={}",
            shared.offset, shared.session_wall_delta
        );
        // Print the segment-2 body timestamps: do they continue ON the report
        // timeline (offset by the inter-session wall gap) or RESET to ~0? This is
        // the body-ts-absolute-vs-per-segment question the spike must answer.
        let s2_first_ts: Option<i64> = out_s2
            .events_string
            .lines()
            .next()
            .and_then(|l| l.split('|').next())
            .and_then(|t| t.parse().ok());
        eprintln!("[spike] s2 first emitted body ts = {s2_first_ts:?}");
        eprintln!(
            "[spike] inter-session wall gap (ms) = {}",
            1780641600000i64 - 1780641553946i64
        );

        // OBSERVATION 1: the tuple table grows monotonically across the boundary —
        // session 2's events keep allocating into the SAME table (the player's
        // re-used abilities reuse session-1 tuple indices; the new monster gets a
        // fresh one). This is the structural property a live cumulative-master model
        // relies on.
        assert!(
            tuples_after_s2 >= tuples_after_s1,
            "tuple table must not shrink across the boundary"
        );
        eprintln!(
            "[spike] tuple table is monotonic across the boundary: {tuples_after_s1} -> {tuples_after_s2}"
        );
        // Dump the shared tuple table: the first `tuples_after_s1` records are
        // session 1's; any record at index <= tuples_after_s1 that session 2 ALSO
        // references proves cross-segment A-ref reuse (the cumulative-master model's
        // correctness condition — segment 2 can reference a tuple the server already
        // holds from segment 1).
        eprintln!("[spike] shared tuple table (src,tgt,ability) — index = A:");
        for (i, t) in shared.tuples().iter().enumerate() {
            let origin = if (i as u32) < tuples_after_s1 {
                "s1"
            } else {
                "s2"
            };
            eprintln!(
                "[spike]   A={} {:?}  (first allocated in {origin})",
                i + 1,
                t
            );
        }
        // The reused player ability (Roll Dodge 28549) appears in BOTH sessions; its
        // A must be a single index <= tuples_after_s1 reused by session 2 (not a new
        // duplicate). If session 2 re-allocated it, the table would have grown by 2.
        eprintln!(
            "[spike] session 2 added exactly {} new tuple(s) — a reused ability does NOT re-allocate",
            tuples_after_s2 - tuples_after_s1
        );

        // OBSERVATION 2: session 2's body timestamps are anchored to session 1's
        // first event (REPORT-ABSOLUTE), NOT re-zeroed — they carry the cross-session
        // wall separation. Print whether they exceed the inter-session gap so the
        // synthesis can confirm the body-ts model the live driver must preserve.
        if let Some(ts) = s2_first_ts {
            eprintln!(
                "[spike] s2 body ts is report-absolute (>= inter-session gap?): {}",
                ts >= (1780641600000i64 - 1780641553946i64)
            );
        }

        // MODEL B — the one-shot production path on the SAME two-session log: what
        // does build_native_payload actually produce, and does the one-shot
        // segment_time_bounds collapse to the FIRST session only (the documented
        // reason multi-session is routed away)? Reads the bounds the live model must
        // instead compute PER SEGMENT.
        let (start, end) = segment_time_bounds(&all);
        eprintln!("[spike] === one-shot segment_time_bounds on the full 2-session log ===");
        eprintln!(
            "[spike] (start, end) = ({start}, {end})  span_ms = {}",
            end.saturating_sub(start)
        );
        eprintln!(
            "[spike] NOTE: one-shot uses the FIRST BEGIN_LOG wall ({}) + last event rel — \
             a live driver must instead bound EACH segment from the running wall anchor.",
            1780641553946u64
        );

        // The whole two-session log still builds a structurally-valid one-shot
        // payload (the encoder doesn't crash on multi-session input — it's the
        // segment-time-bounds + body-ts continuity that the live model must own).
        match build_native_payload(&all) {
            Ok(Some((seg, _master))) => eprintln!(
                "[spike] one-shot build_native_payload OK: segment {} ZIP bytes, window=({},{})",
                seg.bytes.len(),
                seg.start_time,
                seg.end_time
            ),
            Ok(None) => eprintln!("[spike] one-shot build_native_payload: None (no valid session)"),
            Err(e) => eprintln!("[spike] one-shot build_native_payload Err: {e}"),
        }

        // OBSERVATION 3 — the actor-index-STABILITY hazard the cumulative-master
        // model depends on. A cumulative rebuild recomputes identity_to_actor from
        // all_lines_so_far each cut. That is index-stable ONLY if a later cut never
        // RENUMBERS an actor that an earlier segment already referenced. The
        // registering_monster_identities filter (encode.rs) is whole-list: a monster
        // ADDED in segment 1 but first REGISTERING (landing a combat event) in
        // segment 2 is EXCLUDED from the segment-1 actor map and INCLUDED in the
        // segment-2 map — shifting every later actor's index by +1. Compare the maps
        // over s1 alone vs the full log to detect any such shift on THIS fixture.
        let (id2a_s1, _) = super::super::encode::actor_ability_maps(s1);
        eprintln!("[spike] === actor-index stability across the cut ===");
        eprintln!(
            "[spike] identity_to_actor over s1 alone: {} actors",
            id2a_s1.len()
        );
        eprintln!(
            "[spike] identity_to_actor over full log: {} actors",
            id2a.len()
        );
        let mut shifted = 0usize;
        for (identity, &idx_full) in &id2a {
            if let Some(&idx_s1) = id2a_s1.get(identity) {
                if idx_s1 != idx_full {
                    shifted += 1;
                    eprintln!(
                        "[spike]   RENUMBERED: {identity:?} was index {idx_s1} in s1, now {idx_full} — \
                         a segment-1 tuple referencing {idx_s1} would now point elsewhere"
                    );
                }
            }
        }
        eprintln!(
            "[spike] actors renumbered between the s1-only and cumulative maps: {shifted} \
             (0 = index-stable on this fixture; >0 = the deferred-registration hazard is REAL)"
        );

        // OBSERVATION 4 — DIRECTLY CONSTRUCT the deferred-registration case to prove
        // the hazard exists (the prior fixture's monsters both register in their own
        // session, so they don't trigger it). Here: session 1 ADDS two monsters
        // (A then B) but only B registers in s1; A's first registering damage lands
        // in s2. Under a cumulative rebuild, A is excluded from the s1 actor map
        // (so B and the player get the low indices) but INCLUDED in the full map,
        // shifting B's index. A segment-1 tuple that referenced B by its old index
        // would, after the s2 master rebuild, resolve to a different actor.
        let player =
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,3,9,\"Hero\",\"@hero\",111,50,1735,0,PLAYER_ALLY,T";
        // Monster A (unit 40, monsterId 90001) is added FIRST but does not register in s1.
        let mon_a = "0,UNIT_ADDED,40,MONSTER,F,0,90001,F,0,0,\"Wisp A\",\"\",0,50,160,0,HOSTILE,F";
        // Monster B (unit 41, monsterId 90002) is added second and DOES register in s1.
        let mon_b = "0,UNIT_ADDED,41,MONSTER,F,0,90002,F,0,0,\"Wisp B\",\"\",0,50,160,0,HOSTILE,F";
        let state = "16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5,0.5,4.0";
        let tgt = "40000/45000,0/0,0/0,0/0,0/0,0,0.4,0.5,0.0";
        // s1: player hits B (B registers); A is only ADDED, never a combat target.
        let dmg_b = format!("500,COMBAT_EVENT,DAMAGE,FIRE,1,500,0,5000,28549,1,{state},41,{tgt}");
        // s2: player hits A (A registers only now, in the second session).
        let dmg_a = format!("600,COMBAT_EVENT,DAMAGE,FIRE,1,700,0,5000,28549,1,{state},40,{tgt}");
        let s1_lines: Vec<&str> = vec![
            "0,BEGIN_LOG,1780641553946,15,\"NA Megaserver\",\"en\",\"eso.live.11.3\"",
            "0,ZONE_CHANGED,1129,\"Hall\",NONE",
            player,
            mon_a,
            mon_b,
            "10,BEGIN_COMBAT",
            "10,ABILITY_INFO,28549,\"Roll Dodge\",\"/esoui/art/icons/ability_rogue_035.dds\",F,T",
            &dmg_b,
            "1500,END_COMBAT",
            "2000,END_LOG",
        ];
        let mut full_lines = s1_lines.clone();
        full_lines.extend_from_slice(&[
            "0,BEGIN_LOG,1780641600000,15,\"NA Megaserver\",\"en\",\"eso.live.11.3\"",
            "0,ZONE_CHANGED,1129,\"Hall\",NONE",
            player,
            mon_a,
            mon_b,
            "10,BEGIN_COMBAT",
            "10,ABILITY_INFO,28549,\"Roll Dodge\",\"/esoui/art/icons/ability_rogue_035.dds\",F,T",
            &dmg_a,
            "1600,END_COMBAT",
            "2100,END_LOG",
        ]);
        let (id_s1, _) = super::super::encode::actor_ability_maps(&s1_lines);
        let (id_full, _) = super::super::encode::actor_ability_maps(&full_lines);
        eprintln!("[spike] === DEFERRED-REGISTRATION constructed case ===");
        let b_id = "m:90002:Wisp B"; // monster identity = m:{raw_id}:{name} (encode.rs:568)
        eprintln!(
            "[spike] 'Wisp B' index in s1-only map = {:?}, in cumulative map = {:?}",
            id_s1.get(b_id),
            id_full.get(b_id)
        );
        eprintln!(
            "[spike] s1-only actors: {}, cumulative actors: {}",
            id_s1.len(),
            id_full.len()
        );
        match (id_s1.get(b_id), id_full.get(b_id)) {
            (Some(a), Some(b)) if a != b => eprintln!(
                "[spike] >>> HAZARD CONFIRMED: 'Wisp B' RENUMBERED {a} -> {b} across the cumulative \
                 rebuild. A live driver MUST freeze actor indices at first sight (append-only, \
                 ignoring the registering filter for already-emitted actors) or a per-segment \
                 master rebuild corrupts earlier segments' A-refs."
            ),
            (Some(a), Some(b)) => eprintln!(
                "[spike] 'Wisp B' index stable ({a}=={b}) — registering filter did not shift it here"
            ),
            other => eprintln!("[spike] (could not resolve Wisp B identity: {other:?})"),
        }
    }
}

#[cfg(test)]
mod combat_fixture {
    use super::*;

    /// Assemble the full real-combat capture (a 10.7MB raid log with the complete
    /// event vocabulary) and assert the output is **structurally valid end to
    /// end**: every line is well-formed, every code is one we model, and every
    /// subordinal `A` is a real allocated counter value. This exercises the
    /// damage/heal/dot/power/player-info/regen paths that the small golden sample
    /// does not contain.
    ///
    /// The fixture is a decode-only file (too large to commit), so the test is a
    /// no-op on a clean checkout — present locally it is a strong end-to-end gate.
    #[test]
    fn combat_capture_assembles_structurally_valid() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../.decode-samples/combat_raw_encounter.log"
        );
        let Ok(raw) = std::fs::read_to_string(path) else {
            return; // not present in a clean checkout — nothing to assemble.
        };
        let lines: Vec<&str> = raw.lines().collect();
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        let max_a = e.allocated();

        // Sanity envelope: a real raid log emits tens of thousands of events.
        assert!(
            out.event_count > 10_000,
            "expected a large event stream from the combat capture, got {}",
            out.event_count
        );
        assert_eq!(
            out.events_string.lines().count() as u64,
            out.event_count,
            "event_count must equal the number of emitted lines"
        );

        let a_bearing = [
            "1", "2", "3", "4", "5", "6", "7", "8", "10", "11", "12", "15", "16", "26",
        ];
        let mut codes_seen = std::collections::BTreeSet::new();
        for line in out.events_string.lines() {
            let f: Vec<&str> = line.split('|').collect();
            assert!(f.len() >= 2, "line too short: {line}");
            assert!(f[0].parse::<u64>().is_ok(), "ts not numeric: {line}");
            let code = f[1];
            codes_seen.insert(code.to_string());
            if a_bearing.contains(&code) {
                let a: u32 = f[2].split('.').next().unwrap().parse().unwrap_or(0);
                assert!(
                    a >= 1 && a <= max_a,
                    "subordinal A={a} out of range 1..={max_a}: {line}"
                );
            }
        }
        // The full vocabulary should appear: damage, effects, casts, power, regen,
        // player info, zone/map, and combat boundaries.
        for c in [
            "1", "2", "3", "26", "4", "5", "7", "10", "12", "15", "16", "44", "41", "51", "52",
            "53",
        ] {
            assert!(
                codes_seen.contains(c),
                "expected code {c} in a full raid log's output"
            );
        }
    }

    /// DIAGNOSTIC (manual, `--ignored`): diff OUR fights-segment against the
    /// OFFICIAL captured segment for the same raw log, and print the structural
    /// deltas — total event count, per-code counts, and the first diverging line.
    /// This is how we find WHY a server-accepted segment fails to render: the
    /// stream is well-formed but does not match what the parser expects. Read the
    /// printed report with `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore = "diagnostic; needs .decode-samples golden pair, run with --ignored --nocapture"]
    fn diff_against_official_combat_segment() {
        let base = env!("CARGO_MANIFEST_DIR");
        let raw_path = format!("{base}/../.decode-samples/combat_raw_encounter.log");
        let off_path = format!("{base}/../.decode-samples/combat_fights_segment.txt");
        let (Ok(raw), Ok(official)) = (
            std::fs::read_to_string(&raw_path),
            std::fs::read_to_string(&off_path),
        ) else {
            eprintln!("[diff] golden pair not present; skipping");
            return;
        };
        let lines: Vec<&str> = raw.lines().collect();
        let ours = build_fights_segment(&lines).expect("our segment builds");

        // Header + total event count (line 2 of each).
        let our_count: u64 = ours
            .lines()
            .nth(1)
            .and_then(|l| l.parse().ok())
            .unwrap_or(0);
        let off_count: u64 = official
            .lines()
            .nth(1)
            .and_then(|l| l.parse().ok())
            .unwrap_or(0);
        eprintln!("[diff] our header: {:?}", ours.lines().next());
        eprintln!("[diff] off header: {:?}", official.lines().next());
        eprintln!(
            "[diff] TOTAL EVENTS  ours={our_count}  official={off_count}  delta={}",
            our_count as i64 - off_count as i64
        );
        eprintln!(
            "[diff] LINE COUNT    ours={}  official={}",
            ours.lines().count(),
            official.lines().count()
        );

        // Per-code counts (field 1 after the 2-line header), to see which event
        // types we over/under-produce.
        fn code_counts(seg: &str) -> std::collections::BTreeMap<String, u64> {
            let mut m = std::collections::BTreeMap::new();
            for l in seg.lines().skip(2) {
                if let Some(code) = l.split('|').nth(1) {
                    *m.entry(code.to_string()).or_insert(0) += 1;
                }
            }
            m
        }
        let oc = code_counts(&ours);
        let fc = code_counts(&official);
        let mut codes: std::collections::BTreeSet<String> = oc.keys().cloned().collect();
        codes.extend(fc.keys().cloned());
        eprintln!("[diff] per-code (code: ours / official):");
        for c in &codes {
            let o = oc.get(c).copied().unwrap_or(0);
            let f = fc.get(c).copied().unwrap_or(0);
            let mark = if o == f { "" } else { "  <-- DIFF" };
            eprintln!("[diff]   {c:>4}: {o:>7} / {f:>7}{mark}");
        }

        // First diverging line (after the 2-line headers, body only — header
        // differences in the count are expected and reported above).
        let our_body: String = ours.lines().skip(2).collect::<Vec<_>>().join("\n");
        let off_body: String = official.lines().skip(2).collect::<Vec<_>>().join("\n");
        match super::super::differential::diff_segments(&our_body, &off_body) {
            super::super::differential::Diff::Identical => {
                eprintln!("[diff] BODY IDENTICAL")
            }
            super::super::differential::Diff::Diverged {
                line,
                ours,
                official,
            } => {
                eprintln!("[diff] FIRST BODY DIVERGENCE at body line {line}:");
                eprintln!("[diff]   ours: {ours}");
                eprintln!("[diff]   offl: {official}");
            }
            super::super::differential::Diff::LengthMismatch { ours, official } => {
                eprintln!("[diff] BODY LENGTH MISMATCH ours={ours} official={official}")
            }
        }
    }

    /// Regression guard for the per-code accuracy reached against the official
    /// combat segment. Locks in the codes we reproduce EXACTLY and bounds the
    /// residual codes so a change can't silently regress them. Runs only when the
    /// (gitignored) golden pair is present locally; a no-op on a clean checkout.
    ///
    /// CROSS-CAPTURE VALIDATION (2nd capture, an Ossein Cage veteran-trial slice):
    /// on that independent encounter, codes 2/5/6/7/8/10/11/12/15/16/26/27/38/41/44/
    /// 51/52/53 are ALL byte-count-EXACT. That is decisive for two large Maarselok
    /// residuals: **code 5 (+260) and code 16 (−492) are capture-specific artifacts
    /// of the Maarselok log, NOT encoder bugs** — the buff/effect family and the cast
    /// path reproduce exactly on a clean capture, so do not "fix" them toward Maarselok
    /// (that would break Ossein). The genuinely-still-open codes (9/14/28 and the
    /// code-1/19 parser-internal predicates) are small on BOTH captures.
    ///
    /// The remaining non-exact Maarselok codes are documented residuals:
    /// * 5 (+260) / 16 (−492): Maarselok-capture artifacts (EXACT on Ossein) — leave.
    /// * 1 (−139): zero-hit damage the official keeps — emit-vs-drop predicate lives
    ///   in the parser crate's is_damage_event, underdetermined on both captures.
    /// * 9/14/28 (~50 events): rare codes the reference does not model (no construct
    ///   site); stateful shield-pool / CC-fade correlations underdetermined on both.
    /// * 19 (death): count is right but positioning is the parser's intra-timestamp
    ///   tiebreak (not in the stream); reworking it would regress the exact count.
    /// * 27 (interrupted): 17 of 19 emitted byte-correct (EXACT on Ossein); the 2
    ///   dropped need a tuple for an interrupting ability seen only on an END_CAST
    ///   INTERRUPTED — tied to the missing-tuple tail.
    /// Tighten these bounds (toward 0) as more rules are proven.
    #[test]
    fn per_code_counts_stay_within_known_bounds() {
        let base = env!("CARGO_MANIFEST_DIR");
        let raw_path = format!("{base}/../.decode-samples/combat_raw_encounter.log");
        let off_path = format!("{base}/../.decode-samples/combat_fights_segment.txt");
        let (Ok(raw), Ok(official)) = (
            std::fs::read_to_string(&raw_path),
            std::fs::read_to_string(&off_path),
        ) else {
            return; // golden pair not present — nothing to check.
        };
        let lines: Vec<&str> = raw.lines().collect();
        let ours = build_fights_segment(&lines).expect("our segment builds");

        fn code_counts(seg: &str) -> std::collections::BTreeMap<String, i64> {
            let mut m = std::collections::BTreeMap::new();
            for l in seg.lines().skip(2) {
                if let Some(code) = l.split('|').nth(1) {
                    *m.entry(code.to_string()).or_insert(0) += 1;
                }
            }
            m
        }
        let oc = code_counts(&ours);
        let fc = code_counts(&official);
        let get = |m: &std::collections::BTreeMap<String, i64>, c: &str| *m.get(c).unwrap_or(&0);

        // Codes reproduced EXACTLY (count delta must be 0).
        for c in [
            "2", "6", "8", "10", "11", "12", "15", "19", "22", "26", "44", "52", "53",
        ] {
            assert_eq!(
                get(&oc, c),
                get(&fc, c),
                "code {c} must stay byte-count-exact vs the official segment"
            );
        }
        // Residual codes: bound the absolute delta so it can't regress past the
        // current best. (max |ours − official| we accept for each.)
        let bound = |c: &str, max: i64| {
            let d = (get(&oc, c) - get(&fc, c)).abs();
            assert!(
                d <= max,
                "code {c} delta {d} exceeds the allowed bound {max} (ours {} / official {})",
                get(&oc, c),
                get(&fc, c)
            );
        };
        bound("5", 300); // passive-aura residual (currently +260)
        bound("7", 20); // FADED edge (currently +10)
                        // code-1 tail format is now byte-correct (IMMUNE/DODGED single flag, blocked,
                        // overflow-fold, target-dead). The residual −139 is the zero-hit/zero-overflow
                        // damage events the official keeps but we drop: the emit-vs-drop discriminator
                        // lives in the parser crate's is_damage_event and is underdetermined from one
                        // capture (cross-faction count 482 ≠ emitted 99) — left as a documented drop.
        bound("1", 200); // currently -139 (zero-zero damage drop)
        bound("3", 4); // now EXACT on Maarselok (zero-zero in-combat heals dropped)
        bound("4", 40); // (currently -18)
        bound("16", 520); // status-queued cast source (currently -492)
                          // Not-yet-modeled rare codes: bound at their full official count (we emit 0).
        bound("9", 30);
        bound("14", 10);
        bound("27", 4); // 17/19 emitted byte-correct; 2 need an END_CAST-only tuple
        bound("28", 30);
        bound("38", 650);
        bound("41", 2);
        bound("51", 2);
    }

    /// Cross-capture regression guard: on the SECOND (Ossein trial) capture, the
    /// high-volume event vocabulary must stay byte-count-EXACT. This locks the proof
    /// that the buff/effect (5/6/7/8/10/11/12), cast (15/16), dot/power (2/26),
    /// interrupt (27) and shield (38) encoders are correct on an independent
    /// encounter — so the large Maarselok-only residuals (code 5/16) are capture
    /// artifacts, not rules to chase. No-op until the Ossein pair is staged.
    #[test]
    fn ossein_capture_high_volume_codes_stay_exact() {
        let base = env!("CARGO_MANIFEST_DIR");
        let Ok(raw) = std::fs::read_to_string(format!("{base}/../.decode-samples/ossein_raw.log"))
        else {
            return; // second capture not present — nothing to check.
        };
        let mut official = String::new();
        for n in 1..=20 {
            if let Ok(s) = std::fs::read_to_string(format!(
                "{base}/../.decode-samples/ossein_fights_segment_{n}.txt"
            )) {
                official.push_str(&s.lines().skip(2).collect::<Vec<_>>().join("\n"));
                official.push('\n');
            }
        }
        if let Ok(s) = std::fs::read_to_string(format!(
            "{base}/../.decode-samples/ossein_fights_segment.txt"
        )) {
            official.push_str(&s.lines().skip(2).collect::<Vec<_>>().join("\n"));
        }
        if official.is_empty() {
            return; // segment files not staged
        }
        let lines: Vec<&str> = raw.lines().collect();
        let ours = build_fights_segment(&lines).expect("our segment builds");
        let our_body: String = ours.lines().skip(2).collect::<Vec<_>>().join("\n");

        fn counts(body: &str) -> std::collections::BTreeMap<String, i64> {
            let mut m = std::collections::BTreeMap::new();
            for l in body.lines() {
                if let Some(c) = l.split('|').nth(1) {
                    *m.entry(c.to_string()).or_insert(0) += 1;
                }
            }
            m
        }
        let oc = counts(&our_body);
        let fc = counts(&official);
        let get = |m: &std::collections::BTreeMap<String, i64>, c: &str| *m.get(c).unwrap_or(&0);
        for c in [
            "2", "5", "6", "7", "8", "10", "11", "12", "15", "16", "26", "27", "38", "41", "44",
            "51", "52", "53",
        ] {
            assert_eq!(
                get(&oc, c),
                get(&fc, c),
                "code {c} must stay byte-count-exact on the Ossein capture"
            );
        }
    }

    /// DIAGNOSTIC (manual, `--ignored`): diff OUR MASTER TABLE against the official
    /// captured master table for the same log. The segment events reference master-
    /// table ids (actors/abilities/tuples); a wrong master table makes the server
    /// ACCEPT the upload (valid envelope) but never render (references unresolvable)
    /// — the exact "accepts but loads forever" symptom. The segment oracle does NOT
    /// cover this; this is the missing check.
    #[test]
    #[ignore = "diagnostic; needs the combat raw + testdata master table"]
    fn diff_against_official_combat_master_table() {
        let base = env!("CARGO_MANIFEST_DIR");
        let raw_path = format!("{base}/../.decode-samples/combat_raw_encounter.log");
        let off_path = format!("{base}/src/uploader/native/testdata/combat_master_table.txt");
        let (Ok(raw), Ok(official)) = (
            std::fs::read_to_string(&raw_path),
            std::fs::read_to_string(&off_path),
        ) else {
            eprintln!("[master] golden master table not present; skipping");
            return;
        };
        let lines: Vec<&str> = raw.lines().collect();
        let ours = super::super::encode::build_master_table(&lines).expect("master builds");

        let o: Vec<&str> = ours.lines().collect();
        let f: Vec<&str> = official.lines().collect();
        eprintln!("[master] LINE COUNT ours={} official={}", o.len(), f.len());
        eprintln!("[master] our header:  {:?}", o.first());
        eprintln!("[master] off header:  {:?}", f.first());
        // Section counts: header, then {lastActorId}\n{actors}{lastAbilityId}\n
        // {abilities}{lastTupleId}\n{tuples}{lastPetId}\n{pets}.
        fn sections(v: &[&str]) -> (String, usize, usize, usize, usize) {
            let mut i = 1;
            let last_actor = v.get(i).unwrap_or(&"?").to_string();
            i += 1;
            let mut a = 0;
            while i < v.len() && v[i].parse::<u64>().is_err() {
                a += 1;
                i += 1;
            }
            i += 1; // skip lastAbilityId
            let mut b = 0;
            while i < v.len() && v[i].parse::<u64>().is_err() {
                b += 1;
                i += 1;
            }
            i += 1; // skip lastTupleId
            let mut t = 0;
            while i < v.len() && v[i].parse::<u64>().is_err() {
                t += 1;
                i += 1;
            }
            i += 1; // skip lastPetId
            let mut p = 0;
            while i < v.len() && !v[i].is_empty() {
                p += 1;
                i += 1;
            }
            (last_actor, a, b, t, p)
        }
        let (oa, oar, oab, ot, op) = sections(&o);
        let (fa, far, fab, ft, fp) = sections(&f);
        eprintln!(
            "[master] OURS: lastActorId={oa} actors={oar} abilities={oab} tuples={ot} pets={op}"
        );
        eprintln!(
            "[master] OFFL: lastActorId={fa} actors={far} abilities={fab} tuples={ft} pets={fp}"
        );
        // Section counts are the bare-integer lines right after each section start.
        // Just report the first few lines of each and the first divergence.
        let n = o.len().min(f.len());
        let mut first_div = None;
        for i in 0..n {
            if o[i] != f[i] {
                first_div = Some(i);
                break;
            }
        }
        match first_div {
            None if o.len() == f.len() => eprintln!("[master] IDENTICAL"),
            None => eprintln!(
                "[master] common prefix identical; LENGTH differs ours={} official={}",
                o.len(),
                f.len()
            ),
            Some(i) => {
                eprintln!("[master] FIRST DIVERGENCE at line {i}:");
                eprintln!("[master]   ours: {}", &o[i][..o[i].len().min(160)]);
                eprintln!("[master]   offl: {}", &f[i][..f[i].len().min(160)]);
            }
        }
    }

    /// Regression guard for the master-table section counts. Locks the tuple +
    /// pet fix (tuples were catastrophically 885 vs 4007; now ~4050 — close enough
    /// that every event resolves its actor/ability reference and the report can
    /// render). No-op without the golden pair.
    #[test]
    fn master_table_section_counts_stay_within_bounds() {
        let base = env!("CARGO_MANIFEST_DIR");
        let raw_path = format!("{base}/../.decode-samples/combat_raw_encounter.log");
        let off_path = format!("{base}/src/uploader/native/testdata/combat_master_table.txt");
        let Ok(raw) = std::fs::read_to_string(&raw_path) else {
            return;
        };
        let _ = std::fs::read_to_string(&off_path); // presence not required for bounds
        let lines: Vec<&str> = raw.lines().collect();
        let ours = super::super::encode::build_master_table(&lines).expect("master builds");
        let v: Vec<&str> = ours.lines().collect();
        // Count tuples + pets: walk to the tuple section and count its records.
        // Sections: header, lastActorId, actors…, lastAbilityId, abilities…,
        // lastTupleId, tuples…, lastPetId, pets…
        let mut i = 1;
        let count_section = |v: &[&str], i: &mut usize| -> usize {
            *i += 1; // skip the lastAssignedId line
            let mut n = 0;
            while *i < v.len() && !v[*i].is_empty() && v[*i].parse::<u64>().is_err() {
                n += 1;
                *i += 1;
            }
            n
        };
        let actors = count_section(&v, &mut i);
        let abilities = count_section(&v, &mut i);
        let tuples = count_section(&v, &mut i);
        let pets = count_section(&v, &mut i);
        // Official: actors 75, abilities 865, tuples 4007, pets 4. Bound each delta
        // so the tuple/pet fix can't silently regress (tuples must NOT collapse back
        // toward 885 — the render-blocking bug).
        assert!(
            actors >= 70 && actors <= 80,
            "actors {actors} out of expected ~75 band"
        );
        assert!(
            abilities >= 860 && abilities <= 890,
            "abilities {abilities} out of expected ~865 band"
        );
        assert!(
            (3900..=4150).contains(&tuples),
            "tuples {tuples} must be ~4007 (was catastrophically 885 — render-blocker)"
        );
        assert_eq!(pets, 4, "pets must be exactly 4");
    }

    /// DIAGNOSTIC (manual, `--ignored`): the SECOND-capture oracle. Diffs our
    /// combined segment for the Ossein Cage trial slice against the official
    /// captured segment(s) for the SAME slice, triangulating the rare-code rules
    /// (death positioning, code-9/14/27/28 forms, the zero-hit-damage predicate)
    /// against the Maarselok dungeon capture. Because the official uploader splits
    /// the slice into per-fight segments, the official side concatenates the bodies
    /// of every `ossein_fights_segment*.txt` it finds.
    ///
    /// Stage the capture as:
    ///   `.decode-samples/ossein_raw.log`            (the uploaded slice)
    ///   `.decode-samples/ossein_fights_segment.txt` (the decoded official segment;
    ///      or multiple `ossein_fights_segment_1.txt`, `_2.txt`, … for the splits)
    /// then run with `--ignored --nocapture`. No-op (returns) until present.
    #[test]
    #[ignore = "second-capture oracle; needs .decode-samples/ossein_* , run with --ignored --nocapture"]
    fn diff_against_official_ossein_segment() {
        let base = env!("CARGO_MANIFEST_DIR");
        let raw_path = format!("{base}/../.decode-samples/ossein_raw.log");
        let Ok(raw) = std::fs::read_to_string(&raw_path) else {
            eprintln!("[ossein] raw slice not present ({raw_path}); skipping");
            return;
        };
        // Gather the official segment(s): a single ossein_fights_segment.txt, or the
        // numbered per-fight splits ossein_fights_segment_1.txt, _2.txt, …
        let mut official_bodies: Vec<String> = Vec::new();
        let single = format!("{base}/../.decode-samples/ossein_fights_segment.txt");
        if let Ok(s) = std::fs::read_to_string(&single) {
            // Drop the 2-line header (logVersion|gameVersion, totalEventCount).
            official_bodies.push(s.lines().skip(2).collect::<Vec<_>>().join("\n"));
        }
        for n in 1..=20 {
            let p = format!("{base}/../.decode-samples/ossein_fights_segment_{n}.txt");
            if let Ok(s) = std::fs::read_to_string(&p) {
                official_bodies.push(s.lines().skip(2).collect::<Vec<_>>().join("\n"));
            }
        }
        if official_bodies.is_empty() {
            eprintln!("[ossein] no official segment files present; skipping");
            return;
        }
        let official_body = official_bodies.join("\n");

        let lines: Vec<&str> = raw.lines().collect();
        let ours = build_fights_segment(&lines).expect("our segment builds");
        let our_body: String = ours.lines().skip(2).collect::<Vec<_>>().join("\n");

        fn code_counts(body: &str) -> std::collections::BTreeMap<String, i64> {
            let mut m = std::collections::BTreeMap::new();
            for l in body.lines() {
                if let Some(c) = l.split('|').nth(1) {
                    *m.entry(c.to_string()).or_insert(0) += 1;
                }
            }
            m
        }
        let oc = code_counts(&our_body);
        let fc = code_counts(&official_body);
        let mut codes: std::collections::BTreeSet<String> = oc.keys().cloned().collect();
        codes.extend(fc.keys().cloned());
        eprintln!("[ossein] per-code (code: ours / official):");
        for c in &codes {
            let o = oc.get(c).copied().unwrap_or(0);
            let f = fc.get(c).copied().unwrap_or(0);
            let mark = if o == f { "" } else { "  <-- DIFF" };
            eprintln!("[ossein]   {c:>4}: {o:>7} / {f:>7}{mark}");
        }
        // Print the first few official lines of each rare code so the byte forms can
        // be triangulated against the Maarselok capture.
        for code in ["9", "14", "19", "22", "27", "28", "38"] {
            let off_samples: Vec<&str> = official_body
                .lines()
                .filter(|l| l.split('|').nth(1) == Some(code))
                .take(4)
                .collect();
            if !off_samples.is_empty() {
                eprintln!("[ossein] official code-{code} samples:");
                for s in off_samples {
                    eprintln!("[ossein]     {s}");
                }
            }
        }
    }
}
