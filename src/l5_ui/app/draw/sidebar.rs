//! The sidebar: the session list, its summary and the Agents block.

use std::fmt::Write as _;
use std::ops::Range;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::{Line, Span};

use crate::app::fleet::{Counts, Facts, Fleet, Line2, Row, summary};
use crate::app::model::{Edge, Stopped};
use crate::app::overview::Open;
use crate::app::select;
use crate::brief::age;
use crate::digest::{SessionEntry, Status};
use crate::theme::Mark;
use crate::tmux::PickerAgent;

use super::geometry::{LIST_TOP, List, dragged, list_rows};
use super::layout::{Hit, Layout};
use super::paint::{LEFT, aged_by, cells, drawn_cols, fit, put, put_line};
use super::{Composer, Ctx, Screen};

/// Where a session row's name starts: one blank after its mark.
pub(super) const NAME: u16 = 4;

/// One line of the session list: a session by its place in the fleet, or the
/// stopped group's one toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listed {
    Session(usize),
    Toggle,
}

/// The list's lines: the fleet's sessions in order and, while any session is
/// stopped, the group's toggle before the first stopped row in the list, else
/// last.
fn list_lines(fleet: &Fleet, stopped: Stopped) -> Vec<Listed> {
    let toggle = (stopped.total > 0).then(|| {
        (fleet.rows.iter())
            .position(Row::is_stopped)
            .unwrap_or(fleet.rows.len())
    });
    let mut lines: Vec<Listed> = (0..fleet.rows.len()).map(Listed::Session).collect();
    if let Some(at) = toggle {
        lines.insert(at, Listed::Toggle);
    }
    lines
}

/// How many lines the list has for `fleet` and its stopped group open: the
/// running sessions, the group's toggle and every stopped session, whether the
/// rows hold them or not. The list's size comes from this, so the group's state
/// never changes it.
pub(crate) fn list_len(fleet: &Fleet, stopped: Stopped) -> usize {
    let shown = fleet.rows.iter().filter(|row| row.is_stopped()).count();
    fleet.rows.len() + usize::from(stopped.total > 0) + stopped.total.saturating_sub(shown)
}

pub(super) fn sidebar(ctx: &Ctx<'_, '_>, buf: &mut Buffer, rule: u16, layout: &mut Layout) {
    let Screen { fleet, model, .. } = *ctx.screen;
    let paint = ctx.paint;
    let end = rule - 2;
    let room = end - LEFT;
    let lines = list_lines(fleet, model.stopped());
    let count = lines.len();
    let selected = model.position(fleet);
    let selected_line =
        selected.and_then(|at| lines.iter().position(|line| *line == Listed::Session(at)));
    // The region is what the list holds with the stopped group open, whatever
    // the group's state: a closed group leaves its rows empty, a list too long
    // for it scrolls inside it, and the Agents section below never moves for a
    // toggle.
    let full = list_len(fleet, model.stopped());
    let (rows, pinned) = list_rows(buf.area, model.split(), full);
    let region = List::of(full, None, rows, pinned, None).end - LIST_TOP;
    let list = List::of(count, selected_line, region, true, model.list_top());
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
    // The session a list line stands for, if it is one.
    let row_of = |at: usize| match lines[at] {
        Listed::Session(row) => Some(&fleet.rows[row]),
        Listed::Toggle => None,
    };
    // An arrow for each side of the list that hides a needy row, at the
    // summary line's right end; the counts keep clear of it.
    let hides = |span: Range<usize>| {
        span.into_iter()
            .any(|at| row_of(at).is_some_and(|row| row.needy))
    };
    let mut arrows = String::new();
    arrows.extend(hides(0..list.visible.start).then_some('↑'));
    arrows.extend(hides(list.visible.end..count).then_some('↓'));
    summary_head(ctx, buf, rule, &arrows);
    if count == 0 {
        let empty = if ctx.wait.fleet {
            "loading"
        } else {
            "No sessions."
        };
        put(buf, LEFT, LIST_TOP, empty, room, paint.fg(|p| p.dim));
    }
    // The one lifted row is what the chat column shows: the session while its
    // shared chat is, a seat only while that seat's screen is. A session whose
    // seat is shown stays a context, never lifted; so does a session whose
    // chat a file read stands in for.
    let viewing = ctx.screen.composer.in_place();
    for (step, at) in list.visible.clone().enumerate() {
        let y = LIST_TOP + cells(step);
        let Listed::Session(row) = lines[at] else {
            stopped_toggle(ctx, buf, y, end, model.stopped());
            layout.record(buf, Rect::new(0, y, rule, 1), Hit::Stopped);
            continue;
        };
        let place = match (Some(row) == selected, viewing) {
            (true, false) => Place::Lifted,
            (true, true) => Place::Context,
            (false, _) => Place::Plain,
        };
        session(ctx, buf, &fleet.rows[row], y, end, place);
        layout.record(
            buf,
            Rect::new(0, y, rule, 1),
            Hit::Session(fleet.rows[row].name.clone()),
        );
    }
    if let Some(y) = list.more {
        let hidden = |at: &usize| !list.visible.contains(at);
        let needy = (0..count)
            .filter(|at| hidden(at) && row_of(*at).is_some_and(|row| row.needy))
            .count();
        let (above, below) = (list.visible.start, count - list.visible.end);
        more_row(ctx, buf, y, room, tone, [above, below, needy]);
    }
    let Some(entry) = ctx.screen.selected.filter(|_| !ctx.wait.fleet) else {
        return;
    };
    let tabs = list.end + 1;
    agents_heading(ctx, buf, tabs, rule);
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

/// The list's `more` row at `y`: how many lines hide above and below the
/// window, and how many of those sessions need the human (`counts` is above,
/// below, needy).
fn more_row(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    y: u16,
    room: u16,
    tone: Style,
    counts: [usize; 3],
) {
    let [above, below, needy] = counts;
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
    // A seat is lifted only while its screen is what the chat column shows. A
    // seat the human stepped to with `n` / `p` carries the cursor instead, a
    // gutter glyph and no ground: it is where the seat keys act, not a view.
    let viewed = match ctx.screen.composer {
        Composer::Viewing { seat } => Some(seat),
        _ => None,
    };
    for (step, row) in drawn.enumerate() {
        let y = top + cells(step);
        let seat = row.seat.as_deref();
        let lifted = seat.is_some() && seat == viewed;
        let cursor = !lifted && seat.is_some() && seat == asked;
        if let Some(seat) = seat {
            layout.record(
                buf,
                Rect::new(LEFT, y, end - LEFT, 1),
                Hit::Seat(seat.to_owned()),
            );
        }
        if row.toggle {
            layout.record(buf, Rect::new(LEFT, y, end - LEFT, 1), Hit::Spawned);
        }
        // A seat's glyph sits on its head row, or on its continuation when
        // that is the first row the body shows.
        let marked = !row.continued || step == 0;
        if lifted && marked {
            let glyph = if ctx.icons { "▸" } else { ">" };
            let ink = paint.fg(|p| p.selected_ink).add_modifier(Modifier::BOLD);
            put(buf, 0, y, glyph, 1, ink);
        } else if cursor && marked {
            let glyph = if ctx.icons { "▹" } else { "-" };
            put(buf, 0, y, glyph, 1, paint.fg(|p| p.text));
        }
        let used = put_line(buf, LEFT, y, &row.left, room);
        if let Some(right) = &row.right {
            let width = cells(right.width());
            if used + 1 + width <= end {
                put_line(buf, end - width, y, right, width);
            }
        }
        if lifted {
            paint.lift(buf, Rect::new(0, y, end, 1));
        }
    }
}

/// Paint the quiet one-line summary on the sidebar ground, leaving a blank row
/// before the list. Needy arrows sit clear of its counts.
fn summary_head(ctx: &Ctx<'_, '_>, buf: &mut Buffer, rule: u16, arrows: &str) {
    let (paint, fleet) = (ctx.paint, ctx.screen.fleet);
    let (end, room) = (rule - 2, rule - 2 - LEFT);
    let wide = cells(arrows.chars().count());
    let counts = room.saturating_sub(wide + u16::from(wide > 0));
    let needy =
        fleet.rows.iter().any(|row| row.needy) || ctx.screen.model.stopped().hidden_needy > 0;
    let tone = paint.fg(|p| if needy { p.needs_you } else { p.text });
    put_line(
        buf,
        LEFT,
        0,
        &summary_band(ctx, fleet, counts, tone),
        counts,
    );
    put(buf, end - wide, 0, arrows, wide, paint.fg(|p| p.needs_you));
}

/// The sidebar's top line: the agents of the running sessions, how many work
/// and how many sessions need the human — `Agents 32 · ● ◉ Working 10 · ⚠ Needs
/// you 1`. Working counts the seats that work and those that run background
/// work. It tries four rungs and keeps the first that fits `room`: the words,
/// short labels (`Agt 32 · ● ◉ Wrk 10 · ⚠ Need 1`), an icon and a count each (`◈
/// 32 ● ◉ 10 ⚠ 1`), the needs-you count alone. Working wears both marks its
/// rows wear (working and background), Needs you its one, before the label,
/// Agents none. A count is never cut and a
/// lower bound keeps its `+?`. Counts use regular text ink and the labels and
/// separators dim ink; the needs-you count wears `tone`, and a mark the accent
/// it always did.
fn summary_band(ctx: &Ctx<'_, '_>, fleet: &Fleet, room: u16, tone: Style) -> Line<'static> {
    let (mut sum, paint) = (summary(fleet), ctx.paint);
    sum.needy += ctx.screen.model.stopped().hidden_needy;
    let more = if sum.unknown { "+?" } else { "" };
    let numbers = [
        format!("{}{more}", sum.agents),
        format!("{}{more}", sum.working),
        sum.needy.to_string(),
    ];
    let (text, dim) = (paint.fg(|p| p.text), paint.fg(|p| p.dim));
    let inks = [text, text, tone];
    let marks: [Vec<(&'static str, Style)>; 3] = [
        vec![(if ctx.icons { "◈" } else { "@" }, text)],
        vec![
            (Mark::Working.glyph(ctx.icons), paint.mark(Mark::Working)),
            (
                Mark::Background.glyph(ctx.icons),
                paint.mark(Mark::Background),
            ),
        ],
        vec![(Mark::NeedsYou.glyph(ctx.icons), tone)],
    ];
    let words = |labels: [&'static str; 3]| {
        let mut spans = Vec::new();
        for (at, (label, number)) in labels.into_iter().zip(&numbers).enumerate() {
            spans.extend((!spans.is_empty()).then(|| Span::styled(" · ", dim)));
            // Agents wears no mark; Working and Needs you wear their rows'.
            if at > 0 {
                for (glyph, ink) in &marks[at] {
                    spans.push(Span::styled(*glyph, *ink));
                    spans.push(Span::styled(" ", dim));
                }
            }
            spans.push(Span::styled(label, dim));
            spans.push(Span::styled(format!(" {number}"), inks[at]));
        }
        Line::from(spans)
    };
    let icons = |keep: &[usize]| {
        let mut spans = Vec::new();
        for &at in keep {
            spans.extend((!spans.is_empty()).then(|| Span::styled(" ", dim)));
            for (n, (glyph, ink)) in marks[at].iter().enumerate() {
                spans.extend((n > 0).then(|| Span::styled(" ", dim)));
                spans.push(Span::styled(*glyph, *ink));
            }
            spans.push(Span::styled(format!(" {}", numbers[at]), inks[at]));
        }
        Line::from(spans)
    };
    [
        words(["Agents", "Working", "Needs you"]),
        words(["Agt", "Wrk", "Need"]),
        icons(&[0, 1, 2]),
        icons(&[2]),
    ]
    .into_iter()
    .find(|line| cells(line.width()) <= room)
    .unwrap_or_default()
}

/// The stopped group's one list line: `▸ Stopped (N)` closed, `▾ Stopped (N)`
/// open (`>` and `v` with `icons = off`), dim, in the cells a session's mark
/// and name use.
fn stopped_toggle(ctx: &Ctx<'_, '_>, buf: &mut Buffer, y: u16, end: u16, stopped: Stopped) {
    let dim = ctx.paint.fg(|p| p.dim);
    let glyph = match (stopped.open, ctx.icons) {
        (false, true) => "▸",
        (true, true) => "▾",
        (false, false) => ">",
        (true, false) => "v",
    };
    put(buf, LEFT, y, glyph, 1, dim);
    let label = format!("Stopped ({})", stopped.total);
    put(buf, NAME, y, &label, end.saturating_sub(NAME), dim);
}

/// How a session row stands against the view the chat column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Its shared chat is shown: the one lifted row.
    Lifted,
    /// It is the selected session, but a seat's screen is shown: its name
    /// keeps the title ink so the seats under it read as its own.
    Context,
    /// Any other session.
    Plain,
}

/// The fewest cells a session name is cut to beside its tally.
const NAME_MIN: u16 = 4;

/// One session's row at `y`: its mark, its name, `◂` when it is the window's
/// own session, and its seat tally at the right end (a stopped session says
/// so instead). The tally and the glyph give way before the name does.
fn session(ctx: &Ctx<'_, '_>, buf: &mut Buffer, row: &Row, y: u16, end: u16, place: Place) {
    let paint = ctx.paint;
    let dim = paint.fg(|p| p.dim);
    let stopped = matches!(row.counts, Counts::Stopped);
    // The lifted session wears the selection a seat does: one `▸` on its row,
    // then the selected ground. A session that needs you says so by its mark,
    // never by an edge.
    if place == Place::Lifted {
        let glyph = if ctx.icons { "▸" } else { ">" };
        put(
            buf,
            0,
            y,
            glyph,
            1,
            paint.fg(|p| p.selected_ink).add_modifier(Modifier::BOLD),
        );
    }
    let mark = if stopped { dim } else { paint.mark(row.mark) };
    put(buf, LEFT, y, row.mark.glyph(ctx.icons), 1, mark);
    // The unread cue rides the blank gutter cell, so no column moves.
    if place == Place::Plain && ctx.screen.model.is_unread(&row.name) {
        let cue = if ctx.icons { "\u{25b8}" } else { ">" };
        let style = paint.fg(|p| p.title).add_modifier(Modifier::BOLD);
        put(buf, 1, y, cue, 1, style);
    }
    let name = match (stopped, place) {
        (true, _) => dim,
        (false, Place::Context) => paint
            .fg(|p| p.title)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        (false, _) => paint.fg(|p| p.text).add_modifier(Modifier::BOLD),
    };
    let fitted = session_fit(
        &row.name,
        row.home,
        &tally_choices(ctx, row),
        end.saturating_sub(NAME),
    );
    let after = put(buf, NAME, y, &fitted.name, end.saturating_sub(NAME), name);
    if fitted.home {
        let glyph = if ctx.icons { "◂" } else { "<" };
        put(buf, after + 1, y, glyph, 1, dim);
    }
    let width: u16 = cells(spans_width(&fitted.right));
    let mut x = end.saturating_sub(width);
    for (at, (text, style)) in fitted.right.iter().enumerate() {
        x += u16::from(at > 0);
        x = put(buf, x, y, text, end.saturating_sub(x), *style);
    }
    if place == Place::Lifted {
        paint.lift(buf, Rect::new(0, y, end, 1));
    }
}

/// A session row's right-aligned spans: text and style, left to right.
type Spans = Vec<(String, Style)>;

/// The right side of `row`, richest first: a stopped session's age, then its
/// word alone; a running one's whole tally, then each shorter one closed on
/// `…` down to its most urgent count. Request debt rides last, dim — `↩<n>`
/// (`r<n>` without icons) — so it is the first thing to give way and never
/// reads as a mark.
fn tally_choices(ctx: &Ctx<'_, '_>, row: &Row) -> Vec<Spans> {
    let (paint, dim) = (ctx.paint, ctx.paint.fg(|p| p.dim));
    let debt = row
        .unanswered
        .filter(|count| *count > 0)
        .map(|count| (unanswered_tally(ctx.icons, count), dim));
    let with_debt = |alone: Spans| -> Vec<Spans> {
        match &debt {
            Some(debt) => {
                let mut rich = alone.clone();
                rich.push(debt.clone());
                vec![rich, alone]
            }
            None => vec![alone],
        }
    };
    match &row.counts {
        Counts::Stopped => {
            let since = match row.line2 {
                Line2::NotRunning { stopped_secs, .. } => stopped_secs,
                _ => None,
            };
            vec![
                vec![(format!("stopped {}", age(since)), dim)],
                vec![("stopped".to_owned(), dim)],
            ]
        }
        Counts::Marks(tallied) if !tallied.is_empty() => {
            let parts: Spans = tallied
                .iter()
                .map(|(mark, blocked, count)| {
                    let letter = if *blocked { "B" } else { "" };
                    let text = format!("{}{letter}{count}", mark.glyph(ctx.icons));
                    (text, paint.mark(*mark))
                })
                .collect();
            let mut choices = with_debt(parts.clone());
            choices.extend((1..parts.len()).rev().map(|kept| {
                let mut spans = parts[..kept].to_vec();
                spans.push(("…".to_owned(), dim));
                spans
            }));
            choices
        }
        Counts::Marks(_) => with_debt(vec![("no agents".to_owned(), dim)]),
        Counts::Unknown => with_debt(vec![("?".to_owned(), dim)]),
    }
}

/// A session's unanswered-request count as the sidebar draws it.
fn unanswered_tally(icons: bool, count: usize) -> String {
    format!("{}{count}", if icons { "↩" } else { "r" })
}

/// The cells `spans` spend, one blank between neighbours.
fn spans_width(spans: &[(String, Style)]) -> usize {
    let words: usize = spans
        .iter()
        .map(|(text, _)| usize::from(drawn_cols(text)))
        .sum();
    words + spans.len().saturating_sub(1)
}

/// What a session row draws in its `room` cells right of the mark.
#[derive(Debug, PartialEq)]
struct Fitted {
    name: String,
    /// Whether `◂` follows the name.
    home: bool,
    right: Spans,
}

/// Fit a session's `name`, its home glyph and its right-side `choices` (richest
/// first) into `room` cells, giving up in this order: the glyph, the richer
/// choices down to the poorest, then the name's tail down to [`NAME_MIN`]
/// cells beside that choice, then the right side whole. A name is cut last.
fn session_fit(name: &str, home: bool, choices: &[Spans], room: u16) -> Fitted {
    let name_cols = drawn_cols(name);
    let glyph = if home { 2 } else { 0 };
    let fits = |glyph: u16, right: &Spans| {
        let gap = u16::from(!right.is_empty());
        name_cols + glyph + gap + cells(spans_width(right)) <= room
    };
    let first = choices.first().filter(|right| fits(glyph, right));
    if let Some(right) = first {
        return Fitted {
            name: name.to_owned(),
            home,
            right: right.clone(),
        };
    }
    if let Some(right) = choices.iter().find(|right| fits(0, right)) {
        return Fitted {
            name: name.to_owned(),
            home: false,
            right: right.clone(),
        };
    }
    if let Some(poorest) = choices.last() {
        let kept = room.saturating_sub(1 + cells(spans_width(poorest)));
        if kept >= NAME_MIN {
            return Fitted {
                name: fit(name, "", kept),
                home: false,
                right: poorest.clone(),
            };
        }
    }
    Fitted {
        name: fit(name, "", room),
        home: false,
        right: Vec::new(),
    }
}

/// The heading over the body: `Agents` and the seat count, a band like the
/// summary's so both sections open alike.
fn agents_heading(ctx: &Ctx<'_, '_>, buf: &mut Buffer, y: u16, rule: u16) {
    let words = match ctx.screen.agents {
        Some(Facts::Seats { agents, .. }) => format!("Agents {}", agents.len()),
        _ => "Agents".to_owned(),
    };
    let style = ctx.paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    ctx.paint.band(buf, Rect::new(0, y, rule, 1));
    put(buf, LEFT, y, &words, rule - 2 - LEFT, style);
}

/// A body row: its left text and, right-aligned, an optional word.
struct Cells {
    left: Line<'static>,
    right: Option<Line<'static>>,
    /// The seat this row belongs to, when it is one of a seat's rows.
    seat: Option<String>,
    /// A seat's second row: its label, under the name, when the head row
    /// could not hold it.
    continued: bool,
    /// The spawned group's one line: a press on it folds or shows the group.
    toggle: bool,
}

impl Cells {
    fn of(spans: Vec<Span<'static>>) -> Self {
        Self {
            left: Line::from(spans),
            right: None,
            seat: None,
            continued: false,
            toggle: false,
        }
    }
}

/// The Agents body: per seat one label row, a second when its label does not
/// fit beside its name and state, and a dim detail row under them for its
/// newest declaration's own reason and age (`no reason given` when it left
/// none, `reason unavailable` when the journal could not be read). The label
/// row carries its mark, name, the client label and the model its frame drew
/// (a missing one is left out, never guessed) and the effort, each only when
/// known and never the profile, and its
/// state word at the right end, in the human words
/// ([`crate::theme::seat_word`]), never the verdict word. The seat's open
/// REQUEST text stays in the chat header, not under the seat; only its own
/// DECLARATION reason is drawn under it. An ask from a seat the roster lacks is
/// a row of its own after the roster; a session ae has no seat facts for says
/// why. Request debt closes the body as one dim row — `↩ <n> unanswered ·
/// oldest <age>` — never a mark.
fn agent_rows(ctx: &Ctx<'_, '_>, entry: &SessionEntry, room: u16) -> Vec<Cells> {
    let dim = ctx.paint.fg(|p| p.dim);
    let gap = |words: &str| vec![Cells::of(vec![Span::styled(words.to_owned(), dim)])];
    let open = &ctx.screen.overview.open;
    let (mut rows, agents) = match ctx.screen.agents {
        _ if entry.status == Status::Stopped => (gap("Not running: no seat facts."), &[][..]),
        Some(Facts::Seats { agents, .. }) => (Vec::new(), agents.as_slice()),
        Some(Facts::OtherServer) => (gap("No seat facts: on another tmux server."), &[][..]),
        Some(Facts::NotPublished) | None => {
            (gap("No seat facts: watchdog publishes none."), &[][..])
        }
    };
    // The roster's slots, read by the App, say which seats are spawned: the
    // fixed seats keep their order, the spawned seats follow under one line.
    let group = ctx.screen.model.spawned();
    let (held, fixed): (Vec<&PickerAgent>, Vec<&PickerAgent>) = agents
        .iter()
        .partition(|agent| group.names.contains(&agent.name));
    let held_drawn: &[&PickerAgent] = if group.folded { &[] } else { &held };
    // The names of the seats drawn share one column, so the models line up
    // under it.
    let column = fixed
        .iter()
        .chain(held_drawn)
        .map(|agent| drawn_cols(&agent.name))
        .max()
        .unwrap_or(0);
    // Where the session's waits end, above the seats whose chain it follows.
    if let Some(words) = entry.wait_end_words().filter(|_| !agents.is_empty()) {
        rows.push(Cells::of(vec![Span::styled(fit(&words, "", room), dim)]));
    }
    for agent in &fixed {
        rows.extend(agent_block(ctx, entry, agent, column, room));
    }
    if !held.is_empty() {
        rows.push(spawned_line(ctx, &held, group.folded, room));
    }
    for agent in held_drawn {
        rows.extend(agent_block(ctx, entry, agent, column, room));
    }
    // A seat the roster lacks has no row of its own, a hidden seat included:
    // only a seat no fact lists is drawn from its ask.
    let listed = |ask: &&Open| agents.iter().any(|agent| agent.name == ask.seat);
    for ask in open.iter().filter(|ask| !listed(ask)) {
        let seat = Seat {
            name: &ask.seat,
            mark: Mark::NeedsYou,
            state: "needs you",
            meta: ["", ""],
        };
        rows.extend(seat_rows(ctx, &seat, 0, room));
    }
    let overview = ctx.screen.overview;
    if let Some(count) = overview.unanswered.filter(|count| *count > 0) {
        let glyph = if ctx.icons { "↩" } else { "r" };
        let words = format!(
            "{glyph} {count} unanswered · oldest {}",
            age(overview.unanswered_oldest_secs)
        );
        rows.push(Cells::of(vec![Span::styled(fit(&words, "", room), dim)]));
    }
    rows
}

/// One seat's rows in the Agents body: its label rows and its declaration row.
fn agent_block(
    ctx: &Ctx<'_, '_>,
    entry: &SessionEntry,
    agent: &PickerAgent,
    column: u16,
    room: u16,
) -> Vec<Cells> {
    // The client label, then the model, each only when known: two seats
    // on one model under different clients read apart.
    let label = [agent.client.as_str(), agent.model.as_str()]
        .into_iter()
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    // A typed wait names its target in place of the generic wait word;
    // what the narrow row would cut rides its declaration row.
    let typed = entry
        .agents
        .iter()
        .find(|seat| seat.name == agent.name && agent.state == "waiting-agent")
        .and_then(|seat| seat.typed_wait(ctx.screen.now))
        .map(|(on, _, _, moved)| on.words(false, moved));
    let seat = Seat {
        name: &agent.name,
        mark: agent.mark(),
        state: typed
            .as_deref()
            .unwrap_or_else(|| crate::theme::seat_word(&agent.state)),
        meta: [label.as_str(), agent.effort.as_str()],
    };
    let mut rows = seat_rows(ctx, &seat, column, room);
    rows.extend(declaration_row(ctx, entry, &agent.name, room));
    rows
}

/// The spawned group's one line, dim, in the cells a session's mark and name
/// use, like the Stopped line: `▾ Spawned (N)` open, `▸ Spawned (N)` folded
/// (`v` and `>` with `icons = off`). Folded, it carries what the hidden seats
/// would say, in the session tally's grammar and order — `▸ Spawned (5) ⚠1
/// ●3` — so a seat that needs you is never out of sight; a tally that does
/// not fit gives way like a session's, down to its most urgent count.
fn spawned_line(ctx: &Ctx<'_, '_>, held: &[&PickerAgent], folded: bool, room: u16) -> Cells {
    let (paint, dim) = (ctx.paint, ctx.paint.fg(|p| p.dim));
    let glyph = match (folded, ctx.icons) {
        (true, true) => "▸",
        (false, true) => "▾",
        (true, false) => ">",
        (false, false) => "v",
    };
    let label = format!("Spawned ({})", held.len());
    let used = 2 + drawn_cols(&label);
    let mut spans = vec![
        Span::styled(glyph, dim),
        Span::raw(" "),
        Span::styled(label, dim),
    ];
    if folded {
        let hidden: Vec<PickerAgent> = held.iter().map(|agent| (*agent).clone()).collect();
        let parts: Vec<(String, Style)> = crate::app::fleet::tally(&hidden)
            .into_iter()
            .map(|(mark, blocked, count)| {
                let letter = if blocked { "B" } else { "" };
                (
                    format!("{}{letter}{count}", mark.glyph(ctx.icons)),
                    paint.mark(mark),
                )
            })
            .collect();
        let fits = |kept: usize, cut: bool| {
            let width: usize = parts[..kept]
                .iter()
                .map(|(text, _)| 1 + usize::from(drawn_cols(text)))
                .sum();
            used + cells(width + 2 * usize::from(cut)) <= room
        };
        // The richest tally that fits, then each shorter one closed on `…`.
        let choice = (1..=parts.len())
            .rev()
            .map(|kept| (kept, kept < parts.len()))
            .find(|(kept, cut)| fits(*kept, *cut));
        if let Some((kept, cut)) = choice {
            for (text, style) in &parts[..kept] {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(text.clone(), *style));
            }
            if cut {
                spans.push(Span::styled(" …", dim));
            }
        }
    }
    Cells {
        left: Line::from(spans),
        right: None,
        seat: None,
        continued: false,
        toggle: true,
    }
}

/// What one seat row says.
struct Seat<'a> {
    name: &'a str,
    mark: Mark,
    /// The state word, drawn at the right end.
    state: &'a str,
    /// The client and model label and the effort; empty when unknown.
    meta: [&'a str; 2],
}

/// `seat`'s rows in `room` cells: its mark, its name and its state word at the
/// right end, and its whole label (client, model and effort) aligned at
/// `column` when it fits there; else a second row carries the label under the
/// name, the effort left off only when that row cannot hold both and the
/// client and model cut on `…` only past it. Short of room the name's tail goes down to [`NAME_MIN`]
/// cells, and last the state word. A state word's parenthetical gives way
/// BEFORE any of that: when the full word would be left off, or would push the
/// label to a second row, its head word (`needs you (blocked)` → `needs you`)
/// is laid out instead if that keeps the state, or the label beside it.
fn seat_rows(ctx: &Ctx<'_, '_>, seat: &Seat<'_>, column: u16, room: u16) -> Vec<Cells> {
    let full = seat_row(ctx, seat, column, room);
    let head = seat
        .state
        .split_once(" (")
        .map_or(seat.state, |(head, _)| head);
    if head == seat.state {
        return full;
    }
    let short = Seat {
        state: head,
        ..*seat
    };
    let short = seat_row(ctx, &short, column, room);
    // What a layout keeps, poorest first: the state word, then one row.
    let kept = |rows: &[Cells]| {
        (
            rows.first().is_some_and(|row| row.right.is_some()),
            rows.len() == 1,
        )
    };
    if kept(&short) > kept(&full) {
        short
    } else {
        full
    }
}

/// `seat`'s rows with its state word exactly as given.
fn seat_row(ctx: &Ctx<'_, '_>, seat: &Seat<'_>, column: u16, room: u16) -> Vec<Cells> {
    let (paint, mark) = (ctx.paint, seat.mark);
    let (text, dim) = (paint.fg(|p| p.text), paint.fg(|p| p.dim));
    // The mark and its blank.
    let lead = 2;
    let name_cols = drawn_cols(seat.name);
    let state_cols = drawn_cols(seat.state);
    let state = lead + name_cols.min(NAME_MIN) + 1 + state_cols <= room;
    let tail = if state { 1 + state_cols } else { 0 };
    let name = fit(seat.name, "", room.saturating_sub(lead + tail));
    let named = drawn_cols(&name);
    let start = lead + column.max(named) + 2;
    let free = room.saturating_sub(start + tail);
    let [label, effort] = seat.meta;
    let both = [label, effort]
        .into_iter()
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let inline = !both.is_empty() && drawn_cols(&both) <= free;
    let mut spans = vec![
        Span::styled(mark.glyph(ctx.icons), paint.mark(mark)),
        Span::raw(" "),
        Span::styled(name, text),
    ];
    if inline {
        let pad = usize::from(start - lead - named);
        spans.push(Span::styled(format!("{}{both}", " ".repeat(pad)), dim));
    }
    let mut rows = vec![Cells {
        left: Line::from(spans),
        right: state.then(|| Line::from(Span::styled(seat.state.to_owned(), paint.mark(mark)))),
        seat: Some(seat.name.to_owned()),
        continued: false,
        toggle: false,
    }];
    if !both.is_empty() && !inline {
        let width = room.saturating_sub(lead);
        let words = if drawn_cols(&both) <= width {
            both
        } else if label.is_empty() {
            fit(&both, "", width)
        } else {
            fit(label, "", width)
        };
        rows.push(Cells {
            left: Line::from(vec![
                Span::raw(" ".repeat(usize::from(lead))),
                Span::styled(words, dim),
            ]),
            right: None,
            seat: Some(seat.name.to_owned()),
            continued: true,
            toggle: false,
        });
    }
    rows
}

/// The dim row under seat `name`: its newest declaration's own reason and the
/// declaration's age, a clipped reason keeping its age. Nothing when the entry
/// carries no declaration for it. A source the seat's state cannot be trusted
/// from says `reason unavailable`, never an absence, and an absent declaration
/// says `no reason given` with no age: ae never supplies a reason of its own.
fn declaration_row(
    ctx: &Ctx<'_, '_>,
    entry: &SessionEntry,
    name: &str,
    room: u16,
) -> Option<Cells> {
    let agent = entry.agents.iter().find(|agent| agent.name == name)?;
    let declaration = agent.declaration.as_ref()?;
    let (mut reason, age) = if entry.agent_state_is_exact() {
        declaration.words(ctx.screen.now)
    } else {
        crate::digest::Declaration::Unreadable.words(ctx.screen.now)
    };
    if let Some((on, missing, overdue, moved)) = agent
        .typed_wait(ctx.screen.now)
        .filter(|_| entry.agent_state_is_exact())
    {
        let notes = [
            (missing && moved.is_none()).then(|| format!("{} (not found)", on.label())),
            overdue.map(|age| format!("overdue {age}")),
        ];
        for note in notes.into_iter().flatten().rev() {
            reason = format!("{note} · {reason}");
        }
    }
    // The mark and its blank, so the words sit under the name.
    let lead = 2;
    let width = room.saturating_sub(lead);
    let words = match age {
        Some(age) => aged_by(&crate::board::terminal_text(&reason), &age, width),
        None => fit(&crate::board::terminal_text(&reason), "", width),
    };
    Some(Cells {
        left: Line::from(vec![
            Span::raw(" ".repeat(usize::from(lead))),
            Span::styled(words, ctx.paint.fg(|p| p.dim)),
        ]),
        right: None,
        seat: Some(name.to_owned()),
        continued: true,
        toggle: false,
    })
}
