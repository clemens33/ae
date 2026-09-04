//! `_launch` against a REAL tmux server: the whole session, built or resumed.
//!
//! The operation runs end to end — the working copy, the tmux session, its
//! panes and their stamps, the meta, the helper shims, the launch scripts and
//! the paste that starts each agent. The agents are the same perl fake the
//! spawn suite uses, named for the tool whose classification the test needs.

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
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
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
                &self.sock.display().to_string(),
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

    fn dir(&self, session: &str) -> PathBuf {
        self.home.join("sessions").join(session)
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
        for _ in 0..200 {
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
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
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

    // The HELPERS, every one a shim that execs the core.
    let dir = rig.dir("lnlocal");
    for helper in [
        "send", "state", "peek", "agents", "focus", "spawn", "watchdog",
    ] {
        let body = std::fs::read_to_string(dir.join(helper))
            .unwrap_or_else(|why| panic!("the {helper} helper should exist: {why}"));
        assert!(
            body.contains("exec ") && body.contains("\"$@\""),
            "{helper} is not a shim:\n{body}"
        );
    }
    assert!(
        std::fs::read_to_string(dir.join("workspace.md"))
            .unwrap_or_default()
            .contains("lead"),
        "the manifest names the roster"
    );

    // The LAUNCH SCRIPT, and the agent it started.
    let script = std::fs::read_to_string(dir.join("launch.main.sh"))
        .unwrap_or_else(|why| panic!("a launch script: {why}"));
    assert!(
        script.contains("--session-id"),
        "a fresh claude bakes its id:\n{script}"
    );
    assert!(
        !rig.launch_argv().is_empty(),
        "the pasted script started the agent"
    );
}

/// A helper shim really does exec the core: `state` writes the caller's
/// declaration, and `peek` reads a pane back.
#[test]
fn a_helper_shim_execs_the_core() {
    if skip() {
        return;
    }
    let rig = Rig::new("shim", &["claude"], None);
    let (code, stdout, stderr) = rig.launch(&["--local", "lnshim"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let dir = rig.dir("lnshim");
    let lead = rig
        .panes("lnshim")
        .into_iter()
        .find(|(_, slot, _)| slot == "main")
        .map(|(pane, _, _)| pane)
        .unwrap_or_default();

    let out = helper(&dir.join("state"))
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &lead)
        .args(["working", "proving the shim"])
        .output()
        .unwrap_or_else(|why| panic!("the state shim should run: {why}"));
    assert!(
        out.status.success(),
        "state shim: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        std::fs::read_to_string(dir.join("events.jsonl"))
            .unwrap_or_default()
            .contains("proving the shim"),
        "the core wrote the declaration through the shim"
    );

    let out = helper(&dir.join("peek"))
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &lead)
        .args(["lead", "20"])
        .output()
        .unwrap_or_else(|why| panic!("the peek shim should run: {why}"));
    assert!(
        out.status.success(),
        "peek shim: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("fake agent transcript"),
        "peek read the pane back"
    );
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
    let script = std::fs::read_to_string(rig.dir("lnres").join("launch.main.sh"))
        .unwrap_or_else(|why| panic!("a launch script: {why}"));
    assert!(
        script.contains(&format!("--resume {sid}")),
        "the resume asks for the SAME conversation:\n{script}"
    );
    assert!(
        script.contains("--continue"),
        "and keeps the CWD-heuristic fallback:\n{script}"
    );
    assert!(
        rig.meta("lnres")
            .contains(&format!("harness_session.main={sid}")),
        "the id survives the rewrite"
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
