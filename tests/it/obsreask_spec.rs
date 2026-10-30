//! Frozen #285 acceptance: one bounded re-ask of a failed tmux observation.
//! Real helpers and a private tmux; a fake `tmux` on PATH answers ONE named read by plan,
//! so no case depends on timing and the guard itself stays real.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "private-server fixtures read and alter only their scratch state"
)]
use super::renametell_routing_spec::{A, Rig};
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::Output;

const BODY: &str = "obsreask-original-task";
const PHYSICAL: &str = "#{session_id} | #{session_created}";
const NAME: &str = "#{session_name}";
const VIEWER: &str =
    "#{@ae_slot} | #{session_name} | #{@ae_agent} | #{@ae_session_uuid} | #{socket_path}";
const REQUESTS_NO_IDENTITY: &str =
    "Error: could not detect current agent identity; use 'requests all' outside an ae pane\n";
const STATE_NO_IDENTITY: &str =
    "Error: could not detect current agent identity; declare state from an ae pane\n";
const ACCEPT_NO_IDENTITY: &str =
    "Error: could not detect current agent identity; run accept from your own ae pane\n";
const UNREADABLE: &str = "could not be proven: its physical tmux session is unreadable";
const STAMPS: &str = "could not be proven: its pane stamps are unreadable or incomplete";
const CHANGED: &str = "could not be proven: its physical session or session id changed";

// Armed reads take one step of OBS_PLAN each, then OBS_REST: `fail` exits 1, `lie` prints a
// POSITIVE answer, `real` asks tmux. The log is the order the helper really asked in. A swallowed
// Enter is a lost key, held until the box draws the turn: tmux returning proves a paste queued,
// not consumed, and only a box still holding it makes the helper's retry reach Stage::Entered.
const TMUX_FAULT: &str = r#"#!/usr/bin/perl
use strict; use warnings;
my ($dir, $real, $target, $format, $pane, $after) =
    @ENV{qw(OBS_DIR OBS_REAL OBS_TARGET OBS_FORMAT OBS_PANE OBS_AFTER)};
sub note { open(my $log, '>>', "$dir/log") or die; print $log "@_\n"; close($log); }
sub seen {
    open(my $log, '<', "$dir/log") or return ();
    my @lines = <$log>; close($log); chomp(@lines); return @lines;
}
sub ask { exec {$real} $real, @ARGV; die 'exec tmux'; }
sub drawn {
    my @server; for (@ARGV) { last if $_ eq 'send-keys'; push @server, $_; }
    my $end = time() + 90;
    while (1) {
        open(my $screen, '-|', $real, @server, 'capture-pane', '-p', '-t', $pane) or die;
        my $text = do { local $/; <$screen> } // ''; close($screen);
        return if index($text, $ENV{OBS_BODY}) >= 0;
        die 'box deadline' if time() > $end;
        select(undef, undef, undef, .01);
    }
}
sub mutate {
    my $mode = $ENV{OBS_MUTATE}; return unless $mode;
    open(my $f, '<', $ENV{OBS_META}) or die; my $text = do { local $/; <$f> }; close($f);
    $text =~ s/^launch_id\.worker\.0=.*$/launch_id.worker.0=tok-replaced/m if $mode eq 'replace';
    $text =~ s/^[a-z_]+\.worker\.0=.*\n//mg if $mode eq 'remove';
    open(my $w, '>', "$ENV{OBS_META}.next") or die; print $w $text; close($w);
    rename("$ENV{OBS_META}.next", $ENV{OBS_META}) or die;
}
my $aimed = $pane ne '' && grep { $_ eq $pane } @ARGV;
if ($aimed && grep { $_ eq 'paste-buffer' } @ARGV) {
    system {$real} $real, @ARGV; exit($? >> 8) if $?;
    note('paste'); mutate() if $after eq 'paste'; exit 0;
}
if ($aimed && $ARGV[-1] eq 'Enter' && grep { $_ eq 'send-keys' } @ARGV) {
    if ($after eq 'enter' && !grep { $_ eq 'swallowed' } seen()) {
        drawn(); note('swallowed'); mutate(); exit 0;
    }
    note('enter'); ask();
}
if ($ARGV[-1] eq $format && grep { $_ eq $target } @ARGV) {
    my @seen = seen();
    my $arm = { start => '', paste => 'paste', enter => 'swallowed' }->{$after};
    if ($arm eq '' || grep { $_ eq $arm } @seen) {
        my $asked = grep { /^read / } @seen;
        my $step = (split ' ', $ENV{OBS_PLAN})[$asked] // $ENV{OBS_REST};
        note("read $step");
        exit 1 if $step eq 'fail';
        if ($step eq 'lie') { print "$ENV{OBS_LIE}\n"; exit 0; }
    }
}
ask();
"#;

/// One planned read: `format` asked of `target`, armed after `after`.
#[derive(Clone, Copy)]
struct Fault<'a> {
    format: &'a str,
    target: &'a str,
    pane: &'a str,
    after: &'a str,
    plan: &'a str,
    rest: &'a str,
    lie: &'a str,
    mutate: &'a str,
}

/// The caller's identity read, off the pane the helper runs in.
const fn caller<'a>(pane: &'a str, plan: &'a str, rest: &'a str) -> Fault<'a> {
    Fault {
        format: VIEWER,
        target: pane,
        pane: "",
        after: "start",
        plan,
        rest,
        lie: "",
        mutate: "",
    }
}

struct World {
    rig: Rig,
    main: String,
    peer: String,
    bin: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let rig = Rig::new(&format!("obs-{tag}"));
        let [main, peer] = rig.create("s", A);
        let bin = rig.scratch.join("fault");
        fs::create_dir_all(&bin).expect("fault directory");
        let shim = bin.join("tmux");
        fs::write(&shim, TMUX_FAULT).expect("fault shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("executable shim");
        Self {
            rig,
            main,
            peer,
            bin,
        }
    }

    /// The target's physical-session read, as the guard asks it.
    fn physical<'a>(&'a self, after: &'a str, plan: &'a str, rest: &'a str) -> Fault<'a> {
        Fault {
            format: PHYSICAL,
            target: &self.peer,
            pane: &self.peer,
            after,
            plan,
            rest,
            lie: "",
            mutate: "",
        }
    }

    /// The target's stamp read, the other half of the guard's one observation.
    fn stamps<'a>(&'a self, after: &'a str, plan: &'a str, rest: &'a str) -> Fault<'a> {
        Fault {
            format: VIEWER,
            ..self.physical(after, plan, rest)
        }
    }

    fn faulted(&self, pane: Option<&str>, args: &[&str], fault: &Fault<'_>) -> Output {
        let log = self.bin.join("log");
        if log.exists() {
            fs::remove_file(&log).expect("fresh read log");
        }
        let mut cmd = self.rig.command(pane);
        cmd.args(args).env("AE_SEND_DEFER_SEC", "15");
        let path = cmd
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .and_then(|(_, value)| value)
            .expect("isolated PATH")
            .to_owned();
        let real = std::env::split_paths(&path)
            .map(|dir| dir.join("tmux"))
            .find(|path| path.is_file())
            .expect("real tmux")
            .canonicalize()
            .expect("absolute real tmux");
        let mut paths = vec![self.bin.clone()];
        paths.extend(std::env::split_paths(&path));
        cmd.env("PATH", std::env::join_paths(paths).expect("fault PATH"))
            .env("OBS_DIR", &self.bin)
            .env("OBS_REAL", real)
            .env("OBS_META", self.rig.dir("s").join("meta"))
            .env("OBS_FORMAT", fault.format)
            .env("OBS_TARGET", fault.target)
            .env("OBS_PANE", fault.pane)
            .env("OBS_AFTER", fault.after)
            .env("OBS_PLAN", fault.plan)
            .env("OBS_REST", fault.rest)
            .env("OBS_LIE", fault.lie)
            .env("OBS_MUTATE", fault.mutate)
            .env("OBS_BODY", BODY);
        cmd.output().expect("faulted helper")
    }

    fn send(&self, fault: &Fault<'_>) -> Output {
        self.faulted(Some(&self.main), &["@s", "send", "colead", BODY], fault)
    }

    fn log(&self) -> Vec<String> {
        fs::read_to_string(self.bin.join("log"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    /// The first unbroken run of armed reads: what one check asked, before its next key.
    fn reads(&self) -> Vec<String> {
        self.log()
            .into_iter()
            .skip_while(|line| !line.starts_with("read "))
            .take_while(|line| line.starts_with("read "))
            .collect()
    }

    fn keys(&self, key: &str) -> usize {
        self.log().iter().filter(|line| *line == key).count()
    }

    fn events(&self) -> String {
        fs::read_to_string(self.rig.dir("s").join("events.jsonl")).unwrap_or_default()
    }

    fn count(&self, action: &str) -> usize {
        self.events()
            .matches(&format!("\"action\":\"{action}\""))
            .count()
    }

    fn line(&self, pane: &str, format: &str) -> String {
        let out = self
            .rig
            .tmux(&["display-message", "-p", "-t", pane, format]);
        out.trim_end().to_owned()
    }

    /// A session that really exists on this server, as the physical read would name it.
    fn elsewhere(&self) -> String {
        self.rig.tmux(&[
            "new-session",
            "-d",
            "-s",
            "elsewhere",
            "-x",
            "80",
            "-y",
            "24",
            "sleep 600",
        ]);
        self.line("=elsewhere:", PHYSICAL)
    }

    /// The real viewer line with one stamp rewritten: a positive answer tmux could give.
    fn stamped(&self, pane: &str, from: &str, to: &str) -> String {
        let line = self.line(pane, VIEWER);
        assert!(line.contains(from), "fixture viewer line: {line}");
        line.replace(from, to)
    }

    fn unstamped_main(&self) -> String {
        self.stamped(&self.main, "main | s | lead | ", " | s |  | ")
    }

    fn ask(&self, pane: &str, target: &str, text: &str) -> String {
        let (code, out, err) = self.rig.helper("s", "ask", &[target, text], Some(pane));
        assert_eq!(code, Some(0), "request setup: {out}{err}");
        out.trim().to_owned()
    }

    fn request_view(&self, mode: &str, pane: Option<&str>) -> String {
        let (code, out, err) = self.rig.helper("s", "requests", &[mode], pane);
        assert_eq!(code, Some(0), "request view setup: {out}{err}");
        out
    }

    fn delivered(&self, output: &Output) {
        let err = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(0), "not delivered: {err}");
        assert!(!err.contains("REFUSED"), "refused: {err}");
        assert_eq!(self.count("refused"), 0, "refusal: {}", self.events());
        let got = self.rig.received(A, "colead");
        assert_eq!(got.len(), 1, "exactly one submit: {got:?}");
        assert!(got[0].contains(BODY), "wrong turn: {got:?}");
    }

    fn refused(&self, output: &Output, after: &str, words: &str) {
        let err = String::from_utf8_lossy(&output.stderr);
        let clause = if after == "paste" {
            "no Enter was sent"
        } else {
            "submission unconfirmed"
        };
        assert_eq!(output.status.code(), Some(1), "not refused: {err}");
        assert!(
            err.contains("REFUSED") && err.contains(words) && err.contains(clause),
            "wrong refusal: {err}"
        );
        assert_eq!(self.count("refused"), 1, "one audit: {}", self.events());
        assert_eq!(self.count("send"), 0, "claimed: {}", self.events());
        assert_eq!(self.keys("enter"), 0, "extra key: {:?}", self.log());
        assert_eq!(
            self.keys("swallowed"),
            usize::from(after == "enter"),
            "keys up to the refusing stage: {:?}",
            self.log()
        );
        assert!(self.rig.received(A, "colead").is_empty(), "a turn landed");
    }

    /// A refused caller wrote nothing, and nothing in the journal names a stand-in for it.
    fn no_stand_in(&self, before: &str) {
        let events = self.events();
        assert_eq!(events, before, "a refused caller wrote a record");
        for actor in ["human", "unknown", "external"] {
            assert!(
                !events.contains(&format!("\"actor\":\"{actor}\"")),
                "failed read became {actor}: {events}"
            );
        }
    }
}

fn pending(view: &str, id: &str) -> bool {
    view.lines()
        .any(|line| line.starts_with("pending") && line.contains(id))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// Pins base. The RED cases differ from this only by their failed read: a false RED of the
// fixture (a lost Enter that never reaches a retry, a read that never fires) fails here.
#[test]
fn with_every_read_answering_each_fixture_path_delivers_once() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("control-{after}"));
        let output = w.send(&w.physical(after, "", "real"));
        w.delivered(&output);
        assert_eq!(w.reads(), ["read real"], "one proof before the next key");
        assert_eq!(w.keys("swallowed"), usize::from(after == "enter"));
        assert_eq!(w.keys("enter"), 1, "{:?}", w.log());
    }
}

// Kills: no re-ask where the Pasted re-proof reads, and a re-ask only at admission.
#[test]
fn a_physical_read_failing_once_before_the_first_enter_is_reasked() {
    let w = World::new("once-paste");
    let output = w.send(&w.physical("paste", "fail", "real"));
    w.delivered(&output);
    assert_eq!(w.reads(), ["read fail", "read real"]);
    assert_eq!(w.count("send"), 1, "{}", w.events());
}

// Kills: a re-ask wired into `send` alone, when tracked verbs share the guard.
#[test]
fn a_tracked_ask_survives_the_same_failed_read() {
    let w = World::new("once-ask");
    let args = ["@s", "ask", "colead", BODY];
    let output = w.faulted(Some(&w.main), &args, &w.physical("paste", "fail", "real"));
    w.delivered(&output);
    assert_eq!(w.reads(), ["read fail", "read real"]);
    assert_eq!(w.count("ask"), 1, "{}", w.events());
}

// Kills: a re-ask at Stage::Pasted only, and a retry Enter sent past an unanswered proof.
#[test]
fn a_physical_read_failing_once_before_a_retry_enter_is_reasked() {
    let w = World::new("once-enter");
    let output = w.send(&w.physical("enter", "fail", "real"));
    w.delivered(&output);
    assert_eq!(w.reads(), ["read fail", "read real"]);
    assert_eq!(w.keys("swallowed"), 1, "{:?}", w.log());
    assert_eq!(w.count("send"), 1, "{}", w.events());
}

// Kills: a re-ask at the guarded stages only, leaving the admission read refusing.
#[test]
fn a_physical_read_failing_once_at_admission_is_reasked() {
    let w = World::new("once-admit");
    let output = w.send(&w.physical("start", "fail", "real"));
    w.delivered(&output);
    assert!(
        w.log()
            .starts_with(&["read fail".into(), "read real".into()]),
        "{:?}",
        w.log()
    );
}

// Kills: a re-ask of the first leg alone: the observation includes naming its session.
#[test]
fn a_session_name_read_failing_once_inside_the_observation_is_reasked() {
    let w = World::new("once-name");
    let id = w.line("=s:", "#{session_id}");
    let target = format!("={id}:");
    let fault = Fault {
        format: NAME,
        target: &target,
        ..w.physical("paste", "fail", "real")
    };
    let output = w.send(&fault);
    w.delivered(&output);
    assert_eq!(w.reads(), ["read fail", "read real"]);
}

// Kills: a re-ask in the stamped branch of the seat proof, skipping a raw pane's.
#[test]
fn a_raw_pane_target_survives_the_same_failed_read() {
    let w = World::new("once-raw");
    for key in ["@ae_slot", "@ae_agent"] {
        w.rig.tmux(&["set-option", "-p", "-u", "-t", &w.peer, key]);
    }
    let args = ["@s", "send", w.peer.as_str(), BODY];
    let fault = w.physical("start", "fail", "real");
    let output = w.faulted(Some(&w.main), &args, &fault);
    w.delivered(&output);
    assert!(
        w.log()
            .starts_with(&["read fail".into(), "read real".into()]),
        "{:?}",
        w.log()
    );
}

// Pins base. Kills: a failed read taken as a pass, as absence, or as a silent stop.
#[test]
fn a_physical_read_failing_every_time_keeps_the_refusal_and_its_one_audit() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("always-{after}"));
        let output = w.send(&w.physical(after, "", "fail"));
        w.refused(&output, after, UNREADABLE);
    }
}

// Pins base. Kills: a re-ask of any refusal: the seat answers are positive and final.
#[test]
fn a_positive_seat_change_refuses_on_its_first_read() {
    for (mode, word) in [("replace", "was replaced"), ("remove", "is retired")] {
        for after in ["paste", "enter"] {
            let w = World::new(&format!("seat-{mode}-{after}"));
            let fault = Fault {
                mutate: mode,
                ..w.physical(after, "", "real")
            };
            let output = w.send(&fault);
            w.refused(&output, after, word);
            assert_eq!(w.reads(), ["read real"], "a positive answer was re-asked");
        }
    }
}

// Pins base. Kills: a re-ask on any mismatch: the second read would be the bound session.
#[test]
fn a_positive_other_session_is_never_reasked_into_a_pass() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("session-{after}"));
        let lie = w.elsewhere();
        let fault = Fault {
            lie: &lie,
            ..w.physical(after, "lie", "real")
        };
        let output = w.send(&fault);
        w.refused(&output, after, CHANGED);
        assert_eq!(w.reads(), ["read lie"], "a positive answer was re-asked");
    }
}

// Pins base. Kills: a re-ask of the whole proof: the second read would be the bound stamp.
#[test]
fn a_positive_replaced_stamp_is_never_reasked_into_a_pass() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("stamp-{after}"));
        let lie = w.stamped(&w.peer, " | colead | ", " | successor | ");
        let fault = Fault {
            lie: &lie,
            ..w.stamps(after, "lie", "real")
        };
        let output = w.send(&fault);
        w.refused(&output, after, "was replaced");
        assert_eq!(w.reads(), ["read lie"], "a positive answer was re-asked");
    }
}

// Kills: a re-ask of the physical half alone: the stamp read shares the guard's observation.
#[test]
fn a_target_stamp_read_failing_once_at_the_guarded_reproof_is_reasked() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("stamp-once-{after}"));
        let output = w.send(&w.stamps(after, "fail", "real"));
        w.delivered(&output);
        assert_eq!(w.reads(), ["read fail", "read real"]);
        assert_eq!(w.count("send"), 1, "{}", w.events());
    }
}

// Pins base. Kills: a failed stamp read taken as a pass or as the bound stamp.
#[test]
fn a_target_stamp_read_failing_every_time_keeps_its_refusal() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("stamp-always-{after}"));
        let output = w.send(&w.stamps(after, "", "fail"));
        w.refused(&output, after, STAMPS);
    }
}

// Kills: a loop on the stamp read: the third read would answer.
#[test]
fn a_target_stamp_read_failing_twice_refuses_after_exactly_two_reads() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("stamp-twice-{after}"));
        let output = w.send(&w.stamps(after, "fail fail", "real"));
        w.refused(&output, after, STAMPS);
        assert_eq!(w.reads(), ["read fail", "read fail"], "not one re-ask");
    }
}

// Kills: a re-ask whose answer skips the comparison: a recovered read is still a read.
#[test]
fn a_reask_answer_is_compared_like_a_first_answer() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("compare-{after}"));
        let lie = w.elsewhere();
        let fault = Fault {
            lie: &lie,
            ..w.physical(after, "fail lie", "real")
        };
        let output = w.send(&fault);
        w.refused(&output, after, CHANGED);
        assert_eq!(w.reads(), ["read fail", "read lie"]);
    }
}

// Kills: no re-ask on the caller read; a fallback to all rows or to another seat's.
#[test]
fn a_caller_read_failing_once_keeps_the_requests_mine_rows() {
    let w = World::new("caller-mine");
    let mine = w.ask(&w.main, "colead", "from the caller");
    let theirs = w.ask(&w.peer, "lead", "to the caller");
    let control = w.request_view("mine", Some(&w.main));
    assert!(pending(&control, &mine) && !control.contains(&theirs));
    let output = w.faulted(
        Some(&w.main),
        &["@s", "requests", "mine"],
        &caller(&w.main, "fail", "real"),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(String::from_utf8_lossy(&output.stdout), control);
    assert_eq!(w.reads(), ["read fail", "read real"]);
}

// Kills: a re-ask at the requests surface alone, leaving `state` refusing.
#[test]
fn a_caller_read_failing_once_keeps_a_state_declaration() {
    let w = World::new("caller-state");
    let output = w.faulted(
        Some(&w.peer),
        &["@s", "state", "working", "reading"],
        &caller(&w.peer, "fail", "real"),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(w.reads(), ["read fail", "read real"]);
    let events = w.events();
    assert!(
        events
            .lines()
            .any(|line| line.contains("\"action\":\"state\"")
                && line.contains("\"actor\":\"colead\"")),
        "declared under the wrong seat: {events}"
    );
}

// Kills: a re-ask that skips `reply`, whose slot check would read a failed read as `none`.
#[test]
fn a_caller_read_failing_once_keeps_a_reply() {
    let w = World::new("caller-reply");
    let id = w.ask(&w.main, "colead", "question");
    let output = w.faulted(
        Some(&w.peer),
        &["@s", "reply", &id, "answer"],
        &caller(&w.peer, "fail", "real"),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        w.log()
            .starts_with(&["read fail".into(), "read real".into()]),
        "{:?}",
        w.log()
    );
    assert!(
        w.rig
            .received(A, "lead")
            .iter()
            .any(|text| text.contains(&format!("[{id}] answer"))),
        "answer missed the asker"
    );
}

// Kills: a re-ask that skips the record-only verbs.
#[test]
fn a_caller_read_failing_once_keeps_an_accept() {
    let w = World::new("caller-accept");
    let id = w.ask(&w.main, "colead", "question");
    let output = w.faulted(
        Some(&w.peer),
        &["@s", "accept", &id, "taken"],
        &caller(&w.peer, "fail", "real"),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(w.count("accept"), 1, "{}", w.events());
}

// Kills: a re-ask per refusing surface: `send` and `ask` read this same caller, and a failed
// read twice refuses both since #292; it no longer journals `human` or drops to a plain send.
#[test]
fn a_caller_read_failing_once_keeps_the_sender_of_a_delivery() {
    for verb in ["send", "ask"] {
        let w = World::new(&format!("caller-{verb}"));
        let args = ["@s", verb, "colead", BODY];
        let output = w.faulted(Some(&w.main), &args, &caller(&w.main, "fail", "real"));
        w.delivered(&output);
        assert!(
            w.log()
                .starts_with(&["read fail".into(), "read real".into()]),
            "{:?}",
            w.log()
        );
        let events = w.events();
        assert!(
            events
                .lines()
                .any(|line| line.contains(&format!("\"action\":\"{verb}\""))
                    && line.contains("\"actor\":\"lead\"")),
            "{verb} lost its sender: {events}"
        );
        assert!(!events.contains("\"actor\":\"human\""), "{events}");
        let got = w.rig.received(A, "colead");
        assert!(
            got[0].contains("from lead") && !got[0].contains("unverified"),
            "{got:?}"
        );
    }
}

// Pins base. Kills: a failed read read as vacant, then as an external or human caller.
#[test]
fn a_caller_read_failing_every_time_refuses_with_the_existing_text() {
    let w = World::new("caller-always");
    let id = w.ask(&w.main, "colead", "question");
    let before = w.events();
    for (pane, args, text) in [
        (
            &w.main,
            vec!["@s", "requests", "mine"],
            REQUESTS_NO_IDENTITY,
        ),
        (
            &w.peer,
            vec!["@s", "requests", "inbox"],
            REQUESTS_NO_IDENTITY,
        ),
        (
            &w.peer,
            vec!["@s", "state", "working", "x"],
            STATE_NO_IDENTITY,
        ),
        (&w.peer, vec!["@s", "accept", &id, "x"], ACCEPT_NO_IDENTITY),
    ] {
        let output = w.faulted(Some(pane), &args, &caller(pane, "", "fail"));
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert_eq!(stderr(&output), text, "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        assert!(!w.reads().is_empty(), "{args:?}: fault did not run");
        w.no_stand_in(&before);
    }
}

// Pins base. Kills: a slotted reply accepted from a caller that could not be read.
#[test]
fn a_reply_from_an_unreadable_caller_closes_nothing() {
    let w = World::new("caller-reply-always");
    let id = w.ask(&w.main, "colead", "question");
    let before = w.events();
    let output = w.faulted(
        Some(&w.peer),
        &["@s", "reply", &id, "answer"],
        &caller(&w.peer, "", "fail"),
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        stderr(&output),
        format!(
            "Error: request '{id}' is assigned to slot 'worker.0'@'s', current pane is slot 'none'@''\n"
        )
    );
    assert!(!w.reads().is_empty(), "fault did not run");
    w.no_stand_in(&before);
    assert!(pending(&w.request_view("all", None), &id));
    assert!(w.rig.received(A, "lead").is_empty(), "a reply landed");
}

// Pins base. Kills: a re-ask of every unknown viewer: a vacant stamp is a positive answer.
#[test]
fn an_unstamped_caller_answer_is_final_on_its_first_read() {
    let w = World::new("caller-vacant");
    let lie = w.unstamped_main();
    let fault = Fault {
        lie: &lie,
        ..caller(&w.main, "lie", "real")
    };
    let output = w.faulted(Some(&w.main), &["@s", "requests", "mine"], &fault);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(stderr(&output), REQUESTS_NO_IDENTITY);
    assert_eq!(w.reads(), ["read lie"], "a positive answer was re-asked");
}

// Kills: a re-ask that treats its own answer as success whatever it says.
#[test]
fn a_reasked_vacant_caller_still_refuses() {
    let w = World::new("caller-reask-vacant");
    let lie = w.unstamped_main();
    let fault = Fault {
        lie: &lie,
        ..caller(&w.main, "fail lie", "real")
    };
    let output = w.faulted(Some(&w.main), &["@s", "requests", "mine"], &fault);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(stderr(&output), REQUESTS_NO_IDENTITY);
    assert_eq!(w.reads(), ["read fail", "read lie"]);
}

// Kills: a loop: the third read would answer, so only the bound refuses here.
#[test]
fn a_physical_read_failing_twice_refuses_after_exactly_two_reads() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("twice-{after}"));
        let output = w.send(&w.physical(after, "fail fail", "real"));
        w.refused(&output, after, UNREADABLE);
        assert_eq!(w.reads(), ["read fail", "read fail"], "not one re-ask");
    }
}

// Kills: a loop on the naming leg alone: the physical bound never reaches this read. Only NAME
// reads count, so a re-ask of the whole two-leg observation stays legal.
#[test]
fn a_session_name_read_failing_twice_refuses_after_exactly_two_reads() {
    for after in ["paste", "enter"] {
        let w = World::new(&format!("twice-name-{after}"));
        let id = w.line("=s:", "#{session_id}");
        let target = format!("={id}:");
        let fault = Fault {
            format: NAME,
            target: &target,
            ..w.physical(after, "fail fail", "real")
        };
        let output = w.send(&fault);
        w.refused(&output, after, UNREADABLE);
        assert_eq!(w.reads(), ["read fail", "read fail"], "not one re-ask");
    }
}

// Kills: a loop on the caller read.
#[test]
fn a_caller_read_failing_twice_refuses_after_exactly_two_reads() {
    let w = World::new("caller-twice");
    for (args, text) in [
        (vec!["@s", "requests", "mine"], REQUESTS_NO_IDENTITY),
        (vec!["@s", "state", "working", "x"], STATE_NO_IDENTITY),
    ] {
        let fault = caller(&w.main, "fail fail", "real");
        let output = w.faulted(Some(&w.main), &args, &fault);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert_eq!(stderr(&output), text, "{args:?}");
        assert_eq!(w.reads(), ["read fail", "read fail"], "{args:?}");
    }
}
