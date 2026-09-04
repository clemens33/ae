//! `doctor`, `doctor --refresh`, `_check-deps`, `_shims-render` and `rename` —
//! against the real binary, real directories, and (where it must be) a real
//! tmux server.
//!
//! The report is unit-tested as a pure function of its facts; what these prove
//! is the part a function cannot: that the binary reads a real state root, that
//! a refresh republishes artifacts a pane can actually exec, and that a rename
//! moves the tmux session, the directory and the meta together.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::path::PathBuf;

use super::cli::{ae, helper};
use super::phase2::run_tmux;

/// One isolated ae home with its own tmux socket.
struct Rig {
    scratch: PathBuf,
    home: PathBuf,
    sock: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let scratch = PathBuf::from(format!("/tmp/aedoc.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(std::fs::create_dir_all(&scratch).is_ok(), "a scratch dir");
        let home = scratch.join("aehome");
        assert!(
            std::fs::create_dir_all(home.join("sessions")).is_ok(),
            "a sessions root"
        );
        let config = scratch.join("config");
        // `/bin/sh` is the one launch command every platform this runs on has,
        // so the profile row resolves without installing a fake.
        assert!(
            std::fs::write(
                &config,
                "[profiles]\nsh = \"/bin/sh -c :\"\n\n[roster]\nlead = sh\n\n[workspace]\nmain = lead\nworkers = \n",
            )
            .is_ok(),
            "a config"
        );
        Self {
            sock: scratch.join("sock"),
            scratch,
            home,
            config,
        }
    }

    /// A session directory with a meta that records THIS rig's tmux socket.
    fn session(&self, name: &str, extra: &str) -> PathBuf {
        let dir = self.home.join("sessions").join(name);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let meta = format!(
            "schema=2\nsession={name}\nmode=local\norigin={origin}\nwork_dir={origin}\nlayout=vertical\nmain_pane=%0\ntmux_server={sock}\ntmux_server_kind=socket\nconfig={config}\n{extra}",
            origin = self.scratch.display(),
            sock = self.sock.display(),
            config = self.config.display(),
        );
        assert!(std::fs::write(dir.join("meta"), meta).is_ok(), "a meta");
        dir
    }

    /// Run the product binary against this rig's home.
    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let out = ae()
            .env("AE_HOME", &self.home)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args(args)
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
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

/// Guard: the tmux-dependent halves prove nothing without a real tmux.
fn skip() -> bool {
    let probe = PathBuf::from(format!("/tmp/aedoc-probe.{}", std::process::id()));
    let _ = std::fs::create_dir_all(&probe);
    let present = super::phase2::tmux_present(&probe);
    let _ = std::fs::remove_dir_all(&probe);
    !present
}

/// Which level a label carries in a rendered report.
fn level_of(report: &str, label: &str) -> Option<String> {
    report.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let level = fields.next()?;
        (fields.next()? == label).then(|| level.to_owned())
    })
}

#[test]
fn doctor_reads_the_real_state_root_and_reports_a_stopped_session_as_an_orphan() {
    let rig = Rig::new("report");
    rig.session("parked", "");
    let (code, stdout, stderr) = rig.run(&[
        ae::cli::DOCTOR,
        "--global",
        &rig.config.display().to_string(),
    ]);
    assert_eq!(code, Some(0), "a clean install exits zero: {stderr}");
    assert!(stdout.starts_with("ae doctor\n\n"), "{stdout}");
    assert_eq!(
        level_of(&stdout, "config").as_deref(),
        Some("OK"),
        "the config parsed: {stdout}"
    );
    assert_eq!(
        level_of(&stdout, "workspace.main").as_deref(),
        Some("OK"),
        "{stdout}"
    );
    assert_eq!(
        level_of(&stdout, "agent:sh").as_deref(),
        Some("OK"),
        "the profile's executable resolved: {stdout}"
    );
    // The session is on disk and not running anywhere, which is the one thing
    // no running-scoped sensor can see.
    assert!(stdout.contains("no running session: parked"), "{stdout}");
    assert!(stdout.contains("orphans-hint"), "{stdout}");
    // Its meta names no core, so the pin row fires too.
    assert!(
        stdout.contains("session parked has no core bound"),
        "{stdout}"
    );
    // Six warnings, and the count is pinned so a new row shows up here:
    // local-config, workspace.workers, orphans + its hint, core-pin + its hint.
    assert!(stdout.ends_with("failure(s), 6 warning(s)\n"), "{stdout}");
    // The two roots are CREATED by the report, as the frozen `mkdir -p` did.
    assert!(rig.home.join("worktrees").is_dir(), "the worktrees root");
}

#[test]
fn doctor_fails_and_exits_one_when_the_config_is_missing() {
    let rig = Rig::new("noconfig");
    let (code, stdout, _) = rig.run(&[
        ae::cli::DOCTOR,
        "--global",
        &rig.scratch.join("absent").display().to_string(),
    ]);
    assert_eq!(code, Some(1), "{stdout}");
    assert_eq!(
        level_of(&stdout, "config").as_deref(),
        Some("FAIL"),
        "{stdout}"
    );
}

#[test]
fn doctor_refuses_an_unknown_flag_as_a_usage_error() {
    let rig = Rig::new("badflag");
    let (code, stdout, stderr) = rig.run(&[ae::cli::DOCTOR, "--frobnicate"]);
    assert_eq!(code, Some(2), "usage errors exit two: {stdout}");
    assert!(stderr.contains("Usage: ae doctor"), "{stderr}");
    assert!(stdout.is_empty(), "nothing was reported: {stdout}");
}

#[test]
fn doctor_refresh_republishes_the_shims_the_manifest_and_the_core_pin() {
    let rig = Rig::new("refresh");
    let dir = rig.session("stale", "ae_core=/nonexistent\nae_core_version=1999.1.1\n");
    // A helper from a previous version, to prove the set is rewritten and not
    // merely topped up.
    assert!(std::fs::write(dir.join("send"), "#!/bin/sh\nexit 7\n").is_ok());

    let (code, stdout, stderr) = rig.run(&[
        ae::cli::DOCTOR,
        "--refresh",
        "stale",
        "--global",
        &rig.config.display().to_string(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("refresh:stale") && stdout.contains("refreshed session helpers"),
        "{stdout}"
    );

    // The WHOLE helper set, and every one of them a link to the core doing the
    // refresh — including the one that was a stale regular file.
    let refresher = std::fs::canonicalize(env!("CARGO_BIN_EXE_ae")).unwrap_or_default();
    for helper in ae::shim::HELPERS {
        let path = dir.join(helper.name);
        let kind = std::fs::symlink_metadata(&path)
            .unwrap_or_else(|why| panic!("{} was not published ({why})", helper.name));
        assert!(kind.file_type().is_symlink(), "{} is a link", helper.name);
        let target = std::fs::read_link(&path).unwrap_or_default();
        assert_eq!(
            std::fs::canonicalize(&target).unwrap_or_default(),
            refresher,
            "{} points at the refreshing core",
            helper.name
        );
    }

    // The pin now names the binary that did the refresh, at its version.
    let meta = std::fs::read_to_string(dir.join("meta")).unwrap_or_default();
    assert!(
        meta.contains(&format!("ae_core_version={}", ae::VERSION)),
        "{meta}"
    );
    assert!(
        meta.contains(&format!("ae_version={}", ae::VERSION)),
        "{meta}"
    );
    assert!(!meta.contains("ae_core=/nonexistent"), "{meta}");
    // And the manifest is back, naming the session.
    let manifest = std::fs::read_to_string(dir.join("workspace.md")).unwrap_or_default();
    assert!(manifest.contains("Session: stale"), "{manifest}");
}

#[test]
fn doctor_refresh_names_a_session_it_cannot_find_and_fails() {
    let rig = Rig::new("refreshmiss");
    let (code, stdout, _) = rig.run(&[
        ae::cli::DOCTOR,
        "--refresh",
        "ghost",
        "--global",
        &rig.config.display().to_string(),
    ]);
    assert_eq!(code, Some(1), "{stdout}");
    assert!(stdout.contains("session 'ghost' not found"), "{stdout}");
}

#[test]
fn shims_render_publishes_a_set_a_pane_can_exec() {
    let rig = Rig::new("shims");
    let dir = rig.session("shimmed", "");
    let (code, _, stderr) = rig.run(&[ae::cli::SHIMS_RENDER, &dir.display().to_string()]);
    assert_eq!(code, Some(0), "{stderr}");

    // RUN one: the whole point of a shim is that a pane execs it by path. A
    // word `state` does not know is the cheapest proof that the CORE parsed the
    // argv — the shim itself parses nothing.
    let out = helper(&dir.join("state"))
        .env_remove("TMUX_PANE")
        .arg("not-a-state")
        .output()
        .unwrap_or_else(|why| panic!("the shim should run: {why}"));
    assert_eq!(
        out.status.code(),
        Some(2),
        "the core answered with its usage"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Usage: state"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn shims_render_refuses_a_directory_that_is_not_there() {
    let rig = Rig::new("shimsmiss");
    let (code, _, stderr) = rig.run(&[
        ae::cli::SHIMS_RENDER,
        &rig.scratch.join("absent").display().to_string(),
    ]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("no session directory"), "{stderr}");
}

#[test]
fn check_deps_refuses_an_old_bash_and_passes_a_current_one() {
    let rig = Rig::new("deps");
    let (code, _, stderr) = rig.run(&[ae::cli::CHECK_DEPS, "--bash-major", "3"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("bash >= 4.0"), "{stderr}");

    if skip() {
        return;
    }
    let (code, _, stderr) = rig.run(&[ae::cli::CHECK_DEPS, "--bash-major", "5"]);
    assert_eq!(code, Some(0), "tmux is present: {stderr}");
}

#[test]
fn rename_moves_the_tmux_session_the_directory_and_the_meta_together() {
    if skip() {
        return;
    }
    let rig = Rig::new("rename");
    let dir = rig.session("before", "");
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "before"]).0,
        "a live session to rename"
    );

    let (code, stdout, stderr) = rig.run(&[ae::cli::RENAME, "before", "after"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("Renamed 'before' → 'after'"), "{stdout}");

    let (_, names) = rig.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(names.lines().any(|line| line == "after"), "{names}");
    assert!(!names.lines().any(|line| line == "before"), "{names}");

    let moved = rig.home.join("sessions").join("after");
    assert!(moved.is_dir(), "the state directory moved");
    assert!(!dir.exists(), "the old directory is gone");
    let meta = std::fs::read_to_string(moved.join("meta")).unwrap_or_default();
    assert!(meta.contains("session=after\n"), "{meta}");
    // The manifest carries the name, so it is re-rendered under it.
    let manifest = std::fs::read_to_string(moved.join("workspace.md")).unwrap_or_default();
    assert!(manifest.contains("Session: after"), "{manifest}");
    // The status bar carries it too.
    // `show-options` does not take the `=` exact-match target form (measured):
    // it answers "no such session". The plain name is the only spelling here.
    let (_, left) = rig.tmux(&["show-options", "-v", "-t", "after", "status-left"]);
    assert!(left.contains("[ae after]"), "{left}");
}

#[test]
fn rename_refuses_a_target_that_is_already_live_and_moves_nothing() {
    if skip() {
        return;
    }
    let rig = Rig::new("renametaken");
    rig.session("src", "");
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "src"]).0,
        "the source"
    );
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "dst"]).0,
        "the target"
    );

    let (code, stdout, stderr) = rig.run(&[ae::cli::RENAME, "src", "dst"]);
    assert_eq!(code, Some(1), "{stdout}");
    assert!(stderr.contains("session 'dst' already exists"), "{stderr}");
    let (_, names) = rig.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(names.lines().any(|line| line == "src"), "{names}");
    assert!(
        rig.home.join("sessions").join("src").is_dir(),
        "the source directory stayed"
    );
}

#[test]
fn rename_refuses_a_source_that_is_not_running() {
    let rig = Rig::new("renamedead");
    rig.session("parked", "");
    let (code, _, stderr) = rig.run(&[ae::cli::RENAME, "parked", "elsewhere"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("is not running"), "{stderr}");
    assert!(
        !rig.home.join("sessions").join("elsewhere").exists(),
        "nothing was created"
    );
}

/// The `--refresh all` sweep, which is what a human runs after an upgrade.
#[test]
fn doctor_refresh_all_visits_every_session_it_can_find() {
    let rig = Rig::new("refreshall");
    for name in ["one", "two"] {
        rig.session(name, "");
    }
    let (code, stdout, stderr) = rig.run(&[
        ae::cli::DOCTOR,
        "--refresh",
        "--global",
        &rig.config.display().to_string(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    for name in ["one", "two"] {
        assert!(stdout.contains(&format!("refresh:{name}")), "{stdout}");
        assert!(
            rig.home.join("sessions").join(name).join("send").is_file(),
            "{name} got its shims"
        );
    }
}

/// A state root with nothing in it is a WARN, not a failure.
#[test]
fn doctor_refresh_with_no_sessions_says_so_without_failing() {
    let rig = Rig::new("refreshempty");
    let (code, stdout, _) = rig.run(&[
        ae::cli::DOCTOR,
        "--refresh",
        "--global",
        &rig.config.display().to_string(),
    ]);
    assert_eq!(code, Some(0), "{stdout}");
    assert!(stdout.contains("no existing sessions found in"), "{stdout}");
}

/// The report's own path facts must be the root the caller named, not a
/// hard-coded home.
#[test]
fn the_report_names_the_state_root_it_actually_read() {
    let rig = Rig::new("roots");
    let (_, stdout, _) = rig.run(&[
        ae::cli::DOCTOR,
        "--global",
        &rig.config.display().to_string(),
    ]);
    let sessions = rig.home.join("sessions").display().to_string();
    assert!(stdout.contains(&sessions), "{stdout}");
}
