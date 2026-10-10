//! Frozen appcopy acceptance spec.
//! Oracles: brief-appcopy.md R1-R8, lead rulings Q1-Q6 and critique rulings (B1-B3, I1-I5, N1-N3),
//! plan-appcopy.md rev2 D9 flash table and D10 preview whys.
//! Existing APIs and isolated tmux server fixtures only.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const TID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);

fn json_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn has_marked_body(screen: &str, body: &str) -> bool {
    let with_space_unicode = format!("│ {body}");
    let no_space_unicode = format!("│{body}");
    let with_space_ascii = format!("| {body}");
    let no_space_ascii = format!("|{body}");
    screen.lines().any(|line| {
        line.contains(&with_space_unicode)
            || line.contains(&no_space_unicode)
            || line.contains(&with_space_ascii)
            || line.contains(&no_space_ascii)
    })
}

fn cell_of(screen: &str, needle: &str) -> Option<(usize, usize)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        line.chars()
            .collect::<Vec<_>>()
            .windows(needle.chars().count())
            .position(|w| w.iter().collect::<String>() == needle)
            .map(|col| (col, row))
    })
}

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    home: String,
    _store: PathBuf,
}

impl Rig {
    fn new(tag: &str, turns: &[&str]) -> Self {
        Self::with_stopped(tag, turns, &[])
    }

    fn with_stopped(tag: &str, turns: &[&str], stopped: &[&str]) -> Self {
        let tool = super::deliver::Rig::new(tag, "claude", 0);
        let root = tool
            .dir
            .parent()
            .expect("sessions")
            .parent()
            .expect("root")
            .to_owned();
        let home = tool
            .dir
            .file_name()
            .expect("session")
            .to_string_lossy()
            .into_owned();
        let socket = root.join("sock");
        let store = root.join("claude-home");

        let lines = turns
            .iter()
            .enumerate()
            .map(|(at, body)| {
                let escaped = json_escape(body);
                format!(
                    r#"{{"type":"user","timestamp":"2026-10-07T20:00:{at:02}Z","message":{{"role":"user","content":"{escaped}"}}}}"#
                )
            })
            .collect::<Vec<_>>();
        super::board::plant_transcript(&store, "work", TID, &lines);

        fs::write(
            tool.dir.join("meta"),
            format!(
                "session={home}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-rig\nharness_session.main={TID}\nconfig_home.main={}\nseat.worker.0=colead\nagent_bin.worker.0=claude\nlaunch_id.worker.0=tok-colead\nconfig_home.worker.0={}\n",
                socket.display(),
                store.display(),
                store.display(),
            ),
        )
        .expect("lead pair meta");

        for sibling in stopped {
            let dir = root.join("sessions").join(sibling);
            fs::create_dir_all(&dir).expect("sibling dir");
            fs::write(
                dir.join("meta"),
                format!(
                    "session={sibling}\nmode=local\nsession_id=0199c0de-bbbb-4890-abcd-ef0123456789\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-stop\n",
                    socket.display(),
                ),
            )
            .expect("stopped meta");
        }

        fs::write(root.join("config"), "[workspace]\nchat = app\n").expect("app config");
        fs::write(
            tool.dir.join(".launch-attempt"),
            ae::time::Timestamp::now().epoch().to_string(),
        )
        .expect("launch stamp");
        if !tool.dir.join("events.jsonl").exists() {
            fs::write(tool.dir.join("events.jsonl"), "").expect("events");
        }

        let rig = Self {
            tool,
            root,
            socket,
            home,
            _store: store,
        };

        for (option, value) in [
            ("@ae_look", "off"),
            ("@ae_session_uuid", UUID),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.home, option, value]);
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

    fn open(&self) -> String {
        let mut cmd = super::cli::ae();
        cmd.env("AE_HOME", &self.root)
            .env("HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("TMUX", format!("{},1,0", self.socket.display()))
            .env("TMUX_PANE", &self.tool.pane)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME");
        let out = cmd.arg("_console").output().expect("open fixture app");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let pane = self
            .tmux(&[
                "list-panes",
                "-s",
                "-t",
                &self.home,
                "-F",
                "#{pane_id}|#{@ae_console}",
            ])
            .lines()
            .find_map(|row| {
                let (pane, stamp) = row.split_once('|')?;
                (stamp == UUID).then(|| pane.to_owned())
            })
            .expect("stamped app pane");
        self.wait_screen(&pane, "GUARD home ready", |text| {
            text.contains("Enter writes") && text.contains("Overview")
        });
        pane
    }

    fn attach(&self, pane: &str) -> super::cli::OwnedChild {
        self.tmux(&["select-pane", "-t", pane]);
        let client = super::cli::tmux_attached_client(&self.socket, &self.home)
            .expect("real attached client");
        let pid = client.id().to_string();
        Self::wait_for("GUARD client attached and viewing app pane", || {
            self.tmux(&["list-clients", "-F", "#{client_pid}|#{pane_id}"])
                .lines()
                .any(|row| row == format!("{pid}|{pane}"))
        });
        client
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn wait_for(description: &str, mut met: impl FnMut() -> bool) {
        let until = Instant::now() + WAIT;
        while !met() {
            assert!(Instant::now() < until, "{description}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_screen(&self, pane: &str, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        loop {
            let s = self.screen(pane);
            if met(&s) {
                return s;
            }
            assert!(Instant::now() < until, "{why}\nscreen:\n{s}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn raw(&self, pane: &str, bytes: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", bytes]);
    }

    fn key(&self, pane: &str, key: &str) {
        self.tmux(&["send-keys", "-t", pane, key]);
    }

    fn click(&self, pane: &str, col0: usize, row0: usize) {
        let seq = format!("\x1b[<0;{};{}M", col0 + 1, row0 + 1);
        self.tmux(&["send-keys", "-t", pane, "-l", "--", &seq]);
    }

    fn buffer(&self) -> Option<String> {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(["show-buffer"].map(ToOwned::to_owned));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        ok.then_some(out)
    }

    fn clear_buffer(&self) {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(["delete-buffer"].map(ToOwned::to_owned));
        let _ = super::phase2::run_tmux(&args, &self.root);
    }
}

// ---------------------------------------------------------------------------
// Environment guard: tmux load-buffer mechanism writes default paste buffer
// ---------------------------------------------------------------------------

#[test]
fn guard_tmux_load_buffer_targets_default_paste_buffer() {
    let mut scratch = super::cli::OwnedScratch::root("appcopy", "guard-buffer");
    let sock = scratch.join("sock");
    scratch.add_tmux_server(sock.clone());
    let mut create = vec!["-S".to_owned(), sock.display().to_string()];
    create.extend(["new-session", "-d", "-s", "target", "sleep", "60"].map(ToOwned::to_owned));
    assert!(super::phase2::run_tmux(&create, scratch.path()).0);

    let test_file = scratch.path().join("staged");
    fs::write(&test_file, "test staged bytes\n").expect("write staged");
    let mut load = vec!["-S".to_owned(), sock.display().to_string()];
    load.extend(["load-buffer", test_file.to_str().expect("utf8 path")].map(ToOwned::to_owned));
    assert!(super::phase2::run_tmux(&load, scratch.path()).0);

    let mut show = vec!["-S".to_owned(), sock.display().to_string()];
    show.extend(["show-buffer"].map(ToOwned::to_owned));
    let (ok, out) = super::phase2::run_tmux(&show, scratch.path());
    assert!(ok, "show-buffer without -b reads default buffer");
    assert_eq!(out, "test staged bytes\n");
}

// ---------------------------------------------------------------------------
// R3: Keys tab includes turn marking and copy keys
// ---------------------------------------------------------------------------

#[test]
fn browse_keys_table_lists_turn_marks_and_copy() {
    let rig = Rig::new("keys-tab", &["one turn"]);
    let app = rig.open();
    rig.raw(&app, "?");
    rig.wait_screen(&app, "Keys overlay opened", |s| s.contains("Browse"));
    let screen = rig.screen(&app);
    assert!(
        screen.contains("[ ]") && screen.contains("mark older / newer turn"),
        "R3: Keys tab lists [ ] to mark older / newer turn\n{screen}"
    );
    assert!(
        screen.contains('y') && screen.contains("copy the marked turn"),
        "R3: Keys tab lists y to copy the marked turn\n{screen}"
    );
}

// ---------------------------------------------------------------------------
// R1, R2, R3, R4, R5, R6: Golden path copy
// ---------------------------------------------------------------------------

#[test]
fn copy_marked_turn_loads_tmux_buffer_and_flashes_truthful_outcome() {
    let rig = Rig::new("golden", &["alpha line 1\twith tab\nbeta line 2\x1b[31m"]);
    let app = rig.open();
    let _viewer = rig.attach(&app);

    let screen_before = rig.wait_screen(&app, "turn text drawn", |s| s.contains("alpha line 1"));
    assert!(
        !has_marked_body(&screen_before, "alpha line 1"),
        "D4: no mark glyph tied to turn body before selection"
    );

    rig.raw(&app, "[");
    rig.wait_screen(&app, "turn visibly marked with glyph", |s| {
        has_marked_body(s, "alpha line 1")
    });

    rig.clear_buffer();
    rig.raw(&app, "y");

    rig.wait_screen(&app, "flash line confirms copy", |s| {
        s.contains(
            "copied 2 lines to the tmux buffer; terminal clipboard if your terminal allows it",
        )
    });

    let buf = rig.buffer().expect("tmux paste buffer was loaded");
    assert_eq!(
        buf, "alpha line 1\twith tab\nbeta line 2\u{FFFD}[31m",
        "R1/R2/R4: tmux paste buffer received exact body neutralised without styling or chrome"
    );

    rig.clear_buffer();
    rig.raw(&app, "y");
    Rig::wait_for("repeated y copies again keeping mark", || {
        rig.buffer().as_deref() == Some("alpha line 1\twith tab\nbeta line 2\u{FFFD}[31m")
    });
}

// ---------------------------------------------------------------------------
// R3, D3, D5: Click marks turn, click on marked turn unmarks
// ---------------------------------------------------------------------------

#[test]
fn click_turn_toggles_mark() {
    let rig = Rig::new("click-toggle", &["clickable turn body"]);
    let app = rig.open();
    let _viewer = rig.attach(&app);

    let screen = rig.wait_screen(&app, "turn text drawn", |s| {
        s.contains("clickable turn body")
    });
    assert!(!has_marked_body(&screen, "clickable turn body"));

    let (col, row) = cell_of(&screen, "clickable turn body").expect("body cell");
    rig.click(&app, col, row);

    rig.wait_screen(&app, "turn marked by click", |s| {
        has_marked_body(s, "clickable turn body")
    });

    rig.click(&app, col, row);
    rig.wait_screen(&app, "turn unmarked by second click", |s| {
        !has_marked_body(s, "clickable turn body")
    });

    rig.clear_buffer();
    rig.raw(&app, "y");
    rig.wait_screen(&app, "y flashes hint after unmarking", |s| {
        s.contains("no turn marked - click one or press [ ]")
    });
    assert_eq!(rig.buffer(), None, "D3/D9: no buffer loaded when unmarked");
}

// ---------------------------------------------------------------------------
// R3, R6, D9: No mark and zero-turn invariants
// ---------------------------------------------------------------------------

#[test]
fn copy_without_marked_turn_flashes_hint_and_never_touches_tmux_buffer() {
    let rig = Rig::new("no-mark", &["first turn"]);
    let app = rig.open();
    let _viewer = rig.attach(&app);
    rig.clear_buffer();

    rig.raw(&app, "y");
    rig.wait_screen(&app, "flash hints to mark a turn", |s| {
        s.contains("no turn marked - click one or press [ ]")
    });
    assert_eq!(
        rig.buffer(),
        None,
        "D3/D9: no buffer loaded when no turn is marked"
    );
}

#[test]
fn copy_zero_turns_lane_navigation_and_copy_hint() {
    let rig = Rig::new("zero-turns", &[]);
    let app = rig.open();
    let _viewer = rig.attach(&app);

    rig.raw(&app, "[");
    rig.raw(&app, "]");
    rig.raw(&app, "y");
    rig.wait_screen(&app, "y on zero-turn lane flashes hint", |s| {
        s.contains("no turn marked - click one or press [ ]")
    });
}

// ---------------------------------------------------------------------------
// R3, D3, I3, N2: Bounds, newer turn step, settings retention
// ---------------------------------------------------------------------------

#[test]
fn turn_navigation_bounds_and_settings_retention() {
    let rig = Rig::new("bounds", &["turn zero", "turn one"]);
    let app = rig.open();
    let _viewer = rig.attach(&app);

    let screen = rig.wait_screen(&app, "both turns drawn", |s| {
        s.contains("turn zero") && s.contains("turn one")
    });
    assert!(!has_marked_body(&screen, "turn zero"));
    assert!(!has_marked_body(&screen, "turn one"));

    rig.raw(&app, "[");
    rig.wait_screen(&app, "initial mark on newest turn one", |s| {
        has_marked_body(s, "turn one") && !has_marked_body(s, "turn zero")
    });

    rig.raw(&app, "[");
    rig.wait_screen(&app, "step to older turn zero", |s| {
        has_marked_body(s, "turn zero") && !has_marked_body(s, "turn one")
    });

    rig.raw(&app, "[");
    rig.raw(&app, "[");
    rig.clear_buffer();
    rig.raw(&app, "y");
    Rig::wait_for("stepping past oldest turn keeps turn zero marked", || {
        rig.buffer().as_deref() == Some("turn zero")
    });

    rig.raw(&app, "]");
    rig.wait_screen(&app, "step to newer turn one", |s| {
        has_marked_body(s, "turn one") && !has_marked_body(s, "turn zero")
    });

    rig.raw(&app, "]");
    rig.raw(&app, "]");
    rig.clear_buffer();
    rig.raw(&app, "y");
    Rig::wait_for("stepping past newest turn keeps turn one marked", || {
        rig.buffer().as_deref() == Some("turn one")
    });

    rig.raw(&app, "s");
    rig.wait_screen(&app, "Settings overlay opened", |s| s.contains("Esc close"));
    rig.key(&app, "Escape");
    rig.wait_screen(&app, "Settings overlay closed", |s| {
        !s.contains("Esc close")
    });
    let after_settings = rig.screen(&app);
    assert!(
        has_marked_body(&after_settings, "turn one"),
        "N2: Model.turn mark survives Settings open and Esc close"
    );
}

// ---------------------------------------------------------------------------
// R3, D3: Selecting another session clears mark
// ---------------------------------------------------------------------------

#[test]
fn session_switch_clears_mark() {
    let rig = Rig::with_stopped("session-switch", &["turn zero"], &["sibling"]);
    let app = rig.open();
    let _viewer = rig.attach(&app);

    rig.wait_screen(&app, "turn zero drawn", |s| s.contains("turn zero"));
    rig.raw(&app, "[");
    rig.wait_screen(&app, "turn marked", |s| has_marked_body(s, "turn zero"));

    rig.raw(&app, "2");
    rig.wait_screen(&app, "switched to sibling session", |s| {
        s.contains("sibling") && s.contains("stopped")
    });

    rig.raw(&app, "1");
    rig.wait_screen(&app, "switched back to home session", |s| {
        s.contains("Enter writes") && s.contains("turn zero")
    });

    let screen = rig.screen(&app);
    assert!(
        !has_marked_body(&screen, "turn zero"),
        "D3: session switch clears marked turn"
    );

    rig.clear_buffer();
    rig.raw(&app, "y");
    rig.wait_screen(&app, "y flashes hint after session switch", |s| {
        s.contains("no turn marked - click one or press [ ]")
    });
    assert_eq!(
        rig.buffer(),
        None,
        "D3/D9: no buffer loaded after session switch"
    );
}

// ---------------------------------------------------------------------------
// R1, R6, D9, D10: Truthful preview flash strings
// ---------------------------------------------------------------------------

#[test]
fn copy_preview_kinds_flashes_truthful_why_suffix() {
    let rig = Rig::new("preview-kinds", &[]);
    let events = rig.tool.dir.join("events.jsonl");
    let mut rows = fs::read_to_string(&events).unwrap_or_default();
    writeln!(
        rows,
        r#"{{"ts":"2026-10-07T20:01:00Z","actor":"console:local","action":"ask","ref":"0199c0de-9999","target":"lead","summary":"human question summary"}}"#
    )
    .unwrap();
    fs::write(events, rows).expect("append ask event");

    let app = rig.open();
    let _viewer = rig.attach(&app);

    rig.wait_screen(&app, "ask row drawn in lane", |s| {
        s.contains("human question summary")
    });

    rig.raw(&app, "[");
    rig.raw(&app, "y");
    rig.wait_screen(&app, "preview flash for journal summary", |s| {
        s.contains(
            "copied 1 line (preview only: journal summary, line breaks flattened, capped at 600 chars)",
        )
    });

    let buf = rig.buffer().expect("tmux paste buffer was loaded");
    assert_eq!(
        buf, "human question summary",
        "D10: preview copy extracts recorded summary bytes"
    );
}

// ---------------------------------------------------------------------------
// R5, R6, D6, D9: Fallback when no attached client
// ---------------------------------------------------------------------------

#[test]
fn copy_fallback_when_no_provable_client_still_loads_buffer() {
    let rig = Rig::new("no-client", &["line to buffer"]);
    let app = rig.open();
    rig.clear_buffer();

    rig.wait_screen(&app, "turn text drawn", |s| s.contains("line to buffer"));
    rig.raw(&app, "[");
    rig.raw(&app, "y");

    rig.wait_screen(&app, "flash notes buffer only without client", |s| {
        s.contains("copied 1 line to the tmux buffer only - no tmux client is showing this app")
    });

    let buf = rig.buffer().expect("tmux paste buffer was loaded");
    assert_eq!(
        buf, "line to buffer",
        "R5: tmux paste buffer is always loaded when inside tmux even with no client"
    );
}

// ---------------------------------------------------------------------------
// D1, D9: Empty body refusal
// ---------------------------------------------------------------------------

#[test]
fn copy_empty_body_refuses_with_hint() {
    let rig = Rig::new("empty-turn", &[]);
    let events = rig.tool.dir.join("events.jsonl");
    let mut rows = fs::read_to_string(&events).unwrap_or_default();
    writeln!(
        rows,
        r#"{{"ts":"2026-10-07T20:01:00Z","actor":"console:local","action":"ask","ref":"0199c0de-9999","target":"lead","summary":""}}"#
    )
    .unwrap();
    fs::write(events, rows).expect("append ask event");

    let app = rig.open();
    let _viewer = rig.attach(&app);
    rig.clear_buffer();

    rig.wait_screen(&app, "empty ask drawn in lane", |s| s.contains("to lead"));
    rig.raw(&app, "[");
    rig.raw(&app, "y");

    rig.wait_screen(&app, "empty body refusal flash", |s| {
        s.contains("nothing to copy: that turn has no text")
    });
    assert_eq!(rig.buffer(), None, "D1: empty turn loads no tmux buffer");
}
