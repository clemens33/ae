//! Independent acceptance for lead-rulings-r1: per-seat proved model + effort.
//! Real _run/watchdog/stop/restore; only the external vendor TUI is simulated.
#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures own scratch files, private servers and clocks"
)]

use std::fmt::Write as _;
use std::time::{Duration, Instant, SystemTime};

use ae::json::Value;
use ae::tool::ToolKind;

use super::cli::{OwnedChild, Runner, ae};
use super::seat_relaunch::Rig;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const CONV: &str = "01a1592e-c600-7222-8222-222222222222";
const LAUNCH: &str = "77777777-7777-4777-8777-777777777777";
const WAIT: Duration = Duration::from_mins(1);

// The fake reads argv, draws measured current-frame syntax, and accepts a
// fixture-only replacement pair. It never decides whether ae should preserve it.
const VENDOR: &str = r#"use strict;
use warnings;
use utf8;
use Encode qw(encode_utf8);
my $root='__ROOT__';
my $tool='__TOOL__';
my ($model,$effort)=($tool eq 'claude' ? 'fable' : 'gpt-6-astra','xhigh');
for (my $i=0;$i<@ARGV;$i++) {
  $model=$ARGV[$i+1] if $ARGV[$i] eq '--model' || $ARGV[$i] eq '-m';
  $effort=$ARGV[$i+1] if $ARGV[$i] eq '--effort';
  $effort=$1 if $ARGV[$i] =~ /^model_reasoning_effort=(\w+)$/;
}
my %labels=('fable'=>'Fable 5.1','claude-opus-5-5'=>'Opus 5.5',
  'claude-sonnet-5-5'=>'Sonnet 5.5','claude-haiku-5-5'=>'Haiku 5.5');
$model=$labels{$model} if exists $labels{$model};
open(my $argv,'>',"$root/argv.$$.tmp") or die;
print $argv join("\036",@ARGV); close($argv);
rename("$root/argv.$$.tmp","$root/last-argv") or die;
open(my $log,'>>',"$root/launch-pids") or die; print $log "$$\n"; close($log);
system('stty raw -echo 2>/dev/null'); binmode(STDIN,':raw'); binmode(STDOUT,':raw'); $|=1;
sub state {
  my ($m,$e)=($model,$effort);
  for my $pair (['manual-model',\$m],['manual-effort',\$e]) {
    if (open(my $f,'<',"$root/$pair->[0]")) { ${$pair->[1]}=<$f>; chomp(${$pair->[1]}); close($f); }
  }
  return "$m|$e";
}
sub draw {
  my ($m,$e)=split(/\|/,shift,2);
  my $out="\e[H\e[2J";
  if ($tool eq 'claude') {
    my $bar='─' x 60;
    $out.="✻ Fake · done (0s)\r\n$bar\r\n❯ \r\n$bar\r\n  🧠 $m ($e)  📁 work\r\n  ⏵⏵ bypass permissions on\r\n";
  } else {
    $out.="• ok\r\n\r\n› Ask Codex to do anything\r\n\r\n  $m $e · /tmp/work\r\n";
  }
  print encode_utf8($out);
  open(my $drawn,'>',"$root/drawn.$$.tmp") or die; print $drawn $tool eq 'claude' ? 'done (0s)' : 'Ask Codex to do anything'; close($drawn);
  rename("$root/drawn.$$.tmp","$root/drawn.$$") or die;
}
my $shown=state(); draw($shown);
while (1) {
  my $now=state(); if ($now ne $shown) { draw($now); $shown=$now; }
  my $ready=''; vec($ready,fileno(STDIN),1)=1;
  if (select($ready,undef,undef,0.05)>0) {
    my $chunk=''; sysread(STDIN,$chunk,4096);
    if (index($chunk,"\003")>=0) { system('stty sane 2>/dev/null'); exit 0; }
  }
}
"#;

struct Seat {
    rig: Rig,
    tool: &'static str,
}

#[allow(
    clippy::expect_used,
    reason = "test fixtures fail at the unmet setup premise"
)]
impl Seat {
    fn new(tag: &str, tool: &'static str) -> Self {
        let mut rig = Rig::new(tag);
        let root = std::fs::canonicalize(&rig.scratch).expect("physical scratch");
        rig.scratch.clone_from(&root);
        for name in ["home", "account"] {
            std::fs::create_dir_all(root.join(name)).expect("private home");
        }
        std::fs::write(
            root.join(format!("{tool}.pl")),
            VENDOR
                .replace("__ROOT__", root.to_str().expect("fixture path"))
                .replace("__TOOL__", tool),
        )
        .expect("external vendor fake");
        let seat = Self { rig, tool };
        seat.config(&[("base", seat.command(seat.base_model(), "xhigh"))], false);
        for (key, value) in [
            ("profile.main", "base"),
            ("agent_bin.main", tool),
            ("session_id", SID),
            ("harness_session.main", CONV),
            ("launch_id.main", LAUNCH),
            ("watchdog", "on"),
        ] {
            seat.row(key, value);
        }
        seat.row(
            "work_dir",
            root.join("work").to_str().expect("physical work"),
        );
        seat.row(
            "config_home.main",
            root.join("account").to_str().expect("retained account"),
        );
        std::fs::write(ae::run::started_marker(&seat.rig.dir, "main"), "")
            .expect("retained conversation marker");
        let account = root.join("account");
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
        std::fs::create_dir_all(transcript.parent().expect("store parent"))
            .expect("retained store");
        std::fs::write(transcript, "{\"type\":\"user\"}\n").expect("conversation exists");
        seat
    }

    fn base_model(&self) -> &'static str {
        if self.tool == "claude" {
            "fable"
        } else {
            "gpt-6-astra"
        }
    }

    fn kind(&self) -> ToolKind {
        if self.tool == "claude" {
            ToolKind::Claude
        } else {
            ToolKind::Codex
        }
    }

    fn command(&self, model: &str, effort: &str) -> String {
        let root = &self.rig.scratch;
        let prefix = if self.tool == "claude" {
            "CLAUDE_CONFIG_DIR"
        } else {
            "CODEX_HOME"
        };
        let flags = if self.tool == "claude" {
            format!("--permission-mode bypassPermissions --model {model} --effort {effort}")
        } else {
            format!("--yolo -m {model} -c model_reasoning_effort={effort}")
        };
        format!(
            "{prefix}={} {} {} {flags}",
            root.join("account").display(),
            root.join("tools").join(self.tool).display(),
            root.join(format!("{}.pl", self.tool)).display()
        )
    }

    fn config(&self, profiles: &[(&str, String)], auto: bool) {
        let mut text = String::from("[profiles]\n");
        for (name, command) in profiles {
            writeln!(text, "{name} = \"{command}\"").expect("string write");
        }
        writeln!(text, "[roster]\nlead = base\n[workspace]\nmain = lead\nlayout = vertical\nchat = off\nquota = {}\nauto_effort = {}\nauto_reseat = off\nauto_upgrade = off\nauto_trust = off\nrestore = on\nidle_nudge_secs = 0", if auto { "on" } else { "off" }, if auto { "on" } else { "off" }).expect("string write");
        ae::config::parse_identity(&text).expect("acceptance config parses");
        std::fs::write(self.rig.scratch.join("config"), text).expect("private config");
    }

    fn row(&self, key: &str, value: &str) {
        let mut text = self
            .rig
            .meta()
            .lines()
            .filter(|line| line.split('=').next() != Some(key))
            .fold(String::new(), |mut out, line| {
                writeln!(out, "{line}").expect("string write");
                out
            });
        writeln!(text, "{key}={value}").expect("string write");
        std::fs::write(self.rig.dir.join("meta"), text).expect("fixture row");
    }

    fn observe_rows(&self, model: &str, effort: Option<&str>) {
        self.row("observed_model.main", model);
        self.row("observed_model_pin.main", self.base_model());
        if let Some(effort) = effort {
            self.row("observed_effort.main", effort);
            self.row("observed_effort_pin.main", "xhigh");
        }
    }

    fn cli(&self, args: &[&str]) -> Runner {
        let mut command = ae();
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env("HOME", self.rig.scratch.join("home"))
            .env("AE_HOME", &self.rig.scratch)
            .env("CONFIG_FILE", self.rig.scratch.join("config"))
            .env("TMUX_TMPDIR", &self.rig.scratch)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.rig.sock)
            .env("AE_NO_AUTOSTART", "1")
            .current_dir(self.rig.scratch.join("work"))
            .args(args);
        command
    }

    fn plan(&self) -> (Vec<String>, String) {
        let out = self
            .cli(&["_run", "--print"])
            .arg(&self.rig.dir)
            .arg("main")
            .output()
            .expect("real _run plan");
        assert!(
            out.status.success(),
            "plan: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let Value::Obj(mut fields) =
            ae::json::parse(String::from_utf8_lossy(&out.stdout).trim()).expect("plan JSON")
        else {
            panic!("plan object")
        };
        let Value::Arr(words) = fields
            .swap_remove(
                fields
                    .iter()
                    .position(|(key, _)| key == "argv")
                    .expect("argv field"),
            )
            .1
        else {
            panic!("argv array")
        };
        (
            words
                .into_iter()
                .map(|word| {
                    if let Value::Str(text) = word {
                        text
                    } else {
                        panic!("argv string")
                    }
                })
                .collect(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn start(&self) {
        self.start_drawn();
        self.until(|| {
            ae::harness_state::current_identity(&self.rig.capture(&self.rig.main_pane), self.kind())
                .model
                .is_some()
        });
    }

    // A malformed flag may draw an unsupported frame; starting it must not
    // require the product reader to prove that frame's model.
    fn start_drawn(&self) {
        let root = &self.rig.scratch;
        let before = self.pids().len();
        assert!(
            self.rig
                .tmux(&[
                    "respawn-pane",
                    "-k",
                    "-c",
                    root.join("work").to_str().expect("work"),
                    "-t",
                    &self.rig.main_pane,
                    "env",
                    "-u",
                    "CLAUDE_CONFIG_DIR",
                    "-u",
                    "CODEX_HOME",
                    &format!("HOME={}", root.join("home").display()),
                    &format!("AE_HOME={}", root.display()),
                    &format!("CONFIG_FILE={}", root.join("config").display()),
                    "/bin/sh",
                ])
                .0,
            "private tool start"
        );
        self.rig.start(&self.rig.main_pane, "main", self.tool);
        self.until(|| self.pids().len() > before);
        let pid = self.pids().pop().expect("started tool pid");
        self.until(|| root.join(format!("drawn.{pid}")).exists());
    }

    fn pids(&self) -> Vec<String> {
        std::fs::read_to_string(self.rig.scratch.join("launch-pids"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn argv(&self) -> Vec<String> {
        std::fs::read_to_string(self.rig.scratch.join("last-argv"))
            .expect("executed argv")
            .split('\u{1e}')
            .map(str::to_owned)
            .collect()
    }

    fn manual(&self, model: &str, effort: &str) {
        for (name, value) in [("manual-model", model), ("manual-effort", effort)] {
            std::fs::write(self.rig.scratch.join(name), value).expect("external TUI choice");
        }
        self.until(|| {
            let identity = ae::harness_state::current_identity(
                &self.rig.capture(&self.rig.main_pane),
                self.kind(),
            );
            // An unknown model must still be drawable for the fail-closed cell.
            let frame = self.rig.capture(&self.rig.main_pane);
            frame.contains(model) && identity.effort.as_deref() == Some(effort)
        });
    }

    fn until(&self, condition: impl Fn() -> bool) {
        let end = Instant::now() + WAIT;
        while Instant::now() < end {
            if condition() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "acceptance timed out\nmeta={}\nevents={}\nframe={}\ndaemon={}",
            self.rig.meta(),
            self.rig.events(),
            self.rig.capture(&self.rig.main_pane),
            std::fs::read_to_string(self.rig.scratch.join("daemon-err")).unwrap_or_default()
        );
    }

    fn watch(&self) -> OwnedChild {
        let send = self.rig.dir.join("send");
        if !send.exists() {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), send).expect("private helper");
        }
        let log = std::fs::File::create(self.rig.scratch.join("daemon-err")).expect("watchdog log");
        self.cli(&["_watchdog-run"])
            .arg(&self.rig.dir)
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
            .stdout(std::process::Stdio::null())
            .stderr(log)
            .env("AE_SEND_DEFER_SEC", "1")
            .spawn()
            .expect("real watchdog")
    }

    fn cycles(&self, count: usize) {
        // Beat is written before the observer. One extra beat proves all the
        // requested preceding sweeps completed, without a wall-clock sleep oracle.
        let beat = || {
            std::fs::metadata(self.rig.dir.join(".watchdog-beat"))
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        };
        for _ in 0..=count {
            let before = beat();
            self.until(|| beat() > before);
        }
    }

    fn listing(&self, json: bool) -> String {
        let args = if json {
            &["list", "--json"][..]
        } else {
            &["list"][..]
        };
        let out = self.cli(args).output().expect("real list");
        assert!(
            out.status.success(),
            "list: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn diagnoses_damage(&self) {
        let doctor = self.cli(&["doctor"]).output().expect("doctor");
        let diagnosed = format!(
            "{}{}",
            String::from_utf8_lossy(&doctor.stdout),
            String::from_utf8_lossy(&doctor.stderr)
        );
        for surface in [self.listing(false), diagnosed] {
            let surface = surface.to_lowercase();
            assert!(
                surface.lines().any(|line| {
                    (line.contains("main") || line.contains("lead"))
                        && (line.contains("observ")
                            || line.contains("manual")
                            || line.contains("launch"))
                        && [
                            "invalid",
                            "damaged",
                            "duplicate",
                            "incomplete",
                            "unusable",
                            "partial",
                        ]
                        .iter()
                        .any(|word| line.contains(word))
                }),
                "damage diagnostic must name this seat and record on the same line: {surface}"
            );
        }
    }

    fn pair(&self, words: &[String], model: &str, effort: &str) {
        let short = &words[..words.len().min(12)];
        let flag = if self.tool == "claude" {
            "--model"
        } else {
            "-m"
        };
        assert!(
            words.windows(2).any(|w| w == [flag, model]),
            "model {model}: {short:?}"
        );
        if self.tool == "claude" {
            assert!(
                words.windows(2).any(|w| w == ["--effort", effort]),
                "effort {effort}: {short:?}"
            );
        } else {
            assert!(
                words
                    .iter()
                    .any(|w| w == &format!("model_reasoning_effort={effort}")),
                "effort {effort}: {short:?}"
            );
        }
        assert!(
            words.iter().any(|word| word == CONV),
            "same conversation: {short:?}"
        );
    }

    fn saved(&self, model: &str, effort: &str) {
        assert_eq!(
            self.rig.meta_row("observed_model.main"),
            model,
            "{}",
            self.rig.meta()
        );
        assert_eq!(
            self.rig.meta_row("observed_effort.main"),
            effort,
            "{}",
            self.rig.meta()
        );
        assert_eq!(
            self.rig.meta_row("observed_model_pin.main"),
            self.base_model()
        );
        assert_eq!(self.rig.meta_row("observed_effort_pin.main"), "xhigh");
    }

    fn no_saved(&self) {
        for key in [
            "observed_model.main",
            "observed_model_pin.main",
            "observed_effort.main",
            "observed_effort_pin.main",
        ] {
            assert!(
                self.rig.meta_row(key).is_empty(),
                "retired {key}: {}",
                self.rig.meta()
            );
        }
    }

    fn records(&self, action: &str) -> Vec<ae::events::Event> {
        self.rig
            .events()
            .lines()
            .map(|line| ae::events::Event::parse_line(line).expect("journal record"))
            .filter(|event| event.action == action)
            .collect()
    }

    fn crash(&self) {
        let (ok, pid) = self.rig.tmux(&["display-message", "-p", "#{pid}"]);
        assert!(ok);
        let pid = pid.trim().parse::<u32>().expect("private server pid");
        assert!(pid > 1);
        let _ = self.rig.tmux(&["run-shell", &format!("kill -KILL {pid}")]);
        self.until(|| !self.rig.tmux(&["list-sessions"]).0);
    }

    fn launch(&self, bare: bool) {
        let mut command = if bare {
            self.cli(&[])
        } else {
            self.cli(&[&self.rig.session])
        };
        let out = command
            .arg("--no-attach")
            .output()
            .expect("real resume/restore");
        assert!(
            out.status.success(),
            "resume/restore: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn quota(&self, weekly: f64) {
        let now = ae::time::Timestamp::now().epoch();
        let json = format!(
            r#"{{"cachedUsageUtilization":{{"fetchedAtMs":{},"utilization":{{"limits":[{{"kind":"weekly_all","group":"weekly_all","percent":{weekly},"resets_at":"{}","scope":null}},{{"kind":"session","group":"session","percent":0,"resets_at":"{}","scope":null}}]}}}}}}"#,
            now * 1000,
            ae::time::Timestamp::from_epoch(now + 86400),
            ae::time::Timestamp::from_epoch(now + 3600)
        );
        ae::json::parse(&json).expect("quota JSON");
        std::fs::write(self.rig.scratch.join("account/.claude.json"), json).expect("private quota");
    }
}

#[test]
fn a1_claude_resumes_both_recorded_values_on_the_same_conversation() {
    let seat = Seat::new("mpa1", "claude");
    seat.config(
        &[
            ("base", seat.command("fable", "xhigh")),
            ("opus", seat.command("claude-opus-5-5", "xhigh")),
        ],
        false,
    );
    seat.observe_rows("Opus 5.5", Some("high"));
    let (argv, _) = seat.plan();
    seat.pair(&argv, "claude-opus-5-5", "high");
}

#[test]
fn a2_codex_resumes_both_values_after_the_resume_id() {
    let seat = Seat::new("mpa2", "codex");
    seat.config(
        &[
            ("base", seat.command("gpt-6-astra", "xhigh")),
            ("sol", seat.command("gpt-6.1-sol", "max")),
        ],
        false,
    );
    seat.observe_rows("gpt-6.1-sol", Some("high"));
    let (argv, _) = seat.plan();
    seat.pair(&argv, "gpt-6.1-sol", "high");
    let resume = argv
        .iter()
        .position(|word| word == CONV)
        .expect("resume id");
    let effort = argv
        .iter()
        .position(|word| word == "model_reasoning_effort=high")
        .expect("manual effort");
    assert!(effort > resume, "Codex reads overrides after resume <id>");
}

#[test]
fn a3_effort_only_choice_needs_no_other_model_profile() {
    for (tag, tool, label) in [
        ("mpa3c", "claude", "Fable 5.1"),
        ("mpa3x", "codex", "gpt-6-astra"),
    ] {
        let seat = Seat::new(tag, tool);
        seat.observe_rows(label, Some("high"));
        let (argv, _) = seat.plan();
        seat.pair(&argv, seat.base_model(), "high");
    }
}

#[test]
fn a4_editing_either_recorded_pin_before_resume_uses_the_whole_new_profile() {
    for (tag, model, effort) in [
        ("mpa4m", "claude-sonnet-5-5", "xhigh"),
        ("mpa4e", "fable", "max"),
    ] {
        let seat = Seat::new(tag, "claude");
        seat.observe_rows("Opus 5.5", Some("high"));
        seat.config(
            &[
                ("base", seat.command(model, effort)),
                ("opus", seat.command("claude-opus-5-5", "xhigh")),
            ],
            false,
        );
        let (argv, _) = seat.plan();
        seat.pair(&argv, model, effort);
        seat.no_saved();
    }
}

#[test]
fn a5_refusal_holds_the_pair_through_sweeps_and_each_new_tool_start() {
    let seat = Seat::new("mpa5", "claude");
    seat.observe_rows("Sonnet 5.5", Some("high"));
    let done = "{\"ts\":\"2026-10-20T00:00:00Z\",\"actor\":\"lead\",\"action\":\"state\",\"target\":\"lead\",\"ref\":\"done\",\"summary\":\"done\"}\n";
    std::fs::write(seat.rig.dir.join("events.jsonl"), done).expect("carried done");
    seat.start();
    seat.pair(&seat.argv(), "fable", "xhigh");
    let marker1 = seat.rig.meta_row("launched.main");
    assert_eq!(
        marker1,
        format!("main:{LAUNCH}:{}", seat.pids().last().expect("tool pid")),
        "tool-start identity"
    );
    let daemon = seat.watch();
    seat.cycles(2);
    seat.saved("Sonnet 5.5", "high");
    let first = seat.rig.events();
    let refusals = |text: &str| {
        text.lines()
            .map(|line| ae::events::Event::parse_line(line).expect("event"))
            .filter(|e| e.reference.as_deref() == Some(marker1.as_str()))
            .collect::<Vec<_>>()
    };
    let records = refusals(&first);
    assert_eq!(records.len(), 1, "one refusal per start: {first}");
    assert_eq!(records[0].actor, "watchdog");
    assert!(
        seat.listing(false).contains("no profile for Sonnet 5.5"),
        "unresolved refusal is visible before config resolves it"
    );
    assert!(
        seat.listing(false).contains("finished"),
        "refusal ends no carried done"
    );
    let _ = seat.plan();
    let _ = seat.listing(false);
    assert_eq!(
        refusals(&seat.rig.events()).len(),
        1,
        "read surfaces journal nothing"
    );
    drop(daemon);
    seat.start();
    let marker2 = seat.rig.meta_row("launched.main");
    assert_ne!(
        marker1, marker2,
        "new tool start even though launch_id is retained"
    );
    let _daemon = seat.watch();
    seat.cycles(2);
    seat.saved("Sonnet 5.5", "high");
    let all = seat.rig.events();
    assert_eq!(
        all.lines()
            .map(|line| ae::events::Event::parse_line(line).expect("event"))
            .filter(|e| e.reference.as_deref() == Some(marker2.as_str()))
            .count(),
        1
    );
    seat.config(
        &[
            ("base", seat.command("fable", "xhigh")),
            ("sonnet", seat.command("claude-sonnet-5-5", "max")),
        ],
        false,
    );
    let shown = seat.listing(false);
    assert!(
        !shown.contains("no profile for"),
        "resolved reason clears immediately: {shown}"
    );
}

#[test]
fn a6_witnessed_new_choice_then_return_to_both_pins_retires_refused_pair() {
    let seat = Seat::new("mpa6", "claude");
    seat.observe_rows("Sonnet 5.5", Some("high"));
    seat.start();
    let _daemon = seat.watch();
    seat.manual("Opus 5.5", "max");
    seat.cycles(1);
    seat.saved("Opus 5.5", "max");
    assert!(seat.rig.meta_row("launched.main").is_empty());
    seat.manual("Fable 5.1", "xhigh");
    seat.cycles(1);
    seat.no_saved();
}

#[test]
fn a7_auto_owned_level_is_not_manual_and_recovery_still_lands() {
    let seat = Seat::new("mpa7", "claude");
    seat.config(
        &[
            ("base", seat.command("fable", "xhigh")),
            ("low", seat.command("fable", "high")),
        ],
        true,
    );
    seat.quota(95.0);
    seat.start();
    let _daemon = seat.watch();
    seat.until(|| seat.records("effort-applied").len() == 1);
    seat.cycles(1);
    seat.no_saved();
    seat.quota(0.0);
    seat.until(|| seat.records("effort-restored").len() == 1);
    seat.pair(&seat.argv(), "fable", "xhigh");
    seat.no_saved();
}

#[test]
fn a7_manual_effort_while_an_episode_is_open_yields_and_is_preserved() {
    let seat = Seat::new("mpa7manual", "claude");
    seat.config(
        &[
            ("base", seat.command("fable", "xhigh")),
            ("low", seat.command("fable", "high")),
        ],
        true,
    );
    seat.quota(95.0);
    seat.start();
    let _daemon = seat.watch();
    seat.until(|| seat.records("effort-applied").len() == 1);
    seat.manual("Fable 5.1", "max");
    seat.until(|| !seat.records("effort-yielded").is_empty());
    seat.cycles(1);
    seat.saved("Fable 5.1", "max");
    seat.quota(0.0);
    seat.cycles(1);
    assert!(seat.records("effort-restored").is_empty());
    seat.pair(&seat.plan().0, "fable", "max");
}

#[test]
fn a8_candidate_effort_is_ignored_but_only_its_model_value_is_taken() {
    let seat = Seat::new("mpa8", "claude");
    let base = seat.command("fable", "xhigh");
    seat.config(
        &[
            ("base", base.clone()),
            ("sonnet", seat.command("claude-sonnet-5-5", "max")),
        ],
        false,
    );
    let config = std::fs::read(seat.rig.scratch.join("config")).expect("config bytes");
    seat.observe_rows("Sonnet 5.5", Some("high"));
    seat.pair(&seat.plan().0, "claude-sonnet-5-5", "high");
    assert_eq!(
        std::fs::read(seat.rig.scratch.join("config")).expect("config after"),
        config
    );
    assert_eq!(seat.rig.meta_row("profile.main"), "base");
}

#[test]
fn a8_account_executable_permission_and_ambiguous_candidates_refuse_the_whole_pair() {
    for guard in ["account", "executable", "permission", "ambiguous"] {
        let seat = Seat::new(&format!("mpa8{guard}"), "claude");
        let candidate = seat.command("claude-sonnet-5-5", "high");
        let changed = match guard {
            "account" => candidate.replace("/account ", "/other-account "),
            "executable" => candidate.replace("/tools/claude ", "/other/claude "),
            "permission" => candidate.replace("bypassPermissions", "acceptEdits"),
            _ => candidate.replace("claude-sonnet-5-5", "claude-sonnet-5-5-20260101"),
        };
        let mut profiles = vec![("base", seat.command("fable", "xhigh")), ("other", changed)];
        if guard == "ambiguous" {
            profiles.push((
                "another",
                candidate.replace("claude-sonnet-5-5", "claude-sonnet-5-5-20260202"),
            ));
        }
        seat.config(&profiles, false);
        seat.observe_rows("Sonnet 5.5", Some("high"));
        seat.pair(&seat.plan().0, "fable", "xhigh");
        seat.saved("Sonnet 5.5", "high");
    }
}

#[test]
fn a9_named_resume_and_bare_crash_restore_keep_all_four_rows() {
    for (tag, bare) in [("mpa9named", false), ("mpa9bare", true)] {
        let seat = Seat::new(tag, "claude");
        seat.config(
            &[
                ("base", seat.command("fable", "xhigh")),
                ("opus", seat.command("claude-opus-5-5", "max")),
            ],
            false,
        );
        seat.observe_rows("Opus 5.5", Some("high"));
        let now = ae::time::Timestamp::now().epoch();
        std::fs::write(
            seat.rig.dir.join(".launch-attempt"),
            format!("{}\n", now - 600),
        )
        .expect("eligible launch");
        let beat =
            std::fs::File::create(seat.rig.dir.join(".watchdog-beat")).expect("watched cohort");
        beat.set_modified(
            SystemTime::UNIX_EPOCH
                + Duration::from_secs(u64::try_from(now - 60).expect("positive epoch")),
        )
        .expect("recent cohort");
        seat.crash();
        seat.launch(bare);
        seat.saved("Opus 5.5", "high");
        seat.pair(&seat.plan().0, "claude-opus-5-5", "high");
        assert_eq!(seat.rig.meta_row("harness_session.main"), CONV);
    }
}

#[test]
fn a9_clean_stop_observes_the_current_pair_before_named_resume() {
    let seat = Seat::new("mpa9stop", "claude");
    seat.config(
        &[
            ("base", seat.command("fable", "xhigh")),
            ("opus", seat.command("claude-opus-5-5", "xhigh")),
        ],
        false,
    );
    seat.start();
    seat.manual("Opus 5.5", "high");
    // The human authorizes this stop of the fixture's own private session.
    let out = seat
        .cli(&["stop", &seat.rig.session, "-y"])
        .output()
        .expect("clean private stop");
    assert!(
        out.status.success(),
        "stop: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    seat.saved("Opus 5.5", "high");
    seat.launch(false);
    seat.pair(&seat.plan().0, "claude-opus-5-5", "high");
}

#[test]
fn a10_list_shows_manual_effort_and_keeps_the_json_key_contract() {
    use std::collections::BTreeSet;

    fn keys(value: &Value) -> BTreeSet<&str> {
        let Value::Obj(fields) = value else {
            panic!("documented JSON object")
        };
        fields.iter().map(|(key, _)| key.as_str()).collect()
    }

    fn shape(value: &Value) -> String {
        match value {
            Value::Obj(fields) => {
                let mut fields = fields
                    .iter()
                    .map(|(key, value)| format!("{key}:{}", shape(value)))
                    .collect::<Vec<_>>();
                fields.sort();
                format!("{{{}}}", fields.join(","))
            }
            Value::Arr(values) => format!(
                "[{}]",
                values.iter().map(shape).collect::<Vec<_>>().join(",")
            ),
            _ => "value".to_owned(),
        }
    }
    let seat = Seat::new("mpa10", "claude");
    let before = seat.listing(true);
    seat.observe_rows("Opus 5.5", Some("high"));
    let shown = seat.listing(false);
    assert!(
        shown.contains("Opus 5.5 (high)"),
        "manual pair is visible: {shown}"
    );
    assert_eq!(
        shape(&ae::json::parse(&before).expect("before JSON")),
        shape(&ae::json::parse(&seat.listing(true)).expect("after JSON")),
        "manual pair adds no JSON field"
    );
    // Literal published schema-2 contract, independent of both current outputs.
    let doc = ae::json::parse(&seat.listing(true)).expect("schema-2 document");
    assert_eq!(
        keys(&doc),
        BTreeSet::from([
            "schema_version",
            "generated_at",
            "sessions",
            "inventory_complete"
        ])
    );
    let Some(Value::Arr(sessions)) = doc.get("sessions") else {
        panic!("sessions array")
    };
    assert_eq!(sessions.len(), 1);
    let Some(Value::Arr(agents)) = sessions[0].get("agents") else {
        panic!("agents array")
    };
    assert_eq!(agents.len(), 1);
    assert_eq!(
        keys(&agents[0]),
        BTreeSet::from([
            "ref",
            "alias",
            "name",
            "session_id",
            "alive",
            "observed",
            "state",
            "reason",
            "client",
            "account",
            "runtime"
        ])
    );
    assert_eq!(
        keys(agents[0].get("runtime").expect("runtime object")),
        BTreeSet::from(["model", "effort"])
    );
}

#[test]
fn a11_a_legacy_record_adopts_the_effort_from_a_later_proved_frame() {
    let seat = Seat::new("mpa11", "claude");
    seat.config(
        &[
            ("base", seat.command("fable", "xhigh")),
            ("opus", seat.command("claude-opus-5-5", "xhigh")),
        ],
        false,
    );
    seat.observe_rows("Opus 5.5", None);
    seat.pair(&seat.plan().0, "claude-opus-5-5", "xhigh");
    seat.start();
    seat.manual("Opus 5.5", "high");
    let _daemon = seat.watch();
    seat.cycles(1);
    seat.saved("Opus 5.5", "high");
    seat.pair(&seat.plan().0, "claude-opus-5-5", "high");
}

#[test]
fn malformed_pair_rows_are_ignored_preserved_and_reported() {
    for damage in ["partial", "duplicate", "invalid"] {
        let seat = Seat::new(&format!("mpbad{damage}"), "claude");
        seat.config(
            &[
                ("base", seat.command("fable", "xhigh")),
                ("opus", seat.command("claude-opus-5-5", "xhigh")),
            ],
            false,
        );
        seat.observe_rows("Opus 5.5", Some("high"));
        match damage {
            "partial" => {
                let text = seat
                    .rig
                    .meta()
                    .lines()
                    .filter(|line| !line.starts_with("observed_effort_pin.main="))
                    .collect::<Vec<_>>()
                    .join("\n");
                std::fs::write(seat.rig.dir.join("meta"), format!("{text}\n"))
                    .expect("partial record");
            }
            "duplicate" => {
                let text = format!("{}observed_effort.main=max\n", seat.rig.meta());
                std::fs::write(seat.rig.dir.join("meta"), text).expect("duplicated row");
            }
            _ => seat.row("observed_effort.main", "bogus"),
        }
        let raw = seat.rig.meta();
        seat.pair(&seat.plan().0, "fable", "xhigh");
        assert_eq!(seat.rig.meta(), raw, "parse doubt never deletes a record");
        seat.diagnoses_damage();
    }
}

#[test]
fn haiku_is_observed_but_a_future_unpublished_label_stays_fail_closed() {
    let seat = Seat::new("mphaiku", "claude");
    seat.start();
    let _daemon = seat.watch();
    seat.manual("Haiku 5.5", "high");
    seat.cycles(1);
    seat.saved("Haiku 5.5", "high");
    seat.manual("Future 99.9", "max");
    seat.cycles(1);
    seat.saved("Haiku 5.5", "high");
}

#[test]
fn damaged_refusal_marker_is_not_cleanup_authority() {
    let seat = Seat::new("mpbadmarker", "claude");
    seat.start();
    seat.observe_rows("Sonnet 5.5", Some("high"));
    seat.row("launched.main", "damaged marker");
    let _daemon = seat.watch();
    seat.cycles(1);
    seat.saved("Sonnet 5.5", "high");
    assert_eq!(seat.rig.meta_row("launched.main"), "damaged marker");
    seat.diagnoses_damage();
    seat.manual("Opus 5.5", "max");
    seat.cycles(1);
    seat.saved("Opus 5.5", "max");
    assert_eq!(seat.rig.meta_row("launched.main"), "damaged marker");
}

#[test]
fn a_non_refused_start_replaces_even_a_damaged_old_start_baseline() {
    let seat = Seat::new("mpclear", "claude");
    seat.row("launched.main", "damaged marker");
    seat.start();
    assert_eq!(
        seat.rig.meta_row("launched.main"),
        format!("main:{LAUNCH}:{}", seat.pids().last().expect("tool pid"))
    );
    assert_eq!(seat.rig.meta_row("launched_model.main"), "fable");
    assert_eq!(seat.rig.meta_row("launched_effort.main"), "xhigh");
}

#[test]
fn g7_model_or_effort_profile_edits_while_running_retire_old_pair() {
    for (tag, model, effort) in [
        ("mpg7model", "claude-sonnet-5-5", "xhigh"),
        ("mpg7effort", "fable", "max"),
    ] {
        let seat = Seat::new(tag, "claude");
        seat.start();
        seat.config(
            &[
                ("base", seat.command(model, effort)),
                ("opus", seat.command("claude-opus-5-5", "xhigh")),
            ],
            false,
        );
        let _daemon = seat.watch();
        seat.cycles(1);
        seat.no_saved();
        seat.pair(&seat.plan().0, model, effort);
    }
}

#[test]
fn g7_a_human_choice_after_a_profile_edit_is_recorded_under_the_new_pin() {
    let seat = Seat::new("mpg7new", "claude");
    seat.start();
    seat.observe_rows("Opus 5.5", Some("high"));
    seat.config(
        &[
            ("base", seat.command("claude-sonnet-5-5", "max")),
            ("haiku", seat.command("claude-haiku-5-5", "xhigh")),
        ],
        false,
    );
    seat.manual("Haiku 5.5", "high");
    let _daemon = seat.watch();
    seat.cycles(1);
    assert_eq!(seat.rig.meta_row("observed_model.main"), "Haiku 5.5");
    assert_eq!(seat.rig.meta_row("observed_effort.main"), "high");
    assert_eq!(
        seat.rig.meta_row("observed_model_pin.main"),
        "claude-sonnet-5-5"
    );
    assert_eq!(seat.rig.meta_row("observed_effort_pin.main"), "max");
    seat.pair(&seat.plan().0, "claude-haiku-5-5", "high");
}

#[test]
fn g7_a_followed_pair_is_not_re_pinned_after_a_running_profile_edit() {
    for (tag, model, effort) in [
        ("mpg7followm", "claude-sonnet-5-5", "xhigh"),
        ("mpg7followe", "fable", "max"),
    ] {
        let seat = Seat::new(tag, "claude");
        seat.config(
            &[
                ("base", seat.command("fable", "xhigh")),
                ("opus", seat.command("claude-opus-5-5", "xhigh")),
            ],
            false,
        );
        seat.observe_rows("Opus 5.5", Some("high"));
        seat.start();
        seat.pair(&seat.argv(), "claude-opus-5-5", "high");
        seat.config(
            &[
                ("base", seat.command(model, effort)),
                ("opus", seat.command("claude-opus-5-5", "xhigh")),
            ],
            false,
        );
        let daemon = seat.watch();
        seat.cycles(1);
        seat.saved("Opus 5.5", "high");
        drop(daemon);
        seat.start();
        seat.pair(&seat.argv(), model, effort);
        seat.no_saved();
    }
}

#[test]
fn refusal_to_its_recorded_pair_then_to_profile_pair_is_a_witnessed_return() {
    let seat = Seat::new("mpreturn", "claude");
    seat.observe_rows("Sonnet 5.5", Some("high"));
    seat.start();
    let _daemon = seat.watch();
    seat.manual("Sonnet 5.5", "high");
    seat.cycles(1);
    seat.saved("Sonnet 5.5", "high");
    assert!(seat.rig.meta_row("launched.main").is_empty());
    seat.manual("Fable 5.1", "xhigh");
    seat.cycles(1);
    seat.no_saved();
}

#[test]
fn malformed_effort_flags_refuse_retain_and_never_append_an_effort_flag() {
    for (tag, suffix, count) in [
        ("mpnoflag", "", 0),
        ("mpdupflag", " --effort high", 2),
        ("mpbadflag", "unsupported", 1),
    ] {
        let seat = Seat::new(tag, "claude");
        let base = seat.command("fable", "xhigh");
        let broken = match suffix {
            "" => base.replace(" --effort xhigh", ""),
            "unsupported" => base.replace("--effort xhigh", "--effort extreme"),
            _ => format!("{base}{suffix}"),
        };
        seat.config(
            &[
                ("base", broken),
                ("opus", seat.command("claude-opus-5-5", "xhigh")),
            ],
            false,
        );
        seat.observe_rows("Opus 5.5", Some("high"));
        let (argv, _) = seat.plan();
        assert!(
            argv.windows(2).any(|words| words == ["--model", "fable"]),
            "whole-pair fallback: {:?}",
            &argv[..argv.len().min(12)]
        );
        assert_eq!(
            argv.iter().filter(|word| *word == "--effort").count(),
            count,
            "never append"
        );
        seat.saved("Opus 5.5", "high");
        seat.start_drawn();
        seat.saved("Opus 5.5", "high");
        let reference = format!("main:{LAUNCH}:{}", seat.pids().last().expect("tool pid"));
        let events = seat.rig.events();
        assert_eq!(
            events
                .lines()
                .map(|line| ae::events::Event::parse_line(line).expect("event"))
                .filter(|event| event.reference.as_deref() == Some(reference.as_str()))
                .count(),
            1,
            "one refusal record: {events}"
        );
    }
}

#[test]
fn explicit_reseat_drops_the_observed_pair() {
    let seat = Seat::new("mpreseat", "claude");
    seat.config(
        &[
            ("base", seat.command("fable", "xhigh")),
            ("new", seat.command("claude-opus-5-5", "high")),
        ],
        false,
    );
    seat.start();
    seat.observe_rows("Sonnet 5.5", Some("max"));
    let out = seat
        .cli(&["reseat", &seat.rig.session, "lead", "--using", "new"])
        .output()
        .expect("explicit private reseat");
    assert!(
        out.status.success(),
        "reseat: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    seat.no_saved();
    assert_eq!(seat.rig.meta_row("profile.main"), "new");
}
