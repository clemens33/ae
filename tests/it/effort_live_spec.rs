//! #253 acceptance: lead R1–R6, B3, profile-target ruling, restart plan r2.1.
//! Real private tmux + real watchdog/_run/leg; only vendor TUI and quota are fake.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own scratch files, clocks and private tmux servers"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ae::config::{AutoEffort, auto_effort_in, global_auto_effort};
use ae::effort::{
    Direction, Episode, Fold, Foreign, Leg, Level, Record, Resume, Stored, Switch, Transition,
    on_resume,
};
use ae::events::{AlertMeaning, Event};
use ae::time::Timestamp;

use super::seat_relaunch::Rig;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const CONV: &str = "01a1592e-c600-7222-8222-222222222222";
const EP: i64 = 1_700_000_000;
const VALID: &str = "v1 applied slot:main base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:tok-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:custom client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700000100";
const WAIT: Duration = Duration::from_secs(90);

fn transition(up: bool) -> Transition {
    Transition {
        direction: if up { Direction::Up } else { Direction::Down },
        from: if up { Level::High } else { Level::Xhigh },
        to: if up { Level::Xhigh } else { Level::High },
    }
}

fn authorized(up: bool) -> Episode {
    Episode {
        key: EP,
        threshold: 80,
        down: if up {
            Leg::Done(transition(false))
        } else {
            Leg::Reserved(transition(false))
        },
        up: if up {
            Leg::Reserved(transition(true))
        } else {
            Leg::Unspent
        },
        yielded: false,
        named: vec![],
    }
}

fn restarting(up: bool) -> Record {
    let literal = VALID.replace(
        "v1 applied",
        if up {
            "v1 restart-up"
        } else {
            "v1 restart-down"
        },
    );
    Record::parse(&literal).expect("R1 new authorized restart grammar must parse")
}

#[test]
fn absent_is_off_but_only_explicit_strict_global_on_enables() {
    for text in ["", "[workspace]\n", "[profiles]\nauto_effort = on\n"] {
        assert_eq!(
            auto_effort_in(Path::new("global"), text),
            AutoEffort::Off,
            "{text:?}"
        );
    }
    assert_eq!(global_auto_effort(None), AutoEffort::Off);
    assert_eq!(
        auto_effort_in(Path::new("global"), "[workspace]\nauto_effort = on\n"),
        AutoEffort::On
    );
    for text in [
        "[workspace]\nauto_effort = ON\n",
        "[workspace]\nauto_effort = on\nauto_effort = on\n",
    ] {
        assert!(matches!(
            auto_effort_in(Path::new("global"), text),
            AutoEffort::Unusable(_)
        ));
    }
}

#[test]
fn restart_replay_requires_this_episode_threshold_direction_and_unyielded_authority() {
    for up in [false, true] {
        let row = restarting(up);
        let stored = Stored::Present(row.clone());
        assert_eq!(Record::parse(&row.encode()).expect("round trip"), row);
        let good = authorized(up);
        assert_eq!(
            on_resume(
                &stored,
                Ok(()),
                Some(&Fold::Open(good.clone())),
                Switch::On,
                true
            ),
            Resume::Replay(transition(up).to)
        );
        let mut cases = vec![good.clone(); 7];
        cases[0].key += 1;
        cases[1].threshold = 81;
        cases[2].yielded = true;
        cases[3].down = Leg::Unspent;
        cases[4].up = Leg::Done(transition(true));
        if up {
            cases[5].down = Leg::Reserved(transition(false));
            cases[6].up = Leg::Reserved(transition(false));
        } else {
            cases[5].down = Leg::Done(transition(false));
            cases[6].up = Leg::Reserved(transition(true));
        }
        for bad in cases {
            assert_eq!(
                on_resume(
                    &stored,
                    Ok(()),
                    Some(&Fold::Open(bad.clone())),
                    Switch::On,
                    true
                ),
                Resume::Refuse(row.state),
                "up={up}, {bad:?}"
            );
        }
        for ledger in [None, Some(Fold::Armed), Some(Fold::Doubt("unread"))] {
            assert_eq!(
                on_resume(&stored, Ok(()), ledger.as_ref(), Switch::On, true),
                Resume::Refuse(row.state)
            );
        }
        for foreign in [Foreign::Profile, Foreign::Model, Foreign::Baseline] {
            assert!(!matches!(
                on_resume(
                    &stored,
                    Err(foreign),
                    Some(&Fold::Open(good.clone())),
                    Switch::On,
                    true
                ),
                Resume::Replay(_)
            ));
        }
        let old = Record::parse(
            &VALID.replace("applied", if up { "pending-up" } else { "pending-down" }),
        )
        .expect("legacy pending");
        assert_eq!(
            on_resume(
                &Stored::Present(old.clone()),
                Ok(()),
                Some(&Fold::Open(good)),
                Switch::On,
                true
            ),
            Resume::Refuse(old.state)
        );
    }
}

#[test]
fn restart_down_fresh_fallback_is_failed_so_pressure_cannot_restart_forever() {
    let row = restarting(false);
    let result = on_resume(
        &Stored::Present(row),
        Ok(()),
        Some(&Fold::Open(authorized(false))),
        Switch::On,
        false,
    );
    assert!(
        format!("{result:?}").starts_with("Failed"),
        "fresh down must spend failure, not reset the episode: {result:?}"
    );
}

#[test]
fn effort_outcomes_never_break_quiet_or_become_an_alert() {
    for action in [
        "effort-episode",
        "effort-reserved",
        "effort-applied",
        "effort-restored",
        "effort-failed",
        "effort-held",
        "effort-yielded",
    ] {
        let event = Event::parse_line(&format!(r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"{action}","target":"lead","target_slot":"main","target_session":"fixture","ref":"1700000000-down","summary":"xhigh->high"}}"#)).expect("literal audit event");
        assert_eq!(event.alert_meaning(), AlertMeaning::Undefined);
        for state in ["done", "waiting-agent", "waiting-user", "blocked"] {
            assert!(
                !ae::watchdog::ends_quiet(state, &event, false, &[]),
                "{action} ended {state}"
            );
        }
    }
}

#[test]
fn activity_shows_applied_restored_and_failed_moves_without_reseat_events() {
    let meta = ae::session_menu::MetaSource::Parsed {
        uuid: SID.to_owned(),
        actors: vec!["lead".to_owned()],
        mode: "local".to_owned(),
        origin: String::new(),
        work_dir: String::new(),
        branch: String::new(),
        meta: Box::new(ae::meta::Meta::parse(&format!(
            "session_id={SID}\nseat.main=lead\n"
        ))),
    };
    let bytes = ["effort-applied", "effort-restored", "effort-failed"].map(|action| format!(r#"{{"ts":"2023-11-14T22:13:20Z","actor":"watchdog","action":"{action}","target":"lead","summary":"xhigh->high"}}
"#)).concat().into_bytes();
    let rows = ae::session_menu::activity_cells(
        &ae::tmux::OptionReading::Set(SID.to_owned()),
        &meta,
        &ae::store::SourceRead::Ready(bytes),
        Timestamp::from_epoch(EP + 10),
    );
    let kinds: Vec<_> = rows
        .iter()
        .filter_map(|row| match row {
            ae::session_menu::CellRow::Cells(cells) => Some(cells[2].as_str()),
            ae::session_menu::CellRow::Gap(_) => None,
        })
        .collect();
    assert_eq!(
        kinds,
        ["effort-failed", "effort-restored", "effort-applied"]
    );
}

#[test]
fn list_renders_restart_direction_and_keeps_effort_display_out_of_json() {
    let mut session = ae::digest::SessionEntry::new("fixture", ae::digest::Status::Running);
    let mut agent = ae::digest::AgentEntry {
        reference: "fixture:lead".to_owned(),
        alias: "fixture".to_owned(),
        name: "lead".to_owned(),
        session_id: None,
        alive: Some(true),
        observed: ae::harness_state::HarnessState::Idle,
        state: None,
        done_progress: None,
        wait_progress: None,
        reason: None,
        own_work: None,
        model_drift: ae::model_drift::ModelDrift::Quiet,
        auto_reseat_open: false,
        unreachable: false,
        declaration: None,
        effort: None,
        ..Default::default()
    };
    let all = ae::filters::ListArgs::parse(&["--all"]).expect("list flags");
    let json = ae::filters::ListArgs::parse(&["--all", "--json"]).expect("JSON flags");
    for (up, expected) in [
        (false, "effort:restarting xhigh->high"),
        (true, "effort:restarting high->xhigh"),
    ] {
        agent.effort = Some(Stored::Present(restarting(up)));
        session.agents = vec![agent.clone()];
        let world = ae::listing::World::new(Timestamp::from_epoch(EP), vec![session.clone()]);
        assert!(ae::listing::render(&all, &world).contains(expected));
        let mut clean = world.clone();
        clean.sessions[0].agents[0].effort = None;
        assert_eq!(
            ae::listing::render(&json, &world),
            ae::listing::render(&json, &clean)
        );
    }
}

// Independent vendor protocol fixture. Draws measured bounded composers and
// reads ONLY the real launch argv for effort. __FALLBACK__ imitates a vendor
// registration callback; no actuator, quota fold or safe-point decision is mocked.
const VENDOR: &str = r#"
use strict; use warnings; use utf8; use Encode qw(encode_utf8); use POSIX qw(setsid);
my $root = '__ROOT__'; my $tool = '__TOOL__';
my $effort = 'xhigh';
my $model = $tool eq 'claude' ? 'Opus 5.5' : 'gpt-6.1-sol';
$model = $tool eq 'claude' ? 'Sonnet 5.5' : 'gpt-6-astra' if -e "$root/__WRONG_MODEL__";
for (my $i=0; $i<@ARGV; $i++) {
  $effort=$ARGV[$i+1] if $ARGV[$i] eq '--effort';
  $effort=$1 if $ARGV[$i] =~ /model_reasoning_effort=(\w+)/;
}
$effort='xhigh' if -e "$root/__WRONG_EFFORT__";
open(my $log,'>>',"$root/launched") or die; my $argv=join(' ',@ARGV); $argv =~ s/\n/ /g; print $log $argv,"\n"; close($log);
if (-e "$root/__FALLBACK__") {
  my $meta='__META__'; open(my $in,'<',$meta) or die; local $/; my $s=<$in>; close($in);
  $s =~ s/^harness_session\.main=.*$/harness_session.main=33333333-3333-4333-8333-333333333333/m;
  $s .= "harness_session_prior.main=01a1592e-c600-7222-8222-222222222222\n";
  open(my $out,'>',"$meta.fake") or die; print $out $s; close($out); rename("$meta.fake",$meta) or die;
}
system('stty raw -echo 2>/dev/null'); binmode(STDIN,':raw'); binmode(STDOUT,':raw'); $|=1;
my $kid=0;
sub cleanup { kill 'TERM',-$kid if $kid; system('stty sane 2>/dev/null'); exit 0; }
$SIG{HUP}=\&cleanup; $SIG{TERM}=\&cleanup; $SIG{INT}=\&cleanup;
sub state {
  my $live=$effort;
  if (open(my $fh,'<',"$root/live-effort")) { $live=<$fh>; chomp($live); close($fh); }
  return join(':',map {-e "$root/$_" ? 1:0} qw(__BUSY__ __LIMIT__ __DRAFT__ __BACKGROUND__)).":$live";
}
sub draw {
  my ($busy,$limit,$draft,$background,$live)=split(':',shift);
  if ($background && !$kid) { $kid=fork(); if (!$kid) { setsid(); exec '/bin/sh','-c','sleep 120; :'; exit 1; } }
  if (!$background && $kid) { kill 'TERM',-$kid; waitpid($kid,0); $kid=0; }
  my $bar='─' x 60; my $out="\e[H\e[2J";
  if ($tool eq 'claude') {
    $out .= $busy ? "✳ Thinking… (3s · esc to interrupt)\r\n" : "✻ Fake · done (0s)\r\n";
    $out .= "⎿  You've hit your session limit · resets 3pm\r\n" if $limit;
    $out .= "$bar\r\n❯ ".($draft ? 'half a sentence':'')."\r\n$bar\r\n";
    $out .= "  🧠 $model ($live)  📁 work\r\n  ⏵⏵ bypass permissions on\r\n";
  } else {
    $out .= $busy ? "• Working (3s · esc to interrupt)\r\n" : "• ok\r\n";
    $out .= "You've hit your usage limit\r\n" if $limit;
    $out .= "\r\n\e[1m›\e[0m ".($draft ? 'half a sentence':"\e[2mAsk Codex to do anything\e[0m")."\r\n\r\n  $model $live · /tmp/work\r\n";
  }
  print encode_utf8($out);
  open(my $drawn,'>',"$root/drawn.$$.tmp") or die;
  print $drawn $busy ? 'esc to interrupt' : $draft ? 'half a sentence' : $limit ? 'hit your session limit' : $tool eq 'claude' ? 'done (0s)' : 'Ask Codex to do anything';
  close($drawn); rename("$root/drawn.$$.tmp","$root/drawn.$$") or die;
}
my $shown=state(); draw($shown);
while (1) {
  cleanup() if -e "$root/__EXIT__";
  my $now=state(); if ($shown ne $now) { draw($now); $shown=$now; }
  my $ready=''; vec($ready,fileno(STDIN),1)=1;
  if (select($ready,undef,undef,0.05)>0) {
    my $chunk=''; sysread(STDIN,$chunk,4096);
    open(my $fh,'>>',"$root/received") or die; print $fh $chunk; close($fh);
  }
}
"#;

struct Seat {
    rig: Rig,
    account: PathBuf,
    tool: &'static str,
}

fn put_row(rig: &Rig, key: &str, value: &str) {
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
    std::fs::write(rig.dir.join("meta"), text).expect("fixture meta row");
}

fn setup(tag: &str, tool: &'static str, baseline: &str, knobs: &str) -> Seat {
    let rig = Rig::new(tag);
    let root = std::fs::canonicalize(&rig.scratch).expect("physical scratch");
    let account = root.join("account");
    let home = root.join("home");
    std::fs::create_dir_all(&account).expect("account");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::write(
        account.join("settings.json"),
        "{\"sentinel\":\"untouched\"}\n",
    )
    .expect("vendor settings sentinel");
    let script = root.join(format!("{tool}.pl"));
    std::fs::write(
        &script,
        VENDOR
            .replace("__ROOT__", &root.display().to_string())
            .replace("__TOOL__", tool)
            .replace("__META__", &rig.dir.join("meta").display().to_string()),
    )
    .expect("independent vendor fake");
    let low = if baseline == "max" { "xhigh" } else { "high" };
    let command = |effort: &str| {
        if tool == "claude" {
            format!(
                "CLAUDE_CONFIG_DIR={} {} {} --model opus --effort {effort}",
                account.display(),
                root.join("tools/claude").display(),
                script.display()
            )
        } else {
            format!(
                "CODEX_HOME={} {} {} --yolo -m gpt-6.1-sol -c model_reasoning_effort={effort}",
                account.display(),
                root.join("tools/codex").display(),
                script.display()
            )
        }
    };
    let text = format!(
        "[profiles]\nfake-effort-base = \"{}\"\nfake-effort-low = \"{}\"\n[workspace]\nmain = lead\nlayout = vertical\nauto_reseat = off\nauto_upgrade = off\nquota = on\n{knobs}\n",
        command(baseline),
        command(low)
    );
    std::fs::write(root.join("config"), &text).expect("scratch profile pair");
    ae::config::parse_identity(&text).expect("fixture config must be valid");
    for (key, value) in [
        ("profile.main", "fake-effort-base"),
        ("agent_bin.main", tool),
        ("session_id", SID),
        ("harness_session.main", CONV),
        ("launch_id.main", "effort-fixture"),
        ("work_dir", root.join("work").to_str().expect("work path")),
    ] {
        put_row(&rig, key, value);
    }
    plant_transcript(&root, &account, tool);
    std::fs::write(ae::run::started_marker(&rig.dir, "main"), "").expect("retained launch marker");
    assert!(
        rig.tmux(&[
            "respawn-pane",
            "-k",
            "-c",
            root.join("work").to_str().expect("work path"),
            "-t",
            &rig.main_pane,
            "env",
            "-u",
            "CLAUDE_CONFIG_DIR",
            "-u",
            "CODEX_HOME",
            &format!("HOME={}", home.display()),
            &format!("AE_HOME={}", root.display()),
            &format!("CONFIG_FILE={}", root.join("config").display()),
            "/bin/sh"
        ])
        .0
    );
    rig.start(&rig.main_pane, "main", tool);
    assert_eq!(
        rig.meta_row("harness_session.main"),
        CONV,
        "fixture must actually retain exact resume"
    );
    let frame = rig.capture(&rig.main_pane);
    let kind = if tool == "claude" {
        ae::tool::ToolKind::Claude
    } else {
        ae::tool::ToolKind::Codex
    };
    assert_eq!(
        ae::harness_state::classify(&frame, kind),
        ae::harness_state::HarnessState::Idle,
        "fixture premise: {frame}"
    );
    Seat { rig, account, tool }
}

fn plant_transcript(root: &Path, account: &Path, tool: &str) {
    let transcript = if tool == "claude" {
        account
            .join("projects")
            .join(ae::carry::project_key(&root.join("work")))
            .join(format!("{CONV}.jsonl"))
    } else {
        account
            .join("sessions/2026/10/20")
            .join(format!("rollout-{CONV}.jsonl"))
    };
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("transcript store");
    std::fs::write(transcript, "{\"type\":\"user\"}\n").expect("exact-resume source");
}

fn quota(seat: &Seat, weekly: f64, short: f64) {
    let now = Timestamp::now().epoch();
    if seat.tool == "claude" {
        std::fs::write(seat.account.join(".claude.json"), format!(r#"{{"cachedUsageUtilization":{{"fetchedAtMs":{},"utilization":{{"limits":[{{"kind":"weekly_all","group":"weekly_all","percent":{weekly},"resets_at":"{}","scope":null}},{{"kind":"session","group":"session","percent":{short},"resets_at":"{}","scope":null}}]}}}}}}"#, now * 1_000, Timestamp::from_epoch(now + 86_400), Timestamp::from_epoch(now + 3_600))).expect("Claude own quota");
    } else {
        let path = seat
            .account
            .join(format!("sessions/2026/10/20/rollout-{CONV}.jsonl"));
        let json = r#"{"timestamp":"$TS","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","plan_type":"pro","primary":{"used_percent":$SHORT,"window_minutes":300,"resets_at":$SHORTRESET},"secondary":{"used_percent":$WEEKLY,"window_minutes":10080,"resets_at":$LONGRESET}}}}
"#.replace("$TS", &Timestamp::from_epoch(now).to_string())
            .replace("$SHORTRESET", &(now + 3_600).to_string())
            .replace("$LONGRESET", &(now + 86_400).to_string())
            .replace("$SHORT", &short.to_string()).replace("$WEEKLY", &weekly.to_string());
        ae::json::parse(json.trim()).expect("quota fixture JSON");
        std::fs::write(path, format!("{{\"type\":\"user\"}}\n{json}")).expect("Codex own quota");
    }
}

fn records(rig: &Rig, action: &str) -> Vec<Event> {
    rig.events()
        .lines()
        .map(|line| Event::parse_line(line).expect("valid audit journal"))
        .filter(|event| event.action == action)
        .collect()
}

fn watch(seat: &Seat) -> super::cli::OwnedChild {
    let rig = &seat.rig;
    let trace = rig.scratch.join("effort-quota-trace");
    std::fs::write(&trace, "").expect("new observation barrier");
    let send = rig.dir.join("send");
    if !send.exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), send).expect("private send helper");
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rig.scratch.join("effort-daemon-err"))
        .expect("daemon log");
    super::cli::ae()
        .arg("_watchdog-run")
        .arg(&rig.dir)
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
        .env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("AE_TEST_QUOTA_TRACE", trace)
        .env("AE_SEND_DEFER_SEC", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .spawn()
        .expect("real watchdog")
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
        "effort contract timed out\n{}\n{}\n{}",
        rig.events(),
        rig.capture(&rig.main_pane),
        std::fs::read_to_string(rig.scratch.join("effort-daemon-err")).unwrap_or_default()
    );
}

fn sweeps(rig: &Rig) {
    let observed = || {
        std::fs::read_to_string(rig.scratch.join("effort-quota-trace"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("observed "))
            .count()
    };
    let before = observed();
    until(rig, || observed() >= before + 2);
}

fn unchanged(seat: &Seat, pid: Option<u32>) {
    assert_eq!(
        seat.rig.tool_pid(&seat.rig.main_pane, seat.tool),
        pid,
        "held seat must keep running"
    );
    assert!(
        records(&seat.rig, "effort-reserved").is_empty(),
        "hold cannot spend an attempt: {}",
        seat.rig.events()
    );
    assert!(records(&seat.rig, "effort-applied").is_empty());
    assert!(!seat.rig.meta().contains("\neffort.main="));
}

#[test]
fn fixture_vendor_composers_retain_the_conversation_for_both_tools() {
    for (tag, tool) in [("effpremiseclaude", "claude"), ("effpremisecodex", "codex")] {
        let seat = setup(tag, tool, "xhigh", "auto_effort = on");
        assert!(
            seat.rig.launched().contains(CONV),
            "exact resume reaches real argv"
        );
    }
}

#[test]
fn watchdog_down_and_recovery_keep_pane_profile_account_model_and_conversation() {
    for (tag, tool, baseline, target) in [
        ("effliveclaude", "claude", "xhigh", "high"),
        ("efflivecodex", "codex", "max", "xhigh"),
    ] {
        let seat = setup(tag, tool, baseline, "auto_effort = on");
        let rig = &seat.rig;
        quota(&seat, 79.9, 99.0);
        let original = rig.tool_pid(&rig.main_pane, tool);
        let initial_account = rig.meta_row("config_home.main");
        let daemon = watch(&seat);
        sweeps(rig);
        unchanged(&seat, original);
        quota(&seat, 80.0, 20.0);
        until(rig, || records(rig, "effort-applied").len() == 1);
        let down_pid = rig.tool_pid(&rig.main_pane, tool);
        assert_ne!(down_pid, original, "actual restart required");
        assert_eq!(rig.meta_row("harness_session.main"), CONV);
        assert_eq!(rig.meta_row("profile.main"), "fake-effort-base");
        assert_eq!(rig.meta_row("config_home.main"), initial_account);
        assert!(rig.meta_row("harness_session_prior.main").is_empty());
        let row = ae::effort::stored(rig.meta().as_bytes(), "main");
        assert!(
            matches!(row, Stored::Present(ref record) if record.state == ae::effort::State::Applied && record.low.word() == target && record.base.word() == baseline)
        );
        let launches = rig.launched();
        assert_eq!(
            launches.lines().count(),
            2,
            "exactly one down restart: {launches}"
        );
        assert!(
            launches
                .lines()
                .last()
                .expect("restarted argv")
                .contains(CONV)
        );
        assert!(
            launches
                .lines()
                .last()
                .expect("restarted argv")
                .contains(if tool == "claude" {
                    "--model opus --effort high"
                } else {
                    "model_reasoning_effort=xhigh"
                })
        );
        let audit = rig.events();
        assert!(
            audit.find("effort-reserved").expect("reservation")
                < audit.find("effort-applied").expect("receipt")
        );
        assert!(records(rig, "reseat").is_empty() && records(rig, "relaunch").is_empty());
        for event in records(rig, "effort-applied") {
            assert_eq!(event.actor, "watchdog");
        }
        until(rig, || rig.received().contains("⟦ae:ctx⟧"));
        let notice = rig.received();
        assert!(
            notice.contains("profile") && notice.contains("ae reseat"),
            "profile target + how to change target: {notice}"
        );
        assert_eq!(notice.matches("⟦ae:ctx⟧").count(), 1);
        drop(daemon);
        let _restarted_daemon = watch(&seat);
        sweeps(rig);
        assert_eq!(
            rig.tool_pid(&rig.main_pane, tool),
            down_pid,
            "durable down reservation across daemon restart"
        );
        assert_eq!(records(rig, "effort-applied").len(), 1);
        quota(&seat, 75.0, 0.0);
        sweeps(rig);
        assert!(
            records(rig, "effort-restored").is_empty(),
            "75 is not under75"
        );
        quota(&seat, 74.9, 0.0);
        until(rig, || records(rig, "effort-restored").len() == 1);
        assert!(!rig.meta().contains("\neffort.main="));
        assert_eq!(rig.meta_row("harness_session.main"), CONV);
        assert_eq!(rig.meta_row("profile.main"), "fake-effort-base");
        assert_eq!(rig.launched().lines().count(), 3, "one down + one up only");
        assert_eq!(
            std::fs::read_to_string(seat.account.join("settings.json")).expect("vendor settings"),
            "{\"sentinel\":\"untouched\"}\n"
        );
    }
}

#[test]
fn manual_change_after_move_yields_instead_of_restoring_over_the_human() {
    let seat = setup("effmanual", "claude", "xhigh", "auto_effort = on");
    quota(&seat, 95.0, 0.0);
    let rig = &seat.rig;
    let _daemon = watch(&seat);
    until(rig, || records(rig, "effort-applied").len() == 1);
    let pid = rig.tool_pid(&rig.main_pane, seat.tool);
    std::fs::write(rig.scratch.join("live-effort"), "max\n").expect("manual live effort");
    until(rig, || !records(rig, "effort-yielded").is_empty());
    quota(&seat, 0.0, 0.0);
    sweeps(rig);
    assert_eq!(rig.tool_pid(&rig.main_pane, seat.tool), pid);
    assert!(records(rig, "effort-restored").is_empty());
    assert!(!rig.meta().contains("\neffort.main="));
    assert_eq!(rig.meta_row("profile.main"), "fake-effort-base");
}

#[test]
fn absent_off_busy_draft_and_drawn_limit_never_reserve_or_stop() {
    for (tag, knobs, marker) in [
        ("effabsent", "", None),
        ("effoff", "auto_effort = off", None),
        ("effbusy", "auto_effort = on", Some("__BUSY__")),
        ("effdraft", "auto_effort = on", Some("__DRAFT__")),
        ("efflimit", "auto_effort = on", Some("__LIMIT__")),
    ] {
        let seat = setup(tag, "claude", "xhigh", knobs);
        let rig = &seat.rig;
        if let Some(marker) = marker {
            std::fs::write(rig.scratch.join(marker), "").expect("transient safe point hold");
            until(rig, || {
                rig.capture(&rig.main_pane).contains(match marker {
                    "__BUSY__" => "esc to interrupt",
                    "__DRAFT__" => "half a sentence",
                    _ => "hit your session limit",
                })
            });
        }
        quota(&seat, 99.0, 99.0);
        let pid = rig.tool_pid(&rig.main_pane, seat.tool);
        let _daemon = watch(&seat);
        sweeps(rig);
        unchanged(&seat, pid);
    }
}

#[test]
fn background_shell_holds_once_without_spending_then_idle_can_restart() {
    let seat = setup("effbackground", "claude", "xhigh", "auto_effort = on");
    let rig = &seat.rig;
    std::fs::write(rig.scratch.join("__BACKGROUND__"), "").expect("fake vendor background job");
    quota(&seat, 95.0, 0.0);
    let pid = rig.tool_pid(&rig.main_pane, seat.tool);
    let _daemon = watch(&seat);
    until(rig, || {
        records(rig, "effort-held")
            .iter()
            .any(|event| event.summary.as_deref() == Some("background-task"))
    });
    sweeps(rig);
    unchanged(&seat, pid);
    assert_eq!(
        records(rig, "effort-held")
            .iter()
            .filter(|event| event.summary.as_deref() == Some("background-task"))
            .count(),
        1
    );
    std::fs::remove_file(rig.scratch.join("__BACKGROUND__")).expect("background finishes");
    until(rig, || records(rig, "effort-applied").len() == 1);
}

#[test]
fn fallback_new_conversation_fails_clears_and_never_claims_applied_or_retries() {
    let seat = setup("efffallback", "claude", "xhigh", "auto_effort = on");
    let rig = &seat.rig;
    std::fs::write(rig.scratch.join("__FALLBACK__"), "").expect("vendor fallback on restart");
    quota(&seat, 95.0, 0.0);
    let _daemon = watch(&seat);
    until(rig, || !records(rig, "effort-failed").is_empty());
    sweeps(rig);
    assert!(records(rig, "effort-applied").is_empty());
    assert!(!rig.meta().contains("\neffort.main="));
    assert_eq!(records(rig, "effort-reserved").len(), 1);
    assert_eq!(
        rig.launched().lines().count(),
        2,
        "failed transition is spent for this episode"
    );
}

#[test]
fn a_new_pid_with_same_conversation_but_source_effort_is_failed_not_applied() {
    let seat = setup("effwrongtarget", "claude", "xhigh", "auto_effort = on");
    let rig = &seat.rig;
    let original = rig.tool_pid(&rig.main_pane, seat.tool);
    std::fs::write(rig.scratch.join("__WRONG_EFFORT__"), "")
        .expect("vendor ignores target on next launch");
    quota(&seat, 95.0, 0.0);
    let _daemon = watch(&seat);
    until(rig, || !records(rig, "effort-failed").is_empty());
    assert_ne!(
        rig.tool_pid(&rig.main_pane, seat.tool),
        original,
        "premise: a new process really started"
    );
    assert_eq!(rig.meta_row("harness_session.main"), CONV);
    assert!(
        rig.launched()
            .lines()
            .last()
            .expect("restart argv")
            .contains("--effort high")
    );
    assert!(
        records(rig, "effort-applied").is_empty(),
        "PID + exact id alone cannot be a receipt"
    );
    assert!(!rig.meta().contains("\neffort.main="));
    sweeps(rig);
    assert_eq!(records(rig, "effort-reserved").len(), 1);
    assert_eq!(rig.launched().lines().count(), 2);
}

fn remap_main(seat: &Seat, slot: &str, agent: &str) {
    let rig = &seat.rig;
    let meta = rig.meta().replace(".main=", &format!(".{slot}="));
    std::fs::write(rig.dir.join("meta"), meta).expect("fixture roster remap");
    put_row(rig, &format!("seat.{slot}"), agent);
    std::fs::rename(
        ae::run::started_marker(&rig.dir, "main"),
        ae::run::started_marker(&rig.dir, slot),
    )
    .expect("retained remapped launch");
    for (key, value) in [("@ae_slot", slot), ("@ae_agent", agent)] {
        assert!(
            rig.tmux(&["set-option", "-p", "-t", &rig.main_pane, key, value])
                .0
        );
    }
}

#[test]
fn colead_is_eligible_but_spawned_and_orchestrator_seats_are_excluded() {
    for (tag, slot, agent, orchestrator) in [
        ("effcolead", "worker.0", "colead", false),
        ("effspawned", "spawned.0", "scout", false),
        ("efforchestrator", "main", "lead", true),
    ] {
        let seat = setup(tag, "claude", "xhigh", "auto_effort = on");
        let rig = &seat.rig;
        if slot != "main" {
            remap_main(&seat, slot, agent);
        }
        if orchestrator {
            put_row(rig, "meta_agent", "true");
        }
        quota(&seat, 95.0, 0.0);
        let pid = rig.tool_pid(&rig.main_pane, seat.tool);
        let _daemon = watch(&seat);
        if slot == "worker.0" {
            until(rig, || {
                records(rig, "effort-applied").iter().any(|event| {
                    ae::watchdog::event_is_addressed_to(event, &rig.session, "worker.0", "colead")
                })
            });
            assert_eq!(rig.meta_row("harness_session.worker.0"), CONV);
        } else {
            sweeps(rig);
            unchanged(&seat, pid);
            assert!(records(rig, "effort-episode").is_empty());
        }
    }
}

#[test]
fn missing_twin_is_named_once_and_headroom_inversion_disables_new_attempts() {
    for (tag, inversion) in [("effnotwin", false), ("effinversion", true)] {
        let seat = setup(tag, "claude", "xhigh", "auto_effort = on");
        let rig = &seat.rig;
        let config = rig.scratch.join("config");
        let old = std::fs::read_to_string(&config).expect("config");
        let text = if inversion {
            old.replace("auto_reseat = off", "auto_reseat = on\nauto_reseat_at = 80")
        } else {
            old.lines()
                .filter(|line| !line.starts_with("fake-effort-low ="))
                .fold(String::new(), |mut out, line| {
                    writeln!(out, "{line}").expect("string write");
                    out
                })
        };
        std::fs::write(config, &text).expect("ineligible attempt configuration");
        ae::config::parse_identity(&text).expect("valid ineligible fixture");
        quota(&seat, 95.0, 0.0);
        let pid = rig.tool_pid(&rig.main_pane, seat.tool);
        let _daemon = watch(&seat);
        sweeps(rig);
        unchanged(&seat, pid);
        if inversion {
            let log = format!(
                "{}\n{}",
                rig.events(),
                std::fs::read_to_string(rig.scratch.join("effort-daemon-err")).unwrap_or_default()
            );
            let notes = log
                .lines()
                .filter(|line| line.contains("effort") && line.contains("auto_reseat_at"))
                .count();
            assert_eq!(notes, 1, "inversion note once per daemon: {log}");
        } else {
            assert_eq!(
                records(rig, "effort-held")
                    .iter()
                    .filter(|event| event.summary.as_deref() == Some("no-twin"))
                    .count(),
                1,
                "named once: {}",
                rig.events()
            );
        }
    }
}

#[test]
fn effective_manual_reset_credit_owns_pressure_and_an_unanswered_inbound_holds() {
    for (tag, credit) in [("effcredit", true), ("effinbound", false)] {
        let seat = setup(tag, "claude", "xhigh", "auto_effort = on");
        let rig = &seat.rig;
        if credit {
            let path = rig.scratch.join("config");
            let old = std::fs::read_to_string(&path).expect("config");
            let text = format!(
                "[clients]\ncredited = claude config_home={} manual_resets=1\n{old}",
                seat.account.display()
            );
            ae::config::parse_identity(&text).expect("credit config");
            std::fs::write(path, text).expect("manual reset credit");
        } else {
            let ts = Timestamp::now();
            std::fs::write(rig.dir.join("events.jsonl"), format!(r#"{{"ts":"{ts}","actor":"colead","actor_slot":"worker.0","actor_session":"{}","action":"ask","target":"lead","target_slot":"main","target_session":"{}","target_session_id":"{SID}","ref":"inbound-1","summary":"answer owed"}}
"#, rig.session, rig.session)).expect("unanswered inbound request");
        }
        quota(&seat, 95.0, 0.0);
        let pid = rig.tool_pid(&rig.main_pane, seat.tool);
        let _daemon = watch(&seat);
        sweeps(rig);
        unchanged(&seat, pid);
    }
}

#[test]
fn crash_after_reservation_before_respawn_fails_without_a_second_stop() {
    for (tag, has_record) in [("effcrashreserved", false), ("effcrashmeta", true)] {
        let seat = setup(tag, "claude", "xhigh", "auto_effort = on");
        let rig = &seat.rig;
        quota(&seat, 95.0, 0.0);
        let pid = rig
            .tool_pid(&rig.main_pane, seat.tool)
            .expect("original live tool");
        let now = Timestamp::now();
        let ep = now.epoch();
        let trace = [("effort-episode", ep.to_string(), "threshold 80"), ("effort-reserved", format!("{ep}-down"), "xhigh->high")].map(|(action, reference, summary)| format!(r#"{{"ts":"{now}","actor":"watchdog","action":"{action}","target":"lead","target_slot":"main","target_session":"{}","target_session_id":"{SID}","ref":"{reference}","summary":"{summary}"}}
"#, rig.session)).concat();
        std::fs::write(rig.dir.join("events.jsonl"), trace).expect("durable pre-crash reservation");
        if has_record {
            let row = format!(
                "v1 restart-down slot:main base:xhigh low:high at:80 ep:{ep} sid:{SID} agent:lead tool:claude launch:effort-fixture pid:{pid} conv:{CONV} profile:fake-effort-base client:- model:opus win:weekly_all/~/10080/{ep}/{}",
                ep + 86_400
            );
            put_row(rig, "effort.main", &row);
        }
        let output = super::cli::ae()
            .arg("_auto-effort")
            .arg(&rig.dir)
            .args(["main", &ep.to_string()])
            .env("HOME", rig.scratch.join("home"))
            .env("AE_HOME", &rig.scratch)
            .env("CONFIG_FILE", rig.scratch.join("config"))
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .output()
            .expect("real reconciliation leg");
        assert!(
            output.status.success(),
            "reconciliation entry: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            records(rig, "effort-failed").len(),
            1,
            "reserved pre-respawn crash must close failed: {}",
            rig.events()
        );
        assert_eq!(
            rig.tool_pid(&rig.main_pane, seat.tool),
            Some(pid),
            "reconcile never restarts"
        );
        assert!(records(rig, "effort-applied").is_empty());
        assert!(!rig.meta().contains("\neffort.main="));
        assert_eq!(rig.launched().lines().count(), 1);
    }
}

#[test]
fn another_accounts_hot_weekly_window_cannot_move_the_seat() {
    let seat = setup("effownscope", "claude", "xhigh", "auto_effort = on");
    let rig = &seat.rig;
    quota(&seat, 20.0, 99.0);
    let other = rig.scratch.join("other-account");
    std::fs::create_dir(&other).expect("unrelated account");
    let own_json =
        std::fs::read_to_string(seat.account.join(".claude.json")).expect("own quota fixture");
    std::fs::write(
        other.join(".claude.json"),
        own_json.replace("\"percent\":20", "\"percent\":95"),
    )
    .expect("other hot weekly");
    let path = rig.scratch.join("config");
    let old = std::fs::read_to_string(&path).expect("config");
    let text = format!(
        "[clients]\nother = claude config_home={}\n{old}",
        other.display()
    );
    ae::config::parse_identity(&text).expect("other account config");
    std::fs::write(path, text).expect("two-account config");
    let pid = rig.tool_pid(&rig.main_pane, seat.tool);
    let _daemon = watch(&seat);
    sweeps(rig);
    unchanged(&seat, pid);
}

// Addendum r3: receipt identity, stale detached work and honest crash wording.
// Check stale facts after every invocation, then prove the same episode can reserve.
const DIRECT_LEG_TRIES: usize = 4;
const DIRECT_LEG_GAP: Duration = Duration::from_secs(1);
// Unknown process snapshots intentionally hold a leg until a later cycle. Give
// the positive control the normal fixture deadline, rather than four snapshots.
const DIRECT_LEG_WAIT: Duration = WAIT;

fn still_held(seat: &Seat, ep: i64, original: Option<u32>) {
    for attempt in 0..DIRECT_LEG_TRIES {
        invoke_effort(seat, ep);
        unchanged(seat, original);
        if attempt + 1 < DIRECT_LEG_TRIES {
            std::thread::sleep(DIRECT_LEG_GAP);
        }
    }
}

fn reserve_same_episode(seat: &Seat, ep: i64) {
    let rig = &seat.rig;
    let mut diagnostics = String::new();
    let deadline = Instant::now() + DIRECT_LEG_WAIT;
    let mut attempt = 0;
    while Instant::now() < deadline {
        attempt += 1;
        let output = invoke_effort(seat, ep);
        writeln!(
            diagnostics,
            "attempt {attempt}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .expect("string write");
        if !records(rig, "effort-reserved").is_empty() {
            break;
        }
        std::thread::sleep(DIRECT_LEG_GAP.min(deadline.saturating_duration_since(Instant::now())));
    }
    assert_eq!(
        records(rig, "effort-reserved").len(),
        1,
        "positive control: the same episode must reserve with eligible facts within {DIRECT_LEG_WAIT:?}\n{diagnostics}\n{}",
        rig.events()
    );
}

fn invoke_effort(seat: &Seat, ep: i64) -> std::process::Output {
    let rig = &seat.rig;
    let output = super::cli::ae()
        .arg("_auto-effort")
        .arg(&rig.dir)
        .args(["main", &ep.to_string()])
        .env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("AE_SEND_DEFER_SEC", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .output()
        .expect("real detached effort leg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn open_episode(seat: &Seat, up: bool) -> i64 {
    let rig = &seat.rig;
    let now = Timestamp::now();
    let ep = now.epoch();
    let mut trace = format!(
        r#"{{"ts":"{now}","actor":"watchdog","action":"effort-episode","target":"lead","target_slot":"main","target_session":"{}","target_session_id":"{SID}","ref":"{ep}","summary":"threshold 80"}}
"#,
        rig.session
    );
    if up {
        for (action, dir, pair) in [
            ("effort-reserved", "down", "xhigh->high"),
            ("effort-applied", "down", "xhigh->high"),
            ("effort-reserved", "up", "high->xhigh"),
        ] {
            writeln!(trace, r#"{{"ts":"{now}","actor":"watchdog","action":"{action}","target":"lead","target_slot":"main","target_session":"{}","target_session_id":"{SID}","ref":"{ep}-{dir}","summary":"{pair}"}}"#, rig.session).expect("audit fixture");
        }
    }
    std::fs::write(rig.dir.join("events.jsonl"), trace).expect("durable episode fixture");
    ep
}

fn wrong_model(tool: &'static str) {
    let seat = setup("effwrongmodel", tool, "xhigh", "auto_effort = on");
    let rig = &seat.rig;
    let original = rig.tool_pid(&rig.main_pane, tool);
    std::fs::write(rig.scratch.join("__WRONG_MODEL__"), "")
        .expect("next launch changes model only");
    quota(&seat, 95.0, 0.0);
    let _daemon = watch(&seat);
    until(rig, || {
        !records(rig, "effort-applied").is_empty() || !records(rig, "effort-failed").is_empty()
    });
    assert_ne!(
        rig.tool_pid(&rig.main_pane, tool),
        original,
        "premise: tool restarted"
    );
    assert_eq!(rig.meta_row("harness_session.main"), CONV);
    let frame = rig.capture(&rig.main_pane);
    assert!(
        frame.contains(if tool == "claude" {
            "Sonnet 5.5"
        } else {
            "gpt-6-astra"
        }),
        "wrong-model premise: {frame}"
    );
    assert!(
        records(rig, "effort-applied").is_empty(),
        "R1: exact conversation + new PID + target effort do not prove unchanged model"
    );
    assert!(records(rig, "effort-restored").is_empty());
    assert!(
        !rig.meta().contains("\neffort.main="),
        "failed down cannot retain replay ownership"
    );
    assert_eq!(records(rig, "effort-failed").len(), 1);
}

#[test]
fn r3_claude_wrong_model_cannot_earn_an_applied_receipt() {
    wrong_model("claude");
}

#[test]
fn r3_codex_wrong_model_cannot_earn_an_applied_receipt() {
    wrong_model("codex");
}

#[test]
fn r3_a_detached_leg_rechecks_orchestrator_role_before_reserving() {
    let seat = setup("effstalerole", "claude", "xhigh", "auto_effort = on");
    let rig = &seat.rig;
    quota(&seat, 95.0, 0.0);
    let ep = open_episode(&seat, false);
    put_row(rig, "meta_agent", "true");
    // Let startup input age beyond the safe-point grace before judging the stale fact.
    std::thread::sleep(Duration::from_secs(5));
    let original = rig.tool_pid(&rig.main_pane, seat.tool);
    still_held(&seat, ep, original);
    let text = rig
        .meta()
        .lines()
        .filter(|line| !line.starts_with("meta_agent="))
        .fold(String::new(), |mut out, line| {
            writeln!(out, "{line}").expect("string write");
            out
        });
    std::fs::write(rig.dir.join("meta"), text).expect("remove orchestrator role");
    reserve_same_episode(&seat, ep);
}

#[test]
fn r3_a_detached_leg_rechecks_zero_or_unreadable_quota_cadence() {
    for cadence in ["0", "soon"] {
        let seat = setup("effstalecadence", "claude", "xhigh", "auto_effort = on");
        let rig = &seat.rig;
        quota(&seat, 95.0, 0.0);
        let ep = open_episode(&seat, false);
        put_row(rig, "quota_every_secs", cadence);
        // Let startup input age beyond the safe-point grace before judging the stale fact.
        std::thread::sleep(Duration::from_secs(5));
        let original = rig.tool_pid(&rig.main_pane, seat.tool);
        still_held(&seat, ep, original);
        put_row(rig, "quota_every_secs", "1");
        reserve_same_episode(&seat, ep);
    }
}

#[test]
fn r3_up_with_record_already_cleared_is_unproven_never_failed_or_restored() {
    let seat = setup("effupunproven", "claude", "xhigh", "auto_effort = on");
    let rig = &seat.rig;
    quota(&seat, 10.0, 0.0);
    let ep = open_episode(&seat, true);
    let original = rig.tool_pid(&rig.main_pane, seat.tool);
    invoke_effort(&seat, ep);
    let failed = records(rig, "effort-failed");
    assert_eq!(failed.len(), 1, "conservative spent kind retained");
    let summary = failed[0]
        .summary
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        summary.contains("unproven"),
        "lost identity/PID receipt must be named in the record: {summary}"
    );
    assert!(!summary.contains("restart failed") && !summary.contains("restored"));
    until(rig, || rig.received().contains("⟦ae:ctx⟧"));
    let notice = rig.received().to_ascii_lowercase();
    assert!(
        notice.contains("unproven") && notice.contains("profile"),
        "honest recovery notice: {notice}"
    );
    assert!(!notice.contains("restart failed") && !notice.contains("ae restored"));
    assert!(records(rig, "effort-restored").is_empty());
    assert_eq!(
        rig.tool_pid(&rig.main_pane, seat.tool),
        original,
        "reconciliation never stops the seat"
    );
    assert_eq!(rig.launched().lines().count(), 1);
    assert!(!rig.meta().contains("\neffort.main="));
}
