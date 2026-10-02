//! Independent P3.6 oracle: the pane shows the whole wrapped draft, bounded
//! by ten rows, while edits change the literal draft at its UTF-8 cursor.
//! Real tmux captures judge the screen; no production layout helper supplies
//! expected rows. Private sockets and existing process doors only.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        // New windows inherit these dimensions; delivery's agent remains wide.
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
        // A real bracketed paste carries newlines literally to the terminal.
        let path = self.root.join("paste.txt");
        fs::write(&path, body).expect("paste fixture");
        self.tmux(&["load-buffer", "-b", "spec", path.to_str().expect("path")]);
        self.tmux(&["paste-buffer", "-p", "-b", "spec", "-t", &self.pane]);
    }

    fn key(&self, key: &str) {
        self.tmux(&["send-keys", "-t", &self.pane, key]);
    }

    fn submit(&self, literal: &str) {
        self.key("Enter");
        self.wait("one complete literal ask confirmed", |screen| {
            let unwrapped = screen.replace('\n', "");
            self.tool
                .events()
                .lines()
                .filter_map(|row| Event::parse_line(row).ok())
                .any(|event| {
                    event.actor == "console:local"
                        && event.action == "ask"
                        && event
                            .reference
                            .as_deref()
                            .is_some_and(|id| unwrapped.contains(&format!("sent {id}")))
                })
        });
        let asks: Vec<_> = self
            .tool
            .events()
            .lines()
            .filter_map(|row| Event::parse_line(row).ok())
            .filter(|event| event.actor == "console:local" && event.action == "ask")
            .collect();
        assert_eq!(asks.len(), 1, "one Enter records one ask");
        assert_eq!(
            asks[0].summary.as_deref(),
            Some(literal.replace(['\n', '\t'], " ").as_str()),
            "journal summary independently flattened"
        );
        let delivered = fs::read_to_string(
            asks[0]
                .body_file
                .as_deref()
                .expect("recorded delivered text"),
        )
        .expect("stored delivered bytes");
        assert!(
            delivered.contains(&format!("from console:local: {literal}\n\nREQUIRED:")),
            "whole literal body before reply footer: {delivered}"
        );
        assert_eq!(asks[0].target.as_deref(), Some("lead"));
    }

    fn lane_reply(&self, body: &str) {
        let mut journal = self.tool.events();
        for n in 0..8 {
            writeln!(journal, "{{\"ts\":\"2026-10-02T10:00:0{n}Z\",\"actor\":\"lead\",\"action\":\"chat\",\"summary\":\"lane padding {n}\"}}")
                .expect("external lane padding");
        }
        writeln!(journal, "{{\"ts\":\"2026-10-02T10:00:09Z\",\"actor\":\"lead\",\"action\":\"chat\",\"summary\":\"{body}\"}}")
            .expect("external lane record");
        fs::write(self.tool.dir.join("events.jsonl"), journal).expect("external lane event");
    }
}

/// ASCII is intentional: exact widths follow from the pane and nine-cell
/// prompt, independently of ae's conservative non-ASCII width approximation.
fn rows(body: &str, width: usize) -> Vec<String> {
    let indent = " ".repeat(PROMPT.len());
    let mut out = Vec::new();
    for logical in body.split('\n') {
        if logical.is_empty() {
            out.push(if out.is_empty() {
                PROMPT.to_owned()
            } else {
                indent.clone()
            });
        } else {
            for part in logical.as_bytes().chunks(width - PROMPT.len()) {
                let prefix = if out.is_empty() { PROMPT } else { &indent };
                out.push(format!(
                    "{prefix}{}",
                    std::str::from_utf8(part).expect("ASCII oracle")
                ));
            }
        }
    }
    out
}

fn contains_rows(screen: &str, want: &[String]) -> bool {
    let screen: Vec<_> = screen.lines().map(str::trim_end).collect();
    screen
        .windows(want.len())
        .any(|part| part.iter().zip(want).all(|(a, b)| *a == b.trim_end()))
}

fn long_body() -> String {
    "BEGIN-abcdefghijklmnopqrstuvwxyz-ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789-abcdefghijklmnopqrstuvwxyz-END".to_owned()
}

#[test]
fn the_entire_long_draft_wraps_at_the_real_pane_width() {
    let rig = Rig::new("p36wrap", 40, 30);
    let body = long_body();
    let want = rows(&body, 40);
    rig.paste(&body);
    rig.wait("all draft bytes in aligned physical rows", |screen| {
        contains_rows(screen, &want)
    });
    rig.submit(&body);
}

#[test]
fn bracketed_multiline_paste_shows_each_logical_row_without_submitting() {
    let rig = Rig::new("p36paste", 40, 30);
    let body = "first visible row\nsecond visible row\nthird visible row";
    rig.paste(body);
    rig.wait("all pasted rows visible", |screen| {
        contains_rows(screen, &rows(body, 40))
    });
    assert!(
        !rig.tool.events().contains("\"action\":\"ask\""),
        "paste never Enter"
    );
    rig.submit(body);
}

#[test]
fn ten_row_cap_includes_the_marker_and_retains_the_last_nine_rows() {
    let rig = Rig::new("p36cap", 40, 30);
    let body = (0..14)
        .map(|n| format!("draft-row-{n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    rig.paste(&body);
    let screen = rig.wait("cap marker and final rows", |screen| {
        screen.contains("lines above") && screen.contains("draft-row-13")
    });
    let marker = screen
        .lines()
        .find(|row| row.contains("lines above"))
        .expect("elision row");
    assert!(
        marker.split_whitespace().any(|word| word == "+5"),
        "fourteen physical rows lose five under a ten-row cap: {marker}"
    );
    let visible = screen
        .lines()
        .filter(|row| row.contains("draft-row-"))
        .collect::<Vec<_>>();
    assert_eq!(visible.len(), 9, "marker plus nine rows: {screen}");
    for n in 0..5 {
        assert!(
            !screen.contains(&format!("draft-row-{n:02}")),
            "top elided: {screen}"
        );
    }
    for n in 5..14 {
        assert!(
            screen.contains(&format!("draft-row-{n:02}")),
            "tail retained: {screen}"
        );
    }
}

#[test]
fn lane_output_stays_above_one_complete_composer_after_bottom_scroll() {
    let rig = Rig::new("p36lane", 40, 12);
    let body = long_body();
    let want = rows(&body, 40);
    rig.paste(&body);
    rig.wait("wrapped composer", |screen| contains_rows(screen, &want));
    rig.lane_reply("LANE-ABOVE-COMPOSER");
    let screen = rig.wait("lane followed by intact draft", |screen| {
        screen.contains("LANE-ABOVE-COMPOSER") && contains_rows(screen, &want)
    });
    let lane = screen.find("LANE-ABOVE-COMPOSER").expect("lane output");
    let composer = screen.find("to lead> BEGIN-").expect("composer start");
    assert!(lane < composer, "lane above composer: {screen}");
    assert_eq!(
        screen.matches("BEGIN-").count(),
        1,
        "no stale composer start: {screen}"
    );
    assert_eq!(
        screen.matches("-END").count(),
        1,
        "no stale composer tail: {screen}"
    );
    rig.key("C-u");
    let cleared = rig.wait("old rows cleared", |screen| {
        !screen.contains("BEGIN-") && !screen.contains("-END")
    });
    assert!(
        cleared.contains("LANE-ABOVE-COMPOSER"),
        "clear keeps lane: {cleared}"
    );
}

#[test]
fn resize_rewraps_the_unchanged_draft_in_both_directions() {
    let rig = Rig::new("p36resize", 40, 30);
    let body = long_body();
    rig.paste(&body);
    rig.wait("initial narrow rows", |screen| {
        contains_rows(screen, &rows(&body, 40))
    });
    for width in [65, 32] {
        rig.resize(width, 30);
        let screen = rig.wait("next tick uses current width", |screen| {
            contains_rows(screen, &rows(&body, width))
        });
        assert_eq!(
            screen.matches("BEGIN-").count(),
            1,
            "resize leaves no stale rows: {screen}"
        );
    }
}

#[test]
fn short_panes_bound_the_cap_by_height_and_keep_the_draft_tail_visible() {
    let rig = Rig::new("p36height", 40, 6);
    let body = (0..14)
        .map(|n| format!("short-row-{n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    rig.paste(&body);
    let screen = rig.wait("height-bounded marker and tail", |screen| {
        screen.contains("lines above") && screen.contains("short-row-13")
    });
    let visible = screen
        .lines()
        .filter(|row| row.contains("short-row-"))
        .count();
    assert_eq!(
        visible, 4,
        "height six leaves one lane row, one marker and four draft rows: {screen}"
    );
    assert!(
        screen
            .lines()
            .any(|row| row.split_whitespace().any(|word| word == "+10")
                && row.contains("lines above")),
        "ten omitted rows: {screen}"
    );
    for n in 10..14 {
        assert!(
            screen.contains(&format!("short-row-{n:02}")),
            "tail retained: {screen}"
        );
    }
}

#[test]
fn unicode_resize_and_clear_leave_no_ghost_composer_rows() {
    let rig = Rig::new("p36unicode", 40, 30);
    let body = "ééééééééééééABCDEFGHIJK-END";
    let at40 = vec![
        "to lead> ééééééééééééABCDEFG".to_owned(),
        "         HIJK-END".to_owned(),
    ];
    let at24 = vec![
        "to lead> ééééééé".to_owned(),
        "         éééééABCDE".to_owned(),
        "         FGHIJK-END".to_owned(),
    ];
    let at55 = vec![format!("{PROMPT}{body}")];
    rig.paste(body);
    rig.wait("err-wide wrap", |screen| contains_rows(screen, &at40));
    for (width, want) in [(24, at24), (55, at55)] {
        rig.resize(width, 30);
        let screen = rig.wait("Unicode rewrapped at current width", |screen| {
            contains_rows(screen, &want)
        });
        assert_eq!(
            screen.matches('é').count(),
            12,
            "no duplicate Unicode from old draw: {screen}"
        );
        assert_eq!(screen.matches("-END").count(), 1, "no stale tail: {screen}");
    }
    rig.key("C-u");
    rig.wait("clear erases every old composer row", |screen| {
        !screen.contains('é') && !screen.contains("-END")
    });
}

#[test]
fn narrow_panes_show_each_draft_byte_in_order_and_submit_the_literal_body() {
    let rig = Rig::new("p36narrow", 12, 20);
    let body = "0123456789ABCDEFGHIJ";
    rig.paste(body);
    // Only the body uses these digits/capitals. Do not assume how ae clips
    // the prompt or chooses indentation when nine cells cannot fit.
    let screen = rig.wait("narrow pane retains every draft character", |screen| {
        let draft = narrow_body(screen, body);
        draft.contains(body)
    });
    let draft = narrow_body(&screen, body);
    assert_eq!(
        draft, body,
        "one complete draft, no stale narrow rows: {screen}"
    );
    assert!(
        screen.lines().all(|row| row.chars().count() <= 12),
        "capture sanity: physical rows fit actual pane: {screen}"
    );
    rig.submit(body);
}

#[test]
fn true_wide_resize_erases_all_composer_rows_and_preserves_the_lane() {
    let rig = Rig::new("p36cjk", 40, 16);
    rig.lane_reply("LANE-ABOVE-COMPOSER");
    rig.wait("lane visible before editing", |screen| {
        screen.contains("LANE-ABOVE-COMPOSER")
    });
    let body = "中中中中中中中中中中中中ABCDEFGHIJK-END";
    let at40 = vec![
        "to lead> 中中中中中中中中中中中中ABCDEFG".to_owned(),
        "         HIJK-END".to_owned(),
    ];
    let at24 = vec![
        "to lead> 中中中中中中中".to_owned(),
        "         中中中中中ABCDE".to_owned(),
        "         FGHIJK-END".to_owned(),
    ];
    rig.paste(body);
    rig.wait("true-wide wrap", |screen| contains_rows(screen, &at40));
    rig.resize(24, 16);
    let screen = rig.wait("true-wide shrink rewrap", |screen| {
        contains_rows(screen, &at24)
    });
    assert_eq!(
        screen.matches('中').count(),
        12,
        "no stale true-wide rows: {screen}"
    );
    assert_eq!(
        screen.matches("-END").count(),
        1,
        "one draft tail: {screen}"
    );
    assert!(
        screen.contains("LANE-ABOVE-COMPOSER"),
        "resize preserves nearby lane: {screen}"
    );
    rig.key("C-u");
    let cleared = rig.wait("all true-wide composer rows erased", |screen| {
        !screen.contains('中') && !screen.contains("-END")
    });
    assert!(
        cleared.contains("LANE-ABOVE-COMPOSER"),
        "erase preserves lane: {cleared}"
    );
}

fn narrow_body(screen: &str, body: &str) -> String {
    // Ignore header digits without prescribing how much prompt is clipped.
    // At width12 the prompt must still identify its first row with "to ".
    let rows: Vec<_> = screen.lines().collect();
    rows.iter()
        .rposition(|row| row.starts_with("to "))
        .map_or_else(String::new, |start| {
            rows[start..]
                .iter()
                .flat_map(|row| row.chars())
                .filter(|ch| body.contains(*ch))
                .collect()
        })
}

#[test]
fn resizing_below_the_old_prompt_width_erases_old_rows_and_keeps_the_lane() {
    let rig = Rig::new("p36tinyresize", 40, 30);
    rig.lane_reply("vvv");
    rig.wait("nearby lane marker", |screen| screen.contains("vvv"));
    // Q/Z/X never occur in the header, prompt, lane or timestamps.
    let body = "QZXQZXQZX";
    rig.paste(body);
    rig.wait("full initial draft", |screen| {
        screen.contains(&format!("{PROMPT}{body}"))
    });
    rig.resize(5, 30);
    let screen = rig.wait("new rows at width below old prompt", |screen| {
        screen
            .lines()
            .any(|row| row.starts_with("to") && row.contains("QZX"))
    });
    let visible: String = screen.chars().filter(|ch| body.contains(*ch)).collect();
    assert_eq!(
        visible, body,
        "one complete draft with no narrow-resize ghost: {screen}"
    );
    assert!(screen.contains("vvv"), "tiny resize keeps lane: {screen}");
    rig.resize(40, 30);
    let screen = rig.wait("wide prompt restored", |screen| {
        screen.contains(&format!("{PROMPT}{body}"))
    });
    let visible: String = screen.chars().filter(|ch| body.contains(*ch)).collect();
    assert_eq!(
        visible, body,
        "widen does not duplicate old draft: {screen}"
    );
    assert!(screen.contains("vvv"), "widen keeps lane: {screen}");
}
