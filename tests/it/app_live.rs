//! The live app in a private tmux: what only a real terminal shows
//! (pins-plan.md #45-#47, #49, #58/#59). Real captures judge the screen; the
//! oracles are docs/app.md, the frames' geometry and the fixture journal.
//! Private sockets and existing process doors only.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);

/// One lead-pair session on a private server whose chat window is `ae app`.
struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    name: String,
}

impl Rig {
    /// The session; `look` false sets `@ae_look off` (theme off draws no colour).
    fn new(tag: &str, look: bool) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let root = tool
            .dir
            .parent()
            .expect("sessions")
            .parent()
            .expect("root")
            .to_owned();
        let name = tool
            .dir
            .file_name()
            .expect("session")
            .to_string_lossy()
            .into_owned();
        let socket = root.join("sock");
        let rig = Self {
            tool,
            root,
            socket,
            name,
        };
        fs::write(rig.tool.dir.join("meta"), format!(
            "session={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-rig\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\n",
            rig.name, rig.socket.display()
        )).expect("lead pair meta");
        fs::write(rig.root.join("config"), "[workspace]\nchat = app\n").expect("chat = app");
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        fs::write(rig.tool.dir.join(".launch-attempt"), epoch.to_string()).expect("launch stamp");
        if !look {
            rig.tmux(&["set-option", "-t", &rig.name, "@ae_look", "off"]);
        }
        for (option, value) in [
            ("@ae_session_uuid", UUID),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.name, option, value]);
        }
        rig.tmux(&[
            "set-option",
            "-p",
            "-t",
            &rig.tool.pane,
            "@ae_agent",
            "lead",
        ]);
        rig
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    /// Open the chat window, which `chat = app` makes the app: its pane.
    fn open_app(&self) -> String {
        let out = super::cli::ae()
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("TMUX", format!("{},1,0", self.socket.display()))
            .env("TMUX_PANE", &self.tool.pane)
            .arg("_console")
            .output()
            .expect("the chat window opens");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &self.name,
            "-F",
            "#{pane_id}|#{@ae_console}",
        ])
        .lines()
        .find_map(|row| {
            let (pane, stamp) = row.split_once('|')?;
            (stamp == UUID).then(|| pane.to_owned())
        })
        .expect("the stamped app window")
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    /// The screen once `met` holds, within `within`.
    fn wait(&self, pane: &str, within: Duration, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + within;
        loop {
            let screen = self.screen(pane);
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The row holding `needle`, as characters.
fn row_with(screen: &str, needle: &str) -> Option<Vec<char>> {
    screen
        .lines()
        .find(|row| row.contains(needle))
        .map(|row| row.chars().collect())
}

/// #47, #49, #46: the first paint lists the session the root holds (docs/app.md:
/// one row per session), `Tab` shows the Agents tab well inside one refresh
/// (`POLL_SECS` = 5, so 2 s leaves load slack), and a resize re-lays the frame
/// out at the new width: at 100 columns the sidebar's rule stands at 34
/// (frame 100x30: `Tab` ends two cells before `│`@34).
#[test]
fn the_live_app_paints_takes_tab_and_relays_out_on_resize() {
    let rig = Rig::new("appl", false);
    let pane = rig.open_app();
    let name = rig.name.clone();
    rig.wait(&pane, WAIT, "first paint lists the session", |screen| {
        screen.contains("Sessions 1") && screen.lines().any(|row| row.contains(&name))
    });
    rig.tmux(&["send-keys", "-t", &pane, "Tab"]);
    let painted = rig.tmux(&["capture-pane", "-e", "-p", "-t", &pane]);
    assert!(!painted.contains("38;2;"), "theme off draws no colour (R7)");
    rig.wait(
        &pane,
        Duration::from_secs(2),
        "Tab shows Agents at once",
        |screen| screen.contains("No seat facts"),
    );
    rig.tmux(&["resize-window", "-t", &pane, "-x", "100", "-y", "30"]);
    rig.wait(
        &pane,
        WAIT,
        "the frame re-lays out at 100 columns",
        |screen| row_with(screen, "Overview").is_some_and(|row| row.get(34) == Some(&'│')),
    );
}

/// #14: a session whose look tmux holds none of is drawn in the default
/// palette (docs/app.md: "then the default palette"), so the frame carries
/// direct colour.
#[test]
fn a_session_with_no_look_paints_the_default_palette() {
    let rig = Rig::new("appd", true);
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "first paint", |screen| {
        screen.contains("Sessions 1")
    });
    let painted = rig.tmux(&["capture-pane", "-e", "-p", "-t", &pane]);
    assert!(painted.contains("38;2;"), "the default palette is drawn");
}

/// A rig window running `command` (sh) with the app's scratch environment;
/// its pane.
fn window(rig: &Rig, command: &str) -> String {
    let env = [
        format!("AE_HOME={}", rig.root.display()),
        format!("CONFIG_FILE={}", rig.root.join("config").display()),
        format!("HOME={}", rig.root.display()),
        "AE_TMUX_SERVER_KIND=socket".to_owned(),
        format!("AE_TMUX_SERVER={}", rig.socket.display()),
    ];
    let mut args = vec![
        "new-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-t",
        rig.name.as_str(),
    ];
    for pair in &env {
        args.extend(["-e", pair.as_str()]);
    }
    args.push(command);
    rig.tmux(&args).trim().to_owned()
}

/// #115/#118/#125: a terminal that reports no columns, or no rows, has not
/// said its size (`Tty::size`: `None` when the terminal will not say), so the
/// app draws at its 80x24 fallback, whose floor line names that size
/// (docs/app.md:40). The receipt proves the zero reached the app's pty.
#[test]
fn a_terminal_that_reports_a_zero_size_is_drawn_at_the_fallback_size() {
    let rig = Rig::new("appz", false);
    for (tag, stty, told) in [
        ("cols", "rows 30 cols 0", "30 0"),
        ("rows", "rows 0 cols 100", "0 100"),
    ] {
        let size = rig.root.join(format!("size-{tag}"));
        let pane = window(
            &rig,
            &format!(
                "stty {stty}; stty size > {}; exec {} app {}",
                quote(&size.to_string_lossy()),
                quote(env!("CARGO_BIN_EXE_ae")),
                quote(&rig.name),
            ),
        );
        rig.wait(&pane, WAIT, "the fallback size is drawn", |screen| {
            screen.contains("(now 80x24)")
        });
        let held = fs::read_to_string(&size).expect("the size the pty held");
        assert_eq!(
            held.trim(),
            told,
            "{tag}: the app's terminal reported a zero"
        );
    }
}

/// #126: quitting puts the terminal back — off the alternate screen, out of
/// raw mode (tty.rs: put back on every way out ae controls; docs/app.md:
/// only SIGKILL or SIGTERM leave it raw on the alternate screen).
#[test]
fn quitting_puts_the_terminal_back() {
    let rig = Rig::new("appq", false);
    let modes = rig.root.join("modes");
    let pane = window(
        &rig,
        &format!(
            "{} app {}; stty -a > {}; sleep 60",
            quote(env!("CARGO_BIN_EXE_ae")),
            quote(&rig.name),
            quote(&modes.to_string_lossy()),
        ),
    );
    rig.wait(&pane, WAIT, "first paint", |screen| {
        screen.contains("Sessions 1")
    });
    rig.tmux(&["send-keys", "-t", &pane, "q", "q"]);
    let until = Instant::now() + WAIT;
    while fs::read_to_string(&modes).map_or(true, |text| text.is_empty()) {
        assert!(
            Instant::now() < until,
            "the app quit and the shell read its modes"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    std::thread::sleep(Duration::from_millis(50));
    let screen = rig.tmux(&["display-message", "-p", "-t", &pane, "#{alternate_on}"]);
    assert_eq!(screen.trim(), "0", "off the alternate screen");
    let modes = fs::read_to_string(&modes).expect("modes");
    let flags: Vec<&str> = modes.split_whitespace().collect();
    for on in ["icanon", "echo"] {
        assert!(flags.contains(&on), "{on} is set after quitting: {modes}");
        let off = format!("-{on}");
        assert!(
            !flags.contains(&off.as_str()),
            "{off} after quitting: {modes}"
        );
    }
}

/// #45: the app needs a terminal at BOTH ends; a tty in and a file out is
/// refused with the one line, exit 1, and nothing drawn into the file.
#[test]
fn the_app_with_one_terminal_end_refuses() {
    let rig = Rig::new("appt", false);
    let (out, err, rc) = (
        rig.root.join("out"),
        rig.root.join("err"),
        rig.root.join("rc"),
    );
    let command = format!(
        "{} app {} > {} 2> {}; echo $? > {}",
        quote(env!("CARGO_BIN_EXE_ae")),
        quote(&rig.name),
        quote(&out.to_string_lossy()),
        quote(&err.to_string_lossy()),
        quote(&rc.to_string_lossy()),
    );
    let _ = window(&rig, &command);
    let until = Instant::now() + WAIT;
    while !rc.exists() {
        assert!(
            Instant::now() < until,
            "the app ended; err: {:?}",
            fs::read_to_string(&err)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(fs::read_to_string(&rc).expect("rc").trim(), "1");
    assert_eq!(fs::read_to_string(&err).expect("err"), ae::app::NO_TERMINAL);
    assert_eq!(fs::read_to_string(&out).expect("out"), "", "nothing drawn");
}

/// #58/#59: `/close` in the owning app withdraws only an ask still open in
/// THIS session. The asked seat's retire, recorded here, closed it already
/// (nobody will answer), so the chat's admission refuses (docs/app.md:
/// `/close` withdraws your newest open ask).
#[test]
fn close_in_the_app_honours_a_retire_of_the_asked_seat() {
    let rig = Rig::new("appc", false);
    let name = rig.name.clone();
    let ask = format!(
        r#"{{"ts":"2026-09-30T06:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"ae-20260930T060000Z-0000c105","target_slot":"main","target_session":"{name}","target_server":"{}","target_pane":"{}","target_session_uuid":"{UUID}","summary":"check the scopes"}}"#,
        rig.socket.display(),
        rig.tool.pane
    );
    let retire = format!(
        r#"{{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"retire","target":"lead","target_slot":"main","target_session":"{name}"}}"#
    );
    fs::write(
        rig.tool.dir.join("events.jsonl"),
        format!("{ask}\n{retire}\n"),
    )
    .expect("journal");
    let pane = rig.open_app();
    let composer = format!("to {name} › lead");
    rig.wait(&pane, WAIT, "the owner composer", |screen| {
        screen.contains(&composer)
    });
    rig.tmux(&["send-keys", "-t", &pane, "i"]);
    rig.wait(&pane, WAIT, "writing starts", |screen| {
        screen.contains("Enter sends")
    });
    rig.tmux(&["send-keys", "-t", &pane, "-l", "/close"]);
    rig.tmux(&["send-keys", "-t", &pane, "Enter"]);
    let screen = rig.wait(&pane, WAIT, "the close is answered", |screen| {
        screen.contains("refused: ") || screen.contains("closed an ask")
    });
    assert!(screen.contains("refused: no open ask to close"), "{screen}");
    assert!(
        !rig.tool.events().contains("\"cancel\""),
        "nothing was withdrawn"
    );
}
