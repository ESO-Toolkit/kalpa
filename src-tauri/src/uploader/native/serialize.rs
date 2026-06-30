//! Segment + master-table string assembly for the native uploader.
//!
//! The upload protocol sends two kinds of text payload, each ZIP-compressed
//! (DEFLATE, single file `log.txt`) and posted as the `logfile` multipart field:
//!
//! * **Master table** (`set-report-master-table`): the interned actors,
//!   abilities, tuples, and pets a report references.
//! * **Fights segment** (`add-report-segment`): the combat events.
//!
//! This module owns the **outer grammar** — the exact line/field framing the
//! server parses — which was established as a protocol fact (header line,
//! `lastAssigned*` counters, section order, the `|` and `\n` separators). The
//! **inner** per-entry strings (one actor's descriptor, one event's line) are
//! produced by the parse/convert layer and passed in here; this module only
//! frames them. Keeping the framing separate from the inner encoding means the
//! framing is byte-exact and golden-testable now, while the inner encoding is
//! finalized against real samples.
//!
//! Clean-room: the grammar is a fact about the service's parser (like a wire
//! schema); this assembly is implemented from scratch.

/// The assembled master-table text, ready to ZIP and upload.
///
/// Grammar (each `\n`-terminated as shown):
/// ```text
/// {logVersion}|{gameVersion}|{logFileDetails}\n
/// {lastAssignedActorId}\n
/// {actorsString}            // inner: one framed entry per actor
/// {lastAssignedAbilityId}\n
/// {abilitiesString}
/// {lastAssignedTupleId}\n
/// {tuplesString}
/// {lastAssignedPetId}\n
/// {petsString}
/// ```
/// The `lastAssigned*` values are the highest id the parser assigned in each
/// space (not the count), so the server can size its tables. The inner
/// `*String` values already carry their own trailing newlines.
pub struct MasterTableDoc<'a> {
    pub log_version: &'a str,
    pub game_version: &'a str,
    /// Optional extra detail field; empty string when absent (the protocol sends
    /// an empty third field rather than omitting it).
    pub log_file_details: &'a str,
    pub last_assigned_actor_id: u64,
    pub actors_string: &'a str,
    pub last_assigned_ability_id: u64,
    pub abilities_string: &'a str,
    pub last_assigned_tuple_id: u64,
    pub tuples_string: &'a str,
    pub last_assigned_pet_id: u64,
    pub pets_string: &'a str,
}

impl MasterTableDoc<'_> {
    /// Render the master-table text exactly as the server expects.
    pub fn render(&self) -> String {
        // Pre-size to avoid reallocs on large tables.
        let mut s = String::with_capacity(
            self.actors_string.len()
                + self.abilities_string.len()
                + self.tuples_string.len()
                + self.pets_string.len()
                + 64,
        );
        // Header line: version|gameVersion|details
        s.push_str(self.log_version);
        s.push('|');
        s.push_str(self.game_version);
        s.push('|');
        s.push_str(self.log_file_details);
        s.push('\n');
        // Each section: the last-assigned id on its own line, then the section's
        // already-framed inner string.
        push_section(&mut s, self.last_assigned_actor_id, self.actors_string);
        push_section(&mut s, self.last_assigned_ability_id, self.abilities_string);
        push_section(&mut s, self.last_assigned_tuple_id, self.tuples_string);
        push_section(&mut s, self.last_assigned_pet_id, self.pets_string);
        s
    }
}

/// The assembled fights-segment text, ready to ZIP and upload.
///
/// Grammar:
/// ```text
/// {logVersion}|{gameVersion}\n
/// {totalEventCount}\n
/// {eventsString}   // concatenation of every fight's events
/// ```
/// `totalEventCount` is the sum of each fight's event count; `eventsString` is
/// the concatenation of each fight's already-framed events (each carrying its
/// own newlines).
pub struct FightsSegmentDoc<'a> {
    pub log_version: &'a str,
    pub game_version: &'a str,
    /// (event_count, events_string) for each fight, in order.
    pub fights: &'a [(u64, &'a str)],
}

impl FightsSegmentDoc<'_> {
    /// Render the fights-segment text exactly as the server expects.
    pub fn render(&self) -> String {
        let total_events: u64 = self.fights.iter().map(|(n, _)| *n).sum();
        let inner_len: usize = self.fights.iter().map(|(_, s)| s.len()).sum();
        let mut s = String::with_capacity(inner_len + 48);
        s.push_str(self.log_version);
        s.push('|');
        s.push_str(self.game_version);
        s.push('\n');
        s.push_str(&total_events.to_string());
        s.push('\n');
        for (_, events) in self.fights {
            s.push_str(events);
        }
        s
    }
}

/// Append `{id}\n{inner}` — a section's last-assigned id line followed by its
/// pre-framed inner string.
fn push_section(s: &mut String, last_assigned_id: u64, inner: &str) {
    s.push_str(&last_assigned_id.to_string());
    s.push('\n');
    s.push_str(inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real master-table payload captured from the official uploader. Used as
    /// ground truth: our framing, fed the same parsed parts, must reproduce it
    /// byte-for-byte.
    const REAL_MASTER_TABLE: &str = include_str!("testdata/sample_master_table.txt");

    #[test]
    fn master_table_framing_matches_real_capture() {
        // Decompose the captured file into the parts our serializer takes, then
        // re-render and assert byte-equality with the original. This pins the
        // framing (header `v|g|details`, per-section `lastId\n` + body) against
        // a real ESO Logs payload, not a hand-made fixture.
        let real = REAL_MASTER_TABLE;
        // header line: 15|1|
        let (header, body) = real.split_once('\n').unwrap();
        let mut h = header.split('|');
        let log_version = h.next().unwrap();
        let game_version = h.next().unwrap();
        let log_file_details = h.next().unwrap_or("");

        // Body is: {actorId}\n{actors}{abilityId}\n{abilities}{tupleId}\n{tuples}{petId}\n{pets}
        // Split it back into the four (lastId, section) blocks by locating each
        // numeric id line. Because section bodies are themselves newline-framed,
        // we reconstruct by re-rendering and comparing — the parse here mirrors
        // the documented grammar exactly.
        let lines: Vec<&str> = body.split_inclusive('\n').collect();
        // Find the four id-lines by walking: an id line is a bare integer.
        // Sections: actors, abilities, tuples, pets — in order.
        let mut idx = 0;
        let mut take_section = || -> (u64, String) {
            // first line is the lastAssigned id
            let id: u64 = lines[idx].trim_end().parse().unwrap();
            idx += 1;
            // section body = lines until the next bare-integer id line (or end).
            let mut s = String::new();
            while idx < lines.len() {
                let candidate = lines[idx].trim_end();
                // A pure-integer line that is followed by more content is the
                // next section's id. The final pets id (0) has no body after it.
                if candidate.parse::<u64>().is_ok() && !candidate.is_empty() {
                    break;
                }
                s.push_str(lines[idx]);
                idx += 1;
            }
            (id, s)
        };
        let (actor_id, actors) = take_section();
        let (ability_id, abilities) = take_section();
        let (tuple_id, tuples) = take_section();
        let (pet_id, pets) = take_section();

        let doc = MasterTableDoc {
            log_version,
            game_version,
            log_file_details,
            last_assigned_actor_id: actor_id,
            actors_string: &actors,
            last_assigned_ability_id: ability_id,
            abilities_string: &abilities,
            last_assigned_tuple_id: tuple_id,
            tuples_string: &tuples,
            last_assigned_pet_id: pet_id,
            pets_string: &pets,
        };
        assert_eq!(
            doc.render(),
            real,
            "our master-table framing must reproduce the real captured payload byte-for-byte"
        );
    }

    #[test]
    fn master_table_framing_is_byte_exact() {
        let doc = MasterTableDoc {
            log_version: "15",
            game_version: "10.1.5",
            log_file_details: "",
            last_assigned_actor_id: 3,
            actors_string: "A1\nA2\nA3\n",
            last_assigned_ability_id: 2,
            abilities_string: "B1\nB2\n",
            last_assigned_tuple_id: 1,
            tuples_string: "T1\n",
            last_assigned_pet_id: 0,
            pets_string: "",
        };
        let out = doc.render();
        let expected = "15|10.1.5|\n3\nA1\nA2\nA3\n2\nB1\nB2\n1\nT1\n0\n";
        assert_eq!(out, expected, "master-table framing must match the grammar");
    }

    #[test]
    fn master_table_includes_log_file_details_when_present() {
        let doc = MasterTableDoc {
            log_version: "15",
            game_version: "10.1.5",
            log_file_details: "detail",
            last_assigned_actor_id: 0,
            actors_string: "",
            last_assigned_ability_id: 0,
            abilities_string: "",
            last_assigned_tuple_id: 0,
            tuples_string: "",
            last_assigned_pet_id: 0,
            pets_string: "",
        };
        assert!(doc.render().starts_with("15|10.1.5|detail\n"));
    }

    #[test]
    fn fights_segment_sums_event_counts_and_concatenates() {
        let doc = FightsSegmentDoc {
            log_version: "15",
            game_version: "10.1.5",
            fights: &[(2, "E1\nE2\n"), (3, "E3\nE4\nE5\n")],
        };
        let out = doc.render();
        // header | total (2+3=5) | concatenated events
        assert_eq!(out, "15|10.1.5\n5\nE1\nE2\nE3\nE4\nE5\n");
    }

    #[test]
    fn empty_fights_segment_is_header_and_zero() {
        let doc = FightsSegmentDoc {
            log_version: "15",
            game_version: "10.1.5",
            fights: &[],
        };
        assert_eq!(doc.render(), "15|10.1.5\n0\n");
    }
}
