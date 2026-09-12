//! The fleet picker against a REAL tmux server.
//!
//! A pure argv assertion cannot hold tmux's format timing or client focus. This
//! arm draws the menu, chooses rows, and proves both the ordinary lead-pane jump
//! and the execution-time guard when that pane moves or vanishes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use ae::inventory::ServerId;
use ae::meta::Selector;
use ae::tmux::{PickerPane, PickerSession, display_menu_for_client_args};

use super::cli::{OwnedChild, ae, helper_by_name};
use super::phase2::{run_tmux, tmux_present};

/// How long a poll waits for tmux to catch up before the arm fails.
const PATIENCE: Duration = Duration::from_secs(10);

/// A scratch dir short enough to hold a socket path — `sun_path` is 104 bytes
/// on macOS and the usual temp dir eats most of it.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-menu-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(fs::create_dir_all(&dir).is_ok(), "a short scratch dir");
    dir
}

/// Kill the arm's server and remove its scratch WHATEVER ended the arm — a
/// failed assertion included, so one failure leaves no server behind.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let bin = self.scratch.join("cleanup");
        let _ = fs::create_dir_all(&bin);
        let _ = run_tmux(
            &[
                "-S".to_owned(),
                self.socket.display().to_string(),
                "kill-server".to_owned(),
            ],
            &bin,
        );
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

/// One tmux call on the arm's server, from its own directory so two threads
/// never write each other's capture files.
fn tmux(socket: &Path, dir: &Path, words: &[&str]) -> (bool, String) {
    let _ = fs::create_dir_all(dir);
    let mut args = vec!["-S".to_owned(), socket.display().to_string()];
    args.extend(words.iter().map(|word| (*word).to_owned()));
    run_tmux(&args, dir)
}

/// Whether the real server under test supports mouse-driven menus.
fn menu_mouse(socket: &Path) -> bool {
    ae::transport::observe_tmux_floor(&ServerId::Selected(Selector::Socket(socket.to_path_buf())))
        .menu_mouse()
}

/// Whether a capture contains the stable fleet-picker title stem.
fn picker_is_open(text: &str) -> bool {
    text.contains("ae session —")
}

/// Poll `read` until it answers something `settled` accepts, or fail saying
/// what it last answered.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MenuGeometry {
    left: usize,
    right: usize,
}

#[derive(Debug)]
struct DirectMenu {
    geometry: MenuGeometry,
    raw: Vec<u8>,
}

#[derive(Debug)]
struct MenuTitle {
    row: usize,
    left: usize,
    text: String,
}

/// Read the direct terminal's ANSI cursor positions for the menu whose title
/// carries `needle`, rather than an outer tmux pane capture which can clip a
/// nested client's menu at a pane border. The settings menu and the quota
/// dialog share the parser; only the title needle differs.
fn direct_menu_geometry(bytes: &[u8], needle: &str) -> Option<MenuGeometry> {
    let mut index = 0;
    let mut row = 0;
    let mut column = 0;
    let mut title: Option<MenuTitle> = None;
    while index < bytes.len() {
        match bytes[index] {
            b'\x1b' => {
                index = consume_terminal_escape(bytes, index, &mut row, &mut column);
            }
            b'\r' => {
                column = 0;
                index += 1;
            }
            b'\n' => {
                row = row.saturating_add(1);
                index += 1;
            }
            byte if byte.is_ascii_control() => index += 1,
            _ => {
                let Some((character, length)) = terminal_character(&bytes[index..]) else {
                    index += 1;
                    continue;
                };
                if matches!(character, '╭' | '┌') {
                    title = Some(MenuTitle {
                        row,
                        left: column,
                        text: character.to_string(),
                    });
                } else if let Some(current) = title.as_mut() {
                    if current.row == row {
                        current.text.push(character);
                        if matches!(character, '╮' | '┐') && current.text.contains(needle) {
                            return Some(MenuGeometry {
                                left: current.left,
                                right: column,
                            });
                        }
                    } else {
                        title = None;
                    }
                }
                column = column.saturating_add(1);
                index += length;
            }
        }
    }
    None
}

fn direct_settings_geometry(bytes: &[u8]) -> Option<MenuGeometry> {
    direct_menu_geometry(bytes, "settings")
}

fn direct_dialog_geometry(bytes: &[u8]) -> Option<MenuGeometry> {
    direct_menu_geometry(bytes, "quota for our clients")
}

fn terminal_character(bytes: &[u8]) -> Option<(char, usize)> {
    let width = match *bytes.first()? {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let text = std::str::from_utf8(bytes.get(..width)?).ok()?;
    let character = text.chars().next()?;
    Some((character, character.len_utf8()))
}

fn consume_terminal_escape(
    bytes: &[u8],
    start: usize,
    row: &mut usize,
    column: &mut usize,
) -> usize {
    let Some(kind) = bytes.get(start + 1) else {
        return bytes.len();
    };
    match kind {
        b'[' => consume_csi(bytes, start, row, column),
        b']' => consume_osc(bytes, start),
        _ => start.saturating_add(3).min(bytes.len()),
    }
}

fn consume_csi(bytes: &[u8], start: usize, row: &mut usize, column: &mut usize) -> usize {
    let mut end = start.saturating_add(2);
    while let Some(byte) = bytes.get(end) {
        if (0x40..=0x7e).contains(byte) {
            break;
        }
        end += 1;
    }
    let Some(command) = bytes.get(end) else {
        return bytes.len();
    };
    let values = csi_values(&bytes[start + 2..end]);
    match *command {
        b'H' | b'f' => {
            *row = csi_value(&values, 0).saturating_sub(1);
            *column = csi_value(&values, 1).saturating_sub(1);
        }
        b'G' | b'`' => *column = csi_value(&values, 0).saturating_sub(1),
        b'd' => *row = csi_value(&values, 0).saturating_sub(1),
        b'A' => *row = row.saturating_sub(csi_value(&values, 0)),
        b'B' => *row = row.saturating_add(csi_value(&values, 0)),
        b'C' => *column = column.saturating_add(csi_value(&values, 0)),
        b'D' => *column = column.saturating_sub(csi_value(&values, 0)),
        _ => {}
    }
    end.saturating_add(1)
}

fn consume_osc(bytes: &[u8], start: usize) -> usize {
    let mut index = start.saturating_add(2);
    while let Some(byte) = bytes.get(index) {
        if *byte == b'\x07' {
            return index.saturating_add(1);
        }
        if *byte == b'\x1b' && bytes.get(index + 1) == Some(&b'\\') {
            return index.saturating_add(2);
        }
        index += 1;
    }
    bytes.len()
}

fn csi_values(bytes: &[u8]) -> Vec<usize> {
    bytes
        .split(|byte| *byte == b';')
        .map(|field| {
            let digits =
                field
                    .iter()
                    .copied()
                    .filter(u8::is_ascii_digit)
                    .fold(0_usize, |value, digit| {
                        value
                            .saturating_mul(10)
                            .saturating_add(usize::from(digit - b'0'))
                    });
            if digits == 0 { 1 } else { digits }
        })
        .collect()
}

fn csi_value(values: &[usize], index: usize) -> usize {
    values.get(index).copied().unwrap_or(1)
}

#[test]
fn direct_settings_geometry_survives_every_partial_terminal_tail() {
    let mut record = b"\x1b[30;26H".to_vec();
    record.extend_from_slice("╭─ae settings─╮".as_bytes());
    let expected = MenuGeometry {
        left: 25,
        right: 39,
    };
    assert_eq!(direct_settings_geometry(&record), Some(expected));

    let suffixes: [(&str, &[u8]); 5] = [
        ("ASCII", b"A"),
        ("two-byte scalar", "¢".as_bytes()),
        ("three-byte scalar", "╯".as_bytes()),
        ("four-byte scalar", "😀".as_bytes()),
        ("incomplete CSI", b"\x1b[38;2;"),
    ];
    let mut failures = Vec::new();
    for (kind, suffix) in suffixes {
        for end in 1..=suffix.len() {
            let mut partial = record.clone();
            partial.extend_from_slice(&suffix[..end]);
            if direct_settings_geometry(&partial) != Some(expected) {
                failures.push(format!("{kind}: {suffix:?} through byte {end}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "terminal tails lost geometry: {failures:?}"
    );
}

/// The two sessions the arm needs, a real client watching one of them, and the
/// ids of the panes the picker will target.
struct Staged {
    ids: Vec<String>,
    hub_id: String,
    client: String,
    other_client: String,
    home_pane: String,
}

#[allow(clippy::too_many_lines, reason = "one real two-client fixture")]
fn stage(socket: &Path, main: &Path) -> Staged {
    // The session the picker jumps INTO, and the one the client starts in, so
    // "it switched" is observable rather than assumed.
    for words in [
        &["new-session", "-d", "-s", "hub", "-x", "80", "-y", "24"][..],
        &["split-window", "-t", "hub"][..],
        &["new-session", "-d", "-s", "home", "-x", "80", "-y", "24"][..],
    ] {
        assert!(tmux(socket, main, words).0, "setting up: {words:?}");
    }
    // A real CLIENT, whose terminal is another pane on the same server: a menu
    // is drawn on a client, and there is no client without a terminal.
    let attach = format!("env -u TMUX tmux -S {} attach -t home", socket.display());
    for viewer in ["viewer", "other-viewer"] {
        assert!(
            tmux(
                socket,
                main,
                &[
                    "new-session",
                    "-d",
                    "-s",
                    viewer,
                    "-x",
                    "140",
                    "-y",
                    "40",
                    &attach,
                ],
            )
            .0,
            "the nested client in {viewer}"
        );
    }
    let clients = wait_for(
        "two clients on one pane",
        || {
            tmux(
                socket,
                main,
                &[
                    "list-clients",
                    "-F",
                    "#{client_name}|#{client_session}|#{pane_id}",
                ],
            )
            .1
        },
        |seen| seen.lines().filter(|line| line.contains("|home|")).count() == 2,
    );
    let client_for = |viewer: &str| {
        tmux(
            socket,
            main,
            &["display-message", "-p", "-t", viewer, "#{pane_tty}"],
        )
        .1
        .trim()
        .to_owned()
    };
    let client = client_for("viewer");
    let other_client = client_for("other-viewer");
    assert!(
        clients
            .lines()
            .any(|line| line.starts_with(&format!("{client}|home|"))),
        "{clients}"
    );
    assert!(
        clients
            .lines()
            .any(|line| line.starts_with(&format!("{other_client}|home|"))),
        "{clients}"
    );
    let home_pane = clients
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{client}|home|")))
        .unwrap_or_else(|| panic!("the clicked client row: {clients}"))
        .to_owned();
    assert!(
        clients.contains(&format!("{other_client}|home|{home_pane}")),
        "both clients must watch the SAME pane: {clients}"
    );

    let panes = tmux(
        socket,
        main,
        &["list-panes", "-t", "hub", "-F", "#{pane_id}"],
    )
    .1;
    let ids: Vec<String> = panes.lines().map(|line| line.trim().to_owned()).collect();
    assert_eq!(ids.len(), 2, "two panes in hub: {panes:?}");
    let hub_id = tmux(
        socket,
        main,
        &["display-message", "-p", "-t", "hub", "#{session_id}"],
    )
    .1
    .trim()
    .to_owned();
    assert!(hub_id.starts_with('$'), "{hub_id:?}");

    Staged {
        ids,
        hub_id,
        client,
        other_client,
        home_pane,
    }
}

/// The menu the picker builds, as the argv that draws it on `socket`.
fn picker_argv(socket: &Path, staged: &Staged) -> Vec<String> {
    let sessions = [PickerSession {
        name: "hub".to_owned(),
        id: staged.hub_id.clone(),
        rank: 0,
        glyph: "·".to_owned(),
        main_pane: staged.ids[0].clone(),
        branch: "menu-fix".to_owned(),
        agents: format!(
            "v1;{};60;lead:fable5:working:{};builder:gpt56sol:done:{};gone:gpt56luna:dead:",
            ae::time::Timestamp::now().epoch(),
            staged.ids[0],
            staged.ids[1],
        ),
        spend: format!(
            "v1;{};300;12340000;exact",
            ae::time::Timestamp::now().epoch()
        ),
        goal: "100% of #{everything} | don't stop".to_owned(),
    }];
    let panes = staged
        .ids
        .iter()
        .map(|pane| PickerPane {
            session_id: staged.hub_id.clone(),
            pane: pane.clone(),
        })
        .collect::<Vec<_>>();
    let menu = ae::orchestrator::menu_for_client(
        &sessions,
        &panes,
        true,
        &ae::theme::Palette::DARCULA,
        Some(&staged.client),
    );
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    display_menu_for_client_args(
        &server,
        Some(&staged.client),
        &menu,
        ae::transport::observe_tmux_floor(&server).menu_mouse(),
    )
}

#[test]
fn agent_rows_draw_and_live_or_missing_rows_take_the_guarded_destination() {
    let scratch = scratch("agent-rows");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so agent-row navigation cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let main = scratch.join("main");
    let watcher = scratch.join("watcher");
    let staged = stage(&socket, &main);

    let builder_argv = picker_argv(&socket, &staged);
    let drawn = std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            let seen = wait_for(
                "agent rows",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                |seen| picker_is_open(seen) && seen.contains("builder") && seen.contains("gone"),
            );
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "Home"]).0);
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "Down"]).0);
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "Down"]).0);
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "Enter"]).0);
            seen
        });
        let (succeeded, _) = run_tmux(&builder_argv, &main);
        assert!(succeeded, "tmux refused agent-row menu");
        driver.join().expect("agent key driver")
    });
    assert!(drawn.contains("● lead"), "frozen working mark: {drawn}");
    let landed = wait_for(
        "builder pane",
        || {
            tmux(
                &socket,
                &main,
                &[
                    "list-clients",
                    "-F",
                    "#{client_name}|#{client_session}|#{pane_id}",
                ],
            )
            .1
        },
        |seen| seen.contains(&format!("{}|hub|{}", staged.client, staged.ids[1])),
    );
    assert!(landed.contains(&format!("{}|hub|{}", staged.client, staged.ids[1])));

    assert!(
        tmux(
            &socket,
            &main,
            &["switch-client", "-c", &staged.client, "-t", "home"]
        )
        .0
    );
    let missing_argv = picker_argv(&socket, &staged);
    std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            wait_for(
                "missing agent row",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                |seen| picker_is_open(seen) && seen.contains("gone"),
            );
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "Home"]).0);
            for _ in 0..3 {
                assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "Down"]).0);
            }
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "Enter"]).0);
        });
        let (succeeded, _) = run_tmux(&missing_argv, &main);
        assert!(succeeded, "tmux refused missing-row menu");
        driver.join().expect("missing-row key driver");
    });
    let landed = wait_for(
        "missing row session switch",
        || {
            tmux(
                &socket,
                &main,
                &[
                    "list-clients",
                    "-F",
                    "#{client_name}|#{client_session}|#{pane_id}",
                ],
            )
            .1
        },
        |seen| seen.contains(&format!("{}|hub|{}", staged.client, staged.ids[1])),
    );
    assert!(
        landed.contains(&format!("{}|hub|{}", staged.client, staged.ids[1])),
        "empty pane hint switches session without changing its active pane: {landed}"
    );
}

#[test]
fn a_seven_line_client_draws_only_its_current_sessions_agents() {
    let scratch = scratch("short-agents");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so height degradation cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let main = scratch.join("main");
    let watcher = scratch.join("watcher");
    let staged = stage(&socket, &main);
    assert!(
        tmux(
            &socket,
            &main,
            &["resize-window", "-t", "viewer", "-x", "100", "-y", "7"]
        )
        .0
    );
    let server = ServerId::Selected(Selector::Socket(socket.clone()));
    let snapshot = wait_for(
        "seven-line client snapshot",
        || {
            ae::transport::observe_picker_client_session(&server, &staged.client)
                .map(|client| format!("{}|{}|{}", client.session_id, client.height, client.width))
                .unwrap_or_default()
        },
        |seen| seen.split('|').nth(1) == Some("7"),
    );
    let mut snapshot_fields = snapshot.split('|');
    let home_id = snapshot_fields.next().unwrap_or_default().to_owned();
    let height = snapshot_fields
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default();
    let width = snapshot_fields
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default();
    let now = ae::time::Timestamp::now().epoch();
    let session = |name: &str, id: &str, prefix: &str| PickerSession {
        name: name.to_owned(),
        id: id.to_owned(),
        rank: 0,
        glyph: "·".to_owned(),
        main_pane: String::new(),
        branch: String::new(),
        agents: format!(
            "v1;{now};60;{prefix}0:p:working:;{prefix}1:p:working:;{prefix}2:p:working:"
        ),
        spend: String::new(),
        goal: String::new(),
    };
    let sessions = [
        session("hub", &staged.hub_id, "hidden"),
        session("home", &home_id, "shown"),
    ];
    let menu = ae::orchestrator::menu_for_client_session_in(
        &sessions,
        &[],
        true,
        &ae::theme::Palette::DARCULA,
        Some(&staged.client),
        Some(&home_id),
        ae::orchestrator::PickerBounds {
            height,
            width,
            now_epoch: now,
        },
    )
    .expect("seven rows is the accepted boundary");
    assert_eq!(menu.items.len(), 5, "five items plus two border rows");
    let argv = display_menu_for_client_args(&server, Some(&staged.client), &menu, false);
    let drawn = std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            let seen = wait_for(
                "short-client menu",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                |seen| picker_is_open(seen) && seen.contains("shown2"),
            );
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "q"]).0);
            seen
        });
        let (succeeded, _) = run_tmux(&argv, &main);
        assert!(succeeded, "exact fit must draw");
        driver.join().expect("short client driver")
    });
    assert!(drawn.contains("3 agents, 3 working"), "{drawn}");
    assert!(
        drawn.contains("shown0") && drawn.contains("shown2"),
        "{drawn}"
    );
    assert!(
        !drawn.contains("hidden0"),
        "other session stays collapsed: {drawn}"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real watchdog lifecycle proves whole-fact replacement"
)]
fn watchdog_replaces_the_agent_fact_across_spawn_and_retire_then_unsets_it_on_stop() {
    let scratch = scratch("agent-fact-lifecycle");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the watchdog fact lifecycle cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_watchdog_picker_config(&project, &config, &scratch);
    let session = "factlife";
    launch_ae_session(&socket, &scratch, &root, &project, &config, session);
    let meta_dir = root.join("sessions").join(session);
    let main_pane = tmux(
        &socket,
        &scratch,
        &[
            "show-options",
            "-qv",
            "-t",
            session,
            ae::theme::MAIN_PANE_OPTION,
        ],
    )
    .1
    .trim()
    .to_owned();
    assert!(
        main_pane.starts_with('%'),
        "launched main pane: {main_pane:?}"
    );

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = ae::watchdog_lifecycle::run(
        &root,
        &[
            "start".to_owned(),
            session.to_owned(),
            "--".to_owned(),
            "--interval".to_owned(),
            "1".to_owned(),
            "--quiet-beat-ms".to_owned(),
            "10".to_owned(),
            "--tg-supervise-secs".to_owned(),
            "0".to_owned(),
        ],
        &mut out,
        &mut err,
    )
    .expect("watchdog start writes to buffers");
    assert_eq!(code, 0, "watchdog start: {}", String::from_utf8_lossy(&err));

    let read_fact = || {
        tmux(
            &socket,
            &scratch,
            &[
                "show-options",
                "-qv",
                "-t",
                session,
                ae::theme::AGENTS_OPTION,
            ],
        )
        .1
        .trim()
        .to_owned()
    };
    let initial = wait_for("initial agent fact", read_fact, |fact| {
        ae::tmux::parse_picker_agents(fact, ae::time::Timestamp::now().epoch())
            .is_some_and(|agents| agents.len() == 1 && agents[0].name == "lead")
    });
    assert!(initial.contains(";lead:idle:"), "initial fact: {initial}");
    assert_eq!(
        initial.split(';').nth(2),
        Some("1"),
        "fact carries this daemon's one-second cadence"
    );

    let spawned = ae()
        .env("HOME", &scratch)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", &config)
        .env("TMUX", format!("{},fixture,0", socket.display()))
        .env("TMUX_PANE", &main_pane)
        .arg(ae::cli::SPAWN)
        .arg(&meta_dir)
        .args(["builder", "--using", "idle"])
        .output()
        .expect("spawn runs");
    assert_eq!(
        spawned.status.code(),
        Some(0),
        "spawn: stdout={} stderr={}",
        String::from_utf8_lossy(&spawned.stdout),
        String::from_utf8_lossy(&spawned.stderr)
    );
    let with_builder = wait_for("spawned agent fact", read_fact, |fact| {
        ae::tmux::parse_picker_agents(fact, ae::time::Timestamp::now().epoch()).is_some_and(
            |agents| agents.len() == 2 && agents[0].name == "lead" && agents[1].name == "builder",
        )
    });
    assert_eq!(
        with_builder.matches(";builder:").count(),
        1,
        "{with_builder}"
    );

    let retired = ae()
        .env("HOME", &scratch)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", &config)
        .env("TMUX", format!("{},fixture,0", socket.display()))
        .env("TMUX_PANE", &main_pane)
        .arg(ae::cli::RETIRE)
        .arg(&meta_dir)
        .arg("builder")
        .output()
        .expect("retire runs");
    assert_eq!(
        retired.status.code(),
        Some(0),
        "retire: stdout={} stderr={}",
        String::from_utf8_lossy(&retired.stdout),
        String::from_utf8_lossy(&retired.stderr)
    );
    let after_retire = wait_for("retired agent fact", read_fact, |fact| {
        ae::tmux::parse_picker_agents(fact, ae::time::Timestamp::now().epoch())
            .is_some_and(|agents| agents.len() == 1 && agents[0].name == "lead")
    });
    assert!(
        !after_retire.contains("builder"),
        "replaced fact: {after_retire}"
    );

    out.clear();
    err.clear();
    let code = ae::watchdog_lifecycle::run(
        &root,
        &["stop".to_owned(), session.to_owned()],
        &mut out,
        &mut err,
    )
    .expect("watchdog stop writes to buffers");
    assert_eq!(code, 0, "watchdog stop: {}", String::from_utf8_lossy(&err));
    assert!(read_fact().is_empty(), "watchdog stop unsets @ae_agents");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real watchdog lifecycle proves the publish, the cadence and the retraction"
)]
fn the_watchdog_publishes_a_spend_fact_at_its_quota_cadence_and_unsets_it_on_stop() {
    let scratch = scratch("spend-fact");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the spend fact lifecycle cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    // One second against a two-second verdict cycle: the cadence fires on the
    // first cycle instead of in five minutes, and the request is DELIBERATELY
    // unequal to what whole cycles can deliver, so the published interval proves
    // which of the two the fact advertises.
    write_watchdog_picker_config_with(&project, &config, &scratch, "quota_every_secs = 1\n");
    let session = "spendlife";
    launch_ae_session(&socket, &scratch, &root, &project, &config, session);
    // A launched session derives its global config from its state root, the way
    // an installed ae does; the launch flag is a test convenience.
    assert!(fs::create_dir_all(&root).is_ok());
    assert!(
        fs::copy(&config, root.join("config")).is_ok(),
        "the daemon reads prices from <root>/config"
    );

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = ae::watchdog_lifecycle::run(
        &root,
        &[
            "start".to_owned(),
            session.to_owned(),
            "--".to_owned(),
            "--interval".to_owned(),
            "2".to_owned(),
            "--quiet-beat-ms".to_owned(),
            "10".to_owned(),
            "--tg-supervise-secs".to_owned(),
            "0".to_owned(),
        ],
        &mut out,
        &mut err,
    )
    .expect("watchdog start writes to buffers");
    assert_eq!(code, 0, "watchdog start: {}", String::from_utf8_lossy(&err));

    let read_fact = || {
        tmux(
            &socket,
            &scratch,
            &[
                "show-options",
                "-qv",
                "-t",
                session,
                ae::theme::SPEND_OPTION,
            ],
        )
        .1
        .trim()
        .to_owned()
    };
    let published = wait_for("the spend fact", read_fact, |fact| {
        ae::tmux::parse_picker_spend(fact, ae::time::Timestamp::now().epoch()).is_some()
    });
    assert_eq!(
        published.split(';').nth(2),
        Some("2"),
        "the fact advertises the cadence whole cycles ACHIEVE, not the one the \
         config requested — a reader expires it after two of these: {published}"
    );
    let parsed = ae::tmux::parse_picker_spend(&published, ae::time::Timestamp::now().epoch())
        .expect("the waited-for fact still parses");
    assert_eq!(parsed.usd_micro, 0, "{published}");
    assert!(
        parsed.confidence.uncertain(),
        "a seat whose transcript ae cannot locate is never an exact zero: {published}"
    );
    assert_eq!(published.split(';').nth(4), Some("partial"), "{published}");

    out.clear();
    err.clear();
    let code = ae::watchdog_lifecycle::run(
        &root,
        &["stop".to_owned(), session.to_owned()],
        &mut out,
        &mut err,
    )
    .expect("watchdog stop writes to buffers");
    assert_eq!(code, 0, "watchdog stop: {}", String::from_utf8_lossy(&err));
    assert!(read_fact().is_empty(), "watchdog stop unsets @ae_spend");
}

#[test]
fn the_menu_ae_builds_draws_on_a_real_server_and_its_rows_land_the_client() {
    let scratch = scratch("draw");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!(
            "tmux is not runnable here, so the picker's tmux-side claims cannot be proven; \
             install tmux or run this suite where one exists"
        );
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let main = scratch.join("main");
    let watcher = scratch.join("watcher");

    let staged = stage(&socket, &main);
    let argv = picker_argv(&socket, &staged);

    // `display-menu` holds its client until the menu closes, so the keys come
    // from a second thread while this one waits on tmux.
    let drawn = std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            let seen = wait_for(
                "the menu",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                picker_is_open,
            );
            // One row, one action: the session drops straight into its lead.
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "1"]).0);
            seen
        });
        let (succeeded, _) = run_tmux(&argv, &main);
        assert!(succeeded, "tmux refused the argv ae builds: {argv:?}");
        driver.join().expect("the key driver")
    });

    // What tmux DREW, which is the half no argv assertion can hold.
    assert!(
        drawn.contains("100% of #{everything}"),
        "one hash and one percent, as measured: {drawn}"
    );
    assert!(
        drawn.contains("ae session — 1 running · 0 need you · $12.34 — prefix a"),
        "the drawn picker title carries the stable stem and the fleet's spend: {drawn}"
    );
    assert!(
        drawn.contains("  $12.34 100% of"),
        "the row draws its spend right-aligned before the goal: {drawn}"
    );
    // …and where the row LANDED the client.
    let landed = wait_for(
        "the jump",
        || {
            tmux(
                &socket,
                &main,
                &[
                    "list-clients",
                    "-F",
                    "#{client_name}|#{client_session}|#{pane_id}",
                ],
            )
            .1
        },
        |seen| seen.contains("hub|"),
    );
    assert!(
        landed.contains(&format!("{}|hub|{}", staged.client, staged.ids[0])),
        "the client should sit in the lead pane {}: {landed:?}",
        staged.ids[0]
    );
    assert!(
        landed.contains(&format!(
            "{}|home|{}",
            staged.other_client, staged.home_pane
        )),
        "the other client watching the same pane must stay untouched: {landed:?}"
    );
}

#[derive(Clone, Copy)]
enum StaleLead {
    Moved,
    Vanished,
}

/// Open a row while its build-time-proven lead becomes stale. The row must
/// still switch to the captured session id, but must never follow that stale
/// pane into another session.
#[allow(
    clippy::too_many_lines,
    reason = "one open-mutate-choose race with all observable postconditions"
)]
fn stale_lead_row_lands_in_session(tag: &str, stale: StaleLead, focus_hook: bool) {
    let scratch = scratch(tag);
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the guarded picker row cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let main = scratch.join("main");
    let watcher = scratch.join("watcher");
    let staged = stage(&socket, &main);
    let argv = picker_argv(&socket, &staged);

    let foreign_view = if matches!(stale, StaleLead::Moved) {
        assert!(
            tmux(
                &socket,
                &main,
                &["new-session", "-d", "-s", "foreign", "-x", "80", "-y", "24"]
            )
            .0
        );
        assert!(
            tmux(&socket, &main, &["new-window", "-d", "-t", "foreign"]).0,
            "a second foreign window"
        );
        Some(
            tmux(
                &socket,
                &main,
                &[
                    "display-message",
                    "-p",
                    "-t",
                    "foreign",
                    "#{window_id}|#{pane_id}",
                ],
            )
            .1
            .trim()
            .to_owned(),
        )
    } else {
        None
    };

    if focus_hook {
        let hook = format!(
            "if-shell -F -t {} \"#{{==:#{{session_id}},{}}}\" \
             \"select-window -t {} ; select-pane -t {}\"",
            staged.ids[0], staged.hub_id, staged.ids[0], staged.ids[0]
        );
        assert!(
            tmux(
                &socket,
                &main,
                &[
                    "set-hook",
                    "-t",
                    &staged.ids[0],
                    "client-session-changed",
                    &hook,
                ],
            )
            .0,
            "install the production focus hook"
        );
    }

    std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            wait_for(
                "the stale-lead menu",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                picker_is_open,
            );
            match stale {
                StaleLead::Moved => {
                    assert!(
                        tmux(
                            &socket,
                            &watcher,
                            &["join-pane", "-d", "-s", &staged.ids[0], "-t", "foreign:1"]
                        )
                        .0,
                        "move the captured lead after the menu opened"
                    );
                    if focus_hook {
                        let after_join = tmux(
                            &socket,
                            &watcher,
                            &[
                                "display-message",
                                "-p",
                                "-t",
                                "foreign",
                                "#{window_id}|#{pane_id}",
                            ],
                        )
                        .1;
                        assert_eq!(
                            after_join.trim(),
                            foreign_view.as_deref().unwrap_or_default(),
                            "join-pane -d changed the foreign view before the choice"
                        );
                    }
                }
                StaleLead::Vanished => assert!(
                    tmux(&socket, &watcher, &["kill-pane", "-t", &staged.ids[0]]).0,
                    "kill the captured lead after the menu opened"
                ),
            }
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "1"]).0);
        });
        // A vanished `-t` may make tmux report the guarded tail as failed; the
        // externally visible contract is that the preceding session switch won.
        let _ = run_tmux(&argv, &main);
        driver
            .join()
            .unwrap_or_else(|_| panic!("the stale-lead key driver"));
    });

    let landed = wait_for(
        "the stale-lead session switch",
        || {
            tmux(
                &socket,
                &main,
                &[
                    "list-clients",
                    "-F",
                    "#{client_name}|#{client_session}|#{pane_id}",
                ],
            )
            .1
        },
        |seen| {
            seen.lines()
                .any(|line| line.starts_with(&format!("{}|hub|", staged.client)))
        },
    );
    assert!(
        landed.contains(&format!(
            "{}|home|{}",
            staged.other_client, staged.home_pane
        )),
        "the other client moved: {landed}"
    );
    if let Some(before) = foreign_view {
        let after = tmux(
            &socket,
            &main,
            &[
                "display-message",
                "-p",
                "-t",
                "foreign",
                "#{window_id}|#{pane_id}",
            ],
        )
        .1;
        assert_eq!(
            after.trim(),
            before,
            "the stale pane must not select its new session's window"
        );
    }
}

#[test]
fn a_main_pane_moved_after_open_cannot_pull_the_client_into_its_new_session() {
    stale_lead_row_lands_in_session("moved-lead", StaleLead::Moved, false);
}

#[test]
fn the_focus_hook_cannot_change_a_foreign_view_after_its_lead_moved() {
    stale_lead_row_lands_in_session("moved-lead-hook", StaleLead::Moved, true);
}

#[test]
fn a_main_pane_killed_after_open_still_leaves_the_client_in_the_session() {
    stale_lead_row_lands_in_session("vanished-lead", StaleLead::Vanished, false);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one client-marker fixture also proves picker placement and pane stability"
)]
fn popup_marks_the_clients_session_not_the_calling_panes_session() {
    let scratch = scratch("open-marker-client-session");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the picker marker cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_picker_config(&project, &config);
    for session in ["clicked", "viewed"] {
        launch_ae_session(&socket, &scratch, &root, &project, &config, session);
    }
    assert!(
        tmux(
            &socket,
            &scratch,
            &["split-window", "-d", "-h", "-t", "viewed"]
        )
        .0
    );
    let client = nested_client(&socket, &scratch, "viewed", "viewer");
    let viewed_right_pane = select_client_right_pane(&socket, &scratch, "viewed", &client);
    let clicked_pane = tmux(
        &socket,
        &scratch,
        &["list-panes", "-t", "clicked", "-F", "#{pane_id}"],
    )
    .1
    .lines()
    .next()
    .unwrap_or_else(|| panic!("clicked session has a pane"))
    .to_owned();

    let option = |session: &str| picker_marker(&socket, &scratch, session);
    let open_and_choose = |key: &str| {
        std::thread::scope(|scope| {
            let driver = scope.spawn(|| {
                let menu = wait_for(
                    "the marked picker",
                    || tmux(&socket, &scratch, &["capture-pane", "-p", "-t", "viewer"]).1,
                    picker_is_open,
                );
                let marks = (option("viewed"), option("clicked"));
                let active_pane = tmux(
                    &socket,
                    &scratch,
                    &["display-message", "-p", "-c", &client, "#{pane_id}"],
                )
                .1
                .trim()
                .to_owned();
                assert!(tmux(&socket, &scratch, &["send-keys", "-t", "viewer", key]).0);
                (marks, active_pane, menu)
            });
            let output = ae()
                .env("HOME", &scratch)
                .env("AE_HOME", &root)
                .env("CONFIG_FILE", &config)
                .env("TMUX_TMPDIR", &scratch)
                .env("TMUX", format!("{},0,0", socket.display()))
                .env("TMUX_PANE", &clicked_pane)
                .args(["orchestrator", "--popup", "--client", &client])
                .output()
                .expect("the picker invocation runs");
            assert_eq!(output.status.code(), Some(0), "{:?}", output.stderr);
            driver.join().expect("the marker reader")
        })
    };

    let ((viewed_mark, clicked_mark), active_pane, menu) = open_and_choose("q");
    let title_left = menu
        .lines()
        .find(|line| picker_is_open(line))
        .and_then(|line| {
            line.chars()
                .position(|character| matches!(character, '╭' | '┌'))
        });
    assert_eq!(
        title_left,
        Some(0),
        "picker must touch client left edge:\n{menu}"
    );
    assert_eq!(
        active_pane, viewed_right_pane,
        "opening the picker must not move the client off its active right pane"
    );
    assert!(
        viewed_mark.parse::<i64>().is_ok(),
        "the client's session owns the epoch marker: {viewed_mark:?}"
    );
    assert!(
        clicked_mark.is_empty(),
        "the calling pane's different session stays unmarked: {clicked_mark:?}"
    );
    assert_eq!(
        option("viewed"),
        viewed_mark,
        "Escape has no close hook, so the mark remains until another clear path"
    );

    let first_epoch = viewed_mark.parse::<i64>().unwrap_or_default();
    std::thread::sleep(Duration::from_secs(1));
    let ((reopened_mark, _), _, _) = open_and_choose("q");
    let reopened_epoch = reopened_mark.parse::<i64>().unwrap_or_default();
    assert!(
        reopened_epoch > first_epoch && option("viewed") == reopened_mark,
        "reopening refreshes the epoch and keeps the open menu lit: {reopened_mark:?}"
    );

    let ((selected_mark, _), _, _) = open_and_choose("1");
    assert!(selected_mark.parse::<i64>().is_ok(), "{selected_mark:?}");
    assert!(
        option("viewed").is_empty(),
        "choosing a row clears the originating session's marker"
    );
}

fn write_picker_config(project: &Path, config: &Path) {
    assert!(fs::create_dir_all(project).is_ok());
    assert!(
        fs::write(
            config,
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok()
    );
}

fn write_watchdog_picker_config(project: &Path, config: &Path, scratch: &Path) {
    write_watchdog_picker_config_with(project, config, scratch, "");
}

fn write_watchdog_picker_config_with(
    project: &Path,
    config: &Path,
    scratch: &Path,
    extra_workspace: &str,
) {
    use std::os::unix::fs::PermissionsExt as _;

    assert!(fs::create_dir_all(project).is_ok());
    let codex = scratch.join("codex");
    assert!(
        fs::write(&codex, "#!/bin/sh\nexec sleep 600\n").is_ok(),
        "a long-lived fake codex"
    );
    assert!(
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).is_ok(),
        "an executable fake codex"
    );
    assert!(
        fs::write(
            config,
            format!(
                "[profiles]\nidle = \"{}\"\n\n[roster]\nlead = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n{extra_workspace}",
                codex.display()
            ),
        )
        .is_ok()
    );
}

fn picker_marker(socket: &Path, scratch: &Path, session: &str) -> String {
    menu_marker(socket, scratch, session, ae::theme::MENU_OPEN_OPTION)
}

fn settings_marker(socket: &Path, scratch: &Path, session: &str) -> String {
    menu_marker(socket, scratch, session, ae::theme::SETTINGS_OPEN_OPTION)
}

fn menu_marker(socket: &Path, scratch: &Path, session: &str, option: &str) -> String {
    tmux(
        socket,
        scratch,
        &["show-options", "-qv", "-t", session, option],
    )
    .1
    .trim()
    .to_owned()
}

/// Put `client` on `session`'s right pane and prove it is the live view.
fn select_client_right_pane(socket: &Path, scratch: &Path, session: &str, client: &str) -> String {
    let listing = tmux(
        socket,
        scratch,
        &[
            "list-panes",
            "-t",
            session,
            "-F",
            "#{pane_id}|#{pane_at_right}",
        ],
    )
    .1;
    let right = listing
        .lines()
        .find_map(|line| line.strip_suffix("|1"))
        .unwrap_or_else(|| panic!("{session} right pane: {listing}"));
    assert!(
        tmux(socket, scratch, &["select-pane", "-t", right]).0,
        "the picker opens while the right pane is active"
    );
    assert_eq!(
        tmux(
            socket,
            scratch,
            &["display-message", "-p", "-c", client, "#{pane_id}"],
        )
        .1
        .trim(),
        right,
        "the nested client must start on the right pane"
    );
    right.to_owned()
}

fn launch_ae_session(
    socket: &Path,
    scratch: &Path,
    root: &Path,
    project: &Path,
    config: &Path,
    session: &str,
) {
    let output = ae()
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("HOME", scratch)
        .env("TMUX_TMPDIR", scratch)
        .arg(ae::cli::LAUNCH)
        .args([
            "--home",
            &root.display().to_string(),
            "--cwd",
            &project.display().to_string(),
            "--global",
            &config.display().to_string(),
            "--server-kind",
            "socket",
            "--server",
            &socket.display().to_string(),
            "--no-attach",
            "--",
            "--local",
            session,
        ])
        .output()
        .unwrap_or_else(|why| panic!("launch {session}: {why}"));
    assert_eq!(
        output.status.code(),
        Some(0),
        "launch {session}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_settings_config(project: &Path, config: &Path) {
    assert!(fs::create_dir_all(project).is_ok());
    assert!(
        fs::write(
            config,
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\norchestrator = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok()
    );
}

fn write_settings_quota_overlay(project: &Path, clients: &str) {
    let dir = project.join(".ae");
    assert!(fs::create_dir_all(&dir).is_ok(), "local config directory");
    assert!(
        fs::write(dir.join("config"), clients).is_ok(),
        "local quota config"
    );
}

fn write_settings_claude_quota(home: &Path, used: u8, observed_at: i64) {
    assert!(fs::create_dir_all(home).is_ok(), "Claude config home");
    let reset = ae::time::Timestamp::from_epoch(observed_at + 7_200);
    let cache = format!(
        "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"group\":\"session\",\"percent\":{used},\"resets_at\":\"{reset}\",\"scope\":null}}]}}}}}}\n",
        observed_at * 1_000,
    );
    assert!(fs::write(home.join(".claude.json"), cache).is_ok());
}

#[allow(
    clippy::too_many_arguments,
    reason = "one exact settings invocation tuple"
)]
fn settings_invocation(
    socket: &Path,
    scratch: &Path,
    root: &Path,
    config: &Path,
    caller_pane: &str,
    client: &str,
    action: Option<(&str, &str, &str, &str, &str, &str, i64)>,
) -> std::process::Output {
    settings_invocation_command(socket, scratch, root, config, caller_pane, client, action)
        .output()
        .unwrap_or_else(|error| panic!("the settings invocation runs: {error}"))
}

#[allow(
    clippy::too_many_arguments,
    reason = "one exact settings invocation tuple"
)]
fn settings_invocation_command(
    socket: &Path,
    scratch: &Path,
    root: &Path,
    config: &Path,
    caller_pane: &str,
    client: &str,
    action: Option<(&str, &str, &str, &str, &str, &str, i64)>,
) -> super::cli::Runner {
    let mut command = ae();
    command
        .env("HOME", scratch)
        .env("AE_HOME", root)
        .env("CONFIG_FILE", config)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", socket)
        .env("TMUX_TMPDIR", scratch)
        .env("TMUX", format!("{},fixture,0", socket.display()))
        .env("TMUX_PANE", caller_pane)
        .arg("orchestrator");
    if let Some((target, uuid, client_pid, server_pid, server_start, verb, deadline)) = action {
        command.args([
            "--settings-apply",
            verb,
            "--target",
            target,
            "--uuid",
            uuid,
            "--client",
            client,
            "--client-pid",
            client_pid,
            "--server-pid",
            server_pid,
            "--server-start",
            server_start,
            "--deadline",
            &deadline.to_string(),
        ]);
    } else {
        command.args(["--settings", "--client", client]);
    }
    command
}

#[allow(
    clippy::too_many_arguments,
    reason = "one direct terminal tuple proves independent client geometry"
)]
fn direct_terminal_client(
    socket: &Path,
    scratch: &Path,
    root: &Path,
    config: &Path,
    session: &str,
    width: usize,
    height: usize,
    record: &Path,
) -> (String, OwnedChild) {
    let command = format!(
        "stty cols {width} rows {height}; exec tmux -S {} attach-session -t ={session}",
        socket.display()
    );
    let mut terminal = helper_by_name("script");
    if cfg!(target_os = "macos") {
        terminal.args([
            "-q".to_owned(),
            record.display().to_string(),
            "sh".to_owned(),
            "-c".to_owned(),
            command,
        ]);
    } else {
        terminal.args([
            "-q".to_owned(),
            "-c".to_owned(),
            command,
            record.display().to_string(),
        ]);
    }
    terminal
        .env("HOME", scratch)
        .env("AE_HOME", root)
        .env("CONFIG_FILE", config)
        .env("TMUX_TMPDIR", scratch)
        .env("TERM", "xterm-256color")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = terminal
        .spawn()
        .unwrap_or_else(|error| panic!("private direct terminal starts: {error}"));
    let client = wait_for(
        "private direct menu client",
        || {
            tmux(
                socket,
                scratch,
                &[
                    "list-clients",
                    "-F",
                    "#{client_name}|#{client_session}|#{client_width}x#{client_height}",
                ],
            )
            .1
        },
        |seen| {
            seen.lines()
                .any(|line| line.ends_with(&format!("|{session}|{width}x{height}")))
        },
    );
    let client = client
        .lines()
        .find_map(|line| {
            line.strip_suffix(&format!("|{session}|{width}x{height}"))
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| panic!("private direct menu client: {client}"));
    (client, child)
}

fn select_direct_client_pane(
    socket: &Path,
    scratch: &Path,
    session: &str,
    client: &str,
    right: bool,
) -> String {
    let side = if right { "right" } else { "left" };
    let listing = tmux(
        socket,
        scratch,
        &[
            "list-panes",
            "-t",
            session,
            "-F",
            "#{pane_id}|#{pane_at_left}|#{pane_at_right}",
        ],
    )
    .1;
    let pane = listing
        .lines()
        .find_map(|line| {
            let fields: Vec<_> = line.split('|').collect();
            let at_side = if right {
                fields.get(2) == Some(&"1")
            } else {
                fields.get(1) == Some(&"1")
            };
            at_side.then(|| fields.first().copied())?
        })
        .unwrap_or_else(|| panic!("{session} {side} pane: {listing}"));
    assert!(
        tmux(socket, scratch, &["select-pane", "-t", pane]).0,
        "the direct client selects its {side} pane"
    );
    for (target, context) in [
        (
            vec!["display-message", "-p", "-c", client, "#{pane_id}"],
            "direct client",
        ),
        (
            vec!["display-message", "-p", "-t", session, "#{pane_id}"],
            "settings target",
        ),
    ] {
        assert_eq!(
            tmux(socket, scratch, &target).1.trim(),
            pane,
            "the {context} resolves {side} pane"
        );
    }
    pane.to_owned()
}

#[allow(
    clippy::disallowed_methods,
    reason = "the direct terminal record must be repeatedly read before its client detaches"
)]
fn wait_for_direct_settings_geometry(record: &Path, expected: &str) -> (MenuGeometry, Vec<u8>) {
    let deadline = Instant::now() + PATIENCE;
    let mut last = Vec::new();
    while Instant::now() < deadline {
        last = fs::read(record).unwrap_or_default();
        if let Some(geometry) = direct_settings_geometry(&last)
            && String::from_utf8_lossy(&last).contains(expected)
        {
            return (geometry, last);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let tail_start = last.len().saturating_sub(600);
    let tail = String::from_utf8_lossy(&last[tail_start..]);
    panic!("direct settings terminal title never settled; terminal tail={tail:?}");
}

#[allow(
    clippy::too_many_arguments,
    reason = "one direct settings draw tuple proves independent client geometry"
)]
fn draw_direct_settings_menu(
    socket: &Path,
    scratch: &Path,
    root: &Path,
    config: &Path,
    width: usize,
    right: bool,
    label: &str,
    expected: &str,
) -> DirectMenu {
    let record = scratch.join(format!("settings-{label}-{width}-{right}.terminal"));
    let (client, terminal) =
        direct_terminal_client(socket, scratch, root, config, "viewed", width, 40, &record);
    let caller_pane = select_direct_client_pane(socket, scratch, "viewed", &client, right);
    let mut settings =
        settings_invocation_command(socket, scratch, root, config, &caller_pane, &client, None);
    let settings = settings
        .spawn()
        .unwrap_or_else(|error| panic!("the direct settings invocation starts: {error}"));
    let (geometry, raw) = wait_for_direct_settings_geometry(&record, expected);
    assert!(
        tmux(socket, scratch, &["detach-client", "-t", &client]).0,
        "the direct client detaches after its terminal capture"
    );
    let output = settings
        .wait_with_output()
        .unwrap_or_else(|error| panic!("the direct settings invocation reaps: {error}"));
    assert_eq!(
        output.status.code(),
        Some(0),
        "direct settings: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let terminal = terminal
        .wait_with_output()
        .unwrap_or_else(|error| panic!("the direct terminal reaps: {error}"));
    assert_eq!(
        terminal.status.code(),
        Some(0),
        "direct terminal: {terminal:?}"
    );
    assert!(!raw.is_empty(), "direct terminal record has bytes");
    DirectMenu { geometry, raw }
}

fn assert_direct_menu_geometry(geometry: MenuGeometry, width: usize, columns: usize) {
    assert_eq!(
        geometry,
        MenuGeometry {
            left: width - columns,
            right: width - 1,
        },
        "direct menu occupies its final budget at the invoking client edge"
    );
}

/// A centred dialog is symmetric on its client: its middle column is the
/// client's middle column, whatever its width.
fn assert_direct_dialog_centred(geometry: MenuGeometry, width: usize) {
    let columns = geometry.right.saturating_sub(geometry.left) + 1;
    let middle = geometry.left + columns / 2;
    assert!(
        middle.abs_diff(width / 2) <= 1,
        "dialog centred on a {width}-column client: {geometry:?}"
    );
}

#[allow(
    clippy::disallowed_methods,
    reason = "the direct terminal record must be repeatedly read before its client detaches"
)]
fn wait_for_direct_dialog_geometry(record: &Path, expected: &str) -> (MenuGeometry, Vec<u8>) {
    let deadline = Instant::now() + PATIENCE;
    let mut last = Vec::new();
    while Instant::now() < deadline {
        last = fs::read(record).unwrap_or_default();
        if let Some(geometry) = direct_dialog_geometry(&last)
            && String::from_utf8_lossy(&last).contains(expected)
        {
            return (geometry, last);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let tail_start = last.len().saturating_sub(600);
    let tail = String::from_utf8_lossy(&last[tail_start..]);
    panic!("direct quota dialog title never settled; terminal tail={tail:?}");
}

/// The quota-dialog tail with the pid pair a settings entry captured.
fn quota_dialog_tail(
    client: &str,
    client_pid: &str,
    server_pid: &str,
    server_start: &str,
    session_id: &str,
) -> Vec<String> {
    [
        "--quota-dialog",
        "--client",
        client,
        "--client-pid",
        client_pid,
        "--server-pid",
        server_pid,
        "--server-start",
        server_start,
        "--session-id",
        session_id,
    ]
    .map(ToOwned::to_owned)
    .to_vec()
}

/// The live identity behind one client on its server.
///
/// The session comes from `list-clients`, the same source the product reads:
/// `display-message -c` does not scope `#{session_id}` to the client and would
/// name whatever session that format happens to resolve to instead.
fn dialog_identity(socket: &Path, scratch: &Path, session: &str, client: &str) -> Vec<String> {
    let listed = tmux(
        socket,
        scratch,
        &[
            "list-clients",
            "-F",
            "#{client_name}|#{client_pid}|#{session_id}",
        ],
    )
    .1;
    let (client_pid, session_id) = listed
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{client}|")))
        .and_then(|rest| rest.split_once('|'))
        .unwrap_or_else(|| panic!("the client answers pid and session: {listed:?}"));
    assert!(!client_pid.is_empty(), "the client answers its pid");
    assert!(
        session_id.starts_with('$'),
        "the client answers its session: {session_id:?}"
    );
    let server = tmux(
        socket,
        scratch,
        &[
            "display-message",
            "-p",
            "-t",
            session,
            "#{pid}|#{start_time}",
        ],
    )
    .1
    .trim()
    .to_owned();
    let (server_pid, server_start) = server
        .split_once('|')
        .unwrap_or_else(|| panic!("the server answers pid and start: {server:?}"));
    quota_dialog_tail(client, client_pid, server_pid, server_start, session_id)
}

fn quota_dialog_invocation_command(
    socket: &Path,
    scratch: &Path,
    root: &Path,
    config: &Path,
    caller_pane: &str,
    tail: &[String],
) -> super::cli::Runner {
    let mut command = ae();
    command
        .env("HOME", scratch)
        .env("AE_HOME", root)
        .env("CONFIG_FILE", config)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", socket)
        .env("TMUX_TMPDIR", scratch)
        .env("TMUX", format!("{},fixture,0", socket.display()))
        .env("TMUX_PANE", caller_pane)
        .arg("orchestrator")
        .args(tail);
    command
}

#[allow(
    clippy::too_many_arguments,
    reason = "one direct dialog draw tuple proves centred client geometry"
)]
fn draw_direct_quota_dialog(
    socket: &Path,
    scratch: &Path,
    root: &Path,
    config: &Path,
    width: usize,
    right: bool,
    label: &str,
) -> DirectMenu {
    let record = scratch.join(format!("quota-dialog-{label}-{width}-{right}.terminal"));
    let (client, terminal) =
        direct_terminal_client(socket, scratch, root, config, "viewed", width, 40, &record);
    let caller_pane = select_direct_client_pane(socket, scratch, "viewed", &client, right);
    let tail = dialog_identity(socket, scratch, "viewed", &client);
    let mut dialog =
        quota_dialog_invocation_command(socket, scratch, root, config, &caller_pane, &tail);
    let dialog = dialog
        .spawn()
        .unwrap_or_else(|error| panic!("the direct quota dialog invocation starts: {error}"));
    let (geometry, raw) = wait_for_direct_dialog_geometry(&record, "session 5h");
    assert!(
        tmux(socket, scratch, &["detach-client", "-t", &client]).0,
        "the direct client detaches after its terminal capture"
    );
    let output = dialog
        .wait_with_output()
        .unwrap_or_else(|error| panic!("the direct quota dialog invocation reaps: {error}"));
    assert_eq!(
        output.status.code(),
        Some(0),
        "direct quota dialog: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let terminal = terminal
        .wait_with_output()
        .unwrap_or_else(|error| panic!("the direct terminal reaps: {error}"));
    assert_eq!(
        terminal.status.code(),
        Some(0),
        "direct terminal: {terminal:?}"
    );
    assert!(!raw.is_empty(), "direct terminal record has bytes");
    DirectMenu { geometry, raw }
}

fn picker_invocation(
    socket: &Path,
    scratch: &Path,
    root: &Path,
    config: &Path,
    caller_pane: &str,
    client: &str,
) -> std::process::Output {
    ae().env("HOME", scratch)
        .env("AE_HOME", root)
        .env("CONFIG_FILE", config)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", socket)
        .env("TMUX_TMPDIR", scratch)
        .env("TMUX", format!("{},fixture,0", socket.display()))
        .env("TMUX_PANE", caller_pane)
        .args(["orchestrator", "--popup", "--client", client])
        .output()
        .unwrap_or_else(|error| panic!("the picker invocation runs: {error}"))
}

#[allow(clippy::too_many_arguments, reason = "one real menu draw tuple")]
fn choose_settings_row(
    socket: &Path,
    scratch: &Path,
    root: &Path,
    config: &Path,
    caller_pane: &str,
    client: &str,
    viewer: &str,
    other_viewer: &str,
    expected: &str,
    key: &str,
) -> String {
    std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            let menu = wait_for(
                "settings menu",
                || tmux(socket, scratch, &["capture-pane", "-p", "-t", viewer]).1,
                |seen| seen.contains("settings") && seen.contains(expected),
            );
            let other = tmux(socket, scratch, &["capture-pane", "-p", "-t", other_viewer]).1;
            assert!(!other.contains(expected), "menu leaked: {other}");
            assert!(tmux(socket, scratch, &["send-keys", "-t", viewer, key]).0);
            menu
        });
        let output = settings_invocation(socket, scratch, root, config, caller_pane, client, None);
        assert_eq!(
            output.status.code(),
            Some(0),
            "settings: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        driver
            .join()
            .unwrap_or_else(|_| panic!("settings key driver panicked"))
    })
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one private-tmux Start, renamed Resume and stale-target lifecycle story"
)]
fn settings_starts_then_resumes_the_exact_renamed_role_without_switching_its_client() {
    let scratch = scratch("orchestrator-actions");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so settings actions cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_settings_config(&project, &config);
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "viewed",
                ae::theme::VERSION_OPTION,
                "ae 2099.1.2",
            ],
        )
        .0
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["split-window", "-d", "-h", "-t", "viewed"]
        )
        .0
    );
    let clicked = nested_client(&socket, &scratch, "viewed", "settings-viewer");
    let untouched = nested_client(&socket, &scratch, "viewed", "settings-other");
    let caller_pane = select_client_right_pane(&socket, &scratch, "viewed", &clicked);

    let start_menu = choose_settings_row(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        "settings-viewer",
        "settings-other",
        "Start orchestrator",
        "s",
    );
    assert!(start_menu.contains("ae 2099.1.2 settings"), "{start_menu}");
    let role_dir = root.join("sessions/orchestrator");
    wait_for(
        "canonical role Start",
        || {
            let live = tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0;
            let role = ae::meta::meta_agent_role(&meta_bytes(&role_dir));
            format!("{live}|{role:?}")
        },
        |seen| seen == "true|Role",
    );
    let clients = tmux(
        &socket,
        &scratch,
        &["list-clients", "-F", "#{client_name}|#{client_session}"],
    )
    .1;
    assert!(clients.contains(&format!("{clicked}|viewed")), "{clients}");
    assert!(
        clients.contains(&format!("{untouched}|viewed")),
        "{clients}"
    );
    let renamed = ae()
        .env("HOME", &scratch)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", &config)
        .env("TMUX_TMPDIR", &scratch)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args([ae::cli::RENAME, "orchestrator", "renamed"])
        .output()
        .expect("rename runs");
    assert_eq!(renamed.status.code(), Some(0), "{renamed:?}");
    assert!(
        tmux(&socket, &scratch, &["kill-session", "-t", "=renamed"]).0,
        "the renamed role stops"
    );
    let renamed_dir = root.join("sessions/renamed");
    let resume_menu = choose_settings_row(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        "settings-viewer",
        "settings-other",
        "Resume orchestrator 'renamed'",
        "r",
    );
    assert!(resume_menu.contains("orchestrator: stopped (renamed)"));
    wait_for(
        "exact renamed Resume",
        || {
            tmux(&socket, &scratch, &["has-session", "-t", "=renamed"])
                .0
                .to_string()
        },
        |seen| seen == "true",
    );
    wait_for(
        "renamed Resume completion",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "settings-viewer"],
            )
            .1
        },
        |seen| seen.contains("Resumed orchestrator 'renamed'"),
    );
    assert!(tmux(&socket, &scratch, &["display-message", "-c", &clicked, ""]).0);
    let clients = tmux(
        &socket,
        &scratch,
        &["list-clients", "-F", "#{client_name}|#{client_session}"],
    )
    .1;
    assert!(clients.contains(&format!("{clicked}|viewed")), "{clients}");

    assert!(
        tmux(&socket, &scratch, &["kill-session", "-t", "=renamed"]).0,
        "the resumed role stops for the stale-row case"
    );
    let before = meta_bytes(&renamed_dir);
    assert!(!before.is_empty(), "role bytes before stale Resume");
    let parked = root.join("parked-role");
    std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            wait_for(
                "stale Resume menu",
                || {
                    tmux(
                        &socket,
                        &scratch,
                        &["capture-pane", "-p", "-t", "settings-viewer"],
                    )
                    .1
                },
                |seen| seen.contains("Resume orchestrator 'renamed'"),
            );
            fs::rename(&renamed_dir, &parked).expect("remove target before Resume preflight");
            assert!(
                tmux(
                    &socket,
                    &scratch,
                    &["send-keys", "-t", "settings-viewer", "r"]
                )
                .0
            );
        });
        let output = settings_invocation(
            &socket,
            &scratch,
            &root,
            &config,
            &caller_pane,
            &clicked,
            None,
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        driver.join().expect("stale Resume driver");
    });
    let refusal = wait_for(
        "stale Resume refusal",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "settings-viewer"],
            )
            .1
        },
        |seen| seen.contains("disappeared before Resume"),
    );
    assert!(refusal.contains("Nothing was resumed"), "{refusal}");
    assert!(
        !tmux(&socket, &scratch, &["has-session", "-t", "=renamed"]).0,
        "a missing target was recreated"
    );
    fs::rename(&parked, &renamed_dir).expect("restore the exact stopped identity");
    assert_eq!(meta_bytes(&renamed_dir), before);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one expiry and two forged-identity delivery cases on one real attachment"
)]
fn settings_reports_expiry_only_to_a_reproven_attachment_and_never_to_forged_identity() {
    let scratch = scratch("settings-report-routing");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so exact-client reports cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_settings_config(&project, &config);
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    let clicked = nested_client(&socket, &scratch, "viewed", "report-viewer");
    let untouched = nested_client(&socket, &scratch, "viewed", "report-other");
    let server = ServerId::Selected(Selector::Socket(socket.clone()));
    let identity = ae::transport::observe_server_identity(&server).expect("server identity");
    let client = ae::transport::observe_menu_client(&server, &clicked).expect("client identity");
    let caller_pane = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", "viewed", "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    let lock_path = root.join("sessions/.lifecycle.orchestrator.lock");
    let held = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&lock_path)
        .expect("the canonical lifecycle lock opens");
    held.try_lock().expect("the test holds the Start lock");
    let deadline = ae::time::Timestamp::now().epoch() + 1;
    let action_output = std::thread::scope(|scope| {
        let action = scope.spawn(|| {
            settings_invocation(
                &socket,
                &scratch,
                &root,
                &config,
                &caller_pane,
                &clicked,
                Some((
                    "orchestrator",
                    "",
                    &client.pid,
                    &identity.pid,
                    &identity.start,
                    "start",
                    deadline,
                )),
            )
        });
        std::thread::sleep(Duration::from_secs(2));
        drop(held);
        action.join().expect("expired Start action")
    });
    assert_eq!(action_output.status.code(), Some(1), "{action_output:?}");
    assert!(String::from_utf8_lossy(&action_output.stderr).contains("expired"));
    wait_for(
        "visible expiry",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "report-viewer"],
            )
            .1
        },
        |seen| seen.contains("expired"),
    );
    let other = tmux(
        &socket,
        &scratch,
        &["capture-pane", "-p", "-t", "report-other"],
    )
    .1;
    assert!(!other.contains("expired"), "report leaked: {other}");
    assert!(
        tmux(&socket, &scratch, &["display-message", "-c", &clicked, ""]).0,
        "clear the positive control before negative delivery checks"
    );

    for (tag, client_pid, server_pid, reason) in [
        ("client", "1", identity.pid.as_str(), "different attachment"),
        ("server", client.pid.as_str(), "1", "server was replaced"),
    ] {
        let refused = settings_invocation(
            &socket,
            &scratch,
            &root,
            &config,
            &caller_pane,
            &clicked,
            Some((
                "orchestrator",
                "",
                client_pid,
                server_pid,
                &identity.start,
                "start",
                ae::time::Timestamp::now().epoch() + 60,
            )),
        );
        assert_eq!(refused.status.code(), Some(1), "{tag}: {refused:?}");
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains(reason),
            "{tag}: {refused:?}"
        );
        let visible = tmux(
            &socket,
            &scratch,
            &["capture-pane", "-p", "-t", "report-viewer"],
        )
        .1;
        assert!(!visible.contains(reason), "{tag} leaked: {visible}");
    }
    assert!(!state_kept(&root.join("sessions/orchestrator")));
    assert!(!tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0);
    assert!(!untouched.is_empty());
}

#[test]
#[allow(
    clippy::disallowed_methods,
    clippy::too_many_lines,
    reason = "the private fixture inspects its marker and protected scratch paths"
)]
fn settings_rechecks_role_liveness_and_uuid_after_each_captured_action() {
    const REPLACEMENT_UUID: &str = "44444444-4444-4444-8444-444444444444";
    let scratch = scratch("settings-stale-actions");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so stale settings actions cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_settings_config(&project, &config);
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    launch_ae_session(&socket, &scratch, &root, &project, &config, "role-source");
    let bystander_before = meta_bytes(&root.join("sessions/viewed"));
    let clicked = nested_client(&socket, &scratch, "viewed", "stale-viewer");
    let caller_pane = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", "viewed", "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    let server = ServerId::Selected(Selector::Socket(socket.clone()));
    let identity = ae::transport::observe_server_identity(&server).expect("server identity");
    let client = ae::transport::observe_menu_client(&server, &clicked).expect("client identity");

    // Hold the canonical lock while a separately authorized different-name
    // role lands through the real rename path. The fixed debug marker proves
    // the continuation finished preflight before the competing role appears.
    let canonical_lock = root.join("sessions/.lifecycle.orchestrator.lock");
    let held = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&canonical_lock)
        .expect("the canonical lifecycle lock opens");
    held.try_lock().expect("the test holds the canonical lock");
    let marker = root.join(ae::session_launch::TEST_PRE_LOCK_MARKER);
    assert!(!marker.exists(), "the pre-lock marker starts absent");
    let deadline = (ae::time::Timestamp::now().epoch() + 60).to_string();
    let mut command = ae();
    command
        .env("HOME", &scratch)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", &config)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &socket)
        .env("TMUX_TMPDIR", &scratch)
        .env("TMUX", format!("{},fixture,0", socket.display()))
        .env("TMUX_PANE", &caller_pane)
        .args([
            "orchestrator",
            "--settings-apply",
            "start",
            "--test-pre-lock-marker",
            "--target",
            "orchestrator",
            "--uuid",
            "",
            "--client",
            &clicked,
            "--client-pid",
            &client.pid,
            "--server-pid",
            &identity.pid,
            "--server-start",
            &identity.start,
            "--deadline",
            &deadline,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().expect("the captured Start begins");
    wait_for(
        "settings Start pre-lock marker",
        || marker.is_file().to_string(),
        |seen| seen == "true",
    );
    assert!(
        child.try_wait().expect("inspect marked Start").is_none(),
        "marked Start exited while the test still held its lifecycle lock"
    );
    assert!(!state_kept(&root.join("sessions/orchestrator")));
    assert!(!root.join("sessions/orchestrator/.launch-attempt").exists());
    assert!(!tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0);
    let source_dir = root.join("sessions/role-source");
    let mut source_meta = String::from_utf8(meta_bytes(&source_dir)).expect("text source meta");
    source_meta.push_str("meta_agent=true\n");
    assert!(fs::write(source_dir.join("meta"), source_meta).is_ok());
    let renamed = ae()
        .env("HOME", &scratch)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", &config)
        .env("TMUX_TMPDIR", &scratch)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args([ae::cli::RENAME, "role-source", "renamed-role"])
        .output()
        .expect("the competing role rename runs");
    assert_eq!(renamed.status.code(), Some(0), "{renamed:?}");
    let competing = root.join("sessions/renamed-role");
    let competing_meta = meta_bytes(&competing);
    assert_eq!(
        ae::meta::meta_agent_role(&competing_meta),
        ae::meta::MetaAgentRole::Role
    );
    drop(held);
    let stale_start = child.wait_with_output().expect("captured Start completes");
    let start_err = String::from_utf8_lossy(&stale_start.stderr);
    assert_eq!(stale_start.status.code(), Some(1), "{start_err}");
    assert!(
        start_err.contains("now recorded by 'renamed-role'"),
        "{start_err}"
    );
    assert!(!state_kept(&root.join("sessions/orchestrator")));
    assert_eq!(meta_bytes(&competing), competing_meta);
    assert_eq!(meta_bytes(&root.join("sessions/viewed")), bystander_before);
    assert!(tmux(&socket, &scratch, &["has-session", "-t", "=renamed-role"]).0);
    assert!(tmux(&socket, &scratch, &["kill-session", "-t", "=renamed-role"]).0);
    assert!(fs::remove_dir_all(&competing).is_ok());

    // Capture a stopped exact role, then make it live before invoking Resume.
    launch_ae_session(&socket, &scratch, &root, &project, &config, "renamed");
    let renamed_dir = root.join("sessions/renamed");
    let mut role_meta = String::from_utf8(meta_bytes(&renamed_dir)).expect("text role meta");
    role_meta.push_str("meta_agent=true\n");
    assert!(fs::write(renamed_dir.join("meta"), role_meta).is_ok());
    assert!(tmux(&socket, &scratch, &["kill-session", "-t", "=renamed"]).0);
    let stopped = meta_bytes(&renamed_dir);
    let uuid = ae::meta::sole_value(&stopped, "session_id")
        .map(String::from_utf8_lossy)
        .map(std::borrow::Cow::into_owned)
        .expect("saved role UUID");
    launch_ae_session(&socket, &scratch, &root, &project, &config, "renamed");
    let mut live_role = String::from_utf8(meta_bytes(&renamed_dir)).expect("text live meta");
    live_role.push_str("meta_agent=true\n");
    assert!(fs::write(renamed_dir.join("meta"), live_role).is_ok());
    let live_before = meta_bytes(&renamed_dir);
    let now_live = settings_invocation(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        Some((
            "renamed",
            &uuid,
            &client.pid,
            &identity.pid,
            &identity.start,
            "resume",
            ae::time::Timestamp::now().epoch() + 60,
        )),
    );
    let live_err = String::from_utf8_lossy(&now_live.stderr);
    assert_eq!(now_live.status.code(), Some(1), "{live_err}");
    assert!(
        live_err.contains("is live now; stale Resume refused"),
        "{live_err}"
    );
    assert!(tmux(&socket, &scratch, &["has-session", "-t", "=renamed"]).0);
    assert_eq!(meta_bytes(&renamed_dir), live_before);
    assert!(tmux(&socket, &scratch, &["kill-session", "-t", "=renamed"]).0);

    // Replace only the stopped identity; the captured UUID must not follow it.
    let replacement = ae::meta::rewritten(
        &String::from_utf8(meta_bytes(&renamed_dir)).expect("text stopped meta"),
        "session_id",
        Some(REPLACEMENT_UUID),
    );
    assert!(fs::write(renamed_dir.join("meta"), &replacement).is_ok());
    let replaced = settings_invocation(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        Some((
            "renamed",
            &uuid,
            &client.pid,
            &identity.pid,
            &identity.start,
            "resume",
            ae::time::Timestamp::now().epoch() + 60,
        )),
    );
    let replaced_err = String::from_utf8_lossy(&replaced.stderr);
    assert_eq!(replaced.status.code(), Some(1), "{replaced_err}");
    assert!(replaced_err.contains("was replaced"), "{replaced_err}");
    assert!(!tmux(&socket, &scratch, &["has-session", "-t", "=renamed"]).0);
    assert_eq!(meta_bytes(&renamed_dir), replacement.as_bytes());
    assert_eq!(meta_bytes(&root.join("sessions/viewed")), bystander_before);
}

fn rendered_status(socket: &Path, scratch: &Path, client: &str) -> String {
    tmux(
        socket,
        scratch,
        &[
            "display-message",
            "-p",
            "-c",
            client,
            "#{E:status-format[1]}",
        ],
    )
    .1
}

fn measured_cursor_width(
    socket: &Path,
    scratch: &Path,
    session: &str,
    print_command: &str,
) -> String {
    assert!(
        tmux(
            socket,
            scratch,
            &[
                "new-session",
                "-d",
                "-s",
                session,
                "-x",
                "80",
                "-y",
                "5",
                print_command,
            ],
        )
        .0,
        "create the width-measurement pane for {session}"
    );
    let width = wait_for(
        session,
        || {
            tmux(
                socket,
                scratch,
                &["list-panes", "-t", session, "-F", "#{cursor_x}"],
            )
            .1
        },
        |seen| seen.trim() != "0" && !seen.trim().is_empty(),
    );
    assert!(
        tmux(socket, scratch, &["kill-session", "-t", session]).0,
        "remove the width-measurement pane for {session}"
    );
    width.trim().to_owned()
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one private-tmux quota provenance and three-boundary settings story"
)]
fn settings_quota_uses_the_invoking_overlay_and_degrades_without_losing_the_action() {
    let scratch = scratch("settings-quota");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so settings quota cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let viewed_project = scratch.join("viewed-project");
    let other_project = scratch.join("other-project");
    let config = scratch.join("config");
    write_settings_config(&viewed_project, &config);
    assert!(fs::create_dir_all(&other_project).is_ok());
    write_settings_quota_overlay(
        &viewed_project,
        concat!(
            "[clients]\n",
            "menu-claude = claude config_home=$HOME/.menu-claude\n",
            "menu-grok = grok\n",
            "[profiles]\n",
            "menu-supported = menu-claude\n",
            "menu-unsupported = menu-grok\n",
        ),
    );
    write_settings_quota_overlay(
        &other_project,
        "[clients]\nleak-client = agy\n[profiles]\nleak-profile = leak-client\n",
    );
    let now = ae::time::Timestamp::now().epoch();
    write_settings_claude_quota(&scratch.join(".menu-claude"), 82, now);

    launch_ae_session(&socket, &scratch, &root, &viewed_project, &config, "viewed");
    launch_ae_session(&socket, &scratch, &root, &other_project, &config, "other");
    let clicked = nested_client(&socket, &scratch, "viewed", "quota-viewer");
    let untouched = nested_client(&socket, &scratch, "viewed", "quota-other");
    let caller_pane = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-c", &clicked, "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();

    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "resize-window",
                "-t",
                "quota-viewer",
                "-x",
                "90",
                "-y",
                "40"
            ]
        )
        .0
    );
    wait_for(
        "medium settings quota client",
        || {
            tmux(
                &socket,
                &scratch,
                &[
                    "display-message",
                    "-p",
                    "-c",
                    &clicked,
                    "#{client_width}x#{client_height}",
                ],
            )
            .1
        },
        |seen| seen.trim() == "90x40",
    );
    let full = choose_settings_row(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        "quota-viewer",
        "quota-other",
        "Quota for our clients...",
        "Escape",
    );
    assert!(!full.contains("quota  claude/menu-claude"), "{full}");
    assert!(!full.contains("quota  grok/menu-grok"), "{full}");
    assert!(!full.contains('+'), "{full}");
    assert!(!full.contains("leak-client"), "{full}");
    assert!(full.contains("Start orchestrator"), "{full}");
    assert!(
        !tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0,
        "Escape on the quota entry never acted"
    );
    let selected_style = format!(
        "#[bg={} fg={}]",
        ae::theme::Palette::DARCULA.selected,
        ae::theme::Palette::DARCULA.selected_ink,
    );
    let settings_selected = format!("#[range=user|ae-settings]{selected_style} ⚙ #[norange]");
    let picker_selected = format!("#[range=user|ae]{selected_style}");
    let after_escape = rendered_status(&socket, &scratch, &clicked);
    assert!(
        settings_marker(&socket, &scratch, "viewed")
            .parse::<i64>()
            .is_ok(),
        "Escape leaves only bounded transient settings state"
    );
    assert!(picker_marker(&socket, &scratch, "viewed").is_empty());
    assert!(after_escape.contains(&settings_selected), "{after_escape}");
    assert!(!after_escape.contains(&picker_selected), "{after_escape}");

    let (picker, settings_during_picker, picker_during_picker, picker_status) =
        std::thread::scope(|scope| {
            let driver = scope.spawn(|| {
                let picker = wait_for(
                    "picker replacing an escaped settings menu",
                    || {
                        tmux(
                            &socket,
                            &scratch,
                            &["capture-pane", "-p", "-t", "quota-viewer"],
                        )
                        .1
                    },
                    picker_is_open,
                );
                let settings = settings_marker(&socket, &scratch, "viewed");
                let fleet = picker_marker(&socket, &scratch, "viewed");
                let status = rendered_status(&socket, &scratch, &clicked);
                assert!(
                    tmux(
                        &socket,
                        &scratch,
                        &["send-keys", "-t", "quota-viewer", "Escape"]
                    )
                    .0
                );
                (picker, settings, fleet, status)
            });
            let output =
                picker_invocation(&socket, &scratch, &root, &config, &caller_pane, &clicked);
            assert_eq!(
                output.status.code(),
                Some(0),
                "picker: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            driver.join().expect("picker marker driver")
        });
    assert!(picker_is_open(&picker), "{picker}");
    assert!(
        settings_during_picker.is_empty(),
        "{settings_during_picker:?}"
    );
    assert!(
        picker_during_picker.parse::<i64>().is_ok(),
        "{picker_during_picker:?}"
    );
    assert!(picker_status.contains(&picker_selected), "{picker_status}");
    assert!(
        !picker_status.contains(&settings_selected),
        "{picker_status}"
    );

    assert!(
        tmux(
            &socket,
            &scratch,
            &["resize-window", "-t", "quota-viewer", "-x", "90", "-y", "5"]
        )
        .0
    );
    wait_for(
        "base-height-minus-one settings client",
        || {
            tmux(
                &socket,
                &scratch,
                &["display-message", "-p", "-c", &clicked, "#{client_height}"],
            )
            .1
        },
        |seen| seen.trim() == "5",
    );
    let refused = settings_invocation(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        None,
    );
    assert_eq!(refused.status.code(), Some(1));
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(
        error.contains("this terminal is 90x5; settings needs") && error.contains("x6"),
        "{error}"
    );
    assert!(
        !tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0,
        "the original base refusal remains side-effect free"
    );
    assert!(settings_marker(&socket, &scratch, "viewed").is_empty());
    assert!(
        picker_marker(&socket, &scratch, "viewed")
            .parse::<i64>()
            .is_ok()
    );

    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "resize-window",
                "-t",
                "quota-viewer",
                "-x",
                "90",
                "-y",
                "40"
            ]
        )
        .0
    );
    wait_for(
        "settings client restored for watchdog expiry",
        || {
            tmux(
                &socket,
                &scratch,
                &["display-message", "-p", "-c", &clicked, "#{client_height}"],
            )
            .1
        },
        |seen| seen.trim() == "40",
    );
    let _ = choose_settings_row(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        "quota-viewer",
        "quota-other",
        "Quota for our clients...",
        "Escape",
    );
    assert!(picker_marker(&socket, &scratch, "viewed").is_empty());
    assert!(
        settings_marker(&socket, &scratch, "viewed")
            .parse::<i64>()
            .is_ok()
    );
    let meta_dir = root.join("sessions/viewed");
    // Keep this fixture under the test suite's kill-on-Drop owner: a panic in
    // the expiry assertion must not leave this newly spawned watchdog behind.
    let mut watchdog: super::cli::OwnedChild = ae()
        .arg("_watchdog-run")
        .arg(&meta_dir)
        .args([
            "--interval",
            "1",
            "--quiet-beat-ms",
            "10",
            "--tg-supervise-secs",
            "0",
        ])
        .env("HOME", &scratch)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", &config)
        .stdout(fs::File::create(scratch.join("expiry-watchdog.out")).expect("stdout sink"))
        .stderr(fs::File::create(scratch.join("expiry-watchdog.err")).expect("stderr sink"))
        .spawn()
        .expect("the expiry watchdog starts");
    wait_for(
        "watchdog expiry of an escaped settings marker",
        || settings_marker(&socket, &scratch, "viewed"),
        str::is_empty,
    );
    let _ = watchdog.kill();
    watchdog.wait().expect("the expiry watchdog is reaped");

    assert!(
        tmux(
            &socket,
            &scratch,
            &["resize-window", "-t", "quota-viewer", "-x", "90", "-y", "6"]
        )
        .0
    );
    wait_for(
        "exact-base-height settings client",
        || {
            tmux(
                &socket,
                &scratch,
                &["display-message", "-p", "-c", &clicked, "#{client_height}"],
            )
            .1
        },
        |seen| seen.trim() == "6",
    );
    let degraded = choose_settings_row(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        "quota-viewer",
        "quota-other",
        "+2r +0c",
        "s",
    );
    assert!(degraded.contains("Start orchestrator"), "{degraded}");
    assert!(degraded.contains("+2r"), "{degraded}");
    assert!(degraded.contains("+0c"), "{degraded}");
    assert!(!degraded.contains("Quota for our clients..."), "{degraded}");
    wait_for(
        "orchestrator from exact-height degraded settings",
        || {
            tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"])
                .0
                .to_string()
        },
        |seen| seen == "true",
    );
    assert!(tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0);
    assert!(settings_marker(&socket, &scratch, "viewed").is_empty());
    assert!(picker_marker(&socket, &scratch, "viewed").is_empty());
    assert!(!untouched.is_empty());
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real settings-entry to quota-dialog to Close lifecycle story"
)]
fn settings_quota_entry_opens_the_per_window_dialog_and_close_dismisses_it() {
    let scratch = scratch("settings-quota-dialog");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the quota dialog flow cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let other_project = scratch.join("other-project");
    let config = scratch.join("config");
    write_settings_config(&project, &config);
    write_settings_quota_overlay(
        &project,
        concat!(
            "[clients]\n",
            "menu-claude = claude config_home=$HOME/.menu-claude\n",
            "menu-grok = grok\n",
            "[profiles]\n",
            "menu-supported = menu-claude\n",
            "menu-unsupported = menu-grok\n",
        ),
    );
    // A SECOND ae session with a DISTINCT overlay: the dialog must read the
    // invoking session's overlay, not an arbitrary or first one ("other" sorts
    // before "viewed", so an alphabetical grab would surface other-claude).
    assert!(fs::create_dir_all(&other_project).is_ok());
    write_settings_quota_overlay(
        &other_project,
        "[clients]\nother-claude = claude config_home=$HOME/.other-claude\n[profiles]\nother-profile = other-claude\n",
    );
    let now = ae::time::Timestamp::now().epoch();
    write_settings_claude_quota(&scratch.join(".menu-claude"), 82, now);
    write_settings_claude_quota(&scratch.join(".other-claude"), 11, now);
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    launch_ae_session(&socket, &scratch, &root, &other_project, &config, "other");
    let clicked = nested_client(&socket, &scratch, "viewed", "dialog-viewer");
    let untouched = nested_client(&socket, &scratch, "viewed", "dialog-other");
    let caller_pane = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-c", &clicked, "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "resize-window",
                "-t",
                "dialog-viewer",
                "-x",
                "90",
                "-y",
                "40"
            ]
        )
        .0
    );
    wait_for(
        "wide dialog client",
        || {
            tmux(
                &socket,
                &scratch,
                &[
                    "display-message",
                    "-p",
                    "-c",
                    &clicked,
                    "#{client_width}x#{client_height}",
                ],
            )
            .1
        },
        |seen| seen.trim() == "90x40",
    );

    let (settings_seen, dialog_seen) = std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            let settings = wait_for(
                "settings quota entry",
                || {
                    tmux(
                        &socket,
                        &scratch,
                        &["capture-pane", "-p", "-t", "dialog-viewer"],
                    )
                    .1
                },
                |seen| seen.contains("settings") && seen.contains("Quota for our clients..."),
            );
            assert!(
                !settings.contains("session 5h"),
                "no inline quota rows behind the entry: {settings}"
            );
            // Mutate the invoking session's quota AFTER settings is on screen:
            // the dialog must show this fresh value, proving the dialog-time
            // reread — and the other session's overlay must stay invisible.
            write_settings_claude_quota(
                &scratch.join(".menu-claude"),
                83,
                ae::time::Timestamp::now().epoch(),
            );
            assert!(
                tmux(
                    &socket,
                    &scratch,
                    &["send-keys", "-t", "dialog-viewer", "q"]
                )
                .0
            );
            let dialog = wait_for(
                "quota dialog",
                || {
                    tmux(
                        &socket,
                        &scratch,
                        &["capture-pane", "-p", "-t", "dialog-viewer"],
                    )
                    .1
                },
                |seen| seen.contains("quota for our clients") && seen.contains("session 5h"),
            );
            assert!(
                settings_marker(&socket, &scratch, "viewed").is_empty(),
                "choosing the entry clears the settings marker"
            );
            assert!(
                tmux(
                    &socket,
                    &scratch,
                    &["send-keys", "-t", "dialog-viewer", "c"]
                )
                .0
            );
            let closed = wait_for(
                "closed quota dialog",
                || {
                    tmux(
                        &socket,
                        &scratch,
                        &["capture-pane", "-p", "-t", "dialog-viewer"],
                    )
                    .1
                },
                |seen| !seen.contains("quota for our clients"),
            );
            assert!(!closed.contains("Quota for our clients..."), "{closed}");
            (settings, dialog)
        });
        let output = settings_invocation(
            &socket,
            &scratch,
            &root,
            &config,
            &caller_pane,
            &clicked,
            None,
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "settings: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        driver.join().expect("quota dialog key driver")
    });
    assert!(
        settings_seen.contains("Start orchestrator"),
        "{settings_seen}"
    );
    assert!(dialog_seen.contains("claude/menu-claude"), "{dialog_seen}");
    assert!(
        dialog_seen.contains("  session 5h | 83%"),
        "the dialog rereads at open time under the invoking overlay: {dialog_seen}"
    );
    assert!(
        !dialog_seen.contains("82%"),
        "no stale settings-time value: {dialog_seen}"
    );
    assert!(
        !dialog_seen.contains("other-claude") && !dialog_seen.contains("11%"),
        "the other session's overlay never leaks in: {dialog_seen}"
    );
    assert!(dialog_seen.contains("fresh"), "{dialog_seen}");
    assert!(dialog_seen.contains("unsupported"), "{dialog_seen}");
    assert!(dialog_seen.contains("Close"), "{dialog_seen}");
    assert!(
        !dialog_seen.contains("Start orchestrator"),
        "the dialog carries no settings action: {dialog_seen}"
    );
    assert!(
        !tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0,
        "opening quota never acted"
    );
    assert!(settings_marker(&socket, &scratch, "viewed").is_empty());
    assert!(picker_marker(&socket, &scratch, "viewed").is_empty());
    assert!(!untouched.is_empty());
}

/// The meta record one launched session persisted, or nothing when it did not.
#[allow(
    clippy::disallowed_methods,
    reason = "a fixture reading its own scratch directory; the capability boundary is about what PRODUCT code may reach"
)]
fn session_meta(root: &Path, session: &str) -> String {
    fs::read_to_string(root.join("sessions").join(session).join("meta")).unwrap_or_default()
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one private-tmux unaware-settings story: pinned launch, menu draw, dialog refusal"
)]
fn settings_quota_unaware_session_draws_no_quota_entry_and_refuses_the_dialog() {
    let scratch = scratch("settings-unaware");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the unaware settings surface cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    // `quota = off` pinned at launch: the session meta carries the pin.
    assert!(fs::create_dir_all(&project).is_ok());
    assert!(
        fs::write(
            &config,
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\norchestrator = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\nquota = off\n",
        )
        .is_ok()
    );
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    let meta = session_meta(&root, "viewed");
    assert!(
        meta.contains("quota=off\n"),
        "the unaware settings case starts from a pinned quota=off: {meta}"
    );
    let clicked = nested_client(&socket, &scratch, "viewed", "unaware-viewer");
    let untouched = nested_client(&socket, &scratch, "viewed", "unaware-other");
    let caller_pane = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-c", &clicked, "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "resize-window",
                "-t",
                "unaware-viewer",
                "-x",
                "90",
                "-y",
                "40"
            ]
        )
        .0
    );
    wait_for(
        "medium unaware settings client",
        || {
            tmux(
                &socket,
                &scratch,
                &[
                    "display-message",
                    "-p",
                    "-c",
                    &clicked,
                    "#{client_width}x#{client_height}",
                ],
            )
            .1
        },
        |seen| seen.trim() == "90x40",
    );

    // The settings menu is the base control menu: no quota entry row anywhere.
    let seen = choose_settings_row(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        "unaware-viewer",
        "unaware-other",
        "Start orchestrator",
        "Escape",
    );
    assert!(seen.contains("Start orchestrator"), "{seen}");
    // The entry label is the observable: a whole-capture word sweep is
    // impossible here because the worktree path itself (`ae-wt/quotaaware`)
    // echoes in the pane behind the menu. The render unit tests own the
    // case-insensitive word sweep over the documents themselves.
    assert!(
        !seen.contains("Quota for our clients..."),
        "unaware settings carries no quota entry row: {seen}"
    );

    // The hand-built continuation refuses instead of drawing. The closer
    // driver keeps this hang-free in both worlds: with the refusal the
    // invocation exits at once; if a dialog ever draws (the gate deleted),
    // Close dismisses it so the exit code — not a timeout — is the verdict.
    let tail = dialog_identity(&socket, &scratch, "viewed", &clicked);
    let done = std::sync::atomic::AtomicBool::new(false);
    let output = std::thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(15);
            while !done.load(std::sync::atomic::Ordering::Relaxed) && Instant::now() < deadline {
                let seen = tmux(
                    &socket,
                    &scratch,
                    &["capture-pane", "-p", "-t", "unaware-viewer"],
                )
                .1;
                if seen.contains("quota for our clients") {
                    assert!(
                        tmux(
                            &socket,
                            &scratch,
                            &["send-keys", "-t", "unaware-viewer", "c"]
                        )
                        .0
                    );
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let output =
            quota_dialog_invocation_command(&socket, &scratch, &root, &config, &caller_pane, &tail)
                .output()
                .unwrap_or_else(|error| panic!("the quota dialog invocation runs: {error}"));
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        output
    });
    assert_eq!(
        output.status.code(),
        Some(1),
        "unaware dialog refuses: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("quota awareness is off"),
        "the refusal names the toggle: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!untouched.is_empty());
}

#[test]
fn quota_dialog_window_indent_survives_a_real_centred_menu_draw() {
    let scratch = scratch("quota-indent");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so menu indentation cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let main = scratch.join("main");
    let watcher = scratch.join("watcher");
    let staged = stage(&socket, &main);
    let server = ServerId::Selected(Selector::Socket(socket.clone()));
    let menu = ae::tmux::Menu {
        title: "quota for our clients".to_owned(),
        title_style: String::new(),
        items: vec![
            ae::tmux::MenuItem {
                label: "claude / ~/.claude".to_owned(),
                key: String::new(),
                action: ae::tmux::MenuAction::Disabled,
            },
            ae::tmux::MenuItem {
                label: "  session 5h | 39%".to_owned(),
                key: String::new(),
                action: ae::tmux::MenuAction::Disabled,
            },
            ae::tmux::MenuItem {
                label: "Close".to_owned(),
                key: "c".to_owned(),
                action: ae::tmux::MenuAction::Run(String::new()),
            },
        ],
    };
    let argv = ae::tmux::display_menu_centred_args(
        &server,
        &staged.client,
        &staged.home_pane,
        &menu,
        false,
    );
    let drawn = std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            let seen = wait_for(
                "indented quota menu",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                |seen| seen.contains("session 5h") && seen.contains("Close"),
            );
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "Escape"]).0);
            seen
        });
        let (succeeded, _) = run_tmux(&argv, &main);
        assert!(succeeded, "tmux refused the centred indent probe");
        driver.join().expect("indent key driver")
    });
    assert!(
        drawn.contains("  session 5h"),
        "two-space window indent survives display-menu: {drawn:?}"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one direct-terminal settings geometry proof across full and notice quota branches"
)]
fn settings_menu_uses_client_right_geometry_from_either_split_pane() {
    let scratch = scratch("settings-right-geometry");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so direct settings geometry cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_settings_config(&project, &config);
    write_settings_quota_overlay(
        &project,
        concat!(
            "[clients]\n",
            "menu-claude = claude config_home=$HOME/.menu-claude\n",
            "menu-grok = grok\n",
            "[profiles]\n",
            "menu-supported = menu-claude\n",
            "menu-unsupported = menu-grok\n",
        ),
    );
    let now = ae::time::Timestamp::now().epoch();
    write_settings_claude_quota(&scratch.join(".menu-claude"), 82, now);
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    assert!(
        tmux(
            &socket,
            &scratch,
            &["split-window", "-d", "-h", "-t", "viewed"],
        )
        .0,
        "the settings target has a left and right pane"
    );

    let full_right = draw_direct_settings_menu(
        &socket,
        &scratch,
        &root,
        &config,
        100,
        true,
        "full-right",
        "Quota for our clients...",
    );
    let full_left = draw_direct_settings_menu(
        &socket,
        &scratch,
        &root,
        &config,
        100,
        false,
        "full-left",
        "Quota for our clients...",
    );
    // Width 30 fits the 26-column base menu but not the 32-column entry menu,
    // so the separator carries the exact shortfall: +0r +2c.
    let notice_right = draw_direct_settings_menu(
        &socket,
        &scratch,
        &root,
        &config,
        30,
        true,
        "notice-right",
        "+0r +2c",
    );
    let notice_left = draw_direct_settings_menu(
        &socket,
        &scratch,
        &root,
        &config,
        30,
        false,
        "notice-left",
        "+0r +2c",
    );

    assert!(
        String::from_utf8_lossy(&full_right.raw).contains("Quota for our clients..."),
        "the 100-column branch is full: {:?}",
        full_right.raw
    );
    let notice_text = String::from_utf8_lossy(&notice_right.raw);
    assert!(
        notice_text.contains("+0r") && notice_text.contains("+2c"),
        "the 30-column branch is the quota notice with its exact shortfall: {notice_text:?}"
    );
    assert_direct_menu_geometry(full_right.geometry, 100, 32);
    assert_direct_menu_geometry(notice_right.geometry, 30, 26);
    assert_direct_menu_geometry(full_left.geometry, 100, 32);
    assert_direct_menu_geometry(notice_left.geometry, 30, 26);
}

#[test]
fn quota_dialog_draws_centred_with_indented_windows_from_either_split_pane() {
    let scratch = scratch("quota-dialog-centred");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so dialog centring cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_settings_config(&project, &config);
    write_settings_quota_overlay(
        &project,
        concat!(
            "[clients]\n",
            "menu-claude = claude config_home=$HOME/.menu-claude\n",
            "menu-grok = grok\n",
            "[profiles]\n",
            "menu-supported = menu-claude\n",
            "menu-unsupported = menu-grok\n",
        ),
    );
    let now = ae::time::Timestamp::now().epoch();
    write_settings_claude_quota(&scratch.join(".menu-claude"), 82, now);
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    assert!(
        tmux(
            &socket,
            &scratch,
            &["split-window", "-d", "-h", "-t", "viewed"],
        )
        .0,
        "the dialog target has a left and right pane"
    );

    let dialog_right =
        draw_direct_quota_dialog(&socket, &scratch, &root, &config, 100, true, "right");
    let dialog_left =
        draw_direct_quota_dialog(&socket, &scratch, &root, &config, 100, false, "left");
    for (side, drawn) in [("right", &dialog_right), ("left", &dialog_left)] {
        let text = String::from_utf8_lossy(&drawn.raw);
        assert!(text.contains("claude/menu-claude"), "{side}: {text:?}");
        assert!(
            text.contains("  session 5h | 82%"),
            "{side} keeps the two-space window indent in raw terminal bytes: {text:?}"
        );
        assert!(text.contains("unsupported"), "{side}: {text:?}");
        assert!(text.contains("Close"), "{side}: {text:?}");
        assert_direct_dialog_centred(drawn.geometry, 100);
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real replaced-client, replaced-server and malformed-identity refusal story"
)]
// TEST DEBT, stated not hidden: every pid/start/session mutation here happens
// BEFORE process invocation, so moving prove_quota_dialog_clicker back to the
// build round leaves this test GREEN. It proves the reproof EXISTS; it does not
// prove it sits at the final boundary. The missing test is a deterministic
// after-build/before-proof interleaving — at minimum switching the same client
// session between the two observations inside one invocation.
fn quota_dialog_reproves_client_server_and_session_before_drawing() {
    let scratch = scratch("quota-dialog-identity");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so dialog identity cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    // A real quota source behind the dialog: without one the unavailable
    // fallback menu is small enough to fit the shrunk client below, and a
    // refusal test would draw instead.
    let root = scratch.join("state");
    let config = scratch.join("config");
    assert!(
        fs::write(
            &config,
            "[profiles]\nidle = \"sleep 600\"\nmenu-supported = menu-claude\n\n[roster]\nlead = idle\norchestrator = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n[clients]\nmenu-claude = claude config_home=$HOME/.menu-claude\n",
        )
        .is_ok()
    );
    assert!(fs::create_dir_all(root.join("sessions")).is_ok());
    write_settings_claude_quota(
        &scratch.join(".menu-claude"),
        82,
        ae::time::Timestamp::now().epoch(),
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "new-session",
                "-d",
                "-s",
                "forrest",
                "-x",
                "80",
                "-y",
                "24",
                "sleep 600"
            ]
        )
        .0
    );
    let client = nested_client(&socket, &scratch, "forrest", "identity-viewer");
    let caller_pane = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-c", &client, "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    let real = dialog_identity(&socket, &scratch, "forrest", &client);
    let value = |flag: &str| {
        real.iter()
            .skip_while(|word| word.as_str() != flag)
            .nth(1)
            .unwrap_or_else(|| panic!("captured {flag}"))
            .clone()
    };
    let forged = |flag: &str, replacement: String| {
        let mut tail = real.clone();
        let slot = tail
            .iter()
            .position(|word| word == flag)
            .unwrap_or_else(|| panic!("captured {flag}"));
        tail[slot + 1] = replacement;
        tail
    };
    // A decimal-but-wrong pid is a REPLACEMENT, not a typo: refused after the
    // client resolves, never drawn for.
    let replaced_client = forged("--client-pid", format!("{}0", value("--client-pid")));
    let output = quota_dialog_invocation_command(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &replaced_client,
    )
    .output()
    .expect("the replaced-client invocation runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("client was replaced"),
        "replaced client: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replaced_server = forged("--server-pid", format!("{}0", value("--server-pid")));
    let output = quota_dialog_invocation_command(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &replaced_server,
    )
    .output()
    .expect("the replaced-server invocation runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("server was replaced"),
        "replaced server: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A non-decimal pid never reaches the server: the grammar refuses it.
    let malformed = forged("--client-pid", "nope".to_owned());
    let output = quota_dialog_invocation_command(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &malformed,
    )
    .output()
    .expect("the malformed invocation runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("is not a decimal"),
        "malformed pid: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A replaced server that kept its pid but not its start time is still a
    // replacement: the start comparison is load-bearing, not decorative.
    let restarted = forged("--server-start", format!("{}0", value("--server-start")));
    let output = quota_dialog_invocation_command(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &restarted,
    )
    .output()
    .expect("the restarted-server invocation runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("server was replaced"),
        "restarted server: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The same client on another session reads another overlay: switching
    // session between the menu and the continuation refuses, never draws for
    // the captured session.
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "new-session",
                "-d",
                "-s",
                "second",
                "-x",
                "80",
                "-y",
                "24",
                "sleep 600"
            ]
        )
        .0
    );
    let captured = dialog_identity(&socket, &scratch, "forrest", &client);
    assert!(
        tmux(
            &socket,
            &scratch,
            &["switch-client", "-c", &client, "-t", "second"]
        )
        .0,
        "the client switches session after the capture"
    );
    let output =
        quota_dialog_invocation_command(&socket, &scratch, &root, &config, &caller_pane, &captured)
            .output()
            .expect("the switched-session invocation runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("switched session"),
        "switched session: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A well-formed but vanished client reports and draws nothing.
    let mut vanished = real.clone();
    let slot = vanished
        .iter()
        .position(|word| word == "--client")
        .expect("captured --client");
    vanished[slot + 1] = "/dev/ttys000-vanished".to_owned();
    let output =
        quota_dialog_invocation_command(&socket, &scratch, &root, &config, &caller_pane, &vanished)
            .output()
            .expect("the vanished-client invocation runs");
    assert_eq!(output.status.code(), Some(1));
    // The fit proof reads the LIVE dimensions from the proof round: shrink the
    // client's own terminal after every capture so far, then re-capture. (The
    // client follows the attach terminal's window, not the viewed session's.)
    //
    // TEST DEBT, stated not hidden: the shrink lands BEFORE invocation, so the
    // build round and the proof round both see 40x8; reverting the fit check to
    // the build-round dimensions stays GREEN. It proves the fit check runs; it
    // does not prove it reads the PROOF-round dimensions. The missing test
    // resizes after the build observation and before the proof, with the old
    // build dimensions RED and the final live dimensions GREEN.
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "resize-window",
                "-t",
                "identity-viewer",
                "-x",
                "40",
                "-y",
                "8"
            ]
        )
        .0
    );
    wait_for(
        "shrunk dialog client",
        || {
            tmux(
                &socket,
                &scratch,
                &[
                    "display-message",
                    "-p",
                    "-c",
                    &client,
                    "#{client_width}x#{client_height}",
                ],
            )
            .1
        },
        |seen| seen.trim() == "40x8",
    );
    // Proof precedes fit: a replaced clicker on a too-small client reports the
    // replacement, not the size. (Fit-first code answers "quota needs" here.)
    let small = dialog_identity(&socket, &scratch, "second", &client);
    let replaced_small = {
        let mut tail = small.clone();
        let slot = tail
            .iter()
            .position(|word| word == "--client-pid")
            .expect("captured --client-pid");
        tail[slot + 1] = format!("{}0", tail[slot + 1]);
        tail
    };
    let output = quota_dialog_invocation_command(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &replaced_small,
    )
    .output()
    .expect("the replaced small-client invocation runs");
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("was replaced"), "replacement first: {error}");
    assert!(
        !error.contains("quota needs"),
        "no size report first: {error}"
    );
    // A too-small client reports its live size and draws nothing.
    let output =
        quota_dialog_invocation_command(&socket, &scratch, &root, &config, &caller_pane, &small)
            .output()
            .expect("the small-client invocation runs");
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("this terminal is 40x8; quota needs"),
        "live size reported: {error}"
    );
    let viewer = tmux(
        &socket,
        &scratch,
        &["capture-pane", "-p", "-t", "identity-viewer"],
    )
    .1;
    assert!(
        !viewer.contains("quota for our clients"),
        "no refusal drew a dialog: {viewer}"
    );
}

#[test]
fn competing_menu_markers_never_leave_both_options_set() {
    let scratch = scratch("menu-marker-race");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the marker queue cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    assert!(
        tmux(
            &socket,
            &scratch,
            &["new-session", "-d", "-s", "viewed", "sleep 600"]
        )
        .0
    );
    let session_id = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", "viewed", "#{session_id}"],
    )
    .1
    .trim()
    .to_owned();
    let server = ServerId::Selected(Selector::Socket(socket.clone()));
    let picker_race = scratch.join("picker-race");
    let settings_race = scratch.join("settings-race");
    assert!(fs::create_dir_all(&picker_race).is_ok());
    assert!(fs::create_dir_all(&settings_race).is_ok());
    for round in 0..64 {
        let picker = ae::tmux::replace_session_option_args(
            &server,
            &session_id,
            ae::theme::SETTINGS_OPEN_OPTION,
            ae::theme::MENU_OPEN_OPTION,
            &format!("picker-{round}"),
        );
        let settings = ae::tmux::replace_session_option_args(
            &server,
            &session_id,
            ae::theme::MENU_OPEN_OPTION,
            ae::theme::SETTINGS_OPEN_OPTION,
            &format!("settings-{round}"),
        );
        let barrier = std::sync::Barrier::new(3);
        let (picker_ok, settings_ok) = std::thread::scope(|scope| {
            let picker_run = scope.spawn(|| {
                barrier.wait();
                run_tmux(&picker, &picker_race).0
            });
            let settings_run = scope.spawn(|| {
                barrier.wait();
                run_tmux(&settings, &settings_race).0
            });
            barrier.wait();
            (
                picker_run.join().expect("picker marker command"),
                settings_run.join().expect("settings marker command"),
            )
        });
        assert!(picker_ok && settings_ok, "round {round}");
        let picker = picker_marker(&socket, &scratch, "viewed");
        let settings = settings_marker(&socket, &scratch, "viewed");
        assert_ne!(
            picker.is_empty(),
            settings.is_empty(),
            "round {round}: {picker:?} {settings:?}"
        );
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real two-client active-menu and marker lifecycle story"
)]
fn a_picker_on_a_second_client_takes_the_only_marker_while_settings_remains_active() {
    let scratch = scratch("active-second-menu");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so an active second menu cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_settings_config(&project, &config);
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    let clicked = nested_client(&socket, &scratch, "viewed", "active-second-viewer");
    let picker_client = nested_client(&socket, &scratch, "viewed", "active-picker-viewer");
    let caller_pane = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", "viewed", "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    let selected_style = format!(
        "#[bg={} fg={}]",
        ae::theme::Palette::DARCULA.selected,
        ae::theme::Palette::DARCULA.selected_ink,
    );
    let settings_selected = format!("#[range=user|ae-settings]{selected_style} ⚙ #[norange]");
    let picker_selected = format!("#[range=user|ae]{selected_style}");

    std::thread::scope(|scope| {
        let settings_run = scope.spawn(|| {
            settings_invocation(
                &socket,
                &scratch,
                &root,
                &config,
                &caller_pane,
                &clicked,
                None,
            )
        });
        let settings_menu = wait_for(
            "active settings menu",
            || {
                tmux(
                    &socket,
                    &scratch,
                    &["capture-pane", "-p", "-t", "active-second-viewer"],
                )
                .1
            },
            |seen| seen.contains("ae settings") && seen.contains("orchestrator"),
        );
        assert!(settings_menu.contains("ae settings"), "{settings_menu}");
        assert!(
            settings_marker(&socket, &scratch, "viewed")
                .parse::<i64>()
                .is_ok()
        );
        assert!(picker_marker(&socket, &scratch, "viewed").is_empty());
        let settings_status = rendered_status(&socket, &scratch, &clicked);
        assert!(
            settings_status.contains(&settings_selected),
            "{settings_status}"
        );
        assert!(
            !settings_status.contains(&picker_selected),
            "{settings_status}"
        );

        let picker_driver = scope.spawn(|| {
            let menu = wait_for(
                "picker on the second client while settings remains active",
                || {
                    tmux(
                        &socket,
                        &scratch,
                        &["capture-pane", "-p", "-t", "active-picker-viewer"],
                    )
                    .1
                },
                picker_is_open,
            );
            let settings_pane = tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "active-second-viewer"],
            )
            .1;
            let settings = settings_marker(&socket, &scratch, "viewed");
            let picker = picker_marker(&socket, &scratch, "viewed");
            let status = rendered_status(&socket, &scratch, &picker_client);
            assert!(
                tmux(
                    &socket,
                    &scratch,
                    &["send-keys", "-t", "active-picker-viewer", "Escape"],
                )
                .0
            );
            (menu, settings_pane, settings, picker, status)
        });
        let picker_output = picker_invocation(
            &socket,
            &scratch,
            &root,
            &config,
            &caller_pane,
            &picker_client,
        );
        let (picker_menu, settings_pane, settings, picker, status) =
            picker_driver.join().expect("active second-menu driver");
        assert!(
            tmux(
                &socket,
                &scratch,
                &["send-keys", "-t", "active-second-viewer", "Escape"],
            )
            .0
        );
        let settings_output = settings_run.join().expect("active settings invocation");
        assert_eq!(
            settings_output.status.code(),
            Some(0),
            "{settings_output:?}"
        );
        assert_eq!(picker_output.status.code(), Some(0), "{picker_output:?}");
        assert!(picker_is_open(&picker_menu), "{picker_menu}");
        assert!(settings_pane.contains("ae settings"), "{settings_pane}");
        assert!(settings.is_empty(), "{settings:?}");
        assert!(picker.parse::<i64>().is_ok(), "{picker:?}");
        assert!(status.contains(&picker_selected), "{status}");
        assert!(!status.contains(&settings_selected), "{status}");
    });
}

#[test]
fn settings_draw_failure_retracts_its_marker_on_a_surviving_session() {
    let scratch = scratch("settings-draw-failure");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so settings draw failure cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_settings_config(&project, &config);
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    let clicked = nested_client(&socket, &scratch, "viewed", "draw-failure-viewer");
    let caller_pane = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", "viewed", "#{pane_id}"],
    )
    .1
    .trim()
    .to_owned();
    // Detach only after the atomic marker publish. The session survives with
    // its marker set, while display-menu deterministically loses its client.
    let hook = format!("if-shell -F '#{{@ae_settings_open}}' 'detach-client -t {clicked}' ''");
    assert!(
        tmux(
            &socket,
            &scratch,
            &["set-hook", "-g", "after-set-option", &hook],
        )
        .0
    );
    let output = settings_invocation(
        &socket,
        &scratch,
        &root,
        &config,
        &caller_pane,
        &clicked,
        None,
    );
    assert_eq!(output.status.code(), Some(i32::from(ae::EXIT_UNAVAILABLE)));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("tmux refused to draw settings"),
        "{output:?}"
    );
    assert!(settings_marker(&socket, &scratch, "viewed").is_empty());
    assert!(picker_marker(&socket, &scratch, "viewed").is_empty());
    assert!(
        !tmux(&socket, &scratch, &["list-clients", "-F", "#{client_name}"])
            .1
            .lines()
            .any(|name| name == clicked),
        "the hook removed the exact client before draw"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one real settings-range geometry, render, click and Cancel story"
)]
fn settings_range_measures_renders_clicks_and_cancels_on_the_exact_client() {
    let scratch = scratch("settings-real-range");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the settings range cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    write_settings_config(&project, &config);
    launch_ae_session(&socket, &scratch, &root, &project, &config, "viewed");
    let clicked = nested_client(&socket, &scratch, "viewed", "settings-clicked");
    let untouched = nested_client(&socket, &scratch, "viewed", "settings-untouched");

    let bare = measured_cursor_width(&socket, &scratch, "width-bare-gear", "printf '⚙'; sleep 10");
    let ascii = measured_cursor_width(&socket, &scratch, "width-ascii", "printf '*'; sleep 10");
    let emoji = measured_cursor_width(
        &socket,
        &scratch,
        "width-emoji-gear",
        "printf '⚙️'; sleep 10",
    );
    eprintln!(
        "tmux settings width receipt: bare U+2699={bare}|ASCII *={ascii}|U+2699+FE0F={emoji}"
    );
    assert_eq!(
        (bare.as_str(), ascii.as_str(), emoji.as_str()),
        ("1", "1", "2")
    );
    let icons_format = ae::theme::status_line_one(&ae::theme::Look::DEFAULT);
    assert!(icons_format.contains(
        "#[range=user|ae-settings]#{?@ae_settings_open,#[bg=#214283 fg=#A9B7C6],} ⚙ #[norange]"
    ));
    assert!(
        !icons_format.contains('\u{fe0f}'),
        "settings uses bare U+2699"
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "viewed",
                ae::theme::VERSION_OPTION,
                "ae 2099.1.2",
            ],
        )
        .0
    );
    let wide = rendered_status(&socket, &scratch, &clicked);
    assert!(
        wide.contains("#[range=user|ae-settings] ⚙ #[norange]"),
        "{wide}"
    );
    assert!(!wide.contains("2099.1.2"), "{wide}");

    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-qu",
                "-t",
                "viewed",
                ae::theme::VERSION_OPTION
            ],
        )
        .0
    );
    let missing = rendered_status(&socket, &scratch, &clicked);
    assert!(
        missing.contains("#[range=user|ae-settings] ⚙ #[norange]"),
        "{missing}"
    );
    assert!(!missing.contains("2099.1.2"), "{missing}");

    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "resize-window",
                "-t",
                "settings-clicked",
                "-x",
                "80",
                "-y",
                "40"
            ],
        )
        .0
    );
    wait_for(
        "narrow settings client",
        || {
            tmux(
                &socket,
                &scratch,
                &["display-message", "-p", "-c", &clicked, "#{client_width}"],
            )
            .1
        },
        |seen| seen.trim() == "80",
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "viewed",
                ae::theme::VERSION_OPTION,
                "ae 2099.1.2",
            ],
        )
        .0
    );
    let narrow = rendered_status(&socket, &scratch, &clicked);
    assert!(
        narrow.contains("#[range=user|ae-settings] ⚙ #[norange]"),
        "{narrow}"
    );
    assert!(!narrow.contains("2099.1.2"), "{narrow}");

    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "resize-window",
                "-t",
                "settings-clicked",
                "-x",
                "140",
                "-y",
                "40"
            ],
        )
        .0
    );
    wait_for(
        "wide ASCII settings client",
        || {
            tmux(
                &socket,
                &scratch,
                &["display-message", "-p", "-c", &clicked, "#{client_width}"],
            )
            .1
        },
        |seen| seen.trim() == "140",
    );
    let ascii_format = ae::theme::status_line_one(&ae::theme::Look::read("off", "", "", ""));
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "viewed",
                "status-format[1]",
                &ascii_format
            ],
        )
        .0
    );
    let ascii = rendered_status(&socket, &scratch, &clicked);
    assert!(
        ascii.contains("#[range=user|ae-settings] * #[norange]"),
        "{ascii}"
    );
    assert!(!ascii.contains("2099.1.2"), "{ascii}");

    // Put only the real range at a deterministic coordinate while retaining
    // the launch-installed bindings. Left click Starts; right click reaches
    // Pause confirmation, whose Cancel must preserve the target exactly.
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "viewed",
                "status-format[1]",
                "#[range=user|ae-settings] ⚙ #[norange]",
            ],
        )
        .0
    );
    click_status(&socket, &scratch, "settings-clicked", &clicked, 0, 1);
    wait_for(
        "settings from real left click",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "settings-clicked"],
            )
            .1
        },
        |seen| seen.contains("ae 2099.1.2 settings") && seen.contains("Start orchestrator"),
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", "settings-clicked", "s"]
        )
        .0
    );
    let role_dir = root.join("sessions/orchestrator");
    wait_for(
        "orchestrator after left-click Start",
        || {
            format!(
                "{}|{:?}",
                tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0,
                ae::meta::meta_agent_role(&meta_bytes(&role_dir))
            )
        },
        |seen| seen == "true|Role",
    );
    wait_for(
        "left-click Start completion",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "settings-clicked"],
            )
            .1
        },
        |seen| seen.contains("Started orchestrator without switching this client."),
    );
    let before_cancel = meta_bytes(&role_dir);

    click_status(&socket, &scratch, "settings-clicked", &clicked, 2, 3);
    wait_for(
        "settings Pause from real right click",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "settings-clicked"],
            )
            .1
        },
        |seen| seen.contains("ae 2099.1.2 settings") && seen.contains("Pause orchestrator"),
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", "settings-clicked", "p"]
        )
        .0
    );
    wait_for(
        "Pause confirmation",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "settings-clicked"],
            )
            .1
        },
        |seen| seen.contains("Pause orchestrator 'orchestrator'?") && seen.contains("PRESERVED"),
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", "settings-clicked", "c"]
        )
        .0
    );
    wait_for(
        "Pause cancellation",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "settings-clicked"],
            )
            .1
        },
        |seen| !seen.contains("Pause orchestrator 'orchestrator'?"),
    );
    assert!(tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0);
    assert_eq!(meta_bytes(&role_dir), before_cancel);
    let other = tmux(
        &socket,
        &scratch,
        &["capture-pane", "-p", "-t", "settings-untouched"],
    )
    .1;
    assert!(
        !other.contains("ae settings") && !other.contains("Pause orchestrator"),
        "{other}"
    );

    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "resize-window",
                "-t",
                "settings-clicked",
                "-x",
                "20",
                "-y",
                "5"
            ],
        )
        .0
    );
    wait_for(
        "tiny settings client",
        || {
            tmux(
                &socket,
                &scratch,
                &[
                    "display-message",
                    "-p",
                    "-c",
                    &clicked,
                    "#{client_width}x#{client_height}",
                ],
            )
            .1
        },
        |seen| seen.trim() == "20x5",
    );
    click_status(&socket, &scratch, "settings-clicked", &clicked, 0, 2);
    let tiny = wait_for(
        "tiny settings refusal",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "settings-clicked"],
            )
            .1
        },
        |seen| seen.contains("this terminal is 20x"),
    );
    assert!(tiny.contains("this terminal is 20x"), "{tiny}");
    assert!(tmux(&socket, &scratch, &["has-session", "-t", "=orchestrator"]).0);
    assert_eq!(meta_bytes(&role_dir), before_cancel);
    assert!(!untouched.is_empty());
}

/// Launch two sessions with one real client pair, then prove a right-click on
/// the product's installed `ae` status range opens the picker on that client.
/// Paths are inputs to the actual checkout launch and tmux binding, not argv
/// assertions. Each case owns and tears down its private server.
#[allow(
    clippy::too_many_lines,
    reason = "one real-click punctuation-path witness with same-pane client isolation"
)]
fn assert_menu_picker_case(tag: &str, root_name: &str, config_name: &str) {
    let scratch = scratch(tag);
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so a status-range click cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join(root_name);
    let project = scratch.join("project");
    let config = scratch.join(config_name);
    assert!(fs::create_dir_all(&project).is_ok());
    assert!(
        fs::write(
            &config,
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok()
    );
    for session in ["fleet-a", "fleet-b"] {
        launch_ae_session(&socket, &scratch, &root, &project, &config, session);
    }
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "fleet-a",
                "status-format[1]",
                "#[range=user|ae] ≡ #[norange]",
            ],
        )
        .0
    );
    let clicked = nested_client(&socket, &scratch, "fleet-a", "clicked-viewer");
    let untouched = nested_client(&socket, &scratch, "fleet-a", "untouched-viewer");
    let clients = wait_for(
        "two clients on fleet-a's pane",
        || {
            tmux(
                &socket,
                &scratch,
                &[
                    "list-clients",
                    "-F",
                    "#{client_name}|#{client_session}|#{pane_id}",
                ],
            )
            .1
        },
        |seen| {
            seen.lines()
                .filter(|line| line.contains("|fleet-a|"))
                .count()
                == 2
        },
    );
    let clicked_pane = clients
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{clicked}|fleet-a|")))
        .unwrap_or_else(|| panic!("clicked client: {clients}"));
    assert!(
        clients.contains(&format!("{untouched}|fleet-a|{clicked_pane}")),
        "both clients must watch the SAME pane: {clients}"
    );

    click_status(&socket, &scratch, "clicked-viewer", &clicked, 2, 2);
    let menu = wait_for(
        "the fleet menu from a real menu-button click",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "clicked-viewer"],
            )
            .1
        },
        |seen| picker_is_open(seen) && seen.contains("fleet-b"),
    );
    assert!(
        menu.contains("fleet-a") && menu.contains("fleet-b"),
        "{menu}"
    );
    let other = tmux(
        &socket,
        &scratch,
        &["capture-pane", "-p", "-t", "untouched-viewer"],
    )
    .1;
    assert!(
        !picker_is_open(&other),
        "menu leaked to other client: {other}"
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", "clicked-viewer", "q"]
        )
        .0
    );
    wait_for(
        "the fleet menu to close",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "clicked-viewer"],
            )
            .1
        },
        |seen| !picker_is_open(seen),
    );
}

/// Plain paths establish the control; comma paths exercise the actual binding's
/// quoting while preserving the two-client proof.
#[test]
fn right_clicking_menu_picker_survives_comma_paths() {
    assert_menu_picker_case("status-picker-control", "custom-state", "nondefault.config");
    assert_menu_picker_case("status-picker-comma", "state,comma", "config,comma");
}

/// Plain paths establish the control; closing-brace paths exercise the actual
/// binding's quoting while preserving the two-client proof.
#[test]
fn right_clicking_menu_picker_survives_closing_brace_paths() {
    assert_menu_picker_case(
        "status-picker-control-brace",
        "custom-state",
        "nondefault.config",
    );
    assert_menu_picker_case("status-picker-brace", "state}brace", "config}brace");
}

/// The mnemonic binding carries punctuation-heavy checkout namespace words
/// through its one `run-shell` format layer and targets only the client that
/// pressed it.
#[test]
fn prefix_a_opens_the_picker_on_only_its_nested_client_with_punctuation_paths() {
    let scratch = scratch("hotkey-punctuation");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the picker hotkey cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state,comma}brace");
    let project = scratch.join("project");
    let config = scratch.join("config,comma}brace");
    assert!(fs::create_dir_all(&project).is_ok());
    assert!(
        fs::write(
            &config,
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok()
    );
    for session in ["fleet-a", "fleet-b"] {
        launch_ae_session(&socket, &scratch, &root, &project, &config, session);
    }
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "fleet-b",
                ae::tmux::BRANCH_OPTION,
                "feat | menu\nbad",
            ],
        )
        .0
    );

    let clicked = nested_client(&socket, &scratch, "fleet-a", "clicked-viewer");
    let untouched = nested_client(&socket, &scratch, "fleet-a", "untouched-viewer");
    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", "clicked-viewer", "C-b", "a"],
        )
        .0
    );
    let menu = wait_for(
        "the fleet menu from prefix a",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "clicked-viewer"],
            )
            .1
        },
        |seen| picker_is_open(seen) && seen.contains("fleet-b"),
    );
    assert!(menu.contains("prefix a"), "{menu}");
    assert!(menu.contains("feat  menubad"), "sanitized branch: {menu}");
    assert_eq!(
        tmux(
            &socket,
            &scratch,
            &[
                "show-option",
                "-qv",
                "-t",
                "fleet-b",
                ae::tmux::BRANCH_OPTION,
            ],
        )
        .1,
        "feat | menu\nbad\n",
        "the picker reader must not rewrite the raw branch fact"
    );
    let other = tmux(
        &socket,
        &scratch,
        &["capture-pane", "-p", "-t", "untouched-viewer"],
    )
    .1;
    assert!(
        !picker_is_open(&other),
        "menu leaked to {untouched}: {other}"
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", "clicked-viewer", "q"],
        )
        .0
    );
    assert!(!clicked.is_empty());
}

fn nested_client(socket: &Path, scratch: &Path, session: &str, viewer: &str) -> String {
    let attach = format!(
        "env -u TMUX tmux -S {} attach -t {session}",
        socket.display()
    );
    assert!(
        tmux(
            socket,
            scratch,
            &[
                "new-session",
                "-d",
                "-s",
                viewer,
                "-x",
                "140",
                "-y",
                "40",
                &attach,
            ],
        )
        .0,
        "the nested client in {viewer}"
    );
    let tty = tmux(
        socket,
        scratch,
        &["display-message", "-p", "-t", viewer, "#{pane_tty}"],
    )
    .1
    .trim()
    .to_owned();
    wait_for(
        &format!("client {tty}"),
        || tmux(socket, scratch, &["list-clients", "-F", "#{client_name}"]).1,
        |seen| seen.lines().any(|line| line == tty),
    );
    tty
}

fn click_status(socket: &Path, scratch: &Path, viewer: &str, client: &str, button: u8, x: usize) {
    std::thread::sleep(Duration::from_millis(600));
    let height_text = tmux(
        socket,
        scratch,
        &["display-message", "-p", "-c", client, "#{client_height}"],
    )
    .1;
    let height = height_text
        .trim()
        .parse::<usize>()
        .unwrap_or_else(|_| panic!("client height: {height_text:?}"));
    mouse_event(socket, scratch, viewer, button, x, height, 'M');
    mouse_event(socket, scratch, viewer, button, x, height, 'm');
}

fn mouse_event(
    socket: &Path,
    scratch: &Path,
    viewer: &str,
    button: u8,
    x: usize,
    y: usize,
    suffix: char,
) {
    let event = format!("\u{1b}[<{button};{x};{y}{suffix}");
    assert!(
        tmux(socket, scratch, &["send-keys", "-t", viewer, "-l", &event],).0,
        "button {button}{suffix} on {viewer} at {x},{y}"
    );
}

/// This starts from the binding ae installed, feeds a real SGR mouse event to
/// the menu range, and observes the resulting menu on the exact client.
/// A second client watches the same pane: `$TMUX_PANE` alone cannot distinguish
/// them, which is the regression the explicit `--client` contract prevents.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end two-client status mouse story"
)]
fn clicking_the_menu_range_opens_the_fleet_and_a_row_lands_on_the_lead() {
    let scratch = scratch("status-picker");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so a status-range click cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("custom-state");
    let project = scratch.join("project");
    let config = scratch.join("nondefault.config");
    assert!(fs::create_dir_all(&project).is_ok());
    assert!(
        fs::write(
            &config,
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok()
    );
    for session in ["fleet-a", "fleet-b"] {
        launch_ae_session(&socket, &scratch, &root, &project, &config, session);
    }

    // Put the real menu range at a deterministic coordinate while keeping
    // the launch-installed binding and the product's two-line status shape.
    let status_set = tmux(
        &socket,
        &scratch,
        &[
            "set-option",
            "-t",
            "fleet-a",
            "status-format[1]",
            "#[range=user|ae] ≡ #[norange]",
        ],
    );
    assert!(status_set.0, "set deterministic status");
    let clicked = nested_client(&socket, &scratch, "fleet-a", "clicked-viewer");
    let untouched = nested_client(&socket, &scratch, "fleet-a", "untouched-viewer");
    let clients = wait_for(
        "two clients on fleet-a's pane",
        || {
            tmux(
                &socket,
                &scratch,
                &[
                    "list-clients",
                    "-F",
                    "#{client_name}|#{client_session}|#{pane_id}",
                ],
            )
            .1
        },
        |seen| {
            seen.lines()
                .filter(|line| line.contains("|fleet-a|"))
                .count()
                == 2
        },
    );
    let clicked_pane = clients
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{clicked}|fleet-a|")))
        .unwrap_or_else(|| panic!("clicked client: {clients}"));
    let clicked_pane = clicked_pane.to_owned();
    assert!(
        clients.contains(&format!("{untouched}|fleet-a|{clicked_pane}")),
        "both clients must watch the SAME pane: {clients}"
    );

    // Mouse-aware servers open on press and must survive the trailing release.
    // The 3.4 floor opens on release, after the event that would close its
    // keyboard-driven menu, and therefore must not also dispatch on press.
    std::thread::sleep(Duration::from_millis(600));
    let height_text = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-c", &clicked, "#{client_height}"],
    )
    .1;
    let height = height_text
        .trim()
        .parse::<usize>()
        .unwrap_or_else(|_| panic!("client height: {height_text:?}"));
    let menu_mouse = menu_mouse(&socket);
    mouse_event(&socket, &scratch, "clicked-viewer", 0, 2, height, 'M');
    let menu = if menu_mouse {
        let menu = wait_for(
            "the fleet menu from a real menu-button press",
            || {
                tmux(
                    &socket,
                    &scratch,
                    &["capture-pane", "-p", "-t", "clicked-viewer"],
                )
                .1
            },
            |seen| picker_is_open(seen) && seen.contains("fleet-b"),
        );
        mouse_event(&socket, &scratch, "clicked-viewer", 0, 2, height, 'm');
        menu
    } else {
        std::thread::sleep(Duration::from_millis(250));
        let held = tmux(
            &socket,
            &scratch,
            &["capture-pane", "-p", "-t", "clicked-viewer"],
        )
        .1;
        assert!(
            !picker_is_open(&held),
            "tmux 3.4 dispatched the picker before release: {held}"
        );
        mouse_event(&socket, &scratch, "clicked-viewer", 0, 2, height, 'm');
        wait_for(
            "the fleet menu from a real menu-button release",
            || {
                tmux(
                    &socket,
                    &scratch,
                    &["capture-pane", "-p", "-t", "clicked-viewer"],
                )
                .1
            },
            |seen| picker_is_open(seen) && seen.contains("fleet-b"),
        )
    };
    assert!(
        menu.contains("fleet-a") && menu.contains("fleet-b"),
        "{menu}"
    );
    std::thread::sleep(Duration::from_secs(1));
    let menu = tmux(
        &socket,
        &scratch,
        &["capture-pane", "-p", "-t", "clicked-viewer"],
    )
    .1;
    assert!(
        picker_is_open(&menu) && menu.contains("fleet-b"),
        "the menu closed after the status-button click: {menu}"
    );
    let other = tmux(
        &socket,
        &scratch,
        &["capture-pane", "-p", "-t", "untouched-viewer"],
    )
    .1;
    assert!(
        !picker_is_open(&other),
        "menu leaked to other client: {other}"
    );
    let fleet_b_main = tmux(
        &socket,
        &scratch,
        &[
            "show-options",
            "-v",
            "-t",
            "=fleet-b:",
            ae::theme::MAIN_PANE_OPTION,
        ],
    )
    .1
    .trim()
    .to_owned();
    assert!(fleet_b_main.starts_with('%'), "{fleet_b_main:?}");
    if menu_mouse {
        let (row_y, row_x) = menu
            .lines()
            .enumerate()
            .find_map(|(y, line)| line.find("fleet-b").map(|x| (y + 1, x + 1)))
            .unwrap_or_else(|| panic!("fleet-b row coordinates: {menu}"));
        // A real pointer moves onto the row before clicking it. In SGR mouse
        // mode, 35 is motion with no button held; tmux uses it to highlight
        // the choice that the following press selects.
        mouse_event(&socket, &scratch, "clicked-viewer", 35, row_x, row_y, 'M');
        mouse_event(&socket, &scratch, "clicked-viewer", 0, row_x, row_y, 'M');
        std::thread::sleep(Duration::from_millis(100));
        mouse_event(&socket, &scratch, "clicked-viewer", 0, row_x, row_y, 'm');
    } else {
        assert!(
            tmux(
                &socket,
                &scratch,
                &["send-keys", "-t", "clicked-viewer", "2"],
            )
            .0,
            "tmux 3.4 chooses the second row by its key"
        );
    }
    let landed = wait_for(
        "the fleet-b lead jump",
        || {
            tmux(
                &socket,
                &scratch,
                &[
                    "list-clients",
                    "-F",
                    "#{client_name}|#{client_session}|#{pane_id}",
                ],
            )
            .1
        },
        |seen| seen.contains(&format!("{clicked}|fleet-b|{fleet_b_main}")),
    );
    assert!(
        landed.contains(&format!("{untouched}|fleet-a|{clicked_pane}")),
        "the other same-pane client moved with the row: {landed}"
    );

    // Context-clicking the same range still opens the same picker.
    assert!(
        tmux(
            &socket,
            &scratch,
            &["switch-client", "-c", &clicked, "-t", "=fleet-a"],
        )
        .0
    );
    wait_for(
        "the clicked client to return to fleet-a",
        || {
            tmux(
                &socket,
                &scratch,
                &["list-clients", "-F", "#{client_name}|#{client_session}"],
            )
            .1
        },
        |seen| seen.contains(&format!("{clicked}|fleet-a")),
    );
    click_status(&socket, &scratch, "clicked-viewer", &clicked, 2, 2);
    wait_for(
        "the fleet menu from a context click",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "clicked-viewer"],
            )
            .1
        },
        |seen| picker_is_open(seen) && seen.contains("fleet-b"),
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", "clicked-viewer", "q"]
        )
        .0
    );
    wait_for(
        "the context-click fleet menu to close",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "clicked-viewer"],
            )
            .1
        },
        |seen| !picker_is_open(seen),
    );

    // The same binding keeps the existing session-tab context menu. This is a
    // real session range click, not a direct display-menu invocation.
    let fleet_a_id = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-t", "fleet-a", "#{session_id}"],
    )
    .1;
    assert!(fleet_a_id.trim().starts_with('$'), "{fleet_a_id:?}");
    assert!(
        tmux(
            &socket,
            &scratch,
            &["split-window", "-d", "-h", "-t", "fleet-a:0"],
        )
        .0
    );
    let before_flip = tmux(
        &socket,
        &scratch,
        &[
            "list-panes",
            "-t",
            "fleet-a:0",
            "-F",
            "#{pane_id}|#{pane_left}",
        ],
    )
    .1;
    let clicked_left = before_flip
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{clicked_pane}|")))
        .unwrap_or_else(|| panic!("clicked pane before flip: {before_flip}"))
        .to_owned();
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "fleet-a",
                "status-format[1]",
                &format!("#[range=session|{}]fleet-a#[norange]", fleet_a_id.trim()),
            ],
        )
        .0
    );
    click_status(&socket, &scratch, "clicked-viewer", &clicked, 2, 2);
    wait_for(
        "the flip menu from a real session-range click",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "clicked-viewer"],
            )
            .1
        },
        |seen| seen.contains("Flip lead/colead panes"),
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &["send-keys", "-t", "clicked-viewer", "f"],
        )
        .0
    );
    wait_for(
        "the flip action to swap the clicked window",
        || {
            tmux(
                &socket,
                &scratch,
                &[
                    "list-panes",
                    "-t",
                    "fleet-a:0",
                    "-F",
                    "#{pane_id}|#{pane_left}",
                ],
            )
            .1
        },
        |seen| {
            seen.lines()
                .find_map(|line| line.strip_prefix(&format!("{clicked_pane}|")))
                .is_some_and(|left| left != clicked_left)
        },
    );
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "fleet-a",
                "status-format[1]",
                "#[range=user|ae] ≡ #[norange]",
            ],
        )
        .0
    );

    // Once that client disappears, the public command refuses it instead of
    // letting tmux choose the other client still watching the same pane.
    assert!(tmux(&socket, &scratch, &["kill-session", "-t", "clicked-viewer"],).0);
    wait_for(
        "the clicked client to vanish",
        || tmux(&socket, &scratch, &["list-clients", "-F", "#{client_name}"]).1,
        |seen| !seen.lines().any(|line| line == clicked),
    );
    let refused = ae()
        .env("HOME", &scratch)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", &config)
        .env("TMUX_TMPDIR", &scratch)
        .env("TMUX", format!("{},0,0", socket.display()))
        .env("TMUX_PANE", clicked_pane)
        .args(["orchestrator", "--popup", "--client", &clicked])
        .output()
        .expect("the picker invocation runs");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert_eq!(refused.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&clicked) && stderr.contains("vanished"),
        "{stderr}"
    );
}

const STATUS_MENU_ACTION: &str = "if-shell -F '##{&&:##{==:##{window_panes},2},##{==:##{window_zoomed_flag},0}}' 'swap-pane -d -s \"{top-left}\" -t \"{bottom-right}\"' 'display-message \"flip needs an unzoomed two-pane window\"'";

/// Two sessions and a real client viewing one, while the other supplies the
/// pane target for the menu.
fn stage_status_menu_target(socket: &Path, main: &Path) -> (String, String) {
    for words in [
        &["new-session", "-d", "-s", "clicked", "-x", "80", "-y", "24"][..],
        &["split-window", "-h", "-t", "clicked"][..],
        &["new-session", "-d", "-s", "viewed", "-x", "80", "-y", "24"][..],
        &["split-window", "-h", "-t", "viewed"][..],
    ] {
        assert!(tmux(socket, main, words).0, "setting up: {words:?}");
    }
    let attach = format!("env -u TMUX tmux -S {} attach -t viewed", socket.display());
    assert!(
        tmux(
            socket,
            main,
            &[
                "new-session",
                "-d",
                "-s",
                "viewer",
                "-x",
                "140",
                "-y",
                "40",
                &attach,
            ],
        )
        .0,
        "the nested client"
    );
    let client = wait_for(
        "a client on viewed",
        || {
            tmux(
                socket,
                main,
                &["list-clients", "-F", "#{client_name}|#{session_name}"],
            )
            .1
        },
        |seen| seen.lines().any(|line| line.ends_with("|viewed")),
    )
    .lines()
    .find_map(|line| line.strip_suffix("|viewed"))
    .unwrap_or_else(|| panic!("the viewed client has a name"))
    .to_owned();
    let clicked_pane = tmux(
        socket,
        main,
        &["list-panes", "-t", "clicked", "-F", "#{pane_id}"],
    )
    .1
    .lines()
    .next()
    .unwrap_or_else(|| panic!("the clicked session has a pane"))
    .to_owned();
    (client, clicked_pane)
}

fn pane_order(socket: &Path, main: &Path, session: &str) -> String {
    tmux(
        socket,
        main,
        &[
            "list-panes",
            "-t",
            session,
            "-F",
            "#{pane_index}|#{pane_id}|#{pane_left}",
        ],
    )
    .1
}

/// `display-menu -t <clicked pane>` supplies the target context to commands
/// chosen from the menu. This is the tmux-side half of the status binding pin:
/// the argv unit test fixes `<clicked pane>` as `{mouse}`, while this arm proves
/// the guarded action changes that window rather than the client's own window.
#[test]
fn a_status_menu_flip_targets_the_clicked_sessions_window() {
    let scratch = scratch("status-target");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!(
            "tmux is not runnable here, so the status menu's target context cannot be proven; \
             install tmux or run this suite where one exists"
        );
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let main = scratch.join("main");
    let watcher = scratch.join("watcher");
    let (client, clicked_pane) = stage_status_menu_target(&socket, &main);
    let clicked_before = pane_order(&socket, &main, "clicked");
    let viewed_before = pane_order(&socket, &main, "viewed");

    std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            wait_for(
                "the flip menu",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                |text| text.contains("Flip lead/colead panes"),
            );
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "f"]).0);
        });
        assert!(
            tmux(
                &socket,
                &main,
                &[
                    "display-menu",
                    "-c",
                    &client,
                    "-t",
                    &clicked_pane,
                    "-T",
                    "#{session_name}",
                    "-x",
                    "C",
                    "-y",
                    "C",
                    "Flip lead/colead panes",
                    "f",
                    STATUS_MENU_ACTION,
                ],
            )
            .0,
            "tmux accepts the status menu action"
        );
        driver.join().expect("the key driver");
    });

    let clicked_after = pane_order(&socket, &main, "clicked");
    let viewed_after = pane_order(&socket, &main, "viewed");
    assert_ne!(
        clicked_after, clicked_before,
        "the clicked session flips: {clicked_before:?}"
    );
    assert_eq!(
        viewed_after, viewed_before,
        "the client's own session does not flip"
    );
}

/// The guard must be evaluated when the menu action runs, not when the menu is
/// drawn. Split the clicked window after the menu is visible, then choose the
/// row: a stale draw-time `2` would still swap panes instead of refusing.
#[test]
fn a_status_menu_flip_refuses_after_clicked_window_grows() {
    let scratch = scratch("status-guard");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!(
            "tmux is not runnable here, so the status menu guard cannot be proven; \
             install tmux or run this suite where one exists"
        );
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let main = scratch.join("main");
    let watcher = scratch.join("watcher");
    let (client, clicked_pane) = stage_status_menu_target(&socket, &main);

    std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            wait_for(
                "the flip menu",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                |text| text.contains("Flip lead/colead panes"),
            );
            // Change the clicked window while display-menu is holding the client.
            assert!(tmux(&socket, &watcher, &["split-window", "-h", "-t", "clicked"]).0);
            let grown = wait_for(
                "the clicked window to grow",
                || pane_order(&socket, &watcher, "clicked"),
                |order| order.lines().count() == 3,
            );
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "f"]).0);
            std::thread::sleep(Duration::from_millis(250));
            let message = tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1;
            (grown, message)
        });
        assert!(
            tmux(
                &socket,
                &main,
                &[
                    "display-menu",
                    "-c",
                    &client,
                    "-t",
                    &clicked_pane,
                    "-T",
                    "#{session_name}",
                    "-x",
                    "C",
                    "-y",
                    "C",
                    "Flip lead/colead panes",
                    "f",
                    STATUS_MENU_ACTION,
                ],
            )
            .0,
            "tmux accepts the status menu action"
        );
        let (grown, message) = driver.join().expect("the key driver");
        let after = pane_order(&socket, &main, "clicked");
        assert!(
            after == grown && message.contains("flip needs an unzoomed two-pane window"),
            "guard must refuse without swapping; before={grown:?}, after={after:?}, message={message:?}"
        );
    });
}

/// Whether the session state a stop must PRESERVE is still on disk.
#[allow(
    clippy::disallowed_methods,
    reason = "a fixture reading its own scratch directory; the capability boundary is about what PRODUCT code may reach"
)]
fn state_kept(dir: &Path) -> bool {
    fs::metadata(dir).is_ok()
}

/// The exact bytes of one session's metadata, for a byte-identical claim.
#[allow(
    clippy::disallowed_methods,
    reason = "a fixture reading its own scratch directory; the capability boundary is about what PRODUCT code may reach"
)]
fn meta_bytes(dir: &Path) -> Vec<u8> {
    fs::read(dir.join("meta")).unwrap_or_default()
}

/// The events one session recorded, or nothing when it recorded none.
#[allow(
    clippy::disallowed_methods,
    reason = "a fixture reading its own scratch directory; the capability boundary is about what PRODUCT code may reach"
)]
fn events_of(dir: &Path) -> String {
    fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default()
}

/// Wait until `session` is gone from `socket`, or give up and report.
fn session_gone(socket: &Path, scratch: &Path, session: &str) -> bool {
    for _ in 0..80 {
        let listed = tmux(socket, scratch, &["list-sessions", "-F", "#{session_name}"]).1;
        if !listed.lines().any(|line| line == session) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// The row the menu overlay drew `needle` on, counted from the top of the
/// captured client.
fn row_of(text: &str, needle: &str) -> usize {
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not on the client: {text}"))
}

/// Open the clicked session's context menu from a real right-click and return
/// what the invoking client then shows.
fn open_context_menu(socket: &Path, scratch: &Path, viewer: &str, client: &str) -> String {
    let height_text = tmux(
        socket,
        scratch,
        &["display-message", "-p", "-c", client, "#{client_height}"],
    )
    .1;
    let height = height_text
        .trim()
        .parse::<usize>()
        .unwrap_or_else(|_| panic!("client height: {height_text:?}"));
    let menu_mouse = menu_mouse(socket);
    mouse_event(socket, scratch, viewer, 2, 2, height, 'M');
    if !menu_mouse {
        std::thread::sleep(Duration::from_millis(250));
    }
    mouse_event(socket, scratch, viewer, 2, 2, height, 'm');
    wait_for(
        "the clicked session's context menu",
        || tmux(socket, scratch, &["capture-pane", "-p", "-t", viewer]).1,
        |seen| seen.contains("Flip lead/colead panes") && seen.contains("Stop session"),
    )
}

/// The whole forward chain, from a real right-click to a stopped session.
///
/// Two ae sessions, and the client is attached to the one that is NOT clicked:
/// every step has to carry the clicked session, never the viewed one. Cancel
/// must leave both alive, and only the confirmed row may stop anything.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end story: click, centred menu, confirm, cancel, confirm again, stop"
)]
fn a_right_click_offers_stop_and_only_a_confirmed_row_stops_the_clicked_session() {
    let scratch = scratch("session-menu-stop");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the session menu chain cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    assert!(fs::create_dir_all(&project).is_ok());
    assert!(
        fs::write(
            &config,
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok()
    );
    for session in ["menu-clicked", "menu-viewed"] {
        launch_ae_session(&socket, &scratch, &root, &project, &config, session);
    }
    let listing = tmux(
        &socket,
        &scratch,
        &["list-sessions", "-F", "#{session_name}|#{session_id}"],
    )
    .1;
    let clicked_id = listing
        .lines()
        .find_map(|line| line.strip_prefix("menu-clicked|"))
        .unwrap_or_else(|| panic!("the clicked session is on the server: {listing}"))
        .to_owned();

    // A deterministic `session` range on the VIEWED session's status line,
    // naming the session that is NOT being viewed.
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "menu-viewed",
                "status-format[1]",
                &format!("#[range=session|{clicked_id}] C #[norange]"),
            ],
        )
        .0,
        "set a deterministic session range"
    );
    // The viewed session gets a second pane: a centred menu must use the
    // client's whole terminal, not the pane the click landed in.
    assert!(tmux(&socket, &scratch, &["split-window", "-t", "menu-viewed"]).0);
    let viewer = "menu-viewer";
    let client = nested_client(&socket, &scratch, "menu-viewed", viewer);
    // A SECOND client on the same session. `$TMUX_PANE` cannot tell the two
    // apart, so only the explicit client carried through the chain can.
    let bystander = "menu-bystander";
    let _ = nested_client(&socket, &scratch, "menu-viewed", bystander);
    std::thread::sleep(Duration::from_millis(600));

    let menu = open_context_menu(&socket, &scratch, viewer, &client);
    assert!(
        menu.contains("menu-clicked"),
        "the menu is titled with the CLICKED session: {menu}"
    );
    let height_text = tmux(
        &socket,
        &scratch,
        &["display-message", "-p", "-c", &client, "#{client_height}"],
    )
    .1;
    let height = height_text
        .trim()
        .parse::<usize>()
        .unwrap_or_else(|_| panic!("client height: {height_text:?}"));
    let flip_row = row_of(&menu, "Flip lead/colead panes");
    assert!(
        flip_row > height / 5 && flip_row < height * 4 / 5,
        "the menu is centred on the client, not parked at the status line: row {flip_row} of {height}\n{menu}"
    );

    // The first row only ASKS. Cancel must leave both sessions alone.
    assert!(tmux(&socket, &scratch, &["send-keys", "-t", viewer, "s"]).0);
    let confirm = wait_for(
        "the confirmation menu",
        || tmux(&socket, &scratch, &["capture-pane", "-p", "-t", viewer]).1,
        |seen| seen.contains("Stop 'menu-clicked' now"),
    );
    assert!(
        confirm.contains("Stop session 'menu-clicked'?") && confirm.contains("Cancel"),
        "the confirmation names the exact session and offers cancel first: {confirm}"
    );
    let other = tmux(&socket, &scratch, &["capture-pane", "-p", "-t", bystander]).1;
    assert!(
        !other.contains("Stop 'menu-clicked' now"),
        "the confirmation reached a client that did not ask for it: {other}"
    );
    let clicked_dir = root.join("sessions").join("menu-clicked");
    let before_meta = meta_bytes(&clicked_dir);
    let before_events = events_of(&clicked_dir);
    assert!(
        !before_meta.is_empty(),
        "the fixture has metadata to compare"
    );

    // CANCEL, then ESCAPE: dismissing the question and answering "no" must be
    // the same act, and neither may touch anything.
    assert!(tmux(&socket, &scratch, &["send-keys", "-t", viewer, "c"]).0);
    std::thread::sleep(Duration::from_secs(2));
    let dismissed = open_context_menu(&socket, &scratch, viewer, &client);
    assert!(dismissed.contains("Stop session"), "{dismissed}");
    assert!(tmux(&socket, &scratch, &["send-keys", "-t", viewer, "s"]).0);
    wait_for(
        "the confirmation menu before Escape",
        || tmux(&socket, &scratch, &["capture-pane", "-p", "-t", viewer]).1,
        |seen| seen.contains("Stop 'menu-clicked' now"),
    );
    assert!(tmux(&socket, &scratch, &["send-keys", "-t", viewer, "Escape"]).0);
    std::thread::sleep(Duration::from_secs(2));

    let listed = tmux(
        &socket,
        &scratch,
        &["list-sessions", "-F", "#{session_name}"],
    )
    .1;
    for session in ["menu-clicked", "menu-viewed"] {
        assert!(
            listed.lines().any(|line| line == session),
            "cancel or escape stopped {session}: {listed}"
        );
    }
    assert!(state_kept(&clicked_dir), "cancel removed session state");
    assert_eq!(
        meta_bytes(&clicked_dir),
        before_meta,
        "cancel and escape must leave the metadata byte for byte"
    );
    assert_eq!(
        events_of(&clicked_dir),
        before_events,
        "cancel and escape must write no event at all"
    );

    // The destructive row, and only it, hands the stop to the lifecycle owner.
    let reopened = open_context_menu(&socket, &scratch, viewer, &client);
    assert!(reopened.contains("Stop session"), "{reopened}");
    assert!(tmux(&socket, &scratch, &["send-keys", "-t", viewer, "s"]).0);
    wait_for(
        "the confirmation menu again",
        || tmux(&socket, &scratch, &["capture-pane", "-p", "-t", viewer]).1,
        |seen| seen.contains("Stop 'menu-clicked' now"),
    );
    assert!(tmux(&socket, &scratch, &["send-keys", "-t", viewer, "S"]).0);
    assert!(
        session_gone(&socket, &scratch, "menu-clicked"),
        "the confirmed session is still running"
    );
    let listed = tmux(
        &socket,
        &scratch,
        &["list-sessions", "-F", "#{session_name}"],
    )
    .1;
    assert!(
        listed.lines().any(|line| line == "menu-viewed"),
        "the session the client was VIEWING must be untouched: {listed}"
    );
    let bystander_saw = tmux(&socket, &scratch, &["capture-pane", "-p", "-t", bystander]).1;
    assert!(
        !bystander_saw.contains("Stopped menu-clicked"),
        "the outcome of one human's answer was broadcast to another client: {bystander_saw}"
    );
    assert!(
        state_kept(&root.join("sessions").join("menu-clicked")),
        "a stop preserves the session's state"
    );
    let events = events_of(&root.join("sessions").join("menu-clicked"));
    assert!(
        events.contains("stop confirmed on the session menu by client"),
        "the human's confirmation is recorded with its provenance: {events}"
    );
}

/// The harder half of the same story: the client is attached to the session it
/// clicks. The `run-shell` job that answers lives on the server, so the stop
/// must still complete after the pane the human was looking at is gone — which
/// is exactly what the detached supervisor is for.
#[test]
fn confirming_the_session_you_are_viewing_still_completes_out_of_pane() {
    let scratch = scratch("session-menu-self");
    if !tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        panic!("tmux is not runnable here, so the self-stop menu chain cannot be proven");
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("state");
    let project = scratch.join("project");
    let config = scratch.join("config");
    assert!(fs::create_dir_all(&project).is_ok());
    assert!(
        fs::write(
            &config,
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok()
    );
    // A second session so the fleet is not emptied by the one being stopped.
    for session in ["self-stopped", "self-other"] {
        launch_ae_session(&socket, &scratch, &root, &project, &config, session);
    }
    let listing = tmux(
        &socket,
        &scratch,
        &["list-sessions", "-F", "#{session_name}|#{session_id}"],
    )
    .1;
    let own_id = listing
        .lines()
        .find_map(|line| line.strip_prefix("self-stopped|"))
        .unwrap_or_else(|| panic!("the session is on the server: {listing}"))
        .to_owned();
    // The clicked range names the session the client is ALREADY viewing.
    assert!(
        tmux(
            &socket,
            &scratch,
            &[
                "set-option",
                "-t",
                "self-stopped",
                "status-format[1]",
                &format!("#[range=session|{own_id}] C #[norange]"),
            ],
        )
        .0
    );
    let viewer = "self-viewer";
    let client = nested_client(&socket, &scratch, "self-stopped", viewer);
    std::thread::sleep(Duration::from_millis(600));

    let menu = open_context_menu(&socket, &scratch, viewer, &client);
    assert!(menu.contains("self-stopped"), "{menu}");
    assert!(tmux(&socket, &scratch, &["send-keys", "-t", viewer, "s"]).0);
    wait_for(
        "the confirmation for the session being viewed",
        || tmux(&socket, &scratch, &["capture-pane", "-p", "-t", viewer]).1,
        |seen| seen.contains("Stop 'self-stopped' now"),
    );
    assert!(tmux(&socket, &scratch, &["send-keys", "-t", viewer, "S"]).0);
    assert!(
        session_gone(&socket, &scratch, "self-stopped"),
        "a session confirmed from inside itself must still be stopped"
    );
    let listed = tmux(
        &socket,
        &scratch,
        &["list-sessions", "-F", "#{session_name}"],
    )
    .1;
    assert!(
        listed.lines().any(|line| line == "self-other"),
        "and only that one: {listed}"
    );
    assert!(
        state_kept(&root.join("sessions").join("self-stopped")),
        "a stop preserves the state of the session it was run from"
    );
    let events = events_of(&root.join("sessions").join("self-stopped"));
    assert!(
        events.contains("stop confirmed on the session menu by client"),
        "the provenance survives the pane that asked: {events}"
    );
}
