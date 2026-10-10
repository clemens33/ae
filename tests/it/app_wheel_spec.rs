//! Frozen appuse A acceptance. Oracle: lead ruling A1-A3 and the drawn frame.
//! Real app, private tmux/socket/stores; no model-private API or substitute reducer.
//! List wheel = one whole session; Overview/Agents = three rendered text rows.
//! Baseline controls establish the fixture and pointer before every RED assertion.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use crate::app_mode::writing;
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const FRAME: Duration = Duration::from_secs(3);

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    home: String,
    names: Vec<String>,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
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
        let turns = (0..12)
            .map(|at| {
                super::board::user(
                    &format!("2026-10-04T10:00:{at:02}Z"),
                    &format!("wheel-chat-{at:02} {}", "fixture words ".repeat(35)),
                )
            })
            .collect::<Vec<_>>();
        super::board::plant_transcript(&store, "work", tid, &turns);
        fs::write(tool.dir.join("meta"), format!(
            "session={home}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\ngoal=wheel-goal\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-rig\nharness_session.main={tid}\nconfig_home.main={}\nseat.worker.0=colead\nagent_bin.worker.0=claude\nlaunch_id.worker.0=tok-colead\nconfig_home.worker.0={}\n",
            socket.display(), store.display(), store.display(),
        )).expect("lead pair meta");
        fs::write(tool.dir.join("events.jsonl"), "").expect("empty fixture journal");
        let mut names = vec![home.clone()];
        for at in 0..12 {
            let name = format!("ws{at:02}");
            let dir = root.join("sessions").join(&name);
            fs::create_dir(&dir).expect("stopped sibling");
            fs::write(dir.join("meta"), format!(
                "session={name}\nmode=local\nsession_id=0199c0de-bbbb-4890-abcd-ef012345{at:04x}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok-stop\n",
                socket.display(),
            )).expect("sibling meta");
            names.push(name);
        }
        fs::write(root.join("config"), "[workspace]\nchat = app\n").expect("app config");
        let rig = Self {
            tool,
            root,
            socket,
            home,
            names,
        };
        for (option, value) in [
            ("@ae_look", "off"),
            ("@ae_session_uuid", UUID),
            ("@ae_motion", "off"),
            ("@ae_attn_rank", "0"),
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
        rig.topics(30);
        rig.agents();
        rig
    }

    fn topics(&self, count: usize) {
        let mut rows = String::new();
        for at in 0..count {
            writeln!(
                rows,
                "2026-10-07T16:00:{at:02}Z\tcl:lead\ttopic{at:02}\tbody{at:02}"
            )
            .expect("memo row");
        }
        fs::write(self.tool.dir.join("memo.tsv"), rows).expect("fixture memos");
    }

    fn agents(&self) {
        let mut raw = format!("v1;{};300", ae::time::Timestamp::now().epoch());
        for at in 0..20 {
            write!(raw, ";seat{at:02}:sonnet55x:working:").expect("seat fact");
        }
        assert_eq!(
            ae::tmux::parse_picker_agents(&raw, ae::time::Timestamp::now().epoch())
                .expect("valid roster")
                .len(),
            20
        );
        self.tmux(&["set-option", "-t", &self.home, "@ae_agents", &raw]);
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
        self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &self.home,
            "-F",
            "#{pane_id}|#{@ae_console}",
        ])
        .lines()
        .find_map(|line| {
            let (pane, stamp) = line.split_once('|')?;
            (stamp == UUID).then(|| pane.to_owned())
        })
        .expect("stamped app pane")
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn wait(&self, pane: &str, limit: Duration, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + limit;
        let mut previous = None;
        loop {
            let screen = self.screen(pane);
            if met(&screen) && previous.as_deref() == Some(screen.as_str()) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; screen:\n{screen}");
            previous = Some(screen);
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn ready(&self, pane: &str) -> String {
        let first = self.wait(pane, WAIT, "GUARD undragged app frame", |s| {
            s.contains("Sessions 13") && s.contains("Enter writes") && s.contains("Overview")
        });
        let edge = first
            .lines()
            .position(|line| line.starts_with("──"))
            .expect("drawn list border");
        assert_eq!(edge, 30, "GUARD new undragged list border");
        assert_eq!(self.cards(&first).len(), 11, "GUARD new undragged cards");
        // This suite judges wheel behavior at its original viewport. The
        // appside spec independently pins the changed undragged default.
        self.literal(
            pane,
            &format!("\x1b[<0;2;{}M\x1b[<32;2;25M\x1b[<0;2;25m", edge + 1),
        );
        let pinned = self.wait(
            pane,
            WAIT,
            "GUARD divider drag restored wheel fixture",
            |s| {
                s.lines().position(|line| line.starts_with("──")) == Some(24)
                    && self.cards(s).len() == 8
            },
        );
        assert_eq!(
            pinned.lines().position(|line| line.starts_with("──")),
            Some(24)
        );
        assert_eq!(self.cards(&pinned).len(), 8);
        self.wait(pane, WAIT, "GUARD home, overflow and memos ready", |s| {
            s.contains("Sessions 13") && s.contains("Enter writes") && s.contains("topic29")
        })
    }

    fn literal(&self, pane: &str, bytes: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", bytes]);
    }

    fn report(&self, pane: &str, code: u8, cell: (usize, usize), count: usize) {
        let one = format!("\x1b[<{code};{};{}M", cell.0 + 1, cell.1 + 1);
        self.literal(pane, &one.repeat(count));
    }

    fn cards(&self, screen: &str) -> Vec<String> {
        let width = drawn_rule(screen).unwrap_or_default();
        screen
            .lines()
            .filter_map(|line| {
                let side = line.chars().take(width).collect::<String>();
                self.names
                    .iter()
                    .find(|name| side.split_whitespace().any(|word| word == name.as_str()))
                    .cloned()
            })
            .collect()
    }

    fn home_holds(&self, screen: &str, writing: bool) {
        let header = screen
            .lines()
            .nth(1)
            .expect("chat header")
            .chars()
            .skip(rule(screen) + 1)
            .collect::<String>();
        assert!(
            header.contains(&self.home),
            "selection stayed home: {header}"
        );
        assert!(
            if writing {
                crate::app_mode::writing(screen)
            } else {
                screen.contains("Enter writes")
            },
            "input mode preserved:\n{screen}"
        );
        assert!(
            !screen.contains("newer turns below"),
            "sidebar wheel did not scroll chat"
        );
    }
}

fn drawn_rule(screen: &str) -> Option<usize> {
    screen
        .lines()
        .next()
        .and_then(|row| row.chars().position(|c| c == '│'))
}

fn rule(screen: &str) -> usize {
    drawn_rule(screen).expect("drawn vertical rule")
}

fn cell(screen: &str, needle: &str) -> (usize, usize) {
    screen
        .lines()
        .enumerate()
        .find_map(|(y, line)| {
            line.chars()
                .collect::<Vec<_>>()
                .windows(needle.chars().count())
                .position(|part| part.iter().collect::<String>() == needle)
                .map(|x| (x, y))
        })
        .unwrap_or_else(|| panic!("GUARD drawn {needle}:\n{screen}"))
}

fn sidebar_cell(screen: &str, needle: &str) -> (usize, usize) {
    let width = rule(screen);
    screen
        .lines()
        .enumerate()
        .find_map(|(y, line)| {
            line.chars()
                .take(width)
                .collect::<Vec<_>>()
                .windows(needle.chars().count())
                .position(|part| part.iter().collect::<String>() == needle)
                .map(|x| (x, y))
        })
        .unwrap_or_else(|| panic!("GUARD sidebar {needle}:\n{screen}"))
}

fn body_rows(screen: &str) -> Vec<String> {
    // A capture can fall between the resize clear and the tab's repaint.
    // Wait predicates must see no body yet, rather than panic on its label.
    let Some(top) = screen.lines().position(|row| row.contains("Overview")) else {
        return Vec::new();
    };
    let Some(width) = drawn_rule(screen) else {
        return Vec::new();
    };
    let top = top + 2;
    let end = screen.lines().count().saturating_sub(2);
    screen
        .lines()
        .skip(top)
        .take(end.saturating_sub(top))
        .map(|line| {
            let row = line.chars().take(width).collect::<String>();
            // The focused-seat marker takes a blank cell; the pins read the row without it.
            if row.trim_start().starts_with('▸') {
                return row.replacen('▸', " ", 1).trim_end().to_owned();
            }
            row.trim_end().to_owned()
        })
        .collect()
}

fn body(screen: &str) -> Vec<String> {
    body_rows(screen)
        .into_iter()
        .filter(|line| !line.contains("more rows"))
        .map(|line| {
            // Compare content identity, independent of a refresh crossing an age minute.
            let text = line.trim();
            if text.starts_with("topic") && !text.contains("body") {
                text.split_whitespace()
                    .next()
                    .expect("topic name")
                    .to_owned()
            } else if text.starts_with("wheel-goal")
                || (text.starts_with("topic") && text.contains("body"))
            {
                text.split(" · ")
                    .next()
                    .expect("content before age")
                    .to_owned()
            } else {
                line
            }
        })
        .collect()
}

fn more(screen: &str) -> String {
    screen
        .lines()
        .map(|line| line.chars().take(rule(screen)).collect::<String>())
        .find(|line| line.contains(" more"))
        .expect("GUARD list overflow marker")
}

#[test]
fn guard_complete_drawn_rows_prove_overview_agents_and_resize_oracles() {
    let rig = Rig::new("wloracle");
    let pane = rig.open();
    let first = rig.ready(&pane);
    assert_eq!(body_rows(&first).len(), 18, "GUARD original body rectangle");
    rig.tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "100"]);
    let complete = rig.wait(&pane, FRAME, "GUARD all Overview content fits", |s| {
        s.lines().count() == 100 && s.contains("body00") && !s.contains("more rows")
    });
    let mut expected = vec![
        "mode: local",
        "dir: unrecorded",
        "Goal",
        "wheel-goal",
        "Waiting on you 0",
        "Nothing waits on you.",
        "Decided · latest memo",
        "No decision memo.",
        "Topics",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    for at in (0..30).rev() {
        expected.push(format!("topic{at:02}"));
        expected.push(format!("body{at:02}"));
    }
    let nonempty = |s: &str| {
        body(s)
            .iter()
            .map(|row| row.trim().to_owned())
            .filter(|row| !row.is_empty())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        nonempty(&complete),
        expected,
        "GUARD 69 exact compacted rows"
    );
    let visible = if first.contains("more rows") { 17 } else { 18 };
    assert_eq!(
        nonempty(&first),
        expected[..visible],
        "GUARD original rows match complete content"
    );
    rig.report(&pane, 65, (2, cell(&complete, "Overview").1 + 2), 40);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        nonempty(&rig.screen(&pane)),
        expected,
        "fitting Overview wheel is a no-op"
    );
    rig.literal(&pane, "\t");
    let agents = rig.wait(&pane, FRAME, "GUARD all Agents content fits", |s| {
        s.contains("seat19")
    });
    let seats = nonempty(&agents);
    assert_eq!(seats.len(), 40, "GUARD twenty two-row seats");
    for at in 0..20 {
        let name = format!("seat{at:02}");
        assert_eq!(
            seats[2 * at]
                .split_whitespace()
                .find(|word| word.starts_with("seat")),
            Some(name.as_str()),
            "GUARD seat row order"
        );
        assert_eq!(seats[2 * at + 1], "sonnet55x", "GUARD profile facts row");
    }
    assert!(seats[3].contains("sonnet55x"));
    rig.report(&pane, 65, (2, cell(&agents, "Overview").1 + 2), 40);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        nonempty(&rig.screen(&pane)),
        seats,
        "fitting Agents wheel is a no-op"
    );
    rig.literal(&pane, "\t");
    rig.tmux(&["resize-window", "-t", &pane, "-x", "100", "-y", "80"]);
    let narrow = rig.wait(&pane, FRAME, "GUARD complete narrow topics fit", |s| {
        s.lines().count() == 80
            && drawn_rule(s) == Some(34)
            && s.contains("body00")
            && !s.contains("more rows")
    });
    let mut expected_narrow = expected[..9].to_vec();
    expected_narrow.extend((0..30).rev().map(|at| format!("topic{at:02} body{at:02}")));
    assert_eq!(
        nonempty(&narrow),
        expected_narrow,
        "GUARD narrow layout has 39 rows"
    );
}

#[test]
fn click_pins_the_drawn_window_even_when_keys_were_following_selection() {
    let rig = Rig::new("wlpin");
    let pane = rig.open();
    rig.ready(&pane);
    rig.literal(&pane, &"j".repeat(12));
    let bottom = rig.wait(
        &pane,
        FRAME,
        "GUARD key selection exposes final card",
        |s| {
            rig.cards(s) == rig.names[5..13]
                && s.lines().nth(1).is_some_and(|line| line.contains("ws11"))
        },
    );
    let point = sidebar_cell(&bottom, "ws04");
    rig.report(&pane, 0, (point.0, point.1 + 1), 1);
    let clicked = rig.wait(&pane, FRAME, "GUARD click selects first drawn card", |s| {
        s.lines().nth(1).is_some_and(|line| line.contains("ws04"))
    });
    assert_eq!(
        rig.cards(&clicked),
        rig.cards(&bottom),
        "A-CLICK-PIN following window stays under pointer"
    );
    assert_eq!(sidebar_cell(&clicked, "ws04"), point);
    rig.literal(&pane, "\x1b");
    rig.wait(
        &pane,
        FRAME,
        "A-CLICK-PIN-KEY selection key restores anchored rule",
        |s| rig.cards(s) == rig.names[..8] && s.contains("Enter writes"),
    );
}

#[test]
fn body_cut_marker_counts_hidden_text_rows_and_owns_wheel_but_no_click() {
    let rig = Rig::new("wlmarker");
    let pane = rig.open();
    let first = rig.ready(&pane);
    // Contract fixture: two facts + seven section rows + sixty topic rows = 69.
    // The drawn 18-row body gives one row to the marker, leaving 17 content rows.
    let marked = rig.wait(
        &pane,
        FRAME,
        "A-BODY-MARKER top hides 52 of 69 compacted rows",
        |s| {
            body_rows(s)
                .last()
                .is_some_and(|row| row.trim() == "↓ 52 more rows")
        },
    );
    assert_eq!(body(&marked).len(), 17);
    let point = sidebar_cell(&marked, "more rows");
    rig.report(&pane, 0, point, 1);
    std::thread::sleep(Duration::from_millis(150));
    let clicked = rig.screen(&pane);
    assert_eq!(
        body_rows(&clicked),
        body_rows(&marked),
        "marker has no click action"
    );
    rig.home_holds(&clicked, false);
    rig.report(&pane, 65, point, 1);
    let down = rig.wait(
        &pane,
        FRAME,
        "A-BODY-MARKER-WHEEL three rows through marker cells",
        |s| {
            body_rows(s)
                .last()
                .is_some_and(|row| row.trim() == "↑ 3 · ↓ 49 more rows")
        },
    );
    assert_eq!(body(&down).first(), body(&first).get(3));
    rig.report(&pane, 65, point, 40);
    rig.wait(
        &pane,
        FRAME,
        "A-BODY-MARKER-BOTTOM bound reserves marker row",
        |s| {
            body_rows(s)
                .last()
                .is_some_and(|row| row.trim() == "↑ 52 more rows")
                && body(s).last().is_some_and(|row| row.contains("body00"))
        },
    );
    rig.topics(0);
    rig.wait(
        &pane,
        WAIT,
        "A-BODY-MARKER-FITS no cut marker when body fits",
        |s| {
            s.contains("No topics.")
                && !s.contains("more rows")
                && body(s)
                    .first()
                    .is_some_and(|row| row.contains("mode: local"))
        },
    );
}

#[test]
fn list_wheel_moves_one_whole_card_and_clamps_without_remembering_overshoot() {
    let rig = Rig::new("wlunit");
    let pane = rig.open();
    let first = rig.ready(&pane);
    let cards = rig.cards(&first);
    assert_eq!(cards.len(), 8, "GUARD fixture has eight cards");
    assert_eq!(cards[0], rig.home, "GUARD home is first");
    let point = sidebar_cell(&first, &rig.home);
    rig.report(&pane, 65, point, 1);
    let down = rig.wait(&pane, FRAME, "A-LIST-UNIT exactly one session down", |s| {
        rig.cards(s) == rig.names[1..9]
    });
    assert!(more(&down).contains("↑ 1") && more(&down).contains("↓ 4"));
    rig.home_holds(&down, false);
    rig.report(&pane, 65, point, 40);
    let bottom = rig.wait(&pane, FRAME, "A-LIST-BOTTOM clamped to last eight", |s| {
        rig.cards(s) == rig.names[5..13]
    });
    assert!(more(&bottom).contains("↑ 5") && !more(&bottom).contains('↓'));
    rig.report(&pane, 64, point, 1);
    rig.wait(
        &pane,
        FRAME,
        "A-LIST-REVERSE overshoot was discarded",
        |s| rig.cards(s) == rig.names[4..12],
    );
    rig.report(&pane, 64, point, 40);
    let top = rig.wait(&pane, FRAME, "A-LIST-TOP clamped to first eight", |s| {
        rig.cards(s) == cards
    });
    assert!(!more(&top).contains('↑') && more(&top).contains("↓ 5"));
    rig.report(&pane, 65, point, 1);
    rig.wait(&pane, FRAME, "A-LIST-TOP-REVERSE one notch from top", |s| {
        rig.cards(s) == rig.names[1..9]
    });
}

#[test]
fn clicked_revealed_card_keeps_window_and_same_selection_key_reveals_it() {
    let rig = Rig::new("wlclick");
    let pane = rig.open();
    let first = rig.ready(&pane);
    rig.report(&pane, 65, sidebar_cell(&first, &rig.home), 3);
    let down = rig.wait(
        &pane,
        FRAME,
        "A-CLICK-PRECOND wheel exposes later cards",
        |s| rig.cards(s) == rig.names[3..11],
    );
    let target = "ws09";
    let point = sidebar_cell(&down, target);
    rig.report(&pane, 0, (point.0, point.1 + 1), 1);
    let selected = rig.wait(
        &pane,
        FRAME,
        "A-CLICK-SECOND-LINE selects drawn card",
        |s| {
            s.lines().nth(1).is_some_and(|line| line.contains(target))
                && s.contains("ws09 is stopped")
        },
    );
    assert_eq!(
        sidebar_cell(&selected, target),
        point,
        "click does not jump wheeled list"
    );
    assert_eq!(rig.cards(&selected), rig.cards(&down));
    rig.literal(&pane, "\x1b");
    let home = rig.wait(&pane, FRAME, "A-KEY-ESC resets list to home window", |s| {
        rig.cards(s) == rig.names[..8] && s.contains("Enter writes")
    });
    rig.report(&pane, 65, sidebar_cell(&home, &rig.home), 1);
    rig.wait(&pane, FRAME, "A-KEY-SAME-PRECOND home scrolled out", |s| {
        rig.cards(s) == rig.names[1..9]
    });
    rig.literal(&pane, "1");
    let back = rig.wait(
        &pane,
        FRAME,
        "A-KEY-SAME digit reveals already-selected home",
        |s| rig.cards(s) == rig.names[..8],
    );
    rig.home_holds(&back, false);
}

#[test]
fn overview_wheel_moves_three_drawn_rows_and_tabs_and_selection_reset_body() {
    let rig = Rig::new("wlover");
    fs::write(
        rig.root.join("sessions/ws00/memo.tsv"),
        fs::read(rig.tool.dir.join("memo.tsv")).expect("fixture topic bytes"),
    )
    .expect("foreign session overflow");
    let pane = rig.open();
    let first = rig.ready(&pane);
    let rows = body(&first);
    assert!(
        rows[0].contains("mode: local") && rows.len() > 6,
        "GUARD overflow body drawn"
    );
    let point = (2, cell(&first, "Overview").1 + 2);
    rig.report(&pane, 65, point, 1);
    let down = rig.wait(&pane, FRAME, "A-OVERVIEW-UNIT three text rows down", |s| {
        body(s).first() == rows.get(3)
    });
    assert_eq!(rig.cards(&down), rig.cards(&first));
    rig.home_holds(&down, false);
    rig.report(&pane, 64, point, 1);
    rig.wait(
        &pane,
        FRAME,
        "A-OVERVIEW-UP returns exactly to first row",
        |s| body(s) == rows,
    );
    rig.report(&pane, 65, point, 40);
    let bottom = rig.wait(
        &pane,
        FRAME,
        "A-OVERVIEW-BOTTOM last topic reachable",
        |s| body(s).last().is_some_and(|row| row.contains("body00")),
    );
    rig.report(&pane, 65, point, 40);
    rig.report(&pane, 64, point, 1);
    let reversed = rig.wait(
        &pane,
        FRAME,
        "A-OVERVIEW-REVERSE starts at clamped bottom",
        |s| body(s).get(3) == body(&bottom).first(),
    );
    assert!(!body(&reversed)[0].contains("mode: local"));
    rig.literal(&pane, "\t\t");
    let reset = rig.wait(&pane, FRAME, "A-OVERVIEW-TAB reset", |s| body(s) == rows);
    rig.report(&pane, 65, point, 1);
    rig.wait(
        &pane,
        FRAME,
        "A-OVERVIEW-SELECT-PRECOND scrolled again",
        |s| body(s).first() == rows.get(3),
    );
    rig.literal(&pane, "2");
    rig.wait(
        &pane,
        FRAME,
        "A-OVERVIEW-KEY-SELECT reset overflowing foreign body",
        |s| {
            s.lines().nth(1).is_some_and(|line| line.contains("ws00"))
                && body(s)
                    .first()
                    .is_some_and(|row| row.contains("mode: local"))
                && s.contains("topic29")
        },
    );
    rig.literal(&pane, "\x1b");
    rig.wait(&pane, FRAME, "A-OVERVIEW-ESC reset home body", |s| {
        body(s) == body(&reset)
    });
    rig.report(&pane, 65, point, 1);
    let wheeled = rig.wait(
        &pane,
        FRAME,
        "A-OVERVIEW-CLICK-PRECOND scrolled again",
        |s| body(s).first() == rows.get(3),
    );
    rig.report(&pane, 0, sidebar_cell(&wheeled, "ws00"), 1);
    rig.wait(
        &pane,
        FRAME,
        "A-OVERVIEW-CLICK-SELECT reset overflowing foreign body",
        |s| {
            s.lines().nth(1).is_some_and(|line| line.contains("ws00"))
                && body(s)
                    .first()
                    .is_some_and(|row| row.contains("mode: local"))
                && s.contains("topic29")
        },
    );
    rig.literal(&pane, "\x1b");
    let selected = rig.wait(&pane, FRAME, "A-OVERVIEW-SELECT home reset", |s| {
        body(s) == body(&reset)
    });
    rig.home_holds(&selected, false);
}

#[test]
fn agents_wheel_moves_three_text_rows_including_half_a_seat_and_clamps() {
    let rig = Rig::new("wlagent");
    let pane = rig.open();
    rig.ready(&pane);
    rig.literal(&pane, "\t");
    let first = rig.wait(
        &pane,
        FRAME,
        "GUARD published twenty-seat Agents body",
        |s| body(s).first().is_some_and(|row| row.contains("seat00")) && s.contains("Agents 20"),
    );
    let rows = body(&first);
    assert!(
        rows[3].contains("sonnet55x") && !rows[3].contains("seat01"),
        "GUARD fourth row is seat facts"
    );
    let point = (2, cell(&first, "Overview").1 + 2);
    rig.report(&pane, 65, point, 1);
    let down = rig.wait(
        &pane,
        FRAME,
        "A-AGENTS-UNIT three text rows, not three seats",
        |s| body(s).first() == rows.get(3),
    );
    rig.home_holds(&down, false);
    assert_eq!(
        body_rows(&down).last().expect("marker").trim(),
        "↑ 3 · ↓ 20 more rows",
        "Agents counts text rows"
    );
    rig.report(&pane, 65, point, 40);
    let bottom = rig.wait(
        &pane,
        FRAME,
        "A-AGENTS-BOTTOM final seat facts reachable",
        |s| {
            let rows = body(s);
            rows.len() > 1
                && rows[rows.len() - 2].contains("seat19")
                && rows[rows.len() - 1].contains("sonnet55x")
        },
    );
    assert_eq!(
        body_rows(&bottom).last().expect("marker").trim(),
        "↑ 23 more rows",
        "Agents bound reserves marker row"
    );
    rig.report(&pane, 65, point, 40);
    rig.report(&pane, 64, point, 1);
    rig.wait(&pane, FRAME, "A-AGENTS-REVERSE overshoot discarded", |s| {
        body(s).get(3) == body(&bottom).first()
    });
    rig.report(&pane, 64, point, 40);
    rig.wait(&pane, FRAME, "A-AGENTS-TOP clamped to first seat", |s| {
        body(s) == rows
    });
}

#[test]
fn list_and_body_wheel_while_writing_keep_exact_draft_mode_selection_and_journal() {
    let rig = Rig::new("wlwrite");
    let pane = rig.open();
    let first = rig.ready(&pane);
    let before = fs::read(rig.tool.dir.join("events.jsonl")).expect("before journal");
    rig.literal(&pane, "i");
    rig.wait(&pane, FRAME, "GUARD write mode before draft bytes", |s| {
        writing(s)
    });
    rig.literal(&pane, "WHEEL-draft-123");
    rig.wait(&pane, FRAME, "GUARD composing draft", |s| {
        s.contains("WHEEL-draft-123") && writing(s)
    });
    rig.report(&pane, 65, sidebar_cell(&first, &rig.home), 1);
    let list = rig.wait(
        &pane,
        FRAME,
        "A-WRITE-LIST scrolls without ending compose",
        |s| rig.cards(s) == rig.names[1..9],
    );
    rig.home_holds(&list, true);
    assert!(list.contains("WHEEL-draft-123"));
    let rows = body(&list);
    rig.report(&pane, 65, (2, cell(&list, "Overview").1 + 2), 1);
    let down = rig.wait(&pane, FRAME, "A-WRITE-BODY scrolls without editing", |s| {
        body(s).first() == rows.get(3)
    });
    rig.home_holds(&down, true);
    assert!(down.contains("WHEEL-draft-123"));
    rig.literal(&pane, "\x1b");
    let kept = rig.wait(&pane, FRAME, "draft kept after browse", |s| {
        s.contains("draft kept") && s.contains("WHEEL-draft-123")
    });
    assert_eq!(rig.cards(&kept), rig.cards(&down));
    assert_eq!(
        fs::read(rig.tool.dir.join("events.jsonl")).expect("after journal"),
        before,
        "wheel writes no event"
    );
}

#[test]
fn refresh_clamps_shrunk_list_and_body_before_drawing() {
    let rig = Rig::new("wlrefresh");
    let pane = rig.open();
    let first = rig.ready(&pane);
    rig.report(&pane, 65, sidebar_cell(&first, &rig.home), 40);
    let down = rig.wait(&pane, FRAME, "A-REFRESH-PRECOND list at bottom", |s| {
        rig.cards(s) == rig.names[5..13]
    });
    rig.report(&pane, 65, (2, cell(&down, "Overview").1 + 2), 40);
    rig.wait(&pane, FRAME, "A-REFRESH-BODY-PRECOND final topic", |s| {
        body(s).last().is_some_and(|row| row.contains("body00"))
    });
    for name in &rig.names[3..] {
        fs::remove_dir_all(rig.root.join("sessions").join(name)).expect("shrink fixture fleet");
    }
    rig.topics(0);
    let shrunk = rig.wait(
        &pane,
        WAIT,
        "A-REFRESH-CLAMP current content visible",
        |s| {
            s.contains("Sessions 3")
                && rig.cards(s) == rig.names[..3]
                && body(s)
                    .first()
                    .is_some_and(|row| row.contains("mode: local"))
                && s.contains("No topics.")
        },
    );
    rig.home_holds(&shrunk, false);
}

#[test]
fn resize_growth_clamps_body_offset_and_hidden_sidebar_has_no_live_wheel_target() {
    let rig = Rig::new("wlresize");
    let pane = rig.open();
    let first = rig.ready(&pane);
    rig.report(&pane, 65, (2, cell(&first, "Overview").1 + 2), 40);
    rig.wait(&pane, FRAME, "A-RESIZE-PRECOND body at bottom", |s| {
        body(s).last().is_some_and(|row| row.contains("body00"))
    });
    rig.tmux(&["resize-window", "-t", &pane, "-x", "100", "-y", "28"]);
    let narrow = rig.wait(&pane, FRAME, "A-RESIZE-NARROW body drawn", |s| {
        s.lines().count() == 28 && s.contains("topic") && rig.cards(s).len() == 7
    });
    rig.report(&pane, 65, (2, cell(&narrow, "Overview").1 + 2), 40);
    rig.wait(&pane, FRAME, "A-RESIZE-NARROW-BOTTOM final topic", |s| {
        body(s).last().is_some_and(|row| row.contains("body00"))
    });
    rig.tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "100"]);
    rig.wait(
        &pane,
        FRAME,
        "A-RESIZE-GROW all body fits and top visible",
        |s| {
            s.lines().count() == 100
                && body(s)
                    .first()
                    .is_some_and(|row| row.contains("mode: local"))
                && s.contains("body00")
        },
    );
    rig.tmux(&["resize-window", "-t", &pane, "-x", "80", "-y", "24"]);
    let hidden = rig.wait(&pane, FRAME, "GUARD sidebar hidden", |s| {
        s.contains("sidebar needs") && !s.contains("Sessions 13")
    });
    let chat = cell(&hidden, "to ");
    rig.report(&pane, 64, (2, chat.1), 1);
    rig.wait(
        &pane,
        FRAME,
        "A-HIDDEN-SIDEBAR only drawn chat owns old sidebar cells",
        |s| s.contains("newer turns below"),
    );
}

#[test]
fn wheel_ends_drag_then_list_scrolls_and_later_motion_cannot_move_border() {
    let rig = Rig::new("wldrag");
    let pane = rig.open();
    let first = rig.ready(&pane);
    let border = rule(&first);
    rig.report(&pane, 0, (border, 1), 1);
    rig.report(&pane, 32, (border + 6, 1), 1);
    let dragged = rig.wait(&pane, FRAME, "GUARD border drag moved", |s| {
        drawn_rule(s) == Some(border + 6)
    });
    rig.report(&pane, 65, sidebar_cell(&dragged, &rig.home), 1);
    rig.wait(
        &pane,
        FRAME,
        "A-DRAG-LIST notch ends drag and scrolls",
        |s| rig.cards(s) == rig.names[1..9],
    );
    rig.report(&pane, 32, (border + 12, 1), 1);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        rule(&rig.screen(&pane)),
        border + 6,
        "old drag ended at the notch"
    );
}

#[test]
fn guard_header_tab_rule_keys_and_settings_never_scroll_underlying_sidebar() {
    let rig = Rig::new("wlareas");
    let pane = rig.open();
    let first = rig.ready(&pane);
    let rows = body(&first);
    let cards = rig.cards(&first);
    let tab = cell(&first, "Overview");
    for point in [
        (2, 1),
        (2, 3),
        tab,
        (2, tab.1 + 1),
        (rule(&first), 5),
        (2, first.lines().count() - 1),
    ] {
        rig.report(&pane, 65, point, 1);
    }
    std::thread::sleep(Duration::from_millis(150));
    let same = rig.screen(&pane);
    assert_eq!(rig.cards(&same), cards);
    assert_eq!(body(&same), rows);
    rig.home_holds(&same, false);
    rig.literal(&pane, "s");
    rig.wait(&pane, FRAME, "GUARD Settings owns screen", |s| {
        s.contains("Settings") && s.contains("Quota") && s.contains("About")
    });
    rig.report(&pane, 65, (2, 6), 20);
    rig.literal(&pane, "s");
    let closed = rig.wait(&pane, FRAME, "GUARD closed Settings", |s| {
        s.contains("Sessions 13") && s.contains("Enter writes")
    });
    assert_eq!(rig.cards(&closed), cards, "covered list never moved");
    assert_eq!(body(&closed), rows, "covered body never moved");
}
