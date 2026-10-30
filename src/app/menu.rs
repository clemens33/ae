//! The session menu `ae app` opens on a card — `m` on the selection, a right
//! press on any card — offering Start on a stopped session and Stop on a
//! running one, the Stop question its `Stop…` asks, and the read-only
//! Activity and Memos dialogs every card offers. Pure: what a card offers,
//! what a key or press does, and how all of it is drawn. The app proves and
//! acts; the reader thread reads.

use std::cell::Cell;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Modifier;
use ratatui_core::text::Span;

use super::draw::{Hit, Layout, Paint, cells, put};
use super::fleet::Facts;
use crate::board::terminal_text;
use crate::console::input::Key;
use crate::digest::Status;
use crate::event_text::clip_head;
use crate::lifecycle::StopExpectation;
use crate::session_menu::{self, CellRow};

/// The Stop question's text width: its longest fixed line.
const QUESTION_INNER: u16 = 48;
/// The fewest cells a dialog's text keeps before the window is too small.
const DIALOG_INNER_MIN: u16 = 12;
/// The question's two buttons, in their drawn order.
const BUTTONS: [(Choice, &str); 2] = [(Choice::Cancel, "[ Cancel ]"), (Choice::Stop, "[ Stop ]")];

/// What a frame drew of one session's card, frozen with it: a menu opened on
/// that frame acts on this and on nothing a later read changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Card {
    pub(super) name: String,
    /// The incarnation the card was drawn from.
    pub(super) uuid: String,
    /// The tmux id its roster fact was published with.
    pub(super) id: Option<String>,
    /// Why nothing on the card may act: the first drawn state that refuses.
    pub(super) refused: Option<String>,
    /// The session is stopped: the card offers Start, not Stop.
    pub(super) stopped: bool,
    /// This app's own pane is in it: known once the Stop question read the
    /// clients.
    pub(super) home: bool,
    /// The orchestrator: its menu offers nothing.
    pub(super) orchestrator: bool,
}

impl Card {
    /// `name`'s card as a frame drew it: `uuid` the incarnation it painted,
    /// `sole` the one its record named exactly once in that same read,
    /// `status` and `facts` its painted states.
    pub(super) fn new(
        name: &str,
        uuid: &str,
        sole: Option<&str>,
        status: Option<Status>,
        facts: Option<&Facts>,
    ) -> Self {
        let refused = if uuid.is_empty() || sole != Some(uuid) {
            Some(format!("{name}'s record names no single session_id"))
        } else {
            match (status, facts) {
                (Some(Status::Unknown) | None, _) => Some(format!("{name}'s state is unknown")),
                (_, Some(Facts::OtherServer)) => Some(format!("{name} is on another tmux server")),
                _ => None,
            }
        };
        Self {
            name: name.to_owned(),
            uuid: uuid.to_owned(),
            id: match facts {
                Some(Facts::Seats { id, .. }) => Some(id.clone()),
                _ => None,
            },
            refused,
            stopped: status == Some(Status::Stopped),
            home: false,
            orchestrator: name == crate::orchestrator::ORCHESTRATOR_SESSION,
        }
    }

    /// What the menu offers, in its drawn order.
    pub(super) fn items(&self) -> &'static [Item] {
        match (self.orchestrator, self.stopped) {
            (true, _) => &[],
            (false, true) => &[Item::Start, Item::Activity, Item::Memos],
            (false, false) => &[Item::Stop, Item::Activity, Item::Memos],
        }
    }

    /// Why nothing on this card may act; `None` when it may.
    pub(super) fn refusal(&self) -> Option<&str> {
        self.refused.as_deref()
    }
}

/// One menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Item {
    /// Resume the stopped session through the picker's own launch path.
    Start,
    /// The Stop question.
    Stop,
    /// The session's recent activity, read-only.
    Activity,
    /// The session's latest memo per topic, read-only.
    Memos,
}

impl Item {
    /// The verb a refusal line names.
    pub(super) fn word(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Activity => "Activity",
            Self::Memos => "Memos",
        }
    }

    /// The dialog this item opens: `None` for the two that act.
    pub(super) fn dialog(self) -> Option<Dialog> {
        match self {
            Self::Start | Self::Stop => None,
            Self::Activity => Some(Dialog::Activity),
            Self::Memos => Some(Dialog::Memos),
        }
    }

    fn label(self, icons: bool) -> &'static str {
        match (self, icons) {
            (Self::Start, _) => "s  Start",
            (Self::Stop, true) => "s  Stop…",
            (Self::Stop, false) => "s  Stop...",
            (Self::Activity, true) => "a  Activity…",
            (Self::Activity, false) => "a  Activity...",
            (Self::Memos, true) => "o  Memos…",
            (Self::Memos, false) => "o  Memos...",
        }
    }
}

/// A read-only dialog over a session's records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Dialog {
    Activity,
    Memos,
}

impl Dialog {
    /// Its title word.
    fn word(self) -> &'static str {
        match self {
            Self::Activity => "Activity",
            Self::Memos => "Memos",
        }
    }

    /// The fixed columns the tmux session menu aligns this dialog's rows by.
    fn caps(self) -> &'static [usize] {
        match self {
            Self::Activity => session_menu::ACTIVITY_FIXED_CAPS,
            Self::Memos => session_menu::MEMO_FIXED_CAPS,
        }
    }
}

/// The one modal open over the frame: it takes every key and press first.
#[derive(Debug)]
pub(super) enum Modal {
    Menu(Menu),
    Question(Question),
    Records(Records),
}

/// A card's menu.
#[derive(Debug)]
pub(super) struct Menu {
    pub(super) card: Card,
    /// The focused item's place in [`Card::items`].
    pub(super) focus: usize,
    /// When a frame showing it whole was written; `None` until then.
    pub(super) shown_at: Option<Instant>,
    /// The dialog read out for this menu: the menu stays on screen until the
    /// rows land, and takes no second item meanwhile.
    pub(super) reading: Option<Asked>,
}

/// The read a menu waits on: its generation and the dialog it will open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Asked {
    pub(super) generation: u64,
    pub(super) dialog: Dialog,
}

impl Menu {
    pub(super) fn new(card: Card) -> Self {
        Self {
            card,
            focus: 0,
            shown_at: None,
            reading: None,
        }
    }

    /// The step an item takes: none while a dialog read is out.
    fn act(&self, item: Item) -> Step {
        if self.reading.is_some() {
            return Step::Kept;
        }
        Step::Act(item)
    }

    /// One key, or one `byte` of the typed run `key` is.
    fn input(&mut self, key: &Key, byte: u8) -> Step {
        let items = self.card.items();
        let by = match (key, byte) {
            (Key::Text(_), b's') => {
                return items.first().map_or(Step::Kept, |&item| self.act(item));
            }
            (Key::Text(_), b'a') => {
                return (items.iter().find(|item| **item == Item::Activity))
                    .map_or(Step::Kept, |&item| self.act(item));
            }
            (Key::Text(_), b'o') => {
                return (items.iter().find(|item| **item == Item::Memos))
                    .map_or(Step::Kept, |&item| self.act(item));
            }
            (Key::Text(_), b'm' | b'q') => return Step::Close,
            (Key::Text(_), b'j') | (Key::Down | Key::Tab, _) => 1,
            (Key::Text(_), b'k') | (Key::Up, _) => items.len().saturating_sub(1),
            (Key::Enter, _) => {
                return (items.get(self.focus)).map_or(Step::Kept, |&item| self.act(item));
            }
            _ => return Step::Kept,
        };
        let to = (self.focus + by) % items.len().max(1);
        refocus(&mut self.focus, to)
    }
}

/// A dialog over the rows the tmux session menu's own folds made of a
/// session's records, scrolled by the keys.
#[derive(Debug)]
pub(super) struct Records {
    dialog: Dialog,
    name: String,
    rows: Vec<CellRow>,
    /// The first row drawn, and how many rows the last frame had room for:
    /// [`paint_records`] writes both, so a key scrolls within what was drawn.
    top: Cell<usize>,
    room: Cell<usize>,
    /// When a frame showing it whole was written; `None` until then.
    pub(super) shown_at: Option<Instant>,
}

impl Records {
    pub(super) fn new(dialog: Dialog, name: &str, rows: Vec<CellRow>) -> Self {
        Self {
            dialog,
            name: name.to_owned(),
            rows,
            top: Cell::new(0),
            room: Cell::new(0),
            shown_at: None,
        }
    }

    /// One key, or one `byte` of the typed run `key` is: the rows scroll, `q`
    /// closes.
    fn input(&mut self, key: &Key, byte: u8) -> Step {
        let (top, room) = (self.top.get(), self.room.get().max(1));
        let last = self.rows.len().saturating_sub(room);
        let to = match (key, byte) {
            (Key::Text(_), b'q') => return Step::Close,
            (Key::Text(_), b'j') | (Key::Down | Key::Tab, _) => top + 1,
            (Key::Text(_), b'k') | (Key::Up, _) => top.saturating_sub(1),
            (Key::PageDown, _) => top + room,
            (Key::PageUp, _) => top.saturating_sub(room),
            (Key::Home, _) => 0,
            (Key::End, _) => last,
            _ => return Step::Kept,
        }
        .min(last);
        if to == top {
            return Step::Kept;
        }
        self.top.set(to);
        Step::Moved
    }
}

/// The Stop question, with everything its answer proves captured.
#[derive(Debug)]
pub(super) struct Question {
    pub(super) card: Card,
    /// The whole expectation the guarded Stop owner proves under its lock.
    pub(super) expect: StopExpectation,
    pub(super) focus: Choice,
    /// When a frame showing it whole was written; `None` until then.
    pub(super) shown_at: Option<Instant>,
}

impl Question {
    /// The question `card`'s Stop… asks, `expect` captured: Cancel focused,
    /// not yet on screen.
    pub(super) fn new(card: Card, expect: StopExpectation) -> Self {
        Self {
            card,
            expect,
            focus: Choice::Cancel,
            shown_at: None,
        }
    }

    /// One key, or one `byte` of the typed run `key` is.
    fn input(&mut self, key: &Key, byte: u8) -> Step {
        let to = match (key, byte, self.focus) {
            (Key::Text(_), b'y', _) | (Key::Enter, _, Choice::Stop) => return Step::Affirm,
            (Key::Text(_), b'n' | b'q', _) | (Key::Enter, _, Choice::Cancel) => return Step::Close,
            (Key::Text(_), b's', _) | (Key::Left | Key::Right | Key::Tab, _, Choice::Cancel) => {
                Choice::Stop
            }
            (Key::Text(_), b'c', _) | (Key::Left | Key::Right | Key::Tab, _, Choice::Stop) => {
                Choice::Cancel
            }
            _ => return Step::Kept,
        };
        refocus(&mut self.focus, to)
    }
}

/// Move `focus` to `to`: `Kept` when it is there already.
fn refocus<T: PartialEq>(focus: &mut T, to: T) -> Step {
    if *focus == to {
        return Step::Kept;
    }
    *focus = to;
    Step::Moved
}

/// The question's two answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Cancel,
    Stop,
}

/// What one key or press did to the modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// It was the modal's, and changed nothing.
    Kept,
    /// The focus moved.
    Moved,
    /// Dismissed: nothing acts.
    Close,
    /// The menu item, activated.
    Act(Item),
    /// The question answered Stop.
    Affirm,
}

impl Step {
    /// The modal is done with: the rest of the key's read is spent.
    pub(super) fn ends(self) -> bool {
        !matches!(self, Self::Kept | Self::Moved)
    }
}

impl Modal {
    /// When a frame showing it whole was written.
    pub(super) fn shown_at(&self) -> Option<Instant> {
        match self {
            Self::Menu(menu) => menu.shown_at,
            Self::Question(question) => question.shown_at,
            Self::Records(records) => records.shown_at,
        }
    }

    /// The first frame showing it whole was written at `at`; a repaint never
    /// moves that.
    pub(super) fn stamp(&mut self, at: Instant) {
        let shown = match self {
            Self::Menu(menu) => &mut menu.shown_at,
            Self::Question(question) => &mut question.shown_at,
            Self::Records(records) => &mut records.shown_at,
        };
        shown.get_or_insert(at);
    }

    /// What the hint row calls it.
    pub(super) fn word(&self) -> &'static str {
        match self {
            Self::Menu(_) => "menu",
            Self::Question(_) => "Stop question",
            Self::Records(_) => "dialog",
        }
    }

    /// One key: a typed run steps byte by byte until one ends the modal, any
    /// other key once.
    pub(super) fn key(&mut self, key: &Key) -> Step {
        let bytes: &[u8] = match key {
            Key::Escape | Key::Interrupt => return Step::Close,
            Key::Text(bytes) => bytes.as_slice(),
            _ => &[0],
        };
        let mut moved = Step::Kept;
        for byte in bytes {
            let step = match self {
                Self::Menu(menu) => menu.input(key, *byte),
                Self::Question(question) => question.input(key, *byte),
                Self::Records(records) => records.input(key, *byte),
            };
            if step.ends() {
                return step;
            }
            if step == Step::Moved {
                moved = step;
            }
        }
        moved
    }

    /// A press on the drawn target `hit`: a right press anywhere, and a left
    /// one outside the modal, close it.
    pub(super) fn press(&self, right: bool, hit: Option<&Hit>) -> Step {
        match (self, right, hit) {
            (_, false, Some(Hit::Modal)) => Step::Kept,
            (Self::Menu(menu), false, Some(Hit::Item(at))) => {
                (menu.card.items().get(*at)).map_or(Step::Kept, |item| menu.act(*item))
            }
            (Self::Question(_), false, Some(Hit::Item(at))) => match BUTTONS.get(*at) {
                Some((Choice::Stop, _)) => Step::Affirm,
                Some((Choice::Cancel, _)) => Step::Close,
                None => Step::Kept,
            },
            _ => Step::Close,
        }
    }
}

/// Paint `modal` over the frame in `buf` and record its targets in `layout`
/// ahead of everything under them. `Err` is the hint row's line for a modal
/// that does not fit whole; nothing is painted then.
pub(super) fn paint(
    modal: &Modal,
    paint: Paint,
    icons: bool,
    buf: &mut Buffer,
    layout: &mut Layout,
) -> Result<(), String> {
    match modal {
        Modal::Menu(menu) => paint_menu(menu, paint, icons, buf, layout),
        Modal::Question(question) => paint_question(question, paint, buf, layout),
        Modal::Records(records) => paint_records(records, paint, buf, layout),
    }
}

/// The menu, under its card where it fits and over it otherwise.
fn paint_menu(
    menu: &Menu,
    paint: Paint,
    icons: bool,
    buf: &mut Buffer,
    layout: &mut Layout,
) -> Result<(), String> {
    let area = buf.area;
    let card = &menu.card;
    let name = terminal_text(&card.name);
    let now = format!("now {}x{}", area.width, area.height);
    let Some(at) = layout.card_at(&card.name) else {
        return Err(format!(
            "menu closed: {name}'s card is not on screen ({now})"
        ));
    };
    let items = card.items();
    // A refusal is every one the card has: one dim line says it.
    let why = card.refusal().map(terminal_text);
    let labels = items.iter().map(|item| item.label(icons));
    let want = (labels.clone().chain(why.as_deref()))
        .map(width)
        .chain([width(&name) + 2])
        .max()
        .unwrap_or(0);
    let x = at.x + 1;
    let room = area.width.saturating_sub(x + 4);
    let height = cells(items.len() + usize::from(why.is_some())) + 2;
    let y = if at.bottom() + height < area.height {
        Some(at.bottom())
    } else {
        at.y.checked_sub(height)
    };
    let least = labels.clone().map(width).max().unwrap_or(0);
    let (Some(y), true) = (y, room >= least) else {
        return Err(format!("too small for the session menu ({now})"));
    };
    let inner = want.min(room);
    let rect = Rect::new(x, y, inner + 4, height);
    boxed(
        buf,
        rect,
        &clip_head(&format!(" {name} "), usize::from(inner)),
        paint,
    );
    layout.overlay(buf, rect, Hit::Modal);
    for (place, label) in labels.enumerate() {
        let row = y + 1 + cells(place);
        let style = if place == menu.focus {
            paint.selected()
        } else if card.refusal().is_some() && items.get(place).is_some_and(|i| i.dialog().is_none())
        {
            paint.fg(|p| p.dim)
        } else {
            paint.fg(|p| p.text)
        };
        put(buf, x + 2, row, label, inner, style);
        layout.overlay(buf, Rect::new(x + 1, row, inner + 2, 1), Hit::Item(place));
    }
    if let Some(why) = why {
        let row = y + 1 + cells(items.len());
        let line = clip_head(&why, usize::from(inner));
        put(buf, x + 2, row, &line, inner, paint.fg(|p| p.dim));
    }
    Ok(())
}

/// The dialog, centred over the frame above its keys row: the rows the tmux
/// session menu's dialog aligns, at this window's width, scrolled from the
/// first row the keys left. A frame too small for it paints nothing.
fn paint_records(
    records: &Records,
    paint: Paint,
    buf: &mut Buffer,
    layout: &mut Layout,
) -> Result<(), String> {
    let area = buf.area;
    let lines: Vec<String> = session_menu::aligned_lines(
        &records.rows,
        records.dialog.caps(),
        usize::from(area.width),
    )
    .iter()
    .map(|line| terminal_text(line))
    .collect();
    let title = format!(
        " {} · {} ",
        records.dialog.word(),
        terminal_text(&records.name)
    );
    let room = usize::from(area.height.saturating_sub(5));
    if room < 2 || area.width < DIALOG_INNER_MIN + 4 {
        return Err(format!(
            "too small for the {} dialog (needs {}x7, now {}x{})",
            records.dialog.word(),
            DIALOG_INNER_MIN + 4,
            area.width,
            area.height
        ));
    }
    let across = lines.iter().map(|line| width(line)).max().unwrap_or(0);
    let inner = across.max(width(&title)).min(area.width - 4);
    // A longer dialog gives its last row to the scroll line.
    let scrolls = lines.len() > room;
    let shown = if scrolls { room - 1 } else { lines.len() };
    let top = records.top.get().min(lines.len().saturating_sub(shown));
    records.top.set(top);
    records.room.set(shown);
    let (wide, high) = (inner + 4, cells(shown + usize::from(scrolls)) + 2);
    let x = (area.width - wide) / 2;
    let rect = Rect::new(x, (area.height - 1 - high) / 2, wide, high);
    boxed(buf, rect, &clip_head(&title, usize::from(inner)), paint);
    layout.overlay(buf, rect, Hit::Modal);
    let (text, dim) = (paint.fg(|p| p.text), paint.fg(|p| p.dim));
    for (place, (row, line)) in records
        .rows
        .iter()
        .zip(&lines)
        .skip(top)
        .take(shown)
        .enumerate()
    {
        let style = if matches!(row, CellRow::Gap(_)) {
            dim
        } else {
            text
        };
        put(buf, x + 2, rect.y + 1 + cells(place), line, inner, style);
    }
    if scrolls {
        let line = format!(
            "{}-{} of {} · j/k scroll · q closes",
            top + 1,
            top + shown,
            lines.len()
        );
        put(buf, x + 2, rect.bottom() - 2, &line, inner, dim);
    }
    Ok(())
}

/// The question, centred over the frame above its keys row.
fn paint_question(
    question: &Question,
    paint: Paint,
    buf: &mut Buffer,
    layout: &mut Layout,
) -> Result<(), String> {
    let area = buf.area;
    let inner = QUESTION_INNER;
    let name = terminal_text(&question.card.name);
    let mut lines = vec![
        "Its agents are interrupted; their state is kept.".to_owned(),
        clip_head(
            &format!("To resume it later: ae {name}"),
            usize::from(inner),
        ),
    ];
    if question.card.home {
        lines.push("This app runs in it: this app closes.".to_owned());
    }
    let (wide, height) = (inner + 4, cells(lines.len()) + 4);
    if area.width < wide || area.height <= height {
        return Err(format!(
            "too small for the Stop question (needs {wide}x{}, now {}x{})",
            height + 1,
            area.width,
            area.height
        ));
    }
    let x = (area.width - wide) / 2;
    let rect = Rect::new(x, (area.height - 1 - height) / 2, wide, height);
    let title = format!(" Stop {}? ", clip_head(&name, usize::from(inner - 8)));
    boxed(buf, rect, &title, paint);
    layout.overlay(buf, rect, Hit::Modal);
    let text = paint.fg(|p| p.text);
    for (place, line) in lines.iter().enumerate() {
        put(buf, x + 2, rect.y + 1 + cells(place), line, inner, text);
    }
    let row = rect.bottom() - 2;
    let span: u16 = BUTTONS
        .iter()
        .map(|(_, label)| width(label) + 3)
        .sum::<u16>()
        - 3;
    let mut at = x + 2 + (inner - span) / 2;
    for (place, (choice, label)) in BUTTONS.iter().enumerate() {
        let style = if question.focus == *choice {
            paint.selected()
        } else {
            text
        };
        let end = put(buf, at, row, label, width(label), style);
        layout.overlay(buf, Rect::new(at, row, end - at, 1), Hit::Item(place));
        at = end + 3;
    }
    Ok(())
}

/// Clear `rect` to the base ground and rule its edge, `title` set in the top.
fn boxed(buf: &mut Buffer, rect: Rect, title: &str, paint: Paint) {
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            buf[(x, y)].reset();
        }
    }
    buf.set_style(rect, paint.ground(|p| p.base));
    let rule = paint.fg(|p| p.border);
    let across = "─".repeat(usize::from(rect.width.saturating_sub(2)));
    let (x, bottom) = (rect.x, rect.bottom() - 1);
    let mut edge = |x, y, text: &str, max| put(buf, x, y, text, max, rule);
    edge(x, rect.y, &format!("┌{across}┐"), rect.width);
    edge(x, bottom, &format!("└{across}┘"), rect.width);
    for y in rect.y + 1..bottom {
        edge(x, y, "│", 1);
        edge(rect.right() - 1, y, "│", 1);
    }
    let heading = paint.fg(|p| p.title).add_modifier(Modifier::BOLD);
    let room = rect.width.saturating_sub(4);
    put(buf, x + 2, rect.y, title, room, heading);
}

/// The cells `text` takes.
fn width(text: &str) -> u16 {
    cells(Span::raw(text).width())
}
