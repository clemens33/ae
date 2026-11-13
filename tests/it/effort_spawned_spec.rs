//! P2 S1: spawned seats use the existing one-step effort policy.
//! Real watchdog and real launch argv; no floor or v2-record assumption.

use super::*;

fn retire_applied(seat: &Seat) {
    let rig = &seat.rig;
    let episode = ae::effort::stored(rig.meta().as_bytes(), "spawned.0")
        .episode()
        .expect("held applied episode")
        .to_string();
    let main = rig.new_pane("main", "lead");
    for (key, value) in [
        ("main_pane", main.as_str()),
        ("seat.main", "lead"),
        ("profile.main", "fake-effort-base"),
        ("agent_bin.main", seat.tool),
    ] {
        put_row(rig, key, value);
    }
    let (code, stdout, stderr) =
        rig.run_top(&main, &[&format!("@{}", rig.session), "retire", "scout"]);
    assert_eq!(code, Some(0), "real retire: {stdout}\n{stderr}");
    let discarded = records(rig, "effort-discarded");
    assert_eq!(
        discarded.len(),
        1,
        "real caller journals exactly one ending"
    );
    assert_eq!(discarded[0].reference.as_deref(), Some(episode.as_str()));
    assert_eq!(discarded[0].summary.as_deref(), Some("retire"));
    assert!(ae::watchdog::event_is_addressed_to(
        &discarded[0],
        &rig.session,
        "spawned.0",
        "scout"
    ));
    assert!(matches!(
        ae::effort::stored(rig.meta().as_bytes(), "spawned.0"),
        Stored::Absent
    ));
    assert!(!rig.meta().contains("seat.spawned.0="));
    assert_eq!(
        ae::effort::fold(
            &rig.events()
                .lines()
                .map(|line| Event::parse_line(line).expect("journal event"))
                .collect::<Vec<_>>(),
            ae::events::SessionKey::new(&rig.session, SID),
            "spawned.0",
            "scout"
        ),
        Fold::Armed
    );
}

fn spawned_cycle(tag: &str, tool: &'static str, retire: bool) {
    let seat = setup(tag, tool, "xhigh", "auto_effort = on");
    remap_main(&seat, "spawned.0", "scout");
    let rig = &seat.rig;
    let original = rig.tool_pid(&rig.main_pane, tool);
    let account = rig.meta_row("config_home.spawned.0");
    quota(&seat, 95.0, 0.0);
    let daemon = watch(&seat);
    until(rig, || records(rig, "effort-applied").len() == 1);
    assert_ne!(
        rig.tool_pid(&rig.main_pane, tool),
        original,
        "real restart required"
    );
    assert_eq!(rig.meta_row("harness_session.spawned.0"), CONV);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-effort-base");
    assert_eq!(rig.meta_row("config_home.spawned.0"), account);
    assert_eq!(rig.launched().lines().count(), 2, "one down restart");
    let launched = rig.launched();
    let down = launched.lines().last().expect("actual restarted argv");
    assert!(down.contains(CONV));
    assert!(
        down.contains(if tool == "claude" {
            "--effort high"
        } else {
            "model_reasoning_effort=high"
        }),
        "same-model effort-only change: {down}"
    );
    assert!(
        matches!(ae::effort::stored(rig.meta().as_bytes(), "spawned.0"), Stored::Present(record) if record.state == ae::effort::State::Applied && record.base == Level::Xhigh && record.low == Level::High)
    );
    let applied = records(rig, "effort-applied");
    assert!(
        applied
            .iter()
            .all(|event| ae::watchdog::event_is_addressed_to(
                event,
                &rig.session,
                "spawned.0",
                "scout"
            ))
    );
    drop(daemon);
    if retire {
        retire_applied(&seat);
        return;
    }
    let _daemon = watch(&seat);
    std::thread::sleep(Duration::from_secs(17));
    sweeps(rig);
    assert_eq!(
        records(rig, "effort-applied").len(),
        1,
        "a resumed watchdog cannot repeat the step"
    );
    assert_eq!(rig.launched().lines().count(), 2);
    quota(&seat, 74.9, 0.0);
    until(rig, || records(rig, "effort-restored").len() == 1);
    let restored = rig.launched();
    let up = restored.lines().last().expect("actual restoration argv");
    assert!(up.contains(CONV));
    assert!(up.contains(if tool == "claude" {
        "--effort xhigh"
    } else {
        "model_reasoning_effort=xhigh"
    }));
    assert_eq!(restored.lines().count(), 3, "one restoration restart");
    assert_eq!(rig.meta_row("harness_session.spawned.0"), CONV);
    assert_eq!(rig.meta_row("config_home.spawned.0"), account);
    assert!(matches!(
        ae::effort::stored(rig.meta().as_bytes(), "spawned.0"),
        Stored::Absent
    ));
}

#[test]
fn p2_s1_spawned_claude_adjusts_and_restores_without_changing_identity() {
    spawned_cycle("p2s1claude", "claude", false);
}

#[test]
fn p2_s1_spawned_codex_adjusts_and_restores_without_changing_identity() {
    spawned_cycle("p2s1codex", "codex", false);
}

#[test]
fn p2_s1_real_retire_closes_a_watchdog_applied_spawned_episode() {
    spawned_cycle("p2s1retire", "codex", true);
}

#[test]
fn p2_s1_spawned_seat_in_an_orchestrator_session_is_still_excluded() {
    let seat = setup("p2s1orch", "codex", "xhigh", "auto_effort = on");
    remap_main(&seat, "spawned.0", "scout");
    put_row(&seat.rig, "meta_agent", "true");
    let original = seat.rig.tool_pid(&seat.rig.main_pane, seat.tool);
    quota(&seat, 95.0, 0.0);
    let _daemon = watch(&seat);
    std::thread::sleep(Duration::from_secs(17));
    sweeps(&seat.rig);
    assert_eq!(seat.rig.tool_pid(&seat.rig.main_pane, seat.tool), original);
    assert!(records(&seat.rig, "effort-episode").is_empty());
    assert!(records(&seat.rig, "effort-reserved").is_empty());
    assert!(records(&seat.rig, "effort-applied").is_empty());
    assert!(matches!(
        ae::effort::stored(seat.rig.meta().as_bytes(), "spawned.0"),
        Stored::Absent
    ));
}
