//! #305 part 3; oracle: brief invariants + lead L1-L12, before implementation.
//! Child of `reseat_headroom_spec`: reuses fixture mechanics, not decision oracles.
use super::*;

struct LeadSeat {
    rig: Rig,
    pane: String,
    slot: &'static str,
    name: &'static str,
}

fn put(rig: &Rig, key: &str, value: &str) {
    let prefix = format!("{key}=");
    let mut text = rig
        .meta()
        .lines()
        .filter(|line| !line.starts_with(&prefix))
        .fold(String::new(), |mut text, line| {
            writeln!(text, "{line}").expect("string");
            text
        });
    writeln!(text, "{prefix}{value}").expect("string");
    std::fs::write(rig.dir.join("meta"), text).expect("private meta");
}

fn fixture(tag: &str, layout: &str, slot: &'static str, knobs: &str) -> LeadSeat {
    let rig = Rig::new(tag);
    configure(
        &rig,
        &format!("auto_reseat = all\n{knobs}"),
        "fake-claude-a",
        "fake-claude-b",
    );
    phase1_remove_unused_profile(&rig, "fake-claude");
    put(&rig, "layout", layout);
    // A pair already on one known provider may keep it. This prevents the
    // independent pair guard from hiding a threshold result on worker.0.
    put(&rig, "profile.main", "fake-claude-a");
    put(&rig, "agent_bin.main", "claude");
    let (name, pane) = if slot == "main" {
        ("lead", rig.main_pane.clone())
    } else {
        rig.seat_rows(slot, "scout", "claude-a", "claude");
        ("scout", rig.new_pane(slot, "scout"))
    };
    quota(&rig, "home-a", &[("weekly_all", 20.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 20.0)], 0);
    isolate(&rig, &pane);
    rig.start(&pane, slot, "claude");
    assert!(
        rig.tool_pid(&pane, "claude").is_some(),
        "live source premise"
    );
    LeadSeat {
        rig,
        pane,
        slot,
        name,
    }
}

fn rows(seat: &LeadSeat, action: &str) -> Vec<Event> {
    records(&seat.rig)
        .into_iter()
        .filter(|e| {
            e.action == action
                && ae::watchdog::event_is_addressed_to(e, &seat.rig.session, seat.slot, seat.name)
        })
        .collect()
}

fn crossing(seat: &LeadSeat, used: f64, expected: bool) {
    let config = seat.rig.scratch.join("config");
    let text = std::fs::read_to_string(&config).expect("config");
    std::fs::write(
        &config,
        text.replace("auto_reseat_grace_secs = 0", "auto_reseat_grace_secs = 600"),
    )
    .expect("grace");
    quota(&seat.rig, "home-a", &[("weekly_all", used)], 0);
    let pid = seat.rig.tool_pid(&seat.pane, "claude");
    let mut daemon = watch_stepped(&seat.rig);
    sweeps_stepped(&seat.rig, &mut daemon);
    let opened = rows(seat, "auto-reseat-headroom");
    assert_eq!(
        !opened.is_empty(),
        expected,
        "slot={} used={used}: {}",
        seat.slot,
        seat.rig.events()
    );
    assert_eq!(
        seat.rig.tool_pid(&seat.pane, "claude"),
        pid,
        "600s grace preserves process"
    );
    assert!(rows(seat, ATTEMPT_ACTION).is_empty(), "no early attempt");
}

#[test]
fn leadat_new_red_pair_main_and_colead_do_not_cross_below_96() {
    for (tag, slot) in [("latbelowm", "main"), ("latbelowc", "worker.0")] {
        let seat = fixture(
            tag,
            "lead-pair",
            slot,
            "auto_reseat_at = 90\nauto_reseat_lead_at = 96",
        );
        crossing(&seat, 95.9, false);
    }
}

#[test]
fn leadat_new_red_same_seat_sensitive_control_92_is_suppressed_by_override() {
    let seat = fixture(
        "latsensitive92",
        "lead-pair",
        "main",
        "auto_reseat_at = 90\nauto_reseat_lead_at = 96",
    );
    crossing(&seat, 92.0, false);
    assert!(rows(&seat, "auto-reseat-headroom").is_empty());
}

#[test]
fn leadat_control_exact_96_crosses_on_both_lead_slots() {
    for (tag, slot) in [("latatmain", "main"), ("latatco", "worker.0")] {
        let seat = fixture(
            tag,
            "lead-pair",
            slot,
            "auto_reseat_at = 90\nauto_reseat_lead_at = 96",
        );
        crossing(&seat, 96.0, true);
    }
}

#[test]
fn leadat_control_other_slots_and_layouts_keep_global_bar() {
    for (tag, layout, slot) in [
        ("latverticalm", "vertical", "main"),
        ("latverticalc", "vertical", "worker.0"),
        ("lathorizontalm", "horizontal", "main"),
        ("latotherworker", "lead-pair", "worker.1"),
        ("latspawned", "lead-pair", "spawned.0"),
    ] {
        let seat = fixture(
            tag,
            layout,
            slot,
            "auto_reseat_at = 90\nauto_reseat_lead_at = off",
        );
        crossing(&seat, 92.0, true);
    }
}

#[test]
fn leadat_new_red_number_works_when_global_bar_is_off() {
    for (tag, slot) in [("latgloffm", "main"), ("latgloffc", "worker.0")] {
        let seat = fixture(
            tag,
            "lead-pair",
            slot,
            "auto_reseat_at = off\nauto_reseat_lead_at = 96",
        );
        crossing(&seat, 96.0, true);
    }
}

#[test]
fn leadat_control_global_off_stays_off_outside_lead_pair() {
    for (tag, layout, slot) in [
        ("latgofsolo", "vertical", "main"),
        ("latgofwc", "vertical", "worker.0"),
        ("latgofworker", "lead-pair", "worker.1"),
        ("latgofspawn", "lead-pair", "spawned.0"),
    ] {
        let seat = fixture(
            tag,
            layout,
            slot,
            "auto_reseat_at = off\nauto_reseat_lead_at = 96",
        );
        crossing(&seat, 96.0, false);
    }
}

#[test]
fn leadat_new_red_off_disables_crossing_on_both_lead_slots() {
    for (tag, slot) in [("latoffm", "main"), ("latoffc", "worker.0")] {
        let seat = fixture(
            tag,
            "lead-pair",
            slot,
            "auto_reseat_at = 90\nauto_reseat_lead_at = off",
        );
        crossing(&seat, 99.0, false);
    }
}

#[test]
fn leadat_control_orchestrator_and_on_main_eligibility_are_unchanged() {
    for (tag, orchestrator) in [("latorch", true), ("latonmain", false)] {
        let seat = fixture(
            tag,
            "lead-pair",
            "main",
            "auto_reseat_at = 90\nauto_reseat_lead_at = 50",
        );
        if orchestrator {
            put(&seat.rig, "meta_agent", "true");
        } else {
            let config = seat.rig.scratch.join("config");
            let text = std::fs::read_to_string(&config).expect("config");
            std::fs::write(
                config,
                text.replace("auto_reseat = all", "auto_reseat = on"),
            )
            .expect("on excludes main");
        }
        crossing(&seat, 99.0, false);
    }
}

fn reserve(seat: &LeadSeat, action: &str) -> Timestamp {
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
    let cause = if action == "limit" {
        "usage limit"
    } else {
        "headroom 96% weekly_all 7d"
    };
    let mut text = seat.rig.events();
    for (at, action, reference, summary) in [
        (key, action, String::new(), cause),
        (
            Timestamp::from_epoch(key.epoch() + 1),
            ATTEMPT_ACTION,
            format!(",\"ref\":\"{key}\""),
            "from fake-claude-a to fake-claude-b",
        ),
    ] {
        writeln!(text, "{{\"ts\":\"{at}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"{}\",\"target_slot\":\"{}\",\"target_session\":\"{}\"{reference},\"summary\":\"{summary}\"}}", seat.name, seat.slot, seat.rig.session).expect("journal");
    }
    std::fs::write(seat.rig.dir.join("events.jsonl"), text).expect("pre-leg history");
    key
}

fn run_leg(seat: &LeadSeat, key: Timestamp) -> std::process::Output {
    core(&seat.rig)
        .env_remove("TMUX_PANE")
        .arg("_auto-reseat")
        .arg(&seat.rig.dir)
        .args([seat.slot, &key.to_string()])
        .output()
        .expect("real detached leg")
}

fn stopped_or_moved(seat: &LeadSeat, key: Timestamp, moves: bool, held_word: &str) {
    let pid = seat.rig.tool_pid(&seat.pane, "claude");
    let output = run_leg(seat, key);
    if moves {
        assert!(output.status.success(), "{output:?}\n{}", seat.rig.events());
        assert_eq!(
            seat.rig.meta_row(&format!("profile.{}", seat.slot)),
            "fake-claude-b"
        );
        assert_eq!(rows(seat, DONE_ACTION).len(), 1, "{}", seat.rig.events());
    } else {
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(
            seat.rig.meta_row(&format!("profile.{}", seat.slot)),
            "fake-claude-a"
        );
        assert_eq!(seat.rig.tool_pid(&seat.pane, "claude"), pid);
        let held = rows(seat, HELD_ACTION);
        assert_eq!(held.len(), 1, "{}", seat.rig.events());
        assert!(
            held[0]
                .summary
                .as_deref()
                .unwrap_or_default()
                .contains(held_word),
            "{held:?}"
        );
    }
}

#[test]
fn leadat_new_red_leg_rechecks_lead_band_not_global_band() {
    let seat = fixture(
        "latlegbelow",
        "lead-pair",
        "worker.0",
        "auto_reseat_at = 95\nauto_reseat_lead_at = 96",
    );
    quota(&seat.rig, "home-a", &[("weekly_all", 90.9)], 0);
    let key = reserve(&seat, "auto-reseat-headroom");
    stopped_or_moved(
        &seat,
        key,
        false,
        "the account no longer reads at the headroom threshold",
    );
}

#[test]
fn leadat_control_leg_at_threshold_boundary_is_still_pressed() {
    let seat = fixture(
        "latlegat",
        "lead-pair",
        "worker.0",
        "auto_reseat_at = 95\nauto_reseat_lead_at = 96",
    );
    quota(&seat.rig, "home-a", &[("weekly_all", 96.0)], 0);
    let key = reserve(&seat, "auto-reseat-headroom");
    stopped_or_moved(&seat, key, true, "");
}

#[test]
fn leadat_new_red_lead_off_hold_names_the_active_key() {
    let seat = fixture(
        "latlegoff",
        "lead-pair",
        "main",
        "auto_reseat_at = 95\nauto_reseat_lead_at = off",
    );
    quota(&seat.rig, "home-a", &[("weekly_all", 96.0)], 0);
    let key = reserve(&seat, "auto-reseat-headroom");
    stopped_or_moved(
        &seat,
        key,
        false,
        "ineligible: held: auto_reseat_lead_at is off (headroom 96% weekly_all 7d)",
    );
}

#[test]
fn leadat_new_red_lead_number_survives_global_off_in_leg() {
    let seat = fixture(
        "latlegglobaloff",
        "lead-pair",
        "main",
        "auto_reseat_at = off\nauto_reseat_lead_at = 96",
    );
    quota(&seat.rig, "home-a", &[("weekly_all", 96.0)], 0);
    let key = reserve(&seat, "auto-reseat-headroom");
    stopped_or_moved(&seat, key, true, "");
}

#[test]
fn leadat_control_absent_row_keeps_exact_global_off_hold() {
    let seat = fixture("latlegacyoff", "lead-pair", "main", "auto_reseat_at = off");
    let key = reserve(&seat, "auto-reseat-headroom");
    stopped_or_moved(
        &seat,
        key,
        false,
        "ineligible: held: auto_reseat_at is off (headroom 96% weekly_all 7d)",
    );
}

#[test]
fn leadat_new_red_headroom_destination_uses_lead_bar() {
    let seat = fixture(
        "latdestbelow",
        "lead-pair",
        "worker.0",
        "auto_reseat_at = 95\nauto_reseat_lead_at = 96",
    );
    quota(&seat.rig, "home-a", &[("weekly_all", 98.0)], 0);
    quota(&seat.rig, "home-b", &[("weekly_all", 95.5)], 0);
    let key = reserve(&seat, "auto-reseat-headroom");
    stopped_or_moved(&seat, key, true, "");
}

#[test]
fn leadat_control_destination_exact_lead_bar_is_skipped() {
    let seat = fixture(
        "latdestat",
        "lead-pair",
        "worker.0",
        "auto_reseat_at = 95\nauto_reseat_lead_at = 96",
    );
    quota(&seat.rig, "home-a", &[("weekly_all", 98.0)], 0);
    quota(&seat.rig, "home-b", &[("weekly_all", 96.0)], 0);
    let key = reserve(&seat, "auto-reseat-headroom");
    let pid = seat.rig.tool_pid(&seat.pane, "claude");
    let output = run_leg(&seat, key);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(seat.rig.meta_row("profile.worker.0"), "fake-claude-a");
    assert_eq!(seat.rig.tool_pid(&seat.pane, "claude"), pid);
    let refused = rows(&seat, REFUSED_ACTION);
    assert_eq!(refused.len(), 1, "{}", seat.rig.events());
    assert!(
        refused[0]
            .summary
            .as_deref()
            .unwrap_or_default()
            .contains("fake-claude-b (no headroom)")
    );
}

#[test]
fn leadat_control_lead_off_does_not_disable_drawn_vendor_limit() {
    for (tag, lead) in [("latlimitoff", "off"), ("latlimitat", "96")] {
        let seat = fixture(
            tag,
            "lead-pair",
            "main",
            &format!("auto_reseat_at = 95\nauto_reseat_lead_at = {lead}"),
        );
        quota(&seat.rig, "home-a", &[("weekly_all", 99.0)], 0);
        seat.rig.mark_limited(&seat.pane);
        let key = reserve(&seat, "limit");
        stopped_or_moved(&seat, key, true, "");
    }
}

#[test]
fn leadat_new_red_bad_and_duplicate_rows_note_once_and_inherit_global() {
    for (tag, global, raw, used, expected) in [
        (
            "latbad80",
            "80",
            "auto_reseat_lead_at = nonsense",
            80.0,
            true,
        ),
        (
            "latbadoff",
            "off",
            "auto_reseat_lead_at = nonsense",
            99.0,
            false,
        ),
        (
            "lattwice80",
            "80",
            "auto_reseat_lead_at = 96\nauto_reseat_lead_at = off",
            80.0,
            true,
        ),
        (
            "lattwiceoff",
            "off",
            "auto_reseat_lead_at = 96\nauto_reseat_lead_at = off",
            99.0,
            false,
        ),
    ] {
        let seat = fixture(
            tag,
            "lead-pair",
            "main",
            &format!("auto_reseat_at = {global}\n{raw}"),
        );
        crossing(&seat, used, expected);
        let notes = records(&seat.rig)
            .into_iter()
            .filter(|e| {
                e.action == ae::autoreseat::NOTICE_ACTION
                    && e.summary
                        .as_deref()
                        .is_some_and(|s| s.contains("auto_reseat_lead_at"))
            })
            .collect::<Vec<_>>();
        assert_eq!(notes.len(), 1, "{}", seat.rig.events());
        let note = &notes[0];
        assert!(
            note.summary
                .as_deref()
                .unwrap_or_default()
                .ends_with("; auto_reseat_at used")
        );
        assert!(
            note.target.as_deref().unwrap_or_default().is_empty(),
            "target-less config note"
        );
    }
}

#[test]
fn leadat_control_project_overlay_cannot_supply_lead_knob() {
    let seat = fixture("latoverlay", "lead-pair", "main", "auto_reseat_at = 90");
    std::fs::create_dir_all(seat.rig.scratch.join(".ae")).expect("overlay");
    std::fs::write(
        seat.rig.scratch.join(".ae/config"),
        "[workspace]\nauto_reseat_lead_at = off\n",
    )
    .expect("overlay knob");
    crossing(&seat, 92.0, true);
}

#[test]
fn leadat_new_red_trigger_uses_lead_threshold_before_reserving() {
    let seat = fixture(
        "lattriggergone",
        "lead-pair",
        "worker.0",
        "auto_reseat_at = 95\nauto_reseat_lead_at = 96",
    );
    quota(&seat.rig, "home-a", &[("weekly_all", 90.9)], 0);
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
    let text = format!(
        "{{\"ts\":\"{key}\",\"actor\":\"watchdog\",\"action\":\"auto-reseat-headroom\",\"target\":\"{}\",\"target_slot\":\"{}\",\"target_session\":\"{}\",\"summary\":\"headroom 96% weekly_all 7d\"}}\n",
        seat.name, seat.slot, seat.rig.session
    );
    std::fs::write(seat.rig.dir.join("events.jsonl"), text).expect("episode without attempt");
    let pid = seat.rig.tool_pid(&seat.pane, "claude");
    let output = core(&seat.rig)
        .args([
            &format!("@{}", seat.rig.session),
            "send",
            seat.name,
            "auto reseat",
        ])
        .env("AE_SENDER_OVERRIDE", "watchdog")
        .env("_AE_EVENT_ACTION", ATTEMPT_ACTION)
        .output()
        .expect("trigger");
    assert!(
        !output.status.success(),
        "trigger holds before reserve: {output:?}"
    );
    assert!(
        rows(&seat, ATTEMPT_ACTION).is_empty(),
        "{}",
        seat.rig.events()
    );
    assert_eq!(seat.rig.tool_pid(&seat.pane, "claude"), pid);
}

#[test]
fn leadat_control_absent_same_seat_exact_headroom_open_effect() {
    for (tag, slot) in [("latlegacyopenm", "main"), ("latlegacyopenc", "worker.0")] {
        let seat = fixture(tag, "lead-pair", slot, "auto_reseat_at = 90");
        crossing(&seat, 92.0, true);
        let opened = rows(&seat, "auto-reseat-headroom");
        assert_eq!(opened.len(), 1);
        assert_eq!(
            opened[0].summary.as_deref(),
            Some("headroom 92% weekly_all 7d")
        );
        assert_eq!(opened[0].actor, "watchdog");
        assert_eq!(opened[0].target.as_deref(), Some(seat.name));
    }
}
