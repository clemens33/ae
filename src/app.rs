//! `ae app`: the Side Quiet layout — the fleet in a sidebar, the selected
//! session's Overview or Agents beside it, and the selected session's chat.
//!
//! The pure halves live in the submodules: [`fleet`] folds the sidebar rows,
//! [`overview`] the Overview tab, [`model`] the browse reducer and [`draw`] the
//! cells. Each reads only what the existing owners already computed. The
//! world is read on the [`loader`]'s one background thread, through those
//! owners; this file draws what it last answered, so no key waits on a read.
//! The ONE write is the composer's ask or close into the selected session,
//! through the chat's own admission path (`term::submit_ask`,
//! `submit::close_owned`), under that session's writer lease, held while
//! this app writes it; nothing else here writes into any session.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{IsTerminal as _, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use ratatui_core::buffer::Buffer;
use ratatui_core::terminal::Terminal;

use crate::console::input::{Effect, Input, Key, Keys, Mouse, MouseKind, Reading, Size, View};
use crate::console::lane::{Item, Kind, Lane, Seat};
use crate::console::needs::Section;
use crate::console::{self, Console, submit, term};
use crate::digest::{SessionEntry, Status};
use crate::inventory::ServerId;
use crate::listing::World;
use crate::time::Timestamp;
use crate::{brief, doors, theme, tmux};

use loader::{Answer, Binding, Request, Wake};

mod backend;
pub mod draw;
pub mod fleet;
mod lane;
#[cfg(test)]
mod lane_spec;
mod loader;
pub mod model;
#[cfg(test)]
mod order_spec;
pub mod overview;
mod settings;
mod tty;

/// What `ae app` says, on stderr, when it has no terminal to draw on.
pub const NO_TERMINAL: &str = "ae app draws a terminal UI; use ae chat to print the lane\n";

/// The usage text.
pub const USAGE: &str = "Usage: ae app [session]\n\n  session   the home session, selected first (default: the session this pane belongs to)\n";

/// How often the fleet, home and the selection are read again.
const REFRESH: Duration = Duration::from_secs(crate::board::follow::POLL_SECS);
/// How long a wait for a key lasts before the size and the Esc bound are read.
const TICK: Duration = Duration::from_millis(100);
/// The most of ae's own notices a session's lane shows.
const NOTICES: usize = 5;

/// One session's last read lane, and what it was read as.
struct Shown {
    /// The `session_id` the reading console was bound to.
    id: String,
    seq: u64,
    lane: Rc<Lane>,
}

/// The selection's lane as drawn — its last read with ae's own notices for
/// it merged in by time — and what it was merged from, so a show rebuilds it
/// only when the selection, the read or the notices changed.
struct Merged {
    name: String,
    base: Option<Rc<Lane>>,
    notices: u64,
    lane: Rc<Lane>,
}

/// The writer lease this app holds while it writes.
struct Lease {
    /// Held open: dropping it releases the lease.
    _file: File,
    /// When it was taken: no key read before it is draft.
    at: Instant,
    /// The keys dropped for that have been named.
    named: bool,
}

/// Everything the app was answered, and the browse and compose state.
struct App {
    home: Option<String>,
    model: model::Model,
    fleet: fleet::Fleet,
    world: World,
    dirs: BTreeMap<String, PathBuf>,
    ids: BTreeMap<String, String>,
    facts: BTreeMap<String, fleet::Facts>,
    needs: BTreeMap<String, Section>,
    memos: BTreeMap<String, Result<Vec<brief::Filed>, String>>,
    /// Each session's lead pair as the last fleet read found it.
    pairs: BTreeMap<String, Result<Vec<Seat>, String>>,
    /// Each session's last read lane, by name: what a selection shows at once.
    shown: BTreeMap<String, Shown>,
    /// Each session's drawn look and the viewer's zone, by name.
    looks: BTreeMap<String, (Option<theme::Look>, Option<String>)>,
    lane: Rc<Lane>,
    merged: Option<Merged>,
    /// The selection's lane has not been read yet.
    loading: bool,
    /// The fleet has been read at least once.
    fleeted: bool,
    overview: overview::Overview,
    /// The selection's lead pair as its meta names it now, main first.
    pair: Vec<String>,
    /// Each incarnation's input, by `(name, session_id)`: its draft and
    /// speaker outlive a writer, until a fleet read proves it gone.
    inputs: BTreeMap<(String, String), Input>,
    /// The lead pair each incarnation was first written with: a changed pair
    /// refuses rather than sending elsewhere.
    bound: BTreeMap<(String, String), Vec<Seat>>,
    /// Why the selection's composer only reads.
    read_only: String,
    /// The writer lease, held exactly while this app writes.
    lease: Option<Lease>,
    /// The entry writing now, bound to what it proved; with the lease.
    writer: Option<Binding>,
    /// Entries counted, so a late proof of an older one changes nothing.
    entries: u64,
    /// Why the last entry was refused: HELD, every key swallowed until Esc,
    /// ^C, a click or an Enter that tries again.
    held: Option<String>,
    /// ae's own lines — refusals and outcomes — by the session they are about.
    notices: Vec<(String, Item)>,
    /// Bumped with every notice, so the merged lane knows it is stale.
    noticed: u64,
    draft_view: View,
    draft: String,
    layout: draw::Layout,
    /// The reader's requests; `None` for an app driven by hand.
    ask: Option<Sender<Request>>,
    /// The selection the reader was last told of.
    focused: Option<String>,
    /// Keys held, in order, behind a wheel notch the last frame could not
    /// bound: the next frame produces more of the lane, then they replay.
    deferred: Vec<(Key, Instant)>,
    /// The settings overlay's bodies, cold until its answers land.
    settings_bodies: settings::SettingsBodies,
    /// The generation the next open mints; answers pin to the open one.
    settings_generation: u64,
    /// The row order the last fleet read chose, applied when the next frame
    /// is drawn: until then keys and clicks resolve against the order the
    /// human sees.
    pending: Option<Vec<String>>,
}

impl App {
    /// An app that has read nothing yet: home is selected. Where it runs
    /// decides nothing: the writer lease does.
    fn new(
        home: Option<String>,
        _server: Option<ServerId>,
        _pane: Option<String>,
        ask: Option<Sender<Request>>,
    ) -> Self {
        let fleet = fleet::Fleet {
            rows: Vec::new(),
            home: home.clone(),
        };
        Self {
            home,
            model: model::Model::new(&fleet),
            fleet,
            world: World::new(Timestamp::now(), Vec::new()),
            dirs: BTreeMap::new(),
            ids: BTreeMap::new(),
            facts: BTreeMap::new(),
            needs: BTreeMap::new(),
            memos: BTreeMap::new(),
            pairs: BTreeMap::new(),
            shown: BTreeMap::new(),
            looks: BTreeMap::new(),
            lane: Rc::default(),
            merged: None,
            loading: true,
            fleeted: false,
            overview: overview::Overview::default(),
            pair: Vec::new(),
            inputs: BTreeMap::new(),
            bound: BTreeMap::new(),
            read_only: "the fleet is not read yet".to_owned(),
            lease: None,
            writer: None,
            entries: 0,
            held: None,
            notices: Vec::new(),
            noticed: 0,
            draft_view: View::default(),
            draft: String::new(),
            layout: draw::Layout::default(),
            ask,
            focused: None,
            deferred: Vec::new(),
            settings_bodies: settings::SettingsBodies::default(),
            settings_generation: 0,
            pending: None,
        }
    }

    /// Fold one answer from the reader.
    fn answer(&mut self, answer: Answer) {
        match answer {
            Answer::Fleet(read) => self.absorb(read),
            Answer::Look { name, look, zone } => drop(self.looks.insert(name, (look, zone))),
            // Only the entry writing now: a late proof of an older one is stale.
            Answer::Owned { entry, reading, at }
                if self.writer.as_ref().is_some_and(|w| w.entry == entry) =>
            {
                self.take(reading, at);
            }
            Answer::Owned { .. } => {}
            Answer::View(view) => self.viewed(view),
            Answer::Settings {
                generation,
                quota,
                config,
                about,
            } => self.settled(generation, quota, config, about),
        }
    }

    /// Fold one settings answer. Only the open overlay's own generation
    /// paints: a late answer for a closed overlay is dropped. Quota rides
    /// every answer; config and about ride the first per open, then stay.
    fn settled(
        &mut self,
        generation: u64,
        quota: Vec<settings::QuotaRow>,
        config: Option<settings::ConfigView>,
        about: Option<settings::AboutFacts>,
    ) {
        if !self.model.settings_generation(generation) {
            return;
        }
        self.settings_bodies.quota = Some(quota);
        if let Some(config) = config {
            self.settings_bodies.config = config;
        }
        if let Some(about) = about {
            self.settings_bodies.about = Some(about);
        }
    }

    /// Open the overlay on a fresh generation and ask the reader for it.
    fn open_settings(&mut self) {
        if self.model.settings_open() {
            return;
        }
        self.settings_generation = self.settings_generation.wrapping_add(1);
        self.model.open_settings(self.settings_generation);
        self.settings_bodies = settings::SettingsBodies::default();
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Settings {
                open: true,
                generation: self.settings_generation,
            });
        }
    }

    /// Close the overlay and tell the reader to stop reading for it. It
    /// always tells: the model may already hold the close the key caused.
    fn close_settings(&mut self) {
        self.model.close_settings();
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Settings {
                open: false,
                generation: self.settings_generation,
            });
        }
    }

    /// Fold one read of the fleet: a session that left it, or whose recorded
    /// identity changed, takes its last lane and look with it, and its input
    /// — once, named — when the read proves it (a changed `session_id`, or a
    /// complete read without it), never on an id it could not read.
    fn absorb(&mut self, read: loader::FleetRead) {
        let home = self.home.clone();
        let kept = |name: &String, id: &str| {
            home.as_ref() == Some(name) || read.ids.get(name).is_some_and(|now| now == id)
        };
        self.shown.retain(|name, shown| kept(name, &shown.id));
        let old = std::mem::take(&mut self.ids);
        self.looks.retain(|name, _| match old.get(name) {
            Some(id) => kept(name, id),
            None => home.as_ref() == Some(name) || read.ids.contains_key(name),
        });
        self.dirs = read.dirs;
        self.ids = read.ids;
        self.world = read.world;
        self.facts = read.facts;
        self.needs = read.needs;
        let drawn: Vec<String> = self.fleet.rows.iter().map(|row| row.name.clone()).collect();
        self.pending = Some(read.fleet.rows.iter().map(|row| row.name.clone()).collect());
        self.fleet = read.fleet.arranged(&drawn);
        self.memos = read.memos;
        self.pairs = read.pairs;
        self.fleeted = true;
        self.merged = None;
        let gone = |(name, id): &(String, String)| match self.ids.get(name) {
            Some(now) if !now.is_empty() && now != id => Some("the session was replaced"),
            None if read.scanned => Some("the session is gone"),
            _ => None,
        };
        // R-B4 ext: a HELD entry's reason follows its selection's read.
        let was = |name: &str| Some((name.to_owned(), old.get(name)?.clone()));
        let was = (self.model.selected().and_then(was)).filter(|(_, id)| !id.is_empty());
        let lapsed = was.and_then(|key| gone(&key));
        let dropped: Vec<_> = (self.inputs.extract_if(.., |key, _| gone(key).is_some()))
            .filter_map(|(key, input)| Some((gone(&key)?, key, input)))
            .collect();
        for (why, key, input) in dropped {
            if self.writer.as_ref().is_some_and(|w| w.key() == key) {
                self.write(false);
                self.held = Some(why.to_owned());
            }
            self.bound.remove(&key);
            if !input.draft().is_empty() {
                self.notice(&key.0, format!("draft for {} dropped: {why}", key.0));
            }
        }
        if self.model.selected().is_none() {
            self.model.restart(&self.fleet);
        }
        // The target is always the selection, and an incomplete read that omits
        // it moves nothing (R-B4 ext); a read that moved it or proved its own
        // incarnation stopped ends the writing HELD, never into browse (D3).
        let writing = self.writer.as_ref().map(Binding::key);
        let listed = |name: &str| self.fleet.rows.iter().any(|row| row.name == name);
        if read.scanned || writing.as_ref().is_none_or(|(name, _)| listed(name)) {
            self.model.reconcile(&self.fleet);
        }
        if let Some((name, id)) = writing {
            let stopped = self.id(&name) == id
                && (self.entry(&name)).is_some_and(|entry| entry.status == Status::Stopped);
            if stopped || Some(name.as_str()) != self.model.selected() {
                self.write(false);
                let why = if stopped {
                    "is stopped"
                } else {
                    "is no longer listed"
                };
                self.held = Some(format!("{name} {why}"));
            }
        }
        if let Some(why) = lapsed.filter(|_| self.held.is_some()) {
            self.held = Some(why.to_owned());
        }
        self.evict();
        self.show();
    }

    /// One read of a session's lane. A lane read as an identity its session
    /// no longer records, or older than the one shown, is dropped; home's
    /// lane stays until its next read.
    fn viewed(&mut self, view: loader::ViewRead) {
        let home = self.home.as_deref() == Some(view.name.as_str());
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
        // The seats `/open` names, as the chat's own tick hands them over.
        let key = (view.name.clone(), view.id.clone());
        if let (Some(seats), Some(input)) = (view.roster, self.inputs.get_mut(&key)) {
            input.set_seats(seats);
        }
        let selected = self.model.selected() == Some(view.name.as_str());
        let shown = Shown {
            id: view.id,
            seq: view.seq,
            lane: Rc::new(view.lane),
        };
        self.shown.insert(view.name, shown);
        if selected {
            self.show();
        }
    }

    /// Keep only the lanes of the sessions the reader keeps read.
    fn evict(&mut self) {
        let rows: Vec<String> = self.fleet.rows.iter().map(|row| row.name.clone()).collect();
        let keep = loader::order(&rows, self.model.selected(), self.home.as_deref());
        self.shown
            .retain(|name, _| keep.iter().any(|(kept, _)| kept == name));
    }

    /// Tell the reader which session shows now, once per change.
    fn focus(&mut self) {
        let selected = self.model.selected().map(str::to_owned);
        if selected == self.focused {
            return;
        }
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Focus(selected.clone()));
        }
        self.focused = selected;
        self.evict();
    }

    /// Ask the reader to read the written session again: this app wrote into it.
    fn reread(&self) {
        if let (Some(ask), Some(writer)) = (&self.ask, &self.writer) {
            let _ = ask.send(Request::Reread(writer.console.name().to_owned()));
        }
    }

    /// The look the chat reads (F3): the home session's drawn look, else the
    /// selected one's, else the default drawn look. A look not read yet
    /// paints no colour rather than a guess a `theme = off` session refuses.
    fn dressed(&self) -> (Option<theme::Look>, Option<&str>) {
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
    fn take(&mut self, reading: Reading, _at: Instant) {
        if let Reading::NotOwner(why) = reading
            && (self.composing() || self.held.is_some())
        {
            self.write(false);
            self.held = Some(why);
        }
    }

    /// Start or stop writing the selection: the one writer of the lease. A
    /// refused start is HELD. Each is a new entry, and the reader is told
    /// which writer it re-proves.
    fn write(&mut self, on: bool) {
        (self.lease, self.held, self.writer) = (None, None, None);
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

    /// Enter writing session `name`, bound to the incarnation and lead pair
    /// it proves now — the pair its input was first written with wins — under
    /// its lease, taken without waiting: the instant it is held gates every
    /// key after, and an empty composer gets the kept line back. Nothing is
    /// written before the lease.
    fn enter(&mut self, name: &str) -> Result<(), String> {
        let dir = self.writable(name)?;
        let console = Console::bound(name.to_owned(), dir, self.id(name));
        let pair = console.seats().and_then(term::pair_of)?;
        let key = (name.to_owned(), console.uuid().to_owned());
        let seats = match self.bound.get(&key) {
            Some(bound) => term::still(bound, Ok(pair)).map(|()| bound.clone())?,
            None => pair,
        };
        let file = term::lease(&console)?;
        let at = Instant::now();
        let draft = submit::restore(console.dir());
        self.lease = Some(Lease {
            _file: file,
            at,
            named: false,
        });
        let input = self.inputs.entry(key.clone()).or_insert_with(|| {
            self.bound.insert(key, seats.clone());
            let mut input = Input::new(names(&seats));
            let _ = input.tick(Reading::Owner, at);
            input
        });
        let restored = input.draft().is_empty().then(|| input.restore(draft));
        self.writer = Some(Binding {
            entry: self.entries,
            console,
            seats,
        });
        self.effects(restored.unwrap_or_default());
        Ok(())
    }

    /// Session `name`'s record directory when it may be written now, else
    /// why not.
    fn writable(&self, name: &str) -> Result<PathBuf, String> {
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
    fn id(&self, name: &str) -> String {
        self.ids.get(name).cloned().unwrap_or_default()
    }

    /// The input the composer shows: the writer's, else the selection's.
    fn shown_input(&self) -> Option<&Input> {
        let name = self.model.selected()?;
        let writer = self.writer.as_ref().map(Binding::key);
        let key = writer.unwrap_or_else(|| (name.to_owned(), self.id(name)));
        self.inputs.get(&key)
    }

    /// Who the composer speaks to: its input's speaker, else the pair's main.
    fn speaker(&self) -> &str {
        self.shown_input().map_or_else(
            || self.pair.first().map_or("", String::as_str),
            Input::speaker,
        )
    }

    /// Whether this app writes home: it holds the lease.
    fn composing(&self) -> bool {
        self.lease.is_some()
    }

    /// Show the selected session from what was answered: its last read lane,
    /// or `loading` until its first read lands — never another session's
    /// lane — and its Overview from the fleet's read.
    fn show(&mut self) {
        let now = Timestamp::now();
        let Some(name) = self.model.selected().map(str::to_owned) else {
            (self.lane, self.overview) = (Rc::default(), overview::Overview::default());
            self.loading = false;
            return;
        };
        self.read_only = self.writable(&name).err().unwrap_or_default();
        if self.held.is_some() && !self.read_only.is_empty() {
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
        let listed = self.entry(&name).is_some();
        let entry = self.entry(&name).unwrap_or(&empty).clone();
        if !self.dirs.contains_key(&name) {
            let gap = format!("{name} has no record directory ae can read");
            let lane = Rc::new(Lane {
                items: Vec::new(),
                coverage: vec![gap.clone()],
            });
            self.lane = self.merge(&name, Some(lane));
            self.loading = false;
            self.overview = overview::of(&entry, None, Err(gap), now);
            self.unlist(listed);
            return;
        }
        let shown = self.shown.get(&name).map(|shown| Rc::clone(&shown.lane));
        self.loading = shown.is_none();
        self.lane = self.merge(&name, shown);
        let memo = self.memos.get(&name).map_or_else(
            || Err("memo not read yet".to_owned()),
            |memo| memo.as_deref().map_err(String::clone),
        );
        self.overview = overview::of_filed(&entry, self.needs.get(&name), memo, now);
        self.unlist(listed);
    }

    /// A selection the world read holds no entry for names that in place of
    /// launch facts it would only have guessed.
    fn unlist(&mut self, listed: bool) {
        if !listed {
            self.overview.launch = vec![overview::LAUNCH_GAP.to_owned()];
        }
    }

    /// Session `name`'s lane `base` with ae's own notices in it by time —
    /// its own, and those of a session no longer recorded — built once per
    /// read and per notice rather than on every show.
    fn merge(&mut self, name: &str, base: Option<Rc<Lane>>) -> Rc<Lane> {
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

    fn entry(&self, name: &str) -> Option<&SessionEntry> {
        self.world.sessions.iter().find(|entry| entry.name == name)
    }

    /// Whether Enter may start a line: the selection's last reading proved it.
    fn can_compose(&self) -> bool {
        self.read_only.is_empty() && self.model.selected().is_some()
    }

    /// One key while composing: Esc keeps the draft and browses, ^C quits,
    /// one read before the lease was taken is dropped, and anything else is
    /// the chat's own input. `None` quits.
    fn compose(&mut self, key: Key, origin: Instant) -> Option<()> {
        match key {
            Key::Escape => self.write(false),
            Key::Interrupt => return None,
            _ if self.lease.as_ref().is_some_and(|lease| origin < lease.at) => {
                if let Some(lease) = self.lease.as_mut().filter(|lease| !lease.named) {
                    lease.named = true;
                    let line = "dropped the keys typed before writing started";
                    self.effects(vec![Effect::Print(line.to_owned())]);
                }
            }
            key => {
                let input = self.writer.as_ref().map(Binding::key);
                let effects = input
                    .and_then(|input| self.inputs.get_mut(&input))
                    .map(|input| input.keyed(vec![(key, origin)]))
                    .unwrap_or_default();
                if !effects.is_empty() {
                    self.effects(effects);
                    self.reread();
                }
            }
        }
        Some(())
    }

    /// One key while HELD: Esc browses, ^C quits, Enter tries the lease
    /// again, and anything else is swallowed. `None` quits.
    fn retry(&mut self, key: &Key) -> Option<bool> {
        match key {
            Key::Interrupt => return None,
            Key::Escape => self.write(false),
            Key::Enter => self.write(true),
            _ => return Some(false),
        }
        Some(true)
    }

    /// A wheel notch landing past a bound the last frame has not proven the
    /// top — page keys may have asked further back — waits for the next
    /// frame rather than stopping short at it.
    fn unbounded(&self, key: &Key) -> bool {
        let Key::Mouse(mouse) = key else {
            return false;
        };
        matches!(mouse.kind, MouseKind::WheelUp | MouseKind::WheelDown)
            && self.layout.in_chat(*mouse)
            && !self.layout.complete
            && self.model.wheel_passes(
                mouse.kind == MouseKind::WheelUp,
                self.layout.page_rows,
                self.layout.max_scroll,
            )
    }

    /// Mouse actions use the last drawn frame, in either input mode. Left
    /// motion moves a dragged border; every other event first ends a drag —
    /// its release, or a press or wheel after a release that never came.
    fn mouse(&mut self, mouse: Mouse) -> bool {
        if self.model.settings_open() {
            return self.settings_mouse(mouse);
        }
        if mouse.kind == MouseKind::Drag {
            return self.drag(mouse);
        }
        let ended = self.model.let_go();
        match mouse.kind {
            MouseKind::Click => self.click(mouse) || ended,
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
            MouseKind::Drag | MouseKind::Release => ended,
        }
    }

    /// A mouse event with the overlay open: the wheel scrolls its body
    /// wherever it lands, a press on the gear or the close label closes, one
    /// on a tab title shows that tab, and any other press dies.
    fn settings_mouse(&mut self, mouse: Mouse) -> bool {
        match mouse.kind {
            MouseKind::WheelUp => {
                self.model.settings_wheel(-model::WHEEL_ROWS.cast_signed());
                true
            }
            MouseKind::WheelDown => {
                self.model.settings_wheel(model::WHEEL_ROWS.cast_signed());
                true
            }
            MouseKind::Click => match self.layout.hit(mouse) {
                Some(draw::Hit::Settings | draw::Hit::SettingsClose) => {
                    self.close_settings();
                    true
                }
                Some(draw::Hit::SettingsTab(tab)) => {
                    self.model.show_settings_tab(tab) == model::Act::Redraw
                }
                _ => false,
            },
            MouseKind::Drag | MouseKind::Release => false,
        }
    }

    /// A press: grab the border under it, else act on the target drawn
    /// there. Any press ends a HELD entry.
    fn click(&mut self, mouse: Mouse) -> bool {
        let held = self.held.take().is_some();
        if let Some((edge, at)) = self.layout.grab(mouse) {
            let from = match edge {
                model::Edge::Sidebar => mouse.column,
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
            draw::Hit::Tab(tab) => {
                self.write(false);
                self.model.show_tab(tab)
            }
            draw::Hit::Compose if !self.composing() => {
                self.model
                    .key(model::Key::Compose, &self.fleet, self.can_compose(), true)
            }
            draw::Hit::Compose => return false,
            draw::Hit::Settings => {
                self.open_settings();
                return true;
            }
            draw::Hit::SettingsTab(_) | draw::Hit::SettingsClose => return held,
        };
        apply(self, &act).unwrap_or(false) || was_writing != self.composing() || held
    }

    /// Left motion: the dragged border follows it while the last frame drew
    /// that border; a border not drawn keeps the size it was given.
    fn drag(&mut self, mouse: Mouse) -> bool {
        let Some(drag) = self.model.drag() else {
            return false;
        };
        if !self.layout.shows(drag.edge) {
            return false;
        }
        let size = draw::dragged_to(drag, mouse, self.layout.area(), self.fleet.rows.len());
        self.model.drag_to(size)
    }

    /// Carry out what the input asked for; each outcome becomes a notice,
    /// shown at once.
    fn effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            // A writer's session is always the selection.
            let about = self.model.selected().or(self.home.as_deref());
            let about = about.unwrap_or_default().to_owned();
            let line = match effect {
                Effect::Print(line) => line,
                Effect::Ask { raw, seat, body } => self.ask(&raw, &seat, body),
                Effect::Close(id) => self.close(id.as_deref()),
                Effect::Open(seat) => format!(
                    "refused: /open {}: ae app selects no pane - ae chat does",
                    seat.name
                ),
                Effect::Paste(_) | Effect::Lane(_) | Effect::Styled(_) => continue,
            };
            self.notice(&about, line);
        }
        self.show();
    }

    /// One of ae's own lines about session `name`; each keeps its newest few.
    fn notice(&mut self, name: &str, line: String) {
        let item = Item {
            micros: Timestamp::now().epoch().saturating_mul(1_000_000),
            kind: Kind::Said {
                who: "ae".to_owned(),
            },
            body: line,
            record: None,
        };
        self.notices.push((name.to_owned(), item));
        self.noticed = self.noticed.wrapping_add(1);
        let mut of = self.notices.iter().filter(|(of, _)| of == name);
        if of.nth(NOTICES).is_some()
            && let Some(first) = self.notices.iter().position(|(of, _)| of == name)
        {
            self.notices.remove(first);
        }
    }

    /// What the admission re-proves: this app holds the lease, and the
    /// incarnation and lead pair its entry proved are the session's now. A
    /// session no longer proven is kept in `lost`.
    fn owns(&self, lost: &Cell<Option<String>>) -> Result<(), String> {
        self.lease.as_ref().ok_or("not writing")?;
        let writer = self.writer.as_ref().ok_or("not writing")?;
        let seats = writer.console.seats();
        term::still(&writer.seats, seats).inspect_err(|why| lost.set(Some(why.clone())))
    }

    /// A session the admission no longer proved ends the writing at once,
    /// HELD with `line`, the entry it refused, back in its draft: memory only.
    fn unproven(&mut self, lost: Cell<Option<String>>, line: &[u8]) {
        let Some(why) = lost.into_inner() else {
            return;
        };
        let key = self.writer.as_ref().map(Binding::key);
        self.take(Reading::NotOwner(why), Instant::now());
        if let Some(input) = key.and_then(|key| self.inputs.get_mut(&key)) {
            let _ = input.restore(submit::Draft::Kept(line.to_vec()));
        }
    }

    /// One ask of `seat`, through the chat's own admission and tracked path.
    /// A session whose recorded server that path cannot name is no longer
    /// proven: refused in its words before anything is written.
    fn ask(&mut self, raw: &[u8], seat: &str, body: String) -> String {
        let Some(writer) = &self.writer else {
            return "refused: not writing".to_owned();
        };
        let (console, lost) = (&writer.console, Cell::default());
        let owns = || {
            self.owns(&lost)?;
            let name = console.name();
            let routed = crate::tracked::named_server(console.dir(), name, name);
            let routed = routed.map(drop).map_err(|why| why.message());
            routed.inspect_err(|why| lost.set(Some(why.clone())))
        };
        let line = match term::submit_ask(console, owns, raw, seat, body) {
            Ok((_, outcome)) => crate::console::input::outcome_line(&outcome, seat),
            Err(why) => format!("refused: {why}"),
        };
        self.unproven(lost, raw);
        line
    }

    /// `/close`: this app's own open ask, through the chat's admission.
    fn close(&mut self, id: Option<&str>) -> String {
        let Some(writer) = &self.writer else {
            return "refused: not writing".to_owned();
        };
        let (console, lost) = (&writer.console, Cell::default());
        let owns = || self.owns(&lost);
        let line = match submit::close_owned(console.dir(), console.name(), id, owns) {
            Ok(()) => "closed an ask".to_owned(),
            Err(why) => format!("refused: {why}"),
        };
        let entry = id.map_or_else(|| "/close".to_owned(), |id| format!("/close {id}"));
        self.unproven(lost, entry.as_bytes());
        line
    }

    /// The composer line for the selection.
    fn composer(&self) -> draw::Composer<'_> {
        match (self.model.selected(), &self.held) {
            (None, _) => draw::Composer::NoHome,
            (Some(_), Some(why)) => draw::Composer::Held { why },
            (Some(_), None) if !self.read_only.is_empty() && !self.composing() => {
                draw::Composer::ReadOnly {
                    why: &self.read_only,
                }
            }
            (Some(name), None) => draw::Composer::Home {
                home: name,
                speaker: self.speaker(),
                view: self.composing().then_some(&self.draft_view),
                draft: &self.draft,
            },
        }
    }

    /// One frame into `buf`, then the chat scroll bounded by what it held.
    /// Before the fleet is read, the selection's header stands on what its
    /// name alone says.
    fn frame(&mut self, buf: &mut Buffer) {
        if let Some(order) = self.pending.take() {
            self.fleet = std::mem::take(&mut self.fleet).arranged(&order);
        }
        if let Some(name) = self.model.selected() {
            let area = buf.area;
            let address = draw::address(name, self.home.as_deref(), self.speaker());
            let size = Size {
                width: draw::draft_width(area, self.model.split(), &address),
                height: draw::composer_pane(area),
            };
            let shown = self.shown_input().map(|i| (i.bare_view(size), i.draft()));
            (self.draft_view, self.draft) = shown.unwrap_or_default();
        }
        let name = self.model.selected();
        let unread = name
            .filter(|_| !self.fleeted)
            .map(|name| SessionEntry::new(name, Status::Unknown));
        let selected = name.and_then(|name| self.entry(name)).or(unread.as_ref());
        let agents = name.and_then(|name| self.facts.get(name));
        let (look, zone) = self.dressed();
        let screen = draw::Screen {
            fleet: &self.fleet,
            model: &self.model,
            overview: &self.overview,
            selected,
            pair: &self.pair,
            agents,
            lane: &self.lane,
            composer: self.composer(),
            look,
            zone,
            now: Timestamp::now(),
        };
        let wait = draw::Wait {
            fleet: !self.fleeted,
            lane: self.loading,
        };
        self.layout = draw::draw_with_layout(&screen, wait, buf);
        self.model
            .settle(self.layout.list_start, self.layout.body_top);
        // Held keys replay from the scroll they were read at, as one read.
        if self.deferred.is_empty() {
            self.model
                .clamp_scroll(self.layout.max_scroll, self.layout.page_rows);
        }
        if let Some(open) = self.model.settings() {
            self.layout.strip_for_settings();
            draw::paint_settings(
                open.tab,
                open.scroll,
                &self.settings_bodies,
                look.as_ref(),
                buf,
                &mut self.layout,
            );
            self.model
                .set_settings_page_rows(self.layout.settings_page_rows);
            if self.deferred.is_empty() {
                self.model
                    .clamp_settings_scroll(self.layout.settings_max_scroll);
            }
        }
    }
}

/// What the picker's read says of `entry`'s seats.
fn facts_of(
    entry: &SessionEntry,
    picker: Option<&[tmux::PickerSession]>,
    now: Timestamp,
) -> fleet::Facts {
    let Some(rows) = picker else {
        return fleet::Facts::NotPublished;
    };
    let Some(row) = rows.iter().find(|row| row.name == entry.name) else {
        return fleet::Facts::OtherServer;
    };
    match tmux::parse_picker_agents(&row.agents, now.epoch()) {
        Some(agents) => fleet::Facts::Seats {
            id: row.id.clone(),
            agents,
        },
        None => fleet::Facts::NotPublished,
    }
}

/// The pair's names, in the order `term::pair_of` put them: main first.
fn names(seats: &[Seat]) -> Vec<String> {
    seats.iter().map(|seat| seat.name.clone()).collect()
}

/// Move stdin's stamped reads into the app's one wake channel, then say the
/// input closed.
fn forward(reads: Receiver<(Instant, Vec<u8>)>, wake: Sender<Wake>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("ae-app-keys".to_owned())
        .spawn(move || {
            for (stamp, bytes) in reads {
                if wake.send(Wake::Keys(stamp, bytes)).is_err() {
                    return;
                }
            }
            let _ = wake.send(Wake::Closed);
        })?;
    Ok(())
}

/// `ae app [session]`.
///
/// # Errors
///
/// [`crate::Error::Io`] when the terminal or `err` cannot be written.
pub fn run(tail: &[String], out: &mut impl Write, err: &mut impl Write) -> crate::Result<u8> {
    let session = match tail {
        [] => None,
        [name] if !name.starts_with('-') => Some(name.clone()),
        _ => {
            write!(err, "{USAGE}")?;
            return Ok(crate::entry::EXIT_USAGE);
        }
    };
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        write!(err, "{NO_TERMINAL}")?;
        return Ok(crate::EXIT_UNAVAILABLE);
    }
    let Some(root) = crate::state_root() else {
        writeln!(err, "ae: {}", crate::NO_STATE_ROOT)?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let home = session.or_else(crate::calling_session_name);
    if let Some(name) = home.as_deref()
        && console::locate(&root, name).is_none()
    {
        let line = format!("ae app: no session named {name}");
        writeln!(err, "{}", crate::board::terminal_text(&line))?;
        return Ok(crate::EXIT_UNAVAILABLE);
    }
    let Some(reads) = console::term::stdin_reads() else {
        write!(err, "{NO_TERMINAL}")?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let declared = doors::declared_server(crate::shape::current());
    let server = doors::launch_target(declared.as_ref());
    let (wake, wakes) = mpsc::channel();
    let mut reader = loader::Reader::new(root.clone(), home.clone(), server.clone());
    reader.settings_paths(loader::SettingsPaths {
        home: doors::home(),
        global: doors::config_file(crate::shape::current(), &root),
        local: doors::local_config(&doors::cwd()),
    });
    let started = forward(reads, wake.clone()).and_then(|()| loader::spawn(reader, wake));
    let (ask, reader) = match started {
        Ok(started) => started,
        Err(why) => {
            writeln!(err, "ae app: {why}")?;
            return Ok(crate::EXIT_UNAVAILABLE);
        }
    };
    let tty = match tty::Tty::start() {
        Ok(tty) => tty,
        Err(why) => {
            writeln!(err, "ae app: {why}")?;
            return Ok(crate::EXIT_UNAVAILABLE);
        }
    };
    let mut size = tty.size().unwrap_or((80, 24));
    let mut terminal = Terminal::new(backend::Ansi::new(out, size))?;
    let mut app = App::new(home, None, None, Some(ask));
    let mut keys = Keys::app();
    let mut dirty = true;
    loop {
        if let Some(now) = tty.size().filter(|now| *now != size) {
            size = now;
            terminal.backend_mut().resize(size);
            dirty = true;
        }
        if dirty {
            terminal.draw(|frame| {
                app.frame(frame.buffer_mut());
                if let Some(at) = app.layout.cursor {
                    frame.set_cursor_position(at);
                }
            })?;
            dirty = false;
        }
        // Held keys replay as soon as the frame above has produced more.
        let wait = if app.deferred.is_empty() {
            TICK
        } else {
            Duration::ZERO
        };
        let first = match wakes.recv_timeout(wait) {
            Ok(wake) => Some(wake),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let Some(redraw) = drain(&mut app, &mut keys, &wakes, first) else {
            drop(tty);
            return Ok(0);
        };
        dirty |= redraw;
        app.focus();
        if reader.is_finished() {
            drop(tty);
            writeln!(err, "ae app: its background reader stopped")?;
            return Ok(1);
        }
    }
    drop(tty);
    Ok(0)
}

/// Take the held keys, then `first` and every wake already waiting behind
/// it, then the idle keys when none came: `Some(redraw)`, or `None` to quit.
fn drain(
    app: &mut App,
    keys: &mut Keys,
    wakes: &mpsc::Receiver<Wake>,
    first: Option<Wake>,
) -> Option<bool> {
    let held = std::mem::take(&mut app.deferred);
    let (mut redraw, mut fed) = (take_keys(app, held)?, false);
    for wake in first
        .into_iter()
        .chain(std::iter::from_fn(|| wakes.try_recv().ok()))
    {
        let keyed = match wake {
            Wake::Keys(stamp, bytes) => {
                crate::read_gate("@app-keys");
                fed = true;
                keys.feed(&bytes, stamp)
            }
            Wake::Answer(answer) => {
                app.answer(*answer);
                redraw = true;
                continue;
            }
            Wake::Closed => return None,
        };
        redraw |= take_keys(app, keyed)?;
    }
    if !fed {
        redraw |= take_keys(app, keys.idle(Instant::now()))?;
    }
    Some(redraw)
}

/// Take decoded keys: `Some(redraw)`, or `None` to quit. From a wheel notch
/// the frame cannot bound yet on, every key is held in order behind it, and
/// a held `^C` still quits at once.
fn take_keys(app: &mut App, keyed: Vec<(Key, Instant)>) -> Option<bool> {
    let mut redraw = false;
    let mut keyed = keyed.into_iter();
    while let Some((key, origin)) = keyed.next() {
        if !app.deferred.is_empty() || app.unbounded(&key) {
            app.deferred.push((key, origin));
            app.deferred.extend(keyed);
            let quit = app.deferred.iter().any(|(key, _)| *key == Key::Interrupt);
            return (!quit).then_some(true);
        }
        redraw |= if let Key::Mouse(mouse) = &key {
            app.mouse(*mouse)
        } else {
            // A key or a paste ends a drag first, then acts as it always has.
            let ended = app.model.let_go();
            let acted = if app.model.settings_open() {
                settings_key(app, &key, origin)?
            } else if app.composing() {
                app.compose(key, origin).map(|()| true)?
            } else if app.held.is_some() {
                app.retry(&key)?
            } else {
                browse(app, &key, origin)?
            };
            acted || ended
        };
    }
    Some(redraw)
}

/// Take one key: `Some(redraw)`, or `None` to quit. A batched Text key
/// spells in byte order through the current mode: an `s` opens mid-chunk
/// and the rest routes modal, an `i` starts writing and the rest is draft.
fn browse(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    if let Key::Text(bytes) = key {
        let mut redraw = false;
        let mut rest = bytes.as_slice();
        while let Some((&byte, tail)) = rest.split_first() {
            if app.model.settings_open() {
                redraw |= settings_byte(app, byte);
                rest = tail;
                continue;
            }
            if app.held.is_some() {
                // Writing was refused mid-chunk: the rest is HELD, swallowed.
                return Some(true);
            }
            if app.composing() {
                // Writing started mid-chunk: the rest is draft, whole, with
                // its original instant — never re-spelled per byte.
                app.compose(Key::Text(rest.to_vec()), origin)
                    .map(|()| true)?;
                return Some(true);
            }
            redraw |= browse_byte(app, byte)?;
            rest = tail;
        }
        return Some(redraw);
    }
    let mut redraw = false;
    for key in model::browse_keys(key) {
        let act = app.model.key(key, &app.fleet, app.can_compose(), true);
        redraw |= apply(app, &act)?;
    }
    Some(redraw)
}

/// One Text byte while browsing: `s` opens with its request, anything else
/// decodes as it always has. Browsing on entry; the caller routes the modes.
fn browse_byte(app: &mut App, byte: u8) -> Option<bool> {
    if byte == b's' {
        app.open_settings();
        return Some(true);
    }
    let mut redraw = false;
    for key in model::browse_keys(&Key::Text(vec![byte])) {
        let act = app.model.key(key, &app.fleet, app.can_compose(), true);
        redraw |= apply(app, &act)?;
    }
    Some(redraw)
}

/// Take one key with the overlay open: `Some(redraw)`, or `None` to quit.
/// The overlay routes before composing; a close it causes tells the reader.
/// Bytes after a mid-chunk close resume the actual mode: the suspended
/// composer when writing, browse otherwise.
fn settings_key(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    if *key == Key::Interrupt {
        return None;
    }
    if let Key::Text(bytes) = key {
        let mut redraw = false;
        let mut rest = bytes.as_slice();
        while let Some((&byte, tail)) = rest.split_first() {
            if !app.model.settings_open() {
                if app.composing() {
                    // The rest of the run goes back to the draft whole, with
                    // its original instant — never re-spelled per byte.
                    app.compose(Key::Text(rest.to_vec()), origin)
                        .map(|()| true)?;
                    return Some(true);
                }
                redraw |= browse_byte(app, byte)?;
                rest = tail;
                continue;
            }
            redraw |= settings_byte(app, byte);
            rest = tail;
        }
        return Some(redraw);
    }
    let mut redraw = false;
    for key in model::browse_keys(key) {
        // The overlay branch returns no Quit, Select or Compose; a close
        // shows as the state it leaves behind, told below.
        if app.model.key(key, &app.fleet, app.can_compose(), true) == model::Act::Redraw {
            redraw = true;
        }
    }
    if !app.model.settings_open() {
        app.close_settings();
        redraw = true;
    }
    Some(redraw)
}

/// One Text byte with the overlay open: `s` closes with its request, the
/// rest decodes modal. The overlay is open on entry; the caller resumes the
/// actual mode after a close.
fn settings_byte(app: &mut App, byte: u8) -> bool {
    if byte == b's' {
        app.close_settings();
        return true;
    }
    let mut redraw = false;
    for key in model::browse_keys(&Key::Text(vec![byte])) {
        if app.model.key(key, &app.fleet, app.can_compose(), true) == model::Act::Redraw {
            redraw = true;
        }
    }
    if !app.model.settings_open() {
        app.close_settings();
        redraw = true;
    }
    redraw
}

/// Carry out a browse action from either keys or a frame's mouse target.
fn apply(app: &mut App, act: &model::Act) -> Option<bool> {
    match act {
        model::Act::Quit => None,
        model::Act::Select(_) => {
            app.show();
            Some(true)
        }
        model::Act::Redraw => Some(true),
        model::Act::Compose => {
            app.write(true);
            Some(true)
        }
        model::Act::None => Some(false),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::Rect;

    use super::loader::{self, Answer, Reader, ViewRead};
    use super::{
        App, USAGE, browse, draw, facts_of, fleet, run, settings, settings_key, take_keys,
    };
    use crate::app::fleet::{Counts, Fleet, Row};
    use crate::app::model::{Edge, Key as Browse, Model, SettingsTab, Tab};
    use crate::attention::Reason;
    use crate::console::input::{Effect, Input, Key, Reading};
    use crate::console::lane::{Item, Kind, Lane};
    use crate::digest::{SessionEntry, Status};
    use crate::listing::World;
    use crate::theme::Mark;
    use crate::time::Timestamp;
    use crate::tmux;

    pub(in crate::app) const ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";

    /// A state root holding one lead-pair session, `api`, removed on drop.
    pub(in crate::app) struct Root(pub(in crate::app) PathBuf);
    impl Root {
        pub(in crate::app) fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("ae-app-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let dir = root.join("sessions").join("api");
            std::fs::create_dir_all(&dir).expect("session dir");
            let meta = format!(
                "schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n"
            );
            std::fs::write(dir.join("meta"), meta).expect("meta");
            Self(root)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn one_row(home: Option<&str>) -> Fleet {
        Fleet {
            rows: vec![row("api", 1, home.is_some())],
            home: home.map(str::to_owned),
        }
    }

    fn row(name: &str, index: usize, home: bool) -> Row {
        Row {
            name: name.to_owned(),
            index,
            mark: Mark::Working,
            needy: false,
            counts: Counts::Unknown,
            line2: fleet::Line2::NoGoal,
            home,
        }
    }

    fn said(body: &str, at: i64) -> Item {
        Item {
            micros: 1_759_500_000_000_000 + at * 1_000_000,
            kind: Kind::Said {
                who: "lead".to_owned(),
            },
            body: body.to_owned(),
            record: None,
        }
    }

    fn framed(app: &mut App) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
        app.frame(&mut buf);
        text(&buf)
    }

    /// An app driven by hand, as if its first fleet read had landed with no
    /// lane to wait for.
    fn app(home: Option<&str>) -> App {
        let mut app = App::new(home.map(str::to_owned), None, None, None);
        (app.fleeted, app.loading) = (true, false);
        app
    }

    /// [`app`] on `root`'s home `api`, as its first fleet read finds it.
    fn housed(root: &Root) -> (App, Reader) {
        let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
        let mut app = app(Some("api"));
        refold(&mut app, &reader, root, &[("api", Status::Running)]);
        (app, reader)
    }

    /// `root`'s sessions `live` with their statuses, as `reader` folds them.
    fn refold(app: &mut App, reader: &Reader, root: &Root, live: &[(&str, Status)]) {
        let dir = |name: &str| (name.to_owned(), root.0.join("sessions").join(name));
        let dirs = live.iter().map(|(name, _)| dir(name)).collect();
        let world = live.iter().map(|(name, status)| entry(name, *status, None));
        let world = World::new(Timestamp::now(), world.collect());
        let order = crate::theme::FleetOrder::EMPTY;
        let read = reader.fold(dirs, world, None, &order, Timestamp::now());
        app.answer(Answer::Fleet(read));
    }

    /// The draft the composer shows.
    fn drafted(app: &App) -> String {
        app.shown_input().map(Input::draft).unwrap_or_default()
    }

    /// `root`'s home `api` recorded as incarnation `id`, lead pair lead + `peer`.
    pub(in crate::app) fn meta(root: &Root, id: &str, peer: &str) {
        let meta =
            format!("session_id={id}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0={peer}\n");
        let dir = root.0.join("sessions").join("api");
        std::fs::write(dir.join("meta"), format!("schema=2\n{meta}")).expect("meta");
    }

    /// [`housed`]'s `app` writing home under its lease: the instant after
    /// it was taken.
    fn writing(app: &mut App) -> Instant {
        app.take(Reading::Owner, Instant::now());
        app.write(true);
        assert!(app.composing(), "the lease is taken");
        Instant::now()
    }

    /// A lane of one turn saying `body`, read as `id` at `seq`.
    fn view(name: &str, id: &str, seq: u64, body: &str) -> Answer {
        Answer::View(ViewRead {
            name: name.to_owned(),
            id: id.to_owned(),
            seq,
            lane: Lane {
                items: vec![said(body, 0)],
                coverage: Vec::new(),
            },
            needs: None,
            roster: None,
        })
    }

    /// Two foreign sessions, `web` and `ops`, read by the fleet as `ID`.
    fn two(app: &mut App) {
        app.fleet = Fleet {
            rows: vec![row("web", 1, false), row("ops", 2, false)],
            home: None,
        };
        for name in ["web", "ops"] {
            app.dirs.insert(name.to_owned(), PathBuf::from(name));
            app.ids.insert(name.to_owned(), ID.to_owned());
        }
        app.world = World::new(
            Timestamp::now(),
            vec![
                entry("web", Status::Running, None),
                entry("ops", Status::Running, None),
            ],
        );
        app.model = Model::new(&app.fleet);
    }

    /// A click belongs to the drawn session, never a replacement at its index.
    #[test]
    fn a_departed_mouse_target_never_selects_its_replacement() {
        let mut app = app(None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        let shown = framed(&mut app);
        let (row, column) = shown
            .lines()
            .enumerate()
            .find_map(|(row, line)| line.find("api").map(|column| (row, column)))
            .expect("drawn api session");
        app.fleet.rows[0].name = "web".to_owned();
        let redraw = app.mouse(super::Mouse {
            kind: super::MouseKind::Click,
            column: u16::try_from(column).expect("frame column"),
            row: u16::try_from(row).expect("frame row"),
        });
        assert!(!redraw, "a departed target is a full no-op");
        assert_eq!(
            app.model.selected(),
            Some("api"),
            "the click never selected web"
        );
    }

    /// Motion on a dragged list rule the frame stopped drawing moves nothing,
    /// though the sidebar beside it is still drawn; shown again, the rule
    /// stands where it was last dragged.
    #[test]
    fn motion_on_a_list_rule_the_frame_hid_keeps_its_size() {
        let rule_row = |shown: &str| shown.lines().position(|line| line.starts_with('─'));
        let mut app = app(None);
        two(&mut app);
        let first = framed(&mut app);
        let row = u16::try_from(rule_row(&first).expect("list rule")).expect("frame row");
        let at = |kind, row| super::Mouse {
            kind,
            column: 2,
            row,
        };
        assert!(
            app.mouse(at(super::MouseKind::Click, row)),
            "a press grabs the rule"
        );
        assert!(
            app.mouse(at(super::MouseKind::Drag, row + 3)),
            "motion drags it"
        );
        let chosen = app.model.split();
        let dragged = rule_row(&framed(&mut app));
        assert_eq!(dragged, Some(usize::from(row + 3)), "the rule followed");
        let world = app.world.clone();
        app.world = World::new(Timestamp::now(), Vec::new());
        let hidden = framed(&mut app);
        assert_eq!(
            rule_row(&hidden),
            None,
            "no selected entry draws no list rule"
        );
        let sidebar = |shown: &str| shown.lines().next().and_then(|top| top.find('│'));
        assert!(sidebar(&first).is_some(), "a sidebar rule to keep");
        assert_eq!(
            sidebar(&hidden),
            sidebar(&first),
            "the sidebar rule still stands"
        );
        assert!(
            !app.mouse(at(super::MouseKind::Drag, row + 8)),
            "motion on the hidden rule is a no-op"
        );
        assert!(app.model.drag().is_some(), "the drag is still held");
        assert_eq!(app.model.split(), chosen, "the hidden rule kept its size");
        app.world = world;
        assert_eq!(
            rule_row(&framed(&mut app)),
            dragged,
            "shown again where it was dragged"
        );
    }

    /// A fleet read that empties the fleet, then one that fills it again,
    /// keeps both chosen sizes and the drag still held: a read is not input.
    /// The refilled fleet otherwise starts fresh, on its first row's Overview.
    #[test]
    fn an_emptied_then_refilled_fleet_keeps_its_sizes_and_drag() {
        let rule_row = |shown: &str| shown.lines().position(|line| line.starts_with('─'));
        let sidebar = |shown: &str| {
            (shown.lines().next()).and_then(|top| top.chars().position(|cell| cell == '│'))
        };
        let mouse = |kind, column: usize, row: usize| super::Mouse {
            kind,
            column: u16::try_from(column).expect("frame column"),
            row: u16::try_from(row).expect("frame row"),
        };
        let mut app = app(None);
        two(&mut app);
        let full = read_of(&app, &[("web", ID), ("ops", ID)]);
        let mut empty = read_of(&app, &[]);
        (empty.fleet, empty.world) = (Fleet::default(), World::new(Timestamp::now(), Vec::new()));
        let first = framed(&mut app);
        let column = sidebar(&first).expect("sidebar rule");
        assert!(app.mouse(mouse(super::MouseKind::Click, column, 1)));
        assert!(app.mouse(mouse(super::MouseKind::Drag, column + 6, 1)));
        assert!(app.mouse(mouse(super::MouseKind::Release, column + 6, 1)));
        let widened = framed(&mut app);
        let row = rule_row(&widened).expect("list rule");
        assert!(app.mouse(mouse(super::MouseKind::Click, 2, row)));
        assert!(app.mouse(mouse(super::MouseKind::Drag, 2, row + 3)));
        app.model.show_tab(Tab::Agents);
        app.answer(Answer::Fleet(empty));
        assert_eq!(
            app.model.selected(),
            None,
            "the empty fleet selects nothing"
        );
        app.answer(Answer::Fleet(full));
        let refilled = framed(&mut app);
        assert_eq!(
            sidebar(&refilled),
            Some(column + 6),
            "the sidebar kept its width"
        );
        assert_eq!(rule_row(&refilled), Some(row + 3), "the list kept its rows");
        assert_eq!(
            app.model.selected(),
            Some("web"),
            "the first row is selected"
        );
        assert_eq!(app.model.tab(), Tab::Overview, "on a fresh Overview");
        assert!(app.model.drag().is_some(), "the drag is still held");
        assert!(
            app.mouse(mouse(super::MouseKind::Drag, 2, row + 4)),
            "and still drags"
        );
        assert_eq!(rule_row(&framed(&mut app)), Some(row + 4));
    }

    /// Leaving writing on the current tab must repaint immediately, keeping the draft.
    #[test]
    fn a_same_tab_mouse_click_ends_writing_and_requests_redraw() {
        let root = Root::new("mouse-same-tab");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.world = World::new(Timestamp::now(), vec![entry("api", Status::Running, None)]);
        let at = writing(&mut app);
        assert_eq!(app.compose(Key::Text(b"kept draft".to_vec()), at), Some(()));
        let shown = framed(&mut app);
        let (row, column) = shown
            .lines()
            .enumerate()
            .find_map(|(row, line)| {
                line.find("Overview")
                    .map(|byte| (row, line[..byte].chars().count()))
            })
            .expect("drawn Overview tab");
        assert!(
            app.mouse(super::Mouse {
                kind: super::MouseKind::Click,
                column: u16::try_from(column).expect("frame column"),
                row: u16::try_from(row).expect("frame row"),
            }),
            "the mode change needs an immediate redraw"
        );
        assert!(!app.composing());
        assert_eq!(app.model.tab(), crate::app::model::Tab::Overview);
        assert_eq!(drafted(&app), "kept draft");
    }

    /// The bottom keys row is outside chat, while the composer just above still scrolls.
    #[test]
    fn mouse_wheel_on_the_keys_row_is_a_full_noop() {
        let mut app = app(None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        app.lane = Rc::new(Lane {
            items: (0..60).map(|at| said(&format!("turn {at}"), at)).collect(),
            coverage: Vec::new(),
        });
        let shown = framed(&mut app);
        assert!(shown.lines().last().expect("keys row").contains("q quit"));
        let mut mouse = super::Mouse {
            kind: super::MouseKind::WheelUp,
            column: 159,
            row: 44,
        };
        assert!(!app.mouse(mouse), "keys-row wheel is a full no-op");
        assert_eq!(app.model.scroll_rows(app.layout.page_rows), 0);
        mouse.row -= 1;
        assert!(app.mouse(mouse), "the composer is inside chat");
        assert_eq!(app.model.scroll_rows(app.layout.page_rows), 3);
        mouse.row += 1;
        mouse.kind = super::MouseKind::WheelDown;
        assert!(!app.mouse(mouse), "keys-row wheel down is also a no-op");
        assert_eq!(app.model.scroll_rows(app.layout.page_rows), 3);
    }

    /// The whole drawn list takes the wheel; a notch past its end is a no-op.
    #[test]
    fn a_list_notch_over_its_last_card_scrolls_and_none_scrolls_past_the_end() {
        let mut app = app(None);
        let rows = (0..30).map(|at| row(&format!("n{at:02}"), at + 1, false));
        app.fleet = one_row(None);
        app.fleet.rows = rows.collect();
        app.model = Model::new(&app.fleet);
        let cards = |shown: &str| -> Vec<(usize, u8)> {
            let name = |w: &str| w.strip_prefix('n')?.parse().ok();
            let card = |(y, l): (usize, &str)| Some((y, l.split_whitespace().find_map(name)?));
            shown.lines().enumerate().skip(5).filter_map(card).collect()
        };
        let first = cards(&framed(&mut app));
        let row = u16::try_from(first.last().expect("drawn cards").0).expect("row");
        let notch = super::Mouse {
            kind: super::MouseKind::WheelDown,
            column: 2,
            row,
        };
        assert!(app.mouse(notch), "the last drawn card scrolls");
        assert_eq!(cards(&framed(&mut app))[0].1, 1, "by one session");
        for _ in first.len()..29 {
            assert!(app.mouse(notch), "one notch per hidden session");
            framed(&mut app);
        }
        let end = framed(&mut app);
        assert_eq!(cards(&end).last().map(|card| card.1), Some(29));
        assert!(!app.mouse(notch), "past the end is a no-op");
        assert_eq!(framed(&mut app), end, "and draws nothing new");
    }

    /// Every drawn tab label includes its last cell and excludes the next blank cell.
    #[test]
    fn mouse_tab_targets_end_at_the_last_drawn_label_cell() {
        let mut app = app(None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        app.world = World::new(Timestamp::now(), vec![entry("api", Status::Running, None)]);
        app.facts.insert(
            "api".to_owned(),
            fleet::Facts::Seats {
                id: "$1".to_owned(),
                agents: Vec::new(),
            },
        );
        let shown = framed(&mut app);
        for (label, tab) in [
            ("Overview", crate::app::model::Tab::Overview),
            ("Agents 0", crate::app::model::Tab::Agents),
        ] {
            let (row, column) = shown
                .lines()
                .enumerate()
                .find_map(|(row, line)| {
                    line.find(label)
                        .map(|byte| (row, line[..byte].chars().count()))
                })
                .expect("drawn tab label");
            let mut mouse = super::Mouse {
                kind: super::MouseKind::Click,
                column: u16::try_from(column + label.chars().count() - 1).expect("last label cell"),
                row: u16::try_from(row).expect("frame row"),
            };
            assert!(
                matches!(app.layout.hit(mouse), Some(super::draw::Hit::Tab(hit)) if hit == tab),
                "last cell of {label}"
            );
            mouse.column += 1;
            assert!(app.layout.hit(mouse).is_none(), "one cell past {label}");
        }
    }

    /// B2-F4: a frame bounds the scroll to the pages the lane holds — the
    /// bound is the FIRST page count that shows the oldest turn, so one page
    /// fewer hides it — and a lane that fits scrolls nowhere.
    #[test]
    fn a_frame_bounds_the_scroll_to_the_oldest_turn() {
        let mut app = app(None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        let mut items = vec![said("oldest turn", 0)];
        items.extend((1..60).map(|at| said(&format!("turn {at}"), at)));
        app.lane = Rc::new(Lane {
            items,
            coverage: Vec::new(),
        });
        for _ in 0..80 {
            let _ = app.model.key(Browse::PageUp, &app.fleet, false, true);
        }
        let shown = framed(&mut app);
        let bound = app.model.pages();
        assert!(bound > 0 && bound < 80, "clamped to the lane: {bound}");
        assert!(shown.contains("oldest turn"), "the bound shows the oldest");
        let _ = app.model.key(Browse::PageDown, &app.fleet, false, true);
        assert!(
            !framed(&mut app).contains("oldest turn"),
            "one fewer hides it"
        );
        assert_eq!(app.model.pages(), bound - 1, "a scroll inside stays put");
        Rc::make_mut(&mut app.lane).items.truncate(2);
        let _ = framed(&mut app);
        assert_eq!(app.model.pages(), 0, "a lane that fits scrolls nowhere");
    }

    /// An ask without the lease, or after home was replaced under its name,
    /// is refused BEFORE any ask, kept draft or body is written and says so
    /// in the home lane; the unproven home ends the writing (R-B5/R-B7).
    #[test]
    fn an_unproven_ask_is_refused_and_writes_nothing() {
        let root = Root::new("ask");
        let dir = root.0.join("sessions").join("api");
        let (mut app, _reader) = housed(&root);
        app.take(Reading::Owner, Instant::now());
        let refused = app.ask(b"x", "lead", "x".to_owned());
        assert_eq!(refused, "refused: not writing");
        let typed = writing(&mut app);
        meta(&root, &ID.replace("1234", "bbbb"), "colead");
        for key in [Key::Text(b"check the scopes".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, typed), Some(()));
        }
        let notice = &app.notices.last().expect("the outcome is said").1.body;
        assert_eq!(notice, "refused: the session was replaced or renamed");
        let written = crate::store::open(&dir).events_source();
        assert!(
            matches!(written, crate::store::SourceRead::Absent),
            "no record"
        );
        assert_eq!(super::submit::restore(&dir), super::submit::Draft::Nothing);
        let body = std::fs::remove_dir(dir.join("messages")).map_err(|why| why.kind());
        assert_eq!(body, Err(std::io::ErrorKind::NotFound), "no body written");
        assert!(!app.composing(), "and no writing");
        assert_eq!(
            app.held.as_deref(),
            Some("the session was replaced or renamed")
        );
    }

    /// `/open` in the app names why it opens nothing, as the docs say, once
    /// the home roster is read: the app selects no pane, `ae chat` does.
    #[test]
    fn open_is_refused_with_the_apps_own_reason() {
        let root = Root::new("open");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        let typed = writing(&mut app);
        let lead = crate::console::needs::SeatRef {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
        };
        if let Answer::View(mut read) = view("api", ID, 1, "") {
            read.roster = Some(vec![lead]);
            app.answer(Answer::View(read));
        }
        for key in [Key::Text(b"/open lead".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, typed), Some(()));
        }
        let notice = &app.notices.last().expect("the refusal is said").1.body;
        assert_eq!(
            notice,
            "refused: /open lead: ae app selects no pane - ae chat does"
        );
    }

    // ---- mutation pins (pins-plan.md). Oracles: the fixture root's own
    // ---- records, docs/app.md, docs/chat.md and the existing owners.

    /// A second lead-pair session `name` under `root`, as `Root::new` writes `api`.
    pub(in crate::app) fn session(root: &Root, name: &str, meta_tail: &str) -> PathBuf {
        let dir = root.0.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("session dir");
        let meta = format!(
            "schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n{meta_tail}"
        );
        std::fs::write(dir.join("meta"), meta).expect("meta");
        dir
    }

    pub(in crate::app) fn entry(
        name: &str,
        status: Status,
        attention: Option<Reason>,
    ) -> SessionEntry {
        let mut entry = SessionEntry::new(name, status);
        entry.attention = attention;
        entry
    }

    /// R-B6: only an acquisition restores the kept line, and only into an
    /// empty composer, with no merge; a demotion ends the writing and frees
    /// its lease, and a refused entry is HELD, naming the writer.
    #[test]
    fn only_an_acquisition_into_an_empty_composer_restores_the_kept_line() {
        let root = Root::new("take");
        let store = crate::store::open(&root.0.join("sessions").join("api"));
        store.publish_console_draft(b"kept").expect("a kept draft");
        let (mut app, _reader) = housed(&root);
        app.take(Reading::Owner, Instant::now());
        assert_eq!(drafted(&app), "", "a promotion restores nothing");
        let at = writing(&mut app);
        assert_eq!(app.compose(Key::Text(b" edited".to_vec()), at), Some(()));
        app.write(false);
        writing(&mut app);
        assert_eq!(drafted(&app), "kept edited", "restored once, no merge");
        app.take(Reading::NotOwner("gone".to_owned()), Instant::now());
        let _other = store.console_writer().expect("a demotion frees the lease");
        app.take(Reading::Owner, Instant::now());
        app.write(true);
        assert_eq!(app.held.as_deref(), Some("an ae app is writing to api"));
    }

    /// R-B2/R-B3: the lease's own instant gates the keys: one read before it
    /// is dropped and named once, one read at that instant is draft.
    #[test]
    fn keys_read_before_the_lease_are_dropped_and_named_once() {
        let root = Root::new("late");
        let (mut app, _reader) = housed(&root);
        let before = Instant::now();
        writing(&mut app);
        let taken = app.lease.as_ref().map(|lease| lease.at).expect("held");
        for key in [Key::Text(b"early".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, before), Some(()));
        }
        assert_eq!(app.compose(Key::Text(b"late".to_vec()), taken), Some(()));
        let draft = drafted(&app);
        assert_eq!(draft, "late", "only keys after the lease: {draft}");
        let named = |item: &Item| item.body.contains("keys typed before writing started");
        assert_eq!(app.notices.iter().filter(|(_, i)| named(i)).count(), 1);
    }

    /// Ruling appuse-b R-B2: only keys read before writing started are
    /// dropped. A press on the composer while writing starts nothing new: the
    /// same lease stands, and a key read after it was taken is drafted.
    #[test]
    fn a_composer_press_while_writing_keeps_its_lease() {
        let root = Root::new("composer-press");
        let (mut app, _reader) = strip_app(&root);
        let typed = writing(&mut app);
        let taken = app.lease.as_ref().map(|it| it.at);
        let _ = framed(&mut app);
        let press = (0..45)
            .flat_map(|row| (0..160).map(move |column| (column, row)))
            .map(|(column, row)| super::Mouse {
                kind: super::MouseKind::Click,
                column,
                row,
            })
            .find(|press| matches!(app.layout.hit(*press), Some(super::draw::Hit::Compose)))
            .expect("drawn composer");
        let _ = app.click(press);
        assert_eq!(app.lease.as_ref().map(|it| it.at), taken, "same lease");
        assert!(take_keys(&mut app, vec![(Key::Text(b"k".to_vec()), typed)]).is_some());
        assert_eq!(drafted(&app), "k");
    }

    /// R-C1.3/R-C1.5: the three drawn draft rows and hint are Compose;
    /// after Esc the former upper draft row is lane, not a ghost target.
    #[test]
    fn every_grown_composer_row_and_hint_are_targets_until_browse_compacts_it() {
        let root = Root::new("c1-targets");
        let (mut app, _reader) = strip_app(&root);
        let at = writing(&mut app);
        let _ = app.compose(Key::Pasted(b"first\nsecond\nthird".to_vec()), at);
        let shown = framed(&mut app);
        assert!(
            shown
                .lines()
                .nth(39)
                .expect("first draft row")
                .contains("first")
        );
        let press = |row| super::Mouse {
            kind: super::MouseKind::Click,
            column: 70,
            row,
        };
        let taken = app.lease.as_ref().map(|lease| lease.at);
        for row in 39..=42 {
            assert!(
                matches!(app.layout.hit(press(row)), Some(super::draw::Hit::Compose)),
                "grown row or hint {row}"
            );
            let _ = app.click(press(row));
            assert_eq!(app.lease.as_ref().map(|lease| lease.at), taken);
            assert_eq!(
                app.inputs.values().next().expect("input").draft(),
                "first second third"
            );
        }
        let _ = app.compose(Key::Escape, Instant::now());
        let shown = framed(&mut app);
        assert!(
            shown
                .lines()
                .nth(41)
                .expect("compact composer")
                .contains("to api")
        );
        assert!(!matches!(
            app.layout.hit(press(39)),
            Some(super::draw::Hit::Compose)
        ));
        let _ = app.click(press(39));
        assert!(!app.composing(), "old upper row cannot restart writing");
        for row in 41..=42 {
            assert!(matches!(
                app.layout.hit(press(row)),
                Some(super::draw::Hit::Compose)
            ));
        }
    }

    /// #23/#25: an entry is found by its own name, and a selected running
    /// session draws its tabs (frame calm r18).
    #[test]
    fn an_entry_is_found_by_its_own_name() {
        let mut app = app(None);
        app.world = World::new(
            Timestamp::now(),
            vec![
                entry("api", Status::Running, None),
                entry("web", Status::Stopped, None),
            ],
        );
        assert_eq!(app.entry("web").map(|e| e.name.as_str()), Some("web"));
        assert_eq!(app.entry("api").map(|e| e.name.as_str()), Some("api"));
        assert!(app.entry("zzz").is_none());
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        assert!(framed(&mut app).contains("Overview   Agents"));
    }

    /// D3 (docs/app.md Ownership): a selection is read-only and says why
    /// before the fleet is read, while stopped, while its pair is unread or
    /// unreadable, and while its pair differs from the one it was written with.
    #[test]
    fn an_unwritable_selection_is_read_only_and_says_why() {
        let root = Root::new("readonly");
        let (mut app, _reader) = housed(&root);
        let why = |app: &App| app.writable("api").err().unwrap_or_default();
        assert_eq!(why(&app), "", "GUARD a running read session is writable");
        app.world = World::new(Timestamp::now(), vec![entry("api", Status::Stopped, None)]);
        app.show();
        let text = framed(&mut app);
        assert!(text.contains("read-only · api is stopped"), "{text}");
        assert!(!text.contains("to api ›"), "{text}");
        app.world = World::new(Timestamp::now(), vec![entry("api", Status::Running, None)]);
        let unreadable = Err("the meta names no single main seat".to_owned());
        let read = app.pairs.insert("api".to_owned(), unreadable);
        assert_eq!(why(&app), "the meta names no single main seat");
        app.pairs.clear();
        assert_eq!(why(&app), "the lead pair is not read yet");
        app.pairs.insert("api".to_owned(), read.expect("pair"));
        app.bound
            .insert(("api".to_owned(), ID.to_owned()), Vec::new());
        assert!(why(&app).contains("restart"), "{}", why(&app));
        app.fleeted = false;
        assert_eq!(why(&app), "the fleet is not read yet");
    }

    /// B2: a session that is not home is written under its own lease and
    /// input, after an address saying so; only a read proving its stop ends it.
    #[test]
    fn a_selection_that_is_not_home_is_written_under_its_own_lease() {
        let root = Root::new("selected");
        let (mut app, reader) = housed(&root);
        let web = session(&root, "web", "");
        let api = root.0.join("sessions").join("api");
        let fleet = |web| [("api", Status::Running), ("web", web)];
        refold(&mut app, &reader, &root, &fleet(Status::Running));
        let _ = app.model.select_name(&app.fleet, "web", 0);
        app.show();
        assert!(framed(&mut app).contains("to web (not home) › lead   "));
        let at = writing(&mut app);
        assert_eq!(app.compose(Key::Text(b"w".to_vec()), at), Some(()));
        assert!(crate::store::open(&web).console_writer().is_err());
        drop(crate::store::open(&api).console_writer().expect("api free"));
        let keys: Vec<_> = app.inputs.keys().collect();
        assert_eq!(keys, [&("web".to_owned(), ID.to_owned())]);
        let mut read = read_of(&app, &[("api", ID)]);
        (read.scanned, read.fleet) = (false, one_row(Some("api")));
        app.answer(Answer::Fleet(read));
        let typed = vec![(Key::Text(b"q".to_vec()), at)];
        assert_eq!(take_keys(&mut app, typed), Some(true), "never Quit");
        assert!(app.composing(), "an incomplete read keeps the lease");
        refold(&mut app, &reader, &root, &fleet(Status::Unknown));
        assert!(app.composing(), "an Unknown read revokes nothing");
        refold(&mut app, &reader, &root, &fleet(Status::Stopped));
        assert_eq!(app.held.as_deref(), Some("web is stopped"));
        let q = vec![(Key::Text(b"q".to_vec()), at)];
        assert_eq!(take_keys(&mut app, q), Some(false), "q stays swallowed");
        drop(crate::store::open(&web).console_writer().expect("released"));
        assert_eq!(drafted(&app), "wq", "its draft kept");
        let _ = take_keys(&mut app, vec![(Key::Escape, at), (Key::Enter, at)]);
        assert_eq!(app.held, None, "read-only Enter browses (D3)");
    }

    /// R-B4 ext (SELECTION-MOVED): a complete read that no longer lists the
    /// writing target, or proves it gone, ends the writing HELD, never browse.
    #[test]
    fn a_complete_read_without_the_writing_target_ends_it_held() {
        let root = Root::new("unlisted");
        let (mut app, reader) = housed(&root);
        session(&root, "web", "");
        let both = [("api", Status::Running), ("web", Status::Running)];
        refold(&mut app, &reader, &root, &both);
        let _ = app.model.select_name(&app.fleet, "web", 0);
        writing(&mut app);
        let mut read = read_of(&app, &[("api", ID), ("web", ID)]);
        read.fleet.rows.retain(|row| row.name == "api");
        app.answer(Answer::Fleet(read));
        assert_eq!(app.held.as_deref(), Some("web is no longer listed"));
        refold(&mut app, &reader, &root, &both);
        let _ = app.model.select_name(&app.fleet, "web", 0);
        writing(&mut app);
        refold(&mut app, &reader, &root, &both[..1]);
        assert_eq!(app.held.as_deref(), Some("the session is gone"));
    }

    /// R-B4 ext (G2): a refused entry, like a replaced writer, is HELD with each read's reason.
    #[test]
    fn a_refused_entry_is_held_with_the_reason_each_read_gives() {
        let root = Root::new("held-read");
        let (mut app, reader) = housed(&root);
        let dir = root.0.join("sessions").join("api");
        let busy = crate::store::open(&dir).console_writer();
        app.write(true);
        assert_eq!(app.held.as_deref(), Some("an ae app is writing to api"));
        refold(&mut app, &reader, &root, &[("api", Status::Stopped)]);
        assert_eq!(app.held.as_deref(), Some("api is stopped"));
        meta(&root, &ID.replace("1234", "bbbb"), "colead");
        refold(&mut app, &reader, &root, &[("api", Status::Running)]);
        assert_eq!(app.held.as_deref(), Some("the session was replaced"));
        assert!(
            app.lease.is_none() && app.writer.is_none(),
            "never acquires"
        );
        drop(busy);
        writing(&mut app);
        meta(&root, &ID.replace("1234", "cccc"), "colead");
        refold(&mut app, &reader, &root, &[("api", Status::Running)]);
        assert_eq!(app.held.as_deref(), Some("the session was replaced"));
    }

    /// #27/#28: `/close` answers with the chat admission's own answer: a
    /// proven home with no tmux reaches the journal; a changed pair is
    /// refused before it and ends the writing; no home session, no home.
    #[test]
    fn close_reports_the_chats_own_admission_answer() {
        let root = Root::new("close");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.dirs
            .insert("api".to_owned(), root.0.join("sessions").join("api"));
        let typed = writing(&mut app);
        let close = |app: &mut App| {
            for key in [Key::Text(b"/close".to_vec()), Key::Enter] {
                assert_eq!(app.compose(key, typed), Some(()));
            }
            app.notices
                .last()
                .map_or_else(String::new, |said| said.1.body.clone())
        };
        assert_eq!(close(&mut app), "refused: no open ask to close");
        meta(&root, ID, "peer");
        let changed = "the lead pair changed since this chat opened - restart this chat";
        assert_eq!(close(&mut app), format!("refused: {changed}"));
        assert!(!app.composing(), "an unproven home ends the writing");
        assert_eq!(app.close(None), "refused: not writing");
    }

    /// #36: seat facts come from the session's own picker row.
    #[test]
    fn seat_facts_come_from_the_sessions_own_picker_row() {
        let now = Timestamp::from_epoch(1_880);
        let picker = |name: &str, id: &str| tmux::PickerSession {
            name: name.to_owned(),
            id: id.to_owned(),
            rank: 0,
            glyph: String::new(),
            main_pane: String::new(),
            branch: String::new(),
            agents: "v1;1880;60;lead:fable5:working:".to_owned(),
            activity: None,
            created_at: None,
            goal: String::new(),
        };
        let rows = [picker("web", "$7"), picker("api", "$3")];
        let api = entry("api", Status::Running, None);
        assert!(
            matches!(facts_of(&api, Some(&rows), now), fleet::Facts::Seats { id, .. } if id == "$3")
        );
        assert!(matches!(
            facts_of(&api, None, now),
            fleet::Facts::NotPublished
        ));
        assert!(matches!(
            facts_of(&api, Some(&rows[..1]), now),
            fleet::Facts::OtherServer
        ));
    }

    /// #40: a dash is never a session name (grammar), so it is a usage error.
    #[test]
    fn a_dash_argument_is_a_usage_error_not_a_session() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&["-x".to_owned()], &mut out, &mut err).expect("writes");
        assert_eq!(code, crate::entry::EXIT_USAGE);
        assert_eq!(String::from_utf8_lossy(&err), USAGE);
    }

    /// #50/#56/#57: browsing, only `q` quits (docs/app.md BROWSE keys).
    #[test]
    fn only_q_quits_while_browsing() {
        let mut app = app(None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        assert!(browse(&mut app, &Key::Text(b"x".to_vec()), Instant::now()).is_some());
        assert!(browse(&mut app, &Key::Text(b"q".to_vec()), Instant::now()).is_none());
    }

    // ---- appsnappy: what the reader answered, and what is drawn of it.

    /// A lane read for one session while another is selected changes only
    /// what that session will show; the drawn lane stays the selection's.
    #[test]
    fn a_lane_for_another_session_never_changes_the_drawn_one() {
        let mut app = app(None);
        two(&mut app);
        app.answer(view("web", ID, 1, "web turn"));
        app.answer(view("ops", ID, 2, "ops turn"));
        let shown = framed(&mut app);
        assert!(shown.contains("web turn"), "{shown}");
        assert!(!shown.contains("ops turn"), "{shown}");
        let _ = app.model.key(Browse::Digit(2), &app.fleet, false, true);
        app.show();
        let shown = framed(&mut app);
        assert!(shown.contains("ops turn"), "the cached lane shows at once");
        assert!(!shown.contains("web turn"), "{shown}");
    }

    /// A never-read selection draws its header and ONE `loading` row,
    /// never the lane of the session selected before it.
    #[test]
    fn a_cold_selection_says_loading_never_the_last_lane() {
        let mut app = app(None);
        two(&mut app);
        app.answer(view("web", ID, 1, "web turn"));
        let _ = app.model.key(Browse::Digit(2), &app.fleet, false, true);
        app.show();
        let shown = framed(&mut app);
        assert!(!shown.contains("web turn"), "{shown}");
        assert_eq!(shown.matches("loading").count(), 1, "{shown}");
        let header = shown.lines().nth(1).expect("header row");
        assert!(header.contains("ops"), "{header}");
        app.answer(view("ops", ID, 2, "ops turn"));
        let shown = framed(&mut app);
        assert!(shown.contains("ops turn") && !shown.contains("loading"));
    }

    /// A selection the world holds an entry for draws that entry's launch
    /// facts; one the world does not hold names the gap and guesses none.
    #[test]
    fn a_selection_outside_the_world_names_its_launch_gap() {
        let mut app = app(None);
        two(&mut app);
        app.show();
        assert_eq!(app.overview.launch[0], "mode: unrecorded");
        app.world = World::new(Timestamp::now(), Vec::new());
        app.show();
        assert_eq!(app.overview.launch, [crate::app::overview::LAUNCH_GAP]);
    }

    /// A lane older than the one shown, or read as an identity the session
    /// no longer records, is dropped; a fleet read naming a new identity
    /// takes the old lane with it.
    #[test]
    fn a_stale_or_replaced_lane_never_reinstalls() {
        let mut app = app(None);
        two(&mut app);
        app.answer(view("web", ID, 5, "fresh"));
        app.answer(view("web", ID, 4, "older"));
        assert!(framed(&mut app).contains("fresh"));
        app.answer(view("web", "another-id", 6, "replaced"));
        assert!(!framed(&mut app).contains("replaced"));
        let read = loader::FleetRead {
            dirs: app.dirs.clone(),
            ids: [("web", "new-id"), ("ops", ID)]
                .into_iter()
                .map(|(name, id)| (name.to_owned(), id.to_owned()))
                .collect(),
            world: app.world.clone(),
            facts: app.facts.clone(),
            needs: app.needs.clone(),
            fleet: app.fleet.clone(),
            pairs: std::collections::BTreeMap::new(),
            scanned: true,
            memos: std::collections::BTreeMap::new(),
        };
        app.answer(Answer::Fleet(read));
        let shown = framed(&mut app);
        assert!(
            !shown.contains("fresh") && shown.contains("loading"),
            "{shown}"
        );
        app.answer(view("web", ID, 7, "old identity, late"));
        assert!(!framed(&mut app).contains("old identity"));
        app.answer(view("web", "new-id", 8, "new identity"));
        assert!(framed(&mut app).contains("new identity"));
    }

    /// Before the fleet is read the first frame stands on the home name: its
    /// header, `loading` in the list and in the chat, and no tab.
    #[test]
    fn the_first_frame_stands_before_any_read() {
        let mut app = App::new(Some("api".to_owned()), None, None, None);
        app.show();
        let shown = framed(&mut app);
        assert!(shown.lines().nth(1).is_some_and(|row| row.contains("api")));
        assert_eq!(shown.matches("loading").count(), 2, "{shown}");
        assert!(!shown.contains("Overview"), "{shown}");
        assert!(!shown.contains("No sessions."), "{shown}");
    }

    /// A look read before any fleet read names its session survives the
    /// first one: nothing is guessed before it, the read look after it.
    #[test]
    fn a_look_read_before_the_first_fleet_dresses_after_it() {
        let mut app = App::new(Some("api".to_owned()), None, None, None);
        assert_eq!(app.dressed(), (None, None), "nothing read: no colour");
        let off = crate::theme::Look::read("", "", "off", "");
        app.answer(Answer::Look {
            name: "api".to_owned(),
            look: Some(off),
            zone: None,
        });
        let read = loader::FleetRead {
            dirs: app.dirs.clone(),
            ids: std::iter::once(("api".to_owned(), ID.to_owned())).collect(),
            world: app.world.clone(),
            facts: app.facts.clone(),
            needs: app.needs.clone(),
            fleet: one_row(Some("api")),
            pairs: std::collections::BTreeMap::new(),
            scanned: true,
            memos: std::collections::BTreeMap::new(),
        };
        app.answer(Answer::Fleet(read));
        assert_eq!(app.dressed(), (Some(off), None), "home's read look stays");
    }

    /// One fleet read built from what `app` holds, naming `ids`.
    fn read_of(app: &App, ids: &[(&str, &str)]) -> loader::FleetRead {
        loader::FleetRead {
            dirs: app.dirs.clone(),
            ids: ids
                .iter()
                .map(|(name, id)| ((*name).to_owned(), (*id).to_owned()))
                .collect(),
            world: app.world.clone(),
            facts: app.facts.clone(),
            needs: app.needs.clone(),
            fleet: app.fleet.clone(),
            pairs: app.pairs.clone(),
            scanned: true,
            memos: std::collections::BTreeMap::new(),
        }
    }

    /// A fleet read naming the same identity keeps a foreign session's last
    /// lane and look; home keeps its own whatever the fleet names; a look
    /// read before any fleet read survives the first one for a listed
    /// session, and home's even unlisted.
    #[test]
    fn a_fleet_read_keeps_what_still_describes_its_session() {
        let mut app = app(Some("api"));
        two(&mut app);
        let drawn = Some(crate::theme::Look::DEFAULT);
        for name in ["web", "api"] {
            app.answer(Answer::Look {
                name: name.to_owned(),
                look: drawn,
                zone: None,
            });
        }
        app.ids.clear();
        app.answer(Answer::Fleet(read_of(&app, &[("web", ID), ("ops", ID)])));
        assert!(
            app.looks.contains_key("web"),
            "a listed session's early look"
        );
        assert!(app.looks.contains_key("api"), "home's early look, unlisted");
        app.answer(view("web", ID, 1, "web turn"));
        app.shown.insert(
            "api".to_owned(),
            super::Shown {
                id: "home-id".to_owned(),
                seq: 1,
                lane: Rc::default(),
            },
        );
        app.answer(Answer::Fleet(read_of(&app, &[("web", ID), ("ops", ID)])));
        assert!(
            app.shown.contains_key("web"),
            "same identity keeps its lane"
        );
        assert!(app.looks.contains_key("web"), "and its look");
        assert!(app.shown.contains_key("api"), "home keeps its lane");
    }

    /// The lanes kept are the reader's kept order: a session past KEEP takes
    /// its lane with it.
    #[test]
    fn a_lane_past_the_kept_order_is_dropped() {
        let mut app = app(None);
        let names: Vec<String> = (0..20).map(|at| format!("s{at:02}")).collect();
        app.fleet = Fleet {
            rows: names
                .iter()
                .enumerate()
                .map(|(at, name)| row(name, at + 1, false))
                .collect(),
            home: None,
        };
        for name in &names {
            app.dirs.insert(name.clone(), PathBuf::from(name));
            app.ids.insert(name.clone(), ID.to_owned());
        }
        app.model = Model::new(&app.fleet);
        for name in ["s00", "s15", "s19"] {
            app.answer(view(name, ID, 1, name));
        }
        app.evict();
        assert!(app.shown.contains_key("s00") && app.shown.contains_key("s15"));
        assert!(!app.shown.contains_key("s19"), "beyond KEEP");
    }

    /// Ruling 5: a write through the home composer asks the reader to read
    /// home again; a key that writes nothing asks nothing.
    #[test]
    fn a_composer_write_asks_for_a_home_reread() {
        let root = Root::new("reread");
        let (mut app, _reader) = housed(&root);
        let (ask, asks) = std::sync::mpsc::channel();
        app.ask = Some(ask);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        let typed = writing(&mut app);
        asks.try_iter().for_each(drop);
        assert_eq!(app.compose(Key::Text(b"/close".to_vec()), typed), Some(()));
        assert!(asks.try_iter().next().is_none(), "typing writes nothing");
        assert_eq!(app.compose(Key::Enter, typed), Some(()));
        let rereads: Vec<String> = asks
            .try_iter()
            .filter_map(|request| match request {
                loader::Request::Reread(name) => Some(name),
                loader::Request::Focus(_)
                | loader::Request::Writing(_)
                | loader::Request::Settings { .. } => None,
            })
            .collect();
        assert_eq!(rereads, ["api"]);
    }

    /// A lone ESC completes once a wait ends with no keys: on the timeout
    /// alone, and on answers alone.
    #[test]
    fn a_wait_with_no_keys_completes_a_lone_escape() {
        let (_wake, wakes) = std::sync::mpsc::channel();
        let quiet = Answer::Look {
            name: "web".to_owned(),
            look: None,
            zone: None,
        };
        let root = Root::new("lone-esc");
        for first in [None, Some(loader::Wake::Answer(Box::new(quiet)))] {
            let (mut app, _reader) = housed(&root);
            writing(&mut app);
            let mut keys = crate::console::input::Keys::app();
            let typed = Instant::now()
                .checked_sub(Duration::from_secs(1))
                .expect("an instant a second ago");
            assert!(keys.feed(b"\x1b", typed).is_empty(), "ESC waits");
            assert_eq!(super::drain(&mut app, &mut keys, &wakes, first), Some(true));
            assert!(!app.composing(), "an Esc read before the lease browses");
        }
    }

    /// The foreign session `api` showing `turns` one-row turns, as read.
    fn scrolled(turns: i64) -> App {
        let mut app = app(None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        app.world = World::new(Timestamp::now(), vec![entry("api", Status::Running, None)]);
        app.dirs.insert("api".to_owned(), PathBuf::from("api"));
        app.ids.insert("api".to_owned(), ID.to_owned());
        app.answer(Answer::View(ViewRead {
            name: "api".to_owned(),
            id: ID.to_owned(),
            seq: 1,
            lane: Lane {
                items: (0..turns)
                    .map(|at| said(&format!("turn {at}"), at))
                    .collect(),
                coverage: Vec::new(),
            },
            needs: None,
            roster: None,
        }));
        app
    }

    /// `bytes` read at once on a 160x36 pane (27 chat rows), taken through
    /// the loop's own drain and frame until no key is held.
    fn replayed(app: &mut App, bytes: &[u8]) -> usize {
        let (_wake, wakes) = std::sync::mpsc::channel();
        let mut keys = crate::console::input::Keys::app();
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 36));
        app.frame(&mut buf);
        let mut first = Some(loader::Wake::Keys(Instant::now(), bytes.to_vec()));
        for _ in 0..64 {
            assert!(super::drain(app, &mut keys, &wakes, first.take()).is_some());
            app.frame(&mut buf);
            if app.deferred.is_empty() {
                return app.model.scroll_rows(27);
            }
        }
        panic!("keys still held");
    }

    /// A wheel notch down after page keys in the same read lands three rows
    /// below where the pages went, as on a fully drawn lane, and never at the
    /// edge of what the first frame happened to produce.
    #[test]
    fn a_notch_down_after_page_keys_keeps_the_pages() {
        let down = b"\x1b[<65;48;6M";
        let mut bytes = b"\x1b[5~".repeat(3);
        bytes.extend_from_slice(down);
        assert_eq!(replayed(&mut scrolled(160), &bytes), 81 - 3);
        // Past the true top the notch is taken from where the pages asked,
        // then bounded: the top, not three rows under it.
        let mut bytes = b"\x1b[5~".repeat(100);
        bytes.extend_from_slice(down);
        let top = 80 * 3 - 1 - 27;
        assert_eq!(replayed(&mut scrolled(80), &bytes), top);
    }

    /// A notch landing exactly on the last row the frame drew is taken in
    /// the same read: only one past it waits for the next frame.
    #[test]
    fn a_notch_onto_the_drawn_edge_is_taken_at_once() {
        let mut app = scrolled(160);
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 36));
        app.frame(&mut buf);
        assert!(!app.layout.complete);
        let edge = app.layout.max_scroll;
        assert_eq!((edge - 27) % 3, 0, "the edge is whole notches above a page");
        let (_wake, wakes) = std::sync::mpsc::channel();
        let mut keys = crate::console::input::Keys::app();
        let mut bytes = b"\x1b[5~".to_vec();
        bytes.extend(b"\x1b[<64;48;6M".repeat((edge - 27) / 3));
        let read = Some(loader::Wake::Keys(Instant::now(), bytes));
        assert!(super::drain(&mut app, &mut keys, &wakes, read).is_some());
        assert!(app.deferred.is_empty(), "held at the edge");
        assert_eq!(app.model.scroll_rows(27), edge);
        let up = Some(loader::Wake::Keys(
            Instant::now(),
            b"\x1b[<64;48;6M".to_vec(),
        ));
        assert!(super::drain(&mut app, &mut keys, &wakes, up).is_some());
        assert_eq!(app.deferred.len(), 1, "one past the edge waits");
    }

    /// Ruling 3: a selection shares its read lane rather than copying it.
    #[test]
    fn a_foreign_selection_shares_its_read_lane() {
        let mut app = app(Some("api"));
        two(&mut app);
        app.answer(view("web", ID, 1, "web turn"));
        assert_eq!(app.model.selected(), Some("web"));
        assert!(Rc::ptr_eq(&app.lane, &app.shown["web"].lane), "a copy");
    }

    /// Home's lane is its read with ae's own notices in it, in the order a
    /// stable sort by time gives — the read's own order kept on a tie — and
    /// it is built once per read and per notice, not on every show.
    #[test]
    fn homes_merge_is_built_once_per_read_and_notice() {
        let mut app = app(Some("api"));
        let base = Rc::new(Lane {
            items: vec![said("late", 30), said("early", 10), said("tie read", 20)],
            coverage: vec!["gap".to_owned()],
        });
        let api = |body, at| ("api".to_owned(), said(body, at));
        app.notices = vec![api("tie notice", 20), api("first", 0)];
        let bodies = |lane: &Lane| -> Vec<String> {
            lane.items.iter().map(|item| item.body.clone()).collect()
        };
        let merged = app.merge("api", Some(Rc::clone(&base)));
        assert_eq!(
            bodies(&merged),
            ["first", "early", "tie read", "tie notice", "late"]
        );
        assert_eq!(merged.coverage, ["gap"]);
        let again = app.merge("api", Some(Rc::clone(&base)));
        assert!(Rc::ptr_eq(&merged, &again), "rebuilt with nothing new");
        let read = Rc::new((*base).clone());
        let reread = app.merge("api", Some(Rc::clone(&read)));
        assert!(!Rc::ptr_eq(&again, &reread), "a new read is merged afresh");
        app.effects(vec![Effect::Print("said".to_owned())]);
        let noticed = app.merge("api", Some(Rc::clone(&read)));
        assert!(!Rc::ptr_eq(&reread, &noticed), "a new notice is merged");
        assert_eq!(bodies(&noticed).last().map(String::as_str), Some("said"));
        let unread = app.merge("api", None);
        assert_eq!(bodies(&unread).len(), 3, "notices alone before a read");
        assert!(Rc::ptr_eq(&unread, &app.merge("api", None)));
        // B2 NOTICES: a recorded session's notices draw only in its own lane,
        // an unrecorded one's in whichever lane is shown.
        app.dirs.insert("api".to_owned(), PathBuf::new());
        app.notice("gone", "gone's".to_owned());
        let web = app.merge("web", Some(Rc::clone(&read)));
        assert_eq!(bodies(&web), ["early", "tie read", "late", "gone's"]);
        app.dirs.insert("gone".to_owned(), PathBuf::new());
        app.noticed += 1;
        assert!(Rc::ptr_eq(&app.merge("web", Some(Rc::clone(&read))), &read));
        (0..=super::NOTICES).for_each(|n| app.notice("web", n.to_string()));
        let web = app.notices.iter().filter(|(of, _)| of == "web").count();
        assert_eq!(web, super::NOTICES, "capped per session");
    }

    /// The look of a session dresses the app only as that session's: the
    /// selection's look never dresses home, a drawn home look wins.
    #[test]
    fn a_look_dresses_only_as_the_session_it_describes() {
        let mut app = app(Some("api"));
        let mut fleet = one_row(Some("api"));
        fleet.rows.push(row("web", 2, false));
        app.fleet = fleet;
        app.model = Model::new(&app.fleet);
        let drawn = crate::theme::Look::DEFAULT;
        app.answer(Answer::Look {
            name: "web".to_owned(),
            look: Some(drawn),
            zone: Some("+0200".to_owned()),
        });
        let _ = app.model.key(Browse::Digit(2), &app.fleet, false, true);
        assert_eq!(app.dressed(), (None, None), "home unread: no colour");
        let _ = app.model.key(Browse::Digit(1), &app.fleet, false, true);
        app.answer(Answer::Look {
            name: "api".to_owned(),
            look: None,
            zone: None,
        });
        assert_eq!(app.dressed().1, None, "web's zone never dresses home");
        let _ = app.model.key(Browse::Digit(2), &app.fleet, false, true);
        assert_eq!(app.dressed().1, Some("+0200"));
        app.answer(Answer::Look {
            name: "api".to_owned(),
            look: Some(drawn),
            zone: Some("-0500".to_owned()),
        });
        assert_eq!(app.dressed().1, Some("-0500"), "home's drawn look wins");
        for name in ["api", "web"] {
            app.answer(Answer::Look {
                name: name.to_owned(),
                look: None,
                zone: None,
            });
        }
        assert_eq!(
            app.dressed().0,
            Some(crate::theme::Look::DEFAULT),
            "read and lookless: the default drawn look"
        );
        let unread = App::new(None, None, None, None);
        assert_eq!(unread.dressed(), (None, None), "nothing read: no colour");
    }

    /// A selection made by the fleet itself — its first choice, or a
    /// selected row leaving — is told to the reader like a key's.
    #[test]
    fn every_change_of_selection_is_told_to_the_reader() {
        let (ask, asks) = std::sync::mpsc::channel();
        let mut app = App::new(None, None, None, Some(ask));
        let told = || -> Vec<Option<String>> {
            asks.try_iter()
                .filter_map(|request| match request {
                    loader::Request::Focus(name) => Some(name),
                    loader::Request::Reread(_)
                    | loader::Request::Writing(_)
                    | loader::Request::Settings { .. } => None,
                })
                .collect()
        };
        app.focus();
        assert!(told().is_empty(), "nothing selected, nothing told");
        two(&mut app);
        app.focus();
        assert_eq!(told(), [Some("web".to_owned())]);
        app.focus();
        assert!(told().is_empty(), "once per change");
        app.fleet.rows.remove(0);
        app.model.reconcile(&app.fleet);
        app.focus();
        assert_eq!(told(), [Some("ops".to_owned())]);
    }

    // ---- settings overlay pins. Oracles: docs/app.md ## Settings and the
    // ---- frozen tests/it/app_settings_spec.rs (live); these pin the seams
    // ---- the live rig cannot see: request order, generation drops, merges.

    fn settings_requests(asks: &std::sync::mpsc::Receiver<loader::Request>) -> Vec<(bool, u64)> {
        asks.try_iter()
            .filter_map(|request| match request {
                loader::Request::Settings { open, generation } => Some((open, generation)),
                loader::Request::Focus(_)
                | loader::Request::Reread(_)
                | loader::Request::Writing(_) => None,
            })
            .collect()
    }

    /// Open asks once per open on a fresh generation; close always tells.
    #[test]
    fn settings_open_asks_once_and_close_tells() {
        let (ask, asks) = std::sync::mpsc::channel();
        let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
        app.open_settings();
        app.open_settings();
        assert_eq!(settings_requests(&asks), [(true, 1)]);
        app.close_settings();
        assert_eq!(settings_requests(&asks), [(false, 1)]);
        app.open_settings();
        assert_eq!(settings_requests(&asks), [(true, 2)]);
    }

    /// Only the open generation paints; quota-only answers keep the rest.
    #[test]
    fn settled_merges_own_generation_and_drops_stale() {
        let mut app = app(Some("api"));
        app.open_settings();
        let quota = || {
            vec![settings::QuotaRow {
                label: "q".to_owned(),
                header: false,
            }]
        };
        let config = || settings::ConfigView::Rows {
            header: settings::ConfigHeader::default(),
            rows: Vec::new(),
        };
        let about = || settings::AboutFacts {
            tmux_version: "3.5".to_owned(),
            tmux_verdict: "ok",
            state_root: "/r".to_owned(),
            config_file: "/c".to_owned(),
            server: "ambient".to_owned(),
        };
        app.settled(99, quota(), Some(config()), Some(about()));
        assert!(app.settings_bodies.quota.is_none(), "stale dropped");
        app.settled(1, quota(), Some(config()), Some(about()));
        assert!(app.settings_bodies.quota.is_some(), "own paints");
        assert!(matches!(
            app.settings_bodies.config,
            settings::ConfigView::Rows { .. }
        ));
        app.settled(
            1,
            vec![settings::QuotaRow {
                label: "q2".to_owned(),
                header: false,
            }],
            None,
            None,
        );
        assert_eq!(app.settings_bodies.quota.as_ref().expect("quota").len(), 1);
        assert_eq!(
            app.settings_bodies.quota.as_ref().expect("quota")[0].label,
            "q2"
        );
        assert!(
            matches!(
                app.settings_bodies.config,
                settings::ConfigView::Rows { .. }
            ),
            "quota-only keeps config"
        );
        assert!(
            app.settings_bodies.about.is_some(),
            "quota-only keeps about"
        );
        app.close_settings();
        app.settled(1, quota(), Some(config()), Some(about()));
        assert_eq!(
            app.settings_bodies.quota.as_ref().expect("quota")[0].label,
            "q2",
            "closed drops"
        );
    }

    /// Modal keys: Esc/q/s close and tell, Tab/1-3 switch, the rest dies.
    #[test]
    fn settings_keys_route_before_compose() {
        let (ask, asks) = std::sync::mpsc::channel();
        let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
        app.fleet = one_row(Some("api"));
        let at = Instant::now();
        app.open_settings();
        assert!(settings_key(&mut app, &Key::Text(b"x".to_vec()), at).is_some());
        assert!(app.model.settings_open(), "text swallowed");
        assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Quota);
        assert!(settings_key(&mut app, &Key::Tab, at).expect("tab"));
        assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Config);
        assert!(settings_key(&mut app, &Key::Text(b"4".to_vec()), at).is_some());
        assert!(app.model.settings_open(), "digit 4 swallowed");
        assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Config);
        assert!(settings_key(&mut app, &Key::Escape, at).expect("close"));
        assert!(!app.model.settings_open());
        assert_eq!(
            settings_requests(&asks).last(),
            Some(&(false, 1)),
            "close told"
        );
        app.open_settings();
        assert!(settings_key(&mut app, &Key::Text(b"q".to_vec()), at).expect("q closes"));
        assert!(!app.model.settings_open());
        app.open_settings();
        assert!(settings_key(&mut app, &Key::Text(b"s!".to_vec()), at).expect("s closes"));
        assert!(!app.model.settings_open());
        app.open_settings();
        assert!(
            settings_key(&mut app, &Key::Interrupt, at).is_none(),
            "^C quits"
        );
    }

    /// Pins win over changed files; the overlay beats the recorded global;
    /// global-only keys read the current global alone.
    #[test]
    fn resolve_config_pins_win_and_scopes_hold() {
        let root = Root::new("settings-resolve");
        let recorded = root.0.join("recorded");
        let overlay = root.0.join("overlay");
        let current = root.0.join("config");
        std::fs::write(
            &recorded,
            "[workspace]\nchat = on\nquota = on\nfleet_order = recorded-fleet\n",
        )
        .expect("recorded");
        std::fs::write(&overlay, "[workspace]\nchat = app\n").expect("overlay");
        std::fs::write(&current, "[workspace]\nfleet_order = current-fleet\n").expect("current");
        let dir = session(
            &root,
            "api",
            &format!(
                "config={}\nlocal_config={}\nquota=off\n",
                recorded.display(),
                overlay.display()
            ),
        );
        let settings::ConfigView::Rows { header, rows } =
            settings::resolve_config(Some(&dir), &current)
        else {
            panic!("rows expected");
        };
        let row = |key: &str| rows.iter().find(|row| row.key == key).expect(key);
        assert_eq!(header.global, recorded.display().to_string());
        assert_eq!(
            header.current_global.as_deref(),
            Some(current.display().to_string()).as_deref()
        );
        assert_eq!(row("quota").value, "off");
        assert_eq!(row("quota").source, settings::ConfigSource::Launch);
        assert_eq!(row("chat").value, "app");
        assert_eq!(row("chat").source, settings::ConfigSource::Session);
        assert_eq!(row("fleet_order").value, "current-fleet");
        assert_eq!(row("fleet_order").source, settings::ConfigSource::Global);
        assert!(row("fleet_order").global_only);
        assert_eq!(row("main").value, "(unset)");
        assert_eq!(row("main").source, settings::ConfigSource::Default);
    }

    /// A torn consulted file is one honest row naming the file and the reason.
    #[test]
    fn resolve_config_torn_file_is_one_honest_row() {
        let root = Root::new("settings-torn");
        let recorded = root.0.join("broken");
        std::fs::write(&recorded, "[prompt]\ninstructions = \"\"\"\nunclosed\n").expect("broken");
        let current = root.0.join("config");
        std::fs::write(&current, "[workspace]\nchat = on\n").expect("current");
        let dir = session(&root, "api", &format!("config={}\n", recorded.display()));
        let settings::ConfigView::Unreadable(why) = settings::resolve_config(Some(&dir), &current)
        else {
            panic!("one honest row expected");
        };
        assert!(
            why.contains(&recorded.display().to_string()),
            "names the file: {why}"
        );
        assert!(why.contains("unterminated"), "names the reason: {why}");
    }

    /// Missing files are no entries: everything falls through to defaults.
    #[test]
    fn resolve_config_missing_files_fall_to_defaults() {
        let root = Root::new("settings-missing");
        let current = root.0.join("config");
        let dir = session(&root, "api", "");
        let settings::ConfigView::Rows { header, rows } =
            settings::resolve_config(Some(&dir), &current)
        else {
            panic!("rows expected");
        };
        assert_eq!(header.global, current.display().to_string());
        assert!(header.current_global.is_none(), "not distinct");
        assert!(header.session.is_none(), "no overlay selected");
        assert_eq!(rows.len(), 20);
        let row = |key: &str| rows.iter().find(|row| row.key == key).expect(key);
        assert_eq!(row("chat").value, "on");
        assert_eq!(row("chat").source, settings::ConfigSource::Default);
    }

    /// No home session: one honest row, never guessed defaults.
    #[test]
    fn resolve_config_without_home_is_honest() {
        let root = Root::new("settings-nohome");
        let current = root.0.join("config");
        assert!(matches!(
            settings::resolve_config(None, &current),
            settings::ConfigView::Unreadable(_)
        ));
    }

    /// A home session whose meta cannot be read names the meta file.
    #[test]
    fn resolve_config_unreadable_meta_names_file() {
        let root = Root::new("settings-nometa");
        let dir = root.0.join("sessions").join("ghost");
        std::fs::create_dir_all(&dir).expect("session dir");
        let current = root.0.join("config");
        let settings::ConfigView::Unreadable(why) = settings::resolve_config(Some(&dir), &current)
        else {
            panic!("one honest row expected");
        };
        assert!(
            why.contains(&dir.display().to_string()),
            "names the meta: {why}"
        );
    }

    /// Page keys move the drawn body height the frame set, not a constant.
    #[test]
    fn settings_page_keys_use_drawn_height() {
        let mut app = app(Some("api"));
        app.fleet = one_row(Some("api"));
        app.open_settings();
        app.model.set_settings_page_rows(5);
        app.model.key(Browse::PageDown, &app.fleet, false, true);
        assert_eq!(app.model.settings().expect("open").scroll, 5);
        app.model.key(Browse::PageDown, &app.fleet, false, true);
        assert_eq!(app.model.settings().expect("open").scroll, 10);
        app.model.key(Browse::PageUp, &app.fleet, false, true);
        assert_eq!(app.model.settings().expect("open").scroll, 5);
    }

    /// A batched chunk spells in byte order: `2s` selects session 2, then
    /// opens; `qs` quits before ever reaching s; `sq` opens and closes,
    /// every transition carrying its request.
    #[test]
    fn settings_chunk_bytes_keep_order_and_requests() {
        let two = || Fleet {
            rows: vec![row("api", 1, true), row("web", 2, false)],
            home: Some("api".to_owned()),
        };
        let (ask, asks) = std::sync::mpsc::channel();
        let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
        app.fleet = two();
        assert!(browse(&mut app, &Key::Text(b"2s".to_vec()), Instant::now()).is_some());
        assert_eq!(app.model.selected(), Some("web"));
        assert!(app.model.settings_open());
        assert_eq!(settings_requests(&asks), [(true, 1)]);
        let (ask, asks) = std::sync::mpsc::channel();
        let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
        app.fleet = two();
        assert!(
            browse(&mut app, &Key::Text(b"qs".to_vec()), Instant::now()).is_none(),
            "quits"
        );
        assert!(!app.model.settings_open());
        assert!(settings_requests(&asks).is_empty(), "no open asked");
        let (ask, asks) = std::sync::mpsc::channel();
        let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
        app.fleet = two();
        assert!(browse(&mut app, &Key::Text(b"sq".to_vec()), Instant::now()).is_some());
        assert!(!app.model.settings_open());
        assert_eq!(settings_requests(&asks), [(true, 1), (false, 1)]);
    }

    /// Writing that starts mid-chunk drops the rest, read before the lease:
    /// `is` starts writing on `i` and names the dropped `s`, never opening
    /// settings (R-B3).
    #[test]
    fn settings_browse_chunk_resumes_composer_mid_chunk() {
        let root = Root::new("settings-is");
        let (ask, asks) = std::sync::mpsc::channel();
        let (mut app, _reader) = housed(&root);
        app.ask = Some(ask);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        let at = Instant::now();
        app.take(Reading::Owner, at);
        assert!(take_keys(&mut app, vec![(Key::Text(b"is".to_vec()), at)]).is_some());
        assert!(app.composing(), "writing started");
        assert!(!app.model.settings_open(), "never opened");
        assert_eq!(drafted(&app), "");
        let said = &app.notices.last().expect("the drop is said").1.body;
        assert_eq!(said, "dropped the keys typed before writing started");
        assert!(
            settings_requests(&asks).is_empty(),
            "zero settings requests"
        );
    }

    /// A modal close mid-chunk dispatches the rest as browse: `q2` closes
    /// with its request, then selects session 2 with its lane.
    #[test]
    fn settings_modal_close_dispatches_rest_as_browse() {
        let (ask, asks) = std::sync::mpsc::channel();
        let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
        app.fleet = Fleet {
            rows: vec![row("api", 1, true), row("web", 2, false)],
            home: Some("api".to_owned()),
        };
        app.fleeted = true;
        app.open_settings();
        assert!(settings_key(&mut app, &Key::Text(b"q2".to_vec()), Instant::now()).is_some());
        assert!(!app.model.settings_open());
        assert_eq!(app.model.selected(), Some("web"));
        assert!(
            app.lane.coverage.iter().any(|line| line.contains("web")),
            "lane follows the selection"
        );
        assert_eq!(settings_requests(&asks).last(), Some(&(false, 1)));
    }

    /// A close mid-chunk resumes the suspended composer: the rest of the
    /// run types into the kept draft whole — `s`, digits and multibyte runs
    /// alike — with exactly one close request and no selection change.
    #[test]
    fn settings_modal_close_resumes_suspended_composer() {
        let root = Root::new("settings-resume");
        let (ask, asks) = std::sync::mpsc::channel();
        let (mut app, _reader) = housed(&root);
        app.ask = Some(ask);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        let at = writing(&mut app);
        assert_eq!(app.compose(Key::Text(b"kept".to_vec()), at), Some(()));
        app.open_settings();
        assert!(settings_key(&mut app, &Key::Text("qsé".as_bytes().to_vec()), at).is_some());
        assert!(!app.model.settings_open());
        assert!(app.composing(), "still writing");
        assert_eq!(
            drafted(&app),
            "keptsé",
            "rest typed whole into the kept draft"
        );
        assert_eq!(app.model.selected(), Some("api"), "no browse dispatch");
        assert_eq!(settings_requests(&asks), [(true, 1), (false, 1)]);
        app.open_settings();
        assert!(settings_key(&mut app, &Key::Text(b"q2".to_vec()), at).is_some());
        assert_eq!(drafted(&app), "keptsé2", "digits type too");
        assert_eq!(app.model.selected(), Some("api"));
        assert_eq!(
            settings_requests(&asks),
            [(true, 2), (false, 2)],
            "no extra requests"
        );
    }

    /// A gear click while the overlay is open closes it with its close
    /// request and keeps the suspended composer: still writing, draft kept.
    #[test]
    fn settings_gear_click_closes_and_keeps_suspended_composer() {
        let root = Root::new("settings-gear-close");
        let (ask, asks) = std::sync::mpsc::channel();
        let (mut app, _reader) = housed(&root);
        app.ask = Some(ask);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        let at = writing(&mut app);
        assert_eq!(app.compose(Key::Text(b"kept".to_vec()), at), Some(()));
        app.open_settings();
        let shown = framed(&mut app);
        let keys = shown.lines().last().expect("keys row");
        let tail = keys.trim_end();
        assert!(
            tail.ends_with('\u{2699}') || tail.ends_with('*'),
            "gear drawn last: {tail:?}"
        );
        assert!(
            app.mouse(super::Mouse {
                kind: super::MouseKind::Click,
                column: 159,
                row: 44,
            }),
            "gear click closes"
        );
        assert!(!app.model.settings_open());
        assert_eq!(settings_requests(&asks), [(true, 1), (false, 1)]);
        assert!(app.composing(), "still writing");
        assert_eq!(drafted(&app), "kept");
    }

    /// The overlay's tab title and close label, still in the last frame's
    /// layout after a key closed it, do nothing when pressed.
    #[test]
    fn settings_targets_left_in_the_last_frame_do_nothing_once_closed() {
        let root = Root::new("settings-stale-targets");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.open_settings();
        let shown = framed(&mut app);
        let column = |row: usize, word: &str| {
            let line = shown.lines().nth(row).expect("drawn row");
            u16::try_from(line[..line.find(word).expect("drawn word")].chars().count())
                .expect("column")
        };
        let (config, close) = (column(1, "Config"), column(0, "close"));
        app.close_settings();
        for (column, row) in [(config, 1), (close, 0)] {
            let kind = super::MouseKind::Click;
            assert!(!app.mouse(super::Mouse { kind, column, row }));
        }
        assert!(!app.model.settings_open());
    }

    /// A drawn title press shows its tab now, without a reader answer to
    /// repaint it later. A scrolled same-tab press resets; one at top is idle.
    #[test]
    fn settings_title_click_requests_its_frame_without_a_background_answer() {
        let mut app = App::new(None, None, None, None);
        app.open_settings();
        let (_wake, wakes) = std::sync::mpsc::channel();
        let mut keys = crate::console::input::Keys::app();
        for (name, tab, scroll, redraw) in [
            ("Config", SettingsTab::Config, 3, true),
            ("Config", SettingsTab::Config, 3, true),
            ("Config", SettingsTab::Config, 0, false),
            ("About", SettingsTab::About, 3, true),
            ("Quota", SettingsTab::Quota, 3, true),
        ] {
            let shown = framed(&mut app);
            let titles = shown.lines().nth(1).expect("drawn tab titles");
            let byte = titles.find(name).expect("drawn title");
            let column = titles[..byte].chars().count() + name.chars().count() - 1;
            app.model.settings_wheel(scroll);
            let press = format!("\x1b[<0;{};2M", column + 1).into_bytes();
            let first = Some(loader::Wake::Keys(Instant::now(), press));
            assert_eq!(
                super::drain(&mut app, &mut keys, &wakes, first),
                Some(redraw),
                "{name} click must request its own changed frame"
            );
            let open = app.model.settings().expect("overlay stays open");
            assert_eq!(open.tab, tab, "drawn title selects its tab");
            assert_eq!(open.scroll, 0, "title click resets like a key");
        }
    }

    /// A swallowed byte keeps the chunk's earlier redraw: `sx` opens on `s`
    /// and swallows `x` modal, returning `Some(true)` with one open request.
    #[test]
    fn settings_chunk_swallowed_byte_keeps_earlier_redraw() {
        let (ask, asks) = std::sync::mpsc::channel();
        let mut app = App::new(Some("api".to_owned()), None, None, Some(ask));
        app.fleet = Fleet {
            rows: vec![row("api", 1, true), row("web", 2, false)],
            home: Some("api".to_owned()),
        };
        assert_eq!(
            take_keys(&mut app, vec![(Key::Text(b"sx".to_vec()), Instant::now())]),
            Some(true),
            "open redraw survives the swallowed byte"
        );
        assert!(app.model.settings_open());
        assert_eq!(settings_requests(&asks), [(true, 1)]);
    }

    /// A housed home plus a foreign row: session, tab, compose, border,
    /// chat and gear geometry all drawn in one closed frame.
    fn strip_app(root: &Root) -> (App, Reader) {
        let (mut app, reader) = housed(root);
        app.fleet = Fleet {
            rows: vec![row("api", 1, true), row("web", 2, false)],
            home: Some("api".to_owned()),
        };
        app.model = Model::new(&app.fleet);
        assert_eq!(app.model.selected(), Some("api"));
        app.take(Reading::Owner, Instant::now());
        app.world = World::new(
            Timestamp::now(),
            vec![
                entry("api", Status::Running, None),
                entry("web", Status::Running, None),
            ],
        );
        for name in ["api", "web"] {
            app.facts.insert(
                name.to_owned(),
                fleet::Facts::Seats {
                    id: ID.to_owned(),
                    agents: Vec::new(),
                },
            );
        }
        (app, reader)
    }

    /// The first cell of `needle` in `shown` whose drawn target matches.
    fn strip_cell(
        shown: &str,
        app: &App,
        needle: &str,
        want: &str,
        matches: impl Fn(Option<super::draw::Hit>) -> bool,
    ) -> (usize, usize) {
        let click = |column: usize, row: usize| super::Mouse {
            kind: super::MouseKind::Click,
            column: u16::try_from(column).expect("frame column"),
            row: u16::try_from(row).expect("frame row"),
        };
        shown
            .lines()
            .enumerate()
            .find_map(|(row, line)| {
                line.match_indices(needle).find_map(|(byte, _)| {
                    let column = line[..byte].chars().count();
                    matches(app.layout.hit(click(column, row))).then_some((row, column))
                })
            })
            .unwrap_or_else(|| panic!("drawn {want}"))
    }

    /// Painting the open overlay strips what it covers: session, tab and
    /// compose targets, borders and chat geometry go, the gear target stays.
    #[test]
    fn settings_paint_strips_covered_targets_borders_and_chat() {
        let root = Root::new("settings-strip");
        let (mut app, _reader) = strip_app(&root);
        let shown = framed(&mut app);
        let click = |column: usize, row: usize| super::Mouse {
            kind: super::MouseKind::Click,
            column: u16::try_from(column).expect("frame column"),
            row: u16::try_from(row).expect("frame row"),
        };
        let (session_row, session_column) =
            strip_cell(&shown, &app, "web", "session target", |hit| {
                matches!(hit, Some(super::draw::Hit::Session(_)))
            });
        let (tab_row, tab_column) = strip_cell(&shown, &app, "Agents 0", "tab target", |hit| {
            matches!(hit, Some(super::draw::Hit::Tab(_)))
        });
        let (compose_row, compose_column) =
            strip_cell(&shown, &app, "api", "compose target", |hit| {
                matches!(hit, Some(super::draw::Hit::Compose))
            });
        assert!(app.layout.shows(Edge::Sidebar), "closed sidebar border");
        let chat_column = (0..160)
            .find(|&column| app.layout.in_chat(click(column, 10)))
            .expect("closed chat geometry");
        assert!(
            matches!(
                app.layout.hit(click(159, 44)),
                Some(super::draw::Hit::Settings)
            ),
            "closed gear target"
        );
        app.open_settings();
        let _ = framed(&mut app);
        assert!(
            app.layout.hit(click(session_column, session_row)).is_none(),
            "covered session target gone"
        );
        assert!(
            app.layout.hit(click(tab_column, tab_row)).is_none(),
            "covered tab target gone"
        );
        assert!(
            app.layout.hit(click(compose_column, compose_row)).is_none(),
            "covered compose target gone"
        );
        assert!(!app.layout.shows(Edge::Sidebar), "covered border gone");
        assert!(
            !app.layout.in_chat(click(chat_column, 10)),
            "covered chat geometry gone"
        );
        assert!(
            matches!(
                app.layout.hit(click(159, 44)),
                Some(super::draw::Hit::Settings)
            ),
            "gear target stays"
        );
    }

    /// The gear-only branch keeps the toggle live: at 89x19 the version has
    /// no room, the gear draws in the last cell, and clicking it opens.
    #[test]
    fn settings_gear_only_cell_opens_settings() {
        let root = Root::new("settings-gear-only");
        let (ask, asks) = std::sync::mpsc::channel();
        let (mut app, _reader) = housed(&root);
        app.ask = Some(ask);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.take(Reading::Owner, Instant::now());
        let mut buf = Buffer::empty(Rect::new(0, 0, 89, 19));
        app.frame(&mut buf);
        let shown = text(&buf);
        assert!(
            !shown.contains(crate::version_line().as_str()),
            "version has no room"
        );
        let keys = shown.lines().last().expect("keys row");
        let tail = keys.trim_end();
        assert!(
            tail.ends_with('\u{2699}') || tail.ends_with('*'),
            "gear drawn last: {tail:?}"
        );
        assert!(
            app.mouse(super::Mouse {
                kind: super::MouseKind::Click,
                column: 88,
                row: 18,
            }),
            "gear cell opens"
        );
        assert!(app.model.settings_open());
        assert_eq!(settings_requests(&asks), [(true, 1)]);
    }

    /// Below the floor in either dimension an open overlay keeps the
    /// closed fallback: the literal row, blank rest, no panel, no targets.
    #[test]
    fn settings_below_floor_either_dimension_keeps_fallback() {
        for (width, height) in [(39u16, 19u16), (89u16, 7u16)] {
            let root = Root::new("settings-floor");
            let (mut app, _reader) = housed(&root);
            app.fleet = one_row(Some("api"));
            app.model = Model::new(&app.fleet);
            app.open_settings();
            let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
            app.frame(&mut buf);
            let shown = text(&buf);
            let expected = format!("ae app needs at least 40x8 (now {width}x{height})");
            let mut rows = shown.lines();
            assert_eq!(
                rows.next().expect("fallback row").trim_end(),
                expected,
                "{width}x{height} literal"
            );
            assert!(
                rows.all(|row| row.trim().is_empty()),
                "{width}x{height} rest blank"
            );
            for tab in ["Settings", "Quota", "Config", "About"] {
                assert!(!shown.contains(tab), "{width}x{height} no {tab}");
            }
            assert_eq!(app.layout.settings_page_rows, 0, "{width}x{height} no body");
            assert_eq!(
                app.layout.settings_max_scroll, 0,
                "{width}x{height} no scroll"
            );
            for (column, row) in [(0, 0), (width / 2, height / 2), (width - 1, height - 1)] {
                assert!(
                    app.layout
                        .hit(super::Mouse {
                            kind: super::MouseKind::Click,
                            column,
                            row,
                        })
                        .is_none(),
                    "{width}x{height} no target at {column}x{row}"
                );
            }
        }
    }

    /// The panel grounds only the rows above the keys row: with a drawn
    /// DARCULA look the closed last row is all ink, and the open overlay
    /// paints base over the covered rows while the last row stays ink.
    #[test]
    fn settings_panel_grounds_covered_rows_and_keeps_keys_ink() {
        use ratatui_core::style::Color;
        let (ink, base) = (Color::Rgb(0x2B, 0x2B, 0x2B), Color::Rgb(0x31, 0x33, 0x35));
        let root = Root::new("settings-grounds");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        let darcula = crate::theme::Look::read("on", "darcula", "on", "on");
        app.answer(Answer::Look {
            name: "api".to_owned(),
            look: Some(darcula),
            zone: None,
        });
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
        app.frame(&mut buf);
        for x in 0..160 {
            assert_eq!(buf[(x, 44)].bg, ink, "closed keys row ink at {x}");
        }
        app.open_settings();
        app.frame(&mut buf);
        let shown = text(&buf);
        assert!(
            shown
                .lines()
                .next()
                .is_some_and(|line| line.contains("Settings")),
            "open title drawn"
        );
        assert!(
            shown.lines().last().is_some_and(|line| {
                let tail = line.trim_end();
                tail.ends_with('\u{2699}') || tail.ends_with('*')
            }),
            "gear present"
        );
        for x in 0..160 {
            assert_eq!(buf[(x, 44)].bg, ink, "open keys row stays ink at {x}");
        }
        for (x, y) in [(0, 0), (80, 20), (159, 43)] {
            assert_eq!(buf[(x, y)].bg, base, "covered row base at ({x},{y})");
        }
    }

    /// At the widest panes the title still draws: `width + LEFT` would
    /// overflow u16 where `width - LEFT` fits.
    #[test]
    fn settings_title_draws_at_terminal_max_width() {
        for width in [65534u16, 65535u16] {
            let root = Root::new("settings-max-width");
            let (mut app, _reader) = housed(&root);
            app.fleet = one_row(Some("api"));
            app.model = Model::new(&app.fleet);
            app.open_settings();
            let mut buf = Buffer::empty(Rect::new(0, 0, width, 8));
            app.frame(&mut buf);
            let shown = text(&buf);
            let mut rows = shown.lines();
            assert!(
                rows.next().is_some_and(|line| line.contains("Settings")),
                "{width} title row"
            );
            let tabs = rows.next().expect("tab row");
            for tab in ["Quota", "Config", "About"] {
                assert!(tabs.contains(tab), "{width} {tab}");
            }
            assert!(
                shown
                    .lines()
                    .last()
                    .is_some_and(|line| line.contains("close")),
                "{width} close hint"
            );
        }
    }

    /// An empty ready quota reads honestly: cold shows loading, the empty
    /// answer shows the no-scopes message instead.
    #[test]
    fn settings_empty_ready_quota_reads_honest() {
        let mut app = app(Some("api"));
        app.open_settings();
        let cold = framed(&mut app);
        assert!(cold.contains("loading"), "cold loads:\n{cold}");
        app.answer(Answer::Settings {
            generation: 1,
            quota: Vec::new(),
            config: None,
            about: None,
        });
        let shown = framed(&mut app);
        assert!(
            shown.contains("no quota scopes read"),
            "honest empty:\n{shown}"
        );
        assert!(!shown.contains("loading"), "ready, not loading:\n{shown}");
    }

    /// The paint resets covered cells whole and clamps a past-the-end scroll
    /// to the last page instead of an emptied body.
    #[test]
    fn settings_paint_resets_cells_and_clamps_bottom() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        for y in 0..10 {
            for x in 0..40 {
                buf[(x, y)].set_symbol("X");
            }
        }
        let rows = |count: usize| {
            (0..count)
                .map(|at| settings::QuotaRow {
                    label: format!("row-{at}"),
                    header: false,
                })
                .collect::<Vec<_>>()
        };
        let bodies = settings::SettingsBodies {
            quota: Some(rows(10)),
            config: settings::ConfigView::Loading,
            about: None,
        };
        let mut layout = draw::Layout::default();
        draw::paint_settings(SettingsTab::Quota, 99, &bodies, None, &mut buf, &mut layout);
        let frame = text(&buf);
        let rows: Vec<&str> = frame.lines().collect();
        assert!(
            !rows[..9].iter().any(|row| row.contains('X')),
            "cells reset"
        );
        assert_eq!(rows[3].trim(), "row-4", "clamped to the last page");
        assert_eq!(rows[8].trim(), "row-9");
        assert_eq!(layout.settings_max_scroll, 4);
        assert_eq!(layout.settings_page_rows, 6);
    }

    /// I13: re-selecting the open tab with its digit shows it from the
    /// first row again, even when already selected.
    #[test]
    fn reselecting_the_open_tab_restores_its_first_row() {
        let mut app = app(Some("api"));
        app.open_settings();
        let quota: Vec<settings::QuotaRow> = (0..60)
            .map(|at| settings::QuotaRow {
                label: format!("qrow-{at:02}"),
                header: false,
            })
            .collect();
        app.answer(Answer::Settings {
            generation: 1,
            quota,
            config: None,
            about: None,
        });
        let first_body_row = |shown: &str| {
            shown
                .lines()
                .find(|row| row.contains("qrow-"))
                .expect("a body row")
                .to_owned()
        };
        let top = first_body_row(&framed(&mut app));
        assert!(top.contains("qrow-00"), "starts at the top:\n{top}");
        let at = Instant::now();
        assert!(take_keys(&mut app, vec![(Key::Text(b"j".to_vec()), at)]).is_some());
        let scrolled = first_body_row(&framed(&mut app));
        assert_ne!(scrolled, top, "j scrolls off the top");
        assert!(scrolled.contains("qrow-01"), "one row down:\n{scrolled}");
        assert!(take_keys(&mut app, vec![(Key::Text(b"1".to_vec()), at)]).is_some());
        assert_eq!(
            first_body_row(&framed(&mut app)),
            top,
            "reselect restores the first row"
        );
    }

    /// I14: the frame settles an overscrolled overlay offset to the drawn
    /// maximum, so the next key up visibly moves instead of redrawing the
    /// same bottom page.
    #[test]
    fn an_overscrolled_overlay_settles_before_the_next_key() {
        let mut app = app(Some("api"));
        app.open_settings();
        let quota: Vec<settings::QuotaRow> = (0..10)
            .map(|at| settings::QuotaRow {
                label: format!("qrow-{at:02}"),
                header: false,
            })
            .collect();
        app.answer(Answer::Settings {
            generation: 1,
            quota,
            config: None,
            about: None,
        });
        let body_ends = |app: &mut App| {
            let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
            app.frame(&mut buf);
            let body: Vec<String> = text(&buf)
                .lines()
                .filter(|row| row.contains("qrow-"))
                .map(str::to_owned)
                .collect();
            (
                body.first().expect("a first body row").clone(),
                body.last().expect("a last body row").clone(),
            )
        };
        let (top, _) = body_ends(&mut app);
        assert!(top.contains("qrow-00"), "starts at the top:\n{top}");
        let at = Instant::now();
        let past_end = vec![(Key::Text(vec![b'j'; 20]), at)];
        assert!(take_keys(&mut app, past_end).is_some());
        let (top, last) = body_ends(&mut app);
        assert!(top.contains("qrow-04"), "settled bottom page:\n{top}");
        assert!(last.contains("qrow-09"), "last row visible:\n{last}");
        let up = vec![(Key::Text(b"k".to_vec()), at)];
        assert!(take_keys(&mut app, up).is_some());
        let (top, last) = body_ends(&mut app);
        assert!(top.contains("qrow-03"), "one line up:\n{top}");
        assert!(last.contains("qrow-08"), "page moved:\n{last}");
    }

    /// R-B4: revocation never turns the rest of a refused stream into browse keys.
    #[test]
    fn a_held_entry_stays_held_when_home_is_revoked() {
        let root = Root::new("revoke-held");
        let dir = root.0.join("sessions").join("api");
        let (mut app, _reader) = housed(&root);
        app.take(Reading::Owner, Instant::now());
        let _busy = crate::store::open(&dir)
            .console_writer()
            .expect("other writer");
        app.write(true);
        assert!(app.held.is_some(), "GUARD refused entry is HELD");
        std::fs::remove_file(dir.join("meta")).expect("home becomes unproven");
        let why = "home revoked";
        app.take(Reading::NotOwner(why.to_owned()), Instant::now());
        assert_eq!(app.held.as_deref(), Some(why), "revocation updates HELD");
        let q = vec![(Key::Text(b"q".to_vec()), Instant::now())];
        assert_eq!(take_keys(&mut app, q), Some(false), "q stays swallowed");
        assert_eq!(app.held.as_deref(), Some(why));
        let retry = vec![(Key::Enter, Instant::now())];
        assert_eq!(take_keys(&mut app, retry), Some(true));
        assert!(app.held.is_some(), "unproven retry remains HELD");
    }

    /// R-B5/R-B7: a revoked writer releases, keeps its draft, and holds later text.
    #[test]
    fn a_revoked_writer_releases_the_lease_and_holds_its_kept_draft() {
        let root = Root::new("revoke-write");
        let dir = root.0.join("sessions").join("api");
        let (mut app, _reader) = housed(&root);
        let at = writing(&mut app);
        assert_eq!(app.compose(Key::Text(b"keep me".to_vec()), at), Some(()));
        std::fs::remove_file(dir.join("meta")).expect("home becomes unproven");
        let why = "home revoked";
        app.take(Reading::NotOwner(why.to_owned()), Instant::now());
        assert!(!app.composing(), "revocation ends writing");
        drop(
            crate::store::open(&dir)
                .console_writer()
                .expect("lease released"),
        );
        assert_eq!(app.held.as_deref(), Some(why), "writer lands HELD");
        assert_eq!(
            app.inputs.values().next().expect("input").draft(),
            "keep me"
        );
        let q = vec![(Key::Text(b"q".to_vec()), Instant::now())];
        assert_eq!(take_keys(&mut app, q), Some(false), "q stays swallowed");
        assert_eq!(
            app.inputs.values().next().expect("input").draft(),
            "keep me"
        );
        let retry = vec![(Key::Enter, Instant::now())];
        assert_eq!(take_keys(&mut app, retry), Some(true));
        assert!(app.held.is_some(), "unproven retry remains HELD");
        assert_eq!(
            app.inputs.values().next().expect("input").draft(),
            "keep me"
        );
    }

    /// R-B5/R-B7: admission revocation keeps even the line Enter consumed.
    #[test]
    fn an_admission_revocation_keeps_the_entered_line_only_in_memory() {
        let root = Root::new("revoke-enter");
        let dir = root.0.join("sessions").join("api");
        let (mut app, _reader) = housed(&root);
        let at = writing(&mut app);
        assert_eq!(app.compose(Key::Text(b"keep me".to_vec()), at), Some(()));
        meta(&root, &ID.replace("1234", "bbbb"), "colead");
        assert_eq!(app.compose(Key::Enter, at), Some(()));
        assert!(!app.composing(), "refused admission releases the lease");
        drop(crate::store::open(&dir).console_writer().expect("released"));
        assert_eq!(
            app.inputs.values().next().expect("input").draft(),
            "keep me"
        );
        assert_eq!(
            app.held.as_deref(),
            Some("the session was replaced or renamed")
        );
        let q = vec![(Key::Text(b"q".to_vec()), Instant::now())];
        assert_eq!(take_keys(&mut app, q), Some(false), "q stays swallowed");
        assert_eq!(super::submit::restore(&dir), super::submit::Draft::Nothing);
        assert!(matches!(
            crate::store::open(&dir).events_source(),
            crate::store::SourceRead::Absent
        ));
        let body = std::fs::remove_dir(dir.join("messages")).map_err(|why| why.kind());
        assert_eq!(body, Err(std::io::ErrorKind::NotFound), "no body written");
    }
}
