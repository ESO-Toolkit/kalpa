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
/// Cap on a single companion SavedVariables file. `ESOTKCompanion.lua` is an append-forever
/// snapshot store, and a multi-environment setup (live + liveeu + pts, Proton prefixes) has
/// one per environment — so an uncapped read scales with months of play times the number of
/// environments. Every other SavedVariables reader in the repo bounds its read (20 MB for the
/// SV editor, 1 MB per file for the LAM scan); a few MB is far more than the ring needs, so
/// anything larger is corrupt or foreign and is skipped without being read.
const MAX_COMPANION_BYTES: u64 = 8 * 1024 * 1024;

/// Read the ESOTK Companion snapshots for the players in `evidence` and return them for the
/// sidecar, or `None` when there's no companion file, nothing usable, or anything fails.
///
/// A user can have several companion files (live + liveeu + pts), and the first readable
/// one isn't necessarily the one for the uploaded report. Prefer the first file whose
/// snapshots actually name-match this report's players; only when no file matches, fall
/// back to the first file that yielded any snapshots at all (esotk still matches by
/// character + time, so an unfiltered forward is safe).
pub(crate) fn read_for_upload(evidence: &KalpaBuildEvidence) -> Option<KalpaCompanionEvidence> {
    let logger_chars = logger_character_names(evidence);
    pick_evidence(companion_file_contents(), &logger_chars)
}

/// Rank candidate companion-file contents: the first file whose snapshots name-match this
/// report's players wins; otherwise the first file with any snapshots at all.
///
/// Takes an ITERATOR of owned contents so the caller can read each file lazily: at most one
/// file's text is resident at a time, and a name-match short-circuits the rest entirely.
fn pick_evidence(
    contents: impl Iterator<Item = String>,
    logger_chars: &[String],
) -> Option<KalpaCompanionEvidence> {
    let mut fallback: Option<Vec<serde_json::Value>> = None;
    for content in contents {
        let (snapshots, matched) = select_snapshots(&content, logger_chars);
        if snapshots.is_empty() {
            continue;
        }
        if matched {
            return Some(KalpaCompanionEvidence { snapshots });
        }
        if fallback.is_none() {
            fallback = Some(snapshots);
        }
    }
    fallback.map(|snapshots| KalpaCompanionEvidence { snapshots })
}

/// Locate every `SavedVariables/ESOTKCompanion.lua` across the standard ESO environments,
/// in candidate order, and read them LAZILY — one file's contents at a time, skipping any
/// file above [`MAX_COMPANION_BYTES`] without reading it.
fn companion_file_contents() -> impl Iterator<Item = String> {
    companion_file_paths().into_iter().filter_map(|path| {
        let size = std::fs::metadata(&path).ok()?.len();
        if size > MAX_COMPANION_BYTES {
            return None;
        }
        std::fs::read_to_string(&path).ok()
    })
}

fn companion_file_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for base in documents_candidates() {
        for env in ESO_ENVS {
            paths.push(
                base.join("Elder Scrolls Online")
                    .join(env)
                    .join("SavedVariables")
                    .join(COMPANION_FILE),
            );
        }
    }
    paths
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
/// matches by character + time, so an unfiltered forward is safe). The second tuple field
/// reports whether any snapshot name-matched, so callers can rank multiple files.
fn select_snapshots(content: &str, logger_chars: &[String]) -> (Vec<serde_json::Value>, bool) {
    let Ok(root) = parse_sv_file(content, SV_TABLE) else {
        return (Vec::new(), false);
    };
    let Some(sv) = child(&root, SV_TABLE) else {
        return (Vec::new(), false);
    };
    let Some(default) = child(sv, "Default") else {
        return (Vec::new(), false);
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
    (
        items.into_iter().map(|(_, _, value)| value).collect(),
        any_matched,
    )
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
        let (snaps, matched) = select_snapshots(SAMPLE, &["grappa'ko'laid".to_string()]);
        assert!(matched);
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
        let (snaps, matched) = select_snapshots(SAMPLE, &["nonexistent-character".to_string()]);
        assert!(!matched);
        assert_eq!(
            snaps.len(),
            3,
            "no name match => forward all, esotk filters by time"
        );
    }

    #[test]
    fn empty_logger_set_forwards_all() {
        let (snaps, _) = select_snapshots(SAMPLE, &[]);
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
        let (snaps, _) = select_snapshots(&body, &["zed".to_string()]);
        assert_eq!(snaps.len(), MAX_SNAPSHOTS);
    }

    #[test]
    fn prefers_name_matched_file_over_first_readable() {
        // A stale wrong-region file (no matching chars) must not shadow the
        // right-region file that names the logger's character.
        let stale = "ESOTKCompanionSV = {
[\"Default\"]={[\"@a\"]={[\"$AccountWide\"]={[\"snapshots\"]={
[1]={[\"ts\"]=1,[\"char\"]=\"WrongRegionAlt\"},
}}}}
}
";
        let matching = "ESOTKCompanionSV = {
[\"Default\"]={[\"@a\"]={[\"$AccountWide\"]={[\"snapshots\"]={
[1]={[\"ts\"]=2,[\"char\"]=\"Zed\"},
}}}}
}
";
        let chars = vec!["zed".to_string()];
        let picked = pick_evidence(
            [stale.to_string(), matching.to_string()].into_iter(),
            &chars,
        )
        .unwrap();
        assert_eq!(picked.snapshots.len(), 1);
        assert_eq!(
            picked.snapshots[0].get("char").and_then(|c| c.as_str()),
            Some("Zed")
        );
        // With no match anywhere, the first readable file still wins.
        let picked = pick_evidence([stale.to_string()].into_iter(), &chars).unwrap();
        assert_eq!(
            picked.snapshots[0].get("char").and_then(|c| c.as_str()),
            Some("WrongRegionAlt")
        );
    }

    #[test]
    fn garbage_content_yields_empty() {
        assert!(select_snapshots("not a savedvars file at all", &[])
            .0
            .is_empty());
        assert!(select_snapshots("", &[]).0.is_empty());
    }
}
