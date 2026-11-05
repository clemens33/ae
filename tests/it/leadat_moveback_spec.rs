//! #305 part 3; return oracle: lead L4/L5/L9, strict T-5.
use super::*;

const LEAD_SLOT: &str = "worker.0";

fn lead_fixture(tag: &str, knobs: &str) -> (Rig, String) {
    let (rig, pane, _) = fixture(tag, knobs);
    let meta = rig.meta().replace(".spawned.0=", ".worker.0=");
    std::fs::write(rig.dir.join("meta"), meta).expect("same seat remapped to colead");
    row(&rig, "layout", "lead-pair");
    row(&rig, "profile.main", "origin");
    row(&rig, "agent_bin.main", "claude");
    let text = rig.events().replace("spawned.0", LEAD_SLOT);
    std::fs::write(rig.dir.join("events.jsonl"), text).expect("historical lead move");
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", LEAD_SLOT])
            .0
    );
    (rig, pane)
}

fn lead_rows(rig: &Rig, action: &str) -> Vec<Event> {
    records(rig)
        .into_iter()
        .filter(|e| {
            e.action == action
                && ae::watchdog::event_is_addressed_to(e, &rig.session, LEAD_SLOT, NAME)
        })
        .collect()
}

fn origin_after(rig: &Rig) {
    until(rig, "lead return completion is journaled", || {
        lead_rows(rig, DONE_ACTION).len() == 2
    });
    assert_eq!(rig.meta_row("profile.worker.0"), "origin");
    assert_eq!(
        lead_rows(rig, DONE_ACTION).len(),
        2,
        "one departure, one return"
    );
    let done = lead_rows(rig, DONE_ACTION);
    assert_eq!(done[1].reference.as_deref(), Some("substitute"));
}

#[test]
fn leadat_new_red_daemon_return_uses_91_band_and_strict_boundary() {
    let (rig, pane) = lead_fixture(
        "latreturn91",
        "auto_reseat_at = 95\nauto_reseat_lead_at = 96",
    );
    low(&rig, "home-a", 91.0);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert!(lead_rows(&rig, RETURN).is_empty(), "91 is not below 91");
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    low(&rig, "home-a", 90.9);
    sweeps(&rig);
    assert!(
        !lead_rows(&rig, RETURN).is_empty(),
        "90.9 must open a return at lead T96: {}",
        rig.events()
    );
    origin_after(&rig);
}

#[test]
fn leadat_new_red_lower_lead_threshold_blocks_an_early_return() {
    let (rig, pane) = lead_fixture(
        "latreturn75",
        "auto_reseat_at = 95\nauto_reseat_lead_at = 80",
    );
    low(&rig, "home-a", 78.0);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert!(
        lead_rows(&rig, RETURN).is_empty(),
        "78 is not below lead band75: {}",
        rig.events()
    );
    assert_eq!(rig.meta_row("profile.worker.0"), "substitute");
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
}

#[test]
fn leadat_control_lead_off_returns_only_below_90() {
    let (rig, pane) = lead_fixture(
        "latreturnoff",
        "auto_reseat_at = off\nauto_reseat_lead_at = off",
    );
    // A limit-triggered historical departure remains eligible under off.
    std::fs::write(
        rig.dir.join("events.jsonl"),
        rig.events().replace("auto-reseat-headroom", "limit"),
    )
    .expect("limit departure");
    low(&rig, "home-a", 90.0);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert!(lead_rows(&rig, RETURN).is_empty());
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    low(&rig, "home-a", 89.9);
    sweeps(&rig);
    assert!(
        !lead_rows(&rig, RETURN).is_empty(),
        "off still permits return: {}",
        rig.events()
    );
    origin_after(&rig);
}

#[test]
fn leadat_new_red_return_lead_bar_still_applies_when_global_off() {
    let (rig, _) = lead_fixture(
        "latreturngloff",
        "auto_reseat_at = off\nauto_reseat_lead_at = 96",
    );
    low(&rig, "home-a", 90.9);
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert!(
        !lead_rows(&rig, RETURN).is_empty(),
        "lead band91 survives global off: {}",
        rig.events()
    );
    origin_after(&rig);
}

#[test]
fn leadat_new_red_detached_return_rechecks_the_leads_band() {
    let (rig, _) = lead_fixture(
        "latreturnleg",
        "auto_reseat_at = 95\nauto_reseat_lead_at = 96",
    );
    low(&rig, "home-a", 90.9);
    let key = open_return(&rig);
    let history = rig.events().replace("spawned.0", LEAD_SLOT);
    std::fs::write(rig.dir.join("events.jsonl"), history).expect("return attempt on lead");
    let output = core(&rig)
        .arg("_auto-reseat")
        .arg(&rig.dir)
        .args([LEAD_SLOT, &key.to_string()])
        .output()
        .expect("detached return leg");
    assert!(output.status.success(), "{output:?}\n{}", rig.events());
    origin_after(&rig);
}

#[test]
fn leadat_control_absent_row_same_colead_return_effect() {
    let (rig, _) = lead_fixture("latreturnlegacy", "auto_reseat_at = 95");
    low(&rig, "home-a", 89.0);
    let _daemon = watch(&rig);
    origin_after(&rig);
    let returned = lead_rows(&rig, DONE_ACTION);
    assert_eq!(
        returned[1].summary.as_deref(),
        Some(
            "pair-provider: pair stays on Anthropic: to origin, seeded (return to origin: 89% weekly_all 7d)"
        ),
        "base-captured same-seat effect: {returned:?}"
    );
}
