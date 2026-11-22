//! Public autostart contract, authored before the implementation.
//! Start real ae binaries on private tmux, with fixture seats that never call a model.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns isolated config, PTY output and state fixtures"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{Rig, ae, bare_session, stop};
use ae::events::Event;

const ORCH: &str = "orchestrator";
const TARGET: &str = "=orchestrator";
const WAIT: Duration = Duration::from_secs(15);
const CONFIG: &str = "[profiles]\nidle = /bin/sleep 600\n\
    [roster]\nlead = idle\norchestrator = idle\n\
    [workspace]\nmain = lead\nlayout = vertical\nchat = off\nwatchdog = false\n\
    quota = off\nauto_upgrade = off\n";

struct Fixture {
    rig: Rig,
    tty: PathBuf,
    launch_server: PathBuf,
}

impl Fixture {
    fn new(tag: &str, config: &str) -> Self {
        let mut rig = Rig::idle(tag);
        fs::write(&rig.config, config).expect("inert global fixture config");
        let tty = rig.scratch.join("tty");
        rig.scratch.add_tmux_server(tty.clone());
        bare_session(&rig, &tty, "terminal");
        let launch_server = rig.sock.clone();
        Self {
            rig,
            tty,
            launch_server,
        }
    }

    fn pane(&self, name: &str, suppress: &str, gate: &Path, args: &[&str]) -> String {
        // Both streams remain on a PTY. The shell records the exit status
        // out of band and holds the pane so tmux can report the output.
        // Clearing TMUX simulates an external human terminal.
        let result = self.rig.scratch.join(format!("{name}.status"));
        let mut command = vec![
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "terminal:",
            "-n",
            name,
            "-c",
            self.rig.project.to_str().expect("fixture project"),
            "/bin/sh",
            "-c",
            r#"result=$1; gate=$2; shift 2
if [ -n "$gate" ]; then
    : > "$result.ready"
    while [ ! -e "$gate" ]; do /bin/sleep 0.02; done
fi
env -u TMUX -u TMUX_PANE "$@"
code=$?
printf '%s\n' "$code" > "$result"
exec /bin/sleep 600"#,
            "ae-terminal",
            result.to_str().expect("status path"),
            gate.to_str().expect("gate path"),
        ];
        let isolated = ae();
        let path = isolated
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .and_then(|(_, value)| value)
            .expect("the runner's harness shadow PATH");
        let environment = [
            format!("PATH={}", path.to_string_lossy()),
            format!("HOME={}", self.rig.scratch.display()),
            format!("AE_HOME={}", self.rig.home.display()),
            format!("CONFIG_FILE={}", self.rig.config.display()),
            format!("TMUX_TMPDIR={}", self.rig.scratch.display()),
            "AE_TMUX_SERVER_KIND=socket".to_owned(),
            format!("AE_TMUX_SERVER={}", self.launch_server.display()),
            format!("AE_NO_AUTOSTART={suppress}"),
            "SHELL=/bin/sh".to_owned(),
        ];
        // env consumes these assignments before it execs ae.
        command.extend(environment.iter().map(String::as_str));
        command.push(env!("CARGO_BIN_EXE_ae"));
        command.extend_from_slice(args);
        let (ok, pane) = self.rig.tmux_on(&self.tty, &command);
        assert!(ok, "private terminal for {name}: {pane}");
        pane.trim().to_owned()
    }

    fn screen(&self, pane: &str) -> String {
        let (ok, screen) = self.rig.tmux_on(
            &self.tty,
            &["capture-pane", "-p", "-J", "-S", "-", "-t", pane],
        );
        assert!(ok, "read the private terminal: {screen}");
        screen
    }

    fn completed(&self, name: &str, pane: &str) -> String {
        self.completed_with(name, pane, "0")
    }

    fn completed_with(&self, name: &str, pane: &str, expected: &str) -> String {
        let status = self.rig.scratch.join(format!("{name}.status"));
        let deadline = Instant::now() + WAIT;
        loop {
            if let Ok(code) = fs::read_to_string(&status)
                && code.ends_with('\n')
            {
                let screen = self.screen(pane);
                assert_eq!(code.trim(), expected, "{name} exit:\n{screen}");
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "{name} did not return:\n{}",
                self.screen(pane)
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn run(&self, name: &str, suppress: &str, args: &[&str]) -> String {
        let pane = self.pane(name, suppress, Path::new(""), args);
        self.completed(name, &pane)
    }

    fn ordinary(&self, name: &str, suppress: &str) -> String {
        self.run(name, suppress, &[name, "--local", "--no-attach"])
    }

    fn orchestrator_dir(&self) -> PathBuf {
        self.rig.home.join("sessions").join(ORCH)
    }

    fn crash(&self, sessions: &[&str]) {
        // Each launched seat had a watchdog beat before the socket went stale.
        // Use the same private-server crash proof as restore_spec::Fleet.
        for session in sessions {
            let dir = self.rig.home.join("sessions").join(session);
            fs::File::create(dir.join(".watchdog-beat")).expect("fixture's last watchdog beat");
        }
        let (ok, pid) = self.rig.tmux(&["display-message", "-p", "#{pid}"]);
        let pid = pid.trim().parse::<u32>().expect("private server pid");
        assert!(ok && pid > 1, "a proved private server pid");
        let _ = self.rig.tmux(&["run-shell", &format!("kill -KILL {pid}")]);
        super::super::relay_chat_spec::wait_for(
            "the private socket proves a crashed server",
            || {
                !self.rig.tmux(&["list-sessions"]).0
                    && fs::read_to_string(self.rig.scratch.join("stderr"))
                        .is_ok_and(|line| line.starts_with("no server running on "))
            },
        );
    }

    fn restore_and_detach(&self, suppress: &str, exit: &str) -> String {
        let pane = self.pane("reopen", suppress, Path::new(""), &["--tmux"]);
        let bound = Instant::now() + WAIT;
        let client = loop {
            let (ok, out) =
                self.rig
                    .tmux(&["list-clients", "-F", "#{client_name}|#{session_name}"]);
            if ok && let Some((client, session)) = out.trim().split_once('|') {
                assert_eq!(
                    session, "ordinary",
                    "bare ae kept its ordinary attach target"
                );
                break client.to_owned();
            }
            assert!(
                Instant::now() < bound,
                "no client attached:\n{}",
                self.screen(&pane)
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(
            self.rig.tmux(&["detach-client", "-t", &client]).0,
            "detach only the fixture's client"
        );
        self.completed_with("reopen", &pane, exit)
    }

    fn orchestrator(&self, starts: usize) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let rows = self
                .rig
                .tmux(&[
                    "list-sessions",
                    "-F",
                    "#{session_name}|#{session_attached}|#{session_id}|#{@ae_session_uuid}",
                ])
                .1;
            if let Some(row) = rows.lines().find(|row| row.starts_with("orchestrator|"))
                && let Some(id) = row.strip_prefix("orchestrator|0|")
                && id.split_once('|').is_some_and(|(_, uuid)| !uuid.is_empty())
                && self.start_count() >= starts
            {
                return id.to_owned();
            }
            assert!(
                Instant::now() < deadline,
                "expected a detached stamped orchestrator: {rows}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn start_count(&self) -> usize {
        let events =
            fs::read_to_string(self.orchestrator_dir().join("events.jsonl")).unwrap_or_default();
        events
            .lines()
            .filter_map(|line| Event::parse_line(line).ok())
            .filter(|event| event.action == "session-start")
            .count()
    }

    fn absent(&self) {
        assert!(!self.rig.tmux(&["has-session", "-t", TARGET]).0);
        assert!(!self.orchestrator_dir().exists());
        assert!(!self.rig.home.join("orchestrator.config").exists());
    }
}

#[test]
fn default_starts_cold_and_detached_in_the_requested_sandbox() {
    let f = Fixture::new("oa-cold", CONFIG);
    f.absent();
    let output = f.ordinary("origin", "0");
    assert!(!output.contains(ORCH), "{output}");
    f.orchestrator(1);
    assert_eq!(f.start_count(), 1);
    assert!(f.rig.home.join("orchestrator.config").is_file());
    assert!(!f.rig.tmux_on(&f.tty, &["has-session", "-t", TARGET]).0);
    assert!(
        !f.rig
            .tmux_named(
                ae::doors::DEFAULT_SERVER_NAME,
                &["has-session", "-t", TARGET]
            )
            .0
    );
    assert!(!f.rig.scratch.join(".ae/sessions/orchestrator").exists());
}

#[test]
fn app_starts_the_orchestrator_without_a_session_launch() {
    let f = Fixture::new("oa-app", CONFIG);
    f.absent();
    assert!(
        f.rig
            .tmux_on(
                &f.tty,
                &["set-option", "-t", "terminal", "default-size", "120x40"]
            )
            .0,
        "the app's private terminal meets the sidebar dimensions"
    );
    let pane = f.pane("app", "0", Path::new(""), &["app"]);
    f.orchestrator(1);
    let deadline = Instant::now() + WAIT;
    loop {
        let screen = f.screen(&pane);
        if screen.contains("No sessions.") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the app did not paint the fleet:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_existing_orchestrator_is_left_alone() {
    let mut f = Fixture::new("oa-live", CONFIG);
    f.run("manual", "1", &[ORCH, "--no-attach"]);
    let identity = f.orchestrator(1);
    let meta = f.orchestrator_dir().join("meta");
    let before = fs::read(&meta).expect("initial state");
    // Explicit reattach would restamp this hook. Keep the orchestrator itself
    // fully stamped and running, and observe that autostart leaves it alone.
    let hook = [
        "set-hook",
        "-t",
        ORCH,
        "client-session-changed",
        "display-message sentinel",
    ];
    assert!(f.rig.tmux(&hook).0, "install the observer");
    let hook = ["show-hooks", "-t", ORCH, "client-session-changed"];
    let before_hook = f.rig.tmux(&hook);
    assert!(before_hook.0 && before_hook.1.contains("sentinel"));
    let output = f.ordinary("other", "0");
    assert!(!output.contains(ORCH), "{output}");
    f.launch_server.clone_from(&f.tty);
    let output = f.ordinary("cross", "0");
    assert!(!output.contains(ORCH), "{output}");
    assert!(!f.rig.tmux_on(&f.tty, &["has-session", "-t", TARGET]).0);
    assert_eq!(f.orchestrator(1), identity);
    assert_eq!(fs::read(meta).expect("current state"), before);
    assert_eq!(f.rig.tmux(&hook), before_hook);
    assert_eq!(f.start_count(), 1);
}

#[test]
fn a_stopped_orchestrator_resumes_the_same_record() {
    let f = Fixture::new("oa-resume", CONFIG);
    f.run("manual", "1", &[ORCH, "--no-attach"]);
    let before = f.orchestrator(1);
    bare_session(&f.rig, &f.rig.sock, "keeper");
    stop(&f.rig, ORCH);
    let output = f.ordinary("origin", "0");
    assert!(!output.contains(ORCH), "{output}");
    let after = f.orchestrator(2);
    assert_ne!(before, after, "tmux incarnation changed");
    assert_eq!(
        before.split_once('|').map(|(_, uuid)| uuid),
        after.split_once('|').map(|(_, uuid)| uuid)
    );
    assert_eq!(f.start_count(), 2);
}

#[test]
fn concurrent_interactive_launches_create_one_orchestrator() {
    let f = Fixture::new("oa-race", CONFIG);
    bare_session(&f.rig, &f.rig.sock, "keeper");
    let gate = f.rig.scratch.join("go");
    let a = f.pane("one", "0", &gate, &["one", "--local", "--no-attach"]);
    let b = f.pane("two", "0", &gate, &["two", "--local", "--no-attach"]);
    super::super::relay_chat_spec::wait_for("both terminal launches reached the gate", || {
        ["one", "two"]
            .into_iter()
            .all(|name| f.rig.scratch.join(format!("{name}.status.ready")).is_file())
    });
    fs::write(gate, "").expect("release both terminals");
    for (name, pane) in [("one", a), ("two", b)] {
        let output = f.completed(name, &pane);
        assert!(!output.contains(ORCH), "{output}");
    }
    f.orchestrator(1);
    assert_eq!(f.start_count(), 1);
}

#[test]
fn global_off_and_per_invocation_suppression_are_independent() {
    let off = format!("{CONFIG}orchestrator_autostart = off\n");
    let f = Fixture::new("oa-off", &off);
    let output = f.ordinary("off", "0");
    assert!(!output.contains(ORCH), "{output}");
    f.absent();

    fs::write(
        &f.rig.config,
        format!("{CONFIG}orchestrator_autostart = on\n"),
    )
    .expect("enable autostart globally");
    let output = f.ordinary("once", "1");
    assert!(!output.contains(ORCH), "{output}");
    f.absent();

    let output = f.ordinary("again", "0");
    assert!(!output.contains(ORCH), "{output}");
    f.orchestrator(1);
    assert_eq!(f.start_count(), 1);
}

#[test]
fn launch_reports_a_late_start_refusal_once_and_still_succeeds() {
    let f = Fixture::new("oa-fail", CONFIG);
    f.run("manual", "1", &[ORCH, "--no-attach"]);
    bare_session(&f.rig, &f.rig.sock, "keeper");
    stop(&f.rig, ORCH);
    let stamp = f.orchestrator_dir().join(".launch-attempt");
    fs::remove_file(&stamp).expect("remove the fixture's old stamp");
    fs::create_dir(&stamp).expect("obstruct publishing a new stamp");
    let output = f.ordinary("origin", "0");
    let lines: Vec<_> = output.lines().filter(|line| line.contains(ORCH)).collect();
    assert_eq!(lines.len(), 1, "{output}");
    assert!(lines[0].contains("launch attempt"), "{output}");
    assert!(lines[0].contains("ae orchestrator"), "{output}");
    assert!(f.rig.tmux(&["has-session", "-t", "=origin"]).0);
    assert!(!f.rig.tmux(&["has-session", "-t", TARGET]).0);
    assert_eq!(f.start_count(), 1);
}

#[test]
fn crash_recovers_the_orchestrator_quietly_without_changing_the_attach_target() {
    let f = Fixture::new("oa-crash", CONFIG);
    f.run("manual", "1", &[ORCH, "--no-attach"]);
    let before = f.orchestrator(1);
    f.ordinary("ordinary", "1");
    f.crash(&[ORCH, "ordinary"]);

    let output = f.restore_and_detach("0", "0");
    assert!(output.contains("ae: restored ordinary"), "{output}");
    assert!(!output.contains(ORCH), "{output}");
    let after = f.orchestrator(2);
    assert_eq!(
        before.split_once('|').map(|(_, uuid)| uuid),
        after.split_once('|').map(|(_, uuid)| uuid)
    );
    assert_eq!(f.start_count(), 2);
}

#[test]
fn both_opt_outs_keep_the_crashed_orchestrator_stopped_during_ordinary_restore() {
    let off = format!("{CONFIG}orchestrator_autostart = off\n");
    for (tag, config, suppress) in [("oa-cr-off", off.as_str(), "0"), ("oa-cr-env", CONFIG, "1")] {
        let f = Fixture::new(tag, config);
        f.run("manual", "1", &[ORCH, "--no-attach"]);
        f.ordinary("ordinary", "1");
        f.crash(&[ORCH, "ordinary"]);
        let meta = f.orchestrator_dir().join("meta");
        let saved = fs::read(&meta).expect("saved orchestrator metadata");

        let output = f.restore_and_detach(suppress, "0");
        assert!(output.contains("ae: restored ordinary"), "{output}");
        assert!(!output.contains(ORCH), "{output}");
        assert!(!f.rig.tmux(&["has-session", "-t", TARGET]).0);
        assert_eq!(f.start_count(), 1);
        assert_eq!(
            fs::read(meta).expect("retained orchestrator metadata"),
            saved
        );
    }
}

#[test]
fn an_ordinary_restore_failure_keeps_its_diagnostic_and_exit_status() {
    let f = Fixture::new("oa-cr-fail", CONFIG);
    f.ordinary("ordinary", "1");
    f.ordinary("broken", "1");
    f.crash(&["broken", "ordinary"]);
    let meta = f.rig.home.join("sessions/broken/meta");
    let saved = fs::read_to_string(&meta).expect("saved ordinary metadata");
    let damaged = saved.replace("profile.main=idle\n", "profile.main=unavailable\n");
    assert_ne!(
        damaged, saved,
        "the fixture really selected an unavailable profile"
    );
    fs::write(&meta, &damaged).expect("save the unavailable profile");

    let output = f.restore_and_detach("1", "1");
    assert!(output.contains("ae: restore failed broken:"), "{output}");
    assert!(output.contains("ae: restored ordinary"), "{output}");
    assert!(!f.rig.tmux(&["has-session", "-t", "=broken"]).0);
    assert_eq!(
        fs::read_to_string(meta).expect("unchanged ordinary metadata"),
        damaged
    );
}

#[test]
fn bare_tmux_never_attaches_a_new_orchestrator_when_the_ordinary_target_closes() {
    let f = Fixture::new("oa-attach-gone", CONFIG);
    f.ordinary("ordinary", "1");
    // This hook runs during the companion's new-session command, before
    // the foreground invocation can attach.
    assert!(
        f.rig
            .tmux(&[
                "set-hook",
                "-g",
                "after-new-session",
                "kill-session -t ordinary",
            ])
            .0,
        "remove the ordinary session on the next private launch"
    );
    let pane = f.pane("reopen", "0", Path::new(""), &["--tmux"]);
    let deadline = Instant::now() + WAIT;
    loop {
        let (_, clients) = f.rig.tmux(&["list-clients", "-F", "#{session_name}"]);
        assert!(
            !clients.lines().any(|name| name == ORCH),
            "bare ae attached to its new orchestrator"
        );
        if f.rig.scratch.join("reopen.status").is_file() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "bare ae did not return after its target closed:\n{}",
            f.screen(&pane)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(!f.rig.tmux(&["has-session", "-t", "=ordinary"]).0);
    f.orchestrator(1);
    assert_eq!(f.start_count(), 1);
}

#[test]
fn a_foreign_session_cannot_satisfy_autostart_with_a_saved_record() {
    let f = Fixture::new("oa-foreign", CONFIG);
    f.run("manual", "1", &[ORCH, "--no-attach"]);
    f.orchestrator(1);
    bare_session(&f.rig, &f.rig.sock, "keeper");
    stop(&f.rig, ORCH);
    bare_session(&f.rig, &f.rig.sock, ORCH);
    let meta = f.orchestrator_dir().join("meta");
    let saved = fs::read(&meta).expect("saved orchestrator metadata");
    let observe = || {
        let (ok, rows) = f.rig.tmux(&[
            "list-sessions",
            "-F",
            "#{session_name}|#{session_id}|#{@ae_session_uuid}",
        ]);
        assert!(ok, "read the private server: {rows}");
        rows.lines()
            .find(|row| row.starts_with("orchestrator|"))
            .expect("foreign orchestrator session")
            .to_owned()
    };
    let before = observe();
    assert!(before.ends_with('|'), "fixture is unstamped: {before}");

    let output = f.ordinary("origin", "0");
    let lines: Vec<_> = output.lines().filter(|line| line.contains(ORCH)).collect();
    assert_eq!(lines.len(), 1, "{output}");
    assert!(lines[0].contains("autostart failed"), "{output}");
    assert!(lines[0].contains("ae orchestrator"), "{output}");
    assert_eq!(observe(), before);
    assert_eq!(
        fs::read(meta).expect("retained orchestrator metadata"),
        saved
    );
    assert_eq!(f.start_count(), 1);
}
