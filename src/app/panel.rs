//! The orchestrator panel: a second chat column on the orchestrator, which is no row of the list. Its state, its keys
//! and its mouse live here; it is drawn and acted on through the chat column the main window uses.

use super::{
    App, Buffer, Drawn, HISTORY, Instant, Key, Lane, Merged, Mouse, MouseKind, ORCH, Position, Rc,
    Size, Timestamp, View, apply, draw, fleet, loader, model, overview,
};

/// The open orchestrator panel: how it stands, which incarnation it opened
/// on, and its own chat column.
pub(super) struct Panel {
    /// It covers the frame instead of standing beside the main column.
    pub(super) full: bool,
    /// It takes the keys; the main column does otherwise.
    focused: bool,
    /// The incarnation it opened on; `None` opened inert and stays so.
    pub(super) pin: Option<String>,
    /// What an inert opening says.
    opened: String,
    /// When it opened: a press made on a panel drawn before is not this one's.
    since: Instant,
    pub(super) column: Column,
}

/// The state a chat column draws and acts from, beyond the model's chat:
/// the main column's lives in the [`App`], the panel's here, and
/// [`App::swap_column`] trades them.
#[derive(Default)]
pub(super) struct Column {
    chat: model::Chat,
    lane: Rc<Lane>,
    merged: Option<Merged>,
    loading: bool,
    overview: overview::Overview,
    pair: Vec<String>,
    read_only: String,
    read_line: String,
    draft_view: View,
    draft: String,
    keyed: Option<(Rc<Lane>, Rc<[u64]>)>,
    pub(super) layout: draw::Layout,
    history: Vec<(Instant, draw::Layout)>,
    lost: Option<Instant>,
    drawn: Option<Drawn>,
}

/// The orchestrator's state beside the main column.
#[derive(Default)]
pub(super) struct Orch {
    /// Whether the last fleet read proved one orchestrator incarnation.
    pub(super) proof: loader::Proof,
    /// The open panel; `None` while it is hidden.
    pub(super) panel: Option<Panel>,
    /// The panel's column stands in the app's fields now ([`App::in_panel`]).
    pub(super) active: bool,
    /// The panel the reader was last told of.
    pub(super) told: Option<String>,
    /// Whether the next proof opens the panel full.
    pub(super) opening: Opening,
    /// When the panel raised the hint row's flash: its own line, which
    /// hiding the panel drops.
    flash: Option<Instant>,
}

impl Orch {
    /// An app homed on the orchestrator shows it in its panel, full.
    pub(super) fn homed(home: Option<&str>) -> Self {
        let opening = if home == Some(ORCH) {
            Opening::Home
        } else {
            Opening::No
        };
        Self {
            opening,
            ..Self::default()
        }
    }
}

/// Whether the next proof of the orchestrator opens its panel full.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) enum Opening {
    #[default]
    No,
    /// The app's home is the orchestrator.
    Home,
    /// A handed-over view of the orchestrator, bound to this uuid.
    Handed(String),
}

/// What the panel says when it opens on `proof` and acts on nothing, `None`
/// when it opens on a proven incarnation.
pub(super) fn inert_line(proof: &loader::Proof) -> Option<String> {
    match proof {
        loader::Proof::Proven(_) => None,
        loader::Proof::Gap(gap) => Some(format!("orchestrator unproven: {gap}")),
        loader::Proof::Absent => Some("no orchestrator running".to_owned()),
    }
}

/// What the open panel's view state does with a browse byte (see
/// [`App::panel_byte`]).
pub(super) enum PanelByte {
    /// The main column takes it, as with no panel.
    Main,
    /// The panel's own column takes it.
    Panel,
    /// It does nothing in the panel.
    Inert,
}

impl App {
    /// The mouse over a panel in the chat column, the seat view or the reader.
    pub(super) fn panel_mouse(&mut self, mouse: Mouse, fresh: bool, reading: bool) -> bool {
        match mouse.kind {
            MouseKind::WheelUp | MouseKind::WheelDown => {
                let up = mouse.kind == MouseKind::WheelUp;
                if self.layout.in_list(mouse) {
                    let start = self.layout.list_start.unwrap_or(0);
                    self.model.wheel_list(up, start, self.layout.list_max)
                } else if self.layout.in_body(mouse) {
                    self.model.wheel_body(up, self.layout.body_max)
                } else if self.layout.in_chat_body(mouse) {
                    if reading {
                        self.model.reader_wheel(up);
                    } else {
                        self.model.peek_wheel(up);
                    }
                    true
                } else {
                    false
                }
            }
            MouseKind::Click => match self.layout.hit(mouse) {
                Some(draw::Hit::Session(name)) if self.model.selected() == Some(name.as_str()) => {
                    if reading {
                        self.close_reader();
                    } else {
                        self.close_peek();
                    }
                    true
                }
                Some(draw::Hit::Session(_) | draw::Hit::Seat(_) | draw::Hit::Stopped) => {
                    self.clicked(mouse, fresh)
                }
                _ if self.layout.grab(mouse).is_some() => self.clicked(mouse, fresh),
                _ => {
                    self.arm(mouse, None, fresh);
                    false
                }
            },
            MouseKind::Drag => self.drag(mouse) || self.stretch(mouse),
            MouseKind::Release => {
                let ended = self.model.let_go();
                self.finish(mouse) || ended
            }
            MouseKind::Menu => false,
        }
    }

    /// [`App::mouse_made`] for a press on the open panel, `None` for any
    /// other: the panel frame on screen when it was made stands in for the
    /// panel's current one, from the panel column's own history, while the
    /// main column's frames resolve the rest as for any press; one made on a
    /// panel frame the bound dropped is not taken. The panel's cells are the
    /// ones the press's own frame drew, whatever mode the panel took since; a
    /// press on a panel that closed or opened again since is not taken.
    pub(super) fn panel_made(&mut self, mouse: Mouse, origin: Instant) -> Option<bool> {
        if mouse.kind != MouseKind::Click || self.orch.active {
            return None;
        }
        let at = Position::new(mouse.column, mouse.row);
        (self.layout_at(origin).panel).filter(|cells| cells.contains(at))?;
        let Some(panel) = self
            .orch
            .panel
            .as_mut()
            .filter(|panel| panel.since <= origin)
        else {
            return Some(self.frame_gone());
        };
        if panel.column.lost.is_some_and(|lost| origin <= lost) {
            return Some(self.frame_gone());
        }
        let seen = (panel.column.history.iter()).position(|(replaced, _)| origin <= *replaced)?;
        let older = std::mem::take(&mut panel.column.history[seen].1);
        let current = std::mem::replace(&mut panel.column.layout, older);
        let acted = self.frame_made(mouse, origin);
        if let Some(panel) = self.orch.panel.as_mut() {
            let older = std::mem::replace(&mut panel.column.layout, current);
            if let Some(entry) = panel.column.history.get_mut(seen) {
                entry.1 = older;
            }
        }
        Some(acted)
    }
}

/// How a copy names its text: `1 line`, `3 lines`.
/// The orchestrator panel: a second chat column on the orchestrator, which is
/// no row of the list. Its column state lives in [`Panel::column`] and stands
/// in the app's own fields only inside [`App::in_panel`], so the chat's
/// renderer, keys and composer serve it unchanged.
impl App {
    /// Trade the main column's state for the panel's, either way.
    fn swap_column(&mut self) {
        let Some(panel) = self.orch.panel.as_mut() else {
            return;
        };
        let column = &mut panel.column;
        self.model.swap_chat(&mut column.chat);
        std::mem::swap(&mut self.lane, &mut column.lane);
        std::mem::swap(&mut self.merged, &mut column.merged);
        std::mem::swap(&mut self.loading, &mut column.loading);
        std::mem::swap(&mut self.overview, &mut column.overview);
        std::mem::swap(&mut self.pair, &mut column.pair);
        std::mem::swap(&mut self.read_only, &mut column.read_only);
        std::mem::swap(&mut self.read_line, &mut column.read_line);
        std::mem::swap(&mut self.draft_view, &mut column.draft_view);
        std::mem::swap(&mut self.draft, &mut column.draft);
        std::mem::swap(&mut self.keyed, &mut column.keyed);
        std::mem::swap(&mut self.layout, &mut column.layout);
        std::mem::swap(&mut self.history, &mut column.history);
        std::mem::swap(&mut self.lost, &mut column.lost);
        std::mem::swap(&mut self.drawn, &mut column.drawn);
        self.orch.active = !self.orch.active;
    }

    /// Run `act` with the panel's column in the app's fields, then put the
    /// main column back and tell the reader what changed. `None` with no
    /// panel open.
    pub(super) fn in_panel<R>(&mut self, act: impl FnOnce(&mut Self) -> R) -> Option<R> {
        self.orch.panel.as_ref()?;
        if self.orch.active {
            return Some(act(self));
        }
        let flash = |app: &Self| app.flash.as_ref().map(|(_, at)| *at);
        let before = flash(self);
        self.swap_column();
        let done = act(self);
        if self.orch.active {
            self.swap_column();
        }
        if flash(self) != before && self.orch.panel.is_some() {
            self.orch.flash = flash(self);
        }
        self.focus();
        Some(done)
    }

    /// Whether the open panel takes the keys.
    pub(super) fn panel_focused(&self) -> bool {
        self.orch.panel.as_ref().is_some_and(|panel| panel.focused)
    }

    /// The session the running entry writes.
    pub(super) fn writer_name(&self) -> Option<&str> {
        self.writer.as_ref().map(|writer| writer.console.name())
    }

    /// Whether the column standing in the app's fields writes: the entry is
    /// its own, not the other column's.
    pub(super) fn writes_here(&self) -> bool {
        self.composing()
            && self
                .writer_name()
                .is_none_or(|name| Some(name) == self.model.selected())
    }

    /// Why the open panel acts on nothing now; `None` while the orchestrator
    /// is the incarnation it opened on. It never takes another.
    pub(super) fn panel_line(&self) -> Option<String> {
        let panel = self.orch.panel.as_ref()?;
        let Some(pin) = &panel.pin else {
            return Some(panel.opened.clone());
        };
        match &self.orch.proof {
            loader::Proof::Proven(uuid) if uuid == pin => None,
            loader::Proof::Proven(_) => Some("orchestrator replaced; O reopens".to_owned()),
            proof => inert_line(proof),
        }
    }

    /// Whether the orchestrator needs the human as a target of `!` and the
    /// button: the incarnation the open panel is pinned to, or while hidden
    /// the one this read proved.
    fn orchestrator_needy(&self) -> bool {
        let proven = match &self.orch.panel {
            Some(_) => self.panel_line().is_none(),
            None => matches!(self.orch.proof, loader::Proof::Proven(_)),
        };
        proven
            && fleet::orchestrator(&self.world, &self.facts, &self.needs, Timestamp::now())
                .is_some_and(|row| row.needy)
    }

    /// How the panel stands, for the model to draw by.
    pub(super) fn columns(&self) -> model::Columns {
        model::Columns {
            needy: self.orchestrator_needy(),
            panel: self
                .orch
                .panel
                .as_ref()
                .map(|panel| (panel.full, panel.focused)),
            drawing_panel: false,
        }
    }

    /// Open the panel with the keys, side by side or `full`, on the
    /// incarnation this read proved: one step, so what it shows and what it
    /// pinned cannot differ. Nothing proven opens it inert.
    pub(super) fn open_panel(&mut self, full: bool) {
        if self.orch.panel.is_some() {
            return;
        }
        if self.composing() || self.held.is_some() {
            self.write(false);
        }
        let pin = match &self.orch.proof {
            loader::Proof::Proven(uuid) => Some(uuid.clone()),
            _ => None,
        };
        self.orch.panel = Some(Panel {
            full,
            focused: true,
            pin,
            opened: inert_line(&self.orch.proof).unwrap_or_default(),
            since: Instant::now(),
            column: Column {
                chat: model::Chat::of(ORCH),
                ..Column::default()
            },
        });
        // `show` sets the column's `loading` on every path.
        self.in_panel(Self::show);
    }

    /// Hide the panel: its entry ends, its draft stays under its input.
    pub(super) fn close_panel(&mut self) {
        if self.orch.active {
            self.swap_column();
        }
        // No entry is HELD here: every caller acts on a press, which ended
        // it, or on a key, which a HELD entry takes for itself.
        if self.writer_name() == Some(ORCH) {
            self.write(false);
        }
        self.orch.panel = None;
        let mine = std::mem::take(&mut self.orch.flash);
        self.flash
            .take_if(|(_, at)| mine.is_some_and(|mine| mine == *at));
        self.focus();
    }

    /// `O` and the button: open the panel beside the main column, or hide
    /// it; an open panel a main column covers takes the keys and shows.
    pub(super) fn toggle_panel(&mut self) {
        match self.panel_area() {
            _ if self.orch.panel.is_none() => self.open_panel(false),
            None => self.focus_column(true),
            Some(_) => self.close_panel(),
        }
    }

    /// The cells the open panel covers in the last frame, `None` while it is
    /// closed or the main column covers it.
    fn panel_area(&self) -> Option<ratatui_core::layout::Rect> {
        let panel = self.orch.panel.as_ref()?;
        let columns = model::Columns {
            panel: Some((panel.full, panel.focused)),
            ..model::Columns::default()
        };
        draw::panel_area(self.layout.area(), self.model.split(), columns)
    }

    /// `F` and the mode label: the panel between side and full as the last
    /// frame showed it; full takes the keys. Whether the panel is open.
    pub(super) fn flip_panel(&mut self) -> bool {
        let shown = self.orch.panel.as_ref().is_some_and(|panel| {
            let columns = model::Columns {
                panel: Some((panel.full, panel.focused)),
                ..model::Columns::default()
            };
            draw::panel_full(self.layout.area(), self.model.split(), columns)
        });
        if self.orch.panel.is_none() {
            return false;
        }
        if !shown {
            self.focus_column(true);
        }
        if let Some(panel) = self.orch.panel.as_mut() {
            panel.full = !shown;
        }
        true
    }

    /// Give the keys to the panel or to the main column: a change ends the
    /// running entry as Esc does, its draft kept under its own input. A full
    /// panel keeps them: the main column it covers never holds the keys, so
    /// it never writes unseen.
    pub(super) fn focus_column(&mut self, on_panel: bool) {
        let Some(panel) = self.orch.panel.as_mut() else {
            return;
        };
        if panel.focused == on_panel || (panel.full && !on_panel) {
            return;
        }
        panel.focused = on_panel;
        if self.composing() || self.held.is_some() {
            self.write(false);
        }
    }

    /// `!`: the next entry that needs the human after the cursor, in one ring
    /// of the orchestrator first and then the rows, wrapping. The cursor is
    /// the orchestrator while the panel holds the keys, else the selection.
    /// The orchestrator opens or focuses the side panel; a row is selected in
    /// the main column, which takes the keys, a full panel turning side.
    pub(super) fn next_need(&mut self) -> bool {
        let count = self.fleet.rows.len() + 1;
        let cursor = if self.panel_focused() {
            Some(0)
        } else {
            self.model.position(&self.fleet).map(|at| at + 1)
        };
        let after = cursor.map_or(0, |at| at + 1);
        let orchestrator = self.orchestrator_needy();
        let target =
            (0..count)
                .map(|step| (after + step) % count)
                .find(|at| match at.checked_sub(1) {
                    None => orchestrator,
                    Some(row) => self.fleet.rows[row].needy,
                });
        match target {
            None => false,
            Some(0) => {
                if self.orch.panel.is_none() {
                    self.open_panel(false);
                }
                self.focus_column(true);
                if let Some(panel) = self.orch.panel.as_mut() {
                    panel.full = false;
                }
                true
            }
            Some(at) => {
                if let Some(panel) = self.orch.panel.as_mut() {
                    panel.full = false;
                }
                self.focus_column(false);
                let act = self.model.select_at(&self.fleet, at - 1);
                let _ = apply(self, &act, Instant::now());
                true
            }
        }
    }

    /// Who takes browse byte `byte` while the panel holds the keys: the
    /// list's own keys stay the main column's beside a side panel and do
    /// nothing over a full one, a seat view or pane does nothing, and the
    /// rest is the panel's chat.
    pub(super) fn panel_byte(&self, byte: u8) -> PanelByte {
        let full = self.orch.panel.as_ref().is_some_and(|panel| {
            let columns = model::Columns {
                panel: Some((panel.full, panel.focused)),
                ..model::Columns::default()
            };
            draw::panel_full(self.layout.area(), self.model.split(), columns)
        });
        match byte {
            b'v' | b'o' => PanelByte::Inert,
            b'1'..=b'9' | b'j' | b'k' if full => PanelByte::Inert,
            b'1'..=b'9' | b'j' | b'k' | b'q' | b',' | b'?' | b's' | b'z' | b'm' => PanelByte::Main,
            _ => PanelByte::Panel,
        }
    }

    /// A key the composer of the column that holds the keys takes.
    pub(super) fn compose_routed(&mut self, key: Key, origin: Instant) -> Option<()> {
        if self.panel_focused() && !self.orch.active {
            return self
                .in_panel(|app| app.compose(key, origin))
                .unwrap_or(Some(()));
        }
        self.compose(key, origin)
    }

    /// The reader a panel turn opened stands in the main column, which takes
    /// the keys; a full panel turns side so the file shows.
    pub(super) fn reader_to_main(&mut self) {
        if let Some(panel) = self.orch.panel.as_mut() {
            panel.full = false;
        }
        self.focus_column(false);
    }

    /// The fold of one proof of the orchestrator: an open panel that lost its
    /// incarnation ends its own entry, never HELD and never the main
    /// column's; a home or handed-over view of it opens the panel full.
    pub(super) fn proved(&mut self, proof: loader::Proof) {
        self.orch.proof = proof;
        let opening = std::mem::take(&mut self.orch.opening);
        let opens = match (&opening, &self.orch.proof) {
            (Opening::Home, _) => true,
            (Opening::Handed(id), loader::Proof::Proven(uuid)) => id == uuid,
            _ => false,
        };
        if opens {
            self.open_panel(true);
        }
        if self.orch.panel.is_some() {
            if self.panel_line().is_some() && self.writer_name() == Some(ORCH) {
                self.write(false);
            }
            self.in_panel(Self::show);
        }
    }

    /// Whether a mouse event lands on the panel in the frame it is resolved
    /// against: the one on screen, or the one a delayed press was made on.
    pub(super) fn on_panel(&self, mouse: Mouse) -> bool {
        (self.layout.panel)
            .is_some_and(|cells| cells.contains(Position::new(mouse.column, mouse.row)))
    }

    /// A mouse event on the open panel: its button, mode label and gear act
    /// on the app; a press gives it the keys, and the rest is its chat's.
    pub(super) fn panel_route(&mut self, mouse: Mouse, fresh: bool) -> bool {
        // Any press ends a HELD entry, as in the main column's `click`.
        if mouse.kind == MouseKind::Click {
            self.held = None;
        }
        let hit = (self.orch.panel.as_ref())
            .filter(|_| mouse.kind == MouseKind::Click)
            .and_then(|panel| panel.column.layout.hit(mouse));
        match hit {
            Some(draw::Hit::Orchestrator) => {
                self.toggle_panel();
                return true;
            }
            Some(draw::Hit::PanelMode) => return self.flip_panel(),
            Some(draw::Hit::Settings) => {
                self.open_settings();
                return true;
            }
            _ => {}
        }
        if mouse.kind == MouseKind::Click {
            self.focus_column(true);
        }
        self.in_panel(|app| app.route(mouse, fresh))
            .unwrap_or(false)
            || mouse.kind == MouseKind::Click
    }

    /// Draw the open panel over its cells of `buf`, from its own column; a
    /// panel the main column covers draws nothing and sees no turn.
    pub(super) fn frame_panel(&mut self, buf: &mut Buffer) {
        let mut columns = self.columns();
        if draw::panel_area(buf.area, self.model.split(), columns).is_none() {
            return;
        }
        columns.drawing_panel = true;
        let main = self.model.columns();
        let cursor = self.in_panel(|app| {
            app.show();
            app.model.set_columns(columns);
            let area = buf.area;
            let size = Size {
                width: draw::column_width(area, app.model.split(), columns),
                height: draw::composer_pane(area),
            };
            let shown = app.shown_input().map(|i| (i.bare_view(size), i.draft()));
            (app.draft_view, app.draft) = shown.unwrap_or_default();
            let keys = app.keys();
            let (folds, open, sent) = app.fold_state();
            let entry = app.entry(ORCH);
            let (look, zone) = app.dressed();
            let screen = draw::Screen {
                fleet: &app.fleet,
                model: &app.model,
                overview: &app.overview,
                selected: entry,
                pair: &app.pair,
                agents: app.facts.get(ORCH),
                lane: &app.lane,
                composer: app.composer(),
                look,
                zone,
                now: Timestamp::now(),
            };
            let dest = app.destination(ORCH);
            let wait = draw::Wait {
                fleet: !app.fleeted,
                lane: app.loading,
                keys: &keys,
                folds: &folds,
                open: &open,
                sent: &sent,
                dest: dest.as_ref().map(draw::Dest::of),
                md: Some(&app.md),
            };
            let drawn = draw::draw_panel(&screen, wait, buf);
            let replaced = std::mem::replace(&mut app.layout, drawn);
            app.history.push((Instant::now(), replaced));
            if app.history.len() > HISTORY {
                app.lost = Some(app.history.remove(0).0);
            }
            if app.deferred.is_empty() {
                app.model
                    .clamp_scroll(app.layout.max_scroll, app.layout.page_rows);
            }
            app.drawn = Some(app.freeze(ORCH));
            app.model.set_columns(main);
            (app.layout.cursor, app.layout.panel)
        });
        let (cursor, cells) = cursor.unwrap_or_default();
        if let Some(cursor) = cursor {
            self.layout.cursor = Some(cursor);
        }
        self.layout.panel = cells;
    }
}
