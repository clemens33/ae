//! The chat column's header: the selection's name, its reason, and the panel
//! and zoom controls.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::{Line, Span};

use crate::brief::age;
use crate::digest::Status;
use crate::event_text::clip_tail;
use crate::theme::Mark;

use super::geometry::{Stand, stand};
use super::layout::{Hit, Layout};
use super::paint::{aged, cells, put, put_line};
use super::{Composer, Ctx};

/// The chat header: the session, its pair, and right-aligned its branch (at
/// 140 columns or more) and its activity.
pub(super) fn header(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    (left, end): (u16, u16),
    layout: &mut Layout,
) {
    let screen = ctx.screen;
    let paint = ctx.paint;
    let end = panel_head(ctx, buf, (left, end), layout);
    let end = zoom_head(ctx, buf, (left, end), layout);
    let Some(entry) = screen.selected else {
        // A panel bound to no running seat names the seat and nothing else:
        // no state, no age, no pair.
        if screen.model.columns().drawing_panel {
            let name = paint.fg(|p| p.title).add_modifier(Modifier::BOLD);
            let seat = crate::orchestrator::ORCHESTRATOR_SESSION;
            put(buf, left, 0, seat, end.saturating_sub(left), name);
        }
        return;
    };
    // With the panel open, the column that takes the keys is named in the
    // title ink and the other dim; with no colour a `▸` marks it.
    let columns = screen.model.columns();
    let focused = columns
        .panel
        .map(|(_, panel)| panel == columns.drawing_panel);
    let ink = match focused {
        Some(false) => paint.fg(|p| p.dim),
        _ => paint.fg(|p| p.title),
    };
    let name = match paint.0 {
        Some(_) => ink.add_modifier(Modifier::BOLD),
        None => Style::new().add_modifier(Modifier::BOLD),
    };
    if focused == Some(true) && paint.0.is_none() {
        put(buf, left.saturating_sub(2), 0, "▸", 1, name);
    }
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
    let width = |right: &str| cells(right.chars().count());
    let rights = branch.iter().chain(std::iter::once(&activity));
    let fits: Vec<&String> = rights
        .filter(|right| after + 2 + width(right) <= end)
        .collect();
    let right = (fits.iter().copied())
        .find(|right| end.saturating_sub(width(right) + 2) >= wanted)
        .or_else(|| fits.first().copied());
    // The orchestrator's column names the work of the whole fleet in the longest
    // of its steps that fits in the room the names and the right-hand labels
    // leave: the branch gives way to a longer step, the activity word does not.
    // A header too narrow for the shortest step shows none, and every other
    // column names what waits on the human: the seat's ask and its age, cut to
    // what the room leaves, which it needs a dozen cells to read.
    let segment = ctx
        .wait
        .work
        .and_then(|work| work_step(&work, ctx.icons, &fits, (from, end)));
    let right = segment.as_ref().map_or(right, |(_, right)| Some(*right));
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
    let lead = match ctx.wait.work {
        Some(_) => segment.map(|(step, _)| step),
        None => reason_words(ctx, room)
            .map(|words| vec![(words, crate::app::fleet::Ink::Mark(Mark::NeedsYou))]),
    };
    let dir_from = lead.as_ref().map_or(from, |step| {
        let used = put_step(ctx, buf, from, room, step);
        used + 3
    });
    let room = stop.saturating_sub(dir_from);
    if lead.is_some() && room < REASON_MIN {
        return;
    }
    let shown = clip_tail(dir, usize::from(room));
    put(buf, dir_from, 0, &shown, room, paint.fg(|p| p.dim));
}

/// The longest step of the fleet's work segment that fits the header columns
/// `from..end` beside one of the right-hand labels, richest first, and that
/// label. The activity word stays: a step that fits only beside a shorter
/// label takes the room of the branch, never the activity's.
fn work_step<'a>(
    work: &crate::app::fleet::Work,
    icons: bool,
    rights: &[&'a String],
    (from, end): (u16, u16),
) -> Option<(crate::app::fleet::Step, &'a String)> {
    work.steps(icons).into_iter().find_map(|step| {
        let need = cells(crate::app::fleet::flat(&step).chars().count());
        (rights.iter().copied())
            .find(|right| {
                let stop = end.saturating_sub(cells(right.chars().count()) + 2);
                need <= stop.saturating_sub(from)
            })
            .map(|right| (step, right))
    })
}

/// Draws `step` from column `x` in its inks and answers the column after it.
fn put_step(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    x: u16,
    room: u16,
    step: &crate::app::fleet::Step,
) -> u16 {
    use crate::app::fleet::Ink;
    let paint = ctx.paint;
    let needs = paint.fg(|p| p.needs_you);
    let spans: Vec<Span<'static>> = step
        .iter()
        .map(|(text, ink)| {
            let style = match ink {
                Ink::Mark(mark) => paint.mark(*mark),
                Ink::Text => paint.fg(|p| p.text),
                Ink::Dim => paint.fg(|p| p.dim),
                Ink::Needs => needs,
            };
            Span::styled(text.clone(), style)
        })
        .collect();
    put_line(buf, x, 0, &Line::from(spans), room)
}

/// The main column header's right end: the zoom label, `X zoom` or `X unzoom`,
/// a click target. The answer is where the rest of the header ends; the
/// panel's column keeps `end`, its own label being [`panel_head`]'s.
fn zoom_head(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    (left, end): (u16, u16),
    layout: &mut Layout,
) -> u16 {
    let model = ctx.screen.model;
    if model.columns().drawing_panel {
        return end;
    }
    let label = if model.columns().zoomed() {
        "X unzoom"
    } else {
        "X zoom"
    };
    let width = cells(label.chars().count());
    if end < left + width {
        return end;
    }
    let from = end - width;
    put(buf, from, 0, label, width, ctx.paint.fg(|p| p.title));
    layout.record(buf, Rect::new(from, 0, width, 1), Hit::Zoom);
    from.saturating_sub(2).max(left)
}

/// The word of the panel header's pane target, plain whatever the icons.
const PANE_WORD: &str = "pane";

/// The panel header's mic, drawn with icons on.
const MIC_ICON: &str = "🎤";

/// The panel header's mic, drawn with icons off.
const MIC_WORD: &str = "mic";

/// The panel header's right end: the mode label, `F full` or `F side`, a
/// click target, before it `pane` while the panel is bound to a running seat,
/// and before that why a side panel did not fit. The answer is
/// where the rest of the header ends; the main column's header keeps `end`.
fn panel_head(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    (left, end): (u16, u16),
    layout: &mut Layout,
) -> u16 {
    let model = ctx.screen.model;
    let columns = model.columns();
    if !columns.drawing_panel {
        return end;
    }
    let stand = stand(buf.area, model.split(), columns);
    // The label names the human's zoom, never the layout's fallback.
    let label = if matches!(stand, Stand::Full { note: None }) {
        "F side"
    } else {
        "F full"
    };
    let width = cells(label.chars().count());
    if end < left + width {
        return end;
    }
    let from = end - width;
    put(buf, from, 0, label, width, ctx.paint.fg(|p| p.title));
    layout.record(buf, Rect::new(from, 0, width, 1), Hit::PanelMode);
    let mut stop = from.saturating_sub(2);
    // A panel bound to a running seat offers its pane; a status column draws
    // no session and so no target.
    let pane = cells(PANE_WORD.chars().count());
    if ctx.screen.selected.is_some() && stop >= left + pane {
        let at = stop - pane;
        put(buf, at, 0, PANE_WORD, pane, ctx.paint.fg(|p| p.title));
        layout.record(buf, Rect::new(at, 0, pane, 1), Hit::PanelPane);
        stop = at.saturating_sub(2);
        // The mic beside it, once a voice read of that seat has answered.
        if let Some(mic) = ctx.wait.mic {
            let glyph = if ctx.icons { MIC_ICON } else { MIC_WORD };
            let text = format!("{glyph} {}", mic.word());
            let need = cells(Span::raw(text.as_str()).width());
            if stop >= left + need {
                let at = stop - need;
                put(buf, at, 0, &text, need, ctx.paint.fg(|p| p.title));
                layout.record(buf, Rect::new(at, 0, need, 1), Hit::PanelMic);
                stop = at.saturating_sub(2);
            }
        }
    }
    if let Stand::Full { note: Some(note) } = stand {
        let need = cells(note.chars().count());
        if stop >= left + need {
            put(buf, stop - need, 0, &note, need, ctx.paint.fg(|p| p.dim));
            stop = stop.saturating_sub(need + 2);
        }
    }
    stop.max(left)
}

/// The fewest cells the header's reason or work segment, or a directory beside
/// it, is worth drawing in.
const REASON_MIN: u16 = 12;

/// The open ask the header names, as `⚠ <seat>: <words> · <age>` in `room`
/// cells: the seat the chat column views, else the one the cursor stands on,
/// else the first that waits. `None` when nothing waits or the room is under
/// [`REASON_MIN`].
pub(super) fn reason_words(ctx: &Ctx<'_, '_>, room: u16) -> Option<String> {
    let screen = ctx.screen;
    let viewed = match screen.composer {
        Composer::Viewing { seat } => Some(seat),
        _ => None,
    };
    let open = &screen.overview.open;
    let ask = [viewed, screen.model.focus()]
        .into_iter()
        .flatten()
        .find_map(|seat| open.iter().find(|ask| ask.seat == seat))
        .or(open.first())?;
    let glyph = Mark::NeedsYou.glyph(ctx.icons);
    let lead = format!("{glyph} ");
    let room = room.checked_sub(cells(lead.chars().count()))?;
    if room < REASON_MIN {
        return None;
    }
    let words = format!("{}: {}", ask.seat, ask.text);
    Some(format!("{lead}{}", aged(&words, ask.age_secs, room)))
}
