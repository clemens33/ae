//! #288 persistence (P1): one conversation's counters and resume state,
//! applied incrementally (plan r4 §1).
//!
//! [`Resume`] keeps a conversation's canonical evidence: per file (a stream
//! role and the digest of its store root) each distinct adapter [`Event`]
//! once, its malformed and over-cap line counts, and the discovery gaps of
//! its last round. Events hold only identities and digests, never a prompt,
//! an argument, a result, a path or a native id. [`Summary`] is derived from
//! that evidence by the fold's own per-conversation count after every change,
//! so a late result, context, proof, child replay or copy collision moves the
//! counted cells exactly as the fold rules decide, and a replay of evidence
//! already held changes nothing: `apply` is total and order-free.
//!
//! [`combine`] draws a [`View`] from published summaries: the identity cap
//! over every published name, the client from the CURRENT memberships, then
//! the fold's own tables. A Read only a proven directory attributes is kept
//! by directory with every row the directory's proofs name, so its row is
//! chosen after the cap maps their names, as the fold chooses it. Equal to
//! the fold over the same events, the identity cap included (fuzz
//! `tool_stats_incr`).

use super::fold::{self, Lines, Names, Obs};
use super::{
    Adapter, Ask, Caps, Cell, Cols, Conversation, Dir, Event, GapKind, Item, Key, Kind, Membership,
    Page, Scope, SkillCols, State, Stream, StreamRole, View, client,
};
use std::collections::{BTreeMap, BTreeSet};

/// One persisted file: its stream role and the digest of its store root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamKey {
    pub role: StreamRole,
    pub copy: [u8; 16],
}

/// Whether a counted file still exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Present,
    Gone,
}

/// A counted row: page, raw-name item, harness, model, effort.
pub type RowKey = (Page, Item, Adapter, Cell, Cell);

/// Why a conversation is refused whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    Observations,
    Turns,
    Size,
}

/// An update whose counts a summary or a combine could not add up in `u64`:
/// it was not taken (see [`Conv::note`], [`Published::admit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overflow;

/// The total every gap line of one conversation's summary is a part of:
/// each stream's malformed and over-cap lines, one per mark, one for a size
/// refusal. `None` past `u64`: such held counts are damage, never counted.
pub(super) fn line_total(lines: &BTreeMap<StreamKey, (u64, u64)>, marks: usize) -> Option<u64> {
    let marks = u64::try_from(marks).ok()?;
    lines
        .values()
        .flat_map(|(malformed, over_cap)| [*malformed, *over_cap])
        .chain([marks, 1])
        .try_fold(0_u64, u64::checked_add)
}

/// A row's counts in their persisted order.
pub(super) fn numbers(cols: &Cols) -> Vec<u64> {
    match cols {
        Cols::Skill(c) => vec![
            c.attempts,
            c.loaded,
            c.not_loaded,
            c.unknown,
            c.unresolved,
            c.reads,
        ],
        Cols::Mcp(c) => vec![c.attempts, c.returned, c.success, c.error, c.unresolved],
        Cols::Tool(c) => vec![c.attempts, c.returned, c.error, c.unresolved],
    }
}

/// Every count of one published conversation a combine may add up, each
/// name counted once for the identity cap's dropped names.
fn conv_counts(conv: &PublishedConv) -> impl Iterator<Item = u64> + '_ {
    let summary = &conv.summary;
    let names = summary.names.values().map(BTreeSet::len).sum::<usize>();
    [summary.orphans, u64::try_from(names).unwrap_or(u64::MAX)]
        .into_iter()
        .chain(summary.gaps.values().copied())
        .chain(summary.inferred.values().copied())
        .chain(summary.rows.values().flat_map(numbers))
}

/// One conversation's counters: no client, no path, no native id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub adapter: Adapter,
    pub id: String,
    pub rows: BTreeMap<RowKey, Cols>,
    pub orphans: u64,
    pub range: Option<(i64, i64)>,
    pub gaps: BTreeMap<GapKind, u64>,
    pub observations: usize,
    pub turns: usize,
    pub read: bool,
    pub files: BTreeMap<StreamKey, Source>,
    pub refused: Option<Refused>,
    /// Every identity the evidence names, per kind: the combine takes the
    /// identity cap over all published conversations' names.
    pub names: BTreeMap<Kind, BTreeSet<String>>,
    /// Reads only a proven directory attributes: by directory, raw model
    /// and raw effort.
    pub inferred: BTreeMap<(Dir, Cell, Cell), u64>,
    /// Each directory of `inferred` with every row its proofs name, raw.
    pub proofs: BTreeMap<Dir, BTreeSet<Item>>,
}

/// One file's status, cursor and block witness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
    pub mtime: (i64, u32),
    pub ctime: (i64, u32),
    pub cursor: u64,
    pub skipping: bool,
    pub blocks: Vec<[u8; 32]>,
}

/// What a later append needs to continue the count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resume {
    pub(super) files: BTreeMap<StreamKey, FileState>,
    /// Each file's distinct events.
    pub(super) streams: BTreeMap<StreamKey, BTreeSet<Event>>,
    /// Each file's malformed and over-cap line counts.
    pub(super) lines: BTreeMap<StreamKey, (u64, u64)>,
    /// The discovery gaps of the conversation's last round.
    pub(super) marks: Vec<GapKind>,
    /// O2: the Reads dropped to fit the conversation cap, never kept again.
    pub(super) dropped: BTreeSet<Key>,
    /// O2: set once the fallback ran; later unproven Reads are dropped too.
    pub(super) fallback: bool,
    /// The digest of the recorded roots the held events were classified by
    /// (Claude only): another one rebuilds the conversation.
    pub(super) roots: Option<Key>,
    /// The conversation was rebuilt whole from its sources (P6 boundary):
    /// its document was damaged, oversized or of another format, or its held
    /// counts were damaged, so counts of its deleted sources are lost. Kept by every later rebuild; only a
    /// store reset (`r`) clears it.
    pub(super) rebuilt: bool,
}

impl Resume {
    #[must_use]
    pub fn file(&self, stream: &StreamKey) -> Option<&FileState> {
        self.files.get(stream)
    }

    /// The digest of the roots the held events were classified by.
    #[must_use]
    pub const fn roots(&self) -> Option<&Key> {
        self.roots.as_ref()
    }

    /// Record the roots digest the held events are classified by.
    pub const fn set_roots(&mut self, roots: Option<Key>) {
        self.roots = roots;
    }

    /// Insert or replace `stream`'s status; the summary is untouched.
    pub fn set_file(&mut self, stream: StreamKey, state: FileState) {
        self.files.insert(stream, state);
    }

    /// The Reads no Skill call proves yet that name no seat skill directly
    /// and are within the path limit.
    #[must_use]
    pub fn pending_reads(&self) -> usize {
        // Proof is the same through either adapter; only Claude reads.
        let held = self.conversation(Adapter::Claude, "", false);
        self.unproven(&fold::proven_dirs(&held)).len()
    }

    /// The held evidence as the fold reads a conversation.
    fn conversation(&self, adapter: Adapter, id: &str, read: bool) -> Conversation {
        let keys: BTreeSet<&StreamKey> = self.streams.keys().chain(self.lines.keys()).collect();
        let streams = keys
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                let (malformed, over_cap) = self.lines.get(key).copied().unwrap_or((0, 0));
                Stream {
                    role: key.role.clone(),
                    copy: u32::try_from(index).unwrap_or(u32::MAX),
                    events: self
                        .streams
                        .get(key)
                        .map(|set| set.iter().cloned().collect())
                        .unwrap_or_default(),
                    malformed,
                    over_cap,
                }
            })
            .collect();
        Conversation {
            adapter,
            id: id.to_owned(),
            memberships: Vec::new(),
            evidence_complete: false,
            reseated: false,
            streams,
            gaps: self.marks.clone(),
            read,
        }
    }

    /// The held Reads no `proven` directory covers, within the path limit
    /// and naming no seat skill directly.
    fn unproven(&self, proven: &BTreeSet<Dir>) -> BTreeSet<Key> {
        self.streams
            .values()
            .flatten()
            .filter_map(|event| match event {
                Event::Read {
                    id,
                    direct: None,
                    ancestors,
                    over: false,
                    ..
                } if !ancestors.iter().any(|dir| proven.contains(dir)) => Some(*id),
                _ => None,
            })
            .collect()
    }
}

/// One conversation's durable state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conv {
    pub summary: Summary,
    pub resume: Resume,
}

impl Conv {
    #[must_use]
    pub fn new(adapter: Adapter, id: &str) -> Self {
        Self {
            summary: Summary {
                adapter,
                id: id.to_owned(),
                rows: BTreeMap::new(),
                orphans: 0,
                range: None,
                gaps: BTreeMap::new(),
                observations: 0,
                turns: 0,
                read: false,
                files: BTreeMap::new(),
                refused: None,
                names: BTreeMap::new(),
                inferred: BTreeMap::new(),
                proofs: BTreeMap::new(),
            },
            resume: Resume::default(),
        }
    }

    /// Add `events` read from `stream`. Evidence already held changes
    /// nothing; a refused conversation takes none.
    pub fn apply(&mut self, stream: &StreamKey, events: &[Event], caps: &Caps) {
        if self.summary.refused.is_some() {
            return;
        }
        let dropped = &self.resume.dropped;
        let set = self.resume.streams.entry(stream.clone()).or_default();
        for event in events {
            if let Event::Read { id, .. } = event
                && dropped.contains(id)
            {
                continue;
            }
            if !set.contains(event) {
                set.insert(event.clone());
            }
        }
        if self.resume.fallback {
            self.drop_unproven();
        }
        let conversation = self.conversation();
        let (observations, turns) = fold::size(&conversation);
        if observations > caps.per_conversation {
            self.refuse(Refused::Observations);
        } else if turns > caps.turns {
            self.refuse(Refused::Turns);
        } else {
            self.summarize_from(&conversation);
        }
    }

    /// Add `malformed` and `over_cap` lines read from `stream`, exactly.
    ///
    /// # Errors
    ///
    /// [`Overflow`] when the conversation's gap lines could no longer be
    /// added up ([`line_total`]): held counts no real source makes, so the
    /// conversation is DAMAGED and reset whole, nothing carried; the reader
    /// reads every file again from its start. Nothing of the update is kept.
    pub fn note(
        &mut self,
        stream: &StreamKey,
        malformed: u64,
        over_cap: u64,
    ) -> Result<(), Overflow> {
        if malformed == 0 && over_cap == 0 {
            return Ok(());
        }
        let mut lines = self.resume.lines.clone();
        let held = lines.entry(stream.clone()).or_default();
        let grown = held
            .0
            .checked_add(malformed)
            .zip(held.1.checked_add(over_cap));
        let Some(grown) = grown else {
            return Err(self.damaged());
        };
        *held = grown;
        if line_total(&lines, self.resume.marks.len()).is_none() {
            return Err(self.damaged());
        }
        self.resume.lines = lines;
        self.summarize();
        Ok(())
    }

    /// The conversation's discovery gaps for this round, each counted once.
    ///
    /// # Errors
    ///
    /// As [`Self::note`].
    pub fn set_gaps(&mut self, gaps: Vec<GapKind>) -> Result<(), Overflow> {
        if self.resume.marks == gaps {
            return Ok(());
        }
        if line_total(&self.resume.lines, gaps.len()).is_none() {
            return Err(self.damaged());
        }
        self.resume.marks = gaps;
        self.summarize();
        Ok(())
    }

    /// Route the conversation to damage: its held counts are ones no real
    /// source makes, so nothing of them is carried, as with a damaged
    /// document. It is reset whole and rebuilt from its sources; identity
    /// and a refusal are kept, as a refusal is retried only by a store reset.
    fn damaged(&mut self) -> Overflow {
        let refused = self.summary.refused;
        *self = Self::new(self.summary.adapter, &self.summary.id);
        if let Some(reason) = refused {
            self.refuse(reason);
        }
        self.mark_rebuilt();
        Overflow
    }

    /// The conversation is rebuilt whole from its sources (P6 boundary): the
    /// counts of its deleted sources are lost, so it carries a visible mark
    /// that every later rebuild keeps and only a store reset clears.
    pub fn mark_rebuilt(&mut self) {
        if !self.resume.rebuilt {
            self.resume.rebuilt = true;
            self.summarize();
        }
    }

    /// Rebuild the conversation from its sources: every file's state is
    /// forgotten, so each file still on disk is read again from its start.
    /// No evidence is dropped here: each stream keeps its events, line
    /// counts and source mark until a read of it from 0 is taken
    /// ([`Self::restart`]). So a stream that is gone, or is found gone by
    /// its own lstat, keeps its counts (P6, until `r`), under the
    /// classification it was read with. The O2 state, the roots digest and a
    /// refusal are kept: a refusal is retried only by a store reset.
    pub fn rebuild(&mut self) {
        self.resume.files.clear();
    }

    /// A read of `stream` from its start is taken: it replaces the events
    /// and line counts held for it. The source mark stays until the taken
    /// read marks it present. A refused conversation forgets them too: the
    /// taken read's line counts replace, never add to, the held ones.
    pub fn restart(&mut self, stream: &StreamKey) {
        let events = self.resume.streams.remove(stream);
        let lines = self.resume.lines.remove(stream);
        if events.is_some() || lines.is_some() {
            self.summarize();
        }
    }

    /// `stream` was read: it is on disk again if it had gone.
    pub fn source_present(&mut self, stream: &StreamKey) {
        if self.summary.files.get(stream) != Some(&Source::Present) {
            self.summary.files.insert(stream.clone(), Source::Present);
            self.summarize();
        }
    }

    /// `stream` is no longer on disk: its counts stay, visibly.
    pub fn source_gone(&mut self, stream: &StreamKey) {
        if self.summary.files.get(stream) != Some(&Source::Gone) {
            self.summary.files.insert(stream.clone(), Source::Gone);
            self.summarize();
        }
    }

    /// Hold the conversation under `caps.conv_cap` by the document size
    /// `size` measures (plan r4 §4): over it, the O2 fallback drops the
    /// unproven Reads without a `skills` component once; still over, the
    /// conversation is refused whole.
    pub fn bound(&mut self, caps: &Caps, size: impl Fn(&Self) -> u64) {
        if self.summary.refused.is_some() || size(self) <= caps.conv_cap {
            return;
        }
        if !self.resume.fallback {
            self.resume.fallback = true;
            self.drop_unproven();
            self.summarize();
            if size(self) <= caps.conv_cap {
                return;
            }
        }
        self.refuse(Refused::Size);
    }

    /// Drop every unproven Read whose path has no `skills` component.
    fn drop_unproven(&mut self) {
        let proven = fold::proven_dirs(&self.conversation());
        let mut gone = self.resume.unproven(&proven);
        for set in self.resume.streams.values() {
            for event in set {
                if let Event::Read {
                    id, skills: true, ..
                } = event
                {
                    gone.remove(id);
                }
            }
        }
        if gone.is_empty() {
            return;
        }
        for set in self.resume.streams.values_mut() {
            set.retain(|event| !matches!(event, Event::Read { id, .. } if gone.contains(id)));
        }
        self.resume.dropped.extend(gone);
    }

    /// Refuse the conversation whole: its evidence goes, its file statuses
    /// and coverage lines stay.
    fn refuse(&mut self, reason: Refused) {
        self.summary.refused = Some(reason);
        self.resume.streams.clear();
        self.resume.dropped.clear();
        self.summarize();
    }

    /// The held evidence as the fold reads a conversation.
    fn conversation(&self) -> Conversation {
        self.resume
            .conversation(self.summary.adapter, &self.summary.id, self.summary.read)
    }

    /// Derive the counters from the held evidence.
    pub(super) fn summarize(&mut self) {
        let conversation = self.conversation();
        self.summarize_from(&conversation);
    }

    /// Derive the counters from `conversation`, the held evidence.
    fn summarize_from(&mut self, conversation: &Conversation) {
        let refused = self.summary.refused;
        let counted = fold::count(conversation, refused.is_none());
        let (observations, turns) = fold::size(conversation);
        let summary = &mut self.summary;
        let adapter = summary.adapter;
        summary.rows = counted
            .rows
            .into_iter()
            .map(|((page, item, model, effort), cols)| ((page, item, adapter, model, effort), cols))
            .collect();
        summary.orphans = counted.orphans;
        summary.range = counted.range;
        summary.names = counted.names;
        summary.inferred = counted.inferred;
        summary.proofs = counted.proofs;
        summary.observations = observations;
        summary.turns = turns;
        let mut gaps = counted.gaps;
        if refused == Some(Refused::Size) {
            gaps.remove(&GapKind::Ledger);
            *gaps.entry(GapKind::OverCap).or_default() += 1;
        }
        let gone = summary
            .files
            .values()
            .filter(|source| **source == Source::Gone)
            .count();
        let dropped = self.resume.dropped.len();
        for (kind, count) in [
            (GapKind::SourceGone, gone),
            (GapKind::UnprovenReadsDropped, dropped),
            (GapKind::RebuiltFromSource, usize::from(self.resume.rebuilt)),
        ] {
            if count > 0 {
                gaps.insert(kind, u64::try_from(count).unwrap_or(u64::MAX));
            }
        }
        summary.gaps = gaps;
    }
}

/// A conversation's ae memberships and the client rule's flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Association {
    pub memberships: Vec<Membership>,
    pub evidence_complete: bool,
    pub reseated: bool,
}

pub type Memberships = BTreeMap<(Adapter, String), Association>;

/// One conversation as the derived view holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedConv {
    pub summary: Summary,
    pub sessions: BTreeSet<String>,
    pub client: Cell,
}

/// The derived view document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub generation: u64,
    pub convs: Vec<PublishedConv>,
    pub over_cap: u64,
}

impl Published {
    /// The total every sum a [`combine`] makes is a part of: orphans, each
    /// gap line, the dropped names, the over-cap line, every table cell and
    /// total. `None` past `u64`.
    #[must_use]
    pub fn total(&self) -> Option<u64> {
        self.convs
            .iter()
            .flat_map(conv_counts)
            .chain([self.over_cap])
            .try_fold(0_u64, u64::checked_add)
    }

    /// Publish `conv` in place of its conversation's entry, in `(adapter,
    /// id)` order.
    ///
    /// # Errors
    ///
    /// [`Overflow`] when the view's counts could no longer be added up
    /// ([`Self::total`]): the view is kept as it was, and the conversation
    /// is DAMAGED, to be rebuilt from its sources.
    pub fn admit(&mut self, conv: PublishedConv) -> Result<(), Overflow> {
        let key = |one: &PublishedConv| (one.summary.adapter, one.summary.id.clone());
        let at = self.convs.binary_search_by_key(&key(&conv), key);
        let old = match at {
            Ok(index) => Some(std::mem::replace(&mut self.convs[index], conv)),
            Err(index) => {
                self.convs.insert(index, conv);
                None
            }
        };
        if self.total().is_some() {
            return Ok(());
        }
        match (at, old) {
            (Ok(index), Some(old)) => self.convs[index] = old,
            (Err(index), _) => {
                self.convs.remove(index);
            }
            (Ok(_), None) => {}
        }
        Err(Overflow)
    }
}

/// `summary` with the scope and client its current `association` gives
/// (plan r3 §5): no association is no member session and no client.
#[must_use]
pub fn publish(summary: &Summary, association: Option<&Association>) -> PublishedConv {
    let Some(association) = association else {
        return PublishedConv {
            summary: summary.clone(),
            sessions: BTreeSet::new(),
            client: Cell::Unknown,
        };
    };
    let sessions = association
        .memberships
        .iter()
        .filter_map(|membership| membership.session.clone())
        .collect();
    let client = client(&Conversation {
        adapter: summary.adapter,
        id: summary.id.clone(),
        memberships: association.memberships.clone(),
        evidence_complete: association.evidence_complete,
        reseated: association.reseated,
        streams: Vec::new(),
        gaps: Vec::new(),
        read: summary.read,
    });
    PublishedConv {
        summary: summary.clone(),
        sessions,
        client,
    }
}

fn name_cell(names: &Names, kind: Kind, cell: &Cell) -> Cell {
    match cell {
        Cell::Value(name) => names.cell(kind, &super::Ident::Name(name.clone())),
        other => other.clone(),
    }
}

fn name_item(names: &Names, item: &Item) -> Item {
    match item {
        Item::Tool(tool) => Item::Tool(names.map(Kind::Tool, tool)),
        Item::Mcp { server, tool } => Item::Mcp {
            server: names.map(Kind::Server, server),
            tool: names.map(Kind::Tool, tool),
        },
        Item::Skill { name, scope } => Item::Skill {
            name: names.map(Kind::Skill, name),
            scope: *scope,
        },
    }
}

/// The page of `ask` over `published`.
#[must_use]
pub fn combine(published: &Published, ask: &Ask, caps: &Caps) -> View {
    combine_parts(&published.convs, published.over_cap, ask, caps)
}

/// The view-wide coverage only the reader knows (lead ruling Q2 = A): the
/// association sources whose evidence was unavailable, the memberships not
/// kept for a field over the limit, the files being read again, and each
/// unsupported conversation as its tool word and the ae sessions holding
/// it. Written beside the view in the same plan, with its generation; no
/// conversation id, path or transcript byte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Coverage {
    pub generation: u64,
    pub hidden: u64,
    pub oversize: u64,
    pub recounting: u64,
    /// One entry per unsupported conversation, sorted.
    pub unsupported: Vec<(String, BTreeSet<Option<String>>)>,
    /// Unsupported conversations left out to keep the document under its
    /// cap: drawn as one visible over-cap line, never dropped silently.
    pub omitted: u64,
}

impl Coverage {
    /// Add the coverage lines `ask`'s scope reaches to `view`.
    pub fn apply(&self, view: &mut View, ask: &Ask) {
        let mut unsupported: BTreeMap<&str, u64> = BTreeMap::new();
        for (tool, sessions) in &self.unsupported {
            let shown = match &ask.scope {
                Scope::All => true,
                Scope::This(None) => false,
                Scope::This(Some(session)) => sessions
                    .iter()
                    .any(|held| held.as_deref() == Some(session.as_str())),
            };
            if shown {
                let count = unsupported.entry(tool).or_default();
                *count = count.saturating_add(1);
            }
        }
        for (tool, count) in unsupported {
            view.gaps.push(super::GapLine {
                kind: GapKind::Unsupported(tool.to_owned()),
                conversation: None,
                count,
            });
        }
        if self.hidden > 0 {
            view.gaps.push(super::GapLine {
                kind: GapKind::Unreadable,
                conversation: None,
                count: self.hidden,
            });
        }
        if self.oversize > 0 {
            view.gaps.push(super::GapLine {
                kind: GapKind::FieldOverLimit,
                conversation: None,
                count: self.oversize,
            });
        }
        // Left out of the document by its cap: which session holds them is
        // not kept, so every scope that shows a session says so.
        if self.omitted > 0 && ask.scope != Scope::This(None) {
            view.gaps.push(super::GapLine {
                kind: GapKind::Ledger,
                conversation: None,
                count: self.omitted,
            });
        }
        view.gaps.sort();
        view.recounting = self.recounting;
    }
}

/// The page of `ask` over the published conversations `convs`, `over_cap`
/// more left out of the view by its cap.
#[must_use]
pub fn combine_parts(convs: &[PublishedConv], over_cap: u64, ask: &Ask, caps: &Caps) -> View {
    let mut view = fold::blank(ask);
    if ask.scope == Scope::This(None) {
        view.state = State::NoSessionId;
        return view;
    }
    let mut keep: BTreeMap<Kind, BTreeSet<String>> = BTreeMap::new();
    for conv in convs {
        for (kind, set) in &conv.summary.names {
            keep.entry(*kind).or_default().extend(set.iter().cloned());
        }
    }
    let names = Names::of(keep, caps.idents);
    let mut lines = Lines::default();
    let mut obs = Vec::new();
    for conv in convs {
        let summary = &conv.summary;
        let in_scope = match &ask.scope {
            Scope::All => true,
            Scope::This(session) => session
                .as_ref()
                .is_some_and(|session| conv.sessions.contains(session)),
        };
        if !in_scope {
            continue;
        }
        view.listed += 1;
        view.read += u64::from(summary.read);
        for (kind, count) in &summary.gaps {
            // The rebuilt-from-source mark is one line summed over the
            // conversations.
            let id = (*kind != GapKind::RebuiltFromSource).then_some(summary.id.as_str());
            lines.add(kind.clone(), id, *count);
        }
        if summary.refused.is_some() {
            continue;
        }
        if let Some((from, to)) = summary.range {
            view.range = Some(
                view.range
                    .map_or((from, to), |(a, b)| (a.min(from), b.max(to))),
            );
        }
        view.orphans += summary.orphans;
        for ((page, item, harness, model, effort), cols) in &summary.rows {
            if *page != ask.page {
                continue;
            }
            obs.push(Obs {
                item: name_item(&names, item),
                harness: *harness,
                client: conv.client.clone(),
                model: name_cell(&names, Kind::Model, model),
                effort: name_cell(&names, Kind::Effort, effort),
                cols: *cols,
            });
        }
        if ask.page != Page::Skills {
            continue;
        }
        for ((dir, model, effort), reads) in &summary.inferred {
            let item = summary
                .proofs
                .get(dir)
                .and_then(|items| items.iter().map(|item| name_item(&names, item)).min());
            let Some(item) = item else { continue };
            obs.push(Obs {
                item,
                harness: summary.adapter,
                client: conv.client.clone(),
                model: name_cell(&names, Kind::Model, model),
                effort: name_cell(&names, Kind::Effort, effort),
                cols: Cols::Skill(SkillCols {
                    reads: *reads,
                    ..SkillCols::default()
                }),
            });
        }
    }
    lines.add(GapKind::IdentCap, None, names.dropped);
    lines.add(GapKind::ViewOverCap, None, over_cap);
    view.gaps = lines.finish();
    if view.listed == 0 {
        view.state = State::NoSupported;
    }
    fold::tables(&mut view, &obs, caps);
    view
}

#[cfg(test)]
#[path = "incr_tests.rs"]
mod tests;
