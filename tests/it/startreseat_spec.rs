//! Frozen acceptance: restart promptly leaves fresh exhausted capacity.
//! Installed as a child of `tests/it/session_launch.rs` to reuse its private `Rig`.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures read and write only their owned scratch stores"
)]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION, FAILED_ACTION, REFUSED_ACTION};
use ae::events::Event;
use ae::time::Timestamp;

use super::super::cli::{OwnedChild, ae};
use super::Rig;

// Well below the configured 600-second grace; includes quota checkpoint delivery.
const PROMPT_BOUND: Duration = Duration::from_secs(90);

const AGENT: &str = r#"use strict;
use warnings;
$| = 1;
open(my $out, '>>', '__LOG__') or die;
my @logged = map { my $arg = $_; $arg =~ s/[\r\n]/ /g; $arg } @ARGV;
print $out ($ENV{CLAUDE_CONFIG_DIR} // ''), "|", join(' ', @logged), "\n";
close($out);
system('stty raw -echo 2>/dev/null');
binmode(STDIN, ':raw');
binmode(STDOUT, ':raw');
my $bar = "\xe2\x94\x80" x 60;
my $effort = '';
my $pinned = 0;
for (my $i=0; $i<@ARGV; $i++) {
    $effort = $ARGV[$i+1] if $ARGV[$i] eq '--effort';
    $pinned = 1 if $ARGV[$i] eq '--model';
}
print "\e[H\e[2J\xe2\x9c\xbb Fake \xc2\xb7 done (0s)\r\n";
print "$bar\r\n\xe2\x9d\xaf \r\n$bar\r\n";
my $identity = !$pinned ? 'Unbound fixture' : $effort eq '' ? 'Opus 5' : "Opus 5.5 ($effort)";
print "\xf0\x9f\xa7\xa0 $identity  \xf0\x9f\x93\x81 work\r\n\xe2\x8f\xb5\xe2\x8f\xb5 accept edits on\r\n";
while (1) {
    my $ready = '';
    vec($ready, fileno(STDIN), 1) = 1;
    if (select($ready, undef, undef, 0.05) > 0) {
        my $chunk = '';
        sysread(STDIN, $chunk, 4096);
        open(my $received, '>>', '__RECEIVED__') or die;
        print $received $chunk;
        close($received);
    }
}
"#;

fn fixture(tag: &str, knobs: &str, candidates: &str) -> Rig {
    let mut rig = Rig::new(tag, &["claude"], None);
    // Launch metadata and the fake vendor's transcript use one physical cwd.
    // macOS /tmp is a symlink; effort probes the recorded work_dir literally.
    rig.project = std::fs::canonicalize(&rig.project).expect("physical launch project");
    // A copied interpreter reports the actual tool name to ps and tmux on both OSes.
    std::fs::copy("/usr/bin/perl", rig.bin.join("claude")).expect("owned interpreter copy");
    let script = rig.scratch.join("agent.pl");
    std::fs::write(
        &script,
        AGENT
            .replace("__LOG__", &rig.launched.display().to_string())
            .replace(
                "__RECEIVED__",
                &rig.scratch.join("received").display().to_string(),
            ),
    )
    .expect("owned fake tool");
    for name in ["source", "unavailable", "destination"] {
        std::fs::create_dir_all(rig.scratch.join(name)).expect("owned account home");
    }
    let pin = if knobs.contains("model-test") {
        " --model opus"
    } else {
        ""
    };
    let knobs = knobs.replace("model-test\n", "");
    std::fs::write(
        &rig.config,
        format!(
            "[profiles]\nsource = \"CLAUDE_CONFIG_DIR={} {} {}{pin}\"\nunavailable = \"CLAUDE_CONFIG_DIR={} {} {}{pin}\"\ndestination = \"CLAUDE_CONFIG_DIR={} {} {}{pin}\"\n[roster]\nlead = source\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\nchat = off\nquota_every_secs = 1\nauto_reseat = all\nauto_reseat_grace_secs = 600\nauto_reseat_at = off\n{knobs}[auto_reseat]\nsource = {candidates}\n",
            rig.scratch.join("source").display(), rig.bin.join("claude").display(), script.display(),
            rig.scratch.join("unavailable").display(), rig.bin.join("claude").display(), script.display(),
            rig.scratch.join("destination").display(), rig.bin.join("claude").display(), script.display(),
        ),
    )
    .expect("explicit restart policy");
    quota(&rig, "source", "session", 10, 0, None);
    quota(&rig, "unavailable", "session", 100, 0, None);
    quota(&rig, "destination", "session", 10, 0, None);
    rig
}

fn quota(rig: &Rig, account: &str, kind: &str, used: u8, age: i64, model: Option<&str>) {
    let now = Timestamp::now().epoch();
    let scope = model.map_or_else(
        || "null".to_owned(),
        |model| format!("{{\"model\":{{\"display_name\":\"{model}\"}}}}"),
    );
    let healthy_long = if kind == "session" {
        format!(
            ",{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":10,\"resets_at\":\"{}\",\"scope\":null}}",
            Timestamp::from_epoch(now + 86_400),
        )
    } else {
        String::new()
    };
    std::fs::write(
        rig.scratch.join(account).join(".claude.json"),
        format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"{kind}\",\"group\":\"{kind}\",\"percent\":{used},\"resets_at\":\"{}\",\"scope\":{scope}}}{healthy_long}]}}}}}}\n",
            (now - age) * 1_000, Timestamp::from_epoch(now + 86_400),
        ),
    )
    .expect("owned vendor cache");
    if let Some(model) = model {
        // Calibrate through the real reader: a malformed scoped fixture must
        // not make the unrelated-model negative pass as an Unknown reading.
        let reading = ae()
            .args(["quota", "--json"])
            .env("HOME", &rig.scratch)
            .env("AE_HOME", &rig.home)
            .env("CONFIG_FILE", &rig.config)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .output()
            .expect("real scoped quota observation");
        assert!(reading.status.success(), "{reading:?}");
        let text = String::from_utf8(reading.stdout).expect("quota JSON");
        let doc = ae::json::parse(&text).expect("quota JSON grammar");
        let Some(ae::json::Value::Arr(scopes)) = doc.get("scopes") else {
            panic!("quota scopes: {text}");
        };
        assert!(scopes.iter().any(|scope| {
            let owns_account = matches!(scope.get("profiles"), Some(ae::json::Value::Arr(names)) if names.iter().any(|name| name.as_str() == Some(account)));
            owns_account && matches!(scope.get("windows"), Some(ae::json::Value::Arr(windows)) if windows.iter().any(|window| {
                window.get_str("bucket") == Some(kind)
                    && window.get_str("qualifier") == Some(model)
                    && window.get_str("status") == Some("fresh")
                    && window.get("effective_percent") == Some(&ae::json::Value::Num(i64::from(used)))
            }))
        }), "fixture must expose a fresh scoped percentage on its own account: {text}");
    }
}

fn events(rig: &Rig, session: &str) -> Vec<Event> {
    std::fs::read_to_string(rig.dir(session).join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| Event::parse_line(line).expect("valid product journal"))
        .collect()
}

fn booked(rig: &Rig, session: &str, action: &str) -> Vec<Event> {
    booked_slot(rig, session, action, "main", "lead")
}

fn booked_slot(rig: &Rig, session: &str, action: &str, slot: &str, actor: &str) -> Vec<Event> {
    events(rig, session)
        .into_iter()
        .filter(|event| {
            event.action == action
                && ae::watchdog::event_is_addressed_to(event, session, slot, actor)
        })
        .collect()
}

fn profile(rig: &Rig, session: &str) -> String {
    row(&rig.meta(session), "profile.main")
}

fn account_argv(line: &str, rig: &Rig, account: &str) -> bool {
    let (home, _) = line
        .split_once('|')
        .expect("fake log separates home and argv");
    let expected = rig.scratch.join(account);
    std::fs::canonicalize(home).expect("logged account home")
        == std::fs::canonicalize(expected).expect("owned account home")
}

fn row(meta: &str, key: &str) -> String {
    meta.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .unwrap_or_default()
        .to_owned()
}

fn launch(rig: &Rig, session: &str) {
    launch_with_frame(rig, session, ae::harness_state::HarnessState::Idle);
}

fn launch_with_frame(rig: &Rig, session: &str, expected: ae::harness_state::HarnessState) {
    let result = rig.launch(&["--local", session]);
    assert_eq!(result.0, Some(0), "{result:?}");
    let pane = row(&rig.meta(session), "main_pane");
    let capture = || {
        let (ok, frame) = rig.tmux(&["capture-pane", "-p", "-t", &pane]);
        ok.then_some(frame)
    };
    until(rig, session, || {
        capture().is_some_and(|frame| {
            ae::harness_state::read_frame(&frame, ae::tool::ToolKind::Claude)
                .is_some_and(|read| read.state == expected)
        })
    });
    let config = std::fs::read_to_string(&rig.config).expect("fixture profile config");
    if !config.contains("--model") {
        let frame = capture().expect("owned live frame");
        assert!(
            ae::harness_state::observed_identity(&frame, ae::tool::ToolKind::Claude)
                .model
                .is_none(),
            "unbound fixture must prove idle without inventing a model choice: {frame}"
        );
    }
}

/// Sanitized Claude Code 2.1.316 resume: transcript and transient hint, empty
/// box and measured footer shape, with no finished-turn row after a resume.
fn draw_real_resume(rig: &Rig, fresh_too: bool) {
    let script = rig.scratch.join("agent.pl");
    let bytes = std::fs::read_to_string(&script).expect("owned fake");
    let done = r#"print "\e[H\e[2J\xe2\x9c\xbb Fake \xc2\xb7 done (0s)\r\n";"#;
    assert_eq!(bytes.matches(done).count(), 1, "one renderer to replace");
    let quiet = if fresh_too {
        r#"print "\e[H\e[2JClaude Code v2.1.316\r\n";"#
    } else {
        r#"my $resumed = grep { $_ eq '--resume' } @ARGV;
if ($resumed) {
    print "\e[H\e[2J\xe2\x8f\xba ok\r\n";
    print "tmux detected - scroll with PgUp/PgDn\r\n";
} else {
    print "\e[H\e[2J\xe2\x9c\xbb Fake \xc2\xb7 done (0s)\r\n";
}"#
    };
    std::fs::write(script, bytes.replace(done, quiet)).expect("real resume rendering");
}

/// Use actual launch metadata, then plant the transcript the fake tool does not emit.
fn retained(rig: &Rig, session: &str) -> (String, PathBuf) {
    retained_slot(rig, session, "main")
}

fn retained_slot(rig: &Rig, session: &str, slot: &str) -> (String, PathBuf) {
    let meta = rig.meta(session);
    let sid = row(&meta, &format!("harness_session.{slot}"));
    let id = sid.strip_prefix("claude:").unwrap_or(&sid);
    assert_eq!(id.len(), 36, "a real launch id: {meta}");
    let relative = PathBuf::from("projects")
        .join(ae::carry::project_key(
            &std::fs::canonicalize(&rig.project).expect("project"),
        ))
        .join(format!("{id}.jsonl"));
    let source = rig.scratch.join("source").join(&relative);
    std::fs::create_dir_all(source.parent().expect("transcript parent")).expect("owned store");
    std::fs::write(source, "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n")
        .expect("retained conversation");
    (id.to_owned(), relative)
}

fn stop(rig: &Rig, session: &str) {
    assert!(
        rig.tmux(&["kill-session", "-t", &format!("={session}")]).0,
        "owned session stops"
    );
}

fn watch(rig: &Rig, session: &str) -> OwnedChild {
    watch_every(rig, session, "1")
}

fn watch_every(rig: &Rig, session: &str, quota_every: &str) -> OwnedChild {
    let trace = rig.scratch.join("restart-trace");
    std::fs::write(&trace, "").expect("fresh daemon barrier");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rig.scratch.join("restart-daemon-err"))
        .expect("daemon log");
    ae().arg("_watchdog-run")
        .arg(rig.dir(session))
        .args([
            "--interval",
            "1",
            "--stale-secs",
            "999999",
            "--tg-supervise-secs",
            "0",
            "--quota-every-secs",
            quota_every,
        ])
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env("AE_TEST_QUOTA_TRACE", trace)
        .env("AE_SEND_DEFER_SEC", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("AE_SENDER_OVERRIDE")
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .spawn()
        .expect("owned watchdog")
}

fn until(rig: &Rig, session: &str, condition: impl Fn() -> bool) {
    let until = Instant::now() + PROMPT_BOUND;
    while Instant::now() < until {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "restart contract timed out; meta={}\nevents={:?}\ndaemon={}",
        rig.meta(session),
        events(rig, session),
        std::fs::read_to_string(rig.scratch.join("restart-daemon-err")).unwrap_or_default()
    );
}

fn sweeps(rig: &Rig, session: &str) {
    let completed = || {
        std::fs::read_to_string(rig.scratch.join("restart-trace"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("observed "))
            .count()
    };
    let before = completed();
    until(rig, session, || completed() >= before + 12);
}

#[test]
fn stopped_restart_moves_fresh_short_exhaustion_before_grace_and_carries_exact_resume_once() {
    let session = "restartshort";
    let rig = fixture(session, "", "unavailable, destination");
    launch(&rig, session);
    let (id, relative) = retained(&rig, session);
    quota(&rig, "source", "session", 100, 0, None);
    stop(&rig, session);
    let start = Instant::now();
    launch(&rig, session);
    let daemon = watch(&rig, session);
    until(&rig, session, || {
        booked(&rig, session, DONE_ACTION).len() == 1
    });
    assert!(
        start.elapsed() < PROMPT_BOUND,
        "600-second grace must not delay exhausted restart"
    );
    assert_eq!(
        profile(&rig, session),
        "destination",
        "first usable declared candidate"
    );
    assert!(
        rig.scratch.join("destination").join(relative).is_file(),
        "same transcript carried"
    );
    let argv = rig.launch_argv();
    assert!(
        argv.lines()
            .any(|line| account_argv(line, &rig, "destination")
                && line.contains(&format!("--resume {id}"))),
        "exact destination resume: {argv}"
    );
    assert!(
        !events(&rig, session)
            .iter()
            .any(|event| event.action == "limit"),
        "no failed turn is needed"
    );
    sweeps(&rig, session);
    drop(daemon);
    let _again = watch(&rig, session);
    sweeps(&rig, session);
    assert_eq!(
        booked(&rig, session, DONE_ACTION).len(),
        1,
        "completion survives daemon restart"
    );
    assert_eq!(
        booked(&rig, session, ATTEMPT_ACTION).len(),
        1,
        "one move per episode"
    );
}

#[test]
fn real_resumed_quiet_box_moves_without_a_done_row_and_passes_the_final_stop() {
    let session = "restartquiet";
    let rig = fixture(session, "", "destination");
    draw_real_resume(&rig, false);
    launch(&rig, session);
    let (id, relative) = retained(&rig, session);
    quota(&rig, "source", "session", 100, 0, None);
    stop(&rig, session);
    launch_with_frame(&rig, session, ae::harness_state::HarnessState::Unknown);
    let pane = row(&rig.meta(session), "main_pane");
    let (ok, capture) = rig.tmux(&["capture-pane", "-p", "-t", &pane]);
    assert!(
        ok && capture.contains("⏺ ok"),
        "real transcript shape: {capture}"
    );
    assert!(!capture.contains(" · done "), "no invented idle proof");
    assert!(!row(&rig.meta(session), "launch_settled").is_empty());
    let _daemon = watch(&rig, session);
    until(&rig, session, || {
        booked(&rig, session, DONE_ACTION).len() == 1
    });
    assert_eq!(profile(&rig, session), "destination");
    assert!(rig.scratch.join("destination").join(relative).is_file());
    let argv = rig.launch_argv();
    assert!(
        argv.lines()
            .any(|line| account_argv(line, &rig, "destination")
                && line.contains(&format!("--resume {id}"))),
        "final stop must accept the same quiet proof and preserve conversation: {argv}"
    );
    assert_eq!(booked(&rig, session, ATTEMPT_ACTION).len(), 1);
    assert!(
        !events(&rig, session)
            .iter()
            .any(|event| event.action == "limit"),
        "quiet proof needs no failed turn"
    );
    sweeps(&rig, session);
    assert_eq!(booked(&rig, session, DONE_ACTION).len(), 1);
}

#[test]
fn a_fresh_logo_and_empty_box_without_resume_settlement_do_not_gain_a_quiet_waiver() {
    let session = "freshquiet";
    let rig = fixture(session, "", "destination");
    draw_real_resume(&rig, true);
    quota(&rig, "source", "session", 100, 0, None);
    launch_with_frame(&rig, session, ae::harness_state::HarnessState::Unknown);
    assert!(row(&rig.meta(session), "launch_settled").is_empty());
    let _daemon = watch(&rig, session);
    sweeps(&rig, session);
    assert_eq!(profile(&rig, session), "source");
    assert!(booked(&rig, session, ATTEMPT_ACTION).is_empty());
    assert!(booked(&rig, session, ae::autoreseat::HEADROOM_ACTION).is_empty());
}

#[test]
fn real_resumed_quiet_box_without_exhaustion_keeps_the_ordinary_unproven_hold() {
    let session = "restartquietnear";
    let rig = fixture(session, "", "destination");
    draw_real_resume(&rig, false);
    let config = std::fs::read_to_string(&rig.config).expect("config");
    std::fs::write(
        &rig.config,
        config
            .replace("auto_reseat_at = off", "auto_reseat_at = 95")
            .replace("auto_reseat_grace_secs = 600", "auto_reseat_grace_secs = 0"),
    )
    .expect("ordinary headroom with zero configured quota wait");
    launch(&rig, session);
    retained(&rig, session);
    quota(&rig, "source", "weekly_all", 96, 0, None);
    stop(&rig, session);
    launch_with_frame(&rig, session, ae::harness_state::HarnessState::Unknown);
    assert!(!row(&rig.meta(session), "launch_settled").is_empty());
    let _daemon = watch(&rig, session);
    until(&rig, session, || {
        !booked(&rig, session, ae::autoreseat::HEADROOM_ACTION).is_empty()
    });
    sweeps(&rig, session);
    assert_eq!(profile(&rig, session), "source");
    assert!(booked(&rig, session, ATTEMPT_ACTION).is_empty());
    assert!(booked(&rig, session, DONE_ACTION).is_empty());
    assert!(
        booked(&rig, session, ae::autoreseat::HELD_ACTION)
            .iter()
            .any(|event| event
                .summary
                .as_deref()
                .is_some_and(|summary| summary.starts_with("unknown-frame:"))),
        "nonexhausted ordinary headroom cannot use startup quiet proof, even after its configured quota wait expires"
    );
}

#[test]
fn a_restarted_lead_pair_on_one_exhausted_login_moves_both_seats_once_inside_the_window() {
    let session = "restartpair";
    let rig = fixture(session, "", "destination");
    let config = std::fs::read_to_string(&rig.config).expect("config");
    std::fs::write(
        &rig.config,
        config
            .replace("lead = source\n", "lead = source\ncolead = source\n")
            .replace("layout = vertical", "layout = lead-pair\nworkers = colead"),
    )
    .expect("real paired roster");
    launch(&rig, session);
    let conversations = [
        retained_slot(&rig, session, "main"),
        retained_slot(&rig, session, "worker.0"),
    ];
    assert_ne!(conversations[0].0, conversations[1].0);
    quota(&rig, "source", "session", 100, 0, None);
    stop(&rig, session);
    launch(&rig, session);
    let start = Instant::now();
    let _daemon = watch(&rig, session);
    until(&rig, session, || {
        [("main", "lead"), ("worker.0", "colead")]
            .iter()
            .all(|(slot, actor)| booked_slot(&rig, session, DONE_ACTION, slot, actor).len() == 1)
    });
    assert!(start.elapsed() < PROMPT_BOUND);
    let meta = rig.meta(session);
    let settled: i64 = row(&meta, "launch_settled")
        .parse()
        .expect("resume settlement");
    for ((slot, actor), (id, relative)) in [("main", "lead"), ("worker.0", "colead")]
        .into_iter()
        .zip(conversations)
    {
        assert_eq!(row(&meta, &format!("profile.{slot}")), "destination");
        assert_eq!(row(&meta, &format!("harness_session.{slot}")), id);
        assert!(rig.scratch.join("destination").join(relative).is_file());
        let done = booked_slot(&rig, session, DONE_ACTION, slot, actor);
        assert!(
            done[0].ts.epoch() <= settled + 360,
            "second seat cannot miss startup"
        );
        assert_eq!(
            booked_slot(&rig, session, ATTEMPT_ACTION, slot, actor).len(),
            1
        );
        assert!(booked_slot(&rig, session, REFUSED_ACTION, slot, actor).is_empty());
    }
    sweeps(&rig, session);
    for (slot, actor) in [("main", "lead"), ("worker.0", "colead")] {
        assert_eq!(
            booked_slot(&rig, session, DONE_ACTION, slot, actor).len(),
            1
        );
        assert_eq!(
            booked_slot(&rig, session, ATTEMPT_ACTION, slot, actor).len(),
            1
        );
        assert!(booked_slot(&rig, session, REFUSED_ACTION, slot, actor).is_empty());
    }
}

#[test]
fn raw_exhaustion_relieved_by_declared_manual_resets_does_not_move_at_restart() {
    let session = "restartresets";
    let rig = fixture(session, "", "destination");
    let config = std::fs::read_to_string(&rig.config).expect("config");
    std::fs::write(
        &rig.config,
        format!(
            "[clients]\nreset-account = claude config_home={} manual_resets=1\n{config}",
            rig.scratch.join("source").display()
        ),
    )
    .expect("declared own-account reset");
    launch(&rig, session);
    retained(&rig, session);
    quota(&rig, "source", "session", 100, 0, None);
    stop(&rig, session);
    launch(&rig, session);
    // Verify the real reader resolves raw 100 to effective 50 for this profile.
    let reading = ae()
        .args(["quota", "--json"])
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .output()
        .expect("real quota observation");
    assert!(reading.status.success(), "{reading:?}");
    let text = String::from_utf8(reading.stdout).expect("quota JSON");
    let doc = ae::json::parse(&text).expect("quota JSON grammar");
    let Some(ae::json::Value::Arr(scopes)) = doc.get("scopes") else {
        panic!("quota scopes: {text}");
    };
    assert!(scopes.iter().any(|scope| {
        let owns_source = matches!(scope.get("profiles"), Some(ae::json::Value::Arr(names)) if names.iter().any(|name| name.as_str() == Some("source")));
        owns_source && matches!(scope.get("windows"), Some(ae::json::Value::Arr(windows)) if windows.iter().any(|window| {
            window.get("used_percent") == Some(&ae::json::Value::Num(100))
                && window.get("effective_percent") == Some(&ae::json::Value::Num(50))
        }))
    }), "fixture must prove effective relief on the seat's own profile: {text}");
    let _daemon = watch(&rig, session);
    sweeps(&rig, session);
    assert_eq!(profile(&rig, session), "source");
    assert!(
        booked(&rig, session, ATTEMPT_ACTION).is_empty(),
        "raw percentage is not urgency"
    );
}

fn effort_fixture(session: &str, reseat: bool) -> Rig {
    let rig = fixture(session, "auto_effort = on\n", "destination");
    let config = std::fs::read_to_string(&rig.config).expect("config");
    let source = config
        .lines()
        .find(|line| line.starts_with("source = \""))
        .expect("source profile");
    let source = source.trim_end_matches('"');
    let config = config.replace(
        "[roster]",
        &format!(
            "{} --model opus --effort high\"\n[roster]",
            source.replacen("source =", "source-low =", 1)
        ),
    );
    let config = config
        .lines()
        .map(|line| {
            if line.starts_with("source = \"") {
                format!(
                    "{} --model opus --effort xhigh\"",
                    line.trim_end_matches('"')
                )
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let config = config.replace("auto_reseat_at = off", "auto_reseat_at = 90");
    let config = if reseat {
        config
    } else {
        config.replace("auto_reseat = all", "auto_reseat = off")
    };
    ae::config::parse_identity(&config).expect("real effort twin profiles");
    std::fs::write(&rig.config, config).expect("effort and reseat policy");
    rig
}

fn restart_effort_fixture(rig: &Rig, session: &str) -> String {
    launch(rig, session);
    let (id, _) = retained(rig, session);
    quota(rig, "source", "weekly_all", 100, 0, None);
    stop(rig, session);
    launch(rig, session);
    let pane = row(&rig.meta(session), "main_pane");
    let (ok, frame) = rig.tmux(&["capture-pane", "-p", "-t", &pane]);
    assert!(ok);
    assert_eq!(
        ae::harness_state::observed_identity(&frame, ae::tool::ToolKind::Claude)
            .effort
            .as_deref(),
        Some("xhigh"),
        "effort arbitration must use a genuinely readable vendor frame: {frame}"
    );
    id
}

#[test]
fn effort_control_restarts_the_same_exhausted_fixture_when_reseating_is_off() {
    let session = "restarteffcontrol";
    let rig = effort_fixture(session, false);
    let id = restart_effort_fixture(&rig, session);
    let _daemon = watch(&rig, session);
    until(&rig, session, || {
        booked(&rig, session, "effort-applied").len() == 1
    });
    assert_eq!(profile(&rig, session), "source");
    assert_eq!(row(&rig.meta(session), "harness_session.main"), id);
    assert!(
        rig.launch_argv()
            .lines()
            .any(|line| line.contains("--effort high") && line.contains(&format!("--resume {id}")))
    );
    assert!(booked(&rig, session, ATTEMPT_ACTION).is_empty());
}

#[test]
fn exhausted_restart_reseat_wins_over_enabled_effort_without_restarting_the_dead_account() {
    let session = "restarteffort";
    let rig = effort_fixture(session, true);
    let id = restart_effort_fixture(&rig, session);
    let source_before = rig
        .launch_argv()
        .lines()
        .filter(|line| account_argv(line, &rig, "source"))
        .count();
    assert_eq!(
        source_before, 2,
        "create and restart both ran on the source"
    );
    let _daemon = watch(&rig, session);
    until(&rig, session, || {
        booked(&rig, session, DONE_ACTION).len() == 1
    });
    sweeps(&rig, session);
    assert_eq!(profile(&rig, session), "destination");
    assert_eq!(row(&rig.meta(session), "harness_session.main"), id);
    assert_eq!(booked(&rig, session, ATTEMPT_ACTION).len(), 1);
    assert!(
        booked(&rig, session, "effort-reserved").is_empty(),
        "urgent reseat owns the move"
    );
    assert!(booked(&rig, session, "effort-applied").is_empty());
    assert_eq!(
        rig.launch_argv()
            .lines()
            .filter(|line| account_argv(line, &rig, "source"))
            .count(),
        source_before,
        "no effort restart on the exhausted source"
    );
    assert!(rig.launch_argv().lines().any(|line| {
        account_argv(line, &rig, "destination") && line.contains(&format!("--resume {id}"))
    }));
}

fn assert_unproven_reading_holds(case: &str) {
    let session = format!("restart{case}");
    let rig = fixture(
        &session,
        if matches!(case, "othermodel" | "ambiguousmodel") {
            "model-test\n"
        } else {
            ""
        },
        "destination",
    );
    launch(&rig, &session);
    retained(&rig, &session);
    match case {
        "stale" => quota(&rig, "source", "session", 100, 901, None),
        "missing" => {
            std::fs::remove_file(rig.scratch.join("source/.claude.json")).expect("missing source");
        }
        "unknown" => quota(&rig, "source", "unrecognised", 100, 0, None),
        "otheraccount" => quota(&rig, "unavailable", "session", 100, 0, None),
        "othermodel" | "unboundmodel" => {
            quota(&rig, "source", "weekly_scoped", 100, 0, Some("Sonnet"));
        }
        "ambiguousmodel" => quota(&rig, "source", "weekly_scoped", 100, 0, Some("mystery")),
        "switchedaccount" => {
            quota(&rig, "source", "session", 100, 0, None);
            let path = rig.scratch.join("source/.claude.json");
            let text = std::fs::read_to_string(&path).expect("cache");
            let text = text.replacen("{\"cachedUsageUtilization\":{", "{\"oauthAccount\":{\"accountUuid\":\"current\"},\"cachedUsageUtilization\":{\"accountUuid\":\"previous\",", 1);
            std::fs::write(path, text).expect("account-switch provenance");
        }
        _ => unreachable!(),
    }
    stop(&rig, &session);
    launch(&rig, &session);
    let _daemon = watch(&rig, &session);
    sweeps(&rig, &session);
    assert_eq!(profile(&rig, &session), "source", "{case}");
    assert!(
        booked(&rig, &session, ATTEMPT_ACTION).is_empty(),
        "{case}: {:?}",
        events(&rig, &session)
    );
    assert!(
        booked(&rig, &session, ae::autoreseat::HEADROOM_ACTION).is_empty(),
        "{case}: an unproven reading cannot even open an urgent episode"
    );
}

#[test]
fn restart_does_not_treat_stale_reading_as_exhausted() {
    assert_unproven_reading_holds("stale");
}

#[test]
fn restart_does_not_treat_missing_reading_as_exhausted() {
    assert_unproven_reading_holds("missing");
}

#[test]
fn restart_does_not_treat_unknown_bucket_reading_as_exhausted() {
    assert_unproven_reading_holds("unknown");
}

#[test]
fn restart_does_not_treat_another_account_reading_as_exhausted() {
    assert_unproven_reading_holds("otheraccount");
}

#[test]
fn restart_does_not_treat_another_model_reading_as_exhausted() {
    assert_unproven_reading_holds("othermodel");
}

#[test]
fn restart_does_not_treat_unknown_model_reading_as_exhausted() {
    assert_unproven_reading_holds("ambiguousmodel");
}

#[test]
fn restart_does_not_treat_unbound_model_reading_as_exhausted() {
    assert_unproven_reading_holds("unboundmodel");
}

#[test]
fn restart_does_not_treat_switched_account_reading_as_exhausted() {
    assert_unproven_reading_holds("switchedaccount");
}

#[test]
fn restart_respects_auto_reseat_off_scope_and_main_exclusion() {
    for (case, from, to) in [
        ("off", "auto_reseat = all", "auto_reseat = off"),
        ("mainon", "auto_reseat = all", "auto_reseat = on"),
        (
            "scope",
            "auto_reseat = all",
            "auto_reseat = all\nauto_reseat_sessions = elsewhere",
        ),
    ] {
        let session = format!("restart{case}");
        let rig = fixture(&session, "", "destination");
        let text = std::fs::read_to_string(&rig.config).expect("config");
        std::fs::write(&rig.config, text.replace(from, to)).expect("explicit disabled policy");
        launch(&rig, &session);
        retained(&rig, &session);
        quota(&rig, "source", "session", 100, 0, None);
        stop(&rig, &session);
        launch(&rig, &session);
        let _daemon = watch(&rig, &session);
        sweeps(&rig, &session);
        assert_eq!(profile(&rig, &session), "source", "{case}");
        assert!(booked(&rig, &session, ATTEMPT_ACTION).is_empty(), "{case}");
    }
}

#[test]
fn exhausted_restart_without_a_usable_destination_stays_and_does_not_loop() {
    let session = "restartnoroom";
    let rig = fixture(session, "", "unavailable");
    // Keep the family universe to the unavailable candidate, so derivation cannot rescue it.
    let text = std::fs::read_to_string(&rig.config).expect("config");
    let text: String = text
        .lines()
        .filter(|line| !line.starts_with("destination = "))
        .fold(String::new(), |mut out, line| {
            out.push_str(line);
            out.push('\n');
            out
        });
    std::fs::write(&rig.config, text).expect("closed candidate universe");
    launch(&rig, session);
    retained(&rig, session);
    quota(&rig, "source", "session", 100, 0, None);
    stop(&rig, session);
    launch(&rig, session);
    let daemon = watch(&rig, session);
    until(&rig, session, || {
        !booked(&rig, session, REFUSED_ACTION).is_empty()
    });
    sweeps(&rig, session);
    drop(daemon);
    let _again = watch(&rig, session);
    sweeps(&rig, session);
    assert_eq!(profile(&rig, session), "source");
    assert_eq!(
        booked(&rig, session, REFUSED_ACTION).len(),
        1,
        "one refusal, existing cooldown"
    );
    assert!(booked(&rig, session, DONE_ACTION).is_empty());
    assert!(booked(&rig, session, FAILED_ACTION).is_empty());
}

#[test]
fn stopped_restart_moves_long_exhaustion_and_matching_model_scope_without_grace() {
    for (case, kind, model, knobs) in [
        ("long", "weekly_all", None, ""),
        (
            "matchedmodel",
            "weekly_scoped",
            Some("Opus"),
            "model-test\n",
        ),
    ] {
        let session = format!("restart{case}");
        let rig = fixture(&session, knobs, "destination");
        launch(&rig, &session);
        retained(&rig, &session);
        quota(&rig, "source", kind, 100, 0, model);
        stop(&rig, &session);
        launch(&rig, &session);
        let _daemon = watch(&rig, &session);
        until(&rig, &session, || {
            booked(&rig, &session, DONE_ACTION).len() == 1
        });
        assert_eq!(profile(&rig, &session), "destination");
        assert!(
            !events(&rig, &session)
                .iter()
                .any(|event| event.action == "limit")
        );
    }
}

#[test]
fn near_headroom_after_restart_keeps_existing_grace() {
    let session = "restartnear";
    let rig = fixture(session, "", "destination");
    let text = std::fs::read_to_string(&rig.config).expect("config");
    std::fs::write(
        &rig.config,
        text.replace("auto_reseat_at = off", "auto_reseat_at = 95"),
    )
    .expect("ordinary headroom enabled");
    launch(&rig, session);
    retained(&rig, session);
    quota(&rig, "source", "weekly_all", 96, 0, None);
    stop(&rig, session);
    launch(&rig, session);
    let _daemon = watch(&rig, session);
    until(&rig, session, || {
        !booked(&rig, session, ae::autoreseat::HEADROOM_ACTION).is_empty()
    });
    sweeps(&rig, session);
    assert_eq!(profile(&rig, session), "source");
    assert!(
        booked(&rig, session, ATTEMPT_ACTION).is_empty(),
        "nonexhausted headroom keeps its grace"
    );
}

#[test]
fn quota_arriving_after_the_first_watchdog_read_is_reobserved_before_normal_cadence() {
    let session = "restartlatefeed";
    let rig = fixture(session, "", "destination");
    let config = std::fs::read_to_string(&rig.config).expect("config");
    std::fs::write(
        &rig.config,
        config.replace("quota_every_secs = 1", "quota_every_secs = 3600"),
    )
    .expect("real meta cadence, which overrides the daemon CLI default");
    quota(&rig, "source", "session", 10, 2, None);
    launch(&rig, session);
    retained(&rig, session);
    stop(&rig, session);
    launch(&rig, session);
    let _daemon = watch_every(&rig, session, "3600");
    until(&rig, session, || {
        std::fs::read_to_string(rig.scratch.join("restart-trace"))
            .unwrap_or_default()
            .lines()
            .any(|line| line.starts_with("observed "))
    });
    assert_eq!(
        profile(&rig, session),
        "source",
        "first reading was healthy"
    );
    let before = events(&rig, session).len();
    quota(&rig, "source", "session", 100, 0, None);
    until(&rig, session, || {
        booked(&rig, session, DONE_ACTION).len() == 1
    });
    assert_eq!(
        profile(&rig, session),
        "destination",
        "one startup follow-up must not wait 3600 seconds"
    );
    let after = events(&rig, session);
    assert!(
        !after.iter().skip(before).any(|event| {
            matches!(event.action.as_str(), "quota-checkpoint" | "quota-advisory")
                && ae::watchdog::event_is_addressed_to(event, session, "main", "lead")
        }),
        "an urgent dead seat gets neither checkpoint ask nor advisory turn: {after:?}"
    );
}

#[test]
fn startup_follow_up_does_not_change_disabled_or_excluded_seats_quota_cadence() {
    for (case, from, to) in [
        ("off", "auto_reseat = all", "auto_reseat = off"),
        ("mainon", "auto_reseat = all", "auto_reseat = on"),
        (
            "scope",
            "auto_reseat = all",
            "auto_reseat = all\nauto_reseat_sessions = elsewhere",
        ),
    ] {
        let session = format!("late{case}");
        let rig = fixture(&session, "", "destination");
        let config = std::fs::read_to_string(&rig.config).expect("config");
        std::fs::write(
            &rig.config,
            config
                .replace("quota_every_secs = 1", "quota_every_secs = 3600")
                .replace(from, to),
        )
        .expect("disabled or excluded real startup policy");
        quota(&rig, "source", "session", 10, 2, None);
        launch(&rig, &session);
        retained(&rig, &session);
        stop(&rig, &session);
        launch(&rig, &session);
        let _daemon = watch_every(&rig, &session, "3600");
        let observations = || {
            std::fs::read_to_string(rig.scratch.join("restart-trace"))
                .unwrap_or_default()
                .lines()
                .filter(|line| line.starts_with("observed "))
                .count()
        };
        until(&rig, &session, || observations() == 1);
        quota(&rig, "source", "session", 100, 0, None);
        // Enabled startup's second read is due after five seconds. Across ten
        // one-second cycles these modes must retain their 3600-second cadence.
        let through = Instant::now() + Duration::from_secs(10);
        while Instant::now() < through {
            assert_eq!(observations(), 1, "{case}: no added quota pass");
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(profile(&rig, &session), "source");
        assert!(booked(&rig, &session, ATTEMPT_ACTION).is_empty());
        assert!(booked(&rig, &session, "quota-checkpoint").is_empty());
        assert!(booked(&rig, &session, "quota-advisory").is_empty());
    }
}

fn replace_row(rig: &Rig, session: &str, key: &str, value: Option<&str>) {
    let path = rig.dir(session).join("meta");
    let text = rig.meta(session);
    let mut kept: String = text
        .lines()
        .filter(|line| !line.starts_with(&format!("{key}=")))
        .fold(String::new(), |mut out, line| {
            out.push_str(line);
            out.push('\n');
            out
        });
    if let Some(value) = value {
        writeln!(kept, "{key}={value}").expect("string write");
    }
    std::fs::write(path, kept).expect("owned settlement fixture");
}

#[test]
fn unfinished_launch_has_no_urgent_move_and_completion_enables_it_without_losing_start_identity() {
    let session = "restartsettled";
    let rig = fixture(session, "", "destination");
    launch(&rig, session);
    assert!(
        row(&rig.meta(session), "launch_settled").is_empty(),
        "create is outside restart urgency"
    );
    retained(&rig, session);
    stop(&rig, session);
    launch(&rig, session);
    let resumed = rig.meta(session);
    let settled = row(&resumed, "launch_settled");
    let started = row(&resumed, "started");
    assert!(!settled.is_empty(), "resume publishes phase-2 completion");
    replace_row(&rig, session, "launch_settled", None);
    quota(&rig, "source", "session", 100, 0, None);
    let daemon = watch(&rig, session);
    sweeps(&rig, session);
    assert_eq!(profile(&rig, session), "source");
    assert!(
        booked(&rig, session, ATTEMPT_ACTION).is_empty(),
        "missing completion cannot race a pending first turn"
    );
    replace_row(&rig, session, "launch_settled", Some(&settled));
    until(&rig, session, || {
        booked(&rig, session, DONE_ACTION).len() == 1
    });
    drop(daemon);
    let _again = watch(&rig, session);
    sweeps(&rig, session);
    assert_eq!(
        row(&rig.meta(session), "started"),
        started,
        "reseat keeps full-session start identity"
    );
    assert_eq!(
        row(&rig.meta(session), "launch_settled"),
        settled,
        "reseat cannot create a fresh startup opportunity"
    );
}

fn action(rig: &Rig, session: &str, action: &str) -> std::process::Output {
    let pane = row(&rig.meta(session), "main_pane");
    ae().args([
        &format!("@{session}"),
        "send",
        "lead",
        "ignored watchdog placeholder",
    ])
    .env("HOME", &rig.scratch)
    .env("AE_HOME", &rig.home)
    .env("CONFIG_FILE", &rig.config)
    .env("TMUX", format!("{},0,0", rig.sock.display()))
    .env("TMUX_PANE", pane)
    .env("AE_SENDER_OVERRIDE", "watchdog")
    .env("_AE_EVENT_ACTION", action)
    .output()
    .expect("real guarded action entry")
}

#[test]
fn current_incarnation_armed_brief_blocks_urgent_trigger_then_delivers_once_before_move() {
    let session = "restartbrief";
    let rig = fixture(session, "", "destination");
    launch(&rig, session);
    retained(&rig, session);
    stop(&rig, session);
    launch(&rig, session);
    let meta = rig.meta(session);
    let record = ae::brief_retry::Record {
        slot: "main".to_owned(),
        reference: "spawn-main".to_owned(),
        pane: row(&meta, "main_pane"),
        launch_id: row(&meta, "launch_id.main"),
        actor: "lead".to_owned(),
        attempts: 0,
        created: Timestamp::now().epoch(),
        phase: ae::brief_retry::Phase::Armed,
        body: "startreseat unique pending brief body".to_owned(),
    };
    ae::brief_retry::publish(&rig.dir(session), &record).expect("real armed record grammar");
    let record_path = ae::brief_retry::path(&rig.dir(session), "main");
    let armed = std::fs::read(&record_path).expect("armed bytes");
    quota(&rig, "source", "session", 100, 0, None);
    let ts = Timestamp::now();
    let log = rig.dir(session).join("events.jsonl");
    let mut text = std::fs::read_to_string(&log).unwrap_or_default();
    writeln!(text, "{{\"ts\":\"{ts}\",\"actor\":\"watchdog\",\"action\":\"auto-reseat-headroom\",\"target\":\"lead\",\"target_slot\":\"main\",\"target_session\":\"{session}\",\"summary\":\"headroom 100% session 5h\"}}").expect("string write");
    std::fs::write(log, text).expect("real headroom opener");
    let result = action(&rig, session, ATTEMPT_ACTION);
    assert!(
        !result.status.success(),
        "armed brief must refuse the urgent trigger: {result:?}"
    );
    assert_eq!(profile(&rig, session), "source");
    assert_eq!(std::fs::read(&record_path).expect("record retained"), armed);
    assert!(booked(&rig, session, ATTEMPT_ACTION).is_empty());
    let delivered = action(&rig, session, "brief-retry");
    assert!(delivered.status.success(), "{delivered:?}");
    assert!(!record_path.exists(), "submitted brief retires its record");
    let _daemon = watch(&rig, session);
    until(&rig, session, || {
        booked(&rig, session, DONE_ACTION).len() == 1
    });
    sweeps(&rig, session);
    let received =
        std::fs::read_to_string(rig.scratch.join("received")).expect("tool received bytes");
    assert_eq!(
        received.matches(&record.body).count(),
        1,
        "pending brief is never lost or delivered twice"
    );
}
fn runparsed_lastword_command(
    rig: &Rig,
    session: &str,
    staged_record: &std::path::Path,
    live_record: &std::path::Path,
) -> (super::super::cli::Runner, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let mut command = ae();
    // Reuse the existing pass-through trace wrapper to resolve the isolated
    // runner PATH and real tmux; replace only its owned fixture script.
    let trace = super::launchx_trace_command(&mut command, &rig.scratch);
    let wrapper = rig.scratch.join("trace-bin/tmux");
    std::fs::write(
        &wrapper,
        r#"#!/usr/bin/perl
use strict;
use warnings;
use Fcntl qw(:flock);
if (-e $ENV{AE_LASTWORD_STAGED} && grep { $_ eq $ENV{AE_LASTWORD_PROBE} } @ARGV) {
    if (open(my $lock, '+<', $ENV{AE_LASTWORD_LOCK})) {
        if (!flock($lock, LOCK_EX | LOCK_NB)) {
            rename($ENV{AE_LASTWORD_STAGED}, $ENV{AE_LASTWORD_LIVE}) or die "publish armed brief: $!";
            open(my $seen, '>', $ENV{AE_LASTWORD_SEEN}) or die "barrier receipt: $!";
            print $seen "armed under lifecycle\n";
            close($seen) or die "close receipt: $!";
        }
        close($lock);
    }
}
exec {$ENV{AE_LAUNCHX_REAL_TMUX}} $ENV{AE_LAUNCHX_REAL_TMUX}, @ARGV;
die "exec private tmux: $!";
"#,
    )
    .expect("owned pass-through barrier");
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
        .expect("owned executable barrier");
    let seen = trace.with_extension("lastword");
    command
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .env("AE_LASTWORD_STAGED", staged_record)
        .env("AE_LASTWORD_LIVE", live_record)
        .env("AE_LASTWORD_SEEN", &seen)
        .env("AE_LASTWORD_PROBE", ae::tmux::PANE_PROBE_FORMAT)
        .env(
            "AE_LASTWORD_LOCK",
            rig.home
                .join("sessions")
                .join(format!(".lifecycle.{session}.lock")),
        );
    (command, seen)
}

fn runparsed_stage_lastword_brief(
    rig: &Rig,
    session: &str,
    before: &str,
    pane: &str,
    ts: Timestamp,
) -> (PathBuf, PathBuf, Vec<u8>) {
    let dir = rig.dir(session);
    let staged = rig.scratch.join("staged-last-word");
    std::fs::create_dir_all(&staged).expect("owned staging directory");
    let record = ae::brief_retry::Record {
        slot: "main".to_owned(),
        reference: "spawn-main".to_owned(),
        pane: pane.to_owned(),
        launch_id: row(before, "launch_id.main"),
        actor: "lead".to_owned(),
        attempts: 0,
        created: ts.epoch(),
        phase: ae::brief_retry::Phase::Armed,
        body: "last-word armed brief".to_owned(),
    };
    ae::brief_retry::publish(&staged, &record).expect("real brief grammar");
    let staged_record = ae::brief_retry::path(&staged, "main");
    let armed_bytes = std::fs::read(&staged_record).expect("staged brief");
    let live_record = ae::brief_retry::path(&dir, "main");
    assert!(!live_record.exists(), "admission initially remains urgent");

    (staged_record, live_record, armed_bytes)
}

#[test]
fn runparsed_dead_seat_rechecks_urgency_under_lifecycle_before_any_move_write() {
    // Contract: docs/internals/reseat.md startup urgency requires the last
    // start_read under the move's locks. An armed brief withdraws urgency,
    // including when there is no running tool to stop.
    let session = "restartlastword";
    let rig = fixture(session, "", "destination");
    launch(&rig, session);
    let (_, relative) = retained(&rig, session);
    quota(&rig, "source", "session", 100, 0, None);
    stop(&rig, session);
    launch(&rig, session);
    let before = rig.meta(session);
    assert!(
        !row(&before, "launch_settled").is_empty(),
        "real resumed launch"
    );
    let pane = row(&before, "main_pane");
    assert!(
        rig.tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            &pane,
            "-c",
            &rig.project.display().to_string(),
            "exec /bin/sh",
        ])
        .0,
        "owned seat returns to its shell"
    );
    until(&rig, session, || {
        let (ok, command) = rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{pane_current_command}",
        ]);
        ok && matches!(command.trim(), "sh" | "bash" | "dash" | "zsh" | "fish")
    });

    let dir = rig.dir(session);
    let marker = dir.join("launch.main.started");
    let marker_before = std::fs::read(&marker).expect("real launch marker");
    let prompt = dir.join("launch.main.prompt");
    std::fs::write(&prompt, b"last-word brief must survive").expect("owned old brief");
    let ts = Timestamp::now();
    let log = dir.join("events.jsonl");
    let mut journal = std::fs::read_to_string(&log).unwrap_or_default();
    for (action, reference) in [
        ("auto-reseat-headroom", String::new()),
        (ATTEMPT_ACTION, format!(",\"ref\":\"{ts}\"")),
    ] {
        writeln!(journal, "{{\"ts\":\"{ts}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"lead\",\"target_slot\":\"main\",\"target_session\":\"{session}\"{reference},\"summary\":\"headroom 100% session 5h\"}}").expect("real episode records");
    }
    std::fs::write(&log, &journal).expect("open urgent attempt");

    let (staged_record, live_record, armed_bytes) =
        runparsed_stage_lastword_brief(&rig, session, &before, &pane, ts);

    let (mut command, seen) =
        runparsed_lastword_command(&rig, session, &staged_record, &live_record);
    let result = command
        .arg("_auto-reseat")
        .arg(&dir)
        .arg("main")
        .arg(ts.to_string())
        .output()
        .expect("real auto-reseat leg");

    assert!(seen.is_file(), "causal barrier was not reached: {result:?}");
    assert_eq!(
        std::fs::read_to_string(&seen).expect("causal barrier reached"),
        "armed under lifecycle\n"
    );
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert_eq!(rig.meta(session), before, "declined move writes no meta");
    assert_eq!(
        std::fs::read(&marker).expect("marker retained"),
        marker_before
    );
    assert_eq!(
        std::fs::read(&prompt).expect("old brief retained"),
        b"last-word brief must survive"
    );
    assert_eq!(
        std::fs::read(live_record).expect("armed brief retained"),
        armed_bytes
    );
    assert!(!dir.join("seed.lead.md").exists(), "no seed publication");
    assert!(
        !rig.scratch.join("destination").join(relative).exists(),
        "no conversation copy"
    );
    assert!(booked(&rig, session, "reseat").is_empty(), "no move audit");
    assert!(
        booked(&rig, session, DONE_ACTION).is_empty(),
        "no completion"
    );
    let refused = booked(&rig, session, REFUSED_ACTION);
    assert_eq!(refused.len(), 1, "one refusal: {result:?}");
    assert!(
        refused[0]
            .summary
            .as_deref()
            .is_some_and(|text| text.starts_with("other:")),
        "hook decline retains Other: {refused:?}"
    );
}
