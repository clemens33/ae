//! Frozen acceptance: #305 part 1, brief plus lead rulings M1–M7.
//! Authored by the non-implementer before product changes. Existing CLI only.
//! Historical fixtures reproduce the pre-feature journal (M1 legacy eligibility).

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns only the existing private Rig files, tools and tmux server"
)]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION, HELD_ACTION, NOTICE_ACTION, REFUSED_ACTION};
use ae::events::Event;
use ae::time::Timestamp;

use super::cli::OwnedChild;
use super::seat_relaunch::Rig;

const SLOT: &str = "spawned.0";
const NAME: &str = "scout";
const RETURN: &str = "auto-reseat-return";
const WAIT: Duration = Duration::from_secs(90);

fn row(rig: &Rig, key: &str, value: &str) {
    let prefix = format!("{key}=");
    let mut text = String::new();
    let mut found = false;
    for line in rig.meta().lines() {
        if line.starts_with(&prefix) {
            assert!(!found, "fixture has one row per key");
            found = true;
            writeln!(text, "{prefix}{value}").expect("string");
        } else {
            writeln!(text, "{line}").expect("string");
        }
    }
    if !found {
        writeln!(text, "{prefix}{value}").expect("string");
    }
    std::fs::write(rig.dir.join("meta"), text).expect("private meta");
}

fn command(rig: &Rig, account: &str) -> String {
    format!(
        "CLAUDE_CONFIG_DIR={} {} {} --model claude-opus-5",
        rig.scratch.join(account).display(),
        rig.scratch.join("tools/claude").display(),
        rig.scratch.join("claude.pl").display()
    )
}

fn configure(rig: &Rig, knobs: &str) {
    let switch = if knobs.lines().any(|line| line.starts_with("auto_reseat =")) {
        ""
    } else {
        "auto_reseat = on\n"
    };
    let text = format!(
        "[profiles]\norigin = \"{}\"\nsubstitute = \"{}\"\nmanual = \"{}\"\n[workspace]\nmain = lead\nlayout = vertical\n{switch}auto_reseat_grace_secs = 0\n{knobs}\n[auto_reseat]\norigin = substitute\nsubstitute = manual\nmanual = substitute\n",
        command(rig, "home-a"),
        command(rig, "home-b"),
        command(rig, "home-c")
    );
    std::fs::write(rig.scratch.join("config"), text).expect("private config");
}

/// `reset` is relative to NOW; `observed` is an exact vendor sample epoch.
fn quota(rig: &Rig, account: &str, observed: i64, windows: &[(&str, f64, i64, Option<&str>)]) {
    let now = Timestamp::now().epoch();
    let limits = windows.iter().map(|(kind, used, reset, model)| {
        let scope = model.map_or_else(|| "null".to_owned(), |name| {
            format!("{{\"model\":{{\"display_name\":\"{name}\"}}}}")
        });
        format!("{{\"kind\":\"{kind}\",\"group\":\"{kind}\",\"percent\":{used},\"resets_at\":\"{}\",\"scope\":{scope}}}", Timestamp::from_epoch(now + reset))
    }).collect::<Vec<_>>().join(",");
    let dir = rig.scratch.join(account);
    std::fs::create_dir_all(&dir).expect("private account");
    std::fs::write(dir.join(".claude.json"), format!("{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{limits}]}}}}}}", observed * 1000)).expect("private quota sample");
}

fn low(rig: &Rig, account: &str, used: f64) {
    quota(
        rig,
        account,
        Timestamp::now().epoch(),
        &[("weekly_all", used, 86_400, None)],
    );
}

fn append(rig: &Rig, actor: &str, action: &str, at: i64, reference: &str, summary: &str) {
    let mut text = rig.events();
    let reference = if reference.is_empty() {
        String::new()
    } else {
        format!(",\"ref\":\"{reference}\"")
    };
    // All callers use fixed ASCII fixture text with no JSON metacharacters.
    writeln!(text, "{{\"ts\":\"{}\",\"actor\":\"{actor}\",\"action\":\"{action}\",\"target\":\"{NAME}\",\"target_slot\":\"{SLOT}\",\"target_session\":\"{}\"{reference},\"summary\":\"{summary}\"}}", Timestamp::from_epoch(at), rig.session).expect("string");
    std::fs::write(rig.dir.join("events.jsonl"), text).expect("private historical journal");
}

/// A complete historical hop; timestamp and append order are independent.
fn hop(rig: &Rig, from: &str, to: &str, kind: &str, at: i64) {
    let key = Timestamp::from_epoch(at - 3).to_string();
    let cause = match kind {
        RETURN => "return to origin: 20% weekly_all 7d",
        "limit" => "usage limit",
        _ => "headroom 95% weekly_all 7d",
    };
    append(rig, "watchdog", kind, at - 3, "", cause);
    append(
        rig,
        "watchdog",
        ATTEMPT_ACTION,
        at - 2,
        &key,
        &format!("from {from} to {to} ({cause})"),
    );
    append(
        rig,
        "watchdog",
        "reseat",
        at - 1,
        "",
        &format!(
            "reseated {NAME} (pane %1, slot {SLOT}) to {to}, seed submitted [from {from} to {to}, prior none]"
        ),
    );
    append(
        rig,
        "watchdog",
        DONE_ACTION,
        at,
        from,
        &format!("to {to}, seeded ({cause})"),
    );
}

fn start(rig: &Rig, profile: &str) -> String {
    row(rig, &format!("seat.{SLOT}"), NAME);
    row(rig, &format!("profile.{SLOT}"), profile);
    row(rig, &format!("agent_bin.{SLOT}"), "claude");
    let pane = rig.new_pane(SLOT, NAME);
    std::fs::create_dir_all(rig.scratch.join("home")).expect("private HOME");
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
        "isolate the shell before _run records its account"
    );
    rig.start(&pane, SLOT, "claude");
    transcript(rig);
    pane
}

/// The existing fake draws but does not write a conversation, unlike a real CLI.
fn transcript(rig: &Rig) {
    let id = rig.meta_row(&format!("harness_session.{SLOT}"));
    let id = id.strip_prefix("claude:").unwrap_or(&id);
    let id = if id.is_empty() || id == "pending" {
        // The existing fake never persists its minted --session-id. Supply
        // the same store evidence a real harness + capture would produce.
        let id = "12345678-1234-1234-1234-123456789abc";
        row(
            rig,
            &format!("harness_session.{SLOT}"),
            &format!("claude:{id}"),
        );
        id
    } else {
        id
    };
    let account = PathBuf::from(rig.meta_row(&format!("config_home.{SLOT}")));
    let work = PathBuf::from(rig.meta_row("work_dir"));
    let path = account
        .join("projects")
        .join(ae::carry::project_key(&work))
        .join(format!("{id}.jsonl"));
    std::fs::create_dir_all(path.parent().expect("transcript parent")).expect("private store");
    std::fs::write(path, b"{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n")
        .expect("fake conversation");
}

fn fixture(tag: &str, knobs: &str) -> (Rig, String, i64) {
    let rig = Rig::new(tag);
    configure(&rig, knobs);
    let away = Timestamp::now().epoch() - 60;
    append(&rig, "lead", "spawn", away - 20, "", "spawned scout");
    hop(&rig, "origin", "substitute", "auto-reseat-headroom", away);
    low(&rig, "home-a", 20.0);
    low(&rig, "home-b", 10.0);
    low(&rig, "home-c", 10.0);
    let pane = start(&rig, "substitute");
    (rig, pane, away)
}

fn records(rig: &Rig) -> Vec<Event> {
    rig.events()
        .lines()
        .map(|line| Event::parse_line(line).expect("valid fixture/product journal"))
        .collect()
}

fn booked(rig: &Rig, action: &str) -> Vec<Event> {
    records(rig)
        .into_iter()
        .filter(|event| {
            event.action == action
                && ae::watchdog::event_is_addressed_to(event, &rig.session, SLOT, NAME)
        })
        .collect()
}

fn until(rig: &Rig, why: &str, mut test: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if test() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "{why}: {}\n{}",
        rig.events(),
        std::fs::read_to_string(rig.scratch.join("headroom-daemon-err")).unwrap_or_default()
    );
}

fn watch(rig: &Rig) -> OwnedChild {
    super::reseat_headroom_spec::watch(rig)
}

fn sweeps(rig: &Rig) {
    let count = || {
        std::fs::read_to_string(rig.scratch.join("headroom-quota-trace"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    let before = count();
    until(rig, "three further quota cycles", || count() >= before + 3);
}

fn core(rig: &Rig) -> super::cli::Runner {
    let mut cmd = super::cli::ae();
    cmd.env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("AE_SENDER_OVERRIDE");
    cmd
}

fn leg_command(rig: &Rig, key: Timestamp) -> super::cli::Runner {
    let mut cmd = core(rig);
    cmd.arg("_auto-reseat")
        .arg(&rig.dir)
        .args([SLOT, &key.to_string()]);
    cmd
}

fn manual(rig: &Rig, to: &str) -> Output {
    core(rig)
        .args(["reseat", &rig.session, NAME, "--using", to])
        .output()
        .expect("real manual reseat")
}

fn open_return(rig: &Rig) -> Timestamp {
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 10);
    append(
        rig,
        "watchdog",
        RETURN,
        key.epoch(),
        "",
        "return to origin: 20% weekly_all 7d",
    );
    append(
        rig,
        "watchdog",
        ATTEMPT_ACTION,
        key.epoch() + 1,
        &key.to_string(),
        "from substitute to origin (return 20% weekly_all 7d)",
    );
    key
}

fn assert_quiet(rig: &Rig, pane: &str, before: (usize, usize, usize), pid: Option<u32>) {
    assert_eq!(rig.meta_row(&format!("profile.{SLOT}")), "substitute");
    assert_eq!(
        rig.tool_pid(pane, "claude"),
        pid,
        "no stop before idle/relief is proven"
    );
    assert_eq!(
        booked(rig, ATTEMPT_ACTION).len(),
        before.0,
        "no new attempt"
    );
    assert_eq!(
        booked(rig, HELD_ACTION).len(),
        before.1,
        "silent pre-attempt hold"
    );
    assert_eq!(
        booked(rig, NOTICE_ACTION).len(),
        before.2,
        "silent pre-attempt hold"
    );
}

fn quiet_counts(rig: &Rig) -> (usize, usize, usize) {
    (
        booked(rig, ATTEMPT_ACTION).len(),
        booked(rig, HELD_ACTION).len(),
        booked(rig, NOTICE_ACTION).len(),
    )
}

fn returned(rig: &Rig, before_done: usize) {
    until(
        rig,
        "idle auto-moved seat returns on fresh origin relief",
        || booked(rig, DONE_ACTION).len() > before_done,
    );
    assert_eq!(rig.meta_row(&format!("profile.{SLOT}")), "origin");
    let dones = booked(rig, DONE_ACTION);
    let done = dones.last().expect("return done");
    assert_eq!(done.reference.as_deref(), Some("substitute"));
    assert!(
        done.summary
            .as_deref()
            .unwrap_or_default()
            .contains("return"),
        "return identified in done: {done:?}"
    );
    assert!(!booked(rig, RETURN).is_empty(), "return opener journaled");
    assert!(
        !ae::autoreseat::left_profiles(&records(rig), &rig.session, SLOT, NAME)
            .iter()
            .any(|(profile, _)| profile == "substitute"),
        "return does not make the substitute a left-on-pressure profile"
    );
}

fn barrier(rig: &Rig, key: &str) -> PathBuf {
    let dir = rig.scratch.join("return-read-gate");
    std::fs::create_dir_all(&dir).expect("private read gate");
    std::fs::write(dir.join(key), "").expect("arm existing seam");
    dir
}

fn wait_barrier(rig: &Rig, dir: &std::path::Path, key: &str, child: &mut OwnedChild) {
    until(rig, "existing deterministic lifecycle seam reached", || {
        let held = std::fs::read_to_string(dir.join("trace"))
            .unwrap_or_default()
            .lines()
            .any(|line| line == format!("held {key}"));
        assert!(
            held || child.try_wait().expect("child status").is_none(),
            "leg ended without reaching {key}: {}",
            rig.events()
        );
        held
    });
}

#[test]
fn legacy_automatic_chain_returns_on_fresh_relief_and_announces_once() {
    let (rig, _, _) = fixture("mbreturn", "");
    let _daemon = watch(&rig);
    returned(&rig, 1);
    until(&rig, "human notice names return", || {
        records(&rig).iter().any(|event| {
            event.action == "chat"
                && event
                    .summary
                    .as_deref()
                    .is_some_and(|s| s.contains("return") && s.contains("origin"))
        })
    });
    sweeps(&rig);
    assert_eq!(booked(&rig, DONE_ACTION).len(), 2, "one return");
}

#[test]
fn genuine_headroom_departure_then_relief_returns_through_the_daemon() {
    let rig = Rig::new("mbrealhop");
    configure(&rig, "");
    low(&rig, "home-a", 95.0);
    low(&rig, "home-b", 10.0);
    low(&rig, "home-c", 10.0);
    let _pane = start(&rig, "origin");
    let at = Timestamp::now().epoch() - 20;
    let key = Timestamp::from_epoch(at);
    append(
        &rig,
        "watchdog",
        "auto-reseat-headroom",
        at,
        "",
        "headroom 95% weekly_all 7d",
    );
    append(
        &rig,
        "watchdog",
        ATTEMPT_ACTION,
        at + 1,
        &key.to_string(),
        "from origin to substitute (headroom 95% weekly_all 7d)",
    );
    let moved = leg_command(&rig, key).output().expect("real away leg");
    assert!(
        moved.status.success(),
        "away leg: {}",
        String::from_utf8_lossy(&moved.stderr)
    );
    assert_eq!(rig.meta_row(&format!("profile.{SLOT}")), "substitute");
    assert_eq!(booked(&rig, DONE_ACTION).len(), 1);
    transcript(&rig);
    std::thread::sleep(Duration::from_millis(1100));
    low(&rig, "home-a", 20.0);
    let _daemon = watch(&rig);
    returned(&rig, 1);
}

#[test]
fn strict_rearm_band_holds_at_ninety_and_returns_below_it() {
    let (rig, pane, _) = fixture("mbband", "");
    low(&rig, "home-a", 90.0);
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_quiet(&rig, &pane, before, pid);
    low(&rig, "home-a", 89.0);
    returned(&rig, 1);
}

#[test]
fn current_threshold_edit_changes_the_return_band() {
    let (rig, pane, _) = fixture("mbthreshold", "auto_reseat_at = 90");
    low(&rig, "home-a", 87.0);
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_quiet(&rig, &pane, before, pid);
    configure(&rig, "auto_reseat_at = 95");
    returned(&rig, 1);
}

#[test]
fn threshold_off_still_returns_a_limit_chain_using_default_rearm_band() {
    let (rig, _, away) = fixture("mboffat", "auto_reseat_at = off");
    std::fs::write(rig.dir.join("events.jsonl"), "").expect("replace history before daemon");
    hop(&rig, "origin", "substitute", "limit", away);
    let _daemon = watch(&rig);
    returned(&rig, 1);
}

#[test]
fn fresh_samples_at_or_before_the_away_stamp_do_not_prove_relief() {
    for (tag, delta) in [("mbequal", 0), ("mbolder", -1)] {
        let (rig, pane, away) = fixture(tag, "");
        quota(
            &rig,
            "home-a",
            away + delta,
            &[("weekly_all", 20.0, 86_400, None)],
        );
        let before = quiet_counts(&rig);
        let pid = rig.tool_pid(&pane, "claude");
        let _daemon = watch(&rig);
        sweeps(&rig);
        assert_quiet(&rig, &pane, before, pid);
    }
}

#[test]
fn stale_or_missing_origin_sample_never_returns() {
    for tag in ["mbstale", "mbmissing"] {
        let (rig, pane, _) = fixture(tag, "");
        if tag == "mbstale" {
            quota(
                &rig,
                "home-a",
                Timestamp::now().epoch() - 1800,
                &[("weekly_all", 20.0, 86_400, None)],
            );
        } else {
            std::fs::remove_file(rig.scratch.join("home-a/.claude.json"))
                .expect("remove only private sample");
        }
        let before = quiet_counts(&rig);
        let pid = rig.tool_pid(&pane, "claude");
        let _daemon = watch(&rig);
        sweeps(&rig);
        assert_quiet(&rig, &pane, before, pid);
    }
}

#[test]
fn passed_reset_without_a_new_sample_holds_then_a_fresh_sample_returns() {
    let (rig, pane, _) = fixture("mbreset", "");
    quota(
        &rig,
        "home-a",
        Timestamp::now().epoch(),
        &[("weekly_all", 0.0, -1, None)],
    );
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_quiet(&rig, &pane, before, pid);
    low(&rig, "home-a", 0.0);
    returned(&rig, 1);
}

#[test]
fn every_present_binding_key_must_prove_room_not_only_the_best_one() {
    let (rig, pane, _) = fixture("mballkeys", "");
    quota(
        &rig,
        "home-a",
        Timestamp::now().epoch(),
        &[("session", 0.0, 3600, None), ("weekly_all", 0.0, -1, None)],
    );
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_quiet(&rig, &pane, before, pid);
    quota(
        &rig,
        "home-a",
        Timestamp::now().epoch(),
        &[
            ("session", 0.0, 3600, None),
            ("weekly_all", 0.0, 86_400, None),
        ],
    );
    returned(&rig, 1);
}

#[test]
fn tied_opposed_readings_and_unjudgeable_keys_cannot_prove_room() {
    for tag in ["mbtie", "mbunknownkey"] {
        let (rig, pane, _) = fixture(tag, "");
        let second = if tag == "mbtie" {
            "weekly_all"
        } else {
            "unmeasured_bucket"
        };
        quota(
            &rig,
            "home-a",
            Timestamp::now().epoch(),
            &[
                ("weekly_all", 20.0, 86_400, None),
                (second, 99.0, 86_400, None),
            ],
        );
        let before = quiet_counts(&rig);
        let pid = rig.tool_pid(&pane, "claude");
        let _daemon = watch(&rig);
        sweeps(&rig);
        assert_quiet(&rig, &pane, before, pid);
        low(&rig, "home-a", 20.0);
        returned(&rig, 1);
    }
}

#[test]
fn another_account_or_only_another_models_windows_do_not_prove_room() {
    for tag in ["mbwrongaccount", "mbwrongmodel"] {
        let (rig, pane, _) = fixture(tag, "");
        if tag == "mbwrongaccount" {
            low(&rig, "home-a", 92.0);
            low(&rig, "home-c", 0.0);
        } else {
            quota(
                &rig,
                "home-a",
                Timestamp::now().epoch(),
                &[("weekly_scoped", 0.0, 86_400, Some("Sonnet 5"))],
            );
        }
        let before = quiet_counts(&rig);
        let pid = rig.tool_pid(&pane, "claude");
        let _daemon = watch(&rig);
        sweeps(&rig);
        assert_quiet(&rig, &pane, before, pid);
        low(&rig, "home-a", 20.0);
        returned(&rig, 1);
    }
}

#[test]
fn a_short_limit_departure_returns_after_its_short_window_and_long_window_recover() {
    let (rig, pane, away) = fixture("mbshort", "auto_reseat_at = off");
    std::fs::write(rig.dir.join("events.jsonl"), "").expect("private legacy limit history");
    hop(&rig, "origin", "substitute", "limit", away);
    quota(
        &rig,
        "home-a",
        Timestamp::now().epoch(),
        &[
            ("session", 100.0, -1, None),
            ("weekly_all", 20.0, 86_400, None),
        ],
    );
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_quiet(&rig, &pane, before, pid);
    quota(
        &rig,
        "home-a",
        Timestamp::now().epoch(),
        &[
            ("session", 0.0, 18_000, None),
            ("weekly_all", 20.0, 86_400, None),
        ],
    );
    returned(&rig, 1);
}

#[test]
fn no_long_window_means_no_return_even_when_the_short_window_is_fresh_low() {
    let (rig, pane, _) = fixture("mbnolong", "");
    quota(
        &rig,
        "home-a",
        Timestamp::now().epoch(),
        &[("session", 0.0, 18_000, None)],
    );
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_quiet(&rig, &pane, before, pid);
}

#[test]
fn vanished_short_key_is_the_ruled_residual_while_a_long_key_remains() {
    let (rig, pane, _) = fixture("mbvanished", "");
    quota(
        &rig,
        "home-a",
        Timestamp::now().epoch(),
        &[
            ("session", 92.0, 18_000, None),
            ("weekly_all", 20.0, 86_400, None),
        ],
    );
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_quiet(&rig, &pane, before, pid);
    low(&rig, "home-a", 20.0);
    returned(&rig, 1);
}

#[test]
fn manual_reseat_after_an_auto_move_cancels_return_ownership() {
    let (rig, _, _) = fixture("mbmanual", "");
    let out = manual(&rig, "manual");
    assert!(
        out.status.success(),
        "manual move: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(rig.meta_row(&format!("profile.{SLOT}")), "manual");
    let before = booked(&rig, DONE_ACTION).len();
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_eq!(rig.meta_row(&format!("profile.{SLOT}")), "manual");
    assert_eq!(booked(&rig, DONE_ACTION).len(), before);
    assert!(booked(&rig, RETURN).is_empty());
}

#[test]
fn same_profile_manual_record_or_new_spawn_ends_the_chain() {
    for tag in ["mbsame", "mbspawn"] {
        let (rig, pane, _) = fixture(tag, "");
        if tag == "mbsame" {
            append(
                &rig,
                "human",
                "reseat",
                Timestamp::now().epoch(),
                "",
                "already using substitute",
            );
        } else {
            append(
                &rig,
                "lead",
                "spawn",
                Timestamp::now().epoch(),
                "",
                "new incarnation",
            );
        }
        let before = quiet_counts(&rig);
        let pid = rig.tool_pid(&pane, "claude");
        let _daemon = watch(&rig);
        sweeps(&rig);
        assert_quiet(&rig, &pane, before, pid);
        assert!(booked(&rig, RETURN).is_empty());
    }
}

#[test]
fn orphan_done_without_its_watchdog_reseat_is_never_return_provenance() {
    let (rig, pane, _) = fixture("mborphan", "");
    let text = rig
        .events()
        .lines()
        .filter(|line| !Event::parse_line(line).is_ok_and(|event| event.action == "reseat"))
        .fold(String::new(), |mut text, line| {
            writeln!(text, "{line}").expect("string");
            text
        });
    std::fs::write(rig.dir.join("events.jsonl"), text).expect("orphan historical done");
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_quiet(&rig, &pane, before, pid);
    assert!(booked(&rig, RETURN).is_empty());
}

#[test]
fn manual_choice_between_return_choice_and_lifecycle_lock_is_never_overwritten() {
    let (rig, pane, _) = fixture("mblockrace", "");
    let key = open_return(&rig);
    let gate = barrier(&rig, "@auto-reseat-pair");
    let mut child = leg_command(&rig, key)
        .env("AE_TEST_APP_READ_GATE", &gate)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("return leg");
    wait_barrier(&rig, &gate, "@auto-reseat-pair", &mut child);
    let out = manual(&rig, "manual");
    assert!(
        out.status.success(),
        "manual wins: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pid = rig.tool_pid(&pane, "claude");
    let before_reseat = booked(&rig, "reseat").len();
    let before_done = booked(&rig, DONE_ACTION).len();
    std::fs::remove_file(gate.join("@auto-reseat-pair")).expect("release choice barrier");
    child.wait_with_output().expect("return leg ended");
    assert_eq!(rig.meta_row(&format!("profile.{SLOT}")), "manual");
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    assert_eq!(
        booked(&rig, "reseat").len(),
        before_reseat,
        "automatic gate must precede stop and carry"
    );
    assert_eq!(booked(&rig, DONE_ACTION).len(), before_done);
    assert!(
        booked(&rig, HELD_ACTION).iter().any(|event| event
            .summary
            .as_deref()
            .is_some_and(|s| s.starts_with("return-changed:"))),
        "{}",
        rig.events()
    );
}

#[test]
fn manual_choice_between_away_reseat_and_done_is_not_reclaimed_by_that_done() {
    let rig = Rig::new("mbdonerace");
    configure(&rig, "");
    low(&rig, "home-a", 95.0);
    low(&rig, "home-b", 10.0);
    low(&rig, "home-c", 10.0);
    let pane = start(&rig, "origin");
    let at = Timestamp::now().epoch() - 20;
    let key = Timestamp::from_epoch(at);
    append(
        &rig,
        "watchdog",
        "auto-reseat-headroom",
        at,
        "",
        "headroom 95% weekly_all 7d",
    );
    append(
        &rig,
        "watchdog",
        ATTEMPT_ACTION,
        at + 1,
        &key.to_string(),
        "from origin to substitute",
    );
    let gate = barrier(&rig, "@auto-reseat-after");
    let mut child = leg_command(&rig, key)
        .env("AE_TEST_APP_READ_GATE", &gate)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("away leg");
    wait_barrier(&rig, &gate, "@auto-reseat-after", &mut child);
    transcript(&rig);
    let out = manual(&rig, "manual");
    assert!(
        out.status.success(),
        "manual in done gap: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pid = rig.tool_pid(&pane, "claude");
    std::fs::remove_file(gate.join("@auto-reseat-after")).expect("release done barrier");
    child.wait_with_output().expect("away leg ended");
    std::thread::sleep(Duration::from_millis(1100));
    low(&rig, "home-a", 20.0);
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_eq!(rig.meta_row(&format!("profile.{SLOT}")), "manual");
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    assert_eq!(booked(&rig, DONE_ACTION).len(), 1);
    assert!(booked(&rig, RETURN).is_empty());
}

#[test]
fn busy_and_draft_return_holds_are_silent_then_idle_returns() {
    for tag in ["mbbusy", "mbdraft"] {
        let (rig, pane, _) = fixture(tag, "");
        if tag == "mbbusy" {
            rig.mark_busy(&pane);
        } else {
            rig.mark_draft(&pane);
        }
        let before = quiet_counts(&rig);
        let pid = rig.tool_pid(&pane, "claude");
        let _daemon = watch(&rig);
        sweeps(&rig);
        assert_quiet(&rig, &pane, before, pid);
        let marker = if tag == "mbbusy" {
            "__BUSY__"
        } else {
            "__DRAFT__"
        };
        std::fs::remove_file(rig.scratch.join(marker)).expect("fake becomes idle");
        returned(&rig, 1);
    }
}

#[test]
fn return_grace_is_silent_and_starts_when_return_first_becomes_eligible() {
    let (rig, pane, _) = fixture("mbgrace", "");
    let path = rig.scratch.join("config");
    let config = std::fs::read_to_string(&path).expect("config");
    std::fs::write(
        path,
        config.replace("auto_reseat_grace_secs = 0", "auto_reseat_grace_secs = 6"),
    )
    .expect("return grace");
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    until(&rig, "return opener starts grace", || {
        !booked(&rig, RETURN).is_empty()
    });
    let opened = booked(&rig, RETURN)[0].ts;
    std::thread::sleep(Duration::from_secs(2));
    assert_quiet(&rig, &pane, before, pid);
    returned(&rig, 1);
    let attempt = booked(&rig, ATTEMPT_ACTION)
        .last()
        .expect("return attempt")
        .clone();
    assert!(attempt.ts.epoch() >= opened.epoch() + 6);
    assert_eq!(
        attempt.reference.as_deref(),
        Some(opened.to_string().as_str())
    );
}

#[test]
fn pair_guard_refuses_a_return_to_the_partners_provider_and_tells_once() {
    let (rig, pane, _) = fixture("mbpair", "");
    // Current scout is proven OpenAI, paired with an Anthropic seat. The fake
    // only has to be idle for refusal; it must never be stopped or launched.
    // The current seat really runs the recorded substitute tool. Historical
    // journal fields do not encode the tool, so this is a valid cross-tool hop.
    let config = std::fs::read_to_string(rig.scratch.join("config")).expect("config");
    let config = config.replace(
        &format!("substitute = \"{}\"", command(&rig, "home-b")),
        &format!(
            "substitute = \"{} {}\"",
            rig.scratch.join("tools/codex").display(),
            rig.scratch.join("codex.pl").display()
        ),
    );
    std::fs::write(rig.scratch.join("config"), config).expect("different current provider");
    row(&rig, &format!("agent_bin.{SLOT}"), "codex");
    rig.kill_tools(&pane);
    rig.start(&pane, SLOT, "codex");
    row(&rig, "seat.spawned.1", "scout-nav");
    row(&rig, "profile.spawned.1", "manual");
    row(&rig, "agent_bin.spawned.1", "claude");
    let key = open_return(&rig);
    let before = rig.meta();
    let pid = rig.tool_pid(&pane, "codex");
    leg_command(&rig, key).output().expect("return leg");
    assert_eq!(rig.meta(), before);
    assert_eq!(rig.tool_pid(&pane, "codex"), pid);
    assert!(
        booked(&rig, REFUSED_ACTION).iter().any(|event| event
            .summary
            .as_deref()
            .is_some_and(|s| s.starts_with("pair-provider:"))),
        "{}",
        rig.events()
    );
    let notices = booked(&rig, NOTICE_ACTION).len();
    leg_command(&rig, key).output().expect("duplicate leg");
    assert_eq!(booked(&rig, NOTICE_ACTION).len(), notices);
}

#[test]
fn readiness_is_rechecked_by_the_leg_before_any_stop() {
    let (rig, pane, _) = fixture("mbrecheck", "");
    let key = open_return(&rig);
    low(&rig, "home-a", 93.0);
    let before = rig.meta();
    let pid = rig.tool_pid(&pane, "claude");
    leg_command(&rig, key).output().expect("return leg recheck");
    assert_eq!(rig.meta(), before);
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    assert_eq!(booked(&rig, DONE_ACTION).len(), 1);
    assert!(
        booked(&rig, HELD_ACTION).iter().any(|event| event
            .summary
            .as_deref()
            .is_some_and(|s| s.contains("origin account no longer proves room"))),
        "{}",
        rig.events()
    );
}

#[test]
fn switch_off_and_session_exclusion_are_byte_inert_for_return_records() {
    for (tag, knobs) in [
        ("mbdisabled", "auto_reseat = off"),
        ("mbexcluded", "auto_reseat_sessions = elsewhere"),
        ("mbquotaoff", "quota = off"),
    ] {
        let (rig, pane, _) = fixture(tag, knobs);
        let before = quiet_counts(&rig);
        let journal = booked(&rig, DONE_ACTION).len();
        let pid = rig.tool_pid(&pane, "claude");
        let _daemon = watch(&rig);
        sweeps(&rig);
        assert_quiet(&rig, &pane, before, pid);
        assert_eq!(booked(&rig, DONE_ACTION).len(), journal);
        assert!(booked(&rig, RETURN).is_empty());
    }
}

#[test]
fn multi_hop_chain_returns_to_its_root_and_not_its_intermediate() {
    let (rig, _, away) = fixture("mbroot", "");
    // Preserve product-valid chronology: current substitute is the third hop;
    // historical middle profile is manual, with a later newer done stamp.
    std::fs::write(rig.dir.join("events.jsonl"), "").expect("private multi-hop history");
    hop(&rig, "origin", "manual", "auto-reseat-headroom", away - 30);
    hop(&rig, "manual", "substitute", "limit", away);
    let _daemon = watch(&rig);
    returned(&rig, 2);
}

#[test]
fn third_return_in_rolling_day_is_held_once_across_daemon_restart() {
    let (rig, pane, away) = fixture("mbflap", "");
    std::fs::write(rig.dir.join("events.jsonl"), "").expect("private repeated history");
    for at in [away - 600, away - 300] {
        hop(&rig, "origin", "substitute", "limit", at);
        hop(&rig, "substitute", "origin", RETURN, at + 40);
    }
    hop(&rig, "origin", "substitute", "limit", away);
    let pid = rig.tool_pid(&pane, "claude");
    {
        let _daemon = watch(&rig);
        until(&rig, "flap bound produces its one hold", || {
            booked(&rig, HELD_ACTION).iter().any(|event| {
                event
                    .summary
                    .as_deref()
                    .is_some_and(|s| s.contains("24 h") && s.contains("return"))
            })
        });
        sweeps(&rig);
    }
    let holds = booked(&rig, HELD_ACTION).len();
    let notices = booked(&rig, NOTICE_ACTION).len();
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_eq!(rig.meta_row(&format!("profile.{SLOT}")), "substitute");
    assert_eq!(rig.tool_pid(&pane, "claude"), pid);
    assert_eq!(booked(&rig, DONE_ACTION).len(), 5);
    assert_eq!(booked(&rig, HELD_ACTION).len(), holds);
    assert_eq!(booked(&rig, NOTICE_ACTION).len(), notices);
}

#[test]
fn an_old_return_outside_the_rolling_day_does_not_spend_todays_allowance() {
    let (rig, _, away) = fixture("mbflapage", "");
    std::fs::write(rig.dir.join("events.jsonl"), "").expect("private rolling history");
    for at in [away - 90_000, away - 300] {
        hop(&rig, "origin", "substitute", "limit", at);
        hop(&rig, "substitute", "origin", RETURN, at + 40);
    }
    hop(&rig, "origin", "substitute", "limit", away);
    let _daemon = watch(&rig);
    returned(&rig, 5);
}

#[test]
fn malformed_or_truncated_origin_is_not_silently_dropped_from_the_proof() {
    for tag in ["mbbadjson", "mbtruncated"] {
        let (rig, pane, _) = fixture(tag, "");
        let bytes = if tag == "mbbadjson" {
            b"{broken".to_vec()
        } else {
            vec![b' '; 2 * 1024 * 1024]
        };
        std::fs::write(rig.scratch.join("home-a/.claude.json"), bytes)
            .expect("private unreadable observation");
        let before = quiet_counts(&rig);
        let pid = rig.tool_pid(&pane, "claude");
        let _daemon = watch(&rig);
        sweeps(&rig);
        assert_quiet(&rig, &pane, before, pid);
        low(&rig, "home-a", 20.0);
        returned(&rig, 1);
    }
}

#[test]
fn an_unproven_frame_never_receives_the_startup_quiet_waiver_for_a_return() {
    let (rig, pane, _) = fixture("mbunknownframe", "");
    let script = rig.scratch.join("claude.pl");
    let body = std::fs::read_to_string(&script).expect("private fake");
    assert!(body.contains("my $frame = 1;"));
    std::fs::write(&script, body.replace("my $frame = 1;", "my $frame = 0;"))
        .expect("unmodeled external frame");
    rig.kill_tools(&pane);
    rig.start(&pane, SLOT, "claude");
    let before = quiet_counts(&rig);
    let pid = rig.tool_pid(&pane, "claude");
    let _daemon = watch(&rig);
    sweeps(&rig);
    assert_quiet(&rig, &pane, before, pid);
}

#[test]
fn an_already_collapsed_pair_may_return_and_names_that_it_stays_collapsed() {
    let (rig, _, _) = fixture("mbcollapsed", "");
    row(&rig, "seat.spawned.1", "scout-nav");
    row(&rig, "profile.spawned.1", "manual");
    row(&rig, "agent_bin.spawned.1", "claude");
    let _daemon = watch(&rig);
    returned(&rig, 1);
    assert!(
        booked(&rig, DONE_ACTION)
            .last()
            .expect("return")
            .summary
            .as_deref()
            .is_some_and(|s| s.starts_with("pair-provider: pair stays on Anthropic")),
        "{}",
        rig.events()
    );
}

#[test]
fn ordinary_headroom_or_drawn_limit_on_the_substitute_outranks_return() {
    for (tag, limit) in [("mbpressure", false), ("mblimitpriority", true)] {
        let (rig, pane, _) = fixture(tag, "");
        if limit {
            rig.mark_limited(&pane);
        } else {
            low(&rig, "home-b", 95.0);
        }
        let _daemon = watch(&rig);
        until(&rig, "ordinary pressure moves first", || {
            booked(&rig, DONE_ACTION).len() > 1
        });
        let done = booked(&rig, DONE_ACTION)[1].clone();
        assert!(
            done.summary
                .as_deref()
                .is_some_and(|s| s.contains("manual") && !s.contains("return")),
            "ordinary route keeps candidate order: {done:?}"
        );
    }
}

#[path = "leadat_moveback_spec.rs"]
mod leadat;
