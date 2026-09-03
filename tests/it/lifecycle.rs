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
