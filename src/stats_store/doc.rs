//! The maintained store's documents on disk, pure: what each payload file
//! holds, written and read back whole. No IO; [`super::disk`] decides which
//! file holds which document and when it is replaced.
//!
//! Every document but the epoch is one JSON record a line and ends in an
//! `{"end":N}` record naming the records before it, so a file cut short at a
//! line boundary reads as damaged rather than as a smaller store.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Doc, Error, Evidence, Origin, count, counted, expect_keys, field, line, number, object,
    optional, optional_text, parse_entry, parse_head, parse_line, parse_source_with, render_lines,
    render_scope, source_fields, text,
};
use crate::archive::is_canonical_uuid;
use crate::json::Value;
use crate::stats::{Fold, Scoped, Tally};

/// The format of every document this core writes.
const FORMAT: i64 = 1;

/// The bytes of a journal a source's witness hashes, ending at its cursor.
pub(super) const WITNESS: u64 = 4096;

/// A session group's key: its session id, or `None` for the id-less group.
pub(super) type Key = Option<String>;

/// The file stem of a group's documents: its session id, or `none`.
pub(super) fn stem(key: &Key) -> &str {
    key.as_deref().unwrap_or("none")
}

/// The group a file stem names; `None` for a stem no group has.
pub(super) fn key_of(stem: &str) -> Option<Key> {
    match stem {
        "none" => Some(None),
        id if is_canonical_uuid(id) => Some(Some(id.to_owned())),
        _ => None,
    }
}

fn key_value(key: &Key) -> Value {
    optional(key.as_deref())
}

fn parse_key(value: &Value) -> Result<Key, Error> {
    let key = optional_text(value)?;
    if key.as_deref().is_some_and(|id| !is_canonical_uuid(id)) {
        return Err(Error::Damaged("group"));
    }
    Ok(key)
}

fn flag(value: &Value) -> Result<bool, Error> {
    match value {
        Value::Bool(on) => Ok(*on),
        _ => Err(Error::Damaged("flag")),
    }
}

/// A hex SHA-256, as a witness carries it.
fn is_hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// The SHA-256 of `bytes`, hex.
pub(super) fn hash(bytes: &[u8]) -> String {
    crate::install::sha256_hex(bytes)
}

/// One record of a document: its fields in order.
type Record = Vec<(String, Value)>;

/// Records of a document, its final LF and its `end` record checked and taken
/// off.
fn records(text: &str) -> Result<Vec<Record>, Error> {
    let body = text
        .strip_suffix('\n')
        .ok_or(Error::Damaged("no final LF"))?;
    let mut records = body
        .split('\n')
        .map(object)
        .collect::<Result<Vec<_>, _>>()?;
    let end = records.pop().ok_or(Error::Damaged("empty"))?;
    expect_keys(&end, &["end"])?;
    let written = u64::try_from(records.len()).map_err(|_| Error::Damaged("end"))?;
    if number(field(&end, "end")?)? != written {
        return Err(Error::Damaged("end"));
    }
    if records.is_empty() {
        return Err(Error::Damaged("empty"));
    }
    Ok(records)
}

/// `out`, closed by its `end` record.
fn close(mut out: String) -> String {
    let written = u64::try_from(out.matches('\n').count()).unwrap_or(u64::MAX);
    line(&mut out, &[("end", count(written))]);
    out
}

// --- the epoch -------------------------------------------------------------

/// The epoch as its file holds it.
pub(super) fn render_epoch(epoch: u64) -> String {
    format!("{epoch}\n")
}

/// An epoch file: one decimal number and LF, no sign, no leading zero.
pub(super) fn parse_epoch(bytes: &[u8]) -> Result<u64, Error> {
    counted(Doc::Epoch);
    let digits = bytes.strip_suffix(b"\n").ok_or(Error::Damaged("epoch"))?;
    let canonical = !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit)
        && (digits == b"0" || !digits.starts_with(b"0"));
    if !canonical {
        return Err(Error::Damaged("epoch"));
    }
    std::str::from_utf8(digits)
        .ok()
        .and_then(|digits| digits.parse().ok())
        .ok_or(Error::Damaged("epoch"))
}

// --- the intent ------------------------------------------------------------

/// A publication in flight: which groups it rewrites, and the one whose
/// evidence decides it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Intent {
    pub(super) epoch: u64,
    pub(super) generation: u64,
    pub(super) primary: Key,
    pub(super) touched: BTreeSet<Key>,
}

pub(super) fn render_intent(intent: &Intent) -> String {
    let mut out = String::new();
    let touched = intent.touched.iter().map(key_value).collect();
    line(
        &mut out,
        &[
            ("stats_intent", Value::Num(FORMAT)),
            ("epoch", count(intent.epoch)),
            ("gen", count(intent.generation)),
            ("primary", key_value(&intent.primary)),
            ("touched", Value::Arr(touched)),
        ],
    );
    close(out)
}

pub(super) fn parse_intent(input: &str) -> Result<Intent, Error> {
    counted(Doc::Intent);
    let records = records(input)?;
    let [head] = records.as_slice() else {
        return Err(Error::Damaged("intent records"));
    };
    expect_keys(
        head,
        &["stats_intent", "epoch", "gen", "primary", "touched"],
    )?;
    if field(head, "stats_intent")? != &Value::Num(FORMAT) {
        return Err(Error::Damaged("version"));
    }
    let Value::Arr(list) = field(head, "touched")? else {
        return Err(Error::Damaged("touched"));
    };
    let mut touched = BTreeSet::new();
    for key in list {
        if !touched.insert(parse_key(key)?) {
            return Err(Error::Damaged("touched"));
        }
    }
    let primary = parse_key(field(head, "primary")?)?;
    if !touched.contains(&primary) {
        return Err(Error::Damaged("primary"));
    }
    Ok(Intent {
        epoch: number(field(head, "epoch")?)?,
        generation: number(field(head, "gen")?)?,
        primary,
        touched,
    })
}

// --- a group's evidence ------------------------------------------------------

/// Where a source's reading stands in its journal: the stamp a prepared read
/// is compared with, and the witness of the bytes it consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Provenance {
    /// The generation at which the source was registered or last reset.
    pub(super) incarnation: u64,
    /// The journal's length when the source was last read.
    pub(super) seen: u64,
    /// Where that read ended: at `seen`, nothing past the cursor ends a line.
    pub(super) read: u64,
    pub(super) dev: u64,
    pub(super) ino: u64,
    /// The SHA-256 of the at most [`WITNESS`] bytes ending at the cursor.
    pub(super) hash: String,
    /// Inside a line already counted over the limit: the next read passes
    /// through its first LF without counting.
    pub(super) skipping: bool,
}

impl Provenance {
    /// A source that has no journal to witness.
    pub(super) fn none(incarnation: u64) -> Self {
        Self {
            incarnation,
            seen: 0,
            read: 0,
            dev: 0,
            ino: 0,
            hash: hash(b""),
            skipping: false,
        }
    }
}

const PROVENANCE: [&str; 7] = ["inc", "seen", "read", "dev", "ino", "hash", "skipping"];

/// One of a group's own sources: its evidence and its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Own {
    pub(super) evidence: Evidence,
    pub(super) provenance: Provenance,
}

/// One session group's evidence: its own sources whole, and a copy of what
/// every other group's sources wrote about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Group {
    pub(super) key: Key,
    pub(super) own: BTreeMap<String, Own>,
    /// Other groups' sources, each with only its records naming this group.
    pub(super) fragments: BTreeMap<String, Evidence>,
}

impl Group {
    pub(super) const fn new(key: Key) -> Self {
        Self {
            key,
            own: BTreeMap::new(),
            fragments: BTreeMap::new(),
        }
    }

    /// The group's counts: its own sources whole and the fragments naming it,
    /// read in source order as one store reads them.
    pub(super) fn tally(&self) -> Tally {
        let mut sources: Vec<(&String, &Evidence, bool)> = self
            .own
            .iter()
            .map(|(id, own)| (id, &own.evidence, true))
            .chain(self.fragments.iter().map(|(id, part)| (id, part, false)))
            .collect();
        sources.sort_by(|left, right| left.0.cmp(right.0));
        let mut fold = Fold::default();
        for (_, evidence, whole) in sources {
            if whole {
                evidence.add_whole(&mut fold);
            } else if let Some(group) = self.key.as_deref() {
                evidence.add_fragment(&mut fold, group);
            }
        }
        fold.finish_exact(self.key.as_deref())
    }

    /// The fragments of `primary`'s sources, taken again from its evidence:
    /// whatever those sources wrote about this group, and nothing else.
    pub(super) fn refragment(&mut self, primary: &Self) {
        let mine = primary.key.as_deref();
        self.fragments
            .retain(|_, part| part.origin.session() != mine);
        let Some(group) = self.key.as_deref() else {
            return;
        };
        for (id, own) in &primary.own {
            if let Some(part) = own.evidence.fragment(group) {
                self.fragments.insert(id.clone(), part);
            }
        }
    }

    /// The rows a session view keeps of the own sources.
    pub(super) fn rows(&self) -> BTreeMap<String, SourceRow> {
        self.own
            .iter()
            .map(|(id, own)| {
                (
                    id.clone(),
                    SourceRow {
                        failed: matches!(own.evidence.origin, Origin::Failed { .. }),
                        cursor: own.evidence.cursor,
                        provenance: own.provenance.clone(),
                    },
                )
            })
            .collect()
    }
}

impl Evidence {
    /// This source's records naming `group`, under its own label and
    /// session: what another group keeps of it. `None` when it names none.
    fn fragment(&self, group: &str) -> Option<Self> {
        let offsets = self.named.get(group)?;
        let origin = match &self.origin {
            Origin::Live { label, session, .. } => Origin::Live {
                label: label.clone(),
                session: session.clone(),
                seats: Vec::new(),
            },
            Origin::Archive { id } => Origin::Archive { id: id.clone() },
            Origin::Unreadable { .. } | Origin::Failed { .. } => return None,
        };
        let mut part = Self::new(origin);
        for offset in offsets {
            if let Some(kept) = self.lines.get(offset) {
                part.keep(*offset, kept.clone());
            }
        }
        Some(part)
    }
}

pub(super) fn render_group(group: &Group, epoch: u64) -> String {
    let mut out = String::new();
    line(
        &mut out,
        &[
            ("stats_evidence", Value::Num(FORMAT)),
            ("epoch", count(epoch)),
            ("group", key_value(&group.key)),
        ],
    );
    for (id, own) in &group.own {
        let mut fields = source_fields(id, &own.evidence);
        let mark = &own.provenance;
        fields.extend([
            ("inc", count(mark.incarnation)),
            ("seen", count(mark.seen)),
            ("read", count(mark.read)),
            ("dev", count(mark.dev)),
            ("ino", count(mark.ino)),
            ("hash", Value::str(mark.hash.as_str())),
            ("skipping", Value::Bool(mark.skipping)),
        ]);
        line(&mut out, &fields);
        render_lines(&mut out, &own.evidence);
    }
    for (id, part) in &group.fragments {
        let mut fields = vec![("fragment", Value::str(id.as_str()))];
        match &part.origin {
            Origin::Live { label, session, .. } => fields.extend([
                ("kind", Value::str("live")),
                ("label", Value::str(label.as_str())),
                ("session", optional(session.as_deref())),
            ]),
            Origin::Archive { id: uuid } => fields.extend([
                ("kind", Value::str("archive")),
                ("label", Value::str(uuid.as_str())),
            ]),
            Origin::Unreadable { .. } | Origin::Failed { .. } => continue,
        }
        line(&mut out, &fields);
        render_lines(&mut out, part);
    }
    close(out)
}

/// A group's evidence and the epoch it was written in.
pub(super) fn parse_group(input: &str) -> Result<(Group, u64), Error> {
    counted(Doc::Evidence);
    let records = records(input)?;
    let (head, body) = records.split_first().ok_or(Error::Damaged("empty"))?;
    expect_keys(head, &["stats_evidence", "epoch", "group"])?;
    if field(head, "stats_evidence")? != &Value::Num(FORMAT) {
        return Err(Error::Damaged("version"));
    }
    let epoch = number(field(head, "epoch")?)?;
    let mut group = Group::new(parse_key(field(head, "group")?)?);
    // The record the next `at` lines belong to: own (true) or a fragment.
    let mut current: Option<(String, bool)> = None;
    for fields in body {
        match fields.first().map(|(key, _)| key.as_str()) {
            Some("source") => {
                if !group.fragments.is_empty() {
                    return Err(Error::Damaged("source after fragments"));
                }
                let (id, evidence) = parse_source_with(fields, &PROVENANCE)?;
                if evidence.origin.session() != group.key.as_deref() {
                    return Err(Error::Damaged("source of another group"));
                }
                if group
                    .own
                    .last_key_value()
                    .is_some_and(|(last, _)| *last >= id)
                {
                    return Err(Error::Damaged("source order"));
                }
                let provenance = parse_provenance(fields)?;
                if matches!(evidence.origin, Origin::Failed { .. }) && provenance.skipping {
                    return Err(Error::Damaged("skipping a source with no lines"));
                }
                current = Some((id.clone(), true));
                group.own.insert(
                    id,
                    Own {
                        evidence,
                        provenance,
                    },
                );
            }
            Some("fragment") => {
                let (id, part) = parse_fragment(fields, &group.key)?;
                if group.own.contains_key(&id)
                    || group
                        .fragments
                        .last_key_value()
                        .is_some_and(|(last, _)| *last >= id)
                {
                    return Err(Error::Damaged("fragment order"));
                }
                current = Some((id.clone(), false));
                group.fragments.insert(id, part);
            }
            Some("at") => {
                let (id, own) = current
                    .as_ref()
                    .ok_or(Error::Damaged("line before source"))?;
                let evidence = if *own {
                    group.own.get_mut(id).map(|own| &mut own.evidence)
                } else {
                    group.fragments.get_mut(id)
                }
                .ok_or(Error::Damaged("line before source"))?;
                parse_line(fields, evidence)?;
                if !*own {
                    let names = group.key.as_deref().ok_or(Error::Damaged("fragment"))?;
                    if evidence.named.keys().any(|named| named != names)
                        || evidence.lines.len() != evidence.named.values().map(BTreeSet::len).sum()
                    {
                        return Err(Error::Damaged("fragment line naming another group"));
                    }
                }
            }
            _ => return Err(Error::Damaged("record")),
        }
    }
    if group.fragments.values().any(|part| part.lines.is_empty()) {
        return Err(Error::Damaged("empty fragment"));
    }
    Ok((group, epoch))
}

fn parse_provenance(fields: &[(String, Value)]) -> Result<Provenance, Error> {
    let hash = text(fields, "hash")?;
    if !is_hash(hash) {
        return Err(Error::Damaged("hash"));
    }
    Ok(Provenance {
        incarnation: number(field(fields, "inc")?)?,
        seen: number(field(fields, "seen")?)?,
        read: number(field(fields, "read")?)?,
        dev: number(field(fields, "dev")?)?,
        ino: number(field(fields, "ino")?)?,
        hash: hash.to_owned(),
        skipping: flag(field(fields, "skipping")?)?,
    })
}

fn parse_fragment(fields: &[(String, Value)], key: &Key) -> Result<(String, Evidence), Error> {
    if key.is_none() {
        return Err(Error::Damaged("fragment of the id-less group"));
    }
    let id = text(fields, "fragment")?.to_owned();
    let label = text(fields, "label")?.to_owned();
    let origin = match text(fields, "kind")? {
        "live" => {
            expect_keys(fields, &["fragment", "kind", "label", "session"])?;
            let session = optional_text(field(fields, "session")?)?;
            if session.as_deref().is_some_and(|id| !is_canonical_uuid(id)) {
                return Err(Error::Damaged("session"));
            }
            Origin::Live {
                label,
                session,
                seats: Vec::new(),
            }
        }
        "archive" => {
            expect_keys(fields, &["fragment", "kind", "label"])?;
            Origin::Archive { id: label }
        }
        _ => return Err(Error::Damaged("kind")),
    };
    if origin.session() == key.as_deref() {
        return Err(Error::Damaged("fragment of the group itself"));
    }
    Ok((id, Evidence::new(origin)))
}

// --- a group's session view --------------------------------------------------

/// What a session view keeps of one own source: enough to prepare its next
/// read without opening the evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SourceRow {
    /// A source that gave no journal when registered.
    pub(super) failed: bool,
    pub(super) cursor: u64,
    pub(super) provenance: Provenance,
}

/// One group's counts, kept beside its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SessionView {
    pub(super) epoch: u64,
    pub(super) generation: u64,
    pub(super) key: Key,
    pub(super) tally: Tally,
    pub(super) rows: BTreeMap<String, SourceRow>,
}

impl SessionView {
    pub(super) fn of(group: &Group, epoch: u64, generation: u64) -> Self {
        Self {
            epoch,
            generation,
            key: group.key.clone(),
            tally: group.tally(),
            rows: group.rows(),
        }
    }

    pub(super) fn pending(&self) -> u64 {
        let pending = self
            .rows
            .values()
            .filter(|row| row.provenance.skipping || row.cursor < row.provenance.seen)
            .count();
        u64::try_from(pending).unwrap_or(u64::MAX)
    }

    pub(super) fn listed(&self) -> u64 {
        u64::try_from(self.rows.len()).unwrap_or(u64::MAX)
    }
}

pub(super) fn render_session(view: &SessionView) -> String {
    let mut out = String::new();
    line(
        &mut out,
        &[
            ("stats_session", Value::Num(FORMAT)),
            ("epoch", count(view.epoch)),
            ("gen", count(view.generation)),
            ("group", key_value(&view.key)),
        ],
    );
    render_scope(&mut out, "tally", &view.tally.scoped());
    for (id, row) in &view.rows {
        let mark = &row.provenance;
        line(
            &mut out,
            &[
                ("row", Value::str(id.as_str())),
                ("failed", Value::Bool(row.failed)),
                ("cursor", count(row.cursor)),
                ("inc", count(mark.incarnation)),
                ("seen", count(mark.seen)),
                ("read", count(mark.read)),
                ("dev", count(mark.dev)),
                ("ino", count(mark.ino)),
                ("hash", Value::str(mark.hash.as_str())),
                ("skipping", Value::Bool(mark.skipping)),
            ],
        );
    }
    close(out)
}

pub(super) fn parse_session(input: &str) -> Result<SessionView, Error> {
    counted(Doc::SessionView);
    let records = records(input)?;
    let (head, body) = records.split_first().ok_or(Error::Damaged("empty"))?;
    expect_keys(head, &["stats_session", "epoch", "gen", "group"])?;
    if field(head, "stats_session")? != &Value::Num(FORMAT) {
        return Err(Error::Damaged("version"));
    }
    let (tally, rest) = scope_of(body, "tally")?;
    let mut rows = BTreeMap::new();
    for fields in rest {
        expect_keys(
            fields,
            &[
                "row", "failed", "cursor", "inc", "seen", "read", "dev", "ino", "hash", "skipping",
            ],
        )?;
        let id = text(fields, "row")?.to_owned();
        if rows
            .last_key_value()
            .is_some_and(|(last, _): (&String, _)| *last >= id)
        {
            return Err(Error::Damaged("row order"));
        }
        let row = SourceRow {
            failed: flag(field(fields, "failed")?)?,
            cursor: number(field(fields, "cursor")?)?,
            provenance: parse_provenance(fields)?,
        };
        rows.insert(id, row);
    }
    Ok(SessionView {
        epoch: number(field(head, "epoch")?)?,
        generation: number(field(head, "gen")?)?,
        key: parse_key(field(head, "group")?)?,
        tally,
        rows,
    })
}

/// The leading records of `records` that lay out scope `name`, as the counts
/// they were laid out from, and the records after them.
fn scope_of<'a>(records: &'a [Record], name: &str) -> Result<(Tally, &'a [Record]), Error> {
    let of_scope = |fields: &Record| matches!(fields.first(), Some((key, Value::Str(scope))) if key == "scope" && scope == name);
    let width = records.iter().take_while(|fields| of_scope(fields)).count();
    let (mine, rest) = records.split_at(width);
    let (head, entries) = mine.split_first().ok_or(Error::Damaged("missing scope"))?;
    let mut scoped: Scoped = parse_head(head)?;
    for entry in entries {
        parse_entry(entry, &mut scoped)?;
    }
    super::check(&scoped)?;
    let tally = Tally::of_scoped(&scoped).ok_or(Error::Damaged("scope no count makes"))?;
    Ok((tally, rest))
}

// --- the root view -----------------------------------------------------------

/// What the root keeps of one group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RootRow {
    pub(super) generation: u64,
    pub(super) listed: u64,
    pub(super) pending: u64,
    pub(super) range: Option<(String, String)>,
}

impl RootRow {
    pub(super) fn of(view: &SessionView) -> Self {
        Self {
            generation: view.generation,
            listed: view.listed(),
            pending: view.pending(),
            range: view.tally.range().cloned(),
        }
    }
}

/// Every group added up: the all-sessions answer, what the id-less group says
/// of every session, and one row per group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Root {
    pub(super) epoch: u64,
    pub(super) generation: u64,
    pub(super) all: Tally,
    pub(super) covering: Tally,
    pub(super) rows: BTreeMap<Key, RootRow>,
}

impl Root {
    pub(super) fn empty(epoch: u64) -> Self {
        Self {
            epoch,
            generation: 0,
            all: Tally::default(),
            covering: Tally::default(),
            rows: BTreeMap::new(),
        }
    }

    /// The dates every group spans together.
    pub(super) fn range(&self) -> Option<(String, String)> {
        let mut span: Option<(String, String)> = None;
        for (from, to) in self.rows.values().filter_map(|row| row.range.as_ref()) {
            span = Some(match span {
                None => (from.clone(), to.clone()),
                Some((first, last)) => (first.min(from.clone()), last.max(to.clone())),
            });
        }
        span
    }

    pub(super) fn pending(&self) -> u64 {
        self.rows
            .values()
            .fold(0_u64, |sum, row| sum.saturating_add(row.pending))
    }

    pub(super) fn listed(&self) -> u64 {
        self.rows
            .values()
            .fold(0_u64, |sum, row| sum.saturating_add(row.listed))
    }

    /// The root the session views add up to, every group's counts read again.
    pub(super) fn sum<'a>(
        epoch: u64,
        generation: u64,
        views: impl IntoIterator<Item = &'a SessionView>,
    ) -> Self {
        let mut root = Self::empty(epoch);
        root.generation = generation;
        for view in views {
            root.all.add(&view.tally);
            if view.key.is_none() {
                root.covering = view.tally.covering();
            }
            root.rows.insert(view.key.clone(), RootRow::of(view));
        }
        root
    }
}

pub(super) fn render_root(root: &Root) -> String {
    let mut out = String::new();
    line(
        &mut out,
        &[
            ("stats_root", Value::Num(FORMAT)),
            ("epoch", count(root.epoch)),
            ("gen", count(root.generation)),
        ],
    );
    render_scope(&mut out, "all", &root.all.scoped());
    render_scope(&mut out, "covering", &root.covering.scoped());
    for (key, row) in &root.rows {
        let (from, to) = row.range.as_ref().map_or((None, None), |(from, to)| {
            (Some(from.as_str()), Some(to.as_str()))
        });
        line(
            &mut out,
            &[
                ("group", key_value(key)),
                ("gen", count(row.generation)),
                ("sources", count(row.listed)),
                ("pending", count(row.pending)),
                ("from", optional(from)),
                ("to", optional(to)),
            ],
        );
    }
    close(out)
}

pub(super) fn parse_root(input: &str) -> Result<Root, Error> {
    counted(Doc::RootView);
    let records = records(input)?;
    let (head, body) = records.split_first().ok_or(Error::Damaged("empty"))?;
    expect_keys(head, &["stats_root", "epoch", "gen"])?;
    if field(head, "stats_root")? != &Value::Num(FORMAT) {
        return Err(Error::Damaged("version"));
    }
    let (all, rest) = scope_of(body, "all")?;
    let (covering, rest) = scope_of(rest, "covering")?;
    let mut rows = BTreeMap::new();
    for fields in rest {
        expect_keys(
            fields,
            &["group", "gen", "sources", "pending", "from", "to"],
        )?;
        let key = parse_key(field(fields, "group")?)?;
        if rows
            .last_key_value()
            .is_some_and(|(last, _): (&Key, _)| *last >= key)
        {
            return Err(Error::Damaged("group order"));
        }
        let range = match (
            optional_text(field(fields, "from")?)?,
            optional_text(field(fields, "to")?)?,
        ) {
            (None, None) => None,
            (Some(from), Some(to)) if from <= to => Some((from, to)),
            _ => return Err(Error::Damaged("range")),
        };
        let row = RootRow {
            generation: number(field(fields, "gen")?)?,
            listed: number(field(fields, "sources")?)?,
            pending: number(field(fields, "pending")?)?,
            range,
        };
        if row.pending > row.listed {
            return Err(Error::Damaged("pending"));
        }
        rows.insert(key, row);
    }
    let root = Root {
        epoch: number(field(head, "epoch")?)?,
        generation: number(field(head, "gen")?)?,
        all,
        covering,
        rows,
    };
    if root.all.range() != root.range().as_ref() {
        return Err(Error::Damaged("range of the groups"));
    }
    Ok(root)
}
