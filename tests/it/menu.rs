//! The fleet picker against a REAL tmux server.
//!
//! Three of the picker's claims are claims about tmux, not about ae, and a
//! pure argv assertion cannot hold any of them: that a menu name is expanded by
//! the plain format expander, so `##` collapses and `%%` does not; that the
//! flags must be ended before a row whose name begins with a hyphen, or getopt
//! reads that name as flags; and that a nested menu survives being quoted into
//! one command word. This arm asks a real server all three at once, and then
//! chooses a row and proves where the client landed.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ae::attention::Reason;
use ae::digest::{AgentEntry, SessionEntry, Status};
use ae::inventory::ServerId;
use ae::listing::World;
use ae::meta::Selector;
use ae::orchestrator::{AgentPane, Located, Placement};
use ae::time::Timestamp;
use ae::tmux::display_menu_args;

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

/// The world the menu is built from: one reachable session and one that is not.
fn fleet() -> World {
    let mut hub = SessionEntry::new("hub", Status::Running);
    // Every character the escaping claims are about, in text an operator wrote.
    hub.goal = Some("100% of #{everything}, don't stop".to_owned());
    hub.last_active_epoch = Some(1_780_000_000);
    hub.agents = vec![
        AgentEntry {
            reference: "lead".to_owned(),
            alias: "cl".to_owned(),
            name: "lead".to_owned(),
            session_id: None,
            alive: Some(true),
            state: Some("working".to_owned()),
            reason: None,
        },
        AgentEntry {
            reference: "helper".to_owned(),
            alias: "cl".to_owned(),
            name: "helper".to_owned(),
            session_id: None,
            alive: Some(true),
            state: Some("blocked".to_owned()),
            reason: Some(Reason::Blocked),
        },
    ];
    // `dead` outranks everything, so this one is drawn FIRST — and it is the row
    // that cannot be chosen, which is the getopt hazard.
    let mut far = SessionEntry::new("far", Status::Running);
    far.attention = Some(Reason::Dead);
    World::new(Timestamp::from_epoch(1_780_000_000), vec![hub, far])
}

/// The two sessions the arm needs, a real client watching one of them, and the
/// ids of the panes the picker will target.
fn stage(socket: &Path, main: &Path) -> Vec<String> {
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
    wait_for(
        "a client",
        || tmux(socket, main, &["list-clients", "-F", "#{client_session}"]).1,
        |seen| seen.contains("home"),
    );

    let panes = tmux(
        socket,
        main,
        &["list-panes", "-t", "hub", "-F", "#{pane_id}"],
    )
    .1;
    let ids: Vec<String> = panes.lines().map(|line| line.trim().to_owned()).collect();
    assert_eq!(ids.len(), 2, "two panes in hub: {panes:?}");

    ids
}

/// The menu the picker builds for `ids`, as the argv that draws it on `socket`.
fn picker_argv(socket: &Path, ids: &[String]) -> Vec<String> {
    let world = fleet();
    let located = [
        Located {
            session: "hub".to_owned(),
            placement: Placement::Here(vec![
                AgentPane {
                    agent: "lead".to_owned(),
                    pane: ids[0].clone(),
                },
                AgentPane {
                    agent: "helper".to_owned(),
                    pane: ids[1].clone(),
                },
            ]),
        },
        Located {
            session: "far".to_owned(),
            placement: Placement::Elsewhere("tmux -L elsewhere attach -t far".to_owned()),
        },
    ];
    let menu = ae::orchestrator::menu(&world, &located, world.now);
    display_menu_args(
        &ServerId::Selected(Selector::Socket(socket.to_path_buf())),
        &menu,
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

    let ids = stage(&socket, &main);
    let argv = picker_argv(&socket, &ids);

    // `display-menu` holds its client until the menu closes, so the keys come
    // from a second thread while this one waits on tmux.
    let drawn = std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            let seen = wait_for(
                "the menu",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                |text| text.contains("ae fleet"),
            );
            // `far` is the first row and cannot be chosen, so `1` is `hub`.
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "1"]).0);
            wait_for(
                "the agent menu",
                || tmux(&socket, &watcher, &["capture-pane", "-p", "-t", "viewer"]).1,
                |text| text.contains("helper"),
            );
            // …and `2` is `helper`, the SECOND pane of hub.
            assert!(tmux(&socket, &watcher, &["send-keys", "-t", "viewer", "2"]).0);
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
        drawn.contains("tmux -L elsewhere attach -t far"),
        "the unreachable session names the command that reaches it: {drawn}"
    );
    assert!(
        drawn.contains("dead"),
        "…and keeps the attention word: {drawn}"
    );

    // …and where the row LANDED the client.
    let landed = wait_for(
        "the jump",
        || {
            tmux(
                &socket,
                &main,
                &["list-clients", "-F", "#{client_session}|#{pane_id}"],
            )
            .1
        },
        |seen| seen.contains("hub|"),
    );
    assert!(
        landed.contains(&format!("hub|{}", ids[1])),
        "the client should sit in helper's pane {}: {landed:?}",
        ids[1]
    );
}
