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

/// Draw `screen` into `buf`, over `buf.area`.
pub fn draw(screen: &Screen<'_>, buf: &mut Buffer) {
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
        put(buf, area.x, area.y, &line, area.width, dim);
        return;
    }
    buf.set_style(area, ctx.paint.ground(|p| p.base));
    let (height, width) = (area.height, area.width);
    let chat = if let Some(rule) = sidebar_width(area) {
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
    let ground = Rect::new(chat.start, 1, chat.end - chat.start, height - 2);
    buf.set_style(ground, ctx.paint.ground(|p| p.ink));
    chat_column(&ctx, buf, chat.start + 2..width - 2, chat.start > 0);
    keys_row(&ctx, buf);
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
/// bottom, the composer. `beside` says a sidebar stands to its left.
fn chat_column(ctx: &Ctx<'_, '_>, buf: &mut Buffer, columns: Range<u16>, beside: bool) {
    let screen = ctx.screen;
    let paint = ctx.paint;
    let height = buf.area.height;
    let (left, end) = (columns.start, columns.end);
    let room = end - left;
    let (dim, border) = (paint.fg(|p| p.dim), paint.fg(|p| p.border));
    header(ctx, buf, left, end, beside);
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
}

/// The chat header: the session, its pair or where it is viewed from, and
/// right-aligned its branch (wide only) and its activity.
fn header(ctx: &Ctx<'_, '_>, buf: &mut Buffer, left: u16, end: u16, beside: bool) {
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
    let wide = beside && buf.area.width >= 140;
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

/// `text` and its age when both fit `room`; else the text alone, clipped.
fn aged(text: &str, since: Option<i64>, room: u16) -> String {
    let full = format!("{text} · {}", age(since));
    if full.chars().count() <= usize::from(room) {
        full
    } else {
        clip_head(text, usize::from(room))
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
