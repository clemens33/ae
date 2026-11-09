//! Observed skill loads and tool / MCP calls (#288 slice 1): the pure fold.
//!
//! Pure: adapter events in, a [`View`] out, no IO and no clock. The adapters
//! (`usage::claude_calls`, `usage::codex_calls`) turn one native transcript
//! line into [`Event`]s; the reader (`usage::calls_read`) groups them per
//! [`Conversation`] and [`Stream`]; [`fold`] counts them for one [`Ask`].
//!
//! Evidence classes stay separate columns and are never summed: a Skill
//! attempt, its load outcome, a successful resource read, a tool attempt and
//! its returned result. Nothing here keeps prompt, argument or result text, or
//! a path: identities are normalized words ([`ident`]), native ids and
//! directories are digests ([`Key`], [`Dir`], [`Resource`]).

use crate::meta::RecordedClient;
use crate::stats::Cell;
use std::fmt::Write as _;

pub mod doc;
mod fold;
pub mod incr;
pub mod store;
pub use fold::fold;

/// The longest normalized identity component, in bytes. A joined Codex display
/// name `namespace/name` holds two components and may reach 129 bytes.
pub const IDENT_MAX: usize = 64;
/// The longest `Read` path resolved against skill directories, in bytes. No
/// supported platform opens a longer path (Linux `PATH_MAX`; macOS is 1024),
/// so a successful read never exceeds it; a longer one is a diagnostic.
pub const READ_PATH_MAX: usize = 4096;
/// The prefix of the first text line of an injected skill body (Q1 b).
pub const BASE_PREFIX: &str = "Base directory for this skill: ";
/// The ledger bytes held back for two published views (plan §7, lead ruling
/// 2026-11-08): a fixed 2 x 4 MiB, taken off [`Caps::ledger`] before
/// admission, never a measured view size, so admission stays a function of
/// the admitted set.
pub const VIEW_RESERVE: usize = 2 * (4 << 20);

/// A digest of one native id (call, turn or message id): SHA-256, first 16
/// bytes, over a domain word and the id. Equal ids give equal keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(pub [u8; 16]);

impl Key {
    /// The key of native id `id` in `domain` (`b"call"`, `b"turn"`, `b"message"`).
    #[must_use]
    pub fn of(domain: &[u8], id: &[u8]) -> Self {
        Self(digest16(&[domain, b"\0", id]))
    }
}

/// A digest of one canonical directory ([`canonical_dir`]), class-free: the
/// join between a loaded Skill's base directory and a `Read`'s ancestors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dir(pub [u8; 16]);

impl Dir {
    /// The digest of `canonical` read through `adapter`.
    #[must_use]
    pub fn of(adapter: Adapter, canonical: &str) -> Self {
        Self(digest16(&[
            b"dir\0",
            adapter.word().as_bytes(),
            b"\0",
            canonical.as_bytes(),
        ]))
    }
}

/// The opaque resource id of a skill directory: 12 hex digits of SHA-256 over
/// `(adapter, scope class, canonical dir)`. The path itself is never kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Resource(pub [u8; 6]);

impl Resource {
    /// The resource id of `canonical` in `class`, read through `adapter`.
    #[must_use]
    pub fn of(adapter: Adapter, class: ScopeClass, canonical: &str) -> Self {
        let full = digest16(&[
            b"resource\0",
            adapter.word().as_bytes(),
            b"\0",
            class.word().as_bytes(),
            b"\0",
            canonical.as_bytes(),
        ]);
        let mut out = [0_u8; 6];
        out.copy_from_slice(&full[..6]);
        Self(out)
    }

    /// The 12 lowercase hex digits a table shows.
    #[must_use]
    pub fn hex(self) -> String {
        let mut out = String::with_capacity(12);
        for byte in self.0 {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }
}

fn digest16(parts: &[&[u8]]) -> [u8; 16] {
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    for part in parts {
        context.update(part);
    }
    let done = context.finish();
    let mut out = [0_u8; 16];
    out.copy_from_slice(&done.as_ref()[..16]);
    out
}

/// The lexical canonical form of an absolute directory or file path: `.` and
/// empty components dropped, no trailing `/`. `None` for a relative path, a
/// `..` component, a NUL, or a path over [`READ_PATH_MAX`] bytes. Never asks
/// the file system.
#[must_use]
pub fn canonical_dir(path: &str) -> Option<String> {
    if path.len() > READ_PATH_MAX || path.contains('\0') || !path.starts_with('/') {
        return None;
    }
    let mut out = String::with_capacity(path.len());
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part => {
                out.push('/');
                out.push_str(part);
            }
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    Some(out)
}

/// Whether canonical `path` lies strictly inside canonical `dir`.
#[must_use]
pub fn inside(path: &str, dir: &str) -> bool {
    if dir == "/" {
        return path.len() > 1 && path.starts_with('/');
    }
    path.strip_prefix(dir)
        .is_some_and(|rest| rest.len() > 1 && rest.starts_with('/'))
}

/// Every ancestor directory of canonical `path`, nearest first, root excluded.
#[must_use]
pub fn ancestors(path: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut at = path;
    while let Some(cut) = at.rfind('/') {
        at = &at[..cut];
        if at.is_empty() {
            break;
        }
        out.push(at);
    }
    out
}

/// What an identity names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Skill,
    Server,
    Tool,
    Model,
    Effort,
}

/// A normalized identity. Each component has bytes `[A-Za-z0-9_.:+-]`,
/// 1..=[`IDENT_MAX`] bytes, and does not start with `.`; anything else is
/// unresolved and none of it is kept. A plain name is one component. A Codex
/// tool with a namespace joins two valid components with exactly one `/`, so
/// that display name may reach 129 bytes and contains exactly one `/`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ident {
    Name(String),
    Unresolved(Kind),
}

/// THE identity normalizer.
#[must_use]
pub fn ident(kind: Kind, raw: &str) -> Ident {
    let bytes = raw.as_bytes();
    let allowed = |byte: &u8| byte.is_ascii_alphanumeric() || b"_.:+-".contains(byte);
    if bytes.is_empty()
        || bytes.len() > IDENT_MAX
        || bytes.first() == Some(&b'.')
        || !bytes.iter().all(allowed)
    {
        return Ident::Unresolved(kind);
    }
    Ident::Name(raw.to_owned())
}

/// Where a skill's base directory sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScopeClass {
    /// Under `<config home>/skills/`.
    User,
    /// Under `<seat work dir>/.claude/skills/`.
    Project,
    /// Under `<config home>/plugins/`.
    Plugin,
    Other,
}

impl ScopeClass {
    /// The word a table and the resource digest use.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Plugin => "plugin",
            Self::Other => "other",
        }
    }
}

/// Which native format a conversation is read through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Adapter {
    Claude,
    Codex,
}

impl Adapter {
    /// The harness word a table shows.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

/// A Claude Skill result's load outcome (plan §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Load {
    Loaded,
    NotLoaded,
    Unknown,
}

/// A Codex `McpToolCall` item's outcome (plan §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum McpOutcome {
    Success,
    Error,
    Returned,
    /// `failed` with `isError: false`: outcome unknown, a diagnostic.
    Conflict,
}

/// One observation an adapter takes from one native line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Event {
    /// A record's native timestamp, epoch microseconds: the covered range.
    Stamp(i64),
    /// A top-level native tool call. `model` is the generating record's
    /// (Claude); `turn` its turn (Codex); `message` the Claude `message.id`
    /// (the sidechain replay rule); `mcp` the (server, tool) split of a raw
    /// Claude name starting `mcp__`: exactly one `__` after the prefix, no
    /// `___`, both parts valid identities, else both unresolved; `None` for
    /// any other name and for Codex.
    Attempt {
        id: Key,
        tool: Ident,
        mcp: Option<(Ident, Ident)>,
        model: Option<Ident>,
        turn: Option<Key>,
        message: Option<Key>,
    },
    /// A Claude `Skill` call; the same call is ALSO an [`Event::Attempt`].
    Skill { id: Key, skill: Ident },
    /// Q1 b: the injected body linked to Skill call `id` by `sourceToolUseID`
    /// starts with [`BASE_PREFIX`]; its directory, classified and digested.
    SkillBase {
        id: Key,
        class: ScopeClass,
        resource: Resource,
        dir: Dir,
    },
    /// A matched result of call `id`. `load` is Claude's load outcome for the
    /// call were it a Skill (the fold reads it only for one): `is_error` true →
    /// not loaded; else, in a record with exactly one `tool_result`, root
    /// `toolUseResult.success` true → loaded, false → not loaded, absent or not
    /// a bool → unknown; in a record with more than one → unknown. `None` for
    /// Codex.
    Result {
        id: Key,
        error: bool,
        load: Option<Load>,
    },
    /// Q2 b: a native `Read` of an absolute `..`-free path. `direct` when the
    /// path is exactly `<root>/skills/<d>/SKILL.md` for a seat root; every
    /// ancestor directory digested, nearest first; `over` for a path over
    /// [`READ_PATH_MAX`] (no ancestors, a diagnostic). `skills` when a
    /// component of the canonical ancestor directories is exactly `skills`:
    /// the fold ignores it, only the persisted O2 fallback reads it.
    Read {
        id: Key,
        direct: Option<(Ident, ScopeClass, Resource)>,
        ancestors: Vec<Dir>,
        over: bool,
        skills: bool,
    },
    /// A Codex `item_completed` `McpToolCall`.
    Mcp {
        id: Key,
        server: Ident,
        tool: Ident,
        outcome: McpOutcome,
        turn: Option<Key>,
    },
    /// A Codex `turn_context`.
    Context {
        turn: Key,
        model: Ident,
        effort: Ident,
    },
}

/// One line's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parsed {
    /// Read; zero or more events kept.
    Kept,
    /// A record the adapter does not read (another type, a summary, `isMeta`).
    Ignored,
    /// Not JSON, or a read record with a wrong type or a missing key.
    Malformed,
}

/// How a conversation belongs to an ae session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// A live seat's current conversation.
    Live,
    /// A live seat's `harness_session_prior`.
    Prior,
    /// A `retire` record's conversation.
    Retired,
    /// A kept archive's `harness.tsv` current or prior.
    Archived,
}

/// One membership of a conversation; a conversation keeps every one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    /// The session's durable id (`session_id`), when known.
    pub session: Option<String>,
    pub slot: String,
    pub role: Role,
    pub client: RecordedClient,
}

/// Which file of a conversation a stream is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StreamRole {
    Parent,
    /// A Claude `subagents/` file, by its file name.
    Child(String),
}

/// One file's observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub role: StreamRole,
    /// Which located copy (store) this file is: copies of one stream alias.
    pub copy: u32,
    pub events: Vec<Event>,
    pub malformed: u64,
    pub over_cap: u64,
}

/// Why a conversation's numbers are partial, or a diagnostic about them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GapKind {
    Unlocated,
    Unreadable,
    Changed,
    NotUnique,
    Malformed,
    OverCap,
    Conflict,
    /// A seat of another harness: `{tool}: {c} conversations not read`.
    Unsupported(String),
    ReadPathOverLimit,
    /// The ledger refused the conversation whole.
    Ledger,
    /// An identity table full: `unresolved (cap)`.
    IdentCap,
    /// Session metas named memberships whose session id, slot or client
    /// label passed the reader's field limit; they were not kept.
    FieldOverLimit,
    /// O2: unproven non-skills Reads dropped to fit the conversation cap.
    UnprovenReadsDropped,
    /// P6: counted sources no longer on disk; their counts are kept.
    SourceGone,
    /// Conversations left out of the published view by its cap.
    ViewOverCap,
    /// P6 boundary: the conversation was rebuilt whole from its sources
    /// (its document damaged, oversized or of another format, or its held
    /// counts damaged); counts of its deleted sources are lost. Sticky
    /// until `r`.
    RebuiltFromSource,
}

/// One coverage line: `conversation` is a short conversation id, never a path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GapLine {
    pub kind: GapKind,
    pub conversation: Option<String>,
    pub count: u64,
}

impl GapLine {
    /// The line the page draws: a count and, for one conversation, its
    /// short id. Never a path, and no harness word that is not an identity.
    #[must_use]
    pub fn line(&self) -> String {
        let count = self.count;
        let mut out = match &self.kind {
            GapKind::Unlocated => format!("unlocated {count}"),
            GapKind::Unreadable => format!("unreadable {count}"),
            GapKind::Changed => format!("changed during read {count}"),
            GapKind::NotUnique => format!("id not unique {count}"),
            GapKind::Malformed => format!("malformed {count}"),
            GapKind::OverCap => format!("line over cap {count}"),
            GapKind::Conflict => format!("conflict {count}"),
            GapKind::Unsupported(tool) => {
                let tool = match ident(Kind::Tool, tool) {
                    Ident::Name(tool) => tool,
                    Ident::Unresolved(_) => "unresolved".to_owned(),
                };
                format!("unsupported harness ({tool}: {count} conversations not read)")
            }
            GapKind::ReadPathOverLimit => format!("read path over limit {count}"),
            GapKind::Ledger => format!("over cap, not counted {count}"),
            GapKind::IdentCap => format!("unresolved (cap) {count}"),
            GapKind::FieldOverLimit => format!("membership over field limit {count}"),
            GapKind::UnprovenReadsDropped => format!("unproven reads dropped {count}"),
            GapKind::SourceGone => format!("source gone {count}"),
            GapKind::ViewOverCap => format!("view over cap {count}"),
            GapKind::RebuiltFromSource => format!(
                "rebuilt from source: {count} conversations (counts of deleted sources there are lost)"
            ),
        };
        if let Some(id) = self.conversation.as_deref().and_then(short_id) {
            let _ = write!(out, " · conversation {id}");
        }
        out
    }
}

/// A conversation's short id for a coverage line: its last 8 bytes when
/// they are ASCII letters or digits (a UUID's random tail), else none.
pub(crate) fn short_id(id: &str) -> Option<String> {
    let tail = id.get(id.len().saturating_sub(8)..)?;
    (!tail.is_empty() && tail.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then(|| tail.to_owned())
}

/// One ae-associated conversation as the reader holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub adapter: Adapter,
    pub id: String,
    pub memberships: Vec<Membership>,
    /// Plan §5 (a): every member session's meta read and journal scanned to
    /// EOF with no gap, every listing archive read.
    pub evidence_complete: bool,
    /// Plan §5 (c): a `reseat` record names the id.
    pub reseated: bool,
    pub streams: Vec<Stream>,
    pub gaps: Vec<GapKind>,
    /// Every file read to its current end.
    pub read: bool,
}

/// The client cell of a conversation: a label only on proof (plan §5).
#[must_use]
pub fn client(conversation: &Conversation) -> Cell {
    if !conversation.evidence_complete || conversation.reseated {
        return Cell::Unknown;
    }
    let mut label: Option<&str> = None;
    for membership in &conversation.memberships {
        let (Role::Live, RecordedClient::Label(found)) = (membership.role, &membership.client)
        else {
            return Cell::Unknown;
        };
        match label {
            Some(seen) if seen != found => return Cell::Unknown,
            _ => label = Some(found),
        }
    }
    label.map_or(Cell::Unknown, |label| Cell::Value(label.to_owned()))
}

/// Which table a page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Page {
    #[default]
    Skills,
    Mcp,
    Tools,
}

/// Which observations a view counts.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Every admitted observation once.
    All,
    /// Observations with a stream whose conversation has a membership in
    /// session `uuid`; `None`: the overlay's session has no id (U6).
    This(Option<String>),
}

/// The one active filter: `all`, or one dimension's value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Filter {
    #[default]
    All,
    Harness(Adapter),
    Client(Cell),
    Model(Cell),
    Effort(Cell),
}

/// What the loader folds for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub page: Page,
    pub scope: Scope,
    pub filter: Filter,
}

/// Every retained-memory and table bound (plan §7, §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Bytes the ledger may retain.
    pub ledger: usize,
    /// Rows of a main table.
    pub rows: usize,
    /// Rows of Detail.
    pub detail: usize,
    /// Observations of one conversation; more refuses it.
    pub per_conversation: usize,
    /// Distinct identities per [`Kind`].
    pub idents: usize,
    /// Codex turn contexts per conversation.
    pub turns: usize,
    /// Bytes of one rendered conversation document.
    pub conv_cap: u64,
    /// Bytes of the rendered view document.
    pub view_cap: u64,
    /// Bytes of the rendered coverage document.
    pub coverage_cap: u64,
}

impl Caps {
    /// The product's bounds.
    pub const SLICE: Self = Self {
        ledger: 64 << 20,
        rows: 256,
        detail: 4096,
        per_conversation: 131_072,
        idents: 8192,
        turns: 4096,
        conv_cap: doc::CONV_CAP,
        view_cap: doc::VIEW_CAP,
        coverage_cap: doc::COVERAGE_CAP,
    };
}

/// A table's item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    /// A skill and its proven resource; `None` scope: unresolved.
    Skill {
        name: Ident,
        scope: Option<(ScopeClass, Resource)>,
    },
    Mcp {
        server: Ident,
        tool: Ident,
    },
    Tool(Ident),
}

/// A row's numbers, one per column of its page (plan §8, plus Q2 b reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SkillCols {
    pub attempts: u64,
    pub loaded: u64,
    pub not_loaded: u64,
    /// Returned, load status unknown.
    pub unknown: u64,
    /// No result.
    pub unresolved: u64,
    /// Successful resource reads (Q2 b), not use of the instructions.
    pub reads: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct McpCols {
    pub attempts: u64,
    pub returned: u64,
    pub success: u64,
    pub error: u64,
    pub unresolved: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToolCols {
    pub attempts: u64,
    pub returned: u64,
    pub error: u64,
    pub unresolved: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cols {
    Skill(SkillCols),
    Mcp(McpCols),
    Tool(ToolCols),
}

impl Cols {
    /// The attempts column, the rank key.
    #[must_use]
    pub const fn attempts(&self) -> u64 {
        match self {
            Self::Skill(cols) => cols.attempts,
            Self::Mcp(cols) => cols.attempts,
            Self::Tool(cols) => cols.attempts,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub item: Item,
    pub cols: Cols,
}

/// One Detail row: the joint breakdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRow {
    pub item: Item,
    pub harness: Adapter,
    pub client: Cell,
    pub model: Cell,
    pub effort: Cell,
    pub attempts: u64,
}

/// What a truncated table leaves out: `+{items} more items, {attempts} attempts not shown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct More {
    pub items: u64,
    pub attempts: u64,
}

/// What a page says beside (or instead of) its numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Numbers (U2, U3, U4 by `read`/`listed`/`gaps`).
    Numbers,
    /// U5: no supported ae-associated conversation.
    NoSupported,
    /// U6: This session, and the session has no id.
    NoSessionId,
}

/// One folded page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub ask: Ask,
    pub state: State,
    /// Ranked attempts desc, then item asc; at most `Caps::rows`.
    pub rows: Vec<Row>,
    pub more: Option<More>,
    /// Over every item, shown or not.
    pub items: u64,
    pub totals: Cols,
    /// Results with no attempt in their stream (Tools page line).
    pub orphans: u64,
    /// At most `Caps::detail` rows, same ranking.
    pub detail: Vec<DetailRow>,
    pub detail_more: Option<More>,
    /// Earliest and latest native record timestamp read in covered streams.
    pub range: Option<(i64, i64)>,
    /// Conversations read to their end, and listed.
    pub read: u64,
    pub listed: u64,
    /// Changed files being read again this round.
    pub recounting: u64,
    pub gaps: Vec<GapLine>,
    /// The `]`/`[` cycle for the shown scope: `All`, then every harness,
    /// client, model and effort value present, in that order, values sorted.
    pub filters: Vec<Filter>,
}

impl View {
    /// Whether the page must say `Partial`.
    #[must_use]
    pub fn partial(&self) -> bool {
        self.read < self.listed || self.gaps.iter().any(|gap| gap.count > 0)
    }
}

#[cfg(test)]
#[path = "tool_stats_spec.rs"]
mod spec;

#[cfg(test)]
#[path = "tool_stats_budget_spec.rs"]
mod budget_spec;

#[cfg(test)]
#[path = "tool_stats/persist_spec.rs"]
mod persist_spec;
