//! The chat column: the lane, the composer and the hint row.

use std::ops::Range;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::{Position, Rect};
use ratatui_core::text::{Line, Span};

use crate::app::model::{ChatAnchor, ChatScroll};
use crate::app::select;
use crate::console::input::View;
use crate::console::lane::Kind;
use crate::console::target;
use crate::console::view;
use crate::theme::Mark;

use super::geometry::{Stand, stand};
use super::header::header;
use super::layout::{Hit, Layout};
use super::overlay::{overlay_open, tail};
use super::paint::{Paint, cells, put, put_line};
use super::{Composer, Ctx};

/// The address a draft to `target` is drawn under.
pub(crate) fn address(target: &str) -> String {
    format!("to {target}")
}

fn chat_head(ctx: &Ctx<'_, '_>, buf: &mut Buffer, columns: &Range<u16>, layout: &mut Layout) {
    let (left, end) = (columns.start, columns.end);
    header(ctx, buf, (left, end), layout);
    let room = end - left;
    let rule = ctx.paint.fg(|p| p.border);
    put(buf, left, 1, &"─".repeat(usize::from(room)), room, rule);
}

/// The chat column over `columns`: the header, the chat window, the composer.
pub(super) fn chat_column(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    columns: Range<u16>,
    layout: &mut Layout,
) {
    let (screen, paint) = (ctx.screen, ctx.paint);
    let height = buf.area.height;
    let (left, end) = (columns.start, columns.end);
    let room = end - left;
    let border = paint.fg(|p| p.border);
    chat_head(ctx, buf, &columns, layout);
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
    let room_rows = usize::from(bottom.saturating_sub(top));
    let (tail, window, scroll) = chat_window(ctx, room, room_rows);
    let crate::app::lane::Tail {
        mut rows,
        complete,
        turns: produced,
        links,
        files,
    } = tail;
    let (turns, bars, folds) = place_turns(ctx, &produced, &mut rows, layout);
    let Range {
        start: first,
        end: last,
    } = window;
    let y0 = bottom - cells(last - first);
    for (step, line) in rows[first..last].iter().enumerate() {
        put_line(buf, left, y0 + cells(step), line, room);
    }
    layout.link_runs(links.get(first..last).unwrap_or_default(), (left, y0, room));
    layout.file_runs(
        files.get(first..last).unwrap_or_default(),
        ctx,
        (left, y0, room),
    );
    turn_targets(buf, layout, (&turns, &folds), (first..last, left, y0, room));
    layout.lane(buf, (left, top, room, room_rows), &bars, (first..last, y0));
    let viewing = screen.composer.in_place();
    if room_rows > 0 && last < rows.len() && !viewing {
        newer_hint(
            buf,
            layout,
            (left, bottom, room),
            (&produced, last, paint, ctx.icons),
        );
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
        composer_draft(
            buf,
            layout,
            view,
            Rect::new(left, composer + 1, room, grown),
            paint,
        );
    } else {
        put_line(buf, left, composer + 1, &input_row, room);
    }
    if !viewing {
        layout.record(
            buf,
            Rect::new(left, composer, room, grown + 2),
            Hit::Compose,
        );
    }
    layout.page_rows = room_rows;
    layout.max_scroll = rows.len().saturating_sub(room_rows);
    layout.complete = complete;
    layout.scroll = scroll;
}

/// Wrap the tail through the previous anchor, choose the rows to show, and
/// record where the resulting window starts for the next frame.
fn chat_window(
    ctx: &Ctx<'_, '_>,
    room: u16,
    room_rows: usize,
) -> (crate::app::lane::Tail, Range<usize>, Option<ChatScroll>) {
    let screen = ctx.screen;
    let main = screen
        .selected
        .and_then(|entry| entry.main_agent.as_deref())
        .or_else(|| match screen.composer {
            Composer::NoHome => None,
            _ => screen.pair.first().map(String::as_str),
        })
        .unwrap_or_default();
    let clock = view::Style::resolve(true, screen.look, screen.zone, main);
    let (scroll, prior) = screen.model.requested_scroll(room_rows);
    let anchor = prior.filter(|_| scroll > 0).and_then(|prior| {
        let first = prior.first?;
        let at = ctx.wait.folds.iter().position(|&key| key == first.turn)?;
        Some((at, first.row, prior.back))
    });
    // One page and a notch past the window, so a wheel notch or a page key
    // stays inside what this frame produced until the next produces more.
    let need = scroll
        .saturating_add(room_rows.saturating_mul(2))
        .saturating_add(crate::app::model::WHEEL_ROWS);
    let through = anchor.map(|(at, _, back)| {
        let margin = room_rows + crate::app::model::WHEEL_ROWS;
        (at, margin.saturating_add(scroll.saturating_sub(back)))
    });
    let mut tail = body_tail(ctx, &clock, main, (room, need), through);
    if ctx.wait.lane {
        let dim = ctx.paint.fg(|p| p.dim);
        tail.rows.push(Line::from(Span::styled("loading", dim)));
        tail.links.push(Vec::new());
        tail.files.push(Vec::new());
    }
    let pinned = anchor.and_then(|(at, offset, back)| {
        let turn = tail.turns.iter().find(|turn| turn.at == at)?;
        let end = turn.first + turn.rows.saturating_sub(1);
        let top = turn.first.saturating_add_signed(offset).min(end);
        // Apply only movement the human requested since the anchor was drawn.
        Some(top.saturating_add(room_rows + back).saturating_sub(scroll))
    });
    let total = tail.rows.len();
    let last = pinned
        .unwrap_or_else(|| total.saturating_sub(scroll))
        .clamp(room_rows.min(total), total);
    let first = last.saturating_sub(room_rows);
    let back = total - last;
    let drawn = (room_rows > 0).then(|| {
        let first = (back > 0).then(|| {
            let turn = tail
                .turns
                .iter()
                .find(|turn| turn.first + turn.rows > first)?;
            Some(ChatAnchor {
                turn: *ctx.wait.folds.get(turn.at)?,
                row: first.checked_signed_diff(turn.first)?,
            })
        });
        ChatScroll {
            first: first.flatten(),
            back,
            page_rows: room_rows,
        }
    });
    (tail, first..last, drawn)
}

/// Draw and expose only the draft rows between the address and the hint.
fn composer_draft(
    buf: &mut Buffer,
    layout: &mut Layout,
    view: &View,
    mut area: Rect,
    paint: Paint,
) {
    let (left, first, room) = (area.x, area.y, area.width);
    let height = buf.area.height;
    area.height = area.height.min(height.saturating_sub(first + 1));
    let text = paint.fg(|p| p.text);
    layout.selectable(buf, select::Panel::Composer, area);
    for (index, (y, row)) in (first..area.bottom()).zip(&view.rows).enumerate() {
        put(buf, left, y, row, room, text);
        if !view.text_rows.contains(&index) {
            layout
                .map
                .blanks
                .push(Rect::new(left, y, room, 1).intersection(buf.area));
        }
    }
    let before = cells(Span::raw(view.before.as_str()).width());
    layout.cursor = (room > 0)
        .then(|| Position::new(left + before.min(room - 1), first + cells(view.cursor_row)));
}

/// The hint on row `y`, under a scrolled-up lane whose column is `room` cells
/// from `left`: how many of the produced `turns` start at or below row `last`,
/// centered, in the accent. Its words are a click target that scrolls to the
/// newest turn.
fn newer_hint(
    buf: &mut Buffer,
    layout: &mut Layout,
    (left, y, room): (u16, u16, u16),
    (turns, last, paint, icons): (&[crate::app::lane::Turn], usize, Paint, bool),
) {
    // Every row below the window is produced, so the count is exact.
    let below = turns.iter().filter(|turn| turn.first >= last).count();
    let (arrow, divider) = if icons { ("↓", "·") } else { ("v", "-") };
    let words = match below {
        0 => format!("{arrow} more below {divider} End"),
        1 => format!("{arrow} 1 newer turn {divider} End"),
        count => format!("{arrow} {count} newer turns {divider} End"),
    };
    let width = cells(Span::raw(words.as_str()).width()).min(room);
    let from = left + room.saturating_sub(width) / 2;
    let end = put(buf, from, y, &words, room, paint.fg(|p| p.title));
    layout.overlay(buf, Rect::new(from, y, end - from, 1), Hit::Newest);
}

/// The hint row, the pane's last: a pending line — an armed key, a refused
/// paste, an ask on its way — in full ink so it is not missed, else the
/// composer's hint; then the keys that work in this mode, when they fit whole;
/// at the right end `ae <version>` and the gear.
pub(super) fn hint_row(
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
    let keys = if ctx.wait.confirming {
        ""
    } else {
        mode_hint(&ctx.screen.composer)
    };
    let rest_of = |keys: &str| match (first.is_empty(), keys.is_empty()) {
        (_, true) => String::new(),
        (true, false) => keys.to_owned(),
        (false, false) => format!(" · {keys}"),
    };
    let fits = |rest: &str| cells(Span::raw(rest).width()) <= end.saturating_sub(at);
    // The clear key is named only over text, and only where it fits whole.
    let text = matches!(&ctx.screen.composer, Composer::Home { view: Some(_), draft, .. } if !draft.is_empty());
    let long = rest_of(&format!("{keys} · ^C clears"));
    let rest = if text && !keys.is_empty() && fits(&long) {
        long
    } else {
        rest_of(keys)
    };
    if fits(&rest) {
        at = put(buf, at, y, &rest, end - at, paint.fg(|p| p.dim));
    }
    // A panel's footer is its own row: no second gear under it. The open
    // orchestrator panel draws the frame's tail, and the main column none.
    let model = ctx.screen.model;
    let columns = model.columns();
    let side = matches!(stand(buf.area, model.split(), columns), Stand::Side { .. });
    let panel = columns.drawing_panel;
    let ends = panel || !side;
    if ends && (!overlay_open(model) || panel && !model.settings_open()) {
        tail(ctx, buf, layout, (y, at), false);
    }
    // The orchestrator button stands in the gutter left of the hint: the
    // main column's, or a full panel's that covers it.
    if (!panel || !side) && !overlay_open(model) {
        orchestrator_button(ctx, buf, layout, (left.saturating_sub(2), y));
    }
}

/// The orchestrator panel's button at `(x, y)`: `◆` (`O` with `icons = off`),
/// in the needs-you ink while the orchestrator waits on the human, else dim. A
/// click on it or the blank after it opens or hides the panel.
fn orchestrator_button(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    layout: &mut Layout,
    (x, y): (u16, u16),
) {
    let button = if ctx.icons { "\u{25c6}" } else { "O" };
    let ink = if ctx.screen.model.columns().needy {
        ctx.paint.mark(Mark::NeedsYou)
    } else {
        ctx.paint.fg(|p| p.dim)
    };
    put(buf, x, y, button, 1, ink);
    layout.record(buf, Rect::new(x, y, 2, 1), Hit::Orchestrator);
}

/// The keys that work in this mode: writing names the three that matter,
/// held takes every key and says nothing.
fn mode_hint(composer: &Composer<'_>) -> &'static str {
    match composer {
        Composer::Home { view: Some(_), .. } => "Enter sends · Alt+Enter ^J newline · Esc browses",
        Composer::Held { .. } => "",
        _ => BROWSE_HINT,
    }
}

/// The browse hint: the keys that work while no draft is written.
const BROWSE_HINT: &str = "? keys";

/// The lane's newest rows as the app's own state shows them: `size` is the
/// column's width and how many rows are wanted.
/// The chat body's rows: a status column's line, else the lane's tail.
fn body_tail(
    ctx: &Ctx<'_, '_>,
    clock: &view::Style,
    main: &str,
    size: (u16, usize),
    through: Option<(usize, usize)>,
) -> crate::app::lane::Tail {
    match status_line(ctx) {
        Some(why) => status_tail(ctx, why, size.0),
        None => lane_tail(ctx, clock, main, size, through),
    }
}

/// The line an orchestrator panel bound to no running seat shows: the panel
/// draws no session then, and its read-only reason is its whole body.
fn status_line<'a>(ctx: &Ctx<'a, '_>) -> Option<&'a str> {
    let screen = ctx.screen;
    match screen.composer {
        Composer::ReadOnly { why }
            if screen.model.columns().drawing_panel && screen.selected.is_none() =>
        {
            Some(why)
        }
        _ => None,
    }
}

/// The body of a status column: its line `why`, wrapped to `room` cells, in
/// the text ink, with no turn, link or file.
fn status_tail(ctx: &Ctx<'_, '_>, why: &str, room: u16) -> crate::app::lane::Tail {
    let ink = ctx.paint.fg(|p| p.text);
    let text = crate::board::terminal_text(why);
    let rows: Vec<Line<'static>> = crate::console::wrap::wrap_in(&text, usize::from(room), 0, 0)
        .into_iter()
        .map(|(lead, piece)| Line::from(Span::styled(format!("{}{piece}", " ".repeat(lead)), ink)))
        .collect();
    let count = rows.len();
    crate::app::lane::Tail {
        rows,
        complete: true,
        turns: Vec::new(),
        links: vec![Vec::new(); count],
        files: vec![Vec::new(); count],
    }
}

fn lane_tail(
    ctx: &Ctx<'_, '_>,
    clock: &view::Style,
    main: &str,
    size: (u16, usize),
    through: Option<(usize, usize)>,
) -> crate::app::lane::Tail {
    // The viewer's day now, in the zone the turn times wear.
    let (today, _) =
        crate::board::clock_text(clock.shift(ctx.screen.now.epoch().saturating_mul(1_000_000)));
    let show = crate::app::lane::Show {
        clock,
        main,
        open: (!ctx.wait.open.is_empty()).then_some(ctx.wait.open),
        sent: ctx.wait.sent,
        icons: ctx.icons,
        base: &ctx.screen.overview.base,
        today: Some(&today),
    };
    let (room, need) = size;
    let lane = ctx.screen.lane;
    let shown = (&show, ctx.wait.md);
    crate::app::lane::tail_through(
        lane,
        usize::from(room),
        ctx.paint,
        shown,
        need,
        through,
        None,
    )
}

/// A turn's fingerprint and the rows it spans.
type TurnSpan = (u64, Range<usize>);

/// The produced turns with their fingerprints and row spans, oldest first, the
/// bars of the marked one and each fold row as its fold key and row. The
/// marked turn is recoloured in place, and the frame keeps each turn's place
/// and fold for the keys.
fn place_turns(
    ctx: &Ctx<'_, '_>,
    turns: &[crate::app::lane::Turn],
    rows: &mut [Line<'static>],
    layout: &mut Layout,
) -> (Vec<TurnSpan>, Vec<usize>, Vec<(u64, usize)>) {
    // A target switch is visible in the lane but leaves the latest turn as
    // the first stop for [ and ].
    let keyed: Vec<_> = (turns.iter())
        .filter(|turn| {
            !ctx.screen.lane.items.get(turn.at).is_some_and(|item| {
                item.record.is_none()
                    && matches!(&item.kind, Kind::Said { who } if who == "ae")
                    && item.body.starts_with(crate::app::TARGET_NOTICE)
            })
        })
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
            let barred = crate::app::lane::mark_rows(&mut rows[span.clone()], ctx.paint, ctx.icons);
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
    (turns, folds): (&[TurnSpan], &[(u64, usize)]),
    (window, left, y0, room): (Range<usize>, u16, u16, u16),
) {
    // A fold row is recorded before its turn: the first target to hold a cell wins.
    for (key, row) in folds {
        if window.contains(row) {
            let rect = Rect::new(left, y0 + cells(*row - window.start), room, 1);
            layout.record(buf, rect, Hit::Fold(*key));
        }
    }
    for (key, span) in turns {
        let (from, to) = (span.start.max(window.start), span.end.min(window.end));
        if from < to {
            let y = y0 + cells(from - window.start);
            let rect = Rect::new(left, y, room, cells(to - from));
            layout.record(buf, rect, Hit::Turn(*key));
        }
    }
}

/// The composer's address row, the input row under it and the hint under that.
pub(super) fn composer_lines(ctx: &Ctx<'_, '_>) -> (Line<'static>, Line<'static>, String) {
    let paint = ctx.paint;
    let (text, dim) = (paint.fg(|p| p.text), paint.fg(|p| p.dim));
    let caption = || {
        ctx.wait.target.map(|(target, look)| {
            let style = Paint::of(look).fg(|palette| {
                if target.orchestrator() {
                    palette.working
                } else {
                    palette.title
                }
            });
            Span::styled(target::label(target), style)
        })
    };
    let quiet = |words: String| Line::from(Span::styled(words, dim));
    match ctx.screen.composer {
        Composer::Home { home, view, draft } => {
            let mut address = vec![
                caption().unwrap_or_else(|| Span::styled(address(home), paint.fg(|p| p.title))),
            ];
            if let Some(dest) = ctx.wait.dest {
                let how = if dest.named { "named" } else { "auto" };
                let seat = crate::board::terminal_text(dest.seat);
                address.push(Span::styled(format!(" · {seat} ({how})"), dim));
            }
            let address = Line::from(address);
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
        // A status column's reason is its body: the row says only read-only.
        Composer::ReadOnly { .. } if status_line(ctx).is_some() => (
            quiet("read-only".to_owned()),
            Line::default(),
            String::new(),
        ),
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
        Composer::Viewing { seat } => (
            Line::from(Span::styled(
                format!(
                    "viewing {} · read-only · Esc chat",
                    crate::board::terminal_text(seat)
                ),
                dim,
            )),
            Line::default(),
            "Esc or click the session for its chat".to_owned(),
        ),
        Composer::Reading { path } => (
            Line::from(Span::styled(
                format!(
                    "reading {} · read-only · Esc lane",
                    crate::board::terminal_text(path)
                ),
                dim,
            )),
            Line::default(),
            "Esc or click the session for its chat".to_owned(),
        ),
        Composer::NoHome => (
            quiet("read-only · no home session: ae app <session> picks one".to_owned()),
            Line::default(),
            String::new(),
        ),
    }
}
