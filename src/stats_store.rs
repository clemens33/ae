//! The maintained Statistics store's pure model: the evidence each journal
//! source leaves, kept up to date as lines arrive, and the counts the tab shows,
//! kept beside it so that reading them is a read and never a fold.
//!
//! This module is pure: no IO, no lock, no clock. Its children are not:
//! [`disk`] keeps the model on disk under `<root>/stats/` and `stats.lock`,
//! and [`maint`] is the upkeep step `ae app`'s thread runs.
//!
//! A source is a caller-chosen id over one journal; [`Store::chunk`]
//! consumes it in order from a byte cursor, and
//! [`Store::event`] takes a certificate or a linked `spawn` at its journal
//! offset as it is written, ahead of the cursor. What a source keeps is reduced:
//! a family record keeps its envelope and family members, a legacy `spawn`
//! only a hash of its line, every other record only its date. No prompt text
//! is kept.
//!
//! The counts are kept per session group. A group is re-counted, with the one
//! [`Fold`], from its own sources and the records other sources wrote about
//! it whenever its evidence changes; the groups' counts add up to the
//! all-sessions answer, and one session's answer is its group plus what the
//! session-less group says of every session.

use std::collections::{BTreeMap, BTreeSet};

use crate::archive::is_canonical_uuid;
use crate::birth::{self, Record};
use crate::events::Event;
use crate::json::{self, Value};
use crate::stats::{
    self, Cell, Fold, Gap, Reading, Reason, Row, Scoped, SourceGap, Tally, ThisSession, View,
};
use crate::time::Timestamp;

pub mod disk;
mod doc;
pub mod maint;
#[cfg(test)]
mod s2_spec;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    reason = "frozen S3 amendment: fixture IO removes and reads real store files"
)]
mod s3_absence;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 amendment: fixture IO changes and reads the real state root, and reuses the frozen S3 cases module"
)]
mod s3_archive_namespace;
#[cfg(test)]
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 amendment: reuses the frozen S3 cases module and its fold oracle, and matches store file names exactly"
)]
mod s3_census;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 amendment: fixture IO changes and reads the real state root, and reuses the frozen S3 cases module"
)]
mod s3_commit_source;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 amendment: reuses the frozen S3 cases module and its fold oracle"
)]
mod s3_failed_gap;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 amendment: reuses the frozen S3 cases module and its fold oracle"
)]
mod s3_fragment_absence;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 amendment: reuses the frozen S3 cases module and its fold oracle"
)]
mod s3_prune_evidence;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 amendment: reuses the frozen S3 cases module and its fold oracle"
)]
mod s3_prune_legacy;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 amendment: reuses the frozen S3 cases module and its fold oracle"
)]
mod s3_prune_view;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    reason = "frozen S3 amendment: fixture IO walks and reads the real store, and asserts one plan variant"
)]
mod s3_purge_recovery;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 amendment: fixture IO changes and reads the real state root, and reuses the frozen S3 cases module"
)]
mod s3_purge_retired;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    clippy::similar_names,
    reason = "frozen S3 amendment: reuses the frozen S3 cases module and its fold oracle; frozen binding names"
)]
mod s3_rebind_boundary;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::match_wildcard_for_single_variants,
    reason = "frozen S3 amendment: fixture IO changes the real archive tree"
)]
mod s3_sandwich;
#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    clippy::duplicate_mod,
    clippy::match_wildcard_for_single_variants,
    clippy::semicolon_if_nothing_returned,
    reason = "frozen S3 acceptance: fixture IO reads real files, shares the S2 fold oracle module, asserts one plan variant, and its frozen bytes are not reformatted"
)]
mod s3_spec;
#[cfg(test)]
mod s3_standing_files;
#[cfg(test)]
mod s3_standing_live;

/// The evidence format this core reads and writes.
const VERSION: i64 = 1;
/// The view format this core reads and writes.
const VIEW_VERSION: i64 = 1;
/// The longest line read, inclusive; a longer one is a `LineLimit` gap.
const LINE_LIMIT: usize = 4 * 1024 * 1024;
/// The bytes of a `YYYY-MM-DD` date.
const DATE_LEN: usize = 10;
/// The envelope members a family record keeps beside its family members.
const ENVELOPE: [&str; 6] = [
    "ts",
    "actor",
    "action",
    "target",
    "target_slot",
    "target_session_id",
];
/// The envelope members a legacy `spawn` keeps beside the hash of its line.
const LEGACY_ENVELOPE: [&str; 5] = ["ts", "actor", "action", "target", "target_slot"];
/// The member holding a legacy `spawn`'s line hash.
const LEGACY: &str = "legacy";
/// The announcement's journal action.
const SPAWN: &str = "spawn";

/// Why the store refused an operation or a document. Nothing changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No source has this id.
    UnknownSource(String),
    /// The source gave no journal: nothing can be read from it.
    FailedSource(String),
    /// A chunk that does not start at the source's cursor.
    Cursor { cursor: u64, start: u64 },
    /// An event that is not one certificate or linked `spawn` line.
    NotAnEvent,
    /// The offset arithmetic would pass `u64`.
    Offset,
    /// A persisted document this core does not accept, and why.
    Damaged(&'static str),
}

/// Where a source's records come from, and what it proves of its session.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    Live {
        label: String,
        session: Option<String>,
        seats: Vec<(String, String)>,
    },
    /// A live source whose meta is not text: it proves no identity and is
    /// counted unreadable, though its journal is still framed.
    Unreadable {
        label: String,
    },
    Archive {
        id: String,
    },
    Failed {
        label: String,
        session: Option<String>,
        gap: SourceGap,
    },
}

impl Origin {
    /// The group the source's own entries belong to.
    fn session(&self) -> Option<&str> {
        match self {
            Self::Live { session, .. } | Self::Failed { session, .. } => session.as_deref(),
            Self::Archive { id } => is_canonical_uuid(id).then_some(id.as_str()),
            Self::Unreadable { .. } => None,
        }
    }

    /// The label a reading resumes under, when the source has lines.
    fn reading(&self) -> Option<&str> {
        match self {
            Self::Live { label, .. } => Some(label),
            Self::Archive { id } => Some(id),
            Self::Unreadable { .. } | Self::Failed { .. } => None,
        }
    }
}

/// One kept record: its reduced line, and the other session it names, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    text: String,
    names: Option<String>,
}

/// One source's evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Evidence {
    origin: Origin,
    /// Bytes consumed in order: every line before it has been read.
    cursor: u64,
    /// Kept records by journal offset, sequential and event-time alike.
    lines: BTreeMap<u64, Line>,
    /// The offsets of the kept records naming each other session.
    named: BTreeMap<String, BTreeSet<u64>>,
    malformed: u64,
    line_limit: u64,
    /// The dates of the consumed records that keep nothing else.
    range: Option<(String, String)>,
}

impl Evidence {
    const fn new(origin: Origin) -> Self {
        Self {
            origin,
            cursor: 0,
            lines: BTreeMap::new(),
            named: BTreeMap::new(),
            malformed: 0,
            line_limit: 0,
            range: None,
        }
    }

    /// Every group this source's evidence reaches.
    fn groups(&self) -> BTreeSet<Option<String>> {
        let mut groups: BTreeSet<Option<String>> = self
            .named
            .keys()
            .map(|session| Some(session.clone()))
            .collect();
        groups.insert(self.origin.session().map(ToOwned::to_owned));
        groups
    }

    fn keep(&mut self, offset: u64, line: Line) {
        self.forget(offset);
        if let Some(session) = &line.names {
            self.named
                .entry(session.clone())
                .or_default()
                .insert(offset);
        }
        self.lines.insert(offset, line);
    }

    fn forget(&mut self, offset: u64) {
        let Some(old) = self.lines.remove(&offset) else {
            return;
        };
        if let Some(session) = old.names
            && let Some(offsets) = self.named.get_mut(&session)
        {
            offsets.remove(&offset);
            if offsets.is_empty() {
                self.named.remove(&session);
            }
        }
    }

    fn widen(&mut self, date: &str) {
        match &mut self.range {
            Some((from, to)) => {
                if date < from.as_str() {
                    date.clone_into(from);
                }
                if date > to.as_str() {
                    date.clone_into(to);
                }
            }
            empty => *empty = Some((date.to_owned(), date.to_owned())),
        }
    }

    /// A digested read applied: the event-time records in its range give way
    /// to the ordered read, and the cursor moves to its end.
    fn absorb(&mut self, digest: Digest) {
        let early: Vec<u64> = self
            .lines
            .range(digest.start..digest.end)
            .map(|(at, _)| *at)
            .collect();
        for offset in early {
            self.forget(offset);
        }
        if !matches!(self.origin, Origin::Unreadable { .. }) {
            for (offset, item) in digest.items {
                match item {
                    Item::OverLimit => self.line_limit = self.line_limit.saturating_add(1),
                    Item::Malformed => self.malformed = self.malformed.saturating_add(1),
                    Item::Dated(Some(date)) => self.widen(&date),
                    Item::Dated(None) => {}
                    Item::Kept(kept) => self.keep(offset, kept),
                }
            }
        }
        self.cursor = digest.end;
    }

    /// What a certificate or linked `spawn` `line` at `offset` adds, written
    /// as it happens: `None` when the ordered read has passed it, the source
    /// keeps no lines, or it is kept already.
    fn take_event(&self, offset: u64, line: &[u8], limit: usize) -> Result<Option<Line>, Error> {
        if line.len() > limit || line.contains(&b'\n') {
            return Err(Error::NotAnEvent);
        }
        let Class::Kept(kept, true) = classify(line, self.origin.session()) else {
            return Err(Error::NotAnEvent);
        };
        if offset < self.cursor
            || matches!(self.origin, Origin::Unreadable { .. })
            || self.lines.get(&offset) == Some(&kept)
        {
            return Ok(None);
        }
        Ok(Some(kept))
    }

    /// The whole source into `fold`: its reading, or its gap.
    fn add_whole(&self, fold: &mut Fold) {
        let reading = match &self.origin {
            Origin::Failed {
                label,
                session,
                gap,
            } => return fold.add(stats::Source::failed(label, session.as_deref(), *gap)),
            Origin::Unreadable { label } => {
                return fold.add(stats::Source::failed(label, None, SourceGap::Unreadable));
            }
            Origin::Live {
                label,
                session,
                seats,
            } => Reading::resume(label, session.as_deref(), seats.clone(), fold.room()),
            Origin::Archive { id } => {
                Reading::resume(id, self.origin.session(), vec![], fold.room())
            }
        };
        let mut reading = match reading {
            Ok(reading) => reading,
            Err(refused) => return fold.add(refused),
        };
        let range = self
            .range
            .as_ref()
            .map(|(from, to)| (from.as_str(), to.as_str()));
        reading.carry(self.malformed, self.line_limit, range);
        for line in self.lines.values() {
            if let Err(gap) = reading.line(line.text.as_bytes()) {
                return fold.add(reading.fail(gap));
            }
        }
        fold.add_reading(reading);
    }

    /// The records naming `group` into `fold`, read under this source's own
    /// label and session but with no roster: everything they make for `group`
    /// is what this source makes for it, and the rest is not `group`'s.
    fn add_fragment(&self, fold: &mut Fold, group: &str) {
        let (Some(label), Some(offsets)) = (self.origin.reading(), self.named.get(group)) else {
            return;
        };
        let Ok(mut reading) = Reading::resume(label, self.origin.session(), vec![], fold.room())
        else {
            return;
        };
        for offset in offsets {
            if let Some(line) = self.lines.get(offset)
                && reading.line(line.text.as_bytes()).is_err()
            {
                return;
            }
        }
        fold.add_reading(reading);
    }
}

/// What one parsed line leaves.
enum Class {
    Malformed,
    /// A record that keeps nothing but its date.
    Dated(Option<String>),
    /// A family record or a legacy `spawn`, reduced; and whether it is one the
    /// writer may hand over at event time.
    Kept(Line, bool),
}

/// One line of a read, judged before any store is touched.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    OverLimit,
    Malformed,
    Dated(Option<String>),
    Kept(Line),
}

/// A read framed and judged without the store, so that the judging can run
/// outside any lock: what [`Evidence::absorb`] applies at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Digest {
    start: u64,
    end: u64,
    items: Vec<(u64, Item)>,
}

impl Digest {
    /// `bytes` read from `start` under a source proving `session`: every line
    /// ending in LF, and with `eof` the unterminated last line too.
    pub(crate) fn of(
        session: Option<&str>,
        start: u64,
        bytes: &[u8],
        eof: bool,
        limit: usize,
    ) -> Result<Self, Error> {
        let consumed = match bytes.iter().rposition(|byte| *byte == b'\n') {
            _ if eof => bytes.len(),
            Some(last) => last + 1,
            None => 0,
        };
        let end = u64::try_from(consumed)
            .ok()
            .and_then(|consumed| start.checked_add(consumed))
            .ok_or(Error::Offset)?;
        let mut items = Vec::new();
        let mut at = start;
        let read = bytes.get(..consumed).unwrap_or_default();
        let mut lines = read.split(|byte| *byte == b'\n').peekable();
        while let Some(line) = lines.next() {
            if lines.peek().is_none() && line.is_empty() {
                break;
            }
            if let Some(item) = judge(line, session, limit) {
                items.push((at, item));
            }
            let width = u64::try_from(line.len()).unwrap_or(u64::MAX);
            at = at.saturating_add(width).saturating_add(1);
        }
        Ok(Self { start, end, items })
    }

    /// The bytes `[start, end)` of one line known to pass the limit: counted
    /// once, nothing else read.
    pub(crate) fn over_long(start: u64, end: u64) -> Self {
        Self {
            start,
            end,
            items: vec![(start, Item::OverLimit)],
        }
    }

    /// The bytes `[start, end)` of a line already counted over the limit:
    /// read past, nothing counted.
    pub(crate) const fn passed(start: u64, end: u64) -> Self {
        Self {
            start,
            end,
            items: Vec::new(),
        }
    }

    /// This read, its range widened back to `start` over bytes it passes.
    pub(crate) fn widened(mut self, start: u64) -> Self {
        self.start = start.min(self.start);
        self
    }

    /// Where the cursor stands once this read is applied.
    pub(crate) const fn end(&self) -> u64 {
        self.end
    }
}

/// What one line, LF excluded, leaves: nothing for a blank one.
fn judge(line: &[u8], session: Option<&str>, limit: usize) -> Option<Item> {
    if line.len() > limit {
        return Some(Item::OverLimit);
    }
    if line.trim_ascii().is_empty() {
        return None;
    }
    Some(match classify(line, session) {
        Class::Malformed => Item::Malformed,
        Class::Dated(date) => Item::Dated(date),
        Class::Kept(kept, _) => Item::Kept(kept),
    })
}

/// Read one line under a source proving `session`.
fn classify(line: &[u8], session: Option<&str>) -> Class {
    let Some((text, value, event)) = std::str::from_utf8(line).ok().and_then(|text| {
        let value = json::parse(text).ok()?;
        let event = Event::from_json(&value).ok()?;
        Some((text, value, event))
    }) else {
        return Class::Malformed;
    };
    let Value::Obj(fields) = value else {
        return Class::Malformed;
    };
    let decoded = birth::decode(&event);
    let reduced = match &decoded {
        Some(_) => keep_members(&fields, &ENVELOPE, &birth::MEMBERS),
        None if event.action == SPAWN => {
            let mut kept = keep_members(&fields, &LEGACY_ENVELOPE, &[]);
            kept.push((
                LEGACY.to_owned(),
                Value::str(crate::install::sha256_hex(text.as_bytes())),
            ));
            kept
        }
        None => return Class::Dated(date_of(&event)),
    };
    let stated = match &decoded {
        Some(Ok(Record::Birth(certificate))) => certificate.session.clone(),
        Some(Ok(Record::Fact(fact))) => fact.session.clone(),
        Some(Ok(Record::Unsupported { session, .. })) => session.clone(),
        Some(Err(damage)) => damage.session.clone(),
        Some(Ok(Record::SpawnLink { .. })) | None => None,
    };
    let names = stated.filter(|named| Some(named.as_str()) != session);
    let insertable = matches!(
        decoded,
        Some(Ok(Record::Birth(_) | Record::SpawnLink { .. }))
    );
    Class::Kept(
        Line {
            text: Value::Obj(reduced).render(),
            names,
        },
        insertable,
    )
}

/// The fields named in `envelope` or `members`, in order, repeats kept.
fn keep_members(
    fields: &[(String, Value)],
    envelope: &[&str],
    members: &[&str],
) -> Vec<(String, Value)> {
    fields
        .iter()
        .filter(|(key, _)| envelope.contains(&key.as_str()) || members.contains(&key.as_str()))
        .cloned()
        .collect()
}

/// A record's `YYYY-MM-DD`.
fn date_of(event: &Event) -> Option<String> {
    event.ts.to_string().get(..DATE_LEN).map(ToOwned::to_owned)
}

/// The kinds of persisted document the store parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Doc {
    Epoch,
    Intent,
    RootView,
    SessionView,
    Evidence,
    /// A document [`Store::parse`] reads.
    Store,
}

impl Doc {
    const fn index(self) -> usize {
        match self {
            Self::Epoch => 0,
            Self::Intent => 1,
            Self::RootView => 2,
            Self::SessionView => 3,
            Self::Evidence => 4,
            Self::Store => 5,
        }
    }
}

thread_local! {
    /// This thread's parses by [`Doc::index`].
    static PARSES: std::cell::Cell<[u64; 6]> = const { std::cell::Cell::new([0; 6]) };
}

/// This thread's count of parses of documents of `kind` that were attempted,
/// accepted or refused as damaged. A read refused over its cap adds nothing:
/// the parser is never reached.
#[must_use]
pub fn parse_count(kind: Doc) -> u64 {
    PARSES.with(|parses| parses.get().get(kind.index()).copied().unwrap_or_default())
}

/// One parse of `kind` begins: counted before any byte is examined.
fn counted(kind: Doc) {
    PARSES.with(|parses| {
        let mut held = parses.get();
        if let Some(count) = held.get_mut(kind.index()) {
            *count = count.saturating_add(1);
        }
        parses.set(held);
    });
}

/// The maintained store: every source's evidence and the counts it makes.
#[derive(Debug, Clone)]
pub struct Store {
    line_limit: usize,
    sources: BTreeMap<String, Evidence>,
    /// Each session group's counts, kept current with its evidence.
    groups: BTreeMap<Option<String>, Tally>,
    /// Every group added up: the all-sessions answer.
    total: Tally,
    /// What the session-less group says of every session.
    covering: Tally,
}

impl Default for Store {
    fn default() -> Self {
        Self::with_line_limit(LINE_LIMIT)
    }
}

impl Store {
    /// An empty store that reads lines up to `line` bytes, inclusive.
    #[must_use]
    pub fn with_line_limit(line: usize) -> Self {
        Self {
            line_limit: line,
            sources: BTreeMap::new(),
            groups: BTreeMap::new(),
            total: Tally::default(),
            covering: Tally::default(),
        }
    }

    /// A live session `id`: its `label` and raw meta. An existing `id` is
    /// replaced, its evidence dropped.
    ///
    /// # Errors
    ///
    /// None today; the signature leaves room for a refused source.
    pub fn live_source(&mut self, id: &str, label: &str, meta: &[u8]) -> Result<(), Error> {
        let origin = match stats::identity(meta) {
            Some((session, seats)) => Origin::Live {
                label: label.to_owned(),
                session,
                seats,
            },
            None => Origin::Unreadable {
                label: label.to_owned(),
            },
        };
        self.register(id, origin);
        Ok(())
    }

    /// A kept archive `id`, named by its session id `uuid`.
    ///
    /// # Errors
    ///
    /// None today; the signature leaves room for a refused source.
    pub fn archive_source(&mut self, id: &str, uuid: &str) -> Result<(), Error> {
        self.register(
            id,
            Origin::Archive {
                id: uuid.to_owned(),
            },
        );
        Ok(())
    }

    /// A source `id` that gave no journal, for `gap`.
    ///
    /// # Errors
    ///
    /// None today; the signature leaves room for a refused source.
    pub fn failed_source(
        &mut self,
        id: &str,
        label: &str,
        session: Option<&str>,
        gap: SourceGap,
    ) -> Result<(), Error> {
        self.register(
            id,
            Origin::Failed {
                label: label.to_owned(),
                session: session.map(ToOwned::to_owned),
                gap,
            },
        );
        Ok(())
    }

    fn register(&mut self, id: &str, origin: Origin) {
        let fresh = Evidence::new(origin);
        let mut touched = fresh.groups();
        if let Some(old) = self.sources.insert(id.to_owned(), fresh) {
            touched.extend(old.groups());
        }
        self.recount(touched);
    }

    /// The source `id` that can take lines.
    fn readable(&mut self, id: &str) -> Result<&mut Evidence, Error> {
        let evidence = self
            .sources
            .get_mut(id)
            .ok_or_else(|| Error::UnknownSource(id.to_owned()))?;
        if matches!(evidence.origin, Origin::Failed { .. }) {
            return Err(Error::FailedSource(id.to_owned()));
        }
        Ok(evidence)
    }

    /// A certificate or linked `spawn` `line` (LF excluded) of source `id` as
    /// it is written at journal `offset`. Idempotent, and nothing once the
    /// cursor has passed `offset`: the ordered read keeps it then.
    ///
    /// # Errors
    ///
    /// An unknown or failed source, or a line that is not such a record.
    pub fn event(&mut self, id: &str, offset: u64, line: &[u8]) -> Result<(), Error> {
        let limit = self.line_limit;
        let evidence = self.readable(id)?;
        let Some(kept) = evidence.take_event(offset, line, limit)? else {
            return Ok(());
        };
        let mut touched = evidence.groups();
        evidence.keep(offset, kept);
        touched.extend(evidence.groups());
        self.recount(touched);
        Ok(())
    }

    /// Bytes of source `id` from `start`, which must be its cursor. Every line
    /// ending in LF is read, and with `eof` the unterminated last line too.
    /// Returns the new cursor; the caller hands any tail over again from it.
    ///
    /// # Errors
    ///
    /// An unknown or failed source, or `start` other than the cursor.
    pub fn chunk(&mut self, id: &str, start: u64, bytes: &[u8], eof: bool) -> Result<u64, Error> {
        let limit = self.line_limit;
        let evidence = self.readable(id)?;
        if start != evidence.cursor {
            return Err(Error::Cursor {
                cursor: evidence.cursor,
                start,
            });
        }
        let digest = Digest::of(evidence.origin.session(), start, bytes, eof, limit)?;
        let end = digest.end;
        let mut touched = evidence.groups();
        evidence.absorb(digest);
        touched.extend(evidence.groups());
        self.recount(touched);
        Ok(end)
    }

    /// Drop source `id` and everything it counted. An absent `id` is fine.
    ///
    /// # Errors
    ///
    /// None today; the signature leaves room for a refused removal.
    pub fn remove(&mut self, id: &str) -> Result<(), Error> {
        if let Some(old) = self.sources.remove(id) {
            self.recount(old.groups());
        }
        Ok(())
    }

    /// Re-count each touched group from its evidence, then the sums.
    fn recount(&mut self, touched: BTreeSet<Option<String>>) {
        for group in touched {
            let tally = self.count(group.as_deref());
            if tally == Tally::default() {
                self.groups.remove(&group);
            } else {
                self.groups.insert(group, tally);
            }
        }
        let mut total = Tally::default();
        for tally in self.groups.values() {
            total.add(tally);
        }
        self.total = total;
        self.covering = self
            .groups
            .get(&None)
            .map(Tally::covering)
            .unwrap_or_default();
    }

    /// One group's counts: its own sources whole, and what every other source
    /// wrote about it.
    fn count(&self, group: Option<&str>) -> Tally {
        let mut fold = Fold::default();
        for evidence in self.sources.values() {
            if evidence.origin.session() == group {
                evidence.add_whole(&mut fold);
            } else if let Some(group) = group {
                evidence.add_fragment(&mut fold, group);
            }
        }
        fold.finish_exact(group)
    }

    /// Both scopes, from the counts kept: no evidence is read.
    #[must_use]
    pub fn view(&self, this: Option<&str>) -> View {
        let all = self.total.scoped();
        let this = this.map_or(ThisSession::NoUuid, |id| {
            let mut scope = self.covering.clone();
            if let Some(own) = self.groups.get(&Some(id.to_owned())) {
                scope.add(own);
            }
            ThisSession::Ready(Box::new(scope.scoped()))
        });
        View { all, this }
    }

    /// The evidence as text, one JSON record a line.
    ///
    /// # Errors
    ///
    /// None today; the signature leaves room for a store too large to write.
    pub fn render(&self) -> Result<String, Error> {
        let mut out = String::new();
        line(
            &mut out,
            &[
                ("stats_store", Value::Num(VERSION)),
                (
                    "line_limit",
                    count(u64::try_from(self.line_limit).unwrap_or(u64::MAX)),
                ),
            ],
        );
        for (id, evidence) in &self.sources {
            render_source(&mut out, id, evidence);
        }
        Ok(out)
    }

    /// Evidence written by [`Store::render`], counted again.
    ///
    /// # Errors
    ///
    /// [`Error::Damaged`] for anything [`Store::render`] would not write.
    pub fn parse(text: &str) -> Result<Self, Error> {
        counted(Doc::Store);
        let body = text
            .strip_suffix('\n')
            .ok_or(Error::Damaged("no final LF"))?;
        let mut records = body.split('\n');
        let header = object(records.next().ok_or(Error::Damaged("empty"))?)?;
        expect_keys(&header, &["stats_store", "line_limit"])?;
        if header.first().map(|(_, value)| value) != Some(&Value::Num(VERSION)) {
            return Err(Error::Damaged("version"));
        }
        let limit = number(field(&header, "line_limit")?)?;
        let mut store = Self::with_line_limit(
            usize::try_from(limit).map_err(|_| Error::Damaged("line limit"))?,
        );
        let mut current: Option<(String, Evidence)> = None;
        for record in records {
            let fields = object(record)?;
            match fields.first().map(|(key, _)| key.as_str()) {
                Some("source") => {
                    if let Some((id, evidence)) = current.take() {
                        store.adopt(id, evidence)?;
                    }
                    current = Some(parse_source(&fields)?);
                }
                Some("at") => {
                    let (_, evidence) = current
                        .as_mut()
                        .ok_or(Error::Damaged("line before source"))?;
                    parse_line(&fields, evidence)?;
                }
                _ => return Err(Error::Damaged("record")),
            }
        }
        if let Some((id, evidence)) = current.take() {
            store.adopt(id, evidence)?;
        }
        let groups: BTreeSet<Option<String>> =
            store.sources.values().flat_map(Evidence::groups).collect();
        store.recount(groups);
        Ok(store)
    }

    fn adopt(&mut self, id: String, evidence: Evidence) -> Result<(), Error> {
        if let Some((last, _)) = self.sources.last_key_value()
            && last.as_str() >= id.as_str()
        {
            return Err(Error::Damaged("source order"));
        }
        self.sources.insert(id, evidence);
        Ok(())
    }
}

/// One record line.
fn line(out: &mut String, fields: &[(&str, Value)]) {
    Value::obj(fields.iter().map(|(key, value)| (*key, value.clone()))).render_into(out);
    out.push('\n');
}

/// A count as JSON: an integer when it fits, else its digits verbatim.
fn count(value: u64) -> Value {
    i64::try_from(value).map_or_else(|_| Value::Raw(value.to_string()), Value::Num)
}

/// The count a [`count`] value holds.
fn number(value: &Value) -> Result<u64, Error> {
    match value {
        Value::Num(number) => u64::try_from(*number).map_err(|_| Error::Damaged("negative")),
        Value::Raw(digits)
            if digits.bytes().all(|byte| byte.is_ascii_digit()) && !digits.starts_with('0') =>
        {
            digits.parse().map_err(|_| Error::Damaged("number"))
        }
        _ => Err(Error::Damaged("number")),
    }
}

/// One record line's fields.
fn object(text: &str) -> Result<Vec<(String, Value)>, Error> {
    match json::parse(text) {
        Ok(Value::Obj(fields)) => Ok(fields),
        _ => Err(Error::Damaged("not an object")),
    }
}

/// Exactly these keys, in this order: what the writer writes and no more.
fn expect_keys(fields: &[(String, Value)], keys: &[&str]) -> Result<(), Error> {
    let written: Vec<&str> = fields.iter().map(|(key, _)| key.as_str()).collect();
    if written == keys {
        Ok(())
    } else {
        Err(Error::Damaged("keys"))
    }
}

fn field<'a>(fields: &'a [(String, Value)], key: &str) -> Result<&'a Value, Error> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .ok_or(Error::Damaged("missing key"))
}

fn text<'a>(fields: &'a [(String, Value)], key: &str) -> Result<&'a str, Error> {
    field(fields, key)?
        .as_str()
        .ok_or(Error::Damaged("not text"))
}

/// The source names a gap's word.
const GAPS: [(SourceGap, &str); 7] = [
    (SourceGap::Missing, "missing"),
    (SourceGap::Unreadable, "unreadable"),
    (SourceGap::Oversized, "oversized"),
    (SourceGap::Budget, "budget"),
    (SourceGap::Invalid, "invalid"),
    (SourceGap::Claimed, "claimed"),
    (SourceGap::Retained, "retained"),
];

fn gap_word(gap: SourceGap) -> &'static str {
    GAPS.iter()
        .find(|(known, _)| *known == gap)
        .map_or("missing", |(_, word)| word)
}

fn gap_of(word: &str) -> Result<SourceGap, Error> {
    GAPS.iter()
        .find(|(_, known)| *known == word)
        .map(|(gap, _)| *gap)
        .ok_or(Error::Damaged("gap"))
}

fn optional(value: Option<&str>) -> Value {
    value.map_or(Value::Null, Value::str)
}

fn optional_text(value: &Value) -> Result<Option<String>, Error> {
    match value {
        Value::Null => Ok(None),
        Value::Str(text) => Ok(Some(text.clone())),
        _ => Err(Error::Damaged("not text")),
    }
}

fn render_source(out: &mut String, id: &str, evidence: &Evidence) {
    line(out, &source_fields(id, evidence));
    render_lines(out, evidence);
}

/// A source record's fields, in the order [`parse_source`] expects them.
fn source_fields<'a>(id: &'a str, evidence: &'a Evidence) -> Vec<(&'static str, Value)> {
    let mut fields = vec![("source", Value::str(id))];
    match &evidence.origin {
        Origin::Live {
            label,
            session,
            seats,
        } => {
            fields.push(("kind", Value::str("live")));
            fields.push(("label", Value::str(label.as_str())));
            fields.push(("session", optional(session.as_deref())));
            let seats = seats
                .iter()
                .map(|(slot, name)| {
                    Value::Arr(vec![Value::str(slot.as_str()), Value::str(name.as_str())])
                })
                .collect();
            fields.push(("seats", Value::Arr(seats)));
        }
        Origin::Unreadable { label } => {
            fields.push(("kind", Value::str("unreadable")));
            fields.push(("label", Value::str(label.as_str())));
        }
        Origin::Archive { id: uuid } => {
            fields.push(("kind", Value::str("archive")));
            fields.push(("label", Value::str(uuid.as_str())));
        }
        Origin::Failed {
            label,
            session,
            gap,
        } => {
            fields.push(("kind", Value::str("failed")));
            fields.push(("label", Value::str(label.as_str())));
            fields.push(("session", optional(session.as_deref())));
            fields.push(("gap", Value::str(gap_word(*gap))));
        }
    }
    let (from, to) = evidence.range.as_ref().map_or((None, None), |(from, to)| {
        (Some(from.as_str()), Some(to.as_str()))
    });
    fields.extend([
        ("cursor", count(evidence.cursor)),
        ("malformed", count(evidence.malformed)),
        ("over_limit", count(evidence.line_limit)),
        ("from", optional(from)),
        ("to", optional(to)),
    ]);
    fields
}

/// A source's kept records, one `at` record each, in offset order.
fn render_lines(out: &mut String, evidence: &Evidence) {
    for (offset, kept) in &evidence.lines {
        line(
            out,
            &[
                ("at", count(*offset)),
                ("line", Value::str(kept.text.as_str())),
            ],
        );
    }
}

const TAIL: [&str; 5] = ["cursor", "malformed", "over_limit", "from", "to"];

fn parse_source(fields: &[(String, Value)]) -> Result<(String, Evidence), Error> {
    parse_source_with(fields, &[])
}

/// [`parse_source`] of a record that carries `extra` keys after its own, which
/// the caller reads.
fn parse_source_with(
    fields: &[(String, Value)],
    extra: &[&str],
) -> Result<(String, Evidence), Error> {
    let id = text(fields, "source")?.to_owned();
    let label = text(fields, "label")?.to_owned();
    let kind = text(fields, "kind")?;
    let with = |head: &[&str]| -> Vec<String> {
        head.iter()
            .chain(TAIL.iter())
            .chain(extra.iter())
            .map(|key| (*key).to_owned())
            .collect()
    };
    let (keys, origin) = match kind {
        "live" => {
            let session = optional_text(field(fields, "session")?)?;
            if session.as_deref().is_some_and(|id| !is_canonical_uuid(id)) {
                return Err(Error::Damaged("session"));
            }
            let Value::Arr(list) = field(fields, "seats")? else {
                return Err(Error::Damaged("seats"));
            };
            let mut seats = Vec::new();
            for seat in list {
                match seat {
                    Value::Arr(pair) => match pair.as_slice() {
                        [Value::Str(slot), Value::Str(name)] => {
                            seats.push((slot.clone(), name.clone()));
                        }
                        _ => return Err(Error::Damaged("seat")),
                    },
                    _ => return Err(Error::Damaged("seat")),
                }
            }
            (
                with(&["source", "kind", "label", "session", "seats"]),
                Origin::Live {
                    label,
                    session,
                    seats,
                },
            )
        }
        "unreadable" => (
            with(&["source", "kind", "label"]),
            Origin::Unreadable { label },
        ),
        "archive" => (
            with(&["source", "kind", "label"]),
            Origin::Archive { id: label },
        ),
        "failed" => (
            with(&["source", "kind", "label", "session", "gap"]),
            Origin::Failed {
                label,
                session: optional_text(field(fields, "session")?)?,
                gap: gap_of(text(fields, "gap")?)?,
            },
        ),
        _ => return Err(Error::Damaged("kind")),
    };
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    expect_keys(fields, &keys)?;
    let mut evidence = Evidence::new(origin);
    evidence.cursor = number(field(fields, "cursor")?)?;
    evidence.malformed = number(field(fields, "malformed")?)?;
    evidence.line_limit = number(field(fields, "over_limit")?)?;
    evidence.range = match (
        optional_text(field(fields, "from")?)?,
        optional_text(field(fields, "to")?)?,
    ) {
        (None, None) => None,
        (Some(from), Some(to)) if is_date(&from) && is_date(&to) && from <= to => Some((from, to)),
        _ => return Err(Error::Damaged("range")),
    };
    let silent = !matches!(
        evidence.origin,
        Origin::Live { .. } | Origin::Archive { .. }
    );
    if silent && (evidence.malformed, evidence.line_limit, &evidence.range) != (0, 0, &None) {
        return Err(Error::Damaged("counts of a source with no lines"));
    }
    if matches!(evidence.origin, Origin::Failed { .. }) && evidence.cursor != 0 {
        return Err(Error::Damaged("cursor of a failed source"));
    }
    Ok((id, evidence))
}

fn parse_line(fields: &[(String, Value)], evidence: &mut Evidence) -> Result<(), Error> {
    expect_keys(fields, &["at", "line"])?;
    if evidence.origin.reading().is_none() {
        return Err(Error::Damaged("line of a source with no lines"));
    }
    let offset = number(field(fields, "at")?)?;
    if evidence
        .lines
        .last_key_value()
        .is_some_and(|(last, _)| *last >= offset)
    {
        return Err(Error::Damaged("line order"));
    }
    let kept = text(fields, "line")?;
    let Class::Kept(line, _) = classify(kept.as_bytes(), evidence.origin.session()) else {
        return Err(Error::Damaged("not a kept record"));
    };
    if !is_reduced(kept, &line.text) {
        return Err(Error::Damaged("not a reduced record"));
    }
    let line = Line {
        text: kept.to_owned(),
        names: line.names,
    };
    evidence.keep(offset, line);
    Ok(())
}

/// Whether `kept` is a line [`classify`] writes: a family record reduces to
/// itself; a legacy `spawn` is its envelope and one line hash, last.
fn is_reduced(kept: &str, again: &str) -> bool {
    if kept == again {
        return true;
    }
    let Ok(Value::Obj(mut fields)) = json::parse(kept) else {
        return false;
    };
    let Some((key, Value::Str(hash))) = fields.pop() else {
        return false;
    };
    let hex = hash.len() == 64
        && hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    let spawn = fields
        .iter()
        .any(|(key, value)| key == "action" && value.as_str() == Some(SPAWN));
    key == LEGACY
        && hex
        && spawn
        && fields
            .iter()
            .all(|(key, _)| LEGACY_ENVELOPE.contains(&key.as_str()))
        && {
            let mut again = fields;
            again.push((LEGACY.to_owned(), Value::Str(hash)));
            Value::Obj(again).render() == kept
        }
}

/// Whether `text` is a `YYYY-MM-DD` a record's timestamp can carry: the
/// calendar check is [`Timestamp::parse`]'s own.
fn is_date(text: &str) -> bool {
    text.len() == DATE_LEN && Timestamp::parse(&format!("{text}T00:00:00Z")).is_some()
}

// --- the materialized view ------------------------------------------------

/// A view as text, one JSON record a line.
#[must_use]
pub fn render_view(view: &View) -> String {
    let mut out = String::new();
    line(&mut out, &[("stats_view", Value::Num(VIEW_VERSION))]);
    render_scope(&mut out, "all", &view.all);
    match &view.this {
        ThisSession::Ready(this) => render_scope(&mut out, "this", this),
        ThisSession::NoUuid => line(
            &mut out,
            &[("scope", Value::str("this")), ("none", Value::Bool(true))],
        ),
    }
    out
}

fn render_scope(out: &mut String, scope: &str, scoped: &Scoped) {
    let (from, to) = scoped.range.as_ref().map_or((None, None), |(from, to)| {
        (Some(from.as_str()), Some(to.as_str()))
    });
    line(
        out,
        &[
            ("scope", Value::str(scope)),
            ("total", count(scoped.total)),
            ("legacy", count(scoped.legacy)),
            ("model_known", count(scoped.model_known)),
            ("effort_known", count(scoped.effort_known)),
            ("from", optional(from)),
            ("to", optional(to)),
            ("sources_read", count(scoped.sources_read)),
            ("sources_total", count(scoped.sources_total)),
        ],
    );
    for (name, rows) in [
        ("harness", &scoped.harness),
        ("client", &scoped.client),
        ("model_effort", &scoped.model_effort),
        ("joint", &scoped.joint),
    ] {
        for row in rows {
            let cells = row.cells.iter().map(cell_value).collect();
            line(
                out,
                &[
                    ("scope", Value::str(scope)),
                    ("table", Value::str(name)),
                    ("cells", Value::Arr(cells)),
                    ("count", count(row.count)),
                ],
            );
        }
    }
    for gap in &scoped.gaps {
        let mut fields = vec![
            ("scope", Value::str(scope)),
            ("gap", Value::str(reason_word(&gap.reason))),
        ];
        match &gap.reason {
            Reason::Source(source) => fields.push(("of", Value::str(gap_word(*source)))),
            Reason::LegacyExcluded { sessions, a, b } => {
                fields.extend([
                    ("sessions", count(*sessions)),
                    ("a", count(*a)),
                    ("b", count(*b)),
                ]);
            }
            Reason::Unproved { word, .. } => fields.push(("word", Value::str(word.as_str()))),
            _ => {}
        }
        fields.push(("source", optional(gap.source.as_deref())));
        fields.push(("count", count(gap.count)));
        line(out, &fields);
    }
}

/// A cell as JSON: a value is `=` and the value, the rest their word.
fn cell_value(cell: &Cell) -> Value {
    match cell {
        Cell::Value(value) => Value::Str(format!("={value}")),
        other => Value::str(other.word()),
    }
}

fn cell_of(value: &Value) -> Result<Cell, Error> {
    let text = value.as_str().ok_or(Error::Damaged("cell"))?;
    match text {
        "none set" => Ok(Cell::NoneSet),
        "direct" => Ok(Cell::Direct),
        "unproved" => Ok(Cell::Unproved),
        "unknown" => Ok(Cell::Unknown),
        _ => text
            .strip_prefix('=')
            .map(|value| Cell::Value(value.to_owned()))
            .ok_or(Error::Damaged("cell")),
    }
}

/// The reasons a view names, by word.
fn reason_word(reason: &Reason) -> &'static str {
    match reason {
        Reason::Source(_) => "source",
        Reason::Malformed => "malformed",
        Reason::Damaged => "damaged",
        Reason::Unsupported => "unsupported",
        Reason::CertificateLost => "certificate_lost",
        Reason::Conflict => "conflict",
        Reason::SessionMismatch => "session_mismatch",
        Reason::NoSessionId => "no_session_id",
        Reason::LiveNoBirth => "live_no_birth",
        Reason::LegacyExcluded { .. } => "legacy_excluded",
        Reason::Unproved { model: true, .. } => "model_unproved",
        Reason::Unproved { model: false, .. } => "effort_unproved",
        Reason::LineLimit => "line_limit",
    }
}

/// The keys each reason's gap record carries between `gap` and `source`.
fn reason_of(word: &str, fields: &[(String, Value)]) -> Result<(Reason, Vec<&'static str>), Error> {
    let plain = |reason| Ok((reason, vec![]));
    match word {
        "source" => Ok((Reason::Source(gap_of(text(fields, "of")?)?), vec!["of"])),
        "malformed" => plain(Reason::Malformed),
        "damaged" => plain(Reason::Damaged),
        "unsupported" => plain(Reason::Unsupported),
        "certificate_lost" => plain(Reason::CertificateLost),
        "conflict" => plain(Reason::Conflict),
        "session_mismatch" => plain(Reason::SessionMismatch),
        "no_session_id" => plain(Reason::NoSessionId),
        "live_no_birth" => plain(Reason::LiveNoBirth),
        "line_limit" => plain(Reason::LineLimit),
        "legacy_excluded" => Ok((
            Reason::LegacyExcluded {
                sessions: number(field(fields, "sessions")?)?,
                a: number(field(fields, "a")?)?,
                b: number(field(fields, "b")?)?,
            },
            vec!["sessions", "a", "b"],
        )),
        "model_unproved" | "effort_unproved" => Ok((
            Reason::Unproved {
                model: word == "model_unproved",
                word: text(fields, "word")?.to_owned(),
            },
            vec!["word"],
        )),
        _ => Err(Error::Damaged("reason")),
    }
}

/// A view written by [`render_view`], checked whole.
///
/// # Errors
///
/// [`Error::Damaged`] for any view a fold could not have made.
pub fn parse_view(text: &str) -> Result<View, Error> {
    let body = text
        .strip_suffix('\n')
        .ok_or(Error::Damaged("no final LF"))?;
    let mut records = body.split('\n');
    let header = object(records.next().ok_or(Error::Damaged("empty"))?)?;
    if header != [("stats_view".to_owned(), Value::Num(VIEW_VERSION))] {
        return Err(Error::Damaged("version"));
    }
    let mut all: Option<Scoped> = None;
    let mut this: Option<ThisSession> = None;
    for record in records {
        let fields = object(record)?;
        let scope = match fields.first() {
            Some((key, Value::Str(scope))) if key == "scope" => scope.as_str(),
            _ => return Err(Error::Damaged("record")),
        };
        match (scope, all.is_some(), this.is_some()) {
            ("all", false, false) => all = Some(parse_head(&fields)?),
            ("this", true, false) => {
                this = Some(
                    if fields.get(1).map(|(key, _)| key.as_str()) == Some("none") {
                        expect_keys(&fields, &["scope", "none"])?;
                        if field(&fields, "none")? != &Value::Bool(true) {
                            return Err(Error::Damaged("none"));
                        }
                        ThisSession::NoUuid
                    } else {
                        ThisSession::Ready(Box::new(parse_head(&fields)?))
                    },
                );
            }
            ("all", true, false) => {
                let scoped = all.as_mut().ok_or(Error::Damaged("scope order"))?;
                parse_entry(&fields, scoped)?;
            }
            ("this", true, true) => match this.as_mut() {
                Some(ThisSession::Ready(scoped)) => parse_entry(&fields, scoped)?,
                _ => return Err(Error::Damaged("entry of no scope")),
            },
            _ => return Err(Error::Damaged("scope order")),
        }
    }
    let (Some(all), Some(this)) = (all, this) else {
        return Err(Error::Damaged("missing scope"));
    };
    check(&all)?;
    if let ThisSession::Ready(scoped) = &this {
        check(scoped)?;
        within(scoped, &all)?;
    }
    Ok(View { all, this })
}

fn parse_head(fields: &[(String, Value)]) -> Result<Scoped, Error> {
    expect_keys(
        fields,
        &[
            "scope",
            "total",
            "legacy",
            "model_known",
            "effort_known",
            "from",
            "to",
            "sources_read",
            "sources_total",
        ],
    )?;
    let range = match (
        optional_text(field(fields, "from")?)?,
        optional_text(field(fields, "to")?)?,
    ) {
        (None, None) => None,
        (Some(from), Some(to)) => Some((from, to)),
        _ => return Err(Error::Damaged("range")),
    };
    Ok(Scoped {
        total: number(field(fields, "total")?)?,
        legacy: number(field(fields, "legacy")?)?,
        model_known: number(field(fields, "model_known")?)?,
        effort_known: number(field(fields, "effort_known")?)?,
        range,
        sources_read: number(field(fields, "sources_read")?)?,
        sources_total: number(field(fields, "sources_total")?)?,
        ..Scoped::default()
    })
}

fn parse_entry(fields: &[(String, Value)], scoped: &mut Scoped) -> Result<(), Error> {
    match fields.get(1).map(|(key, _)| key.as_str()) {
        Some("table") => {
            expect_keys(fields, &["scope", "table", "cells", "count"])?;
            let Value::Arr(cells) = field(fields, "cells")? else {
                return Err(Error::Damaged("cells"));
            };
            let row = Row {
                cells: cells.iter().map(cell_of).collect::<Result<_, _>>()?,
                count: number(field(fields, "count")?)?,
            };
            let table = match text(fields, "table")? {
                "harness" => &mut scoped.harness,
                "client" => &mut scoped.client,
                "model_effort" => &mut scoped.model_effort,
                "joint" => &mut scoped.joint,
                _ => return Err(Error::Damaged("table")),
            };
            table.push(row);
        }
        Some("gap") => {
            let (reason, extra) = reason_of(text(fields, "gap")?, fields)?;
            let mut keys = vec!["scope", "gap"];
            keys.extend(extra);
            keys.extend(["source", "count"]);
            expect_keys(fields, &keys)?;
            scoped.gaps.push(Gap {
                source: optional_text(field(fields, "source")?)?,
                reason,
                count: number(field(fields, "count")?)?,
            });
        }
        _ => return Err(Error::Damaged("entry")),
    }
    Ok(())
}

/// Whether a scope is one a fold could make: every table the joint table's
/// projection, in order; every counter its share of the joint table; every gap
/// once, in order; dates in order.
fn check(scoped: &Scoped) -> Result<(), Error> {
    let damaged = Err(Error::Damaged("inconsistent scope"));
    if scoped
        .joint
        .iter()
        .any(|row| row.cells.len() != 4 || row.count == 0)
    {
        return damaged;
    }
    let mut joint: BTreeMap<[Cell; 4], u64> = BTreeMap::new();
    for row in &scoped.joint {
        let cells: [Cell; 4] = row
            .cells
            .clone()
            .try_into()
            .map_err(|_| Error::Damaged("width"))?;
        if joint.insert(cells, row.count).is_some() {
            return damaged;
        }
    }
    let tally = Tally::of_joint(&joint).ok_or(Error::Damaged("overflow"))?;
    let expected = tally.scoped();
    let total = expected.total;
    let sum = |test: &dyn Fn(&[Cell; 4]) -> bool| -> u64 {
        joint
            .iter()
            .filter(|(cells, _)| test(cells))
            .map(|(_, count)| *count)
            .sum()
    };
    let unknown = sum(&|cells| cells.iter().all(|cell| *cell == Cell::Unknown));
    let unproved = |index: usize| sum(&|cells: &[Cell; 4]| cells[index] == Cell::Unproved);
    let flagged = |model: bool| -> Option<u64> {
        scoped
            .gaps
            .iter()
            .filter(|gap| matches!(&gap.reason, Reason::Unproved { model: m, .. } if *m == model))
            .try_fold(0_u64, |sum, gap| sum.checked_add(gap.count))
    };
    let tables_match = expected.harness == scoped.harness
        && expected.client == scoped.client
        && expected.model_effort == scoped.model_effort
        && expected.joint == scoped.joint;
    let counters_match = scoped.total == total
        && scoped.legacy <= unknown
        && scoped.model_known == expected.model_known
        && scoped.effort_known == expected.effort_known
        && scoped.sources_read <= scoped.sources_total
        && flagged(true) == Some(unproved(2))
        && flagged(false) == Some(unproved(3));
    let range_ok = scoped
        .range
        .as_ref()
        .is_none_or(|(from, to)| is_date(from) && is_date(to) && from <= to);
    if !(tables_match && counters_match && range_ok) {
        return damaged;
    }
    let mut excluded = 0;
    let mut last: Option<(&Reason, &Option<String>)> = None;
    for gap in &scoped.gaps {
        if gap.count == 0 || last.is_some_and(|before| before >= (&gap.reason, &gap.source)) {
            return damaged;
        }
        last = Some((&gap.reason, &gap.source));
        match &gap.reason {
            Reason::LegacyExcluded { sessions, a, b } => {
                excluded += 1;
                if gap.source.is_some() || gap.count != *sessions || a > b {
                    return damaged;
                }
            }
            Reason::Unproved { .. } if gap.source.is_some() => return damaged,
            _ => {}
        }
    }
    if excluded > 1 {
        return damaged;
    }
    Ok(())
}

/// Whether one session's scope fits inside the all-sessions scope.
fn within(this: &Scoped, all: &Scoped) -> Result<(), Error> {
    let counters = this.total <= all.total
        && this.legacy <= all.legacy
        && this.model_known <= all.model_known
        && this.effort_known <= all.effort_known
        && this.sources_read <= all.sources_read
        && this.sources_total <= all.sources_total;
    let rows = this.joint.iter().all(|row| {
        all.joint
            .iter()
            .any(|other| other.cells == row.cells && other.count >= row.count)
    });
    let gaps = this.gaps.iter().all(|gap| match &gap.reason {
        Reason::LegacyExcluded { sessions, a, b } => all.gaps.iter().any(|other| {
            matches!(&other.reason, Reason::LegacyExcluded { sessions: s, a: x, b: y }
                if s >= sessions && x >= a && y >= b)
        }),
        reason => all.gaps.iter().any(|other| {
            other.reason == *reason && other.source == gap.source && other.count >= gap.count
        }),
    });
    let range = match (&this.range, &all.range) {
        (None, _) => true,
        (Some((from, to)), Some((first, last))) => first <= from && to <= last,
        (Some(_), None) => false,
    };
    if counters && rows && gaps && range {
        Ok(())
    } else {
        Err(Error::Damaged("this scope outside all"))
    }
}
