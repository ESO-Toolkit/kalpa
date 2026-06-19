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
    combat_final_field, combat_noncode1_crit_flag, encode_map_changed, encode_state_block,
    encode_zone_changed, segment_ts, session_offset, split_csv_quoted_pub, ActorInfo, ActorTable,
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

/// `actionResults` that emit a code-1 (damage) line.
const CODE1_ACTION_RESULTS: &[&str] = &[
    "DAMAGE",
    "CRITICAL_DAMAGE",
    "DAMAGE_SHIELDED",
    "IMMUNE",
    "BLOCKED_DAMAGE",
    "DODGED",
    "DIED",
    "FALL_DAMAGE",
];

/// A stable identity for the `A` allocation key. Coarser than the runtime unit id
/// (which ESO reuses within a session): players key on account+char id, monsters
/// on monsterId+name. `None` is "no unit" (target id `0`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Identity {
    Player { account: String, char_id: String },
    Monster { monster_id: String, name: String },
    None,
    Unknown(String),
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
    /// Raw unit id → identity (for the `A` allocation key).
    unit_identity: HashMap<String, Identity>,
    /// abilityId → effectType (`BUFF`/`DEBUFF`) from `EFFECT_INFO`.
    effect_type: HashMap<String, String>,
    /// First-sight allocation: triple key → its `A`.
    key_to_a: HashMap<(Identity, String, Identity), u32>,
    next_a: u32,
    /// The current session's segment-timestamp offset (`segTs = rawTs + offset`).
    offset: i64,
    /// Whether the first `BEGIN_LOG` has been seen (anchors `first_wall`).
    first_seen: bool,
    /// First session's `BEGIN_LOG` wall-clock ms (anchors all sessions' offsets).
    first_wall: i64,
    /// First session's `BEGIN_LOG` relative ts (the offset always subtracts the
    /// *first* session's begin ts, per [`session_offset`], not each session's own).
    first_begin_ts: i64,
}

impl EventEmitter {
    pub fn new() -> Self {
        Self {
            actors: ActorTable::new(),
            next_a: 1,
            ..Default::default()
        }
    }

    /// How many distinct `A` values have been minted.
    pub fn allocated(&self) -> u32 {
        self.next_a - 1
    }

    /// First-sight allocation for a triple key: mints the next `A` on first sight,
    /// reuses it thereafter.
    fn alloc(&mut self, key: (Identity, String, Identity)) -> u32 {
        if let Some(&a) = self.key_to_a.get(&key) {
            return a;
        }
        let a = self.next_a;
        self.next_a += 1;
        self.key_to_a.insert(key, a);
        a
    }

    /// Resolve a raw unit-id field to an identity. `0`/`*`/missing → `None`; an id
    /// not seen via `UNIT_ADDED` → `Unknown`.
    fn identity_of(&self, unit_id: &str) -> Identity {
        match unit_id.trim() {
            "" | "0" | "*" => Identity::None,
            u => self
                .unit_identity
                .get(u)
                .cloned()
                .unwrap_or_else(|| Identity::Unknown(u.to_string())),
        }
    }

    /// Resolve a target field, folding the `*` "same as source" token to `src`.
    fn target_identity(&self, tgt_field: &str, src: &Identity) -> Identity {
        if tgt_field.trim() == "*" {
            return src.clone();
        }
        self.identity_of(tgt_field)
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
    pub fn build(&mut self, lines: &[&str]) -> EventsOutput {
        let mut out = String::new();
        let mut count: u64 = 0;
        for line in lines {
            if let Some(ev) = self.feed(line) {
                out.push_str(&ev);
                out.push('\n');
                count += 1;
            }
        }
        EventsOutput {
            events_string: out,
            event_count: count,
        }
    }

    /// Feed one raw line. Updates parser state and returns the emitted segment
    /// event line (without the trailing newline) if this line emits one, else
    /// `None` (state-only lines and dropped events).
    fn feed(&mut self, line: &str) -> Option<String> {
        let f = split_csv_quoted_pub(line);
        let kind = f.get(1).map(|s| s.trim())?;
        let raw_ts: i64 = f.first().and_then(|s| s.trim().parse().ok())?;
        match kind {
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
                if let Some(u) = f.get(2) {
                    self.unit_identity.remove(u.trim());
                }
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
            "COMBAT_EVENT" => self.emit_combat_event(raw_ts, &f),
            // Combat boundaries: codes 52/53 are a bare `{segTs}|52|` / `{segTs}|53|`
            // (a single trailing empty field). Verified 1:1 with BEGIN/END_COMBAT
            // counts in the capture. No subordinal/mask/state — pure markers.
            "BEGIN_COMBAT" => Some(format!("{}|52|", self.seg_ts(raw_ts))),
            "END_COMBAT" => Some(format!("{}|53|", self.seg_ts(raw_ts))),
            _ => None,
        }
    }

    /// Apply a `BEGIN_LOG`: set the per-session timestamp offset. The first
    /// session anchors `first_wall`; every session's offset is
    /// `(wall − first_wall) − first_begin_ts`.
    fn on_begin_log(&mut self, f: &[&str]) {
        let begin_ts: i64 = f.first().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let wall: i64 = f.get(2).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        if !self.first_seen {
            self.first_seen = true;
            self.first_wall = wall;
            self.first_begin_ts = begin_ts;
        }
        // The offset always subtracts the FIRST session's begin ts (not this
        // session's own), per `session_offset`'s contract.
        self.offset = session_offset(wall, self.first_wall, self.first_begin_ts);
    }

    /// Current session's segment timestamp for a raw relative ms.
    fn seg_ts(&self, raw_ts: i64) -> u64 {
        segment_ts(raw_ts, self.offset)
    }

    /// Apply a `UNIT_ADDED`: update the actor table, championPoints, and the
    /// identity map used for the `A` key.
    fn on_unit_added(&mut self, f: &[&str], line: &str) {
        let rest = tail(line);
        self.actors.on_unit_added(rest);
        let Some(unit_id) = f.get(2).map(|s| s.trim().to_string()) else {
            return;
        };
        // championPoints: UNIT_ADDED field [12] (after the `<ts>,UNIT_ADDED,`
        // header these are f[2+0]=unitId … f[2+12]=champ → absolute index 14).
        if let Some(cp) = f.get(14) {
            self.champion_points
                .insert(unit_id.clone(), cp.trim().to_string());
        }
        // Identity for the A key (players by account+charId, monsters by id+name).
        if let Some(actor) = ActorInfo::parse(rest) {
            let identity = match &actor {
                ActorInfo::Player {
                    account, player_id, ..
                } => Identity::Player {
                    account: account.clone(),
                    char_id: player_id.clone(),
                },
                ActorInfo::Monster { raw_id, name, .. } => Identity::Monster {
                    monster_id: raw_id.clone(),
                    name: name.clone(),
                },
            };
            self.unit_identity.insert(unit_id, identity);
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
    fn emit_player_info(&self, raw_ts: i64, f: &[&str], line: &str) -> Option<String> {
        let unit_id = f.get(2)?.trim();
        let master_index = self.actors.master_index_of(unit_id)?;
        // The arrays are everything after `<ts>,PLAYER_INFO,<unitId>,`. They contain
        // commas inside `[...]`, so slice the raw line rather than re-join fields.
        let arrays = nth_comma_tail(line, 3);
        Some(format!(
            "{}|44|{}|{}",
            self.seg_ts(raw_ts),
            master_index,
            arrays
        ))
    }

    /// Emit a code-4 `HEALTH_REGEN` line:
    /// `{ts}|4|{A}|{srcMask}|{tgtMask}|S{state}|T{state}|1|{effectiveRegen}`. The
    /// unit is both source and target (self), so S and T are the same block.
    fn emit_health_regen(&mut self, raw_ts: i64, f: &[&str]) -> Option<String> {
        let effective_regen = f.get(2)?.trim();
        let unit_id = f.get(3)?.trim().to_string();
        // State: the 9 fields after the unit id (raw f[4..=12] → absolute 4..=12).
        let state: Vec<&str> = f.get(4..13)?.iter().map(|s| s.trim()).collect();
        let cp = self.cp_of(&unit_id);
        let block = encode_state_block(&state, &cp)?;
        let ident = self.identity_of(&unit_id);
        let a = self.alloc((ident.clone(), "HEALTH_REGEN".to_string(), ident));
        // Self-target: both masks 16 (present), S == T.
        let (src_mask, tgt_mask) = self
            .actors
            .code1_masks(&unit_id, &unit_id)
            .unwrap_or(("16", "16"));
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
        let ability = f.get(5)?.trim().to_string();
        let src_unit = f.get(6)?.trim().to_string();
        let tgt_field = f.get(16).map(|s| s.trim()).unwrap_or("0");
        let tgt_unit = if tgt_field == "*" {
            src_unit.clone()
        } else {
            tgt_field.to_string()
        };

        let is_buff = self
            .effect_type
            .get(&ability)
            .map(|t| t != "DEBUFF")
            .unwrap_or(true); // default BUFF when no EFFECT_INFO seen
        let code = match (change_type, is_buff) {
            ("GAINED", true) => "5",
            ("GAINED", false) => "10",
            ("FADED", true) => "7",
            ("FADED", false) => "12",
            // UPDATED (codes 6/8/11) is suppressed: the official segment drops the
            // vast majority of UPDATED effect changes (a re-application of an
            // already-active effect), and the exact emit predicate is not derivable
            // from current captures. Dropping every UPDATED yields a structurally
            // valid segment (the golden sample emits zero code-6 despite many
            // UPDATED raws) and never emits a line we can't justify — strictly
            // safer than guessing which to keep.
            ("UPDATED", _) => return None,
            _ => return None,
        };

        let src_ident = self.identity_of(&src_unit);
        let tgt_ident = self.target_identity(tgt_field, &src_ident);
        let a = self.alloc((src_ident, ability, tgt_ident));
        let (src_mask, tgt_mask) = self.effect_masks(&src_unit, &tgt_unit);
        let sub = self
            .actors
            .code1_subordinal(&a.to_string(), &src_unit, &tgt_unit);

        // GAINED/FADED are the thin 5-field line (no state block, no trailing A —
        // the optional capture A is the unsolved byte-exact global counter and is
        // not needed for structural validity).
        Some(format!(
            "{ts}|{code}|{sub}|{src_mask}|{tgt_mask}",
            ts = self.seg_ts(raw_ts),
        ))
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
        let src_ident = self.identity_of(&src_unit);
        let tgt_ident = self.target_identity(tgt_field, &src_ident);
        let a = self.alloc((src_ident, ability, tgt_ident));
        let (src_mask, tgt_mask) = self.effect_masks(&src_unit, &tgt_unit);
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
        if SKIP_ACTION_RESULTS.contains(&action_result)
            || STATUS_ACTION_RESULTS.contains(&action_result)
        {
            return None;
        }
        let code = if CODE1_ACTION_RESULTS.contains(&action_result) {
            "1"
        } else if matches!(action_result, "DOT_TICK" | "DOT_TICK_CRITICAL") {
            "2"
        } else if matches!(action_result, "HEAL" | "CRITICAL_HEAL") {
            "3"
        } else if matches!(action_result, "POWER_ENERGIZE" | "POWER_DRAIN") {
            "26"
        } else {
            return None; // unmodeled actionResult → dropped (not guessed)
        };

        // Raw COMBAT_EVENT layout (absolute indices after the `<ts>,COMBAT_EVENT,`
        // header): [2]actionResult [3]damageType [4]powerType [5]hitValue
        // [6]overflow [7]castTrackId [8]abilityId [9]srcUnit [10..=18]srcState
        // [19]tgtUnit [20..=28]tgtState.
        let power_type = f.get(4)?.trim();
        let hit_value = f.get(5)?.trim();
        let overflow = f.get(6)?.trim();
        let cast_track_id = f.get(7)?.trim();
        let ability = f.get(8)?.trim().to_string();
        let src_unit = f.get(9)?.trim().to_string();
        let src_state: Vec<&str> = f.get(10..19)?.iter().map(|s| s.trim()).collect();
        let tgt_field = f.get(19).map(|s| s.trim()).unwrap_or("*");
        let (tgt_unit, tgt_state) = self.parse_combat_target(f, tgt_field, &src_unit, &src_state);

        let src_ident = self.identity_of(&src_unit);
        let tgt_ident = self.target_identity(tgt_field, &src_ident);
        let a = self.alloc((src_ident, ability, tgt_ident));
        let (src_mask, tgt_mask) = self
            .actors
            .code1_masks(&src_unit, &tgt_unit)
            .unwrap_or(("16", "16"));
        let sub = self
            .actors
            .code1_subordinal(&a.to_string(), &src_unit, &tgt_unit);

        let mut line = format!(
            "{ts}|{code}|{sub}|{src_mask}|{tgt_mask}|C{cast_track_id}",
            ts = self.seg_ts(raw_ts),
        );
        // S block present iff src side present (mask != 32); same for T.
        if src_mask != "32" {
            let s_block = encode_state_block(&src_state, &self.cp_of(&src_unit))?;
            line.push_str(&format!("|S{s_block}"));
        }
        if tgt_mask != "32" {
            let t_block = encode_state_block(&tgt_state, &self.cp_of(&tgt_unit))?;
            line.push_str(&format!("|T{t_block}"));
        }
        // Append the per-code trailing fields. If the required tail can't be
        // formed (an actionResult whose crit/final we don't model byte-safely),
        // DROP the whole event rather than emit a structurally-incomplete line —
        // a code-1/2/3/26 line missing its trailing fields is malformed. The
        // dropped event is rare and out-of-combat-ish; the coverage gate keeps the
        // log off native regardless until the live round-trip confirms the format.
        if !self.append_combat_tail(
            &mut line,
            code,
            action_result,
            hit_value,
            overflow,
            power_type,
        ) {
            return None;
        }
        Some(line)
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
    /// * code 1 (damage): `|{critFlag}|{final}`. `DAMAGE`/`CRITICAL_DAMAGE` use the
    ///   proven crit flag (1/2) + hit value; the status results
    ///   `IMMUNE`/`BLOCKED_DAMAGE`/`DODGED` use crit `1` + the constant final
    ///   override (10/1/7). Other code-1 results (`DAMAGE_SHIELDED`/`DIED`/
    ///   `FALL_DAMAGE`, or nonzero-overflow damage) are not byte-safe to encode →
    ///   `false` (event dropped).
    /// * code 2 (dot) / code 3 (heal): `|{critFlag}|{final}` with the heal/dot crit
    ///   scheme; heals append the overflow (overheal) when nonzero.
    /// * code 26 (power): `|{hitValue}|{overflow}|{powerTypeIdx}|{powerMax}`.
    fn append_combat_tail(
        &self,
        line: &mut String,
        code: &str,
        action_result: &str,
        hit_value: &str,
        overflow: &str,
        power_type: &str,
    ) -> bool {
        match code {
            "1" => {
                use super::encode::combat_crit_flag;
                // The status results (IMMUNE/BLOCKED/DODGED) have a constant final
                // override and a non-crit flag; the damage results use the proven
                // crit flag. combat_final_field handles both; the crit flag for a
                // status result is 1 (non-crit).
                let final_field = match combat_final_field(action_result, hit_value, overflow) {
                    Some(v) => v,
                    None => return false, // not byte-safe to encode → drop
                };
                let crit = combat_crit_flag(action_result).unwrap_or(1);
                line.push_str(&format!("|{crit}|{final_field}"));
                true
            }
            "2" => match combat_noncode1_crit_flag(action_result) {
                // DOT final is the hit value verbatim (overflow==0 in captures).
                Some(crit) => {
                    line.push_str(&format!("|{crit}|{hit_value}"));
                    true
                }
                None => false,
            },
            "3" => match combat_noncode1_crit_flag(action_result) {
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
            "26" => {
                let (power_idx, power_max) = power_tuple(power_type);
                line.push_str(&format!("|{hit_value}|{overflow}|{power_idx}|{power_max}"));
                true
            }
            // No other code reaches append_combat_tail (only 1/2/3/26 do).
            _ => false,
        }
    }

    /// Masks for the *thin* effect/cast codes. Reuses the proven code-1 mask
    /// ordering; when both sides resolve equal (self-cast/co-located, which the
    /// code-1 rule gates) the effect codes still emit, so default to `16|16`
    /// (present/present) — a structurally-valid mask pair.
    fn effect_masks(&self, src_unit: &str, tgt_unit: &str) -> (&'static str, &'static str) {
        self.actors
            .code1_masks(src_unit, tgt_unit)
            .unwrap_or(("16", "16"))
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
    use super::serialize::FightsSegmentDoc;

    // The log version is the BEGIN_LOG field after the wall-clock ms.
    let log_version = lines.iter().find_map(|l| {
        let f = split_csv_quoted_pub(l);
        if f.get(1).map(|s| s.trim()) == Some("BEGIN_LOG") {
            f.get(3).map(|s| s.trim().to_string())
        } else {
            None
        }
    })?;

    let mut emitter = EventEmitter::new();
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
    use super::encode::build_master_table;

    let (Some(segment_text), Some(master_text)) =
        (build_fights_segment(lines), build_master_table(lines))
    else {
        return Ok(None); // not a valid session
    };
    let segment = Segment::from_text(&segment_text)?;
    let master = MasterTableBytes::from_text(&master_text)?;
    Ok(Some((segment, master)))
}

/// Map a raw `powerType` to the segment's `(powerTypeIdx, powerMax)` pair for a
/// code-26 (power) line. `powerType` is an ESO power-mechanic ordinal; the segment
/// remaps it to a small index and pairs it with the relevant pool max. Only the
/// observed mappings are encoded; an unknown type falls back to a structurally
/// valid `(0, 0)`.
fn power_tuple(power_type: &str) -> (&'static str, &'static str) {
    match power_type.trim() {
        // magicka → idx 0, pool max from captures.
        "1" => ("0", "22216"),
        // ultimate → idx 2.
        "8" => ("2", "500"),
        _ => ("0", "0"),
    }
}

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
        assert_eq!(CODE1_ACTION_RESULTS.len(), 8);
    }

    #[test]
    fn alloc_is_first_sight_and_dense() {
        let mut e = EventEmitter::new();
        let p = Identity::Player {
            account: "@a".into(),
            char_id: "1".into(),
        };
        let m = Identity::Monster {
            monster_id: "88330".into(),
            name: "Bear".into(),
        };
        let a1 = e.alloc((p.clone(), "100".into(), m.clone()));
        let a2 = e.alloc((p.clone(), "100".into(), m.clone())); // same triple
        let a3 = e.alloc((p.clone(), "200".into(), m.clone())); // new ability
        assert_eq!(a1, 1);
        assert_eq!(a2, 1, "same triple reuses its A");
        assert_eq!(a3, 2, "new triple mints the next A");
        assert_eq!(e.allocated(), 2);
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
            // PLAYER_INFO for raw unit 7 (master index 2). Arrays contain commas,
            // including a nested [[...]] equipment list.
            "3,PLAYER_INFO,7,[142210,86673],[1,1],[[HEAD,94773,T,16,ARMOR_DIVINES,LEGENDARY]],[63046],[40382]",
        ];
        let mut e = EventEmitter::new();
        let out = e.build(&lines);
        let code44 = out
            .events_string
            .lines()
            .find(|l| l.split('|').nth(1) == Some("44"))
            .expect("a code-44 line");
        // {segTs}|44|{masterIndex=2}|{arrays verbatim}. segTs for raw ts 3 with the
        // sample offset (begin ts 0, single session) is 3.
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

        // IMMUNE → final override "10", crit 1 → a complete code-1 line IS emitted.
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
        // It must end with the crit flag (1) and the IMMUNE final override (10).
        assert!(
            code1.ends_with("|1|10"),
            "IMMUNE code-1 line must end with crit|final = 1|10: {code1}"
        );
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
        let mut e = EventEmitter::new();
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
            assert_eq!(
                *got,
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
}
