// Frozen bgmark acceptance (.local/bgmark/freeze.tsv), installed byte-identically.
include!("../../../../tests/spec/bgmark_draw.rs");
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::{Position, Rect};
use ratatui_core::style::Modifier;

use super::{Composer, LEFT, Paint, Screen, aged, dragged_to, draw, instructions_body, keys_body};
use crate::app::fleet::{Counts, Facts, Fleet, Line2, Row};
use crate::app::model::{Drag, Edge, Model};
use crate::app::overview::{Open, Overview};
use crate::console::input::View;
use crate::console::input::{Mouse, MouseKind};
use crate::console::lane::{Item, Kind, Lane};
use crate::digest::{SessionEntry, Status};
use crate::theme::{Look, Mark};
use crate::time::Timestamp;
use crate::tmux::PickerAgent;

fn row(name: &str, index: usize, needy: bool) -> Row {
    Row {
        name: name.to_owned(),
        index,
        mark: if needy { Mark::NeedsYou } else { Mark::Working },
        needy,
        unanswered: None,
        counts: Counts::Marks(vec![(Mark::Working, false, 3), (Mark::Done, true, 12)]),
        line2: Line2::Question {
            text: "a question long enough to be clipped at every width it meets".to_owned(),
            age_secs: Some(240),
        },
        home: index == 1,
    }
}

/// The sweep's populated asks and lane: an open ask, a recorded dir, a lane
/// that scrolls, a coverage line.
fn sweep_body() -> (Overview, Lane) {
    let overview = Overview {
        open: vec![Open {
            seat: "colead".to_owned(),
            text: "an open question long enough to clip".to_owned(),
            age_secs: Some(5),
        }],
        dir: Some("~/a/launch/directory/long/enough/to/clip".to_owned()),
        base: crate::console::link::Base::default(),
        unanswered: None,
        unanswered_oldest_secs: None,
    };
    let item = Item {
        micros: 1_759_500_000_000_000,
        kind: Kind::Said {
            who: "lead".to_owned(),
        },
        body: "a turn long enough to wrap over several rows of the chat column ".repeat(6),
        record: None,
    };
    let lane = Lane {
        items: vec![item; 9],
        coverage: vec!["colead — transcript unreadable".to_owned()],
    };
    (overview, lane)
}

/// The sweep's Agents body: nine seats with long names.
fn sweep_seats() -> Facts {
    let agent = PickerAgent {
        name: "a-seat-name-long-enough-to-clip".to_owned(),
        profile: "sonnet55x".to_owned(),
        state: "waiting-agent".to_owned(),
        pane: "%7".to_owned(),
        client: String::new(),
        model: "Sonnet 5.5".to_owned(),
        effort: "xhigh".to_owned(),
        drift: false,
        task: false,
    };
    Facts::Seats {
        id: "$1".to_owned(),
        agents: vec![agent; 9],
    }
}

/// The instructions rows neutralise controls before they wrap, keep every
/// word of a rule and its owner, fit the panel at the narrowest widths,
/// name the winning source or the files that held none, and give each
/// seat one line naming what it receives or its gap.
#[test]
fn instructions_rows_are_neutralised_wrapped_and_name_their_gaps() {
    use crate::app::settings::{
        ConfigSource, CustomInstructions, InstructionsView, Protocol, SeatLine,
    };
    use crate::render::{Audience, GenericRule};
    let hostile = format!("\u{1b}[31mred\u{7} {}\tend\nnext line", "word ".repeat(40));
    let ready = |custom, receives| {
        let seat = SeatLine {
            name: "lead".to_owned(),
            slot: "main".to_owned(),
            role: "lead",
            receives,
        };
        InstructionsView::Ready(Protocol {
            session: "api".to_owned(),
            files: vec!["/g".to_owned()],
            custom,
            spawn: None,
            rules: vec![GenericRule {
                audience: Audience::Workers,
                owner: Some(hostile.clone()),
                text: hostile.clone(),
            }],
            seats: vec![seat],
        })
    };
    let custom = CustomInstructions {
        source: ConfigSource::Global,
        file: "/g".to_owned(),
        text: hostile.clone(),
    };
    let paint = Paint::of(None);
    let clean = crate::board::terminal_text(&hostile);
    for width in [40_u16, 80] {
        let view = ready(
            Some(custom.clone()),
            Ok(&[Audience::Every, Audience::LeadPair]),
        );
        let rows = instructions_body(&view, width, paint);
        let lines: Vec<&str> = rows.iter().map(|(line, _)| line.as_str()).collect();
        for line in &lines {
            assert!(!line.chars().any(char::is_control), "{line:?}");
            assert!(
                line.chars().count() <= usize::from(width - LEFT),
                "{line:?}"
            );
        }
        assert!(lines.contains(&"source: global /g"), "{lines:?}");
        let from = lines.iter().position(|line| *line == "Rules for workers");
        let to = lines.iter().position(|line| *line == "Seats");
        let kept: Vec<&str> = lines[from.expect("rules heading") + 1..to.expect("seats")]
            .iter()
            .flat_map(|line| line.split_whitespace())
            .collect();
        let twice = [&clean, &clean].map(|text| text.split_whitespace());
        assert_eq!(kept, twice.into_iter().flatten().collect::<Vec<_>>());
        let words: Vec<&str> = lines
            .iter()
            .flat_map(|line| line.split_whitespace())
            .collect();
        let seat = [
            "lead", "·", "main", "·", "lead", "·", "every", "seat", "+", "lead", "pair",
        ];
        assert!(
            words.windows(seat.len()).any(|run| run == seat),
            "{lines:?}"
        );
    }
    let lines = |view| instructions_body(&view, 80, paint);
    let gapped = lines(ready(None, Err("unknown slot".to_owned())));
    let gapped: Vec<&str> = gapped.iter().map(|(line, _)| line.as_str()).collect();
    assert!(gapped.contains(&"none in /g"), "{gapped:?}");
    assert!(
        gapped.contains(&"lead · main · lead · gap: unknown slot"),
        "{gapped:?}"
    );
    let torn = InstructionsView::Gap("\u{1b}]0;x\u{7}torn".to_owned());
    assert_eq!(lines(torn)[0].0, "\u{fffd}]0;x\u{fffd}torn");
    let fit = "x".repeat(usize::from(80 - LEFT));
    let mut tight = custom;
    tight.text.clone_from(&fit);
    let filled = lines(ready(Some(tight), Ok(&[])));
    assert!(filled.iter().any(|row| row.0 == fit));
}

/// The spawn block draws only when `spawn_instructions` is in force, after
/// the custom instructions and before the rules; a config without the key
/// draws exactly the rows it drew before.
#[test]
fn instructions_rows_draw_the_spawn_block_only_when_the_key_is_in_force() {
    use crate::app::settings::{
        ConfigSource, CustomInstructions, InstructionsView, Protocol, SeatLine,
    };
    let protocol = |spawn| {
        InstructionsView::Ready(Protocol {
            session: "api".to_owned(),
            files: vec!["/g".to_owned()],
            custom: None,
            spawn,
            rules: Vec::new(),
            seats: vec![SeatLine {
                name: "lead".to_owned(),
                slot: "main".to_owned(),
                role: "lead",
                receives: Ok(&[]),
            }],
        })
    };
    let paint = Paint::of(None);
    let rows = |view| -> Vec<String> {
        instructions_body(&view, 120, paint)
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    };
    let before = rows(protocol(None));
    assert!(
        !before
            .iter()
            .any(|line| line.contains("Spawn instructions")),
        "{before:?}"
    );
    let after = rows(protocol(Some(CustomInstructions {
        source: ConfigSource::Session,
        file: "/s".to_owned(),
        text: "pick \u{1b}[31mcheap".to_owned(),
    })));
    let at = after
        .iter()
        .position(|line| line == "Spawn instructions ([prompt] spawn_instructions)")
        .expect("the spawn heading");
    assert_eq!(after[at + 1], "source: session /s");
    // The explanation wraps to the panel; the text row ends the block.
    let text_at = after
        .iter()
        .position(|line| line == "pick \u{fffd}[31mcheap")
        .expect("the neutralised text");
    let explained = after[at + 2..text_at]
        .iter()
        .flat_map(|line| line.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        explained.contains("pointing at the same section of workspace.md"),
        "{explained}"
    );
    let mut without: Vec<String> = after[..at - 1].to_vec();
    without.extend(after[text_at + 1..].iter().cloned());
    assert_eq!(without, before, "the block is the only difference");
}

/// Every write is bounds-checked, so no size panics: a populated screen
/// is drawn across both fallbacks and every threshold, with
/// fleets of one and of many and the chat scrolled past its top.
#[test]
fn no_size_panics() {
    let (overview, lane) = sweep_body();
    let entry = SessionEntry::new("s1", Status::Running);
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let look = Look::read("on", "darcula", "on", "on");
    let facts = sweep_seats();
    let draft = View {
        rows: vec!["a draft long enough to run past the composer row ".repeat(4)],
        text_rows: 0..1,
        cursor_row: 0,
        before: String::new(),
        anchor: String::new(),
    };
    let compose = Composer::Home {
        home: "s1",
        view: Some(&draft),
        draft: "",
    };
    let read_only = Composer::ReadOnly {
        why: "owned elsewhere",
    };
    // Each geometry the frame draws: the Agents body without and with
    // seats, and the browse and compose composers. Foreign, no-home and mono draw
    // the same rows with other words, so they add no geometry.
    for (at, (count, agents, composer)) in [
        (12, false, read_only),
        (1, false, read_only),
        (12, true, read_only),
        (12, false, compose),
    ]
    .into_iter()
    .enumerate()
    {
        let fleet = Fleet {
            rows: (1..=count)
                .map(|at| row(&format!("s{at}"), at, at % 3 == 0))
                .collect(),
            home: Some("s1".to_owned()),
        };
        let mut model = Model::new(&fleet);
        for _ in 0..40 {
            let _ = model.key(crate::app::model::Key::PageUp, &fleet, false);
        }
        let screen = Screen {
            fleet: &fleet,
            model: &model,
            overview: &overview,
            selected: Some(&entry),
            pair: &pair,
            agents: agents.then_some(&facts),
            lane: &lane,
            composer,
            look: Some(look),
            zone: None,
            now: Timestamp::from_epoch(1_759_500_600),
        };
        // The first geometry: every width at the heights around each
        // threshold, and every height at the widths around each one. The
        // others: the thresholds crossed, plus every width at two heights.
        let heights = [
            0, 1, 2, 5, 6, 7, 8, 9, 12, 19, 20, 21, 25, 30, 39, 40, 41, 45, 50,
        ];
        let widths = [0, 1, 39, 40, 41, 89, 90, 91, 139, 140, 141, 170];
        let (every_width, every_height): (&[u16], &[u16]) = if at == 0 {
            (&heights, &widths)
        } else {
            (&[20, 45], &[])
        };
        let sizes = (0..=170)
            .flat_map(|width| every_width.iter().map(move |height| (width, *height)))
            .chain(
                every_height
                    .iter()
                    .flat_map(|width| (0..=50).map(move |height| (*width, height))),
            )
            .chain(
                widths
                    .into_iter()
                    .flat_map(|width| heights.map(|height| (width, height))),
            );
        for (width, height) in sizes {
            let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
            draw(&screen, &mut buf);
        }
    }
}

/// F7: a text that must be clipped keeps its age, the ellipsis on the
/// text; a text that fits whole without its age stays whole (the frozen
/// 100x28 topic line); a room the age cannot share clips the text alone.
#[test]
fn a_tight_line_keeps_its_age_and_clips_the_text() {
    let text = "resume at scope checks";
    assert_eq!(aged(text, Some(120), 27), "resume at scope checks · 2m");
    assert_eq!(aged(text, Some(120), 26), "resume at scope checks");
    assert_eq!(aged(text, Some(120), 22), "resume at scope checks");
    assert_eq!(aged(text, Some(120), 21), "resume at scope… · 2m");
    assert_eq!(aged(text, Some(120), 20), "resume at scop… · 2m");
    assert_eq!(aged(text, Some(7_200), 8), "re… · 2h");
    assert_eq!(aged(text, Some(120), 6), "resum…");
    assert_eq!(aged(text, None, 10), "resum… · -");
}

/// Below the floor the app says what it needs on the rows it has: the
/// line wraps by word, so the pane's own size stays legible.
#[test]
fn a_pane_below_the_floor_still_reads_its_own_size() {
    let (overview, lane) = sweep_body();
    let fleet = Fleet {
        rows: vec![row("s1", 1, false)],
        home: Some("s1".to_owned()),
    };
    let model = Model::new(&fleet);
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: None,
        pair: &[],
        agents: None,
        lane: &lane,
        composer: Composer::NoHome,
        look: None,
        zone: None,
        now: Timestamp::from_epoch(1_759_500_600),
    };
    let rows = |width: u16, height: u16| {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        draw(&screen, &mut buf);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rows(30, 6)[..3],
        ["ae app needs at least 40x8", "(now 30x6)", ""]
    );
    assert_eq!(
        rows(39, 7)[..2],
        ["ae app needs at least 40x8 (now 39x7)", ""]
    );
}

// ---- mutation pins (pins-plan.md). Oracles: the o3-side-q frames' cells,
// ---- docs/app.md and docs/chat.md; never this module's output.

const PIN_NOW: i64 = 1_759_500_600;

/// One selected running session `api` drawn at a size, with the pieces a
/// pin varies.
struct Shot {
    fleet: Fleet,
    model: Model,
    overview: Overview,
    entry: SessionEntry,
    pair: Vec<String>,
    agents: Option<Facts>,
    lane: Lane,
}

impl Shot {
    fn new() -> Self {
        let fleet = Fleet {
            rows: vec![row("api", 1, false)],
            home: Some("api".to_owned()),
        };
        let model = Model::new(&fleet);
        Self {
            fleet,
            model,
            overview: Overview::default(),
            entry: SessionEntry::new("api", Status::Running),
            pair: vec!["lead".to_owned(), "colead".to_owned()],
            agents: None,
            lane: Lane::default(),
        }
    }

    fn key(&mut self, key: crate::app::model::Key) {
        let _ = self.model.key(key, &self.fleet, false);
    }

    fn draw(&self, width: u16, height: u16, composer: Composer<'_>) -> Buffer {
        self.laid(width, height, composer).0
    }

    fn screen<'a>(&'a self, composer: Composer<'a>) -> Screen<'a> {
        Screen {
            fleet: &self.fleet,
            model: &self.model,
            overview: &self.overview,
            selected: Some(&self.entry),
            pair: &self.pair,
            agents: self.agents.as_ref(),
            lane: &self.lane,
            composer,
            look: Some(Look::read("on", "darcula", "on", "on")),
            zone: None,
            now: Timestamp::from_epoch(PIN_NOW),
        }
    }

    /// One footer row on a one-row buffer: the closed frame's hint row
    /// in a sidebar-less chat column (cols 2..width-2), or the panel
    /// footer: what it drew, and the layout it recorded.
    fn footer_at(
        &self,
        width: u16,
        composer: Composer<'_>,
        closed: bool,
    ) -> (String, super::Layout) {
        let screen = self.screen(composer);
        let ctx = super::Ctx {
            screen: &screen,
            paint: Paint::of(screen.look.as_ref()),
            icons: true,
            wait: super::Wait::default(),
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 1));
        let mut layout = super::Layout::default();
        if closed {
            let (_, _, hint) = super::composer_lines(&ctx);
            super::hint_row(&ctx, &mut buf, &mut layout, (2, width - 2), &hint);
        } else {
            super::keys_row(&ctx, &mut buf, &mut layout);
        }
        (line(&buf, 0).concat().trim_end().to_owned(), layout)
    }

    /// The frame and the layout it recorded.
    fn laid(&self, width: u16, height: u16, composer: Composer<'_>) -> (Buffer, super::Layout) {
        let screen = self.screen(composer);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        let layout = super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
        (buf, layout)
    }
}

const READ_ONLY: Composer<'static> = Composer::ReadOnly {
    why: "owned elsewhere",
};

/// Row `y` as one char per cell.
fn line(buf: &Buffer, y: u16) -> Vec<String> {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_owned())
        .collect()
}

/// The first row holding `needle`, and the column it starts at.
fn spot(buf: &Buffer, needle: &str) -> Option<(u16, usize)> {
    (0..buf.area.height).find_map(|y| {
        let cells = line(buf, y);
        let text: String = cells.concat();
        let byte = text.find(needle)?;
        Some((y, text[..byte].chars().count()))
    })
}

fn last_ink(buf: &Buffer, y: u16, before: u16) -> Option<u16> {
    (0..before).rev().find(|x| buf[(*x, y)].symbol() != " ")
}

/// The last cell of the chat column's rule (row 1): where the drawn
/// column ends. The frames disagree by one at the right edge, so the
/// header is judged against the rule drawn with it.
fn rule_end(buf: &Buffer) -> Option<u16> {
    (0..buf.area.width)
        .rev()
        .find(|x| buf[(*x, 1)].symbol() == "─")
}

/// A stopped session is one row: `stopped 2d` at the right end, and no
/// second line counting its seats.
#[test]
fn a_stopped_row_says_how_long_it_has_been_stopped_on_its_one_row() {
    let mut shot = Shot::new();
    for (at, seats) in [(1, 3), (2, 1)] {
        let mut stopped = row(&format!("s{at}"), at, false);
        stopped.counts = Counts::Stopped;
        stopped.line2 = Line2::NotRunning {
            seats,
            stopped_secs: Some(172_800),
        };
        if at == 1 {
            shot.fleet.rows[0] = stopped;
        } else {
            shot.fleet.rows.push(stopped);
        }
    }
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(160, 43, READ_ONLY);
    let first = spot(&buf, " s1").expect("the first stopped row").0;
    assert!(
        line(&buf, first)[..44]
            .concat()
            .trim_end()
            .ends_with("stopped 2d"),
        "{:?}",
        line(&buf, first).concat()
    );
    assert_eq!(spot(&buf, " s2").map(|at| at.0), Some(first + 1));
    assert_eq!(spot(&buf, "not running"), None, "no second line");
}

/// Frames calm r18 (160x45) and needs-you r17 (100x30): the row the tab
/// row held now carries the static `Agents N` heading at the sidebar's
/// left edge, bold, with no `Overview` or `Tab` word. 90x20 is the
/// narrowest sidebar (rule 34).
#[test]
fn the_agents_heading_sits_where_the_frames_put_the_tab_row() {
    for (width, height) in [(160, 43), (100, 28), (90, 20)] {
        let mut shot = Shot::new();
        shot.agents = Some(match sweep_seats() {
            Facts::Seats { id, agents } => Facts::Seats {
                id,
                agents: agents.into_iter().take(6).collect(),
            },
            other => other,
        });
        let buf = shot.draw(width, height, READ_ONLY);
        let size = format!("{width}x{height}");
        let (y, x) = spot(&buf, "Agents 6").expect("a heading");
        assert_eq!(x, 2, "{size}");
        assert!(buf[(2, y)].modifier.contains(Modifier::BOLD), "{size}");
        assert_eq!(spot(&buf, "Overview"), None, "{size}");
        assert_eq!(spot(&buf, "Tab  │"), None, "{size}");
    }
}

/// At 100x30 the chat column is narrow: the header's reason keeps its age
/// and stays inside the column, cut with an ellipsis before the age.
#[test]
fn a_narrow_header_reason_keeps_its_age_inside_the_column() {
    let mut shot = Shot::new();
    shot.overview.open = vec![Open {
        seat: "ghost".to_owned(),
        text: "resume at scope checks after the fixtures land on main".to_owned(),
        age_secs: Some(360),
    }];
    let buf = shot.draw(100, 30, READ_ONLY);
    let header = line(&buf, 0).concat();
    assert!(header.contains("⚠ ghost: resume"), "{header:?}");
    let end = rule_end(&buf).expect("the chat column's rule");
    let (_, x) = spot(&buf, "· 6m").expect("the age is kept");
    assert!(x + 4 <= usize::from(end), "inside the column: {header:?}");
    assert!(header.contains("… · 6m"), "{header:?}");
}

/// The chat wrap contract: a long turn wraps inside the chat column and
/// every word of it is drawn.
#[test]
fn a_long_turn_wraps_inside_the_chat_column() {
    let mut shot = Shot::new();
    let body = (1..=36)
        .map(|at| format!("w{at:02}"))
        .collect::<Vec<_>>()
        .join(" ")
        + " "
        + &"filler ".repeat(16);
    shot.lane = Lane {
        items: vec![Item {
            micros: PIN_NOW * 1_000_000,
            kind: Kind::Said {
                who: "lead".to_owned(),
            },
            body: body.clone(),
            record: None,
        }],
        coverage: Vec::new(),
    };
    let buf = shot.draw(160, 45, READ_ONLY);
    let all: String = (0..buf.area.height)
        .map(|y| line(&buf, y).concat())
        .collect::<Vec<_>>()
        .join("\n");
    for word in body.split_whitespace().filter(|word| word.starts_with('w')) {
        assert!(all.contains(word), "{word} is drawn");
    }
}

fn turns(count: usize) -> Lane {
    Lane {
        items: (0..count)
            .map(|at| Item {
                micros: (PIN_NOW - 3_600) * 1_000_000 + i64::try_from(at).unwrap_or(0),
                kind: Kind::Said {
                    who: "lead".to_owned(),
                },
                body: format!("turn {at}"),
                record: None,
            })
            .collect(),
        coverage: Vec::new(),
    }
}

/// Issue 302: the day separator is a dim row above a turn from another
/// day than the one before it, and above an older lane's first turn; a lane
/// that starts today has none.
#[test]
fn a_day_separator_is_drawn_dim_above_a_turn_from_another_day() {
    let turn = |secs: i64, body: &str| Item {
        micros: secs * 1_000_000,
        kind: Kind::Said {
            who: "lead".to_owned(),
        },
        body: body.to_owned(),
        record: None,
    };
    let mut shot = Shot::new();
    // 2025-10-02T10:13Z (Thu), then 2025-10-03T13:10Z (Fri); PIN_NOW is
    // 2025-10-03T14:10Z. The dates are from GNU `date -u -d @…`.
    shot.lane = Lane {
        items: vec![turn(1_759_400_000, "older"), turn(PIN_NOW - 3_600, "today")],
        coverage: Vec::new(),
    };
    let buf = shot.draw(160, 45, READ_ONLY);
    let dim = super::colour(crate::theme::Palette::DARCULA.dim);
    let mut at = Vec::new();
    for want in ["── Thu 2025-10-02 ──", "── Fri 2025-10-03 ──"] {
        let (y, x) = spot(&buf, want).expect(want);
        assert_eq!(
            buf[(u16::try_from(x).expect("a column"), y)].fg,
            dim,
            "{want}"
        );
        at.push(y);
    }
    assert!(at[0] < at[1], "the older day is above the newer one");
    shot.lane = Lane {
        items: vec![turn(PIN_NOW - 3_600, "today")],
        coverage: Vec::new(),
    };
    let buf = shot.draw(160, 45, READ_ONLY);
    assert_eq!(spot(&buf, "2025-10-03"), None, "the lane starts today");
    assert!(spot(&buf, "today").is_some(), "the turn is drawn");
}

/// Issue 302: the hint under a scrolled-up lane is centered in the chat
/// column, drawn in the accent, and counts the turns that start below the
/// window; its words are a click target and nothing beside them is.
#[test]
fn the_newer_turns_hint_is_centered_accented_counted_and_clickable() {
    let mut shot = Shot::new();
    shot.lane = turns(150);
    shot.key(crate::app::model::Key::PageUp);
    // 45 rows: the lane window is 38 rows (rows 2..40) and a page up
    // hides the newest 38 of the 117 rows the frame produced, three per
    // turn (blank, header, body), 39 turns. The window then ends on the
    // blank row above the 14th-newest turn's header: 13 turns start below.
    let (buf, layout) = shot.laid(160, 45, READ_ONLY);
    let words = "↓ 13 newer turns · End";
    let (y, x) = spot(&buf, words).expect("the counted hint");
    let x = u16::try_from(x).expect("a column");
    let width = u16::try_from(words.chars().count()).expect("a width");
    // The composer's rule, the row under the hint, spans the chat column.
    let end = (0..buf.area.width)
        .rev()
        .find(|x| buf[(*x, y + 1)].symbol() == "─")
        .expect("the rule");
    let begin = (0..=end)
        .rev()
        .take_while(|x| buf[(*x, y + 1)].symbol() == "─")
        .last()
        .expect("the rule's first cell");
    let room = end + 1 - begin;
    assert_eq!(x - begin, (room - width) / 2, "centered in the column");
    let accent = super::colour(crate::theme::Palette::DARCULA.title);
    let dim = super::colour(crate::theme::Palette::DARCULA.dim);
    for at in x..x + width {
        let cell = &buf[(at, y)];
        assert_eq!(cell.fg, accent, "accent at {at}");
        assert_ne!(cell.fg, dim);
    }
    let press = |column: u16, row: u16| Mouse {
        kind: MouseKind::Click,
        column,
        row,
    };
    for at in x..x + width {
        let mouse = press(at, y);
        assert!(
            matches!(layout.hit(mouse), Some(super::Hit::Newest)),
            "the words are the target at {at}"
        );
        assert!(layout.file(mouse).is_none(), "no file link under it");
        assert!(!layout.link_at(mouse), "no web link under it");
    }
    for beside in [x - 1, x + width] {
        let hit = layout.hit(press(beside, y));
        assert!(!matches!(hit, Some(super::Hit::Newest)), "beside the words");
    }
    let above = layout.hit(press(x, y - 1));
    assert!(!matches!(above, Some(super::Hit::Newest)), "the row above");
}

/// One turn below reads in the singular, and the rest of the newest turn
/// alone below counts no turn.
#[test]
fn the_newer_turns_hint_counts_one_and_none() {
    let mut shot = Shot::new();
    shot.lane = turns(150);
    shot.model.wheel(true, 38, 1000);
    // One notch hides three rows: the newest turn's blank, header, body.
    let buf = shot.draw(160, 45, READ_ONLY);
    assert!(spot(&buf, "↓ 1 newer turn · End").is_some(), "one turn");
    shot.lane.items.last_mut().expect("a turn").body = "a\nb\nc\nd\ne\nf".to_owned();
    let buf = shot.draw(160, 45, READ_ONLY);
    // Its blank, header and six body rows are drawn; the last three rows
    // (d, e, f) hide, the header stays: no turn starts below.
    assert!(spot(&buf, "↓ more below · End").is_some(), "no turn");
    assert_eq!(spot(&buf, "newer turn"), None);
}

/// The `↓ 3 newer turns · End` hint means newer turns are hidden below:
/// none for a lane that fits, even asked to scroll; one on a long lane
/// scrolled a page up.
#[test]
fn the_newer_turns_hint_shows_only_when_newer_turns_are_hidden() {
    const HINT: &str = "newer turns · End";
    let mut shot = Shot::new();
    shot.lane = turns(2);
    shot.key(crate::app::model::Key::PageUp);
    let fits = shot.draw(160, 45, READ_ONLY);
    for label in [HINT, "newer turn · End", "more below · End"] {
        assert_eq!(spot(&fits, label), None, "a lane that fits: {label}");
    }
    shot.lane = turns(150);
    let buf = shot.draw(160, 45, READ_ONLY);
    assert!(spot(&buf, HINT).is_some(), "a long lane a page up");
    // A viewed seat replaces the column: no chat hint under its screen,
    // and the chat scroll is kept for the return.
    let viewing = shot.draw(160, 45, Composer::Viewing { seat: "colead" });
    assert_eq!(spot(&viewing, HINT), None, "viewing suppresses the hint");
    assert_eq!(shot.model.pages(), 1, "the chat scroll is retained");
    let back = shot.draw(160, 45, READ_ONLY);
    assert!(
        spot(&back, HINT).is_some(),
        "the hint returns with the chat"
    );
}

/// A pane with no row for the lane draws no hint, however far the chat is
/// scrolled: the hint stands under lane rows that hide newer ones.
#[test]
fn a_pane_with_no_room_for_the_lane_draws_no_hint() {
    let mut shot = Shot::new();
    shot.lane = turns(150);
    shot.model.wheel(true, 38, 1000);
    // The smallest pane is 8 rows high; a two-row draft takes the lane's
    // last row there. One row taller leaves the lane one row, which
    // draws the hint, so the absence below is the pane's, not the fixture's.
    let draft = drafted(&["one", "two"]);
    let one_row = shot.draw(160, 9, writing(&draft, "one\ntwo"));
    assert!(spot(&one_row, "newer turn").is_some(), "one lane row");
    let none = shot.draw(160, 8, writing(&draft, "one\ntwo"));
    for label in ["newer turn", "more below"] {
        assert_eq!(spot(&none, label), None, "no lane row: {label}");
    }
}

/// The header stays inside the chat column (frames: header text never
/// passes the rule under it). A name at the grammar's 128 and a long
/// pair middle both stay inside it.
#[test]
fn a_long_header_stays_inside_the_chat_column() {
    let mut shot = Shot::new();
    shot.entry = SessionEntry::new("n".repeat(128), Status::Running);
    let buf = shot.draw(160, 45, READ_ONLY);
    let end = rule_end(&buf).expect("the rule");
    assert!(
        last_ink(&buf, 0, 160).is_some_and(|x| x <= end),
        "a long name"
    );
    let mut shot = Shot::new();
    shot.pair = vec!["h".repeat(60), "g".repeat(60)];
    let buf = shot.draw(160, 45, READ_ONLY);
    let (y, _) = spot(&buf, "hhh").expect("the long pair middle");
    assert_eq!(y, 0);
    let end = rule_end(&buf).expect("the rule");
    assert!(
        last_ink(&buf, 0, 160).is_some_and(|x| x <= end),
        "a long middle"
    );
}

/// A stopped session's header says so, as its sidebar row does (frame
/// many r18 `stopped 2d`), never `active`.
#[test]
fn a_stopped_session_header_says_stopped() {
    let mut shot = Shot::new();
    shot.entry = SessionEntry::new("api", Status::Stopped);
    shot.entry.last_active_epoch = Some(PIN_NOW - 172_800);
    let buf = shot.draw(160, 45, READ_ONLY);
    let header = line(&buf, 0).concat();
    assert!(
        header.trim_end().ends_with("stopped 2d  X zoom"),
        "{header:?}"
    );
    assert!(!header.contains("active"), "{header:?}");
}

/// Frame calm r1 draws `feat/auth-v2  ·  active 1m` when it fits after
/// the middle; when it would not fit after a two-cell gap the branch is
/// dropped, the activity stays right-aligned to the column's end (the
/// rule's last cell, now the zoom label's) and the middle is whole. At 160 the
/// middle starts @53 and the 34-cell branch and label end the column: 69
/// cells leave two blank cells before the branch, 70 leave one.
#[test]
fn the_header_drops_the_branch_before_it_overlaps() {
    let branch = "feat/auth-v2  ·  active 1m  X zoom";
    for (middle, fits) in [(69, true), (70, false)] {
        let mut shot = Shot::new();
        shot.entry.branch = Some("feat/auth-v2".to_owned());
        shot.entry.last_active_epoch = Some(PIN_NOW - 60);
        // `lead + ` is 7 cells, so the second name fills the middle.
        shot.pair = vec!["lead".to_owned(), "c".repeat(middle - 7)];
        let buf = shot.draw(160, 43, READ_ONLY);
        let header = line(&buf, 0).concat();
        let shown = header.trim_end();
        assert!(
            shown.contains(&shot.pair.join(" + ")),
            "the middle is whole: {header:?}"
        );
        assert_eq!(last_ink(&buf, 0, 160), rule_end(&buf), "{header:?}");
        if fits {
            assert!(shown.ends_with(branch), "{header:?}");
        } else {
            assert!(
                shown.ends_with("active 1m  X zoom") && !shown.contains("feat/auth-v2"),
                "{header:?}"
            );
        }
    }
}

/// The activity word is drawn only while it fits after the middle: at 160
/// columns 86 middle cells leave its two blank cells and nine, 87 do not.
#[test]
fn the_header_drops_the_activity_when_it_would_not_fit() {
    for (middle, fits) in [(86, true), (87, false)] {
        let mut shot = Shot::new();
        shot.entry.last_active_epoch = Some(PIN_NOW - 60);
        shot.pair = vec!["lead".to_owned(), "c".repeat(middle - 7)];
        let header = line(&shot.draw(160, 43, READ_ONLY), 0).concat();
        assert_eq!(header.contains("active 1m"), fits, "{middle}: {header:?}");
    }
}

/// A recorded directory keeps twelve cells before the branch takes them:
/// at 160 columns 57 middle cells leave the branch label (26 cells) room
/// beside the directory for fewer than twelve, so the branch gives way to
/// the activity word and the directory stays whole.
#[test]
fn the_header_drops_the_branch_before_it_squeezes_the_directory() {
    let mut shot = Shot::new();
    shot.entry.branch = Some("feat/auth-v2".to_owned());
    shot.entry.last_active_epoch = Some(PIN_NOW - 60);
    shot.overview.dir = Some("projects/xyz".to_owned());
    shot.pair = vec!["lead".to_owned(), "c".repeat(50)];
    let header = line(&shot.draw(160, 43, READ_ONLY), 0).concat();
    assert!(
        header.contains("projects/xyz")
            && header.contains("active 1m")
            && !header.contains("feat/auth-v2"),
        "{header:?}"
    );
}

// ---- lane pins (#74, #80-85, #90), drawn through `lane::rows` here because
// ---- `Paint::of` is this module's own. Oracle: the frames' chat column.

fn said(who: &str, at: i64) -> Item {
    Item {
        micros: (PIN_NOW + at) * 1_000_000,
        kind: Kind::Said {
            who: who.to_owned(),
        },
        body: format!("from {who}"),
        record: None,
    }
}

fn lane_rows(items: Vec<Item>) -> Vec<ratatui_core::text::Line<'static>> {
    let look = Look::read("on", "darcula", "on", "on");
    let lane = Lane {
        items,
        coverage: Vec::new(),
    };
    let clock = crate::console::view::Style::PLAIN;
    crate::app::lane::rows(&lane, 100, super::Paint::of(Some(&look)), &clock, "lead")
}

fn text(line: &ratatui_core::text::Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// Frame calm chat column r16-r24: a turn's head, its body, ONE blank
/// row, the next head; nothing before the first turn.
#[test]
fn turns_are_separated_by_one_blank_row() {
    let rows = lane_rows(vec![said("lead", 0), said("colead", 60)]);
    let shown: Vec<String> = rows.iter().map(text).collect();
    assert_eq!(shown.len(), 5, "{shown:?}");
    assert!(shown[0].starts_with("lead"), "{shown:?}");
    assert_eq!(shown[1].trim(), "from lead", "{shown:?}");
    assert_eq!(shown[2], "", "{shown:?}");
    assert!(shown[3].starts_with("colead"), "{shown:?}");
}

/// The frames' speaker hues (palette `lead` #6897BB, `sys` = the dim
/// role): the main seat speaks in the lead hue, the colead in another
/// voice that is neither the lead hue nor dim (R7: text, bold), ae dim.
#[test]
fn speakers_wear_the_frames_hues() {
    use ratatui_core::style::{Color, Modifier};
    let rows = lane_rows(vec![said("lead", 0), said("colead", 60), said("ae", 120)]);
    let speaker = |at: usize| rows[at].spans[0].style;
    let (lead, colead, ae) = (speaker(0), speaker(3), speaker(6));
    let [r, g, b] = crate::theme::rgb(crate::theme::Palette::DARCULA.dim);
    let dim = Some(Color::Rgb(r, g, b));
    assert_eq!(lead.fg, Some(Color::Rgb(0x68, 0x97, 0xBB)), "the lead hue");
    assert_ne!(colead.fg, lead.fg, "the colead is not the lead");
    assert_ne!(colead.fg, dim, "the colead is not dim");
    assert!(
        colead.add_modifier.contains(Modifier::BOLD),
        "R7: the colead is bold"
    );
    assert_eq!(ae.fg, dim, "ae speaks dim, as the frames' sys");
}

/// The chat's own tag (console/lane.rs goldens `lead answers · follow-up 1`):
/// a first answer is plain `answers`, a later one names its follow-up.
#[test]
fn an_answer_names_its_follow_up_as_the_chat_does() {
    let answer = |follow_up: usize| Item {
        micros: PIN_NOW * 1_000_000,
        kind: Kind::Answer {
            seat: "lead".to_owned(),
            id: "ae-20260930T060000Z-000000aa".to_owned(),
            follow_up,
            late: false,
            gap: None,
            speaker: None,
        },
        body: "done".to_owned(),
        record: None,
    };
    let head = |follow_up| text(&lane_rows(vec![answer(follow_up)])[0]);
    let first = head(0);
    assert!(first.trim_end().ends_with("answers"), "{first:?}");
    assert!(head(2).trim_end().ends_with("answers · follow-up 2"));
}

/// The frames' grounds. A frame is the terminal, its last two rows tmux's
/// status, so the app pane is 160x43 (100x28). The sidebar is panel
/// #313335 on every row and everything from its rule on is base
/// #2B2B2B, the first and last rows included.
#[test]
fn the_grounds_are_the_frames() {
    use ratatui_core::style::Color;
    let (panel, chat) = (Color::Rgb(0x31, 0x33, 0x35), Color::Rgb(0x2B, 0x2B, 0x2B));
    let shot = Shot::new();
    for (width, height, rule, inside) in [(160, 43, 44, 100), (100, 28, 34, 70)] {
        let buf = shot.draw(width, height, READ_ONLY);
        let size = format!("{width}x{height}");
        for x in [1, rule - 1] {
            assert_eq!(buf[(x, 10)].bg, panel, "{size}: the sidebar at {x}");
        }
        assert_eq!(
            buf[(0, height - 1)].bg,
            panel,
            "{size}: the sidebar on the last row"
        );
        for (x, y) in [(rule, 10), (inside, 0), (inside, 10), (inside, height - 1)] {
            assert_eq!(buf[(x, y)].bg, chat, "{size}: the chat ground at ({x},{y})");
        }
    }
}

/// A body that does not fit gives its last row to a dim line naming the
/// rows it hides; a body that fits names none (ruling C3).
#[test]
fn a_cut_body_names_its_hidden_rows_in_the_dim_hue() {
    let mut shot = Shot::new();
    let Facts::Seats { id, agents } = sweep_seats() else {
        panic!("the sweep's seats");
    };
    let agents = agents.into_iter().cycle().take(20).collect();
    shot.agents = Some(Facts::Seats { id, agents });
    let buf = shot.draw(100, 21, READ_ONLY);
    assert_eq!(
        spot(&buf, "↓ 26 more rows"),
        Some((20, 2)),
        "the body's last row"
    );
    let dim = super::colour(crate::theme::Palette::DARCULA.dim);
    assert_eq!(buf[(2, 20)].fg, dim);
    let tall = shot.draw(100, 40, READ_ONLY);
    assert_eq!(
        spot(&tall, "↓ 7 more rows"),
        Some((39, 2)),
        "the wrapped body no longer fits"
    );
    shot.fleet = frame_fleet();
    // The smallest sidebar frame gives the body six rows, the last of
    // them the line naming what it hides.
    let smallest = shot.draw(100, 20, READ_ONLY);
    assert_eq!(spot(&smallest, "↓ 35 more rows").map(|at| at.0), Some(19));
}

/// docs/app.md: below 40x8 the app only says how large it needs to be;
/// 40x8 itself is the app.
#[test]
fn the_floor_is_below_forty_by_eight() {
    const FLOOR: &str = "ae app needs at least";
    let shot = Shot::new();
    assert_eq!(
        spot(&shot.draw(40, 8, READ_ONLY), FLOOR),
        None,
        "40x8 draws the app"
    );
    for (width, height) in [(39, 8), (40, 7)] {
        let buf = shot.draw(width, height, READ_ONLY);
        let all: String = (0..height)
            .map(|y| line(&buf, y).concat())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(all.contains("ae app needs"), "{width}x{height}: {all:?}");
    }
}

/// The floor wraps by word onto the rows there are (docs/app.md:40): a
/// word that fills its row exactly stays on it.
#[test]
fn a_floor_word_that_fits_exactly_stays_on_its_row() {
    let buf = Shot::new().draw(21, 7, READ_ONLY);
    assert_eq!(line(&buf, 0).concat(), "ae app needs at least");
}

/// The rule stands at 44 on every row, the hint row included: the
/// writing hint stands in the chat column right of it.
#[test]
fn the_rule_runs_the_full_height_beside_the_hint_row() {
    let draft = View {
        rows: vec!["a short draft".to_owned()],
        text_rows: 0..1,
        cursor_row: 0,
        before: String::new(),
        anchor: String::new(),
    };
    let composing = Composer::Home {
        home: "api",
        view: Some(&draft),
        draft: "a short draft",
    };
    let buf = Shot::new().draw(160, 43, composing);
    for y in 0..43 {
        assert_eq!(buf[(44, y)].symbol(), "│", "row {y}");
    }
    let keys = line(&buf, 42).concat();
    assert!(
        keys.contains("Enter sends · Alt+Enter ^J newline · Esc browses"),
        "the writing hint stands on the row: {keys:?}"
    );
}

/// The hint row names `^C clears` over typed text and never over an empty
/// composer, nor over a draft Esc kept (its hint is `Enter writes`); at a width that cannot hold it whole it drops.
#[test]
fn the_hint_row_names_the_clear_key_only_over_text() {
    let shot = Shot::new();
    let (full, empty) = (drafted(&["hello"]), drafted(&[""]));
    for (composer, named, words) in [
        (writing(&full, "hello"), true, "typed text"),
        (writing(&empty, ""), false, "an empty composer"),
        (
            browsing("kept"),
            false,
            "a kept draft in browse (Enter writes)",
        ),
        (browsing(""), false, "no draft"),
    ] {
        let buf = shot.draw(160, 43, composer);
        let keys = line(&buf, 42).concat();
        assert_eq!(keys.contains("^C clears"), named, "{words}: {keys:?}");
    }
    let narrow = shot.draw(60, 43, writing(&full, "hello"));
    let keys = line(&narrow, 42).concat();
    assert!(!keys.contains("^C clears"), "dropped, not cut: {keys:?}");
}

/// One summary row names live agents, working and needs-you, using short
/// labels where the sidebar cannot hold all full labels.
#[test]
fn the_summary_band_names_each_count_at_any_width() {
    let mut shot = sessions(9);
    for row in &mut shot.fleet.rows {
        row.counts = Counts::Marks(vec![(Mark::Working, false, 2)]);
    }
    shot.model = Model::new(&shot.fleet);
    for (width, expected) in [
        (90, "Agt 18 · ● ◉ Wrk 18 · ⚠ Need 0"),
        (120, "Agt 18 · ● ◉ Wrk 18 · ⚠ Need 0"),
    ] {
        let (buf, layout) = declutter_frame(&shot, width, true);
        assert_eq!(declutter_line(&buf, &layout, 0).trim(), expected);
        assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
    }
}

/// Fixed labels keep a done-but-live agent separate from working.
#[test]
fn the_summary_words_agree_with_their_counts() {
    let mut shot = sessions(1);
    shot.fleet.rows[0].counts = Counts::Marks(vec![(Mark::Done, false, 1)]);
    shot.fleet.rows[0].needy = true;
    shot.model = Model::new(&shot.fleet);
    let (buf, layout) = declutter_frame(&shot, 160, true);
    assert_eq!(
        declutter_line(&buf, &layout, 0).trim(),
        "Agents 1 · ● ◉ Working 0 · ⚠ Needs you 1"
    );
    assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
}

/// Working wears the two marks its rows wear and Needs you its one, before the
/// label (`● ◉` and `⚠`, `* &` and `!` with icons off) in the mark's accent and the
/// line's own tone; Agents wears none. A width that holds only the icons
/// draws them with the same cells and ink.
#[test]
fn the_summary_words_wear_the_marks_their_rows_wear() {
    let palette = crate::theme::Palette::DARCULA;
    let working = super::colour(palette.accent(Mark::Working));
    let background = super::colour(palette.accent(Mark::Background));
    for needy in [false, true] {
        for icons in [true, false] {
            let (work, bg, need) = if icons {
                ("●", "◉", "⚠")
            } else {
                ("*", "&", "!")
            };
            let ink = super::colour(if needy {
                palette.needs_you
            } else {
                palette.text
            });
            let mut shot = sessions(9);
            for row in &mut shot.fleet.rows {
                row.counts = Counts::Marks(vec![(Mark::Working, false, 2)]);
            }
            shot.fleet.rows[0].needy = needy;
            shot.model = Model::new(&shot.fleet);
            let rungs = [(90, "Agt 18 · ", "Wrk", "Need")];
            for (width, head, working_word, needs_word) in rungs {
                let (buf, layout) = declutter_frame(&shot, width, icons);
                let text = declutter_line(&buf, &layout, 0);
                let want = format!(
                    "{head}{work} {bg} {working_word} 18 · {need} {needs_word} {}",
                    u8::from(needy)
                );
                assert_eq!(text.trim(), want, "{width}/{icons}/{needy}");
                let at = |glyph: &str| {
                    (0..layout.list.width)
                        .find(|x| buf[(*x, 0)].symbol() == glyph)
                        .expect("the mark is drawn")
                };
                assert_eq!(buf[(at(work), 0)].fg, working, "{width}/{icons}");
                assert_eq!(buf[(at(bg), 0)].fg, background, "{width}/{icons}/bg");
                assert_eq!(buf[(at(need), 0)].fg, ink, "{width}/{icons}/{needy}");
            }
        }
    }
    let mut shot = sessions(1);
    shot.fleet.rows[0].counts = Counts::Marks(vec![(Mark::Working, false, 123_456_789)]);
    shot.model = Model::new(&shot.fleet);
    let (buf, layout) = declutter_frame(&shot, 90, true);
    let text = declutter_line(&buf, &layout, 0);
    assert_eq!(text.trim(), "◈ 123456789 ● ◉ 123456789 ⚠ 0");
    let mark = (0..layout.list.width).find(|x| buf[(*x, 0)].symbol() == "●");
    assert_eq!(mark.map(|x| buf[(x, 0)].fg), Some(working), "icons rung");
}

/// Working counts a seat that runs background work with the seats that
/// work: one working and one background seat make Working 2, a fleet of
/// background seats only counts them, and both marks lead the label.
#[test]
fn the_working_count_includes_background_seats() {
    let mut shot = sessions(1);
    shot.fleet.rows[0].counts = Counts::Marks(vec![
        (Mark::Working, false, 1),
        (Mark::Background, false, 1),
    ]);
    shot.model = Model::new(&shot.fleet);
    assert_eq!(super::summary(&shot.fleet).working, 2);
    let (buf, layout) = declutter_frame(&shot, 200, true);
    assert_eq!(
        declutter_line(&buf, &layout, 0).trim(),
        "Agents 2 · ● ◉ Working 2 · ⚠ Needs you 0"
    );
    let (buf, layout) = declutter_frame(&shot, 200, false);
    assert_eq!(
        declutter_line(&buf, &layout, 0).trim(),
        "Agents 2 · * & Working 2 · ! Needs you 0"
    );
    let mut shot = sessions(2);
    for row in &mut shot.fleet.rows {
        row.counts = Counts::Marks(vec![(Mark::Background, false, 3)]);
    }
    shot.model = Model::new(&shot.fleet);
    assert_eq!(super::summary(&shot.fleet).working, 6);
    let (buf, layout) = declutter_frame(&shot, 90, true);
    assert_eq!(
        declutter_line(&buf, &layout, 0).trim(),
        "Agt 6 · ● ◉ Wrk 6 · ⚠ Need 0"
    );
}

/// Narrow summaries keep complete digits by dropping labels first; icons
/// off uses the corresponding ASCII glyphs.
#[test]
fn a_narrow_summary_band_drops_words_before_counts() {
    let mut shot = sessions(9);
    for row in &mut shot.fleet.rows {
        row.counts = Counts::Marks(vec![(Mark::Working, false, 2)]);
    }
    shot.fleet.rows[0].counts = Counts::Marks(vec![(Mark::Working, false, 123_456_789)]);
    shot.model = Model::new(&shot.fleet);
    for (icons, expected) in [
        (true, "◈ 123456805 ● ◉ 123456805 ⚠ 0"),
        (false, "@ 123456805 * & 123456805 ! 0"),
    ] {
        let (buf, layout) = declutter_frame(&shot, 90, icons);
        assert_eq!(declutter_line(&buf, &layout, 0).trim(), expected);
        assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
    }
}

/// The quiet summary stays on the sidebar ground; only `Agents` keeps a
/// raised band. A blank row separates the summary from the compact list.
#[test]
fn only_the_agents_heading_keeps_the_raised_panel_ground() {
    let mut shot = sessions(4);
    shot.agents = Some(seat_facts(&["lead"]));
    for name in ["darcula", "a", "b"] {
        let look = Look::read("on", name, "on", "on");
        let (buf, _) = framed(&shot, 160, Some(&look));
        let palette = look.palette;
        let (panel, base) = (super::colour(palette.panel), super::colour(palette.base));
        let rule = (0..160)
            .find(|x| buf[(*x, 0)].symbol() == "│")
            .expect("the rule");
        let heading = (2..45)
            .find(|y| line(&buf, *y).concat().contains("Agents"))
            .expect("the Agents heading");
        for x in 0..rule {
            assert_eq!(buf[(x, heading)].bg, panel, "{name} band ({x},{heading})");
        }
        for y in [0, 1, 2, 3, heading + 2, heading + 3] {
            for x in 0..rule - 2 {
                assert_ne!(buf[(x, y)].bg, panel, "{name} row ({x},{y})");
            }
        }
        assert_eq!(buf[(5, 5)].bg, base, "{name}: a session row's ground");
    }
}

/// Summary counts keep regular text ink; labels and separators use dim
/// ink on base. The Agents band keeps text ink on panel.
#[test]
fn the_summary_uses_regular_counts_and_dim_labels_on_base() {
    let mut shot = sessions(30);
    shot.fleet.rows[0].needy = true;
    // Both needy sessions are in view and each tallies one working agent: an
    // arrow or a third digit would cost the cells the short rung, with its
    // marks, fills.
    shot.fleet.rows[1].needy = true;
    for row in &mut shot.fleet.rows {
        row.counts = Counts::Marks(vec![(Mark::Working, false, 1)]);
    }
    shot.agents = Some(seat_facts(&["lead"]));
    shot.model = Model::new(&shot.fleet);
    for name in ["darcula", "a", "b"] {
        let look = Look::read("on", name, "on", "on");
        let (buf, layout) = framed(&shot, 160, Some(&look));
        let palette = look.palette;
        let text = super::colour(palette.text);
        let dim = super::colour(palette.dim);
        assert_eq!(buf[(2, 0)].fg, dim, "{name}: Agents label");
        assert_eq!(buf[(6, 0)].fg, text, "{name}: agent count");
        for y in [0, 1] {
            for x in 0..44 {
                assert!(
                    !buf[(x, y)].modifier.contains(Modifier::BOLD),
                    "{name}: regular summary at {x},{y}"
                );
            }
        }
        let heading = (2..45)
            .find(|y| line(&buf, *y).concat().contains("Agents"))
            .expect("the Agents heading");
        assert_eq!(buf[(2, heading)].fg, text, "{name}: Agents heading text");
        let label = (2..layout.list.width)
            .find(|x| buf[(*x, 0)].symbol() == "N")
            .expect("Needs you label");
        assert_eq!(buf[(label, 0)].fg, dim, "{name}: Needs you label");
        assert_eq!(
            buf[(label + 5, 0)].symbol(),
            "2",
            "{name}: two needy sessions"
        );
        assert_eq!(
            buf[(label + 5, 0)].fg,
            super::colour(palette.needs_you),
            "{name}: needs-you count"
        );
        let mut accented = 0;
        for y in [0, 1, heading] {
            for x in 0..layout.list.width {
                let cell = &buf[(x, y)];
                if cell.symbol() == " " || cell.fg == text || (y < 2 && cell.fg == dim) {
                    continue;
                }
                assert!(
                    matches!(cell.symbol(), "↑" | "↓")
                        || (y == 0 && matches!(cell.symbol(), "●" | "⚠"))
                        || (y == 0 && x == label + 5 && cell.symbol() == "2"),
                    "{name} ({x},{y}): {:?} wears {:?}",
                    cell.symbol(),
                    cell.fg
                );
                accented += 1;
            }
        }
        assert!(
            accented >= 1,
            "{name}: the marks and the needs-you count keep their accents"
        );
        assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
    }
}

/// One blank row separates the two-row summary from the compact session
/// list. Hit and selection rectangles begin at the first session.
#[test]
fn one_blank_row_separates_the_summary_from_the_session_list() {
    for count in [0, 1, 4, 30] {
        for (width, height) in [(90, 20), (120, 30), (220, 60)] {
            let (buf, layout) = sessions(count).laid(width, height, READ_ONLY);
            let left = |y| line(&buf, y)[..usize::from(layout.list.width)].concat();
            let frame = format!("{count} sessions at {width}x{height}");
            assert!(
                left(0).trim_start().starts_with(['A', '◈', '@', '⚠', '!']),
                "{frame}"
            );
            let first = if count == 0 { "No sessions." } else { "s00" };
            assert!(left(1).trim().is_empty(), "{frame}: spacer");
            assert!(left(2).contains(first), "{frame}: row 2 is {:?}", left(2));
            assert_eq!(layout.list.y, 2, "{frame}");
        }
    }
}

/// The closed hint row carries the composer's hint, the mode's keys and
/// the version/gear; held has no hint and no keys. A panel's footer
/// keeps its mode word and its one hint.
#[test]
fn the_hint_row_and_the_panel_footer_carry_at_most_one_hint() {
    let mut shot = Shot::new();
    let draft = drafted(&["a draft"]);
    let tail = format!("{} \u{2699}", crate::shape::version_line());
    let named = [
        "Enter",
        "Esc browse",
        "^C",
        "qq",
        "oo",
        "1-9",
        "Tab",
        "j/k",
        "n/p",
        "PgUp",
        "settings",
        "! next",
        "Esc home",
        "help",
    ];
    let refused = Composer::Held {
        why: "owned elsewhere",
    };
    let cases = [
        (browsing("kept"), "draft kept · Enter writes · ? keys"),
        (READ_ONLY, "? keys"),
        (
            writing(&draft, "a draft"),
            "Enter sends · Alt+Enter ^J newline · Esc browses · ^C clears",
        ),
        (refused, ""),
    ];
    for (composer, words) in cases {
        let (row, _) = shot.footer_at(100, composer, true);
        let head = row
            .strip_suffix(tail.as_str())
            .expect("version and gear end it");
        // The orchestrator button stands in the gutter before the hint.
        assert_eq!(
            head.trim_end(),
            format!("\u{25c6} {words}").trim_end(),
            "{words}"
        );
        if words.is_empty() {
            assert!(named.iter().all(|key| !row.contains(key)), "{row}");
        }
    }
    shot.model.open_settings(1);
    let (row, _) = shot.footer_at(100, browsing(""), false);
    let head = row
        .strip_suffix(tail.as_str())
        .expect("version and gear end it");
    assert_eq!(head.trim_end(), "  settings   Esc close");
}

/// Narrower than the floor (the app never draws it, the row still has
/// its order): the version goes first, then the gear, and the gear is
/// the settings click target for as long as it is drawn.
#[test]
fn narrowing_the_hint_row_drops_the_version_then_the_gear() {
    let shot = Shot::new();
    let gear = "\u{2699}";
    let tail = format!("{} {gear}", crate::shape::version_line());
    let head = "\u{25c6} Enter writes · ? keys";
    let full = head.chars().count() + 2 + tail.chars().count();
    let drop = |width: usize, drawn: &str| {
        let at = u16::try_from(width).unwrap_or(0);
        let (row, layout) = shot.footer_at(at, browsing(""), true);
        assert!(row.starts_with(head), "{width}: {row:?}");
        assert_eq!(
            row.trim_start_matches(head).trim(),
            drawn,
            "{width}: {row:?}"
        );
        let click = Mouse {
            kind: MouseKind::Click,
            column: at - 1,
            row: 0,
        };
        assert_eq!(
            matches!(layout.hit(click), Some(super::Hit::Settings)),
            !drawn.is_empty(),
            "{width}: the gear is the click target while drawn"
        );
    };
    drop(full + 5, &tail);
    drop(full, &tail);
    drop(full - 1, gear);
    drop(head.chars().count() + 3, gear);
    drop(head.chars().count() + 2, "");
}

/// A draft of `rows`, its cursor after the last row's cells.
fn drafted(rows: &[&str]) -> View {
    View {
        rows: rows.iter().map(|row| (*row).to_owned()).collect(),
        text_rows: 0..rows.len(),
        cursor_row: rows.len().saturating_sub(1),
        before: rows.last().map(|row| (*row).to_owned()).unwrap_or_default(),
        anchor: String::new(),
    }
}

/// A composer writing `view`.
fn writing<'a>(view: &'a View, draft: &'a str) -> Composer<'a> {
    Composer::Home {
        home: "api",
        view: Some(view),
        draft,
    }
}

/// A composer browsing with `draft` kept.
fn browsing(draft: &str) -> Composer<'_> {
    Composer::Home {
        home: "api",
        view: None,
        draft,
    }
}

/// Row `y` from column `x` to the end, trailing blanks trimmed.
fn row_from(buf: &Buffer, y: u16, x: usize) -> String {
    line(buf, y)[x..].concat().trim_end().to_owned()
}

fn column(x: usize) -> u16 {
    u16::try_from(x).unwrap_or(0)
}

/// The composer is the address on its own row and the draft the rows
/// under it, from the column's left edge: the cursor follows the last
/// row, and writing carries its keys hint on the keys row.
#[test]
fn a_draft_sits_under_its_address_from_the_columns_left_edge() {
    let shot = Shot::new();
    let draft = drafted(&["first words", "second"]);
    for (width, height) in [(100, 30), (60, 20)] {
        let size = format!("{width}x{height}");
        let (buf, layout) = shot.laid(width, height, writing(&draft, "first words second"));
        let (ay, ax) = spot(&buf, "to api").expect("the address");
        assert_eq!(ay, height - 4, "{size}: over two draft rows and the hint");
        assert_eq!(row_from(&buf, ay, ax), "to api", "{size}: alone on its row");
        assert_eq!(spot(&buf, "first words"), Some((ay + 1, ax)), "{size}");
        assert_eq!(spot(&buf, "second"), Some((ay + 2, ax)), "{size}");
        assert_eq!(
            layout.cursor,
            Some(Position::new(column(ax + 6), ay + 2)),
            "{size}: after the last row"
        );
        assert_eq!(buf[(column(ax), ay - 1)].symbol(), "─", "{size}: the rule");
        let all: String = (0..height).map(|y| line(&buf, y).concat()).collect();
        assert!(all.contains("Enter sends"), "{size}: the writing hint");
    }
}

/// Browsing keeps the address on its row and shows a kept draft on the
/// input row under it, with its hint; with none the input row is blank.
#[test]
fn browse_shows_a_kept_draft_on_the_input_row() {
    let shot = Shot::new();
    for (draft, hint) in [
        ("a kept thought", "draft kept · Enter writes"),
        ("", "Enter writes"),
    ] {
        let (buf, layout) = shot.laid(100, 30, browsing(draft));
        let (ay, ax) = spot(&buf, "to api").expect("the address");
        assert_eq!(ay, 27, "{draft:?}");
        assert_eq!(row_from(&buf, ay, ax), "to api", "{draft:?}");
        assert_eq!(
            row_from(&buf, ay + 1, ax),
            draft,
            "{draft:?}: the input row"
        );
        let shown = row_from(&buf, ay + 2, ax);
        assert!(
            shown.starts_with(&format!("{hint} · ? keys")),
            "{draft:?}: the hint row: {shown:?}"
        );
        assert!(
            shown.ends_with(&format!("{} \u{2699}", crate::shape::version_line())),
            "{draft:?}: version and gear end it: {shown:?}"
        );
        assert_eq!(layout.cursor, None, "{draft:?}");
    }
}

/// A read-only, held or home-less composer keeps the block the others
/// do: its line on the address row, the input row blank under it.
#[test]
fn a_quiet_composer_keeps_the_block_and_leaves_the_input_row_blank() {
    let shot = Shot::new();
    for (composer, words) in [
        (READ_ONLY, "read-only · owned elsewhere"),
        (
            Composer::Held {
                why: "owned elsewhere",
            },
            "not writing: owned elsewhere · Esc browses",
        ),
        (
            Composer::NoHome,
            "read-only · no home session: ae app <session> picks one",
        ),
    ] {
        let (buf, _) = shot.laid(100, 30, composer);
        let (ay, ax) = spot(&buf, words).expect("the quiet line");
        assert_eq!(ay, 27, "{words}");
        assert_eq!(row_from(&buf, ay, ax), words, "{words}");
        assert_eq!(row_from(&buf, ay + 1, ax), "", "{words}: the input row");
        let shown = row_from(&buf, ay + 2, ax);
        assert_eq!(
            shown.starts_with("? keys"),
            !words.starts_with("not writing"),
            "{words}: the hint row: {shown:?}"
        );
        assert!(
            shown.ends_with(&format!("{} \u{2699}", crate::shape::version_line())),
            "{words}: the tail stands: {shown:?}"
        );
    }
}

/// A pending line takes the hint row, in full ink; writing otherwise
/// shows its keys there.
#[test]
fn a_note_takes_the_hint_row_and_writing_shows_its_keys() {
    let mut shot = Shot::new();
    let draft = drafted(&["hello"]);
    let (buf, _) = shot.laid(100, 30, writing(&draft, "hello"));
    let (ay, ax) = spot(&buf, "to api").expect("the address");
    assert_eq!(ay, 27);
    assert!(
        row_from(&buf, 29, ax).starts_with("Enter sends"),
        "the writing keys on the hint row"
    );
    shot.model.set_note(Some("q again to quit".to_owned()));
    let (buf, _) = shot.laid(100, 30, writing(&draft, "hello"));
    assert_eq!(spot(&buf, "q again to quit"), Some((29, ax)));
}

/// A note that fills the hint row of the widest pane leaves no room for
/// the keys after it: the row draws without them, and without wrapping
/// the column arithmetic past the u16 edge.
#[test]
fn a_full_hint_row_at_the_widest_pane_drops_the_keys() {
    let mut shot = Shot::new();
    shot.model.set_note(Some("n".repeat(70_000)));
    let (buf, layout) = shot.laid(u16::MAX, 28, READ_ONLY);
    assert_eq!(buf[(60_000, 27)].symbol(), "n", "the note fills the row");
    assert_eq!(
        buf[(65_532, 27)].symbol(),
        "n",
        "the note's last cell stands"
    );
    assert_eq!(spot(&buf, "? keys"), None, "no room for the keys");
    assert_eq!(
        spot(&buf, &crate::shape::version_line()),
        None,
        "no room for the version either"
    );
    let click = Mouse {
        kind: MouseKind::Click,
        column: u16::MAX - 1,
        row: 27,
    };
    assert!(
        !matches!(layout.hit(click), Some(super::Hit::Settings)),
        "no gear, no settings target"
    );
}

/// A click anywhere from the address row to the note row opens the
/// composer; the rule above and the blank row below are not it.
#[test]
fn the_compose_target_spans_the_address_the_draft_rows_and_the_note_row() {
    let shot = Shot::new();
    let draft = drafted(&["one", "two"]);
    let (buf, layout) = shot.laid(100, 30, writing(&draft, "one two"));
    let (ay, ax) = spot(&buf, "to api").expect("the address");
    let compose = |row| {
        let click = Mouse {
            kind: MouseKind::Click,
            column: column(ax),
            row,
        };
        matches!(layout.hit(click), Some(super::Hit::Compose))
    };
    assert!(!compose(ay - 1), "the rule");
    for row in ay..ay + 4 {
        assert!(compose(row), "row {row}: address, two draft rows, note");
    }
    assert!(!compose(ay + 4), "the blank row");
}

/// At the 40x8 floor the lane keeps one row: both rules are drawn,
/// every chat row is whole, and a scrolled lane draws no newer-turns
/// hint over the header.
#[test]
fn the_floor_size_keeps_every_chat_row_whole() {
    let mut shot = Shot::new();
    shot.lane = turns(150);
    shot.model.wheel(true, 10, 100);
    let (buf, _) = shot.laid(40, 8, browsing(""));
    assert_eq!(spot(&buf, "to api"), Some((5, 2)));
    assert_eq!(spot(&buf, "Enter writes"), Some((7, 2)));
    // The sidebar warning rides the header rule's left; the rule stands
    // past it, and the composer's own rule stands over the address.
    assert_eq!(buf[(35, 1)].symbol(), "─", "the header rule");
    assert_eq!(buf[(2, 4)].symbol(), "─", "the composer's rule");
    assert!(line(&buf, 0).concat().contains("api"), "the header");
    // One turn starts below the one drawn row; the 20-cell words are
    // centered in the 36-cell column that starts at 2: `newer` at 2 + 8 + 4.
    assert_eq!(
        spot(&buf, "newer turn"),
        Some((3, 14)),
        "the hint under the lane's one row, never over the header"
    );
    assert!(
        row_from(&buf, 7, 0)
            .trim_start()
            .trim_start_matches('\u{25c6}')
            .trim_start()
            .starts_with("Enter writes · ? keys"),
        "the hint row"
    );
}

/// A tall draft leaves the lane its floor: at 60x20 the composer grows
/// to ten draft rows and an eight-row draft leaves the lane r2-r7, and
/// no height from 13 up lets the biggest draft take the lane under it.
#[test]
fn a_tall_draft_leaves_the_lane_its_floor() {
    let mut shot = Shot::new();
    shot.lane = turns(40);
    let area = Rect::new(0, 0, 60, 20);
    assert_eq!(
        super::composer_pane(area),
        11,
        "ten draft rows and one kept back"
    );
    let draft = drafted(&["row"; 8]);
    let (buf, layout) = shot.laid(60, 20, writing(&draft, "row"));
    let (ay, ax) = spot(&buf, "to api").expect("the address");
    assert_eq!(ay, 10);
    assert_eq!(buf[(column(ax), 9)].symbol(), "─", "the rule");
    assert_eq!(layout.page_rows, 6);
    assert_eq!(
        spot(&buf, "turn 39"),
        Some((7, ax + 2)),
        "the lane's last row"
    );
    assert_eq!(row_from(&buf, 8, ax), "", "the blank row over the rule");
    for height in 13..=60 {
        let rows = super::composer_pane(Rect::new(0, 0, 60, height)) - 1;
        let draft = drafted(&["x"].repeat(rows));
        let (_, layout) = shot.laid(60, height, writing(&draft, "x"));
        assert!(
            layout.page_rows >= usize::from(super::LANE_FLOOR),
            "{height}: {} lane rows",
            layout.page_rows
        );
    }
}

/// The address costs the draft no cell: a name that fills the column is
/// clipped on its own row, the draft keeps every cell of the one under
/// it, and the cursor stops on the last of them.
#[test]
fn a_long_address_is_clipped_on_its_row_and_the_draft_keeps_the_column() {
    let shot = Shot::new();
    let long = "h".repeat(60);
    let area = Rect::new(0, 0, 60, 20);
    let full = "x".repeat(super::draft_width(area, super::Split::default()));
    let draft = drafted(&[full.as_str()]);
    for home in ["api", long.as_str()] {
        let composer = Composer::Home {
            home,
            view: Some(&draft),
            draft: &full,
        };
        let (buf, layout) = shot.laid(60, 20, composer);
        let (ay, ax) =
            spot(&buf, &format!("to {}", &home[..home.len().min(8)])).expect("the address");
        assert_eq!(ay, 17, "{home}");
        assert_eq!(row_from(&buf, ay + 1, ax), full, "{home}: every cell");
        assert_eq!(
            layout.cursor,
            Some(Position::new(column(ax + full.len() - 1), ay + 1)),
            "{home}: the last cell"
        );
    }
}

/// The cells a draft has are the column's: from its left edge to the
/// rule's end, with nothing taken for the address (frames calm, needs-you).
#[test]
fn a_draft_has_the_columns_cells_from_its_left_edge_to_its_rule_end() {
    for (width, height) in [(160, 43), (100, 28)] {
        let buf = Shot::new().draw(width, height, READ_ONLY);
        let end = rule_end(&buf).expect("the rule");
        let (_, left) = spot(&buf, "read-only").expect("the quiet line");
        let area = Rect::new(0, 0, width, height);
        let cells = super::draft_width(area, super::Split::default());
        assert_eq!(left + cells - 1, usize::from(end), "{width}x{height}");
    }
}

/// F8 (re-amended): the branch is drawn at >=140 columns, sidebar or not;
/// below 140 it is dropped (100x30 frame: `active 1m` alone). Short panes
/// keep no sidebar, so these are the chat alone.
#[test]
fn the_branch_is_drawn_at_140_columns_with_or_without_a_sidebar() {
    let mut shot = Shot::new();
    shot.entry.branch = Some("feat/auth-v2".to_owned());
    shot.entry.last_active_epoch = Some(PIN_NOW - 60);
    for (width, shown) in [(160, true), (140, true), (139, false)] {
        let header = line(&shot.draw(width, 18, READ_ONLY), 0).concat();
        let header = header.trim_end();
        assert!(
            header.ends_with("active 1m  X zoom"),
            "{width}x18: {header:?}"
        );
        assert_eq!(
            header.contains("feat/auth-v2  ·  "),
            shown,
            "{width}x18: {header:?}"
        );
    }
}

/// Frame needs-you 100x30: four sessions sit one row apart, each a single
/// line, measured from the first.
#[test]
fn four_sessions_keep_one_row_each_at_100x28() {
    let mut shot = Shot::new();
    shot.fleet.rows = (1..=4)
        .map(|at| row(&format!("s{at}"), at, false))
        .collect();
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(100, 28, READ_ONLY);
    let ys: Vec<u16> = (1..=4)
        .map(|at| spot(&buf, &format!(" s{at} ")).expect("a session row").0)
        .collect();
    assert_eq!(ys, [ys[0], ys[0] + 1, ys[0] + 2, ys[0] + 3], "{ys:?}");
}

/// The window a long fleet shows. A 100x28 pane's list is 12 rows (two
/// thirds of the rows it shares with the tab body, never fewer than 11),
/// one per session with the more row inside it, so sixteen sessions show
/// 11 and `↓ 5 more`; at 160x43 it is 22, so thirty show 21 and `↓ 9 more`.
#[test]
fn a_long_fleet_shows_the_window_the_frames_do() {
    for (width, height, count, shown, more) in
        [(100, 28, 16, 11, "↓ 5 more"), (160, 43, 30, 21, "↓ 9 more")]
    {
        let mut shot = Shot::new();
        shot.fleet.rows = (1..=count)
            .map(|at| row(&format!("s{at}"), at, false))
            .collect();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(width, height, READ_ONLY);
        let size = format!("{width}x{height}");
        for at in 1..=count {
            let drawn = spot(&buf, &format!(" s{at} ")).is_some();
            assert_eq!(drawn, at <= shown, "{size}: s{at}");
        }
        let (y, _) = spot(&buf, more).expect("the more row");
        let rule = if width >= 140 { 44 } else { 34 };
        let drawn = line(&buf, y)[..rule].concat();
        assert_eq!(
            drawn.trim(),
            more,
            "{size}: no needy row hides, so no count"
        );
        assert!(width < 160 || y <= 25, "{size}: the more row at {y}");
        let last = spot(&buf, &format!(" s{shown} "))
            .expect("the last shown")
            .0;
        assert_eq!(
            y,
            last + 1,
            "{size}: the more row follows the last session's row"
        );
        let tabs = (2..buf.area.height)
            .find(|y| line(&buf, *y).concat().contains("Agents"))
            .expect("the heading");
        assert_eq!(tabs, y + 2, "{size}: one blank row, then the heading");
    }
    let mut empty = Shot::new();
    empty.fleet.rows.clear();
    empty.model = Model::new(&empty.fleet);
    let buf = empty.draw(160, 43, READ_ONLY);
    let none = spot(&buf, "No sessions.").expect("the empty list").0;
    assert_eq!(
        (2..buf.area.height).find(|y| line(&buf, *y).concat().contains("Agents")),
        Some(none + 2),
        "one blank row, then the heading"
    );
    // The more row counts the needy rows it hides, above and below.
    for (presses, more) in [
        (0, "↓ 5 more, ⚠ 2"),
        (15, "↑ 5 more, ⚠ 1"),
        (12, "↑ 2 · ↓ 3 more, ⚠ 3"),
    ] {
        let mut shot = Shot::new();
        shot.fleet.rows = (1..=16)
            .map(|at| row(&format!("s{at}"), at, matches!(at, 2 | 14 | 15)))
            .collect();
        shot.fleet.home = Some("s1".to_owned());
        shot.model = Model::new(&shot.fleet);
        for _ in 0..presses {
            shot.key(crate::app::model::Key::Down);
        }
        let buf = shot.draw(100, 28, READ_ONLY);
        let (y, _) = spot(&buf, "more").expect("the more row");
        let drawn = line(&buf, y)[..34].concat();
        assert_eq!(drawn.trim(), more, "{presses} presses down");
    }
}

/// Frame many's nine sessions: infra, billing and ops need you, ops is down.
fn frame_fleet() -> Fleet {
    let names = [
        "api", "docs", "infra", "research", "web", "billing", "mobile", "ops", "design",
    ];
    let rows = names
        .iter()
        .enumerate()
        .map(|(at, name)| {
            let mut row = row(name, at + 1, matches!(*name, "infra" | "billing" | "ops"));
            if *name == "ops" {
                row.mark = Mark::Dead;
            }
            row
        })
        .collect();
    Fleet { rows, home: None }
}

/// Several needy rows use a count and plain location suffixes; the
/// arrows stand alone at the sidebar's end, with no `!` key marker.
#[test]
fn the_attention_row_keeps_the_frames_forms() {
    let names = [
        "api", "docs", "infra", "research", "web", "billing", "mobile", "ops", "design",
    ];
    let mut shot = Shot::new();
    shot.fleet.rows = names
        .iter()
        .enumerate()
        .map(|(at, name)| {
            let mut row = row(name, at + 1, matches!(*name, "infra" | "billing" | "ops"));
            if *name == "ops" {
                row.mark = Mark::Dead;
            }
            row
        })
        .collect();
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(160, 43, READ_ONLY);
    let attention = line(&buf, 0).concat();
    assert!(
        !attention.contains("infra"),
        "no attention prose: {attention:?}"
    );
    let mut shot = Shot::new();
    shot.fleet.rows = vec![
        row("a", 1, false),
        row("needy-0010", 2, true),
        row("needy-00011", 3, true),
    ];
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(160, 43, READ_ONLY);
    assert_eq!(buf[(41, 0)].symbol(), " ", "no key marker at end-1");
    // The short rung with its marks spans cols 2-30, so the count ends the line.
    assert_eq!(
        buf[(30, 0)].symbol(),
        "2",
        "the count ends the line: {:?}",
        line(&buf, 0).concat()
    );
    // (c) the short rung of three needy sessions ends at col 31 and leaves the end cells blank.
    let mut shot = Shot::new();
    shot.fleet.rows = vec![
        row("a", 1, false),
        row("needy-000012", 2, true),
        row("needy-000013", 3, true),
        row("needy-000014", 4, true),
    ];
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(160, 43, READ_ONLY);
    assert_eq!(
        buf[(31, 0)].symbol(),
        "3",
        "the count ends the line: {:?}",
        line(&buf, 0).concat()
    );
    assert_eq!(buf[(32, 0)].symbol(), " ", "the end cells stay blank");
    // (d) only needy rows outside the view contribute to the suffix.
    let mut shot = Shot::new();
    shot.fleet.rows = (1..=30)
        .map(|at| {
            let name = match at {
                3 => "infra".to_owned(),
                29 => "billing".to_owned(),
                30 => "ops".to_owned(),
                _ => format!("s{at}"),
            };
            let mut row = row(&name, at, matches!(at, 3 | 29 | 30));
            if at == 30 {
                row.mark = Mark::Dead;
            }
            row
        })
        .collect();
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(160, 43, READ_ONLY);
    assert_eq!(buf[(41, 0)].symbol(), "↓", "off-screen attention below");
    assert_eq!(buf[(42, 0)].symbol(), " ", "no key marker");
}

/// Frame agents r20-r35: a seat's state is right-aligned at the sidebar's
/// end with a blank before it, never against the name. The state word is
/// the last thing to go: a name too long for the room is cut with an
/// ellipsis instead. At 160 the end is col 42: `● ` plus a 30-cell name
/// ends at col 33, `working` takes cols 35-41.
#[test]
fn an_agent_state_keeps_the_right_edge_and_the_name_gives_way() {
    for (cells, cut) in [(30, false), (31, true), (60, true)] {
        let mut shot = Shot::new();
        let Facts::Seats { id, agents } = sweep_seats() else {
            panic!("the sweep's seats");
        };
        let mut seat = agents.into_iter().next().expect("a seat");
        seat.name = "n".repeat(cells);
        seat.state = "working".to_owned();
        shot.agents = Some(Facts::Seats {
            id,
            agents: vec![seat],
        });
        let buf = shot.draw(160, 43, READ_ONLY);
        let (y, _) = spot(&buf, "nnnn").expect("the seat row");
        let row = line(&buf, y)[..44].concat();
        assert!(row.trim_end().ends_with(" working"), "{row:?}");
        assert_eq!(row.contains('…'), cut, "a {cells}-cell name: {row:?}");
        let name = row.find('n').expect("the name");
        let state = row.find("working").expect("the state");
        assert!(name < state, "{row:?}");
    }
}

/// Frame many 160: a session that needs you keeps its ⚠ mark and draws no
/// left edge and no question line. Column 0 holds the selection's `▸` and
/// nothing else, on the selected session's one row. The summary row
/// and its spacer put infra at r4, billing at r7 and ops at r9.
#[test]
fn a_needy_session_draws_no_bar_and_keeps_its_mark_without_a_question_row() {
    let mut shot = Shot::new();
    shot.fleet = frame_fleet();
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(160, 43, READ_ONLY);
    let col0: Vec<String> = (0..43).map(|y| buf[(0, y)].symbol().to_owned()).collect();
    assert!(
        !col0.iter().any(|symbol| symbol == "│"),
        "no edge: {col0:?}"
    );
    let selected = shot.model.selected().expect("a selection");
    let at = shot
        .fleet
        .rows
        .iter()
        .position(|row| row.name == selected)
        .expect("the selected row");
    let drawn: Vec<usize> = (0..43)
        .filter(|y| buf[(0, *y)].symbol() == "▸")
        .map(usize::from)
        .collect();
    assert_eq!(drawn, [2 + at], "the selection's glyph, one row");
    for (name, y) in [("infra", 4), ("billing", 7), ("ops", 9)] {
        let first = line(&buf, y)[..44].concat();
        assert!(first.contains(name), "{name}: {first:?}");
    }
    let infra = line(&buf, 4)[..44].concat();
    assert!(infra.contains('⚠'), "its mark: {infra:?}");
    for y in 0..43 {
        let sidebar = line(&buf, y)[..44].concat();
        assert!(
            !sidebar.contains("a question"),
            "no question row: {sidebar:?}"
        );
    }
}

/// The selected session wears what the viewed seat wears: the selected
/// ground over its one row out to the body's end, the glyph in the
/// selected ink, in every palette; the next session keeps the sidebar's
/// ground and a needy one is no different from a quiet one. On the
/// ground only the selected ink and the marks' accents are drawn, so the
/// pairs are the ones `theme`'s contrast bar already measures.
#[test]
fn the_selected_session_sits_on_the_selected_ground_in_every_palette() {
    let mut shot = sessions(3);
    shot.fleet.rows[1].needy = true;
    shot.fleet.rows[1].mark = Mark::NeedsYou;
    shot.model = Model::new(&shot.fleet);
    for name in ["darcula", "a", "b"] {
        let look = Look::read("on", name, "on", "on");
        let (buf, _) = framed(&shot, 160, Some(&look));
        let palette = look.palette;
        let rule = (0..160)
            .find(|x| buf[(*x, 0)].symbol() == "│")
            .expect("the rule");
        let (ground, ink) = (
            super::colour(palette.selected),
            super::colour(palette.selected_ink),
        );
        let glyph = &buf[(0, 2)];
        assert_eq!(glyph.symbol(), "▸", "{name}");
        assert_eq!(
            (glyph.fg, glyph.modifier.contains(Modifier::BOLD)),
            (ink, true)
        );
        for x in 0..rule - 2 {
            assert_eq!(buf[(x, 2)].bg, ground, "{name} ({x},2)");
        }
        for x in 0..rule - 2 {
            let cell = &buf[(x, 2)];
            if cell.symbol() == " " {
                continue;
            }
            let accents = Mark::BY_URGENCY.map(|mark| super::colour(palette.accent(mark)));
            let ok = cell.fg == ink || accents.contains(&cell.fg);
            assert!(ok, "{name}: {:?} {:?}", cell.symbol(), cell.fg);
        }
        for y in [3, 4, 5, 6, 7, 8] {
            for x in 0..rule - 2 {
                assert_ne!(buf[(x, y)].bg, ground, "{name}: row {y} col {x}");
            }
        }
    }
}

/// No colour: the selected session is reversed over its one row, as a
/// seat is, and no other session row is.
#[test]
fn the_selected_session_is_reversed_with_no_colour() {
    let shot = sessions(3);
    let (buf, _) = framed(&shot, 160, None);
    let reversed = |y| buf[(8, y)].modifier.contains(Modifier::REVERSED);
    assert!(reversed(2), "the selected session");
    assert!(!reversed(3) && !reversed(4), "the next session");
}

/// A long name is cut with an ellipsis beside the shortest tally, so the
/// row stays one line inside the rule.
#[test]
fn a_long_session_name_is_cut_beside_its_shortest_tally() {
    let mut shot = Shot::new();
    shot.fleet.rows = vec![row(&"n".repeat(60), 1, false)];
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(160, 43, READ_ONLY);
    let expected = "n".repeat(32) + "… ●3 …";
    assert_eq!(line(&buf, 2)[4..42].concat(), expected);
    assert!(!line(&buf, 3)[..42].concat().contains("✓B12"), "one row");
    assert_eq!(buf[(44, 2)].symbol(), "│");
}

/// The home glyph follows the name after one blank, on that row alone; a
/// 28-cell name keeps it beside the tally, a 29-cell one drops it first.
#[test]
fn this_window_marks_the_home_row_alone_inside_the_name_line() {
    let mut shot = Shot::new();
    shot.fleet = frame_fleet();
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(160, 43, READ_ONLY);
    let sidebar = |buf: &Buffer, y: u16| line(buf, y)[..44].concat();
    let tagged: Vec<u16> = (0..43)
        .filter(|y| sidebar(&buf, *y).contains('◂'))
        .collect();
    assert_eq!(tagged, [2], "the home row alone");
    assert!(
        sidebar(&buf, 2).starts_with("▸ ● api ◂"),
        "{:?}",
        sidebar(&buf, 2)
    );
    assert!(
        !sidebar(&buf, 2).contains("this window"),
        "a glyph, no words"
    );
    for (cells, shown) in [(28, true), (29, false)] {
        let mut shot = Shot::new();
        shot.fleet.rows = vec![row(&"n".repeat(cells), 1, false)];
        shot.model = Model::new(&shot.fleet);
        let first = sidebar(&shot.draw(160, 43, READ_ONLY), 2);
        assert_eq!(first.contains('◂'), shown, "a {cells}-cell name: {first:?}");
        assert!(first.contains("✓B12"), "the tally stays: {first:?}");
    }
}

/// A crowded status draws no second line: the questions a session holds
/// are told in the chat header, never under its row.
#[test]
fn a_session_draws_no_question_line() {
    let mut shot = Shot::new();
    shot.fleet = frame_fleet();
    shot.model = Model::new(&shot.fleet);
    let buf = shot.draw(160, 43, READ_ONLY);
    for y in 2..11 {
        let cells = line(&buf, y);
        assert!(!cells[..44].concat().contains("a question"), "row {y}");
        assert_eq!(cells[42..44].concat(), "  ");
    }
}

// -----------------------------------------------------------------------
// dragged borders. Oracles: brief-appresize rulings 5/5a/5b (sidebar 30
// up to 40 chat columns; one session and its more row; three body rows,
// never short of the undragged rule) and the drawn frame.
// -----------------------------------------------------------------------

/// A shot of `count` sessions, home first.
fn sessions(count: usize) -> Shot {
    let mut shot = Shot::new();
    shot.fleet.rows = (0..count)
        .map(|at| row(&format!("s{at:02}"), at + 1, false))
        .collect();
    shot.fleet.home = shot.fleet.rows.first().map(|row| row.name.clone());
    shot.model = Model::new(&shot.fleet);
    shot.entry = SessionEntry::new("s00", Status::Running);
    shot
}

/// The column of the drawn vertical rule, on the top row.
fn vertical(buf: &Buffer) -> Option<u16> {
    (0..buf.area.width).find(|x| buf[(*x, 0)].symbol() == "│")
}

/// The row of the drawn horizontal rule under the tabs.
fn horizontal(buf: &Buffer) -> Option<u16> {
    (0..buf.area.height).find(|y| buf[(0, *y)].symbol() == "─")
}

/// Press `edge` where it is drawn, then move the pointer to `to` along
/// its axis; the next frame.
fn drag(shot: &mut Shot, buf: &Buffer, edge: Edge, to: u16) -> Buffer {
    let at = match edge {
        Edge::Sidebar => vertical(buf),
        Edge::List => horizontal(buf),
        // A shot draws no orchestrator panel.
        Edge::Panel => None,
    }
    .expect("the border is drawn");
    let drag = Drag { edge, from: at, at };
    shot.model.grab(drag);
    let mouse = match edge {
        Edge::Sidebar | Edge::Panel => Mouse {
            kind: MouseKind::Drag,
            column: to,
            row: 1,
        },
        Edge::List => Mouse {
            kind: MouseKind::Drag,
            column: 1,
            row: to,
        },
    };
    let size = dragged_to(
        drag,
        mouse,
        buf.area,
        shot.model.split(),
        shot.fleet.rows.len(),
    );
    let _ = shot.model.drag_to(size);
    let _ = shot.model.let_go();
    shot.draw(buf.area.width, buf.area.height, READ_ONLY)
}

/// The undragged list is two thirds of the rows it shares with the tab
/// body, and never fewer than the 18 (from 40 high) or 11 rows it once had.
#[test]
fn the_undragged_list_takes_two_thirds_and_never_fewer_than_it_had() {
    for (height, rows) in [
        (20, 10),
        (24, 11),
        (28, 12),
        (30, 13),
        (40, 20),
        (45, 23),
        (62, 34),
        (65535, 43683),
    ] {
        let area = Rect::new(0, 0, 160, height);
        assert_eq!(
            super::list_rows(area, crate::app::model::Split::default(), 99),
            (rows, false)
        );
    }
}

/// Ruling 5b with the summary spacer: a 100x20 pane of 13 sessions draws
/// its list rule on row 14 (nine shown, the more row, tabs on 13). A press there
/// moved nowhere keeps it; dragged up and back down past it, it stops on
/// 14, never 15.
#[test]
fn a_short_pane_keeps_its_undragged_list_rule_reachable() {
    let mut shot = sessions(13);
    let first = shot.draw(100, 20, READ_ONLY);
    assert_eq!(horizontal(&first), Some(14), "the undragged rule");
    let still = drag(&mut shot, &first, Edge::List, 14);
    assert_eq!(horizontal(&still), Some(14), "a press moved nowhere");
    let up = drag(&mut shot, &still, Edge::List, 9);
    assert_eq!(horizontal(&up), Some(9));
    let down = drag(&mut shot, &up, Edge::List, 16);
    assert_eq!(horizontal(&down), Some(14), "never past the undragged rule");
}

/// Ruling 5a: the list keeps one session's row — one alone, a row and the
/// more row among several — and its dragged-down stop keeps the row
/// budget it had, the body taking the rows the frame gave back.
#[test]
fn a_dragged_list_keeps_one_session_and_its_row_budget() {
    for (count, least) in [(1, 5), (2, 6), (13, 6)] {
        let mut shot = sessions(count);
        let first = shot.draw(160, 45, READ_ONLY);
        let top = drag(&mut shot, &first, Edge::List, 0);
        assert_eq!(horizontal(&top), Some(least), "{count} sessions");
        let bottom = drag(&mut shot, &top, Edge::List, 44);
        // Border 36: body rows 37 to 44, the pane's last.
        assert_eq!(horizontal(&bottom), Some(36), "{count} sessions");
    }
}

/// Ruling 5: the sidebar keeps 30 columns and the chat 40 right of the
/// rule; a chosen width comes back when the pane grows again.
#[test]
fn a_dragged_sidebar_keeps_thirty_columns_and_forty_for_the_chat() {
    let mut shot = sessions(1);
    let first = shot.draw(160, 45, READ_ONLY);
    let narrow = drag(&mut shot, &first, Edge::Sidebar, 0);
    assert_eq!(vertical(&narrow), Some(30));
    let wide = drag(&mut shot, &narrow, Edge::Sidebar, 159);
    assert_eq!(vertical(&wide), Some(119), "160 - 40 - the rule");
    assert_eq!(vertical(&shot.draw(90, 20, READ_ONLY)), Some(49));
    assert_eq!(vertical(&shot.draw(89, 20, READ_ONLY)), None);
    assert_eq!(vertical(&shot.draw(160, 45, READ_ONLY)), Some(119));
}

/// Every chosen size at every pane from the sidebar's minimum up draws
/// inside its areas: the rule where the clamps allow, the list rule above
/// the keys row, and no panic.
#[test]
fn every_dragged_size_draws_inside_the_pane() {
    for count in [0, 1, 2, 13] {
        for asked in [0, 9, 33, u16::MAX] {
            let mut shot = sessions(count);
            shot.model.grab(Drag {
                edge: Edge::Sidebar,
                from: 0,
                at: 0,
            });
            let _ = shot.model.drag_to(asked);
            shot.model.grab(Drag {
                edge: Edge::List,
                from: 0,
                at: 0,
            });
            let _ = shot.model.drag_to(asked);
            let _ = shot.model.let_go();
            for width in (90..=220).step_by(13) {
                for height in (20..=62).step_by(7) {
                    let buf = shot.draw(width, height, READ_ONLY);
                    let rule = vertical(&buf).expect("the sidebar's rule");
                    assert!((30..=width - 41).contains(&rule), "{width}x{height}");
                    if let Some(row) = horizontal(&buf) {
                        assert!(row < height - 1, "{width}x{height}");
                    }
                }
            }
        }
    }
}

/// Ruling 4: a press grabs a border on its cells or one beside them, never
/// where a click target is drawn; on a corner the vertical rule wins.
#[test]
fn a_press_grabs_a_border_only_where_no_target_is() {
    let shot = sessions(3);
    let screen = Screen {
        fleet: &shot.fleet,
        model: &shot.model,
        overview: &shot.overview,
        selected: Some(&shot.entry),
        pair: &shot.pair,
        agents: None,
        lane: &shot.lane,
        composer: READ_ONLY,
        look: None,
        zone: None,
        now: Timestamp::from_epoch(PIN_NOW),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    let layout = super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
    let rule = horizontal(&buf).expect("the list rule");
    let at = |column, row| {
        layout.grab(Mouse {
            kind: MouseKind::Click,
            column,
            row,
        })
    };
    for column in [43, 44, 45] {
        assert_eq!(at(column, 0), Some((Edge::Sidebar, 44)), "column {column}");
    }
    assert_eq!(at(46, 0), None);
    assert_eq!(at(42, 0), None);
    assert_eq!(
        at(43, 1),
        Some((Edge::Sidebar, 44)),
        "the blank spacer beside the rule is no target"
    );
    assert_eq!(at(42, 1), None, "the spacer away from the rule");
    assert_eq!(at(43, 2), None, "a session row is a target");
    assert_eq!(at(44, 2), Some((Edge::Sidebar, 44)));
    for row in [rule - 1, rule, rule + 1] {
        assert_eq!(at(30, row), Some((Edge::List, rule)), "row {row}");
    }
    assert_eq!(at(30, rule + 2), None);
    assert_eq!(
        at(2, rule - 1),
        Some((Edge::List, rule)),
        "the heading is no target"
    );
    assert_eq!(at(44, rule), Some((Edge::Sidebar, 44)), "the corner");
    assert_eq!(at(43, rule), Some((Edge::List, rule)), "its own cell");
    assert_eq!(
        at(44, 44),
        Some((Edge::Sidebar, 44)),
        "the rule runs to the hint row"
    );
    assert_eq!(at(159, 44), None, "the gear is a target, not a border");
}

/// Fixed text width for geometry-only fixtures. This deliberately unmapped
/// 13-cell word passes through unchanged; production vocabulary is pinned
/// independently by `honestwords_spec`. These fixtures keep their measured
/// label capacity, with the existing unknown-word Stale mark.
const FIT_STATE: &str = "unmapped-word";

/// Two seats in the Agents body, by name.
fn seat_facts(names: &[&str]) -> Facts {
    let Facts::Seats { id, agents } = sweep_seats() else {
        unreachable!("the sweep draws seats")
    };
    let seat_of = |name: &&str| PickerAgent {
        name: (*name).to_owned(),
        state: FIT_STATE.to_owned(),
        ..agents[0].clone()
    };
    Facts::Seats {
        id,
        agents: names.iter().map(seat_of).collect(),
    }
}

/// One frame of `shot` at `width` x 45 in `look`.
fn framed(shot: &Shot, width: u16, look: Option<&Look>) -> (Buffer, super::Layout) {
    framed_as(shot, width, look, READ_ONLY)
}

/// [`framed`] under `composer`.
fn framed_as(
    shot: &Shot,
    width: u16,
    look: Option<&Look>,
    composer: Composer<'_>,
) -> (Buffer, super::Layout) {
    let screen = Screen {
        fleet: &shot.fleet,
        model: &shot.model,
        overview: &shot.overview,
        selected: Some(&shot.entry),
        pair: &shot.pair,
        agents: shot.agents.as_ref(),
        lane: &shot.lane,
        composer,
        look: look.copied(),
        zone: None,
        now: Timestamp::from_epoch(PIN_NOW),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, width, 45));
    let layout = super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
    (buf, layout)
}

/// The seat rows of `buf` whose first cell is `glyph`: those under the
/// Agents rule, so the selected session's own `▸` above it is not one.
fn marked(buf: &Buffer, glyph: &str) -> Vec<u16> {
    let rule = horizontal(buf).expect("the Agents rule");
    (rule + 1..buf.area.height)
        .filter(|y| buf[(0, *y)].symbol() == glyph)
        .collect()
}

/// The seat the keys act on and the seat the chat column shows are marked
/// apart. Nothing is marked until the human steps with `n` / `p` (`▹`, no
/// ground) or views a seat (`▸` and the selected ground); the first seat
/// drawn is still the keys' target, a seat row is a click target that
/// ends at the body, and the keys row names no seat key.
#[test]
fn the_seat_cursor_and_the_viewed_seat_are_marked_apart() {
    let mut shot = Shot::new();
    shot.agents = Some(seat_facts(&["lead", "colead"]));
    let (buf, layout) = framed(&shot, 160, None);
    assert_eq!(layout.seats, ["lead", "colead"]);
    assert_eq!(layout.focused.as_deref(), Some("lead"), "the keys' target");
    assert!(marked(&buf, "▸").is_empty() && marked(&buf, "▹").is_empty());
    let rule = horizontal(&buf).expect("the Agents rule");
    let seat_row = |buf: &Buffer, name: &str| {
        (rule + 1..45)
            .find(|y| line(buf, *y).concat().contains(name))
            .expect(name)
    };
    let (first, second) = (seat_row(&buf, "lead"), seat_row(&buf, "colead"));
    assert_eq!(second, first + 1, "one row a seat");
    let keys: String = line(&buf, 44).concat();
    assert!(
        keys.contains("? keys") && !keys.contains("oo open seat") && !keys.contains("n/p"),
        "{keys}"
    );
    let at = Mouse {
        kind: MouseKind::Click,
        column: 8,
        row: second,
    };
    assert!(matches!(layout.hit(at), Some(super::Hit::Seat(name)) if name == "colead"));
    let rule_x = (0..160)
        .find(|x| buf[(*x, at.row)].symbol() == "│")
        .expect("the rule");
    let seat = |column| {
        matches!(
            layout.hit(Mouse { column, ..at }),
            Some(super::Hit::Seat(_))
        )
    };
    assert!(
        seat(rule_x - 3) && !seat(rule_x - 2),
        "a seat ends at the body"
    );
    // The cursor: a gutter glyph on the seat, no ground, no reverse.
    shot.model.set_focus(Some("colead".to_owned()));
    let (buf, layout) = framed(&shot, 160, None);
    assert_eq!(marked(&buf, "▹"), [second]);
    assert!(marked(&buf, "▸").is_empty(), "a cursor is not a view");
    assert!(!buf[(8, second)].modifier.contains(Modifier::REVERSED));
    assert_eq!(layout.focused.as_deref(), Some("colead"));
    shot.model.set_focus(Some("ghost".to_owned()));
    let (buf, layout) = framed(&shot, 160, None);
    assert_eq!(layout.focused.as_deref(), Some("lead"), "an undrawn name");
    assert!(marked(&buf, "▹").is_empty(), "an undrawn name draws none");
    shot.model.set_focus(Some("colead".to_owned()));
    let ascii = Look::read("off", "darcula", "on", "on");
    let (buf, _) = framed(&shot, 160, Some(&ascii));
    assert_eq!(marked(&buf, "-"), [second]);
    // The viewed seat is lifted, and the cursor yields to it.
    let viewing = Composer::Viewing { seat: "colead" };
    let (buf, _) = framed_as(&shot, 160, None, viewing);
    assert_eq!(marked(&buf, "▸"), [second]);
    assert!(marked(&buf, "▹").is_empty(), "the lift replaces the cursor");
    assert!(buf[(8, second)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(8, first)].modifier.contains(Modifier::REVERSED));
    let (buf, _) = framed_as(&shot, 160, Some(&ascii), viewing);
    assert_eq!(marked(&buf, ">"), [second]);
    // Scrolled so the cursor's seat is gone, no row carries the glyph and
    // the first drawn seat is the keys' target.
    let names: Vec<String> = (0..60).map(|n| format!("s{n:02}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    shot.agents = Some(seat_facts(&names));
    shot.model.set_focus(None);
    assert!(shot.model.wheel_body(false, 9));
    let (buf, layout) = framed(&shot, 160, None);
    assert_eq!(layout.focused.as_deref(), Some("s03"));
    assert!(marked(&buf, "▹").is_empty() && marked(&buf, "▸").is_empty());
}

/// The cursor stays where `n` / `p` left it when another seat is viewed;
/// the viewed seat is the one lifted row, and its session becomes the
/// context, titled but not lifted, instead of a second lifted row.
#[test]
fn the_cursor_stays_while_another_seat_is_viewed_and_the_session_is_a_context() {
    let mut shot = Shot::new();
    shot.agents = Some(seat_facts(&["lead", "colead"]));
    shot.model.set_focus(Some("lead".to_owned()));
    let viewing = Composer::Viewing { seat: "colead" };
    let (buf, _) = framed_as(&shot, 160, None, viewing);
    let rule = horizontal(&buf).expect("the Agents rule");
    let (first, second) = (rule + 1, rule + 2);
    assert_eq!(marked(&buf, "▸"), [second], "the viewed seat is lifted");
    assert_eq!(marked(&buf, "▹"), [first], "the cursor stays");
    assert_eq!(buf[(0, 2)].symbol(), " ", "the session is not lifted");
    let name = &buf[(4, 2)];
    assert!(
        name.modifier
            .contains(Modifier::BOLD | Modifier::UNDERLINED)
    );
    assert!(!name.modifier.contains(Modifier::REVERSED));
    // The shared chat shows the session: it is the lifted row.
    let (buf, _) = framed(&shot, 160, None);
    assert_eq!(buf[(0, 2)].symbol(), "▸");
    assert!(buf[(4, 2)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(4, 2)].modifier.contains(Modifier::UNDERLINED));
    assert_eq!(marked(&buf, "▹"), [first], "the cursor is no view");
}

/// The composer's address names the seat a plain line reaches now and how
/// it was picked; only the home composer says so.
#[test]
fn the_composer_address_names_the_seat_the_line_would_reach() {
    let shot = Shot::new();
    // The chat column below the header: the address and hint rows live here.
    let chat = |dest, composer| {
        let screen = shot.screen(composer);
        let wait = super::Wait {
            dest,
            ..super::Wait::default()
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
        let _ = super::draw_with_layout(&screen, wait, &mut buf);
        (1..45)
            .map(|y| line(&buf, y)[45..].concat())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let auto = Some(super::Dest {
        seat: "colead",
        named: false,
    });
    let named = Some(super::Dest {
        seat: "lead",
        named: true,
    });
    assert!(chat(auto, browsing("")).contains("to api · colead (auto)"));
    assert!(chat(named, browsing("")).contains("to api · lead (named)"));
    let none = chat(None, browsing(""));
    assert!(none.contains("to api") && !none.contains("to api ·") && !none.contains("(auto)"));
    let held = Composer::Held { why: "owned" };
    assert!(!chat(auto, held).contains("colead"), "not home: no seat");
    let viewing = Composer::Viewing { seat: "scout" };
    assert!(!chat(auto, viewing).contains("(auto)"));
    assert!(chat(auto, viewing).contains("viewing scout"));
    assert!(!chat(auto, READ_ONLY).contains("(auto)"));
}

/// The header names the open ask of the seat viewed, else of the seat
/// under the cursor, else the first that waits.
#[test]
fn the_header_names_the_viewed_seats_ask_then_the_cursors_then_the_first() {
    let mut shot = Shot::new();
    shot.agents = Some(seat_facts(&["lead", "colead"]));
    shot.overview.open = vec![
        Open {
            seat: "lead".to_owned(),
            text: "first ask".to_owned(),
            age_secs: Some(60),
        },
        Open {
            seat: "colead".to_owned(),
            text: "second ask".to_owned(),
            age_secs: Some(120),
        },
    ];
    let header = |shot: &Shot, composer| line(&framed_as(shot, 160, None, composer).0, 0).concat();
    assert!(header(&shot, READ_ONLY).contains("⚠ lead: first ask · 1m"));
    shot.model.set_focus(Some("colead".to_owned()));
    let at_cursor = header(&shot, READ_ONLY);
    assert!(
        at_cursor.contains("⚠ colead: second ask · 2m"),
        "{at_cursor}"
    );
    assert!(!at_cursor.contains("first ask"), "one ask at a time");
    shot.model.set_focus(Some("lead".to_owned()));
    let viewing = Composer::Viewing { seat: "colead" };
    assert!(header(&shot, viewing).contains("⚠ colead: second ask"));
    shot.model.set_focus(Some("scout".to_owned()));
    let quiet = Composer::Viewing { seat: "scout" };
    assert!(
        header(&shot, quiet).contains("⚠ lead: first ask"),
        "falls to the first"
    );
    let ascii = Look::read("off", "darcula", "on", "on");
    let (buf, _) = framed_as(&shot, 160, Some(&ascii), READ_ONLY);
    assert!(line(&buf, 0).concat().contains("! lead: first ask · 1m"));
}

/// The reason comes before the directory: both when the room allows, and
/// when it does not the directory goes whole rather than the reason; a
/// reason needs 12 cells to be drawn at all.
#[test]
fn the_header_reason_comes_before_the_directory() {
    let mut shot = Shot::new();
    shot.overview.dir = Some("~/projects/workspace/ae".to_owned());
    shot.overview.open = vec![Open {
        seat: "lead".to_owned(),
        text: "approve the plan".to_owned(),
        age_secs: Some(60),
    }];
    let wide = line(&framed(&shot, 160, None).0, 0).concat();
    let (reason, dir) = (wide.find('⚠'), wide.find("~/projects"));
    assert!(reason.is_some() && dir.is_some() && reason < dir, "{wide}");
    shot.overview.open[0].text = "approve the plan ".repeat(8);
    let narrow = line(&shot.draw(100, 30, READ_ONLY), 0).concat();
    assert!(
        narrow.contains('⚠') && narrow.contains("… · 1m"),
        "{narrow}"
    );
    assert!(!narrow.contains("~/"), "no directory fragment: {narrow}");
    let screen = shot.screen(READ_ONLY);
    let ctx = super::Ctx {
        screen: &screen,
        paint: Paint::of(screen.look.as_ref()),
        icons: true,
        wait: super::Wait::default(),
    };
    assert_eq!(
        super::reason_words(&ctx, 13),
        None,
        "under 12 beside the mark"
    );
    let words = super::reason_words(&ctx, 14).expect("12 cells and the mark");
    assert!(super::drawn_cols(&words) <= 14, "{words:?}");
}

/// A directory after the reason needs twelve cells: at 160 columns 39
/// middle cells leave exactly twelve beside the reason and the activity
/// word, 40 leave eleven and no part of the directory is drawn.
#[test]
fn the_header_draws_a_directory_after_the_reason_only_in_twelve_cells() {
    for (middle, shown) in [(39, true), (40, false)] {
        let mut shot = Shot::new();
        shot.entry.last_active_epoch = Some(PIN_NOW - 60);
        shot.overview.dir = Some("projects/xyz".to_owned());
        shot.overview.open = vec![Open {
            seat: "lead".to_owned(),
            text: "approve the plan".to_owned(),
            age_secs: Some(60),
        }];
        shot.pair = vec!["lead".to_owned(), "c".repeat(middle - 7)];
        let header = line(&shot.draw(160, 43, READ_ONLY), 0).concat();
        assert!(
            header.contains("approve the plan") && header.contains("active 1m"),
            "{middle}: the reason and the activity stay: {header:?}"
        );
        assert_eq!(header.contains("xyz"), shown, "{middle}: {header:?}");
    }
}

/// Approved legend teaches all seven task marks in each glyph vocabulary.
#[test]
fn honestwords_keys_tab_explains_every_mark_in_both_glyph_vocabularies() {
    for icons in [true, false] {
        let rows: Vec<String> = super::settings_body(
            super::SettingsTab::Keys,
            &super::SettingsBodies::default(),
            Paint::of(None),
            100,
            icons,
        )
        .into_iter()
        .map(|(text, _)| text)
        .collect();
        for (unicode, ascii, word) in [
            ("✖", "x", "dead"),
            ("⚠", "!", "needs you"),
            ("◌", "?", "unknown"),
            ("●", "*", "working"),
            ("◔", "~", "waiting on an agent"),
            ("✓", "+", "finished"),
            ("·", "-", "no current status"),
        ] {
            let glyph = if icons { unicode } else { ascii };
            assert!(
                rows.iter()
                    .any(|row| row.trim_start().starts_with(glyph) && row.contains(word)),
                "{glyph} {word}: {rows:?}"
            );
        }
        let text = rows.join("\n");
        assert!(
            text.contains("declared done"),
            "completion needs a declaration: {text}"
        );
        assert!(
            text.contains("silent past"),
            "unknown includes silence: {text}"
        );
        assert!(
            text.contains("no current done/waiting/blocked declaration"),
            "Idle ruling: {text}"
        );
        assert!(
            text.contains("Browse") && text.contains("Write") && text.contains("Mouse"),
            "existing key help survives"
        );
    }
}

/// Lead final-r1 ruling: a qualifier yields before identity; its head
/// remains visible at the 30-cell sidebar floor. Independent geometry:
/// sidebar minus four gives room; colead spends ten cells before its label.
#[test]
fn honestwords_app_qualifier_yields_before_identity_and_preserves_state_at_floor() {
    for icons in [true, false] {
        let look = Look::read(if icons { "on" } else { "off" }, "darcula", "on", "off");
        for (sidebar, model, effort, full, wrapped) in [
            (60, "gpt-6.1-sol", "xhigh", true, false),
            (44, "gpt-6.1-sol", "xhigh", false, false),
            (30, "gpt-6.1-sol", "xhigh", false, true),
            (30, "", "", false, false),
            (44, "m12345678901234567890123456789", "xhigh", true, true),
        ] {
            let mut shot = Shot::new();
            let mut facts = label_facts(&[("colead", model, effort, ""), ("next", "", "", "")]);
            let Facts::Seats { agents, .. } = &mut facts else {
                unreachable!("synthetic seat facts");
            };
            agents[0].state = "waiting-user".to_owned();
            agents[1].state = "working".to_owned();
            shot.agents = Some(facts);
            with_sidebar(&mut shot, sidebar);
            let (buf, layout) = framed(&shot, 160, Some(&look));
            let rule = horizontal(&buf).expect("the Agents rule");
            let row = |y| line(&buf, y)[..usize::from(sidebar)].concat();
            let first = row(rule + 1);
            assert!(
                first.contains("colead") && first.contains("needs you"),
                "{sidebar}: {first}"
            );
            assert!(
                first
                    .trim_start()
                    .starts_with(if icons { "⚠" } else { "!" }),
                "{first}"
            );
            assert_eq!(
                first.contains("(waiting on you)"),
                full,
                "{sidebar}: {first}"
            );
            let label = if wrapped {
                row(rule + 2)
            } else {
                first.clone()
            };
            if !model.is_empty() {
                assert!(
                    label.contains(model) && label.contains(effort),
                    "{sidebar}: {label}"
                );
                if wrapped {
                    assert!(!first.contains(model), "label belongs on row two: {first}");
                }
            }
            let next = row(rule + if wrapped { 3 } else { 2 });
            assert!(
                next.contains("next"),
                "seat row group stays bounded: {next}"
            );
            assert_eq!(layout.seats, ["colead", "next"]);
        }
    }
}

/// The Keys tab teaches the new glyphs and the positional digits.
#[test]
fn the_keys_tab_names_the_home_glyph_the_cursor_and_the_nth_session() {
    let rows: Vec<String> = keys_body(Paint::of(None))
        .into_iter()
        .map(|r| r.0)
        .collect();
    let row = |start: &str, words: &[&str]| {
        assert!(
            rows.iter()
                .any(|r| r.starts_with(start) && words.iter().all(|w| r.contains(w))),
            "{start}: {words:?} in {rows:?}"
        );
    };
    row("1-9", &["Nth session"]);
    row("n/p", &["cursor", "▹"]);
    row("Esc", &["cursor", "home"]);
    row("◂ or <", &["home session"]);
    assert!(!rows.iter().any(|r| r.contains("this window")));
}

/// At the narrowest sidebar (30 columns) a tall fleet still draws one
/// session a row, each ending inside the rule's two blank cells.
#[test]
fn a_floor_sidebar_keeps_each_session_on_one_row() {
    let mut shot = sessions(28);
    let first = shot.draw(100, 30, READ_ONLY);
    let buf = drag(&mut shot, &first, Edge::Sidebar, 30);
    assert_eq!(vertical(&buf), Some(30), "the floor width");
    assert!(
        line(&buf, 1)[..30].concat().trim().is_empty(),
        "summary spacer"
    );
    let ys: Vec<u16> = (2..30)
        .filter(|y| line(&buf, *y)[4..5].concat() == "s")
        .collect();
    assert!(ys.len() >= 5, "{ys:?}");
    assert_eq!(
        ys,
        (2..2 + u16::try_from(ys.len()).unwrap_or(0)).collect::<Vec<_>>()
    );
    for y in ys {
        let cells = line(&buf, y);
        assert_eq!(cells[28..30].concat(), "  ", "row {y}");
        assert!(
            cells[4..28].concat().trim_end().chars().count() >= 4,
            "row {y}"
        );
    }
}

/// R3: the viewed seat's row shares the selected ground out to the body's
/// end and the glyph is the selected ink, never the title or a state hue;
/// the next seat keeps the sidebar's ground. On the ground only the
/// selected ink and the marks' accents are drawn, so the pairs are the
/// ones `theme`'s contrast bar already measures.
#[test]
fn the_viewed_seat_sits_on_the_selected_ground_in_every_palette() {
    let mut shot = Shot::new();
    shot.agents = Some(seat_facts(&["lead", "colead"]));
    for name in ["darcula", "a", "b"] {
        let look = Look::read("on", name, "on", "on");
        let viewing = Composer::Viewing { seat: "lead" };
        let (buf, _) = framed_as(&shot, 160, Some(&look), viewing);
        let palette = look.palette;
        let first = *marked(&buf, "▸").first().expect("a marked row");
        let rule = (0..160)
            .find(|x| buf[(*x, first)].symbol() == "│")
            .expect("the rule");
        let ground = super::colour(palette.selected);
        let ink = super::colour(palette.selected_ink);
        let glyph = &buf[(0, first)];
        assert_eq!(
            (glyph.fg, glyph.modifier.contains(Modifier::BOLD)),
            (ink, true)
        );
        for x in 0..rule - 2 {
            assert_eq!(buf[(x, first)].bg, ground, "{name} ({x},{first})");
        }
        let mark = super::colour(palette.accent(Mark::Stale));
        let drawn: Vec<_> = (0..rule - 2)
            .map(|x| &buf[(x, first)])
            .filter(|cell| cell.symbol() != " ")
            .collect();
        assert!(!drawn.is_empty(), "{name} row {first}");
        for cell in drawn {
            let ok = cell.fg == ink || cell.fg == mark;
            assert!(ok, "{name}: {:?} {:?}", cell.symbol(), cell.fg);
        }
        let sidebar = super::colour(palette.base);
        assert_eq!(buf[(5, first + 1)].bg, sidebar, "{name}: the next seat");
    }
}

/// R1/R4: a seat's one row carries its model, the client when its frame
/// drew none (see the client-beside-the-model test for both), and the
/// effort, each only when known and never its profile name.
#[test]
fn the_agents_row_is_model_and_effort_never_the_profile() {
    let mut shot = Shot::new();
    let seat = |name: &str, model: &str, effort: &str| PickerAgent {
        name: name.to_owned(),
        profile: "sonnet55x".to_owned(),
        state: "working".to_owned(),
        pane: "%7".to_owned(),
        client: "claude".to_owned(),
        model: model.to_owned(),
        effort: effort.to_owned(),
        drift: false,
        task: false,
    };
    shot.agents = Some(Facts::Seats {
        id: "$1".to_owned(),
        agents: vec![
            PickerAgent {
                client: String::new(),
                ..seat("seat-a", "Sonnet 5.5", "xhigh")
            },
            seat("seat-b", "", ""),
            seat("seat-c", "", "max"),
            PickerAgent {
                client: String::new(),
                ..seat("seat-d", "", "")
            },
        ],
    });
    let (buf, _) = framed(&shot, 160, None);
    let between = |name: &str| {
        let y = spot(&buf, name).expect(name).0;
        let row = line(&buf, y)[..44].concat();
        let (from, to) = (
            row.find(name).expect(name) + name.len(),
            row.find("working"),
        );
        row[from..to.expect("its state")].trim().to_owned()
    };
    assert_eq!(between("seat-a"), "Sonnet 5.5 xhigh");
    assert_eq!(between("seat-b"), "claude", "no placeholder");
    assert_eq!(between("seat-c"), "claude max");
    assert_eq!(
        between("seat-d"),
        "",
        "the row stays: the geometry is stable"
    );
    let frame: String = (0..45).map(|y| line(&buf, y).concat()).collect();
    assert!(!frame.contains("sonnet55x"), "the profile is not drawn");
}

/// #286: two seats on one model under different `[clients]` labels read
/// apart — each row names its client label beside its model (and effort),
/// a client the fact does not carry is left out, never invented from the
/// profile, and a seat with no model keeps its client alone.
#[test]
fn the_agents_row_names_the_client_beside_the_model() {
    let mut shot = Shot::new();
    let seat = |name: &str, client: &str, model: &str, effort: &str| PickerAgent {
        name: name.to_owned(),
        profile: "sonnet55x".to_owned(),
        state: "working".to_owned(),
        pane: "%7".to_owned(),
        client: client.to_owned(),
        model: model.to_owned(),
        effort: effort.to_owned(),
        drift: false,
        task: false,
    };
    shot.agents = Some(Facts::Seats {
        id: "$1".to_owned(),
        agents: vec![
            seat("seat-a", "claude", "Sonnet 5.5", "xhigh"),
            seat("seat-b", "claude-a", "Sonnet 5.5", "xhigh"),
            seat("seat-c", "", "Sonnet 5.5", ""),
            seat("seat-d", "codex", "", "max"),
        ],
    });
    with_sidebar(&mut shot, 60);
    let (buf, _) = framed(&shot, 160, None);
    let between = |name: &str| {
        let y = spot(&buf, name).expect(name).0;
        let row = line(&buf, y)[..60].concat();
        let (from, to) = (
            row.find(name).expect(name) + name.len(),
            row.find("working"),
        );
        row[from..to.expect("its state")].trim().to_owned()
    };
    assert_eq!(between("seat-a"), "claude Sonnet 5.5 xhigh");
    assert_eq!(between("seat-b"), "claude-a Sonnet 5.5 xhigh");
    assert_eq!(between("seat-c"), "Sonnet 5.5", "no client is invented");
    assert_eq!(between("seat-d"), "codex max");
    let frame: String = (0..45).map(|y| line(&buf, y).concat()).collect();
    assert!(!frame.contains("sonnet55x"), "the profile is not drawn");
}

/// #286: the client-and-model label degrades by the same cut rules as the
/// model alone did. Past the row it wraps whole under the name; past that
/// the effort goes first and the label is cut on `…` at its tail, so the
/// client — what tells two seats on one model apart — is the last to go.
#[test]
fn the_client_and_model_label_keeps_todays_cut_rules() {
    let mut shot = Shot::new();
    // 25 cells > free 20, but within the second row's 38.
    shot.agents = Some(label_facts(&[
        ("aa", "Sonnet 5.5", "xhigh", "claude-a"),
        ("bb", "", "", ""),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        !line(&buf, rule + 1).concat().contains("claude-a"),
        "no label on row one"
    );
    assert!(
        line(&buf, rule + 2)
            .concat()
            .contains("claude-a Sonnet 5.5 xhigh"),
        "the whole label on row two"
    );
    // 8 + 1 + 40 > 38: the effort is left off and the tail is cut.
    let model = "m".repeat(40);
    let mut shot = Shot::new();
    shot.agents = Some(label_facts(&[
        ("aa", &model, "xhigh", "claude-a"),
        ("bb", "", "", ""),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    let tail = line(&buf, rule + 2).concat();
    assert!(
        tail.contains(&format!("claude-a {}…", "m".repeat(28))),
        "the client survives the cut: {tail}"
    );
    assert!(!tail.contains("xhigh"), "the effort goes first: {tail}");
}

/// R7: a seat that waits on the human is told in the chat header, never
/// under its row: its Agents row stays one line; an ask from a seat the
/// roster lacks is a row of its own after the roster, so none is lost;
/// and no Overview or Tab word is left in the frame.
#[test]
fn a_seat_waiting_on_the_human_is_told_in_the_header_not_under_its_row() {
    let mut shot = Shot::new();
    shot.agents = Some(seat_facts(&["seat-a", "seat-b"]));
    shot.overview.open = vec![
        Open {
            seat: "seat-b".to_owned(),
            text: "Approve X?".to_owned(),
            age_secs: Some(240),
        },
        Open {
            seat: "ghost".to_owned(),
            text: "registry down".to_owned(),
            age_secs: Some(3600),
        },
    ];
    let look = Look::read("on", "darcula", "on", "on");
    let (buf, layout) = framed(&shot, 160, Some(&look));
    let sidebar = |y| line(&buf, y)[..42].concat().trim().to_owned();
    let at = |name: &str| (2..45).find(|y| sidebar(*y).contains(name)).expect(name);
    assert_eq!(at("seat-b"), at("seat-a") + 1, "seat-a asks nothing");
    assert_eq!(layout.seats, ["seat-a", "seat-b", "ghost"]);
    assert_eq!(at("ghost"), at("seat-b") + 1, "after the roster, one row");
    let ghost = sidebar(at("ghost"));
    assert!(
        ghost.starts_with("⚠ ghost") && ghost.ends_with("needs you"),
        "{ghost}"
    );
    let side: String = (0..45).map(|y| line(&buf, y)[..42].concat()).collect();
    assert!(
        !side.contains("Approve X?") && !side.contains("registry down"),
        "no reason under a row: {side}"
    );
    let header = line(&buf, 0).concat();
    assert!(header.contains("⚠ seat-b: Approve X? · 4m"), "{header}");
    let viewing = Composer::Viewing { seat: "ghost" };
    let (buf, _) = framed_as(&shot, 160, Some(&look), viewing);
    let header = line(&buf, 0).concat();
    assert!(header.contains("⚠ ghost: registry down · 1h"), "{header}");
    let frame: String = (0..45).map(|y| line(&buf, y).concat()).collect();
    assert!(
        !frame.contains("Overview") && !frame.contains("Tab"),
        "{frame}"
    );
}

/// R4(d)/I2: each section of the Keys tab says what its own `^C` and its
/// mouse do, so the list is complete section by section.
#[test]
fn the_keys_tab_states_each_sections_ctrl_c_and_mouse() {
    let rows: Vec<String> = keys_body(Paint::of(None))
        .into_iter()
        .map(|r| r.0)
        .collect();
    let titles = ["Browse", "Write", "Held", "Settings", "Mouse"];
    let has = |title: &str, words: &[&str]| {
        let from = rows
            .iter()
            .position(|row| row.starts_with(title))
            .expect(title);
        let rest = &rows[from + 1..];
        let to = rest
            .iter()
            .position(|r| titles.iter().any(|t| r.starts_with(t)));
        let body = &rest[..to.unwrap_or(rest.len())];
        let found = body.iter().any(|row| words.iter().all(|w| row.contains(w)));
        assert!(found, "{title} has no row with {words:?}: {body:?}");
    };
    has("Browse", &["^C", "quit at once"]);
    has("Write", &["^C", "clear a draft", "empty", "quits at once"]);
    has("Write", &["^C", "after a clear", "arms"]);
    has("Write", &["^U", "whole draft"]);
    has("Write", &["^W", "word before"]);
    has("Write", &["Alt+Bksp", "punctuation", "Option as Meta"]);
    has("Write", &["^K", "end of the line"]);
    has("Write", &["^Y", "cleared draft", "once"]);
    has("Settings", &["gear", "Esc close", "closes"]);
    has("Settings", &["wheel", "3 rows"]);
    has("Mouse", &["click", "session", "tab"]);
    has("Mouse", &["wheel", "3 rows"]);
    has("Mouse", &["drag", "border"]);
}
// Independent S1 acceptance: frozen by appdeclutter-nav; install unchanged.
fn declutter_frame(shot: &Shot, width: u16, icons: bool) -> (Buffer, super::Layout) {
    declutter_frame_at(shot, width, 28, icons)
}

fn declutter_frame_at(
    shot: &Shot,
    width: u16,
    height: u16,
    icons: bool,
) -> (Buffer, super::Layout) {
    let mut screen = shot.screen(READ_ONLY);
    screen.look = Some(Look::read(
        if icons { "on" } else { "off" },
        "darcula",
        "on",
        "on",
    ));
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let layout = super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
    (buf, layout)
}

fn declutter_line(buf: &Buffer, layout: &super::Layout, y: u16) -> String {
    line(buf, y)[..usize::from(layout.list.width)].concat()
}

#[test]
fn declutter_counts_ride_the_name_row_hide_goals_and_keep_home_hits_and_cells() {
    for width in [90, 100, 160] {
        for icons in [false, true] {
            let mut shot = Shot::new();
            shot.fleet.rows[0].name = "界api".to_owned();
            shot.fleet.rows[0].line2 = Line2::Goal {
                text: "goal hidden in list".to_owned(),
                age_secs: Some(60),
            };
            shot.fleet.home = Some("界api".to_owned());
            shot.model = Model::new(&shot.fleet);
            let (buf, layout) = declutter_frame(&shot, width, icons);
            let name = declutter_line(&buf, &layout, 2);
            let counts = if icons { "●3 ✓B12" } else { "*3 +B12" };
            let home = if icons { "◂" } else { "<" };
            assert!(name.contains("界 api") && name.contains(home), "{name:?}");
            assert!(name.contains(counts), "{width}/{icons}: {name:?}");
            assert!(!name.contains("goal"), "{name:?}");
            let below = declutter_line(&buf, &layout, 3);
            assert!(
                !below.contains(counts) && !below.contains("goal"),
                "{below:?}"
            );
            for column in [0, super::NAME, layout.list.width - 1] {
                assert!(
                    matches!(layout.hit(Mouse { kind: MouseKind::Click, column, row: 2 }), Some(super::Hit::Session(name)) if name == "界api")
                );
            }
            assert!(
                !matches!(
                    layout.hit(Mouse {
                        kind: MouseKind::Click,
                        column: 0,
                        row: 3
                    }),
                    Some(super::Hit::Session(_))
                ),
                "a session is one row to click"
            );
            assert_eq!(buf[(layout.list.width, 2)].symbol(), "│");
            shot.fleet.rows[0].line2 = Line2::NoGoal;
            let (buf, layout) = declutter_frame(&shot, width, icons);
            let name = declutter_line(&buf, &layout, 2);
            assert!(name.contains(counts) && !name.contains("goal"));
            shot.fleet.rows[0].counts = Counts::Unknown;
            let (buf, layout) = declutter_frame(&shot, width, icons);
            assert!(declutter_line(&buf, &layout, 2).trim_end().ends_with(" ?"));
        }
    }
}

#[test]
fn declutter_header_sums_hidden_multiplicities_and_keeps_unknown_distinct_from_zero() {
    let mut shot = sessions(9);
    for row in &mut shot.fleet.rows {
        row.counts = Counts::Marks(Vec::new());
        row.line2 = Line2::NoGoal;
        row.needy = matches!(row.index, 3 | 8 | 9);
    }
    shot.fleet.rows[0].counts = Counts::Marks(vec![
        (Mark::Working, false, 7),
        (Mark::WaitingAgent, false, 2),
        (Mark::Done, true, 3),
    ]);
    shot.fleet.rows[6].counts = Counts::Unknown;
    shot.fleet.rows[7].counts = Counts::Marks(vec![(Mark::Working, false, 5)]);
    shot.fleet.rows[8].counts = Counts::Stopped;
    shot.fleet.rows[8].line2 = Line2::NotRunning {
        seats: 5,
        stopped_secs: None,
    };
    shot.model.grab(Drag {
        edge: Edge::List,
        from: 0,
        at: 0,
    });
    let _ = shot.model.drag_to(5);
    let _ = shot.model.let_go();
    for (icons, bare) in [
        (true, "◈ 17+? ● ◉ 12+? ⚠ 3"),
        (false, "@ 17+? * & 12+? ! 3"),
    ] {
        let (buf, layout) = declutter_frame(&shot, 100, icons);
        assert_eq!(
            declutter_line(&buf, &layout, 0)
                .trim()
                .strip_suffix('↓')
                .map(str::trim_end),
            Some(bare),
            "the short rung with its marks takes 34 cells, the line beside one arrow 28"
        );
        assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
        shot.key(crate::app::model::Key::NextNeed);
        assert_eq!(shot.model.selected(), Some("s02"));
        shot.key(crate::app::model::Key::NextNeed);
        let (buf, layout) = declutter_frame(&shot, 100, icons);
        assert_eq!(
            declutter_line(&buf, &layout, 0)
                .trim()
                .strip_suffix("↑↓")
                .map(str::trim_end),
            Some(bare)
        );
        shot.key(crate::app::model::Key::Esc);
    }
    shot.fleet.rows.truncate(1);
    shot.model = Model::new(&shot.fleet);
    for (counts, expected) in [
        (Counts::Marks(Vec::new()), "Agt 0 · * & Wrk 0 · ! Need 0"),
        (Counts::Unknown, "@ 0+? * & 0+? ! 0"),
    ] {
        shot.fleet.rows[0].counts = counts;
        let (buf, layout) = declutter_frame(&shot, 100, false);
        assert_eq!(declutter_line(&buf, &layout, 0).trim(), expected);
        assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
        if matches!(shot.fleet.rows[0].counts, Counts::Marks(_)) {
            assert!(declutter_line(&buf, &layout, 2).contains("no agents"));
        }
    }
}

#[test]
fn declutter_crowded_header_keeps_complete_session_and_attention_counts() {
    let mut shot = sessions(2);
    shot.fleet.rows[0].counts = Counts::Marks(vec![(Mark::Working, false, usize::MAX)]);
    shot.fleet.rows[1].counts = Counts::Unknown;
    shot.fleet.rows[1].needy = true;
    let (buf, layout) = declutter_frame(&shot, 90, false);
    assert_eq!(layout.list.width, 34);
    assert_eq!(
        declutter_line(&buf, &layout, 0).trim(),
        "! 1",
        "whole low-priority counts give way before a digit is cut"
    );
    assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
    shot.fleet.rows[0].counts = Counts::Marks(vec![
        (Mark::Working, false, 3),
        (Mark::Done, false, usize::MAX - 3),
    ]);
    let (buf, layout) = declutter_frame(&shot, 90, false);
    assert_eq!(declutter_line(&buf, &layout, 0).trim(), "! 1");
    shot = sessions(130);
    for row in &mut shot.fleet.rows {
        row.counts = Counts::Marks(Vec::new());
        row.needy = row.index <= 100;
    }
    shot.fleet.rows[0].counts =
        Counts::Marks(vec![(Mark::Working, false, 1000), (Mark::Done, false, 150)]);
    shot.fleet.rows[1].counts = Counts::Unknown;
    let (buf, layout) = declutter_frame(&shot, 90, false);
    assert_eq!(
        declutter_line(&buf, &layout, 0)
            .trim_end()
            .strip_suffix('↓')
            .map(str::trim),
        Some("@ 1150+? * & 1000+? ! 100"),
        "the right-aligned arrow reserves room in the one-line ladder"
    );
}

#[test]
fn declutter_crowded_counts_keep_the_urgent_ones_and_close_on_an_ellipsis() {
    let mut shot = Shot::new();
    shot.fleet.rows[0].counts = Counts::Marks(
        Mark::BY_URGENCY
            .into_iter()
            .map(|mark| (mark, false, 12))
            .collect(),
    );
    for status in [
        Line2::Question {
            text: "approve deploy now".to_owned(),
            age_secs: Some(60),
        },
        Line2::SeatDown,
    ] {
        shot.fleet.rows[0].line2 = status;
        let (buf, layout) = declutter_frame(&shot, 90, true);
        let name = declutter_line(&buf, &layout, 2);
        assert!(
            name.contains("✖12") && name.trim_end().ends_with('…'),
            "{name:?}"
        );
        assert!(
            !name.contains("✓12") && !name.contains("·12"),
            "low urgency counts yield: {name:?}"
        );
        assert!(
            !name.contains("approve") && !name.contains("down"),
            "no status words in a session row: {name:?}"
        );
    }
}

#[test]
fn declutter_reason_clips_cells_and_keeps_whole_graphemes_and_age() {
    for (text, room, expected) in [
        ("界".repeat(10), 20, "界".repeat(10)),
        ("界".repeat(11), 20, "界".repeat(7) + "… · 4m"),
        ("界".repeat(8), 22, "界".repeat(8) + " · 4m"),
        ("e\u{301}".repeat(23), 29, "e\u{301}".repeat(23) + " · 4m"),
        ("e\u{301}".repeat(30), 29, "e\u{301}".repeat(23) + "… · 4m"),
    ] {
        let words = super::aged(&text, Some(240), room);
        assert_eq!(words, expected, "{text}/{room}");
        assert!(super::drawn_cols(&words) <= room, "{text}/{room}");
    }
}

/// The help lists `v` among the browse keys and gives the peek panel its
/// own section, between Held and Settings, saying nothing reaches the pane.
#[test]
fn the_keys_tab_lists_v_and_the_peek_panel() {
    let rows: Vec<String> = keys_body(Paint::of(None))
        .into_iter()
        .map(|r| r.0)
        .collect();
    let at = |start: &str| {
        rows.iter()
            .position(|row| row.starts_with(start))
            .expect(start)
    };
    let (held, peek, settings) = (at("Held"), at("Peek"), at("Settings"));
    assert!(held < peek && peek < settings, "{rows:?}");
    let browse = &rows[at("Browse")..at("Write")];
    assert!(
        browse
            .iter()
            .any(|r| r.starts_with("v ") && r.contains("peek"))
    );
    let section = &rows[peek..settings];
    for words in [
        &["j/k", "scroll a line"][..],
        &["PgUp", "scroll a page"],
        &["Esc q v", "back to chat", "click the session"],
        &["wheel", "3 rows"],
        &["any other", "nothing reaches the pane"],
    ] {
        assert!(
            section.iter().any(|r| words.iter().all(|w| r.contains(w))),
            "{words:?}: {section:?}"
        );
    }
}

/// The help names the orchestrator panel's pane controls: `oo` while the
/// panel holds the keys, and the click on the header's `pane`.
#[test]
fn the_keys_tab_names_the_orchestrator_panels_pane_controls() {
    let rows: Vec<String> = keys_body(Paint::of(None))
        .into_iter()
        .map(|r| r.0)
        .collect();
    let row = |start: &str, words: &[&str]| {
        assert!(
            rows.iter()
                .any(|r| r.starts_with(start) && words.iter().all(|w| r.contains(w))),
            "{start}: {words:?} in {rows:?}"
        );
    };
    row("oo", &["open seat", "panel", "orchestrator pane"]);
    row("O F", &["orchestrator panel", "click pane"]);
    row("click mic", &["panel", "start or stop", "dictation"]);
    assert!(
        !rows.iter().any(|r| r.contains("not enabled")),
        "the activated mic says no inactive word: {rows:?}"
    );
}

/// With a seat viewed the keys row says `viewing` and `Esc chat`,
/// keeps the version and records no gear, which would be a dead click.
#[test]
fn the_peek_keys_row_hides_the_gear_and_keeps_the_version() {
    let mut shot = Shot::new();
    shot.model.open_peek(1);
    let (row, layout) = shot.footer_at(100, browsing(""), false);
    let tail = crate::shape::version_line();
    assert_eq!(
        row.strip_suffix(tail.as_str())
            .map(|head| head.trim_end().to_owned()),
        Some("  viewing   Esc chat".to_owned()),
        "{row:?}"
    );
    let last = Mouse {
        kind: MouseKind::Click,
        column: 99,
        row: 0,
    };
    assert!(layout.hit(last).is_none(), "no gear target");
    let (narrow, _) = shot.footer_at(22, browsing(""), false);
    assert_eq!(narrow, "  viewing   Esc chat", "the version drops, no gear");
}

fn peek_view(body: Option<&crate::app::peek::Body>) -> crate::app::peek::View<'_> {
    crate::app::peek::View {
        session: "api",
        seat: "lead",
        body,
        age_secs: 1,
        back: 0,
        newer_waiting: false,
    }
}

/// The close label sits on the title row, right-aligned, and is the
/// recorded click target of exactly its own cells, also when a long
/// session and seat name must be cut short of it.
#[test]
fn the_peek_close_label_is_a_target_over_its_own_cells() {
    let body = crate::app::peek::Body::Lines(vec!["x".to_owned()]);
    let long = "development-long-session";
    for (width, session, seat) in [
        (60, "api", "lead"),
        (40, "aedev", "apppeek-nav3"),
        (60, long, "apppeek-navigator-3"),
    ] {
        let view = crate::app::peek::View {
            session,
            seat,
            ..peek_view(Some(&body))
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 10));
        let mut layout = super::Layout::default();
        let bounds = super::paint_peek(&view, None, &mut buf, &mut layout);
        assert_eq!((bounds.visible, bounds.max_back), (6, 0));
        let title = line(&buf, 0).concat();
        assert!(title.starts_with("  Peek · "), "{title:?}");
        assert_eq!(title.contains(seat), seat == "lead", "the title is cut");
        let start = title.find("Esc close").expect("label");
        let column = u16::try_from(title[..start].chars().count()).expect("fits");
        let hit = |column| {
            let click = Mouse {
                kind: MouseKind::Click,
                column,
                row: 0,
            };
            matches!(layout.hit(click), Some(super::Hit::PeekClose))
        };
        assert!(hit(column) && hit(column + 8), "the label's cells");
        assert!(!hit(column - 1) && !hit(column + 9) && !hit(3), "only them");
        assert_eq!(column + 9 + LEFT, width, "right-aligned with the gutter");
        assert_eq!(layout.targets.len(), 1, "no other target survives");
    }
}

/// The status note runs to the right gutter, not to half the width: at 40
/// cells a 39-cell note is cut at its 38th and not before.
#[test]
fn the_peek_status_note_runs_to_the_right_gutter() {
    let body = crate::app::peek::Body::Lines(vec!["x".to_owned()]);
    let view = crate::app::peek::View {
        age_secs: 123_456_789,
        ..peek_view(Some(&body))
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
    let _ = super::paint_peek(&view, None, &mut buf, &mut super::Layout::default());
    assert_eq!(
        line(&buf, 1).concat().trim_end(),
        "  STALE · captured 123456789s ago · 1 ro",
    );
}

/// A long line wraps at the pane width with its continuation rows
/// hanging two cells, and a line that fits exactly does not wrap.
#[test]
fn a_peek_line_wraps_at_the_pane_width_with_a_two_cell_hang() {
    let room = 40 - usize::from(LEFT);
    let fits = "a".repeat(room);
    let words = "word ".repeat(30);
    let body = crate::app::peek::Body::Lines(vec![fits.clone(), words]);
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 14));
    let _ = super::draw_peek(&peek_view(Some(&body)), None, &mut buf);
    let rows: Vec<String> = (3..13).map(|y| line(&buf, y).concat()).collect();
    assert_eq!(rows[0].trim_end(), format!("  {fits}"), "exactly the room");
    assert!(rows[1].trim_start().starts_with("word"), "{rows:?}");
    assert!(rows[2].starts_with("    word"), "hung two more: {rows:?}");
    assert!(rows.iter().all(|row| row.chars().count() == 40));
}

/// Retiring the column for the seat view keeps the sidebar's targets
/// and seats, and drops the chat's own Compose target.
#[test]
fn the_peek_column_keeps_the_sidebar_targets() {
    let mut shot = Shot::new();
    shot.agents = Some(seat_facts(&["lead", "colead"]));
    let (buf, mut layout) = framed(&shot, 160, None);
    assert!(!layout.targets.is_empty(), "the closed frame has targets");
    let lane = layout.lane_area().expect("lane drawn");
    layout.peek_column(&buf, lane);
    assert!(
        layout
            .targets
            .iter()
            .any(|(_, hit)| matches!(hit, super::Hit::Session(_))),
        "sidebar targets stand"
    );
    assert!(
        !layout
            .targets
            .iter()
            .any(|(_, hit)| matches!(hit, super::Hit::Compose)),
        "the composer target is gone"
    );
    assert!(!layout.seats.is_empty(), "drawn seats stand");
}

/// Last whole count spans stay when they fit; overflow leaves one ellipsis.
#[test]
fn declutter_count_tail_fills_the_last_cell_without_losing_a_whole_span() {
    for (last, expected) in [
        (12, "✖B123 ⚠B123 ◌B12 ●B12 ◔123 ✓123 ·12"),
        (123, "✖B123 ⚠B123 ◌B12 ●B12 ◔123 ✓123 ·123"),
        (1234, "✖B123 ⚠B123 ◌B12 ●B12 ◔123 ✓123 …"),
    ] {
        let mut shot = Shot::new();
        shot.fleet.rows[0].home = false;
        shot.fleet.rows[0].name = "a".to_owned();
        shot.fleet.rows[0].line2 = Line2::NoGoal;
        shot.fleet.rows[0].counts = Counts::Marks(vec![
            (Mark::Dead, true, 123),
            (Mark::NeedsYou, true, 123),
            (Mark::Stale, true, 12),
            (Mark::Working, true, 12),
            (Mark::WaitingAgent, false, 123),
            (Mark::Done, false, 123),
            (Mark::Idle, false, last),
        ]);
        let (buf, layout) = shot.laid(160, 43, READ_ONLY);
        assert_eq!(layout.list.width, 44, "40 cells right of the mark");
        let row = line(&buf, 2)[..45].concat();
        assert!(row.ends_with(&format!("{expected}  │")), "{row:?}");
    }
}

/// A name that fits keeps its stopped age; a name too long for it is cut
/// beside the shortest tally. Either way the gap and two cells before the
/// rule stay.
#[test]
fn declutter_long_names_leave_the_stopped_age_and_right_edge_intact() {
    for (stopped, long) in [(false, 60), (true, 60), (true, 25)] {
        let mut shot = Shot::new();
        shot.fleet.rows = vec![row(&"n".repeat(long), 1, false)];
        shot.fleet.rows[0].home = false;
        shot.fleet.rows[0].counts = if stopped {
            Counts::Stopped
        } else {
            Counts::Marks(Vec::new())
        };
        shot.fleet.rows[0].line2 = if stopped {
            Line2::NotRunning {
                seats: 3,
                stopped_secs: Some(180),
            }
        } else {
            Line2::NoGoal
        };
        shot.model = Model::new(&shot.fleet);
        let (buf, layout) = shot.laid(160, 43, READ_ONLY);
        assert_eq!(layout.list.width, 44);
        let expected = match (stopped, long) {
            (false, _) => format!("  ● {}… no agents  │", "n".repeat(27)),
            (true, 60) => format!("  ● {}… stopped  │", "n".repeat(29)),
            _ => format!("  ● {}   stopped 3m  │", "n".repeat(25)),
        };
        assert_eq!(
            line(&buf, 2)[..45].concat(),
            expected,
            "stopped={stopped}/{long}"
        );
        assert_eq!(line(&buf, 2)[42..44].concat(), "  ");
        assert_eq!(buf[(44, 3)].symbol(), "│");
    }
}

/// A dragged u16-width sidebar still draws the complete stopped label.
#[test]
fn declutter_stopped_age_survives_a_sidebar_above_half_the_u16_range() {
    let mut shot = Shot::new();
    shot.fleet.rows[0].home = false;
    shot.fleet.rows[0].counts = Counts::Stopped;
    shot.fleet.rows[0].line2 = Line2::NotRunning {
        seats: 1,
        stopped_secs: Some(60),
    };
    let area = Rect::new(0, 0, 32_820, 20);
    assert_eq!(area.width, 32_820, "Rect keeps the actual terminal width");
    let grabbed = Drag {
        edge: Edge::Sidebar,
        from: 44,
        at: 44,
    };
    shot.model.grab(grabbed);
    let requested = dragged_to(
        grabbed,
        Mouse {
            kind: MouseKind::Drag,
            column: 32_775,
            row: 1,
        },
        area,
        shot.model.split(),
        1,
    );
    assert_eq!(requested, 32_775, "the actual drag allows this sidebar");
    let _ = shot.model.drag_to(requested);
    let _ = shot.model.let_go();
    let mut buf = Buffer::empty(area);
    assert_eq!(buf.area.width, 32_820);
    let layout = super::draw_with_layout(&shot.screen(READ_ONLY), super::Wait::default(), &mut buf);
    assert_eq!(layout.list.width, 32_775);
    assert_eq!(line(&buf, 2)[32_763..32_776].concat(), "stopped 1m  │");
}

/// Clipping preserves original grapheme barriers, then appends one ellipsis.
#[test]
fn declutter_clipped_reasons_keep_source_graphemes_separated() {
    for (text, expected) in [
        (format!("🇦\u{200b}🇧{}", "x".repeat(40)), "🇦🇧"),
        (format!("🇦\t🇧{}", "x".repeat(40)), "🇦🇧"),
        (format!("a\u{200b}b{}", "x".repeat(40)), "ab"),
    ] {
        let words = super::aged(&text, Some(240), 20);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        super::put(
            &mut buf,
            0,
            0,
            &words,
            20,
            ratatui_core::style::Style::default(),
        );
        let cells = line(&buf, 0).concat();
        assert_eq!(
            cells,
            format!("{expected}{}… · 4m", "x".repeat(12)),
            "{text}"
        );
    }
}

/// R1: two session rows under `look`, `unread` marked; the rest of the
/// screen rides a stock shot (its selection is api, as here).
fn laid_cue(look: &Look, unread: Vec<String>) -> Buffer {
    let shot = Shot::new();
    let rows = vec![row("api", 1, false), row("beta", 2, false)];
    let fleet = Fleet {
        rows,
        home: Some("api".to_owned()),
    };
    let mut model = Model::new(&fleet);
    model.set_unread(unread.into_iter().collect());
    let base = shot.screen(browsing(""));
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        look: Some(*look),
        ..base
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
    buf
}

/// `name`'s row: its y and the cue cell at col 1.
fn cue_cell(buf: &Buffer, name: &str) -> (u16, String) {
    for y in 0..buf.area.height {
        if line(buf, y)[4..4 + name.len()].concat() == name {
            return (y, buf[(1, y)].symbol().to_owned());
        }
    }
    panic!("row {name} not drawn");
}

/// The cue glyph reads in both palettes and in ASCII, and it moves no column.
#[test]
fn unread_cue_cell_reads_in_every_palette_and_ascii_without_shifting() {
    for (icons, palette, glyph) in [
        ("on", "darcula", "▸"),
        ("on", "neutral", "▸"),
        ("on", "warm", "▸"),
        ("off", "darcula", ">"),
    ] {
        let look = Look::read(icons, palette, "on", "on");
        let buf = laid_cue(&look, vec!["beta".to_owned()]);
        assert_eq!(cue_cell(&buf, "beta").1, glyph, "{icons}/{palette}");
        let blank = cue_cell(&buf, "api").1;
        assert_eq!(blank, " ", "{icons}/{palette}: control");
        let (y, _) = cue_cell(&buf, "beta");
        assert_eq!(
            buf[(2, y)].symbol(),
            Mark::Working.glyph(icons == "on"),
            "mark col"
        );
        assert_eq!(line(&buf, y)[4..8].concat(), "beta", "name col");
    }
}

/// R3: the gear click covers the gear cell and its leading blank, no
/// further; drawn cells are exactly what base draws.
#[test]
fn gear_click_covers_the_leading_blank_and_no_further() {
    let shot = Shot::new();
    let at_mouse = |col: u16| Mouse {
        kind: MouseKind::Click,
        column: col,
        row: 0,
    };
    let settings = |layout: &super::Layout, col: u16| {
        matches!(layout.hit(at_mouse(col)), Some(super::Hit::Settings))
    };
    let gear = "\u{2699}";
    let tail = format!("{} {gear}", crate::shape::version_line());
    let head = "\u{25c6} Enter writes · ? keys";
    let full = head.chars().count() + 2 + tail.chars().count();
    for (width, drawn) in [(full, tail.as_str()), (full - 1, gear)] {
        let at = u16::try_from(width).unwrap_or(0);
        let (row, layout) = shot.footer_at(at, browsing(""), true);
        let pad = " ".repeat(width - head.chars().count() - drawn.chars().count());
        let want = format!("{head}{pad}{drawn}");
        assert_eq!(row, want, "w={width}: drawn unchanged");
        assert!(settings(&layout, at - 1), "w={width}: gear cell");
        assert!(settings(&layout, at - 2), "w={width}: leading blank");
        assert!(!settings(&layout, at - 3), "w={width}: two left is not");
    }
    let at = u16::try_from(head.chars().count() + 2).unwrap_or(0);
    let (row, layout) = shot.footer_at(at, browsing(""), true);
    assert!(!row.contains(gear), "narrow: no gear drawn");
    assert!(!settings(&layout, at - 1), "narrow: no target");
}

/// I-R1: a kept quota body draws its line first, in the stale tone, above the kept
/// rows; without the marker, and on any other tab, no such line is drawn.
#[test]
fn a_kept_quota_body_draws_its_line_first_in_the_stale_tone() {
    use crate::app::model::SettingsTab;
    let look = Look::read("on", "darcula", "on", "on");
    let paint = Paint::of(Some(&look));
    let line = "last updated 3m ago — refreshing";
    let mut bodies = crate::app::settings::SettingsBodies {
        quota: Some(crate::quota::view::unavailable()),
        quota_kept: Some(line.to_owned()),
        ..crate::app::settings::SettingsBodies::default()
    };
    let kept = super::settings_body(SettingsTab::Quota, &bodies, paint, 80, true);
    let stale = paint.fg(|p| p.stale);
    assert_ne!(stale, paint.fg(|p| p.text), "the stale tone is its own");
    assert_eq!(kept[0], (line.to_owned(), stale), "{kept:?}");
    let rows = kept[1..]
        .iter()
        .any(|(text, _)| text.contains("quota: unavailable"));
    assert!(rows, "the kept rows follow: {kept:?}");
    let config = super::settings_body(SettingsTab::Config, &bodies, paint, 80, true);
    assert!(!config.iter().any(|(text, _)| text.contains("refreshing")));
    bodies.quota_kept = None;
    let fresh = super::settings_body(SettingsTab::Quota, &bodies, paint, 80, true);
    assert_eq!(fresh[..], kept[1..], "no marker, no line");
}

/// Hangul jamo/syllable widths and the zero-width barrier, cut whole:
/// jamo U+1100 is 2 cells, ZWSP 0, jungseong U+1161 combining 0, syllable
/// U+AC00 2 cells. The ZWSP/tab breaks L+V conjoining; kept zero-width
/// bytes stay in the cut.
#[test]
fn unicode_barrier_rows_clip_whole_graphemes_and_age() {
    // (text, room, expected); content + … + tail must fit `room`.
    for (text, room, expected) in [
        (
            "ᄀ\u{200b}가".to_owned() + &"x".repeat(40),
            20,
            "ᄀ\u{200b}가".to_owned() + &"x".repeat(10) + "… · 4m",
        ),
        (
            "ᄀ\u{200b}ᅡ".to_owned() + &"x".repeat(40),
            20,
            "ᄀ\u{200b}ᅡ".to_owned() + &"x".repeat(12) + "… · 4m",
        ),
        (
            "ᄀ\t가".to_owned() + &"x".repeat(40),
            20,
            "ᄀ\t가".to_owned() + &"x".repeat(10) + "… · 4m",
        ),
        // A combining jungseong kept whole: 14 + 0 + … + tail.
        (
            "x".repeat(14) + "ᅡ" + &"x".repeat(40),
            20,
            "x".repeat(14) + "ᅡ… · 4m",
        ),
        // A wide syllable never split: 13 cells used, needs 2 of 1 left.
        (
            "x".repeat(13) + "가" + &"x".repeat(40),
            20,
            "x".repeat(13) + "… · 4m",
        ),
        // A halfwidth cluster (halfwidth ka + voicing mark) is one 2-cell
        // grapheme (mark compensated per ratatui-core cell_width).
        (
            "x".repeat(12) + "ｶﾞ" + &"x".repeat(40),
            20,
            "x".repeat(12) + "ｶﾞ… · 4m",
        ),
        // A tab is filtered from the stream (0 cells); neighbours never glue.
        (
            "a\tb".to_owned() + &"x".repeat(40),
            20,
            "a\tb".to_owned() + &"x".repeat(12) + "… · 4m",
        ),
    ] {
        let words = super::aged(&text, Some(240), room);
        assert_eq!(words, expected, "{text:?}/{room}");
        assert!(super::drawn_cols(&words) <= room, "{text:?}/{room}");
    }
}

/// Control graphemes never reach cells and zero-width ones take none:
/// aZWSPbTABc draws abc, a wide jamo itself plus its reset continuation,
/// a halfwidth cluster whole from its first cell. Pure `aged()` cannot prove
/// it keeps the raw bytes.
#[test]
fn zero_width_source_cells_stand_as_blanks_without_conjoining() {
    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::Rect;

    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
    super::put_line(
        &mut buf,
        0,
        0,
        &ratatui_core::text::Line::from("a\u{200b}b\tc"),
        10,
    );
    assert_eq!(line(&buf, 0)[..5].concat(), "abc  ");
    super::put_line(
        &mut buf,
        0,
        1,
        &ratatui_core::text::Line::from("ᄀ\u{200b}ᅡ"),
        10,
    );
    assert_eq!(line(&buf, 1)[..3].concat(), "ᄀ  ");
    super::put_line(&mut buf, 0, 2, &ratatui_core::text::Line::from("ｶﾞ"), 10);
    assert_eq!(line(&buf, 2)[..1].concat(), "ｶﾞ");
}

// agentwrap frozen behavioral acceptance — r5 fragment for src/l5_ui/app/draw.rs
// INSERT: immediately before the closing brace of `mod tests` in src/l5_ui/app/draw.rs
// (the module containing a_cut_body_names_its_hidden_rows_in_the_dim_hue,
// draw.rs:3311 after the driver rebase onto 54f96696).
// Supersedes frozen/r4/draw-agentwrap-tests.rs (kept immutable). Folds colead
// gate review-20261019T081224Z-16d86b15 + oracle-spec-r5-delta eeb164845370:
// B1 the fitting wide model is SEVEN glyphs / 14 cells (r4 mislabelled it
//     5/10): joined 14+1+5 = 20 = free — the wide exact-fit case; separator
//     x 22, effort x 23..27 (the wrap case's 22/23..27 was already right);
// B2 the floor literals were 25/26 chars, now 24/25: capacity-24 whole fit,
//     25 closes at 23 cells + …;
// I1 casts are checked (u16::try_from, the codebase's own test idiom);
// I2 no unused bindings (the I4 test's rule binding removed);
// I3 doc comments carry no bare identifiers or shouty prose for doc_markdown;
// I4 region guards compare FULL Cells (symbol + style) and now include the
//     right gap columns 42..44 beside the gutter and the border/chat columns.
//
// Derived geometry (binding, verified): LEFT = 2 (draw.rs:88); room = rule - 4
// (draw.rs:740-741); rule = sidebar_width(area, split), split.sidebar pinned
// via grab/drag_to/let_go (model.rs:546-565); clamp-to-30 and area-fallback
// semantics owner-anchored (provenance-addendum.md). seat_row inline:
// mark@0, blank@1, name@2, meta start = 2 + column + 2, state tail =
// 1 + state_cols. aa/bb/cc (S=44): column 2, start 6, free 20, capacity 38,
// meta x 8, name x 4. sNN (S=44): column 3, free 19. Sweep at draw(100, 21):
// room 30, free 0, name clipped to 13 chars + ellipsis, every seat wraps
// post-fix, shown 13, more-line (20, 2). shot.draw(.., None) frames: mono
// render, icons on; Look::read("off", "darcula", "on", "on") frames: colour
// darcula palette, icons off; the palette lift sets the selected background
// and repaints dim words in the selected ink, never the reverse modifier
// (draw.rs:399-406).

/// Seats with controlled label fields: (name, model, effort, client).
fn label_facts(specs: &[(&str, &str, &str, &str)]) -> Facts {
    let Facts::Seats { id, agents } = sweep_seats() else {
        unreachable!("the sweep draws seats");
    };
    let base = agents[0].clone();
    Facts::Seats {
        id,
        agents: specs
            .iter()
            .map(|(name, model, effort, client)| PickerAgent {
                name: (*name).to_owned(),
                state: FIT_STATE.to_owned(),
                model: (*model).to_owned(),
                effort: (*effort).to_owned(),
                client: (*client).to_owned(),
                ..base.clone()
            })
            .collect(),
    }
}

/// Pin the sidebar's width: grab the edge, drag it to `size`, let go.
fn with_sidebar(shot: &mut Shot, size: u16) {
    shot.model.grab(Drag {
        edge: Edge::Sidebar,
        from: 0,
        at: 0,
    });
    let _ = shot.model.drag_to(size);
    let _ = shot.model.let_go();
}

/// R2/R6: the joined label filling `free` exactly stays on one line.
#[test]
fn exact_fit_stays_one_line_with_the_full_label() {
    let mut shot = Shot::new();
    // free = 20; 14 + 1 + 5 = 20.
    shot.agents = Some(label_facts(&[
        ("aa", "m-model-fourte", "e1234", ""),
        ("bb", "", "", "claude"),
        ("cc", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, layout) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert_eq!(layout.seats, ["aa", "bb", "cc"]);
    let one = line(&buf, rule + 1).concat();
    assert!(one.contains("m-model-fourte e1234"), "{one}");
    assert!(one.contains(FIT_STATE), "{one}");
    assert_eq!(buf[(8, rule + 1)].symbol(), "m", "the meta column");
    assert_eq!(buf[(29, rule + 1)].symbol(), "u", "the state word starts");
    assert_eq!(buf[(41, rule + 1)].symbol(), "d", "the state word ends");
    assert!(
        line(&buf, rule + 2).concat().contains("bb"),
        "one row a seat"
    );
}

/// R1/R3/R6: one cell over `free` moves the COMPLETE label to row two —
/// today the effort half is silently lost on row one (RED: effort-missing).
#[test]
fn one_cell_deficit_moves_the_complete_label_to_row_two() {
    let mut shot = Shot::new();
    // free = 20; 15 + 1 + 5 = 21 = free + 1.
    shot.agents = Some(label_facts(&[
        ("aa", "m-model-fifteen", "e1234", ""),
        ("bb", "", "", "claude"),
        ("cc", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, layout) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    let head = line(&buf, rule + 1).concat();
    assert!(
        !head.contains("m-model-fifteen"),
        "no meta on row one: {head}"
    );
    let tail = line(&buf, rule + 2).concat();
    assert!(
        tail.contains("m-model-fifteen e1234"),
        "full label on row two: {tail}"
    );
    assert_eq!(
        buf[(2, rule + 2)].symbol(),
        " ",
        "no mark on the continuation"
    );
    assert_eq!(buf[(3, rule + 2)].symbol(), " ", "blank under the blank");
    assert_eq!(buf[(4, rule + 2)].symbol(), "m", "the continuation x is 4");
    let click = Mouse {
        kind: MouseKind::Click,
        column: 8,
        row: rule + 2,
    };
    assert!(matches!(layout.hit(click), Some(super::Hit::Seat(name)) if name == "aa"));
    assert_eq!(layout.seats, ["aa", "bb", "cc"], "logical seats, deduped");
    assert!(
        line(&buf, rule + 3).concat().contains("bb"),
        "mixed geometry"
    );
}

/// R1/R3/R4: the model alone over `free` wraps instead of dropping the whole
/// label (today: meta = None, nothing drawn — RED: model-missing).
#[test]
fn model_only_overflow_wraps_instead_of_dropping() {
    let mut shot = Shot::new();
    // model 21 cells > free 20; effort empty.
    shot.agents = Some(label_facts(&[
        ("aa", "m12345678901234567890", "", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        !line(&buf, rule + 1)
            .concat()
            .contains("m12345678901234567890"),
        "no label on row one"
    );
    assert!(
        line(&buf, rule + 2)
            .concat()
            .contains("m12345678901234567890"),
        "the full model on row two"
    );
}

/// R7 case (c): the joined label exceeds row-2 capacity while the model alone
/// fits it: row two keeps the FULL model and deliberately omits the effort,
/// which appears nowhere (today: both dropped — RED).
#[test]
fn row_two_capacity_keeps_the_full_model_and_omits_the_effort() {
    let mut shot = Shot::new();
    // model 12, effort 30 -> joined 43 > 38 = row-2 capacity; model 12 <= 38.
    shot.agents = Some(label_facts(&[
        ("aa", "m12cellwords", "e12345678901234567890123456789", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        line(&buf, rule + 2).concat().contains("m12cellwords"),
        "the full model on row two"
    );
    assert!(
        !line(&buf, rule + 1).concat().contains("e1234567890")
            && !line(&buf, rule + 2).concat().contains("e1234567890"),
        "the effort is deliberately omitted, nowhere drawn"
    );
}

/// R7 case (d): the model alone exceeds row-2 capacity: row two fits it at a
/// grapheme and closes on an ellipsis — never blank, never lossless (RED).
/// The cut reserves one cell for the ellipsis: 37 cells of text + … = 38.
#[test]
fn model_past_row_two_capacity_closes_on_an_ellipsis() {
    let mut shot = Shot::new();
    // model 41 cells > 38.
    let model = "m1234567890123456789012345678901234567890";
    shot.agents = Some(label_facts(&[
        ("aa", model, "", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    let tail = line(&buf, rule + 2).concat();
    let fitted = format!("{}…", &model[..37]);
    assert!(
        tail.contains(&fitted),
        "the 37-cell prefix closed on …: {tail}"
    );
    assert!(!tail.contains(model), "the full model does not fit");
    assert!(tail.trim().len() > 1, "never blank");
}

/// R4 (colead pin): an effort-only seat over `free` wraps and keeps a
/// truthful nonblank FITTED label (today: silently dropped — RED).
#[test]
fn effort_only_overflow_keeps_a_fitted_nonblank_label() {
    let mut shot = Shot::new();
    // effort 21 cells > free 20; model and client empty.
    shot.agents = Some(label_facts(&[
        ("aa", "", "e12345678901234567890", ""),
        ("bb", "", "", ""),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    let tail = line(&buf, rule + 2).concat();
    assert!(
        tail.contains("e12345678901234567890"),
        "the full effort: {tail}"
    );
    assert!(tail.trim().len() > 1, "never blank");
}

/// R4/R7: an effort-only seat past row-2 capacity is fitted, not blanked.
#[test]
fn effort_only_past_row_two_capacity_is_fitted_not_blanked() {
    let mut shot = Shot::new();
    // effort 50 cells > 38; fitted = 37 cells + ….
    let effort = "e1234567890123456789012345678901234567890123456789";
    shot.agents = Some(label_facts(&[("aa", "", effort, ""), ("bb", "", "", "")]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    let tail = line(&buf, rule + 2).concat();
    assert!(
        tail.contains(&format!("{}…", &effort[..37])),
        "fitted: {tail}"
    );
    assert!(!tail.contains(effort), "the full effort does not fit");
    assert!(tail.trim().len() > 1, "never blank");
}

/// R4: the client fallback is the label source too: it fits on row one, and
/// over `free` it wraps whole (today the overflow drops it — RED).
#[test]
fn client_fallback_label_wraps_instead_of_dropping() {
    let mut shot = Shot::new();
    // client 21 cells > free 20; model and effort empty.
    shot.agents = Some(label_facts(&[
        ("aa", "", "", "c12345678901234567890"),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        !line(&buf, rule + 1)
            .concat()
            .contains("c12345678901234567890"),
        "no label on row one"
    );
    assert!(
        line(&buf, rule + 2)
            .concat()
            .contains("c12345678901234567890"),
        "the full client label on row two"
    );
    // The fitting fallback stays on row one (today's contract, unchanged).
    let mut shot = Shot::new();
    shot.agents = Some(label_facts(&[("aa", "", "", "claude")]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(line(&buf, rule + 1).concat().contains("claude"));
}

/// R9 (colead I2): a wide label is measured in display cells and EVERY painted
/// cell is asserted: each wide glyph at its even column, a blank trailing cell
/// after each, then the separator and the effort. A wide glyph's trailing cell
/// is blank, so no contiguous run ever appears in a line concat.
#[test]
fn wide_labels_are_measured_in_display_cells() {
    let mut shot = Shot::new();
    // Wrap half: 9 wide chars = 18 cells; 18 + 1 + 5 = 24 > 20.
    let model = "モデルモデルモデル";
    shot.agents = Some(label_facts(&[
        ("aa", model, "e1234", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        !line(&buf, rule + 1).concat().contains('モ'),
        "no wide label on row one"
    );
    for (at, ch) in model.chars().enumerate() {
        let at = u16::try_from(at).expect("small index");
        assert_eq!(
            buf[(4 + 2 * at, rule + 2)].symbol(),
            ch.to_string(),
            "glyph {at}"
        );
        assert_eq!(
            buf[(5 + 2 * at, rule + 2)].symbol(),
            " ",
            "trailing cell {at}"
        );
    }
    assert_eq!(buf[(22, rule + 2)].symbol(), " ", "the separator");
    for (at, ch) in "e1234".chars().enumerate() {
        let at = u16::try_from(at).expect("small index");
        assert_eq!(
            buf[(23 + at, rule + 2)].symbol(),
            ch.to_string(),
            "effort glyph {at}"
        );
    }
    // Fit half: SEVEN wide chars = 14 cells; 14 + 1 + 5 = 20 = free: the wide
    // exact-fit case, separator x 22, effort x 23..27.
    let mut shot = Shot::new();
    let model = "モデルモデルモ";
    shot.agents = Some(label_facts(&[("aa", model, "e1234", "")]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    for (at, ch) in model.chars().enumerate() {
        let at = u16::try_from(at).expect("small index");
        assert_eq!(
            buf[(8 + 2 * at, rule + 1)].symbol(),
            ch.to_string(),
            "glyph {at}"
        );
        assert_eq!(
            buf[(9 + 2 * at, rule + 1)].symbol(),
            " ",
            "trailing cell {at}"
        );
    }
    assert_eq!(buf[(22, rule + 1)].symbol(), " ", "the separator");
    for (at, ch) in "e1234".chars().enumerate() {
        let at = u16::try_from(at).expect("small index");
        assert_eq!(
            buf[(23 + at, rule + 1)].symbol(),
            ch.to_string(),
            "effort glyph {at}"
        );
    }
}

/// R9: combining marks cost no cells: a label of 12 cells carrying a
/// combining mark at the exact-fit boundary stays on one line.
#[test]
fn combining_marks_cost_no_cells_at_the_exact_fit_boundary() {
    let mut shot = Shot::new();
    // "e" + U+0301 = 1 cell, plus 11 digits = 12 cells; 12 + 1 + 7 = 20 = free.
    shot.agents = Some(label_facts(&[
        ("aa", "e\u{301}23456789012", "e123456", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        line(&buf, rule + 1)
            .concat()
            .contains("e\u{301}23456789012 e123456")
    );
    assert!(line(&buf, rule + 2).concat().contains("bb"), "one row");
}

/// R5/D1 + N1: every name keeps the roster column (abs x 4) and the
/// continuation text begins at the owner-pinned abs x 4 under the name.
#[test]
fn names_stay_aligned_and_the_continuation_starts_at_abs_x_four() {
    let mut shot = Shot::new();
    shot.agents = Some(label_facts(&[
        ("aa", "m-model-fifteen", "e1234", ""),
        ("bb", "", "", "claude"),
        ("cc", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    for y in [rule + 1, rule + 3, rule + 4] {
        assert_eq!(buf[(3, y)].symbol(), " ", "blank before the name");
        assert!(buf[(4, y)].symbol() != " ", "the name at x 4");
    }
    assert_eq!(buf[(3, rule + 2)].symbol(), " ", "blank before the label");
    assert_eq!(buf[(4, rule + 2)].symbol(), "m", "the label at x 4");
}

/// R10: the cursor glyph rides the head row only; the viewed seat's lift
/// covers BOTH rows. The plain frame (no look) is the mono render: icons on,
/// reverse-video ground. The `Look::read` frame is the colour darcula palette
/// with icons off: the palette lift sets the selected background and repaints
/// dim words in the selected ink; it never sets the reverse modifier
/// (draw.rs:399-406).
#[test]
fn cursor_and_lift_cover_the_wrapped_seat() {
    let mut shot = Shot::new();
    shot.agents = Some(label_facts(&[
        ("aa", "m-model-fifteen", "e1234", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 44);
    shot.model.set_focus(Some("aa".to_owned()));
    // The mono frame: icons on, reverse-video ground.
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert_eq!(marked(&buf, "▹"), [rule + 1], "the cursor on the head row");
    let viewing = Composer::Viewing { seat: "aa" };
    let (buf, _) = framed_as(&shot, 160, None, viewing);
    assert_eq!(marked(&buf, "▸"), [rule + 1], "the lift glyph");
    assert!(buf[(8, rule + 1)].modifier.contains(Modifier::REVERSED));
    assert!(
        buf[(8, rule + 2)].modifier.contains(Modifier::REVERSED),
        "mono reverses both rows"
    );
    // The colour frame: ASCII glyphs, no reverse modifier.
    let look = Look::read("off", "darcula", "on", "on");
    let (buf, _) = framed(&shot, 160, Some(&look));
    assert_eq!(marked(&buf, "-"), [rule + 1], "the ASCII cursor");
    let (buf, _) = framed_as(&shot, 160, Some(&look), viewing);
    assert_eq!(marked(&buf, ">"), [rule + 1], "the ASCII lift glyph");
    assert_eq!(
        buf[(8, rule + 1)].bg,
        super::colour(crate::theme::Palette::DARCULA.selected),
        "the selected ground on the head row"
    );
    assert_eq!(
        buf[(8, rule + 2)].bg,
        super::colour(crate::theme::Palette::DARCULA.selected),
        "the selected ground on the continuation"
    );
    assert_eq!(
        buf[(4, rule + 2)].fg,
        super::colour(crate::theme::Palette::DARCULA.selected_ink),
        "dim words lift into the selected ink"
    );
    assert!(
        !buf[(8, rule + 2)].modifier.contains(Modifier::REVERSED),
        "colour never reverses"
    );
}

/// R8/D3 + R12: the viewport may open on a continuation: that row keeps the
/// cursor glyph and the seat's hit (precondition: the body really wrapped and
/// cut, so a false green cannot hide a missing second row). Wrapper labels
/// are 23 cells over the three-letter seat names' free 19.
#[test]
fn the_viewport_may_open_on_a_continuation_keeping_glyph_and_hit() {
    const WRAP: &str = "m1234567890123456789012";
    let mut shot = Shot::new();
    let specs: Vec<(&str, &str, &str, &str)> = (0..40)
        .map(|at| {
            let name: &'static str = match at {
                0 => "aa",
                39 => "zz",
                _ => Box::leak(format!("s{at:02}").into_boxed_str()),
            };
            let model = if at == 0 || at == 39 { WRAP } else { "" };
            (name, model, "", "")
        })
        .collect();
    shot.agents = Some(label_facts(&specs));
    with_sidebar(&mut shot, 44);
    shot.model.set_focus(Some("aa".to_owned()));
    // aa occupies body rows 0-1; its continuation is body row 1.
    shot.model.settle(None, Some(1));
    let (buf, layout) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        layout.body_max >= 1,
        "precondition: the body wrapped and cut"
    );
    assert_eq!(
        buf[(4, rule + 1)].symbol(),
        "m",
        "the first visible row is the continuation, not the head"
    );
    assert!(
        line(&buf, rule + 1).concat().contains(WRAP),
        "the label is drawn"
    );
    assert_eq!(
        marked(&buf, "▹"),
        [rule + 1],
        "the glyph rides the continuation"
    );
    let click = Mouse {
        kind: MouseKind::Click,
        column: 8,
        row: rule + 1,
    };
    assert!(matches!(layout.hit(click), Some(super::Hit::Seat(name)) if name == "aa"));
}

/// R8/D3 + R14: the body scrolls to its last row: the more-line then names
/// only rows above, and the LAST agent's continuation (the last agent wraps)
/// is fully visible above the more-line.
#[test]
fn the_body_reaches_its_last_row_and_names_nothing_below() {
    const WRAP: &str = "m1234567890123456789012";
    let mut shot = Shot::new();
    let specs: Vec<(&str, &str, &str, &str)> = (0..40)
        .map(|at| {
            let name: &'static str = match at {
                0 => "aa",
                39 => "zz",
                _ => Box::leak(format!("s{at:02}").into_boxed_str()),
            };
            let model = if at == 0 || at == 39 { WRAP } else { "" };
            (name, model, "", "")
        })
        .collect();
    shot.agents = Some(label_facts(&specs));
    with_sidebar(&mut shot, 44);
    let (_, layout) = framed(&shot, 160, None);
    let top = layout.body_max;
    assert!(top >= 1, "precondition: the body wrapped and cut");
    shot.model.settle(None, Some(top));
    let (buf, layout) = framed(&shot, 160, None);
    assert_eq!(layout.body_top, Some(top), "the last row is reachable");
    let more = line(&buf, 44).concat();
    assert!(
        more.contains('↑') && !more.contains('↓'),
        "nothing hides below: {more}"
    );
    let last = line(&buf, 43).concat();
    assert!(last.contains(WRAP), "zz's continuation visible: {last}");
    assert_eq!(buf[(4, 43)].symbol(), "m", "the continuation at x 4");
}

/// R8/D3 + R14 (colead I4): the viewport may END on a head row whose one
/// continuation is hidden: settling to one under the frame's last-row bound
/// makes the LAST agent's head the final drawn row for EVERY shown, so
/// exactly one row hides below.
#[test]
fn a_head_row_may_end_the_viewport_with_its_continuation_hidden() {
    const WRAP: &str = "m1234567890123456789012";
    let mut shot = Shot::new();
    let specs: Vec<(&str, &str, &str, &str)> = (0..40)
        .map(|at| {
            let name: &'static str = match at {
                0 => "aa",
                39 => "zz",
                _ => Box::leak(format!("s{at:02}").into_boxed_str()),
            };
            let model = if at == 0 || at == 39 { WRAP } else { "" };
            (name, model, "", "")
        })
        .collect();
    shot.agents = Some(label_facts(&specs));
    with_sidebar(&mut shot, 44);
    let (_, layout) = framed(&shot, 160, None);
    let top = layout.body_max;
    assert!(top >= 2, "precondition: the body wrapped and cut");
    shot.model.settle(None, Some(top - 1));
    let (buf, layout) = framed(&shot, 160, None);
    let last = line(&buf, 43).concat();
    assert!(
        last.contains("zz"),
        "the head row is the last drawn row: {last}"
    );
    assert!(
        !last.contains(WRAP),
        "the continuation is not drawn: {last}"
    );
    let more = line(&buf, 44).concat();
    assert!(
        more.contains("↓ 1 more rows"),
        "exactly one row hides below: {more}"
    );
    let click = Mouse {
        kind: MouseKind::Click,
        column: 8,
        row: 43,
    };
    assert!(matches!(layout.hit(click), Some(super::Hit::Seat(name)) if name == "zz"));
}

/// R14 (oracle r2 B2 + r3 B1): at the default draw(100, 21) geometry room is
/// 30, so the sweep's 30-cell names clip to 13 chars + ellipsis (budget 14)
/// and the 16-cell label misses `free` (saturating to 0): POST-FIX every one
/// of the 20 seats wraps and the body doubles: 2 x 20 - 14 shown = 26 hidden.
/// The one-line summary gives the body one more row.
/// The head is addressed by the rule, never by the full name.
#[test]
fn the_more_row_line_counts_rows_not_seats() {
    let mut shot = Shot::new();
    let Facts::Seats { id, agents } = sweep_seats() else {
        unreachable!("the sweep draws seats");
    };
    let mut agents = agents;
    for agent in &mut agents {
        agent.state = FIT_STATE.to_owned();
    }
    let agents: Vec<PickerAgent> = agents.into_iter().cycle().take(20).collect();
    shot.agents = Some(Facts::Seats { id, agents });
    let wrapped = shot.draw(100, 21, READ_ONLY);
    let rule = horizontal(&wrapped).expect("the Agents rule");
    assert!(
        line(&wrapped, rule + 1).concat().contains("a-seat-name-l…"),
        "precondition: the clipped head row"
    );
    assert!(
        line(&wrapped, rule + 2).concat().contains("Sonnet 5.5"),
        "precondition: the first seat wrapped"
    );
    assert_eq!(
        spot(&wrapped, "↓ 26 more rows"),
        Some((20, 2)),
        "2 x 20 - 14"
    );
}

/// R15/N2: the session rows above the Agents rule are untouched by wrapping:
/// every FULL CELL from the top row through the rule, across the whole width,
/// is identical between a wrapping frame and a fitting frame (colead I4/I6).
#[test]
fn session_rows_are_untouched_by_wrapping() {
    let mut wrapped = Shot::new();
    wrapped.fleet = frame_fleet();
    wrapped.agents = Some(label_facts(&[
        ("aa", "m-model-fifteen", "e1234", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut wrapped, 44);
    let (wbuf, _) = framed(&wrapped, 160, None);
    let above = horizontal(&wbuf).expect("the Agents rule");
    let mut fitting = Shot::new();
    fitting.fleet = frame_fleet();
    fitting.agents = Some(label_facts(&[
        ("aa", "m-model-fourte", "e1234", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut fitting, 44);
    let (buf, _) = framed(&fitting, 160, None);
    for y in 0..=above {
        for x in 0..buf.area.width {
            assert_eq!(
                buf[(x, y)],
                wbuf[(x, y)],
                "the full cell is unchanged by wrapping at ({x}, {y})"
            );
        }
    }
}

/// R11 (colead I4/I6): nothing the sidebar writes lands outside its own
/// cells: the gutter columns 0..2, the right gap 42..44 and every cell from
/// the border column 44 rightward of the two label rows are FULL-CELL
/// identical between the wrapping frame and a no-meta frame — whatever the
/// chat draws there.
#[test]
fn wrapped_cells_stay_inside_the_body() {
    let mut wrapped = Shot::new();
    wrapped.agents = Some(label_facts(&[
        ("aa", "m-model-fifteen", "e1234", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut wrapped, 44);
    let (wbuf, _) = framed(&wrapped, 160, None);
    let rule = horizontal(&wbuf).expect("the Agents rule");
    let mut bare = Shot::new();
    bare.agents = Some(label_facts(&[
        ("aa", "", "", "claude"),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut bare, 44);
    let (bbuf, _) = framed(&bare, 160, None);
    for y in [rule + 1, rule + 2] {
        for x in 0..2 {
            assert_eq!(
                wbuf[(x, y)],
                bbuf[(x, y)],
                "the gutter is untouched at ({x}, {y})"
            );
        }
        for x in 42..wbuf.area.width {
            assert_eq!(
                wbuf[(x, y)],
                bbuf[(x, y)],
                "the right gap, border and chat are untouched at ({x}, {y})"
            );
        }
    }
}

/// R4 (colead I5): a seat with NOTHING known — model, client and effort all
/// empty — draws ONE row, invents no placeholder, and leaves the meta column
/// and every later row where they were.
#[test]
fn fully_unknown_labels_draw_one_row_and_invent_nothing() {
    let mut shot = Shot::new();
    shot.agents = Some(label_facts(&[
        ("aa", "", "", ""),
        ("bb", "", "", ""),
        ("cc", "", "", ""),
    ]));
    with_sidebar(&mut shot, 44);
    let (buf, layout) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert_eq!(layout.seats, ["aa", "bb", "cc"], "one logical seat each");
    assert_eq!(buf[(8, rule + 1)].symbol(), " ", "no meta for aa");
    assert!(
        line(&buf, rule + 2).concat().contains("bb"),
        "bb follows directly"
    );
    assert_eq!(buf[(8, rule + 2)].symbol(), " ", "no meta for bb");
    assert!(
        line(&buf, rule + 3).concat().contains("cc"),
        "cc follows directly"
    );
    for y in [rule + 1, rule + 2, rule + 3] {
        assert!(
            !line(&buf, y).concat().contains('…'),
            "no ellipsis is invented at ({y})"
        );
    }
}

/// R7 (colead I3): the S = 30 sidebar floor pins the row-2 capacity boundary
/// executably: room 26, capacity 24, free 6. A 24-cell model is drawn WHOLE;
/// a 25-cell model closes on the ellipsis at 23 cells + …; a 6-cell joined
/// label still fits row one at the floor; an UNDERSIZED request clamps to the
/// floor instead of refusing, and the boundary holds there too.
#[test]
fn the_sidebar_floor_pins_the_row_two_capacity_boundary() {
    let full = "m12345678901234567890123";
    let mut shot = Shot::new();
    shot.agents = Some(label_facts(&[
        ("aa", full, "", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 30);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        line(&buf, rule + 2).concat().contains(full),
        "24 cells fit row two whole at the floor"
    );
    assert!(
        !line(&buf, rule + 2).concat().contains('…'),
        "no ellipsis at 24"
    );
    let over = "m123456789012345678901234";
    let mut shot = Shot::new();
    shot.agents = Some(label_facts(&[
        ("aa", over, "", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 30);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        line(&buf, rule + 2)
            .concat()
            .contains(&format!("{}…", &over[..23])),
        "25 cells close on the ellipsis at 23 + …"
    );
    assert!(
        !line(&buf, rule + 2).concat().contains(over),
        "the whole does not fit"
    );
    let mut shot = Shot::new();
    shot.agents = Some(label_facts(&[
        ("aa", "m12345", "", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 30);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        line(&buf, rule + 1).concat().contains("m12345"),
        "the 6-cell joined label fits row one at the floor"
    );
    assert!(
        line(&buf, rule + 2).concat().contains("bb"),
        "one row at the floor"
    );
    // The undersized request clamps to the 30 floor (owner anchor): the
    // capacity-24 whole fit still holds after requesting 20.
    let mut shot = Shot::new();
    shot.agents = Some(label_facts(&[
        ("aa", full, "", ""),
        ("bb", "", "", "claude"),
    ]));
    with_sidebar(&mut shot, 20);
    let (buf, _) = framed(&shot, 160, None);
    let rule = horizontal(&buf).expect("the Agents rule");
    assert!(
        line(&buf, rule + 2).concat().contains(full),
        "the request clamped to the floor: 24 cells still fit whole"
    );
}
