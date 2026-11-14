//! The motion ticker: the strip state it carries, the cadence rules, the pace the loops run on and the ticker itself.

use super::{
    ADOPTION_TICK, ATTACHED_MOTION_TICK, ATTACHED_OBSERVATION, Adoption, Carry,
    DETACHED_MOTION_TICK, Duration, FRAME_PHASES, Instant, Journal, Look, MOTION_FAILURE_LIMIT,
    Mark, Meta, NON_AGENT_PANES, OptionScope, Path, PendingCycle, SeatVerdict, SystemTime,
    WAKE_REARM, Write, adoption_due, adoption_writes, fleet_rows, theme, tmux, transport,
    window_agents_line,
};

/// One pane's verdict this cycle.
pub(super) struct PaneMark {
    /// The `%<n>` pane id.
    pub(super) pane: String,
    /// What the accounting made of it, with the seat tool's job beside it.
    pub(super) verdict: SeatVerdict,
    /// The watchdog-owned public observation, including restart carry.
    pub(super) observed: String,
}

/// One cycle verdict in the window layout the ticker animates between cycles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MotionVerdict {
    pub(super) pane: String,
    pub(super) window: String,
    pub(super) verdict: SeatVerdict,
}

/// The orchestrator target publication state, including not-yet-published.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum PublishedOrchestrator {
    /// No fleet observation has been published since this daemon attached.
    #[default]
    Unknown,
    /// The observed fleet has no orchestrator target.
    Unset,
    /// The observed fleet's orchestrator session id.
    Target(String),
}

/// The orchestrator strip publication state, including not-yet-published.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum PublishedOrchestratorStrip {
    /// No fleet observation has been published since this daemon attached.
    #[default]
    Unknown,
    /// The observed fleet has no orchestrator segment.
    Unset,
    /// The rendered orchestrator segment.
    Value(String),
}

/// The ticker's carry: the most recent cycle verdicts and working frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct MotionState {
    pub(super) verdicts: Vec<MotionVerdict>,
    pub(super) panes: Vec<tmux::MotionPane>,
    pub(super) fleet: Vec<theme::FleetRow>,
    pub(super) fleet_target: Option<String>,
    /// The human's `[workspace] fleet_order`, STORED rather than read here. The
    /// ticker redraws at motion cadence and must neither open the config nor draw
    /// a different order from the one the cycle just published — that would flap.
    /// The cycle refreshes this once per cycle; BOTH writers read it from here.
    pub(super) fleet_order: theme::FleetOrder,
    pub(super) published_fleet: Option<String>,
    /// The last orchestrator target publication, including a successful unset.
    pub(super) published_orchestrator_id: PublishedOrchestrator,
    /// The last orchestrator segment publication, including a successful unset.
    pub(super) published_orchestrator_strip: PublishedOrchestratorStrip,
    pub(super) spin: u64,
}

impl MotionState {
    /// Replace only the cycle-owned verdicts. The cycle already published the
    /// corresponding static window labels.
    pub(super) fn replace_verdicts(&mut self, verdicts: Vec<MotionVerdict>) {
        self.verdicts = verdicts;
    }

    /// Replace the ticker's two observations together: the panes decide
    /// visibility and local animation, while the fleet drives line two.
    pub(super) fn replace_observation(
        &mut self,
        panes: Vec<tmux::MotionPane>,
        sessions: &[tmux::FleetListingRow],
        known: &[String],
        session: &str,
    ) {
        self.panes = panes;
        self.replace_fleet(sessions, known, session);
    }

    /// Take this cycle's fleet order. Called from the VERDICT path only: the
    /// ticker preserves whatever the last cycle left here, which is what keeps
    /// the motion frames and the verdict write drawing one order.
    pub(super) fn set_fleet_order(&mut self, order: &theme::FleetOrder) {
        self.fleet_order.clone_from(order);
    }

    /// Replace the fleet half of an observation and remember which exact
    /// session table owns the strip. The stored order is deliberately untouched
    /// — an observation says who is on the server, never how to arrange them.
    ///
    /// `known` is the adoption scan's answer to "which of these does ae's own
    /// record vouch for", which is what lets a RANKLESS running session be
    /// drawn Stale here instead of vanishing from this session's own strip.
    pub(super) fn replace_fleet(
        &mut self,
        sessions: &[tmux::FleetListingRow],
        known: &[String],
        session: &str,
    ) {
        self.fleet = fleet_rows(sessions, known, session);
        // The target comes off the DRAWN rows, never the raw listing: a session
        // that draws no row of its own — rankless and not vouched for — has no
        // strip to own, exactly as when the reader dropped rankless rows for it.
        self.fleet_target = self
            .fleet
            .iter()
            .find(|row| row.current)
            .map(|row| row.id.clone());
    }

    /// Add the fleet strip when its text has changed. A working frame makes
    /// that true on every animation step; a static fleet writes only after a
    /// rank, name or order change.
    pub(super) fn push_fleet_write(
        &mut self,
        writes: &mut Vec<tmux::OptionWrite>,
        look: &Look,
        working_frame: Option<&theme::WorkingFrame>,
    ) {
        let Some(target) = self.fleet_target.as_deref() else {
            return;
        };
        let strip = theme::fleet_strip(look, &self.fleet, working_frame, &self.fleet_order);
        if self.published_fleet.as_deref() == Some(&strip) {
            return;
        }
        writes.push(tmux::OptionWrite::new(
            OptionScope::Session,
            target,
            theme::FLEET_STRIP_OPTION,
            &strip,
        ));
        self.published_fleet = Some(strip);
    }

    /// Add the orchestrator target only when it changed. A missing target is
    /// still recorded so the caller can unset it once, rather than spawning a
    /// tmux process on every motion tick.
    pub(super) fn push_orchestrator_id_write(
        &mut self,
        writes: &mut Vec<tmux::OptionWrite>,
        target: &str,
        desired: Option<&str>,
    ) -> bool {
        let desired = desired.map_or(PublishedOrchestrator::Unset, |id| {
            PublishedOrchestrator::Target(id.to_owned())
        });
        if self.published_orchestrator_id == desired {
            return false;
        }
        if let PublishedOrchestrator::Target(id) = &desired {
            writes.push(tmux::OptionWrite::new(
                OptionScope::Session,
                target,
                theme::ORCHESTRATOR_ID_OPTION,
                id,
            ));
        }
        self.published_orchestrator_id = desired;
        true
    }

    /// Add the orchestrator segment only when its rendered text changed. A
    /// missing orchestrator is recorded so the caller can unset stale output
    /// once rather than spawning a tmux process on every motion tick.
    pub(super) fn push_orchestrator_strip_write(
        &mut self,
        writes: &mut Vec<tmux::OptionWrite>,
        look: &Look,
        target: &str,
        row: Option<&theme::FleetRow>,
        working_frame: Option<&theme::WorkingFrame>,
    ) -> bool {
        let desired = row.map_or(PublishedOrchestratorStrip::Unset, |row| {
            PublishedOrchestratorStrip::Value(theme::orchestrator_strip(look, row, working_frame))
        });
        if self.published_orchestrator_strip == desired {
            return false;
        }
        if let PublishedOrchestratorStrip::Value(value) = &desired {
            writes.push(tmux::OptionWrite::new(
                OptionScope::Session,
                target,
                theme::ORCHESTRATOR_STRIP_OPTION,
                value,
            ));
        }
        self.published_orchestrator_strip = desired;
        true
    }

    /// The cached verdicts whose pane is a live agent drawing the working mark.
    /// Field by field, so a caller may advance the spin while it holds them.
    pub(super) fn working<'a>(
        verdicts: &'a [MotionVerdict],
        panes: &[tmux::MotionPane],
    ) -> Vec<&'a MotionVerdict> {
        verdicts
            .iter()
            .filter(|entry| entry.verdict.mark() == Mark::Working)
            .filter(|entry| {
                panes.iter().any(|pane| {
                    pane.pane_id == entry.pane
                        && pane.agent.as_deref().is_some_and(|agent| {
                            !agent.is_empty() && !NON_AGENT_PANES.contains(&agent)
                        })
                })
            })
            .collect()
    }

    /// Whether the fleet strip carries a working row, which pulses too.
    fn fleet_working(&self) -> bool {
        self.fleet.iter().any(|row| row.mark == Mark::Working)
    }

    /// Whether [`MotionState::step`] draws a frame from this observation: a
    /// client can see the session, and something on it is working.
    fn animates(&self) -> bool {
        session_attached(&self.panes)
            && (!Self::working(&self.verdicts, &self.panes).is_empty() || self.fleet_working())
    }

    /// Advance every attached working surface from the cached observation.
    pub(super) fn step(&mut self, look: &Look) -> Vec<tmux::OptionWrite> {
        if !self.panes.iter().any(|pane| pane.session_attached > 0) {
            return Vec::new();
        }
        let working = Self::working(&self.verdicts, &self.panes);
        let fleet_working = self.fleet_working();
        if !working.is_empty() || fleet_working {
            self.spin = self.spin.wrapping_add(FRAME_PHASES);
        }
        let frame = theme::working_frame(self.spin, &look.palette, look.icons);
        let mut writes = Vec::new();
        let mut windows = Vec::new();
        for entry in &working {
            writes.push(tmux::OptionWrite::new(
                OptionScope::Pane,
                &entry.pane,
                theme::PANE_STATE_OPTION,
                // The VERDICT's word, never a literal: `Active` and
                // `Meta(MetaSweeping)` share the Working mark, and a hardcoded
                // "working" here repainted a just-published `● sweeping` as
                // `● working` within one tick. A fresh `waiting-agent` is not
                // repainted at all — its mark is static, so the published
                // `◔ waiting-agent` stands until the next verdict cycle.
                &theme::pane_state_frame(&frame, entry.verdict.reason()),
            ));
            if !windows.contains(&entry.window) {
                windows.push(entry.window.clone());
            }
        }
        for window in &windows {
            let agents: Vec<(String, Mark)> = self
                .verdicts
                .iter()
                .filter(|entry| entry.window == *window)
                .filter_map(|entry| {
                    let agent = self
                        .panes
                        .iter()
                        .find(|pane| pane.pane_id == entry.pane)?
                        .agent
                        .as_deref()
                        .filter(|agent| !agent.is_empty() && !NON_AGENT_PANES.contains(agent))?;
                    Some((theme::agent_label(agent), entry.verdict.mark()))
                })
                .collect();
            writes.push(tmux::OptionWrite::new(
                OptionScope::Window,
                window,
                theme::WINDOW_AGENTS_OPTION,
                &window_agents_line(&agents, look, Some(&frame)),
            ));
        }
        self.push_fleet_write(&mut writes, look, fleet_working.then_some(&frame));
        if let Some(target) = self.fleet_target.clone() {
            let orchestrator = self
                .fleet
                .iter()
                .find(|row| row.name == crate::orchestrator::ORCHESTRATOR_SESSION)
                .cloned();
            if let Some(row) = orchestrator.as_ref() {
                let _ = self.push_orchestrator_strip_write(
                    &mut writes,
                    look,
                    &target,
                    Some(row),
                    fleet_working.then_some(&frame),
                );
            }
        }
        writes
    }

    /// The static half of `step` for `motion = off`: fleet and orchestrator
    /// publications without animation frames. Same attached gate and
    /// write-on-change caching as `step`; F6's unset lands here too.
    pub(super) fn step_static(&mut self, look: &Look) -> Vec<tmux::OptionWrite> {
        if !self.panes.iter().any(|pane| pane.session_attached > 0) {
            return Vec::new();
        }
        let mut writes = Vec::new();
        self.push_fleet_write(&mut writes, look, None);
        if let Some(target) = self.fleet_target.clone() {
            let orchestrator = self
                .fleet
                .iter()
                .find(|row| row.name == crate::orchestrator::ORCHESTRATOR_SESSION)
                .cloned();
            if let Some(row) = orchestrator.as_ref() {
                let _ =
                    self.push_orchestrator_strip_write(&mut writes, look, &target, Some(row), None);
            }
        }
        writes
    }
}

/// Whether a tick that carries no write must still observe: none has been
/// committed yet, or the last is [`ATTACHED_OBSERVATION`] old. A tick that
/// writes always observes, because its read rides the same invocation.
pub(super) fn motion_observation_due(observed_at: Option<Instant>, now: Instant) -> bool {
    observed_at.is_none_or(|at| now.saturating_duration_since(at) >= ATTACHED_OBSERVATION)
}

/// Whether this look permits periodic redraws at all.
pub(super) const fn motion_ticker_enabled(look: &Look) -> bool {
    look.drawn && look.motion
}

/// Which wait a verdict interval gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TickerMode {
    /// `theme = off`, or no look ever read: the cycle owns `@ae_*`.
    Idle,
    /// `motion = off`: re-observation without animation frames.
    Static,
    /// Drawn and animated: the full motion ticker.
    Animated,
}

/// Motion gates ANIMATION FRAMES only: drawn-but-still keeps its fleet
/// strip fresh, and only `theme = off` sleeps the whole interval.
pub(super) const fn ticker_mode(look: Option<&Look>) -> TickerMode {
    match look {
        Some(look) if motion_ticker_enabled(look) => TickerMode::Animated,
        Some(look) if look.drawn => TickerMode::Static,
        _ => TickerMode::Idle,
    }
}

/// The observation cadence when motion is off: the SAME cadence the
/// animated ticker observes at when nothing animates.
pub(super) fn static_observe_cadence(panes: &[tmux::MotionPane]) -> Duration {
    if session_attached(panes) {
        ATTACHED_OBSERVATION
    } else {
        DETACHED_MOTION_TICK
    }
}

/// Whether an observation saw a client on this session.
pub(super) fn session_attached(panes: &[tmux::MotionPane]) -> bool {
    panes.iter().any(|pane| pane.session_attached > 0)
}

/// The animated ticker's next rest from its committed observation: a frame
/// every [`ATTACHED_MOTION_TICK`] while something visible is working, else
/// the static observer's cadence.
pub(super) fn motion_cadence(state: &MotionState) -> Duration {
    if state.animates() {
        ATTACHED_MOTION_TICK
    } else {
        static_observe_cadence(&state.panes)
    }
}

/// Advance the bounded failure streak and say whether ticking must stop for
/// this verdict interval.
pub(super) const fn motion_failure(prior: u8) -> (u8, bool) {
    let next = prior.saturating_add(1);
    (next, next >= MOTION_FAILURE_LIMIT)
}

/// A write against cached pane identities may fail because the fleet changed
/// since observation. Refresh it without spending the transport failure
/// budget; only a write against fresh identities proves a ticker failure.
///
/// Every write rides a read and is drawn from the observation before it, so
/// the identities are fresh once the observation that came with a refused
/// write has been committed: `observed` is whether the previous publication
/// was refused too.
pub(super) const fn motion_publish_failure(prior: u8, observed: bool) -> (u8, bool) {
    if observed {
        motion_failure(prior)
    } else {
        (prior, false)
    }
}

/// Whether a fleet-picker marker has reached half a watchdog interval.
/// Malformed transient state expires too, so it cannot pin the highlight.
pub(super) fn menu_open_expired(opened: &str, now_epoch: i64, interval_secs: u64) -> bool {
    let Ok(opened_epoch) = opened.parse::<i64>() else {
        return true;
    };
    let Ok(interval) = i64::try_from(interval_secs / 2) else {
        return false;
    };
    now_epoch.saturating_sub(opened_epoch) >= interval
}

/// Wait for the next verdict cycle while publishing motion at its own cadence.
/// A failed tick still sleeps its cadence, so transient tmux failures cannot
/// shorten the verdict interval. Three consecutive failures stop the ticker
/// and sleep the remainder before the next liveness proof.
pub(super) fn wait_between_cycles(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    interval_secs: u64,
) {
    wait_between_cycles_with(
        server,
        session,
        carry,
        Duration::from_secs(interval_secs),
        &mut RealPace,
    );
}

/// The clock and the external wait the interval loop runs on.
///
/// Production is [`RealPace`]; a test replaces time and the wait's answer
/// only, never a ticker decision.
pub(super) trait Pace {
    /// The current instant.
    fn now(&self) -> Instant;
    /// Rest for `duration`.
    fn sleep(&mut self, duration: Duration);
    /// Wait until `session` has a client, `channel` is signalled, or
    /// `deadline` passes.
    fn wait(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        channel: &str,
        deadline: Instant,
    ) -> transport::Waited;
}

/// The wall clock and the real tmux wait.
pub(super) struct RealPace;

impl Pace for RealPace {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn wait(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        channel: &str,
        deadline: Instant,
    ) -> transport::Waited {
        transport::run_tmux_wait(server, session, channel, deadline)
    }
}

/// The longest a stepped daemon waits for its next step before it runs a
/// cycle anyway: a safety bound far outside any test's wait, never a clock.
pub(super) const STEP_BACKSTOP: Duration = Duration::from_mins(10);

/// The test-only pace of `_watchdog-run --step-channel <channel>`: real time,
/// except that a signal on `channel` ends the current interval.
///
/// The interval loop is given [`STEP_BACKSTOP`] as its interval and runs
/// unchanged, motion ticker included. Every rest it takes waits on ONE plain
/// `wait-for <channel>` client, kept across rests until it exits, so a release
/// can never race a step already on its way. A step moves this clock past the
/// interval's deadline, and the loop leaves through its own deadline check.
/// `interval_secs` keeps every arithmetic role it has.
pub(super) struct StepPace {
    server: crate::inventory::ServerId,
    channel: String,
    /// `<channel>-done`, signalled after every completed iteration.
    done: String,
    /// How far this clock runs ahead of the wall clock: one backstop per step.
    pub(super) skew: Duration,
    waiter: Option<transport::ChannelWaiter>,
}

impl StepPace {
    pub(super) fn new(server: crate::inventory::ServerId, channel: &str) -> Self {
        Self {
            server,
            channel: channel.to_owned(),
            done: format!("{channel}-done"),
            skew: Duration::ZERO,
            waiter: None,
        }
    }

    /// Say that one loop iteration completed.
    pub(super) fn acknowledge(&self) {
        let _ = transport::signal_channel(&self.server, &self.done);
    }

    /// Wait for a step until the real instant `until`. A step, or a waiter
    /// that cannot wait (its server is gone), ends the interval: the next
    /// liveness probe decides from there.
    fn await_step(&mut self, until: Instant) -> transport::Waited {
        if self.waiter.is_none() {
            self.waiter = transport::ChannelWaiter::start(&self.server, &self.channel);
        }
        let waited = self
            .waiter
            .as_mut()
            .map_or(transport::Waited::Failed, |waiter| waiter.wait_until(until));
        if waited != transport::Waited::TimedOut {
            self.waiter = None;
            self.skew = self.skew.saturating_add(STEP_BACKSTOP);
        }
        waited
    }
}

impl Pace for StepPace {
    fn now(&self) -> Instant {
        let now = Instant::now();
        now.checked_add(self.skew).unwrap_or(now)
    }

    fn sleep(&mut self, duration: Duration) {
        let until = Instant::now()
            .checked_add(duration)
            .unwrap_or_else(Instant::now);
        let _ = self.await_step(until);
    }

    fn wait(
        &mut self,
        _server: &crate::inventory::ServerId,
        _session: &str,
        _channel: &str,
        deadline: Instant,
    ) -> transport::Waited {
        let until = deadline.checked_sub(self.skew).unwrap_or_else(Instant::now);
        self.await_step(until)
    }
}

impl Drop for StepPace {
    /// A waiter still waiting when the daemon stops is released, never killed.
    fn drop(&mut self) {
        if let Some(waiter) = self.waiter.take() {
            waiter.release(&self.server);
        }
    }
}

/// [`wait_between_cycles`] over an explicit [`Pace`]: the ONE interval loop
/// production runs. The deadline is the interval's end, whatever happens in
/// between.
pub(super) fn wait_between_cycles_with(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    interval: Duration,
    pace: &mut impl Pace,
) {
    let Some(look) = carry.look else {
        // No look has EVER answered, so this session draws nothing of its own —
        // but a peer it is filling still has a line to keep.
        wait_idle_between_cycles(server, carry, interval, pace);
        return;
    };
    match ticker_mode(carry.look.as_ref()) {
        TickerMode::Idle => wait_idle_between_cycles(server, carry, interval, pace),
        TickerMode::Static => {
            wait_static_between_cycles(server, session, carry, &look, interval, pace);
        }
        TickerMode::Animated => run_ticker(server, session, carry, &look, interval, pace),
    }
}

/// Drive one [`MotionTicker`] to the end of the interval, then hand its
/// state back to the carry.
///
/// While the session is detached and this cycle proved its wake hook, the
/// loop holds ONE waiter instead of observing every two seconds. A wake, or a
/// client the waiter's own read counted, proves nothing by itself: the next
/// tick observes fresh, and that observation decides between motion and the
/// next wait, which starts no sooner than [`WAKE_REARM`] after the last. The
/// interval ends at its deadline whatever happens in between, so a wake never
/// starts a verdict cycle early. A failed wait polls for the rest of the
/// interval; an adopter always polls, because its peer's strip is a duty no
/// hook of this session announces.
fn run_ticker(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    look: &Look,
    interval: Duration,
    pace: &mut impl Pace,
) {
    let started = pace.now();
    let deadline = started + interval;
    let mut ticker = MotionTicker::new(carry.motion.clone(), carry.adoption.clone());
    let mut wake = carry
        .wake
        .clone()
        .filter(|_| carry.adoption.targets.is_empty());
    let mut waited_at: Option<Instant> = None;
    loop {
        let now = pace.now();
        if now >= deadline {
            break;
        }
        if let Some(channel) = wake.as_deref()
            && !session_attached(&ticker.motion.panes)
        {
            if let Some(earliest) = waited_at.map(|at| at + WAKE_REARM)
                && now < earliest
            {
                pace.sleep(earliest.min(deadline).saturating_duration_since(now));
                continue;
            }
            waited_at = Some(now);
            match pace.wait(server, session, channel, deadline) {
                transport::Waited::TimedOut => continue,
                transport::Waited::Failed => {
                    wake = None;
                    continue;
                }
                transport::Waited::Woke | transport::Waited::Attached => {}
            }
        }
        let remaining = deadline.saturating_duration_since(pace.now());
        if remaining.is_zero() {
            break;
        }
        let next = ticker.tick(server, session, look, pace.now());
        let remaining = interval.saturating_sub(pace.now().saturating_duration_since(started));
        if next.stopped {
            pace.sleep(remaining);
            break;
        }
        pace.sleep(next.after.min(remaining));
    }
    carry.motion = ticker.motion;
    carry.adoption = ticker.adoption;
}

/// Wait for the next verdict cycle with nothing of this session's OWN to draw.
///
/// `theme = off`, or no look has ever answered: the verdict cycle owns every
/// `@ae_*` this session publishes, so the whole interval is one sleep — which is
/// what it costs when this daemon has adopted nobody, the common case.
///
/// A daemon that IS filling a watchdog-less peer's fleet strip owes that duty to
/// the TARGET's look, not to its own, so it cannot sleep through the interval:
/// it wakes at the adoption cadence for one listing and at most one write per
/// tick, and goes back to the whole-interval sleep as soon as the peer's own
/// watchdog comes back.
pub(super) fn wait_idle_between_cycles(
    server: &crate::inventory::ServerId,
    carry: &mut Carry,
    interval: Duration,
    pace: &mut impl Pace,
) {
    // Decided ONCE: the target set is the verdict cycle's to change, so a
    // daemon with none sleeps exactly as it did before adoption existed.
    if carry.adoption.targets.is_empty() {
        pace.sleep(interval);
        return;
    }
    let started = pace.now();
    let mut failures = 0_u8;
    loop {
        let remaining = interval.saturating_sub(pace.now().saturating_duration_since(started));
        if remaining.is_zero() {
            break;
        }
        let mut adoption = carry.adoption.clone();
        let landed = match transport::observe_fleet_listing(server) {
            Some(listing) => {
                let writes = adoption_writes(&mut adoption, &listing, &carry.motion.fleet_order);
                writes.is_empty() || transport::publish_options(server, &writes)
            }
            None => false,
        };
        if !landed {
            let failed = motion_failure(failures);
            failures = failed.0;
            let remaining = interval.saturating_sub(pace.now().saturating_duration_since(started));
            if failed.1 {
                pace.sleep(remaining);
                break;
            }
            pace.sleep(ADOPTION_TICK.min(remaining));
            continue;
        }
        failures = 0;
        carry.adoption = adoption;
        let remaining = interval.saturating_sub(pace.now().saturating_duration_since(started));
        pace.sleep(ADOPTION_TICK.min(remaining));
    }
}

/// Wait for the next verdict cycle while keeping the fleet strip fresh at
/// the observation cadence, without animation frames. Same three-failure
/// rule as the animated ticker: the same [`MotionTicker`], which takes its
/// static arm from the look.
pub(super) fn wait_static_between_cycles(
    server: &crate::inventory::ServerId,
    session: &str,
    carry: &mut Carry,
    look: &Look,
    interval: Duration,
    pace: &mut impl Pace,
) {
    run_ticker(server, session, carry, look, interval, pace);
}

/// What one ticker iteration asks of the interval loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TickNext {
    /// Rest this long before the next tick; the loop caps it at the interval's
    /// remainder.
    pub(super) after: Duration,
    /// The failure limit was reached: rest the remainder of the interval
    /// without further motion work.
    pub(super) stopped: bool,
}

/// The motion ticker between two verdict cycles: the state it commits and
/// the memory it schedules by. One is made per interval from the carry.
#[derive(Debug, Clone)]
pub(super) struct MotionTicker {
    /// The committed motion observation and write-on-change memory.
    pub(super) motion: MotionState,
    /// The committed adoption targets and their write-on-change memory.
    adoption: Adoption,
    /// Consecutive failed ticks.
    pub(super) failures: u8,
    /// The cadence the last observation chose; `None` before the first tick.
    cadence: Option<Duration>,
    /// When the last observation committed; `None` before the first.
    observed_at: Option<Instant>,
    /// The last publication was refused, so the observation committed with
    /// it is the fresh one the next write is drawn from.
    refused: bool,
    /// When the adopted strips were last drawn.
    adopted_at: Option<Instant>,
}

impl MotionTicker {
    /// A ticker starting from the carry's committed state.
    pub(super) const fn new(motion: MotionState, adoption: Adoption) -> Self {
        Self {
            motion,
            adoption,
            failures: 0,
            cadence: None,
            observed_at: None,
            refused: false,
            adopted_at: None,
        }
    }

    /// One tick: publish what the committed observation draws, observe in
    /// the same invocation, commit what landed. `look` picks the arm —
    /// animated frames, or the static observer for `motion = off`.
    pub(super) fn tick(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        look: &Look,
        now: Instant,
    ) -> TickNext {
        if ticker_mode(Some(look)) == TickerMode::Static {
            self.tick_static(server, session, look, now)
        } else {
            self.tick_animated(server, session, look, now)
        }
    }

    fn tick_animated(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        look: &Look,
        now: Instant,
    ) -> TickNext {
        let cadence = *self.cadence.get_or_insert(ATTACHED_MOTION_TICK);
        // STAGED: the frame is drawn from the committed observation, and its
        // write-on-change memory commits only once tmux took the writes.
        let mut drawn = self.motion.clone();
        let writes = drawn.step(look);
        if writes.is_empty() && !motion_observation_due(self.observed_at, now) {
            return TickNext {
                after: cadence,
                stopped: false,
            };
        }
        let exchange = transport::exchange_motion(server, session, true, &writes);
        self.commit(server, session, exchange, drawn, now, motion_cadence)
    }

    fn tick_static(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        look: &Look,
        now: Instant,
    ) -> TickNext {
        self.cadence
            .get_or_insert_with(|| static_observe_cadence(&self.motion.panes));
        let mut drawn = self.motion.clone();
        let writes = drawn.step_static(look);
        let exchange = transport::exchange_motion(server, session, true, &writes);
        self.commit(server, session, exchange, drawn, now, |state| {
            static_observe_cadence(&state.panes)
        })
    }

    /// Commit what one exchange proved, and nothing it did not.
    ///
    /// - The reading is unusable: a read did not complete (tmux then ran none
    ///   of the writes), or today's interpreter refused a section (the writes
    ///   may have landed). No state commits, and the failure counts; a write
    ///   that landed anyway is asserted again by the next tick.
    /// - It is usable: the observation commits, whatever became of the writes.
    /// - The writes landed (or there were none): `drawn`'s spin and
    ///   write-on-change memory commit too, and the adopted strips are drawn
    ///   from this fresh listing when their cadence is due.
    /// - The writes were refused: the spin and the memory stay, so the next
    ///   tick draws the same phase again from the fresh observation.
    fn commit(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        exchange: transport::MotionExchange,
        drawn: MotionState,
        now: Instant,
        cadence_of: impl Fn(&MotionState) -> Duration,
    ) -> TickNext {
        let cadence = self.cadence.unwrap_or(ATTACHED_MOTION_TICK);
        let Some((panes, fleet)) = exchange.reading else {
            let failed = motion_failure(self.failures);
            self.failures = failed.0;
            return TickNext {
                after: cadence,
                stopped: failed.1,
            };
        };
        let landed = exchange.published != Some(false);
        if landed {
            self.motion = drawn;
        }
        self.motion
            .replace_observation(panes, &fleet, &self.adoption.known, session);
        self.observed_at = Some(now);
        let cadence = cadence_of(&self.motion);
        self.cadence = Some(cadence);
        if !landed {
            let failed = motion_publish_failure(self.failures, self.refused);
            self.failures = failed.0;
            self.refused = true;
            return TickNext {
                after: cadence,
                stopped: failed.1,
            };
        }
        self.refused = false;
        // Adoption's own cadence, off the listing this tick just read: never
        // a frame this daemon's own attachment gates. Its writes change only
        // when a strip does, so they cost an invocation of their own only then.
        if adoption_due(&mut self.adopted_at, now) {
            let mut adoption = self.adoption.clone();
            let writes = adoption_writes(&mut adoption, &fleet, &self.motion.fleet_order);
            if !writes.is_empty() && !transport::publish_options(server, &writes) {
                let failed = motion_failure(self.failures);
                self.failures = failed.0;
                return TickNext {
                    after: cadence,
                    stopped: failed.1,
                };
            }
            self.adoption = adoption;
        }
        self.failures = 0;
        TickNext {
            after: cadence,
            stopped: false,
        }
    }
}

/// The branch publication, the pending-id recovery and the Telegram revive,
/// once per cycle.
#[allow(
    clippy::too_many_arguments,
    reason = "the cycle's context, passed rather than gathered: each is a fact the loop \
              already owns, and a struct would only rename them"
)]
pub(super) fn tick_pane_duties(
    server: &crate::inventory::ServerId,
    meta_dir: &Path,
    session: &str,
    meta: &Meta,
    deferred: &mut crate::watchdog_glue::Deferred,
    journal: &Journal<'_>,
    pending: &mut PendingCycle,
    err: &mut impl Write,
) -> crate::error::Result<()> {
    if let Some(session_id) = pending.session_id() {
        let ops = crate::watchdog_glue::branch_ops(
            session_id,
            crate::watchdog_glue::branch_reading(meta.work_dir()).as_ref(),
        );
        pending.ops.extend(ops);
    }
    for row in crate::watchdog_glue::recover(meta_dir, meta.roster()) {
        // The DURABLE record of a post-launch capture.
        journal.record_referring(
            "recover",
            &row.agent,
            &row.captured,
            &crate::watchdog_glue::recovered_summary(&row),
            err,
        )?;
    }
    deferred.supervise(server, session, SystemTime::now());
    Ok(())
}
