//! Frozen acceptance contract: headroom judges long windows only (weeklyroom R1-R4).
//!
//! A headroom episode OPENS only on a usable window PROVEN at least one day
//! long (1440 min) at or past `auto_reseat_at`; due/near/relief and the
//! chooser's headroom skip read the same set. Short or unknown-length windows
//! never open one, and the chooser never skips for headroom on them — while a
//! short window at CRITICAL still disqualifies via the tier path, and the
//! vendor-limit path is unchanged.
//!
//! All quota stores, tools and tmux servers belong to the existing private Rig;
//! every config fixture parses through the real `src/config.rs` parser in the
//! real watchdog. RED tests fail on base at the asserted line; GUARD tests pin
//! unchanged behavior and pass on base and after.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures create and inspect scratch quota stores and journals only"
)]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use ae::autoreseat::{
    ATTEMPT_ACTION, DONE_ACTION, HEADROOM_ACTION, REARMED_ACTION, REFUSED_ACTION,
};
use ae::events::Event;

use super::seat_relaunch::Rig;

// Quota checkpoint delivery can defer 30 seconds twice before the same
// watchdog reaches its pane/action phase. Keep barriers bounded above that.
const WAIT: Duration = Duration::from_secs(90);

fn configure(rig: &Rig, knobs: &str, from: &str, candidates: &str) {
    let path = rig.scratch.join("config");
    let text = std::fs::read_to_string(&path).expect("fixture config");
    let profiles = text.split("[workspace]").next().expect("profiles");
    let switch = if knobs.lines().any(|line| line.starts_with("auto_reseat =")) {
        ""
    } else {
        "auto_reseat = on\n"
    };
    std::fs::write(
        path,
        format!(
            "{profiles}[workspace]\nmain = lead\nlayout = vertical\n{switch}auto_reseat_grace_secs = 0\n{knobs}\n[auto_reseat]\n{from} = {candidates}\n"
        ),
    )
    .expect("headroom config");
}

fn quota(rig: &Rig, home: &str, windows: &[(&str, f64)], age: i64) {
    let now = ae::time::Timestamp::now().epoch();
    let limits = windows
        .iter()
        .map(|(kind, percent)| {
            format!(
                "{{\"kind\":\"{kind}\",\"group\":\"{kind}\",\"percent\":{percent},\"resets_at\":\"{}\",\"scope\":null}}",
                ae::time::Timestamp::from_epoch(now + 86_400),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let dir = rig.scratch.join(home);
    std::fs::create_dir_all(&dir).expect("scratch account");
    std::fs::write(
        dir.join(".claude.json"),
        format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{limits}]}}}}}}\n",
            (now - age) * 1_000,
        ),
    )
    .expect("scratch quota");
}

fn seat(rig: &Rig, profile: &str) -> String {
    rig.seat_rows("spawned.0", "scout", profile, "claude");
    let pane = rig.new_pane("spawned.0", "scout");
    isolate(rig, &pane);
    rig.start(&pane, "spawned.0", "claude");
    pane
}

/// Start the fixture shell with scratch identity BEFORE `_run` records it.
fn isolate(rig: &Rig, pane: &str) {
    let home = rig.scratch.join("home");
    std::fs::create_dir_all(&home).expect("scratch home");
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
            "/bin/sh",
        ])
        .0,
        "atomic scratch shell environment"
    );
}

fn records(rig: &Rig) -> Vec<Event> {
    rig.events()
        .lines()
        .map(|line| Event::parse_line(line).expect("valid journal record"))
        .collect()
}

fn booked(rig: &Rig, action: &str) -> Vec<Event> {
    records(rig)
        .into_iter()
        .filter(|event| {
            event.action == action
                && ae::watchdog::event_is_addressed_to(event, &rig.session, "spawned.0", "scout")
        })
        .collect()
}

fn auto_records(rig: &Rig) -> Vec<Event> {
    records(rig)
        .into_iter()
        .filter(|event| event.action.starts_with("auto-reseat"))
        .collect()
}

fn watch(rig: &Rig) -> super::cli::OwnedChild {
    let trace = rig.scratch.join("weeklyroom-quota-trace");
    std::fs::write(&trace, "").expect("fresh per-spawn observation barrier");
    let send = rig.dir.join("send");
    if !send.exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), send).expect("fixture send link");
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rig.scratch.join("weeklyroom-daemon-err"))
        .expect("scratch daemon log");
    super::cli::ae()
        .arg("_watchdog-run")
        .arg(&rig.dir)
        .args(["--interval", "1", "--stale-secs", "999999"])
        .args(["--tg-supervise-secs", "0", "--quota-every-secs", "1"])
        .env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("AE_TEST_QUOTA_TRACE", trace)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .spawn()
        .expect("real fixture watchdog")
}

fn until(rig: &Rig, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "weeklyroom contract timed out: {}\n{}",
        rig.events(),
        std::fs::read_to_string(rig.scratch.join("weeklyroom-daemon-err")).unwrap_or_default(),
    );
}

/// Await two more completed observations from THIS daemon start, so absence
/// assertions cannot outrun its following seat/action phase.
fn sweeps(rig: &Rig) {
    let observed = || {
        std::fs::read_to_string(rig.scratch.join("weeklyroom-quota-trace"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("observed "))
            .count()
    };
    let before = observed();
    until(rig, || observed() >= before + 2);
}

fn unchanged(rig: &Rig, pane: &str, profile: &str, pid: Option<u32>) {
    assert_eq!(rig.meta_row("profile.spawned.0"), profile);
    assert_eq!(
        rig.tool_pid(pane, "claude"),
        pid,
        "the turn was not stopped"
    );
    assert!(booked(rig, ATTEMPT_ACTION).is_empty(), "{}", rig.events());
    assert!(booked(rig, DONE_ACTION).is_empty(), "{}", rig.events());
}

// Only these scratch profiles enter the identity config, so unrelated Rig
// defaults cannot supply a twin.
fn phase1_remove_unused_profile(rig: &Rig, profile: &str) {
    let path = rig.scratch.join("config");
    let text = std::fs::read_to_string(&path).expect("fixture config");
    let mut profiles = false;
    let mut kept = String::new();
    let prefix = format!("{profile} =");
    for line in text.lines() {
        if line.starts_with('[') {
            profiles = line == "[profiles]";
        }
        if !(profiles && line.starts_with(&prefix)) {
            writeln!(kept, "{line}").expect("fixture config line");
        }
    }
    std::fs::write(path, kept).expect("fixed phase1 candidate universe");
}

/// R1 RED: a 5h-only crossing opens nothing — no long window, no episode.
#[test]
fn five_hour_crossing_alone_opens_no_headroom() {
    let rig = Rig::new("wrshortonly");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    quota(&rig, "home-a", &[("session", 96.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 50.0)], 0);
    let pane = seat(&rig, "claude-a");
    let pid = rig.tool_pid(&pane, "claude");
    let _watch = watch(&rig);
    sweeps(&rig);
    assert!(
        auto_records(&rig).is_empty(),
        "a short crossing opens no episode: {}",
        rig.events()
    );
    unchanged(&rig, &pane, "fake-claude-a", pid);
}

/// R1 RED: mixed 5h 99% + weekly 50% opens nothing — the worst LONG window is
/// below the threshold.
#[test]
fn mixed_five_hour_crossing_with_low_weekly_opens_nothing() {
    let rig = Rig::new("wrmixedlow");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    quota(
        &rig,
        "home-a",
        &[("session", 99.0), ("weekly_all", 50.0)],
        0,
    );
    quota(&rig, "home-b", &[("weekly_all", 50.0)], 0);
    let pane = seat(&rig, "claude-a");
    let pid = rig.tool_pid(&pane, "claude");
    let _watch = watch(&rig);
    sweeps(&rig);
    assert!(
        auto_records(&rig).is_empty(),
        "weekly 50% relieves a 5h 99%: {}",
        rig.events()
    );
    unchanged(&rig, &pane, "fake-claude-a", pid);
}

/// R1 RED: when both cross, the opener names the weekly window, not the 5h one.
#[test]
fn mixed_crossing_names_the_weekly_window() {
    let rig = Rig::new("wrcause");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    quota(
        &rig,
        "home-a",
        &[("session", 99.0), ("weekly_all", 96.0)],
        0,
    );
    quota(&rig, "home-b", &[("weekly_all", 50.0)], 0);
    let _pane = seat(&rig, "claude-a");
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, HEADROOM_ACTION).len() == 1);
    let cause = booked(&rig, HEADROOM_ACTION)[0]
        .summary
        .clone()
        .unwrap_or_default();
    assert!(
        cause.contains("weekly_all"),
        "opener names the long window: {cause}"
    );
}

/// R1+R2 RED: relief follows the weekly window while the 5h stays hot — the
/// open episode re-arms once weekly reads strictly below threshold minus 5.
#[test]
fn relief_follows_the_weekly_window_while_five_hour_stays_hot() {
    let rig = Rig::new("wrrearm");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    quota(
        &rig,
        "home-a",
        &[("session", 99.0), ("weekly_all", 96.0)],
        0,
    );
    quota(&rig, "home-b", &[("weekly_all", 50.0)], 0);
    let pane = seat(&rig, "claude-a");
    rig.mark_busy(&pane);
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, HEADROOM_ACTION).len() == 1);
    quota(
        &rig,
        "home-a",
        &[("session", 99.0), ("weekly_all", 50.0)],
        0,
    );
    sweeps(&rig);
    assert_eq!(
        booked(&rig, REARMED_ACTION).len(),
        1,
        "weekly relief re-arms past a hot 5h: {}",
        rig.events()
    );
}

/// R3 RED: the chooser ignores a short crossing on the candidate — 5h 85% at
/// threshold 80, below critical, still has room and the seat moves.
#[test]
fn chooser_ignores_a_short_crossing_on_the_candidate() {
    let rig = Rig::new("wrchooseshort");
    configure(
        &rig,
        "auto_reseat_at = 80",
        "fake-claude-a",
        "fake-claude-b",
    );
    phase1_remove_unused_profile(&rig, "fake-claude");
    quota(&rig, "home-a", &[("weekly_all", 96.0)], 0);
    quota(
        &rig,
        "home-b",
        &[("session", 85.0), ("weekly_all", 50.0)],
        0,
    );
    let _pane = seat(&rig, "claude-a");
    let _watch = watch(&rig);
    until(&rig, || {
        booked(&rig, DONE_ACTION).len() == 1 || booked(&rig, REFUSED_ACTION).len() == 1
    });
    assert_eq!(
        booked(&rig, DONE_ACTION).len(),
        1,
        "short 85% keeps room: {}",
        rig.events()
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
}

/// R1 GUARD: a weekly crossing still moves an idle seat — the oracle's
/// unchanged opener, now the ONLY opener.
#[test]
fn weekly_crossing_still_moves_an_idle_seat() {
    let rig = Rig::new("wrweekly");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    quota(
        &rig,
        "home-a",
        &[("session", 50.0), ("weekly_all", 96.0)],
        0,
    );
    quota(&rig, "home-b", &[("weekly_all", 50.0)], 0);
    let _pane = seat(&rig, "claude-a");
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
}

/// R1 GUARD: a window whose length the vendor never stated never opens
/// headroom — no usable reading without minutes, on base and after.
#[test]
fn unknown_length_window_never_opens_headroom() {
    let rig = Rig::new("wrunknown");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    quota(&rig, "home-a", &[("nextgen_quota", 99.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 50.0)], 0);
    let pane = seat(&rig, "claude-a");
    let pid = rig.tool_pid(&pane, "claude");
    let _watch = watch(&rig);
    sweeps(&rig);
    assert!(
        auto_records(&rig).is_empty(),
        "unstated length opens nothing: {}",
        rig.events()
    );
    unchanged(&rig, &pane, "fake-claude-a", pid);
}

/// R3 GUARD: the chooser still skips a candidate whose WEEKLY is at the
/// threshold, naming no headroom.
#[test]
fn chooser_still_skips_a_candidate_on_weekly() {
    let rig = Rig::new("wrchooseweekly");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    phase1_remove_unused_profile(&rig, "fake-claude");
    quota(&rig, "home-a", &[("weekly_all", 96.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 96.0)], 0);
    let _pane = seat(&rig, "claude-a");
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, REFUSED_ACTION).len() == 1);
    assert!(booked(&rig, DONE_ACTION).is_empty(), "{}", rig.events());
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-a");
    let summary = booked(&rig, REFUSED_ACTION)[0]
        .summary
        .clone()
        .unwrap_or_default();
    assert!(
        summary.contains("no headroom"),
        "skip reason named: {summary}"
    );
}

/// R3 GUARD: a candidate with a 5h at critical still loses, by tier, to a
/// clean candidate — the critical path is unchanged.
#[test]
fn five_hour_critical_candidate_still_loses_to_a_clean_one() {
    let rig = Rig::new("wrcritical");
    configure(&rig, "", "fake-claude-a", "fake-claude-b, fake-claude");
    quota(&rig, "home-a", &[("weekly_all", 96.0)], 0);
    quota(
        &rig,
        "home-b",
        &[("session", 96.0), ("weekly_all", 50.0)],
        0,
    );
    quota(&rig, "home", &[("session", 50.0), ("weekly_all", 50.0)], 0);
    let _pane = seat(&rig, "claude-a");
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "fake-claude",
        "{}",
        rig.events()
    );
}

/// R4 GUARD: the vendor-limit path is unchanged — a limit moves the seat even
/// when only a short window is hot, with limit spelling.
#[test]
fn vendor_limit_still_moves_when_only_a_short_window_is_hot() {
    let rig = Rig::new("wrlimit");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    quota(
        &rig,
        "home-a",
        &[("session", 96.0), ("weekly_all", 50.0)],
        0,
    );
    quota(&rig, "home-b", &[("weekly_all", 50.0)], 0);
    let pane = seat(&rig, "claude-a");
    rig.mark_limited(&pane);
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
    assert!(
        !booked(&rig, ATTEMPT_ACTION)[0]
            .summary
            .as_deref()
            .unwrap_or_default()
            .contains("headroom"),
        "limit spelling preserved"
    );
}
