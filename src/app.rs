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
//! this app writes it; nothing else here writes into any session. Opening a
//! seat writes nothing either: it hands one tmux client to the seat's pane,
//! through the chat's own proof (`term::prove_open`).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{IsTerminal as _, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Position;
use ratatui_core::terminal::Terminal;

use crate::board::terminal_text;
use crate::console::input::{Effect, Input, Key, Keys, Mouse, MouseKind, Reading, Size, View};
use crate::console::lane::{Item, Kind, Lane, Seat};
use crate::console::needs::{SeatRef, Section};
use crate::console::open::{self, Move, Refusal};
use crate::console::{self, Console, submit, term, toggle};
use crate::digest::{SessionEntry, Status};
use crate::inventory::ServerId;
use crate::listing::World;
use crate::time::Timestamp;
use crate::{brief, doors, theme, tmux, transport};

use loader::{Answer, Binding, PeekAsk, Request, Wake};
use peek::Body;

mod backend;
pub mod draw;
#[cfg(test)]
#[path = "../tests/it/app_early_spec.rs"]
mod early_spec;
pub mod fleet;
#[cfg(test)]
mod fleet_spec;
pub mod handover;
mod journals;
mod lane;
#[cfg(test)]
mod lane_spec;
mod loader;
pub mod model;
#[cfg(test)]
mod order_spec;
pub mod overview;
pub mod peek;
pub mod select;
#[cfg(test)]
mod select_spec;
mod settings;
mod tty;
pub mod upgrade;
#[cfg(test)]
#[path = "../tests/it/app_view_state_spec.rs"]
mod view_spec;

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
/// How long an armed key waits for its second press.
const QUIT_WINDOW: Duration = Duration::from_secs(2);
/// How long a refused paste or an outcome word stays on the hint row.
const FLASH_WINDOW: Duration = Duration::from_secs(5);
/// The most UTF-8 bytes one copy writes to the terminal clipboard outside tmux;
/// more is refused, never cut.
const CLIP_MAX: usize = 65536;
/// The most replaced layouts kept for a click made on one of them.
const HISTORY: usize = 8;
/// The most navigation keys held for the first fleet read; the oldest go first.
const EARLY_CAP: usize = 64;
/// What the hint row says once when a key read before the fleet is dropped.
const EARLY_NOTICE: &str = "input before sessions loaded was dropped";

/// One session's last read lane, and what it was read as.
struct Shown {
    /// The `session_id` the reading console was bound to.
    id: String,
    seq: u64,
    lane: Rc<Lane>,
    /// The roster of that read, as `/open` names its seats.
    roster: Option<Vec<SeatRef>>,
}

/// What the last frame painted of the selection's seats, frozen when it was
/// drawn: a seat key acts on this and on nothing a later read changed.
#[derive(Debug, Clone)]
struct Drawn {
    session: String,
    /// The incarnation the roster came from.
    uuid: String,
    /// Each drawn seat, with its identity in that roster when it held one.
    rows: Vec<(String, Option<SeatRef>)>,
    focused: Option<String>,
}

impl Drawn {
    /// The focused seat as the frame drew it, or why there is none.
    fn seat(&self) -> Result<&SeatRef, Refusal> {
        let row = (self.rows.iter()).find(|(name, _)| Some(name) == self.focused.as_ref());
        match row {
            None => Err(Refusal::NoSeat(format!(
                "no seat is shown to open in {}",
                self.session
            ))),
            Some((name, None)) => Err(Refusal::NoSeat(format!(
                "{name} is not in the roster read for {}",
                self.session
            ))),
            Some((_, Some(seat))) => Ok(seat),
        }
    }
}

/// The one armed key: the press that completes it acts, any other disarms.
#[derive(Debug, Clone)]
struct Armed {
    at: Instant,
    what: Arm,
}

/// What a press armed.
#[derive(Debug, Clone)]
enum Arm {
    Quit,
    /// ^C over a draft that would be lost.
    Interrupt,
    /// `o` on the frame it saw: the second press opens that frame's seat.
    Open(Drawn),
}

impl Arm {
    /// The hint-row line that says what the next press does.
    fn line(&self) -> String {
        match self {
            Self::Quit => "q again to quit".to_owned(),
            Self::Interrupt => "^C again to quit - the draft is lost".to_owned(),
            Self::Open(d) => format!("o again to open {}", d.focused.as_deref().unwrap_or("")),
        }
    }
}

/// An ask the input produced, held until the frame that names its target is
/// drawn: the entry that wrote it and the line, for a writer that ended.
struct Queued {
    key: (String, String),
    entry: u64,
    raw: Vec<u8>,
    seat: String,
    body: String,
    about: String,
}

/// An ask's outcome word, shown until the lane holds the ask itself.
struct Settling {
    at: Instant,
    id: String,
    word: String,
    /// The session the ask went to: its hint row alone shows the word.
    about: String,
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

/// What the open peek panel holds of its pane, pinned to the model's
/// generation: a capture that landed for another is dropped.
#[derive(Debug, Default)]
struct PeekPane {
    generation: u64,
    /// The session and seat the panel was opened on.
    target: Option<(String, String)>,
    /// The body on screen; `None` until the first answer lands.
    shown: Option<Body>,
    /// A newer capture held back while the viewport is scrolled back.
    held: Option<Body>,
    /// When the newest capture, held or shown, was taken.
    at: Option<Instant>,
    /// The age in whole seconds the last frame painted.
    painted: Option<u64>,
}

/// A drag that began in a selectable region, from its press until its copy is
/// shipped or it is cleared.
#[derive(Debug)]
struct Held {
    sel: select::Selection,
    /// The model's [`model::Model::epoch`] when it began: any other is a panel
    /// change and ends it.
    epoch: u64,
    /// The press toggled a turn's mark: `before` is put back at the first
    /// motion, when the press turns out to be a drag.
    toggled: bool,
    before: Option<u64>,
    /// The button is still down.
    down: bool,
    stage: Stage,
}

/// Where a release's copy stands. It lives in [`Held`], so whatever ends the
/// selection ends the copy with it.
#[derive(Debug, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    None,
    /// Released: the next frame paints the final range.
    Requested,
    /// What that frame highlighted, to ship once it is written.
    Ready(String),
}

impl PeekPane {
    /// The viewport is scrolled back over captured lines: a newer capture waits.
    fn paused(&self, back: usize) -> bool {
        back > 0 && matches!(self.shown, Some(Body::Lines(_)))
    }

    /// Whole seconds since the newest capture, at `now`.
    fn age(&self, now: Instant) -> u64 {
        self.at
            .map_or(0, |at| now.saturating_duration_since(at).as_secs())
    }
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

/// A view an earlier app handed over, waiting for the fleet read that settles
/// it; `at` is when it was written.
struct Keep {
    view: handover::View,
    at: i64,
}

/// Everything the app was answered, and the browse and compose state.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent facts of one loop, each read where it is set: loading, fleeted, touched, restored, early_named, again"
)]
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
    /// Each layout a frame replaced, with the instant it did: kept until no held
    /// key or queued ask can refer to the frame it was on screen in, and never
    /// more than [`HISTORY`] of them.
    history: Vec<(Instant, draw::Layout)>,
    /// The instant of the newest layout the bound dropped.
    lost: Option<Instant>,
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
    /// The open peek panel's captures, and the generation the next open mints.
    peek: PeekPane,
    peek_generation: u64,
    /// The row order the last fleet read chose, applied when the next frame
    /// is drawn: until then keys and clicks resolve against the order the
    /// human sees.
    pending: Option<Vec<String>>,
    /// The tmux server and pane this app runs in, from tmux's own markers;
    /// `None` outside tmux.
    server: Option<ServerId>,
    pane: Option<String>,
    /// The seats the last frame drew.
    drawn: Option<Drawn>,
    /// The key the last press armed, until its window or the next key ends it.
    armed: Option<Armed>,
    /// Why a paste was not taken, until its window ends.
    flash: Option<(String, Instant)>,
    /// The lane the turn fingerprints were built from, with them.
    keyed: Option<(Rc<Lane>, Rc<[u64]>)>,
    /// The read of the browse paste that started this entry: its later
    /// fragments keep their text, nothing else read before the lease does.
    admitted: Option<Instant>,
    /// Asks waiting for the loop to deliver them.
    queued: Vec<Queued>,
    /// The outcome word of the last ask, until the lane shows it.
    settling: Option<Settling>,
    /// What the read-only composer says: `read_only`, with how to resume.
    read_line: String,
    /// Another installed core, as the last fleet read found the command link.
    upgrade: Option<upgrade::Upgrade>,
    /// When the last key, click or paste was read.
    last_input: Option<Instant>,
    /// The drag in progress, or the highlight it left.
    select: Option<Held>,
    /// The cells of the frame on screen, before any highlight.
    shot: Buffer,
    /// The last frame left a hint it did not draw: draw again.
    again: bool,
    /// The handed-over view, until a fleet read applies or drops it.
    keep: Option<Keep>,
    /// Input was read, a key or a click: no handed-over view is taken after it.
    touched: bool,
    /// A handed-over scroll is applied and its lane is not read yet.
    restored: bool,
    /// Navigation keys read before the first fleet read, with their read
    /// instants, replayed on it.
    early: Vec<(Key, Instant)>,
    /// The drop of an early key was told on the hint row.
    early_named: bool,
    /// A copy outside tmux, for the loop to write to the terminal.
    clip: Option<String>,
}

impl App {
    /// An app that has read nothing yet: home is selected. Where it runs
    /// decides nothing: the writer lease does.
    fn new(
        home: Option<String>,
        server: Option<ServerId>,
        pane: Option<String>,
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
            history: Vec::new(),
            lost: None,
            ask,
            focused: None,
            deferred: Vec::new(),
            settings_bodies: settings::SettingsBodies::default(),
            settings_generation: 0,
            peek: PeekPane::default(),
            peek_generation: 0,
            pending: None,
            server,
            pane,
            drawn: None,
            armed: None,
            flash: None,
            keyed: None,
            admitted: None,
            queued: Vec::new(),
            settling: None,
            read_line: String::new(),
            upgrade: None,
            last_input: None,
            select: None,
            shot: Buffer::default(),
            again: false,
            keep: None,
            touched: false,
            restored: false,
            early: Vec::new(),
            early_named: false,
            clip: None,
        }
    }

    /// Fold one answer from the reader.
    fn answer(&mut self, answer: Answer) {
        match answer {
            Answer::Fleet(read) => {
                self.absorb(read);
                // What was read before the fleet navigates it now, at the
                // instants it was read. A press armed by a later key stays
                // armed: the key that ended any older one has already acted.
                let (early, armed) = (std::mem::take(&mut self.early), self.armed.take());
                let _ = take_keys(self, early);
                self.armed = armed;
            }
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
                instructions,
            } => self.settled(generation, quota, config, about, instructions),
            Answer::Core(seen) => self.upgrade = seen,
            Answer::Handover { view, at } => {
                self.keep = (!self.touched).then_some(Keep { view, at });
            }
            Answer::Peek {
                generation,
                at,
                body,
            } => self.peeked(generation, at, body),
        }
    }

    /// Fold one peek capture. Only the open panel's own generation paints. A
    /// refusal or a failed capture replaces the body at once, scrolled back
    /// or not, and returns to the newest row; lines wait while the viewport
    /// is scrolled back over lines.
    fn peeked(&mut self, generation: u64, at: Instant, body: Body) {
        if self.model.peek_generation() != Some(generation) {
            return;
        }
        self.peek_sync();
        self.peek.at = Some(at);
        if matches!(body, Body::Gap(_)) {
            self.model.peek_newest();
            self.peek.held = None;
            self.peek.shown = Some(body);
        } else {
            self.peek.held = Some(body);
        }
        self.peek_adopt();
    }

    /// Start the panel's state afresh when the model's generation is not the
    /// one it holds, and drop it once the panel is closed.
    fn peek_sync(&mut self) {
        match self.model.peek_generation() {
            None => self.peek = PeekPane::default(),
            Some(generation) if generation != self.peek.generation => {
                self.peek = PeekPane {
                    generation,
                    ..PeekPane::default()
                };
            }
            Some(_) => {}
        }
    }

    /// Show the held capture unless the viewport is scrolled back over lines.
    fn peek_adopt(&mut self) {
        let back = self.model.peek_back().unwrap_or(0);
        if !self.peek.paused(back)
            && !self.holds_peek()
            && let Some(held) = self.peek.held.take()
        {
            self.peek.shown = Some(held);
        }
    }

    /// `v`: open the peek panel on the seat the last frame highlighted, as
    /// that frame drew it. A refusal is said on the hint row, opens nothing and
    /// asks the reader nothing; the proof against tmux is the reader's, every
    /// capture.
    fn open_peek(&mut self) {
        let Some(session) = self.model.selected().map(str::to_owned) else {
            let line = "refused: peek: no session is selected".to_owned();
            self.flash = Some((line, Instant::now()));
            return;
        };
        let drawn = self.drawn.clone().filter(|drawn| drawn.session == session);
        let label = (drawn.as_ref())
            .and_then(|drawn| drawn.focused.clone())
            .unwrap_or_else(|| session.clone());
        match self.peek_ask(&session, drawn.as_ref()) {
            Ok(ask) => {
                self.peek_generation = ask.generation;
                self.model.open_peek(ask.generation);
                self.peek = PeekPane {
                    generation: ask.generation,
                    target: Some((session, ask.seat.name.clone())),
                    ..PeekPane::default()
                };
                if let Some(tx) = &self.ask {
                    let _ = tx.send(Request::Peek(Some(ask)));
                }
            }
            Err(why) => {
                let line = format!("refused: peek {label}: {}", why.why());
                self.flash = Some((line, Instant::now()));
            }
        }
    }

    /// What `session`'s drawn seat asks the reader to capture, or why the
    /// panel cannot open: the checks that need no tmux read.
    fn peek_ask(&self, session: &str, drawn: Option<&Drawn>) -> Result<Box<PeekAsk>, Refusal> {
        let Some(server) = self.server.clone() else {
            return Err(Refusal::NoTmux);
        };
        if (self.entry(session)).is_some_and(|entry| entry.status == Status::Stopped) {
            return Err(Refusal::Stopped(session.to_owned()));
        }
        let Some(dir) = self.dirs.get(session) else {
            let why = format!("{session} has no record directory ae can read");
            return Err(Refusal::NoSeat(why));
        };
        let Some(drawn) = drawn else {
            return Err(Refusal::NoSeat("the view changed".to_owned()));
        };
        Ok(Box::new(PeekAsk {
            generation: self.peek_generation.wrapping_add(1),
            session: session.to_owned(),
            uuid: drawn.uuid.clone(),
            dir: dir.clone(),
            seat: drawn.seat()?.clone(),
            server,
        }))
    }

    /// Close the panel and tell the reader to stop capturing. It always tells:
    /// the model may already hold the close the key caused.
    fn close_peek(&mut self) {
        self.model.close_peek();
        self.peek = PeekPane::default();
        if let Some(ask) = &self.ask {
            let _ = ask.send(Request::Peek(None));
        }
    }

    /// A mouse event with the peek panel open: the wheel scrolls wherever it
    /// lands, a press on the drawn close label closes, a press on its body
    /// begins a selection, and any other press dies.
    fn peek_mouse(&mut self, mouse: Mouse, fresh: bool) -> bool {
        match mouse.kind {
            MouseKind::WheelUp | MouseKind::WheelDown => {
                self.model.peek_wheel(mouse.kind == MouseKind::WheelUp);
                true
            }
            MouseKind::Click if matches!(self.layout.hit(mouse), Some(draw::Hit::PeekClose)) => {
                self.close_peek();
                true
            }
            MouseKind::Click => {
                self.arm(mouse, None, fresh);
                false
            }
            MouseKind::Drag => self.stretch(mouse),
            MouseKind::Release => self.finish(mouse),
        }
    }

    /// Fold one settings answer. Only the open overlay's own generation
    /// paints: a late answer for a closed overlay is dropped. Quota rides
    /// every answer; config, about and instructions ride the first per open,
    /// then stay.
    fn settled(
        &mut self,
        generation: u64,
        quota: Vec<settings::QuotaRow>,
        config: Option<settings::ConfigView>,
        about: Option<settings::AboutFacts>,
        instructions: Option<settings::InstructionsView>,
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
        if let Some(instructions) = instructions {
            self.settings_bodies.instructions = instructions;
        }
    }

    /// Open the overlay on a fresh generation and ask the reader for it.
    fn open_settings(&mut self) {
        if self.model.settings_open() {
            return;
        }
        // The reader captures the session it was last told is selected, and a
        // drain tells it only after the keys: tell it before the open.
        self.focus();
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
        self.settle_keep(read.scanned);
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

    /// Settle the handed-over view with this read: applied when it lists the
    /// session as the incarnation the view was written for, dropped when it is
    /// stale, proves the session gone or replaced, and kept for a later read
    /// when an incomplete one cannot say.
    fn settle_keep(&mut self, scanned: bool) {
        let Some(keep) = self.keep.take() else {
            return;
        };
        let name = keep.view.session.as_str();
        let row = self.fleet.rows.iter().any(|row| row.name == name);
        let id = self.ids.get(name).map(String::as_str);
        let now = Timestamp::now().epoch();
        match handover::decide(keep.at, now, &keep.view, row, id, scanned) {
            handover::Verdict::Apply => {
                self.model.adopt(&self.fleet, &keep.view);
                self.restored = true;
            }
            handover::Verdict::Defer => self.keep = Some(keep),
            handover::Verdict::Discard => {}
        }
    }

    /// What this app hands to the one that replaces it in `pane`, at `now`:
    /// the view of its selection, bound to the uuid the last read recorded.
    fn handover(&self, pane: &str, version: &str, now: i64) -> Option<handover::Handover> {
        let name = self.model.selected()?;
        let view = self.model.view(self.ids.get(name)?)?;
        Some(handover::Handover {
            pane: pane.to_owned(),
            version: version.to_owned(),
            at: now,
            view,
        })
    }

    /// Publish [`App::handover`] on the home session, best effort: a restart
    /// never waits on it and never fails for it.
    fn publish_view(&self, restart: &toggle::Restart, version: &str) {
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
        let how = "(Agents tab: n/p, oo opens a seat)";
        let restored = input
            .draft()
            .is_empty()
            .then(|| input.restore_for(draft, how));
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
        let stopped = format!("{name} is stopped");
        self.read_line = if self.read_only == stopped {
            format!("{stopped}; ae {name} resumes it")
        } else {
            self.read_only.clone()
        };
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
        // The browse paste that started this entry keeps every fragment.
        let origin = match (&key, &self.lease) {
            (Key::Pasted(_), Some(lease)) if self.admitted == Some(origin) => lease.at,
            _ => origin,
        };
        match key {
            Key::Escape => self.write(false),
            Key::Interrupt if self.protects() => {
                return self.press(Arm::Interrupt, origin).is_none().then_some(());
            }
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
            Key::Pasted(_) => self.refuse_paste("not writing - Esc browses first"),
            _ => return Some(false),
        }
        Some(true)
    }

    /// Whether a ^C here would lose a draft only this process holds.
    fn protects(&self) -> bool {
        let input = self.writer.as_ref().map(Binding::key);
        let input = input.and_then(|key| self.inputs.get(&key));
        input.is_some_and(|input| !input.draft().is_empty())
    }

    /// Press `what`: the arm it completes (the same kind, left within the
    /// window by the key before), else it arms and nothing completes.
    fn press(&mut self, what: Arm, origin: Instant) -> Option<Arm> {
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
    fn refuse_paste(&mut self, why: &str) {
        self.flash = Some((format!("paste not taken: {why}"), Instant::now()));
    }

    /// The fingerprints of the lane's turns, in item order: built once for
    /// each lane, so a frame, a key and a click share one build.
    fn keys(&mut self) -> Rc<[u64]> {
        let same = |(lane, _): &&(Rc<Lane>, Rc<[u64]>)| Rc::ptr_eq(lane, &self.lane);
        if let Some((_, keys)) = self.keyed.as_ref().filter(same) {
            return Rc::clone(keys);
        }
        let keys: Rc<[u64]> = self.lane.items.iter().map(lane::key).collect();
        self.keyed = Some((Rc::clone(&self.lane), Rc::clone(&keys)));
        keys
    }

    /// `[` / `]`: mark the next older / newer turn the last frame produced and
    /// bring it into view; with none marked, the newest turn the frame shows.
    /// An edge is a no-op that keeps the mark. Whether anything moved.
    fn step_turn(&mut self, older: bool) -> bool {
        let page = self.layout.page_rows;
        let scroll = self.model.scroll_rows(page);
        let turns = &self.layout.turns;
        let marked = (self.model.turn()).and_then(|mark| turns.iter().position(|t| t.0 == mark));
        let next = match marked {
            Some(at) if older => Some(at + 1),
            Some(at) => at.checked_sub(1),
            None => turns
                .iter()
                .position(|&(_, below, rows)| below < scroll + page && below + rows > scroll),
        };
        let Some(&(key, below, rows)) = next.and_then(|at| turns.get(at)) else {
            return false;
        };
        self.model.set_turn(Some(key));
        self.model.reveal(below, below + rows, page);
        true
    }

    /// `y`: the marked turn's body goes to the tmux buffer, and the hint row
    /// says what reached where.
    fn copy(&mut self) {
        let line = self.copied();
        self.flash = Some((line, Instant::now()));
    }

    /// The words for a copy. A refusal before any tmux call is a plain hint;
    /// only a copy that was tried, or cannot exist, says `copy failed`.
    fn copied(&mut self) -> String {
        let Some(mark) = self.model.turn() else {
            return "no turn marked - click one or press [ ]".to_owned();
        };
        let keys = self.keys();
        let found = keys.iter().position(|key| *key == mark);
        let Some(item) = found.and_then(|at| self.lane.items.get(at)) else {
            self.model.set_turn(None);
            return "that turn changed or left the lane - mark it again".to_owned();
        };
        let text = terminal_text(&item.body);
        if text.is_empty() {
            return "nothing to copy: that turn has no text".to_owned();
        }
        let preview = lane::preview_why(item).map(|why| format!(" (preview only: {why})"));
        let count = format!("{}{}", lines_word(&text), preview.unwrap_or_default());
        self.ship(&text, &count)
    }

    /// Send `text` to the tmux buffer and to the terminal clipboard of the
    /// client showing this app; the words say what reached where. `count` is
    /// how the text is named. Outside tmux there is no client to name: the
    /// loop writes the text to the terminal itself.
    fn ship(&mut self, text: &str, count: &str) -> String {
        let (Some(server), Some(pane)) = (&self.server, &self.pane) else {
            if text.len() > CLIP_MAX {
                let size = text.len();
                return format!(
                    "not copied: {size} bytes is over the {CLIP_MAX}-byte clipboard limit"
                );
            }
            self.clip = Some(text.to_owned());
            return format!("sent {count} to the terminal clipboard - unconfirmed");
        };
        let viewer = match transport::observe_clients(server) {
            None => Err("the tmux clients did not answer".to_owned()),
            Some(clients) => open::copy_client(&clients, pane).map_err(|why| why.why()),
        }
        .and_then(|name| {
            let named = open::client_name(&name);
            named
                .then_some(name)
                .ok_or("the client name failed its grammar".to_owned())
        });
        match (
            transport::copy_buffer(server, viewer.as_deref().ok(), text.as_bytes()),
            viewer,
        ) {
            (Err(why), _) => format!("copy failed: {}", terminal_text(&why)),
            (Ok(()), Ok(_)) => format!(
                "copied {count} to the tmux buffer; terminal clipboard if your terminal allows it"
            ),
            (Ok(()), Err(why)) => format!("copied {count} to the tmux buffer only - {why}"),
        }
    }

    /// The hint-row line now, loudest first: an ask on its way, an armed
    /// key, a refused paste, an outcome the lane has not caught up with,
    /// another installed core.
    fn note(&self) -> Option<String> {
        let sending = self
            .queued
            .first()
            .map(|ask| format!("sending to {}…", ask.seat));
        sending
            .or_else(|| self.armed.as_ref().map(|armed| armed.what.line()))
            .or_else(|| self.flash.as_ref().map(|(line, _)| line.clone()))
            .or_else(|| {
                let settling = self.settling.as_ref();
                let here = |s: &&Settling| self.model.selected() == Some(s.about.as_str());
                settling.filter(here).map(|settling| settling.word.clone())
            })
            .or_else(|| self.upgrade.as_ref().map(upgrade::hint))
    }

    /// What holds a restart back now: any draft, not only the writer's —
    /// one kept in memory after Esc is lost too.
    fn blocker(&self, now: Instant) -> Option<upgrade::Blocker> {
        upgrade::restart_blocker(
            self.inputs.values().any(|input| !input.draft().is_empty()),
            !self.queued.is_empty(),
            !self.deferred.is_empty(),
            self.composing(),
            self.held.is_some(),
            self.model.settings_open() || self.model.peek_open(),
            self.model.turn().is_some(),
            self.armed.is_some(),
            self.last_input,
            now,
        )
    }

    /// The restart due now: another core is installed, this app may restart
    /// into it, and nothing holds it back.
    fn restart_due(&self, now: Instant) -> Option<(String, toggle::Restart)> {
        let upgrade = self.upgrade.as_ref()?;
        let restart = upgrade.restart.as_ref().ok()?;
        (self.blocker(now).is_none() && self.clip.is_none())
            .then(|| (upgrade.version.clone(), restart.clone()))
    }

    /// End what `now` outlived: whether any line went away.
    fn lapse(&mut self, now: Instant) -> bool {
        let over = |at: Instant, window| now.saturating_duration_since(at) > window;
        let armed = self.armed.take_if(|a| over(a.at, QUIT_WINDOW)).is_some();
        let flash = (self.flash.take_if(|(_, at)| over(*at, FLASH_WINDOW))).is_some();
        let word = (self.settling.take_if(|s| over(s.at, FLASH_WINDOW))).is_some();
        // The open panel's capture ages: one redraw for each new second.
        let aged = self.model.peek_open()
            && self.peek.at.is_some()
            && self.peek.painted != Some(self.peek.age(now));
        armed | flash | word | aged
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

    /// A press acts on the frame on screen when it was made: one made before a
    /// frame replaced the layout is hit against the layout that frame replaced,
    /// so an ask in flight or a read since cannot move its target. A held wheel
    /// notch keeps replaying against the newest frame, which bounds it. One
    /// made on a frame the bound dropped is not taken, and the hint row says so.
    fn mouse_made(&mut self, mouse: Mouse, origin: Instant) -> bool {
        // Every press ends a selection first, a stale or refused one too: no
        // frame is kept to aim a drag at.
        let cleared = mouse.kind == MouseKind::Click && self.unselect();
        if mouse.kind == MouseKind::Click && self.lost.is_some_and(|lost| origin <= lost) {
            self.flash = Some((
                "click not taken: its frame is gone".to_owned(),
                Instant::now(),
            ));
            self.model.let_go();
            return true;
        }
        let seen = (self.history.iter()).position(|(replaced, _)| origin <= *replaced);
        let Some(at) = seen.filter(|_| mouse.kind == MouseKind::Click) else {
            return self.mouse(mouse) || cleared;
        };
        let older = std::mem::take(&mut self.history[at].1);
        let current = std::mem::replace(&mut self.layout, older);
        let acted = self.mouse_in(mouse, false);
        self.history[at].1 = std::mem::replace(&mut self.layout, current);
        acted || cleared
    }

    /// Every wake up to now was read: the layouts older than the oldest held key
    /// or the read that began the decoder's unfinished sequence (`begun`) are
    /// done with, and all of them are while no ask waits, since the wakes still
    /// unread follow its key.
    fn forget(&mut self, begun: Option<Instant>) {
        if self.queued.is_empty() {
            let floor = (self.deferred.iter().map(|(_, at)| *at).chain(begun)).min();
            self.history
                .retain(|(replaced, _)| floor.is_some_and(|floor| *replaced >= floor));
        }
    }

    /// Mouse actions use the last drawn frame, in either input mode. Left
    /// motion moves a dragged border; every other event first ends a drag —
    /// its release, or a press or wheel after a release that never came.
    fn mouse(&mut self, mouse: Mouse) -> bool {
        self.mouse_in(mouse, true)
    }

    /// [`App::mouse`] for a press made on the frame that is on screen (`fresh`)
    /// or on one a later frame replaced, which can arm no selection. A
    /// selection a panel change left behind goes first, and so does one a
    /// press ends.
    fn mouse_in(&mut self, mouse: Mouse, fresh: bool) -> bool {
        let mut redraw = self.stale_selection();
        if mouse.kind == MouseKind::Click {
            redraw |= self.unselect();
        }
        redraw | self.route(mouse, fresh)
    }

    fn route(&mut self, mouse: Mouse, fresh: bool) -> bool {
        if self.model.settings_open() {
            return self.settings_mouse(mouse);
        }
        if self.model.peek_open() {
            return self.peek_mouse(mouse, fresh);
        }
        if mouse.kind == MouseKind::Drag {
            return self.drag(mouse) || self.stretch(mouse);
        }
        let ended = self.model.let_go();
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
            MouseKind::Drag => ended,
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

    /// A press: what it always did, then a selection armed where it landed.
    fn clicked(&mut self, mouse: Mouse, fresh: bool) -> bool {
        let before = self.model.turn();
        let acted = self.click(mouse);
        self.arm(mouse, before, fresh);
        acted
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
            draw::Hit::Seat(name) => {
                self.focus_seat(Some(name));
                return true;
            }
            draw::Hit::Settings => {
                self.open_settings();
                return true;
            }
            draw::Hit::SettingsTab(_) | draw::Hit::SettingsClose | draw::Hit::PeekClose => {
                return held;
            }
            draw::Hit::Turn(key) => {
                self.write(false);
                self.model
                    .set_turn((self.model.turn() != Some(key)).then_some(key));
                return true;
            }
        };
        apply(self, &act, Instant::now()).unwrap_or(false)
            || was_writing != self.composing()
            || held
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

    /// Begin a selection at the press `mouse` made on the frame on screen:
    /// not on a border's grab, not on a frame a later one replaced, and only
    /// where that frame recorded a selectable region. `before` is the marked
    /// turn from before the press.
    fn arm(&mut self, mouse: Mouse, before: Option<u64>, fresh: bool) {
        let at = Position::new(mouse.column, mouse.row);
        let region = (fresh && self.layout.grab(mouse).is_none())
            .then(|| self.layout.map.region_at(at).cloned())
            .flatten();
        let Some(region) = region else {
            return;
        };
        let fp = select::fingerprint(&self.shot, region.area, &self.layout.map.blanks);
        let turn = matches!(self.layout.hit(mouse), Some(draw::Hit::Turn(_)));
        self.select = Some(Held {
            sel: select::Selection::start(&region, at, fp),
            epoch: self.model.epoch(),
            toggled: turn,
            before,
            down: true,
            stage: Stage::None,
        });
    }

    /// The button is down on a selection and the pointer is at `mouse`: its
    /// head follows, and the first cell past the anchor makes it a drag, which
    /// puts back the mark the press toggled. Whether the head moved and
    /// whether that was the drag's first cell; `None` with no such selection.
    fn follow(&mut self, mouse: Mouse) -> Option<(bool, bool)> {
        let held = self.select.as_mut().filter(|held| held.down)?;
        let first = !held.sel.moved();
        let changed = held.sel.extend(Position::new(mouse.column, mouse.row));
        if first && held.sel.moved() && std::mem::take(&mut held.toggled) {
            self.model.set_turn(held.before);
        }
        Some((changed, first))
    }

    /// Left motion: whether a frame is due.
    fn stretch(&mut self, mouse: Mouse) -> bool {
        (self.follow(mouse)).is_some_and(|(changed, first)| changed || first)
    }

    /// The button let go: a selection that never moved was a click; one that
    /// did asks for its copy, which the next frame paints and then ships.
    fn finish(&mut self, mouse: Mouse) -> bool {
        self.follow(mouse).is_some_and(|_| {
            let moved = self.select.as_mut().is_some_and(|held| {
                held.down = false;
                held.stage = Stage::Requested;
                held.sel.moved()
            });
            if !moved {
                self.select = None;
            }
            moved
        })
    }

    /// A press or Esc ends a selection: whether a highlight went away.
    fn unselect(&mut self) -> bool {
        self.select.take().is_some_and(|held| held.sel.moved())
    }

    /// Drop a selection that the model has since moved out from under: a
    /// session, a tab or an overlay changed, even if it changed back. Whether
    /// a highlight went away.
    fn stale_selection(&mut self) -> bool {
        let epoch = self.model.epoch();
        let stale = self.select.take_if(|held| held.epoch != epoch);
        stale.is_some_and(|held| held.sel.moved())
    }

    /// A newer peek capture waits while a selection in the panel is held down
    /// or has a copy to ship; once shipped or cleared the next frame adopts it.
    fn holds_peek(&self) -> bool {
        (self.select.as_ref()).is_some_and(|held| {
            held.sel.panel() == select::Panel::Peek && (held.down || held.stage != Stage::None)
        })
    }

    /// The selection against the frame just drawn into `buf`. The held
    /// fingerprint is compared with these NEW cells, before they replace
    /// `shot` or wear the highlight: a retained shot could never differ from
    /// itself. Passing, a requested copy becomes the very text the highlight
    /// covers; failing, the selection and its copy are gone.
    fn settle_selection(&mut self, buf: &mut Buffer) {
        let epoch = self.model.epoch();
        let map = &self.layout.map;
        let proven = (self.select.as_ref()).map(|held| {
            let region = map.region(held.sel.panel());
            held.epoch == epoch
                && region.is_some_and(|region| region.area == held.sel.area())
                && select::fingerprint(buf, held.sel.area(), &map.blanks) == held.sel.fp()
        });
        self.shot.clone_from(buf);
        match (proven, self.select.as_mut()) {
            (Some(true), Some(held)) => {
                if held.stage == Stage::Requested {
                    held.stage = Stage::Ready(select::text(buf, &held.sel, &map.blanks));
                }
                if held.sel.moved() {
                    select::highlight(buf, &held.sel, &map.blanks);
                }
            }
            (Some(false), Some(_)) => {
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

    /// What the loop owes after a frame: the staged copy shipped and a hint
    /// the frame had no room to draw. Whether another frame is due.
    fn shipped(&mut self) -> bool {
        self.ship_staged() || std::mem::take(&mut self.again)
    }

    /// A frame's debts, settled together: the copy it shipped and the asks
    /// that waited for it. Neither skips the other; whether a frame is due.
    fn owed(&mut self) -> bool {
        self.shipped() | self.deliver()
    }

    /// Ship the copy the last frame highlighted, now that it is written: the
    /// text goes the way `y` sends a turn. Whether it said anything.
    fn ship_staged(&mut self) -> bool {
        let ready = (self.select.as_mut()).filter(|held| matches!(held.stage, Stage::Ready(_)));
        let Some(Stage::Ready(text)) = ready.map(|held| std::mem::take(&mut held.stage)) else {
            return false;
        };
        let line = if text.trim().is_empty() {
            "nothing to copy: the selection is blank".to_owned()
        } else {
            self.ship(&text, &lines_word(&text))
        };
        self.flash = Some((line, Instant::now()));
        true
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
                Effect::Ask { raw, seat, body } => {
                    let Some((key, entry)) = (self.writer.as_ref()).map(|w| (w.key(), w.entry))
                    else {
                        self.notice(&about, "refused: not writing".to_owned());
                        continue;
                    };
                    self.queued.push(Queued {
                        key,
                        entry,
                        raw,
                        seat,
                        body,
                        about,
                    });
                    continue;
                }
                Effect::Close(id) => self.close(id.as_deref()),
                Effect::Open(seat) => self.open_written(&seat),
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
    fn ask(
        &mut self,
        raw: &[u8],
        seat: &str,
        body: String,
    ) -> Result<(String, submit::Outcome), String> {
        let Some(writer) = &self.writer else {
            return Err("refused: not writing".to_owned());
        };
        let (console, lost) = (&writer.console, Cell::default());
        let owns = || {
            self.owns(&lost)?;
            let name = console.name();
            let routed = crate::tracked::named_server(console.dir(), name, name);
            let routed = routed.map(drop).map_err(|why| why.message());
            routed.inspect_err(|why| lost.set(Some(why.clone())))
        };
        let told = term::submit_ask(console, owns, raw, seat, body);
        self.unproven(lost, raw);
        told.map_err(|why| format!("refused: {why}"))
    }

    /// Deliver the queued asks, after the frame that named their target. Each
    /// is re-proven first: a writer that ended since asks nothing and its
    /// line goes back to the draft in memory. Whether any ran.
    fn deliver(&mut self) -> bool {
        let queued = std::mem::take(&mut self.queued);
        let ran = !queued.is_empty();
        for q in queued {
            let live = (self.writer.as_ref()).is_some_and(|writer| writer.entry == q.entry);
            let told = if live {
                match self.ask(&q.raw, &q.seat, q.body) {
                    Ok((id, outcome)) => self.settle(id, &q.seat, &q.about, outcome),
                    Err(refused) => vec![refused],
                }
            } else {
                if let Some(input) = self.inputs.get_mut(&q.key) {
                    let _ = input.restore(submit::Draft::Kept(q.raw));
                }
                let why = "writing ended; the line is back in the draft";
                vec![format!("not sent to {}: {why}", q.seat)]
            };
            for line in told {
                self.notice(&q.about, line);
            }
            self.reread();
        }
        self.show();
        ran
    }

    /// What an ask's outcome shows. The lane already draws an uncertain or
    /// undelivered ask in the same words, so those wait on the hint row until
    /// it holds the ask; the rest has no lane line and is a notice.
    fn settle(
        &mut self,
        id: String,
        seat: &str,
        about: &str,
        outcome: submit::Outcome,
    ) -> Vec<String> {
        let check = format!("check {seat} pane (Agents tab: n/p, oo opens a seat)");
        let word = match outcome {
            submit::Outcome::Sent(_, kept) => {
                return ["sent".to_owned()].into_iter().chain(kept).collect();
            }
            submit::Outcome::Unknown(..) => {
                return vec![format!(
                    "no record of it; it may have been delivered - {check}"
                )];
            }
            submit::Outcome::Uncertain(_) => format!("uncertain: {check}"),
            submit::Outcome::NotDelivered(_) => "not delivered".to_owned(),
        };
        let (at, about) = (Instant::now(), about.to_owned());
        self.settling = Some(Settling {
            at,
            id,
            word,
            about,
        });
        Vec::new()
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

    /// `/open <seat>` typed in the composer: a seat of the session this app
    /// writes, as the incarnation its entry proved names it.
    fn open_written(&self, seat: &SeatRef) -> String {
        let Some(writer) = &self.writer else {
            return "refused: not writing".to_owned();
        };
        let console = &writer.console;
        self.open(console.name(), console.uuid(), Ok(seat), &seat.name)
    }

    /// `o`: the first press arms on the seat the frame highlights and moves
    /// nothing; the second within the window opens THAT seat, whatever the
    /// frame drew since. A frame with no seat to open answers at once.
    fn open_key(&mut self, origin: Instant) {
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
    fn open_focused(&mut self) {
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
            _ => Refusal::NoSeat("the view changed".to_owned()).line(&session),
        };
        self.notice(&session, line);
        self.show();
    }

    /// Open `seat` of `session`, bound to the incarnation `uuid`: the chat's
    /// proof, then ONE guarded command handing the one client showing this
    /// app to its pane. The line to tell, refused or opened.
    fn open(
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
        let client = match &step {
            Move::Client(client) => Some(client.as_str()),
            Move::Here => None,
        };
        let Some((ran, stdout)) = transport::open_seat_via(server, &target, client) else {
            return format!("refused: /open {label}: a fact failed its grammar; nothing selected");
        };
        if open::took(ran, &stdout) {
            let _ = match client {
                Some(client) => transport::display_client_message(
                    server,
                    client,
                    &format!("ae: {label} - prefix h = chat, prefix L = back"),
                ),
                None => transport::display_message(
                    server,
                    &target.pane,
                    &format!("ae: {label} - prefix h returns"),
                ),
            };
        }
        match client {
            Some(_) => open::outcome_moved(ran, &stdout, label, session),
            None => open::outcome(ran, &stdout, label),
        }
    }

    /// `n` / `p`: the highlight steps to the next or previous seat the last
    /// frame drew, wrapping; whether it moved.
    fn move_focus(&mut self, forward: bool) -> bool {
        let selected = self.model.selected();
        let Some(drawn) = self
            .drawn
            .as_ref()
            .filter(|d| selected == Some(d.session.as_str()))
        else {
            return false;
        };
        let count = drawn.rows.len();
        let at = (drawn.focused.as_ref())
            .and_then(|name| drawn.rows.iter().position(|(row, _)| row == name))
            .unwrap_or(0);
        let to = if forward {
            (at + 1) % count.max(1)
        } else {
            (at + count.max(1) - 1) % count.max(1)
        };
        let Some(name) = drawn.rows.get(to).map(|(name, _)| name.clone()) else {
            return false;
        };
        self.focus_seat(Some(name));
        true
    }

    /// Highlight seat `name`: the model keeps the preference, the frozen
    /// frame the cursor the keys after it step from.
    fn focus_seat(&mut self, name: Option<String>) {
        if let (Some(drawn), Some(name)) = (&mut self.drawn, &name)
            && drawn.rows.iter().any(|(row, _)| row == name)
        {
            drawn.focused = Some(name.clone());
        }
        self.model.set_focus(name);
    }

    /// Freeze what the frame just drew of `session`'s seats.
    fn freeze(&self, session: &str) -> Drawn {
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

    /// The composer line for the selection.
    fn composer(&self) -> draw::Composer<'_> {
        match (self.model.selected(), &self.held) {
            (None, _) => draw::Composer::NoHome,
            (Some(_), Some(why)) => draw::Composer::Held { why },
            (Some(_), None) if !self.read_only.is_empty() && !self.composing() => {
                draw::Composer::ReadOnly {
                    why: &self.read_line,
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
        if self.model.selected().is_some() {
            let area = buf.area;
            let size = Size {
                width: draw::draft_width(area, self.model.split()),
                height: draw::composer_pane(area),
            };
            let shown = self.shown_input().map(|i| (i.bare_view(size), i.draft()));
            (self.draft_view, self.draft) = shown.unwrap_or_default();
        }
        // An ask the lane shows needs no word of its own.
        let shown = |item: &Item, id: &str| matches!(&item.kind, Kind::Asked { id: ask, .. } | Kind::NotDelivered { id: ask, .. } if ask == id);
        let lane = Rc::clone(&self.lane);
        self.settling
            .take_if(|settling| lane.items.iter().any(|item| shown(item, &settling.id)));
        self.model.set_note(self.note());
        let keys = self.keys();
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
            keys: &keys,
        };
        let drawn = draw::draw_with_layout(&screen, wait, buf);
        let replaced = std::mem::replace(&mut self.layout, drawn);
        self.history.push((Instant::now(), replaced));
        if self.history.len() > HISTORY {
            self.lost = Some(self.history.remove(0).0);
        }
        self.model
            .settle(self.layout.list_start, self.layout.body_top);
        // Held keys replay from the scroll they were read at, as one read. A
        // handed-over scroll waits for the lane it scrolls: until that is
        // read there is no bound to hold it to.
        let waits = self.restored && self.loading;
        self.restored = waits;
        if self.deferred.is_empty() && !waits {
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
        if self.model.peek_open() {
            self.paint_peek(look.as_ref(), buf);
        }
        let drawn = self.model.selected().map(|name| self.freeze(name));
        self.drawn = drawn;
        self.settle_selection(buf);
    }

    /// Paint the open peek panel over the frame: its newest body with its age
    /// as of now, then keep the viewport inside what was drawn.
    fn paint_peek(&mut self, look: Option<&theme::Look>, buf: &mut Buffer) {
        self.peek_sync();
        self.peek_adopt();
        self.layout.strip_for_peek();
        let age = self.peek.age(Instant::now());
        let (session, seat) = match &self.peek.target {
            Some((session, seat)) => (session.as_str(), seat.as_str()),
            None => (self.model.selected().unwrap_or_default(), ""),
        };
        let view = peek::View {
            session,
            seat,
            body: self.peek.shown.as_ref(),
            age_secs: age,
            back: self.model.peek_back().unwrap_or(0),
            newer_waiting: self.peek.held.is_some(),
        };
        let bounds = draw::paint_peek(&view, look, buf, &mut self.layout);
        self.peek.painted = Some(age);
        self.model.set_peek_page_rows(bounds.visible);
        if self.deferred.is_empty() {
            self.model.clamp_peek(bounds.max_back);
        }
    }
}

/// How a copy names its text: `1 line`, `3 lines`.
fn lines_word(text: &str) -> String {
    match text.lines().count() {
        1 => "1 line".to_owned(),
        lines => format!("{lines} lines"),
    }
}

/// `seat`'s pane in `session` on `server`, proven NOW: the recorded server,
/// not the launch target, must be the one this app's own pane lives on, then
/// the chat's own proof. One door for `/open` and the peek panel's captures.
fn proven_pane(
    server: &ServerId,
    session: &str,
    uuid: &str,
    dir: &Path,
    seat: Result<&SeatRef, Refusal>,
) -> Result<open::Target, Refusal> {
    let mut sockets = crate::SocketPaths::asking(crate::transport::observe_socket_path);
    let recorded = crate::session_launch::recorded_server_resolved(dir);
    if !recorded.is_some_and(|recorded| sockets.proven_same(server, &recorded)) {
        return Err(Refusal::Elsewhere(session.to_owned()));
    }
    let console = Console::bound(session.to_owned(), dir.to_owned(), uuid.to_owned());
    term::prove_open(server, &console, seat?)
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
    let (caller, pane) = (doors::caller_server(), doors::calling_pane_id());
    let mut reader = loader::Reader::new(root.clone(), home.clone(), server.clone());
    reader.settings_paths(loader::SettingsPaths {
        home: doors::home(),
        global: doors::config_file(crate::shape::current(), &root),
        local: doors::local_config(&doors::cwd()),
    });
    reader.own_pane(caller.clone(), pane.clone());
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
    let mut app = App::new(home, caller, pane, Some(ask));
    let mut keys = Keys::app();
    let (mut dirty, mut first, mut restart) = (true, None, None);
    let code = loop {
        if let Some(now) = tty.size().filter(|now| *now != size) {
            size = now;
            terminal.backend_mut().resize(size);
            dirty = true;
        }
        if dirty || !app.queued.is_empty() {
            terminal.draw(|frame| {
                app.frame(frame.buffer_mut());
                if let Some(at) = app.layout.cursor {
                    frame.set_cursor_position(at);
                }
            })?;
        }
        // A release's copy ships only once the frame showing it is out. An
        // ask waits for the frame that names its target; its outcome needs
        // another.
        dirty = settle(&mut app, terminal.backend_mut())?;
        // Held keys replay as soon as the frame above has produced more.
        let wait = if app.deferred.is_empty() {
            TICK
        } else {
            Duration::ZERO
        };
        if first.is_none() {
            first = match wakes.recv_timeout(wait) {
                Ok(wake) => Some(wake),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break 0,
            };
        }
        let Some(redraw) = drain(&mut app, &mut keys, &wakes, &mut first) else {
            break clipped(&mut app, terminal.backend_mut()).map(|()| 0)?;
        };
        dirty |= redraw | app.lapse(Instant::now());
        app.focus();
        if reader.is_finished() {
            break 1;
        }
        restart = app.restart_due(Instant::now());
        if restart.is_some() {
            break 0;
        }
    };
    end(tty, app, code, restart, err)
}

/// How `ae app` ends, its terminal put back first: restarted into the core
/// the command link names when that was due, else with the line that says
/// how to start it again.
fn end(
    tty: tty::Tty,
    app: App,
    code: u8,
    restart: Option<(String, toggle::Restart)>,
    err: &mut impl Write,
) -> crate::Result<u8> {
    drop(tty);
    let in_tmux = app.server.is_some();
    if let Some((version, restart)) = restart {
        app.publish_view(&restart, &version);
        // The lease goes before the pane, so the app that replaces it can
        // write at once.
        drop(app);
        let Err(why) = toggle::respawn_own(&restart) else {
            return Ok(0);
        };
        let line = format!("ae app: could not restart into ae {version} - {why}");
        writeln!(err, "{}", terminal_text(&line))?;
        writeln!(err, "{}", exit_hint(in_tmux))?;
        return Ok(crate::entry::EXIT_FAILED);
    }
    if code != 0 {
        writeln!(err, "ae app: its background reader stopped")?;
    }
    writeln!(err, "{}", exit_hint(in_tmux))?;
    Ok(code)
}

/// The one plain line `ae app` leaves on the normal screen when it ends: how
/// to start it again. Outside tmux it names no tmux key.
#[must_use]
pub fn exit_hint(in_tmux: bool) -> &'static str {
    if in_tmux {
        "ae app closed - run: ae app (in a chat window, prefix h reopens it)"
    } else {
        "ae app closed - run: ae app"
    }
}

/// What a frame owes (`App::owed`), then the copy staged for the terminal
/// outside tmux: written once the frame is out, so no escape lands inside it.
fn settle<W: Write>(app: &mut App, backend: &mut backend::Ansi<W>) -> std::io::Result<bool> {
    let dirty = app.owed();
    clipped(app, backend)?;
    Ok(dirty)
}

/// The copy staged for the terminal, written. A quit writes it too: a copy
/// accepted in the batch that quit is not lost with the app.
fn clipped<W: Write>(app: &mut App, backend: &mut backend::Ansi<W>) -> std::io::Result<()> {
    match app.clip.take() {
        Some(text) => backend.clipboard(&text),
        None => Ok(()),
    }
}

/// Take the held keys, then `first` and every wake already waiting behind
/// it, then the idle keys when none came: `Some(redraw)`, or `None` to quit.
/// Once an ask is queued nothing more is taken, `first` included: it stays in
/// place with the rest of the channel until the loop has delivered the ask.
fn drain(
    app: &mut App,
    keys: &mut Keys,
    wakes: &mpsc::Receiver<Wake>,
    first: &mut Option<Wake>,
) -> Option<bool> {
    let held = std::mem::take(&mut app.deferred);
    let (mut redraw, mut fed) = (take_keys(app, held)?, false);
    while app.queued.is_empty()
        && let Some(wake) = first.take().or_else(|| wakes.try_recv().ok())
    {
        let keyed = match wake {
            Wake::Keys(stamp, bytes) => {
                crate::read_gate("@app-keys");
                fed = true;
                app.last_input = Some(stamp);
                // A lone Esc, a paste start or a dropped control byte decodes
                // to no key yet: the read itself is the human's input.
                if !bytes.is_empty() {
                    (app.keep, app.touched) = (None, true);
                }
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
    app.forget(keys.begun());
    Some(redraw)
}

/// Take decoded keys: `Some(redraw)`, or `None` to quit. From a wheel notch
/// the frame cannot bound yet on, and behind an ask the loop has not
/// delivered, every key is held in order; a held `^C` still quits at once,
/// unless it would lose an ask or a draft. Any key but a browsing letter or
/// `^C` ends the armed press.
fn take_keys(app: &mut App, keyed: Vec<(Key, Instant)>) -> Option<bool> {
    if !keyed.is_empty() {
        // Whatever the human did, a view handed over later never yanks it.
        (app.keep, app.touched) = (None, true);
    }
    let mut redraw = false;
    let mut keyed = keyed.into_iter();
    while let Some((key, origin)) = keyed.next() {
        if !app.fleeted && key != Key::Interrupt {
            redraw |= early(app, &key, origin)?;
            continue;
        }
        if !app.deferred.is_empty() || !app.queued.is_empty() || app.unbounded(&key) {
            app.deferred.push((key, origin));
            app.deferred.extend(keyed);
            let held = app.deferred.iter().any(|(key, _)| *key == Key::Interrupt);
            let quit = held && app.queued.is_empty() && !app.protects();
            return (!quit).then_some(true);
        }
        let browsing = !app.model.settings_open() && !app.composing() && app.held.is_none();
        if key != Key::Interrupt && !(browsing && matches!(key, Key::Text(_))) {
            redraw |= app.armed.take().is_some();
        }
        // A selection a panel change left behind is gone before this key; Esc
        // with one still held only clears it.
        redraw |= app.stale_selection();
        if key == Key::Escape && app.select.take().is_some_and(|held| held.sel.moved()) {
            redraw = true;
            continue;
        }
        redraw |= if let Key::Mouse(mouse) = &key {
            app.mouse_made(*mouse, origin)
        } else {
            // A key or a paste ends a drag first, then acts as it always has.
            let ended = app.model.let_go();
            let acted = if app.model.settings_open() {
                settings_key(app, &key, origin)?
            } else if app.model.peek_open() {
                peek_key(app, &key, origin)?
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

/// Take a key read before the first fleet read, a Text run byte by byte:
/// `q` presses its armed key now, a navigation key waits for the fleet it
/// navigates ([`EARLY_CAP`], the oldest go first) and nothing else is taken,
/// told once on the hint row. Each of those ends an armed press. `Some(true)`,
/// or `None` to quit.
fn early(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    let pieces = match key {
        Key::Text(bytes) => bytes.iter().map(|byte| Key::Text(vec![*byte])).collect(),
        other => vec![other.clone()],
    };
    for piece in pieces {
        if piece == Key::Text(b"q".to_vec()) {
            browse(app, &piece, origin)?;
            continue;
        }
        app.armed = None;
        let nav = match &piece {
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Tab | Key::Escape => true,
            Key::Text(byte) => matches!(byte.as_slice(), [b'1'..=b'9' | b'!' | b'j' | b'k']),
            _ => false,
        };
        let evicted = nav && app.early.len() >= EARLY_CAP;
        if evicted {
            app.early.remove(0);
        }
        if nav {
            app.early.push((piece, origin));
        }
        if (!nav || evicted) && !std::mem::replace(&mut app.early_named, true) {
            app.flash = Some((EARLY_NOTICE.to_owned(), Instant::now()));
        }
    }
    Some(true)
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
            if app.model.peek_open() {
                redraw |= peek_keys(app, model::browse_keys(&Key::Text(vec![byte])));
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
            redraw |= browse_byte(app, byte, origin)?;
            rest = tail;
        }
        return Some(redraw);
    }
    if let Key::Pasted(bytes) = key {
        paste(app, bytes, origin);
        return Some(true);
    }
    let mut redraw = false;
    for key in model::browse_keys(key) {
        let act = app.model.key(key, &app.fleet, app.can_compose(), true);
        redraw |= apply(app, &act, origin)?;
    }
    Some(redraw)
}

/// A paste while browsing is never a command: a writable selection starts
/// writing with it as the draft, anything else says why it was not taken.
fn paste(app: &mut App, bytes: &[u8], origin: Instant) {
    if !app.can_compose() {
        let why = Some(app.read_line.as_str()).filter(|why| !why.is_empty());
        let why = format!("read-only · {}", why.unwrap_or("no session is selected"));
        return app.refuse_paste(&why);
    }
    app.write(true);
    if !app.composing() {
        let why = app.held.clone().unwrap_or_default();
        return app.refuse_paste(&why);
    }
    app.admitted = Some(origin);
    let _ = app.compose(Key::Pasted(bytes.to_vec()), origin);
}

/// One Text byte while browsing: `q` and `o` press their armed key, `s`
/// opens the settings and `?` its keys, anything else decodes as it always
/// has and ends the armed press. Browsing on entry; the caller routes the
/// modes.
fn browse_byte(app: &mut App, byte: u8, origin: Instant) -> Option<bool> {
    let disarmed = !matches!(byte, b'q' | b'o') && app.armed.take().is_some();
    match byte {
        b'q' => return app.press(Arm::Quit, origin).is_none().then_some(true),
        b's' => {
            app.open_settings();
            return Some(true);
        }
        b'?' => {
            app.open_settings();
            app.model.show_settings_tab(model::SettingsTab::Keys);
            return Some(true);
        }
        _ => {}
    }
    let mut redraw = disarmed;
    for key in model::browse_keys(&Key::Text(vec![byte])) {
        let act = app.model.key(key, &app.fleet, app.can_compose(), true);
        redraw |= apply(app, &act, origin)?;
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
                redraw |= browse_byte(app, byte, origin)?;
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

/// Take one key with the peek panel open: `Some(redraw)`, or `None` to quit.
/// The panel owns every key: it scrolls, it closes, and a paste or any other
/// key dies. Bytes after a mid-chunk close resume the actual mode.
fn peek_key(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    if *key == Key::Interrupt {
        return None;
    }
    if matches!(key, Key::Text(_)) {
        // `browse` feeds the open panel byte by byte and hands the rest of
        // the chunk to the mode it is really in once the panel closes.
        return browse(app, key, origin);
    }
    Some(peek_keys(app, model::browse_keys(key)))
}

/// Feed `keys` to the open peek panel; a close they cause tells the reader.
fn peek_keys(app: &mut App, keys: Vec<model::Key>) -> bool {
    let mut redraw = false;
    for key in keys {
        redraw |= app.model.key(key, &app.fleet, false, false) == model::Act::Redraw;
    }
    if !app.model.peek_open() {
        app.close_peek();
        redraw = true;
    }
    redraw
}

/// One Text byte with the overlay open: `s` closes with its request, the
/// rest decodes modal. The overlay is open on entry; the caller resumes the
/// actual mode after a close.
fn settings_byte(app: &mut App, byte: u8) -> bool {
    if byte == b's' {
        app.close_settings();
        return true;
    }
    if byte == b'?' {
        app.model.show_settings_tab(model::SettingsTab::Keys);
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
fn apply(app: &mut App, act: &model::Act, origin: Instant) -> Option<bool> {
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
        model::Act::Open => {
            app.open_key(origin);
            Some(true)
        }
        model::Act::SeatNext => Some(app.move_focus(true)),
        model::Act::SeatPrev => Some(app.move_focus(false)),
        model::Act::TurnOlder => Some(app.step_turn(true)),
        model::Act::TurnNewer => Some(app.step_turn(false)),
        model::Act::Copy => {
            app.copy();
            Some(true)
        }
        model::Act::Peek => {
            app.open_peek();
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
    use super::upgrade::Blocker;
    use super::{
        App, USAGE, browse, draw, facts_of, fleet, run, select, settings, settings_key, take_keys,
    };
    use crate::app::fleet::{Counts, Fleet, Row};
    use crate::app::model::{Drag, Edge, Key as Browse, Model, SettingsTab, Tab};
    use crate::attention::Reason;
    use crate::console::input::{Effect, Input, Key, Mouse, MouseKind, Reading};
    use crate::console::lane::{Item, Kind, Lane};
    use crate::digest::{SessionEntry, Status};
    use crate::listing::World;
    use crate::render::Audience;
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
        assert!(shown.lines().last().expect("keys row").contains("? keys"));
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
        assert_eq!(refused, Err("refused: not writing".to_owned()));
        let typed = writing(&mut app);
        meta(&root, &ID.replace("1234", "bbbb"), "colead");
        for key in [Key::Text(b"check the scopes".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, typed), Some(()));
        }
        assert!(app.deliver(), "the entered line was queued, then asked");
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

    /// `/open` in an app with no tmux identity (no `$TMUX`, no `$TMUX_PANE`)
    /// names that and moves nothing, once the home roster is read.
    #[test]
    fn open_outside_tmux_is_refused_by_name() {
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
            "refused: /open lead: ae app is not running inside tmux; nothing selected"
        );
    }

    fn seat_ref(slot: &str, name: &str) -> crate::console::needs::SeatRef {
        crate::console::needs::SeatRef {
            slot: slot.to_owned(),
            name: name.to_owned(),
        }
    }

    /// A frame of `api` that drew `names`, none of them in a roster.
    fn drawn_rows(names: &[&str], focused: &str) -> super::Drawn {
        super::Drawn {
            session: "api".to_owned(),
            uuid: ID.to_owned(),
            rows: names
                .iter()
                .map(|name| ((*name).to_owned(), None))
                .collect(),
            focused: Some(focused.to_owned()),
        }
    }

    /// `n` / `p` step the rows the LAST frame drew, wrapping both ways and
    /// from the cursor a key before it moved; a frame of another session
    /// moves nothing, and a name the frame did not draw is only a preference.
    #[test]
    fn the_seat_cursor_steps_the_drawn_rows_and_wraps() {
        let root = Root::new("cursor");
        let (mut app, _reader) = housed(&root);
        let cursor = |app: &App| app.drawn.as_ref().and_then(|d| d.focused.clone());
        let names = ["lead", "colead", "scout"];
        app.drawn = Some(drawn_rows(&names, "lead"));
        assert!(app.move_focus(true) && app.move_focus(true));
        assert_eq!(cursor(&app).as_deref(), Some("scout"));
        assert_eq!(app.model.focus(), Some("scout"));
        assert!(app.move_focus(true), "past the last row wraps");
        assert_eq!(cursor(&app).as_deref(), Some("lead"));
        assert!(app.move_focus(false), "before the first row wraps");
        assert_eq!(cursor(&app).as_deref(), Some("scout"));
        app.focus_seat(Some("ghost".to_owned()));
        assert_eq!(cursor(&app).as_deref(), Some("scout"), "not a drawn row");
        assert_eq!(app.model.focus(), Some("ghost"), "the preference is kept");
        app.drawn = Some(super::Drawn {
            session: "web".to_owned(),
            ..drawn_rows(&names, "lead")
        });
        assert!(!app.move_focus(true), "a frame of another session");
        app.drawn = Some(super::Drawn {
            rows: Vec::new(),
            ..drawn_rows(&[], "lead")
        });
        assert!(!app.move_focus(false), "no seat drawn");
    }

    /// A frame freezes each drawn seat with the identity the roster of the
    /// read it drew from held, and the incarnation of that read; `o` acts on
    /// that and says why when it holds none.
    #[test]
    fn a_frame_freezes_the_drawn_seats_with_the_roster_it_drew_from() {
        let root = Root::new("freeze");
        let (mut app, _reader) = housed(&root);
        app.shown.insert(
            "api".to_owned(),
            super::Shown {
                id: "read-id".to_owned(),
                seq: 1,
                lane: Rc::default(),
                roster: Some(vec![
                    seat_ref("main", "lead"),
                    seat_ref("spawned.0", "scout"),
                ]),
            },
        );
        (app.layout.seats, app.layout.focused) = (
            vec!["lead".to_owned(), "ghost".to_owned()],
            Some("lead".to_owned()),
        );
        let mut drawn = app.freeze("api");
        assert_eq!(drawn.uuid, "read-id");
        assert_eq!(
            drawn.rows,
            [
                ("lead".to_owned(), Some(seat_ref("main", "lead"))),
                ("ghost".to_owned(), None)
            ]
        );
        assert_eq!(drawn.seat(), Ok(&seat_ref("main", "lead")));
        drawn.focused = Some("ghost".to_owned());
        assert_eq!(
            drawn.seat().map_err(|why| why.line("ghost")),
            Err(
                "refused: /open ghost: ghost is not in the roster read for api; nothing selected"
                    .to_owned()
            )
        );
        drawn.focused = None;
        assert!(
            matches!(drawn.seat(), Err(crate::console::open::Refusal::NoSeat(why)) if why.contains("no seat is shown"))
        );
        assert_eq!(
            app.freeze("web").uuid,
            "",
            "a session not read has no incarnation"
        );
    }

    /// `o` acts only on the frame the human saw: none, or one of another
    /// session, is refused by name; outside tmux it is refused before any read.
    #[test]
    fn open_acts_only_on_the_frame_that_was_drawn() {
        let root = Root::new("frozen-open");
        let (mut app, _reader) = housed(&root);
        let said = |app: &App| app.notices.last().expect("said").1.body.clone();
        app.open_focused();
        assert_eq!(
            said(&app),
            "refused: /open api: the view changed; nothing selected"
        );
        app.drawn = Some(super::Drawn {
            session: "web".to_owned(),
            ..drawn_rows(&["lead"], "lead")
        });
        app.open_focused();
        assert_eq!(
            said(&app),
            "refused: /open api: the view changed; nothing selected"
        );
        app.drawn = Some(drawn_rows(&["lead"], "lead"));
        app.open_focused();
        assert_eq!(
            said(&app),
            "refused: /open lead: ae app is not running inside tmux; nothing selected"
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

    /// R-C1.3/R-C1.5: the address, the three drawn draft rows and the note
    /// row are Compose; after Esc the rows above the compact composer are
    /// lane, not ghost targets.
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
        for row in 38..=42 {
            assert!(
                matches!(app.layout.hit(press(row)), Some(super::draw::Hit::Compose)),
                "address, grown row or note {row}"
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
                .nth(40)
                .expect("compact composer")
                .contains("to api")
        );
        assert!(!matches!(
            app.layout.hit(press(39)),
            Some(super::draw::Hit::Compose)
        ));
        let _ = app.click(press(39));
        assert!(!app.composing(), "old upper row cannot restart writing");
        for row in 40..=42 {
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

    /// #50/#56/#57, R2: browsing, only a second `q` within the window quits;
    /// one arms, another key or a late `q` does not complete it.
    #[test]
    fn only_a_second_q_in_the_window_quits_while_browsing() {
        let mut app = app(None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        let (t0, q) = (Instant::now(), Key::Text(b"q".to_vec()));
        assert!(browse(&mut app, &Key::Text(b"x".to_vec()), t0).is_some());
        assert!(browse(&mut app, &q, t0).is_some(), "one q only arms");
        assert_eq!(app.note().as_deref(), Some("q again to quit"));
        assert!(browse(&mut app, &Key::Text(b"x".to_vec()), t0).is_some());
        assert!(app.armed.is_none(), "another key disarms");
        assert!(browse(&mut app, &q, t0).is_some());
        let late = t0 + super::QUIT_WINDOW + Duration::from_millis(1);
        assert!(browse(&mut app, &q, late).is_some(), "a late q re-arms");
        assert!(app.lapse(late + super::QUIT_WINDOW * 2), "the arm expires");
        assert!(app.armed.is_none() && app.note().is_none());
        assert!(browse(&mut app, &q, late).is_some());
        assert!(browse(&mut app, &q, late).is_none(), "the second q quits");
    }

    /// R1/B1: a paste while browsing starts writing with it as the draft and
    /// every later fragment of that read keeps its text, while text typed in
    /// that read stays dropped; a selection that cannot be written says why.
    #[test]
    fn a_browse_paste_writes_whole_and_a_read_only_one_says_why() {
        let root = Root::new("paste");
        let (mut app, _reader) = housed(&root);
        let t0 = Instant::now();
        let paste = |bytes: &[u8]| (Key::Pasted(bytes.to_vec()), t0);
        assert_eq!(take_keys(&mut app, vec![paste(b"one ")]), Some(true));
        assert!(app.composing() && app.admitted == Some(t0));
        let typed = (Key::Text(b"X".to_vec()), t0);
        let early = t0.checked_sub(Duration::from_millis(1)).expect("booted");
        let other = (Key::Pasted(b"Y".to_vec()), early);
        let batch = vec![paste(b"two"), typed, other];
        assert_eq!(take_keys(&mut app, batch), Some(true));
        assert_eq!(
            drafted(&app),
            "one two",
            "typed text and another read's paste are dropped, the tail kept"
        );
        app.write(false);
        assert!(
            app.admitted.is_none(),
            "a way out of writing forgets the paste"
        );
        let mut cold = self::app(Some("api"));
        assert_eq!(take_keys(&mut cold, vec![paste(b"x")]), Some(true));
        let said = cold.note().expect("a refused paste says so");
        assert!(said.starts_with("paste not taken: read-only"), "{said}");
    }

    /// P1/C2: `^C` over a draft arms and asks twice, an edit disarms it, and
    /// an empty composer quits at once.
    #[test]
    fn interrupt_over_a_draft_arms_and_an_empty_composer_quits() {
        let root = Root::new("c2");
        let (mut app, _reader) = housed(&root);
        let at = writing(&mut app);
        assert_eq!(app.compose(Key::Interrupt, at), None, "empty quits at once");
        assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
        assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, at)]), Some(true));
        let line = "^C again to quit - the draft is lost";
        assert_eq!(app.note().as_deref(), Some(line));
        assert_eq!(
            take_keys(&mut app, vec![(Key::Text(b"!".to_vec()), at)]),
            Some(true)
        );
        assert!(app.armed.is_none(), "an edit disarms");
        let twice = vec![(Key::Interrupt, at), (Key::Interrupt, at)];
        assert_eq!(take_keys(&mut app, twice), None, "the second quits");
    }

    /// P2/O1: the first `o` arms on the frame it saw and moves nothing; the
    /// second within the window opens THAT seat, though a later frame moved
    /// the highlight or drew no seat at all; a late second only arms again.
    #[test]
    fn an_armed_open_keeps_the_seat_it_saw() {
        let root = Root::new("armed-open");
        let (mut app, _reader) = housed(&root);
        let frame = |focused: &str| super::Drawn {
            rows: vec![
                ("lead".to_owned(), Some(seat_ref("main", "lead"))),
                ("colead".to_owned(), Some(seat_ref("worker.0", "colead"))),
            ],
            ..drawn_rows(&[], focused)
        };
        let (t0, said) = (Instant::now(), |app: &App| {
            app.notices.last().map(|n| n.1.body.clone())
        });
        app.drawn = Some(frame("lead"));
        app.open_key(t0);
        assert_eq!(app.note().as_deref(), Some("o again to open lead"));
        assert_eq!(said(&app), None, "the first press opens nothing");
        app.drawn = Some(frame("colead"));
        app.open_key(t0 + Duration::from_millis(500));
        let opened = "refused: /open lead: ae app is not running inside tmux; nothing selected";
        assert_eq!(said(&app).as_deref(), Some(opened), "the captured seat");
        app.open_key(t0 + Duration::from_secs(9));
        assert_eq!(app.note().as_deref(), Some("o again to open colead"));
        // I1: a refresh that drew no seat still completes the stored arm.
        app.notices.clear();
        app.drawn = None;
        app.open_key(t0 + Duration::from_millis(9500));
        let colead = "refused: /open colead: ae app is not running inside tmux; nothing selected";
        assert_eq!(said(&app).as_deref(), Some(colead), "the stored seat");
        app.drawn = Some(frame("colead"));
        app.open_key(t0 + Duration::from_secs(9));
        let moved = Fleet {
            rows: vec![row("web", 1, true)],
            home: Some("web".to_owned()),
        };
        app.model = Model::new(&moved);
        app.open_key(t0 + Duration::from_secs(10));
        assert!(said(&app).is_some_and(|line| line.contains("the view changed")));
    }

    /// I5: a `^C` held behind other keys quits at once, unless it would lose a
    /// draft only this process holds - then it waits, in order, with them.
    #[test]
    fn a_held_interrupt_quits_at_once_unless_it_would_lose_a_draft() {
        let root = Root::new("held-quit");
        let (mut app, _reader) = housed(&root);
        let at = writing(&mut app);
        let behind = |app: &mut App| {
            app.deferred = vec![(Key::Text(b"x".to_vec()), at)];
            take_keys(app, vec![(Key::Interrupt, at)])
        };
        assert_eq!(behind(&mut app), None, "nothing to lose: it quits");
        assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
        assert_eq!(behind(&mut app), Some(true), "a draft is not lost");
        assert_eq!(app.deferred.len(), 2, "the ^C waits behind the key");
    }

    /// O1: a press after the window passed arms the seat the CURRENT frame
    /// highlights, not the stale one; a frame of the selection with no seat to
    /// open answers at once and arms nothing.
    #[test]
    fn an_expired_open_rearms_the_current_seat_and_a_seatless_frame_refuses() {
        let root = Root::new("open-expired");
        let (mut app, _reader) = housed(&root);
        let frame = |focused: &str| super::Drawn {
            rows: vec![
                ("lead".to_owned(), Some(seat_ref("main", "lead"))),
                ("colead".to_owned(), Some(seat_ref("worker.0", "colead"))),
            ],
            ..drawn_rows(&[], focused)
        };
        let t0 = Instant::now();
        app.drawn = Some(frame("lead"));
        app.open_key(t0);
        app.drawn = Some(frame("colead"));
        app.open_key(t0 + super::QUIT_WINDOW + Duration::from_millis(1));
        assert_eq!(app.note().as_deref(), Some("o again to open colead"));
        assert!(app.notices.is_empty(), "the stale arm opened nothing");
        app.armed = None;
        app.drawn = Some(super::Drawn {
            focused: None,
            ..frame("lead")
        });
        app.open_key(t0 + Duration::from_secs(9));
        assert!(app.armed.is_none(), "no seat, nothing armed");
        let said = (app.notices.last()).map_or_else(String::new, |n| n.1.body.clone());
        assert!(
            said.starts_with("refused: /open api"),
            "answered at once: {said}"
        );
    }

    /// P1: the settled word of an ask shows on the hint row of the session it
    /// answers and of no other.
    #[test]
    fn a_settled_word_shows_on_its_own_sessions_hint_row_only() {
        let root = Root::new("settled-word");
        let (mut app, _reader) = housed(&root);
        let settling = |about: &str| super::Settling {
            at: Instant::now(),
            id: "r".to_owned(),
            word: "delivered".to_owned(),
            about: about.to_owned(),
        };
        app.settling = Some(settling("api"));
        assert_eq!(app.note().as_deref(), Some("delivered"));
        app.settling = Some(settling("web"));
        assert_eq!(app.note(), None, "another session's word is not shown");
    }

    /// P1: a line lapses once its window has passed, not at the window itself,
    /// and `lapse` says so whenever any of the three went away.
    #[test]
    fn a_line_lapses_after_its_window_and_any_lapse_is_said() {
        let root = Root::new("lapse");
        let (mut app, _reader) = housed(&root);
        let t0 = Instant::now();
        let past = |window: Duration| t0 + window + Duration::from_millis(1);
        let arm = |on: bool| {
            on.then_some(super::Armed {
                at: t0,
                what: super::Arm::Quit,
            })
        };
        let flash = |on: bool| on.then(|| ("paste not taken: x".to_owned(), t0));
        let word = |on: bool| {
            on.then(|| super::Settling {
                at: t0,
                id: "r".to_owned(),
                word: "delivered".to_owned(),
                about: "api".to_owned(),
            })
        };
        app.armed = arm(true);
        assert!(
            !app.lapse(t0 + super::QUIT_WINDOW),
            "at the window it still waits"
        );
        assert!(app.armed.is_some());
        for (on, at) in [
            ((true, false, false), past(super::QUIT_WINDOW)),
            ((true, true, false), past(super::FLASH_WINDOW)),
            ((true, false, true), past(super::FLASH_WINDOW)),
            ((false, true, true), past(super::FLASH_WINDOW)),
            ((true, true, true), past(super::FLASH_WINDOW)),
            ((false, true, false), past(super::FLASH_WINDOW)),
            ((false, false, true), past(super::FLASH_WINDOW)),
        ] {
            (app.armed, app.flash, app.settling) = (arm(on.0), flash(on.1), word(on.2));
            assert!(app.lapse(at), "{on:?} lapsed");
            let left = (app.flash.is_some(), app.settling.is_some());
            assert_eq!(
                left,
                (false, false),
                "{on:?}: flash and word are over by {at:?}"
            );
        }
        app.flash = flash(true);
        assert!(
            !app.lapse(t0 + super::FLASH_WINDOW),
            "at the window it still waits"
        );
    }

    /// B1: a paste while the app is HELD is refused with a hint, every
    /// fragment of it again, and starts no draft.
    #[test]
    fn a_paste_while_held_is_refused_on_the_hint_row_fragment_by_fragment() {
        let root = Root::new("held-paste");
        let (mut app, _reader) = housed(&root);
        app.held = Some("an ae app is writing to api".to_owned());
        let at = Instant::now();
        for fragment in [b"one".as_slice(), b"two"] {
            app.flash = None;
            let pasted = vec![(Key::Pasted(fragment.to_vec()), at)];
            assert_eq!(take_keys(&mut app, pasted), Some(true));
            let said = app.note().expect("a held paste says why");
            assert!(
                said.starts_with("paste not taken: ") && said.contains("Esc"),
                "{said}"
            );
        }
        assert!(
            !app.composing() && drafted(&app).is_empty(),
            "no draft started"
        );
    }

    /// A read-only selection's refused paste carries the real reason the
    /// session cannot be written, in full.
    #[test]
    fn a_refused_browse_paste_names_the_sessions_real_reason() {
        let mut app = self::app(Some("api"));
        app.read_line = "api is stopped; ae api resumes it".to_owned();
        let pasted = vec![(Key::Pasted(b"x".to_vec()), Instant::now())];
        assert_eq!(take_keys(&mut app, pasted), Some(true));
        let line = "paste not taken: read-only · api is stopped; ae api resumes it";
        assert_eq!(app.note().as_deref(), Some(line));
    }

    /// `app` drawn into a `width` x 45 buffer, as text.
    fn framed_at(app: &mut App, width: u16) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 45));
        app.frame(&mut buf);
        text(&buf)
    }

    /// P1: a pending line takes the hint row, the third from the bottom.
    #[test]
    fn a_pending_line_is_drawn_on_the_hint_row_above_the_keys_row() {
        let root = Root::new("hint-row");
        let (mut app, _reader) = housed(&root);
        app.refuse_paste("read-only");
        let shown = framed_at(&mut app, 160);
        let rows: Vec<&str> = shown.lines().collect();
        assert!(rows[42].contains("paste not taken: read-only"), "{shown}");
        assert!(rows[41].trim().is_empty() || !rows[41].contains("paste not taken"));
    }

    /// The settings overlay puts three cells between its tab titles when all of
    /// them fit that way, else one.
    #[test]
    fn the_settings_tab_titles_keep_three_cells_only_while_all_fit_so() {
        let root = Root::new("tab-gaps");
        let (mut app, _reader) = housed(&root);
        app.open_settings();
        let row = |app: &mut App, width: u16| {
            let shown = framed_at(app, width);
            shown
                .lines()
                .nth(1)
                .expect("the tab row")
                .trim_end()
                .to_owned()
        };
        // Two cells of margin, five titles of 32 cells, four gaps of three.
        let wide = "  Quota   Config   About   Instructions   Keys";
        assert_eq!(row(&mut app, 46), wide);
        assert_eq!(row(&mut app, 45), "  Quota Config About Instructions Keys");
    }

    /// R4(f)/I5: Enter queues its ask behind the frame that names it; a key
    /// behind it waits in order, an answer behind it waits in the channel,
    /// and a `^C` behind it quits only after the ask ran.
    #[test]
    fn an_entered_ask_is_delivered_before_anything_behind_it_acts() {
        let root = Root::new("queued");
        let (mut app, _reader) = housed(&root);
        let at = writing(&mut app);
        // The admission refuses the ask, so nothing is really delivered.
        meta(&root, &ID.replace("1234", "bbbb"), "colead");
        let (wake, wakes) = std::sync::mpsc::channel();
        let mut keys = crate::console::input::Keys::app();
        let late = super::Wake::Answer(Box::new(view("api", ID, 9, "late")));
        wake.send(super::Wake::Keys(at, b"hi\r\x03".to_vec()))
            .expect("sent");
        wake.send(late).expect("sent");
        assert_eq!(
            super::drain(&mut app, &mut keys, &wakes, &mut None),
            Some(true)
        );
        assert_eq!((app.queued.len(), app.deferred.len()), (1, 1), "^C waits");
        assert_eq!(app.note().as_deref(), Some("sending to lead…"));
        assert!(wakes.try_recv().is_ok(), "the answer was not pulled");
        assert!(app.deliver() && app.queued.is_empty());
        let notice = &app.notices.last().expect("said").1.body;
        assert_eq!(notice, "refused: the session was replaced or renamed");
        assert_eq!(super::drain(&mut app, &mut keys, &wakes, &mut None), None);
    }

    /// I5: an ask whose writer ended while it waited asks nothing: the line
    /// goes back to its draft in memory and the human is told.
    #[test]
    fn a_queued_ask_whose_writer_ended_is_not_sent() {
        let root = Root::new("revoked-queue");
        let dir = root.0.join("sessions").join("api");
        let (mut app, _reader) = housed(&root);
        let at = writing(&mut app);
        for key in [Key::Text(b"keep me".to_vec()), Key::Enter] {
            assert_eq!(app.compose(key, at), Some(()));
        }
        app.write(false);
        assert!(app.deliver());
        assert_eq!(drafted(&app), "keep me");
        let notice = &app.notices.last().expect("said").1.body;
        assert!(
            notice.starts_with("not sent to lead: writing ended"),
            "{notice}"
        );
        let written = crate::store::open(&dir).events_source();
        assert!(matches!(written, crate::store::SourceRead::Absent));
    }

    /// I5/B1: a held `Enter` replays and queues its ask; the wake read before
    /// it (a revocation, the input closing) waits for the delivery, then acts.
    #[test]
    fn a_prefetched_wake_waits_for_the_ask_a_held_enter_queued() {
        for closed in [false, true] {
            let root = Root::new("prefetch");
            let (mut app, _reader) = housed(&root);
            let at = writing(&mut app);
            meta(&root, &ID.replace("1234", "bbbb"), "colead");
            assert_eq!(app.compose(Key::Text(b"hi".to_vec()), at), Some(()));
            app.deferred = vec![(Key::Enter, at)];
            let entry = app.writer.as_ref().map(|w| w.entry).expect("writing");
            let reading = Reading::NotOwner("gone".to_owned());
            let owned = Answer::Owned { entry, reading, at };
            let mut first = Some(super::Wake::Answer(Box::new(owned)));
            if closed {
                first = Some(super::Wake::Closed);
            }
            let (_wake, wakes) = std::sync::mpsc::channel();
            let mut keys = crate::console::input::Keys::app();
            let drain = super::drain(&mut app, &mut keys, &wakes, &mut first);
            assert!(drain.is_some() && first.is_some(), "kept, and no quit");
            assert_eq!(app.queued.len(), 1, "the held Enter queued its ask");
            assert!(app.composing(), "the revocation waits for the delivery");
            assert!(app.deliver() && app.queued.is_empty());
            let drain = super::drain(&mut app, &mut keys, &wakes, &mut first);
            assert_eq!(drain.is_none(), closed, "then the wake acts");
        }
    }

    /// I5: behind a queued ask the rest of the read waits in order - an
    /// `Esc`, a second `Enter`, a wheel notch - and none acts before it.
    #[test]
    fn keys_behind_an_entered_ask_wait_in_order_for_its_delivery() {
        let wheel = super::Mouse {
            kind: super::MouseKind::WheelDown,
            column: 60,
            row: 10,
        };
        for behind in [Key::Escape, Key::Enter, Key::Mouse(wheel)] {
            let root = Root::new("queued-order");
            let (mut app, _reader) = housed(&root);
            let at = writing(&mut app);
            meta(&root, &ID.replace("1234", "bbbb"), "colead");
            let hi = (Key::Text(b"hi".to_vec()), at);
            let read = vec![hi, (Key::Enter, at), (behind.clone(), at)];
            assert_eq!(take_keys(&mut app, read), Some(true));
            let (queued, held) = (app.queued.len(), app.deferred.len());
            assert_eq!((queued, held), (1, 1), "{behind:?} waits behind one ask");
            assert!(app.composing() && app.deliver());
        }
    }

    /// `api` writing with `web` drawn under it, then a fleet read that `read`
    /// reorders, truncates or swaps (`ops` in `web`'s place): the app and the
    /// press on `web`'s card, made before that read's first frame.
    fn pressed_web(root: &Root, read: &str) -> (App, Instant, String) {
        let (mut app, reader) = housed(root);
        session(root, "web", "");
        let both = [("api", Status::Running), ("web", Status::Running)];
        refold(&mut app, &reader, root, &both);
        let at = writing(&mut app);
        meta(root, &ID.replace("1234", "bbbb"), "colead");
        let shown = framed(&mut app);
        let at_row = shown.lines().position(|line| line.contains("web"));
        let mut data = read_of(&app, &[("api", ID), ("web", ID)]);
        match read {
            "order" => data.fleet.rows.reverse(),
            "gone" => data.fleet.rows.truncate(1),
            _ => data.fleet.rows[1] = row("ops", 2, false),
        }
        app.answer(Answer::Fleet(data));
        (
            app,
            at,
            format!("\x1b[<0;3;{}M", at_row.expect("web row") + 1),
        )
    }

    /// B2: a click held behind a queued ask acts on the row it was made on, whatever the
    /// sending frame drew of a fleet read; one made after that frame acts on what it drew.
    #[test]
    fn a_click_held_behind_a_queued_ask_acts_on_the_row_it_was_made_on() {
        for (read, held, fresh) in [
            ("swap", "api", "ops"),
            ("order", "web", "api"),
            ("gone", "api", "api"),
        ] {
            let root = Root::new(&format!("click-held-{read}"));
            let (mut app, at, click) = pressed_web(&root, read);
            let mut keys = crate::console::input::Keys::app();
            let mut keyed = vec![(Key::Text(b"hi".to_vec()), at), (Key::Enter, at)];
            keyed.extend(keys.feed(click.as_bytes(), Instant::now()));
            assert_eq!(take_keys(&mut app, keyed), Some(true));
            assert_eq!((app.queued.len(), app.deferred.len()), (1, 1));
            let _sending = framed(&mut app);
            assert!(app.deliver() && app.queued.is_empty());
            let (_wake, wakes) = std::sync::mpsc::channel();
            assert!(super::drain(&mut app, &mut keys, &wakes, &mut None).is_some());
            assert_eq!(app.model.selected(), Some(held), "{read}: held click");
            let _ = take_keys(&mut app, keys.feed(click.as_bytes(), Instant::now()));
            assert_eq!(app.model.selected(), Some(fresh), "{read}: later click");
        }
    }

    /// B2: a second ask queued behind the first repaints once more; the click
    /// held behind both still acts on the row it was made on. A refusal ends the
    /// writing, so the lease is taken again for the held Enter to queue its ask.
    #[test]
    fn a_click_held_behind_two_queued_asks_acts_on_the_row_it_was_made_on() {
        let root = Root::new("click-held-two");
        let (mut app, at, click) = pressed_web(&root, "swap");
        let mut keys = crate::console::input::Keys::app();
        let soon = Instant::now() + Duration::from_mins(1);
        let text = |bytes: &[u8], at| (Key::Text(bytes.to_vec()), at);
        let mut keyed = vec![
            text(b"a", at),
            (Key::Enter, at),
            text(b"b", soon),
            (Key::Enter, soon),
        ];
        keyed.extend(keys.feed(click.as_bytes(), Instant::now()));
        assert_eq!(take_keys(&mut app, keyed), Some(true));
        let (_wake, wakes) = std::sync::mpsc::channel();
        for queued in [1, 0] {
            let _sending = framed(&mut app);
            assert!(app.deliver() && app.queued.is_empty());
            if queued == 1 {
                (app.held, ()) = (None, meta(&root, ID, "colead"));
                writing(&mut app);
                meta(&root, &ID.replace("1234", "bbbb"), "colead");
            }
            assert!(super::drain(&mut app, &mut keys, &wakes, &mut None).is_some());
            assert_eq!(app.queued.len(), queued, "the second ask queues behind");
        }
        assert_eq!(app.model.selected(), Some("api"), "no retarget");
    }

    /// B2: a click the terminal splits over three reads is begun at the first,
    /// and the frames that drew since stay kept for it until its last byte lands.
    #[test]
    fn a_click_split_across_reads_acts_on_the_row_it_was_begun_on() {
        let root = Root::new("click-split");
        let (mut app, _at, click) = pressed_web(&root, "swap");
        let (mut keys, (_wake, wakes)) = (
            crate::console::input::Keys::app(),
            std::sync::mpsc::channel(),
        );
        let click = click.as_bytes();
        let mut read = |app: &mut App, bytes: &[u8]| {
            let mut first = Some(loader::Wake::Keys(Instant::now(), bytes.to_vec()));
            assert!(super::drain(app, &mut keys, &wakes, &mut first).is_some());
        };
        read(&mut app, &[b"a\r", &click[..3]].concat());
        assert_eq!((app.queued.len(), app.deferred.len()), (1, 0));
        let _sending = framed(&mut app);
        assert!(app.deliver() && app.queued.is_empty());
        read(&mut app, &click[3..click.len() - 1]);
        let _outcome = framed(&mut app);
        read(&mut app, &click[click.len() - 1..]);
        assert_eq!(app.model.selected(), Some("api"), "no retarget");
        assert!(app.history.is_empty(), "nothing kept once it landed");
    }

    /// B2: a report that never ends keeps the eight newest frames and its click
    /// is not taken, said on the hint row, yet it still ends a drag left open;
    /// one the decoder discarded keeps none and ends nothing.
    #[test]
    fn the_frames_kept_for_an_unfinished_click_are_bounded() {
        let gone = Some("click not taken: its frame is gone");
        for (filler, kept, said) in [(20, 8, gone), (40, 0, None)] {
            let grab = Drag {
                edge: Edge::Sidebar,
                from: 44,
                at: 44,
            };
            let root = Root::new(&format!("click-bound-{filler}"));
            let (mut app, _at, click) = pressed_web(&root, "swap");
            let (mut keys, (_wake, wakes)) = (
                crate::console::input::Keys::app(),
                std::sync::mpsc::channel(),
            );
            let mut read = |app: &mut App, bytes: &[u8]| {
                let mut first = Some(loader::Wake::Keys(Instant::now(), bytes.to_vec()));
                assert!(super::drain(app, &mut keys, &wakes, &mut first).is_some());
            };
            let (head, tail) = click.as_bytes().split_at(7);
            read(&mut app, head);
            app.model.grab(grab);
            for _ in 0..filler {
                let _frame = framed(&mut app);
                read(&mut app, b"0");
            }
            assert_eq!(app.history.len(), kept, "{filler} bytes of one report");
            read(&mut app, tail);
            assert_eq!(app.model.selected(), Some("api"), "no retarget");
            assert_eq!(app.note().as_deref(), said);
            assert_eq!(app.model.drag().is_some(), kept == 0, "the drag");
        }
    }

    /// R3: the exit line names the way back, and outside tmux no tmux key.
    #[test]
    fn the_exit_line_names_tmux_keys_only_inside_tmux() {
        assert!(super::exit_hint(true).ends_with("(in a chat window, prefix h reopens it)"));
        assert_eq!(super::exit_hint(false), "ae app closed - run: ae app");
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
                roster: None,
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
                | loader::Request::Peek(_)
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
        for mut first in [None, Some(loader::Wake::Answer(Box::new(quiet)))] {
            let (mut app, _reader) = housed(&root);
            writing(&mut app);
            let mut keys = crate::console::input::Keys::app();
            let typed = Instant::now()
                .checked_sub(Duration::from_secs(1))
                .expect("an instant a second ago");
            assert!(keys.feed(b"\x1b", typed).is_empty(), "ESC waits");
            assert_eq!(
                super::drain(&mut app, &mut keys, &wakes, &mut first),
                Some(true)
            );
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

    /// `bytes` read at once on a 160x37 pane (27 chat rows), taken through
    /// the loop's own drain and frame until no key is held.
    fn replayed(app: &mut App, bytes: &[u8]) -> usize {
        let (_wake, wakes) = std::sync::mpsc::channel();
        let mut keys = crate::console::input::Keys::app();
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 37));
        app.frame(&mut buf);
        let mut first = Some(loader::Wake::Keys(Instant::now(), bytes.to_vec()));
        for _ in 0..64 {
            assert!(super::drain(app, &mut keys, &wakes, &mut first).is_some());
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
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 37));
        app.frame(&mut buf);
        assert!(!app.layout.complete);
        let edge = app.layout.max_scroll;
        assert_eq!((edge - 27) % 3, 0, "the edge is whole notches above a page");
        let (_wake, wakes) = std::sync::mpsc::channel();
        let mut keys = crate::console::input::Keys::app();
        let mut bytes = b"\x1b[5~".to_vec();
        bytes.extend(b"\x1b[<64;48;6M".repeat((edge - 27) / 3));
        let mut read = Some(loader::Wake::Keys(Instant::now(), bytes));
        assert!(super::drain(&mut app, &mut keys, &wakes, &mut read).is_some());
        assert!(app.deferred.is_empty(), "held at the edge");
        assert_eq!(app.model.scroll_rows(27), edge);
        let mut up = Some(loader::Wake::Keys(
            Instant::now(),
            b"\x1b[<64;48;6M".to_vec(),
        ));
        assert!(super::drain(&mut app, &mut keys, &wakes, &mut up).is_some());
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
                    | loader::Request::Peek(_)
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
                | loader::Request::Peek(_)
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

    /// R2: the reader captures the session it was last told is selected, so
    /// an open tells the selection the UI shows first, and only a change.
    #[test]
    fn settings_open_tells_the_current_selection_first() {
        let (ask, asks) = std::sync::mpsc::channel();
        let mut app = App::new(None, None, None, Some(ask));
        let order = |asks: &std::sync::mpsc::Receiver<loader::Request>| -> Vec<String> {
            asks.try_iter()
                .map(|request| match request {
                    loader::Request::Focus(name) => format!("focus {name:?}"),
                    loader::Request::Settings { open, .. } => format!("settings {open}"),
                    loader::Request::Reread(_)
                    | loader::Request::Peek(_)
                    | loader::Request::Writing(_) => String::new(),
                })
                .collect()
        };
        two(&mut app);
        app.open_settings();
        assert_eq!(order(&asks), [r#"focus Some("web")"#, "settings true"]);
        app.close_settings();
        app.open_settings();
        assert_eq!(order(&asks), ["settings false", "settings true"]);
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
        app.settled(99, quota(), Some(config()), Some(about()), None);
        assert!(app.settings_bodies.quota.is_none(), "stale dropped");
        app.settled(1, quota(), Some(config()), Some(about()), None);
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
        app.settled(1, quota(), Some(config()), Some(about()), None);
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
        let shown = app.model.settings().expect("open").tab;
        assert_eq!(
            shown,
            SettingsTab::Instructions,
            "digit 4 shows the fourth tab"
        );
        assert!(settings_key(&mut app, &Key::Text(b"5".to_vec()), at).is_some());
        assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Keys);
        assert!(settings_key(&mut app, &Key::Text(b"6".to_vec()), at).is_some());
        assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Keys);
        assert!(settings_key(&mut app, &Key::Tab, at).expect("wraps"));
        assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Quota);
        assert!(settings_key(&mut app, &Key::Text(b"?".to_vec()), at).is_some());
        assert_eq!(app.model.settings().expect("open").tab, SettingsTab::Keys);
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

    fn ready(dir: &std::path::Path, name: &str) -> settings::Protocol {
        match settings::resolve_instructions(Some(dir), name) {
            settings::InstructionsView::Ready(protocol) => protocol,
            other => panic!("ready expected: {other:?}"),
        }
    }

    /// The rules are the render owner's generic rules, each roster seat names
    /// the audiences its role receives, the winning file is named (an empty
    /// winner masks, no row falls to the global) and a slot the context
    /// classifies as neither lead nor worker is a named gap.
    #[test]
    fn resolve_instructions_is_the_render_owners_rules_and_each_seats_audiences() {
        let root = Root::new("instr-seats");
        let (global, overlay) = (root.0.join("global"), root.0.join("overlay"));
        std::fs::write(&global, "[prompt]\ninstructions = from global\n").expect("global");
        std::fs::write(&overlay, "[prompt]\ninstructions = from session\n").expect("overlay");
        let tail = format!(
            "work_dir=/w\nconfig={}\nlocal_config={}\nseat.spawned.1=helper\nwork_dir.spawned.1=/apart\nseat.extra=odd\n",
            global.display(),
            overlay.display()
        );
        let dir = session(&root, "api", &tail);
        let files = [global, overlay.clone()];
        let protocol = ready(&dir, "api");
        let custom = protocol.custom.as_ref().expect("instructions in force");
        assert_eq!(
            (custom.source, custom.file.as_str(), custom.text.as_str()),
            (
                settings::ConfigSource::Session,
                overlay.to_str().expect("utf-8"),
                "from session"
            )
        );
        let roles: Vec<_> = protocol.seats.iter().map(|seat| seat.role).collect();
        assert_eq!(roles, ["lead", "lead", "worker", "other"]);
        assert_eq!(protocol.rules, crate::render::generic_rules(&dir, &files));
        let received = |at: usize| protocol.seats[at].receives.as_ref().ok().copied();
        let (every, pair, workers) = (Audience::Every, Audience::LeadPair, Audience::Workers);
        assert_eq!(received(0), Some(&[every, pair][..]));
        assert_eq!(received(1), Some(&[every, pair][..]));
        assert_eq!(received(2), Some(&[every, workers][..]));
        let odd = protocol.seats[3].receives.as_ref();
        assert!(odd.expect_err("unknown slot").contains("unknown slot"));
        std::fs::write(&overlay, "[prompt]\ninstructions = \"\"\n").expect("overlay");
        let masked = ready(&dir, "api");
        assert_eq!(masked.custom, None, "the empty winner masks the global");
    }

    /// What cannot be rendered from is named: no records, a meta that cannot
    /// be read, a torn config (the whole tab) and a slot that is no slot (the
    /// one seat); a seat's work dir and name are no gap, the tab shows neither.
    #[test]
    fn resolve_instructions_names_every_gap() {
        let root = Root::new("instr-gaps");
        let gap =
            |dir: Option<&std::path::Path>, name| match settings::resolve_instructions(dir, name) {
                settings::InstructionsView::Gap(why) => why,
                other => panic!("gap expected: {other:?}"),
            };
        assert!(gap(None, "web").contains("web: no records"));
        let torn = root.0.join("torn");
        std::fs::write(&torn, "[prompt]\ninstructions = \"\"\"\nunclosed\n").expect("torn");
        let tail = format!("work_dir=/w\nconfig={}\n", torn.display());
        let why = gap(Some(&session(&root, "api", &tail)), "api");
        assert!(why.contains(torn.to_str().expect("utf-8")) && why.contains("unterminated"));
        let dir = session(
            &root,
            "web",
            "seat.spawned.2=-bad\nwork_dir.worker.0=relative\nseat.worker.0x=ok-one\nseat.spawned.=ok-two\n",
        );
        let seats = ready(&dir, "web").seats;
        let gaps: Vec<_> = seats
            .iter()
            .map(|seat| seat.receives.as_ref().err())
            .collect();
        assert!(gaps[..3].iter().all(Option::is_none), "{gaps:?}");
        assert!(
            gaps[3..]
                .iter()
                .all(|why| why.is_some_and(|why| why.contains("unknown slot"))),
            "{gaps:?}"
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
        assert_eq!(rows.len(), 21);
        let row = |key: &str| rows.iter().find(|row| row.key == key).expect(key);
        assert_eq!(row("chat").value, "on");
        assert_eq!(row("chat").source, settings::ConfigSource::Default);
        assert_eq!(row("auto_trust").value, "on");
        assert_eq!(row("auto_trust").source, settings::ConfigSource::Default);
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
    /// opens; `qqs` quits before ever reaching s; `sq` opens and closes,
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
            browse(&mut app, &Key::Text(b"qqs".to_vec()), Instant::now()).is_none(),
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
        app.fleeted = true;
        app.open_settings();
        let (_wake, wakes) = std::sync::mpsc::channel();
        let mut keys = crate::console::input::Keys::app();
        for (name, tab, scroll, redraw) in [
            ("Config", SettingsTab::Config, 3, true),
            ("Config", SettingsTab::Config, 3, true),
            ("Config", SettingsTab::Config, 0, false),
            ("About", SettingsTab::About, 3, true),
            ("Quota", SettingsTab::Quota, 3, true),
            ("Instructions", SettingsTab::Instructions, 3, true),
        ] {
            let shown = framed(&mut app);
            let titles = shown.lines().nth(1).expect("drawn tab titles");
            let byte = titles.find(name).expect("drawn title");
            let column = titles[..byte].chars().count() + name.chars().count() - 1;
            app.model.settings_wheel(scroll);
            let press = format!("\x1b[<0;{};2M", column + 1).into_bytes();
            let mut first = Some(loader::Wake::Keys(Instant::now(), press));
            assert_eq!(
                super::drain(&mut app, &mut keys, &wakes, &mut first),
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
        app.fleeted = true;
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

    /// The gear is the toggle: at 80x19 the version and the gear end the
    /// keys row, and clicking the gear cell opens Settings.
    #[test]
    fn settings_gear_cell_beside_the_version_opens_settings() {
        let root = Root::new("settings-gear-only");
        let (ask, asks) = std::sync::mpsc::channel();
        let (mut app, _reader) = housed(&root);
        app.ask = Some(ask);
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        app.take(Reading::Owner, Instant::now());
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 19));
        app.frame(&mut buf);
        let shown = text(&buf);
        let keys = shown.lines().last().expect("keys row");
        let tail = keys.trim_end();
        let version = crate::version_line();
        assert!(
            tail.ends_with(&format!("{version} \u{2699}"))
                || tail.ends_with(&format!("{version} *")),
            "version, then the gear last: {tail:?}"
        );
        assert!(
            app.mouse(super::Mouse {
                kind: super::MouseKind::Click,
                column: 79,
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
            instructions: None,
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
            instructions: settings::InstructionsView::Loading,
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
            instructions: None,
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
            instructions: None,
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
        assert!(app.deliver(), "the queued ask meets the admission");
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

    /// An app on `api` whose lane holds one `said` turn for each of `bodies`,
    /// framed once so the keys and clicks have a layout to act on.
    fn turned(bodies: &[&str]) -> App {
        let mut app = app(Some("api"));
        app.fleet = one_row(Some("api"));
        app.model = Model::new(&app.fleet);
        let items = (bodies.iter().zip(0..)).map(|(body, at)| said(body, at));
        app.lane = Rc::new(Lane {
            items: items.collect(),
            coverage: Vec::new(),
        });
        let _ = framed(&mut app);
        app
    }

    fn key_of(app: &App, at: usize) -> u64 {
        super::lane::key(&app.lane.items[at])
    }

    /// `[` and `]` walk the produced turns: none marked takes the newest,
    /// each press steps one, and an edge is silent and keeps the mark.
    #[test]
    fn the_turn_keys_walk_the_produced_turns_and_keep_the_mark_at_an_edge() {
        let mut app = turned(&["one", "two", "three"]);
        assert!(app.step_turn(false), "none marked: the newest shown");
        assert_eq!(app.model.turn(), Some(key_of(&app, 2)));
        assert!(!app.step_turn(false), "no newer turn: silent");
        assert_eq!(app.model.turn(), Some(key_of(&app, 2)));
        assert!(app.step_turn(true) && app.step_turn(true));
        assert_eq!(app.model.turn(), Some(key_of(&app, 0)));
        assert!(!app.step_turn(true), "no older turn: silent");
        assert_eq!(app.model.turn(), Some(key_of(&app, 0)));
        assert!(app.step_turn(false));
        assert_eq!(app.model.turn(), Some(key_of(&app, 1)));
        let mut empty = turned(&[]);
        assert!(
            !empty.step_turn(true) && !empty.step_turn(false),
            "no turns"
        );
        assert_eq!(empty.model.turn(), None);
    }

    /// Every refusal before a copy is a plain hint; outside tmux the copy goes
    /// to the terminal and is not claimed to have arrived. The stale mark is
    /// cleared, the others kept.
    #[test]
    fn a_copy_that_cannot_reach_tmux_says_why_on_the_hint_row() {
        let mut app = turned(&["one", ""]);
        let hint = |app: &mut App| {
            app.copy();
            app.flash.take().expect("a hint").0
        };
        assert_eq!(hint(&mut app), "no turn marked - click one or press [ ]");
        app.model.set_turn(Some(0));
        assert_eq!(
            hint(&mut app),
            "that turn changed or left the lane - mark it again"
        );
        assert_eq!(app.model.turn(), None, "the stale mark is cleared");
        app.model.set_turn(Some(key_of(&app, 1)));
        assert_eq!(hint(&mut app), "nothing to copy: that turn has no text");
        assert_eq!(app.model.turn(), Some(key_of(&app, 1)), "the mark is kept");
        app.model.set_turn(Some(key_of(&app, 0)));
        assert_eq!(
            hint(&mut app),
            "sent 1 line to the terminal clipboard - unconfirmed"
        );
        assert_eq!(app.clip.as_deref(), Some("one"), "staged for the terminal");
    }

    /// The terminal clipboard takes 65536 UTF-8 bytes; one more is refused by
    /// name before anything is staged, and nothing is ever cut.
    #[test]
    fn the_terminal_clipboard_takes_65536_bytes_and_refuses_one_more() {
        let mut app = app(Some("api"));
        let fits = format!("{}a", "\u{4f60}".repeat(21845));
        assert_eq!(fits.len(), 65536);
        assert!(app.ship(&fits, "1 line").starts_with("sent 1 line "));
        assert_eq!(app.clip.as_deref(), Some(fits.as_str()));
        app.clip = None;
        let said = app.ship(&format!("{fits}b"), "1 line");
        assert_eq!(
            said,
            "not copied: 65537 bytes is over the 65536-byte clipboard limit"
        );
        assert_eq!(app.clip, None);
    }

    /// Inside tmux the copy is tmux's, whatever it says: the app writes no
    /// escape of its own there, for tmux takes or drops one by `set-clipboard`.
    #[test]
    fn inside_tmux_a_copy_stages_nothing_for_the_terminal() {
        let mut app = app(Some("api"));
        let socket = std::path::PathBuf::from("/nonexistent-appclip/sock");
        app.server = Some(crate::inventory::ServerId::Selected(
            crate::meta::Selector::Socket(socket),
        ));
        app.pane = Some("%1".to_owned());
        let said = app.ship("alpha", "1 line");
        assert!(said.starts_with("copy failed: "), "{said}");
        assert_eq!(app.clip, None);
    }

    /// The fingerprints are built once for each lane: a frame, a key and a
    /// click share one build, and another lane gets its own.
    #[test]
    fn the_turn_keys_are_built_once_for_each_lane() {
        let mut app = turned(&["one", "two"]);
        let first = app.keys();
        assert!(Rc::ptr_eq(&first, &app.keys()), "same lane, same build");
        app.lane = Rc::new(Lane {
            items: vec![said("one", 0)],
            coverage: Vec::new(),
        });
        assert_eq!(app.keys().len(), 1, "another lane, rebuilt");
        assert_eq!(first.len(), 2);
    }

    /// A press on a drawn turn marks it, a second press unmarks it, and a
    /// press while writing returns to browsing, draft kept; the sidebar rule
    /// still grabs beside it.
    #[test]
    fn a_press_on_a_turn_marks_it_and_a_second_unmarks_it() {
        let root = Root::new("turn-press");
        let (mut app, _reader) = housed(&root);
        let typed = writing(&mut app);
        if let Answer::View(mut read) = view("api", ID, 1, "") {
            read.lane.items = vec![said("one", 0), said("two", 1)];
            app.answer(Answer::View(read));
        }
        assert_eq!(app.compose(Key::Text(b"draft".to_vec()), typed), Some(()));
        let _ = framed(&mut app);
        let press = |column, row| super::Mouse {
            kind: super::MouseKind::Click,
            column,
            row,
        };
        let on_turn = |app: &App, column, row| {
            matches!(
                app.layout.hit(press(column, row)),
                Some(super::draw::Hit::Turn(_))
            )
        };
        let row = (0..45)
            .find(|row| on_turn(&app, 47, *row))
            .expect("a drawn turn");
        assert!(!on_turn(&app, 46, row), "the margin is no target");
        assert!(app.layout.grab(press(44, row)).is_some(), "the rule grabs");
        assert!(app.click(press(47, row)));
        assert!(!app.composing(), "a press browses");
        assert!(app.inputs.values().any(|input| input.draft() == "draft"));
        let marked = app.model.turn();
        assert!(marked.is_some_and(|key| app.keys().contains(&key)));
        assert!(app.click(press(47, row)));
        assert_eq!(app.model.turn(), None, "a second press unmarks it");
    }

    /// On a lane taller than the window a press finds the turn drawn on its
    /// row, the gap row under a turn finds none, and the frame keeps each
    /// turn above the newer ones plus that one gap row.
    #[test]
    fn a_press_finds_the_turn_drawn_on_the_row() {
        let bodies: Vec<String> = (0..30).map(|at| format!("word{at:02}")).collect();
        let mut app = turned(&bodies.iter().map(String::as_str).collect::<Vec<_>>());
        let drawn = framed(&mut app);
        let hit = |row: usize| {
            let (column, row) = (47, u16::try_from(row).expect("row"));
            match app.layout.hit(super::Mouse {
                kind: super::MouseKind::Click,
                column,
                row,
            }) {
                Some(super::draw::Hit::Turn(key)) => Some(key),
                _ => None,
            }
        };
        let body = |at: usize| {
            drawn
                .lines()
                .position(|l| l.contains(&format!("word{at:02}")))
        };
        let (first, last) = (0..30).fold((30, 0), |(lo, hi), at| match body(at) {
            Some(_) => (lo.min(at), hi.max(at)),
            None => (lo, hi),
        });
        assert!(first > 0 && last == 29, "the window is not the whole lane");
        for at in first..=last {
            let row = body(at).expect("a drawn body");
            assert_eq!(hit(row), Some(key_of(&app, at)), "the body row of {at}");
            assert_eq!(hit(row + 1), None, "the gap under {at}");
        }
        let spans: Vec<_> = app.layout.turns.iter().map(|t| (t.1, t.2)).collect();
        assert_eq!(spans[0].0, 0, "nothing beneath the newest turn");
        assert!(spans.windows(2).all(|w| w[1].0 == w[0].0 + w[0].1 + 1));
    }

    /// With none marked `[` / `]` take the newest turn the window shows (a
    /// turn is shown when one of its rows is), and `[` brings an older turn
    /// in by the least scroll.
    #[test]
    fn the_turn_keys_follow_the_rows_in_view() {
        let mut app = turned(&["a", "b", "c"]);
        let (old, mid, new) = (key_of(&app, 0), key_of(&app, 1), key_of(&app, 2));
        app.layout.page_rows = 5;
        app.layout.turns = vec![(new, 0, 3), (mid, 4, 3), (old, 8, 3)];
        let mark = |app: &mut App, scroll: usize| {
            app.model.set_turn(None);
            app.model.reveal(scroll, scroll + 5, 5);
            (app.step_turn(false), app.model.turn())
        };
        assert_eq!(
            mark(&mut app, 6),
            (true, Some(mid)),
            "newest is out of view"
        );
        assert_eq!(
            mark(&mut app, 3),
            (true, Some(mid)),
            "its top edge is the window's"
        );
        assert_eq!(mark(&mut app, 0), (true, Some(new)), "newest in view");
        assert!(app.step_turn(true));
        assert_eq!(app.model.scroll_rows(5), 2, "the least scroll shows `mid`");
        app.layout.turns = vec![(new, 5, 3)];
        assert_eq!(mark(&mut app, 0), (false, None), "wholly out of the window");
    }

    /// Another core at `2026.10.99`, which the app restarts into, or `why` not.
    fn upgrade_to(restart: Result<(), &str>) -> Answer {
        let restart = restart.map(|()| crate::console::toggle::Restart {
            server: crate::inventory::ServerId::Ambient,
            pane: "%1".to_owned(),
            command: vec!["ae".to_owned(), "app".to_owned(), "api".to_owned()],
        });
        Answer::Core(Some(super::upgrade::Upgrade {
            version: "2026.10.99".to_owned(),
            restart: restart.map_err(str::to_owned),
        }))
    }

    /// U3: an app with nothing to lose restarts into the linked core; writing
    /// holds it back, and so does a draft kept after Esc, while the hint row
    /// says it waits.
    #[test]
    fn a_restart_is_due_only_while_nothing_would_be_lost() {
        let root = Root::new("upgrade");
        let (mut app, _reader) = housed(&root);
        let t0 = Instant::now();
        assert_eq!(app.restart_due(t0), None, "no other core, no restart");
        app.answer(upgrade_to(Ok(())));
        let (version, restart) = app.restart_due(t0).expect("an idle app restarts");
        assert_eq!(
            (version.as_str(), restart.pane.as_str()),
            ("2026.10.99", "%1")
        );
        let waits = "ae 2026.10.99 installed - restarts when idle";
        assert_eq!(app.note().as_deref(), Some(waits));
        let at = writing(&mut app);
        assert_eq!(app.blocker(at), Some(Blocker::Composing));
        assert_eq!(app.compose(Key::Text(b"keep".to_vec()), at), Some(()));
        app.write(false);
        assert!(!app.composing());
        assert_eq!(
            app.blocker(at),
            Some(Blocker::Draft),
            "a kept draft is lost too"
        );
        assert_eq!(app.restart_due(at), None);
        assert_eq!(app.note().as_deref(), Some(waits));
    }

    /// U3: every key read stamps the quiet window a restart waits out; an
    /// armed key outranks the hint and holds it back; a hint-only upgrade,
    /// or none, never restarts.
    #[test]
    fn a_key_holds_a_restart_and_a_hint_only_upgrade_never_restarts() {
        let mut app = app(None);
        app.fleet = one_row(None);
        app.model = Model::new(&app.fleet);
        app.answer(upgrade_to(Ok(())));
        let (mut keys, t0) = (crate::console::input::Keys::app(), Instant::now());
        let (_wake, wakes) = std::sync::mpsc::channel();
        let mut first = Some(loader::Wake::Keys(t0, b"x".to_vec()));
        assert!(super::drain(&mut app, &mut keys, &wakes, &mut first).is_some());
        assert_eq!(app.last_input, Some(t0));
        let quiet = t0 + super::REFRESH;
        let inside = quiet.checked_sub(Duration::from_millis(1)).expect("booted");
        assert_eq!(app.blocker(inside), Some(Blocker::RecentInput));
        assert!(
            app.restart_due(quiet).is_some(),
            "the quiet window has passed"
        );
        assert!(browse(&mut app, &Key::Text(b"q".to_vec()), quiet).is_some());
        assert_eq!(app.note().as_deref(), Some("q again to quit"));
        assert_eq!(app.blocker(quiet), Some(Blocker::Armed));
        app.armed = None;
        app.answer(upgrade_to(Err("auto_upgrade is off")));
        assert_eq!(app.restart_due(quiet), None);
        let only = "ae 2026.10.99 installed - restart ae app to use it";
        assert_eq!(app.note().as_deref(), Some(only));
        app.answer(Answer::Core(None));
        assert_eq!((app.note(), app.restart_due(quiet)), (None, None));
    }

    /// An `aedev` app that has drawn two seats, `colead` highlighted, with a
    /// channel for what it asks the reader.
    fn peeker() -> (App, std::sync::mpsc::Receiver<loader::Request>) {
        use crate::console::needs::SeatRef;
        let (tx, rx) = std::sync::mpsc::channel();
        let server = Some(crate::inventory::ServerId::Ambient);
        let mut app = App::new(Some("aedev".to_owned()), server, None, Some(tx));
        app.fleeted = true;
        app.world
            .sessions
            .push(SessionEntry::new("aedev", Status::Running));
        let mut fleet = one_row(Some("aedev"));
        fleet.rows[0].name = "aedev".to_owned();
        app.model = Model::new(&fleet);
        app.fleet = fleet;
        app.dirs
            .insert("aedev".to_owned(), PathBuf::from("/tmp/aedev"));
        let seat = |slot: &str, name: &str| {
            (
                name.to_owned(),
                Some(SeatRef {
                    slot: slot.to_owned(),
                    name: name.to_owned(),
                }),
            )
        };
        app.drawn = Some(super::Drawn {
            session: "aedev".to_owned(),
            uuid: ID.to_owned(),
            rows: vec![seat("main", "lead"), seat("worker.0", "colead")],
            focused: Some("colead".to_owned()),
        });
        (app, rx)
    }

    fn pressed(app: &mut App, bytes: &[u8]) -> Option<bool> {
        take_keys(app, vec![(Key::Text(bytes.to_vec()), Instant::now())])
    }

    /// `v` on a drawn, highlighted seat opens the panel and asks the reader
    /// once for exactly that seat, pinned to a new generation each open; every
    /// close tells the reader once.
    #[test]
    fn v_opens_the_panel_on_the_highlighted_seat_and_each_open_has_its_own_generation() {
        let (mut app, asks) = peeker();
        assert_eq!(pressed(&mut app, b"v"), Some(true));
        assert!(app.model.peek_open());
        let loader::Request::Peek(Some(ask)) = asks.try_recv().expect("an ask") else {
            panic!("the open asks for a peek");
        };
        assert_eq!(ask.generation, 1);
        assert_eq!((ask.session.as_str(), ask.uuid.as_str()), ("aedev", ID));
        assert_eq!(
            (ask.seat.slot.as_str(), ask.seat.name.as_str()),
            ("worker.0", "colead")
        );
        assert_eq!(ask.dir, PathBuf::from("/tmp/aedev"));
        assert!(asks.try_recv().is_err(), "one ask per open");
        assert_eq!(app.model.peek_generation(), Some(1));
        assert_eq!(
            app.peek.target,
            Some(("aedev".to_owned(), "colead".to_owned()))
        );
        assert_eq!(pressed(&mut app, b"q"), Some(true));
        assert!(!app.model.peek_open());
        assert!(matches!(asks.try_recv(), Ok(loader::Request::Peek(None))));
        assert!(asks.try_recv().is_err(), "one close per close");
        let _ = pressed(&mut app, b"v");
        let loader::Request::Peek(Some(again)) = asks.try_recv().expect("an ask") else {
            panic!("the open asks for a peek");
        };
        assert_eq!(again.generation, 2, "a reopen is a new generation");
    }

    /// A refusal before the panel opens says so on the hint row by seat and
    /// reason, opens nothing and asks nothing.
    #[test]
    fn a_seat_the_frame_did_not_draw_refuses_by_name_and_asks_nothing() {
        let (mut app, asks) = peeker();
        app.drawn.as_mut().expect("drawn").focused = Some("ghost".to_owned());
        let _ = pressed(&mut app, b"v");
        assert!(!app.model.peek_open() && asks.try_recv().is_err());
        let (line, _) = app.flash.clone().expect("a flash");
        assert_eq!(
            line,
            "refused: peek ghost: no seat is shown to open in aedev"
        );
        app.drawn.as_mut().expect("drawn").focused = Some("colead".to_owned());
        app.drawn.as_mut().expect("drawn").session = "other".to_owned();
        let _ = pressed(&mut app, b"v");
        assert!(!app.model.peek_open() && asks.try_recv().is_err());
        let (line, _) = app.flash.clone().expect("a flash");
        assert_eq!(line, "refused: peek aedev: the view changed");
    }

    /// With no session selected `v` says so on the hint row and asks nothing.
    #[test]
    fn v_with_no_session_selected_says_so_and_asks_nothing() {
        let (mut app, asks) = peeker();
        app.model = Model::new(&Fleet::default());
        let _ = pressed(&mut app, b"v");
        assert!(!app.model.peek_open() && asks.try_recv().is_err());
        let said = "refused: peek: no session is selected";
        assert_eq!(
            app.flash.as_ref().map(|(text, _)| text.as_str()),
            Some(said)
        );
        assert!(framed(&mut app).contains(said), "the hint row says it");
    }

    /// A capture that lands after the panel closed, or for another open,
    /// paints nothing and leaves no state behind.
    #[test]
    fn a_late_capture_for_a_closed_or_replaced_panel_is_dropped() {
        let (mut app, _asks) = peeker();
        let answer = |app: &mut App, generation, row: &str| {
            app.answer(Answer::Peek {
                generation,
                at: Instant::now(),
                body: super::Body::Lines(vec![row.to_owned()]),
            });
        };
        answer(&mut app, 1, "BEFORE OPEN");
        assert!(app.peek.shown.is_none() && app.peek.at.is_none());
        let _ = pressed(&mut app, b"v");
        answer(&mut app, 9, "WRONG GENERATION");
        assert!(app.peek.shown.is_none() && app.peek.at.is_none());
        answer(&mut app, 1, "RIGHT ONE");
        assert!(framed(&mut app).contains("RIGHT ONE"));
        let _ = pressed(&mut app, b"v");
        answer(&mut app, 1, "AFTER CLOSE");
        assert!(app.peek.shown.is_none() && app.peek.at.is_none());
        assert!(!framed(&mut app).contains("AFTER CLOSE"));
    }

    /// The open panel's capture ages on screen: one redraw for each new
    /// second, none before the first answer or once closed.
    #[test]
    fn the_open_panel_redraws_once_for_each_new_second_of_age() {
        let (mut app, _asks) = peeker();
        let t0 = Instant::now();
        let after = |millis| t0 + Duration::from_millis(millis);
        let _ = pressed(&mut app, b"v");
        assert!(!app.lapse(after(5000)), "nothing captured yet");
        app.peek.at = Some(t0);
        app.peek.painted = Some(0);
        assert!(!app.lapse(after(999)), "still the second painted");
        assert!(app.lapse(after(1000)), "a new second");
        app.peek.painted = Some(1);
        assert!(!app.lapse(after(1999)));
        assert!(app.lapse(after(2000)));
        app.peek.at = Some(after(3000));
        assert_eq!(app.peek.age(t0), 0, "a capture from the future is 0 s old");
        app.peek.at = Some(t0);
        app.close_peek();
        app.peek.at = Some(t0);
        app.peek.painted = Some(0);
        assert!(!app.lapse(after(9000)), "a closed panel ages nothing");
    }

    /// With the panel open ^C quits, a paste and the mouse's presses die, and
    /// a restart waits for the panel as it does for settings.
    #[test]
    fn the_open_panel_quits_on_ctrl_c_swallows_pastes_and_presses_and_holds_a_restart() {
        let (mut app, _asks) = peeker();
        let _ = pressed(&mut app, b"v");
        let now = Instant::now();
        assert_eq!(app.blocker(now), Some(Blocker::Settings));
        let pasted = vec![(Key::Pasted(b"hello".to_vec()), now)];
        assert_eq!(take_keys(&mut app, pasted), Some(false));
        assert!(app.model.peek_open());
        assert!(!app.composing(), "a paste starts no draft");
        for kind in [MouseKind::Click, MouseKind::Drag, MouseKind::Release] {
            let press = Mouse {
                kind,
                column: 20,
                row: 20,
            };
            assert!(!app.mouse(press), "{kind:?} dies");
            assert!(app.model.peek_open(), "{kind:?} leaves the panel");
        }
        assert_eq!(take_keys(&mut app, vec![(Key::Interrupt, now)]), None);
    }

    /// A close in the middle of a typed chunk hands the rest to the mode the
    /// app is really in: `vq` closes, then `q` arms the quit; `vqq` quits.
    #[test]
    fn a_chunk_that_closes_the_panel_hands_its_rest_to_browse() {
        let (mut app, _asks) = peeker();
        let _ = pressed(&mut app, b"v");
        assert_eq!(pressed(&mut app, b"vq"), Some(true));
        assert!(!app.model.peek_open());
        assert_eq!(app.note().as_deref(), Some("q again to quit"));
        let _ = pressed(&mut app, b"v");
        assert_eq!(pressed(&mut app, b"vqq"), None, "the second q quits");
    }

    /// The pane's state belongs to the model's panel: the same generation
    /// keeps it, another generation or a closed panel starts it afresh.
    #[test]
    fn the_pane_state_starts_afresh_when_the_models_panel_is_not_its_own() {
        let (mut app, _asks) = peeker();
        let holding = || super::PeekPane {
            generation: 1,
            target: Some(("aedev".to_owned(), "colead".to_owned())),
            shown: Some(super::Body::Lines(vec!["old".to_owned()])),
            held: Some(super::Body::Lines(vec!["newer".to_owned()])),
            at: Some(Instant::now()),
            painted: Some(3),
        };
        let afresh = |pane: &super::PeekPane, generation: u64| {
            pane.generation == generation
                && pane.target.is_none()
                && pane.shown.is_none()
                && pane.held.is_none()
                && pane.at.is_none()
                && pane.painted.is_none()
        };
        app.model.open_peek(1);
        app.peek = holding();
        app.peek_sync();
        assert!(!afresh(&app.peek, 1), "its own generation keeps its state");
        assert_eq!(app.peek.painted, Some(3));
        app.model.open_peek(2);
        app.peek_sync();
        assert!(afresh(&app.peek, 2), "another generation starts afresh");
        app.peek = holding();
        app.model.close_peek();
        app.peek_sync();
        assert!(afresh(&app.peek, 0), "a closed panel holds nothing");
    }

    /// The pane starts on the model's generation, every open its own, and
    /// keeps its seat through the first frames: the title names the seat,
    /// before and after a capture.
    #[test]
    fn the_panel_starts_on_the_models_generation_and_its_title_names_the_seat() {
        let (mut app, _asks) = peeker();
        let _ = pressed(&mut app, b"v");
        assert_eq!(app.peek.generation, 1);
        let _ = pressed(&mut app, b"v");
        let _ = pressed(&mut app, b"v");
        assert_eq!(app.peek.generation, 2, "a reopen is its own generation");
        assert!(framed(&mut app).contains("Peek · aedev · colead"));
        app.answer(Answer::Peek {
            generation: 2,
            at: Instant::now(),
            body: super::Body::Lines(vec!["ROW".to_owned()]),
        });
        let shown = framed(&mut app);
        assert!(shown.contains("Peek · aedev · colead") && shown.contains("ROW"));
    }

    /// A capture that aged and any other line that ended lapse in one call,
    /// each saying it did.
    #[test]
    fn an_aged_capture_lapses_together_with_every_line_that_ends() {
        let t0 = Instant::now();
        let at = t0 + super::FLASH_WINDOW + Duration::from_millis(1);
        for (armed, flash, word) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (true, true, true),
        ] {
            let (mut app, _asks) = peeker();
            let _ = pressed(&mut app, b"v");
            app.peek.at = Some(t0);
            app.peek.painted = Some(0);
            app.armed = armed.then_some(super::Armed {
                at: t0,
                what: super::Arm::Quit,
            });
            app.flash = flash.then(|| ("paste not taken: x".to_owned(), t0));
            app.settling = word.then(|| super::Settling {
                at: t0,
                id: "r".to_owned(),
                word: "delivered".to_owned(),
                about: "aedev".to_owned(),
            });
            assert!(app.lapse(at), "{armed} {flash} {word} lapsed");
            let left = (
                app.armed.is_some(),
                app.flash.is_some(),
                app.settling.is_some(),
            );
            assert_eq!(left, (false, false, false), "{armed} {flash} {word}");
        }
    }

    /// A chunk that opens or closes the panel redraws whatever bytes follow it,
    /// also ones the next mode swallows.
    #[test]
    fn a_chunk_that_opens_or_closes_the_panel_redraws_whatever_follows() {
        let (mut app, _asks) = peeker();
        assert_eq!(pressed(&mut app, b"vz"), Some(true), "opens; z dies");
        assert!(app.model.peek_open());
        assert_eq!(
            pressed(&mut app, b"vz"),
            Some(true),
            "closes; z is browse's"
        );
        assert!(!app.model.peek_open());
    }

    /// With the panel open a scroll key redraws and a key it swallows does not.
    #[test]
    fn an_open_panel_redraws_for_a_scroll_and_not_for_a_swallowed_key() {
        let (mut app, _asks) = peeker();
        let _ = pressed(&mut app, b"v");
        assert_eq!(pressed(&mut app, b"k"), Some(true), "up scrolls");
        assert_eq!(app.model.peek_back(), Some(1));
        assert_eq!(pressed(&mut app, b"z"), Some(false), "z is swallowed");
        assert_eq!(app.model.peek_back(), Some(1));
    }

    /// The frame `app` draws, and the cell where `word` starts in it.
    fn shown(app: &mut App, word: &str) -> (Buffer, u16, u16) {
        let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
        app.frame(&mut buf);
        let spelled: Vec<String> = word.chars().map(String::from).collect();
        let (width, height) = (buf.area.width, buf.area.height);
        let found = (0..height).find_map(|y| {
            (0..width).find_map(|x| {
                let fits = (0..)
                    .zip(&spelled)
                    .all(|(at, ch)| x + at < width && buf[(x + at, y)].symbol() == ch.as_str());
                fits.then_some((x, y))
            })
        });
        let (x, y) = found.expect("a drawn word");
        (buf, x, y)
    }

    fn lit(buf: &Buffer, y: u16) -> Vec<u16> {
        let inverse = |x: &u16| {
            let cell = &buf[(*x, y)];
            cell.modifier
                .contains(ratatui_core::style::Modifier::REVERSED)
        };
        (0..buf.area.width).filter(inverse).collect()
    }

    fn at(kind: MouseKind, column: u16, row: u16) -> Mouse {
        Mouse { kind, column, row }
    }

    fn staged(app: &App) -> Option<&str> {
        match app.select.as_ref().map(|held| &held.stage) {
            Some(super::Stage::Ready(text)) => Some(text),
            _ => None,
        }
    }

    /// The drag paints the cells it covers, the release asks for a copy of
    /// the cells a frame painted - the release cell included, though no drag
    /// event reached it - and the copy leaves only after that frame.
    #[test]
    fn a_drag_paints_its_cells_and_the_release_ships_what_the_frame_painted() {
        let mut app = turned(&["alpha beta gamma", "second"]);
        let (_, x, y) = shown(&mut app, "alpha beta gamma");
        assert!(app.mouse(at(MouseKind::Click, x + 6, y)));
        let held = app.select.as_ref().expect("a press arms");
        assert!(held.down && !held.sel.moved());
        assert!(app.model.turn().is_some(), "the press marked the turn");
        assert!(app.mouse(at(MouseKind::Drag, x + 8, y)));
        assert_eq!(app.model.turn(), None, "motion puts the mark back");
        assert!(app.mouse(at(MouseKind::Release, x + 9, y)));
        assert_eq!(staged(&app), None, "nothing leaves before a frame");
        let (buf, ..) = shown(&mut app, "alpha beta gamma");
        assert_eq!(lit(&buf, y), (x + 6..=x + 9).collect::<Vec<_>>());
        assert_eq!(staged(&app), Some("beta"), "the very cells painted");
        assert!(app.ship_staged());
        let said = app.flash.as_ref().map(|(line, _)| line.as_str());
        assert!(said.is_some_and(|line| line.starts_with("sent 1 line to the terminal")));
        assert!(!app.ship_staged(), "a copy ships once");
        let (buf, ..) = shown(&mut app, "alpha");
        assert_eq!(lit(&buf, y).len(), 4, "the highlight stays");
    }

    /// A press and release with no motion is the click it always was: the
    /// turn stays marked, nothing is staged, nothing is highlighted.
    #[test]
    fn a_plain_click_selects_nothing_and_keeps_its_old_behaviour() {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        assert!(app.mouse(at(MouseKind::Click, x + 2, y)));
        let marked = app.model.turn();
        assert!(marked.is_some());
        let _ = app.mouse(at(MouseKind::Release, x + 2, y));
        assert!(app.select.is_none() && !app.ship_staged());
        assert_eq!(app.model.turn(), marked, "the mark stays");
        let (buf, ..) = shown(&mut app, "alpha");
        assert!(lit(&buf, y).is_empty());
    }

    /// A press anywhere ends the selection, and a copy staged but not yet
    /// shipped goes with it; so does Esc, which is consumed.
    #[test]
    fn a_press_or_esc_ends_a_selection_and_its_unshipped_copy() {
        for esc in [false, true] {
            let mut app = turned(&["alpha beta gamma"]);
            let (_, x, y) = shown(&mut app, "alpha");
            app.mouse(at(MouseKind::Click, x, y));
            app.mouse(at(MouseKind::Drag, x + 3, y));
            app.mouse(at(MouseKind::Release, x + 3, y));
            if esc {
                let keys = vec![(Key::Escape, Instant::now())];
                assert_eq!(take_keys(&mut app, keys), Some(true));
                assert!(app.model.selected().is_some(), "Esc only cleared it");
            } else {
                app.mouse(at(MouseKind::Click, 100, 44));
            }
            assert!(app.select.is_none());
            let (buf, ..) = shown(&mut app, "alpha");
            assert!(lit(&buf, y).is_empty() && !app.ship_staged(), "esc={esc}");
        }
    }

    /// A panel change drops the selection even when the panel changes back
    /// to the very cells it held; a changed cell drops it too, saying so.
    #[test]
    fn a_selection_dies_with_its_panel_and_with_its_cells() {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        let drag = |app: &mut App| {
            app.mouse(at(MouseKind::Click, x, y));
            app.mouse(at(MouseKind::Drag, x + 3, y));
        };
        drag(&mut app);
        app.model.open_settings(1);
        app.model.close_settings();
        assert!(app.stale_selection(), "away and back is still a change");
        assert!(app.select.is_none());
        drag(&mut app);
        app.mouse(at(MouseKind::Release, x + 3, y));
        app.lane = Rc::new(Lane {
            items: vec![said("omega beta gamma", 0)],
            coverage: Vec::new(),
        });
        let (buf, ..) = shown(&mut app, "omega");
        assert!(app.select.is_none() && lit(&buf, y).is_empty());
        let said = app.flash.as_ref().map(|(line, _)| line.as_str());
        assert_eq!(said, Some("selection cleared: the screen changed"));
        assert!(!app.ship_staged() && std::mem::take(&mut app.again));
    }

    /// A press on a frame a later one replaced arms nothing and still ends
    /// a selection held; a border grab arms nothing either.
    #[test]
    fn a_stale_press_and_a_border_grab_arm_no_selection() {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        app.mouse(at(MouseKind::Click, x, y));
        app.mouse(at(MouseKind::Drag, x + 3, y));
        assert!(app.mouse_in(at(MouseKind::Click, x, y), false));
        assert!(app.select.is_none(), "it ended the old, armed no new one");
        let rule = (0..160).find(|c| app.layout.grab(at(MouseKind::Click, *c, y)).is_some());
        let rule = rule.expect("a sidebar rule on the row");
        app.mouse(at(MouseKind::Click, rule, y));
        assert!(app.select.is_none(), "the grab wins");
    }

    /// A press that takes a lit selection away asks for the redraw that
    /// removes the highlight, on the frame on screen and on one a later frame
    /// replaced, though the press itself acts on nothing.
    #[test]
    fn a_press_that_ends_a_highlight_asks_for_the_redraw_on_either_frame() {
        for historical in [false, true] {
            let mut app = turned(&["alpha beta gamma"]);
            let origin = Instant::now();
            let (_, x, y) = shown(&mut app, "alpha");
            let press = |app: &mut App| {
                let inert = at(MouseKind::Click, 100, 44);
                if historical {
                    app.mouse_made(inert, origin)
                } else {
                    app.mouse(inert)
                }
            };
            assert!(
                !press(&mut app),
                "{historical}: on its own it acts on nothing"
            );
            app.mouse(at(MouseKind::Click, x, y));
            app.mouse(at(MouseKind::Drag, x + 3, y));
            let (buf, ..) = shown(&mut app, "alpha");
            let held = app.select.as_ref().expect("a drag holds a selection");
            assert!(held.sel.moved() && !lit(&buf, y).is_empty(), "{historical}");
            assert!(
                press(&mut app),
                "{historical}: the highlight must be redrawn away"
            );
            assert!(app.select.is_none(), "{historical}: the press ended it");
            let (buf, ..) = shown(&mut app, "alpha");
            assert!(lit(&buf, y).is_empty(), "{historical}: nothing stays lit");
        }
    }

    /// Every motion that moves the lit end asks for a frame, not only the
    /// first one: the highlight follows the pointer.
    #[test]
    fn each_drag_that_moves_the_lit_end_asks_for_a_frame() {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        app.mouse(at(MouseKind::Click, x, y));
        assert!(app.mouse(at(MouseKind::Drag, x + 1, y)), "the first motion");
        let _ = shown(&mut app, "alpha");
        assert!(app.mouse(at(MouseKind::Drag, x + 3, y)), "a later motion");
        let (buf, ..) = shown(&mut app, "alpha");
        assert_eq!(lit(&buf, y), (x..=x + 3).collect::<Vec<_>>());
    }

    /// A frame is owed after a shipped copy and after a hint with no room,
    /// and only then; a waiting hint is taken.
    #[test]
    fn a_frame_is_owed_for_a_shipped_copy_or_a_waiting_hint() {
        for (copy, hint) in [(false, false), (true, false), (false, true)] {
            let mut app = turned(&["alpha beta gamma"]);
            let (_, x, y) = shown(&mut app, "alpha");
            if copy || hint {
                app.mouse(at(MouseKind::Click, x, y));
                app.mouse(at(MouseKind::Drag, x + 3, y));
                app.mouse(at(MouseKind::Release, x + 3, y));
            }
            if hint {
                // The screen changes before the copy leaves: dropped, with a hint.
                app.lane = Rc::new(Lane {
                    items: vec![said("omega beta gamma", 0)],
                    coverage: Vec::new(),
                });
                let _ = shown(&mut app, "omega");
            } else if copy {
                let _ = shown(&mut app, "alpha");
            }
            assert_eq!(app.again, hint, "copy {copy}, hint {hint}: a hint waits");
            assert_eq!(app.shipped(), copy || hint, "copy {copy}, hint {hint}");
            assert!(!app.again, "copy {copy}, hint {hint}: the hint is taken");
        }
    }

    /// The loop settles the copy and the queued asks together: a frame is due
    /// when either ran, and each runs whatever the other did.
    #[test]
    fn a_frame_settles_the_copy_and_the_queued_asks_together() {
        for (copy, ask) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut app = turned(&["alpha beta gamma"]);
            let (_, x, y) = shown(&mut app, "alpha");
            if copy {
                app.mouse(at(MouseKind::Click, x, y));
                app.mouse(at(MouseKind::Drag, x + 3, y));
                app.mouse(at(MouseKind::Release, x + 3, y));
                let _ = shown(&mut app, "alpha");
            }
            if ask {
                app.queued.push(super::Queued {
                    key: ("api".to_owned(), "id".to_owned()),
                    entry: 0,
                    raw: b"hi".to_vec(),
                    seat: "lead".to_owned(),
                    body: "hi".to_owned(),
                    about: "api".to_owned(),
                });
            }
            assert_eq!(app.owed(), copy || ask, "copy {copy}, ask {ask}");
            assert!(app.queued.is_empty(), "copy {copy}, ask {ask}: asks ran");
            assert_eq!(app.flash.is_some(), copy, "copy {copy}, ask {ask}: shipped");
        }
    }

    /// A selection that lost nothing a person saw or asked for - a press that
    /// never moved, or a highlight whose copy already left - goes without a
    /// word when the screen changes under it.
    #[test]
    fn a_selection_that_lost_nothing_goes_quietly_when_the_screen_changes() {
        for copied in [false, true] {
            let mut app = turned(&["alpha beta gamma"]);
            let (_, x, y) = shown(&mut app, "alpha");
            app.mouse(at(MouseKind::Click, x, y));
            if copied {
                app.mouse(at(MouseKind::Drag, x + 3, y));
                app.mouse(at(MouseKind::Release, x + 3, y));
                let _ = shown(&mut app, "alpha");
                assert!(app.ship_staged(), "the copy left");
            }
            let before = app.flash.as_ref().map(|(line, _)| line.clone());
            app.lane = Rc::new(Lane {
                items: vec![said("omega beta gamma", 0)],
                coverage: Vec::new(),
            });
            let (buf, ..) = shown(&mut app, "omega");
            assert!(app.select.is_none() && lit(&buf, y).is_empty(), "{copied}");
            let after = app.flash.as_ref().map(|(line, _)| line.clone());
            assert_eq!(after, before, "{copied}: nothing new is said");
            assert!(!std::mem::take(&mut app.again), "{copied}: no hint owed");
        }
    }

    /// A key that finds a lit selection left behind by a panel change asks for
    /// the redraw that removes the highlight, though the key itself does
    /// nothing.
    #[test]
    fn a_key_after_a_panel_change_redraws_the_stale_highlight_away() {
        for stale in [false, true] {
            let mut app = turned(&["alpha beta gamma"]);
            let (_, x, y) = shown(&mut app, "alpha");
            if stale {
                app.mouse(at(MouseKind::Click, x, y));
                app.mouse(at(MouseKind::Drag, x + 3, y));
                let _ = shown(&mut app, "alpha");
                app.model.open_settings(1);
                app.model.close_settings();
            }
            let keys = vec![(Key::Text(b"~".to_vec()), Instant::now())];
            assert_eq!(take_keys(&mut app, keys), Some(stale), "stale {stale}");
            assert!(app.select.is_none(), "stale {stale}");
        }
    }

    /// In a lane taller than the screen the marked turn's bar is blanked on
    /// the very cells it is drawn on, whichever turns scrolled out above.
    #[test]
    fn a_marked_turns_bar_is_blanked_where_it_is_drawn_in_a_long_lane() {
        let bodies: Vec<String> = (0..80).map(|n| format!("turn{n:02} text")).collect();
        let mut app = turned(&bodies.iter().map(String::as_str).collect::<Vec<_>>());
        let key = key_of(&app, 79);
        app.model.set_turn(Some(key));
        let (buf, _, y) = shown(&mut app, "turn79");
        let left = app.layout.map.region(select::Panel::Lane).map(|r| r.area.x);
        let left = left.expect("a lane region");
        assert!(matches!(buf[(left, y)].symbol(), "│" | "|"), "the bar");
        let bars =
            (0..buf.area.height).filter(|row| matches!(buf[(left, *row)].symbol(), "│" | "|"));
        for row in bars {
            let cell = ratatui_core::layout::Position::new(left, row);
            let blank = app.layout.map.blanks.iter().any(|b| b.contains(cell));
            assert!(blank, "the bar on row {row} is a recorded blank");
        }
    }

    /// Every session row the list draws is inside the list's region, the last
    /// one included.
    #[test]
    fn the_list_region_holds_every_drawn_session_row() {
        let mut app = turned(&["alpha"]);
        app.fleet = Fleet {
            rows: vec![row("api", 1, true), row("zzzlast", 2, false)],
            home: Some("api".to_owned()),
        };
        app.model = Model::new(&app.fleet);
        let (_, x, y) = shown(&mut app, "zzzlast");
        let at = ratatui_core::layout::Position::new(x, y);
        let panel = app.layout.map.region_at(at).map(|region| region.panel);
        assert_eq!(panel, Some(select::Panel::List), "the last drawn row");
    }

    /// The bar a marked turn wears is chrome: the cell is a recorded blank,
    /// and dragging across it copies the very bytes the unmarked turn gives.
    #[test]
    fn a_marked_turns_bar_is_never_copied() {
        let copied = |marked: bool| {
            let mut app = turned(&["alpha beta", "other"]);
            if marked {
                let key = key_of(&app, 0);
                app.model.set_turn(Some(key));
            }
            let (buf, x, y) = shown(&mut app, "alpha");
            let left = app.layout.map.region(select::Panel::Lane).map(|r| r.area.x);
            let left = left.expect("a lane region");
            let bar = app.layout.map.blanks.iter().find(|blank| blank.y == y);
            assert_eq!(bar.is_some(), marked, "only a marked turn has a bar");
            if marked {
                assert!(matches!(buf[(left, y)].symbol(), "│" | "|"), "the bar");
            }
            app.mouse(at(MouseKind::Click, left, y));
            app.mouse(at(MouseKind::Drag, x + 4, y));
            app.mouse(at(MouseKind::Release, x + 4, y));
            let _ = shown(&mut app, "alpha");
            staged(&app).map(str::to_owned)
        };
        let (marked, plain) = (copied(true), copied(false));
        assert_eq!(marked, plain, "marked and unmarked bytes agree");
        assert!(marked.is_some_and(|text| text.ends_with("alpha")));
    }

    /// A press whose release never came is not a selection: it holds nothing
    /// for Esc, which closes the panel as it always did.
    #[test]
    fn esc_after_a_press_with_no_motion_keeps_its_usual_job() {
        let (mut app, _asks) = peeker();
        let _ = pressed(&mut app, b"v");
        app.peek.shown = Some(super::peek::Body::Lines(vec!["alphaZ".to_owned()]));
        let (_, x, y) = shown(&mut app, "alphaZ");
        app.mouse(at(MouseKind::Click, x, y));
        assert!(app.select.is_some(), "the press is armed");
        let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
        assert!(!app.model.peek_open(), "Esc closed the panel");
        assert!(app.select.is_none(), "and dropped the arm");
    }

    /// A drag whose motion reports were coalesced away - the release lands on
    /// another cell - is still a drag: the mark its press toggled goes back.
    #[test]
    fn a_release_on_another_cell_puts_back_the_mark_the_press_toggled() {
        let mut app = turned(&["alpha beta gamma"]);
        let (_, x, y) = shown(&mut app, "alpha");
        app.mouse(at(MouseKind::Click, x, y));
        assert!(app.model.turn().is_some(), "the press marked the turn");
        app.mouse(at(MouseKind::Release, x + 4, y));
        assert_eq!(app.model.turn(), None, "a drag leaves the mark as it was");
        let _ = shown(&mut app, "alpha");
        assert_eq!(staged(&app), Some("alpha"));
    }

    /// A view handed over for `web`, written now.
    fn handed(pages: usize, rows: usize) -> Answer {
        let view = super::handover::View {
            session: "web".to_owned(),
            id: ID.to_owned(),
            tab: Tab::Agents,
            focus: None,
            sidebar: None,
            list: None,
            pages,
            rows,
            list_top: None,
            body_top: 0,
        };
        let at = Timestamp::now().epoch();
        Answer::Handover { view, at }
    }

    /// A raw read that decodes to no key - a lone Esc, a paste start, a
    /// dropped `^Z` - still ends a handed-over view, waiting or yet to come.
    #[test]
    fn raw_input_that_decodes_to_no_key_cancels_a_handed_over_view() {
        for raw in [&b"\x1b"[..], b"\x1b[200~", b"\x1a"] {
            for waiting in [true, false] {
                let root = Root::new("raw-cancel");
                session(&root, "web", "");
                let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
                let mut app = App::new(Some("api".to_owned()), None, None, None);
                let (wake, wakes) = std::sync::mpsc::channel();
                let mut keys = crate::console::input::Keys::app();
                let sent = loader::Wake::Keys(Instant::now(), raw.to_vec());
                let late = loader::Wake::Answer(Box::new(handed(0, 0)));
                if waiting {
                    app.answer(handed(0, 0));
                    wake.send(sent).expect("sent");
                } else {
                    wake.send(sent).expect("sent");
                    wake.send(late).expect("sent");
                }
                assert!(super::drain(&mut app, &mut keys, &wakes, &mut None).is_some());
                assert!(
                    app.touched && app.keep.is_none(),
                    "{raw:?} waiting={waiting}"
                );
                let live = [("api", Status::Running), ("web", Status::Running)];
                refold(&mut app, &reader, &root, &live);
                assert_eq!(app.model.selected(), Some("api"), "{raw:?} no late yank");
            }
        }
    }

    /// A restored scroll waits for the lane it scrolls: the frame before the
    /// session's first read keeps it, and the read then bounds it as ever.
    #[test]
    fn a_restored_scroll_survives_the_frame_before_its_lane_is_read() {
        let root = Root::new("keep-scroll");
        session(&root, "web", "");
        let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
        let mut app = App::new(Some("api".to_owned()), None, None, None);
        app.answer(handed(2, 3));
        let live = [("api", Status::Running), ("web", Status::Running)];
        refold(&mut app, &reader, &root, &live);
        assert_eq!(app.model.selected(), Some("web"));
        assert!(app.loading, "web's lane is not read yet");
        let _ = framed(&mut app);
        assert_eq!(app.model.scroll_rows(10), 23, "loading frame keeps it");
        app.answer(Answer::View(ViewRead {
            name: "web".to_owned(),
            id: ID.to_owned(),
            seq: 1,
            lane: Lane {
                items: (0..400).map(|at| said(&format!("turn {at}"), at)).collect(),
                coverage: Vec::new(),
            },
            needs: None,
            roster: None,
        }));
        let _ = framed(&mut app);
        assert_ne!(app.model.scroll_rows(10), 0, "a long history keeps it");
        assert!(!app.loading);
    }

    /// Cold `2q` arms quit after queueing the 2: its replay keeps the press.
    #[test]
    fn a_press_armed_before_the_fleet_survives_the_replay_of_the_keys_before_it() {
        let root = Root::new("early-armed");
        session(&root, "web", "");
        let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
        let mut app = App::new(Some("api".to_owned()), None, None, None);
        let at = Instant::now();
        let _ = take_keys(&mut app, vec![(Key::Text(b"2q".to_vec()), at)]);
        let live = [("api", Status::Running), ("web", Status::Running)];
        refold(&mut app, &reader, &root, &live);
        assert_eq!(app.model.selected(), Some("web"), "the 2 was replayed");
        let again = vec![(Key::Text(b"q".to_vec()), at + Duration::from_millis(1))];
        assert!(take_keys(&mut app, again).is_none(), "the second q quits");
    }

    /// A handed-over view finds its session when that is the fleet's only row.
    #[test]
    fn a_handed_over_view_restores_on_a_fleet_whose_only_row_is_its_session() {
        let root = Root::new("sole-row");
        session(&root, "web", "");
        let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
        let mut app = App::new(Some("api".to_owned()), None, None, None);
        app.answer(handed(2, 3));
        refold(&mut app, &reader, &root, &[("web", Status::Running)]);
        assert_eq!(app.model.selected(), Some("web"));
        assert_eq!(app.model.tab(), Tab::Agents, "the tab was restored");
        assert_eq!(app.model.scroll_rows(10), 23, "and the scroll");
        assert!(app.keep.is_none(), "applied once");
    }

    /// Once its lane is read, a restored scroll is bound by it and waits no more.
    #[test]
    fn a_restored_scroll_is_bounded_by_the_lane_once_it_is_read() {
        let root = Root::new("bound-scroll");
        session(&root, "web", "");
        let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
        let mut app = App::new(Some("api".to_owned()), None, None, None);
        app.answer(handed(2, 3));
        let live = [("api", Status::Running), ("web", Status::Running)];
        refold(&mut app, &reader, &root, &live);
        app.answer(view("web", ID, 1, "short"));
        let _ = framed(&mut app);
        assert_eq!(app.model.scroll_rows(10), 0, "a short lane bounds it");
        assert!(!app.restored, "and the wait is over");
    }

    /// A key dropped before the first fleet read says so on the hint row now.
    #[test]
    fn a_key_dropped_before_the_first_fleet_read_repaints_its_notice() {
        let mut app = App::new(Some("api".to_owned()), None, None, None);
        let dropped = take_keys(&mut app, vec![(Key::Text(b"v".to_vec()), Instant::now())]);
        assert_eq!(dropped, Some(true), "the notice repaints before the fleet");
        let said = "input before sessions loaded was dropped";
        assert_eq!(app.note().as_deref(), Some(said));
    }
}
