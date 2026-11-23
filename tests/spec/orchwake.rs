// Frozen orchwake acceptance (navigator orchwake-nav, Anthropic Opus 5.5),
// installed byte-identically and included in watchdog_daemon::tests.
// Invariant 2: the orchestrator main's sweep runs the existing human-prompt
// lifecycle before any overview delivery. Invariant 3: a human-only prompt
// that holds the orchestrator seat longer than one cycle tells the owner once,
// through the watchdog's own targetless chat row (the row the console shows
// and the Telegram bridge relays), and again only after the state changes.
// Invariant 1 (the durable continuation) is frozen as the scenario contract in
// .local/orchwake/contract.md; its seam is the driver's design.

/// The orchestrator main as the cycle sees it: the sweep branch applies, the
/// overview is changed and due, and the pane shows `prompt` (or none).
fn orchwake_seat(prompt: Option<crate::watchdog::HumanPrompt>) -> Observation {
    let mut observed = seen();
    observed.sweep = Some(SweepObservation::new(at(0), None));
    observed.human_prompt = prompt;
    observed
}

fn orchwake_report_only() -> crate::watchdog::HumanPrompt {
    crate::watchdog::HumanPrompt {
        kind: crate::watchdog::DialogKind::Login,
        question: "Select login method".to_owned(),
        keys: "enter to select · esc to cancel".to_owned(),
        ..prompt()
    }
}

fn orchwake_sweeps(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::SweepNudge))
        .count()
}

/// A positive prompt holds the overview: nothing is pasted, no retry is
/// spent, and the prompt is counted and named exactly as on any other seat.
#[test]
fn orchwake_a_prompt_on_the_orchestrator_holds_the_overview_and_is_named_once() {
    let knobs = Knobs::default();
    let modal = orchwake_seat(Some(orchwake_report_only()));
    let prior = PaneState {
        sweep: crate::watchdog::SweepState {
            fails: 2,
            ..crate::watchdog::SweepState::default()
        },
        ..PaneState::default()
    };

    let first = account(&prior, &modal, &knobs);
    assert_eq!(orchwake_sweeps(&first.effects), 0, "{:?}", first.effects);
    assert_eq!(first.next.human_prompt_streak, 1, "the prompt is counted");
    assert_eq!(first.next.sweep.fails, 2, "a hold spends no retry");
    assert_eq!(first.next.sweep.last_sweep, None, "a hold moves no cadence");
    assert!(
        emitted(&first.effects).is_empty(),
        "one cycle is a redraw: {:?}",
        first.effects
    );

    let second = account(&first.next, &modal, &knobs);
    assert_eq!(orchwake_sweeps(&second.effects), 0, "{:?}", second.effects);
    assert_eq!(second.next.sweep.fails, 2, "a hold spends no retry");
    let named: Vec<&str> = emitted(&second.effects)
        .into_iter()
        .filter(|(action, _)| *action == "human-prompt")
        .map(|(_, summary)| summary)
        .collect();
    assert_eq!(named.len(), 1, "named once: {:?}", second.effects);
    assert!(named[0].contains("Select login method"), "{}", named[0]);
    assert!(named[0].contains("enter to select"), "{}", named[0]);

    let third = account(&second.next, &modal, &knobs);
    assert_eq!(orchwake_sweeps(&third.effects), 0, "{:?}", third.effects);
    assert!(
        emitted(&third.effects)
            .iter()
            .all(|(action, _)| *action != "human-prompt"),
        "not named again: {:?}",
        third.effects
    );
}

/// The first cycle that READS the pane clear retracts the naming and books
/// the held overview in the same cycle; an unread frame is no clear.
#[test]
fn orchwake_the_first_clear_read_retracts_the_prompt_and_delivers_the_overview() {
    let knobs = Knobs::default();
    let modal = orchwake_seat(Some(orchwake_report_only()));
    let first = account(&PaneState::default(), &modal, &knobs);
    let named = account(&first.next, &modal, &knobs);
    assert_eq!(named.next.human_prompt_streak, 2);

    let mut unread = orchwake_seat(None);
    unread.capture_ok = false;
    let held = account(&named.next, &unread, &knobs);
    assert!(
        emitted(&held.effects)
            .iter()
            .all(|(action, _)| *action != "human-prompt-cleared"),
        "an unread frame is no clear: {:?}",
        held.effects
    );
    assert_eq!(held.next.human_prompt_streak, 2, "the latch holds");

    let clear = account(&named.next, &orchwake_seat(None), &knobs);
    assert!(
        emitted(&clear.effects)
            .iter()
            .any(|(action, _)| *action == "human-prompt-cleared"),
        "{:?}",
        clear.effects
    );
    assert_eq!(clear.next.human_prompt_streak, 0);
    assert_eq!(
        orchwake_sweeps(&clear.effects),
        1,
        "the held overview goes on the first clear cycle: {:?}",
        clear.effects
    );
}

/// An exact eligible trust block uses the existing proof from its first
/// sight; a doubtful journal answers nothing and names the prompt at the bound.
#[test]
fn orchwake_the_orchestrator_trust_block_takes_the_existing_proof_and_nothing_else() {
    let knobs = Knobs::default();
    let trust = orchwake_seat(Some(prompt()));

    let due = crate::autotrust::Due::Yes;
    let answered = super::account_with_trust(&PaneState::default(), &trust, &knobs, Some(&due));
    assert!(
        answered
            .effects
            .contains(&Effect::AnswerTrust { name: false }),
        "{:?}",
        answered.effects
    );
    assert_eq!(
        orchwake_sweeps(&answered.effects),
        0,
        "{:?}",
        answered.effects
    );

    let doubt = crate::autotrust::Due::Doubt("journal unreadable".to_owned());
    let first = super::account_with_trust(&PaneState::default(), &trust, &knobs, Some(&doubt));
    let second = super::account_with_trust(&first.next, &trust, &knobs, Some(&doubt));
    for booked in [&first, &second] {
        assert!(
            booked
                .effects
                .iter()
                .all(|effect| !matches!(effect, Effect::AnswerTrust { .. })),
            "doubt answers nothing: {:?}",
            booked.effects
        );
        assert_eq!(orchwake_sweeps(&booked.effects), 0, "{:?}", booked.effects);
    }
    assert!(
        emitted(&second.effects)
            .iter()
            .any(|(action, _)| *action == "human-prompt"),
        "report-only at the bound: {:?}",
        second.effects
    );
}

/// The owner rows: the watchdog's own targetless chat lines.
fn orchwake_owner_rows(dir: &Path) -> Vec<String> {
    read_events(dir)
        .into_iter()
        .filter(|event| event.action == "chat" && event.actor == ACTOR && event.target.is_none())
        .map(|event| event.summary.unwrap_or_default())
        .collect()
}

/// One cycle's booking for the orchestrator main, applied as the cycle applies
/// it, against this cycle's journal. The server is a private socket nobody
/// serves, so a display or key effect reaches no real tmux.
fn orchwake_cycle(
    dir: &Path,
    helper: &SendHelper,
    server: &ServerId,
    prior: &PaneState,
    observed: &Observation,
) -> PaneState {
    let knobs = Knobs::default();
    let booked = account(prior, observed, &knobs);
    let events = read_events(dir);
    let on = super::Acting {
        agent: "orchestrator",
        slot: "main",
        pane: "%0",
        seen: observed,
        events: &events,
        overview: None,
    };
    let mut state = booked.next.clone();
    let mut refresh = false;
    let mut err = Vec::new();
    demo_cycle(dir, helper, server)
        .apply_booked(&booked.effects, &on, &mut state, &mut refresh, &mut err)
        .expect("the booking applies");
    state
}

/// Invariant 3: told once per episode, never on a redraw, not again after a
/// daemon restart, and again once the prompt cleared and a new one held.
#[test]
fn orchwake_a_held_prompt_tells_the_owner_once_per_episode() {
    let scratch = Scratch::new("orchwake-owner");
    let helper = journaling_helper(&scratch.0);
    let server = ServerId::Selected(Selector::Socket(scratch.0.join("nobody.sock")));
    let mut login = orchwake_report_only();
    login.question = "Select login method\u{1b}[31m".to_owned();
    let modal = orchwake_seat(Some(login));

    let first = orchwake_cycle(&scratch.0, &helper, &server, &PaneState::default(), &modal);
    assert!(
        orchwake_owner_rows(&scratch.0).is_empty(),
        "one cycle is a redraw"
    );
    let second = orchwake_cycle(&scratch.0, &helper, &server, &first, &modal);
    let told = orchwake_owner_rows(&scratch.0);
    assert_eq!(told.len(), 1, "told once: {told:?}");
    assert!(
        told[0].contains("orchestrator"),
        "names the seat: {}",
        told[0]
    );
    assert!(
        told[0].contains("Select login method"),
        "names the dialog: {}",
        told[0]
    );
    assert!(
        told[0].contains("enter to select"),
        "names the action: {}",
        told[0]
    );
    assert!(!told[0].contains('\u{1b}'), "inert: {:?}", told[0]);
    let _ = orchwake_cycle(&scratch.0, &helper, &server, &second, &modal);
    assert_eq!(orchwake_owner_rows(&scratch.0).len(), 1, "not on a loop");

    // A restarted daemon: no carried state, the journal says the prompt is named.
    let restarted = Observation {
        prompt_named: true,
        ..modal.clone()
    };
    let again = orchwake_cycle(
        &scratch.0,
        &helper,
        &server,
        &PaneState::default(),
        &restarted,
    );
    let again = orchwake_cycle(&scratch.0, &helper, &server, &again, &restarted);
    let _ = orchwake_cycle(&scratch.0, &helper, &server, &again, &restarted);
    assert_eq!(
        orchwake_owner_rows(&scratch.0).len(),
        1,
        "a restart repeats no unchanged notice"
    );

    // The prompt clears, then a different one holds: a new episode.
    let cleared = Observation {
        prompt_named: true,
        ..orchwake_seat(None)
    };
    let clear = orchwake_cycle(&scratch.0, &helper, &server, &again, &cleared);
    assert_eq!(
        orchwake_owner_rows(&scratch.0).len(),
        1,
        "a clear tells nothing"
    );
    let trust = orchwake_seat(Some(prompt()));
    let next = orchwake_cycle(&scratch.0, &helper, &server, &clear, &trust);
    let _ = orchwake_cycle(&scratch.0, &helper, &server, &next, &trust);
    let told = orchwake_owner_rows(&scratch.0);
    assert_eq!(told.len(), 2, "the new episode is told: {told:?}");
    assert!(
        told[1].contains("Do you trust the contents of this project?"),
        "{}",
        told[1]
    );
}
