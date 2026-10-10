//! The cells: the sidebar, its tabs and the chat column, into one buffer.
//!
//! PURE. Colour is spent on meaning only and comes from the drawn look's
//! palette; `theme = off`, or no look, draws no colour at all and marks the
//! selection with bold and reverse instead. Every write is bounded by the
//! buffer, so a pane of any size draws without a panic.

use std::fmt::Write as _;
use std::ops::Range;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::{Position, Rect};
use ratatui_core::style::{Color, Modifier, Style};
use ratatui_core::text::{Line, Span};

use super::fleet::{Counts, Facts, Fleet, Line2, Row};
use super::model::{Drag, Edge, Model, SettingsTab, Split, Tab};
use super::overview::Overview;
use super::settings::{AboutFacts, ConfigView, InstructionsView, SettingsBodies};
use crate::brief::age;
use crate::console::input::View;
use crate::console::input::{Mouse, ROWS_MAX};
use crate::console::lane::Lane;
use crate::console::view;
use crate::console::wrap::wrap;
use crate::digest::{SessionEntry, Status};
use crate::event_text::clip_head;
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
const LIST_TOP: u16 = 5;
/// The sidebar's first content column.
const LEFT: u16 = 2;
/// Where a session row's name starts.
const NAME: u16 = 6;
/// The narrowest a drag leaves the sidebar: the tab row still keeps a gap
/// between `Agents NN` and `Tab`, and a name beside its counts.
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
    Tab(Tab),
    /// A drawn seat row: moves the highlight to that seat.
    Seat(String),
    Compose,
    /// The keys row's gear: toggles the settings overlay.
    Settings,
    /// A drawn Settings tab title.
    SettingsTab(SettingsTab),
    /// The overlay's drawn close label.
    SettingsClose,
    /// A drawn chat turn, by its fingerprint: marks it, or unmarks it.
    Turn(u64),
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
    /// The cell the terminal cursor shows: the draft's, while writing.
    pub cursor: Option<Position>,
    /// Body rows the open settings overlay shows at once.
    pub settings_page_rows: usize,
    /// How far back the settings body can scroll.
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
    /// The seats the tab body drew, in drawn order, and the one it highlighted.
    pub seats: Vec<String>,
    pub focused: Option<String>,
}

impl Layout {
    fn record(&mut self, buf: &Buffer, area: Rect, target: Hit) {
        self.targets.push((area.intersection(buf.area), target));
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

    /// Record a border at `at` over `cells`, grabbed within a cell of it.
    fn border(&mut self, buf: &Buffer, edge: Edge, at: u16, cells: Rect) {
        let grab = match edge {
            Edge::Sidebar => Rect::new(cells.x.saturating_sub(1), cells.y, 3, cells.height),
            Edge::List => Rect::new(cells.x, cells.y.saturating_sub(1), cells.width, 3),
        };
        // Above the keys row: nothing there grabs a border.
        let room = Rect::new(0, 0, buf.area.width, buf.area.height.saturating_sub(1));
        self.borders.push(Border {
            edge,
            at,
            cells: cells.intersection(room),
            grab: grab.intersection(room),
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
        self.targets.retain(|(_, hit)| matches!(hit, Hit::Settings));
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
        self.cursor = None;
        self.settings_page_rows = 0;
        self.settings_max_scroll = 0;
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
    fn ground(self, token: impl Fn(&Palette) -> &'static str) -> Style {
        self.0.map_or_else(Style::new, |palette| {
            Style::new().bg(colour(token(&palette)))
        })
    }

    /// A mark's accent.
    fn mark(self, mark: Mark) -> Style {
        self.fg(|palette| palette.accent(mark))
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
    // The frames' grounds: the chat's ink everywhere, the keys row and the
    // rule's own column included, and the panel under the sidebar alone.
    buf.set_style(area, ctx.paint.ground(|p| p.ink));
    let (height, width) = (area.height, area.width);
    let split = screen.model.split();
    let chat = if let Some(rule) = sidebar_width(area, split) {
        buf.set_style(
            Rect::new(0, 0, rule, height - 1),
            ctx.paint.ground(|p| p.base),
        );
        let cells = Rect::new(rule, 0, 1, height - 1);
        layout.border(buf, Edge::Sidebar, rule, cells);
        sidebar(&ctx, buf, rule, &mut layout);
        let border = ctx.paint.rule(dragged(screen.model, Edge::Sidebar));
        for y in 0..height - 1 {
            put(buf, rule, y, "│", 1, border);
        }
        rule + 1..width
    } else {
        let line = format!(
            "  sidebar needs {}x{} (now {width}x{height})",
            SIDEBAR_MIN.0, SIDEBAR_MIN.1
        );
        put(buf, 0, 0, &line, width, dim);
        0..width
    };
    layout.chat = Rect::new(chat.start, 0, width - chat.start, height - 1);
    chat_column(&ctx, buf, chat.start + 2..width - 2, &mut layout);
    keys_row(&ctx, buf, &mut layout);
    layout
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
    let room = usize::from(area.height.saturating_sub(9 + LANE_FLOOR));
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
    // two rows.
    let shared = area.height.saturating_sub(LIST_TOP + 5);
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
        .saturating_sub(LIST_TOP + 3 + BODY_FLOOR + 2)
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
    put(
        buf,
        LEFT,
        1,
        &format!("Sessions {count}"),
        room,
        paint.fg(|p| p.text),
    );
    let selected = model.position(fleet);
    let (rows, pinned) = list_rows(buf.area, model.split(), count);
    let list = List::of(count, selected, rows, pinned, model.list_top());
    layout.list = Rect::new(0, LIST_TOP, rule, list.end - LIST_TOP);
    layout.list_start = Some(list.visible.start);
    layout.list_max = count - list.visible.len();
    let needy = fleet.rows.iter().any(|row| row.needy);
    let line = super::fleet::attention(fleet, list.visible.clone(), room - 2);
    let tone = if needy {
        paint.fg(|p| p.needs_you)
    } else {
        paint.fg(|p| p.dim)
    };
    put(buf, LEFT, 3, &line, room - 2, tone);
    if needy {
        let key = paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
        put(buf, end - 1, 3, "!", 1, key);
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
            let _ = write!(text, ", {needy} need you");
        }
        put(buf, LEFT, y, &text, room, tone);
    }
    let Some(entry) = ctx.screen.selected.filter(|_| !ctx.wait.fleet) else {
        return;
    };
    let tabs = list.end + 1;
    tab_row(ctx, buf, tabs, end, layout);
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
    let body = match model.tab() {
        Tab::Overview => overview_rows(ctx, room, rule >= 44),
        Tab::Agents => agent_rows(ctx, entry),
    };
    tab_body(ctx, buf, body, tabs + 2, rule, room, layout);
}

/// The tab body from row `top` down to the floor, in the sidebar's `room`,
/// scrolled to the model's row; a body that does not fit names the rows it
/// hides on its last row.
fn tab_body(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    mut body: Vec<Cells>,
    top: u16,
    rule: u16,
    room: u16,
    layout: &mut Layout,
) {
    let (end, paint) = (rule - 2, ctx.paint);
    let floor = buf.area.height.saturating_sub(2);
    let room_rows = floor.saturating_sub(top);
    // A body that would lose rows gives up its gaps first.
    if body.len() > usize::from(room_rows) {
        body.retain(|row| row.left.width() > 0);
    }
    // A body that still does not fit gives its last row to the rows it hides.
    let cut = body.len() > usize::from(room_rows);
    let shown = usize::from(room_rows.saturating_sub(u16::from(cut)));
    layout.body = Rect::new(0, top, rule, room_rows);
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
    for (step, row) in drawn.enumerate() {
        let y = top + cells(step);
        if let Some(seat) = &row.seat {
            layout.record(
                buf,
                Rect::new(LEFT, y, end - LEFT, 1),
                Hit::Seat(seat.clone()),
            );
            if layout.focused.as_ref() == Some(seat) {
                let glyph = if ctx.icons { "▸" } else { ">" };
                put(
                    buf,
                    0,
                    y,
                    glyph,
                    1,
                    paint.fg(|p| p.title).add_modifier(Modifier::BOLD),
                );
            }
        }
        let used = put_line(buf, LEFT, y, &row.left, room);
        if let Some(right) = &row.right {
            let width = cells(right.width());
            if used + 1 + width <= end {
                put_line(buf, end - width, y, right, width);
            }
        }
    }
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
    if row.index <= 9 {
        put(buf, LEFT + 2, y, &row.index.to_string(), 1, dim);
    }
    let counts = counts(ctx, row);
    let counts_width = cells(counts.width());
    let counts_at = end.saturating_sub(counts_width);
    put_line(buf, counts_at, y, &counts, counts_width);
    let name = if selected {
        paint.selected()
    } else if stopped {
        dim
    } else {
        paint.fg(|p| p.text).add_modifier(Modifier::BOLD)
    };
    let room = counts_at.saturating_sub(NAME + 1);
    let after = put(buf, NAME, y, &row.name, room, name);
    let tag = "this window";
    if row.home && after + 2 + cells(tag.len()) < counts_at {
        put(buf, after + 2, y, tag, cells(tag.len()), dim);
    }
    let room = end - NAME;
    let (text, tone) = match &row.line2 {
        Line2::Question { text, age_secs } => (aged(text, *age_secs, room), paint.fg(|p| p.text)),
        Line2::SeatDown => ("a seat is down".to_owned(), paint.fg(|p| p.text)),
        Line2::Goal { text, age_secs } => (aged(text, *age_secs, room), dim),
        Line2::NoGoal => ("no goal set".to_owned(), dim),
        Line2::NotRunning { seats, .. } => (format!("{} not running", seats_word(*seats)), dim),
    };
    put(buf, NAME, y + 1, &text, room, tone);
}

/// The right side of a session's first line.
fn counts(ctx: &Ctx<'_, '_>, row: &Row) -> Line<'static> {
    let dim = ctx.paint.fg(|p| p.dim);
    match &row.counts {
        Counts::Marks(tallied) => {
            let mut spans = Vec::new();
            for (mark, blocked, count) in tallied {
                if !spans.is_empty() {
                    spans.push(Span::raw(" "));
                }
                let letter = if *blocked { "B" } else { "" };
                let text = format!("{}{letter}{count}", mark.glyph(ctx.icons));
                spans.push(Span::styled(text, ctx.paint.mark(*mark)));
            }
            Line::from(spans)
        }
        Counts::Unknown => Line::from(Span::styled("?", dim)),
        Counts::Stopped => {
            let since = match row.line2 {
                Line2::NotRunning { stopped_secs, .. } => stopped_secs,
                _ => None,
            };
            Line::from(Span::styled(format!("stopped {}", age(since)), dim))
        }
    }
}

fn seats_word(seats: usize) -> String {
    match seats {
        1 => "1 seat,".to_owned(),
        n => format!("{n} seats,"),
    }
}

/// The tab row: Overview and Agents, the showing one bold and underlined.
fn tab_row(ctx: &Ctx<'_, '_>, buf: &mut Buffer, y: u16, end: u16, layout: &mut Layout) {
    let paint = ctx.paint;
    let style = |showing: bool| {
        if showing {
            paint
                .fg(|p| p.text)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            paint.fg(|p| p.dim)
        }
    };
    let tab = ctx.screen.model.tab();
    let agents = match ctx.screen.agents {
        Some(Facts::Seats { agents, .. }) => format!("Agents {}", agents.len()),
        _ => "Agents".to_owned(),
    };
    let x = put(
        buf,
        LEFT,
        y,
        "Overview",
        end - LEFT,
        style(tab == Tab::Overview),
    );
    layout.record(
        buf,
        Rect::new(LEFT, y, x - LEFT, 1),
        Hit::Tab(Tab::Overview),
    );
    let after = put(
        buf,
        x + 3,
        y,
        &agents,
        end.saturating_sub(x + 3),
        style(tab == Tab::Agents),
    );
    layout.record(
        buf,
        Rect::new(x + 3, y, after.saturating_sub(x + 3), 1),
        Hit::Tab(Tab::Agents),
    );
    put(buf, end - 3, y, "Tab", 3, paint.fg(|p| p.dim));
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

    fn blank() -> Self {
        Self::of(Vec::new())
    }
}

/// The Overview tab: the launch facts, the goal, what waits on you, the latest
/// decision and the latest memo per topic. Narrow, every item takes one
/// clipped line.
fn overview_rows(ctx: &Ctx<'_, '_>, room: u16, wide: bool) -> Vec<Cells> {
    let paint = ctx.paint;
    let overview = ctx.screen.overview;
    let (text, dim) = (paint.fg(|p| p.text), paint.fg(|p| p.dim));
    let heading =
        |words: String| Cells::of(vec![Span::styled(words, text.add_modifier(Modifier::BOLD))]);
    let quiet = |words: &str| Cells::of(vec![Span::styled(words.to_owned(), dim)]);
    let mut rows: Vec<Cells> = overview
        .launch
        .iter()
        .map(|fact| quiet(&clip_head(fact, usize::from(room))))
        .collect();
    if !rows.is_empty() {
        rows.push(Cells::blank());
    }
    rows.push(heading("Goal".to_owned()));
    rows.push(match &overview.goal {
        Some((goal, since)) => Cells::of(vec![Span::styled(aged(goal, *since, room), text)]),
        None => quiet("No goal set."),
    });
    rows.push(Cells::blank());
    rows.push(heading(format!("Waiting on you {}", overview.open.len())));
    if overview.open.is_empty() {
        rows.push(quiet("Nothing waits on you."));
    }
    let mark = Span::styled("⚠ ", paint.fg(|p| p.needs_you));
    for open in &overview.open {
        let room = room.saturating_sub(2);
        if wide {
            rows.push(
                Cells::of(vec![
                    mark.clone(),
                    Span::styled(clip_head(&open.text, usize::from(room)), text),
                ])
                .of_seat(&open.seat),
            );
            let asked = format!("  asked by {} · {}", open.seat, age(open.age_secs));
            rows.push(Cells::of(vec![Span::styled(asked, dim)]).of_seat(&open.seat));
        } else {
            let line = aged(&open.text, open.age_secs, room);
            rows.push(Cells::of(vec![mark.clone(), Span::styled(line, text)]).of_seat(&open.seat));
        }
    }
    rows.push(Cells::blank());
    rows.push(heading("Decided · latest memo".to_owned()));
    let gap = overview
        .memo_gap
        .as_ref()
        .map(|why| format!("Memo unreadable: {why}"));
    match (&overview.decided, &gap) {
        (_, Some(gap)) => rows.push(quiet(gap)),
        (Some(line), None) => rows.push(Cells::of(vec![Span::styled(
            aged(&line.text, line.age_secs, room),
            text,
        )])),
        (None, None) => rows.push(quiet("No decision memo.")),
    }
    rows.push(Cells::blank());
    rows.push(heading("Topics".to_owned()));
    if let Some(gap) = &gap {
        rows.push(quiet(gap));
    } else if overview.topics.is_empty() {
        rows.push(quiet("No topics."));
    }
    for line in overview.topics.iter().filter(|_| gap.is_none()) {
        let topic = Span::styled(line.topic.clone(), text.add_modifier(Modifier::BOLD));
        if wide {
            rows.push(Cells::of(vec![
                topic,
                Span::styled(format!("  {}", age(line.age_secs)), dim),
            ]));
            let shown = clip_head(&line.text, usize::from(room.saturating_sub(2)));
            rows.push(Cells::of(vec![Span::styled(format!("  {shown}"), text)]));
        } else {
            let room = room.saturating_sub(cells(line.topic.chars().count()) + 1);
            let shown = aged(&line.text, line.age_secs, room);
            rows.push(Cells::of(vec![
                topic,
                Span::styled(format!(" {shown}"), text),
            ]));
        }
    }
    rows
}

/// The Agents tab: each seat with its state, then client, profile and model;
/// a session ae has no seat facts for says why.
fn agent_rows(ctx: &Ctx<'_, '_>, entry: &SessionEntry) -> Vec<Cells> {
    let paint = ctx.paint;
    let dim = paint.fg(|p| p.dim);
    let gap = |words: &str| vec![Cells::of(vec![Span::styled(words.to_owned(), dim)])];
    if entry.status == Status::Stopped {
        return gap("Not running: no seat facts.");
    }
    let agents = match ctx.screen.agents {
        Some(Facts::Seats { agents, .. }) => agents,
        Some(Facts::OtherServer) => return gap("No seat facts: on another tmux server."),
        Some(Facts::NotPublished) | None => return gap("No seat facts: watchdog publishes none."),
    };
    let mut rows = Vec::new();
    for agent in agents {
        let mark = agent.mark();
        rows.push(Cells {
            left: Line::from(vec![
                Span::styled(mark.glyph(ctx.icons), paint.mark(mark)),
                Span::raw(" "),
                Span::styled(agent.name.clone(), paint.fg(|p| p.text)),
            ]),
            right: Some(Line::from(Span::styled(
                agent.state.clone(),
                paint.mark(mark),
            ))),
            seat: Some(agent.name.clone()),
        });
        let facts: Vec<&str> = [&agent.client, &agent.profile, &agent.model]
            .into_iter()
            .map(String::as_str)
            .filter(|fact| !fact.is_empty())
            .collect();
        rows.push(
            Cells::of(vec![Span::styled(format!("  {}", facts.join(" · ")), dim)])
                .of_seat(&agent.name),
        );
    }
    rows
}

// ---------------------------------------------------------------------------
// the chat column
// ---------------------------------------------------------------------------

/// The chat column over `columns`: the header, the turns anchored to the
/// bottom, the composer.
fn chat_column(ctx: &Ctx<'_, '_>, buf: &mut Buffer, columns: Range<u16>, layout: &mut Layout) {
    let screen = ctx.screen;
    let paint = ctx.paint;
    let height = buf.area.height;
    let (left, end) = (columns.start, columns.end);
    let room = end - left;
    let (dim, border) = (paint.fg(|p| p.dim), paint.fg(|p| p.border));
    header(ctx, buf, left, end);
    put(buf, left, 2, &"─".repeat(usize::from(room)), room, border);
    // While writing, the composer has a row for each row of the draft's view,
    // under the address row; the note row, a blank and the keys row follow.
    let grown = match screen.composer {
        Composer::Home {
            view: Some(view), ..
        } => cells(view.rows.len().max(1)),
        _ => 1,
    };
    let composer = height.saturating_sub(4 + grown);
    let (top, bottom) = (3, composer.saturating_sub(2));
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
    } = super::lane::tail(
        screen.lane,
        usize::from(room),
        paint,
        &clock,
        main,
        need,
        None,
    );
    if ctx.wait.lane {
        rows.push(Line::from(Span::styled("loading", dim)));
    }
    let turns = place_turns(ctx, &turns, &mut rows, layout);
    let last = rows
        .len()
        .saturating_sub(scroll)
        .max(room_rows.min(rows.len()));
    let first = last.saturating_sub(room_rows);
    let y0 = bottom - cells(last - first);
    for (step, line) in rows[first..last].iter().enumerate() {
        put_line(buf, left, y0 + cells(step), line, room);
    }
    turn_targets(buf, layout, &turns, (first..last, left, y0, room));
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
    // A pending line — an armed key, a refused paste, an ask on its way —
    // takes the hint row, in full ink so it is not missed.
    match screen.model.note() {
        Some(note) => put(buf, left, height - 3, note, room, paint.fg(|p| p.text)),
        None => put(buf, left, height - 3, &hint, room, dim),
    };
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

/// The produced turns with their fingerprints and row spans, oldest first. The
/// marked turn is recoloured in place, and the frame keeps each turn's place
/// for the keys.
fn place_turns(
    ctx: &Ctx<'_, '_>,
    turns: &[super::lane::Turn],
    rows: &mut [Line<'static>],
    layout: &mut Layout,
) -> Vec<(u64, Range<usize>)> {
    let turns: Vec<(u64, Range<usize>)> = (turns.iter())
        .filter_map(|turn| {
            let key = *ctx.wait.keys.get(turn.at)?;
            Some((key, turn.first..turn.first + turn.rows))
        })
        .collect();
    for (key, span) in &turns {
        if ctx.screen.model.turn() == Some(*key) {
            super::lane::mark_rows(&mut rows[span.clone()], ctx.paint, ctx.icons);
        }
    }
    let end = rows.len();
    layout.turns = (turns.iter().rev())
        .map(|(key, span)| (*key, end - span.end, span.len()))
        .collect();
    turns
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
    let after = put(buf, left, 1, &entry.name, end - left, name);
    let middle = screen.pair.join(" + ");
    let after = put(
        buf,
        after + 3,
        1,
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
    for right in branch.iter().chain(std::iter::once(&activity)) {
        let width = cells(right.chars().count());
        if after + 2 + width <= end {
            put(buf, end - width, 1, right, width, paint.fg(|p| p.dim));
            break;
        }
    }
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
const BROWSE_KEYS: [(&str, &str); 14] = [
    ("1-9", "select that session"),
    ("j/k Up/Down", "previous / next session"),
    ("!", "next session needing you"),
    ("Tab", "Overview / Agents tab"),
    ("oo", "open seat; twice in 2 s"),
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

/// The keys row, the full width of the last row: the mode word and at most
/// one hint from the left, then `ae <version>` and the gear right-aligned.
/// Narrowing drops the version first, then the gear.
fn keys_row(ctx: &Ctx<'_, '_>, buf: &mut Buffer, layout: &mut Layout) {
    let paint = ctx.paint;
    let y = buf.area.height - 1;
    let composing = matches!(ctx.screen.composer, Composer::Home { view: Some(_), .. });
    let open = ctx.screen.model.settings_open();
    let width = buf.area.width;
    // The one hint is the key that works in that mode: writing and held take
    // every key, so they say the word alone.
    let (word, hint) = if open {
        ("settings", "Esc close")
    } else if composing {
        ("write", "")
    } else if matches!(ctx.screen.composer, Composer::Held { .. }) {
        ("held", "")
    } else {
        ("browse", "? keys")
    };
    let x = put(
        buf,
        LEFT,
        y,
        word,
        width - LEFT,
        paint.fg(|p| p.text).add_modifier(Modifier::BOLD),
    );
    let rest = if hint.is_empty() {
        String::new()
    } else {
        format!("   {hint}")
    };
    let hints_end = put(
        buf,
        x,
        y,
        &rest,
        width.saturating_sub(x),
        paint.fg(|p| p.dim),
    );
    let gear = if ctx.icons { "\u{2699}" } else { "*" };
    let tail = format!("{} {gear}", crate::version_line());
    let tail_cells = cells(Span::raw(tail.as_str()).width());
    let gear_cells = cells(Span::raw(gear).width());
    let room = |need: u16| hints_end.saturating_add(2).saturating_add(need) <= width;
    let style = paint.fg(|p| p.dim);
    if room(tail_cells) {
        put(buf, width - tail_cells, y, &tail, tail_cells, style);
        layout.record(
            buf,
            Rect::new(width - gear_cells, y, gear_cells, 1),
            Hit::Settings,
        );
    } else if room(gear_cells) {
        put(buf, width - gear_cells, y, gear, gear_cells, style);
        layout.record(
            buf,
            Rect::new(width - gear_cells, y, gear_cells, 1),
            Hit::Settings,
        );
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
            body: &settings_body(tab, bodies, paint, width),
            scroll,
        },
    );
}

/// What one full-pane panel draws: its title, tab row and scrolled body.
struct Panel<'a> {
    title: &'a str,
    tabs: &'a [&'a str],
    /// The click target of each tab title, in tab order.
    hits: &'a [Hit],
    /// The label drawn right-aligned on the title row, and its target.
    close: (&'a str, Hit),
    selected: usize,
    body: &'a [(String, Style)],
    scroll: usize,
}

/// One full-pane panel over rows `0..height-1`: title, tab row, rule and a
/// scrolled body, then its scroll bounds. Settings is the only caller; the
/// later `?` help reuses the seam. Below [`MIN`] the closed frame's fallback
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
    let title_end = put(buf, LEFT, 0, panel.title, width - LEFT, heading);
    let (label, close) = &panel.close;
    let label_x = width.saturating_sub(LEFT + cells(Span::raw(*label).width()));
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
    let rule: String = "─".repeat(usize::from(width));
    put(buf, 0, 2, &rule, width, paint.fg(|p| p.border));
    let visible = usize::from(height.saturating_sub(4));
    // The draw offset clamps to what is there before a line is chosen: past
    // the end paints an emptied body, and the model's clamp after this frame
    // converges the state to the same bound.
    let max = panel.body.len().saturating_sub(visible);
    let start = panel.scroll.min(max);
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

/// The overlay body's styled rows: every field through the terminal-text
/// neutraliser, because paths and config values are hostile text.
fn settings_body(
    tab: SettingsTab,
    bodies: &SettingsBodies,
    paint: Paint,
    width: u16,
) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let head = paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    let clean = crate::board::terminal_text;
    match tab {
        SettingsTab::Quota => match bodies.quota.as_ref() {
            None => vec![("loading".to_owned(), dim)],
            Some(rows) if rows.is_empty() => vec![("no quota scopes read".to_owned(), dim)],
            Some(rows) => rows
                .iter()
                .map(|row| {
                    let style = if row.header { head } else { text };
                    (clean(&row.label), style)
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
const WRITE_KEYS: [(&str, &str); 15] = [
    ("Enter", "send the line"),
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
    ("paste", "literal text, never Enter"),
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
const MOUSE_KEYS: [(&str, &str); 3] = [
    ("click", "select a session, tab, seat or turn"),
    ("wheel", "scroll 3 rows (the list: 1 session)"),
    ("drag", "a border resizes the sidebar / list"),
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
fn cells(count: usize) -> u16 {
    u16::try_from(count).unwrap_or(u16::MAX)
}

/// Whether (x, y) lies inside `buf`.
fn inside(buf: &Buffer, x: u16, y: u16) -> bool {
    let area = buf.area;
    x >= area.left() && x < area.right() && y >= area.top() && y < area.bottom()
}

/// Write `text` at (x, y), at most `max` cells and never past the buffer;
/// answers the column after it.
fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, max: u16, style: Style) -> u16 {
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

    use super::{
        Composer, LEFT, Paint, Screen, aged, dragged_to, draw, instructions_body, keys_body,
    };
    use crate::app::fleet::{Counts, Facts, Fleet, Line2, Row};
    use crate::app::model::{Drag, Edge, Model};
    use crate::app::overview::{Open, Overview};
    use crate::brief::TopicLine;
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

    /// The sweep's populated Overview body and lane: every section filled,
    /// a lane that scrolls, a coverage line.
    fn sweep_body() -> (Overview, Lane) {
        let topic = TopicLine {
            topic: "parking".to_owned(),
            age_secs: Some(60),
            author: "lead".to_owned(),
            text: "resume here: a topic text long enough to clip".to_owned(),
        };
        let overview = Overview {
            goal: Some((
                "a goal long enough to clip in the narrow sidebar".to_owned(),
                Some(9),
            )),
            open: vec![Open {
                seat: "colead".to_owned(),
                text: "an open question long enough to clip".to_owned(),
                age_secs: Some(5),
            }],
            decided: Some(topic.clone()),
            topics: vec![topic; 4],
            memo_gap: None,
            launch: vec![
                "mode: worktree".to_owned(),
                "dir: a launch directory long enough to clip".to_owned(),
                "source: /src".to_owned(),
            ],
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
        // Each geometry the frame draws: the Overview and Agents bodies, and
        // the browse and compose composers. Foreign, no-home and mono draw
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
                let _ = model.key(crate::app::model::Key::PageUp, &fleet, false, true);
            }
            if agents {
                let _ = model.key(crate::app::model::Key::Tab, &fleet, false, true);
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
            let _ = self.model.key(key, &self.fleet, false, true);
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

        /// Only the keys row, on a buffer `width` wide and one row high: what
        /// it drew, and the layout it recorded.
        fn keys_at(&self, width: u16, composer: Composer<'_>) -> (String, super::Layout) {
            let screen = self.screen(composer);
            let ctx = super::Ctx {
                screen: &screen,
                paint: Paint::of(screen.look.as_ref()),
                icons: true,
                wait: super::Wait::default(),
            };
            let mut buf = Buffer::empty(Rect::new(0, 0, width, 1));
            let mut layout = super::Layout::default();
            super::keys_row(&ctx, &mut buf, &mut layout);
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

    /// The last cell of the chat column's rule (row 2): where the drawn
    /// column ends. The frames disagree by one at the right edge, so the
    /// header is judged against the rule drawn with it.
    fn rule_end(buf: &Buffer) -> Option<u16> {
        (0..buf.area.width)
            .rev()
            .find(|x| buf[(*x, 2)].symbol() == "─")
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

    /// Frames calm r18 (160x45) and needs-you r17 (100x30): the tab row reads
    /// `Overview`@2, `Agents N`@13 and `Tab` ending two cells before the
    /// sidebar's `│`; the showing tab is bold and underlined, the other one
    /// as dim as `Tab`. 90x20 is the narrowest sidebar (rule 34).
    #[test]
    fn the_tab_row_sits_where_the_frames_put_it() {
        use ratatui_core::style::Modifier;
        for (width, height, border) in [(160, 43, 44_u16), (100, 28, 34), (90, 20, 34)] {
            for agents in [false, true] {
                let mut shot = Shot::new();
                shot.agents = Some(match sweep_seats() {
                    Facts::Seats { id, agents } => Facts::Seats {
                        id,
                        agents: agents.into_iter().take(6).collect(),
                    },
                    other => other,
                });
                if agents {
                    shot.key(crate::app::model::Key::Tab);
                }
                let buf = shot.draw(width, height, READ_ONLY);
                let size = format!("{width}x{height} agents {agents}");
                let (y, x) = spot(&buf, "Overview").expect("a tab row");
                assert_eq!(x, 2, "{size}");
                assert_eq!(spot(&buf, "Agents 6"), Some((y, 13)), "{size}");
                let tab = border - 5;
                assert_eq!(spot(&buf, "Tab  │"), Some((y, usize::from(tab))), "{size}");
                let (overview, agents_cell, tab) = (&buf[(2, y)], &buf[(13, y)], &buf[(tab, y)]);
                let shown = Modifier::BOLD | Modifier::UNDERLINED;
                let (on, off) = if agents {
                    (agents_cell, overview)
                } else {
                    (overview, agents_cell)
                };
                assert!(on.modifier.contains(shown), "{size}: the showing tab");
                assert!(
                    !off.modifier.contains(Modifier::BOLD),
                    "{size}: the other tab"
                );
                assert_eq!(off.fg, tab.fg, "{size}: the other tab is as dim as Tab");
                assert_ne!(on.fg, tab.fg, "{size}: the showing tab is not dim");
            }
        }
    }

    /// At 100x30 the Overview is narrow: a topic row keeps its age inside the
    /// sidebar, ending one cell before the two blank cells the `│` stands
    /// after (frame needs-you r17 geometry: content stops at end-1 = 31).
    #[test]
    fn a_narrow_topic_row_keeps_its_age_inside_the_column() {
        let mut shot = Shot::new();
        shot.overview.topics = vec![TopicLine {
            topic: "parking".to_owned(),
            age_secs: Some(360),
            author: "lead".to_owned(),
            text: "resume at scope checks after the fixtures land on main".to_owned(),
        }];
        let buf = shot.draw(100, 30, READ_ONLY);
        let (y, x) = spot(&buf, "parking").expect("the topic row");
        assert_eq!(x, 2);
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
            last_ink(&buf, 1, 160).is_some_and(|x| x <= end),
            "a long name"
        );
        let mut shot = Shot::new();
        shot.pair = vec!["h".repeat(60), "g".repeat(60)];
        let buf = shot.draw(160, 45, READ_ONLY);
        let (y, _) = spot(&buf, "hhh").expect("the long pair middle");
        assert_eq!(y, 1);
        let end = rule_end(&buf).expect("the rule");
        assert!(
            last_ink(&buf, 1, 160).is_some_and(|x| x <= end),
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
        let header = line(&buf, 1).concat();
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
            let header = line(&buf, 1).concat();
            let shown = header.trim_end();
            assert!(
                shown.contains(&shot.pair.join(" + ")),
                "the middle is whole: {header:?}"
            );
            assert_eq!(last_ink(&buf, 1, 160), rule_end(&buf), "{header:?}");
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
    /// status, so the app pane is 160x43 (100x28). Above the keys row the
    /// sidebar is panel #313335 and everything from its rule on is base
    /// #2B2B2B, row 0 included; the keys row is base across the width.
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
            for (x, y) in [
                (rule, 10),
                (inside, 0),
                (inside, 10),
                (0, height - 1),
                (inside, height - 1),
            ] {
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
        shot.key(crate::app::model::Key::Tab);
        let buf = shot.draw(100, 24, READ_ONLY);
        assert_eq!(
            spot(&buf, "↓ 7 more rows"),
            Some((21, 2)),
            "the body's last row"
        );
        let dim = super::colour(crate::theme::Palette::DARCULA.dim);
        assert_eq!(buf[(2, 21)].fg, dim);
        let tall = shot.draw(100, 40, READ_ONLY);
        assert_eq!(spot(&tall, "more rows"), None, "a body that fits");
        shot.fleet = frame_fleet();
        // A one-row body is only the line naming what it hides.
        let one_row = shot.draw(100, 20, READ_ONLY);
        assert_eq!(spot(&one_row, "more rows").map(|at| at.0), Some(17));
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

    /// Frame calm: the rule stands at 44 on every row above the keys row
    /// (r0-r41) and not on the keys row (r42). Composing, the mode word ends
    /// before column 44, so the keys row shows it.
    #[test]
    fn the_rule_stops_above_the_keys_row() {
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
        for y in 0..42 {
            assert_eq!(buf[(44, y)].symbol(), "│", "row {y}");
        }
        assert_ne!(buf[(44, 42)].symbol(), "│", "the keys row");
        let keys = line(&buf, 42).concat();
        assert!(keys.trim_start().starts_with("write"), "the mode word");
        assert!(!keys.contains("Esc browse"), "no key names on the row");
    }

    /// The keys row is the mode word and at most one hint, where its key
    /// works: browse points at the Keys tab, Settings at its close, writing
    /// and held say the word alone; no other key is named on it.
    #[test]
    fn the_keys_row_is_the_mode_word_and_at_most_one_hint() {
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
            (browsing("kept"), "browse   ? keys"),
            (READ_ONLY, "browse   ? keys"),
            (writing(&draft, "a draft"), "write"),
            (refused, "held"),
        ];
        for (composer, words) in cases {
            let (row, _) = shot.keys_at(100, composer);
            let head = row
                .strip_suffix(tail.as_str())
                .expect("version and gear end it");
            assert_eq!(head.trim_end(), format!("  {words}"), "{words}");
            assert!(named.iter().all(|key| !row.contains(key)), "{row}");
        }
        shot.model.open_settings(1);
        let (row, _) = shot.keys_at(100, browsing(""));
        let head = row
            .strip_suffix(tail.as_str())
            .expect("version and gear end it");
        assert_eq!(head.trim_end(), "  settings   Esc close");
    }

    /// Narrower than the floor (the app never draws it, the row still has
    /// its order): the version goes first, then the gear, and the gear is
    /// the settings click target for as long as it is drawn.
    #[test]
    fn narrowing_the_keys_row_drops_the_version_then_the_gear() {
        let shot = Shot::new();
        let gear = "\u{2699}";
        let tail = format!("{} {gear}", crate::version_line());
        let head = "  browse   ? keys";
        let full = head.chars().count() + 2 + tail.chars().count();
        let drop = |width: usize, drawn: &str| {
            let at = u16::try_from(width).unwrap_or(0);
            let (row, layout) = shot.keys_at(at, browsing(""));
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
    /// row, and writing draws no send hint.
    #[test]
    fn a_draft_sits_under_its_address_from_the_columns_left_edge() {
        let shot = Shot::new();
        let draft = drafted(&["first words", "second"]);
        for (width, height) in [(100, 30), (60, 20)] {
            let size = format!("{width}x{height}");
            let (buf, layout) = shot.laid(width, height, writing(&draft, "first words second"));
            let (ay, ax) = spot(&buf, "to api › lead").expect("the address");
            assert_eq!(
                ay,
                height - 6,
                "{size}: over two draft rows, the note, a blank"
            );
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
            assert!(!all.contains("Enter sends"), "{size}: no send hint");
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
            assert_eq!(ay, 25, "{draft:?}");
            assert_eq!(row_from(&buf, ay, ax), "to api › lead", "{draft:?}");
            assert_eq!(
                row_from(&buf, ay + 1, ax),
                draft,
                "{draft:?}: the input row"
            );
            assert_eq!(row_from(&buf, ay + 2, ax), hint, "{draft:?}: the hint");
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
            assert_eq!(ay, 25, "{words}");
            assert_eq!(row_from(&buf, ay, ax), words, "{words}");
            assert_eq!(row_from(&buf, ay + 1, ax), "", "{words}: the input row");
            assert_eq!(row_from(&buf, ay + 2, ax), "", "{words}: the note row");
        }
    }

    /// A pending line takes the row under the composer, in full ink; writing
    /// puts no hint on it otherwise.
    #[test]
    fn a_note_takes_the_row_under_the_composer_and_writing_has_no_hint() {
        let mut shot = Shot::new();
        let draft = drafted(&["hello"]);
        let (buf, _) = shot.laid(100, 30, writing(&draft, "hello"));
        let (ay, ax) = spot(&buf, "to api › lead").expect("the address");
        assert_eq!(ay, 25);
        assert_eq!(row_from(&buf, 27, ax), "", "no hint while writing");
        shot.model.set_note(Some("q again to quit".to_owned()));
        let (buf, _) = shot.laid(100, 30, writing(&draft, "hello"));
        assert_eq!(spot(&buf, "q again to quit"), Some((27, ax)));
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

    /// At the 40x8 floor the lane has no row: the header's rule is the
    /// composer's top rule, every chat row is whole, and a scrolled lane
    /// draws no newer-turns hint over the header.
    #[test]
    fn the_floor_size_keeps_every_chat_row_whole() {
        let mut shot = Shot::new();
        shot.lane = turns(150);
        shot.model.wheel(true, 10, 100);
        let (buf, _) = shot.laid(40, 8, browsing(""));
        assert_eq!(spot(&buf, "to api › lead"), Some((3, 2)));
        assert_eq!(spot(&buf, "Enter writes"), Some((5, 2)));
        assert_eq!(buf[(2, 2)].symbol(), "─", "the rule");
        assert!(line(&buf, 1).concat().contains("api"), "the header");
        assert_eq!(spot(&buf, "newer turns"), None, "no hint over the header");
        assert!(
            row_from(&buf, 7, 0).trim_start().starts_with("browse"),
            "the keys row"
        );
    }

    /// A tall draft leaves the lane its floor: at 60x20 the composer grows
    /// to eight draft rows and the lane keeps r3-r5, and no height from 13
    /// up lets the biggest draft take the lane under it.
    #[test]
    fn a_tall_draft_leaves_the_lane_its_floor() {
        let mut shot = Shot::new();
        shot.lane = turns(40);
        let area = Rect::new(0, 0, 60, 20);
        assert_eq!(
            super::composer_pane(area),
            9,
            "eight draft rows and one kept back"
        );
        let draft = drafted(&["row"; 8]);
        let (buf, layout) = shot.laid(60, 20, writing(&draft, "row"));
        let (ay, ax) = spot(&buf, "to api › lead").expect("the address");
        assert_eq!(ay, 8);
        assert_eq!(buf[(column(ax), 7)].symbol(), "─", "the rule");
        assert_eq!(layout.page_rows, 3);
        assert_eq!(
            spot(&buf, "turn 39"),
            Some((5, ax + 2)),
            "the lane's last row"
        );
        assert_eq!(row_from(&buf, 6, ax), "", "the blank row over the rule");
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
            assert_eq!(ay, 15, "{home}");
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
            let header = line(&shot.draw(width, 18, READ_ONLY), 1).concat();
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
            let tabs = spot(&buf, "Overview").expect("the tab row").0;
            assert_eq!(tabs, y + 2, "{size}: one blank row, then the tabs");
        }
        let mut empty = Shot::new();
        empty.fleet.rows.clear();
        empty.model = Model::new(&empty.fleet);
        let buf = empty.draw(160, 43, READ_ONLY);
        let none = spot(&buf, "No sessions.").expect("the empty list").0;
        assert_eq!(
            spot(&buf, "Overview").map(|at| at.0),
            Some(none + 2),
            "one blank row, then the tabs"
        );
        // (e)/(f) frame many 100x30 r14: the more row counts the needy rows it hides.
        for (select, more) in [
            (None, "↓ 4 more, 2 need you"),
            (Some(9), "↑ 4 more, 1 need you"),
            (Some(6), "↑ 1 · ↓ 3 more, 1 need you"),
        ] {
            let mut shot = Shot::new();
            shot.fleet = frame_fleet();
            shot.model = Model::new(&shot.fleet);
            if let Some(digit) = select {
                let _ = shot.model.key(
                    crate::app::model::Key::Digit(digit),
                    &shot.fleet,
                    false,
                    true,
                );
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

    /// R1: several needy rows use a count and plain location suffixes;
    /// the `!` key remains at sidebar end-1 (col 41 at 160).
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
        let attention = line(&buf, 3).concat();
        assert!(attention.starts_with("  3 need you  "), "{attention:?}");
        let mut shot = Shot::new();
        shot.fleet.rows = vec![
            row("a", 1, false),
            row("needy-0010", 2, true),
            row("needy-00011", 3, true),
        ];
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        assert_eq!(buf[(41, 3)].symbol(), "!", "the key at end-1");
        assert_eq!(
            buf[(40, 3)].symbol(),
            " ",
            "a blank before it: {:?}",
            line(&buf, 3).concat()
        );
        // (c) several long names still leave a blank before the key.
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
            buf[(40, 3)].symbol(),
            " ",
            "clipped before the key: {:?}",
            line(&buf, 3).concat()
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
        let attention = line(&shot.draw(160, 43, READ_ONLY), 3).concat();
        assert!(
            attention.starts_with("  3 need you · 2 below  "),
            "{attention:?}"
        );
    }

    /// A body that fits exactly keeps its gaps; one row short, it gives the
    /// gaps up first and loses nothing else. One session puts the tabs at
    /// row 8 and the body at row 10; the empty Overview is eleven rows, so
    /// it reaches row 20 exactly at 100x23 (the floor at row 21).
    #[test]
    fn an_overview_that_fits_exactly_keeps_its_gaps() {
        let mut shot = Shot::new();
        shot.fleet.rows = vec![row("api", 1, false)];
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(100, 23, READ_ONLY);
        let topics = spot(&buf, "Topics").expect("the topics heading").0;
        assert_eq!(
            spot(&buf, "No topics.").map(|at| at.0),
            Some(20),
            "the last row above the floor"
        );
        assert_eq!(topics, 19);
        assert!(
            line(&buf, topics - 1)[..34].concat().trim().is_empty(),
            "the gap above Topics stays"
        );
        let buf = shot.draw(100, 22, READ_ONLY);
        let topics = spot(&buf, "Topics").expect("the topics heading").0;
        assert!(
            !line(&buf, topics - 1)[..34].concat().trim().is_empty(),
            "the gaps go first"
        );
        assert_eq!(
            spot(&buf, "No topics.").map(|at| at.0),
            Some(topics + 1),
            "nothing else is lost"
        );
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
            shot.key(crate::app::model::Key::Tab);
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
    /// other (r9-r10 infra, r15-r16 billing, r19-r20 ops).
    #[test]
    fn a_needy_bar_covers_its_own_two_rows() {
        let mut shot = Shot::new();
        shot.fleet = frame_fleet();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        let barred: Vec<u16> = (0..43).filter(|y| buf[(0, *y)].symbol() == "│").collect();
        assert_eq!(barred, [9, 10, 15, 16, 19, 20]);
    }

    /// Frame many r5-r21: a session's counts close its first line with a
    /// blank before them; a name too long for the room is clipped short of
    /// that blank, never into the counts.
    #[test]
    fn a_long_session_name_stops_a_blank_before_its_counts() {
        let mut shot = Shot::new();
        shot.fleet.rows = vec![row(&"n".repeat(60), 1, false)];
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        let cells = line(&buf, 5);
        let name = cells
            .iter()
            .rposition(|cell| cell == "n")
            .expect("the name");
        assert_eq!(
            cells[name + 1],
            " ",
            "a blank after the name: {:?}",
            cells.concat()
        );
        assert_eq!(
            cells[name + 2],
            "●",
            "then the counts: {:?}",
            cells.concat()
        );
    }

    /// Frame many r5: `this window` follows the home session's name after
    /// two blanks, on that row alone; with the counts at col 35 (`●3 ✓B12`
    /// at 160) a 15-cell name leaves the tag a blank before them and a
    /// 16-cell one drops it rather than touch them.
    #[test]
    fn this_window_marks_the_home_row_alone_and_never_touches_its_counts() {
        let mut shot = Shot::new();
        shot.fleet = frame_fleet();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        let sidebar = |buf: &Buffer, y: u16| line(buf, y)[..44].concat();
        let tagged: Vec<u16> = (0..43)
            .filter(|y| sidebar(&buf, *y).contains("this window"))
            .collect();
        assert_eq!(tagged, [5], "the home row alone");
        assert!(
            sidebar(&buf, 5).starts_with("  ● 1 api  this window"),
            "{:?}",
            sidebar(&buf, 5)
        );
        for (cells, shown) in [(15, true), (16, false)] {
            let mut shot = Shot::new();
            shot.fleet.rows = vec![row(&"n".repeat(cells), 1, false)];
            shot.model = Model::new(&shot.fleet);
            let first = sidebar(&shot.draw(160, 43, READ_ONLY), 5);
            assert_eq!(
                first.contains("this window"),
                shown,
                "a {cells}-cell name: {first:?}"
            );
        }
    }

    /// Frame many r6-r22: a session's second line ends its age at col 41 at
    /// 160 (r10 `… 4m` @40-41), two blanks before the rule.
    #[test]
    fn a_sessions_second_line_ends_where_the_frames_does() {
        let mut shot = Shot::new();
        shot.fleet = frame_fleet();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        for y in [6, 10] {
            let cells = line(&buf, y);
            assert_eq!(
                cells[40..44].concat(),
                "4m  ",
                "row {y}: {:?}",
                cells[..45].concat()
            );
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

    /// Ruling 5b: a 100x20 pane of 13 sessions draws its list rule on row 16
    /// (two-row steps, four shown, the more row, tabs on 15). A press there
    /// moved nowhere keeps it; dragged up and back down past it, it stops on
    /// 16, never 17, though that leaves the tab body one row.
    #[test]
    fn a_short_pane_keeps_its_undragged_list_rule_reachable() {
        let mut shot = sessions(13);
        let first = shot.draw(100, 20, READ_ONLY);
        assert_eq!(horizontal(&first), Some(16), "the undragged rule");
        let still = drag(&mut shot, &first, Edge::List, 16);
        assert_eq!(horizontal(&still), Some(16), "a press moved nowhere");
        let up = drag(&mut shot, &still, Edge::List, 12);
        assert_eq!(horizontal(&up), Some(12));
        let down = drag(&mut shot, &up, Edge::List, 19);
        assert_eq!(horizontal(&down), Some(16), "never past the undragged rule");
    }

    /// Ruling 5a: the list keeps one session's rows — two alone, two and the
    /// more row among several — and above the floor three body rows.
    #[test]
    fn a_dragged_list_keeps_one_session_and_three_body_rows() {
        for (count, least) in [(1, 9), (2, 10), (13, 10)] {
            let mut shot = sessions(count);
            let first = shot.draw(160, 45, READ_ONLY);
            let top = drag(&mut shot, &first, Edge::List, 0);
            assert_eq!(horizontal(&top), Some(least), "{count} sessions");
            let bottom = drag(&mut shot, &top, Edge::List, 44);
            // Floor 43: body rows 40, 41 and 42.
            assert_eq!(horizontal(&bottom), Some(39), "{count} sessions");
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
        assert_eq!(at(2, rule - 1), None, "a tab label is a target");
        assert_eq!(at(44, rule), Some((Edge::Sidebar, 44)), "the corner");
        assert_eq!(at(43, rule), Some((Edge::List, rule)), "its own cell");
        assert_eq!(at(44, 44), None, "the keys row");
    }

    /// Two seats on the Agents tab, by name.
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
        let _ = shot
            .model
            .key(crate::app::model::Key::Tab, &shot.fleet, false, true);
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(layout.seats, ["lead", "colead"]);
        assert_eq!(layout.focused.as_deref(), Some("lead"));
        let first = *marked(&buf, "▸").first().expect("a marked row");
        assert_eq!(marked(&buf, "▸"), [first, first + 1]);
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
        assert_eq!(marked(&buf, "▸"), [first + 2, first + 3]);
        assert_eq!(layout.focused.as_deref(), Some("colead"));
        shot.model.set_focus(Some("ghost".to_owned()));
        let (_, layout) = framed(&shot, 160, None);
        assert_eq!(layout.focused.as_deref(), Some("lead"), "an undrawn name");
        let ascii = Look::read("off", "darcula", "on", "on");
        let (buf, _) = framed(&shot, 160, Some(&ascii));
        assert_eq!(marked(&buf, ">"), [first, first + 1]);
    }

    /// A tab body that draws no seat focuses nothing; a waiting entry is the
    /// seat's two rows wide and its one row narrow.
    #[test]
    fn overview_entries_are_seat_rows_wide_and_narrow() {
        let mut shot = Shot::new();
        let (_, layout) = framed(&shot, 160, None);
        assert_eq!((layout.seats.len(), layout.focused), (0, None));
        shot.overview.open = vec![Open {
            seat: "colead".to_owned(),
            text: "a question".to_owned(),
            age_secs: Some(5),
        }];
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(layout.seats, ["colead"]);
        assert_eq!(marked(&buf, "▸").len(), 2, "both rows of a wide entry");
        let (buf, layout) = framed(&shot, 100, None);
        assert_eq!(layout.seats, ["colead"]);
        assert_eq!(marked(&buf, "▸").len(), 1, "the one row of a narrow entry");
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
}
