//! #305 part 3; `auto_effort` guard uses THIS seat's effective threshold (L7/L9).
use super::*;

fn lead_setup(tag: &str, layout: &str, global: &str, lead: Option<&str>) -> Seat {
    let seat = setup(tag, "claude", "xhigh", "auto_effort = on");
    let config = seat.rig.scratch.join("config");
    let old = std::fs::read_to_string(&config).expect("config");
    let knobs = format!(
        "auto_reseat = all\nauto_reseat_at = {global}{}",
        lead.map_or_else(String::new, |value| format!(
            "\nauto_reseat_lead_at = {value}"
        ))
    );
    std::fs::write(config, old.replace("auto_reseat = off", &knobs))
        .expect("reseat guard settings");
    put_row(&seat.rig, "layout", layout);
    quota(&seat, 85.0, 0.0);
    seat
}

fn guard_step(seat: &Seat, should_step: bool, note_key: Option<&str>) {
    let rig = &seat.rig;
    let pid = rig.tool_pid(&rig.main_pane, seat.tool);
    let _daemon = watch(seat);
    sweeps(rig);
    if should_step {
        until(rig, || !records(rig, "effort-reserved").is_empty());
        assert!(
            !records(rig, "effort-episode").is_empty(),
            "effective T>80/off permits effort: {}",
            rig.events()
        );
        assert!(
            !records(rig, "effort-reserved").is_empty(),
            "effective T>80/off must permit a reservation after completed cycles: {}",
            rig.events()
        );
        until(rig, || !records(rig, "effort-applied").is_empty());
        assert_eq!(records(rig, "effort-applied").len(), 1);
    } else {
        unchanged(seat, pid);
        assert!(
            records(rig, "effort-reserved").is_empty(),
            "inversion bars step downs, not episode bookkeeping"
        );
        assert!(records(rig, "effort-applied").is_empty());
    }
    if let Some(key) = note_key {
        let log =
            std::fs::read_to_string(rig.scratch.join("effort-daemon-err")).unwrap_or_default();
        let notes = log
            .lines()
            .filter(|line| line.contains("no new step down while"))
            .collect::<Vec<_>>();
        assert_eq!(notes.len(), 1, "one note per daemon: {log}");
        assert!(
            notes[0].contains(&format!("while {key} is 80 (at most 80)")),
            "{log}"
        );
    }
}

#[test]
fn leadat_new_red_effort_step_can_run_above_global_inversion() {
    let seat = lead_setup("lateffabove", "lead-pair", "80", Some("96"));
    guard_step(&seat, true, None);
}

#[test]
fn leadat_new_red_effort_lead_off_allows_step_above_global_inversion() {
    let seat = lead_setup("lateffoff", "lead-pair", "80", Some("off"));
    guard_step(&seat, true, None);
}

#[test]
fn leadat_new_red_effort_lower_lead_bar_blocks_step_and_names_lead_key() {
    let seat = lead_setup("lateffbelow", "lead-pair", "95", Some("80"));
    guard_step(&seat, false, Some("auto_reseat_lead_at"));
}

#[test]
fn leadat_new_red_equal_threshold_still_names_explicit_lead_key() {
    let seat = lead_setup("lateffequal", "lead-pair", "80", Some("80"));
    guard_step(&seat, false, Some("auto_reseat_lead_at"));
}

#[test]
fn leadat_control_nonpair_main_keeps_global_inversion() {
    let seat = lead_setup("lateffnonpair", "vertical", "80", Some("96"));
    guard_step(&seat, false, Some("auto_reseat_at"));
}

#[test]
fn leadat_control_absent_row_keeps_global_inversion_and_note() {
    let seat = lead_setup("latefflegacy", "lead-pair", "80", None);
    guard_step(&seat, false, Some("auto_reseat_at"));
}

#[test]
fn leadat_new_red_detached_effort_leg_rechecks_lead_bar() {
    let seat = lead_setup("lateffleg", "lead-pair", "80", Some("96"));
    let ep = open_episode(&seat, false);
    reserve_same_episode(&seat, ep);
}

#[test]
fn leadat_new_red_detached_effort_leg_holds_at_lower_lead_bar() {
    let seat = lead_setup("lateffleghold", "lead-pair", "95", Some("80"));
    let ep = open_episode(&seat, false);
    let pid = seat.rig.tool_pid(&seat.rig.main_pane, "claude");
    still_held(&seat, ep, pid);
    assert!(records(&seat.rig, "effort-reserved").is_empty());
}

#[test]
fn leadat_control_effort_off_holds_after_on_twin_applies_and_in_detached_leg() {
    let off = lead_setup("lateffswitchof", "lead-pair", "95", None);
    let on = lead_setup("lateffswitchon", "lead-pair", "95", None);
    quota(&off, 95.0, 0.0);
    quota(&on, 95.0, 0.0);
    let config = off.rig.scratch.join("config");
    let enabled = std::fs::read_to_string(&config).expect("effort configuration");
    std::fs::write(
        &config,
        enabled.replace("auto_effort = on", "auto_effort = off"),
    )
    .expect("disable effort only");
    let pid = off.rig.tool_pid(&off.rig.main_pane, off.tool);
    assert!(pid.is_some(), "off fixture starts with a live vendor");
    let off_daemon = watch(&off);
    let _on_daemon = watch(&on);
    until(&on.rig, || !records(&on.rig, "effort-reserved").is_empty());
    until(&on.rig, || !records(&on.rig, "effort-applied").is_empty());
    sweeps(&off.rig);
    unchanged(&off, pid);

    // A queued leg may outlive the daemon/config that opened its episode.
    drop(off_daemon);
    let ep = open_episode(&off, false);
    still_held(&off, ep, pid);
    // The same queued episode must work when only the switch changes back.
    std::fs::write(config, enabled).expect("enable the same queued episode");
    reserve_same_episode(&off, ep);
}
