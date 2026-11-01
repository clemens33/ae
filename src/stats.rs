//! The Statistics tab's fold: recorded worker spawns by configured route.
//!
//! Pure: journal bytes in, a [`View`] out, no IO and no clock. A spawn is
//! identified by `(session id, birth id)`, so the same journal seen live and in
//! an archive, or under two names, counts once. A `spawn` with no link is legacy:
//! each is one spawn of unknown route, compared whole, line by line, because a
//! legacy record has no id. Nothing here reads the current meta for a value: a
//! spawn's route is what its certificate wrote, or `unknown`.
//!
//! Prompt text is never kept past the read: a legacy line stays only until the
//! fold finishes, and no [`View`] member carries one.

use std::collections::{BTreeMap, BTreeSet};

use crate::archive::is_canonical_uuid;
use crate::birth::{self, Damage, Fact, Family, Record};
use crate::events::Event;
use crate::meta::Meta;

/// The announcement's journal action.
const SPAWN: &str = "spawn";
/// The roster slot prefix of a seat a spawn made.
const SPAWNED: &str = "spawned.";
/// The meta key of a session's durable id.
const SESSION_ID: &str = "session_id=";
/// The bytes of a `YYYY-MM-DD` date.
const DATE_LEN: usize = 10;

/// Why a source gave no bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceGap {
    Missing,
    Unreadable,
    Oversized,
    Budget,
    Invalid,
    Claimed,
}

/// One journal as the reader found it.
pub struct Source {
    label: String,
    session: Option<String>,
    journal: Result<Vec<u8>, SourceGap>,
    /// `(slot, name)` of each spawned seat the live roster lists.
    seats: Vec<(String, String)>,
}

impl Source {
    /// A live session: `name`, its raw meta and its journal. Unreadable meta
    /// proves no identity, so its journal is not attributed.
    #[must_use]
    pub fn live(name: &str, meta: &[u8], journal: Result<Vec<u8>, SourceGap>) -> Self {
        let Ok(text) = std::str::from_utf8(meta) else {
            return Self::failed(name, None, SourceGap::Unreadable);
        };
        let seats = Meta::parse(text)
            .roster()
            .iter()
            .filter(|seat| seat.slot.starts_with(SPAWNED))
            .map(|seat| (seat.slot.clone(), seat.name.clone()))
            .collect();
        Self {
            label: name.to_owned(),
            session: session_id(text),
            journal,
            seats,
        }
    }

    /// A kept archive, named by its session id.
    #[must_use]
    pub fn archive(id: &str, journal: Result<Vec<u8>, SourceGap>) -> Self {
        Self {
            label: id.to_owned(),
            session: is_canonical_uuid(id).then(|| id.to_owned()),
            journal,
            seats: Vec::new(),
        }
    }

    /// A source that gave nothing.
    #[must_use]
    pub fn failed(label: &str, session: Option<&str>, gap: SourceGap) -> Self {
        Self {
            label: label.to_owned(),
            session: session.map(ToOwned::to_owned),
            journal: Err(gap),
            seats: Vec::new(),
        }
    }
}

/// The id a meta proves: exactly one `session_id` line, canonical.
fn session_id(meta: &str) -> Option<String> {
    let mut ids = meta
        .split('\n')
        .filter_map(|line| line.strip_prefix(SESSION_ID));
    match (ids.next(), ids.next()) {
        (Some(id), None) if is_canonical_uuid(id) => Some(id.to_owned()),
        _ => None,
    }
}

/// `(session id, birth id)`: one worker's birth.
type Key = (Option<String>, String);
/// A cell, and the word a [`Cell::Unproved`] flag gave.
type Flag = (Cell, Option<String>);
/// Harness, client, model and effort, as one source wrote them.
type Claim = [Flag; 4];

/// The index of the model flag in a [`Claim`].
const MODEL: usize = 2;
/// The index of the effort flag in a [`Claim`].
const EFFORT: usize = 3;

/// A claim that proves nothing.
fn unknown() -> Claim {
    std::array::from_fn(|_| (Cell::Unknown, None))
}

/// What one certificate wrote. A flag's absence is a fact of the command.
fn claim(birth: &birth::Birth) -> Claim {
    let flag = |value: &Option<String>, word: &Option<String>| match (value, word) {
        (Some(value), _) => (Cell::Value(value.clone()), None),
        (None, Some(word)) => (Cell::Unproved, Some(word.clone())),
        (None, None) => (Cell::NoneSet, None),
    };
    let cell =
        |value: &Option<String>, absent: Cell| (value.clone().map_or(absent, Cell::Value), None);
    [
        cell(&birth.tool, Cell::Unknown),
        cell(&birth.client, Cell::Direct),
        flag(&birth.model, &birth.model_unproved),
        flag(&birth.effort, &birth.effort_unproved),
    ]
}

/// Every source's claims on one birth. Sets, so that order is no evidence.
#[derive(Default)]
struct Certificates {
    claims: BTreeSet<Claim>,
    /// The least label of a source holding it, whichever came first.
    label: Option<String>,
    damaged: bool,
    unsupported: bool,
}

impl Certificates {
    /// Each cell's one value. A cell two sources disagree on is unknown and a
    /// conflict; a cell one source cannot vouch for is unknown.
    fn settle(&self) -> (Claim, bool) {
        let mut settled = unknown();
        let mut conflict = false;
        for (index, cell) in settled.iter_mut().enumerate() {
            let flags: BTreeSet<&Flag> = self.claims.iter().map(|claim| &claim[index]).collect();
            let stated = flags.iter().filter(|flag| flag.0 != Cell::Unknown).count();
            conflict |= stated > 1;
            if let [sole] = flags.into_iter().collect::<Vec<_>>().as_slice() {
                cell.clone_from(sole);
            }
        }
        (settled, conflict)
    }
}

/// One source's coverage facts.
struct Seen {
    session: Option<String>,
    read: bool,
    range: Option<(String, String)>,
}

/// One coverage entry. `session` scopes it, and an entry with none reaches every
/// scope: what it failed to prove may be about any session.
struct Note {
    session: Option<String>,
    source: Option<String>,
    reason: Reason,
    count: u64,
}

/// What a roster or a journal names, per session.
#[derive(Default)]
struct Named {
    slots: BTreeSet<String>,
    names: BTreeSet<String>,
}

/// A live roster to check against [`Named`] once every source is in.
struct Roster {
    session: Option<String>,
    label: String,
    seats: Vec<(String, String)>,
}

/// The sources read so far: everything [`Fold::finish`] needs, and no journal.
#[derive(Default)]
pub struct Fold {
    seen: Vec<Seen>,
    certificates: BTreeMap<Key, Certificates>,
    /// Each source's facts, per birth.
    facts: BTreeMap<Key, Vec<Vec<Fact>>>,
    /// Links no certificate in their own source answers, with their least label.
    links: BTreeMap<Key, String>,
    /// Each identified source's legacy lines, per session.
    legacy: BTreeMap<String, Vec<Vec<String>>>,
    /// Legacy counts of sources with no session id: nothing to compare them by.
    loose: u64,
    notes: Vec<Note>,
    rosters: Vec<Roster>,
    named: BTreeMap<Option<String>, Named>,
}

impl Fold {
    /// Read one source. Its bytes are gone when this returns.
    pub fn add(&mut self, source: Source) {
        let Source {
            label,
            session,
            journal,
            seats,
        } = source;
        match journal {
            Ok(bytes) => {
                let read = self.read(&label, session.as_ref(), &bytes);
                self.seen.push(Seen {
                    session: session.clone(),
                    read: true,
                    range: read,
                });
                if session.is_none() {
                    self.note(None, Some(&label), Reason::NoSessionId, 1);
                }
                self.rosters.push(Roster {
                    session,
                    label,
                    seats,
                });
            }
            Err(gap) => {
                self.note(session.clone(), Some(&label), Reason::Source(gap), 1);
                self.seen.push(Seen {
                    session,
                    read: false,
                    range: None,
                });
            }
        }
    }

    fn note(&mut self, session: Option<String>, source: Option<&str>, reason: Reason, count: u64) {
        self.notes.push(Note {
            session,
            source: source.map(ToOwned::to_owned),
            reason,
            count,
        });
    }

    /// Every record of one readable journal; its date range back.
    fn read(
        &mut self,
        label: &str,
        session: Option<&String>,
        bytes: &[u8],
    ) -> Option<(String, String)> {
        let mut pass = Pass::new(label, session);
        let mut malformed = 0;
        for line in bytes.split(|byte| *byte == b'\n') {
            if line.trim_ascii().is_empty() {
                continue;
            }
            let parsed = std::str::from_utf8(line)
                .ok()
                .and_then(|text| Some((text, Event::parse_line(text).ok()?)));
            match parsed {
                Some((text, event)) => pass.record(text, &event),
                None => malformed += 1,
            }
        }
        if malformed > 0 {
            self.note(session.cloned(), Some(label), Reason::Malformed, malformed);
        }
        self.absorb(pass)
    }

    /// One journal's reading into the fold.
    fn absorb(&mut self, pass: Pass<'_>) -> Option<(String, String)> {
        let Pass {
            label,
            session,
            range,
            claims,
            facts,
            links,
            legacy,
            notes,
            named,
        } = pass;
        let session = session.cloned();
        let own: BTreeSet<&str> = claims.iter().map(|(key, _)| key.1.as_str()).collect();
        for birth in links.iter().filter(|birth| !own.contains(birth.as_str())) {
            least(
                self.links
                    .entry((session.clone(), birth.clone()))
                    .or_insert_with(|| label.to_owned()),
                label,
            );
        }
        for (key, claim) in claims {
            let entry = self.certificates.entry(key).or_default();
            entry.claims.insert(claim.claim);
            entry.damaged |= claim.damaged;
            entry.unsupported |= claim.unsupported;
            match &mut entry.label {
                Some(held) => least(held, label),
                empty => *empty = Some(label.to_owned()),
            }
        }
        for (key, list) in facts {
            self.facts.entry(key).or_default().push(list);
        }
        match &session {
            Some(id) if !legacy.is_empty() => {
                self.legacy.entry(id.clone()).or_default().push(legacy);
            }
            _ => self.loose += u64::try_from(legacy.len()).unwrap_or(u64::MAX),
        }
        for (subject, reason) in notes {
            self.note(subject, Some(label), reason, 1);
        }
        let held = self.named.entry(session).or_default();
        held.slots.extend(named.slots);
        held.names.extend(named.names);
        range
    }

    /// Both scopes, from everything read.
    #[must_use]
    pub fn finish(self, this: Option<&str>) -> View {
        let ledger = self.settle();
        let all = ledger.scoped(None);
        debug_assert!(all.balanced(), "every table sums to the total");
        let this = this.map_or(ThisSession::NoUuid, |id| {
            ThisSession::Ready(Box::new(ledger.scoped(Some(id))))
        });
        View { all, this }
    }

    /// The fold's one judgment: which spawns exist and what each proves.
    fn settle(mut self) -> Ledger {
        let mut spawns = Vec::new();
        for ((session, birth), held) in &self.certificates {
            let label = held.label.as_deref();
            let (claim, split) = held.settle();
            let diverged = self
                .facts
                .get(&(session.clone(), birth.clone()))
                .is_some_and(|lists| !agree(lists));
            for (flagged, reason) in [
                (split || diverged, Reason::Conflict),
                (held.damaged, Reason::Damaged),
                (held.unsupported, Reason::Unsupported),
            ] {
                if flagged {
                    self.notes.push(Note {
                        session: session.clone(),
                        source: label.map(ToOwned::to_owned),
                        reason,
                        count: 1,
                    });
                }
            }
            spawns.push(Spawn::of(session.clone(), claim));
        }
        for ((session, birth), label) in &self.links {
            if self
                .certificates
                .contains_key(&(session.clone(), birth.clone()))
            {
                continue;
            }
            self.notes.push(Note {
                session: session.clone(),
                source: Some(label.clone()),
                reason: Reason::CertificateLost,
                count: 1,
            });
            spawns.push(Spawn::of(session.clone(), unknown()));
        }
        let mut excluded = Vec::new();
        for (session, lists) in &self.legacy {
            let longest = lists.iter().max_by_key(|list| list.len());
            let shortest = lists.iter().map(Vec::len).min().unwrap_or(0);
            let longest = longest.map_or(&[][..], Vec::as_slice);
            if lists.iter().all(|list| subsequence(list, longest)) {
                spawns.extend(longest.iter().map(|_| Spawn::legacy(Some(session.clone()))));
            } else {
                excluded.push((session.clone(), shortest, longest.len()));
            }
        }
        spawns.extend((0..self.loose).map(|_| Spawn::legacy(None)));
        for roster in std::mem::take(&mut self.rosters) {
            let named = self.named.get(&roster.session);
            let silent = roster.seats.iter().filter(|(slot, name)| {
                !named.is_some_and(|named| named.slots.contains(slot) || named.names.contains(name))
            });
            let count = u64::try_from(silent.count()).unwrap_or(u64::MAX);
            if count > 0 {
                self.note(
                    roster.session,
                    Some(&roster.label),
                    Reason::LiveNoBirth,
                    count,
                );
            }
        }
        Ledger {
            seen: self.seen,
            spawns,
            notes: self.notes,
            excluded,
        }
    }
}

/// Lower `held` to `label`, keeping each birth's label order-free.
fn least(held: &mut String, label: &str) {
    if label < held.as_str() {
        label.clone_into(held);
    }
}

/// One source's claim on a birth while it is being read.
struct Held {
    claim: Claim,
    damaged: bool,
    unsupported: bool,
}

/// One pass over one journal.
struct Pass<'a> {
    label: &'a str,
    session: Option<&'a String>,
    range: Option<(String, String)>,
    claims: Vec<(Key, Held)>,
    facts: BTreeMap<Key, Vec<Fact>>,
    links: BTreeSet<String>,
    legacy: Vec<String>,
    notes: Vec<(Option<String>, Reason)>,
    named: Named,
}

impl<'a> Pass<'a> {
    fn new(label: &'a str, session: Option<&'a String>) -> Self {
        Self {
            label,
            session,
            range: None,
            claims: Vec::new(),
            facts: BTreeMap::new(),
            links: BTreeSet::new(),
            legacy: Vec::new(),
            notes: Vec::new(),
            named: Named::default(),
        }
    }

    /// The key a record belongs to, and whether it contradicts its own source:
    /// a record names `Some(session)` the journal does not belong to.
    fn key(&self, session: Option<String>, birth: String) -> (Key, bool) {
        let foreign =
            session.is_some() && self.session.is_some() && session.as_ref() != self.session;
        ((session.or_else(|| self.session.cloned()), birth), foreign)
    }

    fn record(&mut self, text: &str, event: &Event) {
        let date = event.ts.to_string();
        if let Some(date) = date.get(..DATE_LEN) {
            match &mut self.range {
                Some((from, to)) => {
                    least(from, date);
                    if date > to.as_str() {
                        date.clone_into(to);
                    }
                }
                empty => *empty = Some((date.to_owned(), date.to_owned())),
            }
        }
        if let Some(slot) = event.target_slot.value() {
            self.named.slots.insert(slot.to_owned());
        }
        if (event.action == SPAWN || birth::is_family(&event.action))
            && let Some(name) = &event.target
        {
            self.named.names.insert(name.clone());
        }
        match birth::decode(event) {
            Some(Ok(record)) => self.family(record),
            Some(Err(damage)) => self.damaged(damage),
            None if event.action == SPAWN => self.legacy.push(text.to_owned()),
            None => {}
        }
    }

    fn family(&mut self, record: Record) {
        match record {
            Record::Birth(certificate) => {
                let stated = claim(&certificate);
                let (key, foreign) = self.key(certificate.session, certificate.birth);
                let claim = if foreign {
                    self.notes.push((key.0.clone(), Reason::SessionMismatch));
                    unknown()
                } else {
                    stated
                };
                self.hold(key, claim, false, false);
            }
            Record::Fact(fact) => {
                let (key, foreign) = self.key(fact.session.clone(), fact.birth.clone());
                if foreign {
                    self.notes.push((key.0, Reason::SessionMismatch));
                } else {
                    let fact = Fact {
                        session: None,
                        ..fact
                    };
                    self.facts.entry(key).or_default().push(fact);
                }
            }
            Record::Unsupported {
                family: Family::Birth,
                birth,
                session,
                ..
            } => {
                let (key, foreign) = self.key(session, birth);
                if foreign {
                    self.notes.push((key.0.clone(), Reason::SessionMismatch));
                }
                self.hold(key, unknown(), false, true);
            }
            Record::Unsupported { session, .. } => {
                let subject = session.or_else(|| self.session.cloned());
                self.notes.push((subject, Reason::Unsupported));
            }
            Record::SpawnLink { birth } => drop(self.links.insert(birth)),
        }
    }

    fn damaged(&mut self, damage: Damage) {
        match damage {
            Damage {
                family: Family::Birth,
                birth: Some(birth),
                session,
            } => {
                let (key, foreign) = self.key(session, birth);
                if foreign {
                    self.notes.push((key.0.clone(), Reason::SessionMismatch));
                }
                self.hold(key, unknown(), true, false);
            }
            Damage { session, .. } => {
                let subject = session.or_else(|| self.session.cloned());
                self.notes.push((subject, Reason::Damaged));
            }
        }
    }

    fn hold(&mut self, key: Key, claim: Claim, damaged: bool, unsupported: bool) {
        self.claims.push((
            key,
            Held {
                claim,
                damaged,
                unsupported,
            },
        ));
    }
}

/// Whether every list is a subsequence of the longest: one journal, trimmed.
fn agree<T: PartialEq>(lists: &[Vec<T>]) -> bool {
    let longest = lists.iter().max_by_key(|list| list.len());
    longest.is_none_or(|longest| lists.iter().all(|list| subsequence(list, longest)))
}

/// Whether `short` is `long` with records dropped, order kept.
fn subsequence<T: PartialEq>(short: &[T], long: &[T]) -> bool {
    let mut rest = long.iter();
    short.iter().all(|item| rest.any(|other| other == item))
}

/// One counted spawn.
struct Spawn {
    session: Option<String>,
    cells: [Cell; 4],
    /// The model's and the effort's unproved word.
    words: [Option<String>; 2],
    legacy: bool,
}

impl Spawn {
    fn of(session: Option<String>, claim: Claim) -> Self {
        let words = [claim[MODEL].1.clone(), claim[EFFORT].1.clone()];
        Self {
            session,
            cells: claim.map(|(cell, _)| cell),
            words,
            legacy: false,
        }
    }

    fn legacy(session: Option<String>) -> Self {
        Self {
            legacy: true,
            ..Self::of(session, unknown())
        }
    }

    /// Whether `index`'s flag state is proved: a value, or no flag at all.
    fn proved(&self, index: usize) -> bool {
        matches!(self.cells[index], Cell::Value(_) | Cell::NoneSet)
    }
}

/// Everything settled, ready to be scoped.
struct Ledger {
    seen: Vec<Seen>,
    spawns: Vec<Spawn>,
    notes: Vec<Note>,
    /// `(session, shortest list, longest list)` of each divergent legacy history.
    excluded: Vec<(String, usize, usize)>,
}

/// Whether an entry for `session` belongs in the scope of `this`.
fn strict(session: Option<&String>, this: Option<&str>) -> bool {
    this.is_none() || session.map(String::as_str) == this
}

/// Like [`strict`], for an entry that may be about any session when it has none.
fn covers(session: Option<&String>, this: Option<&str>) -> bool {
    session.is_none() || strict(session, this)
}

impl Ledger {
    /// One scope: all retained for `None`, one session for `Some`.
    fn scoped(&self, this: Option<&str>) -> Scoped {
        let spawns: Vec<&Spawn> = self
            .spawns
            .iter()
            .filter(|spawn| strict(spawn.session.as_ref(), this))
            .collect();
        let tally = |test: &dyn Fn(&Spawn) -> bool| -> u64 {
            let count = spawns.iter().filter(|spawn| test(spawn)).count();
            u64::try_from(count).unwrap_or(u64::MAX)
        };
        let sources: Vec<&Seen> = self
            .seen
            .iter()
            .filter(|seen| covers(seen.session.as_ref(), this))
            .collect();
        let read = sources.iter().filter(|seen| seen.read).count();
        Scoped {
            total: tally(&|_| true),
            legacy: tally(&|spawn| spawn.legacy),
            model_known: tally(&|spawn| spawn.proved(MODEL)),
            effort_known: tally(&|spawn| spawn.proved(EFFORT)),
            range: self.range(this),
            sources_read: u64::try_from(read).unwrap_or(u64::MAX),
            sources_total: u64::try_from(sources.len()).unwrap_or(u64::MAX),
            harness: table(&spawns, &[0]),
            client: table(&spawns, &[1]),
            model_effort: table(&spawns, &[MODEL, EFFORT]),
            joint: table(&spawns, &[0, 1, MODEL, EFFORT]),
            gaps: self.gaps(&spawns, this),
        }
    }

    /// The dates of the sources this scope owns outright.
    fn range(&self, this: Option<&str>) -> Option<(String, String)> {
        let mut seen = self
            .seen
            .iter()
            .filter(|seen| strict(seen.session.as_ref(), this))
            .filter_map(|seen| seen.range.as_ref());
        let (from, to) = seen.next()?.clone();
        Some(seen.fold((from, to), |(from, to), (first, last)| {
            (from.min(first.clone()), to.max(last.clone()))
        }))
    }

    fn gaps(&self, spawns: &[&Spawn], this: Option<&str>) -> Vec<Gap> {
        let mut gaps: BTreeMap<(Reason, Option<String>), u64> = BTreeMap::new();
        for note in &self.notes {
            if covers(note.session.as_ref(), this) {
                *gaps
                    .entry((note.reason.clone(), note.source.clone()))
                    .or_default() += note.count;
            }
        }
        for spawn in spawns {
            for (model, word) in [(true, &spawn.words[0]), (false, &spawn.words[1])] {
                if let Some(word) = word {
                    let word = word.clone();
                    *gaps
                        .entry((Reason::Unproved { model, word }, None))
                        .or_default() += 1;
                }
            }
        }
        let excluded = self
            .excluded
            .iter()
            .filter(|(session, ..)| strict(Some(session), this));
        let (mut sessions, mut a, mut b) = (0_u64, 0_u64, 0_u64);
        for (_, shortest, longest) in excluded {
            sessions += 1;
            a += u64::try_from(*shortest).unwrap_or(u64::MAX);
            b += u64::try_from(*longest).unwrap_or(u64::MAX);
        }
        if sessions > 0 {
            gaps.insert((Reason::LegacyExcluded { sessions, a, b }, None), sessions);
        }
        gaps.into_iter()
            .map(|((reason, source), count)| Gap {
                source,
                reason,
                count,
            })
            .collect()
    }
}

/// One table over `columns`, count descending, then cells ascending.
fn table(spawns: &[&Spawn], columns: &[usize]) -> Vec<Row> {
    let mut counts: BTreeMap<Vec<&Cell>, u64> = BTreeMap::new();
    for spawn in spawns {
        let cells = columns.iter().map(|column| &spawn.cells[*column]).collect();
        *counts.entry(cells).or_default() += 1;
    }
    let mut rows: Vec<Row> = counts
        .into_iter()
        .map(|(cells, count)| Row {
            cells: cells.into_iter().cloned().collect(),
            count,
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.count));
    rows
}

/// Fold every source.
#[must_use]
pub fn fold(sources: Vec<Source>, this: Option<&str>) -> View {
    let mut fold = Fold::default();
    for source in sources {
        fold.add(source);
    }
    fold.finish(this)
}

/// Both scopes of one read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub all: Scoped,
    pub this: ThisSession,
}

/// The settings session's scope, when it has a session id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThisSession {
    Ready(Box<Scoped>),
    NoUuid,
}

/// One scope's counts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scoped {
    /// Every counted spawn: the sum of each table.
    pub total: u64,
    /// The spawns with no certificate of any kind: a lower bound.
    pub legacy: u64,
    /// Spawns whose model flag is a value or proved absent.
    pub model_known: u64,
    /// The same for effort.
    pub effort_known: u64,
    /// First and last `YYYY-MM-DD` of any parsed record.
    pub range: Option<(String, String)>,
    pub sources_read: u64,
    pub sources_total: u64,
    pub harness: Vec<Row>,
    pub client: Vec<Row>,
    pub model_effort: Vec<Row>,
    pub joint: Vec<Row>,
    pub gaps: Vec<Gap>,
}

impl Scoped {
    /// Whether every table sums to the total and no part exceeds its whole.
    #[must_use]
    pub fn balanced(&self) -> bool {
        [&self.harness, &self.client, &self.model_effort, &self.joint]
            .iter()
            .all(|table| table.iter().map(|row| row.count).sum::<u64>() == self.total)
            && self.legacy <= self.total
            && self.model_known <= self.total
            && self.effort_known <= self.total
            && self.sources_read <= self.sources_total
    }

    /// Whether some evidence was not read or cannot be trusted. A note is not:
    /// an unproved flag and a live seat with no birth lose nothing.
    #[must_use]
    pub fn partial(&self) -> bool {
        self.sources_read < self.sources_total || self.gaps.iter().any(|gap| gap.reason.partial())
    }
}

/// One table row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub count: u64,
}

/// One table cell.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cell {
    /// The value as written: an alias stays one.
    Value(String),
    /// The command carries no such flag, which says nothing of the harness.
    NoneSet,
    /// No `[clients]` label: a route, not an account.
    Direct,
    /// The command proves neither a value nor an absence.
    Unproved,
    /// No trusted record says.
    Unknown,
}

impl Cell {
    /// What a table shows.
    #[must_use]
    pub fn word(&self) -> &str {
        match self {
            Self::Value(value) => value,
            Self::NoneSet => "none set",
            Self::Direct => "direct",
            Self::Unproved => "unproved",
            Self::Unknown => "unknown",
        }
    }
}

/// One coverage line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    /// The source it is about: `None` for an aggregate.
    pub source: Option<String>,
    pub reason: Reason,
    pub count: u64,
}

impl Gap {
    /// The line the tab shows.
    #[must_use]
    pub fn line(&self) -> String {
        let count = self.count;
        let what = match &self.reason {
            Reason::Source(gap) => gap.word().to_owned(),
            Reason::Malformed => format!("{count} malformed"),
            Reason::Damaged => format!("{count} damaged"),
            Reason::Unsupported => format!("{count} unsupported"),
            Reason::CertificateLost => format!("{count} certificate lost"),
            Reason::Conflict => format!("{count} conflict"),
            Reason::SessionMismatch => format!("{count} session mismatch"),
            Reason::NoSessionId => "no session id".to_owned(),
            Reason::LiveNoBirth => format!("{count} live with no birth"),
            Reason::LegacyExcluded { sessions, a, b } => {
                format!("legacy excluded: {sessions} sessions diverge, {a} to {b} spawns")
            }
            Reason::Unproved { model, word } => {
                let flag = if *model { "model" } else { "effort" };
                format!("{count} {flag} unproved: {word}")
            }
        };
        self.source
            .as_ref()
            .map_or_else(|| what.clone(), |source| format!("{source}: {what}"))
    }
}

impl SourceGap {
    const fn word(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Unreadable => "unreadable",
            Self::Oversized => "oversized",
            Self::Budget => "over the read budget",
            Self::Invalid => "invalid archive",
            Self::Claimed => "claimed archive",
        }
    }
}

/// Why a coverage line exists.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    Source(SourceGap),
    Malformed,
    Damaged,
    Unsupported,
    CertificateLost,
    Conflict,
    SessionMismatch,
    NoSessionId,
    LiveNoBirth,
    LegacyExcluded { sessions: u64, a: u64, b: u64 },
    Unproved { model: bool, word: String },
}

impl Reason {
    /// Whether this costs evidence, rather than qualifying it.
    #[must_use]
    pub const fn partial(&self) -> bool {
        !matches!(self, Self::Unproved { .. } | Self::LiveNoBirth)
    }
}

#[cfg(test)]
#[path = "stats_spec.rs"]
mod spec;
