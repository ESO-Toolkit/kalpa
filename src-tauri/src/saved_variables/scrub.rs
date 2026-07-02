//! Identity templating + heuristic value scrubbing for SavedVariables trees.
//!
//! Used as the foundation for `.esopack` v2 settings export: takes a parsed
//! SV tree plus the exporter's identities and returns a templated, scrubbed
//! copy alongside a report listing every drop and substitution.
//!
//! Currently surfaced only via a debug-only Tauri command
//! (`dev_scrub_saved_variable`); production export/import wiring lands in a
//! later change. Remove the module-level `dead_code` allow then.
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
//!   config (allowlists). Substituting an importer's identity would silently
//!   change behaviour, so we drop those leaves and record them in the report.
//!   The `@Handle` shape is detected on its own, so player ignore/whisper
//!   lists holding handles are caught even when their containing key looks
//!   benign.
//! * **Self-mapping helper keys are dropped.** `$LastCharacterName` and
//!   similar helpers are author-local and meaningless on import.

#![cfg_attr(not(debug_assertions), allow(dead_code))]

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

/// Substrings (lowercased) that, when found in a **table** key name, cause
/// the entire subtree to be dropped.
///
/// Only applied to table-typed nodes — scalar leaves (bools, numbers, short
/// strings) are never dropped by this heuristic, because addons often store
/// numeric config like `maxSavedFights = 50` whose key happens to contain
/// "fight". The heuristic is designed to nuke data-heavy *collections*, not
/// individual settings.
///
/// Also skipped at path depth 1 (direct children of the synthetic file root),
/// where the key is an addon's top-level variable name. Those names sometimes
/// embed category words (e.g. `TamrielTradeCentreVars`) that would otherwise
/// wipe the entire addon.
///
/// NB: "ignore" is intentionally absent. The fixture run showed it collides
/// with legitimate ability-ignore-list config (e.g. ADR's `ignoredAbilities`);
/// player-ignore lists are caught by the `@Handle`-in-string-value rule when
/// they actually hold handles.
const BLOCKED_KEY_SUBSTRINGS: &[&str] = &[
    "mail",
    "friend",
    "whisper",
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
    "fight", // CombatMetrics: fightData, fights (table collections)
    "combatlog",
    "logs",
    "events",
    "messages",
    "guildstore",
    "guildhistory",
    "guildbank",
    "guildroster",
    "charidtoname", // IIfA: CharIdToName / CharNameToId lookup tables
    "charnametoid",
    "linestrings", // pChat: LineStrings = per-session chat log
];

/// Exact key names (case-sensitive) that should always be dropped because
/// they encode the exporter's identity in a way templating can't fix.
const ALWAYS_DROPPED_KEYS: &[&str] = &[
    "$LastCharacterName",
    // Srendarr and similar addons store the last-used character name as a
    // plain string value under this key; same semantics as $LastCharacterName.
    "lastCharname",
    // pChat stores character name as a string value inside chatConfSync
    // entries keyed by character ID. The value is always a character name.
    "charName",
];

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
    /// Addon was disabled via an `AddonOverride`.
    OverrideDisabled,
    /// Path was in the `deny_paths` list of an `AddonOverride`.
    OverrideDenyPath,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TemplateKind {
    Account,
    /// Bare account handle (no `@` prefix), e.g. HarvestMap's `["account"]` container.
    AccountName,
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

/// Privacy-safe summary of scrubbing for serialization into `.esopack` files.
/// Contains only counts and sizes — no paths, identities, or original values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrubSummary {
    pub drop_count: usize,
    pub template_count: usize,
    pub original_bytes: usize,
    pub scrubbed_bytes: usize,
}

impl From<&ScrubReport> for ScrubSummary {
    fn from(report: &ScrubReport) -> Self {
        Self {
            drop_count: report.drops.len(),
            template_count: report.templated_keys.len(),
            original_bytes: report.original_bytes,
            scrubbed_bytes: report.scrubbed_bytes,
        }
    }
}

/// Per-addon scrub configuration supplied by the caller.
///
/// Overrides are matched by `addon` (the addon folder name, case-sensitive).
/// `allow_paths` is a list of dot-separated key paths that should survive even
/// if they'd otherwise be dropped by a heuristic (e.g. `"HarvestMap.Default.@Account"`).
/// `deny_paths` is a list of paths that should always be dropped.
///
/// Phase 1 ships with an empty registry; entries are added as real-file testing
/// reveals addon-specific exceptions (e.g. HarvestMap's non-`@` account keys).
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonOverride {
    /// Addon folder name this override applies to (case-sensitive).
    pub addon: String,
    /// If true, scrub produces an empty tree for this addon (opt-out).
    #[serde(default)]
    pub disabled: bool,
    /// Paths to preserve even if a heuristic would drop them (dot-joined keys).
    #[serde(default)]
    pub allow_paths: Vec<String>,
    /// Paths to always drop regardless of other rules (dot-joined keys).
    #[serde(default)]
    pub deny_paths: Vec<String>,
}

/// Run the scrubber with per-addon overrides. Returns the cleaned tree plus a
/// report. The caller should populate `overrides` from a registry built during
/// real-file testing; an empty slice is equivalent to calling [`scrub`].
#[allow(dead_code)]
pub fn scrub_with_overrides(
    tree: &SvTreeNode,
    ctx: &ScrubContext,
    overrides: &[AddonOverride],
) -> (SvTreeNode, ScrubReport) {
    // Determine which top-level variable names are covered by a disabled
    // override, and which paths are explicitly allowed/denied.
    // We look at the tree's direct children (top-level addon variables) and
    // match them against the addon folder via the variable name prefix — not
    // the folder name directly, since the SV file can declare multiple vars.
    // For now the matching is best-effort: if any variable name starts with or
    // equals the addon name (case-insensitive), we apply the override.
    let original_bytes = serialize_to_lua(tree).len();
    let mut report = ScrubReport {
        original_bytes,
        ..ScrubReport::default()
    };
    let mut placeholders = PlaceholderTable::new(ctx);
    let mut path: Vec<String> = Vec::new();

    let children = match &tree.children {
        Some(c) => c,
        None => {
            return (
                SvTreeNode {
                    key: tree.key.clone(),
                    value_type: SvValueType::Table,
                    value: None,
                    children: Some(Vec::new()),
                    raw_lua_value: None,
                },
                report,
            )
        }
    };

    let mut new_children: Vec<SvTreeNode> = Vec::new();
    for top_var in children {
        let var_name = top_var.key.as_str();
        let ov = overrides.iter().find(|o| {
            var_name
                .to_ascii_lowercase()
                .starts_with(&o.addon.to_ascii_lowercase())
        });

        if let Some(ov) = ov {
            if ov.disabled {
                let removed = serialize_to_lua(top_var).len();
                report.drops.push(DropEntry {
                    path: vec![var_name.to_string()],
                    reason: DropReason::OverrideDisabled,
                    bytes_removed: removed,
                });
                continue;
            }
            // Build allow/deny prefix sets for this variable.
            let allow_set: Vec<Vec<String>> = ov
                .allow_paths
                .iter()
                .map(|p| p.split('.').map(str::to_string).collect())
                .collect();
            let deny_set: Vec<Vec<String>> = ov
                .deny_paths
                .iter()
                .map(|p| p.split('.').map(str::to_string).collect())
                .collect();

            path.push(var_name.to_string());
            if let Some(scrubbed) = scrub_node_override(
                top_var,
                &mut path,
                ctx,
                &mut placeholders,
                &mut report,
                &allow_set,
                &deny_set,
            ) {
                new_children.push(scrubbed);
            }
            path.pop();
        } else {
            path.push(var_name.to_string());
            if let Some(scrubbed) =
                scrub_node(top_var, &mut path, ctx, &mut placeholders, &mut report)
            {
                new_children.push(scrubbed);
            }
            path.pop();
        }
    }

    let scrubbed_root = SvTreeNode {
        key: tree.key.clone(),
        value_type: SvValueType::Table,
        value: None,
        children: Some(new_children),
        raw_lua_value: None,
    };
    report.scrubbed_bytes = serialize_to_lua(&scrubbed_root).len();
    (scrubbed_root, report)
}

/// Replace identity placeholders in a Lua string with the importer's real values.
///
/// Substitution order: longer tokens first to avoid `${ACCOUNT}` matching as a
/// prefix of `${ACCOUNT:1}`. Tokens with no mapping in `ctx` are left as-is.
/// `world_names` should be `WELL_KNOWN_WORLDS`; `${WORLD}` maps to
/// `ctx.extra_worlds[0]` if set, otherwise the first well-known world name.
pub fn substitute_placeholders(lua: &str, ctx: &ScrubContext, world_names: &[&str]) -> String {
    let mut pairs: Vec<(String, String)> = Vec::new();

    for (i, account) in ctx.accounts.iter().enumerate() {
        let token = if i == 0 {
            "${ACCOUNT}".to_string()
        } else {
            format!("${{ACCOUNT:{i}}}")
        };
        pairs.push((token, account.clone()));

        // Bare-name token: substitute with the account handle minus its `@` prefix.
        let bare = account.trim_start_matches('@').to_string();
        if !bare.is_empty() {
            let name_token = if i == 0 {
                "${ACCOUNT_NAME}".to_string()
            } else {
                format!("${{ACCOUNT_NAME:{i}}}")
            };
            pairs.push((name_token, bare));
        }
    }
    for (i, character) in ctx.characters.iter().enumerate() {
        pairs.push((format!("${{CHAR:{i}}}"), character.clone()));
    }
    for (i, id) in ctx.character_ids.iter().enumerate() {
        pairs.push((format!("${{CHAR_ID:{i}}}"), id.clone()));
    }
    let world = ctx
        .extra_worlds
        .first()
        .map(|s| s.as_str())
        .or_else(|| world_names.first().copied())
        .unwrap_or("NA Megaserver");
    pairs.push(("${WORLD}".to_string(), world.to_string()));

    // Sort by token length descending so longer tokens match first.
    pairs.sort_by_key(|b| std::cmp::Reverse(b.0.len()));

    let mut result = lua.to_string();
    for (token, replacement) in &pairs {
        result = result.replace(token.as_str(), replacement.as_str());
    }
    result
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
    /// Bare account handles (without `@`) → `${ACCOUNT_NAME:N}` token.
    /// Derived from `accounts`: for each `@Foo`, inserts `Foo` → `${ACCOUNT_NAME:N}`.
    /// Used to template HarvestMap-style `["account"] = { ["Foo"] = {...} }` layouts.
    account_names: std::collections::HashMap<String, String>,
    characters: std::collections::HashMap<String, String>,
    character_ids: std::collections::HashMap<String, String>,
}

impl PlaceholderTable {
    fn new(ctx: &ScrubContext) -> Self {
        let mut accounts = std::collections::HashMap::new();
        let mut account_names = std::collections::HashMap::new();
        for (i, a) in ctx.accounts.iter().enumerate() {
            let label = if i == 0 {
                "${ACCOUNT}".to_string()
            } else {
                format!("${{ACCOUNT:{i}}}")
            };
            accounts.insert(a.clone(), label);

            // Derive bare form: "@Foo" → "Foo" → "${ACCOUNT_NAME:N}"
            let bare = a.trim_start_matches('@');
            if !bare.is_empty() {
                let name_label = if i == 0 {
                    "${ACCOUNT_NAME}".to_string()
                } else {
                    format!("${{ACCOUNT_NAME:{i}}}")
                };
                account_names.insert(bare.to_string(), name_label);
            }
        }
        let mut characters = std::collections::HashMap::new();
        for (i, c) in ctx.characters.iter().enumerate() {
            characters.insert(c.clone(), format!("${{CHAR:{i}}}"));
        }
        let mut character_ids = std::collections::HashMap::new();
        for (i, id) in ctx.character_ids.iter().enumerate() {
            character_ids.insert(id.clone(), format!("${{CHAR_ID:{i}}}"));
        }
        Self {
            accounts,
            account_names,
            characters,
            character_ids,
        }
    }

    fn template_for_key(&self, key: &str, ctx: &ScrubContext) -> Option<(String, TemplateKind)> {
        if let Some(p) = self.accounts.get(key) {
            return Some((p.clone(), TemplateKind::Account));
        }
        // Characters and character IDs are checked before bare account names.
        // A player named "Author" (char) takes priority over the bare form of
        // account "@Author" — character keys appear far more frequently in the
        // standard SV layout and the bare-name match is only needed for niche
        // layouts like HarvestMap's ["account"] container.
        if let Some(p) = self.characters.get(key) {
            return Some((p.clone(), TemplateKind::Character));
        }
        if let Some(p) = self.character_ids.get(key) {
            return Some((p.clone(), TemplateKind::CharacterId));
        }
        if let Some(p) = self.account_names.get(key) {
            return Some((p.clone(), TemplateKind::AccountName));
        }
        if WELL_KNOWN_WORLDS.contains(&key) || ctx.extra_worlds.iter().any(|w| w == key) {
            return Some(("${WORLD}".to_string(), TemplateKind::World));
        }
        None
    }
}

/// Returns true if the key matches a `BLOCKED_KEY_SUBSTRINGS` heuristic.
/// Does NOT check `ALWAYS_DROPPED_KEYS` — the caller handles those separately
/// so they can be applied regardless of node type or depth.
fn key_is_heuristic_blocked(key: &str) -> bool {
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

/// Walk a parsed SV tree and infer the exporter's identities by recognising
/// the canonical SavedVariables shape ESO produces.
///
/// Handles four observed layouts:
///
/// ```text
/// -- Standard (ZO_SavedVars:New)
/// MyAddon_SV = {
///     ["Default"] = {
///         ["@AccountHandle"] = {         -- account
///             ["$AccountWide"] = ...
///             ["CharacterName"] = ...    -- character
///         },
///     },
/// }
///
/// -- World-scoped (ZO_SavedVars:NewAccountWide with GetWorldName())
/// MyAddon_SV = {
///     ["Default"] = {
///         ["NA Megaserver"] = {          -- world layer
///             ["@AccountHandle"] = { ... },
///         },
///     },
/// }
///
/// -- World-first, no Default (pChat)
/// PCHAT_OPTS = {
///     ["NA Megaserver"] = {              -- world layer at depth 2
///         ["@AccountHandle"] = { ... },
///     },
/// }
///
/// -- Direct account key under top-level var (IIfA, some others)
/// IIfA_Data = {
///     ["Default"] = { ["@Primary"] = { ... } },
///     ["@Secondary"] = { ... },         -- extra account at depth 2
/// }
/// ```
///
/// Anything outside these shapes is treated as addon config and not inspected,
/// so config keys are never mis-detected as identities.
pub fn detect_identities_from_tree(tree: &SvTreeNode) -> ScrubContext {
    let mut acc = DetectAcc::default();

    if let Some(top_levels) = tree_children(tree) {
        for top in top_levels {
            let top_children = match tree_children(top) {
                Some(c) => c,
                None => continue,
            };
            for layer in top_children {
                classify_under_top(layer, &mut acc);
            }
        }
    }

    acc.into_context()
}

#[derive(Default)]
struct DetectAcc {
    accounts: std::collections::BTreeSet<String>,
    characters: std::collections::BTreeSet<String>,
    character_ids: std::collections::BTreeSet<String>,
    extra_worlds: std::collections::BTreeSet<String>,
}

impl DetectAcc {
    fn into_context(self) -> ScrubContext {
        ScrubContext {
            accounts: self.accounts.into_iter().collect(),
            characters: self.characters.into_iter().collect(),
            character_ids: self.character_ids.into_iter().collect(),
            extra_worlds: self.extra_worlds.into_iter().collect(),
        }
    }
}

fn tree_children(node: &SvTreeNode) -> Option<&Vec<SvTreeNode>> {
    if !matches!(node.value_type, SvValueType::Table) {
        return None;
    }
    node.children.as_ref()
}

/// `node` is a direct child of a top-level addon variable. It may be:
///   - `"Default"` (standard layout — recurse into its children)
///   - `"@AccountHandle"` (account key at depth 2, e.g. IIfA secondary account)
///   - A world name (pChat world-first layout)
///   - Something else (addon config — skip)
fn classify_under_top(node: &SvTreeNode, acc: &mut DetectAcc) {
    let key = node.key.as_str();
    if key == "Default" {
        // Standard layout: Default → (world? | account) → characters.
        if let Some(children) = tree_children(node) {
            for child in children {
                classify_account_or_world(child, acc);
            }
        }
    } else if key.starts_with('@') {
        // Account key sitting directly under the addon variable (no Default
        // wrapper). Record the account but do NOT inspect children for
        // character names — at this depth the children are addon section keys
        // (e.g. "settings", "servers"), not ESO character names.
        // Only collect numeric character IDs, which are unambiguous.
        acc.accounts.insert(key.to_string());
        if let Some(children) = tree_children(node) {
            for child in children {
                let k = child.key.as_str();
                if !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) && k.len() >= 10 {
                    acc.character_ids.insert(k.to_string());
                }
            }
        }
    } else if WELL_KNOWN_WORLDS.contains(&key) || key.contains(' ') {
        // World-first layout (pChat): world layer at depth 2, accounts below.
        classify_world_layer(key, node, acc);
    }
}

/// `node` is under `Default` — either a world name or an account handle.
fn classify_account_or_world(node: &SvTreeNode, acc: &mut DetectAcc) {
    let key = node.key.as_str();
    if key.starts_with('@') {
        acc.accounts.insert(key.to_string());
        if let Some(children) = tree_children(node) {
            for child in children {
                classify_under_account(child, acc);
            }
        }
    } else if WELL_KNOWN_WORLDS.contains(&key) || key.contains(' ') {
        classify_world_layer(key, node, acc);
    }
}

/// `node` is a world-name layer; its children should be account handles.
fn classify_world_layer(key: &str, node: &SvTreeNode, acc: &mut DetectAcc) {
    if !WELL_KNOWN_WORLDS.contains(&key) {
        acc.extra_worlds.insert(key.to_string());
    }
    if let Some(children) = tree_children(node) {
        for child in children {
            if child.key.starts_with('@') {
                acc.accounts.insert(child.key.clone());
                if let Some(grand) = tree_children(child) {
                    for g in grand {
                        classify_under_account(g, acc);
                    }
                }
            }
        }
    }
}

/// `node` is something sitting directly under an account handle. ESO uses
/// either `$AccountWide` (account-wide subtable) or a character name / ID
/// here.
fn classify_under_account(node: &SvTreeNode, acc: &mut DetectAcc) {
    let key = node.key.as_str();
    if key.starts_with('$') {
        // $AccountWide and friends — markers, not identities.
        return;
    }
    if !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()) {
        if key.len() >= 10 {
            acc.character_ids.insert(key.to_string());
        }
        return;
    }
    if !key.is_empty() {
        acc.characters.insert(key.to_string());
    }
}

/// A character discovered for the Characters roster, with its megaserver when
/// the layout makes it derivable.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterCharacter {
    /// The verbatim SavedVariables key (may carry a raw `^Mx` caret suffix).
    pub name: String,
    /// "NA Megaserver" / "EU Megaserver" / "PTS" when stored under a world-scoped
    /// layout; `None` for the account-keyed layout that omits the world.
    pub world: Option<String>,
}

/// Extract ESO characters for the **Characters roster** from a SavedVariables
/// tree.
///
/// This is intentionally stricter than [`detect_identities_from_tree`], which
/// over-collects on purpose (any non-marker key under an account is templated so
/// scrubbing never leaks an identity). The roster is user-facing and feeds
/// backups, so a false positive here would surface an addon config section as a
/// fake character. A key under an account handle is accepted only when it is a
/// **table** value and *looks like* an ESO character name (uppercase-led;
/// letters, spaces, hyphens, apostrophes). Real character names always satisfy
/// this, so none are lost, while config tables an addon stores beside characters
/// (`settings`, `profiles`, `guilds`) are rejected — even when a `$AccountWide`
/// marker is present. Numeric character-IDs and `$`-markers are skipped.
///
/// The world (when present) only labels the megaserver, never proves a key.
///
/// Returns verbatim keys (caret suffix preserved) so callers can both display a
/// cleaned name and match the raw key on disk.
///
/// Retained as the **differential-testing oracle** for the production roster
/// path, which now streams the raw bytes via
/// [`crate::saved_variables::roster_stream`]. A parity test asserts the streamer
/// yields the identical `(name, world)` set on every fixture, so this tree-based
/// detector is the executable spec the streamer must match. It is compiled only
/// for tests.
#[cfg(test)]
pub fn detect_roster_characters_from_tree(tree: &SvTreeNode) -> Vec<RosterCharacter> {
    let mut out: Vec<RosterCharacter> = Vec::new();
    let mut seen: std::collections::BTreeSet<(String, Option<String>)> =
        std::collections::BTreeSet::new();

    if let Some(top_levels) = tree_children(tree) {
        for top in top_levels {
            if let Some(layers) = tree_children(top) {
                for layer in layers {
                    roster_under_top(layer, &mut out, &mut seen);
                }
            }
        }
    }

    out
}

#[cfg(test)]
type RosterSeen = std::collections::BTreeSet<(String, Option<String>)>;

#[cfg(test)]
fn roster_under_top(node: &SvTreeNode, out: &mut Vec<RosterCharacter>, seen: &mut RosterSeen) {
    let key = node.key.as_str();
    if key == "Default" {
        if let Some(children) = tree_children(node) {
            for child in children {
                roster_account_or_world(child, out, seen);
            }
        }
    } else if WELL_KNOWN_WORLDS.contains(&key) || key.contains(' ') {
        // World-first layout (pChat): world layer directly under the addon var.
        roster_world_layer(key, node, out, seen);
    }
    // An account handle sitting directly under the addon variable (no `Default`)
    // holds addon section keys at this depth, not character names — skip it.
}

#[cfg(test)]
fn roster_account_or_world(
    node: &SvTreeNode,
    out: &mut Vec<RosterCharacter>,
    seen: &mut RosterSeen,
) {
    let key = node.key.as_str();
    if key.starts_with('@') {
        roster_chars_under_account(node, None, out, seen);
    } else if WELL_KNOWN_WORLDS.contains(&key) || key.contains(' ') {
        roster_world_layer(key, node, out, seen);
    }
}

#[cfg(test)]
fn roster_world_layer(
    world_key: &str,
    node: &SvTreeNode,
    out: &mut Vec<RosterCharacter>,
    seen: &mut RosterSeen,
) {
    // Only attribute a real megaserver; a non-canonical custom world stays None.
    let world = if WELL_KNOWN_WORLDS.contains(&world_key) {
        Some(world_key)
    } else {
        None
    };
    if let Some(children) = tree_children(node) {
        for child in children {
            if child.key.starts_with('@') {
                roster_chars_under_account(child, world, out, seen);
            }
        }
    }
}

/// Common addon config-section keys that pass the character-name shape check but
/// are not characters. Compared case-insensitively against the whole key.
const CONFIG_SECTION_KEYS: &[&str] = &[
    "settings",
    "setting",
    "profile",
    "profiles",
    "options",
    "option",
    "config",
    "configuration",
    "data",
    "global",
    "globals",
    "default",
    "defaults",
    "general",
    "account",
    "accountwide",
    "preferences",
    "version",
    "server",
    "servers",
];

/// Does `key` have the shape of an ESO character name? ESO names start with an
/// uppercase letter and contain only letters, spaces, hyphens, and apostrophes.
/// A raw `^Mx` caret suffix is allowed (the part before it is checked). Common
/// capitalized config-section words (`Settings`, `Profile`, …) are rejected even
/// though they match the shape. This is the discriminator used to recover
/// characters from accounts without admitting addon config keys.
///
/// Exposed to the crate so the streaming roster extractor
/// (`roster_stream`) applies the EXACT same acceptance rule as the
/// tree-based detector, keeping the two in parity.
pub(crate) fn looks_like_character_name(key: &str) -> bool {
    let base = key.split('^').next().unwrap_or(key);
    let mut chars = base.chars();
    match chars.next() {
        Some(c) if c.is_uppercase() => {}
        _ => return false,
    }
    let len = base.chars().count();
    if len == 0 || len > 32 {
        return false;
    }
    if !base
        .chars()
        .all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\'' | '\u{2019}'))
    {
        return false;
    }
    // Reject well-known config-section names that happen to look like a name.
    let lowered = base.to_lowercase();
    !CONFIG_SECTION_KEYS.contains(&lowered.as_str())
}

#[cfg(test)]
fn roster_chars_under_account(
    account_node: &SvTreeNode,
    world: Option<&str>,
    out: &mut Vec<RosterCharacter>,
    seen: &mut RosterSeen,
) {
    let children = match tree_children(account_node) {
        Some(c) => c,
        None => return,
    };
    for child in children {
        let key = child.key.as_str();
        if key.is_empty() || key.starts_with('$') {
            continue;
        }
        if key.bytes().all(|b| b.is_ascii_digit()) {
            // Numeric characterId — no display name lives at this key.
            continue;
        }
        if !matches!(child.value_type, SvValueType::Table) {
            // Scalar config (version flags, etc.) is never a character.
            continue;
        }
        // Require the key to look like an ESO character name. Real names always
        // do (uppercase-led, letters/space/hyphen/apostrophe), so this loses no
        // characters, while it excludes config-section tables (`settings`,
        // `profiles`, `guilds`) that addons store alongside characters — even
        // under an account that also has a `$AccountWide` marker.
        if !looks_like_character_name(key) {
            continue;
        }
        let entry = RosterCharacter {
            name: key.to_string(),
            world: world.map(|w| w.to_string()),
        };
        if seen.insert((entry.name.clone(), entry.world.clone())) {
            out.push(entry);
        }
    }
}

/// Recursive worker. Returns `Some(node)` if the node survives, `None` if it
/// (and its key in the parent) should be dropped.
///
/// `path` tracks the current key path from the root (empty = synthetic root,
/// length 1 = addon variable name, length 2+ = data inside the addon).
fn scrub_node(
    node: &SvTreeNode,
    path: &mut Vec<String>,
    ctx: &ScrubContext,
    placeholders: &mut PlaceholderTable,
    report: &mut ScrubReport,
) -> Option<SvTreeNode> {
    if path.len() > 512 {
        return Some(node.clone());
    }

    // Block subtrees whose key name suggests sensitive data.
    //
    // Two constraints:
    // 1. Depth 1 is skipped — those keys are addon variable names (e.g.
    //    `TamrielTradeCentreVars`), not data categories.
    // 2. Only table nodes are blocked — scalar leaves like `maxSavedFights =
    //    50` should survive even when their key contains "fight". The heuristic
    //    is designed to drop data *collections*, not individual config values.
    let is_table = matches!(node.value_type, SvValueType::Table);
    let is_always_dropped = !path.is_empty() && ALWAYS_DROPPED_KEYS.contains(&node.key.as_str());
    let is_heuristic_blocked = path.len() >= 2 && is_table && key_is_heuristic_blocked(&node.key);

    if is_always_dropped || is_heuristic_blocked {
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

/// Like `scrub_node` but also checks per-addon allow/deny path lists.
#[allow(dead_code)]
fn scrub_node_override(
    node: &SvTreeNode,
    path: &mut Vec<String>,
    ctx: &ScrubContext,
    placeholders: &mut PlaceholderTable,
    report: &mut ScrubReport,
    allow_set: &[Vec<String>],
    deny_set: &[Vec<String>],
) -> Option<SvTreeNode> {
    if path.len() > 512 {
        return Some(node.clone());
    }

    // Check deny paths first — explicit deny wins.
    if deny_set
        .iter()
        .any(|deny| path.starts_with(deny.as_slice()))
    {
        let removed = serialize_to_lua(node).len();
        report.drops.push(DropEntry {
            path: path.clone(),
            reason: DropReason::OverrideDenyPath,
            bytes_removed: removed,
        });
        return None;
    }

    // Check if path is explicitly allowed — skip all heuristics if so.
    let explicitly_allowed = allow_set
        .iter()
        .any(|allow| path.starts_with(allow.as_slice()));

    if explicitly_allowed {
        // Pass through with identity templating only (no heuristic drops).
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
        return match node.value_type {
            SvValueType::Table => {
                let mut new_children = Vec::new();
                if let Some(children) = &node.children {
                    for child in children {
                        path.push(child.key.clone());
                        if let Some(c) = scrub_node_override(
                            child,
                            path,
                            ctx,
                            placeholders,
                            report,
                            allow_set,
                            deny_set,
                        ) {
                            new_children.push(c);
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
            _ => Some(SvTreeNode {
                key: new_key,
                value_type: node.value_type,
                value: node.value.clone(),
                children: None,
                raw_lua_value: node.raw_lua_value.clone(),
            }),
        };
    }

    // Not in allow or deny — fall back to normal scrub, but propagate
    // allow/deny into children.
    let is_table = matches!(node.value_type, SvValueType::Table);
    let is_always_dropped = !path.is_empty() && ALWAYS_DROPPED_KEYS.contains(&node.key.as_str());
    let is_heuristic_blocked = path.len() >= 2 && is_table && key_is_heuristic_blocked(&node.key);

    if is_always_dropped || is_heuristic_blocked {
        let removed = serialize_to_lua(node).len();
        report.drops.push(DropEntry {
            path: path.clone(),
            reason: drop_reason_for_key(&node.key),
            bytes_removed: removed,
        });
        return None;
    }

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
            let mut new_children = Vec::new();
            if let Some(children) = &node.children {
                for child in children {
                    path.push(child.key.clone());
                    if let Some(c) = scrub_node_override(
                        child,
                        path,
                        ctx,
                        placeholders,
                        report,
                        allow_set,
                        deny_set,
                    ) {
                        new_children.push(c);
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
        _ => Some(SvTreeNode {
            key: new_key,
            value_type: node.value_type,
            value: node.value.clone(),
            children: None,
            raw_lua_value: node.raw_lua_value.clone(),
        }),
    }
}

/// Walk a scrubbed SV tree and strip per-character subtrees under each
/// account-handle key. Keys starting with `${CHAR` or `${CHAR_ID` (the
/// templated forms of character names and numeric character IDs) are dropped;
/// everything else — `$AccountWide`, addon-specific root keys, scalar values,
/// etc. — is preserved.
///
/// Must be called **after** [`scrub`] because account keys will already be
/// templated to `${ACCOUNT}` / `${ACCOUNT:N}` and world keys to `${WORLD}`.
/// The checks here recognise both raw (`@Author`) and templated forms.
pub fn strip_per_character_data(tree: &SvTreeNode) -> SvTreeNode {
    fn filter_account_node(node: &SvTreeNode) -> SvTreeNode {
        let new_children = node
            .children
            .as_ref()
            .map(|children| {
                children
                    .iter()
                    .filter(|child| {
                        !child.key.starts_with("${CHAR") && !child.key.starts_with("${CHAR_ID")
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        SvTreeNode {
            key: node.key.clone(),
            value_type: SvValueType::Table,
            value: None,
            children: Some(new_children),
            raw_lua_value: None,
        }
    }

    fn filter_top_var(node: &SvTreeNode) -> SvTreeNode {
        let new_children = node
            .children
            .as_ref()
            .map(|children| {
                children
                    .iter()
                    .map(|child| {
                        if child.key == "Default"
                            || child.key.contains(' ')
                            || child.key == "${WORLD}"
                        {
                            // Default or world layer — recurse one more level
                            let inner = child
                                .children
                                .as_ref()
                                .map(|gc| {
                                    gc.iter()
                                        .map(|gchild| {
                                            if gchild.key.starts_with('@')
                                                || gchild.key.starts_with("${ACCOUNT")
                                            {
                                                filter_account_node(gchild)
                                            } else if gchild.key.contains(' ')
                                                || gchild.key == "${WORLD}"
                                            {
                                                // world under Default — recurse
                                                let accounts = gchild
                                                    .children
                                                    .as_ref()
                                                    .map(|ac| {
                                                        ac.iter()
                                                            .filter(|a| {
                                                                a.key.starts_with('@')
                                                                    || a.key
                                                                        .starts_with("${ACCOUNT")
                                                            })
                                                            .map(filter_account_node)
                                                            .collect::<Vec<_>>()
                                                    })
                                                    .unwrap_or_default();
                                                SvTreeNode {
                                                    key: gchild.key.clone(),
                                                    value_type: SvValueType::Table,
                                                    value: None,
                                                    children: Some(accounts),
                                                    raw_lua_value: None,
                                                }
                                            } else {
                                                gchild.clone()
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            SvTreeNode {
                                key: child.key.clone(),
                                value_type: SvValueType::Table,
                                value: None,
                                children: Some(inner),
                                raw_lua_value: None,
                            }
                        } else if child.key.starts_with('@') || child.key.starts_with("${ACCOUNT") {
                            // Account key directly under top var (no Default wrapper)
                            filter_account_node(child)
                        } else {
                            child.clone()
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        SvTreeNode {
            key: node.key.clone(),
            value_type: SvValueType::Table,
            value: None,
            children: Some(new_children),
            raw_lua_value: None,
        }
    }

    let new_children = tree
        .children
        .as_ref()
        .map(|children| children.iter().map(filter_top_var).collect::<Vec<_>>())
        .unwrap_or_default();
    SvTreeNode {
        key: tree.key.clone(),
        value_type: SvValueType::Table,
        value: None,
        children: Some(new_children),
        raw_lua_value: None,
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
            "expected ${{ACCOUNT}} placeholder in:\n{serialized}"
        );
        assert!(
            !serialized.contains("@Author"),
            "raw account handle leaked: {serialized}"
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
            "exporter handle leaked: {serialized}"
        );
        // The third-party handle should also be dropped via the @-shape rule.
        assert!(
            !serialized.contains("@SomebodyElse"),
            "third-party handle leaked: {serialized}"
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
        assert!(serialized.contains("[\"enabled\"] = true"), "{}", serialized);
        assert!(serialized.contains("[\"scale\"] = 1.25"), "{}", serialized);
        assert!(
            serialized.contains("[\"mode\"] = \"compact\""),
            "{}",
            serialized
        );
        assert!(serialized.contains("[\"r\"] = 0.8"), "{}", serialized);
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

    #[test]
    fn ignored_abilities_config_survives() {
        // Regression: the fixture run showed that a generic "ignore" blocklist
        // entry nukes legitimate ability-ignore-list config (ADR's
        // `ignoredAbilities`). That entry has been removed; the @-handle
        // string-value heuristic still catches social ignore lists that
        // actually hold handles.
        let tree = parse(
            r#"ADR_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = {
                            ["ignoredAbilities"] = {
                                ["1"] = "Critical Surge",
                                ["2"] = "Resolving Vigor",
                            },
                        },
                    },
                },
            }"#,
        );
        let (out, _) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(
            serialized.contains("ignoredAbilities"),
            "ability ignore list should not be dropped: {serialized}"
        );
        assert!(serialized.contains("Critical Surge"));
    }

    #[test]
    fn ignore_list_holding_handles_still_drops_the_handles() {
        // Player ignore lists are typically `@Handle` strings, which the
        // string-value heuristic catches even when the surrounding key
        // doesn't trigger any rule.
        let tree = parse(
            r#"SocialAddon_SV = {
                ["ignoreList"] = {
                    ["1"] = "@SomeJerk",
                    ["2"] = "@AnotherOne",
                },
            }"#,
        );
        let (out, _) = scrub(&tree, &ScrubContext::default());
        let serialized = serialize_to_lua(&out);
        assert!(!serialized.contains("@SomeJerk"));
        assert!(!serialized.contains("@AnotherOne"));
    }

    #[test]
    fn detect_identities_finds_account_and_characters() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                        ["Mainchar"] = { ["x"] = 1 },
                        ["Alttank"] = { ["x"] = 2 },
                    },
                },
            }"#,
        );
        let detected = detect_identities_from_tree(&tree);
        assert_eq!(detected.accounts, vec!["@Author".to_string()]);
        assert!(detected.characters.contains(&"Mainchar".to_string()));
        assert!(detected.characters.contains(&"Alttank".to_string()));
        assert!(detected.character_ids.is_empty());
    }

    #[test]
    fn detect_identities_handles_world_layer() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["NA Megaserver"] = {
                        ["@Author"] = {
                            ["$AccountWide"] = { ["enabled"] = true },
                            ["Mainchar"] = { ["x"] = 1 },
                        },
                    },
                },
            }"#,
        );
        let detected = detect_identities_from_tree(&tree);
        assert_eq!(detected.accounts, vec!["@Author".to_string()]);
        assert_eq!(detected.characters, vec!["Mainchar".to_string()]);
    }

    #[test]
    fn roster_detects_account_keyed_characters() {
        // Standard layout: $AccountWide sibling proves these are characters.
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                        ["Mainchar"] = { ["x"] = 1 },
                        ["Alttank"] = { ["x"] = 2 },
                    },
                },
            }"#,
        );
        let roster = detect_roster_characters_from_tree(&tree);
        let names: Vec<&str> = roster.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Mainchar"));
        assert!(names.contains(&"Alttank"));
        assert!(roster.iter().all(|c| c.world.is_none()));
    }

    #[test]
    fn roster_maps_world_scoped_server() {
        // World layer under Default: the megaserver is derivable, and the
        // $AccountWide marker proves these are characters.
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["EU Megaserver"] = {
                        ["@Author"] = {
                            ["$AccountWide"] = { ["enabled"] = true },
                            ["Mainchar"] = { ["x"] = 1 },
                        },
                    },
                },
            }"#,
        );
        let roster = detect_roster_characters_from_tree(&tree);
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].name, "Mainchar");
        assert_eq!(roster[0].world.as_deref(), Some("EU Megaserver"));
    }

    #[test]
    fn roster_excludes_world_scoped_config_without_marker() {
        // World/account-scoped config tables (no $-marker) must NOT become
        // characters just because they sit under a megaserver layer.
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["NA Megaserver"] = {
                        ["@Author"] = {
                            ["guilds"] = { ["g1"] = true },
                            ["profiles"] = { ["p1"] = true },
                        },
                    },
                },
            }"#,
        );
        assert!(detect_roster_characters_from_tree(&tree).is_empty());
    }

    #[test]
    fn roster_recovers_markerless_character_names() {
        // No $-marker, but the keys look like ESO character names — recover them
        // (an addon storing only per-character data in account-wide mode).
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["Mainchar"] = { ["x"] = 1 },
                        ["Alt Ego"] = { ["x"] = 2 },
                    },
                },
            }"#,
        );
        let roster = detect_roster_characters_from_tree(&tree);
        let names: Vec<&str> = roster.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Mainchar"));
        assert!(names.contains(&"Alt Ego"));
    }

    #[test]
    fn roster_excludes_config_siblings_in_marked_account() {
        // $AccountWide marker present, but config tables sit beside the character.
        // Only the name-shaped key is a character.
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["x"] = 1 },
                        ["Mainchar"] = { ["x"] = 1 },
                        ["settings"] = { ["volume"] = 5 },
                        ["profiles"] = { ["p1"] = true },
                    },
                },
            }"#,
        );
        let roster = detect_roster_characters_from_tree(&tree);
        let names: Vec<&str> = roster.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Mainchar"]);
    }

    #[test]
    fn roster_excludes_capitalized_config_siblings() {
        // Capitalized config-section tables match the name shape but are not
        // characters; only the real character is kept.
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["x"] = 1 },
                        ["Mainchar"] = { ["x"] = 1 },
                        ["Settings"] = { ["volume"] = 5 },
                        ["Profile"] = { ["p"] = 1 },
                        ["Servers"] = { ["na"] = true },
                    },
                },
            }"#,
        );
        let roster = detect_roster_characters_from_tree(&tree);
        let names: Vec<&str> = roster.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Mainchar"]);
    }

    #[test]
    fn roster_excludes_config_section_without_marker() {
        // No $-marker sibling and not under a world: a config section under
        // @account must NOT surface as a character.
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["settings"] = { ["volume"] = 5 },
                        ["servers"] = { ["NA"] = true },
                    },
                },
            }"#,
        );
        assert!(detect_roster_characters_from_tree(&tree).is_empty());
    }

    #[test]
    fn roster_excludes_scalar_and_numeric_keys() {
        // Scalar config values and numeric characterIds are never roster names,
        // even when a $-marker proves the account is a character container.
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                        ["version"] = 3,
                        ["123456789012345"] = { ["x"] = 1 },
                        ["Realchar"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
        let roster = detect_roster_characters_from_tree(&tree);
        let names: Vec<&str> = roster.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Realchar"]);
    }

    #[test]
    fn detect_identities_finds_numeric_character_ids() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["123456789012345"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
        let detected = detect_identities_from_tree(&tree);
        assert_eq!(detected.accounts, vec!["@Author".to_string()]);
        assert_eq!(detected.character_ids, vec!["123456789012345".to_string()]);
        assert!(detected.characters.is_empty());
    }

    #[test]
    fn detect_then_scrub_round_trip() {
        // End-to-end: the detector's output should fully scrub a tree without
        // the caller having to know identities up front.
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["Mainchar"] = { ["pos"] = 5 },
                    },
                },
            }"#,
        );
        let detected = detect_identities_from_tree(&tree);
        let (out, _) = scrub(&tree, &detected);
        let serialized = serialize_to_lua(&out);
        assert!(!serialized.contains("@Author"));
        assert!(!serialized.contains("Mainchar"));
        assert!(serialized.contains("${ACCOUNT}"));
        assert!(serialized.contains("${CHAR:0}"));
    }

    #[test]
    fn drops_last_charname_helper() {
        // Regression: Srendarr stores lastCharname = "CharName" per-character.
        let tree = parse(
            r#"SrendarrDB = {
                ["Default"] = {
                    ["@Author"] = {
                        ["123456789012345"] = {
                            ["lastCharname"] = "Mainchar",
                            ["enabled"] = true,
                        },
                    },
                },
            }"#,
        );
        let (out, report) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(
            !serialized.contains("Mainchar"),
            "character name leaked via lastCharname: {serialized}"
        );
        assert!(serialized.contains("[\"enabled\"] = true"));
        assert!(report
            .drops
            .iter()
            .any(|d| matches!(d.reason, DropReason::AlwaysDropped)));
    }

    #[test]
    fn top_level_var_name_with_trade_not_wiped() {
        // Regression: TamrielTradeCentreVars contains "trade" in its name.
        // The depth-1 skip should prevent the entire addon from being dropped.
        let tree = parse(
            r#"TamrielTradeCentreVars = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = {
                            ["version"] = 3,
                            ["showTooltip"] = true,
                        },
                    },
                },
            }"#,
        );
        let (out, report) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(
            serialized.contains("showTooltip"),
            "config wiped by top-level var name heuristic: {serialized}"
        );
        assert!(
            !report.drops.iter().any(|d| d.path.len() == 1),
            "depth-1 node should never be dropped by heuristic"
        );
    }

    #[test]
    fn scalar_fight_config_survives_table_fight_data_drops() {
        // Regression: CombatMetrics stores maxSavedFights = 50 (scalar) and
        // a fightData table. The scalar should survive; the table should drop.
        let tree = parse(
            r#"CombatMetrics_Save = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = {
                            ["maxSavedFights"] = 50,
                            ["keepbossfights"] = false,
                            ["fightData"] = {
                                [1] = { ["dps"] = 50000, ["log"] = "big data" },
                            },
                        },
                    },
                },
            }"#,
        );
        let (out, report) = scrub(&tree, &ctx());
        let serialized = serialize_to_lua(&out);
        assert!(
            serialized.contains("maxSavedFights"),
            "scalar fight config dropped: {serialized}"
        );
        assert!(
            serialized.contains("keepbossfights"),
            "scalar fight config dropped: {serialized}"
        );
        assert!(
            !serialized.contains("fightData"),
            "fight data table should be dropped: {serialized}"
        );
        assert!(report
            .drops
            .iter()
            .any(|d| matches!(d.reason, DropReason::BlockedKeyHeuristic)));
    }

    #[test]
    fn detect_identities_world_first_no_default() {
        // pChat layout: world key sits directly under the addon var (no Default).
        let tree = parse(
            r#"PCHAT_OPTS = {
                ["NA Megaserver"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                        ["Mainchar"] = { ["x"] = 1 },
                    },
                },
            }"#,
        );
        let detected = detect_identities_from_tree(&tree);
        assert!(
            detected.accounts.contains(&"@Author".to_string()),
            "account not detected in world-first layout: {:?}",
            detected.accounts
        );
        assert!(
            detected.characters.contains(&"Mainchar".to_string()),
            "character not detected in world-first layout"
        );
    }

    #[test]
    fn detect_identities_direct_account_under_top_var() {
        // IIfA layout: account key appears directly under the top-level variable
        // (without a Default wrapper), as a secondary account entry.
        let tree = parse(
            r#"IIfA_Data = {
                ["Default"] = {
                    ["@Primary"] = { ["settings"] = { ["x"] = 1 } },
                },
                ["@Secondary"] = {
                    ["settings"] = { ["y"] = 2 },
                },
            }"#,
        );
        let detected = detect_identities_from_tree(&tree);
        assert!(
            detected.accounts.contains(&"@Primary".to_string()),
            "primary account not detected: {:?}",
            detected.accounts
        );
        assert!(
            detected.accounts.contains(&"@Secondary".to_string()),
            "secondary account (no Default wrapper) not detected: {:?}",
            detected.accounts
        );
    }

    #[test]
    fn scrub_with_overrides_disabled_drops_entire_addon() {
        let tree = parse(
            r#"HarvestMap_Data = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["nodes"] = { [1] = { ["x"] = 0.5 } } },
                    },
                },
            }"#,
        );
        let ov = AddonOverride {
            addon: "HarvestMap".to_string(),
            disabled: true,
            ..AddonOverride::default()
        };
        let (out, report) = scrub_with_overrides(&tree, &ctx(), &[ov]);
        let serialized = serialize_to_lua(&out);
        // The disabled addon's variable should not appear in output.
        assert!(
            !serialized.contains("HarvestMap_Data"),
            "disabled addon still present: {serialized}"
        );
        assert!(report
            .drops
            .iter()
            .any(|d| matches!(d.reason, DropReason::OverrideDisabled)));
    }

    #[test]
    fn scrub_with_overrides_deny_path_drops_subtree() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = {
                            ["keepConfig"] = true,
                            ["sensitiveTable"] = { ["x"] = 1 },
                        },
                    },
                },
            }"#,
        );
        let ov = AddonOverride {
            addon: "MyAddon".to_string(),
            deny_paths: vec![
                "MyAddon_SV.Default.${ACCOUNT}.$AccountWide.sensitiveTable".to_string()
            ],
            ..AddonOverride::default()
        };
        // Note: deny_path matching uses the literal path *before* templating.
        // This test checks that deny_paths work when the path matches exactly.
        let ov_literal = AddonOverride {
            addon: "MyAddon".to_string(),
            deny_paths: vec!["MyAddon_SV.Default.@Author.$AccountWide.sensitiveTable".to_string()],
            ..AddonOverride::default()
        };
        let (out, report) = scrub_with_overrides(&tree, &ctx(), &[ov_literal]);
        let serialized = serialize_to_lua(&out);
        assert!(
            serialized.contains("keepConfig"),
            "allowed key was dropped: {serialized}"
        );
        assert!(
            !serialized.contains("sensitiveTable"),
            "denied key survived: {serialized}"
        );
        // Suppress unused `ov` warning from above.
        let _ = ov;
        assert!(report
            .drops
            .iter()
            .any(|d| matches!(d.reason, DropReason::OverrideDenyPath)));
    }

    #[test]
    fn scrub_with_empty_overrides_equals_scrub() {
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                    },
                },
            }"#,
        );
        let (plain_out, _) = scrub(&tree, &ctx());
        let (override_out, _) = scrub_with_overrides(&tree, &ctx(), &[]);
        assert_eq!(
            serialize_to_lua(&plain_out),
            serialize_to_lua(&override_out)
        );
    }

    #[test]
    fn substitute_placeholders_replaces_all_tokens() {
        let ctx = ScrubContext {
            accounts: vec!["@Real".to_string(), "@Alt".to_string()],
            characters: vec!["MyChar".to_string(), "AltChar".to_string()],
            character_ids: vec!["123456789012345".to_string()],
            extra_worlds: vec![],
        };
        let lua = concat!(
            r#"["${ACCOUNT}"] = { "#,
            r#"["${CHAR:0}"] = { ["id"] = "${CHAR_ID:0}" }, "#,
            r#"["${CHAR:1}"] = { } } "#,
            r#"["${ACCOUNT:1}"] = { } "#,
            r#"["${WORLD}"] = { }"#,
        );
        let result = substitute_placeholders(lua, &ctx, WELL_KNOWN_WORLDS);
        assert!(
            result.contains("@Real"),
            "account not substituted: {result}"
        );
        assert!(
            result.contains("@Alt"),
            "alt account not substituted: {result}"
        );
        assert!(result.contains("MyChar"), "char not substituted: {result}");
        assert!(
            result.contains("AltChar"),
            "alt char not substituted: {result}"
        );
        assert!(
            result.contains("123456789012345"),
            "char id not substituted: {result}"
        );
        assert!(
            result.contains("NA Megaserver"),
            "world not substituted: {result}"
        );
    }

    // ── strip_per_character_data tests ────────────────────────────────────

    fn make_leaf(key: &str) -> SvTreeNode {
        SvTreeNode {
            key: key.to_string(),
            value_type: SvValueType::String,
            value: Some(serde_json::Value::String(key.to_string())),
            children: None,
            raw_lua_value: None,
        }
    }

    fn make_table(key: &str, children: Vec<SvTreeNode>) -> SvTreeNode {
        SvTreeNode {
            key: key.to_string(),
            value_type: SvValueType::Table,
            value: None,
            children: Some(children),
            raw_lua_value: None,
        }
    }

    fn make_root(children: Vec<SvTreeNode>) -> SvTreeNode {
        make_table("__root__", children)
    }

    fn child_keys(node: &SvTreeNode) -> Vec<String> {
        node.children
            .as_ref()
            .map(|c| c.iter().map(|n| n.key.clone()).collect())
            .unwrap_or_default()
    }

    /// Standard layout: root → MyAddonVars → Default → ${ACCOUNT} → [$AccountWide, ${CHAR:0}]
    /// After filter: ${CHAR:0} is stripped, $AccountWide survives.
    #[test]
    fn strip_char_data_under_templated_account() {
        let account_node = make_table(
            "${ACCOUNT}",
            vec![
                make_table("$AccountWide", vec![make_leaf("setting1")]),
                make_table("${CHAR:0}", vec![make_leaf("charData")]),
            ],
        );
        let default_node = make_table("Default", vec![account_node]);
        let addon_var = make_table("MyAddonVars", vec![default_node]);
        let root = make_root(vec![addon_var]);

        let filtered = strip_per_character_data(&root);

        // Drill down: root → MyAddonVars → Default → ${ACCOUNT}
        let addon = &filtered.children.as_ref().unwrap()[0];
        let default = &addon.children.as_ref().unwrap()[0];
        assert_eq!(default.key, "Default");
        let account = &default.children.as_ref().unwrap()[0];
        assert_eq!(account.key, "${ACCOUNT}");
        let kept = child_keys(account);
        assert_eq!(
            kept,
            vec!["$AccountWide"],
            "should strip ${{CHAR:0}}, got: {kept:?}"
        );
    }

    /// Raw @-prefixed account key (pre-scrub form) with templated char key: ${CHAR:0} stripped,
    /// $AccountWide and any other non-char keys preserved.
    #[test]
    fn strip_char_data_under_raw_account() {
        let account_node = make_table(
            "@Author",
            vec![
                make_table("$AccountWide", vec![make_leaf("setting1")]),
                make_table("${CHAR:0}", vec![make_leaf("charData")]),
            ],
        );
        let default_node = make_table("Default", vec![account_node]);
        let addon_var = make_table("MyAddonVars", vec![default_node]);
        let root = make_root(vec![addon_var]);

        let filtered = strip_per_character_data(&root);

        let addon = &filtered.children.as_ref().unwrap()[0];
        let default = &addon.children.as_ref().unwrap()[0];
        let account = &default.children.as_ref().unwrap()[0];
        let kept = child_keys(account);
        assert_eq!(kept, vec!["$AccountWide"]);
    }

    /// Direct-account layout (no Default wrapper): root → MyAddonVars → ${ACCOUNT} → [...]
    /// Non-$AccountWide non-char keys (e.g. addon root keys) survive; ${CHAR:N} is stripped.
    #[test]
    fn strip_char_data_direct_account_no_default_wrapper() {
        let account_node = make_table(
            "${ACCOUNT}",
            vec![
                make_table("$AccountWide", vec![make_leaf("x")]),
                make_table("${CHAR:0}", vec![make_leaf("y")]),
                make_table("AddonRootKey", vec![make_leaf("z")]),
            ],
        );
        let addon_var = make_table("IIfA_Data", vec![account_node]);
        let root = make_root(vec![addon_var]);

        let filtered = strip_per_character_data(&root);

        let addon = &filtered.children.as_ref().unwrap()[0];
        let account = &addon.children.as_ref().unwrap()[0];
        assert_eq!(account.key, "${ACCOUNT}");
        let kept = child_keys(account);
        // ${CHAR:0} stripped; $AccountWide and AddonRootKey survive
        assert_eq!(kept, vec!["$AccountWide", "AddonRootKey"]);
    }

    /// World-first layout (e.g. pChat): root → MyVar → ${WORLD} → ${ACCOUNT} → [...]
    /// ${CHAR:N} stripped; $AccountWide and other non-char keys survive.
    #[test]
    fn strip_char_data_world_first_layout() {
        let account_node = make_table(
            "${ACCOUNT}",
            vec![
                make_table("$AccountWide", vec![make_leaf("wideData")]),
                make_table("${CHAR:0}", vec![make_leaf("perChar")]),
            ],
        );
        let world_node = make_table("${WORLD}", vec![account_node]);
        let addon_var = make_table("pChatSavedVars", vec![world_node]);
        let root = make_root(vec![addon_var]);

        let filtered = strip_per_character_data(&root);

        let addon = &filtered.children.as_ref().unwrap()[0];
        assert_eq!(addon.children.as_ref().unwrap()[0].key, "${WORLD}");
        let world = &addon.children.as_ref().unwrap()[0];
        let account = &world.children.as_ref().unwrap()[0];
        let kept = child_keys(account);
        assert_eq!(kept, vec!["$AccountWide"]);
    }

    /// Raw world name (pre-scrub form) also handled; ${CHAR_ID:N} stripped.
    #[test]
    fn strip_char_data_raw_world_name() {
        let account_node = make_table(
            "@Author",
            vec![
                make_table("$AccountWide", vec![make_leaf("x")]),
                make_table("${CHAR_ID:0}", vec![make_leaf("y")]),
            ],
        );
        let world_node = make_table("NA Megaserver", vec![account_node]);
        let addon_var = make_table("SomeVar", vec![world_node]);
        let root = make_root(vec![addon_var]);

        let filtered = strip_per_character_data(&root);

        let addon = &filtered.children.as_ref().unwrap()[0];
        let world = &addon.children.as_ref().unwrap()[0];
        let account = &world.children.as_ref().unwrap()[0];
        let kept = child_keys(account);
        assert_eq!(kept, vec!["$AccountWide"]);
    }

    /// Addon with only ${CHAR:N} children (no $AccountWide, no other keys):
    /// account node ends up with empty children after stripping.
    #[test]
    fn strip_char_data_only_char_keys_yields_empty() {
        let account_node = make_table(
            "${ACCOUNT}",
            vec![
                make_table("${CHAR:0}", vec![make_leaf("charData")]),
                make_table("${CHAR:1}", vec![make_leaf("charData2")]),
            ],
        );
        let default_node = make_table("Default", vec![account_node]);
        let addon_var = make_table("MyVar", vec![default_node]);
        let root = make_root(vec![addon_var]);

        let filtered = strip_per_character_data(&root);

        let addon = &filtered.children.as_ref().unwrap()[0];
        let default = &addon.children.as_ref().unwrap()[0];
        let account = &default.children.as_ref().unwrap()[0];
        let kept = child_keys(account);
        assert!(
            kept.is_empty(),
            "all ${{CHAR:N}} → empty children, got: {kept:?}"
        );
    }

    /// Addon with no $AccountWide but with non-char keys: those keys survive.
    #[test]
    fn strip_char_data_preserves_non_char_non_account_wide_keys() {
        let account_node = make_table(
            "${ACCOUNT}",
            vec![
                make_table("HarvestMapData", vec![make_leaf("pinData")]),
                make_table("${CHAR:0}", vec![make_leaf("charData")]),
            ],
        );
        let default_node = make_table("Default", vec![account_node]);
        let addon_var = make_table("HarvestMapSavedVars", vec![default_node]);
        let root = make_root(vec![addon_var]);

        let filtered = strip_per_character_data(&root);

        let addon = &filtered.children.as_ref().unwrap()[0];
        let default = &addon.children.as_ref().unwrap()[0];
        let account = &default.children.as_ref().unwrap()[0];
        let kept = child_keys(account);
        assert_eq!(
            kept,
            vec!["HarvestMapData"],
            "non-char key should survive, got: {kept:?}"
        );
    }

    /// Full round-trip: build a realistic tree, run scrub(), then strip_per_character_data().
    /// Verifies the two functions compose correctly — templated ${CHAR:N} is stripped,
    /// $AccountWide survives.
    #[test]
    fn strip_char_data_round_trip_after_scrub() {
        let ctx = ScrubContext {
            accounts: vec!["@RealPlayer".to_string()],
            characters: vec!["HeroChar".to_string()],
            character_ids: vec![],
            extra_worlds: vec![],
        };

        // Construct: MyVar → Default → @RealPlayer → [$AccountWide → {setting}, HeroChar → {data}]
        let account_node = make_table(
            "@RealPlayer",
            vec![
                make_table("$AccountWide", vec![make_leaf("mySetting")]),
                make_table("HeroChar", vec![make_leaf("charSpecific")]),
            ],
        );
        let tree = make_root(vec![make_table(
            "MyVar",
            vec![make_table("Default", vec![account_node])],
        )]);

        let (scrubbed, _report) = scrub(&tree, &ctx);
        let filtered = strip_per_character_data(&scrubbed);

        // Drill to account level
        let my_var = &filtered.children.as_ref().unwrap()[0];
        let default = &my_var.children.as_ref().unwrap()[0];
        let account = &default.children.as_ref().unwrap()[0];

        // Key should be templated
        assert!(
            account.key.starts_with("${ACCOUNT"),
            "account key should be templated, got: {}",
            account.key
        );
        // ${CHAR:0} (templated "HeroChar") stripped; only $AccountWide remains
        let kept = child_keys(account);
        assert_eq!(kept, vec!["$AccountWide"], "got: {kept:?}");
    }

    /// Default → world → account (3 levels deep): root → MyVar → Default → ${WORLD} → ${ACCOUNT} → [...]
    /// Some ESO addons use this layout (world node inside Default, not world-first).
    #[test]
    fn strip_char_data_default_world_account_three_levels() {
        let account_node = make_table(
            "${ACCOUNT}",
            vec![
                make_table("$AccountWide", vec![make_leaf("deepSetting")]),
                make_table("${CHAR:0}", vec![make_leaf("deepChar")]),
            ],
        );
        let world_node = make_table("${WORLD}", vec![account_node]);
        let default_node = make_table("Default", vec![world_node]);
        let addon_var = make_table("SomeAddonVars", vec![default_node]);
        let root = make_root(vec![addon_var]);

        let filtered = strip_per_character_data(&root);

        let addon = &filtered.children.as_ref().unwrap()[0];
        let default = &addon.children.as_ref().unwrap()[0];
        assert_eq!(default.key, "Default");
        let world = &default.children.as_ref().unwrap()[0];
        assert_eq!(world.key, "${WORLD}");
        let account = &world.children.as_ref().unwrap()[0];
        assert_eq!(account.key, "${ACCOUNT}");
        let kept = child_keys(account);
        assert_eq!(kept, vec!["$AccountWide"], "got: {kept:?}");
    }

    // ── ${ACCOUNT_NAME} bare-handle templating (HarvestMap layout) ───────

    #[test]
    fn templates_bare_account_name_key() {
        // HarvestMap stores data under a bare handle (no @):
        // HarvestMap_Data = { ["account"] = { ["Spike'jo"] = { ... } } }
        // After scrub the bare key "Spike'jo" should become "${ACCOUNT_NAME}".
        let tree = parse(
            r#"HarvestMap_Data = {
                ["account"] = {
                    ["Authorbare"] = {
                        ["nodes"] = { [1] = 1 },
                    },
                },
            }"#,
        );
        let ctx = ScrubContext {
            accounts: vec!["@Authorbare".to_string()],
            characters: vec![],
            character_ids: vec![],
            extra_worlds: vec![],
        };
        let (out, report) = scrub(&tree, &ctx);
        let serialized = serialize_to_lua(&out);
        assert!(
            serialized.contains("${ACCOUNT_NAME}"),
            "bare account name should be templated: {serialized}"
        );
        assert!(
            !serialized.contains("Authorbare"),
            "raw bare account name leaked: {serialized}"
        );
        assert!(
            report
                .templated_keys
                .iter()
                .any(|t| matches!(t.kind, TemplateKind::AccountName)),
            "no AccountName entry in templated_keys: {:?}",
            report.templated_keys
        );
    }

    #[test]
    fn substitute_placeholders_handles_account_name_tokens() {
        let ctx = ScrubContext {
            accounts: vec!["@RealHandle".to_string(), "@AltHandle".to_string()],
            characters: vec![],
            character_ids: vec![],
            extra_worlds: vec![],
        };
        let lua = r#"["${ACCOUNT_NAME}"] = { } ["${ACCOUNT_NAME:1}"] = { }"#;
        let result = substitute_placeholders(lua, &ctx, WELL_KNOWN_WORLDS);
        assert!(
            result.contains("RealHandle"),
            "primary bare account not substituted: {result}"
        );
        assert!(
            result.contains("AltHandle"),
            "secondary bare account not substituted: {result}"
        );
        assert!(
            !result.contains("${ACCOUNT_NAME}"),
            "token not replaced: {result}"
        );
    }

    #[test]
    fn detect_then_scrub_templates_bare_name_in_harvestmap_layout() {
        // End-to-end: detect_identities_from_tree finds @Author via the standard
        // HarvestMapSavedVars variable, then scrub() templates the bare "Author"
        // key inside HarvestMap_Data's ["account"] container.
        let tree = parse(
            r#"HarvestMapSavedVars = {
                ["Default"] = {
                    ["@Author"] = {
                        ["$AccountWide"] = { ["enabled"] = true },
                    },
                },
            }
            HarvestMap_Data = {
                ["account"] = {
                    ["Author"] = {
                        ["nodes"] = { [1] = 42 },
                    },
                },
            }"#,
        );
        let detected = detect_identities_from_tree(&tree);
        assert!(
            detected.accounts.contains(&"@Author".to_string()),
            "account not detected: {:?}",
            detected.accounts
        );
        let (out, report) = scrub(&tree, &detected);
        let serialized = serialize_to_lua(&out);
        assert!(
            !serialized.contains(r#"["Author"]"#),
            "bare handle leaked: {serialized}"
        );
        assert!(
            serialized.contains("${ACCOUNT_NAME}"),
            "bare handle not templated: {serialized}"
        );
        assert!(
            report
                .templated_keys
                .iter()
                .any(|t| matches!(t.kind, TemplateKind::AccountName)),
            "no AccountName in report"
        );
    }

    #[test]
    fn character_name_wins_over_bare_account_name_collision() {
        // When ctx.accounts = ["@Author"] and ctx.characters = ["Author"],
        // a key "Author" in the standard layout (under @Author → ...) should
        // template as ${CHAR:0}, not ${ACCOUNT_NAME}. Character identity takes
        // priority because bare-name matching is only a fallback for niche
        // HarvestMap-style containers.
        let tree = parse(
            r#"MyAddon_SV = {
                ["Default"] = {
                    ["@Author"] = {
                        ["Author"] = { ["x"] = 1 },
                        ["$AccountWide"] = { ["y"] = 2 },
                    },
                },
            }"#,
        );
        let ctx = ScrubContext {
            accounts: vec!["@Author".to_string()],
            characters: vec!["Author".to_string()],
            character_ids: vec![],
            extra_worlds: vec![],
        };
        let (out, report) = scrub(&tree, &ctx);
        let serialized = serialize_to_lua(&out);
        assert!(
            serialized.contains("${CHAR:0}"),
            "character key should win collision: {serialized}"
        );
        assert!(
            !serialized.contains("${ACCOUNT_NAME}"),
            "account-name template should not win over character: {serialized}"
        );
        assert!(
            report
                .templated_keys
                .iter()
                .any(|t| matches!(t.kind, TemplateKind::Character)),
            "no Character entry in report"
        );
    }

    // ── Realistic-fixture report ──────────────────────────────────────────
    //
    // Synthetic SV files modelled after the *shape* of real popular addons.
    // Run with:
    //
    //     cargo test --lib saved_variables::scrub::tests::fixture_report \
    //         -- --nocapture --include-ignored
    //
    // The test is `#[ignore]` so normal CI runs stay quiet; it's intended as
    // a hand-driven Phase 0 readout, not a regression check.

    fn fixture_action_duration_reminder() -> String {
        // Pure config: toggles, numbers, colors. Should survive almost intact.
        r#"ActionDurationReminderSV = {
            ["Default"] = {
                ["@Author"] = {
                    ["$AccountWide"] = {
                        ["enabled"] = true,
                        ["scale"] = 1.25,
                        ["fadeOut"] = 0.3,
                        ["showStackCount"] = true,
                        ["barColor"] = { ["r"] = 0.8, ["g"] = 0.4, ["b"] = 0.2, ["a"] = 1.0 },
                        ["frame"] = {
                            ["x"] = 640.5,
                            ["y"] = 480.0,
                            ["width"] = 240,
                            ["height"] = 32,
                        },
                        ["ignoredAbilities"] = {
                            ["1"] = "Critical Surge",
                            ["2"] = "Resolving Vigor",
                            ["3"] = "Echoing Vigor",
                        },
                        ["$LastCharacterName"] = "Mainchar",
                    },
                },
            },
        }"#
        .to_string()
    }

    fn fixture_combat_metrics(fights: usize) -> String {
        // Typical CombatMetrics shape: lots of fight data per character.
        let mut s = String::from(
            "CombatMetrics_Save = {\n\
             [\"Default\"] = {\n\
             [\"@Author\"] = {\n\
             [\"$AccountWide\"] = {\n\
             [\"enabled\"] = true,\n\
             [\"liveReport\"] = true,\n\
             [\"theme\"] = \"dark\",\n\
             },\n\
             [\"Mainchar\"] = {\n\
             [\"fightData\"] = {\n",
        );
        for i in 0..fights {
            s.push_str(&format!(
                "[{}] = {{ [\"DPSOut\"] = {}.5, [\"duration\"] = {}, [\"bossName\"] = \"Boss-{}\", [\"log\"] = \"line A\\nline B\\nline C\\nline D\\nline E\" }},\n",
                i + 1,
                10000 + i * 137,
                30 + i % 60,
                i
            ));
        }
        s.push_str("},\n},\n},\n},\n}\n");
        s
    }

    fn fixture_master_merchant(listings: usize) -> String {
        let mut s = String::from(
            "MasterMerchant_SavedVariables = {\n\
             [\"Default\"] = {\n\
             [\"@Author\"] = {\n\
             [\"$AccountWide\"] = {\n\
             [\"trimDecimals\"] = true,\n\
             [\"showFullPrice\"] = true,\n\
             [\"defaultDays\"] = 30,\n\
             [\"SalesHistory\"] = {\n",
        );
        for i in 0..listings {
            s.push_str(&format!(
                "[{}] = {{ [\"itemLink\"] = \"|H1:item:{}:|h|h\", [\"price\"] = {}, [\"buyer\"] = \"@Buyer{}\", [\"seller\"] = \"@Author\", [\"guild\"] = \"GuildName{}\" }},\n",
                i + 1,
                10000 + i,
                500 + i * 17,
                i % 200,
                i % 5
            ));
        }
        s.push_str(
            "},\n\
             [\"GuildRoster\"] = { [\"GuildName0\"] = { [\"@MemberA\"] = true, [\"@MemberB\"] = true } },\n\
             },\n},\n},\n}\n",
        );
        s
    }

    fn fixture_lib_histoire(events: usize) -> String {
        let mut s = String::from(
            "LibHistoire_SV = {\n\
             [\"Default\"] = {\n\
             [\"@Author\"] = {\n\
             [\"$AccountWide\"] = {\n\
             [\"showProgress\"] = true,\n\
             [\"guildHistory\"] = {\n\
             [\"GuildName0\"] = {\n",
        );
        for i in 0..events {
            s.push_str(&format!(
                "[{}] = {{ [\"type\"] = \"deposit\", [\"who\"] = \"@Friend{}\", [\"amount\"] = {}, [\"timestamp\"] = {} }},\n",
                i + 1,
                i % 50,
                100 + i * 3,
                1700000000 + i * 60
            ));
        }
        s.push_str("},\n},\n},\n},\n},\n}\n");
        s
    }

    fn fixture_p_chat(messages: usize) -> String {
        let mut s = String::from(
            "pChatData = {\n\
             [\"Default\"] = {\n\
             [\"@Author\"] = {\n\
             [\"$AccountWide\"] = {\n\
             [\"timestampFormat\"] = \"HH:mm\",\n\
             [\"channelColors\"] = { [\"say\"] = \"FFFFFF\", [\"yell\"] = \"FF0000\" },\n\
             [\"chatLogs\"] = {\n",
        );
        for i in 0..messages {
            s.push_str(&format!(
                "[{}] = {{ [\"channel\"] = \"whisper\", [\"from\"] = \"@Friend{}\", [\"text\"] = \"hello there friend, how are you doing today?\" }},\n",
                i + 1,
                i % 30
            ));
        }
        s.push_str(
            "},\n\
             [\"recentWhispers\"] = { [\"1\"] = \"@A\", [\"2\"] = \"@B\" },\n\
             },\n},\n},\n}\n",
        );
        s
    }

    fn realistic_ctx() -> ScrubContext {
        ScrubContext {
            accounts: vec!["@Author".to_string()],
            characters: vec!["Mainchar".to_string()],
            character_ids: vec![],
            extra_worlds: vec![],
        }
    }

    /// Drop-stat aggregator for the report.
    fn run_one(name: &str, lua: String, ctx: &ScrubContext) -> (usize, usize, usize, usize) {
        let tree = parse_sv_file(&lua, &format!("{name}.lua")).expect("parse");
        let original_bytes = lua.len();
        let (_scrubbed, report) = scrub(&tree, ctx);

        // Count drops by reason.
        let mut by_block = 0usize;
        let mut by_identity = 0usize;
        let mut by_handle = 0usize;
        let mut by_always = 0usize;
        for d in &report.drops {
            match d.reason {
                DropReason::BlockedKeyHeuristic => by_block += d.bytes_removed,
                DropReason::StringValueContainsIdentity => by_identity += d.bytes_removed,
                DropReason::StringValueLooksLikeAccount => by_handle += d.bytes_removed,
                DropReason::AlwaysDropped => by_always += d.bytes_removed,
                DropReason::OverrideDisabled | DropReason::OverrideDenyPath => {}
            }
        }

        println!("\n── {name} ─────────────────────────────────────────");
        println!("  raw input bytes (Lua source)       : {original_bytes:>10}");
        println!(
            "  parsed re-serialized (baseline)    : {:>10}",
            report.original_bytes
        );
        println!(
            "  scrubbed bytes                     : {:>10}",
            report.scrubbed_bytes
        );
        let pct = if report.original_bytes > 0 {
            100.0 * report.scrubbed_bytes as f64 / report.original_bytes as f64
        } else {
            0.0
        };
        println!("  retained vs baseline               : {pct:>9.1}%");
        println!("  drops:");
        println!("    blocked-key-heuristic            : {by_block:>10} bytes");
        println!("    string-value-contains-identity   : {by_identity:>10} bytes");
        println!("    string-value-looks-like-account  : {by_handle:>10} bytes");
        println!("    always-dropped                   : {by_always:>10} bytes");
        println!(
            "  templated keys                     : {:>10}",
            report.templated_keys.len()
        );
        println!(
            "  drop entries                       : {:>10}",
            report.drops.len()
        );

        (
            original_bytes,
            report.original_bytes,
            report.scrubbed_bytes,
            report.drops.len(),
        )
    }

    #[test]
    #[ignore = "fixture report — run explicitly with --include-ignored --nocapture"]
    fn fixture_report() {
        let ctx = realistic_ctx();

        println!("\n=== Phase 0 scrub fixture report ===");

        // 1) Pure-config addon: should be tiny and almost fully retained.
        let (raw_a, base_a, post_a, _) = run_one(
            "ActionDurationReminder",
            fixture_action_duration_reminder(),
            &ctx,
        );
        // 2) Combat log heavy.
        let (raw_b, base_b, post_b, _) = run_one(
            "CombatMetrics (500 fights)",
            fixture_combat_metrics(500),
            &ctx,
        );
        // 3) Sales history heavy.
        let (raw_c, base_c, post_c, _) = run_one(
            "MasterMerchant (2000 listings)",
            fixture_master_merchant(2000),
            &ctx,
        );
        // 4) Guild history heavy.
        let (raw_d, base_d, post_d, _) = run_one(
            "LibHistoire (3000 events)",
            fixture_lib_histoire(3000),
            &ctx,
        );
        // 5) Chat log heavy.
        let (raw_e, base_e, post_e, _) =
            run_one("pChat (2000 messages)", fixture_p_chat(2000), &ctx);

        println!("\n=== Summary ===");
        println!(
            "{:<32} {:>10} {:>10} {:>10} {:>8}",
            "fixture", "raw", "baseline", "scrubbed", "kept%"
        );
        for (name, raw, base, post) in [
            ("ActionDurationReminder", raw_a, base_a, post_a),
            ("CombatMetrics", raw_b, base_b, post_b),
            ("MasterMerchant", raw_c, base_c, post_c),
            ("LibHistoire", raw_d, base_d, post_d),
            ("pChat", raw_e, base_e, post_e),
        ] {
            let pct = if base > 0 {
                100.0 * post as f64 / base as f64
            } else {
                0.0
            };
            println!("{name:<32} {raw:>10} {base:>10} {post:>10} {pct:>7.1}%");
        }

        // Sanity: heavy fixtures should be heavily stripped; the config-only
        // fixture should be mostly retained (above 30% — it loses identity
        // keys but not real config).
        assert!(
            post_a as f64 / base_a as f64 > 0.3,
            "config-only addon should retain >30% after scrub"
        );
        for (label, base, post) in [
            ("CombatMetrics", base_b, post_b),
            ("MasterMerchant", base_c, post_c),
            ("LibHistoire", base_d, post_d),
            ("pChat", base_e, post_e),
        ] {
            assert!(
                (post as f64 / base as f64) < 0.2,
                "{} should be reduced below 20% (got {:.1}%)",
                label,
                100.0 * post as f64 / base as f64
            );
        }
    }

    // ── Real-file scrubber ────────────────────────────────────────────────
    //
    // Hand-driven helper for validating the scrubber against an actual
    // SavedVariables file without needing the Tauri app. Accepts a path
    // through the `KALPA_SCRUB_PATH` env var, runs identity auto-detection
    // + scrub, prints a report, and (optionally) writes the scrubbed Lua
    // to `KALPA_SCRUB_OUT` so a human can diff it against the original.
    //
    //   KALPA_SCRUB_PATH=/path/to/SavedVariables/ActionDurationReminder.lua \
    //   KALPA_SCRUB_OUT=/tmp/adr.scrubbed.lua \
    //   cargo test --lib \
    //     saved_variables::scrub::tests::real_file_scrub \
    //     -- --include-ignored --nocapture
    //
    // No assertions beyond "the file parses" — the human reads the report.

    #[test]
    #[ignore = "real-file scrub helper — set KALPA_SCRUB_PATH and run with --include-ignored"]
    fn real_file_scrub() {
        let path = match std::env::var("KALPA_SCRUB_PATH") {
            Ok(p) => p,
            Err(_) => {
                eprintln!("Set KALPA_SCRUB_PATH to a SavedVariables .lua file path and re-run.");
                return;
            }
        };

        let content = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let file_name = std::path::Path::new(&path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown.lua".to_string());

        let tree = parse_sv_file(&content, &file_name)
            .unwrap_or_else(|e| panic!("parse {file_name}: {e}"));

        let detected = detect_identities_from_tree(&tree);
        println!("\n── Detected identities ──────────────────────────────────────");
        println!("  accounts        : {:?}", detected.accounts);
        println!("  characters      : {:?}", detected.characters);
        println!("  character_ids   : {:?}", detected.character_ids);
        println!("  extra_worlds    : {:?}", detected.extra_worlds);

        let (scrubbed, report) = scrub(&tree, &detected);
        let scrubbed_lua = serialize_to_lua(&scrubbed);

        println!("\n── Scrub report ────────────────────────────────────────────");
        println!("  file                 : {file_name}");
        println!("  raw input bytes      : {:>12}", content.len());
        println!("  parsed → reserialized: {:>12}", report.original_bytes);
        println!("  scrubbed bytes       : {:>12}", report.scrubbed_bytes);
        if report.original_bytes > 0 {
            let pct = 100.0 * report.scrubbed_bytes as f64 / report.original_bytes as f64;
            println!("  retained vs baseline : {pct:>11.2}%");
        }
        println!(
            "  templated keys       : {:>12}",
            report.templated_keys.len()
        );
        println!("  drops                : {:>12}", report.drops.len());

        // Drop breakdown by reason.
        let mut by_reason: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
        for d in &report.drops {
            let reason_key = match d.reason {
                DropReason::BlockedKeyHeuristic => "blocked-key-heuristic",
                DropReason::AlwaysDropped => "always-dropped",
                DropReason::StringValueContainsIdentity => "string-value-contains-identity",
                DropReason::StringValueLooksLikeAccount => "string-value-looks-like-account",
                DropReason::OverrideDisabled => "override-disabled",
                DropReason::OverrideDenyPath => "override-deny-path",
            }
            .to_string();
            let entry = by_reason.entry(reason_key).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += d.bytes_removed;
        }

        println!("\n── Drop breakdown by reason ────────────────────────────────");
        for (reason, (count, bytes)) in &by_reason {
            println!("  {reason:<35} {count:>6} drops, {bytes:>10} bytes");
        }

        // First few drop paths for spot-checking.
        let preview = report.drops.iter().take(15);
        if preview.clone().count() > 0 {
            println!("\n── First 15 dropped paths ──────────────────────────────────");
            for d in preview {
                println!(
                    "  [{:>10} bytes] {:?}  ({:?})",
                    d.bytes_removed, d.path, d.reason
                );
            }
            if report.drops.len() > 15 {
                println!("  ...and {} more", report.drops.len() - 15);
            }
        }

        if let Ok(out_path) = std::env::var("KALPA_SCRUB_OUT") {
            std::fs::write(&out_path, &scrubbed_lua)
                .unwrap_or_else(|e| panic!("write {out_path}: {e}"));
            println!("\nScrubbed Lua written to: {out_path}");
        }
    }
}
