//! #250 O3 phase 2 R13: additive ordinary-chat schema; origin is audit only.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private persisted state"
)]
use ae::events::Event;
const BASE: &str = r#"{"ts":"2026-10-25T09:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"o3-origin-fixture""#;
const ORIGIN: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";

#[test]
fn r13_legacy_and_future_unknown_keys_remain_ordinary_chat() {
    let fixture = include_str!("../fixtures/console/relay-origin.jsonl");
    assert_eq!(
        Event::parse_line(fixture.trim())
            .expect("writer fixture")
            .actor,
        "console:local"
    );
    let event =
        Event::parse_line(&format!("{BASE},\"future_o3_key\":5}}")).expect("unknown future field");
    assert_eq!(event.action, "ask");
    assert_eq!(event.reference.as_deref(), Some("o3-origin-fixture"));
}

#[test]
fn r13_origin_is_retained_strictly_instead_of_ignored_or_authoritative() {
    let legacy = Event::parse_line(&format!("{BASE}}}")).expect("legacy");
    let valid =
        Event::parse_line(&format!("{BASE},\"relay_origin\":\"{ORIGIN}\"}}")).expect("valid UUID");
    assert_ne!(valid, legacy, "origin retained for audit");
    for bad in ["null", "5", "true", "[]", "{}"] {
        assert!(
            Event::parse_line(&format!("{BASE},\"relay_origin\":{bad}}}")).is_err(),
            "bad origin {bad}"
        );
    }
    let duplicate = format!("{BASE},\"relay_origin\":\"{ORIGIN}\",\"relay_origin\":\"{ORIGIN}\"}}");
    assert!(
        Event::parse_line(&duplicate).is_err(),
        "duplicate known origin refused"
    );
    assert_eq!(
        valid.actor, "console:local",
        "origin does not mint a new speaker"
    );
    for damaged in ["", "not-a-uuid", "../orch"] {
        let parsed = Event::parse_line(&format!("{BASE},\"relay_origin\":\"{damaged}\"}}"))
            .expect("damaged grammar stays an ask");
        assert_eq!(parsed.actor, "console:local");
        assert_eq!(
            ae::console::lane::open_asks(&[parsed], "target").len(),
            1,
            "bad origin cannot erase open ask/cap"
        );
    }
}

// Reader-first: persisted ordinary asks work before any relay producer exists.
#[test]
fn r13_reader_first_badge_reaches_chat_and_actual_app_without_uuid_leak() {
    use super::o2guard_p2_spec::{World, uuid_of};
    use std::fs;
    let owner = super::app_writer_spec::Rig::new("o3reader");
    let world = std::mem::ManuallyDrop::new(World {
        root: owner.root.clone(),
        sock: owner.socket.clone(),
    });
    let orch = world.session("orch", "boss", "meta_agent=true\n");
    let uuid = uuid_of("orch");
    let renamed = orch.dir.with_file_name("_renamed");
    let pane = owner.console(false);
    for (step, name) in [
        "orch",
        "_renamed",
        "(name unavailable)",
        "(name unavailable)",
        "(origin unreadable)",
    ]
    .into_iter()
    .enumerate()
    {
        if step == 1 {
            assert!(world.tmux("rename-session -t", &["=orch", "_renamed"]).0);
            fs::rename(&orch.dir, &renamed).expect("rename origin dir");
            let meta = fs::read_to_string(renamed.join("meta")).expect("origin meta");
            fs::write(
                renamed.join("meta"),
                meta.replace("session=orch\n", "session=_renamed\n"),
            )
            .expect("renamed meta");
        }
        if step == 2 {
            let other = world.session("clone", "peer", "");
            let meta = fs::read_to_string(other.dir.join("meta")).expect("clone meta");
            fs::write(
                other.dir.join("meta"),
                meta.replace(&uuid_of("clone"), &uuid),
            )
            .expect("ambiguous UUID");
            assert!(
                world
                    .tmux("set-option -t", &["clone", "@ae_session_uuid", &uuid])
                    .0
            );
        }
        if step == 3 {
            for dir in [&renamed, &world.root.join("sessions/clone")] {
                fs::remove_file(dir.join("meta")).expect("missing origin metadata");
            }
        }
        let origin = if step == 4 { "not-a-uuid" } else { &uuid };
        let row = format!(
            r#"{{"ts":"2026-10-25T09:00:00Z","actor":"console:local","action":"ask","target":"lead","target_slot":"main","target_session":"{}","ref":"reader-badge","summary":"fixture owner instruction","relay_origin":"{origin}"}}"#,
            owner.home
        );
        fs::write(owner.tool.dir.join("events.jsonl"), row + "\n").expect("persisted opening");
        let shown = world.run(None, &["chat", &owner.home], &[]);
        assert_eq!(shown.0, Some(0), "{}", shown.2);
        let badge = format!("via orchestrator {name}");
        assert!(shown.1.contains(&badge), "{}", shown.1);
        let app = owner.wait(&pane, "app origin badge", |s| s.contains(&badge));
        assert!(
            !shown.1.contains(&uuid) && !app.contains(&uuid),
            "origin UUID is audit data"
        );
    }
}
