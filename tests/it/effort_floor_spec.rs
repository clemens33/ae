//! P2 live acceptance, nested in `effort_live_spec` to reuse its vendor fixture.
//! Oracle: lead P2 ruling and #318. All effort observations come from real argv.

use super::*;

const FLOORS: &str =
    "[effort_floor]\n\"opus\" = \"medium\"\n\"gpt-6.1-sol\" = \"medium\"\n\"sonnet\" = \"high\"\n";

fn floor_seat(tag: &str, tool: &'static str, baseline: &str, floors: &str) -> Seat {
    let seat = setup(tag, tool, baseline, "auto_effort = on");
    let path = seat.rig.scratch.join("config");
    let text = std::fs::read_to_string(&path).expect("fixture config");
    let base = text
        .lines()
        .find_map(|line| line.strip_prefix("fake-effort-base = "))
        .expect("baseline command");
    let knob = if tool == "claude" {
        format!("--effort {baseline}")
    } else {
        format!("model_reasoning_effort={baseline}")
    };
    let mut profiles = String::new();
    let vocabulary: &[&str] = if tool == "claude" {
        &["low", "medium", "high", "xhigh", "max"]
    } else {
        &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"]
    };
    let existing_low = if baseline == "max" { "xhigh" } else { "high" };
    for &target in vocabulary {
        if target == baseline || target == existing_low {
            continue;
        }
        let target_knob = if tool == "claude" {
            format!("--effort {target}")
        } else {
            format!("model_reasoning_effort={target}")
        };
        writeln!(
            profiles,
            "p2-{target} = {}",
            base.replace(&knob, &target_knob)
        )
        .expect("profile text");
    }
    let updated = format!(
        "{}{floors}",
        text.replace("[workspace]", &format!("{profiles}[workspace]"))
    );
    ae::config::parse_identity(&updated).expect("valid model-floor fixture config");
    std::fs::write(path, updated).expect("all same-command effort twins and literal floor rows");
    seat
}

fn last_effort(seat: &Seat, expected: &str) {
    let launched = seat.rig.launched();
    let last = launched.lines().last().expect("real launch argv");
    let marker = if seat.tool == "claude" {
        format!("--effort {expected}")
    } else {
        format!("model_reasoning_effort={expected}")
    };
    assert!(
        last.split_whitespace()
            .collect::<Vec<_>>()
            .windows(if seat.tool == "claude" { 2 } else { 1 })
            .any(|words| words.join(" ") == marker),
        "expected exact effort {expected}: {last}"
    );
    assert!(last.contains(CONV), "exact conversation survives: {last}");
}

fn applied(seat: &Seat, count: usize, expected: &str) {
    until(&seat.rig, || {
        records(&seat.rig, "effort-applied").len() == count
    });
    last_effort(seat, expected);
}

/// Cover the existing 15-second detached-leg cadence as well as complete sweeps.
/// Merely observing two fast sweeps could miss an eligible third-step attempt.
fn hold(seat: &Seat, count: usize, expected: &str) {
    let launches = seat.rig.launched().lines().count();
    std::thread::sleep(Duration::from_secs(17));
    sweeps(&seat.rig);
    assert_eq!(
        records(&seat.rig, "effort-applied").len(),
        count,
        "no extra applied step: {}",
        seat.rig.events()
    );
    assert_eq!(
        seat.rig.launched().lines().count(),
        launches,
        "holding cannot restart the tool"
    );
    last_effort(seat, expected);
}

fn restore(seat: &Seat, slot: &str, current: &str, baseline: &str) {
    let before = seat.rig.launched().lines().count();
    quota(seat, 75.0, 0.0);
    hold(seat, records(&seat.rig, "effort-applied").len(), current);
    assert!(
        records(&seat.rig, "effort-restored").is_empty(),
        "75 is not below 75"
    );
    quota(seat, 74.9, 0.0);
    until(&seat.rig, || {
        !records(&seat.rig, "effort-restored").is_empty()
    });
    assert_eq!(
        records(&seat.rig, "effort-restored").len(),
        1,
        "one episode ends in one restoration"
    );
    last_effort(seat, baseline);
    assert_eq!(
        seat.rig.launched().lines().count(),
        before + 1,
        "restore directly to baseline in one restart"
    );
    assert!(
        !seat
            .rig
            .meta()
            .lines()
            .any(|line| line.starts_with(&format!("effort.{slot}="))),
        "accepted recovery releases claim"
    );
}

fn two_step_journal(rig: &Rig, restored: bool) {
    let episodes = records(rig, "effort-episode");
    assert_eq!(episodes.len(), 1, "both steps share one episode");
    let ep = episodes[0].reference.as_deref().expect("episode reference");
    for action in ["effort-reserved", "effort-applied"] {
        let down: Vec<_> = records(rig, action)
            .into_iter()
            .filter(|event| {
                event.reference.as_deref().is_some_and(|reference| {
                    reference.ends_with("-down") || reference.ends_with("-down2")
                })
            })
            .collect();
        assert_eq!(down.len(), 2, "two durable downward receipts: {action}");
        assert_eq!(
            down[0].reference.as_deref(),
            Some(format!("{ep}-down").as_str())
        );
        assert_eq!(down[0].summary.as_deref(), Some("xhigh->high"));
        assert_eq!(
            down[1].reference.as_deref(),
            Some(format!("{ep}-down2").as_str())
        );
        assert_eq!(down[1].summary.as_deref(), Some("high->medium"));
    }
    if restored {
        let up = records(rig, "effort-restored");
        assert_eq!(up.len(), 1, "one direct baseline restoration");
        assert_eq!(
            up[0].reference.as_deref(),
            Some(format!("{ep}-up").as_str())
        );
        assert_eq!(up[0].summary.as_deref(), Some("medium->xhigh"));
    }
}

fn two_steps(tag: &str, tool: &'static str, slot: &str, agent: &str) {
    let seat = floor_seat(tag, tool, "xhigh", FLOORS);
    if slot != "main" {
        remap_main(&seat, slot, agent);
    }
    let rig = &seat.rig;
    let pane = rig.main_pane.clone();
    let account = rig.meta_row(&format!("config_home.{slot}"));
    let baseline_launches = rig.launched().lines().count();
    quota(&seat, 79.9, 99.0);
    let daemon = watch(&seat);
    hold(&seat, 0, "xhigh");
    quota(&seat, 80.0, 0.0);
    applied(&seat, 1, "high");
    drop(daemon);
    let daemon = watch(&seat);
    quota(&seat, 89.9, 99.0);
    hold(&seat, 1, "high");
    quota(&seat, 90.0, 0.0);
    applied(&seat, 2, "medium");
    two_step_journal(rig, false);
    assert_eq!(
        rig.launched().lines().count(),
        baseline_launches + 2,
        "one restart per applied step"
    );
    drop(daemon);
    let daemon = watch(&seat);
    quota(&seat, 100.0, 100.0);
    hold(&seat, 2, "medium");
    assert_eq!(rig.meta_row(&format!("harness_session.{slot}")), CONV);
    assert_eq!(rig.meta_row(&format!("profile.{slot}")), "fake-effort-base");
    assert_eq!(rig.meta_row(&format!("config_home.{slot}")), account);
    assert_eq!(rig.main_pane, pane);
    assert!(
        records(rig, "effort-applied")
            .iter()
            .all(|event| ae::watchdog::event_is_addressed_to(event, &rig.session, slot, agent)),
        "both adjustments belong to this seat"
    );
    restore(&seat, slot, "medium", "xhigh");
    two_step_journal(rig, true);
    let finished = (rig.launched(), rig.events());
    drop(daemon);
    let _daemon = watch(&seat);
    sweeps(rig);
    assert_eq!(rig.launched(), finished.0, "restart cannot restore twice");
    assert_eq!(
        records(rig, "effort-restored").len(),
        finished
            .1
            .lines()
            .filter(
                |line| Event::parse_line(line).is_ok_and(|event| event.action == "effort-restored")
            )
            .count()
    );
}

#[test]
fn p2_opus_main_obeys_both_thresholds_floor_replay_and_single_restart_restore() {
    two_steps("p2opusmain", "claude", "main", "lead");
}

#[test]
fn p2_sol_worker_obeys_both_thresholds_floor_replay_and_single_restart_restore() {
    two_steps("p2solworker", "codex", "worker.0", "colead");
}

#[test]
fn p2_sol_spawned_obeys_both_thresholds_floor_replay_and_single_restart_restore() {
    two_steps("p2solspawned", "codex", "spawned.0", "scout");
}

#[test]
fn p2_two_step_cap_stops_above_a_lower_configured_floor() {
    let seat = floor_seat("p2cap", "codex", "max", FLOORS);
    quota(&seat, 80.0, 0.0);
    let _daemon = watch(&seat);
    applied(&seat, 1, "xhigh");
    quota(&seat, 90.0, 0.0);
    applied(&seat, 2, "high");
    quota(&seat, 100.0, 0.0);
    hold(&seat, 2, "high");
}

#[test]
fn p2_no_exact_usable_model_row_retains_exactly_one_legacy_step() {
    for (tag, floors) in [
        ("p2norow", ""),
        ("p2otherrow", "[effort_floor]\n\"opus\" = \"medium\"\n"),
        (
            "p2casekey",
            "[effort_floor]\n\"GPT-6.1-sol\" = \"medium\"\n",
        ),
        (
            "p2suffixkey",
            "[effort_floor]\n\"gpt-6.1-sol-extra\" = \"medium\"\n",
        ),
        (
            "p2invalidlevel",
            "[effort_floor]\n\"gpt-6.1-sol\" = \"not-a-level\"\n",
        ),
    ] {
        let seat = floor_seat(tag, "codex", "xhigh", floors);
        quota(&seat, 100.0, 0.0);
        let _daemon = watch(&seat);
        applied(&seat, 1, "high");
        hold(&seat, 1, "high");
        quota(&seat, 74.9, 0.0);
        until(&seat.rig, || {
            !records(&seat.rig, "effort-restored").is_empty()
        });
        last_effort(&seat, "xhigh");
        assert_eq!(
            seat.rig.launched().lines().count(),
            3,
            "one legacy down and one restore"
        );
    }
}

fn sonnet_seat(tag: &str, baseline: &str) -> Seat {
    let seat = floor_seat(tag, "claude", baseline, FLOORS);
    let rig = &seat.rig;
    let path = rig.scratch.join("config");
    let config = std::fs::read_to_string(&path)
        .expect("config")
        .replace("--model opus", "--model sonnet");
    std::fs::write(path, config).expect("same-command Sonnet profiles");
    let script = rig.scratch.join("claude.pl");
    let source = std::fs::read_to_string(&script)
        .expect("vendor fixture")
        .replace("'Opus 5.5'", "'Sonnet 5.5'");
    std::fs::write(script, source).expect("vendor reports actual Sonnet model");
    rig.kill_tools(&rig.main_pane);
    rig.start(&rig.main_pane, "main", "claude");
    assert!(
        rig.capture(&rig.main_pane).contains("Sonnet 5.5"),
        "Sonnet premise"
    );
    seat
}

#[test]
fn p2_sonnet_high_floor_clamps_at_high_with_lower_twins_available() {
    let seat = sonnet_seat("p2sonnet", "xhigh");
    quota(&seat, 80.0, 0.0);
    let _daemon = watch(&seat);
    applied(&seat, 1, "high");
    quota(&seat, 100.0, 100.0);
    hold(&seat, 1, "high");
}

#[test]
fn p2_sonnet_max_baseline_consumes_its_high_floor_row_for_two_adjacent_steps() {
    let seat = sonnet_seat("p2sonnetmax", "max");
    quota(&seat, 80.0, 0.0);
    let _daemon = watch(&seat);
    applied(&seat, 1, "xhigh");
    quota(&seat, 90.0, 0.0);
    applied(&seat, 2, "high");
    quota(&seat, 100.0, 100.0);
    hold(&seat, 2, "high");
}

#[test]
fn p2_effective_weekly_credit_keeps_a_raw_ninety_five_percent_account_below_entry() {
    let seat = floor_seat("p2credit", "claude", "xhigh", FLOORS);
    let path = seat.rig.scratch.join("config");
    let old = std::fs::read_to_string(&path).expect("config");
    let text = format!(
        "[clients]\ncredited = claude config_home={} manual_resets=1\n{old}",
        seat.account.display()
    );
    ae::config::parse_identity(&text).expect("manual reset credit fixture");
    std::fs::write(path, text).expect("credited own account");
    quota(&seat, 95.0, 0.0);
    let _daemon = watch(&seat);
    hold(&seat, 0, "xhigh");
}

#[test]
fn p2_a_literal_v1_applied_record_can_take_step_two_and_restore_the_v1_baseline() {
    // Final state is the state old ae emits after xhigh -> high. Only vendor
    // launch protocol is simulated; record and journal grammar are literal v1.
    let seat = floor_seat("p2v1upgrade", "codex", "high", FLOORS);
    let rig = &seat.rig;
    last_effort(&seat, "high");
    let config_path = rig.scratch.join("config");
    let config = std::fs::read_to_string(&config_path).expect("fixture config");
    let baseline = config
        .lines()
        .find(|line| line.starts_with("fake-effort-base = "))
        .expect("baseline row");
    let upgraded_baseline = baseline.replace(
        "model_reasoning_effort=high",
        "model_reasoning_effort=xhigh",
    );
    let updated = config
        .replacen(baseline, &upgraded_baseline, 1)
        .lines()
        .filter(|line| !line.starts_with("p2-xhigh = "))
        .fold(String::new(), |mut text, line| {
            text.push_str(line);
            text.push('\n');
            text
        });
    std::fs::write(config_path, updated)
        .expect("retained original xhigh baseline and high/medium twins");
    let now = Timestamp::now();
    let ep = now.epoch();
    let pid = rig
        .tool_pid(&rig.main_pane, "codex")
        .expect("real running high process");
    let launch = rig.meta_row("launch_id.main");
    assert!(!launch.is_empty());
    let value = format!(
        "v1 applied slot:main base:xhigh low:high at:80 ep:{ep} sid:{SID} agent:lead tool:codex launch:{launch} pid:{pid} conv:{CONV} profile:fake-effort-base client:- model:gpt-6.1-sol win:weekly/~/10080/{ep}/{}",
        ep + 86_400
    );
    let record =
        ae::effort::Record::parse(&value).expect("literal legacy applied record remains valid");
    assert_eq!(record.base, Level::Xhigh);
    assert_eq!(record.low, Level::High);
    put_row(rig, "effort.main", &value);
    let mut journal = String::new();
    for (action, reference, summary) in [
        ("effort-episode", ep.to_string(), "threshold 80"),
        ("effort-reserved", format!("{ep}-down"), "xhigh->high"),
        ("effort-applied", format!("{ep}-down"), "xhigh->high"),
    ] {
        writeln!(journal, "{{\"ts\":\"{now}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"lead\",\"target_slot\":\"main\",\"target_session\":\"{}\",\"target_session_id\":\"{SID}\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}", rig.session).expect("literal legacy history");
    }
    std::fs::write(rig.dir.join("events.jsonl"), journal).expect("retained legacy episode");
    assert!(
        matches!(
            ae::effort::fold(
                &rig.events()
                    .lines()
                    .map(|line| Event::parse_line(line)
                        .expect("literal event in original journal order"))
                    .collect::<Vec<_>>(),
                ae::events::SessionKey::new(&rig.session, SID),
                "main",
                "lead"
            ),
            Fold::Open(_)
        ),
        "legacy history is an open episode before the new watchdog starts"
    );
    quota(&seat, 89.9, 0.0);
    let _daemon = watch(&seat);
    hold(&seat, 1, "high");
    quota(&seat, 90.0, 0.0);
    applied(&seat, 2, "medium");
    assert_eq!(
        rig.launched().lines().count(),
        2,
        "upgrade takes only the remaining second step"
    );
    restore(&seat, "main", "medium", "xhigh");
    assert_eq!(
        rig.launched().lines().count(),
        3,
        "one direct legacy-baseline restoration"
    );
}

#[test]
fn p2_each_step_needs_a_command_identical_twin_at_its_actual_target() {
    let seat = floor_seat("p2secondtwin", "codex", "xhigh", FLOORS);
    let path = seat.rig.scratch.join("config");
    let good = std::fs::read_to_string(&path).expect("all twins");
    let medium = good
        .lines()
        .find(|line| line.starts_with("p2-medium = "))
        .expect("medium twin");
    let wrong_model = medium.replace("-m gpt-6.1-sol", "-m another-model");
    assert_ne!(medium, wrong_model, "the false twin differs by model");
    std::fs::write(&path, good.replacen(medium, &wrong_model, 1)).expect("misleading medium twin");
    quota(&seat, 80.0, 0.0);
    let _daemon = watch(&seat);
    applied(&seat, 1, "high");
    quota(&seat, 90.0, 0.0);
    hold(&seat, 1, "high");
    assert_eq!(
        records(&seat.rig, "effort-reserved").len(),
        1,
        "no second reservation without its own proof"
    );
    std::fs::write(path, good).expect("supply the exact medium twin");
    applied(&seat, 2, "medium");
}

#[test]
fn p2_a_floor_at_or_above_the_baseline_spends_no_step() {
    for (tag, floor) in [("p2equalfloor", "xhigh"), ("p2abovefloor", "ultra")] {
        let floors = format!("[effort_floor]\n\"gpt-6.1-sol\" = \"{floor}\"\n");
        let seat = floor_seat(tag, "codex", "xhigh", &floors);
        quota(&seat, 100.0, 0.0);
        let _daemon = watch(&seat);
        hold(&seat, 0, "xhigh");
        assert!(records(&seat.rig, "effort-reserved").is_empty());
        assert!(records(&seat.rig, "effort-episode").is_empty());
    }
}

#[test]
fn p2_matching_floor_recovers_directly_after_only_the_first_step() {
    let seat = floor_seat("p2firstrestore", "codex", "xhigh", FLOORS);
    quota(&seat, 80.0, 0.0);
    let _daemon = watch(&seat);
    applied(&seat, 1, "high");
    restore(&seat, "main", "high", "xhigh");
    assert_eq!(records(&seat.rig, "effort-applied").len(), 1);
    let restored = records(&seat.rig, "effort-restored");
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].summary.as_deref(), Some("high->xhigh"));
}
