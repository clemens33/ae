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
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::cli::{ae, git_in, helper};
use super::phase2::run_tmux;

/// A TUI-shaped fake agent: it records its argv, then sits there drawing the
/// ornament the input sensor reads. Nothing here is claude-specific — the
/// FILE NAME decides the classification, so the same body serves as `claude`
/// and as `codex`.
const FAKE_AGENT: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
system("stty raw -echo 2>/dev/null");
binmode(STDIN, ':raw');
binmode(STDOUT, ':raw');
$| = 1;
open(my $log, '>>', "__LAUNCHED__") or die; print $log join(" ", @ARGV), "\n"; close($log);
if (length("__SID__")) {
    open(my $sid, '>', "__SID__") or die; print $sid "cafe-1234\n"; close($sid);
}
print "\e[?2004h";
my $border = "\xe2\x94\x80" x 400;
my $ornament = "\xe2\x9d\xaf";
my $nbsp = "\xc2\xa0";
print "\e[H\e[2J";
print "fake agent transcript\r\n";
print "\e[1m$ornament\e[0m$nbsp\r\n";
print "$border\r\n";
print "  fake-model  ~/x\r\n";
while (1) { sleep 1; }
"#;

/// A roster whose agent does nothing but stay in its pane.
const IDLE_CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
     [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n";

/// One isolated ae home, one project directory, one tmux server.
struct Rig {
    scratch: PathBuf,
    sock: PathBuf,
    home: PathBuf,
    project: PathBuf,
    config: PathBuf,
    launched: PathBuf,
}

impl Rig {
    /// `tools` names each fake agent to install, and whether it writes a
    /// `codex.<slot>.sid` handshake file into the session directory.
    fn new(tag: &str, tools: &[&str], sid_for: Option<&str>) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let scratch = PathBuf::from(format!("/tmp/aeln.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(std::fs::create_dir_all(&scratch).is_ok(), "a scratch dir");
        let home = scratch.join("aehome");
        let project = scratch.join("project");
        assert!(std::fs::create_dir_all(&project).is_ok(), "a project dir");
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
        Self {
            sock: scratch.join("sock"),
            scratch,
            home,
            project,
            config,
            launched,
        }
    }

    /// The same rig with a bare sleeper as its agent.
    ///
    /// A perl TUI exists for the input sensor, and a test that never reads a
    /// pane pays for it in contention with every other launch arm. Two of them
    /// run here — this session and the companion it starts — so both are cheap.
    fn idle(tag: &str) -> Self {
        let rig = Self::new(tag, &[], None);
        assert!(
            std::fs::write(&rig.config, IDLE_CONFIG).is_ok(),
            "an idle config"
        );
        rig
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.sock.clone()),
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
    ///
    /// `TMUX_TMPDIR` points at this rig's scratch so that a launch which falls
    /// back to the ambient server falls back into the rig rather than onto the
    /// developer's own tmux. Proving that a bad pair does NOT build a session
    /// means letting it try.
    fn launch_with_server(
        &self,
        kind: &str,
        value: &str,
        tail: &[&str],
    ) -> (Option<i32>, String, String) {
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
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
                "--",
            ])
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
    ///
    /// The only way to read a pane's argv without a pane, now that the argv
    /// exists nowhere but in the `execve` itself.
    fn plan(&self, session: &str, slot: &str) -> String {
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
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

    /// Plant the orchestrator scaffold the companion autostart opts in on.
    ///
    /// Its agent is a bare sleeper rather than the TUI fake: what this proves is
    /// that the companion is LAUNCHED, under its own config, and a second perl
    /// TUI would only add contention to a suite that already runs several.
    fn scaffold_orchestrator(&self) -> PathBuf {
        let dir = self.home.join("orchestrator");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a scaffold dir");
        let config = dir.join("orchestrator.config");
        assert!(
            std::fs::write(&config, IDLE_CONFIG).is_ok(),
            "a scaffold config"
        );
        config
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
    ///
    /// The unit a LEAD layout is about: which window a seat lands in, and what
    /// that window is called. The `ae-monitor` window is dropped — it holds the
    /// watchdog and the event tail, is parked at a high index precisely so it
    /// never mixes with the agent windows, and a layout has no opinion about it.
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
        // 20s, not 5. The wait is over as soon as the file appears, so a
        // generous bound costs nothing when the machine is idle — and a bound
        // that a loaded machine can miss is a flaky test, not a product fact.
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

impl Drop for Rig {
    /// Kill the server, then keep removing the scratch until it STAYS removed.
    ///
    /// One removal is not enough: a launch leaves DETACHED children behind it —
    /// the id capture, and the orchestrator companion, which is a whole second
    /// launch — and a child still writing recreates the tree a moment after the
    /// first `remove_dir_all` succeeds. That left a `/tmp/aeln.<pid>.<tag>`
    /// skeleton behind every run of the companion arm.
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        for _ in 0..40 {
            let _ = std::fs::remove_dir_all(&self.scratch);
            if !self.scratch.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&self.scratch);
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
    // The hint names a command that EXISTS. It used to say `ae orchestrator
    // --attach`, which is a retired word that now refuses — and which named the
    // companion rather than the session just started.
    assert!(
        stdout.contains(&format!(
            "Attach with: tmux -S {} attach -t lnlocal",
            rig.sock.display()
        )),
        "{stdout}"
    );
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

    // NO LAUNCH SCRIPT: the pane's command is the core, and there is no bash
    // left anywhere in the session directory.
    assert!(
        !dir.join("launch.main.sh").exists(),
        "slice Z2 writes no bash into a session directory"
    );
    // The ARGV the agent actually got, and the same argv reported without a
    // pane. A fresh claude bakes its own id. The wait is the argv's: the tool
    // reports itself only after `_run` has become it, so a recorded argv is
    // proof the marker below was already written.
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
    // not when the launch returns. Peek until it has, within a bound.
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
    let sid = fresh
        .lines()
        .find_map(|line| line.strip_prefix("harness_session.main="))
        .unwrap_or_default()
        .to_owned();
    assert!(!sid.is_empty(), "claude's id is known upfront:\n{fresh}");

    // The session stops; its state stays.
    assert!(
        rig.tmux(&["kill-session", "-t", "lnres"]).0,
        "the kill lands"
    );

    let (code, stdout, stderr) = rig.launch(&["--local", "lnres"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("Resuming session lnres"),
        "the resume announces itself: {stdout}"
    );
    // THE RESUME DECISION IS THE CORE'S NOW, and it is the start marker plus a
    // probe rather than a shell `if` in a generated script. With no transcript
    // on disk for this id, the probe says no and the pane runs the CWD-heuristic
    // fallback — which is exactly what the script's `else` branch did.
    let plan = rig.plan("lnres", "main");
    assert!(plan.contains(r#""mode":"resume""#), "{plan}");
    assert!(
        plan.contains(r#""--continue""#) && !plan.contains(&format!(r#""--resume","{sid}""#)),
        "no transcript for this id, so the fallback: {plan}"
    );

    // Plant the transcript claude would have written, and the same seat resumes
    // the SAME conversation. The probe is a file test; this is the file.
    let home = std::env::var("HOME").unwrap_or_default();
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
        !rig.dir("lnres").join("launch.main.sh").exists(),
        "and no bash was written to decide any of it"
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

/// The codex handshake: the tool writes `codex.<slot>.sid`, and the capture
/// pass turns it into the roster's `harness_session.main`.
#[test]
fn a_codex_launch_captures_the_session_id_it_registers() {
    if skip() {
        return;
    }
    let rig = Rig::new("cap", &["codex"], Some("main"));
    let (code, stdout, stderr) = rig.launch(&["--local", "cap"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        rig.meta("cap").contains("harness_session.main=pending"),
        "codex has no launch-time id:\n{}",
        rig.meta("cap")
    );
    // The capture polls on its own thread; the first look is one interval in.
    for _ in 0..80 {
        if rig.meta("cap").contains("harness_session.main=cafe-1234") {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!("the capture never registered the id:\n{}", rig.meta("cap"));
}

/// The orchestrator companion, started BY THE CORE.
///
/// The bug this pins: the autostart ran `env AE_NO_AUTOSTART=1 <glue>
/// orchestrator`, and the glue's `orchestrator` arm is a RETIRED word that
/// refuses with exit 2 — on a stderr the detached child sends to `/dev/null`.
/// So the companion had not started since the glue cut, and nothing said so.
///
/// The rig passes no glue path at all (the flag is gone), so a companion that
/// appears here can only have been launched by the core itself.
#[test]
fn a_scaffold_starts_the_orchestrator_companion_from_the_core() {
    if skip() {
        return;
    }
    let rig = Rig::idle("orch");
    let config = rig.scaffold_orchestrator();
    let (code, stdout, stderr) = rig.launch(&["--local", "lnorch"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("Starting orchestrator companion session"),
        "the launch says it started one: {stdout}"
    );

    // The child is DETACHED, and its two observable facts do not land together:
    // tmux lists a session the moment it is created, while the meta is
    // published further down the launch. Waiting on the RECORD waits for both.
    let recorded = format!("config={}", config.display());
    let mut meta = String::new();
    for _ in 0..160 {
        meta = rig.meta("orchestrator");
        if meta.contains(&recorded) {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let seen = rig.sessions();
    assert!(
        seen.iter().any(|name| name == "orchestrator"),
        "the companion must be up on the rig's own server: {seen:?}"
    );

    // ISOLATION is the whole point of the retired trampoline: the companion
    // must run under the SCAFFOLD's config and directory, never the caller's.
    assert!(
        meta.contains(&recorded),
        "the companion read the scaffold's config:\n{meta}"
    );
    assert!(
        meta.contains(&format!(
            "origin={}",
            rig.home.join("orchestrator").display()
        )),
        "the companion ran in the scaffold's directory:\n{meta}"
    );

    // And it starts NO companion of its own: the structural guard (a session
    // named for a scaffold) and `--no-autostart` both hold, so there is exactly
    // one orchestrator however many times the recursion could have gone round.
    assert_eq!(
        rig.sessions()
            .iter()
            .filter(|name| *name == "orchestrator")
            .count(),
        1,
        "the recursion guard holds"
    );
}

/// `--glue` is GONE, and an unknown flag is refused exactly as before.
///
/// The flag existed to record `ae_path` in meta — the `ae` COMMAND the watchdog
/// re-exec'd for its Telegram revive. That revive is in-process now, so the row
/// had no reader and the flag had no subject. No compat arm: a caller still
/// passing it is refused before any side effect, which is the whole point of
/// reading the preamble first.
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
///
/// The bug: the restore read `cfg.profile()` and handed the string to the pane
/// shell. `config::launch_plan` lexes the `[roster]` seats and `_spawn` lexes
/// the profile it is given, but nothing lexed THIS one — so a profile holding
/// `touch m ; tail -f /dev/null` ran both halves on resume, which is the defect
/// the spawn gate closed, reached through the restore.
///
/// `marker` is the proof: it exists only if a second command ran.
fn resumable_rig(tag: &str, session: &str, profile: &str) -> (Rig, PathBuf) {
    let rig = Rig::idle(tag);
    let marker = rig.home.join("MARKER");
    let mut config = std::fs::read_to_string(&rig.config).unwrap_or_default();
    // A SECOND `[profiles]` header: the rig's config ends inside `[workspace]`,
    // and a bare `key = value` appended there is a workspace key, not a profile.
    config.push_str("\n[profiles]\n");
    config.push_str(&profile.replace("__MARKER__", &marker.display().to_string()));
    assert!(std::fs::write(&rig.config, config).is_ok(), "a bad profile");
    // A STOPPED session: meta on disk, nothing running. That is what a resume
    // reads its roster out of.
    let dir = rig.dir(session);
    assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
    assert!(
        std::fs::write(
            dir.join("meta"),
            format!(
                "session={session}\nmode=local\nlayout=vertical\nwork_dir={home}\n\
                 origin={home}\nschema=2\nseat.main=lead\nprofile.main=idle\n\
                 agent_bin.main=sleep\nseat.spawned.0=helper\nprofile.spawned.0=bad\n\
                 agent_bin.spawned.0=sleep\n",
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
}

/// An unusable `--server-kind` is refused before anything is built.
///
/// The bug: an unknown kind fell through to the ambient server, so the launch
/// exited 0 having created the session on the DEFAULT tmux while recording the
/// unusable pair in meta. Nothing could act on it afterwards — every lifecycle
/// operation refuses the record it is handed — so the session was a phantom:
/// running, ae's by its marker, and unmanageable.
///
/// `ambiguous` is the case that matters: it is what the glue emits for a socket
/// path it could not canonicalise, which is a statement that the caller's own
/// server could not be identified. There is nothing to substitute for it.
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

    // socket: the rig's own, which every other arm here already depends on.
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
            "Attach with: tmux -S {} attach -t lnsock",
            rig.sock.display()
        )),
        "{stdout}"
    );

    // name: a `-L` server, which nothing else here exercises.
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
        stdout.contains(&format!("Attach with: tmux -L {named} attach -t lnnamed")),
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

/// The two LEAD layouts seat each agent in the window their layout names.
///
/// `lead-solo` keeps window 0 for the lead alone and puts every worker in the
/// second window. `lead-pair` seats the FIRST worker beside the lead in window
/// 0 — the leadership pair is two EQUAL seats, which is the whole point of the
/// layout — and the rest go to the second window.
///
/// The window names are fixed role LITERALS, `leads` and `workers`. An agent
/// name would reach `rename-window`'s format sink, where `#` runs a shell.
///
/// The layout is pinned into meta at launch because META WINS on resume: a
/// session keeps the shape it was started with even after the config moves on.
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
            ("0".to_owned(), "lsolo".to_owned(), 1),
            ("1".to_owned(), "workers".to_owned(), 2),
        ],
        "the lead is alone, and the workers share the role-named second window"
    );
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
            ("0".to_owned(), "leads".to_owned(), 2),
            ("1".to_owned(), "workers".to_owned(), 2),
        ],
        "both leadership seats are in window 0, and both windows carry a ROLE name"
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
    assert!(
        pair.meta("lpair").contains("layout=lead-pair"),
        "the layout is pinned:\n{}",
        pair.meta("lpair")
    );
}

/// The status bar is AE-OWNED, and its first line still RENDERS.
///
/// ae bakes both lines once at launch; the live segments arrive as USER OPTIONS
/// the watchdog feeds, so nothing ever rewrites `status-right` itself. The
/// regression this pins is tmux's array-option rule: setting `status-format[1]`
/// at session scope SHADOWS THE WHOLE ARRAY and leaves index 0 — the standard
/// bar — empty, which shipped once as two blank green lines. The launch copies
/// the global [0] in alongside its own [1] for exactly that reason, so the
/// assertion has to be that line 0 renders, not merely that it is set.
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
    assert_eq!(option("status-left"), "[ae lnbar] ");
    // The watch segment is a user option at the END, referenced exactly once:
    // the retired save/prepend/restore doubled it on every watchdog restart.
    let right = option("status-right");
    assert!(
        right.starts_with('[') && right.ends_with("] #{@ae_watchdog_status}"),
        "the ae-baked shape: {right}"
    );
    assert_eq!(
        right.matches("#{@ae_watchdog_status}").count(),
        1,
        "exactly one watch reference: {right}"
    );
    assert!(
        option("status-format[1]").contains("#{@ae_agent}"),
        "the second line is the focused agent's identity"
    );
    assert!(
        !option("status-format[0]").is_empty(),
        "the standard bar is present at session scope"
    );
    // PRESENT is not RENDERED. `#{T:…}` expands the option the way tmux draws
    // it, which is the only reading that can tell a copied bar from a blank one.
    let (_, drawn) = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        "lnbar:0",
        "#{T:status-format[0]}",
    ]);
    assert!(
        drawn.contains("lnbar"),
        "line 0 draws the session rather than a blank line: {drawn:?}"
    );
    // The per-window glyph renders only through these, at SESSION scope — and
    // the GLOBAL table stays the operator's.
    assert!(
        option("window-status-format").contains("#{@ae_window_status}"),
        "the window glyph has somewhere to render"
    );
    let (_, global) = rig.tmux(&["show-options", "-gv", "window-status-format"]);
    assert!(
        !global.contains("@ae_window_status"),
        "ae does not theme the global window table: {global}"
    );
}
