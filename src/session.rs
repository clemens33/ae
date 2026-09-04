//! Reading a session directory: the meta rows, the event stream, and the digest
//! entry they produce.
//!
//! Read-only. Nothing here creates, writes or touches a file.
//!
//! Liveness, branch and watchdog verdicts are not session-directory facts, so
//! they arrive from the caller in [`SessionRuntime`] rather than being guessed.
//!
//! An event carrying a routing key whose session is stale — after a rename —
//! stays unassociated rather than being matched by display name.

use std::fs;
use std::io;
use std::path::Path;

use crate::attention::Reason;
use crate::digest::{AgentEntry, FactState, RenderKnowledge, SessionEntry, Status};
use crate::events::{
    AlertMeaning, Cursor, Drain, Event, EventLog, Identity, RefMeaning, RoutingMember, SkippedLine,
};
use crate::meta::{Anomaly, Meta};
use crate::time::Timestamp;

/// The `unanswered` threshold when nothing tunes it.
pub const DEFAULT_UNANSWERED_SECS: i64 = 1800;

/// An `ask` / `review` whose target has not replied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    /// The request id.
    pub id: String,
    /// Which of the two actions opened it.
    pub action: String,
    /// Who asked.
    pub actor: String,
    /// Who was asked, as the event named them.
    pub target: Option<String>,
    /// When it was sent.
    pub sent_at: Timestamp,
}

impl PendingRequest {
    /// How long this request has been waiting, as of `now`.
    #[must_use]
    pub const fn age_secs(&self, now: Timestamp) -> i64 {
        self.sent_at.seconds_until(now)
    }
}

/// What one session directory's event stream says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRead {
    /// Every event read, in stream order. Kept so every derived field answers
    /// from one snapshot of a moving stream.
    pub events: Vec<Event>,
    /// The newest event's timestamp — the activity clock.
    pub last_active: Option<Timestamp>,
    /// Requests still waiting on their target, oldest first.
    pub pending: Vec<PendingRequest>,
    /// Where a reader would resume (generation + offset).
    pub cursor: Cursor,
    /// Lines that were not events, kept rather than dropped.
    pub skipped: Vec<SkippedLine>,
}

impl SessionRead {
    /// Read every generation of the event log under `dir`.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] when the log EXISTS and cannot be read,
    /// or when a cursor's history has gone (see [`EventLog::drain`]). A missing or
    /// zero-byte log is a quiet empty stream, not an error — that is the whole
    /// distinction between a session that has not spoken yet and one whose words
    /// were lost.
    pub fn open(dir: &Path) -> io::Result<Self> {
        let log = EventLog::discover(dir);
        let drain = log.drain_all(Cursor::default())?;
        Ok(Self::from_drain(&drain))
    }

    /// Read an already-drained stream.
    ///
    /// ```
    /// use ae::attention::Reason;
    /// use ae::events::{Cursor, Drain, Event};
    /// use ae::session::{DEFAULT_UNANSWERED_SECS, SessionRead};
    /// use ae::time::Timestamp;
    ///
    /// // An ask at 09:00 that nobody answered, read at 14:00.
    /// let ask = Event::parse_line(concat!(
    ///     r#"{"ts":"2026-05-29T09:00:00Z","actor":"claude:lead","action":"ask","#,
    ///     r#""target":"codex:coworker","ref":"ae-1"}"#,
    /// ))?;
    /// let read = SessionRead::from_drain(&Drain {
    ///     events: vec![ask],
    ///     cursor: Cursor::default(),
    ///     skipped: Vec::new(),
    ///     drained: true,
    /// });
    ///
    /// let now = Timestamp::parse("2026-05-29T14:00:00Z").unwrap();
    /// assert_eq!(read.unanswered(now, DEFAULT_UNANSWERED_SECS).len(), 1);
    /// assert_eq!(
    ///     read.attention_contribution(now, DEFAULT_UNANSWERED_SECS),
    ///     Some(Reason::Unanswered),
    /// );
    /// # Ok::<(), ae::events::EventError>(())
    /// ```
    #[must_use]
    pub fn from_drain(drain: &Drain) -> Self {
        Self {
            last_active: drain.events.iter().map(|event| event.ts).max(),
            pending: pending_requests(&drain.events),
            cursor: drain.cursor,
            skipped: drain.skipped.clone(),
            events: drain.events.clone(),
        }
    }

    /// The requests that have been waiting longer than `threshold_secs`.
    ///
    /// Age must EXCEED the threshold; equality is not past it.
    #[must_use]
    pub fn unanswered(&self, now: Timestamp, threshold_secs: i64) -> Vec<&PendingRequest> {
        self.pending
            .iter()
            .filter(|request| request.age_secs(now) > threshold_secs)
            .collect()
    }

    /// This session's event-derived contribution to the attention rollup.
    #[must_use]
    pub fn attention_contribution(&self, now: Timestamp, threshold_secs: i64) -> Option<Reason> {
        if self.unanswered(now, threshold_secs).is_empty() {
            None
        } else {
            Some(Reason::Unanswered)
        }
    }

    /// When the goal was last set.
    ///
    /// The `ts` of the LAST APPENDED goal event, not the numerically greatest `ts`:
    /// a max-timestamp fold would let clock skew reorder committed state. Same
    /// ordering as [`SessionRead::declared_state_of`] and
    /// [`SessionRead::alert_reason_of`].
    #[must_use]
    pub fn goal_set_at(&self) -> Option<Timestamp> {
        self.events
            .iter()
            .filter(|event| event.action == "goal")
            .map(|event| event.ts)
            .next_back()
    }

    /// The work state this agent last declared.
    ///
    /// Identity is the routing key when the event carries one for THIS session, the
    /// display name otherwise; a renamed session's older events have no key.
    ///
    /// The LAST APPENDED declaration wins, whatever `ts` it claims. Ordering by `ts`
    /// would let a stale-stamped record appended afterwards overwrite the real
    /// latest one — silently, since both values are legal states.
    #[must_use]
    pub fn declared_state_of(&self, session: &str, slot: &str, reference: &str) -> Option<&str> {
        self.events
            .iter()
            .filter(|event| is_actor(event, session, slot, reference))
            .filter_map(Event::declared_state)
            .next_back()
    }

    /// The watchdog's standing verdict on this agent, or `None`.
    ///
    /// The question is not "is there an alert" but "what is the NEWEST thing the log
    /// says about this agent". An `alert` raises, a watchdog clear retracts, and any
    /// other event the agent ITSELF wrote is recovery. Everything else — a `nudge`
    /// above all, which the watchdog writes and which names the agent as TARGET —
    /// decides nothing, and the scan looks further back. Treating an inbound event
    /// as recovery would clear every alert the moment it was raised, since nudges
    /// are what precede one.
    ///
    /// Newest by LEDGER ORDER, not by timestamp, for the same reason as
    /// [`SessionRead::declared_state_of`]. `last_active` is deliberately NOT one of
    /// these scans: it takes `max(ts)` because a future-stamped event must count as
    /// active rather than silently hide a live session.
    #[must_use]
    pub fn alert_reason_of(&self, session: &str, slot: &str, reference: &str) -> Option<Reason> {
        alert_reason_in(&self.events, session, slot, reference)
    }

    /// Whether this read lost anything.
    ///
    /// A skipped malformed COMPLETE record is loss. A buffered unterminated tail is
    /// not, and the reader never reports one as skipped.
    #[must_use]
    pub fn lost_records(&self) -> bool {
        !self.skipped.is_empty()
    }
}

/// What one agent's RUNTIME says — facts no session directory holds.
///
/// `alert` arrives as a decided reason rather than as prose this reader parses:
/// free text is never a discriminator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRuntime {
    /// The slot this describes — `main` / `worker.<n>` / `spawned.<n>`.
    pub slot: String,
    /// Whether the agent's pane is alive, three-valued.
    ///
    /// `None` is "not established" — a failed pane query, an ambiguous marker, or a
    /// session whose own liveness is unknown. It is NOT "dead".
    pub alive: Option<bool>,
    /// The watchdog's typed reason for this agent, if any.
    ///
    /// `None` is not an unread source: positive runtime alerts enter only when the
    /// caller supplies one.
    pub alert: Option<Reason>,
}

/// What a session's RUNTIME says — the facts tmux and git own rather than the
/// session directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRuntime {
    /// Running or stopped.
    pub status: Status,
    /// The live git branch. Not a meta key.
    pub branch: Option<String>,
    /// One entry per slot the runtime knows about.
    pub agents: Vec<AgentRuntime>,
}

impl SessionRuntime {
    /// A runtime that knows only whether the session is running.
    #[must_use]
    pub fn new(status: Status) -> Self {
        Self {
            status,
            branch: None,
            agents: Vec::new(),
        }
    }

    fn agent(&self, slot: &str) -> Option<&AgentRuntime> {
        self.agents.iter().find(|agent| agent.slot == slot)
    }
}

/// Whether `event`'s actor is the agent at `slot` / `reference` in `session`.
///
/// An event that carries a routing key is matched on that key or not at all, so
/// a stale session after a rename leaves the event UNASSOCIATED rather than
/// attributed by display name. Display matching survives only for events with NO
/// routing key at all. A key that is half-given, or given EMPTY, still counts as
/// given: it identifies nobody.
fn is_actor(event: &Event, session: &str, slot: &str, reference: &str) -> bool {
    match (&event.actor_slot, &event.actor_session) {
        (RoutingMember::Value(event_slot), RoutingMember::Value(event_session)) => {
            event_slot == slot && event_session == session
        }
        // No routing key at all: the display name is all there is.
        (RoutingMember::Absent, RoutingMember::Absent) => event.actor == reference,
        // Partial, or present-and-empty: routed, to nobody nameable.
        _ => false,
    }
}

/// The alert the durable log still shows for one agent, or `None`.
///
/// A free function over a slice, so the watchdog daemon can ask against the
/// events it already read this cycle. ONE definition: the method above delegates
/// here, because two walks of one log with one meaning is how they drift apart.
///
/// `events` must arrive in APPEND order, and the LAST decisive record wins.
/// `next_back` rather than `last`: this walks backward and stops at the first
/// decisive record instead of reading the whole log to find it.
#[must_use]
pub fn alert_reason_in(
    events: &[Event],
    session: &str,
    slot: &str,
    reference: &str,
) -> Option<Reason> {
    match events
        .iter()
        .filter_map(|event| decisive_verdict(event, session, slot, reference))
        .next_back()
    {
        Some(Verdict::Raised(reason)) => Some(reason),
        Some(Verdict::Clear) | None => None,
    }
}

/// What one record settles for one agent, when it settles anything at all.
///
/// Absence of a `Verdict` is the third answer and the load-bearing one:
/// collapsing it into [`Verdict::Clear`] would let every watchdog nudge cancel
/// the alert it was sent about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// The watchdog says this agent needs a human, for this reason.
    Raised(Reason),
    /// Nothing stands any more — a watchdog retraction, or the agent's own
    /// activity proving it is back.
    Clear,
}

/// What `event` settles for the agent at `slot` / `reference` in `session`.
fn decisive_verdict(event: &Event, session: &str, slot: &str, reference: &str) -> Option<Verdict> {
    let own = is_actor(event, session, slot, reference);
    if !own && !is_addressed_to(event, session, slot, reference) {
        return None;
    }
    match event.alert_meaning() {
        AlertMeaning::Raised(reason) => Some(Verdict::Raised(reason)),
        AlertMeaning::Cleared => Some(Verdict::Clear),
        // The agent's OWN activity is recovery; an inbound record decides nothing.
        AlertMeaning::Undefined if own => Some(Verdict::Clear),
        AlertMeaning::Undefined => None,
    }
}

/// Whether `event` is ADDRESSED TO the agent at `slot` / `reference` — the
/// mirror of [`is_actor`] on the target side.
///
/// An agent never writes the record that says it is dead. The watchdog is the
/// actor of every alert, so consulting only the actor would see none of them.
fn is_addressed_to(event: &Event, session: &str, slot: &str, reference: &str) -> bool {
    match event.target_identity() {
        Some(Identity::Routed {
            slot: event_slot,
            session: event_session,
        }) => event_slot == slot && event_session == session,
        Some(Identity::Display(name)) => {
            name == reference || is_cross_session_form(name, session, reference)
        }
        // Half a routing key addresses nobody, and neither does no target.
        Some(Identity::Unassociated) | None => false,
    }
}

/// Whether `name` is the `@<session>:<agent>` spelling of THIS session's agent.
///
/// Matched by stripping rather than by building the string: this runs once per
/// agent per record.
fn is_cross_session_form(name: &str, session: &str, reference: &str) -> bool {
    name.strip_prefix('@')
        .and_then(|rest| rest.strip_prefix(session))
        .and_then(|rest| rest.strip_prefix(':'))
        .is_some_and(|rest| rest == reference)
}

/// The digest entry for the session directory at `dir`.
///
/// Cannot fail: one bad session degrades its own entry and the document always
/// closes. That degradation is VISIBLE — `degraded: true` reaches the JSON
/// whenever data was actually lost, so damage never renders identically to
/// legitimate sparsity.
///
/// Loss is an unreadable or anomalous `meta`, an event log that EXISTS and would
/// not read, or a skipped malformed complete record. An absent or zero-byte
/// event log is a quiet stream, not damage.
#[must_use]
pub fn entry_for(
    dir: &Path,
    name: &str,
    runtime: &SessionRuntime,
    now: Timestamp,
    unanswered_secs: i64,
) -> SessionEntry {
    entry_from(
        &RecordSnapshot::read(dir),
        name,
        runtime,
        now,
        unanswered_secs,
    )
}

/// What happened when phase 1 tried to read a candidate's `meta`.
///
/// A tmux-only candidate has no record to lose; a durable candidate whose `meta`
/// will not read has one and lost its contents. Rendering the two the same way
/// would report a destroyed record as a session that never had one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetaRead {
    /// Read and parsed.
    Parsed,
    /// No `meta` in the state directory.
    ///
    /// The DEFAULT, deliberately: a snapshot nobody has read yet has not established
    /// that anything was lost.
    #[default]
    Absent,
    /// A `meta` that exists and would not read.
    Unreadable,
}

/// Everything one session directory said, read ONCE.
///
/// A digest built from two observations is a digest whose facts never coexisted:
/// a `meta` unreadable at discovery could become readable before rendering and
/// silently repair its own loss fact. Both halves are optional because both
/// reads fail independently, and the difference matters — an unreadable meta is
/// damage where an absent event log is not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordSnapshot {
    /// The parsed `meta`, when it could be read.
    pub meta: Option<Meta>,
    /// WHY there is or is not a `meta` above — the outcome of the very read that
    /// produced it.
    ///
    /// `Option<Meta>` cannot tell absent from unreadable. Reconstructing the
    /// distinction downstream means a SECOND observation wearing a different name,
    /// and one that answers "absent" for a directory the asker may not traverse.
    pub meta_read: MetaRead,
    /// The event stream, when it could be read.
    pub events: Option<SessionRead>,
}

impl RecordSnapshot {
    /// Read both halves of the record at `dir`.
    ///
    /// The ONLY I/O on this path: everything downstream is a pure function of what
    /// this captured, including WHY each half is missing.
    #[must_use]
    pub fn read(dir: &Path) -> Self {
        let (meta, meta_read) = match Meta::read(dir) {
            Ok(meta) => (Some(meta), MetaRead::Parsed),
            // The ONE place absent and unreadable are told apart, from the error the
            // read itself returned.
            Err(error) if error.kind() == io::ErrorKind::NotFound => (None, MetaRead::Absent),
            Err(_) => (None, MetaRead::Unreadable),
        };
        Self {
            meta,
            meta_read,
            events: SessionRead::open(dir).ok(),
        }
    }
}

/// The branch checked out in the work tree at `dir`, read from git's own files.
///
/// Reading `HEAD` rather than shelling out keeps `list` off a process launch per
/// session and keeps it working where git is not installed. Every failure is
/// `None`. Three shapes: `.git` as a DIRECTORY, `.git` as a FILE holding
/// `gitdir: <path>` (a worktree — ae's own `--worktree` mode creates these, so
/// this is the common case here), and a detached `HEAD` holding a raw object id,
/// which renders short.
#[allow(
    clippy::disallowed_methods,
    reason = "a door: the git branch read — `HEAD` and the worktree `.git` pointer, \
              under a work tree ae already records. Registered in the criterion-3 \
              read-site inventory rather than routed around it. Bounded: two reads of \
              named files under a known directory, no enumeration, and every failure \
              is `None` so a listing never fails on it."
)]
fn branch_at(dir: &Path) -> Option<String> {
    let dot_git = dir.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let pointer = fs::read_to_string(&dot_git).ok()?;
        let target = pointer.trim().strip_prefix("gitdir:")?.trim();
        let target = Path::new(target);
        if target.is_absolute() {
            target.to_path_buf()
        } else {
            dir.join(target)
        }
    };
    let raw = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    // Framing is validated on the RAW bytes, before any normalisation: the
    // previous shape trimmed first and then counted lines, so a leading blank line
    // and a doubled trailing one both normalised to one line and were ACCEPTED.
    // Well-formed `HEAD` is one line with an optional single terminal newline.
    let head = raw.strip_suffix('\n').unwrap_or(&raw);
    if head.chars().any(char::is_control) {
        return None;
    }
    let head = head.trim_matches(' ');
    if let Some(reference) = head.strip_prefix("ref:") {
        // `refs/heads/feature/x` keeps its slashes; only the prefix goes.
        let reference = reference.trim();
        let branch = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        safe_branch(branch).map(ToOwned::to_owned)
    } else if head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit()) {
        // Detached. Short form, matching how a person refers to a commit.
        Some(head.chars().take(7).collect())
    } else {
        None
    }
}

/// `branch` if it is a safe ref name to render, otherwise `None`.
///
/// A RENDERING guard, not a git validator: the bytes come from a file ae did not
/// create and go straight into a terminal, so the question is whether they can
/// rewrite the display. An allowlist answers escape, carriage return and newline
/// at once, where a blocklist answers whichever ones it names.
///
/// Permitted: printable ASCII minus git's own forbidden bytes. A tighter set was
/// tried and rejected refs git accepts. Non-ASCII is refused deliberately — git
/// permits UTF-8 ref names, but a bidirectional override reorders the rest of
/// the line.
fn safe_branch(branch: &str) -> Option<&str> {
    if branch.is_empty() || branch.len() > 255 {
        return None;
    }
    // Printable ASCII, excluding space and the bytes git forbids in a ref name.
    let permitted =
        |c: char| c.is_ascii_graphic() && !matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\');
    if !branch.chars().all(permitted) {
        return None;
    }
    // git's structural rules, plus what matters for a path-shaped value reaching a
    // terminal: no empty segment, no `..` to climb, no leading `-` to read as a
    // flag, no `@{` reflog syntax. Bare `@` is accepted — git calls it valid.
    if branch.contains("@{")
        // git's rule is that a ref may not END in the literal `.lock`,
        // case-sensitively. Compared as BYTES: a case-insensitive test would refuse
        // `a.LOCK`, which git accepts, and `Path::extension` reports None for a bare
        // `.lock`, which git refuses. Both divergences are silent.
        || branch.as_bytes().ends_with(b".lock")
        || branch.ends_with('.')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.contains("//")
        || branch.contains("..")
        || branch.starts_with('-')
    {
        return None;
    }
    Some(branch)
}

/// The digest entry for a record already read.
///
/// Pure: same snapshot, same runtime, same answer, whatever the filesystem is
/// doing by the time this runs. Cannot fail, and reports loss exactly as
/// [`entry_for`] does.
#[must_use]
pub fn entry_from(
    snapshot: &RecordSnapshot,
    name: &str,
    runtime: &SessionRuntime,
    now: Timestamp,
    unanswered_secs: i64,
) -> SessionEntry {
    let mut entry = SessionEntry::new(name, runtime.status);
    entry.branch.clone_from(&runtime.branch);

    // The one raw-record producer: it attaches the values and the provenance that
    // decides whether every serializer may publish them.
    let meta = snapshot.meta.as_ref();
    if let Some(meta) = meta {
        entry.mode = meta.mode().map(ToOwned::to_owned);
        entry.origin = meta.origin().map(ToOwned::to_owned);
        entry.work_dir = meta.work_dir().map(ToOwned::to_owned);
        entry.goal = meta.goal().map(ToOwned::to_owned);
        entry.ae_version = meta.ae_version().map(ToOwned::to_owned);
        // A runtime observation wins when one exists; otherwise read the work tree.
        if entry.branch.is_none() {
            entry.branch = meta.work_dir().and_then(|dir| branch_at(Path::new(dir)));
        }
    }

    let read = snapshot.events.as_ref();
    if let Some(read) = read {
        entry.last_active_epoch = read.last_active.map(Timestamp::epoch);
        entry.goal_set_epoch = read.goal_set_at().map(Timestamp::epoch);
    }

    if let Some(meta) = meta {
        entry.agents = agent_entries(meta, read, runtime, name);
        entry.set_established_runtime_dead_agents(established_runtime_dead_agents(meta, runtime));
    }
    // The MAX across agent reasons PLUS session-level unresolved-request facts.
    // `unanswered` is a PAIR fact, so it never appears as any agents[].reason.
    entry.attention = Reason::rollup(
        entry
            .agents
            .iter()
            .filter_map(|agent| agent.reason)
            .chain(read.and_then(|read| read.attention_contribution(now, unanswered_secs))),
    );
    entry.degraded =
        meta.is_none_or(|meta| anomalies_degrade(meta.anomalies())) || !events_complete(snapshot);
    entry.set_render_knowledge(render_knowledge(snapshot));
    entry
}

/// The runtime-origin `Dead` facts this snapshot may retain under ledger loss.
///
/// A typed runtime hand-in is independent of the ledger, so a malformed tail
/// cannot clear it. Only rostered references are retained.
fn established_runtime_dead_agents(meta: &Meta, runtime: &SessionRuntime) -> Vec<String> {
    meta.roster()
        .iter()
        .filter_map(|slot| {
            runtime
                .agent(&slot.slot)
                .filter(|agent| agent.alert.is_some_and(Reason::is_severity_maximum))
                .map(|_| slot.reference())
        })
        .collect()
}

/// Map one raw-record snapshot to the source knowledge each rendered member
/// needs. Aggregate degradation is deliberately computed separately above.
fn render_knowledge(snapshot: &RecordSnapshot) -> RenderKnowledge {
    let meta = snapshot.meta.as_ref();
    RenderKnowledge {
        mode: FactState::from_complete(
            meta.is_some_and(|meta| meta_member_complete(meta, "mode", meta.mode().is_some())),
        ),
        origin: FactState::from_complete(
            meta.is_some_and(|meta| meta_member_complete(meta, "origin", meta.origin().is_some())),
        ),
        work_dir: FactState::from_complete(
            meta.is_some_and(|meta| {
                meta_member_complete(meta, "work_dir", meta.work_dir().is_some())
            }),
        ),
        goal: FactState::from_complete(
            meta.is_some_and(|meta| meta_member_complete(meta, "goal", meta.goal().is_some())),
        ),
        events: FactState::from_complete(events_complete(snapshot)),
        roster: FactState::from_complete(meta.is_some_and(roster_complete)),
    }
}

/// Whether the complete event stream was available to every event-derived fact.
fn events_complete(snapshot: &RecordSnapshot) -> bool {
    snapshot
        .events
        .as_ref()
        .is_some_and(|read| !read.lost_records())
}

/// Whether one optional meta member was settled by the parsed record.
///
/// A duplicate of the member's own key makes its value unknowable. With no value
/// found, an unattributed malformed line makes its absence unknowable too.
fn meta_member_complete(meta: &Meta, key: &str, value_present: bool) -> bool {
    let duplicate = meta.anomalies().iter().any(|anomaly| {
        matches!(anomaly, Anomaly::DuplicateKey { key: duplicate, .. } if duplicate == key)
    });
    let unattributed_loss = meta
        .anomalies()
        .iter()
        .any(|anomaly| matches!(anomaly, Anomaly::MalformedLine { .. }));
    !duplicate && (value_present || !unattributed_loss)
}

/// Whether meta established the complete agent population.
///
/// A readable zero-entry roster is complete. Only missing meta or parse loss
/// that could name an agent makes the population unenumerable.
fn roster_complete(meta: &Meta) -> bool {
    !meta.anomalies().iter().any(|anomaly| match anomaly {
        Anomaly::MalformedLine { .. }
        | Anomaly::MalformedRosterEntry { .. }
        | Anomaly::MixedSchemaSlot { .. }
        | Anomaly::DuplicateName { .. } => true,
        Anomaly::DuplicateKey { key, .. } => key.starts_with("agent.") || key.starts_with("seat."),
        Anomaly::UnknownKey { .. } => false,
    })
}

/// What is known about one roster agent's liveness.
///
/// The two grains are related in one direction only: a session proven `stopped`
/// implies every roster agent dead, an `unknown` session implies unknown agents,
/// and a `running` session defers to the pane observation. A runtime naming no
/// member for the slot has made no observation of it, so the answer is unknown
/// rather than dead — that default is where absence of evidence became evidence
/// of absence.
fn agent_liveness(runtime: &SessionRuntime, agent: Option<&AgentRuntime>) -> Option<bool> {
    match runtime.status {
        // Proven absent: every roster agent with it.
        Status::Stopped => Some(false),
        // Nothing established about the session, so nothing about its panes.
        Status::Unknown => None,
        // The pane observation decides, and its absence decides nothing.
        Status::Running => agent.and_then(|agent| agent.alive),
    }
}

/// The `agents[]` array: the meta's roster, answered by the runtime and the
/// event stream.
///
/// Membership is roster-defined. A runtime-only pane never invents an agent, and
/// an unknown pane never erases an independently established alert.
fn agent_entries(
    meta: &Meta,
    read: Option<&SessionRead>,
    runtime: &SessionRuntime,
    session: &str,
) -> Vec<AgentEntry> {
    meta.roster()
        .iter()
        .map(|slot| {
            let reference = slot.reference();
            let declared =
                read.and_then(|read| read.declared_state_of(session, &slot.slot, &reference));
            let runtime_agent = runtime.agent(&slot.slot);
            AgentEntry {
                reference: reference.clone(),
                // Schema 2 keeps publishing `alias`; for a v2 row that is the profile.
                alias: slot.profile.clone().unwrap_or_default(),
                name: slot.name.clone(),
                session_id: slot.harness_session.clone(),
                alive: agent_liveness(runtime, runtime_agent),
                state: declared.map(ToOwned::to_owned),
                // This agent's OWN contribution, from the two evidence classes: ALERT-DERIVED
                // dead/stale/throttled, and SELF-DECLARED waiting-user/blocked. `None` means
                // no agent-owned contribution exists — a session-level `unanswered` never
                // reaches this field, because no agent owns a pair fact.
                reason: Reason::rollup(
                    runtime_agent
                        .and_then(|agent| agent.alert)
                        .into_iter()
                        .chain(
                            read.and_then(|read| {
                                read.alert_reason_of(session, &slot.slot, &reference)
                            }),
                        )
                        .chain(declared.and_then(declared_reason)),
                ),
            }
        })
        .collect()
}

/// Whether a meta's anomalies degrade the session, per anomaly KIND.
///
/// Unknown keys are TOLERATED: they are the normal state of a real meta, so
/// degrading on them would make the flag constant-true. A malformed line, a
/// duplicate key or a malformed roster value is a value the reader could not
/// take, and does degrade.
fn anomalies_degrade(anomalies: &[Anomaly]) -> bool {
    anomalies.iter().any(|anomaly| match anomaly {
        Anomaly::UnknownKey { .. } => false,
        Anomaly::MalformedLine { .. }
        | Anomaly::DuplicateKey { .. }
        | Anomaly::MalformedRosterEntry { .. }
        | Anomaly::MixedSchemaSlot { .. }
        | Anomaly::DuplicateName { .. } => true,
    })
}

/// The attention reason a DECLARED work state contributes, if any.
///
/// `working` and `done` are states, not reasons: an agent that is working does
/// not need a human.
fn declared_reason(state: &str) -> Option<Reason> {
    match state {
        "waiting-user" => Some(Reason::WaitingUser),
        "blocked" => Some(Reason::Blocked),
        _ => None,
    }
}

/// The `ask`/`review` events with no qualifying reply, oldest first.
///
/// Closure requires the FULL mirror: the same `ref`, the reply's actor is the
/// request's target, AND the reply's target is the request's actor. A loud
/// false-pending is safer than a silent false-closure, which loses the question
/// entirely.
///
/// Identity compares routing keys when both sides carry them and display names
/// when neither does; a MIXED pair matches nothing. Only the newest ask/review
/// per `ref` counts, so a re-ask restarts the clock.
fn pending_requests(events: &[Event]) -> Vec<PendingRequest> {
    // One forward pass over an append-only log, so a reply that appears BEFORE its
    // request finds nothing open and closes nothing.
    let mut open: Vec<&Event> = Vec::new();
    for event in events {
        let RefMeaning::RequestId(id) = event.ref_meaning() else {
            continue;
        };
        match event.action.as_str() {
            "ask" | "review" => {
                open.retain(|existing| existing.reference.as_deref() != Some(id));
                open.push(event);
            }
            "reply" => open.retain(|request| !closes(request, event, id)),
            _ => {}
        }
    }
    let mut pending: Vec<PendingRequest> = open
        .into_iter()
        .filter_map(|event| {
            Some(PendingRequest {
                id: event.reference.clone()?,
                action: event.action.clone(),
                actor: event.actor.clone(),
                target: event.target.clone(),
                sent_at: event.ts,
            })
        })
        .collect();
    pending.sort_by_key(|request| request.sent_at);
    pending
}

/// Whether `reply` closes `request` — the mirror match, in full.
fn closes(request: &Event, reply: &Event, reply_ref: &str) -> bool {
    if request.reference.as_deref() != Some(reply_ref) {
        return false;
    }
    let from_the_target = request
        .target_identity()
        .is_some_and(|target| target.matches(reply.actor_identity()));
    let to_the_asker = reply
        .target_identity()
        .is_some_and(|target| target.matches(request.actor_identity()));
    from_the_target && to_the_asker
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real directories; the boundary is about \
                  what PRODUCT code may reach"
    )]

    use super::{AgentRuntime, DEFAULT_UNANSWERED_SECS, SessionRead, SessionRuntime, entry_for};
    use crate::attention::Reason;
    use crate::digest::Status;
    use crate::events::{Cursor, Drain, Event};
    use crate::time::Timestamp;
    use std::fs;
    use std::path::PathBuf;

    const NOW: Timestamp = Timestamp::from_epoch(1_780_000_000);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ae-session-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a scratch dir");
            Self(dir)
        }

        fn meta(&self, text: &str) {
            fs::write(self.0.join("meta"), text).expect("writing a fixture");
        }

        fn events(&self, lines: &[String]) {
            let mut body = lines.join("\n");
            body.push('\n');
            fs::write(self.0.join("events.jsonl"), body).expect("writing a fixture");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn at(seconds_ago: i64) -> String {
        Timestamp::from_epoch(NOW.epoch() - seconds_ago).to_string()
    }

    fn event(ts: &str, actor: &str, action: &str, extra: &str) -> String {
        format!(r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}"{extra}}}"#)
    }

    fn read(lines: &[String]) -> SessionRead {
        let events: Vec<Event> = lines
            .iter()
            .map(|line| Event::parse_line(line).expect("a fixture line must be an event"))
            .collect();
        SessionRead::from_drain(&Drain {
            events,
            cursor: Cursor::default(),
            skipped: Vec::new(),
            drained: true,
        })
    }

    #[test]
    fn sc_017e_the_activity_clock_is_the_newest_event() {
        let lines = [
            event(&at(900), "claude:lead", "done", ""),
            event(&at(30), "claude:lead", "memo", r#","ref":"design""#),
            event(&at(600), "watchdog", "nudge", ""),
        ];
        let read = read(&lines);
        assert_eq!(
            read.last_active,
            Some(Timestamp::from_epoch(NOW.epoch() - 30)),
            "the newest event, not the last line"
        );
    }

    #[test]
    fn a_session_with_no_events_has_no_activity_clock() {
        assert_eq!(read(&[]).last_active, None);
    }

    #[test]
    fn sc_017g_an_ask_whose_target_never_replied_is_unanswered_past_the_threshold() {
        let lines = [event(
            &at(DEFAULT_UNANSWERED_SECS + 60),
            "claude:lead",
            "ask",
            r#","target":"codex:coworker","ref":"ae-1""#,
        )];
        let read = read(&lines);
        assert_eq!(read.pending.len(), 1);
        assert_eq!(read.unanswered(NOW, DEFAULT_UNANSWERED_SECS).len(), 1);
        assert_eq!(
            read.attention_contribution(NOW, DEFAULT_UNANSWERED_SECS),
            Some(Reason::Unanswered)
        );
    }

    #[test]
    fn sc_017g_a_request_still_inside_the_threshold_is_not_an_attention_reason() {
        let lines = [event(
            &at(60),
            "claude:lead",
            "ask",
            r#","target":"codex:coworker","ref":"ae-1""#,
        )];
        let read = read(&lines);
        assert_eq!(read.pending.len(), 1, "it is pending");
        assert!(
            read.unanswered(NOW, DEFAULT_UNANSWERED_SECS).is_empty(),
            "but it has not been waiting long enough to need a human"
        );
        assert_eq!(
            read.attention_contribution(NOW, DEFAULT_UNANSWERED_SECS),
            None
        );
    }

    #[test]
    fn sc_017g_the_threshold_is_passed_not_merely_reached() {
        // At exactly the threshold it has not passed it yet; one second later it has.
        for (age, expected) in [
            (DEFAULT_UNANSWERED_SECS - 1, 0),
            (DEFAULT_UNANSWERED_SECS, 0),
            (DEFAULT_UNANSWERED_SECS + 1, 1),
        ] {
            let lines = [event(
                &at(age),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1""#,
            )];
            assert_eq!(
                read(&lines).unanswered(NOW, DEFAULT_UNANSWERED_SECS).len(),
                expected,
                "age {age}"
            );
        }
    }

    #[test]
    fn sc_017g_a_reply_from_the_target_closes_the_request() {
        let lines = [
            event(
                &at(3600),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1""#,
            ),
            event(
                &at(3000),
                "codex:coworker",
                "reply",
                r#","target":"claude:lead","ref":"ae-1""#,
            ),
        ];
        assert!(read(&lines).pending.is_empty());
    }

    #[test]
    fn sc_518_a_reply_addressed_to_someone_else_does_not_close_it() {
        // Right responder, wrong recipient.
        let lines = [
            event(
                &at(3600),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1""#,
            ),
            event(
                &at(3000),
                "codex:coworker",
                "reply",
                r#","target":"gemini:someone-else","ref":"ae-1""#,
            ),
        ];
        assert_eq!(read(&lines).pending.len(), 1);
    }

    #[test]
    fn sc_518_a_reply_with_no_target_at_all_closes_nothing() {
        let lines = [
            event(
                &at(3600),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1""#,
            ),
            event(&at(3000), "codex:coworker", "reply", r#","ref":"ae-1""#),
        ];
        assert_eq!(read(&lines).pending.len(), 1);
    }

    #[test]
    fn sc_405j_a_partial_key_reply_cannot_close_a_display_only_ask() {
        // The reply carries actor_slot with no actor_session, and that half-key used
        // to read as its display name — which matches the ask's target.
        for reply_keys in [r#","actor_slot":"worker.0""#, r#","actor_session":"live""#] {
            let lines = [
                event(
                    &at(3600),
                    "claude:lead",
                    "ask",
                    r#","target":"codex:coworker","ref":"ae-1""#,
                ),
                event(
                    &at(3000),
                    "codex:coworker",
                    "reply",
                    &format!(r#","target":"claude:lead","ref":"ae-1"{reply_keys}"#),
                ),
            ];
            assert_eq!(read(&lines).pending.len(), 1, "{reply_keys}");
        }
    }

    #[test]
    fn sc_405j_a_partial_key_on_the_ask_side_is_equally_unclosable() {
        // Mirrored: the REQUEST has half a key, so nothing can answer it.
        for ask_keys in [
            r#","target_slot":"worker.0""#,
            r#","target_session":"live""#,
        ] {
            let lines = [
                event(
                    &at(3600),
                    "claude:lead",
                    "ask",
                    &format!(r#","target":"codex:coworker","ref":"ae-1"{ask_keys}"#),
                ),
                event(
                    &at(3000),
                    "codex:coworker",
                    "reply",
                    r#","target":"claude:lead","ref":"ae-1""#,
                ),
            ];
            assert_eq!(read(&lines).pending.len(), 1, "{ask_keys}");
        }
    }

    #[test]
    fn sc_405j_an_empty_key_reply_cannot_false_close_a_display_only_ask() {
        // The reply declares itself routed with an EMPTY member.
        for reply_keys in [
            r#","actor_slot":"""#,
            r#","actor_session":"""#,
            r#","actor_slot":"","actor_session":"""#,
            r#","actor_slot":"","actor_session":"live""#,
        ] {
            let lines = [
                event(
                    &at(3600),
                    "claude:lead",
                    "ask",
                    r#","target":"codex:coworker","ref":"ae-1""#,
                ),
                event(
                    &at(3000),
                    "codex:coworker",
                    "reply",
                    &format!(r#","target":"claude:lead","ref":"ae-1"{reply_keys}"#),
                ),
            ];
            assert_eq!(read(&lines).pending.len(), 1, "{reply_keys}");
        }
    }

    #[test]
    fn sc_405j_an_empty_key_on_the_ask_side_is_equally_unclosable() {
        for ask_keys in [
            r#","target_slot":"""#,
            r#","target_session":"""#,
            r#","target_slot":"","target_session":"""#,
        ] {
            let lines = [
                event(
                    &at(3600),
                    "claude:lead",
                    "ask",
                    &format!(r#","target":"codex:coworker","ref":"ae-1"{ask_keys}"#),
                ),
                event(
                    &at(3000),
                    "codex:coworker",
                    "reply",
                    r#","target":"claude:lead","ref":"ae-1""#,
                ),
            ];
            assert_eq!(read(&lines).pending.len(), 1, "{ask_keys}");
        }
    }

    #[test]
    fn sc_405j_an_empty_key_state_event_associates_with_no_agent() {
        // The same rule on the other consumer of identity: a declared state whose
        // routing key is empty belongs to nobody.
        let scratch = Scratch::new("emptykeystate");
        scratch.meta(META);
        scratch.events(&[event(
            &at(600),
            "claude:lead",
            "state",
            r#","ref":"blocked","actor_slot":"","actor_session":"live""#,
        )]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].state, None);
        assert_eq!(entry.agents[0].reason, None);
        assert!(
            !entry.degraded,
            "an unroutable identity is not read/parse loss"
        );
    }

    #[test]
    fn sc_405j_two_unassociated_sides_do_not_match_each_other() {
        // Both sides failed to say where they came from. That is not agreement.
        let lines = [
            event(
                &at(3600),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1","target_slot":"worker.0""#,
            ),
            event(
                &at(3000),
                "codex:coworker",
                "reply",
                r#","target":"claude:lead","ref":"ae-1","actor_slot":"worker.0""#,
            ),
        ];
        assert_eq!(read(&lines).pending.len(), 1);
    }

    #[test]
    fn sc_518_a_mixed_identity_pair_matches_nothing() {
        // Request routed, reply display-only: mixed matches nothing.
        let lines = [
            event(
                &at(3600),
                "claude:lead",
                "ask",
                concat!(
                    r#","target":"codex:coworker","ref":"ae-1""#,
                    r#","actor_slot":"main","actor_session":"s""#,
                    r#","target_slot":"worker.0","target_session":"s""#
                ),
            ),
            event(
                &at(3000),
                "codex:coworker",
                "reply",
                r#","target":"claude:lead","ref":"ae-1""#,
            ),
        ];
        assert_eq!(read(&lines).pending.len(), 1);
    }

    #[test]
    fn sc_017g_a_reply_from_someone_else_does_not_close_it() {
        // "whose TARGET never replied" — a stray reply leaves it pending.
        let lines = [
            event(
                &at(3600),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1""#,
            ),
            event(
                &at(3000),
                "gemini:bystander",
                "reply",
                r#","target":"claude:lead","ref":"ae-1""#,
            ),
        ];
        assert_eq!(read(&lines).pending.len(), 1);
    }

    #[test]
    fn sc_510c_a_reply_carrying_another_request_id_closes_nothing() {
        let lines = [
            event(
                &at(3600),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1""#,
            ),
            event(
                &at(3000),
                "codex:coworker",
                "reply",
                r#","target":"claude:lead","ref":"ae-2""#,
            ),
        ];
        assert_eq!(read(&lines).pending.len(), 1);
    }

    #[test]
    fn sc_511b_pairing_uses_the_routing_key_when_both_sides_carry_one() {
        // The display name churns; the slot+session key does not. The reply's
        // actor is spelled differently and must still close the request.
        let lines = [
            event(
                &at(3600),
                "claude:lead",
                "ask",
                concat!(
                    r#","target":"codex:coworker","ref":"ae-1""#,
                    r#","actor_slot":"main","actor_session":"s""#,
                    r#","target_slot":"worker.0","target_session":"s""#
                ),
            ),
            event(
                &at(3000),
                "codex:renamed-since",
                "reply",
                concat!(
                    r#","target":"claude:lead","ref":"ae-1""#,
                    r#","actor_slot":"worker.0","actor_session":"s""#,
                    r#","target_slot":"main","target_session":"s""#
                ),
            ),
        ];
        assert!(
            read(&lines).pending.is_empty(),
            "a renamed agent still answered"
        );
    }

    #[test]
    fn sc_511b_a_routing_key_from_another_session_is_a_different_participant() {
        let lines = [
            event(
                &at(3600),
                "claude:lead",
                "ask",
                concat!(
                    r#","target":"codex:coworker","ref":"ae-1""#,
                    r#","actor_slot":"main","actor_session":"s""#,
                    r#","target_slot":"worker.0","target_session":"s""#
                ),
            ),
            event(
                &at(3000),
                "codex:coworker",
                "reply",
                concat!(
                    r#","target":"claude:lead","ref":"ae-1""#,
                    r#","actor_slot":"worker.0","actor_session":"another-session""#
                ),
            ),
        ];
        assert_eq!(
            read(&lines).pending.len(),
            1,
            "same slot, different session, different agent"
        );
    }

    #[test]
    fn a_review_is_a_request_too() {
        let lines = [event(
            &at(3600),
            "claude:lead",
            "review",
            r#","target":"codex:coworker","ref":"ae-1""#,
        )];
        assert_eq!(read(&lines).pending.len(), 1);
    }

    #[test]
    fn a_re_ask_restarts_the_clock_rather_than_leaving_two_open() {
        let lines = [
            event(
                &at(7200),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1""#,
            ),
            event(
                &at(60),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1""#,
            ),
        ];
        let read = read(&lines);
        assert_eq!(read.pending.len(), 1);
        assert!(
            read.unanswered(NOW, DEFAULT_UNANSWERED_SECS).is_empty(),
            "the newest ask is the one waiting"
        );
    }

    #[test]
    fn a_reply_that_predates_its_request_does_not_answer_it() {
        let lines = [
            event(
                &at(3000),
                "codex:coworker",
                "reply",
                r#","target":"claude:lead","ref":"ae-1""#,
            ),
            event(
                &at(2400),
                "claude:lead",
                "ask",
                r#","target":"codex:coworker","ref":"ae-1""#,
            ),
        ];
        assert_eq!(read(&lines).pending.len(), 1);
    }

    #[test]
    fn an_ask_without_a_target_is_pending_and_nothing_can_close_it() {
        // No target means no "the target replied" to test against.
        let lines = [
            event(&at(3600), "claude:lead", "ask", r#","ref":"ae-1""#),
            event(
                &at(3000),
                "codex:coworker",
                "reply",
                r#","target":"claude:lead","ref":"ae-1""#,
            ),
        ];
        assert_eq!(read(&lines).pending.len(), 1);
    }

    #[test]
    fn pending_requests_are_reported_oldest_first() {
        let lines = [
            event(&at(600), "a:x", "ask", r#","target":"b:y","ref":"newer""#),
            event(&at(7200), "a:x", "ask", r#","target":"b:y","ref":"older""#),
        ];
        let read = read(&lines);
        assert_eq!(
            read.pending
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["older", "newer"]
        );
    }

    /// The roster every scratch session uses unless it needs another.
    const META: &str = "mode=local\norigin=/src\nwork_dir=/src\nagent.main=claude:lead:e795c9e9\n";

    fn running() -> SessionRuntime {
        SessionRuntime::new(Status::Running)
    }

    #[test]
    fn sc_509b_a_session_whose_meta_will_not_read_is_degraded() {
        // Meta absence is real loss: mode, origin, work_dir, goal and the roster.
        let scratch = Scratch::new("nometa");
        let entry = entry_for(
            &scratch.0,
            "no-meta-here",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        assert!(entry.degraded);
        assert_eq!(entry.name, "no-meta-here", "identity survives (SC-509b)");
        assert_eq!(entry.status, Status::Running);
        assert_eq!(entry.mode, None, "never fabricated");
        assert!(entry.agents.is_empty());
    }

    #[test]
    fn sc_509b_a_readable_empty_roster_is_complete_and_quiet() {
        // A readable meta naming zero agents enumerates an empty roster, which is
        // distinct from a missing meta — that loses the roster itself.
        for (meta_text, why) in [
            (String::new(), "an empty file"),
            (
                "mode=local\norigin=/src\nwork_dir=/src\n".to_owned(),
                "context but no roster",
            ),
            (
                "agent_bin.main=claude\n".to_owned(),
                "a binary with no identity",
            ),
        ] {
            let scratch = Scratch::new("rosterless");
            scratch.meta(&meta_text);
            let entry = entry_for(
                &scratch.0,
                "empty",
                &running(),
                NOW,
                DEFAULT_UNANSWERED_SECS,
            );
            assert!(!entry.degraded, "{why}");
            assert!(entry.agents.is_empty(), "{why}");
            assert_eq!(
                entry.to_json().get("agents"),
                Some(&crate::json::Value::Arr(vec![])),
                "{why}: agents stays an array"
            );
            assert_eq!(entry.to_json().get("degraded"), None, "{why}");
            assert_eq!(
                entry.to_json().get("attention"),
                Some(&crate::json::Value::Null),
                "{why}: exact quiet"
            );
        }
    }

    #[test]
    fn sc_509b_a_malformed_line_makes_absent_meta_members_unreadable() {
        let scratch = Scratch::new("unattributed-meta-loss");
        scratch.meta("mode=local\nthis line has no equals sign\nagent.main=claude:lead\n");
        let entry = entry_for(
            &scratch.0,
            "damaged",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );

        let value = entry.to_json();
        assert!(entry.degraded);
        assert_eq!(
            value.get("origin"),
            None,
            "the malformed line could have been the absent origin"
        );
        assert_eq!(
            value.get("work_dir"),
            None,
            "the malformed line could have been the absent work_dir"
        );
        assert_eq!(
            value.get("goal"),
            None,
            "the malformed line could have been the absent goal"
        );
    }

    #[test]
    fn sc_405g_branch_preserves_legacy_healthy_and_degraded_shapes_pending_a_source_slice() {
        let healthy = Scratch::new("branch-observation-healthy");
        healthy.meta(META);
        let empty_roster = Scratch::new("branch-observation-empty-roster");
        empty_roster.meta("mode=local\n");
        let degraded = Scratch::new("branch-observation-degraded");
        degraded.meta("mode=local\nthis line has no equals sign\nagent.main=claude:lead\n");
        // Unimplemented observation is not a new degradation fact.
        let healthy = entry_for(&healthy.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        let empty_roster = entry_for(
            &empty_roster.0,
            "empty",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        let degraded = entry_for(
            &degraded.0,
            "damaged",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        assert_eq!(
            healthy.to_json().get("branch"),
            Some(&crate::json::Value::Null),
            "healthy legacy bytes survive until the SC-405g source slice"
        );
        assert!(
            !healthy.degraded,
            "unimplemented branch observation is not loss"
        );
        assert_eq!(empty_roster.agents, Vec::new());
        assert!(!empty_roster.degraded);
        assert_eq!(
            empty_roster.to_json().get("branch"),
            Some(&crate::json::Value::Null),
            "a readable empty roster keeps the healthy legacy branch bytes"
        );
        assert_eq!(
            degraded.to_json().get("branch"),
            None,
            "degraded legacy rows omit an absent branch"
        );
        assert!(degraded.degraded);
    }

    #[test]
    fn sc_509b_a_malformed_line_keeps_the_roster_unenumerable() {
        let scratch = Scratch::new("malformed-roster-completeness");
        scratch.meta("agent.main=claude:lead\nthis line has no equals sign\n");
        let entry = entry_for(
            &scratch.0,
            "damaged",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        let value = entry.to_json();

        assert!(entry.degraded);
        assert_eq!(
            value.get("needs_attention"),
            Some(&crate::json::Value::Bool(false))
        );
        assert_eq!(
            value.get("attention"),
            None,
            "a malformed line may name another roster member, so quiet is inexact"
        );
        assert_eq!(value.get("attention_rank"), None);
    }

    #[test]
    fn sc_509b_an_unknown_meta_key_does_not_make_the_roster_incomplete() {
        let scratch = Scratch::new("unknown-meta-key-roster");
        scratch.meta("agent.main=claude:lead\nfuture_meta_key=permitted\n");
        let entry = entry_for(
            &scratch.0,
            "tolerated",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        let value = entry.to_json();

        assert!(!entry.degraded);
        assert_eq!(value.get("attention"), Some(&crate::json::Value::Null));
        assert_eq!(
            value.get("attention_rank"),
            Some(&crate::json::Value::Num(0))
        );
    }

    #[test]
    fn sc_509b_a_duplicate_agent_key_makes_quiet_attention_inexact() {
        let scratch = Scratch::new("duplicate-agent-roster");
        scratch.meta("agent.main=claude:lead\nagent.main=claude:replacement\n");
        let entry = entry_for(
            &scratch.0,
            "damaged",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        let value = entry.to_json();

        assert!(entry.degraded);
        assert_eq!(
            value.get("needs_attention"),
            Some(&crate::json::Value::Bool(false))
        );
        assert_eq!(value.get("attention"), None);
        assert_eq!(value.get("attention_rank"), None);
    }

    #[test]
    fn sc_405k_a_meta_that_names_one_agent_is_not_rosterless() {
        // The neighbour of the case above: one agent is a roster.
        let scratch = Scratch::new("oneagent");
        scratch.meta("agent.main=claude:lead\n");
        let entry = entry_for(
            &scratch.0,
            "small",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        assert!(!entry.degraded);
        assert_eq!(entry.agents.len(), 1);
    }

    #[test]
    fn sc_519_a_session_with_meta_and_no_event_log_is_quiet_not_degraded() {
        let scratch = Scratch::new("noevents");
        scratch.meta(META);
        let entry = entry_for(
            &scratch.0,
            "fresh",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        assert!(!entry.degraded, "an absent event log is a quiet stream");
        assert_eq!(entry.last_active_epoch, None);
        assert_eq!(entry.mode.as_deref(), Some("local"));
        assert_eq!(entry.agents.len(), 1, "the roster still names its agents");
    }

    #[test]
    fn sc_520_a_skipped_malformed_record_degrades_the_session() {
        let scratch = Scratch::new("malformed");
        scratch.meta(META);
        fs::write(
            scratch.0.join("events.jsonl"),
            format!(
                "{}\nnot an event\n",
                event(&at(10), "claude:lead", "done", "")
            ),
        )
        .expect("writing a fixture");
        let entry = entry_for(
            &scratch.0,
            "damaged",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        assert!(entry.degraded, "loss must reach the public JSON");
        assert_eq!(
            entry.last_active_epoch,
            Some(NOW.epoch() - 10),
            "the model retains its partially read fact"
        );
        assert_eq!(
            entry.to_json().get("last_active_epoch"),
            None,
            "but the incomplete event source may not publish a stale current value"
        );
    }

    #[test]
    fn sc_509b_incomplete_events_omit_partial_current_members() {
        let scratch = Scratch::new("partial-events");
        scratch.meta(META);
        scratch.events(&[
            event(&at(600), "claude:lead", "goal", ""),
            event(&at(300), "claude:lead", "state", r#","ref":"blocked""#),
            "not an event".to_owned(),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        let value = entry.to_json();
        let Some(crate::json::Value::Arr(agents)) = value.get("agents") else {
            panic!("the readable roster stays present");
        };

        assert_eq!(entry.goal_set_epoch, Some(NOW.epoch() - 600));
        assert_eq!(entry.last_active_epoch, Some(NOW.epoch() - 300));
        assert_eq!(entry.agents[0].state.as_deref(), Some("blocked"));
        assert_eq!(value.get("goal_set_epoch"), None);
        assert_eq!(value.get("last_active_epoch"), None);
        assert_eq!(agents[0].get("state"), None);
        assert_eq!(agents[0].get("reason"), None);
        assert_eq!(value.get("attention"), None);
        assert_eq!(value.get("attention_rank"), None);
        assert_eq!(
            value.get("needs_attention"),
            Some(&crate::json::Value::Bool(true))
        );
    }

    #[test]
    fn sc_509b_runtime_dead_stays_exact_when_a_malformed_ledger_tail_is_lost() {
        let scratch = Scratch::new("runtime-dead-with-event-loss");
        scratch.meta(META);
        scratch.events(&["not an event".to_owned()]);
        let runtime = SessionRuntime {
            status: Status::Running,
            branch: None,
            agents: vec![AgentRuntime {
                slot: "main".to_owned(),
                alive: None,
                alert: Some(Reason::Dead),
            }],
        };
        let entry = entry_for(&scratch.0, "live", &runtime, NOW, DEFAULT_UNANSWERED_SECS);
        let value = entry.to_json();
        let Some(crate::json::Value::Arr(agents)) = value.get("agents") else {
            panic!("the readable roster remains present");
        };

        assert!(entry.degraded);
        assert_eq!(entry.attention, Some(Reason::Dead));
        assert_eq!(value.get_str("attention"), Some("dead"));
        assert_eq!(
            value.get("attention_rank"),
            Some(&crate::json::Value::Num(Reason::Dead.rank()))
        );
        assert_eq!(agents[0].get_str("reason"), Some("dead"));
        assert!(
            crate::listing::table(&[&entry]).contains("attn:dead"),
            "the human surface shares the exactness predicate"
        );
        assert!(
            crate::listing::table(&[&entry]).contains("unknown"),
            "event loss keeps the SC-017h declared-state cell unknown"
        );
    }

    /// The rendering guard agrees with git on every name git has a verdict for.
    ///
    /// The verdicts below were MEASURED with `git check-ref-format --branch`, not
    /// reasoned about: a guard checked only against its author's intuition agrees
    /// with that intuition by construction. The one deliberate divergence is
    /// non-ASCII, which is refused.
    #[test]
    fn the_branch_guard_matches_git_s_own_verdicts() {
        // (name, git check-ref-format --branch says valid)
        let measured = [
            ("main", true),
            ("release@2026", true),
            ("feature=api", true),
            ("fix.123", true),
            ("feature/a-b_1.2+x", true),
            ("@", true),
            ("a~b", false),
            ("a^b", false),
            ("a:b", false),
            ("a?b", false),
            ("a*b", false),
            ("a[b", false),
            ("a\\b", false),
            ("a b", false),
            ("a..b", false),
            ("-rf", false),
            ("a/", false),
            ("/a", false),
            ("a//b", false),
            ("a@{0}", false),
            ("a.lock", false),
            (".lock", false),
            ("a.b.lock", false),
            ("a.LOCK", true),
            ("lock", true),
            ("a.locket", true),
            ("a.", false),
        ];
        for (name, git_accepts) in measured {
            assert_eq!(
                super::safe_branch(name).is_some(),
                git_accepts,
                "{name:?}: the guard must agree with git check-ref-format --branch"
            );
        }

        // The one deliberate divergence, and the reason for it.
        for hostile in ["caf\u{e9}", "a\u{202e}b", "\u{4f60}\u{597d}"] {
            assert_eq!(
                super::safe_branch(hostile),
                None,
                "non-ASCII is refused even where git would accept it: a bidi \
                 override reorders every character after it on the line"
            );
        }
    }

    /// A hostile `HEAD` cannot reach the terminal.
    ///
    /// Rendering a decorative field is never worth executing attacker-chosen control
    /// bytes.
    #[test]
    fn a_hostile_head_renders_no_branch_rather_than_its_bytes() {
        let root = std::env::temp_dir().join(format!("ae-head-inject-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let repo = root.join("repo");
        fs::create_dir_all(repo.join(".git")).expect("a git dir");

        let hostile = [
            (
                "ref: refs/heads/a\u{1b}[31mRED",
                "an ANSI escape repaints the terminal",
            ),
            (
                "ref: refs/heads/a\rOVERWRITE",
                "a carriage return overwrites the line",
            ),
            (
                "ref: refs/heads/a\nfake  running  /evil",
                "a newline forges a whole row",
            ),
            ("ref: refs/heads/a\u{7}bell", "a control byte is not a name"),
            ("ref: refs/heads/../../etc/passwd", "dot-dot is refused"),
            (
                "ref: refs/heads/-rf",
                "a leading dash could be read as a flag",
            ),
            (
                "ref: refs/heads/a b",
                "a space would split a whitespace-parsed cell",
            ),
            ("ref: refs/heads/", "an empty name is not a name"),
            ("ref: refs/heads//a", "an empty leading segment"),
            ("ref: refs/heads/a//b", "an empty interior segment"),
            ("not a ref and not a hex object id", "an unparseable head"),
            // The earlier guard trimmed first and then counted lines, so both of these
            // normalised to one line and were accepted — the repair erased the evidence.
            ("\nref: refs/heads/main\n", "a LEADING blank line"),
            ("\n\nref: refs/heads/main\n", "two leading blank lines"),
            ("ref: refs/heads/main\n\n", "an EXTRA trailing blank line"),
            (
                "ref: refs/heads/main\n\nref: refs/heads/other\n",
                "a second ref line",
            ),
            ("ref: refs/heads/main\r\n", "carriage-return framing"),
            ("\rref: refs/heads/main", "a leading carriage return"),
        ];
        for (content, why) in hostile {
            fs::write(repo.join(".git/HEAD"), content).expect("HEAD");
            assert_eq!(
                super::branch_at(&repo),
                None,
                "{why}: {content:?} must not render"
            );
        }

        // The guard must not have swallowed the ordinary cases with them.
        for (content, expected) in [
            ("ref: refs/heads/main", "main"),
            ("ref: refs/heads/main\n", "main"),
            ("ref: refs/heads/feature/a-b_1.2+x\n", "feature/a-b_1.2+x"),
            // Both accepted by `git check-ref-format --branch`, and both rejected by the
            // first version of this guard.
            ("ref: refs/heads/release@2026\n", "release@2026"),
            ("ref: refs/heads/feature=api\n", "feature=api"),
            ("ref: refs/heads/fix.123\n", "fix.123"),
        ] {
            fs::write(repo.join(".git/HEAD"), content).expect("HEAD");
            assert_eq!(
                super::branch_at(&repo).as_deref(),
                Some(expected),
                "a legitimate ref still renders"
            );
        }

        let _ = fs::remove_dir_all(&root);
    }

    /// The branch read, in all three shapes it meets in the wild.
    ///
    /// The worktree shape is not exotic here — ae's own `--worktree` mode creates
    /// exactly it.
    #[test]
    fn the_branch_is_read_from_an_ordinary_clone_a_worktree_and_a_detached_head() {
        let root = std::env::temp_dir().join(format!("ae-branch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        // 1. Ordinary clone: `.git` is a directory.
        let clone = root.join("clone");
        fs::create_dir_all(clone.join(".git")).expect("a git dir");
        fs::write(clone.join(".git/HEAD"), "ref: refs/heads/rust-rewrite\n").expect("HEAD");
        assert_eq!(
            super::branch_at(&clone).as_deref(),
            Some("rust-rewrite"),
            "an ordinary clone reports its branch"
        );

        // A slashed branch keeps every slash after the refs/heads/ prefix.
        fs::write(clone.join(".git/HEAD"), "ref: refs/heads/feature/nested\n").expect("HEAD");
        assert_eq!(
            super::branch_at(&clone).as_deref(),
            Some("feature/nested"),
            "only the refs/heads/ prefix is stripped"
        );

        // 2. Worktree: `.git` is a FILE pointing at the real git dir.
        let real = root.join("real-git-dir");
        fs::create_dir_all(&real).expect("the pointed-to dir");
        fs::write(real.join("HEAD"), "ref: refs/heads/worktree-branch\n").expect("HEAD");
        let worktree = root.join("worktree");
        fs::create_dir_all(&worktree).expect("a worktree");
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", real.display()),
        )
        .expect("the pointer file");
        assert_eq!(
            super::branch_at(&worktree).as_deref(),
            Some("worktree-branch"),
            "a worktree follows its gitdir pointer"
        );

        // 3. Detached HEAD: a raw object id renders short, not as a branch.
        fs::write(
            clone.join(".git/HEAD"),
            "b6a492748114f89533d8b4629fe3c20048879cc0\n",
        )
        .expect("HEAD");
        assert_eq!(
            super::branch_at(&clone).as_deref(),
            Some("b6a4927"),
            "a detached head renders the short object id"
        );

        // 4. Not a repository at all, and a directory that is not there: a
        //    listing must never fail because a session's tree moved.
        assert_eq!(super::branch_at(&root).as_deref(), None, "no .git is None");
        assert_eq!(
            super::branch_at(&root.join("absent")).as_deref(),
            None,
            "a vanished work tree is None, never an error"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sc_017q_unknown_liveness_does_not_erase_a_handed_in_reason() {
        let scratch = Scratch::new("reason-with-unknown-pane");
        scratch.meta(META);
        let runtime = SessionRuntime {
            status: Status::Running,
            branch: None,
            agents: vec![AgentRuntime {
                slot: "main".to_owned(),
                alive: None,
                alert: Some(Reason::Blocked),
            }],
        };
        let entry = entry_for(&scratch.0, "live", &runtime, NOW, DEFAULT_UNANSWERED_SECS);

        assert_eq!(entry.agents[0].alive, None);
        assert_eq!(entry.agents[0].reason, Some(Reason::Blocked));
        assert_eq!(entry.attention, Some(Reason::Blocked));
    }

    #[test]
    fn sc_017q_stopped_liveness_does_not_manufacture_an_agent_reason() {
        let scratch = Scratch::new("stopped-without-reason");
        scratch.meta(META);
        let entry = entry_for(
            &scratch.0,
            "stopped",
            &SessionRuntime::new(Status::Stopped),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );

        assert_eq!(entry.agents[0].alive, Some(false));
        assert_eq!(entry.agents[0].reason, None);
        assert_eq!(entry.attention, None);
        assert_eq!(
            entry.to_json().get("attention"),
            Some(&crate::json::Value::Null)
        );
        assert!(
            !crate::listing::table(&[&entry]).contains("attn:"),
            "liveness never manufactures an attention class"
        );
    }

    #[test]
    fn sc_509b_an_unreadable_event_log_costs_the_declared_state_and_nothing_else() {
        // The roster came from the meta, which read fine.
        let scratch = Scratch::new("badlog");
        scratch.meta(META);
        fs::create_dir_all(scratch.0.join("events.jsonl")).expect("a directory in its place");
        let entry = entry_for(
            &scratch.0,
            "damaged",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        assert!(entry.degraded, "an existing log that will not read is loss");
        assert_eq!(entry.agents.len(), 1, "the roster survives");
        assert_eq!(entry.agents[0].reference, "claude:lead");
        assert_eq!(
            entry.agents[0].state, None,
            "but its declared state is gone"
        );
        assert_eq!(entry.last_active_epoch, None);
        assert_eq!(entry.mode.as_deref(), Some("local"));
    }

    #[test]
    fn sc_405d_an_unknown_meta_key_is_tolerated_and_never_degrades() {
        // Unknown keys are the NORMAL state of a real meta, so degrading on them
        // would make the flag constant-true.
        let scratch = Scratch::new("unknownkey");
        scratch.meta(&format!(
            "{META}ae_path=/usr/local/bin/ae\nlayout=vertical\nwatchdog=1234\n"
        ));
        let entry = entry_for(&scratch.0, "odd", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert!(!entry.degraded, "tolerated silently");
        assert_eq!(entry.mode.as_deref(), Some("local"));
        assert_eq!(entry.agents.len(), 1);
    }

    #[test]
    fn sc_405e_a_shape_the_reader_could_not_take_still_degrades() {
        // Each is a VALUE the reader could not accept, unlike a key it ignores.
        for (meta_text, why) in [
            (
                format!("{META}this line has no equals sign\n"),
                "malformed line",
            ),
            (format!("{META}goal=first\ngoal=second\n"), "duplicate key"),
            (
                format!("{META}agent.worker.0=justanalias\n"),
                "malformed roster value",
            ),
        ] {
            let scratch = Scratch::new("badshape");
            scratch.meta(&meta_text);
            let entry = entry_for(&scratch.0, "odd", &running(), NOW, DEFAULT_UNANSWERED_SECS);
            assert!(entry.degraded, "{why}");
        }
    }

    #[test]
    fn sc_405d_and_e_an_unknown_key_beside_a_malformed_one_still_degrades() {
        // The predicate is per-KIND, not "any anomaly" and not "no anomaly".
        let scratch = Scratch::new("mixedanomalies");
        scratch.meta(&format!(
            "{META}ae_path=/usr/local/bin/ae\nno equals here\n"
        ));
        let entry = entry_for(&scratch.0, "odd", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert!(entry.degraded);
    }

    #[test]
    fn sc_017g_amended_unanswered_is_a_session_fact_and_never_an_agent_reason() {
        // A pending ask is a PAIR fact with no owning agent, so no agents[].reason
        // ever reads "unanswered" — but it still reaches the session marker.
        let scratch = Scratch::new("unansweredrollup");
        scratch.meta(META);
        scratch.events(&[event(
            &at(7200),
            "claude:lead",
            "ask",
            r#","target":"codex:coworker","ref":"ae-1""#,
        )]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.attention,
            Some(Reason::Unanswered),
            "it reaches the marker"
        );
        assert!(
            entry.agents.iter().all(|agent| agent.reason.is_none()),
            "and no agent owns it"
        );
        assert_ne!(entry.agents[0].reason, Some(Reason::Unanswered));
    }

    #[test]
    fn sc_405b_and_c_the_meta_fills_the_context_fields_and_the_roster() {
        let scratch = Scratch::new("filled");
        scratch.meta(concat!(
            "mode=worktree\n",
            "origin=/home/c/projects/ae\n",
            "work_dir=/home/c/.ae/worktrees/x\n",
            "goal=ship the login flow\n",
            "agent.main=claude:lead:e795c9e9\n",
            "agent_bin.main=claude\n",
            "agent.worker.0=codex:coworker\n",
        ));
        scratch.events(&[event(&at(900), "claude:lead", "done", "")]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);

        assert!(!entry.degraded);
        assert_eq!(entry.mode.as_deref(), Some("worktree"));
        assert_eq!(entry.origin.as_deref(), Some("/home/c/projects/ae"));
        assert_eq!(entry.work_dir.as_deref(), Some("/home/c/.ae/worktrees/x"));
        assert_eq!(entry.goal.as_deref(), Some("ship the login flow"));
        assert_eq!(entry.last_active_epoch, Some(NOW.epoch() - 900));

        assert_eq!(entry.agents.len(), 2);
        assert_eq!(entry.agents[0].reference, "claude:lead");
        assert_eq!(entry.agents[0].alias, "claude");
        assert_eq!(entry.agents[0].name, "lead");
        assert_eq!(entry.agents[0].session_id.as_deref(), Some("e795c9e9"));
        assert_eq!(entry.agents[1].reference, "codex:coworker");
        assert_eq!(entry.agents[1].session_id, None);
    }

    #[test]
    fn frozen_meta_version_reaches_the_human_entry_but_not_the_digest() {
        let scratch = Scratch::new("human-version");
        scratch.meta(&format!("{META}ae_version=0.2.1\n"));
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);

        assert_eq!(entry.ae_version.as_deref(), Some("0.2.1"));
        assert_eq!(
            entry.to_json().get("ae_version"),
            None,
            "frozen's digest never carried the human-subline version"
        );
    }

    #[test]
    fn sc_405f_the_goal_epoch_comes_from_the_latest_goal_event_not_the_meta() {
        let scratch = Scratch::new("goalevent");
        scratch.meta(&format!("{META}goal=ship the login flow\n"));
        scratch.events(&[
            event(&at(7200), "claude:lead", "goal", ""),
            event(&at(600), "claude:lead", "goal", ""),
            event(&at(60), "claude:lead", "done", ""),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.goal.as_deref(), Some("ship the login flow"));
        assert_eq!(
            entry.goal_set_epoch,
            Some(NOW.epoch() - 600),
            "the LATEST goal event, and never a meta key"
        );
    }

    #[test]
    fn a_session_that_never_set_a_goal_has_no_goal_epoch() {
        let scratch = Scratch::new("nogoal");
        scratch.meta(META);
        scratch.events(&[event(&at(60), "claude:lead", "done", "")]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.goal_set_epoch, None);
    }

    #[test]
    fn sc_510c_an_agent_s_declared_state_reaches_the_digest() {
        let scratch = Scratch::new("declared");
        scratch.meta(META);
        scratch.events(&[
            event(&at(3600), "claude:lead", "state", r#","ref":"working""#),
            event(&at(600), "claude:lead", "state", r#","ref":"blocked""#),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.agents[0].state.as_deref(),
            Some("blocked"),
            "the LATEST"
        );
        assert_eq!(
            entry.agents[0].reason,
            Some(Reason::Blocked),
            "blocked is one of the two self-declared attention reasons"
        );
        assert_eq!(entry.attention, Some(Reason::Blocked), "and it rolls up");
    }

    #[test]
    fn sc_017g_working_and_done_are_states_but_not_reasons() {
        for declared in ["working", "done"] {
            let scratch = Scratch::new(&format!("state-{declared}"));
            scratch.meta(META);
            scratch.events(&[event(
                &at(600),
                "claude:lead",
                "state",
                &format!(r#","ref":"{declared}""#),
            )]);
            let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
            assert_eq!(entry.agents[0].state.as_deref(), Some(declared));
            assert_eq!(entry.agents[0].reason, None, "{declared}");
            assert_eq!(entry.attention, None, "{declared}");
        }
    }

    #[test]
    fn sc_511b_a_state_event_matches_its_agent_by_routing_key_when_it_carries_one() {
        // FIXTURE ORDER IS THE INSTRUMENT: the events that must NOT match are the
        // NEWEST, so latest-wins cannot rescue a matcher that accepts them. (It did
        // exactly that until cargo-mutants rewrote the matcher's `&&` to `||`.)
        let scratch = Scratch::new("routedstate");
        scratch.meta(META);
        scratch.events(&[
            // Should win: this session's main slot, under a churned display name.
            event(
                &at(900),
                "claude:renamed",
                "state",
                r#","ref":"waiting-user","actor_slot":"main","actor_session":"live""#,
            ),
            // Same slot, ANOTHER session. Newer, so a matcher that accepts it decides.
            event(
                &at(600),
                "claude:lead",
                "state",
                r#","ref":"blocked","actor_slot":"main","actor_session":"somewhere-else""#,
            ),
            // This session, ANOTHER slot. Newer still, for the same reason.
            event(
                &at(300),
                "claude:lead",
                "state",
                r#","ref":"done","actor_slot":"worker.0","actor_session":"live""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.agents[0].state.as_deref(),
            Some("waiting-user"),
            "neither half of the routing key alone identifies an agent"
        );
        assert_eq!(entry.agents[0].reason, Some(Reason::WaitingUser));
    }

    #[test]
    fn sc_405j_a_routed_event_with_a_stale_session_stays_unassociated() {
        // The display name matches the roster exactly; the routing key does not. A
        // loud false-negative beats a false attribution.
        let scratch = Scratch::new("stalesession");
        scratch.meta(META);
        scratch.events(&[event(
            &at(600),
            "claude:lead",
            "state",
            r#","ref":"blocked","actor_slot":"main","actor_session":"the-old-name""#,
        )]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].state, None);
        assert_eq!(entry.agents[0].reason, None);
    }

    #[test]
    fn sc_405j_a_partial_routing_key_identifies_nobody() {
        // Half a key is a key, and half a key routes nowhere.
        for partial in [
            r#","ref":"blocked","actor_slot":"main""#,
            r#","ref":"blocked","actor_session":"live""#,
        ] {
            let scratch = Scratch::new("partialkey");
            scratch.meta(META);
            scratch.events(&[event(&at(600), "claude:lead", "state", partial)]);
            let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
            assert_eq!(entry.agents[0].state, None, "{partial}");
        }
    }

    #[test]
    fn sc_405j_an_event_with_no_routing_key_at_all_still_matches_by_display_name() {
        // Every pre-routing-key event in an existing log looks like this.
        let scratch = Scratch::new("displayonly");
        scratch.meta(META);
        scratch.events(&[event(
            &at(600),
            "claude:lead",
            "state",
            r#","ref":"blocked""#,
        )]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].state.as_deref(), Some("blocked"));
    }

    #[test]
    fn sc_980_a_watchdog_alert_arrives_typed_and_outranks_a_self_declaration() {
        // Alert discrimination is an INPUT at this seam, never free text this reader
        // parses. An agent can be both blocked and dead; the more actionable wins.
        let scratch = Scratch::new("alert");
        scratch.meta(META);
        scratch.events(&[event(
            &at(600),
            "claude:lead",
            "state",
            r#","ref":"blocked""#,
        )]);
        let runtime = SessionRuntime {
            status: Status::Running,
            branch: Some("feature/login".to_owned()),
            agents: vec![AgentRuntime {
                slot: "main".to_owned(),
                alive: Some(false),
                alert: Some(Reason::Dead),
            }],
        };
        let entry = entry_for(&scratch.0, "live", &runtime, NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reason, Some(Reason::Dead));
        assert_eq!(
            entry.agents[0].state.as_deref(),
            Some("blocked"),
            "both are reported"
        );
        assert_eq!(entry.agents[0].alive, Some(false));
        assert_eq!(entry.attention, Some(Reason::Dead));
        assert_eq!(
            entry.branch.as_deref(),
            Some("feature/login"),
            "SC-405g input"
        );
    }

    #[test]
    fn sc_017q_an_agent_the_runtime_never_mentions_is_unknown_rather_than_dead() {
        // THIS TEST PINNED THE DEFECT: it used to assert `alive == false` for an
        // agent no observation ever mentioned — absence of evidence recorded as
        // evidence of absence. The session is RUNNING, so the pane observation
        // decides; there was none, so nothing is decided.
        let scratch = Scratch::new("noruntime");
        scratch.meta(META);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].alive, None, "unknown, never dead");
        assert_eq!(
            entry.agents[0].reason, None,
            "and an unknown agent does not manufacture an attention reason"
        );
    }

    #[test]
    fn sc_017q_the_session_grain_constrains_the_agent_grain_in_one_direction() {
        // Not a free Cartesian product: a stopped session proves every roster agent
        // dead, an unknown one leaves every agent unknown, and only a running session
        // defers to the pane observation.
        let scratch = Scratch::new("grain");
        scratch.meta(META);
        for (status, expected, why) in [
            (
                Status::Stopped,
                Some(false),
                "proven absent takes its panes with it",
            ),
            (
                Status::Unknown,
                None,
                "nothing observed about the session, nothing about its panes",
            ),
            (
                Status::Running,
                None,
                "running defers to a pane observation, and there was none",
            ),
        ] {
            let entry = entry_for(
                &scratch.0,
                "s",
                &SessionRuntime::new(status),
                NOW,
                DEFAULT_UNANSWERED_SECS,
            );
            assert_eq!(entry.agents[0].alive, expected, "{status:?}: {why}");
        }
    }

    #[test]
    fn sc_017q_a_running_session_permits_all_three_agent_answers() {
        let scratch = Scratch::new("all-three");
        scratch.meta(META);
        for (observed, expected) in [
            (Some(true), Some(true)),
            (Some(false), Some(false)),
            (None, None),
        ] {
            let mut runtime = SessionRuntime::new(Status::Running);
            runtime.agents = vec![AgentRuntime {
                slot: "main".to_owned(),
                alive: observed,
                alert: None,
            }];
            let entry = entry_for(&scratch.0, "s", &runtime, NOW, DEFAULT_UNANSWERED_SECS);
            assert_eq!(entry.agents[0].alive, expected, "observed {observed:?}");
        }
    }

    #[test]
    fn the_session_rollup_takes_the_most_actionable_reason_across_agents() {
        let scratch = Scratch::new("rollup");
        scratch.meta(concat!(
            "mode=local\norigin=/src\nwork_dir=/src\n",
            "agent.main=claude:lead\n",
            "agent.worker.0=codex:coworker\n",
        ));
        scratch.events(&[
            event(&at(600), "claude:lead", "state", r#","ref":"blocked""#),
            event(
                &at(600),
                "codex:coworker",
                "state",
                r#","ref":"waiting-user""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reason, Some(Reason::Blocked));
        assert_eq!(entry.agents[1].reason, Some(Reason::WaitingUser));
        assert_eq!(
            entry.attention,
            Some(Reason::WaitingUser),
            "waiting-user outranks blocked"
        );
    }

    #[test]
    fn an_unanswered_request_never_displaces_a_more_actionable_agent_reason() {
        let scratch = Scratch::new("unanswered-vs-dead");
        scratch.meta(META);
        scratch.events(&[event(
            &at(7200),
            "claude:lead",
            "ask",
            r#","target":"codex:coworker","ref":"ae-1""#,
        )]);
        let runtime = SessionRuntime {
            status: Status::Running,
            branch: None,
            agents: vec![AgentRuntime {
                slot: "main".to_owned(),
                alive: Some(true),
                alert: Some(Reason::Dead),
            }],
        };
        let entry = entry_for(&scratch.0, "live", &runtime, NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.attention, Some(Reason::Dead));
    }

    #[test]
    fn a_malformed_line_is_carried_out_of_the_read_rather_than_hidden() {
        let scratch = Scratch::new("skipped");
        fs::write(
            scratch.0.join("events.jsonl"),
            format!("{}\nnot an event\n", event(&at(10), "a:x", "done", "")),
        )
        .expect("writing a fixture");
        let read = SessionRead::open(&scratch.0).expect("the container reads");
        assert_eq!(read.skipped.len(), 1);
        assert_eq!(
            read.last_active,
            Some(Timestamp::from_epoch(NOW.epoch() - 10))
        );
    }

    // ---- the alert-derived evidence class ----------------------------------

    /// Two agents, so a test can prove an alert reaches the one it names and
    /// only that one.
    const PAIR_META: &str =
        "mode=local\nagent.main=fake:high:pending\nagent.worker.0=fake:low:pending\n";

    /// The corpus's own alert bytes at the ruled grain: the summary a real watchdog
    /// wrote, and the class the targeted agent owns.
    const CORPUS_ALERTS: [(&str, Reason); 3] = [
        ("agent process dead — dropped to shell", Reason::Dead),
        (
            "max nudges reached (no recent events), needs attention",
            Reason::Stale,
        ),
        ("throttled for 10s — may need attention", Reason::Throttled),
    ];

    #[test]
    fn sc_509c_an_alert_naming_an_agent_as_target_is_that_agents_own_reason() {
        // The defect this closes: every one of these rendered `reason: null` beside a
        // session `attention` naming the same class, so the digest said a session
        // needed a human and named no owner.
        for (summary, want) in CORPUS_ALERTS {
            let scratch = Scratch::new("alertderived");
            scratch.meta(META);
            scratch.events(&[event(
                &at(600),
                "_watchdog",
                "alert",
                &format!(r#","target":"claude:lead","summary":"{summary}""#),
            )]);
            let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
            assert_eq!(entry.agents[0].reason, Some(want), "{summary:?}");
            assert_eq!(
                entry.attention,
                Some(want),
                "and the session marker is the rollup over it, not a second derivation"
            );
        }
    }

    #[test]
    fn sc_509c_an_alert_reaches_only_the_agent_it_names() {
        // A session-level reason with no owner, and a reason smeared across the
        // roster, are the same error inverted.
        let scratch = Scratch::new("alertone");
        scratch.meta(PAIR_META);
        scratch.events(&[event(
            &at(600),
            "_watchdog",
            "alert",
            r#","target":"fake:high","summary":"agent process dead — dropped to shell""#,
        )]);
        let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reference, "fake:high");
        assert_eq!(entry.agents[0].reason, Some(Reason::Dead));
        assert_eq!(entry.agents[1].reference, "fake:low");
        assert_eq!(entry.agents[1].reason, None, "no carrier names fake:low");
        assert_eq!(entry.attention, Some(Reason::Dead));
    }

    #[test]
    fn sc_509c_each_agent_owns_its_own_class_and_the_session_takes_the_worst() {
        // The nudges are the instrument: they sit between the alerts and name the
        // worker as TARGET, so a matcher reading an inbound event as recovery would
        // clear the very alert that follows them.
        let scratch = Scratch::new("deadoverstale");
        scratch.meta(PAIR_META);
        scratch.events(&[
            event(
                &at(900),
                "_watchdog",
                "alert",
                r#","target":"fake:high","summary":"agent process dead — dropped to shell""#,
            ),
            event(
                &at(600),
                "watchdog",
                "nudge",
                r#","target":"fake:low","summary":"no recent events, no recent ae activity""#,
            ),
            event(
                &at(300),
                "watchdog",
                "nudge",
                r#","target":"fake:low","summary":"no recent events, no recent ae activity""#,
            ),
            event(
                &at(60),
                "_watchdog",
                "alert",
                r#","target":"fake:low","summary":"max nudges reached (no recent events), needs attention""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reason, Some(Reason::Dead));
        assert_eq!(entry.agents[1].reason, Some(Reason::Stale));
        assert_eq!(
            entry.attention,
            Some(Reason::Dead),
            "the session marker is the most actionable of the two, not the newest"
        );
    }

    #[test]
    fn sc_509c_the_agents_own_later_event_supersedes_the_alert() {
        // An alert is a claim about a moment, and the ledger keeps claiming it
        // forever. The agent's own later event supersedes it.
        let scratch = Scratch::new("superseded");
        scratch.meta(PAIR_META);
        scratch.events(&[
            event(
                &at(900),
                "_watchdog",
                "alert",
                r#","target":"fake:high","summary":"agent process dead — dropped to shell""#,
            ),
            event(
                &at(300),
                "fake:high",
                "ask",
                r#","target":"fake:low","ref":"ae-1","summary":"still here""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.agents[0].reason, None,
            "an agent that spoke after the alert is not dead"
        );
        assert_eq!(entry.attention, None);
    }

    #[test]
    fn sc_509c_the_agents_own_event_before_the_alert_does_not_rescue_it() {
        // The mirror of the rule above, and the one a `!=` in the wrong place satisfies.
        let scratch = Scratch::new("notrescued");
        scratch.meta(PAIR_META);
        scratch.events(&[
            event(
                &at(900),
                "fake:high",
                "ask",
                r#","target":"fake:low","ref":"ae-1","summary":"working""#,
            ),
            event(
                &at(300),
                "_watchdog",
                "alert",
                r#","target":"fake:high","summary":"agent process dead — dropped to shell""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reason, Some(Reason::Dead));
    }

    #[test]
    fn sc_509c_an_inbound_event_after_the_alert_is_not_recovery() {
        // Only the agent can prove it is back. A nudge is the watchdog asking the
        // very question the alert answers.
        for (actor, action) in [("watchdog", "nudge"), ("fake:low", "send")] {
            let scratch = Scratch::new("inbound");
            scratch.meta(PAIR_META);
            scratch.events(&[
                event(
                    &at(900),
                    "_watchdog",
                    "alert",
                    r#","target":"fake:high","summary":"agent process dead — dropped to shell""#,
                ),
                event(
                    &at(300),
                    actor,
                    action,
                    r#","target":"fake:high","summary":"anyone there?""#,
                ),
            ]);
            let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
            assert_eq!(
                entry.agents[0].reason,
                Some(Reason::Dead),
                "{actor}/{action}"
            );
        }
    }

    #[test]
    fn sc_509c_a_watchdog_clear_after_the_alert_retracts_it() {
        // The watchdog owns its own retractions, so a clear ends the scan without the
        // agent having to speak. No corpus fixture carries one.
        for clear in ["alert-cleared", "throttle-cleared"] {
            let scratch = Scratch::new("cleared");
            scratch.meta(PAIR_META);
            scratch.events(&[
                event(
                    &at(900),
                    "_watchdog",
                    "alert",
                    r#","target":"fake:high","summary":"throttled for 10s — may need attention""#,
                ),
                event(
                    &at(300),
                    "_watchdog",
                    clear,
                    r#","target":"fake:high","summary":"throttling cleared after 3 cycles""#,
                ),
            ]);
            let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
            assert_eq!(entry.agents[0].reason, None, "{clear}");
            assert_eq!(entry.attention, None, "{clear}");
        }
    }

    #[test]
    fn sc_509c_an_older_alert_survives_a_newer_one_being_absent() {
        // An ancient alert under a long quiet log is still the newest thing said
        // ABOUT the agent. Capping the scan would make deadness expire with nobody
        // having recovered.
        let scratch = Scratch::new("ancient");
        scratch.meta(PAIR_META);
        let mut lines = vec![event(
            &at(100_000),
            "_watchdog",
            "alert",
            r#","target":"fake:high","summary":"agent process dead — dropped to shell""#,
        )];
        for age in (100..900).step_by(100) {
            lines.push(event(
                &at(age),
                "fake:low",
                "memo",
                r#","ref":"design","summary":"somebody else's traffic""#,
            ));
        }
        scratch.events(&lines);
        let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reason, Some(Reason::Dead));
    }

    #[test]
    fn sc_405j_an_alert_addressed_by_routing_key_reaches_its_slot_and_no_other() {
        // FIXTURE ORDER IS THE INSTRUMENT, as in the declared-state test above.
        let scratch = Scratch::new("routedalert");
        scratch.meta(PAIR_META);
        scratch.events(&[
            // Should win: this session's main slot, under a churned name.
            event(
                &at(900),
                "_watchdog",
                "alert",
                concat!(
                    r#","target":"fake:renamed","summary":"agent process dead — dropped to shell""#,
                    r#","target_slot":"main","target_session":"pair""#,
                ),
            ),
            // Same slot, ANOTHER session. Newer.
            event(
                &at(600),
                "_watchdog",
                "alert",
                concat!(
                    r#","target":"fake:high","summary":"throttled for 10s — may need attention""#,
                    r#","target_slot":"main","target_session":"somewhere-else""#,
                ),
            ),
            // This session, ANOTHER slot. Newer still.
            event(
                &at(300),
                "_watchdog",
                "alert",
                concat!(
                    r#","target":"fake:high","summary":"max nudges reached, needs attention""#,
                    r#","target_slot":"worker.0","target_session":"pair""#,
                ),
            ),
        ]);
        let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.agents[0].reason,
            Some(Reason::Dead),
            "neither half of the routing key alone addresses an agent"
        );
        assert_eq!(
            entry.agents[1].reason,
            Some(Reason::Stale),
            "and the worker.0 alert lands on worker.0"
        );
    }

    #[test]
    fn sc_405j_an_alert_with_half_a_routing_key_addresses_nobody() {
        // Half a key that says main-slot-of-nowhere must not fall through to the
        // display name it also carries.
        for partial in [
            r#","target_slot":"main""#,
            r#","target_session":"pair""#,
            r#","target_slot":"","target_session":"pair""#,
        ] {
            let scratch = Scratch::new("partialalert");
            scratch.meta(PAIR_META);
            scratch.events(&[event(
                &at(600),
                "_watchdog",
                "alert",
                &format!(
                    r#","target":"fake:high","summary":"agent process dead — dropped to shell"{partial}"#
                ),
            )]);
            let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
            assert_eq!(entry.agents[0].reason, None, "{partial}");
        }
    }

    #[test]
    fn the_cross_session_spelling_of_this_sessions_agent_is_still_this_sessions_agent() {
        // `@<session>:<agent>` is the form the helpers pass through, so a locally
        // addressed record can wear it — and a foreign session's must not match.
        let scratch = Scratch::new("crosssession");
        scratch.meta(PAIR_META);
        scratch.events(&[event(
            &at(600),
            "_watchdog",
            "alert",
            r#","target":"@pair:fake:high","summary":"agent process dead — dropped to shell""#,
        )]);
        let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reason, Some(Reason::Dead));

        let other = Scratch::new("crosssessionforeign");
        other.meta(PAIR_META);
        other.events(&[event(
            &at(600),
            "_watchdog",
            "alert",
            r#","target":"@elsewhere:fake:high","summary":"agent process dead — dropped to shell""#,
        )]);
        let entry = entry_for(&other.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.agents[0].reason, None,
            "another session's agent of the same name is another agent"
        );
    }

    #[test]
    fn sc_509c_the_ledger_order_decides_when_a_clock_disagrees_with_it() {
        // BOTH CROSSED DIRECTIONS in one test, so neither can regress alone. Ordering
        // by `ts` gets exactly one of the two right, and the other one silently wrong.
        let alert = r#","target":"fake:high","summary":"agent process dead — dropped to shell""#;
        let spoke = r#","ref":"working","summary":"an independently clocked agent""#;

        // Appended LAST, stamped EARLIER: the alert stands. Timestamp order would
        // drop it and read the agent as recovered — the quiet failure.
        let late_alert = Scratch::new("ledgerlatealert");
        late_alert.meta(PAIR_META);
        late_alert.events(&[
            event(&at(300), "fake:high", "state", spoke),
            event(&at(900), "_watchdog", "alert", alert),
        ]);
        let entry = entry_for(
            &late_alert.0,
            "pair",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        assert_eq!(
            entry.agents[0].reason,
            Some(Reason::Dead),
            "an alert appended after the agent's own record is the newer claim, \
             whatever clock stamped it"
        );

        // Mirrored: the agent speaks LAST and is stamped EARLIER, so nothing stands.
        // Timestamp order would keep the alert — loud, but still wrong.
        let late_recovery = Scratch::new("ledgerlaterecovery");
        late_recovery.meta(PAIR_META);
        late_recovery.events(&[
            event(&at(300), "_watchdog", "alert", alert),
            event(&at(900), "fake:high", "state", spoke),
        ]);
        let entry = entry_for(
            &late_recovery.0,
            "pair",
            &running(),
            NOW,
            DEFAULT_UNANSWERED_SECS,
        );
        assert_eq!(
            entry.agents[0].reason, None,
            "and an agent that spoke last has recovered, whatever clock stamped it"
        );
    }

    #[test]
    fn two_decisive_records_at_one_instant_are_settled_by_the_later_one() {
        // A reader reaches for a tie-break here and there is deliberately none: the
        // later APPEND wins because it is the later record.
        let scratch = Scratch::new("tie");
        scratch.meta(PAIR_META);
        let same = at(600);
        scratch.events(&[
            event(
                &same,
                "_watchdog",
                "alert",
                r#","target":"fake:high","summary":"agent process dead — dropped to shell""#,
            ),
            event(
                &same,
                "_watchdog",
                "alert-cleared",
                r#","target":"fake:high","summary":"back""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reason, None);
    }

    #[test]
    fn sc_509c_an_agent_with_no_carrier_stays_null_beside_a_session_that_needs_a_human() {
        // One alert, one self-declaration, one aged unanswered ask between two OTHER
        // agents. Three reasons, two owners — the other two must stay null.
        const FOUR: &str = concat!(
            "mode=local\nagent.main=fake:high:pending\nagent.worker.0=fake:low:pending\n",
            "agent.worker.1=fake:third:pending\nagent.worker.2=fake:asker:pending\n",
        );
        let scratch = Scratch::new("competing");
        scratch.meta(FOUR);
        scratch.events(&[
            event(
                &at(900),
                "_watchdog",
                "alert",
                r#","target":"fake:high","summary":"agent process dead — dropped to shell""#,
            ),
            event(
                &at(600),
                "fake:low",
                "state",
                r#","ref":"waiting-user","summary":"asked the human""#,
            ),
            event(
                &at(100_000),
                "fake:asker",
                "ask",
                r#","target":"fake:third","ref":"ae-never-answered","summary":"a question""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "four", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reason, Some(Reason::Dead), "alert-derived");
        assert_eq!(
            entry.agents[1].reason,
            Some(Reason::WaitingUser),
            "self-declared"
        );
        assert_eq!(
            entry.agents[2].reason, None,
            "the TARGET of an unanswered ask owns nothing: unanswered is a pair fact"
        );
        assert_eq!(entry.agents[3].reason, None, "and neither does the asker");
        assert_eq!(entry.attention, Some(Reason::Dead));
    }

    #[test]
    fn sc_509c_a_handed_in_alert_and_a_ledger_alert_are_one_fact_in_one_rollup() {
        // A handed-in typed reason and the ledger route are not competing, and
        // neither silences the other. Both enter the same rollup.
        let scratch = Scratch::new("bothroutes");
        scratch.meta(PAIR_META);
        scratch.events(&[event(
            &at(600),
            "_watchdog",
            "alert",
            r#","target":"fake:low","summary":"throttled for 10s — may need attention""#,
        )]);
        let runtime = SessionRuntime {
            status: Status::Running,
            branch: None,
            agents: vec![AgentRuntime {
                slot: "main".to_owned(),
                alive: Some(true),
                alert: Some(Reason::Stale),
            }],
        };
        let entry = entry_for(&scratch.0, "pair", &runtime, NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.agents[0].reason,
            Some(Reason::Stale),
            "the handed-in reason still reaches its slot"
        );
        assert_eq!(
            entry.agents[1].reason,
            Some(Reason::Throttled),
            "and the ledger's alert still reaches the other"
        );
        assert_eq!(entry.attention, Some(Reason::Stale));
    }

    #[test]
    fn sc_509c_a_throttled_action_reaches_the_agent_it_names() {
        // A `throttled` action names its owner in `target` and its contribution in
        // the action itself, so no summary is read.
        let scratch = Scratch::new("throttledaction");
        scratch.meta(PAIR_META);
        scratch.events(&[
            event(
                &at(900),
                "fake:high",
                "state",
                r#","ref":"blocked","summary":"the higher-rank declaration""#,
            ),
            event(
                &at(600),
                "_watchdog",
                "throttled",
                r#","target":"fake:low","summary":"upstream throttling detected — pausing nudges""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].reason, Some(Reason::Blocked));
        assert_eq!(entry.agents[1].reason, Some(Reason::Throttled));
        assert_eq!(
            entry.attention,
            Some(Reason::Blocked),
            "blocked outranks throttled, and the NEWER record does not win the rollup"
        );
    }

    #[test]
    fn sc_509c_a_throttled_action_is_cleared_and_superseded_like_any_other_carrier() {
        // Currentness belongs to the CARRIER CLASS, not to the `alert` action, so both
        // ways a verdict stops being current are proven against `throttled` too.
        for ending in [
            // The watchdog retracts it.
            event(
                &at(300),
                "_watchdog",
                "throttle-cleared",
                r#","target":"fake:low","summary":"throttling cleared after 3 cycles""#,
            ),
            // The agent itself speaks: recovery.
            event(
                &at(300),
                "fake:low",
                "memo",
                r#","ref":"notes","summary":"back""#,
            ),
        ] {
            let scratch = Scratch::new("throttledended");
            scratch.meta(PAIR_META);
            scratch.events(&[
                event(
                    &at(900),
                    "_watchdog",
                    "throttled",
                    r#","target":"fake:low","summary":"upstream throttling detected""#,
                ),
                ending.clone(),
            ]);
            let entry = entry_for(&scratch.0, "pair", &running(), NOW, DEFAULT_UNANSWERED_SECS);
            assert_eq!(entry.agents[1].reason, None, "{ending}");
            assert_eq!(entry.attention, None, "{ending}");
        }
    }

    #[test]
    fn sc_509c_a_throttle_that_escalated_to_an_alert_reads_as_the_alert() {
        // Both carriers name the same agent and both say throttled, so the ANSWER
        // cannot discriminate. The assertion that can is that the newer record is
        // the one consulted.
        let scratch = Scratch::new("escalated");
        scratch.meta(META);
        scratch.events(&[
            event(
                &at(900),
                "_watchdog",
                "throttled",
                r#","target":"claude:lead","summary":"upstream throttling detected — pausing nudges""#,
            ),
            event(
                &at(600),
                "_watchdog",
                "alert",
                r#","target":"claude:lead","summary":"pane missing — agent no longer visible in session""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.agents[0].reason,
            Some(Reason::Dead),
            "the newest carrier decides, not the most severe and not the first"
        );
    }

    #[test]
    fn sc_510c_the_ledger_order_decides_a_declaration_when_a_clock_disagrees() {
        // THE CROSSED SEED: the LATER-APPENDED declaration carries the EARLIER
        // timestamp and must win. Under max-by-ts the agent reads `working` while the
        // ledger's last word is `blocked` — silently, both being legal states.
        let scratch = Scratch::new("statecrossed");
        scratch.meta(META);
        scratch.events(&[
            event(
                &at(300),
                "claude:lead",
                "state",
                r#","ref":"working","summary":"earlier append, later clock""#,
            ),
            event(
                &at(900),
                "claude:lead",
                "state",
                r#","ref":"blocked","summary":"later append, earlier clock""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.agents[0].state.as_deref(),
            Some("blocked"),
            "the last APPENDED declaration wins, whatever ts it claims"
        );
        assert_eq!(
            entry.agents[0].reason,
            Some(Reason::Blocked),
            "and it reaches the attention rollup, which is why the ordering is \
             not merely a display question"
        );
        assert_eq!(entry.attention, Some(Reason::Blocked));
    }

    #[test]
    fn sc_510c_the_ordinary_monotonic_case_is_unchanged() {
        // THE CONTROL: ts and append order agree, as in every corpus fixture. Without
        // it the crossed test alone could be satisfied by a reader that is backwards
        // for the wrong reason.
        let scratch = Scratch::new("statemonotonic");
        scratch.meta(META);
        scratch.events(&[
            event(
                &at(900),
                "claude:lead",
                "state",
                r#","ref":"working","summary":"first""#,
            ),
            event(
                &at(600),
                "claude:lead",
                "state",
                r#","ref":"blocked","summary":"second""#,
            ),
            event(
                &at(300),
                "claude:lead",
                "state",
                r#","ref":"waiting-user","summary":"third""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].state.as_deref(), Some("waiting-user"));
        assert_eq!(entry.agents[0].reason, Some(Reason::WaitingUser));
    }

    #[test]
    fn sc_510c_a_legacy_done_event_is_still_a_declaration() {
        // The frozen readers were split on a bare `done`, and the one corpus fixture
        // emitting both agrees by accident — which is why this needs a test.
        let scratch = Scratch::new("legacydone");
        scratch.meta(META);
        scratch.events(&[
            event(
                &at(600),
                "claude:lead",
                "state",
                r#","ref":"working","summary":"still going""#,
            ),
            event(
                &at(300),
                "claude:lead",
                "done",
                r#","summary":"finished, pre-state-helper shape""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.agents[0].state.as_deref(),
            Some("done"),
            "a bare `done` is the declaration a pre-state-helper session leaves"
        );
        assert_eq!(
            entry.agents[0].reason, None,
            "and `done` is not an attention reason"
        );
    }

    #[test]
    fn sc_510c_a_state_event_that_says_nothing_declares_nothing() {
        // A `state` event with no `ref` has not said WHAT, so it must not displace the
        // declaration before it.
        let scratch = Scratch::new("statenoref");
        scratch.meta(META);
        scratch.events(&[
            event(
                &at(600),
                "claude:lead",
                "state",
                r#","ref":"blocked","summary":"a real declaration""#,
            ),
            event(
                &at(300),
                "claude:lead",
                "state",
                r#","summary":"no ref at all""#,
            ),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.agents[0].state.as_deref(), Some("blocked"));
        assert_eq!(entry.agents[0].reason, Some(Reason::Blocked));
    }

    #[test]
    fn sc_405f_the_goal_epoch_is_the_last_appended_goal_not_the_best_stamped_one() {
        // THE OPPOSED-ORDER FIXTURE: 13:00 appended first, 12:00 second, so a
        // last-record reader and a max-timestamp reader cannot both be right. The
        // frozen digest renders the 12:00 one.
        let scratch = Scratch::new("goalopposed");
        scratch.meta(META);
        scratch.events(&[
            r#"{"ts":"2025-08-12T13:00:00Z","actor":"claude:lead","action":"goal","summary":"GOAL-TEXT-WITH-NEWER-TS"}"#.to_owned(),
            r#"{"ts":"2025-08-12T12:00:00Z","actor":"claude:lead","action":"goal","summary":"GOAL-TEXT-WITH-OLDER-TS"}"#.to_owned(),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(
            entry.goal_set_epoch,
            Some(1_755_000_000),
            "the LAST APPENDED goal supplies the epoch; 1755003600 is the \
             max-timestamp answer the row forbids"
        );
    }

    #[test]
    fn sc_405f_the_agreeing_order_is_unchanged() {
        // The control beside the opposed arm, for the same reason the declared-state
        // ruling needed one.
        let scratch = Scratch::new("goalagreeing");
        scratch.meta(META);
        scratch.events(&[
            r#"{"ts":"2025-08-12T12:00:00Z","actor":"claude:lead","action":"goal","summary":"first"}"#.to_owned(),
            r#"{"ts":"2025-08-12T13:00:00Z","actor":"claude:lead","action":"goal","summary":"second"}"#.to_owned(),
        ]);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert_eq!(entry.goal_set_epoch, Some(1_755_003_600));
    }
}
