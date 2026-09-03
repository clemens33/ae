//! The two daemons' LIFECYCLE, against a real tmux server.
//!
//! `_watchdog` and `_telegram` start and stop long-lived processes, and the
//! whole risk in porting them out of bash is in what the guards do with a real
//! server's answers: a `start` that cannot see a running daemon spawns a second
//! one, and a `stop` that cannot see a pane reports a kill it did not perform.
//! So both arms drive the product entries against a live server rather than a
//! double.
//!
//! No bridge is ever spawned here. The Telegram arm plants a tmux session under
//! the bridge's own name, which is exactly what the liveness check looks for —
//! proving `start`'s idempotence and `stop`'s kill without a process that would
//! long-poll a real API with a fake token.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fs;
use std::path::{Path, PathBuf};

use ae::telegram::bridge::Paths;

use super::parity::Invocation;
use super::parity::capture::raw;
use super::phase2::{run_tmux, tmux_present};

/// A scratch dir short enough to hold a socket path — `sun_path` is 104 bytes
/// on macOS and the usual temp dir eats most of it.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-dl-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(fs::create_dir_all(&dir).is_ok(), "a short scratch dir");
    dir
}

fn socket_of(scratch: &Path) -> PathBuf {
    scratch.join("s")
}

/// Kill the arm's server and remove its scratch, WHATEVER ended the arm.
///
/// A failed assertion skips every line after it, and the frozen shape of these
/// tests (kill at the end) then leaves a tmux server and its `sleep` children
/// behind for a minute — on a machine where the next timing-sensitive test is
/// already running. That cascade is how one real failure becomes three
/// mysterious ones, so the cleanup is a `Drop` rather than a last line.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        // NOT the panicking `tmux` helper: this runs while a panic may already
        // be unwinding, and a second panic there aborts the whole process —
        // taking the failure report with it.
        let out = self.scratch.join("cleanup-out");
        let err = self.scratch.join("cleanup-err");
        let invocation = Invocation::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server");
        let _ = raw::run(&invocation, &self.scratch, &out, &err);
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

fn tmux(socket: &Path, scratch: &Path, words: &[&str]) -> (bool, String) {
    let mut args = vec!["-S".to_owned(), socket.display().to_string()];
    args.extend(words.iter().map(|word| (*word).to_owned()));
    run_tmux(&args, scratch)
}

fn require_tmux(scratch: &Path) {
    if !tmux_present(scratch) {
        let _ = fs::remove_dir_all(scratch);
        panic!(
            "tmux is not runnable here, so the daemon lifecycle's real-server arms cannot be \
             proven; install tmux or run this suite where one exists"
        );
    }
}

/// Write `body` at `path` and make it executable.
fn plant_script(path: &Path, body: &str) {
    assert!(fs::write(path, body).is_ok(), "the script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).is_ok(),
            "the script must be executable"
        );
    }
}

/// A session meta dir naming `socket` as its server, with the two helpers the
/// start path runs: a `watchdog` that publishes a pidfile and then stays alive,
/// and an `events-tail` for the monitor window.
///
/// The watchdog stand-in is REAL rather than stubbed at the transport, because
/// the registration wait judges a start by a pidfile a live process published —
/// a fixture that wrote the file itself would prove the polling and nothing
/// about the contract between the pane and the starter.
fn plant_session(root: &Path, session: &str, socket: &Path) -> PathBuf {
    let meta_dir = root.join("sessions").join(session);
    assert!(fs::create_dir_all(&meta_dir).is_ok(), "a session meta dir");
    let meta = format!(
        "mode=local\nsession={session}\ntmux_server_kind=socket\ntmux_server={}\n\
         seat.main=lead\nprofile.main=cl\nagent_bin.main=claude\n",
        socket.display()
    );
    assert!(fs::write(meta_dir.join("meta"), meta).is_ok(), "the record");
    plant_script(
        &meta_dir.join("watchdog"),
        "#!/bin/sh\n\
         d=$(cd \"$(dirname \"$0\")\" && pwd)\n\
         printf '%s\\n' \"$$\" > \"$d/.watchdog.pid.staged\"\n\
         mv \"$d/.watchdog.pid.staged\" \"$d/.watchdog.pid\"\n\
         exec sleep 60\n",
    );
    plant_script(&meta_dir.join("events-tail"), "#!/bin/sh\nexec sleep 60\n");
    meta_dir
}

/// Run one product entry and return `(code, stdout, stderr)`.
fn watchdog(root: &Path, words: &[&str]) -> (u8, String, String) {
    let tail: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = match ae::watchdog_lifecycle::run(root, &tail, &mut out, &mut err) {
        Ok(code) => code,
        Err(why) => panic!("the entry writes to in-memory buffers: {why}"),
    };
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn telegram(ae_home: &Path, words: &[&str]) -> (u8, String, String) {
    let tail: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = match ae::telegram_lifecycle::run(ae_home, &tail, &mut out, &mut err) {
        Ok(code) => code,
        Err(why) => panic!("the entry writes to in-memory buffers: {why}"),
    };
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

#[test]
fn a_watchdog_starts_once_reports_its_pid_and_stops_with_its_pane() {
    let scratch = scratch("wd");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let meta_dir = plant_session(&root, "wdlife", &socket);
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "wdlife", "sleep", "60"]
        )
        .0,
        "the session the watchdog watches"
    );

    // BEFORE: nothing published, nothing running.
    let (code, out, _) = watchdog(&root, &["status", "wdlife"]);
    assert_eq!((code, out.trim()), (0, "Watchdog is not running."));

    let (code, out, err) = watchdog(&root, &["start", "wdlife"]);
    assert_eq!(code, 0, "the start failed: {err}");
    assert!(
        out.contains("Watchdog started in hidden ae-monitor window"),
        "unexpected start output: {out}"
    );
    let pid = ae::watchdog_glue::read_pid(&meta_dir).expect("the daemon published a pidfile");

    // The pane carries the stamp every later decision keys on.
    let (_, panes) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", "wdlife", "-F", "#{@ae_agent}"],
    );
    assert!(
        panes.lines().any(|line| line == "_watchdog"),
        "no stamped watchdog pane: {panes:?}"
    );

    let (code, out, _) = watchdog(&root, &["status", "wdlife"]);
    assert_eq!(
        (code, out.trim()),
        (0, format!("Watchdog is running (pid {pid}).").as_str())
    );

    // IDEMPOTENCE: a second start does not spawn a second daemon. The pid is the
    // proof — a duplicate would have published its own.
    let (code, out, _) = watchdog(&root, &["start", "wdlife"]);
    assert_eq!(code, 0);
    assert!(
        out.contains(&format!("Watchdog is already running (pid {pid}).")),
        "a second start did not defer: {out}"
    );
    assert_eq!(ae::watchdog_glue::read_pid(&meta_dir), Some(pid));

    let (code, out, err) = watchdog(&root, &["stop", "wdlife"]);
    assert_eq!(code, 0, "the stop failed: {err}");
    assert_eq!(out.trim(), "Watchdog stopped.");
    assert_eq!(
        ae::watchdog_glue::read_pid(&meta_dir),
        None,
        "the registration outlived the daemon"
    );
    let (_, panes) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", "wdlife", "-F", "#{@ae_agent}"],
    );
    assert!(
        !panes.lines().any(|line| line == "_watchdog"),
        "the watchdog pane outlived the stop: {panes:?}"
    );

    // AFTER: a second stop is not an error, and status agrees with it.
    let (code, out, _) = watchdog(&root, &["stop", "wdlife"]);
    assert_eq!((code, out.trim()), (0, "Watchdog is not running."));
    let (code, out, _) = watchdog(&root, &["status", "wdlife"]);
    assert_eq!((code, out.trim()), (0, "Watchdog is not running."));
}

#[test]
fn the_watchdog_entry_refuses_a_session_it_cannot_name_or_find() {
    let scratch = scratch("wdref");
    let root = scratch.join("home");
    assert!(fs::create_dir_all(root.join("sessions")).is_ok());

    // No name, and no pane to derive one from.
    let (code, _, err) = watchdog(&root, &["status"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("no session name given and not inside an ae tmux session"),
        "unexpected refusal: {err}"
    );

    // A name that is not a session.
    let (code, _, err) = watchdog(&root, &["status", "nope"]);
    assert_eq!(code, 1);
    assert!(err.contains("session 'nope' not found"), "{err}");

    // A name that is not a NAME. The refusal comes before any path is joined.
    let (code, _, err) = watchdog(&root, &["status", "../../etc"]);
    assert_eq!(code, 1);
    assert!(err.contains("not found"), "{err}");

    // An action that is not one.
    let (code, _, err) = watchdog(&root, &["restart", "nope"]);
    assert_eq!(code, 2, "a usage error is 2, not 1");
    assert!(err.contains("start|stop|status"), "{err}");

    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn telegram_start_is_idempotent_status_reports_it_and_stop_kills_the_bridge() {
    let scratch = scratch("tg");
    require_tmux(&scratch);
    let socket = socket_of(&scratch);
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let ae_home = scratch.join("home");
    assert!(fs::create_dir_all(&ae_home).is_ok());
    let token = ae_home.join("token");
    assert!(fs::write(&token, "123456:fake-token\n").is_ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).is_ok());
    }
    let config = ae_home.join("config");
    assert!(
        fs::write(
            &config,
            format!(
                "[workspace]\nmain = lead\n\n[telegram]\ntoken_file = {}\nchat_id = 7\ninclude = state\n",
                token.display()
            ),
        )
        .is_ok(),
        "the config"
    );
    let server = [
        "--server-kind",
        "socket",
        "--server",
        &socket.display().to_string(),
    ]
    .map(ToOwned::to_owned);

    // The bridge's own session, planted rather than spawned: the liveness check
    // is an exact name match over `list-sessions`, so this is what a running
    // bridge looks like to every one of these commands.
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "ae-telegram", "sleep", "60"]
        )
        .0,
        "the stand-in bridge session"
    );

    let mut words: Vec<&str> = vec!["start"];
    words.extend(server.iter().map(String::as_str));
    let (code, out, err) = telegram(&ae_home, &words);
    assert_eq!(code, 0, "the start failed: {err}");
    assert!(
        out.contains("already running (tmux session ae-telegram)"),
        "a start beside a live bridge did not defer: {out}"
    );
    // It still recorded the INTENT, which is what makes a later launch revive it.
    let written = fs::read_to_string(&config).expect("the config survived");
    assert!(
        ae::telegram_lifecycle::enabled_in(&written),
        "the intent was not persisted: {written}"
    );
    assert!(
        written.contains("[workspace]\nmain = lead\n"),
        "the rewrite disturbed another section: {written}"
    );

    let mut words: Vec<&str> = vec!["status"];
    words.extend(server.iter().map(String::as_str));
    let (code, out, _) = telegram(&ae_home, &words);
    assert_eq!(code, 0);
    for expected in [
        "intent:  enabled=true",
        "runtime: daemon running (tmux session ae-telegram)",
        "token:   OK",
        "include: state",
        "WARN: 'chat' not in include",
    ] {
        assert!(out.contains(expected), "status missed {expected:?}: {out}");
    }

    let mut words: Vec<&str> = vec!["stop"];
    words.extend(server.iter().map(String::as_str));
    let (code, out, err) = telegram(&ae_home, &words);
    assert_eq!(code, 0, "the stop failed: {err}");
    assert!(out.contains("ae telegram: stopped"), "{out}");
    let (_, sessions) = tmux(
        &socket,
        &scratch,
        &["list-sessions", "-F", "#{session_name}"],
    );
    assert!(
        !sessions.lines().any(|line| line == "ae-telegram"),
        "the bridge session outlived the stop: {sessions:?}"
    );
    let written = fs::read_to_string(&config).expect("the config survived");
    assert!(
        !ae::telegram_lifecycle::enabled_in(&written),
        "a stop that leaves the intent enabled is undone by the next launch: {written}"
    );

    let mut words: Vec<&str> = vec!["stop"];
    words.extend(server.iter().map(String::as_str));
    let (code, out, _) = telegram(&ae_home, &words);
    assert_eq!((code, out.trim()), (0, "ae telegram: was not running"));
}

#[test]
fn a_telegram_start_without_credentials_refuses_before_it_enables_anything() {
    let scratch = scratch("tgcred");
    let ae_home = scratch.join("home");
    assert!(fs::create_dir_all(&ae_home).is_ok());
    let config = ae_home.join("config");
    assert!(fs::write(&config, "[workspace]\nmain = lead\n").is_ok());

    let (code, _, err) = telegram(&ae_home, &["start"]);
    assert_eq!(code, 1, "a start with no credentials must fail");
    assert!(err.contains("token_file is not set"), "{err}");
    let written = fs::read_to_string(&config).expect("the config survived");
    assert_eq!(
        written, "[workspace]\nmain = lead\n",
        "a refused start must not leave enabled=true for a later autostart to act on"
    );

    // The autostart makes the same judgement, and says nothing at all when the
    // config never asked for a bridge.
    let mut err = Vec::new();
    let started = ae::telegram_lifecycle::autostart(
        &Paths::under(&ae_home),
        &ae::inventory::ServerId::Ambient,
        "some-session",
        &ae_home,
        &mut err,
    )
    .expect("the autostart writes to an in-memory buffer");
    assert!(
        !started,
        "nothing may start from a config that says nothing"
    );
    assert!(
        err.is_empty(),
        "a disabled bridge is not a warning: {}",
        String::from_utf8_lossy(&err)
    );

    let _ = fs::remove_dir_all(&scratch);
}
