//! The chat's "needs you" section: every roster seat whose attention verdict
//! says the human is needed, as a SNAPSHOT of the current set.
//!
//! PURE — no clock, no I/O, no tmux. The verdict is the one `ae list` prints,
//! `AgentEntry.reason`, computed by the caller through `session::entry_from`;
//! this module attributes it to its evidence and names every reason it cannot
//! be trusted.
//! What is printed when, and how, is the view's concern.

use crate::attention::Reason;
use crate::digest::SessionEntry;
use crate::events::Event;
use crate::meta::RosterEntry;
use crate::session::{AgentRuntime, RecordSnapshot, SessionRuntime};
use crate::time::Timestamp;
use crate::tmux::Evidence;

/// One roster seat, by the two facts that identify it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeatRef {
    /// `main` / `worker.<n>` / `spawned.<n>`.
    pub slot: String,
    /// The seat's name.
    pub name: String,
}

/// Why a seat's verdict cannot be trusted. One per seat, in this precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    /// The watchdog has never beaten for this session.
    WatchdogOff,
    /// The watchdog's beat is there and could not be read.
    WatchdogUnreadable,
    /// The watchdog's last beat is older than [`STALE_SECS`].
    WatchdogStale {
        /// When it last beat, epoch micros.
        last_micros: i64,
    },
    /// tmux did not answer which panes the session has.
    RuntimeUnread,
    /// tmux answered, but which pane is this seat's could not be proven.
    PaneUnproven,
    /// The journal was read with lines it could not parse.
    JournalPartial {
        /// How many lines were skipped.
        skipped: usize,
    },
}

/// What a seat's row says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The seat's attention reason, as `ae list` reads it.
    Reason(Reason),
    /// No reason is known and the source that would say so cannot be trusted.
    Unknown(Cause),
}

/// The evidence a row's reason is attributed to. It decides nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A journal alert the watchdog booked, by its action.
    Alert {
        /// The record's action, e.g. `human-prompt`, `limit`, `alert`.
        action: String,
    },
    /// The seat's current declaration.
    Declaration,
    /// No pane carries the seat's slot now.
    NoPane,
    /// A reason no evidence above explains.
    Unattributed,
    /// The row is an [`Verdict::Unknown`].
    Unverified,
}

/// One seat that needs the human, or whose verdict cannot be trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub seat: SeatRef,
    /// The seat is one of the session's lead pair.
    pub lead_pair: bool,
    pub verdict: Verdict,
    pub source: Source,
    /// A reason row whose source is off, stale or unreadable.
    pub stale: Option<Cause>,
    /// The deciding record's stamp, epoch micros.
    pub since_micros: Option<i64>,
    /// The deciding record's summary, RAW: the view makes it inert.
    pub detail: String,
    /// The deciding record's position in the snapshot's events.
    pub record: Option<usize>,
}

/// The current set, most severe first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    pub rows: Vec<Row>,
}

/// Watchdog cycles a beat may miss before it is stale: three at the daemon's
/// default interval, the cadence `launch_grace` also counts in.
const BEAT_STALE_CYCLES: u64 = 3;

/// A beat strictly older than this is stale. The bound follows the daemon's
/// DEFAULT interval: a daemon started with `--interval` publishes no cadence a
/// directory read can see.
pub const STALE_SECS: i64 =
    (BEAT_STALE_CYCLES * crate::watchdog_daemon::DEFAULT_INTERVAL_SECS).cast_signed();

/// Everything the fold reads, each already read by its own owner.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// The session's name.
    pub session: &'a str,
    /// The session's meta and journal, read together.
    pub snapshot: &'a RecordSnapshot,
    /// The session's entry, computed from `snapshot` and `runtime` by
    /// `session::entry_from`: each agent's `reason` is the verdict.
    pub entry: &'a SessionEntry,
    /// What tmux said of the session's panes.
    pub runtime: &'a SessionRuntime,
    /// Whether tmux answered at all.
    pub runtime_read: bool,
    /// The watchdog's beat.
    pub beat: Evidence,
    /// The meta's `layout` is `lead-pair`, so `worker.0` is of the lead pair.
    pub lead_pair: bool,
    /// Now.
    pub now: Timestamp,
}

/// The current set.
///
/// # Errors
///
/// Why there is no settled read: the view keeps the rows it showed and says so.
pub fn fold(inputs: &Inputs<'_>) -> Result<Section, String> {
    let Some(meta) = inputs.snapshot.meta.as_ref() else {
        return Err("meta unreadable".to_owned());
    };
    // A damaged meta may have dropped a seat, so it proves no row cleared.
    if crate::session::anomalies_degrade(meta.anomalies()) {
        return Err("meta damaged, the roster may be incomplete".to_owned());
    }
    let Some(read) = inputs.snapshot.events.as_ref() else {
        return Err("journal unreadable".to_owned());
    };
    let watchdog = match inputs.beat {
        Evidence::Silent => Some(Cause::WatchdogOff),
        Evidence::Unreadable => Some(Cause::WatchdogUnreadable),
        Evidence::At(epoch) if inputs.now.epoch().saturating_sub(epoch) > STALE_SECS => {
            Some(Cause::WatchdogStale {
                last_micros: micros(epoch),
            })
        }
        Evidence::At(_) => None,
    };
    let skipped = read.skipped.len();
    let mut rows: Vec<(usize, Row)> = Vec::new();
    for (place, seat) in meta.roster().iter().enumerate() {
        let reference = seat.reference();
        let runtime = inputs
            .runtime
            .agents
            .iter()
            .find(|agent| agent.slot == seat.slot);
        let cause = watchdog
            .or((!inputs.runtime_read).then_some(Cause::RuntimeUnread))
            .or(runtime
                .and_then(|agent| agent.alive)
                .is_none()
                .then_some(Cause::PaneUnproven))
            .or((skipped > 0).then_some(Cause::JournalPartial { skipped }));
        let reason = inputs
            .entry
            .agents
            .iter()
            .find(|agent| agent.reference == reference)
            .and_then(|agent| agent.reason);
        let (verdict, stale, found) = match (reason, cause) {
            (Some(reason), _) => (
                Verdict::Reason(reason),
                cause,
                attribute(inputs, &read.events, seat, runtime, reason),
            ),
            (None, Some(cause)) => (
                Verdict::Unknown(cause),
                None,
                Found::none(Source::Unverified),
            ),
            (None, None) => continue,
        };
        rows.push((
            place,
            Row {
                seat: SeatRef {
                    slot: seat.slot.clone(),
                    name: seat.name.clone(),
                },
                lead_pair: crate::watchdog_daemon::in_lead_pair(&seat.slot, inputs.lead_pair),
                verdict,
                source: found.source,
                stale,
                since_micros: found.since_micros,
                detail: found.detail,
                record: found.record,
            },
        ));
    }
    rows.sort_by_key(|(place, row)| {
        let rank = match row.verdict {
            Verdict::Reason(reason) => Some(reason.rank()),
            Verdict::Unknown(_) => None,
        };
        (
            std::cmp::Reverse(rank),
            row.since_micros.is_none(),
            row.since_micros,
            *place,
        )
    });
    Ok(Section {
        rows: rows.into_iter().map(|(_, row)| row).collect(),
    })
}

/// The evidence a reason was found in.
struct Found {
    source: Source,
    since_micros: Option<i64>,
    detail: String,
    record: Option<usize>,
}

impl Found {
    /// `source`, with no record behind it.
    fn none(source: Source) -> Self {
        Self {
            source,
            since_micros: None,
            detail: String::new(),
            record: None,
        }
    }

    /// `source`, decided by the record at `at`.
    fn record(events: &[Event], at: usize, source: Source) -> Self {
        let event = &events[at];
        Self {
            source,
            since_micros: Some(micros(event.ts.epoch())),
            detail: event.summary.clone().unwrap_or_default(),
            record: Some(at),
        }
    }
}

/// Where `reason` came from, in the halves `session::entry_from` rolls up: the
/// journal alert that raised it, the runtime's own alert, the declaration it
/// read. It decides nothing: `reason` is already decided.
fn attribute(
    inputs: &Inputs<'_>,
    events: &[Event],
    seat: &RosterEntry,
    runtime: Option<&AgentRuntime>,
    reason: Reason,
) -> Found {
    let (session, reference) = (inputs.session, seat.reference());
    let raised = crate::session::alert_record_since_in(
        events,
        session,
        &seat.slot,
        &reference,
        inputs.entry.started_epoch,
    );
    if let Some((at, _)) = raised.filter(|(_, raised)| *raised == reason) {
        let action = events[at].action.clone();
        return Found::record(events, at, Source::Alert { action });
    }
    if runtime.and_then(|agent| agent.alert) == Some(reason) {
        return Found::none(Source::NoPane);
    }
    let declared = crate::session::declaration_of(events, session, &seat.slot, &reference)
        .filter(|(event, _)| match event.declared_state() {
            Some("waiting-user") => reason == Reason::WaitingUser,
            Some("blocked" | "waiting-agent") => reason == Reason::Blocked,
            _ => false,
        })
        .and_then(|(event, _)| events.iter().position(|at| std::ptr::eq(at, event)));
    match declared {
        Some(at) => Found::record(events, at, Source::Declaration),
        None => Found::none(Source::Unattributed),
    }
}

/// An epoch second as epoch micros, the lane's clock.
fn micros(epoch: i64) -> i64 {
    epoch.saturating_mul(1_000_000)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::{Inputs, Section, Source, Verdict, fold};
    use crate::attention::Reason;
    use crate::digest::{SessionEntry, Status};
    use crate::events::{Cursor, Event};
    use crate::inventory::MetaRead;
    use crate::meta::Meta;
    use crate::session::{AgentRuntime, RecordSnapshot, SessionRead, SessionRuntime};
    use crate::time::Timestamp;
    use crate::tmux::Evidence;

    const NOW: i64 = 1_790_748_000;

    fn event(secs_ago: i64, actor: &str, action: &str, state: &str) -> Event {
        let ts = Timestamp::from_epoch(NOW - secs_ago);
        let line = format!(
            r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}","ref":"{state}","summary":"words"}}"#
        );
        Event::parse_line(&line).expect("a record")
    }

    struct Rig {
        snapshot: RecordSnapshot,
        runtime: SessionRuntime,
    }

    impl Rig {
        fn new(layout: &str, seats: &[(&str, &str)], events: Vec<Event>) -> Self {
            let mut raw = format!("schema=2\nlayout={layout}\n");
            for (slot, name) in seats {
                let _ = writeln!(raw, "seat.{slot}={name}");
            }
            let snapshot = RecordSnapshot {
                meta: Some(Meta::parse(&raw)),
                meta_read: MetaRead::Parsed,
                events: Some(SessionRead {
                    last_active: None,
                    events,
                    pending: Vec::new(),
                    cursor: Cursor::default(),
                    skipped: Vec::new(),
                }),
                legacy_created_epoch: None,
            };
            let mut runtime = SessionRuntime::new(Status::Running);
            runtime.branch = Some("b".to_owned());
            runtime.agents = seats
                .iter()
                .map(|(slot, _)| AgentRuntime {
                    slot: (*slot).to_owned(),
                    alive: Some(true),
                    alert: None,
                    observed: crate::harness_state::HarnessState::Unknown,
                })
                .collect();
            Self { snapshot, runtime }
        }

        fn entry(&self) -> SessionEntry {
            let now = Timestamp::from_epoch(NOW);
            crate::session::entry_from(&self.snapshot, "s", &self.runtime, now, 1800)
        }

        fn fold_with(&self, entry: &SessionEntry, lead_pair: bool) -> Section {
            fold(&Inputs {
                session: "s",
                snapshot: &self.snapshot,
                entry,
                runtime: &self.runtime,
                runtime_read: true,
                beat: Evidence::At(NOW),
                lead_pair,
                now: Timestamp::from_epoch(NOW),
            })
            .expect("a settled read")
        }
    }

    #[test]
    fn a_seat_no_pane_carries_is_attributed_to_the_runtime_with_no_record() {
        let mut rig = Rig::new("lead-pair", &[("main", "lead")], Vec::new());
        rig.runtime.agents[0].alive = Some(false);
        rig.runtime.agents[0].alert = Some(Reason::Dead);
        let rows = rig.fold_with(&rig.entry(), true).rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].verdict, Verdict::Reason(Reason::Dead));
        assert_eq!(rows[0].source, Source::NoPane);
        assert_eq!((rows[0].since_micros, rows[0].record), (None, None));
    }

    #[test]
    fn a_reason_no_evidence_explains_is_shown_unattributed_never_dropped() {
        let rig = Rig::new("lead-pair", &[("spawned.0", "scout")], Vec::new());
        let mut entry = rig.entry();
        entry.agents[0].reason = Some(Reason::Throttled);
        let rows = rig.fold_with(&entry, true).rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, Source::Unattributed);
        assert!(rows[0].detail.is_empty() && rows[0].since_micros.is_none());
    }

    #[test]
    fn worker_zero_is_of_the_lead_pair_only_in_a_lead_pair_layout() {
        let seats = [("main", "lead"), ("worker.0", "w")];
        for (layout, lead_pair) in [("lead-pair", true), ("vertical", false)] {
            let rig = Rig::new(layout, &seats, Vec::new());
            let mut entry = rig.entry();
            for agent in &mut entry.agents {
                agent.reason = Some(Reason::Unanswered);
            }
            let rows = rig.fold_with(&entry, lead_pair).rows;
            let pair: Vec<bool> = rows.iter().map(|row| row.lead_pair).collect();
            assert_eq!(pair, [true, lead_pair], "{layout}");
        }
    }

    #[test]
    fn an_escalated_waiting_agent_is_attributed_to_its_own_current_declaration() {
        let events = vec![event(7_200, "scout", "state", "waiting-agent")];
        let rig = Rig::new("lead-pair", &[("spawned.0", "scout")], events);
        let entry = rig.entry();
        assert_eq!(entry.agents[0].reason, Some(Reason::Blocked), "fixture");
        let rows = rig.fold_with(&entry, true).rows;
        assert_eq!(rows[0].source, Source::Declaration);
        assert_eq!(rows[0].record, Some(0));
        assert_eq!(rows[0].since_micros, Some((NOW - 7_200) * 1_000_000));
        assert_eq!(rows[0].detail, "words");
    }
}
