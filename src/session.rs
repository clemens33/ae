//! Reading a session directory — the parts the manifest rows decide.
//!
//! **Read-only, absolutely.** SC-1202's intended fix is "read/validation paths
//! never bootstrap", and the M2 pre-dispatch bootstrap is deliberately absent
//! from this crate. Nothing in this module creates, writes or touches a file.
//!
//! # What this reader derives, and from which row
//!
//! * `mode` / `origin` / `work_dir` / `goal` — the meta keys of SC-405b, read
//!   through [`crate::meta`].
//! * `agents[]` — the roster keys of SC-405c. **SC-405k**: membership is
//!   roster-defined, so a runtime-only slot never invents an agent, and a
//!   session with no roster degrades through SC-405i.
//! * `last_active_epoch` — SC-017e makes an **ae event** the activity clock, so
//!   the newest event's `ts` is when the session was last active.
//! * `goal_set_epoch` — SC-405f: the latest `goal` EVENT, never a meta key.
//! * `agents[].state` — the declared work state, which SC-510c (as amended)
//!   puts in a `state` event's `ref`.
//! * attention reasons — SC-017g: `waiting-user` and `blocked` from those
//!   declarations, `unanswered` from SC-518's request pairing. The marker is
//!   the MAX across agent reasons plus session-level unresolved-request facts,
//!   and `unanswered` is a pair fact that no agent owns.
//! * `degraded` — SC-509b's actual-loss test, gathered from every reader here.
//!
//! # What this reader must be TOLD
//!
//! Not gaps any more — ratified seams. Each is a fact no session directory
//! holds, so it arrives in [`SessionRuntime`] instead of being guessed:
//!
//! * **`status` and `alive`** — tmux facts. The contract never defines liveness
//!   detection as row behavior.
//! * **`branch`** — SC-405g: the live tmux branch with a git fallback.
//! * **`dead` / `stale` / `throttled`** — SC-980: successor alert events carry a
//!   typed reason, and free text is never a discriminator, so the decided
//!   reason is handed in rather than parsed out of prose here.
//!
//! # The known limitation, written down
//!
//! **SC-405j**: an event carrying a routing key whose session is stale — after a
//! rename — stays UNASSOCIATED rather than being matched by display name.
//! Attributing it by name would be a false attribution, and SC-518/SC-511b both
//! rule that the wrong direction to fail in. The state is lost loudly until
//! SC-977's stable identity lands at the P2 routing cutover.

use std::io;
use std::path::Path;

use crate::attention::Reason;
use crate::digest::{AgentEntry, SessionEntry, Status};
use crate::events::{
    Cursor, Drain, Event, EventLog, Identity, RefMeaning, RoutingMember, SkippedLine,
};
use crate::meta::{Anomaly, Meta};
use crate::time::Timestamp;

/// The `unanswered` threshold when nothing tunes it.
///
/// **SC-523** makes 1800s normative, and says implementations may take it as a
/// caller parameter — which every call here does. `AE_ATTN_REQUEST_SECS`'s
/// unset/override/malformed behavior stays with SC-1410j.
pub const DEFAULT_UNANSWERED_SECS: i64 = 1800;

/// An `ask` / `review` whose target has not replied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    /// The request id — SC-510c's `ref` for ask/review/reply.
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
    /// Every event read, in stream order. Kept because several digest fields
    /// are derived from it (SC-405f's goal epoch, SC-510c's declared states),
    /// and reading the file again per question would answer each from a
    /// different snapshot of a moving stream.
    pub events: Vec<Event>,
    /// The newest event's timestamp — SC-017e's activity clock.
    pub last_active: Option<Timestamp>,
    /// Requests still waiting on their target, oldest first.
    pub pending: Vec<PendingRequest>,
    /// Where a reader would resume (DR-001's generation + offset).
    pub cursor: Cursor,
    /// Lines that were not events, kept rather than dropped.
    pub skipped: Vec<SkippedLine>,
}

impl SessionRead {
    /// Read every generation of the event log under `dir`.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] when the log EXISTS and cannot be
    /// read, or when a cursor's history has gone (see [`EventLog::drain`]).
    ///
    /// A missing or zero-byte container on a FRESH read is NOT one of those
    /// cases: SC-519 makes it a quiet empty stream, so this returns `Ok` with
    /// no events. That exception is the whole distinction between a session
    /// that has not spoken yet and one whose words were lost — [`entry_for`]
    /// degrades the second under SC-509b and leaves the first alone.
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
    /// **SC-522**: age must EXCEED the threshold — equality is not past it.
    #[must_use]
    pub fn unanswered(&self, now: Timestamp, threshold_secs: i64) -> Vec<&PendingRequest> {
        self.pending
            .iter()
            .filter(|request| request.age_secs(now) > threshold_secs)
            .collect()
    }

    /// This session's event-derived contribution to the attention rollup.
    ///
    /// Exactly one of SC-017g's six reasons is derivable from the event stream
    /// alone; the module docs say which five are not, and why.
    #[must_use]
    pub fn attention_contribution(&self, now: Timestamp, threshold_secs: i64) -> Option<Reason> {
        if self.unanswered(now, threshold_secs).is_empty() {
            None
        } else {
            Some(Reason::Unanswered)
        }
    }

    /// When the goal was last set — SC-405f.
    ///
    /// NOT a meta key: the row is explicit that the digest derives this from
    /// the latest `goal` event, so a meta that carried such a key would not be
    /// consulted for it.
    #[must_use]
    pub fn goal_set_at(&self) -> Option<Timestamp> {
        self.events
            .iter()
            .filter(|event| event.action == "goal")
            .map(|event| event.ts)
            .max()
    }

    /// The work state `agent` last declared — SC-510c as amended.
    ///
    /// The declared value rides in `ref` on a `state` event. Identity follows
    /// SC-511b: the routing key when the event carries one for THIS session,
    /// the display name otherwise. A renamed session's older events fall back
    /// to the display name, which is why both paths exist rather than one.
    #[must_use]
    pub fn declared_state_of(&self, session: &str, slot: &str, reference: &str) -> Option<&str> {
        self.events
            .iter()
            .filter(|event| is_actor(event, session, slot, reference))
            .filter_map(|event| match event.ref_meaning() {
                RefMeaning::DeclaredState(state) => Some((event.ts, state)),
                _ => None,
            })
            .max_by_key(|(ts, _)| *ts)
            .map(|(_, state)| state)
    }

    /// Whether this read lost anything — SC-509b's "ACTUAL read/parse loss".
    ///
    /// SC-520: a skipped malformed COMPLETE record is loss and must reach the
    /// public JSON. A buffered unterminated tail is not (SC-975b), and the
    /// reader never reports one as skipped.
    #[must_use]
    pub fn lost_records(&self) -> bool {
        !self.skipped.is_empty()
    }
}

/// What one agent's RUNTIME says — facts no session directory holds.
///
/// `alive` is a tmux pane fact (Q4 seat confirmation: liveness detection is
/// never row behavior). `alert` is SC-980's typed reason: the successor's alert
/// events carry a key sufficient to discriminate dead | stale | throttled, and
/// free text is never a discriminator — so this reader is handed the decided
/// reason rather than parsing prose for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRuntime {
    /// The slot this describes — `main` / `worker.<n>` / `spawned.<n>`.
    pub slot: String,
    /// Whether the agent's pane is alive.
    pub alive: bool,
    /// The watchdog's typed reason for this agent, if any (SC-980).
    pub alert: Option<Reason>,
}

/// What a session's RUNTIME says — the facts SC-405g and SC-017a/b/c leave to
/// tmux and git rather than to the session directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRuntime {
    /// Running or stopped (SC-017a/b/c).
    pub status: Status,
    /// The live git branch (SC-405g — the watchdog's status segment, git
    /// fallback). Not a meta key.
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
/// **SC-405j** — an event that carries a routing key is matched on that key or
/// not at all. A stale session after a rename therefore leaves the event
/// UNASSOCIATED rather than attributed by display name: falling back would
/// invent an attribution, and SC-518/SC-511b both rule that direction the wrong
/// one to fail in. Rename loss is the documented known limitation until
/// SC-977's stable identity lands at the P2 routing cutover.
///
/// Display matching survives only for events with NO routing key at all — every
/// pre-SC-511a record in an existing log. A key that is half-given, or given
/// EMPTY, still counts as given: it identifies nobody rather than falling
/// through to a name (see [`RoutingMember`]).
fn is_actor(event: &Event, session: &str, slot: &str, reference: &str) -> bool {
    match (&event.actor_slot, &event.actor_session) {
        (RoutingMember::Value(event_slot), RoutingMember::Value(event_session)) => {
            event_slot == slot && event_session == session
        }
        // No routing key present at all: the display name is all there is, and
        // every pre-SC-511a record in an existing log depends on this arm.
        (RoutingMember::Absent, RoutingMember::Absent) => event.actor == reference,
        // Partial, or present-and-empty: routed, to nobody nameable.
        _ => false,
    }
}

/// The SC-509 entry for the session directory at `dir`.
///
/// Cannot fail. SC-506 says one bad session degrades its own entry and the
/// document always closes, and SC-509b says that degradation is VISIBLE:
/// `degraded: true` reaches the JSON whenever data was actually lost, so damage
/// never renders identically to legitimate sparsity.
///
/// What counts as loss, per the rows:
///
/// * the `meta` could not be read or held something SC-405d/e leave open
///   (unknown key, malformed line, duplicate key, malformed roster value);
/// * the event log EXISTS but could not be read (SC-519);
/// * a malformed COMPLETE record was skipped (SC-520).
///
/// What does not: an absent or zero-byte event log, which SC-519 rules is a
/// quiet stream and not damage.
#[must_use]
pub fn entry_for(
    dir: &Path,
    name: &str,
    runtime: &SessionRuntime,
    now: Timestamp,
    unanswered_secs: i64,
) -> SessionEntry {
    let mut entry = SessionEntry::new(name, runtime.status);
    entry.branch.clone_from(&runtime.branch);

    // SC-405i: a present session dir with a missing meta is degraded. Identity
    // beyond the directory name and the entire roster are lost at once, which
    // is actual loss by SC-509b's test — unlike a missing EVENT log, which
    // SC-519 makes quiet.
    let meta = Meta::read(dir).ok();
    if let Some(meta) = &meta {
        entry.degraded |= anomalies_degrade(meta.anomalies());
        // SC-405k routes a MISSING ROSTER through SC-405i: a session that
        // cannot name a single agent has lost the same thing a session with no
        // meta lost, and a readable-but-empty file is not evidence that a
        // session genuinely has no agents. An empty `agents` array with no
        // `degraded` beside it would assert exactly that.
        entry.degraded |= meta.roster().is_empty();
        entry.mode = meta.mode().map(ToOwned::to_owned);
        entry.origin = meta.origin().map(ToOwned::to_owned);
        entry.work_dir = meta.work_dir().map(ToOwned::to_owned);
        entry.goal = meta.goal().map(ToOwned::to_owned);
    } else {
        entry.degraded = true;
    }

    // SC-519 has already turned an absent or zero-byte log into a quiet stream,
    // so an error here is real loss: a log that EXISTS and will not read, a
    // record the reader could not take, or — once generations exist — a cursor
    // whose history is gone beneath it (DR-001). Every one of those is a fact
    // SC-509b says the digest must show rather than render as silence.
    let read = SessionRead::open(dir).ok();
    if let Some(read) = &read {
        entry.degraded |= read.lost_records();
        entry.last_active_epoch = read.last_active.map(Timestamp::epoch);
        entry.goal_set_epoch = read.goal_set_at().map(Timestamp::epoch);
    } else {
        entry.degraded = true;
    }

    // The roster comes from the META, so an unreadable event log costs the
    // agents their declared STATE and nothing else. SC-509b omits the facts
    // that were lost, not the ones that happen to sit next to them.
    if let Some(meta) = &meta {
        entry.agents = agent_entries(meta, read.as_ref(), runtime, name);
    }
    // SC-017g as AMENDED: the MAX across agent reasons PLUS session-level
    // unresolved-request facts. `unanswered` is a PAIR fact — a cross-session
    // ask makes target ownership non-local — so it joins the rollup here and
    // never appears as any agents[].reason.
    entry.attention = Reason::rollup(
        entry
            .agents
            .iter()
            .filter_map(|agent| agent.reason)
            .chain(read.and_then(|read| read.attention_contribution(now, unanswered_secs))),
    );
    entry
}

/// The `agents[]` array: the meta's roster, answered by the runtime and the
/// event stream.
///
/// **SC-405k** — membership is roster-defined (SC-405c). A runtime-only pane or
/// slot never invents an agent, because SC-509's `agents[]` fields ARE roster
/// fields; a missing roster routes through SC-405i instead.
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
                alias: slot.alias.clone(),
                name: slot.name.clone(),
                session_id: slot.session_id.clone(),
                alive: runtime_agent.is_some_and(|agent| agent.alive),
                state: declared.map(ToOwned::to_owned),
                // SC-017g: dead/stale/throttled come from the watchdog (SC-980
                // hands them here typed); waiting-user/blocked are
                // self-declared. An agent can be both, so the more actionable
                // one wins — the same rollup the session marker uses.
                reason: Reason::rollup(
                    runtime_agent
                        .and_then(|agent| agent.alert)
                        .into_iter()
                        .chain(declared.and_then(declared_reason)),
                ),
            }
        })
        .collect()
}

/// Whether a meta's anomalies degrade the session, per anomaly KIND.
///
/// **SC-405d, closed**: unknown keys are TOLERATED and never degrade. The
/// digest consumes only SC-405b/c and every other key passes silently — they
/// are the normal state of a real meta, not damage. SC-405h was rejected with
/// it, so there is deliberately no enumeration of the tolerated population here
/// to drift out of date.
///
/// A malformed line, a duplicate key or a malformed roster value still degrade:
/// each is ACTUAL loss by SC-509b's test — a value the reader could not take.
/// SC-405e still owes the exact malformed shapes, so that half stays interim.
fn anomalies_degrade(anomalies: &[Anomaly]) -> bool {
    anomalies.iter().any(|anomaly| match anomaly {
        Anomaly::UnknownKey { .. } => false,
        Anomaly::MalformedLine { .. }
        | Anomaly::DuplicateKey { .. }
        | Anomaly::MalformedRosterEntry { .. } => true,
    })
}

/// The attention reason a DECLARED work state contributes, if any.
///
/// SC-017g: `waiting-user` and `blocked` are the two self-declared reasons.
/// `working` and `done` are states, not reasons — an agent that is working does
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
/// **SC-518** — closure requires the FULL mirror match: the same `ref`
/// (SC-510c), the reply's actor is the request's target, AND the reply's target
/// is the request's actor. A reply from the right agent addressed to somebody
/// else does not close the request. The row states the reason as a direction:
/// a loud false-pending is safer than a silent false-closure, because the first
/// wastes a human's glance and the second loses the question entirely.
///
/// Identity is compared the way SC-511b and SC-518 say: routing keys when both
/// sides carry them, display names when neither does, and a MIXED pair matches
/// nothing.
///
/// Only the newest ask/review per `ref` is considered: a re-ask restarts the
/// clock on that request rather than leaving the original pending forever.
fn pending_requests(events: &[Event]) -> Vec<PendingRequest> {
    // One forward pass over an append-only log, so a reply can only ever close
    // a request already seen — a reply that appears BEFORE its request in the
    // file finds nothing open and closes nothing, which is the behavior a
    // separate "did it predate the request" check would have bought.
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

/// Whether `reply` closes `request` — the SC-518 mirror, in full.
fn closes(request: &Event, reply: &Event, reply_ref: &str) -> bool {
    if request.reference.as_deref() != Some(reply_ref) {
        return false;
    }
    let from_the_target = request
        .target_identity()
        .is_some_and(|target| same_participant(target, reply.actor_identity()));
    let to_the_asker = reply
        .target_identity()
        .is_some_and(|target| same_participant(target, request.actor_identity()));
    from_the_target && to_the_asker
}

/// Whether two identities name the same participant (SC-511b, SC-405j).
///
/// Routing keys compare to routing keys — that is the whole point of a
/// churn-proof key. When NEITHER side carries one, the display name is all
/// there is, and the row's own fallback applies.
///
/// Everything else is false, and the two ways that happens are worth naming
/// separately because both are loud-direction rulings:
///
/// * a MIXED pair — one side routed, the other display-only — has nothing in
///   common to compare (SC-518);
/// * an [`Identity::Unassociated`] side is half a routing key, and matches
///   nothing INCLUDING another `Unassociated`. Two events that each failed to
///   say where they came from have not thereby said the same thing.
fn same_participant(left: Identity<'_>, right: Identity<'_>) -> bool {
    match (left, right) {
        (
            Identity::Routed {
                slot: left_slot,
                session: left_session,
            },
            Identity::Routed {
                slot: right_slot,
                session: right_session,
            },
        ) => left_slot == right_slot && left_session == right_session,
        (Identity::Display(left), Identity::Display(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
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
        // "went unanswered PAST the threshold": at exactly the threshold it has
        // not passed it yet, one second later it has.
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
        // The half the seats reopened: right responder, wrong recipient. A
        // reply codex sent to a third agent is not an answer to lead's ask.
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
        // The reported defect: the reply carries actor_slot with no
        // actor_session, that half-key used to read as its display name, and
        // the display name matches the ask's target — so a reply that never
        // said where it came from closed the request.
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
        // Same defect mirrored: the REQUEST is the one with half a key, so its
        // target identity names nobody and nothing can answer it.
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
        // The reported defect, exactly: the reply declares itself routed with
        // an EMPTY member. Normalising that to absent made it (None,None) ->
        // Display, whose name matches the ask's target — so a malformed reply
        // closed a request it never answered.
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
        // The same rule on the other consumer of identity: a declared state
        // whose routing key is empty belongs to nobody, so it never reaches an
        // agent's `state` — even though the display name matches the roster.
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
        // Request routed, reply display-only: there is no common key to compare,
        // and SC-518 says mixed matches nothing rather than guessing.
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
        // Meta absence is real loss: mode, origin, work_dir, goal and the whole
        // roster are gone. Unlike the event log (SC-519), no row makes a
        // missing meta ordinary.
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
    fn sc_405k_a_rosterless_meta_degrades_and_still_emits_an_agents_array() {
        // A readable meta that names no agent has lost what a missing meta
        // loses. `agents` stays an ARRAY (SC-509b) — the loss is stated by the
        // flag, not by a missing or null field.
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
            assert!(entry.degraded, "{why}");
            assert!(entry.agents.is_empty(), "{why}");
            assert_eq!(
                entry.to_json().get("agents"),
                Some(&crate::json::Value::Arr(vec![])),
                "{why}: agents stays an array"
            );
            assert_eq!(
                entry.to_json().get("degraded"),
                Some(&crate::json::Value::Bool(true)),
                "{why}"
            );
        }
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
            "and the records that DID read are still reported"
        );
    }

    #[test]
    fn sc_509b_an_unreadable_event_log_costs_the_declared_state_and_nothing_else() {
        // The roster came from the meta, which read fine. Omitting the agents
        // too would drop a fact that was never lost.
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
        // Closed the other way from the interim: unknown keys are the NORMAL
        // state of a real meta, so degrading on them would make the flag
        // constant-true and stop it discriminating anything. The fixture no
        // longer uses `tmux_server` for this: SC-405l took that exact family out
        // of SC-405d's catch-all, so it is read now rather than tolerated.
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
        // Each of these is a VALUE the reader could not accept — actual loss by
        // SC-509b's test, unlike a key it simply does not consume.
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
        // The amendment: MAX across agent reasons PLUS session-level
        // unresolved-request facts. A pending ask is a PAIR fact with no owning
        // agent, so no agents[].reason ever reads "unanswered" — but it still
        // reaches the session marker.
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
        // FIXTURE ORDER IS THE INSTRUMENT HERE. The events that must NOT match
        // are the NEWEST ones: latest-wins would otherwise rescue the assertion
        // from a matcher that accepts them, and the test would pass while
        // proving nothing about the property in its own name. (It did exactly
        // that until cargo-mutants rewrote the matcher's `&&` to `||` and every
        // assertion still held.)
        let scratch = Scratch::new("routedstate");
        scratch.meta(META);
        scratch.events(&[
            // The one that SHOULD win: this session's main slot, under a
            // display name that has churned since.
            event(
                &at(900),
                "claude:renamed",
                "state",
                r#","ref":"waiting-user","actor_slot":"main","actor_session":"live""#,
            ),
            // Same slot, ANOTHER session. Newer, so a matcher that accepts it
            // changes the answer.
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
        // The display name matches the roster exactly. The routing key does
        // not. SC-405j: the event stays unassociated rather than being
        // attributed by name — a loud false-negative beats a false attribution.
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
        // "display matching remains only for events with NO routing keys at
        // all". Half a key is a key, and half a key routes nowhere.
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
        // Every pre-SC-511a event in an existing log looks like this, and
        // dropping display matching entirely would lose all of them.
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
        // The seat ruling: alert discrimination is an INPUT at this seam, never
        // free text this reader parses. An agent can be both blocked and dead;
        // SC-017g says the more actionable reason wins.
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
                alive: false,
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
        assert!(!entry.agents[0].alive);
        assert_eq!(entry.attention, Some(Reason::Dead));
        assert_eq!(
            entry.branch.as_deref(),
            Some("feature/login"),
            "SC-405g input"
        );
    }

    #[test]
    fn an_agent_the_runtime_never_mentions_is_not_alive() {
        let scratch = Scratch::new("noruntime");
        scratch.meta(META);
        let entry = entry_for(&scratch.0, "live", &running(), NOW, DEFAULT_UNANSWERED_SECS);
        assert!(!entry.agents[0].alive);
        assert_eq!(entry.agents[0].reason, None);
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
                alive: true,
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
}
