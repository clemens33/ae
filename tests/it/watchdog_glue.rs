//! The watchdog PANE, against a real tmux server.
//!
//! Slice A.3 makes `helper_watchdog_main`'s `_run` a pane that execs
//! `ae-core _watchdog-run`, so the question this file answers is the only one
//! that matters about that cut: does a session whose pane runs ONLY the core
//! still behave as it did with the bash wrapper around it?
//!
//! Four arms, one per duty that had no proof before this slice:
//!
//! * a stale pane is NUDGED — the loop's own work, kept here because it is what
//!   the pane exists to do and a refactor that broke it would otherwise be
//!   caught by nothing;
//! * a pane that dropped to a shell is ALERTED;
//! * the branch pair is PUBLISHED on the session, which was the bash wrapper's
//!   per-cycle git read;
//! * the legacy reap REFUSES a foreign pane — the ownership guard, driven
//!   against a real server where a pane id genuinely belongs to someone else.
//!
//! The daemon runs on its own thread and is stopped by killing the session it
//! watches, which is its own documented self-termination. Every wait is bounded:
//! a watchdog test that can hang is a suite that can hang.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ae::inventory::ServerId;
use ae::meta::Selector;
use ae::watchdog_daemon::{Knobs, run};
use ae::watchdog_glue::{self, KillOutcome};

use super::parity::Invocation;
use super::parity::capture::ExitOutcome;
use super::parity::capture::raw;
use super::phase2::{run_tmux, tmux_present};

/// How long any arm waits for the daemon to produce its evidence.
const BUDGET: Duration = Duration::from_secs(20);

/// A scratch dir short enough to hold a socket path — `sun_path` is 104 bytes
/// on macOS and the usual temp dir eats most of it.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-wg-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(fs::create_dir_all(&dir).is_ok(), "a short scratch dir");
    dir
}

fn server_of(socket: &Path) -> ServerId {
    ServerId::Selected(Selector::Socket(socket.to_path_buf()))
}

fn tmux(socket: &Path, scratch: &Path, words: &[&str]) -> (bool, String) {
    let mut args = ae::tmux::server_args(&server_of(socket));
    args.extend(words.iter().map(|word| (*word).to_owned()));
    run_tmux(&args, scratch)
}

/// Run one `git -C <dir>` through the parity harness's raw door — the test
/// target's own, because `clippy.toml` confines `Command` to three files and a
/// fourth is a decision, not a fixture detail.
fn git(dir: &Path, scratch: &Path, words: &[&str]) -> bool {
    let mut invocation = Invocation::new("git").arg("-C").arg(dir);
    for word in words {
        invocation = invocation.arg(word);
    }
    let out = scratch.join("git-out");
    let err = scratch.join("git-err");
    raw::run(&invocation, scratch, &out, &err)
        .is_ok_and(|status| matches!(status.outcome(), ExitOutcome::Code(0)))
}

fn kill_server(socket: &Path, scratch: &Path) {
    let _ = tmux(socket, scratch, &["kill-server"]);
}

fn require_tmux(scratch: &Path) {
    if !tmux_present(scratch) {
        let _ = fs::remove_dir_all(scratch);
        panic!(
            "tmux is not runnable here, so the watchdog pane's real-server arms cannot be \
             proven; install tmux or run this suite where one exists"
        );
    }
}

/// A session meta dir naming `socket` as its server, with a `send` helper that
/// records what it was asked to deliver and succeeds.
///
/// The helper is REAL — the daemon spawns it — because a nudge's delivery is
/// judged by that process's exit status, and a fixture that stubbed the
/// transport would prove the decision without proving the delivery.
fn plant(root: &Path, session: &str, socket: &Path, work_dir: Option<&Path>) -> PathBuf {
    let meta_dir = root.join("sessions").join(session);
    assert!(fs::create_dir_all(&meta_dir).is_ok(), "a session meta dir");
    let mut meta = format!(
        "mode=local\nsession={session}\ntmux_server_kind=socket\ntmux_server={}\n\
         seat.main=lead\nprofile.main=cl\nagent_bin.main=claude\n",
        socket.display()
    );
    if let Some(work_dir) = work_dir {
        use std::fmt::Write as _;
        let _ = writeln!(meta, "work_dir={}", work_dir.display());
    }
    assert!(fs::write(meta_dir.join("meta"), meta).is_ok(), "the record");
    let send = meta_dir.join("send");
    assert!(
        fs::write(
            &send,
            "#!/bin/sh\nprintf '%s %s %s\\n' \"${AE_SENDER_OVERRIDE:-none}\" \"${_AE_EVENT_ACTION:-none}\" \"$1\" >> \"$(dirname \"$0\")/delivered\"\nexit 0\n",
        )
        .is_ok(),
        "the send helper"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            fs::set_permissions(&send, fs::Permissions::from_mode(0o755)).is_ok(),
            "the send helper must be executable"
        );
    }
    meta_dir
}

/// Knobs that make one real cycle answer the question, not one real minute.
///
/// `stale_secs = 0` is not a cheat: the composite still requires an UNCHANGED
/// hash, which only holds on the second cycle, so the nudge arm still proves the
/// cross-cycle bookkeeping rather than a first-look verdict.
fn quick() -> Knobs {
    Knobs {
        interval_secs: 1,
        stale_secs: 0,
        max_nudges: 2,
        quiet_beat_ms: 10,
        // A supervise would run the session's recorded `ae`, and this fixture
        // records none — but zero says so rather than relying on that.
        tg_supervise_secs: 0,
        ..Knobs::default()
    }
}

/// Run the daemon on its own thread until `done` holds or the budget runs out,
/// then stop it the way its own docs say it stops: by killing the session.
fn watch_until(
    meta_dir: &Path,
    socket: &Path,
    scratch: &Path,
    session: &str,
    done: impl Fn() -> bool,
) -> bool {
    let meta_dir = meta_dir.to_path_buf();
    let daemon = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&meta_dir, quick(), &mut out, &mut err);
        (code, String::from_utf8_lossy(&out).into_owned())
    });
    let deadline = Instant::now() + BUDGET;
    let mut satisfied = false;
    while Instant::now() < deadline {
        if done() {
            satisfied = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = tmux(socket, scratch, &["kill-session", "-t", session]);
    let joined = daemon.join();
    assert!(joined.is_ok(), "the daemon thread must not panic");
    if let Ok((_, banner)) = joined {
        assert!(
            banner.contains("ae watchdog — session:"),
            "the pane prints its own banner: {banner}"
        );
    }
    satisfied
}

fn events(meta_dir: &Path) -> String {
    fs::read_to_string(meta_dir.join("events.jsonl")).unwrap_or_default()
}

#[test]
fn a_stale_pane_is_nudged_by_a_pane_running_only_the_core() {
    // THE CUT'S HEADLINE. With the bash wrapper gone there is no `send` between
    // the decision and the pane but the daemon's own spawn of the session
    // helper, so this arm proves the whole path: observe, account, deliver,
    // record.
    let scratch = scratch("nudge");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let root = scratch.join("home");
    let meta_dir = plant(&root, "nudged", &socket, None);

    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "nudged", "cat"]
        )
        .0,
        "the watched session"
    );
    // `cat` holds a static pane, which is what "unchanged hash" needs.
    assert!(
        tmux(
            &socket,
            &scratch,
            &["set-option", "-p", "-t", "nudged", "@ae_agent", "lead"]
        )
        .0
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["set-option", "-p", "-t", "nudged", "@ae_slot", "main"]
        )
        .0
    );

    let delivered = meta_dir.join("delivered");
    let reached = watch_until(&meta_dir, &socket, &scratch, "nudged", || {
        fs::read_to_string(&delivered).is_ok_and(|text| text.contains("lead"))
    });
    let receipt = fs::read_to_string(&delivered).unwrap_or_default();
    kill_server(&socket, &scratch);
    assert!(reached, "the nudge must be delivered within the budget");
    // The EVENT is the send helper's to write — the daemon hands it the actor
    // and the action, and this fixture records what it was handed. Asserting
    // the envelope proves the contract the real helper reads.
    assert!(
        receipt.starts_with("watchdog nudge lead"),
        "the nudge arrives as the watchdog's, tagged as a nudge: {receipt:?}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn a_pane_that_dropped_to_a_shell_is_alerted_once() {
    let scratch = scratch("dead");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let root = scratch.join("home");
    let meta_dir = plant(&root, "died", &socket, None);

    // The pane's foreground command is a shell and no descendant is named
    // `claude` (the roster's `agent_bin.main`) — the frozen dead check exactly.
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "died", "sh"]
        )
        .0,
        "the watched session"
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["set-option", "-p", "-t", "died", "@ae_agent", "lead"]
        )
        .0
    );
    // The SLOT is what resolves `agent_bin.main=claude`; without it the
    // descendant probe is Unknown and Unknown is deliberately not dead.
    assert!(
        tmux(
            &socket,
            &scratch,
            &["set-option", "-p", "-t", "died", "@ae_slot", "main"]
        )
        .0
    );

    let reached = watch_until(&meta_dir, &socket, &scratch, "died", || {
        events(&meta_dir).contains("dropped to shell")
    });
    let log = events(&meta_dir);
    kill_server(&socket, &scratch);
    assert!(reached, "the dead pane must be alerted within the budget");
    assert_eq!(
        log.matches("dropped to shell").count(),
        1,
        "latched: one alert however many cycles ran: {log}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn the_branch_pair_is_published_on_the_session_the_daemon_watches() {
    // The bash wrapper's per-cycle git read (ae:14432-14433). Both options,
    // because they are different values for different consumers: the machine one
    // is untrimmed and undecorated, the display one carries the dirty marker.
    let scratch = scratch("branch");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let root = scratch.join("home");
    let work = scratch.join("repo");
    assert!(fs::create_dir_all(&work).is_ok(), "a work dir");
    for words in [
        vec!["init", "-q", "-b", "slice-a3"],
        vec!["config", "user.email", "t@example.invalid"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "root"],
    ] {
        assert!(git(&work, &scratch, &words), "git {words:?} must succeed");
    }
    // An uncommitted TRACKED change, so the display value must carry the marker
    // and the machine value must not.
    assert!(fs::write(work.join("f"), "one").is_ok());
    assert!(git(&work, &scratch, &["add", "f"]));

    let meta_dir = plant(&root, "branchy", &socket, Some(&work));
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "branchy", "cat"]
        )
        .0,
        "the watched session"
    );

    let read = |name: &str| {
        tmux(
            &socket,
            &scratch,
            &["show-options", "-v", "-t", "branchy", name],
        )
        .1
        .trim()
        .to_owned()
    };
    // Both readings are taken INSIDE the wait: `watch_until` stops the daemon by
    // killing the session, and a killed session has no options left to show.
    let seen = std::cell::RefCell::new((String::new(), String::new()));
    let reached = watch_until(&meta_dir, &socket, &scratch, "branchy", || {
        let machine = read("@ae_branch_name");
        if machine != "slice-a3" {
            return false;
        }
        *seen.borrow_mut() = (machine, read("@ae_branch_status"));
        true
    });
    let (machine, display) = seen.into_inner();
    kill_server(&socket, &scratch);
    assert!(reached, "the machine branch value must be published");
    assert_eq!(machine, "slice-a3", "untrimmed, undecorated");
    assert_eq!(
        display, "slice-a3*",
        "the DISPLAY value carries the dirty marker; tmux trims the leading space \
         off `show-options -v`"
    );
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn the_legacy_reap_refuses_a_pane_that_belongs_to_someone_else() {
    // A pane id is server-local and REUSED, so the reap may not act on an id
    // alone. Two arms against ONE real server: a pane in a FOREIGN session
    // (a real stale-id collision, since the ids share a numbering space) and a
    // pane in the right session carrying the WRONG stamp. Both must survive.
    let scratch = scratch("reap");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let server = server_of(&socket);
    let root = scratch.join("home");
    let meta_dir = plant(&root, "ours", &socket, None);

    assert!(tmux(&socket, &scratch, &["new-session", "-d", "-s", "ours"]).0);
    assert!(tmux(&socket, &scratch, &["new-session", "-d", "-s", "theirs"]).0);
    let foreign = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", "theirs", "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    let ours = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", "ours", "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    // Stamp OUR pane as a live agent — it is the pane a wrong reap would take.
    assert!(
        tmux(
            &socket,
            &scratch,
            &["set-option", "-p", "-t", &ours, "@ae_agent", "lead"]
        )
        .0
    );

    let mut err = Vec::new();
    let foreign_verdict =
        watchdog_glue::kill_owned_pane(&server, &foreign, "ours", Some("_shepherd"), &mut err);
    let stamp_verdict =
        watchdog_glue::kill_owned_pane(&server, &ours, "ours", Some("_shepherd"), &mut err);
    // The artifacts a reap cleans up regardless, and the pane it must NOT find.
    assert!(fs::write(meta_dir.join(".loop.pid"), "4242\n").is_ok());
    let mut reap_err = Vec::new();
    let reaped = watchdog_glue::reap_legacy(&server, "ours", &meta_dir, &mut reap_err);

    let both_alive = tmux(&socket, &scratch, &["list-panes", "-a", "-F", "#{pane_id}"]).1;
    let diagnostics = String::from_utf8_lossy(&err).into_owned();
    kill_server(&socket, &scratch);

    assert_eq!(
        foreign_verdict.ok(),
        Some(KillOutcome::WrongSession("theirs".to_owned())),
        "a pane in another session is refused by NAME, not by silence"
    );
    assert_eq!(
        stamp_verdict.ok(),
        Some(KillOutcome::WrongAgent("lead".to_owned())),
        "our own agent's pane is refused because its stamp disagrees"
    );
    assert!(
        both_alive.contains(&foreign) && both_alive.contains(&ours),
        "neither refused pane may be killed: {both_alive}"
    );
    assert!(
        diagnostics.contains("refusing to kill pane") && diagnostics.contains("stale pane id"),
        "a refusal is LOUD: {diagnostics}"
    );
    assert_eq!(
        reaped.ok(),
        Some(Vec::new()),
        "no legacy pane exists, so nothing is reported reaped"
    );
    assert!(
        fs::read_to_string(meta_dir.join(".loop.pid")).is_err(),
        "the stale artifact is cleaned up even with no pane to kill"
    );
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn the_legacy_reap_takes_the_pane_it_does_own_and_its_artifacts_with_it() {
    // The POSITIVE half. A guard that only ever refuses is a guard that has
    // stopped reaping, and the refusal arms above cannot tell the two apart.
    let scratch = scratch("reaped");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let server = server_of(&socket);
    let root = scratch.join("home");
    let meta_dir = plant(&root, "ours", &socket, None);

    assert!(tmux(&socket, &scratch, &["new-session", "-d", "-s", "ours"]).0);
    let legacy = tmux(
        &socket,
        &scratch,
        &["split-window", "-d", "-t", "ours", "-P", "-F", "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    assert!(
        tmux(
            &socket,
            &scratch,
            &["set-option", "-p", "-t", &legacy, "@ae_agent", "_shepherd"]
        )
        .0
    );
    assert!(fs::write(meta_dir.join(".shepherd.pid"), "4242\n").is_ok());
    assert!(fs::write(meta_dir.join(".shepherd.status"), "stale\n").is_ok());

    let mut err = Vec::new();
    let reaped = watchdog_glue::reap_legacy(&server, "ours", &meta_dir, &mut err);
    let panes = tmux(&socket, &scratch, &["list-panes", "-a", "-F", "#{pane_id}"]).1;
    let diagnostics = String::from_utf8_lossy(&err).into_owned();
    kill_server(&socket, &scratch);

    assert_eq!(
        reaped.ok(),
        Some(vec!["shepherd"]),
        "the reap reports which legacy watchdog it found"
    );
    assert!(
        !panes.contains(&legacy),
        "the legacy pane is gone: {panes} still holds {legacy}"
    );
    assert!(
        diagnostics.is_empty(),
        "a POSITIVE match is silent; only a refusal is loud: {diagnostics}"
    );
    for artifact in [".shepherd.pid", ".shepherd.status"] {
        assert!(
            fs::read_to_string(meta_dir.join(artifact)).is_err(),
            "{artifact} must be cleaned up"
        );
    }
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn an_unreadable_pane_is_refused_rather_than_taken_on_faith() {
    // The FAIL-CLOSED half, and the one a mock cannot exhibit: measured on a
    // real server, an unknown pane answers rc 0 with an empty session and an
    // unknown server answers rc 1. Both are "no owner named".
    let scratch = scratch("unread");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let server = server_of(&socket);
    assert!(tmux(&socket, &scratch, &["new-session", "-d", "-s", "solo"]).0);

    let mut err = Vec::new();
    let missing = watchdog_glue::kill_owned_pane(&server, "%999", "solo", Some("_loop"), &mut err);
    kill_server(&socket, &scratch);
    // The server is gone now, so this second probe cannot answer at all.
    let unreachable =
        watchdog_glue::kill_owned_pane(&server, "%0", "solo", Some("_loop"), &mut err);

    assert_eq!(missing.ok(), Some(KillOutcome::Unreadable));
    assert_eq!(unreachable.ok(), Some(KillOutcome::Unreadable));
    assert!(
        String::from_utf8_lossy(&err).is_empty(),
        "a pane that is not there is nothing to kill, and nothing to complain about"
    );
    let _ = fs::remove_dir_all(&scratch);
}

/// A writer that fails the way a closed pipe does.
struct Broken;

impl std::io::Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_daemon_that_dies_between_publish_and_watch_takes_its_pidfile_with_it() {
    // colead gate 135cf36a: `run` published the pidfile, then hit `?` on the
    // banner write (stdout a closed pipe) and left `.watchdog.pid` naming an
    // exited process — `watchdog status` then reported a daemon that was not
    // there. The release is RAII now; this arm is the failing Write that proves
    // every exit after publish releases.
    let scratch = scratch("pidfile-raii");
    require_tmux(&scratch);
    let socket = scratch.join("s");
    let root = scratch.join("home");
    let meta_dir = plant(&root, "raii", &socket, None);
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "raii", "cat"]
        )
        .0,
        "the watched session"
    );

    let mut err = Vec::new();
    let outcome = run(&meta_dir, quick(), &mut Broken, &mut err);
    let pidfile = meta_dir.join(".watchdog.pid");
    let leaked = pidfile.exists();
    kill_server(&socket, &scratch);

    assert!(
        outcome.is_err(),
        "the broken stdout must surface as the run's error"
    );
    assert!(
        !leaked,
        "the pidfile must not outlive the daemon that published it: {}",
        pidfile.display()
    );
    let _ = fs::remove_dir_all(&scratch);
}
