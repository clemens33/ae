//! The full-pane readers: the peek panel, the file reader and the Markdown
//! reader.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::{Line, Span};

use crate::app::peek;
use crate::app::reader;
use crate::app::select::{self, Map};
use crate::console::wrap::wrap;
use crate::theme::Look;

use super::layout::{Hit, Layout};
use super::overlay::{Panel, Scroll, panel};
use super::paint::{LEFT, Paint, cells, put, put_line};

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
            inner: None,
            close: (PEEK_CLOSE, Hit::PeekClose),
            selected: 0,
            body: &peek_rows(
                view,
                paint,
                usize::from(buf.area.width.saturating_sub(LEFT)) + 1,
            ),
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

/// Paint the seat view into the frame's chat column: the status on the
/// lane's first row, the captured body under it, wrapped to the column.
/// The sidebar and the composer rows the frame drew stand.
pub(crate) fn paint_peek_column(
    view: &peek::View<'_>,
    look: Option<&Look>,
    buf: &mut Buffer,
    layout: &mut Layout,
) -> Bounds {
    let Some(lane) = layout.lane_area() else {
        layout.peek_column(buf, Rect::default());
        return Bounds::default();
    };
    let paint = Paint::of(look);
    for y in lane.y..lane.bottom() {
        for x in lane.x..lane.right() {
            buf[(x, y)].reset();
        }
    }
    buf.set_style(lane, paint.ground(|p| p.ink));
    put(
        buf,
        lane.x,
        lane.y,
        &peek::status(view),
        lane.width,
        paint.fg(|p| p.dim),
    );
    let body = Rect::new(
        lane.x,
        lane.y.saturating_add(1),
        lane.width,
        lane.height.saturating_sub(1),
    );
    let rows = peek_rows(view, paint, usize::from(lane.width) + 1);
    let visible = usize::from(body.height);
    let (start, max_back) = peek::window(rows.len(), visible, view.back);
    for (at, (text, style)) in rows.iter().skip(start).take(visible).enumerate() {
        put(buf, body.x, body.y + cells(at), text, body.width, *style);
    }
    layout.peek_column(buf, body);
    Bounds { visible, max_back }
}

/// Paint the reader into a bare `buf`: the pure draw the acceptance specs
/// reach. The buffer is the whole panel, header row first.
#[must_use]
pub fn draw_reader(view: &reader::View<'_>, look: Option<Look>, buf: &mut Buffer) -> Bounds {
    let area = buf.area;
    paint_reader(view, Paint::of(look.as_ref()), buf, area, None).0
}

/// Markdown has its own draw surface; raw files retain the original contract.
#[must_use]
pub fn draw_markdown(
    view: &crate::app::markdown::View<'_>,
    look: Option<Look>,
    buf: &mut Buffer,
) -> Bounds {
    paint_markdown(view, Paint::of(look.as_ref()), buf, buf.area, 0).0
}

pub(crate) fn paint_markdown_column(
    index: usize,
    view: &crate::app::markdown::View<'_>,
    look: Option<&Look>,
    buf: &mut Buffer,
    layout: &mut Layout,
) -> Bounds {
    let area = layout.lane_area().unwrap_or_default();
    let (bounds, text) = paint_markdown(view, Paint::of(look), buf, area, index);
    layout.peek_column(buf, text);
    bounds
}

#[allow(
    clippy::too_many_lines,
    reason = "bounded panel draw keeps header and body geometry together"
)]
fn paint_markdown(
    view: &crate::app::markdown::View<'_>,
    paint: Paint,
    buf: &mut Buffer,
    area: Rect,
    index: usize,
) -> (Bounds, Rect) {
    use crate::app::markdown::Shown;
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return (Bounds::default(), Rect::default());
    }
    let visible = usize::from(area.height.saturating_sub(1));
    let (total, mode) = match view.shown {
        Shown::Rendered(r) => (r.rows(), "rendered"),
        Shown::Raw(flow) => (flow.rows(), "raw"),
    };
    let (start, max_back) = reader::window(total, visible, view.top);
    let body = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    );
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            buf[(x, y)].reset();
        }
    }
    buf.set_style(area, paint.ground(|p| p.ink));
    let text = match view.shown {
        Shown::Raw(flow) => {
            let raw = reader::View {
                path: view.path,
                line: view.line,
                body: None,
                flow: Some(flow),
                top: view.top,
                links: view.links,
            };
            paint_file_rows(
                &raw,
                flow,
                reader::gutter(view.lines.len()),
                paint,
                buf,
                body,
            )
            .1
        }
        Shown::Rendered(r) => {
            for (step, row) in (start..total).take(visible).enumerate() {
                let Some(text) = r.text(row) else {
                    continue;
                };
                let mut spans = Vec::new();
                let mut from = 0;
                let mut old = r.marks(row, 0);
                for (at, _) in text.char_indices().skip(1) {
                    let mark = r.marks(row, at);
                    if mark != old {
                        spans.push(Span::styled(
                            crate::board::terminal_text(&text[from..at]),
                            crate::app::highlight::fenced(
                                markdown_style(old, paint),
                                old.syn,
                                paint,
                            ),
                        ));
                        from = at;
                        old = mark;
                    }
                }
                spans.push(Span::styled(
                    crate::board::terminal_text(&text[from..]),
                    crate::app::highlight::fenced(markdown_style(old, paint), old.syn, paint),
                ));
                let y = body.y + cells(step);
                if r.marks(row, text.len()).code_block {
                    buf.set_style(
                        Rect::new(body.x, y, body.width, 1),
                        markdown_style(r.marks(row, text.len()), paint),
                    );
                }
                put_line(buf, body.x, y, &Line::from(spans), body.width);
            }
            body
        }
    };
    let suffix = view.line.map(|n| format!(":{n}")).unwrap_or_default();
    let mode = view
        .refusal
        .map_or_else(|| mode.to_owned(), |why| format!("raw: {why}"));
    let first = if total == 0 { 0 } else { start + 1 };
    let last = start.saturating_add(visible).min(total);
    let links_note = read_place(index, view.links);
    let header = format!(
        "Read · {}{suffix} · {mode} · rows {first}-{last}/{total}{links_note}",
        crate::board::terminal_text(view.path)
    );
    put(
        buf,
        area.x,
        area.y,
        &header,
        area.width,
        paint.fg(|p| p.dim),
    );
    (Bounds { visible, max_back }, text)
}

fn read_place(index: usize, count: usize) -> String {
    if count <= 1 {
        return String::new();
    }
    let place = if index == 0 {
        "first".to_owned()
    } else {
        (index + 1).to_string()
    };
    format!(" · {place} of {count} links")
}

pub(in crate::l5_ui::app) fn markdown_style(
    mark: crate::app::markdown::Marks,
    paint: Paint,
) -> Style {
    let mut style = if mark.target || mark.note {
        paint.fg(|p| p.dim)
    } else if mark.heading.is_some() {
        paint.fg(|p| p.title)
    } else {
        paint.fg(|p| p.text)
    };
    if mark.code || mark.code_block {
        style = style.patch(paint.ground(|p| p.panel));
    }
    if mark.heading.is_some() || mark.strong {
        style = style.add_modifier(Modifier::BOLD);
    }
    if mark.emphasis {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if mark.strike {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    style
}

/// Paint the reader into the frame's chat column: its header on the lane's
/// first row, the numbered file under it. The sidebar and the composer rows
/// the frame drew stand, and a drag selects in the file text, never the
/// gutter.
pub(crate) fn paint_reader_column(
    index: usize,
    view: &reader::View<'_>,
    look: Option<&Look>,
    buf: &mut Buffer,
    layout: &mut Layout,
) -> Bounds {
    let Some(lane) = layout.lane_area() else {
        layout.peek_column(buf, Rect::default());
        return Bounds::default();
    };
    let (bounds, text) = paint_reader(view, Paint::of(look), buf, lane, Some(index));
    layout.peek_column(buf, text);
    bounds
}

/// The reader's panel in `area` of `buf`: a dim header row, then the body
/// rows from the view's top, each file line numbered once with its
/// continuation rows hung under it. The target line's number is bold. A
/// refusal or `loading` has no gutter. The bounds, and the cells a drag may
/// select. Every cell of file text and of the path passes the neutraliser.
fn paint_reader(
    view: &reader::View<'_>,
    paint: Paint,
    buf: &mut Buffer,
    area: Rect,
    index: Option<usize>,
) -> (Bounds, Rect) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return (Bounds::default(), Rect::default());
    }
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            buf[(x, y)].reset();
        }
    }
    buf.set_style(area, paint.ground(|p| p.ink));
    let dim = paint.fg(|p| p.dim);
    let visible = usize::from(area.height.saturating_sub(1));
    let shown = view.body.and_then(reader::Body::lines);
    let lines = shown.map(|(lines, _)| lines);
    let gutter = shown.map_or(0, |(_, gutter)| gutter);
    // A text body the caller has not wrapped is wrapped here, at the width
    // the panel leaves it.
    let own;
    let flow = match (lines, view.flow) {
        (Some(_), Some(flow)) => Some(flow),
        (Some(lines), None) => {
            own = reader::flow(lines, usize::from(area.width).saturating_sub(gutter));
            Some(&own)
        }
        (None, _) => None,
    };
    let status = reader::status(
        &reader::View {
            flow,
            links: if index.is_some() { 0 } else { view.links },
            ..*view
        },
        visible,
    );
    let place = index
        .map(|at| read_place(at, view.links))
        .unwrap_or_default();
    let at = view.line.map(|line| format!(":{line}")).unwrap_or_default();
    let header = format!(
        "Read · {}{at} · {status}{place}",
        crate::board::terminal_text(view.path)
    );
    put(buf, area.x, area.y, &header, area.width, dim);
    let body = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    );
    if let (Some(_), Some(flow)) = (lines, flow) {
        return paint_file_rows(view, flow, gutter, paint, buf, body);
    }
    paint_reader_note(view, paint, buf, body)
}

/// The numbered rows of a read file in `body`, from the view's top, each file
/// line's number once with its continuation rows hung under it, the target
/// line's number bold. The bounds, and the cells a drag may select: the text,
/// the gutter not among them.
fn paint_file_rows(
    view: &reader::View<'_>,
    flow: &reader::Flow,
    gutter: usize,
    paint: Paint,
    buf: &mut Buffer,
    body: Rect,
) -> (Bounds, Rect) {
    let (dim, text) = (paint.fg(|p| p.dim), paint.fg(|p| p.text));
    let visible = usize::from(body.height);
    let (start, max_top) = reader::window(flow.rows(), visible, view.top);
    let digits = gutter.saturating_sub(1);
    let text_x = body.x.saturating_add(cells(gutter).min(body.width));
    let room = body.right().saturating_sub(text_x);
    let number = dim.add_modifier(Modifier::BOLD);
    let mut syntax = crate::app::highlight::Reading::open(view, flow, start, paint);
    for (step, row) in (start..flow.rows()).take(visible).enumerate() {
        let Some((line, first, gap, piece)) = flow.display(row) else {
            break;
        };
        let y = body.y.saturating_add(cells(step));
        if first {
            let style = if view.line == Some(line + 1) {
                number
            } else {
                dim
            };
            put(
                buf,
                body.x,
                y,
                &format!("{:>digits$} ", line + 1),
                cells(gutter),
                style,
            );
        }
        if let Some(line) = syntax.as_mut().and_then(|syntax| syntax.row(row)) {
            put_line(buf, text_x, y, &line, room);
            continue;
        }
        put(
            buf,
            text_x,
            y,
            &format!("{}{piece}", " ".repeat(gap)),
            room,
            text,
        );
    }
    (
        Bounds {
            visible,
            max_back: max_top,
        },
        Rect::new(text_x, body.y, room, body.height),
    )
}

/// The reader's `body` when no file text stands there: the refusal's words,
/// wrapped, or one dim `loading`. No gutter; the whole body is selectable.
fn paint_reader_note(
    view: &reader::View<'_>,
    paint: Paint,
    buf: &mut Buffer,
    body: Rect,
) -> (Bounds, Rect) {
    let (dim, text) = (paint.fg(|p| p.dim), paint.fg(|p| p.text));
    let visible = usize::from(body.height);
    let rows: Vec<(String, Style)> = match view.body {
        Some(reader::Body::Gap(why)) => wrap(
            &crate::board::terminal_text(why),
            usize::from(body.width) + 1,
            0,
        )
        .into_iter()
        .map(|(gap, piece)| (format!("{}{piece}", " ".repeat(gap)), text))
        .collect(),
        _ => vec![("loading".to_owned(), dim)],
    };
    let (start, max_top) = reader::window(rows.len(), visible, view.top);
    for (step, (row, style)) in rows.iter().skip(start).take(visible).enumerate() {
        put(
            buf,
            body.x,
            body.y.saturating_add(cells(step)),
            row,
            body.width,
            *style,
        );
    }
    (
        Bounds {
            visible,
            max_back: max_top,
        },
        body,
    )
}

/// The peek body's rows: each captured line neutralised, then wrapped to
/// `room` with its continuation rows hanging two cells; before the first
/// answer one dim `loading`. `wrap` measures from a one-cell bar the
/// painter does not draw, so `room` is the drawn width plus one.
fn peek_rows(view: &peek::View<'_>, paint: Paint, room: usize) -> Vec<(String, Style)> {
    let text = paint.fg(|p| p.text);
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
