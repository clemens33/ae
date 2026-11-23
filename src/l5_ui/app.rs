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
use crate::clipboard::lines_word;
use crate::console::input::{Effect, Input, Key, Keys, Mouse, MouseKind, Reading, Size, View};
use crate::console::lane::{Item, Kind, Lane, Seat};
use crate::console::link;
use crate::console::needs::{SeatRef, Section};
use crate::console::open::{self, Move, Refusal};
use crate::console::target::{self, RoleRoute, Target};
use crate::console::{self, Console, shell, submit, term, toggle};
use crate::digest::{SessionEntry, Status};
use crate::inventory::ServerId;
use crate::listing::World;
use crate::session_menu::CellRow;
use crate::time::Timestamp;
use crate::{doors, theme, tmux, transport};

use compose::Merged;
use flight::{FLIGHT_QUIT_BOUND, Flight, Queued, Settling};
use frame::draw_frame;
#[cfg(test)]
use keys::{Mode, disarms, held_quit, mode};
use keys::{apply, browse, browse_byte, guarded_at, take_keys};
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
mod compose;
mod cue;
#[cfg(test)]
mod dechist_spec;
pub mod draw;
#[cfg(test)]
mod early_settings_spec;
#[cfg(test)]
#[path = "../../tests/it/app_early_spec.rs"]
mod early_spec;
mod filedoor;
pub mod fleet;
#[cfg(test)]
mod fleet_spec;
mod flight;
mod frame;
pub mod handover;
pub mod highlight;
mod journals;
mod keys;
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
#[cfg(test)]
mod seatfold_edges_spec;
#[cfg(test)]
mod seatfold_spec;
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
mod stats3_driver_tests;
#[cfg(test)]
mod stats3_stopped_spec;
#[cfg(test)]
mod stats_spec;
#[cfg(test)]
mod tools_keys_tests;
#[cfg(test)]
mod tools_spec;
mod tty;
pub mod upgrade;
#[cfg(test)]
#[path = "../../tests/it/app_view_state_spec.rs"]
mod view_spec;
mod viewer;

#[cfg(test)]
#[path = "../../tests/it/app_longpaste_spec.rs"]
mod longpaste_spec;

#[cfg(test)]
#[path = "../../tests/it/app_livepeek_spec.rs"]
mod livepeek_spec;

#[cfg(test)]
#[path = "../../tests/it/app_takekeys_spec.rs"]
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
/// The reserved opening of ae's composer target notices.
const TARGET_NOTICE: &str = "Target: ";
/// How long an armed key waits for its second press.
const QUIT_WINDOW: Duration = Duration::from_secs(2);
/// How long after a clear an empty-composer ^C only warns, so a second ^C
/// meant for the text never ends the app.
const QUIT_GUARD: Duration = Duration::from_secs(2);
/// How long a refused paste or an outcome word stays on the hint row.
const FLASH_WINDOW: Duration = Duration::from_secs(5);
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
    /// `o` over the orchestrator panel, on the pane target its frame drew: the
    /// second press opens that incarnation's main seat. A kind of its own, so
    /// a press armed in the main column never completes in the panel.
    Pane(Drawn),
}

impl Arm {
    /// The hint-row line that says what the next press does.
    fn line(&self) -> String {
        match self {
            Self::Quit => "q again to quit".to_owned(),
            Self::Interrupt => "^C again to quit · ^Y brings draft back".to_owned(),
            Self::Open(d) | Self::Pane(d) => {
                format!("o again to open {}", d.focused.as_deref().unwrap_or(""))
            }
        }
    }
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
    reason = "independent facts of one loop, each read where it is set: loading, fleeted, touched, restored, early_named, again, stopped_open, spawned_folded"
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
    /// Whether the Agents body folds the selected session's spawned seats
    /// into their one group line: this window's memory, open at start, never
    /// persisted and never handed over, one flag for every session.
    spawned_folded: bool,
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
    /// The last address that took the app's keys.
    composer_target: Option<Target>,
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
    redirect_recovery: submit::Recovery,
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
    /// The instant the newest frame that put another surface under the
    /// coordinates of the one before it (a zoom or unzoom of either column)
    /// was published. It belongs to the app, not to a column: a press made at
    /// or before it acts on nothing, whichever column the press landed on.
    zoom_cut: Option<Instant>,
    /// The frame just drawn changed the surface and is not yet published: the
    /// cut moves to the instant it is.
    zoom_pending: bool,
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
    /// What asked for the queued pane view: the words its refusals say. Set
    /// only with `pane_view`, by [`App::queue_pane_view`].
    pane_verb: paneview::Verb,
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
    /// The Statistics upkeep: a reset owed to it, and why it stopped.
    stats_upkeep: settings::UpkeepFlow,
    /// The orchestrator profile pick: owed to the writer, in flight, revealed.
    orchestrator: settings::OrchestratorFlow,
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
    #[cfg(test)]
    carry: Carry,
    /// The orchestrator, which has its own panel instead of a row.
    orch: Orch,
    /// How a process that must outlive this app is started: Stop's supervisor
    /// and a Start outside tmux.
    detach: Detach,
}

/// The delivery of one admitted ask, which says what the path wrote.
#[cfg(test)]
type Carry = Arc<dyn Fn(term::Carry) -> String + Send + Sync>;

/// Whether a detached argv started.
type Detach = Box<dyn Fn(&crate::lifecycle::DetachedArgv) -> bool>;

impl App {
    /// An app that has read nothing yet: home is selected. Where it runs
    /// decides nothing: each window writes under its own entry.
    #[allow(
        clippy::too_many_lines,
        reason = "one line per field of the App it builds"
    )]
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
            spawned_folded: false,
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
            composer_target: None,
            lane: Rc::default(),
            merged: None,
            loading: true,
            fleeted: false,
            overview: overview::Overview::default(),
            pair: Vec::new(),
            inputs: BTreeMap::new(),
            recovery: submit::Recovery::default(),
            redirect_recovery: submit::Recovery::default(),
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
            zoom_cut: None,
            zoom_pending: false,
            ask,
            focused: None,
            deferred: Vec::new(),
            pane_view: None,
            pane_verb: paneview::Verb::default(),
            resumed_at: None,
            settings_bodies: settings::SettingsBodies::default(),
            quota_at: None,
            quota_failed: false,
            settings_generation: 0,
            stats_shown: None,
            tools_asked: None,
            stats_upkeep: settings::UpkeepFlow::default(),
            orchestrator: settings::OrchestratorFlow::default(),
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
            #[cfg(test)]
            carry: Arc::new(term::Carry::run),
            detach: Box::new(transport::run_detached),
            orch,
        }
    }

    /// Fold one answer from the reader.
    fn answer(&mut self, answer: Answer) {
        let paints = matches!(
            answer,
            Answer::Fleet(_)
                | Answer::Look { .. }
                | Answer::View(_)
                | Answer::Live { .. }
                | Answer::Orchestrator(_)
                | Answer::ComposerRole { .. }
                | Answer::Seat { .. }
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
            Answer::Live { id, live } => self.read_live(id, *live),
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
            Answer::OrchestratorProfile { generation, view } => {
                self.orchestrator_resolved(generation, view);
            }
            Answer::Stats { generation, glance } => self.stats_landed(generation, &glance),
            Answer::Tools { generation, view } => self.tools_landed(generation, view),
            Answer::Core(seen) => self.upgrade = seen,
            // A handed-over view named `orchestrator` is the seat's or a
            // namesake row's: the seat's proof opens its panel, and the role
            // holder is no row, so only a namesake row takes the view.
            Answer::Handover { view, at } => {
                if view.session == ORCH && view.orchestrator.is_none() && !self.touched {
                    self.orch.opening = Opening::Handed(view.id.clone());
                }
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
            Answer::Orchestrator(proof) => self.proved(proof, Timestamp::now().epoch()),
            Answer::ComposerRole { holder, route } => {
                (self.orch.holder, self.orch.route) = (holder, route);
            }
            Answer::Seat { generation, said } => self.seated(generation, said),
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
                if !self.panel_entry(&key) {
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
        let listed = |key: &(String, String)| {
            self.panel_entry(key) || self.fleet.rows.iter().any(|row| row.name == key.0)
        };
        if read.scanned || writing.as_ref().is_none_or(listed) {
            self.model.reconcile(&self.fleet);
        }
        // The panel's entry is the panel's: the read that stops its seat or
        // its proof ends it ([`App::end_inert_panel_entry`]).
        if let Some((name, id)) = writing.filter(|key| !self.panel_entry(key)) {
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
        self.end_inert_panel_entry();
        if let Some(why) = lapsed.filter(|_| self.held.is_some()) {
            self.held = Some(why.to_owned());
        }
        self.evict();
        self.show();
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

#[cfg(test)]
mod tests;
