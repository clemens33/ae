//! #288 persistence (P2): the versioned documents under `stats/tools/`,
//! pure. No IO; [`super::store`] decides which file holds which document.
//!
//! Every document is one JSON record a line. Its head names the document
//! and [`TOOLS_FORMAT`]; its last record `{"end":N}` counts the records
//! before it, so a file cut short at a line boundary reads as damaged. A
//! document is read only in its canonical form: a parse renders what it
//! decoded and refuses anything that does not render to the same bytes, so
//! a successful parse re-renders exactly and no hand edit is half-trusted.
//!
//! The documents hold identities ([`Ident`] names, conversation and session
//! ids, slots, client labels, child stream file names), digests ([`Key`],
//! [`Dir`], [`Resource`], store roots, witness blocks), counts and enums:
//! never a prompt, an argument, a result, a path or a native call id.

use super::incr::{
    self, Association, Conv, FileState, Memberships, Published, PublishedConv, Refused, Source,
    StreamKey, Summary,
};
use super::{
    Adapter, Cols, Dir, Event, GapKind, Ident, Item, Key, Kind, Load, McpCols, McpOutcome,
    Membership, Page, Resource, Role, ScopeClass, SkillCols, StreamRole, ToolCols,
};
use crate::json::{self, Value};
use crate::meta::RecordedClient;
use crate::stats::Cell;
use std::collections::{BTreeMap, BTreeSet};

/// The value of each document's head version key.
pub const TOOLS_FORMAT: i64 = 1;
/// The chosen cap of one conversation document.
pub const CONV_CAP: u64 = 16 << 20;
/// The cap of the derived view document.
pub const VIEW_CAP: u64 = 8 << 20;
/// The cap of the association document.
pub const ASSOC_CAP: u64 = 8 << 20;
/// The cap of the publication intent.
pub const INTENT_CAP: u64 = 4096;

/// Why a document is not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocError {
    /// The head names another format.
    Format(i64),
    Damaged(&'static str),
    Oversized,
}

type Fields = Vec<(String, Value)>;
type Parsed<T> = Result<T, DocError>;

/// The persisted association: source statuses and memberships.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assoc {
    pub sources: BTreeMap<String, FileState>,
    pub memberships: Memberships,
}

/// An interrupted publication's record: the conversation document it
/// commits and the view generation it makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub conv: String,
    pub generation: u64,
}

// --- writing ---------------------------------------------------------------

/// A document being written: records, then [`Out::close`].
struct Out {
    text: String,
    records: u64,
}

impl Out {
    fn head<'a>(key: &'a str, mut fields: Vec<(&'a str, Value)>) -> Self {
        let mut out = Self {
            text: String::new(),
            records: 0,
        };
        fields.insert(0, (key, Value::Num(TOOLS_FORMAT)));
        out.record(fields);
        out
    }

    fn record(&mut self, fields: Vec<(&str, Value)>) {
        Value::obj(fields).render_into(&mut self.text);
        self.text.push('\n');
        self.records += 1;
    }

    fn close(mut self) -> Vec<u8> {
        let end = number(self.records);
        self.record(vec![("end", end)]);
        self.text.into_bytes()
    }
}

fn number(value: u64) -> Value {
    i64::try_from(value).map_or_else(|_| Value::Raw(value.to_string()), Value::Num)
}

fn signed(value: i64) -> Value {
    Value::Num(value)
}

fn hex(bytes: &[u8]) -> Value {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        out.push(char::from(b"0123456789abcdef"[usize::from(byte & 15)]));
    }
    Value::Str(out)
}

fn optional<T>(value: Option<T>, of: impl FnOnce(T) -> Value) -> Value {
    value.map_or(Value::Null, of)
}

const fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Skill => "skill",
        Kind::Server => "server",
        Kind::Tool => "tool",
        Kind::Model => "model",
        Kind::Effort => "effort",
    }
}

fn ident(ident: &Ident) -> Value {
    match ident {
        Ident::Name(name) => Value::str(name.as_str()),
        Ident::Unresolved(kind) => Value::Arr(vec![Value::str(kind_word(*kind))]),
    }
}

fn cell(cell: &Cell) -> Value {
    match cell {
        Cell::Value(value) => Value::str(value.as_str()),
        Cell::Unknown => Value::Null,
        Cell::NoneSet => Value::Arr(vec![Value::str("none_set")]),
        Cell::Direct => Value::Arr(vec![Value::str("direct")]),
        Cell::Unproved => Value::Arr(vec![Value::str("unproved")]),
    }
}

fn stream(key: &StreamKey) -> Value {
    let role = match &key.role {
        StreamRole::Parent => Value::Null,
        StreamRole::Child(name) => Value::str(name.as_str()),
    };
    Value::Arr(vec![role, hex(&key.copy)])
}

const fn adapter_word(adapter: Adapter) -> &'static str {
    adapter.word()
}

const fn page_word(page: Page) -> &'static str {
    match page {
        Page::Skills => "skills",
        Page::Mcp => "mcp",
        Page::Tools => "tools",
    }
}

const fn refused_word(refused: Refused) -> &'static str {
    match refused {
        Refused::Observations => "observations",
        Refused::Turns => "turns",
        Refused::Size => "size",
    }
}

const fn load_word(load: Load) -> &'static str {
    match load {
        Load::Loaded => "loaded",
        Load::NotLoaded => "not_loaded",
        Load::Unknown => "unknown",
    }
}

const fn outcome_word(outcome: McpOutcome) -> &'static str {
    match outcome {
        McpOutcome::Success => "success",
        McpOutcome::Error => "error",
        McpOutcome::Returned => "returned",
        McpOutcome::Conflict => "conflict",
    }
}

const fn role_word(role: Role) -> &'static str {
    match role {
        Role::Live => "live",
        Role::Prior => "prior",
        Role::Retired => "retired",
        Role::Archived => "archived",
    }
}

fn gap(kind: &GapKind) -> Value {
    let word = match kind {
        GapKind::Unlocated => "unlocated",
        GapKind::Unreadable => "unreadable",
        GapKind::Changed => "changed",
        GapKind::NotUnique => "not_unique",
        GapKind::Malformed => "malformed",
        GapKind::OverCap => "over_cap",
        GapKind::Conflict => "conflict",
        GapKind::Unsupported(tool) => {
            return Value::Arr(vec![Value::str("unsupported"), Value::str(tool.as_str())]);
        }
        GapKind::ReadPathOverLimit => "read_path_over_limit",
        GapKind::Ledger => "ledger",
        GapKind::IdentCap => "ident_cap",
        GapKind::FieldOverLimit => "field_over_limit",
        GapKind::UnprovenReadsDropped => "unproven_reads_dropped",
        GapKind::SourceGone => "source_gone",
        GapKind::ViewOverCap => "view_over_cap",
    };
    Value::str(word)
}

fn item(item: &Item) -> Value {
    match item {
        Item::Tool(tool) => Value::Arr(vec![Value::str("tool"), ident(tool)]),
        Item::Mcp { server, tool } => {
            Value::Arr(vec![Value::str("mcp"), ident(server), ident(tool)])
        }
        Item::Skill { name, scope } => Value::Arr(vec![
            Value::str("skill"),
            ident(name),
            optional(scope.as_ref(), |(class, resource)| {
                Value::Arr(vec![Value::str(class.word()), hex(&resource.0)])
            }),
        ]),
    }
}

fn cols(cols: &Cols) -> Value {
    Value::Arr(incr::numbers(cols).into_iter().map(number).collect())
}

fn stamp((seconds, nanos): (i64, u32)) -> Value {
    Value::Arr(vec![signed(seconds), number(u64::from(nanos))])
}

fn status_fields<'a>(key: &'a str, name: Value, state: &FileState) -> Vec<(&'a str, Value)> {
    vec![
        (key, name),
        ("dev", number(state.dev)),
        ("ino", number(state.ino)),
        ("len", number(state.len)),
        ("mtime", stamp(state.mtime)),
        ("ctime", stamp(state.ctime)),
        ("cursor", number(state.cursor)),
        ("skipping", Value::Bool(state.skipping)),
        (
            "blocks",
            Value::Arr(state.blocks.iter().map(|block| hex(block)).collect()),
        ),
    ]
}

fn event(out: &mut Out, event: &Event) {
    let fields = match event {
        Event::Stamp(at) => vec![("stamp", signed(*at))],
        Event::Attempt {
            id,
            tool,
            mcp,
            model,
            turn,
            message,
        } => vec![
            ("attempt", hex(&id.0)),
            ("tool", ident(tool)),
            (
                "mcp",
                optional(mcp.as_ref(), |(server, tool)| {
                    Value::Arr(vec![ident(server), ident(tool)])
                }),
            ),
            ("model", optional(model.as_ref(), ident)),
            ("turn", optional(turn.as_ref(), |key| hex(&key.0))),
            ("message", optional(message.as_ref(), |key| hex(&key.0))),
        ],
        Event::Skill { id, skill } => vec![("skill", hex(&id.0)), ("name", ident(skill))],
        Event::SkillBase {
            id,
            class,
            resource,
            dir,
        } => vec![
            ("base", hex(&id.0)),
            ("class", Value::str(class.word())),
            ("resource", hex(&resource.0)),
            ("dir", hex(&dir.0)),
        ],
        Event::Result { id, error, load } => vec![
            ("result", hex(&id.0)),
            ("error", Value::Bool(*error)),
            ("load", optional(*load, |load| Value::str(load_word(load)))),
        ],
        Event::Read {
            id,
            direct,
            ancestors,
            over,
            skills,
        } => vec![
            ("read", hex(&id.0)),
            (
                "direct",
                optional(direct.as_ref(), |(name, class, resource)| {
                    Value::Arr(vec![
                        ident(name),
                        Value::str(class.word()),
                        hex(&resource.0),
                    ])
                }),
            ),
            (
                "ancestors",
                Value::Arr(ancestors.iter().map(|dir| hex(&dir.0)).collect()),
            ),
            ("over", Value::Bool(*over)),
            ("skills", Value::Bool(*skills)),
        ],
        Event::Mcp {
            id,
            server,
            tool,
            outcome,
            turn,
        } => vec![
            ("mcp_call", hex(&id.0)),
            ("server", ident(server)),
            ("tool", ident(tool)),
            ("outcome", Value::str(outcome_word(*outcome))),
            ("turn", optional(turn.as_ref(), |key| hex(&key.0))),
        ],
        Event::Context {
            turn,
            model,
            effort,
        } => vec![
            ("context", hex(&turn.0)),
            ("model", ident(model)),
            ("effort", ident(effort)),
        ],
    };
    out.record(fields);
}

fn files(out: &mut Out, files: &BTreeMap<StreamKey, Source>) {
    for (key, source) in files {
        out.record(vec![
            ("file", stream(key)),
            ("gone", Value::Bool(*source == Source::Gone)),
        ]);
    }
}

/// One conversation's authoritative document: its resume state and the
/// facts the reader sets; the counters are derived from them on parse.
#[must_use]
pub fn render_conv(conv: &Conv) -> Vec<u8> {
    let summary = &conv.summary;
    let resume = &conv.resume;
    let mut out = Out::head(
        "tools_conv",
        vec![
            ("adapter", Value::str(adapter_word(summary.adapter))),
            ("id", Value::str(summary.id.as_str())),
            ("read", Value::Bool(summary.read)),
            (
                "refused",
                optional(summary.refused, |refused| Value::str(refused_word(refused))),
            ),
            ("fallback", Value::Bool(resume.fallback)),
        ],
    );
    files(&mut out, &summary.files);
    for (key, state) in &resume.files {
        out.record(status_fields("status", stream(key), state));
    }
    for (key, (malformed, over_cap)) in &resume.lines {
        out.record(vec![
            ("lines", stream(key)),
            ("malformed", number(*malformed)),
            ("over_cap", number(*over_cap)),
        ]);
    }
    for kind in &resume.marks {
        out.record(vec![("mark", gap(kind))]);
    }
    for key in &resume.dropped {
        out.record(vec![("dropped", hex(&key.0))]);
    }
    for (key, events) in &resume.streams {
        out.record(vec![("stream", stream(key))]);
        for one in events {
            event(&mut out, one);
        }
    }
    out.close()
}

/// The derived view: every published conversation's counters and scope.
#[must_use]
pub fn render_view(view: &Published) -> Vec<u8> {
    let mut out = Out::head(
        "tools_view",
        vec![
            ("generation", number(view.generation)),
            ("over_cap", number(view.over_cap)),
        ],
    );
    for conv in &view.convs {
        let summary = &conv.summary;
        out.record(vec![
            ("conv", Value::str(adapter_word(summary.adapter))),
            ("id", Value::str(summary.id.as_str())),
            ("read", Value::Bool(summary.read)),
            (
                "refused",
                optional(summary.refused, |refused| Value::str(refused_word(refused))),
            ),
            ("orphans", number(summary.orphans)),
            (
                "range",
                optional(summary.range, |(from, to)| {
                    Value::Arr(vec![signed(from), signed(to)])
                }),
            ),
            (
                "observations",
                number(u64::try_from(summary.observations).unwrap_or(u64::MAX)),
            ),
            (
                "turns",
                number(u64::try_from(summary.turns).unwrap_or(u64::MAX)),
            ),
            ("client", cell(&conv.client)),
        ]);
        for session in &conv.sessions {
            out.record(vec![("session", Value::str(session.as_str()))]);
        }
        files(&mut out, &summary.files);
        for (kind, count) in &summary.gaps {
            out.record(vec![("gap", gap(kind)), ("count", number(*count))]);
        }
        for (kind, names) in &summary.names {
            for name in names {
                out.record(vec![
                    ("name", Value::str(kind_word(*kind))),
                    ("value", Value::str(name.as_str())),
                ]);
            }
        }
        for ((page, row, harness, model, effort), counts) in &summary.rows {
            out.record(vec![
                ("row", Value::str(page_word(*page))),
                ("item", item(row)),
                ("harness", Value::str(adapter_word(*harness))),
                ("model", cell(model)),
                ("effort", cell(effort)),
                ("cols", cols(counts)),
            ]);
        }
        for (dir, items) in &summary.proofs {
            for one in items {
                out.record(vec![("proof", hex(&dir.0)), ("item", item(one))]);
            }
        }
        for ((dir, model, effort), reads) in &summary.inferred {
            out.record(vec![
                ("inferred", hex(&dir.0)),
                ("model", cell(model)),
                ("effort", cell(effort)),
                ("reads", number(*reads)),
            ]);
        }
    }
    out.close()
}

/// The association: session sources and every conversation's memberships.
#[must_use]
pub fn render_assoc(assoc: &Assoc) -> Vec<u8> {
    let mut out = Out::head("tools_assoc", Vec::new());
    for (name, state) in &assoc.sources {
        out.record(status_fields("source", Value::str(name.as_str()), state));
    }
    for ((adapter, id), association) in &assoc.memberships {
        out.record(vec![
            ("conv", Value::str(adapter_word(*adapter))),
            ("id", Value::str(id.as_str())),
            ("complete", Value::Bool(association.evidence_complete)),
            ("reseated", Value::Bool(association.reseated)),
        ]);
        for member in &association.memberships {
            let client = match &member.client {
                RecordedClient::Missing => Value::Null,
                RecordedClient::Label(label) => Value::str(label.as_str()),
                RecordedClient::Invalid => Value::Arr(vec![Value::str("invalid")]),
            };
            out.record(vec![
                ("member", optional(member.session.as_deref(), Value::str)),
                ("slot", Value::str(member.slot.as_str())),
                ("role", Value::str(role_word(member.role))),
                ("client", client),
            ]);
        }
    }
    out.close()
}

/// A publication's intent.
#[must_use]
pub fn render_intent(intent: &Intent) -> Vec<u8> {
    let out = Out::head(
        "tools_intent",
        vec![
            ("conv", Value::str(intent.conv.as_str())),
            ("generation", number(intent.generation)),
        ],
    );
    out.close()
}

// --- reading ---------------------------------------------------------------

fn damaged<T>(what: &'static str) -> Parsed<T> {
    Err(DocError::Damaged(what))
}

/// The records of a document of `key` under `cap`: its head checked for the
/// format first, its final LF and `end` record checked and taken off.
fn records(bytes: &[u8], cap: u64, key: &str) -> Parsed<Vec<Fields>> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > cap {
        return Err(DocError::Oversized);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| DocError::Damaged("utf8"))?;
    let head = text.split('\n').next().unwrap_or_default();
    let head = object(head)?;
    match head.first() {
        Some((name, Value::Num(format))) if name == key => {
            if *format != TOOLS_FORMAT {
                return Err(DocError::Format(*format));
            }
        }
        _ => return damaged("head"),
    }
    let Some(body) = text.strip_suffix('\n') else {
        return damaged("no final LF");
    };
    let mut records = body.split('\n').map(object).collect::<Parsed<Vec<_>>>()?;
    let end = records.pop().ok_or(DocError::Damaged("empty"))?;
    let written = u64::try_from(records.len()).unwrap_or(u64::MAX);
    match end.as_slice() {
        [(name, value)] if name == "end" && count(value)? == written => {}
        _ => return damaged("end"),
    }
    if records.is_empty() {
        return damaged("empty");
    }
    Ok(records)
}

fn object(line: &str) -> Parsed<Fields> {
    match json::parse(line) {
        Ok(Value::Obj(fields)) => Ok(fields),
        _ => damaged("record"),
    }
}

fn field<'a>(fields: &'a Fields, key: &str) -> Parsed<&'a Value> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .ok_or(DocError::Damaged("field"))
}

fn count(value: &Value) -> Parsed<u64> {
    match value {
        Value::Num(n) => u64::try_from(*n).map_err(|_| DocError::Damaged("number")),
        Value::Raw(text) => text.parse().map_err(|_| DocError::Damaged("number")),
        _ => damaged("number"),
    }
}

fn int(value: &Value) -> Parsed<i64> {
    match value {
        Value::Num(n) => Ok(*n),
        _ => damaged("number"),
    }
}

fn flag(value: &Value) -> Parsed<bool> {
    match value {
        Value::Bool(on) => Ok(*on),
        _ => damaged("flag"),
    }
}

fn text(value: &Value) -> Parsed<String> {
    match value {
        Value::Str(text) => Ok(text.clone()),
        _ => damaged("text"),
    }
}

fn digest<const N: usize>(value: &Value) -> Parsed<[u8; N]> {
    let Value::Str(text) = value else {
        return damaged("digest");
    };
    let bytes = text.as_bytes();
    if bytes.len() != 2 * N {
        return damaged("digest");
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => damaged("digest"),
    };
    let mut out = [0_u8; N];
    for (at, pair) in bytes.chunks_exact(2).enumerate() {
        if let (Some(slot), [high, low]) = (out.get_mut(at), pair) {
            *slot = (nibble(*high)? << 4) | nibble(*low)?;
        }
    }
    Ok(out)
}

fn key(value: &Value) -> Parsed<Key> {
    digest(value).map(Key)
}

fn maybe<T>(value: &Value, of: impl FnOnce(&Value) -> Parsed<T>) -> Parsed<Option<T>> {
    match value {
        Value::Null => Ok(None),
        other => of(other).map(Some),
    }
}

fn array(value: &Value) -> Parsed<&[Value]> {
    match value {
        Value::Arr(items) => Ok(items),
        _ => damaged("array"),
    }
}

fn kind_of(word: &str) -> Parsed<Kind> {
    Ok(match word {
        "skill" => Kind::Skill,
        "server" => Kind::Server,
        "tool" => Kind::Tool,
        "model" => Kind::Model,
        "effort" => Kind::Effort,
        _ => return damaged("kind"),
    })
}

/// A name of `kind` as the adapters make it: one component
/// [`super::ident`] keeps whole, or with `composite` (a Codex tool) two
/// such components joined by exactly one `/`.
fn admissible(kind: Kind, name: &str, composite: bool) -> bool {
    let one = |part: &str| matches!(super::ident(kind, part), Ident::Name(_));
    one(name)
        || (composite
            && name
                .split_once('/')
                .is_some_and(|(space, tool)| one(space) && one(tool)))
}

/// An identity of `kind` in its position: a name [`admissible`] there, or
/// unresolved of that same kind.
fn parse_ident(value: &Value, kind: Kind, composite: bool) -> Parsed<Ident> {
    match value {
        Value::Str(name) if admissible(kind, name, composite) => Ok(Ident::Name(name.clone())),
        Value::Arr(items) => match items.as_slice() {
            [Value::Str(word)] if kind_of(word) == Ok(kind) => Ok(Ident::Unresolved(kind)),
            _ => damaged("ident"),
        },
        _ => damaged("ident"),
    }
}

/// A model or effort cell: unknown, or a name of `kind`.
fn parse_cell(value: &Value, kind: Kind) -> Parsed<Cell> {
    match value {
        Value::Null => Ok(Cell::Unknown),
        Value::Str(name) if admissible(kind, name, false) => Ok(Cell::Value(name.clone())),
        _ => damaged("cell"),
    }
}

/// A client cell: unknown, or a client label (an agent name).
fn parse_client_cell(value: &Value) -> Parsed<Cell> {
    match value {
        Value::Null => Ok(Cell::Unknown),
        Value::Str(label) if crate::config::is_agent_name(label) => Ok(Cell::Value(label.clone())),
        _ => damaged("client"),
    }
}

fn parse_stream(value: &Value) -> Parsed<StreamKey> {
    match array(value)? {
        [role, copy] => Ok(StreamKey {
            role: match role {
                Value::Null => StreamRole::Parent,
                Value::Str(name) => StreamRole::Child(name.clone()),
                _ => return damaged("stream"),
            },
            copy: digest(copy)?,
        }),
        _ => damaged("stream"),
    }
}

fn parse_adapter(value: &Value) -> Parsed<Adapter> {
    match value {
        Value::Str(word) if word == "claude" => Ok(Adapter::Claude),
        Value::Str(word) if word == "codex" => Ok(Adapter::Codex),
        _ => damaged("adapter"),
    }
}

fn parse_page(value: &Value) -> Parsed<Page> {
    match value {
        Value::Str(word) if word == "skills" => Ok(Page::Skills),
        Value::Str(word) if word == "mcp" => Ok(Page::Mcp),
        Value::Str(word) if word == "tools" => Ok(Page::Tools),
        _ => damaged("page"),
    }
}

fn parse_refused(value: &Value) -> Parsed<Option<Refused>> {
    maybe(value, |value| match value {
        Value::Str(word) if word == "observations" => Ok(Refused::Observations),
        Value::Str(word) if word == "turns" => Ok(Refused::Turns),
        Value::Str(word) if word == "size" => Ok(Refused::Size),
        _ => damaged("refused"),
    })
}

fn parse_class(value: &Value) -> Parsed<ScopeClass> {
    match value {
        Value::Str(word) if word == "user" => Ok(ScopeClass::User),
        Value::Str(word) if word == "project" => Ok(ScopeClass::Project),
        Value::Str(word) if word == "plugin" => Ok(ScopeClass::Plugin),
        Value::Str(word) if word == "other" => Ok(ScopeClass::Other),
        _ => damaged("class"),
    }
}

fn parse_gap(value: &Value) -> Parsed<GapKind> {
    if let Value::Arr(items) = value {
        return match items.as_slice() {
            [Value::Str(word), Value::Str(tool)]
                if word == "unsupported" && admissible(Kind::Tool, tool, false) =>
            {
                Ok(GapKind::Unsupported(tool.clone()))
            }
            _ => damaged("gap"),
        };
    }
    let Value::Str(word) = value else {
        return damaged("gap");
    };
    Ok(match word.as_str() {
        "unlocated" => GapKind::Unlocated,
        "unreadable" => GapKind::Unreadable,
        "changed" => GapKind::Changed,
        "not_unique" => GapKind::NotUnique,
        "malformed" => GapKind::Malformed,
        "over_cap" => GapKind::OverCap,
        "conflict" => GapKind::Conflict,
        "read_path_over_limit" => GapKind::ReadPathOverLimit,
        "ledger" => GapKind::Ledger,
        "ident_cap" => GapKind::IdentCap,
        "field_over_limit" => GapKind::FieldOverLimit,
        "unproven_reads_dropped" => GapKind::UnprovenReadsDropped,
        "source_gone" => GapKind::SourceGone,
        "view_over_cap" => GapKind::ViewOverCap,
        _ => return damaged("gap"),
    })
}

/// An item; `composite` admits a Codex tool name.
fn parse_item(value: &Value, composite: bool) -> Parsed<Item> {
    match array(value)? {
        [Value::Str(word), tool] if word == "tool" => {
            Ok(Item::Tool(parse_ident(tool, Kind::Tool, composite)?))
        }
        [Value::Str(word), server, tool] if word == "mcp" => Ok(Item::Mcp {
            server: parse_ident(server, Kind::Server, false)?,
            tool: parse_ident(tool, Kind::Tool, false)?,
        }),
        [Value::Str(word), name, scope] if word == "skill" => Ok(Item::Skill {
            name: parse_ident(name, Kind::Skill, false)?,
            scope: maybe(scope, |scope| match array(scope)? {
                [class, resource] => Ok((parse_class(class)?, Resource(digest(resource)?))),
                _ => damaged("scope"),
            })?,
        }),
        _ => damaged("item"),
    }
}

fn parse_cols(page: Page, value: &Value) -> Parsed<Cols> {
    let numbers = array(value)?
        .iter()
        .map(count)
        .collect::<Parsed<Vec<u64>>>()?;
    Ok(match (page, numbers.as_slice()) {
        (Page::Skills, [attempts, loaded, not_loaded, unknown, unresolved, reads]) => {
            Cols::Skill(SkillCols {
                attempts: *attempts,
                loaded: *loaded,
                not_loaded: *not_loaded,
                unknown: *unknown,
                unresolved: *unresolved,
                reads: *reads,
            })
        }
        (Page::Mcp, [attempts, returned, success, error, unresolved]) => Cols::Mcp(McpCols {
            attempts: *attempts,
            returned: *returned,
            success: *success,
            error: *error,
            unresolved: *unresolved,
        }),
        (Page::Tools, [attempts, returned, error, unresolved]) => Cols::Tool(ToolCols {
            attempts: *attempts,
            returned: *returned,
            error: *error,
            unresolved: *unresolved,
        }),
        _ => return damaged("cols"),
    })
}

fn parse_stamp(value: &Value) -> Parsed<(i64, u32)> {
    match array(value)? {
        [seconds, nanos] => Ok((
            int(seconds)?,
            u32::try_from(count(nanos)?).map_err(|_| DocError::Damaged("stamp"))?,
        )),
        _ => damaged("stamp"),
    }
}

fn parse_status(fields: &Fields) -> Parsed<FileState> {
    Ok(FileState {
        dev: count(field(fields, "dev")?)?,
        ino: count(field(fields, "ino")?)?,
        len: count(field(fields, "len")?)?,
        mtime: parse_stamp(field(fields, "mtime")?)?,
        ctime: parse_stamp(field(fields, "ctime")?)?,
        cursor: count(field(fields, "cursor")?)?,
        skipping: flag(field(fields, "skipping")?)?,
        blocks: array(field(fields, "blocks")?)?
            .iter()
            .map(digest)
            .collect::<Parsed<_>>()?,
    })
}

fn parse_load(value: &Value) -> Parsed<Option<Load>> {
    maybe(value, |value| match value {
        Value::Str(word) if word == "loaded" => Ok(Load::Loaded),
        Value::Str(word) if word == "not_loaded" => Ok(Load::NotLoaded),
        Value::Str(word) if word == "unknown" => Ok(Load::Unknown),
        _ => damaged("load"),
    })
}

fn parse_outcome(value: &Value) -> Parsed<McpOutcome> {
    match value {
        Value::Str(word) if word == "success" => Ok(McpOutcome::Success),
        Value::Str(word) if word == "error" => Ok(McpOutcome::Error),
        Value::Str(word) if word == "returned" => Ok(McpOutcome::Returned),
        Value::Str(word) if word == "conflict" => Ok(McpOutcome::Conflict),
        _ => damaged("outcome"),
    }
}

/// An event of a conversation; `composite` (Codex) admits a namespaced tool.
fn parse_event(fields: &Fields, composite: bool) -> Parsed<Event> {
    let Some((name, value)) = fields.first() else {
        return damaged("event");
    };
    let get = |key: &str| field(fields, key);
    Ok(match name.as_str() {
        "stamp" => Event::Stamp(int(value)?),
        "attempt" => Event::Attempt {
            id: key(value)?,
            tool: parse_ident(get("tool")?, Kind::Tool, composite)?,
            mcp: maybe(get("mcp")?, |pair| match array(pair)? {
                [server, tool] => Ok((
                    parse_ident(server, Kind::Server, false)?,
                    parse_ident(tool, Kind::Tool, false)?,
                )),
                _ => damaged("mcp"),
            })?,
            model: maybe(get("model")?, |model| {
                parse_ident(model, Kind::Model, false)
            })?,
            turn: maybe(get("turn")?, key)?,
            message: maybe(get("message")?, key)?,
        },
        "skill" => Event::Skill {
            id: key(value)?,
            skill: parse_ident(get("name")?, Kind::Skill, false)?,
        },
        "base" => Event::SkillBase {
            id: key(value)?,
            class: parse_class(get("class")?)?,
            resource: Resource(digest(get("resource")?)?),
            dir: Dir(digest(get("dir")?)?),
        },
        "result" => Event::Result {
            id: key(value)?,
            error: flag(get("error")?)?,
            load: parse_load(get("load")?)?,
        },
        "read" => Event::Read {
            id: key(value)?,
            direct: maybe(get("direct")?, |direct| match array(direct)? {
                [name, class, resource] => Ok((
                    parse_ident(name, Kind::Skill, false)?,
                    parse_class(class)?,
                    Resource(digest(resource)?),
                )),
                _ => damaged("direct"),
            })?,
            ancestors: array(get("ancestors")?)?
                .iter()
                .map(|dir| digest(dir).map(Dir))
                .collect::<Parsed<_>>()?,
            over: flag(get("over")?)?,
            skills: flag(get("skills")?)?,
        },
        "mcp_call" => Event::Mcp {
            id: key(value)?,
            server: parse_ident(get("server")?, Kind::Server, false)?,
            tool: parse_ident(get("tool")?, Kind::Tool, false)?,
            outcome: parse_outcome(get("outcome")?)?,
            turn: maybe(get("turn")?, key)?,
        },
        "context" => Event::Context {
            turn: key(value)?,
            model: parse_ident(get("model")?, Kind::Model, false)?,
            effort: parse_ident(get("effort")?, Kind::Effort, false)?,
        },
        _ => return damaged("event"),
    })
}

/// The canonical-form check every parse ends with.
fn canonical<T>(bytes: &[u8], value: T, render: impl Fn(&T) -> Vec<u8>) -> Parsed<T> {
    if render(&value) == bytes {
        Ok(value)
    } else {
        damaged("not canonical")
    }
}

fn first_key(fields: &Fields) -> &str {
    fields.first().map_or("", |(name, _)| name.as_str())
}

/// # Errors
///
/// [`DocError::Format`] for another format, [`DocError::Oversized`] past
/// [`CONV_CAP`], else [`DocError::Damaged`] for anything not canonical.
pub fn parse_conv(bytes: &[u8]) -> Result<Conv, DocError> {
    let records = records(bytes, CONV_CAP, "tools_conv")?;
    let (head, body) = records.split_first().ok_or(DocError::Damaged("empty"))?;
    let mut conv = Conv::new(
        parse_adapter(field(head, "adapter")?)?,
        &text(field(head, "id")?)?,
    );
    conv.summary.read = flag(field(head, "read")?)?;
    conv.summary.refused = parse_refused(field(head, "refused")?)?;
    conv.resume.fallback = flag(field(head, "fallback")?)?;
    let mut current: Option<StreamKey> = None;
    for fields in body {
        let lead = fields.first().map(|(_, value)| value);
        match (first_key(fields), lead) {
            ("file", Some(value)) => {
                let source = if flag(field(fields, "gone")?)? {
                    Source::Gone
                } else {
                    Source::Present
                };
                conv.summary.files.insert(parse_stream(value)?, source);
            }
            ("status", Some(value)) => {
                conv.resume
                    .files
                    .insert(parse_stream(value)?, parse_status(fields)?);
            }
            ("lines", Some(value)) => {
                conv.resume.lines.insert(
                    parse_stream(value)?,
                    (
                        count(field(fields, "malformed")?)?,
                        count(field(fields, "over_cap")?)?,
                    ),
                );
            }
            ("mark", Some(value)) => conv.resume.marks.push(parse_gap(value)?),
            ("dropped", Some(value)) => {
                conv.resume.dropped.insert(key(value)?);
            }
            ("stream", Some(value)) => {
                let stream = parse_stream(value)?;
                conv.resume.streams.entry(stream.clone()).or_default();
                current = Some(stream);
            }
            _ => {
                let Some(stream) = &current else {
                    return damaged("event outside a stream");
                };
                let event = parse_event(fields, conv.summary.adapter == Adapter::Codex)?;
                conv.resume
                    .streams
                    .entry(stream.clone())
                    .or_default()
                    .insert(event);
            }
        }
    }
    // Counts are kept exactly; held counts a summary could not add up are
    // damage, as a live update refuses them (`Conv::note`).
    if incr::line_total(&conv.resume.lines, conv.resume.marks.len()).is_none() {
        return damaged("count overflow");
    }
    conv.summarize();
    canonical(bytes, conv, render_conv)
}

/// # Errors
///
/// As [`parse_conv`], under [`VIEW_CAP`].
pub fn parse_view(bytes: &[u8]) -> Result<Published, DocError> {
    let records = records(bytes, VIEW_CAP, "tools_view")?;
    let (head, body) = records.split_first().ok_or(DocError::Damaged("empty"))?;
    let mut view = Published {
        generation: count(field(head, "generation")?)?,
        convs: Vec::new(),
        over_cap: count(field(head, "over_cap")?)?,
    };
    for fields in body {
        let lead = fields.first().map(|(_, value)| value);
        if let ("conv", Some(value)) = (first_key(fields), lead) {
            let observations = count(field(fields, "observations")?)?;
            let turns = count(field(fields, "turns")?)?;
            view.convs.push(PublishedConv {
                summary: Summary {
                    adapter: parse_adapter(value)?,
                    id: text(field(fields, "id")?)?,
                    rows: BTreeMap::new(),
                    orphans: count(field(fields, "orphans")?)?,
                    range: maybe(field(fields, "range")?, |range| match array(range)? {
                        [from, to] => Ok((int(from)?, int(to)?)),
                        _ => damaged("range"),
                    })?,
                    gaps: BTreeMap::new(),
                    observations: usize::try_from(observations)
                        .map_err(|_| DocError::Damaged("observations"))?,
                    turns: usize::try_from(turns).map_err(|_| DocError::Damaged("turns"))?,
                    read: flag(field(fields, "read")?)?,
                    files: BTreeMap::new(),
                    refused: parse_refused(field(fields, "refused")?)?,
                    names: BTreeMap::new(),
                    inferred: BTreeMap::new(),
                    proofs: BTreeMap::new(),
                },
                sessions: BTreeSet::new(),
                client: parse_client_cell(field(fields, "client")?)?,
            });
            continue;
        }
        let Some(conv) = view.convs.last_mut() else {
            return damaged("record outside a conversation");
        };
        view_record(conv, fields)?;
    }
    // Every inferred Read's directory has its proofs, and only those do.
    for conv in &view.convs {
        let summary = &conv.summary;
        let used: BTreeSet<&Dir> = summary.inferred.keys().map(|(dir, ..)| dir).collect();
        if !used.iter().copied().eq(summary.proofs.keys()) {
            return damaged("proofs");
        }
    }
    // Each conversation once, in strict `(adapter, id)` order: the order
    // `Published::admit` keeps and replaces in.
    let ordered = view.convs.windows(2).all(|pair| match pair {
        [one, next] => {
            (one.summary.adapter, &one.summary.id) < (next.summary.adapter, &next.summary.id)
        }
        _ => true,
    });
    if !ordered {
        return damaged("conversation order");
    }
    // A view a combine could not add up is damage, as `Published::admit`
    // refuses it live.
    if view.total().is_none() {
        return damaged("count overflow");
    }
    canonical(bytes, view, render_view)
}

/// One record of a view's conversation after its `conv` record.
fn view_record(conv: &mut PublishedConv, fields: &Fields) -> Parsed<()> {
    let lead = fields.first().map(|(_, value)| value);
    let summary = &mut conv.summary;
    let codex = summary.adapter == Adapter::Codex;
    match (first_key(fields), lead) {
        ("session", Some(value)) => {
            conv.sessions.insert(text(value)?);
        }
        ("file", Some(value)) => {
            let source = if flag(field(fields, "gone")?)? {
                Source::Gone
            } else {
                Source::Present
            };
            summary.files.insert(parse_stream(value)?, source);
        }
        ("gap", Some(value)) => {
            summary
                .gaps
                .insert(parse_gap(value)?, count(field(fields, "count")?)?);
        }
        ("name", Some(value)) => {
            let Value::Str(word) = value else {
                return damaged("name");
            };
            let kind = kind_of(word)?;
            let name = text(field(fields, "value")?)?;
            if !admissible(kind, &name, codex && kind == Kind::Tool) {
                return damaged("name");
            }
            summary.names.entry(kind).or_default().insert(name);
        }
        ("row", Some(value)) => {
            let page = parse_page(value)?;
            if parse_adapter(field(fields, "harness")?)? != summary.adapter {
                return damaged("harness");
            }
            summary.rows.insert(
                (
                    page,
                    parse_item(field(fields, "item")?, codex)?,
                    summary.adapter,
                    parse_cell(field(fields, "model")?, Kind::Model)?,
                    parse_cell(field(fields, "effort")?, Kind::Effort)?,
                ),
                parse_cols(page, field(fields, "cols")?)?,
            );
        }
        ("proof", Some(value)) => {
            let item = parse_item(field(fields, "item")?, false)?;
            if !matches!(item, Item::Skill { scope: Some(_), .. }) {
                return damaged("proof");
            }
            summary
                .proofs
                .entry(Dir(digest(value)?))
                .or_default()
                .insert(item);
        }
        ("inferred", Some(value)) => {
            let reads = count(field(fields, "reads")?)?;
            if reads == 0 {
                return damaged("inferred");
            }
            summary.inferred.insert(
                (
                    Dir(digest(value)?),
                    parse_cell(field(fields, "model")?, Kind::Model)?,
                    parse_cell(field(fields, "effort")?, Kind::Effort)?,
                ),
                reads,
            );
        }
        _ => return damaged("view record"),
    }
    Ok(())
}

fn parse_role(value: &Value) -> Parsed<Role> {
    match value {
        Value::Str(word) if word == "live" => Ok(Role::Live),
        Value::Str(word) if word == "prior" => Ok(Role::Prior),
        Value::Str(word) if word == "retired" => Ok(Role::Retired),
        Value::Str(word) if word == "archived" => Ok(Role::Archived),
        _ => damaged("role"),
    }
}

fn parse_client(value: &Value) -> Parsed<RecordedClient> {
    match value {
        Value::Null => Ok(RecordedClient::Missing),
        Value::Str(label) if crate::config::is_agent_name(label) => {
            Ok(RecordedClient::Label(label.clone()))
        }
        Value::Arr(items) => match items.as_slice() {
            [Value::Str(word)] if word == "invalid" => Ok(RecordedClient::Invalid),
            _ => damaged("client"),
        },
        _ => damaged("client"),
    }
}

/// # Errors
///
/// As [`parse_conv`], under [`ASSOC_CAP`].
pub fn parse_assoc(bytes: &[u8]) -> Result<Assoc, DocError> {
    let records = records(bytes, ASSOC_CAP, "tools_assoc")?;
    let (_, body) = records.split_first().ok_or(DocError::Damaged("empty"))?;
    let mut assoc = Assoc {
        sources: BTreeMap::new(),
        memberships: BTreeMap::new(),
    };
    let mut current: Option<(Adapter, String)> = None;
    for fields in body {
        let lead = fields.first().map(|(_, value)| value);
        match (first_key(fields), lead) {
            ("source", Some(value)) => {
                assoc.sources.insert(text(value)?, parse_status(fields)?);
            }
            ("conv", Some(value)) => {
                let key = (parse_adapter(value)?, text(field(fields, "id")?)?);
                assoc.memberships.insert(
                    key.clone(),
                    Association {
                        memberships: Vec::new(),
                        evidence_complete: flag(field(fields, "complete")?)?,
                        reseated: flag(field(fields, "reseated")?)?,
                    },
                );
                current = Some(key);
            }
            ("member", Some(value)) => {
                let Some(association) = current
                    .as_ref()
                    .and_then(|key| assoc.memberships.get_mut(key))
                else {
                    return damaged("member outside a conversation");
                };
                association.memberships.push(Membership {
                    session: maybe(value, text)?,
                    slot: text(field(fields, "slot")?)?,
                    role: parse_role(field(fields, "role")?)?,
                    client: parse_client(field(fields, "client")?)?,
                });
            }
            _ => return damaged("assoc record"),
        }
    }
    canonical(bytes, assoc, render_assoc)
}

/// # Errors
///
/// As [`parse_conv`], under [`INTENT_CAP`].
pub fn parse_intent(bytes: &[u8]) -> Result<Intent, DocError> {
    let records = records(bytes, INTENT_CAP, "tools_intent")?;
    let (head, body) = records.split_first().ok_or(DocError::Damaged("empty"))?;
    if !body.is_empty() {
        return damaged("intent record");
    }
    let intent = Intent {
        conv: text(field(head, "conv")?)?,
        generation: count(field(head, "generation")?)?,
    };
    canonical(bytes, intent, render_intent)
}

#[cfg(test)]
#[path = "doc_tests.rs"]
mod tests;
