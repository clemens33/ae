//! Frozen C1 acceptance: ruling R-C1.1..6, chat's documented wrapping, and
//! real terminal cells/cursor are the oracle. No Layout.cursor or private
//! wrapping helper is consulted. Every app/store/socket belongs to this rig.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use crate::app_mode::writing;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const FRAME: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Frame {
    text: String,
    cursor: (usize, usize, usize), // terminal flag, column, row
}

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    home: String,
}

impl Rig {
    fn new(tag: &str, turns: usize) -> Self {
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
        let tid = "0199c0de-aaaa-4890-abcd-ef0123456789";
        let lines = (0..turns)
            .map(|at| {
                super::board::user(
                    &format!("2026-10-07T20:00:{at:02}Z"),
                    &format!("lane-{at:02}"),
                )
            })
            .collect::<Vec<_>>();
        super::board::plant_transcript(&store, "work", tid, &lines);
        fs::write(tool.dir.join("meta"), format!(
            "session={home}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-rig\nharness_session.main={tid}\nconfig_home.main={}\nseat.worker.0=colead\nagent_bin.worker.0=claude\nlaunch_id.worker.0=tok-colead\nconfig_home.worker.0={}\n",
            socket.display(), store.display(), store.display(),
        )).expect("lead pair meta");
        fs::write(root.join("config"), "[workspace]\nchat = app\n").expect("app config");
        fs::write(
            tool.dir.join(".launch-attempt"),
            ae::time::Timestamp::now().epoch().to_string(),
        )
        .expect("launch stamp");
        let rig = Self {
            tool,
            root,
            socket,
            home,
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
        let out = super::cli::ae()
            .env("AE_HOME", &self.root)
            .env("HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("TMUX", format!("{},1,0", self.socket.display()))
            .env("TMUX_PANE", &self.tool.pane)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .arg("_console")
            .output()
            .expect("open fixture app");
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
        self.wait(&pane, WAIT, "GUARD home ready", |f| {
            f.text.contains("Enter writes") && f.text.contains("Overview")
        });
        pane
    }

    fn frame(&self, pane: &str) -> Frame {
        let text = self.tmux(&["capture-pane", "-p", "-t", pane]);
        let cursor = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{cursor_flag},#{cursor_x},#{cursor_y}",
        ]);
        let coords = cursor
            .trim()
            .split(',')
            .map(|n| n.parse::<usize>().expect("terminal cursor number"))
            .collect::<Vec<_>>();
        assert_eq!(coords.len(), 3, "terminal cursor format");
        Frame {
            text,
            cursor: (coords[0], coords[1], coords[2]),
        }
    }

    /// Cell output and cursor escapes stream independently. Two equal real
    /// terminal snapshots after a named input witness avoid a partial repaint.
    fn wait(&self, pane: &str, limit: Duration, why: &str, met: impl Fn(&Frame) -> bool) -> Frame {
        let until = Instant::now() + limit;
        let mut previous = None;
        loop {
            let frame = self.frame(pane);
            if met(&frame) && previous.as_ref() == Some(&frame) {
                return frame;
            }
            assert!(Instant::now() < until, "{why}; terminal: {frame:?}");
            previous = Some(frame);
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn send(&self, pane: &str, raw: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", raw]);
    }

    fn writing(&self, pane: &str) -> Frame {
        self.send(pane, "i");
        self.wait(pane, FRAME, "GUARD lease acquired before draft keys", |f| {
            writing(&f.text)
        })
    }

    fn paste(&self, pane: &str, draft: &str, witness: &str) -> Frame {
        self.send(pane, &format!("\x1b[200~{draft}\x1b[201~"));
        self.wait(pane, FRAME, "GUARD paste consumed", |f| {
            f.text.contains(witness)
        })
    }

    fn resize(&self, pane: &str, width: usize, height: usize) -> Frame {
        self.tmux(&[
            "resize-window",
            "-t",
            pane,
            "-x",
            &width.to_string(),
            "-y",
            &height.to_string(),
        ]);
        self.wait(pane, FRAME, "GUARD resize repainted", |f| {
            f.text.lines().count() == height
                && if width < 40 || height < 8 {
                    f.text.contains("needs at least 40x8")
                } else {
                    // tmux can resize its old screen before ae repaints: a
                    // bottom deletion loses the keys row; a top deletion
                    // keeps it but can leave the old grown window too tall.
                    let composing = writing(&f.text);
                    let cap = if composing {
                        height.saturating_sub(11).clamp(1, 10)
                    } else {
                        1
                    };
                    let address = format!("to {} › lead", self.home);
                    f.text.lines().last().is_some_and(|row| {
                        row.chars().count() == width
                            && (row.contains("Enter send") || row.contains("Enter write"))
                    }) && f.text.lines().enumerate().any(|(row, text)| {
                        row >= height - 3 - cap && row <= height - 4 && text.contains(&address)
                    })
                }
        })
    }

    fn report(&self, pane: &str, code: u8, column: usize, row: usize, count: usize) {
        self.send(
            pane,
            &format!("\x1b[<{code};{};{}M", column + 1, row + 1).repeat(count),
        );
    }

    fn address(&self) -> String {
        format!("to {} › lead", self.home)
    }

    fn composer(&self, frame: &Frame) -> (usize, usize, Vec<String>) {
        let (column, top) = cell(&frame.text, &self.address());
        let text_column = column + self.address().chars().count() + 3;
        let height = frame.text.lines().count();
        let rows = frame
            .text
            .lines()
            .skip(top)
            .take((height - 3).saturating_sub(top))
            .map(|row| {
                row.chars()
                    .skip(text_column)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect();
        (text_column, top, rows)
    }

    fn holds(&self, frame: &Frame, rows: &[String], cursor_row: usize, before_cells: usize) {
        let (text_column, top, drawn) = self.composer(frame);
        assert_eq!(
            drawn, rows,
            "whole draft window, first-row address and subsequent indentation"
        );
        let height = frame.text.lines().count();
        assert_eq!(
            top,
            height - 3 - rows.len(),
            "grown composer consumes lane rows, not hint/keys"
        );
        assert!(writing(&frame.text));
        assert!(
            frame
                .text
                .lines()
                .nth(height - 1)
                .expect("keys")
                .contains("Enter send")
        );
        assert_eq!(
            frame.cursor,
            (1, text_column + before_cells, top + cursor_row),
            "physical writing cursor"
        );
        assert_eq!(
            frame.text.matches(&self.address()).count(),
            1,
            "address only on the first row"
        );
    }
}

fn cell(text: &str, label: &str) -> (usize, usize) {
    text.lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find(label)
                .map(|byte| (line[..byte].chars().count(), row))
        })
        .expect("complete frame has drawn label")
}

fn lines(count: usize) -> String {
    (0..count)
        .map(|at| format!("L{at:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}
fn strings(rows: &[&str]) -> Vec<String> {
    rows.iter().map(|row| (*row).to_owned()).collect()
}
fn tail() -> Vec<String> {
    let mut rows = vec!["… +11 lines above".to_owned()];
    rows.extend((11..20).map(|at| format!("L{at:02}")));
    rows
}

#[test]
fn last_space_wrap_keeps_both_rows_and_display_wrap_does_not_change_submission() {
    let rig = Rig::new("cwspace", 0);
    let pane = rig.open();
    rig.resize(&pane, 100, 30);
    let blank = rig.writing(&pane);
    let (column, _, _) = rig.composer(&blank);
    let width = 98 - column; // two-cell gutter, drawn address, ASCII draft
    let prefix = "A".repeat(width - 4);
    let raw = format!("{prefix} xyzQ");
    let frame = rig.paste(&pane, &raw, "xyzQ");
    rig.holds(&frame, &[prefix, "xyzQ".to_owned()], 1, 4);
    rig.send(&pane, "\r");
    rig.wait(&pane, FRAME, "literal ask submitted", |f| {
        f.text.contains("sent")
    });
    let journal = fs::read_to_string(rig.tool.dir.join("events.jsonl")).expect("ask journal");
    let records = journal
        .lines()
        .map(|row| ae::events::Event::parse_line(row).expect("fixture event"))
        .collect::<Vec<_>>();
    let ask = records
        .iter()
        .find(|record| record.action == "ask" && record.actor == "console:local")
        .expect("console ask");
    assert_eq!(
        ask.summary.as_deref(),
        Some(raw.as_str()),
        "only display wrapped; submitted prose retains its space"
    );
    assert!(
        rig.tool.submitted().contains(&raw),
        "real receiver got the unwrapped draft"
    );
}

#[test]
fn long_word_crlf_and_empty_hard_line_each_draw_their_rows() {
    let rig = Rig::new("cwword", 0);
    let pane = rig.open();
    rig.resize(&pane, 100, 30);
    let blank = rig.writing(&pane);
    let width = 98 - rig.composer(&blank).0;
    let frame = rig.paste(
        &pane,
        &format!("{}\r\n\r\nline-end", "x".repeat(width + 2)),
        "line-end",
    );
    rig.holds(
        &frame,
        &[
            "x".repeat(width),
            "xx".to_owned(),
            String::new(),
            "line-end".to_owned(),
        ],
        3,
        8,
    );
}

#[test]
fn inserting_at_the_start_pushes_a_word_across_the_wrap_without_losing_it() {
    let rig = Rig::new("cwedit", 0);
    let pane = rig.open();
    rig.resize(&pane, 100, 30);
    let blank = rig.writing(&pane);
    let width = 98 - rig.composer(&blank).0;
    rig.paste(
        &pane,
        &format!("{} word-end", "x".repeat(width - 1)),
        "word-end",
    );
    rig.send(&pane, "\x01z \x1b[D");
    let frame = rig.wait(
        &pane,
        FRAME,
        "GUARD inserted prefix reached cursor row",
        |f| f.text.contains("lead   z"),
    );
    rig.holds(
        &frame,
        &["z".to_owned(), "x".repeat(width - 1), "word-end".to_owned()],
        0,
        1,
    );
}

#[test]
fn terminal_cursor_uses_drawn_ascii_and_unicode_cell_widths() {
    let rig = Rig::new("cwcell", 0);
    let pane = rig.open();
    rig.writing(&pane);
    let frame = rig.paste(&pane, "aé中", "aé中");
    rig.holds(&frame, &strings(&["aé中"]), 0, 4); // a=1, é=1, 中=2 on the terminal
    rig.send(&pane, "\x1b[DX");
    let edited = rig.wait(&pane, FRAME, "GUARD edit before wide glyph", |f| {
        f.text.contains("aéX中")
    });
    rig.holds(&edited, &strings(&["aéX中"]), 0, 3);
}

#[test]
fn conservative_unicode_wrap_and_actual_cursor_width_share_the_drawn_row() {
    let rig = Rig::new("cwutf", 0);
    let pane = rig.open();
    rig.resize(&pane, 100, 30);
    let blank = rig.writing(&pane);
    let width = 98 - rig.composer(&blank).0;
    let first = format!("{}é", "x".repeat(width - 2));
    let frame = rig.paste(&pane, &format!("{first}中Z"), "中Z");
    rig.holds(&frame, &[first, "中Z".to_owned()], 1, 3);
}

#[test]
fn an_exactly_full_final_row_has_a_new_empty_cursor_row() {
    let rig = Rig::new("cwedge", 0);
    let pane = rig.open();
    rig.resize(&pane, 100, 30);
    let blank = rig.writing(&pane);
    let width = 98 - rig.composer(&blank).0;
    // A full final row already has an empty cursor row in chat's view. The
    // old app draws only that empty row, so Esc provides the paste witness.
    rig.send(
        &pane,
        &format!("\x1b[200~{}\x1b[201~\x1b", "x".repeat(width)),
    );
    rig.wait(&pane, FRAME, "GUARD full draft kept", |f| {
        f.text.contains(&"x".repeat(width)) && f.text.contains("draft kept")
    });
    rig.send(&pane, "i");
    let frame = rig.wait(&pane, FRAME, "GUARD resumed writing", |f| writing(&f.text));
    rig.holds(&frame, &["x".repeat(width), String::new()], 1, 0);
}

#[test]
fn cursor_before_a_break_after_a_full_row_stays_on_its_last_physical_cell() {
    let rig = Rig::new("cwbreak", 0);
    let pane = rig.open();
    rig.resize(&pane, 100, 30);
    let blank = rig.writing(&pane);
    let width = 98 - rig.composer(&blank).0;
    let first = "x".repeat(width);
    rig.paste(&pane, &format!("{first}\nbreak-tail"), "break-tail");
    rig.send(&pane, &format!("\x01{}", "\x1b[C".repeat(width)));
    let frame = rig.wait(
        &pane,
        FRAME,
        "cursor moved to full row before newline",
        |f| f.text.contains(&first),
    );
    rig.holds(&frame, &[first, "break-tail".to_owned()], 0, width - 1);
}

#[test]
fn tail_window_names_eleven_cut_rows_inside_the_ten_row_cap() {
    let rig = Rig::new("cwtail", 0);
    let pane = rig.open();
    rig.writing(&pane);
    let frame = rig.paste(&pane, &lines(20), "L19");
    rig.holds(&frame, &tail(), 9, 3);
    assert!(!frame.text.contains("L10"));
}

#[test]
fn interior_cursor_window_names_both_cut_counts() {
    let rig = Rig::new("cwmid", 0);
    let pane = rig.open();
    rig.writing(&pane);
    rig.paste(&pane, &lines(20), "L19");
    rig.send(&pane, &format!("\x01{}", "\x1b[C".repeat(40)));
    let frame = rig.wait(&pane, FRAME, "GUARD cursor moved to L10", |f| {
        f.text.contains("L10") && !f.text.contains("L19")
    });
    let mut rows = vec!["… +6 lines above".to_owned()];
    rows.extend((6..14).map(|at| format!("L{at:02}")));
    rows.push("… +6 lines below".to_owned());
    rig.holds(&frame, &rows, 5, 0);
}

#[test]
fn home_cursor_window_keeps_nine_rows_and_the_below_marker() {
    let rig = Rig::new("cwhead", 0);
    let pane = rig.open();
    rig.writing(&pane);
    rig.paste(&pane, &lines(20), "L19");
    rig.send(&pane, "\x01");
    let frame = rig.wait(&pane, FRAME, "GUARD cursor reached first hard row", |f| {
        f.text.contains("L00")
    });
    let mut rows = (0..9).map(|at| format!("L{at:02}")).collect::<Vec<_>>();
    rows.push("… +11 lines below".to_owned());
    rig.holds(&frame, &rows, 0, 0);
}

#[test]
fn short_window_shrinks_composer_first_and_preserves_three_lane_rows() {
    let rig = Rig::new("cwshort", 40);
    let pane = rig.open();
    rig.resize(&pane, 100, 16);
    rig.writing(&pane);
    let frame = rig.paste(&pane, &lines(20), "L19");
    let rows = strings(&["… +16 lines above", "L16", "L17", "L18", "L19"]);
    rig.holds(&frame, &rows, 4, 3);
    assert_eq!(
        rig.composer(&frame).1,
        8,
        "three lane rows, marker and rule precede composer"
    );
    assert!(
        frame
            .text
            .lines()
            .take(6)
            .skip(3)
            .any(|row| row.contains("lane-39")),
        "newest turn stays in the three-row lane"
    );
}

#[test]
fn one_and_two_row_windows_omit_markers_but_show_the_cursor_window() {
    let rig = Rig::new("cwtiny", 0);
    let pane = rig.open();
    rig.resize(&pane, 100, 13);
    rig.writing(&pane);
    let frame = rig.paste(&pane, &lines(20), "L19");
    rig.holds(&frame, &strings(&["L18", "L19"]), 1, 3);
    assert!(!frame.text.contains("lines above") && !frame.text.contains("lines below"));
    let small = rig.resize(&pane, 100, 12);
    rig.holds(&small, &strings(&["L19"]), 0, 3);
    assert!(!small.text.contains("lines above") && !small.text.contains("lines below"));
}

#[test]
fn resize_recomputes_the_window_and_hides_cursor_below_minimum() {
    let rig = Rig::new("cwsize", 0);
    let pane = rig.open();
    rig.writing(&pane);
    let frame = rig.paste(&pane, &lines(20), "L19");
    rig.holds(&frame, &tail(), 9, 3);
    let small = rig.resize(&pane, 100, 16);
    rig.holds(
        &small,
        &strings(&["… +16 lines above", "L16", "L17", "L18", "L19"]),
        4,
        3,
    );
    assert!(!small.text.contains("L11"), "no stale grown rows");
    let tiny = rig.resize(&pane, 40, 7);
    assert_eq!(tiny.cursor.0, 0, "below MIN has no writing cursor");
    let large = rig.resize(&pane, 160, 45);
    rig.holds(&large, &tail(), 9, 3);
}

#[test]
fn wheel_over_grown_composer_moves_three_lane_rows_and_bounds_survive_shrink() {
    let rig = Rig::new("cwwheel", 40);
    let pane = rig.open();
    rig.wait(&pane, WAIT, "GUARD transcript loaded", |f| {
        f.text.contains("lane-39")
    });
    rig.writing(&pane);
    let before = rig.paste(&pane, &lines(20), "L19");
    rig.holds(&before, &tail(), 9, 3);
    let (column, top, _) = rig.composer(&before);
    let lane = |frame: &Frame| {
        frame
            .text
            .lines()
            .skip(3)
            .take(top - 5)
            .map(|row| {
                row.chars()
                    .skip(47)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };
    let original = lane(&before);
    rig.report(&pane, 64, column, top, 1);
    let after = rig.wait(&pane, FRAME, "one notch moved lane", |f| {
        f.text.contains("newer turns below")
    });
    rig.holds(&after, &tail(), 9, 3);
    let shifted = lane(&after);
    assert_eq!(
        &shifted[3..],
        &original[..original.len() - 3],
        "one composer notch = three rendered lane rows"
    );
    rig.report(&pane, 64, column, top, 200);
    let oldest = rig.wait(&pane, FRAME, "grown lane reaches oldest", |f| {
        f.text.contains("lane-00")
    });
    rig.report(&pane, 64, column, top, 1);
    rig.send(&pane, "!"); // processing witness for the boundary notch
    let edge = rig.wait(
        &pane,
        FRAME,
        "GUARD boundary notch processed before edit",
        |f| f.text.contains("L19!"),
    );
    assert_eq!(
        lane(&edge),
        lane(&oldest),
        "notch above oldest changes no lane row"
    );
    rig.send(&pane, "\x15");
    let compact = rig.wait(&pane, FRAME, "GUARD clear shrinks draft", |f| {
        !f.text.contains("L19") && writing(&f.text)
    });
    rig.holds(&compact, &strings(&[""]), 0, 0);
    let first = compact
        .text
        .lines()
        .nth(3)
        .expect("oldest lane row")
        .chars()
        .skip(47)
        .collect::<String>();
    assert!(
        !first.trim().is_empty(),
        "larger lane reclamps without blank rows above oldest"
    );
    assert!(
        compact
            .text
            .lines()
            .skip(3)
            .take(4) // coverage notice, gap, turn heading, then its body
            .any(|row| row.contains("lane-00")),
        "oldest turn begins the enlarged lane"
    );
}

#[test]
fn settings_hides_the_writing_cursor_then_restores_same_draft_cell() {
    let rig = Rig::new("cwgear", 0);
    let pane = rig.open();
    rig.writing(&pane);
    let before = rig.paste(&pane, "gear-head\ngear-tail", "gear-tail");
    rig.holds(&before, &strings(&["gear-head", "gear-tail"]), 1, 9);
    let (x, y) = cell(&before.text, "⚙");
    rig.report(&pane, 0, x, y, 1);
    let settings = rig.wait(
        &pane,
        FRAME,
        "GUARD Settings opened by gear while writing",
        |f| {
            f.text.contains("Settings")
                && f.text.contains("Quota")
                && f.text.contains("About")
                && !writing(&f.text)
        },
    );
    assert_eq!(settings.cursor.0, 0, "Settings hides cursor");
    rig.send(&pane, "\x1b");
    let resumed = rig.wait(&pane, FRAME, "GUARD Settings closed", |f| {
        writing(&f.text) && f.text.contains("gear-tail")
    });
    rig.holds(&resumed, &strings(&["gear-head", "gear-tail"]), 1, 9);
    assert_eq!(resumed.cursor, before.cursor);
}

#[test]
fn leaving_writing_compacts_kept_draft_and_discards_the_old_grown_hit_target() {
    let rig = Rig::new("cwkeep", 0);
    let pane = rig.open();
    rig.writing(&pane);
    let writing = rig.paste(&pane, "kept-head\nkept-mid\nkept-tail", "kept-tail");
    rig.holds(
        &writing,
        &strings(&["kept-head", "kept-mid", "kept-tail"]),
        2,
        9,
    );
    let (column, old_top, _) = rig.composer(&writing);
    rig.send(&pane, "\x1b");
    let kept = rig.wait(&pane, FRAME, "GUARD Esc keeps draft in browse", |f| {
        f.text.contains("draft kept")
    });
    assert_eq!(kept.cursor.0, 0);
    assert_eq!(
        cell(&kept.text, &rig.address()).1,
        41,
        "browse composer is one row"
    );
    rig.report(&pane, 0, column, old_top, 1);
    rig.send(&pane, "\t");
    let browsed = rig.wait(
        &pane,
        FRAME,
        "GUARD Tab after old composer row still browses",
        |f| f.text.contains("No seat facts"),
    );
    assert_eq!(browsed.cursor.0, 0);
    assert!(browsed.text.contains("draft kept"));
    rig.report(&pane, 0, column, 42, 1); // compact hint is still Compose
    let resumed = rig.wait(
        &pane,
        FRAME,
        "GUARD current compact target enters writing",
        |f| crate::app_mode::writing(&f.text),
    );
    rig.holds(
        &resumed,
        &strings(&["kept-head", "kept-mid", "kept-tail"]),
        2,
        9,
    );
}

#[test]
fn browse_busy_held_stopped_and_revoked_read_only_keep_the_cursor_hidden() {
    let rig = Rig::new("cwquiet", 0);
    let stopped = rig.root.join("sessions").join("zzstop");
    fs::create_dir(&stopped).expect("stopped sibling");
    fs::write(stopped.join("meta"), format!("session=zzstop\nmode=local\nsession_id=0199c0de-bbbb-4890-abcd-ef0123456789\nlayout=lead-pair\nseat.main=lead\ntmux_server_kind=socket\ntmux_server={}\n", rig.socket.display())).expect("stopped meta");
    let pane = rig.open();
    assert_eq!(rig.frame(&pane).cursor.0, 0, "browse hides cursor");
    let lease = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(rig.tool.dir.join(".console-writer.lock"))
        .expect("independent lease node");
    lease.try_lock().expect("kernel lease held by fixture");
    rig.send(&pane, "i");
    let held = rig.wait(&pane, FRAME, "GUARD busy app held", |f| {
        f.text.contains("not writing:")
    });
    assert_eq!(held.cursor.0, 0);
    assert_eq!(cell(&held.text, "not writing:").1, 41, "Held stays compact");
    rig.send(&pane, "\x1b");
    let browse = rig.wait(&pane, FRAME, "GUARD held exits to browse", |f| {
        f.text.contains("Enter writes")
    });
    let (x, y) = cell(&browse.text, "zzstop");
    rig.report(&pane, 0, x, y, 1);
    let foreign = rig.wait(&pane, FRAME, "B2 selected stopped target", |f| {
        f.text.contains("zzstop is stopped")
    });
    assert_eq!(foreign.cursor.0, 0);
    assert_eq!(cell(&foreign.text, "zzstop is stopped").1, 41);
    drop(lease);
    let (home_x, home_y) = foreign
        .text
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            let sidebar = line.chars().take(44).collect::<String>();
            sidebar
                .find(&rig.home)
                .map(|byte| (sidebar[..byte].chars().count(), row))
        })
        .expect("drawn home card");
    rig.report(&pane, 0, home_x, home_y, 1);
    rig.wait(&pane, FRAME, "GUARD selection returned home", |f| {
        f.text.contains("Enter writes")
    });
    rig.writing(&pane);
    rig.paste(&pane, "memory", "lead   memory");
    fs::remove_file(rig.tool.dir.join("meta")).expect("revoke home proof");
    let revoked = rig.wait(&pane, WAIT, "GUARD refresh revoked writing", |f| {
        f.text.contains("not writing:") && !writing(&f.text)
    });
    assert_eq!(revoked.cursor.0, 0);
    rig.send(&pane, "\x1b");
    let read_only = rig.wait(&pane, FRAME, "GUARD revoked app browses read-only", |f| {
        f.text.contains("read-only ·")
    });
    assert_eq!(read_only.cursor.0, 0);
    assert_eq!(cell(&read_only.text, "read-only ·").1, 41);
}

#[test]
fn no_home_selected_browse_hides_cursor_and_terminal_leave_restores_it() {
    let rig = Rig::new("cwnone", 0);
    let quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
    let command = format!(
        "env -u TMUX -u TMUX_PANE -u CLAUDE_CONFIG_DIR -u CODEX_HOME AE_HOME={} HOME={} CONFIG_FILE={} AE_TMUX_SERVER_KIND=socket AE_TMUX_SERVER={} {} app; printf C1-LEFT; exec sleep 30",
        quote(&rig.root.display().to_string()),
        quote(&rig.root.display().to_string()),
        quote(&rig.root.join("config").display().to_string()),
        quote(&rig.socket.display().to_string()),
        quote(env!("CARGO_BIN_EXE_ae"))
    );
    let pane = rig
        .tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &rig.home,
            &command,
        ])
        .trim()
        .to_owned();
    let outside = rig.wait(
        &pane,
        WAIT,
        "GUARD outside app drew selected session",
        |f| f.text.contains("Sessions 1") && f.text.contains("Overview"),
    );
    let address = format!("to {} (not home) › lead", rig.home);
    assert!(
        outside.text.contains(&address),
        "B2 selected address outside tmux"
    );
    assert!(
        outside.text.contains("Enter writes"),
        "B2 no home needed to write"
    );
    assert_eq!(outside.cursor.0, 0);
    assert_eq!(cell(&outside.text, &address).1, 41);
    rig.send(&pane, "qq");
    let left = rig.wait(&pane, FRAME, "GUARD terminal left app", |f| {
        f.text.contains("C1-LEFT")
    });
    assert_eq!(left.cursor.0, 1, "tty LEAVE restores shell cursor");
}
