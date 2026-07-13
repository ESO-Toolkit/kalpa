//! Sidecar build evidence recovered from native raw logs.
//!
//! This does not participate in ESO Logs' upload payload. It preserves exact
//! client-side `PLAYER_INFO` build facts that the public report API can lose or
//! omit, then passes a small versioned JSON payload to ESO Log Aggregator.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use crate::uploader::types::{
    KalpaBuildEvidence, KalpaFoodEvidence, KalpaPlayerBuildEvidence, KalpaScribedSkillEvidence,
};

use super::encode::split_csv_quoted_pub;

const SCHEMA_VERSION: u8 = 1;
// Bumped 2 -> 3: scribed-skill evidence now carries the equipped Focus/Signature/Affix
// scripts (recovered from the raw `ABILITY_INFO` line, which ESO Logs strips from its
// API). Wire-compatible additive change — schemaVersion stays 1.
const EXTRACTOR_VERSION: u16 = 3;
const SOURCE: &str = "kalpa-native-player-info";
const CLASS_MASTERY_MAX_PICKS: usize = 2;
const MAX_SCRIBED_SKILLS: usize = 12;
const MAX_CHAMPION_POINT_PASSIVES: usize = 12;

const CHAMPION_POINT_PASSIVE_IDS: &[u32] = &[
    5857, 30923, 38963, 45546, 63663, 63880, 64079, 92134, 141899, 141942, 141991, 141993, 141997,
    141999, 142007, 142034, 142079, 142092, 142094, 142121, 142207, 142210, 142224, 142230, 142231,
    147889, 151748, 151749, 156008, 156017, 160057,
];

const FOOD_ABILITY_IDS: &[u32] = &[
    17407, 17577, 61218, 61255, 61257, 61259, 61260, 61261, 61264, 61278, 61294, 61298, 61314,
    61322, 61325, 61326, 61328, 66125, 66128, 66130, 66132, 66137, 66551, 66568, 66576, 66586,
    66590, 66594, 68411, 68412, 72824, 72957, 72960, 72962, 73553, 84678, 84711, 84720, 84723,
    84731, 84732, 84733, 84734, 85485, 85486, 86669, 86673, 89919, 89939, 89953, 89954, 89955,
    89956, 89957, 89958, 89959, 89971, 89972, 89973, 91368, 91369, 93376, 100487, 100498, 100499,
    107789, 107793, 127595, 127596, 146563, 146725, 153013, 158543, 158548, 158549, 160169, 160170,
    160171, 160172, 160174, 160175, 160176, 160312, 160494, 161213, 161215,
];

#[derive(Debug, Default)]
struct PlayerBuilder {
    unit_id: String,
    unit_occurrence_id: String,
    character_name: Option<String>,
    account_name: Option<String>,
    character_id: Option<String>,
    class_id: Option<u16>,
    race_id: Option<u16>,
    level: Option<u16>,
    champion_points: Option<u32>,
    class_name_from_unit: Option<String>,
    class_name_from_mastery: Option<String>,
    class_mastery_passives: Vec<u32>,
    champion_point_passives: Vec<u32>,
    passive_ability_ids: Vec<u32>,
    slotted_skill_ids: Vec<u32>,
    /// This unit's scribing scripts, keyed by the scribed ability id. Persistent across
    /// the unit's repeated `PLAYER_INFO` snapshots: a scribed ability id is shared across
    /// players (only the Focus changes it), so scripts MUST be tracked per unit rather
    /// than in the global ability map. Filled from `pending_scribing` at each `PLAYER_INFO`.
    scribed_scripts: BTreeMap<u32, ScribedScripts>,
    saw_player_info: bool,
}

#[derive(Debug, Clone)]
struct AbilityEvidenceInfo {
    ability_id: u32,
    name: Option<String>,
    icon: Option<String>,
}

/// The three equipped scripts of a scribed ability, read verbatim from the extra
/// fields of a scribed `ABILITY_INFO` line (focus, signature, affix in that order).
#[derive(Debug, Clone, Default)]
struct ScribedScripts {
    focus: Option<String>,
    signature: Option<String>,
    affix: Option<String>,
}

#[derive(Debug, Default)]
struct PlayerAccumulator {
    players: BTreeMap<String, PlayerBuilder>,
    active_key_by_unit: BTreeMap<String, String>,
    next_reuse_index_by_unit: BTreeMap<String, usize>,
}

impl PlayerAccumulator {
    fn active_player_mut(&mut self, unit_id: &str) -> &mut PlayerBuilder {
        let key = self.active_key_for_unit(unit_id);
        self.players
            .entry(key.clone())
            .or_insert_with(|| PlayerBuilder {
                unit_id: unit_id.to_string(),
                unit_occurrence_id: key,
                ..PlayerBuilder::default()
            })
    }

    fn active_player_for_unit_added_mut(
        &mut self,
        unit_id: &str,
        identity: &PlayerIdentity,
        facts: &PlayerUnitFacts,
    ) -> &mut PlayerBuilder {
        let should_split = self
            .active_key_by_unit
            .get(unit_id)
            .and_then(|key| self.players.get(key))
            .is_some_and(|player| {
                player_identity_conflicts(player, identity)
                    || player_unit_facts_conflict(player, identity, facts)
            });

        if should_split {
            let key = self.next_player_key(unit_id);
            self.active_key_by_unit
                .insert(unit_id.to_string(), key.clone());
            self.players
                .entry(key.clone())
                .or_insert_with(|| PlayerBuilder {
                    unit_id: unit_id.to_string(),
                    unit_occurrence_id: key,
                    ..PlayerBuilder::default()
                });
        }

        self.active_player_mut(unit_id)
    }

    fn into_players(self) -> BTreeMap<String, PlayerBuilder> {
        self.players
    }

    fn active_key_for_unit(&mut self, unit_id: &str) -> String {
        if let Some(key) = self.active_key_by_unit.get(unit_id) {
            return key.clone();
        }

        let key = self.next_player_key(unit_id);
        self.active_key_by_unit
            .insert(unit_id.to_string(), key.clone());
        key
    }

    fn next_player_key(&mut self, unit_id: &str) -> String {
        let next_reuse_index = self
            .next_reuse_index_by_unit
            .entry(unit_id.to_string())
            .or_insert(0);
        loop {
            let key = if *next_reuse_index == 0 {
                unit_id.to_string()
            } else {
                format!("{unit_id}#{next_reuse_index}")
            };
            *next_reuse_index += 1;
            if !self.players.contains_key(&key) {
                return key;
            }
        }
    }
}

#[derive(Debug, Default)]
struct PlayerIdentity {
    character_name: Option<String>,
    account_name: Option<String>,
    character_id: Option<String>,
}

#[derive(Debug, Default)]
struct PlayerUnitFacts {
    class_id: Option<u16>,
    race_id: Option<u16>,
    level: Option<u16>,
    champion_points: Option<u32>,
}

pub(crate) fn extract_from_file(
    path: impl AsRef<Path>,
    report_code: Option<String>,
) -> Result<KalpaBuildEvidence, String> {
    extract_from_file_from(path, 0, report_code)
}

pub(crate) fn extract_from_file_from(
    path: impl AsRef<Path>,
    start_offset: u64,
    report_code: Option<String>,
) -> Result<KalpaBuildEvidence, String> {
    let mut file =
        File::open(path.as_ref()).map_err(|e| format!("Read build evidence failed: {e}"))?;
    file.seek(SeekFrom::Start(start_offset))
        .map_err(|e| format!("Read build evidence failed: {e}"))?;

    let reader = BufReader::new(file);
    let mut accumulator = BuildEvidenceAccumulator::default();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("Read build evidence failed: {e}"))?;
        accumulator.ingest_line(&line);
    }

    Ok(accumulator.finish(report_code))
}

#[cfg(test)]
fn extract_from_lines(lines: &[&str], report_code: Option<String>) -> KalpaBuildEvidence {
    let mut accumulator = BuildEvidenceAccumulator::default();
    for line in lines {
        accumulator.ingest_line(line);
    }

    accumulator.finish(report_code)
}

#[derive(Debug, Default)]
struct BuildEvidenceAccumulator {
    players: PlayerAccumulator,
    ability_infos: BTreeMap<u32, AbilityEvidenceInfo>,
    /// Scribing scripts seen since the last `PLAYER_INFO`. The log emits a player's
    /// scribed `ABILITY_INFO` block immediately before that player's `PLAYER_INFO`, so
    /// this buffer is flushed onto the next player and cleared.
    pending_scribing: BTreeMap<u32, ScribedScripts>,
}

impl BuildEvidenceAccumulator {
    fn ingest_line(&mut self, line: &str) {
        let fields = split_csv_quoted_pub(line);
        match fields.get(1).map(|s| s.trim()) {
            Some("ABILITY_INFO") => {
                ingest_ability_info(&mut self.ability_infos, &mut self.pending_scribing, &fields)
            }
            Some("UNIT_ADDED") => ingest_unit_added(&mut self.players, &fields),
            Some("PLAYER_INFO") => {
                ingest_player_info(&mut self.players, &fields, line);
                // Attribute the pending scribed-ability block to this player. Merge (not
                // replace) so a repeated PLAYER_INFO that only re-emits some scribed
                // abilities keeps the unit's earlier scripts for the untouched ones.
                if let Some(unit_id) = fields.get(2).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if !self.pending_scribing.is_empty() {
                        let player = self.players.active_player_mut(unit_id);
                        for (ability_id, scripts) in std::mem::take(&mut self.pending_scribing) {
                            player.scribed_scripts.insert(ability_id, scripts);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(self, report_code: Option<String>) -> KalpaBuildEvidence {
        let ability_infos = self.ability_infos;
        let players = self
            .players
            .into_players()
            .into_values()
            .filter(|p| {
                p.saw_player_info
                    || p.class_id.is_some()
                    || p.character_name.is_some()
                    || p.account_name.is_some()
                    || !p.class_mastery_passives.is_empty()
                    || !p.champion_point_passives.is_empty()
                    || resolve_food(&p.passive_ability_ids, &ability_infos).is_some()
                    || p.slotted_skill_ids.iter().any(|ability_id| {
                        ability_infos
                            .get(ability_id)
                            .and_then(|info| info.icon.as_deref())
                            .is_some_and(is_grimoire_icon_slug)
                    })
            })
            .map(|p| {
                let class_name = p.class_name_from_mastery.or(p.class_name_from_unit);
                let evidence = if p.saw_player_info {
                    "raw-player-info"
                } else {
                    "raw-unit-added"
                };
                let food = resolve_food(&p.passive_ability_ids, &ability_infos);
                let scribed_skills = resolve_scribed_skills(
                    &p.slotted_skill_ids,
                    &p.scribed_scripts,
                    &ability_infos,
                );
                KalpaPlayerBuildEvidence {
                    unit_id: p.unit_id,
                    unit_occurrence_id: Some(p.unit_occurrence_id),
                    character_name: p.character_name,
                    account_name: p.account_name,
                    character_id: p.character_id,
                    class_id: p.class_id,
                    race_id: p.race_id,
                    level: p.level,
                    champion_points: p.champion_points,
                    class_name,
                    class_mastery_passives: p.class_mastery_passives,
                    champion_point_passives: p.champion_point_passives,
                    food,
                    scribed_skills,
                    evidence: evidence.to_string(),
                    confidence: "exact".to_string(),
                }
            })
            .collect();

        KalpaBuildEvidence {
            schema_version: SCHEMA_VERSION,
            extractor_version: Some(EXTRACTOR_VERSION),
            source: SOURCE.to_string(),
            report_code,
            players,
        }
    }
}

fn ingest_ability_info(
    ability_infos: &mut BTreeMap<u32, AbilityEvidenceInfo>,
    pending_scribing: &mut BTreeMap<u32, ScribedScripts>,
    fields: &[&str],
) {
    let Some(ability_id) = fields.get(2).and_then(|s| parse_u32(s)) else {
        return;
    };
    let name = fields.get(3).and_then(|s| non_empty_string(unquote(s)));
    let icon = fields.get(4).and_then(|s| ability_icon_slug(unquote(s)));

    ability_infos
        .entry(ability_id)
        .or_insert(AbilityEvidenceInfo {
            ability_id,
            name,
            icon,
        });

    // A scribed ABILITY_INFO carries three extra fields — focus, signature, affix — that
    // ESO Logs strips from its API. Buffer them for the next PLAYER_INFO to attribute to
    // the owning player (the same ability id recurs per player with different scripts).
    let scripts = ScribedScripts {
        focus: fields.get(7).and_then(|s| non_empty_string(unquote(s))),
        signature: fields.get(8).and_then(|s| non_empty_string(unquote(s))),
        affix: fields.get(9).and_then(|s| non_empty_string(unquote(s))),
    };
    if scripts.focus.is_some() || scripts.signature.is_some() || scripts.affix.is_some() {
        pending_scribing.insert(ability_id, scripts);
    }
}

fn ingest_unit_added(players: &mut PlayerAccumulator, fields: &[&str]) {
    if fields.get(3).map(|s| unquote(s)) != Some("PLAYER") {
        return;
    }

    let Some(unit_id) = fields.get(2).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return;
    };

    let identity = PlayerIdentity {
        character_name: fields.get(10).and_then(|s| non_empty_string(unquote(s))),
        account_name: fields.get(11).and_then(|s| non_empty_string(unquote(s))),
        character_id: fields.get(12).and_then(|s| non_zero_string(s.trim())),
    };

    let facts = PlayerUnitFacts {
        class_id: fields.get(8).and_then(|s| parse_u16(s)),
        race_id: fields.get(9).and_then(|s| parse_u16(s)),
        level: fields.get(13).and_then(|s| parse_u16(s)),
        champion_points: fields.get(14).and_then(|s| parse_non_negative_u32(s)),
    };

    let entry = players.active_player_for_unit_added_mut(unit_id, &identity, &facts);

    entry.class_id = facts.class_id.or(entry.class_id);
    entry.race_id = facts.race_id.or(entry.race_id);
    entry.level = facts.level.or(entry.level);
    entry.champion_points = facts.champion_points.or(entry.champion_points);
    entry.class_name_from_unit = entry
        .class_id
        .and_then(class_name_from_unit_class_id)
        .map(str::to_string)
        .or(entry.class_name_from_unit.take());
    entry.character_name = identity.character_name.or(entry.character_name.take());
    entry.account_name = identity.account_name.or(entry.account_name.take());
    entry.character_id = identity.character_id.or(entry.character_id.take());
}

fn ingest_player_info(players: &mut PlayerAccumulator, fields: &[&str], line: &str) {
    let Some(unit_id) = fields.get(2).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return;
    };

    let entry = players.active_player_mut(unit_id);
    entry.saw_player_info = true;

    let arrays = tail_after_commas(line, 3);
    let top_level_arrays = split_top_level_commas(arrays);
    let Some(passive_ids_raw) = top_level_arrays.first().copied() else {
        return;
    };
    let passive_ids = parse_u32_array(passive_ids_raw);
    entry.passive_ability_ids = passive_ids.iter().copied().filter(|id| *id > 0).collect();
    entry.class_name_from_mastery = None;
    entry.class_mastery_passives.clear();
    entry.champion_point_passives.clear();
    for ability_id in &entry.passive_ability_ids {
        if entry.champion_point_passives.len() >= MAX_CHAMPION_POINT_PASSIVES {
            break;
        }
        if is_champion_point_passive(*ability_id)
            && !entry.champion_point_passives.contains(ability_id)
        {
            entry.champion_point_passives.push(*ability_id);
        }
    }

    entry.slotted_skill_ids.clear();
    if let Some(front_bar) = top_level_arrays.get(3) {
        entry
            .slotted_skill_ids
            .extend(parse_positive_u32_array(front_bar));
    }
    if let Some(back_bar) = top_level_arrays.get(4) {
        entry
            .slotted_skill_ids
            .extend(parse_positive_u32_array(back_bar));
    }

    for ability_id in passive_ids {
        let Some(owner) = class_name_from_class_mastery_passive(ability_id) else {
            continue;
        };
        if entry.class_name_from_mastery.is_none() {
            entry.class_name_from_mastery = Some(owner.to_string());
        }
        if entry.class_name_from_mastery.as_deref() != Some(owner)
            || entry.class_mastery_passives.contains(&ability_id)
        {
            continue;
        }
        entry.class_mastery_passives.push(ability_id);
        if entry.class_mastery_passives.len() >= CLASS_MASTERY_MAX_PICKS {
            break;
        }
    }
}

fn resolve_scribed_skills(
    slotted_skill_ids: &[u32],
    scribed_scripts: &BTreeMap<u32, ScribedScripts>,
    ability_infos: &BTreeMap<u32, AbilityEvidenceInfo>,
) -> Vec<KalpaScribedSkillEvidence> {
    let mut seen = std::collections::BTreeSet::<u32>::new();
    let mut result = Vec::new();
    for ability_id in slotted_skill_ids {
        if !seen.insert(*ability_id) {
            continue;
        }
        let Some(info) = ability_infos.get(ability_id) else {
            continue;
        };
        if !info.icon.as_deref().is_some_and(is_grimoire_icon_slug) {
            continue;
        }
        let scripts = scribed_scripts.get(ability_id);
        result.push(KalpaScribedSkillEvidence {
            ability_id: info.ability_id,
            name: info.name.clone(),
            icon: info.icon.clone(),
            focus_script: scripts.and_then(|s| s.focus.clone()),
            signature_script: scripts.and_then(|s| s.signature.clone()),
            affix_script: scripts.and_then(|s| s.affix.clone()),
        });
        if result.len() >= MAX_SCRIBED_SKILLS {
            break;
        }
    }
    result
}

fn resolve_food(
    passive_ability_ids: &[u32],
    ability_infos: &BTreeMap<u32, AbilityEvidenceInfo>,
) -> Option<KalpaFoodEvidence> {
    for ability_id in passive_ability_ids {
        let info = ability_infos.get(ability_id);
        if is_food_ability(*ability_id, info) {
            return Some(KalpaFoodEvidence {
                ability_id: *ability_id,
                name: info.and_then(|value| value.name.clone()),
                icon: info.and_then(|value| value.icon.clone()),
            });
        }
    }
    None
}

fn tail_after_commas(line: &str, commas_to_skip: usize) -> &str {
    let mut seen = 0;
    for (index, byte) in line.bytes().enumerate() {
        if byte == b',' {
            seen += 1;
            if seen == commas_to_skip {
                return &line[index + 1..];
            }
        }
    }
    ""
}

fn split_top_level_commas(input: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut depth = 0_i32;
    let mut in_quotes = false;

    for (index, byte) in input.bytes().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b'[' if !in_quotes => depth += 1,
            b']' if !in_quotes => depth -= 1,
            b',' if !in_quotes && depth == 0 => {
                fields.push(input[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    fields.push(input[start..].trim());
    fields
}

fn player_identity_conflicts(player: &PlayerBuilder, identity: &PlayerIdentity) -> bool {
    identity_field_conflicts(
        player.character_id.as_deref(),
        identity.character_id.as_deref(),
    ) || identity_field_conflicts(
        player.account_name.as_deref(),
        identity.account_name.as_deref(),
    ) || identity_field_conflicts(
        player.character_name.as_deref(),
        identity.character_name.as_deref(),
    )
}

fn player_unit_facts_conflict(
    player: &PlayerBuilder,
    identity: &PlayerIdentity,
    facts: &PlayerUnitFacts,
) -> bool {
    if player_identity_matches(player, identity) {
        return false;
    }

    fact_conflicts(player.class_id, facts.class_id)
        || fact_conflicts(player.race_id, facts.race_id)
        || fact_conflicts(player.level, facts.level)
        || fact_conflicts(player.champion_points, facts.champion_points)
}

fn player_identity_matches(player: &PlayerBuilder, identity: &PlayerIdentity) -> bool {
    if identity_field_matches(
        player.character_id.as_deref(),
        identity.character_id.as_deref(),
    ) {
        return true;
    }

    let account_matches = identity_field_matches(
        player.account_name.as_deref(),
        identity.account_name.as_deref(),
    );
    if !account_matches {
        return false;
    }

    identity.character_name.is_none()
        || player.character_name.is_none()
        || identity_field_matches(
            player.character_name.as_deref(),
            identity.character_name.as_deref(),
        )
}

fn identity_field_conflicts(existing: Option<&str>, next: Option<&str>) -> bool {
    let Some(existing) = existing
        .map(normalize_identity_field)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    let Some(next) = next.map(normalize_identity_field).filter(|s| !s.is_empty()) else {
        return false;
    };
    existing != next
}

fn identity_field_matches(existing: Option<&str>, next: Option<&str>) -> bool {
    let Some(existing) = existing
        .map(normalize_identity_field)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    let Some(next) = next.map(normalize_identity_field).filter(|s| !s.is_empty()) else {
        return false;
    };
    existing == next
}

fn fact_conflicts<T: Copy + PartialEq>(existing: Option<T>, next: Option<T>) -> bool {
    match (existing, next) {
        (Some(existing), Some(next)) => existing != next,
        _ => false,
    }
}

fn normalize_identity_field(input: &str) -> String {
    input.trim().to_ascii_lowercase()
}

fn parse_u32_array(input: &str) -> Vec<u32> {
    let trimmed = input.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed)
        .trim();
    if inner.is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .filter_map(|part| part.trim().parse::<u32>().ok())
        .collect()
}

fn parse_u16(input: &str) -> Option<u16> {
    input.trim().parse::<u16>().ok().filter(|value| *value > 0)
}

fn parse_u32(input: &str) -> Option<u32> {
    input.trim().parse::<u32>().ok().filter(|value| *value > 0)
}

fn parse_non_negative_u32(input: &str) -> Option<u32> {
    input.trim().parse::<u32>().ok()
}

fn parse_positive_u32_array(input: &str) -> Vec<u32> {
    parse_u32_array(input)
        .into_iter()
        .filter(|id| *id > 0)
        .collect()
}

fn non_empty_string(input: &str) -> Option<String> {
    let trimmed = input.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn non_zero_string(input: &str) -> Option<String> {
    let trimmed = input.trim();
    (!trimmed.is_empty() && trimmed != "0").then(|| trimmed.to_string())
}

fn unquote(input: &str) -> &str {
    input.trim().trim_matches('"')
}

fn ability_icon_slug(input: &str) -> Option<String> {
    let normalized = input.trim().trim_end_matches(".dds").to_lowercase();
    let basename = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    non_empty_string(basename)
}

fn is_grimoire_icon_slug(input: &str) -> bool {
    input.starts_with("ability_grimoire_")
}

fn is_champion_point_passive(ability_id: u32) -> bool {
    CHAMPION_POINT_PASSIVE_IDS.contains(&ability_id)
}

fn is_food_ability(ability_id: u32, info: Option<&AbilityEvidenceInfo>) -> bool {
    if FOOD_ABILITY_IDS.contains(&ability_id) {
        return true;
    }

    let Some(info) = info else {
        return false;
    };
    let name = info.name.as_deref().unwrap_or("");
    if is_named_food(name) {
        return true;
    }

    let icon = info.icon.as_deref().unwrap_or("");
    has_food_icon(icon) && is_generic_food_effect(name)
}

fn is_named_food(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    [
        "artaeum takeaway broth",
        "bewitched sugar skulls",
        "candied jester",
        "clockwork citrus filet",
        "crown fortifying meal",
        "crown vigorous tincture",
        "dubious camoran throne",
        "eye scream",
        "ghastly eye bowl",
        "jewels of misrule",
        "lava foot soup",
        "smoked bear haunch",
        "witchmother",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn is_generic_food_effect(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    [
        "increase all primary stats",
        "increase health regen",
        "increase health",
        "increase magicka",
        "increase stamina",
        "increase max health & magicka",
        "increase max health & stamina",
        "increase max magicka & stamina",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn has_food_icon(icon: &str) -> bool {
    icon.contains("food")
        || icon.contains("drink")
        || icon.contains("tricolor")
        || icon.contains("halloween_2016_iron_cup_bones")
}

fn class_name_from_unit_class_id(class_id: u16) -> Option<&'static str> {
    match class_id {
        1 => Some("Dragonknight"),
        2 => Some("Sorcerer"),
        3 => Some("Nightblade"),
        4 => Some("Warden"),
        5 => Some("Necromancer"),
        6 => Some("Templar"),
        117 => Some("Arcanist"),
        _ => None,
    }
}

fn class_name_from_class_mastery_passive(ability_id: u32) -> Option<&'static str> {
    match ability_id {
        238232 | 240268 | 259224 | 263220 | 263247 => Some("Dragonknight"),
        263316 | 263398 | 263410 | 263412 | 263416 => Some("Arcanist"),
        263448 | 263465 | 263509 | 263549 | 263554 => Some("Necromancer"),
        263519..=263523 => Some("Warden"),
        263585..=263589 => Some("Templar"),
        263603..=263607 => Some("Nightblade"),
        263870..=263874 => Some("Sorcerer"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_from_file_from_offset_skips_previous_sessions() {
        let old_session = "0,BEGIN_LOG,1700000000000,15,\"NA\",\"en\",\"eso.live\"\n\
             1,UNIT_ADDED,1,PLAYER,T,1,0,F,2,9,\"Old Arc\",\"@old\",111,50,1700,0,PLAYER_ALLY,T\n\
             2,PLAYER_INFO,1,[263870,142210,84731],[1,1,1],[],[],[]\n\
             3,END_LOG\n";
        let new_session =
            "0,BEGIN_LOG,1700001000000,15,\"NA\",\"en\",\"eso.live\"\n\
             1,UNIT_ADDED,2,PLAYER,T,2,0,F,6,5,\"New Beam\",\"@new\",222,50,2100,0,PLAYER_ALLY,T\n\
             2,ABILITY_INFO,89958,\"Increase Stamina\",\"/esoui/art/icons/store_magickafood_001.dds\",T,T\n\
             3,PLAYER_INFO,2,[263585,263586,156017,142092,89958],[1,1,1,1,1],[],[],[]\n";
        let content = format!("{old_session}{new_session}");
        let start_offset = old_session.len() as u64;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Encounter.log");
        std::fs::write(&path, content).expect("write fixture");

        let evidence =
            extract_from_file_from(&path, start_offset, Some("report123".into())).unwrap();

        assert_eq!(evidence.report_code.as_deref(), Some("report123"));
        assert_eq!(evidence.players.len(), 1);
        assert_eq!(
            evidence.players[0].character_name.as_deref(),
            Some("New Beam")
        );
        assert_eq!(evidence.players[0].account_name.as_deref(), Some("@new"));
        assert_eq!(evidence.players[0].class_name.as_deref(), Some("Templar"));
        assert_eq!(
            evidence.players[0].class_mastery_passives,
            vec![263585, 263586]
        );
        assert_eq!(
            evidence.players[0].champion_point_passives,
            vec![156017, 142092]
        );
        assert_eq!(
            evidence.players[0]
                .food
                .as_ref()
                .map(|food| food.ability_id),
            Some(89958)
        );
    }

    #[test]
    fn extracts_class_mastery_from_player_info() {
        let lines = [
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,2,9,\"Arc Spark\",\"@tester\",111,50,1700,0,PLAYER_ALLY,T",
            "1,ABILITY_INFO,68411,\"Increase All Primary Stats\",\"/esoui/art/icons/store_tricolor_food_01.dds\",T,T",
            "2,PLAYER_INFO,1,[263870,263871,142210,142079,68411],[1,1,1,1,1],[],[],[]",
        ];

        let evidence = extract_from_lines(&lines, Some("ABC123".to_string()));

        assert_eq!(evidence.schema_version, 1);
        assert_eq!(evidence.extractor_version, Some(EXTRACTOR_VERSION));
        assert_eq!(evidence.report_code.as_deref(), Some("ABC123"));
        assert_eq!(evidence.players.len(), 1);
        assert_eq!(evidence.players[0].class_id, Some(2));
        assert_eq!(evidence.players[0].race_id, Some(9));
        assert_eq!(evidence.players[0].level, Some(50));
        assert_eq!(evidence.players[0].champion_points, Some(1700));
        assert_eq!(evidence.players[0].class_name.as_deref(), Some("Sorcerer"));
        assert_eq!(
            evidence.players[0].class_mastery_passives,
            vec![263870, 263871]
        );
        assert_eq!(
            evidence.players[0].champion_point_passives,
            vec![142210, 142079]
        );
        assert_eq!(
            evidence.players[0].food,
            Some(KalpaFoodEvidence {
                ability_id: 68411,
                name: Some("Increase All Primary Stats".to_string()),
                icon: Some("store_tricolor_food_01".to_string()),
            })
        );
    }

    #[test]
    fn dedupes_and_limits_class_mastery_picks() {
        let lines = [
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,2,9,\"Arc Spark\",\"@tester\",111,50,1700,0,PLAYER_ALLY,T",
            "1,PLAYER_INFO,1,[263870,263870,263871,263872],[1,1,1,1],[],[],[]",
        ];

        let evidence = extract_from_lines(&lines, None);

        assert_eq!(
            evidence.players[0].class_mastery_passives,
            vec![263870, 263871]
        );
    }

    #[test]
    fn infers_class_from_passive_when_unit_class_id_is_unknown() {
        let lines = [
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,0,9,\"Book Beamin\",\"@tester\",111,50,1700,0,PLAYER_ALLY,T",
            "1,PLAYER_INFO,1,[263316],[1],[],[],[]",
        ];

        let evidence = extract_from_lines(&lines, None);

        assert_eq!(evidence.players[0].class_id, None);
        assert_eq!(evidence.players[0].class_name.as_deref(), Some("Arcanist"));
        assert_eq!(evidence.players[0].class_mastery_passives, vec![263316]);
    }

    #[test]
    fn handles_quoted_player_identity_with_commas() {
        let lines = [
            "0,UNIT_ADDED,44,PLAYER,F,63,0,F,3,9,\"Blade, With Comma\",\"@Acct,Name\",222,50,2100,0,PLAYER_ALLY,T",
            "1,PLAYER_INFO,44,[263603,263604],[1,1],[[HEAD,94773,T,16,ARMOR_DIVINES,LEGENDARY]],[63046],[40382]",
        ];

        let evidence = extract_from_lines(&lines, None);

        assert_eq!(
            evidence.players[0].character_name.as_deref(),
            Some("Blade, With Comma")
        );
        assert_eq!(
            evidence.players[0].account_name.as_deref(),
            Some("@Acct,Name")
        );
        assert_eq!(
            evidence.players[0].class_name.as_deref(),
            Some("Nightblade")
        );
        assert_eq!(
            evidence.players[0].class_mastery_passives,
            vec![263603, 263604]
        );
    }

    #[test]
    fn extracts_only_scribed_skills_from_player_bars() {
        let lines = [
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,2,9,\"Arc Spark\",\"@tester\",111,50,1700,0,PLAYER_ALLY,T",
            "1,ABILITY_INFO,63046,\"Regular Skill\",\"/esoui/art/icons/ability_sorcerer_dark_magic.dds\",F,T",
            "1,ABILITY_INFO,220543,\"Dazing Trample\",\"/esoui/art/icons/ability_grimoire_assault.dds\",F,T",
            "2,PLAYER_INFO,1,[12345],[1],[],[63046,220543],[40382]",
        ];

        let evidence = extract_from_lines(&lines, None);
        let skills = &evidence.players[0].scribed_skills;

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].ability_id, 220543);
        assert_eq!(skills[0].name.as_deref(), Some("Dazing Trample"));
        assert_eq!(skills[0].icon.as_deref(), Some("ability_grimoire_assault"));
    }

    #[test]
    fn scribed_skills_carry_per_player_scripts_with_carry_over_and_respec() {
        // Real capture shape: two players share the SAME scribed ability ids (217460
        // Warding Burst, 217784 Leashing Soul) but with DIFFERENT scripts. The log emits
        // each player's scribed ABILITY_INFO block immediately before that player's
        // PLAYER_INFO, so scripts must be attributed per unit. At ts 55698 player 1
        // respecs 217784, while 217460 is NOT re-emitted and must carry over player 1's
        // OWN earlier scripts — never player 23's variant of the shared id.
        let lines = [
            "6,UNIT_ADDED,1,PLAYER,T,3,0,F,1,5,\"Uro\",\"@TheMrPancake\",1800204486173734503,50,1874,0,PLAYER_ALLY,T",
            "6,UNIT_ADDED,23,PLAYER,F,4,0,F,117,4,\"Ally\",\"@Ally\",1000000,50,2318,0,PLAYER_ALLY,T",
            "2408,ABILITY_INFO,217460,\"Warding Burst\",\"/esoui/art/icons/ability_grimoire_soulmagic2.dds\",F,T,\"Damage Shield\",\"Class Mastery\",\"Intellect and Endurance\"",
            "2408,ABILITY_INFO,217784,\"Leashing Soul\",\"/esoui/art/icons/ability_grimoire_soulmagic1.dds\",F,T,\"Pull\",\"Druid's Resurgence\",\"Maim\"",
            "2409,PLAYER_INFO,1,[],[],[],[217460,217784],[217460,217784]",
            "2409,ABILITY_INFO,217460,\"Warding Burst\",\"/esoui/art/icons/ability_grimoire_soulmagic2.dds\",F,T,\"Damage Shield\",\"Crusader's Defiance\",\"Savagery and Prophecy\"",
            "2409,ABILITY_INFO,217784,\"Leashing Soul\",\"/esoui/art/icons/ability_grimoire_soulmagic1.dds\",F,T,\"Pull\",\"Anchorite's Potency\",\"Resolve\"",
            "2409,PLAYER_INFO,23,[],[],[],[186366,182977,38901,63044,40195,36514],[22095,39011,217460,217784,25267,189867]",
            "55698,ABILITY_INFO,217784,\"Leashing Soul\",\"/esoui/art/icons/ability_grimoire_soulmagic1.dds\",F,T,\"Pull\",\"Lingering Torment\",\"Defile\"",
            "55698,PLAYER_INFO,1,[],[],[],[217460,217784],[217460,217784]",
            "55698,PLAYER_INFO,23,[],[],[],[186366,182977,38901,63044,40195,36514],[22095,39011,217460,217784,25267,189867]",
        ];

        let evidence = extract_from_lines(&lines, None);
        let script_of = |unit: &str, ability: u32| -> KalpaScribedSkillEvidence {
            evidence
                .players
                .iter()
                .find(|p| p.unit_id == unit)
                .unwrap_or_else(|| panic!("missing player unit {unit}"))
                .scribed_skills
                .iter()
                .find(|s| s.ability_id == ability)
                .unwrap_or_else(|| panic!("missing scribed ability {ability} for unit {unit}"))
                .clone()
        };

        // Player 1: 217784 reflects the LATER respec (Lingering Torment / Defile) ...
        let p1_soul = script_of("1", 217784);
        assert_eq!(p1_soul.focus_script.as_deref(), Some("Pull"));
        assert_eq!(
            p1_soul.signature_script.as_deref(),
            Some("Lingering Torment")
        );
        assert_eq!(p1_soul.affix_script.as_deref(), Some("Defile"));
        // ... while 217460 (not re-emitted at 55698) carries over player 1's own earlier
        // scripts, NOT player 23's variant of the same shared ability id.
        let p1_ward = script_of("1", 217460);
        assert_eq!(p1_ward.signature_script.as_deref(), Some("Class Mastery"));
        assert_eq!(
            p1_ward.affix_script.as_deref(),
            Some("Intellect and Endurance")
        );

        // Player 23 keeps its own distinct scripts for the same shared ability ids.
        let p23_soul = script_of("23", 217784);
        assert_eq!(
            p23_soul.signature_script.as_deref(),
            Some("Anchorite's Potency")
        );
        assert_eq!(p23_soul.affix_script.as_deref(), Some("Resolve"));
        let p23_ward = script_of("23", 217460);
        assert_eq!(
            p23_ward.signature_script.as_deref(),
            Some("Crusader's Defiance")
        );
        assert_eq!(
            p23_ward.affix_script.as_deref(),
            Some("Savagery and Prophecy")
        );
    }

    #[test]
    fn extracts_build_facts_for_multiple_players() {
        let lines = [
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,2,9,\"Arc Spark\",\"@tester\",111,50,1700,0,PLAYER_ALLY,T",
            "0,UNIT_ADDED,2,PLAYER,T,2,0,F,6,5,\"Sun Beam\",\"@healer\",222,50,2100,0,PLAYER_ALLY,T",
            "1,ABILITY_INFO,84731,\"Witchmother's Potent Brew\",\"/esoui/art/icons/event_halloween_2016_iron_cup_bones.dds\",T,T",
            "2,ABILITY_INFO,89958,\"Increase Stamina\",\"/esoui/art/icons/store_magickafood_001.dds\",T,T",
            "3,PLAYER_INFO,1,[263870,263871,142210,142079,84731],[1,1,1,1,1],[],[],[]",
            "4,PLAYER_INFO,2,[263585,263586,156017,142092,89958],[1,1,1,1,1],[],[],[]",
        ];

        let evidence = extract_from_lines(&lines, None);

        assert_eq!(evidence.players.len(), 2);
        assert_eq!(evidence.players[0].unit_id, "1");
        assert_eq!(evidence.players[0].class_name.as_deref(), Some("Sorcerer"));
        assert_eq!(
            evidence.players[0].class_mastery_passives,
            vec![263870, 263871]
        );
        assert_eq!(
            evidence.players[0].champion_point_passives,
            vec![142210, 142079]
        );
        assert_eq!(
            evidence.players[0].food.as_ref().map(|f| f.ability_id),
            Some(84731)
        );

        assert_eq!(evidence.players[1].unit_id, "2");
        assert_eq!(evidence.players[1].race_id, Some(5));
        assert_eq!(evidence.players[1].champion_points, Some(2100));
        assert_eq!(evidence.players[1].class_name.as_deref(), Some("Templar"));
        assert_eq!(
            evidence.players[1].class_mastery_passives,
            vec![263585, 263586]
        );
        assert_eq!(
            evidence.players[1].champion_point_passives,
            vec![156017, 142092]
        );
        assert_eq!(
            evidence.players[1].food.as_ref().map(|f| f.ability_id),
            Some(89958)
        );
    }

    #[test]
    fn later_player_info_replaces_build_facts_for_same_player() {
        let lines = [
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,2,9,\"Arc Spark\",\"@tester\",111,50,1700,0,PLAYER_ALLY,T",
            "1,ABILITY_INFO,68411,\"Increase All Primary Stats\",\"/esoui/art/icons/store_tricolor_food_01.dds\",T,T",
            "2,ABILITY_INFO,84731,\"Witchmother's Potent Brew\",\"/esoui/art/icons/event_halloween_2016_iron_cup_bones.dds\",T,T",
            "3,PLAYER_INFO,1,[263870,142210,68411],[1,1,1],[],[],[]",
            "4,PLAYER_INFO,1,[263871,142079,84731],[1,1,1],[],[],[]",
        ];

        let evidence = extract_from_lines(&lines, None);

        assert_eq!(evidence.players.len(), 1);
        assert_eq!(evidence.players[0].class_mastery_passives, vec![263871]);
        assert_eq!(evidence.players[0].champion_point_passives, vec![142079]);
        assert_eq!(
            evidence.players[0].food.as_ref().map(|f| f.ability_id),
            Some(84731)
        );
    }

    #[test]
    fn unit_id_reuse_preserves_first_players_build_facts() {
        let lines = [
            "0,UNIT_ADDED,48,PLAYER,F,46,0,F,3,4,\"teach me too blade\",\"@Mayhem713\",2834060361547667383,50,1911,0,PLAYER_ALLY,T",
            "1,PLAYER_INFO,48,[142210,142079,263605,263604,127596],[1,1,1,1,1],[],[],[]",
            "2,UNIT_ADDED,48,PLAYER,F,53,0,F,117,7,\"Dud Spud Bud\",\"@conterri\",18432196684152629755,50,1651,0,PLAYER_ALLY,F",
        ];

        let evidence = extract_from_lines(&lines, None);

        assert_eq!(evidence.players.len(), 2);
        let first = evidence
            .players
            .iter()
            .find(|player| player.account_name.as_deref() == Some("@Mayhem713"))
            .expect("first occupant should remain matchable by account");
        assert_eq!(first.unit_id, "48");
        assert_eq!(first.unit_occurrence_id.as_deref(), Some("48"));
        assert_eq!(first.character_name.as_deref(), Some("teach me too blade"));
        assert_eq!(first.race_id, Some(4));
        assert_eq!(first.champion_points, Some(1911));
        assert_eq!(first.class_name.as_deref(), Some("Nightblade"));
        assert_eq!(first.class_mastery_passives, vec![263605, 263604]);
        assert_eq!(first.champion_point_passives, vec![142210, 142079]);

        let second = evidence
            .players
            .iter()
            .find(|player| player.account_name.as_deref() == Some("@conterri"))
            .expect("second occupant should get a separate sidecar row");
        assert_eq!(second.unit_id, "48");
        assert_eq!(second.unit_occurrence_id.as_deref(), Some("48#1"));
        assert_eq!(second.character_name.as_deref(), Some("Dud Spud Bud"));
        assert_eq!(second.race_id, Some(7));
        assert_eq!(second.champion_points, Some(1651));
        assert_eq!(second.class_name.as_deref(), Some("Arcanist"));
        assert!(second.class_mastery_passives.is_empty());
        assert!(second.champion_point_passives.is_empty());
    }

    #[test]
    fn anonymous_unit_id_reuse_does_not_overwrite_named_player_facts() {
        let lines = [
            "0,UNIT_ADDED,45,PLAYER,F,39,0,F,1,7,\"Adolphc\",\"@hulin15823987726\",14384772626918308164,50,2145,0,PLAYER_ALLY,T",
            "1,PLAYER_INFO,45,[142210,142079,61218,150054,147226],[1,1,1,1,1],[],[],[]",
            "2,UNIT_ADDED,45,PLAYER,F,50,0,F,6,7,\"\",\"\",0,50,773,0,PLAYER_ALLY,F",
            "3,PLAYER_INFO,45,[263585,263586,142092,156017,89958],[1,1,1,1,1],[],[],[]",
        ];

        let evidence = extract_from_lines(&lines, None);

        assert_eq!(evidence.players.len(), 2);
        let first = evidence
            .players
            .iter()
            .find(|player| player.account_name.as_deref() == Some("@hulin15823987726"))
            .expect("named first occupant should remain matchable by account");
        assert_eq!(first.unit_id, "45");
        assert_eq!(first.unit_occurrence_id.as_deref(), Some("45"));
        assert_eq!(first.character_name.as_deref(), Some("Adolphc"));
        assert_eq!(first.class_id, Some(1));
        assert_eq!(first.race_id, Some(7));
        assert_eq!(first.champion_points, Some(2145));
        assert_eq!(first.class_name.as_deref(), Some("Dragonknight"));
        assert!(first.class_mastery_passives.is_empty());
        assert_eq!(first.champion_point_passives, vec![142210, 142079]);

        let second = evidence
            .players
            .iter()
            .find(|player| player.account_name.is_none() && player.champion_points == Some(773))
            .expect("anonymous reused occupant should get a separate sidecar row");
        assert_eq!(second.unit_id, "45");
        assert_eq!(second.unit_occurrence_id.as_deref(), Some("45#1"));
        assert_eq!(second.class_id, Some(6));
        assert_eq!(second.race_id, Some(7));
        assert_eq!(second.class_name.as_deref(), Some("Templar"));
        assert_eq!(second.class_mastery_passives, vec![263585, 263586]);
        assert_eq!(second.champion_point_passives, vec![142092, 156017]);
    }

    #[test]
    fn labels_player_info_evidence_even_without_class_mastery() {
        let lines = [
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,2,9,\"Arc Spark\",\"@tester\",111,50,1700,0,PLAYER_ALLY,T",
            "1,PLAYER_INFO,1,[12345],[1],[],[63046],[40382]",
        ];

        let evidence = extract_from_lines(&lines, None);

        assert_eq!(evidence.players[0].evidence, "raw-player-info");
        assert_eq!(
            evidence.players[0].class_mastery_passives,
            Vec::<u32>::new()
        );
        assert!(evidence.players[0].scribed_skills.is_empty());
    }
}
