//! The cells: the sidebar, its tabs and the chat column, into one buffer.
//!
//! PURE. Colour is spent on meaning only and comes from the drawn look's
//! palette; `theme = off`, or no look, draws no colour at all and marks the
//! selection with bold and reverse instead. Every write is bounded by the
//! buffer, so a pane of any size draws without a panic.

use std::fmt::Write as _;
use std::ops::Range;

use ratatui_core::buffer::{Buffer, CellWidth};
use ratatui_core::layout::{Position, Rect};
use ratatui_core::style::{Color, Modifier, Style};
use ratatui_core::text::{Line, Span};

use super::fleet::{Counts, Facts, Fleet, Line2, Row, summary};
use super::model::{Drag, Edge, Model, SettingsTab, Split, Stopped};
use super::overview::{Open, Overview};
use super::peek;
use super::reader;
use super::select::{self, Map, Region};
use super::settings::{AboutFacts, ConfigView, InstructionsView, SettingsBodies};
use crate::brief::age;
use crate::console::input::View;
use crate::console::input::{Mouse, ROWS_MAX};
use crate::console::lane::Lane;
use crate::console::view;
use crate::console::wrap::wrap;
use crate::digest::{SessionEntry, Status};
use crate::event_text::clip_tail;
use crate::quota::view::Tone;
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

/// The smallest pane the app draws in at all.
const MIN: (u16, u16) = (40, 8);
/// The smallest pane that keeps the sidebar.
const SIDEBAR_MIN: (u16, u16) = (90, 20);
/// The first session row, after the one-row summary and one blank row.
const LIST_TOP: u16 = 2;
/// The rows the frame gave back at the top, the blank rows once left between
/// the summary and the list and the band's second row included: `LIST_TOP` and
/// this still make five, so the list keeps the share it had and the tab body
/// takes the row the one-line summary freed.
const TOP_GIVEN: u16 = 3;
/// The sidebar's first content column.
const LEFT: u16 = 2;
/// Where a session row's name starts: one blank after its mark.
const NAME: u16 = 4;
/// The narrowest a drag leaves the sidebar: the tab row still keeps a gap
/// between the `Agents NN` heading and its rule, and a name beside its counts.
const SIDEBAR_FLOOR: u16 = 30;
/// The chat columns a drag leaves right of the rule.
const CHAT_FLOOR: u16 = 40;
/// The tab body rows a drag leaves above the floor.
const BODY_FLOOR: u16 = 3;
/// The lane rows a growing composer leaves: one wheel notch.
const LANE_FLOOR: u16 = 3;

/// A click target recorded where its cells are drawn.
#[derive(Debug, Clone)]
pub(crate) enum Hit {
    Session(String),
    /// The stopped group's line: opens or closes the group.
    Stopped,
    /// A drawn seat row: moves the highlight to that seat.
    Seat(String),
    Compose,
    /// The keys row's gear: toggles the settings overlay.
    Settings,
    /// A drawn Settings tab title.
    SettingsTab(SettingsTab),
    /// The overlay's drawn close label.
    SettingsClose,
    /// The peek panel's drawn close label.
    PeekClose,
    /// A drawn chat turn, by its fingerprint: marks it, or unmarks it.
    Turn(u64),
    /// The open modal's item or button, by its place in it.
    Item(usize),
    /// The open modal's own cells: a press there is the modal's.
    Modal,
    /// A drawn fold row, by its turn's fold key: folds or unfolds the turn.
    Fold(u64),
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
    /// The turns the frame produced, newest first: fingerprint, the rows
    /// between its last row and the newest row, and its row count.
    pub turns: Vec<(u64, usize, usize)>,
    /// The turns with a fold row: fingerprint, then fold key.
    pub folds: Vec<(u64, u64)>,
    /// The cell the terminal cursor shows: the draft's, while writing.
    pub cursor: Option<Position>,
    /// Body rows the open full-pane panel (settings or peek) shows at once.
    pub settings_page_rows: usize,
    /// How far the open panel's body can scroll.
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
    /// What a drag may select in this frame.
    pub(crate) map: Map,
    /// The seats the tab body drew, in drawn order, and the one it highlighted.
    pub seats: Vec<String>,
    pub focused: Option<String>,
    /// Each card the frame drew, as the app read it then: a menu opened on
    /// this frame acts on that and on nothing a later read changed.
    pub(super) cards: std::collections::BTreeMap<String, super::menu::Card>,
    /// The session the frame drew those seats for: a delayed click replayed
    /// against this frame opens only that session's seat, never a same-named
    /// one of the session selected since.
    pub seats_session: Option<String>,
    /// The links the frame drew, one run per link per row, clipped to the
    /// column: the backend wraps exactly these cells in OSC 8.
    pub(crate) links: Vec<super::backend::Run>,
}

impl Layout {
    /// The runs of the visible lane rows `links`, the first on row `y0`, left
    /// of the column at `left` and `room` cells wide: each link clipped to the
    /// cells the row actually draws.
    fn link_runs(&mut self, links: &[super::lane::Links], (left, y0, room): (u16, u16, u16)) {
        for (step, row) in links.iter().enumerate() {
            for (offset, width, url) in row {
                let start = u16::try_from(*offset).unwrap_or(u16::MAX);
                let wide = u16::try_from(*width).unwrap_or(u16::MAX);
                let width = wide.min(room.saturating_sub(start));
                if width == 0 {
                    continue;
                }
                self.links.push(super::backend::Run {
                    x: left.saturating_add(start),
                    y: y0.saturating_add(cells(step)),
                    width,
                    url: url.clone(),
                });
            }
        }
    }

    fn record(&mut self, buf: &Buffer, area: Rect, target: Hit) {
        self.targets.push((area.intersection(buf.area), target));
    }

    /// Record `target` over `area` ahead of every target under it: an
    /// overlay's cells are its own.
    pub(super) fn overlay(&mut self, buf: &Buffer, area: Rect, target: Hit) {
        self.targets
            .insert(0, (area.intersection(buf.area), target));
    }

    /// The sessions whose cards the frame drew.
    pub(super) fn sessions(&self) -> impl Iterator<Item = &str> {
        self.targets.iter().filter_map(|(_, target)| match target {
            Hit::Session(name) => Some(name.as_str()),
            _ => None,
        })
    }

    /// The frame painted a modal whole.
    pub(super) fn shows_modal(&self) -> bool {
        (self.targets.iter()).any(|(_, target)| matches!(target, Hit::Modal))
    }

    /// Where the frame drew `name`'s card.
    pub(super) fn card_at(&self, name: &str) -> Option<Rect> {
        (self.targets.iter()).find_map(|(area, target)| {
            matches!(target, Hit::Session(at) if at == name).then_some(*area)
        })
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

    /// The chat lane above its last row: the panel footer owns that row, so
    /// a wheel there never reaches the body drawn above it.
    pub(crate) fn in_chat_body(&self, mouse: Mouse) -> bool {
        self.in_chat(mouse) && mouse.row < self.chat.bottom().saturating_sub(1)
    }

    pub(crate) fn in_list(&self, mouse: Mouse) -> bool {
        self.list.contains((mouse.column, mouse.row).into())
    }

    pub(crate) fn in_body(&self, mouse: Mouse) -> bool {
        self.body.contains((mouse.column, mouse.row).into())
    }

    /// Let a drag select in `area` (clipped to the frame): `panel`'s cells.
    fn selectable(&mut self, buf: &Buffer, panel: select::Panel, area: Rect) {
        let area = area.intersection(buf.area);
        if !area.is_empty() {
            self.map.regions.push(Region { panel, area });
        }
    }

    /// Let a drag select in the lane: `rows` rows below `top`, from `left`.
    /// The `bars` among the `shown` rows (from row `y0`) drew a mark bar in
    /// their first cell, which reads as a blank.
    fn lane(
        &mut self,
        buf: &Buffer,
        (left, top, room, rows): (u16, u16, u16, usize),
        bars: &[usize],
        (shown, y0): (Range<usize>, u16),
    ) {
        self.selectable(
            buf,
            select::Panel::Lane,
            Rect::new(left, top, room, cells(rows)),
        );
        for row in bars.iter().filter(|row| shown.contains(*row)) {
            let bar = Rect::new(left, y0 + cells(row - shown.start), 1, 1);
            self.map.blanks.push(bar.intersection(buf.area));
        }
    }

    /// Record a border at `at` over `cells`, grabbed within a cell of it.
    fn border(&mut self, buf: &Buffer, edge: Edge, at: u16, cells: Rect) {
        let grab = match edge {
            Edge::Sidebar => Rect::new(cells.x.saturating_sub(1), cells.y, 3, cells.height),
            Edge::List => Rect::new(cells.x, cells.y.saturating_sub(1), cells.width, 3),
        };
        self.borders.push(Border {
            edge,
            at,
            cells: cells.intersection(buf.area),
            grab: grab.intersection(buf.area),
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
        self.strip(|hit| matches!(hit, Hit::Settings));
    }

    /// The lane's selectable cells this frame drew, if it drew any: the
    /// chat column the seat view paints into.
    pub(crate) fn lane_area(&self) -> Option<Rect> {
        self.map
            .regions
            .iter()
            .find(|region| region.panel == select::Panel::Lane)
            .map(|region| region.area)
    }

    /// Retire the chat column's own targets and lane selection for the seat
    /// view painted over it, and let a drag select in `body` instead. The
    /// sidebar's targets, borders and scroll bounds stand.
    pub(crate) fn peek_column(&mut self, buf: &Buffer, body: Rect) {
        self.targets
            .retain(|(_, hit)| !matches!(hit, Hit::Compose | Hit::Turn(_) | Hit::Fold(_)));
        if let Some(lane) = self.lane_area() {
            self.map
                .regions
                .retain(|region| region.panel != select::Panel::Lane);
            self.map
                .blanks
                .retain(|blank| blank.intersection(lane).is_empty());
        }
        self.selectable(buf, select::Panel::Peek, body);
    }

    fn strip(&mut self, keep: impl Fn(&Hit) -> bool) {
        self.targets.retain(|(_, hit)| keep(hit));
        self.borders.clear();
        self.chat = Rect::default();
        (self.list, self.body) = (Rect::default(), Rect::default());
        (self.list_start, self.body_top) = (None, None);
        (self.list_max, self.body_max) = (0, 0);
        (self.seats, self.focused) = (Vec::new(), None);
        self.page_rows = 0;
        self.max_scroll = 0;
        self.complete = true;
        self.turns.clear();
        self.folds.clear();
        self.cursor = None;
        self.settings_page_rows = 0;
        self.settings_max_scroll = 0;
        self.map = Map::default();
    }
}

/// The colours one frame draws in: a palette, or none.
#[derive(Debug, Clone, Copy)]
pub(super) struct Paint(Option<Palette>);

impl Paint {
    pub(super) fn of(look: Option<&Look>) -> Self {
        Self(look.filter(|look| look.drawn).map(|look| look.palette))
    }

    /// Whether a look is drawn at all: colour and emphasis are worn only then.
    pub(super) fn drawn(self) -> bool {
        self.0.is_some()
    }

    /// A foreground from the palette, or none.
    pub(super) fn fg(self, token: impl Fn(&Palette) -> &'static str) -> Style {
        self.0.map_or_else(Style::new, |palette| {
            Style::new().fg(colour(token(&palette)))
        })
    }

    /// A ground from the palette, or none.
    pub(super) fn ground(self, token: impl Fn(&Palette) -> &'static str) -> Style {
        self.0.map_or_else(Style::new, |palette| {
            Style::new().bg(colour(token(&palette)))
        })
    }

    /// A mark's accent.
    fn mark(self, mark: Mark) -> Style {
        self.fg(|palette| palette.accent(mark))
    }

    /// A section header's `area`: the raised panel ground behind its words, so
    /// the header reads apart from the rows under it; no ground with no colour.
    fn band(self, buf: &mut Buffer, area: Rect) {
        if self.0.is_some() {
            buf.set_style(area, self.ground(|palette| palette.panel));
        }
    }

    /// The selected row `area`, a session's two lines or a seat's: the selected
    /// ground, the dim words in the selected ink so only it and the accents sit
    /// on it; else reverse video.
    fn lift(self, buf: &mut Buffer, area: Rect) {
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
    pub(super) fn selected(self) -> Style {
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
    let chat = if let Some(rule) = sidebar_width(area, split) {
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
    layout.chat = Rect::new(chat.start, 0, width - chat.start, height);
    layout.seats_session = ctx.screen.selected.map(|entry| entry.name.clone());
    chat_column(&ctx, buf, chat.start + 2..width - 2, &mut layout);
    // The warning rides the chat's rule row, over the left of the rule.
    if let Some(line) = fallback {
        put(buf, 0, 1, &line, width, dim);
    }
    if overlay_open(screen.model) {
        keys_row(&ctx, buf, &mut layout);
    }
    layout
}

/// Whether a full-pane panel is open: the footer row is its own, and the
/// chat's hint row draws no version or gear under it.
fn overlay_open(model: &Model) -> bool {
    model.settings_open() || model.peek_open() || model.reader_open()
}

/// The cells a draft has in the chat column of `area` split as `split`: what
/// the caller wraps it to.
pub(crate) fn draft_width(area: Rect, split: Split) -> usize {
    let left = sidebar_width(area, split).map_or(0, |rule| rule + 1) + 2;
    usize::from(area.width.saturating_sub(2).saturating_sub(left)).max(1)
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

/// The address a draft to `target` is drawn under.
pub(crate) fn address(target: &str) -> String {
    format!("to {target}")
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
/// all when its sessions need fewer: two thirds of the rows the list and the
/// tab body share, never fewer than the 18 (from 40 high) or 11 the list once
/// had, sized to its sessions, until a drag asks for a height of its own.
fn list_rows(area: Rect, split: Split, count: usize) -> (u16, bool) {
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

/// Which sessions the list shows and where it ends.
struct List {
    visible: Range<usize>,
    /// The `↑ / ↓ N more` row, when sessions are out of view.
    more: Option<u16>,
    /// The first row past the list.
    end: u16,
}

impl List {
    /// The list for `count` sessions in `budget` rows, one row each: all of
    /// them while they fit, else a window from `top`, or with `selected` kept
    /// in view, under a `more` row. A `pinned` list ends at its budget, else
    /// where its rows do.
    fn of(
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

fn sidebar(ctx: &Ctx<'_, '_>, buf: &mut Buffer, rule: u16, layout: &mut Layout) {
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
/// and how many sessions need the human — `Agents 32 · Working 10 · Needs you
/// 1`. It tries four rungs and keeps the first that fits `room`: the words, short
/// labels (`Agt 32 · Wrk 10 · Need 1`), an icon and a count each (`◈ 32 ● 10 ⚠
/// 1`), the needs-you count alone. A count is never cut and a lower bound keeps
/// its `+?`. Counts use regular text ink and the labels and separators dim ink;
/// the needs-you count wears `tone`, and an icon the accent it always did.
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
    let words = |labels: [&'static str; 3]| {
        let mut spans = Vec::new();
        for ((label, number), ink) in labels.into_iter().zip(&numbers).zip(inks) {
            spans.extend((!spans.is_empty()).then(|| Span::styled(" · ", dim)));
            spans.push(Span::styled(label, dim));
            spans.push(Span::styled(format!(" {number}"), ink));
        }
        Line::from(spans)
    };
    let icons = |keep: &[usize]| {
        let marks = [
            (if ctx.icons { "◈" } else { "@" }, text),
            (Mark::Working.glyph(ctx.icons), paint.mark(Mark::Working)),
            (Mark::NeedsYou.glyph(ctx.icons), tone),
        ];
        let mut spans = Vec::new();
        for &at in keep {
            spans.extend((!spans.is_empty()).then(|| Span::styled(" ", dim)));
            spans.push(Span::styled(marks[at].0.to_owned(), marks[at].1));
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
fn drawn_cols(text: &str) -> u16 {
    cells(drawn(text).iter().map(|(_, width)| width).sum())
}

/// `text` then `tail` in `room` cells: whole, else the text alone, else its
/// head cut at a grapheme and closed on `…`.
fn fit(text: &str, tail: &str, room: u16) -> String {
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
}

impl Cells {
    fn of(spans: Vec<Span<'static>>) -> Self {
        Self {
            left: Line::from(spans),
            right: None,
            seat: None,
            continued: false,
        }
    }
}

/// The Agents body: per seat one label row, a second when its label does not
/// fit beside its name and state, and a dim detail row under them for its
/// newest declaration's own reason and age (`no reason given` when it left
/// none, `reason unavailable` when the journal could not be read). The label
/// row carries its mark, name, the model its frame drew (the client when none
/// was) and the effort, each only when known and never the profile, and its
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
    // The roster's names share one column, so the models line up under it.
    let column = agents
        .iter()
        .map(|agent| drawn_cols(&agent.name))
        .max()
        .unwrap_or(0);
    // Where the session's waits end, above the seats whose chain it follows.
    if let Some(words) = entry.wait_end_words().filter(|_| !agents.is_empty()) {
        rows.push(Cells::of(vec![Span::styled(fit(&words, "", room), dim)]));
    }
    for agent in agents {
        let model = if agent.model.is_empty() {
            agent.client.as_str()
        } else {
            agent.model.as_str()
        };
        // A typed wait names its target in place of the generic wait word;
        // what the narrow row would cut rides its declaration row.
        let typed = entry
            .agents
            .iter()
            .find(|seat| seat.name == agent.name && agent.state == "waiting-agent")
            .and_then(|seat| seat.typed_wait(ctx.screen.now))
            .map(|(on, _, _)| on.words(false));
        let seat = Seat {
            name: &agent.name,
            mark: agent.mark(),
            state: typed
                .as_deref()
                .unwrap_or_else(|| crate::theme::seat_word(&agent.state)),
            meta: [model, agent.effort.as_str()],
        };
        rows.extend(seat_rows(ctx, &seat, column, room));
        rows.extend(declaration_row(ctx, entry, &agent.name, room));
    }
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

/// What one seat row says.
struct Seat<'a> {
    name: &'a str,
    mark: Mark,
    /// The state word, drawn at the right end.
    state: &'a str,
    /// The model (or client) and the effort; empty when unknown.
    meta: [&'a str; 2],
}

/// `seat`'s rows in `room` cells: its mark, its name and its state word at the
/// right end, and its whole label (model and effort) aligned at `column` when
/// it fits there; else a second row carries the label under the name, the
/// effort left off only when that row cannot hold both and the model cut on
/// `…` only past it. Short of room the name's tail goes down to [`NAME_MIN`]
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
    let [model, effort] = seat.meta;
    let both = [model, effort]
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
    }];
    if !both.is_empty() && !inline {
        let width = room.saturating_sub(lead);
        let words = if drawn_cols(&both) <= width {
            both
        } else if model.is_empty() {
            fit(&both, "", width)
        } else {
            fit(model, "", width)
        };
        rows.push(Cells {
            left: Line::from(vec![
                Span::raw(" ".repeat(usize::from(lead))),
                Span::styled(words, dim),
            ]),
            right: None,
            seat: Some(seat.name.to_owned()),
            continued: true,
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
    if let Some((on, missing, overdue)) = agent
        .typed_wait(ctx.screen.now)
        .filter(|_| entry.agent_state_is_exact())
    {
        let notes = [
            missing.then(|| format!("{} (not found)", on.label())),
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
    })
}

// ---------------------------------------------------------------------------
// the chat column
// ---------------------------------------------------------------------------

/// The chat column over `columns`: the header, the turns anchored to the
/// bottom, the composer.
fn chat_column(ctx: &Ctx<'_, '_>, buf: &mut Buffer, columns: Range<u16>, layout: &mut Layout) {
    let (screen, paint) = (ctx.screen, ctx.paint);
    let height = buf.area.height;
    let (left, end) = (columns.start, columns.end);
    let room = end - left;
    let (dim, border) = (paint.fg(|p| p.dim), paint.fg(|p| p.border));
    header(ctx, buf, left, end);
    put(buf, left, 1, &"─".repeat(usize::from(room)), room, border);
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
    let main = screen
        .selected
        .and_then(|entry| entry.main_agent.as_deref())
        .or_else(|| match screen.composer {
            Composer::NoHome => None,
            _ => screen.pair.first().map(String::as_str),
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
    let super::lane::Tail {
        mut rows,
        complete,
        turns,
        mut links,
    } = lane_tail(ctx, &clock, main, (room, need));
    if ctx.wait.lane {
        rows.push(Line::from(Span::styled("loading", dim)));
        links.push(Vec::new());
    }
    let (turns, bars, folds) = place_turns(ctx, &turns, &mut rows, layout);
    let last = rows
        .len()
        .saturating_sub(scroll)
        .max(room_rows.min(rows.len()));
    let first = last.saturating_sub(room_rows);
    let y0 = bottom - cells(last - first);
    for (step, line) in rows[first..last].iter().enumerate() {
        put_line(buf, left, y0 + cells(step), line, room);
    }
    layout.link_runs(links.get(first..last).unwrap_or_default(), (left, y0, room));
    // A fold row is recorded before its turn: the first target to hold a cell wins.
    for (key, row) in folds {
        if (first..last).contains(&row) {
            let rect = Rect::new(left, y0 + cells(row - first), room, 1);
            layout.record(buf, rect, Hit::Fold(key));
        }
    }
    turn_targets(buf, layout, &turns, (first..last, left, y0, room));
    layout.lane(buf, (left, top, room, room_rows), &bars, (first..last, y0));
    let viewing = screen.composer.in_place();
    if room_rows > 0 && scroll > 0 && last < rows.len() && !viewing {
        put(buf, left, bottom, "↓ newer turns below · PgDn", room, dim);
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
        // The draft under the address, from the column's left edge, and the
        // cursor after the cells its row draws before it, never past the row.
        let text = paint.fg(|p| p.text);
        for (y, row) in (composer + 1..).zip(&view.rows) {
            put(buf, left, y, row, room, text);
        }
        let before = cells(Span::raw(view.before.as_str()).width());
        layout.cursor = (room > 0).then(|| {
            let row = composer + 1 + cells(view.cursor_row);
            Position::new(left + before.min(room - 1), row)
        });
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
}

/// The hint row, the pane's last: a pending line — an armed key, a refused
/// paste, an ask on its way — in full ink so it is not missed, else the
/// composer's hint; then the keys that work in this mode, when they fit whole;
/// at the right end `ae <version>` and the gear.
fn hint_row(
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
    let keys = mode_hint(&ctx.screen.composer);
    let rest_of = |keys: &str| match (first.is_empty(), keys.is_empty()) {
        (_, true) => String::new(),
        (true, false) => keys.to_owned(),
        (false, false) => format!(" · {keys}"),
    };
    let fits = |rest: &str| cells(Span::raw(rest).width()) <= end.saturating_sub(at);
    // The clear key is named only over text, and only where it fits whole.
    let text = matches!(&ctx.screen.composer, Composer::Home { view: Some(_), draft, .. } if !draft.is_empty());
    let long = rest_of(&format!("{keys} · ^C clears"));
    let rest = if text && fits(&long) {
        long
    } else {
        rest_of(keys)
    };
    if fits(&rest) {
        at = put(buf, at, y, &rest, end - at, paint.fg(|p| p.dim));
    }
    // A panel's footer is its own row: no second gear under it.
    if !overlay_open(ctx.screen.model) {
        tail(ctx, buf, layout, (y, at), false);
    }
}

/// The keys that work in this mode: writing names the three that matter,
/// held takes every key and says nothing.
fn mode_hint(composer: &Composer<'_>) -> &'static str {
    match composer {
        Composer::Home { view: Some(_), .. } => "Enter sends · Alt+Enter ^J newline · Esc browses",
        Composer::Held { .. } => "",
        _ => "? keys",
    }
}

/// The lane's newest rows as the app's own state shows them: `size` is the
/// column's width and how many rows are wanted.
fn lane_tail(
    ctx: &Ctx<'_, '_>,
    clock: &view::Style,
    main: &str,
    size: (u16, usize),
) -> super::lane::Tail {
    let show = super::lane::Show {
        clock,
        main,
        open: (!ctx.wait.open.is_empty()).then_some(ctx.wait.open),
        sent: ctx.wait.sent,
        icons: ctx.icons,
        base: &ctx.screen.overview.base,
    };
    let (room, need) = size;
    let lane = ctx.screen.lane;
    super::lane::tail_with(lane, usize::from(room), ctx.paint, &show, need, None)
}

/// A turn's fingerprint and the rows it spans.
type TurnSpan = (u64, Range<usize>);

/// The produced turns with their fingerprints and row spans, oldest first, the
/// bars of the marked one and each fold row as its fold key and row. The
/// marked turn is recoloured in place, and the frame keeps each turn's place
/// and fold for the keys.
fn place_turns(
    ctx: &Ctx<'_, '_>,
    turns: &[super::lane::Turn],
    rows: &mut [Line<'static>],
    layout: &mut Layout,
) -> (Vec<TurnSpan>, Vec<usize>, Vec<(u64, usize)>) {
    let keyed: Vec<_> = (turns.iter())
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
            let barred = super::lane::mark_rows(&mut rows[span.clone()], ctx.paint, ctx.icons);
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
    turns: &[(u64, Range<usize>)],
    (window, left, y0, room): (Range<usize>, u16, u16, u16),
) {
    for (key, span) in turns {
        let (from, to) = (span.start.max(window.start), span.end.min(window.end));
        if from < to {
            let y = y0 + cells(from - window.start);
            let rect = Rect::new(left, y, room, cells(to - from));
            layout.record(buf, rect, Hit::Turn(*key));
        }
    }
}

/// The chat header: the session, its pair, and right-aligned its branch (at
/// 140 columns or more) and its activity.
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
    let width = |right: &&String| cells(right.chars().count());
    let rights: Vec<&String> = branch.iter().chain(std::iter::once(&activity)).collect();
    let room_for = |right: &&String| after + 2 + width(right) <= end;
    let right = (rights.iter())
        .find(|right| room_for(right) && end.saturating_sub(width(right) + 2) >= wanted)
        .or_else(|| rights.iter().find(|right| room_for(right)));
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
    // What waits on the human comes before the directory: the seat's ask and
    // its age, cut to what the room leaves. It needs a dozen cells to read.
    let reason = reason_words(ctx, room);
    let dir_from = reason.as_ref().map_or(from, |words| {
        let ink = paint.mark(Mark::NeedsYou);
        let used = put(buf, from, 0, words, room, ink);
        used + 3
    });
    let room = stop.saturating_sub(dir_from);
    if reason.is_some() && room < REASON_MIN {
        return;
    }
    let shown = clip_tail(dir, usize::from(room));
    put(buf, dir_from, 0, &shown, room, paint.fg(|p| p.dim));
}

/// The fewest cells the header's reason segment, or a directory beside it, is
/// worth drawing in.
const REASON_MIN: u16 = 12;

/// The open ask the header names, as `⚠ <seat>: <words> · <age>` in `room`
/// cells: the seat the chat column views, else the one the cursor stands on,
/// else the first that waits. `None` when nothing waits or the room is under
/// [`REASON_MIN`].
fn reason_words(ctx: &Ctx<'_, '_>, room: u16) -> Option<String> {
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

/// The composer's address row, the input row under it and the hint under that.
fn composer_lines(ctx: &Ctx<'_, '_>) -> (Line<'static>, Line<'static>, String) {
    let paint = ctx.paint;
    let (text, dim) = (paint.fg(|p| p.text), paint.fg(|p| p.dim));
    let quiet = |words: String| Line::from(Span::styled(words, dim));
    match ctx.screen.composer {
        Composer::Home { home, view, draft } => {
            let mut address = vec![Span::styled(address(home), paint.fg(|p| p.title))];
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
            quiet(format!(
                "viewing {} · read-only · Esc chat",
                crate::board::terminal_text(seat)
            )),
            Line::default(),
            "Esc or click the session for its chat".to_owned(),
        ),
        Composer::Reading { path } => (
            quiet(format!(
                "reading {} · read-only · Esc lane",
                crate::board::terminal_text(path)
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

/// The browse keys the Keys tab lists, in the order it shows them: the keys
/// and what they do.
const BROWSE_KEYS: [(&str, &str); 19] = [
    ("1-9", "select the Nth session"),
    ("j/k Up/Down", "previous / next session"),
    ("!", "next session needing you"),
    ("oo", "open seat; twice in 2 s"),
    ("m", "session menu; right-click a card too"),
    ("s", "start / stop the selected session"),
    ("z", "show / hide stopped sessions"),
    ("v", "peek at the seat's screen"),
    ("r", "read the marked turn's first file link"),
    ("n/p", "step the seat cursor (▹ or -)"),
    ("PgUp/PgDn", "scroll the chat"),
    ("[ ]", "mark older / newer turn"),
    ("y", "copy the marked turn"),
    ("Enter i", "write; a paste writes too"),
    ("Esc", "clear the cursor, then go home"),
    ("◂ or <", "marks the home session"),
    (",", "settings"),
    ("?", "this help"),
    ("qq", "quit: twice within 2 s"),
];

/// The footer a full-pane panel keeps on the pane's last row, the full width:
/// the panel's word and its one hint from the left, then `ae <version>` and the
/// gear right-aligned. The cells are reset first, so the closed frame's hint
/// row, sidebar ground and rule never show under the panel.
fn keys_row(ctx: &Ctx<'_, '_>, buf: &mut Buffer, layout: &mut Layout) {
    let paint = ctx.paint;
    let (width, y) = (buf.area.width, buf.area.height - 1);
    for x in 0..width {
        buf[(x, y)].reset();
    }
    buf.set_style(Rect::new(0, y, width, 1), paint.ground(|p| p.ink));
    let reading = ctx.screen.model.reader_open();
    let peeking = ctx.screen.model.peek_open() || reading;
    let (word, hint) = if ctx.screen.model.settings_open() {
        ("settings", "Esc close")
    } else if reading {
        (
            "reading",
            ctx.screen.model.note().unwrap_or("Esc back · g/G ends"),
        )
    } else {
        // A copy's outcome rides here as well as on the hint row.
        ("viewing", ctx.screen.model.note().unwrap_or("Esc chat"))
    };
    let x = put(
        buf,
        LEFT,
        y,
        word,
        width - LEFT,
        paint.fg(|p| p.text).add_modifier(Modifier::BOLD),
    );
    let rest = format!("   {hint}");
    let hints_end = put(
        buf,
        x,
        y,
        &rest,
        width.saturating_sub(x),
        paint.fg(|p| p.dim),
    );
    tail(ctx, buf, layout, (y, hints_end), peeking);
}

/// `ae <version>` and the gear, right-aligned on row `y`, each only when it
/// fits after `hints_end` and a gap: narrowing drops the version first, then
/// the gear. The peek panel hides the gear, because a click on it would be dead.
fn tail(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    layout: &mut Layout,
    (y, hints_end): (u16, u16),
    peeking: bool,
) {
    let width = buf.area.width;
    let gear = if ctx.icons { "\u{2699}" } else { "*" };
    let tail = if peeking {
        crate::version_line()
    } else {
        format!("{} {gear}", crate::version_line())
    };
    let tail_cells = cells(Span::raw(tail.as_str()).width());
    let gear_cells = cells(Span::raw(gear).width());
    let room = |need: u16| usize::from(hints_end) + 2 + usize::from(need) <= usize::from(width);
    let style = ctx.paint.fg(|p| p.dim);
    // The click covers the gear and the blank before it, like the status
    // line's settings range; `room` keeps that blank free.
    let target = Rect::new(width.saturating_sub(gear_cells + 1), y, gear_cells + 1, 1);
    if room(tail_cells) {
        put(buf, width - tail_cells, y, &tail, tail_cells, style);
        if !peeking {
            layout.record(buf, target, Hit::Settings);
        }
    } else if !peeking && room(gear_cells) {
        put(buf, width - gear_cells, y, gear, gear_cells, style);
        layout.record(buf, target, Hit::Settings);
    }
}

// ---------------------------------------------------------------------------
// the settings overlay: a reusable full-pane panel
// ---------------------------------------------------------------------------

/// The overlay's title row.
const SETTINGS_TITLE: &str = "Settings";
/// The tabs in draw order.
const SETTINGS_TABS: [&str; 5] = ["Quota", "Config", "About", "Instructions", "Keys"];
/// The clickable close label, right-aligned on the title row.
const SETTINGS_CLOSE: &str = "Esc close";
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
    let width = buf.area.width;
    let selected = match tab {
        SettingsTab::Quota => 0,
        SettingsTab::Config => 1,
        SettingsTab::About => 2,
        SettingsTab::Instructions => 3,
        SettingsTab::Keys => 4,
    };
    panel(
        buf,
        layout,
        paint,
        &Panel {
            title: SETTINGS_TITLE,
            note: "",
            tabs: &SETTINGS_TABS,
            hits: &[
                Hit::SettingsTab(SettingsTab::Quota),
                Hit::SettingsTab(SettingsTab::Config),
                Hit::SettingsTab(SettingsTab::About),
                Hit::SettingsTab(SettingsTab::Instructions),
                Hit::SettingsTab(SettingsTab::Keys),
            ],
            close: (SETTINGS_CLOSE, Hit::SettingsClose),
            selected,
            body: &settings_body(
                tab,
                bodies,
                paint,
                width,
                look.is_none_or(|look| look.icons),
            ),
            scroll: Scroll::Top(scroll),
        },
    );
}

/// What one full-pane panel draws: its title, tab row and scrolled body.
struct Panel<'a> {
    title: &'a str,
    /// A dim line on the tab row, after any tabs: the peek panel's status.
    note: &'a str,
    tabs: &'a [&'a str],
    /// The click target of each tab title, in tab order.
    hits: &'a [Hit],
    /// The label drawn right-aligned on the title row, and its target.
    close: (&'a str, Hit),
    selected: usize,
    body: &'a [(String, Style)],
    scroll: Scroll,
}

/// How far a panel's body is scrolled.
enum Scroll {
    /// Rows down from the first.
    Top(usize),
    /// Rows back from the newest, which the body ends on.
    Back(usize),
}

/// One full-pane panel over rows `0..height-1`: title, tab row, rule and a
/// scrolled body, then its scroll bounds. Settings and the peek panel are the
/// callers; the later `?` help reuses the seam. Below [`MIN`] the closed frame's fallback
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
    let (label, close) = &panel.close;
    let label_x = width.saturating_sub(LEFT + cells(Span::raw(*label).width()));
    // The title is cut a cell short of the close label, which keeps its cells.
    let title_room = label_x.saturating_sub(LEFT + 1);
    let title_end = put(buf, LEFT, 0, panel.title, title_room, heading);
    if label_x > title_end {
        let end = put(buf, label_x, 0, label, width - label_x, paint.fg(|p| p.dim));
        layout.record(buf, Rect::new(label_x, 0, end - label_x, 1), close.clone());
    }
    // Three cells between titles when all of them fit that way, else one.
    let titles: usize = panel.tabs.iter().map(|name| Span::raw(*name).width()).sum();
    let wide = usize::from(LEFT) + titles + 3 * panel.tabs.len().saturating_sub(1);
    let gap = if wide <= usize::from(width) {
        "   "
    } else {
        " "
    };
    let mut x = LEFT;
    for (index, name) in panel.tabs.iter().enumerate() {
        let style = if index == panel.selected {
            paint.selected()
        } else {
            paint.fg(|p| p.text)
        };
        let end = put(buf, x, 1, name, width.saturating_sub(x), style);
        if let Some(hit) = panel.hits.get(index) {
            layout.record(buf, Rect::new(x, 1, end - x, 1), hit.clone());
        }
        x = put(buf, end, 1, gap, width.saturating_sub(end), style);
    }
    put(buf, LEFT, 1, panel.note, width - LEFT, paint.fg(|p| p.dim));
    let rule: String = "─".repeat(usize::from(width));
    put(buf, 0, 2, &rule, width, paint.fg(|p| p.border));
    let visible = usize::from(height.saturating_sub(4));
    // The draw offset clamps to what is there before a line is chosen: past
    // the end paints an emptied body, and the model's clamp after this frame
    // converges the state to the same bound.
    let (start, max) = match panel.scroll {
        Scroll::Top(offset) => {
            let max = panel.body.len().saturating_sub(visible);
            (offset.min(max), max)
        }
        Scroll::Back(back) => peek::window(panel.body.len(), visible, back),
    };
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
    paint_reader(view, Paint::of(look.as_ref()), buf, area).0
}

/// Paint the reader into the frame's chat column: its header on the lane's
/// first row, the numbered file under it. The sidebar and the composer rows
/// the frame drew stand, and a drag selects in the file text, never the
/// gutter.
pub(crate) fn paint_reader_column(
    view: &reader::View<'_>,
    look: Option<&Look>,
    buf: &mut Buffer,
    layout: &mut Layout,
) -> Bounds {
    let Some(lane) = layout.lane_area() else {
        layout.peek_column(buf, Rect::default());
        return Bounds::default();
    };
    let (bounds, text) = paint_reader(view, Paint::of(look), buf, lane);
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
    let lines = match view.body {
        Some(reader::Body::Text { lines, .. }) => Some(lines),
        _ => None,
    };
    let gutter = lines.map_or(0, |lines| reader::gutter(lines.len()));
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
    let status = reader::status(&reader::View { flow, ..*view }, visible);
    let at = view.line.map(|line| format!(":{line}")).unwrap_or_default();
    let header = format!(
        "Read · {}{at} · {status}",
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

/// The overlay body's styled rows: every field through the terminal-text
/// neutraliser, because paths and config values are hostile text.
fn settings_body(
    tab: SettingsTab,
    bodies: &SettingsBodies,
    paint: Paint,
    width: u16,
    icons: bool,
) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let head = paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    let clean = crate::board::terminal_text;
    let mut body = match tab {
        SettingsTab::Quota => match bodies.quota.as_ref() {
            None => vec![("loading".to_owned(), dim)],
            Some(scopes) if scopes.is_empty() => vec![("no quota scopes read".to_owned(), dim)],
            Some(scopes) => crate::quota::view::layout(
                scopes,
                crate::quota::view::Geometry {
                    width: usize::from(width.saturating_sub(LEFT)),
                    rows: None,
                    icons,
                },
            )
            .into_iter()
            .map(|line| {
                let style = match line.tone {
                    Tone::Plain => text,
                    Tone::Dim => dim,
                    Tone::Head => head,
                    Tone::Low => paint.fg(|p| p.waiting_agent),
                    Tone::Critical => paint.fg(|p| p.needs_you),
                    Tone::Stale => paint.fg(|p| p.stale),
                };
                (clean(&line.text), style)
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
        SettingsTab::Instructions => instructions_body(&bodies.instructions, width, paint),
        SettingsTab::Keys => {
            let mut rows = marks_legend(paint, icons);
            rows.extend(keys_body(paint));
            rows
        }
    };
    // A kept quota body says how old it is, above it, in the stale tone.
    if let (SettingsTab::Quota, Some(kept)) = (tab, &bodies.quota_kept) {
        body.insert(0, (clean(kept), paint.fg(|p| p.stale)));
    }
    body
}

/// The write, held and settings keys the help lists beside the browse table.
const WRITE_KEYS: [(&str, &str); 21] = [
    ("Enter", "send the line"),
    ("Alt+Enter ^J", "new line in the draft"),
    ("^V", "attach a clipboard PNG"),
    ("Esc", "keep the draft, browse"),
    ("^C", "clear a draft; an empty one quits at once"),
    ("^C", "just after a clear: arms; again in 2 s quits"),
    ("Left Right", "move the cursor"),
    ("Home End", "start / end of line"),
    ("^A ^E", "start / end of line"),
    ("Backspace", "erase before the cursor"),
    ("Delete", "erase at the cursor"),
    ("^W", "erase the word before the cursor"),
    ("Alt+Bksp", "same, stopping at punctuation (Option as Meta)"),
    ("^K", "erase to the end of the line"),
    ("^U", "clear the whole draft, every line"),
    ("^Y", "put the cleared draft / last erase back, once"),
    ("paste", "literal text, never Enter; CR becomes LF"),
    ("@seat text", "ask that seat"),
    ("/close [id]", "withdraw your open ask"),
    ("/open seat", "open that seat's pane"),
    ("!command", "run it: a popup in tmux, else started"),
];
const HELD_KEYS: [(&str, &str); 4] = [
    ("Enter", "try to write again"),
    ("Esc", "browse"),
    ("^C", "quit; a kept draft clears first"),
    ("any other", "swallowed; a paste too"),
];
const MOUSE_KEYS: [(&str, &str); 4] = [
    ("click", "select a session, tab, seat or turn"),
    ("wheel", "scroll 3 rows (the list: 1 session)"),
    ("drag", "a border resizes the sidebar / list"),
    (
        "drag text",
        "select drawn text, copy on release; Esc clears",
    ),
];
const PEEK_KEYS: [(&str, &str); 6] = [
    ("j/k Up/Down", "scroll a line"),
    ("PgUp/PgDn", "scroll a page"),
    ("Esc q v", "back to chat; click the session too"),
    ("^C", "quit; a draft clears first"),
    ("wheel", "scroll 3 rows per notch"),
    ("any other", "swallowed; nothing reaches the pane"),
];
const READER_KEYS: [(&str, &str); 7] = [
    ("j/k Up/Down", "scroll a line"),
    ("PgUp/PgDn", "scroll a page"),
    ("g/G", "top / bottom of the file; Home/End too"),
    ("Esc q r", "back to the lane"),
    ("^C", "quit; a draft clears first"),
    ("wheel", "scroll 3 rows per notch"),
    ("any other", "swallowed; nothing is typed or written"),
];
const SETTINGS_KEYS: [(&str, &str); 10] = [
    ("Tab 1-5", "show that tab"),
    ("j/k Up/Down", "scroll a line"),
    ("PgUp/PgDn", "scroll a page"),
    ("?", "show these keys"),
    ("Esc q ,", "close"),
    ("^C", "quit; a draft clears first"),
    ("click tab", "show that tab"),
    ("click gear", "or Esc close label: closes it"),
    ("wheel", "scroll 3 rows per notch"),
    ("any other", "swallowed; a paste too"),
];

/// What each of the seven marks means, in the glyph vocabulary being drawn.
/// The words are the ones a seat row prints ([`crate::theme::seat_word`]); the
/// rest says what the mark rests on, so a finished seat is only ever one that
/// declared it and an empty input box is never read as completion.
fn marks_legend(paint: Paint, icons: bool) -> Vec<(String, Style)> {
    let (text, head) = (paint.fg(|p| p.text), paint.fg(|p| p.title));
    let mut rows = vec![(
        "Marks (the glyph before a name)".to_owned(),
        head.add_modifier(Modifier::BOLD),
    )];
    rows.extend(
        [
            (Mark::Dead, "dead — agent process gone"),
            (
                Mark::NeedsYou,
                "needs you — waiting on you / blocked / problem; the reason follows",
            ),
            (
                Mark::Stale,
                "unknown — state unestablished, or silent past the window",
            ),
            (Mark::Working, "working — a turn is running"),
            (
                Mark::WaitingAgent,
                "waiting on an agent — declared; its reason may name whom",
            ),
            (Mark::Done, "finished — only when the agent declared done"),
            (
                Mark::Idle,
                "no current status — alive, not working, no current done/waiting/blocked declaration",
            ),
        ]
        .map(|(mark, words)| (format!("{:<2}{words}", mark.glyph(icons)), text)),
    );
    rows
}

/// The Keys tab's rows: every key of browse, write, held and the overlay.
fn keys_body(paint: Paint) -> Vec<(String, Style)> {
    let (text, head) = (paint.fg(|p| p.text), paint.fg(|p| p.title));
    let head = head.add_modifier(Modifier::BOLD);
    let mut rows = vec![("Browse".to_owned(), head)];
    rows.extend(
        BROWSE_KEYS
            .iter()
            .map(|(keys, does)| (format!("{keys:<12}{does}"), text)),
    );
    rows.push((
        format!("{:<12}quit at once; a kept draft clears first", "^C"),
        text,
    ));
    rows.push(("Any other key cancels qq / oo.".to_owned(), text));
    let sections = [
        ("Write (Enter or a paste starts it)", &WRITE_KEYS[..]),
        ("Held (writing was refused)", &HELD_KEYS[..]),
        ("Peek (v on a highlighted seat)", &PEEK_KEYS[..]),
        ("Reader (r on a marked turn)", &READER_KEYS[..]),
        ("Settings (the gear, or ,)", &SETTINGS_KEYS[..]),
        ("Mouse (outside Settings)", &MOUSE_KEYS[..]),
    ];
    for (title, keys) in sections {
        rows.push((title.to_owned(), head));
        rows.extend(
            keys.iter()
                .map(|(keys, does)| (format!("{keys:<12}{does}"), text)),
        );
    }
    rows
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

/// The instructions tab's rows: the render-now line, the custom instructions,
/// each generic rule once under its audience and one line per seat naming the
/// rules it receives. Every field is neutralised first and then wrapped to the
/// panel, a continuation row hanging two cells; nothing is cut here, the panel
/// scrolls.
fn instructions_body(view: &InstructionsView, width: u16, paint: Paint) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let head = paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    // `wrap` keeps a one-cell bar free ahead of every row.
    let room = usize::from(width.saturating_sub(LEFT)) + 1;
    let rows = |body: &str, style: Style| -> Vec<(String, Style)> {
        wrap(&crate::board::terminal_text(body), room, 0)
            .into_iter()
            .map(|(gap, piece)| (format!("{}{piece}", " ".repeat(gap)), style))
            .collect()
    };
    let protocol = match view {
        InstructionsView::Loading => return vec![("loading".to_owned(), dim)],
        InstructionsView::Gap(why) => return rows(why, text),
        InstructionsView::Ready(protocol) => protocol,
    };
    let mut out = rows(&format!("session: {}", protocol.session), dim);
    out.extend(rows(
        &format!(
            "<session>, <helpers> and <owner line> stand for what a launch fills in per seat. Rendered now by {}: the text a launch injects today. A running seat holds the text of its own launch time.",
            crate::version_line()
        ),
        dim,
    ));
    out.push((String::new(), text));
    out.extend(rows("Custom instructions ([prompt] instructions)", head));
    if let Some(custom) = &protocol.custom {
        out.extend(rows(
            &format!("source: {} {}", custom.source.word(), custom.file),
            dim,
        ));
        out.extend(rows(&custom.text, text));
    } else {
        let named = if protocol.files.is_empty() {
            "no config file".to_owned()
        } else {
            protocol.files.join(", ")
        };
        out.extend(rows(&format!("none in {named}"), dim));
    }
    for rule in &protocol.rules {
        out.push((String::new(), text));
        out.extend(rows(&format!("Rules for {}", rule.audience.word()), head));
        if let Some(owner) = &rule.owner {
            out.extend(rows(owner, dim));
        }
        out.extend(rows(&rule.text, text));
    }
    out.push((String::new(), text));
    out.extend(rows("Seats", head));
    for seat in &protocol.seats {
        let receives = match &seat.receives {
            Ok(audiences) => audiences
                .iter()
                .map(|audience| audience.word())
                .collect::<Vec<_>>()
                .join(" + "),
            Err(why) => format!("gap: {why}"),
        };
        let line = format!("{} · {} · {} · {receives}", seat.name, seat.slot, seat.role);
        out.extend(rows(&line, text));
    }
    out
}

// ---------------------------------------------------------------------------
// cells
// ---------------------------------------------------------------------------

/// `text · <age>` in `room` cells: a clipped text keeps its age.
fn aged(text: &str, since: Option<i64>, room: u16) -> String {
    aged_by(text, &age(since), room)
}

/// [`aged`] with the age already in words.
fn aged_by(text: &str, words: &str, room: u16) -> String {
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
pub(super) fn cells(count: usize) -> u16 {
    u16::try_from(count).unwrap_or(u16::MAX)
}

/// Whether (x, y) lies inside `buf`.
fn inside(buf: &Buffer, x: u16, y: u16) -> bool {
    let area = buf.area;
    x >= area.left() && x < area.right() && y >= area.top() && y < area.bottom()
}

/// Write `text` at (x, y), at most `max` cells and never past the buffer;
/// answers the column after it.
pub(super) fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, max: u16, style: Style) -> u16 {
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
    use ratatui_core::layout::{Position, Rect};
    use ratatui_core::style::Modifier;

    use super::{
        Composer, LEFT, Paint, Screen, aged, dragged_to, draw, instructions_body, keys_body,
    };
    use crate::app::fleet::{Counts, Facts, Fleet, Line2, Row};
    use crate::app::model::{Drag, Edge, Model};
    use crate::app::overview::{Open, Overview};
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
            unanswered: None,
            counts: Counts::Marks(vec![(Mark::Working, false, 3), (Mark::Done, true, 12)]),
            line2: Line2::Question {
                text: "a question long enough to be clipped at every width it meets".to_owned(),
                age_secs: Some(240),
            },
            home: index == 1,
        }
    }

    /// The sweep's populated asks and lane: an open ask, a recorded dir, a lane
    /// that scrolls, a coverage line.
    fn sweep_body() -> (Overview, Lane) {
        let overview = Overview {
            open: vec![Open {
                seat: "colead".to_owned(),
                text: "an open question long enough to clip".to_owned(),
                age_secs: Some(5),
            }],
            dir: Some("~/a/launch/directory/long/enough/to/clip".to_owned()),
            base: crate::console::link::Base::default(),
            unanswered: None,
            unanswered_oldest_secs: None,
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

    /// The instructions rows neutralise controls before they wrap, keep every
    /// word of a rule and its owner, fit the panel at the narrowest widths,
    /// name the winning source or the files that held none, and give each
    /// seat one line naming what it receives or its gap.
    #[test]
    fn instructions_rows_are_neutralised_wrapped_and_name_their_gaps() {
        use crate::app::settings::{
            ConfigSource, CustomInstructions, InstructionsView, Protocol, SeatLine,
        };
        use crate::render::{Audience, GenericRule};
        let hostile = format!("\u{1b}[31mred\u{7} {}\tend\nnext line", "word ".repeat(40));
        let ready = |custom, receives| {
            let seat = SeatLine {
                name: "lead".to_owned(),
                slot: "main".to_owned(),
                role: "lead",
                receives,
            };
            InstructionsView::Ready(Protocol {
                session: "api".to_owned(),
                files: vec!["/g".to_owned()],
                custom,
                rules: vec![GenericRule {
                    audience: Audience::Workers,
                    owner: Some(hostile.clone()),
                    text: hostile.clone(),
                }],
                seats: vec![seat],
            })
        };
        let custom = CustomInstructions {
            source: ConfigSource::Global,
            file: "/g".to_owned(),
            text: hostile.clone(),
        };
        let paint = Paint::of(None);
        let clean = crate::board::terminal_text(&hostile);
        for width in [40_u16, 80] {
            let view = ready(
                Some(custom.clone()),
                Ok(&[Audience::Every, Audience::LeadPair]),
            );
            let rows = instructions_body(&view, width, paint);
            let lines: Vec<&str> = rows.iter().map(|(line, _)| line.as_str()).collect();
            for line in &lines {
                assert!(!line.chars().any(char::is_control), "{line:?}");
                assert!(
                    line.chars().count() <= usize::from(width - LEFT),
                    "{line:?}"
                );
            }
            assert!(lines.contains(&"source: global /g"), "{lines:?}");
            let from = lines.iter().position(|line| *line == "Rules for workers");
            let to = lines.iter().position(|line| *line == "Seats");
            let kept: Vec<&str> = lines[from.expect("rules heading") + 1..to.expect("seats")]
                .iter()
                .flat_map(|line| line.split_whitespace())
                .collect();
            let twice = [&clean, &clean].map(|text| text.split_whitespace());
            assert_eq!(kept, twice.into_iter().flatten().collect::<Vec<_>>());
            let words: Vec<&str> = lines
                .iter()
                .flat_map(|line| line.split_whitespace())
                .collect();
            let seat = [
                "lead", "·", "main", "·", "lead", "·", "every", "seat", "+", "lead", "pair",
            ];
            assert!(
                words.windows(seat.len()).any(|run| run == seat),
                "{lines:?}"
            );
        }
        let lines = |view| instructions_body(&view, 80, paint);
        let gapped = lines(ready(None, Err("unknown slot".to_owned())));
        let gapped: Vec<&str> = gapped.iter().map(|(line, _)| line.as_str()).collect();
        assert!(gapped.contains(&"none in /g"), "{gapped:?}");
        assert!(
            gapped.contains(&"lead · main · lead · gap: unknown slot"),
            "{gapped:?}"
        );
        let torn = InstructionsView::Gap("\u{1b}]0;x\u{7}torn".to_owned());
        assert_eq!(lines(torn)[0].0, "\u{fffd}]0;x\u{fffd}torn");
        let fit = "x".repeat(usize::from(80 - LEFT));
        let mut tight = custom;
        tight.text.clone_from(&fit);
        let filled = lines(ready(Some(tight), Ok(&[])));
        assert!(filled.iter().any(|row| row.0 == fit));
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
            view: Some(&draft),
            draft: "",
        };
        let read_only = Composer::ReadOnly {
            why: "owned elsewhere",
        };
        // Each geometry the frame draws: the Agents body without and with
        // seats, and the browse and compose composers. Foreign, no-home and mono draw
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
                let _ = model.key(crate::app::model::Key::PageUp, &fleet, false);
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
            let _ = self.model.key(key, &self.fleet, false);
        }

        fn draw(&self, width: u16, height: u16, composer: Composer<'_>) -> Buffer {
            self.laid(width, height, composer).0
        }

        fn screen<'a>(&'a self, composer: Composer<'a>) -> Screen<'a> {
            Screen {
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
            }
        }

        /// One footer row on a one-row buffer: the closed frame's hint row
        /// in a sidebar-less chat column (cols 2..width-2), or the panel
        /// footer: what it drew, and the layout it recorded.
        fn footer_at(
            &self,
            width: u16,
            composer: Composer<'_>,
            closed: bool,
        ) -> (String, super::Layout) {
            let screen = self.screen(composer);
            let ctx = super::Ctx {
                screen: &screen,
                paint: Paint::of(screen.look.as_ref()),
                icons: true,
                wait: super::Wait::default(),
            };
            let mut buf = Buffer::empty(Rect::new(0, 0, width, 1));
            let mut layout = super::Layout::default();
            if closed {
                let (_, _, hint) = super::composer_lines(&ctx);
                super::hint_row(&ctx, &mut buf, &mut layout, (2, width - 2), &hint);
            } else {
                super::keys_row(&ctx, &mut buf, &mut layout);
            }
            (line(&buf, 0).concat().trim_end().to_owned(), layout)
        }

        /// The frame and the layout it recorded.
        fn laid(&self, width: u16, height: u16, composer: Composer<'_>) -> (Buffer, super::Layout) {
            let screen = self.screen(composer);
            let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
            let layout = super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
            (buf, layout)
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

    /// The last cell of the chat column's rule (row 1): where the drawn
    /// column ends. The frames disagree by one at the right edge, so the
    /// header is judged against the rule drawn with it.
    fn rule_end(buf: &Buffer) -> Option<u16> {
        (0..buf.area.width)
            .rev()
            .find(|x| buf[(*x, 1)].symbol() == "─")
    }

    /// A stopped session is one row: `stopped 2d` at the right end, and no
    /// second line counting its seats.
    #[test]
    fn a_stopped_row_says_how_long_it_has_been_stopped_on_its_one_row() {
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
        let first = spot(&buf, " s1").expect("the first stopped row").0;
        assert!(
            line(&buf, first)[..44]
                .concat()
                .trim_end()
                .ends_with("stopped 2d"),
            "{:?}",
            line(&buf, first).concat()
        );
        assert_eq!(spot(&buf, " s2").map(|at| at.0), Some(first + 1));
        assert_eq!(spot(&buf, "not running"), None, "no second line");
    }

    /// Frames calm r18 (160x45) and needs-you r17 (100x30): the row the tab
    /// row held now carries the static `Agents N` heading at the sidebar's
    /// left edge, bold, with no `Overview` or `Tab` word. 90x20 is the
    /// narrowest sidebar (rule 34).
    #[test]
    fn the_agents_heading_sits_where_the_frames_put_the_tab_row() {
        for (width, height) in [(160, 43), (100, 28), (90, 20)] {
            let mut shot = Shot::new();
            shot.agents = Some(match sweep_seats() {
                Facts::Seats { id, agents } => Facts::Seats {
                    id,
                    agents: agents.into_iter().take(6).collect(),
                },
                other => other,
            });
            let buf = shot.draw(width, height, READ_ONLY);
            let size = format!("{width}x{height}");
            let (y, x) = spot(&buf, "Agents 6").expect("a heading");
            assert_eq!(x, 2, "{size}");
            assert!(buf[(2, y)].modifier.contains(Modifier::BOLD), "{size}");
            assert_eq!(spot(&buf, "Overview"), None, "{size}");
            assert_eq!(spot(&buf, "Tab  │"), None, "{size}");
        }
    }

    /// At 100x30 the chat column is narrow: the header's reason keeps its age
    /// and stays inside the column, cut with an ellipsis before the age.
    #[test]
    fn a_narrow_header_reason_keeps_its_age_inside_the_column() {
        let mut shot = Shot::new();
        shot.overview.open = vec![Open {
            seat: "ghost".to_owned(),
            text: "resume at scope checks after the fixtures land on main".to_owned(),
            age_secs: Some(360),
        }];
        let buf = shot.draw(100, 30, READ_ONLY);
        let header = line(&buf, 0).concat();
        assert!(header.contains("⚠ ghost: resume at sc"), "{header:?}");
        let end = rule_end(&buf).expect("the chat column's rule");
        let (_, x) = spot(&buf, "· 6m").expect("the age is kept");
        assert!(x + 4 <= usize::from(end), "inside the column: {header:?}");
        assert!(header.contains("… · 6m"), "{header:?}");
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
        // A viewed seat replaces the column: no chat hint under its screen,
        // and the chat scroll is kept for the return.
        let viewing = shot.draw(160, 45, Composer::Viewing { seat: "colead" });
        assert_eq!(spot(&viewing, HINT), None, "viewing suppresses the hint");
        assert_eq!(shot.model.pages(), 1, "the chat scroll is retained");
        let back = shot.draw(160, 45, READ_ONLY);
        assert!(
            spot(&back, HINT).is_some(),
            "the hint returns with the chat"
        );
    }

    /// The header stays inside the chat column (frames: header text never
    /// passes the rule under it). A name at the grammar's 128 and a long
    /// pair middle both stay inside it.
    #[test]
    fn a_long_header_stays_inside_the_chat_column() {
        let mut shot = Shot::new();
        shot.entry = SessionEntry::new("n".repeat(128), Status::Running);
        let buf = shot.draw(160, 45, READ_ONLY);
        let end = rule_end(&buf).expect("the rule");
        assert!(
            last_ink(&buf, 0, 160).is_some_and(|x| x <= end),
            "a long name"
        );
        let mut shot = Shot::new();
        shot.pair = vec!["h".repeat(60), "g".repeat(60)];
        let buf = shot.draw(160, 45, READ_ONLY);
        let (y, _) = spot(&buf, "hhh").expect("the long pair middle");
        assert_eq!(y, 0);
        let end = rule_end(&buf).expect("the rule");
        assert!(
            last_ink(&buf, 0, 160).is_some_and(|x| x <= end),
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
        let header = line(&buf, 0).concat();
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
            let header = line(&buf, 0).concat();
            let shown = header.trim_end();
            assert!(
                shown.contains(&shot.pair.join(" + ")),
                "the middle is whole: {header:?}"
            );
            assert_eq!(last_ink(&buf, 0, 160), rule_end(&buf), "{header:?}");
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
    /// status, so the app pane is 160x43 (100x28). The sidebar is panel
    /// #313335 on every row and everything from its rule on is base
    /// #2B2B2B, the first and last rows included.
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
            assert_eq!(
                buf[(0, height - 1)].bg,
                panel,
                "{size}: the sidebar on the last row"
            );
            for (x, y) in [(rule, 10), (inside, 0), (inside, 10), (inside, height - 1)] {
                assert_eq!(buf[(x, y)].bg, chat, "{size}: the chat ground at ({x},{y})");
            }
        }
    }

    /// A body that does not fit gives its last row to a dim line naming the
    /// rows it hides; a body that fits names none (ruling C3).
    #[test]
    fn a_cut_body_names_its_hidden_rows_in_the_dim_hue() {
        let mut shot = Shot::new();
        let Facts::Seats { id, agents } = sweep_seats() else {
            panic!("the sweep's seats");
        };
        let agents = agents.into_iter().cycle().take(20).collect();
        shot.agents = Some(Facts::Seats { id, agents });
        let buf = shot.draw(100, 21, READ_ONLY);
        assert_eq!(
            spot(&buf, "↓ 26 more rows"),
            Some((20, 2)),
            "the body's last row"
        );
        let dim = super::colour(crate::theme::Palette::DARCULA.dim);
        assert_eq!(buf[(2, 20)].fg, dim);
        let tall = shot.draw(100, 40, READ_ONLY);
        assert_eq!(
            spot(&tall, "↓ 7 more rows"),
            Some((39, 2)),
            "the wrapped body no longer fits"
        );
        shot.fleet = frame_fleet();
        // The smallest sidebar frame gives the body six rows, the last of
        // them the line naming what it hides.
        let smallest = shot.draw(100, 20, READ_ONLY);
        assert_eq!(spot(&smallest, "↓ 35 more rows").map(|at| at.0), Some(19));
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

    /// The rule stands at 44 on every row, the hint row included: the
    /// writing hint stands in the chat column right of it.
    #[test]
    fn the_rule_runs_the_full_height_beside_the_hint_row() {
        let draft = View {
            rows: vec!["a short draft".to_owned()],
            cursor_row: 0,
            before: String::new(),
            anchor: String::new(),
        };
        let composing = Composer::Home {
            home: "api",
            view: Some(&draft),
            draft: "a short draft",
        };
        let buf = Shot::new().draw(160, 43, composing);
        for y in 0..43 {
            assert_eq!(buf[(44, y)].symbol(), "│", "row {y}");
        }
        let keys = line(&buf, 42).concat();
        assert!(
            keys.contains("Enter sends · Alt+Enter ^J newline · Esc browses"),
            "the writing hint stands on the row: {keys:?}"
        );
    }

    /// The hint row names `^C clears` over typed text and never over an empty
    /// composer, nor over a draft Esc kept (its hint is `Enter writes`); at a width that cannot hold it whole it drops.
    #[test]
    fn the_hint_row_names_the_clear_key_only_over_text() {
        let shot = Shot::new();
        let (full, empty) = (drafted(&["hello"]), drafted(&[""]));
        for (composer, named, words) in [
            (writing(&full, "hello"), true, "typed text"),
            (writing(&empty, ""), false, "an empty composer"),
            (
                browsing("kept"),
                false,
                "a kept draft in browse (Enter writes)",
            ),
            (browsing(""), false, "no draft"),
        ] {
            let buf = shot.draw(160, 43, composer);
            let keys = line(&buf, 42).concat();
            assert_eq!(keys.contains("^C clears"), named, "{words}: {keys:?}");
        }
        let narrow = shot.draw(60, 43, writing(&full, "hello"));
        let keys = line(&narrow, 42).concat();
        assert!(!keys.contains("^C clears"), "dropped, not cut: {keys:?}");
    }

    /// One summary row names live agents, working and needs-you, using short
    /// labels where the sidebar cannot hold all full labels.
    #[test]
    fn the_summary_band_names_each_count_at_any_width() {
        let mut shot = sessions(9);
        for row in &mut shot.fleet.rows {
            row.counts = Counts::Marks(vec![(Mark::Working, false, 2)]);
        }
        shot.model = Model::new(&shot.fleet);
        for (width, expected) in [
            (90, "Agt 18 · Wrk 18 · Need 0"),
            (120, "Agt 18 · Wrk 18 · Need 0"),
            (200, "Agents 18 · Working 18 · Needs you 0"),
        ] {
            let (buf, layout) = declutter_frame(&shot, width, true);
            assert_eq!(declutter_line(&buf, &layout, 0).trim(), expected);
            assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
        }
    }

    /// Fixed labels keep a done-but-live agent separate from working.
    #[test]
    fn the_summary_words_agree_with_their_counts() {
        let mut shot = sessions(1);
        shot.fleet.rows[0].counts = Counts::Marks(vec![(Mark::Done, false, 1)]);
        shot.fleet.rows[0].needy = true;
        shot.model = Model::new(&shot.fleet);
        let (buf, layout) = declutter_frame(&shot, 160, true);
        assert_eq!(
            declutter_line(&buf, &layout, 0).trim(),
            "Agents 1 · Working 0 · Needs you 1"
        );
        assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
    }

    /// Narrow summaries keep complete digits by dropping labels first; icons
    /// off uses the corresponding ASCII glyphs.
    #[test]
    fn a_narrow_summary_band_drops_words_before_counts() {
        let mut shot = sessions(9);
        for row in &mut shot.fleet.rows {
            row.counts = Counts::Marks(vec![(Mark::Working, false, 2)]);
        }
        shot.fleet.rows[0].counts = Counts::Marks(vec![(Mark::Working, false, 123_456_789)]);
        shot.model = Model::new(&shot.fleet);
        for (icons, expected) in [
            (true, "◈ 123456805 ● 123456805 ⚠ 0"),
            (false, "@ 123456805 * 123456805 ! 0"),
        ] {
            let (buf, layout) = declutter_frame(&shot, 90, icons);
            assert_eq!(declutter_line(&buf, &layout, 0).trim(), expected);
            assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
        }
    }

    /// The quiet summary stays on the sidebar ground; only `Agents` keeps a
    /// raised band. A blank row separates the summary from the compact list.
    #[test]
    fn only_the_agents_heading_keeps_the_raised_panel_ground() {
        let mut shot = sessions(4);
        shot.agents = Some(seat_facts(&["lead"]));
        for name in ["darcula", "a", "b"] {
            let look = Look::read("on", name, "on", "on");
            let (buf, _) = framed(&shot, 160, Some(&look));
            let palette = look.palette;
            let (panel, base) = (super::colour(palette.panel), super::colour(palette.base));
            let rule = (0..160)
                .find(|x| buf[(*x, 0)].symbol() == "│")
                .expect("the rule");
            let heading = (2..45)
                .find(|y| line(&buf, *y).concat().contains("Agents"))
                .expect("the Agents heading");
            for x in 0..rule {
                assert_eq!(buf[(x, heading)].bg, panel, "{name} band ({x},{heading})");
            }
            for y in [0, 1, 2, 3, heading + 2, heading + 3] {
                for x in 0..rule - 2 {
                    assert_ne!(buf[(x, y)].bg, panel, "{name} row ({x},{y})");
                }
            }
            assert_eq!(buf[(5, 5)].bg, base, "{name}: a session row's ground");
        }
    }

    /// Summary counts keep regular text ink; labels and separators use dim
    /// ink on base. The Agents band keeps text ink on panel.
    #[test]
    fn the_summary_uses_regular_counts_and_dim_labels_on_base() {
        let mut shot = sessions(30);
        shot.fleet.rows[0].needy = true;
        shot.fleet.rows[29].needy = true;
        shot.agents = Some(seat_facts(&["lead"]));
        shot.model = Model::new(&shot.fleet);
        for name in ["darcula", "a", "b"] {
            let look = Look::read("on", name, "on", "on");
            let (buf, layout) = framed(&shot, 160, Some(&look));
            let palette = look.palette;
            let text = super::colour(palette.text);
            let dim = super::colour(palette.dim);
            assert_eq!(buf[(2, 0)].fg, dim, "{name}: Agents label");
            assert_eq!(buf[(9, 0)].fg, text, "{name}: agent count");
            for y in [0, 1] {
                for x in 0..44 {
                    assert!(
                        !buf[(x, y)].modifier.contains(Modifier::BOLD),
                        "{name}: regular summary at {x},{y}"
                    );
                }
            }
            let heading = (2..45)
                .find(|y| line(&buf, *y).concat().contains("Agents"))
                .expect("the Agents heading");
            assert_eq!(buf[(2, heading)].fg, text, "{name}: Agents heading text");
            let label = (2..layout.list.width)
                .find(|x| buf[(*x, 0)].symbol() == "N")
                .expect("Needs you label");
            assert_eq!(buf[(label, 0)].fg, dim, "{name}: Needs you label");
            assert_eq!(
                buf[(label + 10, 0)].symbol(),
                "2",
                "{name}: two needy sessions"
            );
            assert_eq!(
                buf[(label + 10, 0)].fg,
                super::colour(palette.needs_you),
                "{name}: needs-you count"
            );
            let mut accented = 0;
            for y in [0, 1, heading] {
                for x in 0..layout.list.width {
                    let cell = &buf[(x, y)];
                    if cell.symbol() == " " || cell.fg == text || (y < 2 && cell.fg == dim) {
                        continue;
                    }
                    assert!(
                        matches!(cell.symbol(), "↑" | "↓")
                            || (y == 0 && x == label + 10 && cell.symbol() == "2"),
                        "{name} ({x},{y}): {:?} wears {:?}",
                        cell.symbol(),
                        cell.fg
                    );
                    accented += 1;
                }
            }
            assert!(
                accented >= 1,
                "{name}: off-screen attention keeps its accent"
            );
            assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
        }
    }

    /// One blank row separates the two-row summary from the compact session
    /// list. Hit and selection rectangles begin at the first session.
    #[test]
    fn one_blank_row_separates_the_summary_from_the_session_list() {
        for count in [0, 1, 4, 30] {
            for (width, height) in [(90, 20), (120, 30), (220, 60)] {
                let (buf, layout) = sessions(count).laid(width, height, READ_ONLY);
                let left = |y| line(&buf, y)[..usize::from(layout.list.width)].concat();
                let frame = format!("{count} sessions at {width}x{height}");
                assert!(
                    left(0).trim_start().starts_with(['A', '◈', '@', '⚠', '!']),
                    "{frame}"
                );
                let first = if count == 0 { "No sessions." } else { "s00" };
                assert!(left(1).trim().is_empty(), "{frame}: spacer");
                assert!(left(2).contains(first), "{frame}: row 2 is {:?}", left(2));
                assert_eq!(layout.list.y, 2, "{frame}");
            }
        }
    }

    /// The closed hint row carries the composer's hint, the mode's keys and
    /// the version/gear; held has no hint and no keys. A panel's footer
    /// keeps its mode word and its one hint.
    #[test]
    fn the_hint_row_and_the_panel_footer_carry_at_most_one_hint() {
        let mut shot = Shot::new();
        let draft = drafted(&["a draft"]);
        let tail = format!("{} \u{2699}", crate::version_line());
        let named = [
            "Enter",
            "Esc browse",
            "^C",
            "qq",
            "oo",
            "1-9",
            "Tab",
            "j/k",
            "n/p",
            "PgUp",
            "settings",
            "! next",
            "Esc home",
            "help",
        ];
        let refused = Composer::Held {
            why: "owned elsewhere",
        };
        let cases = [
            (browsing("kept"), "draft kept · Enter writes · ? keys"),
            (READ_ONLY, "? keys"),
            (
                writing(&draft, "a draft"),
                "Enter sends · Alt+Enter ^J newline · Esc browses · ^C clears",
            ),
            (refused, ""),
        ];
        for (composer, words) in cases {
            let (row, _) = shot.footer_at(100, composer, true);
            let head = row
                .strip_suffix(tail.as_str())
                .expect("version and gear end it");
            assert_eq!(head.trim_end(), format!("  {words}").trim_end(), "{words}");
            if words.is_empty() {
                assert!(named.iter().all(|key| !row.contains(key)), "{row}");
            }
        }
        shot.model.open_settings(1);
        let (row, _) = shot.footer_at(100, browsing(""), false);
        let head = row
            .strip_suffix(tail.as_str())
            .expect("version and gear end it");
        assert_eq!(head.trim_end(), "  settings   Esc close");
    }

    /// Narrower than the floor (the app never draws it, the row still has
    /// its order): the version goes first, then the gear, and the gear is
    /// the settings click target for as long as it is drawn.
    #[test]
    fn narrowing_the_hint_row_drops_the_version_then_the_gear() {
        let shot = Shot::new();
        let gear = "\u{2699}";
        let tail = format!("{} {gear}", crate::version_line());
        let head = "  Enter writes · ? keys";
        let full = head.chars().count() + 2 + tail.chars().count();
        let drop = |width: usize, drawn: &str| {
            let at = u16::try_from(width).unwrap_or(0);
            let (row, layout) = shot.footer_at(at, browsing(""), true);
            assert!(row.starts_with(head), "{width}: {row:?}");
            assert_eq!(
                row.trim_start_matches(head).trim(),
                drawn,
                "{width}: {row:?}"
            );
            let click = Mouse {
                kind: MouseKind::Click,
                column: at - 1,
                row: 0,
            };
            assert_eq!(
                matches!(layout.hit(click), Some(super::Hit::Settings)),
                !drawn.is_empty(),
                "{width}: the gear is the click target while drawn"
            );
        };
        drop(full + 5, &tail);
        drop(full, &tail);
        drop(full - 1, gear);
        drop(head.chars().count() + 3, gear);
        drop(head.chars().count() + 2, "");
    }

    /// A draft of `rows`, its cursor after the last row's cells.
    fn drafted(rows: &[&str]) -> View {
        View {
            rows: rows.iter().map(|row| (*row).to_owned()).collect(),
            cursor_row: rows.len().saturating_sub(1),
            before: rows.last().map(|row| (*row).to_owned()).unwrap_or_default(),
            anchor: String::new(),
        }
    }

    /// A composer writing `view`.
    fn writing<'a>(view: &'a View, draft: &'a str) -> Composer<'a> {
        Composer::Home {
            home: "api",
            view: Some(view),
            draft,
        }
    }

    /// A composer browsing with `draft` kept.
    fn browsing(draft: &str) -> Composer<'_> {
        Composer::Home {
            home: "api",
            view: None,
            draft,
        }
    }

    /// Row `y` from column `x` to the end, trailing blanks trimmed.
    fn row_from(buf: &Buffer, y: u16, x: usize) -> String {
        line(buf, y)[x..].concat().trim_end().to_owned()
    }

    fn column(x: usize) -> u16 {
        u16::try_from(x).unwrap_or(0)
    }

    /// The composer is the address on its own row and the draft the rows
    /// under it, from the column's left edge: the cursor follows the last
    /// row, and writing carries its keys hint on the keys row.
    #[test]
    fn a_draft_sits_under_its_address_from_the_columns_left_edge() {
        let shot = Shot::new();
        let draft = drafted(&["first words", "second"]);
        for (width, height) in [(100, 30), (60, 20)] {
            let size = format!("{width}x{height}");
            let (buf, layout) = shot.laid(width, height, writing(&draft, "first words second"));
            let (ay, ax) = spot(&buf, "to api").expect("the address");
            assert_eq!(ay, height - 4, "{size}: over two draft rows and the hint");
            assert_eq!(row_from(&buf, ay, ax), "to api", "{size}: alone on its row");
            assert_eq!(spot(&buf, "first words"), Some((ay + 1, ax)), "{size}");
            assert_eq!(spot(&buf, "second"), Some((ay + 2, ax)), "{size}");
            assert_eq!(
                layout.cursor,
                Some(Position::new(column(ax + 6), ay + 2)),
                "{size}: after the last row"
            );
            assert_eq!(buf[(column(ax), ay - 1)].symbol(), "─", "{size}: the rule");
            let all: String = (0..height).map(|y| line(&buf, y).concat()).collect();
            assert!(all.contains("Enter sends"), "{size}: the writing hint");
        }
    }

    /// Browsing keeps the address on its row and shows a kept draft on the
    /// input row under it, with its hint; with none the input row is blank.
    #[test]
    fn browse_shows_a_kept_draft_on_the_input_row() {
        let shot = Shot::new();
        for (draft, hint) in [
            ("a kept thought", "draft kept · Enter writes"),
            ("", "Enter writes"),
        ] {
            let (buf, layout) = shot.laid(100, 30, browsing(draft));
            let (ay, ax) = spot(&buf, "to api").expect("the address");
            assert_eq!(ay, 27, "{draft:?}");
            assert_eq!(row_from(&buf, ay, ax), "to api", "{draft:?}");
            assert_eq!(
                row_from(&buf, ay + 1, ax),
                draft,
                "{draft:?}: the input row"
            );
            let shown = row_from(&buf, ay + 2, ax);
            assert!(
                shown.starts_with(&format!("{hint} · ? keys")),
                "{draft:?}: the hint row: {shown:?}"
            );
            assert!(
                shown.ends_with(&format!("{} \u{2699}", crate::version_line())),
                "{draft:?}: version and gear end it: {shown:?}"
            );
            assert_eq!(layout.cursor, None, "{draft:?}");
        }
    }

    /// A read-only, held or home-less composer keeps the block the others
    /// do: its line on the address row, the input row blank under it.
    #[test]
    fn a_quiet_composer_keeps_the_block_and_leaves_the_input_row_blank() {
        let shot = Shot::new();
        for (composer, words) in [
            (READ_ONLY, "read-only · owned elsewhere"),
            (
                Composer::Held {
                    why: "owned elsewhere",
                },
                "not writing: owned elsewhere · Esc browses",
            ),
            (
                Composer::NoHome,
                "read-only · no home session: ae app <session> picks one",
            ),
        ] {
            let (buf, _) = shot.laid(100, 30, composer);
            let (ay, ax) = spot(&buf, words).expect("the quiet line");
            assert_eq!(ay, 27, "{words}");
            assert_eq!(row_from(&buf, ay, ax), words, "{words}");
            assert_eq!(row_from(&buf, ay + 1, ax), "", "{words}: the input row");
            let shown = row_from(&buf, ay + 2, ax);
            assert_eq!(
                shown.starts_with("? keys"),
                !words.starts_with("not writing"),
                "{words}: the hint row: {shown:?}"
            );
            assert!(
                shown.ends_with(&format!("{} \u{2699}", crate::version_line())),
                "{words}: the tail stands: {shown:?}"
            );
        }
    }

    /// A pending line takes the hint row, in full ink; writing otherwise
    /// shows its keys there.
    #[test]
    fn a_note_takes_the_hint_row_and_writing_shows_its_keys() {
        let mut shot = Shot::new();
        let draft = drafted(&["hello"]);
        let (buf, _) = shot.laid(100, 30, writing(&draft, "hello"));
        let (ay, ax) = spot(&buf, "to api").expect("the address");
        assert_eq!(ay, 27);
        assert!(
            row_from(&buf, 29, ax).starts_with("Enter sends"),
            "the writing keys on the hint row"
        );
        shot.model.set_note(Some("q again to quit".to_owned()));
        let (buf, _) = shot.laid(100, 30, writing(&draft, "hello"));
        assert_eq!(spot(&buf, "q again to quit"), Some((29, ax)));
    }

    /// A note that fills the hint row of the widest pane leaves no room for
    /// the keys after it: the row draws without them, and without wrapping
    /// the column arithmetic past the u16 edge.
    #[test]
    fn a_full_hint_row_at_the_widest_pane_drops_the_keys() {
        let mut shot = Shot::new();
        shot.model.set_note(Some("n".repeat(70_000)));
        let (buf, layout) = shot.laid(u16::MAX, 28, READ_ONLY);
        assert_eq!(buf[(60_000, 27)].symbol(), "n", "the note fills the row");
        assert_eq!(
            buf[(65_532, 27)].symbol(),
            "n",
            "the note's last cell stands"
        );
        assert_eq!(spot(&buf, "? keys"), None, "no room for the keys");
        assert_eq!(
            spot(&buf, &crate::version_line()),
            None,
            "no room for the version either"
        );
        let click = Mouse {
            kind: MouseKind::Click,
            column: u16::MAX - 1,
            row: 27,
        };
        assert!(
            !matches!(layout.hit(click), Some(super::Hit::Settings)),
            "no gear, no settings target"
        );
    }

    /// A click anywhere from the address row to the note row opens the
    /// composer; the rule above and the blank row below are not it.
    #[test]
    fn the_compose_target_spans_the_address_the_draft_rows_and_the_note_row() {
        let shot = Shot::new();
        let draft = drafted(&["one", "two"]);
        let (buf, layout) = shot.laid(100, 30, writing(&draft, "one two"));
        let (ay, ax) = spot(&buf, "to api").expect("the address");
        let compose = |row| {
            let click = Mouse {
                kind: MouseKind::Click,
                column: column(ax),
                row,
            };
            matches!(layout.hit(click), Some(super::Hit::Compose))
        };
        assert!(!compose(ay - 1), "the rule");
        for row in ay..ay + 4 {
            assert!(compose(row), "row {row}: address, two draft rows, note");
        }
        assert!(!compose(ay + 4), "the blank row");
    }

    /// At the 40x8 floor the lane keeps one row: both rules are drawn,
    /// every chat row is whole, and a scrolled lane draws no newer-turns
    /// hint over the header.
    #[test]
    fn the_floor_size_keeps_every_chat_row_whole() {
        let mut shot = Shot::new();
        shot.lane = turns(150);
        shot.model.wheel(true, 10, 100);
        let (buf, _) = shot.laid(40, 8, browsing(""));
        assert_eq!(spot(&buf, "to api"), Some((5, 2)));
        assert_eq!(spot(&buf, "Enter writes"), Some((7, 2)));
        // The sidebar warning rides the header rule's left; the rule stands
        // past it, and the composer's own rule stands over the address.
        assert_eq!(buf[(35, 1)].symbol(), "─", "the header rule");
        assert_eq!(buf[(2, 4)].symbol(), "─", "the composer's rule");
        assert!(line(&buf, 0).concat().contains("api"), "the header");
        assert_eq!(
            spot(&buf, "newer turns"),
            Some((3, 4)),
            "the hint under the lane's one row, never over the header"
        );
        assert!(
            row_from(&buf, 7, 0)
                .trim_start()
                .starts_with("Enter writes · ? keys"),
            "the hint row"
        );
    }

    /// A tall draft leaves the lane its floor: at 60x20 the composer grows
    /// to ten draft rows and an eight-row draft leaves the lane r2-r7, and
    /// no height from 13 up lets the biggest draft take the lane under it.
    #[test]
    fn a_tall_draft_leaves_the_lane_its_floor() {
        let mut shot = Shot::new();
        shot.lane = turns(40);
        let area = Rect::new(0, 0, 60, 20);
        assert_eq!(
            super::composer_pane(area),
            11,
            "ten draft rows and one kept back"
        );
        let draft = drafted(&["row"; 8]);
        let (buf, layout) = shot.laid(60, 20, writing(&draft, "row"));
        let (ay, ax) = spot(&buf, "to api").expect("the address");
        assert_eq!(ay, 10);
        assert_eq!(buf[(column(ax), 9)].symbol(), "─", "the rule");
        assert_eq!(layout.page_rows, 6);
        assert_eq!(
            spot(&buf, "turn 39"),
            Some((7, ax + 2)),
            "the lane's last row"
        );
        assert_eq!(row_from(&buf, 8, ax), "", "the blank row over the rule");
        for height in 13..=60 {
            let rows = super::composer_pane(Rect::new(0, 0, 60, height)) - 1;
            let draft = drafted(&["x"].repeat(rows));
            let (_, layout) = shot.laid(60, height, writing(&draft, "x"));
            assert!(
                layout.page_rows >= usize::from(super::LANE_FLOOR),
                "{height}: {} lane rows",
                layout.page_rows
            );
        }
    }

    /// The address costs the draft no cell: a name that fills the column is
    /// clipped on its own row, the draft keeps every cell of the one under
    /// it, and the cursor stops on the last of them.
    #[test]
    fn a_long_address_is_clipped_on_its_row_and_the_draft_keeps_the_column() {
        let shot = Shot::new();
        let long = "h".repeat(60);
        let area = Rect::new(0, 0, 60, 20);
        let full = "x".repeat(super::draft_width(area, super::Split::default()));
        let draft = drafted(&[full.as_str()]);
        for home in ["api", long.as_str()] {
            let composer = Composer::Home {
                home,
                view: Some(&draft),
                draft: &full,
            };
            let (buf, layout) = shot.laid(60, 20, composer);
            let (ay, ax) =
                spot(&buf, &format!("to {}", &home[..home.len().min(8)])).expect("the address");
            assert_eq!(ay, 17, "{home}");
            assert_eq!(row_from(&buf, ay + 1, ax), full, "{home}: every cell");
            assert_eq!(
                layout.cursor,
                Some(Position::new(column(ax + full.len() - 1), ay + 1)),
                "{home}: the last cell"
            );
        }
    }

    /// The cells a draft has are the column's: from its left edge to the
    /// rule's end, with nothing taken for the address (frames calm, needs-you).
    #[test]
    fn a_draft_has_the_columns_cells_from_its_left_edge_to_its_rule_end() {
        for (width, height) in [(160, 43), (100, 28)] {
            let buf = Shot::new().draw(width, height, READ_ONLY);
            let end = rule_end(&buf).expect("the rule");
            let (_, left) = spot(&buf, "read-only").expect("the quiet line");
            let area = Rect::new(0, 0, width, height);
            let cells = super::draft_width(area, super::Split::default());
            assert_eq!(left + cells - 1, usize::from(end), "{width}x{height}");
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
            let header = line(&shot.draw(width, 18, READ_ONLY), 0).concat();
            let header = header.trim_end();
            assert!(header.ends_with("active 1m"), "{width}x18: {header:?}");
            assert_eq!(
                header.contains("feat/auth-v2  ·  "),
                shown,
                "{width}x18: {header:?}"
            );
        }
    }

    /// Frame needs-you 100x30: four sessions sit one row apart, each a single
    /// line, measured from the first.
    #[test]
    fn four_sessions_keep_one_row_each_at_100x28() {
        let mut shot = Shot::new();
        shot.fleet.rows = (1..=4)
            .map(|at| row(&format!("s{at}"), at, false))
            .collect();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(100, 28, READ_ONLY);
        let ys: Vec<u16> = (1..=4)
            .map(|at| spot(&buf, &format!(" s{at} ")).expect("a session row").0)
            .collect();
        assert_eq!(ys, [ys[0], ys[0] + 1, ys[0] + 2, ys[0] + 3], "{ys:?}");
    }

    /// The window a long fleet shows. A 100x28 pane's list is 12 rows (two
    /// thirds of the rows it shares with the tab body, never fewer than 11),
    /// one per session with the more row inside it, so sixteen sessions show
    /// 11 and `↓ 5 more`; at 160x43 it is 22, so thirty show 21 and `↓ 9 more`.
    #[test]
    fn a_long_fleet_shows_the_window_the_frames_do() {
        for (width, height, count, shown, more) in
            [(100, 28, 16, 11, "↓ 5 more"), (160, 43, 30, 21, "↓ 9 more")]
        {
            let mut shot = Shot::new();
            shot.fleet.rows = (1..=count)
                .map(|at| row(&format!("s{at}"), at, false))
                .collect();
            shot.model = Model::new(&shot.fleet);
            let buf = shot.draw(width, height, READ_ONLY);
            let size = format!("{width}x{height}");
            for at in 1..=count {
                let drawn = spot(&buf, &format!(" s{at} ")).is_some();
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
            assert!(width < 160 || y <= 25, "{size}: the more row at {y}");
            let last = spot(&buf, &format!(" s{shown} "))
                .expect("the last shown")
                .0;
            assert_eq!(
                y,
                last + 1,
                "{size}: the more row follows the last session's row"
            );
            let tabs = (2..buf.area.height)
                .find(|y| line(&buf, *y).concat().contains("Agents"))
                .expect("the heading");
            assert_eq!(tabs, y + 2, "{size}: one blank row, then the heading");
        }
        let mut empty = Shot::new();
        empty.fleet.rows.clear();
        empty.model = Model::new(&empty.fleet);
        let buf = empty.draw(160, 43, READ_ONLY);
        let none = spot(&buf, "No sessions.").expect("the empty list").0;
        assert_eq!(
            (2..buf.area.height).find(|y| line(&buf, *y).concat().contains("Agents")),
            Some(none + 2),
            "one blank row, then the heading"
        );
        // The more row counts the needy rows it hides, above and below.
        for (presses, more) in [
            (0, "↓ 5 more, ⚠ 2"),
            (15, "↑ 5 more, ⚠ 1"),
            (12, "↑ 2 · ↓ 3 more, ⚠ 3"),
        ] {
            let mut shot = Shot::new();
            shot.fleet.rows = (1..=16)
                .map(|at| row(&format!("s{at}"), at, matches!(at, 2 | 14 | 15)))
                .collect();
            shot.fleet.home = Some("s1".to_owned());
            shot.model = Model::new(&shot.fleet);
            for _ in 0..presses {
                shot.key(crate::app::model::Key::Down);
            }
            let buf = shot.draw(100, 28, READ_ONLY);
            let (y, _) = spot(&buf, "more").expect("the more row");
            let drawn = line(&buf, y)[..34].concat();
            assert_eq!(drawn.trim(), more, "{presses} presses down");
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

    /// Several needy rows use a count and plain location suffixes; the
    /// arrows stand alone at the sidebar's end, with no `!` key marker.
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
        let attention = line(&buf, 0).concat();
        assert!(
            !attention.contains("infra"),
            "no attention prose: {attention:?}"
        );
        let mut shot = Shot::new();
        shot.fleet.rows = vec![
            row("a", 1, false),
            row("needy-0010", 2, true),
            row("needy-00011", 3, true),
        ];
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        assert_eq!(buf[(41, 0)].symbol(), " ", "no key marker at end-1");
        assert_eq!(
            buf[(40, 0)].symbol(),
            " ",
            "a blank before it: {:?}",
            line(&buf, 0).concat()
        );
        // (c) several long names still leave the end cells blank.
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
            buf[(40, 0)].symbol(),
            " ",
            "the end cells stay blank: {:?}",
            line(&buf, 0).concat()
        );
        // (d) only needy rows outside the view contribute to the suffix.
        let mut shot = Shot::new();
        shot.fleet.rows = (1..=30)
            .map(|at| {
                let name = match at {
                    3 => "infra".to_owned(),
                    29 => "billing".to_owned(),
                    30 => "ops".to_owned(),
                    _ => format!("s{at}"),
                };
                let mut row = row(&name, at, matches!(at, 3 | 29 | 30));
                if at == 30 {
                    row.mark = Mark::Dead;
                }
                row
            })
            .collect();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        assert_eq!(buf[(41, 0)].symbol(), "↓", "off-screen attention below");
        assert_eq!(buf[(42, 0)].symbol(), " ", "no key marker");
    }

    /// Frame agents r20-r35: a seat's state is right-aligned at the sidebar's
    /// end with a blank before it, never against the name. The state word is
    /// the last thing to go: a name too long for the room is cut with an
    /// ellipsis instead. At 160 the end is col 42: `● ` plus a 30-cell name
    /// ends at col 33, `working` takes cols 35-41.
    #[test]
    fn an_agent_state_keeps_the_right_edge_and_the_name_gives_way() {
        for (cells, cut) in [(30, false), (31, true), (60, true)] {
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
            let buf = shot.draw(160, 43, READ_ONLY);
            let (y, _) = spot(&buf, "nnnn").expect("the seat row");
            let row = line(&buf, y)[..44].concat();
            assert!(row.trim_end().ends_with(" working"), "{row:?}");
            assert_eq!(row.contains('…'), cut, "a {cells}-cell name: {row:?}");
            let name = row.find('n').expect("the name");
            let state = row.find("working").expect("the state");
            assert!(name < state, "{row:?}");
        }
    }

    /// Frame many 160: a session that needs you keeps its ⚠ mark and draws no
    /// left edge and no question line. Column 0 holds the selection's `▸` and
    /// nothing else, on the selected session's one row. The summary row
    /// and its spacer put infra at r4, billing at r7 and ops at r9.
    #[test]
    fn a_needy_session_draws_no_bar_and_keeps_its_mark_without_a_question_row() {
        let mut shot = Shot::new();
        shot.fleet = frame_fleet();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        let col0: Vec<String> = (0..43).map(|y| buf[(0, y)].symbol().to_owned()).collect();
        assert!(
            !col0.iter().any(|symbol| symbol == "│"),
            "no edge: {col0:?}"
        );
        let selected = shot.model.selected().expect("a selection");
        let at = shot
            .fleet
            .rows
            .iter()
            .position(|row| row.name == selected)
            .expect("the selected row");
        let drawn: Vec<usize> = (0..43)
            .filter(|y| buf[(0, *y)].symbol() == "▸")
            .map(usize::from)
            .collect();
        assert_eq!(drawn, [2 + at], "the selection's glyph, one row");
        for (name, y) in [("infra", 4), ("billing", 7), ("ops", 9)] {
            let first = line(&buf, y)[..44].concat();
            assert!(first.contains(name), "{name}: {first:?}");
        }
        let infra = line(&buf, 4)[..44].concat();
        assert!(infra.contains('⚠'), "its mark: {infra:?}");
        for y in 0..43 {
            let sidebar = line(&buf, y)[..44].concat();
            assert!(
                !sidebar.contains("a question"),
                "no question row: {sidebar:?}"
            );
        }
    }

    /// The selected session wears what the viewed seat wears: the selected
    /// ground over its one row out to the body's end, the glyph in the
    /// selected ink, in every palette; the next session keeps the sidebar's
    /// ground and a needy one is no different from a quiet one. On the
    /// ground only the selected ink and the marks' accents are drawn, so the
    /// pairs are the ones `theme`'s contrast bar already measures.
    #[test]
    fn the_selected_session_sits_on_the_selected_ground_in_every_palette() {
        let mut shot = sessions(3);
        shot.fleet.rows[1].needy = true;
        shot.fleet.rows[1].mark = Mark::NeedsYou;
        shot.model = Model::new(&shot.fleet);
        for name in ["darcula", "a", "b"] {
            let look = Look::read("on", name, "on", "on");
            let (buf, _) = framed(&shot, 160, Some(&look));
            let palette = look.palette;
            let rule = (0..160)
                .find(|x| buf[(*x, 0)].symbol() == "│")
                .expect("the rule");
            let (ground, ink) = (
                super::colour(palette.selected),
                super::colour(palette.selected_ink),
            );
            let glyph = &buf[(0, 2)];
            assert_eq!(glyph.symbol(), "▸", "{name}");
            assert_eq!(
                (glyph.fg, glyph.modifier.contains(Modifier::BOLD)),
                (ink, true)
            );
            for x in 0..rule - 2 {
                assert_eq!(buf[(x, 2)].bg, ground, "{name} ({x},2)");
            }
            for x in 0..rule - 2 {
                let cell = &buf[(x, 2)];
                if cell.symbol() == " " {
                    continue;
                }
                let accents = Mark::BY_URGENCY.map(|mark| super::colour(palette.accent(mark)));
                let ok = cell.fg == ink || accents.contains(&cell.fg);
                assert!(ok, "{name}: {:?} {:?}", cell.symbol(), cell.fg);
            }
            for y in [3, 4, 5, 6, 7, 8] {
                for x in 0..rule - 2 {
                    assert_ne!(buf[(x, y)].bg, ground, "{name}: row {y} col {x}");
                }
            }
        }
    }

    /// No colour: the selected session is reversed over its one row, as a
    /// seat is, and no other session row is.
    #[test]
    fn the_selected_session_is_reversed_with_no_colour() {
        let shot = sessions(3);
        let (buf, _) = framed(&shot, 160, None);
        let reversed = |y| buf[(8, y)].modifier.contains(Modifier::REVERSED);
        assert!(reversed(2), "the selected session");
        assert!(!reversed(3) && !reversed(4), "the next session");
    }

    /// A long name is cut with an ellipsis beside the shortest tally, so the
    /// row stays one line inside the rule.
    #[test]
    fn a_long_session_name_is_cut_beside_its_shortest_tally() {
        let mut shot = Shot::new();
        shot.fleet.rows = vec![row(&"n".repeat(60), 1, false)];
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        let expected = "n".repeat(32) + "… ●3 …";
        assert_eq!(line(&buf, 2)[4..42].concat(), expected);
        assert!(!line(&buf, 3)[..42].concat().contains("✓B12"), "one row");
        assert_eq!(buf[(44, 2)].symbol(), "│");
    }

    /// The home glyph follows the name after one blank, on that row alone; a
    /// 28-cell name keeps it beside the tally, a 29-cell one drops it first.
    #[test]
    fn this_window_marks_the_home_row_alone_inside_the_name_line() {
        let mut shot = Shot::new();
        shot.fleet = frame_fleet();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        let sidebar = |buf: &Buffer, y: u16| line(buf, y)[..44].concat();
        let tagged: Vec<u16> = (0..43)
            .filter(|y| sidebar(&buf, *y).contains('◂'))
            .collect();
        assert_eq!(tagged, [2], "the home row alone");
        assert!(
            sidebar(&buf, 2).starts_with("▸ ● api ◂"),
            "{:?}",
            sidebar(&buf, 2)
        );
        assert!(
            !sidebar(&buf, 2).contains("this window"),
            "a glyph, no words"
        );
        for (cells, shown) in [(28, true), (29, false)] {
            let mut shot = Shot::new();
            shot.fleet.rows = vec![row(&"n".repeat(cells), 1, false)];
            shot.model = Model::new(&shot.fleet);
            let first = sidebar(&shot.draw(160, 43, READ_ONLY), 2);
            assert_eq!(first.contains('◂'), shown, "a {cells}-cell name: {first:?}");
            assert!(first.contains("✓B12"), "the tally stays: {first:?}");
        }
    }

    /// A crowded status draws no second line: the questions a session holds
    /// are told in the chat header, never under its row.
    #[test]
    fn a_session_draws_no_question_line() {
        let mut shot = Shot::new();
        shot.fleet = frame_fleet();
        shot.model = Model::new(&shot.fleet);
        let buf = shot.draw(160, 43, READ_ONLY);
        for y in 2..11 {
            let cells = line(&buf, y);
            assert!(!cells[..44].concat().contains("a question"), "row {y}");
            assert_eq!(cells[42..44].concat(), "  ");
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

    /// The undragged list is two thirds of the rows it shares with the tab
    /// body, and never fewer than the 18 (from 40 high) or 11 rows it once had.
    #[test]
    fn the_undragged_list_takes_two_thirds_and_never_fewer_than_it_had() {
        for (height, rows) in [
            (20, 10),
            (24, 11),
            (28, 12),
            (30, 13),
            (40, 20),
            (45, 23),
            (62, 34),
            (65535, 43683),
        ] {
            let area = Rect::new(0, 0, 160, height);
            assert_eq!(
                super::list_rows(area, crate::app::model::Split::default(), 99),
                (rows, false)
            );
        }
    }

    /// Ruling 5b with the summary spacer: a 100x20 pane of 13 sessions draws
    /// its list rule on row 14 (nine shown, the more row, tabs on 13). A press there
    /// moved nowhere keeps it; dragged up and back down past it, it stops on
    /// 14, never 15.
    #[test]
    fn a_short_pane_keeps_its_undragged_list_rule_reachable() {
        let mut shot = sessions(13);
        let first = shot.draw(100, 20, READ_ONLY);
        assert_eq!(horizontal(&first), Some(14), "the undragged rule");
        let still = drag(&mut shot, &first, Edge::List, 14);
        assert_eq!(horizontal(&still), Some(14), "a press moved nowhere");
        let up = drag(&mut shot, &still, Edge::List, 9);
        assert_eq!(horizontal(&up), Some(9));
        let down = drag(&mut shot, &up, Edge::List, 16);
        assert_eq!(horizontal(&down), Some(14), "never past the undragged rule");
    }

    /// Ruling 5a: the list keeps one session's row — one alone, a row and the
    /// more row among several — and its dragged-down stop keeps the row
    /// budget it had, the body taking the rows the frame gave back.
    #[test]
    fn a_dragged_list_keeps_one_session_and_its_row_budget() {
        for (count, least) in [(1, 5), (2, 6), (13, 6)] {
            let mut shot = sessions(count);
            let first = shot.draw(160, 45, READ_ONLY);
            let top = drag(&mut shot, &first, Edge::List, 0);
            assert_eq!(horizontal(&top), Some(least), "{count} sessions");
            let bottom = drag(&mut shot, &top, Edge::List, 44);
            // Border 36: body rows 37 to 44, the pane's last.
            assert_eq!(horizontal(&bottom), Some(36), "{count} sessions");
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
            assert_eq!(at(column, 0), Some((Edge::Sidebar, 44)), "column {column}");
        }
        assert_eq!(at(46, 0), None);
        assert_eq!(at(42, 0), None);
        assert_eq!(
            at(43, 1),
            Some((Edge::Sidebar, 44)),
            "the blank spacer beside the rule is no target"
        );
        assert_eq!(at(42, 1), None, "the spacer away from the rule");
        assert_eq!(at(43, 2), None, "a session row is a target");
        assert_eq!(at(44, 2), Some((Edge::Sidebar, 44)));
        for row in [rule - 1, rule, rule + 1] {
            assert_eq!(at(30, row), Some((Edge::List, rule)), "row {row}");
        }
        assert_eq!(at(30, rule + 2), None);
        assert_eq!(
            at(2, rule - 1),
            Some((Edge::List, rule)),
            "the heading is no target"
        );
        assert_eq!(at(44, rule), Some((Edge::Sidebar, 44)), "the corner");
        assert_eq!(at(43, rule), Some((Edge::List, rule)), "its own cell");
        assert_eq!(
            at(44, 44),
            Some((Edge::Sidebar, 44)),
            "the rule runs to the hint row"
        );
        assert_eq!(at(159, 44), None, "the gear is a target, not a border");
    }

    /// Fixed text width for geometry-only fixtures. This deliberately unmapped
    /// 13-cell word passes through unchanged; production vocabulary is pinned
    /// independently by `honestwords_spec`. These fixtures keep their measured
    /// label capacity, with the existing unknown-word Stale mark.
    const FIT_STATE: &str = "unmapped-word";

    /// Two seats in the Agents body, by name.
    fn seat_facts(names: &[&str]) -> Facts {
        let Facts::Seats { id, agents } = sweep_seats() else {
            unreachable!("the sweep draws seats")
        };
        let seat_of = |name: &&str| PickerAgent {
            name: (*name).to_owned(),
            state: FIT_STATE.to_owned(),
            ..agents[0].clone()
        };
        Facts::Seats {
            id,
            agents: names.iter().map(seat_of).collect(),
        }
    }

    /// One frame of `shot` at `width` x 45 in `look`.
    fn framed(shot: &Shot, width: u16, look: Option<&Look>) -> (Buffer, super::Layout) {
        framed_as(shot, width, look, READ_ONLY)
    }

    /// [`framed`] under `composer`.
    fn framed_as(
        shot: &Shot,
        width: u16,
        look: Option<&Look>,
        composer: Composer<'_>,
    ) -> (Buffer, super::Layout) {
        let screen = Screen {
            fleet: &shot.fleet,
            model: &shot.model,
            overview: &shot.overview,
            selected: Some(&shot.entry),
            pair: &shot.pair,
            agents: shot.agents.as_ref(),
            lane: &shot.lane,
            composer,
            look: look.copied(),
            zone: None,
            now: Timestamp::from_epoch(PIN_NOW),
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 45));
        let layout = super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
        (buf, layout)
    }

    /// The seat rows of `buf` whose first cell is `glyph`: those under the
    /// Agents rule, so the selected session's own `▸` above it is not one.
    fn marked(buf: &Buffer, glyph: &str) -> Vec<u16> {
        let rule = horizontal(buf).expect("the Agents rule");
        (rule + 1..buf.area.height)
            .filter(|y| buf[(0, *y)].symbol() == glyph)
            .collect()
    }

    /// The seat the keys act on and the seat the chat column shows are marked
    /// apart. Nothing is marked until the human steps with `n` / `p` (`▹`, no
    /// ground) or views a seat (`▸` and the selected ground); the first seat
    /// drawn is still the keys' target, a seat row is a click target that
    /// ends at the body, and the keys row names no seat key.
    #[test]
    fn the_seat_cursor_and_the_viewed_seat_are_marked_apart() {
        let mut shot = Shot::new();
        shot.agents = Some(seat_facts(&["lead", "colead"]));
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(layout.seats, ["lead", "colead"]);
        assert_eq!(layout.focused.as_deref(), Some("lead"), "the keys' target");
        assert!(marked(&buf, "▸").is_empty() && marked(&buf, "▹").is_empty());
        let rule = horizontal(&buf).expect("the Agents rule");
        let seat_row = |buf: &Buffer, name: &str| {
            (rule + 1..45)
                .find(|y| line(buf, *y).concat().contains(name))
                .expect(name)
        };
        let (first, second) = (seat_row(&buf, "lead"), seat_row(&buf, "colead"));
        assert_eq!(second, first + 1, "one row a seat");
        let keys: String = line(&buf, 44).concat();
        assert!(
            keys.contains("? keys") && !keys.contains("oo open seat") && !keys.contains("n/p"),
            "{keys}"
        );
        let at = Mouse {
            kind: MouseKind::Click,
            column: 8,
            row: second,
        };
        assert!(matches!(layout.hit(at), Some(super::Hit::Seat(name)) if name == "colead"));
        let rule_x = (0..160)
            .find(|x| buf[(*x, at.row)].symbol() == "│")
            .expect("the rule");
        let seat = |column| {
            matches!(
                layout.hit(Mouse { column, ..at }),
                Some(super::Hit::Seat(_))
            )
        };
        assert!(
            seat(rule_x - 3) && !seat(rule_x - 2),
            "a seat ends at the body"
        );
        // The cursor: a gutter glyph on the seat, no ground, no reverse.
        shot.model.set_focus(Some("colead".to_owned()));
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(marked(&buf, "▹"), [second]);
        assert!(marked(&buf, "▸").is_empty(), "a cursor is not a view");
        assert!(!buf[(8, second)].modifier.contains(Modifier::REVERSED));
        assert_eq!(layout.focused.as_deref(), Some("colead"));
        shot.model.set_focus(Some("ghost".to_owned()));
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(layout.focused.as_deref(), Some("lead"), "an undrawn name");
        assert!(marked(&buf, "▹").is_empty(), "an undrawn name draws none");
        shot.model.set_focus(Some("colead".to_owned()));
        let ascii = Look::read("off", "darcula", "on", "on");
        let (buf, _) = framed(&shot, 160, Some(&ascii));
        assert_eq!(marked(&buf, "-"), [second]);
        // The viewed seat is lifted, and the cursor yields to it.
        let viewing = Composer::Viewing { seat: "colead" };
        let (buf, _) = framed_as(&shot, 160, None, viewing);
        assert_eq!(marked(&buf, "▸"), [second]);
        assert!(marked(&buf, "▹").is_empty(), "the lift replaces the cursor");
        assert!(buf[(8, second)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(8, first)].modifier.contains(Modifier::REVERSED));
        let (buf, _) = framed_as(&shot, 160, Some(&ascii), viewing);
        assert_eq!(marked(&buf, ">"), [second]);
        // Scrolled so the cursor's seat is gone, no row carries the glyph and
        // the first drawn seat is the keys' target.
        let names: Vec<String> = (0..60).map(|n| format!("s{n:02}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        shot.agents = Some(seat_facts(&names));
        shot.model.set_focus(None);
        assert!(shot.model.wheel_body(false, 9));
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(layout.focused.as_deref(), Some("s03"));
        assert!(marked(&buf, "▹").is_empty() && marked(&buf, "▸").is_empty());
    }

    /// The cursor stays where `n` / `p` left it when another seat is viewed;
    /// the viewed seat is the one lifted row, and its session becomes the
    /// context, titled but not lifted, instead of a second lifted row.
    #[test]
    fn the_cursor_stays_while_another_seat_is_viewed_and_the_session_is_a_context() {
        let mut shot = Shot::new();
        shot.agents = Some(seat_facts(&["lead", "colead"]));
        shot.model.set_focus(Some("lead".to_owned()));
        let viewing = Composer::Viewing { seat: "colead" };
        let (buf, _) = framed_as(&shot, 160, None, viewing);
        let rule = horizontal(&buf).expect("the Agents rule");
        let (first, second) = (rule + 1, rule + 2);
        assert_eq!(marked(&buf, "▸"), [second], "the viewed seat is lifted");
        assert_eq!(marked(&buf, "▹"), [first], "the cursor stays");
        assert_eq!(buf[(0, 2)].symbol(), " ", "the session is not lifted");
        let name = &buf[(4, 2)];
        assert!(
            name.modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        assert!(!name.modifier.contains(Modifier::REVERSED));
        // The shared chat shows the session: it is the lifted row.
        let (buf, _) = framed(&shot, 160, None);
        assert_eq!(buf[(0, 2)].symbol(), "▸");
        assert!(buf[(4, 2)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(4, 2)].modifier.contains(Modifier::UNDERLINED));
        assert_eq!(marked(&buf, "▹"), [first], "the cursor is no view");
    }

    /// The composer's address names the seat a plain line reaches now and how
    /// it was picked; only the home composer says so.
    #[test]
    fn the_composer_address_names_the_seat_the_line_would_reach() {
        let shot = Shot::new();
        // The chat column below the header: the address and hint rows live here.
        let chat = |dest, composer| {
            let screen = shot.screen(composer);
            let wait = super::Wait {
                dest,
                ..super::Wait::default()
            };
            let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
            let _ = super::draw_with_layout(&screen, wait, &mut buf);
            (1..45)
                .map(|y| line(&buf, y)[45..].concat())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let auto = Some(super::Dest {
            seat: "colead",
            named: false,
        });
        let named = Some(super::Dest {
            seat: "lead",
            named: true,
        });
        assert!(chat(auto, browsing("")).contains("to api · colead (auto)"));
        assert!(chat(named, browsing("")).contains("to api · lead (named)"));
        let none = chat(None, browsing(""));
        assert!(none.contains("to api") && !none.contains("to api ·") && !none.contains("(auto)"));
        let held = Composer::Held { why: "owned" };
        assert!(!chat(auto, held).contains("colead"), "not home: no seat");
        let viewing = Composer::Viewing { seat: "scout" };
        assert!(!chat(auto, viewing).contains("(auto)"));
        assert!(chat(auto, viewing).contains("viewing scout"));
        assert!(!chat(auto, READ_ONLY).contains("(auto)"));
    }

    /// The header names the open ask of the seat viewed, else of the seat
    /// under the cursor, else the first that waits.
    #[test]
    fn the_header_names_the_viewed_seats_ask_then_the_cursors_then_the_first() {
        let mut shot = Shot::new();
        shot.agents = Some(seat_facts(&["lead", "colead"]));
        shot.overview.open = vec![
            Open {
                seat: "lead".to_owned(),
                text: "first ask".to_owned(),
                age_secs: Some(60),
            },
            Open {
                seat: "colead".to_owned(),
                text: "second ask".to_owned(),
                age_secs: Some(120),
            },
        ];
        let header =
            |shot: &Shot, composer| line(&framed_as(shot, 160, None, composer).0, 0).concat();
        assert!(header(&shot, READ_ONLY).contains("⚠ lead: first ask · 1m"));
        shot.model.set_focus(Some("colead".to_owned()));
        let at_cursor = header(&shot, READ_ONLY);
        assert!(
            at_cursor.contains("⚠ colead: second ask · 2m"),
            "{at_cursor}"
        );
        assert!(!at_cursor.contains("first ask"), "one ask at a time");
        shot.model.set_focus(Some("lead".to_owned()));
        let viewing = Composer::Viewing { seat: "colead" };
        assert!(header(&shot, viewing).contains("⚠ colead: second ask"));
        shot.model.set_focus(Some("scout".to_owned()));
        let quiet = Composer::Viewing { seat: "scout" };
        assert!(
            header(&shot, quiet).contains("⚠ lead: first ask"),
            "falls to the first"
        );
        let ascii = Look::read("off", "darcula", "on", "on");
        let (buf, _) = framed_as(&shot, 160, Some(&ascii), READ_ONLY);
        assert!(line(&buf, 0).concat().contains("! lead: first ask · 1m"));
    }

    /// The reason comes before the directory: both when the room allows, and
    /// when it does not the directory goes whole rather than the reason; a
    /// reason needs 12 cells to be drawn at all.
    #[test]
    fn the_header_reason_comes_before_the_directory() {
        let mut shot = Shot::new();
        shot.overview.dir = Some("~/projects/clemens33/ae".to_owned());
        shot.overview.open = vec![Open {
            seat: "lead".to_owned(),
            text: "approve the plan".to_owned(),
            age_secs: Some(60),
        }];
        let wide = line(&framed(&shot, 160, None).0, 0).concat();
        let (reason, dir) = (wide.find('⚠'), wide.find("~/projects"));
        assert!(reason.is_some() && dir.is_some() && reason < dir, "{wide}");
        shot.overview.open[0].text = "approve the plan ".repeat(8);
        let narrow = line(&shot.draw(100, 30, READ_ONLY), 0).concat();
        assert!(
            narrow.contains('⚠') && narrow.contains("… · 1m"),
            "{narrow}"
        );
        assert!(!narrow.contains("~/"), "no directory fragment: {narrow}");
        let screen = shot.screen(READ_ONLY);
        let ctx = super::Ctx {
            screen: &screen,
            paint: Paint::of(screen.look.as_ref()),
            icons: true,
            wait: super::Wait::default(),
        };
        assert_eq!(
            super::reason_words(&ctx, 13),
            None,
            "under 12 beside the mark"
        );
        let words = super::reason_words(&ctx, 14).expect("12 cells and the mark");
        assert!(super::drawn_cols(&words) <= 14, "{words:?}");
    }

    /// Approved legend teaches all seven task marks in each glyph vocabulary.
    #[test]
    fn honestwords_keys_tab_explains_every_mark_in_both_glyph_vocabularies() {
        for icons in [true, false] {
            let rows: Vec<String> = super::settings_body(
                super::SettingsTab::Keys,
                &super::SettingsBodies::default(),
                Paint::of(None),
                100,
                icons,
            )
            .into_iter()
            .map(|(text, _)| text)
            .collect();
            for (unicode, ascii, word) in [
                ("✖", "x", "dead"),
                ("⚠", "!", "needs you"),
                ("◌", "?", "unknown"),
                ("●", "*", "working"),
                ("◔", "~", "waiting on an agent"),
                ("✓", "+", "finished"),
                ("·", "-", "no current status"),
            ] {
                let glyph = if icons { unicode } else { ascii };
                assert!(
                    rows.iter()
                        .any(|row| row.trim_start().starts_with(glyph) && row.contains(word)),
                    "{glyph} {word}: {rows:?}"
                );
            }
            let text = rows.join("\n");
            assert!(
                text.contains("declared done"),
                "completion needs a declaration: {text}"
            );
            assert!(
                text.contains("silent past"),
                "unknown includes silence: {text}"
            );
            assert!(
                text.contains("no current done/waiting/blocked declaration"),
                "Idle ruling: {text}"
            );
            assert!(
                text.contains("Browse") && text.contains("Write") && text.contains("Mouse"),
                "existing key help survives"
            );
        }
    }

    /// Lead final-r1 ruling: a qualifier yields before identity; its head
    /// remains visible at the 30-cell sidebar floor. Independent geometry:
    /// sidebar minus four gives room; colead spends ten cells before its label.
    #[test]
    fn honestwords_app_qualifier_yields_before_identity_and_preserves_state_at_floor() {
        for icons in [true, false] {
            let look = Look::read(if icons { "on" } else { "off" }, "darcula", "on", "off");
            for (sidebar, model, effort, full, wrapped) in [
                (60, "gpt-6.1-sol", "xhigh", true, false),
                (44, "gpt-6.1-sol", "xhigh", false, false),
                (30, "gpt-6.1-sol", "xhigh", false, true),
                (30, "", "", false, false),
                (44, "m12345678901234567890123456789", "xhigh", true, true),
            ] {
                let mut shot = Shot::new();
                let mut facts =
                    label_facts(&[("colead", model, effort, "atlas"), ("next", "", "", "")]);
                let Facts::Seats { agents, .. } = &mut facts else {
                    unreachable!("synthetic seat facts");
                };
                agents[0].state = "waiting-user".to_owned();
                agents[1].state = "working".to_owned();
                shot.agents = Some(facts);
                with_sidebar(&mut shot, sidebar);
                let (buf, layout) = framed(&shot, 160, Some(&look));
                let rule = horizontal(&buf).expect("the Agents rule");
                let row = |y| line(&buf, y)[..usize::from(sidebar)].concat();
                let first = row(rule + 1);
                assert!(
                    first.contains("colead") && first.contains("needs you"),
                    "{sidebar}: {first}"
                );
                assert!(
                    first
                        .trim_start()
                        .starts_with(if icons { "⚠" } else { "!" }),
                    "{first}"
                );
                assert_eq!(
                    first.contains("(waiting on you)"),
                    full,
                    "{sidebar}: {first}"
                );
                let label = if wrapped {
                    row(rule + 2)
                } else {
                    first.clone()
                };
                if !model.is_empty() {
                    assert!(
                        label.contains(model) && label.contains(effort),
                        "{sidebar}: {label}"
                    );
                    assert!(
                        !label.contains("atlas"),
                        "observed model replaces client: {label}"
                    );
                    if wrapped {
                        assert!(!first.contains(model), "label belongs on row two: {first}");
                    }
                }
                let next = row(rule + if wrapped { 3 } else { 2 });
                assert!(
                    next.contains("next"),
                    "seat row group stays bounded: {next}"
                );
                assert_eq!(layout.seats, ["colead", "next"]);
            }
        }
    }

    /// The Keys tab teaches the new glyphs and the positional digits.
    #[test]
    fn the_keys_tab_names_the_home_glyph_the_cursor_and_the_nth_session() {
        let rows: Vec<String> = keys_body(Paint::of(None))
            .into_iter()
            .map(|r| r.0)
            .collect();
        let row = |start: &str, words: &[&str]| {
            assert!(
                rows.iter()
                    .any(|r| r.starts_with(start) && words.iter().all(|w| r.contains(w))),
                "{start}: {words:?} in {rows:?}"
            );
        };
        row("1-9", &["Nth session"]);
        row("n/p", &["cursor", "▹"]);
        row("Esc", &["cursor", "home"]);
        row("◂ or <", &["home session"]);
        assert!(!rows.iter().any(|r| r.contains("this window")));
    }

    /// At the narrowest sidebar (30 columns) a tall fleet still draws one
    /// session a row, each ending inside the rule's two blank cells.
    #[test]
    fn a_floor_sidebar_keeps_each_session_on_one_row() {
        let mut shot = sessions(28);
        let first = shot.draw(100, 30, READ_ONLY);
        let buf = drag(&mut shot, &first, Edge::Sidebar, 30);
        assert_eq!(vertical(&buf), Some(30), "the floor width");
        assert!(
            line(&buf, 1)[..30].concat().trim().is_empty(),
            "summary spacer"
        );
        let ys: Vec<u16> = (2..30)
            .filter(|y| line(&buf, *y)[4..5].concat() == "s")
            .collect();
        assert!(ys.len() >= 5, "{ys:?}");
        assert_eq!(
            ys,
            (2..2 + u16::try_from(ys.len()).unwrap_or(0)).collect::<Vec<_>>()
        );
        for y in ys {
            let cells = line(&buf, y);
            assert_eq!(cells[28..30].concat(), "  ", "row {y}");
            assert!(
                cells[4..28].concat().trim_end().chars().count() >= 4,
                "row {y}"
            );
        }
    }

    /// R3: the viewed seat's row shares the selected ground out to the body's
    /// end and the glyph is the selected ink, never the title or a state hue;
    /// the next seat keeps the sidebar's ground. On the ground only the
    /// selected ink and the marks' accents are drawn, so the pairs are the
    /// ones `theme`'s contrast bar already measures.
    #[test]
    fn the_viewed_seat_sits_on_the_selected_ground_in_every_palette() {
        let mut shot = Shot::new();
        shot.agents = Some(seat_facts(&["lead", "colead"]));
        for name in ["darcula", "a", "b"] {
            let look = Look::read("on", name, "on", "on");
            let viewing = Composer::Viewing { seat: "lead" };
            let (buf, _) = framed_as(&shot, 160, Some(&look), viewing);
            let palette = look.palette;
            let first = *marked(&buf, "▸").first().expect("a marked row");
            let rule = (0..160)
                .find(|x| buf[(*x, first)].symbol() == "│")
                .expect("the rule");
            let ground = super::colour(palette.selected);
            let ink = super::colour(palette.selected_ink);
            let glyph = &buf[(0, first)];
            assert_eq!(
                (glyph.fg, glyph.modifier.contains(Modifier::BOLD)),
                (ink, true)
            );
            for x in 0..rule - 2 {
                assert_eq!(buf[(x, first)].bg, ground, "{name} ({x},{first})");
            }
            let mark = super::colour(palette.accent(Mark::Stale));
            let drawn: Vec<_> = (0..rule - 2)
                .map(|x| &buf[(x, first)])
                .filter(|cell| cell.symbol() != " ")
                .collect();
            assert!(!drawn.is_empty(), "{name} row {first}");
            for cell in drawn {
                let ok = cell.fg == ink || cell.fg == mark;
                assert!(ok, "{name}: {:?} {:?}", cell.symbol(), cell.fg);
            }
            let sidebar = super::colour(palette.base);
            assert_eq!(buf[(5, first + 1)].bg, sidebar, "{name}: the next seat");
        }
    }

    /// R1/R4: a seat's one row carries its model, the client when its frame
    /// drew none, and the effort, each only when known and never its profile
    /// name.
    #[test]
    fn the_agents_row_is_model_and_effort_never_the_profile() {
        let mut shot = Shot::new();
        let seat = |name: &str, model: &str, effort: &str| PickerAgent {
            name: name.to_owned(),
            profile: "sonnet55x".to_owned(),
            state: "working".to_owned(),
            pane: "%7".to_owned(),
            client: "claude".to_owned(),
            model: model.to_owned(),
            effort: effort.to_owned(),
            drift: false,
        };
        shot.agents = Some(Facts::Seats {
            id: "$1".to_owned(),
            agents: vec![
                seat("seat-a", "Sonnet 5.5", "xhigh"),
                seat("seat-b", "", ""),
                seat("seat-c", "", "max"),
                PickerAgent {
                    client: String::new(),
                    ..seat("seat-d", "", "")
                },
            ],
        });
        let (buf, _) = framed(&shot, 160, None);
        let between = |name: &str| {
            let y = spot(&buf, name).expect(name).0;
            let row = line(&buf, y)[..44].concat();
            let (from, to) = (
                row.find(name).expect(name) + name.len(),
                row.find("working"),
            );
            row[from..to.expect("its state")].trim().to_owned()
        };
        assert_eq!(between("seat-a"), "Sonnet 5.5 xhigh");
        assert_eq!(between("seat-b"), "claude", "no placeholder");
        assert_eq!(between("seat-c"), "claude max");
        assert_eq!(
            between("seat-d"),
            "",
            "the row stays: the geometry is stable"
        );
        let frame: String = (0..45).map(|y| line(&buf, y).concat()).collect();
        assert!(!frame.contains("sonnet55x"), "the profile is not drawn");
    }

    /// R7: a seat that waits on the human is told in the chat header, never
    /// under its row: its Agents row stays one line; an ask from a seat the
    /// roster lacks is a row of its own after the roster, so none is lost;
    /// and no Overview or Tab word is left in the frame.
    #[test]
    fn a_seat_waiting_on_the_human_is_told_in_the_header_not_under_its_row() {
        let mut shot = Shot::new();
        shot.agents = Some(seat_facts(&["seat-a", "seat-b"]));
        shot.overview.open = vec![
            Open {
                seat: "seat-b".to_owned(),
                text: "Approve X?".to_owned(),
                age_secs: Some(240),
            },
            Open {
                seat: "ghost".to_owned(),
                text: "registry down".to_owned(),
                age_secs: Some(3600),
            },
        ];
        let look = Look::read("on", "darcula", "on", "on");
        let (buf, layout) = framed(&shot, 160, Some(&look));
        let sidebar = |y| line(&buf, y)[..42].concat().trim().to_owned();
        let at = |name: &str| (2..45).find(|y| sidebar(*y).contains(name)).expect(name);
        assert_eq!(at("seat-b"), at("seat-a") + 1, "seat-a asks nothing");
        assert_eq!(layout.seats, ["seat-a", "seat-b", "ghost"]);
        assert_eq!(at("ghost"), at("seat-b") + 1, "after the roster, one row");
        let ghost = sidebar(at("ghost"));
        assert!(
            ghost.starts_with("⚠ ghost") && ghost.ends_with("needs you"),
            "{ghost}"
        );
        let side: String = (0..45).map(|y| line(&buf, y)[..42].concat()).collect();
        assert!(
            !side.contains("Approve X?") && !side.contains("registry down"),
            "no reason under a row: {side}"
        );
        let header = line(&buf, 0).concat();
        assert!(header.contains("⚠ seat-b: Approve X? · 4m"), "{header}");
        let viewing = Composer::Viewing { seat: "ghost" };
        let (buf, _) = framed_as(&shot, 160, Some(&look), viewing);
        let header = line(&buf, 0).concat();
        assert!(header.contains("⚠ ghost: registry down · 1h"), "{header}");
        let frame: String = (0..45).map(|y| line(&buf, y).concat()).collect();
        assert!(
            !frame.contains("Overview") && !frame.contains("Tab"),
            "{frame}"
        );
    }

    /// R4(d)/I2: each section of the Keys tab says what its own `^C` and its
    /// mouse do, so the list is complete section by section.
    #[test]
    fn the_keys_tab_states_each_sections_ctrl_c_and_mouse() {
        let rows: Vec<String> = keys_body(Paint::of(None))
            .into_iter()
            .map(|r| r.0)
            .collect();
        let titles = ["Browse", "Write", "Held", "Settings", "Mouse"];
        let has = |title: &str, words: &[&str]| {
            let from = rows
                .iter()
                .position(|row| row.starts_with(title))
                .expect(title);
            let rest = &rows[from + 1..];
            let to = rest
                .iter()
                .position(|r| titles.iter().any(|t| r.starts_with(t)));
            let body = &rest[..to.unwrap_or(rest.len())];
            let found = body.iter().any(|row| words.iter().all(|w| row.contains(w)));
            assert!(found, "{title} has no row with {words:?}: {body:?}");
        };
        has("Browse", &["^C", "quit at once"]);
        has("Write", &["^C", "clear a draft", "empty", "quits at once"]);
        has("Write", &["^C", "after a clear", "arms"]);
        has("Write", &["^U", "whole draft"]);
        has("Write", &["^W", "word before"]);
        has("Write", &["Alt+Bksp", "punctuation", "Option as Meta"]);
        has("Write", &["^K", "end of the line"]);
        has("Write", &["^Y", "cleared draft", "once"]);
        has("Settings", &["gear", "Esc close", "closes"]);
        has("Settings", &["wheel", "3 rows"]);
        has("Mouse", &["click", "session", "tab"]);
        has("Mouse", &["wheel", "3 rows"]);
        has("Mouse", &["drag", "border"]);
    }
    // Independent S1 acceptance: frozen by appdeclutter-nav; install unchanged.
    fn declutter_frame(shot: &Shot, width: u16, icons: bool) -> (Buffer, super::Layout) {
        declutter_frame_at(shot, width, 28, icons)
    }

    fn declutter_frame_at(
        shot: &Shot,
        width: u16,
        height: u16,
        icons: bool,
    ) -> (Buffer, super::Layout) {
        let mut screen = shot.screen(READ_ONLY);
        screen.look = Some(Look::read(
            if icons { "on" } else { "off" },
            "darcula",
            "on",
            "on",
        ));
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        let layout = super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
        (buf, layout)
    }

    fn declutter_line(buf: &Buffer, layout: &super::Layout, y: u16) -> String {
        line(buf, y)[..usize::from(layout.list.width)].concat()
    }

    #[test]
    fn declutter_counts_ride_the_name_row_hide_goals_and_keep_home_hits_and_cells() {
        for width in [90, 100, 160] {
            for icons in [false, true] {
                let mut shot = Shot::new();
                shot.fleet.rows[0].name = "界api".to_owned();
                shot.fleet.rows[0].line2 = Line2::Goal {
                    text: "goal hidden in list".to_owned(),
                    age_secs: Some(60),
                };
                shot.fleet.home = Some("界api".to_owned());
                shot.model = Model::new(&shot.fleet);
                let (buf, layout) = declutter_frame(&shot, width, icons);
                let name = declutter_line(&buf, &layout, 2);
                let counts = if icons { "●3 ✓B12" } else { "*3 +B12" };
                let home = if icons { "◂" } else { "<" };
                assert!(name.contains("界 api") && name.contains(home), "{name:?}");
                assert!(name.contains(counts), "{width}/{icons}: {name:?}");
                assert!(!name.contains("goal"), "{name:?}");
                let below = declutter_line(&buf, &layout, 3);
                assert!(
                    !below.contains(counts) && !below.contains("goal"),
                    "{below:?}"
                );
                for column in [0, super::NAME, layout.list.width - 1] {
                    assert!(
                        matches!(layout.hit(Mouse { kind: MouseKind::Click, column, row: 2 }), Some(super::Hit::Session(name)) if name == "界api")
                    );
                }
                assert!(
                    !matches!(
                        layout.hit(Mouse {
                            kind: MouseKind::Click,
                            column: 0,
                            row: 3
                        }),
                        Some(super::Hit::Session(_))
                    ),
                    "a session is one row to click"
                );
                assert_eq!(buf[(layout.list.width, 2)].symbol(), "│");
                shot.fleet.rows[0].line2 = Line2::NoGoal;
                let (buf, layout) = declutter_frame(&shot, width, icons);
                let name = declutter_line(&buf, &layout, 2);
                assert!(name.contains(counts) && !name.contains("goal"));
                shot.fleet.rows[0].counts = Counts::Unknown;
                let (buf, layout) = declutter_frame(&shot, width, icons);
                assert!(declutter_line(&buf, &layout, 2).trim_end().ends_with(" ?"));
            }
        }
    }

    #[test]
    fn declutter_header_sums_hidden_multiplicities_and_keeps_unknown_distinct_from_zero() {
        let mut shot = sessions(9);
        for row in &mut shot.fleet.rows {
            row.counts = Counts::Marks(Vec::new());
            row.line2 = Line2::NoGoal;
            row.needy = matches!(row.index, 3 | 8 | 9);
        }
        shot.fleet.rows[0].counts = Counts::Marks(vec![
            (Mark::Working, false, 7),
            (Mark::WaitingAgent, false, 2),
            (Mark::Done, true, 3),
        ]);
        shot.fleet.rows[6].counts = Counts::Unknown;
        shot.fleet.rows[7].counts = Counts::Marks(vec![(Mark::Working, false, 5)]);
        shot.fleet.rows[8].counts = Counts::Stopped;
        shot.fleet.rows[8].line2 = Line2::NotRunning {
            seats: 5,
            stopped_secs: None,
        };
        shot.model.grab(Drag {
            edge: Edge::List,
            from: 0,
            at: 0,
        });
        let _ = shot.model.drag_to(5);
        let _ = shot.model.let_go();
        for (icons, bare) in [(true, "◈ 17+? ● 12+? ⚠ 3"), (false, "@ 17+? * 12+? ! 3")] {
            let (buf, layout) = declutter_frame(&shot, 100, icons);
            assert_eq!(
                declutter_line(&buf, &layout, 0)
                    .trim()
                    .strip_suffix('↓')
                    .map(str::trim_end),
                Some("Agt 17+? · Wrk 12+? · Need 3")
            );
            assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
            shot.key(crate::app::model::Key::NextNeed);
            assert_eq!(shot.model.selected(), Some("s02"));
            shot.key(crate::app::model::Key::NextNeed);
            let (buf, layout) = declutter_frame(&shot, 100, icons);
            assert_eq!(
                declutter_line(&buf, &layout, 0)
                    .trim()
                    .strip_suffix("↑↓")
                    .map(str::trim_end),
                Some(bare)
            );
            shot.key(crate::app::model::Key::Esc);
        }
        shot.fleet.rows.truncate(1);
        shot.model = Model::new(&shot.fleet);
        for (counts, expected) in [
            (Counts::Marks(Vec::new()), "Agt 0 · Wrk 0 · Need 0"),
            (Counts::Unknown, "Agt 0+? · Wrk 0+? · Need 0"),
        ] {
            shot.fleet.rows[0].counts = counts;
            let (buf, layout) = declutter_frame(&shot, 100, false);
            assert_eq!(declutter_line(&buf, &layout, 0).trim(), expected);
            assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
            if matches!(shot.fleet.rows[0].counts, Counts::Marks(_)) {
                assert!(declutter_line(&buf, &layout, 2).contains("no agents"));
            }
        }
    }

    #[test]
    fn declutter_crowded_header_keeps_complete_session_and_attention_counts() {
        let mut shot = sessions(2);
        shot.fleet.rows[0].counts = Counts::Marks(vec![(Mark::Working, false, usize::MAX)]);
        shot.fleet.rows[1].counts = Counts::Unknown;
        shot.fleet.rows[1].needy = true;
        let (buf, layout) = declutter_frame(&shot, 90, false);
        assert_eq!(layout.list.width, 34);
        assert_eq!(
            declutter_line(&buf, &layout, 0).trim(),
            "! 1",
            "whole low-priority counts give way before a digit is cut"
        );
        assert!(declutter_line(&buf, &layout, 1).trim().is_empty());
        shot.fleet.rows[0].counts = Counts::Marks(vec![
            (Mark::Working, false, 3),
            (Mark::Done, false, usize::MAX - 3),
        ]);
        let (buf, layout) = declutter_frame(&shot, 90, false);
        assert_eq!(declutter_line(&buf, &layout, 0).trim(), "! 1");
        shot = sessions(130);
        for row in &mut shot.fleet.rows {
            row.counts = Counts::Marks(Vec::new());
            row.needy = row.index <= 100;
        }
        shot.fleet.rows[0].counts =
            Counts::Marks(vec![(Mark::Working, false, 1000), (Mark::Done, false, 150)]);
        shot.fleet.rows[1].counts = Counts::Unknown;
        let (buf, layout) = declutter_frame(&shot, 90, false);
        assert_eq!(
            declutter_line(&buf, &layout, 0)
                .trim_end()
                .strip_suffix('↓')
                .map(str::trim),
            Some("@ 1150+? * 1000+? ! 100"),
            "the right-aligned arrow reserves room in the one-line ladder"
        );
    }

    #[test]
    fn declutter_crowded_counts_keep_the_urgent_ones_and_close_on_an_ellipsis() {
        let mut shot = Shot::new();
        shot.fleet.rows[0].counts = Counts::Marks(
            Mark::BY_URGENCY
                .into_iter()
                .map(|mark| (mark, false, 12))
                .collect(),
        );
        for status in [
            Line2::Question {
                text: "approve deploy now".to_owned(),
                age_secs: Some(60),
            },
            Line2::SeatDown,
        ] {
            shot.fleet.rows[0].line2 = status;
            let (buf, layout) = declutter_frame(&shot, 90, true);
            let name = declutter_line(&buf, &layout, 2);
            assert!(
                name.contains("✖12") && name.trim_end().ends_with('…'),
                "{name:?}"
            );
            assert!(
                !name.contains("✓12") && !name.contains("·12"),
                "low urgency counts yield: {name:?}"
            );
            assert!(
                !name.contains("approve") && !name.contains("down"),
                "no status words in a session row: {name:?}"
            );
        }
    }

    #[test]
    fn declutter_reason_clips_cells_and_keeps_whole_graphemes_and_age() {
        for (text, room, expected) in [
            ("界".repeat(10), 20, "界".repeat(10)),
            ("界".repeat(11), 20, "界".repeat(7) + "… · 4m"),
            ("界".repeat(8), 22, "界".repeat(8) + " · 4m"),
            ("e\u{301}".repeat(23), 29, "e\u{301}".repeat(23) + " · 4m"),
            ("e\u{301}".repeat(30), 29, "e\u{301}".repeat(23) + "… · 4m"),
        ] {
            let words = super::aged(&text, Some(240), room);
            assert_eq!(words, expected, "{text}/{room}");
            assert!(super::drawn_cols(&words) <= room, "{text}/{room}");
        }
    }

    /// The help lists `v` among the browse keys and gives the peek panel its
    /// own section, between Held and Settings, saying nothing reaches the pane.
    #[test]
    fn the_keys_tab_lists_v_and_the_peek_panel() {
        let rows: Vec<String> = keys_body(Paint::of(None))
            .into_iter()
            .map(|r| r.0)
            .collect();
        let at = |start: &str| {
            rows.iter()
                .position(|row| row.starts_with(start))
                .expect(start)
        };
        let (held, peek, settings) = (at("Held"), at("Peek"), at("Settings"));
        assert!(held < peek && peek < settings, "{rows:?}");
        let browse = &rows[at("Browse")..at("Write")];
        assert!(
            browse
                .iter()
                .any(|r| r.starts_with("v ") && r.contains("peek"))
        );
        let section = &rows[peek..settings];
        for words in [
            &["j/k", "scroll a line"][..],
            &["PgUp", "scroll a page"],
            &["Esc q v", "back to chat", "click the session"],
            &["wheel", "3 rows"],
            &["any other", "nothing reaches the pane"],
        ] {
            assert!(
                section.iter().any(|r| words.iter().all(|w| r.contains(w))),
                "{words:?}: {section:?}"
            );
        }
    }

    /// With a seat viewed the keys row says `viewing` and `Esc chat`,
    /// keeps the version and records no gear, which would be a dead click.
    #[test]
    fn the_peek_keys_row_hides_the_gear_and_keeps_the_version() {
        let mut shot = Shot::new();
        shot.model.open_peek(1);
        let (row, layout) = shot.footer_at(100, browsing(""), false);
        let tail = crate::version_line();
        assert_eq!(
            row.strip_suffix(tail.as_str())
                .map(|head| head.trim_end().to_owned()),
            Some("  viewing   Esc chat".to_owned()),
            "{row:?}"
        );
        let last = Mouse {
            kind: MouseKind::Click,
            column: 99,
            row: 0,
        };
        assert!(layout.hit(last).is_none(), "no gear target");
        let (narrow, _) = shot.footer_at(22, browsing(""), false);
        assert_eq!(narrow, "  viewing   Esc chat", "the version drops, no gear");
    }

    fn peek_view(body: Option<&crate::app::peek::Body>) -> crate::app::peek::View<'_> {
        crate::app::peek::View {
            session: "api",
            seat: "lead",
            body,
            age_secs: 1,
            back: 0,
            newer_waiting: false,
        }
    }

    /// The close label sits on the title row, right-aligned, and is the
    /// recorded click target of exactly its own cells, also when a long
    /// session and seat name must be cut short of it.
    #[test]
    fn the_peek_close_label_is_a_target_over_its_own_cells() {
        let body = crate::app::peek::Body::Lines(vec!["x".to_owned()]);
        let long = "development-long-session";
        for (width, session, seat) in [
            (60, "api", "lead"),
            (40, "aedev", "apppeek-nav3"),
            (60, long, "apppeek-navigator-3"),
        ] {
            let view = crate::app::peek::View {
                session,
                seat,
                ..peek_view(Some(&body))
            };
            let mut buf = Buffer::empty(Rect::new(0, 0, width, 10));
            let mut layout = super::Layout::default();
            let bounds = super::paint_peek(&view, None, &mut buf, &mut layout);
            assert_eq!((bounds.visible, bounds.max_back), (6, 0));
            let title = line(&buf, 0).concat();
            assert!(title.starts_with("  Peek · "), "{title:?}");
            assert_eq!(title.contains(seat), seat == "lead", "the title is cut");
            let start = title.find("Esc close").expect("label");
            let column = u16::try_from(title[..start].chars().count()).expect("fits");
            let hit = |column| {
                let click = Mouse {
                    kind: MouseKind::Click,
                    column,
                    row: 0,
                };
                matches!(layout.hit(click), Some(super::Hit::PeekClose))
            };
            assert!(hit(column) && hit(column + 8), "the label's cells");
            assert!(!hit(column - 1) && !hit(column + 9) && !hit(3), "only them");
            assert_eq!(column + 9 + LEFT, width, "right-aligned with the gutter");
            assert_eq!(layout.targets.len(), 1, "no other target survives");
        }
    }

    /// The status note runs to the right gutter, not to half the width: at 40
    /// cells a 39-cell note is cut at its 38th and not before.
    #[test]
    fn the_peek_status_note_runs_to_the_right_gutter() {
        let body = crate::app::peek::Body::Lines(vec!["x".to_owned()]);
        let view = crate::app::peek::View {
            age_secs: 123_456_789,
            ..peek_view(Some(&body))
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        let _ = super::paint_peek(&view, None, &mut buf, &mut super::Layout::default());
        assert_eq!(
            line(&buf, 1).concat().trim_end(),
            "  STALE · captured 123456789s ago · 1 ro",
        );
    }

    /// A long line wraps at the pane width with its continuation rows
    /// hanging two cells, and a line that fits exactly does not wrap.
    #[test]
    fn a_peek_line_wraps_at_the_pane_width_with_a_two_cell_hang() {
        let room = 40 - usize::from(LEFT);
        let fits = "a".repeat(room);
        let words = "word ".repeat(30);
        let body = crate::app::peek::Body::Lines(vec![fits.clone(), words]);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 14));
        let _ = super::draw_peek(&peek_view(Some(&body)), None, &mut buf);
        let rows: Vec<String> = (3..13).map(|y| line(&buf, y).concat()).collect();
        assert_eq!(rows[0].trim_end(), format!("  {fits}"), "exactly the room");
        assert!(rows[1].trim_start().starts_with("word"), "{rows:?}");
        assert!(rows[2].starts_with("    word"), "hung two more: {rows:?}");
        assert!(rows.iter().all(|row| row.chars().count() == 40));
    }

    /// Retiring the column for the seat view keeps the sidebar's targets
    /// and seats, and drops the chat's own Compose target.
    #[test]
    fn the_peek_column_keeps_the_sidebar_targets() {
        let mut shot = Shot::new();
        shot.agents = Some(seat_facts(&["lead", "colead"]));
        let (buf, mut layout) = framed(&shot, 160, None);
        assert!(!layout.targets.is_empty(), "the closed frame has targets");
        let lane = layout.lane_area().expect("lane drawn");
        layout.peek_column(&buf, lane);
        assert!(
            layout
                .targets
                .iter()
                .any(|(_, hit)| matches!(hit, super::Hit::Session(_))),
            "sidebar targets stand"
        );
        assert!(
            !layout
                .targets
                .iter()
                .any(|(_, hit)| matches!(hit, super::Hit::Compose)),
            "the composer target is gone"
        );
        assert!(!layout.seats.is_empty(), "drawn seats stand");
    }

    /// Last whole count spans stay when they fit; overflow leaves one ellipsis.
    #[test]
    fn declutter_count_tail_fills_the_last_cell_without_losing_a_whole_span() {
        for (last, expected) in [
            (12, "✖B123 ⚠B123 ◌B12 ●B12 ◔123 ✓123 ·12"),
            (123, "✖B123 ⚠B123 ◌B12 ●B12 ◔123 ✓123 ·123"),
            (1234, "✖B123 ⚠B123 ◌B12 ●B12 ◔123 ✓123 …"),
        ] {
            let mut shot = Shot::new();
            shot.fleet.rows[0].home = false;
            shot.fleet.rows[0].name = "a".to_owned();
            shot.fleet.rows[0].line2 = Line2::NoGoal;
            shot.fleet.rows[0].counts = Counts::Marks(vec![
                (Mark::Dead, true, 123),
                (Mark::NeedsYou, true, 123),
                (Mark::Stale, true, 12),
                (Mark::Working, true, 12),
                (Mark::WaitingAgent, false, 123),
                (Mark::Done, false, 123),
                (Mark::Idle, false, last),
            ]);
            let (buf, layout) = shot.laid(160, 43, READ_ONLY);
            assert_eq!(layout.list.width, 44, "40 cells right of the mark");
            let row = line(&buf, 2)[..45].concat();
            assert!(row.ends_with(&format!("{expected}  │")), "{row:?}");
        }
    }

    /// A name that fits keeps its stopped age; a name too long for it is cut
    /// beside the shortest tally. Either way the gap and two cells before the
    /// rule stay.
    #[test]
    fn declutter_long_names_leave_the_stopped_age_and_right_edge_intact() {
        for (stopped, long) in [(false, 60), (true, 60), (true, 25)] {
            let mut shot = Shot::new();
            shot.fleet.rows = vec![row(&"n".repeat(long), 1, false)];
            shot.fleet.rows[0].home = false;
            shot.fleet.rows[0].counts = if stopped {
                Counts::Stopped
            } else {
                Counts::Marks(Vec::new())
            };
            shot.fleet.rows[0].line2 = if stopped {
                Line2::NotRunning {
                    seats: 3,
                    stopped_secs: Some(180),
                }
            } else {
                Line2::NoGoal
            };
            shot.model = Model::new(&shot.fleet);
            let (buf, layout) = shot.laid(160, 43, READ_ONLY);
            assert_eq!(layout.list.width, 44);
            let expected = match (stopped, long) {
                (false, _) => format!("  ● {}… no agents  │", "n".repeat(27)),
                (true, 60) => format!("  ● {}… stopped  │", "n".repeat(29)),
                _ => format!("  ● {}   stopped 3m  │", "n".repeat(25)),
            };
            assert_eq!(
                line(&buf, 2)[..45].concat(),
                expected,
                "stopped={stopped}/{long}"
            );
            assert_eq!(line(&buf, 2)[42..44].concat(), "  ");
            assert_eq!(buf[(44, 3)].symbol(), "│");
        }
    }

    /// A dragged u16-width sidebar still draws the complete stopped label.
    #[test]
    fn declutter_stopped_age_survives_a_sidebar_above_half_the_u16_range() {
        let mut shot = Shot::new();
        shot.fleet.rows[0].home = false;
        shot.fleet.rows[0].counts = Counts::Stopped;
        shot.fleet.rows[0].line2 = Line2::NotRunning {
            seats: 1,
            stopped_secs: Some(60),
        };
        let area = Rect::new(0, 0, 32_820, 20);
        assert_eq!(area.width, 32_820, "Rect keeps the actual terminal width");
        let grabbed = Drag {
            edge: Edge::Sidebar,
            from: 44,
            at: 44,
        };
        shot.model.grab(grabbed);
        let requested = dragged_to(
            grabbed,
            Mouse {
                kind: MouseKind::Drag,
                column: 32_775,
                row: 1,
            },
            area,
            1,
        );
        assert_eq!(requested, 32_775, "the actual drag allows this sidebar");
        let _ = shot.model.drag_to(requested);
        let _ = shot.model.let_go();
        let mut buf = Buffer::empty(area);
        assert_eq!(buf.area.width, 32_820);
        let layout =
            super::draw_with_layout(&shot.screen(READ_ONLY), super::Wait::default(), &mut buf);
        assert_eq!(layout.list.width, 32_775);
        assert_eq!(line(&buf, 2)[32_763..32_776].concat(), "stopped 1m  │");
    }

    /// Clipping preserves original grapheme barriers, then appends one ellipsis.
    #[test]
    fn declutter_clipped_reasons_keep_source_graphemes_separated() {
        for (text, expected) in [
            (format!("🇦\u{200b}🇧{}", "x".repeat(40)), "🇦🇧"),
            (format!("🇦\t🇧{}", "x".repeat(40)), "🇦🇧"),
            (format!("a\u{200b}b{}", "x".repeat(40)), "ab"),
        ] {
            let words = super::aged(&text, Some(240), 20);
            let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
            super::put(
                &mut buf,
                0,
                0,
                &words,
                20,
                ratatui_core::style::Style::default(),
            );
            let cells = line(&buf, 0).concat();
            assert_eq!(
                cells,
                format!("{expected}{}… · 4m", "x".repeat(12)),
                "{text}"
            );
        }
    }

    /// R1: two session rows under `look`, `unread` marked; the rest of the
    /// screen rides a stock shot (its selection is api, as here).
    fn laid_cue(look: &Look, unread: Vec<String>) -> Buffer {
        let shot = Shot::new();
        let rows = vec![row("api", 1, false), row("beta", 2, false)];
        let fleet = Fleet {
            rows,
            home: Some("api".to_owned()),
        };
        let mut model = Model::new(&fleet);
        model.set_unread(unread.into_iter().collect());
        let base = shot.screen(browsing(""));
        let screen = Screen {
            fleet: &fleet,
            model: &model,
            look: Some(*look),
            ..base
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
        super::draw_with_layout(&screen, super::Wait::default(), &mut buf);
        buf
    }

    /// `name`'s row: its y and the cue cell at col 1.
    fn cue_cell(buf: &Buffer, name: &str) -> (u16, String) {
        for y in 0..buf.area.height {
            if line(buf, y)[4..4 + name.len()].concat() == name {
                return (y, buf[(1, y)].symbol().to_owned());
            }
        }
        panic!("row {name} not drawn");
    }

    /// The cue glyph reads in both palettes and in ASCII, and it moves no column.
    #[test]
    fn unread_cue_cell_reads_in_every_palette_and_ascii_without_shifting() {
        for (icons, palette, glyph) in [
            ("on", "darcula", "▸"),
            ("on", "neutral", "▸"),
            ("on", "warm", "▸"),
            ("off", "darcula", ">"),
        ] {
            let look = Look::read(icons, palette, "on", "on");
            let buf = laid_cue(&look, vec!["beta".to_owned()]);
            assert_eq!(cue_cell(&buf, "beta").1, glyph, "{icons}/{palette}");
            let blank = cue_cell(&buf, "api").1;
            assert_eq!(blank, " ", "{icons}/{palette}: control");
            let (y, _) = cue_cell(&buf, "beta");
            assert_eq!(
                buf[(2, y)].symbol(),
                Mark::Working.glyph(icons == "on"),
                "mark col"
            );
            assert_eq!(line(&buf, y)[4..8].concat(), "beta", "name col");
        }
    }

    /// R3: the gear click covers the gear cell and its leading blank, no
    /// further; drawn cells are exactly what base draws.
    #[test]
    fn gear_click_covers_the_leading_blank_and_no_further() {
        let shot = Shot::new();
        let at_mouse = |col: u16| Mouse {
            kind: MouseKind::Click,
            column: col,
            row: 0,
        };
        let settings = |layout: &super::Layout, col: u16| {
            matches!(layout.hit(at_mouse(col)), Some(super::Hit::Settings))
        };
        let gear = "\u{2699}";
        let tail = format!("{} {gear}", crate::version_line());
        let head = "  Enter writes · ? keys";
        let full = head.chars().count() + 2 + tail.chars().count();
        for (width, drawn) in [(full, tail.as_str()), (full - 1, gear)] {
            let at = u16::try_from(width).unwrap_or(0);
            let (row, layout) = shot.footer_at(at, browsing(""), true);
            let pad = " ".repeat(width - head.chars().count() - drawn.chars().count());
            let want = format!("{head}{pad}{drawn}");
            assert_eq!(row, want, "w={width}: drawn unchanged");
            assert!(settings(&layout, at - 1), "w={width}: gear cell");
            assert!(settings(&layout, at - 2), "w={width}: leading blank");
            assert!(!settings(&layout, at - 3), "w={width}: two left is not");
        }
        let at = u16::try_from(head.chars().count() + 2).unwrap_or(0);
        let (row, layout) = shot.footer_at(at, browsing(""), true);
        assert!(!row.contains(gear), "narrow: no gear drawn");
        assert!(!settings(&layout, at - 1), "narrow: no target");
    }

    /// I-R1: a kept quota body draws its line first, in the stale tone, above the kept
    /// rows; without the marker, and on any other tab, no such line is drawn.
    #[test]
    fn a_kept_quota_body_draws_its_line_first_in_the_stale_tone() {
        use crate::app::model::SettingsTab;
        let look = Look::read("on", "darcula", "on", "on");
        let paint = Paint::of(Some(&look));
        let line = "last updated 3m ago — refreshing";
        let mut bodies = crate::app::settings::SettingsBodies {
            quota: Some(crate::quota::view::unavailable()),
            quota_kept: Some(line.to_owned()),
            ..crate::app::settings::SettingsBodies::default()
        };
        let kept = super::settings_body(SettingsTab::Quota, &bodies, paint, 80, true);
        let stale = paint.fg(|p| p.stale);
        assert_ne!(stale, paint.fg(|p| p.text), "the stale tone is its own");
        assert_eq!(kept[0], (line.to_owned(), stale), "{kept:?}");
        let rows = kept[1..]
            .iter()
            .any(|(text, _)| text.contains("quota: unavailable"));
        assert!(rows, "the kept rows follow: {kept:?}");
        let config = super::settings_body(SettingsTab::Config, &bodies, paint, 80, true);
        assert!(!config.iter().any(|(text, _)| text.contains("refreshing")));
        bodies.quota_kept = None;
        let fresh = super::settings_body(SettingsTab::Quota, &bodies, paint, 80, true);
        assert_eq!(fresh[..], kept[1..], "no marker, no line");
    }

    /// Hangul jamo/syllable widths and the zero-width barrier, cut whole:
    /// jamo U+1100 is 2 cells, ZWSP 0, jungseong U+1161 combining 0, syllable
    /// U+AC00 2 cells. The ZWSP/tab breaks L+V conjoining; kept zero-width
    /// bytes stay in the cut.
    #[test]
    fn unicode_barrier_rows_clip_whole_graphemes_and_age() {
        // (text, room, expected); content + … + tail must fit `room`.
        for (text, room, expected) in [
            (
                "ᄀ\u{200b}가".to_owned() + &"x".repeat(40),
                20,
                "ᄀ\u{200b}가".to_owned() + &"x".repeat(10) + "… · 4m",
            ),
            (
                "ᄀ\u{200b}ᅡ".to_owned() + &"x".repeat(40),
                20,
                "ᄀ\u{200b}ᅡ".to_owned() + &"x".repeat(12) + "… · 4m",
            ),
            (
                "ᄀ\t가".to_owned() + &"x".repeat(40),
                20,
                "ᄀ\t가".to_owned() + &"x".repeat(10) + "… · 4m",
            ),
            // A combining jungseong kept whole: 14 + 0 + … + tail.
            (
                "x".repeat(14) + "ᅡ" + &"x".repeat(40),
                20,
                "x".repeat(14) + "ᅡ… · 4m",
            ),
            // A wide syllable never split: 13 cells used, needs 2 of 1 left.
            (
                "x".repeat(13) + "가" + &"x".repeat(40),
                20,
                "x".repeat(13) + "… · 4m",
            ),
            // A halfwidth cluster (halfwidth ka + voicing mark) is one 2-cell
            // grapheme (mark compensated per ratatui-core cell_width).
            (
                "x".repeat(12) + "ｶﾞ" + &"x".repeat(40),
                20,
                "x".repeat(12) + "ｶﾞ… · 4m",
            ),
            // A tab is filtered from the stream (0 cells); neighbours never glue.
            (
                "a\tb".to_owned() + &"x".repeat(40),
                20,
                "a\tb".to_owned() + &"x".repeat(12) + "… · 4m",
            ),
        ] {
            let words = super::aged(&text, Some(240), room);
            assert_eq!(words, expected, "{text:?}/{room}");
            assert!(super::drawn_cols(&words) <= room, "{text:?}/{room}");
        }
    }

    /// Control graphemes never reach cells and zero-width ones take none:
    /// aZWSPbTABc draws abc, a wide jamo itself plus its reset continuation,
    /// a halfwidth cluster whole from its first cell. Pure `aged()` cannot prove
    /// it keeps the raw bytes.
    #[test]
    fn zero_width_source_cells_stand_as_blanks_without_conjoining() {
        use ratatui_core::buffer::Buffer;
        use ratatui_core::layout::Rect;

        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        super::put_line(
            &mut buf,
            0,
            0,
            &ratatui_core::text::Line::from("a\u{200b}b\tc"),
            10,
        );
        assert_eq!(line(&buf, 0)[..5].concat(), "abc  ");
        super::put_line(
            &mut buf,
            0,
            1,
            &ratatui_core::text::Line::from("ᄀ\u{200b}ᅡ"),
            10,
        );
        assert_eq!(line(&buf, 1)[..3].concat(), "ᄀ  ");
        super::put_line(&mut buf, 0, 2, &ratatui_core::text::Line::from("ｶﾞ"), 10);
        assert_eq!(line(&buf, 2)[..1].concat(), "ｶﾞ");
    }

    // agentwrap frozen behavioral acceptance — r5 fragment for src/app/draw.rs
    // INSERT: immediately before the closing brace of `mod tests` in src/app/draw.rs
    // (the module containing a_cut_body_names_its_hidden_rows_in_the_dim_hue,
    // draw.rs:3311 after the driver rebase onto 54f96696).
    // Supersedes frozen/r4/draw-agentwrap-tests.rs (kept immutable). Folds colead
    // gate review-20261019T081224Z-16d86b15 + oracle-spec-r5-delta eeb164845370:
    // B1 the fitting wide model is SEVEN glyphs / 14 cells (r4 mislabelled it
    //     5/10): joined 14+1+5 = 20 = free — the wide exact-fit case; separator
    //     x 22, effort x 23..27 (the wrap case's 22/23..27 was already right);
    // B2 the floor literals were 25/26 chars, now 24/25: capacity-24 whole fit,
    //     25 closes at 23 cells + …;
    // I1 casts are checked (u16::try_from, the codebase's own test idiom);
    // I2 no unused bindings (the I4 test's rule binding removed);
    // I3 doc comments carry no bare identifiers or shouty prose for doc_markdown;
    // I4 region guards compare FULL Cells (symbol + style) and now include the
    //     right gap columns 42..44 beside the gutter and the border/chat columns.
    //
    // Derived geometry (binding, verified): LEFT = 2 (draw.rs:88); room = rule - 4
    // (draw.rs:740-741); rule = sidebar_width(area, split), split.sidebar pinned
    // via grab/drag_to/let_go (model.rs:546-565); clamp-to-30 and area-fallback
    // semantics owner-anchored (provenance-addendum.md). seat_row inline:
    // mark@0, blank@1, name@2, meta start = 2 + column + 2, state tail =
    // 1 + state_cols. aa/bb/cc (S=44): column 2, start 6, free 20, capacity 38,
    // meta x 8, name x 4. sNN (S=44): column 3, free 19. Sweep at draw(100, 21):
    // room 30, free 0, name clipped to 13 chars + ellipsis, every seat wraps
    // post-fix, shown 13, more-line (20, 2). shot.draw(.., None) frames: mono
    // render, icons on; Look::read("off", "darcula", "on", "on") frames: colour
    // darcula palette, icons off; the palette lift sets the selected background
    // and repaints dim words in the selected ink, never the reverse modifier
    // (draw.rs:399-406).

    /// Seats with controlled label fields: (name, model, effort, client).
    fn label_facts(specs: &[(&str, &str, &str, &str)]) -> Facts {
        let Facts::Seats { id, agents } = sweep_seats() else {
            unreachable!("the sweep draws seats");
        };
        let base = agents[0].clone();
        Facts::Seats {
            id,
            agents: specs
                .iter()
                .map(|(name, model, effort, client)| PickerAgent {
                    name: (*name).to_owned(),
                    state: FIT_STATE.to_owned(),
                    model: (*model).to_owned(),
                    effort: (*effort).to_owned(),
                    client: (*client).to_owned(),
                    ..base.clone()
                })
                .collect(),
        }
    }

    /// Pin the sidebar's width: grab the edge, drag it to `size`, let go.
    fn with_sidebar(shot: &mut Shot, size: u16) {
        shot.model.grab(Drag {
            edge: Edge::Sidebar,
            from: 0,
            at: 0,
        });
        let _ = shot.model.drag_to(size);
        let _ = shot.model.let_go();
    }

    /// R2/R6: the joined label filling `free` exactly stays on one line.
    #[test]
    fn exact_fit_stays_one_line_with_the_full_label() {
        let mut shot = Shot::new();
        // free = 20; 14 + 1 + 5 = 20.
        shot.agents = Some(label_facts(&[
            ("aa", "m-model-fourte", "e1234", "claude"),
            ("bb", "", "", "claude"),
            ("cc", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, layout) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert_eq!(layout.seats, ["aa", "bb", "cc"]);
        let one = line(&buf, rule + 1).concat();
        assert!(one.contains("m-model-fourte e1234"), "{one}");
        assert!(one.contains(FIT_STATE), "{one}");
        assert_eq!(buf[(8, rule + 1)].symbol(), "m", "the meta column");
        assert_eq!(buf[(29, rule + 1)].symbol(), "u", "the state word starts");
        assert_eq!(buf[(41, rule + 1)].symbol(), "d", "the state word ends");
        assert!(
            line(&buf, rule + 2).concat().contains("bb"),
            "one row a seat"
        );
    }

    /// R1/R3/R6: one cell over `free` moves the COMPLETE label to row two —
    /// today the effort half is silently lost on row one (RED: effort-missing).
    #[test]
    fn one_cell_deficit_moves_the_complete_label_to_row_two() {
        let mut shot = Shot::new();
        // free = 20; 15 + 1 + 5 = 21 = free + 1.
        shot.agents = Some(label_facts(&[
            ("aa", "m-model-fifteen", "e1234", "claude"),
            ("bb", "", "", "claude"),
            ("cc", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, layout) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        let head = line(&buf, rule + 1).concat();
        assert!(
            !head.contains("m-model-fifteen"),
            "no meta on row one: {head}"
        );
        let tail = line(&buf, rule + 2).concat();
        assert!(
            tail.contains("m-model-fifteen e1234"),
            "full label on row two: {tail}"
        );
        assert_eq!(
            buf[(2, rule + 2)].symbol(),
            " ",
            "no mark on the continuation"
        );
        assert_eq!(buf[(3, rule + 2)].symbol(), " ", "blank under the blank");
        assert_eq!(buf[(4, rule + 2)].symbol(), "m", "the continuation x is 4");
        let click = Mouse {
            kind: MouseKind::Click,
            column: 8,
            row: rule + 2,
        };
        assert!(matches!(layout.hit(click), Some(super::Hit::Seat(name)) if name == "aa"));
        assert_eq!(layout.seats, ["aa", "bb", "cc"], "logical seats, deduped");
        assert!(
            line(&buf, rule + 3).concat().contains("bb"),
            "mixed geometry"
        );
    }

    /// R1/R3/R4: the model alone over `free` wraps instead of dropping the whole
    /// label (today: meta = None, nothing drawn — RED: model-missing).
    #[test]
    fn model_only_overflow_wraps_instead_of_dropping() {
        let mut shot = Shot::new();
        // model 21 cells > free 20; effort empty.
        shot.agents = Some(label_facts(&[
            ("aa", "m12345678901234567890", "", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            !line(&buf, rule + 1)
                .concat()
                .contains("m12345678901234567890"),
            "no label on row one"
        );
        assert!(
            line(&buf, rule + 2)
                .concat()
                .contains("m12345678901234567890"),
            "the full model on row two"
        );
    }

    /// R7 case (c): the joined label exceeds row-2 capacity while the model alone
    /// fits it: row two keeps the FULL model and deliberately omits the effort,
    /// which appears nowhere (today: both dropped — RED).
    #[test]
    fn row_two_capacity_keeps_the_full_model_and_omits_the_effort() {
        let mut shot = Shot::new();
        // model 12, effort 30 -> joined 43 > 38 = row-2 capacity; model 12 <= 38.
        shot.agents = Some(label_facts(&[
            (
                "aa",
                "m12cellwords",
                "e12345678901234567890123456789",
                "claude",
            ),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            line(&buf, rule + 2).concat().contains("m12cellwords"),
            "the full model on row two"
        );
        assert!(
            !line(&buf, rule + 1).concat().contains("e1234567890")
                && !line(&buf, rule + 2).concat().contains("e1234567890"),
            "the effort is deliberately omitted, nowhere drawn"
        );
    }

    /// R7 case (d): the model alone exceeds row-2 capacity: row two fits it at a
    /// grapheme and closes on an ellipsis — never blank, never lossless (RED).
    /// The cut reserves one cell for the ellipsis: 37 cells of text + … = 38.
    #[test]
    fn model_past_row_two_capacity_closes_on_an_ellipsis() {
        let mut shot = Shot::new();
        // model 41 cells > 38.
        let model = "m1234567890123456789012345678901234567890";
        shot.agents = Some(label_facts(&[
            ("aa", model, "", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        let tail = line(&buf, rule + 2).concat();
        let fitted = format!("{}…", &model[..37]);
        assert!(
            tail.contains(&fitted),
            "the 37-cell prefix closed on …: {tail}"
        );
        assert!(!tail.contains(model), "the full model does not fit");
        assert!(tail.trim().len() > 1, "never blank");
    }

    /// R4 (colead pin): an effort-only seat over `free` wraps and keeps a
    /// truthful nonblank FITTED label (today: silently dropped — RED).
    #[test]
    fn effort_only_overflow_keeps_a_fitted_nonblank_label() {
        let mut shot = Shot::new();
        // effort 21 cells > free 20; model and client empty.
        shot.agents = Some(label_facts(&[
            ("aa", "", "e12345678901234567890", ""),
            ("bb", "", "", ""),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        let tail = line(&buf, rule + 2).concat();
        assert!(
            tail.contains("e12345678901234567890"),
            "the full effort: {tail}"
        );
        assert!(tail.trim().len() > 1, "never blank");
    }

    /// R4/R7: an effort-only seat past row-2 capacity is fitted, not blanked.
    #[test]
    fn effort_only_past_row_two_capacity_is_fitted_not_blanked() {
        let mut shot = Shot::new();
        // effort 50 cells > 38; fitted = 37 cells + ….
        let effort = "e1234567890123456789012345678901234567890123456789";
        shot.agents = Some(label_facts(&[("aa", "", effort, ""), ("bb", "", "", "")]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        let tail = line(&buf, rule + 2).concat();
        assert!(
            tail.contains(&format!("{}…", &effort[..37])),
            "fitted: {tail}"
        );
        assert!(!tail.contains(effort), "the full effort does not fit");
        assert!(tail.trim().len() > 1, "never blank");
    }

    /// R4: the client fallback is the label source too: it fits on row one, and
    /// over `free` it wraps whole (today the overflow drops it — RED).
    #[test]
    fn client_fallback_label_wraps_instead_of_dropping() {
        let mut shot = Shot::new();
        // client 21 cells > free 20; model and effort empty.
        shot.agents = Some(label_facts(&[
            ("aa", "", "", "c12345678901234567890"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            !line(&buf, rule + 1)
                .concat()
                .contains("c12345678901234567890"),
            "no label on row one"
        );
        assert!(
            line(&buf, rule + 2)
                .concat()
                .contains("c12345678901234567890"),
            "the full client label on row two"
        );
        // The fitting fallback stays on row one (today's contract, unchanged).
        let mut shot = Shot::new();
        shot.agents = Some(label_facts(&[("aa", "", "", "claude")]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(line(&buf, rule + 1).concat().contains("claude"));
    }

    /// R9 (colead I2): a wide label is measured in display cells and EVERY painted
    /// cell is asserted: each wide glyph at its even column, a blank trailing cell
    /// after each, then the separator and the effort. A wide glyph's trailing cell
    /// is blank, so no contiguous run ever appears in a line concat.
    #[test]
    fn wide_labels_are_measured_in_display_cells() {
        let mut shot = Shot::new();
        // Wrap half: 9 wide chars = 18 cells; 18 + 1 + 5 = 24 > 20.
        let model = "モデルモデルモデル";
        shot.agents = Some(label_facts(&[
            ("aa", model, "e1234", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            !line(&buf, rule + 1).concat().contains('モ'),
            "no wide label on row one"
        );
        for (at, ch) in model.chars().enumerate() {
            let at = u16::try_from(at).expect("small index");
            assert_eq!(
                buf[(4 + 2 * at, rule + 2)].symbol(),
                ch.to_string(),
                "glyph {at}"
            );
            assert_eq!(
                buf[(5 + 2 * at, rule + 2)].symbol(),
                " ",
                "trailing cell {at}"
            );
        }
        assert_eq!(buf[(22, rule + 2)].symbol(), " ", "the separator");
        for (at, ch) in "e1234".chars().enumerate() {
            let at = u16::try_from(at).expect("small index");
            assert_eq!(
                buf[(23 + at, rule + 2)].symbol(),
                ch.to_string(),
                "effort glyph {at}"
            );
        }
        // Fit half: SEVEN wide chars = 14 cells; 14 + 1 + 5 = 20 = free: the wide
        // exact-fit case, separator x 22, effort x 23..27.
        let mut shot = Shot::new();
        let model = "モデルモデルモ";
        shot.agents = Some(label_facts(&[("aa", model, "e1234", "claude")]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        for (at, ch) in model.chars().enumerate() {
            let at = u16::try_from(at).expect("small index");
            assert_eq!(
                buf[(8 + 2 * at, rule + 1)].symbol(),
                ch.to_string(),
                "glyph {at}"
            );
            assert_eq!(
                buf[(9 + 2 * at, rule + 1)].symbol(),
                " ",
                "trailing cell {at}"
            );
        }
        assert_eq!(buf[(22, rule + 1)].symbol(), " ", "the separator");
        for (at, ch) in "e1234".chars().enumerate() {
            let at = u16::try_from(at).expect("small index");
            assert_eq!(
                buf[(23 + at, rule + 1)].symbol(),
                ch.to_string(),
                "effort glyph {at}"
            );
        }
    }

    /// R9: combining marks cost no cells: a label of 12 cells carrying a
    /// combining mark at the exact-fit boundary stays on one line.
    #[test]
    fn combining_marks_cost_no_cells_at_the_exact_fit_boundary() {
        let mut shot = Shot::new();
        // "e" + U+0301 = 1 cell, plus 11 digits = 12 cells; 12 + 1 + 7 = 20 = free.
        shot.agents = Some(label_facts(&[
            ("aa", "e\u{301}23456789012", "e123456", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            line(&buf, rule + 1)
                .concat()
                .contains("e\u{301}23456789012 e123456")
        );
        assert!(line(&buf, rule + 2).concat().contains("bb"), "one row");
    }

    /// R5/D1 + N1: every name keeps the roster column (abs x 4) and the
    /// continuation text begins at the owner-pinned abs x 4 under the name.
    #[test]
    fn names_stay_aligned_and_the_continuation_starts_at_abs_x_four() {
        let mut shot = Shot::new();
        shot.agents = Some(label_facts(&[
            ("aa", "m-model-fifteen", "e1234", "claude"),
            ("bb", "", "", "claude"),
            ("cc", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        for y in [rule + 1, rule + 3, rule + 4] {
            assert_eq!(buf[(3, y)].symbol(), " ", "blank before the name");
            assert!(buf[(4, y)].symbol() != " ", "the name at x 4");
        }
        assert_eq!(buf[(3, rule + 2)].symbol(), " ", "blank before the label");
        assert_eq!(buf[(4, rule + 2)].symbol(), "m", "the label at x 4");
    }

    /// R10: the cursor glyph rides the head row only; the viewed seat's lift
    /// covers BOTH rows. The plain frame (no look) is the mono render: icons on,
    /// reverse-video ground. The `Look::read` frame is the colour darcula palette
    /// with icons off: the palette lift sets the selected background and repaints
    /// dim words in the selected ink; it never sets the reverse modifier
    /// (draw.rs:399-406).
    #[test]
    fn cursor_and_lift_cover_the_wrapped_seat() {
        let mut shot = Shot::new();
        shot.agents = Some(label_facts(&[
            ("aa", "m-model-fifteen", "e1234", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 44);
        shot.model.set_focus(Some("aa".to_owned()));
        // The mono frame: icons on, reverse-video ground.
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert_eq!(marked(&buf, "▹"), [rule + 1], "the cursor on the head row");
        let viewing = Composer::Viewing { seat: "aa" };
        let (buf, _) = framed_as(&shot, 160, None, viewing);
        assert_eq!(marked(&buf, "▸"), [rule + 1], "the lift glyph");
        assert!(buf[(8, rule + 1)].modifier.contains(Modifier::REVERSED));
        assert!(
            buf[(8, rule + 2)].modifier.contains(Modifier::REVERSED),
            "mono reverses both rows"
        );
        // The colour frame: ASCII glyphs, no reverse modifier.
        let look = Look::read("off", "darcula", "on", "on");
        let (buf, _) = framed(&shot, 160, Some(&look));
        assert_eq!(marked(&buf, "-"), [rule + 1], "the ASCII cursor");
        let (buf, _) = framed_as(&shot, 160, Some(&look), viewing);
        assert_eq!(marked(&buf, ">"), [rule + 1], "the ASCII lift glyph");
        assert_eq!(
            buf[(8, rule + 1)].bg,
            super::colour(crate::theme::Palette::DARCULA.selected),
            "the selected ground on the head row"
        );
        assert_eq!(
            buf[(8, rule + 2)].bg,
            super::colour(crate::theme::Palette::DARCULA.selected),
            "the selected ground on the continuation"
        );
        assert_eq!(
            buf[(4, rule + 2)].fg,
            super::colour(crate::theme::Palette::DARCULA.selected_ink),
            "dim words lift into the selected ink"
        );
        assert!(
            !buf[(8, rule + 2)].modifier.contains(Modifier::REVERSED),
            "colour never reverses"
        );
    }

    /// R8/D3 + R12: the viewport may open on a continuation: that row keeps the
    /// cursor glyph and the seat's hit (precondition: the body really wrapped and
    /// cut, so a false green cannot hide a missing second row). Wrapper labels
    /// are 23 cells over the three-letter seat names' free 19.
    #[test]
    fn the_viewport_may_open_on_a_continuation_keeping_glyph_and_hit() {
        const WRAP: &str = "m1234567890123456789012";
        let mut shot = Shot::new();
        let specs: Vec<(&str, &str, &str, &str)> = (0..40)
            .map(|at| {
                let name: &'static str = match at {
                    0 => "aa",
                    39 => "zz",
                    _ => Box::leak(format!("s{at:02}").into_boxed_str()),
                };
                let model = if at == 0 || at == 39 { WRAP } else { "" };
                (name, model, "", "claude")
            })
            .collect();
        shot.agents = Some(label_facts(&specs));
        with_sidebar(&mut shot, 44);
        shot.model.set_focus(Some("aa".to_owned()));
        // aa occupies body rows 0-1; its continuation is body row 1.
        shot.model.settle(None, Some(1));
        let (buf, layout) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            layout.body_max >= 1,
            "precondition: the body wrapped and cut"
        );
        assert_eq!(
            buf[(4, rule + 1)].symbol(),
            "m",
            "the first visible row is the continuation, not the head"
        );
        assert!(
            line(&buf, rule + 1).concat().contains(WRAP),
            "the label is drawn"
        );
        assert_eq!(
            marked(&buf, "▹"),
            [rule + 1],
            "the glyph rides the continuation"
        );
        let click = Mouse {
            kind: MouseKind::Click,
            column: 8,
            row: rule + 1,
        };
        assert!(matches!(layout.hit(click), Some(super::Hit::Seat(name)) if name == "aa"));
    }

    /// R8/D3 + R14: the body scrolls to its last row: the more-line then names
    /// only rows above, and the LAST agent's continuation (the last agent wraps)
    /// is fully visible above the more-line.
    #[test]
    fn the_body_reaches_its_last_row_and_names_nothing_below() {
        const WRAP: &str = "m1234567890123456789012";
        let mut shot = Shot::new();
        let specs: Vec<(&str, &str, &str, &str)> = (0..40)
            .map(|at| {
                let name: &'static str = match at {
                    0 => "aa",
                    39 => "zz",
                    _ => Box::leak(format!("s{at:02}").into_boxed_str()),
                };
                let model = if at == 0 || at == 39 { WRAP } else { "" };
                (name, model, "", "claude")
            })
            .collect();
        shot.agents = Some(label_facts(&specs));
        with_sidebar(&mut shot, 44);
        let (_, layout) = framed(&shot, 160, None);
        let top = layout.body_max;
        assert!(top >= 1, "precondition: the body wrapped and cut");
        shot.model.settle(None, Some(top));
        let (buf, layout) = framed(&shot, 160, None);
        assert_eq!(layout.body_top, Some(top), "the last row is reachable");
        let more = line(&buf, 44).concat();
        assert!(
            more.contains('↑') && !more.contains('↓'),
            "nothing hides below: {more}"
        );
        let last = line(&buf, 43).concat();
        assert!(last.contains(WRAP), "zz's continuation visible: {last}");
        assert_eq!(buf[(4, 43)].symbol(), "m", "the continuation at x 4");
    }

    /// R8/D3 + R14 (colead I4): the viewport may END on a head row whose one
    /// continuation is hidden: settling to one under the frame's last-row bound
    /// makes the LAST agent's head the final drawn row for EVERY shown, so
    /// exactly one row hides below.
    #[test]
    fn a_head_row_may_end_the_viewport_with_its_continuation_hidden() {
        const WRAP: &str = "m1234567890123456789012";
        let mut shot = Shot::new();
        let specs: Vec<(&str, &str, &str, &str)> = (0..40)
            .map(|at| {
                let name: &'static str = match at {
                    0 => "aa",
                    39 => "zz",
                    _ => Box::leak(format!("s{at:02}").into_boxed_str()),
                };
                let model = if at == 0 || at == 39 { WRAP } else { "" };
                (name, model, "", "claude")
            })
            .collect();
        shot.agents = Some(label_facts(&specs));
        with_sidebar(&mut shot, 44);
        let (_, layout) = framed(&shot, 160, None);
        let top = layout.body_max;
        assert!(top >= 2, "precondition: the body wrapped and cut");
        shot.model.settle(None, Some(top - 1));
        let (buf, layout) = framed(&shot, 160, None);
        let last = line(&buf, 43).concat();
        assert!(
            last.contains("zz"),
            "the head row is the last drawn row: {last}"
        );
        assert!(
            !last.contains(WRAP),
            "the continuation is not drawn: {last}"
        );
        let more = line(&buf, 44).concat();
        assert!(
            more.contains("↓ 1 more rows"),
            "exactly one row hides below: {more}"
        );
        let click = Mouse {
            kind: MouseKind::Click,
            column: 8,
            row: 43,
        };
        assert!(matches!(layout.hit(click), Some(super::Hit::Seat(name)) if name == "zz"));
    }

    /// R14 (oracle r2 B2 + r3 B1): at the default draw(100, 21) geometry room is
    /// 30, so the sweep's 30-cell names clip to 13 chars + ellipsis (budget 14)
    /// and the 16-cell label misses `free` (saturating to 0): POST-FIX every one
    /// of the 20 seats wraps and the body doubles: 2 x 20 - 14 shown = 26 hidden.
    /// The one-line summary gives the body one more row.
    /// The head is addressed by the rule, never by the full name.
    #[test]
    fn the_more_row_line_counts_rows_not_seats() {
        let mut shot = Shot::new();
        let Facts::Seats { id, agents } = sweep_seats() else {
            unreachable!("the sweep draws seats");
        };
        let mut agents = agents;
        for agent in &mut agents {
            agent.state = FIT_STATE.to_owned();
        }
        let agents: Vec<PickerAgent> = agents.into_iter().cycle().take(20).collect();
        shot.agents = Some(Facts::Seats { id, agents });
        let wrapped = shot.draw(100, 21, READ_ONLY);
        let rule = horizontal(&wrapped).expect("the Agents rule");
        assert!(
            line(&wrapped, rule + 1).concat().contains("a-seat-name-l…"),
            "precondition: the clipped head row"
        );
        assert!(
            line(&wrapped, rule + 2).concat().contains("Sonnet 5.5"),
            "precondition: the first seat wrapped"
        );
        assert_eq!(
            spot(&wrapped, "↓ 26 more rows"),
            Some((20, 2)),
            "2 x 20 - 14"
        );
    }

    /// R15/N2: the session rows above the Agents rule are untouched by wrapping:
    /// every FULL CELL from the top row through the rule, across the whole width,
    /// is identical between a wrapping frame and a fitting frame (colead I4/I6).
    #[test]
    fn session_rows_are_untouched_by_wrapping() {
        let mut wrapped = Shot::new();
        wrapped.fleet = frame_fleet();
        wrapped.agents = Some(label_facts(&[
            ("aa", "m-model-fifteen", "e1234", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut wrapped, 44);
        let (wbuf, _) = framed(&wrapped, 160, None);
        let above = horizontal(&wbuf).expect("the Agents rule");
        let mut fitting = Shot::new();
        fitting.fleet = frame_fleet();
        fitting.agents = Some(label_facts(&[
            ("aa", "m-model-fourte", "e1234", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut fitting, 44);
        let (buf, _) = framed(&fitting, 160, None);
        for y in 0..=above {
            for x in 0..buf.area.width {
                assert_eq!(
                    buf[(x, y)],
                    wbuf[(x, y)],
                    "the full cell is unchanged by wrapping at ({x}, {y})"
                );
            }
        }
    }

    /// R11 (colead I4/I6): nothing the sidebar writes lands outside its own
    /// cells: the gutter columns 0..2, the right gap 42..44 and every cell from
    /// the border column 44 rightward of the two label rows are FULL-CELL
    /// identical between the wrapping frame and a no-meta frame — whatever the
    /// chat draws there.
    #[test]
    fn wrapped_cells_stay_inside_the_body() {
        let mut wrapped = Shot::new();
        wrapped.agents = Some(label_facts(&[
            ("aa", "m-model-fifteen", "e1234", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut wrapped, 44);
        let (wbuf, _) = framed(&wrapped, 160, None);
        let rule = horizontal(&wbuf).expect("the Agents rule");
        let mut bare = Shot::new();
        bare.agents = Some(label_facts(&[
            ("aa", "", "", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut bare, 44);
        let (bbuf, _) = framed(&bare, 160, None);
        for y in [rule + 1, rule + 2] {
            for x in 0..2 {
                assert_eq!(
                    wbuf[(x, y)],
                    bbuf[(x, y)],
                    "the gutter is untouched at ({x}, {y})"
                );
            }
            for x in 42..wbuf.area.width {
                assert_eq!(
                    wbuf[(x, y)],
                    bbuf[(x, y)],
                    "the right gap, border and chat are untouched at ({x}, {y})"
                );
            }
        }
    }

    /// R4 (colead I5): a seat with NOTHING known — model, client and effort all
    /// empty — draws ONE row, invents no placeholder, and leaves the meta column
    /// and every later row where they were.
    #[test]
    fn fully_unknown_labels_draw_one_row_and_invent_nothing() {
        let mut shot = Shot::new();
        shot.agents = Some(label_facts(&[
            ("aa", "", "", ""),
            ("bb", "", "", ""),
            ("cc", "", "", ""),
        ]));
        with_sidebar(&mut shot, 44);
        let (buf, layout) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert_eq!(layout.seats, ["aa", "bb", "cc"], "one logical seat each");
        assert_eq!(buf[(8, rule + 1)].symbol(), " ", "no meta for aa");
        assert!(
            line(&buf, rule + 2).concat().contains("bb"),
            "bb follows directly"
        );
        assert_eq!(buf[(8, rule + 2)].symbol(), " ", "no meta for bb");
        assert!(
            line(&buf, rule + 3).concat().contains("cc"),
            "cc follows directly"
        );
        for y in [rule + 1, rule + 2, rule + 3] {
            assert!(
                !line(&buf, y).concat().contains('…'),
                "no ellipsis is invented at ({y})"
            );
        }
    }

    /// R7 (colead I3): the S = 30 sidebar floor pins the row-2 capacity boundary
    /// executably: room 26, capacity 24, free 6. A 24-cell model is drawn WHOLE;
    /// a 25-cell model closes on the ellipsis at 23 cells + …; a 6-cell joined
    /// label still fits row one at the floor; an UNDERSIZED request clamps to the
    /// floor instead of refusing, and the boundary holds there too.
    #[test]
    fn the_sidebar_floor_pins_the_row_two_capacity_boundary() {
        let full = "m12345678901234567890123";
        let mut shot = Shot::new();
        shot.agents = Some(label_facts(&[
            ("aa", full, "", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 30);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            line(&buf, rule + 2).concat().contains(full),
            "24 cells fit row two whole at the floor"
        );
        assert!(
            !line(&buf, rule + 2).concat().contains('…'),
            "no ellipsis at 24"
        );
        let over = "m123456789012345678901234";
        let mut shot = Shot::new();
        shot.agents = Some(label_facts(&[
            ("aa", over, "", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 30);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            line(&buf, rule + 2)
                .concat()
                .contains(&format!("{}…", &over[..23])),
            "25 cells close on the ellipsis at 23 + …"
        );
        assert!(
            !line(&buf, rule + 2).concat().contains(over),
            "the whole does not fit"
        );
        let mut shot = Shot::new();
        shot.agents = Some(label_facts(&[
            ("aa", "m12345", "", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 30);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            line(&buf, rule + 1).concat().contains("m12345"),
            "the 6-cell joined label fits row one at the floor"
        );
        assert!(
            line(&buf, rule + 2).concat().contains("bb"),
            "one row at the floor"
        );
        // The undersized request clamps to the 30 floor (owner anchor): the
        // capacity-24 whole fit still holds after requesting 20.
        let mut shot = Shot::new();
        shot.agents = Some(label_facts(&[
            ("aa", full, "", "claude"),
            ("bb", "", "", "claude"),
        ]));
        with_sidebar(&mut shot, 20);
        let (buf, _) = framed(&shot, 160, None);
        let rule = horizontal(&buf).expect("the Agents rule");
        assert!(
            line(&buf, rule + 2).concat().contains(full),
            "the request clamped to the floor: 24 cells still fit whole"
        );
    }
}
