//! #309 part A: drive the real automatic leg against a private real tmux server.
//! Human brief + lead F1-F5 determine expected behavior, independently of the fix.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect only their private Rig stores and server"
)]

use super::seat_relaunch::Rig;
use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION, REFUSED_ACTION};
use ae::events::Event;
use ae::time::Timestamp;
use std::fmt::Write as _;

const REASON: &str = "a lead seat never changes model unasked";

struct Seat {
    rig: Rig,
    pane: String,
    slot: &'static str,
    name: &'static str,
    key: Timestamp,
}

fn row(rig: &Rig, key: &str, value: &str) {
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

fn command(rig: &Rig, account: &str, model: &str) -> String {
    format!(
        "CLAUDE_CONFIG_DIR={} {} {} --model {model}",
        rig.scratch.join(account).display(),
        rig.scratch.join("tools/claude").display(),
        rig.scratch.join("claude.pl").display()
    )
}

fn quota(rig: &Rig, account: &str, used: u8) {
    let now = Timestamp::now().epoch();
    let dir = rig.scratch.join(account);
    std::fs::create_dir_all(&dir).expect("private account");
    std::fs::write(dir.join(".claude.json"), format!(
        "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":{used},\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}",
        now * 1000, Timestamp::from_epoch(now + 86400)
    )).expect("private quota cache");
}

fn isolate(rig: &Rig, pane: &str) {
    let home = rig.scratch.join("home");
    std::fs::create_dir_all(&home).expect("private home");
    assert!(
        rig.tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            pane,
            "env",
            "-u",
            "CLAUDE_CONFIG_DIR",
            "-u",
            "CODEX_HOME",
            &format!("HOME={}", home.display()),
            &format!("AE_HOME={}", rig.scratch.display()),
            &format!("CONFIG_FILE={}", rig.scratch.join("config").display()),
            "/bin/sh"
        ])
        .0,
        "private shell environment before recording source identity"
    );
}

fn fixture(
    tag: &str,
    layout: &str,
    slot: &'static str,
    declared: bool,
    twin: bool,
    headroom: bool,
    off_account_sibling: bool,
) -> Seat {
    let rig = Rig::new(tag);
    let sibling_account = if off_account_sibling { "away" } else { "own" };
    quota(&rig, "own", if headroom { 96 } else { 20 });
    quota(&rig, "away", 20);
    quota(&rig, "twin", 20);
    let mut config = format!(
        "[profiles]\nsource = \"{}\"\nsibling = \"{}\"\npeer = \"{} {}\"\n",
        command(&rig, "own", "opus"),
        command(&rig, sibling_account, "haiku"),
        rig.scratch.join("tools/codex").display(),
        rig.scratch.join("codex.pl").display()
    );
    if twin {
        writeln!(config, "twin = \"{}\"", command(&rig, "twin", "opus")).expect("string");
    }
    writeln!(config,
        "[workspace]\nmain = lead\nlayout = {layout}\nauto_reseat = all\nauto_reseat_grace_secs = 0\nauto_reseat_at = 95"
    ).expect("string");
    if declared {
        config.push_str("[auto_reseat]\nsource = sibling\n");
    }
    std::fs::write(rig.scratch.join("config"), config).expect("private config");
    row(&rig, "layout", layout);
    let name = if slot == "main" { "lead" } else { "subject" };
    if slot == "main" {
        row(&rig, "profile.main", "source");
        row(&rig, "agent_bin.main", "claude");
        // A proven OpenAI partner keeps #305 from masking the model-floor rule.
        row(&rig, "seat.worker.0", "peer");
        row(&rig, "profile.worker.0", "peer");
        row(&rig, "agent_bin.worker.0", "codex");
    } else {
        row(&rig, "profile.main", "peer");
        row(&rig, "agent_bin.main", "codex");
        row(&rig, &format!("seat.{slot}"), name);
        row(&rig, &format!("profile.{slot}"), "source");
        row(&rig, &format!("agent_bin.{slot}"), "claude");
    }
    let pane = if slot == "main" {
        rig.main_pane.clone()
    } else {
        rig.new_pane(slot, name)
    };
    isolate(&rig, &pane);
    rig.start(&pane, slot, "claude");
    if !headroom {
        rig.mark_limited(&pane);
    }
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 60);
    let action = if headroom {
        "auto-reseat-headroom"
    } else {
        "limit"
    };
    let cause = if headroom {
        ",\"summary\":\"headroom 96% weekly_all 7d\""
    } else {
        ""
    };
    std::fs::write(rig.dir.join("events.jsonl"), format!(
        "{{\"ts\":\"{key}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"{name}\",\"target_slot\":\"{slot}\",\"target_session\":\"{}\"{cause}}}\n{{\"ts\":\"{}\",\"actor\":\"watchdog\",\"action\":\"{ATTEMPT_ACTION}\",\"target\":\"{name}\",\"target_slot\":\"{slot}\",\"target_session\":\"{}\",\"ref\":\"{key}\"}}\n",
        rig.session, Timestamp::now(), rig.session
    )).expect("watchdog episode and attempt");
    Seat {
        rig,
        pane,
        slot,
        name,
        key,
    }
}

fn core(rig: &Rig) -> super::cli::Runner {
    let mut command = super::cli::ae();
    command
        .env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("AE_SENDER_OVERRIDE");
    command
}

fn run_leg(seat: &Seat) -> std::process::Output {
    core(&seat.rig)
        .arg("_auto-reseat")
        .arg(&seat.rig.dir)
        .args([seat.slot, &seat.key.to_string()])
        .output()
        .expect("real automatic leg")
}

fn records(seat: &Seat, action: &str) -> Vec<Event> {
    seat.rig
        .events()
        .lines()
        .map(|line| Event::parse_line(line).expect("valid journal"))
        .filter(|event| {
            event.action == action
                && ae::watchdog::event_is_addressed_to(
                    event,
                    &seat.rig.session,
                    seat.slot,
                    seat.name,
                )
        })
        .collect()
}

fn refuses(seat: &Seat) {
    let before = seat
        .rig
        .tool_pid(&seat.pane, "claude")
        .expect("source tool is running");
    let output = run_leg(seat);
    let refused = records(seat, REFUSED_ACTION);
    assert_eq!(
        refused.len(),
        1,
        "must refuse selection, not stop/move or hold unmapped: {output:?}\n{}",
        seat.rig.events()
    );
    let summary = refused[0].summary.as_deref().expect("refusal cause");
    assert!(
        summary.starts_with("no-candidate: refused: no usable candidate:"),
        "{summary}"
    );
    assert!(
        summary.contains(&format!("sibling, derived sibling ({REASON})")),
        "{summary}"
    );
    assert_eq!(
        refused[0].reference.as_deref(),
        Some(seat.key.to_string().as_str())
    );
    assert!(records(seat, DONE_ACTION).is_empty());
    assert!(records(seat, "reseat").is_empty());
    assert!(records(seat, "stop").is_empty());
    assert_eq!(
        seat.rig.meta_row(&format!("profile.{}", seat.slot)),
        "source"
    );
    assert_eq!(seat.rig.tool_pid(&seat.pane, "claude"), Some(before));
    assert!(
        seat.rig
            .events()
            .lines()
            .filter_map(|line| Event::parse_line(line).ok())
            .any(|event| event.action == "chat"
                && event.actor == "watchdog"
                && event
                    .summary
                    .as_deref()
                    .is_some_and(|text| text.contains(REASON))),
        "human sees reason: {}",
        seat.rig.events()
    );
}

fn moves(seat: &Seat, profile: &str, provenance: &str) {
    let output = run_leg(seat);
    assert!(output.status.success(), "{output:?}\n{}", seat.rig.events());
    assert_eq!(
        seat.rig.meta_row(&format!("profile.{}", seat.slot)),
        profile,
        "{output:?}\n{}",
        seat.rig.events()
    );
    let done = records(seat, DONE_ACTION);
    assert_eq!(done.len(), 1, "{}", seat.rig.events());
    assert!(
        done[0]
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains(provenance)),
        "{done:?}"
    );
    assert!(records(seat, REFUSED_ACTION).is_empty());
}

#[test]
fn leadfloor_real_limit_main_with_only_siblings_refuses_without_stopping() {
    refuses(&fixture(
        "lfmain",
        "lead-pair",
        "main",
        false,
        false,
        false,
        false,
    ));
}

#[test]
fn leadfloor_real_limit_colead_and_off_account_sibling_are_also_protected() {
    refuses(&fixture(
        "lfcolead",
        "lead-pair",
        "worker.0",
        false,
        false,
        false,
        true,
    ));
}

#[test]
fn leadfloor_real_nonlead_and_vertical_main_keep_the_same_sibling_move() {
    for (tag, layout, slot) in [
        ("lfworker", "lead-pair", "spawned.0"),
        ("lfvertical", "vertical", "main"),
    ] {
        moves(
            &fixture(tag, layout, slot, false, false, false, false),
            "sibling",
            "derived sibling",
        );
    }
}

#[test]
fn leadfloor_real_lead_uses_twin_instead_of_kept_sibling() {
    moves(
        &fixture("lftwin", "lead-pair", "main", false, true, false, false),
        "twin",
        "derived twin",
    );
}

#[test]
fn leadfloor_real_lead_keeps_declared_weaker_model_authority() {
    let seat = fixture("lfdeclared", "lead-pair", "main", true, false, false, false);
    moves(&seat, "sibling", "to sibling");
    assert!(
        !records(&seat, DONE_ACTION)[0]
            .summary
            .as_deref()
            .expect("done")
            .contains("derived")
    );
}

#[test]
fn leadfloor_real_headroom_main_with_only_siblings_refuses() {
    refuses(&fixture(
        "lfroom",
        "lead-pair",
        "main",
        false,
        false,
        true,
        true,
    ));
}

#[test]
fn leadfloor_real_human_manual_model_change_remains_available() {
    let seat = fixture("lfmanual", "lead-pair", "main", false, false, false, false);
    let output = core(&seat.rig)
        .args(["reseat", &seat.rig.session, seat.name, "--using", "sibling"])
        .output()
        .expect("human manual reseat");
    assert!(output.status.success(), "{output:?}\n{}", seat.rig.events());
    assert_eq!(seat.rig.meta_row("profile.main"), "sibling");
    assert!(
        records(&seat, DONE_ACTION).is_empty(),
        "manual move has no automatic done"
    );
}
