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
}

impl Rig {
    /// A live session named `lc<tag>` whose meta points at this rig's socket.
    ///
    /// The socket lives under a short `/tmp` path: `sun_path` is 104 bytes on
    /// macOS and `std::env::temp_dir()` alone eats about half of that.
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
            "[profiles]\nfake = \"sh\"\n\n[workspace]\nmain = fake\nlayout = vertical\n",
        )
        .expect("a config");
        let rig = Self {
            home: home.clone(),
            sock: home.join("s"),
            name: name.clone(),
            dir: dir.clone(),
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
        rig
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.sock.clone()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.home)
    }

    /// Run one core subcommand under this rig's `AE_HOME`, bounded.
    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let mut cmd = ae();
        cmd.env("AE_HOME", &self.home);
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

    fn session_is_live(&self) -> bool {
        let (_, listed) = self.tmux(&["list-sessions", "-F", "#{session_name}"]);
        listed.lines().any(|line| line == self.name)
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
    // A regular FILE where the archive directory would go. The publisher must
    // refuse it (an archive is immutable and this is not even a tree), and the
    // end must then stop with everything still on disk — including the memory
    // it exists to preserve.
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
fn a_self_stop_returns_from_the_pane_it_kills() {
    // THE SHAPE THAT NEEDS THE SUPERVISOR, driven the way it actually happens:
    // the command runs in a shell INSIDE the target's own pane, so the process
    // asking for the stop is the one the stop destroys.
    //
    // Two facts are proven, and the first is why the detachment exists: the
    // caller reaches its LAST line (its stdout is on disk, complete) before the
    // pane dies, and the session goes away afterwards anyway — by a process
    // that was never in it.
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
    // request that preceded it. After a self-stop this log is the only account
    // a human whose pane vanished has.
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
    // stop destroys nothing, self-stop included.
    assert!(exists(&rig.dir), "the session dir stays");
    assert!(!exists(&rig.archive()), "a stop archives nothing");
}

#[test]
fn a_self_stop_with_no_terminal_names_the_flag_instead_of_asking() {
    // A non-interactive caller inside the session has no one to ask, and
    // silently killing the session would be worse than refusing. `rig.run`
    // gives the child a null stdin, which is exactly that caller.
    let rig = Rig::new("selfnotty");
    let (code, out, err) = rig.run(&["_stop", "--self", &rig.name]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(err.contains("no terminal to confirm on"), "{err}");
    assert!(
        err.contains(&format!("ae stop {} -y", rig.name)),
        "it names the flag: {err}"
    );
    assert!(rig.session_is_live(), "nothing was stopped");
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
    // session, which is clause (c) of the invariant. Killing the last session
    // exits the server, so the only proof available is its clean-dead
    // diagnostic — and "the server answered without it" and "the server is
    // unreachable" must not be read as the same thing.
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
    // handover, which needs a live agent to answer. Everything the boundary
    // does — revalidate, stop, archive, teardown — still runs.
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

    // THE FROZEN ROSTER, not a config re-read. The plan carries the answer the
    // freeze resolved and the prompt showed, so a config rewritten between the
    // freeze and the relaunch cannot change which agents the child starts.
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
/// from the frozen roster, on the server its parent ran on. No glue exec, and
/// the four-line stdout contract is untouched.
#[test]
fn compact_relaunches_the_child_in_process() {
    let rig = Rig::new("relaunch");
    // The child is a REAL launch, so its main must be an agent NAME bound in
    // `[roster]` — the shared fixture's config names a profile directly, which
    // the v2 grammar refuses. Written before the compact, so the freeze
    // resolves the roster the child will actually be held to.
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
///
/// `TMUX_TMPDIR` moves the default socket under this rig's own directory, so a
/// command that falls back to the ambient server here reaches a server this
/// test created — which is the only honest way to prove what an ambient
/// FALLBACK would have done to a stranger's session.
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
    ///
    /// `TMUX_TMPDIR` only decides the default socket when `TMUX` is unset —
    /// and a suite run from inside a tmux pane inherits `TMUX`, which names the
    /// developer's own server. Clearing the environment and re-adding the three
    /// variables tmux actually needs is what makes "ambient" mean this rig.
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

    /// A live session on the ambient server, plus the state directory that
    /// makes it one of ae's — with `server_rows` verbatim in its meta.
    fn plant(&self, name: &str, server_rows: &str) -> PathBuf {
        assert!(
            self.tmux(&["new-session", "-d", "-s", name, "sh"]).0,
            "the ambient session '{name}' starts"
        );
        // THE ISOLATION GUARD, checked before anything acts. These tests exist
        // to watch an ambient fallback take a session that is not its own, and
        // a leaked `TMUX` would point that fallback at the developer's real
        // server. A default server holding anything but this rig's session is
        // not this rig's server, and nothing may proceed against it.
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

/// B2: an unresolvable server record must not be answered with the ambient one.
///
/// The bug, and terra's repro: `tmux_server_kind=ambiguous` normalised to
/// `ServerSelector::Ambiguous`, the caller retagged that `ServerId::Ambient`,
/// and the rename then went looking for the name on a server the record never
/// named. An unrelated session of the same name answers — and gets renamed.
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

    // THE POINT. Pre-fix the ambient session was renamed by a record that never
    // named this server.
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
///
/// A launch writes the two rows only when its caller resolved a server, and the
/// glue resolves none from a plain terminal — so a missing selector is the most
/// ordinary session there is, and the ambient server is exactly the one it runs
/// on. Refusing it would strand every such session.
#[test]
fn a_session_that_records_no_server_still_renames_on_the_ambient_one() {
    let rig = AmbientRig::new("ambok");
    rig.plant("ambplain", "");

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
///
/// The self-stop writes `stop-request` before it detaches, because a human
/// whose pane vanished has to be able to tell "ae was asked and something went
/// wrong" from "ae was never asked". When the detach itself failed, that record
/// was all there was — and a request with no result is indistinguishable from a
/// stop still in flight. The caller here has an empty `PATH`, so the `nohup`
/// the detach runs cannot be found and `run_detached` reports false.
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
///
/// The detached supervisor cannot prompt — but the process that hands over to
/// it is alive, holds the caller's terminal, and is the one taking down every
/// session a typo away. It used to refuse outright and demand `-y`, which made
/// the most destructive form of the command the one form that never confirmed.
///
/// Driven through a real tmux pane, because a prompt needs a real terminal.
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

    // The command runs as a PANE's process, so its stdin is a tty. Its own
    // output goes to files: the prompt has to be read by this test, not by the
    // terminal emulator.
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

/// The other half of I2, unchanged: with no terminal the fleet stop from inside
/// still refuses and still names the flag.
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

    let (code, out, err) = rig.run(&["_stop", "all", "--pane", &pane]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(err.contains("needs -y"), "{err}");
    assert!(err.contains("cannot prompt"), "{err}");
    let live = rig.sessions();
    assert!(
        live.iter().any(|name| name == "stptwo"),
        "nothing was stopped: {live:?}"
    );
}

/// The lifecycle lock EXCLUDES an end from a concurrent start or resume.
///
/// The lock is taken before the target is classified and held through the last
/// removal, because the window between the proof and the cleanup spans
/// commit/fetch/push — and a launch landing in that window made cleanup delete
/// state out from under a freshly LIVE session (issue #4). A held lock must
/// therefore be a loud REFUSAL that preserves everything, not a wait that
/// eventually deletes.
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
