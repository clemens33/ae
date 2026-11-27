//! Frozen supplementary acceptance for chatstick, beside the frozen r2
//! `chatstick_spec`: the page keys while writing, the independent anchors of
//! the main chat and a side orchestrator panel, and a panel send.
//! Oracle: brief-chatstick rulings 1-4, docs/app.md as the candidate states
//! them (`PgUp` and `PgDn` scroll the chat a page while writing, End moves the
//! draft cursor, the two chats scroll independently), and navigator review
//! findings 1-3 of review-20261127T091104Z-efd61554. Every check reads the
//! drawn frame.

use super::o7app_units::{at as frame_at, border_of, orchestrated, rule_of};
use super::*;

const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const W: u16 = 160;
const H: u16 = 45;
const ROW: u16 = 20;

/// Entries `<prefix>000` up to `<prefix><count-1>`, one turn each.
fn entries(prefix: &str, count: i64) -> Vec<Item> {
    (0..count)
        .map(|at| said(&format!("{prefix}{at:03}"), at))
        .collect()
}

/// Session `name`'s lane, read as `ID` at `seq`, now `count` entries long.
fn arrive(app: &mut App, name: &str, seq: u64, prefix: &str, count: i64) {
    let Answer::View(mut read) = view(name, ID, seq, "") else {
        unreachable!()
    };
    read.lane.items = entries(prefix, count);
    app.answer(Answer::View(read));
}

/// The cells `xs` of row `y`, as text.
fn segment(buf: &Buffer, y: u16, xs: std::ops::Range<u16>) -> String {
    xs.map(|x| buf[(x, y)].symbol()).collect()
}

/// The `<prefix>NNN` entry numbers the cells `xs` show, top to bottom; each
/// at most once.
fn visible(buf: &Buffer, xs: &std::ops::Range<u16>, prefix: &str) -> Vec<u32> {
    let width = prefix.len();
    let seen: Vec<u32> = (0..buf.area.height)
        .flat_map(|y| {
            let line = segment(buf, y, xs.clone());
            line.match_indices(prefix)
                .filter_map(|(at, _)| line.get(at + width..at + width + 3)?.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect();
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    let drawn = text(buf);
    assert_eq!(unique.len(), seen.len(), "an entry drawn twice:\n{drawn}");
    seen
}

fn last(buf: &Buffer, xs: &std::ops::Range<u16>, prefix: &str) -> u32 {
    let drawn = text(buf);
    *visible(buf, xs, prefix)
        .last()
        .unwrap_or_else(|| panic!("no {prefix} entry drawn:\n{drawn}"))
}

/// The count the hint inside the cells `xs` gives, `None` when none is drawn.
fn hint(buf: &Buffer, xs: &std::ops::Range<u16>) -> Option<usize> {
    (0..buf.area.height).find_map(|y| {
        let line = segment(buf, y, xs.clone());
        let (before, _) = line.split_once(" newer turn")?;
        before.split_whitespace().last()?.parse().ok()
    })
}

/// Where a press on the words of the hint inside the cells `xs` lands.
fn hint_press(buf: &Buffer, xs: &std::ops::Range<u16>) -> Mouse {
    let drawn = text(buf);
    let (row, column) = (0..buf.area.height)
        .find_map(|y| {
            let line = segment(buf, y, xs.clone());
            let byte = line.find(" newer turn")?;
            let words = line[..byte].trim_end().rfind(' ').map_or(0, |at| at + 1);
            let cells = u16::try_from(line[..words].chars().count()).ok()?;
            Some((y, xs.start + cells))
        })
        .unwrap_or_else(|| panic!("a hint to press:\n{drawn}"));
    Mouse {
        kind: MouseKind::Click,
        column: column + 1,
        row,
    }
}

fn wheel_up(app: &mut App, column: u16, notches: usize) {
    for _ in 0..notches {
        let kind = MouseKind::WheelUp;
        let _ = app.mouse(Mouse {
            kind,
            column,
            row: ROW,
        });
    }
}

fn press(app: &mut App, key: Key) {
    assert!(
        take_keys(app, vec![(key, Instant::now())]).is_some(),
        "the app stays open"
    );
}

/// `root`'s session `name` recorded on a named tmux server, as an ask needs.
fn served(root: &Root, name: &str) {
    let dir = root.0.join("sessions").join(name);
    let bytes = crate::meta::read_bytes(&dir).expect("meta");
    let meta = String::from_utf8(bytes).expect("utf-8 meta");
    let meta = format!("{meta}tmux_server_kind=name\ntmux_server=fixture\n");
    std::fs::write(dir.join("meta"), meta).expect("meta written");
}

/// Finding 1: while writing, `PgUp` moves the chat exactly the page a browse
/// `PgUp` moves it, `PgDn` returns it to the bottom, and End stays the draft
/// cursor's key: it moves no chat row and the next edit lands at the end.
#[test]
fn page_keys_scroll_the_chat_a_browse_page_while_writing_and_end_stays_the_cursor() {
    let root = Root::new("chatstick-more-pages");
    let (mut app, _reader) = housed(&root);
    arrive(&mut app, "api", 1, "entry", 60);
    let all = 0..W;
    let buf = frame_at(W, H, &mut app);
    assert_eq!(last(&buf, &all, "entry"), 59, "a fresh view at the bottom");
    press(&mut app, Key::PageUp);
    let browse_page = visible(&frame_at(W, H, &mut app), &all, "entry");
    assert!(!browse_page.contains(&59), "a browse page went up");
    press(&mut app, Key::End);
    let buf = frame_at(W, H, &mut app);
    assert_eq!(last(&buf, &all, "entry"), 59, "End in browse: the bottom");
    let typed = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"draft".to_vec()), typed), Some(()));
    let _ = frame_at(W, H, &mut app);
    press(&mut app, Key::PageUp);
    let buf = frame_at(W, H, &mut app);
    let drawn = text(&buf);
    assert_eq!(
        visible(&buf, &all, "entry"),
        browse_page,
        "PgUp while writing moves one browse page:\n{drawn}"
    );
    press(&mut app, Key::Home);
    press(&mut app, Key::Text(b"<".to_vec()));
    assert_eq!(drafted(&app), "<draft", "Home put the cursor at the start");
    press(&mut app, Key::End);
    press(&mut app, Key::Text(b"!".to_vec()));
    assert_eq!(drafted(&app), "<draft!", "End put the cursor at the end");
    let buf = frame_at(W, H, &mut app);
    let drawn = text(&buf);
    assert_eq!(
        visible(&buf, &all, "entry"),
        browse_page,
        "Home and End move no chat row:\n{drawn}"
    );
    press(&mut app, Key::PageDown);
    let buf = frame_at(W, H, &mut app);
    let drawn = text(&buf);
    assert_eq!(last(&buf, &all, "entry"), 59, "PgDn returns:\n{drawn}");
    assert_eq!(hint(&buf, &all), None, "no hint at the bottom:\n{drawn}");
    assert!(app.composing(), "still writing");
    assert_eq!(drafted(&app), "<draft!", "the draft is kept");
}

/// The side panel's frame and its two columns: the main chat between the
/// sidebar's rule and the panel's border, the panel right of the border.
fn split(app: &mut App) -> (Buffer, std::ops::Range<u16>, std::ops::Range<u16>) {
    let buf = frame_at(W, H, app);
    let (rule, border) = (rule_of(&buf), border_of(&buf));
    assert!(rule + 20 < border && border + 20 < W, "two chat columns");
    (buf, rule + 1..border, border + 1..W)
}

/// Both chats with forty entries, drawn once at the bottom, then each
/// scrolled up four notches: what each shows.
fn both_scrolled(app: &mut App) -> (Vec<u32>, Vec<u32>) {
    arrive(app, "api", 1, "main", 40);
    arrive(app, ORCH, 1, "orch", 40);
    let (buf, main, panel) = split(app);
    assert_eq!(last(&buf, &main, "main"), 39, "main at the bottom");
    assert_eq!(last(&buf, &panel, "orch"), 39, "panel at the bottom");
    wheel_up(app, u16::midpoint(main.start, main.end), 4);
    wheel_up(app, u16::midpoint(panel.start, panel.end), 4);
    let (buf, main, panel) = split(app);
    let drawn = text(&buf);
    assert_eq!(hint(&buf, &main), Some(4), "main scrolled up:\n{drawn}");
    assert_eq!(hint(&buf, &panel), Some(4), "panel scrolled up:\n{drawn}");
    (visible(&buf, &main, "main"), visible(&buf, &panel, "orch"))
}

/// Finding 2: the main chat and a side panel keep their own anchors: both
/// scrolled up stay put on arrivals to both lanes, and the panel's hint
/// returns the panel alone, which then follows while the main chat stays.
#[test]
fn the_main_chat_and_a_side_panel_keep_independent_anchors() {
    let root = Root::new("chatstick-more-split");
    let (mut app, _reader) = orchestrated(&root, false);
    let (main_up, panel_up) = both_scrolled(&mut app);
    arrive(&mut app, "api", 2, "main", 43);
    arrive(&mut app, ORCH, 2, "orch", 44);
    let (buf, main, panel) = split(&mut app);
    let drawn = text(&buf);
    assert_eq!(visible(&buf, &main, "main"), main_up, "main:\n{drawn}");
    assert_eq!(visible(&buf, &panel, "orch"), panel_up, "panel:\n{drawn}");
    assert_eq!(hint(&buf, &main), Some(7), "main counts:\n{drawn}");
    assert_eq!(hint(&buf, &panel), Some(8), "panel counts:\n{drawn}");
    assert!(
        app.mouse(hint_press(&buf, &panel)),
        "the panel hint redraws"
    );
    let (buf, main, panel) = split(&mut app);
    let drawn = text(&buf);
    assert_eq!(
        last(&buf, &panel, "orch"),
        43,
        "the panel returned:\n{drawn}"
    );
    assert_eq!(
        visible(&buf, &main, "main"),
        main_up,
        "main stays:\n{drawn}"
    );
    arrive(&mut app, "api", 3, "main", 45);
    arrive(&mut app, ORCH, 3, "orch", 46);
    let (buf, main, panel) = split(&mut app);
    let drawn = text(&buf);
    assert_eq!(
        last(&buf, &panel, "orch"),
        45,
        "the panel follows:\n{drawn}"
    );
    assert_eq!(
        visible(&buf, &main, "main"),
        main_up,
        "main stays:\n{drawn}"
    );
    assert_eq!(hint(&buf, &main), Some(9), "main counts:\n{drawn}");
}

/// Finding 3: a send the app admits from a scrolled-up side panel returns
/// the panel to the bottom; the scrolled-up main chat beside it stays put.
#[test]
fn a_panel_send_returns_the_panel_alone_to_the_bottom() {
    let root = Root::new("chatstick-more-send");
    let (mut app, _reader) = orchestrated(&root, false);
    served(&root, ORCH);
    app.carry = std::sync::Arc::new(|_| String::new());
    let (main_up, _) = both_scrolled(&mut app);
    arrive(&mut app, "api", 2, "main", 43);
    arrive(&mut app, ORCH, 2, "orch", 43);
    let _ = split(&mut app);
    press(&mut app, Key::Enter);
    assert_eq!(app.writer_name(), Some(ORCH), "the panel writes");
    press(&mut app, Key::Text(b"hello".to_vec()));
    let (buf, _, panel) = split(&mut app);
    let drawn = text(&buf);
    assert!(
        hint(&buf, &panel).is_some(),
        "writing moved nothing:\n{drawn}"
    );
    let before = last(&buf, &panel, "orch");
    assert!(before < 42, "the newest is off the panel:\n{drawn}");
    press(&mut app, Key::Enter);
    assert_eq!(app.queued.len(), 1, "the line was taken as a send");
    assert!(app.deliver(), "the queued ask meets the admission");
    assert!(app.flight.is_some(), "the app admitted the send");
    let (buf, main, panel) = split(&mut app);
    let drawn = text(&buf);
    assert_eq!(
        last(&buf, &panel, "orch"),
        42,
        "the panel returned:\n{drawn}"
    );
    assert_eq!(hint(&buf, &panel), None, "no panel hint:\n{drawn}");
    assert_eq!(
        visible(&buf, &main, "main"),
        main_up,
        "main stays:\n{drawn}"
    );
    assert_eq!(hint(&buf, &main), Some(7), "main counts:\n{drawn}");
}
