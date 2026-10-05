//! `ae app`: the Side Quiet layout — the fleet in a sidebar, the selected
//! session's Overview or Agents beside it, and the selected session's chat.
//!
//! The pure halves live in the submodules: [`fleet`] folds the sidebar rows,
//! [`overview`] the Overview tab, [`model`] the browse reducer and [`draw`] the
//! cells. Each reads only what the existing owners already computed. The
//! world is read on the [`loader`]'s one background thread, through those
//! owners; this file draws what it last answered, so no key waits on a read.
//! The ONE write is the home composer's ask or close, through the chat's own
//! admission path (`term::submit_ask`, `submit::close_owned`); nothing else
//! here writes into any session.

use std::collections::BTreeMap;
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

use loader::{Answer, Request, Wake};

mod backend;
pub mod draw;
pub mod fleet;
mod lane;
#[cfg(test)]
mod lane_spec;
mod loader;
pub mod model;
pub mod overview;
mod tty;

/// What `ae app` says, on stderr, when it has no terminal to draw on.
pub const NO_TERMINAL: &str = "ae app draws a terminal UI; use ae chat to print the lane\n";

/// The usage text.
pub const USAGE: &str = "Usage: ae app [session]\n\n  session   the home session, whose chat you write to (default: the session this pane belongs to)\n";

/// How often the fleet, home and the selection are read again.
const REFRESH: Duration = Duration::from_secs(crate::board::follow::POLL_SECS);
/// How long a wait for a key lasts before the size and the Esc bound are read.
const TICK: Duration = Duration::from_millis(100);
/// The most of ae's own notices the home lane shows.
const NOTICES: usize = 5;

/// One session's last read lane, and what it was read as.
struct Shown {
    /// The `session_id` the reading console was bound to.
    id: String,
    seq: u64,
    lane: Rc<Lane>,
}

/// Home's lane as drawn — its last read with ae's own notices merged in by
/// time — and what it was merged from, so a show rebuilds it only when the
/// read or the notices changed.
struct Merged {
    base: Option<Rc<Lane>>,
    notices: u64,
    lane: Rc<Lane>,
}

/// Everything the app was answered, and the browse and compose state.
struct App {
    home: Option<String>,
    server: Option<ServerId>,
    model: model::Model,
    fleet: fleet::Fleet,
    world: World,
    dirs: BTreeMap<String, PathBuf>,
    ids: BTreeMap<String, String>,
    facts: BTreeMap<String, fleet::Facts>,
    needs: BTreeMap<String, Section>,
    memos: BTreeMap<String, Result<Vec<brief::Filed>, String>>,
    /// The home session's console, opened by the reader: every ask goes
    /// through it, and it is never read here.
    home_console: Option<Console>,
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
    /// The home lead pair as its meta names it now, main first.
    pair: Vec<String>,
    /// The home lead pair this app composes for, fixed when it opened: a
    /// changed pair refuses the ask rather than sending it elsewhere.
    seats: Vec<Seat>,
    /// The chat's own input step machine; `None` when there is no usable pair.
    input: Option<Input>,
    /// This app's own pane, which must be the one that owns the input.
    me: Option<String>,
    /// Why the home composer only reads.
    read_only: String,
    composing: bool,
    /// ae's own lines — refusals and outcomes — shown in the home lane.
    notices: Vec<Item>,
    /// Bumped with every notice, so the merged home lane knows it is stale.
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
}

impl App {
    /// An app that has read nothing yet: home is selected, and its pair and
    /// console arrive from the reader.
    fn new(
        home: Option<String>,
        server: Option<ServerId>,
        me: Option<String>,
        ask: Option<Sender<Request>>,
    ) -> Self {
        let fleet = fleet::Fleet {
            rows: Vec::new(),
            home: home.clone(),
        };
        let read_only = home
            .as_ref()
            .map_or_else(String::new, |_| "ownership not read yet".to_owned());
        Self {
            home,
            server,
            model: model::Model::new(&fleet),
            fleet,
            world: World::new(Timestamp::now(), Vec::new()),
            dirs: BTreeMap::new(),
            ids: BTreeMap::new(),
            facts: BTreeMap::new(),
            needs: BTreeMap::new(),
            memos: BTreeMap::new(),
            home_console: None,
            shown: BTreeMap::new(),
            looks: BTreeMap::new(),
            lane: Rc::default(),
            merged: None,
            loading: true,
            fleeted: false,
            overview: overview::Overview::default(),
            pair: Vec::new(),
            seats: Vec::new(),
            input: None,
            me,
            read_only,
            composing: false,
            notices: Vec::new(),
            noticed: 0,
            draft_view: View {
                rows: Vec::new(),
                cursor_row: 0,
                before: String::new(),
                anchor: String::new(),
            },
            draft: String::new(),
            layout: draw::Layout::default(),
            ask,
            focused: None,
            deferred: Vec::new(),
        }
    }

    /// Fold one answer from the reader.
    fn answer(&mut self, answer: Answer) {
        match answer {
            Answer::Home(home) => self.housed(home),
            Answer::Fleet(read) => self.absorb(read),
            Answer::Look { name, look, zone } => drop(self.looks.insert(name, (look, zone))),
            Answer::Owned { reading, at, draft } => self.take(reading, at, draft),
            Answer::View(view) => self.viewed(view),
        }
    }

    /// The home session as the reader opened it: the console every ask goes
    /// through and the lead pair the composer writes to.
    fn housed(&mut self, home: Option<(Console, Result<Vec<Seat>, String>)>) {
        let Some((console, pair)) = home else {
            self.read_only = String::new();
            return;
        };
        match pair {
            Ok(seats) => {
                self.pair = names(&seats);
                self.input = (!seats.is_empty()).then(|| Input::new(self.pair.clone()));
                self.seats = seats;
            }
            Err(why) => self.read_only = format!("input off: {why}"),
        }
        self.home_console = Some(console);
    }

    /// Fold one read of the fleet: a session that left it, or whose recorded
    /// identity changed, takes its last lane and look with it.
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
        self.fleet = read.fleet;
        self.memos = read.memos;
        if let Some(pair) = read.pair {
            self.pair = pair;
        }
        self.fleeted = true;
        if self.model.selected().is_none() {
            self.model = model::Model::new(&self.fleet);
        }
        self.model.reconcile(&self.fleet);
        self.evict();
        self.show();
    }

    /// One read of a session's lane. A lane read as an identity its session
    /// no longer records, or older than the one shown, is dropped; home is
    /// bound to the incarnation it opened.
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
        if let (Some(seats), Some(input)) = (view.roster, self.input.as_mut()) {
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

    /// Ask the reader to read home again: this app wrote into it.
    fn reread(&self) {
        if let (Some(ask), Some(home)) = (&self.ask, &self.home) {
            let _ = ask.send(Request::Reread(home.clone()));
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

    /// Hand one ownership reading, completed at `at`, to the composer; a
    /// promotion puts back the kept `draft` read with it, as the chat does.
    fn take(&mut self, reading: Reading, at: Instant, draft: Option<submit::Draft>) {
        let Some(input) = &mut self.input else {
            return;
        };
        self.read_only = match &reading {
            Reading::NotOwner(why) => why.clone(),
            Reading::Unknown => UNREAD.to_owned(),
            Reading::Owner => String::new(),
        };
        let was = input.taking();
        let _ = input.tick(reading, at);
        if !was && input.taking() {
            let restored = input.restore(draft.unwrap_or(submit::Draft::Nothing));
            self.effects(restored);
        }
        if !self.input.as_ref().is_some_and(Input::taking) {
            self.composing = false;
        }
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
        if !self.fleeted {
            (self.lane, self.overview) = (Rc::default(), overview::Overview::default());
            self.loading = true;
            return;
        }
        let empty = SessionEntry::new(&name, Status::Unknown);
        let entry = self.entry(&name).unwrap_or(&empty).clone();
        if !self.dirs.contains_key(&name) {
            let gap = format!("{name} has no record directory ae can read");
            self.lane = Rc::new(Lane {
                items: Vec::new(),
                coverage: vec![gap.clone()],
            });
            self.loading = false;
            self.overview = overview::of(&entry, None, Err(gap), now);
            return;
        }
        let shown = self.shown.get(&name).map(|shown| Rc::clone(&shown.lane));
        self.loading = shown.is_none();
        self.lane = if self.home.as_deref() == Some(name.as_str()) {
            self.merge(shown)
        } else {
            shown.unwrap_or_default()
        };
        let memo = self.memos.get(&name).map_or_else(
            || Err("memo not read yet".to_owned()),
            |memo| memo.as_deref().map_err(String::clone),
        );
        self.overview = overview::of_filed(&entry, self.needs.get(&name), memo, now);
    }

    /// Home's lane `base` with ae's own notices in it by time, built once
    /// per read and per notice rather than on every show.
    fn merge(&mut self, base: Option<Rc<Lane>>) -> Rc<Lane> {
        let same = |merged: &&Merged| {
            merged.notices == self.noticed
                && merged.base.as_ref().map(Rc::as_ptr) == base.as_ref().map(Rc::as_ptr)
        };
        if let Some(merged) = self.merged.as_ref().filter(same) {
            return Rc::clone(&merged.lane);
        }
        let mut lane = base.as_deref().cloned().unwrap_or_default();
        lane.items.extend(self.notices.iter().cloned());
        lane.items.sort_by_key(|item| item.micros);
        let lane = Rc::new(lane);
        self.merged = Some(Merged {
            base,
            notices: self.noticed,
            lane: Rc::clone(&lane),
        });
        lane
    }

    fn entry(&self, name: &str) -> Option<&SessionEntry> {
        self.world.sessions.iter().find(|entry| entry.name == name)
    }

    /// Whether Enter may start a line: this app owns the home input.
    fn can_compose(&self) -> bool {
        self.input.as_ref().is_some_and(Input::taking)
    }

    /// One key while composing: Esc keeps the draft and browses, ^C quits,
    /// anything else is the chat's own input. `None` quits.
    fn compose(&mut self, key: Key, origin: Instant) -> Option<()> {
        match key {
            Key::Escape => self.composing = false,
            Key::Interrupt => return None,
            key => {
                let effects = self
                    .input
                    .as_mut()
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
        if mouse.kind == MouseKind::Drag {
            return self.drag(mouse);
        }
        let ended = self.model.let_go();
        match mouse.kind {
            MouseKind::Click => self.click(mouse) || ended,
            MouseKind::WheelUp | MouseKind::WheelDown => {
                if !self.layout.in_chat(mouse) {
                    return ended;
                }
                self.model.wheel(
                    mouse.kind == MouseKind::WheelUp,
                    self.layout.page_rows,
                    self.layout.max_scroll,
                );
                true
            }
            MouseKind::Drag | MouseKind::Release => ended,
        }
    }

    /// A press: grab the border under it, else act on the target drawn there.
    fn click(&mut self, mouse: Mouse) -> bool {
        if let Some((edge, at)) = self.layout.grab(mouse) {
            let from = match edge {
                model::Edge::Sidebar => mouse.column,
                model::Edge::List => mouse.row,
            };
            self.model.grab(model::Drag { edge, from, at });
            return true;
        }
        let Some(hit) = self.layout.hit(mouse) else {
            return false;
        };
        let was_writing = self.composing;
        let act = match hit {
            draw::Hit::Session(name) => {
                let Some(act) = self.model.select_name(&self.fleet, &name) else {
                    return false;
                };
                self.composing = false;
                act
            }
            draw::Hit::Tab(tab) => {
                self.composing = false;
                self.model.show_tab(tab)
            }
            draw::Hit::Compose if !self.composing => {
                self.model
                    .key(model::Key::Compose, &self.fleet, self.can_compose(), true)
            }
            draw::Hit::Compose => return false,
        };
        apply(self, &act).unwrap_or(false) || was_writing != self.composing
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
            self.notices.push(Item {
                micros: Timestamp::now().epoch().saturating_mul(1_000_000),
                kind: Kind::Said {
                    who: "ae".to_owned(),
                },
                body: line,
                record: None,
            });
            self.noticed = self.noticed.wrapping_add(1);
        }
        let over = self.notices.len().saturating_sub(NOTICES);
        self.notices.drain(..over);
        self.show();
    }

    /// The ownership the admission re-proves: the pair this app opened with,
    /// and this pane still the owner.
    fn owns(&self, console: &Console) -> Result<(), String> {
        term::still(&self.seats, console.seats())?;
        term::owns(console, self.server.as_ref(), self.me.as_deref())
            .unwrap_or_else(|| Err(UNREAD.to_owned()))
    }

    /// One ask of `seat`, through the chat's own admission and tracked path.
    fn ask(&self, raw: &[u8], seat: &str, body: String) -> String {
        let Some(console) = &self.home_console else {
            return "refused: no home session".to_owned();
        };
        match term::submit_ask(console, || self.owns(console), raw, seat, body) {
            Ok((_, outcome)) => crate::console::input::outcome_line(&outcome, seat),
            Err(why) => format!("refused: {why}"),
        }
    }

    /// `/close`: this app's own open ask, through the chat's admission.
    fn close(&self, id: Option<&str>) -> String {
        let Some(console) = &self.home_console else {
            return "refused: no home session".to_owned();
        };
        match submit::close_owned(console.dir(), console.name(), id, || self.owns(console)) {
            Ok(()) => "closed an ask".to_owned(),
            Err(why) => format!("refused: {why}"),
        }
    }

    /// The composer line for the selection.
    fn composer(&self) -> draw::Composer<'_> {
        let speaker = self.input.as_ref().map_or_else(
            || self.pair.first().map_or("", String::as_str),
            Input::speaker,
        );
        match &self.home {
            None => draw::Composer::NoHome,
            Some(home) if self.model.selected() != Some(home.as_str()) => {
                draw::Composer::Foreign { home, speaker }
            }
            Some(home) if self.can_compose() => draw::Composer::Home {
                home,
                speaker,
                view: self.composing.then_some(&self.draft_view),
                draft: &self.draft,
            },
            Some(_) => draw::Composer::ReadOnly {
                why: &self.read_only,
            },
        }
    }

    /// One frame into `buf`, then the chat scroll bounded by what it held.
    /// Before the fleet is read, the selection's header stands on what its
    /// name alone says.
    fn frame(&mut self, buf: &mut Buffer) {
        if let (Some(input), Some(home)) = (&self.input, &self.home) {
            let area = buf.area;
            let width = draw::draft_width(area, self.model.split(), home, input.speaker());
            let size = Size {
                width,
                height: usize::from(area.height),
            };
            (self.draft_view, self.draft) = (input.bare_view(size), input.draft());
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
        // Held keys replay from the scroll they were read at, as one read.
        if self.deferred.is_empty() {
            self.model
                .clamp_scroll(self.layout.max_scroll, self.layout.page_rows);
        }
    }
}

/// What the app says when tmux did not answer who owns the input.
const UNREAD: &str = "tmux did not answer who owns the input";

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
    let me = doors::calling_pane_id();
    let (wake, wakes) = mpsc::channel();
    let reader = loader::Reader::new(root, home.clone(), server.clone(), me.clone());
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
    let mut app = App::new(home, server, me, Some(ask));
    let mut keys = Keys::app();
    let mut dirty = true;
    loop {
        if let Some(now) = tty.size().filter(|now| *now != size) {
            size = now;
            terminal.backend_mut().resize(size);
            dirty = true;
        }
        if dirty {
            terminal.draw(|frame| app.frame(frame.buffer_mut()))?;
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
            let acted = if app.composing {
                app.compose(key, origin).map(|()| true)?
            } else {
                browse(app, &key)?
            };
            acted || ended
        };
    }
    Some(redraw)
}

/// Take one key: `Some(redraw)`, or `None` to quit.
fn browse(app: &mut App, key: &Key) -> Option<bool> {
    let mut redraw = false;
    for key in model::browse_keys(key) {
        let act = app.model.key(key, &app.fleet, app.can_compose(), true);
        redraw |= apply(app, &act)?;
    }
    Some(redraw)
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
            app.composing = true;
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
    use super::{App, UNREAD, USAGE, browse, facts_of, fleet, run};
    use crate::app::fleet::{Counts, Fleet, Row};
    use crate::app::model::{Key as Browse, Model};
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

    /// [`app`] on `root`'s home `api`, opened by the reader as `run` opens it.
    fn housed(root: &Root) -> (App, Reader) {
        let mut reader = Reader::new(root.0.clone(), Some("api".to_owned()), None, None);
        let mut app = app(Some("api"));
        app.answer(reader.open_home());
        (app, reader)
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

    /// Leaving writing on the current tab must repaint immediately, keeping the draft.
    #[test]
    fn a_same_tab_mouse_click_ends_writing_and_requests_redraw() {
        let root = Root::new("mouse-same-tab");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.world = World::new(Timestamp::now(), vec![entry("api", Status::Running, None)]);
        let at = Instant::now();
        app.take(Reading::Owner, at, None);
        app.composing = true;
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
        assert!(!app.composing);
        assert_eq!(app.model.tab(), crate::app::model::Tab::Overview);
        assert_eq!(
            app.input.as_ref().expect("home input").draft(),
            "kept draft"
        );
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

    /// The home composer is the chat's own input: an ask this app cannot
    /// prove it owns is refused BEFORE anything is written and says so in
    /// the home lane; Esc keeps the draft and browses.
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test reads its isolated event ledger"
    )]
    fn an_unproven_ask_is_refused_and_writes_nothing() {
        let root = Root::new("ask");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.dirs
            .insert("api".to_owned(), root.0.join("sessions").join("api"));
        let begun = Instant::now();
        let input = app.input.as_mut().expect("a lead pair makes an input");
        let _ = input.tick(Reading::Owner, begun);
        assert!(app.can_compose(), "the owner composes");
        let typed = begun + Duration::from_millis(5);
        app.composing = true;
        for key in [Key::Text(b"check the scopes".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, typed), Some(()));
        }
        let notice = &app.notices.last().expect("the outcome is said").body;
        assert!(
            notice.starts_with("refused: ") && notice.contains(super::UNREAD),
            "{notice}"
        );
        let journal = root.0.join("sessions").join("api").join("events.jsonl");
        let written = std::fs::read_to_string(&journal).unwrap_or_default();
        assert!(!written.contains("\"ask\""), "nothing was asked: {written}");
        assert!(
            framed(&mut app).contains("refused: "),
            "the home lane shows it"
        );
        assert_eq!(app.compose(Key::Text(b"later".to_vec()), typed), Some(()));
        assert_eq!(app.compose(Key::Escape, typed), Some(()));
        assert!(!app.composing, "Esc browses");
        let shown = framed(&mut app);
        assert!(shown.contains("to api › lead   later"), "{shown}");
        assert!(shown.contains("draft kept · Enter writes"));
        assert_eq!(app.compose(Key::Interrupt, typed), None, "^C quits");
    }

    /// `/open` in the app names why it opens nothing, as the docs say, once
    /// the home roster is read: the app selects no pane, `ae chat` does.
    #[test]
    fn open_is_refused_with_the_apps_own_reason() {
        let root = Root::new("open");
        let (mut app, mut reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.dirs
            .insert("api".to_owned(), root.0.join("sessions").join("api"));
        app.answer(reader.view("api").expect("home reads"));
        let begun = Instant::now();
        let input = app.input.as_mut().expect("a lead pair makes an input");
        let _ = input.tick(Reading::Owner, begun);
        app.composing = true;
        let typed = begun + Duration::from_millis(5);
        for key in [Key::Text(b"/open lead".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, typed), Some(()));
        }
        let notice = &app.notices.last().expect("the refusal is said").body;
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

    /// #15: no server to ask means tmux did not answer who owns the input.
    #[test]
    fn an_unanswered_ownership_says_tmux_did_not_answer() {
        let root = Root::new("own");
        let (mut app, _reader) = housed(&root);
        let console = app.home_console.as_ref().expect("home opened");
        let reading = loader::reading(console, None, None);
        app.take(reading, Instant::now(), None);
        assert_eq!(app.read_only, UNREAD);
    }

    /// #16-18: only the promotion to owner brings the kept draft back, once
    /// — a later owner reading carrying the disk draft never overwrites the
    /// draft being edited — and losing the input ends composing; a demotion
    /// then a promotion restores again.
    #[test]
    fn only_a_promotion_brings_the_kept_draft_back_once() {
        let root = Root::new("take");
        let dir = root.0.join("sessions").join("api");
        crate::store::open(&dir)
            .publish_console_draft(b"kept line")
            .expect("a kept draft");
        let (mut app, _reader) = housed(&root);
        let at = Instant::now();
        let kept = || Some(crate::console::submit::restore(&dir));
        let draft = |app: &App| app.input.as_ref().map(Input::draft).unwrap_or_default();
        app.take(Reading::Unknown, at, None);
        assert_eq!(draft(&app), "");
        app.take(Reading::Owner, at, kept());
        assert_eq!(draft(&app), "kept line", "the promotion restores");
        app.composing = true;
        assert_eq!(app.compose(Key::Text(b" edited".to_vec()), at), Some(()));
        app.take(Reading::Owner, at, kept());
        assert_eq!(draft(&app), "kept line edited", "no second restore");
        assert!(app.composing, "the owner keeps composing");
        app.take(Reading::NotOwner("owned by window @2".to_owned()), at, None);
        assert!(!app.composing, "losing the input ends composing");
        app.take(Reading::Owner, at, kept());
        assert_eq!(draft(&app), "kept line", "a re-promotion restores again");
        app.take(Reading::NotOwner("owned by window @2".to_owned()), at, None);
        app.take(Reading::Owner, at, None);
        assert_eq!(draft(&app), "", "a promotion with no draft restores none");
    }

    /// I5: an owner reading delivered late still counts from when it was
    /// READ: keys typed before it are dropped, keys after it are taken.
    #[test]
    fn a_late_owner_reading_counts_from_when_it_was_read() {
        let root = Root::new("late");
        let (mut app, _reader) = housed(&root);
        let begun = Instant::now();
        let read = begun + Duration::from_millis(10);
        app.take(Reading::Owner, read, None);
        app.composing = true;
        let before = begun + Duration::from_millis(5);
        let after = begun + Duration::from_millis(15);
        assert_eq!(app.compose(Key::Text(b"early".to_vec()), before), Some(()));
        assert_eq!(app.compose(Key::Text(b"late".to_vec()), after), Some(()));
        let draft = app.input.as_ref().map(Input::draft).unwrap_or_default();
        assert_eq!(draft, "late", "only keys after the reading: {draft}");
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

    /// #26/#35: a home app that does not own the input is read-only and says
    /// why (docs/app.md Ownership).
    #[test]
    fn a_home_app_that_does_not_own_the_input_is_read_only() {
        let root = Root::new("readonly");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        let text = framed(&mut app);
        assert!(text.contains("ownership not read yet"), "{text}");
        assert!(!text.contains("to api ›"), "{text}");
    }

    /// #29: a foreign view says typing goes home (docs/app.md Ownership).
    #[test]
    fn a_foreign_view_says_typing_goes_home() {
        let root = Root::new("foreign");
        let (mut app, _reader) = housed(&root);
        let mut fleet = one_row(Some("api"));
        fleet.rows.push(row("web", 2, false));
        app.fleet = fleet;
        app.model = Model::new(&app.fleet);
        let _ = app.model.key(Browse::Digit(2), &app.fleet, false, true);
        assert!(framed(&mut app).contains("typing writes to api › lead"));
    }

    /// #27/#28: `/close` answers with the chat admission's own refusal: no
    /// tmux to prove ownership, so tmux did not answer (submit.rs `close_owned`
    /// proves ownership before the ask); no home session, so no home.
    #[test]
    fn close_reports_the_chats_own_admission_answer() {
        let root = Root::new("close");
        let (mut app, _reader) = housed(&root);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.dirs
            .insert("api".to_owned(), root.0.join("sessions").join("api"));
        let begun = Instant::now();
        let input = app.input.as_mut().expect("a lead pair makes an input");
        let _ = input.tick(Reading::Owner, begun);
        app.composing = true;
        let typed = begun + Duration::from_millis(5);
        for key in [Key::Text(b"/close".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, typed), Some(()));
        }
        let notice = &app.notices.last().expect("the outcome is said").body;
        assert_eq!(notice, &format!("refused: {}", super::UNREAD));
        app.home_console = None;
        assert_eq!(app.close(None), "refused: no home session");
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
        assert!(browse(&mut app, &Key::Text(b"x".to_vec())).is_some());
        assert!(browse(&mut app, &Key::Text(b"q".to_vec())).is_none());
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
            pair: None,
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
            pair: None,
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
            pair: None,
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
        let begun = Instant::now();
        let input = app.input.as_mut().expect("a lead pair makes an input");
        let _ = input.tick(Reading::Owner, begun);
        app.composing = true;
        let typed = begun + Duration::from_millis(5);
        assert_eq!(app.compose(Key::Text(b"/close".to_vec()), typed), Some(()));
        assert!(asks.try_iter().next().is_none(), "typing writes nothing");
        assert_eq!(app.compose(Key::Enter, typed), Some(()));
        let rereads: Vec<String> = asks
            .try_iter()
            .filter_map(|request| match request {
                loader::Request::Reread(name) => Some(name),
                loader::Request::Focus(_) => None,
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
        for first in [None, Some(loader::Wake::Answer(Box::new(quiet)))] {
            let mut app = app(None);
            app.composing = true;
            let mut keys = crate::console::input::Keys::app();
            let typed = Instant::now()
                .checked_sub(Duration::from_secs(1))
                .expect("an instant a second ago");
            assert!(keys.feed(b"\x1b", typed).is_empty(), "ESC waits");
            assert_eq!(super::drain(&mut app, &mut keys, &wakes, first), Some(true));
            assert!(!app.composing, "Esc browses");
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
        app.notices = vec![said("tie notice", 20), said("first", 0)];
        let bodies = |lane: &Lane| -> Vec<String> {
            lane.items.iter().map(|item| item.body.clone()).collect()
        };
        let merged = app.merge(Some(Rc::clone(&base)));
        assert_eq!(
            bodies(&merged),
            ["first", "early", "tie read", "tie notice", "late"]
        );
        assert_eq!(merged.coverage, ["gap"]);
        let again = app.merge(Some(Rc::clone(&base)));
        assert!(Rc::ptr_eq(&merged, &again), "rebuilt with nothing new");
        let read = Rc::new((*base).clone());
        let reread = app.merge(Some(Rc::clone(&read)));
        assert!(!Rc::ptr_eq(&again, &reread), "a new read is merged afresh");
        app.effects(vec![Effect::Print("said".to_owned())]);
        let noticed = app.merge(Some(Rc::clone(&read)));
        assert!(!Rc::ptr_eq(&reread, &noticed), "a new notice is merged");
        assert_eq!(bodies(&noticed).last().map(String::as_str), Some("said"));
        let unread = app.merge(None);
        assert_eq!(bodies(&unread).len(), 3, "notices alone before a read");
        assert!(Rc::ptr_eq(&unread, &app.merge(None)));
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
                    loader::Request::Reread(_) => None,
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
}
