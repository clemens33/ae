//! `[workspace] relay_replies` through the app's own lane loader: the
//! orchestrator panel's chat hides the raw relay answer echo by default and
//! draws it under `relay_replies = on`; the receipt echo stays either way, and
//! a session that holds no orchestrator role keeps a same-shaped row.
#![allow(clippy::expect_used, reason = "bounded acceptance fixtures")]

use std::time::Instant;

use super::{Answer, Reader, SettingsPaths};
use crate::app::tests::{ID, Root, session};

const MARK: &str = "RQUIET-APP-MARK";
const NOTE: &str = "relay — answer echoes hidden";

/// A reader over one session, `orchestrator` or ordinary, whose journal holds
/// the receipt echo, the raw answer echo and another ae row, under a global
/// config that sets `relay_replies` to `setting` (absent when `None`).
fn read(tag: &str, role: bool, setting: Option<&str>) -> (Vec<String>, Vec<String>) {
    let root = Root::new(tag);
    let global = root.0.join("config");
    let config = setting.map_or_else(
        || "[workspace]\nchat = on\n".to_owned(),
        |value| format!("[workspace]\nrelay_replies = {value}\n"),
    );
    std::fs::write(&global, config).expect("global config");
    let dir = session(
        &root,
        "orchestrator",
        if role { "meta_agent=true\n" } else { "" },
    );
    let rows = [
        ("ae:relay", "routed to home: ask (sent), request r-1"),
        ("ae:relay", &format!("reply fd5bf0 from lead: {MARK}")),
        ("ae:compact", "reboot: handover complete"),
    ];
    let journal = rows
        .iter()
        .map(|(actor, text)| {
            format!(
                "{{\"ts\":\"2026-11-19T17:00:00Z\",\"actor\":\"{actor}\",\"action\":\"chat\",\"ref\":\"ae-20261119T170000Z-0000abcd\",\"summary\":\"{text}\"}}\n"
            )
        })
        .collect::<Vec<String>>()
        .concat();
    std::fs::write(dir.join(crate::store::EVENTS), journal).expect("journal");
    let mut reader = Reader::new(root.0.clone(), None, None);
    reader.fleet_at = Some(Instant::now());
    reader.rows = vec!["orchestrator".to_owned()];
    reader.dirs.insert("orchestrator".to_owned(), dir);
    reader.ids.insert("orchestrator".to_owned(), ID.to_owned());
    reader.settings_paths(SettingsPaths {
        home: Some(root.0.join("home")),
        global,
        local: None,
    });
    match reader.view("orchestrator") {
        Some(Answer::View(view)) => (
            view.lane.items.into_iter().map(|item| item.body).collect(),
            view.lane.coverage,
        ),
        _ => panic!("a lane read"),
    }
}

fn has(bodies: &[String], text: &str) -> bool {
    bodies.iter().any(|body| body.contains(text))
}

#[test]
fn the_orchestrator_panel_hides_the_raw_answer_by_default_and_keeps_the_rest() {
    for setting in [None, Some("off"), Some("maybe")] {
        let (bodies, coverage) = read("rquiet-app-default", true, setting);
        assert!(!has(&bodies, MARK), "{setting:?}: {bodies:?}");
        assert!(has(&bodies, "routed to home"), "{setting:?}: {bodies:?}");
        assert!(has(&bodies, "reboot: handover"), "{setting:?}: {bodies:?}");
        assert!(
            coverage.iter().any(|row| row.starts_with(NOTE)),
            "{setting:?}: {coverage:?}"
        );
    }
}

#[test]
fn the_orchestrator_panel_draws_it_under_the_setting_on() {
    let (bodies, coverage) = read("rquiet-app-on", true, Some("on"));
    assert!(has(&bodies, MARK), "{bodies:?}");
    assert!(has(&bodies, "routed to home"), "{bodies:?}");
    assert!(
        !coverage.iter().any(|row| row.contains("answer echoes")),
        "{coverage:?}"
    );
}

#[test]
fn a_session_with_no_orchestrator_role_keeps_a_same_shaped_row_under_both_settings() {
    for setting in [None, Some("on")] {
        let (bodies, coverage) = read("rquiet-app-ordinary", false, setting);
        assert!(has(&bodies, MARK), "{setting:?}: {bodies:?}");
        assert!(
            !coverage.iter().any(|row| row.contains("answer echoes")),
            "{setting:?}: {coverage:?}"
        );
    }
}

/// A reader over the `orchestrator` session, `role` or not, whose journal
/// holds one `say` of its main seat as the real writer records it
/// (`chat_line`: the slot, no session key), under `relay_replies` `setting`.
fn read_with_say(tag: &str, role: bool, setting: Option<&str>) -> (Vec<String>, Vec<String>) {
    let root = Root::new(tag);
    let global = root.0.join("config");
    let config = setting.map_or_else(String::new, |value| {
        format!("[workspace]\nrelay_replies = {value}\n")
    });
    std::fs::write(&global, config).expect("global config");
    let dir = session(
        &root,
        "orchestrator",
        if role { "meta_agent=true\n" } else { "" },
    );
    let said = crate::chat_line(crate::time::Timestamp::now(), "lead", "main", SAY);
    std::fs::write(dir.join(crate::store::EVENTS), format!("{said}\n")).expect("journal");
    let mut reader = Reader::new(root.0.clone(), None, None);
    reader.fleet_at = Some(Instant::now());
    reader.rows = vec!["orchestrator".to_owned()];
    reader.dirs.insert("orchestrator".to_owned(), dir);
    reader.ids.insert("orchestrator".to_owned(), ID.to_owned());
    reader.settings_paths(SettingsPaths {
        home: Some(root.0.join("home")),
        global,
        local: None,
    });
    match reader.view("orchestrator") {
        Some(Answer::View(view)) => (
            view.lane.items.into_iter().map(|item| item.body).collect(),
            view.lane.coverage,
        ),
        _ => panic!("a lane read"),
    }
}

const SAY: &str = "RQUIET-APP-SAY";

#[test]
fn the_orchestrator_panel_draws_its_seats_real_say_under_both_settings() {
    for setting in [None, Some("on"), Some("off")] {
        let (bodies, coverage) = read_with_say("rquiet-app-say", true, setting);
        assert!(has(&bodies, SAY), "{setting:?}: {bodies:?} {coverage:?}");
        assert!(
            !coverage.iter().any(|row| row.contains("other seats")),
            "{setting:?}: {coverage:?}"
        );
    }
}

#[test]
fn an_ordinary_session_reads_a_say_as_it_always_did() {
    // Only an orchestrator's chat recognises the half-keyed say: another
    // session's lane is unchanged, the say still counted as another seat's.
    let (bodies, coverage) = read_with_say("rquiet-app-say-ordinary", false, None);
    assert!(!has(&bodies, SAY), "{bodies:?}");
    assert!(
        coverage.iter().any(|row| row.contains("other seats")),
        "{coverage:?}"
    );
}
