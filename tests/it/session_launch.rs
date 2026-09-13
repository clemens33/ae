//! `_launch` against a REAL tmux server: the whole session, built or resumed.
//!
//! The operation runs end to end — the working copy, the tmux session, its
//! panes and their stamps, the meta, the helper links, and the paste that hands
//! each pane the core command which becomes its agent. The agents are the same
//! perl fake the spawn suite uses, named for the tool whose classification the
//! test needs.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fmt::Write as _;
use std::io::BufRead as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use super::cli::{OwnedChild, OwnedScratch, Runner, ae, git_in, helper};
use super::phase2::run_tmux;

/// A TUI-shaped fake agent: it records its argv, then sits there drawing the
/// ornament the input sensor reads.
const FAKE_AGENT: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
system("stty raw -echo 2>/dev/null");
binmode(STDIN, ':raw');
binmode(STDOUT, ':raw');
$| = 1;
open(my $log, '>>', "__LAUNCHED__") or die;
print $log join(" ", @ARGV), "\n";
print $log "CLAUDE_CONFIG_DIR=", ($ENV{CLAUDE_CONFIG_DIR} // "<unset>"), "\n";
close($log);
if (length("__SID__")) {
    open(my $sid, '>', "__SID__") or die; print $sid "cafe-1234\n"; close($sid);
}
print "\e[?2004h";
my $border = "\xe2\x94\x80" x 400;
my $codex = $0 =~ /codex\z/;
my $ornament = $codex ? "\xe2\x80\xba" : "\xe2\x9d\xaf";
my $nbsp = "\xc2\xa0";
print "\e[H\e[2J";
print "fake agent transcript\r\n";
print "\e[1m$ornament\e[0m$nbsp\r\n";
if ($codex) { print "\r\n"; } else { print "$border\r\n"; }
print "  fake-model  ~/x\r\n";
while (1) { sleep 1; }
"#;

/// A roster whose agent does nothing but stay in its pane.
const IDLE_CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
     [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n";

const SOLO_OVERRIDE_CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\nsolx = \"tail -f /dev/null\"\n\n\
     [roster]\nlead = idle\ncolead = idle\nbuilder = idle\n\n\
     [workspace]\nmain = lead\nworkers = colead, builder\nlayout = lead-pair\nwatchdog = false\n";

/// One isolated ae home, one project directory, one tmux server.
struct Rig {
    scratch: OwnedScratch,
    sock: PathBuf,
    home: PathBuf,
    project: PathBuf,
    config: PathBuf,
    bin: PathBuf,
    launched: PathBuf,
}

impl Rig {
    /// `tools` names each fake agent to install, and whether it writes a
    /// `codex.<slot>.sid` handshake file into the session directory.
    fn new(tag: &str, tools: &[&str], sid_for: Option<&str>) -> Self {
        let mut scratch = OwnedScratch::existing(PathBuf::from(format!(
            "/tmp/aeln.{}.{tag}",
            std::process::id()
        )));
        let home = scratch.join("aehome");
        let project = scratch.join("project");
        assert!(std::fs::create_dir_all(&project).is_ok(), "a project dir");
        for path in [scratch.join(".claude"), scratch.join(".codex")] {
            assert!(std::fs::create_dir_all(path).is_ok(), "a config home");
        }
        let launched = scratch.join("launched");
        let bin_dir = scratch.join("bin");
        assert!(std::fs::create_dir_all(&bin_dir).is_ok(), "a bin dir");
        let mut profiles = String::from("[profiles]\n");
        for tool in tools {
            let bin = bin_dir.join(tool);
            let sid = sid_for.map_or_else(String::new, |slot| {
                home.join("sessions")
                    .join(tag)
                    .join(format!("codex.{slot}.sid"))
                    .display()
                    .to_string()
            });
            let body = FAKE_AGENT
                .replace("__LAUNCHED__", &launched.display().to_string())
                .replace("__SID__", if *tool == "codex" { &sid } else { "" });
            assert!(std::fs::write(&bin, body).is_ok(), "the fake {tool}");
            assert!(
                std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).is_ok(),
                "an executable fake {tool}"
            );
            let _ = writeln!(profiles, "{tool} = \"{}\"", bin.display());
        }
        let config = scratch.join("config");
        let main_tool = tools.first().copied().unwrap_or("claude");
        assert!(
            std::fs::write(
                &config,
                format!(
                    "{profiles}\n[roster]\nlead = {main_tool}\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n"
                ),
            )
            .is_ok(),
            "a config"
        );
        let sock = scratch.join("sock");
        scratch.add_tmux_server(sock.clone());
        let uid = std::fs::metadata(&scratch).map_or(0, |metadata| metadata.uid());
        scratch.add_tmux_server(scratch.join(format!("tmux-{uid}")).join("default"));
        Self {
            scratch,
            sock,
            home,
            project,
            config,
            bin: bin_dir,
            launched,
        }
    }

    /// The same rig with a bare sleeper as its agent.
    fn idle(tag: &str) -> Self {
        let rig = Self::new(tag, &[], None);
        assert!(
            std::fs::write(&rig.config, IDLE_CONFIG).is_ok(),
            "an idle config"
        );
        rig
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        self.tmux_on(&self.sock, tail)
    }

    fn tmux_on(&self, socket: &Path, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(socket.to_path_buf()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    fn tmux_named(&self, name: &str, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Name(name.to_owned()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    /// Run `_launch` with the preamble this rig implies.
    fn launch(&self, tail: &[&str]) -> (Option<i32>, String, String) {
        self.launch_with_server("socket", &self.sock.display().to_string(), tail)
    }

    /// The same launch with an ARBITRARY server pair — what the flag-validation
    /// arms need.
    fn launch_with_server(
        &self,
        kind: &str,
        value: &str,
        tail: &[&str],
    ) -> (Option<i32>, String, String) {
        let out = self
            .launch_command_with_server(kind, value, tail)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn launch_command_with_server(&self, kind: &str, value: &str, tail: &[&str]) -> Runner {
        self.launch_command_with_server_options(kind, value, tail, false)
    }

    fn launch_command_with_server_options(
        &self,
        kind: &str,
        value: &str,
        tail: &[&str],
        pre_lock_marker: bool,
    ) -> Runner {
        let mut command = ae();
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            // THE RIG'S OWN HOME, not the runner's per-call one: `launch` and
            // `plan` must agree about where an agent tool keeps its
            // conversation store, and neither may be the developer's.
            .env("HOME", &self.scratch)
            .env("TMUX_TMPDIR", &self.scratch)
            .arg(ae::cli::LAUNCH)
            .args([
                "--home",
                &self.home.display().to_string(),
                "--cwd",
                &self.project.display().to_string(),
                "--global",
                &self.config.display().to_string(),
                "--server-kind",
                kind,
                "--server",
                value,
                "--no-attach",
            ]);
        if pre_lock_marker {
            command.arg("--test-pre-lock-marker");
        }
        command.arg("--").args(tail);
        command
    }

    fn launch_child(&self, tail: &[&str]) -> OwnedChild {
        let mut command =
            self.launch_command_with_server("socket", &self.sock.display().to_string(), tail);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|why| panic!("the ae binary should start: {why}"))
    }

    fn launch_child_with_pre_lock_marker(&self, tail: &[&str]) -> OwnedChild {
        let mut command = self.launch_command_with_server_options(
            "socket",
            &self.sock.display().to_string(),
            tail,
            true,
        );
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|why| panic!("the ae binary should start: {why}"))
    }

    fn wait_for_pre_lock_marker(&self) {
        let marker = self.home.join(ae::session_launch::TEST_PRE_LOCK_MARKER);
        for _ in 0..1_000 {
            if marker.is_file() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "launch did not finish preflight within 10s: {}",
            marker.display()
        );
    }

    /// Launch with attach enabled and an optional real caller pane/socket.
    fn launch_attaching(
        &self,
        destination: &Path,
        caller: Option<(&Path, &str)>,
        tail: &[&str],
    ) -> (Option<i32>, String, String) {
        let mut command = ae();
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &self.scratch)
            .env("TMUX_TMPDIR", &self.scratch)
            .arg(ae::cli::LAUNCH)
            .args([
                "--home",
                &self.home.display().to_string(),
                "--cwd",
                &self.project.display().to_string(),
                "--global",
                &self.config.display().to_string(),
                "--server-kind",
                "socket",
                "--server",
                &destination.display().to_string(),
            ]);
        if let Some((socket, pane)) = caller {
            command
                .arg("--caller-socket")
                .arg(socket)
                .arg("--inside-tmux")
                .env("TMUX", format!("{},1,0", socket.display()))
                .env("TMUX_PANE", pane);
        }
        let out = command
            .args(["--attach", "--"])
            .args(tail)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// The sessions on a server addressed the way `tmux` itself would be — with
    /// the operator's environment dropped, so `TMUX_TMPDIR` decides the default
    /// socket instead of an inherited `TMUX`.
    fn sessions_on(&self, server_args: &[&str]) -> Vec<String> {
        let mut invocation = super::parity::Invocation::new("tmux")
            .env_cleared()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.scratch)
            .env("TMUX_TMPDIR", &self.scratch);
        for arg in server_args {
            invocation = invocation.arg(arg);
        }
        for arg in ["list-sessions", "-F", "#{session_name}"] {
            invocation = invocation.arg(arg);
        }
        let out = self.scratch.join("amb-out");
        let err = self.scratch.join("amb-err");
        let _ = super::parity::capture::raw::run(&invocation, &self.scratch, &out, &err);
        std::fs::read_to_string(&out)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Kill a server this rig addressed by `server_args`, so a named one does
    /// not outlive the test.
    fn kill_server_at(&self, server_args: &[&str]) {
        let mut invocation = super::parity::Invocation::new("tmux")
            .env_cleared()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.scratch)
            .env("TMUX_TMPDIR", &self.scratch);
        for arg in server_args {
            invocation = invocation.arg(arg);
        }
        invocation = invocation.arg("kill-server");
        let out = self.scratch.join("kill-out");
        let err = self.scratch.join("kill-err");
        let _ = super::parity::capture::raw::run(&invocation, &self.scratch, &out, &err);
    }

    fn dir(&self, session: &str) -> PathBuf {
        self.home.join("sessions").join(session)
    }

    /// `_run --print` for one seat: the JSON plan the pane's own command would
    /// exec, without execing it.
    fn plan(&self, session: &str, slot: &str) -> String {
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &self.scratch)
            .current_dir(&self.project)
            .arg(ae::cli::RUN)
            .arg("--print")
            .arg(self.dir(session))
            .arg(slot)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        assert!(
            out.status.success(),
            "_run --print {slot}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// The session names the rig's server holds right now.
    fn sessions(&self) -> Vec<String> {
        let (_, listed) = self.tmux(&["list-sessions", "-F", "#{session_name}"]);
        listed.lines().map(str::to_owned).collect()
    }

    fn meta(&self, session: &str) -> String {
        std::fs::read_to_string(self.dir(session).join("meta")).unwrap_or_default()
    }

    /// Every AGENT window of `session` as `(index, name, pane count)`, in tmux
    /// order.
    fn windows(&self, session: &str) -> Vec<(String, String, usize)> {
        let (_, listed) = self.tmux(&[
            "list-windows",
            "-t",
            session,
            "-F",
            "#{window_index}|#{window_name}|#{window_panes}",
        ]);
        listed
            .lines()
            .map(|line| {
                let mut fields = line.splitn(3, '|');
                (
                    fields.next().unwrap_or_default().to_owned(),
                    fields.next().unwrap_or_default().to_owned(),
                    fields
                        .next()
                        .unwrap_or_default()
                        .parse::<usize>()
                        .unwrap_or_default(),
                )
            })
            .filter(|window| window.1 != "ae-monitor")
            .collect()
    }

    fn wait_for_pane_command(&self, session: &str, slot: &str, command: &str) {
        for _ in 0..200 {
            let (_, listed) = self.tmux(&[
                "list-panes",
                "-s",
                "-t",
                &format!("={session}"),
                "-F",
                "#{@ae_slot}|#{pane_current_command}",
            ]);
            if listed
                .lines()
                .any(|line| line == format!("{slot}|{command}"))
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("{session} {slot} never ran {command}");
    }

    fn panes(&self, session: &str) -> Vec<(String, String, String)> {
        let (_, listed) = self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            session,
            "-F",
            "#{pane_id}|#{@ae_slot}|#{@ae_agent}",
        ]);
        listed
            .lines()
            .map(|line| {
                let mut fields = line.splitn(3, '|');
                (
                    fields.next().unwrap_or_default().to_owned(),
                    fields.next().unwrap_or_default().to_owned(),
                    fields.next().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }

    /// Wait briefly for the fake agent to record that it started.
    fn launch_argv(&self) -> String {
        // 20s, not 5.
        for _ in 0..800 {
            let seen = std::fs::read_to_string(&self.launched).unwrap_or_default();
            if !seen.is_empty() {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        std::fs::read_to_string(&self.launched).unwrap_or_default()
    }
}

fn tmux_present(scratch: &Path) -> bool {
    super::phase2::tmux_present(scratch)
}

/// Guard: without tmux none of this proves anything.
fn skip() -> bool {
    let probe = PathBuf::from(format!("/tmp/aeln-probe.{}", std::process::id()));
    let _ = std::fs::create_dir_all(&probe);
    let present = tmux_present(&probe);
    let _ = std::fs::remove_dir_all(&probe);
    !present
}

/// Add an ordinary profile row to a rig's config without changing its roster.
fn add_profile(rig: &Rig, name: &str, command: &str) {
    let mut config = std::fs::read_to_string(&rig.config).unwrap_or_default();
    let _ = writeln!(config, "\n[profiles]\n{name} = \"{command}\"");
    assert!(std::fs::write(&rig.config, config).is_ok(), "a profile");
}

#[test]
fn quota_cadence_is_validated_before_launch_and_persisted_for_the_daemon() {
    if skip() {
        return;
    }
    let rig = Rig::new("quota-cadence", &["claude"], None);
    let base = std::fs::read_to_string(&rig.config).expect("the rig config");
    for (index, value) in ["", "-1", "soon", "18446744073709551616"]
        .into_iter()
        .enumerate()
    {
        let config = base.replace(
            "watchdog = false\n",
            &format!("watchdog = false\nquota_every_secs = {value}\n"),
        );
        assert!(std::fs::write(&rig.config, config).is_ok(), "bad config");
        let session = format!("badquota{index}");
        let (code, stdout, stderr) = rig.launch(&["--local", &session]);
        assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
        assert!(
            stderr.contains("[workspace] quota_every_secs")
                && stderr.contains(&format!("got '{value}'.")),
            "{stderr}"
        );
        assert!(
            !rig.dir(&session).exists(),
            "invalid cadence created a session directory"
        );
    }

    let config = base.replace(
        "watchdog = false\n",
        "watchdog = false\nquota_every_secs = 420\n",
    );
    assert!(std::fs::write(&rig.config, config).is_ok(), "valid config");
    let (code, stdout, stderr) = rig.launch(&["--local", "goodquota"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.meta("goodquota").contains("quota_every_secs=420\n"),
        "the daemon's persisted input is missing"
    );
    assert!(
        rig.tmux(&["kill-session", "-t", "=goodquota"]).0,
        "stop the first runtime without removing its state"
    );
    let changed = base.replace(
        "watchdog = false\n",
        "watchdog = false\nquota_every_secs = 7\n",
    );
    assert!(
        std::fs::write(&rig.config, changed).is_ok(),
        "changed config"
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "goodquota"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.meta("goodquota").contains("quota_every_secs=420\n"),
        "resume replaced the persisted cadence from changed config"
    );
    rig.kill_server_at(&["-S", &rig.sock.display().to_string()]);
}

#[test]
fn idle_nudge_cadence_defaults_validates_and_survives_resume() {
    if skip() {
        return;
    }
    let rig = Rig::new("idle-nudge-cadence", &["claude"], None);
    let base = std::fs::read_to_string(&rig.config).expect("the rig config");

    let (code, stdout, stderr) = rig.launch(&["--local", "defaultidle"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.meta("defaultidle").contains("idle_nudge_secs=300\n"),
        "the default cadence is a durable launch fact"
    );

    let invalid = base.replace(
        "watchdog = false\n",
        "watchdog = false\nidle_nudge_secs = soon\n",
    );
    assert!(std::fs::write(&rig.config, invalid).is_ok(), "bad config");
    let (code, stdout, stderr) = rig.launch(&["--local", "badidle"]);
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("[workspace] idle_nudge_secs") && stderr.contains("got 'soon'."),
        "{stderr}"
    );
    assert!(!rig.dir("badidle").exists());

    let configured = base.replace(
        "watchdog = false\n",
        "watchdog = false\nidle_nudge_secs = 420\n",
    );
    assert!(
        std::fs::write(&rig.config, configured).is_ok(),
        "valid config"
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "goodidle"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(rig.meta("goodidle").contains("idle_nudge_secs=420\n"));
    assert!(
        rig.tmux(&["kill-session", "-t", "=goodidle"]).0,
        "stop runtime without removing state"
    );
    let changed = base.replace(
        "watchdog = false\n",
        "watchdog = false\nidle_nudge_secs = 7\n",
    );
    assert!(std::fs::write(&rig.config, changed).is_ok());
    let (code, stdout, stderr) = rig.launch(&["--local", "goodidle"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.meta("goodidle").contains("idle_nudge_secs=420\n"),
        "resume replaced persisted cadence"
    );
    rig.kill_server_at(&["-S", &rig.sock.display().to_string()]);
}

#[test]
fn quota_awareness_defaults_on_pins_off_and_survives_resume() {
    if skip() {
        return;
    }
    let rig = Rig::new("quota-aware", &["claude"], None);
    let base = std::fs::read_to_string(&rig.config).expect("the rig config");

    let (code, stdout, stderr) = rig.launch(&["--local", "defaultaware"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.meta("defaultaware").contains("quota=on\n"),
        "absent means ON, pinned as a durable launch fact"
    );

    let off = base.replace("watchdog = false\n", "watchdog = false\nquota = off\n");
    assert!(std::fs::write(&rig.config, off).is_ok(), "off config");
    let (code, stdout, stderr) = rig.launch(&["--local", "offaware"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.meta("offaware").contains("quota=off\n"),
        "the daemon's persisted awareness is missing"
    );
    assert!(
        rig.tmux(&["kill-session", "-t", "=offaware"]).0,
        "stop the first runtime without removing its state"
    );
    assert!(std::fs::write(&rig.config, base).is_ok(), "changed config");
    let (code, stdout, stderr) = rig.launch(&["--local", "offaware"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.meta("offaware").contains("quota=off\n"),
        "resume replaced the persisted awareness from changed config"
    );
    rig.kill_server_at(&["-S", &rig.sock.display().to_string()]);
}

#[test]
#[allow(clippy::too_many_lines, reason = "one end-to-end retained-store story")]
fn a_client_profile_launches_the_configured_executable_and_home() {
    if skip() {
        return;
    }
    let rig = Rig::new("client-profile", &["claude"], None);
    let executable = rig.bin.join("claude");
    let first_home = rig.scratch.join(".claude-mic");
    let second_home = rig.scratch.join(".claude-other");
    assert!(std::fs::create_dir_all(&first_home).is_ok(), "first store");
    assert!(
        std::fs::create_dir_all(&second_home).is_ok(),
        "second store"
    );
    let first_home = std::fs::canonicalize(first_home).expect("canonical first store");
    let second_home = std::fs::canonicalize(second_home).expect("canonical second store");
    assert!(
        std::fs::write(
            &rig.config,
            format!(
                "[clients]\ncc = {} config_home=$HOME/.claude-mic\n\
                 [profiles]\nfable = cc --served client\n\
                 [roster]\nlead = fable\n\
                 [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
                executable.display()
            )
        )
        .is_ok(),
        "a client-based config"
    );

    let (code, stdout, stderr) = rig.launch(&["--local", "lnclientprofile"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let evidence = rig.launch_argv();
    assert!(
        evidence.contains("--served client"),
        "argv evidence: {evidence}"
    );
    assert!(
        evidence.contains(&format!("CLAUDE_CONFIG_DIR={}", first_home.display())),
        "env evidence: {evidence}"
    );
    let plan = rig.plan("lnclientprofile", "main");
    assert!(
        plan.contains(&format!(r#""argv":["{}""#, executable.display())),
        "{plan}"
    );
    assert!(
        plan.contains(&format!(
            r#""CLAUDE_CONFIG_DIR":"{}""#,
            first_home.display()
        )),
        "{plan}"
    );
    let first_meta = rig.meta("lnclientprofile");
    assert!(
        first_meta.contains(&format!("config_home.main={}\n", first_home.display())),
        "{first_meta}"
    );

    let sid = first_meta
        .lines()
        .find_map(|line| line.strip_prefix("harness_session.main="))
        .unwrap_or_default();
    let key: String = std::fs::canonicalize(&rig.project)
        .unwrap_or_else(|_| rig.project.clone())
        .display()
        .to_string()
        .chars()
        .map(|ch| if ch == '/' { '-' } else { ch })
        .collect();
    let transcript = first_home
        .join("projects")
        .join(key)
        .join(format!("{sid}.jsonl"));
    assert!(
        std::fs::create_dir_all(transcript.parent().unwrap_or(&first_home)).is_ok()
            && std::fs::write(&transcript, "{}\n").is_ok(),
        "a retained transcript"
    );
    assert!(
        rig.tmux(&["kill-session", "-t", "=lnclientprofile"]).0,
        "the session stops"
    );
    let changed = std::fs::read_to_string(&rig.config)
        .unwrap_or_default()
        .replace(".claude-mic", ".claude-other");
    assert!(std::fs::write(&rig.config, changed).is_ok(), "config moves");
    let _ = std::fs::remove_file(&rig.launched);

    let (code, stdout, stderr) = rig.launch(&["--local", "lnclientprofile"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let resumed = rig.launch_argv();
    assert!(
        resumed.contains(&format!("--resume {sid}"))
            && resumed.contains(&format!("CLAUDE_CONFIG_DIR={}", first_home.display())),
        "recorded row controls probe and exec: {resumed}"
    );
    assert!(
        !resumed.contains(&format!("CLAUDE_CONFIG_DIR={}", second_home.display())),
        "changed config does not move retained conversation: {resumed}"
    );
    let rebuilt = rig.meta("lnclientprofile");
    assert!(
        rebuilt.contains(&format!("config_home.main={}\n", first_home.display())),
        "full resume carries recorded home: {rebuilt}"
    );
}

fn bare_session(rig: &Rig, socket: &Path, name: &str) -> String {
    let (created, pane) = rig.tmux_on(
        socket,
        &[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-s",
            name,
        ],
    );
    assert!(created, "a bare session named {name}");
    pane.trim().to_owned()
}

fn bare_named_session(rig: &Rig, server: &str, name: &str) -> String {
    let (created, pane) = rig.tmux_named(
        server,
        &[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-s",
            name,
        ],
    );
    assert!(created, "a bare session named {name}");
    pane.trim().to_owned()
}

fn set_named_session_env(rig: &Rig, server: &str, session: &str, key: &str, value: &str) {
    assert!(
        rig.tmux_named(
            server,
            &["set-environment", "-t", &format!("={session}"), key, value]
        )
        .0,
        "set {key} on {session}"
    );
}

#[test]
fn a_verified_stopped_session_repairs_to_the_proposed_server_only_after_build() {
    if skip() {
        return;
    }
    let rig = Rig::idle("repair-stopped");
    let first = rig.launch(&["--local", "repair-stopped"]);
    assert_eq!(first.0, Some(0), "{first:?}");
    let _keep = bare_session(&rig, &rig.sock, "keep-old-server");
    assert!(
        rig.tmux(&["kill-session", "-t", "=repair-stopped"]).0,
        "stop the recorded session while its server remains queryable"
    );
    let old_meta = rig.meta("repair-stopped");
    let destination = rig.scratch.join("repair-destination.sock");

    let resumed = rig.launch_with_server(
        "socket",
        &destination.display().to_string(),
        &["--local", "repair-stopped"],
    );
    let new_meta = rig.meta("repair-stopped");

    assert_eq!(resumed.0, Some(0), "{resumed:?}");
    assert!(!rig.sessions().contains(&"repair-stopped".to_owned()));
    assert!(
        rig.sessions_on(&["-S", &destination.display().to_string()])
            .contains(&"repair-stopped".to_owned())
    );
    assert_ne!(
        new_meta, old_meta,
        "the successful build publishes the move"
    );
    assert!(
        new_meta.contains(&format!("tmux_server={}\n", destination.display())),
        "{new_meta}"
    );
    rig.kill_server_at(&["-S", &destination.display().to_string()]);
}

#[test]
fn a_failed_creation_keeps_the_old_pair_and_the_next_resume_can_repair_it() {
    if skip() {
        return;
    }
    let rig = Rig::idle("repair-retry");
    let first = rig.launch(&["--local", "repair-retry"]);
    assert_eq!(first.0, Some(0), "{first:?}");
    let _keep = bare_session(&rig, &rig.sock, "keep-retry-source");
    assert!(
        rig.tmux(&["kill-session", "-t", "=repair-retry"]).0,
        "stop the recorded session while its server remains queryable"
    );
    let before = rig.meta("repair-retry");
    let parent = rig.scratch.join("late-socket-dir");
    let destination = parent.join("sock");

    let failed = rig.launch_with_server(
        "socket",
        &destination.display().to_string(),
        &["--local", "repair-retry"],
    );
    assert_eq!(failed.0, Some(1), "{failed:?}");
    assert_eq!(rig.meta("repair-retry"), before);

    assert!(std::fs::create_dir_all(&parent).is_ok(), "socket parent");
    let retried = rig.launch_with_server(
        "socket",
        &destination.display().to_string(),
        &["--local", "repair-retry"],
    );
    assert_eq!(retried.0, Some(0), "{retried:?}");
    assert!(
        rig.meta("repair-retry")
            .contains(&format!("tmux_server={}\n", destination.display()))
    );
    rig.kill_server_at(&["-S", &destination.display().to_string()]);
}

#[test]
fn a_running_session_keeps_its_recorded_server_and_pair() {
    if skip() {
        return;
    }
    let rig = Rig::idle("repair-running");
    let first = rig.launch(&["--local", "repair-running"]);
    assert_eq!(first.0, Some(0), "{first:?}");
    let before = rig.meta("repair-running");
    let other = rig.scratch.join("unused-destination.sock");

    let resumed = rig.launch_with_server(
        "socket",
        &other.display().to_string(),
        &["--local", "repair-running"],
    );

    assert_eq!(resumed.0, Some(0), "{resumed:?}");
    assert!(rig.sessions().contains(&"repair-running".to_owned()));
    assert!(
        rig.sessions_on(&["-S", &other.display().to_string()])
            .is_empty(),
        "the proposed server stays cold"
    );
    assert_eq!(rig.meta("repair-running"), before);
}

#[test]
fn an_unknown_recorded_server_refuses_without_changing_meta() {
    if skip() {
        return;
    }
    let rig = Rig::idle("repair-unknown");
    let first = rig.launch(&["--local", "repair-unknown"]);
    assert_eq!(first.0, Some(0), "{first:?}");
    let before = rig.meta("repair-unknown");
    let other = rig.scratch.join("unknown-destination.sock");

    assert!(
        std::fs::set_permissions(&rig.sock, std::fs::Permissions::from_mode(0o000)).is_ok(),
        "make the recorded server unreachable, not absent"
    );

    let refused = rig.launch_with_server(
        "socket",
        &other.display().to_string(),
        &["--local", "repair-unknown"],
    );
    assert!(
        std::fs::set_permissions(&rig.sock, std::fs::Permissions::from_mode(0o600)).is_ok(),
        "restore the socket for rig cleanup"
    );

    assert_eq!(refused.0, Some(1), "{refused:?}");
    assert!(refused.2.contains("cannot verify"), "{refused:?}");
    assert_eq!(rig.meta("repair-unknown"), before);
}

#[test]
fn a_destination_namesake_refuses_before_the_old_pair_changes() {
    if skip() {
        return;
    }
    let rig = Rig::idle("repair-namesake");
    let first = rig.launch(&["--local", "repair-namesake"]);
    assert_eq!(first.0, Some(0), "{first:?}");
    let _keep = bare_session(&rig, &rig.sock, "keep-namesake-source");
    assert!(
        rig.tmux(&["kill-session", "-t", "=repair-namesake"]).0,
        "stop the recorded session"
    );
    let before = rig.meta("repair-namesake");
    let destination = rig.scratch.join("namesake-destination.sock");
    let _foreign = bare_session(&rig, &destination, "repair-namesake");

    let refused = rig.launch_with_server(
        "socket",
        &destination.display().to_string(),
        &["--local", "repair-namesake"],
    );

    assert_eq!(refused.0, Some(1), "{refused:?}");
    assert!(refused.2.contains("exists but is not an ae session"));
    assert_eq!(rig.meta("repair-namesake"), before);
    rig.kill_server_at(&["-S", &destination.display().to_string()]);
}

#[test]
fn an_owned_pre_pair_session_is_backfilled_on_historical_default() {
    if skip() {
        return;
    }
    let rig = Rig::idle("legacy-owned");
    let pane = bare_named_session(&rig, "default", "legacy-owned");
    let historical_socket = rig
        .tmux_named("default", &["display-message", "-p", "#{socket_path}"])
        .1
        .trim()
        .to_owned();
    set_named_session_env(&rig, "default", "legacy-owned", "AE_SESSION", "1");
    set_named_session_env(
        &rig,
        "default",
        "legacy-owned",
        "AE_HOME",
        &rig.home.display().to_string(),
    );
    let dir = rig.dir("legacy-owned");
    std::fs::create_dir_all(&dir).expect("legacy session dir");
    std::fs::write(
        dir.join("meta"),
        format!("schema=2\nsession=legacy-owned\nmain_pane={pane}\nmode=local\n"),
    )
    .expect("legacy meta");
    let proposed = rig.scratch.join("legacy-unused.sock");

    let resumed = rig.launch_with_server(
        "socket",
        &proposed.display().to_string(),
        &["--local", "legacy-owned"],
    );
    let meta = rig.meta("legacy-owned");

    assert_eq!(resumed.0, Some(0), "{resumed:?}");
    assert!(meta.contains("tmux_server_kind=socket\n"), "{meta}");
    assert!(
        meta.contains(&format!("tmux_server={historical_socket}\n")),
        "the backfill records the historical server's proven socket: {meta}"
    );
    assert!(
        rig.sessions_on(&["-S", &proposed.display().to_string()])
            .is_empty(),
        "the caller/proposed server gets no duplicate"
    );
    rig.kill_server_at(&["-L", "default"]);
}

#[test]
fn a_pre_pair_lookalike_from_another_home_is_refused_byte_for_byte() {
    if skip() {
        return;
    }
    let rig = Rig::idle("legacy-foreign");
    let pane = bare_named_session(&rig, "default", "legacy-foreign");
    let other_home = rig.scratch.join("other-home");
    std::fs::create_dir_all(&other_home).expect("foreign home");
    set_named_session_env(&rig, "default", "legacy-foreign", "AE_SESSION", "1");
    set_named_session_env(
        &rig,
        "default",
        "legacy-foreign",
        "AE_HOME",
        &other_home.display().to_string(),
    );
    let dir = rig.dir("legacy-foreign");
    std::fs::create_dir_all(&dir).expect("legacy session dir");
    let before = format!("schema=2\nsession=legacy-foreign\nmain_pane={pane}\nmode=local\n");
    std::fs::write(dir.join("meta"), &before).expect("legacy meta");

    let refused = rig.launch_with_server(
        "socket",
        &rig.scratch.join("unused.sock").display().to_string(),
        &["--local", "legacy-foreign"],
    );

    assert_eq!(refused.0, Some(1), "{refused:?}");
    assert!(refused.2.contains("could not prove"), "{refused:?}");
    assert_eq!(rig.meta("legacy-foreign"), before);
    rig.kill_server_at(&["-L", "default"]);
}

#[test]
fn attach_outside_tmux_uses_the_destination_server() {
    if skip() {
        return;
    }
    let rig = Rig::idle("attach-outside");
    let first = rig.launch(&["--local", "attach-outside"]);
    assert_eq!(first.0, Some(0), "{first:?}");

    let attached = rig.launch_attaching(&rig.sock, None, &["--local", "attach-outside"]);

    assert_ne!(
        attached.0,
        Some(0),
        "a non-terminal cannot attach: {attached:?}"
    );
    assert!(
        attached.1.is_empty(),
        "no foreign-server hint: {attached:?}"
    );
    assert!(
        !attached.2.contains("no sessions") && !attached.2.contains("can't find session"),
        "the attach reached the recorded destination: {attached:?}"
    );
}

#[test]
fn attach_inside_the_same_server_uses_switch_client() {
    if skip() {
        return;
    }
    let rig = Rig::idle("attach-same");
    let first = rig.launch(&["--local", "attach-same"]);
    assert_eq!(first.0, Some(0), "{first:?}");
    let caller = bare_session(&rig, &rig.sock, "same-server-caller");

    let switched = rig.launch_attaching(
        &rig.sock,
        Some((&rig.sock, &caller)),
        &["--local", "attach-same"],
    );

    assert!(
        switched.1.is_empty(),
        "no foreign-server hint: {switched:?}"
    );
    assert!(
        !switched.2.contains("can't find session") && !switched.2.contains("no sessions"),
        "switch-client reached the recorded destination: {switched:?}"
    );
}

#[test]
fn attach_inside_a_foreign_server_prints_the_destination_command() {
    if skip() {
        return;
    }
    let rig = Rig::idle("attach-foreign");
    let first = rig.launch(&["--local", "attach-foreign"]);
    assert_eq!(first.0, Some(0), "{first:?}");
    let foreign = rig.scratch.join("foreign.sock");
    let caller = bare_session(&rig, &foreign, "foreign-caller");

    let hinted = rig.launch_attaching(
        &rig.sock,
        Some((&foreign, &caller)),
        &["--local", "attach-foreign"],
    );

    assert_eq!(hinted.0, Some(0), "{hinted:?}");
    assert_eq!(
        hinted.1,
        format!(
            "Session 'attach-foreign' is on another tmux server. Attach with: tmux -S {} attach -t \"=attach-foreign\"\n",
            rig.sock.display()
        )
    );
    assert!(hinted.2.is_empty(), "{hinted:?}");
    rig.kill_server_at(&["-S", &foreign.display().to_string()]);
}

/// The whole local launch: session, pane, stamps, meta, helpers, launch script,
/// and an agent that actually started from the pasted script.
#[test]
fn a_local_launch_builds_the_whole_session() {
    if skip() {
        return;
    }
    let rig = Rig::new("local", &["claude"], None);
    let (code, stdout, stderr) = rig.launch(&["--local", "lnlocal"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    // The hint names a command that EXISTS.
    assert!(!stdout.contains("orchestrator --attach"), "{stdout}");

    // The SESSION and its stamped pane.
    let panes = rig.panes("lnlocal");
    let lead = panes
        .iter()
        .find(|(_, slot, _)| slot == "main")
        .unwrap_or_else(|| panic!("a stamped main pane: {panes:?}"));
    assert_eq!(lead.2, "lead", "the pane carries the bare v2 name");

    // The META, published as one v2 document.
    let meta = rig.meta("lnlocal");
    for row in [
        "session=lnlocal",
        "mode=local",
        "schema=2",
        "seat.main=lead",
        "profile.main=claude",
        "tmux_server_kind=socket",
        // THE SHAPE ROW, asserted on a meta a real launch published. A unit
        // test over the chain's own parser cannot see this: delete the row's
        // emission and every such test stays green while every new session
        // becomes unresumable, because a missing row IS the refusal.
        "meta_version=2",
    ] {
        assert!(meta.contains(row), "meta is missing {row}:\n{meta}");
    }
    assert!(
        meta.contains(&format!("work_dir={}", rig.project.display())),
        "local mode works in the caller's own directory:\n{meta}"
    );
    assert!(
        meta.contains("ae_core="),
        "the core is pinned per session:\n{meta}"
    );
    // The row the launch writes IS the version this core migrates to — spelled
    // from the module rather than from the literal above, so a bump to the
    // chain that forgets the writer fails here.
    assert!(
        meta.contains(&format!("meta_version={}", ae::migrate::CURRENT)),
        "the launch wrote a shape row this ae does not read:\n{meta}"
    );

    // The HELPERS, every one a LINK to the core this session is pinned to.
    let dir = rig.dir("lnlocal");
    let pinned = meta
        .lines()
        .find_map(|line| line.strip_prefix("ae_core="))
        .unwrap_or_default();
    assert!(!pinned.is_empty(), "the core pin is written:\n{meta}");
    for helper in ae::shim::HELPERS {
        let path = dir.join(helper.name);
        let kind = std::fs::symlink_metadata(&path)
            .unwrap_or_else(|why| panic!("the {} helper should exist: {why}", helper.name));
        assert!(kind.file_type().is_symlink(), "{} is a link", helper.name);
        assert_eq!(
            std::fs::read_link(&path).unwrap_or_default(),
            Path::new(pinned),
            "{} points at the pinned core",
            helper.name
        );
    }
    assert!(
        std::fs::read_to_string(dir.join("workspace.md"))
            .unwrap_or_default()
            .contains("lead"),
        "the manifest names the roster"
    );

    // NO LAUNCH SCRIPT: the pane's command is the core, and no shell file is
    // written into the session directory.
    assert!(
        !dir.join("launch.main.sh").exists(),
        "slice Z2 writes no bash into a session directory"
    );
    // The ARGV the agent actually got, and the same argv reported without a
    // pane.
    let argv = rig.launch_argv();
    assert!(
        dir.join("launch.main.started").is_file(),
        "`_run` marked the seat launched before becoming the tool"
    );
    assert!(
        argv.contains("--session-id"),
        "a fresh claude bakes its id: {argv}"
    );
    assert!(
        argv.contains("--append-system-prompt"),
        "the context rides claude's own channel: {argv}"
    );
    let plan = rig.plan("lnlocal", "main");
    assert!(
        plan.contains(r#""mode":"resume""#),
        "the seat has run once: {plan}"
    );
    assert!(plan.contains(r#""tool":"claude""#), "{plan}");
    assert!(
        plan.contains(r#""env_unset":["CLAUDECODE","CLAUDE_CODE_SESSION"]"#),
        "the nesting guard is an ENV delta now, not an `env` word in a shell string: {plan}"
    );
    assert!(
        plan.contains(r#""CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION":"0""#),
        "{plan}"
    );
}

/// A helper LINK really is the core: `state` writes the caller's declaration,
/// and `peek` reads a pane back — both through a file that is nothing but a
/// symlink to the binary answering.
#[test]
fn a_helper_link_is_the_core_with_its_own_session() {
    if skip() {
        return;
    }
    let rig = Rig::new("link", &["claude"], None);
    let (code, stdout, stderr) = rig.launch(&["--local", "lnlink"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let dir = rig.dir("lnlink");
    let lead = rig
        .panes("lnlink")
        .into_iter()
        .find(|(_, slot, _)| slot == "main")
        .map(|(pane, _, _)| pane)
        .unwrap_or_default();

    let out = helper(&dir.join("state"))
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &lead)
        .args(["working", "proving the link"])
        .output()
        .unwrap_or_else(|why| panic!("the state link should run: {why}"));
    assert!(
        out.status.success(),
        "state link: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        std::fs::read_to_string(dir.join("events.jsonl"))
            .unwrap_or_default()
            .contains("proving the link"),
        "the core wrote the declaration through the link"
    );

    // The agent draws its transcript when its own process gets there, which is
    // not when the launch returns.
    let mut seen = String::new();
    for _ in 0..200 {
        let out = helper(&dir.join("peek"))
            .env("TMUX", format!("{},0,0", rig.sock.display()))
            .env("TMUX_PANE", &lead)
            .args(["lead", "20"])
            .output()
            .unwrap_or_else(|why| panic!("the peek link should run: {why}"));
        assert!(
            out.status.success(),
            "peek link: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        seen = String::from_utf8_lossy(&out.stdout).into_owned();
        if seen.contains("fake agent transcript") {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("peek never read the pane back: {seen}");
}

/// A resume re-runs the SAME session with the resume variant, and does not
/// rebuild it.
#[test]
#[allow(clippy::too_many_lines, reason = "one end-to-end resume story")]
fn a_resume_reruns_with_the_resume_variant() {
    if skip() {
        return;
    }
    let rig = Rig::new("resume", &["claude"], None);
    let (code, stdout, stderr) = rig.launch(&["--local", "lnres"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    // Wait for the agent to actually be running before killing its session:
    // the seat is only "launched once" after `_run` has become the tool.
    assert!(
        !rig.launch_argv().is_empty(),
        "the pasted command started the agent"
    );
    assert!(
        rig.dir("lnres").join("launch.main.started").is_file(),
        "the first run marked the seat"
    );
    let fresh = rig.meta("lnres");
    let created = fresh
        .lines()
        .find_map(|line| line.strip_prefix("created="))
        .unwrap_or_default()
        .to_owned();
    let started = fresh
        .lines()
        .find_map(|line| line.strip_prefix("started="))
        .unwrap_or_default()
        .to_owned();
    assert!(
        created.parse::<i64>().is_ok_and(|epoch| epoch > 0),
        "fresh launch records created: {fresh}"
    );
    assert_eq!(started, created, "one instant owns both first-launch rows");
    let sid = fresh
        .lines()
        .find_map(|line| line.strip_prefix("harness_session.main="))
        .unwrap_or_default()
        .to_owned();
    assert!(!sid.is_empty(), "claude's id is known upfront:\n{fresh}");
    let config_home_row = fresh
        .lines()
        .find_map(|line| line.strip_prefix("config_home.main="))
        .unwrap_or_default()
        .to_owned();
    let config_home = config_home_row
        .strip_prefix("implicit:")
        .unwrap_or_default()
        .to_owned();
    let config_home_base_row = fresh
        .lines()
        .find_map(|line| line.strip_prefix("config_home_base.main="))
        .unwrap_or_default()
        .to_owned();
    assert_eq!(
        Path::new(&config_home),
        std::fs::canonicalize(rig.scratch.join(".claude"))
            .expect("canonical default")
            .as_path(),
        "first start pins its canonical store: {fresh}"
    );
    assert_eq!(
        Path::new(&config_home_base_row),
        std::fs::canonicalize(&rig.scratch)
            .expect("canonical HOME")
            .as_path(),
        "first start pins the HOME that selected its implicit store: {fresh}"
    );

    // The session stops; its state stays.
    assert!(
        rig.tmux(&["kill-session", "-t", "lnres"]).0,
        "the kill lands"
    );
    ae::meta::rewrite(&rig.dir("lnres"), "started", Some("1"))
        .expect("fixture makes the prior started value observable");

    let (code, stdout, stderr) = rig.launch(&["--local", "lnres"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("Resuming session lnres"),
        "the resume announces itself: {stdout}"
    );
    let resumed_meta = rig.meta("lnres");
    assert!(
        resumed_meta.contains(&format!("created={created}\n")),
        "resume preserves first creation: {resumed_meta}"
    );
    assert!(
        resumed_meta
            .lines()
            .find_map(|line| line.strip_prefix("started="))
            .is_some_and(|value| value != "1" && value.parse::<i64>().is_ok_and(|epoch| epoch > 1)),
        "resume refreshes started: {resumed_meta}"
    );
    // THE RESUME DECISION IS THE CORE'S NOW, and it is the start marker plus a
    // probe rather than a shell `if` in a generated script.
    let plan = rig.plan("lnres", "main");
    assert!(plan.contains(r#""mode":"resume""#), "{plan}");
    assert!(
        plan.contains(r#""--continue""#) && !plan.contains(&format!(r#""--resume","{sid}""#)),
        "no transcript for this id, so the fallback: {plan}"
    );

    // Plant the transcript claude would have written, and the same seat resumes
    // the SAME conversation. Under the RIG'S home, so nothing is written into
    // the developer's own `~/.claude/projects`.
    let home = rig.scratch.display().to_string();
    // The PHYSICAL path, because the probe asks `getcwd(2)` — which is what
    // claude's own `process.cwd()` asks, and on macOS `/tmp` is a symlink.
    let key: String = std::fs::canonicalize(&rig.project)
        .unwrap_or_else(|_| rig.project.clone())
        .display()
        .to_string()
        .chars()
        .map(|ch| if ch == '/' { '-' } else { ch })
        .collect();
    let transcripts = Path::new(&home).join(".claude/projects").join(key);
    assert!(
        std::fs::create_dir_all(&transcripts).is_ok(),
        "a transcript dir"
    );
    let transcript = transcripts.join(format!("{sid}.jsonl"));
    assert!(std::fs::write(&transcript, "{}\n").is_ok(), "a transcript");
    let plan = rig.plan("lnres", "main");
    let _ = std::fs::remove_file(&transcript);
    assert!(
        plan.contains(&format!(r#""--resume","{sid}""#)),
        "the resume asks for the SAME conversation: {plan}"
    );

    assert!(
        rig.meta("lnres")
            .contains(&format!("harness_session.main={sid}")),
        "the id survives the resume"
    );
    assert!(
        rig.meta("lnres")
            .contains(&format!("config_home.main={config_home_row}\n")),
        "the recorded config home survives the full meta rebuild"
    );
    assert!(
        rig.meta("lnres")
            .contains(&format!("config_home_base.main={config_home_base_row}\n")),
        "the recorded implicit HOME survives the full meta rebuild"
    );
    assert!(
        !rig.dir("lnres").join("launch.main.sh").exists(),
        "and no bash was written to decide any of it"
    );
}

#[test]
fn a_legacy_resume_promotes_the_main_marker_to_created() {
    if skip() {
        return;
    }
    let rig = Rig::new("legacy-created", &["claude"], None);
    let (code, stdout, stderr) = rig.launch(&["--local", "lnlegacycreated"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(!rig.launch_argv().is_empty(), "the agent started");
    let marker_created = std::fs::metadata(rig.dir("lnlegacycreated").join("launch.main.started"))
        .expect("the original marker remains")
        .mtime();
    assert!(
        rig.tmux(&["kill-session", "-t", "lnlegacycreated"]).0,
        "the kill lands"
    );
    ae::meta::rewrite(&rig.dir("lnlegacycreated"), "created", None)
        .expect("fixture makes this a legacy meta");

    let (code, stdout, stderr) = rig.launch(&["--local", "lnlegacycreated"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let promoted = rig
        .meta("lnlegacycreated")
        .lines()
        .find_map(|line| line.strip_prefix("created="))
        .and_then(|epoch| epoch.parse::<i64>().ok());
    assert_eq!(promoted, Some(marker_created), "legacy marker is promoted");
}

#[test]
fn seat_profile_flags_refuse_unknown_words_before_session_state_is_written() {
    if skip() {
        return;
    }
    let rig = Rig::new("seat-unknown", &["claude", "codex"], None);

    let (code, stdout, stderr) = rig.launch(&["--local", "lnbadagent", "--seat", "ghost=claude"]);
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("unknown launch agent 'ghost'") && stderr.contains("Known agents: lead"),
        "{stderr}"
    );
    assert!(
        !rig.dir("lnbadagent").exists(),
        "unknown agent wrote session state"
    );
    assert!(!rig.home.exists(), "unknown agent wrote under AE_HOME");

    let (code, stdout, stderr) = rig.launch(&["--local", "lnbadprofile", "--lead", "missing"]);
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("unknown profile 'missing'")
            && stderr.contains("Known profiles: claude, codex"),
        "{stderr}"
    );
    assert!(
        !rig.dir("lnbadprofile").exists(),
        "unknown profile wrote session state"
    );
    assert!(!rig.home.exists(), "unknown profile wrote under AE_HOME");
}

#[test]
fn a_fresh_home_validates_seat_profiles_against_the_default_before_seeding_it() {
    if skip() {
        return;
    }
    let rig = Rig::new("seat-fresh-default", &["claude", "codex"], None);
    assert!(
        std::fs::remove_file(&rig.config).is_ok(),
        "config starts absent"
    );

    let (code, stdout, stderr) = rig.launch(&["--local", "lnfreshbad", "--lead", "missing"]);
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("unknown profile 'missing'")
            && stderr.contains("Known profiles:")
            && stderr.contains("fable5"),
        "the embedded defaults decide the refusal: {stderr}"
    );
    assert!(!rig.home.exists(), "an invalid override writes no state");
    assert!(!rig.config.exists(), "an invalid override seeds no config");

    let mut command = rig.launch_command_with_server(
        "socket",
        &rig.sock.display().to_string(),
        &["--local", "lnfreshgood", "--lead", "fable5"],
    );
    command.env(
        "PATH",
        format!(
            "{}:{}",
            rig.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    let output = command
        .output()
        .unwrap_or_else(|why| panic!("the fresh launch should run: {why}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&rig.config).unwrap_or_default(),
        ae::entry::DEFAULT_CONFIG,
        "validation and seeding use the same snapshot"
    );
    assert!(
        rig.meta("lnfreshgood").contains("profile.main=fable5\n"),
        "the validated default override reaches meta"
    );
}

#[test]
fn a_seat_profile_override_is_persisted_and_restored() {
    if skip() {
        return;
    }
    let rig = Rig::new("seat-roundtrip", &["claude"], None);
    let binary = rig.scratch.join("bin/claude");
    add_profile(
        &rig,
        "altclaude",
        &format!("{} --served alt", binary.display()),
    );

    let (code, stdout, stderr) = rig.launch(&["--local", "lnseatround", "--lead", "altclaude"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(!rig.launch_argv().is_empty(), "the first agent started");
    let fresh = rig.meta("lnseatround");
    assert!(fresh.contains("profile.main=altclaude\n"), "{fresh}");
    assert!(fresh.contains("agent_bin.main=claude\n"), "{fresh}");
    assert!(
        rig.plan("lnseatround", "main")
            .contains(r#""--served","alt""#)
    );

    assert!(
        rig.tmux(&["kill-session", "-t", "=lnseatround"]).0,
        "the session stops"
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "lnseatround"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let resumed = rig.meta("lnseatround");
    assert!(resumed.contains("profile.main=altclaude\n"), "{resumed}");
    assert!(
        rig.plan("lnseatround", "main")
            .contains(r#""--served","alt""#),
        "the stored profile chooses the resumed command"
    );
}

#[test]
fn solo_overrides_configured_workers_and_the_frozen_roster_stays_solo() {
    if skip() {
        return;
    }
    let rig = Rig::new("solo-override", &[], None);
    assert!(
        std::fs::write(&rig.config, SOLO_OVERRIDE_CONFIG).is_ok(),
        "a configured lead pair"
    );

    let (code, stdout, stderr) =
        rig.launch(&["--local", "lnsolooverride", "--solo", "--lead", "solx"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let fresh = rig.meta("lnsolooverride");
    assert!(fresh.contains("seat.main=lead\n"), "{fresh}");
    assert!(fresh.contains("profile.main=solx\n"), "{fresh}");
    assert!(!fresh.contains("seat.worker."), "{fresh}");
    assert_eq!(
        rig.windows("lnsolooverride"),
        vec![("0".to_owned(), "lead".to_owned(), 1)]
    );

    let dir = rig.dir("lnsolooverride");
    let context = ae::render::context_document(
        &dir,
        "lnsolooverride",
        &rig.project.display().to_string(),
        "main",
        std::slice::from_ref(&rig.config),
    );
    assert!(context.contains("LEAD ROLE"), "{context}");
    assert!(!context.contains("LEADERSHIP PEER"), "{context}");
    assert!(!context.contains("one of two EQUAL leads"), "{context}");
    let manifest = std::fs::read_to_string(dir.join("workspace.md")).unwrap_or_default();
    assert!(
        manifest.contains("| lead | solx | tail | lead |"),
        "{manifest}"
    );
    assert!(!manifest.contains("| colead |"), "{manifest}");
    assert!(!manifest.contains("| builder |"), "{manifest}");

    assert!(
        rig.tmux(&["kill-session", "-t", "=lnsolooverride"]).0,
        "the solo session stops"
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "lnsolooverride"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let resumed = rig.meta("lnsolooverride");
    assert!(resumed.contains("seat.main=lead\n"), "{resumed}");
    assert!(resumed.contains("profile.main=solx\n"), "{resumed}");
    assert!(!resumed.contains("seat.worker."), "{resumed}");
    assert_eq!(
        rig.windows("lnsolooverride"),
        vec![("0".to_owned(), "lead".to_owned(), 1)]
    );
    rig.wait_for_pane_command("lnsolooverride", "main", "tail");
}

#[test]
fn solo_refuses_worker_overrides_and_a_workerful_resume() {
    if skip() {
        return;
    }
    let rig = Rig::new("solo-refusals", &[], None);
    assert!(
        std::fs::write(&rig.config, SOLO_OVERRIDE_CONFIG).is_ok(),
        "a configured lead pair"
    );
    for (name, flag, value) in [
        ("lnsolocolead", "--colead", "solx"),
        ("lnsoloseat", "--seat", "colead=solx"),
        ("lnsolobuilder", "--seat", "builder=solx"),
    ] {
        let (code, stdout, stderr) = rig.launch(&["--local", name, "--solo", flag, value]);
        assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
        assert_eq!(
            stderr,
            "Error: '--solo' starts the lead alone; drop --colead/--seat <worker>.\n"
        );
        assert!(!rig.dir(name).exists(), "a refused launch wrote state");
    }

    let (code, stdout, stderr) = rig.launch(&[
        "--local",
        "lnsolouse",
        "use",
        "colead",
        "--solo",
        "--seat",
        "colead=solx",
    ]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let selected_main = rig.meta("lnsolouse");
    assert!(
        selected_main.contains("seat.main=colead\n"),
        "{selected_main}"
    );
    assert!(
        selected_main.contains("profile.main=solx\n"),
        "{selected_main}"
    );
    assert!(!selected_main.contains("seat.worker."), "{selected_main}");

    let (code, stdout, stderr) = rig.launch(&["--local", "lnsoloresume"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.tmux(&["kill-session", "-t", "=lnsoloresume"]).0,
        "the lead pair stops"
    );
    let before = rig.meta("lnsoloresume");
    let (code, stdout, stderr) = rig.launch(&["--local", "lnsoloresume", "--solo"]);
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        stderr,
        "Error: 'lnsoloresume' already has workers (colead, builder); '--solo' applies to a first launch.\n"
    );
    assert_eq!(rig.meta("lnsoloresume"), before, "meta is untouched");
    assert!(
        !rig.sessions().contains(&"lnsoloresume".to_owned()),
        "nothing resumed"
    );
}

#[test]
fn doubtful_solo_meta_refuses_before_configured_workers_can_grow_the_roster() {
    if skip() {
        return;
    }
    let rig = Rig::new("solo-doubtful", &[], None);
    assert!(
        std::fs::write(&rig.config, SOLO_OVERRIDE_CONFIG).is_ok(),
        "a config whose workers must not repair doubtful meta"
    );

    for (name, damage, reason) in [
        ("lnsolomalformed", "malformed", "malformed meta line"),
        ("lnsolomissing", "missing-profile", "missing profile.main"),
    ] {
        let (code, stdout, stderr) = rig.launch(&["--local", name, "--solo"]);
        assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
        assert!(
            rig.tmux(&["kill-session", "-t", &format!("={name}")]).0,
            "the solo session stops"
        );

        let meta_path = rig.dir(name).join("meta");
        let original = std::fs::read_to_string(&meta_path).unwrap_or_default();
        let damaged = if damage == "malformed" {
            format!("{original}not-a-meta-row\n")
        } else {
            original.replace("profile.main=idle\n", "")
        };
        assert_ne!(damaged, original, "the fixture damage is real");
        assert!(
            std::fs::write(&meta_path, &damaged).is_ok(),
            "publish the doubtful meta"
        );

        let (code, stdout, stderr) = rig.launch(&["--local", name]);
        assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
        assert!(stderr.contains(reason), "{stderr}");
        assert!(stderr.contains("ae doctor"), "{stderr}");
        assert_eq!(
            std::fs::read_to_string(&meta_path).unwrap_or_default(),
            damaged,
            "a refusal preserves the doubtful bytes"
        );
        assert!(rig.windows(name).is_empty(), "no windows were recreated");
        assert!(rig.panes(name).is_empty(), "no panes were recreated");
    }
}

#[test]
fn a_hostile_config_home_row_refuses_a_stopped_resume_without_rewriting_meta() {
    if skip() {
        return;
    }
    for (tag, session, damage) in [
        ("bad-config-home", "lnbadconfighome", "relative"),
        ("dup-config-home", "lndupconfighome", "duplicate"),
        (
            "missing-config-home-base",
            "lnmissconfighome",
            "missing-base",
        ),
    ] {
        let rig = Rig::new(tag, &["claude"], None);
        let (code, stdout, stderr) = rig.launch(&["--local", session]);
        assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
        assert!(!rig.launch_argv().is_empty(), "the first agent started");
        assert!(
            rig.tmux(&["kill-session", "-t", &format!("={session}")]).0,
            "the session stops"
        );
        let path = rig.dir(session).join("meta");
        let original = std::fs::read_to_string(&path).expect("meta");
        let damaged = if damage == "duplicate" {
            format!("{original}config_home.main=/second\n")
        } else {
            original
                .lines()
                .map(|line| {
                    if damage == "relative" && line.starts_with("config_home.main=") {
                        "config_home.main=relative"
                    } else if damage == "missing-base" && line.starts_with("config_home_base.main=")
                    {
                        ""
                    } else {
                        line
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        };
        std::fs::write(&path, &damaged).expect("hostile meta");
        let (code, stdout, stderr) = rig.launch(&["--local", session]);
        assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
        assert!(
            stderr.contains("doubtful roster metadata")
                && stderr.contains("config_home.main")
                && stderr.contains("ae doctor"),
            "{stderr}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), damaged);
        assert!(!rig.sessions().contains(&session.to_owned()));
    }
}

#[test]
fn a_stopped_seat_can_repair_within_its_tool_kind_and_keeps_its_conversation() {
    if skip() {
        return;
    }
    let rig = Rig::new("seat-repair", &["claude"], None);
    let binary = rig.scratch.join("bin/claude");
    add_profile(
        &rig,
        "otherclaude",
        &format!("{} --model other", binary.display()),
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "lnseatrepair"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(!rig.launch_argv().is_empty(), "the first agent started");
    let before = rig.meta("lnseatrepair");
    let sid = before
        .lines()
        .find_map(|line| line.strip_prefix("harness_session.main="))
        .unwrap_or_default()
        .to_owned();
    assert!(!sid.is_empty(), "{before}");
    assert!(
        rig.tmux(&["kill-session", "-t", "=lnseatrepair"]).0,
        "the session stops"
    );

    let (code, stdout, stderr) =
        rig.launch(&["--local", "lnseatrepair", "--seat", "lead=otherclaude"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let resumed = rig.meta("lnseatrepair");
    assert!(resumed.contains("profile.main=otherclaude\n"), "{resumed}");
    assert!(resumed.contains("agent_bin.main=claude\n"), "{resumed}");
    assert!(
        resumed.contains(&format!("harness_session.main={sid}\n")),
        "the recorded conversation follows the same harness: {resumed}"
    );
    assert!(
        rig.plan("lnseatrepair", "main")
            .contains(r#""--model","other""#),
        "the resumed command uses the replacement profile"
    );
}

#[test]
fn a_stopped_seat_refuses_a_profile_from_another_tool_kind() {
    if skip() {
        return;
    }
    let rig = Rig::new("seat-cross-tool", &["claude", "codex"], None);
    let (code, stdout, stderr) = rig.launch(&["--local", "lnseatcross"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(!rig.launch_argv().is_empty(), "the first agent started");
    assert!(
        rig.tmux(&["kill-session", "-t", "=lnseatcross"]).0,
        "the session stops"
    );
    let before = rig.meta("lnseatcross");

    let (code, stdout, stderr) = rig.launch(&["--local", "lnseatcross", "--lead", "codex"]);
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("cannot change agent 'lead' from claude to codex")
            && stderr.contains("cannot cross tool kinds"),
        "{stderr}"
    );
    assert_eq!(rig.meta("lnseatcross"), before, "meta is untouched");
    assert!(
        !rig.sessions().contains(&"lnseatcross".to_owned()),
        "nothing resumed"
    );
}

#[test]
fn a_running_session_refuses_seat_profile_flags_and_tells_the_user_to_stop() {
    if skip() {
        return;
    }
    let rig = Rig::new("seat-running", &["claude"], None);
    let binary = rig.scratch.join("bin/claude");
    add_profile(
        &rig,
        "otherclaude",
        &format!("{} --model other", binary.display()),
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "lnseatrunning"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let before = rig.meta("lnseatrunning");

    let (code, stdout, stderr) =
        rig.launch(&["--local", "lnseatrunning", "--seat", "lead=otherclaude"]);
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains("is running; stop it before"), "{stderr}");
    assert_eq!(rig.meta("lnseatrunning"), before, "meta is untouched");
    assert!(
        rig.sessions().contains(&"lnseatrunning".to_owned()),
        "the running session remains"
    );
}

#[test]
fn concurrent_resume_loser_refuses_instead_of_dropping_its_override() {
    if skip() {
        return;
    }
    let rig = Rig::new("seat-concurrent", &["claude"], None);
    let binary = rig.scratch.join("bin/claude");
    add_profile(
        &rig,
        "otherclaude",
        &format!("{} --model other", binary.display()),
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "lnseatconcurrent"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(!rig.launch_argv().is_empty(), "the first agent started");
    assert!(
        rig.tmux(&["kill-session", "-t", "=lnseatconcurrent"]).0,
        "the session stops"
    );

    let lock_path = rig
        .home
        .join("sessions")
        .join(".lifecycle.lnseatconcurrent.lock");
    let held = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&lock_path)
        .expect("the lifecycle lock opens");
    held.try_lock()
        .expect("the fixture holds the lifecycle lock");
    let mut first = rig.launch_child(&["--local", "lnseatconcurrent", "--lead", "otherclaude"]);
    let mut second = rig.launch_child(&["--local", "lnseatconcurrent", "--lead", "otherclaude"]);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        matches!(first.try_wait(), Ok(None)) && matches!(second.try_wait(), Ok(None)),
        "both resumes wait behind the fixture lock"
    );
    drop(held);

    let outputs = [first, second].map(|child| {
        child
            .wait_with_output()
            .unwrap_or_else(|why| panic!("the resume should finish: {why}"))
    });
    let mut codes = outputs
        .iter()
        .map(|output| output.status.code())
        .collect::<Vec<_>>();
    codes.sort_unstable();
    assert_eq!(codes, [Some(0), Some(2)], "one resume wins: {codes:?}");
    let loser = outputs
        .iter()
        .find(|output| output.status.code() == Some(2))
        .expect("one loser");
    let stderr = String::from_utf8_lossy(&loser.stderr);
    assert!(
        stderr.contains(
            "session 'lnseatconcurrent' is running; stop it before changing a seat profile"
        ),
        "the loser refuses the override explicitly: {stderr}"
    );
}

#[test]
fn a_config_swap_while_resume_waits_cannot_change_the_preflighted_command() {
    if skip() {
        return;
    }
    let rig = Rig::new("seat-config-swap", &["claude", "codex"], None);
    let claude = rig.scratch.join("bin/claude");
    let codex = rig.scratch.join("bin/codex");
    let old_command = format!("{} --model old", claude.display());
    let swapped_command = format!("{} --model swapped", codex.display());
    add_profile(&rig, "repair", &old_command);
    let (code, stdout, stderr) = rig.launch(&["--local", "lnseatcfgswap"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(!rig.launch_argv().is_empty(), "the first agent started");
    assert!(
        rig.tmux(&["kill-session", "-t", "=lnseatcfgswap"]).0,
        "the session stops"
    );
    let _ = std::fs::remove_file(&rig.launched);

    let lock_path = rig
        .home
        .join("sessions")
        .join(".lifecycle.lnseatcfgswap.lock");
    let held = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&lock_path)
        .expect("the lifecycle lock opens");
    held.try_lock()
        .expect("the fixture holds the lifecycle lock");
    let mut child =
        rig.launch_child_with_pre_lock_marker(&["--local", "lnseatcfgswap", "--lead", "repair"]);
    rig.wait_for_pre_lock_marker();
    assert!(
        matches!(child.try_wait(), Ok(None)),
        "the resume waits after preflight"
    );
    let config = std::fs::read_to_string(&rig.config).unwrap_or_default();
    assert!(
        std::fs::write(&rig.config, config.replace(&old_command, &swapped_command)).is_ok(),
        "the config swaps while the launch waits"
    );
    drop(held);

    let output = child
        .wait_with_output()
        .unwrap_or_else(|why| panic!("the resume should finish: {why}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let launched = rig.launch_argv();
    assert!(launched.contains("--model old"), "{launched}");
    assert!(!launched.contains("swapped"), "{launched}");
    assert!(
        rig.meta("lnseatcfgswap")
            .contains("agent_bin.main=claude\n"),
        "the validated tool kind reaches meta"
    );
}

#[test]
fn a_spawned_seat_launches_its_preflighted_command_after_a_config_swap() {
    if skip() {
        return;
    }
    let rig = Rig::new("spawned-config-swap", &["claude"], None);
    let claude = rig.scratch.join("bin/claude");
    let repair_command = format!("{} --model repaired", claude.display());
    let old_command = format!("{} --model spawned-old", claude.display());
    let swapped_command = format!("{} --model spawned-swapped", claude.display());
    add_profile(&rig, "repair", &repair_command);
    add_profile(&rig, "spawned", &old_command);

    let session = "lnspawnedcfgswap";
    let _keep = bare_session(&rig, &rig.sock, &format!("keep-{session}"));
    let dir = rig.dir(session);
    assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
    assert!(
        std::fs::write(
            dir.join("meta"),
            format!(
                "meta_version={version}\nsession={session}\ntmux_server_kind=socket\n\
                 tmux_server={server}\nmode=local\nlayout=vertical\n\
                 work_dir={project}\norigin={project}\nschema=2\nseat.main=lead\n\
                 profile.main=claude\nagent_bin.main=claude\nseat.spawned.0=helper\n\
                 profile.spawned.0=spawned\nagent_bin.spawned.0=claude\n",
                version = ae::migrate::CURRENT,
                server = rig.sock.display(),
                project = rig.project.display(),
            ),
        )
        .is_ok(),
        "a stopped session with a spawned seat"
    );

    let lock_path = rig
        .home
        .join("sessions")
        .join(format!(".lifecycle.{session}.lock"));
    let held = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&lock_path)
        .expect("the lifecycle lock opens");
    held.try_lock()
        .expect("the fixture holds the lifecycle lock");
    let mut child =
        rig.launch_child_with_pre_lock_marker(&["--local", session, "--lead", "repair"]);
    rig.wait_for_pre_lock_marker();
    assert!(
        matches!(child.try_wait(), Ok(None)),
        "the resume waits after preflight"
    );
    let config = std::fs::read_to_string(&rig.config).unwrap_or_default();
    assert!(
        std::fs::write(&rig.config, config.replace(&old_command, &swapped_command)).is_ok(),
        "the spawned profile swaps while the launch waits"
    );
    drop(held);

    let output = child
        .wait_with_output()
        .unwrap_or_else(|why| panic!("the resume should finish: {why}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let launched = rig.launch_argv();
    assert!(launched.contains("--model spawned-old"), "{launched}");
    assert!(!launched.contains("spawned-swapped"), "{launched}");
}

#[allow(
    clippy::too_many_lines,
    reason = "one live assertion of the capability-canonical server binding set"
)]
fn assert_ae_status_bindings(rig: &Rig) {
    let (_, keys) = rig.tmux(&["list-keys", "-T", "root"]);
    for key in [
        "MouseDown3Pane",
        "M-MouseDown3Pane",
        "MouseDown3StatusLeft",
        "M-MouseDown3Status",
        "M-MouseDown3StatusLeft",
    ] {
        assert!(
            !keys
                .lines()
                .any(|line| line.starts_with(&format!("bind-key  -T root {key} "))),
            "the ae-owned server removes tmux's stock right-click menu for {key}: {keys}"
        );
    }
    let binding = |key: &str| {
        let prefix = format!("bind-key  -T root {key} ");
        keys.lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("one {key} binding: {keys}"))
    };
    let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(rig.sock.clone()));
    let menu_mouse = ae::transport::observe_tmux_floor(&server).menu_mouse();
    let down_click = binding("MouseDown1Status");
    assert!(
        down_click.contains("run-shell -C")
            && down_click.contains("#{||:#{==:#{mouse_status_range},ae}")
            && down_click.contains("#{==:#{mouse_status_range},ae-more}")
            && !down_click.contains("@ae_orchestrator_id")
            && down_click.contains("mouse_status_range},window")
            && down_click.contains("mouse_status_range},session")
            && down_click.contains("select-window -t #{window_id}")
            && down_click.contains("switch-client -c #{q:client_name} -t #{session_id}"),
        "MouseDown1Status keeps strip navigation: {down_click}"
    );
    let picker_click = if menu_mouse {
        assert!(
            !keys.contains("bind-key  -T root MouseUp1Status ")
                && !keys.contains("bind-key  -T root MouseUp3Status "),
            "mouse-aware servers clear stale Up bindings: {keys}"
        );
        down_click
    } else {
        assert!(
            !down_click.contains("orchestrator"),
            "tmux 3.4 must not open twice: {down_click}"
        );
        binding("MouseUp1Status")
    };
    assert!(
        picker_click.contains("orchestrator")
            && picker_click.contains("--popup")
            && picker_click.contains("--client")
            && picker_click.contains("#{q:client_name}")
            && picker_click.contains(&format!("AE_HOME={}", rig.home.display()))
            && picker_click.contains(&format!("CONFIG_FILE={}", rig.config.display()))
            && picker_click.contains(&format!("AE_TMUX_SERVER={}", rig.sock.display()))
            && picker_click.contains("AE_TMUX_SERVER_KIND=socket")
            && picker_click.contains(env!("CARGO_BIN_EXE_ae")),
        "the capability-selected picker binding carries its checkout namespace: {picker_click}"
    );
    let down_menu = binding("MouseDown3Status");
    let menu = if menu_mouse {
        down_menu
    } else {
        assert!(
            !down_menu.contains("orchestrator") && !down_menu.contains("display-menu"),
            "tmux 3.4 MouseDown3Status must be a no-op: {down_menu}"
        );
        binding("MouseUp3Status")
    };
    for needle in [
        "#{||:#{==:#{mouse_status_range},ae}",
        "mouse_status_range},ae-more",
        "orchestrator",
        "--client",
        "display-menu",
        "{mouse}",
        "Flip lead/colead panes",
        "window_panes",
        "window_zoomed_flag",
        "swap-pane -d",
    ] {
        assert!(menu.contains(needle), "missing {needle:?}: {menu}");
    }
    assert_eq!(
        keys.lines()
            .filter(|line| {
                [
                    "MouseDown1Status",
                    "MouseDown3Status",
                    "MouseUp1Status",
                    "MouseUp3Status",
                ]
                .iter()
                .any(|key| line.starts_with(&format!("bind-key  -T root {key} ")))
            })
            .count(),
        if menu_mouse { 2 } else { 4 },
        "one canonical binding set for the server capability: {keys}"
    );
    let (_, prefix) = rig.tmux(&["list-keys", "-T", "prefix"]);
    let hotkey = prefix
        .lines()
        .find(|line| line.contains("-T prefix a "))
        .unwrap_or_else(|| panic!("one prefix a binding: {prefix}"));
    assert!(
        hotkey.contains("run-shell -b")
            && hotkey.contains("orchestrator")
            && hotkey.contains("--popup")
            && hotkey.contains("--client")
            && hotkey.contains("#{q:client_name}")
            && hotkey.contains(&format!("AE_HOME={}", rig.home.display()))
            && hotkey.contains(&format!("CONFIG_FILE={}", rig.config.display())),
        "the hotkey carries the checkout picker namespace: {hotkey}"
    );
}

/// Every entry into a session returns the client to its lead pane. The hook is
/// session-scoped through the lead pane id, so a resume installs the new id and
/// a rename keeps the old one.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one launch-reattach-resume-rename focus contract"
)]
fn the_session_focus_hook_follows_the_lead_through_resume_and_rename() {
    if skip() {
        return;
    }
    let rig = Rig::idle("focus");
    let (code, stdout, stderr) = rig.launch(&["--local", "lnfocus"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    assert_ae_status_bindings(&rig);

    let main_pane = || {
        rig.panes("lnfocus")
            .into_iter()
            .find(|(_, slot, _)| slot == "main")
            .map_or_else(|| panic!("a stamped main pane"), |(pane, _, _)| pane)
    };
    let assert_hook = |name: &str, pane: &str| {
        let target = format!("={name}:");
        let (_, hooks) = rig.tmux(&["show-hooks", "-t", &target]);
        let (_, session_id) = rig.tmux(&["display-message", "-p", "-t", &target, "#{session_id}"]);
        let command = format!("select-window -t {pane} ; select-pane -t {pane}");
        let predicate = format!("#{{==:#{{session_id}},{}}}", session_id.trim());
        assert_eq!(
            hooks
                .lines()
                .filter(|line| {
                    line.starts_with("client-session-changed[0] ")
                        && line.contains(&predicate)
                        && line.contains(&command)
                })
                .count(),
            1,
            "one lead focus hook on {name}: {hooks}"
        );
        let (_, global) = rig.tmux(&["show-hooks", "-g"]);
        assert!(
            !global.lines().any(|line| line.contains(&command)),
            "the lead focus hook is not global: {global}"
        );
    };

    let first_pane = main_pane();
    assert_hook("lnfocus", &first_pane);
    let (_, published) = rig.tmux(&[
        "show-options",
        "-v",
        "-t",
        "=lnfocus:",
        ae::theme::MAIN_PANE_OPTION,
    ]);
    assert_eq!(
        published.trim(),
        first_pane,
        "the launch publishes its lead"
    );

    assert!(
        rig.tmux(&[
            "set-hook",
            "-t",
            &first_pane,
            "client-session-changed",
            &format!("select-window -t {first_pane} ; select-pane -t {first_pane}"),
        ])
        .0,
        "plant the unguarded pre-release focus hook"
    );
    assert!(
        rig.tmux(&[
            "bind-key",
            "-T",
            "root",
            "MouseDown1Status",
            "switch-client",
            "-t",
            "="
        ])
        .0,
        "restore tmux's default to model a server launched before this release"
    );
    assert!(
        rig.tmux(&[
            "bind-key",
            "-T",
            "root",
            "MouseDown3Status",
            "display-message",
            "pre-release"
        ])
        .0,
        "plant a pre-release right-click binding"
    );
    for key in ["MouseUp1Status", "MouseUp3Status"] {
        assert!(
            rig.tmux(&[
                "bind-key",
                "-T",
                "root",
                key,
                "display-message",
                "stale-release"
            ])
            .0,
            "plant stale {key} binding"
        );
    }
    let (code, stdout, stderr) = rig.launch(&["--local", "lnfocus"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("is running"),
        "the live reattach branch: {stdout}"
    );
    assert_hook("lnfocus", &first_pane);
    assert_ae_status_bindings(&rig);

    assert!(
        rig.tmux(&["kill-session", "-t", "=lnfocus"]).0,
        "stop the session before resume"
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "lnfocus"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let resumed_pane = main_pane();
    assert_hook("lnfocus", &resumed_pane);
    let (_, published) = rig.tmux(&[
        "show-options",
        "-v",
        "-t",
        "=lnfocus:",
        ae::theme::MAIN_PANE_OPTION,
    ]);
    assert_eq!(
        published.trim(),
        resumed_pane,
        "the resume republishes its proven lead"
    );
    assert_ae_status_bindings(&rig);

    let renamed = ae()
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("TMUX_TMPDIR", &rig.scratch)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args([ae::cli::RENAME, "lnfocus", "lnrenamed"])
        .output()
        .unwrap_or_else(|why| panic!("the rename should run: {why}"));
    assert_eq!(renamed.status.code(), Some(0), "rename failed: {renamed:?}");
    assert_hook("lnrenamed", &resumed_pane);
}

/// An ambient server may carry the user's own root table. A launch there keeps
/// tmux's default status click instead of replacing a server-global binding.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one ambient server owns both Down and Up status bindings plus the hotkey"
)]
fn an_ambient_launch_does_not_replace_mouse_down_status() {
    if skip() {
        return;
    }
    let rig = Rig::idle("focus-ambient");
    let ambient = |words: &[&str]| {
        run_tmux(
            &words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>(),
            &rig.scratch,
        )
    };
    assert!(
        ambient(&["new-session", "-d", "-s", "ambient-owner", "sleep", "600"]).0,
        "the user's ambient server starts"
    );
    let (listed, stock_keys) = ambient(&["list-keys", "-T", "root"]);
    assert!(listed, "the user's untouched root table: {stock_keys}");
    let assert_stock_menus = |keys: &str| {
        for key in [
            "MouseDown3Pane",
            "M-MouseDown3Pane",
            "MouseDown3StatusLeft",
            "M-MouseDown3Status",
            "M-MouseDown3StatusLeft",
        ] {
            let stock = keys
                .lines()
                .find(|line| line.starts_with(&format!("bind-key  -T root {key} ")))
                .unwrap_or_else(|| panic!("tmux's stock right-click binding for {key}: {keys}"));
            assert!(
                stock.contains("display-menu"),
                "tmux's stock right-click binding still opens its menu: {stock}"
            );
        }
    };
    assert_stock_menus(&stock_keys);
    let (code, stdout, stderr) =
        rig.launch_with_server("", "", &["--local", "lnfocus-ambient-stock"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (listed, post_launch_stock_keys) = ambient(&["list-keys", "-T", "root"]);
    assert!(
        listed,
        "the ambient root table after launch: {post_launch_stock_keys}"
    );
    assert_stock_menus(&post_launch_stock_keys);
    assert!(
        ambient(&[
            "bind-key",
            "-T",
            "root",
            "MouseDown1Status",
            "display-message",
            "ambient-owned"
        ])
        .0,
        "the user owns the ambient binding"
    );
    assert!(
        ambient(&[
            "bind-key",
            "-T",
            "root",
            "MouseDown3Status",
            "display-message",
            "ambient-menu"
        ])
        .0,
        "the user owns the ambient context-menu binding"
    );
    for (key, message) in [
        ("MouseUp1Status", "ambient-up-click"),
        ("MouseUp3Status", "ambient-up-menu"),
    ] {
        assert!(
            ambient(&["bind-key", "-T", "root", key, "display-message", message]).0,
            "the user owns ambient {key}"
        );
    }
    assert!(
        ambient(&[
            "bind-key",
            "-T",
            "prefix",
            "a",
            "display-message",
            "ambient-hotkey"
        ])
        .0,
        "the user owns the ambient prefix binding"
    );
    let (code, stdout, stderr) = rig.launch_with_server("", "", &["--local", "lnfocus-ambient"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (listed, keys) = ambient(&["list-keys", "-T", "root"]);
    assert!(listed, "the ambient server's root table: {keys}");
    let click = keys
        .lines()
        .find(|line| line.starts_with("bind-key  -T root MouseDown1Status "))
        .unwrap_or_else(|| panic!("tmux's MouseDown1Status binding remains: {keys}"));
    assert!(
        click.contains("display-message ambient-owned") && !click.contains("if-shell"),
        "an ambient launch leaves the server-global click binding alone: {click}"
    );
    let menu = keys
        .lines()
        .find(|line| line.starts_with("bind-key  -T root MouseDown3Status "))
        .unwrap_or_else(|| panic!("tmux's MouseDown3Status binding remains: {keys}"));
    assert!(
        menu.contains("display-message ambient-menu") && !menu.contains("display-menu"),
        "an ambient launch leaves the server-global menu binding alone: {menu}"
    );
    for (key, message) in [
        ("MouseUp1Status", "ambient-up-click"),
        ("MouseUp3Status", "ambient-up-menu"),
    ] {
        let line = keys
            .lines()
            .find(|line| line.starts_with(&format!("bind-key  -T root {key} ")))
            .unwrap_or_else(|| panic!("ambient {key} remains: {keys}"));
        assert!(
            line.contains(&format!("display-message {message}")),
            "ambient {key} changed: {line}"
        );
    }
    let (_, prefix) = ambient(&["list-keys", "-T", "prefix"]);
    let hotkey = prefix
        .lines()
        .find(|line| line.contains("-T prefix a "))
        .unwrap_or_default();
    assert!(
        hotkey.contains("display-message ambient-hotkey") && !hotkey.contains("orchestrator"),
        "an ambient launch leaves the server-global hotkey alone: {hotkey}"
    );
}

/// `--worktree` creates a real git worktree; a launch that cannot build its
/// session directory tears the tmux session down again.
#[test]
fn worktree_mode_creates_its_copy_and_a_failed_launch_rolls_back() {
    if skip() {
        return;
    }
    let rig = Rig::new("wt", &["claude"], None);
    // A real repository, so `worktree add` has something to detach from.
    git_in(&rig.project, &["init", "-q"]);
    assert!(std::fs::write(rig.project.join("f"), "x").is_ok());
    git_in(&rig.project, &["add", "-A"]);
    git_in(&rig.project, &["commit", "-qm", "base"]);

    let (code, stdout, stderr) = rig.launch(&["--worktree", "lnwt"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let work = rig.home.join("worktrees").join("lnwt");
    assert!(work.join("f").is_file(), "the worktree carries the commit");
    let meta = rig.meta("lnwt");
    assert!(meta.contains("mode=git"), "{meta}");
    assert!(
        meta.contains(&format!("work_dir={}", work.display())),
        "agents run in the worktree:\n{meta}"
    );
    assert!(
        meta.contains("git_base_commit="),
        "the base commit is recorded once, at birth:\n{meta}"
    );

    // ROLLBACK: a session directory that cannot be created (its path is a
    // FILE) fails after the tmux session exists, and the session must not
    // survive as debris the next launch would read as healthy.
    let sessions = rig.home.join("sessions");
    assert!(std::fs::write(sessions.join("lnbad"), "not a directory").is_ok());
    let (code, stdout, stderr) = rig.launch(&["--local", "lnbad"]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    let (_, alive) = rig.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(
        !alive.lines().any(|line| line == "lnbad"),
        "the failed launch took its tmux session with it: {alive}"
    );
}

/// LIVE-rename production history: a live rename changes session identity,
/// not the working copy's identity — the managed path deliberately stays put
/// (stopped renames move it; see the Slice-A stopped tests). A renamed live
/// session resumes from the recorded directory and replaces its existing meta
/// instead of taking the new name's worktree path as proof that this is a
/// fresh launch.
#[test]
fn a_renamed_worktree_resume_uses_its_recorded_dir_and_existing_meta() {
    if skip() {
        return;
    }
    let rig = Rig::idle("renamed-worktree-resume");
    git_in(&rig.project, &["init", "-q"]);
    assert!(std::fs::write(rig.project.join("f"), "x").is_ok());
    git_in(&rig.project, &["add", "-A"]);
    git_in(&rig.project, &["commit", "-qm", "base"]);

    let old = "lnwtold";
    let new = "lnwtnew";
    let (code, stdout, stderr) = rig.launch(&["--worktree", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let recorded_work = rig.home.join("worktrees").join(old);
    assert!(recorded_work.join("f").is_file(), "the original worktree");

    let renamed = ae()
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("TMUX_TMPDIR", &rig.scratch)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args([ae::cli::RENAME, old, new])
        .output()
        .unwrap_or_else(|why| panic!("the rename should run: {why}"));
    assert_eq!(renamed.status.code(), Some(0), "rename failed: {renamed:?}");
    assert!(
        rig.tmux(&["kill-session", "-t", &format!("={new}")]).0,
        "stop the renamed session"
    );

    let before = rig.meta(new);
    let new_work = rig.home.join("worktrees").join(new);
    assert!(!new_work.exists(), "rename does not move the working copy");
    let (code, stdout, stderr) = rig.launch(&[new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("Resuming session lnwtnew"), "{stdout}");
    assert!(!stdout.contains("Creating git worktree"), "{stdout}");
    assert!(
        !new_work.exists(),
        "resume must not create a second name-derived worktree"
    );
    let after = rig.meta(new);
    assert!(
        before.contains(&format!("work_dir={}", recorded_work.display()))
            && after.contains(&format!("work_dir={}", recorded_work.display())),
        "the stored working directory survives replacement:\n{after}"
    );
    let (_, pane_dir) = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        &format!("={new}:0.0"),
        "#{pane_current_path}",
    ]);
    let actual = std::fs::canonicalize(pane_dir.trim()).unwrap_or_default();
    let expected = std::fs::canonicalize(&recorded_work).unwrap_or(recorded_work);
    assert_eq!(actual, expected, "the resumed agent's cwd");
}

/// Legacy compatibility: a resume whose recorded copy is gone refuses rather
/// than adopting a name-derived copy. Kept as the production-history case
/// for the retained-path era; stopped renames move the recorded copy instead
/// of stranding it (see the Slice-A stopped tests).
#[test]
fn a_resume_with_a_recorded_missing_copy_never_adopts_a_name_derived_copy() {
    if skip() {
        return;
    }
    let rig = Rig::idle("missing-renamed-copy");
    assert!(std::fs::write(rig.project.join("f"), "x").is_ok());

    let old = "lncopyold";
    let new = "lncopynew";
    let (code, stdout, stderr) = rig.launch(&["--copy", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let recorded_work = rig.home.join("worktrees").join(old);

    let renamed = ae()
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("TMUX_TMPDIR", &rig.scratch)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args([ae::cli::RENAME, old, new])
        .output()
        .unwrap_or_else(|why| panic!("the rename should run: {why}"));
    assert_eq!(renamed.status.code(), Some(0), "rename failed: {renamed:?}");
    assert!(
        rig.tmux(&["kill-session", "-t", &format!("={new}")]).0,
        "stop the renamed session"
    );
    assert!(
        std::fs::remove_dir_all(&recorded_work).is_ok(),
        "remove the recorded copy"
    );
    let name_derived = rig.home.join("worktrees").join(new);
    assert!(
        std::fs::create_dir_all(&name_derived).is_ok(),
        "plant an unrelated name-derived copy"
    );
    let unrelated = name_derived.join("unrelated");
    assert!(
        std::fs::write(&unrelated, "not this session").is_ok(),
        "mark the unrelated copy"
    );
    let before = rig.meta(new);

    let (code, stdout, stderr) = rig.launch(&[new]);

    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        stderr,
        format!(
            "Error: '{new}' records its working copy at {} but it is gone — restore it or end the session (ae end {new})\n",
            recorded_work.display()
        )
    );
    assert!(
        !rig.sessions().iter().any(|session| session == new),
        "the refusal must precede tmux creation"
    );
    assert_eq!(
        rig.meta(new),
        before,
        "the refusal leaves meta byte-identical"
    );
    assert_eq!(
        std::fs::read_to_string(unrelated).unwrap_or_default(),
        "not this session",
        "the refusal leaves the namesake copy untouched"
    );
}

/// Run one PUBLIC command (`stop`, `rename`, `end`) against the rig's home
/// with no calling pane — the operator outside every session.
fn public(rig: &Rig, args: &[&str]) -> (Option<i32>, String, String) {
    let out = ae()
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("TMUX_TMPDIR", &rig.scratch)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args(args)
        .output()
        .unwrap_or_else(|why| panic!("the public command should run: {why}"));
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A1: a stopped local rename converges to the new name and stays stopped —
/// no live session, no monitor start — preserving the UUID, every other meta
/// row byte for byte, the event log and the helper links. A same-command
/// retry reads the durable result instead of duplicating it. Smallest
/// defeating mutation: keep the stopped refusal in `locked`.
#[test]
fn a_stopped_local_rename_converges_to_the_new_name() {
    if skip() {
        return;
    }
    let rig = Rig::idle("stopped-local");
    let old = "slocold";
    let new = "slocnew";
    let (code, stdout, stderr) = rig.launch(&["--local", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        !rig.sessions().iter().any(|session| session == old),
        "the session is stopped"
    );

    let uuid = rig
        .meta(old)
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .unwrap_or_default()
        .to_owned();
    assert!(!uuid.is_empty(), "the launch recorded a UUID");
    let meta_before = rig.meta(old);
    let events_before = std::fs::read(rig.dir(old).join("events.jsonl")).unwrap_or_default();
    let send_before = std::fs::read_link(rig.dir(old).join("send")).unwrap_or_default();

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!("Renamed '{old}' → '{new}' (stopped;")),
        "one stopped-state result: {stdout}"
    );
    // Local mode keeps the caller-owned cwd: no provider-portability warning.
    assert!(stderr.is_empty(), "no warning on a local rename: {stderr}");

    // Stays stopped: no live session, no monitor start.
    assert!(
        !rig.sessions().iter().any(|session| session == new),
        "no live session under either name"
    );
    assert!(!rig.dir(old).exists(), "the old address is gone");
    assert!(rig.dir(new).is_dir(), "the new address holds the state");

    // Identity: the UUID survives; every meta row but `session` is identical.
    let meta_after = rig.meta(new);
    let row = |meta: &str, key: &str| {
        meta.lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(row(&meta_after, "session"), new, "{meta_after}");
    assert_eq!(row(&meta_after, "session_id"), uuid, "{meta_after}");
    let kept_before: Vec<&str> = meta_before
        .lines()
        .filter(|line| !line.starts_with("session="))
        .collect();
    let kept_after: Vec<&str> = meta_after
        .lines()
        .filter(|line| !line.starts_with("session="))
        .collect();
    assert_eq!(
        kept_after, kept_before,
        "every other meta row is byte-identical"
    );
    assert_eq!(
        std::fs::read(rig.dir(new).join("events.jsonl")).unwrap_or_default(),
        events_before,
        "the event log survives byte-identical"
    );
    assert_eq!(
        std::fs::read_link(rig.dir(new).join("send")).unwrap_or_default(),
        send_before,
        "helper links still point at the same core"
    );
    let workspace = std::fs::read_to_string(rig.dir(new).join("workspace.md")).unwrap_or_default();
    assert!(
        workspace.contains(new),
        "the manifest names the new session"
    );
    assert!(
        workspace.contains(&rig.dir(new).display().to_string()),
        "the manifest names the new address"
    );

    // The durable result: a same-command retry succeeds without duplicating.
    let intent = rig
        .home
        .join("sessions")
        .join(format!(".rename.{old}.{new}.intent"));
    let intent_body = std::fs::read_to_string(&intent).unwrap_or_default();
    assert!(
        intent_body.contains("phase=complete"),
        "the completed intent is the durable result: {intent_body}"
    );
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!("Renamed '{old}' → '{new}' (stopped;")),
        "the retry prints the same result: {stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(&intent).unwrap_or_default(),
        intent_body,
        "the retry appends no duplicate result"
    );
}

/// Git canonicalizes symlinked ancestors when it prints a registration
/// (a symlinked TMPDIR prints resolved) while the meta records the launch
/// spelling: either spelling proves the spot.
fn porcelain_has(porcelain: &str, work: &Path) -> bool {
    if porcelain.contains(&work.display().to_string()) {
        return true;
    }
    std::fs::canonicalize(work)
        .is_ok_and(|canonical| porcelain.contains(&canonical.display().to_string()))
}

/// Capture the identity witnesses a managed move must preserve: marker
/// bytes, `(device, inode)` for the work dir and every marker, HEAD, the
/// porcelain registration, the worktree-side `.git` pointer (byte-stable:
/// `git worktree move` keeps the admin dir name) and the admin-side `gitdir`
/// file (which the move repoints). Measured against git 2.55.0.
struct WorkWitness {
    work_dev: u64,
    work_ino: u64,
    markers: Vec<(String, u64, u64, Vec<u8>)>,
    head: String,
    status: String,
    porcelain: String,
    git_pointer: String,
    admin_gitdir: String,
    uuid: String,
}

#[allow(
    clippy::expect_used,
    reason = "fixture setup: a witness that cannot be read must fail loudly, like the #[test] caller it feeds"
)]
fn capture_work_witness(
    rig: &Rig,
    session: &str,
    work: &Path,
    origin: &Path,
    admin: &str,
) -> WorkWitness {
    let meta = std::fs::metadata(work).expect("the work dir");
    let mut markers = Vec::new();
    for name in ["untracked", "dirty", "ignored"] {
        let path = work.join(format!("witness_{name}"));
        let file = std::fs::metadata(&path).expect("a witness file");
        markers.push((
            name.to_owned(),
            file.dev(),
            file.ino(),
            std::fs::read(&path).unwrap_or_default(),
        ));
    }
    let link = std::fs::symlink_metadata(work.join("witness_link")).expect("a witness link");
    assert!(link.file_type().is_symlink(), "the link stays a link");
    WorkWitness {
        work_dev: meta.dev(),
        work_ino: meta.ino(),
        markers,
        head: git_in(work, &["rev-parse", "HEAD"]),
        status: git_in(work, &["status", "--porcelain"]),
        porcelain: git_in(origin, &["worktree", "list", "--porcelain"]),
        git_pointer: std::fs::read_to_string(work.join(".git")).unwrap_or_default(),
        admin_gitdir: std::fs::read_to_string(
            origin.join(".git/worktrees").join(admin).join("gitdir"),
        )
        .unwrap_or_default(),
        uuid: rig
            .meta(session)
            .lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .unwrap_or_default()
            .to_owned(),
    }
}

/// A1: a stopped git rename moves the managed worktree — never recreating
/// it — preserving marker bytes, device/inode identity, HEAD/status, the
/// administrative registration and the UUID; production-launch resume reuses
/// the moved worktree; private end tears it down coherently. Smallest
/// defeating mutations: skip the worktree move but rewrite the session
/// (registration/end fails); recreate the worktree or copy bytes into fresh
/// files at the same HEAD (the device/inode/admin witness fails).
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one ordered move/resume/teardown proof across launch, stop, rename, resume and end"
)]
fn a_stopped_git_rename_moves_the_managed_worktree() {
    if skip() {
        return;
    }
    let rig = Rig::idle("stopped-git");
    git_in(&rig.project, &["init", "-q"]);
    git_in(&rig.project, &["config", "user.email", "t@t"]);
    git_in(&rig.project, &["config", "user.name", "t"]);
    assert!(std::fs::write(rig.project.join("f"), "x\n").is_ok());
    git_in(&rig.project, &["add", "-A"]);
    git_in(&rig.project, &["commit", "-qm", "base"]);
    assert!(std::fs::write(rig.project.join(".gitignore"), "witness_ignored\n").is_ok());
    git_in(&rig.project, &["add", "-A"]);
    git_in(&rig.project, &["commit", "-qm", "ignore"]);

    let old = "sgitold";
    let new = "sgitnew";
    let (code, stdout, stderr) = rig.launch(&["--worktree", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join(old);
    let new_work = rig.home.join("worktrees").join(new);

    // Markers the move must carry as the SAME files, not the same bytes.
    let tag = std::process::id().to_string();
    assert!(
        std::fs::write(
            old_work.join("witness_untracked"),
            format!("untracked-{tag}\n")
        )
        .is_ok()
    );
    assert!(std::fs::write(old_work.join("witness_dirty"), "dirty\n").is_ok());
    git_in(&old_work, &["add", "-A"]);
    assert!(std::fs::write(old_work.join("f"), "dirty-worktree\n").is_ok());
    assert!(std::fs::write(old_work.join("witness_ignored"), format!("ignored-{tag}\n")).is_ok());
    std::os::unix::fs::symlink("f", old_work.join("witness_link")).expect("a witness link");
    let before = capture_work_witness(&rig, old, &old_work, &rig.project, old);
    assert!(!before.uuid.is_empty(), "the launch recorded a UUID");
    assert!(
        porcelain_has(&before.porcelain, &old_work) && !porcelain_has(&before.porcelain, &new_work),
        "old registered, new absent: {}",
        before.porcelain
    );

    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let meta_before = rig.meta(old);
    let events_before = std::fs::read(rig.dir(old).join("events.jsonl")).unwrap_or_default();
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!(
            "Renamed '{old}' → '{new}' (stopped; managed work moved to '{}')",
            new_work.display()
        )),
        "{stdout}"
    );
    // Implicit config homes move with a warning, not a provider probe: the
    // later resume re-proves the conversation.
    assert!(
        stderr.contains("implicit or unclassified config homes"),
        "the implicit warning separates rename from resume: {stderr}"
    );
    assert!(!old_work.exists(), "the old work path is gone");
    assert!(new_work.is_dir(), "the new work path holds the move");

    // The SAME files: device/inode and bytes for the work dir and markers.
    let after = capture_work_witness(&rig, new, &new_work, &rig.project, old);
    assert_eq!(
        (after.work_dev, after.work_ino),
        (before.work_dev, before.work_ino)
    );
    assert_eq!(after.markers, before.markers, "markers are the same files");
    assert_eq!(after.head, before.head, "HEAD survives");
    assert_eq!(
        after.status, before.status,
        "dirty/untracked/ignored state survives"
    );
    assert_eq!(after.uuid, before.uuid, "the UUID survives");
    assert!(
        porcelain_has(&after.porcelain, &new_work) && !porcelain_has(&after.porcelain, &old_work),
        "registration moved exactly once: {}",
        after.porcelain
    );
    assert_eq!(
        after.git_pointer, before.git_pointer,
        "the same admin, not a recreated worktree"
    );
    assert!(
        after.admin_gitdir.contains(&new_work.display().to_string())
            || std::fs::canonicalize(&new_work).is_ok_and(|canonical| {
                after
                    .admin_gitdir
                    .contains(&canonical.display().to_string())
            }),
        "the admin gitdir file follows the move: {}",
        after.admin_gitdir
    );
    let meta_after = rig.meta(new);
    assert!(
        meta_after.contains(&format!("work_dir={}", new_work.display()))
            && meta_after.contains(&format!("session={new}\n")),
        "{meta_after}"
    );
    let kept_before: Vec<&str> = meta_before
        .lines()
        .filter(|line| !line.starts_with("session=") && !line.starts_with("work_dir="))
        .collect();
    let kept_after: Vec<&str> = meta_after
        .lines()
        .filter(|line| !line.starts_with("session=") && !line.starts_with("work_dir="))
        .collect();
    assert_eq!(
        kept_after, kept_before,
        "every other meta row is byte-identical"
    );
    assert_eq!(
        std::fs::read(rig.dir(new).join("events.jsonl")).unwrap_or_default(),
        events_before,
        "the event log survives byte-identical"
    );

    // Production-launch resume reuses the moved worktree — address checks
    // alone (path/HEAD equality) never prove this; the witness does.
    let (code, stdout, stderr) = rig.launch(&[new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!("Resuming session {new}")),
        "{stdout}"
    );
    assert!(!stdout.contains("Creating git worktree"), "{stdout}");
    let resumed = capture_work_witness(&rig, new, &new_work, &rig.project, old);
    assert_eq!(
        (resumed.work_dev, resumed.work_ino),
        (before.work_dev, before.work_ino)
    );
    assert_eq!(
        resumed.markers, before.markers,
        "resume keeps the same files"
    );
    assert!(
        porcelain_has(&resumed.porcelain, &new_work),
        "{}",
        resumed.porcelain
    );

    // Private end stays coherent with the moved registration: with no origin
    // remote the worktree is committed and preserved (B3 durability), and the
    // teardown's worktrees/<name> containment accepts the moved address.
    let (code, stdout, stderr) = public(&rig, &["end", new, "-f", "--keep-history"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!("Directory preserved: {}", new_work.display())),
        "{stdout}"
    );
    assert!(
        new_work.is_dir()
            && porcelain_has(
                &git_in(&rig.project, &["worktree", "list", "--porcelain"]),
                &new_work
            ),
        "the moved worktree is preserved and still registered"
    );
    assert!(!rig.dir(new).exists(), "teardown removed the state");
}

/// A1: a stopped full-copy rename moves the managed copy as the same
/// directory (same device/inode, symlinks intact), preserves the recorded
/// quota pin (OFF stays OFF through rename and the later resume), and resumes
/// through the production launcher. Smallest defeating mutation: copy bytes
/// into fresh files instead of moving the directory (inode witness fails).
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one ordered copy/preserve/resume proof across launch, stop, rename and resume"
)]
fn a_stopped_full_rename_moves_the_managed_copy() {
    if skip() {
        return;
    }
    let rig = Rig::idle("stopped-full");
    assert!(
        std::fs::write(&rig.config, format!("{IDLE_CONFIG}quota = off\n")).is_ok(),
        "a quota-off idle config"
    );
    assert!(std::fs::write(rig.project.join("f"), "x\n").is_ok());

    let old = "sfullold";
    let new = "sfullnew";
    let (code, stdout, stderr) = rig.launch(&["--copy", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join(old);
    let new_work = rig.home.join("worktrees").join(new);
    assert!(
        rig.meta(old).contains("quota=off"),
        "the launch pinned quota off: {}",
        rig.meta(old)
    );

    let tag = std::process::id().to_string();
    assert!(std::fs::write(old_work.join("witness_untracked"), format!("u-{tag}\n")).is_ok());
    assert!(std::fs::write(old_work.join("f"), "dirty-copy\n").is_ok());
    std::os::unix::fs::symlink("f", old_work.join("witness_link")).expect("a witness link");
    let id_of =
        |path: &Path| std::fs::metadata(path).map_or((0, 0), |meta| (meta.dev(), meta.ino()));
    let work_id_before = id_of(&old_work);
    let untracked_before = (
        id_of(&old_work.join("witness_untracked")),
        std::fs::read(old_work.join("witness_untracked")).unwrap_or_default(),
    );

    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!(
            "Renamed '{old}' → '{new}' (stopped; managed work moved to '{}')",
            new_work.display()
        )),
        "{stdout}"
    );
    assert!(!old_work.exists(), "the old copy path is gone");
    assert_eq!(
        id_of(&new_work),
        work_id_before,
        "the same directory, moved"
    );
    assert_eq!(
        (
            id_of(&new_work.join("witness_untracked")),
            std::fs::read(new_work.join("witness_untracked")).unwrap_or_default()
        ),
        untracked_before,
        "markers are the same files"
    );
    assert_eq!(
        std::fs::read(new_work.join("f")).unwrap_or_default(),
        b"dirty-copy\n",
        "dirty bytes survive"
    );
    assert_eq!(
        std::fs::read_link(new_work.join("witness_link")).unwrap_or_default(),
        PathBuf::from("f"),
        "the symlink survives with its target"
    );
    assert!(
        rig.meta(new).contains("quota=off"),
        "the quota pin survives the rename: {}",
        rig.meta(new)
    );
    let workspace = std::fs::read_to_string(rig.dir(new).join("workspace.md")).unwrap_or_default();
    assert!(
        !workspace.to_lowercase().contains("quota"),
        "OFF removes quota words from the republished manifest"
    );

    // Production-launch resume reuses the moved copy and keeps the pin.
    let (code, stdout, stderr) = rig.launch(&[new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!("Resuming session {new}")),
        "{stdout}"
    );
    assert!(!stdout.contains("Creating full copy"), "{stdout}");
    assert_eq!(
        id_of(&new_work),
        work_id_before,
        "resume keeps the same files"
    );
    assert!(
        rig.meta(new).contains("quota=off"),
        "ordinary resume keeps the rename-preserved pin: {}",
        rig.meta(new)
    );
}

/// Spawn `ae rename old new` with the crash seam armed and read stderr lines
/// on a thread. Returns the child and the line receiver; the caller waits for
/// the attestation, then kills or lets the ceiling expire.
#[allow(
    clippy::expect_used,
    reason = "fixture setup: a child that cannot start or pipe must fail loudly, like the #[test] caller it feeds"
)]
fn crash_child(
    rig: &Rig,
    old: &str,
    new: &str,
    boundary: &str,
) -> (OwnedChild, mpsc::Receiver<Option<String>>) {
    let mut command = ae();
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("TMUX_TMPDIR", &rig.scratch)
        .env("AE_TEST_RENAME_CRASH_AT", boundary)
        .arg(ae::cli::RENAME)
        .arg(old)
        .arg(new);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|why| panic!("the ae binary should start: {why}"));
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stderr);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(None);
                    break;
                }
                Ok(_) => {
                    if tx.send(Some(line)).is_err() {
                        break;
                    }
                }
            }
        }
    });
    (child, rx)
}

/// Wait for the exact crash attestation, then SIGKILL the parked child.
/// Returns stderr through the attestation line. The kill models process death
/// past committed facts; elapsed time never selects an early cut.
#[allow(
    clippy::expect_used,
    reason = "fixture setup: a child that cannot be killed must fail loudly, like the #[test] caller it feeds"
)]
fn kill_at_boundary(rig: &Rig, old: &str, new: &str, boundary: &str) -> String {
    let (mut child, rx) = crash_child(rig, old, new, boundary);
    let wanted = format!("rename-crash-boundary: {boundary}\n");
    let mut captured = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(line)) => {
                captured.push_str(&line);
                if line == wanted {
                    break;
                }
            }
            Ok(None) => panic!("the child exited before attesting {boundary}: {captured}"),
            Err(_) => {
                assert!(
                    std::time::Instant::now() <= deadline,
                    "no attestation for {boundary} within 30s: {captured}"
                );
                assert!(
                    child.try_wait().is_ok_and(|exited| exited.is_none()),
                    "the child exited before attesting {boundary}: {captured}"
                );
            }
        }
    }
    child.kill().expect("the parked child dies on SIGKILL");
    let _ = child.wait();
    captured
}

/// Retry the same rename unarmed: the recorded transaction must converge to
/// the one stopped-state success line.
fn retry_rename(rig: &Rig, old: &str, new: &str) -> (String, String) {
    let (code, stdout, stderr) = public(rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!("Renamed '{old}' → '{new}' (stopped;")),
        "the retry converges: {stdout}"
    );
    (stdout, stderr)
}

/// Launch an idle local session and publicly stop it: the standard stopped
/// source every crash cut starts from.
fn stopped_local_source(tag: &str, old: &str) -> Rig {
    let rig = Rig::idle(tag);
    let (code, stdout, stderr) = rig.launch(&["--local", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    rig
}

/// A2: killing past the intent attestation leaves the durable intent and no
/// move; the retry converges. Smallest defeating mutation: publish the first
/// move before the intent (no recoverable record at the cut).
#[test]
fn crash_after_intent_recovers_from_the_durable_record() {
    if skip() {
        return;
    }
    let old = "cintold";
    let new = "cintnew";
    let rig = stopped_local_source("crash-intent", old);
    let uuid = rig
        .meta(old)
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .unwrap_or_default()
        .to_owned();

    kill_at_boundary(&rig, old, new, "after-intent");
    let intent = std::fs::read_to_string(
        rig.home
            .join("sessions")
            .join(format!(".rename.{old}.{new}.intent")),
    )
    .unwrap_or_default();
    assert!(intent.contains("phase=prepared"), "{intent}");
    assert!(intent.contains(&format!("session_id={uuid}")), "{intent}");
    assert!(rig.dir(old).is_dir(), "no state move yet");
    assert!(!rig.dir(new).exists(), "no state move yet");

    retry_rename(&rig, old, new);
    assert!(!rig.dir(old).exists());
    assert_eq!(
        rig.meta(new)
            .lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .unwrap_or_default(),
        uuid
    );
}

/// A2: killing past the state-move attestation leaves the state moved with
/// the old meta rows; the retry publishes coherence without touching work.
/// Smallest defeating mutation: advance the phase before the directory lands.
#[test]
fn crash_after_state_move_recovers_coherence() {
    if skip() {
        return;
    }
    let old = "cstold";
    let new = "cstnew";
    let rig = stopped_local_source("crash-state", old);

    kill_at_boundary(&rig, old, new, "after-state-move");
    assert!(!rig.dir(old).exists(), "the state moved");
    assert!(rig.dir(new).is_dir(), "the state landed");
    let meta = rig.meta(new);
    assert!(
        meta.contains(&format!("session={old}\n")),
        "meta still old:\n{meta}"
    );
    let intent = std::fs::read_to_string(
        rig.home
            .join("sessions")
            .join(format!(".rename.{old}.{new}.intent")),
    )
    .unwrap_or_default();
    assert!(intent.contains("phase=state-moved"), "{intent}");

    retry_rename(&rig, old, new);
    assert!(rig.meta(new).contains(&format!("session={new}\n")));
}

/// A2: killing past the meta attestation leaves coherent meta with unchecked
/// assets; the retry finishes assets and the result. Smallest defeating
/// mutation: attest the meta cut before the replacement reads back.
#[test]
fn crash_after_meta_recovers_assets_and_result() {
    if skip() {
        return;
    }
    let old = "cmetaold";
    let new = "cmetanew";
    let rig = stopped_local_source("crash-meta", old);

    kill_at_boundary(&rig, old, new, "after-meta");
    let meta = rig.meta(new);
    assert!(
        meta.contains(&format!("session={new}\n")),
        "meta coherent:\n{meta}"
    );
    let intent = std::fs::read_to_string(
        rig.home
            .join("sessions")
            .join(format!(".rename.{old}.{new}.intent")),
    )
    .unwrap_or_default();
    assert!(intent.contains("phase=meta-published"), "{intent}");

    retry_rename(&rig, old, new);
    let workspace = std::fs::read_to_string(rig.dir(new).join("workspace.md")).unwrap_or_default();
    assert!(workspace.contains(new), "assets published on retry");
}

/// A2: killing past the assets attestation leaves everything but the result;
/// the retry publishes the result once. Smallest defeating mutation: append
/// the completion before the assets verify.
#[test]
fn crash_after_assets_recovers_only_the_result() {
    if skip() {
        return;
    }
    let old = "cassetsold";
    let new = "cassetsnew";
    let rig = stopped_local_source("crash-assets", old);

    kill_at_boundary(&rig, old, new, "after-assets");
    let intent = std::fs::read_to_string(
        rig.home
            .join("sessions")
            .join(format!(".rename.{old}.{new}.intent")),
    )
    .unwrap_or_default();
    assert!(intent.contains("phase=assets-published"), "{intent}");
    let workspace = std::fs::read_to_string(rig.dir(new).join("workspace.md")).unwrap_or_default();
    assert!(workspace.contains(new), "assets already published");

    retry_rename(&rig, old, new);
    let intent = std::fs::read_to_string(
        rig.home
            .join("sessions")
            .join(format!(".rename.{old}.{new}.intent")),
    )
    .unwrap_or_default();
    assert!(intent.contains("phase=complete"), "{intent}");
}

/// A2: killing past the result attestation leaves the durable completion;
/// the retry reads the same result and appends no duplicate. This is a
/// durability receipt, not a sixth recovery case. Smallest defeating
/// mutation: publish (or duplicate) the completion on retry.
#[test]
fn crash_after_result_is_durable_and_idempotent() {
    if skip() {
        return;
    }
    let old = "cresold";
    let new = "cresnew";
    let rig = stopped_local_source("crash-result", old);

    kill_at_boundary(&rig, old, new, "after-result");
    let intent_path = rig
        .home
        .join("sessions")
        .join(format!(".rename.{old}.{new}.intent"));
    let intent_body = std::fs::read_to_string(&intent_path).unwrap_or_default();
    assert!(intent_body.contains("phase=complete"), "{intent_body}");

    let (stdout, _) = retry_rename(&rig, old, new);
    assert_eq!(
        std::fs::read_to_string(&intent_path).unwrap_or_default(),
        intent_body,
        "the retry appends no duplicate result"
    );
    let (stdout2, _) = retry_rename(&rig, old, new);
    assert_eq!(stdout2, stdout, "completions read identical");
}

/// A2: killing past the work-move attestation leaves the managed work moved
/// with the state still old; the retry moves the state and converges.
/// Smallest defeating mutation: emit the work-move boundary before the
/// supported move reports completion.
#[test]
fn crash_after_work_move_recovers_state_and_meta() {
    if skip() {
        return;
    }
    let rig = Rig::idle("crash-work");
    git_in(&rig.project, &["init", "-q"]);
    git_in(&rig.project, &["config", "user.email", "t@t"]);
    git_in(&rig.project, &["config", "user.name", "t"]);
    assert!(std::fs::write(rig.project.join("f"), "x\n").is_ok());
    git_in(&rig.project, &["add", "-A"]);
    git_in(&rig.project, &["commit", "-qm", "base"]);

    let old = "cworkold";
    let new = "cworknew";
    let (code, stdout, stderr) = rig.launch(&["--worktree", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join(old);
    let new_work = rig.home.join("worktrees").join(new);
    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, old, new, "after-work-move");
    assert!(!old_work.exists(), "the managed work moved");
    assert!(new_work.is_dir(), "the managed work landed");
    assert!(rig.dir(old).is_dir(), "the state has not moved yet");
    assert!(!rig.dir(new).exists(), "the state has not moved yet");
    let listed = git_in(&rig.project, &["worktree", "list", "--porcelain"]);
    assert!(
        porcelain_has(&listed, &new_work) && !porcelain_has(&listed, &old_work),
        "registration coherent at the cut: {listed}"
    );
    let intent = std::fs::read_to_string(
        rig.home
            .join("sessions")
            .join(format!(".rename.{old}.{new}.intent")),
    )
    .unwrap_or_default();
    assert!(intent.contains("phase=work-moved"), "{intent}");

    retry_rename(&rig, old, new);
    assert!(!rig.dir(old).exists());
    assert!(
        rig.meta(new)
            .contains(&format!("work_dir={}", new_work.display()))
    );
}

// NOTE (review I8): the former 60-second live-timeout proof lived here and
// passed (exit 1, exact diagnostic, byte-identical state, lock release); it
// now costs every gate ~62s, so the ceiling, the terminal branch, and the
// pass-through are proved deterministically at unit level while production
// keeps its real 60-second park.

/// Run one agent-addressed core subcommand (`ask`, `reply`) from `pane`:
/// the caller's tmux marker plus the session directory, as the helpers do.
fn agent_cmd(
    rig: &Rig,
    pane: &str,
    sub: &str,
    dir: &Path,
    tail: &[&str],
) -> (Option<i32>, String, String) {
    let mut args = vec![sub.to_owned(), dir.to_string_lossy().into_owned()];
    args.extend(tail.iter().map(|arg| (*arg).to_owned()));
    let out = ae()
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", pane)
        .env("HOME", &rig.scratch)
        .env("TMUX_TMPDIR", &rig.scratch)
        .args(&args)
        .output()
        .unwrap_or_else(|why| panic!("the core command should run: {why}"));
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// One pane per slot for a production-launched session: `(slot, pane id)`.
fn slot_panes(rig: &Rig, session: &str) -> Vec<(String, String)> {
    let (_, listed) = rig.tmux(&[
        "list-panes",
        "-t",
        &format!("={session}"),
        "-F",
        "#{@ae_slot} #{pane_id}",
    ]);
    listed
        .lines()
        .filter_map(|line| {
            line.split_once(' ')
                .map(|(slot, pane)| (slot.to_owned(), pane.to_owned()))
        })
        .collect()
}

/// The pending ask/review ids in a session's event log.
fn pending_ids(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter(|line| {
            line.contains("\"action\":\"ask\"") || line.contains("\"action\":\"review\"")
        })
        .map(|line| {
            line.split("\"ref\":\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_default()
                .to_owned()
        })
        .filter(|id| !id.is_empty())
        .collect()
}

/// A1: a pending same-session request refuses the stopped rename before any
/// write, naming its id; the authorized close (reply) unblocks the retry.
/// Base-writer records only: the ask travels the production helper while
/// live. Smallest defeating mutation: skip the pending legacy guard.
#[test]
fn a_pending_same_session_request_refuses_the_stopped_rename() {
    if skip() {
        return;
    }
    let rig = Rig::new("pendsame", &["claude"], None);
    let config = format!(
        "[profiles]\nclaude = \"{}\"\n\n[roster]\nlead = claude\nworker = claude\n\n\
         [workspace]\nmain = lead\nworkers = worker\nlayout = vertical\nwatchdog = false\n",
        rig.bin.join("claude").display()
    );
    assert!(std::fs::write(&rig.config, config).is_ok());
    let (code, stdout, stderr) = rig.launch(&["psess"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let panes = slot_panes(&rig, "psess");
    let have = format!("a lead pane: {panes:?}");
    let lead = panes
        .iter()
        .find_map(|(slot, pane)| (slot == "main").then(|| pane.clone()))
        .expect(&have);
    let (code, stdout, stderr) = agent_cmd(
        &rig,
        &lead,
        ae::cli::ASK,
        &rig.dir("psess"),
        &["worker", "the", "pending", "question"],
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let ids = pending_ids(&rig.dir("psess"));
    assert_eq!(ids.len(), 1, "one real pending ask");
    let id = ids[0].clone();

    let (code, stdout, stderr) = public(&rig, &["stop", "psess"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "psess", "pmoved"]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("pending request") && stderr.contains(&id),
        "the refusal names the stranded id: {stderr}"
    );
    assert!(stderr.contains("Nothing was renamed"), "{stderr}");
    assert!(
        !rig.home
            .join("sessions")
            .join(".rename.psess.pmoved.intent")
            .exists(),
        "the refusal precedes every write"
    );
    assert!(rig.dir("psess").is_dir() && !rig.dir("pmoved").exists());

    // The authorized remedy: resume, reply from the target seat, stop, retry.
    let (code, stdout, stderr) = rig.launch(&["psess"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let panes = slot_panes(&rig, "psess");
    let have = format!("a worker pane: {panes:?}");
    let worker = panes
        .iter()
        .find_map(|(slot, pane)| (slot == "worker.0").then(|| pane.clone()))
        .expect(&have);
    let (code, stdout, stderr) = agent_cmd(
        &rig,
        &worker,
        ae::cli::REPLY,
        &rig.dir("psess"),
        &["--as", "worker", &id, "done"],
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "psess"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "psess", "pmoved"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("Renamed 'psess' → 'pmoved' (stopped;"),
        "{stdout}"
    );
}

/// A1: a pending cross-session request refuses the rename of EITHER
/// participant — the peer ledger, not just the source, is read. Smallest
/// defeating mutation: check only the source ledger.
#[test]
fn a_pending_cross_session_request_refuses_either_rename() {
    if skip() {
        return;
    }
    let rig = Rig::new("pendcross", &["claude"], None);
    for session in ["pxa", "pxb"] {
        let (code, stdout, stderr) = rig.launch(&["--local", session]);
        assert_eq!(
            code,
            Some(0),
            "{session}: stdout: {stdout}\nstderr: {stderr}"
        );
    }
    let panes_a = slot_panes(&rig, "pxa");
    let have_a = format!("a lead pane in pxa: {panes_a:?}");
    let lead_a = panes_a
        .iter()
        .find_map(|(slot, pane)| (slot == "main").then(|| pane.clone()))
        .expect(&have_a);
    let (code, stdout, stderr) = agent_cmd(
        &rig,
        &lead_a,
        ae::cli::ASK,
        &rig.dir("pxa"),
        &["--cross-session", "@pxb:lead", "the", "cross", "question"],
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let ids = pending_ids(&rig.dir("pxa"));
    assert_eq!(ids.len(), 1, "one real pending cross ask");
    let id = ids[0].clone();

    for session in ["pxa", "pxb"] {
        let (code, stdout, stderr) = public(&rig, &["stop", session]);
        assert_eq!(
            code,
            Some(0),
            "{session}: stdout: {stdout}\nstderr: {stderr}"
        );
    }
    // The target side: its own log mirrors the pending request.
    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "pxb", "pxb2"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("pending request") && stderr.contains(&id),
        "the peer refusal names the id: {stderr}"
    );
    // The caller side: its own log holds the pending ask.
    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "pxa", "pxa2"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("pending request") && stderr.contains(&id),
        "the caller refusal names the id: {stderr}"
    );
    assert!(!rig.dir("pxa2").exists() && !rig.dir("pxb2").exists());
}

/// A2: unreadable peer evidence refuses the rename rather than inventing a
/// binding — even when the source log is clean. Smallest defeating mutation:
/// check only the source ledger (the corrupt peer goes unread).
#[test]
fn an_unreadable_peer_log_refuses_the_stopped_rename() {
    if skip() {
        return;
    }
    let rig = Rig::idle("unreadable-peer");
    for session in ["upmain", "uppeer"] {
        let (code, stdout, stderr) = rig.launch(&["--local", session]);
        assert_eq!(
            code,
            Some(0),
            "{session}: stdout: {stdout}\nstderr: {stderr}"
        );
    }
    for session in ["upmain", "uppeer"] {
        let (code, stdout, stderr) = public(&rig, &["stop", session]);
        assert_eq!(
            code,
            Some(0),
            "{session}: stdout: {stdout}\nstderr: {stderr}"
        );
    }
    // Corrupt the peer's ledger the only hermetic way: permissions the
    // existing reader errors on rather than skips.
    let peer_log = rig.dir("uppeer").join("events.jsonl");
    assert!(peer_log.is_file(), "the peer holds a ledger");
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::set_permissions(&peer_log, std::fs::Permissions::from_mode(0o000)).is_ok()
        );
    }
    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "upmain", "upmoved"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("uppeer") && stderr.contains("Nothing was renamed"),
        "the refusal names the unreadable ledger: {stderr}"
    );
    assert!(
        !rig.dir("upmoved").exists(),
        "no write past corrupt evidence"
    );
    // Repair unblocks: the guard is evidence, not a latch.
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::set_permissions(&peer_log, std::fs::Permissions::from_mode(0o644)).is_ok()
        );
    }
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "upmain", "upmoved"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
}

/// A2: a manifest the publisher cannot write fails the rename with the
/// proved retry — never success over an unpublished manifest. A chmod of the
/// old file alone cannot defeat the sibling-temp writer, so the receipt uses
/// an incompatible directory at the destination. Smallest defeating mutation:
/// discard the manifest publish error.
#[test]
fn a_stopped_manifest_failure_is_checked_and_retryable() {
    if skip() {
        return;
    }
    let old = "smanold";
    let new = "smannew";
    let rig = stopped_local_source("manifest-fail", old);
    assert!(std::fs::remove_file(rig.dir(old).join("workspace.md")).is_ok());
    assert!(std::fs::create_dir(rig.dir(old).join("workspace.md")).is_ok());

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("could not publish")
            && stderr.contains("workspace.md")
            && stderr.contains(&format!("ae rename {old} {new}")),
        "checked failure with the proved retry: {stderr}"
    );
    // Partial failure converges forward: intent at meta-published, the state
    // already moved (assets come after), the coherent meta naming the new
    // session intact.
    let intent = std::fs::read_to_string(
        rig.home
            .join("sessions")
            .join(format!(".rename.{old}.{new}.intent")),
    )
    .unwrap_or_default();
    assert!(intent.contains("phase=meta-published"), "{intent}");
    assert!(!rig.dir(old).exists() && rig.dir(new).is_dir());
    assert!(rig.meta(new).contains(&format!("session={new}\n")));

    // The blocker moved with the state directory; removing it unblocks.
    assert!(std::fs::remove_dir(rig.dir(new).join("workspace.md")).is_ok());
    retry_rename(&rig, old, new);
    let workspace = std::fs::read_to_string(rig.dir(new).join("workspace.md")).unwrap_or_default();
    assert!(workspace.contains(new));
}

/// A2: a locked managed worktree refuses before any write, with the unlock
/// remedy; unlocking unblocks the retry. Smallest defeating mutation: skip
/// the registration proof and let git fail mid-transaction.
#[test]
fn a_locked_worktree_refuses_before_side_effects() {
    if skip() {
        return;
    }
    let rig = Rig::idle("locked-wt");
    git_in(&rig.project, &["init", "-q"]);
    git_in(&rig.project, &["config", "user.email", "t@t"]);
    git_in(&rig.project, &["config", "user.name", "t"]);
    assert!(std::fs::write(rig.project.join("f"), "x\n").is_ok());
    git_in(&rig.project, &["add", "-A"]);
    git_in(&rig.project, &["commit", "-qm", "base"]);

    let old = "slockold";
    let new = "slocknew";
    let (code, stdout, stderr) = rig.launch(&["--worktree", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join(old);
    git_in(
        &rig.project,
        &["worktree", "lock", &old_work.display().to_string()],
    );
    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("locked worktree") && stderr.contains("Nothing was renamed"),
        "{stderr}"
    );
    assert!(
        !rig.home
            .join("sessions")
            .join(format!(".rename.{old}.{new}.intent"))
            .exists(),
        "no intent past the refusal"
    );
    assert!(old_work.is_dir() && rig.dir(old).is_dir(), "zero mutation");

    git_in(
        &rig.project,
        &["worktree", "unlock", &old_work.display().to_string()],
    );
    retry_rename(&rig, old, new);
    assert!(!old_work.exists() && rig.home.join("worktrees").join(new).is_dir());
}

/// A1: occupied destinations refuse before writes — a live namesake on the
/// recorded server, and an occupant state directory alike. Smallest defeating
/// mutation: drop either destination check.
#[test]
fn occupied_destinations_refuse_before_writes() {
    if skip() {
        return;
    }
    let rig = Rig::idle("occupied-dest");
    for session in ["soccold", "soccbusy"] {
        let (code, stdout, stderr) = rig.launch(&["--local", session]);
        assert_eq!(
            code,
            Some(0),
            "{session}: stdout: {stdout}\nstderr: {stderr}"
        );
    }
    let (code, stdout, stderr) = public(&rig, &["stop", "soccold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "soccold", "soccbusy"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("already exists") && stderr.contains("Nothing was renamed"),
        "{stderr}"
    );

    assert!(std::fs::create_dir(rig.dir("soccplant")).is_ok());
    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "soccold", "soccplant"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("already exists") && stderr.contains("Nothing was renamed"),
        "{stderr}"
    );
    assert!(
        rig.dir("soccold").is_dir(),
        "the source survives both refusals"
    );
}

/// Pin an explicit config home and a probeable conversation through the
/// production meta publisher, then plant the tool-store transcript for the
/// OLD cwd only. Returns the home and id.
fn pin_explicit_home(rig: &Rig, session: &str, old_work: &Path) -> (PathBuf, String) {
    let home = rig.scratch.join("tool-home");
    assert!(std::fs::create_dir_all(home.join("projects")).is_ok());
    let id = "e795c9e9-1234-4890-abcd-ef0123456789".to_owned();
    let key: String = old_work
        .display()
        .to_string()
        .chars()
        .map(|ch| if ch == '/' { '-' } else { ch })
        .collect();
    assert!(
        std::fs::create_dir_all(home.join("projects").join(&key)).is_ok()
            && std::fs::write(
                home.join("projects").join(&key).join(format!("{id}.jsonl")),
                "{}\n"
            )
            .is_ok()
    );
    assert!(
        ae::meta::rewrite(
            &rig.dir(session),
            "config_home.main",
            Some(&home.display().to_string())
        )
        .is_ok()
    );
    assert!(ae::meta::rewrite(&rig.dir(session), "config_home_base.main", None).is_ok());
    assert!(ae::meta::rewrite(&rig.dir(session), "harness_session.main", Some(&id)).is_ok());
    (home, id)
}

/// Plant the tool-store transcript for one cwd under an explicit home.
fn plant_transcript(home: &Path, cwd: &Path, id: &str) {
    let key: String = cwd
        .display()
        .to_string()
        .chars()
        .map(|ch| if ch == '/' { '-' } else { ch })
        .collect();
    assert!(std::fs::create_dir_all(home.join("projects").join(&key)).is_ok());
    assert!(
        std::fs::write(
            home.join("projects").join(&key).join(format!("{id}.jsonl")),
            "{}\n"
        )
        .is_ok()
    );
}

/// A1: a Claude explicit home whose conversation ae resumes exactly at the
/// old cwd but not at the candidate one refuses before any write; proving
/// the candidate keeps the probe unblocks the retry. Smallest defeating
/// mutation: skip the explicit-home candidate probe.
#[test]
fn a_claude_explicit_home_fallback_refuses_and_its_proof_unblocks() {
    if skip() {
        return;
    }
    let rig = Rig::new("expclaude", &["claude"], None);
    let old = "sexpold";
    let new = "sexpnew";
    let (code, stdout, stderr) = rig.launch(&["--copy", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join(old);
    let new_work = rig.home.join("worktrees").join(new);
    let (home, id) = pin_explicit_home(&rig, old, &old_work);

    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("explicit config home")
            && stderr.contains(&id)
            && stderr.contains("Nothing was renamed"),
        "the refusal names the seat, store and conversation: {stderr}"
    );
    assert!(
        !rig.home
            .join("sessions")
            .join(format!(".rename.{old}.{new}.intent"))
            .exists(),
        "no intent past the refusal"
    );

    // Proving the candidate keeps exact resume unblocks the same rename.
    plant_transcript(&home, &new_work, &id);
    retry_rename(&rig, old, new);
}

/// A1: a Codex explicit home is never refused over a Claude-shaped store —
/// its dated-rollout probe is not cwd-keyed, so the move cannot newly break
/// ae-side exact resume. Smallest defeating mutation: apply the transcript
/// probe to every tool (this rename refuses).
#[test]
fn a_codex_explicit_home_moves_without_a_transcript_probe() {
    if skip() {
        return;
    }
    let rig = Rig::new("expcodex", &["codex"], None);
    let old = "scdxold";
    let new = "scdxnew";
    let (code, stdout, stderr) = rig.launch(&["--copy", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join(old);
    let (_home, _id) = pin_explicit_home(&rig, old, &old_work);

    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    // The Claude-shaped transcript exists for the old cwd only — and must
    // not matter to a Codex seat.
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!("Renamed '{old}' → '{new}' (stopped;")),
        "{stdout}"
    );
}

/// A1: an unclassifiable tool with an explicit home warns rather than
/// refuses — unknown provider behavior is a later resume concern, and the
/// later resume re-proves it. Smallest defeating mutation: refuse every
/// explicit home the probe cannot classify.
#[test]
fn an_unknown_tool_explicit_home_warns_and_moves() {
    if skip() {
        return;
    }
    let rig = Rig::idle("expunknown");
    let old = "sunkold";
    let new = "sunknew";
    let (code, stdout, stderr) = rig.launch(&["--copy", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join(old);
    let (_home, _id) = pin_explicit_home(&rig, old, &old_work);

    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!("Renamed '{old}' → '{new}' (stopped;")),
        "{stdout}"
    );
    assert!(
        stderr.contains("implicit or unclassified config homes"),
        "the unclassified home warns: {stderr}"
    );
}

/// A3: while a transaction is pending, launch/resume/end stand aside with
/// the proved retry instead of reading or mutating a mixed generation;
/// doctor reports the phase. The retry converges and the guards clear.
/// Smallest defeating mutation: drop the intent check from any one reader.
#[test]
fn readers_stand_aside_for_a_pending_transaction() {
    if skip() {
        return;
    }
    let old = "sblkold";
    let new = "sblknew";
    let rig = stopped_local_source("blocked-readers", old);
    kill_at_boundary(&rig, old, new, "after-intent");

    let retry = format!("ae rename {old} {new}");
    let (code, stdout, stderr) = rig.launch(&[new]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains(&retry), "resume stands aside: {stderr}");
    let (code, stdout, stderr) = rig.launch(&[old]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains(&retry), "relaunch stands aside: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["end", old, "-f", "--keep-history"]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains(&retry), "end stands aside: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["doctor"]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    let report = format!("{stdout}{stderr}");
    assert!(
        report.contains(&retry) && report.contains("prepared"),
        "doctor reports the pending phase: {report}"
    );

    retry_rename(&rig, old, new);
    let (code, stdout, stderr) = rig.launch(&[new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(&format!("Resuming session {new}")),
        "{stdout}"
    );
}

/// A3: after a stopped rename, the read-only fleet surface resolves the new
/// address — list shows it, archive preview renders it, doctor is clean —
/// and the old name resolves to nothing.
#[test]
fn renamed_surface_resolves_to_the_new_address() {
    if skip() {
        return;
    }
    let old = "srfold";
    let new = "srfnew";
    let rig = stopped_local_source("renamed-surface", old);
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &["list"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains(new), "list shows the new name: {stdout}");
    assert!(
        !stdout.lines().any(|line| line.contains(old)),
        "list shows no old name: {stdout}"
    );

    let (code, stdout, stderr) = public(&rig, &["archive", "preview", new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(new),
        "preview renders the new address: {stdout}"
    );

    let (code, _, stderr) = public(&rig, &["archive", "preview", old]);
    assert_eq!(code, Some(1), "the old address previews nothing: {stderr}");

    // Doctor is read-only over the new address: no pending-rename row once
    // the transaction completed (a fresh scratch rig fails unrelated
    // environment rows, so the assertion is scoped to rename rows).
    let (code, stdout, stderr) = public(&rig, &["doctor"]);
    let _ = code;
    let report = format!("{stdout}{stderr}");
    assert!(
        !report.contains("rename:"),
        "no pending rename rows: {report}"
    );
}

/// BLOCKER B1: the carrier must name the command's own pair. A valid C→D
/// payload under an A→B filename refuses before any of the three sessions is
/// touched — recovery under A/B locks must never mutate C/D. Smallest
/// defeating mutation: drop the filename/argv/payload equality.
#[test]
fn a_mismatched_carrier_refuses_before_touching_either_pair() {
    if skip() {
        return;
    }
    let rig = Rig::idle("mismatch-carrier");
    for session in ["mcA", "mcC", "mcD"] {
        let (code, stdout, stderr) = rig.launch(&["--local", session]);
        assert_eq!(
            code,
            Some(0),
            "{session}: stdout: {stdout}\nstderr: {stderr}"
        );
        let (code, stdout, stderr) = public(&rig, &["stop", session]);
        assert_eq!(
            code,
            Some(0),
            "{session}: stdout: {stdout}\nstderr: {stderr}"
        );
    }
    let row = |session: &str, key: &str| {
        rig.meta(session)
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    // A well-formed C→D carrier (C's own UUID, server, origin, work) filed
    // under the A→B filename: hostile input, valid signature, wrong scope.
    let carrier = format!(
        "rename_intent=1\nsession_id={}\nold=mcC\nnew=mcD\nmode=local\nold_work={work}\nnew_work={work}\norigin={work}\nserver_kind={kind}\nserver_value={value}\nphase=prepared\nwork_dev=0\nwork_ino=0\nadmin_dev=0\nadmin_ino=0\n",
        row("mcC", "session_id"),
        work = row("mcC", "work_dir"),
        kind = row("mcC", "tmux_server_kind"),
        value = row("mcC", "tmux_server"),
    );
    assert!(carrier.lines().count() == 15, "a parsable carrier");
    let carrier_path = rig.home.join("sessions").join(".rename.mcA.mcB.intent");
    assert!(std::fs::write(&carrier_path, &carrier).is_ok());
    let snapshot = |session: &str| {
        (
            rig.meta(session),
            std::fs::read(rig.dir(session).join("events.jsonl")).unwrap_or_default(),
        )
    };
    let (before_a, before_c, before_d) = (snapshot("mcA"), snapshot("mcC"), snapshot("mcD"));

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "mcA", "mcB"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("does not match its filename") && stderr.contains("Nothing was renamed"),
        "the correlation refusal: {stderr}"
    );
    assert_eq!(snapshot("mcA"), before_a, "A untouched");
    assert_eq!(snapshot("mcC"), before_c, "C untouched");
    assert_eq!(snapshot("mcD"), before_d, "D untouched");
    assert!(!rig.dir("mcB").exists(), "B never created");
    // The mismatch is damage everywhere, not just at the rename: every
    // lifecycle reader stands aside with the same named gap.
    let (code, _, stderr) = rig.launch(&["mcA"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("damaged rename carrier"), "{stderr}");
    let (code, _, stderr) = public(&rig, &["end", "mcA", "-f", "--keep-history"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("damaged rename carrier"), "{stderr}");
}

/// BLOCKER B2: recovery binds mode/origin/work to the located meta. A forged
/// full-mode carrier over a stopped LOCAL session refuses before either path
/// moves — the unrelated managed path stays, the local meta stays coherent.
/// Smallest defeating mutation: skip the payload binding.
#[test]
fn a_forged_mode_carrier_refuses_before_either_path_moves() {
    if skip() {
        return;
    }
    let rig = Rig::idle("forged-mode");
    let (code, stdout, stderr) = rig.launch(&["--local", "flocal"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    // An unrelated managed path the forged carrier points at. Its real
    // fingerprint goes into the carrier so the refusal comes from the mode
    // binding, not the witness shape.
    let stray = rig.home.join("worktrees").join("flocal");
    assert!(std::fs::create_dir_all(&stray).is_ok());
    assert!(std::fs::write(stray.join("unrelated"), "not this session\n").is_ok());
    let (stray_dev, stray_ino) = {
        let meta = std::fs::metadata(&stray).expect("the stray dir");
        (meta.dev(), meta.ino())
    };
    let row = |key: &str| {
        rig.meta("flocal")
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    let forged = format!(
        "rename_intent=1\nsession_id={}\nold=flocal\nnew=fnew\nmode=full\nold_work={}\nnew_work={}\norigin={}\nserver_kind={}\nserver_value={}\nphase=prepared\nwork_dev={stray_dev}\nwork_ino={stray_ino}\nadmin_dev=0\nadmin_ino=0\n",
        row("session_id"),
        stray.display(),
        rig.home.join("worktrees").join("fnew").display(),
        row("origin"),
        row("tmux_server_kind"),
        row("tmux_server"),
    );
    assert!(
        std::fs::write(
            rig.home.join("sessions").join(".rename.flocal.fnew.intent"),
            &forged
        )
        .is_ok()
    );
    let (code, stdout, stderr) = public(&rig, &["stop", "flocal"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let meta_before = rig.meta("flocal");

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "flocal", "fnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("records mode 'local' but the transaction is 'full'"),
        "the binding refusal: {stderr}"
    );
    assert_eq!(
        std::fs::read(stray.join("unrelated")).unwrap_or_default(),
        b"not this session\n",
        "the unrelated path never moves"
    );
    assert!(!rig.home.join("worktrees").join("fnew").exists());
    assert_eq!(
        rig.meta("flocal"),
        meta_before,
        "the local meta stays coherent"
    );
    assert!(!rig.dir("fnew").exists(), "no state move");
}

/// BLOCKER B3: a damaged carrier blocks every reader with the named gap —
/// the rename itself, a launch of either named session, an end, and doctor —
/// instead of reading absence. Smallest defeating mutation: restore
/// ok().flatten (readers proceed over the mixed generation).
#[test]
fn a_damaged_carrier_blocks_every_reader() {
    if skip() {
        return;
    }
    let rig = Rig::idle("damaged-carrier");
    let (code, stdout, stderr) = rig.launch(&["--local", "dgA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "dgA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        std::fs::write(
            rig.home.join("sessions").join(".rename.dgA.dgB.intent"),
            "rename_intent=1\nthis line has no equals\n"
        )
        .is_ok()
    );

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "dgA", "dgB"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("not a valid rename intent"), "{stderr}");

    let (code, _, stderr) = rig.launch(&["dgA"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("damaged rename carrier"),
        "launch stands aside: {stderr}"
    );
    // The filename scope covers the far endpoint too: dgB was never created,
    // and a fresh launch must not materialize over the damage.
    let (code, _, stderr) = rig.launch(&["dgB"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("damaged rename carrier"),
        "far endpoint stands aside: {stderr}"
    );
    let (code, _, stderr) = public(&rig, &["end", "dgA", "-f", "--keep-history"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("damaged rename carrier"),
        "end stands aside: {stderr}"
    );
    let (code, stdout, stderr) = public(&rig, &["doctor"]);
    let _ = code;
    let report = format!("{stdout}{stderr}");
    assert!(
        report.contains("damaged rename carrier") && report.contains("dgA"),
        "doctor fails the damaged pair: {report}"
    );

    // Removing the damage unblocks: the block is evidence, not a latch.
    assert!(std::fs::remove_file(rig.home.join("sessions").join(".rename.dgA.dgB.intent")).is_ok());
    retry_rename(&rig, "dgA", "dgB");
}

/// BLOCKER B4: a prefix pair (pfoobar→pfoo) converges exact bytes end to
/// end — the manifest carries exact new lines, and a launch-shaped opencode
/// pair is republished to the exact new pointer. (Substring traps are pinned
/// at unit level; this proves the converged bytes.)
#[test]
fn a_prefix_rename_converges_exact_bytes() {
    if skip() {
        return;
    }
    let rig = Rig::idle("prefix-bytes");
    let (code, stdout, stderr) = rig.launch(&["--local", "pfoobar"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    // The seat classifies opencode by its recorded binary, so the planted
    // pair below is a required asset (not a stray) with a stale pointer.
    assert!(
        ae::meta::rewrite(
            &rig.dir("pfoobar"),
            "agent_bin.main",
            Some("/tmp/fake-bin/opencode")
        )
        .is_ok()
    );
    // A launch-shaped opencode pair pointing at the old address, plus a
    // stray pair no roster seat requires (removed, never verified).
    let old_ctx = format!("{}/opencode.main.md", rig.dir("pfoobar").display());
    assert!(
        std::fs::write(
            rig.dir("pfoobar").join("opencode.main.md"),
            "You are in an ae multi-agent workspace. Session: pfoobar. Directory: /proj.\n"
        )
        .is_ok()
    );
    assert!(
        std::fs::write(
            rig.dir("pfoobar").join("opencode.main.json"),
            format!("{{\"instructions\":[\"{old_ctx}\"]}}\n")
        )
        .is_ok()
    );
    assert!(
        std::fs::write(
            rig.dir("pfoobar").join("opencode.gone.md"),
            "orphaned generated context\n"
        )
        .is_ok()
    );
    let (code, stdout, stderr) = public(&rig, &["stop", "pfoobar"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "pfoobar", "pfoo"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let manifest =
        std::fs::read_to_string(rig.dir("pfoo").join("workspace.md")).unwrap_or_default();
    let lines: Vec<&str> = manifest.lines().collect();
    assert!(lines.contains(&"Session: pfoo"), "{manifest}");
    assert!(
        !lines.contains(&"Session: pfoobar"),
        "no stale session line: {manifest}"
    );
    assert!(
        manifest.contains(&format!("{}/send", rig.dir("pfoo").display())),
        "new helper address: {manifest}"
    );
    let new_ctx = format!("{}/opencode.main.md", rig.dir("pfoo").display());
    let json =
        std::fs::read_to_string(rig.dir("pfoo").join("opencode.main.json")).unwrap_or_default();
    assert!(
        json.contains(&format!("\"{new_ctx}\"")),
        "exact new pointer: {json}"
    );
    let md = std::fs::read_to_string(rig.dir("pfoo").join("opencode.main.md")).unwrap_or_default();
    assert!(md.contains("Session: pfoo."), "exact new sentence: {md}");
    assert!(
        !rig.dir("pfoo").join("opencode.gone.md").exists()
            && !rig.dir("pfoo").join("opencode.gone.json").exists(),
        "stray pairs are removed, never verified"
    );
}

/// IMPORTANT I7: a dangling helper link is repaired to the proven core, not
/// completed over. Smallest defeating mutation: type-only helper checks.
#[test]
fn a_dangling_helper_link_is_repaired_not_completed() {
    if skip() {
        return;
    }
    let rig = Rig::idle("dangling-helper");
    let (code, stdout, stderr) = rig.launch(&["--local", "dold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let pinned = std::fs::read_link(rig.dir("dold").join("ask")).unwrap_or_default();
    assert!(!pinned.as_os_str().is_empty(), "a recorded target");
    let (code, stdout, stderr) = public(&rig, &["stop", "dold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    // Never write through a helper path: remove, then plant the damage.
    assert!(std::fs::remove_file(rig.dir("dold").join("send")).is_ok());
    std::os::unix::fs::symlink("/nonexistent-core-under-test", rig.dir("dold").join("send"))
        .expect("a dangling link");
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "dold", "dnew"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        std::fs::read_link(rig.dir("dnew").join("send")).unwrap_or_default(),
        pinned,
        "repaired to the proven core"
    );
}

/// IMPORTANT I7: a repointed helper link — even at an existing binary — is
/// repaired to the proven core, not completed over. Smallest defeating
/// mutation: existence-only helper checks.
#[test]
fn a_repointed_helper_link_is_repaired_not_completed() {
    if skip() {
        return;
    }
    let rig = Rig::idle("repointed-helper");
    let (code, stdout, stderr) = rig.launch(&["--local", "rold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let pinned = std::fs::read_link(rig.dir("rold").join("ask")).unwrap_or_default();
    let (code, stdout, stderr) = public(&rig, &["stop", "rold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    assert!(std::fs::remove_file(rig.dir("rold").join("send")).is_ok());
    std::os::unix::fs::symlink("/bin/sh", rig.dir("rold").join("send")).expect("a repointed link");
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "rold", "rnew"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        std::fs::read_link(rig.dir("rnew").join("send")).unwrap_or_default(),
        pinned,
        "repaired to the proven core, not the planted binary"
    );
}

/// BLOCKER (r2-1): a carrier claiming phases its facts never reached
/// normalizes down instead of skipping into success. A local carrier at
/// assets-published with the state still home still moves everything; the
/// same-command retry then reads the durable result. Smallest defeating
/// mutation: trust the recorded phase as the skip boundary (the old address
/// stays put under a printed success).
#[test]
fn a_phase_ahead_carrier_reproves_instead_of_skipping() {
    if skip() {
        return;
    }
    let rig = Rig::idle("phase-ahead");
    let (code, stdout, stderr) = rig.launch(&["--local", "paold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "paold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let row = |key: &str| {
        rig.meta("paold")
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    // A well-formed carrier whose facts stop at prepared, but whose record
    // claims assets-published.
    let ahead = format!(
        "rename_intent=1\nsession_id={}\nold=paold\nnew=panew\nmode=local\nold_work={work}\nnew_work={work}\norigin={work}\nserver_kind={kind}\nserver_value={value}\nphase=assets-published\nwork_dev=0\nwork_ino=0\nadmin_dev=0\nadmin_ino=0\n",
        row("session_id"),
        work = row("work_dir"),
        kind = row("tmux_server_kind"),
        value = row("tmux_server"),
    );
    assert!(
        std::fs::write(
            rig.home.join("sessions").join(".rename.paold.panew.intent"),
            &ahead
        )
        .is_ok()
    );
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "paold", "panew"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("Renamed 'paold' → 'panew' (stopped;"),
        "{stdout}"
    );
    // Converged, not skipped: the old address is gone and the new meta is
    // coherent — a skip would have left the state home with old rows.
    assert!(!rig.dir("paold").exists(), "the state moved");
    assert!(rig.dir("panew").is_dir(), "the state landed");
    let meta = rig.meta("panew");
    assert!(meta.contains("session=panew\n"), "{meta}");
    let intent =
        std::fs::read_to_string(rig.home.join("sessions").join(".rename.paold.panew.intent"))
            .unwrap_or_default();
    assert!(intent.contains("phase=complete"), "{intent}");
}

/// BLOCKER (r2-1, managed): a work-moved claim with the work still home
/// still moves the work. Smallest defeating mutation: trust the recorded
/// phase (the meta would point at a nonexistent destination).
#[test]
fn a_phase_ahead_managed_carrier_still_moves_the_work() {
    if skip() {
        return;
    }
    let rig = Rig::idle("phase-ahead-managed");
    let (code, stdout, stderr) = rig.launch(&["--copy", "pmold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join("pmold");
    let new_work = rig.home.join("worktrees").join("pmnew");
    assert!(std::fs::write(old_work.join("marker"), "mine\n").is_ok());
    // A truthful identity witness with a lying phase: the attack is the
    // phase claim, not a forged fingerprint (fingerprints are public).
    let (work_dev, work_ino) = {
        let meta = std::fs::metadata(&old_work).expect("the work dir");
        (meta.dev(), meta.ino())
    };
    let row = |key: &str| {
        rig.meta("pmold")
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    let ahead = format!(
        "rename_intent=1\nsession_id={}\nold=pmold\nnew=pmnew\nmode=full\nold_work={}\nnew_work={}\norigin={}\nserver_kind={}\nserver_value={}\nphase=work-moved\nwork_dev={work_dev}\nwork_ino={work_ino}\nadmin_dev=0\nadmin_ino=0\n",
        row("session_id"),
        old_work.display(),
        new_work.display(),
        row("origin"),
        row("tmux_server_kind"),
        row("tmux_server"),
    );
    assert!(
        std::fs::write(
            rig.home.join("sessions").join(".rename.pmold.pmnew.intent"),
            &ahead
        )
        .is_ok()
    );
    let (code, stdout, stderr) = public(&rig, &["stop", "pmold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "pmold", "pmnew"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        std::fs::read(new_work.join("marker")).unwrap_or_default(),
        b"mine\n",
        "the work moved instead of being skipped"
    );
    assert!(!old_work.exists(), "the old path is gone");
}

/// BLOCKER (r2-2): replacing the moved work after the cut refuses the retry
/// instead of binding the session to the replacement — before any state or
/// meta publication, with the planted entry preserved. Smallest defeating
/// mutation: path-only work verification.
#[test]
fn a_replaced_managed_work_refuses_the_retry() {
    if skip() {
        return;
    }
    let rig = Rig::idle("replaced-work");
    let old = "rtold";
    let new = "rtnew";
    let (code, stdout, stderr) = rig.launch(&["--copy", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let new_work = rig.home.join("worktrees").join(new);
    let (code, stdout, stderr) = public(&rig, &["stop", old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, old, new, "after-work-move");
    // Swap the moved tree for a same-spelling replacement.
    assert!(std::fs::remove_dir_all(&new_work).is_ok());
    assert!(std::fs::create_dir_all(&new_work).is_ok());
    assert!(std::fs::write(new_work.join("replacement"), "not the session\n").is_ok());

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, old, new]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("does not match the recorded identity")
            && stderr.contains("refusing to move an unproved directory"),
        "{stderr}"
    );
    assert!(
        !rig.dir(new).exists(),
        "no state publication past the refusal"
    );
    assert_eq!(
        std::fs::read(new_work.join("replacement")).unwrap_or_default(),
        b"not the session\n",
        "the planted entry is preserved"
    );
}

/// BLOCKER (r2-2): a dangling symlink at the managed destination refuses
/// before any write — it is an occupant entry, and `rename(2)` would
/// overwrite it. The link itself is preserved.
#[test]
fn a_dangling_destination_symlink_refuses_before_writes() {
    if skip() {
        return;
    }
    let rig = Rig::idle("dangling-dest");
    let (code, stdout, stderr) = rig.launch(&["--copy", "ddold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "ddold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let dest = rig.home.join("worktrees").join("ddnew");
    std::os::unix::fs::symlink("/nonexistent-target-under-test", &dest)
        .expect("a dangling destination link");

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "ddold", "ddnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("is a symlink") && stderr.contains("Nothing was renamed"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_link(&dest).unwrap_or_default(),
        PathBuf::from("/nonexistent-target-under-test"),
        "the planted link is preserved"
    );
    assert!(
        !rig.home
            .join("sessions")
            .join(".rename.ddold.ddnew.intent")
            .exists(),
        "no intent past the refusal"
    );
}

/// IMPORTANT (r2-4): 128-byte legal names hash the carrier instead of
/// overflowing the directory entry; the rename still converges and no
/// overlong file appears. Smallest defeating mutation: literal filenames
/// for every pair (publication fails past `NAME_MAX`).
#[test]
fn max_length_names_hash_the_carrier() {
    if skip() {
        return;
    }
    let rig = Rig::idle("max-names");
    let old: String = std::iter::repeat_n('o', 128).collect();
    let new: String = std::iter::repeat_n('n', 128).collect();
    let (code, stdout, stderr) = rig.launch(&["--local", &old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", &old]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, &old, &new]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let mut carriers = Vec::new();
    for entry in std::fs::read_dir(rig.home.join("sessions")).unwrap_or_else(|_| panic!("sessions"))
    {
        let path = entry.unwrap_or_else(|_| panic!("entry")).path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".rename."))
        {
            carriers.push(path);
        }
    }
    assert_eq!(carriers.len(), 1, "{carriers:?}");
    let name = carriers[0]
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    assert!(
        name.len() < 200 && name.ends_with(".intent"),
        "a bounded digest carrier, not a 272-byte literal: {name}"
    );
    assert!(rig.meta(&new).contains(&format!("session={new}\n")));
}

/// IMPORTANT (r2-4): same-pair reuse by a fresh UUID rotates the old result
/// aside byte-identical and starts a new transaction instead of refusing on
/// the stale completion.
#[test]
fn same_pair_reuse_rotates_the_old_result() {
    if skip() {
        return;
    }
    let rig = Rig::idle("pair-reuse");
    let (code, stdout, stderr) = rig.launch(&["--local", "rsA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "rsA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "rsA", "rsB"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let first_uuid = rig
        .meta("rsB")
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .unwrap_or_default()
        .to_owned();
    let first_intent =
        std::fs::read(rig.home.join("sessions").join(".rename.rsA.rsB.intent")).unwrap_or_default();

    let (code, stdout, stderr) = public(&rig, &["end", "rsB", "-f", "--keep-history"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = rig.launch(&["--local", "rsA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let second_uuid = rig
        .meta("rsA")
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .unwrap_or_default()
        .to_owned();
    assert_ne!(second_uuid, first_uuid, "a fresh incarnation");

    let (code, stdout, stderr) = public(&rig, &["stop", "rsA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "rsA", "rsB"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("Renamed 'rsA' → 'rsB' (stopped;"),
        "{stdout}"
    );
    // The old result survives byte-identical under history; the live carrier
    // carries the fresh UUID to completion.
    let rotated = rig
        .home
        .join("sessions")
        .join(".rename.rsA.rsB.intent.complete");
    assert_eq!(
        std::fs::read(&rotated).unwrap_or_default(),
        first_intent,
        "the old result is preserved, not overwritten"
    );
    assert_eq!(
        rig.meta("rsB")
            .lines()
            .find_map(|line| line.strip_prefix("session_id="))
            .unwrap_or_default(),
        second_uuid
    );
}

/// IMPORTANT (r2-5): an unrelated malformed carrier never blocks a fresh
/// pair — filename relevance filters before any read — while the exact pair
/// still refuses. Smallest defeating mutation: parse every carrier before
/// filtering by filename.
#[test]
fn an_unrelated_damaged_carrier_never_blocks_a_fresh_pair() {
    if skip() {
        return;
    }
    let rig = Rig::idle("unrelated-damage");
    let (code, stdout, stderr) = rig.launch(&["--local", "uxA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "uxA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        std::fs::write(
            rig.home.join("sessions").join(".rename.uxC.uxD.intent"),
            "rename_intent=1\nthis line has no equals\n"
        )
        .is_ok()
    );
    // The fresh pair converges despite the unrelated damage...
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "uxA", "uxB"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    // ...while the damaged pair itself is still reported globally.
    let (code, stdout, stderr) = public(&rig, &["doctor"]);
    let _ = code;
    let report = format!("{stdout}{stderr}");
    assert!(
        report.contains("damaged rename carrier") && report.contains("uxC"),
        "{report}"
    );
}

/// IMPORTANT (r2-8): `list` surfaces a pending transaction on stderr with
/// its phase and retry instead of reading as settled. Smallest defeating
/// mutation: drop the pending warning from the list dispatch.
#[test]
fn list_surfaces_a_pending_transaction() {
    if skip() {
        return;
    }
    let rig = Rig::idle("list-pending");
    let (code, stdout, stderr) = rig.launch(&["--local", "lpA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "lpA"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    kill_at_boundary(&rig, "lpA", "lpB", "after-intent");

    let (code, stdout, stderr) = public(&rig, &["list"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("rename 'lpA' → 'lpB' is in progress at phase 'prepared'")
            && stderr.contains("ae rename lpA lpB"),
        "the pending state reads on stderr, not as a normal row: {stderr}"
    );
    retry_rename(&rig, "lpA", "lpB");
    let (code, stdout, stderr) = public(&rig, &["list"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        !stderr.contains("in progress"),
        "the warning clears with the transaction: {stdout}{stderr}"
    );
}

/// IMPORTANT (r2-7): an opencode seat with no generated pair forces
/// republication — absence is never ready. Required pairs derive from the
/// seat tool, not from whatever files happen to be present. Smallest
/// defeating mutation: presence-based opencode checks (the pair stays
/// missing).
#[test]
fn a_missing_opencode_pair_is_generated_not_skipped() {
    if skip() {
        return;
    }
    let rig = Rig::idle("opencode-required");
    let (code, stdout, stderr) = rig.launch(&["--local", "oqold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    // The idle seat runs `sleep`, but the recorded binary is what classifies
    // the seat: point it at an opencode binary name.
    assert!(
        ae::meta::rewrite(
            &rig.dir("oqold"),
            "agent_bin.main",
            Some("/tmp/fake-bin/opencode")
        )
        .is_ok()
    );
    let (code, stdout, stderr) = public(&rig, &["stop", "oqold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "oqold", "oqnew"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let new_ctx = format!("{}/opencode.main.md", rig.dir("oqnew").display());
    assert_eq!(
        std::fs::read_to_string(rig.dir("oqnew").join("opencode.main.json")).unwrap_or_default(),
        format!("{{\"instructions\":[\"{new_ctx}\"]}}\n"),
        "the required pair is generated byte-exact"
    );
    assert!(
        std::fs::read_to_string(rig.dir("oqnew").join("opencode.main.md"))
            .unwrap_or_default()
            .contains("Session: oqnew."),
        "with the new session sentence"
    );
}

/// BLOCKER (r4-B1): a carrier whose witness does not describe the directory
/// standing at the old address refuses BEFORE the move — a forged or stale
/// fingerprint never strands real work. Smallest defeating mutation: move
/// first, verify after (the old path is gone under a failure).
#[test]
fn a_mismatched_witness_refuses_before_the_move() {
    if skip() {
        return;
    }
    let rig = Rig::idle("witness-mismatch");
    let (code, stdout, stderr) = rig.launch(&["--copy", "wmold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join("wmold");
    assert!(std::fs::write(old_work.join("marker"), "mine\n").is_ok());
    let row = |key: &str| {
        rig.meta("wmold")
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    // Shape-valid carrier (full with a work witness) naming a fingerprint
    // no directory here has.
    let forged = format!(
        "rename_intent=1\nsession_id={}\nold=wmold\nnew=wmnew\nmode=full\nold_work={}\nnew_work={}\norigin={}\nserver_kind={}\nserver_value={}\nphase=prepared\nwork_dev=4242\nwork_ino=4242\nadmin_dev=0\nadmin_ino=0\n",
        row("session_id"),
        old_work.display(),
        rig.home.join("worktrees").join("wmnew").display(),
        row("origin"),
        row("tmux_server_kind"),
        row("tmux_server"),
    );
    assert!(
        std::fs::write(
            rig.home.join("sessions").join(".rename.wmold.wmnew.intent"),
            &forged
        )
        .is_ok()
    );
    let (code, stdout, stderr) = public(&rig, &["stop", "wmold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "wmold", "wmnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("does not match the recorded identity")
            && stderr.contains("refusing to move an unproved directory"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read(old_work.join("marker")).unwrap_or_default(),
        b"mine\n",
        "the old path is intact"
    );
    assert!(
        !rig.home.join("worktrees").join("wmnew").exists(),
        "no destination appears"
    );
    assert!(!rig.dir("wmnew").exists(), "no state move");
}

/// BLOCKER (r4-B1, git): a carrier whose admin fingerprint does not describe
/// the registered worktree refuses BEFORE the move. Smallest defeating
/// mutation: move first, verify after.
#[test]
fn a_mismatched_admin_witness_refuses_before_the_move() {
    if skip() {
        return;
    }
    let rig = Rig::idle("admin-mismatch");
    git_in(&rig.project, &["init", "-q"]);
    git_in(&rig.project, &["config", "user.email", "t@t"]);
    git_in(&rig.project, &["config", "user.name", "t"]);
    assert!(std::fs::write(rig.project.join("f"), "x\n").is_ok());
    git_in(&rig.project, &["add", "-A"]);
    git_in(&rig.project, &["commit", "-qm", "base"]);
    let (code, stdout, stderr) = rig.launch(&["--worktree", "waold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join("waold");
    let row = |key: &str| {
        rig.meta("waold")
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    let (work_dev, work_ino) = {
        let meta = std::fs::metadata(&old_work).expect("the work dir");
        (meta.dev(), meta.ino())
    };
    // True work witness, foreign admin witness: the move must still refuse.
    let forged = format!(
        "rename_intent=1\nsession_id={}\nold=waold\nnew=wanew\nmode=git\nold_work={}\nnew_work={}\norigin={}\nserver_kind={}\nserver_value={}\nphase=prepared\nwork_dev={work_dev}\nwork_ino={work_ino}\nadmin_dev=4242\nadmin_ino=4242\n",
        row("session_id"),
        old_work.display(),
        rig.home.join("worktrees").join("wanew").display(),
        row("origin"),
        row("tmux_server_kind"),
        row("tmux_server"),
    );
    assert!(
        std::fs::write(
            rig.home.join("sessions").join(".rename.waold.wanew.intent"),
            &forged
        )
        .is_ok()
    );
    let (code, stdout, stderr) = public(&rig, &["stop", "waold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "waold", "wanew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("does not match the recorded identity")
            && stderr.contains("refusing to move an unproved worktree"),
        "{stderr}"
    );
    assert!(old_work.is_dir(), "the old path is intact");
    assert!(
        !rig.home.join("worktrees").join("wanew").exists(),
        "no destination appears"
    );
    assert!(!rig.dir("wanew").exists(), "no state move");
}

/// Sweep ADD: a link planted at the new state address during the cut refuses
/// the retry — at the under-lock recheck — before `rename(2)` can overwrite
/// the unowned entry. The link and the old state are preserved. Smallest
/// defeating mutation: follow links at the destination (the link is
/// replaced). (`do_state_move` carries the same guard for the residual race
/// past the recheck; pinned at unit level.)
#[test]
fn a_planted_state_link_refuses_the_retry() {
    if skip() {
        return;
    }
    let rig = Rig::idle("planted-state-link");
    let (code, stdout, stderr) = rig.launch(&["--local", "psold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "psold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, "psold", "psnew", "after-intent");
    std::os::unix::fs::symlink("/nonexistent-target-under-test", rig.dir("psnew"))
        .expect("a planted destination link");

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "psold", "psnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("is a symlink"), "{stderr}");
    assert_eq!(
        std::fs::read_link(rig.dir("psnew")).unwrap_or_default(),
        PathBuf::from("/nonexistent-target-under-test"),
        "the planted link is preserved"
    );
    assert!(rig.dir("psold").is_dir(), "the old state is intact");
}

/// Sweep: recovery re-takes the strict absence proof — an unreachable
/// recorded server at retry refuses over unknown liveness instead of
/// converging blind. Smallest defeating mutation: skip the absence
/// re-proof (the retry converges).
#[test]
fn a_retry_over_an_unreachable_server_refuses_unknown_liveness() {
    if skip() {
        return;
    }
    let rig = Rig::idle("retry-unreachable");
    let (code, stdout, stderr) = rig.launch(&["--local", "ruold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "ruold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, "ruold", "runew", "after-intent");
    // The stale server socket is the only absence witness; unlink it.
    assert!(std::fs::remove_file(&rig.sock).is_ok());

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "ruold", "runew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("cannot prove") && stderr.contains("unreachable"),
        "{stderr}"
    );
    assert!(rig.dir("ruold").is_dir() && !rig.dir("runew").exists());
}

/// Sweep: recovery re-checks the destination name — a live namesake created
/// mid-transaction refuses the retry. Smallest defeating mutation: skip the
/// destination re-check (the retry collides).
#[test]
fn a_retry_over_an_occupied_name_refuses() {
    if skip() {
        return;
    }
    let rig = Rig::idle("retry-occupied");
    let (code, stdout, stderr) = rig.launch(&["--local", "roold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "roold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, "roold", "ronew", "after-intent");
    assert!(
        rig.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "ronew",
            "sleep",
            "60"
        ])
        .0,
        "a live namesake appears mid-transaction"
    );
    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "roold", "ronew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("already exists"), "{stderr}");
    assert!(rig.dir("roold").is_dir() && !rig.dir("ronew").exists());
}

/// Sweep: recovery re-takes worktrees-root authority — a swapped root
/// refuses the retry. Smallest defeating mutation: carry the root proof
/// from preflight (the retry moves through a link).
#[test]
fn a_retry_over_a_swapped_worktrees_root_refuses() {
    if skip() {
        return;
    }
    let rig = Rig::idle("retry-root");
    let (code, stdout, stderr) = rig.launch(&["--copy", "rwold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "rwold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, "rwold", "rwnew", "after-intent");
    let worktrees = rig.home.join("worktrees");
    let shadow = rig.scratch.join("shadow-trees");
    assert!(std::fs::create_dir_all(&shadow).is_ok());
    assert!(std::fs::remove_dir_all(&worktrees).is_ok());
    std::os::unix::fs::symlink(&shadow, &worktrees).expect("a swapped root");

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "rwold", "rwnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("not a real directory"), "{stderr}");
    // Restore for a clean rig teardown (rm -rf follows nothing here, but
    // leave no trap behind).
    assert!(std::fs::remove_file(&worktrees).is_ok());
    assert!(std::fs::create_dir_all(&worktrees).is_ok());
}

/// Sweep: recovery re-scans work sharing — a peer claiming the path
/// mid-transaction refuses the retry. Smallest defeating mutation: carry
/// the sharing proof from preflight.
#[test]
fn a_retry_over_shared_work_refuses() {
    if skip() {
        return;
    }
    let rig = Rig::idle("retry-shared");
    let (code, stdout, stderr) = rig.launch(&["--copy", "rshold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "rshold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, "rshold", "rshnew", "after-intent");
    // A peer session recording the same working copy (adversarial scanner
    // fixture: the scan input, not the renamed session's path).
    let peer = rig.home.join("sessions").join("rshpeer");
    assert!(std::fs::create_dir_all(&peer).is_ok());
    assert!(
        std::fs::write(
            peer.join("meta"),
            format!(
                "session=rshpeer\nmode=full\norigin=/x\nwork_dir={}\n",
                rig.home.join("worktrees").join("rshold").display()
            )
        )
        .is_ok()
    );
    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "rshold", "rshnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("records the same working copy"), "{stderr}");
    assert!(rig.dir("rshold").is_dir() && !rig.dir("rshnew").exists());
}

/// Sweep: recovery re-proves git registration — a lock placed
/// mid-transaction refuses the retry before any move. Smallest defeating
/// mutation: carry the registration proof from preflight.
#[test]
fn a_retry_over_a_locked_worktree_refuses() {
    if skip() {
        return;
    }
    let rig = Rig::idle("retry-locked");
    git_in(&rig.project, &["init", "-q"]);
    git_in(&rig.project, &["config", "user.email", "t@t"]);
    git_in(&rig.project, &["config", "user.name", "t"]);
    assert!(std::fs::write(rig.project.join("f"), "x\n").is_ok());
    git_in(&rig.project, &["add", "-A"]);
    git_in(&rig.project, &["commit", "-qm", "base"]);
    let (code, stdout, stderr) = rig.launch(&["--worktree", "rlold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let (code, stdout, stderr) = public(&rig, &["stop", "rlold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, "rlold", "rlnew", "after-intent");
    let old_work = rig.home.join("worktrees").join("rlold");
    git_in(
        &rig.project,
        &["worktree", "lock", &old_work.display().to_string()],
    );

    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "rlold", "rlnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("locked worktree"), "{stderr}");
    assert!(old_work.is_dir() && rig.dir("rlold").is_dir());
    git_in(
        &rig.project,
        &["worktree", "unlock", &old_work.display().to_string()],
    );
}

/// Sweep: recovery re-probes explicit homes while the state sits home — a
/// transcript layout that changed under the handoff refuses like a fresh
/// one. Smallest defeating mutation: probe once at preflight (the retry
/// newly breaks exact resume).
#[test]
fn a_retry_over_a_changed_transcript_layout_refuses() {
    if skip() {
        return;
    }
    let rig = Rig::new("retry-home", &["claude"], None);
    let (code, stdout, stderr) = rig.launch(&["--copy", "rhole"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join("rhole");
    let new_work = rig.home.join("worktrees").join("rhnew");
    let (home, id) = pin_explicit_home(&rig, "rhole", &old_work);
    // Both transcripts exist: the fresh decision would allow the move.
    plant_transcript(&home, &new_work, &id);
    let (code, stdout, stderr) = public(&rig, &["stop", "rhole"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, "rhole", "rhnew", "after-intent");
    // The candidate transcript vanishes mid-transaction: the retry must see
    // the newly-broken probe, not the preflight's green one.
    assert!(
        std::fs::remove_file(
            home.join("projects")
                .join(
                    new_work
                        .display()
                        .to_string()
                        .chars()
                        .map(|ch| if ch == '/' { '-' } else { ch })
                        .collect::<String>()
                )
                .join(format!("{id}.jsonl"))
        )
        .is_ok()
    );
    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "rhole", "rhnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("explicit config home"), "{stderr}");
    assert!(rig.dir("rhole").is_dir() && !rig.dir("rhnew").exists());
}

/// BLOCKER (r6): attributed digest damage blocks only its endpoints. A
/// valid A→B payload under a foreign digest stem refuses A-side lifecycle
/// loudly, while an unrelated fresh launch and an unrelated stopped rename
/// proceed. Smallest defeating mutation: drop the attributed-Err endpoint
/// filter (unrelated lifecycle/rename go red).
#[test]
fn a_wrong_digest_carrier_blocks_endpoints_only() {
    if skip() {
        return;
    }
    let rig = Rig::idle("wrong-digest");
    for session in ["wdA", "wdC"] {
        let (code, stdout, stderr) = rig.launch(&["--local", session]);
        assert_eq!(
            code,
            Some(0),
            "{session}: stdout: {stdout}\nstderr: {stderr}"
        );
        let (code, stdout, stderr) = public(&rig, &["stop", session]);
        assert_eq!(
            code,
            Some(0),
            "{session}: stdout: {stdout}\nstderr: {stderr}"
        );
    }
    let row = |session: &str, key: &str| {
        rig.meta(session)
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_default()
            .to_owned()
    };
    // A well-formed wdA→wdB carrier (wdA's own UUID, server, origin, work)
    // filed under a foreign digest stem: hostile input, valid signature,
    // wrong address.
    let ghost = format!(
        "rename_intent=1\nsession_id={}\nold=wdA\nnew=wdB\nmode=local\nold_work={work}\nnew_work={work}\norigin={work}\nserver_kind={kind}\nserver_value={value}\nphase=prepared\nwork_dev=0\nwork_ino=0\nadmin_dev=0\nadmin_ino=0\n",
        row("wdA", "session_id"),
        work = row("wdA", "work_dir"),
        kind = row("wdA", "tmux_server_kind"),
        value = row("wdA", "tmux_server"),
    );
    assert!(
        std::fs::write(
            rig.home
                .join("sessions")
                .join(".rename.ffffffffffffffff.intent"),
            &ghost
        )
        .is_ok()
    );
    // Endpoints block loudly with the attributed damage...
    let (code, _, stderr) = rig.launch(&["wdA"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("damaged rename carrier") && stderr.contains("wdA"),
        "{stderr}"
    );
    let (code, _, stderr) = public(&rig, &["end", "wdA", "-f", "--keep-history"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("damaged rename carrier"), "{stderr}");
    // ...while unrelated lifecycle and rename proceed.
    let (code, stdout, stderr) = rig.launch(&["--local", "wdX"]);
    assert_eq!(
        code,
        Some(0),
        "unrelated launch: stdout: {stdout}\nstderr: {stderr}"
    );
    let (code, stdout, stderr) = public(&rig, &[ae::cli::RENAME, "wdC", "wdD"]);
    assert_eq!(
        code,
        Some(0),
        "unrelated rename: stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("Renamed 'wdC' → 'wdD' (stopped;"),
        "{stdout}"
    );
}

/// IMPORTANT (r6): after an `after-intent` cut, a planted managed-work
/// destination refuses the retry before `rename(2)` — symlink and existing
/// directory alike — with old work, planted entry, and old state intact.
/// Smallest defeating mutation: drop the point-of-use destination recheck
/// (the retry overwrites).
#[test]
fn a_planted_work_destination_refuses_the_retry() {
    if skip() {
        return;
    }
    let rig = Rig::idle("planted-work-dest");
    let (code, stdout, stderr) = rig.launch(&["--copy", "pwold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let old_work = rig.home.join("worktrees").join("pwold");
    let new_work = rig.home.join("worktrees").join("pwnew");
    assert!(std::fs::write(old_work.join("marker"), "mine\n").is_ok());
    let (code, stdout, stderr) = public(&rig, &["stop", "pwold"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    kill_at_boundary(&rig, "pwold", "pwnew", "after-intent");
    // Case 1: a dangling symlink at the destination.
    std::os::unix::fs::symlink("/nonexistent-target-under-test", &new_work)
        .expect("a planted destination link");
    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "pwold", "pwnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("became a symlink mid-transaction"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read(old_work.join("marker")).unwrap_or_default(),
        b"mine\n",
        "old work intact"
    );
    assert_eq!(
        std::fs::read_link(&new_work).unwrap_or_default(),
        PathBuf::from("/nonexistent-target-under-test"),
        "planted link intact"
    );
    assert!(rig.dir("pwold").is_dir() && !rig.dir("pwnew").exists());
    // Case 2: an existing directory at the destination.
    assert!(std::fs::remove_file(&new_work).is_ok());
    assert!(std::fs::create_dir_all(&new_work).is_ok());
    assert!(std::fs::write(new_work.join("planted"), "not the session\n").is_ok());
    let (code, _, stderr) = public(&rig, &[ae::cli::RENAME, "pwold", "pwnew"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("appeared mid-transaction"), "{stderr}");
    assert_eq!(
        std::fs::read(old_work.join("marker")).unwrap_or_default(),
        b"mine\n",
        "old work intact"
    );
    assert_eq!(
        std::fs::read(new_work.join("planted")).unwrap_or_default(),
        b"not the session\n",
        "planted entry intact"
    );
    assert!(rig.dir("pwold").is_dir() && !rig.dir("pwnew").exists());
    // Clearing the plant unblocks: the block is evidence, not a latch.
    assert!(std::fs::remove_dir_all(&new_work).is_ok());
    retry_rename(&rig, "pwold", "pwnew");
    assert_eq!(
        std::fs::read(new_work.join("marker")).unwrap_or_default(),
        b"mine\n",
        "the work converges once the plant is gone"
    );
}

/// The codex handshake: the tool writes `codex.<slot>.sid`, and the capture
/// pass turns it into the roster's `harness_session.main`.
#[test]
fn a_codex_launch_captures_the_session_id_it_registers() {
    if skip() {
        return;
    }
    let rig = Rig::new("cap", &["codex"], None);
    let (code, stdout, stderr) = rig.launch(&["--local", "cap"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let meta = rig.meta("cap");
    assert!(meta.contains("harness_session.main=pending"), "{meta}");
    let launch_id = meta
        .lines()
        .find_map(|line| line.strip_prefix("launch_id.main="))
        .expect("the codex launch token");
    let id = "0199c0de-1234-4890-abcd-ef0123456789";
    let day = ae::time::Timestamp::now().to_string()[..10].replace('-', "/");
    let started = ae::time::Timestamp::now();
    let logs = rig.scratch.join(".codex").join("sessions").join(day);
    assert!(
        std::fs::create_dir_all(&logs).is_ok(),
        "a rollout directory"
    );
    assert!(
        std::fs::write(
            logs.join(format!("rollout-{id}.jsonl")),
            format!(
                "{{\"timestamp\":\"{started}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{}\"}}}}\n\
                 {{\"text\":\"AE_CODEX_LAUNCH_ID={launch_id}\"}}\n",
                rig.project.display()
            )
        )
        .is_ok(),
        "the launch's rollout"
    );
    let registered = helper(&rig.dir("cap").join("_register-sid"))
        .arg("main")
        .env("HOME", &rig.scratch)
        .output()
        .unwrap_or_else(|why| panic!("the handshake should run: {why}"));
    assert!(
        registered.status.success(),
        "the launch-token handshake: {}",
        String::from_utf8_lossy(&registered.stderr)
    );
    // The handshake commits immediately; tolerate scheduling around the shim.
    for _ in 0..80 {
        if rig
            .meta("cap")
            .contains(&format!("harness_session.main={id}"))
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!("the capture never registered the id:\n{}", rig.meta("cap"));
}

/// A rollout can be created as soon as codex takes the pane, before ae's
/// post-exec process observation finishes. Its immutable birth must still be
/// on or after the floor published before exec.
#[test]
fn a_fresh_codex_rollout_born_before_the_post_exec_stamp_is_captured() {
    if skip() {
        return;
    }
    let rig = Rig::new("capture-floor-fresh", &["codex"], None);
    let (code, stdout, stderr) = rig.launch(&["--local", "lncapfloor"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let meta = rig.meta("lncapfloor");
    let floor = meta
        .lines()
        .find_map(|line| line.strip_prefix("capture_floor.main="))
        .and_then(|value| value.parse::<i64>().ok())
        .expect("a pre-exec capture floor");
    let launch_time = meta
        .lines()
        .find_map(|line| line.strip_prefix("launch_time.main="))
        .and_then(|value| value.parse::<i64>().ok())
        .expect("the post-exec launch stamp");
    assert!(
        floor <= launch_time,
        "floor={floor}, launch_time={launch_time}"
    );
    ae::meta::rewrite(
        &rig.dir("lncapfloor"),
        "launch_time.main",
        Some(&(floor + 1).to_string()),
    )
    .expect("make the post-exec ordering observable");

    let launch_id = meta
        .lines()
        .find_map(|line| line.strip_prefix("launch_id.main="))
        .expect("the launch token");
    let id = "0199c0de-1111-4890-abcd-ef0123456789";
    let day = ae::time::Timestamp::from_epoch(floor).to_string()[..10].replace('-', "/");
    let rollout = rig
        .scratch
        .join(".codex")
        .join("sessions")
        .join(day)
        .join(format!("rollout-{id}.jsonl"));
    assert!(
        std::fs::create_dir_all(rollout.parent().unwrap_or(&rig.scratch)).is_ok(),
        "a rollout directory"
    );
    assert!(
        std::fs::write(
            &rollout,
            format!(
                "{{\"timestamp\":\"{}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{}\"}}}}\n\
                 {{\"text\":\"AE_CODEX_LAUNCH_ID={launch_id}\"}}\n",
                ae::time::Timestamp::from_epoch(floor),
                rig.project.display()
            )
        )
        .is_ok(),
        "the launch's rollout"
    );
    let registered = helper(&rig.dir("lncapfloor").join("_register-sid"))
        .args(["main", id])
        .env("HOME", &rig.scratch)
        .output()
        .unwrap_or_else(|why| panic!("the handshake should run: {why}"));
    assert!(
        registered.status.success(),
        "the pre-exec floor admits the rollout: {}",
        String::from_utf8_lossy(&registered.stderr)
    );
}

/// An exact resume reopens the already-proved rollout. A later lifecycle stamp
/// must not move the lower bound beyond that conversation's immutable birth.
#[test]
fn an_exact_codex_resume_keeps_the_original_capture_floor() {
    if skip() {
        return;
    }
    let rig = Rig::new("capture-floor-resume", &["codex"], None);
    let (code, stdout, stderr) = rig.launch(&["--local", "lncapresume"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let first = rig.meta("lncapresume");
    let floor = first
        .lines()
        .find_map(|line| line.strip_prefix("capture_floor.main="))
        .and_then(|value| value.parse::<i64>().ok())
        .expect("the original capture floor");
    let launch_id = first
        .lines()
        .find_map(|line| line.strip_prefix("launch_id.main="))
        .expect("the launch token");
    let id = "0199c0de-2222-4890-abcd-ef0123456789";
    let day = ae::time::Timestamp::from_epoch(floor).to_string()[..10].replace('-', "/");
    let rollout = rig
        .scratch
        .join(".codex")
        .join("sessions")
        .join(day)
        .join(format!("rollout-{id}.jsonl"));
    assert!(
        std::fs::create_dir_all(rollout.parent().unwrap_or(&rig.scratch)).is_ok(),
        "a rollout directory"
    );
    assert!(
        std::fs::write(
            &rollout,
            format!(
                "{{\"timestamp\":\"{}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{}\"}}}}\n\
                 {{\"text\":\"AE_CODEX_LAUNCH_ID={launch_id}\"}}\n",
                ae::time::Timestamp::from_epoch(floor),
                rig.project.display()
            )
        )
        .is_ok(),
        "the original rollout"
    );
    let first_registration = helper(&rig.dir("lncapresume").join("_register-sid"))
        .args(["main", id])
        .env("HOME", &rig.scratch)
        .output()
        .unwrap_or_else(|why| panic!("the first handshake should run: {why}"));
    assert!(
        first_registration.status.success(),
        "{first_registration:?}"
    );
    assert!(
        rig.tmux(&["kill-session", "-t", "=lncapresume"]).0,
        "stop before exact resume"
    );

    let (code, stdout, stderr) = rig.launch(&["--local", "lncapresume"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let resumed = rig.meta("lncapresume");
    assert!(
        resumed.contains(&format!("capture_floor.main={floor}\n")),
        "exact resume moved or dropped the proved origin: {resumed}"
    );
    let registered = helper(&rig.dir("lncapresume").join("_register-sid"))
        .args(["main", id])
        .env("HOME", &rig.scratch)
        .output()
        .unwrap_or_else(|why| panic!("the resumed handshake should run: {why}"));
    assert!(
        registered.status.success(),
        "the retained rollout remains admissible: {}",
        String::from_utf8_lossy(&registered.stderr)
    );
}

/// `--glue` is GONE, and an unknown flag is refused exactly as before.
#[test]
fn the_retired_glue_flag_is_refused_before_any_side_effect() {
    let out = ae()
        .arg(ae::cli::LAUNCH)
        .args([
            "--home",
            "/nonexistent",
            "--cwd",
            "/nonexistent",
            "--glue",
            "/bin/ae",
        ])
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    assert_eq!(out.status.code(), Some(2), "a usage refusal");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("offending word: --glue"),
        "the refusal names the word: {stderr}"
    );
}

/// A resume whose spawned seat names a profile the CURRENT config defines as
/// two commands.
fn resumable_rig(tag: &str, session: &str, profile: &str) -> (Rig, PathBuf) {
    let rig = Rig::idle(tag);
    let _keep = bare_session(&rig, &rig.sock, &format!("keep-{session}"));
    let marker = rig.home.join("MARKER");
    let mut config = std::fs::read_to_string(&rig.config).unwrap_or_default();
    // A SECOND `[profiles]` header: the rig's config ends inside `[workspace]`,
    // and a bare `key = value` appended there is a workspace key, not a profile.
    config.push_str("\n[profiles]\n");
    config.push_str(&profile.replace("__MARKER__", &marker.display().to_string()));
    assert!(std::fs::write(&rig.config, config).is_ok(), "a bad profile");
    // A STOPPED session: meta on disk, nothing running.
    let dir = rig.dir(session);
    assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
    assert!(
        std::fs::write(
            dir.join("meta"),
            format!(
                "meta_version={version}\nsession={session}\ntmux_server_kind=socket\n\
                 tmux_server={server}\nmode=local\nlayout=vertical\n\
                 work_dir={home}\norigin={home}\nschema=2\nseat.main=lead\n\
                 profile.main=idle\nagent_bin.main=sleep\nconfig_home.main=absent\n\
                 seat.spawned.0=helper\nprofile.spawned.0=bad\nagent_bin.spawned.0=sleep\n\
                 config_home.spawned.0=implicit:{home}/.agent\nconfig_home_base.spawned.0={home}\n",
                version = ae::migrate::CURRENT,
                server = rig.sock.display(),
                home = rig.project.display(),
            ),
        )
        .is_ok(),
        "a v2 meta with a spawned seat"
    );
    (rig, marker)
}

#[test]
fn a_restored_spawned_seat_whose_profile_is_two_commands_refuses_the_whole_resume() {
    if skip() {
        return;
    }
    let (rig, marker) = resumable_rig(
        "spwbad",
        "lnspwb",
        "bad = \"/usr/bin/touch __MARKER__ ; sleep 600\"\n",
    );

    let (code, stdout, stderr) = rig.launch(&["--local", "lnspwb"]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("profile 'bad' refused") && stderr.contains("Nothing was resumed"),
        "the refusal names the profile and the seat: {stderr}"
    );
    assert!(stderr.contains("helper"), "and the seat: {stderr}");
    assert!(
        !marker.exists(),
        "the second command must never have run: {}",
        marker.display()
    );
    // BEFORE ANY EFFECT: no session, and no pane for the seat.
    let live = rig.sessions();
    assert!(
        !live.iter().any(|name| name == "lnspwb"),
        "nothing was started: {live:?}"
    );
}

/// The control: a restored seat whose profile is ONE command still resumes, and
/// still gets its pane.
#[test]
fn a_restored_spawned_seat_with_a_valid_profile_still_resumes() {
    if skip() {
        return;
    }
    let (rig, marker) = resumable_rig("spwok", "lnspwo", "bad = \"sleep 600\"\n");

    let (code, stdout, stderr) = rig.launch(&["--local", "lnspwo"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(!marker.exists(), "nothing ran a second command");
    let panes = rig.panes("lnspwo");
    assert!(
        panes.iter().any(|(_, slot, _)| slot == "spawned.0"),
        "the restored seat gets its pane back: {panes:?}"
    );
    let rebuilt = rig.meta("lnspwo");
    assert!(rebuilt.contains("config_home.main=absent\n"), "{rebuilt}");
    assert!(
        rebuilt.contains(&format!(
            "config_home.spawned.0=implicit:{}/.agent\n",
            rig.project.display()
        )),
        "restored spawned seats carry the named row: {rebuilt}"
    );
    assert!(
        rebuilt.contains(&format!(
            "config_home_base.spawned.0={}\n",
            rig.project.display()
        )),
        "restored spawned seats carry the implicit HOME row: {rebuilt}"
    );
}

/// A seat name a human typed into the meta reaches a pane border, and the
/// border reads an option VALUE, which tmux takes styles out of.
///
/// The RESUME is where this bites: a name in the config passed
/// `config::is_agent_name` on the way in, and a name read back off the meta
/// never did. So the identity and the drawn name are two options, and only the
/// second is rewritten.
#[test]
fn a_hostile_seat_name_in_a_resumed_meta_cannot_style_a_pane_border() {
    if skip() {
        return;
    }
    let rig = Rig::idle("hostile");
    let session = "lnhost";
    let _keep = bare_session(&rig, &rig.sock, "keep-ln-host");
    let dir = rig.dir(session);
    assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
    let hostile = "evil#[bg=red]";
    assert!(
        std::fs::write(
            dir.join("meta"),
            format!(
                "meta_version={version}\nsession={session}\ntmux_server_kind=socket\n\
                 tmux_server={server}\nmode=local\nlayout=vertical\n\
                 work_dir={home}\norigin={home}\nschema=2\nseat.main={hostile}\n\
                 profile.main=idle\nagent_bin.main=sleep\n",
                version = ae::migrate::CURRENT,
                server = rig.sock.display(),
                home = rig.project.display(),
            ),
        )
        .is_ok(),
        "a meta somebody edited by hand"
    );

    let (code, stdout, stderr) = rig.launch(&["--local", session]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let pane = |name: &str| {
        rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            &format!("{session}:0.0"),
            &format!("#{{{name}}}"),
        ])
        .1
        .trim_end_matches('\n')
        .to_owned()
    };
    // The IDENTITY is verbatim — the roster, the monitor and every lookup match
    // on it, and rewriting it would break the session rather than protect it.
    assert_eq!(pane("@ae_agent"), hostile);
    // The DRAWN name cannot carry a directive.
    let label = pane(ae::theme::AGENT_LABEL_OPTION);
    assert!(!label.contains('#'), "the label is inert: {label}");
    assert!(label.contains("evil"), "and still names the seat: {label}");
    // And the border reads the label, not the identity.
    let format = rig
        .tmux(&["show-options", "-wv", "-t", session, "pane-border-format"])
        .1;
    assert!(
        format.contains(ae::theme::AGENT_LABEL_OPTION),
        "the border draws the label: {format}"
    );
    assert!(
        !format.contains("#{@ae_agent}"),
        "and never the raw identity: {format}"
    );
}

/// An unusable `--server-kind` is refused before anything is built.
#[test]
fn an_unusable_server_pair_is_refused_before_the_session_is_built() {
    if skip() {
        return;
    }
    let rig = Rig::idle("kindbad");
    for (kind, value, expected) in [
        ("ambiguous", "work", "'ambiguous' is not a tmux server kind"),
        ("bogus", "work", "'bogus' is not a tmux server kind"),
        ("socket", "", "--server-kind socket needs a --server value"),
        ("name", "", "--server-kind name needs a --server value"),
        ("", "work", "--server was given without a --server-kind"),
    ] {
        let (code, stdout, stderr) = rig.launch_with_server(kind, value, &["--local", "lnkind"]);
        assert_eq!(
            code,
            Some(2),
            "kind '{kind}' value '{value}': stdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stderr.contains(expected) && stderr.contains("ambient"),
            "kind '{kind}': the refusal names what it could not use and says it did not \
             fall back: {stderr}"
        );
        // BEFORE ANY EFFECT: no state directory, and nothing on the server the
        // fallback would have used.
        assert!(
            !rig.dir("lnkind").exists(),
            "kind '{kind}': no session state was written"
        );
        let ambient = rig.sessions_on(&[]);
        assert!(
            !ambient.iter().any(|name| name == "lnkind"),
            "kind '{kind}': nothing was built on the ambient server: {ambient:?}"
        );
    }
}

/// The control: both typed kinds still reach their own server.
#[test]
fn a_typed_server_pair_still_reaches_its_own_server() {
    if skip() {
        return;
    }
    let rig = Rig::idle("kindok");

    // Socket: the rig's own, which every other arm here already depends on.
    let (code, stdout, stderr) = rig.launch_with_server(
        "socket",
        &rig.sock.display().to_string(),
        &["--local", "lnsock"],
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.sessions().iter().any(|name| name == "lnsock"),
        "the socket server holds it"
    );
    // And the hint names that server, because a session on one is invisible to
    // a bare `tmux attach`.
    assert!(
        stdout.contains(&format!(
            "Attach with: tmux -S {} attach -t \"=lnsock\"",
            rig.sock.display()
        )),
        "{stdout}"
    );

    // Name: a `-L` server, which nothing else here exercises.
    let named = format!("aeln{}", std::process::id());
    let (code, stdout, stderr) = rig.launch_with_server("name", &named, &["--local", "lnnamed"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let held = rig.sessions_on(&["-L", &named]);
    rig.kill_server_at(&["-L", &named]);
    assert!(
        held.iter().any(|name| name == "lnnamed"),
        "the named server holds it: {held:?}"
    );
    assert!(
        stdout.contains(&format!(
            "Attach with: tmux -L {named} attach -t \"=lnnamed\""
        )),
        "{stdout}"
    );
}

/// The roster + workspace a LEAD layout needs: one sleeper profile, `count`
/// workers beside the lead, and the layout under test.
fn lead_config(layout: &str, workers: &[&str]) -> String {
    let mut cfg = String::from("[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n");
    for worker in workers {
        let _ = writeln!(cfg, "{worker} = idle");
    }
    let _ = write!(
        cfg,
        "\n[workspace]\nmain = lead\nworkers = {}\nlayout = {layout}\nwatchdog = false\n",
        workers.join(", ")
    );
    cfg
}

fn assert_equal_widths(rig: &Rig, target: &str) {
    let (_, listed) = rig.tmux(&["list-panes", "-t", target, "-F", "#{pane_width}"]);
    let widths = listed
        .lines()
        .filter_map(|width| width.parse::<usize>().ok())
        .collect::<Vec<_>>();
    assert_eq!(widths.len(), 2, "two lead-pair panes: {listed}");
    assert!(
        widths[0].abs_diff(widths[1]) <= 1,
        "lead panes differ by more than one cell: {listed}"
    );
}

fn assert_resize_preserves_colead_zoom(rig: &Rig, session: &str) {
    let colead_pane = rig
        .panes(session)
        .into_iter()
        .find(|(_, slot, _)| slot == "worker.0")
        .map_or_else(|| panic!("a colead pane"), |(pane, _, _)| pane);
    assert!(
        rig.tmux(&["resize-pane", "-Z", "-t", &colead_pane]).0,
        "zoom the colead"
    );
    assert!(
        rig.tmux(&["resize-window", "-t", session, "-x", "150", "-y", "40"])
            .0,
        "resize the zoomed lead-pair window"
    );
    let (_, zoomed) = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        &colead_pane,
        "#{window_zoomed_flag}",
    ]);
    assert_eq!(zoomed.trim(), "1", "a resize must not unzoom the colead");
}

/// The two LEAD layouts seat each agent together and keep each window's first
/// agent as its stable routing name.
#[test]
fn the_lead_layouts_seat_each_agent_in_the_window_their_layout_names() {
    if skip() {
        return;
    }

    // ── lead-solo: the lead is alone in window 0, both workers share window 1.
    let solo = Rig::new("solo", &[], None);
    assert!(
        std::fs::write(&solo.config, lead_config("lead-solo", &["w1", "w2"])).is_ok(),
        "a lead-solo config"
    );
    let (code, stdout, stderr) = solo.launch(&["--local", "lsolo"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        solo.windows("lsolo"),
        vec![
            ("0".to_owned(), "lead".to_owned(), 1),
            ("1".to_owned(), "w1".to_owned(), 2),
        ],
        "each window keeps the name of the first agent placed in it"
    );
    let (_, automatic) = solo.tmux(&[
        "show-window-options",
        "-v",
        "-t",
        "lsolo:0",
        "automatic-rename",
    ]);
    assert_eq!(automatic.trim(), "off", "the singleton name stays stable");
    assert!(
        solo.meta("lsolo").contains("layout=lead-solo"),
        "the layout is pinned, so a resume keeps this shape:\n{}",
        solo.meta("lsolo")
    );

    // ── lead-pair: the colead joins the lead in window 0 as an equal seat.
    let pair = Rig::new("pair", &[], None);
    assert!(
        std::fs::write(
            &pair.config,
            lead_config("lead-pair", &["colead", "builder", "reviewer"]),
        )
        .is_ok(),
        "a lead-pair config"
    );
    let (code, stdout, stderr) = pair.launch(&["--local", "lpair"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        pair.windows("lpair"),
        vec![
            ("0".to_owned(), "lead".to_owned(), 2),
            ("1".to_owned(), "builder".to_owned(), 2),
        ],
        "a later split does not rename either window"
    );
    // The colead really is the FIRST worker slot, and it really is in the
    // lead's window — a shape assertion alone would pass if the panes were
    // stamped the other way round.
    let (_, colead) = pair.tmux(&[
        "list-panes",
        "-t",
        "lpair:0",
        "-F",
        "#{@ae_slot}|#{@ae_agent}",
    ]);
    assert!(
        colead.lines().any(|row| row == "worker.0|colead"),
        "the colead seat is in window 0: {colead}"
    );
    let (_, main_width) = pair.tmux(&[
        "show-window-options",
        "-v",
        "-t",
        "lpair:0",
        "main-pane-width",
    ]);
    assert_eq!(
        main_width.trim(),
        "50%",
        "the lead-pair width stays percentage-based across resizes"
    );
    let (_, resize_hook) = pair.tmux(&["show-hooks", "-w", "-t", "lpair:0", "window-resized"]);
    assert!(
        resize_hook.contains("select-layout -t") && resize_hook.contains("main-vertical"),
        "the lead-pair window reapplies its percentage after a resize: {resize_hook}"
    );
    assert_resize_preserves_colead_zoom(&pair, "lpair:0");
    assert!(
        pair.meta("lpair").contains("layout=lead-pair"),
        "the layout is pinned:\n{}",
        pair.meta("lpair")
    );
}

#[test]
fn a_running_lead_pair_reasserts_the_resize_policy_without_a_restart() {
    if skip() {
        return;
    }
    let pair = Rig::new("pair-reattach", &[], None);
    assert!(
        std::fs::write(&pair.config, lead_config("lead-pair", &["colead"])).is_ok(),
        "a lead-pair config"
    );
    let (code, stdout, stderr) = pair.launch(&["--local", "lpairlive"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let main_pane = pair
        .panes("lpairlive")
        .into_iter()
        .find(|(_, slot, _)| slot == "main")
        .map_or_else(|| panic!("a lead pane"), |(pane, _, _)| pane);

    assert!(
        pair.tmux(&["set-hook", "-wu", "-t", &main_pane, "window-resized"])
            .0,
        "remove the new policy to model a pre-upgrade session"
    );
    assert!(
        pair.tmux(&[
            "set-window-option",
            "-t",
            &main_pane,
            "main-pane-width",
            "50%"
        ])
        .0,
        "restore the old equal width"
    );
    assert!(
        pair.tmux(&["select-layout", "-t", &main_pane, "even-horizontal"])
            .0,
        "restore the old equal layout"
    );

    let (code, stdout, stderr) = pair.launch(&["--local", "lpairlive"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("is running"),
        "the live reattach path: {stdout}"
    );
    let (_, main_width) = pair.tmux(&[
        "show-window-options",
        "-v",
        "-t",
        &main_pane,
        "main-pane-width",
    ]);
    assert_eq!(main_width.trim(), "50%", "reattach restores pair width");
    let (_, resize_hook) = pair.tmux(&["show-hooks", "-w", "-t", &main_pane, "window-resized"]);
    assert!(
        resize_hook.contains("window_zoomed_flag") && resize_hook.contains("main-vertical"),
        "reattach restores the guarded resize hook: {resize_hook}"
    );
    assert!(
        pair.tmux(&[
            "resize-window",
            "-t",
            "lpairlive:0",
            "-x",
            "150",
            "-y",
            "40"
        ])
        .0,
        "resize the repaired window"
    );
    assert_equal_widths(&pair, "lpairlive:0");
}

/// `[workspace] theme = off` writes the FACTS and NONE of the layout.
///
/// The opt-out is only worth having if it is total: a session that still gets
/// `pane-border-status` or a `status-format` has taken the user's own look away
/// while claiming not to. Every option ae would have written is asserted absent
/// at the scope ae would have written it.
#[test]
fn a_session_with_the_theme_off_keeps_the_users_own_look() {
    if skip() {
        return;
    }
    let rig = Rig::idle("themeoff");
    assert!(
        std::fs::write(&rig.config, format!("{IDLE_CONFIG}theme = off\n")).is_ok(),
        "a config with the look turned off"
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "lnoff"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    // `show-options -v` with no session-scoped value prints nothing at all.
    let session = |name: &str| {
        rig.tmux(&["show-options", "-v", "-t", "lnoff", name])
            .1
            .trim_end_matches('\n')
            .to_owned()
    };
    for name in ae::theme::LAYOUT_OPTIONS {
        assert!(
            session(name).is_empty(),
            "{name} was written on a session whose look is off"
        );
    }
    // Every window, the monitor window included — that one was dressed by hand
    // before the look existed, and it is the regression this test names. The
    // plumbing identity is not a look option and remains present so a custom
    // status line can distinguish that window without relying on its name.
    let (_, windows) = rig.tmux(&["list-windows", "-t", "lnoff", "-F", "#{window_name}"]);
    assert!(
        windows.lines().any(|name| name == "ae-monitor"),
        "the monitor window is part of the scene: {windows}"
    );
    for window in ["lnoff:0", "lnoff:ae-monitor"] {
        for name in ae::theme::window_option_names() {
            let (_, value) = rig.tmux(&["show-options", "-wv", "-t", window, name.as_str()]);
            assert!(
                value.trim().is_empty(),
                "{name} was written on {window} with the look off: {value}"
            );
        }
    }
    let (_, plumbing) = rig.tmux(&[
        "show-options",
        "-wv",
        "-t",
        "lnoff:ae-monitor",
        ae::theme::WINDOW_PLUMBING_OPTION,
    ]);
    assert_eq!(plumbing.trim(), "1");
    // And the FACTS are all there, so a hand-written status line can read them.
    assert_eq!(session(ae::theme::LOOK_OPTION), "off");
    assert_eq!(session(ae::theme::PALETTE_OPTION), "darcula");
    assert!(!session(ae::theme::ATTENTION_GLYPH_OPTION).is_empty());
    assert!(!session(ae::theme::PATHS_OPTION).is_empty());

    // Input policy is not part of the look: the capability-selected status
    // bindings and picker hotkey remain.
    assert_ae_status_bindings(&rig);
}

#[test]
fn an_agent_named_ae_monitor_keeps_its_window_while_the_plumbing_window_stays_hidden() {
    if skip() {
        return;
    }
    let rig = Rig::idle("monitor-name");
    let config = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nae-monitor = idle\n\n\
                  [workspace]\nmain = ae-monitor\nlayout = vertical\nwatchdog = false\n";
    assert!(
        std::fs::write(&rig.config, config).is_ok(),
        "a valid agent name that matches the plumbing window"
    );
    let (code, stdout, stderr) = rig.launch(&["--local", "monitor-name"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (_, windows) = rig.tmux(&[
        "list-windows",
        "-t",
        "monitor-name",
        "-F",
        "#{window_index}|#{window_name}|#{@ae_window_plumbing}",
    ]);
    assert!(
        windows.lines().any(|line| line == "0|ae-monitor|"),
        "{windows}"
    );
    assert!(
        windows.lines().any(|line| line == "99|ae-monitor|1"),
        "{windows}"
    );

    let (_, format) = rig.tmux(&[
        "show-options",
        "-v",
        "-t",
        "monitor-name",
        "status-format[0]",
    ]);
    let (_, drawn) = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        "monitor-name:0",
        "-F",
        format.trim_end(),
    ]);
    assert!(
        drawn.contains("0:ae-monitor"),
        "agent window missing: {drawn}"
    );
    assert!(!drawn.contains("99:ae-monitor"), "plumbing leaked: {drawn}");
}

/// The status bar is AE-OWNED, SESSION-SCOPED, and its first line still
/// RENDERS.
#[test]
fn the_status_bar_is_ae_owned_and_its_first_line_still_renders() {
    if skip() {
        return;
    }
    let rig = Rig::idle("bar");
    let (code, stdout, stderr) = rig.launch(&["--local", "lnbar"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let option = |name: &str| {
        rig.tmux(&["show-options", "-v", "-t", "lnbar", name])
            .1
            .trim_end_matches('\n')
            .to_owned()
    };
    // Line 0 is ae's own: the windows and the right-hand facts. The session
    // NAME is not on it — the fleet strip on line 1 names
    // every session and raises this one — so the format depends on the look
    // alone. The watch segment is a user option at the END, referenced exactly
    // once, so a watchdog restart cannot double it.
    let zero = option("status-format[0]");
    assert!(zero.starts_with("#[align=left]#[nobold"), "{zero}");
    assert!(!zero.contains("#{@ae_attn_style}"), "{zero}");
    assert!(!zero.contains("#{@ae_attn_glyph}"), "{zero}");
    assert!(!zero.contains("lnbar"), "{zero}");
    assert!(zero.contains("#{window_name}"), "{zero}");
    assert!(zero.contains("#{@ae_window_agents}"), "{zero}");
    assert!(zero.contains("#{@ae_branch_status}"), "{zero}");
    assert_eq!(
        zero.matches("#{@ae_watchdog_status}").count(),
        1,
        "exactly one watch reference: {zero}"
    );
    assert!(!zero.contains("#("), "no format ever shells out: {zero}");
    // Line 1 is the fleet strip and the core version. Agents live in their
    // window entries on line 0.
    let one = option("status-format[1]");
    assert!(one.contains("#{@ae_fleet_strip}"), "{one}");
    assert!(!one.contains(concat!("@ae_agents_", "status")), "{one}");
    // PRESENT is not RENDERED. Both lines are drawn and read back, because a
    // format that tmux cannot expand does not fail — it prints the source text,
    // and a bar with `#{` still in it is a broken bar that a "contains the
    // session name" assertion would happily pass.
    let render = |index: u8| {
        rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            "lnbar:0",
            &format!("#{{T:status-format[{index}]}}"),
        ])
        .1
    };
    let drawn = render(0);
    assert!(
        !drawn.contains("#{"),
        "line 0 left a format unexpanded: {drawn:?}"
    );
    assert!(
        drawn.contains("lead"),
        "line 0 draws the window's agent rather than a blank line: {drawn:?}"
    );
    assert!(
        drawn.contains("0:lead"),
        "the fallback window name is its single agent: {drawn:?}"
    );
    assert!(
        drawn.contains("range=window|0"),
        "the window entry is a click target: {drawn:?}"
    );
    // The attention seed remains published for borders, menus, and fleet
    // strip, but line 0 starts directly with the window list.
    // Both watchdog-owned surfaces render their option values. The rig runs no
    // watchdog, so seed them directly.
    let (set, why) = rig.tmux(&[
        "set-option",
        "-t",
        "lnbar",
        ae::theme::FLEET_STRIP_OPTION,
        "FLEETMARK lnbar",
    ]);
    assert!(set, "seeding fleet strip: {why}");
    let (set, why) = rig.tmux(&[
        "set-option",
        "-w",
        "-t",
        "lnbar:0",
        ae::theme::WINDOW_AGENTS_OPTION,
        "AGENTMARK lead",
    ]);
    assert!(set, "seeding window agents: {why}");
    let agents = render(0);
    assert!(agents.contains("0:AGENTMARK lead"), "{agents:?}");
    let strip = render(1);
    assert!(
        !strip.contains("#{"),
        "line 1 left a format unexpanded: {strip:?}"
    );
    assert!(
        strip.contains("FLEETMARK lnbar"),
        "line 1 draws the fleet strip: {strip:?}"
    );
}

/// The per-window half of the look, and the tables ae must never write.
#[test]
fn the_window_half_of_the_look_is_stamped_per_window_and_never_globally() {
    if skip() {
        return;
    }
    let rig = Rig::idle("winlook");
    let (code, stdout, stderr) = rig.launch(&["--local", "lnwin"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let option = |name: &str| {
        rig.tmux(&["show-options", "-v", "-t", "lnwin", name])
            .1
            .trim_end_matches('\n')
            .to_owned()
    };
    // Stamped on the WINDOW, because tmux keeps pane-border and menu styles
    // there — and a `set -t <session>` would reach only the current window.
    let window = |name: &str| {
        rig.tmux(&["show-options", "-wv", "-t", "lnwin:0", name])
            .1
            .trim_end_matches('\n')
            .to_owned()
    };
    assert_eq!(window("pane-border-status"), "top");
    assert_eq!(window("pane-border-lines"), "heavy");
    assert!(
        window("pane-border-format").contains("#{@ae_agent_label}")
            && !window("pane-border-format").contains("#{@ae_profile}"),
        "the border names the agent and its state, never the profile: {}",
        window("pane-border-format")
    );
    assert_eq!(
        window("@ae_theme"),
        ae::theme::window_stamp(&ae::theme::Look::DEFAULT),
        "the stamp names the LOOK the window was dressed in, not just that it was"
    );
    assert!(!window("menu-style").is_empty(), "the picker is themed too");
    // The MONITOR window is stamped like every other one — it is the window a
    // caller once dressed by hand, and the look owns those options now.
    let monitor = rig
        .tmux(&["show-options", "-wv", "-t", "lnwin:ae-monitor", "@ae_theme"])
        .1
        .trim_end_matches('\n')
        .to_owned();
    assert_eq!(
        monitor,
        ae::theme::window_stamp(&ae::theme::Look::DEFAULT),
        "the monitor window too"
    );
    // The LOOK STAMP says what the layout was written for, which is what the
    // watchdog compares against to notice a knob turned on a live session.
    assert_eq!(option(ae::theme::LOOK_STAMP_OPTION), "19:darcula:on:on");
    // The GLOBAL tables stay the operator's — every ae option is written at
    // session or window scope, never at `-g`.
    for (flags, name) in [
        (["show-options", "-gv"], "status-format[0]"),
        (["show-options", "-gwv"], "pane-border-format"),
        (["show-options", "-gwv"], "menu-style"),
    ] {
        let (_, global) = rig.tmux(&[flags[0], flags[1], name]);
        assert!(
            !global.contains("@ae_"),
            "ae does not theme the global table: {name} = {global}"
        );
    }
}
