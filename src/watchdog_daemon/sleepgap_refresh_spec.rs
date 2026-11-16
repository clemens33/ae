//! Independent #238 regression: a held limit-release refresh survives to the free cycle.
//! This spec defines the new `Carry::refresh_owed` retained-request contract.
//! The socket fixture is shared with `transport::quietwd_spec::Rig`.

use super::{Carry, Cycle, Knobs, PaneState, SendHelper};
use crate::time::Timestamp;
use crate::watchdog::CycleClock;
use std::time::Instant;

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the fixture creates only its private journal directory"
)]
fn sleepgap_limit_release_refresh_is_retained_until_the_first_free_cycle() {
    let rig = crate::transport::quietwd_spec::Rig::new("sleepgap-refresh");
    let meta = rig.path("meta");
    std::fs::create_dir(&meta).expect("owned journal directory");
    let helper = SendHelper::for_session(&meta);
    // The absent helper is an external failure oracle: every attempted send
    // increments undelivered_streak, without typing into any pane.
    let cycle = Cycle {
        knobs: Knobs {
            quota_aware: false,
            quota_every_secs: 0,
            ..Knobs::default()
        },
        meta_dir: &meta,
        helper: &helper,
        server: &rig.server,
        session: "current",
        goal: None,
        roster: Vec::new(),
        local_config: None,
        lead_pair: false,
        fleet_order: crate::theme::FleetOrder::EMPTY,
        auto: crate::autoreseat::settings(None),
        auto_trust: crate::config::AutoTrust::Off,
        meta_agent: false,
        launch_ids: Vec::new(),
        started: None,
        settled: None,
    };
    let now = Timestamp::now().epoch();
    let panes =
        crate::transport::observe_watch_panes(&rig.server, "current").expect("private panes");
    let pane = panes
        .iter()
        .find(|pane| pane.pane_id == rig.pane)
        .expect("owned seat");
    let slot = pane.slot.as_deref().unwrap_or_default();
    let agent = pane.agent.as_deref().expect("fixture agent stamp");
    let capture = crate::transport::capture_pane(&rig.server, &rig.pane).expect("owned frame");
    let mut carry = Carry::new();
    carry.clock = Some(CycleClock {
        wall: now - 7_200,
        mono: Instant::now(),
    });
    carry.panes.push((
        rig.pane.clone(),
        PaneState {
            identity: Some(crate::watchdog::quiet_hash(&format!("{slot}\n{agent}"))),
            prev_hash: Some(crate::watchdog::quiet_hash(&capture)),
            last_hash_change: Some(now - 10_000),
            idle_since_epoch: Some(now - 10_000),
            limit_since: Some(now - 10_000),
            nudge_count: 1,
            undelivered_streak: 1,
            ..PaneState::default()
        },
    ));
    let mut err = Vec::new();
    for (pass, expected_attempts, expected_owed) in [(0, 1, true), (1, 1, true), (2, 2, false)] {
        cycle.run(&mut carry, &mut err).expect("private real cycle");
        let (_, state) = carry
            .panes
            .iter()
            .find(|(pane, _)| pane == &rig.pane)
            .expect("carried pane");
        assert_eq!(
            state.undelivered_streak,
            expected_attempts,
            "pass={pass}, err={}",
            String::from_utf8_lossy(&err)
        );
        assert_eq!(state.nudge_count, 1, "no budget reset or fake success");
        assert_eq!(state.limit_since, None, "the release is one-shot");
        assert_eq!(
            carry.refresh_owed, expected_owed,
            "pass={pass}: a held request must survive its source latch"
        );
    }
    let events = super::read_events(&meta);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.action == "alert-cleared"
                && event.summary.as_deref()
                    == Some("usage limit cleared — pane no longer shows it"))
            .count(),
        1,
        "one release supplies the deferred refresh"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.action == "host-suspended")
            .count(),
        1
    );
}
