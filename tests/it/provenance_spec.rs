//! Frozen #292 acceptance: a failed read of the caller is not a human, and a failed read of a
//! target is not a pass. Real helpers and a private tmux; a fake `tmux` on PATH answers the named
//! reads by plan and logs every verb that can reach a pane, so no case depends on timing and the
//! guards and the classifier stay real.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "private-server fixtures read and alter only their scratch state"
)]
use super::cli::{Runner, ae};
use super::renametell_routing_spec::{A, B, CEILING, Rig};
use super::seat_relaunch::Rig as SeatRig;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

const BODY: &str = "provenance-original-task";
const VIEWER: &str =
    "#{@ae_slot} | #{session_name} | #{@ae_agent} | #{@ae_session_uuid} | #{socket_path}";
const OWNER: &str = "#{session_name} | #{@ae_agent}";
const FAMILY: &str = "could not detect current agent identity; ";
const FALLBACK: &str = "Warning: could not detect caller identity (no @ae_agent on this pane). Using 'send' instead.\n";
const OTHER_SEAT: &str = "reseat works on the caller's own session only";
const CROSSING: &str = "REFUSED — another ae session";
const RAW_GAP: &str = "ae: delivery REFUSED — invalid participant session ''; nothing further typed. Report through your own lead; owner routes for an orchestrator role: pane, ae chat, ae app, Telegram.\n";

// Armed reads take one step of OBS_PLAN each, then OBS_REST: `fail` exits 1, `real` asks tmux.
// `wire` lines are the verbs that type into, create or remove a pane, wherever they aim.
const TMUX_FAULT: &str = r#"#!/usr/bin/perl
use strict; use warnings;
my ($dir, $real, $target) = @ENV{qw(OBS_DIR OBS_REAL OBS_TARGET)};
my %format = map { $_ => 1 } split(/\n/, $ENV{OBS_FORMATS});
my %wire = map { $_ => 1 } qw(send-keys paste-buffer load-buffer set-buffer respawn-pane
    kill-pane kill-window new-window split-window);
sub note { open(my $log, '>>', "$dir/log") or die; print $log "@_\n"; close($log); }
sub seen {
    open(my $log, '<', "$dir/log") or return ();
    my @lines = <$log>; close($log); chomp(@lines); return @lines;
}
for (@ARGV) { if ($wire{$_}) { note("wire $_"); last; } }
if ($format{$ARGV[-1]} && grep { $_ eq $target } @ARGV) {
    my $asked = grep { /^read / } seen();
    my $step = (split ' ', $ENV{OBS_PLAN})[$asked] // $ENV{OBS_REST};
    note("read $step");
    exit 1 if $step eq 'fail';
}
exec {$real} $real, @ARGV;
die 'exec tmux';
"#;

/// One planned read: `formats` asked of `target`, step by step through `plan`, then `rest`.
#[derive(Clone, Copy)]
struct Fault<'a> {
    formats: &'a [&'a str],
    target: &'a str,
    plan: &'a str,
    rest: &'a str,
}

/// The caller's identity reads, both spellings, off the pane the verb runs in.
const fn caller<'a>(pane: &'a str, plan: &'a str, rest: &'a str) -> Fault<'a> {
    Fault {
        formats: &[VIEWER, OWNER],
        target: pane,
        plan,
        rest,
    }
}

/// A resolved target's viewer read.
const fn aimed<'a>(pane: &'a str, plan: &'a str, rest: &'a str) -> Fault<'a> {
    Fault {
        formats: &[VIEWER],
        target: pane,
        plan,
        rest,
    }
}

struct Shim {
    bin: PathBuf,
}

impl Shim {
    fn at(scratch: &Path) -> Self {
        let bin = scratch.join("fault");
        fs::create_dir_all(&bin).expect("fault directory");
        let shim = bin.join("tmux");
        fs::write(&shim, TMUX_FAULT).expect("fault shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("executable shim");
        Self { bin }
    }

    fn arm(&self, cmd: &mut Runner, fault: &Fault<'_>) {
        let log = self.bin.join("log");
        if log.exists() {
            fs::remove_file(&log).expect("fresh read log");
        }
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
            .env("OBS_FORMATS", fault.formats.join("\n"))
            .env("OBS_TARGET", fault.target)
            .env("OBS_PLAN", fault.plan)
            .env("OBS_REST", fault.rest);
    }

    fn log(&self, prefix: &str) -> Vec<String> {
        fs::read_to_string(self.bin.join("log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with(prefix))
            .map(ToOwned::to_owned)
            .collect()
    }

    fn reads(&self) -> Vec<String> {
        self.log("read ")
    }

    fn wire(&self) -> Vec<String> {
        self.log("wire ")
    }
}

/// Every plain file of a session directory but its locks: the journal, the meta, the memo TSV,
/// the goal, the stored bodies. A helper link is the core binary, not session state.
fn state(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(next) = todo.pop() {
        for entry in fs::read_dir(&next).expect("session directory") {
            let path = entry.expect("session entry").path();
            let kind = fs::symlink_metadata(&path).expect("entry type").file_type();
            if kind.is_dir() {
                todo.push(path);
            } else if kind.is_file() && !path.to_string_lossy().ends_with(".lock") {
                let bytes = fs::read(&path).expect("session file");
                found.insert(path, bytes);
            }
        }
    }
    found
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Exit 1, the family words with what to do next, nothing on stdout.
fn family(output: &Output) {
    let err = stderr(output);
    assert_eq!(output.status.code(), Some(1), "not refused: {output:?}");
    let next = err.split(FAMILY).nth(1).unwrap_or_default();
    assert!(!next.trim().is_empty(), "wrong refusal: {err}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

/// The refusal R2 names, on any surface: [`family`], no key on the wire, and a session directory
/// byte for byte what it was. No file
/// moving covers every record (`human` or a stand-in), a request, a memo row and the meta.
fn refused(output: &Output, shim: &Shim, before: &BTreeMap<PathBuf, Vec<u8>>, after: &Path) {
    family(output);
    assert_eq!(shim.wire(), Vec::<String>::new(), "a pane was touched");
    let now = state(after);
    assert_eq!(
        now.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>(),
        "a session file came or went"
    );
    for (path, bytes) in before {
        assert_eq!(
            now.get(path).map(|now| String::from_utf8_lossy(now)),
            Some(String::from_utf8_lossy(bytes)),
            "{} moved",
            path.display()
        );
    }
}

struct World {
    rig: Rig,
    main: String,
    peer: String,
    shim: Shim,
}

impl World {
    fn new(tag: &str) -> Self {
        let rig = Rig::new(&format!("prov-{tag}"));
        let [main, peer] = rig.create("s", A);
        let shim = Shim::at(&rig.scratch);
        Self {
            rig,
            main,
            peer,
            shim,
        }
    }

    fn run(&self, pane: Option<&str>, args: &[&str], fault: &Fault<'_>) -> Output {
        let mut cmd = self.rig.command(pane);
        cmd.args(args).env("AE_SEND_DEFER_SEC", "15");
        self.shim.arm(&mut cmd, fault);
        cmd.output().expect("faulted helper")
    }

    /// The same, with the two caller markers exactly as given.
    fn marked(
        &self,
        tmux: Option<&str>,
        pane: Option<&str>,
        args: &[&str],
        fault: &Fault<'_>,
    ) -> Output {
        let mut cmd = self.rig.command(None);
        cmd.args(args).env("AE_SEND_DEFER_SEC", "15");
        if let Some(tmux) = tmux {
            cmd.env("TMUX", tmux);
        }
        if let Some(pane) = pane {
            cmd.env("TMUX_PANE", pane);
        }
        self.shim.arm(&mut cmd, fault);
        cmd.output().expect("faulted helper")
    }

    fn override_run(&self, actor: &str, args: &[&str], fault: &Fault<'_>) -> Output {
        let mut cmd = self.rig.command(Some(&self.main));
        cmd.args(args)
            .env("AE_SEND_DEFER_SEC", "15")
            .env("AE_SENDER_OVERRIDE", actor);
        self.shim.arm(&mut cmd, fault);
        cmd.output().expect("faulted helper")
    }

    /// The `s` journal.
    fn events(&self) -> String {
        fs::read_to_string(self.rig.dir("s").join("events.jsonl")).unwrap_or_default()
    }

    fn count(&self, action: &str) -> usize {
        self.events()
            .matches(&format!("\"action\":\"{action}\""))
            .count()
    }

    /// The `action` record under `actor`, whole.
    fn record(&self, action: &str, actor: &str) -> String {
        let events = self.events();
        events
            .lines()
            .find(|line| {
                line.contains(&format!("\"action\":\"{action}\""))
                    && line.contains(&format!("\"actor\":\"{actor}\""))
            })
            .unwrap_or_else(|| panic!("no {action} by {actor}: {events}"))
            .to_owned()
    }

    fn state(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        state(&self.rig.dir("s"))
    }

    fn refused(&self, output: &Output, before: &BTreeMap<PathBuf, Vec<u8>>) {
        refused(output, &self.shim, before, &self.rig.dir("s"));
        assert!(self.rig.received(A, "colead").is_empty(), "a turn landed");
    }

    /// Exit 0, and exactly one turn in `colead`'s box, the one this case sent.
    fn delivered(&self, output: &Output) -> String {
        let err = stderr(output);
        assert_eq!(output.status.code(), Some(0), "not delivered: {err}");
        assert!(!err.contains("REFUSED"), "refused: {err}");
        let got = self.rig.received(A, "colead");
        assert_eq!(got.len(), 1, "exactly one submit: {got:?}");
        assert!(got[0].contains(BODY), "wrong turn: {got:?}");
        got[0].clone()
    }

    /// Two reads that fail, then one that would answer: only the bound refuses here.
    fn caller_fails_twice(&self) -> Fault<'_> {
        caller(&self.main, "fail fail", "real")
    }

    /// The whole table's case: UNREAD on `args`, from the main pane.
    fn unread(&self, args: &[&str]) {
        let before = self.state();
        let output = self.run(Some(&self.main), args, &self.caller_fails_twice());
        self.refused(&output, &before);
        assert_eq!(
            self.shim.reads(),
            ["read fail", "read fail"],
            "not the one re-ask: {args:?}"
        );
    }

    /// A fresh pane of `s` ae never stamped: tmux answers it, positively vacant.
    fn vacant(&self) -> String {
        self.rig
            .tmux(&[
                "split-window",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-t",
                "s:main",
                "sleep 600",
            ])
            .trim()
            .to_owned()
    }

    /// A spawned seat `helper`, as `retire` proves one: composer, stamps, roster rows.
    fn spawned(&self) -> String {
        let command = format!(
            "exec perl {} {}",
            self.rig.scratch.join("claude").display(),
            self.rig.receipt(A, "helper").display()
        );
        let out = self.rig.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "s:",
            "-n",
            "helper",
            &command,
        ]);
        let pane = out.trim().to_owned();
        self.rig
            .tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "spawned.1"]);
        self.rig
            .tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "helper"]);
        let meta = self.rig.dir("s").join("meta");
        let text = format!(
            "{}seat.spawned.1=helper\nprofile.spawned.1=fake\nagent_bin.spawned.1=claude\nharness_session.spawned.1=pending\nlaunch_id.spawned.1=tok-helper\npane.spawned.1={pane}\n",
            fs::read_to_string(&meta).expect("session meta")
        );
        fs::write(&meta, text).expect("seat rows");
        let deadline = Instant::now() + CEILING;
        while !self
            .rig
            .tmux(&["capture-pane", "-p", "-t", &pane])
            .contains("fake-model")
        {
            assert!(Instant::now() < deadline, "fixture composer did not start");
            std::thread::sleep(Duration::from_millis(25));
        }
        pane
    }

    /// A second session on the same server; its stamped `colead` pane.
    fn abroad(&self) -> String {
        let [_, peer] = self.rig.create("other", B);
        peer
    }
}

/// A session with a provably dead spawned seat `scout`: what `reseat` and `relaunch` move.
struct Dead {
    rig: SeatRig,
    shim: Shim,
}

impl Dead {
    fn new(tag: &str) -> Self {
        let rig = SeatRig::new(tag);
        rig.seat_rows("spawned.0", "scout", "grok", "grok");
        let seat = rig.new_pane("spawned.0", "scout");
        rig.start(&seat, "spawned.0", "grok");
        rig.kill_tools(&seat);
        let shim = Shim::at(&rig.scratch);
        Self { rig, shim }
    }

    /// The same seat ALIVE: its recorded tool running under the pane, its frame provably idle, so
    /// a `reseat` that gets past the caller reaches the stop and respawns the pane. The pane too.
    fn live(tag: &str) -> (Self, String) {
        let rig = SeatRig::new(tag);
        rig.seat_rows("spawned.0", "scout", "claude", "claude");
        let seat = rig.new_pane("spawned.0", "scout");
        rig.start(&seat, "spawned.0", "claude");
        let shim = Shim::at(&rig.scratch);
        (Self { rig, shim }, seat)
    }

    /// A pane of a second session, carrying a seat's stamps when `stamped`.
    fn elsewhere(&self, stamped: bool) -> String {
        let (ok, panes) = self.rig.tmux(&[
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-s",
            "elsewhere",
            "exec /bin/sh",
        ]);
        assert!(ok, "second session: {panes}");
        let pane = panes.trim().to_owned();
        if stamped {
            for (option, value) in [("@ae_agent", "lead"), ("@ae_slot", "main")] {
                assert!(
                    self.rig
                        .tmux(&["set-option", "-p", "-t", &pane, option, value])
                        .0
                );
            }
        }
        pane
    }

    fn run(&self, pane: Option<&str>, args: &[&str], fault: &Fault<'_>) -> Output {
        let mut cmd = ae();
        cmd.env("AE_HOME", &self.rig.scratch)
            .env_remove("AE_SENDER_OVERRIDE")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args(args);
        if let Some(pane) = pane {
            cmd.env("TMUX", format!("{},0,0", self.rig.sock.display()))
                .env("TMUX_PANE", pane);
        }
        self.shim.arm(&mut cmd, fault);
        cmd.output().expect("faulted core")
    }

    fn reseat(&self, pane: Option<&str>, fault: &Fault<'_>) -> Output {
        let args = [
            "reseat",
            &self.rig.session,
            "scout",
            "--using",
            "fake-opencode",
        ];
        self.run(pane, &args, fault)
    }

    fn moved(&self, output: &Output) {
        assert_eq!(output.status.code(), Some(0), "not moved: {output:?}");
        assert_eq!(self.rig.meta_row("profile.spawned.0"), "fake-opencode");
    }

    fn stayed(&self) {
        assert_eq!(self.rig.meta_row("profile.spawned.0"), "fake-grok");
        assert!(
            !self.rig.events().contains("\"action\":\"reseat\""),
            "{}",
            self.rig.events()
        );
    }

    /// UNREAD on `reseat` from `pane`: refused, then proved the read was the only reason.
    fn unread_reseat(&self, pane: &str) {
        let before = state(&self.rig.dir);
        let output = self.reseat(Some(pane), &caller(pane, "fail fail", "real"));
        refused(&output, &self.shim, &before, &self.rig.dir);
        assert_eq!(self.shim.reads(), ["read fail", "read fail"]);
        self.stayed();
    }
}

// ---- R2: every surface refuses, the third read never asked ---------------------------------

// Kills: `human` on a failed read, the issue's own case; a third read taken instead of a refusal.
#[test]
fn an_unread_caller_cannot_send() {
    World::new("send").unread(&["@s", "send", "colead", BODY]);
}

// Kills: `unidentified` reached on UNREAD: the warning and the drop to a plain send.
#[test]
fn an_unread_caller_cannot_ask() {
    World::new("ask").unread(&["@s", "ask", "colead", BODY]);
}

// Kills: a gate on `ask` alone: `review` is its own request kind.
#[test]
fn an_unread_caller_cannot_review() {
    World::new("review").unread(&["@s", "review", "colead", BODY]);
}

// Kills: a gate below the bare cancel, whose keys have no body to recover by.
#[test]
fn an_unread_caller_cannot_interrupt_bare() {
    World::new("int-bare").unread(&["@s", "interrupt", "colead"]);
}

// Kills: a gate on the message arm alone.
#[test]
fn an_unread_caller_cannot_interrupt_with_a_message() {
    World::new("int-msg").unread(&["@s", "interrupt", "colead", BODY]);
}

// Kills: a gate on the delivery helpers: a memo row keeps author `human` for good.
#[test]
fn an_unread_caller_cannot_add_a_memo() {
    World::new("memo").unread(&["@s", "memo", "add", "--topic", "t", BODY]);
}

// Kills: a goal setter journaled `human`, or spared by the session notice.
#[test]
fn an_unread_caller_cannot_set_a_goal() {
    World::new("goal").unread(&["@s", "goal", BODY]);
}

// Kills: a gate on the `Set` arm alone.
#[test]
fn an_unread_caller_cannot_clear_a_goal() {
    let w = World::new("goal-clear");
    let (code, out, err) = w.rig.helper("s", "goal", &["seeded"], Some(&w.main));
    assert_eq!(code, Some(0), "goal setup: {out}{err}");
    w.unread(&["@s", "goal", "--clear"]);
}

// Kills: a seat spawned by nobody ae can name, its spawner and its owner lost.
#[test]
fn an_unread_caller_cannot_spawn() {
    let w = World::new("spawn");
    w.unread(&["@s", "spawn", "scout", "--using", "fake", BODY]);
    assert!(!w.rig.tmux(&["list-windows", "-a"]).contains("scout"));
}

// Kills: a seat retired by nobody: the rows, the pane and the requests it closes are gone.
#[test]
fn an_unread_caller_cannot_retire() {
    let w = World::new("retire");
    let pane = w.spawned();
    w.unread(&["@s", "retire", "helper"]);
    assert!(
        w.rig
            .tmux(&["list-panes", "-a", "-F", "#{pane_id}"])
            .contains(&pane),
        "the seat's pane went"
    );
}

// Kills: a dead seat restarted by nobody. The seat is really dead, so no later refusal covers.
#[test]
fn an_unread_caller_cannot_relaunch() {
    let w = Dead::new("rela");
    let pane = w.rig.main_pane.clone();
    let before = state(&w.rig.dir);
    let dir = w.rig.dir.display().to_string();
    let args = ["_relaunch", dir.as_str(), "scout"];
    let output = w.run(Some(&pane), &args, &caller(&pane, "fail fail", "real"));
    refused(&output, &w.shim, &before, &w.rig.dir);
    assert_eq!(w.shim.reads(), ["read fail", "read fail"]);
}

// Kills: the owner read of a named pane failing into `human`, and a loop on it: the third read
// would answer, so only the bound refuses here. The core argv equals what routing builds for a
// bare `ae watchdog start s` typed in that pane, so this is that case too.
#[test]
fn an_unread_caller_naming_its_pane_cannot_touch_a_watchdog() {
    for verb in ["start", "stop"] {
        let w = World::new(&format!("wd-pane-{verb}"));
        let main = w.main.clone();
        w.unread(&["watchdog", verb, "s", "--pane", &main]);
    }
}

// Kills: a refusal placed after the kill: the daemon outlives a caller nobody can name.
#[test]
fn an_unread_caller_naming_its_pane_leaves_a_running_watchdog_running() {
    let w = World::new("wd-live");
    let main = w.main.clone();
    let started = w.rig.run(&["watchdog", "start", "s"], Some(&main));
    assert_eq!(started.0, Some(0), "watchdog setup: {started:?}");
    // The daemon churns its own quota history and beat, so this compares the two records named.
    let records = || ["events.jsonl", "meta"].map(|name| fs::read(w.rig.dir("s").join(name)));
    let before = records().map(|read| read.expect("session record"));
    let args = ["watchdog", "stop", "s", "--pane", &main];
    let output = w.run(Some(&main), &args, &w.caller_fails_twice());
    family(&output);
    assert_eq!(w.shim.reads(), ["read fail", "read fail"]);
    assert_eq!(w.shim.wire(), Vec::<String>::new(), "a pane was touched");
    assert_eq!(w.count("watchdog-stop"), 0, "{}", w.events());
    let after = records().map(|read| read.expect("session record"));
    assert_eq!(after, before, "the refusal wrote a session record");
    let status = w.rig.run(&["watchdog", "status", "s"], None);
    assert!(status.1.contains("Watchdog is running"), "{status:?}");
}

// Kills: no re-ask on the owner read: one failure journals `human` for a seat that named its pane.
#[test]
fn a_watchdog_owner_read_failing_once_keeps_the_true_actor() {
    let w = World::new("wd-once");
    let main = w.main.clone();
    let args = ["watchdog", "stop", "s", "--pane", &main];
    let output = w.run(Some(&main), &args, &caller(&main, "fail", "real"));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(w.shim.reads(), ["read fail", "read real"]);
    w.record("watchdog-stop", "lead");
    assert!(
        !w.events().contains("\"actor\":\"human\""),
        "{}",
        w.events()
    );
}

// Pins base. Kills: a re-ask of a vacant owner answer, or vacant read as UNREAD: a pane ae never
// stamped named by `--pane` stays `human` on its one read.
#[test]
fn a_vacant_named_pane_keeps_its_watchdog_verb() {
    let w = World::new("wd-vacant");
    let pane = w.vacant();
    let args = ["watchdog", "stop", "s", "--pane", &pane];
    let output = w.run(Some(&pane), &args, &caller(&pane, "", "real"));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(w.shim.reads(), ["read real"]);
    w.record("watchdog-stop", "human");
}

// Pins base, and the residual ruled out of scope. Routing appends `--pane` from `$TMUX_PANE`
// (src/entry.rs), so this is what a true no-`--pane` caller looks like: `TMUX` alone, a tmux
// binding. No read happens, so none failed, and both verbs keep `human`. Kills: a gate on
// `caller_server` alone; a new caller read at this surface.
#[test]
fn a_watchdog_verb_with_no_pane_reads_no_caller() {
    let w = World::new("wd-nopane");
    let socket = format!("{},0,0", w.rig.socket.display());
    for (verb, action) in [("start", "watchdog-start"), ("stop", "watchdog-stop")] {
        let args = ["watchdog", verb, "s"];
        let output = w.marked(Some(&socket), None, &args, &caller(&w.main, "", "fail"));
        assert_eq!(output.status.code(), Some(0), "{verb}: {output:?}");
        assert_eq!(w.shim.reads(), Vec::<String>::new(), "{verb}");
        w.record(action, "human");
    }
}

// Kills: a failed read as the unstamped-shell branch, which may reseat any session (R4).
#[test]
fn an_unread_stamped_pane_of_another_session_cannot_reseat() {
    let w = Dead::new("rsab");
    let pane = w.elsewhere(true);
    w.unread_reseat(&pane);
}

// Kills: a refusal only for a pane provably abroad: an unread pane proves nothing either way.
#[test]
fn an_unread_pane_of_the_same_session_cannot_reseat() {
    let w = Dead::new("rsown");
    let pane = w.rig.main_pane.clone();
    w.unread_reseat(&pane);
}

// Kills: a gate placed after `stop_running_tool`. A dead seat cannot see that order: nothing is
// there to stop. Here the tool is proven running and idle, so a late gate has already killed it.
#[test]
fn an_unread_caller_stops_no_running_seat() {
    for tag in ["abroad", "home"] {
        let (w, seat) = Dead::live(&format!("rslive{tag}"));
        let pane = if tag == "abroad" {
            w.elsewhere(true)
        } else {
            w.rig.main_pane.clone()
        };
        let tool = w.rig.tool_pid(&seat, "claude");
        assert!(tool.is_some(), "fixture: the seat's tool runs");
        let before = state(&w.rig.dir);
        let output = w.reseat(Some(&pane), &caller(&pane, "fail fail", "real"));
        assert_eq!(
            w.rig.tool_pid(&seat, "claude"),
            tool,
            "the tool was stopped"
        );
        refused(&output, &w.shim, &before, &w.rig.dir);
        assert_eq!(w.shim.reads(), ["read fail", "read fail"]);
        assert_eq!(w.rig.meta_row("profile.spawned.0"), "fake-claude");
        assert!(!w.rig.events().contains("stopped"), "{}", w.rig.events());
    }
}

// Pins base. The control of that fixture: from a shell the same `reseat` reaches the stop and
// respawns the pane. Kills: a fixture whose tool `stop_running_tool` would never stop, which
// would make the case above pass for no reason.
#[test]
fn a_shell_reaches_the_stop_of_a_running_seat() {
    let (w, seat) = Dead::live("rslivectl");
    let output = w.reseat(None, &caller("%never", "", "fail"));
    w.moved(&output);
    assert!(w.shim.wire().contains(&"wire respawn-pane".to_owned()));
    assert!(w.rig.tool_pid(&seat, "claude").is_none(), "tool still runs");
    assert!(w.rig.events().contains("stopped claude in place"));
}

// Kills: the issue's own journal: a read failing every time, delivered under `human`.
#[test]
fn a_caller_read_failing_every_time_writes_no_human() {
    for (tag, args) in [
        ("always-send", vec!["@s", "send", "colead", BODY]),
        ("always-ask", vec!["@s", "ask", "colead", BODY]),
        ("always-memo", vec!["@s", "memo", "add", BODY]),
    ] {
        let w = World::new(tag);
        let before = w.state();
        let output = w.run(Some(&w.main), &args, &caller(&w.main, "", "fail"));
        w.refused(&output, &before);
        assert!(!w.events().contains("\"actor\":\"human\""), "{args:?}");
    }
}

// ---- recovery and final answers ------------------------------------------------------------

// Pins base. Kills: a refusal on any failed first read: the re-ask exists so this one lands.
#[test]
fn a_caller_read_failing_once_keeps_the_true_actor() {
    let w = World::new("once-send");
    let args = ["@s", "send", "colead", BODY];
    let output = w.run(Some(&w.main), &args, &caller(&w.main, "fail", "real"));
    let got = w.delivered(&output);
    assert_eq!(w.shim.reads()[..2], ["read fail", "read real"]);
    assert!(got.contains("from lead") && !got.contains("unverified"));
    w.record("send", "lead");
    let w = World::new("once-memo");
    let args = ["@s", "memo", "add", BODY];
    let output = w.run(Some(&w.main), &args, &caller(&w.main, "fail", "real"));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    w.record("memo", "lead");
    assert!(!w.events().contains("\"actor\":\"human\""));
}

// Pins base. Kills: a first read that failed remembered as UNREAD whatever the re-ask answered.
#[test]
fn a_vacant_answer_after_a_failed_read_is_still_vacant() {
    let w = World::new("once-vacant");
    let pane = w.vacant();
    let args = ["@s", "send", "colead", BODY];
    let output = w.run(Some(&pane), &args, &caller(&pane, "fail", "real"));
    w.delivered(&output);
    assert_eq!(w.shim.reads()[..2], ["read fail", "read real"]);
    w.record("send", "human");
}

// Pins base. Kills: a positive answer turned UNREAD by a later failure of the same read (R6). The
// count is `goal` below: `memo` and `send` also read this pane for correlation, `goal` does not.
#[test]
fn a_positive_first_answer_is_final_whatever_a_later_read_does() {
    let w = World::new("final");
    let vacant = w.vacant();
    for (tag, pane, actor) in [("stamped", &w.main, "lead"), ("vacant", &vacant, "human")] {
        let args = ["@s", "memo", "add", "--topic", tag, BODY];
        let output = w.run(Some(pane), &args, &caller(pane, "real", "fail"));
        assert_eq!(output.status.code(), Some(0), "{tag}: {}", stderr(&output));
        assert_eq!(w.shim.reads()[0], "read real", "{tag}");
        assert_eq!(w.count("memo"), usize::from(tag == "vacant") + 1, "{tag}");
        w.record("memo", actor);
    }
    let args = ["@s", "send", "colead", BODY];
    let output = w.run(Some(&w.main), &args, &caller(&w.main, "real", "fail"));
    w.delivered(&output);
    w.record("send", "lead");
}

// Pins base. Kills: a re-ask of a positive answer, a loop on success, or a second proof per
// surface: `goal` takes one caller observation, so one read is the whole of it, stamped or vacant.
#[test]
fn a_positive_answer_is_read_exactly_once() {
    let w = World::new("once-read");
    let vacant = w.vacant();
    for (tag, pane, actor) in [("stamped", &w.main, "lead"), ("vacant", &vacant, "human")] {
        let args = ["@s", "goal", tag];
        let output = w.run(Some(pane), &args, &caller(pane, "", "real"));
        assert_eq!(output.status.code(), Some(0), "{tag}: {}", stderr(&output));
        assert_eq!(w.shim.reads(), ["read real"], "{tag}");
        w.record("goal", actor);
    }
}

// Pins base. Kills: a gate above the override: a bridge reads no pane, so none can fail it.
#[test]
fn a_sender_override_is_unaffected_by_a_failing_read() {
    let w = World::new("override-send");
    let args = ["@s", "send", "colead", BODY];
    let output = w.override_run("watchdog", &args, &caller(&w.main, "", "fail"));
    w.delivered(&output);
    w.record("send", "watchdog");
    let w = World::new("override-ask");
    let args = ["@s", "ask", "colead", BODY];
    let output = w.override_run("telegram:human", &args, &caller(&w.main, "", "fail"));
    w.delivered(&output);
    w.record("ask", "telegram:human");
}

// Pins base. Kills: an override taken as a reason to skip the caller read: `send` reads its caller
// for the session too, so a pane abroad would lose `--cross-session` and journal the wrong session.
#[test]
fn a_sender_override_keeps_the_cross_session_gate() {
    let w = World::new("override-abroad");
    let abroad = w.abroad();
    let mut cmd = w.rig.command(Some(&abroad));
    cmd.args(["@s", "send", "colead", BODY])
        .env("AE_SEND_DEFER_SEC", "15")
        .env("AE_SENDER_OVERRIDE", "bridge");
    w.shim.arm(&mut cmd, &caller(&abroad, "", "real"));
    let output = cmd.output().expect("faulted helper");
    assert_eq!(output.status.code(), Some(1), "delivered: {output:?}");
    assert!(stderr(&output).contains(CROSSING), "{output:?}");
    assert_eq!(w.shim.wire(), Vec::<String>::new(), "a turn crossed");
    assert!(w.rig.received(A, "colead").is_empty(), "a turn crossed");
}

// Pins base. Kills: a whole-verb gate: these three read no caller, and refuse nothing.
#[test]
fn a_verb_that_writes_nothing_ignores_an_unread_caller() {
    let w = World::new("readers");
    for args in [
        vec!["@s", "memo", "read"],
        vec!["@s", "goal"],
        vec!["watchdog", "status", "s"],
    ] {
        let output = w.run(Some(&w.main), &args, &caller(&w.main, "", "fail"));
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert!(!stderr(&output).contains(FAMILY), "{args:?}: {output:?}");
    }
}

// ---- READ, vacant ----------------------------------------------------------------------------

// Pins base. Kills: a vacant viewer refused like a failed one: a human's own tmux keeps working.
#[test]
fn a_vacant_pane_keeps_sending_as_human() {
    let w = World::new("vacant-send");
    let pane = w.vacant();
    let args = ["@s", "send", "colead", BODY];
    let output = w.run(Some(&pane), &args, &caller(&pane, "", "real"));
    let got = w.delivered(&output);
    assert!(got.contains("unverified"), "{got:?}");
    w.record("send", "human");
}

// Pins base. Kills: the memo gate reading `!is_known()` rather than the failed read.
#[test]
fn a_vacant_pane_keeps_its_memo_as_human() {
    let w = World::new("vacant-memo");
    let pane = w.vacant();
    let args = ["@s", "memo", "add", BODY];
    let output = w.run(Some(&pane), &args, &caller(&pane, "", "real"));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    w.record("memo", "human");
    let memos = fs::read_to_string(w.rig.dir("s").join("memo.tsv")).expect("memo file");
    assert!(memos.contains("\thuman\t"), "{memos}");
}

/// A vacant pane's tracked verb: the exact warning, then a plain send, and no request.
fn keeps_the_fallback(tag: &str, verb: &str) {
    let w = World::new(tag);
    let pane = w.vacant();
    let args = ["@s", verb, "colead", BODY];
    let output = w.run(Some(&pane), &args, &caller(&pane, "", "real"));
    assert_eq!(stderr(&output), FALLBACK);
    w.delivered(&output);
    assert_eq!(w.count(verb), 0, "a request from nobody: {}", w.events());
    w.record("send", "human");
}

// Pins base. Kills: the `ask` gate reading `!is_known()`: vacant keeps its warning and fallback.
#[test]
fn a_vacant_pane_keeps_the_ask_fallback() {
    keeps_the_fallback("vacant-ask", "ask");
}

// Pins base. Kills: a vacant refusal on `review` alone: the arm is shared, the verb is not.
#[test]
fn a_vacant_pane_keeps_the_review_fallback() {
    keeps_the_fallback("vacant-review", "review");
}

// ---- OUTSIDE ---------------------------------------------------------------------------------

// Pins base, and the named residual of R1. Kills: `tracked::classify_caller` as the gate: it calls
// a partial pair and a malformed marker unreadable, and those are ae's own key bindings, the chat's
// `!` popup and a shell outside tmux. The armed read proves none of them reads a pane.
#[test]
fn a_caller_with_no_readable_pair_keeps_acting_as_human() {
    let w = World::new("outside");
    let socket = format!("{},0,0", w.rig.socket.display());
    let main = w.main.clone();
    for (tmux, pane) in [
        (None, None),
        (Some(socket.as_str()), None),
        (None, Some(main.as_str())),
        (Some("relative/socket,0,0"), Some(main.as_str())),
        (Some("not-a-marker"), Some(main.as_str())),
    ] {
        let sends = w.count("send");
        let turns = w.rig.received(A, "colead").len();
        let args = ["@s", "send", "colead", BODY];
        let output = w.marked(tmux, pane, &args, &caller(&main, "", "fail"));
        assert_eq!(
            output.status.code(),
            Some(0),
            "{tmux:?} {pane:?}: {output:?}"
        );
        assert_eq!(w.count("send"), sends + 1, "{tmux:?} {pane:?}");
        assert_eq!(w.rig.received(A, "colead").len(), turns + 1);
        assert_eq!(w.shim.reads(), Vec::<String>::new(), "{tmux:?} {pane:?}");
        w.record("send", "human");
        let memos = w.count("memo");
        let args = ["@s", "memo", "add", BODY];
        let output = w.marked(tmux, pane, &args, &caller(&main, "", "fail"));
        assert_eq!(
            output.status.code(),
            Some(0),
            "{tmux:?} {pane:?}: {output:?}"
        );
        assert_eq!(w.count("memo"), memos + 1);
    }
    assert!(!w.events().contains("\"actor\":\"lead\""), "{}", w.events());
}

// Pins base. Kills: a gate on `--pane` alone: with no caller server no read happens.
#[test]
fn a_shell_keeps_its_watchdog_verbs() {
    let w = World::new("outside-wd");
    let main = w.main.clone();
    for args in [
        vec!["watchdog", "stop", "s"],
        vec!["watchdog", "stop", "s", "--pane", main.as_str()],
    ] {
        let output = w.run(None, &args, &caller(&main, "", "fail"));
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert_eq!(w.shim.reads(), Vec::<String>::new(), "{args:?}");
        w.record("watchdog-stop", "human");
    }
}

// ---- R4: reseat -----------------------------------------------------------------------------

// Pins base. Kills: a refusal whose rule replaced the one it guards: a positive answer from a
// stamped pane abroad still reads the own-session rule, first read or re-ask, and moves nothing.
#[test]
fn a_stamped_pane_of_another_session_keeps_the_own_session_refusal() {
    let w = Dead::new("rsrule");
    let pane = w.elsewhere(true);
    for plan in ["", "fail"] {
        let before = state(&w.rig.dir);
        let output = w.reseat(Some(&pane), &caller(&pane, plan, "real"));
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(stderr(&output).contains(OTHER_SEAT), "{output:?}");
        assert_eq!(state(&w.rig.dir), before, "plan {plan:?}");
        w.stayed();
    }
}

// Pins base. Kills: reseat refused from a plain shell, the moment it exists for: a lead's quota
// dead and nothing of that session able to run anything.
#[test]
fn a_shell_outside_tmux_keeps_moving_a_seat() {
    let w = Dead::new("rsout");
    let output = w.reseat(None, &caller("%never", "", "fail"));
    w.moved(&output);
    assert_eq!(w.shim.reads(), Vec::<String>::new());
}

// Pins base. Kills: vacant read as UNREAD on `reseat`, which reads `viewer.slot` alone.
#[test]
fn an_unstamped_tmux_pane_keeps_moving_a_seat() {
    let w = Dead::new("rsvac");
    let pane = w.elsewhere(false);
    let output = w.reseat(Some(&pane), &caller(&pane, "", "real"));
    w.moved(&output);
}

// ---- R5: the resolved target's read ------------------------------------------------------------

// Kills: no re-ask at the resolved read: one failure leaves `Resolved.session` empty, past the
// gate, and the caller is never told the target is another session.
#[test]
fn a_named_target_read_failing_once_still_needs_the_flag() {
    let w = World::new("r5-once");
    let peer = w.abroad();
    let args = ["@s", "send", "@other:colead", BODY];
    let output = w.run(Some(&w.main), &args, &aimed(&peer, "fail", "real"));
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).contains(CROSSING), "{output:?}");
    assert_eq!(w.shim.reads()[..2], ["read fail", "read real"]);
    assert_eq!(w.shim.wire(), Vec::<String>::new());
    assert!(w.rig.received(B, "colead").is_empty(), "a turn crossed");
}

// Kills: the resolved read refusing on its first failure: the re-ask exists so this one lands.
#[test]
fn a_same_session_target_read_failing_once_delivers() {
    let w = World::new("r5-home");
    let peer = w.peer.clone();
    let args = ["@s", "send", "colead", BODY];
    let output = w.run(Some(&w.main), &args, &aimed(&peer, "fail", "real"));
    w.delivered(&output);
    assert_eq!(w.shim.reads()[..2], ["read fail", "read real"]);
    w.record("send", "lead");
}

// Kills: an empty slot routed on: the re-asked answer must reach the request, not only the gate.
#[test]
fn a_target_read_failing_once_keeps_the_routing_of_a_request() {
    let w = World::new("r5-route");
    let peer = w.abroad();
    let args = ["@s", "ask", "--cross-session", "@other:colead", BODY];
    let output = w.run(Some(&w.main), &args, &aimed(&peer, "fail", "real"));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let line = w.record("ask", "lead");
    assert!(
        line.contains("\"target_slot\":\"worker.0\"")
            && line.contains("\"target_session\":\"other\""),
        "{line}"
    );
    assert_eq!(w.rig.received(B, "colead").len(), 1);
}

/// A named target's two failed reads refuse at resolve, in words of their own: the roster proved
/// the pane exists, so the admission's empty-session words would describe the wrong failure.
fn names_the_target(output: &Output) {
    let err = stderr(output);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_ne!(err, RAW_GAP, "fell through to the raw words");
    assert!(!err.contains("invalid participant session"), "{err}");
    assert!(err.contains("colead"), "the target is not named: {err}");
}

// Kills: a resolved read with no bound, and the re-ask falling through to the raw words.
fn named_target_fails_twice(tag: &str, args: &[&str]) {
    let w = World::new(tag);
    let peer = w.peer.clone();
    let output = w.run(Some(&w.main), args, &aimed(&peer, "fail fail", "real"));
    names_the_target(&output);
    assert_eq!(w.shim.reads(), ["read fail", "read fail"], "{args:?}");
    assert_eq!(w.shim.wire(), Vec::<String>::new(), "{args:?}");
    assert!(w.rig.received(A, "colead").is_empty(), "{args:?}");
    assert!(
        !w.events().contains("\"actor\":\"human\""),
        "{}",
        w.events()
    );
    for action in ["send", "ask", "interrupt"] {
        assert_eq!(w.count(action), 0, "{action}: {}", w.events());
    }
}

#[test]
fn a_named_target_read_failing_twice_refuses_a_send() {
    named_target_fails_twice("r5-send", &["@s", "send", "colead", BODY]);
}

#[test]
fn a_named_target_read_failing_twice_refuses_an_ask() {
    named_target_fails_twice("r5-ask", &["@s", "ask", "colead", BODY]);
}

#[test]
fn a_named_target_read_failing_twice_refuses_a_bare_interrupt() {
    named_target_fails_twice("r5-int", &["@s", "interrupt", "colead"]);
}

#[test]
fn a_named_target_read_failing_twice_refuses_an_interrupt_message() {
    named_target_fails_twice("r5-intm", &["@s", "interrupt", "colead", BODY]);
}

// Kills: the same read refusing at home but passing abroad.
#[test]
fn a_named_target_read_failing_twice_abroad_types_nothing() {
    let w = World::new("r5-twice-far");
    let peer = w.abroad();
    let args = ["@s", "send", "--cross-session", "@other:colead", BODY];
    let output = w.run(Some(&w.main), &args, &aimed(&peer, "fail fail", "real"));
    names_the_target(&output);
    assert_eq!(w.shim.reads(), ["read fail", "read fail"]);
    assert_eq!(w.shim.wire(), Vec::<String>::new());
    assert!(w.rig.received(B, "colead").is_empty(), "a turn crossed");
}

// Kills: a re-ask of the named branch alone: a raw pane reaches the same read. R5 lets the display
// read above it stay, so a raw target spends one read more or none: the plans hold for both.
#[test]
fn a_raw_target_read_failing_once_still_needs_the_flag() {
    for (tag, plan) in [("fail-first", "fail"), ("fail-second", "real fail")] {
        let w = World::new(&format!("r5-raw-{tag}"));
        let peer = w.abroad();
        let args = ["@s", "send", peer.as_str(), BODY];
        let output = w.run(Some(&w.main), &args, &aimed(&peer, plan, "real"));
        assert_eq!(output.status.code(), Some(1), "{plan}: {output:?}");
        assert!(stderr(&output).contains(CROSSING), "{plan}: {output:?}");
        assert_eq!(w.shim.wire(), Vec::<String>::new(), "{plan}");
        assert!(w.rig.received(B, "colead").is_empty(), "{plan}");
    }
}

// Pins base. Kills: a refusal at resolve for a raw pane, or a loop: a failed re-ask falls through
// to the admission's existing words, which ask no read of their own, so the next read would pass.
// Two reads when the display read is merged into the proof, three when it stays.
#[test]
fn a_raw_target_read_failing_every_time_types_nothing() {
    let w = World::new("r5-raw-twice");
    let peer = w.abroad();
    let args = ["@s", "send", "--cross-session", peer.as_str(), BODY];
    let output = w.run(
        Some(&w.main),
        &args,
        &aimed(&peer, "fail fail fail", "real"),
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(stderr(&output), RAW_GAP);
    let reads = w.shim.reads();
    assert!((2..=3).contains(&reads.len()), "{reads:?}");
    assert!(reads.iter().all(|read| read == "read fail"), "{reads:?}");
    assert_eq!(w.shim.wire(), Vec::<String>::new());
    assert!(w.rig.received(B, "colead").is_empty(), "a turn crossed");
}

// Pins base. Kills: a gone pane's words moved: a raw target has no roster to prove it existed, so
// its failed read is no new refusal.
#[test]
fn a_gone_raw_pane_keeps_its_words() {
    let w = World::new("r5-gone");
    for args in [
        vec!["@s", "send", "%999", BODY],
        vec!["@s", "ask", "%999", BODY],
        vec!["@s", "interrupt", "%999"],
        vec!["@s", "interrupt", "%999", BODY],
    ] {
        let output = w.run(Some(&w.main), &args, &aimed("%999", "", "real"));
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert_eq!(stderr(&output), RAW_GAP, "{args:?}");
        assert_eq!(w.shim.wire(), Vec::<String>::new(), "{args:?}");
    }
    assert!(
        !w.events().contains("\"actor\":\"human\""),
        "{}",
        w.events()
    );
}

// Pins base. Kills: a vacant target refused like a failed one: a raw pane ae never stamped.
#[test]
fn a_positive_vacant_raw_pane_still_takes_a_turn() {
    let w = World::new("r5-vacant");
    for key in ["@ae_slot", "@ae_agent"] {
        w.rig.tmux(&["set-option", "-p", "-u", "-t", &w.peer, key]);
    }
    let args = ["@s", "send", w.peer.as_str(), BODY];
    let output = w.run(Some(&w.main), &args, &aimed(&w.peer, "", "real"));
    w.delivered(&output);
    w.record("send", "lead");
}
