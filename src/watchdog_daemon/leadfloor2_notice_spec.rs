//! #309 unusable-floor warning: the daemon journals one target-less notice per note.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance owns and removes only its private config and event journal"
)]

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Root(std::path::PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn leadfloor2_daemon_journals_the_unusable_floor_once_without_a_target() {
    let owned = Root(std::env::temp_dir().join(format!(
        "ae-lf2-floor-notice-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir_all(&owned.0).expect("private journal directory");
    let helper = SendHelper::for_session(&owned.0);
    let server = crate::inventory::ServerId::Ambient;
    let cycle = Cycle {
        knobs: Knobs::default(),
        meta_dir: &owned.0,
        helper: &helper,
        server: &server,
        session: "check",
        goal: None,
        roster: Vec::new(),
        local_config: None,
        lead_pair: true,
        fleet_order: crate::theme::FleetOrder::EMPTY,
        meta_agent: false,
        launch_ids: Vec::new(),
        started: None,
        settled: None,
        auto: crate::autoreseat::settings_in(
            &owned.0.join("config"),
            "[workspace]\nauto_reseat = all\nauto_reseat_lead_floor = xyzzy\n",
        ),
        auto_trust: crate::config::AutoTrust::Off,
    };
    let mut noted = Vec::new();
    let mut err = Vec::new();
    cycle
        .auto_note(&mut noted, &mut err)
        .expect("first cycle notice");
    let first = std::fs::read(owned.0.join("events.jsonl")).expect("durable warning");
    let events = read_events(&owned.0);
    assert_eq!(events.len(), 1, "one configured problem gives one notice");
    let event = &events[0];
    assert_eq!(event.actor, "watchdog");
    assert_eq!(event.action, crate::autoreseat::NOTICE_ACTION);
    assert!(event.target.is_none(), "{event:?}");
    assert_eq!(event.target_slot, crate::events::RoutingMember::Absent);
    assert_eq!(event.target_session, crate::events::RoutingMember::Absent);
    assert_eq!(
        event.actor_session,
        crate::events::RoutingMember::Value("check".to_owned())
    );
    let summary = event.summary.as_deref().expect("warning text");
    for required in [
        "auto_reseat_lead_floor",
        "xyzzy",
        "not a whole number",
        "no model change",
    ] {
        assert!(summary.contains(required), "missing {required}: {summary}");
    }
    assert_eq!(noted, [summary]);
    cycle
        .auto_note(&mut noted, &mut err)
        .expect("next cycle notice");
    assert_eq!(
        std::fs::read(owned.0.join("events.jsonl")).expect("journal"),
        first
    );
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
}
