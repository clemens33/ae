//! #309 part B: reuse the unchanged part-A Rig fixture to drive real automatic moves.
//! Install as tests/it/leadfloor2_spec.rs, child module of leadfloor_spec.rs.

use super::*;
use std::process::Stdio;
use std::time::{Duration, Instant};

fn floor(seat: &Seat, value: &str, tiers: &str) {
    let file = seat.rig.scratch.join("config");
    let mut text = std::fs::read_to_string(&file).expect("private config");
    write!(
        text,
        "\n[workspace]\nauto_reseat_lead_floor = {value}\n[model_tiers]\n{tiers}\n"
    )
    .expect("string");
    std::fs::write(file, text).expect("private floor config");
}

fn protected(seat: &Seat, row: &str) {
    let before = seat
        .rig
        .tool_pid(&seat.pane, "claude")
        .expect("running source");
    let output = run_leg(seat);
    let refused = records(seat, REFUSED_ACTION);
    assert_eq!(refused.len(), 1, "{output:?}\n{}", seat.rig.events());
    let summary = refused[0].summary.as_deref().expect("refusal summary");
    for word in ["no usable candidate", "sibling", "haiku", "tier", row] {
        assert!(summary.contains(word), "missing {word:?}: {summary}");
    }
    assert!(records(seat, DONE_ACTION).is_empty());
    assert!(records(seat, "reseat").is_empty());
    assert!(records(seat, "stop").is_empty());
    assert_eq!(
        seat.rig.meta_row(&format!("profile.{}", seat.slot)),
        "source"
    );
    assert_eq!(seat.rig.tool_pid(&seat.pane, "claude"), Some(before));
}

#[test]
fn leadfloor2_real_declared_floor_protects_both_leads_on_limit_and_headroom() {
    for (tag, slot, room) in [
        ("lf2declmain", "main", false),
        ("lf2declco", "worker.0", false),
        ("lf2declroom", "main", true),
        ("lf2declcoroom", "worker.0", true),
    ] {
        let seat = fixture(tag, "lead-pair", slot, true, false, room, true);
        floor(&seat, "3", "small = haiku, 1\nlarge = opus, 4");
        protected(&seat, "[auto_reseat] source");
    }
}

#[test]
fn leadfloor2_real_derived_equal_sibling_is_available_again() {
    let seat = fixture("lf2equal", "lead-pair", "main", false, false, false, true);
    floor(&seat, "3", "small = haiku, 3");
    moves(&seat, "sibling", "derived sibling");
}

#[test]
fn leadfloor2_real_derived_low_sibling_names_the_floor_and_keeps_the_process() {
    let seat = fixture("lf2derived", "lead-pair", "main", false, false, false, true);
    floor(&seat, "3", "small = haiku, 1");
    protected(&seat, "derived sibling");
}

#[test]
fn leadfloor2_real_nonlead_keeps_declared_low_model_choice() {
    let seat = fixture(
        "lf2nonlead",
        "lead-pair",
        "spawned.0",
        true,
        false,
        false,
        true,
    );
    floor(&seat, "3", "small = haiku, 1");
    moves(&seat, "sibling", "to sibling");
}

#[test]
fn leadfloor2_real_project_overlay_cannot_relax_the_global_lead_floor() {
    let seat = fixture("lf2overlay", "lead-pair", "main", true, false, false, true);
    floor(&seat, "3", "small = haiku, 1");
    let local = seat.rig.scratch.join(".ae");
    std::fs::create_dir_all(&local).expect("private project overlay");
    std::fs::write(
        local.join("config"),
        "[workspace]\nauto_reseat_lead_floor = 0\n[model_tiers]\nsmall = haiku, 255\n",
    )
    .expect("project override attempt");
    protected(&seat, "[auto_reseat] source");
}

#[test]
fn leadfloor2_real_twin_needs_no_model_tier_even_with_an_unusable_floor() {
    for (tag, value) in [("lf2twin", "3"), ("lf2twintypo", "bad")] {
        let seat = fixture(tag, "lead-pair", "main", false, true, false, false);
        floor(&seat, value, "small = haiku, 1");
        moves(&seat, "twin", "derived twin");
    }
}

#[test]
fn leadfloor2_real_manual_reseat_can_choose_a_below_floor_model() {
    let seat = fixture("lf2manual", "lead-pair", "main", false, false, false, true);
    floor(&seat, "3", "small = haiku, 1");
    let output = core(&seat.rig)
        .args(["reseat", &seat.rig.session, seat.name, "--using", "sibling"])
        .output()
        .expect("manual reseat");
    assert!(output.status.success(), "{output:?}\n{}", seat.rig.events());
    assert_eq!(seat.rig.meta_row("profile.main"), "sibling");
    assert!(records(&seat, DONE_ACTION).is_empty());
}

#[test]
fn leadfloor2_real_raised_floor_after_choice_refuses_before_the_stop() {
    let seat = fixture("lf2late", "lead-pair", "main", true, false, false, true);
    floor(&seat, "1", "small = haiku, 1");
    let before = seat
        .rig
        .tool_pid(&seat.pane, "claude")
        .expect("source running");
    let gate = seat.rig.scratch.join("floor-gate");
    std::fs::create_dir_all(&gate).expect("private barrier");
    std::fs::write(gate.join("@auto-reseat-pair"), "").expect("arm barrier");
    let mut command = core(&seat.rig);
    command
        .env("AE_TEST_APP_READ_GATE", &gate)
        .arg("_auto-reseat")
        .arg(&seat.rig.dir)
        .args([seat.slot, &seat.key.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("automatic leg child");
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if std::fs::read_to_string(gate.join("trace"))
            .unwrap_or_default()
            .lines()
            .any(|line| line == "held @auto-reseat-pair")
        {
            break;
        }
        assert!(
            child.try_wait().expect("child state").is_none(),
            "barrier not reached: {}",
            seat.rig.events()
        );
        assert!(
            Instant::now() < until,
            "barrier timed out: {}",
            seat.rig.events()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let file = seat.rig.scratch.join("config");
    let text = std::fs::read_to_string(&file).expect("private config");
    assert!(text.contains("auto_reseat_lead_floor = 1"));
    std::fs::write(
        &file,
        text.replace("auto_reseat_lead_floor = 1", "auto_reseat_lead_floor = 3"),
    )
    .expect("raise floor while choice is held");
    std::fs::remove_file(gate.join("@auto-reseat-pair")).expect("release choice");
    let output = child.wait_with_output().expect("automatic leg ending");
    assert!(
        records(&seat, DONE_ACTION).is_empty(),
        "{output:?}\n{}",
        seat.rig.events()
    );
    assert!(records(&seat, "reseat").is_empty());
    assert!(records(&seat, "stop").is_empty());
    assert_eq!(seat.rig.meta_row("profile.main"), "source");
    assert_eq!(seat.rig.tool_pid(&seat.pane, "claude"), Some(before));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("floor") && stderr.contains("nothing was stopped"),
        "{stderr}"
    );
}
