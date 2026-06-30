//! Raw ESO `Encounter.log` → structured upload representation.
//!
//! This is the substantial, clean-room piece of the native uploader. The scanner
//! gives us a session's raw log lines (as byte ranges); the upload protocol wants
//! a compact, structured form: a sequence of **events** plus a **master table**
//! that interns the units, abilities, and effects those events reference (so each
//! event carries small integer ids instead of repeating full descriptors).
//!
//! ## Two stages, kept separate
//!
//! 1. **Parse** — each raw line becomes a typed [`LogEvent`] (or is skipped). The
//!    ESO log line grammar (`<relativeMs>,<TYPE>,<fields…>`) is public and is the
//!    only thing this stage encodes. This stage is pure and fully unit-testable
//!    without any network or format-version coupling.
//! 2. **Serialize** — the parsed events + interned [`MasterTable`] are written to
//!    the wire bytes the server accepts, keyed to [`super::format::FORMAT_VERSION`].
//!    This stage is pinned by golden-file tests (parse → serialize → assert
//!    byte-stable) so a format regression is caught locally, never in production.
//!
//! Splitting the stages means the bulk of the logic (parsing) is verifiable now,
//! and only the thin final serialization depends on the empirically-pinned
//! format version.

use std::collections::HashMap;

use super::format::FormatError;

/// An interned reference into the [`MasterTable`]. Small and Copy so events stay
/// cheap. The concrete id space (and whether units/abilities/effects share one)
/// is part of the pinned format and finalized with the serializer.
pub type MasterId = u32;

/// The master table: every distinct unit/ability/effect a segment references,
/// interned to a stable id. Built incrementally as events are parsed so the
/// first reference assigns the id and later references reuse it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MasterTable {
    units: Interner<UnitKey>,
    abilities: Interner<u64>,
    effects: Interner<u64>,
}

impl MasterTable {
    /// Intern a unit (by its log-assigned key), returning its stable id.
    pub fn intern_unit(&mut self, key: UnitKey) -> MasterId {
        self.units.intern(key)
    }

    /// Intern an ability id, returning its stable master id.
    pub fn intern_ability(&mut self, ability_id: u64) -> MasterId {
        self.abilities.intern(ability_id)
    }

    /// Intern an effect id, returning its stable master id.
    pub fn intern_effect(&mut self, effect_id: u64) -> MasterId {
        self.effects.intern(effect_id)
    }

    pub fn unit_count(&self) -> usize {
        self.units.len()
    }
    pub fn ability_count(&self) -> usize {
        self.abilities.len()
    }
    pub fn effect_count(&self) -> usize {
        self.effects.len()
    }
}

/// Identifies a unit within a session. ESO reuses small "unit ids" within a
/// session (assigned at `UNIT_ADDED`), so the session-scoped id is the key; the
/// richer descriptor (name, type, etc.) is attached at intern time by the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitKey(pub u64);

/// Order-stable interner: first `intern` of a value assigns the next id; repeats
/// return the same id. Iteration order is insertion order, which the serializer
/// relies on for byte-stable master tables.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Interner<T: std::hash::Hash + Eq + Clone> {
    ids: HashMap<T, MasterId>,
    order: Vec<T>,
}

impl<T: std::hash::Hash + Eq + Clone> Default for Interner<T> {
    fn default() -> Self {
        Self {
            ids: HashMap::new(),
            order: Vec::new(),
        }
    }
}

impl<T: std::hash::Hash + Eq + Clone> Interner<T> {
    fn intern(&mut self, value: T) -> MasterId {
        if let Some(&id) = self.ids.get(&value) {
            return id;
        }
        let id = self.order.len() as MasterId;
        self.order.push(value.clone());
        self.ids.insert(value, id);
        id
    }

    fn len(&self) -> usize {
        self.order.len()
    }
}

/// A single parsed log event. This is the typed intermediate form between raw
/// text and wire bytes. Only the variants the protocol needs are modeled; lines
/// outside that set are skipped during parsing.
///
/// Deliberately minimal for now — the parse stage grows variant-by-variant, each
/// covered by a unit test against a real sample line, so the IR never gets ahead
/// of verified parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    /// Session header (`BEGIN_LOG`): wall-clock epoch ms + declared log version.
    BeginLog { unix_ms: u64, log_version: String },
    /// A combat boundary (`BEGIN_COMBAT` / `END_COMBAT`), relative ms.
    CombatBoundary { rel_ms: u64, begin: bool },
    /// Any other modeled event, carrying its relative timestamp. Placeholder for
    /// the typed variants (combat events, effect changes, unit add/remove) added
    /// as parsing is implemented and golden-tested.
    Other { rel_ms: u64 },
}

impl LogEvent {
    /// The event's relative-ms timestamp (0 for the session header line).
    pub fn rel_ms(&self) -> u64 {
        match self {
            LogEvent::BeginLog { .. } => 0,
            LogEvent::CombatBoundary { rel_ms, .. } => *rel_ms,
            LogEvent::Other { rel_ms } => *rel_ms,
        }
    }
}

/// Parse a single raw log line into a [`LogEvent`], or `Ok(None)` to skip it.
///
/// Pure and allocation-light. Implements the public ESO log grammar only. Grows
/// as more event types are modeled; every added branch comes with a golden/unit
/// test against a real sample line.
pub fn parse_line(line: &str) -> Result<Option<LogEvent>, FormatError> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }

    let mut it = line.splitn(3, ',');
    let rel_ms = it
        .next()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .ok_or_else(|| FormatError::Unparseable("missing/!numeric leading timestamp".into()))?;
    let Some(kind) = it.next().map(str::trim) else {
        return Ok(None);
    };
    let rest = it.next().unwrap_or("");

    match kind {
        "BEGIN_LOG" => {
            // BEGIN_LOG,<unixMs>,<logVersion>,<realm>,...
            let mut f = rest.split(',');
            let unix_ms = f
                .next()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .ok_or_else(|| FormatError::Unparseable("BEGIN_LOG without unix ms".into()))?;
            let log_version = f.next().unwrap_or("").trim().trim_matches('"').to_string();
            Ok(Some(LogEvent::BeginLog {
                unix_ms,
                log_version,
            }))
        }
        "BEGIN_COMBAT" => Ok(Some(LogEvent::CombatBoundary {
            rel_ms,
            begin: true,
        })),
        "END_COMBAT" => Ok(Some(LogEvent::CombatBoundary {
            rel_ms,
            begin: false,
        })),
        // Lines we don't yet model are skipped rather than errored: an unknown
        // line type is normal (the format has many), not a parse failure.
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interner_assigns_stable_insertion_order_ids() {
        let mut t = MasterTable::default();
        let a = t.intern_ability(1000);
        let b = t.intern_ability(2000);
        let a2 = t.intern_ability(1000);
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(a2, a, "re-interning returns the same id");
        assert_eq!(t.ability_count(), 2);
    }

    #[test]
    fn interner_spaces_are_independent_per_kind() {
        let mut t = MasterTable::default();
        let u = t.intern_unit(UnitKey(5));
        let a = t.intern_ability(5);
        // Same numeric value in different spaces both get id 0 — they're separate.
        assert_eq!(u, 0);
        assert_eq!(a, 0);
        assert_eq!(t.unit_count(), 1);
        assert_eq!(t.ability_count(), 1);
    }

    #[test]
    fn parses_begin_log() {
        let line = "0,BEGIN_LOG,1699999999999,15,\"NA Megaserver\",\"en\",\"eso.live.10.1.5\"";
        let ev = parse_line(line).unwrap().unwrap();
        match ev {
            LogEvent::BeginLog {
                unix_ms,
                log_version,
            } => {
                assert_eq!(unix_ms, 1699999999999);
                assert_eq!(log_version, "15");
            }
            other => panic!("expected BeginLog, got {other:?}"),
        }
    }

    #[test]
    fn parses_combat_boundaries() {
        assert_eq!(
            parse_line("12345,BEGIN_COMBAT").unwrap().unwrap(),
            LogEvent::CombatBoundary {
                rel_ms: 12345,
                begin: true
            }
        );
        assert_eq!(
            parse_line("23456,END_COMBAT,...").unwrap().unwrap(),
            LogEvent::CombatBoundary {
                rel_ms: 23456,
                begin: false
            }
        );
    }

    #[test]
    fn skips_blank_and_unmodeled_lines() {
        assert_eq!(parse_line("").unwrap(), None);
        assert_eq!(parse_line("   ").unwrap(), None);
        // A real but not-yet-modeled line type is skipped, not an error.
        assert_eq!(
            parse_line("999,ZONE_CHANGED,1207,\"Foo\",veteran").unwrap(),
            None
        );
    }

    #[test]
    fn rejects_nonnumeric_timestamp() {
        let err = parse_line("notanumber,BEGIN_COMBAT").unwrap_err();
        assert!(matches!(err, FormatError::Unparseable(_)));
    }
}
