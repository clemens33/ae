//! The return path's pure core (#305 part 1): the chain of automatic moves,
//! the origin's proof of room, the return episode's fold and the flap count.

use super::*;

const SESSION: &str = "aedev";
const SLOT: &str = "spawned.3";
const AGENT: &str = "scout";
const T0: i64 = 1_790_000_000;

fn at(since: i64) -> Timestamp {
    Timestamp::from_epoch(T0 + since)
}

/// A record `since` seconds after `T0` by `actor`, addressed to the seat by
/// routing key, as the legs and the verbs write them.
fn record(since: i64, actor: &str, action: &str, reference: &str, summary: &str) -> Event {
    let reference = if reference.is_empty() {
        String::new()
    } else {
        format!(r#","ref":"{reference}""#)
    };
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"{actor}","action":"{action}","target":"{AGENT}","target_slot":"{SLOT}","target_session":"{SESSION}"{reference},"summary":"{summary}"}}"#,
        at(since)
    ))
    .expect("a well-formed specimen")
}

/// A `reseat` record exactly as `ae reseat` journals it: the target's name
/// and slot, its pane, and no session key at all.
fn reseat(since: i64, actor: &str, summary: &str) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"{actor}","action":"reseat","target":"{AGENT}","target_slot":"{SLOT}","target_pane":"%1","caller_pane":"%0","summary":"{summary}"}}"#,
        at(since)
    ))
    .expect("a well-formed specimen")
}

fn moved(since: i64, actor: &str, to: &str) -> Event {
    reseat(
        since,
        actor,
        &format!("reseated {AGENT} (pane %1, slot {SLOT}) to {to}, seed submitted"),
    )
}

/// One whole automatic hop opened by `opener` at `since`: opener, attempt,
/// the watchdog's moved `reseat`, done.
fn hop(since: i64, opener: &str, from: &str, to: &str) -> Vec<Event> {
    let key = at(since).to_string();
    vec![
        record(since, WATCHDOG_ACTOR, opener, "", "cause"),
        record(since + 1, WATCHDOG_ACTOR, ATTEMPT_ACTION, &key, "attempt"),
        moved(since + 2, WATCHDOG_ACTOR, to),
        record(since + 3, WATCHDOG_ACTOR, DONE_ACTION, from, "to"),
    ]
}

fn chain_of(events: &[Event]) -> Option<Chain> {
    chain(events, SESSION, SLOT, AGENT)
}

fn origin(found: Option<Chain>) -> Option<String> {
    found.map(|chain| chain.origin)
}

#[test]
fn a_headroom_or_limit_hop_starts_a_chain_at_the_profile_it_left() {
    for opener in [HEADROOM_ACTION, LIMIT_ACTION] {
        let events = hop(0, opener, "origin", "substitute");
        let found = chain_of(&events).expect("one automatic hop is a chain");
        assert_eq!(found.origin, "origin");
        assert_eq!(found.away_at, at(3));
        assert_eq!(found.newest, at(3));
    }
}

#[test]
fn a_multi_hop_chain_keeps_its_root_and_first_away_stamp() {
    let mut events = hop(0, HEADROOM_ACTION, "origin", "middle");
    events.push(record(5, WATCHDOG_ACTOR, CLEARED_ACTION, "", ""));
    events.extend(hop(10, LIMIT_ACTION, "middle", "substitute"));
    let found = chain_of(&events).expect("two hops");
    assert_eq!(found.origin, "origin");
    assert_eq!(found.away_at, at(3));
    assert_eq!(found.newest, at(13));
}

#[test]
fn any_reseat_but_the_watchdogs_ends_the_chain_whatever_it_says() {
    for summary in [
        "already using substitute",
        "stopped claude in place (pane %1)",
    ] {
        let mut events = hop(0, HEADROOM_ACTION, "origin", "substitute");
        events.push(reseat(9, "human", summary));
        assert_eq!(chain_of(&events), None, "{summary}");
    }
    let mut events = hop(0, HEADROOM_ACTION, "origin", "substitute");
    events.push(moved(9, "lead", "origin"));
    assert_eq!(chain_of(&events), None, "a manual move home is a choice");
}

#[test]
fn a_manual_reseat_between_the_watchdog_move_and_its_done_is_never_reclaimed() {
    let mut events = hop(0, HEADROOM_ACTION, "origin", "substitute");
    let done = events.pop().expect("the done");
    events.push(moved(3, "human", "manual"));
    events.push(done);
    assert_eq!(chain_of(&events), None);
    // Nor does a later hop's done revive the old root.
    events.extend(hop(20, LIMIT_ACTION, "manual", "substitute"));
    assert_eq!(origin(chain_of(&events)).as_deref(), Some("manual"));
}

#[test]
fn an_orphan_done_or_a_move_with_no_done_is_no_provenance() {
    let orphan: Vec<Event> = hop(0, HEADROOM_ACTION, "origin", "substitute")
        .into_iter()
        .filter(|event| event.action != RESEAT_ACTION)
        .collect();
    assert_eq!(chain_of(&orphan), None);
    let mut open = hop(0, HEADROOM_ACTION, "origin", "substitute");
    open.extend(hop(20, LIMIT_ACTION, "substitute", "manual"));
    open.pop();
    assert_eq!(chain_of(&open), None, "the newest move is not closed");
    // A watchdog `reseat` that moved nothing does not suspend the chain.
    let mut stop = hop(0, HEADROOM_ACTION, "origin", "substitute");
    stop.push(reseat(9, WATCHDOG_ACTOR, "stopped claude in place"));
    assert_eq!(origin(chain_of(&stop)).as_deref(), Some("origin"));
}

#[test]
fn a_spawn_or_a_return_hop_ends_the_chain_and_returns_are_counted() {
    let mut events = hop(0, HEADROOM_ACTION, "origin", "substitute");
    events.push(record(9, "lead", SPAWN_ACTION, "", ""));
    assert_eq!(chain_of(&events), None);
    let mut events = hop(0, LIMIT_ACTION, "origin", "substitute");
    events.extend(hop(10, RETURN_ACTION, "substitute", "origin"));
    assert_eq!(chain_of(&events), None, "home again");
    assert_eq!(returns_since(&events, SESSION, SLOT, AGENT, T0), 1);
    assert_eq!(returns_since(&events, SESSION, SLOT, AGENT, T0 + 14), 0);
    assert!(
        !left_profiles(&events, SESSION, SLOT, AGENT)
            .iter()
            .any(|(profile, _)| profile == "substitute"),
        "a return leaves nothing behind"
    );
    events.extend(hop(20, LIMIT_ACTION, "origin", "substitute"));
    let found = chain_of(&events).expect("a new chain");
    assert_eq!((found.origin.as_str(), found.away_at), ("origin", at(23)));
    assert_eq!(returns_since(&events, SESSION, SLOT, AGENT, T0), 1);
    events.push(record(30, "lead", SPAWN_ACTION, "", ""));
    assert_eq!(returns_since(&events, SESSION, SLOT, AGENT, T0), 0);
}

#[test]
fn the_return_episode_opens_on_its_record_and_ends_on_a_move_a_choice_or_an_opener() {
    let base = hop(0, LIMIT_ACTION, "origin", "substitute");
    let opener = record(
        10,
        WATCHDOG_ACTOR,
        RETURN_ACTION,
        "",
        "return to origin: 20%",
    );
    let mut events = base.clone();
    events.push(opener.clone());
    let found = return_episode(&events, SESSION, SLOT, AGENT).expect("open");
    assert_eq!((found.trigger, found.key), (Trigger::Return, at(10)));
    assert_eq!(found.cause(), "return to origin: 20%");
    for ender in [
        reseat(11, "human", "already using substitute"),
        record(11, "lead", SPAWN_ACTION, "", ""),
        record(11, WATCHDOG_ACTOR, HEADROOM_ACTION, "", "headroom 95%"),
        record(11, WATCHDOG_ACTOR, DONE_ACTION, "substitute", "to origin"),
    ] {
        let mut ended = events.clone();
        ended.push(ender.clone());
        assert_eq!(
            return_episode(&ended, SESSION, SLOT, AGENT),
            None,
            "{}",
            ender.action
        );
    }
    // An opener does not end a return whose attempt is in flight.
    let key = at(10).to_string();
    events.push(record(12, WATCHDOG_ACTOR, ATTEMPT_ACTION, &key, "attempt"));
    events.push(record(
        13,
        WATCHDOG_ACTOR,
        HEADROOM_ACTION,
        "",
        "headroom 95%",
    ));
    let flying = return_episode(&events, SESSION, SLOT, AGENT).expect("still in flight");
    assert_eq!(flying.open, Some(at(12)));
    assert_eq!(
        keyed(&events, (SESSION, SLOT, AGENT), at(10)).map(|found| found.trigger),
        Some(Trigger::Return)
    );
}

#[test]
fn a_refused_return_round_cools_like_a_limit_round_and_gives_up_on_its_third() {
    let mut events = hop(0, LIMIT_ACTION, "origin", "substitute");
    events.push(record(10, WATCHDOG_ACTOR, RETURN_ACTION, "", "return"));
    let key = at(10).to_string();
    let mut since = 11;
    for round in 1..=MAX_ROUNDS {
        events.push(record(
            since,
            WATCHDOG_ACTOR,
            ATTEMPT_ACTION,
            &key,
            "attempt",
        ));
        events.push(record(
            since + 1,
            WATCHDOG_ACTOR,
            REFUSED_ACTION,
            &key,
            "pair-provider: x",
        ));
        let found = return_episode(&events, SESSION, SLOT, AGENT).expect("closed, kept");
        assert_eq!(found.rounds, round);
        assert_eq!(found.terminal, Some(Outcome::Refused));
        let closed = T0 + since + 1;
        assert!(!found.cooled(closed + RETRY_COOLDOWN_SECS - 1));
        assert_eq!(
            found.cooled(closed + RETRY_COOLDOWN_SECS),
            round < MAX_ROUNDS
        );
        assert_eq!(found.gave_up(), round == MAX_ROUNDS);
        since += RETRY_COOLDOWN_SECS + 10;
    }
}

#[test]
fn a_return_holds_an_unproven_frame_and_the_new_class_is_a_transient_hold() {
    let mut events = hop(0, LIMIT_ACTION, "origin", "substitute");
    events.push(record(10, WATCHDOG_ACTOR, RETURN_ACTION, "", "return"));
    let found = return_episode(&events, SESSION, SLOT, AGENT);
    let pane = Pane {
        frame: Frame::Unproven,
        human_prompt: false,
        client_input: None,
    };
    assert_eq!(
        decide(found.as_ref(), 0, &pane, T0 + 20),
        Decision::Hold(HoldReason::Unproven)
    );
    assert_eq!(CauseClass::ReturnChanged.word(), "return-changed");
    assert!(CauseClass::ReturnChanged.transient());
}

fn window(judged: f64, long: bool, observed_at: i64, status: Status) -> Window {
    Window {
        judged,
        critical: false,
        long,
        observed_at,
        resets_at: Some(T0 + 86_400),
        status,
        label: "w".to_owned(),
    }
}

fn fresh(judged: f64, long: bool) -> Sample {
    Sample::Judged(window(judged, long, T0 + 100, Status::Fresh))
}

const AWAY: i64 = T0 + 50;

#[test]
fn every_window_must_read_fresh_after_the_move_and_strictly_below_the_band() {
    let band = return_band(Some(95));
    assert!((band - 90.0).abs() < f64::EPSILON);
    assert!(
        (return_band(None) - 90.0).abs() < f64::EPSILON,
        "off reads the default"
    );
    assert!((return_band(Some(80)) - 75.0).abs() < f64::EPSILON);
    let ready = return_ready(
        &[("7d", fresh(20.0, true)), ("5h", fresh(10.0, false))],
        AWAY,
        band,
    );
    assert_eq!(ready.map(|found| found.judged), Some(20.0));
    for (why, samples) in [
        ("at the band", vec![("7d", fresh(90.0, true))]),
        (
            "one window over",
            vec![("7d", fresh(20.0, true)), ("5h", fresh(95.0, false))],
        ),
        ("no long window", vec![("5h", fresh(0.0, false))]),
        ("nothing read", vec![]),
        (
            "stale",
            vec![(
                "7d",
                Sample::Judged(window(1.0, true, T0 + 100, Status::Stale)),
            )],
        ),
        (
            "reset passed",
            vec![(
                "7d",
                Sample::Judged(window(1.0, true, T0 + 100, Status::Unknown)),
            )],
        ),
        (
            "read at the move",
            vec![("7d", Sample::Judged(window(1.0, true, AWAY, Status::Fresh)))],
        ),
        (
            "read before the move",
            vec![(
                "7d",
                Sample::Judged(window(1.0, true, AWAY - 1, Status::Fresh)),
            )],
        ),
        (
            "not a number",
            vec![(
                "7d",
                Sample::Judged(window(f64::NAN, true, T0 + 100, Status::Fresh)),
            )],
        ),
    ] {
        assert_eq!(return_ready(&samples, AWAY, band), None, "{why}");
    }
}

#[test]
fn the_newest_reading_of_each_window_rules_and_nothing_unjudged_is_dropped() {
    let band = 90.0;
    let older = |judged| Sample::Judged(window(judged, true, T0 + 60, Status::Fresh));
    assert!(
        return_ready(
            &[("7d", older(99.0)), ("7d", fresh(20.0, true))],
            AWAY,
            band
        )
        .is_some(),
        "a newer low reading supersedes an older high one"
    );
    for (why, samples) in [
        (
            "newer high",
            vec![("7d", older(20.0)), ("7d", fresh(95.0, true))],
        ),
        (
            "tie opposed",
            vec![("7d", fresh(20.0, true)), ("7d", fresh(95.0, true))],
        ),
        (
            "tie unjudged",
            vec![
                ("7d", fresh(20.0, true)),
                ("7d", Sample::Unjudged(Some(T0 + 100))),
            ],
        ),
        (
            "newer unknown",
            vec![
                ("7d", older(20.0)),
                (
                    "7d",
                    Sample::Judged(window(1.0, true, T0 + 100, Status::Unknown)),
                ),
            ],
        ),
        (
            "newer unjudged",
            vec![
                ("7d", older(20.0)),
                ("7d", Sample::Unjudged(Some(T0 + 100))),
            ],
        ),
        (
            "unstamped",
            vec![("7d", fresh(20.0, true)), ("7d", Sample::Unjudged(None))],
        ),
        (
            "an unjudged other window",
            vec![
                ("7d", fresh(20.0, true)),
                ("x", Sample::Unjudged(Some(T0 + 100))),
            ],
        ),
    ] {
        assert_eq!(return_ready(&samples, AWAY, band), None, "{why}");
    }
    // An older unjudged reading a newer judged one supersedes does not deny.
    let superseded = [
        ("7d", Sample::Unjudged(Some(T0 + 60))),
        ("7d", fresh(20.0, true)),
    ];
    assert!(return_ready(&superseded, AWAY, band).is_some());
}

#[test]
fn a_reseat_record_is_the_seats_only_by_its_own_name_and_slot_with_no_session() {
    let other = |line: &str| {
        Event::parse_line(&format!(
            r#"{{"ts":"{}","actor":"human","action":"reseat",{line},"summary":"reseated, seed submitted"}}"#,
            at(9)
        ))
        .expect("a well-formed specimen")
    };
    for (stranger, why) in [
        (
            other(r#""target":"other","target_slot":"spawned.3""#),
            "another agent",
        ),
        (
            other(r#""target":"scout","target_slot":"spawned.9""#),
            "another slot",
        ),
        (
            other(r#""target":"scout","target_slot":"spawned.3","target_session":"elsewhere""#),
            "another session's routed seat",
        ),
        (
            other(r#""target":"scout","target_slot":"spawned.3","target_session":"""#),
            "half a routing key",
        ),
        (
            other(r#""target":"scout","target_slot":"spawned.3","target_session_id":"x""#),
            "a session id without its session",
        ),
        (
            record(9, "human", "relaunch", "", "relaunched"),
            "not a reseat",
        ),
    ] {
        let mut events = hop(0, HEADROOM_ACTION, "origin", "substitute");
        events.push(stranger);
        assert_eq!(
            origin(chain_of(&events)).as_deref(),
            Some("origin"),
            "{why}"
        );
    }
    let mut events = hop(0, HEADROOM_ACTION, "origin", "substitute");
    events.push(reseat(9, "human", "reseated, seed submitted"));
    assert_eq!(chain_of(&events), None, "the seat's own unrouted reseat");
}

/// The path's records are the watchdog's alone: an `auto-reseat` anyone else
/// journals never decides which kind of move the watchdog's next hop was.
#[test]
fn an_attempt_anyone_but_the_watchdog_journals_never_relabels_a_hop() {
    let mut events = hop(0, HEADROOM_ACTION, "origin", "substitute");
    let mut back = hop(10, RETURN_ACTION, "substitute", "origin");
    let done = back.pop().expect("the done");
    let moved = back.pop().expect("the moved reseat");
    events.extend(back);
    events.push(record(12, "lead", ATTEMPT_ACTION, "", "forged"));
    events.push(moved);
    events.push(done);
    assert_eq!(chain_of(&events), None, "the return hop ended the chain");
    assert_eq!(returns_since(&events, SESSION, SLOT, AGENT, T0), 1);
}

/// The cause names the WORST long window of the origin: the one with the
/// highest judged percentage, whatever order the readings come in.
#[test]
fn the_return_names_the_worst_long_window_in_either_order() {
    let band = return_band(Some(95));
    let labelled = |judged: f64, label: &str| {
        let mut found = window(judged, true, T0 + 100, Status::Fresh);
        label.clone_into(&mut found.label);
        Sample::Judged(found)
    };
    for samples in [
        vec![("7d", labelled(10.0, "7d")), ("30d", labelled(30.0, "30d"))],
        vec![("30d", labelled(30.0, "30d")), ("7d", labelled(10.0, "7d"))],
    ] {
        let found = return_ready(&samples, AWAY, band).expect("both fresh and low");
        assert_eq!((found.judged, found.label.as_str()), (30.0, "30d"));
    }
}

/// A return is in flight from its attempt until its outcome, for at most
/// [`IN_FLIGHT_SECS`], and only while auto reseat is on; a limit or headroom
/// attempt is not one.
#[test]
fn a_return_is_in_flight_only_between_its_attempt_and_outcome_within_the_bound() {
    let on = Settings {
        switch: Switch::On,
        sessions: None,
        grace_secs: 0,
        map: Vec::new(),
        headroom_at: None,
        headroom_note: None,
        lead: crate::autoreseat::Lead::default(),
        notes: Vec::new(),
    };
    let off = Settings {
        switch: Switch::Off,
        ..on.clone()
    };
    let seat = (SESSION, SLOT, AGENT);
    let key = at(10).to_string();
    let opener = record(10, WATCHDOG_ACTOR, RETURN_ACTION, "", "return");
    let attempt = record(11, WATCHDOG_ACTOR, ATTEMPT_ACTION, &key, "attempt");
    let flying = vec![opener.clone(), attempt.clone()];
    let started = T0 + 11;
    assert!(return_in_flight(&on, &flying, seat, started + 1));
    assert!(return_in_flight(
        &on,
        &flying,
        seat,
        started + IN_FLIGHT_SECS - 1
    ));
    assert!(
        !return_in_flight(&on, &flying, seat, started + IN_FLIGHT_SECS),
        "past its bound an attempt is overdue, not in flight"
    );
    assert!(!return_in_flight(&off, &flying, seat, started + 1), "off");
    assert!(
        !return_in_flight(&on, std::slice::from_ref(&opener), seat, started + 1),
        "opened, no attempt"
    );
    assert!(!return_in_flight(&on, &[], seat, started + 1), "no episode");
    let mut ended = flying.clone();
    ended.push(record(12, WATCHDOG_ACTOR, REFUSED_ACTION, &key, "refused"));
    assert!(
        !return_in_flight(&on, &ended, seat, started + 1),
        "an outcome"
    );
    let limit = vec![
        record(10, WATCHDOG_ACTOR, LIMIT_ACTION, "", "limit"),
        record(11, WATCHDOG_ACTOR, ATTEMPT_ACTION, &key, "attempt"),
    ];
    assert!(
        !return_in_flight(&on, &limit, seat, started + 1),
        "a limit attempt is not a return"
    );
}
