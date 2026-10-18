//! Frozen acceptance: sidecompact single-line sidebar plus truthful highlight.
//!
//! Synthetic fixtures only (platform/571/demo/svc-*; lead/colead/ghost/solo).
//! Never touches the real screenshot or journals (colead data boundary).
//! Public API only: the app draw, fleet, model and overview modules, plus the
//! console lane, digest, theme and tmux modules. Install byte-identical as the
//! sidecompact spec integration test plus one mod line; no other tree change.
//!
//! RED = fails on base 5c470588 at the asserted line, passes after (10 tests).
//! GUARD = passes on base and after, pins the contract (7 tests).

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "frozen acceptance renders buffers and indexes rule columns only"
)]

use ae::app::draw::{Composer, Screen, draw};
use ae::app::fleet::{Counts, Facts, Fleet, Line2, Row};
use ae::app::model::{Act, Key, Model};
use ae::app::overview::{self, Open, Overview};
use ae::console::lane::Lane;
use ae::digest::{SessionEntry, Status};
use ae::theme::{Look, Mark};
use ae::time::Timestamp;
use ae::tmux::PickerAgent;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Modifier;

const NOW: i64 = 1_791_446_400;

#[allow(
    clippy::unnecessary_wraps,
    reason = "symmetric Option<Look> constructors"
)]
fn icons_on() -> Option<Look> {
    Some(Look::read("on", "darcula", "on", "on"))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "symmetric Option<Look> constructors"
)]
fn icons_off() -> Option<Look> {
    Some(Look::read("off", "", "off", "off"))
}

fn theme_off() -> Option<Look> {
    None
}

fn agent(name: &str, state: &str, client: &str, model: &str, effort: &str) -> PickerAgent {
    PickerAgent {
        name: name.to_owned(),
        profile: format!("{name}-profile"),
        state: state.to_owned(),
        pane: "%11".to_owned(),
        client: client.to_owned(),
        model: model.to_owned(),
        effort: effort.to_owned(),
        drift: false,
    }
}

fn seats() -> Facts {
    Facts::Seats {
        id: "$7".to_owned(),
        agents: vec![
            agent("lead", "working", "atlas", "Opus 5.5", "high"),
            agent("colead", "waiting-user", "atlas", "gpt-6.1-sol", "xhigh"),
        ],
    }
}

fn srow(name: &str, index: usize, home: bool, line2: Line2) -> Row {
    Row {
        name: name.to_owned(),
        index,
        mark: Mark::Working,
        needy: false,
        counts: Counts::Marks(vec![(Mark::Working, false, 1)]),
        line2,
        home,
    }
}

fn question() -> Line2 {
    Line2::Question {
        text: "awaiting owner reply".to_owned(),
        age_secs: Some(120),
    }
}

/// Three synthetic sessions; the selected one is `home`.
fn fleet3(home: &str) -> Fleet {
    let names = ["platform", "571", "demo"];
    Fleet {
        rows: names
            .iter()
            .enumerate()
            .map(|(at, name)| srow(name, at + 1, *name == home, question()))
            .collect(),
        home: Some(home.to_owned()),
    }
}

struct Shot {
    /// Full-width text rows.
    text: Vec<String>,
    /// Per-cell REVERSED flags (no-colour lift).
    rev: Vec<Vec<bool>>,
    /// Sidebar rule cell column (`width` when no sidebar drawn).
    rule: usize,
}

/// One render's inputs; a struct keeps `render` under the argument limit.
#[derive(Clone, Copy)]
struct Frame<'a> {
    width: u16,
    height: u16,
    fleet: &'a Fleet,
    model: &'a Model,
    entry: &'a SessionEntry,
    pair: &'a [String],
    agents: Option<&'a Facts>,
    overview: &'a Overview,
    composer: Composer<'a>,
    look: Option<Look>,
}

fn render(frame: Frame<'_>) -> Shot {
    let lane = Lane::default();
    let now = Timestamp::from_epoch(NOW);
    let screen = Screen {
        fleet: frame.fleet,
        model: frame.model,
        overview: frame.overview,
        selected: Some(frame.entry),
        pair: frame.pair,
        agents: frame.agents,
        lane: &lane,
        composer: frame.composer,
        look: frame.look,
        zone: None,
        now,
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, frame.width, frame.height));
    draw(&screen, &mut buf);
    let mut text = Vec::new();
    let mut rev_rows = Vec::new();
    for y in 0..frame.height {
        let mut row = String::new();
        let mut row_rev = Vec::new();
        for x in 0..frame.width {
            row.push_str(buf[(x, y)].symbol());
            row_rev.push(buf[(x, y)].modifier.contains(Modifier::REVERSED));
        }
        text.push(row.trim_end().to_owned());
        rev_rows.push(row_rev);
    }
    // Cell-index rule: first '│' cell on row 2 (the sidebar rule spans it).
    let cells: Vec<Vec<String>> = (0..frame.height)
        .map(|y| {
            (0..frame.width)
                .map(|x| buf[(x, y)].symbol().to_owned())
                .collect()
        })
        .collect();
    let rule = cells
        .get(2)
        .and_then(|row| row.iter().position(|cell| cell == "│"))
        .unwrap_or(frame.width as usize);
    Shot {
        text,
        rev: rev_rows,
        rule,
    }
}

/// Sidebar text: every row clipped to the rule column.
fn sidebar(shot: &Shot) -> Vec<String> {
    shot.text
        .iter()
        .map(|line| {
            line.chars()
                .take(shot.rule)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn entry(name: &str) -> SessionEntry {
    SessionEntry::new(name, Status::Running)
}

fn home_composer(home: &str) -> Composer<'_> {
    Composer::Home {
        home,
        view: None,
        draft: "",
    }
}

fn tokens(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

fn lines_with<'a>(lines: &'a [String], token: &str) -> Vec<&'a String> {
    lines
        .iter()
        .filter(|line| tokens(line).contains(&token))
        .collect()
}

// ---------------------------------------------------------------------------
// RED: single-line rows (A1)
// ---------------------------------------------------------------------------

/// RED A1: three sessions take rows 3,4,5 after the summary spacer;
/// the Agents heading sits at row 7;
/// no session row carries a second line (no tally `·`, no reason text).
#[test]
fn session_rows_are_single_line_with_heading_right_below() {
    let fleet = fleet3("platform");
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("platform"),
        look: icons_on(),
    });
    let side = sidebar(&shot);
    assert!(side[2].is_empty(), "summary spacer:\n{}", side[2]);
    assert!(side[3].contains("platform"), "row 3:\n{}", side[3]);
    assert!(side[4].contains("571"), "row 4:\n{}", side[4]);
    assert!(side[5].contains("demo"), "row 5:\n{}", side[5]);
    assert!(
        side[7].trim_start().starts_with("Agents"),
        "heading at row 7:\n{}",
        side[7]
    );
    for (y, line) in side.iter().enumerate().take(6).skip(3) {
        assert!(
            !line.contains('·'),
            "row {y} has no second-line text:\n{line}"
        );
        assert!(
            !line.contains("awaiting"),
            "row {y} carries no reason:\n{line}"
        );
    }
    assert!(
        !side.join("\n").contains("awaiting owner reply"),
        "no reason anywhere in the list"
    );
}

// ---------------------------------------------------------------------------
// RED: no UI index digit; numeric names stay (A2 draw)
// ---------------------------------------------------------------------------

/// RED A2: the `571` row shows `571` once; no other all-digit token (the old
/// positional index digit is gone).
#[test]
fn numeric_session_name_renders_once_with_no_index_digit() {
    let fleet = fleet3("platform");
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("platform"),
        look: icons_on(),
    });
    let side = sidebar(&shot);
    let line = lines_with(&side, "571");
    assert_eq!(line.len(), 1, "571 on exactly one row");
    let digits = tokens(line[0])
        .iter()
        .filter(|token| token.chars().all(|cell| cell.is_ascii_digit()))
        .count();
    assert_eq!(digits, 1, "no index digit beside the name:\n{}", line[0]);
}

// ---------------------------------------------------------------------------
// RED: home glyph (A3)
// ---------------------------------------------------------------------------

/// RED A3: the home row carries `◂`; the `this window` words are gone.
#[test]
fn home_row_carries_its_glyph_not_words() {
    let fleet = fleet3("demo");
    let model = Model::new(&fleet);
    let torn = entry("demo");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("demo"),
        look: icons_on(),
    });
    let side = sidebar(&shot);
    let home = lines_with(&side, "demo");
    assert_eq!(home.len(), 1);
    assert!(home[0].contains('◂'), "home glyph:\n{}", home[0]);
    assert!(
        !side.join("\n").contains("this window"),
        "no `this window` words"
    );
}

/// RED A3 icons-off: the home glyph is `<`.
#[test]
fn home_glyph_is_ascii_with_icons_off() {
    let fleet = fleet3("demo");
    let model = Model::new(&fleet);
    let torn = entry("demo");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 100,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("demo"),
        look: icons_off(),
    });
    let side = sidebar(&shot);
    let home = lines_with(&side, "demo");
    assert_eq!(home.len(), 1);
    assert!(home[0].contains('<'), "ascii home glyph:\n{}", home[0]);
    assert!(
        !side.join("\n").contains("this window"),
        "no `this window` words"
    );
}

// ---------------------------------------------------------------------------
// RED: tally on the session row; stopped stays one line (A4)
// ---------------------------------------------------------------------------

/// RED A4: the session's own row ends with its right-aligned tally.
#[test]
fn session_row_ends_with_its_tally() {
    let fleet = fleet3("platform");
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("platform"),
        look: icons_on(),
    });
    let side = sidebar(&shot);
    let row = lines_with(&side, "platform");
    assert_eq!(row.len(), 1);
    assert!(row[0].ends_with("●1"), "right-aligned tally:\n{}", row[0]);
}

/// RED A4: a stopped session keeps mark+name with `stopped` on its one row;
/// the old `not running` second line is gone.
#[test]
fn stopped_session_is_one_row() {
    let mut fleet = fleet3("platform");
    fleet.rows.push(Row {
        name: "archives".to_owned(),
        index: 4,
        mark: Mark::Dead,
        needy: false,
        counts: Counts::Stopped,
        line2: Line2::NotRunning {
            seats: 2,
            stopped_secs: Some(259_200),
        },
        home: false,
    });
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("platform"),
        look: icons_on(),
    });
    let side = sidebar(&shot);
    let row = lines_with(&side, "archives");
    assert_eq!(row.len(), 1, "stopped session on one row");
    assert!(row[0].contains("stopped"), "keeps `stopped`:\n{}", row[0]);
    assert!(
        !side.join("\n").contains("not running"),
        "no second-line status"
    );
}

// ---------------------------------------------------------------------------
// RED: single-line agent rows (A5)
// ---------------------------------------------------------------------------

/// RED A5: each agent is one row with mark, name, model, effort and state;
/// the client word is gone when a model is drawn; an unlisted ask seat is
/// one `needs you` row.
#[test]
fn agent_rows_are_single_line_without_client_or_reason_rows() {
    let fleet = fleet3("platform");
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let facts = seats();
    let overview = Overview {
        open: vec![Open {
            seat: "ghost".to_owned(),
            text: "ghost needs input".to_owned(),
            age_secs: Some(60),
        }],
        dir: None,
    };
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: Some(&facts),
        overview: &overview,
        composer: home_composer("platform"),
        look: icons_on(),
    });
    let side = sidebar(&shot);
    let lead = lines_with(&side, "lead");
    assert_eq!(lead.len(), 1, "lead on one row");
    for word in ["lead", "Opus 5.5", "high", "working"] {
        assert!(
            lead[0].contains(word),
            "lead row keeps {word}:\n{}",
            lead[0]
        );
    }
    let colead = lines_with(&side, "colead");
    assert_eq!(colead.len(), 1, "colead on one row");
    for word in ["colead", "gpt-6.1-sol", "waiting-user"] {
        assert!(
            colead[0].contains(word),
            "colead row keeps {word}:\n{}",
            colead[0]
        );
    }
    assert!(
        !side.join("\n").contains("atlas"),
        "client word dropped when a model is drawn"
    );
    let ghost = lines_with(&side, "ghost");
    assert_eq!(ghost.len(), 1, "unlisted ask seat on one row");
    assert!(
        ghost[0].contains("needs you"),
        "unlisted row says `needs you`:\n{}",
        ghost[0]
    );
}

// ---------------------------------------------------------------------------
// RED: shared chat highlights the session only (A6)
// ---------------------------------------------------------------------------

/// RED A6: with no agent selected the selected session is the only lifted
/// row; no agent row carries `▸`.
#[test]
fn shared_chat_lifts_only_the_selected_session() {
    let fleet = fleet3("platform");
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let facts = seats();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: Some(&facts),
        overview: &overview,
        composer: home_composer("platform"),
        look: theme_off(),
    });
    let side = sidebar(&shot);
    let lifted: Vec<usize> = side
        .iter()
        .enumerate()
        .filter(|(y, _)| shot.rev[*y].iter().take(shot.rule).any(|rev| *rev))
        .map(|(y, _)| y)
        .collect();
    assert_eq!(lifted.len(), 1, "exactly one lifted row: {lifted:?}");
    assert!(
        side[lifted[0]].contains("platform"),
        "the lifted row is the session:\n{}",
        side[lifted[0]]
    );
    assert_eq!(
        side.join("\n").matches('▸').count(),
        1,
        "no agent row carries `▸`"
    );
}

// ---------------------------------------------------------------------------
// RED: header reason segment (A12, list half is A1)
// ---------------------------------------------------------------------------

/// RED A12: an open ask names its seat and words in the chat header row.
#[test]
fn open_ask_reaches_the_header_row() {
    let fleet = fleet3("platform");
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let overview = Overview {
        open: vec![Open {
            seat: "colead".to_owned(),
            text: "awaiting owner reply".to_owned(),
            age_secs: Some(120),
        }],
        dir: None,
    };
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("platform"),
        look: icons_on(),
    });
    assert!(
        shot.text[0].contains("colead") && shot.text[0].contains("awaiting owner reply"),
        "header names seat + words:\n{}",
        shot.text[0]
    );
}

// ---------------------------------------------------------------------------
// GUARD: digits stay positional incl. numeric names; Esc goes home (A2 keys)
// ---------------------------------------------------------------------------

fn fleet10() -> Fleet {
    let names = [
        "platform",
        "571",
        "pipelines",
        "realtime",
        "docs",
        "demo",
        "console",
        "helpdesk",
        "svc-09",
        "svc-10",
    ];
    Fleet {
        rows: names
            .iter()
            .enumerate()
            .map(|(at, name)| Row {
                name: (*name).to_owned(),
                index: at + 1,
                mark: Mark::Idle,
                needy: false,
                counts: Counts::Unknown,
                line2: Line2::NoGoal,
                home: at == 0,
            })
            .collect(),
        home: Some("platform".to_owned()),
    }
}

/// GUARD A2: `1`-`9` select by row position; a numeric name never confuses
/// them. Esc returns to home.
#[test]
fn digits_select_by_position_and_esc_returns_home() {
    let fleet = fleet10();
    let mut model = Model::new(&fleet);
    assert_eq!(model.selected(), Some("platform"));
    assert_eq!(
        model.key(Key::Digit(2), &fleet, false),
        Act::Select("571".to_owned())
    );
    assert_eq!(model.selected(), Some("571"));

    let mut model = Model::new(&fleet);
    assert_eq!(
        model.key(Key::Digit(5), &fleet, false),
        Act::Select("docs".to_owned())
    );
    assert_eq!(model.selected(), Some("docs"));

    let mut model = Model::new(&fleet);
    assert_eq!(
        model.key(Key::Digit(9), &fleet, false),
        Act::Select("svc-09".to_owned())
    );

    let mut model = Model::new(&fleet);
    model.key(Key::Down, &fleet, false);
    model.key(Key::Down, &fleet, false);
    assert_eq!(model.selected(), Some("pipelines"));
    assert_eq!(
        model.key(Key::Esc, &fleet, false),
        Act::Select("platform".to_owned())
    );
    assert_eq!(model.selected(), Some("platform"));
}

// ---------------------------------------------------------------------------
// GUARD: composer address + viewing row survive (A9 baseline)
// ---------------------------------------------------------------------------

/// GUARD A9: the composer address names the session; the viewing row names
/// the seat. Both survive the slice unchanged.
#[test]
fn composer_address_and_viewing_row_survive() {
    let fleet = fleet3("demo");
    let model = Model::new(&fleet);
    let torn = entry("demo");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("demo"),
        look: icons_on(),
    });
    assert!(
        shot.text.join("\n").contains("to demo"),
        "address names the session"
    );
    let viewing = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: Composer::Viewing { seat: "colead" },
        look: icons_on(),
    });
    assert!(
        viewing.text.join("\n").contains("viewing colead"),
        "viewing row names the seat"
    );
}

// ---------------------------------------------------------------------------
// GUARD: Esc with peek open closes the peek, touching nothing else
// ---------------------------------------------------------------------------

/// GUARD (colead ordering): Esc in peek returns the chat before any cursor
/// handling; the selection stands.
#[test]
fn esc_with_peek_open_closes_the_peek_only() {
    let fleet = fleet3("platform");
    let mut model = Model::new(&fleet);
    model.open_peek(7);
    assert!(model.peek_open());
    assert_eq!(model.key(Key::Esc, &fleet, false), Act::Redraw);
    assert!(!model.peek_open());
    assert_eq!(model.selected(), Some("platform"));
}

// ---------------------------------------------------------------------------
// GUARD: seat keys keep their acts; client falls back when model is empty
// ---------------------------------------------------------------------------

/// GUARD A7/A8 keys: n/p/v/o still address seats through their acts.
#[test]
fn seat_keys_keep_their_acts() {
    let fleet = fleet3("platform");
    let mut model = Model::new(&fleet);
    assert_eq!(model.key(Key::SeatNext, &fleet, false), Act::SeatNext);
    assert_eq!(model.key(Key::SeatPrev, &fleet, false), Act::SeatPrev);
    assert_eq!(model.key(Key::Peek, &fleet, false), Act::Peek);
    assert_eq!(model.key(Key::Open, &fleet, false), Act::Open);
}

/// RED A5: with no model drawn the client fallback lands on the one row
/// (on base it sits on the separate facts row, so this fails there too).
#[test]
fn client_word_kept_when_no_model_drawn() {
    let fleet = fleet3("platform");
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let facts = Facts::Seats {
        id: "$7".to_owned(),
        agents: vec![agent("solo", "idle", "atlas", "", "")],
    };
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: Some(&facts),
        overview: &overview,
        composer: home_composer("platform"),
        look: icons_on(),
    });
    let side = sidebar(&shot);
    let solo = lines_with(&side, "solo");
    assert_eq!(solo.len(), 1);
    assert!(solo[0].contains("atlas"), "client fallback:\n{}", solo[0]);
}

// ---------------------------------------------------------------------------
// GUARD: scroll/window geometry + tiny frames (A10); quiet header (A12)
// ---------------------------------------------------------------------------

fn fleet60(home: &str, needy_last: bool) -> Fleet {
    Fleet {
        rows: (1..=60)
            .map(|index| {
                let name = format!("svc-{index:02}");
                Row {
                    name,
                    index,
                    mark: Mark::Working,
                    needy: needy_last && index == 60,
                    counts: Counts::Unknown,
                    line2: Line2::NoGoal,
                    home: false,
                }
            })
            .collect(),
        home: Some(home.to_owned()),
    }
}

/// GUARD A10: a 60-session fleet keeps the selection visible, counts its
/// `more` row, and keeps the needy arrow.
#[test]
fn long_fleet_keeps_selection_counts_more_and_marks_needy() {
    let fleet = fleet60("svc-01", true);
    let model = Model::new(&fleet);
    let torn = entry("svc-01");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 160,
        height: 43,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("svc-01"),
        look: icons_on(),
    });
    let side = sidebar(&shot);
    assert!(side[0].contains('↓'), "needy arrow kept:\n{}", side[0]);
    assert!(
        side[0].contains("60") || side[1].contains("60"),
        "fleet counted"
    );
    assert!(side.join("\n").contains("more"), "`more` row counted");

    let fleet = fleet60("svc-60", false);
    let model = Model::new(&fleet);
    let torn = entry("svc-60");
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 160,
        height: 43,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("svc-60"),
        look: icons_on(),
    });
    assert!(
        sidebar(&shot).join("\n").contains("svc-60"),
        "last selection stays visible"
    );
}

/// GUARD A10: below the sidebar floor there is no sidebar.
#[test]
fn tiny_frame_draws_no_sidebar() {
    let fleet = fleet3("platform");
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 89,
        height: 19,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("platform"),
        look: icons_on(),
    });
    let all = shot.text.join("\n");
    assert!(!all.contains("sessions"), "no summary band");
    assert!(!all.contains("Agents"), "no agents heading");
}

/// GUARD A12: with no open ask the header carries no reason segment.
#[test]
fn quiet_header_carries_no_reason() {
    let fleet = fleet3("platform");
    let model = Model::new(&fleet);
    let torn = entry("platform");
    let pair: Vec<String> = Vec::new();
    let overview = overview::of(&torn, None, None, Timestamp::from_epoch(NOW));
    let shot = render(Frame {
        width: 150,
        height: 30,
        fleet: &fleet,
        model: &model,
        entry: &torn,
        pair: &pair,
        agents: None,
        overview: &overview,
        composer: home_composer("platform"),
        look: icons_on(),
    });
    let chat: String = shot.text[0].chars().skip(shot.rule).collect::<String>();
    assert!(!chat.contains('⚠'), "no reason segment:\n{}", shot.text[0]);
}
