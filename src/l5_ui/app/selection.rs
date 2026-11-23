//! The selection: which session, seat or item is selected and opened, the stopped fold, the parked panes and the handover view.

use super::fleet::Facts;
use super::panel::{Opening, Panel};
use super::{
    Answer, App, Arm, Buffer, Drawn, Instant, Keep, Key, LANE_HOLD, Move, ORCH, QUIT_WINDOW, Rc,
    Reading, Refusal, Request, SeatRef, Shown, Stage, Status, Timestamp, handover, loader, model,
    open, paneview, proven_pane, select, take_keys, theme, tmux, toggle, transport,
};

impl App {
    /// Fold the stopped sessions of the fleet just read into the sidebar's one
    /// group: out of the rows while it is closed, in them while it is open. A
    /// read opens it when its own selection, already read once, turned stopped,
    /// and when it takes a handed-over view of a stopped session: a session the
    /// human is on never goes missing from under them.
    pub(super) fn fold_stopped(&mut self, seen: bool, scanned: bool) {
        let stopped =
            |name: &str| (self.fleet.rows.iter()).any(|row| row.name == name && row.is_stopped());
        let selected = seen && self.model.selected().is_some_and(stopped);
        let handed = self.keep.as_ref().is_some_and(|keep| {
            let name = keep.view.session.as_str();
            let id = self.ids.get(name).map(String::as_str);
            let now = Timestamp::now().epoch();
            stopped(name) && self.keeps_view(keep, true, id, scanned, now)
        });
        self.stopped_open |= selected || handed;
        self.hidden.clear();
        if !self.stopped_open {
            let (fleet, hidden) = std::mem::take(&mut self.fleet).without_stopped();
            (self.fleet, self.hidden) = (fleet, hidden);
        }
    }

    /// The stopped group the sidebar draws: every stopped session of the
    /// fleet, in the rows or held back, whether it is open and how many it holds
    /// back that need the human.
    pub(super) fn stopped(&self) -> model::Stopped {
        let shown = self
            .fleet
            .rows
            .iter()
            .filter(|row| row.is_stopped())
            .count();
        model::Stopped {
            total: shown + self.hidden.len(),
            open: self.stopped_open,
            hidden_needy: self.hidden.iter().filter(|row| row.needy).count(),
        }
    }

    /// `z`, or a press on the group's line: open the stopped sessions into the
    /// list, or close them out of it. Opening moves no selection. Closing with
    /// the selection among them hands it to home when the list holds it, else to
    /// the first row, else to nothing, and ends the writing and the panels of the
    /// session it leaves. Whether a frame is due.
    pub(super) fn toggle_stopped(&mut self) -> bool {
        if self.stopped().total == 0 {
            return false;
        }
        self.stopped_open = !self.stopped_open;
        if self.stopped_open {
            let hidden = std::mem::take(&mut self.hidden);
            let home = self.home.as_deref();
            self.fleet = std::mem::take(&mut self.fleet).with_stopped(hidden, home);
            return true;
        }
        let (fleet, hidden) = std::mem::take(&mut self.fleet).without_stopped();
        (self.fleet, self.hidden) = (fleet, hidden);
        let before = self.model.selected().map(str::to_owned);
        self.model.reconcile(&self.fleet);
        if before.as_deref() != self.model.selected() {
            self.write(false);
            if self.model.peek_open() {
                self.close_peek();
            }
            if self.model.reader_open() {
                self.close_reader();
            }
            self.show();
        }
        true
    }

    /// The selected running session's spawned seats the Agents body lists:
    /// the facts' seats whose roster slot, by its own read, is `spawned.<n>`.
    /// The slot decides, never the seat's name; a seat the roster does not
    /// hold, a session whose roster is not read yet and a roster read from
    /// another incarnation than the one the fleet names now (a home lane is
    /// kept until its next read) have none.
    pub(super) fn spawned_names(&self) -> Vec<String> {
        let Some(name) = self.model.selected() else {
            return Vec::new();
        };
        let stopped = (self.entry(name)).is_some_and(|entry| entry.status == Status::Stopped);
        let current = self.ids.get(name);
        let roster = (self.shown.get(name))
            .filter(|shown| current == Some(&shown.id))
            .and_then(|shown| shown.roster.as_deref());
        let (Some(Facts::Seats { agents, .. }), Some(roster)) = (self.facts.get(name), roster)
        else {
            return Vec::new();
        };
        let spawned = |seat: &str| {
            roster.iter().any(|held| {
                held.name == seat && crate::seatpack::slot_class(&held.slot) == "spawned"
            })
        };
        agents
            .iter()
            .filter(|agent| !stopped && spawned(&agent.name))
            .map(|agent| agent.name.clone())
            .collect()
    }

    /// Hand the frame about to be drawn the spawned group it draws: the
    /// selection's spawned seats and the window's one fold flag.
    pub(super) fn sync_spawned(&mut self) {
        let names = self.spawned_names();
        let folded = self.spawned_folded;
        self.model.set_spawned(model::Spawned { names, folded });
    }

    /// `Z`, or a press on the spawned group's line: fold the selected
    /// session's spawned seats into the line, or draw them again. It acts on
    /// the seats the facts list, drawn or not, and does nothing (the flag
    /// stays) for a session with none. Folding takes the hidden seats out of
    /// reach at once: a cursor on one clears, a peek of one closes back to the
    /// chat, and the frozen frame loses their rows until the next frame is
    /// drawn. Whether a frame is due.
    pub(super) fn toggle_spawned(&mut self) -> bool {
        let names = self.spawned_names();
        if names.is_empty() {
            return false;
        }
        self.spawned_folded = !self.spawned_folded;
        if !self.spawned_folded {
            return true;
        }
        let hidden = |seat: &str| names.iter().any(|name| name == seat);
        if self.model.focus().is_some_and(hidden) {
            self.model.set_focus(None);
        }
        if let Some(drawn) = self.drawn.as_mut() {
            drawn.rows.retain(|(seat, _)| !hidden(seat));
            if drawn.focused.as_deref().is_some_and(hidden) {
                drawn.focused = drawn.rows.first().map(|(seat, _)| seat.clone());
            }
        }
        let viewed = (self.peek.target.as_ref()).is_some_and(|(session, seat)| {
            self.model.selected() == Some(session.as_str()) && hidden(seat)
        });
        if self.model.peek_open() && viewed {
            self.close_peek();
        }
        true
    }

    /// Settle the handed-over view with this read: applied when it lists the
    /// session as the incarnation the view was written for, dropped when it is
    /// stale, proves the session gone or replaced, and kept for a later read
    /// when an incomplete one cannot say.
    pub(super) fn settle_keep(&mut self, scanned: bool) {
        let Some(keep) = self.keep.take() else {
            return;
        };
        let name = keep.view.session.as_str();
        let row = self.fleet.rows.iter().any(|row| row.name == name);
        let id = self.ids.get(name).map(String::as_str);
        let now = Timestamp::now().epoch();
        match self.verdict_on(&keep, row, id, scanned, now) {
            handover::Verdict::Apply if keep.saved => {
                // The layout and the panel were taken at the answer. The
                // selection comes along only for an app that named no home
                // of its own, or whose home is the saved session.
                if self
                    .home
                    .as_deref()
                    .is_none_or(|home| home == keep.view.session)
                {
                    self.model.adopt(&self.fleet, &keep.view);
                    self.restored = true;
                }
            }
            handover::Verdict::Apply => {
                self.model.adopt(&self.fleet, &keep.view);
                self.restored = true;
                if let Some(panel) = keep.view.orchestrator {
                    let homed = self.orch.opening == Opening::Home;
                    self.orch.opening = Opening::Restore(panel, keep.at, homed);
                }
            }
            handover::Verdict::Defer => self.keep = Some(keep),
            handover::Verdict::Discard => {}
        }
    }

    /// What a fleet read decides for a kept view: a handed one by
    /// [`handover::decide`], with its age; one from the durable file by
    /// [`handover::decide_saved`], whatever its age.
    fn verdict_on(
        &self,
        keep: &Keep,
        row: bool,
        id: Option<&str>,
        scanned: bool,
        now: i64,
    ) -> handover::Verdict {
        if keep.saved {
            handover::decide_saved(&keep.view, row, id, scanned)
        } else {
            handover::decide(keep.at, now, &keep.view, row, id, scanned)
        }
    }

    /// Whether the kept view would be applied for the listed `row` now.
    fn keeps_view(
        &self,
        keep: &Keep,
        row: bool,
        id: Option<&str>,
        scanned: bool,
        now: i64,
    ) -> bool {
        self.verdict_on(keep, row, id, scanned, now) == handover::Verdict::Apply
            && (!keep.saved
                || self
                    .home
                    .as_deref()
                    .is_none_or(|home| home == keep.view.session))
    }

    /// Take in the durable view the loader read at start. A handed view that
    /// is already kept, or any input, wins: the file is never read over
    /// either. The layout applies at once, before any fleet read; the panel
    /// follows the first proof of the orchestrator, and the selection the
    /// fleet read that settles the keep.
    pub(super) fn saved_in(&mut self, view: handover::View) {
        if self.touched || self.keep.is_some() {
            return;
        }
        self.model.adopt_layout(&view);
        let homed = self.orch.opening == Opening::Home;
        self.orch.opening = Opening::Saved(view.orchestrator.clone(), homed);
        if !view.unselected() {
            let at = Timestamp::now().epoch();
            self.keep = Some(Keep {
                view,
                at,
                saved: true,
            });
        }
    }

    /// The view the durable file keeps, as the app stands: only once a fleet
    /// read named the sessions' uuids, so a start that has read nothing never
    /// writes a half-built view over the file.
    pub(super) fn saved_view(&self) -> Option<handover::View> {
        if !self.fleeted {
            return None;
        }
        let id = (self.model.selected()).and_then(|name| self.ids.get(name).map(String::as_str));
        let mut view = self.model.saved(id);
        view.orchestrator = self.orch.panel.as_ref().and_then(Panel::handover);
        Some(view)
    }

    /// What the settled view stands at now, for [`App::viewed_by_the_human`].
    pub(super) fn settled_view(&self) -> Option<handover::View> {
        self.saved_view().map(|view| view.settled())
    }

    /// Take `keyed` as the human's action and judge what it changed in the view,
    /// around this one call: an answer, a proof or a fleet read taken in the
    /// same turn is never the human's change. Every dispatch of the human's keys
    /// goes through here: the loop's three intakes and the keys a first fleet
    /// read replays.
    pub(super) fn human(&mut self, keyed: Vec<(Key, Instant)>) -> Option<bool> {
        if keyed.is_empty() {
            return take_keys(self, keyed);
        }
        let before = self.settled_view();
        let taken = take_keys(self, keyed);
        self.viewed_by_the_human(before);
        taken
    }

    /// Judge one action of the human, around the keys it dispatched: when the
    /// settled view differs from `before`, taken just ahead of them, the human
    /// changed the view, and the next debounced write and the final one are
    /// due. A change an answer made (a fleet read, a proof, a vanished session)
    /// never comes through here, even in a turn that also read a key.
    pub(super) fn viewed_by_the_human(&mut self, before: Option<handover::View>) {
        if before.is_some() && self.settled_view() != before {
            self.view_dirty = true;
            self.view_owed = true;
        }
    }

    /// The text of the durable file for the view as it stands, `None` before a
    /// fleet read.
    fn saved_text(&self) -> Option<String> {
        let view = self.saved_view()?;
        let held =
            handover::Handover::durable(crate::shape::VERSION, Timestamp::now().epoch(), view);
        Some(handover::render_file(&held))
    }

    /// Hand the changed view to the writer when its debounce allows.
    pub(super) fn persist_view(&mut self, now: Instant) {
        if !self.view_dirty || !self.saver.as_ref().is_some_and(|saver| saver.ready(now)) {
            return;
        }
        let Some(text) = self.saved_text() else {
            return;
        };
        if self
            .saver
            .as_mut()
            .is_some_and(|saver| saver.submit(text, now))
        {
            self.view_dirty = false;
        }
    }

    /// The last word on the way out: when the human changed the view in this
    /// run, write it now and wait for the write, however long it takes, behind
    /// any debounced write still running; a run that changed nothing leaves the
    /// file alone. A failure is returned once.
    ///
    /// # Errors
    ///
    /// Why the write failed.
    pub(super) fn persist_final_view(&mut self) -> Result<(), String> {
        if !self.view_owed {
            return Ok(());
        }
        let (Some(text), Some(saver)) = (self.saved_text(), self.saver.as_mut()) else {
            return Ok(());
        };
        (self.view_dirty, self.view_owed) = (false, false);
        saver.finish(text)
    }

    /// What this app hands to the one that replaces it in `pane`, at `now`:
    /// the view of its selection and panel, each bound to its recorded uuid.
    pub(super) fn handover(
        &self,
        pane: &str,
        version: &str,
        now: i64,
    ) -> Option<handover::Handover> {
        let name = self.model.selected()?;
        let mut view = self.model.view(self.ids.get(name)?)?;
        view.orchestrator = self.orch.panel.as_ref().and_then(Panel::handover);
        Some(handover::Handover {
            pane: pane.to_owned(),
            version: version.to_owned(),
            at: now,
            view,
        })
    }

    /// Publish [`App::handover`] on the home session, best effort: a restart
    /// never waits on it and never fails for it.
    pub(super) fn publish_view(&self, restart: &toggle::Restart, version: &str) {
        let Some(home) = self.home.as_deref() else {
            return;
        };
        let now = Timestamp::now().epoch();
        let Some(text) = self.handover(&restart.pane, version, now) else {
            return;
        };
        if let Some(id) = transport::observe_session_id(&restart.server, home) {
            let scope = tmux::OptionScope::Session;
            let _ = transport::publish_option(
                &restart.server,
                scope,
                &id,
                handover::OPTION,
                &text.render(),
            );
        }
    }

    /// One read of a session's lane. A lane read as an identity its session
    /// no longer records, or older than the one shown, is dropped; home's
    /// lane stays until its next read.
    pub(super) fn viewed(&mut self, view: loader::ViewRead) {
        let home = self.home.as_deref() == Some(view.name.as_str());
        // The seat's lane is read for its panel: open on that very
        // incarnation, else dropped. A listed namesake is an ordinary row.
        let pinned =
            (self.orch.panel.as_ref()).is_some_and(|panel| panel.pin() == Some(view.id.as_str()));
        let panel = view.name == ORCH && pinned;
        let listed = self.fleet.rows.iter().any(|row| row.name == view.name);
        if view.name == ORCH && !pinned && !home && !listed {
            return;
        }
        if !home && self.ids.get(&view.name) != Some(&view.id) {
            return;
        }
        if self
            .shown
            .get(&view.name)
            .is_some_and(|shown| shown.seq >= view.seq)
        {
            return;
        }
        if let Some(section) = view.needs {
            self.needs.insert(view.name.clone(), section);
        }
        // The board rides the panel's read: kept for that incarnation only.
        if panel && let Some(desk) = view.desk {
            self.read_desk(view.id.clone(), desk);
        }
        // The seats `/open` names, as the chat's own tick hands them over.
        let key = (view.name.clone(), view.id.clone());
        let roster = view.roster.clone();
        if let (Some(seats), Some(input)) = (view.roster, self.inputs.get_mut(&key)) {
            input.set_seats(seats);
        }
        let selected = self.model.selected() == Some(view.name.as_str());
        let shown = Shown {
            id: view.id,
            seq: view.seq,
            lane: Rc::new(view.lane),
            roster,
        };
        self.shown.insert(view.name, shown);
        if selected {
            self.show();
        }
        if panel {
            self.in_panel(Self::show);
        }
    }

    /// Keep only the lanes of the sessions the reader keeps read.
    pub(super) fn evict(&mut self) {
        let rows: Vec<String> = self.fleet.rows.iter().map(|row| row.name.clone()).collect();
        let keep = loader::order(&rows, self.model.selected(), self.home.as_deref());
        let open = self.orch.panel.is_some();
        if !open {
            self.orch.desk = None;
        }
        self.shown
            .retain(|name, _| open && name == ORCH || keep.iter().any(|(kept, _)| kept == name));
    }

    /// Tell the reader which session shows now, once per change.
    pub(super) fn focus(&mut self) {
        if self.orch.active {
            return;
        }
        let panel = self.orch.panel.as_ref().map(|_| ORCH.to_owned());
        if panel != self.orch.told {
            if let Some(ask) = &self.ask {
                let _ = ask.send(Request::Panel(panel.clone()));
            }
            self.orch.told = panel;
            self.evict();
        }
        let selected = self.model.selected().map(str::to_owned);
        self.focus_composer(selected.as_deref());
        if selected == self.focused {
            return;
        }
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Focus(selected.clone()));
        }
        self.focused = selected;
        self.evict();
    }

    fn focus_composer(&mut self, selected: Option<&str>) {
        let next = if self.panel_focused() {
            self.panel_line()
                .is_none()
                .then(|| super::Target::Orchestrator(ORCH.to_owned()))
        } else {
            selected.map(|name| self.target(name))
        };
        let Some(next) = next else {
            return;
        };
        if self.composer_target.as_ref() == Some(&next) {
            return;
        }
        let switched = self.composer_target.replace(next.clone()).is_some();
        let name = if self.panel_focused() {
            Some(ORCH)
        } else {
            selected
        };
        if switched && let Some(name) = name {
            self.notice(
                name,
                format!(
                    "{}{label}",
                    super::TARGET_NOTICE,
                    label = super::target::label(&next)
                ),
            );
            self.show();
        }
    }

    /// Ask the reader to read the written session again: this app wrote into it.
    pub(super) fn reread(&self) {
        if let Some(writer) = &self.writer {
            self.reread_target(writer.console.name());
        }
    }

    pub(super) fn reread_target(&self, name: &str) {
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Reread(name.to_owned()));
        }
    }

    /// The look the chat reads (F3): the home session's drawn look, else the
    /// selected one's, else the default drawn look. A look not read yet
    /// paints no colour rather than a guess a `theme = off` session refuses.
    pub(super) fn dressed(&self) -> (Option<theme::Look>, Option<&str>) {
        let mut first_zone = None;
        for name in self
            .home
            .as_deref()
            .into_iter()
            .chain(self.model.selected())
        {
            let Some((look, zone)) = self.looks.get(name) else {
                return (None, first_zone);
            };
            if look.is_some() {
                return (*look, zone.as_deref());
            }
            first_zone = first_zone.or(zone.as_deref());
        }
        if !self.fleeted {
            return (None, first_zone);
        }
        (Some(theme::Look::DEFAULT), first_zone)
    }

    /// Hand one reading of whether the writer may still write to the
    /// composer. One that ends it ends the writing too, HELD with its reason
    /// and the draft kept; any other changes nothing.
    pub(super) fn take(&mut self, reading: Reading, _at: Instant) {
        if let Reading::NotOwner(why) = reading
            && (self.composing() || self.held.is_some())
        {
            self.write(false);
            self.held = Some(why);
        }
    }

    /// A draft edit invalidates a selection even when the edit is offscreen.
    pub(super) fn clear_composer_selection(&mut self) {
        self.select
            .take_if(|held| held.sel.panel() == select::Panel::Composer);
    }

    /// Drop a selection that the model has since moved out from under: a
    /// session, a tab or an overlay changed, even if it changed back. Whether
    /// a highlight went away.
    pub(super) fn stale_selection(&mut self) -> bool {
        let epoch = self.model.epoch();
        self.read_press.take_if(|(_, _, pressed)| *pressed != epoch);
        let stale = self.select.take_if(|held| held.epoch != epoch);
        stale.is_some_and(|held| held.sel.moved())
    }

    /// A drag, or the copy it asked for, owns the screen: the cells it lit are
    /// the cells that ship. Not past [`LANE_HOLD`] without mouse input.
    pub(super) fn holds_lane(&self) -> bool {
        (self.select.as_ref()).is_some_and(|held| {
            (held.down || held.stage != Stage::None) && held.touched.elapsed() < LANE_HOLD
        })
    }

    /// Keep a read back while a drag holds the screen: one per kind and name,
    /// the newest, carrying what the one it replaced alone had read.
    pub(super) fn park(&mut self, mut answer: Answer) {
        let same = |old: &Answer| match (old, &answer) {
            (Answer::Fleet(_), Answer::Fleet(_))
            | (Answer::ComposerRole { .. }, Answer::ComposerRole { .. })
            | (Answer::Live { .. }, Answer::Live { .. }) => true,
            (Answer::Look { name: a, .. }, Answer::Look { name: b, .. }) => a == b,
            (Answer::View(a), Answer::View(b)) => a.name == b.name,
            _ => false,
        };
        if let Some(at) = self.parked.iter().position(same)
            && let (Answer::View(old), Answer::View(new)) = (self.parked.remove(at), &mut answer)
            && old.id == new.id
        {
            new.needs = new.needs.take().or(old.needs);
            new.roster = new.roster.take().or(old.roster);
        }
        self.parked.push(answer);
    }

    /// Let the parked reads in, oldest first, once nothing holds them: the drag
    /// ended and its copy shipped, or it went quiet for [`LANE_HOLD`] and is
    /// cleared, saying so. Whether any landed.
    pub(super) fn unpark(&mut self) -> bool {
        if self.parked.is_empty() || self.holds_lane() {
            return false;
        }
        if (self.select)
            .take_if(|held| held.down || held.stage != Stage::None)
            .is_some()
        {
            let line = format!(
                "selection cleared: no mouse input for {} s",
                LANE_HOLD.as_secs()
            );
            self.flash = Some((line, Instant::now()));
        }
        for answer in std::mem::take(&mut self.parked) {
            self.answer(answer);
        }
        true
    }

    /// A newer peek capture waits while a selection in the panel is held down
    /// or has a copy to ship; once shipped or cleared the next frame adopts it.
    pub(super) fn holds_peek(&self) -> bool {
        (self.select.as_ref()).is_some_and(|held| {
            held.sel.panel() == select::Panel::Peek && (held.down || held.stage != Stage::None)
        })
    }

    /// The selection against the frame just drawn into `buf`. The held
    /// fingerprint is compared with these NEW cells, before they replace
    /// `shot` or wear the highlight: a retained shot could never differ from
    /// itself. Passing, a requested copy becomes the very text the highlight
    /// covers; failing, the selection and its copy are gone. The cells are
    /// checked against the map of the column that drew them: the panel's for
    /// a selection made in the panel, gone with the panel.
    pub(super) fn settle_selection(&mut self, buf: &mut Buffer) {
        let epoch = self.model.epoch();
        let owner = if self.select_on_panel {
            (self.orch.panel.as_ref()).map(|panel| &panel.column.layout.map)
        } else {
            Some(&self.layout.map)
        };
        let proven = (self.select.as_ref()).map(|held| {
            owner.is_some_and(|map| {
                let region = map.region(held.sel.panel());
                held.epoch == epoch
                    && region.is_some_and(|region| region.area == held.sel.area())
                    && select::fingerprint(buf, held.sel.area(), &map.blanks) == held.sel.fp()
            })
        });
        self.shot.clone_from(buf);
        match (proven, self.select.as_mut(), owner) {
            (Some(true), Some(held), Some(map)) => {
                if held.stage == Stage::Requested {
                    held.stage = Stage::Ready(select::text(buf, &held.sel, &map.blanks));
                }
                if held.sel.moved() {
                    select::highlight(buf, &held.sel, &map.blanks);
                }
            }
            (Some(false), Some(_), _) => {
                let lost = self.select.take();
                if lost
                    .is_some_and(|held| held.down && held.sel.moved() || held.stage != Stage::None)
                {
                    let line = "selection cleared: the screen changed".to_owned();
                    self.flash = Some((line, Instant::now()));
                    self.again = true;
                }
            }
            _ => {}
        }
    }

    /// `/open <seat>` typed in the composer: a seat of the session this app
    /// writes, as the incarnation its entry proved names it.
    pub(super) fn open_written(&mut self, seat: &SeatRef) -> Option<String> {
        let Some(writer) = &self.writer else {
            return Some("refused: not writing".to_owned());
        };
        let (name, uuid) = (
            writer.console.name().to_owned(),
            writer.console.uuid().to_owned(),
        );
        self.open(&name, &uuid, Ok(seat), &seat.name)
    }

    /// `o`: the first press arms on the seat the frame highlights and moves
    /// nothing; the second within the window opens THAT seat, whatever the
    /// frame drew since. A frame with no seat to open answers at once.
    pub(super) fn open_key(&mut self, origin: Instant) {
        // A still-current arm completes from the frame it saw, before the
        // fresh frame is asked anything.
        let stored = self.armed.as_ref().and_then(|armed| match &armed.what {
            Arm::Open(drawn) if origin.saturating_duration_since(armed.at) <= QUIT_WINDOW => {
                Some(drawn.clone())
            }
            _ => None,
        });
        let selected = self.model.selected();
        let fresh = (self.drawn.clone())
            .filter(|drawn| selected == Some(drawn.session.as_str()) && drawn.seat().is_ok());
        let Some(drawn) = stored.or(fresh) else {
            self.armed = None;
            return self.open_focused();
        };
        if let Some(Arm::Open(armed)) = self.press(Arm::Open(drawn), origin) {
            self.open_drawn(Some(armed));
        }
    }

    /// The seat the last frame highlighted, as that frame drew it.
    pub(super) fn open_focused(&mut self) {
        self.open_drawn(self.drawn.clone());
    }

    /// The seat `drawn` highlighted, as that frame drew it. A selection that
    /// moved since is refused, never followed.
    fn open_drawn(&mut self, drawn: Option<Drawn>) {
        let Some(session) = self.model.selected().map(str::to_owned) else {
            return;
        };
        let line = match drawn {
            Some(drawn) if drawn.session == session => {
                let label = drawn.focused.as_deref().unwrap_or(&session);
                self.open(&session, &drawn.uuid, drawn.seat(), label)
            }
            _ => Some(Refusal::NoSeat("the view changed".to_owned()).line(&session)),
        };
        if let Some(line) = line {
            self.notice(&session, line);
        }
        self.show();
    }

    /// Open `seat` of `session`, bound to the incarnation `uuid`. Inside tmux:
    /// the chat's proof, then the one client that pressed the key handed to a
    /// clean view of the seat's pane, and the line to tell, refused or opened.
    /// Outside tmux: this terminal is handed to the seat's pane once the frame
    /// is out ([`App::open_outside`]), and there is nothing to tell yet.
    pub(super) fn open(
        &mut self,
        session: &str,
        uuid: &str,
        seat: Result<&SeatRef, Refusal>,
        label: &str,
    ) -> Option<String> {
        if self.server.is_some() {
            return Some(self.open_in_tmux(session, uuid, seat, label));
        }
        self.open_outside(session, uuid, seat, label)
    }

    /// Outside tmux there is no client to move, so the app's own terminal goes
    /// to the seat's pane the way a click does ([`paneview`]), with the draft
    /// and the lane left as they are. What needs no read is refused now, in the
    /// chat's words; the pane view proves the pane on the session's recorded
    /// server, under its lock and right before it attaches.
    fn open_outside(
        &mut self,
        session: &str,
        uuid: &str,
        seat: Result<&SeatRef, Refusal>,
        label: &str,
    ) -> Option<String> {
        if (self.entry(session)).is_some_and(|entry| entry.status == Status::Stopped) {
            return Some(Refusal::Stopped(session.to_owned()).line(label));
        }
        let Some(dir) = self.dirs.get(session).cloned() else {
            let why = format!("{session} has no record directory ae can read");
            return Some(Refusal::NoSeat(why).line(label));
        };
        let seat = match seat {
            Ok(seat) => seat.clone(),
            Err(why) => return Some(why.line(label)),
        };
        let ask = paneview::Ask {
            session: session.to_owned(),
            uuid: uuid.to_owned(),
            dir,
            seat,
            label: label.to_owned(),
        };
        self.queue_pane_view(ask, paneview::Verb::Open);
        None
    }

    /// [`App::open`] inside tmux, where the app's own server and pane are known.
    fn open_in_tmux(
        &self,
        session: &str,
        uuid: &str,
        seat: Result<&SeatRef, Refusal>,
        label: &str,
    ) -> String {
        let (Some(server), Some(pane)) = (&self.server, &self.pane) else {
            return Refusal::NoTmux.line(label);
        };
        if (self.entry(session)).is_some_and(|entry| entry.status == Status::Stopped) {
            return Refusal::Stopped(session.to_owned()).line(label);
        }
        let Some(dir) = self.dirs.get(session) else {
            let why = format!("{session} has no record directory ae can read");
            return Refusal::NoSeat(why).line(label);
        };
        let target = match proven_pane(server, session, uuid, dir, seat) {
            Ok(target) => target,
            Err(why) => return why.line(label),
        };
        // Read last: what the clients show is as near the move as it gets.
        let Some(clients) = transport::observe_clients(server) else {
            return Refusal::Unread("the tmux clients".to_owned()).line(label);
        };
        let step = match open::mover(&clients, pane, session) {
            Ok(step) => step,
            Err(why) => return why.line(label),
        };
        // Only the one client that pressed the key moves, into a clean view
        // of the seat; F12 or C-] takes it back to this app. A tie names no
        // client and moves nothing, in the app's own session as in another.
        let client = match step {
            Move::Client(client) => client,
            Move::Here => match open::copy_client(&clients, pane) {
                Ok(client) => client,
                Err(why) => return why.line(label),
            },
        };
        let viewer = paneview::Viewer {
            client: &client,
            app_pane: pane,
        };
        paneview::open_inside(server, dir, &target, &viewer, session, label, uuid)
    }

    /// `n` / `p`: the seat cursor steps to the next or previous seat the last
    /// frame drew, wrapping; with no cursor yet, `n` lands on the first seat
    /// and `p` on the last. Whether it moved.
    pub(super) fn move_focus(&mut self, forward: bool) -> bool {
        let selected = self.model.selected();
        let Some(drawn) = self
            .drawn
            .as_ref()
            .filter(|d| selected == Some(d.session.as_str()))
        else {
            return false;
        };
        let count = drawn.rows.len();
        if count == 0 {
            return false;
        }
        let at = (self.model.focus())
            .and_then(|name| drawn.rows.iter().position(|(row, _)| row == name));
        let to = match (at, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(at), true) => (at + 1) % count,
            (Some(at), false) => (at + count - 1) % count,
        };
        let Some(name) = drawn.rows.get(to).map(|(name, _)| name.clone()) else {
            return false;
        };
        self.focus_seat(Some(name));
        true
    }

    /// Highlight seat `name`: the model keeps the preference, the frozen
    /// frame the cursor the keys after it step from.
    pub(super) fn focus_seat(&mut self, name: Option<String>) {
        if let (Some(drawn), Some(name)) = (&mut self.drawn, &name)
            && drawn.rows.iter().any(|(row, _)| row == name)
        {
            drawn.focused = Some(name.clone());
        }
        self.model.set_focus(name);
    }

    /// Freeze what the frame just drew of `session`'s seats.
    pub(super) fn freeze(&self, session: &str) -> Drawn {
        let shown = self.shown.get(session);
        let roster = shown
            .and_then(|shown| shown.roster.as_deref())
            .unwrap_or_default();
        let held = |name: &String| roster.iter().find(|seat| seat.name == *name).cloned();
        Drawn {
            session: session.to_owned(),
            uuid: shown.map(|shown| shown.id.clone()).unwrap_or_default(),
            rows: (self.layout.seats.iter())
                .map(|name| (name.clone(), held(name)))
                .collect(),
            focused: self.layout.focused.clone(),
        }
    }
}
