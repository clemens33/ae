//! The block follows the acting seat's own roster entry: with two seats on
//! two clients, the seat that hit the vendor's line blocks its own client
//! and no neighbour's.

#![allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "the fixture applies one booked effect in a scratch session directory"
)]

use super::{Acting, Cycle, Effect, Knobs, PaneState, SendHelper};
use crate::meta::{
    RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RecordedWorkDir, RosterEntry,
};

fn entry(slot: &str, name: &str, label: &str) -> RosterEntry {
    RosterEntry {
        slot: slot.to_owned(),
        name: name.to_owned(),
        profile: None,
        client: RecordedClient::Label(label.to_owned()),
        harness_session: None,
        config_home: RecordedConfigHome::Missing,
        config_home_base: RecordedConfigHomeBase::Missing,
        binary: None,
        work_dir: RecordedWorkDir::Missing,
    }
}

#[test]
fn a_block_is_recorded_for_the_acting_seats_client_and_no_neighbours() {
    let root = std::env::temp_dir().join(format!(
        "ae-vendorblock-roster-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let meta_dir = root.join("sessions/roster");
    std::fs::create_dir_all(&meta_dir).unwrap();
    let helper = SendHelper::for_session(&meta_dir);
    let server = crate::inventory::ServerId::Ambient;
    let auto_file = meta_dir.join("auto");
    let cycle = Cycle {
        knobs: Knobs::default(),
        meta_dir: &meta_dir,
        helper: &helper,
        server: &server,
        session: "roster",
        goal: None,
        // The neighbour comes first: a lookup that skips the acting slot finds it.
        roster: vec![
            entry("spawned.1", "neighbour", "neighbourclient"),
            entry("spawned.0", "muse", "museclient"),
        ],
        local_config: None,
        lead_pair: false,
        fleet_order: crate::theme::FleetOrder::EMPTY,
        meta_agent: false,
        launch_ids: Vec::new(),
        started: None,
        settled: None,
        auto: crate::autoreseat::settings_in(&auto_file, "[workspace]\nauto_reseat = off\n"),
        auto_trust: crate::config::AutoTrust::Off,
    };
    let seen = latch_obs();
    let acting = Acting {
        agent: "muse",
        slot: "spawned.0",
        pane: "%9",
        seen: &seen,
        events: &[],
        overview: None,
    };
    let effect = Effect::BlockScope {
        kind: crate::vendorblock::Kind::Billing,
        tool: crate::tool::ToolKind::Muse,
        reason: "Billing verification failed. Billing Issue".to_owned(),
    };
    let mut state = PaneState::default();
    let mut err = Vec::new();
    cycle.apply(&effect, &acting, &mut state, &mut err).unwrap();
    let err = String::from_utf8_lossy(&err);
    assert!(
        root.join("vendorblock/client-museclient.rec").is_file(),
        "the acting seat's client is blocked: {err}"
    );
    assert!(
        !root.join("vendorblock/client-neighbourclient.rec").exists(),
        "a neighbour's client is not blocked: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn latch_obs() -> super::Observation {
    super::Observation {
        now_epoch: 10_000,
        hash: 7,
        harness: super::HarnessObservation {
            frame: crate::harness_state::HarnessState::Unknown,
            human_draft: false,
            durable_stale: false,
            declaration: None,
        },
        identity: 1,
        is_dead: false,
        throttle: None,
        limit_notice: None,
        limit_reset: None,
        limit_woke_journal: None,
        auto_continue: false,
        capture_ok: true,
        human_prompt: None,
        throttle_quota: None,
        quiet: None,
        ended_wait: None,
        done_progress: crate::watchdog::DoneProgress::None,
        wait_progress: crate::watchdog::WaitProgress::None,
        wait_rung: 0,
        wait_ends_elsewhere: false,
        target_moved: None,
        descendancy: crate::procs::Descendancy::Present,
        launch_attempt: crate::tmux::Evidence::Silent,
        outstanding_alert: None,
        unreachable: false,
        failed_elsewhere: false,
        prompt_named: false,
        start: None,
        last_actor_event_age_secs: 0,
        declared_age_secs: 0,
        suspended: crate::watchdog::Suspensions::default(),
        sweep: None,
        own_work: crate::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}
