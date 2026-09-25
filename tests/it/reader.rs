//! The pane reader against a REAL tmux server.
//!
//! A unit argv pin cannot hold tmux's pane geometry, copy mode or mouse
//! routing. This arm opens the reader over a live source, proves the keyboard
//! stays with the source while the wheel scrolls the reader, and proves the
//! toggle, the retarget and the source-death close. The wheel fixtures install
//! the no-select command the launch asserts; its byte-exact argv is pinned
//! beside the code in `src/session_tmux.rs`, so a drift in that pin is where
//! the spelling breaks first.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::cli::Runner;
use super::cli::ae;
use super::phase2::{run_tmux, tmux_present};

/// A tmux client under load can take seconds to attach and render; these polls
/// await an external process, they are not speed assertions.
const PATIENCE: Duration = Duration::from_mins(1);

/// A scratch dir short enough to hold a socket path (`sun_path`).
fn scratch(tag: &str) -> PathBuf {
    super::cli::OwnedScratch::root("reader", tag).keep()
}

/// Kill the arm's server and remove its scratch whatever ended the arm.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let listed = std::slice::from_ref(&self.socket);
        if super::parity::capture::raw::kill_servers_under(&self.scratch, listed) {
            let _ = fs::remove_dir_all(&self.scratch);
        }
    }
}

/// One tmux call on the arm's server, from the arm's own directory.
fn tmux(socket: &Path, dir: &Path, words: &[&str]) -> (bool, String) {
    let _ = fs::create_dir_all(dir);
    let mut args = vec!["-S".to_owned(), socket.display().to_string()];
    args.extend(words.iter().map(|word| (*word).to_owned()));
    run_tmux(&args, dir)
}

/// Poll `read` until it answers something `settled` accepts.
fn wait_for(
    what: &str,
    mut read: impl FnMut() -> String,
    settled: impl Fn(&str) -> bool,
) -> String {
    let deadline = Instant::now() + PATIENCE;
    let mut last = String::new();
    while Instant::now() < deadline {
        last = read();
        if settled(&last) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("{what} never settled; tmux last said {last:?}");
}

/// The reader verb, resolved through the caller PANE (`$TMUX_PANE`).
fn reader_from_pane(socket: &Path, pane: &str) -> Runner {
    let mut runner = ae();
    runner
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", socket)
        .env("TMUX", format!("{},1,0", socket.display()))
        .env("TMUX_PANE", pane);
    runner.arg("_reader");
    runner
}

/// The reader verb, resolved through the CLIENT the binding captured.
fn reader_from_client(socket: &Path, client: &str) -> Runner {
    let mut runner = ae();
    runner
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", socket)
        .env("TMUX", format!("{},1,0", socket.display()));
    runner.arg("_reader").arg("--client").arg(client);
    runner
}

/// `pane_id|top|active|mode|reader_src`, one per pane of the session, in order.
fn panes(socket: &Path, dir: &Path) -> Vec<(String, i64, String, String, String)> {
    let (ok, out) = tmux(
        socket,
        dir,
        &[
            "list-panes",
            "-t",
            "s",
            "-F",
            "#{pane_id}|#{pane_top}|#{pane_active}|#{pane_in_mode}|#{@ae_reader_src}",
        ],
    );
    assert!(ok, "list-panes: {out}");
    out.lines()
        .filter_map(|line| {
            let mut fields = line.split('|');
            Some((
                fields.next()?.to_owned(),
                fields.next()?.parse().ok()?,
                fields.next()?.to_owned(),
                fields.next()?.to_owned(),
                fields.next().unwrap_or_default().to_owned(),
            ))
        })
        .collect()
}

/// The pane marked as the window's reader, if any.
fn reader_pane(socket: &Path, dir: &Path) -> Option<(String, i64, String, String, String)> {
    let mut readers = panes(socket, dir)
        .into_iter()
        .filter(|row| !row.4.is_empty());
    let first = readers.next()?;
    assert!(
        readers.next().is_none(),
        "one reader per window, never two; tmux says {:?}",
        panes(socket, dir)
    );
    Some(first)
}

/// The session, its source pane and a producer that keeps the pane busy and
/// then serves typed lines on its own tty.
fn stage_source(socket: &Path, dir: &Path) -> String {
    assert!(
        tmux(
            socket,
            dir,
            &["new-session", "-d", "-s", "s", "-x", "100", "-y", "30"]
        )
        .0
    );
    let (ok, out) = tmux(socket, dir, &["list-panes", "-t", "s", "-F", "#{pane_id}"]);
    assert!(ok, "the source pane: {out}");
    let source = out.trim().to_owned();
    assert!(
        tmux(
            socket,
            dir,
            &[
                "respawn-pane",
                "-k",
                "-t",
                &source,
                "/bin/sh",
                "-c",
                "seq 1 400; echo READY; while read l; do echo GOT:$l; done",
            ],
        )
        .0,
        "the source producer"
    );
    source
}

/// A real nested CLIENT: a second session whose pane runs an attach to `s`, so
/// the terminal input the arm injects is a client's own input.
fn stage_client(socket: &Path, dir: &Path) -> (String, String) {
    let attach = format!("env -u TMUX tmux -u -S {} attach -t s", socket.display());
    assert!(
        tmux(
            socket,
            dir,
            &[
                "new-session",
                "-d",
                "-s",
                "viewer",
                "-x",
                "100",
                "-y",
                "30",
                &attach,
            ],
        )
        .0,
        "the nested client"
    );
    let pane = wait_for(
        "the viewer pane",
        || {
            tmux(
                socket,
                dir,
                &["display-message", "-p", "-t", "viewer", "#{pane_id}"],
            )
            .1
        },
        |out| out.trim().starts_with('%'),
    )
    .trim()
    .to_owned();
    let client = wait_for(
        "an attached client",
        || {
            tmux(
                socket,
                dir,
                &["list-clients", "-F", "#{client_name}|#{client_session}"],
            )
            .1
        },
        |out| out.lines().any(|line| line.ends_with("|s")),
    )
    .lines()
    .find(|line| line.ends_with("|s"))
    .map(|line| line.split('|').next().unwrap_or_default().to_owned())
    .unwrap_or_default();
    assert!(client.starts_with('/'), "a client tty name: {client}");
    (pane, client)
}

/// Install the no-select mode-table wheel map the launch asserts. Same command
/// the unit pin in `src/session_tmux.rs` fixes.
fn install_wheel_map(socket: &Path, dir: &Path) {
    for (table, key, direction) in [
        ("copy-mode", "WheelUpPane", "scroll-up"),
        ("copy-mode", "WheelDownPane", "scroll-down"),
        ("copy-mode-vi", "WheelUpPane", "scroll-up"),
        ("copy-mode-vi", "WheelDownPane", "scroll-down"),
    ] {
        assert!(
            tmux(
                socket,
                dir,
                &[
                    "bind-key",
                    "-T",
                    table,
                    key,
                    "send-keys",
                    "-X",
                    "-N",
                    "5",
                    direction,
                ],
            )
            .0,
            "the {table} {key} map"
        );
    }
}

#[test]
fn the_toggle_opens_one_reader_above_the_source_and_a_second_call_closes_it() {
    let scratch = scratch("toggle");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the reader toggle cannot be proven");
    }
    let socket = scratch.join("sock");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let source = stage_source(&socket, &scratch);

    let out = reader_from_pane(&socket, &source)
        .output()
        .expect("the ae binary should run");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stderr}");

    let rows = panes(&socket, &scratch);
    assert_eq!(rows.len(), 2, "the source plus one reader: {rows:?}");
    let (_reader, top, active, mode, marked) =
        reader_pane(&socket, &scratch).expect("a reader pane");
    let (_, source_top, source_active, _, _) = rows
        .iter()
        .find(|row| row.0 == source)
        .expect("the source pane");
    assert_eq!(marked, source, "the reader names its source");
    assert!(top < *source_top, "the reader sits above its source");
    assert_eq!(mode, "1", "the reader shows the history in copy mode");
    assert_eq!(active, "0", "the reader never takes the keyboard");
    assert_eq!(source_active, "1", "the source keeps the keyboard");

    let out = reader_from_pane(&socket, &source)
        .output()
        .expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(0), "the second toggle closes");
    assert!(
        reader_pane(&socket, &scratch).is_none(),
        "the reader is gone"
    );
    assert_eq!(
        panes(&socket, &scratch).len(),
        1,
        "the source alone remains"
    );
}

#[test]
fn a_reader_retargets_one_per_window_and_never_stacks() {
    let scratch = scratch("retarget");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the reader toggle cannot be proven");
    }
    let socket = scratch.join("sock");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let source = stage_source(&socket, &scratch);
    let (ok, out) = tmux(
        &socket,
        &scratch,
        &[
            "split-window",
            "-v",
            "-d",
            "-t",
            &source,
            "-P",
            "-F",
            "#{pane_id}",
        ],
    );
    assert!(ok, "the second source: {out}");
    let second = out.trim().to_owned();

    for (index, target) in [&source, &second].iter().enumerate() {
        let out = reader_from_pane(&socket, target)
            .output()
            .expect("the ae binary should run");
        assert_eq!(
            out.status.code(),
            Some(0),
            "open {index}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let (_, _, _, mode, marked) =
            reader_pane(&socket, &scratch).expect("exactly one reader in the window");
        assert_eq!(mode, "1");
        assert_eq!(&marked, *target, "the reader follows the calling source");
    }
    assert_eq!(
        panes(&socket, &scratch).len(),
        3,
        "source, second, one reader"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real client rig proves the wheel and the typing together, which is the whole ask"
)]
fn the_wheel_scrolls_the_reader_and_typing_reaches_the_source() {
    let scratch = scratch("wheel");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the reader wheel cannot be proven");
    }
    let socket = scratch.join("sock");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let source = stage_source(&socket, &scratch);
    let (viewer, client) = stage_client(&socket, &scratch);
    install_wheel_map(&socket, &scratch);
    assert!(
        tmux(&socket, &scratch, &["set-option", "-t", "s", "mouse", "on"]).0,
        "mouse on, as a launch sets it"
    );
    wait_for(
        "the source producer to reach its read loop",
        || tmux(&socket, &scratch, &["capture-pane", "-p", "-t", &source]).1,
        |out| out.contains("READY"),
    );

    let out = reader_from_client(&socket, &client)
        .output()
        .expect("the ae binary should run");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (reader, _, _, mode, _) = reader_pane(&socket, &scratch).expect("a reader pane");
    assert_eq!(mode, "1");
    let (top, left, height, width) = {
        let (ok, out) = tmux(
            &socket,
            &scratch,
            &[
                "list-panes",
                "-t",
                "s",
                "-F",
                "#{pane_id}|#{pane_top}|#{pane_left}|#{pane_height}|#{pane_width}",
            ],
        );
        assert!(ok, "reader geometry: {out}");
        let fields: Vec<i64> = out
            .lines()
            .find(|line| line.starts_with(&format!("{reader}|")))
            .expect("the reader's geometry")
            .split('|')
            .skip(1)
            .map(|field| field.parse().unwrap_or_default())
            .collect();
        (fields[0], fields[1], fields[2], fields[3])
    };
    let x = left + (width / 2) + 1;
    let y = top + (height / 2) + 1;
    let wheel = format!("\u{1b}[<64;{x};{y}M");
    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", &viewer, "-l", &wheel]
        )
        .0,
        "the wheel over the reader"
    );
    wait_for(
        "the reader to scroll",
        || {
            tmux(
                &socket,
                &scratch,
                &["display-message", "-p", "-t", &reader, "#{scroll_position}"],
            )
            .1
        },
        |out| out.trim().parse::<i64>().is_ok_and(|scroll| scroll > 0),
    );
    let (_, _, source_active, _, _) = panes(&socket, &scratch)
        .into_iter()
        .find(|row| row.0 == source)
        .expect("the source pane");
    assert_eq!(source_active, "1", "the wheel never moves the keyboard");

    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", &viewer, "-l", "hello"]
        )
        .0,
        "typing into the client"
    );
    assert!(
        tmux(&socket, &scratch, &["send-keys", "-t", &viewer, "Enter"]).0,
        "the submit key"
    );
    wait_for(
        "the source to read the typed line",
        || tmux(&socket, &scratch, &["capture-pane", "-p", "-t", &source]).1,
        |out| out.contains("GOT:hello"),
    );
    let (_, _, _, mode, _) = reader_pane(&socket, &scratch).expect("the reader still open");
    assert_eq!(mode, "1", "the reader survived the typing");
}

#[test]
fn a_dead_source_leaves_a_reader_the_toggle_still_closes() {
    let scratch = scratch("dead");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the dead-source close cannot be proven");
    }
    let socket = scratch.join("sock");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let source = stage_source(&socket, &scratch);
    let out = reader_from_pane(&socket, &source)
        .output()
        .expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(0));
    let (reader, _, _, mode, _) = reader_pane(&socket, &scratch).expect("a reader pane");
    assert_eq!(mode, "1");

    assert!(
        tmux(&socket, &scratch, &["kill-pane", "-t", &source]).0,
        "the source dies"
    );
    let (_, _, _, mode, marked) =
        reader_pane(&socket, &scratch).expect("the frozen reader survives its source");
    assert_eq!(mode, "1", "the snapshot is still shown");
    assert_eq!(marked, source, "its stamp is inert data, not a lookup");

    let out = reader_from_pane(&socket, &reader)
        .output()
        .expect("the ae binary should run");
    assert_eq!(
        out.status.code(),
        Some(0),
        "the toggle closes the window's reader: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (listed, _) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-t", "s", "-F", "#{pane_id}"],
    );
    assert!(
        !listed,
        "the reader was the last pane, so its window and session are gone with it"
    );
}

#[test]
fn doctor_names_the_mode_table_wheel_bindings_it_expects() {
    let scratch = scratch("doctor");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the doctor row cannot be proven");
    }
    let socket = scratch.join("sock");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let _source = stage_source(&socket, &scratch);

    let out = ae()
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &socket)
        .arg("doctor")
        .output()
        .expect("the ae binary should run");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("copy-mode WheelUpPane"),
        "a server whose copy-mode wheels are still stock NAMES the entry: {stdout}"
    );
    assert!(
        stdout.contains("copy-mode-vi WheelUpPane"),
        "the vi table is judged too, or a one-table defect passes: {stdout}"
    );

    install_wheel_map(&socket, &scratch);
    let out = ae()
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &socket)
        .arg("doctor")
        .output()
        .expect("the ae binary should run");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        !stdout.contains("copy-mode WheelUpPane") && !stdout.contains("copy-mode-vi WheelUpPane"),
        "the asserted map clears both mode-table entries: {stdout}"
    );
}
