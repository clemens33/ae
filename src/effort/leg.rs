//! #253's live half: the watchdog's step, and the `_auto-effort` leg that
//! restarts a lead-pair seat IN PLACE at another effort, its conversation kept
//! (R1). The leg re-decides under the seat lock and reserves at the stop's last
//! boundary, so a crash leaves a trace the next leg settles (plan D2–D4).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{
    APPLIED, Decision, Direction, DiscardSeat, EPISODE, FAILED, Fold, Foreign, HELD, Leg, Level,
    REARMED, RESERVED, RESTORED, Record, State, Stored, Transition, Twin, TwinRefusal, YIELDED,
};
use crate::events::{Event, SessionKey};
use crate::inventory::ServerId;
use crate::procs::Proc;
use crate::time::Timestamp;

/// The judged percentage a new episode opens at: fixed (lead R3).
pub(crate) const AT: u8 = 80;

const USAGE: &str = "Usage: ae _auto-effort <dir> <slot> <episode>";

/// How long a restarted seat has to show its target effort (plan D2.4).
const PROOF_BOUND: Duration = Duration::from_secs(20);

/// How long the step waits before starting another leg for one seat.
const LEG_GAP: i64 = 15;

/// Whether ae moves the effort of the seat at `slot`: the lead pair alone (R2).
#[must_use]
pub(crate) fn eligible(slot: &str) -> bool {
    matches!(slot, "main" | "worker.0")
}

/// The pane facts a restart waits on, as one reading saw them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Point {
    /// An idle frame, an empty box and no prompt only the human may answer.
    pub(crate) ready: bool,
    /// The newest client input on the pane, in epoch seconds.
    pub(crate) input: Option<i64>,
    /// An auto-reseat attempt of the seat is in flight.
    pub(crate) reseating: bool,
    /// The pane's own process.
    pub(crate) pane_pid: Option<u32>,
}

/// One lead-pair seat as the watchdog's cycle saw it.
pub(crate) struct Sight {
    pub(crate) slot: String,
    pub(crate) capture: String,
    /// The seat's vendor usage limit is latched.
    pub(crate) limited: bool,
    pub(crate) point: Point,
}

/// What the step carries between cycles, in memory.
#[derive(Debug, Default)]
pub(crate) struct Carry {
    noted: bool,
    legs: Vec<(String, i64)>,
}

/// The cycle facts the step reads.
pub(crate) struct Cycle<'a> {
    pub(crate) dir: &'a Path,
    pub(crate) session: &'a str,
    pub(crate) events: &'a [Event],
    pub(crate) quota: Option<&'a crate::quota::Observation>,
    pub(crate) auto: &'a crate::autoreseat::Settings,
    pub(crate) now: i64,
}

/// One seat, read as `_run` reads it, with its effort record and episode.
struct Seat {
    session: String,
    session_id: String,
    entry: crate::meta::RosterEntry,
    run: crate::run::Seat,
    bytes: Vec<u8>,
    stored: Stored,
    fold: Fold,
    guard: Option<Result<(), Foreign>>,
    twin: Result<Twin, TwinRefusal>,
    resumable: bool,
    pin: Option<String>,
}

impl Seat {
    fn slot(&self) -> &str {
        &self.entry.slot
    }

    fn bin(&self) -> &str {
        self.entry.binary.as_deref().unwrap_or_default()
    }

    /// The meta rows a guarded effort write proves.
    fn proof(&self) -> crate::meta::EffortProof<'_> {
        crate::meta::EffortProof {
            session_id: &self.session_id,
            slot: &self.entry.slot,
            agent: &self.entry.name,
            tool: crate::tool::ToolKind::from_binary_name(self.bin()),
            launch_id: &self.run.launch_id,
            rows: crate::meta::effort_claims(&self.bytes, &self.entry.slot)
                .into_iter()
                .map(<[u8]>::to_vec)
                .collect(),
        }
    }

    fn episode(&self) -> Option<&super::Episode> {
        match &self.fold {
            Fold::Open(episode) => Some(episode),
            Fold::Armed | Fold::Doubt(_) => None,
        }
    }

    /// The ONE decision, against `quota` and the frame `capture` draws.
    fn decide(
        &self,
        quota: Option<&crate::quota::Observation>,
        sight: (&str, bool),
        attempts: bool,
        now: i64,
    ) -> Decision {
        let (capture, limited) = sight;
        let scope = super::own_scope(quota, &self.entry, self.pin.as_deref(), now);
        let live = crate::harness_state::observed_identity(capture, self.run.tool).effort;
        super::decide(&super::Inputs {
            at: AT,
            new_attempts: attempts,
            scope: scope.as_ref(),
            limited,
            fold: &self.fold,
            stored: &self.stored,
            guard: self.guard,
            twin: &self.twin,
            live: live.as_deref().and_then(Level::parse),
            now,
        })
    }

    /// Append one effort record as the watchdog.
    fn append(&self, dir: &Path, action: &str, reference: &str, summary: &str, now: i64) -> bool {
        let routed = DiscardSeat {
            session: &self.session,
            session_id: &self.session_id,
            slot: &self.entry.slot,
            agent: &self.entry.name,
        };
        let at = Timestamp::from_epoch(now);
        let line = super::line(at, &routed, action, reference, summary);
        crate::store::open(dir)
            .append_event(&format!("{line}\n"))
            .is_ok()
    }
}

/// The seat at `slot`, its journal read strictly; `None` on any gap.
fn read(dir: &Path, session: &str, slot: &str) -> Option<Seat> {
    let events = crate::autotrust::ledger(crate::store::open(dir).events_source()).ok()?;
    read_with(dir, session, slot, &events)
}

/// The seat at `slot`, its episode folded from `events`.
fn read_with(dir: &Path, session: &str, slot: &str, events: &[Event]) -> Option<Seat> {
    let bytes = crate::meta::read_bytes(dir).ok()?;
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let entry = meta
        .roster()
        .iter()
        .find(|entry| entry.slot == slot)?
        .clone();
    let run = crate::run::read_seat(dir, slot, None).ok()?;
    let session_id = meta.session_id()?.to_owned();
    let stored = meta.effort(slot);
    let key = SessionKey::new(session, &session_id);
    let fold = super::fold(events, key, slot, &entry.name);
    let guard = match &stored {
        Stored::Present(record) => Some(crate::run::effort_guard(record, &run, &meta, slot, true)),
        Stored::Absent | Stored::Damaged(_) => None,
    };
    let (twin, resumable) = crate::run::effort_twin(&run, &entry);
    let pin = crate::launch_cmd::model_flag_value(run.command.as_str(), run.tool);
    Some(Seat {
        session: session.to_owned(),
        session_id,
        entry,
        run,
        bytes,
        stored,
        fold,
        guard,
        twin,
        resumable,
        pin,
    })
}

/// The global config file.
fn global() -> Option<PathBuf> {
    crate::state_root().map(|root| crate::doors::config_file(crate::shape::current(), &root))
}

/// The switch, and the headroom threshold that bars new step downs (R3).
fn switch(auto: &crate::autoreseat::Settings) -> (bool, Option<u8>) {
    let on =
        crate::config::global_auto_effort(global().as_deref()) == crate::config::AutoEffort::On;
    let inverted = auto
        .headroom_at
        .filter(|at| auto.switch != crate::autoreseat::Switch::Off && *at <= AT);
    (on, inverted)
}

/// Why the seat is not restarted now: `Some("")` holds unnamed, a reason is
/// named once per episode, `None` restarts. The step and the leg both ask.
fn holds(dir: &Path, seat: &Seat, point: &Point, capture: &str, now: i64) -> Option<&'static str> {
    let viewed = point
        .input
        .is_some_and(|at| now.saturating_sub(at) < crate::deliver::VIEW_GRACE);
    if !point.ready || viewed || point.reseating || inbound(dir, seat) {
        return Some("");
    }
    if let Some(why) = model_hold(capture, seat.run.tool, seat.pin.as_deref()) {
        return Some(why);
    }
    if !seat.resumable {
        return Some("unresumable");
    }
    match tool_of(seat, point.pane_pid)
        .and_then(|(table, tool)| crate::procs::background_task(Some(&table), tool))
    {
        Some(false) => None,
        Some(true) => Some("background-task"),
        None => Some(""),
    }
}

/// Why the frame holds a restart for its model, so a drift is never restarted
/// over: `None` when it draws the model `pin` names, `Some("")` when it draws
/// none, and a drift named once.
fn model_hold(
    capture: &str,
    tool: crate::tool::ToolKind,
    pin: Option<&str>,
) -> Option<&'static str> {
    let drawn = crate::harness_state::observed_identity(capture, tool);
    match crate::model_drift::decide(&drawn, pin, tool.adapter().pin_match) {
        crate::model_drift::Decision::Retire => None,
        crate::model_drift::Decision::Silent => Some(""),
        crate::model_drift::Decision::Record { .. } => Some("model-drift"),
    }
}

/// The newest input of a client viewing `pane`, in epoch seconds; `Ok(None)`
/// when no client views it. `Err` when the clients were not read or one
/// viewing `pane` carries no usable stamp: neither proves the pane quiet.
fn client_input(
    clients: Option<&[crate::tmux::ObservedClient]>,
    pane: &str,
) -> Result<Option<i64>, ()> {
    let stamps: Option<Vec<i64>> = clients
        .ok_or(())?
        .iter()
        .filter(|client| client.pane == pane)
        .map(|client| client.activity.and_then(|at| i64::try_from(at).ok()))
        .collect();
    stamps.map(|stamps| stamps.into_iter().max()).ok_or(())
}

/// An ask or review to the seat that is not answered yet (plan D6).
fn inbound(dir: &Path, seat: &Seat) -> bool {
    let key = SessionKey::new(&seat.session, &seat.session_id);
    crate::requests::states_in(&crate::store::open(dir).container(), key)
        .iter()
        .any(|request| {
            request.status == crate::requests::Status::Pending && request.sent_to(key, seat.slot())
        })
}

/// ONE snapshot with state flags, and the seat's one recorded tool under the pane.
fn tool_of(seat: &Seat, pane_pid: Option<u32>) -> Option<(Vec<(Proc, String)>, u32)> {
    let table = crate::procs::stat_snapshot()?;
    let pid = crate::procs::tool_in(&table, pane_pid?, seat.bin())?;
    Some((table, pid))
}

/// The watchdog's effort step: per lead-pair seat ONE decision, journaled
/// under the seat lock when it needs no restart; at most ONE leg a cycle.
///
/// # Errors
///
/// Only a failure to write `err`.
pub(crate) fn step(
    cycle: &Cycle<'_>,
    seats: &[Sight],
    carry: &mut Carry,
    err: &mut impl Write,
) -> io::Result<()> {
    let (on, inverted) = switch(cycle.auto);
    if !on {
        return Ok(());
    }
    if let Some(at) = inverted.filter(|_| !carry.noted) {
        carry.noted = true;
        writeln!(
            err,
            "ae: auto_effort: no new step down while auto_reseat_at is {at} (at most {AT}): auto reseat moves the seat first"
        )?;
    }
    let mut started = false;
    for sight in seats {
        let Some(seat) = read_with(cycle.dir, cycle.session, &sight.slot, cycle.events) else {
            continue;
        };
        let decision = seat.decide(
            cycle.quota,
            (&sight.capture, sight.limited),
            inverted.is_none(),
            cycle.now,
        );
        let key = seat.episode().map_or(cycle.now, |episode| episode.key);
        let named = |why: &str| {
            seat.episode()
                .is_some_and(|episode| episode.named.iter().any(|named| named == why))
        };
        let (action, summary) = match decision {
            Decision::Open { threshold } => (EPISODE, format!("threshold {threshold}")),
            Decision::Name(why) => (HELD, why.to_owned()),
            Decision::Yield(why) => (YIELDED, why.to_owned()),
            Decision::Rearm => (REARMED, "relieved".to_owned()),
            Decision::Retire => {
                let _ = locked(cycle.dir, &seat, |_| {
                    let identity = crate::meta::effort_identity(&seat.bytes, seat.slot());
                    crate::meta::publish_effort(cycle.dir, &seat.proof(), &identity, None)
                });
                continue;
            }
            Decision::Down { .. } | Decision::Up | Decision::Reconcile(_) if !started => {
                if !matches!(decision, Decision::Reconcile(_)) {
                    match holds(cycle.dir, &seat, &sight.point, &sight.capture, cycle.now) {
                        None => {}
                        Some(why) if why.is_empty() || named(why) => continue,
                        Some(why) => {
                            let _ = locked(cycle.dir, &seat, |seat| {
                                seat.append(cycle.dir, HELD, &key.to_string(), why, cycle.now)
                            });
                            continue;
                        }
                    }
                }
                carry
                    .legs
                    .retain(|(_, at)| cycle.now.saturating_sub(*at) < LEG_GAP);
                if carry.legs.iter().any(|(slot, _)| *slot == sight.slot) {
                    continue;
                }
                let leg =
                    crate::session_launch::capture::auto_effort_argv(cycle.dir, &sight.slot, key);
                started = crate::shape::resolved_exe()
                    .zip(leg)
                    .is_some_and(|(exe, leg)| crate::transport::spawn_detached(&exe, &leg));
                carry.legs.push((sight.slot.clone(), cycle.now));
                continue;
            }
            _ => continue,
        };
        let reference = key.to_string();
        let _ = locked(cycle.dir, &seat, |seat| {
            seat.append(cycle.dir, action, &reference, &summary, cycle.now)
        });
    }
    Ok(())
}

/// Run `write` under the seat's lock, taken without waiting, and only while
/// the strict journal still folds as `seat` was judged.
fn locked(dir: &Path, seat: &Seat, write: impl FnOnce(&Seat) -> bool) -> bool {
    let Ok(_held) = crate::store::lock(
        &crate::autoreseat::lock_path(dir, seat.slot()),
        Duration::ZERO,
    ) else {
        return false;
    };
    read(dir, &seat.session, seat.slot()).is_some_and(|now| now.fold == seat.fold) && write(seat)
}

/// The leg's world: the seat's pane, and its identity and account rows as the
/// leg found them.
struct Ctx<'a> {
    root: &'a Path,
    dir: PathBuf,
    session: String,
    slot: String,
    episode: i64,
    server: ServerId,
    pane: String,
    identity: Vec<Vec<u8>>,
}

impl Ctx<'_> {
    fn seat(&self) -> Option<Seat> {
        read(&self.dir, &self.session, &self.slot)
    }

    fn capture(&self) -> String {
        crate::transport::capture_pane(&self.server, &self.pane).unwrap_or_default()
    }

    /// The ONE decision, the quota read fresh, against the frame now.
    fn decide(&self, seat: &Seat, capture: &str) -> Decision {
        // A role or cadence changed since the daemon started this leg holds (R2, D2.3).
        let cadence = crate::watchdog_daemon::quota_seconds(&seat.bytes, 300);
        if crate::meta::meta_agent_role(&seat.bytes) != crate::meta::MetaAgentRole::Absent
            || !matches!(cadence, Ok(1..))
        {
            return Decision::Idle;
        }
        let now = Timestamp::now();
        let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&seat.bytes));
        let quota = crate::autoreseat::leg::read_quota_at(self.root, &self.dir, &meta, now);
        let limited = crate::watchdog::limit_notice(capture, seat.bin()).is_some();
        let settings = crate::autoreseat::settings(global().as_deref());
        match switch(&settings) {
            (true, inverted) => seat.decide(
                quota.as_ref(),
                (capture, limited),
                inverted.is_none(),
                now.epoch(),
            ),
            (false, _) => Decision::Idle,
        }
    }

    /// The pane facts now, as the step reads them from a cycle.
    fn point(&self, seat: &Seat, capture: &str) -> Point {
        let tool = seat.run.tool;
        let clients = crate::transport::observe_clients(&self.server);
        let input = client_input(clients.as_deref(), &self.pane);
        let settings = crate::autoreseat::settings(global().as_deref());
        let events = crate::watchdog_daemon::read_events(&self.dir);
        let at = (seat.session.as_str(), seat.slot(), seat.entry.name.as_str());
        let now = Timestamp::now().epoch();
        Point {
            ready: input.is_ok()
                && crate::harness_state::classify(capture, tool)
                    == crate::harness_state::HarnessState::Idle
                && !crate::harness_state::has_human_draft(capture, tool)
                && crate::watchdog::human_prompt_class(capture, seat.bin()).is_none(),
            input: input.ok().flatten(),
            reseating: crate::autoreseat::in_flight(&settings, &events, at, now)
                || crate::autoreseat::headroom_in_flight(&settings, &events, at, now),
            pane_pid: self.pane_pid(),
        }
    }

    fn pane_pid(&self) -> Option<u32> {
        crate::transport::observe_pane_probe(&self.server, &self.pane).and_then(|probe| probe.pid)
    }

    /// The last word before the respawn: the same move, the pane safe, then the
    /// reservation and the record that authorizes the restart.
    fn reserve(&self, expected: &Decision) -> bool {
        let (Some(seat), capture) = (self.seat(), self.capture()) else {
            return false;
        };
        let decision = self.decide(&seat, &capture);
        let point = self.point(&seat, &capture);
        let (Some(episode), Some(pid)) = (
            seat.episode(),
            tool_of(&seat, point.pane_pid).map(|(_, pid)| pid),
        ) else {
            return false;
        };
        if episode.key != self.episode
            || decision != *expected
            || crate::meta::effort_identity(&seat.bytes, &self.slot) != self.identity
            || holds(&self.dir, &seat, &point, &capture, Timestamp::now().epoch()).is_some()
        {
            return false;
        }
        let record = match (&decision, &seat.stored) {
            (Decision::Down { twin, windows }, Stored::Absent) => Record {
                state: State::RestartDown,
                slot: self.slot.clone(),
                base: twin.baseline,
                low: twin.target,
                threshold: episode.threshold,
                episode: episode.key,
                session_id: seat.session_id.clone(),
                agent: seat.entry.name.clone(),
                tool: seat.run.tool,
                launch: seat.run.launch_id.clone(),
                pid,
                conversation: seat.run.harness_session.clone(),
                profile: seat.entry.profile.clone().unwrap_or_default(),
                // `read_seat` refuses an invalid client row, so none reaches here.
                client: seat.entry.client.record_value(),
                model: seat.pin.clone().unwrap_or_default(),
                windows: windows.clone(),
            },
            (Decision::Up, Stored::Present(record)) => Record {
                state: State::RestartUp,
                pid,
                ..record.clone()
            },
            _ => return false,
        };
        let Some(t) = record.pending() else {
            return false;
        };
        let row = record.encode();
        if Record::parse(&row).ok().as_ref() != Some(&record)
            || !self.journal(&seat, RESERVED, t, &pair(t))
        {
            return false;
        }
        let spent = self.seat().and_then(|now| match now.fold {
            Fold::Open(open) if open.key == self.episode => Some(leg_of(&open, t.direction)),
            _ => None,
        });
        spent == Some(Leg::Reserved(t))
            && crate::meta::publish_effort(&self.dir, &seat.proof(), &self.identity, Some(&row))
    }

    /// Settle the reservation of `direction`: the post-start proof, then the
    /// outcome, meta first and the journal after (plan D2.4, D3).
    fn settle(&self, direction: Direction, err: &mut impl Write) -> io::Result<()> {
        let Some(seat) = self.seat() else {
            return Ok(());
        };
        let Some(Leg::Reserved(t)) = seat
            .episode()
            .filter(|open| open.key == self.episode)
            .map(|open| leg_of(open, direction))
        else {
            return Ok(());
        };
        let record = match &seat.stored {
            Stored::Present(record)
                if record.episode == self.episode && seat.guard == Some(Ok(())) =>
            {
                record.clone()
            }
            // Gone after its meta clear, before its journal: the identity and pid
            // receipt went with it (lead ruling, plan D3).
            Stored::Absent if direction == Direction::Up => {
                return self.close(&seat, t, Outcome::Unproven, err);
            }
            _ => return self.close(&seat, t, Outcome::Failed("no record of this restart"), err),
        };
        let restarting = record.state == State::RestartDown && direction == Direction::Down
            || record.state == State::RestartUp && direction == Direction::Up;
        if !restarting && (record.state, direction) != (State::Applied, Direction::Down) {
            return self.close(
                &seat,
                t,
                Outcome::Failed("the seat was never restarted"),
                err,
            );
        }
        match self.prove(&record, t, restarting) {
            Proof::Proven(pid, last) => self.close(&last, t, Outcome::Moved(pid), err),
            Proof::Failed(why) => self.close(&seat, t, Outcome::Failed(why), err),
            Proof::Unproven => Ok(()),
        }
    }

    /// Two readings `FRAME_GAP` apart from ONE new process at the target effort
    /// and the pinned model, the conversation, launch, identity and account rows
    /// unchanged on each (plan D2.4). The last reading's seat closes the move.
    fn prove(&self, record: &Record, t: Transition, restarting: bool) -> Proof {
        let deadline = Instant::now() + PROOF_BOUND;
        let mut last = None;
        while Instant::now() < deadline {
            std::thread::sleep(crate::deliver::FRAME_GAP);
            let Some(seat) = self.seat() else {
                last = None;
                continue;
            };
            // A fallback records the conversation prior and the row anew.
            if seat.run.launch_id != record.launch
                || seat.run.harness_session != record.conversation
                || crate::meta::effort_identity(&seat.bytes, &self.slot) != self.identity
                || seat.guard != Some(Ok(()))
            {
                return Proof::Failed("the seat's conversation or identity changed");
            }
            let capture = self.capture();
            let pid = tool_of(&seat, self.pane_pid()).map(|(_, pid)| pid);
            if restarting && pid == Some(record.pid) {
                return Proof::Failed("the seat was never restarted");
            }
            let drawn = crate::harness_state::observed_identity(&capture, seat.run.tool);
            let model = crate::model_drift::decide(
                &drawn,
                Some(&record.model),
                seat.run.tool.adapter().pin_match,
            );
            // A frame that draws no model is no reading; one that draws another counts.
            let reading = pid
                .zip(
                    drawn
                        .effort
                        .as_deref()
                        .and_then(Level::parse)
                        .filter(|level| [t.from, t.to].contains(level)),
                )
                .filter(|_| model != crate::model_drift::Decision::Silent)
                .map(|(pid, level)| (pid, level, model == crate::model_drift::Decision::Retire));
            match (reading, last) {
                (Some(now), Some(before)) if now == before => {
                    return match now {
                        (pid, level, true) if level == t.to => Proof::Proven(pid, Box::new(seat)),
                        (_, _, false) => Proof::Failed("the restarted seat draws another model"),
                        _ => Proof::Failed("the restarted seat runs its old effort"),
                    };
                }
                _ => last = reading,
            }
        }
        Proof::Unproven
    }

    /// Write the outcome: meta first, then the journal, then the notices.
    fn close(
        &self,
        seat: &Seat,
        t: Transition,
        outcome: Outcome,
        err: &mut impl Write,
    ) -> io::Result<()> {
        if let Stored::Present(record) = &seat.stored {
            // A proven step down is applied at its new pid; a failed step back
            // up returns to the applied record it started from; else dropped.
            let next = match (outcome, t.direction) {
                (Outcome::Moved(pid), Direction::Down) => Some(Record {
                    state: State::Applied,
                    pid,
                    ..record.clone()
                }),
                (Outcome::Failed(_), Direction::Up) => Some(Record {
                    state: State::Applied,
                    ..record.clone()
                }),
                _ => None,
            };
            // Even a record that already stands is published again: the
            // identity rows are proved under the lock before any outcome.
            let row = next.as_ref().map(Record::encode);
            if !crate::meta::publish_effort(
                &self.dir,
                &seat.proof(),
                &self.identity,
                row.as_deref(),
            ) {
                return Ok(()); // the next leg settles it
            }
        }
        let (action, words) = match (outcome, t.direction) {
            (Outcome::Moved(_), Direction::Down) => (APPLIED, applied_words(seat, t)),
            (Outcome::Moved(_), Direction::Up) => (
                RESTORED,
                format!(
                    "ae restored your reasoning effort {} by restarting you in place: same conversation, the profile's own effort again.",
                    pair(t)
                ),
            ),
            (Outcome::Failed(why), _) => (
                FAILED,
                format!(
                    "ae could not prove an in-place restart at effort {} ({why}); this episode makes no further attempt.",
                    pair(t)
                ),
            ),
            (Outcome::Unproven, _) => (
                FAILED,
                format!(
                    "The outcome of ae's in-place restart of your reasoning effort {} is unproven: its record was cleared before the outcome was written, so the identity and pid receipt are lost. You should be back on your profile's own effort; this episode makes no further attempt.",
                    pair(t)
                ),
            ),
        };
        let summary = match outcome {
            Outcome::Unproven => format!(
                "{}; outcome unproven: record cleared, identity and pid receipt lost, profile effort expected",
                pair(t)
            ),
            _ => pair(t),
        };
        if !self.journal(seat, action, t, &summary) {
            return Ok(());
        }
        let told = format!("{}: effort {} — {action}", seat.entry.name, pair(t));
        let _ = crate::store::open(&self.dir).append_event(&crate::chat_line(
            Timestamp::now(),
            crate::watchdog::WATCHDOG_ACTOR,
            "",
            &told,
        ));
        self.notice(&seat.entry, &words, err)?;
        let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&seat.bytes));
        for other in meta
            .roster()
            .iter()
            .filter(|other| other.slot != self.slot && eligible(&other.slot))
        {
            self.notice(other, &told, err)?;
        }
        Ok(())
    }

    /// Journal `action` on the move `t` of this episode, as the watchdog.
    fn journal(&self, seat: &Seat, action: &str, t: Transition, summary: &str) -> bool {
        let reference = match t.direction {
            Direction::Down => format!("{}-down", self.episode),
            Direction::Up => format!("{}-up", self.episode),
        };
        seat.append(
            &self.dir,
            action,
            &reference,
            summary,
            Timestamp::now().epoch(),
        )
    }

    /// ONE `⟦ae:ctx⟧` line to the seat `to`, never retried.
    fn notice(
        &self,
        to: &crate::meta::RosterEntry,
        text: &str,
        err: &mut impl Write,
    ) -> io::Result<()> {
        let Ok((resolved, server)) = crate::tracked::resolve_on(&to.name, &self.session, &self.dir)
        else {
            return Ok(());
        };
        let reference = format!("{}-notice", self.episode);
        let request = crate::deliver::Request {
            dir: &self.dir,
            server: &server,
            pane: &resolved.pane,
            logged_target: &to.name,
            target_session: &self.session,
            pane_slot: &to.slot,
            own_session: &self.session,
            action: "effort-notice",
            reference: &reference,
            actor: crate::watchdog::WATCHDOG_ACTOR,
            body: "",
            shape: crate::deliver::Shape::Context,
            defer: Duration::from_secs(10),
            composed: crate::tool::Composed::NONE,
        };
        crate::deliver::deliver_composed(&request, || Some(text.to_owned()), err).map(|_| ())
    }
}

/// The moved seat's line after a step down (lead ruling: the target is the
/// profile's effort; another is the human's `ae reseat`).
fn applied_words(seat: &Seat, t: Transition) -> String {
    format!(
        "ae lowered your reasoning effort {} on quota pressure, restarting you in place: same \
         conversation and profile. Your target stays the profile's effort ({}), restored once \
         your account's long windows read under {}%. Another effort is `ae reseat {} {} --using \
         <a profile at that effort>`; a change in your own pane is yours, never restored over.",
        pair(t),
        t.from.word(),
        AT - 5,
        seat.session,
        seat.entry.name
    )
}

/// How a settled move ended.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    /// Proven at the target effort, as this new tool process.
    Moved(u32),
    /// Spent without a proven restart, and why.
    Failed(&'static str),
    /// Spent with its record gone: neither the restart nor its failure is proven.
    Unproven,
}

/// What the post-start proof found.
enum Proof {
    /// At the target as this new process, read last as this seat.
    Proven(u32, Box<Seat>),
    Failed(&'static str),
    /// Undecided within the bound: the next leg settles it.
    Unproven,
}

fn leg_of(episode: &super::Episode, direction: Direction) -> Leg {
    match direction {
        Direction::Down => episode.down,
        Direction::Up => episode.up,
    }
}

fn pair(t: Transition) -> String {
    format!("{}->{}", t.from.word(), t.to.word())
}

/// `_auto-effort <dir> <slot> <episode>`: the leg of the episode opened at
/// that epoch second.
/// Every ending but a usage error is exit 0: what happened is in the journal.
///
/// # Errors
///
/// Only a failure to write `err`.
pub(crate) fn run(root: &Path, tail: &[String], err: &mut impl Write) -> io::Result<u8> {
    // The episode key travels as its epoch digits; `parse_argv` proves the rest.
    let parsed = match tail {
        [dir, slot, episode] => episode
            .parse::<i64>()
            .ok()
            .filter(|key| *key > 0 && key.to_string() == *episode)
            .and_then(|key| {
                let argv = [
                    dir.clone(),
                    slot.clone(),
                    Timestamp::from_epoch(key).to_string(),
                ];
                let roots = crate::inventory::Roots::under(root);
                crate::autoreseat::leg::parse_argv(roots.sessions(), &argv)
                    .ok()
                    .map(|argv| (argv, key))
            }),
        _ => None,
    };
    let Some((argv, episode)) = parsed.filter(|(argv, _)| eligible(&argv.slot)) else {
        writeln!(err, "{USAGE}")?;
        return Ok(crate::state::EXIT_USAGE);
    };
    let Ok(_held) = crate::store::lock(
        &crate::autoreseat::lock_path(&argv.dir, &argv.slot),
        Duration::ZERO,
    ) else {
        return Ok(0);
    };
    let Some(seat) = read(&argv.dir, &argv.session, &argv.slot) else {
        return Ok(0);
    };
    let Ok((resolved, server)) =
        crate::tracked::resolve_on(&seat.entry.name, &argv.session, &argv.dir)
    else {
        return Ok(0);
    };
    let ctx = Ctx {
        root,
        identity: crate::meta::effort_identity(&seat.bytes, &argv.slot),
        dir: argv.dir,
        session: argv.session,
        slot: argv.slot,
        episode,
        server,
        pane: resolved.pane,
    };
    if seat.episode().is_none_or(|open| open.key != episode) {
        return Ok(0);
    }
    let decision = ctx.decide(&seat, &ctx.capture());
    let direction = match decision {
        Decision::Reconcile(direction) => direction,
        Decision::Down { .. } => Direction::Down,
        Decision::Up => Direction::Up,
        _ => return Ok(0),
    };
    if !matches!(decision, Decision::Reconcile(_)) {
        let mut hook = || ctx.reserve(&decision);
        crate::reseat::restart_in_place(
            root,
            &ctx.dir,
            &ctx.session,
            &seat.entry.name,
            &mut hook,
            err,
        )?;
    }
    ctx.settle(direction, err)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real directories; the boundary is about \
                  what PRODUCT code may reach"
    )]
    // Driver units (#253 b4, supplementary): what a client list proves, and
    // that a close revalidates the identity CAS even when its record stands.
    use super::{Ctx, Outcome, client_input, model_hold, read};
    use crate::effort::{Direction, Level, Transition};
    use crate::inventory::ServerId;
    use crate::tmux::ObservedClient;
    use crate::tool::ToolKind;

    /// The fixture vendors' measured idle frames (`tests/it/effort_live_spec.rs`).
    fn frame(tool: ToolKind, model: &str) -> String {
        let bar = "─".repeat(60);
        match tool {
            ToolKind::Claude => format!(
                "✻ Fake · done (0s)\n{bar}\n❯ \n{bar}\n  🧠 {model} (xhigh)  📁 work\n  ⏵⏵ bypass permissions on\n"
            ),
            _ => format!("• ok\n\n› Ask Codex to do anything\n\n  {model} xhigh · /tmp/work\n"),
        }
    }

    #[test]
    fn a_drawn_model_the_pin_does_not_name_holds_the_stop_and_is_named() {
        assert_eq!(
            model_hold(
                &frame(ToolKind::Claude, "Opus 5.5"),
                ToolKind::Claude,
                Some("opus")
            ),
            None
        );
        assert_eq!(
            model_hold(
                &frame(ToolKind::Codex, "gpt-6.1-sol"),
                ToolKind::Codex,
                Some("gpt-6.1-sol")
            ),
            None
        );
        assert_eq!(
            model_hold(
                &frame(ToolKind::Claude, "Sonnet 5.5"),
                ToolKind::Claude,
                Some("opus")
            ),
            Some("model-drift")
        );
        assert_eq!(
            model_hold(
                &frame(ToolKind::Codex, "gpt-6-astra"),
                ToolKind::Codex,
                Some("gpt-6.1-sol")
            ),
            Some("model-drift")
        );
        assert_eq!(
            model_hold("no composer here\n", ToolKind::Claude, Some("opus")),
            Some("")
        );
    }

    fn client(pane: &str, activity: Option<u64>) -> ObservedClient {
        ObservedClient {
            name: "/dev/ttys001".to_owned(),
            session: "fx".to_owned(),
            pane: pane.to_owned(),
            activity,
        }
    }

    #[test]
    fn an_unread_list_or_an_unstamped_viewer_proves_no_quiet_pane() {
        assert_eq!(client_input(None, "%1"), Err(()));
        assert_eq!(
            client_input(Some(&[client("%1", Some(5)), client("%1", None)]), "%1"),
            Err(())
        );
        let overflow = u64::try_from(i64::MAX).expect("fits") + 1;
        assert_eq!(
            client_input(Some(&[client("%1", Some(overflow))]), "%1"),
            Err(())
        );
    }

    #[test]
    fn no_viewer_is_quiet_and_the_newest_stamp_wins() {
        assert_eq!(client_input(Some(&[]), "%1"), Ok(None));
        assert_eq!(client_input(Some(&[client("%2", None)]), "%1"), Ok(None));
        let all = [
            client("%1", Some(5)),
            client("%1", Some(9)),
            client("%2", Some(20)),
        ];
        assert_eq!(client_input(Some(&all), "%1"), Ok(Some(9)));
    }

    const SID: &str = "11111111-1111-4111-8111-111111111111";
    const CONV: &str = "22222222-2222-4222-8222-222222222222";
    const EP: i64 = 1_700_000_000;

    /// A session whose lead seat's step down is already `applied` at pid 4242
    /// (a leg that crashed after its meta, before its journal), no server
    /// recorded, so no notice can reach a tmux server.
    fn applied_session(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ae-effort-leg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).expect("scratch session");
        std::fs::write(
            dir.join("config"),
            "[profiles]\nopus55x = \"claude --model opus --effort xhigh\"\n",
        )
        .expect("scratch config");
        let d = dir.display();
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id={SID}\nsession=fx\nwork_dir={d}\norigin={d}\nconfig={d}/config\n\
                 seat.main=lead\nprofile.main=opus55x\nagent_bin.main=claude\n\
                 launch_id.main=tok-1\nharness_session.main={CONV}\nconfig_home.main={d}/home\n\
                 effort.main=v1 applied slot:main base:xhigh low:high at:80 ep:{EP} sid:{SID} \
                 agent:lead tool:claude launch:tok-1 pid:4242 conv:{CONV} profile:opus55x \
                 client:- model:opus win:weekly/~/10080/{EP}/1700000100\n"
            ),
        )
        .expect("scratch meta");
        let line = |action: &str, reference: &str, summary: &str| {
            format!(
                "{{\"ts\":\"2023-11-14T22:13:20Z\",\"actor\":\"watchdog\",\"action\":\"{action}\",\
                 \"target\":\"lead\",\"target_slot\":\"main\",\"target_session\":\"fx\",\
                 \"target_session_id\":\"{SID}\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}\n"
            )
        };
        let journal = line("effort-episode", &EP.to_string(), "threshold 80")
            + &line("effort-reserved", &format!("{EP}-down"), "xhigh->high");
        std::fs::write(dir.join("events.jsonl"), journal).expect("scratch journal");
        dir
    }

    /// Close the applied step down again, the account row moved to `home`
    /// after the leg read its identity when `home` is set: whether the outcome
    /// was journaled, and the meta after.
    fn close_applied(tag: &str, home: Option<&str>) -> (bool, String) {
        let dir = applied_session(tag);
        let seat = read(&dir, "fx", "main").expect("the fixture seat reads");
        let ctx = Ctx {
            root: &dir,
            dir: dir.clone(),
            session: "fx".to_owned(),
            slot: "main".to_owned(),
            episode: EP,
            server: ServerId::Ambient,
            pane: "%0".to_owned(),
            identity: crate::meta::effort_identity(&seat.bytes, "main"),
        };
        if let Some(home) = home {
            let meta = std::fs::read_to_string(dir.join("meta")).expect("meta");
            let moved = meta.replace(&format!("config_home.main={}/home", dir.display()), home);
            std::fs::write(dir.join("meta"), moved).expect("account row moved");
        }
        let before = std::fs::read_to_string(dir.join("meta")).expect("meta");
        let t = Transition {
            direction: Direction::Down,
            from: Level::Xhigh,
            to: Level::High,
        };
        ctx.close(&seat, t, Outcome::Moved(4242), &mut Vec::new())
            .expect("close writes no error stream");
        let events = std::fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default();
        let after = std::fs::read_to_string(dir.join("meta")).expect("meta");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            home.is_none() || after == before,
            "a refused close writes no meta"
        );
        (events.contains("\"action\":\"effort-applied\""), after)
    }

    #[test]
    fn a_close_whose_record_already_stands_still_proves_the_identity_rows() {
        let (journaled, _) = close_applied("moved", Some("config_home.main=/elsewhere"));
        assert!(!journaled, "an account row moved since the leg read it");
        let (journaled, after) = close_applied("kept", None);
        assert!(journaled, "positive control: an unchanged identity closes");
        assert!(after.contains("effort.main=v1 applied "), "{after}");
    }
}
