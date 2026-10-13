//! The cells: the sidebar, its tabs and the chat column, into one buffer.
//!
//! PURE. Colour is spent on meaning only and comes from the drawn look's
//! palette; `theme = off`, or no look, draws no colour at all and marks the
//! selection with bold and reverse instead. Every write is bounded by the
//! buffer, so a pane of any size draws without a panic.

use std::fmt::Write as _;
use std::ops::Range;

use ratatui_core::buffer::{Buffer, CellWidth};
use ratatui_core::layout::{Position, Rect};
use ratatui_core::style::{Color, Modifier, Style};
use ratatui_core::text::{Line, Span};

use super::fleet::{Counts, Facts, Fleet, Line2, Row, summary};
use super::model::{Drag, Edge, Model, SettingsTab, Split};
use super::overview::{Open, Overview};
use super::peek;
use super::select::{self, Map, Region};
use super::settings::{AboutFacts, ConfigView, InstructionsView, SettingsBodies};
use crate::brief::age;
use crate::console::input::View;
use crate::console::input::{Mouse, ROWS_MAX};
use crate::console::lane::Lane;
use crate::console::view;
use crate::console::wrap::wrap;
use crate::digest::{SessionEntry, Status};
use crate::event_text::{clip_head, clip_tail};
use crate::quota::view::Tone;
use crate::theme::{Look, Mark, Palette};
use crate::time::Timestamp;

/// Everything one frame draws.
#[derive(Debug, Clone, Copy)]
pub struct Screen<'a> {
    pub fleet: &'a Fleet,
    pub model: &'a Model,
    pub overview: &'a Overview,
    pub selected: Option<&'a SessionEntry>,
    /// The home lead pair, main first.
    pub pair: &'a [String],
    pub agents: Option<&'a Facts>,
    pub lane: &'a Lane,
    pub composer: Composer<'a>,
    /// `None` draws no colour.
    pub look: Option<Look>,
    /// The viewer's zone, the raw `[+-]HHMM` tmux answered; read exactly as
    /// the chat reads it, so stamps stay UTC when nothing is drawn.
    pub zone: Option<&'a str>,
    pub now: Timestamp,
}

/// The composer's lines.
#[derive(Debug, Clone, Copy)]
pub enum Composer<'a> {
    /// The selected session, writable: typing reaches its lead pair.
    Home {
        home: &'a str,
        speaker: &'a str,
        /// `Some` while composing.
        view: Option<&'a View>,
        draft: &'a str,
    },
    /// The selected session, not writable.
    ReadOnly { why: &'a str },
    /// The selected session, its entry refused: HELD until Esc, ^C, a click
    /// or an Enter that tries again.
    Held { why: &'a str },
    /// No session selected.
    NoHome,
}

/// The smallest pane the app draws in at all.
const MIN: (u16, u16) = (40, 8);
/// The smallest pane that keeps the sidebar.
const SIDEBAR_MIN: (u16, u16) = (90, 20);
/// The first row of the session list.
const LIST_TOP: u16 = 4;
/// The row the frame gave back at the top: the list keeps the share it had,
/// and the tab body takes the row.
const TOP_GIVEN: u16 = 1;
/// The sidebar's first content column.
const LEFT: u16 = 2;
/// Where a session row's name starts.
const NAME: u16 = 6;
/// The narrowest a drag leaves the sidebar: the tab row still keeps a gap
/// between the `Agents NN` heading and its rule, and a name beside its counts.
const SIDEBAR_FLOOR: u16 = 30;
/// The chat columns a drag leaves right of the rule.
const CHAT_FLOOR: u16 = 40;
/// The tab body rows a drag leaves above the floor.
const BODY_FLOOR: u16 = 3;
/// The lane rows a growing composer leaves: one wheel notch.
const LANE_FLOOR: u16 = 3;

/// A click target recorded where its cells are drawn.
#[derive(Debug, Clone)]
pub(crate) enum Hit {
    Session(String),
    /// A drawn seat row: moves the highlight to that seat.
    Seat(String),
    Compose,
    /// The keys row's gear: toggles the settings overlay.
    Settings,
    /// A drawn Settings tab title.
    SettingsTab(SettingsTab),
    /// The overlay's drawn close label.
    SettingsClose,
    /// The peek panel's drawn close label.
    PeekClose,
    /// A drawn chat turn, by its fingerprint: marks it, or unmarks it.
    Turn(u64),
    /// The open modal's item or button, by its place in it.
    Item(usize),
    /// The open modal's own cells: a press there is the modal's.
    Modal,
    /// A drawn fold row, by its turn's fold key: folds or unfolds the turn.
    Fold(u64),
}

/// A border as drawn: its edge, where it stands, its own cells and the cells
/// that grab it.
#[derive(Debug)]
struct Border {
    edge: Edge,
    at: u16,
    cells: Rect,
    grab: Rect,
}

/// The last frame's input geometry and chat scroll bounds.
#[derive(Debug, Default)]
pub(crate) struct Layout {
    targets: Vec<(Rect, Hit)>,
    borders: Vec<Border>,
    area: Rect,
    chat: Rect,
    pub page_rows: usize,
    /// How far back the chat can scroll: exact once `complete`, else only
    /// as far as this frame produced.
    pub max_scroll: usize,
    /// The frame produced the lane's oldest row.
    pub complete: bool,
    /// The turns the frame produced, newest first: fingerprint, the rows
    /// between its last row and the newest row, and its row count.
    pub turns: Vec<(u64, usize, usize)>,
    /// The turns with a fold row: fingerprint, then fold key.
    pub folds: Vec<(u64, u64)>,
    /// The cell the terminal cursor shows: the draft's, while writing.
    pub cursor: Option<Position>,
    /// Body rows the open full-pane panel (settings or peek) shows at once.
    pub settings_page_rows: usize,
    /// How far the open panel's body can scroll.
    pub settings_max_scroll: usize,
    /// The session list's cells, the first session it drew, and the
    /// furthest first session it could draw.
    list: Rect,
    pub list_start: Option<usize>,
    pub list_max: usize,
    /// The tab body's cells, the first row it drew, and the furthest.
    body: Rect,
    pub body_top: Option<usize>,
    pub body_max: usize,
    /// What a drag may select in this frame.
    pub(crate) map: Map,
    /// The seats the tab body drew, in drawn order, and the one it highlighted.
    pub seats: Vec<String>,
    pub focused: Option<String>,
    /// Each card the frame drew, as the app read it then: a menu opened on
    /// this frame acts on that and on nothing a later read changed.
    pub(super) cards: std::collections::BTreeMap<String, super::menu::Card>,
}

impl Layout {
    fn record(&mut self, buf: &Buffer, area: Rect, target: Hit) {
        self.targets.push((area.intersection(buf.area), target));
    }

    /// Record `target` over `area` ahead of every target under it: an
    /// overlay's cells are its own.
    pub(super) fn overlay(&mut self, buf: &Buffer, area: Rect, target: Hit) {
        self.targets
            .insert(0, (area.intersection(buf.area), target));
    }

    /// The sessions whose cards the frame drew.
    pub(super) fn sessions(&self) -> impl Iterator<Item = &str> {
        self.targets.iter().filter_map(|(_, target)| match target {
            Hit::Session(name) => Some(name.as_str()),
            _ => None,
        })
    }

    /// The frame painted a modal whole.
    pub(super) fn shows_modal(&self) -> bool {
        (self.targets.iter()).any(|(_, target)| matches!(target, Hit::Modal))
    }

    /// Where the frame drew `name`'s card.
    pub(super) fn card_at(&self, name: &str) -> Option<Rect> {
        (self.targets.iter()).find_map(|(area, target)| {
            matches!(target, Hit::Session(at) if at == name).then_some(*area)
        })
    }

    pub fn hit(&self, mouse: Mouse) -> Option<Hit> {
        self.targets.iter().find_map(|(area, target)| {
            area.contains((mouse.column, mouse.row).into())
                .then(|| target.clone())
        })
    }

    pub fn in_chat(&self, mouse: Mouse) -> bool {
        self.chat.contains((mouse.column, mouse.row).into())
    }

    pub(crate) fn in_list(&self, mouse: Mouse) -> bool {
        self.list.contains((mouse.column, mouse.row).into())
    }

    pub(crate) fn in_body(&self, mouse: Mouse) -> bool {
        self.body.contains((mouse.column, mouse.row).into())
    }

    /// Let a drag select in `area` (clipped to the frame): `panel`'s cells.
    fn selectable(&mut self, buf: &Buffer, panel: select::Panel, area: Rect) {
        let area = area.intersection(buf.area);
        if !area.is_empty() {
            self.map.regions.push(Region { panel, area });
        }
    }

    /// Let a drag select in the lane: `rows` rows below `top`, from `left`.
    /// The `bars` among the `shown` rows (from row `y0`) drew a mark bar in
    /// their first cell, which reads as a blank.
    fn lane(
        &mut self,
        buf: &Buffer,
        (left, top, room, rows): (u16, u16, u16, usize),
        bars: &[usize],
        (shown, y0): (Range<usize>, u16),
    ) {
        self.selectable(
            buf,
            select::Panel::Lane,
            Rect::new(left, top, room, cells(rows)),
        );
        for row in bars.iter().filter(|row| shown.contains(*row)) {
            let bar = Rect::new(left, y0 + cells(row - shown.start), 1, 1);
            self.map.blanks.push(bar.intersection(buf.area));
        }
    }

    /// Record a border at `at` over `cells`, grabbed within a cell of it.
    fn border(&mut self, buf: &Buffer, edge: Edge, at: u16, cells: Rect) {
        let grab = match edge {
            Edge::Sidebar => Rect::new(cells.x.saturating_sub(1), cells.y, 3, cells.height),
            Edge::List => Rect::new(cells.x, cells.y.saturating_sub(1), cells.width, 3),
        };
        self.borders.push(Border {
            edge,
            at,
            cells: cells.intersection(buf.area),
            grab: grab.intersection(buf.area),
        });
    }

    /// The border a press grabs, and where it stands: none where a click
    /// target is drawn; a border's own cells before the cells beside it.
    pub(crate) fn grab(&self, mouse: Mouse) -> Option<(Edge, u16)> {
        if self.hit(mouse).is_some() {
            return None;
        }
        let cell = (mouse.column, mouse.row).into();
        let on = |border: &&Border| border.cells.contains(cell);
        let near = |border: &&Border| border.grab.contains(cell);
        let border = (self.borders.iter().find(on)).or_else(|| self.borders.iter().find(near))?;
        Some((border.edge, border.at))
    }

    /// Whether the frame drew `edge`.
    pub(crate) fn shows(&self, edge: Edge) -> bool {
        self.borders.iter().any(|border| border.edge == edge)
    }

    /// The pane the frame was drawn in.
    pub(crate) fn area(&self) -> Rect {
        self.area
    }

    /// Strip every target, border and scroll bound the open settings overlay
    /// covers, keeping only the gear that toggles it: drawing over cells
    /// alone would leave hidden clicks and drags live.
    pub(crate) fn strip_for_settings(&mut self) {
        self.strip(|hit| matches!(hit, Hit::Settings));
    }

    /// [`Layout::strip_for_settings`] for the peek panel, which keeps no
    /// target: its own close label is recorded after.
    pub(crate) fn strip_for_peek(&mut self) {
        self.strip(|_| false);
    }

    fn strip(&mut self, keep: impl Fn(&Hit) -> bool) {
        self.targets.retain(|(_, hit)| keep(hit));
        self.borders.clear();
        self.chat = Rect::default();
        (self.list, self.body) = (Rect::default(), Rect::default());
        (self.list_start, self.body_top) = (None, None);
        (self.list_max, self.body_max) = (0, 0);
        (self.seats, self.focused) = (Vec::new(), None);
        self.page_rows = 0;
        self.max_scroll = 0;
        self.complete = true;
        self.turns.clear();
        self.folds.clear();
        self.cursor = None;
        self.settings_page_rows = 0;
        self.settings_max_scroll = 0;
        self.map = Map::default();
    }
}

/// The colours one frame draws in: a palette, or none.
#[derive(Debug, Clone, Copy)]
pub(super) struct Paint(Option<Palette>);

impl Paint {
    pub(super) fn of(look: Option<&Look>) -> Self {
        Self(look.filter(|look| look.drawn).map(|look| look.palette))
    }

    /// A foreground from the palette, or none.
    pub(super) fn fg(self, token: impl Fn(&Palette) -> &'static str) -> Style {
        self.0.map_or_else(Style::new, |palette| {
            Style::new().fg(colour(token(&palette)))
        })
    }

    /// A ground from the palette, or none.
    pub(super) fn ground(self, token: impl Fn(&Palette) -> &'static str) -> Style {
        self.0.map_or_else(Style::new, |palette| {
            Style::new().bg(colour(token(&palette)))
        })
    }

    /// A mark's accent.
    fn mark(self, mark: Mark) -> Style {
        self.fg(|palette| palette.accent(mark))
    }

    /// The focused seat's row `area`: the selected ground, the dim words in
    /// the selected ink so only it and the accents sit on it; else reverse video.
    fn lift(self, buf: &mut Buffer, area: Rect) {
        let Some(palette) = self.0 else {
            buf.set_style(area, Style::new().add_modifier(Modifier::REVERSED));
            return;
        };
        buf.set_style(area, Style::new().bg(colour(palette.selected)));
        let (dim, ink) = (colour(palette.dim), colour(palette.selected_ink));
        for position in area.intersection(buf.area).positions() {
            let cell = &mut buf[position];
            if cell.fg == dim {
                cell.fg = ink;
            }
        }
    }

    /// The selected name: the title hue, or bold and reverse with no colour.
    pub(super) fn selected(self) -> Style {
        match self.0 {
            Some(_) => self.fg(|p| p.title).add_modifier(Modifier::BOLD),
            None => Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        }
    }

    /// A border's rule: the border hue, or while it is dragged the title hue
    /// in bold, or reversed with no colour.
    fn rule(self, dragged: bool) -> Style {
        match (dragged, self.0) {
            (false, _) => self.fg(|p| p.border),
            (true, Some(_)) => self.fg(|p| p.title).add_modifier(Modifier::BOLD),
            (true, None) => Style::new().add_modifier(Modifier::REVERSED),
        }
    }
}

fn colour(token: &str) -> Color {
    let [red, green, blue] = crate::theme::rgb(token);
    Color::Rgb(red, green, blue)
}

/// One frame's shared facts.
struct Ctx<'s, 'a> {
    screen: &'s Screen<'a>,
    paint: Paint,
    icons: bool,
    wait: Wait<'s>,
}

/// What `ae app` has not read yet, drawn as `loading`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Wait<'a> {
    /// The fleet: the sidebar list says `loading` and no tab shows.
    pub(crate) fleet: bool,
    /// The selection's lane: one dim chat row says `loading`.
    pub(crate) lane: bool,
    /// The lane's turns' fingerprints, in item order: with none, no turn is
    /// marked or clickable.
    pub(crate) keys: &'a [u64],
    /// The same turns' fold keys.
    pub(crate) folds: &'a [u64],
    /// Which of them show whole; with none, nothing folds.
    pub(crate) open: &'a [bool],
    /// Which of them are asks that went through.
    pub(crate) sent: &'a [bool],
}

/// Draw `screen` into `buf`, whose area starts at the origin — the app hands
/// it the terminal's whole frame. The answer is how many pages back the chat
/// can scroll in this frame, which bounds the model's scroll: the lane's
/// whole height once the frame reached its oldest row, else at least a page
/// past the window.
pub fn draw(screen: &Screen<'_>, buf: &mut Buffer) -> usize {
    let layout = draw_with_layout(screen, Wait::default(), buf);
    layout.max_scroll.div_ceil(layout.page_rows.max(1))
}

/// [`draw`], answering what a drag may select in the frame instead of its
/// scroll bound.
#[must_use]
pub fn draw_map(screen: &Screen<'_>, buf: &mut Buffer) -> Map {
    draw_with_layout(screen, Wait::default(), buf).map
}

/// Draw and retain only the geometry this frame actually showed.
pub(crate) fn draw_with_layout(screen: &Screen<'_>, wait: Wait<'_>, buf: &mut Buffer) -> Layout {
    let area = buf.area;
    let mut layout = Layout {
        area,
        ..Layout::default()
    };
    let ctx = Ctx {
        screen,
        paint: Paint::of(screen.look.as_ref()),
        icons: screen.look.is_none_or(|look| look.icons),
        wait,
    };
    let dim = ctx.paint.fg(|p| p.dim);
    if area.width < MIN.0 || area.height < MIN.1 {
        let line = format!(
            "ae app needs at least {}x{} (now {}x{})",
            MIN.0, MIN.1, area.width, area.height
        );
        // By word onto the rows there are, so the pane's own size stays legible.
        let (mut y, mut row) = (area.y, String::new());
        for word in line.split(' ') {
            if !row.is_empty() && row.len() + 1 + word.len() > usize::from(area.width) {
                put(buf, area.x, y, &row, area.width, dim);
                (y, row) = (y.saturating_add(1), String::new());
            }
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(word);
        }
        put(buf, area.x, y, &row, area.width, dim);
        return layout;
    }
    // The frames' grounds: the chat's ink everywhere, the rule's own column
    // included, and the panel under the sidebar alone.
    buf.set_style(area, ctx.paint.ground(|p| p.ink));
    let (height, width) = (area.height, area.width);
    let split = screen.model.split();
    let mut fallback = None;
    let chat = if let Some(rule) = sidebar_width(area, split) {
        buf.set_style(Rect::new(0, 0, rule, height), ctx.paint.ground(|p| p.base));
        let cells = Rect::new(rule, 0, 1, height);
        layout.border(buf, Edge::Sidebar, rule, cells);
        sidebar(&ctx, buf, rule, &mut layout);
        let border = ctx.paint.rule(dragged(screen.model, Edge::Sidebar));
        for y in 0..height {
            put(buf, rule, y, "│", 1, border);
        }
        rule + 1..width
    } else {
        fallback = Some(format!(
            "  sidebar needs {}x{} (now {width}x{height}) ",
            SIDEBAR_MIN.0, SIDEBAR_MIN.1
        ));
        0..width
    };
    layout.chat = Rect::new(chat.start, 0, width - chat.start, height);
    chat_column(&ctx, buf, chat.start + 2..width - 2, &mut layout);
    // The warning rides the chat's rule row, over the left of the rule.
    if let Some(line) = fallback {
        put(buf, 0, 1, &line, width, dim);
    }
    if overlay_open(screen.model) {
        keys_row(&ctx, buf, &mut layout);
    }
    layout
}

/// Whether a full-pane panel is open: the footer row is its own, and the
/// chat's hint row draws no version or gear under it.
fn overlay_open(model: &Model) -> bool {
    model.settings_open() || model.peek_open()
}

/// The cells a draft has in the chat column of `area` split as `split`: what
/// the caller wraps it to.
pub(crate) fn draft_width(area: Rect, split: Split) -> usize {
    let left = sidebar_width(area, split).map_or(0, |rule| rule + 1) + 2;
    usize::from(area.width.saturating_sub(2).saturating_sub(left)).max(1)
}

/// The height a draft in `area` is laid out in. A view keeps one row of its
/// height back, so this is one more than the composer's draft rows: ten at
/// most, fewer where the lane would keep under [`LANE_FLOOR`], one at least.
pub(crate) fn composer_pane(area: Rect) -> usize {
    // The six fixed rows: header, rule, the gap above the lane's rule, that
    // rule, the address and the hint.
    let room = usize::from(area.height.saturating_sub(6 + LANE_FLOOR));
    room.clamp(1, ROWS_MAX) + 1
}

/// The address a draft to `target` is drawn under; one that is not the
/// app's `home` says so.
pub(crate) fn address(target: &str, home: Option<&str>, speaker: &str) -> String {
    let away = if home == Some(target) {
        ""
    } else {
        " (not home)"
    };
    format!("to {target}{away} › {speaker}")
}

// ---------------------------------------------------------------------------
// the borders: where each stands, dragged or not
// ---------------------------------------------------------------------------

/// The sidebar's width, which is where its rule stands, or `None` when the
/// pane is too small to keep it: 44 from 140 wide, else 34, until a drag
/// asks for another, clamped so the chat keeps its columns.
fn sidebar_width(area: Rect, split: Split) -> Option<u16> {
    if area.width < SIDEBAR_MIN.0 || area.height < SIDEBAR_MIN.1 {
        return None;
    }
    let default = if area.width >= 140 { 44 } else { 34 };
    Some(
        split
            .sidebar
            .map_or(default, |asked| sidebar_clamp(asked, area)),
    )
}

fn sidebar_clamp(asked: u16, area: Rect) -> u16 {
    let most = area.width.saturating_sub(CHAT_FLOOR + 1);
    asked.min(most).max(SIDEBAR_FLOOR)
}

/// The session list's rows for `count` sessions, and whether it keeps them
/// all when its sessions need fewer: two thirds of the rows the list and the
/// tab body share, never fewer than the 18 (from 40 high) or 11 the list once
/// had, sized to its sessions, until a drag asks for a height of its own.
fn list_rows(area: Rect, split: Split, count: usize) -> (u16, bool) {
    // Below the list: a blank row, the tab row and the rule; below the body:
    // two rows. The list keeps the share it had before the frame gave rows
    // back, and the body takes them.
    let shared = area.height.saturating_sub(LIST_TOP + TOP_GIVEN + 5);
    let once = if area.height >= 40 { 18 } else { 11 };
    // Two thirds without the overflow of doubling a tall pane first.
    let default = (shared / 3 * 2 + shared % 3 * 2 / 3).max(once.min(shared));
    match split.list {
        None => (default, false),
        Some(asked) => (list_clamp(asked, area, count), true),
    }
}

/// A list height a drag may keep: room for one session and its more row, and
/// the tab body's rows above the floor — though never short of where the
/// list stands undragged, so a drag can always put it back.
fn list_clamp(asked: u16, area: Rect, count: usize) -> u16 {
    let least = match count {
        0 => 1,
        1 => 2,
        _ => 3,
    };
    let (default, _) = list_rows(area, Split::default(), count);
    let undragged = List::of(count, None, default, false, None).end - LIST_TOP;
    // Below the list: a blank row, the tab row and the rule; below the
    // floor: two rows.
    let most = area
        .height
        .saturating_sub(LIST_TOP + TOP_GIVEN + 3 + BODY_FLOOR + 2)
        .max(undragged);
    asked.min(most).max(least)
}

/// The row a list of `rows` puts its border on: a blank row, the tab row,
/// then the rule.
fn list_border(rows: u16) -> u16 {
    LIST_TOP + rows + 2
}

/// The size a drag asks for: where its edge stood at the press, moved as far
/// as the pointer has since, clamped to `area` as it is now.
pub(crate) fn dragged_to(drag: Drag, mouse: Mouse, area: Rect, count: usize) -> u16 {
    let now = match drag.edge {
        Edge::Sidebar => mouse.column,
        Edge::List => mouse.row,
    };
    let moved = i32::from(drag.at) + i32::from(now) - i32::from(drag.from);
    let to = u16::try_from(moved.max(0)).unwrap_or(u16::MAX);
    match drag.edge {
        Edge::Sidebar => sidebar_clamp(to, area),
        Edge::List => list_clamp(to.saturating_sub(list_border(0)), area, count),
    }
}

/// Whether `edge` is being dragged.
fn dragged(model: &Model, edge: Edge) -> bool {
    model.drag().is_some_and(|drag| drag.edge == edge)
}

// ---------------------------------------------------------------------------
// the sidebar
// ---------------------------------------------------------------------------

/// Which sessions the list shows and where it ends.
struct List {
    visible: Range<usize>,
    /// Rows per session: two lines and a gap, or two lines.
    step: u16,
    /// The `↑ / ↓ N more` row, when sessions are out of view.
    more: Option<u16>,
    /// The first row past the list.
    end: u16,
}

impl List {
    /// The list for `count` sessions in `budget` rows: three rows each while
    /// they fit, then two, then a window from `top`, else with `selected` kept
    /// in view. A `pinned` list ends at its budget, else where its rows do.
    fn of(
        count: usize,
        selected: Option<usize>,
        budget: u16,
        pinned: bool,
        top: Option<usize>,
    ) -> Self {
        let budget = usize::from(budget);
        let (step, shown) = if count * 3 <= budget + 1 {
            (3, count)
        } else if count * 2 <= budget {
            (2, count)
        } else {
            (2, budget.saturating_sub(1) / 2)
        };
        let at = selected.unwrap_or(0);
        let start = top
            .unwrap_or_else(|| (at + 1).saturating_sub(shown))
            .min(count - shown);
        let used = cells(shown) * step - u16::from(step == 3 && shown > 0);
        let more = (shown < count).then_some(LIST_TOP + used);
        let end = if pinned {
            LIST_TOP + cells(budget)
        } else {
            LIST_TOP + used.max(u16::from(count == 0)) + u16::from(more.is_some())
        };
        Self {
            visible: start..start + shown,
            step,
            more,
            end,
        }
    }
}

fn sidebar(ctx: &Ctx<'_, '_>, buf: &mut Buffer, rule: u16, layout: &mut Layout) {
    let Screen { fleet, model, .. } = *ctx.screen;
    let paint = ctx.paint;
    let end = rule - 2;
    let room = end - LEFT;
    let count = fleet.rows.len();
    let selected = model.position(fleet);
    let (rows, pinned) = list_rows(buf.area, model.split(), count);
    let list = List::of(count, selected, rows, pinned, model.list_top());
    layout.list = Rect::new(0, LIST_TOP, rule, list.end - LIST_TOP);
    layout.selectable(
        buf,
        select::Panel::List,
        Rect::new(LEFT, LIST_TOP, room, list.end - LIST_TOP),
    );
    layout.list_start = Some(list.visible.start);
    layout.list_max = count - list.visible.len();
    let needy = fleet.rows.iter().any(|row| row.needy);
    let tone = if needy {
        paint.fg(|p| p.needs_you)
    } else {
        paint.fg(|p| p.dim)
    };
    put_line(buf, LEFT, 0, &summary_line(ctx, fleet, room, tone), room);
    if needy {
        // An arrow for each side of the list that hides a needy row.
        let hides = |rows: Range<usize>| fleet.rows[rows].iter().any(|row| row.needy);
        let mut arrows = String::new();
        arrows.extend(hides(0..list.visible.start).then_some('↑'));
        arrows.extend(hides(list.visible.end..count).then_some('↓'));
        put(
            buf,
            end - 2 - cells(arrows.chars().count()),
            2,
            &arrows,
            2,
            tone,
        );
    }
    if count == 0 {
        let empty = if ctx.wait.fleet {
            "loading"
        } else {
            "No sessions."
        };
        put(buf, LEFT, LIST_TOP, empty, room, paint.fg(|p| p.dim));
    }
    for (step, at) in list.visible.clone().enumerate() {
        let y = LIST_TOP + cells(step) * list.step;
        session(ctx, buf, &fleet.rows[at], y, end, Some(at) == selected);
        layout.record(
            buf,
            Rect::new(0, y, rule, 2),
            Hit::Session(fleet.rows[at].name.clone()),
        );
    }
    if let Some(y) = list.more {
        let (above, below) = (list.visible.start, count - list.visible.end);
        let hidden = |at: &usize| !list.visible.contains(at);
        let needy = (0..count)
            .filter(|at| hidden(at) && fleet.rows[*at].needy)
            .count();
        let mut parts = Vec::new();
        if above > 0 {
            parts.push(format!("↑ {above}"));
        }
        if below > 0 {
            parts.push(format!("↓ {below}"));
        }
        let mut text = format!("{} more", parts.join(" · "));
        if needy > 0 {
            let _ = write!(text, ", {} {needy}", Mark::NeedsYou.glyph(ctx.icons));
        }
        put(buf, LEFT, y, &text, room, tone);
    }
    let Some(entry) = ctx.screen.selected.filter(|_| !ctx.wait.fleet) else {
        return;
    };
    let tabs = list.end + 1;
    agents_heading(ctx, buf, tabs, end);
    let border = paint.rule(dragged(model, Edge::List));
    put(
        buf,
        0,
        tabs + 1,
        &"─".repeat(usize::from(rule)),
        rule,
        border,
    );
    layout.border(buf, Edge::List, tabs + 1, Rect::new(0, tabs + 1, rule, 1));
    tab_body(
        ctx,
        buf,
        &agent_rows(ctx, entry, room),
        tabs + 2,
        rule,
        room,
        layout,
    );
}

/// The Agents body from row `top` down to the floor, in the sidebar's `room`,
/// scrolled to the model's row; a body that does not fit names the rows it
/// hides on its last row.
fn tab_body(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    body: &[Cells],
    top: u16,
    rule: u16,
    room: u16,
    layout: &mut Layout,
) {
    let (end, paint) = (rule - 2, ctx.paint);
    let floor = buf.area.height;
    let room_rows = floor.saturating_sub(top);
    // A body that still does not fit gives its last row to the rows it hides.
    let cut = body.len() > usize::from(room_rows);
    let shown = usize::from(room_rows.saturating_sub(u16::from(cut)));
    layout.body = Rect::new(0, top, rule, room_rows);
    layout.selectable(
        buf,
        select::Panel::Body,
        Rect::new(LEFT, top, room, room_rows),
    );
    layout.body_max = body.len().saturating_sub(shown);
    let first = ctx.screen.model.body_top().min(layout.body_max);
    layout.body_top = Some(first);
    if cut && room_rows > 0 {
        let below = body.len().saturating_sub(first + shown);
        let parts: Vec<String> = [(first, "↑"), (below, "↓")]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, arrow)| format!("{arrow} {count}"))
            .collect();
        let text = format!("{} more rows", parts.join(" · "));
        put(buf, LEFT, floor - 1, &text, room, paint.fg(|p| p.dim));
    }
    let drawn = body.iter().skip(first).take(shown);
    for seat in drawn.clone().filter_map(|row| row.seat.as_deref()) {
        if !layout.seats.iter().any(|kept| kept == seat) {
            layout.seats.push(seat.to_owned());
        }
    }
    let asked = ctx.screen.model.focus();
    layout.focused = (layout
        .seats
        .iter()
        .find(|seat| Some(seat.as_str()) == asked))
    .or(layout.seats.first())
    .cloned();
    let mut before = None;
    for (step, row) in drawn.enumerate() {
        let y = top + cells(step);
        let seat = row.seat.as_deref();
        let focused = seat.is_some() && layout.focused.as_deref() == seat;
        if let Some(seat) = seat {
            layout.record(
                buf,
                Rect::new(LEFT, y, end - LEFT, 1),
                Hit::Seat(seat.to_owned()),
            );
        }
        // One glyph, on the seat's first drawn row; its rows share the ground.
        if focused && before != seat {
            let glyph = if ctx.icons { "▸" } else { ">" };
            let ink = paint.fg(|p| p.selected_ink).add_modifier(Modifier::BOLD);
            put(buf, 0, y, glyph, 1, ink);
        }
        before = seat;
        let used = put_line(buf, LEFT, y, &row.left, room);
        if let Some(right) = &row.right {
            let width = cells(right.width());
            if used + 1 + width <= end {
                put_line(buf, end - width, y, right, width);
            }
        }
        if focused {
            paint.lift(buf, Rect::new(0, y, end, 1));
        }
    }
}

/// The sidebar's first row: sessions, agents, working and needs as icon-led
/// counts. A narrow row drops whole tokens, lowest priority first, never cutting
/// a count; `+?` marks a lower bound.
fn summary_line(ctx: &Ctx<'_, '_>, fleet: &Fleet, room: u16, tone: Style) -> Line<'static> {
    let (sum, paint) = (summary(fleet), ctx.paint);
    let more = if sum.unknown { "+?" } else { "" };
    let sessions = if ctx.icons { "▣" } else { "#" };
    let agents = if ctx.icons { "◈" } else { "@" };
    let [work, need] = [Mark::Working, Mark::NeedsYou].map(|mark| mark.glyph(ctx.icons));
    let (text, busy) = (paint.fg(|p| p.text), paint.mark(Mark::Working));
    let tokens = [
        Span::styled(format!("{sessions} {}", fleet.rows.len()), text),
        Span::styled(format!("{agents} {}{more}", sum.agents), text),
        Span::styled(format!("{work} {}{more}", sum.working), busy),
        Span::styled(format!("{need} {}", sum.needy), tone),
    ];
    let ladder: [(&[usize], &str); 5] = [
        (&[0, 1, 2, 3], "  "),
        (&[0, 1, 2, 3], " "),
        (&[0, 2, 3], " "),
        (&[0, 3], " "),
        (&[3], " "),
    ];
    for (keep, gap) in ladder {
        let mut spans = Vec::new();
        for &at in keep {
            spans.extend((!spans.is_empty()).then(|| Span::raw(gap)));
            spans.push(tokens[at].clone());
        }
        let line = Line::from(spans);
        if cells(line.width()) <= room {
            return line;
        }
    }
    Line::default()
}

/// One session's two lines at row `y`.
fn session(ctx: &Ctx<'_, '_>, buf: &mut Buffer, row: &Row, y: u16, end: u16, selected: bool) {
    let paint = ctx.paint;
    let dim = paint.fg(|p| p.dim);
    let stopped = matches!(row.counts, Counts::Stopped);
    if row.needy {
        for line in y..y + 2 {
            put(buf, 0, line, "│", 1, paint.fg(|p| p.needs_you));
        }
    }
    let mark = if stopped { dim } else { paint.mark(row.mark) };
    put(buf, LEFT, y, row.mark.glyph(ctx.icons), 1, mark);
    // The unread cue rides the blank gutter cell, so no column moves.
    if !selected && ctx.screen.model.is_unread(&row.name) {
        let cue = if ctx.icons { "\u{25b8}" } else { ">" };
        let style = paint.fg(|p| p.title).add_modifier(Modifier::BOLD);
        put(buf, 1, y, cue, 1, style);
    }
    if row.index <= 9 {
        put(buf, LEFT + 2, y, &row.index.to_string(), 1, dim);
    }
    // Only a stopped row has anything right of its name: its age.
    let mut edge = end;
    if stopped {
        let since = match row.line2 {
            Line2::NotRunning { stopped_secs, .. } => stopped_secs,
            _ => None,
        };
        let text = format!("stopped {}", age(since));
        edge = end.saturating_sub(cells(text.chars().count()));
        put(buf, edge, y, &text, end - edge, dim);
    }
    let name = if selected {
        paint.selected()
    } else if stopped {
        dim
    } else {
        paint.fg(|p| p.text).add_modifier(Modifier::BOLD)
    };
    let room = edge.saturating_sub(NAME + u16::from(stopped));
    let after = put(buf, NAME, y, &row.name, room, name);
    let tag = "this window";
    if row.home && after + 2 + cells(tag.len()) < edge {
        put(buf, after + 2, y, tag, cells(tag.len()), dim);
    }
    second_line(ctx, buf, row, y + 1, end);
}

/// Cells a status keeps at least; the counts yield whole spans for them.
const STATUS_MIN: u16 = 10;

/// A running session's counts, then its question or seat-down status. Counts
/// that do not fit drop from the lowest urgency and close on `…`; a stopped
/// session says how many seats.
fn second_line(ctx: &Ctx<'_, '_>, buf: &mut Buffer, row: &Row, y: u16, end: u16) {
    let (paint, room) = (ctx.paint, end - NAME);
    let dim = paint.fg(|p| p.dim);
    let cols = |text: &str| cells(Line::from(text).width());
    let status = match &row.line2 {
        Line2::NotRunning { seats, .. } => {
            let text = format!("{} not running", seats_word(*seats));
            put(buf, NAME, y, &text, room, dim);
            return;
        }
        Line2::Question { text, .. } => Some(text.as_str()),
        Line2::SeatDown => Some("seat down"),
        Line2::Goal { .. } | Line2::NoGoal => None,
    };
    let budget = room.saturating_sub(status.map_or(0, |text| drawn_cols(text).min(STATUS_MIN) + 2));
    let parts: Vec<(String, Style)> = match &row.counts {
        Counts::Marks(tallied) if !tallied.is_empty() => tallied
            .iter()
            .map(|(mark, blocked, count)| {
                let letter = if *blocked { "B" } else { "" };
                let text = format!("{}{letter}{count}", mark.glyph(ctx.icons));
                (text, paint.mark(*mark))
            })
            .collect(),
        Counts::Marks(_) => vec![("no agents".to_owned(), dim)],
        Counts::Unknown | Counts::Stopped => vec![("agents ?".to_owned(), dim)],
    };
    let mut x = NAME;
    for (at, (text, style)) in parts.iter().enumerate() {
        x += u16::from(at > 0);
        let tail = if at + 1 < parts.len() { 2 } else { 0 };
        if x + cols(text) + tail > NAME + budget {
            x = put(buf, x, y, "…", 1, dim);
            break;
        }
        x = put(buf, x, y, text, budget, *style);
    }
    let Some(text) = status else { return };
    let (at, text_style) = (x + 2, paint.fg(|p| p.text));
    let avail = end.saturating_sub(at);
    let tail = match &row.line2 {
        Line2::Question { age_secs, .. } => format!(" · {}", age(*age_secs)),
        _ => String::new(),
    };
    put(buf, at, y, &fit(text, &tail, avail), avail, text_style);
}

/// Where each grapheme of `text` that the buffer draws ends in `text`, with the
/// cells it spends. The buffer segments the whole string and then skips what
/// spends no cell, so a cut must be a prefix of `text`, never a rejoining.
fn drawn(text: &str) -> Vec<(usize, usize)> {
    let spent = |symbol: &str| usize::from(symbol.cell_width());
    let end = |symbol: &str| symbol.as_ptr() as usize - text.as_ptr() as usize + symbol.len();
    Span::raw(text)
        .styled_graphemes(Style::default())
        .map(|grapheme| (end(grapheme.symbol), spent(grapheme.symbol)))
        .collect()
}

/// The cells `drawn` spends on `text`.
fn drawn_cols(text: &str) -> u16 {
    cells(drawn(text).iter().map(|(_, width)| width).sum())
}

/// `text` then `tail` in `room` cells: whole, else the text alone, else its
/// head cut at a grapheme and closed on `…`.
fn fit(text: &str, tail: &str, room: u16) -> String {
    let (whole, room) = (usize::from(drawn_cols(text)), usize::from(room));
    if room == 0 {
        return String::new();
    }
    if whole + tail.chars().count() <= room {
        return format!("{text}{tail}");
    }
    if whole <= room {
        return text.to_owned();
    }
    let (mut used, mut end) = (1, 0);
    for (stop, width) in drawn(text) {
        if used + width > room {
            break;
        }
        used += width;
        end = stop;
    }
    format!("{}…", &text[..end])
}

fn seats_word(seats: usize) -> String {
    match seats {
        1 => "1 seat,".to_owned(),
        n => format!("{n} seats,"),
    }
}

/// The heading over the body: `Agents` and the seat count.
fn agents_heading(ctx: &Ctx<'_, '_>, buf: &mut Buffer, y: u16, end: u16) {
    let words = match ctx.screen.agents {
        Some(Facts::Seats { agents, .. }) => format!("Agents {}", agents.len()),
        _ => "Agents".to_owned(),
    };
    let style = ctx.paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    put(buf, LEFT, y, &words, end - LEFT, style);
}

/// A body row: its left text and, right-aligned, an optional word.
struct Cells {
    left: Line<'static>,
    right: Option<Line<'static>>,
    /// The seat this row belongs to, when it is one of a seat's rows.
    seat: Option<String>,
}

impl Cells {
    fn of(spans: Vec<Span<'static>>) -> Self {
        Self {
            left: Line::from(spans),
            right: None,
            seat: None,
        }
    }

    /// This row as one of `seat`'s.
    fn of_seat(mut self, seat: &str) -> Self {
        self.seat = Some(seat.to_owned());
        self
    }
}

/// The Agents body: each seat with its state, then its client, the model its
/// frame drew and the effort, each only when known and never the profile, then
/// the reason it waits on the human. An ask from a seat the roster lacks is an
/// entry of its own after the roster; a session ae has no seat facts for says why.
fn agent_rows(ctx: &Ctx<'_, '_>, entry: &SessionEntry, room: u16) -> Vec<Cells> {
    let paint = ctx.paint;
    let (text, dim) = (paint.fg(|p| p.text), paint.fg(|p| p.dim));
    let gap = |words: &str| vec![Cells::of(vec![Span::styled(words.to_owned(), dim)])];
    let open = &ctx.screen.overview.open;
    let reason = |ask: &Open| {
        let words = format!(
            "  {}",
            aged(&ask.text, ask.age_secs, room.saturating_sub(2))
        );
        Cells::of(vec![Span::styled(words, text)]).of_seat(&ask.seat)
    };
    let (mut rows, agents) = match ctx.screen.agents {
        _ if entry.status == Status::Stopped => (gap("Not running: no seat facts."), &[][..]),
        Some(Facts::Seats { agents, .. }) => (Vec::new(), agents.as_slice()),
        Some(Facts::OtherServer) => (gap("No seat facts: on another tmux server."), &[][..]),
        Some(Facts::NotPublished) | None => {
            (gap("No seat facts: watchdog publishes none."), &[][..])
        }
    };
    for agent in agents {
        let mark = agent.mark();
        rows.push(Cells {
            left: Line::from(vec![
                Span::styled(mark.glyph(ctx.icons), paint.mark(mark)),
                Span::raw(" "),
                Span::styled(agent.name.clone(), text),
            ]),
            right: Some(Line::from(Span::styled(
                agent.state.clone(),
                paint.mark(mark),
            ))),
            seat: Some(agent.name.clone()),
        });
        let facts: Vec<&str> = [&agent.client, &agent.model, &agent.effort]
            .into_iter()
            .map(String::as_str)
            .filter(|fact| !fact.is_empty())
            .collect();
        rows.push(
            Cells::of(vec![Span::styled(format!("  {}", facts.join(" · ")), dim)])
                .of_seat(&agent.name),
        );
        rows.extend(open.iter().filter(|ask| ask.seat == agent.name).map(reason));
    }
    let listed = |ask: &&Open| agents.iter().any(|agent| agent.name == ask.seat);
    for ask in open.iter().filter(|ask| !listed(ask)) {
        let mark = Mark::NeedsYou;
        rows.push(Cells {
            left: Line::from(vec![
                Span::styled(mark.glyph(ctx.icons), paint.mark(mark)),
                Span::raw(" "),
                Span::styled(ask.seat.clone(), text),
            ]),
            right: Some(Line::from(Span::styled("needs you", paint.mark(mark)))),
            seat: Some(ask.seat.clone()),
        });
        rows.push(reason(ask));
    }
    rows
}

// ---------------------------------------------------------------------------
// the chat column
// ---------------------------------------------------------------------------

/// The chat column over `columns`: the header, the turns anchored to the
/// bottom, the composer.
fn chat_column(ctx: &Ctx<'_, '_>, buf: &mut Buffer, columns: Range<u16>, layout: &mut Layout) {
    let (screen, paint) = (ctx.screen, ctx.paint);
    let height = buf.area.height;
    let (left, end) = (columns.start, columns.end);
    let room = end - left;
    let (dim, border) = (paint.fg(|p| p.dim), paint.fg(|p| p.border));
    header(ctx, buf, left, end);
    put(buf, left, 1, &"─".repeat(usize::from(room)), room, border);
    // While writing, the composer has a row for each row of the draft's view,
    // under the address row; the hint row, the pane's last, follows.
    let grown = match screen.composer {
        Composer::Home {
            view: Some(view), ..
        } => cells(view.rows.len().max(1)),
        _ => 1,
    };
    let composer = height.saturating_sub(2 + grown);
    let (top, bottom) = (2, composer.saturating_sub(2));
    let main = screen
        .selected
        .and_then(|entry| entry.main_agent.as_deref())
        .or_else(|| match screen.composer {
            Composer::Home { .. } | Composer::ReadOnly { .. } | Composer::Held { .. } => {
                screen.pair.first().map(String::as_str)
            }
            Composer::NoHome => None,
        })
        .unwrap_or_default();
    let clock = view::Style::resolve(true, screen.look, screen.zone, main);
    let room_rows = usize::from(bottom.saturating_sub(top));
    let scroll = screen.model.scroll_rows(room_rows);
    // One page and a notch past the window, so a wheel notch or a page key
    // stays inside what this frame produced until the next produces more.
    let need = scroll
        .saturating_add(room_rows.saturating_mul(2))
        .saturating_add(super::model::WHEEL_ROWS);
    let super::lane::Tail {
        mut rows,
        complete,
        turns,
    } = lane_tail(ctx, &clock, main, (room, need));
    if ctx.wait.lane {
        rows.push(Line::from(Span::styled("loading", dim)));
    }
    let (turns, bars, folds) = place_turns(ctx, &turns, &mut rows, layout);
    let last = rows
        .len()
        .saturating_sub(scroll)
        .max(room_rows.min(rows.len()));
    let first = last.saturating_sub(room_rows);
    let y0 = bottom - cells(last - first);
    for (step, line) in rows[first..last].iter().enumerate() {
        put_line(buf, left, y0 + cells(step), line, room);
    }
    // A fold row is recorded before its turn: the first target to hold a cell wins.
    for (key, row) in folds {
        if (first..last).contains(&row) {
            let rect = Rect::new(left, y0 + cells(row - first), room, 1);
            layout.record(buf, rect, Hit::Fold(key));
        }
    }
    turn_targets(buf, layout, &turns, (first..last, left, y0, room));
    layout.lane(buf, (left, top, room, room_rows), &bars, (first..last, y0));
    if room_rows > 0 && scroll > 0 && last < rows.len() {
        put(buf, left, bottom, "↓ newer turns below · PgDn", room, dim);
    }
    put(
        buf,
        left,
        composer.saturating_sub(1),
        &"─".repeat(usize::from(room)),
        room,
        border,
    );
    let (address_row, input_row, hint) = composer_lines(ctx);
    put_line(buf, left, composer, &address_row, room);
    hint_row(ctx, buf, layout, (left, end), &hint);
    if let Composer::Home {
        view: Some(view), ..
    } = screen.composer
    {
        // The draft under the address, from the column's left edge, and the
        // cursor after the cells its row draws before it, never past the row.
        let text = paint.fg(|p| p.text);
        for (y, row) in (composer + 1..).zip(&view.rows) {
            put(buf, left, y, row, room, text);
        }
        let before = cells(Span::raw(view.before.as_str()).width());
        layout.cursor = (room > 0).then(|| {
            let row = composer + 1 + cells(view.cursor_row);
            Position::new(left + before.min(room - 1), row)
        });
    } else {
        put_line(buf, left, composer + 1, &input_row, room);
    }
    layout.record(
        buf,
        Rect::new(left, composer, room, grown + 2),
        Hit::Compose,
    );
    layout.page_rows = room_rows;
    layout.max_scroll = rows.len().saturating_sub(room_rows);
    layout.complete = complete;
}

/// The hint row, the pane's last: a pending line — an armed key, a refused
/// paste, an ask on its way — in full ink so it is not missed, else the
/// composer's hint; then the keys that work in this mode, when they fit whole;
/// at the right end `ae <version>` and the gear.
fn hint_row(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    layout: &mut Layout,
    (left, end): (u16, u16),
    hint: &str,
) {
    let (paint, y) = (ctx.paint, buf.area.height - 1);
    let (first, style) = match ctx.screen.model.note() {
        Some(note) => (note, paint.fg(|p| p.text)),
        None => (hint, paint.fg(|p| p.dim)),
    };
    let mut at = put(buf, left, y, first, end - left, style);
    let keys = mode_hint(&ctx.screen.composer);
    let rest = match (first.is_empty(), keys.is_empty()) {
        (_, true) => String::new(),
        (true, false) => keys.to_owned(),
        (false, false) => format!(" · {keys}"),
    };
    if cells(Span::raw(rest.as_str()).width()) <= end.saturating_sub(at) {
        at = put(buf, at, y, &rest, end - at, paint.fg(|p| p.dim));
    }
    // A panel's footer is its own row: no second gear under it.
    if !overlay_open(ctx.screen.model) {
        tail(ctx, buf, layout, (y, at), false);
    }
}

/// The keys that work in this mode: writing names the three that matter,
/// held takes every key and says nothing.
fn mode_hint(composer: &Composer<'_>) -> &'static str {
    match composer {
        Composer::Home { view: Some(_), .. } => "Enter sends · Alt+Enter ^J newline · Esc browses",
        Composer::Held { .. } => "",
        _ => "? keys",
    }
}

/// The lane's newest rows as the app's own state shows them: `size` is the
/// column's width and how many rows are wanted.
fn lane_tail(
    ctx: &Ctx<'_, '_>,
    clock: &view::Style,
    main: &str,
    size: (u16, usize),
) -> super::lane::Tail {
    let show = super::lane::Show {
        clock,
        main,
        open: (!ctx.wait.open.is_empty()).then_some(ctx.wait.open),
        sent: ctx.wait.sent,
    };
    let (room, need) = size;
    let lane = ctx.screen.lane;
    super::lane::tail_with(lane, usize::from(room), ctx.paint, &show, need, None)
}

/// A turn's fingerprint and the rows it spans.
type TurnSpan = (u64, Range<usize>);

/// The produced turns with their fingerprints and row spans, oldest first, the
/// bars of the marked one and each fold row as its fold key and row. The
/// marked turn is recoloured in place, and the frame keeps each turn's place
/// and fold for the keys.
fn place_turns(
    ctx: &Ctx<'_, '_>,
    turns: &[super::lane::Turn],
    rows: &mut [Line<'static>],
    layout: &mut Layout,
) -> (Vec<TurnSpan>, Vec<usize>, Vec<(u64, usize)>) {
    let keyed: Vec<_> = (turns.iter())
        .filter_map(|turn| Some((turn, *ctx.wait.keys.get(turn.at)?)))
        .collect();
    let turns: Vec<TurnSpan> = (keyed.iter())
        .map(|(turn, key)| (*key, turn.first..turn.first + turn.rows))
        .collect();
    let folds: Vec<(u64, usize)> = (keyed.iter())
        .filter_map(|(turn, _)| Some((*ctx.wait.folds.get(turn.at)?, turn.first + turn.fold?)))
        .collect();
    layout.folds = (keyed.iter())
        .filter(|(turn, _)| turn.fold.is_some())
        .filter_map(|(turn, key)| Some((*key, *ctx.wait.folds.get(turn.at)?)))
        .collect();
    let mut bars = Vec::new();
    for (key, span) in &turns {
        if ctx.screen.model.turn() == Some(*key) {
            let barred = super::lane::mark_rows(&mut rows[span.clone()], ctx.paint, ctx.icons);
            bars.extend(barred.into_iter().map(|row| span.start + row));
        }
    }
    let end = rows.len();
    layout.turns = (turns.iter().rev())
        .map(|(key, span)| (*key, end - span.end, span.len()))
        .collect();
    (turns, bars, folds)
}

/// A turn's rows are its target, across the column: the column keeps two
/// cells between it and the sidebar rule, so the rule's grab zone is never
/// covered. `window` is the rows drawn, from the column's `left`, the first
/// row's `y0` and the column's `room`.
fn turn_targets(
    buf: &Buffer,
    layout: &mut Layout,
    turns: &[(u64, Range<usize>)],
    (window, left, y0, room): (Range<usize>, u16, u16, u16),
) {
    for (key, span) in turns {
        let (from, to) = (span.start.max(window.start), span.end.min(window.end));
        if from < to {
            let y = y0 + cells(from - window.start);
            let rect = Rect::new(left, y, room, cells(to - from));
            layout.record(buf, rect, Hit::Turn(*key));
        }
    }
}

/// The chat header: the session, its pair, and right-aligned its branch (at
/// 140 columns or more) and its activity.
fn header(ctx: &Ctx<'_, '_>, buf: &mut Buffer, left: u16, end: u16) {
    let screen = ctx.screen;
    let paint = ctx.paint;
    let Some(entry) = screen.selected else {
        return;
    };
    let name = match paint.0 {
        Some(_) => paint.fg(|p| p.title).add_modifier(Modifier::BOLD),
        None => Style::new().add_modifier(Modifier::BOLD),
    };
    let after = put(buf, left, 0, &entry.name, end - left, name);
    let middle = screen.pair.join(" + ");
    let after = put(
        buf,
        after + 3,
        0,
        &middle,
        end.saturating_sub(after + 3),
        paint.fg(|p| p.dim),
    );
    let since = entry
        .last_active_epoch
        .map(|epoch| screen.now.epoch().saturating_sub(epoch));
    let activity = match entry.status {
        Status::Stopped => format!("stopped {}", age(since)),
        _ => format!("active {}", age(since)),
    };
    let wide = buf.area.width >= 140;
    let branch = entry
        .branch
        .as_deref()
        .filter(|_| wide)
        .map(|branch| format!("{branch}  ·  {activity}"));
    // The recorded working dir sits after the pair, cut from the left; the
    // branch gives way to it before the activity does.
    let dir = screen.overview.dir.as_deref().unwrap_or_default();
    let from = if middle.is_empty() { after } else { after + 3 };
    let wanted = if dir.is_empty() {
        after
    } else {
        from + cells(dir.chars().count().min(12))
    };
    let width = |right: &&String| cells(right.chars().count());
    let rights: Vec<&String> = branch.iter().chain(std::iter::once(&activity)).collect();
    let room_for = |right: &&String| after + 2 + width(right) <= end;
    let right = (rights.iter())
        .find(|right| room_for(right) && end.saturating_sub(width(right) + 2) >= wanted)
        .or_else(|| rights.iter().find(|right| room_for(right)));
    let stop = right.map_or(end, |right| {
        put(
            buf,
            end - width(right),
            0,
            right,
            width(right),
            paint.fg(|p| p.dim),
        );
        end - width(right) - 2
    });
    let room = stop.saturating_sub(from);
    let shown = clip_tail(dir, usize::from(room));
    put(buf, from, 0, &shown, room, paint.fg(|p| p.dim));
}

/// The composer's address row, the input row under it and the hint under that.
fn composer_lines(ctx: &Ctx<'_, '_>) -> (Line<'static>, Line<'static>, String) {
    let paint = ctx.paint;
    let (text, dim) = (paint.fg(|p| p.text), paint.fg(|p| p.dim));
    let quiet = |words: String| Line::from(Span::styled(words, dim));
    match ctx.screen.composer {
        Composer::Home {
            home,
            speaker,
            view,
            draft,
        } => {
            let address = address(home, ctx.screen.fleet.home.as_deref(), speaker);
            let address = Line::from(Span::styled(address, paint.fg(|p| p.title)));
            if view.is_some() {
                return (address, Line::default(), String::new());
            }
            let kept = Line::from(Span::styled(draft.to_owned(), text));
            let hint = if draft.is_empty() {
                "Enter writes"
            } else {
                "draft kept · Enter writes"
            };
            (address, kept, hint.to_owned())
        }
        Composer::ReadOnly { why } => (
            quiet(format!("read-only · {why}")),
            Line::default(),
            String::new(),
        ),
        Composer::Held { why } => (
            quiet(format!("not writing: {why} · Esc browses")),
            Line::default(),
            String::new(),
        ),
        Composer::NoHome => (
            quiet("read-only · no home session: ae app <session> picks one".to_owned()),
            Line::default(),
            String::new(),
        ),
    }
}

/// The browse keys the Keys tab lists, in the order it shows them: the keys
/// and what they do.
const BROWSE_KEYS: [(&str, &str); 15] = [
    ("1-9", "select that session"),
    ("j/k Up/Down", "previous / next session"),
    ("!", "next session needing you"),
    ("oo", "open seat; twice in 2 s"),
    ("m", "session menu; right-click a card too"),
    ("v", "peek at the seat's screen"),
    ("n/p", "next / previous seat"),
    ("PgUp/PgDn", "scroll the chat"),
    ("[ ]", "mark older / newer turn"),
    ("y", "copy the marked turn"),
    ("Enter i", "write; a paste writes too"),
    ("Esc", "select the home session"),
    ("s", "settings"),
    ("?", "this help"),
    ("qq", "quit: twice within 2 s"),
];

/// The footer a full-pane panel keeps on the pane's last row, the full width:
/// the panel's word and its one hint from the left, then `ae <version>` and the
/// gear right-aligned. The cells are reset first, so the closed frame's hint
/// row, sidebar ground and rule never show under the panel.
fn keys_row(ctx: &Ctx<'_, '_>, buf: &mut Buffer, layout: &mut Layout) {
    let paint = ctx.paint;
    let (width, y) = (buf.area.width, buf.area.height - 1);
    for x in 0..width {
        buf[(x, y)].reset();
    }
    buf.set_style(Rect::new(0, y, width, 1), paint.ground(|p| p.ink));
    let peeking = ctx.screen.model.peek_open();
    let (word, hint) = if ctx.screen.model.settings_open() {
        ("settings", "Esc close")
    } else {
        // The panel covers the hint row, so a copy's outcome rides here.
        ("peek", ctx.screen.model.note().unwrap_or("Esc close"))
    };
    let x = put(
        buf,
        LEFT,
        y,
        word,
        width - LEFT,
        paint.fg(|p| p.text).add_modifier(Modifier::BOLD),
    );
    let rest = format!("   {hint}");
    let hints_end = put(
        buf,
        x,
        y,
        &rest,
        width.saturating_sub(x),
        paint.fg(|p| p.dim),
    );
    tail(ctx, buf, layout, (y, hints_end), peeking);
}

/// `ae <version>` and the gear, right-aligned on row `y`, each only when it
/// fits after `hints_end` and a gap: narrowing drops the version first, then
/// the gear. The peek panel hides the gear, because a click on it would be dead.
fn tail(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    layout: &mut Layout,
    (y, hints_end): (u16, u16),
    peeking: bool,
) {
    let width = buf.area.width;
    let gear = if ctx.icons { "\u{2699}" } else { "*" };
    let tail = if peeking {
        crate::version_line()
    } else {
        format!("{} {gear}", crate::version_line())
    };
    let tail_cells = cells(Span::raw(tail.as_str()).width());
    let gear_cells = cells(Span::raw(gear).width());
    let room = |need: u16| usize::from(hints_end) + 2 + usize::from(need) <= usize::from(width);
    let style = ctx.paint.fg(|p| p.dim);
    // The click covers the gear and the blank before it, like the status
    // line's settings range; `room` keeps that blank free.
    let target = Rect::new(width.saturating_sub(gear_cells + 1), y, gear_cells + 1, 1);
    if room(tail_cells) {
        put(buf, width - tail_cells, y, &tail, tail_cells, style);
        if !peeking {
            layout.record(buf, target, Hit::Settings);
        }
    } else if !peeking && room(gear_cells) {
        put(buf, width - gear_cells, y, gear, gear_cells, style);
        layout.record(buf, target, Hit::Settings);
    }
}

// ---------------------------------------------------------------------------
// the settings overlay: a reusable full-pane panel
// ---------------------------------------------------------------------------

/// The overlay's title row.
const SETTINGS_TITLE: &str = "Settings";
/// The tabs in draw order.
const SETTINGS_TABS: [&str; 5] = ["Quota", "Config", "About", "Instructions", "Keys"];
/// The clickable close label, right-aligned on the title row.
const SETTINGS_CLOSE: &str = "Esc close";
/// Plain-text links; no OSC 8 in this slice.
const SETTINGS_LINKS: [(&str, &str); 3] = [
    ("repo", "https://github.com/clemens33/ae"),
    ("releases", "https://github.com/clemens33/ae/releases"),
    (
        "docs",
        "https://github.com/clemens33/ae/blob/main/docs/app.md",
    ),
];

/// Paint the open overlay, then record its scroll bounds. The caller
/// stripped the covered layout first.
pub(crate) fn paint_settings(
    tab: SettingsTab,
    scroll: usize,
    bodies: &SettingsBodies,
    look: Option<&Look>,
    buf: &mut Buffer,
    layout: &mut Layout,
) {
    let paint = Paint::of(look);
    let width = buf.area.width;
    let selected = match tab {
        SettingsTab::Quota => 0,
        SettingsTab::Config => 1,
        SettingsTab::About => 2,
        SettingsTab::Instructions => 3,
        SettingsTab::Keys => 4,
    };
    panel(
        buf,
        layout,
        paint,
        &Panel {
            title: SETTINGS_TITLE,
            note: "",
            tabs: &SETTINGS_TABS,
            hits: &[
                Hit::SettingsTab(SettingsTab::Quota),
                Hit::SettingsTab(SettingsTab::Config),
                Hit::SettingsTab(SettingsTab::About),
                Hit::SettingsTab(SettingsTab::Instructions),
                Hit::SettingsTab(SettingsTab::Keys),
            ],
            close: (SETTINGS_CLOSE, Hit::SettingsClose),
            selected,
            body: &settings_body(
                tab,
                bodies,
                paint,
                width,
                look.is_none_or(|look| look.icons),
            ),
            scroll: Scroll::Top(scroll),
        },
    );
}

/// What one full-pane panel draws: its title, tab row and scrolled body.
struct Panel<'a> {
    title: &'a str,
    /// A dim line on the tab row, after any tabs: the peek panel's status.
    note: &'a str,
    tabs: &'a [&'a str],
    /// The click target of each tab title, in tab order.
    hits: &'a [Hit],
    /// The label drawn right-aligned on the title row, and its target.
    close: (&'a str, Hit),
    selected: usize,
    body: &'a [(String, Style)],
    scroll: Scroll,
}

/// How far a panel's body is scrolled.
enum Scroll {
    /// Rows down from the first.
    Top(usize),
    /// Rows back from the newest, which the body ends on.
    Back(usize),
}

/// One full-pane panel over rows `0..height-1`: title, tab row, rule and a
/// scrolled body, then its scroll bounds. Settings and the peek panel are the
/// callers; the later `?` help reuses the seam. Below [`MIN`] the closed frame's fallback
/// stands and nothing is painted or recorded.
fn panel(buf: &mut Buffer, layout: &mut Layout, paint: Paint, panel: &Panel<'_>) {
    let area = buf.area;
    if area.width < MIN.0 || area.height < MIN.1 {
        return;
    }
    let (width, height) = (area.width, area.height);
    // Covered cells are reset whole, not just re-grounded: `set_style` keeps
    // the closed frame's symbols and modifiers, which a sparse body would
    // leave visible. The keys row stands.
    for y in 0..height - 1 {
        for x in 0..width {
            buf[(x, y)].reset();
        }
    }
    buf.set_style(Rect::new(0, 0, width, height - 1), paint.ground(|p| p.base));
    let heading = paint.fg(|p| p.title).add_modifier(Modifier::BOLD);
    let (label, close) = &panel.close;
    let label_x = width.saturating_sub(LEFT + cells(Span::raw(*label).width()));
    // The title is cut a cell short of the close label, which keeps its cells.
    let title_room = label_x.saturating_sub(LEFT + 1);
    let title_end = put(buf, LEFT, 0, panel.title, title_room, heading);
    if label_x > title_end {
        let end = put(buf, label_x, 0, label, width - label_x, paint.fg(|p| p.dim));
        layout.record(buf, Rect::new(label_x, 0, end - label_x, 1), close.clone());
    }
    // Three cells between titles when all of them fit that way, else one.
    let titles: usize = panel.tabs.iter().map(|name| Span::raw(*name).width()).sum();
    let wide = usize::from(LEFT) + titles + 3 * panel.tabs.len().saturating_sub(1);
    let gap = if wide <= usize::from(width) {
        "   "
    } else {
        " "
    };
    let mut x = LEFT;
    for (index, name) in panel.tabs.iter().enumerate() {
        let style = if index == panel.selected {
            paint.selected()
        } else {
            paint.fg(|p| p.text)
        };
        let end = put(buf, x, 1, name, width.saturating_sub(x), style);
        if let Some(hit) = panel.hits.get(index) {
            layout.record(buf, Rect::new(x, 1, end - x, 1), hit.clone());
        }
        x = put(buf, end, 1, gap, width.saturating_sub(end), style);
    }
    put(buf, LEFT, 1, panel.note, width - LEFT, paint.fg(|p| p.dim));
    let rule: String = "─".repeat(usize::from(width));
    put(buf, 0, 2, &rule, width, paint.fg(|p| p.border));
    let visible = usize::from(height.saturating_sub(4));
    // The draw offset clamps to what is there before a line is chosen: past
    // the end paints an emptied body, and the model's clamp after this frame
    // converges the state to the same bound.
    let (start, max) = match panel.scroll {
        Scroll::Top(offset) => {
            let max = panel.body.len().saturating_sub(visible);
            (offset.min(max), max)
        }
        Scroll::Back(back) => peek::window(panel.body.len(), visible, back),
    };
    for (at, (text, style)) in panel.body.iter().skip(start).take(visible).enumerate() {
        put(
            buf,
            LEFT,
            3 + cells(at),
            text,
            width.saturating_sub(LEFT),
            *style,
        );
    }
    layout.settings_page_rows = visible.max(1);
    layout.settings_max_scroll = max;
}

/// The peek panel's clickable close label, right-aligned on its title row.
const PEEK_CLOSE: &str = "Esc close";

/// The rows the peek panel's viewport shows and the furthest it can go back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bounds {
    pub visible: usize,
    pub max_back: usize,
}

/// Paint the peek panel into a bare `buf`: the pure draw the acceptance
/// specs reach. Below the floor nothing is painted and both bounds are 0.
#[must_use]
pub fn draw_peek(view: &peek::View<'_>, look: Option<Look>, buf: &mut Buffer) -> Bounds {
    paint_peek(view, look.as_ref(), buf, &mut Layout::default())
}

/// [`draw_peek`] and what a drag may select in what it painted.
#[must_use]
pub fn draw_peek_map(view: &peek::View<'_>, look: Option<Look>, buf: &mut Buffer) -> (Bounds, Map) {
    let mut layout = Layout::default();
    let bounds = paint_peek(view, look.as_ref(), buf, &mut layout);
    (bounds, layout.map)
}

/// Paint the peek panel over the stripped frame and record its close label
/// and scroll bounds. Pane text is hostile: every line goes through the
/// terminal-text neutraliser before it is wrapped and painted.
pub(crate) fn paint_peek(
    view: &peek::View<'_>,
    look: Option<&Look>,
    buf: &mut Buffer,
    layout: &mut Layout,
) -> Bounds {
    let paint = Paint::of(look);
    let clean = crate::board::terminal_text;
    let title = format!("Peek · {} · {}", clean(view.session), clean(view.seat));
    let note = peek::status(view);
    panel(
        buf,
        layout,
        paint,
        &Panel {
            title: &title,
            note: &note,
            tabs: &[],
            hits: &[],
            close: (PEEK_CLOSE, Hit::PeekClose),
            selected: 0,
            body: &peek_rows(view, paint, buf.area.width),
            scroll: Scroll::Back(view.back),
        },
    );
    let width = buf.area.width;
    let body = Rect::new(
        LEFT,
        3,
        width.saturating_sub(LEFT),
        cells(layout.settings_page_rows),
    );
    layout.selectable(buf, select::Panel::Peek, body);
    Bounds {
        visible: layout.settings_page_rows,
        max_back: layout.settings_max_scroll,
    }
}

/// The peek body's rows: each captured line neutralised, then wrapped to the
/// pane with its continuation rows hanging two cells; before the first answer
/// one dim `loading`.
fn peek_rows(view: &peek::View<'_>, paint: Paint, width: u16) -> Vec<(String, Style)> {
    let text = paint.fg(|p| p.text);
    // `wrap` measures from a one-cell bar the panel does not draw.
    let room = usize::from(width.saturating_sub(LEFT)) + 1;
    let rows = |line: &str| {
        wrap(&crate::board::terminal_text(line), room, 0)
            .into_iter()
            .map(|(gap, piece)| (format!("{}{piece}", " ".repeat(gap)), text))
            .collect::<Vec<_>>()
    };
    match view.body {
        None => vec![("loading".to_owned(), paint.fg(|p| p.dim))],
        Some(peek::Body::Gap(why)) => rows(why),
        Some(peek::Body::Lines(lines)) => lines.iter().flat_map(|line| rows(line)).collect(),
    }
}

/// The overlay body's styled rows: every field through the terminal-text
/// neutraliser, because paths and config values are hostile text.
fn settings_body(
    tab: SettingsTab,
    bodies: &SettingsBodies,
    paint: Paint,
    width: u16,
    icons: bool,
) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let head = paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    let clean = crate::board::terminal_text;
    match tab {
        SettingsTab::Quota => match bodies.quota.as_ref() {
            None => vec![("loading".to_owned(), dim)],
            Some(scopes) if scopes.is_empty() => vec![("no quota scopes read".to_owned(), dim)],
            Some(scopes) => crate::quota::view::layout(
                scopes,
                crate::quota::view::Geometry {
                    width: usize::from(width.saturating_sub(LEFT)),
                    rows: None,
                    icons,
                },
            )
            .into_iter()
            .map(|line| {
                let style = match line.tone {
                    Tone::Plain => text,
                    Tone::Dim => dim,
                    Tone::Head => head,
                    Tone::Low => paint.fg(|p| p.waiting_agent),
                    Tone::Critical => paint.fg(|p| p.needs_you),
                    Tone::Stale => paint.fg(|p| p.stale),
                };
                (clean(&line.text), style)
            })
            .collect(),
        },
        SettingsTab::Config => match &bodies.config {
            ConfigView::Loading => vec![("loading".to_owned(), dim)],
            ConfigView::Unreadable(why) => vec![(clean(why), text)],
            ConfigView::Rows { header, rows } => {
                let mut out = vec![
                    (
                        format!(
                            "session: {}",
                            header
                                .session
                                .as_deref()
                                .map(clean)
                                .as_deref()
                                .unwrap_or("none")
                        ),
                        dim,
                    ),
                    (format!("global: {}", clean(&header.global)), dim),
                ];
                if let Some(current) = header.current_global.as_ref() {
                    out.push((format!("current global: {}", clean(current)), dim));
                }
                out.extend(rows.iter().map(|row| {
                    let mut line = format!(
                        "{} = {}  ({})",
                        row.key,
                        clean(&row.value),
                        row.source.word()
                    );
                    if row.global_only {
                        line.push_str("  global only");
                    }
                    (line, text)
                }));
                out
            }
        },
        SettingsTab::About => match bodies.about.as_ref() {
            None => vec![("loading".to_owned(), dim)],
            Some(about) => about_body(about, text),
        },
        SettingsTab::Instructions => instructions_body(&bodies.instructions, width, paint),
        SettingsTab::Keys => keys_body(paint),
    }
}

/// The write, held and settings keys the help lists beside the browse table.
const WRITE_KEYS: [(&str, &str); 16] = [
    ("Enter", "send the line"),
    ("Alt+Enter ^J", "new line in the draft"),
    ("Esc", "keep the draft, browse"),
    ("^C", "empty draft quits at once"),
    ("^C", "with a draft: arms; again in 2 s quits"),
    ("other key", "disarms an armed ^C"),
    ("Left Right", "move the cursor"),
    ("Home End", "start / end of line"),
    ("^A ^E", "start / end of line"),
    ("Backspace", "erase before the cursor"),
    ("Delete", "erase at the cursor"),
    ("^U", "clear the line"),
    ("paste", "literal text, never Enter; CR becomes LF"),
    ("@seat text", "ask that seat"),
    ("/close [id]", "withdraw your open ask"),
    ("/open seat", "open that seat's pane"),
];
const HELD_KEYS: [(&str, &str); 4] = [
    ("Enter", "try to write again"),
    ("Esc", "browse"),
    ("^C", "quit"),
    ("any other", "swallowed; a paste too"),
];
const MOUSE_KEYS: [(&str, &str); 4] = [
    ("click", "select a session, tab, seat or turn"),
    ("wheel", "scroll 3 rows (the list: 1 session)"),
    ("drag", "a border resizes the sidebar / list"),
    (
        "drag text",
        "select drawn text, copy on release; Esc clears",
    ),
];
const PEEK_KEYS: [(&str, &str); 6] = [
    ("j/k Up/Down", "scroll a line"),
    ("PgUp/PgDn", "scroll a page"),
    ("Esc q v", "close; click Esc close too"),
    ("^C", "quit"),
    ("wheel", "scroll 3 rows per notch"),
    ("any other", "swallowed; nothing reaches the pane"),
];
const SETTINGS_KEYS: [(&str, &str); 10] = [
    ("Tab 1-5", "show that tab"),
    ("j/k Up/Down", "scroll a line"),
    ("PgUp/PgDn", "scroll a page"),
    ("?", "show these keys"),
    ("Esc q s", "close"),
    ("^C", "quit"),
    ("click tab", "show that tab"),
    ("click gear", "or Esc close label: closes it"),
    ("wheel", "scroll 3 rows per notch"),
    ("any other", "swallowed; a paste too"),
];

/// The Keys tab's rows: every key of browse, write, held and the overlay.
fn keys_body(paint: Paint) -> Vec<(String, Style)> {
    let (text, head) = (paint.fg(|p| p.text), paint.fg(|p| p.title));
    let head = head.add_modifier(Modifier::BOLD);
    let mut rows = vec![("Browse".to_owned(), head)];
    rows.extend(
        BROWSE_KEYS
            .iter()
            .map(|(keys, does)| (format!("{keys:<12}{does}"), text)),
    );
    rows.push((format!("{:<12}quit at once", "^C"), text));
    rows.push(("Any other key cancels qq / oo.".to_owned(), text));
    let sections = [
        ("Write (Enter or a paste starts it)", &WRITE_KEYS[..]),
        ("Held (writing was refused)", &HELD_KEYS[..]),
        ("Peek (v on a highlighted seat)", &PEEK_KEYS[..]),
        ("Settings (the gear, or s)", &SETTINGS_KEYS[..]),
        ("Mouse (outside Settings)", &MOUSE_KEYS[..]),
    ];
    for (title, keys) in sections {
        rows.push((title.to_owned(), head));
        rows.extend(
            keys.iter()
                .map(|(keys, does)| (format!("{keys:<12}{does}"), text)),
        );
    }
    rows
}

/// The about tab's rows: versions, paths, the recorded server, plain links.
fn about_body(about: &AboutFacts, text: Style) -> Vec<(String, Style)> {
    let clean = crate::board::terminal_text;
    let mut rows = vec![
        (format!("ae   {}", crate::version_line()), text),
        (
            format!(
                "tmux {} ({})",
                clean(&about.tmux_version),
                about.tmux_verdict
            ),
            text,
        ),
        (format!("state root   {}", clean(&about.state_root)), text),
        (format!("config   {}", clean(&about.config_file)), text),
        (format!("server   {}", clean(&about.server)), text),
    ];
    rows.extend(
        SETTINGS_LINKS
            .iter()
            .map(|(label, link)| (format!("{label}   {link}"), text)),
    );
    rows
}

/// The instructions tab's rows: the render-now line, the custom instructions,
/// each generic rule once under its audience and one line per seat naming the
/// rules it receives. Every field is neutralised first and then wrapped to the
/// panel, a continuation row hanging two cells; nothing is cut here, the panel
/// scrolls.
fn instructions_body(view: &InstructionsView, width: u16, paint: Paint) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let head = paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    // `wrap` keeps a one-cell bar free ahead of every row.
    let room = usize::from(width.saturating_sub(LEFT)) + 1;
    let rows = |body: &str, style: Style| -> Vec<(String, Style)> {
        wrap(&crate::board::terminal_text(body), room, 0)
            .into_iter()
            .map(|(gap, piece)| (format!("{}{piece}", " ".repeat(gap)), style))
            .collect()
    };
    let protocol = match view {
        InstructionsView::Loading => return vec![("loading".to_owned(), dim)],
        InstructionsView::Gap(why) => return rows(why, text),
        InstructionsView::Ready(protocol) => protocol,
    };
    let mut out = rows(&format!("session: {}", protocol.session), dim);
    out.extend(rows(
        &format!(
            "<session>, <helpers> and <owner line> stand for what a launch fills in per seat. Rendered now by {}: the text a launch injects today. A running seat holds the text of its own launch time.",
            crate::version_line()
        ),
        dim,
    ));
    out.push((String::new(), text));
    out.extend(rows("Custom instructions ([prompt] instructions)", head));
    if let Some(custom) = &protocol.custom {
        out.extend(rows(
            &format!("source: {} {}", custom.source.word(), custom.file),
            dim,
        ));
        out.extend(rows(&custom.text, text));
    } else {
        let named = if protocol.files.is_empty() {
            "no config file".to_owned()
        } else {
            protocol.files.join(", ")
        };
        out.extend(rows(&format!("none in {named}"), dim));
    }
    for rule in &protocol.rules {
        out.push((String::new(), text));
        out.extend(rows(&format!("Rules for {}", rule.audience.word()), head));
        if let Some(owner) = &rule.owner {
            out.extend(rows(owner, dim));
        }
        out.extend(rows(&rule.text, text));
    }
    out.push((String::new(), text));
    out.extend(rows("Seats", head));
    for seat in &protocol.seats {
        let receives = match &seat.receives {
            Ok(audiences) => audiences
                .iter()
                .map(|audience| audience.word())
                .collect::<Vec<_>>()
                .join(" + "),
            Err(why) => format!("gap: {why}"),
        };
        let line = format!("{} · {} · {} · {receives}", seat.name, seat.slot, seat.role);
        out.extend(rows(&line, text));
    }
    out
}

// ---------------------------------------------------------------------------
// cells
// ---------------------------------------------------------------------------

/// `text · <age>` in `room` cells: a clipped text keeps its age.
fn aged(text: &str, since: Option<i64>, room: u16) -> String {
    let room = usize::from(room);
    let tail = format!(" · {}", age(since));
    let (text_cells, tail_cells) = (text.chars().count(), tail.chars().count());
    if text_cells + tail_cells <= room {
        return format!("{text}{tail}");
    }
    // A whole text beats its age; a text clipped anyway keeps its age (F7),
    // while one character and its ellipsis still fit beside it.
    if text_cells <= room {
        return text.to_owned();
    }
    match room.checked_sub(tail_cells) {
        Some(left) if left >= 2 => format!("{}{tail}", clip_head(text, left)),
        _ => clip_head(text, room),
    }
}

/// A count of cells as a coordinate.
pub(super) fn cells(count: usize) -> u16 {
    u16::try_from(count).unwrap_or(u16::MAX)
}

/// Whether (x, y) lies inside `buf`.
fn inside(buf: &Buffer, x: u16, y: u16) -> bool {
    let area = buf.area;
    x >= area.left() && x < area.right() && y >= area.top() && y < area.bottom()
}

/// Write `text` at (x, y), at most `max` cells and never past the buffer;
/// answers the column after it.
pub(super) fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, max: u16, style: Style) -> u16 {
    if !inside(buf, x, y) {
        return x;
    }
    buf.set_stringn(x, y, text, usize::from(max), style).0
}

/// [`put`] for a line of styled spans.
fn put_line(buf: &mut Buffer, x: u16, y: u16, line: &Line<'_>, max: u16) -> u16 {
    if !inside(buf, x, y) {
        return x;
    }
    buf.set_line(x, y, line, max).0
}

#[cfg(test)]
mod tests {
    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::{Position, Rect};
    use ratatui_core::style::Modifier;

    use super::{
        Composer, LEFT, Paint, Screen, aged, dragged_to, draw, instructions_body, keys_body,
    };
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
            client: "claude".to_owned(),
            model: "Sonnet 5.5".to_owned(),
            effort: "xhigh".to_owned(),
            drift: false,
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
            cursor_row: 0,
            before: String::new(),
            anchor: String::new(),
        };
        let compose = Composer::Home {
            home: "s1",
            speaker: "lead",
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

    /// Frame many r17-r18: a stopped session's first line says how long it
    /// has been stopped (`stopped 2d`), its second counts its seats,
    /// `3 seats, not running` at column 6, and one seat reads singular.
    #[test]
    fn a_stopped_row_counts_its_seats_as_the_frame_does() {
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
        let first = spot(&buf, "1 s1").expect("the first stopped row").0;
        assert!(
            line(&buf, first)[..44]
                .concat()
                .trim_end()
                .ends_with("stopped 2d"),
            "{:?}",
            line(&buf, first).concat()
        );
        assert_eq!(spot(&buf, "3 seats, not running").map(|at| at.1), Some(6));
        assert_eq!(spot(&buf, "1 seat, not running").map(|at| at.1), Some(6));
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

    /// At 100x30 the sidebar is narrow: an ask's reason row keeps its age
    /// inside the column, ending one cell before the two blank cells the `│`
    /// stands after (frame needs-you r17 geometry: content stops at end-1 = 31).
    #[test]
    fn a_narrow_reason_row_keeps_its_age_inside_the_column() {
        let mut shot = Shot::new();
        shot.overview.open = vec![Open {
            seat: "ghost".to_owned(),
            text: "resume at scope checks after the fixtures land on main".to_owned(),
            age_secs: Some(360),
        }];
        let buf = shot.draw(100, 30, READ_ONLY);
        let (y, x) = spot(&buf, "resume").expect("the reason row");
        assert_eq!(x, 4);
        let text: String = line(&buf, y)[..34].concat();
        assert!(text.trim_end().ends_with("6m"), "the age is kept: {text:?}");
        assert_eq!(last_ink(&buf, y, 34), Some(31), "{text:?}");
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

    /// The `↓ newer turns below` hint means newer turns are hidden below:
    /// none for a lane that fits, even asked to scroll; one on a long lane
    /// scrolled a page up.
    #[test]
    fn the_newer_turns_hint_shows_only_when_newer_turns_are_hidden() {
        const HINT: &str = "newer turns below";
        let mut shot = Shot::new();
        shot.lane = turns(2);
        shot.key(crate::app::model::Key::PageUp);
        assert_eq!(
            spot(&shot.draw(160, 45, READ_ONLY), HINT),
            None,
            "a lane that fits"
        );
        shot.lane = turns(150);
        let buf = shot.draw(160, 45, READ_ONLY);
        assert!(spot(&buf, HINT).is_some(), "a long lane a page up");
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
        assert!(header.trim_end().ends_with("stopped 2d"), "{header:?}");
        assert!(!header.contains("active"), "{header:?}");
    }

    /// Frame calm r1 draws `feat/auth-v2  ·  active 1m` when it fits after
    /// the middle; when it would not fit after a two-cell gap the branch is
    /// dropped, the activity stays right-aligned to the column's end (the
    /// rule's last cell) and the middle is whole. At 160 the middle starts
    /// @53: 77 cells leave cols 130-131 blank before the 26-cell branch @132,
    /// 78 leave one.
    #[test]
    fn the_header_drops_the_branch_before_it_overlaps() {
        let branch = "feat/auth-v2  ·  active 1m";
        for (middle, fits) in [(77, true), (78, false)] {
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
                    shown.ends_with("active 1m") && !shown.contains("feat/auth-v2"),
                    "{header:?}"
                );
            }
        }
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
        shot.agents = Some(sweep_seats());
        let buf = shot.draw(100, 24, READ_ONLY);
        assert_eq!(
            spot(&buf, "↓ 4 more rows"),
            Some((23, 2)),
            "the body's last row"
        );
        let dim = super::colour(crate::theme::Palette::DARCULA.dim);
        assert_eq!(buf[(2, 23)].fg, dim);
        let tall = shot.draw(100, 40, READ_ONLY);
        assert_eq!(spot(&tall, "more rows"), None, "a body that fits");
        shot.fleet = frame_fleet();
        // The smallest sidebar frame gives the body four rows, the last of
        // them the line naming what it hides.
        let smallest = shot.draw(100, 20, READ_ONLY);
        assert_eq!(spot(&smallest, "↓ 15 more rows").map(|at| at.0), Some(19));
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
            cursor_row: 0,
            before: String::new(),
            anchor: String::new(),
        };
        let composing = Composer::Home {
            home: "api",
            speaker: "lead",
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

    /// The closed hint row carries the composer's hint, the mode's keys and
    /// the version/gear; held has no hint and no keys. A panel's footer
    /// keeps its mode word and its one hint.
    #[test]
    fn the_hint_row_and_the_panel_footer_carry_at_most_one_hint() {
        let mut shot = Shot::new();
        let draft = drafted(&["a draft"]);
        let tail = format!("{} \u{2699}", crate::version_line());
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
                "Enter sends · Alt+Enter ^J newline · Esc browses",
            ),
            (refused, ""),
        ];
        for (composer, words) in cases {
            let (row, _) = shot.footer_at(100, composer, true);
            let head = row
                .strip_suffix(tail.as_str())
                .expect("version and gear end it");
            assert_eq!(head.trim_end(), format!("  {words}").trim_end(), "{words}");
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
        let tail = format!("{} {gear}", crate::version_line());
        let head = "  Enter writes · ? keys";
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
            cursor_row: rows.len().saturating_sub(1),
            before: rows.last().map(|row| (*row).to_owned()).unwrap_or_default(),
            anchor: String::new(),
        }
    }

    /// A composer writing `view`.
    fn writing<'a>(view: &'a View, draft: &'a str) -> Composer<'a> {
        Composer::Home {
            home: "api",
            speaker: "lead",
            view: Some(view),
            draft,
        }
    }

    /// A composer browsing with `draft` kept.
    fn browsing(draft: &str) -> Composer<'_> {
        Composer::Home {
            home: "api",
            speaker: "lead",
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
            let (ay, ax) = spot(&buf, "to api › lead").expect("the address");
            assert_eq!(ay, height - 4, "{size}: over two draft rows and the hint");
            assert_eq!(
                row_from(&buf, ay, ax),
                "to api › lead",
                "{size}: alone on its row"
            );
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
            let (ay, ax) = spot(&buf, "to api › lead").expect("the address");
            assert_eq!(ay, 27, "{draft:?}");
            assert_eq!(row_from(&buf, ay, ax), "to api › lead", "{draft:?}");
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
                shown.ends_with(&format!("{} \u{2699}", crate::version_line())),
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
                shown.ends_with(&format!("{} \u{2699}", crate::version_line())),
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
        let (ay, ax) = spot(&buf, "to api › lead").expect("the address");
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
            spot(&buf, &crate::version_line()),
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
        let (ay, ax) = spot(&buf, "to api › lead").expect("the address");
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
        assert_eq!(spot(&buf, "to api › lead"), Some((5, 2)));
        assert_eq!(spot(&buf, "Enter writes"), Some((7, 2)));
        // The sidebar warning rides the header rule's left; the rule stands
        // past it, and the composer's own rule stands over the address.
        assert_eq!(buf[(35, 1)].symbol(), "─", "the header rule");
        assert_eq!(buf[(2, 4)].symbol(), "─", "the composer's rule");
        assert!(line(&buf, 0).concat().contains("api"), "the header");
        assert_eq!(
            spot(&buf, "newer turns"),
            Some((3, 4)),
            "the hint under the lane's one row, never over the header"
        );
        assert!(
            row_from(&buf, 7, 0)
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
        let (ay, ax) = spot(&buf, "to api › lead").expect("the address");
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
                speaker: "lead",
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
            assert!(header.ends_with("active 1m"), "{width}x18: {header:?}");
            assert_eq!(
                header.contains("feat/auth-v2  ·  "),
                shown,
                "{width}x18: {header:?}"
            );
        }
    }

    /// Frame needs-you 100x30: four sessions sit three rows apart (two lines
    /// and a gap), measured from the first.
    #[test]
    fn four_sessions_keep_three_rows_each_at_100x28() {
        let mut shot = Shot::new();
        shot.fleet.rows = (1..=4)
            .map(|at| row(&format!("s{at}"), at, false))
            .collect();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(100, 28, READ_ONLY);
        let ys: Vec<u16> = (1..=4)
            .map(|at| spot(&buf, &format!("{at} s{at}")).expect("a session row").0)
            .collect();
        assert_eq!(ys, [ys[0], ys[0] + 3, ys[0] + 6, ys[0] + 9], "{ys:?}");
    }

    /// The window a long fleet shows. Frame many 100x30: nine sessions show
    /// 1-5 and `↓ 4 more`. At 160x43 the list is 22 rows (two thirds of the
    /// rows it shares with the tab body, two a session) with the more row
    /// inside it, so fourteen sessions show 10 and the more row.
    #[test]
    fn a_long_fleet_shows_the_window_the_frames_do() {
        for (width, height, count, shown, more) in
            [(100, 28, 9, 5, "↓ 4 more"), (160, 43, 14, 10, "↓ 4 more")]
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
                last + 2,
                "{size}: the more row follows the last session's two lines"
            );
            let tabs = spot(&buf, "Agents").expect("the heading").0;
            assert_eq!(tabs, y + 2, "{size}: one blank row, then the heading");
        }
        let mut empty = Shot::new();
        empty.fleet.rows.clear();
        empty.model = Model::new(&empty.fleet);
        let buf = empty.draw(160, 43, READ_ONLY);
        let none = spot(&buf, "No sessions.").expect("the empty list").0;
        assert_eq!(
            spot(&buf, "Agents").map(|at| at.0),
            Some(none + 2),
            "one blank row, then the heading"
        );
        // (e)/(f) frame many 100x30 r14: the more row counts the needy rows it hides.
        for (select, more) in [
            (None, "↓ 4 more, ⚠ 2"),
            (Some(9), "↑ 4 more, ⚠ 1"),
            (Some(6), "↑ 1 · ↓ 3 more, ⚠ 1"),
        ] {
            let mut shot = Shot::new();
            shot.fleet = frame_fleet();
            shot.model = Model::new(&shot.fleet);
            if let Some(digit) = select {
                let _ = shot
                    .model
                    .key(crate::app::model::Key::Digit(digit), &shot.fleet, false);
            }
            let buf = shot.draw(100, 28, READ_ONLY);
            let (y, _) = spot(&buf, "more").expect("the more row");
            let drawn = line(&buf, y)[..34].concat();
            assert_eq!(drawn.trim(), more, "select {select:?}");
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
        let attention = line(&buf, 2).concat();
        assert!(attention[..40].trim().is_empty(), "{attention:?}");
        let mut shot = Shot::new();
        shot.fleet.rows = vec![
            row("a", 1, false),
            row("needy-0010", 2, true),
            row("needy-00011", 3, true),
        ];
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        assert_eq!(buf[(41, 2)].symbol(), " ", "no key marker at end-1");
        assert_eq!(
            buf[(40, 2)].symbol(),
            " ",
            "a blank before it: {:?}",
            line(&buf, 2).concat()
        );
        // (c) several long names still leave the end cells blank.
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
            buf[(40, 2)].symbol(),
            " ",
            "the end cells stay blank: {:?}",
            line(&buf, 2).concat()
        );
        // (d) only needy rows outside the view contribute to the suffix.
        let mut shot = Shot::new();
        shot.fleet.rows = (1..=14)
            .map(|at| {
                let name = match at {
                    3 => "infra".to_owned(),
                    13 => "billing".to_owned(),
                    14 => "ops".to_owned(),
                    _ => format!("s{at}"),
                };
                let mut row = row(&name, at, matches!(at, 3 | 13 | 14));
                if at == 14 {
                    row.mark = Mark::Dead;
                }
                row
            })
            .collect();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        assert_eq!(buf[(39, 2)].symbol(), "↓", "off-screen attention below");
        assert_eq!(buf[(41, 2)].symbol(), " ", "no key marker");
    }

    /// Frame agents r20-r35: a seat's state is right-aligned at the
    /// sidebar's end with blanks before it, never against the name. A state
    /// that would touch the name is dropped; one blank between is enough.
    /// At 160 the end is col 42: `● ` plus a 30-cell name ends at col 33,
    /// `working` takes cols 35-41.
    #[test]
    fn an_agent_state_never_touches_its_name() {
        for (cells, shown) in [(30, true), (31, false)] {
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
            let (y, _) = spot(&buf, &"n".repeat(cells)).expect("the seat row");
            let row = line(&buf, y)[..44].concat();
            assert_eq!(
                row.contains("working"),
                shown,
                "a {cells}-cell name: {row:?}"
            );
        }
    }

    /// Frame many 160: a needy session's bar fills its own two rows and no
    /// other (r8-r9 infra, r14-r15 billing, r18-r19 ops).
    #[test]
    fn a_needy_bar_covers_its_own_two_rows() {
        let mut shot = Shot::new();
        shot.fleet = frame_fleet();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        let barred: Vec<u16> = (0..43).filter(|y| buf[(0, *y)].symbol() == "│").collect();
        assert_eq!(barred, [8, 9, 14, 15, 18, 19]);
    }

    /// A long name uses its first line; counts remain below inside the rule.
    #[test]
    fn a_long_session_name_keeps_its_counts_on_the_second_line() {
        let mut shot = Shot::new();
        shot.fleet.rows = vec![row(&"n".repeat(60), 1, false)];
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        assert_eq!(line(&buf, 4)[6..42].concat(), "n".repeat(36));
        assert!(line(&buf, 5)[6..42].concat().starts_with("●3 ✓B12"));
        assert_eq!(buf[(44, 4)].symbol(), "│");
    }

    /// The home tag follows the name after two blanks, on that row alone;
    /// a 22-cell name leaves one blank before the edge, a 23-cell one drops it.
    #[test]
    fn this_window_marks_the_home_row_alone_inside_the_name_line() {
        let mut shot = Shot::new();
        shot.fleet = frame_fleet();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        let sidebar = |buf: &Buffer, y: u16| line(buf, y)[..44].concat();
        let tagged: Vec<u16> = (0..43)
            .filter(|y| sidebar(&buf, *y).contains("this window"))
            .collect();
        assert_eq!(tagged, [4], "the home row alone");
        assert!(
            sidebar(&buf, 4).starts_with("  ● 1 api  this window"),
            "{:?}",
            sidebar(&buf, 4)
        );
        for (cells, shown) in [(22, true), (23, false)] {
            let mut shot = Shot::new();
            shot.fleet.rows = vec![row(&"n".repeat(cells), 1, false)];
            shot.model = Model::new(&shot.fleet);
            let first = sidebar(&shot.draw(160, 43, READ_ONLY), 4);
            assert_eq!(
                first.contains("this window"),
                shown,
                "a {cells}-cell name: {first:?}"
            );
        }
    }

    /// A crowded status uses its cells for the actionable question prefix.
    #[test]
    fn a_sessions_second_line_keeps_its_question_prefix() {
        let mut shot = Shot::new();
        shot.fleet = frame_fleet();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        for y in [5, 9] {
            let cells = line(&buf, y);
            assert!(cells[6..42].concat().contains("a question"));
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
        }
        .expect("the border is drawn");
        let drag = Drag { edge, from: at, at };
        shot.model.grab(drag);
        let mouse = match edge {
            Edge::Sidebar => Mouse {
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
        let size = dragged_to(drag, mouse, buf.area, shot.fleet.rows.len());
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

    /// Ruling 5b: a 100x20 pane of 13 sessions draws its list rule on row 15
    /// (two-row steps, four shown, the more row, tabs on 14). A press there
    /// moved nowhere keeps it; dragged up and back down past it, it stops on
    /// 15, never 16.
    #[test]
    fn a_short_pane_keeps_its_undragged_list_rule_reachable() {
        let mut shot = sessions(13);
        let first = shot.draw(100, 20, READ_ONLY);
        assert_eq!(horizontal(&first), Some(15), "the undragged rule");
        let still = drag(&mut shot, &first, Edge::List, 15);
        assert_eq!(horizontal(&still), Some(15), "a press moved nowhere");
        let up = drag(&mut shot, &still, Edge::List, 12);
        assert_eq!(horizontal(&up), Some(12));
        let down = drag(&mut shot, &up, Edge::List, 19);
        assert_eq!(horizontal(&down), Some(15), "never past the undragged rule");
    }

    /// Ruling 5a: the list keeps one session's rows — two alone, two and the
    /// more row among several — and its dragged-down stop keeps the row
    /// budget it had, the body taking the rows the frame gave back.
    #[test]
    fn a_dragged_list_keeps_one_session_and_its_row_budget() {
        for (count, least) in [(1, 8), (2, 9), (13, 9)] {
            let mut shot = sessions(count);
            let first = shot.draw(160, 45, READ_ONLY);
            let top = drag(&mut shot, &first, Edge::List, 0);
            assert_eq!(horizontal(&top), Some(least), "{count} sessions");
            let bottom = drag(&mut shot, &top, Edge::List, 44);
            // Border 38: body rows 39 to 44, the pane's last.
            assert_eq!(horizontal(&bottom), Some(38), "{count} sessions");
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
            assert_eq!(at(column, 1), Some((Edge::Sidebar, 44)), "column {column}");
        }
        assert_eq!(at(46, 1), None);
        assert_eq!(at(42, 1), None);
        assert_eq!(at(43, 5), None, "a session row is a target");
        assert_eq!(at(44, 5), Some((Edge::Sidebar, 44)));
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

    /// Two seats in the Agents body, by name.
    fn seat_facts(names: &[&str]) -> Facts {
        let Facts::Seats { id, agents } = sweep_seats() else {
            unreachable!("the sweep draws seats")
        };
        let seat_of = |name: &&str| PickerAgent {
            name: (*name).to_owned(),
            ..agents[0].clone()
        };
        Facts::Seats {
            id,
            agents: names.iter().map(seat_of).collect(),
        }
    }

    /// One frame of `shot` at `width` x 45 in `look`.
    fn framed(shot: &Shot, width: u16, look: Option<&Look>) -> (Buffer, super::Layout) {
        let screen = Screen {
            fleet: &shot.fleet,
            model: &shot.model,
            overview: &shot.overview,
            selected: Some(&shot.entry),
            pair: &shot.pair,
            agents: shot.agents.as_ref(),
            lane: &shot.lane,
            composer: READ_ONLY,
            look: look.copied(),
            zone: None,
            now: Timestamp::from_epoch(PIN_NOW),
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 45));
        let layout = super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
        (buf, layout)
    }

    /// The rows of `buf` whose first cell is `glyph`.
    fn marked(buf: &Buffer, glyph: &str) -> Vec<u16> {
        (0..buf.area.height)
            .filter(|y| buf[(0, *y)].symbol() == glyph)
            .collect()
    }

    /// R4/R5: the focused seat's rows carry the marker in column 0, the first
    /// seat drawn is focused until the model names a drawn one, a seat row is
    /// a click target that ends at the body, and the keys row names no seat
    /// key.
    #[test]
    fn the_focused_seats_rows_are_marked() {
        let mut shot = Shot::new();
        shot.agents = Some(seat_facts(&["lead", "colead"]));
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(layout.seats, ["lead", "colead"]);
        assert_eq!(layout.focused.as_deref(), Some("lead"));
        let first = *marked(&buf, "▸").first().expect("a marked row");
        assert_eq!(marked(&buf, "▸"), [first], "one glyph, on the first row");
        let reversed = |y| buf[(8, y)].modifier.contains(Modifier::REVERSED);
        assert!(reversed(first) && reversed(first + 1), "no colour: reverse");
        assert!(
            !reversed(first + 2) && !reversed(first + 3),
            "the other seat"
        );
        let seat = |buf: &Buffer, y| line(buf, y).concat();
        assert!(seat(&buf, first).contains("lead"), "its first row");
        assert!(seat(&buf, first + 2).contains("colead"), "the next seat");
        let keys: String = line(&buf, 44).concat();
        assert!(
            keys.contains("? keys") && !keys.contains("oo open seat") && !keys.contains("n/p"),
            "{keys}"
        );
        let at = Mouse {
            kind: MouseKind::Click,
            column: 8,
            row: first + 2,
        };
        assert!(matches!(layout.hit(at), Some(super::Hit::Seat(name)) if name == "colead"));
        let rule = (0..160)
            .find(|x| buf[(*x, at.row)].symbol() == "│")
            .expect("the rule");
        let seat = |column| {
            matches!(
                layout.hit(Mouse { column, ..at }),
                Some(super::Hit::Seat(_))
            )
        };
        assert!(seat(rule - 3) && !seat(rule - 2), "a seat ends at the body");
        shot.model.set_focus(Some("colead".to_owned()));
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(marked(&buf, "▸"), [first + 2]);
        assert_eq!(layout.focused.as_deref(), Some("colead"));
        shot.model.set_focus(Some("ghost".to_owned()));
        let (_, layout) = framed(&shot, 160, None);
        assert_eq!(layout.focused.as_deref(), Some("lead"), "an undrawn name");
        let ascii = Look::read("off", "darcula", "on", "on");
        let (buf, _) = framed(&shot, 160, Some(&ascii));
        assert_eq!(marked(&buf, ">"), [first]);
        // Scrolled so the focused seat's name is gone, its first DRAWN row
        // takes the glyph.
        let names: Vec<String> = (0..20).map(|n| format!("s{n:02}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        shot.agents = Some(seat_facts(&names));
        shot.model.set_focus(None);
        assert!(shot.model.wheel_body(false, 9));
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(layout.focused.as_deref(), Some("s01"));
        assert_eq!(marked(&buf, "▸"), [first], "its first drawn row");
        assert!(
            line(&buf, first).concat().contains("claude"),
            "not its name"
        );
    }

    /// R3: with a look, the focused seat's rows share the selected ground out
    /// to the body's end and the glyph is the selected ink, never the title
    /// or a state hue; the next seat keeps the sidebar's ground. On the
    /// ground only the selected ink and the marks' accents are drawn, so the
    /// pairs are the ones `theme`'s contrast bar already measures.
    #[test]
    fn the_focused_seat_sits_on_the_selected_ground_in_every_palette() {
        let mut shot = Shot::new();
        shot.agents = Some(seat_facts(&["lead", "colead"]));
        for name in ["darcula", "a", "b"] {
            let look = Look::read("on", name, "on", "on");
            let (buf, _) = framed(&shot, 160, Some(&look));
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
            for y in [first, first + 1] {
                for x in 0..rule - 2 {
                    assert_eq!(buf[(x, y)].bg, ground, "{name} ({x},{y})");
                }
                let mark = super::colour(palette.accent(Mark::WaitingAgent));
                let drawn: Vec<_> = (0..rule - 2)
                    .map(|x| &buf[(x, y)])
                    .filter(|cell| cell.symbol() != " ")
                    .collect();
                assert!(!drawn.is_empty(), "{name} row {y}");
                for cell in drawn {
                    let ok = cell.fg == ink || cell.fg == mark;
                    assert!(ok, "{name}: {:?} {:?}", cell.symbol(), cell.fg);
                }
            }
            let sidebar = super::colour(palette.base);
            for y in [first + 2, first + 3] {
                assert_eq!(buf[(5, y)].bg, sidebar, "{name}: the next seat");
            }
        }
    }

    /// R1/R4: a seat's second row is its client, the model its frame drew and
    /// the effort, each only when known, and never its profile name.
    #[test]
    fn the_agents_second_row_is_client_model_and_effort_never_the_profile() {
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
        };
        shot.agents = Some(Facts::Seats {
            id: "$1".to_owned(),
            agents: vec![
                seat("seat-a", "Sonnet 5.5", "xhigh"),
                seat("seat-b", "", ""),
                seat("seat-c", "", "max"),
                PickerAgent {
                    client: String::new(),
                    ..seat("seat-d", "", "")
                },
            ],
        });
        let (buf, _) = framed(&shot, 160, None);
        let below = |name: &str| {
            let y = spot(&buf, name).expect(name).0 + 1;
            line(&buf, y)[..34].concat().trim().to_owned()
        };
        assert_eq!(below("seat-a"), "claude · Sonnet 5.5 · xhigh");
        assert_eq!(below("seat-b"), "claude", "no placeholder");
        assert_eq!(below("seat-c"), "claude · max");
        assert_eq!(below("seat-d"), "", "the row stays: the geometry is stable");
        let frame: String = (0..45).map(|y| line(&buf, y).concat()).collect();
        assert!(!frame.contains("sonnet55x"), "the profile is not drawn");
    }

    /// R7: a seat that waits on the human keeps its reason on its own rows;
    /// an ask from a seat the roster lacks is its own entry after the roster,
    /// so none is lost; and no Overview or Tab word is left in the frame.
    #[test]
    fn a_seat_waiting_on_the_human_keeps_its_reason_on_its_agents_rows() {
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
        let at = |name: &str| spot(&buf, name).expect(name).0;
        let sidebar = |y| line(&buf, y)[..40].concat().trim().to_owned();
        assert_eq!(at("seat-b"), at("seat-a") + 2, "seat-a asks nothing");
        assert_eq!(sidebar(at("seat-b") + 2), "Approve X? · 4m");
        assert_eq!(layout.seats, ["seat-a", "seat-b", "ghost"]);
        assert!(at("ghost") > at("seat-b") + 2, "after the roster");
        assert!(sidebar(at("ghost")).starts_with("⚠ ghost"), "its own entry");
        assert_eq!(sidebar(at("ghost") + 1), "registry down · 1h");
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
        has("Write", &["^C", "empty", "quits at once"]);
        has("Write", &["^C", "draft", "2 s"]);
        has("Write", &["other key", "disarms"]);
        has("Settings", &["gear", "Esc close", "closes"]);
        has("Settings", &["wheel", "3 rows"]);
        has("Mouse", &["click", "session", "tab"]);
        has("Mouse", &["wheel", "3 rows"]);
        has("Mouse", &["drag", "border"]);
    }
    // Independent S1 acceptance: frozen by appdeclutter-nav; install unchanged.
    fn declutter_frame(shot: &Shot, width: u16, icons: bool) -> (Buffer, super::Layout) {
        let mut screen = shot.screen(READ_ONLY);
        screen.look = Some(Look::read(
            if icons { "on" } else { "off" },
            "darcula",
            "on",
            "on",
        ));
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 28));
        let layout = super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
        (buf, layout)
    }

    fn declutter_line(buf: &Buffer, layout: &super::Layout, y: u16) -> String {
        line(buf, y)[..usize::from(layout.list.width)].concat()
    }

    #[test]
    fn declutter_counts_below_name_hide_goals_and_keep_home_hits_and_cells() {
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
                let name = declutter_line(&buf, &layout, 4);
                let below = declutter_line(&buf, &layout, 5);
                let counts = if icons { "●3 ✓B12" } else { "*3 +B12" };
                assert!(name.contains("界 api") && name.contains("this window"));
                assert!(!name.contains(counts), "counts belong below: {name:?}");
                assert!(below.contains(counts), "{width}/{icons}: {below:?}");
                assert!(!below.contains("goal"), "{below:?}");
                for y in [4, 5] {
                    for column in [0, super::NAME, layout.list.width - 1] {
                        assert!(
                            matches!(layout.hit(Mouse { kind: MouseKind::Click, column, row: y }), Some(super::Hit::Session(name)) if name == "界api")
                        );
                    }
                    assert_eq!(buf[(layout.list.width, y)].symbol(), "│");
                }
                shot.fleet.rows[0].line2 = Line2::NoGoal;
                let (buf, layout) = declutter_frame(&shot, width, icons);
                let below = declutter_line(&buf, &layout, 5);
                assert!(below.contains(counts) && !below.contains("goal"));
                shot.fleet.rows[0].counts = Counts::Unknown;
                let (buf, layout) = declutter_frame(&shot, width, icons);
                assert!(declutter_line(&buf, &layout, 5).contains("agents ?"));
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
        for (icons, expected) in [
            (true, "▣ 9  ◈ 22+?  ● 12+?  ⚠ 3"),
            (false, "# 9  @ 22+?  * 12+?  ! 3"),
        ] {
            let (buf, layout) = declutter_frame(&shot, 100, icons);
            assert_eq!(declutter_line(&buf, &layout, 0).trim(), expected);
            let cue = declutter_line(&buf, &layout, 2);
            assert_eq!(cue.trim(), "↓");
            shot.key(crate::app::model::Key::NextNeed);
            assert_eq!(shot.model.selected(), Some("s02"));
            shot.key(crate::app::model::Key::NextNeed);
            let (buf, layout) = declutter_frame(&shot, 100, icons);
            let cue = declutter_line(&buf, &layout, 2);
            assert_eq!(cue.trim(), "↑↓");
            shot.key(crate::app::model::Key::Esc);
        }
        shot.fleet.rows.truncate(1);
        shot.model = Model::new(&shot.fleet);
        for (counts, expected) in [
            (Counts::Marks(Vec::new()), "# 1  @ 0  * 0  ! 0"),
            (Counts::Unknown, "# 1  @ 0+?  * 0+?  ! 0"),
        ] {
            shot.fleet.rows[0].counts = counts;
            let (buf, layout) = declutter_frame(&shot, 100, false);
            assert_eq!(declutter_line(&buf, &layout, 0).trim(), expected);
            assert!(declutter_line(&buf, &layout, 2).trim().is_empty());
            if matches!(shot.fleet.rows[0].counts, Counts::Marks(_)) {
                assert!(declutter_line(&buf, &layout, 5).contains("no agents"));
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
        let header = declutter_line(&buf, &layout, 0);
        assert_eq!(
            header.split_whitespace().collect::<Vec<_>>(),
            ["#", "2", "!", "1"]
        );
        shot.fleet.rows[0].counts = Counts::Marks(vec![
            (Mark::Working, false, 3),
            (Mark::Done, false, usize::MAX - 3),
        ]);
        let (buf, layout) = declutter_frame(&shot, 90, false);
        assert_eq!(declutter_line(&buf, &layout, 0).trim(), "# 2 * 3+? ! 1");
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
            declutter_line(&buf, &layout, 0).trim(),
            "# 130 @ 1150+? * 1000+? ! 100"
        );
    }

    #[test]
    fn declutter_crowded_counts_preserve_actionable_question_and_seat_down_cues() {
        let mut shot = Shot::new();
        shot.fleet.rows[0].counts = Counts::Marks(
            Mark::BY_URGENCY
                .into_iter()
                .map(|mark| (mark, false, 12))
                .collect(),
        );
        for (status, cue) in [
            (
                Line2::Question {
                    text: "approve deploy now".to_owned(),
                    age_secs: Some(60),
                },
                "approve",
            ),
            (Line2::SeatDown, "down"),
        ] {
            shot.fleet.rows[0].line2 = status;
            let (buf, layout) = declutter_frame(&shot, 90, true);
            let below = declutter_line(&buf, &layout, 5);
            assert!(below.contains(cue) && below.contains("✖12"), "{below:?}");
            assert!(
                !below.contains("✓12") && !below.contains("·12"),
                "low urgency counts yield: {below:?}"
            );
        }
    }

    #[test]
    fn declutter_questions_clip_cells_and_keep_whole_graphemes_and_age() {
        for icons in [false, true] {
            for (text, count, start, expected) in [
                ("界".repeat(10), 12, 15, "界 ".repeat(8) + "…"),
                ("e\u{301}".repeat(23), 1, 10, "e\u{301}".repeat(21) + "…"),
                ("界".repeat(9), 1, 10, "界 ".repeat(9)),
                ("界".repeat(8), 1, 10, "界 ".repeat(8) + " · 4m"),
                ("界".repeat(8) + "x", 1, 10, "界 ".repeat(8) + "x · 4m"),
                ("界".repeat(11), 1, 10, "界 ".repeat(11)),
                ("ｶﾞ".repeat(12), 1, 10, "ｶﾞ ".repeat(10) + "…"),
                ("ｶﾞ".repeat(4), 12_345_678, 19, "ｶﾞ ".repeat(4) + " · 4m"),
            ] {
                let mut shot = Shot::new();
                shot.fleet.rows[0].counts = Counts::Marks(if count == 1 {
                    vec![(Mark::NeedsYou, false, count)]
                } else {
                    vec![(Mark::Dead, false, count), (Mark::NeedsYou, false, count)]
                });
                shot.fleet.rows[0].line2 = Line2::Question {
                    text: text.clone(),
                    age_secs: Some(240),
                };
                let (buf, layout) = declutter_frame(&shot, 90, icons);
                assert_eq!(layout.list.width, 34, "the rendered cell budget");
                assert_eq!(buf[(34, 5)].symbol(), "│");
                assert_eq!(
                    line(&buf, 5)[start..32].concat().trim_end(),
                    expected.trim_end(),
                    "{icons}/{text}: cell/grapheme/whole-age budget"
                );
            }
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
            &["Esc q v", "close"],
            &["wheel", "3 rows"],
            &["any other", "nothing reaches the pane"],
        ] {
            assert!(
                section.iter().any(|r| words.iter().all(|w| r.contains(w))),
                "{words:?}: {section:?}"
            );
        }
    }

    /// With the peek panel open the keys row says `peek` and `Esc close`,
    /// keeps the version and records no gear, which would be a dead click.
    #[test]
    fn the_peek_keys_row_hides_the_gear_and_keeps_the_version() {
        let mut shot = Shot::new();
        shot.model.open_peek(1);
        let (row, layout) = shot.footer_at(100, browsing(""), false);
        let tail = crate::version_line();
        assert_eq!(
            row.strip_suffix(tail.as_str())
                .map(|head| head.trim_end().to_owned()),
            Some("  peek   Esc close".to_owned()),
            "{row:?}"
        );
        let last = Mouse {
            kind: MouseKind::Click,
            column: 99,
            row: 0,
        };
        assert!(layout.hit(last).is_none(), "no gear target");
        let (narrow, _) = shot.footer_at(22, browsing(""), false);
        assert_eq!(narrow, "  peek   Esc close", "the version drops, no gear");
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

    /// Stripping for the peek panel keeps no target, border or scroll bound
    /// the closed frame had recorded.
    #[test]
    fn stripping_for_the_peek_panel_keeps_no_closed_frame_target() {
        let mut shot = Shot::new();
        shot.agents = Some(seat_facts(&["lead", "colead"]));
        let (_, mut layout) = framed(&shot, 160, None);
        assert!(!layout.targets.is_empty(), "the closed frame has targets");
        layout.strip_for_peek();
        assert!(layout.targets.is_empty());
        assert!(layout.seats.is_empty() && layout.focused.is_none());
        assert!(layout.complete);
    }

    /// Last whole count spans stay when they fit; overflow leaves one ellipsis.
    #[test]
    fn declutter_count_tail_fills_the_last_cell_without_losing_a_whole_span() {
        for (last, expected) in [
            (12, "✖B123 ⚠B123 ◌B12 ●B12 ◔123 ✓123 ·12 "),
            (123, "✖B123 ⚠B123 ◌B12 ●B12 ◔123 ✓123 ·123"),
            (1234, "✖B123 ⚠B123 ◌B12 ●B12 ◔123 ✓123 …   "),
        ] {
            let mut shot = Shot::new();
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
            assert_eq!(layout.list.width, 44, "36 cells for counts below the name");
            assert_eq!(line(&buf, 5)[..45].concat(), format!("      {expected}  │"));
        }
    }

    /// Long names preserve the gap, stopped age and two cells before the rule.
    #[test]
    fn declutter_long_names_leave_the_stopped_age_and_right_edge_intact() {
        for stopped in [false, true] {
            let mut shot = Shot::new();
            shot.fleet.rows = vec![row(&"n".repeat(60), 1, false)];
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
            let expected = if stopped {
                format!("  ● 1 {} stopped 3m  │", "n".repeat(25))
            } else {
                format!("  ● 1 {}  │", "n".repeat(36))
            };
            assert_eq!(line(&buf, 4)[..45].concat(), expected, "stopped={stopped}");
            assert_eq!(line(&buf, 4)[42..44].concat(), "  ");
            assert_eq!(buf[(44, 5)].symbol(), "│");
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
            1,
        );
        assert_eq!(requested, 32_775, "the actual drag allows this sidebar");
        let _ = shot.model.drag_to(requested);
        let _ = shot.model.let_go();
        let mut buf = Buffer::empty(area);
        assert_eq!(buf.area.width, 32_820);
        let layout =
            super::draw_with_layout(&shot.screen(READ_ONLY), super::Wait::default(), &mut buf);
        assert_eq!(layout.list.width, 32_775);
        assert_eq!(line(&buf, 4)[32_763..32_776].concat(), "stopped 1m  │");
    }

    /// Clipping preserves original grapheme barriers, then appends one ellipsis.
    #[test]
    fn declutter_clipped_questions_keep_source_graphemes_separated() {
        for icons in [false, true] {
            for (text, expected) in [
                (
                    format!("🇦\u{200b}🇧{}", "x".repeat(22)),
                    "🇦🇧".to_owned() + &"x".repeat(19) + "…",
                ),
                (
                    format!("🇦\t🇧{}", "x".repeat(22)),
                    "🇦🇧".to_owned() + &"x".repeat(19) + "…",
                ),
                (
                    format!("ᄀ\u{200b}가{}", "x".repeat(22)),
                    "ᄀ 가 ".to_owned() + &"x".repeat(17) + "…",
                ),
                (
                    format!("ᄀ\t가{}", "x".repeat(22)),
                    "ᄀ 가 ".to_owned() + &"x".repeat(17) + "…",
                ),
                (
                    format!("ᄀ\u{200b}ᅡ{}", "x".repeat(22)),
                    "ᄀ ".to_owned() + &"x".repeat(19) + "…",
                ),
                (
                    format!("a\u{200b}b{}", "x".repeat(22)),
                    "ab".to_owned() + &"x".repeat(19) + "…",
                ),
            ] {
                let mut shot = Shot::new();
                shot.fleet.rows[0].counts = Counts::Marks(vec![(Mark::NeedsYou, false, 1)]);
                shot.fleet.rows[0].line2 = Line2::Question {
                    text: text.clone(),
                    age_secs: Some(240),
                };
                let (buf, layout) = declutter_frame(&shot, 90, icons);
                assert_eq!(layout.list.width, 34);
                assert_eq!(buf[(34, 5)].symbol(), "│");
                assert_eq!(line(&buf, 5)[10..32].concat(), expected, "{icons}/{text}");
                assert_eq!(buf[(31, 5)].symbol(), "…");
            }
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
            if line(buf, y)[6..6 + name.len()].concat() == name {
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
            assert_eq!(buf[(4, y)].symbol(), "2", "index col");
            assert_eq!(line(&buf, y)[6..10].concat(), "beta", "name col");
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
        let tail = format!("{} {gear}", crate::version_line());
        let head = "  Enter writes · ? keys";
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
}
