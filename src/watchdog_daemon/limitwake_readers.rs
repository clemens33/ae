use super::{ACTOR, armed_reading, limit_wake_in, reads_reset, waiting_forecast};
use crate::autoreseat::LimitReset;
use crate::events::Event;

fn row(actor: &str, action: &str, target: &str, reference: &str, summary: &str) -> Event {
    Event::parse_line(&format!(
            r#"{{"ts":"2026-11-03T00:00:00Z","actor":"{actor}","action":"{action}","target":"{target}","ref":"{reference}","summary":"{summary}"}}"#
        ))
        .expect("a journal line")
}

const SEAT: (&str, &str, &str) = ("demo", "main", "lead");

#[test]
fn the_spent_reader_takes_the_newest_wake_delivered_or_unconfirmed() {
    let events = [
        row(ACTOR, "limit-wake", "lead", "100", "woke"),
        row(ACTOR, "limit-wake", "lead", "200", "[unconfirmed] woke"),
    ];
    assert_eq!(limit_wake_in(&events, SEAT, false), Some(200));
    assert_eq!(
        limit_wake_in(&events, SEAT, true),
        Some(100),
        "landed skips the unconfirmed row"
    );
}

#[test]
fn the_readers_take_only_the_watchdogs_wakes_to_this_seat_with_an_epoch() {
    let foreign = [
        row("colead", "limit-wake", "lead", "100", "woke"),
        row(ACTOR, "nudge", "lead", "100", "idle"),
        row(ACTOR, "limit-wake", "colead", "100", "woke"),
        row(ACTOR, "limit-wake", "lead", "soon", "woke"),
    ];
    for landed in [false, true] {
        assert_eq!(limit_wake_in(&foreign, SEAT, landed), None, "{landed}");
    }
    assert_eq!(limit_wake_in(&[], SEAT, false), None);
}

/// The cycle reads the seat's OWN reset through its roster row: a seat the
/// roster names gets the held reading's reset, any other slot none.
#[test]
fn the_cycle_reads_the_rostered_seats_own_reset() {
    use super::{Cycle, Knobs, QuotaCarry, SendHelper};
    use crate::meta::{
        RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RecordedWorkDir, RosterEntry,
    };
    use crate::quota::{Group, Observation, Policy, Row, Status};
    let now = 1_700_000_010;
    let entry = RosterEntry {
        slot: "main".into(),
        name: "lead".into(),
        profile: Some("opusx".into()),
        client: RecordedClient::Missing,
        harness_session: None,
        config_home: RecordedConfigHome::Path("/limitwake-cycle-account".into()),
        config_home_base: RecordedConfigHomeBase::Missing,
        binary: Some("claude".into()),
        work_dir: RecordedWorkDir::Missing,
    };
    let row = Row {
        bucket: "session".into(),
        qualifier: None,
        window_minutes: Some(300),
        used_percent: Some("100".into()),
        resets_at: Some(now + 1800),
        observed_at: Some(now),
        status: Status::Fresh,
    };
    let group = Group {
        profiles: vec!["opusx".into()],
        tool: crate::tool::ToolKind::Claude,
        home: None,
        source: Some("/limitwake-cycle-account/.claude.json".into()),
        clients: vec![],
        rollout: None,
        owner: None,
        rows: vec![row],
        hint: None,
        summary: None,
        policy: Policy::default(),
        notes: vec![],
        plan: None,
    };
    let quota = QuotaCarry {
        last_observation: Some(Observation {
            groups: vec![group],
            rendered: vec![],
            home: None,
            now,
        }),
        ..QuotaCarry::default()
    };
    let dir = std::path::Path::new("/limitwake-cycle-meta");
    let helper = SendHelper::for_session(dir);
    let server = crate::inventory::ServerId::Ambient;
    let cycle = Cycle {
        knobs: Knobs::default(),
        meta_dir: dir,
        helper: &helper,
        server: &server,
        session: "demo",
        goal: None,
        roster: vec![entry],
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
    assert_eq!(
        cycle.own_reset(&quota, "main", None, now),
        Some(LimitReset {
            at: now + 1800,
            short: true
        })
    );
    assert_eq!(cycle.own_reset(&quota, "spawned.9", None, now), None);
}

#[test]
fn the_reset_is_read_only_on_the_limit() {
    use crate::watchdog::Throttle;
    assert!(reads_reset(Some(Throttle::LimitReached), false), "drawn");
    assert!(reads_reset(None, true), "latched, notice not read");
    assert!(!reads_reset(None, false));
    assert!(
        !reads_reset(Some(Throttle::Throttled), false),
        "a transient throttle is no limit"
    );
}

#[test]
fn only_a_read_capture_can_show_the_vendor_armed() {
    use crate::tool::ToolKind;
    let rule = "─".repeat(20);
    let armed = format!(
        "{rule}\n❯\n{rule}\n  Continuing automatically at 1am · esc to cancel\n  🧠 Opus\n  ⏵⏵ on\n"
    );
    assert!(armed_reading(true, &armed, ToolKind::Claude));
    assert!(!armed_reading(false, &armed, ToolKind::Claude), "blind");
    let plain = format!("{rule}\n❯\n{rule}\n  🧠 Opus\n  ⏵⏵ on\n");
    assert!(!armed_reading(true, &plain, ToolKind::Claude));
}

#[test]
fn a_waiting_seat_is_forecast_its_wake_and_a_moving_one_is_not() {
    let near = Some(LimitReset {
        at: 10_000,
        short: true,
    });
    let line = waiting_forecast("lead", near, 9_000).expect("waits");
    assert!(line.contains("ae waits for lead's limit"), "{line}");
    assert!(
        line.contains(&crate::time::Timestamp::from_epoch(10_000).to_string()),
        "{line}"
    );
    assert!(!line.contains("will move"), "{line}");
    assert_eq!(waiting_forecast("lead", near, 6_400), None, "60 min: moves");
    assert_eq!(waiting_forecast("lead", None, 9_000), None, "unknown");
}
