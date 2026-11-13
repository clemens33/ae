//! The pure fold (plan §5, §7, §8): conversations to one [`View`].
//!
//! Every step is a function of SETS, never of order: duplicate conversations
//! merge by `(adapter, id)`, events gather into per-key variant sets, and
//! every table is sorted before it is cut. So any input order, duplicate
//! feeding or split point gives one view (the fuzz invariant).

use super::{
    Adapter, Ask, Caps, Cols, Conversation, DetailRow, Dir, Event, Filter, GapKind, GapLine,
    HarnessGroup, Ident, Item, Key, Kind, Load, McpCols, McpOutcome, Membership, More, Page,
    Resource, Row, Scope, ScopeClass, SkillCols, State, Stream, StreamRole, ToolCols, VIEW_RESERVE,
    View, client,
};
use crate::meta::RecordedClient;
use crate::stats::Cell;
use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

/// One call as its attempt record states it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Call {
    tool: Ident,
    mcp: Option<(Ident, Ident)>,
    model: Option<Ident>,
    turn: Option<Key>,
    message: Option<Key>,
}

/// A Skill call's injected base directory (Q1 b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Base {
    class: ScopeClass,
    resource: Resource,
    dir: Dir,
}

/// One terminal result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Reply {
    error: bool,
    load: Option<Load>,
}

/// One `Read` call's path evidence (Q2 b).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReadRec {
    direct: Option<(Ident, ScopeClass, Resource)>,
    ancestors: Vec<Dir>,
    over: bool,
}

/// One Codex `McpToolCall` item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct McpRec {
    server: Ident,
    tool: Ident,
    outcome: McpOutcome,
    turn: Option<Key>,
}

/// Every distinct value seen for one native key, sorted: one value is the
/// evidence, more than one is a conflict whatever their order. A sorted
/// `Vec` of exact capacity 1 for the usual single value, never a `BTreeSet`,
/// whose one value would allocate a whole B-tree leaf (see [`charge`]).
type Variants<T> = BTreeMap<Key, Vec<T>>;

fn put<T: Ord>(map: &mut Variants<T>, id: Key, value: T) {
    let set = map.entry(id).or_insert_with(|| Vec::with_capacity(1));
    if let Err(at) = set.binary_search(&value) {
        set.insert(at, value);
    }
}

/// The single value of a variant set, `None` on a conflict.
fn only<T>(set: &[T]) -> Option<&T> {
    match set {
        [one] => Some(one),
        _ => None,
    }
}

/// One stream's keyed evidence: its copies alias, so they share a group.
#[derive(Debug, Default)]
struct Group {
    calls: Variants<Call>,
    skills: Variants<Ident>,
    bases: Variants<Base>,
    replies: Variants<Reply>,
    reads: Variants<ReadRec>,
    mcps: Variants<McpRec>,
}

impl Group {
    fn take(&mut self, event: &Event) {
        match event {
            Event::Stamp(_) | Event::Context { .. } => {}
            Event::Attempt {
                id,
                tool,
                mcp,
                model,
                turn,
                message,
            } => put(
                &mut self.calls,
                *id,
                Call {
                    tool: tool.clone(),
                    mcp: mcp.clone(),
                    model: model.clone(),
                    turn: *turn,
                    message: *message,
                },
            ),
            Event::Skill { id, skill } => put(&mut self.skills, *id, skill.clone()),
            Event::SkillBase {
                id,
                class,
                resource,
                dir,
            } => put(
                &mut self.bases,
                *id,
                Base {
                    class: *class,
                    resource: *resource,
                    dir: *dir,
                },
            ),
            Event::Result { id, error, load } => put(
                &mut self.replies,
                *id,
                Reply {
                    error: *error,
                    load: *load,
                },
            ),
            Event::Read {
                id,
                direct,
                ancestors,
                over,
                skills: _,
            } => put(
                &mut self.reads,
                *id,
                ReadRec {
                    direct: direct.clone(),
                    ancestors: ancestors.clone(),
                    over: *over,
                },
            ),
            Event::Mcp {
                id,
                server,
                tool,
                outcome,
                turn,
            } => put(
                &mut self.mcps,
                *id,
                McpRec {
                    server: server.clone(),
                    tool: tool.clone(),
                    outcome: *outcome,
                    turn: *turn,
                },
            ),
        }
    }

    /// Drop every record of call `id` (a replayed child call).
    fn drop_call(&mut self, id: &Key) {
        self.calls.remove(id);
        self.skills.remove(id);
        self.bases.remove(id);
        self.replies.remove(id);
        self.reads.remove(id);
    }

    /// The call of `id`, its differing copies merged: fields that disagree
    /// are unknown (plan §5 alias (i)).
    fn call(&self, id: &Key) -> Option<Call> {
        let set = self.calls.get(id)?;
        let first = set.first()?;
        if set.len() == 1 {
            return Some(first.clone());
        }
        let tool = if set.iter().all(|call| call.tool == first.tool) {
            first.tool.clone()
        } else {
            Ident::Unresolved(Kind::Tool)
        };
        let mcp = if set.iter().all(|call| call.mcp == first.mcp) {
            first.mcp.clone()
        } else {
            Some((
                Ident::Unresolved(Kind::Server),
                Ident::Unresolved(Kind::Tool),
            ))
        };
        Some(Call {
            tool,
            mcp,
            model: None,
            turn: None,
            message: None,
        })
    }

    fn outcome(&self, id: &Key) -> Outcome {
        match self.replies.get(id) {
            None => Outcome::Missing,
            Some(set) => only(set).map_or(Outcome::Conflict, |reply| Outcome::One(*reply)),
        }
    }

    /// Keys with differing variants, and Codex MCP items that conflict.
    fn conflicts(&self) -> u64 {
        fn split<T>(map: &Variants<T>) -> usize {
            map.values().filter(|set| set.len() > 1).count()
        }
        let mcp = self
            .mcps
            .values()
            .filter(|set| only(set).is_none_or(|rec| rec.outcome == McpOutcome::Conflict))
            .count();
        let total = split(&self.calls)
            + split(&self.skills)
            + split(&self.bases)
            + split(&self.replies)
            + split(&self.reads)
            + mcp;
        u64::try_from(total).unwrap_or(u64::MAX)
    }

    fn orphans(&self) -> u64 {
        let count = self
            .replies
            .keys()
            .filter(|id| !self.calls.contains_key(id))
            .count();
        u64::try_from(count).unwrap_or(u64::MAX)
    }

    fn over_reads(&self) -> u64 {
        let count = self
            .reads
            .values()
            .filter(|set| set.iter().any(|rec| rec.over))
            .count();
        u64::try_from(count).unwrap_or(u64::MAX)
    }
}

/// A call's terminal result.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    Missing,
    Conflict,
    One(Reply),
}

/// One conversation, gathered.
struct Gathered<'a> {
    conversation: &'a Conversation,
    groups: BTreeMap<StreamRole, Group>,
    /// Codex turn contexts: (model, effort) per turn.
    contexts: Variants<(Ident, Ident)>,
    range: Option<(i64, i64)>,
    client: Cell,
}

fn gather(conversation: &Conversation) -> Gathered<'_> {
    let mut groups: BTreeMap<StreamRole, Group> = BTreeMap::new();
    let mut contexts = Variants::new();
    let mut range: Option<(i64, i64)> = None;
    for stream in &conversation.streams {
        let group = groups.entry(stream.role.clone()).or_default();
        for event in &stream.events {
            match event {
                Event::Stamp(at) => {
                    range =
                        Some(range.map_or((*at, *at), |(from, to)| (from.min(*at), to.max(*at))));
                }
                Event::Context {
                    turn,
                    model,
                    effort,
                } => put(&mut contexts, *turn, (model.clone(), effort.clone())),
                _ => group.take(event),
            }
        }
    }
    // Plan §5 alias (ii): a child record whose message id is a parent
    // message id is the parent's call replayed, counted once.
    let parent: BTreeSet<Key> =
        groups
            .get(&StreamRole::Parent)
            .map_or_else(BTreeSet::new, |group| {
                group
                    .calls
                    .values()
                    .flatten()
                    .filter_map(|call| call.message)
                    .collect()
            });
    for (role, group) in &mut groups {
        if *role == StreamRole::Parent {
            continue;
        }
        let replayed: Vec<Key> = group
            .calls
            .iter()
            .filter(|(_, set)| {
                set.iter().any(|call| {
                    call.message
                        .is_some_and(|message| parent.contains(&message))
                })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &replayed {
            group.drop_call(id);
        }
    }
    let client = client(conversation);
    Gathered {
        conversation,
        groups,
        contexts,
        range,
        client,
    }
}

impl Gathered<'_> {
    /// The (model, effort) cells of a call (plan §2): Claude's model from
    /// the attempt, effort never observed; Codex both from the call's turn,
    /// unknown with no turn, no context or differing contexts.
    fn dims(&self, model: Option<&Ident>, turn: Option<Key>, names: &Names) -> (Cell, Cell) {
        match self.conversation.adapter {
            Adapter::Claude => (
                model.map_or(Cell::Unknown, |model| names.cell(Kind::Model, model)),
                Cell::Unknown,
            ),
            Adapter::Codex => turn
                .and_then(|turn| self.contexts.get(&turn))
                .map(Vec::as_slice)
                .and_then(only)
                .map_or((Cell::Unknown, Cell::Unknown), |(model, effort)| {
                    (
                        names.cell(Kind::Model, model),
                        names.cell(Kind::Effort, effort),
                    )
                }),
        }
    }

    fn obs(&self, item: Item, dims: (Cell, Cell), cols: Cols) -> Obs {
        Obs {
            item,
            harness: self.conversation.adapter,
            client: self.client.clone(),
            model: dims.0,
            effort: dims.1,
            cols,
        }
    }

    /// Each counted, loaded, unconflicted Skill call's proven directory
    /// with the row it names, as `names` maps it (Q2 b).
    fn proofs<'s>(&'s self, names: &'s Names) -> impl Iterator<Item = (Dir, Item)> + 's {
        self.groups.values().flat_map(move |group| {
            group.skills.iter().filter_map(move |(id, skills)| {
                let base = group.bases.get(id).map(Vec::as_slice).and_then(only)?;
                let loaded = matches!(
                    group.outcome(id),
                    Outcome::One(Reply {
                        error: false,
                        load: Some(Load::Loaded),
                    })
                );
                loaded.then(|| {
                    (
                        base.dir,
                        Item::Skill {
                            name: skill_name(skills, names),
                            scope: Some((base.class, base.resource)),
                        },
                    )
                })
            })
        })
    }

    /// Directories a proof covers, with the least row the directory's
    /// proofs name; conversation-wide (Q2 b).
    fn proven(&self, names: &Names) -> BTreeMap<Dir, Item> {
        let mut out: BTreeMap<Dir, Item> = BTreeMap::new();
        for (dir, item) in self.proofs(names) {
            out.entry(dir)
                .and_modify(|seen| {
                    if item < *seen {
                        *seen = item.clone();
                    }
                })
                .or_insert(item);
        }
        out
    }

    /// The page's observations; with `inferred`, a Read only a proven
    /// directory attributes is counted there by directory instead.
    fn observe(
        &self,
        page: Page,
        names: &Names,
        out: &mut Vec<Obs>,
        mut inferred: Option<&mut Inferred>,
    ) {
        let proven = if page == Page::Skills {
            self.proven(names)
        } else {
            BTreeMap::new()
        };
        for group in self.groups.values() {
            match page {
                Page::Tools => self.tools(group, names, out),
                Page::Mcp => self.mcp(group, names, out),
                Page::Skills => {
                    self.skills(group, names, &proven, out, inferred.as_deref_mut());
                }
            }
        }
    }

    fn tools(&self, group: &Group, names: &Names, out: &mut Vec<Obs>) {
        for id in group.calls.keys() {
            let Some(call) = group.call(id) else { continue };
            let mut cols = ToolCols {
                attempts: 1,
                ..ToolCols::default()
            };
            match group.outcome(id) {
                Outcome::Missing => cols.unresolved = 1,
                Outcome::Conflict => cols.returned = 1,
                Outcome::One(reply) => {
                    cols.returned = 1;
                    cols.error = u64::from(reply.error);
                }
            }
            let dims = self.dims(call.model.as_ref(), call.turn, names);
            let item = Item::Tool(names.map(Kind::Tool, &call.tool));
            out.push(self.obs(item, dims, Cols::Tool(cols)));
        }
    }

    fn mcp(&self, group: &Group, names: &Names, out: &mut Vec<Obs>) {
        for id in group.calls.keys() {
            let Some(call) = group.call(id) else { continue };
            let Some((server, tool)) = &call.mcp else {
                continue;
            };
            // Claude: a result as a Tool result, never a success (D4).
            let mut cols = McpCols {
                attempts: 1,
                ..McpCols::default()
            };
            match group.outcome(id) {
                Outcome::Missing => cols.unresolved = 1,
                Outcome::Conflict => cols.returned = 1,
                Outcome::One(reply) => {
                    cols.returned = 1;
                    cols.error = u64::from(reply.error);
                }
            }
            let item = Item::Mcp {
                server: names.map(Kind::Server, server),
                tool: names.map(Kind::Tool, tool),
            };
            let dims = self.dims(call.model.as_ref(), call.turn, names);
            out.push(self.obs(item, dims, Cols::Mcp(cols)));
        }
        for set in group.mcps.values() {
            let Some(first) = set.first() else { continue };
            let one = only(set);
            let server = if set.iter().all(|rec| rec.server == first.server) {
                names.map(Kind::Server, &first.server)
            } else {
                Ident::Unresolved(Kind::Server)
            };
            let tool = if set.iter().all(|rec| rec.tool == first.tool) {
                names.map(Kind::Tool, &first.tool)
            } else {
                Ident::Unresolved(Kind::Tool)
            };
            let mut cols = McpCols {
                attempts: 1,
                returned: 1,
                ..McpCols::default()
            };
            match one.map(|rec| rec.outcome) {
                Some(McpOutcome::Success) => cols.success = 1,
                Some(McpOutcome::Error) => cols.error = 1,
                Some(McpOutcome::Returned | McpOutcome::Conflict) | None => {}
            }
            let dims = self.dims(None, one.and_then(|rec| rec.turn), names);
            out.push(self.obs(Item::Mcp { server, tool }, dims, Cols::Mcp(cols)));
        }
    }

    fn skills(
        &self,
        group: &Group,
        names: &Names,
        proven: &BTreeMap<Dir, Item>,
        out: &mut Vec<Obs>,
        mut inferred: Option<&mut Inferred>,
    ) {
        for (id, skills) in &group.skills {
            let mut cols = SkillCols {
                attempts: 1,
                ..SkillCols::default()
            };
            match group.outcome(id) {
                Outcome::Missing => cols.unresolved = 1,
                Outcome::Conflict => cols.unknown = 1,
                Outcome::One(reply) => match (reply.error, reply.load) {
                    (false, Some(Load::Loaded)) => cols.loaded = 1,
                    (true, _) | (false, Some(Load::NotLoaded)) => cols.not_loaded = 1,
                    (false, Some(Load::Unknown) | None) => cols.unknown = 1,
                },
            }
            let scope = group
                .bases
                .get(id)
                .map(Vec::as_slice)
                .and_then(only)
                .map(|base| (base.class, base.resource));
            let item = Item::Skill {
                name: skill_name(skills, names),
                scope,
            };
            let call = group.call(id);
            let dims = self.dims(
                call.as_ref().and_then(|call| call.model.as_ref()),
                call.as_ref().and_then(|call| call.turn),
                names,
            );
            out.push(self.obs(item, dims, Cols::Skill(cols)));
        }
        for (id, set) in &group.reads {
            let Some(read) = only(set) else { continue };
            if !matches!(group.outcome(id), Outcome::One(Reply { error: false, .. })) {
                continue;
            }
            let (item, via) = if let Some((name, class, resource)) = &read.direct {
                let item = Item::Skill {
                    name: names.map(Kind::Skill, name),
                    scope: Some((*class, *resource)),
                };
                (Some(item), None)
            } else {
                let dir = read.ancestors.iter().find(|dir| proven.contains_key(dir));
                (dir.and_then(|dir| proven.get(dir)).cloned(), dir.copied())
            };
            let Some(item) = item else { continue };
            let call = group.call(id);
            let dims = self.dims(
                call.as_ref().and_then(|call| call.model.as_ref()),
                call.as_ref().and_then(|call| call.turn),
                names,
            );
            if let (Some(dir), Some(sink)) = (via, inferred.as_deref_mut()) {
                *sink.entry((dir, dims.0, dims.1)).or_default() += 1;
                continue;
            }
            let cols = SkillCols {
                reads: 1,
                ..SkillCols::default()
            };
            out.push(self.obs(item, dims, Cols::Skill(cols)));
        }
    }
}

fn skill_name(skills: &[Ident], names: &Names) -> Ident {
    only(skills).map_or(Ident::Unresolved(Kind::Skill), |name| {
        names.map(Kind::Skill, name)
    })
}

/// One counted observation with its joint dimensions.
pub(super) struct Obs {
    pub(super) item: Item,
    pub(super) harness: Adapter,
    pub(super) client: Cell,
    pub(super) model: Cell,
    pub(super) effort: Cell,
    pub(super) cols: Cols,
}

impl Obs {
    fn matches(&self, filter: &Filter) -> bool {
        match filter {
            Filter::All => true,
            Filter::Harness(harness) => self.harness == *harness,
            Filter::Client(cell) => self.client == *cell,
            Filter::Model(cell) => self.model == *cell,
            Filter::Effort(cell) => self.effort == *cell,
        }
    }
}

/// The identity tables (plan §7): at most `cap` names per kind, the
/// smallest by byte order; any other name is `unresolved (cap)`.
pub(super) struct Names {
    keep: BTreeMap<Kind, BTreeSet<String>>,
    pub(super) dropped: u64,
}

impl Names {
    fn over<'a, 'b: 'a>(gathered: impl Iterator<Item = &'a Gathered<'b>>, cap: usize) -> Self {
        let mut keep: BTreeMap<Kind, BTreeSet<String>> = BTreeMap::new();
        let mut see = |kind: Kind, ident: &Ident| {
            if let Ident::Name(name) = ident
                && !keep.get(&kind).is_some_and(|set| set.contains(name))
            {
                keep.entry(kind).or_default().insert(name.clone());
            }
        };
        for one in gathered {
            for (model, effort) in one.contexts.values().flatten() {
                see(Kind::Model, model);
                see(Kind::Effort, effort);
            }
            for group in one.groups.values() {
                for call in group.calls.values().flatten() {
                    see(Kind::Tool, &call.tool);
                    if let Some((server, tool)) = &call.mcp {
                        see(Kind::Server, server);
                        see(Kind::Tool, tool);
                    }
                    if let Some(model) = &call.model {
                        see(Kind::Model, model);
                    }
                }
                for skill in group.skills.values().flatten() {
                    see(Kind::Skill, skill);
                }
                for read in group.reads.values().flatten() {
                    if let Some((name, ..)) = &read.direct {
                        see(Kind::Skill, name);
                    }
                }
                for rec in group.mcps.values().flatten() {
                    see(Kind::Server, &rec.server);
                    see(Kind::Tool, &rec.tool);
                }
            }
        }
        Self::of(keep, cap)
    }

    /// The tables over every name seen, `keep`, cut to `cap` per kind.
    pub(super) fn of(mut keep: BTreeMap<Kind, BTreeSet<String>>, cap: usize) -> Self {
        let mut dropped = 0_u64;
        for set in keep.values_mut() {
            while set.len() > cap {
                set.pop_last();
                dropped += 1;
            }
        }
        Self { keep, dropped }
    }

    pub(super) fn map(&self, kind: Kind, ident: &Ident) -> Ident {
        match ident {
            Ident::Name(name) if !self.keep.get(&kind).is_some_and(|set| set.contains(name)) => {
                Ident::Unresolved(kind)
            }
            _ => ident.clone(),
        }
    }

    pub(super) fn cell(&self, kind: Kind, ident: &Ident) -> Cell {
        match self.map(kind, ident) {
            Ident::Name(name) => Cell::Value(name),
            Ident::Unresolved(_) => Cell::Unknown,
        }
    }
}

/// Merge duplicate feedings of one conversation by `(adapter, id)`; the
/// result is sorted by that key (plan §7 canonical order). A key fed once,
/// or fed only equal copies, stays borrowed; any other key is [`union`].
fn merge(conversations: &[Conversation]) -> Vec<Cow<'_, Conversation>> {
    let mut by: BTreeMap<(Adapter, &str), Vec<&Conversation>> = BTreeMap::new();
    for conversation in conversations {
        by.entry((conversation.adapter, conversation.id.as_str()))
            .or_default()
            .push(conversation);
    }
    by.into_values().filter_map(|feeds| union(&feeds)).collect()
}

/// The streams of a [`union`]: per `(role, copy)` its distinct events and
/// its feeds' largest malformed and over-cap counts.
type Union<'a> = BTreeMap<(&'a StreamRole, u32), (BTreeSet<&'a Event>, u64, u64)>;

/// One conversation's unequal feeds, owning only their union (B2 review r2
/// F2). Memberships and gaps are the first feed's, then each other feed's
/// not yet held; [`charge`] counts every one it keeps. Each `(role, copy)`
/// stream is held once with each distinct event once and the largest
/// malformed and over-cap counts of its feeds, which [`canonical`],
/// `gather` and `cover` read alike. The fold's own event storage is then
/// the canonical evidence [`charge`] counts, however the feeds overlap; the
/// borrowed sets built here live only while it runs.
fn union<'a>(feeds: &[&'a Conversation]) -> Option<Cow<'a, Conversation>> {
    let (&first, rest) = feeds.split_first()?;
    if rest.iter().all(|feed| *feed == first) {
        return Some(Cow::Borrowed(first));
    }
    let mut memberships: Vec<&Membership> = first.memberships.iter().collect();
    let mut gaps: Vec<&GapKind> = first.gaps.iter().collect();
    for feed in rest {
        for membership in &feed.memberships {
            if !memberships.contains(&membership) {
                memberships.push(membership);
            }
        }
        for gap in &feed.gaps {
            if !gaps.contains(&gap) {
                gaps.push(gap);
            }
        }
    }
    let mut streams = Union::new();
    let (mut evidence_complete, mut reseated, mut read) = (true, false, true);
    for feed in feeds {
        evidence_complete &= feed.evidence_complete;
        reseated |= feed.reseated;
        read &= feed.read;
        for stream in &feed.streams {
            let (events, malformed, over_cap) =
                streams.entry((&stream.role, stream.copy)).or_default();
            events.extend(&stream.events);
            *malformed = (*malformed).max(stream.malformed);
            *over_cap = (*over_cap).max(stream.over_cap);
        }
    }
    Some(Cow::Owned(Conversation {
        adapter: first.adapter,
        id: first.id.clone(),
        memberships: memberships.into_iter().cloned().collect(),
        evidence_complete,
        reseated,
        streams: streams
            .into_iter()
            .map(|((role, copy), (events, malformed, over_cap))| Stream {
                role: role.clone(),
                copy,
                events: events.into_iter().cloned().collect(),
                malformed,
                over_cap,
            })
            .collect(),
        gaps: gaps.into_iter().cloned().collect(),
        read,
    }))
}

/// A conversation's canonical evidence (plan §7): each `(role, copy)`
/// stream once, holding each distinct event once, however the conversation
/// was fed. Counting, charging and the turn cap read only this, so an
/// overlapping, split or repeated feed is admitted like its union.
type Canonical<'a> = BTreeMap<(&'a StreamRole, u32), BTreeSet<&'a Event>>;

fn canonical(conversation: &Conversation) -> Canonical<'_> {
    let mut out: Canonical<'_> = BTreeMap::new();
    for stream in &conversation.streams {
        out.entry((&stream.role, stream.copy))
            .or_default()
            .extend(&stream.events);
    }
    out
}

/// Observations a conversation holds: every distinct event of every
/// stream but a timestamp.
fn observations(canonical: &Canonical<'_>) -> usize {
    canonical
        .values()
        .flatten()
        .filter(|event| !matches!(event, Event::Stamp(_)))
        .count()
}

/// Distinct Codex turn contexts.
fn turns(canonical: &Canonical<'_>) -> usize {
    canonical
        .values()
        .flatten()
        .filter_map(|event| match event {
            Event::Context { turn, .. } => Some(*turn),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .len()
}

fn ident_bytes(ident: &Ident) -> usize {
    match ident {
        Ident::Name(name) => name.len(),
        Ident::Unresolved(_) => 0,
    }
}

/// The heap bytes one event owns: its identity names and Read ancestors.
fn owned(event: &Event) -> usize {
    match event {
        Event::Attempt {
            tool, mcp, model, ..
        } => {
            ident_bytes(tool)
                + mcp
                    .as_ref()
                    .map_or(0, |(server, tool)| ident_bytes(server) + ident_bytes(tool))
                + model.as_ref().map_or(0, ident_bytes)
        }
        Event::Skill { skill, .. } => ident_bytes(skill),
        Event::Read {
            direct, ancestors, ..
        } => {
            direct.as_ref().map_or(0, |(name, ..)| ident_bytes(name))
                + ancestors.len() * size_of::<Dir>()
        }
        Event::Mcp { server, tool, .. } => ident_bytes(server) + ident_bytes(tool),
        Event::Context { model, effort, .. } => ident_bytes(model) + ident_bytes(effort),
        Event::Stamp(_) | Event::SkillBase { .. } | Event::Result { .. } => 0,
    }
}

/// The longest single name one event holds.
fn longest(event: &Event) -> usize {
    let most = |idents: &[Option<&Ident>]| {
        idents
            .iter()
            .copied()
            .flatten()
            .map(ident_bytes)
            .max()
            .unwrap_or(0)
    };
    match event {
        Event::Attempt {
            tool, mcp, model, ..
        } => most(&[
            Some(tool),
            mcp.as_ref().map(|(server, _)| server),
            mcp.as_ref().map(|(_, tool)| tool),
            model.as_ref(),
        ]),
        Event::Skill { skill, .. } => most(&[Some(skill)]),
        Event::Read { direct, .. } => most(&[direct.as_ref().map(|(name, ..)| name)]),
        Event::Mcp { server, tool, .. } => most(&[Some(server), Some(tool)]),
        Event::Context { model, effort, .. } => most(&[Some(model), Some(effort)]),
        Event::Stamp(_) | Event::SkillBase { .. } | Event::Result { .. } => 0,
    }
}

/// The ledger's declared allocator model (plan §7, B2 review r1 F2). A
/// non-root std B-tree node is at least 5 of its [`LEAF`] slots full, so a
/// map's nodes and internal edges stay within `TREE` times its entries.
const TREE: usize = 3;
/// The slots of one std B-tree leaf: a map holding one entry allocates them.
const LEAF: usize = 11;
/// A `Vec` grown by pushing holds at most twice its length.
const GROWN: usize = 2;
/// Every fold phase that clones a counted observation's names, all live at
/// once at the end of the fold: the gathered record, the identity name set,
/// the observation (item, client, model, effort), the row key, the Detail
/// key and the proven-directory map. Each clone is at most the event's own
/// names plus the conversation's longest name or client label, `CLONES`
/// times: an overestimate of the 5 item and 6 dimension clones.
const CLONES: usize = 8;

/// The fixed fold-temporary bytes of one counted observation, all phases.
const fn temporary() -> usize {
    // The gathered map entry and its variant slot (an `Event` is at least
    // as large as every gathered record; pinned by a test).
    TREE * size_of::<(Key, Vec<Event>)>()
        + GROWN * size_of::<Event>()
        // The identity name set.
        + TREE * size_of::<String>()
        // The observation list.
        + GROWN * size_of::<Obs>()
        // The row and Detail maps, and the proven-directory map. Both maps are
        // charged a whole Detail-sized entry; the row map holds a borrowed item
        // and its counts and the Detail map a key and one count. The surplus
        // pays for the per-harness group map (#338), pinned by a test.
        + 2 * TREE * size_of::<(DetailKey, Cols)>()
        + TREE * size_of::<(Dir, Item)>()
        // Spare: one keyed entry, the retired shared-id count (O1 = c).
        + TREE * size_of::<(Key, u32)>()
}

/// The fixed fold-temporary bytes of one gathered stream role: its six
/// variant maps, each at least one leaf.
const fn group_floor() -> usize {
    size_of::<(StreamRole, Group)>() + 6 * LEAF * size_of::<(Key, Vec<Event>)>()
}

/// The fixed fold-temporary bytes of one admitted conversation: its gathered
/// record, its context map and its gap-line entries, each at least one leaf.
fn conversation_floor() -> usize {
    size_of::<Option<Gathered<'_>>>()
        + LEAF * size_of::<(Key, Vec<(Ident, Ident)>)>()
        + LEAF * size_of::<((GapKind, Option<String>), u64)>()
}

/// Fold-wide tables that exist once whatever is admitted: the name sets per
/// kind, the row, Detail and filter tables, each at least a leaf.
/// They are held inside [`VIEW_RESERVE`], never charged against admission;
/// the assertion below keeps them within half of it at compile time.
const fn fold_floor() -> usize {
    16 * LEAF * (size_of::<(DetailKey, Cols)>() + size_of::<(Key, Vec<Event>)>())
}

const _: () = assert!(
    fold_floor() < VIEW_RESERVE / 2,
    "the view reserve holds the fold-wide tables' first leaves"
);

/// The bytes the ledger charges a conversation (plan §7, B2 review r1 F2):
/// what the caller retains for it, plus every fold-temporary byte its
/// admission adds, under the declared model above.
///
/// Retained: the conversation, membership, gap and stream structs with
/// every string they own, and each canonical event at its fixed size plus
/// the names it owns. The ownership boundary: the charge is of the
/// CANONICAL conversation, so the caller (the B3 reader) must keep its
/// streams canonical, one event once per `(role, copy)` stream, and bound
/// any duplicate input itself; admission never depends on how the caller's
/// buffers repeat.
///
/// Fold temporaries: per gathered stream role [`group_floor`]; per counted
/// observation (a distinct non-timestamp event of a role; copies alias)
/// [`temporary`] plus [`CLONES`] times its names and the conversation's
/// longest name or client label; the conversation's [`conversation_floor`];
/// and every gap again, as its gap line clones it.
fn charge(conversation: &Conversation) -> usize {
    let canonical = canonical(conversation);
    let mut longest_name = 0;
    let mut bytes = size_of::<Conversation>() + conversation.id.len() + conversation_floor();
    for membership in &conversation.memberships {
        let label = match &membership.client {
            RecordedClient::Label(label) => label.len(),
            RecordedClient::Missing | RecordedClient::Invalid => 0,
        };
        longest_name = longest_name.max(label);
        bytes += size_of::<Membership>()
            + membership.session.as_ref().map_or(0, String::len)
            + membership.slot.len()
            + label;
    }
    for gap in &conversation.gaps {
        let text = match gap {
            GapKind::Unsupported(tool) => tool.len(),
            _ => 0,
        };
        bytes += 2 * (size_of::<GapKind>() + text);
    }
    let mut counted: BTreeSet<(&StreamRole, &Event)> = BTreeSet::new();
    for ((role, _), events) in &canonical {
        bytes += size_of::<Stream>() + stream_role_bytes(role);
        for event in events {
            bytes += size_of::<Event>() + owned(event);
            longest_name = longest_name.max(longest(event));
            if !matches!(event, Event::Stamp(_)) {
                counted.insert((*role, *event));
            }
        }
    }
    let roles: BTreeSet<&StreamRole> = canonical.keys().map(|(role, _)| *role).collect();
    bytes += roles.len() * group_floor();
    for (_, event) in &counted {
        bytes += temporary() + CLONES * (owned(event) + longest_name);
    }
    bytes
}

fn stream_role_bytes(role: &StreamRole) -> usize {
    match role {
        StreamRole::Parent => 0,
        StreamRole::Child(name) => name.len(),
    }
}

/// What admission needs to know of one conversation: its [`charge`] and
/// whether it is over the observation or turn cap (refused alone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Weight {
    charge: usize,
    alone: bool,
}

/// The [`Weight`] of one conversation under `caps`.
fn weigh(conversation: &Conversation, caps: &Caps) -> Weight {
    let canonical = canonical(conversation);
    Weight {
        charge: charge(conversation),
        alone: observations(&canonical) > caps.per_conversation || turns(&canonical) > caps.turns,
    }
}

/// Canonical admission (plan §7) over weights in `(adapter, id)` order:
/// the longest prefix whose charges, each plus the bytes its holder keeps
/// beside it, fit `budget` is admitted, the rest refused whole; a
/// conversation over the observation or turn cap is refused alone.
fn prefix(weights: impl IntoIterator<Item = (Weight, usize)>, budget: usize) -> Vec<bool> {
    let mut used = 0_usize;
    let mut full = false;
    weights
        .into_iter()
        .map(|(weight, extra)| {
            if weight.alone || full {
                return false;
            }
            match used.checked_add(weight.charge.saturating_add(extra)) {
                Some(next) if next <= budget => {
                    used = next;
                    true
                }
                _ => {
                    full = true;
                    false
                }
            }
        })
        .collect()
}

/// The fold's admission: [`prefix`] over the merged conversations with no
/// bytes beside them. The budget is the ledger less the fixed
/// [`VIEW_RESERVE`] (plan §7), which also holds the fold-wide tables'
/// [`fold_floor`].
fn admit(merged: &[Cow<'_, Conversation>], caps: &Caps) -> Vec<bool> {
    prefix(
        merged
            .iter()
            .map(|conversation| (weigh(conversation, caps), 0)),
        caps.ledger.saturating_sub(VIEW_RESERVE),
    )
}

fn in_scope(conversation: &Conversation, scope: &Scope) -> bool {
    match scope {
        Scope::All => true,
        Scope::This(None) => false,
        Scope::This(Some(session)) => conversation
            .memberships
            .iter()
            .any(|membership| membership.session.as_deref() == Some(session.as_str())),
    }
}

pub(super) fn empty_cols(page: Page) -> Cols {
    match page {
        Page::Skills => Cols::Skill(SkillCols::default()),
        Page::Mcp => Cols::Mcp(McpCols::default()),
        Page::Tools => Cols::Tool(ToolCols::default()),
    }
}

fn add(into: &mut Cols, from: &Cols) {
    match (into, from) {
        (Cols::Skill(a), Cols::Skill(b)) => {
            a.attempts += b.attempts;
            a.loaded += b.loaded;
            a.not_loaded += b.not_loaded;
            a.unknown += b.unknown;
            a.unresolved += b.unresolved;
            a.reads += b.reads;
        }
        (Cols::Mcp(a), Cols::Mcp(b)) => {
            a.attempts += b.attempts;
            a.returned += b.returned;
            a.success += b.success;
            a.error += b.error;
            a.unresolved += b.unresolved;
        }
        (Cols::Tool(a), Cols::Tool(b)) => {
            a.attempts += b.attempts;
            a.returned += b.returned;
            a.error += b.error;
            a.unresolved += b.unresolved;
        }
        _ => {}
    }
}

/// Coverage lines, summed per `(kind, conversation)`.
#[derive(Default)]
pub(super) struct Lines(BTreeMap<(GapKind, Option<String>), u64>);

impl Lines {
    pub(super) fn add(&mut self, kind: GapKind, conversation: Option<&str>, count: u64) {
        if count > 0 {
            *self
                .0
                .entry((kind, conversation.and_then(super::short_id)))
                .or_default() += count;
        }
    }

    pub(super) fn finish(self) -> Vec<GapLine> {
        self.0
            .into_iter()
            .map(|((kind, conversation), count)| GapLine {
                kind,
                conversation,
                count,
            })
            .collect()
    }
}

/// Fold `conversations` for `ask` under `caps`: a function of the admitted
/// set alone (any order, duplicates or split point give one view).
#[must_use]
pub fn fold(conversations: &[Conversation], ask: &Ask, caps: &Caps) -> View {
    let mut view = blank(ask);
    if ask.scope == Scope::This(None) {
        view.state = State::NoSessionId;
        return view;
    }
    let merged = merge(conversations);
    let admitted = admit(&merged, caps);
    let gathered: Vec<Option<Gathered<'_>>> = merged
        .iter()
        .zip(&admitted)
        .map(|(conversation, ok)| ok.then(|| gather(conversation)))
        .collect();
    let names = Names::over(gathered.iter().flatten(), caps.idents);
    let mut lines = Lines::default();
    let mut obs = Vec::new();
    for (conversation, one) in merged.iter().zip(&gathered) {
        if !in_scope(conversation, &ask.scope) {
            continue;
        }
        view.listed += 1;
        view.read += u64::from(conversation.read);
        cover(conversation, one.as_ref(), &mut lines);
        let Some(one) = one else { continue };
        if let Some((from, to)) = one.range {
            view.range = Some(
                view.range
                    .map_or((from, to), |(a, b)| (a.min(from), b.max(to))),
            );
        }
        for group in one.groups.values() {
            view.orphans += group.orphans();
        }
        one.observe(ask.page, &names, &mut obs, None);
    }
    lines.add(GapKind::IdentCap, None, names.dropped);
    view.gaps = lines.finish();
    if view.listed == 0 {
        view.state = State::NoSupported;
    }
    tables(&mut view, &obs, caps);
    view
}

/// The view of `ask` before anything is counted.
pub(super) fn blank(ask: &Ask) -> View {
    View {
        ask: ask.clone(),
        state: State::Numbers,
        rows: Vec::new(),
        more: None,
        items: 0,
        totals: empty_cols(ask.page),
        harnesses: Vec::new(),
        orphans: 0,
        detail: Vec::new(),
        detail_more: None,
        range: None,
        read: 0,
        listed: 0,
        recounting: 0,
        gaps: Vec::new(),
        filters: vec![Filter::All],
    }
}

/// One conversation's counts as the persisted model keeps them (#288
/// persistence P1): rows by raw name and no client, which the current
/// memberships give at combine time. `admitted` false is a refused
/// conversation: its coverage lines only, as the fold draws one it refuses.
pub(super) struct Counted {
    pub(super) rows: BTreeMap<(Page, Item, Cell, Cell), Cols>,
    pub(super) orphans: u64,
    pub(super) range: Option<(i64, i64)>,
    pub(super) gaps: BTreeMap<GapKind, u64>,
    /// Every name the evidence holds, per kind: the combine's identity cap
    /// is taken over them, as [`Names::over`] takes it over gathered evidence.
    pub(super) names: BTreeMap<Kind, BTreeSet<String>>,
    /// The Skills page's Reads only a proven directory attributes, by that
    /// directory and their raw (model, effort): not in `rows`, since the
    /// directory's row is chosen after the identity cap maps its names.
    pub(super) inferred: Inferred,
    /// Every row a proof names for each directory of `inferred`, by raw
    /// name: the combine takes the least as the cap maps them.
    pub(super) proofs: BTreeMap<Dir, BTreeSet<Item>>,
}

/// Inferred Reads by proven directory, model and effort.
pub(super) type Inferred = BTreeMap<(Dir, Cell, Cell), u64>;

/// The directories a counted, loaded, unconflicted Skill call of
/// `conversation` proves (Q2 b).
pub(super) fn proven_dirs(conversation: &Conversation) -> BTreeSet<Dir> {
    let one = gather(conversation);
    let names = Names::over(std::iter::once(&one), usize::MAX);
    one.proven(&names).into_keys().collect()
}

/// A conversation's observation and turn counts, which its caps refuse by.
pub(super) fn size(conversation: &Conversation) -> (usize, usize) {
    let canonical = canonical(conversation);
    (observations(&canonical), turns(&canonical))
}

/// Count one conversation alone (see [`Counted`]).
pub(super) fn count(conversation: &Conversation, admitted: bool) -> Counted {
    let mut lines = Lines::default();
    let mut out = Counted {
        rows: BTreeMap::new(),
        orphans: 0,
        range: None,
        gaps: BTreeMap::new(),
        names: BTreeMap::new(),
        inferred: BTreeMap::new(),
        proofs: BTreeMap::new(),
    };
    if admitted {
        let one = gather(conversation);
        cover(conversation, Some(&one), &mut lines);
        let names = Names::over(std::iter::once(&one), usize::MAX);
        let mut obs = Vec::new();
        for page in [Page::Skills, Page::Mcp, Page::Tools] {
            obs.clear();
            let inferred = (page == Page::Skills).then_some(&mut out.inferred);
            one.observe(page, &names, &mut obs, inferred);
            for seen in &obs {
                let key = (
                    page,
                    seen.item.clone(),
                    seen.model.clone(),
                    seen.effort.clone(),
                );
                add(
                    out.rows.entry(key).or_insert_with(|| empty_cols(page)),
                    &seen.cols,
                );
            }
        }
        let used: BTreeSet<Dir> = out.inferred.keys().map(|(dir, ..)| *dir).collect();
        for (dir, item) in one.proofs(&names) {
            if used.contains(&dir) {
                out.proofs.entry(dir).or_default().insert(item);
            }
        }
        out.orphans = one.groups.values().map(Group::orphans).sum();
        out.range = one.range;
        out.names = names.keep;
    } else {
        cover(conversation, None, &mut lines);
    }
    for ((kind, _), count) in lines.0 {
        *out.gaps.entry(kind).or_default() += count;
    }
    out
}

/// One conversation's coverage lines.
fn cover(conversation: &Conversation, one: Option<&Gathered<'_>>, lines: &mut Lines) {
    let id = Some(conversation.id.as_str());
    for gap in &conversation.gaps {
        lines.add(gap.clone(), id, 1);
    }
    // Copies of one stream hold the same lines: the largest copy counts.
    let mut malformed: BTreeMap<&StreamRole, u64> = BTreeMap::new();
    let mut over_cap: BTreeMap<&StreamRole, u64> = BTreeMap::new();
    for stream in &conversation.streams {
        let seen = malformed.entry(&stream.role).or_default();
        *seen = (*seen).max(stream.malformed);
        let seen = over_cap.entry(&stream.role).or_default();
        *seen = (*seen).max(stream.over_cap);
    }
    lines.add(GapKind::Malformed, id, malformed.values().sum());
    lines.add(GapKind::OverCap, id, over_cap.values().sum());
    let Some(one) = one else {
        lines.add(GapKind::Ledger, id, 1);
        return;
    };
    for group in one.groups.values() {
        lines.add(GapKind::Conflict, id, group.conflicts());
        lines.add(GapKind::ReadPathOverLimit, id, group.over_reads());
    }
    // Differing contexts for one Codex turn: once per turn key, however many
    // copies or streams hold them (plan §2, §8 U4).
    let contexts = one.contexts.values().filter(|set| set.len() > 1).count();
    lines.add(
        GapKind::Conflict,
        id,
        u64::try_from(contexts).unwrap_or(u64::MAX),
    );
}

type DetailKey = (Item, Adapter, Cell, Cell, Cell);

/// Filters, ranked rows and Detail from the page's observations.
pub(super) fn tables(view: &mut View, obs: &[Obs], caps: &Caps) {
    let mut filters = BTreeSet::from([Filter::All]);
    for one in obs {
        filters.insert(Filter::Harness(one.harness));
        filters.insert(Filter::Client(one.client.clone()));
        filters.insert(Filter::Model(one.model.clone()));
        filters.insert(Filter::Effort(one.effort.clone()));
    }
    view.filters = filters.into_iter().collect();
    let page = view.ask.page;
    let mut rows: BTreeMap<&Item, Cols> = BTreeMap::new();
    let mut split: BTreeMap<Adapter, (Cols, BTreeMap<&Item, Cols>)> = BTreeMap::new();
    let mut detail: BTreeMap<DetailKey, u64> = BTreeMap::new();
    for one in obs.iter().filter(|one| one.matches(&view.ask.filter)) {
        add(&mut view.totals, &one.cols);
        add(
            rows.entry(&one.item).or_insert_with(|| empty_cols(page)),
            &one.cols,
        );
        let (totals, own) = split
            .entry(one.harness)
            .or_insert_with(|| (empty_cols(page), BTreeMap::new()));
        add(totals, &one.cols);
        add(
            own.entry(&one.item).or_insert_with(|| empty_cols(page)),
            &one.cols,
        );
        let attempts = one.cols.attempts();
        if attempts > 0 {
            let key = (
                one.item.clone(),
                one.harness,
                one.client.clone(),
                one.model.clone(),
                one.effort.clone(),
            );
            *detail.entry(key).or_default() += attempts;
        }
    }
    view.items = u64::try_from(rows.len()).unwrap_or(u64::MAX);
    let mut ranked = rank_rows(rows);
    view.more = cut(&mut ranked, caps.rows, |row| row.cols.attempts());
    view.rows = ranked;
    // The map order is the harness order: Claude, then Codex.
    view.harnesses = split
        .into_iter()
        .map(|(harness, (totals, own))| {
            let items = u64::try_from(own.len()).unwrap_or(u64::MAX);
            let mut rows = rank_rows(own);
            let more = cut(&mut rows, caps.rows, |row| row.cols.attempts());
            HarnessGroup {
                harness,
                items,
                totals,
                rows,
                more,
            }
        })
        .collect();
    let mut ranked: Vec<(DetailKey, u64)> = detail.into_iter().collect();
    ranked.sort_by_key(|row| Reverse(row.1));
    view.detail_more = cut(&mut ranked, caps.detail, |row| row.1);
    view.detail = ranked
        .into_iter()
        .map(
            |((item, harness, client, model, effort), attempts)| DetailRow {
                item,
                harness,
                client,
                model,
                effort,
                attempts,
            },
        )
        .collect();
}

/// `rows` ranked: attempts descending. The map order (item ascending) breaks
/// the ties, because the sort is stable.
fn rank_rows(rows: BTreeMap<&Item, Cols>) -> Vec<Row> {
    let mut ranked: Vec<Row> = rows
        .into_iter()
        .map(|(item, cols)| Row {
            item: item.clone(),
            cols,
        })
        .collect();
    ranked.sort_by_key(|row| Reverse(row.cols.attempts()));
    ranked
}

/// Keep the first `cap` rows; say what the cut left out.
fn cut<T>(rows: &mut Vec<T>, cap: usize, attempts: impl Fn(&T) -> u64) -> Option<More> {
    if rows.len() <= cap {
        return None;
    }
    let rest = rows.split_off(cap);
    Some(More {
        items: u64::try_from(rest.len()).unwrap_or(u64::MAX),
        attempts: rest.iter().map(attempts).sum(),
    })
}

#[cfg(test)]
#[path = "fold_tests.rs"]
mod tests;
