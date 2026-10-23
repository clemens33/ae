//! Frozen P2 acceptance: c98ebc3f ownership/phase2, NIT1, plan r3.
//! Literal records, expected argv and journal traces are independent oracles.

use std::path::Path;

use ae::config::{AutoEffort, auto_effort_in, global_auto_effort};
use ae::effort::{
    Damage, Direction, DiscardSeat, Discarded, Episode, Fold, Foreign, Kept, Leg, Level, Record,
    Resume, State, Stored, Switch, Transition, discard_line, fold, on_resume,
};
use ae::events::{AlertMeaning, Event, SessionKey};
use ae::time::Timestamp;

pub(super) const SID: &str = "11111111-1111-4111-8111-111111111111";
pub(super) const CONV: &str = "22222222-2222-4222-8222-222222222222";
const EP: i64 = 1_700_000_000;
pub(super) const VALID: &str = "v1 applied slot:main base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:tok-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:custom client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700000100";

fn record() -> Record {
    Record::parse(VALID).unwrap_or_else(|why| panic!("independent P1-valid record: {why:?}"))
}

fn transition(direction: Direction) -> Transition {
    let (from, to) = match direction {
        Direction::Down => (Level::Xhigh, Level::High),
        Direction::Up => (Level::High, Level::Xhigh),
    };
    Transition {
        direction,
        from,
        to,
    }
}

fn episode() -> Episode {
    Episode {
        key: EP,
        threshold: 80,
        down: Leg::Done(transition(Direction::Down)),
        up: Leg::Unspent,
        named: vec![],
        yielded: false,
    }
}

pub(super) fn journal_line(action: &str, reference: &str, summary: &str) -> String {
    format!(
        r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"{action}","target":"lead","target_slot":"main","target_session":"fixture","target_session_id":"{SID}","ref":"{reference}","summary":"{summary}"}}
"#
    )
}

pub(super) fn applied_journal() -> String {
    [
        journal_line("effort-episode", "1700000000", "threshold 80"),
        journal_line("effort-reserved", "1700000000-down", "xhigh->high"),
        journal_line("effort-applied", "1700000000-down", "xhigh->high"),
    ]
    .concat()
}

fn event(action: &str, reference: &str, summary: &str) -> Event {
    Event::parse_line(journal_line(action, reference, summary).trim())
        .unwrap_or_else(|why| panic!("literal journal fixture: {why}"))
}

fn folded(events: &[Event]) -> Fold {
    fold(events, SessionKey::new("renamed", SID), "main", "lead")
}

#[test]
fn effort_resume_literal_fixtures_satisfy_the_existing_p1_contract() {
    let trace: Vec<_> = applied_journal()
        .lines()
        .map(|line| Event::parse_line(line).expect("literal trace"))
        .collect();
    assert_eq!(folded(&trace), Fold::Open(episode()));
    assert_eq!(
        ae::effort::stored(format!("effort.main={VALID}\n").as_bytes(), "main"),
        Stored::Present(record())
    );
    for (text, damage) in [
        ("effort.main\n", Damage::Bare),
        ("effort.main=\n", Damage::Version),
        ("effort.main=☃\n", Damage::Encoding),
    ] {
        assert_eq!(
            ae::effort::stored(text.as_bytes(), "main"),
            Stored::Damaged(damage)
        );
    }
}

#[test]
fn effort_resume_absence_precedes_every_other_fact() {
    for switch in [Switch::On, Switch::Off, Switch::Unusable] {
        for retained in [false, true] {
            assert_eq!(
                on_resume(
                    &Stored::Absent,
                    Err(Foreign::Conversation),
                    None,
                    switch,
                    retained
                ),
                Resume::None
            );
        }
    }
}

#[test]
fn effort_resume_fresh_and_off_discard_even_pending_or_damaged() {
    for state in [State::Applied, State::PendingDown, State::PendingUp] {
        let mut row = record();
        row.state = state;
        for switch in [Switch::On, Switch::Off, Switch::Unusable] {
            assert_eq!(
                on_resume(
                    &Stored::Present(row.clone()),
                    Err(Foreign::Session),
                    None,
                    switch,
                    false
                ),
                Resume::Discard {
                    episode: Some(EP),
                    why: Discarded::Fresh
                }
            );
        }
        assert_eq!(
            on_resume(
                &Stored::Present(row),
                Err(Foreign::Session),
                None,
                Switch::Off,
                true
            ),
            Resume::Discard {
                episode: Some(EP),
                why: Discarded::Off
            }
        );
    }
    for damage in [
        Damage::Bare,
        Damage::Duplicated,
        Damage::Encoding,
        Damage::Version,
    ] {
        assert_eq!(
            on_resume(
                &Stored::Damaged(damage),
                Err(Foreign::Session),
                None,
                Switch::On,
                false
            ),
            Resume::Discard {
                episode: None,
                why: Discarded::Fresh
            }
        );
        assert_eq!(
            on_resume(
                &Stored::Damaged(damage),
                Err(Foreign::Session),
                None,
                Switch::Off,
                true
            ),
            Resume::Discard {
                episode: None,
                why: Discarded::Off
            }
        );
    }
}

#[test]
fn effort_resume_unusable_and_damage_never_replay() {
    let ledger = Fold::Open(episode());
    for state in [State::Applied, State::PendingDown, State::PendingUp] {
        let mut row = record();
        row.state = state;
        assert_eq!(
            on_resume(
                &Stored::Present(row),
                Ok(()),
                Some(&ledger),
                Switch::Unusable,
                true
            ),
            Resume::Keep(Kept::Unusable),
            "{state:?}"
        );
    }
    for damage in [
        Damage::Bare,
        Damage::Duplicated,
        Damage::Encoding,
        Damage::Shape,
        Damage::Incoherent,
    ] {
        assert_eq!(
            on_resume(
                &Stored::Damaged(damage),
                Ok(()),
                Some(&ledger),
                Switch::On,
                true
            ),
            Resume::Keep(Kept::Damaged(damage))
        );
        assert_eq!(
            on_resume(
                &Stored::Damaged(damage),
                Ok(()),
                Some(&ledger),
                Switch::Unusable,
                true
            ),
            Resume::Keep(Kept::Unusable)
        );
    }
}

#[test]
fn effort_resume_foreign_incarnation_keeps_and_new_conversation_discards() {
    for state in [State::Applied, State::PendingDown, State::PendingUp] {
        let mut row = record();
        row.state = state;
        for foreign in [
            Foreign::Session,
            Foreign::Slot,
            Foreign::Agent,
            Foreign::Tool,
        ] {
            assert_eq!(
                on_resume(
                    &Stored::Present(row.clone()),
                    Err(foreign),
                    None,
                    Switch::On,
                    true
                ),
                Resume::Keep(Kept::Foreign(foreign))
            );
        }
        assert_eq!(
            on_resume(
                &Stored::Present(row),
                Err(Foreign::Conversation),
                None,
                Switch::On,
                true
            ),
            Resume::Discard {
                episode: Some(EP),
                why: Discarded::Fresh
            }
        );
    }
}

#[test]
fn effort_resume_pending_refuses_before_baseline_or_ledger_checks() {
    for state in [State::PendingDown, State::PendingUp] {
        let mut row = record();
        row.state = state;
        for guard in [
            Ok(()),
            Err(Foreign::Profile),
            Err(Foreign::Client),
            Err(Foreign::Model),
            Err(Foreign::Baseline),
        ] {
            assert_eq!(
                on_resume(&Stored::Present(row.clone()), guard, None, Switch::On, true),
                Resume::Refuse(state)
            );
        }
    }
}

#[test]
fn effort_resume_applied_profile_client_model_or_baseline_edit_keeps() {
    for foreign in [
        Foreign::Profile,
        Foreign::Client,
        Foreign::Model,
        Foreign::Baseline,
    ] {
        assert_eq!(
            on_resume(
                &Stored::Present(record()),
                Err(foreign),
                Some(&Fold::Open(episode())),
                Switch::On,
                true
            ),
            Resume::Keep(Kept::Foreign(foreign))
        );
    }
}

#[test]
fn effort_resume_applied_and_applied_before_outcome_replay_one_tier() {
    for (base, low) in [(Level::Xhigh, Level::High), (Level::Max, Level::Xhigh)] {
        for reserved in [false, true] {
            let mut row = record();
            row.base = base;
            row.low = low;
            let mut ep = episode();
            let down = Transition {
                direction: Direction::Down,
                from: base,
                to: low,
            };
            ep.down = if reserved {
                Leg::Reserved(down)
            } else {
                Leg::Done(down)
            };
            for up in [
                Leg::Unspent,
                Leg::Reserved(Transition {
                    direction: Direction::Up,
                    from: low,
                    to: base,
                }),
                Leg::Failed(Transition {
                    direction: Direction::Up,
                    from: low,
                    to: base,
                }),
            ] {
                ep.up = up;
                // An original PID/launch is provenance, not the retained resume's identity.
                row.pid = 99;
                row.launch = "old-launch".into();
                assert_eq!(
                    on_resume(
                        &Stored::Present(row.clone()),
                        Ok(()),
                        Some(&Fold::Open(ep.clone())),
                        Switch::On,
                        true
                    ),
                    Resume::Replay(low)
                );
            }
        }
    }
}

#[test]
fn effort_resume_positive_manual_yield_wins_and_does_not_rearm() {
    let mut ep = episode();
    ep.yielded = true;
    for down in [Leg::Unspent, Leg::Done(transition(Direction::Down))] {
        ep.down = down;
        assert_eq!(
            on_resume(
                &Stored::Present(record()),
                Ok(()),
                Some(&Fold::Open(ep.clone())),
                Switch::On,
                true
            ),
            Resume::Keep(Kept::Yielded)
        );
    }
    ep.key += 1;
    assert_eq!(
        on_resume(
            &Stored::Present(record()),
            Ok(()),
            Some(&Fold::Open(ep)),
            Switch::On,
            true
        ),
        Resume::Keep(Kept::Ledger)
    );
}

#[test]
fn effort_resume_uncertain_or_disagreeing_ledger_never_replays() {
    let mut cases = vec![
        Fold::Armed,
        Fold::Doubt("damaged"),
        Fold::Open({
            let mut e = episode();
            e.key += 1;
            e
        }),
        Fold::Open({
            let mut e = episode();
            e.threshold = 81;
            e
        }),
    ];
    for down in [
        Leg::Unspent,
        Leg::Failed(transition(Direction::Down)),
        Leg::Done(Transition {
            direction: Direction::Down,
            from: Level::Max,
            to: Level::Xhigh,
        }),
    ] {
        let mut ep = episode();
        ep.down = down;
        cases.push(Fold::Open(ep));
    }
    let mut restored = episode();
    restored.up = Leg::Done(transition(Direction::Up));
    cases.push(Fold::Open(restored));
    assert_eq!(
        on_resume(&Stored::Present(record()), Ok(()), None, Switch::On, true),
        Resume::Keep(Kept::Ledger)
    );
    for ledger in cases {
        assert_eq!(
            on_resume(
                &Stored::Present(record()),
                Ok(()),
                Some(&ledger),
                Switch::On,
                true
            ),
            Resume::Keep(Kept::Ledger),
            "{ledger:?}"
        );
    }
}

#[test]
fn effort_resume_discard_resets_open_closed_terminal_and_doubt() {
    let base: Vec<Event> = applied_journal()
        .lines()
        .map(|line| Event::parse_line(line).expect("literal applied trace"))
        .collect();
    let mut failed_up = base.clone();
    failed_up.push(event("effort-reserved", "1700000000-up", "high->xhigh"));
    failed_up.push(event("effort-failed", "1700000000-up", "high->xhigh"));
    let mut yielded = base.clone();
    yielded.push(event("effort-yielded", "1700000000", "manual-effort"));
    let mut closed = yielded.clone();
    closed.push(event("effort-rearmed", "1700000000", ""));
    for prior in [
        base,
        failed_up,
        yielded,
        closed,
        vec![event("effort-future", "1700000000", "unknown")],
    ] {
        for reference in ["", "1700000000"] {
            for why in ["off", "fresh"] {
                let mut trace = prior.clone();
                trace.push(event("effort-discarded", reference, why));
                assert_eq!(folded(&trace), Fold::Armed, "{reference}/{why}");
                trace.push(event("effort-discarded", reference, why));
                assert_eq!(folded(&trace), Fold::Armed, "retry is idempotent");
                trace.push(event("effort-episode", "1700000000", "threshold 80"));
                assert!(
                    matches!(folded(&trace), Fold::Open(_)),
                    "prior closed key was cleared"
                );
            }
        }
    }
}

#[test]
fn effort_resume_discard_bad_ref_or_summary_is_doubt_and_good_discards_clear_it() {
    for (reference, why) in [
        ("0", "off"),
        ("-1", "off"),
        ("01700000000", "off"),
        ("1700000000-down", "off"),
        ("1700000000", "unknown"),
        ("1700000000", "off; injected"),
        ("", ""),
    ] {
        let mut trace = vec![event("effort-discarded", reference, why)];
        assert!(
            matches!(folded(&trace), Fold::Doubt(_)),
            "{reference:?}/{why:?}"
        );
        trace.push(event("effort-discarded", "", "off"));
        assert_eq!(folded(&trace), Fold::Armed);
    }
}

#[test]
fn effort_resume_discard_foreign_address_or_actor_has_no_effect() {
    let open = event("effort-episode", "1700000000", "threshold 80");
    for (from, to) in [
        ("watchdog", "lead"),
        ("\"target_slot\":\"main\"", "\"target_slot\":\"spawned.1\""),
        (SID, "33333333-3333-4333-8333-333333333333"),
    ] {
        let line = journal_line("effort-discarded", "1700000000", "off").replace(from, to);
        let foreign = Event::parse_line(line.trim()).expect("foreign fixture");
        assert_eq!(
            folded(&[open.clone(), foreign]),
            folded(std::slice::from_ref(&open))
        );
    }
}

#[test]
fn effort_resume_discard_builder_routes_without_paint_or_attention() {
    for (episode, reference) in [(Some(EP), "1700000000"), (None, "")] {
        for (why, summary) in [(Discarded::Off, "off"), (Discarded::Fresh, "fresh")] {
            let line = discard_line(
                Timestamp::from_epoch(EP + 10),
                &DiscardSeat {
                    session: "renamed",
                    session_id: SID,
                    slot: "main",
                    agent: "lead",
                },
                episode,
                why,
            );
            let parsed = Event::parse_line(line.trim());
            assert!(parsed.is_ok(), "one valid journal record: {line:?}");
            let event = parsed.expect("asserted record");
            assert_eq!(event.actor, "watchdog");
            assert_eq!(event.action, "effort-discarded");
            assert_eq!(event.target.as_deref(), Some("lead"));
            assert_eq!(event.target_slot.value(), Some("main"));
            assert_eq!(event.target_session.value(), Some("renamed"));
            assert_eq!(event.target_session_id.value(), Some(SID));
            assert_eq!(event.reference.as_deref().unwrap_or(""), reference);
            assert_eq!(event.summary.as_deref(), Some(summary));
            assert_eq!(event.alert_meaning(), AlertMeaning::Undefined);
            for state in ["done", "waiting-agent", "waiting-user", "blocked"] {
                assert!(!ae::watchdog::ends_quiet(state, &event, false, &[]));
            }
        }
    }
}

#[test]
fn effort_resume_switch_text_has_strict_global_only_grammar() {
    for (text, expected) in [
        ("", AutoEffort::Off),
        ("[workspace]\n", AutoEffort::Off),
        ("[workspace]\nauto_effort = on\n", AutoEffort::On),
        ("[workspace]\nauto_effort = off\n", AutoEffort::Off),
        ("[workspace]\nauto_effort = \"off\"\n", AutoEffort::Off),
        ("[profiles]\nauto_effort = off\n", AutoEffort::Off),
    ] {
        assert_eq!(
            auto_effort_in(Path::new("global"), text),
            expected,
            "{text:?}"
        );
    }
    for text in [
        "[workspace]\nauto_effort = OFF\n",
        "[workspace]\nauto_effort =\n",
        "[workspace]\nauto_effort\n",
        "[workspace]\nauto_effort nonsense\n",
        "[workspace]\nauto_effort = on\nauto_effort = on\n",
        "[workspace]\nauto_effort = off\nauto_effort = on\n",
        "[workspace]\nauto_effort nonsense\nauto_effort = on\n",
        "[workspace]\nauto_effort = on\n[workspace]\nauto_effort = off\n",
    ] {
        assert!(
            matches!(
                auto_effort_in(Path::new("global"), text),
                AutoEffort::Unusable(_)
            ),
            "{text:?}"
        );
    }
}

#[test]
fn effort_resume_switch_global_absence_and_read_error_are_distinct() {
    let scratch = super::cli::OwnedScratch::root("effp2", "switch");
    assert_eq!(global_auto_effort(None), AutoEffort::Off);
    assert_eq!(
        global_auto_effort(Some(&scratch.path().join("missing"))),
        AutoEffort::Off
    );
    assert!(
        matches!(
            global_auto_effort(Some(scratch.path())),
            AutoEffort::Unusable(_)
        ),
        "directory read is doubt"
    );
    let file = scratch.path().join("config");
    std::fs::write(&file, "[workspace]\nauto_effort = off\n").expect("global fixture");
    assert_eq!(global_auto_effort(Some(&file)), AutoEffort::Off);
    std::fs::write(&file, b"[workspace]\nauto_effort=\xff\n").expect("non-UTF8 fixture");
    assert!(matches!(
        global_auto_effort(Some(&file)),
        AutoEffort::Unusable(_)
    ));
}

#[test]
fn effort_resume_legacy_workspace_reader_preserves_last_result_wins() {
    for key in ["auto_trust", "restore", "auto_upgrade", "fleet_order"] {
        assert_eq!(
            ae::config::workspace_key_in(
                Path::new("global"),
                &format!("[workspace]\n{key} nonsense\n{key} = on\n"),
                key
            ),
            Ok(Some("on".into()))
        );
        assert!(
            ae::config::workspace_key_in(
                Path::new("global"),
                &format!("[workspace]\n{key} = on\n{key} nonsense\n"),
                key
            )
            .is_err()
        );
    }
}

#[test]
fn effort_resume_meta_reader_recognizes_optional_family_and_preserves_damage() {
    let meta = ae::meta::Meta::parse(&format!("effort.main={VALID}\n"));
    assert_eq!(meta.effort("main"), Stored::Present(record()));
    assert!(
        meta.anomalies().is_empty(),
        "valid optional family is known"
    );
    assert_eq!(meta.effort("spawned.0"), Stored::Absent);
    for (rows, damage) in [
        ("effort.main\n".into(), Damage::Bare),
        ("effort.main=\n".into(), Damage::Version),
        (
            format!("effort.main={VALID}\neffort.main={VALID}\n"),
            Damage::Duplicated,
        ),
        ("effort.main=☃\n".into(), Damage::Encoding),
    ] {
        assert_eq!(
            ae::meta::Meta::parse(&rows).effort("main"),
            Stored::Damaged(damage),
            "{rows:?}"
        );
    }
}
