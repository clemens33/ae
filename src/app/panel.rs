//! The orchestrator panel: a second chat column on the orchestrator, which is no row of the list. Its state, its keys
//! and its mouse live here; it is drawn and acted on through the chat column the main window uses.

use super::{
    App, Arm, Armed, Buffer, Drawn, HISTORY, Instant, Key, Lane, Merged, Mouse, MouseKind, ORCH,
    Position, QUIT_WINDOW, Rc, Refusal, SeatRef, Size, Timestamp, View, apply, draw, fleet, loader,
    model, overview,
};

/// The open orchestrator panel: how it stands, what it is bound to, and its
/// own chat column.
pub(super) struct Panel {
    /// It covers the main column instead of standing beside it; the sidebar
    /// stays drawn.
    pub(super) full: bool,
    /// It takes the keys; the main column does otherwise.
    focused: bool,
    /// The incarnation it acts on, or why it acts on none.
    bind: Bind,
    /// When it opened: a press made on a panel drawn before is not this one's.
    since: Instant,
    pub(super) column: Column,
}

impl Panel {
    /// The incarnation the panel is bound to; `None` while it acts on none.
    pub(super) fn pin(&self) -> Option<&str> {
        match &self.bind {
            Bind::Live(uuid) => Some(uuid),
            Bind::Awaiting { .. } | Bind::Line(_) => None,
        }
    }
}

/// What the open panel acts on. The cause of its opening decides whether a
/// later proof may bind it: only an opening that asked for the seat waits.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Bind {
    /// The incarnation it opened on or waited for: it never takes another.
    Live(String),
    /// It opened with no orchestrator running and asked for the seat under
    /// `generation`: the first proof of the seat binds it. `line` is what the
    /// ask said last, `None` before it answered or when it ran.
    Awaiting {
        line: Option<String>,
        generation: u64,
    },
    /// It acts on nothing until it is opened again, and says why.
    Line(String),
}

/// What an awaiting panel says before its ask tells more.
const AWAITING: &str = "waiting for the orchestrator seat";

/// What a bound panel says once its seat no longer runs.
const LOST: &str = "orchestrator stopped; O starts it";

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
    /// The generation of the last seat ask an opening made.
    asked: u64,
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

/// The pane target a panel frame drew, as the frozen frame the seat keys act
/// on: the orchestrator session, the incarnation and its one main seat.
fn pane_drawn((uuid, seat): (String, Option<SeatRef>)) -> Drawn {
    Drawn {
        session: ORCH.to_owned(),
        uuid,
        focused: seat.as_ref().map(|seat| seat.name.clone()),
        rows: seat
            .map(|seat| (seat.name.clone(), Some(seat)))
            .into_iter()
            .collect(),
    }
}

/// Why a running orchestrator is no incarnation the panel can bind.
fn unproven(gap: &str) -> String {
    format!("orchestrator unproven: {gap}")
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
    /// `o`: the panel's own pane key ([`App::pane_key`]).
    Open,
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
                Some(
                    draw::Hit::Session(_)
                    | draw::Hit::Seat(_)
                    | draw::Hit::Stopped
                    | draw::Hit::Spawned,
                ) => self.clicked(mouse, fresh),
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

    /// Whether the input `key` is the panel's own: the seat incarnation the
    /// open panel is pinned to. A listed session of the seat's name is an
    /// ordinary session, whatever its name.
    pub(super) fn panel_entry(&self, key: &(String, String)) -> bool {
        key.0 == ORCH && self.orch.panel.as_ref().and_then(Panel::pin) == Some(key.1.as_str())
    }

    /// Whether the running entry is the panel's own ([`App::panel_entry`]).
    fn writes_panel(&self) -> bool {
        (self.writer.as_ref()).is_some_and(|writer| self.panel_entry(&writer.key()))
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
    /// is the incarnation it is bound to. It never takes another. A fleet read
    /// that shows the seat stopped is a loss already: the loader sends that
    /// read and its proof as two wakes, and a frame between them never draws
    /// the stopped seat.
    pub(super) fn panel_line(&self) -> Option<String> {
        let panel = self.orch.panel.as_ref()?;
        let pin = match &panel.bind {
            Bind::Live(pin) => pin,
            Bind::Awaiting { line, .. } => {
                return Some(line.clone().unwrap_or_else(|| AWAITING.to_owned()));
            }
            Bind::Line(line) => return Some(line.clone()),
        };
        let stopped =
            (self.entry(ORCH)).is_some_and(|seat| seat.status == crate::digest::Status::Stopped);
        match &self.orch.proof {
            loader::Proof::Proven(uuid) if uuid == pin && stopped => Some(LOST.to_owned()),
            loader::Proof::Proven(uuid) if uuid == pin => None,
            loader::Proof::Proven(_) => Some("orchestrator replaced; O reopens".to_owned()),
            loader::Proof::Gap(gap) => Some(unproven(gap)),
            loader::Proof::Absent => Some(LOST.to_owned()),
        }
    }

    /// End the panel's entry once the panel acts on nothing: no HELD reason,
    /// and its draft stays under its input. The fleet read that stops the
    /// seat ([`App::absorb`]) and the proof after it ([`App::proved`]) both
    /// call it, so no frame between the two keeps a writable composer.
    pub(super) fn end_inert_panel_entry(&mut self) {
        if self.panel_line().is_some() && self.writes_panel() {
            self.write(false);
        }
    }

    /// What the seat ask of `generation` came to: an awaiting panel of that
    /// generation shows its line, and a refusal ends the wait. Anything else
    /// is an older opening's and changes nothing.
    pub(super) fn seated(&mut self, generation: u64, said: loader::Said) {
        let Some(panel) = self.orch.panel.as_mut() else {
            return;
        };
        let Bind::Awaiting {
            generation: asked, ..
        } = panel.bind
        else {
            return;
        };
        if asked != generation {
            return;
        }
        panel.bind = match said {
            loader::Said::Waiting(line) => Bind::Awaiting { line, generation },
            loader::Said::Refused(line) => Bind::Line(line),
        };
        self.in_panel(Self::show);
    }

    /// Whether the orchestrator needs the human as a target of `!` and the
    /// button: the incarnation the open panel is pinned to, or while hidden
    /// the one this read proved.
    fn orchestrator_needy(&self) -> bool {
        let proven = match &self.orch.panel {
            Some(_) => self.panel_line().is_none(),
            None => matches!(self.orch.proof, loader::Proof::Proven(_)),
        };
        // A proof proves the role holder named `orchestrator`.
        proven
            && fleet::orchestrator(
                &self.world,
                Some(ORCH),
                &self.facts,
                &self.needs,
                Timestamp::now(),
            )
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
    /// pinned cannot differ. With no orchestrator running it asks the reader
    /// for the seat and waits for its proof; an unproven one opens it inert.
    pub(super) fn open_panel(&mut self, full: bool) {
        if self.orch.panel.is_some() {
            return;
        }
        if self.composing() || self.held.is_some() {
            self.write(false);
        }
        let bind = match &self.orch.proof {
            loader::Proof::Proven(uuid) => Bind::Live(uuid.clone()),
            loader::Proof::Gap(gap) => Bind::Line(unproven(gap)),
            loader::Proof::Absent => {
                self.orch.asked += 1;
                let generation = self.orch.asked;
                if let Some(ask) = &self.ask {
                    let _ = ask.send(loader::Request::Seat(generation));
                }
                Bind::Awaiting {
                    line: None,
                    generation,
                }
            }
        };
        self.orch.panel = Some(Panel {
            full,
            focused: true,
            bind,
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
        if self.writes_panel() {
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
    pub(super) fn panel_area(&self) -> Option<ratatui_core::layout::Rect> {
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
    /// nothing over a full one, `v` (the seat view) does nothing, `o` is the
    /// panel's pane key, and the rest is the panel's chat.
    pub(super) fn panel_byte(&self, byte: u8) -> PanelByte {
        let full = self.orch.panel.as_ref().is_some_and(|panel| {
            let columns = model::Columns {
                panel: Some((panel.full, panel.focused)),
                ..model::Columns::default()
            };
            draw::panel_full(self.layout.area(), self.model.split(), columns)
        });
        match byte {
            b'v' => PanelByte::Inert,
            b'o' => PanelByte::Open,
            b'1'..=b'9' | b'j' | b'k' if full => PanelByte::Inert,
            b'1'..=b'9' | b'j' | b'k' | b'q' | b',' | b'?' | b's' | b'z' | b'Z' | b'm' => {
                PanelByte::Main
            }
            _ => PanelByte::Panel,
        }
    }

    /// What the panel's pane target opens: the incarnation the panel is bound
    /// to and, when the roster read of that same incarnation holds one, its
    /// main seat. `None` while the panel is bound to none.
    fn pane_target(&self) -> Option<(String, Option<SeatRef>)> {
        let pin = self.orch.panel.as_ref().and_then(Panel::pin)?;
        let seat = (self.shown.get(ORCH))
            .filter(|shown| shown.id == pin)
            .and_then(|shown| shown.roster.as_deref())
            .and_then(|roster| (roster.iter()).find(|seat| seat.slot == crate::watchdog::MAIN_SLOT))
            .cloned();
        Some((pin.to_owned(), seat))
    }

    /// `o` while the panel holds the keys: the first press arms on the pane
    /// target the last panel frame drew and moves nothing, the second within
    /// the window opens THAT target, whatever the panel drew since. A panel
    /// bound to no running orchestrator refuses at once and arms nothing,
    /// and so does a second press that finds it so ([`App::pane_refusal`]):
    /// the key never starts or resumes the seat. Whether the frame is due.
    pub(super) fn pane_key(&mut self, origin: Instant) -> bool {
        self.in_panel(|app| app.pane_press(origin)).is_some()
    }

    /// [`App::pane_key`] with the panel's column in the app's fields.
    fn pane_press(&mut self, origin: Instant) {
        let armed = (self.armed.take())
            .filter(|armed| origin.saturating_duration_since(armed.at) <= QUIT_WINDOW);
        if let Some(Armed {
            what: Arm::Pane(drawn),
            ..
        }) = armed
        {
            return self.pane_open(&drawn);
        }
        if let Some(why) = self.pane_refusal() {
            return self.pane_said(why.line(ORCH));
        }
        let drawn = pane_drawn(self.layout.pane.clone().unwrap_or_default());
        match drawn.seat() {
            Ok(_) => {
                self.armed = Some(Armed {
                    at: origin,
                    what: Arm::Pane(drawn),
                });
            }
            Err(why) => self.pane_said(why.line(ORCH)),
        }
    }

    /// A press on the panel header's `pane` word: it opens at once what the
    /// frame that drew the word read, as the second `o` does.
    pub(super) fn pane_click(&mut self) -> bool {
        let target = (self.orch.panel.as_ref()).and_then(|panel| panel.column.layout.pane.clone());
        let drawn = pane_drawn(target.unwrap_or_default());
        self.in_panel(|app| app.pane_open(&drawn)).is_some()
    }

    /// Why the panel cannot open a pane now, or `None` while it is bound to
    /// the running orchestrator. The seat proof knows seats, not the panel's
    /// standing: a role census that failed leaves the pinned incarnation's
    /// seat facts valid. A known replaced or stopped incarnation takes the
    /// seat-open refusals' words; any other cause, its own line.
    fn pane_refusal(&self) -> Option<Refusal> {
        let why = self.panel_line()?;
        let Some(pin) = self.orch.panel.as_ref().and_then(Panel::pin) else {
            return Some(Refusal::NoSeat(why));
        };
        Some(match &self.orch.proof {
            loader::Proof::Gap(_) => Refusal::NoSeat(why),
            loader::Proof::Proven(uuid) if uuid != pin => Refusal::SessionReplaced,
            loader::Proof::Proven(_) | loader::Proof::Absent => Refusal::Stopped(ORCH.to_owned()),
        })
    }

    /// Open the orchestrator's pane as the chat's `/open` does, on the
    /// incarnation and seat `drawn` holds: the proof is made now, against that
    /// incarnation, never a fresh lookup by name, and only for a panel still
    /// bound to the running orchestrator ([`App::pane_refusal`]).
    fn pane_open(&mut self, drawn: &Drawn) {
        let label = drawn.focused.as_deref().unwrap_or(ORCH);
        let line = match self.pane_refusal() {
            Some(why) => Some(why.line(label)),
            None => self.open(ORCH, &drawn.uuid, drawn.seat(), label),
        };
        match line {
            Some(line) => self.pane_said(line),
            None => self.show(),
        }
    }

    /// Tell the outcome of a pane press: in the panel's lane while the panel
    /// is a chat, on the hint row while it is a status column, whose body
    /// draws no lane.
    fn pane_said(&mut self, line: String) {
        if self.panel_line().is_some() {
            self.flash = Some((line, Instant::now()));
        } else {
            self.notice(ORCH, line);
        }
        self.show();
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

    /// The fold of one proof of the orchestrator: an awaiting panel binds the
    /// first proven incarnation; an open panel that lost its incarnation ends
    /// its own entry, never HELD and never the main column's; a home or
    /// handed-over view of a running orchestrator opens the panel full.
    pub(super) fn proved(&mut self, proof: loader::Proof) {
        self.orch.proof = proof;
        if let Some(panel) = self.orch.panel.as_mut()
            && matches!(panel.bind, Bind::Awaiting { .. })
            && let loader::Proof::Proven(uuid) = &self.orch.proof
        {
            panel.bind = Bind::Live(uuid.clone());
        }
        let opening = std::mem::take(&mut self.orch.opening);
        // A home that runs no seat opens nothing: O asks for it.
        let opens = match (&opening, &self.orch.proof) {
            (Opening::Home, proof) => *proof != loader::Proof::Absent,
            (Opening::Handed(id), loader::Proof::Proven(uuid)) => id == uuid,
            _ => false,
        };
        if opens {
            self.open_panel(true);
        }
        if self.orch.panel.is_some() {
            self.end_inert_panel_entry();
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
            Some(draw::Hit::PanelPane) => return self.pane_click(),
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
        // The panel wears the look and the clock the main column wears: the
        // orchestrator is no listed session, so its own column reads none.
        let (look, zone) = self.dressed();
        let zone = zone.map(str::to_owned);
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
            // A panel bound to no running seat is a status column: it draws
            // no session, so never a stopped one.
            let bound = app.panel_line().is_none();
            let entry = app.entry(ORCH).filter(|_| bound);
            let screen = draw::Screen {
                fleet: &app.fleet,
                model: &app.model,
                overview: &app.overview,
                selected: entry,
                pair: &app.pair,
                agents: app.facts.get(ORCH).filter(|_| bound),
                lane: &app.lane,
                composer: app.composer(),
                look,
                zone: zone.as_deref(),
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
            // Its `pane` word opens what this frame read, whatever a later
            // read changes.
            app.layout.pane = bound.then(|| app.pane_target()).flatten();
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
        self.layout.panel_border(buf, self.model.split(), columns);
    }
}
