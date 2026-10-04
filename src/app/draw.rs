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
use super::model::{Model, Tab};
use super::overview::Overview;
use crate::brief::age;
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

/// The colours one frame draws in: a palette, or none.
#[derive(Debug, Clone, Copy)]
pub(super) struct Paint(Option<Palette>);

impl Paint {
    fn of(look: Option<&Look>) -> Self {
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
}

/// Draw `screen` into `buf`, whose area starts at the origin — the app hands
/// it the terminal's whole frame. The answer is how many pages back the chat
/// can scroll in this frame, which bounds the model's scroll.
pub fn draw(screen: &Screen<'_>, buf: &mut Buffer) -> usize {
    let area = buf.area;
    let ctx = Ctx {
        screen,
        paint: Paint::of(screen.look.as_ref()),
        icons: screen.look.is_none_or(|look| look.icons),
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
        return 0;
    }
    // The frames' grounds: the chat's ink everywhere, the keys row and the
    // rule's own column included, and the panel under the sidebar alone.
    buf.set_style(area, ctx.paint.ground(|p| p.ink));
    let (height, width) = (area.height, area.width);
    let chat = if let Some(rule) = sidebar_width(area) {
        buf.set_style(
            Rect::new(0, 0, rule, height - 1),
            ctx.paint.ground(|p| p.base),
        );
        sidebar(&ctx, buf, rule);
        let border = ctx.paint.fg(|p| p.border);
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
    let pages = chat_column(&ctx, buf, chat.start + 2..width - 2);
    keys_row(&ctx, buf);
    pages
}

/// The cells a draft has on the composer row of a `width` x `height` pane,
/// after its `to <home> › <speaker>` address: what the caller wraps it to.
pub(crate) fn draft_width(width: u16, height: u16, home: &str, speaker: &str) -> usize {
    let area = Rect::new(0, 0, width, height);
    let left = sidebar_width(area).map_or(0, |rule| rule + 1) + 2;
    let room = usize::from(width.saturating_sub(2).saturating_sub(left));
    let address = Span::raw(format!("to {home} › {speaker}   ")).width();
    room.saturating_sub(address).max(1)
}

/// The sidebar's width, which is where its rule stands, or `None` when the
/// pane is too small to keep it.
fn sidebar_width(area: Rect) -> Option<u16> {
    if area.width < SIDEBAR_MIN.0 || area.height < SIDEBAR_MIN.1 {
        None
    } else if area.width >= 140 {
        Some(44)
    } else {
        Some(34)
    }
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
    /// The list for `count` sessions with `selected` kept in view: three
    /// rows each while they fit the budget, then two, then a window.
    fn of(count: usize, selected: Option<usize>, height: u16) -> Self {
        let budget = if height >= 40 { 18 } else { 11 };
        let (step, shown) = if count * 3 <= budget + 1 {
            (3, count)
        } else if count * 2 <= budget {
            (2, count)
        } else {
            (2, (budget - 1) / 2)
        };
        let at = selected.unwrap_or(0);
        let start = (at + 1).saturating_sub(shown).min(count - shown);
        let used = cells(shown) * step - u16::from(step == 3 && shown > 0);
        let more = (shown < count).then_some(LIST_TOP + used);
        let end = LIST_TOP + used.max(u16::from(count == 0)) + u16::from(more.is_some());
        Self {
            visible: start..start + shown,
            step,
            more,
            end,
        }
    }
}

fn sidebar(ctx: &Ctx<'_, '_>, buf: &mut Buffer, rule: u16) {
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
    let list = List::of(count, selected, buf.area.height);
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
        put(
            buf,
            LEFT,
            LIST_TOP,
            "No sessions.",
            room,
            paint.fg(|p| p.dim),
        );
    }
    for (step, at) in list.visible.clone().enumerate() {
        let y = LIST_TOP + cells(step) * list.step;
        session(ctx, buf, &fleet.rows[at], y, end, Some(at) == selected);
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
    let Some(entry) = ctx.screen.selected else {
        return;
    };
    let tabs = list.end + 1;
    tab_row(ctx, buf, tabs, end);
    let border = paint.fg(|p| p.border);
    put(
        buf,
        0,
        tabs + 1,
        &"─".repeat(usize::from(rule)),
        rule,
        border,
    );
    let mut body = match model.tab() {
        Tab::Overview => overview_rows(ctx, room, rule == 44),
        Tab::Agents => agent_rows(ctx, entry),
    };
    let floor = buf.area.height.saturating_sub(2);
    // A body that would lose rows gives up its gaps first.
    if body.len() > usize::from(floor.saturating_sub(tabs + 2)) {
        body.retain(|row| row.left.width() > 0);
    }
    for (step, row) in body.iter().enumerate() {
        let y = tabs + 2 + cells(step);
        if y >= floor {
            break;
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
fn tab_row(ctx: &Ctx<'_, '_>, buf: &mut Buffer, y: u16, end: u16) {
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
    put(
        buf,
        x + 3,
        y,
        &agents,
        end.saturating_sub(x + 3),
        style(tab == Tab::Agents),
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
fn chat_column(ctx: &Ctx<'_, '_>, buf: &mut Buffer, columns: Range<u16>) -> usize {
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
            Composer::Home { .. } | Composer::ReadOnly { .. } => {
                screen.pair.first().map(String::as_str)
            }
            Composer::Foreign { .. } | Composer::NoHome => None,
        })
        .unwrap_or_default();
    let clock = view::Style::resolve(true, screen.look, screen.zone, main);
    let rows = super::lane::rows(screen.lane, usize::from(room), paint, &clock, main);
    let room_rows = usize::from(bottom.saturating_sub(top));
    let pages = screen.model.pages();
    let last = rows
        .len()
        .saturating_sub(pages.saturating_mul(room_rows))
        .max(room_rows.min(rows.len()));
    let first = last.saturating_sub(room_rows);
    let y0 = bottom - cells(last - first);
    for (step, line) in rows[first..last].iter().enumerate() {
        put_line(buf, left, y0 + cells(step), line, room);
    }
    if pages > 0 && last < rows.len() {
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
    rows.len()
        .saturating_sub(room_rows)
        .div_ceil(room_rows.max(1))
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

/// The keys row, the full width of the last row.
fn keys_row(ctx: &Ctx<'_, '_>, buf: &mut Buffer) {
    let paint = ctx.paint;
    let y = buf.area.height - 1;
    let composing = matches!(ctx.screen.composer, Composer::Home { view: Some(_), .. });
    let (word, keys): (&str, &[&str]) = if composing {
        ("write", &["Enter send", "Esc browse", "^C quit"])
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
    put(
        buf,
        x,
        y,
        &rest,
        width.saturating_sub(x),
        paint.fg(|p| p.dim),
    );
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

    use super::{Composer, Screen, aged, draw};
    use crate::app::fleet::{Counts, Facts, Fleet, Line2, Row};
    use crate::app::model::Model;
    use crate::app::overview::{Open, Overview};
    use crate::brief::TopicLine;
    use crate::console::input::View;
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
}
