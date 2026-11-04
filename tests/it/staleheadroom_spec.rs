//! Independent acceptance: newest usable evidence of one account/window rules.
//! Oracle: lead PLAN RULINGS 1B/2(i)/3/4 and plan-r2 amendment P4/unknown clock.
//! Real quota parser, identity join, watchdog and detached leg; vendor TUI only fake.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures own scratch homes, vendor records, journals and private tmux servers"
)]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION, FAILED_ACTION, HELD_ACTION, REFUSED_ACTION};
use ae::events::Event;
use ae::time::Timestamp;

use super::seat_relaunch::Rig;

const OWN: &str = "01a1592e-c600-7222-8222-222222222222";
const OLD: &str = "01a1592e-c600-7222-8222-333333333333";
const NEW: &str = "01a1592e-c600-7222-8222-444444444444";
const WAIT: Duration = Duration::from_secs(90);

// Same measured idle composer as the existing effort fixture, plus a busy arm.
// No quota or lifecycle result is simulated by this process.
const CODEX: &str = r#"
use strict; use warnings; use utf8; use Encode qw(encode_utf8);
my $root='__ROOT__'; $|=1;
my $effort='xhigh';
for my $arg (@ARGV) { $effort=$1 if $arg =~ /model_reasoning_effort=(\w+)/; }
system('stty raw -echo 2>/dev/null'); binmode(STDIN,':raw'); binmode(STDOUT,':raw');
sub bye { system('stty sane 2>/dev/null'); exit 0; }
$SIG{TERM}=\&bye; $SIG{HUP}=\&bye; $SIG{INT}=\&bye;
sub draw {
  my $busy=shift;
  print encode_utf8("\e[H\e[2J".($busy ? "• Working (3s · esc to interrupt)" : "• ok").
    "\r\n\r\n\e[1m›\e[0m \e[2mAsk Codex to do anything\e[0m\r\n\r\n  gpt-6.1-sol $effort · /tmp/work\r\n");
  open(my $out,'>',"$root/drawn.$$.tmp") or die;
  print $out $busy ? 'esc to interrupt' : 'Ask Codex to do anything'; close($out);
  rename("$root/drawn.$$.tmp","$root/drawn.$$") or die;
}
my $shown=-e "$root/__BUSY__" ? 1:0; draw($shown);
while (1) {
  bye() if -e "$root/__EXIT__";
  my $now=-e "$root/__BUSY__" ? 1:0; if ($now != $shown) { draw($now); $shown=$now; }
  my $ready=''; vec($ready,fileno(STDIN),1)=1;
  if (select($ready,undef,undef,0.05)>0) { my $chunk=''; sysread(STDIN,$chunk,4096); }
}
"#;

struct Fixture {
    rig: Rig,
    account: PathBuf,
    pane: String,
    slot: &'static str,
    agent: &'static str,
    now: i64,
}

fn put(rig: &Rig, key: &str, value: &str) {
    let prefix = format!("{key}=");
    let mut text = rig
        .meta()
        .lines()
        .filter(|line| !line.starts_with(&prefix))
        .fold(String::new(), |mut out, line| {
            writeln!(out, "{line}").expect("string write");
            out
        });
    writeln!(text, "{prefix}{value}").expect("string write");
    std::fs::write(rig.dir.join("meta"), text).expect("scratch meta");
}

fn setup(tag: &str, effort: bool, source: &str) -> Fixture {
    let rig = Rig::new(tag);
    let root = std::fs::canonicalize(&rig.scratch).expect("physical scratch");
    let account = root.join("account");
    std::fs::create_dir_all(account.join("sessions/2026/10/20")).expect("scratch account");
    std::fs::create_dir_all(root.join("home")).expect("scratch HOME");
    std::fs::write(
        root.join("codex.pl"),
        CODEX.replace("__ROOT__", &root.display().to_string()),
    )
    .expect("vendor TUI");
    let original = std::fs::read_to_string(root.join("config")).expect("rig profiles");
    let profiles = original.split("[workspace]").next().expect("profiles");
    let source_command = if source == "codex" {
        format!(
            "CODEX_HOME={} {} {} -m gpt-6.1-sol -c model_reasoning_effort=xhigh",
            account.display(),
            root.join("tools/codex").display(),
            root.join("codex.pl").display()
        )
    } else {
        format!(
            "CLAUDE_CONFIG_DIR={} {} {}",
            account.display(),
            root.join("tools/claude").display(),
            root.join("claude.pl").display()
        )
    };
    let low = source_command.replace(
        "model_reasoning_effort=xhigh",
        "model_reasoning_effort=high",
    );
    let switches = if effort {
        "auto_reseat = off\nauto_effort = on"
    } else {
        "auto_reseat = all\nauto_reseat_at = 90"
    };
    let text = format!(
        "{profiles}source = \"{source_command}\"\nsource-low = \"{low}\"\ntarget = \"CODEX_HOME={} {} {} -m gpt-6.1-sol\"\n[workspace]\nmain = lead\nlayout = vertical\nauto_upgrade = off\nquota = on\nauto_reseat_grace_secs = 600\n{switches}\n[auto_reseat]\nsource = fake-opencode\n",
        account.display(),
        root.join("tools/codex").display(),
        root.join("codex.pl").display()
    );
    ae::config::parse_identity(&text).expect("valid acceptance config");
    std::fs::write(root.join("config"), text).expect("scratch config");
    let (slot, agent, pane) = if effort {
        ("main", "lead", rig.main_pane.clone())
    } else {
        rig.seat_rows("spawned.0", "scout", source, source);
        ("spawned.0", "scout", rig.new_pane("spawned.0", "scout"))
    };
    put(&rig, &format!("profile.{slot}"), "source");
    put(&rig, &format!("agent_bin.{slot}"), source);
    put(&rig, &format!("launch_id.{slot}"), "staleheadroom-fixture");
    put(&rig, &format!("harness_session.{slot}"), OWN);
    put(&rig, "session_id", "11111111-1111-4111-8111-111111111111");
    put(&rig, "work_dir", root.join("work").to_str().expect("work"));
    // An actual conversation with no rate-limit record is a supported state.
    std::fs::write(
        account.join(format!("sessions/2026/10/20/rollout-{OWN}.jsonl")),
        "{\"type\":\"user\"}\n",
    )
    .expect("own conversation");
    std::fs::write(ae::run::started_marker(&rig.dir, slot), "").expect("resume marker");
    isolate(&rig, &root, &pane);
    rig.start(&pane, slot, source);
    let kind = if source == "codex" {
        ae::tool::ToolKind::Codex
    } else {
        ae::tool::ToolKind::Claude
    };
    assert_eq!(
        ae::harness_state::classify(&rig.capture(&pane), kind),
        ae::harness_state::HarnessState::Idle,
        "idle premise"
    );
    Fixture {
        rig,
        account,
        pane,
        slot,
        agent,
        now: Timestamp::now().epoch(),
    }
}

fn isolate(rig: &Rig, root: &std::path::Path, pane: &str) {
    assert!(
        rig.tmux(&[
            "respawn-pane",
            "-k",
            "-c",
            root.join("work").to_str().expect("work"),
            "-t",
            pane,
            "env",
            "-u",
            "CODEX_HOME",
            "-u",
            "CLAUDE_CONFIG_DIR",
            &format!("HOME={}", root.join("home").display()),
            &format!("AE_HOME={}", root.display()),
            &format!("CONFIG_FILE={}", root.join("config").display()),
            "/bin/sh"
        ])
        .0
    );
}

#[derive(Clone, Copy)]
struct Sample<'a> {
    used: u8,
    age: i64,
    bucket: &'a str,
    plan: &'a str,
    minutes: u32,
    clock: bool,
    expired: bool,
}

fn sample(used: u8, age: i64) -> Sample<'static> {
    Sample {
        used,
        age,
        bucket: "codex",
        plan: "pro",
        minutes: 10_080,
        clock: true,
        expired: false,
    }
}

/// Publish another real rostered conversation; scope correlation is not mocked.
fn publish(f: &Fixture, id: &str, account: &std::path::Path, row: Sample<'_>) {
    let day = account.join("sessions/2026/10/20");
    std::fs::create_dir_all(&day).expect("rollout day");
    let dir = f.rig.scratch.join("sessions").join(format!("peer-{id}"));
    std::fs::create_dir_all(&dir).expect("peer metadata directory");
    std::fs::write(dir.join("meta"), format!("schema=2\nseat.main=reader\nprofile.main=target\nagent_bin.main=codex\nharness_session.main={id}\nconfig_home.main={}\n", account.display())).expect("recorded peer account");
    let stamp = if row.clock {
        format!(
            "\"timestamp\":\"{}\",",
            Timestamp::from_epoch(f.now - row.age)
        )
    } else {
        String::new()
    };
    let reset = f.now + if row.expired { -1 } else { 86_400 };
    let text = format!(
        "{{{stamp}\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"rate_limits\":{{\"limit_id\":\"{}\",\"plan_type\":\"{}\",\"primary\":{{\"used_percent\":{},\"window_minutes\":{},\"resets_at\":{reset}}}}}}}}}\n",
        row.bucket, row.plan, row.used, row.minutes
    );
    let parsed = ae::quota::codex::parse(text.as_bytes(), true, f.now)
        .expect("synthetic vendor protocol parses");
    assert_eq!(
        parsed.rows.len(),
        1,
        "fixture must contribute a real quota row"
    );
    assert_eq!(
        parsed.rows[0].used_percent.as_deref(),
        Some(row.used.to_string().as_str())
    );
    assert_eq!(
        parsed.rows[0].observed_at,
        row.clock.then_some(f.now - row.age)
    );
    assert_eq!(
        parsed.rows[0].status,
        if !row.clock || row.expired {
            ae::quota::Status::Unknown
        } else if row.age > 900 {
            ae::quota::Status::Stale
        } else {
            ae::quota::Status::Fresh
        }
    );
    // Rewrite order/mtime is independent of vendor observed_at.
    std::fs::write(day.join(format!("rollout-{id}.jsonl")), text).expect("vendor observation");
}

fn core(f: &Fixture) -> super::cli::Runner {
    let mut cmd = super::cli::ae();
    cmd.env("HOME", f.rig.scratch.join("home"))
        .env("AE_HOME", &f.rig.scratch)
        .env("CONFIG_FILE", f.rig.scratch.join("config"))
        .env("TMUX", format!("{},0,0", f.rig.sock.display()))
        .env("TMUX_PANE", &f.rig.main_pane)
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("AE_SENDER_OVERRIDE");
    cmd
}

fn events(f: &Fixture, action: &str) -> Vec<Event> {
    f.rig
        .events()
        .lines()
        .map(|line| Event::parse_line(line).expect("journal grammar"))
        .filter(|e| {
            e.action == action
                && ae::watchdog::event_is_addressed_to(e, &f.rig.session, f.slot, f.agent)
        })
        .collect()
}

fn until(f: &Fixture, ready: impl Fn() -> bool) {
    let limit = Instant::now() + WAIT;
    while Instant::now() < limit {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "observation barrier timed out: {}\n{}",
        f.rig.events(),
        std::fs::read_to_string(f.rig.scratch.join("quota-err")).unwrap_or_default()
    );
}

fn watch(f: &Fixture) -> super::cli::OwnedChild {
    let trace = f.rig.scratch.join("quota-trace");
    std::fs::write(&trace, "").expect("fresh quota barrier");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), f.rig.dir.join("send"))
        .expect("private helper");
    let log = std::fs::File::create(f.rig.scratch.join("quota-err")).expect("daemon stderr");
    core(f)
        .arg("_watchdog-run")
        .arg(&f.rig.dir)
        .args([
            "--interval",
            "1",
            "--stale-secs",
            "999999",
            "--tg-supervise-secs",
            "0",
            "--quota-every-secs",
            "1",
            "--idle-nudge-secs",
            "0",
        ])
        .env("AE_TEST_QUOTA_TRACE", trace)
        .env("AE_SEND_DEFER_SEC", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .spawn()
        .expect("real watchdog")
}

fn sweeps(f: &Fixture) {
    let count = || {
        std::fs::read_to_string(f.rig.scratch.join("quota-trace"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("observed "))
            .count()
    };
    let before = count();
    until(f, || count() >= before + 2);
}

fn opening(tag: &str, old: Sample<'_>, new: Sample<'_>, foreign: bool, expected: bool) {
    let f = setup(tag, false, "codex");
    publish(&f, NEW, &f.account, old);
    let other = if foreign {
        f.rig.scratch.join("foreign")
    } else {
        f.account.clone()
    };
    publish(&f, OLD, &other, new);
    let pid = f.rig.tool_pid(&f.pane, "codex");
    assert!(pid.is_some());
    let _watch = watch(&f);
    sweeps(&f);
    assert_eq!(
        !events(&f, "auto-reseat-headroom").is_empty(),
        expected,
        "only current usable evidence of the same account/window may open headroom: {}",
        f.rig.events()
    );
    assert!(
        events(&f, ATTEMPT_ACTION).is_empty(),
        "600s grace remains intact"
    );
    assert_eq!(f.rig.tool_pid(&f.pane, "codex"), pid);
}

#[test]
fn newer_fresh_low_prevents_opening_even_when_older_high_was_written_last() {
    let f = setup("shreversemtime", false, "codex");
    publish(&f, OLD, &f.account, sample(1, 10));
    publish(&f, NEW, &f.account, sample(95, 600));
    let _watch = watch(&f);
    sweeps(&f);
    assert!(
        events(&f, "auto-reseat-headroom").is_empty(),
        "superseded fresh high must not open: {}",
        f.rig.events()
    );
}

#[test]
fn newer_stale_low_supersedes_older_stale_high() {
    opening(
        "shstalelow",
        sample(95, 3_000),
        sample(1, 1_200),
        false,
        false,
    );
}

#[test]
fn proven_scope_without_own_rates_still_uses_current_high() {
    opening("shsharedhigh", sample(1, 600), sample(95, 10), false, true);
}

#[test]
fn foreign_account_low_does_not_clear_own_high() {
    opening("shforeignlow", sample(95, 600), sample(1, 10), true, true);
}

#[test]
fn foreign_account_high_does_not_press_own_low() {
    opening("shforeignhigh", sample(1, 600), sample(95, 10), true, false);
}

#[test]
fn newer_other_window_does_not_clear_high() {
    for (tag, row) in [
        (
            "shotherbucket",
            Sample {
                bucket: "codex_bengalfox",
                ..sample(1, 10)
            },
        ),
        (
            "shotherplan",
            Sample {
                plan: "plus",
                ..sample(1, 10)
            },
        ),
        (
            "shotherlength",
            Sample {
                minutes: 1_440,
                ..sample(1, 10)
            },
        ),
    ] {
        opening(tag, sample(95, 600), row, false, true);
    }
}

#[test]
fn equal_timestamp_low_does_not_supersede_high() {
    opening("shtie", sample(95, 10), sample(1, 10), false, true);
}

#[test]
fn newer_unknown_reset_does_not_supersede_usable_high() {
    opening(
        "shunknownreset",
        sample(95, 600),
        Sample {
            expired: true,
            ..sample(1, 10)
        },
        false,
        true,
    );
}

#[test]
fn missing_clock_low_cannot_supersede_known_high() {
    opening(
        "shclocklow",
        sample(95, 600),
        Sample {
            clock: false,
            ..sample(1, 0)
        },
        false,
        true,
    );
}

#[test]
fn missing_clock_high_cannot_turn_current_low_into_pressure() {
    opening(
        "shclockhigh",
        Sample {
            clock: false,
            ..sample(95, 0)
        },
        sample(1, 10),
        false,
        false,
    );
}

#[test]
fn unproven_seat_identity_cannot_borrow_correlated_high() {
    let f = setup("shnoidentity", false, "codex");
    publish(&f, OLD, &f.account, sample(95, 10));
    put(&f.rig, &format!("config_home.{}", f.slot), "unknown");
    let _watch = watch(&f);
    sweeps(&f);
    assert!(
        events(&f, "auto-reseat-headroom").is_empty(),
        "identity is required: {}",
        f.rig.events()
    );
}

fn seed(f: &Fixture, key: Timestamp, attempted: bool) {
    let mut text = format!(
        "{{\"ts\":\"{key}\",\"actor\":\"watchdog\",\"action\":\"auto-reseat-headroom\",\"target\":\"{}\",\"target_slot\":\"{}\",\"target_session\":\"{}\",\"summary\":\"headroom 95% codex (pro) 7d\"}}\n",
        f.agent, f.slot, f.rig.session
    );
    if attempted {
        writeln!(text, "{{\"ts\":\"{}\",\"actor\":\"watchdog\",\"action\":\"{ATTEMPT_ACTION}\",\"target\":\"{}\",\"target_slot\":\"{}\",\"target_session\":\"{}\",\"ref\":\"{key}\"}}", Timestamp::from_epoch(key.epoch() + 1), f.agent, f.slot, f.rig.session).expect("attempt line");
    }
    std::fs::write(f.rig.dir.join("events.jsonl"), text).expect("valid temporal fixture");
    let ledger: Vec<_> = f
        .rig
        .events()
        .lines()
        .map(|line| Event::parse_line(line).expect("seed grammar"))
        .collect();
    let episode = ae::autoreseat::keyed(&ledger, (&f.rig.session, f.slot, f.agent), key)
        .expect("valid keyed headroom episode");
    assert_eq!(
        episode.open.is_some(),
        attempted,
        "seed is a product-valid attempt"
    );
}

#[test]
fn pending_episode_rearms_when_newer_usable_low_arrives() {
    let f = setup("shpending", false, "codex");
    publish(&f, OLD, &f.account, sample(95, 1_200));
    seed(&f, Timestamp::from_epoch(f.now - 700), false);
    // Keep the process busy to avoid an unwanted base-tree move while checking rearm.
    std::fs::write(f.rig.scratch.join("__BUSY__"), "").expect("busy vendor frame");
    until(&f, || {
        ae::harness_state::classify(&f.rig.capture(&f.pane), ae::tool::ToolKind::Codex)
            == ae::harness_state::HarnessState::Busy
    });
    publish(&f, NEW, &f.account, sample(1, 10));
    let _watch = watch(&f);
    sweeps(&f);
    assert_eq!(
        events(&f, "auto-reseat-headroom-cleared").len(),
        1,
        "superseded high cannot keep an episode due: {}",
        f.rig.events()
    );
    assert!(events(&f, ATTEMPT_ACTION).is_empty());
}

fn detached(tag: &str, state: &str) {
    let f = setup(tag, false, "codex");
    publish(&f, OLD, &f.account, sample(95, 1_200));
    let key = Timestamp::from_epoch(f.now - 700);
    seed(&f, key, true);
    // This publication happens AFTER reservation, BEFORE the real leg rereads.
    if state == "low" {
        publish(&f, NEW, &f.account, sample(1, 10));
    }
    if state == "unread" {
        std::fs::write(
            f.account
                .join(format!("sessions/2026/10/20/rollout-{OLD}.jsonl")),
            "not-json\n",
        )
        .expect("source becomes unreadable");
    }
    if state == "absent" {
        std::fs::remove_file(
            f.account
                .join(format!("sessions/2026/10/20/rollout-{OLD}.jsonl")),
        )
        .expect("source has no usable quota");
    }
    let pid = f.rig.tool_pid(&f.pane, "codex");
    assert!(pid.is_some());
    let out = core(&f)
        .arg("_auto-reseat")
        .arg(&f.rig.dir)
        .args([f.slot, &key.to_string()])
        .output()
        .expect("real detached leg");
    if state == "high" {
        assert_eq!(
            out.status.code(),
            Some(0),
            "healthy high must reach a real move: {out:?} {}",
            f.rig.events()
        );
        assert_eq!(
            events(&f, DONE_ACTION).len(),
            1,
            "calibrate that lifecycle gates allow a move"
        );
        assert_eq!(
            f.rig.meta_row(&format!("profile.{}", f.slot)),
            "fake-opencode"
        );
    } else {
        assert_eq!(
            f.rig.tool_pid(&f.pane, "codex"),
            pid,
            "leg must recheck source quota before stopping the process: {}",
            f.rig.events()
        );
        assert_eq!(f.rig.meta_row(&format!("profile.{}", f.slot)), "source");
        assert_eq!(out.status.code(), Some(1), "held leg exit: {out:?}");
        let held = events(&f, HELD_ACTION);
        assert_eq!(
            held.len(),
            1,
            "source relief/Unread must HOLD: {}",
            f.rig.events()
        );
        assert_eq!(held[0].reference.as_deref(), Some(key.to_string().as_str()));
        for action in [DONE_ACTION, REFUSED_ACTION, FAILED_ACTION] {
            assert!(
                events(&f, action).is_empty(),
                "unexpected {action}: {}",
                f.rig.events()
            );
        }
    }
}

#[test]
fn detached_leg_rechecks_new_low_after_attempt_was_reserved() {
    detached("shleglow", "low");
}
#[test]
fn detached_leg_holds_when_source_quota_becomes_unreadable() {
    detached("shlegunread", "unread");
}
#[test]
fn detached_leg_holds_when_nothing_usable_remains() {
    detached("shlegabsent", "absent");
}
#[test]
fn detached_leg_still_moves_on_current_high() {
    detached("shleghigh", "high");
}

fn chooser(tag: &str, foreign_low: bool, current_high: bool) {
    let f = setup(tag, false, "claude");
    let now = f.now;
    std::fs::write(f.account.join(".claude.json"), format!("{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":95,\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}", now * 1000, Timestamp::from_epoch(now + 86_400))).expect("source high");
    publish(&f, OLD, &f.account, sample(95, 1_200));
    let account = if foreign_low {
        f.rig.scratch.join("foreign")
    } else {
        f.account.clone()
    };
    publish(
        &f,
        NEW,
        &account,
        sample(if current_high { 95 } else { 1 }, 10),
    );
    let path = f.rig.scratch.join("config");
    let config = std::fs::read_to_string(&path)
        .expect("config")
        .replace("source = fake-opencode", "source = target, fake-opencode");
    std::fs::write(path, config).expect("chooser candidates");
    let key = Timestamp::from_epoch(now - 700);
    seed(&f, key, true);
    let out = core(&f)
        .arg("_auto-reseat")
        .arg(&f.rig.dir)
        .args([f.slot, &key.to_string()])
        .output()
        .expect("real chooser leg");
    assert_eq!(out.status.code(), Some(0), "{out:?} {}", f.rig.events());
    assert_eq!(
        f.rig.meta_row(&format!("profile.{}", f.slot)),
        if foreign_low || current_high {
            "fake-opencode"
        } else {
            "target"
        },
        "chooser must use current evidence of each proven account separately: {}",
        f.rig.events()
    );
}

#[test]
fn chooser_accepts_candidate_with_newer_usable_low() {
    chooser("shchooser", false, false);
}

#[test]
fn chooser_still_skips_candidate_on_current_high() {
    chooser("shchooserhigh", false, true);
}

#[test]
fn chooser_cannot_clear_account_high_with_another_accounts_low() {
    chooser("shchooserforeign", true, false);
}

#[test]
fn effort_consumer_ignores_superseded_high() {
    let f = setup("sheffort", true, "codex");
    publish(&f, OLD, &f.account, sample(85, 600));
    publish(&f, NEW, &f.account, sample(1, 10));
    let pid = f.rig.tool_pid(&f.pane, "codex");
    let _daemon = watch(&f);
    sweeps(&f);
    assert!(
        events(&f, "effort-episode").is_empty(),
        "auto effort must consume newest usable evidence: {}",
        f.rig.events()
    );
    assert_eq!(f.rig.tool_pid(&f.pane, "codex"), pid);
}

#[test]
fn effort_consumer_still_steps_down_for_current_high() {
    let f = setup("shefforthigh", true, "codex");
    publish(&f, OLD, &f.account, sample(1, 600));
    publish(&f, NEW, &f.account, sample(85, 10));
    let pid = f.rig.tool_pid(&f.pane, "codex");
    let _watch = watch(&f);
    until(&f, || !events(&f, "effort-applied").is_empty());
    assert_eq!(
        events(&f, "effort-episode").len(),
        1,
        "current high still opens effort episode: {}",
        f.rig.events()
    );
    assert_ne!(
        f.rig.tool_pid(&f.pane, "codex"),
        pid,
        "positive effort control must restart the real process"
    );
}
