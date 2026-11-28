//! The borders: where each stands, dragged or not, and the columns the
//! sidebar, the chat, the composer and the orchestrator panel get.

use std::ops::Range;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Style;

use crate::app::model::{Columns, Drag, Edge, Model, Split};
use crate::console::input::{Mouse, ROWS_MAX};

use super::paint::{cells, put};

/// The smallest pane the app draws in at all.
pub(super) const MIN: (u16, u16) = (40, 8);

/// The smallest pane that keeps the sidebar.
pub(super) const SIDEBAR_MIN: (u16, u16) = (90, 20);

/// The first session row, after the one-row summary and one blank row.
pub(super) const LIST_TOP: u16 = 2;

/// The rows the frame gave back at the top, the blank rows once left between
/// the summary and the list and the band's second row included: `LIST_TOP` and
/// this still make five, so the list keeps the share it had and the tab body
/// takes the row the one-line summary freed.
const TOP_GIVEN: u16 = 3;

/// The narrowest a drag leaves the sidebar: the tab row still keeps a gap
/// between the `Agents NN` heading and its rule, and a name beside its counts.
const SIDEBAR_FLOOR: u16 = 30;

/// The chat columns a drag leaves right of the rule.
const CHAT_FLOOR: u16 = 40;

/// The tab body rows a drag leaves above the floor.
const BODY_FLOOR: u16 = 3;

/// The lane rows a growing composer leaves: one wheel notch.
pub(super) const LANE_FLOOR: u16 = 3;

/// The fewest cells each of the main column and a side panel keeps.
const PANEL_FLOOR: u16 = 40;

/// The column the main column starts at in `area` split as `split`: right of
/// the sidebar's rule, or the frame's first column where no sidebar is drawn.
fn chat_start(area: Rect, split: Split, zoomed: bool) -> u16 {
    if zoomed {
        return 0;
    }
    sidebar_width(area, split).map_or(0, |rule| rule + 1)
}

/// Where the orchestrator panel stands in a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Stand {
    /// No panel drawn: none is open, or a side panel did not fit while the
    /// main column holds the keys and covers the frame.
    Closed,
    /// Right of the main column, from its border column to `end`, the frame's
    /// end.
    Side { border: u16, end: u16 },
    /// Over the main column: right of the sidebar's rule, to the frame's end,
    /// the whole frame where no sidebar is drawn. `note` says why a side
    /// panel did not fit.
    Full { note: Option<String> },
}

/// How wide a side panel stands in `area` split as `split`, its border
/// included, when it ends at the column `end`, the frame's end. It is the
/// width it was dragged to, else half of
/// the chat columns, always leaving both columns [`PANEL_FLOOR`]. `None` when
/// no side panel fits. The main column starts where the whole frame puts it,
/// for the sidebar's width is the frame's to give. The one clamp: a dragged
/// width is kept as asked and clamped here.
fn side_width(area: Rect, end: u16, split: Split) -> Option<u16> {
    // A side panel stands only in a frame that keeps its sidebar.
    let start = chat_start(area, split, false);
    let chat = end.saturating_sub(start);
    let half = chat / 2;
    // Both halves keep the floor, so the clamp's bounds never cross.
    (half >= PANEL_FLOOR)
        .then(|| (split.panel).map_or(half, |asked| asked.clamp(PANEL_FLOOR, chat - PANEL_FLOOR)))
}

/// Where the panel `columns` describe stands in `area` split as `split`:
/// side, [`side_width`] wide, when both columns keep [`PANEL_FLOOR`]. Else
/// the column that holds the keys fills the chat column: the panel, saying why,
/// or the main column, the panel not drawn.
pub(super) fn stand(area: Rect, split: Split, columns: Columns) -> Stand {
    let Some((full, focused)) = columns.panel else {
        return Stand::Closed;
    };
    if full {
        return Stand::Full { note: None };
    }
    if columns.hides_panel() {
        return Stand::Closed;
    }
    let end = area.width;
    if let Some(width) = side_width(area, end, split) {
        return Stand::Side {
            border: end - width,
            end,
        };
    }
    if !focused {
        return Stand::Closed;
    }
    let start = chat_start(area, split, false);
    let need = start.saturating_add(PANEL_FLOOR * 2);
    Stand::Full {
        note: Some(format!("side needs {need} columns")),
    }
}

/// Whether the open panel covers the main column now, the sidebar beside it
/// drawn: asked for, or a side panel that did not fit.
pub(crate) fn panel_full(area: Rect, split: Split, columns: Columns) -> bool {
    matches!(stand(area, split, columns), Stand::Full { .. })
}

/// Whether a frame of `area` meets the app's size floor: below it the floor
/// note stands alone and no column draws.
pub(crate) fn floor_met(area: Rect) -> bool {
    area.width >= MIN.0 && area.height >= MIN.1
}

/// The cells the open panel covers in `area`, `None` while it is closed.
pub(crate) fn panel_area(area: Rect, split: Split, columns: Columns) -> Option<Rect> {
    match stand(area, split, columns) {
        Stand::Closed => None,
        Stand::Side { border, end } => Some(Rect::new(border, 0, end - border, area.height)),
        Stand::Full { .. } => {
            let start = chat_start(area, split, columns.zoomed());
            Some(Rect::new(start, 0, area.width - start, area.height))
        }
    }
}

/// The cells a draft has in the chat column of `area` split as `split`: what
/// the caller wraps it to.
#[cfg(test)]
pub(crate) fn draft_width(area: Rect, split: Split) -> usize {
    draft_room(area, chat_start(area, split, false))
}

/// The cells a draft has in a chat column that starts at the column `start`.
fn draft_room(area: Rect, start: u16) -> usize {
    let left = start + 2;
    usize::from(area.width.saturating_sub(2).saturating_sub(left)).max(1)
}

/// [`draft_width`] for the column `columns` draws now: the main column beside
/// an open side panel keeps its half, the panel its own.
pub(crate) fn column_width(area: Rect, split: Split, columns: Columns) -> usize {
    let width = match stand(area, split, columns) {
        Stand::Side { border, end } if columns.drawing_panel => end - border - 1,
        Stand::Side { border, .. } => border - chat_start(area, split, false),
        Stand::Full { .. } if columns.drawing_panel => {
            panel_area(area, split, columns).map_or(0, |panel| panel.width)
        }
        Stand::Closed | Stand::Full { .. } => {
            return draft_room(area, chat_start(area, split, columns.zoomed()));
        }
    };
    usize::from(width.saturating_sub(4)).max(1)
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

/// The sidebar's width, which is where its rule stands, or `None` when the
/// pane is too small to keep it: 44 from 140 wide, else 34, until a drag
/// asks for another, clamped so the chat keeps its columns.
pub(super) fn sidebar_width(area: Rect, split: Split) -> Option<u16> {
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
pub(super) fn list_rows(area: Rect, split: Split, count: usize) -> (u16, bool) {
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
        0 | 1 => 1,
        _ => 2,
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
/// as the pointer has since, clamped to `area` split as `split` is now.
pub(crate) fn dragged_to(drag: Drag, mouse: Mouse, area: Rect, split: Split, count: usize) -> u16 {
    let to = drag_position(drag, mouse);
    match drag.edge {
        Edge::Sidebar => sidebar_clamp(to, area),
        Edge::List => list_clamp(to.saturating_sub(list_border(0)), area, count),
        Edge::Panel => side_width(
            area,
            area.width,
            Split {
                panel: Some(to),
                ..split
            },
        )
        .unwrap_or(to),
    }
}

fn drag_position(drag: Drag, mouse: Mouse) -> u16 {
    let (at, from) = (i32::from(drag.at), i32::from(drag.from));
    let moved = match drag.edge {
        Edge::Sidebar => at + i32::from(mouse.column) - from,
        Edge::List => at + i32::from(mouse.row) - from,
        // This border is left of its column: moving it left widens it.
        Edge::Panel => at + from - i32::from(mouse.column),
    };
    u16::try_from(moved.max(0)).unwrap_or(u16::MAX)
}

/// Whether `edge` is being dragged.
pub(super) fn dragged(model: &Model, edge: Edge) -> bool {
    model.drag().is_some_and(|drag| drag.edge == edge)
}

/// How many cells one `<` or `>` moves the right-hand column's border.
pub(crate) const KEY_STEP: u16 = 4;

/// The rows of a column's rule that draw the grip: the middle ones.
const GRIP_ROWS: u16 = 5;

/// A column's left rule over `rows` of column `x`, in the rule's `style`: the
/// thin line, and on the middle [`GRIP_ROWS`] rows a heavy one, the handle the
/// eye finds. The glyph differs, never the colour, so a frame with no theme
/// shows the grip too.
pub(super) fn rule_line(buf: &mut Buffer, x: u16, rows: Range<u16>, style: Style) {
    let middle = rows.start + (rows.end.saturating_sub(rows.start)) / 2;
    let grip = middle.saturating_sub(GRIP_ROWS / 2)..middle.saturating_add(GRIP_ROWS / 2 + 1);
    for y in rows {
        let glyph = if grip.contains(&y) { "┃" } else { "│" };
        put(buf, x, y, glyph, 1, style);
    }
}

/// What one press of `>` (`grow`) or `<` makes of the right-hand column in
/// `area` split as `split` and stood as `columns`: the side panel. Its edge and
/// the width a step of [`KEY_STEP`] cells asks, clamped as the drag of that
/// edge clamps it; `None` with no side panel, or when the clamp leaves its
/// width as it is.
pub(crate) fn stepped(
    area: Rect,
    split: Split,
    columns: Columns,
    grow: bool,
) -> Option<(Edge, u16)> {
    if !floor_met(area) {
        return None;
    }
    let step = |now: u16| {
        if grow {
            now.saturating_add(KEY_STEP)
        } else {
            now.saturating_sub(KEY_STEP)
        }
    };
    let Stand::Side { border, end } = stand(area, split, columns) else {
        return None;
    };
    let asked = Split {
        panel: Some(step(end - border)),
        ..split
    };
    let size = side_width(area, end, asked)?;
    (size != end - border).then_some((Edge::Panel, size))
}

/// Which sessions the list shows and where it ends.
pub(super) struct List {
    pub(super) visible: Range<usize>,
    /// The `↑ / ↓ N more` row, when sessions are out of view.
    pub(super) more: Option<u16>,
    /// The first row past the list.
    pub(super) end: u16,
}

impl List {
    /// The list for `count` sessions in `budget` rows, one row each: all of
    /// them while they fit, else a window from `top`, or with `selected` kept
    /// in view, under a `more` row. A `pinned` list ends at its budget, else
    /// where its rows do.
    pub(super) fn of(
        count: usize,
        selected: Option<usize>,
        budget: u16,
        pinned: bool,
        top: Option<usize>,
    ) -> Self {
        let budget = usize::from(budget);
        let shown = if count <= budget {
            count
        } else {
            budget.saturating_sub(1)
        };
        let at = selected.unwrap_or(0);
        let start = top
            .unwrap_or_else(|| (at + 1).saturating_sub(shown))
            .min(count - shown);
        let used = cells(shown);
        let more = (shown < count).then_some(LIST_TOP + used);
        let end = if pinned {
            LIST_TOP + cells(budget)
        } else {
            LIST_TOP + used.max(u16::from(count == 0)) + u16::from(more.is_some())
        };
        Self {
            visible: start..start + shown,
            more,
            end,
        }
    }
}
