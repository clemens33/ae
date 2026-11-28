//! The frame's hit map: what a click on a cell reaches, and the geometry the
//! frame drew that the mouse and the scroll bounds read back.

use std::ops::Range;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::{Position, Rect};

use crate::app::model::{ChatScroll, Columns, Edge, SettingsTab, Split};
use crate::app::select::{self, Map, Region};
use crate::app::settings::StatsTab;
use crate::console::input::Mouse;

use super::Ctx;
use super::geometry::{Stand, stand};
use super::paint::cells;

/// A click target recorded where its cells are drawn.
#[derive(Debug, Clone)]
pub(crate) enum Hit {
    Session(String),
    /// The stopped group's line: opens or closes the group.
    Stopped,
    /// The spawned group's line in the Agents body: folds or shows the group.
    Spawned,
    /// A drawn seat row: moves the highlight to that seat.
    Seat(String),
    Compose,
    /// The keys row's gear: toggles the settings overlay.
    Settings,
    /// A drawn Settings tab title.
    SettingsTab(SettingsTab),
    /// A drawn title of the Statistics tab's inner row.
    StatsTab(StatsTab),
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
    /// The `↓ newer turns` hint's drawn words: scroll to the newest turn.
    Newest,
    /// The hint row's orchestrator button: opens or hides the panel.
    Orchestrator,
    /// The panel header's `pane` word: opens the orchestrator's pane.
    PanelPane,
    /// The panel header's mic beside `pane`: asks the seat to dictate.
    PanelMic,
    /// The panel header's mode label: side or full.
    PanelMode,
    /// The main column header's zoom label: zoom the column or leave the zoom.
    Zoom,
}

/// A border as drawn: its edge, where it stands, its own cells and the cells
/// that grab it.
#[derive(Debug)]
pub(super) struct Border {
    edge: Edge,
    at: u16,
    cells: Rect,
    grab: Rect,
}

fn link_area(offset: usize, width: usize, (left, y, room): (u16, u16, u16)) -> Rect {
    let start = u16::try_from(offset).unwrap_or(u16::MAX);
    let width = u16::try_from(width)
        .unwrap_or(u16::MAX)
        .min(room.saturating_sub(start));
    Rect::new(left.saturating_add(start), y, width, 1)
}

/// One visible file occurrence, including the session incarnation's fold key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::l5_ui::app) struct FileHit {
    pub session: String,
    pub fold: u64,
    pub link: crate::app::lane::ReadLink,
}

/// The last frame's input geometry and chat scroll bounds.
#[derive(Debug, Default)]
pub(crate) struct Layout {
    pub(super) targets: Vec<(Rect, Hit)>,
    pub(super) borders: Vec<Border>,
    pub(super) area: Rect,
    pub(super) chat: Rect,
    pub page_rows: usize,
    pub(in crate::l5_ui::app) scroll: Option<ChatScroll>,
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
    /// Set by the frame for one draw: keep the Config tab's highlighted
    /// picker row on screen, moving the scroll the least it takes.
    pub settings_reveal: bool,
    /// The picker's heading row and highlighted row that request resolved to.
    pub(super) settings_keep: Option<(usize, usize)>,
    /// The first body row the open panel drew: the scroll the frame keeps.
    pub settings_start: usize,
    /// The session list's cells, the first session it drew, and the
    /// furthest first session it could draw.
    pub(super) list: Rect,
    pub list_start: Option<usize>,
    pub list_max: usize,
    /// The tab body's cells, the first row it drew, and the furthest.
    pub(super) body: Rect,
    pub body_top: Option<usize>,
    pub body_max: usize,
    /// What a drag may select in this frame.
    pub(crate) map: Map,
    /// The seats the tab body drew, in drawn order, and the one it highlighted.
    pub seats: Vec<String>,
    pub focused: Option<String>,
    /// Each card the frame drew, as the app read it then: a menu opened on
    /// this frame acts on that and on nothing a later read changed.
    pub(in crate::l5_ui::app) cards: std::collections::BTreeMap<String, crate::app::menu::Card>,
    /// The session the frame drew those seats for: a delayed click replayed
    /// against this frame opens only that session's seat, never a same-named
    /// one of the session selected since.
    pub seats_session: Option<String>,
    /// The links the frame drew, one run per link per row, clipped to the
    /// column: the backend wraps exactly these cells in OSC 8.
    pub(crate) links: Vec<crate::app::backend::Run>,
    pub(super) files: Vec<(Rect, FileHit)>,
    /// The cells the lane's rows drew in, for the keys that read its links.
    pub(in crate::l5_ui::app) lane_width: u16,
    /// The cells the open panel drew over in this frame, `None` while none
    /// did: a press made on this frame belongs to the panel there. The
    /// panel's own column records them as it draws, the main column's frame
    /// takes them over.
    pub(crate) panel: Option<Rect>,
    /// What the panel's `pane` word opens, as the frame that drew it read it:
    /// the incarnation the panel is bound to and the main seat that
    /// incarnation's roster named then. Set by the app after the draw, only
    /// for a panel bound to a running orchestrator.
    pub(in crate::l5_ui::app) pane: Option<(String, Option<crate::console::needs::SeatRef>)>,
}

impl Layout {
    /// The runs of the visible lane rows `links`, the first on row `y0`, left
    /// of the column at `left` and `room` cells wide: each link clipped to the
    /// cells the row actually draws.
    pub(super) fn link_runs(
        &mut self,
        links: &[crate::app::lane::Links],
        (left, y0, room): (u16, u16, u16),
    ) {
        for (step, row) in links.iter().enumerate() {
            for (offset, width, url) in row {
                let area = link_area(
                    *offset,
                    *width,
                    (left, y0.saturating_add(cells(step)), room),
                );
                if area.width == 0 {
                    continue;
                }
                self.links.push(crate::app::backend::Run {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    url: url.clone(),
                });
            }
        }
    }

    pub(super) fn file_runs(
        &mut self,
        files: &[crate::app::lane::FileRuns],
        ctx: &Ctx<'_, '_>,
        (left, y0, room): (u16, u16, u16),
    ) {
        for (step, reads) in files.iter().enumerate() {
            for (offset, width, link) in reads {
                let Some(at) = ctx.wait.keys.iter().position(|key| *key == link.turn) else {
                    continue;
                };
                let Some(fold) = ctx.wait.folds.get(at) else {
                    continue;
                };
                let area = link_area(
                    *offset,
                    *width,
                    (left, y0.saturating_add(cells(step)), room),
                );
                if area.width > 0 {
                    self.files.push((
                        area,
                        FileHit {
                            session: ctx.screen.model.selected().unwrap_or_default().to_owned(),
                            fold: *fold,
                            link: link.clone(),
                        },
                    ));
                }
            }
        }
    }

    /// Only cells still owned by a turn can activate a file: folds and
    /// overlays take priority, as do the border grabs.
    pub(in crate::l5_ui::app) fn file(&self, mouse: Mouse) -> Option<&FileHit> {
        if self.grab(mouse).is_some() || !matches!(self.hit(mouse), Some(Hit::Turn(_))) {
            return None;
        }
        self.files.iter().find_map(|(area, file)| {
            area.contains(Position::new(mouse.column, mouse.row))
                .then_some(file)
        })
    }

    /// Whether `mouse` is on a link cell a press does not read itself: a run
    /// the backend wraps in OSC 8, under `file`'s guards, off every file hit.
    pub(in crate::l5_ui::app) fn link_at(&self, mouse: Mouse) -> bool {
        let at = Position::new(mouse.column, mouse.row);
        self.grab(mouse).is_none()
            && matches!(self.hit(mouse), Some(Hit::Turn(_)))
            && self.file(mouse).is_none()
            && (self.links.iter()).any(|run| Rect::new(run.x, run.y, run.width, 1).contains(at))
    }

    pub(super) fn record(&mut self, buf: &Buffer, area: Rect, target: Hit) {
        self.targets.push((area.intersection(buf.area), target));
    }

    /// Record `target` over `area` ahead of every target under it: an
    /// overlay's cells are its own.
    pub(in crate::l5_ui::app) fn overlay(&mut self, buf: &Buffer, area: Rect, target: Hit) {
        let area = area.intersection(buf.area);
        if matches!(target, Hit::Modal) {
            self.uncover(area);
        }
        self.targets.insert(0, (area, target));
    }

    /// Drop the cells of `area`, which a dialog or the open panel paints whole,
    /// from the link runs: a run it covers wholly goes, one it cuts keeps what
    /// stands left and right of it, so the terminal never opens a link from the
    /// text drawn over it.
    pub(in crate::l5_ui::app) fn uncover(&mut self, area: Rect) {
        for run in std::mem::take(&mut self.links) {
            let end = run.x.saturating_add(run.width);
            let (left, start) = (area.x.saturating_sub(run.x), area.right().max(run.x));
            let row = Rect::new(run.x, run.y, run.width, 1);
            let parts = if row.intersects(area) {
                [
                    (run.x, left.min(run.width)),
                    (start, end.saturating_sub(start)),
                ]
            } else {
                [(run.x, run.width), (0, 0)]
            };
            for (x, width) in parts.into_iter().filter(|(_, width)| *width > 0) {
                self.links.push(crate::app::backend::Run {
                    x,
                    y: run.y,
                    width,
                    url: run.url.clone(),
                });
            }
        }
    }

    /// Drop the chat column's own targets, its turns, folds, composer and
    /// newer-turns hint, from the cells of `area`, which a column drawn over the
    /// main column paints whole: a press there is on none of them, and a border
    /// the press lands beside is free to take it.
    pub(in crate::l5_ui::app) fn cover_targets(&mut self, area: Rect) {
        self.targets.retain(|(cells, hit)| {
            !(cells.intersects(area)
                && matches!(
                    hit,
                    Hit::Compose | Hit::Turn(_) | Hit::Fold(_) | Hit::Newest
                ))
        });
    }

    /// The sessions whose cards the frame drew.
    pub(in crate::l5_ui::app) fn sessions(&self) -> impl Iterator<Item = &str> {
        self.targets.iter().filter_map(|(_, target)| match target {
            Hit::Session(name) => Some(name.as_str()),
            _ => None,
        })
    }

    /// The frame painted a modal whole.
    pub(in crate::l5_ui::app) fn shows_modal(&self) -> bool {
        (self.targets.iter()).any(|(_, target)| matches!(target, Hit::Modal))
    }

    /// Where the frame drew `name`'s card.
    pub(in crate::l5_ui::app) fn card_at(&self, name: &str) -> Option<Rect> {
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
    pub(super) fn selectable(&mut self, buf: &Buffer, panel: select::Panel, area: Rect) {
        let area = area.intersection(buf.area);
        if !area.is_empty() {
            self.map.regions.push(Region { panel, area });
        }
    }

    /// Let a drag select in the lane: `rows` rows below `top`, from `left`.
    /// The `bars` among the `shown` rows (from row `y0`) drew a mark bar in
    /// their first cell, which reads as a blank.
    pub(super) fn lane(
        &mut self,
        buf: &Buffer,
        (left, top, room, rows): (u16, u16, u16, usize),
        bars: &[usize],
        (shown, y0): (Range<usize>, u16),
    ) {
        self.lane_width = room;
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
    pub(super) fn border(&mut self, buf: &Buffer, edge: Edge, at: u16, cells: Rect) {
        let grab = match edge {
            Edge::Sidebar | Edge::Panel => {
                Rect::new(cells.x.saturating_sub(1), cells.y, 3, cells.height)
            }
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

    /// Record the border of a side panel drawn in `buf` over this main
    /// column's frame, so a press grabs it before the panel takes the press.
    pub(crate) fn panel_border(&mut self, buf: &Buffer, split: Split, columns: Columns) {
        if let Stand::Side { border, end } = stand(buf.area, split, columns) {
            let cells = Rect::new(border, 0, 1, buf.area.height);
            self.border(buf, Edge::Panel, end - border, cells);
        }
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
        self.scroll = None;
        self.max_scroll = 0;
        self.complete = true;
        self.turns.clear();
        self.folds.clear();
        self.cursor = None;
        // The overlay paints over every cell a link was drawn on: no run may
        // wrap its text.
        self.links.clear();
        self.settings_page_rows = 0;
        self.settings_max_scroll = 0;
        self.settings_keep = None;
        self.settings_start = 0;
        self.map = Map::default();
    }
}
