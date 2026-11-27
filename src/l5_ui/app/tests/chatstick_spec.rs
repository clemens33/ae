//! Frozen acceptance for chatstick: a chat view follows new entries only
//! while it stands at the bottom.
//! Oracle: brief-chatstick rulings 1-4 (lead, 2026-11-27) and the owner's
//! words in it: scrolled up, an arrival never moves the view, also while
//! writing; at the bottom the view follows. Every check reads the drawn
//! frame: which entries show, and the hint row's words.

use super::o7app_units::{at as frame_at, orchestrated};
use super::*;
use crate::theme::Look;

const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
const W: u16 = 160;
const H: u16 = 45;

/// Entries `entry000` up to `entry<count-1>`, one turn each.
fn entries(count: i64) -> Vec<Item> {
    (0..count)
        .map(|at| said(&format!("entry{at:03}"), at))
        .collect()
}

/// Session `name`'s lane, read as `ID` at `seq`, now `count` entries long.
fn arrive(app: &mut App, name: &str, seq: u64, count: i64) {
    let Answer::View(mut read) = view(name, ID, seq, "") else {
        unreachable!()
    };
    read.lane.items = entries(count);
    app.answer(Answer::View(read));
}

/// The frame at `width` x `height`, as text.
fn shown_at(app: &mut App, width: u16, height: u16) -> String {
    text(&frame_at(width, height, app))
}

fn shown(app: &mut App) -> String {
    shown_at(app, W, H)
}

/// The entry numbers the frame shows, top to bottom; each at most once.
fn visible(shown: &str) -> Vec<u32> {
    let seen: Vec<u32> = shown
        .lines()
        .flat_map(|line| {
            line.match_indices("entry")
                .filter_map(|(at, _)| line.get(at + 5..at + 8)?.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect();
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "an entry drawn twice:\n{shown}");
    seen
}

fn first(shown: &str) -> u32 {
    *visible(shown)
        .first()
        .unwrap_or_else(|| panic!("no entry drawn:\n{shown}"))
}

fn last(shown: &str) -> u32 {
    *visible(shown)
        .last()
        .unwrap_or_else(|| panic!("no entry drawn:\n{shown}"))
}

/// The hint's words: the chat's part of the row that says newer turns
/// wait below, right of the sidebar's border where one is drawn.
fn hint_line(shown: &str) -> Option<&str> {
    let line = shown.lines().find(|line| line.contains(" newer turn"))?;
    Some(line.rsplit_once('│').map_or(line, |(_, chat)| chat).trim())
}

/// The count the hint gives, `None` when no hint is drawn.
fn hint(shown: &str) -> Option<usize> {
    let line = hint_line(shown)?;
    let (before, _) = line.split_once(" newer turn")?;
    before.split_whitespace().last()?.parse().ok()
}

fn wheel(app: &mut App, up: bool, (column, row): (u16, u16)) {
    let kind = if up {
        MouseKind::WheelUp
    } else {
        MouseKind::WheelDown
    };
    let _ = app.mouse(Mouse { kind, column, row });
}

/// A cell inside the main chat column beside the sidebar.
const MAIN: (u16, u16) = (150, 20);
/// A cell inside a full orchestrator panel.
const PANEL: (u16, u16) = (80, 20);

/// Four notches up: twelve rows, the four newest turns of three rows each.
fn scroll_up(app: &mut App, at: (u16, u16)) -> String {
    let _ = shown(app);
    for _ in 0..4 {
        wheel(app, true, at);
    }
    let up = shown(app);
    assert_eq!(hint(&up), Some(4), "scrolled up four turns:\n{up}");
    up
}

/// The home `api` with forty entries, drawn once, browsing.
fn browsing(root: &Root) -> App {
    let (mut app, _reader) = housed(root);
    arrive(&mut app, "api", 1, 40);
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 39, "a fresh view stands at the bottom");
    app
}

/// [`browsing`], writing the draft `draft`.
fn drafting(root: &Root) -> (App, Instant) {
    let mut app = browsing(root);
    let typed = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"draft".to_vec()), typed), Some(()));
    (app, typed)
}

/// Where the press on the drawn hint's words lands.
fn hint_press(shown: &str) -> Mouse {
    let (row, column) = shown
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            let byte = line.find(" newer turn")?;
            let words = line[..byte].trim_end().rfind(' ').map_or(0, |at| at + 1);
            Some((row, line[..words].chars().count()))
        })
        .unwrap_or_else(|| panic!("a hint to press:\n{shown}"));
    Mouse {
        kind: MouseKind::Click,
        column: u16::try_from(column + 1).expect("column"),
        row: u16::try_from(row).expect("row"),
    }
}

/// Ruling 1: at the bottom, each arrival comes into view.
#[test]
fn an_anchored_chat_follows_every_arrival() {
    let root = Root::new("chatstick-follow");
    let mut app = browsing(&root);
    for (seq, count) in [(2, 43), (3, 44), (4, 50)] {
        arrive(&mut app, "api", seq, count);
        let drawn = shown(&mut app);
        let newest = u32::try_from(count - 1).expect("count");
        assert_eq!(last(&drawn), newest, "the newest entry shows:\n{drawn}");
        assert_eq!(hint(&drawn), None, "no hint at the bottom:\n{drawn}");
    }
}

/// Ruling 1 and 2: scrolled up, arrivals leave every drawn entry where it
/// stood, and the hint counts the turns below, the arrivals included.
#[test]
fn a_scrolled_up_chat_stays_put_and_the_hint_counts_the_arrivals() {
    let root = Root::new("chatstick-stay");
    let mut app = browsing(&root);
    let up = scroll_up(&mut app, MAIN);
    let before = visible(&up);
    assert!(!before.contains(&39), "the newest is off the view:\n{up}");
    for (seq, count, below) in [(2, 43, 7), (3, 45, 9)] {
        arrive(&mut app, "api", seq, count);
        let drawn = shown(&mut app);
        assert_eq!(visible(&drawn), before, "the view stays put:\n{drawn}");
        assert_eq!(hint(&drawn), Some(below), "the hint counts:\n{drawn}");
    }
}

/// Ruling 1: the same while the human writes below, the draft kept.
#[test]
fn a_scrolled_up_chat_stays_put_while_the_human_writes() {
    let root = Root::new("chatstick-write");
    let (mut app, typed) = drafting(&root);
    let up = scroll_up(&mut app, MAIN);
    let before = visible(&up);
    arrive(&mut app, "api", 2, 43);
    let drawn = shown(&mut app);
    assert_eq!(visible(&drawn), before, "the view stays put:\n{drawn}");
    assert_eq!(hint(&drawn), Some(7), "the hint counts:\n{drawn}");
    assert_eq!(app.compose(Key::Text(b"!".to_vec()), typed), Some(()));
    arrive(&mut app, "api", 3, 44);
    let drawn = shown(&mut app);
    assert_eq!(visible(&drawn), before, "typing moves nothing:\n{drawn}");
    assert!(app.composing(), "still writing");
    assert_eq!(drafted(&app), "draft!", "the draft is kept");
}

/// Ruling 1 and 2: End returns to the bottom, and the view follows again.
#[test]
fn end_returns_to_the_bottom_and_the_chat_follows_again() {
    let root = Root::new("chatstick-end");
    let mut app = browsing(&root);
    let _ = scroll_up(&mut app, MAIN);
    arrive(&mut app, "api", 2, 43);
    let _ = shown(&mut app);
    assert!(take_keys(&mut app, vec![(Key::End, Instant::now())]).is_some());
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 42, "End shows the newest:\n{drawn}");
    assert_eq!(hint(&drawn), None, "no hint at the bottom:\n{drawn}");
    arrive(&mut app, "api", 3, 46);
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 45, "anchored again, it follows:\n{drawn}");
}

/// Ruling 2: a press on the hint returns to the bottom while writing, the
/// draft kept, and the view follows again.
#[test]
fn a_press_on_the_hint_returns_to_the_bottom_while_writing() {
    let root = Root::new("chatstick-press");
    let (mut app, _typed) = drafting(&root);
    let _ = scroll_up(&mut app, MAIN);
    arrive(&mut app, "api", 2, 43);
    let drawn = shown(&mut app);
    assert!(app.click(hint_press(&drawn)), "a press on the hint redraws");
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 42, "the newest shows:\n{drawn}");
    assert_eq!(hint(&drawn), None, "no hint at the bottom:\n{drawn}");
    assert!(app.composing(), "still writing");
    assert_eq!(drafted(&app), "draft", "the draft is kept");
    arrive(&mut app, "api", 3, 45);
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 44, "anchored again, it follows:\n{drawn}");
}

/// Ruling 1: scrolling down to the end anchors the view again.
#[test]
fn wheeling_down_to_the_end_anchors_the_chat_again() {
    let root = Root::new("chatstick-wheel");
    let mut app = browsing(&root);
    let _ = scroll_up(&mut app, MAIN);
    arrive(&mut app, "api", 2, 43);
    let mut drawn = shown(&mut app);
    for _ in 0..12 {
        if hint(&drawn).is_none() {
            break;
        }
        wheel(&mut app, false, MAIN);
        drawn = shown(&mut app);
    }
    assert_eq!(last(&drawn), 42, "the wheel reached the newest:\n{drawn}");
    arrive(&mut app, "api", 3, 45);
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 44, "anchored again, it follows:\n{drawn}");
}

/// `root`'s home `api` recorded on a named tmux server, as an ask needs.
fn served(root: &Root) {
    let dir = root.0.join("sessions/api");
    let bytes = crate::meta::read_bytes(&dir).expect("meta");
    let meta = String::from_utf8(bytes).expect("utf-8 meta");
    let meta = format!("{meta}tmux_server_kind=name\ntmux_server=fixture\n");
    std::fs::write(dir.join("meta"), meta).expect("meta written");
}

/// Ruling 3: the human's own send, once the app admits it into flight,
/// returns the view to the bottom before the next frame.
#[test]
fn the_humans_own_send_returns_to_the_bottom() {
    let root = Root::new("chatstick-send");
    let mut app = browsing(&root);
    served(&root);
    let typed = writing(&mut app);
    assert_eq!(app.compose(Key::Text(b"hello".to_vec()), typed), Some(()));
    app.carry = std::sync::Arc::new(|_| String::new());
    let _ = scroll_up(&mut app, MAIN);
    arrive(&mut app, "api", 2, 43);
    let _ = shown(&mut app);
    assert_eq!(app.compose(Key::Enter, typed), Some(()));
    assert_eq!(app.queued.len(), 1, "the line was taken as a send");
    assert!(app.deliver(), "the queued ask meets the admission");
    assert_eq!(app.note().as_deref(), Some("sending to lead…"));
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 42, "the send shows the newest:\n{drawn}");
    assert_eq!(hint(&drawn), None, "no hint at the bottom:\n{drawn}");
}

/// Ruling 4: a resize keeps an anchored view at the bottom.
#[test]
fn a_resize_keeps_an_anchored_chat_at_the_bottom() {
    let root = Root::new("chatstick-resize-anchor");
    let mut app = browsing(&root);
    for (width, height) in [(W, 30), (120, 30), (W, H)] {
        let drawn = shown_at(&mut app, width, height);
        assert_eq!(last(&drawn), 39, "{width}x{height}:\n{drawn}");
    }
    let _ = shown_at(&mut app, W, 30);
    arrive(&mut app, "api", 2, 42);
    let drawn = shown_at(&mut app, W, 30);
    assert_eq!(last(&drawn), 41, "it follows at the new size:\n{drawn}");
}

/// Ruling 4: a resize keeps a scrolled-up view's first entry, and an
/// arrival after it still moves nothing.
#[test]
fn a_resize_keeps_the_first_entry_of_a_scrolled_up_chat() {
    let root = Root::new("chatstick-resize-up");
    let mut app = browsing(&root);
    let up = scroll_up(&mut app, MAIN);
    let top = first(&up);
    for height in [32, 38, H] {
        let drawn = shown_at(&mut app, W, height);
        assert_eq!(first(&drawn), top, "height {height}:\n{drawn}");
        assert!(hint(&drawn).is_some(), "still scrolled up:\n{drawn}");
    }
    let _ = shown_at(&mut app, W, 32);
    arrive(&mut app, "api", 2, 43);
    let drawn = shown_at(&mut app, W, 32);
    assert_eq!(first(&drawn), top, "an arrival moves nothing:\n{drawn}");
}

/// Ruling 4: a width change that rewraps every turn keeps the first entry
/// of a scrolled-up view: the anchor is the entry, not a row distance.
#[test]
fn a_reflow_keeps_the_first_entry_of_a_scrolled_up_chat() {
    let root = Root::new("chatstick-reflow");
    let (mut app, _reader) = housed(&root);
    let long = |count: i64| -> Vec<Item> {
        (0..count)
            .map(|at| {
                said(
                    &format!("entry{at:03} {}", "wrapping words ".repeat(12)),
                    at,
                )
            })
            .collect()
    };
    let Answer::View(mut read) = view("api", ID, 1, "") else {
        unreachable!()
    };
    read.lane.items = long(40);
    app.answer(Answer::View(read));
    let _ = shown(&mut app);
    for _ in 0..4 {
        wheel(&mut app, true, MAIN);
    }
    let up = shown(&mut app);
    assert!(hint(&up).is_some(), "scrolled up:\n{up}");
    let top = first(&up);
    for width in [130, 110, W] {
        let drawn = shown_at(&mut app, width, H);
        assert_eq!(first(&drawn), top, "width {width}:\n{drawn}");
        assert!(hint(&drawn).is_some(), "still scrolled up:\n{drawn}");
        if width == 110 {
            let (narrow, wide) = (visible(&drawn).len(), visible(&up).len());
            assert!(narrow < wide, "the turns rewrapped taller:\n{drawn}");
        }
    }
    let _ = shown_at(&mut app, 110, H);
    let Answer::View(mut read) = view("api", ID, 2, "") else {
        unreachable!()
    };
    read.lane.items = long(43);
    app.answer(Answer::View(read));
    let drawn = shown_at(&mut app, 110, H);
    assert_eq!(first(&drawn), top, "an arrival moves nothing:\n{drawn}");
}

/// Ruling 2: under icons off the hint wears no arrow glyph, a plain `v`.
#[test]
fn the_hint_has_an_ascii_form_under_icons_off() {
    for icons in [true, false] {
        let root = Root::new("chatstick-ascii");
        let mut app = browsing(&root);
        let look = Look {
            icons,
            ..Look::DEFAULT
        };
        app.looks.insert("api".to_owned(), (Some(look), None));
        let _ = scroll_up(&mut app, MAIN);
        arrive(&mut app, "api", 2, 43);
        let drawn = shown(&mut app);
        let words = hint_line(&drawn).unwrap_or_else(|| panic!("a hint:\n{drawn}"));
        if icons {
            assert!(words.starts_with("↓ 7 newer turns"), "{words:?}");
        } else {
            assert!(words.is_ascii(), "plain ASCII: {words:?}");
            assert!(words.starts_with("v 7 newer turns"), "{words:?}");
        }
    }
}

/// Ruling 5: the orchestrator panel is a live chat too: anchored it
/// follows; scrolled up it stays put and counts; its hint returns it.
#[test]
fn the_orchestrator_panel_sticks_and_follows_like_the_chat() {
    let root = Root::new("chatstick-panel");
    let (mut app, _reader) = orchestrated(&root, true);
    arrive(&mut app, ORCH, 1, 40);
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 39, "a fresh panel stands at the bottom");
    arrive(&mut app, ORCH, 2, 41);
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 40, "anchored, it follows:\n{drawn}");
    let up = scroll_up(&mut app, PANEL);
    let before = visible(&up);
    arrive(&mut app, ORCH, 3, 44);
    let drawn = shown(&mut app);
    assert_eq!(visible(&drawn), before, "the panel stays put:\n{drawn}");
    assert_eq!(hint(&drawn), Some(7), "the hint counts:\n{drawn}");
    // The full panel's targets reach a press through the mouse route.
    assert!(app.mouse(hint_press(&drawn)), "a press on the hint redraws");
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 43, "the newest shows:\n{drawn}");
    arrive(&mut app, ORCH, 4, 46);
    let drawn = shown(&mut app);
    assert_eq!(last(&drawn), 45, "anchored again, it follows:\n{drawn}");
}
