//! The ae message board — the filtered cross-fleet record, derived on read.
//!
//! One schema, thin per-harness readers. Each reader is a pure function over
//! the transcript bytes it is handed; locating the file and opening it belong
//! to a later slice. The view is DERIVED: [`collect`] is the one place rows
//! from many files merge, sort and dedup. Nothing writes a board file.
//!
//! Row identity is the SOURCE RECORD — (`file`, `offset`) — never the actor
//! or the body: two seats may type the same words in the same microsecond, and
//! that is two rows, not one.

pub mod claude;

use crate::tool::ToolKind;

/// Who speaks in a board row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A genuine human turn: bare of any ae marker.
    Human,
    // Phase 7 adds `Assistant`. It is absent on purpose until then: a variant
    // nothing constructs would be dead code the gate refuses.
}

/// One board row: one human turn from one harness transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Epoch micros, in the source store's native precision.
    pub ts: i64,
    /// `session:seat`, from the meta roster.
    pub actor: String,
    /// Who speaks.
    pub role: Role,
    /// The turn's body, trimmed, reminders stripped.
    pub body: String,
    /// Which harness wrote the transcript.
    pub source: ToolKind,
    /// Source file identity, as the caller names it.
    pub file: String,
    /// Byte offset of the record's first byte in that file.
    pub offset: u64,
}

/// A per-seat coverage row: the board is INCOMPLETE for this seat, and says
/// why. A partial board must never look whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// `session:seat`, from the meta roster.
    pub actor: String,
    /// Why coverage is incomplete (`torn last record`, a tripped line cap, …).
    pub reason: String,
}

/// THE one sort/dedup point: stable sort by (`ts`, `file`, `offset`), then
/// drop an identical (`file`, `offset`) read twice. Same-timestamp rows from
/// different files all survive; so do two seats' identical words.
#[must_use]
pub fn collect(mut rows: Vec<Row>) -> Vec<Row> {
    rows.sort_by(|left, right| {
        (left.ts, &left.file, left.offset).cmp(&(right.ts, &right.file, right.offset))
    });
    rows.dedup_by(|later, first| first.file == later.file && first.offset == later.offset);
    rows
}

#[cfg(test)]
mod tests {
    use super::{Coverage, Role, Row, collect};
    use crate::tool::ToolKind;

    fn row(ts: i64, file: &str, offset: u64, body: &str) -> Row {
        Row {
            ts,
            actor: "s:seat".to_owned(),
            role: Role::Human,
            body: body.to_owned(),
            source: ToolKind::Claude,
            file: file.to_owned(),
            offset,
        }
    }

    #[test]
    fn collect_sorts_by_timestamp_file_then_offset() {
        let rows = collect(vec![
            row(3, "b", 0, "fourth"),
            row(1, "b", 9, "third"),
            row(1, "a", 0, "first"),
            row(1, "a", 5, "second"),
        ]);
        let bodies: Vec<&str> = rows.iter().map(|row| row.body.as_str()).collect();
        assert_eq!(bodies, ["first", "second", "third", "fourth"]);
    }

    #[test]
    fn collect_keeps_same_microsecond_rows_from_two_files() {
        // Mutation 4's guard: sorting by (ts, actor) would still order these,
        // but sorting must never consult the actor at all — same actor, same
        // micro, two files, both survive in file order.
        let rows = collect(vec![row(7, "b", 0, "bee"), row(7, "a", 0, "aye")]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].body, "aye");
        assert_eq!(rows[1].body, "bee");
    }

    #[test]
    fn collect_dedups_only_the_same_source_record() {
        let twice = row(5, "a", 3, "same words");
        let rows = collect(vec![
            twice.clone(),
            row(5, "a", 9, "same words"),
            twice,
            row(5, "b", 3, "same words"),
        ]);
        // One (a,3) survives; the same words at another offset or in another
        // file are different records and all survive.
        let kept: Vec<(&str, u64)> = rows
            .iter()
            .map(|row| (row.file.as_str(), row.offset))
            .collect();
        assert_eq!(kept, [("a", 3), ("a", 9), ("b", 3)]);
    }

    #[test]
    fn coverage_rows_carry_their_reason_verbatim() {
        let coverage = Coverage {
            actor: "s:seat".to_owned(),
            reason: "torn last record".to_owned(),
        };
        assert_eq!(coverage.reason, "torn last record");
    }
}
