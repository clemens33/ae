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
    /// What the fold would keep of this source does not fit its memory budget.
    Retained,
}

/// One journal as the reader found it.
#[derive(Debug)]
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
        Self {
            label: name.to_owned(),
            session: session_id(text),
            journal,
            seats: roster(text),
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

/// One journal being read line by line: the same fold as [`Source`]'s whole
/// bytes, a line at a time, so the journal is never held. It owns what the open
/// source keeps and counts it against the room the fold had left; nothing
/// reaches the fold until [`Fold::add_reading`].
pub struct Reading {
    label: String,
    session: Option<String>,
    seats: Vec<(String, String)>,
    pass: Pass,
    malformed: u64,
    line_limit: u64,
    /// The roster's share of `charge`: held while open, never by the fold.
    roster: u64,
    /// Charged bytes so far: the source, its roster and what its lines keep.
    charge: u64,
    room: u64,
}

impl Reading {
    /// A live session: `name`, its raw meta and the room the fold has left. A
    /// meta that is not text proves no identity; a source whose roster does not
    /// fit the room never opens. Both come back as the source that gave nothing.
    ///
    /// # Errors
    ///
    /// The failed [`Source`] to add in its place.
    pub fn live(name: &str, meta: &[u8], room: u64) -> Result<Self, Source> {
        let Ok(text) = std::str::from_utf8(meta) else {
            return Err(Source::failed(name, None, SourceGap::Unreadable));
        };
        Self::open(name.to_owned(), session_id(text), roster(text), room)
    }

    /// A kept archive, named by its session id.
    ///
    /// # Errors
    ///
    /// The failed [`Source`] to add in its place.
    pub fn archive(id: &str, room: u64) -> Result<Self, Source> {
        let session = is_canonical_uuid(id).then(|| id.to_owned());
        Self::open(id.to_owned(), session, Vec::new(), room)
    }

    fn open(
        label: String,
        session: Option<String>,
        seats: Vec<(String, String)>,
        room: u64,
    ) -> Result<Self, Source> {
        let roster = seats.iter().fold(0_u64, |sum, (slot, name)| {
            let text = u64::try_from(slot.len() + name.len()).unwrap_or(u64::MAX);
            sum.saturating_add(text).saturating_add(SEAT_COST)
        });
        let charge = SOURCE_COST.saturating_add(roster);
        if charge > room {
            return Err(Source::failed(
                &label,
                session.as_deref(),
                SourceGap::Retained,
            ));
        }
        Ok(Self {
            pass: Pass::new(label.clone(), session.clone()),
            label,
            session,
            seats,
            malformed: 0,
            line_limit: 0,
            roster,
            charge,
            room,
        })
    }

    /// One journal line, LF excluded. A line that keeps state raises the charge;
    /// a line whose price does not fit the room left is refused before any of it
    /// is kept, and the source is refused whole.
    ///
    /// # Errors
    ///
    /// [`SourceGap::Retained`] when the line would pass the room; the charge and
    /// everything kept stay as they were.
    pub fn line(&mut self, line: &[u8]) -> Result<(), SourceGap> {
        if line.trim_ascii().is_empty() {
            return Ok(());
        }
        let parsed = std::str::from_utf8(line)
            .ok()
            .and_then(|text| Some((text, Event::parse_line(text).ok()?)));
        match parsed {
            Some((text, event)) => {
                let left = self.room.saturating_sub(self.charge);
                let Some(paid) = self.pass.record(text, &event, left) else {
                    return Err(SourceGap::Retained);
                };
                self.charge = self.charge.saturating_add(paid);
            }
            None => self.malformed += 1,
        }
        Ok(())
    }

    /// A line over the line limit, discarded unread.
    pub const fn over_long(&mut self) {
        self.line_limit += 1;
    }

    /// Charged bytes so far, the open roster included.
    #[must_use]
    pub const fn charge(&self) -> u64 {
        self.charge
    }

    /// Give the source up with `gap`: nothing it read is kept.
    #[must_use]
    pub fn fail(self, gap: SourceGap) -> Source {
        Source::failed(&self.label, self.session.as_deref(), gap)
    }
}

/// `(slot, name)` of each spawned seat the live roster lists.
fn roster(meta: &str) -> Vec<(String, String)> {
    Meta::parse(meta)
        .roster()
        .iter()
        .filter(|seat| seat.slot.starts_with(SPAWNED))
        .map(|seat| (seat.slot.clone(), seat.name.clone()))
        .collect()
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

/// What a source costs the fold to hold at all: its row, range and labels.
const SOURCE_COST: u64 = 4096;
/// What one seat of a live roster costs while its source is open: two string
/// headers and two allocations beyond the text.
const SEAT_COST: u64 = 112;
/// What a birth certificate costs beyond its text, in charged bytes: the claim
/// set's own tree node, its place in the fold's maps, the settled spawn, the
/// notes and table rows it can produce. [`CERT_MODEL`] recomputes it.
const CERT_COST: u64 = 6144;
/// A later fact's cost beyond its text.
const FACT_COST: u64 = 3072;
/// A modern `spawn`'s link to its certificate, beyond its text.
const LINK_COST: u64 = 1536;
/// A legacy `spawn` beyond its text, which the fold keeps whole.
const LEGACY_COST: u64 = 512;
/// A per-record note beyond its text.
const NOTE_COST: u64 = 1024;

/// What one record keeps, in charged bytes, priced before any of it is kept: a
/// `base` for each kind of state it makes, and the number of `copies` of its
/// text that state holds. The text is variable and unbounded below the line
/// limit, so each owner of a copy is charged, not the text once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Price {
    base: u64,
    copies: u64,
}

impl Price {
    /// A record that keeps nothing.
    const NONE: Self = Self { base: 0, copies: 0 };

    const fn plus(self, other: Self) -> Self {
        Self {
            base: self.base + other.base,
            copies: self.copies + other.copies,
        }
    }

    /// What `self` has gained since `before`.
    const fn since(self, before: Self) -> Self {
        Self {
            base: self.base - before.base,
            copies: self.copies - before.copies,
        }
    }

    /// The charge for a record of `text` bytes.
    fn of(self, text: usize) -> u64 {
        let text = u64::try_from(text).unwrap_or(u64::MAX);
        self.base.saturating_add(self.copies.saturating_mul(text))
    }
}

/// A certificate holds its text in the claim set's value, the settled spawn,
/// both scopes' table rows, the birth id and the born name: eight copies (see
/// `.local/statsread/retention-audit.md` for the owners).
const CERT: Price = Price {
    base: CERT_COST,
    copies: 8,
};
/// A fact holds its strings in the pass's list and again in the fold's.
const FACT: Price = Price {
    base: FACT_COST,
    copies: 2,
};
/// A link holds the birth id and the born name.
const LINK: Price = Price {
    base: LINK_COST,
    copies: 2,
};
/// A legacy spawn holds its whole line and the born name.
const LEGACY: Price = Price {
    base: LEGACY_COST,
    copies: 2,
};
/// A note holds its subject and the born name.
const NOTE: Price = Price {
    base: NOTE_COST,
    copies: 2,
};

/// A B-tree leaf node holding eleven slots of `slot` bytes.
const fn leaf(slot: usize) -> usize {
    11 * slot + 16
}

/// One entry's share of such a node: every node but the root holds five or more.
const fn share(slot: usize) -> usize {
    leaf(slot).div_ceil(5)
}

/// A certificate's cost, worst case, from the sizes of what it keeps: a claim
/// set's own node (a tree per certificate), its share of the certificates map
/// and of the born set, the pass's copy and the settled spawn, four string
/// allocations, the three notes `settle` may add and two scopes' table rows.
const CERT_MODEL: usize = leaf(size_of::<Claim>())
    + share(size_of::<Key>() + size_of::<Certificates>())
    + share(size_of::<(Option<String>, String)>())
    + size_of::<(Key, Held)>()
    + size_of::<Spawn>()
    + 4 * 32
    + 3 * (size_of::<Note>() + 2 * (36 + 32))
    + 2 * (size_of::<Row>() + 4 * (32 + 32));

/// A fact's cost, worst case: its record twice over beside its key's share of
/// the pass's and the fold's maps and their list headers.
const FACT_MODEL: usize = size_of::<Fact>()
    + 2 * share(size_of::<Key>() + size_of::<Vec<Fact>>())
    + size_of::<Vec<Vec<Fact>>>()
    + 4 * 32;

/// A link's cost, worst case: its share of the pass's set and the fold's map.
const LINK_MODEL: usize =
    share(size_of::<String>()) + share(size_of::<Key>() + size_of::<String>()) + 4 * 32;

/// A legacy spawn's cost, worst case beside its text: the list slot, the
/// settled spawn and two allocations.
const LEGACY_MODEL: usize = size_of::<String>() + size_of::<Spawn>() + 2 * 32;

/// A note's cost, worst case: the pass's row, the fold's note, the born entry
/// and the allocations behind them.
const NOTE_MODEL: usize = size_of::<(Option<String>, Reason)>()
    + size_of::<Note>()
    + share(size_of::<(Option<String>, String)>())
    + 6 * 32;

/// The longest a source's label can be: a directory entry's name. A note keeps
/// its source's label, so each note a record makes carries one beyond the model.
const LABEL: u64 = 255;

const _: () = {
    assert!(
        CERT_MODEL as u64 + 4 * LABEL <= CERT_COST,
        "a certificate outgrew its charge"
    );
    assert!(FACT_MODEL as u64 <= FACT_COST, "a fact outgrew its charge");
    assert!(
        LINK_MODEL as u64 + LABEL <= LINK_COST,
        "a link outgrew its charge"
    );
    assert!(
        LEGACY_MODEL as u64 <= LEGACY_COST,
        "a legacy spawn outgrew its charge"
    );
    assert!(
        NOTE_MODEL as u64 + LABEL <= NOTE_COST,
        "a note outgrew its charge"
    );
};

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
    /// The most the fold may keep, in charged bytes.
    limit: Limit,
    /// What the sources added so far keep, in charged bytes.
    held: u64,
}

/// The retained-state ceiling: none unless one is asked for.
#[derive(Clone, Copy)]
struct Limit(u64);

impl Default for Limit {
    fn default() -> Self {
        Self(u64::MAX)
    }
}

impl Fold {
    /// A fold that refuses a source whose kept state would push the total past
    /// `limit` charged bytes. [`Fold::default`] has no such ceiling.
    #[must_use]
    pub fn with_retained(limit: u64) -> Self {
        Self {
            limit: Limit(limit),
            ..Self::default()
        }
    }

    /// Charged bytes still free for the next source.
    #[must_use]
    pub const fn room(&self) -> u64 {
        self.limit.0.saturating_sub(self.held)
    }

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
                let mut reading = match Reading::open(label, session, seats, self.room()) {
                    Ok(reading) => reading,
                    Err(refused) => return self.add(refused),
                };
                for line in bytes.split(|byte| *byte == b'\n') {
                    if let Err(gap) = reading.line(line) {
                        return self.add(reading.fail(gap));
                    }
                }
                self.add_reading(reading);
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

    /// Take a journal read through to its end: its kept state, its counts, and
    /// its place among the sources. Its roster charge ends here.
    pub fn add_reading(&mut self, reading: Reading) {
        let Reading {
            label,
            session,
            seats,
            pass,
            malformed,
            line_limit,
            roster,
            charge,
            ..
        } = reading;
        if malformed > 0 {
            self.note(session.clone(), Some(&label), Reason::Malformed, malformed);
        }
        if line_limit > 0 {
            self.note(session.clone(), Some(&label), Reason::LineLimit, line_limit);
        }
        let range = self.absorb(pass, &seats);
        if session.is_none() {
            self.note(None, Some(&label), Reason::NoSessionId, 1);
        }
        self.seen.push(Seen {
            session,
            read: true,
            range,
        });
        self.held = self.held.saturating_add(charge.saturating_sub(roster));
    }

    fn note(&mut self, session: Option<String>, source: Option<&str>, reason: Reason, count: u64) {
        self.notes.push(Note {
            session,
            source: source.map(ToOwned::to_owned),
            reason,
            count,
        });
    }

    /// One journal's reading into the fold.
    fn absorb(&mut self, pass: Pass, seats: &[(String, String)]) -> Option<(String, String)> {
        let Pass {
            label,
            session,
            range,
            claims,
            facts,
            links,
            legacy,
            notes,
            born,
            ..
        } = pass;
        let label = label.as_str();
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
        let silent = seats.iter().filter(|seat| !proved(&born, seat)).count();
        if silent > 0 {
            let count = u64::try_from(silent).unwrap_or(u64::MAX);
            self.note(session, Some(label), Reason::LiveNoBirth, count);
        }
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
        Ledger {
            seen: self.seen,
            spawns,
            notes: self.notes,
            excluded,
        }
    }
}

/// Whether one journal's own births and spawns account for a live `seat`: by
/// name, and by slot when the record states one. A slot alone is no proof, for a
/// retired seat's slot is reused.
fn proved(born: &BTreeSet<(Option<String>, String)>, (slot, name): &(String, String)) -> bool {
    born.iter()
        .any(|(stated, named)| named == name && stated.as_ref().is_none_or(|stated| stated == slot))
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
struct Pass {
    label: String,
    session: Option<String>,
    range: Option<(String, String)>,
    claims: Vec<(Key, Held)>,
    facts: BTreeMap<Key, Vec<Fact>>,
    links: BTreeSet<String>,
    legacy: Vec<String>,
    notes: Vec<(Option<String>, Reason)>,
    /// `(slot, name)` of each certificate or `spawn` here, damaged ones too.
    born: BTreeSet<(Option<String>, String)>,
    /// What the records so far keep, by kind; every kind of state is added
    /// through [`Pass::keep`] so the price a record was admitted at is checked
    /// against what it made.
    spent: Price,
}

impl Pass {
    fn new(label: String, session: Option<String>) -> Self {
        Self {
            label,
            session,
            range: None,
            claims: Vec::new(),
            facts: BTreeMap::new(),
            links: BTreeSet::new(),
            legacy: Vec::new(),
            notes: Vec::new(),
            born: BTreeSet::new(),
            spent: Price::NONE,
        }
    }

    /// Whether a record naming `session` contradicts its own source: it names
    /// `Some(session)` the journal does not belong to.
    fn foreign(&self, session: Option<&String>) -> bool {
        session.is_some() && self.session.is_some() && session != self.session.as_ref()
    }

    /// The key a record belongs to, and whether it contradicts its own source.
    fn key(&self, session: Option<String>, birth: String) -> (Key, bool) {
        let foreign = self.foreign(session.as_ref());
        ((session.or_else(|| self.session.clone()), birth), foreign)
    }

    /// Admit one parsed record if its price fits `left`, and fold it in. The
    /// price is worked out from the decoded record before anything is copied, so
    /// a record that does not fit is refused with the pass untouched (`None`);
    /// one that fits comes back as the charged bytes it keeps, nothing for a
    /// record that keeps none (an ordinary event only moves the date range).
    fn record(&mut self, text: &str, event: &Event, left: u64) -> Option<u64> {
        let decoded = birth::decode(event);
        let price = self.price(event, decoded.as_ref());
        let paid = price.of(text.len());
        if paid > left {
            return None;
        }
        let before = self.spent;
        self.fold_in(text, event, decoded);
        debug_assert_eq!(
            self.spent.since(before),
            price,
            "a record kept other state than it was priced for"
        );
        Some(paid)
    }

    /// What `decoded` will keep: the same cases [`Pass::family`] and
    /// [`Pass::damaged`] act on, each kind it will make.
    fn price(&self, event: &Event, decoded: Option<&Result<Record, Damage>>) -> Price {
        let mismatch = |session: &Option<String>| {
            if self.foreign(session.as_ref()) {
                NOTE
            } else {
                Price::NONE
            }
        };
        match decoded {
            Some(Ok(Record::Birth(certificate))) => CERT.plus(mismatch(&certificate.session)),
            Some(Ok(Record::Fact(fact))) => {
                if self.foreign(fact.session.as_ref()) {
                    NOTE
                } else {
                    FACT
                }
            }
            Some(Ok(Record::SpawnLink { .. })) => LINK,
            Some(
                Ok(Record::Unsupported {
                    family: Family::Birth,
                    session,
                    ..
                })
                | Err(Damage {
                    family: Family::Birth,
                    birth: Some(_),
                    session,
                }),
            ) => CERT.plus(mismatch(session)),
            Some(Ok(Record::Unsupported { .. }) | Err(Damage { .. })) => NOTE,
            None if event.action == SPAWN => LEGACY,
            None => Price::NONE,
        }
    }

    /// Note that the state of one kind has been made.
    fn keep(&mut self, kind: Price) {
        self.spent = self.spent.plus(kind);
    }

    fn fold_in(&mut self, text: &str, event: &Event, decoded: Option<Result<Record, Damage>>) {
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
        let slot = event.target_slot.value().map(ToOwned::to_owned);
        if (event.action == SPAWN || event.action == birth::BIRTH)
            && let Some(name) = &event.target
            && slot.is_some() == event.target_slot.is_present()
        {
            self.born.insert((slot, name.clone()));
        }
        match decoded {
            Some(Ok(record)) => self.family(record),
            Some(Err(damage)) => self.damaged(damage),
            None if event.action == SPAWN => {
                self.keep(LEGACY);
                self.legacy.push(text.to_owned());
            }
            None => {}
        }
    }

    fn family(&mut self, record: Record) {
        match record {
            Record::Birth(certificate) => {
                let stated = claim(&certificate);
                let (key, foreign) = self.key(certificate.session, certificate.birth);
                let claim = if foreign {
                    self.note_row(key.0.clone(), Reason::SessionMismatch);
                    unknown()
                } else {
                    stated
                };
                self.hold(key, claim, false, false);
            }
            Record::Fact(fact) => {
                let (key, foreign) = self.key(fact.session.clone(), fact.birth.clone());
                if foreign {
                    self.note_row(key.0, Reason::SessionMismatch);
                } else {
                    let fact = Fact {
                        session: None,
                        ..fact
                    };
                    self.keep(FACT);
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
                    self.note_row(key.0.clone(), Reason::SessionMismatch);
                }
                self.hold(key, unknown(), false, true);
            }
            Record::Unsupported { session, .. } => {
                let subject = session.or_else(|| self.session.clone());
                self.note_row(subject, Reason::Unsupported);
            }
            Record::SpawnLink { birth } => {
                self.keep(LINK);
                self.links.insert(birth);
            }
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
                    self.note_row(key.0.clone(), Reason::SessionMismatch);
                }
                self.hold(key, unknown(), true, false);
            }
            Damage { session, .. } => {
                let subject = session.or_else(|| self.session.clone());
                self.note_row(subject, Reason::Damaged);
            }
        }
    }

    fn note_row(&mut self, subject: Option<String>, reason: Reason) {
        self.keep(NOTE);
        self.notes.push((subject, reason));
    }

    fn hold(&mut self, key: Key, claim: Claim, damaged: bool, unsupported: bool) {
        self.keep(CERT);
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
            Reason::LineLimit => format!("{count} over the line limit"),
        };
        self.source
            .as_ref()
            .map_or_else(|| what.clone(), |source| format!("{source}: {what}"))
    }
}

impl SourceGap {
    /// What a coverage line says.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Unreadable => "unreadable",
            Self::Oversized => "oversized",
            Self::Budget => "over the read budget",
            Self::Invalid => "invalid archive",
            Self::Claimed => "claimed archive",
            Self::Retained => "over the memory budget",
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
    LegacyExcluded {
        sessions: u64,
        a: u64,
        b: u64,
    },
    Unproved {
        model: bool,
        word: String,
    },
    /// Lines over the line limit, discarded unread.
    LineLimit,
}

impl Reason {
    /// Whether this costs evidence, rather than qualifying it.
    #[must_use]
    pub const fn partial(&self) -> bool {
        !matches!(self, Self::Unproved { .. } | Self::LiveNoBirth)
    }
}

/// A live meta's session id and its spawned seats' `(slot, name)`.
pub(crate) type Proved = (Option<String>, Vec<(String, String)>);

/// What a live meta proves to the fold: its session id and its spawned seats.
/// `None` when the meta is not text, which proves no identity.
pub(crate) fn identity(meta: &[u8]) -> Option<Proved> {
    let text = std::str::from_utf8(meta).ok()?;
    Some((session_id(text), roster(text)))
}

impl Reading {
    /// A source read before, opened again from what a store kept of it: its
    /// label, the session it proved and its live roster.
    ///
    /// # Errors
    ///
    /// The failed [`Source`] to add in its place, as [`Reading::live`].
    pub(crate) fn resume(
        label: &str,
        session: Option<&str>,
        seats: Vec<(String, String)>,
        room: u64,
    ) -> Result<Self, Source> {
        Self::open(
            label.to_owned(),
            session.map(ToOwned::to_owned),
            seats,
            room,
        )
    }

    /// Lines a store read before and kept only the counts and dates of: the
    /// same effect on this source as reading those lines again.
    pub(crate) fn carry(&mut self, malformed: u64, line_limit: u64, range: Option<(&str, &str)>) {
        self.malformed = self.malformed.saturating_add(malformed);
        self.line_limit = self.line_limit.saturating_add(line_limit);
        if let Some((from, to)) = range {
            self.pass.widen(from);
            self.pass.widen(to);
        }
    }
}

impl Pass {
    /// Stretch the range to cover `date`, as a record of that date does.
    fn widen(&mut self, date: &str) {
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
}

impl Fold {
    /// The part of everything read whose session is exactly `group`: what
    /// [`Fold::finish`] would count for it, settled by the same judgment. The
    /// parts of disjoint groups add up to the all-sessions scope, and a
    /// session's part plus the session-less part's [`Tally::covering`] is that
    /// session's scope.
    pub(crate) fn finish_exact(self, group: Option<&str>) -> Tally {
        self.settle().exact(group)
    }
}

impl Ledger {
    /// Every entry whose session is exactly `group`, as counts.
    fn exact(&self, group: Option<&str>) -> Tally {
        let mine = |session: Option<&String>| session.map(String::as_str) == group;
        let mut tally = Tally::default();
        for spawn in self
            .spawns
            .iter()
            .filter(|spawn| mine(spawn.session.as_ref()))
        {
            bump(tally.joint.entry(spawn.cells.clone()).or_default(), 1);
            if spawn.legacy {
                bump(&mut tally.legacy, 1);
            }
            for (model, word) in [(true, &spawn.words[0]), (false, &spawn.words[1])] {
                if let Some(word) = word {
                    bump(tally.unproved.entry((model, word.clone())).or_default(), 1);
                }
            }
        }
        for note in self.notes.iter().filter(|note| mine(note.session.as_ref())) {
            let key = (note.reason.clone(), note.source.clone());
            bump(tally.notes.entry(key).or_default(), note.count);
        }
        for seen in self.seen.iter().filter(|seen| mine(seen.session.as_ref())) {
            bump(&mut tally.sources, 1);
            if seen.read {
                bump(&mut tally.read, 1);
            }
            if let Some(range) = &seen.range {
                tally.widen(range);
            }
        }
        for (session, shortest, longest) in &self.excluded {
            if mine(Some(session)) {
                let [sessions, a, b] = &mut tally.excluded;
                bump(sessions, 1);
                bump(a, u64::try_from(*shortest).unwrap_or(u64::MAX));
                bump(b, u64::try_from(*longest).unwrap_or(u64::MAX));
            }
        }
        tally
    }
}

/// Add `count` to `held`, stopping at the top rather than wrapping.
fn bump(held: &mut u64, count: u64) {
    *held = held.saturating_add(count);
}

/// One group's settled entries as counts: what a scope is made of, kept so
/// that groups add up without being read again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    /// Spawns by their four cells.
    joint: BTreeMap<[Cell; 4], u64>,
    legacy: u64,
    /// Spawns by an unproved flag's `(model?, word)`.
    unproved: BTreeMap<(bool, String), u64>,
    /// Coverage notes by `(reason, source)`.
    notes: BTreeMap<(Reason, Option<String>), u64>,
    /// Divergent legacy histories: sessions, shortest and longest lists summed.
    excluded: [u64; 3],
    read: u64,
    sources: u64,
    range: Option<(String, String)>,
}

impl Tally {
    fn widen(&mut self, (first, last): &(String, String)) {
        match &mut self.range {
            Some((from, to)) => {
                least(from, first);
                if last.as_str() > to.as_str() {
                    last.clone_into(to);
                }
            }
            empty => *empty = Some((first.clone(), last.clone())),
        }
    }

    /// Counts of spawns by their four cells and nothing else; `None` when
    /// they sum past `u64`.
    pub(crate) fn of_joint(joint: &BTreeMap<[Cell; 4], u64>) -> Option<Self> {
        joint
            .values()
            .try_fold(0_u64, |sum, count| sum.checked_add(*count))?;
        Some(Self {
            joint: joint.clone(),
            ..Self::default()
        })
    }

    /// Add another group's counts to these.
    pub(crate) fn add(&mut self, other: &Self) {
        for (cells, count) in &other.joint {
            bump(self.joint.entry(cells.clone()).or_default(), *count);
        }
        bump(&mut self.legacy, other.legacy);
        for (word, count) in &other.unproved {
            bump(self.unproved.entry(word.clone()).or_default(), *count);
        }
        for (key, count) in &other.notes {
            bump(self.notes.entry(key.clone()).or_default(), *count);
        }
        for (held, count) in self.excluded.iter_mut().zip(other.excluded) {
            bump(held, count);
        }
        bump(&mut self.read, other.read);
        bump(&mut self.sources, other.sources);
        if let Some(range) = &other.range {
            self.widen(range);
        }
    }

    /// [`Tally::add`] that refuses to saturate: `None`, and these counts
    /// untouched, when any sum would pass `u64`. A total kept by adding and
    /// taking away groups stays exact only while nothing saturates.
    pub(crate) fn checked_add(&mut self, other: &Self) -> Option<()> {
        let mut sum = self.clone();
        for (cells, count) in &other.joint {
            let held = sum.joint.entry(cells.clone()).or_default();
            *held = held.checked_add(*count)?;
        }
        sum.legacy = sum.legacy.checked_add(other.legacy)?;
        for (word, count) in &other.unproved {
            let held = sum.unproved.entry(word.clone()).or_default();
            *held = held.checked_add(*count)?;
        }
        for (key, count) in &other.notes {
            let held = sum.notes.entry(key.clone()).or_default();
            *held = held.checked_add(*count)?;
        }
        for (held, count) in sum.excluded.iter_mut().zip(other.excluded) {
            *held = held.checked_add(count)?;
        }
        sum.read = sum.read.checked_add(other.read)?;
        sum.sources = sum.sources.checked_add(other.sources)?;
        if let Some(range) = &other.range {
            sum.widen(range);
        }
        *self = sum;
        Some(())
    }

    /// Take a group's counts away again: `None`, and these counts untouched,
    /// when `other` holds more of anything than these do. The date range is
    /// left as it is: a range cannot be taken apart, so whoever keeps a total
    /// this way keeps its endpoints apart and sets the range itself.
    pub(crate) fn sub(&mut self, other: &Self) -> Option<()> {
        fn take<K: Ord + Clone>(
            held: &mut BTreeMap<K, u64>,
            other: &BTreeMap<K, u64>,
        ) -> Option<()> {
            for (key, count) in other {
                let left = held.get(key).copied().unwrap_or(0).checked_sub(*count)?;
                if left == 0 {
                    held.remove(key);
                } else {
                    held.insert(key.clone(), left);
                }
            }
            Some(())
        }
        let mut left = self.clone();
        take(&mut left.joint, &other.joint)?;
        left.legacy = left.legacy.checked_sub(other.legacy)?;
        take(&mut left.unproved, &other.unproved)?;
        take(&mut left.notes, &other.notes)?;
        for (held, count) in left.excluded.iter_mut().zip(other.excluded) {
            *held = held.checked_sub(count)?;
        }
        left.read = left.read.checked_sub(other.read)?;
        left.sources = left.sources.checked_sub(other.sources)?;
        *self = left;
        Some(())
    }

    /// The counts a scope was laid out from: `None` unless laying these out
    /// again gives exactly `scoped`, so a scope no tally makes is refused.
    pub(crate) fn of_scoped(scoped: &Scoped) -> Option<Self> {
        let mut tally = Self {
            legacy: scoped.legacy,
            read: scoped.sources_read,
            sources: scoped.sources_total,
            range: scoped.range.clone(),
            ..Self::default()
        };
        for row in &scoped.joint {
            let cells: [Cell; 4] = row.cells.clone().try_into().ok()?;
            if tally.joint.insert(cells, row.count).is_some() {
                return None;
            }
        }
        for gap in &scoped.gaps {
            match &gap.reason {
                Reason::Unproved { model, word } if gap.source.is_none() => {
                    let held = tally.unproved.entry((*model, word.clone())).or_default();
                    *held = held.checked_add(gap.count)?;
                }
                Reason::LegacyExcluded { sessions, a, b } if gap.source.is_none() => {
                    tally.excluded = [*sessions, *a, *b];
                }
                reason => {
                    let held = tally
                        .notes
                        .entry((reason.clone(), gap.source.clone()))
                        .or_default();
                    *held = held.checked_add(gap.count)?;
                }
            }
        }
        (tally.scoped() == *scoped).then_some(tally)
    }

    /// The dates these counts span.
    pub(crate) const fn range(&self) -> Option<&(String, String)> {
        self.range.as_ref()
    }

    /// Set the span outright: a total kept by taking groups away cannot
    /// narrow its own range, so its keeper does.
    pub(crate) fn set_range(&mut self, range: Option<(String, String)>) {
        self.range = range;
    }

    /// What of a session-less group reaches every session's scope: its notes
    /// and its sources, never its spawns or its dates (see [`covers`]).
    pub(crate) fn covering(&self) -> Self {
        Self {
            notes: self.notes.clone(),
            read: self.read,
            sources: self.sources,
            ..Self::default()
        }
    }

    /// The scope these counts make, laid out as [`Ledger::scoped`] lays it.
    pub(crate) fn scoped(&self) -> Scoped {
        let sum = |test: &dyn Fn(&[Cell; 4]) -> bool| {
            self.joint
                .iter()
                .filter(|(cells, _)| test(cells))
                .fold(0_u64, |sum, (_, count)| sum.saturating_add(*count))
        };
        let known = |index: usize| {
            move |cells: &[Cell; 4]| matches!(cells[index], Cell::Value(_) | Cell::NoneSet)
        };
        Scoped {
            total: sum(&|_| true),
            legacy: self.legacy,
            model_known: sum(&known(MODEL)),
            effort_known: sum(&known(EFFORT)),
            range: self.range.clone(),
            sources_read: self.read,
            sources_total: self.sources,
            harness: self.table(&[0]),
            client: self.table(&[1]),
            model_effort: self.table(&[MODEL, EFFORT]),
            joint: self.table(&[0, 1, MODEL, EFFORT]),
            gaps: self.gaps(),
        }
    }

    /// One table over `columns`, ordered as [`table`] orders it.
    fn table(&self, columns: &[usize]) -> Vec<Row> {
        let mut counts: BTreeMap<Vec<&Cell>, u64> = BTreeMap::new();
        for (cells, count) in &self.joint {
            let key = columns.iter().map(|column| &cells[*column]).collect();
            bump(counts.entry(key).or_default(), *count);
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

    /// The coverage lines, as [`Ledger::gaps`] makes them.
    fn gaps(&self) -> Vec<Gap> {
        let mut gaps = self.notes.clone();
        for ((model, word), count) in &self.unproved {
            let reason = Reason::Unproved {
                model: *model,
                word: word.clone(),
            };
            bump(gaps.entry((reason, None)).or_default(), *count);
        }
        let [sessions, a, b] = self.excluded;
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

#[cfg(test)]
#[path = "stats_spec.rs"]
mod spec;
