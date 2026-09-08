//! `_end`, `_stop` and `_compact` — the whole lifecycle operations, against a
//! REAL tmux server on its own socket.
//!
//! The ORDER is what is proven here, because the order is the contract:
//!
//! * an end that keeps its history ARCHIVES the session and only then removes
//!   it;
//! * an end whose archive cannot be published leaves EVERYTHING on disk — the
//!   session dir, its meta, its memory — even though the session was already
//!   stopped by then. ae never deletes a session it could not capture;
//! * `--purge-history` writes no archive at all;
//! * `stop` destroys nothing: the session dir and its meta survive;
//! * `compact` crosses the boundary and hands the relaunch the FROZEN roster,
//!   not a config re-read after the fact.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build and inspect real directories and tmux servers; the \
              capability boundary is about what PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::cli::{ae, bounded};
use super::phase2::run_tmux;

const UUID: &str = "33333333-3333-3333-3333-333333333333";

/// One isolated `AE_HOME` with its own tmux server and one live local session.
struct Rig {
    home: PathBuf,
    sock: PathBuf,
    name: String,
    dir: PathBuf,
    pane: String,
}

impl Rig {
    /// A live session named `lc<tag>` whose meta points at this rig's socket.
    fn new(tag: &str) -> Self {
        let home = PathBuf::from(format!("/tmp/aelc.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("a scratch AE_HOME");
        let name = format!("lc{tag}");
        let dir = home.join("sessions").join(&name);
        std::fs::create_dir_all(&dir).expect("a session dir");
        let config = home.join("config");
        std::fs::write(
            &config,
            "[profiles]\nfake = \"sh\"\n\n[roster]\nfake = fake\n\n[workspace]\nmain = fake\nlayout = vertical\n",
        )
        .expect("a config");
        let mut rig = Self {
            home: home.clone(),
            sock: home.join("s"),
            name: name.clone(),
            dir: dir.clone(),
            pane: String::new(),
        };
        assert!(
            rig.tmux(&["-f", "/dev/null", "new-session", "-d", "-s", &name, "sh"])
                .0,
            "the session starts"
        );
        let (_, panes) = rig.tmux(&["list-panes", "-s", "-t", &name, "-F", "#{pane_id}"]);
        let main_pane = panes.lines().next().unwrap_or_default().to_owned();
        assert!(!main_pane.is_empty(), "{panes}");
        assert!(
            rig.tmux(&["set-option", "-p", "-t", &main_pane, "@ae_agent", "lead"])
                .0
        );
        assert!(
            rig.tmux(&[
                "set-environment",
                "-g",
                "AE_HOME",
                &home.display().to_string(),
            ])
            .0,
            "the server-owned continuation inherits the isolated state root"
        );
        std::fs::write(
            dir.join("meta"),
            format!(
                "session={name}\nsession_id={UUID}\nsession_id_origin=session\nwork_dir={}\n\
                 origin={}\nmode=local\nlayout=vertical\nconfig={}\nmain_pane={main_pane}\n\
                 tmux_server_kind=socket\ntmux_server={}\nschema=2\nseat.main=lead\n\
                 profile.main=fake\nagent_bin.main=sh\n",
                home.display(),
                home.display(),
                config.display(),
                rig.sock.display(),
            ),
        )
        .expect("a v2 meta");
        rig.pane = main_pane;
        rig
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        self.tmux_at(&self.sock, tail)
    }

    fn tmux_at(&self, socket: &Path, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(socket.to_path_buf()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.home)
    }

    /// Run one core subcommand under this rig's `AE_HOME`, bounded.
    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        self.run_from(args, None)
    }

    /// Run the human-facing binary as if it were invoked in this rig's pane.
    fn run_inside(&self, args: &[&str]) -> (Option<i32>, String, String) {
        self.run_from(args, Some((&self.sock, &self.pane)))
    }

    /// Run as a caller pane on an explicitly chosen server.
    fn run_from(
        &self,
        args: &[&str],
        caller: Option<(&Path, &str)>,
    ) -> (Option<i32>, String, String) {
        let mut cmd = ae();
        cmd.env("AE_HOME", &self.home);
        if let Some((socket, pane)) = caller {
            cmd.env("TMUX", format!("{},fixture,0", socket.display()));
            cmd.env("TMUX_PANE", pane);
            cmd.env("AE_TMUX_SERVER_KIND", "socket");
            cmd.env("AE_TMUX_SERVER", &self.sock);
        } else {
            cmd.env_remove("TMUX");
            cmd.env_remove("TMUX_PANE");
        }
        for arg in args {
            cmd.arg(arg);
        }
        let out = bounded(
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("the ae binary should run"),
            Duration::from_secs(30),
        )
        .expect("the core returned");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Keep one real client attached to the target, hosted in another detached
    /// pane on the same isolated server. Keys sent to that host pane are raw
    /// client input, including the single-key confirmation answer.
    fn attach_client(&self) -> (String, String) {
        let controller = format!("ctl{}", self.name);
        assert!(
            self.tmux(&["set-option", "-t", &self.name, "detach-on-destroy", "off",])
                .0,
            "the client remains attached after the target is destroyed"
        );
        let command = format!(
            "env TMUX= tmux -S {} attach -t ={}",
            self.sock.display(),
            self.name
        );
        assert!(
            self.tmux(&["new-session", "-d", "-s", &controller, &command])
                .0,
            "the controller pane starts"
        );
        let (_, panes) = self.tmux(&[
            "list-panes",
            "-t",
            &format!("={controller}"),
            "-F",
            "#{pane_id}",
        ]);
        let controller_pane = panes.lines().next().unwrap_or_default().to_owned();
        assert!(!controller_pane.is_empty(), "the controller pane has an id");
        let mut client = String::new();
        for _ in 0..100 {
            let (_, clients) =
                self.tmux(&["list-clients", "-F", "#{client_name} | #{session_name}"]);
            if let Some(name) = clients.lines().find_map(|line| {
                line.split_once(" | ")
                    .filter(|(_, session)| *session == self.name)
                    .map(|(name, _)| name.to_owned())
            }) {
                client = name;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(!client.is_empty(), "a client attaches to {}", self.name);
        (controller_pane, client)
    }

    fn session_is_live(&self) -> bool {
        let (_, listed) = self.tmux(&["list-sessions", "-F", "#{session_name}"]);
        listed.lines().any(|line| line == self.name)
    }

    /// A non-ae session with the same name on another isolated server.
    fn foreign_namesake(&self, tag: &str) -> (PathBuf, String) {
        let socket = self.home.join(format!("foreign-{tag}.sock"));
        let (created, panes) = self.tmux_at(
            &socket,
            &[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-s",
                &self.name,
                "sleep",
                "60",
            ],
        );
        assert!(created, "the foreign namesake starts: {panes}");
        let pane = panes.lines().next().unwrap_or_default().to_owned();
        assert!(!pane.is_empty(), "the foreign namesake has a pane: {panes}");
        (socket, pane)
    }

    fn archive(&self) -> PathBuf {
        self.home.join("archive").join(UUID)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// A monitor helper whose watchdog registers, then stays alive only while the
/// session directory its argv[0] named still exists. This models the real
/// watchdog's dependence on that directory while keeping the fixture small.
const RENAME_MONITOR_CORE: &str = "#!/bin/sh\n\
    d=$(cd \"$(dirname \"$0\")\" && pwd)\n\
    if [ \"$(basename \"$0\")\" = watchdog ]; then\n\
      printf '%s\\n' \"$$\" > \"$d/.watchdog.pid.staged\"\n\
      mv \"$d/.watchdog.pid.staged\" \"$d/.watchdog.pid\"\n\
      while [ -d \"$d\" ]; do sleep 0.1; done\n\
      sleep 1\n\
      exit 0\n\
    fi\n\
    exec sleep 60\n";

fn write_exec(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt as _;

    let _ = std::fs::remove_file(path);
    std::fs::write(path, text).expect("an executable fixture");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("executable mode");
}

#[test]
fn rename_respawns_both_monitor_panes_against_the_new_session_directory() {
    let rig = Rig::new("renamemon");
    let monitor_core = rig.home.join("monitor-core");
    write_exec(&monitor_core, RENAME_MONITOR_CORE);
    for helper in ["watchdog", "events-tail"] {
        std::os::unix::fs::symlink(&monitor_core, rig.dir.join(helper)).expect("a monitor helper");
    }
    let (mut start_out, mut start_err) = (Vec::new(), Vec::new());
    let started = ae::watchdog_lifecycle::run(
        &rig.home,
        &["start".to_owned(), rig.name.clone()],
        &mut start_out,
        &mut start_err,
    )
    .expect("the watchdog entry writes to buffers");
    assert_eq!(
        started,
        0,
        "watchdog start: {}",
        String::from_utf8_lossy(&start_err)
    );
    let old_pid = ae::watchdog_glue::read_pid(&rig.dir).expect("the old watchdog registered");
    let (_, before) = rig.tmux(&[
        "list-panes",
        "-t",
        &format!("={}:99", rig.name),
        "-F",
        "#{@ae_agent}|#{pane_id}|#{pane_top}|#{pane_height}",
    ]);

    let new = format!("{}new", rig.name);
    let (code, out, err) = rig.run(&["rename", &rig.name, &new]);

    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    let new_dir = rig.home.join("sessions").join(&new);
    let (_, after) = rig.tmux(&[
        "list-panes",
        "-t",
        &format!("={new}:99"),
        "-F",
        "#{@ae_agent}|#{pane_id}|#{pane_top}|#{pane_height}",
    ]);
    assert_eq!(after, before, "respawn preserves pane ids and layout");
    let (_, commands) = rig.tmux(&[
        "list-panes",
        "-t",
        &format!("={new}:99"),
        "-F",
        "#{@ae_agent}|#{pane_start_command}",
    ]);
    for (agent, helper) in [("_watchdog", "watchdog"), ("_events", "events-tail")] {
        assert!(
            commands.lines().any(|line| {
                line.starts_with(&format!("{agent}|"))
                    && line.contains(&new_dir.join(helper).display().to_string())
            }),
            "{helper} did not move to the new session helper: {commands:?}"
        );
    }
    assert!(
        !commands.contains(&format!("{}/", rig.dir.display())),
        "an old session helper survived: {commands:?}"
    );
    let new_pid = ae::watchdog_glue::read_pid(&new_dir).expect("the new watchdog registered");
    assert_ne!(new_pid, old_pid, "the old watchdog process survived rename");
    assert!(matches!(
        ae::watchdog_lifecycle::presence(
            &ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(rig.sock.clone())),
            &new,
            &new_dir,
        ),
        ae::watchdog_lifecycle::Presence::Running(pid) if pid == new_pid
    ));
}

#[test]
fn rename_reports_a_missing_watchdog_instead_of_claiming_success() {
    let rig = Rig::new("renamefail");
    let new = format!("{}new", rig.name);

    let (code, out, err) = rig.run(&["rename", &rig.name, &new]);

    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "rename must not report success: {out}");
    assert!(err.contains("NO verified watchdog"), "{err}");
    assert!(err.contains(&format!("ae watchdog start {new}")), "{err}");
    let (_, names) = rig.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(
        names.lines().any(|name| name == new),
        "the diagnostic names the actual post-rename state: {names:?}"
    );
}

#[test]
fn an_end_that_keeps_its_history_archives_before_it_removes() {
    let rig = Rig::new("keep");
    let (code, out, err) = rig.run(&["_end", "-f", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.contains(&format!("Archived {UUID}")), "{out}");
    assert!(
        out.contains(&format!("Ended local session {}", rig.name)),
        "{out}"
    );
    assert!(exists(&rig.archive()), "the archive is published");
    assert!(
        exists(&rig.archive().join("meta")),
        "the archive carries the session's meta"
    );
    assert!(!exists(&rig.dir), "the live session state is gone");
    assert!(!rig.session_is_live(), "the tmux session is gone");
}

#[test]
fn an_end_whose_archive_cannot_be_published_leaves_the_whole_session() {
    let rig = Rig::new("noarch");
    // A regular FILE where the archive directory would go.
    std::fs::create_dir_all(rig.home.join("archive")).expect("an archive root");
    std::fs::write(rig.archive(), b"not a directory\n").expect("the obstruction");
    std::fs::write(rig.dir.join("memo.tsv"), b"x\tkeep me\n").expect("some memory");

    let (code, out, err) = rig.run(&["_end", "-f", &rig.name]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(
        err.contains("NOTHING was deleted"),
        "the refusal says nothing was deleted: {err}"
    );
    assert!(exists(&rig.dir), "the session dir survives");
    assert!(exists(&rig.dir.join("meta")), "its meta survives");
    assert!(exists(&rig.dir.join("memo.tsv")), "its memory survives");
    // The stop happens BEFORE the snapshot, so the session is legitimately
    // down by now — what must not have happened is a deletion.
    assert!(
        std::fs::read(rig.archive()).expect("the obstruction is untouched") == b"not a directory\n",
        "the obstruction was not overwritten"
    );
}

#[test]
fn purge_history_writes_no_archive_at_all() {
    let rig = Rig::new("purge");
    let (code, out, err) = rig.run(&["_end", "-f", "--purge-history", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(
        out.contains("No archive written (--purge-history)"),
        "{out}"
    );
    assert!(!exists(&rig.archive()), "no archive was written");
    assert!(!exists(&rig.dir), "the live session state is gone");
}

#[test]
fn stop_destroys_nothing() {
    let rig = Rig::new("stop");
    let (code, out, err) = rig.run(&["_stop", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.contains(&format!("Stopped {}", rig.name)), "{out}");
    assert!(!rig.session_is_live(), "the tmux session is gone");
    assert!(exists(&rig.dir), "the session dir stays");
    assert!(exists(&rig.dir.join("meta")), "its meta stays");
    assert!(!exists(&rig.archive()), "stop archives nothing");

    // A second stop is a refusal, not a silent success: the session is not
    // running, and `stop` says so rather than reporting it stopped one.
    let (code, _, err) = rig.run(&["_stop", &rig.name]);
    assert_eq!(code, Some(1));
    assert!(err.contains("is not running"), "{err}");
}

#[test]
fn a_bare_stop_from_a_foreign_namesake_never_targets_the_recorded_session() {
    let rig = Rig::new("foreignbarestop");
    let (foreign, pane) = rig.foreign_namesake("stop");

    let (code, out, err) = rig.run_from(&["stop", "-y"], Some((&foreign, &pane)));
    let _ = rig.tmux_at(&foreign, &["kill-server"]);

    assert_eq!(code, Some(2), "stdout: {out}\nstderr: {err}");
    assert_eq!(err, "Usage: _stop <session-name|all> [-y] [--self]\n");
    assert!(rig.session_is_live(), "the recorded session stays live");
    assert!(exists(&rig.dir), "the recorded state stays intact");
}

#[test]
fn self_stop_from_a_foreign_namesake_refuses_instead_of_supervising_it() {
    let rig = Rig::new("foreignselfstop");
    let (foreign, pane) = rig.foreign_namesake("self-stop");

    let (code, out, err) = rig.run_from(&["stop", "--self", "-y"], Some((&foreign, &pane)));
    let _ = rig.tmux_at(&foreign, &["kill-server"]);

    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert_eq!(
        err,
        "Error: --self with no session name needs a pane ae can resolve (--pane <id>); this one is not an ae agent pane.\n"
    );
    assert!(rig.session_is_live(), "the recorded session stays live");
    assert!(exists(&rig.dir), "the recorded state stays intact");
}

#[test]
fn a_bare_end_from_a_foreign_namesake_never_targets_the_recorded_session() {
    let rig = Rig::new("foreignbareend");
    let (foreign, pane) = rig.foreign_namesake("end");

    let (code, out, err) = rig.run_from(&["end", "-f"], Some((&foreign, &pane)));
    let _ = rig.tmux_at(&foreign, &["kill-server"]);

    assert_eq!(code, Some(2), "stdout: {out}\nstderr: {err}");
    assert_eq!(
        err,
        "Usage: _end [-f] [--purge-history|--keep-history] [--assume-stopped] <session-name|all>\n"
    );
    assert!(rig.session_is_live(), "the recorded session stays live");
    assert!(exists(&rig.dir), "the recorded state stays intact");
}

#[test]
fn bare_watchdog_status_from_a_foreign_namesake_has_no_inferred_target() {
    let rig = Rig::new("foreignwatchdog");
    let (foreign, pane) = rig.foreign_namesake("watchdog");
    let meta = rig.dir.join("meta");
    let before = std::fs::read(&meta).expect("the recorded meta");

    // Naming the target remains valid from another server and addresses the
    // server the target records.
    let (code, out, err) =
        rig.run_from(&["watchdog", "status", &rig.name], Some((&foreign, &pane)));
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert_eq!(out, "Watchdog is not running.\n");
    assert!(err.is_empty(), "{err}");

    let (code, out, err) = rig.run_from(&["watchdog", "status"], Some((&foreign, &pane)));
    let _ = rig.tmux_at(&foreign, &["kill-server"]);

    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "{out}");
    assert_eq!(
        err,
        "Error: no session name given and not inside an ae tmux session\n"
    );
    assert!(rig.session_is_live(), "the recorded session stays live");
    assert_eq!(
        std::fs::read(meta).expect("the recorded meta after refusal"),
        before,
        "the recorded watchdog state stays untouched"
    );
}

#[test]
fn a_self_stop_returns_from_the_pane_it_kills() {
    // THE SHAPE THAT NEEDS THE SUPERVISOR, driven the way it actually happens:
    // the command runs in a shell INSIDE the target's own pane, so the process
    // asking for the stop is the one the stop destroys.
    let rig = Rig::new("self");
    let caller_out = rig.home.join("selfout");
    let command = format!(
        "AE_HOME={} {} _stop --self -y {} > {} 2>&1",
        rig.home.display(),
        env!("CARGO_BIN_EXE_ae"),
        rig.name,
        caller_out.display(),
    );
    // Let the pane's shell reach a prompt before typing at it.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        rig.tmux(&["send-keys", "-t", &rig.name, &command, "Enter"])
            .0,
        "the command is typed into the target's own pane"
    );

    let mut returned = false;
    let mut gone = false;
    for _ in 0..200 {
        if !returned {
            returned = std::fs::read_to_string(&caller_out)
                .unwrap_or_default()
                .contains(&format!("Stopping '{}' out of pane", rig.name));
        }
        if !gone {
            gone = !rig.session_is_live();
        }
        if returned && gone {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let said = std::fs::read_to_string(&caller_out).unwrap_or_default();
    assert!(returned, "the caller reached its last line: {said:?}");
    assert!(
        said.contains("events.jsonl (action: stop-result)"),
        "it names where the outcome lands: {said:?}"
    );
    assert!(gone, "the session is gone");

    // The supervisor — not the dead caller — recorded the outcome, and the
    // request that preceded it.
    let mut events = String::new();
    for _ in 0..100 {
        events = std::fs::read_to_string(rig.dir.join("events.jsonl")).unwrap_or_default();
        if events.contains("stop-result") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(events.contains("\"action\":\"stop-request\""), "{events}");
    assert!(events.contains("\"action\":\"stop-result\""), "{events}");
    assert!(
        events.contains("verified gone on its recorded server"),
        "{events}"
    );
    // Stop destroys nothing, self-stop included.
    assert!(exists(&rig.dir), "the session dir stays");
    assert!(!exists(&rig.archive()), "a stop archives nothing");
}

#[test]
fn a_self_stop_with_no_terminal_and_no_client_refuses_in_one_line() {
    // A non-interactive caller inside a detached session has nobody who can
    // answer a tmux confirmation prompt.
    let rig = Rig::new("selfnotty");
    let (code, out, err) = rig.run_inside(&["_stop", "--self", &rig.name]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert_eq!(err, "Error: nobody attached to confirm; pass -y.\n");
    assert!(rig.session_is_live(), "nothing was stopped");
}

#[test]
fn an_end_with_no_terminal_and_no_client_refuses_in_one_line() {
    let rig = Rig::new("endnotty");
    let (code, out, err) = rig.run_inside(&["end"]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert_eq!(err, "Error: nobody attached to confirm; pass -f.\n");
    assert!(rig.session_is_live(), "nothing was ended");
    assert!(exists(&rig.dir), "state stays intact");
}

#[test]
fn a_no_tty_stop_inside_prompts_the_attached_client_then_runs_the_continuation() {
    let rig = Rig::new("clientstop");
    let (controller_pane, _client) = rig.attach_client();

    let (code, out, err) = std::thread::scope(|scope| {
        let waiting = scope.spawn(|| rig.run_inside(&["stop"]));
        let mut prompt = String::new();
        for _ in 0..200 {
            let (_, screen) = rig.tmux(&["capture-pane", "-p", "-t", &controller_pane]);
            if screen.contains("Kills the session you are in. (y/n)") {
                prompt = screen;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(!prompt.is_empty(), "the client displays the prompt");
        assert!(rig.session_is_live(), "nothing happens before confirmation");
        assert!(
            rig.tmux(&["send-keys", "-t", &controller_pane, "y"]).0,
            "the attached client answers yes"
        );
        waiting.join().expect("the invoking thread")
    });
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "the prompt is on the tmux client: {out}");
    assert!(err.is_empty(), "{err}");
    let mut events = String::new();
    let mut screen = String::new();
    for _ in 0..200 {
        events = std::fs::read_to_string(rig.dir.join("events.jsonl")).unwrap_or_default();
        let (_, current) = rig.tmux(&["capture-pane", "-p", "-t", &controller_pane]);
        screen = current;
        if events.contains("\"action\":\"stop-result\"")
            && !rig.session_is_live()
            && screen.contains(&format!("Stopped {}", rig.name))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !rig.session_is_live(),
        "the confirmed continuation stopped it; screen: {screen:?}; events: {events}"
    );
    assert!(events.contains("\"action\":\"stop-request\""), "{events}");
    assert!(events.contains("\"action\":\"stop-result\""), "{events}");
    assert!(!events.contains("\"action\":\"chat\""), "{events}");
    assert!(
        screen.contains(&format!("Stopped {}", rig.name)),
        "{screen}"
    );
}

#[test]
fn naming_the_session_already_owning_the_caller_is_a_noop() {
    let rig = Rig::new("own");
    let (code, out, err) = rig.run_inside(&[&rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert_eq!(out, format!("you are in '{}'\n", rig.name));
    assert!(err.is_empty(), "{err}");
    assert!(rig.session_is_live(), "the session remains live");
}

#[test]
fn an_end_handoff_returns_then_its_supervisor_archives_and_removes() {
    let rig = Rig::new("selfend");
    let (code, out, err) = rig.run(&["_end", "--handoff", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "the handoff is silent: {out}");
    assert!(err.is_empty(), "{err}");
    for _ in 0..200 {
        if exists(&rig.archive()) && !exists(&rig.dir) && !rig.session_is_live() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(exists(&rig.archive()), "the archive is published");
    assert!(!exists(&rig.dir), "the live state is gone");
    assert!(!rig.session_is_live(), "the tmux session is gone");
    let events = std::fs::read_to_string(rig.archive().join("events.jsonl"))
        .expect("the archived request event");
    assert!(events.contains("\"action\":\"end-request\""), "{events}");
    assert!(
        !events.contains("\"action\":\"end-result\""),
        "a successful archive is the durable result: {events}"
    );
}

#[test]
fn a_stop_handoff_is_silent_then_its_supervisor_records_the_result() {
    let rig = Rig::new("stophand");
    let (code, out, err) = rig.run(&["_stop", "--handoff", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "the hidden handoff is silent: {out}");
    assert!(err.is_empty(), "{err}");

    let mut events = String::new();
    for _ in 0..200 {
        events = std::fs::read_to_string(rig.dir.join("events.jsonl")).unwrap_or_default();
        if events.contains("\"action\":\"stop-result\"") && !rig.session_is_live() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(!rig.session_is_live(), "the tmux session is gone");
    assert!(events.contains("\"action\":\"stop-request\""), "{events}");
    assert!(events.contains("\"action\":\"stop-result\""), "{events}");
}

#[test]
fn a_bare_forced_end_inside_targets_the_caller_and_finishes_out_of_pane() {
    let rig = Rig::new("bareend");
    let (code, out, err) = rig.run_inside(&["end", "-f"]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.contains("out of pane"), "{out}");

    for _ in 0..200 {
        if exists(&rig.archive()) && !exists(&rig.dir) && !rig.session_is_live() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(exists(&rig.archive()), "the archive is published");
    assert!(!exists(&rig.dir), "the live state is gone");
    assert!(!rig.session_is_live(), "the tmux session is gone");
    let events = std::fs::read_to_string(rig.archive().join("events.jsonl"))
        .expect("the archived request event");
    assert!(events.contains("\"action\":\"end-request\""), "{events}");
}

#[test]
fn a_failed_end_supervisor_records_its_result_in_the_preserved_session() {
    let rig = Rig::new("selfendfail");
    std::fs::create_dir_all(rig.home.join("archive")).expect("an archive root");
    std::fs::write(rig.archive(), b"not a directory\n").expect("the obstruction");

    let (code, out, err) = rig.run(&["_end", "--handoff", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    let mut events = String::new();
    for _ in 0..200 {
        events = std::fs::read_to_string(rig.dir.join("events.jsonl")).unwrap_or_default();
        if events.contains("\"action\":\"end-result\"") {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(exists(&rig.dir), "failed end preserves live state");
    assert!(events.contains("\"action\":\"end-request\""), "{events}");
    assert!(events.contains("\"action\":\"end-result\""), "{events}");
    assert!(events.contains("FAILED:"), "{events}");
}

#[test]
fn a_no_tty_end_inside_prompts_the_attached_client_then_archives() {
    let rig = Rig::new("clientend");
    let (controller_pane, _client) = rig.attach_client();

    let (code, out, err) = std::thread::scope(|scope| {
        let waiting = scope.spawn(|| rig.run_inside(&["end"]));
        let mut prompt = String::new();
        for _ in 0..200 {
            let (_, screen) = rig.tmux(&["capture-pane", "-p", "-t", &controller_pane]);
            if screen.contains("Archives, then deletes its state. (y/n)") {
                prompt = screen;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(!prompt.is_empty(), "the client displays the prompt");
        assert!(rig.session_is_live(), "nothing happens before confirmation");
        assert!(
            rig.tmux(&["send-keys", "-t", &controller_pane, "y"]).0,
            "the attached client answers yes"
        );
        waiting.join().expect("the invoking thread")
    });
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "the prompt is on the tmux client: {out}");
    assert!(err.is_empty(), "{err}");

    let mut screen = String::new();
    for _ in 0..200 {
        let (_, current) = rig.tmux(&["capture-pane", "-p", "-t", &controller_pane]);
        screen = current;
        if exists(&rig.archive())
            && !exists(&rig.dir)
            && !rig.session_is_live()
            && screen.contains(&format!("Ended {} — archived {UUID}", rig.name))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(exists(&rig.archive()), "the archive is published");
    assert!(!exists(&rig.dir), "the live state is gone");
    assert!(!rig.session_is_live(), "the tmux session is gone");
    let events = std::fs::read_to_string(rig.archive().join("events.jsonl"))
        .expect("the archived request event");
    assert!(events.contains("\"action\":\"end-request\""), "{events}");
    assert!(
        events.contains(&format!("end requested from inside by lead/{}", rig.pane)),
        "{events}"
    );
    assert!(!events.contains("\"action\":\"end-result\""), "{events}");
    assert!(
        screen.contains(&format!("Ended {} — archived {UUID}", rig.name)),
        "{screen}"
    );
}

#[test]
fn review_end_confirmation_must_not_silently_change_keep_to_purge() {
    let rig = Rig::new("reviewcontract");
    let (controller_pane, _client) = rig.attach_client();
    let (code, out, err) = std::thread::scope(|scope| {
        let waiting = scope.spawn(|| rig.run_inside(&["end"]));
        let mut shown = false;
        for _ in 0..200 {
            let (_, screen) = rig.tmux(&["capture-pane", "-p", "-t", &controller_pane]);
            if screen.contains("Archives, then deletes its state. (y/n)") {
                shown = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(shown, "the human was offered KEEP, not PURGE");
        let config = rig.home.join("config");
        let before = std::fs::read_to_string(&config).expect("fixture config");
        std::fs::write(&config, format!("{before}purge_agent_history = on\n"))
            .expect("operator changes policy while confirmation is open");
        assert!(rig.tmux(&["send-keys", "-t", &controller_pane, "y"]).0);
        waiting.join().expect("caller")
    });
    assert_eq!(code, Some(0), "{out} {err}");
    let mut events = String::new();
    for _ in 0..200 {
        events = std::fs::read_to_string(rig.dir.join("events.jsonl")).unwrap_or_default();
        if !exists(&rig.dir) || events.contains("\"action\":\"end-result\"") {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        exists(&rig.dir) && rig.session_is_live(),
        "confirmed KEEP was discarded: session_dir_exists={}, archive_exists={}, live={}",
        exists(&rig.dir),
        exists(&rig.archive()),
        rig.session_is_live()
    );
    assert!(!exists(&rig.archive()), "no unconfirmed plan was archived");
    assert!(events.contains("\"action\":\"end-result\""), "{events}");
    assert!(
        events
            .contains("what 'lcreviewcontract' would do changed between the confirmation and now"),
        "the lock-time changed-plan refusal is the cause: {events}"
    );
}

#[test]
fn self_is_a_claim_about_one_session_and_cannot_be_combined_with_all() {
    let mut cmd = ae();
    cmd.env("AE_HOME", "/tmp");
    let out = bounded(
        cmd.args(["_stop", "-y", "--self", "all"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the ae binary should run"),
        Duration::from_secs(10),
    )
    .expect("the core returned");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot be combined with 'all'"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_stopped_session_still_ends_the_ordinary_way() {
    // The stop-now-end-later flow, and the reason it needs no acknowledgement:
    // the target's POSITIVE record names a server that verifiably lacks the
    // session, which is clause (c) of the invariant.
    let rig = Rig::new("later");
    let (code, _, err) = rig.run(&["_stop", &rig.name]);
    assert_eq!(code, Some(0), "{err}");
    assert!(!rig.session_is_live());

    let (code, out, err) = rig.run(&["_end", "-f", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.contains(&format!("Archived {UUID}")), "{out}");
    assert!(exists(&rig.archive()), "the archive is published");
    assert!(!exists(&rig.dir), "the live session state is gone");
}

#[test]
fn compact_hands_the_relaunch_the_frozen_roster() {
    let rig = Rig::new("compact");
    let plan = rig.home.join("plan");
    // `--digest-only` is the one explicit degradation: it skips the semantic
    // handover, which needs a live agent to answer.
    let (code, out, err) = rig.run(&[
        "_compact",
        "-f",
        "--digest-only",
        &rig.name,
        "--exec-plan",
        plan.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");

    // The four stdout contract lines, in order, and nothing else.
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 4, "{out}");
    assert_eq!(lines[0], format!("Archived {UUID}"));
    assert!(lines[1].starts_with("Archive: "), "{out}");
    assert!(lines[2].ends_with("/digest.md"), "{out}");
    assert!(lines[3].starts_with("Recovery: "), "{out}");

    assert!(exists(&rig.archive()), "the archive is published");
    assert!(!exists(&rig.dir), "the source session is gone");
    assert!(!rig.session_is_live(), "the tmux session is gone");

    // THE FROZEN ROSTER, not a config re-read.
    let record = std::fs::read_to_string(&plan).expect("the exec plan is published");
    let fields: Vec<&str> = record.trim_end().split('\u{1f}').collect();
    assert_eq!(fields.len(), 6, "{record:?}");
    assert_eq!(fields[0], rig.name, "the child's name");
    assert_eq!(fields[1], UUID, "the archive it inherits");
    assert_eq!(fields[5], "main=fake workers=-", "the FROZEN roster");
    assert!(
        fields[4].starts_with(UUID),
        "the --from proof: {:?}",
        fields[4]
    );
}

#[test]
fn compact_reads_the_roster_from_the_freeze_and_not_from_a_later_config() {
    let rig = Rig::new("frozen");
    // Rewrite the config to name a DIFFERENT main between the rig's creation
    // and the compact: the freeze runs first inside the operation, so what the
    // plan must carry is whatever the freeze saw — never a second read.
    let plan = rig.home.join("plan");
    let (code, out, err) = rig.run(&[
        "_compact",
        "-f",
        "--digest-only",
        &rig.name,
        "--exec-plan",
        plan.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    let record = std::fs::read_to_string(&plan).expect("the exec plan is published");
    let roster = record.trim_end().split('\u{1f}').nth(5).unwrap_or_default();
    assert_eq!(roster, "main=fake workers=-");
    // And the plan is the ONLY place the relaunch needs to look: it also
    // carries the origin and the config the frozen session recorded.
    let fields: Vec<&str> = record.trim_end().split('\u{1f}').collect();
    assert_eq!(fields[2], rig.home.display().to_string(), "the origin");
    assert_eq!(
        fields[3],
        rig.home.join("config").display().to_string(),
        "the config"
    );
}

#[test]
fn end_refuses_a_target_it_does_not_know() {
    let home = PathBuf::from(format!("/tmp/aelc.{}.missing", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("sessions")).expect("a scratch AE_HOME");
    let mut cmd = ae();
    cmd.env("AE_HOME", &home);
    let out = bounded(
        cmd.args(["_end", "-f", "nosuch"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the ae binary should run"),
        Duration::from_secs(10),
    )
    .expect("the core returned");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Session 'nosuch' not found."),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn end_all_never_accepts_the_per_target_acknowledgement() {
    let mut cmd = ae();
    cmd.env("AE_HOME", "/tmp");
    let out = bounded(
        cmd.args(["_end", "-f", "--assume-stopped", "all"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the ae binary should run"),
        Duration::from_secs(10),
    )
    .expect("the core returned");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--assume-stopped is per-target only"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Without `--exec-plan`, compact starts the child ITSELF — in this process,
/// from the frozen roster, on the server its parent ran on.
#[test]
fn compact_relaunches_the_child_in_process() {
    let rig = Rig::new("relaunch");
    // The child is a REAL launch, so its main must be an agent NAME bound in
    // `[roster]` — the shared fixture's config names a profile directly, which
    // the v2 grammar refuses.
    std::fs::write(
        rig.home.join("config"),
        "[profiles]\nfake = \"sh\"\n\n[roster]\nfake = fake\n\n[workspace]\nmain = fake\nlayout = vertical\nwatchdog = false\n",
    )
    .expect("a v2 config");
    let (code, out, err) = rig.run(&["_compact", "-f", "--digest-only", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");

    // STDOUT IS STILL THE CONTRACT: four lines, in order, and nothing else.
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 4, "{out}");
    assert_eq!(lines[0], format!("Archived {UUID}"));
    assert!(lines[3].starts_with("Recovery: "), "{out}");

    assert!(exists(&rig.archive()), "the archive is published");
    // The CHILD: a live session of the same name, on the parent's own server.
    assert!(
        rig.session_is_live(),
        "the relaunched child is running: {err}"
    );
    let meta = std::fs::read_to_string(rig.dir.join("meta")).unwrap_or_default();
    assert!(
        meta.contains(&format!("parent_archive_id={UUID}")),
        "the child records its lineage:\n{meta}"
    );
    assert!(
        meta.contains("seat.main=fake"),
        "the child starts the FROZEN roster:\n{meta}"
    );
}

// ---------------------------------------------------------------------------
// the AMBIENT server, isolated
// ---------------------------------------------------------------------------

/// A rig whose "ambient" tmux server is nobody else's.
struct AmbientRig {
    home: PathBuf,
}

impl AmbientRig {
    fn new(tag: &str) -> Self {
        let home = PathBuf::from(format!("/tmp/aeamb.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("sessions")).expect("a scratch AE_HOME");
        Self { home }
    }

    /// tmux with the OPERATOR'S ENVIRONMENT DROPPED.
    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut invocation = super::parity::Invocation::new("tmux")
            .env_cleared()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("TMUX_TMPDIR", &self.home)
            .arg("-f")
            .arg("/dev/null");
        for arg in tail {
            invocation = invocation.arg(arg);
        }
        let out = self.home.join("t-out");
        let err = self.home.join("t-err");
        let ran = super::parity::capture::raw::run(&invocation, &self.home, &out, &err);
        let succeeded = ran.is_ok_and(|status| {
            matches!(
                status.outcome(),
                super::parity::capture::ExitOutcome::Code(0)
            )
        });
        (succeeded, std::fs::read_to_string(&out).unwrap_or_default())
    }

    /// One core subcommand, with this rig's home AND its ambient server.
    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let mut cmd = ae();
        cmd.env("AE_HOME", &self.home);
        cmd.env("TMUX_TMPDIR", &self.home);
        cmd.env_remove("TMUX");
        cmd.env_remove("TMUX_PANE");
        for arg in args {
            cmd.arg(arg);
        }
        let out = bounded(
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("the ae binary should run"),
            Duration::from_secs(30),
        )
        .expect("the core returned");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// One public command as if a shell inside `pane` invoked it.
    fn run_inside(&self, pane: &str, args: &[&str]) -> (Option<i32>, String, String) {
        let (_, identity) = self.tmux(&["display-message", "-p", "#{socket_path} | #{pid}"]);
        let (socket, pid) = identity
            .trim()
            .split_once(" | ")
            .expect("the isolated server identity");
        let mut cmd = ae();
        cmd.env("AE_HOME", &self.home);
        cmd.env("TMUX_TMPDIR", &self.home);
        cmd.env("TMUX", format!("{socket},{pid},0"));
        cmd.env("TMUX_PANE", pane);
        for arg in args {
            cmd.arg(arg);
        }
        let out = bounded(
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("the ae binary should run"),
            Duration::from_secs(30),
        )
        .expect("the core returned");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// A live session on the ambient server, plus the state directory that
    /// makes it one of ae's — with `server_rows` verbatim in its meta.
    fn plant(&self, name: &str, server_rows: &str) -> PathBuf {
        assert!(
            self.tmux(&["new-session", "-d", "-s", name, "sh"]).0,
            "the ambient session '{name}' starts"
        );
        // THE ISOLATION GUARD, checked before anything acts.
        let live = self.sessions();
        assert_eq!(
            live,
            vec![name.to_owned()],
            "the ambient server must be this rig's alone"
        );
        let dir = self.home.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("a session dir");
        std::fs::write(
            dir.join("meta"),
            format!(
                "session={name}\nsession_id={UUID}\nmode=local\nlayout=vertical\n\
                 work_dir={home}\norigin={home}\nmain_pane=%0\nschema=2\n\
                 seat.main=lead\nprofile.main=fake\nagent_bin.main=sh\n{server_rows}",
                home = self.home.display(),
            ),
        )
        .expect("a v2 meta");
        dir
    }

    fn sessions(&self) -> Vec<String> {
        let (_, listed) = self.tmux(&["list-sessions", "-F", "#{session_name}"]);
        listed.lines().map(str::to_owned).collect()
    }
}

impl Drop for AmbientRig {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

#[test]
fn a_bare_stop_from_a_non_ae_tmux_pane_has_no_inferred_target() {
    let rig = AmbientRig::new("foreignstop");
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "foreign", "sh"]).0,
        "the non-ae session starts"
    );
    let (_, panes) = rig.tmux(&["list-panes", "-t", "=foreign", "-F", "#{pane_id}"]);
    let pane = panes.lines().next().unwrap_or_default();
    assert!(!pane.is_empty(), "the non-ae pane: {panes}");

    let (code, out, err) = rig.run_inside(pane, &["stop"]);

    assert_eq!(code, Some(2), "stdout: {out}\nstderr: {err}");
    assert_eq!(err, "Usage: _stop <session-name|all> [-y] [--self]\n");
    assert_eq!(rig.sessions(), vec!["foreign"], "nothing was killed");
}

/// B2: an unresolvable server record must not be answered with the ambient one.
#[test]
fn an_ambiguous_server_record_refuses_the_rename_rather_than_taking_an_ambient_session() {
    let rig = AmbientRig::new("ambrn");
    let dir = rig.plant("ambold", "tmux_server_kind=ambiguous\ntmux_server=work\n");

    let (code, out, err) = rig.run(&["rename", "ambold", "ambnew"]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(
        err.contains("tmux_server_kind") && err.contains("tmux_server"),
        "the refusal names the rows an operator has to fix: {err}"
    );
    assert!(err.contains("Nothing was renamed"), "{err}");

    // THE POINT.
    let live = rig.sessions();
    assert!(
        live.iter().any(|name| name == "ambold"),
        "the ambient session must be untouched: {live:?}"
    );
    assert!(
        !live.iter().any(|name| name == "ambnew"),
        "nothing was renamed: {live:?}"
    );
    assert!(exists(&dir), "the state directory stays put");
    assert!(!exists(&rig.home.join("sessions").join("ambnew")));
}

/// The control: a record with NO server rows is not the same defect.
#[test]
fn a_session_that_records_no_server_still_renames_on_the_ambient_one() {
    let rig = AmbientRig::new("ambok");
    // This fixture has no helper links or monitor panes; disable the watchdog
    // explicitly so the test stays scoped to ambient-server resolution.
    rig.plant("ambplain", "watchdog=false\n");

    let (code, out, err) = rig.run(&["rename", "ambplain", "ambmoved"]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.contains("Renamed 'ambplain' → 'ambmoved'"), "{out}");
    let live = rig.sessions();
    assert!(
        live.iter().any(|name| name == "ambmoved"),
        "the ambient session was renamed: {live:?}"
    );
    assert!(exists(&rig.home.join("sessions").join("ambmoved")));
}

/// B2, the watchdog half: start, stop and status all address the session by
/// name on the server the record names, so an unresolvable record has to stop
/// them too.
#[test]
fn an_ambiguous_server_record_refuses_every_watchdog_command() {
    let rig = AmbientRig::new("ambwd");
    rig.plant("ambwd1", "tmux_server_kind=ambiguous\ntmux_server=work\n");

    for verb in ["status", "start", "stop"] {
        let (code, out, err) = rig.run(&["_watchdog", verb, "ambwd1"]);
        assert_eq!(code, Some(1), "{verb}: stdout: {out}\nstderr: {err}");
        assert!(
            err.contains("tmux_server_kind") && err.contains("The watchdog was not touched"),
            "{verb}: {err}"
        );
    }
}

/// I1: a handover that never happened is RECORDED as not having happened.
#[test]
fn a_supervisor_that_never_started_is_recorded_as_a_failed_stop() {
    let rig = Rig::new("nosuper");
    let mut cmd = ae();
    cmd.env("AE_HOME", &rig.home);
    cmd.env("PATH", "");
    cmd.env_remove("TMUX");
    cmd.env_remove("TMUX_PANE");
    let out = bounded(
        cmd.args(["_stop", "--self", &rig.name, "-y"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the ae binary should run"),
        Duration::from_secs(30),
    )
    .expect("the core returned");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("could not start the supervisor"),
        "{stderr}"
    );

    let events = std::fs::read_to_string(rig.dir.join("events.jsonl")).unwrap_or_default();
    assert!(
        events.contains("\"action\":\"stop-request\""),
        "the intent was recorded: {events}"
    );
    assert!(
        events.contains("\"action\":\"stop-result\""),
        "and so must the outcome be — a request with no result reads as a stop still running: {events}"
    );
    assert!(
        events.contains("FAILED: supervisor did not start"),
        "the result says what failed: {events}"
    );
    assert!(rig.session_is_live(), "nothing was stopped");
}

/// I2: `stop all` from inside a target ASKS, when there is a terminal to ask on.
#[test]
fn stop_all_from_inside_a_target_prompts_on_a_terminal() {
    let rig = AmbientRig::new("stpall");
    rig.plant("stpone", "");
    let (_, panes) = rig.tmux(&["list-panes", "-t", "stpone", "-F", "#{pane_id}"]);
    let pane = panes.lines().next().unwrap_or_default().to_owned();
    assert!(!pane.is_empty(), "the caller's pane: {panes}");
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "lead"])
            .0
    );

    // The command runs as a PANE's process, so its stdin is a tty.
    let script = rig.home.join("runner.sh");
    let log = rig.home.join("stop-out");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nAE_HOME={home} {ae} _stop all --pane {pane} > {log} 2>&1\n\
             echo \"EXIT:$?\" >> {log}\nsleep 30\n",
            home = rig.home.display(),
            ae = env!("CARGO_BIN_EXE_ae"),
            log = log.display(),
        ),
    )
    .expect("the runner");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("an executable runner");
    }
    assert!(
        rig.tmux(&[
            "new-session",
            "-d",
            "-s",
            "runner",
            &script.display().to_string()
        ])
        .0,
        "the runner pane starts"
    );

    let read = || std::fs::read_to_string(&log).unwrap_or_default();
    let mut said = String::new();
    for _ in 0..200 {
        said = read();
        if said.contains("Stop all") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        said.contains("Stop all") && said.contains("[y/N]"),
        "the fleet stop must ASK before it detaches: {said:?}"
    );

    // Answering no stops nothing — and proves the process was really waiting on
    // that terminal rather than having printed and moved on.
    assert!(rig.tmux(&["send-keys", "-t", "runner", "n", "Enter"]).0);
    for _ in 0..200 {
        said = read();
        if said.contains("EXIT:") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(said.contains("Nothing was stopped."), "{said:?}");
    assert!(said.contains("EXIT:1"), "{said:?}");
    let live = rig.sessions();
    assert!(
        live.iter().any(|name| name == "stpone"),
        "the fleet is untouched: {live:?}"
    );
}

/// An unpaired target cannot prove that it owns the caller. With no terminal,
/// the ordinary fleet confirmation refuses and names its authorization flag.
#[test]
fn stop_all_from_inside_a_target_with_no_terminal_still_needs_the_flag() {
    let rig = AmbientRig::new("stpntty");
    rig.plant("stptwo", "");
    let (_, panes) = rig.tmux(&["list-panes", "-t", "stptwo", "-F", "#{pane_id}"]);
    let pane = panes.lines().next().unwrap_or_default().to_owned();
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "lead"])
            .0
    );

    let (code, out, err) = rig.run_inside(&pane, &["_stop", "all", "--pane", &pane]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert_eq!(
        err,
        "Error: 'stop all' stops every running ae session (1), and there is no terminal to confirm on.\n  Re-run with -y: ae stop all -y\n  Nothing was stopped.\n"
    );
    let live = rig.sessions();
    assert!(
        live.iter().any(|name| name == "stptwo"),
        "nothing was stopped: {live:?}"
    );
}

/// The lifecycle lock EXCLUDES an end from a concurrent start or resume.
#[test]
fn a_held_lifecycle_lock_refuses_the_end_and_preserves_the_whole_session() {
    let rig = Rig::new("lock");
    let lock_path = rig
        .home
        .join("sessions")
        .join(format!(".lifecycle.{}.lock", rig.name));
    let held = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&lock_path)
        .expect("the lock file opens");
    held.try_lock().expect("this test takes the lock first");

    let (code, _, stderr) = rig.run(&["_end", "-f", &rig.name]);
    assert_eq!(code, Some(1), "the end fails rather than waiting forever");
    assert!(
        stderr.contains("another lifecycle operation (start/resume/end) is in progress")
            && stderr.contains("State preserved."),
        "the refusal says why and what it kept: {stderr}"
    );
    assert!(rig.dir.join("meta").is_file(), "the meta is still there");
    assert!(rig.session_is_live(), "and the session was never stopped");

    // Released, the very same end goes through — so the refusal was the lock
    // and not some other property of this session.
    drop(held);
    let (code, stdout, stderr) = rig.run(&["_end", "-f", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(!rig.dir.exists(), "the uncontended end removed the session");
}
