//! The event record, and the generation-aware reader DR-001 requires.
//!
//! # What the rows say
//!
//! * **SC-510a** — every event carries `ts` (ISO 8601 UTC, second precision),
//!   `actor` and `action`. A line missing one is not an event.
//! * **SC-510b** — `target` / `ref` / `summary` never appear as empty strings.
//!   Modelled as [`Option`]: absent and empty are the same absence, and the type
//!   makes the empty string unrepresentable rather than merely unwritten.
//! * **SC-510c** — `ref` is polysemous and the action decides its meaning. That
//!   is a table in prose and a [`RefMeaning`] here, so a caller cannot read a
//!   memo topic as a request id.
//! * **SC-510d** — string values are JSON-escaped (see [`crate::json`]).
//! * **SC-511a** — messaging events also carry `actor_slot` / `actor_session` /
//!   `target_slot` / `target_session` when known, omitted when empty.
//! * **SC-511b** — readers prefer slot+session over display name, and ignore
//!   keys they do not understand. [`Identity`] is that preference, resolved
//!   once, at the boundary.
//! * **SC-511c** — evolution is additive-only, so an unknown key is data this
//!   reader steps over, never an error.
//! * **SC-510e** — a KNOWN key appearing twice makes the whole record
//!   malformed: no row defines precedence, so choosing a winner would be
//!   fabrication. Skipped and counted, degrading through SC-520.
//! * **SC-510f** — a duplicated UNKNOWN key stays inert. A reader cannot
//!   fabricate a value it never took.
//! * **SC-405j** — PRESENCE of a routing member is decided BEFORE any
//!   empty-string normalization. Structurally absent members permit the legacy
//!   display fallback; any present member that does not fully and freshly match
//!   — stale, partial, or empty — makes the identity [`Identity::Unassociated`],
//!   which matches nothing.
//!
//! # DR-001
//!
//! The reader is generation-aware from the first line of Rust, while exactly one
//! container exists. The DR's binding conditions shape the API directly:
//! append-only WITHIN a generation, a reader **drains a stable opened generation
//! before advancing**, and the cursor persists **generation + offset**. So
//! [`Cursor`] is a pair, [`Drain`] reports whether the generation it read is
//! finished, and advancing is a separate, explicit step that only a drained
//! generation permits.
//!
//! What is deliberately NOT here: any *naming convention* for generation files.
//! DR-001 states that the written layout and its legacy-read/migration/write
//! ownership land "at the flip commit", which has not happened. So
//! [`EventLog::discover`] maps today's single container to generation 0 and
//! nothing else, and multi-generation behavior is exercised through
//! [`EventLog::from_sources`], which takes the paths it is given.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::attention::Reason;
use crate::json::{self, Value};
use crate::time::Timestamp;

/// The bash-era event container. SC-400a keeps it readable across every flip.
pub const LEGACY_CONTAINER: &str = "events.jsonl";

/// One record from the event log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// SC-510a: ISO 8601 UTC, second precision.
    pub ts: Timestamp,
    /// SC-510a: `alias:name` of the emitter, or `watchdog` / `human`.
    pub actor: String,
    /// SC-510a: the event type.
    pub action: String,
    /// SC-510b: the recipient, when applicable.
    pub target: Option<String>,
    /// SC-510b/c: the correlation value, whose meaning the action decides.
    pub reference: Option<String>,
    /// SC-510b: a truncated preview of the payload.
    pub summary: Option<String>,
    /// SC-511a: the sender's slot.
    pub actor_slot: RoutingMember,
    /// SC-511a: the sender's session.
    pub actor_session: RoutingMember,
    /// SC-511a: the recipient's slot.
    pub target_slot: RoutingMember,
    /// SC-511a: the recipient's session.
    pub target_session: RoutingMember,
}

/// One half of a routing key, exactly as the record carries it.
///
/// **SC-405j, as amended: PRESENCE is decided BEFORE any empty-string
/// normalization.** A member that appears in the record's JSON is PRESENT even
/// when its value is `""`, so the three states here are the three the contract
/// distinguishes — and collapsing the middle one into [`Self::Absent`] is
/// exactly what let a malformed keyed record fall back to its display name and
/// close a request it never answered.
///
/// The row is explicit that a reader may not lean on SC-510b or SC-511a to do
/// that collapsing: those are PRODUCER rules — what ae writes — and say nothing
/// about what a reader may erase.
///
/// An empty member therefore makes its identity [`Identity::Unassociated`],
/// the same as a partial or stale key. The record is NOT skipped: the fact
/// stays countable and the rest of the event is still true. It identifies
/// nobody, which is a different thing from being unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RoutingMember {
    /// The key is not in the record. Every pre-SC-511a event looks like this.
    #[default]
    Absent,
    /// The key is present and empty: the writer meant to route and did not say
    /// where.
    Invalid,
    /// The key carries a value.
    Value(String),
}

impl RoutingMember {
    /// Read one routing key off a record.
    ///
    /// # Errors
    ///
    /// [`EventError::WrongType`] when the key is present and not a string —
    /// the same rule every other known key follows.
    fn read(value: &Value, key: &'static str) -> Result<Self, EventError> {
        match value.get(key) {
            None => Ok(Self::Absent),
            Some(Value::Str(text)) if text.is_empty() => Ok(Self::Invalid),
            Some(Value::Str(text)) => Ok(Self::Value(text.clone())),
            Some(_) => Err(EventError::WrongType(key)),
        }
    }

    /// The value, when this member carries one.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Value(text) => Some(text),
            Self::Absent | Self::Invalid => None,
        }
    }

    /// Whether the record carries this key at all, in any state.
    ///
    /// A record with NO routing keys present is the only one that falls back to
    /// its display name (SC-405j).
    #[must_use]
    pub const fn is_present(&self) -> bool {
        !matches!(self, Self::Absent)
    }
}

/// What a `ref` means, per the COMPLETE SC-510c action table.
///
/// The row was amended after this reader was first written: the original
/// dropped the authority's own hedge ("usually absent") and its `state` entry,
/// which made the self-declared attention reasons underivable. The amendment
/// cost exactly one variant and one match arm, because the polysemy is a type
/// here rather than a string comparison at each call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefMeaning<'a> {
    /// `ask` / `review` / `reply` — the request id pairing the three.
    RequestId(&'a str),
    /// `memo` — the topic the memo was filed under.
    MemoTopic(&'a str),
    /// `recover` — the captured tool session id.
    CapturedSessionId(&'a str),
    /// `state` — the declared work state (`working` / `waiting-user` /
    /// `blocked` / `done`).
    DeclaredState(&'a str),
    /// Any other action. SC-510c says `ref` is *usually* absent there — never
    /// categorically absent — so a value that turns up carries no meaning the
    /// table defines, which is not the same as carrying none at all.
    Undefined,
}

/// What one event says about the watchdog's standing verdict on an agent.
///
/// Three answers rather than two, because a CLEAR is not the absence of an
/// alert — it is the watchdog retracting one — while an event that is neither
/// must leave a backward scan looking further rather than answering it. Collapse
/// the third into the second and every nudge in the log silently cancels the
/// alert it was sent about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertMeaning {
    /// The watchdog says this agent needs a human, for this reason.
    Raised(Reason),
    /// The watchdog says the condition it alerted on is over.
    Cleared,
    /// Not a watchdog verdict at all.
    Undefined,
}

/// How a reader names a participant.
///
/// SC-511b: pairing and delivery use the churn-proof routing key **where
/// present**, and fall back to the display name where there is none at all.
///
/// SC-405j makes a PARTIAL key negative rather than absent, and the distinction
/// is the whole point of the third variant: an event carrying `actor_slot` with
/// no `actor_session` has told us it is routed and then failed to say where, so
/// reading it as a display name would let it match a display-only counterpart
/// that names a different agent. [`Identity::Unassociated`] matches nothing —
/// including another `Unassociated` — so the failure is a loud non-match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identity<'a> {
    /// The routing key: slot + session, which survives a display-name change.
    Routed {
        /// `main` / `worker.<n>` / `spawned.<n>`.
        slot: &'a str,
        /// The session the slot belongs to.
        session: &'a str,
    },
    /// The display name — all a pre-SC-511a event carries.
    Display(&'a str),
    /// Half a routing key: routed, but to nowhere nameable.
    Unassociated,
}

/// Why a line is not an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventError {
    /// The line is not JSON at all.
    NotJson(json::ParseError),
    /// The line is JSON, but not an object.
    NotAnObject,
    /// SC-510a: a required key is missing or empty.
    MissingKey(&'static str),
    /// SC-510a: `ts` is present but not the documented spelling.
    UnreadableTimestamp(String),
    /// A key the schema DOES define, appearing more than once in one record.
    ///
    /// **SC-510e** — no row defines duplicate-member precedence, RFC 8259 makes
    /// duplicate-name resolution non-interoperable, and picking a first or last
    /// winner among KNOWN keys is forbidden fabrication. The whole record is
    /// skipped and counted, degrading the session through SC-520's path.
    ///
    /// **SC-510f** — duplicate UNKNOWN keys stay inert: additive-schema
    /// semantics ignore a member this reader never reads, however many times it
    /// appears, and it cannot fabricate a value it never took.
    ///
    /// (Not SC-405e: that is meta grammar, still UNCLASSIFIED, and not
    /// authority here.)
    DuplicateKey(&'static str),
    /// A key the schema DOES define, carrying a value of the wrong JSON type —
    /// `"ref": 7`, a numeric `target`, an object `summary`.
    ///
    /// Not the same as absent, and SC-509b/SC-520 forbid rendering it as such:
    /// the writer meant to say something and the reader could not take it, so
    /// the record is skipped and counted rather than silently emptied. Unknown
    /// keys of any type stay ignored — SC-511b says a reader steps over what it
    /// does not understand, and that is exactly what it does NOT understand.
    WrongType(&'static str),
}

impl std::fmt::Display for EventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson(source) => write!(f, "{source}"),
            Self::NotAnObject => write!(f, "not a json object"),
            Self::MissingKey(key) => write!(f, "missing required key: {key}"),
            Self::UnreadableTimestamp(text) => write!(f, "unreadable ts: {text}"),
            Self::WrongType(key) => write!(f, "wrong json type for key: {key}"),
            Self::DuplicateKey(key) => write!(f, "duplicate key: {key}"),
        }
    }
}

impl std::error::Error for EventError {}

impl Event {
    /// Read one event from a JSON line.
    ///
    /// # Errors
    ///
    /// Returns [`EventError`] when the line is not JSON, is not an object, or
    /// is missing one of SC-510a's three required keys.
    ///
    /// ```
    /// let e = ae::events::Event::parse_line(
    ///     r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"done"}"#,
    /// )?;
    /// assert_eq!(e.action, "done");
    /// assert_eq!(e.summary, None);
    /// # Ok::<(), ae::events::EventError>(())
    /// ```
    pub fn parse_line(line: &str) -> Result<Self, EventError> {
        let value = json::parse(line).map_err(EventError::NotJson)?;
        Self::from_json(&value)
    }

    /// Read one event from an already-parsed JSON value.
    ///
    /// # Errors
    ///
    /// As [`Event::parse_line`], minus the JSON syntax case.
    pub fn from_json(value: &Value) -> Result<Self, EventError> {
        let Value::Obj(fields) = value else {
            return Err(EventError::NotAnObject);
        };
        reject_duplicate_known_keys(fields)?;
        let ts_text = required(value, "ts")?;
        let ts = Timestamp::parse(ts_text)
            .ok_or_else(|| EventError::UnreadableTimestamp(ts_text.to_owned()))?;
        Ok(Self {
            ts,
            actor: required(value, "actor")?.to_owned(),
            action: required(value, "action")?.to_owned(),
            target: optional(value, "target")?,
            reference: optional(value, "ref")?,
            summary: optional(value, "summary")?,
            actor_slot: RoutingMember::read(value, "actor_slot")?,
            actor_session: RoutingMember::read(value, "actor_session")?,
            target_slot: RoutingMember::read(value, "target_slot")?,
            target_session: RoutingMember::read(value, "target_session")?,
        })
    }

    /// What this event's `ref` means, per SC-510c.
    ///
    /// ```
    /// let memo = ae::events::Event::parse_line(
    ///     r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"memo","ref":"design"}"#,
    /// )?;
    /// assert_eq!(memo.ref_meaning(), ae::events::RefMeaning::MemoTopic("design"));
    /// # Ok::<(), ae::events::EventError>(())
    /// ```
    #[must_use]
    pub fn ref_meaning(&self) -> RefMeaning<'_> {
        let Some(value) = self.reference.as_deref() else {
            return RefMeaning::Undefined;
        };
        match self.action.as_str() {
            "ask" | "review" | "reply" => RefMeaning::RequestId(value),
            "memo" => RefMeaning::MemoTopic(value),
            "recover" => RefMeaning::CapturedSessionId(value),
            "state" => RefMeaning::DeclaredState(value),
            _ => RefMeaning::Undefined,
        }
    }

    /// The work state this record DECLARES, if any — SC-510c as amended.
    ///
    /// Two shapes, because the ledger has always held two. A `state` event
    /// carries the value in `ref` ([`RefMeaning::DeclaredState`]). A bare `done`
    /// event declares `done` outright: events.md's action table says
    /// "`mark-done` is a shim over `state done`; both are read as `done`", and a
    /// session that predates the `state` helper has only the second shape.
    ///
    /// The frozen readers are split on this and the split is why it is written
    /// down here rather than inferred: `_session_states` (ae:3369 — the reader
    /// the LIST path actually uses) and `ae_latest_state_for` (ae:13263) both
    /// accept `done`, while `_ar_latest_state` (ae:4637) requires `state`.
    /// Ruled 2026-08-24 in favour of retaining the legacy record, which is what
    /// the list path did all along.
    ///
    /// A `state` event with no `ref` declares nothing — it has not said what.
    #[must_use]
    pub fn declared_state(&self) -> Option<&str> {
        match self.ref_meaning() {
            RefMeaning::DeclaredState(state) => Some(state),
            _ if self.action == "done" => Some("done"),
            _ => None,
        }
    }

    /// How to name this event's actor, preferring the routing key (SC-511b).
    #[must_use]
    pub fn actor_identity(&self) -> Identity<'_> {
        identity(&self.actor_slot, &self.actor_session, &self.actor)
    }

    /// How to name this event's target, or `None` when it has none.
    #[must_use]
    pub fn target_identity(&self) -> Option<Identity<'_>> {
        let display = self.target.as_deref()?;
        Some(identity(&self.target_slot, &self.target_session, display))
    }

    /// The watchdog verdict this event carries — SC-509c's alert-derived half.
    ///
    /// **SC-980 rules that an alert carries a TYPED reason key and that
    /// free-text `summary` is never a discriminator — and no row names that
    /// key.** There is therefore nothing here to read it from, and picking a
    /// spelling would be fabrication. The row's own escape clause is taken
    /// instead: the incumbent action/summary byte shapes are "T-WD probe
    /// material for the LEGACY ADAPTER", empirical and never SHOULD. This is
    /// that adapter. When SC-980's key is named it is read FIRST and this
    /// cascade becomes the fallback for records written before it existed.
    ///
    /// `ref` is deliberately not pressed into service as that key. SC-510c
    /// makes `ref`'s meaning the ACTION's to decide, and no row decides it for
    /// `alert` — so [`RefMeaning::Undefined`] is the honest answer and reading a
    /// reason out of it would invent the very schema this doc refuses to invent.
    #[must_use]
    pub fn alert_meaning(&self) -> AlertMeaning {
        match self.action.as_str() {
            "alert" => AlertMeaning::Raised(alert_class(self.summary.as_deref())),
            // The watchdog's own retractions. `throttle-cleared` is documented
            // (events.md:105); `alert-cleared` is not in that table but is
            // emitted by both reference implementations, so it is READ here
            // without being published as a row this crate invented.
            "alert-cleared" | "throttle-cleared" => AlertMeaning::Cleared,
            // A CARRIER, and the ACTION is the whole discrimination: SC-509c
            // wants an owner plus an active contribution, `target` names the
            // owner, and `throttled` names the contribution outright. Reading
            // the summary here would NARROW a decision the action has already
            // made — neither needed nor permitted. Ruled 2026-08-24, after both
            // reference implementations were found to define the carrier class
            // this way; events.md:106's "first cycle of a streak" says WHEN the
            // watchdog emits it, not whether the agent owns it.
            "throttled" => AlertMeaning::Raised(Reason::Throttled),
            _ => AlertMeaning::Undefined,
        }
    }
}

/// The reason an incumbent alert `summary` names.
///
/// Pinned by two INDEPENDENT implementations of one algorithm that agree on
/// every arm: `_agents_alert_reasons` in `ae` @72c7293 and
/// `_agent_alert_reason` in `contrib/aewatch/aewatch`. Both are empirical
/// probe material under SC-980, never authority — which is exactly why they are
/// reproduced rather than improved on.
///
/// **The ORDER is load-bearing, in one place only.** The meta-agent wedge alert
/// says "not sweeping" and must reach [`Reason::Stale`] before any later arm can
/// claim its text; the incumbent carries that same warning at the same line.
///
/// An unrecognised summary is `Stale`, never [`AlertMeaning::Undefined`]:
/// `alert` MEANS attention is required, so an alert whose class this cascade
/// cannot read is an alert of unknown class — dropping it would hide the one
/// thing it exists to report.
fn alert_class(summary: Option<&str>) -> Reason {
    let summary = summary.unwrap_or_default();
    if summary.contains("not sweeping") {
        return Reason::Stale;
    }
    // Both spellings, because the incumbent matches both and the match is
    // case-SENSITIVE in every implementation of it.
    if ["dead", "dropped", "missing", "MISSING"]
        .iter()
        .any(|mark| summary.contains(mark))
    {
        return Reason::Dead;
    }
    if summary.contains("throttl") {
        return Reason::Throttled;
    }
    // "max nudges reached (...)", and every alert shape not yet enumerated.
    Reason::Stale
}

/// SC-511b + SC-405j, in one sentence: BOTH halves present and valid route,
/// NEITHER half present falls back to the display name, and everything between
/// those two identifies nobody.
///
/// "Everything between" is the point. A slot without its session cannot be told
/// apart from the same slot in another session; a slot that is present and
/// EMPTY has said even less while still declaring itself routed. Both are the
/// same species under SC-405j, so both answer [`Identity::Unassociated`] —
/// which matches nothing, including another `Unassociated`.
fn identity<'a>(
    slot: &'a RoutingMember,
    session: &'a RoutingMember,
    display: &'a str,
) -> Identity<'a> {
    match (slot, session) {
        (RoutingMember::Value(slot), RoutingMember::Value(session)) => {
            Identity::Routed { slot, session }
        }
        (RoutingMember::Absent, RoutingMember::Absent) => Identity::Display(display),
        _ => Identity::Unassociated,
    }
}

/// Every key this schema defines. Anything else is SC-511b's business, not
/// this reader's.
///
/// One list, used to police duplicates. It is deliberately the WHOLE documented
/// surface rather than "the keys we happen to read", because a key the schema
/// defines is a key a writer may repeat.
const KNOWN_KEYS: [&str; 10] = [
    "ts",
    "actor",
    "action",
    "target",
    "ref",
    "summary",
    "actor_slot",
    "actor_session",
    "target_slot",
    "target_session",
];

/// Refuse a record that names any KNOWN key twice (SC-510e).
///
/// Detection lives here, at the event-consumption layer, and not in
/// [`crate::json`]: the parser's first-wins lookup is fine for a generic JSON
/// object, and SC-510f keeps a repeated UNKNOWN key inert. It is only for the
/// keys the schema DEFINES that "which one did the writer mean" becomes a
/// question nobody has answered.
fn reject_duplicate_known_keys(fields: &[(String, Value)]) -> Result<(), EventError> {
    for known in KNOWN_KEYS {
        if fields.iter().filter(|(key, _)| key == known).count() > 1 {
            return Err(EventError::DuplicateKey(known));
        }
    }
    Ok(())
}

/// A required string key (SC-510a).
///
/// An empty value is a missing value; a value of the wrong type is neither —
/// see [`EventError::WrongType`].
fn required<'a>(value: &'a Value, key: &'static str) -> Result<&'a str, EventError> {
    match value.get(key) {
        Some(Value::Str(text)) if !text.is_empty() => Ok(text),
        None | Some(Value::Str(_)) => Err(EventError::MissingKey(key)),
        Some(_) => Err(EventError::WrongType(key)),
    }
}

/// An optional string key of the SC-510b trio — `target`, `ref`, `summary`.
///
/// Empty-as-absent belongs to these three and stops here. SC-405j's
/// reader-erasure prohibition is scoped, in the row's own words, to the four
/// ROUTING keys: "the SC-510b trio's reader-side empty-as-omission stands
/// unchanged". So this normalisation is the contract, not an exception to it.
///
/// The routing keys deliberately do NOT share it (see [`RoutingMember`]),
/// because for them an empty value is a claim to be routed with the destination
/// missing — a fact worth keeping rather than erasing.
///
/// The row says such a key never appears empty. A reader that met one anyway
/// would have to decide what an empty target means; treating it as the absence
/// the row describes is the only reading that does not invent a third state.
///
/// A value of the WRONG TYPE is a different fact and gets a different answer:
/// `Err` rather than `Ok(None)`, so the record is skipped and counted instead of
/// being emitted with a field silently blanked (SC-509b, SC-520).
///
/// # Errors
///
/// [`EventError::WrongType`] when the key is present and not a string.
fn optional(value: &Value, key: &'static str) -> Result<Option<String>, EventError> {
    match value.get(key) {
        None => Ok(None),
        Some(Value::Str(text)) if text.is_empty() => Ok(None),
        Some(Value::Str(text)) => Ok(Some(text.clone())),
        Some(_) => Err(EventError::WrongType(key)),
    }
}

/// A position in the event stream: which generation, and how far into it.
///
/// DR-001, binding condition: "the cursor persists generation + offset". The
/// pair is the whole point — an offset alone is meaningless once the container
/// it counted into is no longer the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    /// Which generation this offset counts into.
    pub generation: u64,
    /// Bytes consumed, always landing just past a complete record.
    pub offset: u64,
}

impl Cursor {
    /// The start of `generation`.
    #[must_use]
    pub const fn start_of(generation: u64) -> Self {
        Self {
            generation,
            offset: 0,
        }
    }
}

/// A line that was read but was not an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedLine {
    /// Which generation it came from.
    pub generation: u64,
    /// Byte offset of the line's first byte within that generation.
    pub offset: u64,
    /// Why it was skipped.
    pub reason: EventError,
}

/// The result of draining one generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drain {
    /// The events read, in file order.
    pub events: Vec<Event>,
    /// Where to resume. Never mid-record.
    pub cursor: Cursor,
    /// Lines that were not events. Kept rather than dropped: a reader that
    /// silently discards malformed input reports a quiet stream and a broken
    /// one identically.
    pub skipped: Vec<SkippedLine>,
    /// Whether this generation was read to a stable end — nothing but a
    /// possible partial record remains. DR-001 permits advancing only from
    /// here.
    pub drained: bool,
}

/// One generation's backing file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationSource {
    /// The generation number. Ordering is numeric, not filesystem order.
    pub generation: u64,
    /// Where its records live.
    pub path: PathBuf,
}

/// A session's event stream, as an ordered set of generations.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventLog {
    sources: Vec<GenerationSource>,
}

impl EventLog {
    /// The generations of the session directory at `dir`.
    ///
    /// Today that is exactly one: the bash-era `events.jsonl`, read as
    /// generation 0. DR-001 defers the written multi-generation layout — and
    /// its legacy-read and migration ownership — to the flip commit, so
    /// inventing a filename pattern here would be inventing the half of the DR
    /// that is deliberately unwritten.
    #[must_use]
    pub fn discover(dir: &Path) -> Self {
        Self {
            sources: vec![GenerationSource {
                generation: 0,
                path: dir.join(LEGACY_CONTAINER),
            }],
        }
    }

    /// Build a log over the generations given, ordered by generation number.
    #[must_use]
    pub fn from_sources<I: IntoIterator<Item = GenerationSource>>(sources: I) -> Self {
        let mut sources: Vec<_> = sources.into_iter().collect();
        sources.sort_by_key(|source| source.generation);
        Self { sources }
    }

    /// The generations this log spans, lowest first.
    #[must_use]
    pub fn sources(&self) -> &[GenerationSource] {
        &self.sources
    }

    /// Read one generation from `cursor`, consuming only complete records.
    ///
    /// This is the "drain a stable opened generation" half of DR-001: the file
    /// is opened once and read to the end it had at that moment. A trailing
    /// record without its newline is left unconsumed — the cursor would
    /// otherwise land mid-record, and a mid-record offset is exactly the state
    /// the generation+offset cursor exists to make impossible.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] when the generation's file exists
    /// and cannot be opened or read, and [`io::ErrorKind::InvalidData`] when
    /// `cursor.offset` is past the file's end — a container truncated, rotated
    /// or replaced beneath the reader.
    ///
    /// ONE absence is not an error: a missing file under a FRESH cursor
    /// (`Cursor::default`), which SC-519 rules a quiet empty stream because a
    /// session may not have written its first event yet. A missing file under a
    /// cursor that has been somewhere IS an error — the cursor is evidence the
    /// stream existed, so its disappearance is loss and SC-509b says loss must
    /// be visible.
    pub fn drain(&self, cursor: Cursor) -> io::Result<Drain> {
        let Some(source) = self
            .sources
            .iter()
            .find(|source| source.generation == cursor.generation)
        else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no generation {}", cursor.generation),
            ));
        };

        // SC-519 blesses ONE case: a fresh session that has not written its
        // first event. A fresh read is what a default cursor means, so the
        // tolerance is scoped to it. A cursor that has been somewhere is
        // EVIDENCE the stream existed — if the file is gone now, history was
        // lost, and answering "quiet" would render that loss identically to a
        // session that never spoke (SC-509b's exact prohibition).
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the event-log read — see clippy.toml"
        )]
        let mut file = match File::open(&source.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound && cursor == Cursor::default() => {
                return Ok(Drain {
                    events: Vec::new(),
                    cursor,
                    skipped: Vec::new(),
                    drained: true,
                });
            }
            Err(err) => return Err(err),
        };

        // The same evidence, the other way round: an offset past the end means
        // the container was truncated, rotated or replaced under the reader.
        // Seeking past EOF is legal and reads zero bytes, so without this the
        // answer would be a serene "nothing new" forever.
        let length = file.metadata()?.len();
        if cursor.offset > length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "generation {} cursor at {} is past its {length}-byte end: history was lost",
                    cursor.generation, cursor.offset
                ),
            ));
        }

        file.seek(SeekFrom::Start(cursor.offset))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        let mut events = Vec::new();
        let mut skipped = Vec::new();
        let mut consumed: u64 = 0;
        for record in bytes.split_inclusive(|byte| *byte == b'\n') {
            if record.last() != Some(&b'\n') {
                // A partial trailing record. Not consumed, not reported: the
                // writer has not finished it yet.
                break;
            }
            let offset = cursor.offset.saturating_add(consumed);
            consumed = consumed.saturating_add(record.len() as u64);
            let Ok(text) = std::str::from_utf8(record) else {
                // Not text, so not a JSON object either. Reported, not dropped.
                skipped.push(SkippedLine {
                    generation: cursor.generation,
                    offset,
                    reason: EventError::NotAnObject,
                });
                continue;
            };
            let line = text.trim_end_matches(['\n', '\r']);
            if line.trim().is_empty() {
                continue;
            }
            match Event::parse_line(line) {
                Ok(event) => events.push(event),
                Err(reason) => skipped.push(SkippedLine {
                    generation: cursor.generation,
                    offset,
                    reason,
                }),
            }
        }

        let partial = consumed < bytes.len() as u64;
        Ok(Drain {
            events,
            cursor: Cursor {
                generation: cursor.generation,
                offset: cursor.offset.saturating_add(consumed),
            },
            skipped,
            drained: !partial,
        })
    }

    /// The cursor that follows `drain`'s, advancing a generation when — and
    /// only when — the current one is finished and a later one exists.
    ///
    /// DR-001, binding condition: "a reader drains a stable opened generation
    /// before advancing". Returning the same cursor is the correct answer for a
    /// generation that is still being written to; the reader comes back to it.
    #[must_use]
    pub fn next_cursor(&self, drain: &Drain) -> Cursor {
        if !drain.drained {
            return drain.cursor;
        }
        self.sources
            .iter()
            .find(|source| source.generation > drain.cursor.generation)
            .map_or(drain.cursor, |next| Cursor::start_of(next.generation))
    }

    /// Read every generation from `cursor` to the end of the stream.
    ///
    /// The whole-history read `list --json` needs: it starts at a cursor,
    /// drains each generation in turn, and stops at the first one that is not
    /// finished — never skipping ahead of a generation still being written.
    ///
    /// # Errors
    ///
    /// As [`EventLog::drain`].
    pub fn drain_all(&self, cursor: Cursor) -> io::Result<Drain> {
        let plan = self.traversal_from(cursor.generation)?;

        let mut events = Vec::new();
        let mut skipped = Vec::new();
        let mut last = cursor;
        let mut drained = true;

        for (step, generation) in plan.iter().enumerate() {
            // The caller's cursor addresses the FIRST generation only; every
            // later one is read from its start.
            let at = if step == 0 {
                cursor
            } else {
                Cursor::start_of(*generation)
            };
            let mut drain = self.drain(at)?;
            events.append(&mut drain.events);
            skipped.append(&mut drain.skipped);
            last = drain.cursor;
            drained = drain.drained;
            // DR-001, the binding condition: never advance past a generation
            // that is still being written to. The reader comes back to it.
            if !drained {
                break;
            }
        }

        Ok(Drain {
            events,
            cursor: last,
            skipped,
            drained,
        })
    }

    /// The generations [`EventLog::drain_all`] will visit, in order, starting at
    /// `generation`.
    ///
    /// **This is the bound.** DR-001's condition is that a reader drains a
    /// stable generation BEFORE advancing, so an advance loop that fails to
    /// converge violates the row rather than merely hanging — and a loop that
    /// terminates only when cursor arithmetic stops changing can fail to
    /// converge. Traversal is therefore planned from the DISCOVERED, SORTED
    /// generation set, never computed: the plan is finite because the set is,
    /// whatever the arithmetic says.
    ///
    /// Two properties the plan carries, both asserted in tests rather than
    /// assumed:
    ///
    /// * generation ids are NOT assumed consecutive — a retention policy that
    ///   drops the middle of a range leaves gaps, and `+1` would walk off into
    ///   generations that do not exist;
    /// * each generation appears AT MOST ONCE, so no source can be read twice.
    ///   Duplicate ids in the source list collapse to one visit, which is what
    ///   [`EventLog::drain`] already did by resolving a generation to its first
    ///   matching source — the traversal preserves that rather than changing it.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::NotFound`] when `generation` is not in the set — the
    /// same answer, with the same message, that draining it directly gives.
    fn traversal_from(&self, generation: u64) -> io::Result<Vec<u64>> {
        let start = self
            .sources
            .iter()
            .position(|source| source.generation == generation)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no generation {generation}"),
                )
            })?;

        let mut plan: Vec<u64> = Vec::new();
        for source in &self.sources[start..] {
            if plan.last() != Some(&source.generation) {
                plan.push(source.generation);
            }
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real directories; the boundary is about \
                  what PRODUCT code may reach"
    )]

    use super::{
        AlertMeaning, Cursor, Event, EventError, EventLog, GenerationSource, Identity, KNOWN_KEYS,
        RefMeaning, RoutingMember, alert_class,
    };
    use crate::attention::Reason;
    use crate::time::Timestamp;
    use std::fs;
    use std::path::PathBuf;

    const DONE: &str = r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"done"}"#;

    /// A scratch directory that removes itself. Enough for a read-only test:
    /// the process id and a counter keep concurrent tests apart.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ae-events-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a scratch dir");
            Self(dir)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, contents).expect("writing a fixture");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sc_510a_the_three_required_keys_are_required() {
        let event = Event::parse_line(DONE).expect("the required trio is enough");
        assert_eq!(event.ts, Timestamp::from_epoch(1_779_175_785));
        assert_eq!(event.actor, "claude:lead");
        assert_eq!(event.action, "done");

        for (line, missing) in [
            (r#"{"actor":"a","action":"done"}"#, "ts"),
            (r#"{"ts":"2026-05-19T07:29:45Z","action":"done"}"#, "actor"),
            (r#"{"ts":"2026-05-19T07:29:45Z","actor":"a"}"#, "action"),
        ] {
            assert_eq!(
                Event::parse_line(line),
                Err(EventError::MissingKey(missing)),
                "{line}"
            );
        }
    }

    #[test]
    fn sc_510a_a_ts_that_is_not_the_documented_spelling_is_not_an_event() {
        let line = r#"{"ts":"19 May 2026","actor":"a","action":"done"}"#;
        assert_eq!(
            Event::parse_line(line),
            Err(EventError::UnreadableTimestamp("19 May 2026".to_owned()))
        );
    }

    #[test]
    fn sc_510b_an_empty_optional_key_is_the_same_absence_as_no_key() {
        // The row: target/ref/summary never appear as empty strings.
        let with_empties = concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"done","#,
            r#""target":"","ref":"","summary":""}"#
        );
        let event = Event::parse_line(with_empties).expect("empties are absences, not errors");
        assert_eq!(event.target, None);
        assert_eq!(event.reference, None);
        assert_eq!(event.summary, None);
        assert_eq!(Event::parse_line(DONE).expect("parses"), event);
    }

    #[test]
    fn sc_510a_a_required_key_present_but_empty_is_missing() {
        let line = r#"{"ts":"2026-05-19T07:29:45Z","actor":"","action":"done"}"#;
        assert_eq!(
            Event::parse_line(line),
            Err(EventError::MissingKey("actor"))
        );
    }

    #[test]
    fn sc_510c_ref_means_what_the_action_says_it_means() {
        let cases = [
            ("ask", RefMeaning::RequestId("r-1")),
            ("review", RefMeaning::RequestId("r-1")),
            ("reply", RefMeaning::RequestId("r-1")),
            ("memo", RefMeaning::MemoTopic("r-1")),
            ("recover", RefMeaning::CapturedSessionId("r-1")),
            // SC-510c as AMENDED: `state` carries the declared work state.
            ("state", RefMeaning::DeclaredState("r-1")),
            // "Other actions — USUALLY absent"; a value that turns up anyway
            // has no meaning the table defines, so neither do we.
            ("nudge", RefMeaning::Undefined),
        ];
        for (action, expected) in cases {
            let line = format!(
                r#"{{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"{action}","ref":"r-1"}}"#
            );
            let event = Event::parse_line(&line).expect("parses");
            assert_eq!(event.ref_meaning(), expected, "{action}");
        }
    }

    #[test]
    fn sc_510c_no_ref_means_no_meaning_even_for_an_action_that_could_carry_one() {
        let line = r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"ask"}"#;
        let event = Event::parse_line(line).expect("parses");
        assert_eq!(event.ref_meaning(), RefMeaning::Undefined);
    }

    #[test]
    fn sc_511a_and_b_routing_keys_are_read_and_preferred() {
        let line = concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"send","#,
            r#""target":"codex:coworker","actor_slot":"main","actor_session":"my-feature","#,
            r#""target_slot":"worker.0","target_session":"my-feature"}"#
        );
        let event = Event::parse_line(line).expect("parses");
        assert_eq!(
            event.actor_identity(),
            Identity::Routed {
                slot: "main",
                session: "my-feature"
            }
        );
        assert_eq!(
            event.target_identity(),
            Some(Identity::Routed {
                slot: "worker.0",
                session: "my-feature"
            })
        );
    }

    #[test]
    fn sc_511b_falls_back_to_the_display_name_when_the_key_is_absent() {
        // "An event without those fields ... pairing falls back to the display
        // name" — every pre-SC-511a event in an existing log looks like this.
        let event = Event::parse_line(DONE).expect("parses");
        assert_eq!(event.actor_identity(), Identity::Display("claude:lead"));
        assert_eq!(event.target_identity(), None);
    }

    #[test]
    fn sc_405j_half_a_routing_key_is_unassociated_not_a_display_name() {
        // This test previously asserted Display, which is the bug it was named
        // for: an event that says "I am routed" and then fails to say where
        // must not be answerable by a name a display-only counterpart also
        // carries. Either half alone, on either side.
        for keys in [r#""actor_slot":"main""#, r#""actor_session":"my-feature""#] {
            let line = format!(
                r#"{{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"send","target":"codex:coworker",{keys}}}"#
            );
            let event = Event::parse_line(&line).expect("parses");
            assert_eq!(event.actor_identity(), Identity::Unassociated, "{keys}");
        }

        for keys in [
            r#""target_slot":"worker.0""#,
            r#""target_session":"my-feature""#,
        ] {
            let line = format!(
                r#"{{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"send","target":"codex:coworker",{keys}}}"#
            );
            let event = Event::parse_line(&line).expect("parses");
            assert_eq!(
                event.target_identity(),
                Some(Identity::Unassociated),
                "{keys}"
            );
        }
    }

    #[test]
    fn sc_405j_an_empty_routing_member_is_present_but_invalid_not_absent() {
        // The three states, told apart. An empty member is NOT the absent one.
        let event = Event::parse_line(concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"send","#,
            r#""target":"codex:coworker","actor_slot":"","target_session":"live"}"#
        ))
        .expect("an empty routing key is not a parse failure");

        assert_eq!(event.actor_slot, RoutingMember::Invalid);
        assert_eq!(event.actor_session, RoutingMember::Absent);
        assert_eq!(
            event.target_session,
            RoutingMember::Value("live".to_owned())
        );
        assert!(event.actor_slot.is_present(), "empty is still PRESENT");
        assert_eq!(event.actor_slot.value(), None, "but it carries no value");

        // ...and the identity carrying it names nobody, rather than falling
        // back to the display name a pairing partner might also carry.
        assert_eq!(event.actor_identity(), Identity::Unassociated);
        assert_eq!(event.target_identity(), Some(Identity::Unassociated));
    }

    #[test]
    fn a_routing_member_answers_both_of_its_questions_in_both_directions() {
        // Each accessor asserted BOTH ways. Checking only the negative
        // direction leaves `value()` replaceable by None and `is_present()` by
        // true, and the suite would not notice either.
        let carried = RoutingMember::Value("worker.0".to_owned());
        assert_eq!(carried.value(), Some("worker.0"));
        assert!(carried.is_present());

        assert_eq!(RoutingMember::Invalid.value(), None);
        assert!(
            RoutingMember::Invalid.is_present(),
            "empty is present — that is the whole point of the state"
        );

        assert_eq!(RoutingMember::Absent.value(), None);
        assert!(
            !RoutingMember::Absent.is_present(),
            "and absent is the ONLY state that is not present"
        );

        assert_eq!(RoutingMember::default(), RoutingMember::Absent);
    }

    #[test]
    fn a_routing_member_read_off_a_record_carries_its_value_through() {
        let event = Event::parse_line(concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"send","target":"b","#,
            r#""actor_slot":"main","actor_session":"live","#,
            r#""target_slot":"worker.0","target_session":"live"}"#
        ))
        .expect("parses");
        assert_eq!(event.actor_slot.value(), Some("main"));
        assert_eq!(event.actor_session.value(), Some("live"));
        assert_eq!(event.target_slot.value(), Some("worker.0"));
        assert_eq!(event.target_session.value(), Some("live"));
        assert_eq!(
            event.actor_identity(),
            Identity::Routed {
                slot: "main",
                session: "live"
            }
        );
    }

    #[test]
    fn sc_405j_the_three_way_presence_discriminator() {
        // The discriminator the amended row requires, in one place: keys
        // ABSENT / ONE present-empty member / ALL routing members
        // present-empty. Only the first falls back to a display name, and the
        // two empty shapes must not be answerable by collapsing them into it.
        let base = r#""ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"send","target":"codex:coworker""#;

        // 1. Structurally ABSENT: the legacy fallback pre-SC-511a records need.
        let absent = Event::parse_line(&format!("{{{base}}}")).expect("parses");
        assert_eq!(absent.actor_slot, RoutingMember::Absent);
        assert_eq!(absent.actor_identity(), Identity::Display("claude:lead"));
        assert_eq!(
            absent.target_identity(),
            Some(Identity::Display("codex:coworker"))
        );

        // 2. ONE present-empty member, the other structurally absent.
        let one_empty =
            Event::parse_line(&format!(r#"{{{base},"actor_slot":"","target_slot":""}}"#))
                .expect("parses");
        assert_eq!(one_empty.actor_slot, RoutingMember::Invalid);
        assert_eq!(one_empty.actor_session, RoutingMember::Absent);
        assert_eq!(one_empty.actor_identity(), Identity::Unassociated);
        assert_eq!(one_empty.target_identity(), Some(Identity::Unassociated));

        // 3. ALL routing members present and empty.
        let all_empty = Event::parse_line(&format!(
            r#"{{{base},"actor_slot":"","actor_session":"","target_slot":"","target_session":""}}"#
        ))
        .expect("parses");
        for member in [
            &all_empty.actor_slot,
            &all_empty.actor_session,
            &all_empty.target_slot,
            &all_empty.target_session,
        ] {
            assert_eq!(*member, RoutingMember::Invalid);
            assert!(member.is_present(), "present, whatever the value");
        }
        assert_eq!(all_empty.actor_identity(), Identity::Unassociated);
        assert_eq!(all_empty.target_identity(), Some(Identity::Unassociated));

        // And the three shapes are genuinely three, not two wearing a disguise.
        assert_ne!(absent.actor_identity(), one_empty.actor_identity());
        assert_eq!(one_empty.actor_identity(), all_empty.actor_identity());
        assert_ne!(absent.actor_slot, one_empty.actor_slot);
    }

    #[test]
    fn sc_405j_an_empty_member_is_not_a_record_error() {
        // The ruling is explicit: do NOT record-skip. The fact stays countable
        // and the rest of the event is still true; only the identity is lost.
        let event = Event::parse_line(concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"ask","#,
            r#""target":"codex:coworker","ref":"ae-1","actor_slot":"","actor_session":""}"#
        ))
        .expect("still an event");
        assert_eq!(event.action, "ask");
        assert_eq!(event.ref_meaning(), RefMeaning::RequestId("ae-1"));
        assert_eq!(event.actor_identity(), Identity::Unassociated);
    }

    #[test]
    fn sc_510b_and_sc_405j_diverge_on_an_empty_value() {
        // The spillover guard. The SC-510b trio normalises empty to absent
        // because its row says those keys never appear empty; the routing keys
        // deliberately do NOT share that rule. One line, both behaviours, so a
        // future edit cannot quietly re-unify them.
        let event = Event::parse_line(concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"send","#,
            r#""target":"","ref":"","summary":"","actor_slot":"","actor_session":""}"#
        ))
        .expect("parses");

        // Trio: empty reads as the absence the row describes.
        assert_eq!(event.target, None);
        assert_eq!(event.reference, None);
        assert_eq!(event.summary, None);

        // Routing keys: empty is present-but-invalid, and stays distinguishable.
        assert_eq!(event.actor_slot, RoutingMember::Invalid);
        assert_eq!(event.actor_session, RoutingMember::Invalid);
        assert_ne!(
            event.actor_slot,
            RoutingMember::Absent,
            "empty must not collapse into absent for a routing key"
        );
        assert_eq!(event.actor_identity(), Identity::Unassociated);
    }

    #[test]
    fn sc_405j_unassociated_is_not_even_equal_to_the_absence_of_a_target() {
        // A target that is present but unroutable is not the same fact as no
        // target at all, and the two must stay distinguishable.
        let unroutable = Event::parse_line(concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"send","#,
            r#""target":"b","target_slot":"worker.0"}"#
        ))
        .expect("parses");
        assert_eq!(unroutable.target_identity(), Some(Identity::Unassociated));

        let targetless =
            Event::parse_line(r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"done"}"#)
                .expect("parses");
        assert_eq!(targetless.target_identity(), None);
    }

    #[test]
    fn sc_509b_a_known_key_of_the_wrong_type_is_loss_not_absence() {
        // `"ref": 7` is a writer saying something the reader cannot take. It is
        // NOT the same as omitting `ref`, and blanking it would hide the loss.
        for (line, key) in [
            (
                r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"ask","ref":7}"#,
                "ref",
            ),
            (
                r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"send","target":12}"#,
                "target",
            ),
            (
                r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"chat","summary":{"a":1}}"#,
                "summary",
            ),
            (
                r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"send","actor_slot":0}"#,
                "actor_slot",
            ),
            (
                r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"send","actor_session":[1]}"#,
                "actor_session",
            ),
            (
                r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"send","target_slot":true}"#,
                "target_slot",
            ),
            (
                r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"send","target_session":null}"#,
                "target_session",
            ),
        ] {
            assert_eq!(
                Event::parse_line(line),
                Err(EventError::WrongType(key)),
                "{line}"
            );
        }
    }

    #[test]
    fn sc_509b_a_required_key_of_the_wrong_type_is_loss_too() {
        for (line, key) in [
            (r#"{"ts":7,"actor":"a","action":"done"}"#, "ts"),
            (
                r#"{"ts":"2026-05-19T07:29:45Z","actor":[],"action":"done"}"#,
                "actor",
            ),
            (
                r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":{}}"#,
                "action",
            ),
        ] {
            assert_eq!(
                Event::parse_line(line),
                Err(EventError::WrongType(key)),
                "{line}"
            );
        }
    }

    #[test]
    fn sc_510e_a_duplicated_known_key_makes_the_whole_record_malformed() {
        // Two actors, and no row says which one the writer meant. Taking either
        // is fabrication; taking the first makes the answer depend on order.
        let line = concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"done","#,
            r#""actor":"codex:coworker"}"#
        );
        assert_eq!(
            Event::parse_line(line),
            Err(EventError::DuplicateKey("actor"))
        );
    }

    #[test]
    fn a_duplicated_key_is_refused_before_its_type_is_judged() {
        // A valid actor followed by a wrong-typed one. Reading first-wins would
        // report a perfectly good event and never see the second; reading
        // last-wins would report WrongType. Both answers pick a winner, so
        // neither is available — the duplicate itself is the finding.
        let line = concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"claude:lead","action":"done","#,
            r#""actor":7}"#
        );
        assert_eq!(
            Event::parse_line(line),
            Err(EventError::DuplicateKey("actor"))
        );
    }

    #[test]
    fn a_duplicated_key_gives_the_same_answer_whichever_order_it_arrives_in() {
        // The order-reversal pin: this is the property first-wins lacked.
        let forward = r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"done","actor":"b"}"#;
        let reverse = r#"{"ts":"2026-05-19T07:29:45Z","actor":"b","action":"done","actor":"a"}"#;
        assert_eq!(Event::parse_line(forward), Event::parse_line(reverse));
        assert_eq!(
            Event::parse_line(forward),
            Err(EventError::DuplicateKey("actor"))
        );
    }

    /// The ten key names SC-510a/b/c and SC-511a document, written out here
    /// INDEPENDENTLY of the production list.
    ///
    /// The point is the independence. A test that iterates `KNOWN_KEYS` and
    /// checks a rejection which itself consults `KNOWN_KEYS` proves only that
    /// the constant equals itself: drop a documented key from production and
    /// nothing is generated for it; rename one to a bogus name of any length
    /// and both sides move together. Const membership has no mutant either, so
    /// the mutation lane cannot see the gap. This literal is the second opinion
    /// that makes the pin mean something — the same lesson as counting fixtures
    /// instead of naming them.
    ///
    /// **The class, for whoever adds the eleventh key.** This suite shipped
    /// three tests that looked like constraints and were not: one asserted the
    /// fixture COUNT where it meant the fixture NAMES, so deleting one fixture
    /// and adding another passed; one asserted an accessor only in the
    /// direction where it returns nothing, so replacing its body with that
    /// nothing passed; and this one iterated the production key list to check a
    /// rejection that consulted the production key list, so a bogus name
    /// substituted on both sides passed. One signature underneath all three:
    /// **the expected value and the actual value came from the same place.** A
    /// test shaped that way cannot fail, and it is worse than no test, because
    /// it reports coverage of the thing it does not check. Mutation testing
    /// caught the first two and structurally CANNOT catch this one — const
    /// membership has no mutant to generate — so the only defence here is a
    /// second, independent statement of the expectation, which is what the list
    /// below is. So: adding a key means adding it in BOTH places, and if that
    /// ever feels like pointless duplication, that feeling is the bug. The
    /// duplication is the test.
    const DOCUMENTED_EVENT_KEYS: [&str; 10] = [
        "ts",
        "actor",
        "action",
        "target",
        "ref",
        "summary",
        "actor_slot",
        "actor_session",
        "target_slot",
        "target_session",
    ];

    #[test]
    fn the_policed_key_set_is_exactly_the_documented_one() {
        let mut production = KNOWN_KEYS.to_vec();
        production.sort_unstable();
        let mut documented = DOCUMENTED_EVENT_KEYS.to_vec();
        documented.sort_unstable();
        assert_eq!(
            production, documented,
            "KNOWN_KEYS has drifted from the keys SC-510/SC-511 document"
        );
    }

    #[test]
    fn every_key_the_schema_documents_is_policed_for_duplicates() {
        // Driven from the DOCUMENTED literal, never from the production list:
        // a key the schema defines but production forgot must fail HERE, and it
        // can only do that if the loop knows about it independently.
        for key in DOCUMENTED_EVENT_KEYS {
            let line = format!(
                r#"{{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"done","{key}":"x","{key}":"y"}}"#
            );
            assert_eq!(
                Event::parse_line(&line),
                Err(EventError::DuplicateKey(key)),
                "{key} is documented but not policed"
            );
        }
    }

    #[test]
    fn sc_510f_a_duplicated_unknown_key_stays_inert() {
        // The other side of the line. This reader never reads the value, so it
        // cannot fabricate one — and SC-511b says step over what you do not
        // understand, however many times it appears.
        let line = concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"done","#,
            r#""invented":1,"invented":2,"invented":{"deep":true}}"#
        );
        let event = Event::parse_line(line).expect("an unknown key is not this reader's business");
        assert_eq!(event.actor, "a");
    }

    #[test]
    fn sc_510f_inertness_survives_the_order_reversal_too() {
        // SC-510f carries SC-510e's discriminator: the same duplicate pair with
        // its members swapped must give the same answer. For an unknown key the
        // answer is "fine, twice" — and it has to be fine in both directions,
        // or the reader is reading a value it claims to ignore.
        let forward =
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"done","x":1,"x":"two"}"#;
        let reverse =
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"done","x":"two","x":1}"#;
        assert_eq!(Event::parse_line(forward), Event::parse_line(reverse));
        assert!(Event::parse_line(forward).is_ok());
    }

    #[test]
    fn sc_520_a_duplicated_known_key_degrades_through_the_skipped_path() {
        // End to end: the malformed record is skipped, counted with its
        // position, and the good records around it still arrive.
        let scratch = Scratch::new("dupkey");
        let duplicated = r#"{"ts":"2026-05-19T07:29:46Z","actor":"a","action":"done","actor":"b"}"#;
        scratch.write("events.jsonl", &format!("{DONE}\n{duplicated}\n{DONE}\n"));
        let drain = EventLog::discover(&scratch.0)
            .drain(Cursor::default())
            .expect("reads");
        assert_eq!(drain.events.len(), 2);
        assert_eq!(drain.skipped.len(), 1);
        assert_eq!(drain.skipped[0].reason, EventError::DuplicateKey("actor"));
        assert_eq!(drain.skipped[0].offset, DONE.len() as u64 + 1);
    }

    #[test]
    fn sc_511b_an_unknown_key_of_any_type_is_still_ignored() {
        // The distinction the rows draw: a key the schema DEFINES must be the
        // right type, a key it does not define is none of the reader's business.
        let line = concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"done","#,
            r#""invented":7,"also":{"deep":[1]},"third":null}"#
        );
        assert!(Event::parse_line(line).is_ok());
    }

    #[test]
    fn sc_511c_an_unknown_key_is_data_to_step_over() {
        let line = concat!(
            r#"{"ts":"2026-05-19T07:29:45Z","actor":"a","action":"done","#,
            r#""invented_later":{"deep":[1,2]},"also_new":7}"#
        );
        let event = Event::parse_line(line).expect("an additive key is not an error");
        assert_eq!(event.action, "done");
    }

    #[test]
    fn every_rejection_says_which_line_problem_it_found() {
        // SkippedLine carries these to a human; a Display that returned nothing
        // would turn "malformed at byte 12" into silence.
        let missing =
            Event::parse_line(r#"{"actor":"a","action":"done"}"#).expect_err("ts is required");
        assert_eq!(missing.to_string(), "missing required key: ts");

        let bad_ts = Event::parse_line(r#"{"ts":"nope","actor":"a","action":"done"}"#)
            .expect_err("ts must be the documented spelling");
        assert_eq!(bad_ts.to_string(), "unreadable ts: nope");

        assert_eq!(
            Event::parse_line("[1]")
                .expect_err("not an object")
                .to_string(),
            "not a json object"
        );
        assert!(
            Event::parse_line("{oops")
                .expect_err("not json")
                .to_string()
                .contains("invalid json"),
            "a syntax error keeps the parser's own message"
        );
    }

    #[test]
    fn a_json_line_that_is_not_an_object_is_not_an_event() {
        assert_eq!(Event::parse_line("[1,2,3]"), Err(EventError::NotAnObject));
        assert!(matches!(
            Event::parse_line("{not json"),
            Err(EventError::NotJson(_))
        ));
    }

    #[test]
    fn dr_001_a_drain_reads_the_legacy_container_as_generation_zero() {
        let scratch = Scratch::new("gen0");
        scratch.write("events.jsonl", &format!("{DONE}\n{DONE}\n"));
        let log = EventLog::discover(&scratch.0);
        assert_eq!(log.sources().len(), 1);
        assert_eq!(log.sources()[0].generation, 0);

        let drain = log.drain(Cursor::default()).expect("the container reads");
        assert_eq!(drain.events.len(), 2);
        assert!(drain.drained, "a file read to EOF is drained");
        assert_eq!(drain.cursor.generation, 0);
        assert_eq!(drain.cursor.offset, (DONE.len() as u64 + 1) * 2);
    }

    #[test]
    fn dr_001_a_cursor_resumes_where_it_left_off_and_reads_only_what_is_new() {
        let scratch = Scratch::new("resume");
        let path = scratch.write("events.jsonl", &format!("{DONE}\n"));
        let log = EventLog::discover(&scratch.0);

        let first = log.drain(Cursor::default()).expect("first read");
        assert_eq!(first.events.len(), 1);

        // Append, as a writer does within a generation.
        fs::write(&path, format!("{DONE}\n{DONE}\n")).expect("append");
        let second = log.drain(first.cursor).expect("second read");
        assert_eq!(second.events.len(), 1, "only the new record");
        assert_eq!(second.cursor.offset, (DONE.len() as u64 + 1) * 2);
    }

    #[test]
    fn dr_001_a_partial_trailing_record_is_not_consumed() {
        // The cursor must never land mid-record: that is what generation+offset
        // exists to prevent.
        let scratch = Scratch::new("partial");
        let path = scratch.write("events.jsonl", &format!("{DONE}\n{{\"ts\":\"2026"));
        let log = EventLog::discover(&scratch.0);

        let drain = log.drain(Cursor::default()).expect("reads");
        assert_eq!(drain.events.len(), 1);
        assert!(
            drain.skipped.is_empty(),
            "an unfinished write is not an error"
        );
        assert_eq!(drain.cursor.offset, DONE.len() as u64 + 1);
        assert!(
            !drain.drained,
            "a partial tail means the generation is not done"
        );

        // When the writer finishes the line, the same cursor picks it up whole.
        fs::write(&path, format!("{DONE}\n{DONE}\n")).expect("complete the record");
        let rest = log.drain(drain.cursor).expect("reads");
        assert_eq!(rest.events.len(), 1);
        assert!(rest.drained);
    }

    #[test]
    fn dr_001_advancing_requires_a_drained_generation() {
        let scratch = Scratch::new("advance");
        let g0 = scratch.write("g0", &format!("{DONE}\n{{\"partial"));
        let g1 = scratch.write("g1", &format!("{DONE}\n"));
        let log = EventLog::from_sources([
            GenerationSource {
                generation: 1,
                path: g1,
            },
            GenerationSource {
                generation: 0,
                path: g0.clone(),
            },
        ]);
        assert_eq!(
            log.sources()
                .iter()
                .map(|s| s.generation)
                .collect::<Vec<_>>(),
            vec![0, 1],
            "generations are ordered numerically, not by insertion"
        );

        let drain = log.drain(Cursor::default()).expect("reads generation 0");
        assert!(!drain.drained);
        assert_eq!(
            log.next_cursor(&drain),
            drain.cursor,
            "an undrained generation is not left behind"
        );

        fs::write(&g0, format!("{DONE}\n")).expect("the writer finishes generation 0");
        let drain = log.drain(Cursor::default()).expect("reads generation 0");
        assert!(drain.drained);
        assert_eq!(log.next_cursor(&drain), Cursor::start_of(1));
    }

    #[test]
    fn dr_001_the_last_generation_does_not_advance_past_itself() {
        let scratch = Scratch::new("last");
        let path = scratch.write("g0", &format!("{DONE}\n"));
        let log = EventLog::from_sources([GenerationSource {
            generation: 0,
            path,
        }]);
        let drain = log.drain(Cursor::default()).expect("reads");
        assert!(drain.drained);
        assert_eq!(
            log.next_cursor(&drain),
            drain.cursor,
            "there is nowhere to advance to, and the cursor stays resumable"
        );
    }

    /// Build a log of `generations`, each holding one `done` event, plus an
    /// optional unfinished tail on the generation named by `partial`.
    fn multi_generation(
        tag: &str,
        generations: &[u64],
        partial: Option<u64>,
    ) -> (Scratch, EventLog) {
        let scratch = Scratch::new(tag);
        let sources = generations
            .iter()
            .map(|generation| {
                let body = if partial == Some(*generation) {
                    format!("{DONE}\n{{\"ts\":\"2026")
                } else {
                    format!("{DONE}\n")
                };
                GenerationSource {
                    generation: *generation,
                    path: scratch.write(&format!("g{generation}"), &body),
                }
            })
            .collect::<Vec<_>>();
        let log = EventLog::from_sources(sources);
        (scratch, log)
    }

    #[test]
    fn dr_001_the_traversal_plan_never_assumes_consecutive_generation_ids() {
        // Retention drops the middle of a range, so the set is sparse. A `+1`
        // walk would step into generations that do not exist.
        let (_scratch, log) = multi_generation("sparse", &[0, 7, 41], None);
        assert_eq!(log.traversal_from(0).expect("plan"), vec![0, 7, 41]);

        let drain = log.drain_all(Cursor::default()).expect("reads all three");
        assert_eq!(drain.events.len(), 3);
        assert_eq!(drain.cursor.generation, 41);
        assert!(drain.drained);
    }

    #[test]
    fn dr_001_a_start_cursor_inside_a_later_generation_starts_there() {
        let (_scratch, log) = multi_generation("startlater", &[0, 7, 41], None);
        assert_eq!(log.traversal_from(7).expect("plan"), vec![7, 41]);

        let drain = log
            .drain_all(Cursor::start_of(7))
            .expect("reads from the middle onward");
        assert_eq!(drain.events.len(), 2, "generation 0 is behind the cursor");
        assert_eq!(drain.cursor.generation, 41);

        // And the last generation is a plan of one.
        assert_eq!(log.traversal_from(41).expect("plan"), vec![41]);
        assert_eq!(
            log.drain_all(Cursor::start_of(41))
                .expect("reads")
                .events
                .len(),
            1
        );
    }

    #[test]
    fn dr_001_an_unfinished_middle_generation_stops_the_traversal_dead() {
        // The binding condition, at the hardest point: the generation still
        // being written is in the MIDDLE, so a reader that advanced past it
        // would look correct on the totals while silently reordering history.
        // It must stop, and never visit what comes after.
        let (_scratch, log) = multi_generation("middlepartial", &[0, 7, 41], Some(7));
        assert_eq!(
            log.traversal_from(0).expect("plan"),
            vec![0, 7, 41],
            "the PLAN still spans the set"
        );

        let drain = log.drain_all(Cursor::default()).expect("reads");
        assert_eq!(
            drain.events.len(),
            2,
            "generation 0 whole, generation 7's complete record, nothing from 41"
        );
        assert!(!drain.drained, "it stopped on an unfinished generation");
        assert_eq!(
            drain.cursor.generation, 7,
            "and the cursor stays in the generation it must come back to"
        );
    }

    #[test]
    fn dr_001_traversal_terminates_on_a_singleton_and_refuses_an_empty_set() {
        let (_scratch, single) = multi_generation("singleton", &[3], None);
        assert_eq!(single.traversal_from(3).expect("plan"), vec![3]);
        let drain = single.drain_all(Cursor::start_of(3)).expect("reads");
        assert_eq!(drain.events.len(), 1);
        assert!(drain.drained);

        // An empty log has no generation to start from, and says so rather than
        // looping over nothing or answering a serene empty.
        let empty = EventLog::from_sources([]);
        let err = empty.traversal_from(0).expect_err("nothing to traverse");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            empty.drain_all(Cursor::default()).expect_err("same").kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn dr_001_every_generation_is_visited_at_most_once() {
        // The bounded-call-count pin. The plan IS the call list — one drain per
        // entry — so asserting it holds no repeats asserts no source is read
        // twice.
        let (_scratch, log) = multi_generation("atmostonce", &[0, 7, 41], None);
        let plan = log.traversal_from(0).expect("plan");
        let mut unique = plan.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(plan.len(), unique.len(), "no generation appears twice");
        assert_eq!(plan.len(), 3, "and the count is bounded by the set");

        // End to end: a source read twice would duplicate its event.
        let drain = log.drain_all(Cursor::default()).expect("reads");
        assert_eq!(drain.events.len(), 3);
    }

    #[test]
    fn dr_001_duplicate_generation_ids_collapse_to_one_visit_as_they_always_did() {
        // Guarding the STOP CONDITION on this rewrite: `drain` has always
        // resolved a generation to its FIRST matching source, so a duplicate id
        // was never read. The traversal preserves that rather than changing it.
        let scratch = Scratch::new("dupgen");
        let first = scratch.write("g0-first", &format!("{DONE}\n"));
        let second = scratch.write("g0-second", &format!("{DONE}\n{DONE}\n"));
        let log = EventLog::from_sources([
            GenerationSource {
                generation: 0,
                path: first,
            },
            GenerationSource {
                generation: 0,
                path: second,
            },
        ]);
        assert_eq!(log.traversal_from(0).expect("plan"), vec![0], "one visit");
        let drain = log.drain_all(Cursor::default()).expect("reads");
        assert_eq!(
            drain.events.len(),
            1,
            "the second source for the same id is not read, exactly as before"
        );
    }

    #[test]
    fn drain_all_walks_every_generation_in_order() {
        let scratch = Scratch::new("all");
        let g0 = scratch.write("g0", &format!("{DONE}\n"));
        let g1 = scratch.write(
            "g1",
            &format!(
                "{}\n",
                DONE.replace("\"action\":\"done\"", "\"action\":\"spawn\"")
            ),
        );
        let log = EventLog::from_sources([
            GenerationSource {
                generation: 0,
                path: g0,
            },
            GenerationSource {
                generation: 1,
                path: g1,
            },
        ]);
        let drain = log.drain_all(Cursor::default()).expect("reads both");
        assert_eq!(
            drain
                .events
                .iter()
                .map(|e| e.action.as_str())
                .collect::<Vec<_>>(),
            vec!["done", "spawn"]
        );
        assert_eq!(drain.cursor.generation, 1);
    }

    #[test]
    fn a_malformed_line_is_skipped_with_its_position_rather_than_dropped() {
        let scratch = Scratch::new("malformed");
        scratch.write(
            "events.jsonl",
            &format!("{DONE}\nnot json at all\n{DONE}\n"),
        );
        let log = EventLog::discover(&scratch.0);
        let drain = log.drain(Cursor::default()).expect("reads");
        assert_eq!(drain.events.len(), 2, "the good records still arrive");
        assert_eq!(drain.skipped.len(), 1);
        assert_eq!(drain.skipped[0].offset, DONE.len() as u64 + 1);
        assert!(drain.drained);
    }

    #[test]
    fn a_blank_line_is_neither_an_event_nor_a_complaint() {
        let scratch = Scratch::new("blank");
        scratch.write("events.jsonl", &format!("{DONE}\n\n   \n{DONE}\n"));
        let log = EventLog::discover(&scratch.0);
        let drain = log.drain(Cursor::default()).expect("reads");
        assert_eq!(drain.events.len(), 2);
        assert!(drain.skipped.is_empty());
    }

    #[test]
    fn an_empty_container_is_a_quiet_session_not_a_broken_one() {
        // A session that has been created but has emitted nothing yet. It is
        // drained (there is nothing left to read), it has no events, and it is
        // NOT the missing-container case below.
        let scratch = Scratch::new("empty");
        scratch.write("events.jsonl", "");
        let log = EventLog::discover(&scratch.0);
        let drain = log.drain(Cursor::default()).expect("an empty file reads");
        assert!(drain.events.is_empty());
        assert!(drain.skipped.is_empty());
        assert!(drain.drained);
        assert_eq!(drain.cursor, Cursor::default());
    }

    #[test]
    fn sc_519_a_missing_container_is_the_same_quiet_stream_as_an_empty_one() {
        // The seat ruling that reversed this reader's first answer: a session
        // may have no events file until its first write, so ENOENT is quiet,
        // not loss. Only an EXISTING file that will not read degrades.
        let scratch = Scratch::new("absent");
        let log = EventLog::discover(&scratch.0);
        let drain = log.drain(Cursor::default()).expect("ENOENT is tolerated");
        assert!(drain.events.is_empty());
        assert!(drain.skipped.is_empty());
        assert!(drain.drained);
        assert_eq!(drain.cursor, Cursor::default());

        scratch.write("events.jsonl", "");
        let empty = log.drain(Cursor::default()).expect("an empty file reads");
        assert_eq!(drain, empty, "missing and empty are the same answer");
    }

    #[test]
    fn dr_001_a_vanished_container_under_a_used_cursor_is_loud_not_quiet() {
        // SC-519's tolerance is for a FRESH read. A cursor that has been
        // somewhere proves the stream existed, so its disappearance is loss.
        let scratch = Scratch::new("vanished");
        let path = scratch.write("events.jsonl", &format!("{DONE}\n"));
        let log = EventLog::discover(&scratch.0);
        let first = log.drain(Cursor::default()).expect("reads");
        assert_eq!(first.events.len(), 1);

        fs::remove_file(&path).expect("the container goes away");
        let err = log
            .drain(first.cursor)
            .expect_err("a used cursor over a missing file is loss");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);

        // ...and the fresh-read tolerance is untouched.
        assert!(log.drain(Cursor::default()).is_ok());
    }

    #[test]
    fn dr_001_an_offset_past_the_end_is_loud_rather_than_serenely_empty() {
        // Truncated, rotated or replaced under the reader. Seeking past EOF is
        // legal and reads nothing, so silence here would be permanent.
        let scratch = Scratch::new("truncated");
        let path = scratch.write("events.jsonl", &format!("{DONE}\n{DONE}\n"));
        let log = EventLog::discover(&scratch.0);
        let full = log.drain(Cursor::default()).expect("reads");
        assert_eq!(full.events.len(), 2);

        fs::write(&path, format!("{DONE}\n")).expect("the container shrinks");
        let err = log
            .drain(full.cursor)
            .expect_err("an offset past the end is loss");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("history was lost"),
            "the message must say what happened: {err}"
        );
    }

    #[test]
    fn dr_001_an_offset_exactly_at_the_end_is_the_ordinary_caught_up_case() {
        // The boundary next door to the one above: nothing new is NOT loss.
        let scratch = Scratch::new("caughtup");
        scratch.write("events.jsonl", &format!("{DONE}\n"));
        let log = EventLog::discover(&scratch.0);
        let first = log.drain(Cursor::default()).expect("reads");
        let again = log.drain(first.cursor).expect("caught up is not an error");
        assert!(again.events.is_empty());
        assert!(again.drained);
        assert_eq!(again.cursor, first.cursor);
    }

    #[test]
    fn sc_519_a_container_that_exists_but_will_not_open_is_still_an_error() {
        // A directory where the container should be: the open fails with
        // something other than NotFound, which SC-509b says must reach the
        // digest as loss.
        let scratch = Scratch::new("unreadable");
        fs::create_dir_all(scratch.0.join("events.jsonl")).expect("a directory in its place");
        let log = EventLog::discover(&scratch.0);
        let err = log
            .drain(Cursor::default())
            .expect_err("not a readable file");
        assert_ne!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn a_cursor_into_a_generation_that_does_not_exist_is_an_error() {
        let scratch = Scratch::new("nogen");
        scratch.write("events.jsonl", &format!("{DONE}\n"));
        let log = EventLog::discover(&scratch.0);
        let err = log.drain(Cursor::start_of(7)).expect_err("no generation 7");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    /// Every alert summary the frozen watchdog can actually emit, taken from the
    /// `ae_emit_event "alert"` call sites in `ae` @72c7293 and from
    /// `aewatch`. This is the whole probe population SC-980 names, so the
    /// cascade is asserted against ALL of it rather than against examples.
    const INCUMBENT_ALERT_SUMMARIES: [(&str, Reason); 7] = [
        ("agent process dead — dropped to shell", Reason::Dead),
        (
            "pane missing — agent no longer visible in session",
            Reason::Dead,
        ),
        (
            "max nudges reached (no recent events), needs attention",
            Reason::Stale,
        ),
        ("throttled for 10s — may need attention", Reason::Throttled),
        (
            "meta-agent not sweeping — heartbeat stopped (may be stuck)",
            Reason::Stale,
        ),
        (
            "meta-agent unreachable — 3 sweep nudges undelivered (not sweeping)",
            Reason::Stale,
        ),
        (
            "nudge unreachable/occupied — 2 undelivered attempts (idle 900s)",
            Reason::Stale,
        ),
    ];

    #[test]
    fn sc_980_every_incumbent_alert_summary_names_its_class() {
        for (summary, want) in INCUMBENT_ALERT_SUMMARIES {
            assert_eq!(alert_class(Some(summary)), want, "{summary:?}");
        }
    }

    #[test]
    fn sc_980_an_unreadable_alert_is_stale_and_never_silent() {
        // `alert` MEANS attention is required, so a class this cascade cannot
        // read is an alert of unknown class — not a non-event. Dropping it
        // would hide the one thing the record exists to report.
        for summary in [
            None,
            Some(""),
            Some("something the watchdog has not emitted yet"),
        ] {
            assert_eq!(alert_class(summary), Reason::Stale, "{summary:?}");
        }
    }

    #[test]
    fn sc_980_the_wedge_alert_is_stale_even_when_its_text_names_another_class() {
        // The one place the cascade's ORDER is load-bearing. A wedge summary
        // that also says "throttled" must not be downgraded by the later arm,
        // and one that also says "dead" must not be taken by the earlier one.
        assert_eq!(
            alert_class(Some("meta-agent not sweeping — throttled upstream")),
            Reason::Stale,
        );
        assert_eq!(
            alert_class(Some("meta-agent not sweeping — pane looks dead")),
            Reason::Stale,
        );
    }

    #[test]
    fn sc_980_the_missing_marker_is_matched_in_both_spellings() {
        // The incumbent lists `missing` AND `MISSING` and its match is
        // case-sensitive. Keeping only one spelling silently reclassifies a
        // real pane-missing alert as stale.
        assert_eq!(alert_class(Some("pane is MISSING")), Reason::Dead);
        assert_eq!(alert_class(Some("pane is missing")), Reason::Dead);
    }

    fn event(action: &str, summary: Option<&str>) -> Event {
        let summary = summary.map_or_else(String::new, |text| format!(r#","summary":"{text}""#));
        let line = format!(
            r#"{{"ts":"2026-08-20T15:00:19Z","actor":"_watchdog","action":"{action}","target":"fake:probe"{summary}}}"#
        );
        Event::parse_line(&line).expect("a well-formed watchdog record")
    }

    #[test]
    fn sc_509c_an_alert_raises_the_class_its_summary_names() {
        assert_eq!(
            event("alert", Some("agent process dead — dropped to shell")).alert_meaning(),
            AlertMeaning::Raised(Reason::Dead),
        );
        assert_eq!(
            event("alert", Some("max nudges reached, needs attention")).alert_meaning(),
            AlertMeaning::Raised(Reason::Stale),
        );
    }

    #[test]
    fn sc_509c_a_watchdog_clear_retracts_rather_than_deciding_nothing() {
        // Cleared and Undefined are different answers: a clear ENDS the scan,
        // an undefined record lets it look further back.
        for action in ["alert-cleared", "throttle-cleared"] {
            assert_eq!(
                event(action, Some("recovered")).alert_meaning(),
                AlertMeaning::Cleared,
                "{action}"
            );
        }
    }

    #[test]
    fn sc_509c_an_ordinary_record_carries_no_watchdog_verdict() {
        // `nudge` is the load-bearing entry: the watchdog writes it, it names
        // the agent as TARGET, and it is what PRECEDES an alert — so a reader
        // that let it carry a verdict would answer from the question instead of
        // from the answer.
        for action in ["state", "send", "nudge", "ask", "reply", "memo", "recover"] {
            assert_eq!(
                event(action, Some("agent process dead — dropped to shell")).alert_meaning(),
                AlertMeaning::Undefined,
                "{action}: only `alert` may classify a summary"
            );
        }
    }

    #[test]
    fn sc_509c_a_throttled_action_is_a_carrier_on_its_action_alone() {
        // The ruled evidence class: `target` names the owner and the ACTION
        // names the contribution, so no summary is consulted. Each summary
        // below would classify DIFFERENTLY if one were — "pausing nudges"
        // carries no marker and would fall to Stale, and the second says
        // "dead". The action deciding alone is the property, not a shortcut.
        for summary in [
            Some("upstream throttling detected — pausing nudges"),
            Some("the process looks dead"),
            None,
        ] {
            assert_eq!(
                event("throttled", summary).alert_meaning(),
                AlertMeaning::Raised(Reason::Throttled),
                "{summary:?}: the action decides, and the summary may not narrow it"
            );
        }
    }

    #[test]
    fn sc_510c_an_alert_ref_is_not_pressed_into_service_as_a_typed_reason() {
        // SC-980's typed key is unnamed by any row, so `ref` on an alert stays
        // Undefined. A reader that took it would be inventing the schema.
        let line = concat!(
            r#"{"ts":"2026-08-20T15:00:19Z","actor":"_watchdog","action":"alert","#,
            r#""target":"fake:probe","ref":"throttled","summary":"agent process dead"}"#
        );
        let event = Event::parse_line(line).expect("a well-formed record");
        assert_eq!(event.ref_meaning(), RefMeaning::Undefined);
        assert_eq!(event.alert_meaning(), AlertMeaning::Raised(Reason::Dead));
    }
}
