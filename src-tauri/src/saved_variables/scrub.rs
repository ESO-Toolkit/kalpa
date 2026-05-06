//! Phase 0 spike: identity templating + heuristic value scrubbing for
//! SavedVariables trees.
//!
//! Goal: validate whether a generic scrub pass produces output that is
//! (a) small enough to share over the existing pack channel and
//! (b) free of obvious account/character/social data
//! before committing to a `.esopack` v2 wire format or any UI work.
//!
//! Not wired into export yet — exposed only via a debug-only Tauri command.
//!
//! Current rules (intentionally conservative):
//!
//! * **Block-by-key-name.** Subtrees rooted at a key whose name matches any
//!   substring in `BLOCKED_KEY_SUBSTRINGS` are dropped. This catches
//!   logs/history/social tables (`SalesHistory`, `mailQueue`, `friends`, etc.).
//! * **Identity-keyed branches are templated.** Source account names,
//!   character names, character IDs, and world names that appear as table
//!   *keys* are replaced with placeholders (`${ACCOUNT}`, `${CHAR:N}`,
//!   `${CHAR_ID:N}`, `${WORLD}`).
//! * **Identity-bearing string values are dropped, not templated.** Some
//!   addons store account/character names in string values as legitimate
//!   config (allowlists, ignore lists). Substituting an importer's identity
//!   would silently change behaviour, so we drop those leaves and record
//!   them in the report.
//! * **Self-mapping helper keys are dropped.** `$LastCharacterName` and
//!   similar helpers are author-local and meaningless on import.

use super::serializer::serialize_to_lua;
use super::types::{SvTreeNode, SvValueType};
use serde::{Deserialize, Serialize};

/// Identities the scrubber should template or drop.
///
/// All matching is case-sensitive against ESO's actual stored values, except
/// world names which use the canonical names in `WELL_KNOWN_WORLDS`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrubContext {
    /// Account handles, e.g. `"@Shadowshire"`. Matched as full strings.
    pub accounts: Vec<String>,
    /// Character display names, e.g. `"Mainchar"`. Matched as full strings.
    pub characters: Vec<String>,
    /// Numeric character IDs as strings (ESO sometimes keys per-character
    /// tables by ID instead of name).
    pub character_ids: Vec<String>,
    /// Additional world names to template beyond the well-known list.
    #[serde(default)]
    pub extra_worlds: Vec<String>,
}

/// World names ESO uses as SavedVariables keys.
/// `GetWorldName()` returns these strings.
pub const WELL_KNOWN_WORLDS: &[&str] = &["NA Megaserver", "EU Megaserver", "PTS"];

/// Substrings (lowercased) that, when found in a key name, mark its subtree
/// as containing addon state we don't want to share.
///
/// Kept intentionally aggressive for the spike — false positives are visible
/// in the report and easy to whitelist later via per-addon overrides.
const BLOCKED_KEY_SUBSTRINGS: &[&str] = &[
    "mail",
    "friend",
    "whisper",
    "ignore",
    "sales",
    "history",
    "purchase",
    "trade",
    "roster",
    "bank",
    "inventory",
    "bag",
    "gold",
    "currency",
    "wallet",
    "recent",
    "lastseen",
    "lastonline",
    "fight", // CombatMetrics: fightData, fights
    "combatlog",
    "logs",
    "events",
    "messages",
    "guildstore",
    "guildhistory",
    "guildbank",
    "guildroster",
];

/// Exact key names (case-sensitive) that should always be dropped because
/// they encode the exporter's identity in a way templating can't fix.
const ALWAYS_DROPPED_KEYS: &[&str] = &["$LastCharacterName"];

/// Reason a node was dropped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DropReason {
    /// Key name matched a substring in `BLOCKED_KEY_SUBSTRINGS`.
    BlockedKeyHeuristic,
    /// Key name was in `ALWAYS_DROPPED_KEYS`.
    AlwaysDropped,
    /// String value contained an account/character identity.
    StringValueContainsIdentity,
    /// String value matched the `@Handle` shape even though no specific
    /// identity was provided in `ScrubContext`.
    StringValueLooksLikeAccount,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TemplateKind {
    Account,
    Character,
    CharacterId,
    World,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropEntry {
    pub path: Vec<String>,
    pub reason: DropReason,
    /// Approximate size (in bytes of serialized Lua) of what was removed.
    /// Lets the caller see whether dropping was responsible for most of the
    /// size reduction.
    pub bytes_removed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateEntry {
    pub path: Vec<String>,
    pub kind: TemplateKind,
    pub original: String,
    pub placeholder: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrubReport {
    pub drops: Vec<DropEntry>,
    pub templated_keys: Vec<TemplateEntry>,
    pub original_bytes: usize,
    pub scrubbed_bytes: usize,
}

/// Run the scrubber. Returns the cleaned tree alongside a report describing
/// every drop and template substitution.
pub fn scrub(tree: &SvTreeNode, ctx: &ScrubContext) -> (SvTreeNode, ScrubReport) {
    let original_bytes = serialize_to_lua(tree).len();

    let mut report = ScrubReport {
        original_bytes,
        ..ScrubReport::default()
    };

    // Build placeholder maps so repeat appearances get the same index.
    let mut placeholders = PlaceholderTable::new(ctx);

    let mut path: Vec<String> = Vec::new();
    let scrubbed = scrub_node(tree, &mut path, ctx, &mut placeholders, &mut report)
        // `scrub_node` only returns `None` when the *root* itself is dropped,
        // which can't happen because we never apply key-based rules to the
        // synthetic root. Defensive fallback returns an empty tree.
        .unwrap_or_else(|| SvTreeNode {
            key: tree.key.clone(),
            value_type: SvValueType::Table,
            value: None,
            children: Some(Vec::new()),
            raw_lua_value: None,
        });

    report.scrubbed_bytes = serialize_to_lua(&scrubbed).len();
    (scrubbed, report)
}

// ── internals ────────────────────────────────────────────────────────────

struct PlaceholderTable {
    accounts: std::collections::HashMap<String, String>,
    characters: std::collections::HashMap<String, String>,
    character_ids: std::collections::HashMap<String, String>,
}

impl PlaceholderTable {
    fn new(ctx: &ScrubContext) -> Self {
        let mut accounts = std::collections::HashMap::new();
        for (i, a) in ctx.accounts.iter().enumerate() {
            let label = if i == 0 {
                "${ACCOUNT}".to_string()
            } else {
                format!("${{ACCOUNT:{}}}", i)
            };
            accounts.insert(a.clone(), label);
        }
        let mut characters = std::collections::HashMap::new();
        for (i, c) in ctx.characters.iter().enumerate() {
            characters.insert(c.clone(), format!("${{CHAR:{}}}", i));
        }
        let mut character_ids = std::collections::HashMap::new();
        for (i, id) in ctx.character_ids.iter().enumerate() {
            character_ids.insert(id.clone(), format!("${{CHAR_ID:{}}}", i));
        }
        Self {
            accounts,
            characters,
            character_ids,
        }
    }

    fn template_for_key(&self, key: &str, ctx: &ScrubContext) -> Option<(String, TemplateKind)> {
        if let Some(p) = self.accounts.get(key) {
            return Some((p.clone(), TemplateKind::Account));
        }
        if let Some(p) = self.characters.get(key) {
            return Some((p.clone(), TemplateKind::Character));
        }
        if let Some(p) = self.character_ids.get(key) {
            return Some((p.clone(), TemplateKind::CharacterId));
        }
        if WELL_KNOWN_WORLDS.contains(&key) || ctx.extra_worlds.iter().any(|w| w == key) {
            return Some(("${WORLD}".to_string(), TemplateKind::World));
        }
        None
    }
}

fn key_is_blocked(key: &str) -> bool {
    if ALWAYS_DROPPED_KEYS.contains(&key) {
        return true;
    }
    let lower = key.to_ascii_lowercase();
    BLOCKED_KEY_SUBSTRINGS
        .iter()
        .any(|needle| lower.contains(needle))
}

fn drop_reason_for_key(key: &str) -> DropReason {
    if ALWAYS_DROPPED_KEYS.contains(&key) {
        DropReason::AlwaysDropped
    } else {
        DropReason::BlockedKeyHeuristic
    }
}

/// Heuristic detector: does this string contain something that looks like an
/// ESO account handle (`@` followed by non-whitespace)?
fn looks_like_account_handle(s: &str) -> bool {
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'@' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if !next.is_ascii_whitespace() && next != b'@' {
                return true;
            }
        }
    }
    false
}

fn string_contains_identity(s: &str, ctx: &ScrubContext) -> bool {
    for a in &ctx.accounts {
        if !a.is_empty() && s.contains(a) {
            return true;
        }
    }
    for c in &ctx.characters {
        if !c.is_empty() && s.contains(c) {
            return true;
        }
    }
    for id in &ctx.character_ids {
        if !id.is_empty() && s.contains(id) {
            return true;
        }
    }
    false
}

/// Recursive worker. Returns `Some(node)` if the node survives, `None` if it
/// (and its key in the parent) should be dropped.
fn scrub_node(
    node: &SvTreeNode,
    path: &mut Vec<String>,
    ctx: &ScrubContext,
    placeholders: &mut PlaceholderTable,
    report: &mut ScrubReport,
) -> Option<SvTreeNode> {
    // Block the entire subtree if its key triggers a heuristic. We skip this
    // at depth 0 (the synthetic file-root has no meaningful key).
    if !path.is_empty() && key_is_blocked(&node.key) {
        let removed = serialize_to_lua(node).len();
        report.drops.push(DropEntry {
            path: path.clone(),
            reason: drop_reason_for_key(&node.key),
            bytes_removed: removed,
        });
        return None;
    }

    // Apply identity templating to the key itself.
    let mut new_key = node.key.clone();
    if !path.is_empty() {
        if let Some((placeholder, kind)) = placeholders.template_for_key(&node.key, ctx) {
            report.templated_keys.push(TemplateEntry {
                path: path.clone(),
                kind,
                original: node.key.clone(),
                placeholder: placeholder.clone(),
            });
            new_key = placeholder;
        }
    }

    match node.value_type {
        SvValueType::Table => {
            let mut new_children: Vec<SvTreeNode> = Vec::new();
            if let Some(children) = &node.children {
                for child in children {
                    path.push(child.key.clone());
                    if let Some(scrubbed_child) = scrub_node(child, path, ctx, placeholders, report)
                    {
                        new_children.push(scrubbed_child);
                    }
                    path.pop();
                }
            }
            Some(SvTreeNode {
                key: new_key,
                value_type: SvValueType::Table,
                value: None,
                children: Some(new_children),
                raw_lua_value: None,
            })
        }
        SvValueType::String => {
            let s = node.value.as_ref().and_then(|v| v.as_str()).unwrap_or("");
            // Drop string values that embed an identity, rather than templating
            // them — substituting on import would silently change behaviour for
            // addons that use these as semantic config (allowlists, etc.).
            if string_contains_identity(s, ctx) {
                report.drops.push(DropEntry {
                    path: path.clone(),
                    reason: DropReason::StringValueContainsIdentity,
                    bytes_removed: s.len(),
                });
                return None;
            }
            if looks_like_account_handle(s) {
                report.drops.push(DropEntry {
                    path: path.clone(),
                    reason: DropReason::StringValueLooksLikeAccount,
                    bytes_removed: s.len(),
                });
                return None;
            }
            Some(SvTreeNode {
                key: new_key,
                value_type: SvValueType::String,
                value: node.value.clone(),
                children: None,
                raw_lua_value: node.raw_lua_value.clone(),
            })
        }
        // Numbers, booleans, nil — pass through untouched.
        _ => Some(SvTreeNode {
            key: new_key,
            value_type: node.value_type,
            value: node.value.clone(),
            children: None,
            raw_lua_value: node.raw_lua_value.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saved_variables::parser::parse_sv_file;

    fn parse(s: &str) -> SvTreeNode {
        parse_sv_file(s, "test.lua").expect("parse")
    }

    fn ctx() -> ScrubContext {
        ScrubContext {
            accounts: vec!["@Author".to_string()],
            characters: vec!["Mainchar".to_string(), "Alttank".to_string()],
            character_ids: vec!["123456789012345".to_string()],
            extra_worlds: vec![],
        }
    }

    #[test]
    fn templates_account_key() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = {
                            ["enabled"] = true,
                        },
                    },
                },
            }"#,
        );
        let (out, report) = scrub(&tree, &ctx());

        // The `@Author` key should now be `${ACCOUNT}`.
        let serialized = serialize_to_lua(&out);
        assert!(
            serialized.contains("${ACCOUNT}"),
            "expected ${{ACCOUNT}} placeholder in:\n{}",
            serialized
        );
        assert!(
            !serialized.contains("@Author"),
            "raw account handle leaked: {}",
            serialized
        );
        assert!(report
            .templated_keys
            .iter()
            .any(|t| matches!(t.kind, TemplateKind::Account)));
    }

    #[test]
    fn templates_character_keys_with_indices() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["Mainchar"] = { ["x"] = 1 },
                        ["Alttank"] = { ["x"] = 2 },
                    },
                },
            }"#,
        );
        let (out, _) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(serialized.contains("${CHAR:0}"));
        assert!(serialized.contains("${CHAR:1}"));
        assert!(!serialized.contains("Mainchar"));
        assert!(!serialized.contains("Alttank"));
    }

    #[test]
    fn templates_well_known_world_key() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["NA Megaserver"] = {
                        ["@Author"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
        let (out, _) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(serialized.contains("${WORLD}"));
        assert!(!serialized.contains("NA Megaserver"));
    }

    #[test]
    fn drops_blocked_key_heuristic() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["settings"] = { ["foo"] = true },
                ["SalesHistory"] = { ["1"] = "stuff" },
                ["mailQueue"] = { ["1"] = "more" },
            }"#,
        );
        let (out, report) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(serialized.contains("settings"));
        assert!(!serialized.contains("SalesHistory"));
        assert!(!serialized.contains("mailQueue"));

        let dropped: Vec<_> = report
            .drops
            .iter()
            .filter(|d| matches!(d.reason, DropReason::BlockedKeyHeuristic))
            .collect();
        assert_eq!(dropped.len(), 2);
    }

    #[test]
    fn drops_string_value_containing_identity() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["allowlist"] = {
                    ["1"] = "@Author",
                    ["2"] = "@SomebodyElse",
                },
            }"#,
        );
        let (out, report) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(
            !serialized.contains("@Author"),
            "exporter handle leaked: {}",
            serialized
        );
        // The third-party handle should also be dropped via the @-shape rule.
        assert!(
            !serialized.contains("@SomebodyElse"),
            "third-party handle leaked: {}",
            serialized
        );
        assert!(report.drops.iter().any(|d| matches!(
            d.reason,
            DropReason::StringValueContainsIdentity | DropReason::StringValueLooksLikeAccount
        )));
    }

    #[test]
    fn preserves_normal_config() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = {
                            ["enabled"] = true,
                            ["scale"] = 1.25,
                            ["color"] = { ["r"] = 0.8, ["g"] = 0.4, ["b"] = 0.2 },
                            ["mode"] = "compact",
                        },
                    },
                },
            }"#,
        );
        let (out, _) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(serialized.contains("enabled = true"), "{}", serialized);
        assert!(serialized.contains("scale = 1.25"), "{}", serialized);
        assert!(serialized.contains("mode = \"compact\""), "{}", serialized);
        assert!(serialized.contains("r = 0.8"), "{}", serialized);
    }

    #[test]
    fn drops_last_character_name_helper() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$LastCharacterName"] = "Mainchar",
                        ["$AccountWide"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
        let (out, report) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(!serialized.contains("$LastCharacterName"));
        assert!(report
            .drops
            .iter()
            .any(|d| matches!(d.reason, DropReason::AlwaysDropped)));
    }

    #[test]
    fn templates_numeric_character_id_key() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["123456789012345"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
        let (out, _) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(serialized.contains("${CHAR_ID:0}"));
        assert!(!serialized.contains("123456789012345"));
    }

    #[test]
    fn report_records_size_reduction() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                        ["SalesHistory"] = {
                            ["one"] = "lots and lots and lots of historical data here",
                            ["two"] = "more historical data, large enough to matter",
                        },
                    },
                },
            }"#,
        );
        let (_, report) = scrub(&tree, &ctx());
        assert!(report.scrubbed_bytes < report.original_bytes);
        assert!(report.drops.iter().any(|d| d.bytes_removed > 0));
    }

    #[test]
    fn account_handle_shape_is_dropped_even_without_context() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["partner"] = "@SomeRandomPlayer",
            }"#,
        );
        let empty_ctx = ScrubContext::default();
        let (out, report) = scrub(&tree, &empty_ctx);
        let serialized = serialize_to_lua(&out);
        assert!(!serialized.contains("@SomeRandomPlayer"));
        assert!(report
            .drops
            .iter()
            .any(|d| matches!(d.reason, DropReason::StringValueLooksLikeAccount)));
    }
}
