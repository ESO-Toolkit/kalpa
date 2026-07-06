//! ESOTK Companion snapshot forwarding.
//!
//! After a native upload, read the local ESOTK Companion SavedVariables and forward the
//! logging player's recent snapshots alongside the build-evidence sidecar. The consumer
//! (esotk) matches each snapshot to a fight and renders the champion-point allocation ESO
//! Logs can't carry.
//!
//! Best-effort in every respect: a missing/garbage/oversized file, a parse failure, or no
//! matching snapshot all yield `None`, and the caller must treat that as "no companion data"
//! and never fail the upload. Logger-own-character only — the SavedVariables file only ever
//! holds the local player's captures.

use std::path::PathBuf;

use crate::commands::documents_candidates;
use crate::saved_variables::parser::parse_sv_file;
use crate::saved_variables::types::SvTreeNode;
use crate::uploader::types::{KalpaBuildEvidence, KalpaCompanionEvidence};

const COMPANION_FILE: &str = "ESOTKCompanion.lua";
/// The Lua table the addon writes (the file is named after the addon, the table after the
/// `## SavedVariables:` directive).
const SV_TABLE: &str = "ESOTKCompanionSV";
/// ESO client environments whose SavedVariables we probe, in preference order.
const ESO_ENVS: &[&str] = &["live", "liveeu", "pts"];
/// Cap the forwarded snapshots so the sidecar payload stays small; the consumer picks the
/// right one per fight. The addon keeps a ~200-entry ring — we send only the newest few.
const MAX_SNAPSHOTS: usize = 24;

/// Read the ESOTK Companion snapshots for the players in `evidence` and return them for the
/// sidecar, or `None` when there's no companion file, nothing usable, or anything fails.
pub(crate) fn read_for_upload(evidence: &KalpaBuildEvidence) -> Option<KalpaCompanionEvidence> {
    let content = read_companion_file()?;
    let logger_chars = logger_character_names(evidence);
    let snapshots = select_snapshots(&content, &logger_chars);
    if snapshots.is_empty() {
        None
    } else {
        Some(KalpaCompanionEvidence { snapshots })
    }
}

/// Locate + read `SavedVariables/ESOTKCompanion.lua` across the standard ESO environments.
fn read_companion_file() -> Option<String> {
    for base in documents_candidates() {
        for env in ESO_ENVS {
            let path: PathBuf = base
                .join("Elder Scrolls Online")
                .join(env)
                .join("SavedVariables")
                .join(COMPANION_FILE);
            if let Ok(content) = std::fs::read_to_string(&path) {
                return Some(content);
            }
        }
    }
    None
}

/// Lower-cased character names from the report's players — used to keep only the logger's
/// active-character snapshots (dropping stale captures for other characters the account plays).
fn logger_character_names(evidence: &KalpaBuildEvidence) -> Vec<String> {
    evidence
        .players
        .iter()
        .filter_map(|p| p.character_name.as_deref())
        .map(|n| n.trim().to_lowercase())
        .filter(|n| !n.is_empty())
        .collect()
}

/// Parse the companion file and return the newest snapshots (raw JSON), preferring the
/// logger's characters but falling back to all snapshots if none match by name (esotk still
/// matches by character + time, so an unfiltered forward is safe).
fn select_snapshots(content: &str, logger_chars: &[String]) -> Vec<serde_json::Value> {
    let Ok(root) = parse_sv_file(content, SV_TABLE) else {
        return Vec::new();
    };
    let Some(sv) = child(&root, SV_TABLE) else {
        return Vec::new();
    };
    let Some(default) = child(sv, "Default") else {
        return Vec::new();
    };

    // (ts, matched-by-char, snapshot-json)
    let mut items: Vec<(f64, bool, serde_json::Value)> = Vec::new();
    for account in default.children.iter().flatten() {
        for bucket in account.children.iter().flatten() {
            let Some(snaps) = child(bucket, "snapshots") else {
                continue;
            };
            for snap in snaps.children.iter().flatten() {
                let value = node_to_json(snap);
                let matched = char_matches(&value, logger_chars);
                let ts = value.get("ts").and_then(|t| t.as_f64()).unwrap_or(0.0);
                items.push((ts, matched, value));
            }
        }
    }

    // Prefer name-matched snapshots; if none match, keep all (name normalization can differ).
    let any_matched = items.iter().any(|(_, matched, _)| *matched);
    items.retain(|(_, matched, _)| !any_matched || *matched);

    // Newest first, capped.
    items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    items.truncate(MAX_SNAPSHOTS);
    items.into_iter().map(|(_, _, value)| value).collect()
}

/// Whether a snapshot's `char` is one of the logger's report characters. An empty logger set
/// (no character names captured) matches nothing here, deferring to the unfiltered fallback.
fn char_matches(snapshot: &serde_json::Value, logger_chars: &[String]) -> bool {
    if logger_chars.is_empty() {
        return false;
    }
    snapshot
        .get("char")
        .and_then(|c| c.as_str())
        .map(|c| logger_chars.contains(&c.trim().to_lowercase()))
        .unwrap_or(false)
}

/// Find a direct child node by key.
fn child<'a>(node: &'a SvTreeNode, key: &str) -> Option<&'a SvTreeNode> {
    node.children.iter().flatten().find(|c| c.key == key)
}

/// Convert an `SvTreeNode` subtree to a JSON value: a branch becomes an object keyed by child
/// key (ESO numeric Lua keys become "1".."12" string keys, which the consumer handles); a
/// leaf becomes its parsed value.
fn node_to_json(node: &SvTreeNode) -> serde_json::Value {
    if let Some(children) = &node.children {
        let mut map = serde_json::Map::with_capacity(children.len());
        for c in children {
            map.insert(c.key.clone(), node_to_json(c));
        }
        serde_json::Value::Object(map)
    } else if let Some(v) = &node.value {
        v.clone()
    } else {
        serde_json::Value::Null
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
ESOTKCompanionSV = {
    ["Default"] = {
        ["@acct"] = {
            ["$AccountWide"] = {
                ["schemaVersion"] = 1,
                ["season"] = "U50",
                ["snapshots"] = {
                    [1] = {
                        ["ts"] = 1749384000,
                        ["char"] = "Grappa'Ko'Laid",
                        ["server"] = "NA",
                        ["cp"] = { ["total"] = 3600, ["slotted"] = { [5] = 25, [6] = 31 } },
                        ["stats"] = { ["physicalPen"] = 7778 },
                    },
                    [2] = {
                        ["ts"] = 1749390000,
                        ["char"] = "Grappa'Ko'Laid",
                        ["server"] = "NA",
                        ["cp"] = { ["total"] = 3600 },
                    },
                    [3] = {
                        ["ts"] = 1749380000,
                        ["char"] = "SomeOtherAlt",
                    },
                },
            },
        },
    },
}
"#;

    #[test]
    fn selects_logger_char_snapshots_newest_first() {
        let snaps = select_snapshots(SAMPLE, &["grappa'ko'laid".to_string()]);
        assert_eq!(
            snaps.len(),
            2,
            "only the logger's two snapshots, not the alt"
        );
        // Newest first (ts 1749390000 before 1749384000).
        assert_eq!(
            snaps[0].get("ts").and_then(|t| t.as_f64()),
            Some(1749390000.0)
        );
        assert_eq!(
            snaps[1].get("ts").and_then(|t| t.as_f64()),
            Some(1749384000.0)
        );
        // Nested table round-trips (cp.total, cp.slotted keyed by slot).
        let cp = snaps[1].get("cp").unwrap();
        assert_eq!(cp.get("total").and_then(|t| t.as_f64()), Some(3600.0));
        assert_eq!(
            cp.get("slotted")
                .and_then(|s| s.get("6"))
                .and_then(|v| v.as_f64()),
            Some(31.0),
            "Backstabber (skill id 31) slotted under numeric-key '6'"
        );
    }

    #[test]
    fn falls_back_to_all_snapshots_when_no_name_matches() {
        let snaps = select_snapshots(SAMPLE, &["nonexistent-character".to_string()]);
        assert_eq!(
            snaps.len(),
            3,
            "no name match => forward all, esotk filters by time"
        );
    }

    #[test]
    fn empty_logger_set_forwards_all() {
        let snaps = select_snapshots(SAMPLE, &[]);
        assert_eq!(snaps.len(), 3);
    }

    #[test]
    fn caps_at_max_snapshots() {
        let mut body = String::from(
            "ESOTKCompanionSV = {\n[\"Default\"]={[\"@a\"]={[\"$AccountWide\"]={[\"snapshots\"]={\n",
        );
        for i in 1..=(MAX_SNAPSHOTS + 10) {
            body.push_str(&format!(
                "[{i}]={{[\"ts\"]={ts},[\"char\"]=\"Zed\"}},\n",
                ts = 1_000_000 + i
            ));
        }
        body.push_str("}}}}\n}\n");
        let snaps = select_snapshots(&body, &["zed".to_string()]);
        assert_eq!(snaps.len(), MAX_SNAPSHOTS);
    }

    #[test]
    fn garbage_content_yields_empty() {
        assert!(select_snapshots("not a savedvars file at all", &[]).is_empty());
        assert!(select_snapshots("", &[]).is_empty());
    }
}
