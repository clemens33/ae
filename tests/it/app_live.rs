//! The live app in a private tmux: what only a real terminal shows
//! (pins-plan.md #45-#47, #49, #58/#59). Real captures judge the screen; the
//! oracles are docs/app.md, the frames' geometry and the fixture journal.
//! Private sockets and existing process doors only.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use crate::app_mode::writing;
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
    rig.wait(&pane, WAIT, "writing starts", writing);
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

/// Frozen appload acceptance (R1/R2/R6): hold the actual transcript stages
/// at the checkout-only read door; judge frames while each read is blocked.
/// Lead replaced the bulk/timing oracle with these deterministic gates.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Frozen acceptance judges all three frames while one fixture holds both reads."
)]
fn appload_journal_current_and_earlier_turns_paint_in_order() {
    // Drop releases every hold on assertion unwind; the existing read door
    // also bounds a held read to 60 s. No extra reader or process door.
    struct Holds(Vec<PathBuf>);
    impl Drop for Holds {
        fn drop(&mut self) {
            for path in &self.0 {
                let _ = fs::remove_file(path);
            }
        }
    }
    let rig = Rig::new("appload", false);
    let store = rig.root.join("claude");
    let current = "0199c0de-aaaa-4890-abcd-ef0123456789";
    let prior = "0199c0de-bbbb-4890-abcd-ef0123456789";
    let project = store.join("projects/work");
    fs::create_dir_all(&project).expect("synthetic transcript directory");
    for (id, stamp, words) in [
        (current, "2026-10-04T10:01:00Z", "appload-current-turn"),
        (prior, "2026-10-04T09:59:00Z", "appload-earlier-turn"),
    ] {
        fs::write(
            project.join(format!("{id}.jsonl")),
            super::board::user(stamp, words) + "\n",
        )
        .expect("regular committed synthetic transcript");
    }
    fs::write(rig.tool.dir.join("meta"), format!(
        "schema=2\nsession={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nharness_session.main={current}\nharness_session_prior.main=claude:{prior}\nconfig_home.main={}\n",
        rig.name, rig.socket.display(), store.display(),
    )).expect("one kept seat with a recorded predecessor");
    fs::write(
        rig.tool.dir.join("events.jsonl"),
        "{\"ts\":\"2026-10-04T10:02:00Z\",\"actor\":\"lead\",\"action\":\"chat\",\"summary\":\"appload-journal-row\"}\n",
    ).expect("journal row");

    let sessions = [ae::usage::SessionInput {
        name: rig.name.clone(),
        path: rig.tool.dir.clone(),
    }];
    let inputs = ae::board::Inputs {
        home: Some(&rig.root),
        sessions: &sessions,
        replies: ae::board::Replies::Off,
    };
    let whole = ae::board::observe_selected(&inputs, None, None, &|_| true);
    assert!(
        whole.coverage.is_empty(),
        "valid fixture: {:?}",
        whole.coverage
    );
    assert_eq!(whole.rows.len(), 2, "both fixture generations really read");
    drop(whole);

    let gates = rig.root.join("read-gates");
    fs::create_dir(&gates).expect("private read-gate directory");
    let holds = Holds(vec![
        gates.join("@board-current"),
        gates.join("@board-earlier"),
    ]);
    for path in &holds.0 {
        fs::write(path, "hold").expect("arm transcript read");
    }
    let held = |key: &str| {
        fs::read_to_string(gates.join("trace"))
            .unwrap_or_default()
            .lines()
            .any(|line| line == format!("held {key}"))
    };

    // Direct invocation pins scratch HOME and removes inherited client homes.
    let command = format!(
        "env -u CLAUDE_CONFIG_DIR -u CODEX_HOME AE_TEST_APP_READ_GATE={} {} app {}",
        quote(&gates.display().to_string()),
        quote(env!("CARGO_BIN_EXE_ae")),
        quote(&rig.name),
    );
    let pane = window(&rig, &command);
    rig.tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "45"]);
    let first = rig.wait(&pane, WAIT, "R1/R2: journal visible with current turns still loading", |screen| {
        screen.contains("appload-journal-row")
            && screen.contains("coverage incomplete: pane turns — loading: the current conversations, then the earlier ones")
            && held("@board-current")
    });
    assert!(
        !first.contains("appload-current-turn") && !first.contains("appload-earlier-turn"),
        "journal stage has no transcript rows: {first}"
    );
    assert!(
        held("@board-current") && holds.0[0].exists(),
        "S0 screen judged during the actual blocked current read"
    );
    assert!(
        !held("@board-earlier"),
        "predecessors never precede the current stage"
    );
    fs::remove_file(&holds.0[0]).expect("release current stage");
    let middle = rig.wait(
        &pane,
        WAIT,
        "R1/R6: current turns precede predecessors",
        |screen| {
            screen.contains("appload-current-turn")
                && screen.contains(
                    "coverage incomplete: pane turns — loading: the earlier conversations",
                )
                && held("@board-earlier")
        },
    );
    assert!(
        middle.contains("appload-journal-row") && !middle.contains("appload-earlier-turn"),
        "current stage keeps journal and names missing predecessors: {middle}"
    );
    assert!(
        held("@board-earlier") && holds.0[1].exists(),
        "S1 screen judged during the actual blocked predecessor read"
    );
    fs::remove_file(&holds.0[1]).expect("release predecessor stage");
    let complete = rig.wait(
        &pane,
        WAIT,
        "R2: coverage loading row disappears at completion",
        |screen| {
            screen.contains("appload-earlier-turn") && !screen.contains("pane turns — loading:")
        },
    );
    for words in [
        "appload-journal-row",
        "appload-current-turn",
        "appload-earlier-turn",
    ] {
        assert_eq!(
            complete.matches(words).count(),
            1,
            "one row per source after replacement: {complete}"
        );
    }
}

/// R2: a predecessor stage whose session meta vanished mid-read never loses
/// the predecessors silently. Held at `@board-earlier`, the meta is moved
/// aside and the fleet read is held at `@world`, so no fleet read sees the
/// gap and the console survives; once the meta is back and a later read has
/// painted, the lane still carries the predecessor's turn or names it.
#[test]
fn appload_a_meta_lost_during_the_predecessor_stage_loses_no_predecessor() {
    struct Holds(Vec<PathBuf>);
    impl Drop for Holds {
        fn drop(&mut self) {
            for path in &self.0 {
                let _ = fs::remove_file(path);
            }
        }
    }
    let rig = Rig::new("apploadr2", false);
    let store = rig.root.join("claude");
    let (current, prior) = (
        "0199c0de-aaaa-4890-abcd-ef0123456789",
        "0199c0de-bbbb-4890-abcd-ef0123456789",
    );
    let project = store.join("projects/work");
    fs::create_dir_all(&project).expect("synthetic transcript directory");
    for (id, stamp, words) in [
        (current, "2026-10-04T10:01:00Z", "appload-current-turn"),
        (prior, "2026-10-04T09:59:00Z", "appload-earlier-turn"),
    ] {
        let line = super::board::user(stamp, words) + "\n";
        fs::write(project.join(format!("{id}.jsonl")), line).expect("synthetic transcript");
    }
    let meta = rig.tool.dir.join("meta");
    fs::write(&meta, format!(
        "schema=2\nsession={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nharness_session.main={current}\nharness_session_prior.main=claude:{prior}\nconfig_home.main={}\n",
        rig.name, rig.socket.display(), store.display(),
    )).expect("one kept seat with a recorded predecessor");
    let journal = rig.tool.dir.join("events.jsonl");
    let row = |words: &str| {
        format!(
            "{{\"ts\":\"2026-10-04T10:02:00Z\",\"actor\":\"lead\",\"action\":\"chat\",\"summary\":\"{words}\"}}\n"
        )
    };
    fs::write(&journal, row("appload-journal-row")).expect("journal row");
    let gates = rig.root.join("read-gates");
    fs::create_dir(&gates).expect("private read-gate directory");
    let holds = Holds(vec![gates.join("@board-earlier"), gates.join("@world")]);
    fs::write(&holds.0[0], "hold").expect("arm predecessor read");
    let held = |key: &str| {
        fs::read_to_string(gates.join("trace"))
            .unwrap_or_default()
            .lines()
            .any(|line| line == format!("held {key}"))
    };
    let command = format!(
        "env -u CLAUDE_CONFIG_DIR -u CODEX_HOME AE_TEST_APP_READ_GATE={} {} app {}",
        quote(&gates.display().to_string()),
        quote(env!("CARGO_BIN_EXE_ae")),
        quote(&rig.name),
    );
    let pane = window(&rig, &command);
    rig.tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "45"]);
    rig.wait(&pane, WAIT, "the predecessor stage is held", |screen| {
        screen.contains("appload-current-turn") && held("@board-earlier")
    });
    fs::write(&holds.0[1], "hold").expect("arm the next fleet read");
    let aside = rig.tool.dir.join("meta.aside");
    fs::rename(&meta, &aside).expect("meta moved aside");
    fs::remove_file(&holds.0[0]).expect("release predecessor stage");
    rig.wait(
        &pane,
        WAIT,
        "the stage answered, the fleet read held",
        |screen| !screen.contains("pane turns — loading:") && held("@world"),
    );
    fs::rename(&aside, &meta).expect("meta restored");
    let mut rows = fs::read_to_string(&journal).expect("journal");
    rows.push_str(&row("appload-after-restore"));
    fs::write(&journal, rows).expect("a row only a later read paints");
    fs::remove_file(&holds.0[1]).expect("release fleet read");
    let after = rig.wait(&pane, WAIT, "a later read painted", |screen| {
        screen.contains("appload-after-restore")
    });
    assert!(
        after.contains("appload-earlier-turn") || after.contains("predecessor 1"),
        "the predecessors are read or named, never silently dropped: {after}"
    );
}
