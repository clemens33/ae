//! The fleet picker against a REAL tmux server.
//!
//! A pure argv assertion cannot hold tmux's format timing or client focus. This
//! arm draws the menu, chooses rows, and proves both the ordinary lead-pane jump
//! and the execution-time guard when that pane moves or vanishes.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ae::inventory::ServerId;
use ae::meta::Selector;
use ae::tmux::{PickerPane, PickerSession, display_menu_for_client_args};

use super::cli::ae;
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
                |text| text.contains("ae fleet"),
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
                |text| text.contains("ae fleet"),
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

/// Launch two sessions with one real client pair, then prove a right-click on
/// the product's installed `ae` status range opens the picker on that client.
/// Paths are inputs to the actual checkout launch and tmux binding, not argv
/// assertions. Each case owns and tears down its private server.
#[allow(
    clippy::too_many_lines,
    reason = "one real-click punctuation-path witness with same-pane client isolation"
)]
fn assert_version_picker_case(tag: &str, root_name: &str, config_name: &str) {
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
                "#[range=user|ae]ae #{@ae_version}#[norange]",
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
        "the fleet menu from a real version click",
        || {
            tmux(
                &socket,
                &scratch,
                &["capture-pane", "-p", "-t", "clicked-viewer"],
            )
            .1
        },
        |seen| seen.contains("ae fleet") && seen.contains("fleet-b"),
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
        !other.contains("ae fleet"),
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
        |seen| !seen.contains("ae fleet"),
    );
}

/// Plain paths establish the control; comma paths exercise the actual binding's
/// quoting while preserving the two-client proof.
#[test]
fn right_clicking_version_picker_survives_comma_paths() {
    assert_version_picker_case("status-picker-control", "custom-state", "nondefault.config");
    assert_version_picker_case("status-picker-comma", "state,comma", "config,comma");
}

/// Plain paths establish the control; closing-brace paths exercise the actual
/// binding's quoting while preserving the two-client proof.
#[test]
fn right_clicking_version_picker_survives_closing_brace_paths() {
    assert_version_picker_case(
        "status-picker-control-brace",
        "custom-state",
        "nondefault.config",
    );
    assert_version_picker_case("status-picker-brace", "state}brace", "config}brace");
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
        |seen| seen.contains("ae fleet") && seen.contains("fleet-b"),
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
        !other.contains("ae fleet"),
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
/// the version range, and observes the resulting menu on the exact client.
/// A second client watches the same pane: `$TMUX_PANE` alone cannot distinguish
/// them, which is the regression the explicit `--client` contract prevents.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end two-client status mouse story"
)]
fn clicking_the_version_range_opens_the_fleet_and_a_row_lands_on_the_lead() {
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

    // Put the real version range at a deterministic coordinate while keeping
    // the launch-installed binding and the product's two-line status shape.
    let status_set = tmux(
        &socket,
        &scratch,
        &[
            "set-option",
            "-t",
            "fleet-a",
            "status-format[1]",
            "#[range=user|ae]ae #{@ae_version}#[norange]",
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
            "the fleet menu from a real version press",
            || {
                tmux(
                    &socket,
                    &scratch,
                    &["capture-pane", "-p", "-t", "clicked-viewer"],
                )
                .1
            },
            |seen| seen.contains("ae fleet") && seen.contains("fleet-b"),
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
            !held.contains("ae fleet"),
            "tmux 3.4 dispatched the picker before release: {held}"
        );
        mouse_event(&socket, &scratch, "clicked-viewer", 0, 2, height, 'm');
        wait_for(
            "the fleet menu from a real version release",
            || {
                tmux(
                    &socket,
                    &scratch,
                    &["capture-pane", "-p", "-t", "clicked-viewer"],
                )
                .1
            },
            |seen| seen.contains("ae fleet") && seen.contains("fleet-b"),
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
        menu.contains("ae fleet") && menu.contains("fleet-b"),
        "the menu closed after the status-button click: {menu}"
    );
    let other = tmux(
        &socket,
        &scratch,
        &["capture-pane", "-p", "-t", "untouched-viewer"],
    )
    .1;
    assert!(
        !other.contains("ae fleet"),
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
        |seen| seen.contains("ae fleet") && seen.contains("fleet-b"),
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
        |seen| !seen.contains("ae fleet"),
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
                "#[range=user|ae]ae #{@ae_version}#[norange]",
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
