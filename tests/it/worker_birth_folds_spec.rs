//! Frozen stats287 s1 test 6: family facts are inert to existing lifecycle folds.
//! Oracle is neutral insertion, plus independent live-spawn and quiet-state witnesses.
#![allow(clippy::expect_used, reason = "frozen fixture records and timestamps")]

use ae::events::{Cursor, Drain, Event, SessionKey};

const SID: &str = "11111111-1111-4111-8111-111111111111";
const BID: &str = "22222222-2222-4222-8222-222222222222";

fn event(line: &str) -> Event {
    Event::parse_line(line).expect("fixture event")
}

fn family(actor: &str, target: &str) -> Vec<Event> {
    let slot = if target == "scout" {
        "spawned.0"
    } else {
        "main"
    };
    ["worker-birth", "worker-fact"]
        .into_iter()
        .map(|action| event(&format!(
            "{{\"ts\":\"2026-11-01T10:00:01Z\",\"actor\":\"{actor}\",\"action\":\"{action}\",\"target\":\"{target}\",\"target_slot\":\"{slot}\",\"target_session_id\":\"{SID}\",\"birth_v\":1,\"birth\":\"{BID}\",\"birth_tool\":\"claude\",\"fact\":\"observed\",\"start\":\"spawned.0:{BID}:41\",\"observed_source\":\"frame\",\"observed_model\":\"sonnet\"}}"
        )))
        .collect()
}

fn ask() -> String {
    format!(
        "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"ask\",\"target\":\"scout\",\"ref\":\"ae-20261101T100000Z-wb287\",\"summary\":\"do work\",\"actor_slot\":\"main\",\"actor_session\":\"s\",\"actor_session_id\":\"{SID}\",\"actor_launch_id\":\"33333333-3333-4333-8333-333333333333\",\"target_slot\":\"spawned.0\",\"target_session\":\"s\",\"target_session_id\":\"{SID}\",\"target_launch_id\":\"{BID}\"}}"
    )
}

fn session(events: Vec<Event>) -> ae::session::SessionRead {
    ae::session::SessionRead::from_drain_in(
        &Drain {
            events,
            cursor: Cursor::default(),
            skipped: vec![],
            drained: true,
        },
        SessionKey::new("s", SID),
    )
}

#[test]
fn family_facts_do_not_open_close_or_refresh_existing_work() {
    let spawn = event(
        r#"{"ts":"2026-11-01T09:59:59Z","actor":"lead","action":"spawn","target":"scout","summary":"task"}"#,
    );
    let base = vec![spawn, event(&ask())];
    let live = vec!["lead".to_owned(), "scout".to_owned()];
    let key = SessionKey::new("s", SID);
    let before = ae::session::Outstanding::read_in(&base, key, &live);
    assert_eq!(
        before.spawns().len(),
        1,
        "a real spawn provides the discriminator"
    );
    let summary = session(base.clone());
    let launched = base[0].ts.epoch() - 1;
    assert_eq!(
        ae::restore::ledger(Some(&summary), launched),
        ae::restore::Ledger::Clear
    );

    for (actor, target) in [("lead", "scout"), ("scout", "lead"), ("scout", "scout")] {
        let mut changed = base.clone();
        changed.extend(family(actor, target));
        let after = ae::session::Outstanding::read_in(&changed, key, &live);
        assert_eq!(after.spawns(), before.spawns());
        let changed_summary = session(changed.clone());
        assert_eq!(changed_summary.goal_set_at(), summary.goal_set_at());
        assert_eq!(changed_summary.pending, summary.pending);
        assert_eq!(changed_summary.last_active, summary.last_active);
        assert_eq!(
            ae::effort::fold(&changed, key, "spawned.0", "scout"),
            ae::effort::Fold::Armed
        );
        assert_eq!(
            ae::autoreseat::episode(&changed, "s", "spawned.0", "scout"),
            ae::autoreseat::episode(&base, "s", "spawned.0", "scout")
        );
        assert_eq!(
            ae::autoreseat::headroom_episode(&changed, "s", "spawned.0", "scout"),
            ae::autoreseat::headroom_episode(&base, "s", "spawned.0", "scout")
        );
        assert_eq!(
            ae::autoreseat::left_on_headroom(&changed, "s", "spawned.0", "scout"),
            ae::autoreseat::left_on_headroom(&base, "s", "spawned.0", "scout")
        );
        assert_eq!(
            ae::autoreseat::spawner(&changed, "s", "spawned.0", "scout"),
            Some("lead")
        );
        assert_eq!(
            ae::restore::ledger(Some(&changed_summary), launched),
            ae::restore::Ledger::Clear
        );
        let seat = ae::session_notice::Seat {
            name: "scout".into(),
            slot: "spawned.0".into(),
            session: "s".into(),
            session_id: SID.into(),
            launch_id: BID.into(),
            launch_time: Some(launched),
            standing: false,
        };
        assert_eq!(
            ae::session_notice::due(&changed, &seat, launched + 10, 30),
            ae::session_notice::due(&base, &seat, launched + 10, 30)
        );
    }
}

#[test]
fn family_facts_after_done_and_wait_do_not_supersede_those_declarations() {
    let now = ae::time::Timestamp::parse("2026-11-01T10:00:02Z").expect("now");
    for (actor, target) in [("lead", "scout"), ("scout", "lead"), ("scout", "scout")] {
        let done = vec![event(
            r#"{"ts":"2026-11-01T10:00:00Z","actor":"scout","action":"state","ref":"done","summary":"finished"}"#,
        )];
        let mut changed = done.clone();
        changed.extend(family(actor, target));
        let expected = ae::watchdog::DoneProgress::Provisional {
            confirmations: 0,
            required: 2,
        };
        assert_eq!(
            ae::watchdog::done_progress(&done, "s", "spawned.0", "scout", None, now, 30, 2),
            expected
        );
        assert_eq!(
            ae::watchdog::done_progress(&changed, "s", "spawned.0", "scout", None, now, 30, 2),
            expected
        );
        for (state, word) in [
            (ae::watchdog::WaitState::WaitingAgent, "waiting-agent"),
            (ae::watchdog::WaitState::Blocked, "blocked"),
        ] {
            let wait = vec![event(&format!(
                "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"scout\",\"action\":\"state\",\"ref\":\"{word}\",\"summary\":\"need a result\"}}"
            ))];
            let mut changed = wait.clone();
            changed.extend(family(actor, target));
            let expected = ae::watchdog::WaitProgress::Provisional {
                confirmations: 0,
                required: 2,
            };
            assert_eq!(
                ae::watchdog::wait_progress(
                    &wait,
                    "s",
                    "spawned.0",
                    "scout",
                    None,
                    now,
                    30,
                    2,
                    state
                ),
                expected
            );
            assert_eq!(
                ae::watchdog::wait_progress(
                    &changed,
                    "s",
                    "spawned.0",
                    "scout",
                    None,
                    now,
                    30,
                    2,
                    state
                ),
                expected
            );
        }
    }
}

#[test]
fn family_facts_do_not_change_the_unique_mirrored_request_opening() {
    let opening = format!("{}\n", ask());
    let base = ae::requests::bound_opening(
        [opening.as_bytes(), opening.as_bytes()],
        "ae-20261101T100000Z-wb287",
        None,
    )
    .expect("one mirrored opening");
    let family_bytes = format!(
        "{{\"ts\":\"2026-11-01T10:00:01Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{SID}\",\"birth_v\":1,\"birth\":\"{BID}\",\"birth_tool\":\"claude\"}}\n"
    );
    let changed = format!("{family_bytes}{opening}{family_bytes}");
    assert_eq!(
        ae::requests::bound_opening(
            [changed.as_bytes(), opening.as_bytes()],
            "ae-20261101T100000Z-wb287",
            None
        )
        .expect("same opening"),
        base
    );
}
