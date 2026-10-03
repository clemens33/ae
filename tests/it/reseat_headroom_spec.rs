//! Frozen acceptance contract: preserve subscription headroom without ending a turn.
//! All quota stores, tools and tmux servers belong to the existing private Rig.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures create and inspect scratch quota stores and journals only"
)]

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION, HELD_ACTION, REFUSED_ACTION, settings_in};
use ae::events::Event;
use ae::time::Timestamp;

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
    quota_reset(rig, home, windows, age, 86_400);
}

fn quota_reset(rig: &Rig, home: &str, windows: &[(&str, f64)], age: i64, reset_in: i64) {
    let now = Timestamp::now().epoch();
    let limits = windows
        .iter()
        .map(|(kind, percent)| {
            format!(
                "{{\"kind\":\"{kind}\",\"group\":\"{kind}\",\"percent\":{percent},\"resets_at\":\"{}\",\"scope\":null}}",
                Timestamp::from_epoch(now + reset_in),
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

fn rebind(rig: &Rig, key: &str, value: &str) {
    let path = rig.dir.join("meta");
    let text = std::fs::read_to_string(&path).expect("fixture meta");
    let prefix = format!("{key}=");
    assert!(text.lines().any(|line| line.starts_with(&prefix)));
    let text: String = text
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                format!("{prefix}{value}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    std::fs::write(path, text).expect("rebound fixture row");
}

fn seat(rig: &Rig, profile: &str) -> String {
    rig.seat_rows("spawned.0", "scout", profile, "claude");
    let pane = rig.new_pane("spawned.0", "scout");
    isolate(rig, &pane);
    rig.start(&pane, "spawned.0", "claude");
    pane
}

/// Start the fixture shell with scratch identity BEFORE `_run` records it.
/// The tmux server inherits the test runner's environment; daemon-only HOME
/// isolation would leave the pane's recorded scope pointing outside scratch.
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

/// A valid watchdog record, before any daemon runs. This fixes the ordering
/// of competing episodes without racing real processes to the same second.
fn seed(rig: &Rig, ts: Timestamp, action: &str, reference: Option<Timestamp>) {
    let reference = reference
        .map(|key| format!(r#","ref":"{key}""#))
        .unwrap_or_default();
    let summary = if action == "auto-reseat-headroom" {
        r#","summary":"headroom 95% weekly_all 7d""#
    } else {
        ""
    };
    let line = format!(
        "{}{{\"ts\":\"{ts}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session\":\"{}\"{reference}{summary}}}\n",
        rig.events(),
        rig.session,
    );
    std::fs::write(rig.dir.join("events.jsonl"), line).expect("ordered fixture journal");
}

fn core(rig: &Rig) -> super::cli::Runner {
    let mut command = super::cli::ae();
    command
        .env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("AE_SENDER_OVERRIDE");
    command
}

fn leg(rig: &Rig, key: Timestamp) -> std::process::Output {
    core(rig)
        .arg("_auto-reseat")
        .arg(&rig.dir)
        .args(["spawned.0", &key.to_string()])
        .output()
        .expect("real detached-leg entry")
}

fn trigger(rig: &Rig) -> std::process::Output {
    core(rig)
        .args([&format!("@{}", rig.session), "send", "scout", "auto reseat"])
        .env("AE_SENDER_OVERRIDE", "watchdog")
        .env("_AE_EVENT_ACTION", ATTEMPT_ACTION)
        .output()
        .expect("real watchdog-trigger entry")
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
    let trace = rig.scratch.join("headroom-quota-trace");
    std::fs::write(&trace, "").expect("fresh per-spawn observation barrier");
    let send = rig.dir.join("send");
    if !send.exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), send).expect("fixture send link");
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rig.scratch.join("headroom-daemon-err"))
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
        "headroom contract timed out: {}\n{}",
        rig.events(),
        std::fs::read_to_string(rig.scratch.join("headroom-daemon-err")).unwrap_or_default(),
    );
}

/// Await two more completed observations from THIS daemon start, so absence
/// assertions cannot outrun its following seat/action phase.
fn sweeps(rig: &Rig) {
    let observed = || {
        std::fs::read_to_string(rig.scratch.join("headroom-quota-trace"))
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

fn manual_reset(rig: &Rig, home: &str) {
    let path = rig.scratch.join("config");
    let text = std::fs::read_to_string(&path).expect("config");
    std::fs::write(
        path,
        format!(
            "[clients]\nreset = claude config_home={} manual_resets=1\n{text}",
            rig.scratch.join(home).display(),
        ),
    )
    .expect("effective quota declaration");
}

fn plant_conversation(rig: &Rig) -> (String, std::path::PathBuf) {
    let id = "12345678-1234-1234-1234-123456789abc".to_owned();
    let root = std::fs::canonicalize(&rig.scratch).expect("canonical scratch");
    let work = root.join("work");
    let home = root.join("home-a");
    let path = rig.dir.join("meta");
    let text = std::fs::read_to_string(&path).expect("meta");
    let text: String = text
        .lines()
        .filter(|line| {
            ![
                "work_dir=",
                "harness_session.spawned.0=",
                "config_home.spawned.0=",
                "config_home_base.spawned.0=",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        })
        .fold(String::new(), |mut kept, line| {
            let _ = writeln!(kept, "{line}");
            kept
        });
    std::fs::write(
        path,
        format!(
            "{text}work_dir={}\nharness_session.spawned.0={id}\nconfig_home.spawned.0={}\n",
            work.display(),
            home.display()
        ),
    )
    .expect("conversation meta");
    let key = ae::carry::project_key(&work);
    let relative = std::path::PathBuf::from("projects")
        .join(key)
        .join(format!("{id}.jsonl"));
    let transcript = home.join(&relative);
    std::fs::create_dir_all(transcript.parent().expect("transcript parent")).expect("store");
    std::fs::write(
        transcript,
        b"{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n",
    )
    .expect("synthetic transcript");
    (id, relative)
}

#[test]
fn effective_manual_resets_govern_both_the_source_and_the_candidate() {
    {
        let rig = Rig::new("hreffectsource");
        configure(&rig, "", "fake-claude-a", "fake-opencode");
        quota(&rig, "home-a", &[("weekly_all", 96.0)], 0);
        manual_reset(&rig, "home-a");
        let pane = seat(&rig, "claude-a");
        let pid = rig.tool_pid(&pane, "claude");
        let _watch = watch(&rig);
        sweeps(&rig);
        unchanged(&rig, &pane, "fake-claude-a", pid);
        assert!(
            auto_records(&rig).is_empty(),
            "raw96 judged48 cannot open headroom"
        );
    }
    {
        let rig = Rig::new("hreffectcandidate");
        configure(&rig, "", "fake-claude-a", "fake-claude-b");
        quota(&rig, "home-a", &[("weekly_all", 96.0)], 0);
        quota(&rig, "home-b", &[("weekly_all", 96.0)], 0);
        manual_reset(&rig, "home-b");
        let _pane = seat(&rig, "claude-a");
        let _watch = watch(&rig);
        until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
        assert_eq!(
            rig.meta_row("profile.spawned.0"),
            "fake-claude-b",
            "raw96 judged48 has room"
        );
    }
}

#[test]
fn headroom_skip_does_not_change_a_limit_move_onto_the_same_96_percent_candidate() {
    for (tag, limited) in [("hrlimit96", true), ("hr96", false)] {
        let rig = Rig::new(tag);
        configure(&rig, "", "fake-claude-a", "fake-claude-b");
        // Both arms judge the same account pair; no spare implicit-account twin.
        phase1_remove_unused_profile(&rig, "fake-claude");
        quota(&rig, "home-a", &[("weekly_all", 96.0)], 0);
        quota(&rig, "home-b", &[("weekly_all", 96.0)], 0);
        let pane = seat(&rig, "claude-a");
        if limited {
            rig.mark_limited(&pane);
        }
        let _watch = watch(&rig);
        if limited {
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
        } else {
            until(&rig, || booked(&rig, REFUSED_ACTION).len() == 1);
            assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-a");
            assert!(booked(&rig, DONE_ACTION).is_empty());
        }
    }
}

#[test]
fn threshold_off_disables_headroom_but_preserves_the_vendor_limit_path() {
    let rig = Rig::new("hrlimitoff");
    configure(&rig, "auto_reseat_at = off", "fake-claude", "fake-opencode");
    quota(&rig, "home", &[("weekly_all", 99.0)], 0);
    let pane = seat(&rig, "claude");
    rig.mark_limited(&pane);
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    sweeps(&rig);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    let attempts = booked(&rig, ATTEMPT_ACTION);
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].summary.as_deref(),
        Some("from fake-claude to fake-opencode")
    );
    assert!(
        !auto_records(&rig)
            .iter()
            .any(|e| e.summary.as_deref().is_some_and(|s| s.contains("headroom")))
    );
}

#[test]
fn threshold_turned_off_after_a_headroom_attempt_holds_without_moving_the_seat() {
    let rig = Rig::new("hrlegthresholdoff");
    configure(&rig, "", "fake-claude", "fake-opencode");
    quota(&rig, "home", &[("weekly_all", 95.0)], 0);
    let pane = seat(&rig, "claude");
    let pid = rig.tool_pid(&pane, "claude");
    assert!(pid.is_some(), "the fixture tool is running");
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
    seed(&rig, key, "auto-reseat-headroom", None);
    seed(
        &rig,
        Timestamp::from_epoch(key.epoch() + 1),
        ATTEMPT_ACTION,
        Some(key),
    );
    // The daemon booked its attempt while enabled. Its detached leg must
    // honor a threshold switched off before that leg reads the settings.
    configure(&rig, "auto_reseat_at = off", "fake-claude", "fake-opencode");
    let out = leg(&rig, key);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let held = booked(&rig, HELD_ACTION);
    assert_eq!(held.len(), 1, "{}", rig.events());
    assert_eq!(held[0].reference.as_deref(), Some(key.to_string().as_str()));
    assert_eq!(
        held[0].summary.as_deref(),
        Some("held: auto_reseat_at is off (headroom 95% weekly_all 7d)")
    );
    assert_eq!(booked(&rig, ATTEMPT_ACTION).len(), 1);
    assert!(booked(&rig, DONE_ACTION).is_empty());
    assert!(booked(&rig, REFUSED_ACTION).is_empty());
    assert!(booked(&rig, ae::autoreseat::FAILED_ACTION).is_empty());
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude");
    assert_eq!(
        rig.tool_pid(&pane, "claude"),
        pid,
        "the tool was not stopped"
    );
}

#[test]
fn a_vendor_limit_during_a_headroom_hold_moves_once_across_both_episode_kinds() {
    let rig = Rig::new("hrovertaken");
    configure(&rig, "", "fake-claude", "fake-opencode");
    quota(&rig, "home", &[("weekly_all", 95.0)], 0);
    let pane = seat(&rig, "claude");
    rig.mark_busy(&pane);
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, HELD_ACTION).len() == 1);
    assert!(booked(&rig, ATTEMPT_ACTION).is_empty());
    // Install the vendor refusal before removing BUSY. There is no idle gap
    // where the headroom episode could start its own attempt.
    std::fs::write(rig.scratch.join("__LIMIT__"), "").expect("vendor refusal");
    std::fs::remove_file(rig.scratch.join("__BUSY__")).expect("vendor stopped the fixture turn");
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    sweeps(&rig);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    assert_eq!(
        booked(&rig, ATTEMPT_ACTION).len(),
        1,
        "one move, limit takes priority"
    );
    assert_eq!(booked(&rig, DONE_ACTION).len(), 1, "one shared terminal");
    assert!(booked(&rig, REFUSED_ACTION).is_empty());
    assert!(booked(&rig, ae::autoreseat::FAILED_ACTION).is_empty());
}

#[test]
fn reseat_shared_leg_backs_off_without_waiting_or_writing_while_the_seat_is_locked() {
    for (tag, opener) in [
        ("hrleglocklimit", "limit"),
        ("hrleglockroom", "auto-reseat-headroom"),
    ] {
        let rig = Rig::new(tag);
        configure(&rig, "", "fake-claude", "fake-opencode");
        quota(&rig, "home", &[("weekly_all", 95.0)], 0);
        let pane = seat(&rig, "claude");
        let pid = rig.tool_pid(&pane, "claude");
        let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
        seed(&rig, key, opener, None);
        seed(
            &rig,
            Timestamp::from_epoch(key.epoch() + 1),
            ATTEMPT_ACTION,
            Some(key),
        );
        let held = ae::store::lock(&rig.dir.join("auto-reseat.spawned.0.lock"), Duration::ZERO)
            .expect("fixture owns the seat lock");
        let before = rig.events();
        let started = Instant::now();
        let out = leg(&rig, key);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(rig.events(), before, "a contended leg writes no outcome");
        assert!(started.elapsed() < Duration::from_secs(2), "no lock wait");
        assert_eq!(rig.tool_pid(&pane, "claude"), pid);
        drop(held);
        let out = leg(&rig, key);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(booked(&rig, DONE_ACTION).len(), 1);
        assert_eq!(booked(&rig, ATTEMPT_ACTION).len(), 1);
        assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    }
}

#[test]
fn reseat_shared_open_attempt_blocks_the_other_trigger_kind() {
    for (tag, active, other, drawn) in [
        ("hrflylimit", "auto-reseat-headroom", "limit", true),
        ("hrflyroom", "limit", "auto-reseat-headroom", false),
    ] {
        let rig = Rig::new(tag);
        configure(&rig, "", "fake-claude", "fake-opencode");
        quota(&rig, "home", &[("weekly_all", 95.0)], 0);
        let pane = seat(&rig, "claude");
        let pid = rig.tool_pid(&pane, "claude");
        if drawn {
            rig.mark_limited(&pane);
        }
        let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
        seed(&rig, key, active, None);
        seed(
            &rig,
            Timestamp::from_epoch(key.epoch() + 1),
            ATTEMPT_ACTION,
            Some(key),
        );
        seed(&rig, Timestamp::from_epoch(key.epoch() + 2), other, None);
        let before = rig.events();
        let out = trigger(&rig);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(rig.events(), before, "one open attempt per seat");
        assert_eq!(rig.tool_pid(&pane, "claude"), pid);
        assert_eq!(booked(&rig, ATTEMPT_ACTION).len(), 1);
    }
}

#[test]
fn reseat_shared_equal_keys_resolve_the_kind_with_an_open_attempt() {
    for (tag, active, other, headroom) in [
        ("hrtieroom", "auto-reseat-headroom", "limit", true),
        ("hrtielimit", "limit", "auto-reseat-headroom", false),
    ] {
        let rig = Rig::new(tag);
        configure(&rig, "", "fake-claude", "fake-opencode");
        quota(&rig, "home", &[("weekly_all", 95.0)], 0);
        seat(&rig, "claude");
        let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
        seed(&rig, key, active, None);
        seed(&rig, key, ATTEMPT_ACTION, Some(key));
        seed(&rig, key, other, None);
        let out = leg(&rig, key);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let done = booked(&rig, DONE_ACTION);
        assert_eq!(done.len(), 1);
        assert_eq!(booked(&rig, ATTEMPT_ACTION).len(), 1);
        assert_eq!(booked(&rig, REFUSED_ACTION).len(), 0);
        assert_eq!(
            done[0]
                .summary
                .as_deref()
                .unwrap_or_default()
                .contains("headroom"),
            headroom,
            "terminal names the winning trigger"
        );
        assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    }
}

#[test]
fn reseat_shared_refused_limit_also_closes_the_open_headroom_episode() {
    let rig = Rig::new("hrrefuseboth");
    configure(&rig, "", "fake-claude", "fake-claude-b");
    // R9 now appends family members: keep this candidate set exhausted.
    phase1_remove_unused_profile(&rig, "fake-claude-a");
    quota(&rig, "home", &[("weekly_all", 95.0)], 0);
    // At 100% this candidate is exhausted for the LIMIT chooser too.
    quota(&rig, "home-b", &[("weekly_all", 100.0)], 0);
    let pane = seat(&rig, "claude");
    let pid = rig.tool_pid(&pane, "claude");
    rig.mark_limited(&pane);
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
    seed(&rig, key, "auto-reseat-headroom", None);
    seed(&rig, Timestamp::from_epoch(key.epoch() + 2), "limit", None);
    let out = trigger(&rig);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(booked(&rig, REFUSED_ACTION).len(), 1);
    assert!(booked(&rig, ATTEMPT_ACTION).is_empty());
    rig.unmark_limited(&pane);
    let _watch = watch(&rig);
    sweeps(&rig);
    sweeps(&rig);
    assert_eq!(booked(&rig, REFUSED_ACTION).len(), 1, "one shared terminal");
    assert_eq!(booked(&rig, "auto-reseat-headroom").len(), 1);
    assert!(booked(&rig, ATTEMPT_ACTION).is_empty());
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
}

#[test]
fn same_tool_headroom_move_carries_the_original_conversation() {
    let rig = Rig::new("hrcarry");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 20.0)], 0);
    let pane = seat(&rig, "claude-a");
    let (id, relative) = plant_conversation(&rig);
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    assert!(rig.tool_pid(&pane, "claude").is_some());
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
    assert!(
        rig.meta_row("harness_session.spawned.0").ends_with(&id),
        "same conversation resumed"
    );
    assert_eq!(
        std::fs::read(rig.scratch.join("home-a").join(&relative)).expect("original"),
        std::fs::read(rig.scratch.join("home-b").join(&relative)).expect("carried")
    );
    assert!(
        booked(&rig, DONE_ACTION)[0]
            .summary
            .as_deref()
            .unwrap_or_default()
            .contains("carried")
    );
    assert!(
        !rig.dir.join("seed.scout.md").exists(),
        "carry uses no seed"
    );
}

#[test]
fn headroom_grace_starts_at_the_first_crossing_record() {
    let rig = Rig::new("hrgrace");
    configure(&rig, "", "fake-claude", "fake-opencode");
    let path = rig.scratch.join("config");
    let text = std::fs::read_to_string(&path).expect("config");
    std::fs::write(
        path,
        text.replace("auto_reseat_grace_secs = 0", "auto_reseat_grace_secs = 6"),
    )
    .expect("grace");
    quota(&rig, "home", &[("weekly_all", 95.0)], 0);
    let pane = seat(&rig, "claude");
    let pid = rig.tool_pid(&pane, "claude");
    let _watch = watch(&rig);
    until(&rig, || !auto_records(&rig).is_empty());
    let opened = auto_records(&rig)[0].ts;
    std::thread::sleep(Duration::from_secs(2));
    unchanged(&rig, &pane, "fake-claude", pid);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    let attempt = &booked(&rig, ATTEMPT_ACTION)[0];
    assert!(attempt.ts.epoch() >= opened.epoch() + 6, "grace honoured");
    assert_eq!(
        attempt.reference.as_deref(),
        Some(opened.to_string().as_str())
    );
}

#[test]
fn threshold_config_is_bounded_global_default_and_bad_values_note_once() {
    for raw in [
        "49",
        "101",
        "95.5",
        "nope",
        "",
        "-1",
        "18446744073709551616",
    ] {
        let text = format!(
            "[workspace]\nauto_reseat = on\nauto_reseat_at = {raw}\n[auto_reseat]\na = b\n"
        );
        let settings = settings_in(Path::new("fixture-config"), &text);
        assert_eq!(
            settings.notes.len(),
            1,
            "bad threshold {raw:?}: {settings:?}"
        );
        assert!(settings.notes[0].contains("auto_reseat_at"));
        assert!(settings.notes[0].contains("95"), "fallback named");
    }
    let duplicate = settings_in(
        Path::new("fixture-config"),
        "[workspace]\nauto_reseat = on\nauto_reseat_at = 60\nauto_reseat_at = 80\n[auto_reseat]\na = b\n",
    );
    assert_eq!(
        duplicate.notes.len(),
        1,
        "duplicate threshold defaults visibly"
    );
    for raw in ["50", "95", "100", "off"] {
        let text = format!("[workspace]\nauto_reseat = on\nauto_reseat_at = {raw}\n");
        assert!(
            settings_in(Path::new("fixture-config"), &text)
                .notes
                .is_empty()
        );
    }
    let off = settings_in(
        Path::new("fixture-config"),
        "[workspace]\nauto_reseat = off\nauto_reseat_at = nonsense\n",
    );
    assert!(off.notes.is_empty(), "off reads nothing behind switch");
}

#[test]
fn either_weekly_or_session_crossing_moves_an_idle_seat_before_a_limit() {
    for (tag, windows, named) in [
        (
            "hrweekly",
            vec![("session", 5.0), ("weekly_all", 95.0)],
            "weekly",
        ),
        (
            "hrsession",
            vec![("session", 95.0), ("weekly_all", 5.0)],
            "session",
        ),
    ] {
        let rig = Rig::new(tag);
        configure(&rig, "", "fake-claude", "fake-opencode");
        quota(&rig, "home", &windows, 0);
        let pane = seat(&rig, "claude");
        let _watch = watch(&rig);
        until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
        assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
        assert!(rig.tool_pid(&pane, "opencode").is_some(), "moved in place");
        assert!(
            booked(&rig, "limit").is_empty(),
            "no vendor refusal required"
        );
        let path = auto_records(&rig);
        let first = path.first().expect("first crossing record");
        let attempt = booked(&rig, ATTEMPT_ACTION);
        assert_eq!(attempt.len(), 1, "one attempt");
        assert_eq!(
            attempt[0].reference.as_deref(),
            Some(first.ts.to_string().as_str())
        );
        let summary = attempt[0].summary.as_deref().unwrap_or_default();
        assert!(
            summary.contains("headroom") && summary.contains("95") && summary.contains(named),
            "{summary}"
        );
        let notices: Vec<_> = records(&rig)
            .into_iter()
            .filter(|e| e.action == "chat")
            .collect();
        assert_eq!(notices.len(), 1, "one human notice");
        assert!(
            notices[0]
                .summary
                .as_deref()
                .unwrap_or_default()
                .contains("headroom")
        );
        assert!(
            rig.dir.join("seed.scout.md").exists(),
            "tool change uses existing seed path"
        );
        let ours = records(&rig);
        let started = ours
            .iter()
            .position(|e| e.action == ATTEMPT_ACTION)
            .expect("attempt");
        let ended = ours
            .iter()
            .position(|e| e.action == DONE_ACTION)
            .expect("done");
        assert!(
            !ours[started..ended]
                .iter()
                .any(|e| e.action == "alert" || e.action == "alert-cleared"),
            "respawn shell is no death or limit release"
        );
    }
}

#[test]
fn configurable_crossing_is_inclusive_and_absence_defaults_to_95() {
    for (tag, knob, percent, moves) in [
        ("hrbelow", "", 94.9, false),
        ("hrcustom", "auto_reseat_at = 96", 96.0, true),
        ("hrhigher", "auto_reseat_at = 98", 96.0, false),
        ("hrlower", "auto_reseat_at = 50", 50.0, true),
        ("hrbad", "auto_reseat_at = bad", 95.0, true),
        ("hrdisabled", "auto_reseat_at = off", 99.0, false),
    ] {
        let rig = Rig::new(tag);
        configure(&rig, knob, "fake-claude", "fake-opencode");
        quota(&rig, "home", &[("weekly_all", percent)], 0);
        let pane = seat(&rig, "claude");
        let pid = rig.tool_pid(&pane, "claude");
        let _watch = watch(&rig);
        if moves {
            until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
            if tag == "hrbad" {
                let notes: Vec<_> = auto_records(&rig)
                    .into_iter()
                    .filter(|e| {
                        e.target.is_none()
                            && e.summary
                                .as_deref()
                                .is_some_and(|s| s.contains("auto_reseat_at"))
                    })
                    .collect();
                assert_eq!(
                    notes.len(),
                    1,
                    "fallback visibly noted once: {}",
                    rig.events()
                );
                assert_eq!(notes[0].action, "auto-reseat-notice");
            }
        } else {
            sweeps(&rig);
            unchanged(&rig, &pane, "fake-claude", pid);
            assert!(
                auto_records(&rig).is_empty(),
                "no episode below threshold or off"
            );
        }
    }
}

#[test]
fn a_high_unrelated_account_does_not_move_a_seat_on_its_own_low_scope() {
    let rig = Rig::new("hrownlow");
    configure(&rig, "", "fake-claude-a", "fake-opencode");
    quota(&rig, "home", &[("weekly_all", 100.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 99.0)], 0);
    quota(&rig, "home-a", &[("weekly_all", 10.0)], 0);
    let pane = seat(&rig, "claude-a");
    let pid = rig.tool_pid(&pane, "claude");
    let _watch = watch(&rig);
    sweeps(&rig);
    unchanged(&rig, &pane, "fake-claude-a", pid);
    assert!(
        auto_records(&rig).is_empty(),
        "own scope alone governs crossing"
    );
}

#[test]
fn stale_numbers_still_bind_but_a_passed_reset_does_not() {
    for (tag, age, reset_in, moves) in [
        ("hrstale", 901, 86_400, true),
        ("hrreset", 100_000, -1, false),
    ] {
        let rig = Rig::new(tag);
        configure(&rig, "", "fake-claude", "fake-opencode");
        quota_reset(&rig, "home", &[("weekly_all", 99.0)], age, reset_in);
        let pane = seat(&rig, "claude");
        let pid = rig.tool_pid(&pane, "claude");
        let _watch = watch(&rig);
        if moves {
            until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
        } else {
            sweeps(&rig);
            unchanged(&rig, &pane, "fake-claude", pid);
        }
    }
}

#[test]
fn a_candidate_at_headroom_is_skipped_for_an_unknown_candidate_and_named() {
    let rig = Rig::new("hrcandidate");
    configure(
        &rig,
        "",
        "fake-claude",
        "fake-claude-a, fake-claude-b, fake-opencode",
    );
    quota(&rig, "home", &[("weekly_all", 96.0)], 0);
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("session", 96.0)], 900);
    let pane = seat(&rig, "claude");
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    assert!(rig.tool_pid(&pane, "opencode").is_some());
    let summaries = auto_records(&rig)
        .into_iter()
        .filter_map(|e| e.summary)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        summaries.contains("fake-claude-a") && summaries.contains("fake-claude-b"),
        "skips named: {summaries}"
    );
    assert!(
        summaries.to_lowercase().contains("headroom"),
        "skip reason named: {summaries}"
    );
}

#[test]
fn busy_and_draft_holds_do_not_spend_attempts_and_move_after_idle() {
    for (tag, marker, draft) in [
        ("hrbusy", "__BUSY__", false),
        ("hrdraft", "__DRAFT__", true),
    ] {
        let rig = Rig::new(tag);
        configure(&rig, "", "fake-claude", "fake-opencode");
        quota(&rig, "home", &[("weekly_all", 95.0)], 0);
        let pane = seat(&rig, "claude");
        if draft {
            rig.mark_draft(&pane);
        } else {
            rig.mark_busy(&pane);
        }
        let pid = rig.tool_pid(&pane, "claude");
        let _watch = watch(&rig);
        until(&rig, || booked(&rig, HELD_ACTION).len() == 1);
        sweeps(&rig);
        unchanged(&rig, &pane, "fake-claude", pid);
        assert_eq!(booked(&rig, HELD_ACTION).len(), 1, "one hold per episode");
        std::fs::remove_file(rig.scratch.join(marker)).expect("release fixture hold");
        until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
        assert_eq!(booked(&rig, ATTEMPT_ACTION).len(), 1);
    }
}

#[test]
fn a_readable_unknown_frame_holds_headroom_instead_of_stopping_the_tool() {
    let rig = Rig::new("hrunknown");
    configure(&rig, "", "fake-claude", "fake-opencode");
    quota(&rig, "home", &[("weekly_all", 99.0)], 0);
    let script = rig.scratch.join("claude.pl");
    let original = std::fs::read_to_string(&script).expect("fixture script");
    assert!(original.contains("my $frame = 1;"));
    std::fs::write(
        &script,
        original.replace("my $frame = 1;", "my $frame = 0;"),
    )
    .expect("unknown frame fixture");
    let pane = seat(&rig, "claude");
    let pid = rig.tool_pid(&pane, "claude");
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, HELD_ACTION).len() == 1);
    sweeps(&rig);
    unchanged(&rig, &pane, "fake-claude", pid);
    assert_eq!(booked(&rig, HELD_ACTION).len(), 1);
}

#[test]
fn no_candidate_refuses_once_and_jitter_survives_a_daemon_restart() {
    let rig = Rig::new("hrjitter");
    configure(&rig, "", "fake-claude", "fake-claude-a");
    phase1_remove_unused_profile(&rig, "fake-claude-b");
    quota(&rig, "home", &[("weekly_all", 95.1)], 0);
    quota(&rig, "home-a", &[("weekly_all", 96.0)], 0);
    let _pane = seat(&rig, "claude");
    {
        let _watch = watch(&rig);
        until(&rig, || booked(&rig, REFUSED_ACTION).len() == 1);
        sweeps(&rig);
        assert_eq!(booked(&rig, REFUSED_ACTION).len(), 1);
    }
    for percent in [94.9, 95.1] {
        quota(&rig, "home", &[("weekly_all", percent)], 0);
        let _watch = watch(&rig);
        sweeps(&rig);
        assert_eq!(
            booked(&rig, REFUSED_ACTION).len(),
            1,
            "jitter cannot reopen terminal"
        );
    }
    // Relief has to be strictly below threshold - 5 before a new crossing.
    quota(&rig, "home", &[("weekly_all", 89.9)], 0);
    {
        let _watch = watch(&rig);
        sweeps(&rig);
    }
    quota(&rig, "home", &[("weekly_all", 95.1)], 0);
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, REFUSED_ACTION).len() == 2);
}

#[test]
fn an_open_episode_keeps_its_first_crossing_key_until_strict_relief() {
    let rig = Rig::new("hropen");
    configure(&rig, "", "fake-claude", "fake-opencode");
    quota(&rig, "home", &[("weekly_all", 95.1)], 0);
    let pane = seat(&rig, "claude");
    rig.mark_busy(&pane);
    {
        let _watch = watch(&rig);
        until(&rig, || booked(&rig, HELD_ACTION).len() == 1);
    }
    let key = booked(&rig, HELD_ACTION)[0].reference.clone();
    quota(&rig, "home", &[("weekly_all", 90.0)], 0);
    {
        let _watch = watch(&rig);
        sweeps(&rig);
        assert_eq!(
            booked(&rig, HELD_ACTION).len(),
            1,
            "90 is not strict relief"
        );
    }
    quota(&rig, "home", &[("weekly_all", 94.9)], 0);
    std::fs::remove_file(rig.scratch.join("__BUSY__")).expect("idle fixture");
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    assert_eq!(
        booked(&rig, ATTEMPT_ACTION)[0].reference,
        key,
        "restart keeps first crossing key"
    );
}

#[test]
fn a_profile_left_on_headroom_cannot_ping_pong_on_an_old_low_reading() {
    let rig = Rig::new("hrreturn");
    configure(&rig, "", "fake-claude-a", "fake-claude-b");
    phase1_remove_unused_profile(&rig, "fake-claude");
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
    let _pane = seat(&rig, "claude-a");
    {
        let _watch = watch(&rig);
        until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    }
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
    configure(&rig, "", "fake-claude-b", "fake-claude-a");
    quota(&rig, "home-a", &[("weekly_all", 20.0)], 100);
    quota(&rig, "home-b", &[("weekly_all", 95.0)], 0);
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, REFUSED_ACTION).len() == 1);
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "fake-claude-b",
        "old cache cannot prove relief"
    );
    assert_eq!(booked(&rig, DONE_ACTION).len(), 1, "one move");
    let summaries = auto_records(&rig)
        .into_iter()
        .filter_map(|e| e.summary)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        summaries.contains("fake-claude-a") && summaries.to_lowercase().contains("headroom"),
        "{summaries}"
    );
}

#[test]
fn post_move_relief_uses_the_configured_threshold_even_when_the_number_is_critical() {
    let rig = Rig::new("hrrelief100");
    configure(
        &rig,
        "auto_reseat_at = 100",
        "fake-claude-a",
        "fake-claude-b",
    );
    quota(&rig, "home-a", &[("weekly_all", 100.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
    let _pane = seat(&rig, "claude-a");
    {
        let _watch = watch(&rig);
        until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    }
    std::thread::sleep(Duration::from_millis(1_100));
    // The fake never writes its newly minted conversation. A real Claude
    // seat has this source transcript before a later account carry checks it.
    let current = rig.meta_row("harness_session.spawned.0");
    let id = current.strip_prefix("claude:").unwrap_or(&current);
    let work = std::path::PathBuf::from(rig.meta_row("work_dir"));
    let transcript = rig
        .scratch
        .join("home-b")
        .join("projects")
        .join(ae::carry::project_key(&work))
        .join(format!("{id}.jsonl"));
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("source project");
    std::fs::write(transcript, b"{\"type\":\"user\"}\n").expect("synthetic return conversation");
    configure(
        &rig,
        "auto_reseat_at = 100",
        "fake-claude-b",
        "fake-claude-a",
    );
    quota(&rig, "home-a", &[("weekly_all", 99.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 100.0)], 0);
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 2);
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "fake-claude-a",
        "fresh99 belowT100 proves relief"
    );
}

#[test]
fn global_off_session_filter_and_orchestrator_make_headroom_inert() {
    for (tag, knobs, meta) in [
        ("hroff", "auto_reseat = off", None),
        ("hrfilter", "auto_reseat_sessions = other", None),
        ("hrmeta", "", Some(("meta_agent", "true"))),
    ] {
        let rig = Rig::new(tag);
        configure(&rig, knobs, "fake-claude", "fake-opencode");
        if let Some((key, value)) = meta {
            let path = rig.dir.join("meta");
            let text = std::fs::read_to_string(&path).expect("meta");
            std::fs::write(path, format!("{text}{key}={value}\n")).expect("orchestrator marker");
        }
        quota(&rig, "home", &[("weekly_all", 99.0)], 0);
        let pane = seat(&rig, "claude");
        let pid = rig.tool_pid(&pane, "claude");
        let _watch = watch(&rig);
        sweeps(&rig);
        unchanged(&rig, &pane, "fake-claude", pid);
        assert!(auto_records(&rig).is_empty(), "ineligible books no episode");
        assert!(
            !rig.dir.join("auto-reseat.spawned.0.lock").exists(),
            "ineligible takes no lock"
        );
    }
}

#[test]
fn main_requires_all_and_project_overlay_cannot_change_global_threshold() {
    for (tag, switch, moves) in [("hrmainon", "on", false), ("hrmainall", "all", true)] {
        let rig = Rig::new(tag);
        configure(&rig, "auto_reseat_at = 98", "fake-claude", "fake-opencode");
        let config = rig.scratch.join("config");
        let text = std::fs::read_to_string(&config).expect("config");
        std::fs::write(
            config,
            text.replace("auto_reseat = on", &format!("auto_reseat = {switch}")),
        )
        .expect("switch");
        rebind(&rig, "profile.main", "fake-claude");
        rebind(&rig, "agent_bin.main", "claude");
        std::fs::create_dir_all(rig.scratch.join(".ae")).expect("overlay directory");
        std::fs::write(
            rig.scratch.join(".ae/config"),
            "[workspace]\nauto_reseat_at = off\n",
        )
        .expect("project overlay");
        quota(&rig, "home", &[("weekly_all", 98.0)], 0);
        isolate(&rig, &rig.main_pane);
        rig.start(&rig.main_pane, "main", "claude");
        let pid = rig.tool_pid(&rig.main_pane, "claude");
        let _watch = watch(&rig);
        if moves {
            until(&rig, || rig.meta_row("profile.main") == "fake-opencode");
        } else {
            sweeps(&rig);
            assert_eq!(rig.tool_pid(&rig.main_pane, "claude"), pid);
            assert!(auto_records(&rig).is_empty(), "on excludes main");
        }
    }
}

// Phase 2: independent R8-R11 acceptance contract. Only these scratch profiles
// enter the identity config, so unrelated Rig defaults cannot supply a twin.
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

fn family_command(rig: &Rig, home: &str, flags: &str) -> String {
    format!(
        "CLAUDE_CONFIG_DIR={} {} {} {flags}",
        rig.scratch.join(home).display(),
        rig.scratch.join("tools/claude").display(),
        rig.scratch.join("claude.pl").display(),
    )
}

fn family_config(rig: &Rig, profiles: &[(&str, String)], row: Option<&str>, knobs: &str) {
    let mut text = String::from("[profiles]\n");
    for (name, command) in profiles {
        writeln!(text, "fake-{name} = \"{command}\"").expect("profile fixture");
    }
    writeln!(text, "[workspace]\nmain = lead\nlayout = vertical").expect("workspace fixture");
    if !knobs.lines().any(|line| line.starts_with("auto_reseat =")) {
        text.push_str("auto_reseat = on\n");
    }
    writeln!(text, "auto_reseat_grace_secs = 0\n{knobs}").expect("policy fixture");
    if let Some(row) = row {
        writeln!(text, "[auto_reseat]\nfake-source = {row}").expect("declared row fixture");
    }
    std::fs::write(rig.scratch.join("config"), text).expect("family fixture config");
}

fn family_source(rig: &Rig) -> String {
    let pane = seat(rig, "source");
    plant_conversation(rig);
    pane
}

fn family_open(rig: &Rig, pane: &str, limit: bool) -> Timestamp {
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
    if limit {
        rig.mark_limited(pane);
        seed(rig, key, "limit", None);
    } else {
        seed(rig, key, "auto-reseat-headroom", None);
    }
    key
}

fn family_move(rig: &Rig, pane: &str, limit: bool, to: &str, provenance: &str) {
    let key = family_open(rig, pane, limit);
    let out = trigger(rig);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the real trigger must list this candidate: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        rig.events(),
    );
    until(rig, || !booked(rig, DONE_ACTION).is_empty());
    assert_eq!(rig.meta_row("profile.spawned.0"), to);
    let tool = if to == "fake-opencode" {
        "opencode"
    } else {
        "claude"
    };
    assert!(
        rig.tool_pid(pane, tool).is_some(),
        "the chosen tool owns the seat"
    );
    let attempts = booked(rig, ATTEMPT_ACTION);
    assert_eq!(attempts.len(), 1, "{}", rig.events());
    assert_eq!(
        attempts[0].reference.as_deref(),
        Some(key.to_string().as_str())
    );
    let suffix = if provenance.is_empty() {
        String::new()
    } else {
        format!(", derived {provenance}")
    };
    assert!(
        attempts[0].summary.as_deref().is_some_and(
            |summary| summary.starts_with(&format!("from fake-source to {to}{suffix}"))
        ),
        "attempt names selection and provenance: {}",
        rig.events(),
    );
    let done = booked(rig, DONE_ACTION);
    assert_eq!(done.len(), 1);
    assert!(
        done[0]
            .summary
            .as_deref()
            .is_some_and(|summary| summary.starts_with(&format!("to {to}{suffix}"))),
        "done names the same provenance: {}",
        rig.events(),
    );
    until(rig, || {
        records(rig).iter().any(|event| {
            event.action == "chat"
                && event.summary.as_deref().is_some_and(|summary| {
                    summary.starts_with(&format!(
                        "auto reseat: scout moved fake-source -> {to}{suffix}"
                    ))
                })
        })
    });
}

#[test]
fn phase2_a_declared_pick_keeps_phase1_record_bytes() {
    let rig = Rig::new("hr2declared");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("twin", family_command(&rig, "home-b", "--model opus")),
        ],
        Some("fake-twin"),
        "",
    );
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
    let pane = family_source(&rig);
    family_move(&rig, &pane, false, "fake-twin", "");
    assert_eq!(
        booked(&rig, ATTEMPT_ACTION)[0].summary.as_deref(),
        Some("from fake-source to fake-twin (headroom 95% weekly_all 7d)"),
    );
    assert_eq!(
        booked(&rig, DONE_ACTION)[0].summary.as_deref(),
        Some("to fake-twin, carried (headroom 95% weekly_all 7d)"),
    );
}

#[test]
fn phase2_a_twin_without_a_declared_row_moves_and_carries_at_headroom() {
    let rig = Rig::new("hr2daemon");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("twin", family_command(&rig, "home-b", "--model opus")),
        ],
        None,
        "",
    );
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
    let pane = family_source(&rig);
    let id = rig.meta_row("harness_session.spawned.0");
    let transcript = rig
        .scratch
        .join("home-a/projects")
        .join(ae::carry::project_key(Path::new(&rig.meta_row("work_dir"))))
        .join(format!("{id}.jsonl"));
    let original = std::fs::read(&transcript).expect("synthetic source transcript");
    let _watch = watch(&rig);
    until(&rig, || booked(&rig, DONE_ACTION).len() == 1);
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-twin");
    assert!(rig.tool_pid(&pane, "claude").is_some());
    assert_eq!(rig.meta_row("harness_session.spawned.0"), id);
    let relative = transcript
        .strip_prefix(rig.scratch.join("home-a"))
        .expect("relative store");
    assert_eq!(
        std::fs::read(rig.scratch.join("home-b").join(relative)).expect("carried transcript"),
        original
    );
    assert!(
        booked(&rig, ATTEMPT_ACTION)[0]
            .summary
            .as_deref()
            .is_some_and(|s| s.contains("to fake-twin, derived twin (headroom"))
    );
    assert_eq!(
        booked(&rig, DONE_ACTION)[0].summary.as_deref(),
        Some("to fake-twin, derived twin, carried (headroom 95% weekly_all 7d)")
    );
    until(&rig, || {
        records(&rig).iter().any(|event| {
            event.action == "chat"
                && event.summary.as_deref().is_some_and(|s| {
                    s.contains("scout moved fake-source -> fake-twin, derived twin on headroom")
                })
        })
    });
}

#[test]
fn phase2_twins_precede_siblings_and_keep_declaration_order() {
    for (tag, first_used, chosen) in [
        ("hr2tierfirst", 10.0, "fake-z"),
        ("hr2tiernext", 96.0, "fake-a"),
    ] {
        let rig = Rig::new(tag);
        family_config(
            &rig,
            &[
                ("source", family_command(&rig, "home-a", "--model opus")),
                ("sibling", family_command(&rig, "home-b", "--model sonnet")),
                ("z", family_command(&rig, "home-c", "--model opus")),
                ("a", family_command(&rig, "home-d", "--model opus")),
            ],
            None,
            "",
        );
        for (home, used) in [
            ("home-a", 95.0),
            ("home-b", 10.0),
            ("home-c", first_used),
            ("home-d", 10.0),
        ] {
            quota(&rig, home, &[("weekly_all", used)], 0);
        }
        let pane = family_source(&rig);
        family_move(&rig, &pane, false, chosen, "twin");
        if first_used > 95.0 {
            assert!(
                booked(&rig, ATTEMPT_ACTION)[0]
                    .summary
                    .as_deref()
                    .is_some_and(|s| s.contains("fake-z, derived twin (no headroom)")),
                "{}",
                rig.events()
            );
        }
    }
}

#[test]
fn phase2_siblings_keep_declaration_order_after_model_value_normalization() {
    let rig = Rig::new("hr2siblings");
    family_config(
        &rig,
        &[
            (
                "source",
                family_command(&rig, "home-a", "--model=opus --effort high"),
            ),
            (
                "z",
                family_command(&rig, "home-b", "--model sonnet --effort high"),
            ),
            (
                "a",
                family_command(&rig, "home-c", "--model haiku --effort high"),
            ),
        ],
        None,
        "",
    );
    for (home, used) in [("home-a", 95.0), ("home-b", 10.0), ("home-c", 10.0)] {
        quota(&rig, home, &[("weekly_all", used)], 0);
    }
    let pane = family_source(&rig);
    family_move(&rig, &pane, false, "fake-z", "sibling");
}

#[test]
fn phase2_pinless_profiles_can_have_twins() {
    let rig = Rig::new("hr2pinless");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "")),
            ("twin", family_command(&rig, "home-b", "")),
        ],
        None,
        "",
    );
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
    let pane = family_source(&rig);
    family_move(&rig, &pane, false, "fake-twin", "twin");
}

#[test]
fn phase2_only_the_model_value_and_account_may_differ() {
    for (tag, bad_flags, assignment) in [
        (
            "hr2effort",
            "--model opus --effort low --permission-mode acceptEdits",
            "",
        ),
        (
            "hr2permission",
            "--model opus --effort high --permission-mode plan",
            "",
        ),
        (
            "hr2extra",
            "--model opus --effort high --permission-mode acceptEdits --verbose",
            "",
        ),
        (
            "hr2reorder",
            "--effort high --model opus --permission-mode acceptEdits",
            "",
        ),
        (
            "hr2env",
            "--model opus --effort high --permission-mode acceptEdits",
            "OTHER=fixture ",
        ),
        (
            "hr2ambiguous",
            "--model opus --model sonnet --effort high --permission-mode acceptEdits",
            "",
        ),
        (
            "hr2missing",
            "--effort high --permission-mode acceptEdits --model",
            "",
        ),
    ] {
        let rig = Rig::new(tag);
        let source_flags = "--model opus --effort high --permission-mode acceptEdits";
        family_config(
            &rig,
            &[
                ("source", family_command(&rig, "home-a", source_flags)),
                (
                    "bad",
                    format!("{assignment}{}", family_command(&rig, "home-b", bad_flags)),
                ),
                ("twin", family_command(&rig, "home-c", source_flags)),
            ],
            None,
            "",
        );
        for (home, used) in [("home-a", 95.0), ("home-b", 10.0), ("home-c", 10.0)] {
            quota(&rig, home, &[("weekly_all", used)], 0);
        }
        let pane = family_source(&rig);
        family_move(&rig, &pane, false, "fake-twin", "twin");
        assert!(
            !booked(&rig, ATTEMPT_ACTION)[0]
                .summary
                .as_deref()
                .unwrap_or_default()
                .contains("fake-bad"),
            "non-family profile must not enter the passed-over list"
        );
    }
}

#[test]
fn phase2_declared_cross_tool_choices_precede_derived_family_choices() {
    let rig = Rig::new("hr2crossdeclared");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("twin", family_command(&rig, "home-b", "--model opus")),
            (
                "opencode",
                format!(
                    "{} {}",
                    rig.scratch.join("tools/opencode").display(),
                    rig.scratch.join("opencode.pl").display()
                ),
            ),
        ],
        Some("fake-opencode"),
        "",
    );
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
    let pane = family_source(&rig);
    family_move(&rig, &pane, false, "fake-opencode", "");
    assert!(
        !rig.events().contains("derived"),
        "declared provenance retains old spelling"
    );
}

#[test]
fn phase2_declared_dedup_wins_and_room_rules_fall_through_to_derived() {
    let rig = Rig::new("hr2declaredskip");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("first", family_command(&rig, "home-b", "--model opus")),
            ("twin", family_command(&rig, "home-c", "--model opus")),
        ],
        Some("fake-first"),
        "",
    );
    for (home, used) in [("home-a", 95.0), ("home-b", 100.0), ("home-c", 10.0)] {
        quota(&rig, home, &[("weekly_all", used)], 0);
    }
    let pane = family_source(&rig);
    family_move(&rig, &pane, false, "fake-twin", "twin");
    let attempts = booked(&rig, ATTEMPT_ACTION);
    let summary = attempts[0].summary.as_deref().expect("attempt text");
    assert_eq!(
        summary.matches("fake-first").count(),
        1,
        "name dedup: {summary}"
    );
    assert!(
        summary.contains("fake-first (exhausted)"),
        "declared position and provenance win: {summary}"
    );
}

#[test]
fn phase2_limit_episodes_also_derive_and_keep_the_old_critical_room_rule() {
    let rig = Rig::new("hr2limit");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("twin", family_command(&rig, "home-b", "--model opus")),
        ],
        None,
        "",
    );
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 96.0)], 0);
    let pane = family_source(&rig);
    family_move(&rig, &pane, true, "fake-twin", "twin");
    assert_eq!(
        booked(&rig, ATTEMPT_ACTION)[0].summary.as_deref(),
        Some("from fake-source to fake-twin, derived twin")
    );
    assert_eq!(
        booked(&rig, DONE_ACTION)[0].summary.as_deref(),
        Some("to fake-twin, derived twin, carried")
    );
}

#[test]
fn phase2_empty_or_ignored_rows_derive_without_erasing_the_existing_note() {
    for (tag, row) in [("hr2empty", ""), ("hr2ignored", "bad?profile")] {
        let rig = Rig::new(tag);
        family_config(
            &rig,
            &[
                ("source", family_command(&rig, "home-a", "--model opus")),
                ("twin", family_command(&rig, "home-b", "--model opus")),
            ],
            Some(row),
            "",
        );
        let text = std::fs::read_to_string(rig.scratch.join("config")).expect("fixture config");
        assert!(
            !settings_in(Path::new("scratch-config"), &text)
                .notes
                .is_empty(),
            "ignored row retains its note"
        );
        quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
        quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
        let pane = family_source(&rig);
        family_move(&rig, &pane, false, "fake-twin", "twin");
    }
}

#[test]
fn phase2_default_off_and_explicit_off_do_not_open_derived_episodes() {
    for (tag, absent) in [("hr2defaultoff", true), ("hr2off", false)] {
        let rig = Rig::new(tag);
        family_config(
            &rig,
            &[
                ("source", family_command(&rig, "home-a", "--model opus")),
                ("twin", family_command(&rig, "home-b", "--model opus")),
            ],
            None,
            "auto_reseat = off",
        );
        if absent {
            let path = rig.scratch.join("config");
            let text = std::fs::read_to_string(&path).expect("fixture config");
            std::fs::write(path, text.replace("auto_reseat = off\n", ""))
                .expect("absent product switch");
        }
        quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
        quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
        let pane = family_source(&rig);
        let pid = rig.tool_pid(&pane, "claude");
        let _watch = watch(&rig);
        sweeps(&rig);
        unchanged(&rig, &pane, "fake-source", pid);
        assert!(auto_records(&rig).is_empty());
        assert!(!ae::autoreseat::lock_path(&rig.dir, "spawned.0").exists());
    }
}

#[test]
fn phase2_a_leg_with_no_declared_or_derived_candidate_names_the_hold() {
    let rig = Rig::new("hr2unmappedleg");
    family_config(
        &rig,
        &[("source", family_command(&rig, "home-a", "--model opus"))],
        None,
        "",
    );
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    let pane = family_source(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let key = family_open(&rig, &pane, false);
    seed(&rig, key, ATTEMPT_ACTION, Some(key));
    let out = leg(&rig, key);
    assert_eq!(out.status.code(), Some(1));
    let held = booked(&rig, HELD_ACTION);
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].reference.as_deref(), Some(key.to_string().as_str()));
    assert_eq!(
        held[0].summary.as_deref(),
        Some(
            "held: no declared or derived candidate for this profile (headroom 95% weekly_all 7d)"
        )
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-source");
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    assert!(booked(&rig, DONE_ACTION).is_empty());
    assert!(booked(&rig, REFUSED_ACTION).is_empty());
}

fn family_row(rig: &Rig, key: &str, value: &str) {
    let text = rig.meta();
    let prefix = format!("{key}=");
    let mut kept = String::new();
    for line in text.lines().filter(|line| !line.starts_with(&prefix)) {
        writeln!(kept, "{line}").expect("fixture row");
    }
    writeln!(kept, "{key}={value}").expect("single fixture row");
    std::fs::write(rig.dir.join("meta"), kept).expect("fixture metadata");
}

#[test]
fn phase2_a_same_account_sibling_can_escape_a_different_models_window() {
    let rig = Rig::new("hr2scoped");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("sibling", family_command(&rig, "home-a", "--model sonnet")),
        ],
        None,
        "",
    );
    quota(
        &rig,
        "home-a",
        &[("weekly_scoped", 95.0), ("weekly_all", 10.0)],
        0,
    );
    let cache = rig.scratch.join("home-a/.claude.json");
    let text = std::fs::read_to_string(&cache).expect("scratch quota");
    // Only the first, model-scoped window gets a qualifier. The account-wide
    // window stays unqualified and continues to bind both model families.
    std::fs::write(
        cache,
        text.replacen(
            "\"scope\":null",
            "\"scope\":{\"model\":{\"display_name\":\"Opus\"}}",
            1,
        ),
    )
    .expect("model-scoped fixture");
    let pane = family_source(&rig);
    family_move(&rig, &pane, false, "fake-sibling", "sibling");
}

#[test]
fn phase2_a_canonical_alias_of_the_seats_account_is_not_a_candidate() {
    let rig = Rig::new("hr2seatalias");
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-c", &[("weekly_all", 10.0)], 0);
    std::os::unix::fs::symlink(rig.scratch.join("home-a"), rig.scratch.join("alias-a"))
        .expect("scratch account alias");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("same", family_command(&rig, "alias-a", "--model opus")),
            ("twin", family_command(&rig, "home-c", "--model opus")),
        ],
        None,
        "",
    );
    let pane = family_source(&rig);
    family_move(&rig, &pane, false, "fake-twin", "twin");
    assert!(
        !booked(&rig, ATTEMPT_ACTION)[0]
            .summary
            .as_deref()
            .expect("attempt")
            .contains("fake-same"),
        "an alias must be excluded, not offered then skipped"
    );
}

#[test]
fn phase2_proven_derived_aliases_collapse_against_the_earliest_candidate() {
    for (tag, row) in [
        ("hr2aliasderived", None),
        ("hr2aliasdeclared", Some("fake-b")),
    ] {
        let rig = Rig::new(tag);
        for (home, used) in [("home-a", 95.0), ("home-b", 95.0), ("home-c", 10.0)] {
            quota(&rig, home, &[("weekly_all", used)], 0);
        }
        std::os::unix::fs::symlink(rig.scratch.join("home-b"), rig.scratch.join("alias-b"))
            .expect("scratch account alias");
        family_config(
            &rig,
            &[
                ("source", family_command(&rig, "home-a", "--model opus")),
                ("b", family_command(&rig, "home-b", "--model opus")),
                ("dup", family_command(&rig, "alias-b", "--model opus")),
                ("c", family_command(&rig, "home-c", "--model opus")),
            ],
            row,
            "",
        );
        let pane = family_source(&rig);
        family_move(&rig, &pane, false, "fake-c", "twin");
        let attempts = booked(&rig, ATTEMPT_ACTION);
        let summary = attempts[0].summary.as_deref().expect("attempt");
        assert_eq!(summary.matches("fake-b").count(), 1);
        assert!(
            !summary.contains("fake-dup"),
            "proven alias collapse: {summary}"
        );
        assert!(
            summary.contains(if row.is_some() {
                "fake-b (no headroom)"
            } else {
                "fake-b, derived twin (no headroom)"
            }),
            "earliest provenance wins: {summary}"
        );
    }
}

#[test]
fn phase2_unknown_accounts_do_not_collapse_as_if_none_were_an_identity() {
    for (tag, row) in [
        ("hr2unknownpair", None),
        ("hr2unknowndeclared", Some("fake-first")),
    ] {
        let rig = Rig::new(tag);
        // Missing normal paths have provable canonical ancestors. Dangling
        // links make these two distinct scratch accounts truly unprovable.
        for name in ["first", "second"] {
            std::os::unix::fs::symlink(
                rig.scratch.join(format!("missing-{name}")),
                rig.scratch.join(format!("unknown-{name}")),
            )
            .expect("unprovable scratch account");
        }
        family_config(
            &rig,
            &[
                ("source", family_command(&rig, "home-a", "--model opus")),
                (
                    "first",
                    family_command(&rig, "unknown-first", "--model sonnet"),
                ),
                (
                    "second",
                    family_command(&rig, "unknown-second", "--model sonnet"),
                ),
            ],
            row,
            "",
        );
        quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
        let pane = family_source(&rig);
        let old = Timestamp::from_epoch(Timestamp::now().epoch() - 100);
        seed(&rig, old, "limit", None);
        seed(&rig, old, ATTEMPT_ACTION, Some(old));
        let done = format!(
            "{}{{\"ts\":\"{}\",\"actor\":\"watchdog\",\"action\":\"{DONE_ACTION}\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session\":\"{}\",\"ref\":\"fake-first\",\"summary\":\"to fake-source, seeded\"}}\n",
            rig.events(),
            Timestamp::from_epoch(old.epoch() + 1),
            rig.session
        );
        std::fs::write(rig.dir.join("events.jsonl"), done).expect("prior limit leave");
        let key = family_open(&rig, &pane, false);
        let out = trigger(&rig);
        assert_eq!(
            out.status.code(),
            Some(0),
            "the second unknown account must remain listed: {}\n{}",
            String::from_utf8_lossy(&out.stderr),
            rig.events()
        );
        let attempts = booked(&rig, ATTEMPT_ACTION);
        let selected = attempts
            .iter()
            .find(|event| event.reference.as_deref() == Some(key.to_string().as_str()))
            .expect("new attempt");
        assert!(
            selected
                .summary
                .as_deref()
                .is_some_and(|s| s.starts_with("from fake-source to fake-second, derived sibling")),
            "distinct unknown homes survive: {}",
            rig.events()
        );
        assert!(
            selected
                .summary
                .as_deref()
                .is_some_and(|s| s.contains("left on its limit")),
            "first account remains left: {}",
            rig.events()
        );
        // The selected tool cannot start on a dangling store. Await its
        // existing terminal path so no detached fixture writer escapes the
        // Rig; this pin judges candidate selection, not a successful launch.
        until(&rig, || {
            records(&rig).iter().any(|event| {
                [REFUSED_ACTION, ae::autoreseat::FAILED_ACTION].contains(&event.action.as_str())
                    && event.reference.as_deref() == Some(key.to_string().as_str())
            })
        });
    }
}

#[test]
fn phase2_derivation_uses_the_recorded_client_and_excludes_the_source_profile() {
    let rig = Rig::new("hr2client");
    let suffix = format!("{} --model opus", rig.scratch.join("claude.pl").display());
    family_config(
        &rig,
        &[
            ("source", format!("alpha {suffix}")),
            ("same", format!("beta {suffix}")),
            ("return", format!("alpha {suffix}")),
        ],
        None,
        "",
    );
    let path = rig.scratch.join("config");
    let profiles = std::fs::read_to_string(&path).expect("fixture profiles");
    std::fs::write(
        path,
        format!(
            "[clients]\nalpha = {} config_home={}\nbeta = {} config_home={}\n{profiles}",
            rig.scratch.join("tools/claude").display(),
            rig.scratch.join("home-a").display(),
            rig.scratch.join("tools/claude").display(),
            rig.scratch.join("home-b").display()
        ),
    )
    .expect("scratch client bindings");
    quota(&rig, "home-a", &[("weekly_all", 10.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 95.0)], 0);
    rig.seat_rows("spawned.0", "scout", "source", "claude");
    family_row(&rig, "client.spawned.0", "beta");
    let pane = rig.new_pane("spawned.0", "scout");
    isolate(&rig, &pane);
    rig.start(&pane, "spawned.0", "claude");
    let recorded = rig.meta_row("config_home.spawned.0");
    assert_eq!(
        recorded,
        std::fs::canonicalize(rig.scratch.join("home-b"))
            .expect("client account")
            .display()
            .to_string()
    );
    let (_, relative) = plant_conversation(&rig);
    let target = rig.scratch.join("home-b").join(&relative);
    std::fs::create_dir_all(target.parent().expect("synthetic transcript parent"))
        .expect("scratch source store");
    std::fs::copy(rig.scratch.join("home-a").join(relative), target)
        .expect("synthetic source transcript");
    rebind(&rig, "config_home.spawned.0", &recorded);
    family_move(&rig, &pane, false, "fake-return", "twin");
    let attempts = booked(&rig, ATTEMPT_ACTION);
    let summary = attempts[0].summary.as_deref().expect("attempt");
    assert!(
        !summary.contains("fake-same"),
        "recorded-client account alias is excluded: {summary}"
    );
}

#[test]
fn phase2_derivation_ignores_a_followed_model_and_uses_the_recorded_pin() {
    let rig = Rig::new("hr2recordedpin");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("observed", family_command(&rig, "home-b", "--model sonnet")),
            ("twin", family_command(&rig, "home-c", "--model opus")),
            (
                "followable",
                family_command(&rig, "home-a", "--model sonnet"),
            ),
        ],
        None,
        "",
    );
    for (home, used) in [("home-a", 95.0), ("home-b", 10.0), ("home-c", 10.0)] {
        quota(&rig, home, &[("weekly_all", used)], 0);
    }
    let pane = family_source(&rig);
    family_row(&rig, "observed_model.spawned.0", "Sonnet 5");
    family_row(&rig, "observed_model_pin.spawned.0", "opus");
    family_move(&rig, &pane, false, "fake-twin", "twin");
}

#[test]
fn phase2_an_equivalent_executable_spelled_differently_is_not_family() {
    let rig = Rig::new("hr2executable");
    std::fs::create_dir_all(rig.scratch.join("other")).expect("second executable path");
    std::fs::hard_link(
        rig.scratch.join("tools/claude"),
        rig.scratch.join("other/claude"),
    )
    .expect("same fake at another executable spelling");
    let bad = family_command(&rig, "home-b", "--model opus").replace(
        &rig.scratch.join("tools/claude").display().to_string(),
        &rig.scratch.join("other/claude").display().to_string(),
    );
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("bad", bad),
            ("twin", family_command(&rig, "home-c", "--model opus")),
        ],
        None,
        "",
    );
    for (home, used) in [("home-a", 95.0), ("home-b", 10.0), ("home-c", 10.0)] {
        quota(&rig, home, &[("weekly_all", used)], 0);
    }
    let pane = family_source(&rig);
    family_move(&rig, &pane, false, "fake-twin", "twin");
    assert!(
        !booked(&rig, ATTEMPT_ACTION)[0]
            .summary
            .as_deref()
            .expect("attempt")
            .contains("fake-bad")
    );
}

#[test]
fn phase2_derived_profiles_use_the_same_origin_overlay_as_the_move() {
    let rig = Rig::new("hr2overlay");
    family_config(
        &rig,
        &[("source", family_command(&rig, "home-a", "--model opus"))],
        None,
        "",
    );
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
    std::fs::create_dir_all(rig.scratch.join(".ae")).expect("scratch overlay");
    std::fs::write(
        rig.scratch.join(".ae/config"),
        format!(
            "[profiles]\nfake-twin = \"{}\"\n",
            family_command(&rig, "home-b", "--model opus")
        ),
    )
    .expect("overlay-only twin");
    let pane = family_source(&rig);
    family_move(&rig, &pane, false, "fake-twin", "twin");
}

#[test]
fn phase2_no_family_or_declared_row_remains_silent_and_never_crosses_tools() {
    let rig = Rig::new("hr2unmapped");
    family_config(
        &rig,
        &[
            (
                "source",
                family_command(&rig, "home-a", "--model opus --effort high"),
            ),
            (
                "other",
                family_command(&rig, "home-b", "--model sonnet --effort low"),
            ),
            (
                "opencode",
                format!(
                    "{} {}",
                    rig.scratch.join("tools/opencode").display(),
                    rig.scratch.join("opencode.pl").display()
                ),
            ),
        ],
        None,
        "",
    );
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
    let pane = family_source(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _watch = watch(&rig);
    sweeps(&rig);
    unchanged(&rig, &pane, "fake-source", pid);
    assert!(
        auto_records(&rig).is_empty(),
        "Unmapped remains silent: {}",
        rig.events()
    );
}

#[test]
fn phase2_derived_refusal_names_the_candidate_and_its_room_reason() {
    let rig = Rig::new("hr2refused");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("twin", family_command(&rig, "home-b", "--model opus")),
        ],
        None,
        "",
    );
    quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
    quota(&rig, "home-b", &[("weekly_all", 95.0)], 0);
    let pane = family_source(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let key = family_open(&rig, &pane, false);
    let out = trigger(&rig);
    assert_eq!(out.status.code(), Some(1));
    let refused = booked(&rig, REFUSED_ACTION);
    assert_eq!(
        refused.len(),
        1,
        "a mapped but full family must refuse, not disappear: {}",
        rig.events()
    );
    assert_eq!(
        refused[0].reference.as_deref(),
        Some(key.to_string().as_str())
    );
    assert!(
        refused[0]
            .summary
            .as_deref()
            .is_some_and(|s| s.contains("fake-twin, derived twin (no headroom)")),
        "{}",
        rig.events()
    );
    unchanged(&rig, &pane, "fake-source", pid);
}

#[test]
fn phase2_a_derived_twin_on_a_peers_latched_account_is_skipped() {
    let rig = Rig::new("hr2peer");
    family_config(
        &rig,
        &[
            ("source", family_command(&rig, "home-a", "--model opus")),
            ("first", family_command(&rig, "home-b", "--model opus")),
            ("twin", family_command(&rig, "home-c", "--model opus")),
        ],
        None,
        "",
    );
    for (home, used) in [("home-a", 95.0), ("home-b", 10.0), ("home-c", 10.0)] {
        quota(&rig, home, &[("weekly_all", used)], 0);
    }
    let pane = family_source(&rig);
    rig.seat_rows("spawned.1", "peer", "first", "claude");
    family_row(
        &rig,
        "config_home.spawned.1",
        &std::fs::canonicalize(rig.scratch.join("home-b"))
            .expect("peer store")
            .display()
            .to_string(),
    );
    let old = Timestamp::from_epoch(Timestamp::now().epoch() - 100);
    std::fs::write(rig.dir.join("events.jsonl"), format!("{}{{\"ts\":\"{old}\",\"actor\":\"watchdog\",\"action\":\"limit\",\"target\":\"peer\",\"target_slot\":\"spawned.1\",\"target_session\":\"{}\"}}\n", rig.events(), rig.session)).expect("peer limit latch");
    family_move(&rig, &pane, false, "fake-twin", "twin");
    assert!(
        booked(&rig, ATTEMPT_ACTION)[0]
            .summary
            .as_deref()
            .is_some_and(
                |s| s.contains("fake-first, derived twin (another seat is latched on its account)")
            ),
        "{}",
        rig.events()
    );
}

#[test]
fn phase2_an_ambiguous_missing_or_absent_model_does_not_invent_a_sibling() {
    for (tag, flags) in [
        ("hr2badpinamb", "--model opus --model sonnet"),
        ("hr2badpinmissing", "--model"),
        ("hr2badpinabsent", ""),
    ] {
        let rig = Rig::new(tag);
        family_config(
            &rig,
            &[
                ("source", family_command(&rig, "home-a", "--model opus")),
                ("bad", family_command(&rig, "home-b", flags)),
            ],
            None,
            "",
        );
        quota(&rig, "home-a", &[("weekly_all", 95.0)], 0);
        quota(&rig, "home-b", &[("weekly_all", 10.0)], 0);
        let pane = family_source(&rig);
        let pid = rig.tool_pid(&pane, "claude");
        let _watch = watch(&rig);
        sweeps(&rig);
        unchanged(&rig, &pane, "fake-source", pid);
        assert!(
            auto_records(&rig).is_empty(),
            "invalid model shape is not family: {}",
            rig.events()
        );
    }
}
