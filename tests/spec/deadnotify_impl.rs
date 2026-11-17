// #211 implementer tests, beside the frozen acceptance in `deadnotify.rs` and
// included after it in watchdog_daemon::tests, so they share its `DnRig`. They
// pin what the frozen file leaves open: the pane-gone retry bound, and the
// `pane-back` record's absence wherever no told death ended.

// The hp and the Mac login shell is fish, which is tmux's default-shell: a
// single-string pane command runs as `fish -c`, and its agent stand-in shows up
// as a descendant only after fish has started. These helpers use tmux's argv
// form and wait for the agent evidence before the first cycle.
fn wait_agents(rig: &DnRig) {
    let end = std::time::Instant::now() + Duration::from_mins(1);
    loop {
        let panes = crate::transport::observe_watch_panes(&rig.tmux.server, "current")
            .expect("owned panes");
        let table = crate::procs::snapshot();
        let ready = !panes.is_empty()
            && panes.iter().all(|p| {
                p.current_command == "sleep"
                    || matches!((p.pane_pid, table.as_deref()), (Some(pid), Some(table))
                        if crate::procs::has_descendant_named(table, pid, "sleep"))
            });
        if ready {
            return;
        }
        assert!(
            std::time::Instant::now() < end,
            "fixture must show the agent stand-in in every pane"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn revive_by_argv(rig: &DnRig) {
    rig.tmux.must(&[
        "respawn-pane",
        "-k",
        "-t",
        &rig.tmux.pane,
        "/bin/sh",
        "-c",
        "sleep 600; :",
    ]);
    wait_agents(rig);
}

fn records(rig: &DnRig, action: &str) -> usize {
    read_events(&rig.dir)
        .iter()
        .filter(|e| e.action == action)
        .count()
}

#[test]
#[allow(clippy::disallowed_methods, reason = "owned external failure oracle")]
fn deadnotify_impl_pane_gone_retries_only_proven_pre_submit_failures_and_stops_at_three() {
    for retryable in [false, true] {
        let rig = DnRig::new("parent", true);
        rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
        let script = if retryable {
            "#!/bin/sh\nprintf 'ae-send: retryable-before-submit\\n'\nexit 1\n"
        } else {
            "#!/bin/sh\nexit 1\n"
        };
        std::fs::write(rig.helper.path(), script).expect("owned regular stub");
        let mut carry = Carry::new();
        let counts: Vec<_> = (0..5).map(|_| rig.pass(&mut carry).len()).collect();
        assert_eq!(
            counts,
            if retryable {
                vec![1, 1, 1, 0, 0]
            } else {
                vec![1, 0, 0, 0, 0]
            }
        );
        assert_eq!(
            rig.notices(),
            0,
            "a refusal is never reported as a delivery"
        );
    }
}

#[test]
fn deadnotify_impl_live_seats_book_no_notice_and_no_pane_back() {
    let rig = DnRig::new("parent", true);
    wait_agents(&rig);
    let mut carry = Carry::new();
    assert!(rig.pass(&mut carry).is_empty());
    assert!(rig.pass(&mut carry).is_empty());
    assert_eq!(records(&rig, "dead-notice"), 0);
    assert_eq!(
        records(&rig, "pane-back"),
        0,
        "nothing died, nothing returned"
    );
}

#[test]
fn deadnotify_impl_a_revived_latch_is_cleared_by_dead_cleared_alone() {
    let rig = DnRig::new("parent", true);
    let mut carry = Carry::new();
    rig.shell();
    DnRig::assert_notice(&rig.pass(&mut carry), "shell");
    revive_by_argv(&rig);
    assert!(rig.pass(&mut carry).is_empty());
    assert_eq!(records(&rig, "dead-cleared"), 1, "the latch clears itself");
    assert_eq!(records(&rig, "pane-back"), 0, "and books no second closer");
    assert!(rig.pass(&mut carry).is_empty());
    assert_eq!(records(&rig, "pane-back"), 0);
}

// One death episode keeps ONE notice standing per seat across the shell and the
// pane-gone legs (review finding, ae-20261117T003610Z-1ea2c9ee): a try the
// helper could not prove refused is never repeated, a proven refusal spends the
// same three tries whichever leg makes them, and only a sign of life ends it.
fn stub(rig: &DnRig, retryable: bool) {
    let script = if retryable {
        "#!/bin/sh\nprintf 'ae-send: retryable-before-submit\\n'\nexit 1\n"
    } else {
        "#!/bin/sh\nexit 1\n"
    };
    #[allow(clippy::disallowed_methods, reason = "owned external failure oracle")]
    std::fs::write(rig.helper.path(), script).expect("owned regular stub");
}

fn dispatches(rig: &DnRig, carry: &mut Carry, cycles: usize) -> Vec<usize> {
    (0..cycles).map(|_| rig.pass(carry).len()).collect()
}

fn lose_pane(rig: &DnRig) {
    rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
}

fn return_pane(rig: &DnRig, command: &[&str]) -> String {
    let mut args = vec![
        "new-window",
        "-d",
        "-t",
        "=current:",
        "-P",
        "-F",
        "#{pane_id}",
    ];
    args.extend_from_slice(command);
    let pane = rig.tmux.must(&args).trim().to_owned();
    rig.tmux
        .must(&["set-option", "-p", "-t", &pane, "@ae_agent", "worker"]);
    rig.tmux
        .must(&["set-option", "-p", "-t", &pane, "@ae_slot", "spawned.0"]);
    pane
}

// A pane that was just created reads as `tmux` for a moment, which is no shell
// and so a sign of life: wait for the settled shell before the cycles judge it.
fn settle_shell(rig: &DnRig, pane: &str) {
    let end = std::time::Instant::now() + Duration::from_mins(1);
    let mut steady = 0;
    loop {
        let panes = crate::transport::observe_watch_panes(&rig.tmux.server, "current")
            .expect("owned panes");
        let shell = panes.iter().any(|p| {
            p.pane_id == pane && crate::watchdog::command_is_shell(p.current_command.trim())
        });
        steady = if shell { steady + 1 } else { 0 };
        if steady >= 3 {
            return;
        }
        assert!(
            std::time::Instant::now() < end,
            "the returned pane must settle on a shell"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn journal(rig: &DnRig, line: &str) {
    crate::store::open(&rig.dir)
        .append_event(&format!("{line}\n"))
        .expect("owned event");
}

// A reseat of the seat, as a peer's `ae reseat` journals it: addressed to the
// seat, by a caller that is not the seat.
const RESEAT: &str =
    r#"{"ts":"2026-11-15T00:05:00Z","actor":"parent","action":"reseat","target":"worker"}"#;

#[test]
fn deadnotify_impl_an_uncertain_shell_try_then_pane_loss_is_one_dispatch_in_all() {
    let rig = DnRig::new("parent", true);
    stub(&rig, false);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 1), [1]);
    lose_pane(&rig);
    assert_eq!(dispatches(&rig, &mut carry, 3), [0, 0, 0]);
}

#[test]
fn deadnotify_impl_two_safe_shell_refusals_then_pane_loss_stop_at_three_in_all() {
    let rig = DnRig::new("parent", true);
    stub(&rig, true);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 2), [1, 1]);
    lose_pane(&rig);
    assert_eq!(dispatches(&rig, &mut carry, 3), [1, 0, 0]);
}

#[test]
fn deadnotify_impl_a_pane_returning_as_a_dead_shell_continues_the_episode() {
    let rig = DnRig::new("parent", true);
    stub(&rig, false);
    lose_pane(&rig);
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 1), [1]);
    let pane = return_pane(&rig, &["/bin/sh"]);
    settle_shell(&rig, &pane);
    assert_eq!(dispatches(&rig, &mut carry, 3), [0, 0, 0]);
}

#[test]
fn deadnotify_impl_a_sign_of_life_ends_an_unrecorded_episode() {
    // The refused try left no journal record, so only memory knows the standing;
    // a revival, or a pane that stands live again, must still start a new one.
    let rig = DnRig::new("parent", true);
    stub(&rig, false);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 1), [1]);
    revive_by_argv(&rig);
    assert_eq!(dispatches(&rig, &mut carry, 1), [0]);
    rig.shell();
    assert_eq!(dispatches(&rig, &mut carry, 2), [1, 0]);

    let rig = DnRig::new("parent", true);
    stub(&rig, false);
    lose_pane(&rig);
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 1), [1]);
    let pane = return_pane(&rig, &["/bin/sh", "-c", "sleep 600; :"]);
    wait_agents(&rig);
    assert_eq!(dispatches(&rig, &mut carry, 1), [0]);
    rig.tmux.must(&["kill-pane", "-t", &pane]);
    assert_eq!(dispatches(&rig, &mut carry, 2), [1, 0]);
}

#[test]
fn deadnotify_impl_a_delivered_notice_is_told_again_after_a_reseat_between_observations() {
    let rig = DnRig::new("parent", true);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 2), [1, 0]);
    // The new incarnation died before any cycle read it alive.
    journal(&rig, RESEAT);
    assert_eq!(dispatches(&rig, &mut carry, 3), [1, 0, 0]);
}

#[test]
fn deadnotify_impl_an_uncertain_notice_is_tried_again_after_a_reseat_between_observations() {
    let rig = DnRig::new("parent", true);
    stub(&rig, false);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 2), [1, 0]);
    journal(&rig, RESEAT);
    assert_eq!(dispatches(&rig, &mut carry, 3), [1, 0, 0]);
}

#[test]
fn deadnotify_impl_a_reseat_gives_a_fresh_budget_and_an_old_one_never_resets_it() {
    // Three proven refusals spent, a reseat, three more.
    let rig = DnRig::new("parent", true);
    stub(&rig, true);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 4), [1, 1, 1, 0]);
    journal(&rig, RESEAT);
    assert_eq!(dispatches(&rig, &mut carry, 4), [1, 1, 1, 0]);
    // A closer that is already in the journal when the seat dies spends the
    // budget once, not on every cycle.
    let rig = DnRig::new("parent", true);
    journal(&rig, RESEAT);
    stub(&rig, true);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 5), [1, 1, 1, 0, 0]);
}

// The alert is gone from the journal, so the next cycle clears the latch with
// `dead-cleared` and the same reading latches the dead shell again: both latch
// readings are true, and the closer is only the record the cycle itself booked.
#[allow(clippy::disallowed_methods, reason = "owned journal fixture")]
fn drop_alerts(rig: &DnRig) {
    let file = rig.dir.join(crate::store::EVENTS);
    let mut kept = String::new();
    for line in std::fs::read_to_string(&file)
        .expect("owned journal")
        .lines()
        .filter(|line| !line.contains("\"action\":\"alert\""))
    {
        kept.push_str(line);
        kept.push('\n');
    }
    std::fs::remove_file(&file).expect("owned journal");
    std::fs::write(&file, kept).expect("owned journal");
}

#[test]
fn deadnotify_impl_a_clear_and_re_latch_in_one_cycle_starts_a_new_episode() {
    let rig = DnRig::new("parent", true);
    stub(&rig, false);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 2), [1, 0]);
    drop_alerts(&rig);
    assert_eq!(dispatches(&rig, &mut carry, 1), [1]);
    assert_eq!(records(&rig, "dead-cleared"), 1);
    assert_eq!(dispatches(&rig, &mut carry, 1), [0]);
}

#[test]
fn deadnotify_impl_a_clear_and_re_latch_after_a_delivered_notice_is_told_again() {
    // The journal holds the delivered notice; the cycle's own `dead-cleared`
    // ends that episode, so the same cycle's re-latch is a new one.
    let rig = DnRig::new("parent", true);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 2), [1, 0]);
    assert_eq!(records(&rig, "dead-notice"), 1);
    drop_alerts(&rig);
    assert_eq!(dispatches(&rig, &mut carry, 1), [1]);
    assert_eq!(records(&rig, "dead-notice"), 2);
    assert_eq!(dispatches(&rig, &mut carry, 2), [0, 0]);
}

// An older session may still hold a seat named like the daemon. The daemon,
// and every sender that speaks for it through the helpers, writes records that
// name `watchdog` and carry no routing key; none of them is that seat's own
// activity, so its notice record neither ends its episode nor retracts its
// alert, and the seat is told once and alerted once however long it stays dead.
#[allow(clippy::disallowed_methods, reason = "owned meta and journal fixture")]
fn legacy_watchdog_rig() -> DnRig {
    let rig = DnRig::new("parent", true);
    let meta = crate::store::open(&rig.dir).meta_path();
    let rows = std::fs::read_to_string(&meta).expect("owned meta");
    std::fs::write(
        &meta,
        rows.replace("seat.spawned.0=worker", "seat.spawned.0=watchdog"),
    )
    .expect("owned meta");
    rig.tmux.must(&[
        "set-option",
        "-p",
        "-t",
        &rig.tmux.pane,
        "@ae_agent",
        "watchdog",
    ]);
    let file = rig.dir.join(crate::store::EVENTS);
    std::fs::remove_file(&file).expect("owned journal");
    rig.append_spawn(
        r#"{"ts":"2026-11-15T00:00:00Z","actor":"parent","action":"spawn","target":"watchdog","birth":"worker-first"}"#,
    );
    rig.shell();
    rig
}

#[test]
fn deadnotify_impl_a_seat_named_watchdog_is_told_once_and_alerted_once_over_many_cycles() {
    let rig = legacy_watchdog_rig();
    let mut carry = Carry::new();
    assert_eq!(dispatches(&rig, &mut carry, 5), [1, 0, 0, 0, 0]);
    assert_eq!(records(&rig, "dead-notice"), 1);
    assert_eq!(records(&rig, "alert"), 1);
    assert_eq!(records(&rig, "dead-cleared"), 0);
}

#[test]
fn deadnotify_impl_a_seat_named_watchdog_is_not_told_again_by_a_restarted_daemon() {
    let rig = legacy_watchdog_rig();
    assert_eq!(dispatches(&rig, &mut Carry::new(), 1), [1]);
    assert_eq!(records(&rig, "dead-notice"), 1);
    journal(
        &rig,
        r#"{"ts":"2026-11-15T00:06:00Z","actor":"watchdog","action":"nudge","target":"api"}"#,
    );
    // A restart after the first cycle: nothing in memory, the journal alone
    // says it was told, and the notice did not retract the alert.
    assert_eq!(dispatches(&rig, &mut Carry::new(), 4), [0, 0, 0, 0]);
    assert_eq!(records(&rig, "dead-notice"), 1);
    assert_eq!(records(&rig, "dead-cleared"), 0);
    // The journal latches the human alert across daemon restarts, including
    // a legacy seat named `watchdog`; daemon activity is no seat recovery.
    assert_eq!(
        records(&rig, "alert"),
        1,
        "one human alert per death across daemon restarts"
    );
}

// What a refused dead notice leaves in the journal, in the bytes the product's
// own writers produce: the helper books a `delivery-abandoned` record under the
// sender `watchdog`, keyless, with the dead seat as its ref. `retryable` is a
// refusal proven before the submit (the helper also says so); the other is an
// unproven notice, which is never retried.
#[allow(clippy::disallowed_methods, reason = "owned external failure oracle")]
fn refusing_stub(rig: &DnRig, retryable: bool) {
    let scratch = rig.tmux.path("refusal");
    std::fs::create_dir_all(&scratch).expect("owned scratch");
    let fields = crate::tracked::EventFields::new(
        crate::time::Timestamp::now(),
        "watchdog",
        "dead-notice",
        "parent",
        "watchdog",
        "",
        "",
        "",
        "",
        "worker watchdog is DEAD",
        "",
    );
    let mut sink = Vec::new();
    if retryable {
        crate::tracked::record_abandoned_delivery(
            &scratch,
            &fields,
            crate::deliver::DeferHeld::ComposerOccupied,
            &mut sink,
        )
    } else {
        crate::tracked::record_unproven_notice(&scratch, &fields, &mut sink)
    }
    .expect("owned refusal record");
    let line = std::fs::read_to_string(scratch.join(crate::store::EVENTS)).expect("owned record");
    let marker = if retryable {
        "printf 'ae-send: retryable-before-submit\\n'\n"
    } else {
        ""
    };
    let script = format!(
        "#!/bin/sh\n{marker}cat >> '{}' <<'LINE'\n{line}LINE\nexit 1\n",
        rig.dir.join(crate::store::EVENTS).display()
    );
    std::fs::write(rig.helper.path(), script).expect("owned regular stub");
}

#[test]
fn deadnotify_impl_a_refused_notice_never_closes_the_episode_of_a_seat_named_watchdog() {
    for retryable in [true, false] {
        let rig = legacy_watchdog_rig();
        refusing_stub(&rig, retryable);
        let mut carry = Carry::new();
        assert_eq!(
            dispatches(&rig, &mut carry, 6),
            if retryable {
                [1, 1, 1, 0, 0, 0]
            } else {
                [1, 0, 0, 0, 0, 0]
            },
            "retryable={retryable}: a refusal spends the budget and never resets it"
        );
        assert_eq!(
            records(&rig, "delivery-abandoned"),
            if retryable { 3 } else { 1 },
            "retryable={retryable}"
        );
        assert_eq!(records(&rig, "dead-cleared"), 0, "retryable={retryable}");
        assert_eq!(records(&rig, "alert"), 1, "retryable={retryable}");
    }
}

// One cycle's reading of a LIVE pane's seat, through `notice_step` alone: the
// readings no real cycle of the tmux rig can hold still (a shell that is not
// yet latched, a live seat whose latch the daemon just cleared). Returns what
// the step booked.
fn read_pane(
    reading: (bool, bool),
    seat_state: (bool, &str),
    standing: &mut super::Standing,
    journal: &[&str],
) -> Vec<String> {
    let scratch = Scratch::new("notice-step");
    let helper = journaling_helper(&scratch.0);
    let server = ServerId::Ambient;
    let events: Vec<Event> = journal
        .iter()
        .map(|line| Event::parse_line(line).expect("a fixture line"))
        .collect();
    let mut observed = seen();
    (observed.is_dead, observed.descendancy) = (seat_state.0, Descendancy::Absent);
    let on = super::Acting {
        agent: "worker",
        slot: "spawned.0",
        pane: "%0",
        seen: &observed,
        events: &events,
        overview: None,
    };
    let reading = super::Reading {
        was_latched: reading.0,
        latched: reading.1,
        cleared: false,
        command: seat_state.1,
    };
    demo_cycle(&scratch.0, &helper, &server)
        .notice_step(&on, &reading, (false, standing), &mut Vec::new())
        .expect("the step books");
    read_events(&scratch.0)
        .into_iter()
        .map(|event| event.action)
        .collect()
}

const SPAWNED: &str =
    r#"{"ts":"2026-11-15T00:00:00Z","actor":"parent","action":"spawn","target":"worker"}"#;
const NOTICE: &str = r#"{"ts":"2026-11-15T00:01:00Z","actor":"watchdog","action":"dead-notice","target":"parent","ref":"worker"}"#;

#[test]
fn deadnotify_impl_a_dead_shell_not_yet_latched_keeps_its_standing_and_is_not_back() {
    // A told seat whose pane shows a dead shell again before the latch arms:
    // no sign of life, so no `pane-back`, and the standing stays at its episode
    // with the tries it has spent.
    let mut standing = super::Standing::owed(1, 1);
    let booked = read_pane(
        (false, false),
        (false, "sh"),
        &mut standing,
        &[SPAWNED, NOTICE],
    );
    assert_eq!(booked, Vec::<String>::new());
    assert_eq!(standing, super::Standing::owed(1, 1));
}

#[test]
fn deadnotify_impl_a_latch_the_daemon_cleared_ends_the_standing_without_a_pane_back() {
    // The seat lives and the daemon cleared its latch this cycle: the clear is
    // the retraction, so the standing ends and no `pane-back` follows it.
    let mut standing = super::Standing::settled(1);
    let booked = read_pane(
        (true, false),
        (false, "sleep"),
        &mut standing,
        &[SPAWNED, NOTICE],
    );
    assert_eq!(booked, Vec::<String>::new());
    assert_eq!(standing, super::Standing::default());
}

#[test]
fn deadnotify_impl_a_told_seat_that_lives_again_with_no_standing_books_pane_back_once() {
    // A restarted daemon holds no standing; the journal says the seat was told
    // and nothing closed the episode since. A positive reading books the return.
    let mut standing = super::Standing::default();
    let booked = read_pane(
        (false, false),
        (false, "sleep"),
        &mut standing,
        &[SPAWNED, NOTICE],
    );
    assert_eq!(booked, ["pane-back"]);
    assert_eq!(standing, super::Standing::default());
    // A seat that was never told has nothing to take back.
    let booked = read_pane((false, false), (false, "sleep"), &mut standing, &[SPAWNED]);
    assert_eq!(booked, Vec::<String>::new());
}
