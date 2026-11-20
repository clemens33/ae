//! The chat input: entering the write mode, the lane shown, the route of a typed line, the draft and the guard on quitting.

use super::{
    App, Arm, Armed, Binding, Console, Effect, Entry, Input, Instant, Key, Lane, PathBuf,
    QUIT_WINDOW, Rc, Reading, Request, SessionEntry, Status, Target, Timestamp, doors, guarded_at,
    names, overview, route, submit, term,
};

#[cfg(doc)]
use super::QUIT_GUARD;

/// The selection's lane as drawn — its last read with ae's own notices for
/// it merged in by time — and what it was merged from, so a show rebuilds it
/// only when the selection, the read or the notices changed.
pub(super) struct Merged {
    name: String,
    base: Option<Rc<Lane>>,
    notices: u64,
    lane: Rc<Lane>,
}

impl App {
    /// Start or stop writing the selection: the one writer of the entry. A
    /// refused start is HELD. Each is a new entry, and the reader is told
    /// which writer it re-proves.
    pub(super) fn write(&mut self, on: bool) {
        if let Some(input) = self.writer_input_mut() {
            input.cancel_confirmation();
        }
        (self.entered, self.held, self.writer) = (None, None, None);
        self.admitted = None;
        self.entries = self.entries.wrapping_add(1);
        if let Some(name) = self.model.selected().filter(|_| on).map(str::to_owned)
            && let Err(why) = self.enter(&name)
        {
            self.held = Some(why);
        }
        if let Some(ask) = &self.ask {
            let binding = self.writer.as_ref().map(|writer| Binding {
                entry: writer.entry,
                console: Console::bound(
                    writer.console.name().to_owned(),
                    writer.console.dir().to_path_buf(),
                    writer.console.uuid().to_owned(),
                ),
                seats: writer.seats.clone(),
            });
            let _ = ask.send(Request::Writing(binding.map(Box::new)));
        }
    }

    /// Enter writing session `name` in this window, bound to the incarnation
    /// and lead pair it proves now — the pair its input was first written
    /// with wins. No writer or admission lock is taken: any number of windows
    /// write one session at once, and only a submit takes the lease. The
    /// entry's instant gates every key after, and an empty composer gets the
    /// kept line back.
    fn enter(&mut self, name: &str) -> Result<(), String> {
        let dir = self.writable(name)?;
        let console = Console::bound(name.to_owned(), dir, self.id(name));
        let pair = console.seats().and_then(term::pair_of)?;
        let key = (name.to_owned(), console.uuid().to_owned());
        let seats = match self.bound.get(&key) {
            Some(bound) => term::still(bound, Ok(pair)).map(|()| bound.clone())?,
            None => pair,
        };
        // Stamped before the recovery work: keys read while it runs still count.
        let at = Instant::now();
        let record = self.recovery.record(console.dir())?;
        self.entered = Some(Entry { at, named: false });
        let input = self.inputs.entry(key.clone()).or_insert_with(|| {
            self.bound.insert(key, seats.clone());
            let mut input = Input::new(names(&seats));
            let _ = input.tick(Reading::Owner, at);
            input
        });
        let how = "(Agents: n/p, oo opens a seat)";
        let restored = if input.offering() {
            input.reoffer()
        } else if input.draft().is_empty() {
            input.restore_for(submit::recover(&record), how)
        } else {
            Vec::new()
        };
        self.writer = Some(Binding {
            entry: self.entries,
            console,
            seats,
        });
        self.effects(restored);
        Ok(())
    }

    /// Session `name`'s record directory when it may be written now, else
    /// why not.
    pub(super) fn writable(&self, name: &str) -> Result<PathBuf, String> {
        if !self.fleeted {
            return Err("the fleet is not read yet".to_owned());
        }
        let unrecorded = || format!("{name} has no record directory ae can read");
        let dir = self.dirs.get(name).ok_or_else(unrecorded)?;
        if self.entry(name).map(|entry| entry.status) == Some(Status::Stopped) {
            return Err(format!("{name} is stopped"));
        }
        let unread = "the lead pair is not read yet";
        let pair = self.pairs.get(name).ok_or(unread)?;
        let pair = pair.as_ref().map_err(String::clone)?;
        if let Some(bound) = self.bound.get(&(name.to_owned(), self.id(name))) {
            term::still(bound, Ok(pair.clone()))?;
        }
        Ok(dir.clone())
    }

    /// Session `name`'s recorded `session_id`, empty when not read.
    pub(super) fn id(&self, name: &str) -> String {
        self.ids.get(name).cloned().unwrap_or_default()
    }

    /// The input the composer shows: the writer's, else the selection's.
    pub(super) fn shown_input(&self) -> Option<&Input> {
        let name = self.model.selected()?;
        let writer = (self.writer.as_ref())
            .filter(|writer| writer.console.name() == name)
            .map(Binding::key);
        let key = writer.unwrap_or_else(|| (name.to_owned(), self.id(name)));
        self.inputs.get(&key)
    }

    /// Where a plain line goes: the lead unless it cannot take the line
    /// ([`route`]). A line that names a seat with `@` goes to that seat alone;
    /// the next plain line routes again.
    pub(super) fn routed(
        &self,
        session: &str,
        raw: &[u8],
        seat: String,
        body: String,
    ) -> (String, String, Option<String>) {
        if raw.starts_with(b"@") {
            return (seat, body, None);
        }
        let Some(route) = self.route_of(session) else {
            return (seat, body, None);
        };
        let body = match route.prefix {
            Some(prefix) => format!("{prefix}\n{body}"),
            None => body,
        };
        (route.seat, body, route.note)
    }

    /// The route a plain line to `session` takes now, or `None` when the
    /// router names no seat. The one reading both the send and the composer's
    /// address ask, so the address cannot say a seat the line will not reach.
    fn route_of(&self, session: &str) -> Option<route::Route> {
        let agents = self.entry(session).map(|entry| entry.agents.as_slice());
        let takes: Vec<_> = (self.pair.iter())
            .map(|name| {
                let agent = agents.and_then(|agents| agents.iter().find(|a| a.name == *name));
                (name.as_str(), route::take(agent))
            })
            .collect();
        // Each seat's judged quota, matched to the pair by name; a reading of
        // another incarnation of the session weighs nothing.
        let read = (self.pressures.get(session))
            .filter(|(id, _)| *id == self.id(session))
            .map(|(_, seats)| seats.as_slice())
            .unwrap_or_default();
        let quota: Vec<Option<f64>> = (self.pair.iter())
            .map(|name| {
                read.iter()
                    .find(|(seat, _)| seat == name)
                    .and_then(|(_, used)| *used)
            })
            .collect();
        let route = route::decide(&takes, &quota, route::thread_seat(&self.lane.items));
        (!route.seat.is_empty()).then_some(route)
    }

    /// The seat the draft would reach and whether the draft named it with
    /// `@`, for the composer's address: `None` for a slash command, a `!`
    /// line, a refused line or a pair the router names no seat in.
    pub(super) fn destination(&self, session: &str) -> Option<(String, bool)> {
        let draft = self.draft.as_bytes();
        match draft.first() {
            Some(b'/' | b'!') => None,
            Some(b'@') => match crate::console::input::command(draft, &self.pair) {
                crate::console::input::Command::Ask { seat, .. } => Some((seat, true)),
                _ => None,
            },
            _ => self.route_of(session).map(|route| (route.seat, false)),
        }
    }

    /// Whether this window writes: it entered, holding no lock for it.
    pub(super) fn composing(&self) -> bool {
        self.entered.is_some()
    }

    /// Show the selected session from what was answered: its last read lane,
    /// or `loading` until its first read lands — never another session's
    /// lane — and its open asks and working dir from the fleet's read.
    pub(super) fn show(&mut self) {
        let (now, user) = (Timestamp::now(), doors::home());
        let Some(name) = self.model.selected().map(str::to_owned) else {
            (self.lane, self.overview) = (Rc::default(), overview::Overview::default());
            self.loading = false;
            return;
        };
        // An inert panel shows no lane and writes nothing: its one line says why.
        if let Some(line) = self.panel_line().filter(|_| self.orch.active) {
            (self.lane, self.overview) = (Rc::default(), overview::Overview::default());
            (self.merged, self.loading, self.pair) = (None, false, Vec::new());
            self.read_only.clone_from(&line);
            self.read_line = line;
            return;
        }
        self.read_only = self.writable(&name).err().unwrap_or_default();
        let stopped = format!("{name} is stopped");
        self.read_line = if self.read_only == stopped {
            format!("{stopped}; ae {name} resumes it")
        } else {
            self.read_only.clone()
        };
        if self.held.is_some() && !self.read_only.is_empty() && !self.orch.active {
            self.held = Some(self.read_only.clone());
        }
        let pair = self.pairs.get(&name).and_then(|pair| pair.as_deref().ok());
        self.pair = pair.map(names).unwrap_or_default();
        if !self.fleeted {
            (self.lane, self.overview) = (Rc::default(), overview::Overview::default());
            self.loading = true;
            return;
        }
        let empty = SessionEntry::new(&name, Status::Unknown);
        let entry = self.entry(&name).unwrap_or(&empty).clone();
        if !self.dirs.contains_key(&name) {
            let gap = format!("{name} has no record directory ae can read");
            let lane = Rc::new(Lane {
                items: Vec::new(),
                coverage: vec![gap.clone()],
            });
            self.lane = self.merge(&name, Some(lane));
            self.loading = false;
            self.overview = overview::of(&entry, None, user.as_deref(), now);
            return;
        }
        // The bound panel draws only its own incarnation's read: a lane
        // another uuid answered under the seat's name stays `loading`.
        let pin = (self.orch.panel.as_ref())
            .and_then(super::panel::Panel::pin)
            .filter(|_| self.orch.active);
        let shown = (self.shown.get(&name))
            .filter(|shown| pin.is_none_or(|pin| shown.id == pin))
            .map(|shown| Rc::clone(&shown.lane));
        self.loading = shown.is_none();
        self.lane = self.merge(&name, shown);
        self.overview = overview::of(&entry, self.needs.get(&name), user.as_deref(), now);
    }

    /// Session `name`'s lane `base` with ae's own notices in it by time —
    /// its own, and those of a session no longer recorded — built once per
    /// read and per notice rather than on every show.
    pub(super) fn merge(&mut self, name: &str, base: Option<Rc<Lane>>) -> Rc<Lane> {
        let same = |merged: &&Merged| {
            merged.notices == self.noticed
                && merged.name == name
                && merged.base.as_ref().map(Rc::as_ptr) == base.as_ref().map(Rc::as_ptr)
        };
        if let Some(merged) = self.merged.as_ref().filter(same) {
            return Rc::clone(&merged.lane);
        }
        let dirs = &self.dirs;
        let mut notices = (self.notices.iter())
            .filter(|(of, _)| of == name || !dirs.contains_key(of))
            .map(|(_, item)| item.clone())
            .peekable();
        if notices.peek().is_none() {
            return base.unwrap_or_default();
        }
        let mut lane = base.as_deref().cloned().unwrap_or_default();
        lane.items.extend(notices);
        lane.items.sort_by_key(|item| item.micros);
        let lane = Rc::new(lane);
        self.merged = Some(Merged {
            name: name.to_owned(),
            base,
            notices: self.noticed,
            lane: Rc::clone(&lane),
        });
        lane
    }

    pub(super) fn entry(&self, name: &str) -> Option<&SessionEntry> {
        self.world.sessions.iter().find(|entry| entry.name == name)
    }

    /// Whether Enter may start a line: the selection's last reading proved
    /// it, and no seat's screen stands in the chat's place.
    pub(super) fn can_compose(&self) -> bool {
        self.read_only.is_empty()
            && self.model.selected().is_some()
            && !self.model.peek_open()
            && !self.model.reader_open()
    }

    pub(super) fn target(&self, name: &str) -> Target {
        let panel = self.orch.active && name == super::ORCH && self.panel_line().is_none();
        Target::of(name, panel || self.orch.holder.as_deref() == Some(name))
    }

    pub(super) fn writer_input(&self) -> Option<&Input> {
        self.inputs.get(&self.writer.as_ref()?.key())
    }

    pub(super) fn writer_input_mut(&mut self) -> Option<&mut Input> {
        self.inputs.get_mut(&self.writer.as_ref()?.key())
    }

    pub(super) fn warning_unpainted(&self) -> bool {
        self.writer_input().is_some_and(Input::warning_unpainted)
    }

    fn compose_effects(&mut self, key: Key, origin: Instant) -> Vec<Effect> {
        let Some(writer) = &self.writer else {
            return Vec::new();
        };
        let target = self.target(writer.console.name());
        let fleet = (key == Key::Enter).then(|| {
            let running = self
                .world
                .sessions
                .iter()
                .filter(|entry| entry.status == Status::Running)
                .map(|entry| entry.name.clone())
                .collect::<Vec<_>>();
            let route = self
                .orch
                .route
                .as_ref()
                .filter(|route| {
                    self.entry(&route.name)
                        .is_some_and(|entry| entry.status == Status::Running)
                        && self.ids.get(&route.name) == Some(&route.uuid)
                })
                .cloned();
            (running, route)
        });
        let Some(input) = self.writer_input_mut() else {
            return Vec::new();
        };
        input.set_target(target);
        if let Some((running, route)) = fleet {
            input.set_running(Some(running));
            input.set_route(route);
        }
        input.keyed(vec![(key, origin)])
    }

    /// One key while composing: Esc keeps the draft and browses, ^C over an
    /// empty composer quits (over text it clears, see [`App::clear_draft`]),
    /// one read before the entry began is dropped, and anything else is
    /// the chat's own input. `None` quits.
    pub(super) fn compose(&mut self, key: Key, origin: Instant) -> Option<()> {
        // The browse paste that started this entry keeps every fragment.
        let origin = match (&key, &self.entered) {
            (Key::Pasted(_), Some(entry)) if self.admitted == Some(origin) => entry.at,
            _ => origin,
        };
        match key {
            Key::Escape if !self.writer_input().is_some_and(Input::takes_escape) => {
                self.write(false);
            }
            Key::Interrupt if self.clear_draft(origin) => {}
            Key::Interrupt => return None,
            _ if self.entered.as_ref().is_some_and(|entry| origin < entry.at) => {
                if let Some(entry) = self.entered.as_mut().filter(|entry| !entry.named) {
                    entry.named = true;
                    let line = "dropped the keys typed before writing started";
                    self.effects(vec![Effect::Print(line.to_owned())]);
                }
            }
            key => {
                let had_text = self.protects();
                let deleting = matches!(
                    key,
                    Key::ClearLine | Key::KillBigWord | Key::KillWord | Key::KillToEnd
                );
                let effects = self.compose_effects(key, origin);
                if deleting && had_text && !self.protects() {
                    self.cleared_note(origin);
                }
                if !effects.is_empty() {
                    if self.warning_unpainted() {
                        let _ = self.model.newest();
                    }
                    self.effects(effects);
                    self.reread();
                }
            }
        }
        Some(())
    }

    /// One key while HELD: Esc browses, ^C quits, Enter tries the entry
    /// again, and anything else is swallowed. `None` quits.
    pub(super) fn retry(&mut self, key: &Key) -> Option<bool> {
        match key {
            Key::Interrupt => return None,
            Key::Escape => self.write(false),
            Key::Enter => self.write(true),
            Key::Pasted(_) => self.refuse_paste("not writing - Esc browses first"),
            _ => return Some(false),
        }
        Some(true)
    }

    /// ^C over text in the composer on show — the writer's draft, or the one
    /// Esc kept — clears it whole and keeps it for `^Y`; the app never quits
    /// over text. Whether it cleared.
    pub(super) fn clear_draft(&mut self, origin: Instant) -> bool {
        let Some(name) = self.model.selected().map(str::to_owned) else {
            return false;
        };
        let writer = self.writer.as_ref().map(Binding::key);
        let key = writer.unwrap_or_else(|| (name.clone(), self.id(&name)));
        let cleared = self.inputs.get_mut(&key).is_some_and(Input::clear);
        if cleared {
            self.cleared_note(origin);
        }
        cleared
    }

    /// ^C over an empty composer within [`QUIT_GUARD`] of a clear arms a quit
    /// instead of making it, so a ^C meant for the text never ends the app.
    /// Whether it only armed.
    pub(super) fn guard_quit(&mut self, origin: Instant) -> bool {
        if !self.guarded(origin) {
            return false;
        }
        let armed = self.press(Arm::Interrupt, origin).is_none();
        self.cleared_at = self.cleared_at.filter(|_| armed);
        armed
    }

    /// Whether a ^C pressed at `origin` falls inside [`QUIT_GUARD`] of the last
    /// clear: the press's own time, never the clock when it is looked at.
    fn guarded(&self, origin: Instant) -> bool {
        guarded_at(self.cleared_at, origin)
    }

    /// The hint row's word after a clear: how the text comes back.
    fn cleared_note(&mut self, origin: Instant) {
        self.cleared_at = Some(origin);
        let how = if self.composing() {
            "^Y"
        } else {
            "Enter, then ^Y"
        };
        let line = format!("draft cleared · {how} brings it back");
        self.flash = Some((line, Instant::now()));
    }

    /// Whether the composer on show holds text a ^C would clear.
    pub(super) fn has_text(&self) -> bool {
        self.shown_input()
            .is_some_and(|input| !input.draft().is_empty())
    }

    /// Whether a ^C here would lose a draft only this process holds.
    pub(super) fn protects(&self) -> bool {
        let input = self.writer.as_ref().map(Binding::key);
        let input = input.and_then(|key| self.inputs.get(&key));
        input.is_some_and(|input| !input.draft().is_empty())
    }

    /// Press `what`: the arm it completes (the same kind, left within the
    /// window by the key before), else it arms and nothing completes.
    pub(super) fn press(&mut self, what: Arm, origin: Instant) -> Option<Arm> {
        let kind = std::mem::discriminant(&what);
        let held = |armed: &Armed| {
            std::mem::discriminant(&armed.what) == kind
                && origin.saturating_duration_since(armed.at) <= QUIT_WINDOW
        };
        let completed = self.armed.take().filter(held);
        if completed.is_none() {
            self.armed = Some(Armed { at: origin, what });
        }
        completed.map(|armed| armed.what)
    }

    /// A paste the app did not take: the hint row says why.
    pub(super) fn refuse_paste(&mut self, why: &str) {
        self.flash = Some((format!("paste not taken: {why}"), Instant::now()));
    }
}
