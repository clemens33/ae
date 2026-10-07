//! The cells: the sidebar, its tabs and the chat column, into one buffer.
//!
//! PURE. Colour is spent on meaning only and comes from the drawn look's
//! palette; `theme = off`, or no look, draws no colour at all and marks the
//! selection with bold and reverse instead. Every write is bounded by the
//! buffer, so a pane of any size draws without a panic.

use std::fmt::Write as _;
use std::ops::Range;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier, Style};
use ratatui_core::text::{Line, Span};

use super::fleet::{Counts, Facts, Fleet, Line2, Row};
use super::model::{Drag, Edge, Model, SettingsTab, Split, Tab};
use super::overview::Overview;
use super::settings::{AboutFacts, ConfigView, SettingsBodies};
use crate::brief::age;
use crate::console::input::Mouse;
use crate::console::input::View;
use crate::console::lane::Lane;
use crate::console::view;
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
    /// The home session, owned: typing reaches its lead pair.
    Home {
        home: &'a str,
        speaker: &'a str,
        /// `Some` while composing.
        view: Option<&'a View>,
        draft: &'a str,
    },
    /// The home session, not owned.
    ReadOnly { why: &'a str },
    /// The home session, its writer lease refused: HELD until Esc, ^C, a
    /// click or an Enter that tries again.
    Held { why: &'a str },
    /// A foreign session is selected.
    Foreign { home: &'a str, speaker: &'a str },
    /// No home session.
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

/// A click target recorded where its cells are drawn.
#[derive(Debug, Clone)]
pub(crate) enum Hit {
    Session(String),
    Tab(Tab),
    Compose,
    /// The keys row's gear: toggles the settings overlay.
    Settings,
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
        self.page_rows = 0;
        self.max_scroll = 0;
        self.complete = true;
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
    fn selected(self) -> Style {
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
    wait: Wait,
}

/// What `ae app` has not read yet, drawn as `loading`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Wait {
    /// The fleet: the sidebar list says `loading` and no tab shows.
    pub(crate) fleet: bool,
    /// The selection's lane: one dim chat row says `loading`.
    pub(crate) lane: bool,
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
pub(crate) fn draw_with_layout(screen: &Screen<'_>, wait: Wait, buf: &mut Buffer) -> Layout {
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

/// The cells a draft has on the composer row of `area` split as `split`,
/// after its `to <home> › <speaker>` address: what the caller wraps it to.
pub(crate) fn draft_width(area: Rect, split: Split, home: &str, speaker: &str) -> usize {
    let left = sidebar_width(area, split).map_or(0, |rule| rule + 1) + 2;
    let room = usize::from(area.width.saturating_sub(2).saturating_sub(left));
    let address = Span::raw(format!("to {home} › {speaker}   ")).width();
    room.saturating_sub(address).max(1)
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
/// all when its sessions need fewer: 18 from 40 high, else 11, sized to its
/// sessions, until a drag asks for a height of its own.
fn list_rows(area: Rect, split: Split, count: usize) -> (u16, bool) {
    let default = if area.height >= 40 { 18 } else { 11 };
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
    for (step, row) in body.iter().skip(first).take(shown).enumerate() {
        let y = top + cells(step);
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
}

impl Cells {
    fn of(spans: Vec<Span<'static>>) -> Self {
        Self {
            left: Line::from(spans),
            right: None,
        }
    }

    fn blank() -> Self {
        Self::of(Vec::new())
    }
}

/// The Overview tab: the goal, what waits on you, the latest decision and the
/// latest memo per topic. Narrow, every item takes one clipped line.
fn overview_rows(ctx: &Ctx<'_, '_>, room: u16, wide: bool) -> Vec<Cells> {
    let paint = ctx.paint;
    let overview = ctx.screen.overview;
    let (text, dim) = (paint.fg(|p| p.text), paint.fg(|p| p.dim));
    let heading =
        |words: String| Cells::of(vec![Span::styled(words, text.add_modifier(Modifier::BOLD))]);
    let quiet = |words: &str| Cells::of(vec![Span::styled(words.to_owned(), dim)]);
    let mut rows = vec![heading("Goal".to_owned())];
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
            rows.push(Cells::of(vec![
                mark.clone(),
                Span::styled(clip_head(&open.text, usize::from(room)), text),
            ]));
            let asked = format!("  asked by {} · {}", open.seat, age(open.age_secs));
            rows.push(Cells::of(vec![Span::styled(asked, dim)]));
        } else {
            let line = aged(&open.text, open.age_secs, room);
            rows.push(Cells::of(vec![mark.clone(), Span::styled(line, text)]));
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
        });
        let facts: Vec<&str> = [&agent.client, &agent.profile, &agent.model]
            .into_iter()
            .map(String::as_str)
            .filter(|fact| !fact.is_empty())
            .collect();
        rows.push(Cells::of(vec![Span::styled(
            format!("  {}", facts.join(" · ")),
            dim,
        )]));
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
    let (top, bottom) = (3, height.saturating_sub(6));
    let main = screen
        .selected
        .and_then(|entry| entry.main_agent.as_deref())
        .or_else(|| match screen.composer {
            Composer::Home { .. } | Composer::ReadOnly { .. } | Composer::Held { .. } => {
                screen.pair.first().map(String::as_str)
            }
            Composer::Foreign { .. } | Composer::NoHome => None,
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
    let super::lane::Tail { mut rows, complete } = super::lane::tail(
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
    let last = rows
        .len()
        .saturating_sub(scroll)
        .max(room_rows.min(rows.len()));
    let first = last.saturating_sub(room_rows);
    let y0 = bottom - cells(last - first);
    for (step, line) in rows[first..last].iter().enumerate() {
        put_line(buf, left, y0 + cells(step), line, room);
    }
    if scroll > 0 && last < rows.len() {
        put(buf, left, bottom, "↓ newer turns below · PgDn", room, dim);
    }
    put(
        buf,
        left,
        height - 5,
        &"─".repeat(usize::from(room)),
        room,
        border,
    );
    let (address, hint) = composer_lines(ctx);
    put_line(buf, left, height - 4, &address, room);
    put(buf, left, height - 3, &hint, room, dim);
    layout.record(buf, Rect::new(left, height - 4, room, 2), Hit::Compose);
    layout.page_rows = room_rows;
    layout.max_scroll = rows.len().saturating_sub(room_rows);
    layout.complete = complete;
}

/// The chat header: the session, its pair or where it is viewed from, and
/// right-aligned its branch (at 140 columns or more) and its activity.
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
    let middle = match screen.composer {
        Composer::Foreign { home, .. } => {
            format!("viewed from the {home} window · Esc returns to {home}")
        }
        _ => screen.pair.join(" + "),
    };
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

/// The composer's address row and the hint under it.
fn composer_lines(ctx: &Ctx<'_, '_>) -> (Line<'static>, String) {
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
            let address = Span::styled(format!("to {home} › {speaker}   "), paint.fg(|p| p.title));
            if let Some(view) = view {
                let row = view.rows.get(view.cursor_row).cloned().unwrap_or_default();
                let line = Line::from(vec![address, Span::styled(row, text)]);
                return (line, "Enter sends · Esc keeps the draft".to_owned());
            }
            let line = Line::from(vec![address, Span::styled(draft.to_owned(), text)]);
            let hint = if draft.is_empty() {
                "Enter writes"
            } else {
                "draft kept · Enter writes"
            };
            (line, hint.to_owned())
        }
        Composer::ReadOnly { why } => (
            quiet(format!("read-only · {why} - prefix h opens it")),
            String::new(),
        ),
        Composer::Held { why } => (
            quiet(format!("not writing: {why} · Esc browses")),
            String::new(),
        ),
        Composer::Foreign { home, speaker } => (
            quiet(format!("read-only · typing writes to {home} › {speaker}")),
            String::new(),
        ),
        Composer::NoHome => (
            quiet("read-only · no home session: ae app <session> picks one".to_owned()),
            String::new(),
        ),
    }
}

/// The keys row, the full width of the last row: the mode's hints from
/// the left, then `ae <version>` and the gear right-aligned. Hints never
/// yield a cell: narrowing drops the version first, then the gear.
fn keys_row(ctx: &Ctx<'_, '_>, buf: &mut Buffer, layout: &mut Layout) {
    let paint = ctx.paint;
    let y = buf.area.height - 1;
    let composing = matches!(ctx.screen.composer, Composer::Home { view: Some(_), .. });
    let open = ctx.screen.model.settings_open();
    let (word, keys): (&str, &[&str]) = if open {
        ("settings", &["Esc/q/s close", "^C quit"])
    } else if composing {
        ("write", &["Enter send", "Esc browse", "^C quit"])
    } else if matches!(ctx.screen.composer, Composer::Held { .. }) {
        ("held", &["Enter retry", "Esc browse", "^C quit"])
    } else if matches!(ctx.screen.composer, Composer::Home { .. }) {
        (
            "browse",
            &[
                "1-9 session",
                "! next need",
                "Tab overview / agents",
                "Enter write",
                "q quit",
            ],
        )
    } else {
        (
            "browse",
            &[
                "1-9 session",
                "! next need",
                "Tab overview / agents",
                "q quit",
            ],
        )
    };
    let width = buf.area.width;
    let x = put(
        buf,
        LEFT,
        y,
        word,
        width - LEFT,
        paint.fg(|p| p.text).add_modifier(Modifier::BOLD),
    );
    let rest = format!("   {}", keys.join("   "));
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
const SETTINGS_TABS: [&str; 3] = ["Quota", "Config", "About"];
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
    let selected = match tab {
        SettingsTab::Quota => 0,
        SettingsTab::Config => 1,
        SettingsTab::About => 2,
    };
    panel(
        buf,
        layout,
        paint,
        &Panel {
            title: SETTINGS_TITLE,
            tabs: &SETTINGS_TABS,
            selected,
            body: &settings_body(tab, bodies, paint),
            scroll,
        },
    );
}

/// What one full-pane panel draws: its title, tab row and scrolled body.
struct Panel<'a> {
    title: &'a str,
    tabs: &'a [&'a str],
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
    put(buf, LEFT, 0, panel.title, width - LEFT, heading);
    let mut x = LEFT;
    for (index, name) in panel.tabs.iter().enumerate() {
        let style = if index == panel.selected {
            paint.selected()
        } else {
            paint.fg(|p| p.text)
        };
        x = put(buf, x, 1, name, width.saturating_sub(x), style);
        x = put(buf, x, 1, "   ", width.saturating_sub(x), style);
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
fn settings_body(tab: SettingsTab, bodies: &SettingsBodies, paint: Paint) -> Vec<(String, Style)> {
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
    }
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
    use ratatui_core::layout::Rect;

    use super::{Composer, Screen, aged, dragged_to, draw};
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
            let screen = Screen {
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
            };
            let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
            let _ = draw(&screen, &mut buf);
            buf
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
    /// foreign middle both stay inside it.
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
        let shot = Shot::new();
        let home = "h".repeat(60);
        let buf = shot.draw(
            160,
            45,
            Composer::Foreign {
                home: &home,
                speaker: "lead",
            },
        );
        let (y, _) = spot(&buf, "viewed from the").expect("the foreign middle");
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
        assert_eq!(spot(&shot.draw(100, 20, READ_ONLY), "more rows"), None);
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
    /// (r0-r41) and not on the keys row (r42). Composing, the write keys end
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
        assert!(
            line(&buf, 42).concat().contains("Esc browse"),
            "the write keys"
        );
    }

    /// The cells a draft has: from the end of its `to api › lead   ` address
    /// (frames calm r39 @63, needs-you r24 @53) to the column's end.
    #[test]
    fn a_draft_has_the_cells_from_its_address_to_the_column_end() {
        for (width, height, start) in [(160, 43, 63), (100, 28, 53)] {
            let buf = Shot::new().draw(width, height, READ_ONLY);
            let end = rule_end(&buf).expect("the rule");
            let area = Rect::new(0, 0, width, height);
            let cells = super::draft_width(area, super::Split::default(), "api", "lead");
            assert_eq!(start + cells - 1, usize::from(end), "{width}x{height}");
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
    /// 1-5 and `↓ 4 more`. At 160 the frames' tallest list is 18 rows (frame
    /// many r5-r22, two a session) with the more row inside it, so ten
    /// sessions show 8 and the more row.
    #[test]
    fn a_long_fleet_shows_the_window_the_frames_do() {
        for (width, height, count, shown, more) in
            [(100, 28, 9, 5, "↓ 4 more"), (160, 43, 10, 8, "↓ 2 more")]
        {
            let mut shot = Shot::new();
            shot.fleet.rows = (1..=count)
                .map(|at| row(&format!("s{at}"), at, false))
                .collect();
            shot.model = Model::new(&shot.fleet);
            let buf = shot.draw(width, height, READ_ONLY);
            let size = format!("{width}x{height}");
            for at in 1..=count {
                let drawn = spot(&buf, &format!("{at} s{at} ")).is_some();
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
            assert!(width < 160 || y <= 22, "{size}: the more row at {y}");
            let last = spot(&buf, &format!("{shown} s{shown} "))
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

    /// The attention row's two forms. Frame many 160 r3 draws the full form
    /// when it fits, and every frame keeps blanks before the `!` key at the
    /// sidebar's end-1 (col 41 at 160), so a full form one cell too long for
    /// that room falls back to the compact one.
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
        assert!(
            attention.starts_with("  3 need you  ⚠ infra  ⚠ billing  ✖ ops  "),
            "{attention:?}"
        );
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
        // (c) a compact form too long for the room is clipped short of the key.
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
        // (d) the frame's compact spelling, whole when it fits.
        let mut shot = Shot::new();
        shot.fleet.rows = (1..=10)
            .map(|at| {
                let name = match at {
                    3 => "infra".to_owned(),
                    9 => "billing".to_owned(),
                    10 => "ops".to_owned(),
                    _ => format!("s{at}"),
                };
                let mut row = row(&name, at, matches!(at, 3 | 9 | 10));
                if at == 10 {
                    row.mark = Mark::Dead;
                }
                row
            })
            .collect();
        shot.model = Model::new(&shot.fleet);
        let attention = line(&shot.draw(160, 43, READ_ONLY), 3).concat();
        assert!(
            attention.starts_with("  3 need ⚠infra ⚠billing↓ ✖ops↓ "),
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

    /// Ruling 5b: a 100x20 pane of 13 sessions draws its list rule on row 18
    /// (two-row steps, five shown, the more row, tabs on 17). A press there
    /// moved nowhere keeps it; dragged up and back down past it, it stops on
    /// 18, never 19, though that leaves the tab body no rows.
    #[test]
    fn a_short_pane_keeps_its_undragged_list_rule_reachable() {
        let mut shot = sessions(13);
        let first = shot.draw(100, 20, READ_ONLY);
        assert_eq!(horizontal(&first), Some(18), "the undragged rule");
        let still = drag(&mut shot, &first, Edge::List, 18);
        assert_eq!(horizontal(&still), Some(18), "a press moved nowhere");
        let up = drag(&mut shot, &still, Edge::List, 12);
        assert_eq!(horizontal(&up), Some(12));
        let down = drag(&mut shot, &up, Edge::List, 19);
        assert_eq!(horizontal(&down), Some(18), "never past the undragged rule");
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
}
