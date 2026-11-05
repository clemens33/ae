//! Supplemental to the frozen pairguard acceptance (#305 R8): what an automatic
//! move says about its pair is judged on the pair read AFTER the move, not on
//! the pair its choice was made from. Drives the real leg and the existing read
//! gate, like the frozen suite, with its own copies of the fixture helpers.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures inspect only their own private Rig stores and server"
)]

use std::fmt::Write as _;
use std::process::Stdio;
use std::time::{Duration, Instant};

use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION};
use ae::events::Event;
use ae::time::Timestamp;

use super::seat_relaunch::Rig;

const SLOT: &str = "spawned.0";
const NAME: &str = "slice-nav";

fn row(rig: &Rig, key: &str, value: &str) {
    let prefix = format!("{key}=");
    let mut after = String::new();
    let mut seen = false;
    for line in rig.meta().lines() {
        if line.starts_with(&prefix) {
            seen = true;
            writeln!(after, "{key}={value}").expect("string");
        } else {
            writeln!(after, "{line}").expect("string");
        }
    }
    if !seen {
        writeln!(after, "{key}={value}").expect("string");
    }
    std::fs::write(rig.dir.join("meta"), after).expect("private meta");
}

fn command(rig: &Rig, tool: &str, account: &str, extra: &str) -> String {
    let account = if tool == "claude" {
        format!("CLAUDE_CONFIG_DIR={} ", rig.scratch.join(account).display())
    } else {
        String::new()
    };
    format!(
        "{account}{} {} {extra}",
        rig.scratch.join("tools").join(tool).display(),
        rig.scratch.join(format!("{tool}.pl")).display()
    )
}

/// The profiles `source`, `peer` and `first` (all claude) and `other` (codex),
/// with `source`'s only candidate `first`.
fn configure(rig: &Rig) {
    let path = rig.scratch.join("config");
    let base = std::fs::read_to_string(&path).expect("Rig config");
    let mut text = base
        .split("[workspace]")
        .next()
        .expect("profiles")
        .to_owned();
    for (name, tool) in [
        ("source", "claude"),
        ("peer", "claude"),
        ("first", "claude"),
        ("other", "codex"),
    ] {
        let line = command(rig, tool, &format!("home-{name}"), &format!("--pg {name}"));
        writeln!(text, "{name} = \"{line}\"").expect("string");
    }
    writeln!(
        text,
        "[workspace]\nmain = lead\nlayout = vertical\nauto_reseat = all\nauto_reseat_grace_secs = 0\n\n[auto_reseat]\nsource = first"
    )
    .expect("string");
    std::fs::write(path, text).expect("private config");
}

fn seat(rig: &Rig, slot: &str, name: &str, profile: &str, tool: &str) {
    for (key, value) in [("seat", name), ("profile", profile), ("agent_bin", tool)] {
        row(rig, &format!("{key}.{slot}"), value);
    }
    let pane = rig.new_pane(slot, name);
    assert!(
        rig.tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            &pane,
            "env",
            "-u",
            "CLAUDE_CONFIG_DIR",
            "-u",
            "CODEX_HOME",
            &format!("HOME={}", rig.scratch.join("home").display()),
            &format!("AE_HOME={}", rig.scratch.display()),
            &format!("CONFIG_FILE={}", rig.scratch.join("config").display()),
            "/bin/sh",
        ])
        .0,
        "scratch shell environment before any harness starts"
    );
}

fn append(rig: &Rig, action: &str, ts: Timestamp, reference: &str) {
    let mut text = rig.events();
    let reference = if reference.is_empty() {
        String::new()
    } else {
        format!(",\"ref\":\"{reference}\"")
    };
    writeln!(
        text,
        "{{\"ts\":\"{ts}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"{NAME}\",\"target_slot\":\"{SLOT}\",\"target_session\":\"{}\"{reference}}}",
        rig.session
    )
    .expect("string");
    std::fs::write(rig.dir.join("events.jsonl"), text).expect("private journal");
}

/// A limit episode with its attempt open, as the trigger leaves it.
fn episode(rig: &Rig) -> Timestamp {
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 60);
    append(rig, "limit", key, "");
    append(rig, ATTEMPT_ACTION, Timestamp::now(), &key.to_string());
    key
}

fn of(rig: &Rig, action: &str) -> Vec<Event> {
    rig.events()
        .lines()
        .map(|line| Event::parse_line(line).expect("valid event"))
        .filter(|event| event.action == action)
        .collect()
}

fn wait_barrier(rig: &Rig, dir: &std::path::Path, key: &str, child: &mut super::cli::OwnedChild) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if std::fs::read_to_string(dir.join("trace"))
            .unwrap_or_default()
            .lines()
            .any(|line| line == format!("held {key}"))
        {
            return;
        }
        assert!(
            child.try_wait().expect("child status").is_none(),
            "barrier {key} was never reached: {}",
            rig.events()
        );
        assert!(
            Instant::now() < until,
            "barrier {key} not reached: {}",
            rig.events()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A pair collapsed on Anthropic when the choice is made takes its only,
/// same-provider candidate; the partner then moves onto codex before the
/// lifecycle lock. The move goes on, the pair ends diverse, and nothing says it
/// stays collapsed or has collapsed (#305 R8).
#[test]
fn a_pair_made_diverse_before_the_move_is_neither_labelled_staying_nor_told_collapsed() {
    let rig = Rig::new("pgafter");
    configure(&rig);
    seat(&rig, SLOT, NAME, "source", "claude");
    seat(&rig, "spawned.1", "slice", "peer", "claude");
    let key = episode(&rig);
    let gate = rig.scratch.join("read-gate");
    std::fs::create_dir_all(&gate).expect("private barrier");
    std::fs::write(gate.join("@auto-reseat-pair"), "").expect("arm the read gate");
    let mut cmd = super::cli::ae();
    cmd.env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env("AE_TEST_APP_READ_GATE", &gate)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("AE_SENDER_OVERRIDE")
        .arg("_auto-reseat")
        .arg(&rig.dir)
        .args([SLOT, &key.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("real leg child");
    wait_barrier(&rig, &gate, "@auto-reseat-pair", &mut child);
    row(&rig, "profile.spawned.1", "other");
    row(&rig, "agent_bin.spawned.1", "codex");
    std::fs::remove_file(gate.join("@auto-reseat-pair")).expect("release the choice barrier");
    child.wait_with_output().expect("leg ending");
    assert_eq!(
        rig.meta_row(&format!("profile.{SLOT}")),
        "first",
        "{}",
        rig.events()
    );
    let done = of(&rig, DONE_ACTION);
    assert_eq!(done.len(), 1, "{}", rig.events());
    let summary = done[0].summary.clone().unwrap_or_default();
    assert!(!summary.contains("pair stays on"), "{summary:?}");
    let said: Vec<String> = of(&rig, "chat")
        .into_iter()
        .filter_map(|event| event.summary)
        .collect();
    assert_eq!(said.len(), 1, "one move notice only: {said:?}");
    assert!(
        said[0].contains("slice-nav moved source -> first"),
        "{said:?}"
    );
    assert!(!said[0].contains("pair stays on"), "{said:?}");
    assert!(!said[0].contains("pair-provider collapse"), "{said:?}");
}
