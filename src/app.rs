//! `ae app`: the Side Quiet layout — the fleet in a sidebar, the selected
//! session's Agents beside it, and the selected session's chat.
//!
//! The pure halves live in the submodules: [`fleet`] folds the sidebar rows,
//! [`overview`] what waits on the human and the working dir, [`model`] the browse reducer and [`draw`] the
//! cells. Each reads only what the existing owners already computed. The
//! world is read on the [`loader`]'s one background thread, through those
//! owners; this file draws what it last answered, so no key waits on a read.
//! The ONE write is the composer's ask or close into the selected session,
//! through the chat's own admission path (`term::submit_ask`,
//! `submit::close_owned`), under that session's writer lease, taken by the
//! submit and held through its delivery; composing holds none, so every window
//! writes at once. Clipboard workers publish inert images for the draft. Opening a
//! seat writes nothing either: it hands one tmux client to the seat's pane,
//! through the chat's own proof (`term::prove_open`).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use ratatui_core::buffer::{Buffer, CellDiffOption};
use ratatui_core::layout::Position;
use ratatui_core::terminal::Terminal;

use crate::board::terminal_text;
use crate::console::input::{Effect, Input, Key, Keys, Mouse, MouseKind, Reading, Size, View};
use crate::console::lane::{Item, Kind, Lane, Seat};
use crate::console::link;
use crate::console::needs::{SeatRef, Section};
use crate::console::open::{self, Move, Refusal};
use crate::console::{self, Console, shell, submit, term, toggle};
use crate::digest::{SessionEntry, Status};
use crate::inventory::ServerId;
use crate::listing::World;
use crate::session_menu::CellRow;
use crate::time::Timestamp;
use crate::{doors, theme, tmux, transport};

use frame::draw_frame;
use loader::{Answer, Binding, EndingAsk, PeekAsk, ReadAsk, RecordsAsk, Request, Wake};
use mouse::{Held, Press, Stage};
use panel::{Opening, Orch, PanelByte};
use peek::Body;
#[cfg(test)]
use run::{Outside, Step, drain, operands, says_hint, settle, step};
pub use run::{exit_hint, run};
use settings_flow::{settings_byte, settings_key};
use viewer::{PeekPane, ReaderPane, peek_key, peek_keys, reader_key, reader_keys};

#[cfg(test)]
#[path = "app/applink_spec.rs"]
mod applink_spec;
#[cfg(test)]
#[path = "app/apppolish3_spec.rs"]
mod apppolish3_spec;
#[cfg(test)]
#[path = "app/appread_spec.rs"]
mod appread_spec;
mod backend;
mod cue;
#[cfg(test)]
mod dechist_spec;
pub mod draw;
#[cfg(test)]
mod early_settings_spec;
#[cfg(test)]
#[path = "../tests/it/app_early_spec.rs"]
mod early_spec;
mod filedoor;
pub mod fleet;
#[cfg(test)]
mod fleet_spec;
mod frame;
pub mod handover;
mod journals;
mod lane;
#[cfg(test)]
mod lane_spec;
mod lanemd;
#[cfg(test)]
#[path = "app/lanemd_app_spec.rs"]
mod lanemd_app_spec;
#[cfg(test)]
#[path = "app/lanemd_spec.rs"]
mod lanemd_cache_spec;
mod lifecycle;
#[cfg(test)]
#[path = "app/linkhint_spec.rs"]
mod linkhint_spec;
mod loader;
mod maintainer;
pub mod markdown;
#[cfg(test)]
#[path = "app/mdreader_spec.rs"]
mod mdreader_spec;
mod menu;
mod modal;
pub mod model;
mod mouse;
#[cfg(test)]
#[path = "app/navigate_spec.rs"]
mod navigate_spec;
#[cfg(test)]
mod order_spec;
pub mod overview;
mod panel;
mod paneview;
#[cfg(test)]
mod paneview_spec;
pub mod peek;
pub mod reader;
#[cfg(test)]
#[path = "app/readerlink_spec.rs"]
mod readerlink_spec;
pub(crate) mod route;
mod run;
#[cfg(test)]
mod s5_app;
pub mod select;
#[cfg(test)]
mod select_spec;
mod selection;
mod settings;
mod settings_flow;
#[cfg(test)]
mod skilltab_inner_spec;
#[cfg(test)]
mod skilltab_spec;
#[cfg(test)]
mod stablesplit_spec;
#[cfg(test)]
mod stats_spec;
#[cfg(test)]
mod tools_keys_tests;
#[cfg(test)]
mod tools_spec;
mod tty;
pub mod upgrade;
#[cfg(test)]
#[path = "../tests/it/app_view_state_spec.rs"]
mod view_spec;
mod viewer;

#[cfg(test)]
#[path = "../tests/it/app_longpaste_spec.rs"]
mod longpaste_spec;

#[cfg(test)]
#[path = "../tests/it/app_livepeek_spec.rs"]
mod livepeek_spec;

#[cfg(test)]
#[path = "../tests/it/app_takekeys_spec.rs"]
mod takekeys_spec;

/// What `ae app` says, on stderr, when it has no terminal to draw on.
pub const NO_TERMINAL: &str = "ae app draws a terminal UI; use ae chat to print the lane\n";

/// What a hand-typed `ae app` that opens the app says on stderr, once.
pub const HINT: &str = "ae app: plain ae opens the app";

/// The internal argument an app start ae makes itself carries, never shown in
/// the usage text: it says no [`HINT`]. A session name can never be it.
pub const OWN: &str = "--app-own";

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
/// How long after a clear an empty-composer ^C only warns, so a second ^C
/// meant for the text never ends the app.
const QUIT_GUARD: Duration = Duration::from_secs(2);
/// How long a refused paste or an outcome word stays on the hint row.
const FLASH_WINDOW: Duration = Duration::from_secs(5);
/// The most UTF-8 bytes one copy writes to the terminal clipboard outside tmux;
/// more is refused, never cut.
const CLIP_MAX: usize = 65536;
/// How long a drag holds the reads back without any mouse input.
const LANE_HOLD: Duration = Duration::from_secs(10);
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
    /// ^C over an empty composer just after a clear.
    Interrupt,
    /// `o` on the frame it saw: the second press opens that frame's seat.
    Open(Drawn),
}

impl Arm {
    /// The hint-row line that says what the next press does.
    fn line(&self) -> String {
        match self {
            Self::Quit => "q again to quit".to_owned(),
            Self::Interrupt => "^C again to quit · ^Y brings draft back".to_owned(),
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
    /// The quiet lane note for a line routed away from the lead.
    note: Option<String>,
}

/// The one ask on its way to its seat, carried off the loop's thread.
struct Flight {
    id: String,
    seat: String,
    /// The session the ask went to.
    about: String,
    /// The quiet lane note for a line routed away from the lead.
    note: Option<String>,
    /// Its outcome, once the delivery has ended.
    done: Receiver<submit::Outcome>,
    /// The writer lease its submit took on that very incarnation: held until
    /// the delivery ends, whatever happens to the writing, so no other window
    /// is admitted beside it.
    lease: Arc<LeaseFile>,
}

/// How long a quit waits for an ask in flight before it leaves it: the helpers'
/// own delivery gives up by itself well inside it (the send lock waits two
/// minutes, the quiet gate [`crate::deliver::DEFAULT_DEFER`], the submit
/// proof seconds more).
const FLIGHT_QUIT_BOUND: Duration = Duration::from_mins(4);

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

/// The orchestrator's session name, which never is a row of the list.
const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;

/// The writer lease's file, taken by a submit and kept by its ask in flight: it
/// is released when the last holder lets go.
struct LeaseFile {
    /// Held open: dropping it releases the lease.
    file: File,
}

impl Drop for LeaseFile {
    /// Empty the holder record before the lock goes (a crash skips this).
    fn drop(&mut self) {
        let _ = crate::store::SessionStore::console_writer_note(&self.file, b"");
    }
}

/// This window's entry into writing: per window, with no writer or admission
/// lock held.
struct Entry {
    /// When it began: no key read before it is draft.
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
    reason = "independent facts of one loop, each read where it is set: loading, fleeted, touched, restored, early_named, again, stopped_open"
)]
struct App {
    home: Option<String>,
    model: model::Model,
    fleet: fleet::Fleet,
    /// The stopped sessions the closed group holds back from `fleet`, in the
    /// fleet's order; empty while the group is open.
    hidden: Vec<fleet::Row>,
    /// Whether the stopped group is open: this window's memory, closed at
    /// start, never persisted and never handed over.
    stopped_open: bool,
    world: World,
    dirs: BTreeMap<String, PathBuf>,
    ids: BTreeMap<String, String>,
    /// The `ids` the last fleet read found named exactly once.
    sole: BTreeMap<String, String>,
    facts: BTreeMap<String, fleet::Facts>,
    needs: BTreeMap<String, Section>,
    /// Each session's lead pair as the last fleet read found it.
    pairs: BTreeMap<String, Result<Vec<Seat>, String>>,
    /// Each session's last read lane, by name: what a selection shows at once.
    shown: BTreeMap<String, Shown>,
    /// Each read session's lead-pair quota pressure by seat name, with the
    /// `session_id` it was read for.
    pressures: BTreeMap<String, (String, loader::Pressure)>,
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
    recovery: submit::Recovery,
    /// The lead pair each incarnation was first written with: a changed pair
    /// refuses rather than sending elsewhere.
    bound: BTreeMap<(String, String), Vec<Seat>>,
    /// Why the selection's composer only reads.
    read_only: String,
    /// The entry this window writes under, exactly while it writes.
    entered: Option<Entry>,
    /// The entry writing now, bound to what it proved; with `entered`.
    writer: Option<Binding>,
    clipboard: crate::clipboard::Job<((String, String), u64)>,
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
    /// A seat the human asked to see interactive, outside tmux: the loop
    /// hands the terminal over once this frame is out.
    pane_view: Option<paneview::Ask>,
    /// When a pane view gave the terminal back: reads from before are dropped.
    resumed_at: Option<Instant>,
    /// The settings overlay's bodies, cold until its answers land.
    settings_bodies: settings::SettingsBodies,
    /// When an own-generation answer last replaced the quota body: a kept body's age.
    quota_at: Option<i64>,
    /// The last refresh failed and the held body is kept: its line says so.
    quota_failed: bool,
    /// The generation the next open mints; answers pin to the open one.
    settings_generation: u64,
    /// The open generation whose shown Statistics tab the reader watches.
    stats_shown: Option<u64>,
    /// The tool-statistics page the reader was last told is shown: its open
    /// generation and its ask.
    tools_asked: Option<(u64, crate::tool_stats::Ask)>,
    /// The open generation whose `r` is owed to the upkeep, not yet handed.
    stats_reset: Option<u64>,
    /// The open peek panel's captures, and the generation the next open mints.
    peek: PeekPane,
    peek_generation: u64,
    /// The open reader's file, and the generation the next open mints.
    reader: ReaderPane,
    reader_generation: u64,
    /// The generation the next Activity… or Memos… read mints; answers pin to
    /// the menu that asked.
    records_generation: u64,
    /// The cache that draws agent turns through Markdown, shared by the
    /// frame and the keys that read a turn's links.
    md: RefCell<lanemd::Cache>,
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
    /// When ^C or ^U last cleared a draft: see [`QUIT_GUARD`].
    cleared_at: Option<Instant>,
    /// Why a paste was not taken, until its window ends.
    flash: Option<(String, Instant)>,
    /// The lane the turn fingerprints were built from, with them.
    keyed: Option<(Rc<Lane>, Rc<[u64]>)>,
    /// The read of the browse paste that started this entry: its later
    /// fragments keep their text, nothing else read before the entry does.
    admitted: Option<Instant>,
    /// Asks waiting for the loop to deliver them.
    queued: Vec<Queued>,
    /// The ask being carried to its seat, until its outcome lands.
    flight: Option<Flight>,
    /// The outcome word of the last ask, until the lane shows it.
    settling: Option<Settling>,
    /// The long turns this app shows whole, by [`App::fold_key`]. Memory only.
    expanded: BTreeSet<u64>,
    /// The asks this app saw go through, by [`App::sent_key`]. Memory only.
    sent: BTreeSet<u64>,
    /// What the read-only composer says: `read_only`, with how to resume.
    read_line: String,
    /// Another installed core, as the last fleet read found the command link.
    upgrade: Option<upgrade::Upgrade>,
    /// The version an exec restart failed into, and why: never tried again.
    failed: Option<(String, String)>,
    /// When the last key, click or paste was read.
    last_input: Option<Instant>,
    /// The drag in progress, or the highlight it left.
    select: Option<Held>,
    /// The open panel's column, not the main column, drew the text `select`
    /// holds: its frames are checked against that column's map.
    select_on_panel: bool,
    /// A link press bound to its pointer and panel epoch until release.
    read_press: Option<(Press, Position, u64)>,
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
    /// The newest read of each kind, kept back while a drag holds the screen.
    parked: Vec<Answer>,
    /// The open session menu or Stop question: it takes every key first.
    modal: Option<menu::Modal>,
    /// Every key read at or before this was spent on a modal: dropped, never
    /// replayed into the mode under it.
    cut: Option<Instant>,
    /// The state root a Stop is handed under; `None` for an app driven by hand.
    root: Option<PathBuf>,
    /// When each session was last shown selected: the unread cue's memory.
    cue: cue::Cue,
    /// How an admitted ask is carried to its seat: the helpers' tracked path.
    carry: Carry,
    /// The orchestrator, which has its own panel instead of a row.
    orch: Orch,
    /// How a process that must outlive this app is started: Stop's supervisor
    /// and a Start outside tmux.
    detach: Detach,
}

/// The delivery of one admitted ask, which says what the path wrote.
type Carry = Arc<dyn Fn(term::Carry) -> String + Send + Sync>;

/// Whether a detached argv started.
type Detach = Box<dyn Fn(&crate::lifecycle::DetachedArgv) -> bool>;

impl App {
    /// An app that has read nothing yet: home is selected. Where it runs
    /// decides nothing: each window writes under its own entry.
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
        let orch = Orch::homed(home.as_deref());
        Self {
            home,
            model: model::Model::new(&fleet),
            fleet,
            hidden: Vec::new(),
            stopped_open: false,
            world: World::new(Timestamp::now(), Vec::new()),
            dirs: BTreeMap::new(),
            ids: BTreeMap::new(),
            sole: BTreeMap::new(),
            facts: BTreeMap::new(),
            needs: BTreeMap::new(),
            pairs: BTreeMap::new(),
            shown: BTreeMap::new(),
            pressures: BTreeMap::new(),
            looks: BTreeMap::new(),
            lane: Rc::default(),
            merged: None,
            loading: true,
            fleeted: false,
            overview: overview::Overview::default(),
            pair: Vec::new(),
            inputs: BTreeMap::new(),
            recovery: submit::Recovery::default(),
            bound: BTreeMap::new(),
            read_only: "the fleet is not read yet".to_owned(),
            entered: None,
            writer: None,
            clipboard: None,
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
            pane_view: None,
            resumed_at: None,
            settings_bodies: settings::SettingsBodies::default(),
            quota_at: None,
            quota_failed: false,
            settings_generation: 0,
            stats_shown: None,
            tools_asked: None,
            stats_reset: None,
            peek: PeekPane::default(),
            peek_generation: 0,
            reader: ReaderPane::default(),
            reader_generation: 0,
            records_generation: 0,
            md: RefCell::default(),
            pending: None,
            server,
            pane,
            drawn: None,
            armed: None,
            cleared_at: None,
            flash: None,
            keyed: None,
            admitted: None,
            queued: Vec::new(),
            flight: None,
            settling: None,
            expanded: BTreeSet::new(),
            sent: BTreeSet::new(),
            read_line: String::new(),
            upgrade: None,
            failed: None,
            last_input: None,
            select: None,
            select_on_panel: false,
            read_press: None,
            shot: Buffer::default(),
            again: false,
            keep: None,
            touched: false,
            restored: false,
            early: Vec::new(),
            early_named: false,
            clip: None,
            parked: Vec::new(),
            modal: None,
            cut: None,
            root: None,
            cue: cue::Cue::default(),
            carry: Arc::new(term::Carry::run),
            detach: Box::new(transport::run_detached),
            orch,
        }
    }

    /// Fold one answer from the reader.
    fn answer(&mut self, answer: Answer) {
        let paints = matches!(
            answer,
            Answer::Fleet(_) | Answer::Look { .. } | Answer::View(_) | Answer::Orchestrator(_)
        );
        if paints && (!self.parked.is_empty() || self.holds_lane()) {
            return self.park(answer);
        }
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
                live_seats,
            } => {
                let current = self.model.settings_generation(generation);
                self.settled(generation, quota, config, about, instructions);
                if current && let Some(seats) = live_seats {
                    self.settings_bodies.live_seats = Some(seats);
                }
            }
            Answer::Stats { generation, glance } => self.stats_landed(generation, &glance),
            Answer::Tools { generation, view } => self.tools_landed(generation, view),
            Answer::Core(seen) => self.upgrade = seen,
            // A handed-over view of the orchestrator opens its panel, never a row.
            Answer::Handover { view, .. } if view.session == ORCH => {
                if !self.touched {
                    self.orch.opening = Opening::Handed(view.id);
                }
            }
            Answer::Handover { view, at } => {
                self.keep = (!self.touched).then_some(Keep { view, at });
            }
            Answer::Peek {
                generation,
                at,
                body,
            } => self.peeked(generation, at, body),
            Answer::Read { generation, body } => self.read_landed(generation, body),
            Answer::Records { generation, rows } => self.records_landed(generation, rows),
            Answer::Ended { name, line } => self.tell(&name, line),
            Answer::Pressure { name, id, pressure } => {
                self.pressures.insert(name, (id, pressure));
            }
            Answer::Orchestrator(proof) => self.proved(proof),
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
        self.pressures.retain(|name, (id, _)| kept(name, id));
        let old = std::mem::take(&mut self.ids);
        self.looks.retain(|name, _| match old.get(name) {
            Some(id) => kept(name, id),
            None => home.as_ref() == Some(name) || read.ids.contains_key(name),
        });
        self.dirs = read.dirs;
        self.ids = read.ids;
        self.sole = read.sole;
        self.world = read.world;
        self.facts = read.facts;
        self.needs = read.needs;
        let drawn: Vec<String> = self.fleet.rows.iter().map(|row| row.name.clone()).collect();
        self.pending = Some(read.fleet.rows.iter().map(|row| row.name.clone()).collect());
        self.fleet = read.fleet.arranged(&drawn);
        self.fold_stopped(self.fleeted, read.scanned);
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
        for (why, key, mut input) in dropped {
            if self.writer.as_ref().is_some_and(|w| w.key() == key) {
                self.write(false);
                // The panel ends its own entry and says why in its one line.
                if key.0 != ORCH {
                    self.held = Some(why.to_owned());
                }
            }
            self.bound.remove(&key);
            if !input.draft().is_empty() {
                self.notice(&key.0, format!("draft for {} dropped: {why}", key.0));
            }
            input.vacate();
            crate::clipboard::sweep(&mut input);
        }
        self.settle_keep(read.scanned);
        if self.model.selected().is_none() {
            self.model.restart(&self.fleet);
        }
        // The target is always the selection, and an incomplete read that omits
        // it moves nothing (R-B4 ext); a read that moved it or proved its own
        // incarnation stopped ends the writing HELD, never into browse (D3).
        let writing = self.writer.as_ref().map(Binding::key);
        let listed =
            |name: &str| name == ORCH || self.fleet.rows.iter().any(|row| row.name == name);
        if read.scanned || writing.as_ref().is_none_or(|(name, _)| listed(name)) {
            self.model.reconcile(&self.fleet);
        }
        // The panel's entry is the panel's: its proof ends it ([`App::proved`]).
        if let Some((name, id)) = writing.filter(|(name, _)| name != ORCH) {
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

    /// Start or stop writing the selection: the one writer of the entry. A
    /// refused start is HELD. Each is a new entry, and the reader is told
    /// which writer it re-proves.
    fn write(&mut self, on: bool) {
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

    /// Record where this app's pane sits in the lease a submit just took;
    /// outside tmux, or when tmux or the grammar refuses, the record stays empty.
    fn note_holder(&self, lease: &File) {
        let place = (self.server.as_ref().zip(self.pane.as_deref()))
            .and_then(|(server, pane)| transport::observe_pane_place(server, pane));
        let record = place.and_then(|(session, window)| console::holder::encode(&session, &window));
        let _ = crate::store::SessionStore::console_writer_note(
            lease,
            record.as_deref().unwrap_or_default(),
        );
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
        let writer = (self.writer.as_ref())
            .filter(|writer| writer.console.name() == name)
            .map(Binding::key);
        let key = writer.unwrap_or_else(|| (name.to_owned(), self.id(name)));
        self.inputs.get(&key)
    }

    /// Where a plain line goes: the lead unless it cannot take the line
    /// ([`route`]). A line that names a seat with `@` goes to that seat alone;
    /// the next plain line routes again.
    fn routed(
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
    fn destination(&self, session: &str) -> Option<(String, bool)> {
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
    fn composing(&self) -> bool {
        self.entered.is_some()
    }

    /// Show the selected session from what was answered: its last read lane,
    /// or `loading` until its first read lands — never another session's
    /// lane — and its open asks and working dir from the fleet's read.
    fn show(&mut self) {
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
        let shown = self.shown.get(&name).map(|shown| Rc::clone(&shown.lane));
        self.loading = shown.is_none();
        self.lane = self.merge(&name, shown);
        self.overview = overview::of(&entry, self.needs.get(&name), user.as_deref(), now);
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

    /// Whether Enter may start a line: the selection's last reading proved
    /// it, and no seat's screen stands in the chat's place.
    fn can_compose(&self) -> bool {
        self.read_only.is_empty()
            && self.model.selected().is_some()
            && !self.model.peek_open()
            && !self.model.reader_open()
    }

    /// One key while composing: Esc keeps the draft and browses, ^C over an
    /// empty composer quits (over text it clears, see [`App::clear_draft`]),
    /// one read before the entry began is dropped, and anything else is
    /// the chat's own input. `None` quits.
    fn compose(&mut self, key: Key, origin: Instant) -> Option<()> {
        // The browse paste that started this entry keeps every fragment.
        let origin = match (&key, &self.entered) {
            (Key::Pasted(_), Some(entry)) if self.admitted == Some(origin) => entry.at,
            _ => origin,
        };
        match key {
            Key::Escape
                if !self
                    .writer
                    .as_ref()
                    .and_then(|w| self.inputs.get(&w.key()))
                    .is_some_and(Input::offering) =>
            {
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
                let input = self.writer.as_ref().map(Binding::key);
                let effects = input
                    .and_then(|input| self.inputs.get_mut(&input))
                    .map(|input| input.keyed(vec![(key, origin)]))
                    .unwrap_or_default();
                if deleting && had_text && !self.protects() {
                    self.cleared_note(origin);
                }
                if !effects.is_empty() {
                    self.effects(effects);
                    self.reread();
                }
            }
        }
        Some(())
    }

    /// One key while HELD: Esc browses, ^C quits, Enter tries the entry
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

    /// ^C over text in the composer on show — the writer's draft, or the one
    /// Esc kept — clears it whole and keeps it for `^Y`; the app never quits
    /// over text. Whether it cleared.
    fn clear_draft(&mut self, origin: Instant) -> bool {
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
    fn guard_quit(&mut self, origin: Instant) -> bool {
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
    fn has_text(&self) -> bool {
        self.shown_input()
            .is_some_and(|input| !input.draft().is_empty())
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

    /// What names `item` for its fold across re-reads: the selected session,
    /// its incarnation and the turn's journal position, or its words when it
    /// has none. A changed body keeps it; another incarnation never shares it.
    fn fold_key(&self, item: &Item) -> u64 {
        let name = self.model.selected().unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        (name, self.ids.get(name)).hash(&mut hasher);
        match item.record {
            Some(record) => (0_u8, record).hash(&mut hasher),
            None => (1_u8, item.micros, &item.kind, &item.body).hash(&mut hasher),
        }
        hasher.finish()
    }

    /// What names the ask `id` this app sent to session `name`.
    fn sent_key(&self, name: &str, id: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        (name, self.ids.get(name), id).hash(&mut hasher);
        hasher.finish()
    }

    /// Each lane turn's fold key, whether it shows whole and whether it is an
    /// ask that went through: built per frame, from the sets alone.
    fn fold_state(&self) -> (Vec<u64>, Vec<bool>, Vec<bool>) {
        let name = self.model.selected().unwrap_or_default();
        let items = &self.lane.items;
        let folds: Vec<u64> = items.iter().map(|item| self.fold_key(item)).collect();
        let open = folds
            .iter()
            .map(|key| self.expanded.contains(key))
            .collect();
        let sent = items.iter().map(|item| match &item.kind {
            Kind::Asked { id, .. } => self.sent.contains(&self.sent_key(name, id)),
            _ => false,
        });
        (folds, open, sent.collect())
    }

    /// Fold or unfold the turn with fold key `key`.
    fn toggle_fold(&mut self, key: u64) {
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
    }

    /// Enter on a marked turn that has a fold row: toggle it, the mark kept.
    /// A draft held anywhere is never put aside by it. Whether it toggled.
    fn toggle_marked(&mut self) -> bool {
        let Some(mark) = self.model.turn().filter(|_| !self.protects()) else {
            return false;
        };
        let fold = self.layout.folds.iter().find(|(turn, _)| *turn == mark);
        let Some(&(_, key)) = fold else {
            return false;
        };
        self.toggle_fold(key);
        true
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

    /// The marked turn, or the words that say why there is none. A mark the
    /// lane no longer holds is cleared.
    fn marked_item(&mut self) -> Result<Item, String> {
        let Some(mark) = self.model.turn() else {
            return Err("no turn marked - click one or press [ ]".to_owned());
        };
        let keys = self.keys();
        let found = keys.iter().position(|key| *key == mark);
        let Some(item) = found.and_then(|at| self.lane.items.get(at)) else {
            self.model.set_turn(None);
            return Err("that turn changed or left the lane - mark it again".to_owned());
        };
        Ok(item.clone())
    }

    /// The words for a copy. A refusal before any tmux call is a plain hint;
    /// only a copy that was tried, or cannot exist, says `copy failed`.
    fn copied(&mut self) -> String {
        let item = match self.marked_item() {
            Ok(item) => item,
            Err(words) => return words,
        };
        let text = terminal_text(&item.body);
        if text.is_empty() {
            return "nothing to copy: that turn has no text".to_owned();
        }
        let preview = lane::preview_why(&item).map(|why| format!(" (preview only: {why})"));
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
        let queued = self.queued.first().map(|ask| ask.seat.as_str());
        let flying = self.flight.as_ref().map(|flight| flight.seat.as_str());
        let sending = queued.or(flying).map(|seat| format!("sending to {seat}…"));
        sending
            .or_else(|| self.armed.as_ref().map(|armed| armed.what.line()))
            .or_else(|| self.flash.as_ref().map(|(line, _)| line.clone()))
            .or_else(|| {
                let settling = self.settling.as_ref();
                let here = |s: &&Settling| self.model.selected() == Some(s.about.as_str());
                settling.filter(here).map(|settling| settling.word.clone())
            })
            .or_else(|| {
                let failed = self.failed.as_ref();
                self.upgrade
                    .as_ref()
                    .map(|seen| upgrade::hint(seen, failed))
            })
    }

    /// What holds a restart back now: any draft, not only the writer's —
    /// one kept in memory after Esc is lost too.
    fn blocker(&self, now: Instant) -> Option<upgrade::Blocker> {
        upgrade::restart_blocker(
            self.inputs.values().any(|input| !input.draft().is_empty()),
            !self.queued.is_empty() || self.flight.is_some(),
            !self.deferred.is_empty(),
            self.composing(),
            self.held.is_some(),
            self.model.settings_open()
                || self.model.peek_open()
                || self.model.reader_open()
                || self.modal.is_some(),
            self.model.turn().is_some(),
            self.armed.is_some(),
            self.last_input,
            now,
        )
    }

    /// The restart due now: another core is installed, this app may restart
    /// into it, no exec into it has failed, and nothing holds it back.
    fn restart_due(&self, now: Instant) -> Option<(String, upgrade::Via)> {
        let upgrade = self.upgrade.as_ref()?;
        let restart = upgrade.restart.as_ref().ok()?;
        let tried = (self.failed.as_ref()).is_some_and(|(version, _)| *version == upgrade.version);
        (!tried && self.blocker(now).is_none() && self.clip.is_none() && !self.holds_lane())
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
    /// frame rather than stopping short at it. A viewed seat owns its wheel
    /// at once: the chat the bounds describe is hidden, so its bound waits
    /// for nothing.
    fn unbounded(&self, key: &Key) -> bool {
        if self.model.peek_open() || self.model.reader_open() {
            return false;
        }
        let Key::Mouse(mouse) = key else {
            return false;
        };
        matches!(mouse.kind, MouseKind::WheelUp | MouseKind::WheelDown)
            && self.modal.is_none()
            && self.layout.in_chat(*mouse)
            && !self.layout.complete
            && self.model.wheel_passes(
                mouse.kind == MouseKind::WheelUp,
                self.layout.page_rows,
                self.layout.max_scroll,
            )
    }

    /// What the loop owes after a frame: the staged copy shipped and a hint
    /// the frame had no room to draw. Whether another frame is due.
    fn shipped(&mut self) -> bool {
        self.ship_staged() || std::mem::take(&mut self.again)
    }

    /// A frame's debts, settled together: the copy it shipped and the asks
    /// that waited for it. Neither skips the other; whether a frame is due.
    fn owed(&mut self) -> bool {
        self.inputs.values_mut().for_each(crate::clipboard::sweep);
        self.shipped() | self.deliver() | self.land() | self.unpark() | self.image_ready()
    }

    fn image_ready(&mut self) -> bool {
        let token = self.writer.as_ref().map(|w| (w.key(), w.entry));
        let Some(answer) = crate::clipboard::poll(&mut self.clipboard, token.as_ref()) else {
            return false;
        };
        if self.owns(&Cell::default()).is_err() {
            return false;
        }
        let input = token.and_then(|(key, _)| self.inputs.get_mut(&key));
        if let Some(input) = input {
            if let Err(why) = crate::clipboard::finish(input, answer) {
                self.refuse_paste(&why);
            }
            self.show();
        }
        true
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
                Effect::DiscardDraft => {
                    let result = self.owns(&Cell::default()).and_then(|()| {
                        let writer = self.writer.as_ref().ok_or("not writing")?;
                        self.recovery
                            .record(writer.console.dir())?
                            .clear()
                            .map_err(|why| why.to_string())
                    });
                    match result {
                        Ok(()) => {
                            if let Some(input) = self
                                .writer
                                .as_ref()
                                .and_then(|w| self.inputs.get_mut(&w.key()))
                            {
                                input.discarded();
                            }
                            "kept line discarded".to_owned()
                        }
                        Err(why) => format!("refused: {why}"),
                    }
                }
                Effect::ImagePaste => {
                    if self.owns(&Cell::default()).is_ok()
                        && let Some(w) = &self.writer
                    {
                        crate::clipboard::start(
                            &mut self.clipboard,
                            (w.key(), w.entry),
                            w.console.dir().to_owned(),
                        );
                    }
                    continue;
                }
                Effect::Print(line) => line,
                Effect::Ask { raw, seat, body } => {
                    let Some((key, entry)) = (self.writer.as_ref()).map(|w| (w.key(), w.entry))
                    else {
                        self.notice(&about, "refused: not writing".to_owned());
                        continue;
                    };
                    let (seat, body, note) = self.routed(&about, &raw, seat, body);
                    self.queued.push(Queued {
                        key,
                        entry,
                        raw,
                        seat,
                        body,
                        about,
                        note,
                    });
                    continue;
                }
                Effect::Close(id) => self.close(id.as_deref()),
                Effect::Open(seat) => self.open_written(&seat),
                Effect::Shell { raw, line } => {
                    let lost = Cell::default();
                    match self.owns(&lost) {
                        Ok(()) => self.shell_written(&raw, &line),
                        Err(why) => {
                            self.unproven(lost, &raw);
                            format!("refused: {why}")
                        }
                    }
                }
                Effect::More(_) => {
                    "refused: /more is the tmux chat's; click or press Enter on the fold line"
                        .to_owned()
                }
                Effect::Paste(_) | Effect::Lane(_) | Effect::Styled(_) => continue,
            };
            self.notice(&about, line);
        }
        self.show();
    }

    /// `!` in the window that writes: the human's own line, run in the
    /// session's work dir — a popup on the client showing this app, or a
    /// detached shell when the app has no tmux pane. A refusal puts the line
    /// back in the writer's composer; nothing is recorded either way.
    fn shell_written(&mut self, raw: &[u8], line: &str) -> String {
        let writer = self.writer.as_ref().ok_or_else(|| "not writing".to_owned());
        let started = writer.and_then(|writer| {
            let dir = writer.console.work_dir()?;
            let place = match (self.pane.as_deref(), self.server.as_ref()) {
                (None, _) => shell::Place::Outside,
                (Some(pane), Some(server)) => shell::Place::Tmux { server, pane },
                (Some(_), None) => return Err("tmux did not answer".to_owned()),
            };
            term::shell(place, &dir, line)
        });
        started.unwrap_or_else(|why| {
            let key = self.writer.as_ref().map(Binding::key);
            if let Some(input) = key.and_then(|key| self.inputs.get_mut(&key)) {
                input.put_back(raw);
            }
            shell::refused(line, &why)
        })
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

    /// What the admission re-proves: this window writes, and the incarnation
    /// and lead pair its entry proved are the session's now. A session no
    /// longer proven is kept in `lost`.
    fn owns(&self, lost: &Cell<Option<String>>) -> Result<(), String> {
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

    /// One ask of `seat`, admitted here through the chat's own admission and
    /// carried on a thread of its own through the tracked path: its request id,
    /// the channel its outcome arrives on and the writer lease it holds. The
    /// lease and the admission lock are taken without waiting, so a session
    /// another window is sending to refuses at once. A session whose recorded
    /// server that path cannot name is no longer proven: refused in its words
    /// before anything is written.
    fn begin(
        &mut self,
        raw: &[u8],
        seat: &str,
        body: String,
    ) -> Result<(String, Receiver<submit::Outcome>, Arc<LeaseFile>), String> {
        let Some(writer) = &self.writer else {
            return Err("refused: not writing".to_owned());
        };
        let (console, lost) = (&writer.console, Cell::default());
        let record = self.recovery.record(console.dir())?;
        let (lease, lock) = term::send_locks(console).map_err(|why| format!("refused: {why}"))?;
        let held = Arc::new(LeaseFile { file: lease });
        self.note_holder(&held.file);
        let owns = || {
            self.owns(&lost)?;
            let name = console.name();
            let routed = crate::tracked::named_server(console.dir(), name, name);
            let routed = routed.map(drop).map_err(|why| why.message());
            routed.inspect_err(|why| lost.set(Some(why.clone())))
        };
        let admitted = term::admit_ask_locked(console, record, lock, owns, raw, seat, body);
        self.unproven(lost, raw);
        let (ask, admitted) = admitted.map_err(|why| format!("refused: {why}"))?;
        let id = ask.id().to_owned();
        let (send, done) = mpsc::channel();
        let (carry, named) = (Arc::clone(&self.carry), id.clone());
        let thread = std::thread::Builder::new().name("ae-app-ask".to_owned());
        // The admission lock goes with the ask: it is held until the delivery
        // ends, and a thread that dies lets it go.
        thread
            .spawn(move || {
                let outcome =
                    admitted.finish(&mut crate::events::Held::default(), &named, || carry(ask));
                let _ = send.send(outcome);
            })
            .map_err(|why| format!("refused: could not start the delivery ({why})"))?;
        Ok((id, done, held))
    }

    /// An ask whose delivery ended: its outcome, said as an ask's outcome is.
    /// Whether one landed.
    fn land(&mut self) -> bool {
        let Some(flight) = &self.flight else {
            return false;
        };
        let outcome = match flight.done.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                let why = "the delivery ended without an answer";
                submit::Outcome::Unknown(flight.id.clone(), why.to_owned())
            }
        };
        let Some(flight) = self.flight.take() else {
            return false;
        };
        // The delivery is over: its lease goes, so another window may send.
        drop(flight.lease);
        let mut lines = self.settle(flight.id, &flight.seat, &flight.about, outcome);
        lines.splice(0..0, flight.note);
        for line in lines {
            self.notice(&flight.about, line);
        }
        self.reread();
        self.show();
        true
    }

    /// End the loop's wait on an ask in flight, so a quit never cuts a
    /// delivery off half way: the terminal is already back, the human is told
    /// what is awaited, and the wait is [`FLIGHT_QUIT_BOUND`] at most.
    fn finish_flight(&mut self, err: &mut impl Write, bound: Duration) -> std::io::Result<()> {
        let Some(flight) = self.flight.take() else {
            return Ok(());
        };
        let seat = terminal_text(&flight.seat);
        writeln!(
            err,
            "ae app: finishing the delivery to {seat} (^C stops it)"
        )?;
        if flight.done.recv_timeout(bound) == Err(RecvTimeoutError::Timeout) {
            let secs = bound.as_secs_f32();
            writeln!(
                err,
                "ae app: the delivery to {seat} was not over in {secs}s"
            )?;
        }
        Ok(())
    }

    /// Deliver the queued asks, after the frame that named their target. Each
    /// is re-proven first: a writer that ended since asks nothing and its
    /// line goes back to the draft in memory. An admitted ask is carried on a
    /// thread of its own and lands in [`App::land`]; one at a time, so an ask
    /// behind it is refused where the human sees it. Whether any ran.
    fn deliver(&mut self) -> bool {
        let queued = std::mem::take(&mut self.queued);
        let ran = !queued.is_empty();
        for q in queued {
            let live = (self.writer.as_ref()).is_some_and(|writer| writer.entry == q.entry);
            let told = if live {
                let begun = match &self.flight {
                    Some(flight) => Err(format!(
                        "not sent to {}: an ask to {} is still being delivered; the line is back in the draft",
                        q.seat, flight.seat
                    )),
                    None => self.begin(&q.raw, &q.seat, q.body),
                };
                match begun {
                    Ok((id, done, lease)) => {
                        self.flight = Some(Flight {
                            id,
                            seat: q.seat,
                            about: q.about.clone(),
                            note: q.note,
                            done,
                            lease,
                        });
                        Vec::new()
                    }
                    Err(refused) => {
                        // A writer the refusal ended got its line back from `unproven`; the
                        // one still writing gets it here. A newer writer is not touched: the
                        // line then stays only in the kept-draft file.
                        let live = (self.writer.as_ref()).is_some_and(|w| w.entry == q.entry);
                        if let Some(input) = self.inputs.get_mut(&q.key).filter(|_| live) {
                            input.put_back(&q.raw);
                        }
                        vec![refused]
                    }
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
        // A held drag keeps the screen as drawn: re-aging the open asks under it
        // would change the cells it lit.
        if ran || !self.holds_lane() {
            self.show();
        }
        ran
    }

    /// What an ask's outcome shows. A sent ask is the lane's `· sent` header
    /// suffix and only its kept-draft warning is a notice; the lane already
    /// draws an uncertain or undelivered ask in the same words, so those wait
    /// on the hint row until it holds the ask; the rest has no lane line and
    /// is a notice.
    fn settle(
        &mut self,
        id: String,
        seat: &str,
        about: &str,
        outcome: submit::Outcome,
    ) -> Vec<String> {
        let check = format!("check {seat} pane (Agents: n/p, oo opens a seat)");
        let word = match outcome {
            submit::Outcome::Sent(_, kept) => {
                self.sent.insert(self.sent_key(about, &id));
                let (at, about) = (Instant::now(), about.to_owned());
                let word = "sent".to_owned();
                self.settling = Some(Settling {
                    at,
                    id,
                    word,
                    about,
                });
                return kept.into_iter().collect();
            }
            submit::Outcome::Unknown(_, said) => {
                return vec![crate::console::input::unrecorded_line(&said, &check)];
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

    /// `/close`: the session's newest open console ask, or the one named,
    /// through the chat's admission under the same zero-wait locks as a send.
    /// Locks another window holds refuse at once and put the command back.
    fn close(&mut self, id: Option<&str>) -> String {
        let Some(writer) = &self.writer else {
            return "refused: not writing".to_owned();
        };
        // The ask in flight holds the admission lock a close needs.
        if let Some(flight) = &self.flight {
            return format!(
                "refused: the ask to {} is still being delivered; /close waits for it",
                flight.seat
            );
        }
        let entry = id.map_or_else(|| "/close".to_owned(), |id| format!("/close {id}"));
        let (console, lost) = (&writer.console, Cell::default());
        let (lease, lock) = match term::send_locks(console) {
            Ok(locks) => locks,
            Err(why) => {
                let key = writer.key();
                if let Some(input) = self.inputs.get_mut(&key) {
                    input.put_back(entry.as_bytes());
                }
                return format!("refused: {why}");
            }
        };
        let held = LeaseFile { file: lease };
        self.note_holder(&held.file);
        let owns = || self.owns(&lost);
        let line = match console.close_owned(id, owns, lock) {
            Ok(closed) => closed.line(),
            Err(why) => format!("refused: {why}"),
        };
        drop(held);
        self.unproven(lost, entry.as_bytes());
        line
    }
}

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
    let mut sockets = crate::fleet_read::SocketPaths::live();
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
    while let Some((mut key, origin)) = keyed.next() {
        if !matches!(key, Key::Mouse(_)) {
            app.read_press = None;
        }
        // A pane view a key just asked for takes the terminal next: every key
        // after it, held ones included, was typed for the seat, never here.
        if app.pane_view.is_some() {
            app.deferred.clear();
            return Some(true);
        }
        // Only a draft takes a line break; everywhere else ^J is Enter.
        if key == Key::Newline && !app.composing() {
            key = Key::Enter;
        }
        // An open overlay or seat view owns its keys even before the first
        // fleet read lands; only plain navigation waits for the fleet.
        if !app.fleeted && !overlay_open(app) && key != Key::Interrupt {
            redraw |= early(app, &key, origin)?;
            continue;
        }
        if !app.deferred.is_empty() || !app.queued.is_empty() || app.unbounded(&key) {
            return hold(app, (key, origin), keyed);
        }
        // A read spent on a modal changes nothing more; an open modal takes
        // every key and press before any mode under it.
        if app.spent(origin) {
            continue;
        }
        if app.modal.is_some() {
            redraw |= match &key {
                Key::Mouse(mouse) => app.modal_mouse(*mouse, origin),
                key => app.modal_key(key, origin),
            };
            continue;
        }
        // Browsing here is not `Mode::Browse`: the seat view and the reader
        // count as browsing, the settings overlay does not.
        let browsing = !app.model.settings_open() && !app.composing() && app.held.is_none();
        if disarms(&key, browsing) {
            redraw |= app.armed.take().is_some();
        }
        // A selection a panel change left behind is gone before this key; Esc
        // with one still held only clears it.
        redraw |= app.stale_selection();
        if key == Key::Escape && app.select.take().is_some_and(|held| held.sel.moved()) {
            redraw = true;
            continue;
        }
        redraw |= route_key(app, key, origin)?;
    }
    Some(redraw)
}

/// Whether the settings overlay, the seat view or the reader is open: each
/// owns its keys.
fn overlay_open(app: &App) -> bool {
    app.model.settings_open() || app.model.peek_open() || app.model.reader_open()
}

/// Whether `key` ends the armed press: any key but `^C`, and but a letter
/// while `browsing`, where a Text run decides for itself byte by byte.
fn disarms(key: &Key, browsing: bool) -> bool {
    *key != Key::Interrupt && !(browsing && matches!(key, Key::Text(_)))
}

/// Route one key that passed every gate: a mouse report acts on the frame
/// that drew it; a key or a paste ends a drag first, then acts as it always
/// has, in the mode that owns it. `Some(redraw)`, or `None` to quit.
fn route_key(app: &mut App, key: Key, origin: Instant) -> Option<bool> {
    if let Key::Mouse(mouse) = &key {
        return Some(app.mouse_made(*mouse, origin));
    }
    let ended = app.model.let_go();
    let acted = by_mode(app, key, origin)?;
    Some(acted || ended)
}

/// Act on a key that is no mouse report: a `^C` first clears a draft or only
/// arms a quit when the last clear guards it, anything else goes to the mode
/// that owns the key. Judged here, after the `^C` and the end of a drag, never
/// earlier. `Some(redraw)`, or `None` to quit.
fn by_mode(app: &mut App, key: Key, origin: Instant) -> Option<bool> {
    if key == Key::Interrupt && (app.clear_draft(origin) || app.guard_quit(origin)) {
        return Some(true);
    }
    let overlays = [
        app.model.settings_open(),
        app.model.peek_open(),
        app.model.reader_open(),
    ];
    match mode(overlays, app.composing(), app.held.is_some()) {
        Mode::Settings => settings_key(app, &key, origin),
        Mode::Peek => peek_key(app, &key, origin),
        Mode::Reader => reader_key(app, &key, origin),
        Mode::Composer => app.compose_routed(key, origin).map(|()| true),
        Mode::Held if app.panel_focused() && !app.orch.active => {
            app.in_panel(|app| app.retry(&key)).unwrap_or(Some(false))
        }
        Mode::Held => app.retry(&key),
        Mode::Browse => browse(app, &key, origin),
    }
}

/// Who takes a key, first of these that is true: the settings overlay, the
/// seat view, the file reader, the composer, a refused (held) entry, else the
/// browse keys. Not the `browsing` that decides what ends an armed press in
/// [`take_keys`]: that one counts the seat view and the reader as browsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Settings,
    Peek,
    Reader,
    Composer,
    Held,
    Browse,
}

/// The mode that owns a key given which of them is open (see [`Mode`]): the
/// three `overlays` are settings, seat view and reader, in that order.
fn mode(overlays: [bool; 3], composing: bool, held: bool) -> Mode {
    let [settings, peek, reader] = overlays;
    if settings {
        Mode::Settings
    } else if peek {
        Mode::Peek
    } else if reader {
        Mode::Reader
    } else if composing {
        Mode::Composer
    } else if held {
        Mode::Held
    } else {
        Mode::Browse
    }
}

/// Hold `first` and every key after it (`rest`) in order behind what is
/// waiting: `Some(true)`, or `None` when a held `^C` quits at once.
fn hold(
    app: &mut App,
    first: (Key, Instant),
    rest: impl Iterator<Item = (Key, Instant)>,
) -> Option<bool> {
    app.deferred.push(first);
    app.deferred.extend(rest);
    let quit = held_quit(
        &app.deferred,
        app.queued.is_empty(),
        app.has_text(),
        app.cleared_at,
    );
    (!quit).then_some(true)
}

/// Whether the held keys quit at once: one is a `^C` the last clear (at
/// `cleared_at`) does not guard, judged at that key's own read, and no ask is
/// queued (a quit would lose it) and no draft has text (a quit would lose it).
fn held_quit(
    held: &[(Key, Instant)],
    nothing_queued: bool,
    has_text: bool,
    cleared_at: Option<Instant>,
) -> bool {
    nothing_queued
        && !has_text
        && (held.iter())
            .any(|(key, origin)| *key == Key::Interrupt && !guarded_at(cleared_at, *origin))
}

/// Whether a ^C read at `origin` falls inside [`QUIT_GUARD`] of a clear at
/// `cleared_at`: the press's own time, never the clock when it is looked at.
fn guarded_at(cleared_at: Option<Instant>, origin: Instant) -> bool {
    cleared_at.is_some_and(|at| origin.saturating_duration_since(at) <= QUIT_GUARD)
}

/// Take a key read before the first fleet read, a Text run byte by byte:
/// `q` presses its armed key now, and `,` and `?` open the settings now when
/// nothing waits, the rest of the run going to the overlay; a navigation key
/// waits for the fleet it navigates, and so does an opener behind one, and
/// every key after a waiting opener, which the overlay will own
/// ([`EARLY_CAP`], the oldest go first). Nothing else is taken, told once on
/// the hint row. Each of those ends an armed press. `Some(true)`, or `None`
/// to quit.
fn early(app: &mut App, key: &Key, origin: Instant) -> Option<bool> {
    let pieces = match key {
        Key::Text(bytes) => bytes.iter().map(|byte| Key::Text(vec![*byte])).collect(),
        other => vec![other.clone()],
    };
    for piece in pieces {
        // An earlier byte of this run opened the overlay: it owns the rest.
        if app.model.settings_open() {
            browse(app, &piece, origin)?;
            continue;
        }
        let behind = app.early.iter().any(|(key, _)| opens_settings(key));
        let now = (piece == Key::Text(b"q".to_vec()) && !behind)
            || (opens_settings(&piece) && app.early.is_empty());
        if now {
            browse(app, &piece, origin)?;
            continue;
        }
        app.armed = None;
        let nav = behind
            || opens_settings(&piece)
            || match &piece {
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

/// Whether `key` is one byte that opens the settings while browsing: `,`, or
/// `?` on its keys.
fn opens_settings(key: &Key) -> bool {
    matches!(key, Key::Text(byte) if matches!(byte.as_slice(), [b',' | b'?']))
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
            if app.model.reader_open() {
                redraw |= reader_keys(app, model::browse_keys(&Key::Text(vec![byte])));
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
                app.compose_routed(Key::Text(rest.to_vec()), origin)
                    .map(|()| true)?;
                return Some(true);
            }
            redraw |= browse_byte(app, byte, origin)?;
            rest = tail;
        }
        return Some(redraw);
    }
    // With the panel holding the keys, Esc hides it, the arrows step the
    // list beside a side panel, and the rest is the panel's chat.
    if app.panel_focused() && !app.orch.active {
        match key {
            Key::Escape => {
                app.close_panel();
                return Some(true);
            }
            // The arrows are `j` / `k`: the list's beside a side panel only.
            Key::Up | Key::Down if matches!(app.panel_byte(b'j'), PanelByte::Inert) => {
                return Some(false);
            }
            Key::Up | Key::Down | Key::Interrupt => {}
            _ => {
                return app
                    .in_panel(|app| browse(app, key, origin))
                    .unwrap_or(Some(false));
            }
        }
    }
    if let Key::Pasted(bytes) = key {
        paste(app, bytes, origin);
        return Some(true);
    }
    if *key == Key::Enter && app.toggle_marked() {
        return Some(true);
    }
    let mut redraw = false;
    for key in model::browse_keys(key) {
        let act = app.model.key(key, &app.fleet, app.can_compose());
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

/// One Text byte while browsing: `q` and `o` press their armed key, `,`
/// opens the settings and `?` its keys, `m` the menu on the selection's card,
/// `s` starts or stops the selection, `z` opens or closes the stopped group,
/// anything else decodes as it always has and ends the
/// armed press. A byte of a read the menu spent does nothing. Browsing on
/// entry; the caller routes the modes.
fn browse_byte(app: &mut App, byte: u8, origin: Instant) -> Option<bool> {
    if app.spent(origin) {
        return Some(false);
    }
    let disarmed = !matches!(byte, b'q' | b'o') && app.armed.take().is_some();
    if !app.orch.active {
        match byte {
            b'O' => {
                app.toggle_panel();
                return Some(true);
            }
            b'F' => return Some(app.flip_panel() || disarmed),
            b'!' => return Some(app.next_need() || disarmed),
            _ => {}
        }
        if app.panel_focused() {
            match app.panel_byte(byte) {
                PanelByte::Inert => return Some(disarmed),
                PanelByte::Panel => {
                    let done = app.in_panel(|app| browse_byte(app, byte, origin));
                    if byte == b'r' && app.model.reader_open() {
                        app.reader_to_main();
                    }
                    return done.unwrap_or(Some(disarmed));
                }
                PanelByte::Main => {}
            }
        }
    }
    match byte {
        b'q' => return app.press(Arm::Quit, origin).is_none().then_some(true),
        b',' => {
            app.open_settings();
            return Some(true);
        }
        b's' => return Some(app.toggle(origin)),
        b'z' => return Some(app.toggle_stopped()),
        b'?' => {
            app.open_settings();
            app.model.show_settings_tab(model::SettingsTab::Keys);
            return Some(true);
        }
        b'm' => {
            let card = (app.model.selected()).and_then(|name| app.layout.cards.get(name));
            return Some(app.open_menu(card.cloned(), origin));
        }
        _ => {}
    }
    let mut redraw = disarmed;
    for key in model::browse_keys(&Key::Text(vec![byte])) {
        let act = app.model.key(key, &app.fleet, app.can_compose());
        redraw |= apply(app, &act, origin)?;
    }
    Some(redraw)
}

/// Carry out a browse action from either keys or a frame's mouse target.
fn apply(app: &mut App, act: &model::Act, origin: Instant) -> Option<bool> {
    match act {
        model::Act::Quit => None,
        model::Act::Select(_) => {
            if app.model.peek_open() {
                app.close_peek();
            }
            if app.model.reader_open() {
                app.close_reader();
            }
            app.show();
            Some(true)
        }
        model::Act::Redraw => Some(true),
        model::Act::StatsScope => {
            app.flip_stats_scope();
            Some(true)
        }
        model::Act::StatsReset => {
            app.ask_reset();
            Some(true)
        }
        model::Act::StatsPage(next) => Some(app.stats_page(*next)),
        model::Act::StatsFilter(next) => Some(app.stats_filter(*next)),
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
        model::Act::Read => {
            app.open_reader();
            Some(true)
        }
        // `O` is the panel's own control; the main column never selects the
        // orchestrator.
        model::Act::Orchestrator => {
            if !app.orch.active {
                app.toggle_panel();
            }
            Some(true)
        }
        model::Act::Notice(line) => {
            app.flash = Some((line.clone(), Instant::now()));
            Some(true)
        }
        model::Act::None => Some(false),
    }
}

#[cfg(test)]
mod tests;
