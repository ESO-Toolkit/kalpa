//! Raw-line → master-table encoding (the high-confidence, byte-exact core).
//!
//! This module encodes the parts of the transform that are **unambiguously
//! determined** by the matched golden pair (`testdata/sample_raw_encounter.log`
//! → `sample_master_table.txt`) and proven byte-exact by golden tests:
//!
//! * **Ability records** — `ABILITY_INFO` → `Name|2|iconBasename|0|flags`.
//! * **Actor records** — `UNIT_ADDED` (player) → the `Name^@Account^id^T|...`
//!   record.
//!
//! The stateful *event* correlation (cast pairing, effect indexing — segment
//! codes 5/7/15/16) is deliberately NOT here: the golden-pair analysis found that
//! logic ambiguous from a single sample (contradictory indexing theories,
//! unexplained omissions). Encoding it by guess would risk silent corruption, so
//! it waits for more golden samples. The coverage gate keeps any log needing
//! those codes on the official uploader until each is proven here.
//!
//! Clean-room: rules derived by comparing our own captured input/output; no
//! third-party code.

/// Strip an ESO icon path to the basename the master table uses:
/// `/esoui/art/icons/ability_rogue_035.dds` → `ability_rogue_035`.
pub fn icon_basename(icon_path: &str) -> &str {
    let p = icon_path.trim_matches('"');
    let after_slash = p.rsplit('/').next().unwrap_or(p);
    after_slash.strip_suffix(".dds").unwrap_or(after_slash)
}

/// The two trailing booleans of an `ABILITY_INFO` line (`…,F,T`) → the master
/// record's flags byte. Derived byte-exact from the golden sample:
/// `F,T → 1`, `T,T → 3`, `F,F → 0`, `T,F → 2` — i.e. `2*f6 + f7` with T=1, F=0.
pub fn ability_flags(f6_is_true: bool, f7_is_true: bool) -> u8 {
    (u8::from(f6_is_true) << 1) | u8::from(f7_is_true)
}

/// Parse a quoted-or-bare CSV field, stripping surrounding quotes.
fn unquote(s: &str) -> &str {
    s.trim().trim_matches('"')
}

/// A parsed `ABILITY_INFO` line's fields needed for the master record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityInfo {
    pub ability_id: u64,
    pub name: String,
    pub icon: String,
    pub f6: bool,
    pub f7: bool,
}

impl AbilityInfo {
    /// Parse from the comma-separated tail after `<ts>,ABILITY_INFO,`.
    /// Layout: `<abilityId>,"<name>","<icon>",<f6>,<f7>`.
    pub fn parse(rest: &str) -> Option<Self> {
        // Name/icon are quoted and may contain commas, so split carefully: the
        // ability id is the first field, then two quoted strings, then two bools.
        let mut fields = split_csv_quoted(rest);
        let ability_id = fields.next()?.trim().parse().ok()?;
        let name = unquote(fields.next()?).to_string();
        let icon = icon_basename(fields.next()?).to_string();
        let f6 = unquote(fields.next()?) == "T";
        let f7 = unquote(fields.next()?) == "T";
        Some(AbilityInfo {
            ability_id,
            name,
            icon,
            f6,
            f7,
        })
    }

    /// Render the master-table ability record: `Name|2|abilityId|icon|0|flags`.
    /// The `2` and `0` are constants observed in every record.
    pub fn to_master_record(&self) -> String {
        format!(
            "{}|2|{}|{}|0|{}",
            self.name,
            self.ability_id,
            self.icon,
            ability_flags(self.f6, self.f7)
        )
    }
}

/// The per-session offset that turns a raw relative timestamp into a segment
/// timestamp: `segment_ts = raw_ts + offset`, where
/// `offset = (wall_time_session − wall_time_first_session) − begin_ts_first`.
///
/// Verified byte-exact against every timestamp in the golden pair (session 1:
/// offset −4; session 2: offset 84016105). All-integer math.
pub fn session_offset(wall_time: i64, first_wall_time: i64, first_begin_ts: i64) -> i64 {
    (wall_time - first_wall_time) - first_begin_ts
}

/// Apply a session offset to a raw timestamp. Saturates at 0 (a segment ts is
/// never negative; the first event lands at/after 0 by construction).
pub fn segment_ts(raw_ts: i64, offset: i64) -> u64 {
    (raw_ts + offset).max(0) as u64
}

/// Encode a `ZONE_CHANGED` raw line into its segment event (code 41).
/// Raw: `<ts>,ZONE_CHANGED,<zoneId>,"<name>",<difficulty>` →
/// segment: `<ts>|41|<zoneId>|<name>|<difficultyInt>`. The trailing field is the
/// zone difficulty as an integer: `NONE → 0`, `NORMAL → 1`, `VETERAN → 2` (the
/// first two proven against captures; `VETERAN` follows the sequential mapping and
/// is in any case a structurally-valid integer). An unrecognized token maps to 0.
pub fn encode_zone_changed(seg_ts: u64, rest: &str) -> Option<String> {
    let mut f = split_csv_quoted(rest);
    let zone_id = f.next()?.trim();
    let name = unquote(f.next()?);
    let difficulty = zone_difficulty_int(unquote(f.next().unwrap_or("")));
    Some(format!("{seg_ts}|41|{zone_id}|{name}|{difficulty}"))
}

/// Map a `ZONE_CHANGED` difficulty token to its segment integer. Unknown tokens
/// fall back to `0` (a structurally-valid value); the known mapping keeps us
/// self-consistent with captured segments.
fn zone_difficulty_int(token: &str) -> u8 {
    match token.trim() {
        "NORMAL" => 1,
        "VETERAN" => 2,
        // "NONE" and anything unrecognized → 0.
        _ => 0,
    }
}

/// Encode a `MAP_CHANGED` raw line into its segment event (code 51).
/// Raw: `<ts>,MAP_CHANGED,<mapId>,"<name>","<resource>"` →
/// segment: `<ts>|51|<mapId>|<name>|<resourceLowercased>`. The display `name`
/// keeps its original case; the `resource` path is ASCII-lowercased (verified
/// against a capture where `grahtwood/MaarsOutsideMap001_base` →
/// `grahtwood/maarsoutsidemap001_base` while the name stayed mixed-case).
pub fn encode_map_changed(seg_ts: u64, rest: &str) -> Option<String> {
    let mut f = split_csv_quoted(rest);
    let map_id = f.next()?.trim();
    let name = unquote(f.next()?);
    let resource = unquote(f.next()?).to_ascii_lowercase();
    Some(format!("{seg_ts}|51|{map_id}|{name}|{resource}"))
}

/// A parsed `UNIT_ADDED` line. Handles players (named + anonymized) and monsters.
///
/// Raw `UNIT_ADDED` tail field indices (0-based, after `<relMs>,UNIT_ADDED,`):
/// `[0]unitIdx [1]TYPE [2]? [3]? [4]rawMonsterId(for monsters) [6]race
/// [7]class [8]name [9]account [10]playerId [12]championPoints [14]combatFlag
/// [14]roleType` — verified field-by-field against chunk1/combat master tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorInfo {
    /// A player. `is_local` is the name-token suffix flag (`T` only for the
    /// player whose client wrote the log). Anonymity is separate: a player with
    /// an empty name renders as `nil^nil^a<id>`.
    Player {
        is_local: bool,
        name: String,
        account: String,
        player_id: String,
        race: String,
        class: String,
        champ_points: String,
        /// The unit's registration offset (raw `UNIT_ADDED` field 5). For an
        /// anonymized player the master id is `BEGIN_LOG wall-ms + this offset`.
        reg_offset: u64,
    },
    /// A monster / NPC / pet.
    Monster {
        name: String,
        /// 2 = hostile/boss/summon, 3 = player pet/companion.
        kind: u8,
        raw_id: String,
        icon: String,
        icon_basename: String,
        combat_flag: String,
    },
}

impl ActorInfo {
    /// Parse from the comma tail after `<relMs>,UNIT_ADDED,`. Field positions are
    /// the verified layout. Returns `None` for actor kinds not yet handled.
    pub fn parse(rest: &str) -> Option<Self> {
        let f: Vec<&str> = split_csv_quoted(rest).collect();
        let kind_tok = unquote(f.get(1)?);
        // Common indices observed across all samples:
        //   [2]=isChar(1) [4..]=type-specific. For PLAYER:
        //   T/F anon-flag at [2]? No — the anon flag is the NAME-suffix, derived
        //   from whether name is empty. Field layout (player):
        //   1,PLAYER,T,56,0,F,3,9,"Name","@Acct",playerId,50,champ,0,PLAYER_ALLY,T
        //   indices: [0]unit [1]PLAYER [2]? [3]? [4]? [5]? [6]race [7]class
        //   [8]name [9]account [10]playerId [12]champ
        match kind_tok {
            "PLAYER" => {
                let name = unquote(f.get(8)?).to_string();
                let account = unquote(f.get(9)?).to_string();
                // The name-token suffix flag is the "is local/logging player"
                // field [2] (T only for the player whose client wrote the log),
                // NOT whether the name is present. Verified 11/11 on chunk1.
                Some(ActorInfo::Player {
                    is_local: unquote(f.get(2)?) == "T",
                    name,
                    account,
                    player_id: f.get(10)?.trim().to_string(),
                    race: f.get(6)?.trim().to_string(),
                    class: f.get(7)?.trim().to_string(),
                    champ_points: f.get(12)?.trim().to_string(),
                    reg_offset: f.get(3)?.trim().parse().unwrap_or(0),
                })
            }
            "MONSTER" | "OBJECT" => {
                // 45,MONSTER,F,0,125729,F,0,0,"Name","",0,50,160,0,HOSTILE,F
                // [0]unit [1]MONSTER [2]? [3]? [4]rawId [6]icon [7]? [8]name
                // [12]combatFlag — verified against chunk1 monster record.
                let name = unquote(f.get(8)?).to_string();
                // kind 2 = hostile/boss/summon. Player pets/companions render as
                // kind 3, but distinguishing them needs ownership info not yet
                // decoded; until then everything is 2 and the missing-basename gap
                // (below) keeps monster-bearing logs on fallback anyway.
                Some(ActorInfo::Monster {
                    name,
                    kind: 2,
                    raw_id: f.get(4)?.trim().to_string(),
                    icon: f.get(6)?.trim().to_string(),
                    // icon_basename derives from the monster's later attack/damage
                    // type (NOT in UNIT_ADDED) — left empty until that correlation
                    // is encoded; the coverage gate keeps such logs on fallback.
                    icon_basename: String::new(),
                    combat_flag: f.get(12)?.trim().to_string(),
                })
            }
            _ => None,
        }
    }

    /// Render the master actor record. `index` is the 1-based master actor row
    /// (drives the player role `1000000 + index`); `server` is the session's
    /// `BEGIN_LOG` server (quoted); `begin_wall` is that session's `BEGIN_LOG`
    /// wall-clock ms (used to synthesize an anonymized player's id =
    /// `begin_wall + reg_offset`). Verified byte-exact against named-player,
    /// anonymized-player, and monster records.
    pub fn to_master_record(&self, index: usize, server: &str, begin_wall: u64) -> String {
        match self {
            ActorInfo::Player {
                is_local,
                name,
                account,
                player_id,
                race,
                class,
                champ_points,
                reg_offset,
            } => {
                let role = 1000000 + index as u64;
                let flag = if *is_local { "T" } else { "F" };
                if name.is_empty() {
                    // Anonymized: nil^nil^a<id>, where id = BEGIN_LOG wall-ms +
                    // the unit's registration offset (verified byte-exact).
                    let anon_id = begin_wall + reg_offset;
                    format!(
                        "nil^nil^a{anon_id}^{flag}|1|{role}|{race}|{server}|{class}|nil|{champ_points}"
                    )
                } else {
                    format!(
                        "{name}^{account}^{player_id}^{flag}|1|{role}|{race}|{server}|{class}|nil|{champ_points}"
                    )
                }
            }
            ActorInfo::Monster {
                name,
                kind,
                raw_id,
                icon,
                icon_basename,
                combat_flag,
            } => {
                format!("{name}|{kind}|{raw_id}|{icon}|{server}|0|{icon_basename}|{combat_flag}")
            }
        }
    }

    /// A stable identity key for cross-session dedup. Named players key on
    /// account+id; anonymized players (empty account, id 0) would all collide, so
    /// they key on their registration offset (unique per unit within a session).
    pub fn identity(&self) -> String {
        match self {
            ActorInfo::Player {
                account,
                player_id,
                reg_offset,
                name,
                ..
            } => {
                if name.is_empty() {
                    format!("anon:{reg_offset}")
                } else {
                    format!("p:{account}:{player_id}")
                }
            }
            ActorInfo::Monster { raw_id, name, .. } => format!("m:{raw_id}:{name}"),
        }
    }
}

/// Render a master-table **tuple record** for an ability: `1|flag|index`, where
/// `index` is the ability's 1-based master index and `flag = f6 OR
/// has_effect_info` (`f6` = the ABILITY_INFO line's first trailing boolean). The
/// leading `1` is constant.
///
/// Verified byte-exact against all 9 golden tuple records. The rule is NOT
/// "EFFECT_INFO presence" alone: Swap Weapons has no EFFECT_INFO but `f6=T` →
/// flag 1, while Light/Heavy Attack have `f6=F` and no EFFECT_INFO → flag 0.
pub fn tuple_record(master_index_1based: usize, f6_is_true: bool, has_effect_info: bool) -> String {
    let flag = u8::from(f6_is_true || has_effect_info);
    format!("1|{flag}|{master_index_1based}")
}

/// Encode the normalized-map X coordinate to the state-block integer form:
/// `floor(x * 10000)`. The float multiply is intentional — the official encoder
/// works from the f64 value, so reproducing its representation (e.g.
/// `0.4095 * 10000 = 4094.999… → 4094`) is what makes us byte-exact. Proven
/// 3732/3732 across both source and target state blocks of the combat golden pair.
fn encode_pos_x(v: f64) -> i64 {
    (v * 10000.0).floor() as i64
}

/// Encode the normalized-map Y coordinate: `10000 - floor(y * 10000)`. Y is
/// *flipped* against the map's vertical axis, and the subtraction happens AFTER
/// the floor (operation order matters for byte-exactness). Proven 3732/3732.
///
/// Note: the earlier sign-magnitude form coincided with this only because the
/// single sample then available had a negative Y; this is the corrected,
/// universally-verified rule.
fn encode_pos_y(v: f64) -> i64 {
    10000 - (v * 10000.0).floor() as i64
}

/// Encode the heading (radians) to hundredths: `floor(h * 100)`. **`floor`, not
/// `trunc`** — heading is signed, and the official encoder rounds toward −∞ (e.g.
/// `-2.4237 → -243`, where `trunc` would give `-242`). For the always-positive
/// X/Y this is identical to truncation; it diverges only on negative headings.
/// Proven 3732/3732.
fn encode_pos_heading(v: f64) -> i64 {
    (v * 100.0).floor() as i64
}

/// Render a unit-state block body (without the leading `S`/`T` tag), proven
/// byte-exact against both the golden code-16 cast event and all 3732 source &
/// target state blocks of the combat golden pair.
///
/// A raw `<unitState>` is 10 comma fields: `unitId, health/max, magicka/max,
/// stamina/max, ultimate/max, werewolf/max, shield, mapNormX, mapNormY,
/// headingRadians`. The *encoded* block drops `unitId` (it is carried elsewhere)
/// and reads:
/// `health/max | magicka/max | stamina/max | ultimate/max | werewolf/max |
///  shield | championPoints | encX | encY | encH`.
///
/// `stat_fields` is the slice *after* `unitId` — i.e. the 9 fields
/// `health…shield` (6 of them) followed by `mapNormX, mapNormY, headingRadians`.
/// The first six pass through verbatim; `champion_points` is the unit's CURRENT
/// championPoints (see [`build_master_table`]'s note — initialized from
/// `UNIT_ADDED` and updated by `UNIT_CHANGED`); the three position floats are
/// encoded with [`encode_pos_x`]/[`encode_pos_y`]/[`encode_pos_heading`].
pub fn encode_state_block(stat_fields: &[&str], champion_points: &str) -> Option<String> {
    // Need at least 6 ratio/flag fields + 3 position floats.
    if stat_fields.len() < 9 {
        return None;
    }
    // championPoints must be numeric — a non-numeric value (e.g. a reaction token
    // like "HOSTILE") means an upstream field-index bug fed the wrong field. Bail
    // rather than emit a malformed block; the caller drops the event and the
    // coverage gate keeps the log off native, so corruption never ships.
    if champion_points.trim().parse::<i64>().is_err() {
        return None;
    }
    let passthrough = &stat_fields[0..6];
    let map_x: f64 = stat_fields[6].trim().parse().ok()?;
    let map_y: f64 = stat_fields[7].trim().parse().ok()?;
    let heading: f64 = stat_fields[8].trim().parse().ok()?;
    Some(format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        passthrough[0],
        passthrough[1],
        passthrough[2],
        passthrough[3],
        passthrough[4],
        passthrough[5],
        champion_points,
        encode_pos_x(map_x),
        encode_pos_y(map_y),
        encode_pos_heading(heading),
    ))
}

/// Build the complete master-table text from a raw log's lines, proven byte-exact
/// against the captured sample. Single chronological pass:
///
/// * `BEGIN_LOG` → log version + server (first session's values).
/// * `UNIT_ADDED` (player) → actor records, in order.
/// * `ABILITY_INFO` → ability records (first appearance, global order); each also
///   yields a tuple record (`f6 OR has_effect_info`).
/// * `EFFECT_INFO` → marks its ability as effect-bearing (affects the tuple flag).
///
/// Returns `None` if the log lacks a `BEGIN_LOG` (not a valid session). Pets are
/// always `0` here (none in the sample; non-zero pet handling awaits a sample).
///
/// Because tuple flags depend on EFFECT_INFO that may appear *after* the
/// ABILITY_INFO, this collects abilities first, then resolves flags, then renders.
pub fn build_master_table(lines: &[&str]) -> Option<String> {
    use super::serialize::MasterTableDoc;

    let mut log_version: Option<String> = None;
    let mut server: Option<String> = None;
    let mut begin_wall: u64 = 0;
    let mut actors: Vec<String> = Vec::new();
    // Actors are deduplicated across sessions by identity (the same player
    // re-added in a later session is one actor), matching the golden master
    // table which lists each distinct actor once.
    let mut actor_seen: std::collections::BTreeSet<String> = Default::default();
    // The deduped actor identities in actor order (so the tuple/pet second pass
    // can map an identity back to its 1-based actor index).
    let mut identity_order: Vec<String> = Vec::new();
    // ability id → (record, f6); order tracked separately for stable indices.
    let mut ability_order: Vec<String> = Vec::new();
    let mut ability_seen: std::collections::BTreeSet<String> = Default::default();
    let mut ability_recs: Vec<(String, bool)> = Vec::new(); // (master record, f6)
    let mut effect_ids: std::collections::BTreeSet<String> = Default::default();

    for line in lines {
        let mut it = line.splitn(3, ',');
        let _ts = it.next();
        let Some(kind) = it.next().map(str::trim) else {
            continue;
        };
        let rest = it.next().unwrap_or("");
        match kind {
            "BEGIN_LOG" if log_version.is_none() => {
                // <wallMs>,<logVersion>,"<server>",...
                let mut f = split_csv_quoted(rest);
                begin_wall = f.next().and_then(|w| w.trim().parse().ok()).unwrap_or(0);
                log_version = Some(f.next().unwrap_or("").trim().to_string());
                // server keeps its surrounding quotes (the record embeds them).
                server = Some(f.next().unwrap_or("").trim().to_string());
            }
            "UNIT_ADDED" => {
                if let Some(actor) = ActorInfo::parse(rest) {
                    // Dedup by identity across sessions; the 1-based master index
                    // (assigned on first insert) drives the player role.
                    let identity = actor.identity();
                    if actor_seen.insert(identity.clone()) {
                        let srv = server.clone().unwrap_or_default();
                        let index = actors.len() + 1;
                        actors.push(actor.to_master_record(index, &srv, begin_wall));
                        identity_order.push(identity);
                    }
                }
            }
            "ABILITY_INFO" => {
                if let Some(info) = AbilityInfo::parse(rest) {
                    let id = rest.split(',').next().unwrap_or("").trim().to_string();
                    if ability_seen.insert(id.clone()) {
                        ability_order.push(id);
                        ability_recs.push((info.to_master_record(), info.f6));
                    }
                }
            }
            "EFFECT_INFO" => {
                if let Some(id) = rest.split(',').next() {
                    effect_ids.insert(id.trim().to_string());
                }
            }
            _ => {}
        }
    }

    let log_version = log_version?;

    // Build the index maps the tuple/pet sections need:
    //   identity → 1-based actor index, abilityId → 1-based ability index.
    let identity_to_actor: std::collections::HashMap<String, u32> = identity_order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i as u32 + 1))
        .collect();
    let mut ability_to_index: std::collections::HashMap<String, u32> = ability_order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i as u32 + 1))
        .collect();
    // abilityId "0" → ability index 0 (e.g. SOUL_GEM_RESURRECTION_ACCEPTED).
    ability_to_index.insert("0".to_string(), 0);

    // Second pass: the TUPLE and PET sections. Tuples are the distinct
    // `(srcActorIndex, tgtActorIndex, abilityIndex)` triples referenced by the
    // registering combat/effect/cast events; pets are `(petActorIndex,
    // ownerActorIndex)` for player-owned units. Both need a TIME-AWARE
    // `unitId → actorIndex` live map (unit ids are recycled, so a global lookup
    // mis-pairs).
    let (tuples, pets) = build_tuples_and_pets(lines, &identity_to_actor, &ability_to_index);

    // Render sections.
    let actors_string = join_lines(&actors);
    let abilities_string = join_lines(
        &ability_recs
            .iter()
            .map(|(r, _)| r.clone())
            .collect::<Vec<_>>(),
    );
    let tuples_string = join_lines(&tuples);
    let pets_string = join_lines(&pets);

    let doc = MasterTableDoc {
        log_version: &log_version,
        game_version: "1", // observed constant; uploader-side, not in the raw log
        log_file_details: "",
        last_assigned_actor_id: actors.len() as u64,
        actors_string: &actors_string,
        last_assigned_ability_id: ability_recs.len() as u64,
        abilities_string: &abilities_string,
        last_assigned_tuple_id: tuples.len() as u64,
        tuples_string: &tuples_string,
        last_assigned_pet_id: pets.len() as u64,
        pets_string: &pets_string,
    };
    Some(doc.render())
}

/// Build the master-table TUPLE and PET sections via a time-aware second pass.
///
/// A **tuple** is a distinct `{srcActorIndex}|{tgtActorIndex}|{abilityIndex}`
/// triple referenced by a registering event (a `COMBAT_EVENT`/`EFFECT_CHANGED`/
/// `BEGIN_CAST`). The actor indices come from the live `unitId → actorIndex` map
/// (set on `UNIT_ADDED`, cleared on `UNIT_REMOVED` — unit ids are recycled, so the
/// map must be time-aware). `0` is "unknown/no actor". A self-target (`*`) sets
/// `tgt = src`; a `0` target is "no target"; a triple with both sides unknown, or a
/// `COMBAT_EVENT` whose result never landed, is not registered.
///
/// A **pet** is `{petActorIndex}|{ownerActorIndex}` for a `UNIT_ADDED` whose owner
/// resolves (via the same live map) to a player-side actor.
///
/// Returns `(tuples, pets)` as ordered, de-duplicated record lists.
fn build_tuples_and_pets(
    lines: &[&str],
    identity_to_actor: &std::collections::HashMap<String, u32>,
    ability_to_index: &std::collections::HashMap<String, u32>,
) -> (Vec<String>, Vec<String>) {
    // Live unitId → (actorIndex, is_player). Set on UNIT_ADDED, cleared on REMOVE.
    let mut live: std::collections::HashMap<String, (u32, bool)> = Default::default();
    let mut tuple_seen: std::collections::HashSet<(u32, u32, u32)> = Default::default();
    let mut tuples: Vec<String> = Vec::new();
    let mut pet_seen: std::collections::HashSet<(u32, u32)> = Default::default();
    let mut pets: Vec<String> = Vec::new();

    for line in lines {
        let f = split_csv_quoted_pub(line);
        let Some(kind) = f.get(1).map(|s| s.trim()) else {
            continue;
        };
        match kind {
            "UNIT_ADDED" => {
                let rest = line.splitn(3, ',').nth(2).unwrap_or("");
                let Some(actor) = ActorInfo::parse(rest) else {
                    continue;
                };
                let identity = actor.identity();
                let Some(&idx) = identity_to_actor.get(&identity) else {
                    continue;
                };
                let is_player = matches!(actor, ActorInfo::Player { .. });
                let Some(unit_id) = f.get(2).map(|s| s.trim().to_string()) else {
                    continue;
                };
                // Pet: a UNIT_ADDED whose owner resolves to a player-side actor.
                // ownerUnitId is the field before the reaction (UNIT_ADDED tail
                // index 13 → absolute 15).
                if let Some(owner) = f.get(15).map(|s| s.trim()) {
                    if owner != "0" && !owner.is_empty() {
                        if let Some(&(owner_idx, owner_is_player)) = live.get(owner) {
                            if owner_is_player && pet_seen.insert((idx, owner_idx)) {
                                pets.push(format!("{idx}|{owner_idx}"));
                            }
                        }
                    }
                }
                live.insert(unit_id, (idx, is_player));
            }
            "UNIT_REMOVED" => {
                if let Some(u) = f.get(2) {
                    live.remove(u.trim());
                }
            }
            "COMBAT_EVENT" | "EFFECT_CHANGED" | "BEGIN_CAST" => {
                // Field positions differ between COMBAT_EVENT and the effect/cast
                // lines (which share a layout).
                let (result, ability, src, tgt) = if kind == "COMBAT_EVENT" {
                    (
                        f.get(2).map(|s| s.trim()).unwrap_or(""),
                        f.get(8).map(|s| s.trim()).unwrap_or(""),
                        f.get(9).map(|s| s.trim()).unwrap_or(""),
                        f.get(19).map(|s| s.trim()).unwrap_or("0"),
                    )
                } else {
                    (
                        "",
                        f.get(5).map(|s| s.trim()).unwrap_or(""),
                        f.get(6).map(|s| s.trim()).unwrap_or(""),
                        f.get(16).map(|s| s.trim()).unwrap_or("0"),
                    )
                };
                let Some(&ci) = ability_to_index.get(ability) else {
                    continue; // ability not in the master table → no tuple
                };
                let sa = live.get(src).map(|&(i, _)| i).unwrap_or(0);
                let self_target = tgt == "*";
                let ta = if self_target {
                    sa
                } else if tgt == "0" {
                    0
                } else {
                    live.get(tgt).map(|&(i, _)| i).unwrap_or(0)
                };
                if sa == 0 && ta == 0 {
                    continue; // both sides unknown → not registered
                }
                if kind == "COMBAT_EVENT" && !combat_event_registers(result, self_target) {
                    continue;
                }
                if tuple_seen.insert((sa, ta, ci)) {
                    tuples.push(format!("{sa}|{ta}|{ci}"));
                }
            }
            _ => {}
        }
    }
    (tuples, pets)
}

/// Whether a `COMBAT_EVENT` registers an actor-pairing tuple. The cast that never
/// landed (`QUEUED`/`TARGET_DEAD`/`CASTER_DEAD`/`ABILITY_ON_COOLDOWN`) and the
/// self-targeted control/movement results do not.
fn combat_event_registers(result: &str, self_target: bool) -> bool {
    if matches!(
        result,
        "QUEUED" | "TARGET_DEAD" | "CASTER_DEAD" | "ABILITY_ON_COOLDOWN"
    ) {
        return false;
    }
    if self_target
        && matches!(
            result,
            "STUNNED"
                | "ROOTED"
                | "FEARED"
                | "SPRINTING"
                | "REINCARNATING"
                | "BAD_TARGET"
                | "KNOCKBACK"
                | "TARGET_OUT_OF_RANGE"
        )
    {
        return false;
    }
    true
}

/// Join records with trailing newlines (each record on its own `\n`-terminated
/// line), matching the master-table section format.
fn join_lines(records: &[String]) -> String {
    let mut s = String::new();
    for r in records {
        s.push_str(r);
        s.push('\n');
    }
    s
}

/// Crate-internal access to the quote-aware CSV splitter (used by sibling modules
/// like [`super::a_counter`] that parse whole raw lines). Returns the fields as a
/// `Vec` for index access.
pub(crate) fn split_csv_quoted_pub(s: &str) -> Vec<&str> {
    split_csv_quoted(s).collect()
}

/// Split a CSV tail honoring double-quoted fields (which may contain commas).
/// Lightweight: ESO uses simple `"..."` quoting without escaped inner quotes for
/// these fields.
fn split_csv_quoted(s: &str) -> impl Iterator<Item = &str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut in_q = false;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' => in_q = !in_q,
            b',' if !in_q => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out.into_iter()
}

// ---------------------------------------------------------------------------
// COMBAT_EVENT → segment code 1 (damage events) — the per-event encoding.
//
// Cracked byte-exact from the combat golden pair (3733 strictly-unique matched
// pairs) and corroborated by an adversarial decode+verify pass. A `COMBAT_EVENT`
// whose `actionResult` is a *damage-class* result becomes a code-1 segment line:
//
//   {segTs}|1|{subordinal}|{srcMask}|{tgtMask}|C{castTrackId}|S{srcState}|T{tgtState}|{critFlag}|{final}
//
// A unit whose raw state is absent (unitId 0 / fully-zeroed) has its whole block
// omitted and its mask set to 32, giving a single-block variant.
//
// THREE parts of the line are STATEFUL — they depend on parser state accumulated
// across the WHOLE log (and across all segment codes), not on this one line:
//   * `subordinal` (the `A.B.C` id) — the leading `A` is a GLOBAL monotonic
//     emission-order counter spanning every segment code, so it CANNOT be minted
//     from code-1 data alone (the single biggest blocker to whole-line native
//     code-1; needs a cross-code capture to reproduce).
//   * `srcMask`/`tgtMask` — the two units' relative actor-table index (lower
//     index → 16, higher → 64; an absent side → 32). Needs an incrementally-built
//     actor-index map (raw unit ids are reused intra-session).
//   * each state block's `championPoints` — the unit's CURRENT value, tracked
//     from `UNIT_ADDED`/`UNIT_CHANGED`.
//
// THIS module proves the per-event ENCODING: the field layout, the crit flag, the
// final-field branch, and the state-block numeric encoding ([`encode_state_block`],
// [`combat_crit_flag`], [`combat_final_field`]) — all golden-verified.
//
// NOTE: code 1 is NOT in `coverage::PROVEN_LINE_TYPES` and must not be until the
// subordinal-`A` generator is proven on a cross-code capture. Even with every
// per-field encoder proven, the assembled line is unshippable without `A`. The
// coverage gate keeps any real combat log on the official uploader meanwhile.

/// The trailing **crit flag** (`seg[-2]`) of a code-1 line: `1` for a
/// non-critical hit, `2` for a critical hit. Derived from `actionResult`:
/// `DAMAGE → 1`, `CRITICAL_DAMAGE → 2` (proven byte-exact 3394/3394 on
/// non-shielded events).
///
/// Returns `None` for actionResults whose crit flag is NOT derivable from the
/// line alone, so the caller must route them to the official uploader:
/// * `DAMAGE_SHIELDED` — its crit flag is split 1/2 with no
///   `CRITICAL_DAMAGE_SHIELDED` result to disambiguate (needs upstream context).
/// * `DIED`/`FALL_DAMAGE` and the status results — wider/other variants not yet
///   proven.
pub fn combat_crit_flag(action_result: &str) -> Option<u8> {
    match action_result {
        "DAMAGE" => Some(1),
        "CRITICAL_DAMAGE" => Some(2),
        // Code-1 but not yet byte-reproducible (crit/absorbed/overkill fields are
        // not derivable from the single line) — gated to the official uploader.
        _ => None,
    }
}

/// The crit flag for the heal / dot / power combat codes (3 / 2 / 26), which use
/// the same `1 = normal, 2 = critical` scheme as code 1:
/// `HEAL`/`DOT_TICK`/`POWER_*` → 1, `CRITICAL_HEAL`/`DOT_TICK_CRITICAL` → 2.
/// Verified `1` across the code-2/3 captures (no critical-heal/dot sample, but the
/// scheme is the same one code 1 proved). Returns `None` for anything outside
/// these families so the caller gates it.
pub fn combat_noncode1_crit_flag(action_result: &str) -> Option<u8> {
    match action_result {
        "HEAL" | "DOT_TICK" | "POWER_ENERGIZE" | "POWER_DRAIN" => Some(1),
        "CRITICAL_HEAL" | "DOT_TICK_CRITICAL" => Some(2),
        _ => None,
    }
}

/// The final field (`seg[-1]`) of a code-1 line. It is **not** always the raw
/// hitValue — it branches on `actionResult` (verified against the full segment;
/// a verbatim hitValue would corrupt every immune/blocked/dodged hit):
/// * `DAMAGE`/`CRITICAL_DAMAGE`: the raw hitValue when `overflow == 0` (the only
///   case proven — no nonzero-overflow sample exists yet, so `overflow != 0`
///   returns `None` and stays gated).
/// * `IMMUNE → "10"`, `BLOCKED_DAMAGE → "1"`, `DODGED → "7"` — constant overrides.
///
/// `None` means "not byte-reproducible here → use the official uploader."
pub fn combat_final_field(action_result: &str, hit_value: &str, overflow: &str) -> Option<String> {
    match action_result {
        "DAMAGE" | "CRITICAL_DAMAGE" => {
            // Overflow path is unproven (0 nonzero-overflow samples) — gate it.
            if overflow.trim() != "0" {
                return None;
            }
            Some(hit_value.to_string())
        }
        "IMMUNE" => Some("10".to_string()),
        "BLOCKED_DAMAGE" => Some("1".to_string()),
        "DODGED" => Some("7".to_string()),
        _ => None,
    }
}

/// A parsed `COMBAT_EVENT` line, split into the fields the code-1 encoder needs.
/// Field layout (raw, after `<relMs>,COMBAT_EVENT,`): `actionResult, damageType,
/// powerType, hitValue, overflow, castTrackId, abilityId, <sourceUnitState>,
/// <targetUnitState>`, where `<unitState>` = `unitId, health/max, magicka/max,
/// stamina/max, ultimate/max, werewolf/max, shield, mapX, mapY, heading`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatEvent {
    pub action_result: String,
    pub cast_track_id: String,
    pub hit_value: String,
    /// Source unit id (raw).
    pub source_unit: String,
    /// The 9 source state fields *after* the unit id: `health…shield` (6) then
    /// `mapX, mapY, heading` (3). Empty if the source state was `*`.
    pub source_state: Vec<String>,
    /// Target unit id (raw).
    pub target_unit: String,
    /// The 9 target state fields after the unit id; empty if the state was `*`.
    pub target_state: Vec<String>,
}

impl CombatEvent {
    /// Parse from the comma tail after `<relMs>,COMBAT_EVENT,`. Returns `None` if
    /// the line is too short or malformed.
    pub fn parse(rest: &str) -> Option<Self> {
        let f: Vec<&str> = rest.split(',').collect();
        // Minimum: 7 header fields + at least a unit id for the source.
        if f.len() < 8 {
            return None;
        }
        let action_result = f[0].trim().to_string();
        let cast_track_id = f[5].trim().to_string();
        let hit_value = f[3].trim().to_string();

        // Source unit state starts at index 7. A unit state is either `*`
        // (collapsed, no following fields) or `unitId` + 9 stat/position fields.
        let (source_unit, source_state, next) = parse_unit_state(&f, 7);
        let (target_unit, target_state, _) = parse_unit_state(&f, next);

        Some(CombatEvent {
            action_result,
            cast_track_id,
            hit_value,
            source_unit,
            source_state,
            target_unit,
            target_state,
        })
    }
}

/// Parse a `<unitState>` starting at field index `i`. Returns
/// `(unitId, state_fields, next_index)`. A `*` at `i` means a collapsed state:
/// unit id `*`, no fields, and the next state begins at `i+1`. Otherwise the unit
/// id is at `i` and the 9 state fields are `i+1..=i+9`.
fn parse_unit_state(f: &[&str], i: usize) -> (String, Vec<String>, usize) {
    if i >= f.len() {
        return (String::new(), Vec::new(), i);
    }
    if f[i].trim() == "*" {
        return ("*".to_string(), Vec::new(), i + 1);
    }
    let unit = f[i].trim().to_string();
    let end = (i + 10).min(f.len());
    let state: Vec<String> = f[i + 1..end].iter().map(|s| s.trim().to_string()).collect();
    (unit, state, i + 10)
}

// ---------------------------------------------------------------------------
// Code-1 state-block field masks (seg[3]=srcMask, seg[4]=tgtMask) — proven
// byte-exact (3733/3733 on the combat golden pair).
//
// The mask is NOT a per-unit-type flag. The two units of a code-1 event are
// *ordered*, and the EARLIER unit gets mask 16, the LATER gets 64; a side whose
// raw unit state is absent (unitId 0) is omitted and gets mask 32. The ordering
// key is `(side, masterActorIndex)`:
//   * side: a friendly-reaction unit (PLAYER, PLAYER_ALLY, NPC_ALLY, FRIENDLY)
//     sorts BEFORE a hostile one. (Friendly = "your side" = lower.)
//   * within a side, the lower master-actor index sorts first.
// Pets/companions take their owner's `(side, index)` (via `ownerUnitId`).
//
// Both inputs are STATEFUL: the master-actor index is assigned incrementally as
// `UNIT_ADDED` lines appear (raw unit ids are reused intra-session, so a runtime
// unit id → actor mapping must be kept current), and a unit's reaction can flip
// mid-fight via `UNIT_CHANGED`. [`ActorTable`] maintains exactly this state.

/// The mask value for the unit that sorts EARLIER in a code-1 pair.
const MASK_EARLIER: &str = "16";
/// The mask value for the unit that sorts LATER.
const MASK_LATER: &str = "64";
/// The mask value for a side whose unit state is absent (block omitted).
const MASK_ABSENT: &str = "32";

/// A unit's sort key for mask ordering: `(side, master_index)` where `side` is 0
/// for friendly-reaction units and 1 for hostile. Lower tuple sorts first → 16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ActorSortKey {
    side: u8,
    master_index: u32,
}

/// Whether a raw `reaction` token denotes a friendly ("your side") unit.
fn reaction_is_friendly(reaction: &str) -> bool {
    matches!(
        reaction.trim(),
        "PLAYER" | "PLAYER_ALLY" | "NPC_ALLY" | "FRIENDLY"
    )
}

/// Incremental runtime actor table for mask ordering. Built by replaying
/// `UNIT_ADDED`/`UNIT_CHANGED` in order; answers "what is unit N's sort key right
/// now?" Master indices are assigned on first appearance of a distinct actor
/// identity (deduped like the master table), and a runtime unit id maps to the
/// actor currently occupying it (ids are reused after `UNIT_REMOVED`).
#[derive(Debug, Default)]
pub struct ActorTable {
    /// Distinct actor identity → 1-based master index (stable for the session).
    index_of: std::collections::HashMap<String, u32>,
    next_index: u32,
    /// Current runtime state per unit id: (master_index, owner_unit_id, reaction).
    units: std::collections::HashMap<String, UnitRuntime>,
    /// monsterId → next 0-based instance ordinal (the subordinal `B`/`C` source).
    /// Player-side units always get ordinal 0; each distinct *hostile* unit of a
    /// given monsterId gets the next index (so multiple copies of one monster are
    /// distinguished in the subordinal).
    monster_instance_count: std::collections::HashMap<String, u32>,
}

#[derive(Debug, Clone)]
struct UnitRuntime {
    master_index: u32,
    owner: String,
    reaction: String,
    /// The unit's subordinal ordinal: 0 for player-side units, else its 0-based
    /// per-monsterId instance index. Used to build the `seg[2]` suffix.
    ordinal: u32,
}

impl ActorTable {
    pub fn new() -> Self {
        Self {
            next_index: 1,
            ..Default::default()
        }
    }

    /// Apply a `UNIT_ADDED` line (comma tail after `<ts>,UNIT_ADDED,`). Assigns a
    /// master index on first appearance of the actor identity and binds the
    /// runtime unit id to it.
    pub fn on_unit_added(&mut self, rest: &str) {
        let f: Vec<&str> = split_csv_quoted(rest).collect();
        // Tail layout (after `<ts>,UNIT_ADDED,`): [0]unitId … [13]ownerUnitId
        // [14]reaction [15]isGroupedWithLocalPlayer — verified against the real log.
        let Some(unit_id) = f.first().map(|s| s.trim().to_string()) else {
            return;
        };
        let Some(actor) = ActorInfo::parse(rest) else {
            return;
        };
        let identity = actor.identity();
        let master_index = *self.index_of.entry(identity).or_insert_with(|| {
            let i = self.next_index;
            self.next_index += 1;
            i
        });
        let owner = f.get(13).map(|s| s.trim().to_string()).unwrap_or_default();
        let reaction = f.get(14).map(|s| s.trim().to_string()).unwrap_or_default();
        let unit_type = f.get(1).map(|s| s.trim()).unwrap_or("");
        let monster_id = f.get(4).map(|s| s.trim()).unwrap_or("0");

        // Subordinal ordinal: player-side units (a PLAYER, a "monsterId 0" unit, or
        // a unit owned by a player-side unit — e.g. a pet) are ordinal 0; every
        // other (hostile) unit gets the next 0-based instance index for its
        // monsterId, so multiple copies of one monster are distinguished.
        let owner_is_side0 = !owner.is_empty()
            && owner != "0"
            && self
                .units
                .get(&owner)
                .map(|u| u.ordinal == 0)
                .unwrap_or(false);
        let side0 = unit_type == "PLAYER" || monster_id == "0" || owner_is_side0;
        let ordinal = if side0 {
            0
        } else {
            let c = self
                .monster_instance_count
                .entry(monster_id.to_string())
                .or_insert(0);
            let v = *c;
            *c += 1;
            v
        };

        self.units.insert(
            unit_id,
            UnitRuntime {
                master_index,
                owner,
                reaction,
                ordinal,
            },
        );
    }

    /// Apply a `UNIT_CHANGED` line (comma tail). Updates the unit's reaction (and
    /// owner), keeping its master index. Layout: `[0]unitId … [8]ownerUnitId
    /// [9]reaction` (verified UNIT_CHANGED indices).
    pub fn on_unit_changed(&mut self, rest: &str) {
        let f: Vec<&str> = split_csv_quoted(rest).collect();
        let Some(unit_id) = f.first().map(|s| s.trim()) else {
            return;
        };
        if let Some(u) = self.units.get_mut(unit_id) {
            if let Some(owner) = f.get(8) {
                u.owner = owner.trim().to_string();
            }
            if let Some(reaction) = f.get(9) {
                u.reaction = reaction.trim().to_string();
            }
        }
    }

    /// The current sort key for a unit id, resolving pets to their owner. Returns
    /// `None` if the unit is unknown (e.g. an absent/0 unit) or owner resolution
    /// loops/dangles.
    pub fn sort_key(&self, unit_id: &str) -> Option<ActorSortKey> {
        self.sort_key_depth(unit_id, 0)
    }

    /// A unit's OWN side mask: [`MASK_EARLIER`] (`16`) for a friendly-reaction unit,
    /// [`MASK_LATER`] (`64`) for a hostile one. Used by the thin effect/cast codes
    /// for a self-targeted event (src == tgt), where both mask slots are the unit's
    /// own side rather than the relative earlier/later ordering of two distinct
    /// units. Resolves pets to their owner like [`Self::sort_key`]; `None` for an
    /// unknown/absent unit.
    pub fn side_mask(&self, unit_id: &str) -> Option<&'static str> {
        let key = self.sort_key(unit_id)?;
        Some(if key.side == 0 {
            MASK_EARLIER
        } else {
            MASK_LATER
        })
    }

    fn sort_key_depth(&self, unit_id: &str, depth: u8) -> Option<ActorSortKey> {
        let u = self.units.get(unit_id)?;
        if depth < 4 && !u.owner.is_empty() && u.owner != "0" {
            if let Some(k) = self.sort_key_depth(&u.owner, depth + 1) {
                return Some(k);
            }
        }
        Some(ActorSortKey {
            side: u8::from(!reaction_is_friendly(&u.reaction)),
            master_index: u.master_index,
        })
    }

    /// Compute `(srcMask, tgtMask)` for a code-1 event. `src_unit`/`tgt_unit` are
    /// the raw source/target unit ids; an absent side (unit id `"0"` / unknown)
    /// yields [`MASK_ABSENT`]. Returns `None` if both sides are present but their
    /// keys are equal (an unobserved self/co-located case — gated, not guessed).
    pub fn code1_masks(
        &self,
        src_unit: &str,
        tgt_unit: &str,
    ) -> Option<(&'static str, &'static str)> {
        let src_absent = src_unit == "0";
        let tgt_absent = tgt_unit == "0";
        match (src_absent, tgt_absent) {
            (true, true) => None, // no real state at all — not a code-1 line.
            (true, false) => Some((MASK_ABSENT, MASK_EARLIER)),
            (false, true) => Some((MASK_EARLIER, MASK_ABSENT)),
            (false, false) => {
                let sk = self.sort_key(src_unit)?;
                let tk = self.sort_key(tgt_unit)?;
                match sk.cmp(&tk) {
                    std::cmp::Ordering::Less => Some((MASK_EARLIER, MASK_LATER)),
                    std::cmp::Ordering::Greater => Some((MASK_LATER, MASK_EARLIER)),
                    // Equal keys (self-cast / co-located) never appear in the
                    // golden code-1 set — gate rather than guess.
                    std::cmp::Ordering::Equal => None,
                }
            }
        }
    }

    /// A unit's subordinal ordinal (0 for player-side, else its per-monsterId
    /// instance index). An absent/unknown unit is treated as ordinal 0.
    fn ordinal(&self, unit_id: &str) -> u32 {
        self.units.get(unit_id).map(|u| u.ordinal).unwrap_or(0)
    }

    /// The 1-based master actor index currently bound to a runtime unit id, or
    /// `None` if the id is unknown. This is the same index the master table's
    /// player role encodes (`1000000 + index`) and the `PLAYER_INFO` (code 44)
    /// line emits as its unit reference.
    pub fn master_index_of(&self, unit_id: &str) -> Option<u32> {
        self.units.get(unit_id.trim()).map(|u| u.master_index)
    }

    /// Build the code-1 subordinal string `seg[2]` GIVEN its leading allocation
    /// number `a`. Form: `A.srcOrd.tgtOrd` with trailing-zero components stripped
    /// (the leading `A` is always kept): `A` when both ordinals are 0, `A.B` when
    /// only the target's is 0, else `A.0.C`/`A.B.C`. Proven byte-exact 3733/3733
    /// (with the true `a`).
    ///
    /// `a` itself is NOT mintable from this capture — it is a global cross-code
    /// allocation counter — so this helper takes it as input. It is exercised and
    /// proven in isolation (feeding the golden `a`) so it is ready the moment `a`
    /// is solved; until then whole-line code-1 stays gated.
    pub fn code1_subordinal(&self, a: &str, src_unit: &str, tgt_unit: &str) -> String {
        let src_ord = self.ordinal(src_unit);
        let tgt_ord = self.ordinal(tgt_unit);
        let mut comps = vec![a.to_string(), src_ord.to_string(), tgt_ord.to_string()];
        while comps.len() > 1 && comps.last().map(|s| s == "0").unwrap_or(false) {
            comps.pop();
        }
        comps.join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_basename_strips_path_and_ext() {
        assert_eq!(
            icon_basename("/esoui/art/icons/ability_rogue_035.dds"),
            "ability_rogue_035"
        );
        assert_eq!(
            icon_basename("\"/esoui/art/icons/gear_argonian_heavy_hands_b.dds\""),
            "gear_argonian_heavy_hands_b"
        );
    }

    #[test]
    fn ability_flags_match_golden_cases() {
        // From the golden master table:
        assert_eq!(ability_flags(false, true), 1); // F,T → 1 (Roll Dodge)
        assert_eq!(ability_flags(true, true), 3); // T,T → 3 (Dodge Fatigue)
        assert_eq!(ability_flags(false, false), 0); // F,F → 0
        assert_eq!(ability_flags(true, false), 2); // T,F → 2
    }

    // Byte-exact against EVERY ability record in the real golden master table.
    #[test]
    fn ability_records_match_golden_master_table() {
        let raw = include_str!("testdata/sample_raw_encounter.log");
        let master = include_str!("testdata/sample_master_table.txt");

        // Expected ability records = master lines 5..=13 (the 9 abilities), in
        // order. Pull them straight from the golden file.
        let master_lines: Vec<&str> = master.lines().collect();
        // Line 4 (0-based index 3) is the count "9"; records are the next 9.
        let count: usize = master_lines[3].trim().parse().unwrap();
        let expected: Vec<&str> = master_lines[4..4 + count].to_vec();

        // Build our ability records: first ABILITY_INFO per ability id, in order
        // of first appearance across the whole log.
        let mut seen = std::collections::BTreeSet::new();
        let mut ours = Vec::new();
        for line in raw.lines() {
            let mut it = line.splitn(3, ',');
            let _ts = it.next();
            if it.next().map(str::trim) != Some("ABILITY_INFO") {
                continue;
            }
            let rest = it.next().unwrap_or("");
            let info = AbilityInfo::parse(rest).expect("parse ABILITY_INFO");
            if seen.insert(info.ability_id) {
                ours.push(info.to_master_record());
            }
        }

        assert_eq!(
            ours.len(),
            expected.len(),
            "ability count must match the golden master table"
        );
        for (got, want) in ours.iter().zip(expected.iter()) {
            assert_eq!(got, want, "ability record must be byte-exact");
        }
    }

    // The timestamp offset reproduces both sessions' observed segment timestamps.
    #[test]
    fn session_offset_matches_golden_timestamps() {
        // Session 1: BEGIN_LOG ts=4, wall=1780641553946 (first). offset = -4.
        let off1 = session_offset(1780641553946, 1780641553946, 4);
        assert_eq!(off1, -4);
        assert_eq!(segment_ts(87973, off1), 87969); // raw 87973 → 87969 (verified)

        // Session 2: BEGIN_LOG ts=3, wall=1780725570055. offset = 84016105.
        let off2 = session_offset(1780725570055, 1780641553946, 4);
        assert_eq!(off2, 84016105);
        assert_eq!(segment_ts(141831, off2), 84157936); // verified
        assert_eq!(segment_ts(236498, off2), 84252603); // verified
    }

    // ZONE_CHANGED and MAP_CHANGED encode byte-exact against the golden segment.
    #[test]
    fn zone_and_map_match_golden_segment() {
        // Golden segment line 3 (S1 ZONE): "0|41|1129|Hall of the Lunar Champion|0"
        // raw line 2: 4,ZONE_CHANGED,1129,"Hall of the Lunar Champion",NONE; ts 4→0
        let zone = encode_zone_changed(0, "1129,\"Hall of the Lunar Champion\",NONE").unwrap();
        assert_eq!(zone, "0|41|1129|Hall of the Lunar Champion|0");

        // Golden segment line 4 (S1 MAP): "87969|51|1576|Rimmen|elsweyr/rimmen_base"
        let map = encode_map_changed(87969, "1576,\"Rimmen\",\"elsweyr/rimmen_base\"").unwrap();
        assert_eq!(map, "87969|51|1576|Rimmen|elsweyr/rimmen_base");
    }

    // The tuple section reproduces the golden master table byte-exact. Tuples
    // are derived purely from the abilities (index + EFFECT_INFO presence), so
    // this is fully provable from the current sample.
    #[test]
    fn tuple_records_match_golden_master_table() {
        let raw = include_str!("testdata/sample_raw_encounter.log");
        let master = include_str!("testdata/sample_master_table.txt");
        let lines: Vec<&str> = master.lines().collect();

        // Layout: [0]=header, [1]=actorCount, actors…, then abilityCount, abilities…,
        // then tupleCount, tuples…, then petCount. Locate the tuple block: it
        // follows the ability block. abilityCount is at index 3 (1 actor).
        let ability_count: usize = lines[3].trim().parse().unwrap();
        let tuple_count_idx = 4 + ability_count; // line holding the tuple count
        let tuple_count: usize = lines[tuple_count_idx].trim().parse().unwrap();
        let expected: Vec<&str> =
            lines[tuple_count_idx + 1..tuple_count_idx + 1 + tuple_count].to_vec();

        // Which ability ids have an EFFECT_INFO line anywhere in the log.
        let mut has_effect: std::collections::BTreeSet<&str> = Default::default();
        for l in raw.lines() {
            let mut it = l.splitn(3, ',');
            let _ts = it.next();
            if it.next().map(str::trim) == Some("EFFECT_INFO") {
                if let Some(id) = it.next().and_then(|r| r.split(',').next()) {
                    has_effect.insert(id.trim());
                }
            }
        }

        // Abilities in first-appearance order → tuple records.
        let mut seen = std::collections::BTreeSet::new();
        let mut ours = Vec::new();
        let mut index = 0usize;
        for l in raw.lines() {
            let mut it = l.splitn(3, ',');
            let _ts = it.next();
            if it.next().map(str::trim) != Some("ABILITY_INFO") {
                continue;
            }
            let rest = it.next().unwrap_or("");
            let info = AbilityInfo::parse(rest).expect("parse ABILITY_INFO");
            let id = rest.split(',').next().unwrap_or("").trim();
            if seen.insert(id.to_string()) {
                index += 1;
                ours.push(tuple_record(index, info.f6, has_effect.contains(id)));
            }
        }

        assert_eq!(ours.len(), expected.len(), "tuple count must match");
        for (got, want) in ours.iter().zip(expected.iter()) {
            assert_eq!(got, want, "tuple record must be byte-exact");
        }
    }

    // State-block position encoding, proven byte-exact against the golden code-16
    // event. Key gotchas: Y is flipped (10000 - floor(y*10000)) and heading uses
    // floor (439 from 4.3962).
    #[test]
    fn state_block_position_encoding_matches_golden() {
        // raw line 6 stat tail (fields after the cast's `n` marker):
        // 16000/16000,12000/12000,7960/12000,53/500,0/1000,0,0.5077,-0.0888,4.3962,*
        let fields = vec![
            "16000/16000",
            "12000/12000",
            "7960/12000",
            "53/500",
            "0/1000",
            "0",
            "0.5077",
            "-0.0888",
            "4.3962",
            "*",
        ];
        // championPoints constant 1735 for this actor in the sample.
        let block = encode_state_block(&fields, "1735").unwrap();
        // Golden S block (without the leading S tag):
        let expected = "16000/16000|12000/12000|7960/12000|53/500|0/1000|0|1735|5077|10888|439";
        assert_eq!(
            block, expected,
            "state block must match the golden code-16 event"
        );
    }

    #[test]
    fn position_encoders_match_golden_rules() {
        // X: floor(x*10000). The float multiply reproduces the official encoder's
        // f64 representation, so 0.4095 → 4094 (not 4095) — verified on the combat
        // golden pair.
        assert_eq!(encode_pos_x(0.5077), 5077);
        assert_eq!(encode_pos_x(0.4095), 4094);
        // Y: 10000 - floor(y*10000) (flipped axis). -0.0888 → 10888; 0.5467 → 4533.
        assert_eq!(encode_pos_y(-0.0888), 10888);
        assert_eq!(encode_pos_y(0.5467), 4533);
        // Heading: floor(h*100). floor (not trunc) matters for negatives:
        // 4.3962 → 439, but -2.4237 → -243 (trunc would give -242).
        assert_eq!(encode_pos_heading(4.3962), 439);
        assert_eq!(encode_pos_heading(-2.4237), -243);
    }

    // THE MILESTONE: the COMPLETE master table, assembled end-to-end from the raw
    // log, reproduces the captured payload byte-for-byte. Header + actors +
    // abilities + tuples + pets, all sections, exact.
    #[test]
    fn full_master_table_matches_golden_byte_for_byte() {
        let raw = include_str!("testdata/sample_raw_encounter.log");
        let expected = include_str!("testdata/sample_master_table.txt");
        let lines: Vec<&str> = raw.lines().collect();
        let built = build_master_table(&lines).expect("build master table");
        // The captured file may or may not have a trailing newline; compare on
        // trimmed-trailing-newline to focus on content equality.
        assert_eq!(
            built.trim_end_matches('\n'),
            expected.trim_end_matches('\n'),
            "the full master table must reproduce the captured payload exactly"
        );
    }

    // The actor record reproduces the golden master table's actor line exactly.
    #[test]
    fn actor_record_matches_golden_master_table() {
        let raw = include_str!("testdata/sample_raw_encounter.log");
        let master = include_str!("testdata/sample_master_table.txt");
        // Master line 3 (index 2) is the single actor record.
        let expected = master.lines().nth(2).unwrap();
        let server = "\"NA Megaserver\"";

        // First UNIT_ADDED (player) in the log → actor index 1.
        let actor_line = raw
            .lines()
            .find(|l| l.split(',').nth(1) == Some("UNIT_ADDED"))
            .unwrap();
        let rest = actor_line.splitn(3, ',').nth(2).unwrap();
        let actor = ActorInfo::parse(rest).expect("parse UNIT_ADDED");
        // sample BEGIN_LOG wall = 1780641553946 (named player → unused for id).
        assert_eq!(
            actor.to_master_record(1, server, 1780641553946),
            expected,
            "actor record must be byte-exact vs the golden master table"
        );
    }

    // Multi-actor master tables: chunk1 has 11 players (local + remote + 2
    // anonymized). This proves the index-aware role, the is-local name flag, the
    // anon-player id (BEGIN_LOG wall + reg offset), and cross-session dedup —
    // byte-exact against a captured 11-player roster.
    //
    // NOTE: only the PLAYER rows are asserted byte-exact. Monster rows are not yet
    // fully reproducible: their `icon_basename` (e.g. `death_recap_melee_basic`)
    // is derived from the monster's later attack/damage type, NOT present in its
    // `UNIT_ADDED` line — it requires correlating combat events. Until that's
    // encoded + proven, monster-bearing logs stay on the official uploader via the
    // coverage gate. This test pins the player block, the part that IS proven.
    #[test]
    fn chunk1_player_block_matches_golden() {
        let raw = include_str!("testdata/chunk1_raw.log");
        let master = include_str!("testdata/chunk1_master.txt");
        let lines: Vec<&str> = master.lines().collect();
        // The 11 players are the first 11 actor rows (players precede monsters in
        // this roster). Assert exactly those.
        let player_rows = 11;
        let expected: Vec<&str> = lines[2..2 + player_rows].to_vec();

        // Build just the actor block via the same path build_master_table uses.
        let server = "\"NA Megaserver\"";
        // chunk1 BEGIN_LOG wall (for anon player id synthesis).
        let begin_wall: u64 = 1750388541962;
        let raw_lines: Vec<&str> = raw.lines().collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut ours: Vec<String> = Vec::new();
        for l in &raw_lines {
            let mut it = l.splitn(3, ',');
            let _ts = it.next();
            if it.next().map(str::trim) != Some("UNIT_ADDED") {
                continue;
            }
            let rest = it.next().unwrap_or("");
            if let Some(actor) = ActorInfo::parse(rest) {
                // Only players are asserted here (see test note on monsters).
                if !matches!(actor, ActorInfo::Player { .. }) {
                    continue;
                }
                if seen.insert(actor.identity()) {
                    let idx = ours.len() + 1;
                    ours.push(actor.to_master_record(idx, server, begin_wall));
                }
            }
        }

        // Compare the 11 player rows byte-for-byte.
        for (i, want) in expected.iter().enumerate() {
            let got = ours.get(i).map(String::as_str).unwrap_or("<missing>");
            assert_eq!(got, *want, "player row {} must be byte-exact", i + 1);
        }
        assert_eq!(ours.len(), expected.len(), "player count must match");
    }

    #[test]
    fn combat_crit_flag_maps_damage_results() {
        assert_eq!(combat_crit_flag("DAMAGE"), Some(1));
        assert_eq!(combat_crit_flag("CRITICAL_DAMAGE"), Some(2));
        // Non-derivable / non-code-1 results → None, so the caller routes them to
        // the official uploader rather than guessing.
        assert_eq!(combat_crit_flag("HEAL"), None);
        assert_eq!(combat_crit_flag("POWER_ENERGIZE"), None);
        assert_eq!(combat_crit_flag("DAMAGE_SHIELDED"), None);
        assert_eq!(combat_crit_flag("DIED"), None);
    }

    #[test]
    fn combat_final_field_branches_on_action_result() {
        // DAMAGE/CRIT with no overflow → the hit value verbatim.
        assert_eq!(
            combat_final_field("DAMAGE", "1657", "0").as_deref(),
            Some("1657")
        );
        assert_eq!(
            combat_final_field("CRITICAL_DAMAGE", "3506", "0").as_deref(),
            Some("3506")
        );
        // Status results have CONSTANT final fields — a verbatim hitValue would
        // corrupt them.
        assert_eq!(
            combat_final_field("IMMUNE", "0", "0").as_deref(),
            Some("10")
        );
        assert_eq!(
            combat_final_field("BLOCKED_DAMAGE", "0", "0").as_deref(),
            Some("1")
        );
        assert_eq!(combat_final_field("DODGED", "0", "0").as_deref(), Some("7"));
        // Overflow path is unproven → gated (None) so it can't ship wrong.
        assert_eq!(combat_final_field("DAMAGE", "1657", "55"), None);
        // Unhandled results → None (use official uploader).
        assert_eq!(combat_final_field("HEAL", "100", "0"), None);
    }

    #[test]
    fn combat_event_parses_source_and_target_state() {
        // raw tail after "43,COMBAT_EVENT,":
        let rest = "DAMAGE,MAGIC,1,1657,0,5177180,217784,\
                    6,44582/45632,14125/22216,24687/24687,243/500,1000/1000,0,0.3985,0.5467,3.7322,\
                    31,44757/49116,0/0,0/0,0/0,0/0,0,0.4047,0.5534,0.7109";
        let ev = CombatEvent::parse(rest).unwrap();
        assert_eq!(ev.action_result, "DAMAGE");
        assert_eq!(ev.cast_track_id, "5177180");
        assert_eq!(ev.hit_value, "1657");
        assert_eq!(ev.source_unit, "6");
        assert_eq!(ev.target_unit, "31");
        // 9 state fields each (health…shield, mapX, mapY, heading).
        assert_eq!(ev.source_state.len(), 9);
        assert_eq!(ev.target_state.len(), 9);
        assert_eq!(ev.source_state[0], "44582/45632");
        assert_eq!(ev.source_state[8], "3.7322");
    }

    #[test]
    fn combat_event_handles_collapsed_state() {
        // A `*` source state collapses with no following fields; the target then
        // follows immediately. (POWER_ENERGIZE example shape — not code 1, but the
        // parser must still split it correctly.)
        let rest = "POWER_ENERGIZE,GENERIC,1,600,0,5177180,216942,\
                    6,44582/45632,14725/22216,24687/24687,243/500,1000/1000,0,0.3985,0.5467,3.7322,*";
        let ev = CombatEvent::parse(rest).unwrap();
        assert_eq!(ev.source_unit, "6");
        assert_eq!(ev.source_state.len(), 9);
        assert_eq!(ev.target_unit, "*");
        assert!(ev.target_state.is_empty());
    }

    // THE per-event milestone: for every row in the combat golden fixture, parse
    // the raw COMBAT_EVENT and rebuild the code-1 line's PROVEN parts — the C
    // cast-track field, both encoded state blocks, the result code, and the hit
    // value — then assert they are byte-identical to the captured segment line.
    //
    // The two *stateful* inputs (the `A.B.C` sub-ordinal and the src/tgt masks)
    // and each block's current championPoints are taken from the golden line here:
    // those are produced by the parser's whole-log stateful pass (decoded
    // separately), so this test isolates and proves the per-event ENCODING, which
    // is the byte-exact core. Covers 22 diverse rows: every mask combo, negative
    // headings (floor vs trunc), the championPoints-increment cases, and the
    // float-representation edge (0.4095 → 4094).
    #[test]
    fn code1_event_encoding_matches_golden() {
        let fixture = include_str!("testdata/code1_event_golden.tsv");
        let mut checked = 0;
        for line in fixture.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let (raw, seg) = line.split_once('\t').expect("fixture row is raw\\tseg");
            let seg_fields: Vec<&str> = seg.split('|').collect();
            // Only the proven 28-field DAMAGE/CRITICAL_DAMAGE variant.
            assert_eq!(seg_fields.len(), 28, "fixture must be the 28-field variant");

            // Parse the raw COMBAT_EVENT (drop the leading "<ts>,COMBAT_EVENT,").
            let rest = raw.splitn(3, ',').nth(2).unwrap();
            let ev = CombatEvent::parse(rest).expect("parse COMBAT_EVENT");

            // Stateful inputs read from the golden line (proven elsewhere):
            //   seg[2] = sub-ordinal, seg[3]/seg[4] = masks,
            //   each state block's 7th field = that unit's current championPoints.
            let subordinal = seg_fields[2];
            let src_mask = seg_fields[3];
            let tgt_mask = seg_fields[4];
            // Locate S and T blocks to read their championPoints field.
            let si = seg_fields.iter().position(|f| f.starts_with('S')).unwrap();
            let ti = seg_fields.iter().position(|f| f.starts_with('T')).unwrap();
            let src_cp = seg_fields[si + 6];
            let tgt_cp = seg_fields[ti + 6];

            // Build the proven parts.
            let src_state: Vec<&str> = ev.source_state.iter().map(String::as_str).collect();
            let tgt_state: Vec<&str> = ev.target_state.iter().map(String::as_str).collect();
            let s_block = encode_state_block(&src_state, src_cp).unwrap();
            let t_block = encode_state_block(&tgt_state, tgt_cp).unwrap();
            let crit_flag = combat_crit_flag(&ev.action_result).unwrap();
            // overflow is raw field index 4 (after the 7-field header it's [6];
            // the fixture is all overflow==0, so the final field is the hit value).
            let final_field = combat_final_field(&ev.action_result, &ev.hit_value, "0").unwrap();

            // Assemble the full code-1 line (segTs comes from the timestamp
            // transform, proven separately; read it from the golden line).
            let seg_ts = seg_fields[0];
            let ours = format!(
                "{seg_ts}|1|{subordinal}|{src_mask}|{tgt_mask}|C{ctid}|S{s_block}|T{t_block}|{crit_flag}|{final_field}",
                ctid = ev.cast_track_id,
            );
            assert_eq!(ours, seg, "code-1 line must be byte-exact");
            checked += 1;
        }
        assert!(
            checked >= 20,
            "fixture should cover a diverse set (got {checked})"
        );
    }

    // The championPoints rule: the state block's championPoints field is the
    // unit's CURRENT value — initialized from UNIT_ADDED and updated by
    // UNIT_CHANGED mid-session. This pins the rule with the two real
    // increment cases from the combat log (unit 1: 1740→1741, unit 5: 2142→2143).
    #[test]
    fn champion_points_tracks_current_value() {
        // Before its UNIT_CHANGED, unit 1's CP is 1740; after, 1741. The encoder
        // must use whichever is current at the event's timestamp. This is enforced
        // by build_combat_segment's per-unit tracking; here we assert the rule's
        // intent via the state-block encoder using each value.
        let state = vec![
            "18400/18400",
            "9971/12868",
            "8488/28700",
            "198/500",
            "0/1000",
            "0",
            "0.5760",
            "0.7050",
            "5.9698",
        ];
        let before = encode_state_block(&state, "1740").unwrap();
        let after = encode_state_block(&state, "1741").unwrap();
        assert!(before.contains("|1740|"));
        assert!(after.contains("|1741|"));
        assert_ne!(
            before, after,
            "current championPoints must flow into the block"
        );
    }

    // The code-1 mask rule, proven byte-exact 3733/3733 on the combat golden pair.
    // This test exercises every branch of the ordering with synthetic UNIT_ADDED
    // lines (so the master indices are fully controlled): player→monster,
    // monster→player, a FRIENDLY NPC vs a HOSTILE one (reaction beats index), a pet
    // inheriting its owner's key, an absent side (→32), and a mid-fight reaction
    // flip via UNIT_CHANGED. The synthetic lines use the verified UNIT_ADDED field
    // layout (`unitId,TYPE,isLocal,perSession,monsterId,isBoss,class,race,name,
    // account,charId,level,CP,owner,reaction,grouped`).
    #[test]
    fn code1_masks_order_by_side_then_index() {
        let mut t = ActorTable::new();
        // index 1: the logging player (friendly).
        t.on_unit_added("1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1700,0,PLAYER_ALLY,T");
        // index 2: a hostile monster.
        t.on_unit_added("30,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F");
        // index 3: a FRIENDLY NPC (sorts before any hostile despite a higher index).
        t.on_unit_added("40,MONSTER,F,0,90653,F,0,0,\"Selene\",\"\",0,50,160,0,FRIENDLY,F");
        // index 4: a player's pet, owned by unit 1.
        t.on_unit_added("25,MONSTER,F,0,32695,F,0,0,\"Familiar\",\"\",0,50,160,1,NPC_ALLY,F");

        // player (friendly idx1) attacks bear (hostile idx2): src earlier → 16|64.
        assert_eq!(t.code1_masks("1", "30"), Some(("16", "64")));
        // bear attacks player: src later → 64|16.
        assert_eq!(t.code1_masks("30", "1"), Some(("64", "16")));
        // FRIENDLY Selene (idx3) vs HOSTILE bear (idx2): friendly side wins despite
        // the higher index → Selene earlier → 16|64. (This is the case a pure
        // index comparator gets wrong.)
        assert_eq!(t.code1_masks("40", "30"), Some(("16", "64")));
        // Pet (owned by player 1) attacks bear: pet inherits owner's friendly key
        // (idx1) → earlier → 16|64.
        assert_eq!(t.code1_masks("25", "30"), Some(("16", "64")));
        // Absent source (unit 0) → 32 on that side, the present side → 16.
        assert_eq!(t.code1_masks("0", "30"), Some(("32", "16")));
        assert_eq!(t.code1_masks("30", "0"), Some(("16", "32")));

        // A reaction flip: Selene turns HOSTILE mid-fight (UNIT_CHANGED layout
        // `unitId,class,race,name,account,charId,level,CP,owner,reaction,...`).
        t.on_unit_changed("40,0,0,\"Selene\",\"\",0,50,160,0,HOSTILE,F");
        // Now Selene (hostile idx3) vs bear (hostile idx2): same side → lower index
        // first → bear earlier → Selene later. Selene as src → 64|16.
        assert_eq!(t.code1_masks("40", "30"), Some(("64", "16")));
    }

    // Unit-id reuse: a unit id freed and re-added is a DIFFERENT actor and gets a
    // fresh master index, but a repeat of the SAME identity reuses its index.
    #[test]
    fn actor_table_handles_id_reuse_and_identity_dedup() {
        let mut t = ActorTable::new();
        t.on_unit_added("1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1700,0,PLAYER_ALLY,T");
        let k1 = t.sort_key("1").unwrap();
        // Same identity re-added on a different runtime unit id keeps master index.
        t.on_unit_added("7,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1700,0,PLAYER_ALLY,T");
        assert_eq!(t.sort_key("7").unwrap(), k1, "same identity → same index");
        // A different monster on a reused id gets its own (higher) index.
        t.on_unit_added("1,MONSTER,F,0,99,F,0,0,\"Imp\",\"\",0,50,160,0,HOSTILE,F");
        let reused = t.sort_key("1").unwrap();
        assert_ne!(reused, k1, "reused unit id now points at a different actor");
    }

    // The code-1 subordinal SUFFIX generator (B/C ordinals + arity), proven
    // byte-exact 3733/3733 on the combat golden pair. Player-side units (PLAYER,
    // monsterId 0, or a pet of a player-side unit) get ordinal 0; each distinct
    // hostile unit of a monsterId gets the next 0-based instance index. The
    // subordinal is `A.srcOrd.tgtOrd` with trailing zeros stripped. `A` is fed in
    // (it is the unsolved global counter); this isolates and proves the suffix.
    #[test]
    fn code1_subordinal_suffix_arity_and_ordinals() {
        let mut t = ActorTable::new();
        // player (ordinal 0), two instances of monsterId 88330, one of 88331, a pet.
        t.on_unit_added("1,PLAYER,T,1,0,F,1,3,\"Hero\",\"@hero\",111,50,1700,0,PLAYER_ALLY,T");
        t.on_unit_added("30,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F");
        t.on_unit_added("31,MONSTER,F,0,88330,F,0,0,\"Bear\",\"\",0,50,160,0,HOSTILE,F");
        t.on_unit_added("32,MONSTER,F,0,88331,F,0,0,\"Lion\",\"\",0,50,160,0,HOSTILE,F");
        t.on_unit_added("25,MONSTER,F,0,32695,F,0,0,\"Familiar\",\"\",0,50,160,1,NPC_ALLY,F");

        // Per-monsterId 0-based instance ordinals.
        assert_eq!(t.ordinal("1"), 0, "player → 0");
        assert_eq!(t.ordinal("30"), 0, "first 88330 → 0");
        assert_eq!(t.ordinal("31"), 1, "second 88330 → 1");
        assert_eq!(t.ordinal("32"), 0, "first 88331 → 0");
        assert_eq!(t.ordinal("25"), 0, "pet of a player-side unit → 0");

        // Arity truth table (A fed as "7"):
        // both player-side → just "A"
        assert_eq!(t.code1_subordinal("7", "1", "25"), "7");
        // src player-side, tgt = second bear (ord 1) → "A.0.C"
        assert_eq!(t.code1_subordinal("7", "1", "31"), "7.0.1");
        // src player-side, tgt = first bear (ord 0) → "A"
        assert_eq!(t.code1_subordinal("7", "1", "30"), "7");
        // src = second bear (ord 1), tgt player-side → "A.B"
        assert_eq!(t.code1_subordinal("7", "31", "1"), "7.1");
        // src = second bear (ord 1), tgt = lion (ord 0) → "A.B" (trailing 0 stripped)
        assert_eq!(t.code1_subordinal("7", "31", "32"), "7.1");
        // src = first bear (ord 0), tgt = second bear (ord 1) → "A.0.C"
        assert_eq!(t.code1_subordinal("7", "30", "31"), "7.0.1");
    }

    // Real-data cross-check: replaying chunk1's UNIT_ADDED stream through ActorTable
    // must assign the SAME 1-based master indices the captured master table uses.
    // The master table encodes each player's index in its role field (1000000 +
    // index), so we can verify ActorTable's incremental assignment against ground
    // truth — the same index that drives the mask ordering.
    #[test]
    fn actor_table_indices_match_chunk1_master_roles() {
        let raw = include_str!("testdata/chunk1_raw.log");
        let master = include_str!("testdata/chunk1_master.txt");

        // Build the ActorTable from the raw UNIT_ADDED/UNIT_CHANGED stream.
        let mut t = ActorTable::new();
        // Track the first runtime unit id seen for each master index so we can ask
        // its sort key back. (Players are unit id 1 in their own UNIT_ADDED... no —
        // each player has a distinct unit id; capture them in order.)
        let mut first_unit_for_index: Vec<String> = Vec::new();
        for l in raw.lines() {
            let mut it = l.splitn(3, ',');
            let _ts = it.next();
            match it.next().map(str::trim) {
                Some("UNIT_ADDED") => {
                    let rest = it.next().unwrap_or("");
                    let before = t.next_index;
                    t.on_unit_added(rest);
                    // If a new index was just assigned, remember this unit id.
                    if t.next_index > before {
                        let uid = rest.split(',').next().unwrap_or("").trim().to_string();
                        first_unit_for_index.push(uid);
                    }
                }
                Some("UNIT_CHANGED") => t.on_unit_changed(it.next().unwrap_or("")),
                _ => {}
            }
        }

        // The 11 player rows carry role = 1000000 + master index. Verify the first
        // 11 indices ActorTable assigned line up 1..=11 (players precede monsters in
        // this roster, matching build_master_table's ordering).
        let player_rows: Vec<&str> = master.lines().skip(2).take(11).collect();
        for (i, row) in player_rows.iter().enumerate() {
            let role: u32 = row.split('|').nth(2).unwrap().parse().unwrap();
            assert_eq!(
                role,
                1_000_000 + (i as u32 + 1),
                "sanity: master row {} role encodes index {}",
                i + 1,
                i + 1
            );
            // ActorTable assigned this index to the i-th distinct actor; confirm the
            // unit it bound resolves to that 1-based index.
            let uid = &first_unit_for_index[i];
            assert_eq!(
                t.sort_key(uid).unwrap().master_index,
                i as u32 + 1,
                "ActorTable index for player row {} must match the master table",
                i + 1
            );
        }
    }
}
