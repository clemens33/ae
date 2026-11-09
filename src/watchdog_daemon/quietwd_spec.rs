//! Independent quietwd scheduling and committed-state acceptance.
//! Install as `src/watchdog_daemon/quietwd_spec.rs` under cfg(test).

use super::{
    Adoption, Carry, Look, MotionState, MotionTicker, MotionVerdict, Pace, Verdict,
    overlay_current_rank, wait_between_cycles_with,
};
use crate::inventory::ServerId;
use crate::theme::Mark;
use crate::transport::quietwd_spec::Rig;
use crate::transport::{self, Waited, spawns_during};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

fn observed_motion(rig: &Rig, panes: &[String]) -> MotionState {
    let reading =
        transport::observe_motion_panes(&rig.server, "current").expect("fixture pane read");
    let fleet = transport::observe_fleet_listing(&rig.server).expect("fixture fleet read");
    let mut motion = MotionState::default();
    motion.replace_observation(reading, &fleet, &[], "current");
    motion.replace_verdicts(
        panes
            .iter()
            .map(|pane| MotionVerdict {
                pane: pane.clone(),
                window: rig.window.clone(),
                verdict: Verdict::Active,
            })
            .collect(),
    );
    motion
}

fn split_agent(rig: &Rig) -> String {
    let pane = rig
        .must(&[
            "split-window",
            "-d",
            "-t",
            &rig.pane,
            "-P",
            "-F",
            "#{pane_id}",
            "sleep 600",
        ])
        .trim()
        .to_owned();
    rig.must(&["set-option", "-p", "-t", &pane, "@ae_agent", "worker"]);
    pane
}

#[test]
fn quietwd_real_working_tick_is_one_spawn_for_multiple_panes() {
    let rig = Rig::new("tick");
    let second = split_agent(&rig);
    rig.attach("current", 0);
    let motion = observed_motion(&rig, &[rig.pane.clone(), second.clone()]);
    let mut ticker = MotionTicker::new(motion, Adoption::default());
    let (next, calls) =
        spawns_during(|| ticker.tick(&rig.server, "current", &Look::DEFAULT, Instant::now()));
    assert!(!next.stopped);
    assert_eq!(
        calls.len(),
        1,
        "the production iteration must join reads and all frame writes: {calls:?}"
    );
    for pane in [&rig.pane, &second] {
        let state = rig.option("-p", pane, "@ae_pane_state");
        assert!(
            state.contains("working"),
            "working pane did not receive its frame: {state}"
        );
    }
    assert_eq!(
        next.after,
        Duration::from_millis(200),
        "lead-approved 5 fps"
    );
}

#[test]
fn quietwd_five_fps_keeps_a_two_second_period_instead_of_slows_the_pulse() {
    let rig = Rig::new("period");
    rig.attach("current", 0);
    let motion = observed_motion(&rig, std::slice::from_ref(&rig.pane));
    let mut ticker = MotionTicker::new(motion, Adoption::default());
    let start = Instant::now();
    let mut frames = Vec::new();
    for frame in 0..=10 {
        let next = ticker.tick(
            &rig.server,
            "current",
            &Look::DEFAULT,
            start + Duration::from_millis(frame * 200),
        );
        assert!(!next.stopped);
        frames.push(rig.option("-p", &rig.pane, "@ae_pane_state"));
    }
    assert_eq!(
        frames[0], frames[10],
        "ten 200 ms steps complete one two-second period"
    );
    assert!(
        frames[1..10].iter().any(|frame| frame != &frames[0]),
        "a frozen glyph is not a two-second pulse"
    );
}

#[test]
fn quietwd_static_observation_joins_its_reads_and_never_advances_a_frame() {
    let rig = Rig::new("static");
    rig.attach("current", 0);
    let motion = observed_motion(&rig, std::slice::from_ref(&rig.pane));
    let mut ticker = MotionTicker::new(motion, Adoption::default());
    let look = Look {
        motion: false,
        ..Look::DEFAULT
    };
    let start = Instant::now();
    for n in 0..2 {
        let (next, calls) = spawns_during(|| {
            ticker.tick(
                &rig.server,
                "current",
                &look,
                start + Duration::from_millis(n * 500),
            )
        });
        assert!(!next.stopped);
        assert_eq!(next.after, Duration::from_millis(500));
        assert_eq!(ticker.motion.spin, 0, "motion off is a zero-frame escape");
        assert_eq!(
            calls.len(),
            1,
            "one due static observation invocation: {calls:?}"
        );
    }
}

#[test]
fn quietwd_a_refused_previous_snapshot_write_keeps_spin_but_commits_completed_reads() {
    let rig = Rig::new("commit");
    let gone = split_agent(&rig);
    rig.attach("current", 0);
    let mut motion = observed_motion(&rig, &[rig.pane.clone(), gone.clone()]);
    motion.published_fleet = Some("previous-publication".into());
    let prior_spin = motion.spin;
    let prior_published = motion.published_fleet.clone();
    rig.must(&["kill-pane", "-t", &gone]);
    let mut ticker = MotionTicker::new(motion, Adoption::default());
    let next = ticker.tick(&rig.server, "current", &Look::DEFAULT, Instant::now());
    assert!(
        !next.stopped,
        "one refused publication does not exhaust the failure budget"
    );
    assert!(
        !ticker.motion.panes.iter().any(|pane| pane.pane_id == gone),
        "the completed new read must commit"
    );
    assert_eq!(
        ticker.motion.spin, prior_spin,
        "refused publication must retry the same phase"
    );
    assert_eq!(
        ticker.motion.published_fleet, prior_published,
        "a failed batch cannot advance publication memory"
    );
}

#[test]
fn quietwd_missing_reads_preserve_committed_state_and_stop_after_three_failures() {
    let rig = Rig::new("read-gap");
    let motion = observed_motion(&rig, std::slice::from_ref(&rig.pane));
    let prior = motion.clone();
    let mut ticker = MotionTicker::new(motion, Adoption::default());
    for n in 1..=3 {
        let next = ticker.tick(&rig.server, "absent", &Look::DEFAULT, Instant::now());
        assert_eq!(
            ticker.motion, prior,
            "a missing observation is not new state"
        );
        assert_eq!(next.stopped, n == 3, "unchanged three-failure stop rule");
    }
}

#[test]
fn quietwd_detach_observation_stops_the_next_frames_without_changing_verdicts() {
    let rig = Rig::new("detach");
    rig.attach("current", 0);
    let motion = observed_motion(&rig, std::slice::from_ref(&rig.pane));
    let mut ticker = MotionTicker::new(motion, Adoption::default());
    let start = Instant::now();
    let _ = ticker.tick(&rig.server, "current", &Look::DEFAULT, start);
    rig.must(&["detach-client", "-s", "=current"]);
    rig.await_clients("current", 0);
    let _ = ticker.tick(
        &rig.server,
        "current",
        &Look::DEFAULT,
        start + Duration::from_millis(200),
    );
    let visible_end = rig.option("-p", &rig.pane, "@ae_pane_state");
    let _ = ticker.tick(
        &rig.server,
        "current",
        &Look::DEFAULT,
        start + Duration::from_millis(400),
    );
    assert_eq!(
        rig.option("-p", &rig.pane, "@ae_pane_state"),
        visible_end,
        "no frame after the detached reading commits"
    );
    assert_eq!(
        ticker.motion.verdicts[0].verdict,
        Verdict::Active,
        "ticks never recalculate verdicts"
    );
}

#[test]
fn quietwd_same_cycle_rank_overlay_updates_only_the_current_identity() {
    let row = |name: &str, id: &str, rank: &str| crate::tmux::FleetListingRow {
        name: name.into(),
        id: id.into(),
        rank: Some(rank.into()),
        look: crate::tmux::LookOptions::default(),
    };
    let mut rows = vec![row("current", "$1", "2"), row("peer", "$2", "1")];
    let peer = rows[1].clone();
    overlay_current_rank(&mut rows, "$1", Mark::NeedsYou);
    assert_eq!(
        rows[0].rank.as_deref(),
        Some("4"),
        "current-cycle needs-you, not the previously listed working rank"
    );
    overlay_current_rank(&mut rows, "$1", Mark::Working);
    assert_eq!(rows[0].rank.as_deref(), Some("2"));
    assert_eq!(
        rows[1], peer,
        "this daemon does not vouch for a peer verdict"
    );
    let prior = rows.clone();
    overlay_current_rank(&mut rows, "$999", Mark::Dead);
    assert_eq!(rows, prior, "do not invent an unobserved current row");
}

struct Clock<'a> {
    now: Instant,
    results: VecDeque<(Duration, Waited)>,
    waits: Vec<(Instant, Instant)>,
    attach_on_wait: Option<&'a Rig>,
    scheduled_attach: Option<(Instant, &'a Rig)>,
    attached_at: Option<Instant>,
    frame_probe: Option<&'a Rig>,
    frame_receipts: Vec<Instant>,
}

impl Clock<'_> {
    fn new(results: impl IntoIterator<Item = (Duration, Waited)>) -> Self {
        Self {
            now: Instant::now(),
            results: results.into_iter().collect(),
            waits: Vec::new(),
            attach_on_wait: None,
            scheduled_attach: None,
            attached_at: None,
            frame_probe: None,
            frame_receipts: Vec::new(),
        }
    }

    fn advance_to(&mut self, until: Instant) {
        if self
            .scheduled_attach
            .as_ref()
            .is_some_and(|(at, _)| *at <= until)
        {
            let (at, rig) = self
                .scheduled_attach
                .take()
                .expect("scheduled external attach");
            assert!(
                at >= self.now,
                "the fixture cannot attach in the virtual past"
            );
            self.now = at;
            rig.attach("current", 0);
            self.attached_at = Some(at);
        }
        self.now = until;
    }
}

impl Pace for Clock<'_> {
    fn now(&self) -> Instant {
        self.now
    }

    fn sleep(&mut self, duration: Duration) {
        // Observe actual option effects before the real loop advances its clock.
        // This probe emits only show-options, never the frame write being judged.
        if let Some(rig) = self.frame_probe {
            let state = rig.option("-p", &rig.pane, "@ae_pane_state");
            if state.contains("working") {
                self.frame_receipts.push(self.now);
            }
        }
        self.advance_to(self.now + duration);
    }

    fn wait(
        &mut self,
        _server: &ServerId,
        _session: &str,
        _channel: &str,
        deadline: Instant,
    ) -> Waited {
        self.waits.push((self.now, deadline));
        assert!(
            self.waits.len() <= 100,
            "stale signals caused an immediate re-arm loop"
        );
        // A real external attachment happens inside this controlled wait, not
        // before a hypothetical atomic attachment read could have seen it.
        if let Some(rig) = self.attach_on_wait.take() {
            rig.attach("current", 0);
            self.attached_at = Some(self.now);
        }
        let (elapsed, result) = self
            .results
            .pop_front()
            .unwrap_or((Duration::ZERO, Waited::TimedOut));
        let until = if result == Waited::TimedOut {
            deadline
        } else {
            // If a faster implementation re-arms before the scheduled arrival,
            // its controlled Attached wait includes that external arrival too.
            let start = if result == Waited::Attached {
                self.scheduled_attach
                    .as_ref()
                    .map_or(self.now, |(at, _)| self.now.max(*at))
            } else {
                self.now
            };
            start + elapsed.min(deadline.saturating_duration_since(start))
        };
        self.advance_to(until);
        result
    }
}

fn detached_carry(rig: &Rig, proved: bool) -> Carry {
    let mut carry = Carry::new();
    carry.look = Some(Look::DEFAULT);
    carry.motion = observed_motion(rig, std::slice::from_ref(&rig.pane));
    carry.wake = proved.then(|| "qw-clock".into());
    carry
}

#[test]
fn quietwd_detached_interval_uses_one_wait_and_no_observation_children() {
    let rig = Rig::new("idle-count");
    let mut carry = detached_carry(&rig, true);
    let mut clock = Clock::new([]);
    let began = clock.now;
    let ((), calls) = spawns_during(|| {
        wait_between_cycles_with(
            &rig.server,
            "current",
            &mut carry,
            Duration::from_mins(1),
            &mut clock,
        );
    });
    assert_eq!(clock.now.duration_since(began), Duration::from_mins(1));
    assert_eq!(
        clock.waits.len(),
        1,
        "one external held wait, not two-second polling"
    );
    assert!(
        calls.is_empty(),
        "the fake external wait records separately; no observation children are owed: {calls:?}"
    );
}

#[test]
fn quietwd_wakes_never_shorten_the_verdict_interval_and_stale_rearms_are_spaced() {
    for wake in [Waited::Woke, Waited::Attached] {
        let rig = Rig::new("cadence");
        let mut carry = detached_carry(&rig, true);
        // Attached followed by a detached read is product-valid: the client
        // can leave after the external wait read and before the next observation.
        let mut clock = Clock::new([(Duration::ZERO, wake), (Duration::ZERO, wake)]);
        let began = clock.now;
        wait_between_cycles_with(
            &rig.server,
            "current",
            &mut carry,
            Duration::from_secs(8),
            &mut clock,
        );
        assert_eq!(clock.now.duration_since(began), Duration::from_secs(8));
        assert!(
            clock.waits.len() >= 2,
            "the real interval loop must use its proved wake mechanism"
        );
        for pair in clock.waits.windows(2) {
            assert!(
                pair[1].0.duration_since(pair[0].0) >= Duration::from_secs(2),
                "stale wake re-armed too soon"
            );
        }
        assert!(
            clock
                .waits
                .iter()
                .all(|(_, deadline)| *deadline == began + Duration::from_secs(8)),
            "wake must not reset the verdict deadline"
        );
    }
}

#[test]
fn quietwd_each_wake_enters_real_attached_motion_before_the_unchanged_deadline() {
    for wake in [Waited::Attached, Waited::Woke] {
        let rig = Rig::new("wake-motion");
        // The verdict cycle saw a detached session. A real client arrives inside
        // the external-wait boundary before its controlled result returns.
        let mut carry = detached_carry(&rig, true);
        assert!(
            carry
                .motion
                .panes
                .iter()
                .all(|pane| pane.session_attached == 0)
        );
        assert!(rig.option("-p", &rig.pane, "@ae_pane_state").is_empty());
        let mut clock = Clock::new([(Duration::from_millis(50), wake)]);
        clock.attach_on_wait = Some(&rig);
        clock.frame_probe = Some(&rig);
        let began = clock.now;
        let interval = Duration::from_secs(4);
        let deadline = began + interval;
        let ((), calls) = spawns_during(|| {
            wait_between_cycles_with(&rig.server, "current", &mut carry, interval, &mut clock);
        });
        assert!(
            !clock.waits.is_empty(),
            "the controlled wake path must execute"
        );
        assert!(
            calls.iter().any(|call| {
                ["list-panes", "set-option", "@ae_pane_state"]
                    .iter()
                    .all(|required| call.iter().any(|word| word.as_str() == *required))
            }),
            "a wake must enter the real joined-read/frame-write consumer: {calls:?}"
        );
        assert!(
            clock.frame_receipts.iter().any(|at| {
                *at < deadline && at.duration_since(began) <= Duration::from_millis(500)
            }),
            "a real frame must land within the stated attach bound, before the next cycle"
        );
        assert_eq!(
            clock.now, deadline,
            "motion cannot end or reset the verdict interval"
        );
        assert!(clock.waits.iter().all(|(_, end)| *end == deadline));
    }
}

#[test]
fn quietwd_attach_during_stale_wake_rearm_draws_within_the_restated_virtual_bound() {
    let rig = Rig::new("stale-rearm");
    let mut carry = detached_carry(&rig, true);
    assert!(
        carry
            .motion
            .panes
            .iter()
            .all(|pane| pane.session_attached == 0)
    );
    assert!(rig.option("-p", &rig.pane, "@ae_pane_state").is_empty());
    // The first stale wake is followed by a real detached reading. The external
    // client arrives at 100 ms inside the re-arm sleep, not after that sleep.
    // The next wait sees it. Only the Clock's virtual time drives the bound.
    let mut clock = Clock::new([
        (Duration::ZERO, Waited::Woke),
        (Duration::from_millis(50), Waited::Attached),
    ]);
    let began = clock.now;
    let attached = began + Duration::from_millis(100);
    clock.scheduled_attach = Some((attached, &rig));
    clock.frame_probe = Some(&rig);
    let interval = Duration::from_secs(5);
    let deadline = began + interval;
    let ((), calls) = spawns_during(|| {
        wait_between_cycles_with(&rig.server, "current", &mut carry, interval, &mut clock);
    });
    assert_eq!(
        clock.attached_at,
        Some(attached),
        "the scheduled real client must arrive"
    );
    assert!(
        clock.waits.len() >= 2,
        "stale wake and subsequent armed wait must execute"
    );
    assert!(
        calls.iter().any(|call| {
            ["list-panes", "set-option", "@ae_pane_state"]
                .iter()
                .all(|required| call.iter().any(|word| word.as_str() == *required))
        }),
        "the actual joined reader and frame writer must run"
    );
    let first = *clock
        .frame_receipts
        .first()
        .expect("an actual frame must land");
    assert!(first >= attached && first < deadline);
    assert!(
        first.duration_since(attached) <= Duration::from_millis(2_500),
        "P7-a stale-rearm frame exceeded the restated virtual bound: {:?}",
        first.duration_since(attached)
    );
    assert_eq!(
        clock.now, deadline,
        "the wake cannot advance the verdict cycle"
    );
    assert!(clock.waits.iter().all(|(_, end)| *end == deadline));
}

#[test]
fn quietwd_attached_without_working_agents_keeps_five_hundred_ms_observation_cadence() {
    let rig = Rig::new("no-working");
    rig.must(&["set-option", "-t", "=current:", "@ae_attn_rank", "1"]);
    rig.attach("current", 0);
    let mut carry = detached_carry(&rig, true);
    carry.motion.verdicts[0].verdict = Verdict::Quiet(crate::watchdog::QuietKind::Done);
    let mut clock = Clock::new([]);
    let began = clock.now;
    let ((), calls) = spawns_during(|| {
        wait_between_cycles_with(
            &rig.server,
            "current",
            &mut carry,
            Duration::from_millis(1001),
            &mut clock,
        );
    });
    let observations = calls
        .iter()
        .filter(|call| call.iter().any(|word| word == "list-panes"))
        .count();
    assert_eq!(
        observations, 3,
        "observe at 0, 500 and 1000 ms, not at frame cadence"
    );
    assert_eq!(calls.len(), 3, "one joined invocation per due observation");
    assert!(
        !calls
            .iter()
            .any(|call| call.iter().any(|word| word == "@ae_pane_state"))
    );
    assert_eq!(
        carry.motion.spin, 0,
        "no working agent means no pulse frame"
    );
    assert!(
        clock.waits.is_empty(),
        "an attached static session does not hold a waiter"
    );
    assert_eq!(clock.now.duration_since(began), Duration::from_millis(1001));
}

#[test]
fn quietwd_failed_wait_falls_back_without_ending_the_interval_early() {
    let rig = Rig::new("wait-failed");
    let mut carry = detached_carry(&rig, true);
    let mut clock = Clock::new([(Duration::from_millis(50), Waited::Failed)]);
    let began = clock.now;
    let ((), calls) = spawns_during(|| {
        wait_between_cycles_with(
            &rig.server,
            "current",
            &mut carry,
            Duration::from_secs(8),
            &mut clock,
        );
    });
    assert_eq!(clock.now.duration_since(began), Duration::from_secs(8));
    assert_eq!(
        clock.waits.len(),
        1,
        "a failed external wait switches to polling for this interval"
    );
    assert!(
        calls
            .iter()
            .any(|call| call.iter().any(|word| word == "list-panes")),
        "fallback must really observe"
    );
}

#[test]
fn quietwd_unproven_hooks_retain_polling_and_theme_off_retains_zero_ticker_work() {
    let rig = Rig::new("unproven");
    let mut carry = detached_carry(&rig, false);
    let mut clock = Clock::new([]);
    let ((), calls) = spawns_during(|| {
        wait_between_cycles_with(
            &rig.server,
            "current",
            &mut carry,
            Duration::from_secs(4),
            &mut clock,
        );
    });
    assert!(
        clock.waits.is_empty(),
        "never hold a waiter without this cycle proving its hook"
    );
    assert!(
        !calls.is_empty(),
        "unproven hooks must retain observation polling"
    );
    carry.look = Some(Look {
        drawn: false,
        ..Look::DEFAULT
    });
    carry.wake = Some("qw-off".into());
    let mut clock = Clock::new([]);
    let began = clock.now;
    let ((), calls) = spawns_during(|| {
        wait_between_cycles_with(
            &rig.server,
            "current",
            &mut carry,
            Duration::from_mins(1),
            &mut clock,
        );
    });
    assert_eq!(clock.now.duration_since(began), Duration::from_mins(1));
    assert!(clock.waits.is_empty());
    assert!(calls.is_empty());
}

#[test]
fn quietwd_server_reset_discards_old_hook_proof_and_publication_memory() {
    let mut carry = Carry::new();
    carry.wake = Some("old-server-channel".into());
    carry.motion.published_fleet = Some("old-server-strip".into());
    carry.look = Some(Look::DEFAULT);
    carry.reset();
    assert!(carry.wake.is_none());
    assert!(carry.motion.published_fleet.is_none());
    assert!(carry.look.is_none());
}

#[test]
fn quietwd_theme_off_still_maintains_only_an_adopted_peers_static_strip() {
    let rig = Rig::new("adoption");
    let (peer, _, _) = rig.add_session("peer");
    rig.must(&["set-option", "-t", "=peer:", "@ae_attn_rank", "1"]);
    let mut carry = detached_carry(&rig, true);
    carry.look = Some(Look {
        drawn: false,
        ..Look::DEFAULT
    });
    carry.adoption.known = vec!["peer".into()];
    carry.adoption.targets = vec![super::Adopted {
        name: "peer".into(),
        id: peer,
        meta_dir: rig.path("unwatched-peer"),
        enum_pid: None,
        published: None,
    }];
    let mut clock = Clock::new([]);
    let ((), calls) = spawns_during(|| {
        wait_between_cycles_with(
            &rig.server,
            "current",
            &mut carry,
            Duration::from_secs(4),
            &mut clock,
        );
    });
    assert!(
        clock.waits.is_empty(),
        "an adopter cannot sleep through its peer duty"
    );
    assert!(
        calls
            .iter()
            .any(|call| call.iter().any(|word| word == "list-sessions"))
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.iter().any(|word| word == "list-panes"))
    );
    let writes: Vec<_> = calls
        .iter()
        .filter(|call| call.iter().any(|word| word == "set-option"))
        .collect();
    assert!(!writes.is_empty());
    for call in writes {
        let options: Vec<_> = call
            .iter()
            .filter(|word| word.starts_with("@ae_"))
            .map(String::as_str)
            .collect();
        assert_eq!(
            options,
            [crate::theme::FLEET_STRIP_OPTION],
            "an adopter owns only the strip: {call:?}"
        );
        assert!(!call.iter().any(|word| word == "-g"));
    }
    assert!(
        rig.option("", "=peer:", crate::theme::FLEET_STRIP_OPTION)
            .contains("peer")
    );
}
