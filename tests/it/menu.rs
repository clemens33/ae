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

/// Whether a capture contains the versioned fleet-picker title.
fn picker_is_open(text: &str) -> bool {
    text.contains(&format!("ae {} —", ae::VERSION))
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
        drawn.contains(&format!(
            "ae {} — 1 running · 0 need you · $12.34 — prefix a",
            ae::VERSION
        )),
        "the drawn picker title names its running core and the fleet's spend: {drawn}"
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
    tmux(
        socket,
        scratch,
        &[
            "show-options",
            "-qv",
            "-t",
            session,
            ae::theme::MENU_OPEN_OPTION,
        ],
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
