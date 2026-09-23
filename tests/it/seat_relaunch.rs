//! `relaunch`: bringing ONE provably dead seat back, against a REAL tmux server.
//!
//! The whole operation runs. A seat's pane is stamped and its tool started by
//! pasting the very `_run` line a launch would paste; the tool is a fake that
//! exits on demand, so a seat can be made DEAD the way the field case was —
//! tool gone, pane sitting at its shell.
//!
//! THE FAKE IS A COPY OF THE PERL INTERPRETER, named after the tool it stands
//! in for, because the identity proof is about the process NAME. Measured
//! 2026-09-18: a SYMLINK to the interpreter reports `perl` to
//! `pane_current_command` on macOS (and to both readers on Linux), which would
//! move the proof onto whichever arm happened to fire on the developer's
//! machine. A copy reports the invoked name to `ps` on both platforms and to
//! tmux as well, so both arms of the identity check see the seat's tool.
//! Consequence, stated because it is a real limit of these fixtures: the
//! end-to-end pins do not isolate which arm fired. The arms are pinned apart
//! in `seat_relaunch`'s own unit tests.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::cli::ae;
use super::phase2::run_tmux;

/// The fake agent: it records the argv and working directory it was launched
/// with, logs everything the pane sends it, and EXITS on demand.
///
/// `__COMPOSED__` decides whether it draws an input box a delivery can prove
/// ready. A fake that never draws one is how the undelivered launch turn is
/// pinned without faking a submit.
///
/// `__FRAME__` decides whether it draws claude's MEASURED harness frame — the
/// one `harness_state::classify` reads — so a running seat can be presented to
/// `reseat`'s stop as provably IDLE, or, when the `__BUSY__` file appears, as
/// provably mid-turn. Every other fake draws a frame that grammar does not
/// own, which is how the UNKNOWN arm is pinned.
const FAKE: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
my $exit = "__EXIT__";
# A real TUI restores the terminal on its way out. A fake that does not leaves
# the pane's tty in raw mode, where C-c is a byte rather than a signal and the
# shell the seat falls back to is not the shell a human would find.
sub bye { system("stty sane 2>/dev/null"); exit 0; }
exit 0 if -e $exit;
open(my $log, '>>', "__LAUNCHED__") or die;
print $log join(" ", @ARGV), "\n";
print $log "cwd=" . `pwd`;
close($log);
system("stty raw -echo 2>/dev/null");
binmode(STDIN, ':raw');
binmode(STDOUT, ':raw');
$| = 1;
my $composed = __COMPOSED__;
my $frame = __FRAME__;
my $busyfile = "__BUSY__";
my $rail = "\xe2\x94\x83";
my $corner = "\xe2\x95\xb9";
my $block = "\xe2\x96\x80";
my $ellipsis = "\xe2\x80\xa6";
my $dot = "\xc2\xb7";
my $bar = "\xe2\x94\x80" x 60;
my $caret = "\xe2\x9d\xaf";
my $brain = "\xf0\x9f\xa7\xa0";
my $chev = "\xe2\x8f\xb5";
my $spin = "\xe2\x9c\xb3";
my $done = "\xe2\x9c\xbb";
sub busy_now { return (-e $busyfile) ? 1 : 0; }
sub draw {
    my $busy = shift;
    print "\e[H\e[2J";
    if ($frame) {
        # The row directly above the box is what says whether a turn is
        # running: claude draws its spinner there while it works and a
        # `done` summary when it is finished, and the classifier reads the
        # last such row in the pane's recent history.
        print $busy
            ? "$spin Thinking$ellipsis (3s $dot esc to interrupt)\r\n"
            : "$done Fake $dot done (0s)\r\n";
        print "$bar\r\n";
        print "$caret \r\n";
        print "$bar\r\n";
        print "$brain Opus 5 $dot fake\r\n";
        print "$chev$chev accept edits on\r\n";
    } elsif ($composed) {
        print "opencode\r\n";
        print "$rail\r\n";
        print "$rail  Ask anything$ellipsis \"Fix broken tests\"\r\n";
        print "$rail\r\n";
        print "$rail  Build $dot fake model\r\n";
        print "$corner", ($block x 60), "\r\n";
        print "tab agents  ctrl+p commands\r\n";
    } else {
        print "fake agent is starting\r\n";
    }
}
my $shown = busy_now();
draw($shown);
while (1) {
    bye() if -e $exit;
    if ($frame) {
        my $now = busy_now();
        if ($now != $shown) { draw($now); $shown = $now; }
    }
    my $ready = '';
    vec($ready, fileno(STDIN), 1) = 1;
    if (select($ready, undef, undef, 0.05) > 0) {
        my $chunk = '';
        sysread(STDIN, $chunk, 4096);
        open(my $fh, '>>', "__RECEIVED__") or die;
        print $fh $chunk;
        close($fh);
    }
}
"#;

/// One interpreter COPY per tool class, each named for the tool it stands in
/// for, and the `[profiles]` block that points at them.
fn fakes(scratch: &Path, tools: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let mut profiles = String::from("[profiles]\n");
    for (tool, composed, frame) in [
        ("opencode", "1", "0"),
        ("grok", "0", "0"),
        ("codex", "0", "0"),
        ("agy", "0", "0"),
        // The ONE fake that draws a frame `harness_state::classify` owns.
        ("claude", "0", "1"),
    ] {
        let bin = tools.join(tool);
        assert!(
            std::fs::copy("/usr/bin/perl", &bin).is_ok(),
            "the interpreter copies"
        );
        assert!(
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable fake"
        );
        let script = scratch.join(format!("{tool}.pl"));
        let body = FAKE
            .replace("__EXIT__", &scratch.join("__EXIT__").display().to_string())
            .replace(
                "__LAUNCHED__",
                &scratch.join("launched").display().to_string(),
            )
            .replace(
                "__RECEIVED__",
                &scratch.join("received").display().to_string(),
            )
            .replace("__COMPOSED__", composed)
            .replace("__FRAME__", frame)
            .replace("__BUSY__", &scratch.join("__BUSY__").display().to_string());
        assert!(std::fs::write(&script, body).is_ok(), "the fake body");
        let _ = writeln!(
            profiles,
            "fake-{tool} = \"{} {}\"",
            bin.display(),
            script.display()
        );
    }
    // TWO ACCOUNTS OF ONE TOOL. The same fake, the same script, and nothing
    // different but the config home each names — the shape `reseat`'s account
    // carry exists for, and one a profile can only express as a leading
    // assignment on an otherwise identical command.
    for account in ["a", "b"] {
        let _ = writeln!(
            profiles,
            "fake-claude-{account} = \"CLAUDE_CONFIG_DIR={} {} {}\"",
            scratch.join(format!("home-{account}")).display(),
            tools.join("claude").display(),
            scratch.join("claude.pl").display()
        );
    }
    profiles
}

/// One isolated server with a live session, a v2 meta, and a `[profiles]` that
/// names a fake for each tool class the pins need.
pub struct Rig {
    pub scratch: PathBuf,
    pub sock: PathBuf,
    pub dir: PathBuf,
    pub session: String,
    pub main_pane: String,
}

impl Rig {
    pub fn new(tag: &str) -> Self {
        let scratch = super::cli::OwnedScratch::root("rl", tag).keep();
        let tools = scratch.join("tools");
        assert!(std::fs::create_dir_all(&tools).is_ok(), "a tools dir");
        let session = format!("rl{tag}");
        let dir = scratch.join("sessions").join(&session);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let work = scratch.join("work");
        assert!(std::fs::create_dir_all(&work).is_ok(), "a working copy");

        let profiles = fakes(&scratch, &tools);
        let config = scratch.join("config");
        assert!(
            std::fs::write(
                &config,
                format!("{profiles}\n[workspace]\nmain = lead\nlayout = vertical\n"),
            )
            .is_ok(),
            "a config"
        );
        let sock = scratch.join("sock");
        let rig = Self {
            scratch: scratch.clone(),
            sock,
            dir,
            session: session.clone(),
            main_pane: String::new(),
        };
        assert!(
            rig.tmux(&[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-x",
                "200",
                "-y",
                "40",
                "-s",
                &session,
                // EXPLICIT: the pane shell must not be inherited from $SHELL,
                // or a pin that turns on the recorded tool being read as a
                // shell would depend on the developer's own login shell.
                "/bin/sh",
            ])
            .0,
            "the session starts"
        );
        let (_, panes) = rig.tmux(&["list-panes", "-s", "-t", &session, "-F", "#{pane_id}"]);
        let main_pane = panes.lines().next().unwrap_or_default().to_owned();
        assert!(!main_pane.is_empty(), "{panes}");
        for (option, value) in [("@ae_agent", "lead"), ("@ae_slot", "main")] {
            assert!(
                rig.tmux(&["set-option", "-p", "-t", &main_pane, option, value])
                    .0
            );
        }
        assert!(
            std::fs::write(
                rig.dir.join("meta"),
                format!(
                    "session={session}\nwork_dir={}\norigin={}\nmode=local\nlayout=vertical\nconfig={}\nmain_pane={main_pane}\ntmux_server_kind=socket\ntmux_server={}\nschema=2\nseat.main=lead\nprofile.main=fake-opencode\nagent_bin.main=opencode\n",
                    work.display(),
                    scratch.display(),
                    config.display(),
                    rig.sock.display(),
                ),
            )
            .is_ok(),
            "a v2 meta"
        );
        let mut rig = rig;
        rig.main_pane = main_pane;
        rig
    }

    pub fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.sock.clone()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    /// Run one core subcommand as the LEAD's pane — the caller every pin uses.
    pub fn run(&self, sub: &str, tail: &[&str]) -> (Option<i32>, String, String) {
        self.run_from(&self.main_pane.clone(), sub, tail)
    }

    fn run_from(&self, pane: &str, sub: &str, tail: &[&str]) -> (Option<i32>, String, String) {
        let out = ae()
            .env("TMUX", format!("{},0,0", self.sock.display()))
            .env("TMUX_PANE", pane)
            .env("AE_HOME", &self.scratch)
            .env_remove("AE_SENDER_OVERRIDE")
            .arg(sub)
            .arg(&self.dir)
            .args(tail)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Run one TOP-LEVEL command — argv exactly as a human types it, with no
    /// session directory operand. `reseat` takes a session NAME and reads the
    /// durable world, so it is entered here rather than through the helper
    /// form above.
    pub fn run_top(&self, pane: &str, args: &[&str]) -> (Option<i32>, String, String) {
        let out = ae()
            .env("TMUX", format!("{},0,0", self.sock.display()))
            .env("TMUX_PANE", pane)
            .env("AE_HOME", &self.scratch)
            .env_remove("AE_SENDER_OVERRIDE")
            .args(args)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Add a seat: its meta rows, a stamped pane, and its tool STARTED the way
    /// a launch starts one — by pasting the seat's own `_run` line.
    pub fn seat(&self, slot: &str, name: &str, tool: &str) -> String {
        self.seat_rows(slot, name, tool, tool);
        let pane = self.new_pane(slot, name);
        self.start(&pane, slot, tool);
        pane
    }

    pub fn seat_rows(&self, slot: &str, name: &str, profile_tool: &str, agent_bin: &str) {
        let meta = self.dir.join("meta");
        let mut text = std::fs::read_to_string(&meta).unwrap_or_default();
        let _ = writeln!(
            text,
            "seat.{slot}={name}\nprofile.{slot}=fake-{profile_tool}\nagent_bin.{slot}={agent_bin}"
        );
        assert!(std::fs::write(&meta, text).is_ok(), "the seat's rows");
    }

    pub fn new_pane(&self, slot: &str, name: &str) -> String {
        self.new_pane_running(slot, name, "/bin/sh")
    }

    /// The same pane, with the START command named: `respawn-pane` re-runs it,
    /// which is how a pin can decide what comes back after a stop.
    pub fn new_pane_running(&self, slot: &str, name: &str, command: &str) -> String {
        let (ok, pane) = self.tmux(&[
            "new-window",
            "-d",
            "-t",
            &self.session,
            "-P",
            "-F",
            "#{pane_id}",
            command,
        ]);
        assert!(ok, "a pane for {slot}");
        let pane = pane.trim().to_owned();
        for (option, value) in [("@ae_agent", name), ("@ae_slot", slot)] {
            assert!(
                self.tmux(&["set-option", "-p", "-t", &pane, option, value])
                    .0
            );
        }
        pane
    }

    /// Paste the seat's `_run` line and wait for its TOOL to take the pane.
    ///
    /// Waiting for "not a shell" is not enough and was measured racing: the
    /// pane runs ae's own binary before it runs the tool, so `_run` itself
    /// holds the foreground for a moment. A fixture that started measuring
    /// there would hand the product a pane whose agent has not been exec'd yet.
    pub fn start(&self, pane: &str, slot: &str, tool: &str) {
        let line = format!(
            "{} _run {} {slot}",
            env!("CARGO_BIN_EXE_ae"),
            self.dir.display()
        );
        assert!(self.tmux(&["send-keys", "-t", pane, &line, "Enter"]).0);
        for _ in 0..200 {
            if self.tool_pid(pane, tool).is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "the seat's tool '{tool}' should own pane {pane}, saw '{}'",
            self.pane_cmd(pane)
        );
    }

    /// Make every fake exit, and wait for the pane to fall back to its shell —
    /// the field case this whole verb exists for.
    pub fn kill_tools(&self, pane: &str) {
        assert!(std::fs::write(self.scratch.join("__EXIT__"), "").is_ok());
        assert!(
            self.wait_until(pane, is_shell),
            "the tool should exit, pane still shows '{}'",
            self.pane_cmd(pane)
        );
        assert!(std::fs::remove_file(self.scratch.join("__EXIT__")).is_ok());
    }

    /// Put the frame-drawing fake into its BUSY frame, and wait until the
    /// pane actually shows it — the fake redraws on its own poll, so a test
    /// that ran straight on would race its own setup.
    pub fn mark_busy(&self, pane: &str) {
        assert!(std::fs::write(self.scratch.join("__BUSY__"), "").is_ok());
        assert!(
            self.wait_capture(pane, "esc to interrupt"),
            "the fake should draw its busy frame, saw:\n{}",
            self.capture(pane)
        );
    }

    /// One `display-message` field of a pane, for the facts a stop must keep.
    pub fn pane_fact(&self, pane: &str, format: &str) -> String {
        self.tmux(&["display-message", "-p", "-t", pane, format])
            .1
            .trim()
            .to_owned()
    }

    pub fn capture(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane]).1
    }

    fn wait_capture(&self, pane: &str, needle: &str) -> bool {
        for _ in 0..200 {
            if self.capture(pane).contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn pane_cmd(&self, pane: &str) -> String {
        self.tmux(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{pane_current_command}",
        ])
        .1
        .trim()
        .to_owned()
    }

    fn wait_until(&self, pane: &str, want: impl Fn(&str) -> bool) -> bool {
        for _ in 0..200 {
            if want(&self.pane_cmd(pane)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    pub fn meta(&self) -> String {
        std::fs::read_to_string(self.dir.join("meta")).unwrap_or_default()
    }

    pub fn meta_row(&self, key: &str) -> String {
        self.meta()
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")).map(ToOwned::to_owned))
            .unwrap_or_default()
    }

    pub fn events(&self) -> String {
        std::fs::read_to_string(self.dir.join("events.jsonl")).unwrap_or_default()
    }

    pub fn received(&self) -> String {
        std::fs::read_to_string(self.scratch.join("received")).unwrap_or_default()
    }

    pub fn launched(&self) -> String {
        std::fs::read_to_string(self.scratch.join("launched")).unwrap_or_default()
    }

    pub fn tool_pid(&self, pane: &str, name: &str) -> Option<u32> {
        let pid: u32 = self
            .tmux(&["display-message", "-p", "-t", pane, "#{pane_pid}"])
            .1
            .trim()
            .parse()
            .ok()?;
        let table = ae::procs::snapshot()?;
        table
            .iter()
            .find(|proc| proc.ppid == pid && proc.comm.rsplit('/').next() == Some(name))
            .map(|proc| proc.pid)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // BEST EFFORT, and never a second panic: a failing assertion must
        // report ITS OWN reason, and a destructor that panics while the test
        // is already unwinding aborts the process and hides it.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = self.tmux(&["kill-server"]);
        }));
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

fn is_shell(cmd: &str) -> bool {
    matches!(cmd, "sh" | "bash" | "zsh" | "dash" | "fish" | "")
}

// ---- the seat comes back --------------------------------------------------

#[test]
fn a_dead_fixed_seat_comes_back_in_its_own_pane_on_its_own_slot() {
    let rig = Rig::new("back");
    let pane = rig.seat("worker.1", "w1", "opencode");
    let before = rig.meta_row("launch_time.worker.1");
    rig.kill_tools(&pane);

    let (code, out, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(
        out.contains("relaunched w1") && out.contains(&pane) && out.contains("worker.1"),
        "the line names the agent, its pane and its slot: {out}"
    );
    // THE SAME PANE AND THE SAME SLOT — not a new one beside the old.
    assert_eq!(
        rig.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &rig.session,
            "-F",
            "#{pane_id}|#{@ae_slot}"
        ])
        .1
        .lines()
        .filter(|line| line.ends_with("|worker.1"))
        .collect::<Vec<_>>(),
        vec![format!("{pane}|worker.1")],
        "exactly one pane still carries the slot, and it is the original"
    );
    // The seat's OWN TOOL, proven the way the watchdog's latch release proves it.
    assert!(
        rig.tool_pid(&pane, "opencode").is_some(),
        "the recorded binary runs beneath the pane again"
    );
    let after = rig.meta_row("launch_time.worker.1");
    assert!(
        !after.is_empty() && after != before,
        "the post-exec stamp advanced: {before:?} -> {after:?}"
    );
    assert!(
        rig.events().contains("\"action\":\"relaunch\""),
        "the relaunch is recorded: {}",
        rig.events()
    );
}

#[test]
fn a_seat_whose_tool_is_a_class_ae_never_waits_for_comes_back_too() {
    // `wait_for_process = false`: the launch's own start wait returns at once
    // for this class, so the relaunch's identity observation is the ONLY thing
    // standing between the paste and a claim that the seat is up.
    let rig = Rig::new("unwatched");
    rig.seat_rows("spawned.1", "helper", "grok", "grok");
    let pane = rig.new_pane("spawned.1", "helper");
    rig.start(&pane, "spawned.1", "grok");
    rig.kill_tools(&pane);

    let (code, out, err) = rig.run("_relaunch", &["helper"]);

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(
        rig.tool_pid(&pane, "grok").is_some(),
        "the unwatched tool runs beneath the pane again"
    );
}

// ---- the refusals ---------------------------------------------------------

#[test]
fn a_live_seat_is_refused_and_nothing_is_pasted_into_it() {
    let rig = Rig::new("live");
    let pane = rig.seat("worker.1", "w1", "opencode");

    let (code, out, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("is running") && err.contains(&pane),
        "the refusal names the state and the pane: {err}"
    );
    // PANE BYTES ARE NOT EVIDENCE: the fake logs everything it is sent, so an
    // EMPTY receipt is the proof that nothing was typed at the live agent.
    assert!(
        rig.received().is_empty(),
        "the live agent received: {:?}",
        rig.received()
    );
}

#[test]
fn a_missing_pane_a_dead_pane_and_a_busy_pane_are_three_different_refusals() {
    let rig = Rig::new("ladder");
    let pane = rig.seat("worker.1", "w1", "opencode");

    // 1. NO PANE. A raw pane id resolves without enumerating, so this reaches
    //    the ladder and the probe owns it — deterministic, never a race.
    let (gone_code, _, gone) = rig.run("_relaunch", &["%987"]);
    assert_eq!(gone_code, Some(1), "{gone}");
    assert!(
        gone.contains("is gone") && gone.contains("does not recreate"),
        "a missing pane names the gap and the route: {gone}"
    );

    // 2. BUSY. A process under the seat's shell is not an idle shell, whatever
    //    the dead agent did.
    rig.kill_tools(&pane);
    assert!(rig.tmux(&["send-keys", "-t", &pane, "sleep 30", "Enter"]).0);
    assert!(
        rig.wait_until(&pane, |cmd| cmd == "sleep"),
        "the child runs"
    );
    let (busy_code, _, busy) = rig.run("_relaunch", &["w1"]);
    assert_eq!(busy_code, Some(1), "{busy}");
    assert!(
        busy.contains("is BUSY"),
        "a pane with work under it is refused as busy: {busy}"
    );

    // 3. DEAD PANE. tmux holds the corpse under `remain-on-exit`, and that is
    //    NOT an idle shell either.
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &pane, "remain-on-exit", "on"])
            .0
    );
    assert!(rig.tmux(&["send-keys", "-t", &pane, "C-c"]).0);
    // The shell must own its own stdin again before `exit` is typed, or the
    // still-running child swallows the word and the pane never exits.
    assert!(
        rig.wait_until(&pane, is_shell),
        "the child should die before the shell is told to exit"
    );
    assert!(rig.tmux(&["send-keys", "-t", &pane, "exit", "Enter"]).0);
    let dead = (0..200)
        .find_map(|_| {
            let (_, flag) = rig.tmux(&["display-message", "-p", "-t", &pane, "#{pane_dead}"]);
            (flag.trim() == "1").then_some(()).or_else(|| {
                std::thread::sleep(Duration::from_millis(50));
                None
            })
        })
        .is_some();
    assert!(dead, "the pane should read as dead");
    let (dead_code, _, dead_err) = rig.run("_relaunch", &["w1"]);
    assert_eq!(dead_code, Some(1), "{dead_err}");
    assert!(
        dead_err.contains("is DEAD"),
        "a dead pane is its own refusal: {dead_err}"
    );
    assert!(
        gone != busy && busy != dead_err && gone != dead_err,
        "the three refusals are distinct"
    );
}

#[test]
fn a_seat_whose_recorded_tool_reads_as_a_shell_is_refused_with_the_gap_named() {
    // THE DEAD PROOF'S OWN REFUSAL. The pane sits at an idle shell and the
    // recorded tool is itself a shell name, so ae cannot tell the tool apart
    // from the pane's own shell and must NOT call that dead.
    let rig = Rig::new("gap");
    rig.seat_rows("worker.1", "w1", "opencode", "fish");
    let pane = rig.new_pane("worker.1", "w1");

    let (code, out, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("cannot prove") && err.contains("'fish'"),
        "the refusal NAMES which gap left it unproven: {err}"
    );
    assert!(
        rig.wait_until(&pane, is_shell),
        "nothing was pasted into the pane"
    );
}

#[test]
fn a_pane_that_carries_no_slot_is_not_a_seat_and_is_refused() {
    // A monitor pane carries `@ae_agent` and no `@ae_slot`. It is not a seat,
    // and the refusal says so rather than reporting a missing meta row.
    let rig = Rig::new("monitor");
    let (ok, pane) = rig.tmux(&[
        "new-window",
        "-d",
        "-t",
        &rig.session,
        "-P",
        "-F",
        "#{pane_id}",
        "/bin/sh",
    ]);
    assert!(ok);
    let pane = pane.trim().to_owned();
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "watchdog"])
            .0
    );

    let (code, _, err) = rig.run("_relaunch", &[&pane]);

    assert_eq!(code, Some(1), "{err}");
    assert!(
        err.contains("carries no ae slot"),
        "a non-seat pane is refused as one: {err}"
    );
}

#[test]
fn a_seat_of_another_session_is_refused_because_relaunch_is_own_session_only() {
    let rig = Rig::new("cross");
    // A REAL second session, with its own state directory under the same root,
    // so the target RESOLVES and the own-session check is what refuses it.
    let other = rig.scratch.join("sessions").join("rlother");
    assert!(std::fs::create_dir_all(&other).is_ok());
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "rlother", "/bin/sh"])
            .0,
        "the second session starts"
    );
    let (_, panes) = rig.tmux(&["list-panes", "-s", "-t", "rlother", "-F", "#{pane_id}"]);
    let pane = panes.lines().next().unwrap_or_default().to_owned();
    for (option, value) in [
        ("@ae_agent", "w9"),
        ("@ae_slot", "worker.1"),
        ("@ae_session", "rlother"),
    ] {
        assert!(
            rig.tmux(&["set-option", "-p", "-t", &pane, option, value])
                .0
        );
    }

    let (code, _, err) = rig.run("_relaunch", &[&pane]);

    assert_eq!(code, Some(1), "{err}");
    assert!(
        err.contains("rlother") && err.contains("own session only"),
        "the refusal names the other session and the rule: {err}"
    );
}

#[test]
fn a_seat_whose_profile_is_gone_is_refused_before_anything_is_pasted() {
    let rig = Rig::new("noprofile");
    let pane = rig.seat("worker.1", "w1", "opencode");
    rig.kill_tools(&pane);
    // The profile the seat records is no longer configured on this machine.
    assert!(
        std::fs::write(
            rig.scratch.join("config"),
            "[profiles]\n[workspace]\nmain = lead\n"
        )
        .is_ok()
    );

    let (code, _, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(1), "{err}");
    assert!(
        err.contains("fake-opencode") && err.contains("not configured"),
        "the refusal is the seat resolver's OWN named reason: {err}"
    );
    assert!(
        rig.wait_until(&pane, is_shell),
        "nothing was pasted into the pane"
    );
}

#[test]
fn a_refusal_before_the_paste_says_so_once_and_records_no_relaunch() {
    // The launch-attempt stamp is written BEFORE the paste and is CHECKED, so
    // its failure must read as what it is. A pre-paste refusal that fell into
    // the post-paste arm would print a second, contradicting line and leave an
    // event claiming the seat was touched.
    let rig = Rig::new("nostamp");
    let pane = rig.seat("worker.1", "w1", "opencode");
    rig.kill_tools(&pane);
    let before = rig.events();
    // `create_new` on the stamp's temp file cannot make it into a session
    // directory nothing may write to.
    assert!(set_mode(&rig.dir, 0o555).is_ok());

    let (code, _, err) = rig.run("_relaunch", &["w1"]);

    assert!(set_mode(&rig.dir, 0o755).is_ok());
    assert_eq!(code, Some(1), "{err}");
    assert!(
        err.contains("launch attempt could not be recorded")
            && err.contains("nothing was relaunched"),
        "the stamp failure is the ONLY reason printed: {err}"
    );
    assert!(
        !err.contains("line pasted"),
        "nothing was pasted, so nothing may say it was: {err}"
    );
    assert_eq!(
        rig.events(),
        before,
        "an attempt that never reached the pane records no relaunch"
    );
    assert!(
        rig.wait_until(&pane, is_shell),
        "the pane was left at its shell"
    );
}

/// The one permission write these pins need, kept off `Rig` because nothing
/// else in the module changes a mode.
fn set_mode(dir: &std::path::Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
}

#[test]
fn a_seat_whose_tool_changed_under_it_is_refused_by_the_run_it_pasted() {
    // The helper resolves the command and hands `_run` the snapshot, so `_run`
    // applies its OWN mismatch check in the pane. Exit 0 needs the seat's tool
    // running, and a `_run` that refuses and prints is not that.
    let rig = Rig::new("changed");
    rig.seat_rows("worker.1", "w1", "grok", "opencode");
    let pane = rig.new_pane("worker.1", "w1");

    let (code, _, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(1), "{err}");
    assert!(
        err.contains("tool not seen yet"),
        "the line pasted and no tool came up: {err}"
    );
    assert!(
        rig.tool_pid(&pane, "grok").is_none() && rig.tool_pid(&pane, "opencode").is_none(),
        "no tool was started"
    );
    assert!(
        rig.received().is_empty(),
        "no agent received anything: {:?}",
        rig.received()
    );
}

#[test]
fn a_relaunch_refuses_while_another_lifecycle_operation_holds_the_session() {
    let rig = Rig::new("locked");
    let pane = rig.seat("worker.1", "w1", "opencode");
    rig.kill_tools(&pane);
    let lock = rig
        .scratch
        .join("sessions")
        .join(".lifecycle.rllocked.lock");
    let held = ae::store::lock(&lock, Duration::ZERO).expect("the lifecycle lock is free");

    let (code, _, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(1), "{err}");
    assert!(
        err.contains("another lifecycle operation holds"),
        "the refusal names the contention: {err}"
    );
    assert!(
        rig.wait_until(&pane, is_shell),
        "nothing was pasted while the lock was held"
    );
    drop(held);
}

// ---- the capture floor ----------------------------------------------------

#[test]
fn a_retained_conversation_keeps_its_capture_floor_and_a_pending_one_gets_a_fresh_one() {
    // CASE 1: the recorded id passes its store probe, so the seat resumes the
    // SAME conversation — which was born under the floor already recorded, and
    // moving it would exclude that already-proved origin.
    let rig = Rig::new("floorkeep");
    rig.seat_rows("worker.1", "w1", "opencode", "opencode");
    let meta = rig.dir.join("meta");
    let mut text = std::fs::read_to_string(&meta).unwrap_or_default();
    text.push_str("harness_session.worker.1=abc-123\ncapture_floor.worker.1=111\n");
    assert!(std::fs::write(&meta, text).is_ok());
    let pane = rig.new_pane("worker.1", "w1");
    rig.start(&pane, "worker.1", "opencode");
    rig.kill_tools(&pane);

    let (code, out, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(out.contains("exact"), "the conversation was resumed: {out}");
    assert_eq!(
        rig.meta_row("capture_floor.worker.1"),
        "111",
        "a retained conversation keeps the floor it was born under"
    );
    assert_eq!(rig.meta_row("harness_session.worker.1"), "abc-123");
}

#[test]
fn a_seat_with_no_usable_conversation_gets_a_fresh_floor_before_its_tool_starts() {
    // CASE 2: nothing to retain, so the floor is NOW — published before the
    // exec, so the capture cannot look further back than this seat's own life.
    let rig = Rig::new("floorfresh");
    rig.seat_rows("worker.1", "w1", "opencode", "opencode");
    let pane = rig.new_pane("worker.1", "w1");
    rig.start(&pane, "worker.1", "opencode");
    rig.kill_tools(&pane);

    let (code, out, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(out.contains("fresh"), "no conversation was retained: {out}");
    assert!(
        rig.meta_row("capture_floor.worker.1")
            .parse::<i64>()
            .is_ok_and(|floor| floor > 0),
        "a real epoch was published: {:?}",
        rig.meta_row("capture_floor.worker.1")
    );
}

#[test]
fn a_conversation_that_dies_inside_run_moves_its_floor_forward_and_leaves_the_slot_pending() {
    // CASE 3, the one a pre-exec decision alone would get wrong: the id looked
    // probeable, so NO fresh floor was written — and then `_run`'s own store
    // probe failed and the seat took the fallback. The fallback itself
    // republishes the floor with that moment, still BEFORE the tool is exec'd
    // — the same pre-exec seat CASE 2's fresh floors already publish from, so
    // it cannot be too late by the same argument that makes CASE 2 safe. The
    // stale floor is the real hazard instead: many seats share one working
    // directory, so a tokenless scan from a predecessor-era floor could take a
    // NEIGHBOUR seat's conversation. The slot reads `pending`, which is
    // exactly what makes the post-start re-read schedule the capture.
    let rig = Rig::new("floorfall");
    rig.seat_rows("worker.1", "w1", "agy", "agy");
    let meta = rig.dir.join("meta");
    let mut text = std::fs::read_to_string(&meta).unwrap_or_default();
    text.push_str("harness_session.worker.1=abc-123\ncapture_floor.worker.1=111\n");
    assert!(std::fs::write(&meta, text).is_ok());
    let pane = rig.new_pane("worker.1", "w1");
    rig.start(&pane, "worker.1", "agy");
    rig.kill_tools(&pane);

    let (code, out, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(
        out.contains("fresh"),
        "the recorded conversation did not survive its probe: {out}"
    );
    let floor: i64 = rig
        .meta_row("capture_floor.worker.1")
        .parse()
        .expect("the fallback republishes the floor as an epoch");
    assert!(
        floor > 111,
        "the floor moves forward past the abandoned era: {floor}"
    );
    assert!(
        floor <= ae::time::Timestamp::now().epoch(),
        "the floor is the fallback moment, never the future: {floor}"
    );
    assert_eq!(
        rig.meta_row("harness_session.worker.1"),
        "pending",
        "the slot reads pending, which is what schedules the capture"
    );
}

// ---- what the pane is actually handed -------------------------------------

#[test]
fn a_half_typed_line_is_cleared_rather_than_run_together_with_the_launch_line() {
    // A shell prompt is not a modelled composer: nothing can verify the clear,
    // so the pin is the OUTCOME — the seat's tool starts, which it cannot do if
    // the pasted line were glued onto whatever was already typed.
    let rig = Rig::new("halftyped");
    let pane = rig.seat("worker.1", "w1", "opencode");
    rig.kill_tools(&pane);
    let evidence = rig.scratch.join("EXECUTED");
    assert!(
        rig.tmux(&[
            "send-keys",
            "-t",
            &pane,
            &format!("touch {}", evidence.display()),
        ])
        .0,
        "plant a half-typed line"
    );
    // MID-LINE, not at either end: `C-u` alone would leave everything right of
    // the cursor for the paste to run.
    for _ in 0..3 {
        assert!(rig.tmux(&["send-keys", "-t", &pane, "Left"]).0);
    }

    let (code, out, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(
        rig.tool_pid(&pane, "opencode").is_some(),
        "the seat's tool started, so the launch line reached the shell intact"
    );
    assert!(
        !evidence.exists(),
        "the half-typed line was cleared, not executed"
    );
}

#[test]
fn a_shell_that_wandered_off_resumes_the_seat_in_its_recorded_working_copy() {
    // `pane_line`'s directory is the STATE dir and `_run` never chdirs, so
    // without the restore the agent would come back wherever the shell was
    // left standing.
    let rig = Rig::new("wandered");
    let pane = rig.seat("worker.1", "w1", "opencode");
    rig.kill_tools(&pane);
    assert!(rig.tmux(&["send-keys", "-t", &pane, "cd /", "Enter"]).0);

    let (code, out, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(0), "out={out} err={err}");
    let recorded = rig.meta_row("work_dir");
    let logged = rig.launched();
    assert!(
        logged
            .lines()
            .rev()
            .any(|line| line == format!("cwd={recorded}")),
        "the tool came back in {recorded}, log was: {logged}"
    );
}

#[test]
fn a_launch_turn_that_never_lands_fails_the_relaunch_and_names_the_preserved_text() {
    // Codex is the one adapter whose resumed seat is handed its first turn by
    // PASTE, and this fake never draws an input box a delivery can prove ready.
    // The turn is preserved and the relaunch does NOT claim success.
    let rig = Rig::new("undelivered");
    rig.seat_rows("worker.1", "w1", "codex", "codex");
    let pane = rig.new_pane("worker.1", "w1");
    rig.start(&pane, "worker.1", "codex");
    rig.kill_tools(&pane);

    let (code, out, err) = rig.run("_relaunch", &["w1"]);

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.contains("undelivered"),
        "the failure prints the turn's own state word: {err}"
    );
    assert!(
        rig.dir.join("undelivered.launch-worker.1.txt").exists(),
        "the text the seat never received is preserved"
    );
    assert!(
        rig.tool_pid(&pane, "codex").is_some(),
        "the tool DID come up — it is the turn that failed, and nothing is rolled back"
    );
}
