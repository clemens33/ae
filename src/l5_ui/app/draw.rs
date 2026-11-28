//! The cells: the sidebar, its tabs and the chat column, into one buffer.
//!
//! PURE. Colour is spent on meaning only and comes from the drawn look's
//! palette; `theme = off`, or no look, draws no colour at all and marks the
//! selection with bold and reverse instead. Every write is bounded by the
//! buffer, so a pane of any size draws without a panic.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::fleet::{Facts, Fleet};
use super::model::{Edge, Model};
use super::overview::Overview;
use super::select::Map;
use crate::console::input::View;
use crate::console::lane::Lane;
use crate::console::target::Target;
use crate::digest::SessionEntry;
use crate::theme::Look;
use crate::time::Timestamp;

mod chat;
mod geometry;
mod header;
mod layout;
mod overlay;
mod paint;
mod readers;
mod settings_tabs;
mod sidebar;

mod accounts_summary;
mod stats_body;
mod tool_body;

use chat::chat_column;
use geometry::{MIN, SIDEBAR_MIN, Stand, dragged, rule_line, sidebar_width, stand};
use overlay::{keys_row, overlay_open};
use sidebar::sidebar;

pub(crate) use geometry::{
    column_width, composer_pane, dragged_to, floor_met, panel_area, panel_full, stepped,
};
pub(super) use layout::FileHit;
pub(crate) use layout::{Hit, Layout};
pub(crate) use overlay::paint_settings;
use paint::{LEFT, drawn_cols, fit};
pub(super) use paint::{Paint, cells, put};
pub(super) use readers::markdown_style;
pub use readers::{Bounds, draw_markdown, draw_peek, draw_peek_map, draw_reader};
pub(crate) use readers::{paint_markdown_column, paint_peek_column, paint_reader_column};
pub(crate) use sidebar::list_len;

#[cfg(test)]
use super::fleet::summary;
#[cfg(test)]
use super::model::{SettingsTab, Split};
#[cfg(test)]
use super::settings::SettingsBodies;
#[cfg(test)]
use chat::{composer_lines, hint_row};
#[cfg(test)]
pub(crate) use geometry::draft_width;
#[cfg(test)]
use geometry::{LANE_FLOOR, list_rows};
#[cfg(test)]
use header::reason_words;
#[cfg(test)]
use paint::{aged, colour, put_line};
#[cfg(test)]
use readers::paint_peek;
#[cfg(test)]
use settings_tabs::{instructions_body, keys_body, marks_legend, settings_body};
#[cfg(test)]
use sidebar::NAME;

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
        /// `Some` while composing.
        view: Option<&'a View>,
        draft: &'a str,
    },
    /// The selected session, not writable.
    ReadOnly { why: &'a str },
    /// The selected session, its entry refused: HELD until Esc, ^C, a click
    /// or an Enter that tries again.
    Held { why: &'a str },
    /// A seat's live screen is shown in place of the chat: read-only, and
    /// the row says how the chat comes back.
    Viewing { seat: &'a str },
    /// A file the marked turn links is read in place of the chat: read-only,
    /// and the row says how the chat comes back.
    Reading { path: &'a str },
    /// No session selected.
    NoHome,
}

impl Composer<'_> {
    /// Whether something stands in the chat's place: a seat's screen or a
    /// file read, neither of which the lane's cues or lift apply to.
    fn in_place(&self) -> bool {
        matches!(self, Composer::Viewing { .. } | Composer::Reading { .. })
    }
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
    /// The seat a line written now would reach, for the composer's address.
    pub(crate) dest: Option<Dest<'a>>,
    /// The composer's address and its own drawn look.
    pub(crate) target: Option<(&'a Target, Option<&'a Look>)>,
    /// A warning in the lane takes precedence over the editing shortcuts.
    pub(crate) confirming: bool,
    /// The cache that draws agent turns through Markdown; with none the
    /// lane draws them plain.
    pub(super) md: Option<&'a std::cell::RefCell<super::lanemd::Cache>>,
    /// The work of the whole fleet, drawn in the header of the orchestrator's
    /// column in place of the open ask; with none the header names the ask.
    pub(crate) work: Option<super::fleet::Work>,
    /// The panel header's mic, as the last voice read of the bound seat saw
    /// it; with none, no read has answered and no mic is drawn.
    pub(crate) mic: Option<Mic>,
}

/// What the panel header's mic says about the orchestrator seat's dictation,
/// from the newest fresh read of its pane. No state reads as off: a frame
/// without a voice word proves nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mic {
    /// The seat's tool has no dictation ae can drive.
    Unsupported,
    /// No voice word in the footer, or no footer the read could classify.
    Unknown,
    /// Claude counts Space key events toward a recording, or transcribes.
    Pending,
    /// Claude records.
    Recording,
}

impl Mic {
    /// The word drawn after the mic.
    #[must_use]
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Unsupported => "n/a",
            Self::Unknown => "?",
            Self::Pending => "…",
            Self::Recording => "rec",
        }
    }
}

/// Where the composer's line would land: the seat the router picked, or the
/// one the draft named with `@`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Dest<'a> {
    pub(crate) seat: &'a str,
    /// Whether the draft named it, rather than the router picking it.
    pub(crate) named: bool,
}

impl<'a> Dest<'a> {
    /// The destination a `(seat, named)` pair names.
    pub(crate) fn of((seat, named): &'a (String, bool)) -> Self {
        Self {
            seat,
            named: *named,
        }
    }
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
    let stand = stand(area, screen.model.split(), screen.model.columns());
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
    let columns = screen.model.columns();
    let chat = if columns.zoomed() {
        // Zoomed: the keys-holding column owns the whole frame; the sidebar, its
        // rule, its targets and its fit warning are not drawn.
        0..width
    } else if let Some(rule) = sidebar_width(area, split) {
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
    // A side panel takes the right of the chat; the main column keeps the rest.
    let end = match stand {
        Stand::Side { border, .. } => border,
        _ => width,
    };
    layout.chat = Rect::new(chat.start, 0, end - chat.start, height);
    layout.seats_session = ctx.screen.selected.map(|entry| entry.name.clone());
    chat_column(&ctx, buf, chat.start + 2..end - 2, &mut layout);
    // The warning rides the chat's rule row, over the left of the rule.
    if let Some(line) = fallback {
        put(buf, 0, 1, &line, width, dim);
    }
    if overlay_open(screen.model) {
        keys_row(&ctx, buf, &mut layout);
    }
    layout
}

/// Draw the open orchestrator panel over its cells of the frame `buf` holds:
/// `screen` is the panel's own column, drawn by the chat's renderer. A side
/// panel has its border on its left column; a full one covers the main
/// column, the sidebar's rule being the divider it keeps.
pub(crate) fn draw_panel(screen: &Screen<'_>, wait: Wait<'_>, buf: &mut Buffer) -> Layout {
    let area = buf.area;
    let mut layout = Layout {
        area,
        ..Layout::default()
    };
    let columns = screen.model.columns();
    let stand = stand(area, screen.model.split(), columns);
    if !floor_met(area) || stand == Stand::Closed {
        return layout;
    }
    let ctx = Ctx {
        screen,
        paint: Paint::of(screen.look.as_ref()),
        icons: screen.look.is_none_or(|look| look.icons),
        wait,
    };
    let Some(panel) = panel_area(area, screen.model.split(), columns) else {
        return layout;
    };
    let (height, width, from) = (panel.height, panel.right(), panel.x);
    for y in 0..height {
        for x in from..width {
            buf[(x, y)].reset();
        }
    }
    layout.panel = Some(panel);
    buf.set_style(panel, ctx.paint.ground(|p| p.ink));
    let left = if let Stand::Side { .. } = stand {
        let border = ctx.paint.rule(dragged(screen.model, Edge::Panel));
        rule_line(buf, from, 0..height, border);
        from + 1
    } else {
        from
    };
    layout.chat = Rect::new(left, 0, width - left, height);
    layout.seats_session = ctx.screen.selected.map(|entry| entry.name.clone());
    chat_column(&ctx, buf, left + 2..width - 2, &mut layout);
    layout
}

#[cfg(test)]
mod accounts_spec;

#[cfg(test)]
mod s5_draw;
#[cfg(test)]
mod s5_retention;
#[cfg(test)]
mod seatjoin_spec;
#[cfg(test)]
mod skilltab_spec;
#[cfg(test)]
mod stats3_spec;
#[cfg(test)]
mod stats_spec;

#[cfg(test)]
mod tests;
