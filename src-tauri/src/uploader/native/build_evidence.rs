//! Sidecar build evidence recovered from native raw logs.
//!
//! This does not participate in ESO Logs' upload payload. It preserves exact
//! client-side `PLAYER_INFO` build facts that the public report API can lose or
//! omit, then passes a small versioned JSON payload to ESO Log Aggregator.

use std::collections::BTreeMap;
use std::path::Path;

use crate::uploader::types::{
    KalpaBuildEvidence, KalpaFoodEvidence, KalpaPlayerBuildEvidence, KalpaScribedSkillEvidence,
};

use super::encode::split_csv_quoted_pub;

const SCHEMA_VERSION: u8 = 1;
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
    saw_player_info: bool,
}

#[derive(Debug, Clone)]
struct AbilityEvidenceInfo {
    ability_id: u32,
    name: Option<String>,
    icon: Option<String>,
}

pub(crate) fn extract_from_file(
    path: impl AsRef<Path>,
    report_code: Option<String>,
) -> Result<KalpaBuildEvidence, String> {
    let text = std::fs::read_to_string(path.as_ref())
        .map_err(|e| format!("Read build evidence failed: {e}"))?;
    let lines = text.lines().collect::<Vec<_>>();
    Ok(extract_from_lines(&lines, report_code))
}

pub(crate) fn extract_from_lines(
    lines: &[&str],
    report_code: Option<String>,
) -> KalpaBuildEvidence {
    let mut players = BTreeMap::<String, PlayerBuilder>::new();
    let mut ability_infos = BTreeMap::<u32, AbilityEvidenceInfo>::new();

    for line in lines {
        let fields = split_csv_quoted_pub(line);
        match fields.get(1).map(|s| s.trim()) {
            Some("ABILITY_INFO") => ingest_ability_info(&mut ability_infos, &fields),
            Some("UNIT_ADDED") => ingest_unit_added(&mut players, &fields),
            Some("PLAYER_INFO") => ingest_player_info(&mut players, &fields, line),
            _ => {}
        }
    }

    let players = players
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
            let scribed_skills = resolve_scribed_skills(&p.slotted_skill_ids, &ability_infos);
            KalpaPlayerBuildEvidence {
                unit_id: p.unit_id,
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
        source: SOURCE.to_string(),
        report_code,
        players,
    }
}

fn ingest_ability_info(ability_infos: &mut BTreeMap<u32, AbilityEvidenceInfo>, fields: &[&str]) {
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
}

fn ingest_unit_added(players: &mut BTreeMap<String, PlayerBuilder>, fields: &[&str]) {
    if fields.get(3).map(|s| unquote(s)) != Some("PLAYER") {
        return;
    }

    let Some(unit_id) = fields.get(2).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return;
    };

    let entry = players
        .entry(unit_id.to_string())
        .or_insert_with(|| PlayerBuilder {
            unit_id: unit_id.to_string(),
            ..PlayerBuilder::default()
        });

    entry.class_id = fields.get(8).and_then(|s| parse_u16(s)).or(entry.class_id);
    entry.race_id = fields.get(9).and_then(|s| parse_u16(s)).or(entry.race_id);
    entry.level = fields.get(13).and_then(|s| parse_u16(s)).or(entry.level);
    entry.champion_points = fields
        .get(14)
        .and_then(|s| parse_non_negative_u32(s))
        .or(entry.champion_points);
    entry.class_name_from_unit = entry
        .class_id
        .and_then(class_name_from_unit_class_id)
        .map(str::to_string)
        .or(entry.class_name_from_unit.take());
    entry.character_name = fields
        .get(10)
        .and_then(|s| non_empty_string(unquote(s)))
        .or(entry.character_name.take());
    entry.account_name = fields
        .get(11)
        .and_then(|s| non_empty_string(unquote(s)))
        .or(entry.account_name.take());
    entry.character_id = fields
        .get(12)
        .and_then(|s| non_zero_string(s.trim()))
        .or(entry.character_id.take());
}

fn ingest_player_info(players: &mut BTreeMap<String, PlayerBuilder>, fields: &[&str], line: &str) {
    let Some(unit_id) = fields.get(2).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return;
    };

    let entry = players
        .entry(unit_id.to_string())
        .or_insert_with(|| PlayerBuilder {
            unit_id: unit_id.to_string(),
            ..PlayerBuilder::default()
        });
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
        result.push(KalpaScribedSkillEvidence {
            ability_id: info.ability_id,
            name: info.name.clone(),
            icon: info.icon.clone(),
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
    fn extracts_class_mastery_from_player_info() {
        let lines = [
            "0,UNIT_ADDED,1,PLAYER,T,1,0,F,2,9,\"Arc Spark\",\"@tester\",111,50,1700,0,PLAYER_ALLY,T",
            "1,ABILITY_INFO,68411,\"Increase All Primary Stats\",\"/esoui/art/icons/store_tricolor_food_01.dds\",T,T",
            "2,PLAYER_INFO,1,[263870,263871,142210,142079,68411],[1,1,1,1,1],[],[],[]",
        ];

        let evidence = extract_from_lines(&lines, Some("ABC123".to_string()));

        assert_eq!(evidence.schema_version, 1);
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
