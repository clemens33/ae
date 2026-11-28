//! What every screen draws with: the look's colours, the bounded cell writes
//! and a text's fit in cells.

use ratatui_core::buffer::{Buffer, CellWidth};
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier, Style};
use ratatui_core::text::{Line, Span};

use crate::brief::age;
use crate::theme::{Look, Mark, Palette};

/// The sidebar's first content column.
pub(super) const LEFT: u16 = 2;

/// The colours one frame draws in: a palette, or none.
#[derive(Debug, Clone, Copy)]
pub(in crate::l5_ui::app) struct Paint(pub(super) Option<Palette>);

impl Paint {
    pub(in crate::l5_ui::app) fn of(look: Option<&Look>) -> Self {
        Self(look.filter(|look| look.drawn).map(|look| look.palette))
    }

    /// Whether a look is drawn at all: colour and emphasis are worn only then.
    pub(in crate::l5_ui::app) fn drawn(self) -> bool {
        self.0.is_some()
    }

    /// A foreground from the palette, or none.
    pub(in crate::l5_ui::app) fn fg(self, token: impl Fn(&Palette) -> &'static str) -> Style {
        self.0.map_or_else(Style::new, |palette| {
            Style::new().fg(colour(token(&palette)))
        })
    }

    /// A ground from the palette, or none.
    pub(in crate::l5_ui::app) fn ground(self, token: impl Fn(&Palette) -> &'static str) -> Style {
        self.0.map_or_else(Style::new, |palette| {
            Style::new().bg(colour(token(&palette)))
        })
    }

    /// A mark's accent.
    pub(super) fn mark(self, mark: Mark) -> Style {
        self.fg(|palette| palette.accent(mark))
    }

    /// A section header's `area`: the raised panel ground behind its words, so
    /// the header reads apart from the rows under it; no ground with no colour.
    pub(super) fn band(self, buf: &mut Buffer, area: Rect) {
        if self.0.is_some() {
            buf.set_style(area, self.ground(|palette| palette.panel));
        }
    }

    /// The selected row `area`, a session's two lines or a seat's: the selected
    /// ground, the dim words in the selected ink so only it and the accents sit
    /// on it; else reverse video.
    pub(super) fn lift(self, buf: &mut Buffer, area: Rect) {
        let Some(palette) = self.0 else {
            buf.set_style(area, Style::new().add_modifier(Modifier::REVERSED));
            return;
        };
        buf.set_style(area, Style::new().bg(colour(palette.selected)));
        let (dim, ink) = (colour(palette.dim), colour(palette.selected_ink));
        for position in area.intersection(buf.area).positions() {
            let cell = &mut buf[position];
            if cell.fg == dim {
                cell.fg = ink;
            }
        }
    }

    /// The selected name: the title hue, or bold and reverse with no colour.
    pub(in crate::l5_ui::app) fn selected(self) -> Style {
        match self.0 {
            Some(_) => self.fg(|p| p.title).add_modifier(Modifier::BOLD),
            None => Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        }
    }

    /// A border's rule: the border hue, or while it is dragged the title hue
    /// in bold, or reversed with no colour.
    pub(super) fn rule(self, dragged: bool) -> Style {
        match (dragged, self.0) {
            (false, _) => self.fg(|p| p.border),
            (true, Some(_)) => self.fg(|p| p.title).add_modifier(Modifier::BOLD),
            (true, None) => Style::new().add_modifier(Modifier::REVERSED),
        }
    }
}

pub(super) fn colour(token: &str) -> Color {
    let [red, green, blue] = crate::theme::rgb(token);
    Color::Rgb(red, green, blue)
}

/// Where each grapheme of `text` that the buffer draws ends in `text`, with the
/// cells it spends. The buffer segments the whole string and then skips what
/// spends no cell, so a cut must be a prefix of `text`, never a rejoining.
fn drawn(text: &str) -> Vec<(usize, usize)> {
    let spent = |symbol: &str| usize::from(symbol.cell_width());
    let end = |symbol: &str| symbol.as_ptr() as usize - text.as_ptr() as usize + symbol.len();
    Span::raw(text)
        .styled_graphemes(Style::default())
        .map(|grapheme| (end(grapheme.symbol), spent(grapheme.symbol)))
        .collect()
}

/// The cells `drawn` spends on `text`.
pub(super) fn drawn_cols(text: &str) -> u16 {
    cells(drawn(text).iter().map(|(_, width)| width).sum())
}

/// `text` then `tail` in `room` cells: whole, else the text alone, else its
/// head cut at a grapheme and closed on `…`.
pub(super) fn fit(text: &str, tail: &str, room: u16) -> String {
    let (whole, room) = (usize::from(drawn_cols(text)), usize::from(room));
    if room == 0 {
        return String::new();
    }
    if whole + tail.chars().count() <= room {
        return format!("{text}{tail}");
    }
    if whole <= room {
        return text.to_owned();
    }
    let (mut used, mut end) = (1, 0);
    for (stop, width) in drawn(text) {
        if used + width > room {
            break;
        }
        used += width;
        end = stop;
    }
    format!("{}…", &text[..end])
}

/// `text · <age>` in `room` cells: a clipped text keeps its age.
pub(super) fn aged(text: &str, since: Option<i64>, room: u16) -> String {
    aged_by(text, &age(since), room)
}

/// [`aged`] with the age already in words.
pub(super) fn aged_by(text: &str, words: &str, room: u16) -> String {
    let tail = format!(" · {words}");
    let (whole, tail_cells) = (drawn_cols(text), cells(tail.chars().count()));
    if whole + tail_cells <= room {
        return format!("{text}{tail}");
    }
    // A whole text beats its age; a text clipped anyway keeps its age (F7),
    // while one cell and its ellipsis still fit beside it.
    if whole <= room {
        return text.to_owned();
    }
    match room.checked_sub(tail_cells) {
        Some(left) if left >= 2 => format!("{}{tail}", fit(text, "", left)),
        _ => fit(text, "", room),
    }
}

/// A count of cells as a coordinate.
pub(in crate::l5_ui::app) fn cells(count: usize) -> u16 {
    u16::try_from(count).unwrap_or(u16::MAX)
}

/// Whether (x, y) lies inside `buf`.
fn inside(buf: &Buffer, x: u16, y: u16) -> bool {
    let area = buf.area;
    x >= area.left() && x < area.right() && y >= area.top() && y < area.bottom()
}

/// Write `text` at (x, y), at most `max` cells and never past the buffer;
/// answers the column after it.
pub(in crate::l5_ui::app) fn put(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    text: &str,
    max: u16,
    style: Style,
) -> u16 {
    if !inside(buf, x, y) {
        return x;
    }
    buf.set_stringn(x, y, text, usize::from(max), style).0
}

/// [`put`] for a line of styled spans.
pub(super) fn put_line(buf: &mut Buffer, x: u16, y: u16, line: &Line<'_>, max: u16) -> u16 {
    if !inside(buf, x, y) {
        return x;
    }
    buf.set_line(x, y, line, max).0
}
