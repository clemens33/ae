//! The public entry, black-box: the whole surface a human types at `ae`.
//!
//! The subject is the real binary, because the subject IS what a human typed at
//! `ae`: which environment DOOR supplies which fact, which word routes where,
//! what a cut word does instead of becoming a session name, and what a first run
//! leaves behind.
//!
//! THERE IS NO PREAMBLE TO SPELL. Every fact the product reads is an
//! environment variable this rig sets, or the working directory it runs in —
//! which is the whole contract ae has with a shell.
//!
//! ONE DOOR IS NOT BLACK-BOXED HERE, and the reason is that it has no black-box
//! surface: `TMUX_PANE` only changes an answer when it names a pane that is a
//! REAL ae agent's on the resolved server, and every refusal short of that is
//! the same sentence with the door set or unset (measured). Planting one costs
//! a full launch with an agent in it. What is
//! pinned here instead is the half that can be: the router appends `--pane` for
//! `stop` and `watchdog` and only when the caller named none
//! (`src/entry.rs`'s `the_pane_is_appended_only_when_the_caller_named_none`),
//! and the read itself is one documented door in `src/doors.rs`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use super::cli::{OwnedScratch, ae, git_in};
use super::parity::{Invocation, capture::raw};
use super::phase2::{run_tmux, tmux_present};

const IDLE_CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
     [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n";

/// An isolated ae home and a project directory.
struct Rig {
    scratch: OwnedScratch,
    home: PathBuf,
    project: PathBuf,
    sock: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::existing(PathBuf::from(format!(
            "/tmp/aeentry.{}.{tag}",
            std::process::id()
        )));
        let home = scratch.join("aehome");
        let project = scratch.join("project");
        assert!(std::fs::create_dir_all(&project).is_ok(), "a project dir");
        // tmux derives its default socket directory from this same effective UID.
        #[cfg(unix)]
        let uid = std::fs::metadata(&scratch).map_or(0, |metadata| metadata.uid());
        #[cfg(not(unix))]
        let uid = 0;
        let sock = scratch.join("sock");
        scratch.add_tmux_server(sock.clone());
        scratch.add_tmux_server(scratch.join(format!("tmux-{uid}")).join("default"));
        Self {
            scratch,
            home,
            project,
            sock,
        }
    }

    fn config(&self) -> PathBuf {
        self.home.join("config")
    }

    /// The same rig with the harmless idle profile used by routing tests.
    fn idle(tag: &str) -> Self {
        let rig = Self::new(tag);
        assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
        assert!(
            std::fs::write(rig.config(), IDLE_CONFIG).is_ok(),
            "an idle config"
        );
        rig
    }

    /// Install command-name fakes for the seeded config's real profile rows.
    /// They record which profile reached the pane, then stay alive until the
    /// rig's server cleanup kills them.
    #[cfg(unix)]
    fn fake_profiles(&self) -> (PathBuf, PathBuf) {
        let bin = self.scratch.join("fake-bin");
        let marker = self.scratch.join("fake-agent-launched");
        assert!(std::fs::create_dir_all(&bin).is_ok(), "a fake bin dir");
        for tool in ["claude", "codex"] {
            let body = format!(
                "#!/usr/bin/perl\nuse strict;\nuse warnings;\nsystem(\"stty raw -echo 2>/dev/null\");\nbinmode(STDIN, ':raw');\nbinmode(STDOUT, ':raw');\n$| = 1;\nopen(my $marker, '>>', '{}') or die; print $marker '{}\\n'; close($marker);\nif ($0 =~ /codex$/ && $ENV{{AE_HOME}}) {{\n    if (opendir(my $sessions, \"$ENV{{AE_HOME}}/sessions\")) {{\n        for my $name (readdir($sessions)) {{\n            next if $name =~ /^\\./;\n            my $sid = \"$ENV{{AE_HOME}}/sessions/$name/codex.worker.0.sid\";\n            if (open(my $file, '>', $sid)) {{ print $file \"0199c0de-1234-4890-abcd-ef0123456789\\n\"; close($file); last; }}\n        }}\n        closedir($sessions);\n    }}\n}}\nprint \"\\e[?2004h\";\nprint \"\\e[H\\e[2J\";\nprint \"\\e[1m\\xe2\\x9d\\xaf\\e[0m\\xc2\\xa0\\r\\n\";\nprint ((\"\\xe2\\x94\\x80\" x 400), \"\\r\\n\");\nsleep 600;\n",
                marker.display(),
                tool,
            );
            let path = bin.join(tool);
            assert!(std::fs::write(&path, body).is_ok(), "the fake {tool}");
            assert!(
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).is_ok(),
                "an executable fake {tool}"
            );
        }
        (bin, marker)
    }

    fn sessions(&self) -> PathBuf {
        self.home.join("sessions")
    }

    /// Run the product as a shell would: this rig's doors in the environment,
    /// its project as the working directory, and `argv` verbatim.
    fn run(&self, argv: &[&str]) -> (Option<i32>, String, String) {
        self.run_on(None, argv)
    }

    #[cfg(unix)]
    fn run_on_with_path(
        &self,
        server: Option<&Path>,
        path: &Path,
        argv: &[&str],
    ) -> (Option<i32>, String, String) {
        let mut command = ae();
        let path = format!(
            "{}:{}",
            path.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &self.scratch)
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", self.config())
            .env("AE_NO_AUTOSTART", "1")
            .env("TMUX_TMPDIR", &self.scratch)
            .env("PATH", path)
            .current_dir(&self.project);
        if let Some(socket) = server {
            command
                .env("AE_TMUX_SERVER_KIND", "socket")
                .env("AE_TMUX_SERVER", socket);
        } else {
            command
                .env_remove("AE_TMUX_SERVER_KIND")
                .env_remove("AE_TMUX_SERVER");
        }
        let out = command
            .args(argv)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// The same, with the tmux server pair declared — the door the launch needs
    /// so it lands on this rig's own server and not the developer's.
    fn run_on(&self, server: Option<&Path>, argv: &[&str]) -> (Option<i32>, String, String) {
        self.run_from_on(&self.project, server, argv)
    }

    /// Run through the public entry from an arbitrary directory.
    fn run_from_on(
        &self,
        cwd: &Path,
        server: Option<&Path>,
        argv: &[&str],
    ) -> (Option<i32>, String, String) {
        let mut command = ae();
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &self.scratch)
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", self.config())
            .env("AE_NO_AUTOSTART", "1")
            .env("TMUX_TMPDIR", &self.scratch)
            .current_dir(cwd);
        if let Some(socket) = server {
            command
                .env("AE_TMUX_SERVER_KIND", "socket")
                .env("AE_TMUX_SERVER", socket);
        } else {
            command
                .env_remove("AE_TMUX_SERVER_KIND")
                .env_remove("AE_TMUX_SERVER");
        }
        let out = command
            .args(argv)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.sock.clone()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    fn default_tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Name(ae::doors::DEFAULT_SERVER_NAME.to_owned()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    fn historical_tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Name(ae::doors::HISTORICAL_SERVER_NAME.to_owned()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }
}

/// Confirm that a rig's detached tmux command is gone after its Drop guard.
fn assert_no_tmux_processes(scratch: &Path) {
    let probe = PathBuf::from(format!("/tmp/aeentry-pgrep.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&probe);
    assert!(
        std::fs::create_dir_all(&probe).is_ok(),
        "a pgrep scratch dir"
    );
    let out = probe.join("stdout");
    let err = probe.join("stderr");
    // Any process carrying this scratch path is a leak, including a fake agent
    // that outlives its tmux server.
    let pattern = scratch.display().to_string();
    let search = pattern
        .strip_prefix('/')
        .map_or_else(|| pattern.clone(), |rest| format!("[/]{rest}"));
    let invocation = Invocation::new("pgrep").arg("-fl").arg(search);
    let status = raw::run(&invocation, Path::new("/tmp"), &out, &err)
        .unwrap_or_else(|why| panic!("pgrep must run: {why}"));
    let matches = std::fs::read_to_string(&out).unwrap_or_default();
    assert!(
        matches!(
            status.outcome(),
            super::parity::capture::ExitOutcome::Code(1)
        ),
        "process remained for {}: {matches}",
        scratch.display()
    );
    let _ = std::fs::remove_dir_all(&probe);
}

/// Wait briefly for a pane's fake agent to record that `_run` reached it.
fn assert_agent_launched(marker: &Path, context: &str) {
    for _ in 0..200 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(marker.exists(), "{context}");
}

fn skip() -> bool {
    let probe = PathBuf::from(format!("/tmp/aeentry-probe.{}", std::process::id()));
    let _ = std::fs::create_dir_all(&probe);
    let present = tmux_present(&probe);
    let _ = std::fs::remove_dir_all(&probe);
    !present
}

// ---------------------------------------------------------------------------
// (1) the preamble parse
// ---------------------------------------------------------------------------

/// The entry's contract, end to end: the facts come from the environment, the
/// argv is the caller's alone, and an entry still answers.
#[test]
fn the_doors_carry_the_facts_and_the_argv_is_the_users_alone() {
    let rig = Rig::new("carry");
    let (code, stdout, stderr) = rig.run(&["version"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // Two lines: the core's version, then the tmux floor reading.
    assert!(
        stdout.starts_with(&format!("{}\n", ae::version_line())),
        "{stdout}"
    );
    // The doors alone write NOTHING: only a launch seeds a config.
    assert!(!rig.config().exists(), "a version query created state");
}

/// `version` answers AHEAD of every gate, including the doors themselves.
#[test]
fn version_answers_with_no_environment_at_all() {
    for word in ["version", "--version", "-V"] {
        let out = ae()
            .env_clear()
            .arg(word)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        assert_eq!(out.status.code(), Some(0), "{word}");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            stdout.starts_with(&format!("{}\n", ae::version_line())),
            "{word}: {stdout}"
        );
        // No PATH at all, so no tmux can be run: the floor line reports that
        // rather than failing the word that has to answer everywhere.
        assert!(
            stdout
                .lines()
                .nth(1)
                .unwrap_or_default()
                .starts_with("tmux not found"),
            "{word}: {stdout}"
        );
        assert!(out.stderr.is_empty(), "{word}");
    }
}

/// THE `AE_HOME` DOOR relocates every piece of state, and `CONFIG_FILE` names
/// the global config independently of it.
#[test]
fn the_home_and_config_doors_are_honoured_by_a_checkout_build() {
    let rig = Rig::new("homedoor");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    let (_, stdout, stderr) = rig.run(&["doctor"]);
    let report = format!("{stdout}{stderr}");
    assert!(
        report.contains(&rig.sessions().display().to_string()),
        "AE_HOME names the state root: {report}"
    );

    if skip() {
        return;
    }
    // CONFIG_FILE, through the surface that reads it: a first launch SEEDS the
    // global config, and it seeds the file this door names rather than the
    // default beside the home.
    let elsewhere = rig.scratch.join("elsewhere.config");
    let sock = rig.sock.clone();
    let mut command = ae();
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &elsewhere)
        .env("AE_NO_AUTOSTART", "1")
        .env("TMUX_TMPDIR", &rig.scratch)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &sock)
        .current_dir(&rig.project);
    let out = command
        .args(["--local", "cfgdoor"])
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    assert!(
        elsewhere.exists(),
        "the launch seeded the config CONFIG_FILE named: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !rig.config().exists(),
        "and never the default beside the home"
    );
}

/// A MACHINE THAT CANNOT SAY WHERE ITS STATE LIVES is refused before anything —
/// the one door with no default behind it.
#[test]
fn no_home_and_no_ae_home_is_the_one_refusal_the_doors_can_make() {
    let out = ae()
        .env_clear()
        .arg("list")
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    assert!(out.stdout.is_empty(), "a refusal must not reach stdout");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(ae::NO_STATE_ROOT),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A preamble flag is ordinary argv, and ae does not answer to it.
#[test]
fn a_preamble_flag_is_no_longer_a_flag_ae_answers_to() {
    let rig = Rig::new("nopreamble");
    let (code, stdout, stderr) = rig.run(&["--home", "/x", "--cwd", "/y", "--", "list"]);
    assert_ne!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        !stderr.contains("Usage: ae-core --home"),
        "the preamble usage line is gone with the parse: {stderr}"
    );
    assert!(
        stdout.is_empty(),
        "a refusal must not reach stdout: {stdout}"
    );
    assert!(!rig.sessions().exists(), "a refused argv built state");
}

/// NOTHING CHANGES FOR AN INTERNAL ENTRY CALLED BARE.
#[test]
fn an_internal_entry_keeps_its_own_grammar_and_pays_for_no_door() {
    let rig = Rig::new("internal");
    let dir = rig.sessions().join("s1");
    assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
    assert!(
        std::fs::write(dir.join("meta"), "session=s1\n").is_ok(),
        "meta"
    );
    let out = ae()
        .env_clear()
        .arg("_requests")
        .arg(&dir)
        .arg("all")
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    // env_clear is the assertion: an internal entry carries its own operands,
    // so it must answer with no HOME, no AE_HOME and no PATH to find tmux on.
    assert_eq!(
        out.status.code(),
        Some(0),
        "an internal entry must not need a door: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// An unserved `_` word FAILS CLOSED rather than becoming a session named after
/// ae's own namespace.
#[test]
fn an_unserved_internal_word_is_refused_by_name() {
    let rig = Rig::new("unserved");
    let (code, stdout, stderr) = rig.run(&["_recover-pending"]);
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains("unknown internal command"), "{stderr}");
    assert!(stderr.contains("_recover-pending"), "{stderr}");
    assert!(!rig.sessions().exists(), "a refused word built state");
}

/// THE SERVER PAIR IS READ BY *SET*, NOT BY NONEMPTY, and a pair that cannot be
/// typed is a refusal rather than a fallback.
#[test]
fn an_untypeable_server_pair_refuses_and_never_falls_back() {
    let rig = Rig::new("ambiguous");
    let mut command = ae();
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", rig.config())
        .env("AE_TMUX_SERVER_KIND", "ambiguous")
        .env("AE_TMUX_SERVER", "")
        .current_dir(&rig.project);
    let out = command
        .args(["--local", "nowhere"])
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("not a tmux server kind"), "{stderr}");
    assert!(
        stderr.contains("will not fall back to the ambient server"),
        "{stderr}"
    );
    assert!(
        !rig.sessions().join("nowhere").exists(),
        "a refused pair built state"
    );
}

// ---------------------------------------------------------------------------
// (2) the launch fall-through
// ---------------------------------------------------------------------------

/// The whole point of the slice: a bare word reaches `_launch` with the
/// preamble's facts and builds a real session.
#[test]
fn a_launch_candidate_becomes_a_session_from_the_preamble_facts() {
    if skip() {
        return;
    }
    let rig = Rig::new("launch");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    assert!(
        std::fs::write(
            rig.config(),
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
             [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok(),
        "a config"
    );
    let sock = rig.sock.clone();
    let (code, stdout, stderr) = rig.run_on(Some(&sock), &["--local", "entryone"]);
    // The ATTACH is what decides the code, and it cannot succeed here: `ae
    // <name>` always attaches, and there is no door that says otherwise — and
    // a test process has no
    // terminal to hand to tmux. The session is what this row is about, and it
    // is built before the attach is even attempted.
    assert_ne!(code, Some(2), "not a usage error: {stdout}\n{stderr}");
    assert!(
        rig.sessions().join("entryone").join("meta").exists(),
        "{stdout}"
    );
    let (ok, listed) = rig.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(
        ok && listed.lines().any(|line| line == "entryone"),
        "{listed}"
    );
}

/// A public launch can name its origin without changing the caller's cwd; all
/// three copy modes resolve from that canonical directory.
#[test]
fn a_directory_explicit_launch_uses_that_origin_in_every_mode() {
    if skip() {
        return;
    }
    let rig = Rig::idle("explicit-dir");
    let source = rig.scratch.join("source");
    assert!(std::fs::create_dir_all(&source).is_ok(), "a source dir");
    assert!(std::fs::write(source.join("marker"), "source").is_ok());
    assert!(std::fs::create_dir_all(source.join(".ae")).is_ok());
    assert!(
        std::fs::write(
            source.join(".ae/config"),
            "[workspace]\nlayout = horizontal\nwatchdog = false\n",
        )
        .is_ok()
    );
    let source_link = rig.scratch.join("source-link");
    assert!(
        std::os::unix::fs::symlink(&source, &source_link).is_ok(),
        "a logical spelling for the source"
    );
    let source = std::fs::canonicalize(&source).expect("canonical source");
    let source_arg = source_link.to_string_lossy();
    let sock = rig.sock.clone();

    let (code, stdout, stderr) = rig.run_from_on(
        &rig.project,
        Some(&sock),
        &["explicit-local", "--dir", &source_arg, "--no-attach"],
    );
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let local = std::fs::read_to_string(rig.sessions().join("explicit-local/meta"))
        .unwrap_or_else(|why| panic!("local meta: {why}"));
    assert!(
        local.contains(&format!("origin={}\n", source.display())),
        "{local}"
    );
    assert!(
        local.contains(&format!("work_dir={}\n", source.display())),
        "{local}"
    );
    assert!(local.contains("layout=horizontal\n"), "{local}");

    let (code, stdout, stderr) = rig.run_from_on(
        &rig.project,
        Some(&sock),
        &[
            "explicit-copy",
            "--copy",
            "--dir",
            &source_arg,
            "--no-attach",
        ],
    );
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let copy = std::fs::read_to_string(rig.sessions().join("explicit-copy/meta"))
        .unwrap_or_else(|why| panic!("copy meta: {why}"));
    assert!(
        copy.contains(&format!("origin={}\n", source.display())),
        "{copy}"
    );
    assert!(
        rig.home.join("worktrees/explicit-copy/marker").is_file(),
        "copy came from explicit origin"
    );

    git_in(&source, &["init", "-q"]);
    git_in(&source, &["add", "-A"]);
    git_in(&source, &["commit", "-qm", "base"]);
    let (code, stdout, stderr) = rig.run_from_on(
        &rig.project,
        Some(&sock),
        &[
            "explicit-worktree",
            "--worktree",
            "--dir",
            &source_arg,
            "--no-attach",
        ],
    );
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let worktree = std::fs::read_to_string(rig.sessions().join("explicit-worktree/meta"))
        .unwrap_or_else(|why| panic!("worktree meta: {why}"));
    assert!(
        worktree.contains(&format!("origin={}\n", source.display())),
        "{worktree}"
    );
    assert!(
        rig.home
            .join("worktrees/explicit-worktree/marker")
            .is_file(),
        "worktree came from explicit origin"
    );
}

#[test]
fn a_public_no_attach_launch_starts_and_reattaches_with_the_exact_hint() {
    if skip() {
        return;
    }
    let rig = Rig::idle("public-no-attach");
    let expected = "Session 'detached' started. Attach with: tmux -L ae attach -t \"=detached\"\n";
    let (code, stdout, stderr) = rig.run(&["detached", "--no-attach"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(stdout.ends_with(expected), "{stdout}");
    assert!(
        rig.default_tmux(&["has-session", "-t", "=detached"]).0,
        "session is running"
    );

    let (code, stdout, stderr) = rig.run(&["detached", "--no-attach"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert_eq!(
        stdout,
        "Session 'detached' is running. Attach with: tmux -L ae attach -t \"=detached\"\n"
    );
}

#[test]
fn a_directory_explicit_reattach_refuses_a_different_or_missing_origin() {
    if skip() {
        return;
    }
    let rig = Rig::idle("explicit-dir-refusal");
    let first = rig.scratch.join("first");
    let other = rig.scratch.join("other");
    assert!(std::fs::create_dir_all(&first).is_ok());
    assert!(std::fs::create_dir_all(&other).is_ok());
    let first = first.to_string_lossy();
    let other = other.to_string_lossy();
    let sock = rig.sock.clone();
    let (code, stdout, stderr) =
        rig.run_on(Some(&sock), &["owned", "--dir", &first, "--no-attach"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");

    let (code, stdout, stderr) =
        rig.run_on(Some(&sock), &["owned", "--dir", &other, "--no-attach"]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(
        stderr.contains("exists with a different origin"),
        "{stderr}"
    );

    let absent = rig.scratch.join("absent");
    let (code, stdout, stderr) = rig.run_on(
        Some(&sock),
        &["missing", "--dir", &absent.to_string_lossy(), "--no-attach"],
    );
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(
        stderr.contains("does not exist or is not a directory"),
        "{stderr}"
    );
    assert!(!rig.sessions().join("missing").exists(), "no state written");

    let (code, stdout, stderr) = rig.run_on(Some(&sock), &["missing-value", "--dir"]);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(stderr.contains("--dir requires a path"), "{stderr}");

    let (code, stdout, stderr) = rig.run_on(Some(&sock), &["unknown", "--nope"]);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(stderr.contains("unknown flag '--nope'"), "{stderr}");
}

#[test]
fn a_directory_refusal_preserves_pre_chain_meta_and_skips_config_seed() {
    if skip() {
        return;
    }
    let rig = Rig::new("explicit-dir-pre-chain");
    let first = rig.scratch.join("first");
    let other = rig.scratch.join("other");
    assert!(std::fs::create_dir_all(&first).is_ok());
    assert!(std::fs::create_dir_all(&other).is_ok());
    let first = std::fs::canonicalize(&first).expect("first origin");
    let other = std::fs::canonicalize(&other).expect("other origin");
    let dir = rig.sessions().join("owned");
    assert!(std::fs::create_dir_all(&dir).is_ok(), "session dir");
    assert!(
        rig.historical_tmux(&["new-session", "-d", "-s", "owned"]).0,
        "historical session starts"
    );
    assert!(
        rig.historical_tmux(&["set-environment", "-t", "=owned", "AE_SESSION", "1"])
            .0,
        "historical ownership marker"
    );
    assert!(
        rig.historical_tmux(&[
            "set-environment",
            "-t",
            "=owned",
            "AE_HOME",
            &rig.home.to_string_lossy(),
        ])
        .0,
        "historical ownership root"
    );
    let (ok, pane) = rig.historical_tmux(&["list-panes", "-t", "=owned", "-F", "#{pane_id}"]);
    assert!(ok, "historical pane");
    let (ok, sessions) = rig.historical_tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(
        ok && sessions.lines().any(|line| line == "owned"),
        "{sessions}"
    );
    let before = format!(
        "session=owned\norigin={}\nwork_dir={}\nmode=local\nmain_pane={}\nschema=2\n",
        first.display(),
        first.display(),
        pane.trim()
    );
    assert!(std::fs::write(dir.join("meta"), &before).is_ok(), "meta");

    let (code, stdout, stderr) = rig.run_on(
        Some(&rig.sock),
        &["owned", "--dir", &other.to_string_lossy(), "--no-attach"],
    );
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(
        stderr.contains("exists with a different origin"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read(dir.join("meta")).unwrap_or_default(),
        before.as_bytes()
    );
    assert!(!rig.config().exists(), "a refused launch seeded config");
}

#[test]
fn an_undeclared_cold_launch_uses_the_private_ae_server_and_list_finds_it_once() {
    if skip() {
        return;
    }
    let rig = Rig::idle("default-ae-server");

    let (code, stdout, stderr) = rig.run(&["--local", "coldone"]);
    assert_ne!(code, Some(2), "not a usage error: {stdout}\n{stderr}");
    let cold_meta = std::fs::read_to_string(rig.sessions().join("coldone/meta"))
        .unwrap_or_else(|why| panic!("the cold launch publishes meta: {why}"));
    assert!(cold_meta.contains("tmux_server_kind=name\n"), "{cold_meta}");
    assert!(cold_meta.contains("tmux_server=ae\n"), "{cold_meta}");

    let (ok, socket) = rig.default_tmux(&["display-message", "-p", "#{socket_path}"]);
    assert!(ok, "the named server answers: {socket}");
    let socket = socket.trim();
    let private_root =
        std::fs::canonicalize(&rig.scratch).unwrap_or_else(|_| rig.scratch.path().to_owned());
    assert!(
        Path::new(socket).starts_with(&private_root),
        "the default server escaped the private TMUX_TMPDIR: {socket}"
    );

    // Once the server is warm, the resolver records its proven socket spelling.
    let (code, stdout, stderr) = rig.run(&["--local", "warmtwo"]);
    assert_ne!(code, Some(2), "not a usage error: {stdout}\n{stderr}");
    let warm_meta = std::fs::read_to_string(rig.sessions().join("warmtwo/meta"))
        .unwrap_or_else(|why| panic!("the warm launch publishes meta: {why}"));
    assert!(
        warm_meta.contains("tmux_server_kind=socket\n"),
        "{warm_meta}"
    );
    assert!(
        warm_meta.contains(&format!("tmux_server={socket}\n")),
        "{warm_meta}"
    );

    let (code, listed, stderr) = rig.run(&["list"]);
    assert_eq!(code, Some(0), "{stderr}");
    for name in ["coldone", "warmtwo"] {
        assert_eq!(
            listed.lines().filter(|line| line.contains(name)).count(),
            1,
            "{name} should be listed once across the name/socket aliases:\n{listed}"
        );
    }
}

/// A tmux target without `=` first matches exactly, then by prefix. A longer
/// sibling must therefore never make a missing session look live.
#[test]
fn a_launch_target_is_an_exact_session_name_not_a_prefix() {
    if skip() {
        return;
    }
    let rig = Rig::idle("exact-session-target");
    assert!(
        rig.tmux(&["-f", "/dev/null", "new-session", "-d", "-s", "dotfiles2"])
            .0,
        "the longer sibling is live"
    );
    let sock = rig.sock.clone();

    // The control: the full sibling name still reaches the already-live
    // session rather than creating state for another one.
    let (code, stdout, stderr) = rig.run_on(Some(&sock), &["--local", "dotfiles2"]);
    assert_ne!(code, Some(2), "not a usage error: {stdout}\n{stderr}");
    assert!(
        !rig.sessions().join("dotfiles2").exists(),
        "an exact live session was rebuilt"
    );

    let (code, stdout, stderr) = rig.run_on(Some(&sock), &["--local", "dotfiles"]);
    assert_ne!(code, Some(2), "not a usage error: {stdout}\n{stderr}");
    assert!(
        rig.sessions().join("dotfiles").join("meta").exists(),
        "the shorter session was not built: {stdout}\n{stderr}"
    );
    let (ok, listed) = rig.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(ok, "the tmux server answers: {listed}");
    let mut names = listed.lines().collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, ["dotfiles", "dotfiles2"], "{listed}");
}

/// An EMPTY argv after the preamble is a LAUNCH, not the help an empty argv
/// gets everywhere else — `ae` with no words starts the default session and
/// always has.
#[test]
fn no_argv_at_all_launches_rather_than_printing_help() {
    if skip() {
        return;
    }
    let rig = Rig::idle("bare");
    let sock = rig.sock.clone();
    let (code, stdout, stderr) = rig.run_on(Some(&sock), &[]);
    assert_ne!(code, Some(2), "not a usage error: {stdout}\n{stderr}");
    // Help would have printed the command list and written nothing.
    assert!(!stdout.contains("Usage:"), "help was printed: {stdout}");
    assert!(rig.config().exists(), "the launch prelude did not run");
    let started: Vec<String> = std::fs::read_dir(rig.sessions())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        // The lifecycle lock is a sibling FILE in the same root, not a session.
        .filter(|name| !name.starts_with(".lifecycle."))
        .collect();
    assert_eq!(started.len(), 1, "one derived session: {started:?}");
    assert!(
        started[0].contains("project"),
        "the name is derived from the cwd door: {started:?}"
    );
}

// ---------------------------------------------------------------------------
// (3) the refusals
// ---------------------------------------------------------------------------

/// A CUT WORD CREATES NO SESSION.
#[test]
fn a_cut_word_refuses_and_creates_no_session() {
    let rig = Rig::new("cut");
    for (word, expected) in [
        ("status", "Use 'ae list'"),
        ("hub", "The orchestrator seat is 'ae orchestrator'"),
        ("transfer", "no cross-machine session sync"),
    ] {
        let (code, stdout, stderr) = rig.run(&[word]);
        assert_eq!(code, Some(2), "'{word}': {stdout}{stderr}");
        assert!(stderr.starts_with("Error: "), "'{word}': {stderr}");
        assert!(stderr.contains(expected), "'{word}': {stderr}");
        assert!(stdout.is_empty(), "'{word}' printed to stdout: {stdout}");
        assert!(
            !rig.sessions().join(word).exists(),
            "'{word}' created a session directory"
        );
        // And the prelude never ran: a refusal writes nothing on its way out.
        assert!(!rig.config().exists(), "'{word}' seeded a config");
    }
}

fn write_competing_orchestrator_configs(rig: &Rig) {
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    assert!(
        std::fs::write(
            rig.config(),
            "[profiles]\nother = \"claude\"\n\n[roster]\nlead = other\nworker = other\norchestrator = other\n\n\
             [workspace]\nmain = lead\nworkers = worker\nlayout = lead-pair\nwatchdog = false\n",
        )
        .is_ok(),
        "a global config with workers"
    );
    let local_dir = rig.project.join(".ae");
    assert!(
        std::fs::create_dir_all(&local_dir).is_ok(),
        "a local config dir"
    );
    assert!(
        std::fs::write(
            local_dir.join("config"),
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlocal = idle\nworker = idle\n\n\
             [workspace]\nmain = local\nworkers = worker\nlayout = horizontal\n",
        )
        .is_ok(),
        "a project config with workers"
    );
}

/// The bare `ae orchestrator` seeds and uses its state-local config, ignoring a
/// project overlay that would otherwise add workers. The seed is one-shot.
#[test]
fn the_bare_orchestrator_seeds_its_own_config_and_seats_exactly_one_agent() {
    if skip() {
        return;
    }
    let rig = Rig::new("orch");
    write_competing_orchestrator_configs(&rig);
    let (bin, marker) = rig.fake_profiles();
    let sock = rig.sock.clone();
    let (code, stdout, stderr) = rig.run_on_with_path(Some(&sock), &bin, &["orchestrator"]);
    // The rig has no terminal, so the attach at the end of a launch fails; the
    // build before it is what this test is about.
    assert!(
        code == Some(0) || stderr.contains("open terminal failed"),
        "{stdout}{stderr}"
    );
    assert!(
        rig.sessions().join("orchestrator").join("meta").exists(),
        "the bare word built the seat: {stdout}{stderr}"
    );
    assert!(
        stderr.contains("Created orchestrator config at") && stderr.contains("orchestrator.config"),
        "the seed is named: {stderr}"
    );
    let config = rig.home.join("orchestrator.config");
    let seeded = std::fs::read_to_string(&config)
        .unwrap_or_else(|why| panic!("{}: {why}", config.display()));
    assert!(seeded.contains("main = orchestrator"), "{seeded}");
    assert!(seeded.contains("workers = \"\""), "{seeded}");
    assert!(seeded.contains("orchestrator = true"), "{seeded}");
    assert!(seeded.contains("sweep = 120"), "{seeded}");
    assert!(seeded.contains("Use only ae commands"), "{seeded}");
    assert!(
        seeded.contains("Two plausible matches: ask one line naming both, no relay"),
        "{seeded}"
    );
    assert!(!seeded.contains("\n[profiles]\n"), "{seeded}");
    assert!(!seeded.contains("\n[roster]\n"), "{seeded}");
    let meta_path = rig.sessions().join("orchestrator").join("meta");
    let meta = std::fs::read_to_string(&meta_path)
        .unwrap_or_else(|why| panic!("{}: {why}", meta_path.display()));
    let seats = meta
        .lines()
        .filter(|line| line.starts_with("seat."))
        .collect::<Vec<_>>();
    assert_eq!(seats, ["seat.main=orchestrator"], "{meta}");
    assert!(meta.lines().any(|line| line == "meta_agent=true"), "{meta}");
    assert!(meta.lines().any(|line| line == "sweep_sec=120"), "{meta}");
    assert!(
        meta.contains(&format!("local_config={}", config.display())),
        "the nonstandard overlay is a session fact: {meta}"
    );
    assert_agent_launched(
        &marker,
        "the dedicated config's profile reached the fake executable",
    );
    let (ok, names) = rig.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(ok, "{names}");
    assert!(names.lines().any(|line| line == "orchestrator"), "{names}");

    let customized = format!("{seeded}\n# human customization survives\n");
    assert!(
        std::fs::write(&config, &customized).is_ok(),
        "customize the seeded config"
    );
    let (_, _, second_stderr) = rig.run_on_with_path(Some(&sock), &bin, &["orchestrator"]);
    assert!(
        !second_stderr.contains("Created orchestrator config at"),
        "a reattach must not reseed: {second_stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap_or_default(),
        customized,
        "a reattach preserves the human's config"
    );

    let (killed, kill_output) = rig.tmux(&["kill-session", "-t", "orchestrator"]);
    assert!(killed, "stop the live half for a resume: {kill_output}");
    assert!(
        std::fs::remove_file(&marker).is_ok(),
        "clear the first launch marker"
    );
    let (resume_code, resume_stdout, resume_stderr) =
        rig.run_on_with_path(Some(&sock), &bin, &["orchestrator"]);
    assert!(
        resume_code == Some(0) || resume_stderr.contains("open terminal failed"),
        "{resume_stdout}{resume_stderr}"
    );
    assert_agent_launched(
        &marker,
        &format!("the resumed run resolved the recorded dedicated overlay: {resume_stderr}"),
    );
    assert!(
        !resume_stderr.contains("Created orchestrator config at"),
        "a resume must not reseed: {resume_stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap_or_default(),
        customized,
        "a resume preserves the human's config"
    );
}

/// A legacy seat file may still contain identity sections, but the global
/// roster is the only profile authority. Workspace and prompt values remain
/// local overlays.
#[test]
fn the_orchestrator_ignores_legacy_seat_identity_and_uses_the_global_profile() {
    if skip() {
        return;
    }
    let rig = Rig::new("orch-legacy");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    let global = "[profiles]\nglobal = \"claude\"\n\n[roster]\norchestrator = global\n\n[workspace]\nmain = orchestrator\nwatchdog = false\n";
    assert!(
        std::fs::write(rig.config(), global).is_ok(),
        "a global config"
    );
    let seat = rig.home.join("orchestrator.config");
    let old_seat = "[profiles]\nlocal = \"codex\"\n\n[roster]\norchestrator = local\n\n[workspace]\nmain = orchestrator\nlayout = horizontal\nwatchdog = false\n";
    assert!(
        std::fs::write(&seat, old_seat).is_ok(),
        "an old seat config"
    );
    let (bin, marker) = rig.fake_profiles();
    let (code, stdout, stderr) =
        rig.run_on_with_path(Some(&rig.sock), &bin, &["orchestrator", "--no-attach"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert_eq!(
        stderr,
        format!(
            "ae orchestrator: ignoring [roster]/[profiles] in {}: the seat profile is [roster] orchestrator in {}\n",
            seat.display(),
            rig.config().display()
        )
    );
    assert_agent_launched(&marker, "the global profile reached the fake executable");
    let launched = std::fs::read_to_string(&marker).unwrap_or_default();
    assert!(
        launched.contains("claude") && !launched.contains("codex"),
        "the global profile launched, not the legacy seat profile: {launched}"
    );
    assert_eq!(
        std::fs::read_to_string(&seat).unwrap_or_default(),
        old_seat,
        "the legacy seat file remains untouched"
    );
}

/// The reserved word is special only when it uses the canonical seat overlay;
/// an explicit project-local launch named `orchestrator` keeps its roster.
#[test]
fn a_plain_local_orchestrator_name_keeps_the_project_roster() {
    if skip() {
        return;
    }
    let rig = Rig::new("orch-project");
    write_competing_orchestrator_configs(&rig);
    let (code, stdout, stderr) = rig.run_on(Some(&rig.sock), &["--local", "orchestrator"]);
    assert!(
        code != Some(2),
        "a project launch named orchestrator is not a usage error: {stdout}{stderr}"
    );
    let meta = rig.sessions().join("orchestrator").join("meta");
    let text = std::fs::read_to_string(&meta).unwrap_or_default();
    assert!(
        text.contains("seat.main=local"),
        "project roster kept: {text}"
    );
    assert!(
        text.contains("profile.main=idle"),
        "project profile kept: {text}"
    );
    assert!(!stderr.contains("ignoring [roster]/[profiles]"), "{stderr}");
}

/// A fresh home seeds the global roster first, then the seat overlay, before
/// building the one named orchestrator seat.
#[test]
fn a_fresh_orchestrator_seeds_global_then_seat_and_builds_without_attach() {
    if skip() {
        return;
    }
    let rig = Rig::new("orch-fresh");
    let (bin, marker) = rig.fake_profiles();
    let (code, stdout, stderr) =
        rig.run_on_with_path(Some(&rig.sock), &bin, &["orchestrator", "--no-attach"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let global_at = stderr
        .find("Created default config at")
        .unwrap_or_else(|| panic!("global seed missing: {stderr}"));
    let seat_at = stderr
        .find("Created orchestrator config at")
        .unwrap_or_else(|| panic!("seat seed missing: {stderr}"));
    assert!(
        global_at < seat_at,
        "global seed must precede seat seed: {stderr}"
    );
    assert!(rig.config().exists(), "global config missing");
    let seat = rig.home.join("orchestrator.config");
    let seeded = std::fs::read_to_string(&seat).unwrap_or_default();
    assert!(!seeded.contains("\n[profiles]\n"), "{seeded}");
    assert!(!seeded.contains("\n[roster]\n"), "{seeded}");
    let meta = rig.sessions().join("orchestrator").join("meta");
    let text = std::fs::read_to_string(&meta).unwrap_or_default();
    assert!(
        text.lines().any(|line| line == "seat.main=orchestrator"),
        "{text}"
    );
    assert_agent_launched(
        &marker,
        "the globally bound default profile reached the seat",
    );

    // A running seat reattaches before identity validation. Removing its
    // global row must not make the live session unreachable.
    let global = std::fs::read_to_string(rig.config()).unwrap_or_default();
    let removed_row = global
        .lines()
        .filter(|line| !line.starts_with("orchestrator = "))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert!(
        std::fs::write(rig.config(), removed_row).is_ok(),
        "remove global row"
    );
    let (code, stdout, stderr) =
        rig.run_on_with_path(Some(&rig.sock), &bin, &["orchestrator", "--no-attach"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(
        stdout.contains("Session 'orchestrator' is running."),
        "{stdout}"
    );
    assert!(!stderr.contains("no profile for the seat"), "{stderr}");
}

/// A custom global config must bind the dedicated seat before ae builds its
/// session state.
#[test]
fn the_bare_orchestrator_refuses_without_a_global_profile_row() {
    if skip() {
        return;
    }
    let rig = Rig::new("orch-missing");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    let global = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n[workspace]\nmain = lead\nwatchdog = false\n";
    assert!(
        std::fs::write(rig.config(), global).is_ok(),
        "a global config"
    );
    let seat = rig.home.join("orchestrator.config");
    let old_seat = "[profiles]\nlocal = \"sleep 600\"\n\n[roster]\norchestrator = local\n\n[workspace]\nmain = orchestrator\nwatchdog = false\n";
    assert!(
        std::fs::write(&seat, old_seat).is_ok(),
        "an old seat config"
    );
    let (code, stdout, stderr) = rig.run_on(Some(&rig.sock), &["orchestrator"]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert_eq!(
        stderr,
        format!(
            "ae orchestrator: ignoring [roster]/[profiles] in {}: the seat profile is [roster] orchestrator in {}\nae orchestrator: no profile for the seat — add \"orchestrator = <profile>\" under [roster] in {}\n",
            seat.display(),
            rig.config().display(),
            rig.config().display()
        )
    );
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(rig.config()).unwrap_or_default(),
        global
    );
    assert_eq!(std::fs::read_to_string(&seat).unwrap_or_default(), old_seat);
    assert!(!rig.sessions().join("orchestrator").exists());
}

/// An underscore word nobody serves fails CLOSED for the same reason.
#[test]
fn an_unserved_internal_word_refuses_rather_than_launching() {
    let rig = Rig::new("internal");
    for word in ["_recover-pending", "_stop-supervisor", "_nope"] {
        let (code, stdout, stderr) = rig.run(&[word, "whatever"]);
        assert_eq!(code, Some(2), "'{word}': {stdout}{stderr}");
        assert_eq!(stderr, format!("ae: unknown internal command '{word}'.\n"));
        assert!(
            !rig.sessions().join("whatever").exists(),
            "'{word}' launched"
        );
    }
}

// ---------------------------------------------------------------------------
// (4) help, rendered by the core
// ---------------------------------------------------------------------------

/// `ae help` is the core's, and it is the COMMAND SET: every word the router
/// answers appears, and no word it refuses does.
#[test]
fn help_is_the_command_set_and_names_no_retired_word() {
    let rig = Rig::new("help");
    for spelling in ["help", "-h", "--help"] {
        let (code, stdout, stderr) = rig.run(&[spelling]);
        assert_eq!(code, Some(0), "'{spelling}': {stderr}");
        assert!(stderr.is_empty(), "'{spelling}': {stderr}");
        assert_eq!(stdout, ae::entry::HELP);
    }
    for row in [
        "  ae list [",
        "  ae next [",
        "  ae brief [",
        "  ae orchestrator --popup",
        "  ae doctor [",
        "  ae rename ",
        "  ae watchdog ",
        "  ae telegram ",
        "  ae stop [",
        "  ae compact [",
        "  ae archive preview ",
        "  ae end|rm [",
        "  ae version",
        "  ae help",
    ] {
        assert!(ae::entry::HELP.contains(row), "help is missing {row}");
    }
    for retired in ["ae status", "ae hub", "ae transfer"] {
        assert!(
            !ae::entry::HELP.contains(retired),
            "help still advertises the retired {retired}"
        );
    }
}

/// `ae list --help` is the core's too — on STDERR and exit 0, as it was.
#[test]
fn list_help_is_the_ratified_filter_text_on_stderr() {
    let rig = Rig::new("listhelp");
    for spelling in ["--help", "-h"] {
        for word in ["list", "ls"] {
            let (code, stdout, stderr) = rig.run(&[word, spelling]);
            assert_eq!(code, Some(0), "'{word} {spelling}': {stderr}");
            assert!(stdout.is_empty(), "'{word} {spelling}': {stdout}");
            assert_eq!(stderr, ae::entry::LIST_HELP);
        }
    }
    // Any other tail is still the core's to parse, unknown flags included.
    let (code, _, stderr) = rig.run(&["list", "--nope"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("unknown argument: --nope"), "{stderr}");
}

// ---------------------------------------------------------------------------
// (5) the default config
// ---------------------------------------------------------------------------

/// THE ORDER IS THE CONTRACT: the config is the first write of the run, and it
/// happens only after the dependency gate has passed.
#[test]
fn the_first_run_seeds_the_config_only_after_the_dependency_check() {
    let scratch = {
        let rig = Rig::new("seed");
        // A machine with NO TMUX.
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("AE_HOME", &rig.home)
            .env("CONFIG_FILE", rig.config())
            .env("PATH", "/nonexistent")
            .current_dir(&rig.project)
            .arg("seedme")
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{stderr}");
        assert!(stderr.contains("tmux"), "{stderr}");
        assert!(
            !rig.config().exists(),
            "the config was seeded before the gate refused"
        );

        // The same launch with a PATH the gate accepts writes it, and says so on
        // STDERR — the launch's stdout belongs to the session it is about to become.
        // Fake command names keep this assertion hermetic: no installed coding
        // agent can be reached while the public template is seeded byte-for-byte.
        #[cfg(unix)]
        let (fake_bin, marker) = rig.fake_profiles();
        #[cfg(unix)]
        let (_, stdout, stderr) = rig.run_on_with_path(Some(&rig.sock), &fake_bin, &["seedme"]);
        assert!(rig.config().exists(), "{stderr}");
        assert!(
            stderr.contains(&format!(
                "Created default config at {}",
                rig.config().display()
            )),
            "{stderr}"
        );
        assert!(!stdout.contains("Created default config"), "{stdout}");

        #[cfg(unix)]
        {
            for _ in 0..200 {
                if marker.exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            assert!(
                marker.exists(),
                "the seeded profile reached a fake executable"
            );
        }

        let written = std::fs::read_to_string(rig.config()).unwrap_or_default();
        assert_eq!(written, ae::entry::DEFAULT_CONFIG);
        for section in ["[profiles]", "[roster]", "[workspace]", "[prompt]"] {
            assert!(written.contains(section), "the template lost {section}");
        }
        // No temp file is left beside it.
        let leftovers: Vec<PathBuf> = std::fs::read_dir(&rig.home)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
        rig.scratch.path().to_owned()
    };
    assert_no_tmux_processes(&scratch);
}

/// A config that is already there is never rewritten — the seeding is a first
/// run's, not every run's.
#[test]
fn an_existing_config_is_left_exactly_as_it_was() {
    let rig = Rig::new("keepconfig");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    assert!(std::fs::write(rig.config(), "# mine\n").is_ok(), "a config");
    let (_, _, stderr) = rig.run(&["keepme"]);
    assert!(!stderr.contains("Created default config"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(rig.config()).unwrap_or_default(),
        "# mine\n"
    );
}

// ---------------------------------------------------------------------------
// the path-object guard
// ---------------------------------------------------------------------------

/// A SYMLINK NAMED `valid-name` satisfies every naming rule.
#[test]
fn a_symlinked_session_directory_is_refused_before_anything_happens() {
    let rig = Rig::new("symlink");
    let victim = rig.scratch.join("victim");
    assert!(std::fs::create_dir_all(&victim).is_ok(), "a victim dir");
    assert!(
        std::fs::create_dir_all(rig.sessions()).is_ok(),
        "a sessions root"
    );
    let link = rig.sessions().join("linked");
    assert!(
        std::os::unix::fs::symlink(&victim, &link).is_ok(),
        "a symlinked session dir"
    );
    let (code, stdout, stderr) = rig.run(&["linked"]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains("is not a plain directory"), "{stderr}");
    assert!(stderr.contains("escape wearing a valid name"), "{stderr}");
    assert!(victim.exists(), "the link target was touched");
    assert!(
        std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()),
        "the link itself was replaced"
    );
}

/// A DANGLING link is the case an existence test misses: `-e` reads it as
/// absent and waves it through, so the guard is an lstat.
#[test]
fn a_dangling_symlink_is_refused_too() {
    let rig = Rig::new("dangling");
    assert!(
        std::fs::create_dir_all(rig.sessions()).is_ok(),
        "a sessions root"
    );
    let link = rig.sessions().join("gone");
    assert!(
        std::os::unix::fs::symlink(rig.scratch.join("nothing-here"), &link).is_ok(),
        "a dangling session dir"
    );
    let (code, _, stderr) = rig.run(&["gone"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("is not a plain directory"), "{stderr}");
}

/// A name the grammar itself refuses is refused AS A NAME, by the launch, so
/// the message says what is actually wrong rather than blaming the path.
#[test]
fn a_name_the_grammar_refuses_is_the_launchs_refusal_not_the_paths() {
    let rig = Rig::new("badname");
    let (code, _, stderr) = rig.run(&["../victim"]);
    assert_ne!(code, Some(0), "{stderr}");
    assert!(!stderr.contains("is not a plain directory"), "{stderr}");
    assert!(stderr.contains("invalid session name"), "{stderr}");
}

// ---------------------------------------------------------------------------
// the translated words
// ---------------------------------------------------------------------------

/// The human words reach the core entries behind them, environmental facts
/// appended.
#[test]
fn the_human_words_reach_the_core_entries_behind_them() {
    let rig = Rig::new("words");
    let cases: [(&[&str], &str); 5] = [
        (&["end"], "Usage"),
        (&["stop"], "Usage"),
        (&["rename"], "Usage"),
        (&["watchdog", "nope"], "Usage"),
        (&["archive", "nope"], "Usage: ae archive preview"),
    ];
    for (argv, expected) in cases {
        let (code, stdout, stderr) = rig.run(argv);
        assert_ne!(code, Some(0), "{argv:?}: {stdout}{stderr}");
        assert!(
            stderr.contains(expected),
            "{argv:?} did not reach its entry: {stderr}"
        );
    }
}

/// `ae doctor` names the binary answering and no interpreter: ae ships none, so
/// there is no interpreter version for it to report.
#[test]
fn doctor_names_the_binary_answering_and_no_interpreter() {
    let rig = Rig::new("doctor");
    let (_, stdout, stderr) = rig.run(&["doctor"]);
    let report = format!("{stdout}{stderr}");
    // The row that replaced it: WHICH core answered.
    assert!(report.contains("core "), "no core row:\n{report}");
    assert!(
        !report.contains("bash "),
        "ae ships no interpreter, so no row may report one:\n{report}"
    );
}

/// `ae archive preview` with a name that has no state says so, and never
/// reaches the tracer.
#[test]
fn archive_preview_refuses_a_session_with_no_state() {
    let rig = Rig::new("preview");
    let (code, stdout, stderr) = rig.run(&["archive", "preview", "ghost"]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("no session state for 'ghost'"), "{stderr}");
}

/// And with no name at all, outside a session, it says how to name one.
#[test]
fn archive_preview_outside_a_session_asks_for_a_name() {
    let rig = Rig::new("previewbare");
    let (code, _, stderr) = rig.run(&["archive", "preview"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("Usage: ae archive preview"), "{stderr}");
    assert!(stderr.contains("Run inside an ae tmux session"), "{stderr}");
}

/// The sessions root is where these names resolve — a rig helper the tests
/// above lean on, asserted once so a wrong root cannot make them vacuous.
#[test]
fn the_sessions_root_is_derived_from_the_preamble_home() {
    let rig = Rig::new("root");
    assert_eq!(rig.sessions(), Path::new(&rig.home).join("sessions"));
}

// ---------------------------------------------------------------------------
// (5) the remaining doors, each through the surface that consumes it
// ---------------------------------------------------------------------------

/// The SERVER PAIR decides which tmux server a launch lands on, and the session
/// records the one it was handed rather than asking tmux afterwards.
#[test]
fn the_server_pair_door_decides_where_a_launch_lands() {
    if skip() {
        return;
    }
    let rig = Rig::new("pairdoor");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    assert!(
        std::fs::write(
            rig.config(),
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
             [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok(),
        "a config"
    );
    let sock = rig.sock.clone();
    let (_, stdout, stderr) = rig.run_on(Some(&sock), &["--local", "pairone"]);
    let meta = rig.sessions().join("pairone").join("meta");
    let Ok(text) = std::fs::read_to_string(&meta) else {
        panic!("a meta at {}: {stdout}{stderr}", meta.display());
    };
    assert!(
        text.contains(&format!("tmux_server={}", sock.display())),
        "the pair the door declared is the pair the session records: {text}"
    );
    assert!(text.contains("tmux_server_kind=socket"), "{text}");
}

/// The CWD door: a launch with no name derives one from the working directory,
/// and `$PWD` is honoured only when it names the same directory.
#[test]
fn the_cwd_door_prefers_the_logical_pwd_only_when_it_agrees() {
    if skip() {
        return;
    }
    let rig = Rig::new("cwddoor");
    let sock = rig.sock.clone();
    let mut command = ae();
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", rig.config())
        .env("AE_NO_AUTOSTART", "1")
        .env("TMUX_TMPDIR", &rig.scratch)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &sock)
        // A PWD that names a real directory this process is NOT in.
        .env("PWD", "/")
        .current_dir(&rig.project);
    let out = command
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    let started: Vec<String> = std::fs::read_dir(rig.sessions())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with(".lifecycle."))
        .collect();
    assert_eq!(
        started.len(),
        1,
        "one derived session: {started:?} ({})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        started[0].contains("project"),
        "the stale PWD did not decide where ae thinks it is: {started:?}"
    );
}

/// The `AE_NO_AUTOSTART` door: `=1` starts no companion — the watchdog here,
/// and the Telegram bridge alike — however the config asks.
#[test]
fn the_no_autostart_door_starts_neither_companion() {
    if skip() {
        return;
    }
    let rig = Rig::new("autostart");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    assert!(
        std::fs::write(
            rig.config(),
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
             [workspace]\nmain = lead\nlayout = vertical\nwatchdog = true\n",
        )
        .is_ok(),
        "a config that ASKS for the watchdog"
    );
    let sock = rig.sock.clone();
    // `run_on` sets AE_NO_AUTOSTART=1.
    let (_, stdout, stderr) = rig.run_on(Some(&sock), &["--local", "quiet"]);
    assert!(
        rig.sessions().join("quiet").join("meta").exists(),
        "the session was built: {stdout}{stderr}"
    );
    let (ok, windows) = rig.tmux(&["list-windows", "-t", "quiet", "-F", "#{window_name}"]);
    assert!(ok, "{windows}");
    assert!(
        !windows.lines().any(|line| line.contains("watchdog")),
        "the door suppressed the companion the config asked for: {windows}"
    );
}
