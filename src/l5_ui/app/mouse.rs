//! The mouse: press, drag, click and wheel routing, the drag state and its stretch of the layout.

use super::{App, Instant, Mouse, MouseKind, Position, apply, draw, link, model, select};

/// What a press on a link cell does when its un-dragged release lands there.
#[derive(Debug)]
pub(super) enum Press {
    /// Open this file occurrence in the reader.
    File(draw::FileHit),
    /// Show the hint that names the terminal's link modifier.
    Web,
}

/// A drag that began in a selectable region, from its press until its copy is
/// shipped or it is cleared.
#[derive(Debug)]
pub(super) struct Held {
    pub(super) sel: select::Selection,
    /// The model's [`model::Model::epoch`] when it began: any other is a panel
    /// change and ends it.
    pub(super) epoch: u64,
    /// The press toggled a turn's mark: `before` is put back at the first
    /// motion, when the press turns out to be a drag.
    pub(super) toggled: bool,
    pub(super) before: Option<u64>,
    /// The button is still down.
    pub(super) down: bool,
    pub(super) stage: Stage,
    /// The last mouse input of the drag.
    pub(super) touched: Instant,
}

/// Where a release's copy stands. It lives in [`Held`], so whatever ends the
/// selection ends the copy with it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) enum Stage {
    #[default]
    None,
    /// Released: the next frame paints the final range.
    Requested,
    /// What that frame highlighted, to ship once it is written.
    Ready(String),
}

impl App {
    /// A press acts on the frame on screen when it was made: one made before a
    /// frame replaced the layout is hit against the layout that frame replaced,
    /// so an ask in flight or a read since cannot move its target. A held wheel
    /// notch keeps replaying against the newest frame, which bounds it. One
    /// made on a frame the bound dropped is not taken, and the hint row says so.
    /// A right press opens the menu on the card its frame drew there.
    pub(super) fn mouse_made(&mut self, mouse: Mouse, origin: Instant) -> bool {
        if mouse.kind != MouseKind::Release {
            self.read_press = None;
        }
        if mouse.kind == MouseKind::Menu
            && !self.model.settings_open()
            && !self.model.peek_open()
            && !self.model.reader_open()
        {
            return self.menu_press(mouse, origin);
        }
        // Every press ends a selection first, a stale or refused one too: no
        // frame is kept to aim a drag at.
        let cleared = mouse.kind == MouseKind::Click && self.unselect();
        // A press made before the surface changed aims at cells that are
        // another column's now, the panel's own frames included.
        if mouse.kind == MouseKind::Click && self.zoom_gone(origin) {
            return self.frame_gone() || cleared;
        }
        let acted = match self.panel_made(mouse, origin) {
            Some(acted) => acted,
            None if self.covered_since(mouse, origin) => self.frame_gone(),
            None => self.frame_made(mouse, origin),
        };
        acted || cleared
    }

    /// Whether a press read at `origin` was made on a surface that is gone: at
    /// or before the instant the last surface change was published
    /// ([`App::zoom_cut`]), or while the zoom the keys changed is not drawn yet,
    /// the old frame's cells being the only ones the press can name.
    pub(super) fn zoom_gone(&self, origin: Instant) -> bool {
        self.zoom_cut.is_some_and(|cut| origin <= cut)
            || (!self.orch.active && {
                let columns = self.stand_columns();
                columns.surface() != self.model.columns().surface()
                    || self.layout.desk_replaces_panel()
                        != draw::desk_replaces_panel(
                            self.layout.area(),
                            self.model.split(),
                            columns,
                        )
                    || self.layout.desk_beside()
                        != draw::desk_beside(self.layout.area(), self.model.split(), columns)
            })
    }

    /// Whether `mouse`, a press made at `origin` on the main column, lands
    /// where the open panel draws now: its target is covered, so it never
    /// moves the keys or starts an entry there.
    fn covered_since(&self, mouse: Mouse, origin: Instant) -> bool {
        let at = Position::new(mouse.column, mouse.row);
        let on = |cells: Option<ratatui_core::layout::Rect>| {
            cells.is_some_and(|cells| cells.contains(at))
        };
        mouse.kind == MouseKind::Click && !on(self.layout_at(origin).panel) && on(self.layout.panel)
    }

    /// [`App::mouse_made`] against the main column's frames: a press made
    /// before a frame replaced the layout is hit against the layout that frame
    /// replaced, one made on a frame the bound dropped is not taken.
    pub(super) fn frame_made(&mut self, mouse: Mouse, origin: Instant) -> bool {
        if mouse.kind == MouseKind::Click && self.lost.is_some_and(|lost| origin <= lost) {
            return self.frame_gone();
        }
        let seen = (self.history.iter()).position(|(replaced, _)| origin <= *replaced);
        let Some(at) = seen.filter(|_| mouse.kind == MouseKind::Click) else {
            return self.mouse(mouse);
        };
        let older = std::mem::take(&mut self.history[at].1);
        let current = std::mem::replace(&mut self.layout, older);
        let acted = self.mouse_in(mouse, false);
        self.history[at].1 = std::mem::replace(&mut self.layout, current);
        acted
    }

    /// A press whose frame the bound dropped: not taken, and the hint row
    /// says so.
    pub(super) fn frame_gone(&mut self) -> bool {
        self.flash = Some((
            "click not taken: its frame is gone".to_owned(),
            Instant::now(),
        ));
        self.model.let_go();
        true
    }

    /// Every wake up to now was read: the layouts older than the oldest held key
    /// or the read that began the decoder's unfinished sequence (`begun`) are
    /// done with, and all of them are while no ask waits, since the wakes still
    /// unread follow its key.
    pub(super) fn forget(&mut self, begun: Option<Instant>) {
        if self.queued.is_empty() {
            let floor = (self.deferred.iter().map(|(_, at)| *at).chain(begun)).min();
            self.history
                .retain(|(replaced, _)| floor.is_some_and(|floor| *replaced >= floor));
        }
    }

    /// Mouse actions use the last drawn frame, in either input mode. Left
    /// motion moves a dragged border; every other event first ends a drag —
    /// its release, or a press or wheel after a release that never came.
    pub(super) fn mouse(&mut self, mouse: Mouse) -> bool {
        self.mouse_in(mouse, true)
    }

    /// [`App::mouse`] for a press made on the frame that is on screen (`fresh`)
    /// or on one a later frame replaced, which can arm no selection. A
    /// selection a panel change left behind goes first, and so does one a
    /// press ends.
    pub(super) fn mouse_in(&mut self, mouse: Mouse, fresh: bool) -> bool {
        if mouse.kind != MouseKind::Release {
            self.read_press = None;
        }
        let mut redraw = self.stale_selection();
        if mouse.kind == MouseKind::Click {
            redraw |= self.unselect();
        }
        redraw | self.route(mouse, fresh)
    }

    pub(super) fn route(&mut self, mouse: Mouse, fresh: bool) -> bool {
        if self.model.settings_open() {
            return self.settings_mouse(mouse);
        }
        if self.model.peek_open() {
            return self.peek_mouse(mouse, fresh);
        }
        if self.model.reader_open() {
            return self.reader_mouse(mouse, fresh);
        }
        if !self.orch.active
            && let Some(acted) = self.held_edge(mouse)
        {
            return acted;
        }
        if !self.orch.active && self.layout.in_desk(mouse) {
            return self.desk_mouse(mouse);
        }
        if !self.orch.active && self.on_panel(mouse) {
            return self.panel_route(mouse, fresh);
        }
        // A press beyond the drawn frame, before the redraw after a resize,
        // or on a frame below the size floor lands on no column and moves no
        // keys.
        let area = self.layout.area();
        let drawn = draw::floor_met(area) && area.contains(Position::new(mouse.column, mouse.row));
        let mut changed = false;
        if !self.orch.active && mouse.kind == MouseKind::Click && self.orch.panel.is_some() && drawn
        {
            changed = self.focus_desk(false);
            self.focus_column(false);
            // A full panel kept the keys: a press on a main column cell it
            // covers, aimed at a frame drawn before it turned full, acts on
            // nothing.
            let at = Position::new(mouse.column, mouse.row);
            let covered = self.panel_area().is_some_and(|cells| cells.contains(at));
            if self.panel_focused() && covered {
                return self.frame_gone();
            }
        }
        if mouse.kind == MouseKind::Drag {
            return self.drag(mouse) || self.stretch(mouse);
        }
        let ended = self.model.let_go() || changed;
        match mouse.kind {
            MouseKind::Click => self.clicked(mouse, fresh) || ended,
            MouseKind::WheelUp | MouseKind::WheelDown => {
                let (up, layout) = (mouse.kind == MouseKind::WheelUp, &self.layout);
                if layout.in_chat(mouse) {
                    self.model.wheel(up, layout.page_rows, layout.max_scroll);
                    return true;
                }
                let moved = if layout.in_list(mouse) {
                    let start = layout.list_start.unwrap_or(0);
                    self.model.wheel_list(up, start, layout.list_max)
                } else if layout.in_body(mouse) {
                    self.model.wheel_body(up, layout.body_max)
                } else {
                    false
                };
                moved || ended
            }
            MouseKind::Release => self.finish(mouse) || ended,
            MouseKind::Drag | MouseKind::Menu => ended,
        }
    }

    fn desk_mouse(&mut self, mouse: Mouse) -> bool {
        let ended = self.model.let_go();
        let acted = match mouse.kind {
            MouseKind::WheelUp | MouseKind::WheelDown => {
                self.scroll_desk(mouse.kind == MouseKind::WheelUp, false)
            }
            MouseKind::Click => {
                let held = self.held.take().is_some();
                let focused = !self.composing() && self.focus_desk(true);
                if self.layout.link_at(mouse) {
                    self.web_press(mouse);
                }
                focused || held
            }
            MouseKind::Release => self.finish(mouse),
            MouseKind::Drag | MouseKind::Menu => false,
        };
        acted || ended
    }

    fn web_press(&mut self, mouse: Mouse) {
        let at = Position::new(mouse.column, mouse.row);
        self.read_press = Some((Press::Web, at, self.model.epoch()));
    }

    /// A press: what it always did, then a selection armed where it landed.
    pub(super) fn clicked(&mut self, mouse: Mouse, fresh: bool) -> bool {
        let before = self.model.turn();
        let acted = if let Some(hit) = self.layout.file(mouse).cloned() {
            self.held = None;
            if self.file_list(&hit).is_some() {
                self.read_press = Some((
                    Press::File(hit),
                    Position::new(mouse.column, mouse.row),
                    self.model.epoch(),
                ));
            } else {
                self.changed_read();
            }
            true
        } else {
            let web = self.layout.link_at(mouse);
            let acted = self.click(mouse);
            if web {
                self.web_press(mouse);
            }
            acted
        };
        self.arm(mouse, before, fresh);
        acted
    }

    /// A press: grab the border under it, else act on the target drawn
    /// there. Any press ends a HELD entry.
    pub(super) fn click(&mut self, mouse: Mouse) -> bool {
        let held = self.held.take().is_some();
        if let Some((edge, at)) = self.layout.grab(mouse) {
            let from = match edge {
                model::Edge::Sidebar | model::Edge::Panel | model::Edge::Desk => mouse.column,
                model::Edge::List => mouse.row,
            };
            self.model.grab(model::Drag { edge, from, at });
            return true;
        }
        let Some(hit) = self.layout.hit(mouse) else {
            return held;
        };
        let was_writing = self.composing();
        let act = match hit {
            draw::Hit::Session(name) => {
                let start = self.layout.list_start.unwrap_or(0);
                let Some(act) = self.model.select_name(&self.fleet, &name, start) else {
                    return held;
                };
                self.write(false);
                act
            }
            draw::Hit::Stopped => return self.toggle_stopped() || held,
            draw::Hit::Spawned => return self.toggle_spawned() || held,
            draw::Hit::Compose if !self.composing() => {
                self.model
                    .key(model::Key::Compose, &self.fleet, self.can_compose())
            }
            draw::Hit::Compose => return false,
            draw::Hit::Seat(name) => {
                // The click opens only the seat it named, as the frame that
                // drew it bound it: a delayed click replayed after the seat
                // left, or after its session gave way to another, refuses by
                // that name instead of falling back to another highlight.
                let bound = self.model.selected().is_some_and(|session| {
                    self.layout.seats_session.as_deref() == Some(session)
                        && self.drawn.as_ref().is_some_and(|drawn| {
                            drawn.session == session
                                && drawn.rows.iter().any(|(row, _)| row == &name)
                        })
                });
                if !bound && self.model.selected().is_some() {
                    let line = format!("refused: peek {name}: the view changed");
                    self.flash = Some((line, Instant::now()));
                    return true;
                }
                self.focus_seat(Some(name));
                self.open_seat();
                return true;
            }
            draw::Hit::Settings => {
                self.open_settings();
                return true;
            }
            // `PanelPane` is the panel's own: its route takes the word before
            // the chat's, so the chat never sees it.
            draw::Hit::SettingsTab(_)
            | draw::Hit::StatsTab(_)
            | draw::Hit::SettingsClose
            | draw::Hit::PeekClose
            | draw::Hit::PanelDesk
            | draw::Hit::PanelDeskToggle
            | draw::Hit::PanelPane => {
                return held;
            }
            draw::Hit::Item(_) | draw::Hit::Modal => return held,
            draw::Hit::Turn(key) => {
                self.write(false);
                self.model
                    .set_turn((self.model.turn() != Some(key)).then_some(key));
                return true;
            }
            draw::Hit::Fold(key) => {
                self.toggle_fold(key);
                return true;
            }
            draw::Hit::Newest => self.model.newest(),
            draw::Hit::Orchestrator => {
                self.toggle_panel();
                return true;
            }
            draw::Hit::PanelMode => return self.flip_panel() || held,
            draw::Hit::Zoom => return self.click_zoom() || held,
        };
        apply(self, &act, Instant::now()).unwrap_or(false)
            || was_writing != self.composing()
            || held
    }

    /// Left motion: the dragged border follows it while the last frame drew
    /// that border; a border not drawn keeps the size it was given.
    pub(super) fn drag(&mut self, mouse: Mouse) -> bool {
        let Some(drag) = self.model.drag() else {
            return false;
        };
        if !self.layout.shows(drag.edge) {
            return false;
        }
        let area = self.layout.area();
        let split = self.model.split();
        if drag.edge == model::Edge::Desk && self.model.columns().zoomed() {
            return self
                .model
                .drag_to(draw::zoomed_desk_drag(drag, mouse, area, split));
        }
        let count = draw::list_len(&self.fleet, self.stopped());
        let size = draw::dragged_to(drag, mouse, area, split, count, self.desk_stands());
        self.model.drag_to(size)
    }

    /// The panel and board borders take presses before the panel does. Their
    /// motion and release stay here wherever the pointer goes, without moving
    /// the keys. A drag of the sidebar or list is the panel's once the pointer
    /// lands on it. `None` for every other event.
    pub(super) fn held_edge(&mut self, mouse: Mouse) -> Option<bool> {
        let right_edge = |edge| matches!(edge, model::Edge::Panel | model::Edge::Desk);
        let dragging = self.model.drag().is_some_and(|drag| right_edge(drag.edge));
        match mouse.kind {
            MouseKind::Click => {
                let (edge, at) = self.layout.grab(mouse)?;
                if !right_edge(edge) {
                    return None;
                }
                self.held = None;
                let from = mouse.column;
                self.model.grab(model::Drag { edge, from, at });
                Some(true)
            }
            MouseKind::Drag if dragging => Some(self.drag(mouse)),
            MouseKind::Release if dragging => Some(self.model.let_go()),
            _ => None,
        }
    }

    /// Begin a selection at the press `mouse` made on the frame on screen:
    /// not on a border's grab, not on a frame a later one replaced, and only
    /// where that frame recorded a selectable region. `before` is the marked
    /// turn from before the press.
    pub(super) fn arm(&mut self, mouse: Mouse, before: Option<u64>, fresh: bool) {
        let at = Position::new(mouse.column, mouse.row);
        let region = (fresh && self.layout.grab(mouse).is_none())
            .then(|| self.layout.map.region_at(at).cloned())
            .flatten();
        let Some(region) = region else {
            return;
        };
        let fp = select::fingerprint(&self.shot, region.area, &self.layout.map.blanks);
        let turn = self.layout.file(mouse).is_none()
            && matches!(self.layout.hit(mouse), Some(draw::Hit::Turn(_)));
        self.select = Some(Held {
            sel: select::Selection::start(&region, at, fp),
            epoch: self.model.epoch(),
            toggled: turn,
            before,
            down: true,
            stage: Stage::None,
            touched: Instant::now(),
        });
        self.select_on_panel = self.orch.active;
    }

    /// The button is down on a selection and the pointer is at `mouse`: its
    /// head follows, and the first cell past the anchor makes it a drag, which
    /// puts back the mark the press toggled. Whether the head moved and
    /// whether that was the drag's first cell; `None` with no such selection.
    fn follow(&mut self, mouse: Mouse) -> Option<(bool, bool)> {
        let held = self.select.as_mut().filter(|held| held.down)?;
        held.touched = Instant::now();
        let first = !held.sel.moved();
        let changed = held.sel.extend(Position::new(mouse.column, mouse.row));
        if first && held.sel.moved() && std::mem::take(&mut held.toggled) {
            self.model.set_turn(held.before);
        }
        Some((changed, first))
    }

    /// Left motion: whether a frame is due.
    pub(super) fn stretch(&mut self, mouse: Mouse) -> bool {
        (self.follow(mouse)).is_some_and(|(changed, first)| changed || first)
    }

    /// The button let go: a selection that never moved was a click; one that
    /// did asks for its copy, which the next frame paints and then ships.
    pub(super) fn finish(&mut self, mouse: Mouse) -> bool {
        let pending = self.read_press.take();
        let moved = self.follow(mouse).is_some_and(|_| {
            let moved = self.select.as_mut().is_some_and(|held| {
                held.down = false;
                held.stage = Stage::Requested;
                held.sel.moved()
            });
            if !moved {
                self.select = None;
            }
            moved
        });
        if !moved
            && let Some((hit, at, epoch)) = pending
            && at == Position::new(mouse.column, mouse.row)
            && epoch == self.model.epoch()
        {
            match hit {
                Press::File(hit) => {
                    if let Some(files) = self.file_list(&hit) {
                        self.read_file(files, hit.link.index, hit.link.base);
                    } else {
                        self.changed_read();
                    }
                }
                Press::Web => {
                    self.flash = Some((link::open_hint().to_owned(), Instant::now()));
                }
            }
            return true;
        }
        moved
    }

    /// A press or Esc ends a selection: whether a highlight went away.
    pub(super) fn unselect(&mut self) -> bool {
        self.select.take().is_some_and(|held| held.sel.moved())
    }
}
