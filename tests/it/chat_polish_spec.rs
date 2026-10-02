//! Independent chat-polish oracle (#5 order, #6 word wrap): an ask's header
//! precedes its outcome on the real screen, and the composer breaks rows at
//! spaces with a per-character fallback. Real tmux judges order; the
//! composer's public [`Input::view`] judges wrapping against hand-computed
//! rows, never production layout helpers. Private sockets only.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ae::console::input::{Effect, Input, Reading, Size};
use ae::events::Event;

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const PROMPT: &str = "to lead> ";
const WAIT: Duration = Duration::from_secs(20);

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    name: String,
    pane: String,
}

impl Rig {
    fn new(tag: &str, width: usize, height: usize) -> Self {
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
        let mut rig = Self {
            tool,
            root,
            socket,
            name,
            pane: String::new(),
        };
        fs::write(rig.tool.dir.join("meta"), format!(
            "session={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-rig\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\n",
            rig.name, rig.socket.display()
        )).expect("lead pair meta");
        fs::write(rig.root.join("config"), "").expect("isolated config");
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        fs::write(rig.tool.dir.join(".launch-attempt"), epoch.to_string()).expect("launch stamp");
        rig.tmux(&["set-option", "-t", &rig.name, "@ae_session_uuid", UUID]);
        rig.tmux(&[
            "set-option",
            "-t",
            &rig.name,
            "@ae_main_pane",
            &rig.tool.pane,
        ]);
        rig.tmux(&[
            "set-option",
            "-p",
            "-t",
            &rig.tool.pane,
            "@ae_agent",
            "lead",
        ]);
        rig.tmux(&[
            "set-option",
            "-t",
            &rig.name,
            "default-size",
            &format!("80x{}", height.max(30)),
        ]);
        let out = super::cli::ae()
            .env("AE_HOME", &rig.root)
            .env("CONFIG_FILE", rig.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &rig.socket)
            .env("TMUX", format!("{},1,0", rig.socket.display()))
            .env("TMUX_PANE", &rig.tool.pane)
            .arg("_console")
            .output()
            .expect("real console opens");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        rig.pane = rig
            .tmux(&[
                "list-panes",
                "-s",
                "-t",
                &rig.name,
                "-F",
                "#{pane_id}|#{@ae_console}",
            ])
            .lines()
            .find_map(|row| {
                let (pane, stamp) = row.split_once('|')?;
                (stamp == UUID).then(|| pane.to_owned())
            })
            .expect("stamped console");
        rig.wait("owner prompt", |screen| {
            screen
                .lines()
                .any(|row| row.trim_end() == PROMPT.trim_end())
        });
        rig.resize(width, height);
        rig
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn resize(&self, width: usize, height: usize) {
        self.tmux(&[
            "resize-window",
            "-t",
            &self.pane,
            "-x",
            &width.to_string(),
            "-y",
            &height.to_string(),
        ]);
        let shape = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            &self.pane,
            "#{pane_width}x#{pane_height}",
        ]);
        assert_eq!(
            shape.trim(),
            format!("{width}x{height}"),
            "fixture dimensions"
        );
    }

    fn screen(&self) -> String {
        self.tmux(&["capture-pane", "-p", "-t", &self.pane])
    }

    fn wait(&self, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        loop {
            let screen = self.screen();
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn paste(&self, body: &str) {
        let path = self.root.join("paste.txt");
        fs::write(&path, body).expect("paste fixture");
        self.tmux(&["load-buffer", "-b", "spec", path.to_str().expect("path")]);
        self.tmux(&["paste-buffer", "-p", "-b", "spec", "-t", &self.pane]);
    }

    fn key(&self, key: &str) {
        self.tmux(&["send-keys", "-t", &self.pane, key]);
    }

    fn cursor(&self) -> (usize, usize) {
        let at = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            &self.pane,
            "#{cursor_x},#{cursor_y}",
        ]);
        let (x, y) = at.trim().split_once(',').expect("cursor coordinates");
        (x.parse().expect("x"), y.parse().expect("y"))
    }

    /// The journal's console-ask ids, oldest first, once `count` exist.
    fn ask_ids(&self, count: usize) -> Vec<String> {
        let until = Instant::now() + WAIT;
        loop {
            let ids: Vec<String> = self
                .tool
                .events()
                .lines()
                .filter_map(|row| Event::parse_line(row).ok())
                .filter(|event| event.actor == "console:local" && event.action == "ask")
                .filter_map(|event| event.reference.clone())
                .collect();
            if ids.len() >= count {
                return ids;
            }
            assert!(
                Instant::now() < until,
                "journal holds {count} console asks; events:\n{}",
                self.tool.events()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn submit(&self, literal: &str) -> String {
        self.paste(literal);
        self.key("Enter");
        let id = self.ask_ids(1).pop().expect("one recorded ask");
        let outcome = format!("sent {id}");
        self.wait("outcome names the recorded ask", |screen| {
            screen.replace('\n', "").contains(&outcome)
        });
        id
    }
}

/// A live composer holding `body`, cursor at the end.
fn typed(body: &[u8]) -> (Input, Instant) {
    let mut input = Input::new(vec!["lead".to_owned()]);
    let now = Instant::now();
    let _ = input.tick(Reading::Owner, now);
    assert!(input.chunk(body, now).is_empty(), "typing never submits");
    (input, now)
}

#[test]
fn wrap_breaks_at_the_last_space_that_fits() {
    // Width 20: nine-cell prompt, eleven-cell area.
    let (input, _) = typed(b"the lazy dog and keeps running");
    let view = input
        .view(Size {
            width: 20,
            height: 24,
        })
        .expect("taking input");
    assert_eq!(
        view.rows,
        vec![
            "to lead> the lazy ",
            "         dog and ",
            "         keeps ",
            "         running",
        ],
        "words never split mid-row"
    );
    assert_eq!(view.cursor_row, 3);
    assert_eq!(view.before, "         running");
}

#[test]
fn a_word_longer_than_the_row_still_breaks_per_character() {
    let (input, _) = typed(b"abcdefghij klmnopqrstuvwxyz");
    let view = input
        .view(Size {
            width: 20,
            height: 24,
        })
        .expect("taking input");
    assert_eq!(
        view.rows,
        vec![
            "to lead> abcdefghij ",
            "         klmnopqrstu",
            "         vwxyz",
        ],
        "character fallback past the last fitting space"
    );
    assert_eq!(view.before, "         vwxyz");
}

#[test]
fn a_cursor_at_a_wrap_point_starts_the_next_row() {
    let (mut input, now) = typed(b"the lazy dog");
    for _ in 0..3 {
        assert!(input.chunk(b"\x1b[D", now).is_empty(), "Left never submits");
    }
    let view = input
        .view(Size {
            width: 20,
            height: 24,
        })
        .expect("taking input");
    assert_eq!(view.rows, vec!["to lead> the lazy ", "         dog"]);
    assert_eq!(view.cursor_row, 1, "byte 9 opens the wrapped row");
    assert_eq!(view.before, "         ", "nothing before the wrap point");
}

#[test]
fn a_cursor_inside_a_fallback_split_word_maps_by_byte() {
    let (mut input, now) = typed(b"abcdefghijklmnopqrst");
    assert!(input.chunk(b"\x01", now).is_empty(), "Home never submits");
    for _ in 0..14 {
        assert!(
            input.chunk(b"\x1b[C", now).is_empty(),
            "Right never submits"
        );
    }
    let view = input
        .view(Size {
            width: 20,
            height: 24,
        })
        .expect("taking input");
    assert_eq!(
        view.rows,
        vec!["to lead> abcdefghijk", "         lmnopqrst"]
    );
    assert_eq!(view.cursor_row, 1);
    assert_eq!(view.before, "         lmn", "byte 14 sits after lmn");
}

#[test]
fn a_hard_break_scopes_word_wrap_to_its_segment() {
    // Width 14: nine-cell prompt, five-cell area. The break arrives as a
    // bracketed paste, as the human would; a bare \n is Enter.
    let mut input = Input::new(vec!["lead".to_owned()]);
    let now = Instant::now();
    let _ = input.tick(Reading::Owner, now);
    assert!(
        input
            .chunk(b"\x1b[200~aa bb cc\ndd ee ff\x1b[201~", now)
            .is_empty(),
        "a paste never submits"
    );
    let view = input
        .view(Size {
            width: 14,
            height: 24,
        })
        .expect("taking input");
    assert_eq!(
        view.rows,
        vec![
            "to lead> aa ",
            "         bb cc",
            "         dd ",
            "         ee ff",
        ],
        "no row crosses the newline"
    );
    assert_eq!(view.cursor_row, 3);
    assert_eq!(view.before, "         ee ff");
}

#[test]
fn wide_characters_still_break_early_at_spaces() {
    // Width 15: nine-cell prompt, six-cell area; non-ASCII counts two cells.
    let (input, _) = typed("aa éé bb".as_bytes());
    let view = input
        .view(Size {
            width: 15,
            height: 24,
        })
        .expect("taking input");
    assert_eq!(view.rows, vec!["to lead> aa ", "         é bb"]);
    assert_eq!(view.cursor_row, 1);
    assert_eq!(view.before, "         é bb");
}

#[test]
fn window_markers_count_word_wrapped_rows() {
    // Width 30: nine-cell prompt, twenty-one-cell area; a hundred three-letter
    // words wrap five per row, twenty rows past the ten-row cap.
    let draft = "aaa ".repeat(100);
    let full = "         aaa aaa aaa aaa aaa ";
    let (mut input, now) = typed(draft.as_bytes());
    let size = Size {
        width: 30,
        height: 24,
    };
    let view = input.view(size).expect("taking input");
    assert_eq!(view.rows.len(), 10, "marker plus nine tail rows");
    assert_eq!(view.rows[0], "to lead> … +11 lines above");
    assert_eq!(view.rows[1], full);
    assert_eq!(view.rows[9], full);
    assert_eq!(view.cursor_row, 9);
    assert_eq!(view.before, full);
    assert!(input.chunk(b"\x01", now).is_empty(), "Home never submits");
    let view = input.view(size).expect("taking input");
    assert_eq!(view.rows.len(), 10, "nine head rows plus marker");
    assert_eq!(view.rows[0], format!("to lead> {}", full.trim_start()));
    assert_eq!(view.rows[9], "         … +11 lines below");
    assert_eq!(view.cursor_row, 0);
    assert_eq!(view.before, PROMPT);
}

#[test]
fn enter_sends_exactly_the_draft_bytes() {
    let (mut input, now) = typed(b"the lazy dog");
    let effects = input.chunk(b"\r", now);
    let [Effect::Ask { seat, body, .. }] = effects.as_slice() else {
        panic!("Enter asks");
    };
    assert_eq!(seat, "lead");
    assert_eq!(body, "the lazy dog", "wrapping keeps every space");
    let mut input = Input::new(vec!["lead".to_owned()]);
    let now = Instant::now();
    let _ = input.tick(Reading::Owner, now);
    assert!(
        input.chunk(b"\x1b[200~aa\nbb\x1b[201~", now).is_empty(),
        "a paste never submits"
    );
    let effects = input.chunk(b"\r", now);
    let [Effect::Ask { body, .. }] = effects.as_slice() else {
        panic!("Enter asks");
    };
    assert_eq!(body, "aa\nbb", "wrapping keeps the newline");
}

#[test]
fn narrow_composer_render_shows_word_wrap_and_cursor() {
    // Width 24: nine-cell prompt, fifteen-cell area. The printed screen is
    // the freeze render artifact (nextest --success-output immediate-final).
    let rig = Rig::new("cpol-render", 24, 12);
    rig.paste("the lazy dog and keeps running");
    let screen = rig.wait("full draft drawn", |screen| screen.contains("running"));
    let trimmed: Vec<&str> = screen.lines().map(str::trim_end).collect();
    let rows = ["to lead> the lazy dog", "and keeps", "running"];
    let mut at = 0;
    for want in rows {
        let found = trimmed[at..]
            .iter()
            .position(|row| row.contains(want))
            .unwrap_or_else(|| panic!("screen holds {want}:\n{screen}"));
        at += found + 1;
    }
    assert!(
        !trimmed.iter().any(|row| row.contains("dog a")),
        "no word splits mid-row:\n{screen}"
    );
    let (x, y) = rig.cursor();
    assert_eq!(x, 16, "cursor after `running` at indent 9:\n{screen}");
    println!("CHAT_POLISH_RENDER_BEGIN cursor={x},{y}\n{screen}CHAT_POLISH_RENDER_END");
}

#[test]
fn an_ask_header_prints_before_its_outcome() {
    let rig = Rig::new("cpol-ord", 80, 30);
    let id = rig.submit("order probe one");
    let header = format!("you → lead · {id}");
    let outcome = format!("sent {id}");
    let screen = rig.wait("header and outcome both visible", |screen| {
        screen.contains(&header) && screen.contains(&outcome)
    });
    assert!(
        screen.contains("  order probe one"),
        "the ask body renders under its header:\n{screen}"
    );
    let (at_header, at_outcome) = (
        screen.find(&header).expect("header"),
        screen.find(&outcome).expect("outcome"),
    );
    assert!(
        at_header < at_outcome,
        "header first, then what became of it:\n{screen}"
    );
}

#[test]
fn two_asks_keep_header_outcome_pairs_in_sequence() {
    let rig = Rig::new("cpol-seq", 80, 30);
    rig.paste("sequence probe one");
    rig.key("Enter");
    let first = rig.ask_ids(1).pop().expect("first recorded ask");
    rig.wait("first outcome printed", |screen| {
        screen.contains(&format!("sent {first}"))
    });
    rig.paste("sequence probe two");
    rig.key("Enter");
    let ids = rig.ask_ids(2);
    let (h1, o1) = (
        format!("you → lead · {}", ids[0]),
        format!("sent {}", ids[0]),
    );
    let (h2, o2) = (
        format!("you → lead · {}", ids[1]),
        format!("sent {}", ids[1]),
    );
    let screen = rig.wait("both pairs visible", |screen| {
        screen.contains(&h1) && screen.contains(&o1) && screen.contains(&h2) && screen.contains(&o2)
    });
    let positions = [
        screen.find(&h1).expect("first header"),
        screen.find(&o1).expect("first outcome"),
        screen.find(&h2).expect("second header"),
        screen.find(&o2).expect("second outcome"),
    ];
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "h1, o1, h2, o2 in order:\n{screen}"
    );
}
